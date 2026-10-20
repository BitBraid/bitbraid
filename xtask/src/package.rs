//! `cargo xtask package <kind>...`: the packages of a release, from the
//! binary built for it.
//!
//! Linux: `tarball`, `appimage`, `deb`, `rpm`, and `aur`, the
//! `PKGBUILD` and `.SRCINFO` of `packaging/aur` for this version, which
//! install the tarball; `all` is the five. Each carries the desktop
//! entries, the icon at the usual sizes and as a drawing, the AppStream
//! metadata, BitBraid's licences and those of the crates in the binary
//! (`cargo xtask sbom`). None ships a git: every distribution has one, and
//! the packages require it.
//!
//! Windows: `zip`, the portable archive, and `wxs`, the source of
//! the installer that the release workflow builds with WiX. Both hold the
//! executable with MinGit beside it in `git\` (`--git <dir>`, MinGit
//! unpacked), where `bitbraid_git::shipped` looks for it, and in it
//! `SOURCE.txt`, which says which release of Git for Windows it is and
//! where its source is (its licences are MinGit's own).
//!
//! The binary is built first (`--no-build` takes the one there is) with
//! the lock file as it is, `SOURCE_DATE_EPOCH` (else the time of the last
//! commit), a seed for the constants const-random draws when it is
//! compiled, and the paths of the checkout, cargo's home and the target
//! directory remapped, so that it is the same wherever it is built in the
//! same environment (`packaging/linux/Dockerfile` for Linux). The
//! archives are written here (`archive`, `rpm`), each entry dated by
//! `SOURCE_DATE_EPOCH` and owned by root; the AppImage is made by
//! appimagetool with the runtime it is given (`APPIMAGETOOL`,
//! `APPIMAGE_RUNTIME`). With no kind, the binary is built and that is all.
//!
//! The file names are the release's, kept from one version to the next
//! ([`file_name`]).
//!
//! Sources: the Debian Policy Manual (5.3 the binary control file's
//! fields, 5.6.12 the version and `~`, 7.1 the relationship fields, 12.5
//! the copyright file), deb(5), deb-control(5), deb-md5sums(5); the
//! Filesystem Hierarchy Standard 3.0; the freedesktop Desktop Entry
//! Specification, the Icon Theme Specification (`hicolor`, sizes and
//! `scalable`), the AppStream specification ("Desktop Applications":
//! the metainfo file, `launchable`, `releases`, `content_rating`); the
//! Arch wiki's PKGBUILD(5), .SRCINFO and "AUR submission guidelines"
//! (`-bin` packages); AppImage's documentation (the AppDir, `AppRun`,
//! `.DirIcon`, appimagetool's `--runtime-file`); the WiX Toolset v5
//! documentation (`Package`, `MajorUpgrade`, `MediaTemplate`,
//! `StandardDirectory`, `Component`, `File`, `Shortcut`, `RegistryValue`,
//! `Feature`), Microsoft Learn's "Registering an Application to a URI
//! Scheme" and the ICO format (ICONDIR, ICONDIRENTRY, PNG images);
//! rustc's `--remap-path-prefix`, the MSVC linker's `/Brepro`,
//! reproducible-builds.org's `SOURCE_DATE_EPOCH`.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use md5::Digest as _;

use crate::{
    archive::{self, Node, Tree},
    bundle, rpm, sbom,
};

const USAGE: &str = "\
cargo xtask package [--profile <name>] [--target <triple>] [--no-build] [--out <dir>] [--git <dir>] <kind>...

  kinds: tarball appimage deb rpm aur (all: the five, Linux); zip wxs (Windows)";

/// What the packages say they are.
const NAME: &str = "bitbraid";
const SUMMARY: &str = "See and shape the history of a git repository";
const DESCRIPTION: &str = "\
BitBraid is a native git client written in Rust. It draws a repository's \
history as a graph of lanes on the GPU and works on it from there: staging \
down to single lines, commits, branches and tags, merges, rebases and their \
conflicts, stashes, worktrees and submodules, with GitHub and GitLab built \
in. It runs the git installed on the system.";
/// Who the packages name as their maintainer, at an address that reaches
/// nobody.
const MAINTAINER: &str = "BitBraid contributors <maintainers@bitbraid.invalid>";
const LICENSE: &str = "MIT";

/// The libraries the binary is linked against or opens, which the Linux
/// packages require: as Debian names their packages, and as rpm names
/// the libraries themselves. The window toolkit opens Vulkan and Wayland's
/// client library when it starts.
const DEB_DEPENDS: [&str; 8] = [
    "git",
    "libgcc-s1",
    "libxcb1",
    "libxkbcommon0",
    "libxkbcommon-x11-0",
    "zlib1g",
    "libvulkan1",
    "libwayland-client0",
];
const DEB_RECOMMENDS: &str = "mesa-vulkan-drivers | vulkan-icd";
const RPM_LIBRARIES: [&str; 9] = [
    "libc.so.6",
    "libgcc_s.so.1",
    "libm.so.6",
    "libxcb.so.1",
    "libxkbcommon.so.0",
    "libxkbcommon-x11.so.0",
    "libz.so.1",
    "libvulkan.so.1",
    "libwayland-client.so.0",
];

/// The sizes the icon is drawn at for the icon theme, in pixels; below 24
/// the brand's rules want the one-colour mark, and the drawing serves.
const ICON_SIZES: [u32; 7] = [24, 32, 48, 64, 128, 256, 512];

/// The installer's upgrade code: the same for every version, so that a
/// new one replaces the old.
const UPGRADE_CODE: &str = "6F1E4C2B-7A0D-4B8E-9C35-2D84B1F0A6E7";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Tarball,
    AppImage,
    Deb,
    Rpm,
    Aur,
    Zip,
    Wxs,
}

impl Kind {
    fn parse(word: &str) -> Result<Vec<Kind>> {
        Ok(match word {
            "tarball" => vec![Kind::Tarball],
            "appimage" => vec![Kind::AppImage],
            "deb" => vec![Kind::Deb],
            "rpm" => vec![Kind::Rpm],
            "aur" => vec![Kind::Aur],
            "all" => vec![
                Kind::Tarball,
                Kind::AppImage,
                Kind::Deb,
                Kind::Rpm,
                Kind::Aur,
            ],
            "zip" => vec![Kind::Zip],
            "wxs" => vec![Kind::Wxs],
            other => bail!("package: {other}: not a kind\n{USAGE}"),
        })
    }

    fn windows(self) -> bool {
        matches!(self, Kind::Zip | Kind::Wxs)
    }
}

#[derive(Debug, Default)]
struct Options {
    profile: String,
    target: Option<String>,
    build: bool,
    out: Option<PathBuf>,
    git: Option<PathBuf>,
    kinds: Vec<Kind>,
}

fn options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        profile: "dist".to_owned(),
        build: true,
        ..Options::default()
    };
    let mut args = args.iter();
    let value = |name: &str, args: &mut std::slice::Iter<'_, String>| {
        args.next()
            .cloned()
            .with_context(|| format!("{name} wants a value\n{USAGE}"))
    };
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => options.profile = value("--profile", &mut args)?,
            "--target" => options.target = Some(value("--target", &mut args)?),
            "--out" => options.out = Some(PathBuf::from(value("--out", &mut args)?)),
            "--git" => options.git = Some(PathBuf::from(value("--git", &mut args)?)),
            "--no-build" => options.build = false,
            other if other.starts_with('-') => bail!("package: {other}: not an option\n{USAGE}"),
            other => {
                for kind in Kind::parse(other)? {
                    if !options.kinds.contains(&kind) {
                        options.kinds.push(kind);
                    }
                }
            }
        }
    }
    // The archive the PKGBUILD installs is made first.
    options.kinds.sort_by_key(|kind| *kind as u8);
    Ok(options)
}

/// A processor, by the names each kind of package gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Arch {
    /// Ours, in file names, and the AppImage's and rpm's.
    name: &'static str,
    debian: &'static str,
}

fn arch_of(triple: &str) -> Result<Arch> {
    let processor = triple.split('-').next().unwrap_or_default();
    Ok(match processor {
        "x86_64" => Arch {
            name: "x86_64",
            debian: "amd64",
        },
        "aarch64" => Arch {
            name: "aarch64",
            debian: "arm64",
        },
        other => bail!("package: no packages for the processor {other}"),
    })
}

/// The triple of the host, as rustc names it.
fn host_triple() -> Result<String> {
    let out = Command::new("rustc")
        .arg("-vV")
        .output()
        .context("running rustc -vV")?;
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(str::to_owned)
        .context("rustc -vV names no host")
}

/// A version as Debian and rpm order it: a pre-release after `~`, which
/// both sort before the release.
fn distro_version(version: &str) -> String {
    version.replacen('-', "~", 1)
}

/// The version of the Windows installer: three numbers, no pre-release.
fn msi_version(version: &str) -> Result<String> {
    let release = version.split(['-', '+']).next().unwrap_or_default();
    let parts: Vec<u32> = release
        .split('.')
        .map(|part| part.parse::<u32>())
        .collect::<Result<_, _>>()
        .with_context(|| format!("{version}: not a version the installer can carry"))?;
    match parts.as_slice() {
        [major, minor, patch] if *major <= 255 && *minor <= 255 && *patch <= 65_535 => {
            Ok(format!("{major}.{minor}.{patch}"))
        }
        _ => bail!("{version}: not a version the installer can carry"),
    }
}

/// The release file names, one per kind.
/// They keep the version as it is: the `~` that Debian and rpm order a
/// pre-release by is in the packages themselves, not in names a release
/// page may not keep.
pub fn file_name(kind: &str, version: &str, arch: &str) -> Option<String> {
    let debian = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        other => other,
    };
    Some(match kind {
        "tarball" => format!("bitbraid-{version}-{arch}-linux.tar.gz"),
        "appimage" => format!("bitbraid-{version}-{arch}.AppImage"),
        "deb" => format!("bitbraid_{version}_{debian}.deb"),
        "rpm" => format!("bitbraid-{version}-1.{arch}.rpm"),
        "zip" => format!("bitbraid-{version}-{arch}-windows.zip"),
        "msi" => format!("bitbraid-{version}-{arch}.msi"),
        "dmg" => format!("bitbraid-{version}-universal.dmg"),
        // The bundle alone, for the updater.
        "app" => format!("bitbraid-{version}-universal.app.tar.gz"),
        _ => return None,
    })
}

/// Build the application for a release: `cargo build --locked` of the
/// `bitbraid` package with `profile`, for `targets` (the host's with
/// none), with `SOURCE_DATE_EPOCH` set and the paths that would end up in
/// the binary remapped.
pub fn release_build(
    root: &Path,
    profile: &str,
    targets: &[&str],
    env: &[(&str, &str)],
) -> Result<()> {
    let epoch = sbom::source_date_epoch(root)?;
    let target_dir = bundle::target_dir(root);
    let mut flags: Vec<String> = match std::env::var("CARGO_ENCODED_RUSTFLAGS") {
        Ok(encoded) if !encoded.is_empty() => encoded.split('\x1f').map(str::to_owned).collect(),
        _ => std::env::var("RUSTFLAGS")
            .unwrap_or_default()
            .split_whitespace()
            .map(str::to_owned)
            .collect(),
    };
    for flag in remap_flags(root, &cargo_home(), &target_dir) {
        if !flags.contains(&flag) {
            flags.push(flag);
        }
    }
    let msvc = targets.iter().any(|t| t.ends_with("-msvc"))
        || (targets.is_empty() && cfg!(all(windows, target_env = "msvc")));
    // The MSVC linker writes the time into the executable unless told not to.
    if msvc && !flags.iter().any(|flag| flag == "link-arg=/Brepro") {
        flags.push("-C".to_owned());
        flags.push("link-arg=/Brepro".to_owned());
    }
    let mut cargo =
        Command::new(std::env::var_os("CARGO").unwrap_or_else(|| OsString::from("cargo")));
    cargo
        .args([
            "build",
            "--locked",
            "--package",
            "bitbraid",
            "--profile",
            profile,
        ])
        .env("SOURCE_DATE_EPOCH", epoch.to_string())
        .env("CARGO_ENCODED_RUSTFLAGS", flags.join("\x1f"))
        .env_remove("RUSTFLAGS")
        .current_dir(root);
    // const-random, which gives ahash its keys, draws constants from the
    // system's randomness when it is compiled unless it is given a seed,
    // and two builds of one commit would differ by them. The seed is the
    // version's, as anyone building the release again has to know it.
    if std::env::var_os("CONST_RANDOM_SEED").is_none() {
        cargo.env(
            "CONST_RANDOM_SEED",
            const_random_seed(&bundle::workspace_version(root)?),
        );
    }
    for (key, value) in env {
        cargo.env(key, value);
    }
    for target in targets {
        cargo.args(["--target", target]);
    }
    println!(
        "\n==> cargo build --locked ({profile}) {}",
        targets.join(" ")
    );
    let status = cargo.status().context("starting cargo build")?;
    if !status.success() {
        bail!("cargo build failed with {status}");
    }
    Ok(())
}

/// The seed of const-random's constants in a release of `version`.
fn const_random_seed(version: &str) -> String {
    format!("bitbraid-{version}")
}

/// The prefixes rustc writes in the place of the checkout, cargo's home
/// and the target directory. The target directory comes last, as it may
/// lie in the checkout and the last that matches is the one taken.
fn remap_flags(root: &Path, cargo_home: &Path, target_dir: &Path) -> Vec<String> {
    vec![
        format!("--remap-path-prefix={}=/bitbraid", root.display()),
        format!("--remap-path-prefix={}=/cargo", cargo_home.display()),
        format!("--remap-path-prefix={}=/target", target_dir.display()),
    ]
}

/// Cargo's home: `CARGO_HOME`, else `.cargo` in the user's home.
fn cargo_home() -> PathBuf {
    if let Some(home) = std::env::var_os("CARGO_HOME") {
        return PathBuf::from(home);
    }
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_default();
    home.join(".cargo")
}

/// What every package is made from.
struct Release {
    root: PathBuf,
    version: String,
    epoch: u64,
    arch: Arch,
    triple: String,
    binary: Vec<u8>,
    out: PathBuf,
}

impl Release {
    fn read(&self, path: &str) -> Result<Vec<u8>> {
        std::fs::read(self.root.join(path)).with_context(|| format!("reading {path}"))
    }

    fn read_text(&self, path: &str) -> Result<String> {
        std::fs::read_to_string(self.root.join(path)).with_context(|| format!("reading {path}"))
    }

    fn write(&self, name: &str, bytes: &[u8]) -> Result<PathBuf> {
        let path = self.out.join(name);
        std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
        println!(
            "package: {} ({:.1} MB)",
            path.display(),
            bytes.len() as f64 / 1e6
        );
        Ok(path)
    }

    fn file_name(&self, kind: &str) -> Result<String> {
        file_name(kind, &self.version, self.arch.name)
            .with_context(|| format!("{kind}: no file name"))
    }

    /// The day of the release, for the AppStream metadata.
    fn date(&self) -> String {
        sbom::rfc3339(self.epoch).chars().take(10).collect()
    }

    fn third_party(&self) -> Result<Vec<u8>> {
        let (crates, _) = sbom::shipped_crates(&self.root, &self.triple)?;
        Ok(sbom::third_party_text(&self.version, &self.triple, &crates)?.into_bytes())
    }
}

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args)?;
    let root = crate::root();
    let windows = options.kinds.iter().any(|kind| kind.windows());
    let linux = options.kinds.iter().any(|kind| !kind.windows());
    if windows && linux {
        bail!(
            "package: Linux and Windows packages are made from binaries of their own; ask for one system's at a time"
        );
    }
    let triple = match &options.target {
        Some(target) => target.clone(),
        None => host_triple()?,
    };
    if linux && !triple.contains("-linux") {
        bail!("package: the Linux packages want a Linux binary; {triple} is not one");
    }
    if windows && !triple.contains("-windows") {
        bail!("package: the Windows packages want a Windows binary; {triple} is not one");
    }
    if options.build {
        let targets: Vec<&str> = options.target.as_deref().into_iter().collect();
        release_build(&root, &options.profile, &targets, &[])?;
    }
    if options.kinds.is_empty() {
        return Ok(());
    }

    let target_dir = bundle::target_dir(&root);
    let mut built = target_dir.clone();
    if let Some(target) = &options.target {
        built.push(target);
    }
    built.push(bundle::profile_dir(&options.profile));
    built.push(if windows { "bitbraid.exe" } else { "bitbraid" });
    let binary = std::fs::read(&built)
        .with_context(|| format!("reading {} (built with --no-build?)", built.display()))?;
    let out = options
        .out
        .clone()
        .unwrap_or_else(|| target_dir.join("packages"));
    std::fs::create_dir_all(&out)?;
    let release = Release {
        version: bundle::workspace_version(&root)?,
        epoch: sbom::source_date_epoch(&root)?,
        arch: arch_of(&triple)?,
        triple,
        binary,
        out,
        root,
    };

    let mut tarball_sha = None;
    for kind in &options.kinds {
        match kind {
            Kind::Tarball => {
                let bytes = tarball(&release)?;
                tarball_sha = Some(sha256_hex(&bytes));
                release.write(&release.file_name("tarball")?, &bytes)?;
            }
            Kind::AppImage => appimage(&release)?,
            Kind::Deb => {
                let bytes = deb(&release)?;
                release.write(&release.file_name("deb")?, &bytes)?;
            }
            Kind::Rpm => {
                let bytes = rpm_package(&release)?;
                release.write(&release.file_name("rpm")?, &bytes)?;
            }
            Kind::Aur => {
                let sha = match &tarball_sha {
                    Some(sha) => sha.clone(),
                    None => {
                        let path = release.out.join(release.file_name("tarball")?);
                        sha256_hex(&std::fs::read(&path).with_context(|| {
                            format!(
                                "reading {} (the PKGBUILD installs it: make it first)",
                                path.display()
                            )
                        })?)
                    }
                };
                aur(&release, &sha)?;
            }
            Kind::Zip => {
                let tree = windows_tree(&release, options.git.as_deref())?;
                let bytes = archive::zip(&tree.under("BitBraid")?, release.epoch)?;
                release.write(&release.file_name("zip")?, &bytes)?;
            }
            Kind::Wxs => wxs(&release, options.git.as_deref())?,
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::Digest as _;
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ---------------------------------------------------------------------
// Linux

/// Where each kind keeps the licences: Debian's place is the package's
/// documentation; the others keep them where rpm, Arch and the XDG
/// layout do.
fn licence_dir(kind: Kind) -> &'static str {
    match kind {
        Kind::Deb => "share/doc/bitbraid",
        _ => "share/licenses/bitbraid",
    }
}

/// The highest version of glibc the binary asks for (`GLIBC_2.36`), read
/// from the names of its symbol versions.
fn glibc_needed(binary: &[u8]) -> Option<(u32, u32)> {
    let needle = b"GLIBC_2.";
    let mut best = None;
    let mut at = 0;
    while let Some(found) = binary
        .get(at..)
        .and_then(|rest| rest.windows(needle.len()).position(|w| w == needle))
    {
        let start = at + found + needle.len();
        let digits: Vec<u8> = binary
            .get(start..)
            .unwrap_or_default()
            .iter()
            .take_while(|b| b.is_ascii_digit())
            .copied()
            .collect();
        if let Ok(minor) = String::from_utf8_lossy(&digits).parse::<u32>() {
            best = best.max(Some((2, minor)));
        }
        at = start;
    }
    best
}

/// The files the Linux packages share, relative to their prefix (`/usr`,
/// or the tarball's top directory).
fn linux_tree(release: &Release, kind: Kind) -> Result<Tree> {
    let mut tree = Tree::new();
    tree.file("bin/bitbraid", release.binary.clone(), 0o755)?;
    tree.file(
        "share/applications/bitbraid.desktop",
        release.read("packaging/linux/bitbraid.desktop")?,
        0o644,
    )?;
    if kind != Kind::AppImage {
        // Links of our scheme go to the application; in an
        // AppImage the program has no fixed place to name.
        tree.file(
            "share/applications/bitbraid-url.desktop",
            release.read("packaging/linux/bitbraid-url.desktop")?,
            0o644,
        )?;
    }
    let svg = release.read("brand/icon.svg")?;
    for side in ICON_SIZES {
        tree.file(
            &format!("share/icons/hicolor/{side}x{side}/apps/bitbraid.png"),
            bundle::draw_icon(&svg, side, 1.0)?,
            0o644,
        )?;
    }
    tree.file("share/icons/hicolor/scalable/apps/bitbraid.svg", svg, 0o644)?;
    let metainfo = release
        .read_text("packaging/linux/bitbraid.metainfo.xml")?
        .replace("@VERSION@", &release.version)
        .replace("@DATE@", &release.date());
    tree.file(
        "share/metainfo/bitbraid.metainfo.xml",
        metainfo.into_bytes(),
        0o644,
    )?;

    let licences = licence_dir(kind);
    let license = release.read_text("LICENSE")?;
    let third_party = release.third_party()?;
    if kind == Kind::Deb {
        // Debian's place for every licence is the one copyright file,
        // which is also all that a system installed without
        // documentation (Ubuntu's images) keeps of the directory.
        let copyright = format!(
            "BitBraid\nSource: {}\n\n{license}\n\n\
             ----------------------------------------------------------------------\n\n{}",
            sbom::REPOSITORY,
            String::from_utf8_lossy(&third_party)
        );
        tree.file(
            &format!("{licences}/copyright"),
            copyright.into_bytes(),
            0o644,
        )?;
    } else {
        tree.file(&format!("{licences}/LICENSE"), license.into_bytes(), 0o644)?;
        tree.file(
            &format!("{licences}/THIRD-PARTY-LICENSES.txt"),
            third_party,
            0o644,
        )?;
    }
    Ok(tree)
}

/// The tarball: one directory, `bin` and `share` in it, to be unpacked
/// into a prefix (`tar -xzf <file> -C ~/.local --strip-components=1`) or
/// run from where it is.
fn tarball(release: &Release) -> Result<Vec<u8>> {
    let mut tree = linux_tree(release, Kind::Tarball)?;
    tree.file(
        "share/doc/bitbraid/README",
        tarball_readme(&release.version).into_bytes(),
        0o644,
    )?;
    let top = format!("bitbraid-{}-{}-linux", release.version, release.arch.name);
    let tree = tree.with_parents().under(&top)?;
    archive::gzip(&archive::tar(&tree, release.epoch, false)?)
}

fn tarball_readme(version: &str) -> String {
    format!(
        "BitBraid {version}\n\n\
         Run it from here: bin/bitbraid. It needs git on PATH, a Vulkan driver,\n\
         and Wayland or X11.\n\n\
         To install it for yourself, unpack the archive into ~/.local:\n\n\
         \x20   tar -xzf bitbraid-{version}-*-linux.tar.gz -C ~/.local --strip-components=1\n\n\
         which puts the program in ~/.local/bin, and its desktop entries, icons\n\
         and metadata in ~/.local/share, where the desktop finds them. For\n\
         every user, unpack it into /usr/local the same way, as root.\n\n\
         {}\n",
        sbom::REPOSITORY
    )
}

/// The Debian package.
fn deb(release: &Release) -> Result<Vec<u8>> {
    let tree = linux_tree(release, Kind::Deb)?
        .with_parents()
        .under("usr")?;
    let mut md5sums = String::new();
    let mut kib: u64 = 0;
    for (path, node) in tree.iter() {
        match node {
            Node::File { data, .. } => {
                let digest: String = md5::Md5::digest(data)
                    .iter()
                    .map(|byte| format!("{byte:02x}"))
                    .collect();
                md5sums.push_str(&format!("{digest}  {path}\n"));
                kib += (data.len() as u64).div_ceil(1024);
            }
            _ => kib += 1,
        }
    }
    let mut depends: Vec<String> = DEB_DEPENDS.iter().map(|d| (*d).to_owned()).collect();
    if let Some((major, minor)) = glibc_needed(&release.binary) {
        depends.insert(1, format!("libc6 (>= {major}.{minor})"));
    } else {
        depends.insert(1, "libc6".to_owned());
    }
    let control = format!(
        "Package: {NAME}\n\
         Version: {}\n\
         Architecture: {}\n\
         Maintainer: {MAINTAINER}\n\
         Installed-Size: {kib}\n\
         Depends: {}\n\
         Recommends: {DEB_RECOMMENDS}\n\
         Section: vcs\n\
         Priority: optional\n\
         Homepage: {}\n\
         Description: {SUMMARY}\n{}",
        distro_version(&release.version),
        release.arch.debian,
        depends.join(", "),
        sbom::REPOSITORY,
        debian_long_description(DESCRIPTION),
    );
    let mut meta = Tree::new();
    meta.file("control", control.into_bytes(), 0o644)?;
    meta.file("md5sums", md5sums.into_bytes(), 0o644)?;
    let control_tar = archive::gzip(&archive::tar(&meta, release.epoch, true)?)?;
    let data_tar = archive::gzip(&archive::tar(&tree, release.epoch, true)?)?;
    archive::ar(
        &[
            ("debian-binary", b"2.0\n"),
            ("control.tar.gz", &control_tar),
            ("data.tar.gz", &data_tar),
        ],
        release.epoch,
    )
}

/// A long description as a control file wants it: lines of at most 79
/// characters, each after a space.
fn debian_long_description(text: &str) -> String {
    let mut out = String::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.len() + 1 + word.len() > 78 {
            out.push_str(&format!(" {line}\n"));
            line.clear();
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        out.push_str(&format!(" {line}\n"));
    }
    out
}

/// The rpm package.
fn rpm_package(release: &Release) -> Result<Vec<u8>> {
    let tree = linux_tree(release, Kind::Rpm)?
        .with_parents()
        .under("usr")?;
    let mut requires: Vec<String> = vec!["/usr/bin/git".to_owned()];
    let bits = if release.arch.name == "x86_64" || release.arch.name == "aarch64" {
        "(64bit)"
    } else {
        ""
    };
    for library in RPM_LIBRARIES {
        requires.push(format!("{library}(){bits}"));
    }
    if let Some((major, minor)) = glibc_needed(&release.binary) {
        requires.push(format!("libc.so.6(GLIBC_{major}.{minor}){bits}"));
    }
    let requires: Vec<&str> = requires.iter().map(String::as_str).collect();
    let licences = [
        "usr/share/licenses/bitbraid/LICENSE",
        "usr/share/licenses/bitbraid/THIRD-PARTY-LICENSES.txt",
    ];
    let version = distro_version(&release.version);
    rpm::rpm(
        &rpm::Package {
            name: NAME,
            version: &version,
            release: "1",
            arch: release.arch.name,
            summary: SUMMARY,
            description: DESCRIPTION,
            license: LICENSE,
            url: sbom::REPOSITORY,
            build_time: u32::try_from(release.epoch).context("SOURCE_DATE_EPOCH past 2106")?,
            requires: &requires,
            owned_dirs: &["usr/share/licenses/bitbraid"],
            licences: &licences,
        },
        &tree,
    )
}

/// The AppImage: an AppDir of the package's files, made into one file by
/// appimagetool with the runtime given.
fn appimage(release: &Release) -> Result<()> {
    let mut tree = linux_tree(release, Kind::AppImage)?.under("usr")?;
    tree.file(
        "bitbraid.desktop",
        release.read("packaging/linux/bitbraid.desktop")?,
        0o644,
    )?;
    tree.file(
        "bitbraid.png",
        bundle::draw_icon(&release.read("brand/icon.svg")?, 256, 1.0)?,
        0o644,
    )?;
    tree.link(".DirIcon", "bitbraid.png")?;
    tree.link("AppRun", "usr/bin/bitbraid")?;
    let tree = tree.with_parents();

    let appdir = release.out.join("BitBraid.AppDir");
    if appdir.exists() {
        std::fs::remove_dir_all(&appdir)
            .with_context(|| format!("removing {}", appdir.display()))?;
    }
    write_tree(&tree, &appdir, release.epoch)?;

    let tool = std::env::var_os("APPIMAGETOOL").unwrap_or_else(|| OsString::from("appimagetool"));
    let out = release.out.join(release.file_name("appimage")?);
    let mut command = Command::new(&tool);
    match std::env::var_os("APPIMAGE_RUNTIME") {
        Some(runtime) => {
            command.arg("--runtime-file").arg(runtime);
        }
        None => println!(
            "package: APPIMAGE_RUNTIME names no runtime: appimagetool fetches the latest, and the AppImage is not reproducible"
        ),
    }
    command
        .arg("--no-appstream")
        .arg(&appdir)
        .arg(&out)
        .env("ARCH", release.arch.name)
        .env("SOURCE_DATE_EPOCH", release.epoch.to_string())
        .env("VERSION", &release.version);
    let status = command
        .status()
        .with_context(|| format!("starting {} (set APPIMAGETOOL)", tool.to_string_lossy()))?;
    if !status.success() {
        bail!("appimagetool failed with {status}");
    }
    let _ = std::fs::remove_dir_all(&appdir);
    let size = std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0);
    println!("package: {} ({:.1} MB)", out.display(), size as f64 / 1e6);
    Ok(())
}

/// The tree on disk under `dir`, each file and directory dated `mtime`.
#[cfg(unix)]
fn write_tree(tree: &Tree, dir: &Path, mtime: u64) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime);
    std::fs::create_dir_all(dir)?;
    let mut dirs = vec![dir.to_path_buf()];
    for (path, node) in tree.iter() {
        let at = dir.join(path);
        match node {
            Node::Dir { mode } => {
                std::fs::create_dir_all(&at)?;
                std::fs::set_permissions(&at, std::fs::Permissions::from_mode(*mode))?;
                dirs.push(at);
            }
            Node::File { data, mode } => {
                std::fs::write(&at, data).with_context(|| format!("writing {}", at.display()))?;
                std::fs::set_permissions(&at, std::fs::Permissions::from_mode(*mode))?;
                std::fs::File::options()
                    .write(true)
                    .open(&at)?
                    .set_modified(time)?;
            }
            Node::Link { target } => std::os::unix::fs::symlink(target, &at)?,
        }
    }
    // Directories last: writing into one changes its time.
    for dir in dirs.iter().rev() {
        std::fs::File::open(dir)?.set_modified(time)?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_tree(_tree: &Tree, _dir: &Path, _mtime: u64) -> Result<()> {
    bail!("an AppImage is made on Linux")
}

/// The PKGBUILD of `packaging/aur` with this version and the tarball's
/// checksum, and its `.SRCINFO`.
fn aur(release: &Release, tarball_sha: &str) -> Result<()> {
    let template = release.read_text("packaging/aur/PKGBUILD")?;
    let version = release.version.replace('-', "_");
    let pkgbuild = fill_pkgbuild(&template, &version, &release.version, tarball_sha)?;
    let srcinfo = srcinfo(&pkgbuild)?;
    let dir = release.out.join("aur");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("PKGBUILD"), &pkgbuild)?;
    std::fs::write(dir.join(".SRCINFO"), &srcinfo)?;
    println!("package: {} (PKGBUILD, .SRCINFO)", dir.display());
    // The two as one file of the release: a name that starts with a dot
    // is not kept by a release's files.
    let mut tree = Tree::new();
    tree.file("PKGBUILD", pkgbuild.into_bytes(), 0o644)?;
    tree.file(".SRCINFO", srcinfo.into_bytes(), 0o644)?;
    let bytes = archive::gzip(&archive::tar(
        &tree.under("bitbraid-bin")?,
        release.epoch,
        false,
    )?)?;
    release.write(
        &format!("bitbraid-bin-{}-aur.tar.gz", release.version),
        &bytes,
    )?;
    Ok(())
}

/// The template with `pkgver`, `_version` (the release's own, which a
/// pre-release's `-` keeps) and `sha256sums` set.
fn fill_pkgbuild(template: &str, pkgver: &str, version: &str, sha: &str) -> Result<String> {
    let mut seen = (false, false, false);
    let mut out = String::new();
    for line in template.lines() {
        let line = if line.starts_with("pkgver=") {
            seen.0 = true;
            format!("pkgver={pkgver}")
        } else if line.starts_with("_version=") {
            seen.1 = true;
            format!("_version={version}")
        } else if line.starts_with("sha256sums=") {
            seen.2 = true;
            format!("sha256sums=('{sha}')")
        } else {
            line.to_owned()
        };
        out.push_str(&line);
        out.push('\n');
    }
    if seen != (true, true, true) {
        bail!("packaging/aur/PKGBUILD wants lines of pkgver=, _version= and sha256sums=");
    }
    Ok(out)
}

/// The `.SRCINFO` of a PKGBUILD whose fields are each on one line, as
/// `makepkg --printsrcinfo` writes it.
fn srcinfo(pkgbuild: &str) -> Result<String> {
    let mut fields: Vec<(String, Vec<String>)> = Vec::new();
    for line in pkgbuild.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.is_empty() || !key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            continue;
        }
        let values = if let Some(inner) = value.strip_prefix('(') {
            let inner = inner
                .strip_suffix(')')
                .with_context(|| format!("PKGBUILD: {key}: an array on one line"))?;
            shell_words(inner)?
        } else {
            shell_words(value)?
        };
        fields.push((key.to_owned(), values));
    }
    let get = |key: &str| {
        fields
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let expand = |text: &str| {
        let mut text = text.to_owned();
        for key in ["pkgname", "pkgver", "_version", "pkgrel"] {
            let value = get(key).first().cloned().unwrap_or_default();
            text = text
                .replace(&format!("${{{key}}}"), &value)
                .replace(&format!("${key}"), &value);
        }
        text
    };
    let name = get("pkgname")
        .first()
        .cloned()
        .context("PKGBUILD has no pkgname")?;
    let mut out = format!("pkgbase = {name}\n");
    for key in [
        "pkgdesc",
        "pkgver",
        "pkgrel",
        "url",
        "arch",
        "license",
        "depends",
        "optdepends",
        "provides",
        "conflicts",
        "options",
        "source",
        "sha256sums",
    ] {
        for value in get(key) {
            out.push_str(&format!("\t{key} = {}\n", expand(&value)));
        }
    }
    out.push_str(&format!("\npkgname = {name}\n"));
    Ok(out)
}

/// Words as the shell splits them, for the plain quoting a PKGBUILD's
/// fields use: single quotes, double quotes, and none.
fn shell_words(text: &str) -> Result<Vec<String>> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '\'' {
                        break;
                    }
                    word.push(c);
                }
            }
            '"' => {
                in_word = true;
                for c in chars.by_ref() {
                    if c == '"' {
                        break;
                    }
                    word.push(c);
                }
            }
            '#' if !in_word => break,
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    Ok(words)
}

// ---------------------------------------------------------------------
// Windows

/// The files of the Windows packages: the executable, MinGit beside it in
/// `git`, the licences.
fn windows_tree(release: &Release, git: Option<&Path>) -> Result<Tree> {
    let mut tree = Tree::new();
    tree.file("bitbraid.exe", release.binary.clone(), 0o755)?;
    match git {
        Some(dir) => {
            if !dir.join("cmd").join("git.exe").is_file() {
                bail!("{}: not MinGit unpacked (no cmd\\git.exe)", dir.display());
            }
            add_dir(&mut tree, dir, "git")?;
            let versions = std::fs::read_to_string(dir.join("etc").join("package-versions.txt"))
                .with_context(|| format!("{}: no etc\\package-versions.txt", dir.display()))?;
            tree.file(
                "git/SOURCE.txt",
                mingit_source_note(&versions)?
                    .replace('\n', "\r\n")
                    .into_bytes(),
                0o644,
            )?;
        }
        None => println!("package: no --git: the Windows package carries no git"),
    }
    tree.file(
        "LICENSE.txt",
        release
            .read_text("LICENSE")?
            .replace('\n', "\r\n")
            .into_bytes(),
        0o644,
    )?;
    tree.file("THIRD-PARTY-LICENSES.txt", release.third_party()?, 0o644)?;
    Ok(tree.with_parents())
}

/// What MinGit's list of its packages (`etc/package-versions.txt`) says
/// of its git: the version, Git for Windows' tag of that release, and the
/// folder its packages are installed in.
fn mingit_git(versions: &str) -> Option<(String, String, &'static str)> {
    versions.lines().find_map(|line| {
        let (name, version) = line.trim().split_once(' ')?;
        let folder = match name {
            "mingw-w64-ucrt-x86_64-git" => "ucrt64",
            "mingw-w64-x86_64-git" => "mingw64",
            "mingw-w64-clang-aarch64-git" => "clangarm64",
            _ => return None,
        };
        // `2.56.0.2-1`: the package's own release after the dash.
        let version = version.trim();
        let version = version.split_once('-').map_or(version, |(git, _)| git);
        let parts: Vec<&str> = version.split('.').collect();
        let numbers = parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
        if !numbers || !(3..=4).contains(&parts.len()) {
            return None;
        }
        let git = parts.get(..3)?.join(".");
        let build = parts.get(3).copied().unwrap_or("1");
        Some((
            version.to_owned(),
            format!("v{git}.windows.{build}"),
            folder,
        ))
    })
}

/// `git\SOURCE.txt` of the Windows packages: which MinGit it is, and
/// where its source is.
fn mingit_source_note(versions: &str) -> Result<String> {
    let (version, tag, folder) =
        mingit_git(versions).context("MinGit's etc\\package-versions.txt names no git")?;
    Ok(format!(
        "MinGit {version}, as BitBraid carries it\n\
         \n\
         Git for Windows' build of git for programs to carry, unmodified:\n\
         https://github.com/git-for-windows/git/releases/tag/{tag}\n\
         \n\
         The source of its git is Git for Windows' at that tag:\n\
         https://github.com/git-for-windows/git/tree/{tag}\n\
         The other packages in it are listed with their versions in\n\
         etc\\package-versions.txt; their sources are in the package\n\
         repositories of Git for Windows and of MSYS2\n\
         (https://github.com/git-for-windows, https://github.com/msys2).\n\
         \n\
         git is under the GNU General Public License, version 2\n\
         (LICENSE.txt); the licences of the other packages are in\n\
         {folder}\\share\\licenses.\n"
    ))
}

/// Every file under `dir` into the tree under `prefix`.
fn add_dir(tree: &mut Tree, dir: &Path, prefix: &str) -> Result<()> {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .with_context(|| format!("reading {}", dir.display()))?
        .collect::<Result<_, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let name = entry.file_name();
        let name = name
            .to_str()
            .with_context(|| format!("{}: a name that is not UTF-8", entry.path().display()))?;
        let path = format!("{prefix}/{name}");
        let kind = entry.file_type()?;
        if kind.is_dir() {
            tree.dir(&path, 0o755)?;
            add_dir(tree, &entry.path(), &path)?;
        } else if kind.is_file() {
            tree.file(&path, std::fs::read(entry.path())?, 0o644)?;
        } else {
            bail!("{}: neither a file nor a directory", entry.path().display());
        }
    }
    Ok(())
}

/// The installer's source: the files of [`windows_tree`] staged on disk,
/// and `bitbraid.wxs` naming each, for `wix build`.
fn wxs(release: &Release, git: Option<&Path>) -> Result<()> {
    let tree = windows_tree(release, git)?;
    let stage = release.out.join("msi");
    if stage.exists() {
        std::fs::remove_dir_all(&stage)?;
    }
    let files = stage.join("BitBraid");
    stage_tree(&tree, &files)?;
    let svg = release.read("brand/icon.svg")?;
    let icon = stage.join("bitbraid.ico");
    std::fs::write(&icon, ico(&svg)?)?;
    let source = wxs_source(&tree, &files, &icon, &msi_version(&release.version)?)?;
    let path = stage.join("bitbraid.wxs");
    std::fs::write(&path, source)?;
    println!(
        "package: {} (wix build -arch x64 -o {} {})",
        path.display(),
        release.file_name("msi")?,
        path.display()
    );
    Ok(())
}

/// The files of `tree` written under `files`, which is made first: the
/// tree's own folders are those below it, and a file at its top (the
/// licences) comes before any of them.
fn stage_tree(tree: &Tree, files: &Path) -> Result<()> {
    std::fs::create_dir_all(files).with_context(|| format!("making {}", files.display()))?;
    for (path, node) in tree.iter() {
        let at = files.join(path);
        match node {
            Node::Dir { .. } => {
                std::fs::create_dir_all(&at).with_context(|| format!("making {}", at.display()))?
            }
            Node::File { data, .. } => {
                std::fs::write(&at, data).with_context(|| format!("writing {}", at.display()))?
            }
            Node::Link { .. } => bail!("{path}: the installer holds no links"),
        }
    }
    Ok(())
}

fn xml(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// The WiX source of the installer: a per-machine installation into
/// `Program Files\BitBraid`, a shortcut in the Start menu, the scheme of
/// our links, and a newer version replacing an older one. The
/// shortcut's component is kept by a value under `HKCU`, as WiX's own
/// "How To: Create a Shortcut on the Start Menu" has it, which the
/// installer's checks (ICE38, ICE43) ask of a shortcut.
fn wxs_source(tree: &Tree, files: &Path, icon: &Path, version: &str) -> Result<String> {
    let mut out = String::new();
    out.push_str(&format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n\
         <!-- Written by `cargo xtask package wxs`; built by `wix build -arch x64`. -->\n\
         <Wix xmlns=\"http://wixtoolset.org/schemas/v4/wxs\">\n\
         \x20 <Package Name=\"BitBraid\" Manufacturer=\"BitBraid contributors\" Version=\"{version}\"\n\
         \x20          UpgradeCode=\"{UPGRADE_CODE}\" Scope=\"perMachine\" Language=\"1033\">\n\
         \x20   <MajorUpgrade DowngradeErrorMessage=\"A newer version of BitBraid is already installed.\" />\n\
         \x20   <MediaTemplate EmbedCab=\"yes\" CompressionLevel=\"high\" />\n\
         \x20   <Icon Id=\"bitbraid.ico\" SourceFile=\"{}\" />\n\
         \x20   <Property Id=\"ARPPRODUCTICON\" Value=\"bitbraid.ico\" />\n\
         \x20   <Property Id=\"ARPURLINFOABOUT\" Value=\"{}\" />\n\
         \x20   <StandardDirectory Id=\"ProgramFiles64Folder\">\n\
         \x20     <Directory Id=\"INSTALLFOLDER\" Name=\"BitBraid\">\n",
        xml(&icon.display().to_string()),
        sbom::REPOSITORY,
    ));
    // The directories nest as the paths do; each file is a component of
    // its own, its GUID made by WiX from its place.
    let mut top = WxsDir::default();
    for (path, node) in tree.iter() {
        let mut parts: Vec<&str> = path.split('/').collect();
        let name = parts.pop().unwrap_or_default();
        let mut dir = &mut top;
        for part in parts {
            dir = dir.dirs.entry(part.to_owned()).or_default();
        }
        match node {
            Node::Dir { .. } => {
                dir.dirs.entry(name.to_owned()).or_default();
            }
            Node::File { .. } => dir.files.push(path.to_owned()),
            Node::Link { .. } => bail!("{path}: the installer holds no links"),
        }
    }
    let mut ids = WxsIds::default();
    top.write(&mut out, files, 4, &mut ids);
    let exe = ids.exe.context("the installer has no bitbraid.exe")?;
    let components = ids.components;
    out.push_str(&format!(
        "\x20     </Directory>\n\
         \x20   </StandardDirectory>\n\
         \x20   <StandardDirectory Id=\"ProgramMenuFolder\">\n\
         \x20     <Component Id=\"shortcut\">\n\
         \x20       <Shortcut Id=\"startmenu\" Name=\"BitBraid\" Target=\"[#f{exe}]\" WorkingDirectory=\"INSTALLFOLDER\" />\n\
         \x20       <RegistryValue Root=\"HKCU\" Key=\"Software\\BitBraid\" Name=\"shortcut\" Type=\"integer\" Value=\"1\" KeyPath=\"yes\" />\n\
         \x20     </Component>\n\
         \x20   </StandardDirectory>\n\
         \x20   <Component Id=\"links\" Directory=\"INSTALLFOLDER\">\n\
         \x20     <RegistryValue Root=\"HKLM\" Key=\"Software\\Classes\\bitbraid\" Type=\"string\" Value=\"URL:BitBraid link\" KeyPath=\"yes\" />\n\
         \x20     <RegistryValue Root=\"HKLM\" Key=\"Software\\Classes\\bitbraid\" Name=\"URL Protocol\" Type=\"string\" Value=\"\" />\n\
         \x20     <RegistryValue Root=\"HKLM\" Key=\"Software\\Classes\\bitbraid\\DefaultIcon\" Type=\"string\" Value=\"&quot;[#f{exe}]&quot;,0\" />\n\
         \x20     <RegistryValue Root=\"HKLM\" Key=\"Software\\Classes\\bitbraid\\shell\\open\\command\" Type=\"string\" Value=\"&quot;[#f{exe}]&quot; &quot;%1&quot;\" />\n\
         \x20   </Component>\n\
         \x20   <Feature Id=\"Main\" Title=\"BitBraid\" Level=\"1\">\n"
    ));
    for id in &components {
        out.push_str(&format!("      <ComponentRef Id=\"{id}\" />\n"));
    }
    out.push_str(
        "      <ComponentRef Id=\"shortcut\" />\n\
         \x20     <ComponentRef Id=\"links\" />\n\
         \x20   </Feature>\n\
         \x20 </Package>\n\
         </Wix>\n",
    );
    Ok(out)
}

/// A directory of the installer: its files (by their paths in the tree)
/// and the directories in it.
#[derive(Debug, Default)]
struct WxsDir {
    files: Vec<String>,
    dirs: std::collections::BTreeMap<String, WxsDir>,
}

/// The ids handed out while the directories are written.
#[derive(Debug, Default)]
struct WxsIds {
    components: Vec<String>,
    directories: usize,
    /// The number of `bitbraid.exe`'s file.
    exe: Option<usize>,
}

impl WxsDir {
    /// Its files, then its directories, each in the order of its name.
    fn write(&self, out: &mut String, files: &Path, depth: usize, ids: &mut WxsIds) {
        let indent = "  ".repeat(depth);
        for path in &self.files {
            let id = ids.components.len() + 1;
            let source = files.join(path.replace('/', std::path::MAIN_SEPARATOR_STR));
            out.push_str(&format!(
                "{indent}<Component Id=\"c{id}\"><File Id=\"f{id}\" Source=\"{}\" KeyPath=\"yes\" /></Component>\n",
                xml(&source.display().to_string())
            ));
            ids.components.push(format!("c{id}"));
            if path == "bitbraid.exe" {
                ids.exe = Some(id);
            }
        }
        for (name, dir) in &self.dirs {
            ids.directories += 1;
            out.push_str(&format!(
                "{indent}<Directory Id=\"d{}\" Name=\"{}\">\n",
                ids.directories,
                xml(name)
            ));
            dir.write(out, files, depth + 1, ids);
            out.push_str(&format!("{indent}</Directory>\n"));
        }
    }
}

/// The icon as an `.ico` of PNG pictures, at the sizes Windows shows.
fn ico(svg: &[u8]) -> Result<Vec<u8>> {
    let sizes = [16u32, 24, 32, 48, 64, 128, 256];
    let pictures = sizes
        .iter()
        .map(|side| bundle::draw_icon(svg, *side, 1.0))
        .collect::<Result<Vec<_>>>()?;
    let count = u16::try_from(sizes.len())?;
    let mut out = Vec::new();
    out.extend_from_slice(&0u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&count.to_le_bytes());
    let mut offset = 6 + 16 * sizes.len();
    for (side, png) in sizes.iter().zip(&pictures) {
        // 256 is written as 0.
        let byte = u8::try_from(*side).unwrap_or(0);
        out.extend_from_slice(&[byte, byte, 0, 0]);
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&32u16.to_le_bytes());
        out.extend_from_slice(&u32::try_from(png.len())?.to_le_bytes());
        out.extend_from_slice(&u32::try_from(offset)?.to_le_bytes());
        offset += png.len();
    }
    for png in &pictures {
        out.extend_from_slice(png);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(args: &[&str]) -> Result<Options> {
        options(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn options_and_kinds_are_read() {
        let all = read(&["all"]).unwrap();
        assert_eq!(
            all.kinds,
            [
                Kind::Tarball,
                Kind::AppImage,
                Kind::Deb,
                Kind::Rpm,
                Kind::Aur
            ]
        );
        assert!(all.build);
        assert_eq!(all.profile, "dist");
        // The tarball is made before the PKGBUILD that installs it.
        let some = read(&["aur", "--no-build", "tarball", "deb", "deb"]).unwrap();
        assert_eq!(some.kinds, [Kind::Tarball, Kind::Deb, Kind::Aur]);
        assert!(!some.build);
        let windows = read(&[
            "--git",
            "C:/mingit",
            "--target",
            "x86_64-pc-windows-msvc",
            "zip",
            "wxs",
        ])
        .unwrap();
        assert_eq!(windows.kinds, [Kind::Zip, Kind::Wxs]);
        assert_eq!(windows.git, Some(PathBuf::from("C:/mingit")));
        assert!(read(&["flatpak"]).is_err());
        assert!(read(&["--sign"]).is_err());
        assert!(read(&["--out"]).is_err());
        assert!(read(&[]).unwrap().kinds.is_empty());
    }

    #[test]
    fn file_names_are_the_releases() {
        let name = |kind| file_name(kind, "1.0.0", "x86_64").unwrap();
        assert_eq!(name("tarball"), "bitbraid-1.0.0-x86_64-linux.tar.gz");
        assert_eq!(name("appimage"), "bitbraid-1.0.0-x86_64.AppImage");
        assert_eq!(name("deb"), "bitbraid_1.0.0_amd64.deb");
        assert_eq!(name("rpm"), "bitbraid-1.0.0-1.x86_64.rpm");
        assert_eq!(name("zip"), "bitbraid-1.0.0-x86_64-windows.zip");
        assert_eq!(name("msi"), "bitbraid-1.0.0-x86_64.msi");
        assert_eq!(name("dmg"), "bitbraid-1.0.0-universal.dmg");
        assert_eq!(name("app"), "bitbraid-1.0.0-universal.app.tar.gz");
        assert_eq!(
            file_name("deb", "1.1.0-rc.1", "aarch64").unwrap(),
            "bitbraid_1.1.0-rc.1_arm64.deb"
        );
        assert_eq!(file_name("flatpak", "1.0.0", "x86_64"), None);
    }

    #[test]
    fn versions_are_given_as_each_system_orders_them() {
        assert_eq!(distro_version("1.0.0"), "1.0.0");
        assert_eq!(distro_version("1.0.0-rc.1"), "1.0.0~rc.1");
        assert_eq!(msi_version("1.0.0").unwrap(), "1.0.0");
        assert_eq!(msi_version("1.2.3-rc.1").unwrap(), "1.2.3");
        assert!(msi_version("1.2").is_err());
        assert!(msi_version("256.0.0").is_err());
        assert!(arch_of("x86_64-unknown-linux-gnu").is_ok());
        assert_eq!(
            arch_of("aarch64-unknown-linux-gnu").unwrap().debian,
            "arm64"
        );
        assert!(arch_of("riscv64gc-unknown-linux-gnu").is_err());
    }

    #[test]
    fn the_glibc_needed_is_the_highest_named() {
        let binary = b"\0GLIBC_2.2.5\0GLIBC_2.34\0xGLIBC_2.17\0GLIBC_PRIVATE\0";
        assert_eq!(glibc_needed(binary), Some((2, 34)));
        assert_eq!(glibc_needed(b"no versions here"), None);
        assert_eq!(glibc_needed(b"GLIBC_2."), None);
    }

    #[test]
    fn the_seed_of_compile_time_constants_is_the_versions() {
        assert_eq!(const_random_seed("1.0.0"), "bitbraid-1.0.0");
        assert_ne!(const_random_seed("1.0.0"), const_random_seed("1.0.1"));
    }

    #[test]
    fn paths_are_remapped_target_last() {
        let flags = remap_flags(
            Path::new("/src"),
            Path::new("/cargo"),
            Path::new("/src/target"),
        );
        assert_eq!(
            flags,
            [
                "--remap-path-prefix=/src=/bitbraid",
                "--remap-path-prefix=/cargo=/cargo",
                "--remap-path-prefix=/src/target=/target"
            ]
        );
    }

    #[test]
    fn a_long_description_is_wrapped_after_a_space() {
        let text = debian_long_description(DESCRIPTION);
        assert!(
            text.lines()
                .all(|line| line.starts_with(' ') && line.len() <= 80)
        );
        assert!(text.ends_with('\n'));
        let words: Vec<_> = text.split_whitespace().collect();
        assert_eq!(words, DESCRIPTION.split_whitespace().collect::<Vec<_>>());
    }

    #[test]
    fn the_pkgbuild_is_filled_and_its_srcinfo_written() {
        let template =
            std::fs::read_to_string(crate::root().join("packaging/aur/PKGBUILD")).unwrap();
        let filled = fill_pkgbuild(&template, "1.0.0", "1.0.0", &"ab".repeat(32)).unwrap();
        assert!(filled.contains("\npkgver=1.0.0\n"));
        assert!(filled.contains(&format!("sha256sums=('{}')", "ab".repeat(32))));
        let info = srcinfo(&filled).unwrap();
        assert!(info.starts_with("pkgbase = bitbraid-bin\n"));
        assert!(info.contains("\tpkgver = 1.0.0\n"));
        assert!(info.contains("\tdepends = git\n"));
        assert!(info.contains(
            "\tsource = https://github.com/BitBraid/bitbraid/releases/download/v1.0.0/bitbraid-1.0.0-x86_64-linux.tar.gz\n"
        ));
        assert!(info.ends_with("\npkgname = bitbraid-bin\n"));
        assert!(fill_pkgbuild("pkgname=x\n", "1", "1", "a").is_err());
    }

    #[test]
    fn shell_words_are_split_as_a_pkgbuild_means_them() {
        assert_eq!(
            shell_words("'a b' \"c\" d # note").unwrap(),
            ["a b", "c", "d"]
        );
        assert_eq!(shell_words("").unwrap(), Vec::<String>::new());
    }

    #[test]
    fn the_installers_files_are_staged_with_the_licences_at_the_top() {
        let mut tree = Tree::new();
        tree.file("LICENSE.txt", b"licence".to_vec(), 0o644)
            .unwrap();
        tree.file("bitbraid.exe", b"exe".to_vec(), 0o755).unwrap();
        tree.file("git/cmd/git.exe", b"git".to_vec(), 0o755)
            .unwrap();
        let tree = tree.with_parents();
        let stage = std::env::temp_dir().join(format!("bitbraid-stage-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&stage);
        let files = stage.join("msi").join("BitBraid");
        stage_tree(&tree, &files).unwrap();
        assert_eq!(
            std::fs::read(files.join("LICENSE.txt")).unwrap(),
            b"licence"
        );
        assert_eq!(
            std::fs::read(files.join("git").join("cmd").join("git.exe")).unwrap(),
            b"git"
        );
        let _ = std::fs::remove_dir_all(&stage);
    }

    #[test]
    fn the_installer_names_every_file_once() {
        let mut tree = Tree::new();
        tree.file("bitbraid.exe", vec![1], 0o755).unwrap();
        tree.file("git/cmd/git.exe", vec![2], 0o644).unwrap();
        tree.file("git/etc/gitconfig", vec![3], 0o644).unwrap();
        tree.file("LICENSE.txt", vec![4], 0o644).unwrap();
        let tree = tree.with_parents();
        let source = wxs_source(
            &tree,
            Path::new("/stage/BitBraid"),
            Path::new("/stage/bitbraid.ico"),
            "1.0.0",
        )
        .unwrap();
        assert_eq!(source.matches("<Component Id=\"c").count(), 4);
        assert_eq!(source.matches("<ComponentRef Id=\"c").count(), 4);
        assert_eq!(source.matches("<Directory ").count(), 4);
        assert_eq!(
            source.matches("<Directory ").count(),
            source.matches("</Directory>").count()
        );
        assert!(source.contains("<Directory Id=\"d1\" Name=\"git\">"));
        assert!(source.contains("Version=\"1.0.0\""));
        // The executable is the second file in the tree's order.
        assert!(source.contains("Target=\"[#f2]\""));
        let cmd = source.find("Name=\"cmd\"").unwrap();
        let git_exe = source.find("git.exe\"").unwrap();
        let etc = source.find("Name=\"etc\"").unwrap();
        assert!(cmd < git_exe && git_exe < etc);
    }

    #[test]
    fn mingits_git_and_its_release_are_read_from_its_package_list() {
        let versions = "gawk 5.4.1-1\n\
                        mingw-w64-ucrt-x86_64-git-credential-manager 2.9.1-1\n\
                        mingw-w64-ucrt-x86_64-git 2.56.0.2-1\n\
                        mingw-w64-ucrt-x86_64-gitk 2.56.0.2-1\n";
        assert_eq!(
            mingit_git(versions),
            Some((
                "2.56.0.2".to_owned(),
                "v2.56.0.windows.2".to_owned(),
                "ucrt64"
            ))
        );
        // The first build of a release has no fourth number.
        assert_eq!(
            mingit_git("mingw-w64-x86_64-git 2.57.0-1\n"),
            Some((
                "2.57.0".to_owned(),
                "v2.57.0.windows.1".to_owned(),
                "mingw64"
            ))
        );
        for odd in [
            "",
            "gawk 5.4.1-1\n",
            "mingw-w64-ucrt-x86_64-git\n",
            "mingw-w64-ucrt-x86_64-git 2.56\n",
            "mingw-w64-ucrt-x86_64-git 2.56.0.2.1-1\n",
            "mingw-w64-ucrt-x86_64-git 2.56.x-1\n",
            "mingw-w64-ucrt-x86_64-git 2..0-1\n",
        ] {
            assert_eq!(mingit_git(odd), None, "{odd:?}");
        }
        let note = mingit_source_note(versions).unwrap();
        assert!(note.starts_with("MinGit 2.56.0.2, as BitBraid carries it\n"));
        assert!(note.contains("/releases/tag/v2.56.0.windows.2\n"));
        assert!(note.contains("/git/tree/v2.56.0.windows.2\n"));
        assert!(note.contains("ucrt64\\share\\licenses.\n"));
        assert!(mingit_source_note("gawk 5.4.1-1\n").is_err());
    }

    #[test]
    fn an_ico_holds_a_png_per_size() {
        let svg = std::fs::read(crate::root().join("brand/icon.svg")).unwrap();
        let out = ico(&svg).unwrap();
        assert_eq!(&out[..6], &[0, 0, 1, 0, 7, 0]);
        let first = u32::from_le_bytes(out[18..22].try_into().unwrap()) as usize;
        assert_eq!(first, 6 + 16 * 7);
        assert_eq!(&out[first..first + 8], b"\x89PNG\r\n\x1a\n");
        // The last entry is 256 pixels, written as 0.
        assert_eq!(out[6 + 16 * 6], 0);
    }
}
