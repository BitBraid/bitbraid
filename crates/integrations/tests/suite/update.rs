//! The updater against a feed and packages served over TLS on this
//! machine, signed with a key of the tests' own
//! (`bitbraid_testkit::Signer`), the way GitHub serves a release's files:
//! the latest release's address redirects to the release's, which
//! redirects to where the file is.
//!
//! A good update replaces the AppImage, keeps the old one until the new
//! one starts and asks nothing that names anybody; a package that fails a
//! check (another key, a signature that does not hold, the older kind of
//! signature, a checksum, a size either way, a signature of another
//! release) is not installed and is deleted; an older or equal version is
//! not installed; a feed that is malformed, hostile or not there is said;
//! no network is said, and a download that stops part way; through a proxy
//! the feed and the package come in a tunnel; a build without a key asks
//! nothing; a signature made by minisign itself is taken; the commands
//! answer as the registry has them; on Unix a macOS bundle is replaced with
//! the one its package holds, and on macOS with the one its disk image
//! holds.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{
    http::{Client, Proxies, ProxyAddress},
    update::{
        self, Arch, Installation, Kind, Refusal, System, Target, Then, UpdateError, UpdateKey,
        Updater,
    },
};
use bitbraid_testkit::{Answer, Asked, Authority, Outcome, Proxy, ProxyOptions, Signer, StandIn};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const AGENT: &str = "BitBraid-Test/1.0";
const OLD: &[u8] = b"the copy that runs now";
const NAME: &str = "BitBraid-1.2.0-x86_64.AppImage";

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A folder of the test's own, deleted when it ends.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = bitbraid_testkit::scratch_root()
            .join(format!("update-{name}-{}-{n}", std::process::id()));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Bytes that look like nothing in particular, the same for the same seed.
fn bytes(length: usize, seed: u64) -> Vec<u8> {
    let mut state = seed
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    (0..length)
        .map(|_| {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as u8
        })
        .collect()
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// A TLS server on this machine for `names`, answering each path with
/// what was put there, and 404 otherwise.
struct Site {
    server: StandIn,
    authority: Vec<u8>,
    pages: Arc<Mutex<HashMap<String, Answer>>>,
}

impl Site {
    fn start(names: &[&str]) -> Self {
        let pages: Arc<Mutex<HashMap<String, Answer>>> = Arc::default();
        let served = pages.clone();
        let authority = Authority::new().unwrap();
        let config = authority.server_config(names).unwrap();
        let server = StandIn::start(Some(config), move |asked: &Asked| {
            let path = asked.target.split('?').next().unwrap_or_default();
            served.lock().unwrap().get(path).cloned().unwrap_or(Answer {
                status: 404,
                headers: Vec::new(),
                body: b"not here".to_vec(),
            })
        })
        .unwrap();
        Self {
            server,
            authority: authority.certificate_der(),
            pages,
        }
    }

    fn url(&self, host: &str, path: &str) -> String {
        format!("https://{host}:{}{path}", self.server.port())
    }

    fn put(&self, path: &str, answer: Answer) {
        self.pages.lock().unwrap().insert(path.to_owned(), answer);
    }

    fn file(&self, path: &str, body: Vec<u8>) {
        self.put(
            path,
            Answer {
                status: 200,
                headers: vec![(
                    "Content-Type".to_owned(),
                    "application/octet-stream".to_owned(),
                )],
                body,
            },
        );
    }

    fn redirect(&self, path: &str, to: &str) {
        self.put(
            path,
            Answer {
                status: 302,
                headers: vec![("Location".to_owned(), to.to_owned())],
                body: Vec::new(),
            },
        );
    }

    /// The paths asked so far, in order.
    fn asked(&self) -> Vec<String> {
        self.server
            .asked()
            .into_iter()
            .map(|asked| asked.target)
            .collect()
    }
}

/// Serve `bytes` as the package `name` of `version` as GitHub serves a
/// release's file (a redirect to `object_host`, where it is), and say it
/// in a feed's entry, signed by `signer` with the trusted comment the
/// release pipeline writes.
fn package(
    site: &Site,
    hosts: (&str, &str),
    signer: &Signer,
    version: &str,
    name: &str,
    bytes: &[u8],
) -> Value {
    let (host, object_host) = hosts;
    let download = format!("/releases/download/v{version}/{name}");
    let object = format!("/objects/{name}");
    site.redirect(&download, &site.url(object_host, &object));
    site.file(&object, bytes.to_vec());
    json!({
        "system": "linux",
        "arch": "x86_64",
        "kind": "appimage",
        "name": name,
        "url": site.url(host, &download),
        "size": bytes.len(),
        "sha256": sha256(bytes),
        "signature": signer.sign(bytes, &format!("version:{version} file:{name}")),
    })
}

/// Serve a feed of `version` listing `packages` at the latest release's
/// address, and give that address.
fn feed(site: &Site, host: &str, version: &str, packages: Vec<Value>) -> String {
    let body = json!({
        "schema": 1,
        "version": version,
        "page": site.url(host, &format!("/releases/tag/v{version}")),
        "packages": packages,
    });
    serve_feed(site, host, version, body.to_string().into_bytes())
}

/// Serve `body` as the feed, behind the latest release's redirect.
fn serve_feed(site: &Site, host: &str, version: &str, body: Vec<u8>) -> String {
    let at = format!("/releases/download/v{version}/latest.json");
    site.redirect("/releases/latest/download/latest.json", &at);
    site.file(&at, body);
    site.url(host, "/releases/latest/download/latest.json")
}

/// The updater of a copy at version 1.0.0 on Linux, reading `feed`, with
/// `signer`'s key.
fn updater(
    site: &Site,
    feed: String,
    signer: &Signer,
    installation: Installation,
    downloads: &Path,
) -> Updater {
    Updater {
        fetch: Arc::new(
            Client::trusting(AGENT, vec![site.authority.clone()]).with_proxies(Proxies::none()),
        ),
        key: UpdateKey::from_text(&signer.public_key()),
        feed,
        current: semver::Version::new(1, 0, 0),
        installation,
        target: Some(Target {
            system: System::Linux,
            arch: Arch::X86_64,
        }),
        downloads: Some(downloads.to_path_buf()),
    }
}

/// The AppImage that runs: a file in `dir`, runnable.
fn appimage(dir: &Dir) -> PathBuf {
    let image = dir.path().join("BitBraid.AppImage");
    fs::write(&image, OLD).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&image, fs::Permissions::from_mode(0o750)).unwrap();
    }
    image
}

fn go() -> AtomicBool {
    AtomicBool::new(false)
}

fn version(text: &str) -> semver::Version {
    semver::Version::parse(text).unwrap()
}

#[test]
fn a_good_update_replaces_the_appimage_and_keeps_the_old_one_until_the_new_one_starts() {
    let dir = Dir::new("good");
    let image = appimage(&dir);
    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    // Larger than a read of the client, so that it comes in pieces.
    let new = bytes(300_000, 7);
    let entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        &new,
    );
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    );

    let found = update::check(&updater, &go()).unwrap();
    assert!(found.newer && found.installs, "{found:?}");
    assert_eq!(
        (found.current.as_str(), found.latest.as_str()),
        ("1.0.0", "1.2.0")
    );
    assert_eq!(found.installation, "appimage");
    assert_eq!(found.why_not, None);
    assert_eq!(found.page, site.url("localhost", "/releases/tag/v1.2.0"));
    let offered = found.package.unwrap();
    assert_eq!((offered.name.as_str(), offered.size), (NAME, 300_000));
    // A look downloads nothing.
    assert!(!site.asked().iter().any(|path| path.contains(NAME)));

    let mut said = Vec::new();
    let installed = update::install(&updater, Some(&version("1.2.0")), &go(), &mut |percent| {
        said.push(percent)
    })
    .unwrap();
    assert_eq!(installed.then, Then::Restart);
    assert_eq!(installed.installed, "1.2.0");
    assert_eq!(fs::read(&image).unwrap(), new);
    let old = dir.path().join("BitBraid.AppImage.old");
    assert_eq!(fs::read(&old).unwrap(), OLD, "the old copy is kept");
    assert!(!dir.path().join(".BitBraid.AppImage.download").exists());
    assert_eq!(said.last(), Some(&100));
    assert!(said.windows(2).all(|pair| pair[0] < pair[1]), "{said:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = fs::metadata(&image).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o750, "the new copy runs as the old one did");
    }

    // The feed for the look and again for the installation, the package
    // once, each through its redirects; nothing that names anybody.
    let feed_paths = [
        "/releases/latest/download/latest.json",
        "/releases/download/v1.2.0/latest.json",
    ];
    let mut expected: Vec<String> = feed_paths
        .iter()
        .chain(feed_paths.iter())
        .map(|path| (*path).to_owned())
        .collect();
    expected.push(format!("/releases/download/v1.2.0/{NAME}"));
    expected.push(format!("/objects/{NAME}"));
    assert_eq!(site.asked(), expected);
    for asked in site.server.asked() {
        assert_eq!(asked.method, "GET");
        assert_eq!(asked.header("user-agent"), Some(AGENT));
        for secret in ["authorization", "cookie", "private-token"] {
            assert_eq!(asked.header(secret), None);
        }
    }

    // The new copy, once it starts, deletes the old one.
    update::tidy(&updater.installation, updater.downloads.as_deref());
    assert!(!old.exists());
    assert_eq!(fs::read(&image).unwrap(), new);
}

#[test]
fn a_package_that_fails_a_check_is_not_installed_and_is_deleted() {
    let signer = Signer::new(1).unwrap();
    let new = bytes(20_000, 3);
    let comment = format!("version:1.2.0 file:{NAME}");
    type Change<'a> = Box<dyn Fn(&mut Value) + 'a>;
    let cases: Vec<(&str, Change<'_>, Refusal)> = vec![
        (
            "signed with another key",
            Box::new(|entry| {
                entry["signature"] = json!(Signer::new(2).unwrap().sign(&new, &comment))
            }),
            Refusal::OtherKey,
        ),
        (
            "a signature of other bytes",
            Box::new(|entry| entry["signature"] = json!(signer.sign(b"something else", &comment))),
            Refusal::Signature,
        ),
        (
            "the older kind of signature",
            Box::new(|entry| entry["signature"] = json!(signer.sign_unhashed(&new, &comment))),
            Refusal::OldKind,
        ),
        (
            "another checksum",
            Box::new(|entry| entry["sha256"] = json!(sha256(b"something else"))),
            Refusal::Checksum,
        ),
        (
            "said larger than it is",
            Box::new(|entry| entry["size"] = json!(20_010)),
            Refusal::Size {
                got: 20_000,
                expected: 20_010,
            },
        ),
        (
            "said smaller than it is",
            Box::new(|entry| entry["size"] = json!(19_990)),
            Refusal::TooLarge { expected: 19_990 },
        ),
        (
            "a signature of another release",
            Box::new(|entry| {
                entry["signature"] = json!(signer.sign(&new, &format!("version:1.1.0 file:{NAME}")))
            }),
            Refusal::NotThisPackage {
                comment: format!("version:1.1.0 file:{NAME}"),
            },
        ),
        (
            "a signature of another file",
            Box::new(|entry| {
                entry["signature"] =
                    json!(signer.sign(&new, "version:1.2.0 file:BitBraid-1.2.0-x86_64.deb"))
            }),
            Refusal::NotThisPackage {
                comment: "version:1.2.0 file:BitBraid-1.2.0-x86_64.deb".to_owned(),
            },
        ),
    ];
    for (what, change, refusal) in cases {
        let dir = Dir::new("refused");
        let image = appimage(&dir);
        let site = Site::start(&["localhost"]);
        let mut entry = package(
            &site,
            ("localhost", "localhost"),
            &signer,
            "1.2.0",
            NAME,
            &new,
        );
        change(&mut entry);
        let address = feed(&site, "localhost", "1.2.0", vec![entry]);
        let updater = updater(
            &site,
            address,
            &signer,
            Installation::AppImage(image.clone()),
            &dir.path().join("downloads"),
        );
        let err = update::install(&updater, None, &go(), &mut |_| {}).unwrap_err();
        assert_eq!(
            err,
            UpdateError::Refused {
                version: "1.2.0".to_owned(),
                refusal: refusal.clone(),
            },
            "{what}"
        );
        let said = err.to_string();
        assert!(
            said.starts_with("the download of BitBraid 1.2.0 was not installed: "),
            "{what}: {said}"
        );
        assert_eq!(
            fs::read(&image).unwrap(),
            OLD,
            "{what}: the copy is as it was"
        );
        let left: Vec<_> = fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(
            left,
            ["BitBraid.AppImage"],
            "{what}: nothing is left beside it"
        );
    }
}

#[test]
fn an_older_or_equal_version_is_not_installed() {
    let signer = Signer::new(1).unwrap();
    for (latest, newer) in [
        ("1.0.0", false),
        ("0.9.0", false),
        ("1.0.0+build.5", false),
        ("1.0.0-rc.1", false),
        ("1.0.1", true),
    ] {
        let dir = Dir::new("older");
        let image = appimage(&dir);
        let site = Site::start(&["localhost"]);
        let name = format!("BitBraid-{latest}-x86_64.AppImage");
        let entry = package(
            &site,
            ("localhost", "localhost"),
            &signer,
            latest,
            &name,
            b"package",
        );
        let address = feed(&site, "localhost", latest, vec![entry]);
        let updater = updater(
            &site,
            address,
            &signer,
            Installation::AppImage(image.clone()),
            &dir.path().join("downloads"),
        );
        let found = update::check(&updater, &go()).unwrap();
        assert_eq!(found.newer, newer, "{latest}");
        if !newer {
            assert_eq!(
                update::install(&updater, None, &go(), &mut |_| {}),
                Err(UpdateError::Newest("1.0.0".to_owned())),
                "{latest}"
            );
            assert!(!site.asked().iter().any(|path| path.contains(&name)));
            assert_eq!(fs::read(&image).unwrap(), OLD);
        }
    }
    // The feed names another version than the one that was offered.
    let dir = Dir::new("changed");
    let site = Site::start(&["localhost"]);
    let entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.3.0",
        NAME,
        b"package",
    );
    let address = feed(&site, "localhost", "1.3.0", vec![entry]);
    let updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(appimage(&dir)),
        &dir.path().join("downloads"),
    );
    assert_eq!(
        update::install(&updater, Some(&version("1.2.0")), &go(), &mut |_| {}),
        Err(UpdateError::Changed {
            now: "1.3.0".to_owned(),
            offered: "1.2.0".to_owned()
        })
    );
}

#[test]
fn a_copy_that_does_not_update_itself_says_why_and_downloads_nothing() {
    let signer = Signer::new(1).unwrap();
    let dir = Dir::new("elsewhere");
    let site = Site::start(&["localhost"]);
    let entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        b"package",
    );
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    for installation in [
        Installation::Package(PathBuf::from("/usr/bin/bitbraid")),
        Installation::Elsewhere(PathBuf::from("/home/u/bitbraid/bitbraid")),
    ] {
        let updater = updater(
            &site,
            address.clone(),
            &signer,
            installation.clone(),
            dir.path(),
        );
        let found = update::check(&updater, &go()).unwrap();
        assert!(found.newer && !found.installs);
        assert_eq!(found.package, None);
        let why = found.why_not.unwrap();
        assert!(
            why.contains(&installation.path().display().to_string()),
            "{why}"
        );
        let err = update::install(&updater, None, &go(), &mut |_| {}).unwrap_err();
        assert_eq!(err, UpdateError::NotHere(why));
    }
    // An AppImage on a processor the release has none for.
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(appimage(&dir)),
        dir.path(),
    );
    updater.target = Some(Target {
        system: System::Linux,
        arch: Arch::Aarch64,
    });
    let found = update::check(&updater, &go()).unwrap();
    assert!(!found.installs);
    assert!(
        found
            .why_not
            .unwrap()
            .contains("no AppImage for Linux on aarch64")
    );
    assert!(!site.asked().iter().any(|path| path.contains(NAME)));
}

#[test]
fn a_feed_that_is_malformed_hostile_or_not_there_is_said_without_a_panic() {
    let signer = Signer::new(1).unwrap();
    let deep = format!("{}{}", "[".repeat(300), "]".repeat(300));
    let large = format!("{{\"schema\":1,\"pad\":\"{}\"}}", "x".repeat(70 * 1024));
    let mut hostile: Vec<(Vec<u8>, &str)> = vec![
        (b"not json".to_vec(), "it is not JSON"),
        (b"{}".to_vec(), "its schema is not 1"),
        (deep.into_bytes(), "it is not JSON"),
        (vec![0xff; 64], "it is not JSON"),
        (large.into_bytes(), "larger than a feed is"),
        (
            br#"{"schema":2,"version":"9.0.0","packages":[]}"#.to_vec(),
            "later format",
        ),
        (
            br#"{"schema":1,"version":"1.2.0","packages":{}}"#.to_vec(),
            "no list of packages",
        ),
        (
            br#"{"schema":1,"version":"1.2.0","packages":[{"system":"linux","arch":"x86_64","kind":"appimage","name":"a","url":"http://example.org/a","size":1,"sha256":"00","signature":"x"}]}"#.to_vec(),
            "not an HTTPS address",
        ),
    ];
    // Values of every shape in every field of a package: refused or passed
    // over, never a panic.
    let elsewhere = Site::start(&["localhost"]);
    let sound = package(
        &elsewhere,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        b"x",
    );
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
        for value in [
            json!(null),
            json!(-1),
            json!([]),
            json!("\u{0}"),
            json!(1e300),
        ] {
            let mut entry = sound.clone();
            entry[key] = value;
            let body = json!({"schema": 1, "version": "1.2.0", "packages": [entry]});
            hostile.push((body.to_string().into_bytes(), ""));
        }
    }
    let dir = Dir::new("hostile");
    for (body, said) in hostile {
        let site = Site::start(&["localhost"]);
        let address = serve_feed(&site, "localhost", "1.2.0", body);
        let updater = updater(
            &site,
            address.clone(),
            &signer,
            Installation::AppImage(dir.path().join("BitBraid.AppImage")),
            dir.path(),
        );
        match update::check(&updater, &go()) {
            Err(UpdateError::Feed { url, why }) => {
                assert_eq!(url, address);
                assert!(why.contains(said), "{why} lacks {said}");
            }
            // A package passed over leaves a feed without it.
            Ok(found) => assert!(said.is_empty() && !found.installs, "{found:?}"),
            Err(other) => panic!("{other}"),
        }
    }

    // Not there, and redirects without end or down to plain HTTP.
    let site = Site::start(&["localhost"]);
    let updater_at = |path: &str| {
        updater(
            &site,
            site.url("localhost", path),
            &signer,
            Installation::AppImage(dir.path().join("BitBraid.AppImage")),
            dir.path(),
        )
    };
    let why = |path: &str| match update::check(&updater_at(path), &go()) {
        Err(UpdateError::Feed { why, .. }) => why,
        other => panic!("{other:?}"),
    };
    assert_eq!(why("/nothing.json"), "it answered with status 404");
    site.redirect("/loop.json", "/loop.json");
    assert!(why("/loop.json").contains("too many redirects"));
    site.redirect("/down.json", "http://localhost:9/latest.json");
    assert!(why("/down.json").contains("HTTPS to plain HTTP"));
}

#[test]
fn with_no_network_or_when_stopped_it_says_so() {
    let signer = Signer::new(1).unwrap();
    let dir = Dir::new("offline");
    let site = Site::start(&["localhost"]);
    let closed = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let updater = updater(
        &site,
        format!("https://localhost:{closed}/latest.json"),
        &signer,
        Installation::AppImage(appimage(&dir)),
        dir.path(),
    );
    match update::check(&updater, &go()) {
        Err(UpdateError::Feed { why, .. }) => assert!(why.starts_with("no connection"), "{why}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(
        update::check(&updater, &AtomicBool::new(true)),
        Err(UpdateError::Cancelled)
    );
}

/// A download whose server goes away part way, mid-transfer: said as a
/// connection that ended, with how much came, not as "the answer is not
/// one that can be read: the body ends early"; nothing is installed and
/// nothing is left beside the copy.
#[test]
fn a_download_that_stops_part_way_says_so_and_changes_nothing() {
    use std::io::{Read as _, Write as _};
    let signer = Signer::new(1).unwrap();
    let new = bytes(20_000, 5);
    let dir = Dir::new("cut");
    let image = appimage(&dir);
    let site = Site::start(&["localhost"]);
    // A server on this machine that gives the whole length, sends a part
    // of it and goes.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let part = new.get(..6_000).unwrap().to_vec();
    let length = new.len();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap() == 1 {
            head.push(byte[0]);
        }
        write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        stream.write_all(&part).unwrap();
        stream.flush().unwrap();
    });
    let mut entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        &new,
    );
    entry["url"] = json!(format!("http://127.0.0.1:{port}/{NAME}"));
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    );
    let err = update::install(&updater, None, &go(), &mut |_| {}).unwrap_err();
    server.join().unwrap();
    assert_eq!(
        err,
        UpdateError::Download {
            version: "1.2.0".to_owned(),
            why: "the connection ended after 6000 of its 20000 bytes".to_owned(),
        }
    );
    assert_eq!(
        err.to_string(),
        "the download of BitBraid 1.2.0 failed: the connection ended after 6000 of its 20000 bytes"
    );
    assert_eq!(fs::read(&image).unwrap(), OLD, "the copy is as it was");
    let left: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(left, ["BitBraid.AppImage"], "nothing is left beside it");
}

#[test]
fn through_a_proxy_the_feed_and_the_package_come_in_a_tunnel() {
    let dir = Dir::new("proxy");
    let image = appimage(&dir);
    // Names that resolve nowhere: what reaches the server went through
    // the proxy.
    let site = Site::start(&["updates.example.test", "objects.example.test"]);
    let signer = Signer::new(1).unwrap();
    let new = bytes(50_000, 11);
    let entry = package(
        &site,
        ("updates.example.test", "objects.example.test"),
        &signer,
        "1.2.0",
        NAME,
        &new,
    );
    let address = feed(&site, "updates.example.test", "1.2.0", vec![entry]);
    let proxy = Proxy::start(ProxyOptions::default()).unwrap();
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    );
    updater.fetch = Arc::new(
        Client::trusting(AGENT, vec![site.authority.clone()]).with_proxies(Proxies::through(
            ProxyAddress::parse(&proxy.url()).unwrap(),
            "the test",
        )),
    );
    let installed = update::install(&updater, None, &go(), &mut |_| {}).unwrap();
    assert_eq!(installed.then, Then::Restart);
    assert_eq!(fs::read(&image).unwrap(), new);
    let port = site.server.port();
    let tunnels: Vec<String> = proxy
        .passages()
        .into_iter()
        .map(|passage| {
            assert_eq!(passage.method, "CONNECT");
            assert_eq!(passage.outcome, Outcome::Passed);
            passage.target
        })
        .collect();
    let feed_host = format!("updates.example.test:{port}");
    let object_host = format!("objects.example.test:{port}");
    assert_eq!(
        tunnels,
        [&feed_host, &feed_host, &feed_host, &object_host].map(String::clone)
    );
}

#[test]
fn a_build_without_a_key_asks_nothing() {
    let signer = Signer::new(1).unwrap();
    let dir = Dir::new("keyless");
    let site = Site::start(&["localhost"]);
    let entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        b"package",
    );
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(appimage(&dir)),
        dir.path(),
    );
    // The placeholder `packaging/update-key.pub` holds until a release
    // key replaces it.
    updater.key = UpdateKey::from_text("untrusted comment: placeholder: no update key yet\nnone\n");
    assert!(matches!(updater.key, UpdateKey::Missing));
    let status = update::status(&updater);
    assert!(!status.key && status.installs_itself);
    assert!(status.why_no_key.unwrap().contains("no update key"));
    assert_eq!(update::check(&updater, &go()), Err(UpdateError::NoKey));
    assert_eq!(
        update::install(&updater, None, &go(), &mut |_| {}),
        Err(UpdateError::NoKey)
    );
    updater.key = UpdateKey::from_text("untrusted comment: x\nnot a key\n");
    assert!(matches!(
        update::check(&updater, &go()),
        Err(UpdateError::BadKey(_))
    ));
    assert!(site.asked().is_empty(), "{:?}", site.asked());
    assert!(UpdateError::NoKey.to_string().contains("no update key"));
}

/// A feed as the release pipeline's writer (`cargo xtask feed`) makes it:
/// `notes` for the page, a `name` and a `date` beside, its own file names
/// and the trusted comment `file:<name> version:<v>`. Its AppImage is
/// installed; and a macOS bundle, for which it lists only the disk image,
/// takes that.
#[test]
fn a_feed_as_the_release_pipeline_writes_it_is_taken() {
    let dir = Dir::new("pipeline");
    let image = appimage(&dir);
    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    let new = bytes(10_000, 9);
    let name = "bitbraid-1.2.0-x86_64.AppImage";
    let mut appimage_entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        name,
        &new,
    );
    appimage_entry["signature"] = json!(signer.sign(&new, &format!("file:{name} version:1.2.0")));
    let other = |kind: &str, system: &str, arch: &str, file: &str| {
        let mut entry = appimage_entry.clone();
        entry["kind"] = json!(kind);
        entry["system"] = json!(system);
        entry["arch"] = json!(arch);
        entry["name"] = json!(file);
        entry["url"] = json!(site.url("localhost", &format!("/releases/download/v1.2.0/{file}")));
        entry
    };
    let packages = vec![
        appimage_entry.clone(),
        other("deb", "linux", "x86_64", "bitbraid_1.2.0_amd64.deb"),
        other("rpm", "linux", "x86_64", "bitbraid-1.2.0-1.x86_64.rpm"),
        other(
            "tarball",
            "linux",
            "x86_64",
            "bitbraid-1.2.0-x86_64-linux.tar.gz",
        ),
        other("dmg", "macos", "universal", "bitbraid-1.2.0-universal.dmg"),
        other("msi", "windows", "x86_64", "bitbraid-1.2.0-x86_64.msi"),
        other(
            "zip",
            "windows",
            "x86_64",
            "bitbraid-1.2.0-x86_64-windows.zip",
        ),
    ];
    let notes = site.url("localhost", "/releases/tag/v1.2.0");
    let body = json!({
        "schema": 1,
        "name": "BitBraid",
        "version": "1.2.0",
        "date": "2026-11-02T10:00:00Z",
        "notes": notes,
        "packages": packages,
    });
    let address = serve_feed(&site, "localhost", "1.2.0", body.to_string().into_bytes());
    let updater = updater(
        &site,
        address.clone(),
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    );
    let found = update::check(&updater, &go()).unwrap();
    assert_eq!(found.page, notes);
    assert!(found.newer && found.installs);
    update::install(&updater, None, &go(), &mut |_| {}).unwrap();
    assert_eq!(fs::read(&image).unwrap(), new);

    let mut mac = self::updater(
        &site,
        address,
        &signer,
        Installation::Bundle(PathBuf::from("/Applications/BitBraid.app")),
        &dir.path().join("downloads"),
    );
    mac.target = Some(Target {
        system: System::Macos,
        arch: Arch::Aarch64,
    });
    let found = update::check(&mac, &go()).unwrap();
    assert!(found.installs, "{found:?}");
    let offered = found.package.unwrap();
    assert_eq!(
        (offered.kind, offered.name.as_str()),
        (Kind::Dmg, "bitbraid-1.2.0-universal.dmg")
    );
}

#[test]
fn a_signature_made_by_minisign_itself_is_taken() {
    // Made once with minisign 0.12 (`-G -W`, then `-S -t "version:1.2.0
    // file:…"`); the secret key was not kept.
    let folder = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/minisign");
    let package_bytes = fs::read(folder.join(NAME)).unwrap();
    let signature = fs::read_to_string(folder.join(format!("{NAME}.minisig"))).unwrap();
    let public = fs::read_to_string(folder.join("test.pub")).unwrap();
    let dir = Dir::new("minisign");
    let image = appimage(&dir);
    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    let mut entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        &package_bytes,
    );
    entry["signature"] = json!(signature);
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    );
    updater.key = UpdateKey::from_text(&public);
    update::install(&updater, None, &go(), &mut |_| {}).unwrap();
    assert_eq!(fs::read(&image).unwrap(), package_bytes);
}

#[test]
fn the_commands_look_and_install_with_progress_and_refuse_hostile_parameters() {
    let dir = Dir::new("commands");
    let image = appimage(&dir);
    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    let new = bytes(40_000, 5);
    let entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        NAME,
        &new,
    );
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let registry = bitbraid_integrations::registry();
    let mut ctx = CommandContext::default();
    ctx.extensions.insert(Arc::new(updater(
        &site,
        address,
        &signer,
        Installation::AppImage(image.clone()),
        &dir.path().join("downloads"),
    )));
    let progress: Arc<Mutex<Vec<bitbraid_git::Progress>>> = Arc::default();
    let heard = progress.clone();
    ctx.progress = Some(Arc::new(move |step| heard.lock().unwrap().push(step)));

    // What this copy is, asked of nobody.
    let status = registry.run("update.status", &mut ctx, &json!({})).unwrap();
    assert!(site.asked().is_empty());
    assert_eq!(status["key"], json!(true));
    assert_eq!(status["installs_itself"], json!(true));
    assert_eq!(status["installation"], json!("appimage"));
    assert_eq!(status["current"], json!("1.0.0"));
    assert_eq!(status["path"], json!(image.display().to_string()));

    let found = registry.run("update.check", &mut ctx, &json!({})).unwrap();
    assert_eq!(found["newer"], json!(true));
    assert_eq!(found["installs"], json!(true));
    assert_eq!(found["latest"], json!("1.2.0"));
    assert_eq!(found["installation"], json!("appimage"));
    assert_eq!(found["package"]["kind"], json!("appimage"));

    for hostile in [
        json!({"version": 12}),
        json!({"version": "not a version"}),
        json!({"version": ["1.2.0"]}),
        json!({"version": "1".repeat(5000)}),
    ] {
        match registry.run("update.install", &mut ctx, &hostile) {
            Err(CommandError::InvalidParam { name, .. }) => assert_eq!(name, "version"),
            other => panic!("{hostile}: {other:?}"),
        }
    }
    assert_eq!(fs::read(&image).unwrap(), OLD);
    // Another version than the one offered: refused, in words.
    let err = registry
        .run("update.install", &mut ctx, &json!({"version": "1.1.0"}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("now names BitBraid 1.2.0"), "{err}");

    let installed = registry
        .run("update.install", &mut ctx, &json!({"version": "v1.2.0"}))
        .unwrap();
    assert_eq!(installed["installed"], json!("1.2.0"));
    assert_eq!(installed["then"], json!("restart"));
    assert_eq!(fs::read(&image).unwrap(), new);
    let steps = progress.lock().unwrap().clone();
    assert!(
        steps
            .iter()
            .all(|step| step.stage == "Downloading BitBraid 1.2.0")
    );
    assert_eq!(steps.last().and_then(|step| step.percent), Some(100));
}

/// The macOS bundle's way, on Unix with the system's `tar`: what it does
/// with folders, held on every Unix.
#[cfg(unix)]
#[test]
#[allow(clippy::print_stderr)] // saying why it did not run
fn a_bundle_is_replaced_with_the_one_its_package_holds() {
    use std::process::Command;
    if Command::new("tar").arg("--version").output().is_err() {
        eprintln!("skipped: no tar on this machine");
        return;
    }
    let dir = Dir::new("bundle");
    let bundle = dir.path().join("BitBraid.app");
    fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    fs::write(bundle.join("Contents/MacOS/bitbraid"), OLD).unwrap();
    fs::write(bundle.join("Contents/Info.plist"), "old").unwrap();
    // The package: a tarball with the new bundle at its top.
    let making = dir.path().join("making");
    fs::create_dir_all(making.join("BitBraid.app/Contents/MacOS")).unwrap();
    fs::write(
        making.join("BitBraid.app/Contents/MacOS/bitbraid"),
        b"new program",
    )
    .unwrap();
    fs::write(making.join("BitBraid.app/Contents/Info.plist"), "new").unwrap();
    let tarball = dir.path().join("package.tar.gz");
    let made = Command::new("tar")
        .arg("-czf")
        .arg(&tarball)
        .arg("-C")
        .arg(&making)
        .arg("BitBraid.app")
        .status()
        .unwrap();
    assert!(made.success());
    let package_bytes = fs::read(&tarball).unwrap();

    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    let name = "BitBraid-1.2.0-macos-universal.app.tar.gz";
    let mut entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        name,
        &package_bytes,
    );
    entry["system"] = json!("macos");
    entry["arch"] = json!("universal");
    entry["kind"] = json!("app");
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let downloads = dir.path().join("downloads");
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::Bundle(bundle.clone()),
        &downloads,
    );
    updater.target = Some(Target {
        system: System::Macos,
        arch: Arch::Aarch64,
    });
    let installed = update::install(&updater, None, &go(), &mut |_| {}).unwrap();
    assert_eq!(installed.then, Then::Restart);
    assert_eq!(
        fs::read(bundle.join("Contents/MacOS/bitbraid")).unwrap(),
        b"new program"
    );
    let old = dir.path().join("BitBraid.app.old");
    assert_eq!(fs::read(old.join("Contents/MacOS/bitbraid")).unwrap(), OLD);
    assert!(!dir.path().join(".BitBraid.app.update").exists());
    // Once the new copy starts, the old one and the downloads are gone.
    update::tidy(&updater.installation, Some(&downloads));
    assert!(!old.exists());
    assert!(!downloads.exists());
    assert!(bundle.join("Contents/MacOS/bitbraid").is_file());
}

/// The macOS bundle's way where a release has no tarball of the bundle:
/// its disk image attached read only where nobody sees it, the bundle
/// copied out and the image detached again, nothing left attached.
#[cfg(target_os = "macos")]
#[test]
fn on_macos_a_bundle_is_replaced_from_its_disk_image_and_nothing_stays_attached() {
    use std::process::Command;
    let dir = Dir::new("dmg");
    let bundle = dir.path().join("BitBraid.app");
    fs::create_dir_all(bundle.join("Contents/MacOS")).unwrap();
    fs::write(bundle.join("Contents/MacOS/bitbraid"), OLD).unwrap();
    // The disk image as the release makes it: the bundle and a link to
    // the Applications folder.
    let making = dir.path().join("making");
    fs::create_dir_all(making.join("BitBraid.app/Contents/MacOS")).unwrap();
    fs::write(
        making.join("BitBraid.app/Contents/MacOS/bitbraid"),
        b"new program",
    )
    .unwrap();
    std::os::unix::fs::symlink("/Applications", making.join("Applications")).unwrap();
    let image = dir.path().join("package.dmg");
    let made = Command::new("/usr/bin/hdiutil")
        .args([
            "create", "-quiet", "-format", "UDZO", "-volname", "BitBraid",
        ])
        .arg("-srcfolder")
        .arg(&making)
        .arg(&image)
        .status()
        .unwrap();
    assert!(made.success());
    let package_bytes = fs::read(&image).unwrap();

    let site = Site::start(&["localhost"]);
    let signer = Signer::new(1).unwrap();
    let name = "bitbraid-1.2.0-universal.dmg";
    let mut entry = package(
        &site,
        ("localhost", "localhost"),
        &signer,
        "1.2.0",
        name,
        &package_bytes,
    );
    entry["system"] = json!("macos");
    entry["arch"] = json!("universal");
    entry["kind"] = json!("dmg");
    let address = feed(&site, "localhost", "1.2.0", vec![entry]);
    let downloads = dir.path().join("downloads");
    let mut updater = updater(
        &site,
        address,
        &signer,
        Installation::Bundle(bundle.clone()),
        &downloads,
    );
    updater.target = Some(Target {
        system: System::Macos,
        arch: Arch::Aarch64,
    });
    let installed = update::install(&updater, None, &go(), &mut |_| {}).unwrap();
    assert_eq!(installed.then, Then::Restart);
    assert_eq!(
        fs::read(bundle.join("Contents/MacOS/bitbraid")).unwrap(),
        b"new program"
    );
    let old = dir.path().join("BitBraid.app.old");
    assert_eq!(fs::read(old.join("Contents/MacOS/bitbraid")).unwrap(), OLD);
    assert!(!dir.path().join(".BitBraid.app.update").exists());
    let attached = Command::new("/usr/bin/hdiutil")
        .arg("info")
        .output()
        .unwrap();
    let attached = String::from_utf8_lossy(&attached.stdout);
    assert!(
        !attached.contains(dir.path().to_string_lossy().as_ref()),
        "nothing stays attached: {attached}"
    );
    assert_eq!(fs::read_dir(&downloads).unwrap().count(), 0);
}
