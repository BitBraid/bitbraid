//! `cargo xtask feed <dir>`: the checksums, signatures and update feed of
//! a release, from the files gathered in `<dir>`.
//!
//! Every file there (the packages) is listed in `SHA256SUMS`, in the
//! format of sha256sum(1), and signed with minisign (`<file>.minisig`) by
//! the secret key given (`--key <file>`; its password, where it has one,
//! from `MINISIGN_PASSWORD`). Then `latest.json` is written: the version and,
//! for each package of each system, its address, size, SHA-256 and
//! signature, the feed the updater reads
//! (`crates/integrations/src/update/feed.rs`). The feed and `SHA256SUMS`
//! are signed as well. A signature's trusted comment names the file and
//! the version, so that a signed file cannot be passed off as another or
//! as another version's.
//!
//! Without a key the checksums are written and nothing is signed, and no
//! feed: an updater refuses a package without a signature, so a feed
//! without them would offer nothing.
//!
//! Every signature is minisign's hashed kind, the only one the updater
//! takes (`crates/integrations/src/update/verify.rs`); a legacy one stops
//! it. With `--public-key <file>` (the release workflow gives
//! `packaging/update-key.pub`, the key the application carries) every
//! signature is checked against that key as it is made: a secret key that
//! is not the built-in key's pair, or the placeholder in place of a key,
//! stops the release rather than publishing packages no updater takes.
//!
//! Sources: minisign(1) (`-S`, `-s`, `-m`, `-x`, `-t`, `-V`, `-p`,
//! `-G -W`; the signature file's four lines and the two bytes of its
//! algorithm, `ED` for a hashed signature and `Ed` for a legacy one, at
//! the start of its base64 line; the public key file's two lines; Ed25519
//! signatures are deterministic), sha256sum(1) (`<hash>  <name>`), the
//! GitHub documentation of release assets (`releases/download/<tag>/<name>`,
//! `releases/latest/download/<name>`).

use std::{
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::{bundle, package, sbom};

const USAGE: &str = "cargo xtask feed <dir> [--version <v>] [--base-url <url>] \
                     [--key <minisign secret key>] [--public-key <its public key>]";

/// The feed's form; the updater refuses another.
pub const SCHEMA: u64 = 1;

/// The files that go into the feed: each package's kind, system and
/// processor, by the file name of the release.
fn feed_entries(version: &str) -> Vec<(String, &'static str, &'static str, &'static str)> {
    let mut out = Vec::new();
    for (kind, system, arch) in [
        ("appimage", "linux", "x86_64"),
        ("deb", "linux", "x86_64"),
        ("rpm", "linux", "x86_64"),
        ("tarball", "linux", "x86_64"),
        ("dmg", "macos", "universal"),
        ("app", "macos", "universal"),
        ("msi", "windows", "x86_64"),
        ("zip", "windows", "x86_64"),
    ] {
        let name_arch = if arch == "universal" { "x86_64" } else { arch };
        if let Some(name) = package::file_name(kind, version, name_arch) {
            out.push((name, kind, system, arch));
        }
    }
    out
}

#[derive(Debug)]
struct Options {
    dir: PathBuf,
    version: Option<String>,
    base_url: Option<String>,
    key: Option<PathBuf>,
    public_key: Option<PathBuf>,
}

fn options(args: &[String]) -> Result<Options> {
    let mut dir = None;
    let mut options = Options {
        dir: PathBuf::new(),
        version: None,
        base_url: None,
        key: None,
        public_key: None,
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let mut value = || {
            args.next()
                .cloned()
                .with_context(|| format!("{arg} wants a value\n{USAGE}"))
        };
        match arg.as_str() {
            "--version" => options.version = Some(value()?),
            "--base-url" => options.base_url = Some(value()?),
            "--key" => options.key = Some(PathBuf::from(value()?)),
            "--public-key" => options.public_key = Some(PathBuf::from(value()?)),
            other if other.starts_with('-') => bail!("feed: {other}: not an option\n{USAGE}"),
            other => {
                if dir.replace(PathBuf::from(other)).is_some() {
                    bail!("feed: one directory\n{USAGE}");
                }
            }
        }
    }
    options.dir = dir.with_context(|| format!("feed: which directory?\n{USAGE}"))?;
    if options.public_key.is_some() && options.key.is_none() {
        bail!("feed: --public-key checks what --key signs; give both\n{USAGE}");
    }
    Ok(options)
}

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args)?;
    let root = crate::root();
    let version = match options.version {
        Some(version) => version,
        None => bundle::workspace_version(&root)?,
    };
    let base_url = options
        .base_url
        .unwrap_or_else(|| format!("{}/releases/download/v{version}", sbom::REPOSITORY));
    let epoch = sbom::source_date_epoch(&root)?;
    let signer = match &options.key {
        Some(key) => Some(Signer::new(key, options.public_key.as_deref())?),
        None => None,
    };
    write_release(&options.dir, &version, &base_url, epoch, signer.as_ref())
}

/// Signs files with minisign.
#[derive(Debug)]
pub struct Signer {
    key: PathBuf,
    password: Option<String>,
    program: String,
    /// The public key each signature is checked against, when given.
    public_key: Option<PathBuf>,
}

impl Signer {
    pub fn new(key: &Path, public_key: Option<&Path>) -> Result<Self> {
        if !key.is_file() {
            bail!("feed: {}: no secret key there", key.display());
        }
        if let Some(public_key) = public_key {
            let text = std::fs::read_to_string(public_key)
                .with_context(|| format!("reading {}", public_key.display()))?;
            check_public_key(&text).with_context(|| format!("feed: {}", public_key.display()))?;
        }
        Ok(Self {
            key: key.to_path_buf(),
            password: std::env::var("MINISIGN_PASSWORD")
                .ok()
                .filter(|p| !p.is_empty()),
            program: std::env::var("MINISIGN").unwrap_or_else(|_| "minisign".to_owned()),
            public_key: public_key.map(Path::to_path_buf),
        })
    }

    /// `<file>.minisig` beside the file, with the trusted comment given.
    fn sign(&self, file: &Path, comment: &str) -> Result<String> {
        let signature = PathBuf::from(format!("{}.minisig", file.display()));
        let mut command = Command::new(&self.program);
        command
            .arg("-S")
            .arg("-s")
            .arg(&self.key)
            .arg("-m")
            .arg(file)
            .arg("-x")
            .arg(&signature)
            .arg("-t")
            .arg(comment)
            .stdin(Stdio::piped())
            .stdout(Stdio::null());
        let mut child = command
            .spawn()
            .with_context(|| format!("starting {} (set MINISIGN)", self.program))?;
        // Its input is closed either way; a closed pipe means it did not ask.
        if let Some(mut stdin) = child.stdin.take()
            && let Some(password) = &self.password
        {
            let _ = writeln!(stdin, "{password}");
        }
        let status = child.wait()?;
        if !status.success() {
            bail!("minisign could not sign {}: {status}", file.display());
        }
        let text = std::fs::read_to_string(&signature)
            .with_context(|| format!("reading {}", signature.display()))?;
        if !is_hashed(&text) {
            bail!(
                "{}: a legacy signature, which the updater refuses; sign with minisign 0.10 or later",
                signature.display()
            );
        }
        if let Some(public_key) = &self.public_key {
            let out = Command::new(&self.program)
                .arg("-V")
                .arg("-p")
                .arg(public_key)
                .arg("-m")
                .arg(file)
                .arg("-x")
                .arg(&signature)
                .stdin(Stdio::null())
                .output()
                .with_context(|| format!("starting {}", self.program))?;
            let said = String::from_utf8_lossy(&out.stdout);
            if !out.status.success() {
                bail!(
                    "{}: the signature does not hold for {}: the secret key is not the pair of the key the application carries",
                    file.display(),
                    public_key.display()
                );
            }
            if !said.contains(&format!("Trusted comment: {comment}")) {
                bail!(
                    "{}: minisign read another trusted comment: {said}",
                    file.display()
                );
            }
        }
        Ok(text)
    }
}

/// Whether a `.minisig` file's signature is minisign's hashed kind: its
/// base64 line starts with the algorithm's two bytes, `ED` (a legacy one
/// has `Ed`), which in base64 begin `RU` (`RW` for `Ed`).
fn is_hashed(signature: &str) -> bool {
    signature
        .lines()
        .nth(1)
        .is_some_and(|line| line.starts_with("RU"))
}

/// A public key file as `minisign -G` writes it: a comment, then the key
/// (`Ed` and the key's id and bytes, `RW...` in base64). The placeholder
/// in `packaging/update-key.pub` has `none` there.
fn check_public_key(text: &str) -> Result<()> {
    let key = text.lines().nth(1).unwrap_or_default().trim();
    if key == "none" {
        bail!(
            "the placeholder, not a key: replace it with the public key of the secret key the release is signed with"
        );
    }
    if !key.starts_with("RW") || key.len() != 56 {
        bail!("not a minisign public key");
    }
    Ok(())
}

/// The trusted comment of a release's file: its name and the version.
fn trusted_comment(name: &str, version: &str) -> String {
    format!("file:{name} version:{version}")
}

/// What a file is, for the checksums and the feed.
struct Seen {
    name: String,
    size: u64,
    sha256: String,
}

fn look_at(path: &Path) -> Result<Seen> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Seen {
        name: path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default(),
        size: bytes.len() as u64,
        sha256: sha2::Sha256::digest(&bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
    })
}

/// The files of the release in `dir`: every file there but what this
/// writes, in the order of their names.
fn release_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_file()))
        .map(|entry| entry.path())
        .filter(|path| {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            !name.ends_with(".minisig")
                && name != "SHA256SUMS"
                && name != "latest.json"
                && !name.starts_with('.')
        })
        .collect();
    files.sort();
    Ok(files)
}

/// The checksums, signatures and feed of the release in `dir`.
pub fn write_release(
    dir: &Path,
    version: &str,
    base_url: &str,
    epoch: u64,
    signer: Option<&Signer>,
) -> Result<()> {
    let files = release_files(dir)?;
    if files.is_empty() {
        bail!("feed: {} holds no files", dir.display());
    }
    let mut seen = Vec::new();
    let mut signatures = std::collections::BTreeMap::new();
    for file in &files {
        let info = look_at(file)?;
        if let Some(signer) = signer {
            signatures.insert(
                info.name.clone(),
                signer.sign(file, &trusted_comment(&info.name, version))?,
            );
        }
        seen.push(info);
    }
    let sums: String = seen
        .iter()
        .map(|info| format!("{}  {}\n", info.sha256, info.name))
        .collect();
    let sums_path = dir.join("SHA256SUMS");
    std::fs::write(&sums_path, sums)?;
    println!("feed: {} ({} files)", sums_path.display(), seen.len());

    let Some(signer) = signer else {
        println!(
            "feed: no key: nothing signed, and no latest.json (an updater takes no unsigned package)"
        );
        return Ok(());
    };
    signer.sign(&sums_path, &trusted_comment("SHA256SUMS", version))?;
    let feed = feed(version, base_url, epoch, &seen, &signatures)?;
    let feed_path = dir.join("latest.json");
    let mut text = serde_json::to_string_pretty(&feed)?;
    text.push('\n');
    std::fs::write(&feed_path, text)?;
    signer.sign(&feed_path, &trusted_comment("latest.json", version))?;
    println!(
        "feed: {} ({} packages), every file signed",
        feed_path.display(),
        feed["packages"].as_array().map_or(0, Vec::len)
    );
    Ok(())
}

/// The feed: the packages there are of each system, with what an updater
/// checks before it installs one.
fn feed(
    version: &str,
    base_url: &str,
    epoch: u64,
    seen: &[Seen],
    signatures: &std::collections::BTreeMap<String, String>,
) -> Result<Value> {
    let base = base_url.trim_end_matches('/');
    let mut packages = Vec::new();
    for (name, kind, system, arch) in feed_entries(version) {
        let Some(info) = seen.iter().find(|info| info.name == name) else {
            continue;
        };
        let signature = signatures
            .get(&name)
            .with_context(|| format!("{name} has no signature"))?;
        packages.push(json!({
            "system": system,
            "arch": arch,
            "kind": kind,
            "name": name,
            "url": format!("{base}/{name}"),
            "size": info.size,
            "sha256": info.sha256,
            "signature": signature,
        }));
    }
    if packages.is_empty() {
        bail!("feed: none of the release's packages is there");
    }
    Ok(json!({
        "schema": SCHEMA,
        "name": "BitBraid",
        "version": version,
        "date": sbom::rfc3339(epoch),
        "notes": format!("{}/releases/tag/v{version}", sbom::REPOSITORY),
        "packages": packages,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bitbraid-feed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn options_are_read() {
        let read =
            |args: &[&str]| options(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
        let all = read(&[
            "dist",
            "--version",
            "1.0.0",
            "--key",
            "/k",
            "--base-url",
            "https://x",
        ])
        .unwrap();
        assert_eq!(all.dir, PathBuf::from("dist"));
        assert_eq!(all.version.as_deref(), Some("1.0.0"));
        assert_eq!(all.key, Some(PathBuf::from("/k")));
        assert_eq!(all.public_key, None);
        let checked = read(&["a", "--key", "/k", "--public-key", "/p"]).unwrap();
        assert_eq!(checked.public_key, Some(PathBuf::from("/p")));
        // A key to check against, and nothing signed to check: a mistake.
        assert!(read(&["a", "--public-key", "/p"]).is_err());
        assert!(read(&[]).is_err());
        assert!(read(&["a", "b"]).is_err());
        assert!(read(&["a", "--key"]).is_err());
        assert!(read(&["a", "--sign"]).is_err());
    }

    #[test]
    fn only_hashed_signatures_and_real_public_keys_pass() {
        // `ED` (hashed) begins `RU` in base64, `Ed` (legacy) `RW`.
        let signature = |line: &str| {
            format!("untrusted comment: x\n{line}\ntrusted comment: file:a version:1\nAAAA\n")
        };
        assert!(is_hashed(&signature("RUQf6LRCGA9i5yg9f00")));
        assert!(!is_hashed(&signature("RWQf6LRCGA9i5yg9f00")));
        assert!(!is_hashed("nothing"));
        let placeholder =
            std::fs::read_to_string(crate::root().join("packaging/update-key.pub")).unwrap();
        if placeholder.lines().nth(1) == Some("none") {
            let refused = check_public_key(&placeholder).unwrap_err().to_string();
            assert!(refused.contains("placeholder"), "{refused}");
        }
        let key = "untrusted comment: minisign public key 1\n\
                   RWQf6LRCGA9i53mlYecO4IzT51TGPpvWucNSCh1CBM0QTaLn73Y7GFO3\n";
        check_public_key(key).unwrap();
        assert!(check_public_key("untrusted comment: x\nRWshort\n").is_err());
        assert!(check_public_key("").is_err());
    }

    #[test]
    fn without_a_key_there_are_checksums_and_no_feed() {
        let dir = scratch("unsigned");
        std::fs::write(dir.join("bitbraid-1.0.0-x86_64.AppImage"), b"appimage").unwrap();
        std::fs::write(dir.join("bitbraid-1.0.0-linux.cdx.json"), b"{}").unwrap();
        write_release(&dir, "1.0.0", "https://example.invalid/v1.0.0", 0, None).unwrap();
        let sums = std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap();
        assert_eq!(
            sums,
            "44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a  bitbraid-1.0.0-linux.cdx.json\n\
             8b408ed68dfd56d503752ff2ee2ecb3c0ffa55a26f6fa107bd4444c3943ee6e1  bitbraid-1.0.0-x86_64.AppImage\n"
        );
        assert!(!dir.join("latest.json").exists());
        // Written again, it lists the same files, not its own.
        write_release(&dir, "1.0.0", "https://example.invalid/v1.0.0", 0, None).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("SHA256SUMS")).unwrap(),
            sums
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_feed_lists_each_package_with_what_is_checked() {
        let seen = vec![
            Seen {
                name: "bitbraid-1.0.0-x86_64.AppImage".into(),
                size: 3,
                sha256: "aa".into(),
            },
            Seen {
                name: "bitbraid-1.0.0-universal.dmg".into(),
                size: 4,
                sha256: "bb".into(),
            },
            Seen {
                name: "bitbraid-1.0.0-linux.cdx.json".into(),
                size: 2,
                sha256: "cc".into(),
            },
        ];
        let signatures = std::collections::BTreeMap::from([
            (
                "bitbraid-1.0.0-x86_64.AppImage".to_owned(),
                "sig a".to_owned(),
            ),
            (
                "bitbraid-1.0.0-universal.dmg".to_owned(),
                "sig b".to_owned(),
            ),
            (
                "bitbraid-1.0.0-linux.cdx.json".to_owned(),
                "sig c".to_owned(),
            ),
        ]);
        let out = feed(
            "1.0.0",
            "https://example.invalid/v1.0.0/",
            1_792_251_863,
            &seen,
            &signatures,
        )
        .unwrap();
        assert_eq!(out["schema"], 1);
        assert_eq!(out["version"], "1.0.0");
        assert_eq!(out["date"], "2026-10-17T15:44:23Z");
        let packages = out["packages"].as_array().unwrap();
        assert_eq!(packages.len(), 2);
        assert_eq!(packages[0]["system"], "linux");
        assert_eq!(packages[0]["kind"], "appimage");
        assert_eq!(
            packages[0]["url"],
            "https://example.invalid/v1.0.0/bitbraid-1.0.0-x86_64.AppImage"
        );
        assert_eq!(packages[0]["size"], 3);
        assert_eq!(packages[0]["signature"], "sig a");
        assert_eq!(packages[1]["system"], "macos");
        assert_eq!(packages[1]["arch"], "universal");
        // A package without its signature is an error, not a gap.
        let unsigned = std::collections::BTreeMap::new();
        assert!(feed("1.0.0", "https://x", 0, &seen, &unsigned).is_err());
        assert!(feed("1.0.0", "https://x", 0, &seen[2..], &signatures).is_err());
    }

    /// Signs with a key made for the test, and checks each signature with
    /// minisign itself. Skipped where minisign is not installed.
    #[test]
    fn every_file_is_signed_and_the_signatures_hold() {
        if Command::new("minisign").arg("-v").output().is_err() {
            println!("skipped: minisign is not installed");
            return;
        }
        let dir = scratch("signed");
        let keys = scratch("keys");
        let made = Command::new("minisign")
            .args(["-G", "-W", "-f", "-p"])
            .arg(keys.join("pub"))
            .arg("-s")
            .arg(keys.join("sec"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        std::fs::write(dir.join("bitbraid-1.0.0-x86_64.AppImage"), b"appimage").unwrap();
        std::fs::write(dir.join("bitbraid_1.0.0_amd64.deb"), b"deb").unwrap();
        // Checked against its own public key as it signs, as the release
        // workflow checks against the application's.
        let signer = Signer {
            key: keys.join("sec"),
            password: None,
            program: "minisign".to_owned(),
            public_key: Some(keys.join("pub")),
        };
        write_release(
            &dir,
            "1.0.0",
            "https://example.invalid/v1.0.0",
            0,
            Some(&signer),
        )
        .unwrap();
        assert!(is_hashed(
            &std::fs::read_to_string(dir.join("bitbraid_1.0.0_amd64.deb.minisig")).unwrap()
        ));

        // Another key pair's public key: the release stops.
        let made = Command::new("minisign")
            .args(["-G", "-W", "-f", "-p"])
            .arg(keys.join("other.pub"))
            .arg("-s")
            .arg(keys.join("other.sec"))
            .stdout(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        let mismatched = Signer {
            public_key: Some(keys.join("other.pub")),
            ..Signer::new(&keys.join("sec"), None).unwrap()
        };
        let stopped = write_release(
            &dir,
            "1.0.0",
            "https://example.invalid/v1.0.0",
            0,
            Some(&mismatched),
        )
        .unwrap_err();
        assert!(
            format!("{stopped:#}").contains("not the pair"),
            "{stopped:#}"
        );
        write_release(
            &dir,
            "1.0.0",
            "https://example.invalid/v1.0.0",
            0,
            Some(&signer),
        )
        .unwrap();
        for name in [
            "bitbraid-1.0.0-x86_64.AppImage",
            "bitbraid_1.0.0_amd64.deb",
            "SHA256SUMS",
            "latest.json",
        ] {
            let out = Command::new("minisign")
                .arg("-V")
                .arg("-p")
                .arg(keys.join("pub"))
                .arg("-m")
                .arg(dir.join(name))
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            let said = String::from_utf8_lossy(&out.stdout);
            assert!(
                said.contains(&format!("file:{name} version:1.0.0")),
                "{said}"
            );
        }
        let feed: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("latest.json")).unwrap())
                .unwrap();
        let first = &feed["packages"][0];
        assert_eq!(first["kind"], "appimage");
        assert_eq!(
            first["signature"].as_str().unwrap(),
            std::fs::read_to_string(dir.join("bitbraid-1.0.0-x86_64.AppImage.minisig")).unwrap()
        );
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&keys);
    }
}
