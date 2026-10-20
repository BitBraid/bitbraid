//! The feed of releases, `latest.json`: what the release pipeline writes
//! beside each release and the updater reads.
//!
//! One JSON object: the format's number (`schema`, 1), the release's
//! `version`, its `page` (or `notes`, as the release pipeline's writer
//! names it), and a list of `packages`, each with its `system`, `arch` and
//! `kind`, its file's `name`, its `url`, `size` and `sha256`, and its
//! minisign `signature` as the whole text of its `.minisig` file. Other
//! fields (the writer's `name` and `date`) are not read.
//!
//! Everything in it is hostile until read: the feed is at most
//! [`MAX_FEED`] bytes, a list at most [`MAX_PACKAGES`] long, every text has
//! a length it may not pass, a size a bound, a digest its exact form, an
//! address the form the client asks. A package of a system, processor or
//! kind this build does not know is passed over (a later feed may list
//! more); one it knows that is not as it should be makes the whole feed
//! unreadable, so that a package that is wrong is never mistaken for one
//! that is missing.
//!
//! Sources: RFC 8259 (JSON); semver.org 2.0.0 (versions, their order, and
//! that build metadata does not count); the documentation of `semver`
//! (`Version::parse`, its `Ord`) and of `minisign-verify`
//! (`Signature::decode`); jedisct1.github.io/minisign (the signature's
//! four lines, the trusted comment, `-t`).

use serde_json::Value;

use crate::http::Url;

/// The number of the feed's format this build reads.
pub const SCHEMA: u64 = 1;
/// Largest feed read, in bytes.
pub const MAX_FEED: usize = 64 * 1024;
/// Most packages a feed lists.
pub const MAX_PACKAGES: usize = 64;
/// Largest package taken, in bytes.
pub const MAX_PACKAGE_SIZE: u64 = 1 << 30;
/// Longest version, in bytes.
const MAX_VERSION: usize = 64;
/// Longest file name of a package.
const MAX_NAME: usize = 200;
/// Longest signature, the four lines of a `.minisig` file.
const MAX_SIGNATURE: usize = 2048;

/// A system packages are made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum System {
    /// Linux.
    Linux,
    /// macOS.
    Macos,
    /// Windows.
    Windows,
}

impl System {
    /// The word the feed has for it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Macos => "macos",
            Self::Windows => "windows",
        }
    }

    /// What it is called in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Self::Linux => "Linux",
            Self::Macos => "macOS",
            Self::Windows => "Windows",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [Self::Linux, Self::Macos, Self::Windows]
            .into_iter()
            .find(|system| system.word() == word)
    }
}

/// A processor packages are made for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Arch {
    /// 64-bit x86.
    #[serde(rename = "x86_64")]
    X86_64,
    /// 64-bit ARM.
    Aarch64,
    /// Both of macOS's, in one bundle.
    Universal,
}

impl Arch {
    /// The word the feed has for it.
    pub fn word(self) -> &'static str {
        match self {
            Self::X86_64 => "x86_64",
            Self::Aarch64 => "aarch64",
            Self::Universal => "universal",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [Self::X86_64, Self::Aarch64, Self::Universal]
            .into_iter()
            .find(|arch| arch.word() == word)
    }
}

/// What kind of package a file is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A Linux AppImage: the updater replaces it.
    Appimage,
    /// A Debian package: installed by the package manager.
    Deb,
    /// An RPM package: installed by the package manager.
    Rpm,
    /// A gzip tarball of the program and its files.
    Tarball,
    /// A gzip tarball of the macOS application bundle at its top: what the
    /// updater replaces the bundle with.
    App,
    /// A macOS disk image, for a person to install from.
    Dmg,
    /// A Windows installer: the updater runs it.
    Msi,
    /// A zip of the Windows program, to run from where it is unpacked.
    Zip,
}

impl Kind {
    /// The word the feed has for it.
    pub fn word(self) -> &'static str {
        match self {
            Self::Appimage => "appimage",
            Self::Deb => "deb",
            Self::Rpm => "rpm",
            Self::Tarball => "tarball",
            Self::App => "app",
            Self::Dmg => "dmg",
            Self::Msi => "msi",
            Self::Zip => "zip",
        }
    }

    /// What it is called in a sentence.
    pub fn name(self) -> &'static str {
        match self {
            Self::Appimage => say!("update-kind-appimage"),
            Self::Deb => say!("update-kind-deb"),
            Self::Rpm => say!("update-kind-rpm"),
            Self::Tarball => say!("update-kind-tarball"),
            Self::App => say!("update-kind-app"),
            Self::Dmg => say!("update-kind-dmg"),
            Self::Msi => say!("update-kind-msi"),
            Self::Zip => say!("update-kind-zip"),
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        [
            Self::Appimage,
            Self::Deb,
            Self::Rpm,
            Self::Tarball,
            Self::App,
            Self::Dmg,
            Self::Msi,
            Self::Zip,
        ]
        .into_iter()
        .find(|kind| kind.word() == word)
    }
}

/// One package of a release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Package {
    /// The system it is for.
    pub system: System,
    /// The processor it is for.
    pub arch: Arch,
    /// What it is.
    pub kind: Kind,
    /// Its file's name, which its signature's trusted comment names.
    pub name: String,
    /// Where it is downloaded from.
    pub url: Url,
    /// How large it is, in bytes.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: [u8; 32],
    /// Its minisign signature: the four lines of its `.minisig` file.
    pub signature: String,
}

/// A feed, read and found sound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Feed {
    /// The release's version.
    pub version: semver::Version,
    /// Its page, where a person downloads it and reads what is new.
    pub page: Url,
    /// Its packages this build knows.
    pub packages: Vec<Package>,
}

impl Feed {
    /// The package of this kind for this system and processor: one made
    /// for the processor, else one for both of macOS's.
    pub fn package_for(&self, system: System, arch: Arch, kind: Kind) -> Option<&Package> {
        let fits = |package: &&Package| package.system == system && package.kind == kind;
        self.packages
            .iter()
            .filter(fits)
            .find(|package| package.arch == arch)
            .or_else(|| {
                self.packages
                    .iter()
                    .filter(fits)
                    .find(|package| package.arch == Arch::Universal)
            })
    }
}

/// Why a feed cannot be read: what is wrong, in a few words.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct Unreadable(pub String);

fn wrong(what: impl Into<String>) -> Unreadable {
    Unreadable(what.into())
}

/// The text of a field: a string of at most `limit` bytes. What may be in
/// it is for the field to say.
fn text<'a>(object: &'a Value, key: &str, limit: usize) -> Result<&'a str, Unreadable> {
    let value = object
        .get(key)
        .ok_or_else(|| wrong(say!("update-feed-no-field", key)))?;
    let text = value
        .as_str()
        .ok_or_else(|| wrong(say!("update-feed-not-text", key)))?;
    if text.is_empty() || text.len() > limit {
        return Err(wrong(say!("update-feed-empty-or-long", key)));
    }
    Ok(text)
}

/// A version as a release names it: `1.2.3`, a leading `v` allowed.
pub fn version(text: &str) -> Result<semver::Version, Unreadable> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_VERSION {
        return Err(wrong(say!("update-feed-version-empty-or-long")));
    }
    let bare = text.strip_prefix('v').unwrap_or(text);
    semver::Version::parse(bare).map_err(|_| {
        wrong(say!(
            "update-feed-not-a-version",
            version = format!("{bare:?}")
        ))
    })
}

/// An address the client can ask: HTTPS, or plain HTTP to this machine.
fn address(text: &str, what: &str) -> Result<Url, Unreadable> {
    let url = Url::parse(text).map_err(|_| wrong(say!("update-feed-not-an-address", what)))?;
    let here = url.host() == "localhost"
        || url
            .host()
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    if !url.secure() && !here {
        return Err(wrong(say!("update-feed-not-https", what)));
    }
    Ok(url)
}

/// A SHA-256 written as 64 hexadecimal digits.
fn digest(text: &str) -> Option<[u8; 32]> {
    if text.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (at, byte) in out.iter_mut().enumerate() {
        let pair = text.get(at.saturating_mul(2)..at.saturating_mul(2).saturating_add(2))?;
        *byte = u8::from_str_radix(pair, 16).ok()?;
    }
    Some(out)
}

/// A file name as a release has it: letters, digits, `.`, `_`, `+`, `-`,
/// not starting with a dot or a dash.
fn file_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= MAX_NAME
        && !text.starts_with(['.', '-'])
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'+' | b'-'))
}

/// One package, or none when it is for something this build does not
/// know.
fn package(value: &Value) -> Result<Option<Package>, Unreadable> {
    if !value.is_object() {
        return Err(wrong(say!("update-feed-package-not-object")));
    }
    let (Some(system), Some(arch), Some(kind)) = (
        System::from_word(text(value, "system", 32)?),
        Arch::from_word(text(value, "arch", 32)?),
        Kind::from_word(text(value, "kind", 32)?),
    ) else {
        return Ok(None);
    };
    let within = |err: Unreadable| {
        wrong(say!(
            "update-feed-in-package",
            system = system.word(),
            arch = arch.word(),
            kind = kind.word(),
            error = err.0
        ))
    };
    let name = text(value, "name", MAX_NAME).map_err(within)?;
    if !file_name(name) {
        return Err(within(wrong(say!("update-feed-bad-name"))));
    }
    let url = address(text(value, "url", 2048).map_err(within)?, "url").map_err(within)?;
    let size = value
        .get("size")
        .and_then(Value::as_u64)
        .filter(|size| (1..=MAX_PACKAGE_SIZE).contains(size))
        .ok_or_else(|| within(wrong(say!("update-feed-bad-size"))))?;
    let sha256 = digest(text(value, "sha256", 64).map_err(within)?)
        .ok_or_else(|| within(wrong(say!("update-feed-bad-sha256"))))?;
    let signature = text(value, "signature", MAX_SIGNATURE).map_err(within)?;
    if minisign_verify::Signature::decode(signature).is_err() {
        return Err(within(wrong(say!("update-feed-bad-signature"))));
    }
    Ok(Some(Package {
        system,
        arch,
        kind,
        name: name.to_owned(),
        url,
        size,
        sha256,
        signature: signature.to_owned(),
    }))
}

/// Read a feed. `default_page` is the release's page when the feed names
/// none, made from its version.
pub fn parse(
    bytes: &[u8],
    default_page: impl Fn(&semver::Version) -> String,
) -> Result<Feed, Unreadable> {
    if bytes.len() > MAX_FEED {
        return Err(wrong(say!("update-feed-too-large")));
    }
    let value: Value = serde_json::from_slice(bytes)
        .map_err(|_| wrong(say!("update-feed-not-json").to_owned()))?;
    if !value.is_object() {
        return Err(wrong(say!("update-feed-not-object")));
    }
    match value.get("schema").map(Value::as_u64) {
        Some(Some(SCHEMA)) => {}
        Some(Some(other)) if other > SCHEMA => {
            return Err(wrong(say!("update-feed-later-format", schema = other)));
        }
        _ => return Err(wrong(say!("update-feed-schema"))),
    }
    let version = version(text(&value, "version", MAX_VERSION)?)?;
    let named = |key: &str| value.get(key).is_some_and(|page| !page.is_null());
    let page = if named("page") {
        text(&value, "page", 2048)?.to_owned()
    } else if named("notes") {
        text(&value, "notes", 2048)?.to_owned()
    } else {
        default_page(&version)
    };
    let page = address(&page, "page")?;
    let listed = value
        .get("packages")
        .and_then(Value::as_array)
        .ok_or_else(|| wrong(say!("update-feed-no-packages")))?;
    if listed.len() > MAX_PACKAGES {
        return Err(wrong(say!("update-feed-too-many-packages")));
    }
    let mut packages = Vec::new();
    for value in listed {
        if let Some(package) = package(value)? {
            packages.push(package);
        }
    }
    Ok(Feed {
        version,
        page,
        packages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A signature as minisign writes it; whether it is right is not the
    /// feed's to know.
    const SIGNATURE: &str = "untrusted comment: signature from minisign secret key\n\
        RUQf6LRCGA9i59SLOFxz6NxvASXDJeRtuZykwQepbDEGt87ig1BNpWaVWuNrm73YiIiJbq71Wi+dP9eKL8OC351vwIasSSbXxwA=\n\
        trusted comment: timestamp:1555779966\tfile:test\n\
        QtKMXWyYcwdpZAlPF7tE2ENJkRd1ujvKjlj1m9RtHTBnZPa5WKU5uWRs5GoP5M/VqE81QFuMKI5k/SfNQUaOAA==\n";

    fn page(version: &semver::Version) -> String {
        format!("https://example.org/releases/tag/v{version}")
    }

    fn package_json(system: &str, arch: &str, kind: &str) -> Value {
        json!({
            "system": system,
            "arch": arch,
            "kind": kind,
            "name": format!("BitBraid-1.2.0-{arch}.{kind}"),
            "url": format!("https://example.org/download/BitBraid-1.2.0-{arch}.{kind}"),
            "size": 1234,
            "sha256": "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08",
            "signature": SIGNATURE,
        })
    }

    fn feed_json(packages: Vec<Value>) -> Value {
        json!({ "schema": 1, "version": "1.2.0", "packages": packages })
    }

    fn read(value: &Value) -> Result<Feed, Unreadable> {
        parse(value.to_string().as_bytes(), page)
    }

    #[test]
    fn a_feed_is_read_with_the_packages_this_build_knows() {
        let feed = read(&feed_json(vec![
            package_json("linux", "x86_64", "appimage"),
            package_json("linux", "x86_64", "deb"),
            package_json("macos", "universal", "app"),
            package_json("windows", "aarch64", "msi"),
            // Something a later release may add: passed over.
            package_json("freebsd", "x86_64", "pkg"),
            package_json("linux", "riscv64", "appimage"),
            package_json("linux", "x86_64", "flatpak"),
        ]))
        .unwrap();
        assert_eq!(feed.version, semver::Version::new(1, 2, 0));
        assert_eq!(
            feed.page.to_string(),
            "https://example.org/releases/tag/v1.2.0"
        );
        assert_eq!(feed.packages.len(), 4);
        let image = feed
            .package_for(System::Linux, Arch::X86_64, Kind::Appimage)
            .unwrap();
        assert_eq!(image.size, 1234);
        assert_eq!(image.sha256[0], 0x9f);
        assert_eq!(image.sha256[31], 0x08);
        assert_eq!(image.name, "BitBraid-1.2.0-x86_64.appimage");
        // macOS's bundle is for both processors.
        assert!(
            feed.package_for(System::Macos, Arch::Aarch64, Kind::App)
                .is_some()
        );
        assert!(
            feed.package_for(System::Linux, Arch::Aarch64, Kind::Appimage)
                .is_none()
        );
        assert!(
            feed.package_for(System::Windows, Arch::X86_64, Kind::Msi)
                .is_none()
        );
    }

    #[test]
    fn a_page_named_is_used_and_a_v_before_the_version_is_allowed() {
        let mut value = feed_json(vec![]);
        value["version"] = json!("v2.0.0-rc.1");
        value["page"] = json!("https://example.org/notes");
        let feed = read(&value).unwrap();
        assert_eq!(feed.version, semver::Version::parse("2.0.0-rc.1").unwrap());
        assert_eq!(feed.page.to_string(), "https://example.org/notes");
        // The release pipeline's writer calls it `notes`, beside a name and
        // a date that are not read.
        let mut written = feed_json(vec![]);
        written["notes"] = json!("https://example.org/releases/tag/v1.2.0");
        written["name"] = json!("BitBraid");
        written["date"] = json!("2026-11-02T10:00:00Z");
        let feed = read(&written).unwrap();
        assert_eq!(
            feed.page.to_string(),
            "https://example.org/releases/tag/v1.2.0"
        );
        written["page"] = json!("https://example.org/page");
        assert_eq!(
            read(&written).unwrap().page.to_string(),
            "https://example.org/page"
        );
    }

    #[test]
    fn what_is_wrong_in_a_feed_makes_it_unreadable_and_says_what() {
        let base = feed_json(vec![package_json("linux", "x86_64", "appimage")]);
        let broken = |change: &dyn Fn(&mut Value)| {
            let mut value = base.clone();
            change(&mut value);
            read(&value).unwrap_err().0
        };
        assert_eq!(parse(b"not json", page).unwrap_err().0, "it is not JSON");
        assert_eq!(parse(b"[]", page).unwrap_err().0, "it is not a JSON object");
        assert!(broken(&|v| v["schema"] = json!(2)).contains("later format"));
        assert_eq!(broken(&|v| v["schema"] = json!("1")), "its schema is not 1");
        assert!(broken(&|v| v["version"] = json!("1.2")).contains("is not one"));
        assert!(broken(&|v| v["version"] = json!(12)).contains("not a text"));
        assert!(broken(&|v| v["page"] = json!("http://example.org/")).contains("HTTPS"));
        assert!(broken(&|v| v["packages"] = json!({})).contains("no list"));
        let package = |key: &'static str, value: Value| {
            broken(&move |v| v["packages"][0][key] = value.clone())
        };
        assert!(package("url", json!("http://example.org/x")).contains("HTTPS"));
        assert!(package("url", json!("ftp://example.org/x")).contains("not an address"));
        assert!(package("size", json!(0)).contains("size"));
        assert!(package("size", json!(-5)).contains("size"));
        assert!(package("size", json!(MAX_PACKAGE_SIZE + 1)).contains("size"));
        assert!(package("size", json!(1.5)).contains("size"));
        assert!(package("sha256", json!("abc")).contains("sha256"));
        assert!(package("sha256", json!("zz".repeat(32))).contains("sha256"));
        assert!(package("name", json!("../evil")).contains("name"));
        assert!(package("name", json!(".hidden")).contains("name"));
        assert!(package("signature", json!("nonsense")).contains("signature"));
        let said = package("sha256", json!(""));
        assert!(
            said.starts_with("the linux x86_64 appimage package:"),
            "{said}"
        );
        let many = feed_json(vec![json!({}); MAX_PACKAGES + 1]);
        assert!(read(&many).unwrap_err().0.contains("more packages"));
        let large = vec![b' '; MAX_FEED + 1];
        assert!(parse(&large, page).unwrap_err().0.contains("larger"));
    }

    #[test]
    fn plain_http_is_taken_for_this_machine_only() {
        let mut value = feed_json(vec![package_json("linux", "x86_64", "appimage")]);
        value["packages"][0]["url"] = json!("http://127.0.0.1:9/x");
        value["page"] = json!("http://localhost:9/notes");
        assert!(read(&value).is_ok());
    }

    #[test]
    fn hostile_feeds_never_panic() {
        // A few kilobytes at most each: what is tested is that no shape of
        // value takes it down.
        let deep = format!("{}1{}", "[".repeat(500), "]".repeat(500));
        let mut samples: Vec<Vec<u8>> = vec![
            Vec::new(),
            b"{".to_vec(),
            b"null".to_vec(),
            b"\"text\"".to_vec(),
            deep.into_bytes(),
            vec![0xff; 100],
            br#"{"schema":1,"version":"\u0000","packages":[]}"#.to_vec(),
            br#"{"schema":1e400,"version":"1.0.0","packages":[]}"#.to_vec(),
            br#"{"schema":18446744073709551616,"version":"1.0.0","packages":[]}"#.to_vec(),
        ];
        let odd = [
            json!(null),
            json!(-1),
            json!(u64::MAX),
            json!(1e300),
            json!(""),
            json!("é".repeat(300)),
            json!([]),
            json!({}),
            json!("https://"),
            json!("https://example.org/\u{0}"),
            json!("untrusted comment:\n\ntrusted comment: \n"),
            json!("v"),
            json!("1.0.0-"),
            json!("99999999999999999999999.0.0"),
        ];
        for key in [
            "system",
            "arch",
            "kind",
            "name",
            "url",
            "size",
            "sha256",
            "signature",
        ] {
            for value in &odd {
                let mut package = package_json("linux", "x86_64", "appimage");
                package[key] = value.clone();
                samples.push(feed_json(vec![package]).to_string().into_bytes());
            }
        }
        for key in ["schema", "version", "page", "packages"] {
            for value in &odd {
                let mut feed = feed_json(vec![]);
                feed[key] = value.clone();
                samples.push(feed.to_string().into_bytes());
            }
        }
        for sample in &samples {
            let _ = parse(sample, page);
        }
    }
}
