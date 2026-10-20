//! `cargo xtask sbom`: what the application is built from, for each
//! system it is released for. Two things come from one walk of
//! cargo's resolve, from the `bitbraid` package along its normal
//! dependencies for the system's target (what is linked into the binary;
//! build scripts and procedural macros are not): a software bill of
//! materials in CycloneDX 1.5 JSON, released beside the packages, and the
//! licence texts of those crates, which every package carries
//! (`THIRD-PARTY-LICENSES.txt`).
//!
//! Both are written the same for the same lock file and version: the
//! time in the bill is `SOURCE_DATE_EPOCH`'s, and its serial number is
//! made from the lock file, the version and the system.
//!
//! Sources: the CycloneDX 1.5 JSON reference (`bomFormat`, `specVersion`,
//! `serialNumber`, `metadata.tools.components`, `metadata.component`,
//! `components` with `purl`, `licenses` (`expression`), `hashes`
//! (`SHA-256`) and `externalReferences`, `dependencies`); the package-url
//! specification (`pkg:cargo/<name>@<version>`, `pkg:github/...`);
//! cargo-metadata(1) (`--filter-platform`, the resolve's `dep_kinds`);
//! the Cargo book on `Cargo.lock`'s checksums (SHA-256 of the `.crate`
//! file) and on `license` and `license-file`; RFC 9562 (the version and
//! variant bits of a name-based UUID).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind, MetadataCommand};
use serde_json::{Value, json};
use sha2::Digest as _;

/// The systems a release is for, each with the target its dependencies
/// are resolved for. The macOS binary is universal; its two processors
/// have the same dependencies.
pub const SYSTEMS: [(&str, &str); 3] = [
    ("linux", "x86_64-unknown-linux-gnu"),
    ("macos", "aarch64-apple-darwin"),
    ("windows", "x86_64-pc-windows-msvc"),
];

/// The repository the application comes from: the workspace's
/// `repository`, so that a fork's releases point at the fork.
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");

/// The longest licence file that is read.
const LONGEST_LICENCE: u64 = 512 * 1024;

/// A crate linked into the application.
#[derive(Debug, Clone)]
pub struct Crate {
    pub name: String,
    pub version: String,
    pub license: Option<String>,
    pub repository: Option<String>,
    /// SHA-256 of the `.crate` file, from the lock file.
    pub checksum: Option<String>,
    /// Where its sources are.
    pub dir: PathBuf,
    pub license_file: Option<PathBuf>,
    /// The package-urls of the crates it depends on.
    pub depends_on: BTreeSet<String>,
}

impl Crate {
    pub fn purl(&self) -> String {
        purl(&self.name, &self.version)
    }
}

fn purl(name: &str, version: &str) -> String {
    format!("pkg:cargo/{name}@{version}")
}

/// The crates of others linked into the application for `target`, in the
/// order of their names, and those the application's own crates depend on
/// directly. The workspace's crates are the application itself.
pub fn shipped_crates(root: &Path, target: &str) -> Result<(Vec<Crate>, BTreeSet<String>)> {
    let metadata = MetadataCommand::new()
        .manifest_path(root.join("Cargo.toml"))
        .other_options(vec![
            "--locked".to_owned(),
            "--filter-platform".to_owned(),
            target.to_owned(),
        ])
        .exec()
        .context("cargo metadata")?;
    let resolve = metadata
        .resolve
        .as_ref()
        .context("cargo metadata gave no resolve")?;
    let packages: BTreeMap<_, _> = metadata.packages.iter().map(|p| (&p.id, p)).collect();
    let nodes: BTreeMap<_, _> = resolve.nodes.iter().map(|n| (&n.id, n)).collect();
    let checksums = lock_checksums(root)?;

    let app = metadata
        .packages
        .iter()
        .find(|p| p.name.as_str() == "bitbraid" && p.source.is_none())
        .context("the workspace has no bitbraid package")?;
    // From the application along normal dependencies only.
    let mut seen = BTreeSet::new();
    let mut queue = vec![&app.id];
    while let Some(id) = queue.pop() {
        if !seen.insert(id) {
            continue;
        }
        let node = nodes
            .get(id)
            .with_context(|| format!("{id} is not in the resolve"))?;
        for dep in &node.deps {
            if dep
                .dep_kinds
                .iter()
                .any(|kind| kind.kind == DependencyKind::Normal)
            {
                queue.push(&dep.pkg);
            }
        }
    }

    let mut crates = Vec::new();
    let mut root_deps = BTreeSet::new();
    for id in &seen {
        let package = packages
            .get(id)
            .with_context(|| format!("{id} is not among the packages"))?;
        let node = nodes
            .get(id)
            .with_context(|| format!("{id} is not in the resolve"))?;
        let normal: BTreeSet<String> = node
            .deps
            .iter()
            .filter(|dep| {
                dep.dep_kinds
                    .iter()
                    .any(|kind| kind.kind == DependencyKind::Normal)
            })
            .filter_map(|dep| packages.get(&dep.pkg))
            .filter(|dep| dep.source.is_some())
            .map(|dep| purl(dep.name.as_str(), &dep.version.to_string()))
            .collect();
        if package.source.is_none() {
            // One of ours: part of the application.
            root_deps.extend(normal);
            continue;
        }
        let version = package.version.to_string();
        crates.push(Crate {
            checksum: checksums
                .get(&(package.name.to_string(), version.clone()))
                .cloned(),
            name: package.name.to_string(),
            version,
            license: package.license.clone(),
            repository: package.repository.clone(),
            dir: package
                .manifest_path
                .parent()
                .map(|dir| dir.as_std_path().to_path_buf())
                .unwrap_or_default(),
            license_file: package.license_file().map(|f| f.into_std_path_buf()),
            depends_on: normal,
        });
    }
    crates.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok((crates, root_deps))
}

/// The checksum of each package in the lock file, by name and version.
fn lock_checksums(root: &Path) -> Result<BTreeMap<(String, String), String>> {
    let lock: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("Cargo.lock")).context("reading Cargo.lock")?,
    )
    .context("reading Cargo.lock")?;
    let mut out = BTreeMap::new();
    for package in lock
        .get("package")
        .and_then(toml::Value::as_array)
        .into_iter()
        .flatten()
    {
        let text = |key: &str| package.get(key).and_then(toml::Value::as_str);
        if let (Some(name), Some(version), Some(checksum)) =
            (text("name"), text("version"), text("checksum"))
        {
            out.insert((name.to_owned(), version.to_owned()), checksum.to_owned());
        }
    }
    Ok(out)
}

/// A licence as an SPDX expression: the old `MIT/Apache-2.0` read as
/// `MIT OR Apache-2.0`.
fn expression(license: &str) -> String {
    license
        .split('/')
        .map(str::trim)
        .collect::<Vec<_>>()
        .join(" OR ")
}

/// Seconds since 1970 as an RFC 3339 time in UTC.
pub fn rfc3339(seconds: u64) -> String {
    let (year, month, day) = crate::archive::civil_from_days(seconds / 86_400);
    let in_day = seconds % 86_400;
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        in_day / 3600,
        (in_day % 3600) / 60,
        in_day % 60
    )
}

/// A UUID of the form RFC 9562 gives one made from a name (version 5's
/// bits), from SHA-256 of `parts`.
fn uuid_of(parts: &[&[u8]]) -> String {
    let mut hash = sha2::Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    let digest = hash.finalize();
    let mut bytes = [0u8; 16];
    for (to, from) in bytes.iter_mut().zip(digest.iter()) {
        *to = *from;
    }
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        hex.get(0..8).unwrap_or_default(),
        hex.get(8..12).unwrap_or_default(),
        hex.get(12..16).unwrap_or_default(),
        hex.get(16..20).unwrap_or_default(),
        hex.get(20..32).unwrap_or_default()
    )
}

/// The bill of materials of the application `version` for `system`.
pub fn cyclonedx(
    version: &str,
    system: &str,
    target: &str,
    crates: &[Crate],
    root_deps: &BTreeSet<String>,
    epoch: u64,
    lock: &[u8],
) -> Value {
    let path = REPOSITORY.trim_start_matches("https://github.com/");
    let root_ref = format!("pkg:github/{path}@v{version}");
    let components: Vec<Value> = crates
        .iter()
        .map(|krate| {
            let mut component = json!({
                "type": "library",
                "bom-ref": krate.purl(),
                "name": krate.name,
                "version": krate.version,
                "purl": krate.purl(),
            });
            if let Some(license) = &krate.license {
                component["licenses"] = json!([{ "expression": expression(license) }]);
            } else if let Some(file) = &krate.license_file {
                let name = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                component["licenses"] =
                    json!([{ "license": { "name": format!("see {name} in the crate") } }]);
            }
            if let Some(checksum) = &krate.checksum {
                component["hashes"] = json!([{ "alg": "SHA-256", "content": checksum }]);
            }
            if let Some(repository) = &krate.repository {
                component["externalReferences"] = json!([{ "type": "vcs", "url": repository }]);
            }
            component
        })
        .collect();
    let mut dependencies = vec![json!({ "ref": root_ref, "dependsOn": root_deps })];
    dependencies.extend(
        crates
            .iter()
            .map(|krate| json!({ "ref": krate.purl(), "dependsOn": krate.depends_on })),
    );
    json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.5",
        "serialNumber": format!(
            "urn:uuid:{}",
            uuid_of(&[version.as_bytes(), system.as_bytes(), lock])
        ),
        "version": 1,
        "metadata": {
            "timestamp": rfc3339(epoch),
            "tools": {
                "components": [{
                    "type": "application",
                    "name": "cargo xtask sbom",
                    "version": version,
                }]
            },
            "component": {
                "type": "application",
                "bom-ref": root_ref,
                "name": "bitbraid",
                "version": version,
                "description": "BitBraid, a git client",
                "licenses": [{ "expression": "MIT" }],
                "purl": root_ref,
                "externalReferences": [{ "type": "vcs", "url": REPOSITORY }],
                "properties": [{ "name": "cargo:target", "value": target }],
            },
        },
        "components": components,
        "dependencies": dependencies,
    })
}

/// The files of a crate that hold its licence: `license-file`, and the
/// files at its top (or in a `LICENSES` directory) named as licences are.
fn licence_files(krate: &Crate) -> Vec<PathBuf> {
    let looks_like = |name: &str| {
        let upper = name.to_ascii_uppercase();
        [
            "LICENSE",
            "LICENCE",
            "COPYING",
            "NOTICE",
            "COPYRIGHT",
            "UNLICENSE",
        ]
        .iter()
        .any(|start| upper.starts_with(start))
    };
    let mut found = BTreeSet::new();
    if let Some(file) = &krate.license_file {
        found.insert(file.clone());
    }
    let list = |dir: &Path| -> Vec<(String, PathBuf, bool)> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| {
                (
                    entry.file_name().to_string_lossy().into_owned(),
                    entry.path(),
                    entry.file_type().is_ok_and(|t| t.is_dir()),
                )
            })
            .collect()
    };
    for (name, path, is_dir) in list(&krate.dir) {
        if is_dir && name.eq_ignore_ascii_case("licenses") {
            for (_, inner, inner_dir) in list(&path) {
                if !inner_dir {
                    found.insert(inner);
                }
            }
        } else if !is_dir && looks_like(&name) {
            found.insert(path);
        }
    }
    found.into_iter().filter(|path| path.is_file()).collect()
}

/// The licence texts of the crates, each text once with the crates that
/// ship it, after a list of every crate with its licence and origin.
pub fn third_party_text(version: &str, target: &str, crates: &[Crate]) -> Result<String> {
    // The texts by their contents: who ships each, under which name.
    let mut texts: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut without = Vec::new();
    for krate in crates {
        let files = licence_files(krate);
        if files.is_empty() {
            without.push(format!("{} {}", krate.name, krate.version));
        }
        for file in files {
            let name = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            let length = std::fs::metadata(&file).map(|m| m.len()).unwrap_or(0);
            if length > LONGEST_LICENCE {
                bail!(
                    "{}: longer than a licence is expected to be",
                    file.display()
                );
            }
            let bytes =
                std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
            let text = String::from_utf8_lossy(&bytes).replace("\r\n", "\n");
            let text = text.trim().to_owned();
            if text.is_empty() {
                continue;
            }
            texts
                .entry(text)
                .or_default()
                .insert(format!("{} {} ({name})", krate.name, krate.version));
        }
    }

    let mut out = String::new();
    let title = format!("Third-party software in BitBraid {version} ({target})");
    out.push_str(&format!("{title}\n{}\n\n", "=".repeat(title.len())));
    out.push_str(
        "BitBraid is built with the Rust crates listed here, each under the\n\
         licence it declares. Their licence texts follow the list, each text\n\
         once, with the crates that ship it. BitBraid's own licence, and the\n\
         terms for its name and logo, are in LICENSE.\n\n",
    );
    out.push_str("Crates\n------\n\n");
    for krate in crates {
        out.push_str(&format!(
            "{} {}: {}{}\n",
            krate.name,
            krate.version,
            krate
                .license
                .as_deref()
                .map(expression)
                .unwrap_or_else(|| "see its licence file".to_owned()),
            krate
                .repository
                .as_deref()
                .map(|url| format!(", {url}"))
                .unwrap_or_default()
        ));
    }
    if !without.is_empty() {
        out.push_str(&format!(
            "\nThese crates ship no licence text of their own; the licence they\n\
             declare above applies: {}.\n",
            without.join(", ")
        ));
    }
    out.push_str("\nLicence texts\n-------------\n");
    // In the order of the first crate that ships each, for a reader.
    let mut ordered: Vec<(String, BTreeSet<String>)> = texts.into_iter().collect();
    ordered.sort_by(|a, b| a.1.iter().next().cmp(&b.1.iter().next()));
    for (text, shipped_by) in ordered {
        out.push_str(&format!(
            "\n----------------------------------------------------------------------\n\
             {}\n\n{text}\n",
            shipped_by.into_iter().collect::<Vec<_>>().join(", ")
        ));
    }
    Ok(out)
}

/// The time a release is dated: `SOURCE_DATE_EPOCH`, else the time of the
/// commit checked out.
pub fn source_date_epoch(root: &Path) -> Result<u64> {
    if let Some(value) = std::env::var_os("SOURCE_DATE_EPOCH") {
        let text = value.to_string_lossy();
        return text
            .trim()
            .parse()
            .with_context(|| format!("SOURCE_DATE_EPOCH={text}: not a number of seconds"));
    }
    let out = std::process::Command::new("git")
        .args(["log", "-1", "--format=%ct", "HEAD"])
        .current_dir(root)
        .output()
        .context("running git for the time of the last commit")?;
    if !out.status.success() {
        bail!("git could not say when the last commit was made: set SOURCE_DATE_EPOCH");
    }
    String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse()
        .context("git's time of the last commit is not a number")
}

const USAGE: &str = "cargo xtask sbom [--out <dir>]";

pub fn run(args: &[String]) -> Result<()> {
    let root = crate::root();
    let mut out = crate::bundle::target_dir(&root).join("packages");
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--out" => {
                out = PathBuf::from(
                    args.next()
                        .with_context(|| format!("--out wants a directory\n{USAGE}"))?,
                );
            }
            other => bail!("sbom: {other}: not an option\n{USAGE}"),
        }
    }
    let version = crate::bundle::workspace_version(&root)?;
    let epoch = source_date_epoch(&root)?;
    let lock = std::fs::read(root.join("Cargo.lock")).context("reading Cargo.lock")?;
    std::fs::create_dir_all(&out)?;
    for (system, target) in SYSTEMS {
        let (crates, root_deps) = shipped_crates(&root, target)?;
        let bom = cyclonedx(&version, system, target, &crates, &root_deps, epoch, &lock);
        let path = out.join(format!("bitbraid-{version}-{system}.cdx.json"));
        let mut text = serde_json::to_string_pretty(&bom)?;
        text.push('\n');
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        println!("sbom: {} ({} crates)", path.display(), crates.len());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn krate(name: &str, license: Option<&str>) -> Crate {
        Crate {
            name: name.to_owned(),
            version: "1.0.0".to_owned(),
            license: license.map(str::to_owned),
            repository: Some(format!("https://example.invalid/{name}")),
            checksum: Some("ab".repeat(32)),
            dir: PathBuf::from("/nowhere"),
            license_file: None,
            depends_on: BTreeSet::new(),
        }
    }

    #[test]
    fn the_bill_is_cyclonedx_and_the_same_every_time() {
        let mut a = krate("alpha", Some("MIT/Apache-2.0"));
        a.depends_on.insert(purl("beta", "1.0.0"));
        let crates = [a, krate("beta", None)];
        let roots = BTreeSet::from([purl("alpha", "1.0.0")]);
        let bom = cyclonedx(
            "1.0.0",
            "linux",
            "x86_64-unknown-linux-gnu",
            &crates,
            &roots,
            1_792_251_863,
            b"lock",
        );
        assert_eq!(bom["bomFormat"], "CycloneDX");
        assert_eq!(bom["specVersion"], "1.5");
        let serial = bom["serialNumber"].as_str().unwrap();
        // RFC 9562's form, version 5, variant 10.
        let uuid = serial.strip_prefix("urn:uuid:").unwrap();
        assert_eq!(uuid.len(), 36);
        assert_eq!(&uuid[14..15], "5");
        assert!("89ab".contains(&uuid[19..20]));
        assert_eq!(bom["metadata"]["timestamp"], "2026-10-17T15:44:23Z");
        assert_eq!(bom["components"][0]["purl"], "pkg:cargo/alpha@1.0.0");
        assert_eq!(
            bom["components"][0]["licenses"][0]["expression"],
            "MIT OR Apache-2.0"
        );
        assert_eq!(bom["components"][0]["hashes"][0]["alg"], "SHA-256");
        assert!(bom["components"][1].get("licenses").is_none());
        assert_eq!(
            bom["dependencies"][0]["dependsOn"][0],
            "pkg:cargo/alpha@1.0.0"
        );
        assert_eq!(
            bom["dependencies"][1]["dependsOn"][0],
            "pkg:cargo/beta@1.0.0"
        );
        let again = cyclonedx(
            "1.0.0",
            "linux",
            "x86_64-unknown-linux-gnu",
            &crates,
            &roots,
            1_792_251_863,
            b"lock",
        );
        assert_eq!(bom, again);
        let other = cyclonedx(
            "1.0.0",
            "windows",
            "x86_64-pc-windows-msvc",
            &crates,
            &roots,
            1_792_251_863,
            b"lock",
        );
        assert_ne!(bom["serialNumber"], other["serialNumber"]);
    }

    #[test]
    fn licence_texts_are_gathered_once_each() {
        let scratch =
            std::env::temp_dir().join(format!("bitbraid-sbom-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&scratch);
        let one = scratch.join("one");
        let two = scratch.join("two");
        std::fs::create_dir_all(one.join("LICENSES")).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        std::fs::write(one.join("LICENSE-MIT"), "MIT text\r\n").unwrap();
        std::fs::write(one.join("LICENSES/Apache-2.0.txt"), "Apache text").unwrap();
        std::fs::write(one.join("README.md"), "not a licence").unwrap();
        std::fs::write(two.join("COPYING"), "MIT text\n").unwrap();
        let mut first = krate("one", Some("MIT OR Apache-2.0"));
        first.dir = one;
        let mut second = krate("two", Some("MIT"));
        second.dir = two;
        let third = krate("three", Some("Zlib"));
        let text =
            third_party_text("1.0.0", "x86_64-unknown-linux-gnu", &[first, second, third]).unwrap();
        let _ = std::fs::remove_dir_all(&scratch);
        assert!(text.starts_with("Third-party software in BitBraid 1.0.0"));
        assert!(text.contains("one 1.0.0: MIT OR Apache-2.0, https://example.invalid/one\n"));
        // The same text from two crates is there once, naming both.
        assert_eq!(text.matches("MIT text").count(), 1);
        assert!(text.contains("one 1.0.0 (LICENSE-MIT), two 1.0.0 (COPYING)"));
        assert!(text.contains("Apache text"));
        assert!(!text.contains("not a licence"));
        assert!(text.contains("declare above applies: three 1.0.0."));
    }

    #[test]
    fn times_are_written_in_utc() {
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(951_782_400), "2000-02-29T00:00:00Z");
    }
}
