//! The updater.
//!
//! It is off until the user turns it on: the window keeps that switch
//! (Preferences, General, Updates), asks at start and once a day while it
//! is on, and runs the commands of [`crate::update_cmds`] when the user
//! asks; the command line runs them when it is told to. Nothing here runs
//! by itself.
//!
//! Looking ([`check`]) is one request: the feed of releases ([`feed`]),
//! `latest.json` at the latest release of the project's repository on
//! GitHub (or the address in `BITBRAID_UPDATE_FEED`), which says the newest
//! version and where each of its packages is. Installing ([`install`])
//! reads the feed again, then downloads the package for this system and
//! for the way this copy was installed ([`install::Installation`]), checks
//! it as it comes ([`verify`]: its size, its SHA-256 and its minisign
//! signature against the key built into the application, which also has
//! to name the release and the file) and only then puts it in place the
//! system's way. Nothing that fails a check is installed, and what was
//! downloaded is deleted. A build without the key (the placeholder in
//! `packaging/update-key.pub`) asks nothing and says so.
//!
//! The feed and the package come through the application's one HTTPS
//! client ([`crate::http`]), and through the proxy the user's surroundings
//! name; a redirect (GitHub's release downloads are two) is followed a few
//! times, never from HTTPS to plain HTTP. The signature does not depend on
//! how the bytes came.
//!
//! Sources: docs.github.com "Linking to releases" (the latest release's
//! files under `releases/latest/download/`) and "About releases"; RFC 9110
//! §15.4 (redirects and `Location`); semver.org 2.0.0 (precedence: build
//! metadata does not count); the documents named in [`feed`], [`verify`]
//! and [`install`].

pub mod feed;
pub mod install;
pub mod verify;

use std::{
    fmt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, atomic::AtomicBool},
    time::Duration,
};

use bitbraid_engine::CommandContext;
use minisign_verify::PublicKey;

use crate::http::{Call, Client, Fetch, FetchError, Limits, Method, Response, Url};

pub use feed::{Arch, Feed, Kind, Package, System};
pub use install::{Installation, Then, start_again, tidy};
pub use verify::{Refusal, UpdateKey};

/// The variable that names another feed, for a test or a mirror.
pub const FEED_VARIABLE: &str = "BITBRAID_UPDATE_FEED";

/// Most redirects followed for one file.
const MAX_REDIRECTS: usize = 5;
/// Longest the name is looked up and the connection made.
const CONNECT: Duration = Duration::from_secs(15);
/// Longest the feed is waited for, redirects and all.
const FEED_TIME: Duration = Duration::from_secs(30);
/// Longest a package is waited for: tens of megabytes on a slow line.
const PACKAGE_TIME: Duration = Duration::from_secs(60 * 60);

/// The feed this build reads unless told otherwise: `latest.json` of the
/// latest release of the repository it comes from.
pub fn default_feed() -> String {
    format!(
        "{}/releases/latest/download/latest.json",
        env!("CARGO_PKG_REPOSITORY")
    )
}

/// The page of a release, for a feed that names none.
pub fn release_page(version: &semver::Version) -> String {
    format!("{}/releases/tag/v{version}", env!("CARGO_PKG_REPOSITORY"))
}

/// The system and processor a build runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Target {
    /// The system.
    pub system: System,
    /// The processor.
    pub arch: Arch,
}

impl Target {
    /// This build's; none on a system or processor no package is made for.
    pub fn this() -> Option<Self> {
        let system = match std::env::consts::OS {
            "linux" => System::Linux,
            "macos" => System::Macos,
            "windows" => System::Windows,
            _ => return None,
        };
        let arch = match std::env::consts::ARCH {
            "x86_64" => Arch::X86_64,
            "aarch64" => Arch::Aarch64,
            _ => return None,
        };
        Some(Self { system, arch })
    }
}

/// Everything the updater uses: the network, the key, where the feed is,
/// which version runs, how it was installed, on what, and where a download
/// that is not put beside the running copy goes.
pub struct Updater {
    /// The network.
    pub fetch: Arc<dyn Fetch>,
    /// The key packages are checked against.
    pub key: UpdateKey,
    /// Where the feed is.
    pub feed: String,
    /// The version that runs.
    pub current: semver::Version,
    /// How the running copy was installed.
    pub installation: Installation,
    /// What it runs on; none for a system no package is made for.
    pub target: Option<Target>,
    /// Where a package that is not put beside the copy is downloaded (the
    /// macOS bundle's tarball, the Windows installer).
    pub downloads: Option<PathBuf>,
}

impl fmt::Debug for Updater {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Updater")
            .field("fetch", &self.fetch)
            .field("key", &matches!(self.key, UpdateKey::Key(_)))
            .field("feed", &self.feed)
            .field("current", &self.current.to_string())
            .field("installation", &self.installation)
            .field("target", &self.target)
            .field("downloads", &self.downloads)
            .finish()
    }
}

/// Updaters by the configuration directory they are for.
type ByDirectory = Mutex<Vec<(PathBuf, Arc<Updater>)>>;

/// The updaters installed for configuration directories, for the tests of
/// the window, whose sessions make their own contexts.
fn installed() -> &'static ByDirectory {
    static INSTALLED: OnceLock<ByDirectory> = OnceLock::new();
    INSTALLED.get_or_init(Mutex::default)
}

impl Updater {
    /// The real one: the client, the key built in, the feed of the
    /// repository (or of `BITBRAID_UPDATE_FEED`), this version, this copy
    /// and the cache directory's `updates` folder.
    pub fn for_this_copy() -> Self {
        let feed = std::env::var(FEED_VARIABLE)
            .ok()
            .map(|feed| feed.trim().to_owned())
            .filter(|feed| !feed.is_empty())
            .unwrap_or_else(default_feed);
        Self {
            fetch: Arc::new(Client::new(&crate::services::agent())),
            key: UpdateKey::built_in(),
            feed,
            current: this_version(),
            installation: Installation::detect(),
            target: Target::this(),
            downloads: crate::services::cache_dir().map(|dir| dir.join("updates")),
        }
    }

    /// The real one, made once for the process.
    pub fn this_copy() -> Arc<Self> {
        static ONE: OnceLock<Arc<Updater>> = OnceLock::new();
        ONE.get_or_init(|| Arc::new(Self::for_this_copy())).clone()
    }

    /// Use `updater` for every command whose configuration directory is
    /// `dir`, from now on in this process: for a test of the window.
    pub fn install_for(dir: &Path, updater: Arc<Self>) {
        if let Ok(mut installed) = installed().lock() {
            installed.retain(|(known, _)| known != dir);
            installed.push((dir.to_path_buf(), updater));
        }
    }

    /// The one a command uses: the one in the context's slot, else the one
    /// installed for its configuration directory, else the real one.
    pub fn of(ctx: &CommandContext) -> Arc<Self> {
        if let Some(updater) = ctx.extensions.get::<Self>() {
            return updater;
        }
        if let Some(dir) = ctx.config_dir.as_deref()
            && let Ok(installed) = installed().lock()
            && let Some((_, updater)) = installed.iter().find(|(known, _)| known == dir)
        {
            return updater.clone();
        }
        Self::this_copy()
    }

    /// The key, or why there is none.
    fn key(&self) -> Result<&PublicKey, UpdateError> {
        match &self.key {
            UpdateKey::Key(key) => Ok(key),
            UpdateKey::Missing => Err(UpdateError::NoKey),
            UpdateKey::Unreadable(why) => Err(UpdateError::BadKey(why.clone())),
        }
    }
}

/// This build's version.
pub fn this_version() -> semver::Version {
    semver::Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or(semver::Version::new(0, 0, 0))
}

/// Whether `latest` comes after `current`, by semver's precedence: a
/// pre-release before its release, build metadata not counted.
pub fn is_newer(latest: &semver::Version, current: &semver::Version) -> bool {
    let rank = |version: &semver::Version| {
        (
            version.major,
            version.minor,
            version.patch,
            version.pre.clone(),
        )
    };
    rank(latest) > rank(current)
}

/// Why the updater did not do what it was asked.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UpdateError {
    /// The build has no update key.
    #[error("{}", say!("update-no-key"))]
    NoKey,
    /// The key in the build cannot be read.
    #[error("{}", say!("update-bad-key", why = .0.as_str()))]
    BadKey(String),
    /// The feed could not be read.
    #[error("{}", say!("update-feed-unreadable", url = .url.as_str(), why = .why.as_str()))]
    Feed {
        /// Where it was asked.
        url: String,
        /// Why not.
        why: String,
    },
    /// The feed names this version or an earlier one.
    #[error("{}", say!("update-newest", version = .0.as_str()))]
    Newest(String),
    /// The feed names another version than the one that was offered.
    #[error("{}", say!("update-changed", now = .now.as_str(), offered = .offered.as_str()))]
    Changed {
        /// What it names now.
        now: String,
        /// What was offered.
        offered: String,
    },
    /// This copy does not install the new version itself.
    #[error("{0}")]
    NotHere(String),
    /// The package could not be downloaded.
    #[error("{}", say!("update-download-failed", version = .version.as_str(), why = .why.as_str()))]
    Download {
        /// The version.
        version: String,
        /// Why.
        why: String,
    },
    /// What was downloaded failed a check, and was deleted.
    #[error(
        "{}",
        say!("update-download-refused", version = .version.as_str(), refusal = .refusal.to_string())
    )]
    Refused {
        /// The version.
        version: String,
        /// The check it failed.
        refusal: Refusal,
    },
    /// The package passed every check and could not be put in place.
    #[error("{}", say!("update-install-failed", version = .version.as_str(), why = .why.as_str()))]
    Install {
        /// The version.
        version: String,
        /// Why.
        why: String,
    },
    /// The user stopped it.
    #[error("{}", say!("update-stopped"))]
    Cancelled,
}

/// A package offered for this copy, as the commands answer it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Offered {
    /// What it is.
    pub kind: Kind,
    /// Its file's name.
    pub name: String,
    /// Where it is downloaded from.
    pub url: String,
    /// How large it is, in bytes.
    pub size: u64,
}

/// What a look at the feed found.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Found {
    /// The version that runs.
    pub current: String,
    /// The newest version.
    pub latest: String,
    /// Whether it is newer than the one that runs.
    pub newer: bool,
    /// The release's page.
    pub page: String,
    /// How this copy was installed (`appimage`, `bundle`, `installer`,
    /// `package`, `elsewhere`).
    pub installation: String,
    /// The file, folder or program that is this copy.
    pub path: String,
    /// Whether `update.install` can install the newest version here.
    pub installs: bool,
    /// Why not, when it cannot.
    pub why_not: Option<String>,
    /// The package it would install.
    pub package: Option<Offered>,
}

/// What an installation did.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Installed {
    /// The version installed.
    pub installed: String,
    /// What was replaced, or the installer that runs.
    pub path: String,
    /// What comes next: start again, or close for the installer.
    pub then: Then,
    /// The release's page.
    pub page: String,
}

/// Ask with `ask`, following redirects: a few, and never from HTTPS to
/// plain HTTP.
fn follow(
    start: Url,
    mut ask: impl FnMut(&Url) -> Result<Response, FetchError>,
) -> Result<Response, FetchError> {
    let mut url = start;
    for _ in 0..=MAX_REDIRECTS {
        let response = ask(&url)?;
        if !matches!(response.status, 301 | 302 | 303 | 307 | 308) {
            return Ok(response);
        }
        let location = response
            .location
            .as_deref()
            .ok_or_else(|| FetchError::Malformed(say!("update-redirect-nowhere")))?;
        let next = url.join(location)?;
        if url.secure() && !next.secure() {
            return Err(FetchError::Address(say!("update-redirect-insecure")));
        }
        url = next;
    }
    Err(FetchError::Malformed(say!("update-redirect-too-many")))
}

/// Read the feed.
fn read_feed(updater: &Updater, cancel: &AtomicBool) -> Result<Feed, UpdateError> {
    let failed = |why: String| UpdateError::Feed {
        url: updater.feed.clone(),
        why,
    };
    let url = Url::parse(&updater.feed).map_err(|err| failed(err.to_string()))?;
    let limits = Limits {
        connect: CONNECT,
        total: FEED_TIME,
        body: feed::MAX_FEED,
    };
    let response = follow(url, |url| {
        let call = Call::new(Method::Get, url.clone(), limits)
            .with_header("Accept", "application/json, application/octet-stream;q=0.9");
        updater.fetch.send(&call, cancel)
    })
    .map_err(|err| match err {
        FetchError::Cancelled => UpdateError::Cancelled,
        FetchError::TooLarge => failed(say!("update-feed-too-large").to_owned()),
        other => failed(other.to_string()),
    })?;
    if response.status != 200 {
        return Err(failed(say!("update-feed-status", status = response.status)));
    }
    feed::parse(&response.body, release_page).map_err(|err| failed(err.0))
}

/// What this copy can do with a release: whether it installs it itself,
/// why not, and the package it would install.
fn offer<'a>(updater: &Updater, feed: &'a Feed) -> (bool, Option<String>, Option<&'a Package>) {
    let Some(kind) = updater.installation.kind() else {
        return (false, updater.installation.why_not(), None);
    };
    let Some(target) = updater.target else {
        return (
            false,
            Some(say!("update-no-packages-for-system").to_owned()),
            None,
        );
    };
    let found = updater
        .installation
        .kinds()
        .iter()
        .find_map(|kind| feed.package_for(target.system, target.arch, *kind));
    match found {
        Some(package) => (true, None, Some(package)),
        None => (
            false,
            Some(say!(
                "update-no-package-in-release",
                kind = kind.name(),
                system = target.system.name(),
                arch = target.arch.word()
            )),
            None,
        ),
    }
}

/// What this copy is as far as updates go, asked of nobody.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct Status {
    /// The version that runs.
    pub current: String,
    /// Whether this build has an update key; without one it asks nothing.
    pub key: bool,
    /// Why it has none, when it has none.
    pub why_no_key: Option<String>,
    /// How this copy was installed (`appimage`, `bundle`, `installer`,
    /// `package`, `elsewhere`).
    pub installation: String,
    /// The file, folder or program that is this copy.
    pub path: String,
    /// Whether it installs a newer version itself.
    pub installs_itself: bool,
    /// Why not, when it does not.
    pub why_not: Option<String>,
    /// Where the feed is read.
    pub feed: String,
}

/// What this copy is as far as updates go: whether the build has a key,
/// how it was installed and whether it installs a newer version itself.
/// Nothing is asked of anybody.
pub fn status(updater: &Updater) -> Status {
    let why_no_key = updater.key().err().map(|err| err.to_string());
    let (installs_itself, why_not) = match (updater.installation.kind(), updater.target) {
        (Some(_), Some(_)) => (true, None),
        (Some(_), None) => (
            false,
            Some(say!("update-no-packages-for-system").to_owned()),
        ),
        (None, _) => (false, updater.installation.why_not()),
    };
    Status {
        current: updater.current.to_string(),
        key: why_no_key.is_none(),
        why_no_key,
        installation: updater.installation.word().to_owned(),
        path: updater.installation.path().display().to_string(),
        installs_itself,
        why_not,
        feed: updater.feed.clone(),
    }
}

/// Look at the feed: the newest version, whether it is newer than this
/// one, and what this copy can do about it. One request (and its
/// redirects); none without a key.
pub fn check(updater: &Updater, cancel: &AtomicBool) -> Result<Found, UpdateError> {
    updater.key()?;
    let feed = read_feed(updater, cancel)?;
    let newer = is_newer(&feed.version, &updater.current);
    let (installs, why_not, package) = offer(updater, &feed);
    Ok(Found {
        current: updater.current.to_string(),
        latest: feed.version.to_string(),
        newer,
        page: feed.page.to_string(),
        installation: updater.installation.word().to_owned(),
        path: updater.installation.path().display().to_string(),
        installs,
        why_not,
        package: package.map(|package| Offered {
            kind: package.kind,
            name: package.name.clone(),
            url: package.url.to_string(),
            size: package.size,
        }),
    })
}

/// Download `package` of `feed` into the plan's file, checking it as it
/// comes, and keep it only when it passes every check.
fn download(
    updater: &Updater,
    key: &PublicKey,
    feed: &Feed,
    package: &Package,
    plan: &install::Plan,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u8),
) -> Result<(), UpdateError> {
    let version = feed.version.to_string();
    let refused = |refusal: Refusal| UpdateError::Refused {
        version: version.clone(),
        refusal,
    };
    let failed = |why: String| UpdateError::Download {
        version: version.clone(),
        why,
    };
    let signature = verify::signature_for(key, package).map_err(refused)?;
    let file = plan.open_download().map_err(|why| UpdateError::Install {
        version: version.clone(),
        why,
    })?;
    let size = package.size;
    let mut said: Option<u8> = None;
    let mut on_written = |written: u64| {
        let percent = written
            .saturating_mul(100)
            .checked_div(size)
            .map_or(100, |percent| u8::try_from(percent.min(100)).unwrap_or(100));
        if said != Some(percent) {
            said = Some(percent);
            progress(percent);
        }
    };
    let mut checking =
        verify::Checking::new(file, key, &signature, size, &mut on_written).map_err(refused)?;
    let limits = Limits {
        connect: CONNECT,
        total: PACKAGE_TIME,
        body: usize::try_from(size).unwrap_or(usize::MAX),
    };
    let answer = follow(package.url.clone(), |url| {
        let call = Call::new(Method::Get, url.clone(), limits)
            .with_header("Accept", "application/octet-stream");
        updater.fetch.send_to(&call, cancel, &mut checking)
    });
    match answer {
        Ok(response) if response.status == 200 => {}
        Ok(response) => {
            return Err(failed(say!(
                "update-download-status",
                status = response.status
            )));
        }
        Err(_) if checking.over => return Err(refused(Refusal::TooLarge { expected: size })),
        Err(FetchError::TooLarge) => return Err(refused(Refusal::TooLarge { expected: size })),
        Err(FetchError::Cancelled) => return Err(UpdateError::Cancelled),
        // The connection ended part way (the server stopped, the line went):
        // said as that, not as an answer the client cannot read.
        Err(FetchError::Malformed(_) | FetchError::Broken(_)) if checking.written() > 0 => {
            return Err(failed(say!(
                "update-download-cut",
                got = checking.written(),
                expected = size
            )));
        }
        Err(err) => return Err(failed(err.to_string())),
    }
    let file = checking.finish(package, &version).map_err(refused)?;
    file.sync_all()
        .map_err(|err| failed(say!("update-download-unwritten", error = err.to_string())))?;
    Ok(())
}

/// Install the newest version: read the feed, and when it names a newer
/// version than this one (and `offered`, when given), download the package
/// for this copy, check it, and put it in place the system's way. Told the
/// percentage downloaded as it changes.
pub fn install(
    updater: &Updater,
    offered: Option<&semver::Version>,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(u8),
) -> Result<Installed, UpdateError> {
    let key = updater.key()?;
    let feed = read_feed(updater, cancel)?;
    if let Some(offered) = offered
        && *offered != feed.version
    {
        return Err(UpdateError::Changed {
            now: feed.version.to_string(),
            offered: offered.to_string(),
        });
    }
    if !is_newer(&feed.version, &updater.current) {
        return Err(UpdateError::Newest(updater.current.to_string()));
    }
    let (_, why_not, package) = offer(updater, &feed);
    let package = package.ok_or_else(|| {
        UpdateError::NotHere(why_not.unwrap_or_else(|| say!("update-not-itself").to_owned()))
    })?;
    let plan = install::Plan::new(
        &updater.installation,
        package.kind,
        &feed.version,
        updater.downloads.as_deref(),
    )
    .map_err(UpdateError::NotHere)?;
    if let Err(err) = download(updater, key, &feed, package, &plan, cancel, progress) {
        plan.discard();
        return Err(err);
    }
    let then = plan.put_in_place().map_err(|why| {
        plan.discard();
        UpdateError::Install {
            version: feed.version.to_string(),
            why,
        }
    })?;
    Ok(Installed {
        installed: feed.version.to_string(),
        path: updater.installation.path().display().to_string(),
        then,
        page: feed.page.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn version(text: &str) -> semver::Version {
        semver::Version::parse(text).unwrap()
    }

    #[test]
    fn newer_is_by_precedence_without_build_metadata() {
        assert!(is_newer(&version("1.0.1"), &version("1.0.0")));
        assert!(is_newer(&version("1.1.0"), &version("1.0.9")));
        assert!(is_newer(&version("2.0.0"), &version("1.99.99")));
        assert!(is_newer(&version("1.0.0"), &version("1.0.0-rc.1")));
        assert!(is_newer(&version("1.0.0-rc.2"), &version("1.0.0-rc.1")));
        assert!(!is_newer(&version("1.0.0"), &version("1.0.0")));
        assert!(!is_newer(
            &version("1.0.0+build.9"),
            &version("1.0.0+build.1")
        ));
        assert!(!is_newer(&version("0.9.0"), &version("1.0.0")));
        assert!(!is_newer(&version("1.0.0-rc.1"), &version("1.0.0")));
    }

    #[test]
    fn the_feed_is_the_latest_releases_of_the_repository() {
        assert_eq!(
            default_feed(),
            "https://github.com/BitBraid/bitbraid/releases/latest/download/latest.json"
        );
        assert_eq!(
            release_page(&version("1.2.0")),
            "https://github.com/BitBraid/bitbraid/releases/tag/v1.2.0"
        );
    }

    #[test]
    fn a_redirect_is_followed_a_few_times_and_never_to_plain_http() {
        let start = Url::parse("https://example.org/a").unwrap();
        let moved = |to: &str| Response {
            status: 302,
            location: Some(to.to_owned()),
            ..Default::default()
        };
        let mut asked = Vec::new();
        let answer = follow(start.clone(), |url| {
            asked.push(url.to_string());
            Ok(match url.path() {
                "/a" => moved("/b"),
                "/b" => moved("https://cdn.example.org/c"),
                _ => Response {
                    status: 200,
                    ..Default::default()
                },
            })
        })
        .unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(
            asked,
            [
                "https://example.org/a",
                "https://example.org/b",
                "https://cdn.example.org/c"
            ]
        );
        assert_eq!(
            follow(start.clone(), |_| Ok(moved("http://example.org/x"))),
            Err(FetchError::Address("a redirect from HTTPS to plain HTTP"))
        );
        assert_eq!(
            follow(start.clone(), |_| Ok(moved("/again"))),
            Err(FetchError::Malformed("too many redirects"))
        );
        assert_eq!(
            follow(start, |_| Ok(Response {
                status: 301,
                ..Default::default()
            })),
            Err(FetchError::Malformed("a redirect to nowhere"))
        );
    }
}
