//! The answers of providers kept on disk: what a read was answered, with
//! its `ETag` and `Last-Modified`, so that the next read asks whether it
//! changed (`If-None-Match`, `If-Modified-Since`) and a `304` costs
//! nothing; and so that, when the provider cannot be reached or its
//! allowance of requests is spent, what was kept is answered, said to be
//! so.
//!
//! [`Kept`] stands in front of the network for one account. It keeps the
//! answers to reads only: a `GET`, or a GraphQL question (a `POST` to
//! `…/graphql` whose text asks and does not change). Everything else
//! passes through as it is. Each answer is a file in the account's
//! folder of the cache directory, `providers/<account's hash>/`, which
//! only the user may open (`0700`, files `0600`); its name is the hash of
//! the method, the address and what was sent. The folder is held to
//! [`MAX_BYTES`]: when it grows past that the oldest answers go.
//!
//! What it answers in place of the network:
//!
//! - a `304`: the kept answer, with the allowance the `304` told;
//! - no connection, no answer in time, or a `502`, `503` or `504`: the
//!   kept answer, marked offline with when it was kept;
//! - a `403` or `429` that says the allowance is spent (GitHub's
//!   `x-ratelimit-remaining: 0`, `Retry-After`, GitLab's
//!   `RateLimit-Remaining: 0`): the kept answer marked offline with when
//!   the allowance comes back; until then the provider is not asked for
//!   what was kept.
//!
//! Without a kept answer, what the network said is the answer. A read
//! asked `fresh` is asked without `If-None-Match`, and kept all the same.
//! The marks are header lines of this crate's own ([`OFFLINE`],
//! [`KEPT_AT`]), which no answer from the network can carry: the client
//! keeps only the lines it knows.
//!
//! Sources: RFC 9110 (`ETag`, `If-None-Match`, `Last-Modified`,
//! `If-Modified-Since`, `304 Not Modified`, `Retry-After`, `503`);
//! docs.github.com "Best practices for using the REST API" (conditional
//! requests, a `304` does not count against the limit), "Rate limits for
//! the REST API" (`x-ratelimit-remaining`, `x-ratelimit-reset`, `403` and
//! `429`, `retry-after`); docs.gitlab.com "Rate limits" (`RateLimit-*`,
//! `Retry-After`) and "REST API" (`ETag` on some answers).

use std::{
    fmt, fs,
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::http::{Call, Fetch, FetchError, Method, Response};

/// Most bytes an account's folder holds.
pub const MAX_BYTES: u64 = 64 * 1024 * 1024;
/// Largest answer kept.
const MAX_KEPT: usize = 8 * 1024 * 1024;
/// Most files an account's folder holds.
const MAX_FILES: usize = 4096;
/// Longest a spent allowance is believed, in seconds, whatever the
/// provider said.
const MAX_WAIT: i64 = 24 * 3600;

/// The header line of a kept answer that says it was given because the
/// provider could not be asked: when it was kept, in seconds since the
/// Unix epoch.
pub const OFFLINE: &str = "x-bitbraid-offline";
/// The header line of a kept answer that says when it was kept.
pub const KEPT_AT: &str = "x-bitbraid-kept-at";

/// What is kept of an answer, besides its body.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct Head {
    method: String,
    url: String,
    stored: i64,
    status: u16,
    headers: Vec<(String, String)>,
}

impl Head {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// What the folder knows of an account's allowance.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
struct Allowance {
    /// Spent until this time, in seconds since the Unix epoch.
    spent_until: Option<i64>,
}

/// The folder of an account in the cache directory:
/// `<cache>/providers/<the first 16 hex digits of the hash of its id>`.
pub fn folder(cache_dir: &Path, account: &str) -> PathBuf {
    let digest = Sha256::digest(account.as_bytes());
    let name: String = digest
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    cache_dir.join("providers").join(name)
}

/// Make a folder only the user may open.
fn private_folder(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

/// Write a file only the user may read, in one piece.
fn private_file(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    // A name of its own for each write, so that two threads writing the
    // same answer never write into one file.
    static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let write = WRITES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let partial = path.with_extension(format!("part-{}-{write}", std::process::id()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let written = options
        .open(&partial)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    match written.and_then(|()| fs::rename(&partial, path)) {
        Ok(()) => Ok(()),
        Err(err) => {
            let _ = fs::remove_file(&partial);
            Err(err)
        }
    }
}

/// Whether a request only reads: a `GET`, or a GraphQL question that
/// asks.
fn reads(call: &Call) -> bool {
    match call.method {
        Method::Get => true,
        Method::Post
            if call
                .url
                .path()
                .split('?')
                .next()
                .unwrap_or_default()
                .ends_with("/graphql") =>
        {
            call.body
                .as_deref()
                .and_then(|body| serde_json::from_slice::<serde_json::Value>(body).ok())
                .and_then(|body| body.get("query")?.as_str().map(str::to_owned))
                .is_some_and(|query| {
                    let query = query.trim_start();
                    query.starts_with("query") || query.starts_with('{')
                })
        }
        _ => false,
    }
}

/// Whether an error means the provider could not be reached.
fn unreachable(err: &FetchError) -> bool {
    matches!(
        err,
        FetchError::Resolve(_)
            | FetchError::Connect(_)
            | FetchError::Tls(_)
            | FetchError::Timeout
            | FetchError::Broken(_)
            | FetchError::Malformed(_)
    )
}

/// When a spent allowance comes back, if the answer says it is spent.
fn spent_until(response: &Response, now: i64) -> Option<i64> {
    let number = |name: &str| {
        response
            .header(name)
            .and_then(|value| value.trim().parse::<i64>().ok())
    };
    let remaining = number("x-ratelimit-remaining").or_else(|| number("ratelimit-remaining"));
    let limited = matches!(response.status, 403 | 429)
        && (remaining == Some(0) || response.status == 429 || number("retry-after").is_some());
    if !limited {
        return None;
    }
    let until = number("retry-after")
        .map(|seconds| now.saturating_add(seconds.clamp(0, MAX_WAIT)))
        .or_else(|| {
            number("x-ratelimit-reset")
                .or_else(|| number("ratelimit-reset"))
                .map(|reset| reset.clamp(now, now.saturating_add(MAX_WAIT)))
        })
        .unwrap_or_else(|| now.saturating_add(60));
    Some(until)
}

/// The network for one account, with what it was answered kept in its
/// folder.
pub struct Kept {
    inner: Arc<dyn Fetch>,
    folder: Option<PathBuf>,
    fresh: bool,
    now: fn() -> i64,
}

impl fmt::Debug for Kept {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kept")
            .field("inner", &self.inner)
            .field("folder", &self.folder)
            .field("fresh", &self.fresh)
            .finish()
    }
}

impl Kept {
    /// Keep the answers `inner` gives `account` in `cache_dir`, or none
    /// when there is no cache directory. `fresh` asks without offering
    /// what was kept.
    pub fn new(
        inner: Arc<dyn Fetch>,
        cache_dir: Option<&Path>,
        account: &str,
        fresh: bool,
        now: fn() -> i64,
    ) -> Self {
        Self {
            inner,
            folder: cache_dir.map(|dir| folder(dir, account)),
            fresh,
            now,
        }
    }

    fn key(call: &Call) -> String {
        let mut hasher = Sha256::new();
        hasher.update(call.method.as_str().as_bytes());
        hasher.update(b" ");
        hasher.update(call.url.to_string().as_bytes());
        hasher.update(b"\n");
        hasher.update(call.body.as_deref().unwrap_or_default());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// What was kept for a request: its head and body.
    fn load(folder: &Path, key: &str) -> Option<(Head, Vec<u8>)> {
        let path = folder.join(key);
        let meta = fs::symlink_metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > (MAX_KEPT as u64).saturating_add(64 * 1024) {
            return None;
        }
        let bytes = fs::read(&path).ok()?;
        let split = bytes.iter().position(|byte| *byte == b'\n')?;
        let head: Head = serde_json::from_slice(bytes.get(..split)?).ok()?;
        let body = bytes.get(split.saturating_add(1)..)?.to_vec();
        Some((head, body))
    }

    /// Keep an answer; what cannot be kept is not, and that is no error.
    fn store(&self, folder: &Path, key: &str, call: &Call, response: &Response) {
        if response.body.len() > MAX_KEPT || private_folder(folder).is_err() {
            return;
        }
        if let Some(root) = folder.parent() {
            let _ = private_folder(root);
        }
        let head = Head {
            method: call.method.as_str().to_owned(),
            url: call.url.to_string(),
            stored: (self.now)(),
            status: response.status,
            headers: response.headers.clone(),
        };
        let Ok(mut bytes) = serde_json::to_vec(&head) else {
            return;
        };
        bytes.push(b'\n');
        bytes.extend_from_slice(&response.body);
        if private_file(&folder.join(key), &bytes).is_ok() {
            bound(folder, MAX_BYTES, MAX_FILES);
        }
    }

    fn allowance(folder: &Path) -> Allowance {
        fs::read(folder.join("allowance"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn set_allowance(folder: &Path, allowance: Allowance) {
        if private_folder(folder).is_ok()
            && let Ok(bytes) = serde_json::to_vec(&allowance)
        {
            let _ = private_file(&folder.join("allowance"), &bytes);
        }
    }

    /// The kept answer, given in place of the network's: offline, or
    /// confirmed by a `304` whose allowance lines it takes.
    fn answer(
        head: &Head,
        body: Vec<u8>,
        offline: Option<i64>,
        confirmed: Option<&Response>,
    ) -> Response {
        let mut headers: Vec<(String, String)> = head
            .headers
            .iter()
            .filter(|(name, _)| {
                !name.contains("ratelimit") && !matches!(name.as_str(), OFFLINE | KEPT_AT)
            })
            .cloned()
            .collect();
        if let Some(confirmed) = confirmed {
            headers.extend(
                confirmed
                    .headers
                    .iter()
                    .filter(|(name, _)| name.contains("ratelimit"))
                    .cloned(),
            );
        }
        headers.push((KEPT_AT.to_owned(), head.stored.to_string()));
        if let Some(until) = offline {
            headers.push((OFFLINE.to_owned(), head.stored.to_string()));
            if until > 0 {
                headers.push(("x-ratelimit-remaining".to_owned(), "0".to_owned()));
                headers.push(("x-ratelimit-reset".to_owned(), until.to_string()));
            }
        }
        let header = |name: &str| {
            headers
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        };
        Response {
            status: head.status,
            location: header("location"),
            content_type: header("content-type"),
            body,
            headers,
        }
    }
}

impl Fetch for Kept {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        let Some(folder) = self.folder.as_deref().filter(|_| reads(call)) else {
            return self.inner.send(call, cancel);
        };
        let now = (self.now)();
        let key = Self::key(call);
        let kept = Self::load(folder, &key);
        // Spent until a time that has not come: what was kept, unasked.
        let allowance = Self::allowance(folder);
        if let (Some(until), Some((head, body))) = (allowance.spent_until, &kept)
            && until > now
        {
            return Ok(Self::answer(head, body.clone(), Some(until), None));
        }
        let mut asked = call.clone();
        if !self.fresh
            && let Some((head, _)) = &kept
        {
            if let Some(tag) = head.header("etag") {
                asked = asked.with_header("If-None-Match", tag);
            }
            if let Some(modified) = head.header("last-modified") {
                asked = asked.with_header("If-Modified-Since", modified);
            }
        }
        let response = match self.inner.send(&asked, cancel) {
            Ok(response) => response,
            Err(err) if unreachable(&err) => {
                return match kept {
                    Some((head, body)) => Ok(Self::answer(&head, body, Some(0), None)),
                    None => Err(err),
                };
            }
            Err(err) => return Err(err),
        };
        if let Some(until) = spent_until(&response, now) {
            Self::set_allowance(
                folder,
                Allowance {
                    spent_until: Some(until),
                },
            );
            return Ok(match kept {
                Some((head, body)) => Self::answer(&head, body, Some(until), None),
                None => response,
            });
        }
        if allowance.spent_until.is_some() {
            Self::set_allowance(folder, Allowance::default());
        }
        match (response.status, kept) {
            (304, Some((mut head, body))) => {
                // Still what was kept: kept as of now.
                head.stored = now;
                let answer = Self::answer(&head, body, None, Some(&response));
                self.store(
                    folder,
                    &key,
                    call,
                    &Response {
                        headers: head.headers.clone(),
                        ..answer.clone()
                    },
                );
                Ok(answer)
            }
            (502..=504, Some((head, body))) => Ok(Self::answer(&head, body, Some(0), None)),
            (200..=299, _) => {
                self.store(folder, &key, call, &response);
                Ok(response)
            }
            _ => Ok(response),
        }
    }
}

/// Hold a folder to `max_bytes` and `max_files`: the oldest files go
/// until it is three quarters of that.
pub fn bound(folder: &Path, max_bytes: u64, max_files: usize) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, u64, PathBuf)> = entries
        .flatten()
        .filter(|entry| entry.file_name() != "allowance")
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            meta.is_file().then(|| {
                (
                    meta.modified().unwrap_or(std::time::UNIX_EPOCH),
                    meta.len(),
                    entry.path(),
                )
            })
        })
        .collect();
    let total: u64 = files.iter().map(|(_, size, _)| *size).sum();
    if total <= max_bytes && files.len() <= max_files {
        return;
    }
    files.sort();
    let (mut bytes, mut count) = (total, files.len());
    let (bytes_goal, count_goal) = (
        max_bytes.saturating_mul(3) / 4,
        max_files.saturating_mul(3) / 4,
    );
    for (_, size, path) in files {
        if bytes <= bytes_goal && count <= count_goal {
            break;
        }
        if fs::remove_file(&path).is_ok() {
            bytes = bytes.saturating_sub(size);
            count = count.saturating_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Limits, Url};
    use std::sync::Mutex;

    /// A network that answers what the test lines up, and notes what it
    /// was asked.
    #[derive(Debug, Default)]
    struct Scripted {
        answers: Mutex<Vec<Result<Response, FetchError>>>,
        asked: Mutex<Vec<Call>>,
    }

    impl Scripted {
        fn then(&self, answer: Result<Response, FetchError>) {
            self.answers.lock().unwrap().push(answer);
        }
    }

    impl Fetch for Scripted {
        fn send(&self, call: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
            self.asked.lock().unwrap().push(call.clone());
            let mut answers = self.answers.lock().unwrap();
            if answers.is_empty() {
                return Err(FetchError::Unrecorded(call.url.path().to_owned()));
            }
            answers.remove(0)
        }
    }

    fn answer(status: u16, headers: &[(&str, &str)], body: &str) -> Response {
        Response {
            status,
            headers: headers
                .iter()
                .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
                .collect(),
            body: body.as_bytes().to_vec(),
            ..Default::default()
        }
    }

    fn get(path: &str) -> Call {
        Call::new(
            Method::Get,
            Url::parse(&format!("https://api.github.com{path}")).unwrap(),
            Limits {
                connect: std::time::Duration::from_secs(1),
                total: std::time::Duration::from_secs(1),
                body: 1 << 20,
            },
        )
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bitbraid-kept-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn now() -> i64 {
        1_791_801_000
    }

    fn kept(network: &Arc<Scripted>, dir: &Path, fresh: bool) -> Kept {
        Kept::new(
            network.clone(),
            Some(dir),
            "github:github.com:octo",
            fresh,
            now,
        )
    }

    #[test]
    fn a_read_is_asked_again_with_its_tag_and_a_304_is_what_was_kept() {
        let dir = scratch("tag");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        network.then(Ok(answer(
            200,
            &[
                ("etag", "\"abc\""),
                ("link", "<https://api.github.com/x?page=2>; rel=\"next\""),
                ("x-ratelimit-remaining", "4999"),
            ],
            "[1,2,3]",
        )));
        network.then(Ok(answer(304, &[("x-ratelimit-remaining", "4998")], "")));
        let first = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        assert_eq!(first.body, b"[1,2,3]");
        let again = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        assert_eq!(again.status, 200);
        assert_eq!(again.body, b"[1,2,3]");
        assert_eq!(again.header("x-ratelimit-remaining"), Some("4998"));
        assert!(
            again
                .header("link")
                .is_some_and(|link| link.contains("page=2"))
        );
        assert_eq!(again.header(OFFLINE), None);
        let asked = network.asked.lock().unwrap();
        assert_eq!(asked[0].header("If-None-Match"), None);
        assert_eq!(asked[1].header("If-None-Match"), Some("\"abc\""));
        // Only the user may open what is kept.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let folder = folder(&dir, "github:github.com:octo");
            let mode = |path: &Path| fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&folder), 0o700);
            assert_eq!(mode(&dir.join("providers")), 0o700);
            let file = fs::read_dir(&folder)
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path();
            assert_eq!(mode(&file), 0o600);
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_fresh_read_offers_nothing_and_is_kept_all_the_same() {
        let dir = scratch("fresh");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        network.then(Ok(answer(200, &[("etag", "\"one\"")], "1")));
        network.then(Ok(answer(200, &[("etag", "\"two\"")], "2")));
        network.then(Err(FetchError::Connect("down".into())));
        kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        let fresh = kept(&network, &dir, true).send(&get("/x"), &go).unwrap();
        assert_eq!(fresh.body, b"2");
        assert_eq!(
            network.asked.lock().unwrap()[1].header("If-None-Match"),
            None
        );
        // What the fresh read was answered is what is kept now.
        let offline = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        assert_eq!(offline.body, b"2");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn without_the_network_what_was_kept_is_answered_offline() {
        let dir = scratch("offline");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        network.then(Ok(answer(200, &[("x-ratelimit-remaining", "10")], "kept")));
        network.then(Err(FetchError::Timeout));
        network.then(Ok(answer(503, &[], "maintenance")));
        network.then(Err(FetchError::Resolve("no such host".into())));
        kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        for _ in 0..2 {
            let offline = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
            assert_eq!(offline.body, b"kept");
            assert_eq!(offline.header(OFFLINE), Some(now().to_string().as_str()));
            assert_eq!(offline.header("x-ratelimit-remaining"), None);
        }
        // Nothing kept for another address: the network's error.
        assert_eq!(
            kept(&network, &dir, false)
                .send(&get("/y"), &go)
                .unwrap_err(),
            FetchError::Resolve("no such host".into())
        );
        // Stopped is stopped, kept or not.
        network.then(Err(FetchError::Cancelled));
        assert_eq!(
            kept(&network, &dir, false)
                .send(&get("/x"), &go)
                .unwrap_err(),
            FetchError::Cancelled
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_spent_allowance_is_waited_out_on_what_was_kept() {
        let dir = scratch("spent");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        let reset = (now() + 600).to_string();
        network.then(Ok(answer(200, &[], "kept")));
        network.then(Ok(answer(
            403,
            &[
                ("x-ratelimit-remaining", "0"),
                ("x-ratelimit-reset", reset.as_str()),
            ],
            "{\"message\":\"API rate limit exceeded\"}",
        )));
        kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        let limited = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        assert_eq!(limited.body, b"kept");
        assert!(limited.header(OFFLINE).is_some());
        assert_eq!(limited.header("x-ratelimit-reset"), Some(reset.as_str()));
        // Until then the provider is not asked for what was kept.
        let before = network.asked.lock().unwrap().len();
        let unasked = kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        assert_eq!(unasked.body, b"kept");
        assert_eq!(network.asked.lock().unwrap().len(), before);
        // Without a kept answer the provider's refusal is the answer.
        network.then(Ok(answer(429, &[("retry-after", "30")], "slow down")));
        let refused = kept(&network, &dir, false).send(&get("/y"), &go).unwrap();
        assert_eq!(refused.status, 429);
        // A wait beyond sense is held to a day.
        assert_eq!(
            spent_until(&answer(429, &[("retry-after", "999999999999")], ""), now()),
            Some(now() + MAX_WAIT)
        );
        assert_eq!(
            spent_until(
                &answer(
                    403,
                    &[("ratelimit-remaining", "0"), ("ratelimit-reset", "-5")],
                    ""
                ),
                now()
            ),
            Some(now())
        );
        assert_eq!(spent_until(&answer(403, &[], "forbidden"), now()), None);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_reads_are_kept() {
        let dir = scratch("reads");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        let mut write = get("/repos/o/r/pulls");
        write.method = Method::Post;
        let graphql = |query: &str| {
            let mut call = get("/graphql");
            call.method = Method::Post;
            call.with_body(
                "application/json",
                serde_json::json!({ "query": query, "variables": {} })
                    .to_string()
                    .into_bytes(),
            )
        };
        assert!(reads(&get("/x")));
        assert!(reads(&graphql("query { rateLimit { remaining } }")));
        assert!(!reads(&graphql("mutation { x }")));
        assert!(!reads(&write));
        network.then(Ok(answer(201, &[], "made")));
        network.then(Err(FetchError::Connect("down".into())));
        kept(&network, &dir, false).send(&write, &go).unwrap();
        assert!(kept(&network, &dir, false).send(&write, &go).is_err());
        // Each account has its own folder.
        assert_ne!(folder(&dir, "a"), folder(&dir, "b"));
        // No cache directory: nothing is kept.
        network.then(Ok(answer(200, &[], "1")));
        network.then(Err(FetchError::Connect("down".into())));
        let none = Kept::new(network.clone(), None, "a", false, now);
        none.send(&get("/x"), &go).unwrap();
        assert!(none.send(&get("/x"), &go).is_err());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_folder_is_held_to_its_bound_by_dropping_the_oldest() {
        let dir = scratch("bound");
        fs::create_dir_all(&dir).unwrap();
        for n in 0..10 {
            fs::write(dir.join(format!("f{n}")), vec![b'x'; 100]).unwrap();
            let file = fs::File::options()
                .write(true)
                .open(dir.join(format!("f{n}")))
                .unwrap();
            file.set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000 + n))
                .unwrap();
        }
        fs::write(dir.join("allowance"), b"{}").unwrap();
        bound(&dir, 500, 100);
        let mut left: Vec<String> = fs::read_dir(&dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["allowance", "f7", "f8", "f9"]);
        bound(&dir, 10_000, 2);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_kept_file_that_is_not_one_is_as_none() {
        let dir = scratch("broken");
        let network = Arc::new(Scripted::default());
        let go = AtomicBool::new(false);
        network.then(Ok(answer(200, &[], "good")));
        kept(&network, &dir, false).send(&get("/x"), &go).unwrap();
        let folder = folder(&dir, "github:github.com:octo");
        for entry in fs::read_dir(&folder).unwrap() {
            fs::write(entry.unwrap().path(), b"\x00not a head\nbody").unwrap();
        }
        network.then(Err(FetchError::Connect("down".into())));
        assert!(kept(&network, &dir, false).send(&get("/x"), &go).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
