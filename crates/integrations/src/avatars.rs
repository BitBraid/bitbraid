//! Pictures of authors from Gravatar, or from a service that answers as it
//! does. Opt-in: nothing here runs unless the user asked.
//!
//! What leaves the machine for one author is one request: `GET
//! <service>/<hash>?s=64&d=404`, where the hash is the SHA-256 of the
//! e-mail address, trimmed and in lower case, as the service documents.
//! With it go the host's name, the application's name and version, and
//! which kinds of picture are welcome; the service sees the network
//! address the request comes from, as anything asked over the network
//! does. Not the address itself, no name, nothing of a repository, no
//! cookie. `d=404` asks for "not found" where there is no picture, so
//! that no stand-in is sent and "none" can be remembered.
//!
//! Addresses nobody could have registered are not asked about at all:
//! ones without a domain, under a name reserved for tests and local
//! networks, or made by a host so that mail does not arrive (`noreply`).
//!
//! An answer is kept on disk ([`cache`]): a picture for a week, "none" for
//! three days, before it is asked again. A redirect is followed a few
//! times and only within the service's own domain. When the service does
//! not answer several times in a row it is left alone for a while, longer
//! each time, so that a machine without a network asks next to nothing.
//!
//! Sources: Gravatar For Developers, "Creating identifier (hash)"
//! (<https://docs.gravatar.com/api/avatars/hash/>) for the hash, and
//! "Avatars" (<https://docs.gravatar.com/api/avatars/images/>) for the
//! address of a picture and its `s` and `d` parameters; RFC 2606 and
//! RFC 6761 (domain names reserved for tests and examples), RFC 6762
//! (`.local`).

use std::{
    borrow::Cow,
    fmt::Write as _,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime},
};

use sha2::{Digest as _, Sha256};

use crate::http::{Fetch, FetchError, Limits, Request, Response, Url};

pub mod addressed;
pub mod cache;
pub mod decode;

pub use cache::{Bounds, Cache, Kept};
pub use decode::Bitmap;

/// Where pictures are asked for unless another service is named.
pub const DEFAULT_BASE: &str = "https://gravatar.com/avatar";

/// The side, in pixels, of the picture asked for.
pub const ASKED_SIDE: u32 = 64;

/// The kinds of picture that are read.
const ACCEPT: &str = "image/png, image/jpeg, image/gif, image/webp";

/// Longest a name is looked up and a connection made.
const CONNECT: Duration = Duration::from_secs(5);
/// Longest one picture is waited for, redirects and all.
const TOTAL: Duration = Duration::from_secs(12);
/// Most redirects followed for one picture.
const MAX_REDIRECTS: usize = 3;

/// Failures in a row after which the service is left alone for a while.
const TRIP: u32 = 3;
/// How long it is left alone the first time; twice as long each time
/// after, up to [`MAX_PAUSE`].
const PAUSE: Duration = Duration::from_secs(60);
/// The longest it is left alone.
const MAX_PAUSE: Duration = Duration::from_secs(30 * 60);

/// An e-mail address as it is hashed: without the space around it and in
/// lower case. Borrowed when it is that already, as nearly all are.
pub fn normalised(address: &str) -> Cow<'_, str> {
    let trimmed = address.trim();
    if trimmed
        .bytes()
        .all(|b| b.is_ascii() && !b.is_ascii_uppercase())
    {
        Cow::Borrowed(trimmed)
    } else {
        Cow::Owned(trimmed.to_lowercase())
    }
}

/// Whether an address (already [`normalised`]) is one somebody could have
/// registered a picture for. The others are never hashed or sent.
pub fn worth_asking(address: &str) -> bool {
    if address.len() > 254 || address.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return false;
    }
    let Some((local, domain)) = address.rsplit_once('@') else {
        return false;
    };
    if local.is_empty() || domain.is_empty() {
        return false;
    }
    let mut labels = domain.split('.');
    let Some(last) = labels.next_back() else {
        return false;
    };
    let mut others = 0usize;
    for label in labels {
        // `users.noreply.github.com` and its like: made so that no mail
        // reaches anybody.
        if label.is_empty() || label == "noreply" {
            return false;
        }
        others = others.saturating_add(1);
    }
    let made_of_names = domain
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '.');
    others > 0
        && made_of_names
        && !matches!(
            last,
            "" | "invalid"
                | "test"
                | "example"
                | "localhost"
                | "local"
                | "localdomain"
                | "internal"
                | "lan"
        )
}

/// The hash an address (already [`normalised`]) is asked for by: SHA-256,
/// as 64 hexadecimal digits.
pub fn hash(address: &str) -> String {
    let digest = Sha256::digest(address.as_bytes());
    let mut text = String::with_capacity(64);
    for byte in digest.iter() {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Whom pictures are asked of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Service {
    base: Url,
}

impl Service {
    /// The service at `base` (`https://host/path`, to which the hash is
    /// added), or Gravatar without one.
    pub fn new(base: Option<&str>) -> Result<Self, FetchError> {
        let text = base
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .unwrap_or(DEFAULT_BASE);
        let base = Url::parse(text)?;
        if base.path().contains('?') {
            return Err(FetchError::Address(say!(
                "avatars-service-address-question"
            )));
        }
        Ok(Self { base })
    }

    /// Who is asked: the host, with its port when that is not the usual.
    pub fn name(&self) -> String {
        self.base.authority()
    }

    /// Whether it is asked over TLS. A service that is not, is on this
    /// machine: the client speaks plainly to nothing else.
    pub fn secure(&self) -> bool {
        self.base.secure()
    }

    /// The address of the picture for a hash.
    pub fn address(&self, hash: &str, side: u32) -> Result<Url, FetchError> {
        if !cache::is_hash(hash) {
            return Err(FetchError::Address(say!("avatars-the-hash")));
        }
        let base = self.base.to_string();
        Url::parse(&format!(
            "{}/{hash}?s={side}&d=404",
            base.trim_end_matches('/')
        ))
    }

    /// Whether a redirect to `to` stays with the service: on its own
    /// domain and never from TLS to none. A service on this machine may
    /// only point at itself.
    pub fn may_follow(&self, to: &Url) -> bool {
        if !self.base.secure() {
            return !to.secure() && to.host() == self.base.host() && to.port() == self.base.port();
        }
        let site = self
            .base
            .host()
            .strip_prefix("www.")
            .unwrap_or(self.base.host());
        to.secure()
            && (to.host() == site
                || to
                    .host()
                    .strip_suffix(site)
                    .is_some_and(|front| front.ends_with('.')))
    }
}

/// What came of asking for a picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The picture, at each of the sizes asked for, in their order.
    Picture(Vec<Bitmap>),
    /// There is none, and that is known for a while.
    Absent,
    /// Not known now (no network, the service is not answering, the
    /// request was stopped): worth asking again later.
    Later,
}

#[derive(Debug, Default)]
struct Breaker {
    failures: u32,
    until: Option<Instant>,
}

impl Breaker {
    fn paused(&self, at: Instant) -> bool {
        self.until.is_some_and(|until| at < until)
    }

    fn failed(&mut self, at: Instant) {
        self.failures = self.failures.saturating_add(1);
        if self.failures >= TRIP {
            let doublings = self.failures.saturating_sub(TRIP).min(8);
            let pause = PAUSE
                .saturating_mul(1u32.checked_shl(doublings).unwrap_or(u32::MAX))
                .min(MAX_PAUSE);
            self.until = at.checked_add(pause);
        }
    }

    fn worked(&mut self) {
        *self = Self::default();
    }
}

/// The source of pictures: the service, what is kept of its answers, and
/// whatever fetches.
#[derive(Debug)]
pub struct Avatars {
    fetch: Arc<dyn Fetch>,
    service: Service,
    cache: Option<Cache>,
    breaker: Mutex<Breaker>,
    /// Longest a connection is waited for, and a whole picture.
    patience: (Duration, Duration),
}

impl Avatars {
    /// Pictures of `service`, fetched by `fetch`, kept in `cache_dir` when
    /// there is one (a folder of its own: see [`cache`]).
    pub fn new(fetch: Arc<dyn Fetch>, service: Service, cache_dir: Option<PathBuf>) -> Self {
        Self::with_bounds(fetch, service, cache_dir, Bounds::default())
    }

    /// The same with other bounds on what is kept.
    pub fn with_bounds(
        fetch: Arc<dyn Fetch>,
        service: Service,
        cache_dir: Option<PathBuf>,
        bounds: Bounds,
    ) -> Self {
        let cache = cache_dir.map(|dir| Cache::new(dir, &service.name(), bounds));
        Self {
            fetch,
            service,
            cache,
            breaker: Mutex::new(Breaker::default()),
            patience: (CONNECT, TOTAL),
        }
    }

    /// The same, waiting at most so long for a connection and for a whole
    /// picture, where the usual seconds are too many (the tests).
    pub fn patient(mut self, connect: Duration, total: Duration) -> Self {
        self.patience = (connect, total);
        self
    }

    /// Whom is asked.
    pub fn service(&self) -> &Service {
        &self.service
    }

    /// What is kept on disk, when anything is.
    pub fn cache(&self) -> Option<&Cache> {
        self.cache.as_ref()
    }

    /// The picture of the person with this e-mail address, scaled to each
    /// of `sides`. May take as long as a request does: for a thread that
    /// can wait. `cancel`, once set, ends it at once with [`Outcome::Later`].
    pub fn picture(&self, address: &str, sides: &[u32], cancel: &AtomicBool) -> Outcome {
        self.picture_at(address, sides, cancel, SystemTime::now(), Instant::now())
    }

    /// [`Avatars::picture`] as of a given moment, for what is kept (`now`)
    /// and for how long the service is left alone (`at`).
    pub fn picture_at(
        &self,
        address: &str,
        sides: &[u32],
        cancel: &AtomicBool,
        now: SystemTime,
        at: Instant,
    ) -> Outcome {
        let address = normalised(address);
        if !worth_asking(&address) {
            return Outcome::Absent;
        }
        let hash = hash(&address);
        let kept = match &self.cache {
            Some(cache) => cache.look(&hash, now),
            None => Kept::Nothing,
        };
        // What is kept and recent is the answer, without a word to anyone.
        let stale = match kept {
            Kept::Picture { bytes, fresh } => match decode::decode(&bytes, sides) {
                Some(bitmaps) if fresh => return Outcome::Picture(bitmaps),
                Some(bitmaps) => Some(bitmaps),
                // Kept, and not a picture: damaged on disk. Asked anew.
                None => None,
            },
            Kept::Absent { fresh: true } => return Outcome::Absent,
            Kept::Absent { fresh: false } | Kept::Nothing => None,
        };
        // A picture that is due but cannot be had anew is still shown.
        let fallback = |stale: Option<Vec<Bitmap>>| match stale {
            Some(bitmaps) => Outcome::Picture(bitmaps),
            None => Outcome::Later,
        };
        if self.breaker.lock().is_ok_and(|breaker| breaker.paused(at)) {
            return fallback(stale);
        }
        let keep_absent = || {
            if let Some(cache) = &self.cache {
                cache.keep_absent(&hash, now);
            }
        };
        let answered = |worked: bool| {
            if let Ok(mut breaker) = self.breaker.lock() {
                if worked {
                    breaker.worked();
                } else {
                    breaker.failed(at);
                }
            }
        };
        match self.ask(&hash, cancel) {
            Ok(response) if response.status == 200 => {
                answered(true);
                match decode::decode(&response.body, sides) {
                    Some(bitmaps) => {
                        if let Some(cache) = &self.cache {
                            cache.keep_picture(&hash, &response.body, now);
                        }
                        Outcome::Picture(bitmaps)
                    }
                    // An answer, and not a picture that can be shown: for
                    // this address there is none, for a while.
                    None => {
                        keep_absent();
                        Outcome::Absent
                    }
                }
            }
            Ok(response) if response.status == 404 || response.status == 410 => {
                answered(true);
                keep_absent();
                Outcome::Absent
            }
            Err(FetchError::Cancelled) => fallback(stale),
            // Any other status, and every way of not answering.
            Ok(_) | Err(_) => {
                answered(false);
                fallback(stale)
            }
        }
    }

    /// Ask the service, following it where it points within itself.
    fn ask(&self, hash: &str, cancel: &AtomicBool) -> Result<Response, FetchError> {
        let mut url = self.service.address(hash, ASKED_SIDE)?;
        let started = Instant::now();
        for _ in 0..=MAX_REDIRECTS {
            // The time is for the picture, not for each step towards it.
            let (connect, total) = self.patience;
            let left = total.saturating_sub(started.elapsed());
            let request = Request {
                url: &url,
                accept: ACCEPT,
                limits: Limits {
                    connect: connect.min(left),
                    total: left,
                    body: decode::MAX_BYTES,
                },
            };
            let response = self.fetch.get(&request, cancel)?;
            if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
                return Ok(response);
            }
            let location = response
                .location
                .as_deref()
                .ok_or(FetchError::Malformed(say!("avatars-redirect-to-nowhere")))?;
            let next = url.join(location)?;
            if !self.service.may_follow(&next) {
                return Err(FetchError::Address(say!(
                    "avatars-redirect-away-from-service"
                )));
            }
            url = next;
        }
        Err(FetchError::Malformed(say!("avatars-too-many-redirects")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_trimmed_and_lower_cased_before_it_is_hashed() {
        assert_eq!(normalised("ada@example.org"), "ada@example.org");
        assert!(matches!(normalised("ada@example.org"), Cow::Borrowed(_)));
        assert!(matches!(
            normalised("  ada@example.org\n"),
            Cow::Borrowed(_)
        ));
        assert_eq!(
            normalised("  Ada.Lovelace@Example.ORG \t"),
            "ada.lovelace@example.org"
        );
        assert_eq!(normalised("JÖRG@Example.org"), "jörg@example.org");
        assert_eq!(normalised(""), "");
    }

    #[test]
    fn the_hash_is_the_one_the_service_documents() {
        // SHA-256 of the empty string and of "abc", as FIPS 180-4 gives
        // them: the function is the one it says it is.
        assert_eq!(
            hash(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            hash("abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // However an address is written, it is asked for by one hash.
        let one = hash(&normalised("Ada@Example.org "));
        assert_eq!(one, hash(&normalised("ada@example.org")));
        assert_eq!(one.len(), 64);
        assert!(cache::is_hash(&one));
        assert_ne!(one, hash(&normalised("ada@example.com")));
    }

    #[test]
    fn addresses_nobody_registered_are_not_asked_about() {
        for address in [
            "ada@example.org",
            "a.b+tag@mail.example.co.uk",
            "jörg@bücher.de",
            "x@sub.domain.dev",
        ] {
            assert!(worth_asking(address), "{address}");
        }
        for address in [
            "",
            "ada",
            "@example.org",
            "ada@",
            "ada@localhost",
            "ada@machine",
            "ada@machine.local",
            "ada@machine.localdomain",
            "ada@machine.lan",
            "ada@build.internal",
            "ada@fixture.invalid",
            "ada@unit.test",
            "ada@host.example",
            "ada@machine.(none)",
            "12345+ada@users.noreply.github.com",
            "ada@noreply.codeberg.org",
            "ada@exam ple.org",
            "ada@example.org\nbcc: x",
            "ada@.org",
            "ada@example.",
            "ada@example..org",
        ] {
            assert!(!worth_asking(address), "{address:?}");
        }
        assert!(!worth_asking(&format!("{}@example.org", "a".repeat(250))));
    }

    #[test]
    fn the_address_of_a_picture_is_the_hash_and_two_parameters() {
        let hash = hash("ada@example.org");
        let gravatar = Service::new(None).unwrap();
        assert_eq!(gravatar.name(), "gravatar.com");
        assert!(gravatar.secure());
        assert_eq!(
            gravatar.address(&hash, 64).unwrap().to_string(),
            format!("https://gravatar.com/avatar/{hash}?s=64&d=404")
        );
        let own = Service::new(Some(" http://127.0.0.1:8080/avatar/ ")).unwrap();
        assert_eq!(own.name(), "127.0.0.1:8080");
        assert!(!own.secure());
        assert_eq!(
            own.address(&hash, 64).unwrap().to_string(),
            format!("http://127.0.0.1:8080/avatar/{hash}?s=64&d=404")
        );
        // Nothing but a hash is ever put into an address.
        assert!(gravatar.address("../../etc", 64).is_err());
        assert!(gravatar.address("ada@example.org", 64).is_err());
        // An empty word is no service of one's own.
        assert_eq!(Service::new(Some("  ")).unwrap(), gravatar);
        assert!(Service::new(Some("gravatar.com")).is_err());
        assert!(Service::new(Some("https://example.org/avatar?x=1")).is_err());
        assert!(Service::new(Some("file:///etc/passwd")).is_err());
    }

    #[test]
    fn a_redirect_is_followed_within_the_service_only() {
        let gravatar = Service::new(None).unwrap();
        let to = |text: &str| Url::parse(text).unwrap();
        assert!(gravatar.may_follow(&to("https://gravatar.com/avatar/x")));
        assert!(gravatar.may_follow(&to("https://www.gravatar.com/avatar/x")));
        assert!(gravatar.may_follow(&to("https://secure.gravatar.com/avatar/x")));
        assert!(!gravatar.may_follow(&to("http://gravatar.com/avatar/x")));
        assert!(!gravatar.may_follow(&to("https://example.org/avatar/x")));
        assert!(!gravatar.may_follow(&to("https://notgravatar.com/avatar/x")));
        assert!(!gravatar.may_follow(&to("https://gravatar.com.example.org/x")));
        let www = Service::new(Some("https://www.example.org/a")).unwrap();
        assert!(www.may_follow(&to("https://example.org/a/x")));
        assert!(www.may_follow(&to("https://cdn.example.org/a/x")));
        let own = Service::new(Some("http://127.0.0.1:8080/avatar")).unwrap();
        assert!(own.may_follow(&to("http://127.0.0.1:8080/other")));
        assert!(!own.may_follow(&to("http://127.0.0.1:8081/other")));
        assert!(!own.may_follow(&to("https://127.0.0.1:8080/other")));
        assert!(!own.may_follow(&to("http://localhost:8080/other")));
    }

    #[test]
    fn a_service_that_fails_is_left_alone_longer_each_time() {
        let start = Instant::now();
        let mut breaker = Breaker::default();
        breaker.failed(start);
        breaker.failed(start);
        assert!(!breaker.paused(start));
        breaker.failed(start);
        assert!(breaker.paused(start));
        assert!(breaker.paused(start + Duration::from_secs(59)));
        assert!(!breaker.paused(start + Duration::from_secs(61)));
        // The next failure after the pause doubles it.
        let second = start + Duration::from_secs(61);
        breaker.failed(second);
        assert!(breaker.paused(second + Duration::from_secs(119)));
        assert!(!breaker.paused(second + Duration::from_secs(121)));
        // It never grows past half an hour.
        for _ in 0..100 {
            breaker.failed(second);
        }
        assert!(breaker.paused(second + Duration::from_secs(29 * 60)));
        assert!(!breaker.paused(second + Duration::from_secs(31 * 60)));
        // One answer and all is forgiven.
        breaker.worked();
        assert!(!breaker.paused(second));
        breaker.failed(second);
        assert!(!breaker.paused(second));
    }
}
