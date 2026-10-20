//! Recorded answers of providers, for the tests: a [`Replay`] answers
//! from a recording in place of the network, and a [`Recorder`] makes one
//! while a live test talks to a real provider.
//!
//! A recording is a JSON file:
//!
//! ```json
//! { "note": "where it came from",
//!   "exchanges": [ { "method": "GET", "path_and_query": "/user",
//!                    "request_body": null, "status": 200,
//!                    "headers": { "x-oauth-scopes": "repo" },
//!                    "body": { "login": "user-1" } } ] }
//! ```
//!
//! A request is matched by its method, its path and query and, for one
//! that sends a body, the body (as JSON, or as a form's fields, whatever
//! their order; a recorded body of `null` matches any, and a recorded
//! object matches a body that has its keys with their values, whatever
//! else it has). The host is not looked at. The same request asked again is answered by the next
//! exchange recorded for it, and by the last one once they are used up;
//! one that was not recorded is an error that names it.
//!
//! A recording is scrubbed as it is written. Header lines that sign in
//! are never recorded (no request header line is), and of the answer's
//! only [`KEPT_HEADERS`](crate::http::KEPT_HEADERS). Tokens and codes in
//! bodies become `token-1`, `device-code-1`, `user-code-1`, and so on.
//! People become stand-ins, the same one for the same person throughout a
//! recording: the login `user-1`, the name `User 1`, the address
//! `user-1@example.invalid`, the numeric id `100001`, the node id
//! `node-1`, the picture `https://avatars.example.invalid/user-1`; what
//! else they say of themselves (a place, a company, a site, a biography)
//! is emptied; where
//! their login, name or address is part of another text (an address, a
//! `full_name`, a path) it is replaced there too. An organisation (an
//! account GitHub calls `"type": "Organization"`) is scrubbed the same
//! way as `org-1`, `Org 1`, `200001`. The caller may choose the stand-in
//! of a login ([`Recorder::finish_named`]), so that the same account is
//! the same stand-in in every recording of a test. GitHub's addresses for
//! an account (`ID+login@users.noreply.github.com`) keep their shape:
//! their id and login are replaced in them.
//!
//! What would find the accounts again through a provider's public API is
//! replaced as well. The numeric id of anything else than a person (a
//! repository, a project, a namespace or group, a pull request, a
//! comment, a review, a pipeline: a number of five digits or more under
//! `id`, `…_id`, `…_ids` or `…Id`, or a long number in an address) becomes
//! a stand-in from `300001` on, the same one wherever the same id is, in
//! texts too (`/api/v4/projects/300001/…`, `#discussion_r300002`); a
//! number within a project (`iid`, `number`) stays. GitHub's node ids
//! (`R_kgDO…`, `PR_kwDO…`, and the older base64 ones) become `R_node-1`,
//! `PR_node-2`, `node-3`. A commit id becomes another forty hexadecimal
//! digits made from it (a digest that cannot be turned back), and its
//! abbreviations the same digits shortened; one a test made up (all `0`,
//! a short pattern repeated) stays. A project's runner token is a secret
//! like the others. The recordings a [`Recorder`] writes in one run share
//! their stand-ins of ids, so that the same repository is the same
//! stand-in in each.
//!
//! After it is written the file is read again, and when any of the real
//! values the caller names is still in it, the file is removed and that
//! is an error.
//!
//! Sources: none (a format of this crate's own; the header lines are
//! RFC 9110's and the providers' as in [`crate::http`]; the shapes of the
//! ids are those of GitHub's REST and GraphQL answers, docs.github.com
//! "Using global node IDs", and of GitLab's REST answers).

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::http::{Call, Fetch, FetchError, Method, Response};

/// One request and its answer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Exchange {
    /// The request's method.
    pub method: Method,
    /// Its path with its query.
    pub path_and_query: String,
    /// What it sent: JSON, a form's text, or nothing.
    #[serde(default)]
    pub request_body: Value,
    /// The answer's status.
    pub status: u16,
    /// The answer's kept header lines.
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// The answer's body: JSON, text, or nothing.
    #[serde(default)]
    pub body: Value,
}

/// A recording.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Recording {
    /// Where it came from: which server, when, made how.
    #[serde(default)]
    pub note: String,
    /// What it was made from when it was made by hand and not recorded
    /// from a live server: which examples of which published
    /// description, and why it could not be recorded. `None` for one
    /// recorded live.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hand_made: Option<String>,
    /// What was asked and answered, in order.
    pub exchanges: Vec<Exchange>,
}

/// Why a recording could not be read or written.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplayError {
    /// The file could not be read or written.
    #[error("the recording {0}")]
    File(String),
    /// It is not a recording.
    #[error("not a recording: {0}")]
    Shape(String),
    /// A real value was still in it after scrubbing; it was removed.
    #[error("a real value was left in the recording {0}; it was removed")]
    Leaked(String),
}

/// A body as a recording keeps it: JSON when it is, text otherwise.
fn kept_body(bytes: &[u8]) -> Value {
    if bytes.is_empty() {
        return Value::Null;
    }
    serde_json::from_slice(bytes)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(bytes).into_owned()))
}

/// A body as it is sent again.
fn body_bytes(value: &Value) -> Vec<u8> {
    match value {
        Value::Null => Vec::new(),
        Value::String(text) => text.clone().into_bytes(),
        other => other.to_string().into_bytes(),
    }
}

/// A form's fields, sorted.
fn form_fields(text: &str) -> Vec<(String, String)> {
    let mut fields: Vec<(String, String)> = text
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            (name.to_owned(), value.to_owned())
        })
        .collect();
    fields.sort();
    fields
}

/// Whether what a request sends is what was recorded.
fn same_body(recorded: &Value, sent: Option<&[u8]>) -> bool {
    if recorded.is_null() {
        return true;
    }
    let sent = sent.unwrap_or_default();
    let sent_value = kept_body(sent);
    match (recorded, &sent_value) {
        (Value::String(recorded), Value::String(sent)) => {
            recorded == sent || form_fields(recorded) == form_fields(sent)
        }
        (recorded, sent) => has(sent, recorded),
    }
}

/// Whether `sent` has what `recorded` has: the same value, or, for an
/// object, every key of the recorded one with what it has there. A
/// recording made by hand names only the part of a body that matters (a
/// GraphQL question's variables); one made live names all of it.
fn has(sent: &Value, recorded: &Value) -> bool {
    match (sent, recorded) {
        (Value::Object(sent), Value::Object(recorded)) => recorded
            .iter()
            .all(|(key, value)| sent.get(key).is_some_and(|sent| has(sent, value))),
        (sent, recorded) => sent == recorded,
    }
}

/// The network, answered from a recording.
pub struct Replay {
    recording: Recording,
    used: Mutex<Vec<bool>>,
    asked: Mutex<Vec<String>>,
}

impl fmt::Debug for Replay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Replay")
            .field("exchanges", &self.recording.exchanges.len())
            .finish()
    }
}

impl Replay {
    /// Answer from this recording.
    pub fn new(recording: Recording) -> Self {
        let count = recording.exchanges.len();
        Self {
            recording,
            used: Mutex::new(vec![false; count]),
            asked: Mutex::default(),
        }
    }

    /// Answer from a recording's text.
    pub fn from_json(text: &str) -> Result<Self, ReplayError> {
        let recording: Recording =
            serde_json::from_str(text).map_err(|err| ReplayError::Shape(err.to_string()))?;
        Ok(Self::new(recording))
    }

    /// Answer from a recording's file.
    pub fn load(path: &Path) -> Result<Self, ReplayError> {
        let text = fs::read_to_string(path)
            .map_err(|err| ReplayError::File(format!("{}: {err}", path.display())))?;
        Self::from_json(&text)
    }

    /// What was asked so far: `GET /user`, in order.
    pub fn asked(&self) -> Vec<String> {
        self.asked
            .lock()
            .map(|asked| asked.clone())
            .unwrap_or_default()
    }

    /// The exchanges no request was answered with yet.
    pub fn unused(&self) -> Vec<String> {
        let used = self
            .used
            .lock()
            .map(|used| used.clone())
            .unwrap_or_default();
        self.recording
            .exchanges
            .iter()
            .zip(used)
            .filter(|(_, used)| !used)
            .map(|(exchange, _)| {
                format!("{} {}", exchange.method.as_str(), exchange.path_and_query)
            })
            .collect()
    }
}

impl Fetch for Replay {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(FetchError::Cancelled);
        }
        let path = call.url.path();
        let named = format!("{} {path}", call.method.as_str());
        if let Ok(mut asked) = self.asked.lock() {
            asked.push(named.clone());
        }
        let matching: Vec<usize> = self
            .recording
            .exchanges
            .iter()
            .enumerate()
            .filter(|(_, exchange)| {
                exchange.method == call.method
                    && exchange.path_and_query == path
                    && same_body(&exchange.request_body, call.body.as_deref())
            })
            .map(|(at, _)| at)
            .collect();
        let mut used = self
            .used
            .lock()
            .map_err(|_| FetchError::Unrecorded(named.clone()))?;
        let at = matching
            .iter()
            .copied()
            .find(|at| used.get(*at) == Some(&false))
            .or_else(|| matching.last().copied())
            .ok_or(FetchError::Unrecorded(named))?;
        if let Some(slot) = used.get_mut(at) {
            *slot = true;
        }
        let exchange = self
            .recording
            .exchanges
            .get(at)
            .ok_or(FetchError::Unrecorded(path.to_owned()))?;
        let body = body_bytes(&exchange.body);
        if body.len() > call.limits.body {
            return Err(FetchError::TooLarge);
        }
        let headers: Vec<(String, String)> = exchange
            .headers
            .iter()
            .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
            .collect();
        let header = |name: &str| {
            headers
                .iter()
                .find(|(known, _)| known == name)
                .map(|(_, value)| value.clone())
        };
        Ok(Response {
            status: exchange.status,
            location: header("location"),
            content_type: header("content-type"),
            body,
            headers,
        })
    }
}

/// The real network, with every exchange noted for a recording.
pub struct Recorder {
    inner: Arc<dyn Fetch>,
    path: PathBuf,
    note: String,
    exchanges: Mutex<Vec<Exchange>>,
}

impl fmt::Debug for Recorder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Recorder")
            .field("inner", &self.inner)
            .field("path", &self.path)
            .finish()
    }
}

impl Recorder {
    /// Note what `inner` is asked and answers, for a recording at `path`.
    pub fn new(inner: Arc<dyn Fetch>, path: impl Into<PathBuf>, note: &str) -> Self {
        Self {
            inner,
            path: path.into(),
            note: note.to_owned(),
            exchanges: Mutex::default(),
        }
    }

    /// A recorder writing `<dir>/<name>.json` when `BITBRAID_RECORD=<dir>`
    /// is set; none otherwise.
    pub fn from_environment(inner: Arc<dyn Fetch>, name: &str, note: &str) -> Option<Self> {
        let dir = std::env::var_os("BITBRAID_RECORD")?;
        Some(Self::new(
            inner,
            Path::new(&dir).join(format!("{name}.json")),
            note,
        ))
    }

    /// Where it writes.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Scrub what was noted, write it, and read it again for any of the
    /// `real` values (logins, ids, names, addresses of real people). When
    /// one is still there, the file is removed and that is an error.
    pub fn finish(&self, real: &[&str]) -> Result<PathBuf, ReplayError> {
        self.finish_named(&[], real)
    }

    /// [`finish`](Self::finish), with the stand-ins of some logins chosen:
    /// `named` pairs a real login with its stand-in (`user-2`, `org-1`).
    /// Then the first of `real` is not taken for `user-1`'s; it is looked
    /// for like the others.
    pub fn finish_named(
        &self,
        named: &[(&str, &str)],
        real: &[&str],
    ) -> Result<PathBuf, ReplayError> {
        let exchanges = self
            .exchanges
            .lock()
            .map(|exchanges| exchanges.clone())
            .map_err(|_| ReplayError::File("a thread failed while noting".to_owned()))?;
        let mut recording = Recording {
            note: self.note.clone(),
            hand_made: None,
            exchanges,
        };
        // The stand-ins of ids are those of the recordings made before in
        // this run, and those this one adds are kept for the next.
        let mut book = BOOK
            .lock()
            .map(|book| book.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone());
        scrub_with_book(&mut recording, named, real, &mut book);
        if let Ok(mut shared) = BOOK.lock() {
            *shared = book;
        }
        let text = serde_json::to_string_pretty(&recording)
            .map_err(|err| ReplayError::Shape(err.to_string()))?;
        if let Some(dir) = self.path.parent() {
            fs::create_dir_all(dir).map_err(|err| ReplayError::File(err.to_string()))?;
        }
        fs::write(&self.path, format!("{text}\n"))
            .map_err(|err| ReplayError::File(err.to_string()))?;
        let written = fs::read_to_string(&self.path).unwrap_or_default();
        if let Some(leaked) = real.iter().find(|value| still_there(&written, value)) {
            let _ = fs::remove_file(&self.path);
            return Err(ReplayError::Leaked(format!(
                "{} ({} characters)",
                self.path.display(),
                leaked.len()
            )));
        }
        Ok(self.path.clone())
    }
}

impl Fetch for Recorder {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        let response = self.inner.send(call, cancel)?;
        let exchange = Exchange {
            method: call.method,
            path_and_query: call.url.path().to_owned(),
            request_body: call.body.as_deref().map_or(Value::Null, kept_body),
            status: response.status,
            headers: response
                .headers
                .iter()
                .map(|(name, value)| (name.clone(), value.clone()))
                .collect(),
            body: kept_body(&response.body),
        };
        if let Ok(mut exchanges) = self.exchanges.lock() {
            exchanges.push(exchange);
        }
        Ok(response)
    }
}

/// Keys whose values sign in or stand for a sign-in.
const SECRET_KEYS: &[(&str, &str)] = &[
    ("access_token", "token"),
    ("refresh_token", "refresh-token"),
    ("token", "token"),
    ("private_token", "token"),
    ("id_token", "token"),
    ("client_secret", "secret"),
    ("password", "password"),
    ("device_code", "device-code"),
    ("user_code", "user-code"),
    // A GitLab project's token for registering runners.
    ("runners_token", "runners-token"),
];

/// Keys whose number is a person's id wherever they are.
const PERSON_ID_KEYS: &[&str] = &[
    "user_id",
    "author_id",
    "owner_id",
    "creator_id",
    "assignee_id",
    "assignee_ids",
    "reviewer_ids",
    "merge_user_id",
];

/// What a person's object says of them besides who they are: emptied.
const PERSONAL_KEYS: &[&str] = &[
    "bio",
    "blog",
    "company",
    "discord",
    "github",
    "job_title",
    "linkedin",
    "location",
    "organization",
    "pronouns",
    "skype",
    "twitter",
    "twitter_username",
    "website_url",
];

/// Times and counts of a person's account: a time becomes 2000-01-01, a
/// count nothing.
const PERSON_FACTS: &[&str] = &[
    "collaborators",
    "confirmed_at",
    "created_at",
    "current_sign_in_at",
    "disk_usage",
    "followers",
    "following",
    "last_activity_on",
    "last_sign_in_at",
    "owned_private_repos",
    "private_gists",
    "public_gists",
    "public_repos",
    "total_private_repos",
    "updated_at",
];

/// What [`Scrubber::rewrite`] is told of a number that is a person's id.
const PERSON: &str = "person";

/// Keys of a person's address.
const EMAIL_KEYS: &[&str] = &[
    "email",
    "public_email",
    "commit_email",
    "notification_email",
];

/// Keys of the network addresses a person was seen at (GitLab names
/// where a token was last used): emptied wherever they are.
const NETWORK_KEYS: &[&str] = &[
    "current_sign_in_ip",
    "ip_address",
    "last_sign_in_ip",
    "last_used_ips",
];

/// What [`Scrubber::rewrite`] is told of a number that is the id of
/// something else than a person.
const THING: &str = "thing";

/// The stand-ins of the ids of things are this and one, two, ….
const THING_BASE: u64 = 300_000;

/// The smallest id that is replaced: a smaller one is found everywhere
/// by chance, and is no lead to anything (a GitLab of one's own counts
/// its projects from 1).
const SMALLEST_ID: u64 = 10_000;

/// The fewest digits a number in an address has to have to be taken for
/// an id there (a run's, a job's) when no answer names it as one.
const ADDRESS_ID_DIGITS: usize = 7;

/// The ids of things and the node ids, and their stand-ins.
#[derive(Clone, Default)]
struct Book {
    ids: BTreeMap<u64, u64>,
    nodes: BTreeMap<String, String>,
}

/// The book the recordings of one run share (see [`Recorder::finish`]).
static BOOK: Mutex<Book> = Mutex::new(Book {
    ids: BTreeMap::new(),
    nodes: BTreeMap::new(),
});

impl Book {
    /// The stand-in of the id of a thing.
    fn id(&mut self, real: u64) -> u64 {
        let next = THING_BASE
            .saturating_add(u64::try_from(self.ids.len()).unwrap_or(u64::MAX))
            .saturating_add(1);
        *self.ids.entry(real).or_insert(next)
    }

    /// The stand-in of a node id of the kind `kind` (`PR`; empty for one
    /// of the older shape).
    fn node(&mut self, real: &str, kind: &str) -> String {
        if let Some(known) = self.nodes.get(real) {
            return known.clone();
        }
        let number = self.nodes.len().saturating_add(1);
        let stand_in = if kind.is_empty() {
            format!("node-{number}")
        } else {
            format!("{kind}_node-{number}")
        };
        self.nodes.insert(real.to_owned(), stand_in.clone());
        stand_in
    }
}

/// Whether a key's number (or text of digits) is the id of something:
/// `id`, `project_id`, `assignee_ids`, `databaseId`. One that ends in
/// `iid` is a number within a project.
fn names_an_id(key: &str) -> bool {
    !key.ends_with("iid")
        && (key == "id" || key.ends_with("_id") || key.ends_with("_ids") || key.ends_with("Id"))
}

/// The kind of a GitHub node id: `PR` of `PR_kwDOAbCd…` (the letters of a
/// type, then base64 of a MessagePack array, which begins with `k`), or
/// nothing for one of the older shape (base64 of `012:IssueComment1`);
/// `None` when the text is not a node id.
fn node_kind(text: &str) -> Option<&str> {
    if let Some((kind, rest)) = text.split_once('_')
        && (1..=8).contains(&kind.len())
        && kind.bytes().all(|byte| byte.is_ascii_uppercase())
        && rest.len() >= 8
        && rest.starts_with('k')
        && rest
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
    {
        return Some(kind);
    }
    if text.len() < 8
        || !text.starts_with("MD")
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'/' | b'='))
    {
        return None;
    }
    use base64::Engine as _;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text)
        .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(text))
        .ok()?;
    let decoded = String::from_utf8(decoded).ok()?;
    let (length, rest) = decoded.split_once(':')?;
    let older = length.starts_with('0')
        && length.bytes().all(|byte| byte.is_ascii_digit())
        && rest
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic());
    older.then_some("")
}

/// Whether a text is lower-case hexadecimal digits only.
fn lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Whether a commit id is one a test made up rather than a real one: a
/// pattern of sixteen digits or fewer repeated (all `0`,
/// `0123456789abcdef0123…`).
fn made_up_commit(sha: &str) -> bool {
    let bytes = sha.as_bytes();
    (1..=16).any(|period| {
        bytes
            .iter()
            .enumerate()
            .all(|(at, byte)| bytes.get(at % period) == Some(byte))
    })
}

/// The stand-in of a commit id: forty hexadecimal digits of a digest of
/// it, the same in every recording, from which it cannot be found again.
fn commit_stand_in(real: &str) -> String {
    use sha2::Digest as _;
    let digest = sha2::Sha256::digest(format!("bitbraid-recording:{real}").as_bytes());
    let hex: String = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    hex.get(..40).unwrap_or_default().to_owned()
}

/// The runs of ASCII letters and digits in a text, with where each
/// begins.
fn word_runs(text: &str) -> Vec<(usize, &str)> {
    let mut runs = Vec::new();
    let mut start = None;
    for (at, byte) in text
        .bytes()
        .enumerate()
        .chain(std::iter::once((text.len(), b' ')))
    {
        match (start, word_byte(byte)) {
            (None, true) => start = Some(at),
            (Some(from), false) => {
                if let Some(run) = text.get(from..at) {
                    runs.push((from, run));
                }
                start = None;
            }
            _ => {}
        }
    }
    runs
}

/// The numbers in a run of letters and digits that can be ids: runs of
/// digits with no hexadecimal letter beside them (`2400000078` of
/// `r2400000078`, none of a commit id's), with where each begins.
fn id_numbers(run: &str) -> Vec<(usize, &str)> {
    let bytes = run.as_bytes();
    let hex_letter = |at: Option<usize>| {
        at.and_then(|at| bytes.get(at))
            .is_some_and(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_digit())
    };
    let mut numbers = Vec::new();
    let mut at = 0usize;
    while at < bytes.len() {
        if !bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at = at.saturating_add(1);
            continue;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_digit) {
            at = at.saturating_add(1);
        }
        if !hex_letter(start.checked_sub(1))
            && !hex_letter(Some(at))
            && let Some(digits) = run.get(start..at)
        {
            numbers.push((start, digits));
        }
    }
    numbers
}

/// What is replaced in a recording, and by what.
#[derive(Default)]
struct Scrubber {
    /// Texts, longest first when used.
    texts: Vec<(String, String)>,
    numbers: BTreeMap<u64, u64>,
    people: usize,
    organisations: usize,
    secrets: BTreeMap<&'static str, usize>,
    /// The ids of things and the node ids.
    book: Book,
    /// The commit ids seen, for their abbreviations.
    commits: BTreeSet<String>,
}

/// The kind and number of a stand-in login: `user-2` is a person's
/// second, `org-1` an organisation's first.
fn stand_in_parts(stand_in: &str) -> Option<(&'static str, usize)> {
    let (kind, number) = stand_in.rsplit_once('-')?;
    let kind = match kind {
        "user" => "user",
        "org" => "org",
        _ => return None,
    };
    Some((kind, number.parse().ok()?))
}

/// What the stand-in of a kind and number is called: `User 2`, `Org 1`.
fn stand_in_name(kind: &str, number: usize) -> String {
    match kind {
        "org" => format!("Org {number}"),
        _ => format!("User {number}"),
    }
}

/// The numeric id of the stand-in of a kind and number.
fn stand_in_id(kind: &str, number: usize) -> u64 {
    let base: u64 = if kind == "org" { 200_000 } else { 100_000 };
    base.saturating_add(u64::try_from(number).unwrap_or(0))
}

/// The login inside an address GitHub made for an account
/// (`ID+login@users.noreply.github.com`).
fn noreply_login(email: &str) -> Option<&str> {
    let local = email.strip_suffix("@users.noreply.github.com")?;
    Some(local.split_once('+').map_or(local, |(_, login)| login))
}

impl Scrubber {
    fn text(&mut self, real: &str, stand_in: String) {
        let real = real.trim();
        if real.len() < 2 || self.texts.iter().any(|(known, _)| known == real) {
            return;
        }
        self.texts.push((real.to_owned(), stand_in));
    }

    fn stand_in_for(&self, real: &str) -> Option<&str> {
        self.texts
            .iter()
            .find(|(known, _)| known == real)
            .map(|(_, stand_in)| stand_in.as_str())
    }

    fn secret(&mut self, kind: &'static str, real: &str) {
        if real.is_empty() || self.stand_in_for(real).is_some() {
            return;
        }
        let count = self.secrets.entry(kind).or_default();
        *count = count.saturating_add(1);
        let stand_in = format!("{kind}-{count}");
        self.texts.push((real.to_owned(), stand_in));
    }

    /// A login whose stand-in the caller chose.
    fn named(&mut self, login: &str, stand_in: &str) {
        self.text(login, stand_in.to_owned());
        if let Some((kind, number)) = stand_in_parts(stand_in) {
            let count = if kind == "org" {
                &mut self.organisations
            } else {
                &mut self.people
            };
            *count = (*count).max(number);
        }
    }

    /// A person or an organisation: the login is the key to the others.
    fn person(&mut self, object: &Map<String, Value>) {
        let login = ["login", "username"]
            .iter()
            .find_map(|key| object.get(*key).and_then(Value::as_str));
        let Some(login) = login else {
            return;
        };
        let (kind, number) = match self.stand_in_for(login.trim()) {
            Some(known) => stand_in_parts(known).unwrap_or(("user", 0)),
            None if object.get("type").and_then(Value::as_str) == Some("Organization") => {
                self.organisations = self.organisations.saturating_add(1);
                self.text(login, format!("org-{}", self.organisations));
                ("org", self.organisations)
            }
            None => {
                self.people = self.people.saturating_add(1);
                self.text(login, format!("user-{}", self.people));
                ("user", self.people)
            }
        };
        if let Some(name) = object.get("name").and_then(Value::as_str) {
            self.text(name, stand_in_name(kind, number));
        }
        for key in EMAIL_KEYS {
            if let Some(email) = object.get(*key).and_then(Value::as_str) {
                self.text(email, format!("{kind}-{number}@example.invalid"));
            }
        }
        if let Some(id) = object.get("id").and_then(Value::as_u64) {
            self.number(id, stand_in_id(kind, number));
        }
    }

    /// A person as a commit names them: a name and an address, without a
    /// login. The address is the key to the name.
    fn identity(&mut self, name: Option<&str>, email: &str) {
        // Not a person: GitHub itself, which commits what is made on its
        // site, and whose name is in every address of its answers.
        if !email.contains('@') || email.eq_ignore_ascii_case("noreply@github.com") {
            return;
        }
        // An address GitHub made for an account known here: its id and
        // login are replaced in it, and its name is the account's.
        if let Some(login) = noreply_login(&email.to_ascii_lowercase())
            && let Some((kind, number)) = self
                .texts
                .iter()
                .find(|(known, _)| known.eq_ignore_ascii_case(login))
                .and_then(|(_, stand_in)| stand_in_parts(stand_in))
        {
            if let Some(name) = name {
                self.text(name, stand_in_name(kind, number));
            }
            return;
        }
        let number = match self.stand_in_for(email) {
            Some(known) => known
                .strip_prefix("user-")
                .and_then(|rest| rest.split('@').next())
                .and_then(|number| number.parse::<usize>().ok())
                .unwrap_or(0),
            None => {
                self.people = self.people.saturating_add(1);
                self.text(email, format!("user-{}@example.invalid", self.people));
                self.people
            }
        };
        if let Some(name) = name {
            self.text(name, format!("User {number}"));
        }
    }

    /// A person's numeric id: as a number, and, when it is long enough not
    /// to be found by chance, inside texts too (a picture's address).
    fn number(&mut self, real: u64, stand_in: u64) {
        let stand_in = *self.numbers.entry(real).or_insert(stand_in);
        if real >= 10_000 {
            self.text(&real.to_string(), stand_in.to_string());
        }
    }

    /// Learn what is in `value`.
    fn learn(&mut self, value: &Value) {
        match value {
            Value::Object(object) => {
                self.person(object);
                let text = |key: &str| object.get(key).and_then(Value::as_str);
                if text("login").is_none()
                    && text("username").is_none()
                    && let Some(email) = text("email")
                {
                    self.identity(text("name"), email);
                }
                for (name, email) in [
                    ("author_name", "author_email"),
                    ("committer_name", "committer_email"),
                ] {
                    if let Some(email) = text(email) {
                        self.identity(text(name), email);
                    }
                }
                for (key, inner) in object {
                    if let (Some((_, kind)), Some(text)) = (
                        SECRET_KEYS.iter().find(|(secret, _)| secret == key),
                        inner.as_str(),
                    ) {
                        self.secret(kind, text);
                    }
                    self.learn(inner);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| self.learn(item)),
            Value::String(text) => {
                // A form's fields.
                for (name, field) in form_fields(text) {
                    if let Some((_, kind)) = SECRET_KEYS.iter().find(|(secret, _)| *secret == name)
                    {
                        self.secret(kind, &field);
                    }
                }
            }
            _ => {}
        }
    }

    /// The id of a thing, unless it is small or a person's.
    fn thing(&mut self, real: u64) {
        if real >= SMALLEST_ID && !self.numbers.contains_key(&real) {
            self.book.id(real);
        }
    }

    /// Learn the ids under a key that names one.
    fn learn_id(&mut self, value: &Value) {
        match value {
            Value::Number(number) => {
                if let Some(real) = number.as_u64() {
                    self.thing(real);
                }
            }
            Value::String(text) if !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit()) => {
                if let Ok(real) = text.parse() {
                    self.thing(real);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| self.learn_id(item)),
            _ => {}
        }
    }

    /// Learn the commit ids in a text, and, in an address, the long
    /// numbers, which are ids.
    fn learn_text(&mut self, text: &str, address: bool) {
        for (_, run) in word_runs(text) {
            if run.len() == 40 && lower_hex(run) && !made_up_commit(run) {
                self.commits.insert(run.to_owned());
            }
            if address {
                for (_, digits) in id_numbers(run) {
                    if digits.len() >= ADDRESS_ID_DIGITS
                        && let Ok(real) = digits.parse()
                    {
                        self.thing(real);
                    }
                }
            }
        }
    }

    /// Learn the ids of things in `value`, once its people are known.
    fn learn_ids(&mut self, value: &Value) {
        match value {
            Value::Object(object) => {
                let person = object.contains_key("login") || object.contains_key("username");
                for (key, inner) in object {
                    if names_an_id(key)
                        && !(person && key == "id")
                        && !PERSON_ID_KEYS.contains(&key.as_str())
                    {
                        self.learn_id(inner);
                    }
                    self.learn_ids(inner);
                }
            }
            Value::Array(items) => items.iter().for_each(|item| self.learn_ids(item)),
            Value::String(text) => {
                let address = text.contains("://") || text.starts_with('/');
                self.learn_text(text, address);
            }
            _ => {}
        }
    }

    /// The full commit id a run of hexadecimal digits abbreviates, when
    /// it is the one seen that begins so.
    fn abbreviated(&self, run: &str) -> Option<&str> {
        let mut found = self
            .commits
            .range(run.to_owned()..)
            .take_while(|commit| commit.starts_with(run));
        let first = found.next()?;
        found.next().is_none().then_some(first.as_str())
    }

    /// A run of letters and digits with the ids and commit ids in it
    /// replaced.
    fn replace_run(&self, run: &str) -> String {
        if run.len() >= ADDRESS_ID_DIGITS && lower_hex(run) {
            if run.len() == 40 {
                return if made_up_commit(run) {
                    run.to_owned()
                } else {
                    commit_stand_in(run)
                };
            }
            let an_id = run
                .parse::<u64>()
                .is_ok_and(|number| self.book.ids.contains_key(&number));
            if !an_id && let Some(commit) = self.abbreviated(run) {
                return commit_stand_in(commit)
                    .get(..run.len())
                    .unwrap_or(run)
                    .to_owned();
            }
        }
        let mut out = String::with_capacity(run.len());
        let mut at = 0usize;
        for (start, digits) in id_numbers(run) {
            let stand_in = digits
                .parse::<u64>()
                .ok()
                .and_then(|number| self.book.ids.get(&number));
            if let Some(stand_in) = stand_in {
                out.push_str(run.get(at..start).unwrap_or_default());
                out.push_str(&stand_in.to_string());
                at = start.saturating_add(digits.len());
            }
        }
        out.push_str(run.get(at..).unwrap_or_default());
        out
    }

    /// Replace the ids of things, node ids and commit ids in a text.
    fn replace_ids(&mut self, text: &str) -> String {
        if let Some(kind) = node_kind(text) {
            return self.book.node(text, kind);
        }
        let mut out = String::with_capacity(text.len());
        let mut at = 0usize;
        for (start, run) in word_runs(text) {
            out.push_str(text.get(at..start).unwrap_or_default());
            out.push_str(&self.replace_run(run));
            at = start.saturating_add(run.len());
        }
        out.push_str(text.get(at..).unwrap_or_default());
        out
    }

    /// A text with what was learnt in it replaced: people and secrets,
    /// then ids.
    fn scrubbed(&mut self, text: &str) -> String {
        if node_kind(text).is_some() {
            return self.replace_ids(text);
        }
        let replaced = self.replace(text);
        self.replace_ids(&replaced)
    }

    /// Replace what was learnt in a text: whole words only, so that a
    /// login does not eat into a longer one.
    fn replace(&self, text: &str) -> String {
        let mut sorted: Vec<&(String, String)> = self.texts.iter().collect();
        sorted.sort_by_key(|(real, _)| std::cmp::Reverse(real.len()));
        let mut out = text.to_owned();
        for (real, stand_in) in sorted {
            out = replace_words(&out, real, stand_in);
        }
        out
    }

    fn rewrite(&mut self, value: &mut Value, key: Option<&str>) {
        match value {
            Value::Object(object) => {
                let person = object.contains_key("login") || object.contains_key("username");
                if person {
                    if let Some(Value::String(node)) = object.get_mut("node_id") {
                        let kind = node_kind(node).unwrap_or_default();
                        *node = self.book.node(node, kind);
                    }
                    let login = ["login", "username"]
                        .iter()
                        .find_map(|key| object.get(*key).and_then(Value::as_str))
                        .map(|login| self.replace(login));
                    if let Some(login) = login {
                        for key in ["avatar_url", "avatarUrl"] {
                            if object.contains_key(key) {
                                object.insert(
                                    key.to_owned(),
                                    Value::String(format!(
                                        "https://avatars.example.invalid/{login}"
                                    )),
                                );
                            }
                        }
                        if object.contains_key("gravatar_id") {
                            object.insert("gravatar_id".to_owned(), Value::String(String::new()));
                        }
                    }
                    // What else a person says of themselves is left out,
                    // and what could find their account again is made
                    // the same for everyone.
                    for key in PERSONAL_KEYS {
                        if let Some(Value::String(text)) = object.get_mut(*key)
                            && !text.is_empty()
                        {
                            text.clear();
                        }
                    }
                    for key in PERSON_FACTS {
                        match object.get_mut(*key) {
                            Some(Value::String(text)) if !text.is_empty() => {
                                *text = "2000-01-01T00:00:00Z".to_owned();
                            }
                            Some(Value::Number(number)) => *number = serde_json::Number::from(0),
                            _ => {}
                        }
                    }
                }
                // A person's namespace at GitLab (`"kind": "user"`) shows
                // their picture: the stand-in's.
                let namespace_of = (!person
                    && object.get("kind").and_then(Value::as_str) == Some("user"))
                .then(|| {
                    ["full_path", "path"]
                        .iter()
                        .find_map(|key| object.get(*key).and_then(Value::as_str))
                        .map(|path| self.replace(path))
                })
                .flatten();
                if let Some(login) = namespace_of
                    && object.contains_key("avatar_url")
                {
                    object.insert(
                        "avatar_url".to_owned(),
                        Value::String(format!("https://avatars.example.invalid/{login}")),
                    );
                }
                for key in NETWORK_KEYS {
                    match object.get_mut(*key) {
                        Some(Value::String(text)) => text.clear(),
                        Some(Value::Array(items)) => items.clear(),
                        _ => {}
                    }
                }
                for (inner_key, inner) in object.iter_mut() {
                    // A number is a person's id as a person's `id`, or
                    // under a key that names a person's id elsewhere; the
                    // id of a thing under another key that names an id.
                    let names_person = (person && inner_key == "id")
                        || PERSON_ID_KEYS.contains(&inner_key.as_str());
                    let kind = if names_person {
                        Some(PERSON)
                    } else if names_an_id(inner_key) {
                        Some(THING)
                    } else {
                        None
                    };
                    self.rewrite(inner, kind);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.rewrite(item, key);
                }
            }
            Value::String(text) => *text = self.scrubbed(text),
            Value::Number(number) => {
                let stand_in = match (key, number.as_u64()) {
                    (Some(PERSON), Some(real)) => self.numbers.get(&real),
                    (Some(THING), Some(real)) => {
                        self.book.ids.get(&real).or_else(|| self.numbers.get(&real))
                    }
                    _ => None,
                };
                if let Some(stand_in) = stand_in {
                    *number = serde_json::Number::from(*stand_in);
                }
            }
            _ => {}
        }
    }
}

fn word_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric()
}

/// Whether the three bytes before `at` are a percent-escape (`%2F` of
/// `user-1%2Fproject`), which ends a word as the byte it stands for does.
fn after_an_escape(bytes: &[u8], at: usize) -> bool {
    at.checked_sub(3)
        .and_then(|from| bytes.get(from..at))
        .is_some_and(|escape| {
            escape.first() == Some(&b'%') && escape.iter().skip(1).all(u8::is_ascii_hexdigit)
        })
}

/// `text` with every whole-word `real` (case aside) as `stand_in`.
fn replace_words(text: &str, real: &str, stand_in: &str) -> String {
    if real.is_empty() {
        return text.to_owned();
    }
    let lower_text = text.to_lowercase();
    let lower_real = real.to_lowercase();
    // Lower-casing can change lengths outside ASCII: then only the exact
    // spelling is looked for.
    let (haystack, needle) = if lower_text.len() == text.len() && lower_real.len() == real.len() {
        (lower_text.as_str(), lower_real.as_str())
    } else {
        (text, real)
    };
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut at = 0usize;
    while let Some(found) = haystack.get(at..).and_then(|rest| rest.find(needle)) {
        let start = at.saturating_add(found);
        let end = start.saturating_add(needle.len());
        let before = start
            .checked_sub(1)
            .and_then(|before| bytes.get(before))
            .is_some_and(|byte| word_byte(*byte))
            && !after_an_escape(bytes, start);
        let after = bytes.get(end).is_some_and(|byte| word_byte(*byte));
        out.push_str(text.get(at..start).unwrap_or_default());
        if before || after {
            out.push_str(text.get(start..end).unwrap_or_default());
        } else {
            out.push_str(stand_in);
        }
        at = end;
    }
    out.push_str(text.get(at..).unwrap_or_default());
    out
}

/// Whether a real value is still anywhere in a written recording, even
/// inside a longer word: a false alarm costs a recording, a miss a leak.
/// A number is looked for as a number of its own, and one of fewer than
/// five digits not at all (it would be found everywhere).
fn still_there(text: &str, real: &str) -> bool {
    let real = real.trim();
    if real.is_empty() {
        return false;
    }
    if real.bytes().all(|b| b.is_ascii_digit()) {
        return real.len() >= 5 && replace_words(text, real, "\u{0}") != text;
    }
    text.to_lowercase().contains(&real.to_lowercase())
}

/// Scrub a recording in place. `real` are values that have to go in any
/// case: the first is the login of user-1; the others, where the answers
/// do not name them as someone's, are taken for user-1's too.
#[cfg(test)]
pub fn scrub(recording: &mut Recording, real: &[&str]) {
    scrub_named(recording, &[], real);
}

/// Scrub a recording in place, the stand-ins of the logins in `named`
/// chosen by the caller (`("someone", "user-2")`, `("an-org", "org-1")`).
/// With none named, it is [`scrub`]; with some, every one of `real` is
/// what has to go, the first too.
pub fn scrub_named(recording: &mut Recording, named: &[(&str, &str)], real: &[&str]) {
    scrub_with_book(recording, named, real, &mut Book::default());
}

/// [`scrub_named`], the stand-ins of ids taken from `book` and the new
/// ones added to it.
fn scrub_with_book(
    recording: &mut Recording,
    named: &[(&str, &str)],
    real: &[&str],
    book: &mut Book,
) {
    let mut scrubber = Scrubber {
        book: std::mem::take(book),
        ..Scrubber::default()
    };
    for (login, stand_in) in named {
        scrubber.named(login, stand_in);
    }
    // Without names, the caller's login first, as user-1's, so that it is
    // replaced even where no answer names it as a person.
    let rest = match real.split_first() {
        Some((login, rest)) if named.is_empty() => {
            scrubber.people = 1;
            scrubber.text(login, "user-1".to_owned());
            rest
        }
        _ => real,
    };
    for exchange in &recording.exchanges {
        scrubber.learn(&exchange.request_body);
        scrubber.learn(&exchange.body);
    }
    // What the answers did not name as anyone's.
    for value in rest {
        let value = value.trim();
        if let Ok(id) = value.parse::<u64>() {
            if !scrubber.numbers.contains_key(&id) {
                scrubber.number(id, 100_001);
            }
        } else if scrubber.stand_in_for(value).is_some() {
            continue;
        } else if value.contains('@') {
            scrubber.text(value, "user-1@example.invalid".to_owned());
        } else {
            scrubber.text(value, "User 1".to_owned());
        }
    }
    // The ids of things, once the people's are known; an address in a
    // header names them too (a page's `link`).
    for exchange in &recording.exchanges {
        scrubber.learn_ids(&exchange.request_body);
        scrubber.learn_ids(&exchange.body);
        scrubber.learn_text(&exchange.path_and_query, false);
        for (name, value) in &exchange.headers {
            let address = matches!(name.to_ascii_lowercase().as_str(), "link" | "location");
            scrubber.learn_text(value, address);
        }
    }
    for exchange in &mut recording.exchanges {
        exchange.path_and_query = scrubber.scrubbed(&exchange.path_and_query);
        scrubber.rewrite(&mut exchange.request_body, None);
        scrubber.rewrite(&mut exchange.body, None);
        for value in exchange.headers.values_mut() {
            *value = scrubber.scrubbed(value);
        }
        exchange.headers.remove("set-cookie");
    }
    *book = scrubber.book;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::http::{Limits, Url};
    use serde_json::json;

    fn exchange(method: Method, path: &str, request: Value, status: u16, body: Value) -> Exchange {
        Exchange {
            method,
            path_and_query: path.to_owned(),
            request_body: request,
            status,
            headers: BTreeMap::new(),
            body,
        }
    }

    fn call(method: Method, path: &str, body: Option<&str>) -> Call {
        let mut call = Call::new(
            method,
            Url::parse(&format!("https://api.example.org{path}")).unwrap(),
            Limits {
                connect: std::time::Duration::from_secs(1),
                total: std::time::Duration::from_secs(1),
                body: 1 << 20,
            },
        );
        if let Some(body) = body {
            call = call.with_body(
                "application/x-www-form-urlencoded",
                body.as_bytes().to_vec(),
            );
        }
        call
    }

    #[test]
    fn requests_are_answered_in_the_order_they_were_recorded() {
        let replay = Replay::new(Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/user",
                    Value::Null,
                    200,
                    json!({"login": "a"}),
                ),
                exchange(
                    Method::Post,
                    "/oauth/token",
                    json!("device_code=d1&client_id=c"),
                    400,
                    json!({"error": "authorization_pending"}),
                ),
                exchange(
                    Method::Post,
                    "/oauth/token",
                    json!("device_code=d1&client_id=c"),
                    200,
                    json!({"access_token": "t"}),
                ),
            ],
        });
        let go = AtomicBool::new(false);
        let user = replay.send(&call(Method::Get, "/user", None), &go).unwrap();
        assert_eq!(user.body, br#"{"login":"a"}"#);
        // The fields of a form in another order are the same form.
        let first = replay
            .send(
                &call(
                    Method::Post,
                    "/oauth/token",
                    Some("client_id=c&device_code=d1"),
                ),
                &go,
            )
            .unwrap();
        assert_eq!(first.status, 400);
        let second = replay
            .send(
                &call(
                    Method::Post,
                    "/oauth/token",
                    Some("device_code=d1&client_id=c"),
                ),
                &go,
            )
            .unwrap();
        assert_eq!(second.status, 200);
        // Used up: the last is answered again.
        let again = replay
            .send(
                &call(
                    Method::Post,
                    "/oauth/token",
                    Some("device_code=d1&client_id=c"),
                ),
                &go,
            )
            .unwrap();
        assert_eq!(again.status, 200);
        assert!(replay.unused().is_empty());
        // Another body, path or method was not recorded.
        for (method, path, body) in [
            (
                Method::Post,
                "/oauth/token",
                Some("device_code=other&client_id=c"),
            ),
            (Method::Get, "/user?x=1", None),
            (Method::Delete, "/user", None),
        ] {
            assert!(matches!(
                replay.send(&call(method, path, body), &go),
                Err(FetchError::Unrecorded(named)) if named.contains(path.split('?').next().unwrap())
            ));
        }
        assert_eq!(replay.asked().len(), 7);
    }

    #[test]
    fn people_and_secrets_are_scrubbed_the_same_way_throughout() {
        let mut recording = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/user",
                    Value::Null,
                    200,
                    json!({"login": "RealPerson", "id": 4242424, "node_id": "MDQ6VXNlcjE=",
                           "name": "Real Name", "email": "real@example.com",
                           "avatar_url": "https://avatars.githubusercontent.com/u/4242424?v=4",
                           "html_url": "https://github.com/RealPerson"}),
                ),
                exchange(
                    Method::Get,
                    "/repos/RealPerson/bitbraid-test-core/pulls",
                    Value::Null,
                    200,
                    json!([{"user": {"login": "realperson", "id": 4242424},
                            "head": {"label": "RealPerson:topic",
                                     "repo": {"full_name": "RealPerson/bitbraid-test-core",
                                              "owner": {"login": "RealPerson", "id": 4242424}}},
                            "body": "by Real Name, see https://github.com/realperson"}]),
                ),
                exchange(
                    Method::Post,
                    "/login/oauth/access_token",
                    json!("client_id=abc&device_code=secret-device&grant_type=x"),
                    200,
                    json!({"access_token": "gho_realtoken", "refresh_token": "r-real"}),
                ),
            ],
        };
        recording.exchanges[0].headers.insert(
            "link".into(),
            "<https://api.github.com/user/RealPerson?page=2>; rel=\"next\"".into(),
        );
        scrub(
            &mut recording,
            &["RealPerson", "4242424", "Real Name", "real@example.com"],
        );
        let text = serde_json::to_string(&recording).unwrap();
        for real in [
            "RealPerson",
            "4242424",
            "Real Name",
            "real@example.com",
            "gho_realtoken",
            "r-real",
            "secret-device",
            "MDQ6",
        ] {
            assert!(!text.contains(real), "{real} is still in {text}");
        }
        // Words that only start like a login are left.
        assert_eq!(
            replace_words("RealPersonal realperson2 RealPerson/x", "RealPerson", "u"),
            "RealPersonal realperson2 u/x"
        );
        // After a percent-escape a word begins, as after what it stands for.
        assert_eq!(
            replace_words(
                "o%2FRealPerson repo%3Arealperson x%2RealPerson",
                "RealPerson",
                "u"
            ),
            "o%2Fu repo%3Au x%2RealPerson"
        );
        assert!(text.contains("see https://github.com/user-1"), "{text}");
        assert_eq!(
            recording.exchanges[1].path_and_query,
            "/repos/user-1/bitbraid-test-core/pulls"
        );
        let me = &recording.exchanges[0].body;
        assert_eq!(me["login"], "user-1");
        assert_eq!(me["id"], 100_001);
        assert_eq!(me["name"], "User 1");
        assert_eq!(me["email"], "user-1@example.invalid");
        assert_eq!(me["avatar_url"], "https://avatars.example.invalid/user-1");
        assert_eq!(me["html_url"], "https://github.com/user-1");
        assert_eq!(
            recording.exchanges[1].body[0]["head"]["label"],
            "user-1:topic"
        );
        assert_eq!(recording.exchanges[2].body["access_token"], "token-1");
        assert_eq!(
            recording.exchanges[2].request_body,
            "client_id=abc&device_code=device-code-1&grant_type=x"
        );
        assert!(recording.exchanges[0].headers["link"].contains("/user/user-1?"));
    }

    #[test]
    fn every_person_and_every_author_of_a_commit_has_a_stand_in_of_their_own() {
        let mut recording = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/api/v4/user",
                    Value::Null,
                    200,
                    json!({ "username": "alice-real", "id": 11111, "name": "Alice Real",
                            "email": "alice@real.example" }),
                ),
                exchange(
                    Method::Get,
                    "/api/v4/projects/bob-real%2Fp/merge_requests",
                    Value::Null,
                    200,
                    json!([{ "author": { "username": "bob-real", "id": 22222, "name": "Bob Real",
                                         "avatarUrl": "https://real.example/22222.png" },
                             "web_url": "http://h/bob-real/p/-/merge_requests/1" }]),
                ),
                exchange(
                    Method::Get,
                    "/repos/x/y/commits",
                    Value::Null,
                    200,
                    json!([{ "commit": {
                        "author": { "name": "Carol Real", "email": "carol@real.example",
                                    "date": "2026-10-12T10:30:00Z" },
                        "verification": { "payload": "author Carol Real <carol@real.example> 1\n" } } }]),
                ),
                exchange(
                    Method::Get,
                    "/api/v4/projects/1/repository/commits",
                    Value::Null,
                    200,
                    json!([{ "author_name": "Dan Real", "author_email": "dan@real.example",
                             "committer_name": "Carol Real", "committer_email": "carol@real.example" }]),
                ),
            ],
        };
        scrub(
            &mut recording,
            &[
                "alice-real",
                "11111",
                "Alice Real",
                "alice@real.example",
                "bob-real",
                "22222",
                "Bob Real",
                "bob@real.example",
            ],
        );
        let text = serde_json::to_string(&recording).unwrap();
        for real in [
            "alice",
            "bob",
            "carol",
            "dan",
            "Real",
            "11111",
            "22222",
            "real.example",
        ] {
            assert!(!text.contains(real), "{real} is still in {text}");
        }
        let bob = &recording.exchanges[1];
        assert_eq!(
            bob.path_and_query,
            "/api/v4/projects/user-2%2Fp/merge_requests"
        );
        assert_eq!(bob.body[0]["author"]["username"], "user-2");
        assert_eq!(bob.body[0]["author"]["id"], 100_002);
        assert_eq!(bob.body[0]["author"]["name"], "User 2");
        assert_eq!(
            bob.body[0]["author"]["avatarUrl"],
            "https://avatars.example.invalid/user-2"
        );
        let carol = &recording.exchanges[2].body[0]["commit"];
        assert_eq!(carol["author"]["name"], "User 3");
        assert_eq!(carol["author"]["email"], "user-3@example.invalid");
        assert_eq!(
            carol["verification"]["payload"],
            "author User 3 <user-3@example.invalid> 1\n"
        );
        let dan = &recording.exchanges[3].body[0];
        assert_eq!(dan["author_name"], "User 4");
        assert_eq!(dan["committer_email"], "user-3@example.invalid");
    }

    #[test]
    fn named_logins_organisations_and_githubs_addresses_keep_their_stand_ins() {
        let mut recording = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/repos/real-org/project",
                    Value::Null,
                    200,
                    json!({ "full_name": "real-org/project",
                            "owner": { "login": "real-org", "id": 7777777, "type": "Organization",
                                       "avatar_url": "https://avatars.githubusercontent.com/u/7777777" } }),
                ),
                exchange(
                    Method::Get,
                    "/repos/real-org/project/commits",
                    Value::Null,
                    200,
                    json!([{ "commit": { "author": {
                                 "name": "real-bot",
                                 "email": "5555555+real-bot@users.noreply.github.com" } },
                             "author": { "login": "real-bot", "id": 5555555 } },
                           { "user": { "login": "real-person", "id": 6666666, "name": "Real Person" },
                             "body": "seen by real-bot and Real Person" }]),
                ),
                exchange(
                    Method::Post,
                    "/login/device/code",
                    json!("client_id=abc&scope=repo"),
                    200,
                    json!({ "device_code": "3584d83530557fdd1f46af8289938c8ef79f9dc5",
                            "user_code": "WDJB-MJHT",
                            "verification_uri": "https://github.com/login/device" }),
                ),
            ],
        };
        scrub_named(
            &mut recording,
            &[("real-bot", "user-1"), ("real-person", "user-2")],
            &["real-bot", "5555555", "real-person", "6666666", "real-org"],
        );
        let text = serde_json::to_string(&recording).unwrap();
        for real in [
            "real-bot",
            "real-person",
            "Real Person",
            "real-org",
            "5555555",
            "6666666",
            "7777777",
            "WDJB-MJHT",
            "3584d835",
        ] {
            assert!(!text.contains(real), "{real} is still in {text}");
        }
        let org = &recording.exchanges[0];
        assert_eq!(org.path_and_query, "/repos/org-1/project");
        assert_eq!(org.body["owner"]["login"], "org-1");
        assert_eq!(org.body["owner"]["id"], 200_001);
        assert_eq!(
            org.body["owner"]["avatar_url"],
            "https://avatars.example.invalid/org-1"
        );
        let commits = &recording.exchanges[1].body;
        // GitHub's address for the account keeps its shape.
        assert_eq!(
            commits[0]["commit"]["author"]["email"],
            "100001+user-1@users.noreply.github.com"
        );
        assert_eq!(commits[0]["author"]["id"], 100_001);
        assert_eq!(commits[1]["user"]["login"], "user-2");
        assert_eq!(commits[1]["user"]["name"], "User 2");
        assert_eq!(commits[1]["body"], "seen by user-1 and User 2");
        let code = &recording.exchanges[2].body;
        assert_eq!(code["device_code"], "device-code-1");
        assert_eq!(code["user_code"], "user-code-1");
        // Without names, the first real login is user-1's.
        let mut plain = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![exchange(
                Method::Get,
                "/x",
                Value::Null,
                200,
                json!({ "text": "by first-login" }),
            )],
        };
        scrub(&mut plain, &["first-login"]);
        assert_eq!(plain.exchanges[0].body["text"], "by user-1");
    }

    #[test]
    fn where_a_token_was_used_and_a_gitlab_namespaces_picture_are_left_out() {
        let picture = "https://secure.gravatar.com/avatar/0a1b2c3d4e5f60718293a4b5c6d7e8f9?s=80";
        let mut recording = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/api/v4/user",
                    Value::Null,
                    200,
                    json!({ "username": "real-person", "id": 4242424, "name": "Real Person",
                            "avatar_url": picture, "current_sign_in_ip": "192.0.2.7" }),
                ),
                exchange(
                    Method::Get,
                    "/api/v4/personal_access_tokens/self",
                    Value::Null,
                    200,
                    json!({ "id": 9, "user_id": 4242424, "scopes": ["api"],
                            "last_used_ips": ["192.0.2.7", "2001:db8::7"] }),
                ),
                exchange(
                    Method::Get,
                    "/api/v4/namespaces",
                    Value::Null,
                    200,
                    json!([{ "id": 5, "name": "Real Person", "path": "real-person",
                             "kind": "user", "full_path": "real-person",
                             "avatar_url": picture,
                             "web_url": "https://gitlab.example.org/real-person" },
                           { "id": 6, "name": "A group", "path": "a-group", "kind": "group",
                             "full_path": "a-group",
                             "avatar_url": "https://gitlab.example.org/uploads/group.png" }]),
                ),
            ],
        };
        scrub_named(
            &mut recording,
            &[("real-person", "user-1")],
            &["real-person", "4242424", "Real Person"],
        );
        let text = serde_json::to_string(&recording).unwrap();
        for real in [
            "real-person",
            "Real Person",
            "192.0.2.7",
            "2001:db8",
            "0a1b2c3d",
        ] {
            assert!(!text.contains(real), "{real} is still in {text}");
        }
        assert_eq!(
            recording.exchanges[0].body["avatar_url"],
            "https://avatars.example.invalid/user-1"
        );
        assert_eq!(recording.exchanges[0].body["current_sign_in_ip"], "");
        assert_eq!(recording.exchanges[1].body["last_used_ips"], json!([]));
        let mine = &recording.exchanges[2].body[0];
        assert_eq!(mine["full_path"], "user-1");
        assert_eq!(mine["name"], "User 1");
        assert_eq!(mine["id"], 5);
        assert_eq!(mine["avatar_url"], "https://avatars.example.invalid/user-1");
        // A group's picture is not a person's: it stays.
        assert_eq!(
            recording.exchanges[2].body[1]["avatar_url"],
            "https://gitlab.example.org/uploads/group.png"
        );
    }

    #[test]
    fn ids_of_things_node_ids_and_commits_get_stand_ins_the_same_way_throughout() {
        let commit = "4f1c2e8a9b7d6c5e3f2a1b0c9d8e7f6a5b4c3d2e";
        let parent = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";
        let made_up = "0123456789abcdef0123456789abcdef01234567";
        let mut recording = Recording {
            note: String::new(),
            hand_made: None,
            exchanges: vec![
                exchange(
                    Method::Get,
                    "/repos/real-org/project/pulls",
                    Value::Null,
                    200,
                    json!([{ "id": 2_400_000_078_u64, "node_id": "PR_kwDOMadeUp8AAAABMadeUp",
                             "number": 8,
                             "head": { "sha": commit,
                                       "repo": { "id": 1_234_567_891_u64, "node_id": "R_kgDOMadeUp",
                                                 "full_name": "real-org/project",
                                                 "owner": { "login": "real-org", "id": 7_777_777,
                                                            "type": "Organization",
                                                            "node_id": "MDEyOk9yZ2FuaXphdGlvbjE=" } } },
                             "html_url": "https://github.com/real-org/project/pull/8#pullrequestreview-5550001111",
                             "body": format!("Fixes it in {} and {made_up}", &parent[..7]) }]),
                ),
                exchange(
                    Method::Get,
                    &format!("/repos/real-org/project/commits/{commit}/check-runs"),
                    Value::Null,
                    200,
                    json!({ "check_runs": [{ "pull_request_review_id": 5_550_001_111_u64,
                        "details_url": "https://github.com/real-org/project/actions/runs/38400000001/job/115400000002",
                        "parents": [{ "sha": parent }],
                        "comment": "https://github.com/real-org/project/pull/8#discussion_r2400000078",
                        "databaseId": "1234567891" }] }),
                ),
                exchange(
                    Method::Post,
                    "/graphql",
                    json!({ "variables": { "pullRequestId": "PR_kwDOMadeUp8AAAABMadeUp" } }),
                    200,
                    json!({ "data": { "markPullRequestReadyForReview": { "clientMutationId": null } } }),
                ),
                exchange(
                    Method::Get,
                    "/api/v4/projects/87000001/merge_requests/3",
                    Value::Null,
                    200,
                    json!({ "id": 547_000_003, "iid": 3, "project_id": 87_000_001,
                            "source_project_id": 87_000_001, "target_project_id": 87_000_004,
                            "author": { "username": "real-person", "id": 4_242_424 },
                            "head_pipeline": { "id": 2_900_000_005_u64, "project_id": 87_000_001,
                                               "web_url": "https://gitlab.com/real-person/project/-/pipelines/2900000005" },
                            "short_id": &commit[..8],
                            "service_desk_address": "contact-project+real-person-project-87000001-issue-@incoming.gitlab.com",
                            "runners_token": "GR1348941MadeUpForTheTest00",
                            "namespace": { "id": 28_000_006, "kind": "user", "full_path": "real-person" },
                            "local": { "id": 12, "project_id": 5 } }),
                ),
            ],
        };
        recording.exchanges[0].headers.insert(
            "link".into(),
            "<https://api.github.com/repositories/1234567891/pulls?page=2>; rel=\"next\"".into(),
        );
        scrub_named(
            &mut recording,
            &[("real-org", "org-1"), ("real-person", "user-1")],
            &["real-org", "real-person"],
        );
        let text = serde_json::to_string(&recording).unwrap();
        for real in [
            "2400000078",
            "1234567891",
            "5550001111",
            "38400000001",
            "115400000002",
            "547000003",
            "87000001",
            "87000004",
            "2900000005",
            "28000006",
            "kwDO",
            "kgDO",
            "MDEyOk9y",
            commit,
            &commit[..8],
            &parent[..7],
            "GR1348941",
            "real-org",
            "real-person",
        ] {
            assert!(!text.contains(real), "{real} is still in {text}");
        }
        let pulls = &recording.exchanges[0].body[0];
        let stand_in = |value: &Value| value.as_u64().unwrap();
        let pull = stand_in(&pulls["id"]);
        let repo = stand_in(&pulls["head"]["repo"]["id"]);
        assert!(pull > THING_BASE && repo > THING_BASE && pull != repo);
        assert_eq!(pulls["number"], 8);
        let node = |value: &Value| value.as_str().unwrap().to_owned();
        let pull_node = node(&pulls["node_id"]);
        assert!(pull_node.starts_with("PR_node-"), "{pull_node}");
        assert!(node(&pulls["head"]["repo"]["node_id"]).starts_with("R_node-"));
        assert!(node(&pulls["head"]["repo"]["owner"]["node_id"]).starts_with("node-"));
        assert_eq!(pulls["head"]["repo"]["owner"]["id"], 200_001);
        // The same commit is the same stand-in in a path and in a body, and
        // its abbreviation its stand-in shortened; a made-up one stays.
        let new_commit = pulls["head"]["sha"].as_str().unwrap();
        assert_eq!(new_commit, commit_stand_in(commit));
        assert_eq!(new_commit.len(), 40);
        assert!(lower_hex(new_commit));
        assert_eq!(
            recording.exchanges[1].path_and_query,
            format!("/repos/org-1/project/commits/{new_commit}/check-runs")
        );
        let new_parent = commit_stand_in(parent);
        assert_eq!(
            pulls["body"],
            format!("Fixes it in {} and {made_up}", &new_parent[..7])
        );
        let check = &recording.exchanges[1].body["check_runs"][0];
        assert_eq!(check["parents"][0]["sha"], new_parent.as_str());
        // A review's id is its stand-in in an address too, and so is a
        // comment's in an anchor; ids only an address names have their own.
        let review = stand_in(&check["pull_request_review_id"]);
        assert!(
            pulls["html_url"]
                .as_str()
                .unwrap()
                .ends_with(&format!("/org-1/project/pull/8#pullrequestreview-{review}"))
        );
        assert!(
            check["comment"]
                .as_str()
                .unwrap()
                .ends_with(&format!("#discussion_r{pull}"))
        );
        assert_eq!(check["databaseId"], repo.to_string());
        assert!(
            recording.exchanges[0].headers["link"]
                .contains(&format!("/repositories/{repo}/pulls?page=2"))
        );
        let details = check["details_url"].as_str().unwrap();
        assert!(details.starts_with("https://github.com/org-1/project/actions/runs/3"));
        // A node id sent back is the stand-in it was given in an answer.
        assert_eq!(
            recording.exchanges[2].request_body["variables"]["pullRequestId"],
            pull_node.as_str()
        );
        let merge = &recording.exchanges[3];
        let project = stand_in(&merge.body["project_id"]);
        assert_eq!(
            merge.path_and_query,
            format!("/api/v4/projects/{project}/merge_requests/3")
        );
        assert_eq!(merge.body["source_project_id"], project);
        assert_ne!(stand_in(&merge.body["target_project_id"]), project);
        assert_eq!(merge.body["iid"], 3);
        assert_eq!(merge.body["author"]["id"], 100_001);
        assert_eq!(merge.body["head_pipeline"]["project_id"], project);
        let pipeline = stand_in(&merge.body["head_pipeline"]["id"]);
        assert_eq!(
            merge.body["head_pipeline"]["web_url"],
            format!("https://gitlab.com/user-1/project/-/pipelines/{pipeline}")
        );
        assert_eq!(merge.body["short_id"], &new_commit[..8]);
        assert_eq!(
            merge.body["service_desk_address"],
            format!("contact-project+user-1-project-{project}-issue-@incoming.gitlab.com")
        );
        assert_eq!(merge.body["runners_token"], "runners-token-1");
        assert!(stand_in(&merge.body["namespace"]["id"]) > THING_BASE);
        // Small ids, a GitLab of one's own's, stay.
        assert_eq!(merge.body["local"], json!({ "id": 12, "project_id": 5 }));
    }

    #[test]
    fn the_recordings_of_one_run_share_their_stand_ins_of_ids() {
        #[derive(Debug)]
        struct Repository;
        impl Fetch for Repository {
            fn send(&self, _: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
                Ok(Response {
                    status: 200,
                    body: br#"{"id":918273645,"node_id":"R_kgDOShared1","name":"shared"}"#.to_vec(),
                    ..Default::default()
                })
            }
        }
        let dir = std::env::temp_dir().join(format!("bb-p-replay-book-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let mut written = Vec::new();
        for name in ["one", "two"] {
            let recorder = Recorder::new(
                Arc::new(Repository),
                dir.join(format!("{name}.json")),
                "test",
            );
            recorder
                .send(&call(Method::Get, "/repo", None), &AtomicBool::new(false))
                .unwrap();
            let path = recorder.finish(&["nobody-real"]).unwrap();
            let text = fs::read_to_string(path).unwrap();
            assert!(
                !text.contains("918273645") && !text.contains("kgDO"),
                "{text}"
            );
            let recorded: Recording = serde_json::from_str(&text).unwrap();
            written.push(recorded.exchanges[0].body.clone());
        }
        assert_eq!(written[0]["id"], written[1]["id"]);
        assert_eq!(written[0]["node_id"], written[1]["node_id"]);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_is_and_is_not_a_node_id_or_an_id() {
        assert_eq!(node_kind("PR_kwDOMadeUp8AAAABMadeUp"), Some("PR"));
        assert_eq!(node_kind("R_kgDOMadeUp"), Some("R"));
        assert_eq!(node_kind("MDQ6VXNlcjE="), Some(""));
        assert_eq!(node_kind("MDEwOlJlcG9zaXRvcnkxMjk2MjY5"), Some(""));
        for not in [
            "PR_TITLE",
            "CHANGES_REQUESTED",
            "MERGE_MESSAGE",
            "PR_node-1",
            "node-1",
            "MDNotBase64!",
            "MDQ",
            "",
        ] {
            assert_eq!(node_kind(not), None, "{not}");
        }
        for key in ["id", "project_id", "assignee_ids", "databaseId"] {
            assert!(names_an_id(key), "{key}");
        }
        for key in ["iid", "noteable_iid", "number", "valid", "width"] {
            assert!(!names_an_id(key), "{key}");
        }
        assert!(made_up_commit(&"0".repeat(40)));
        assert!(made_up_commit("0123456789abcdef0123456789abcdef01234567"));
        assert!(!made_up_commit("4f1c2e8a9b7d6c5e3f2a1b0c9d8e7f6a5b4c3d2e"));
        assert_eq!(
            id_numbers("r2400000078"),
            [(1, "2400000078")],
            "beside a letter that is no hexadecimal digit"
        );
        assert!(id_numbers("4f1c2e8a9b7d6c5e").is_empty());
        assert_eq!(id_numbers("123x45"), [(0, "123"), (4, "45")]);
        assert_eq!(
            word_runs("a/bc-12é3"),
            [(0, "a"), (2, "bc"), (5, "12"), (9, "3")]
        );
    }

    #[test]
    fn a_hand_made_object_names_only_what_matters_of_a_body() {
        let replay = Replay::new(Recording {
            note: String::new(),
            hand_made: Some("by hand".into()),
            exchanges: vec![exchange(
                Method::Post,
                "/graphql",
                json!({ "variables": { "search": "repo:o/r is:pr" } }),
                200,
                json!({ "data": {} }),
            )],
        });
        let go = AtomicBool::new(false);
        let ask = |body: Value| {
            let mut call = call(Method::Post, "/graphql", None);
            call = call.with_body("application/json", body.to_string().into_bytes());
            replay.send(&call, &go)
        };
        assert!(
            ask(json!({ "query": "q", "variables": { "search": "repo:o/r is:pr", "first": 100 } }))
                .is_ok()
        );
        assert!(ask(json!({ "query": "q", "variables": { "search": "repo:o/r" } })).is_err());
        assert!(ask(json!({ "query": "q" })).is_err());
    }

    #[test]
    fn a_recording_that_still_holds_a_real_value_is_not_kept() {
        let dir = std::env::temp_dir().join(format!("bb-p-replay-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        #[derive(Debug)]
        struct Echo;
        impl Fetch for Echo {
            fn send(&self, _: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
                Ok(Response {
                    status: 200,
                    // A real value where no person's object names it, and
                    // not as a word of its own: nothing replaces it.
                    body: br#"{"text":"xRealPersonx"}"#.to_vec(),
                    ..Default::default()
                })
            }
        }
        let leaky = Recorder::new(Arc::new(Echo), dir.join("x.json"), "test");
        leaky
            .send(&call(Method::Get, "/user", None), &AtomicBool::new(false))
            .unwrap();
        // Inside a word it is not replaced, and is still found: the file
        // is refused and removed.
        assert!(matches!(
            leaky.finish(&["RealPerson"]),
            Err(ReplayError::Leaked(_))
        ));
        assert!(!dir.join("x.json").exists());
        // As a word of its own it is replaced, and the file is kept.
        #[derive(Debug)]
        struct Named;
        impl Fetch for Named {
            fn send(&self, _: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
                Ok(Response {
                    status: 200,
                    body: br#"{"text":"opened by RealPerson"}"#.to_vec(),
                    ..Default::default()
                })
            }
        }
        let kept = Recorder::new(Arc::new(Named), dir.join("y.json"), "test");
        kept.send(&call(Method::Get, "/user", None), &AtomicBool::new(false))
            .unwrap();
        let path = kept.finish(&["RealPerson"]).unwrap();
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains("opened by user-1"), "{text}");
        let _ = fs::remove_dir_all(&dir);
    }
}
