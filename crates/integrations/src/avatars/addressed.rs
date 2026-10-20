//! Pictures at an address a provider gave: an account's, a repository
//! owner's, an author's whose e-mail address the provider knows. Opt-in
//! like the pictures of [`super::Avatars`]: nothing here runs unless
//! the user asked for pictures from the providers.
//!
//! What leaves the machine for one picture is one `GET` of the address,
//! without a token, a cookie or anything of a repository, and the
//! redirects it answers with, at most three, each to an HTTPS address
//! (or, from this machine, to this machine). The address is kept to its
//! scheme, host and path with its question: only HTTPS, or plain HTTP to
//! this machine (a GitLab of one's own run for trying things out).
//!
//! An answer is kept on disk as the other pictures are ([`super::cache`]),
//! under the SHA-256 of the address, in a folder of its own.
//!
//! Sources: docs.github.com "Users" (`avatar_url`); docs.gitlab.com
//! "Users API" (`avatar_url`); RFC 9110 (redirections).

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant, SystemTime},
};

use super::{
    Outcome,
    cache::{Bounds, Cache, Kept},
    decode,
};
use crate::http::{Fetch, FetchError, Limits, Request, Response, Url};

/// The name of the folder the answers are kept in.
pub const FOLDER: &str = "provider-pictures";
/// The longest address taken.
const LONGEST: usize = 2048;
/// Longest a name is looked up and a connection made.
const CONNECT: Duration = Duration::from_secs(5);
/// Longest one picture is waited for, redirects and all.
const TOTAL: Duration = Duration::from_secs(12);
/// Most redirects followed for one picture.
const MAX_REDIRECTS: usize = 3;
/// The kinds of picture that are read.
const ACCEPT: &str = "image/png, image/jpeg, image/gif, image/webp";

/// Whether a picture's address may be asked: HTTPS, or HTTP to this
/// machine, and nothing that is not plain text.
pub fn may_ask(address: &str) -> bool {
    if address.len() > LONGEST {
        return false;
    }
    match Url::parse(address) {
        Ok(url) => url.secure() || is_loopback(url.host()),
        Err(_) => false,
    }
}

/// Hosts of services that hand out pictures by the hash of an e-mail
/// address, and their subdomains.
const PICTURE_SERVICES: &[&str] = &["gravatar.com", "libravatar.org"];

/// Whether a picture's address is at a service that hands out pictures by
/// the hash of an e-mail address: Gravatar, Libravatar, or the service the
/// user named in place of Gravatar (`base`). Such an address names
/// somebody's e-mail address in a hash, and a provider may answer with one
/// (GitLab does, for a person without a picture of their own there), so it
/// is asked only when the user turned pictures from that service on.
pub fn at_picture_service(address: &str, base: Option<&str>) -> bool {
    let Ok(url) = Url::parse(address.trim()) else {
        return false;
    };
    let host = url.host().trim_end_matches('.').to_ascii_lowercase();
    let known = PICTURE_SERVICES.iter().any(|service| {
        host == *service
            || host
                .strip_suffix(service)
                .is_some_and(|before| before.ends_with('.'))
    });
    let named = base
        .map(str::trim)
        .filter(|base| !base.is_empty())
        .and_then(|base| Url::parse(base).ok())
        .is_some_and(|base| {
            base.host()
                .trim_end_matches('.')
                .eq_ignore_ascii_case(&host)
        });
    known || named
}

fn is_loopback(host: &str) -> bool {
    let name = host.trim_start_matches('[').trim_end_matches(']');
    name == "localhost"
        || name
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

/// Whether a redirect from `from` to `to` is followed: to HTTPS, or from
/// this machine to this machine.
fn may_follow(from: &Url, to: &Url) -> bool {
    if to.secure() {
        return true;
    }
    !from.secure() && is_loopback(from.host()) && is_loopback(to.host())
}

/// Pictures at addresses, fetched by whatever fetches, kept in a folder
/// when there is one.
#[derive(Debug)]
pub struct Addressed {
    fetch: Arc<dyn Fetch>,
    cache: Option<Cache>,
    patience: (Duration, Duration),
}

impl Addressed {
    /// Pictures fetched by `fetch`, kept under `cache_dir` (in the folder
    /// [`FOLDER`]) when there is one.
    pub fn new(fetch: Arc<dyn Fetch>, cache_dir: Option<PathBuf>) -> Self {
        let cache =
            cache_dir.map(|dir| Cache::new(dir.join(FOLDER), "addresses", Bounds::default()));
        Self {
            fetch,
            cache,
            patience: (CONNECT, TOTAL),
        }
    }

    /// The picture at `address`, scaled to each of `sides`. May take as
    /// long as a request does: for a thread that can wait.
    pub fn picture(&self, address: &str, sides: &[u32], cancel: &AtomicBool) -> Outcome {
        self.picture_at(address, sides, cancel, SystemTime::now())
    }

    /// [`Addressed::picture`] as of a given moment, for what is kept.
    pub fn picture_at(
        &self,
        address: &str,
        sides: &[u32],
        cancel: &AtomicBool,
        now: SystemTime,
    ) -> Outcome {
        let address = address.trim();
        if !may_ask(address) {
            return Outcome::Absent;
        }
        let hash = super::hash(address);
        let kept = match &self.cache {
            Some(cache) => cache.look(&hash, now),
            None => Kept::Nothing,
        };
        let stale = match kept {
            Kept::Picture { bytes, fresh } => match decode::decode(&bytes, sides) {
                Some(bitmaps) if fresh => return Outcome::Picture(bitmaps),
                Some(bitmaps) => Some(bitmaps),
                None => None,
            },
            Kept::Absent { fresh: true } => return Outcome::Absent,
            Kept::Absent { fresh: false } | Kept::Nothing => None,
        };
        let fallback = |stale: Option<Vec<super::Bitmap>>| match stale {
            Some(bitmaps) => Outcome::Picture(bitmaps),
            None => Outcome::Later,
        };
        let keep_absent = || {
            if let Some(cache) = &self.cache {
                cache.keep_absent(&hash, now);
            }
        };
        match self.ask(address, cancel) {
            Ok(response) if response.status == 200 => match decode::decode(&response.body, sides) {
                Some(bitmaps) => {
                    if let Some(cache) = &self.cache {
                        cache.keep_picture(&hash, &response.body, now);
                    }
                    Outcome::Picture(bitmaps)
                }
                None => {
                    keep_absent();
                    Outcome::Absent
                }
            },
            Ok(response) if matches!(response.status, 404 | 410) => {
                keep_absent();
                Outcome::Absent
            }
            Ok(_) | Err(_) => fallback(stale),
        }
    }

    /// Ask for the picture, following redirects that may be followed.
    fn ask(&self, address: &str, cancel: &AtomicBool) -> Result<Response, FetchError> {
        let mut url = Url::parse(address)?;
        let started = Instant::now();
        for _ in 0..=MAX_REDIRECTS {
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
            if !may_follow(&url, &next) {
                return Err(FetchError::Address(say!(
                    "avatars-redirect-away-from-https"
                )));
            }
            url = next;
        }
        Err(FetchError::Malformed(say!("avatars-too-many-redirects")))
    }
}

#[cfg(test)]
mod tests {
    use super::{at_picture_service, may_ask, may_follow};
    use crate::http::Url;

    #[test]
    fn an_address_at_a_picture_service_is_known_as_one() {
        for address in [
            "https://secure.gravatar.com/avatar/0123abcd?s=80&d=identicon",
            "https://www.gravatar.com/avatar/0123",
            "https://gravatar.com/avatar/0123",
            "https://GRAVATAR.COM./avatar/0123",
            "https://seccdn.libravatar.org/avatar/0123",
        ] {
            assert!(at_picture_service(address, None), "{address}");
        }
        for address in [
            "https://avatars.githubusercontent.com/u/1?v=4",
            "https://gitlab.com/uploads/-/system/user/avatar/2/a.png",
            "https://notgravatar.com/avatar/0123",
            "https://gravatar.com.example.org/avatar/0123",
            "not an address",
            "",
        ] {
            assert!(!at_picture_service(address, None), "{address}");
        }
        // The service the user named in Gravatar's place.
        let base = Some("https://pictures.example.org/avatar");
        assert!(at_picture_service(
            "https://pictures.example.org/avatar/0123",
            base
        ));
        assert!(!at_picture_service("https://other.example.org/a.png", base));
        assert!(!at_picture_service(
            "https://other.example.org/a.png",
            Some("")
        ));
    }

    #[test]
    fn only_https_or_this_machine_is_asked() {
        assert!(may_ask("https://avatars.githubusercontent.com/u/1?v=4"));
        assert!(may_ask(
            "http://127.0.0.1:8929/uploads/-/system/user/avatar/2/a.png"
        ));
        assert!(may_ask("http://localhost/a.png"));
        assert!(!may_ask("http://gitlab.example.org/a.png"));
        assert!(!may_ask("file:///etc/passwd"));
        assert!(!may_ask("https://a.org/a b.png"));
        assert!(!may_ask(&format!("https://a.org/{}", "x".repeat(3000))));
        assert!(!may_ask(""));
    }

    #[test]
    fn redirects_go_to_https_only() {
        let https = Url::parse("https://github.com/someone.png").unwrap();
        let local = Url::parse("http://127.0.0.1:8929/a.png").unwrap();
        let plain = Url::parse("http://example.org/a.png").unwrap();
        let other = Url::parse("https://avatars.githubusercontent.com/u/1").unwrap();
        assert!(may_follow(&https, &other));
        assert!(!may_follow(&https, &plain));
        assert!(!may_follow(&https, &local));
        assert!(may_follow(
            &local,
            &Url::parse("http://localhost/b.png").unwrap()
        ));
        assert!(!may_follow(&local, &plain));
    }
}
