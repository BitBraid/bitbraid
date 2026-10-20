//! `cargo xtask bundle`: the macOS application, `BitBraid.app`, and with
//! `--dmg` a disk image to install it from.
//!
//! The binary is built with the `dist` profile (another with `--profile`)
//! for this Mac's processor, or with `--universal` for both Apple silicon
//! and Intel, joined into one with `lipo`, as a release builds it
//! (`cargo xtask package`'s build: the lock file, `SOURCE_DATE_EPOCH`, the
//! paths remapped). The bundle gets `packaging/macos/Info.plist` with the
//! workspace's version in it, an icon drawn from `brand/icon.svg` at
//! every size macOS asks for, written here as an `.icns` (the tile at the
//! size macOS's own icons have in their canvas, with nothing added to
//! it), and the licences, BitBraid's and those of the crates in the
//! binary (`cargo xtask sbom`).
//!
//! With `--git <dir>`, a relocatable build of git installed in `<dir>`
//! (`packaging/macos/build-git.sh`, which puts git's licences and
//! `SOURCE.txt` beside it) is copied into `Contents/Resources/git`, links
//! kept, where `bitbraid_git::shipped` looks for it. Not
//! `Contents/Helpers`: what lies in a place for code must be code, every
//! file of it signed, and git's installation is scripts and data as much
//! as programs, a script's signature being kept in extended attributes
//! that the updater's archive does not carry. Among the resources the
//! whole installation is sealed with the bundle, and each Mach-O program
//! of it is signed on its own before that.
//!
//! It is signed ad hoc (`codesign --sign -`), with no developer's
//! certificate and not notarised: the system asks before the first start
//! of a copy that came from the internet (open it from the Finder's menu,
//! or `xattr -dr com.apple.quarantine BitBraid.app`), and the keychain
//! asks once again for its tokens after an update, a new build being
//! another program to it. With `--sign <identity>` (a Developer ID
//! Application certificate in the keychain) every program in it is signed
//! with that identity, the hardened runtime and a secure timestamp, as
//! notarisation asks, and so is the disk image; notarising it is the
//! release workflow's step.
//!
//! The disk image holds the application and a link to `/Applications`,
//! to drag it onto.
//!
//! Sources: Apple's "Bundle Programming Guide" (the layout of an
//! application bundle), "Placing content in a bundle", Technical Note
//! TN2206, "macOS Code Signing In Depth" (the places for nested code hold
//! only signed code; scripts and data in `Contents/Resources`, where code
//! is sealed as a resource), the "Information Property List" reference, the
//! Human Interface Guidelines' "App icons" (the macOS icon grid: the body
//! takes 824 of 1024), "Notarizing macOS software before distribution"
//! and "Resolving common notarization issues" (every executable signed
//! with a Developer ID, the hardened runtime and a secure timestamp,
//! nested code signed before what holds it), codesign(1) (`--options
//! runtime`, `--timestamp`), lipo(1), hdiutil(1), ditto(1), iconutil(1)
//! and the ICNS format as it writes it (the `icns` header and the `icp4`,
//! `icp5` and `ic07` to `ic14` entries that hold PNG pictures); the
//! Mach-O magic numbers of `<mach-o/loader.h>` and `<mach-o/fat.h>`; the
//! documentation of resvg 0.46.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

/// What the bundle is called, and its executable.
const APP: &str = "BitBraid.app";
const EXECUTABLE: &str = "bitbraid";

/// The two processors of a universal build.
const UNIVERSAL: [&str; 2] = ["aarch64-apple-darwin", "x86_64-apple-darwin"];

/// The oldest macOS the bundle is for: `LSMinimumSystemVersion`, and what
/// the binary is built for.
const DEPLOYMENT_TARGET: &str = "11.0";

/// The pictures of the icon: the type of each entry and its side in
/// pixels, those of an iconset (`iconutil`). `ic11` to `ic14` are the
/// doubled ones of 16 to 256.
const ICON_ENTRIES: [(&[u8; 4], u32); 10] = [
    (b"icp4", 16),
    (b"icp5", 32),
    (b"ic07", 128),
    (b"ic08", 256),
    (b"ic09", 512),
    (b"ic10", 1024),
    (b"ic11", 32),
    (b"ic12", 64),
    (b"ic13", 256),
    (b"ic14", 512),
];

/// How much of the canvas the tile takes, as macOS's own icons do.
const ICON_BODY: f32 = 824.0 / 1024.0;

const USAGE: &str = "cargo xtask bundle [--profile <name>] [--universal] [--dmg] [--no-build] [--git <dir>] [--sign <identity>]";

#[derive(Debug, Default)]
struct Options {
    profile: String,
    universal: bool,
    dmg: bool,
    build: bool,
    git: Option<PathBuf>,
    /// The signing identity; `-` signs ad hoc.
    identity: String,
}

fn options(args: &[String]) -> Result<Options> {
    let mut options = Options {
        profile: "dist".to_owned(),
        build: true,
        identity: "-".to_owned(),
        ..Options::default()
    };
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => {
                options.profile = args
                    .next()
                    .with_context(|| format!("--profile wants a name\n{USAGE}"))?
                    .clone();
            }
            "--git" => {
                options.git =
                    Some(PathBuf::from(args.next().with_context(|| {
                        format!("--git wants a directory\n{USAGE}")
                    })?));
            }
            "--sign" => {
                let identity = args
                    .next()
                    .with_context(|| format!("--sign wants an identity\n{USAGE}"))?;
                if identity.trim().is_empty() {
                    bail!("--sign wants an identity\n{USAGE}");
                }
                options.identity = identity.clone();
            }
            "--universal" => options.universal = true,
            "--dmg" => options.dmg = true,
            "--no-build" => options.build = false,
            other => bail!("bundle: {other}: not an option\n{USAGE}"),
        }
    }
    Ok(options)
}

pub fn run(args: &[String]) -> Result<()> {
    let options = options(args)?;
    if !cfg!(target_os = "macos") {
        bail!("bundle: an application bundle is made on macOS, with its own tools");
    }
    let root = crate::root();
    let target = target_dir(&root);
    let version = workspace_version(&root)?;
    if let Some(git) = &options.git {
        check_git(git)?;
    }

    let binaries = if options.universal {
        UNIVERSAL
            .iter()
            .map(|triple| {
                target
                    .join(triple)
                    .join(profile_dir(&options.profile))
                    .join(EXECUTABLE)
            })
            .collect::<Vec<_>>()
    } else {
        vec![target.join(profile_dir(&options.profile)).join(EXECUTABLE)]
    };
    if options.build {
        let targets: &[&str] = if options.universal { &UNIVERSAL } else { &[] };
        crate::package::release_build(
            &root,
            &options.profile,
            targets,
            &[("MACOSX_DEPLOYMENT_TARGET", DEPLOYMENT_TARGET)],
        )?;
    }
    for binary in &binaries {
        if !binary.is_file() {
            bail!("bundle: {} was not built", binary.display());
        }
    }

    let out = target.join("bundle");
    let app = out.join(APP);
    if app.exists() {
        std::fs::remove_dir_all(&app).with_context(|| format!("removing {}", app.display()))?;
    }
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    std::fs::create_dir_all(&macos)?;
    std::fs::create_dir_all(&resources)?;

    let executable = macos.join(EXECUTABLE);
    match binaries.as_slice() {
        [one] => {
            std::fs::copy(one, &executable)
                .with_context(|| format!("copying {}", one.display()))?;
        }
        several => {
            let mut lipo = Command::new("lipo");
            lipo.arg("-create").arg("-output").arg(&executable);
            lipo.args(several);
            run_tool(lipo, "lipo")?;
        }
    }

    let plist = std::fs::read_to_string(root.join("packaging/macos/Info.plist"))
        .context("reading packaging/macos/Info.plist")?;
    std::fs::write(
        contents.join("Info.plist"),
        plist.replace("@VERSION@", &version),
    )?;
    std::fs::write(contents.join("PkgInfo"), "APPL????")?;

    let svg = std::fs::read(root.join("brand/icon.svg")).context("reading the icon")?;
    std::fs::write(resources.join("BitBraid.icns"), icns(&svg)?)?;

    // The licences, as every package carries them.
    std::fs::copy(root.join("LICENSE"), resources.join("LICENSE"))?;
    let (crates, _) = crate::sbom::shipped_crates(&root, UNIVERSAL[0])?;
    std::fs::write(
        resources.join("THIRD-PARTY-LICENSES.txt"),
        crate::sbom::third_party_text(&version, "universal-apple-darwin", &crates)?,
    )?;

    // The shipped git, links kept: its programs are links to one another.
    let shipped = resources.join("git");
    if let Some(git) = &options.git {
        let mut copy = Command::new("ditto");
        copy.arg(git).arg(&shipped);
        run_tool(copy, "ditto")?;
    }

    // The shipped git's programs first, each on its own, then the bundle
    // that seals them with the rest. Ad hoc: no certificate behind it.
    // With an identity: the hardened runtime and a secure timestamp, as
    // notarisation asks.
    let sign = |path: &Path| -> Result<()> {
        let mut codesign = Command::new("codesign");
        codesign.args(["--force", "--sign", &options.identity]);
        if options.identity != "-" {
            codesign.args(["--options", "runtime", "--timestamp"]);
        }
        codesign.arg(path);
        run_tool(codesign, "codesign")
    };
    if shipped.is_dir() {
        for program in mach_o_files(&shipped)? {
            sign(&program)?;
        }
    }
    sign(&app)?;
    let mut verify = Command::new("codesign");
    verify.args(["--verify", "--strict", "--deep"]).arg(&app);
    run_tool(verify, "codesign --verify")?;
    println!("bundle: {}", app.display());

    if options.dmg {
        let arch = if options.universal {
            "universal".to_owned()
        } else {
            std::env::consts::ARCH.replace("aarch64", "arm64")
        };
        let image = out.join(format!("bitbraid-{version}-{arch}.dmg"));
        disk_image(&app, &image)?;
        if options.identity != "-" {
            let mut codesign = Command::new("codesign");
            codesign
                .args(["--force", "--sign", &options.identity, "--timestamp"])
                .arg(&image);
            run_tool(codesign, "codesign")?;
        }
        println!("bundle: {}", image.display());
    }
    Ok(())
}

/// The files a git to ship must have: itself, its licence, and the note of
/// where its source is, which `packaging/macos/build-git.sh` writes.
const SHIPPED_GIT_FILES: [&str; 3] = ["bin/git", "COPYING", "SOURCE.txt"];

/// Whether `dir` holds a git to ship, with its licence and its source's
/// note beside it.
fn check_git(dir: &Path) -> Result<()> {
    let missing: Vec<&str> = SHIPPED_GIT_FILES
        .into_iter()
        .filter(|file| !dir.join(file).is_file())
        .collect();
    if !missing.is_empty() {
        bail!(
            "bundle: {}: no {} there; build it with packaging/macos/build-git.sh",
            dir.display(),
            missing.join(", ")
        );
    }
    Ok(())
}

/// The Mach-O programs and libraries under `dir`, links passed over, the
/// deepest first.
fn mach_o_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in
            std::fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))?
        {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() && is_mach_o(&entry.path()) {
                found.push(entry.path());
            }
        }
    }
    found.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    Ok(found)
}

/// Whether the file starts as a Mach-O file does, thin or universal.
fn is_mach_o(path: &Path) -> bool {
    use std::io::Read as _;
    let mut magic = [0u8; 4];
    let read = std::fs::File::open(path).and_then(|mut file| file.read_exact(&mut magic));
    read.is_ok()
        && matches!(
            magic,
            [0xfe, 0xed, 0xfa, 0xce | 0xcf]
                | [0xce | 0xcf, 0xfa, 0xed, 0xfe]
                | [0xca, 0xfe, 0xba, 0xbe]
        )
}

/// The target directory cargo builds into.
pub(crate) fn target_dir(root: &Path) -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                root.join(dir)
            }
        }
        None => root.join("target"),
    }
}

/// The folder a profile builds into.
pub(crate) fn profile_dir(profile: &str) -> &str {
    match profile {
        "dev" | "test" => "debug",
        "bench" => "release",
        other => other,
    }
}

/// The version of the workspace, from its manifest.
pub(crate) fn workspace_version(root: &Path) -> Result<String> {
    let manifest: toml::Value = toml::from_str(
        &std::fs::read_to_string(root.join("Cargo.toml")).context("reading Cargo.toml")?,
    )?;
    manifest
        .get("workspace")
        .and_then(|workspace| workspace.get("package"))
        .and_then(|package| package.get("version"))
        .and_then(toml::Value::as_str)
        .map(str::to_owned)
        .context("Cargo.toml has no [workspace.package] version")
}

fn run_tool(mut command: Command, what: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("starting {what}"))?;
    if !status.success() {
        bail!("{what} failed with {status}");
    }
    Ok(())
}

/// A disk image holding the application and a link to `/Applications`.
fn disk_image(app: &Path, image: &Path) -> Result<()> {
    let staging = image.with_extension("staging");
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    let mut copy = Command::new("ditto");
    copy.arg(app).arg(staging.join(APP));
    run_tool(copy, "ditto")?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", staging.join("Applications"))
        .context("linking /Applications")?;
    let mut hdiutil = Command::new("hdiutil");
    hdiutil
        .args([
            "create", "-volname", "BitBraid", "-fs", "HFS+", "-format", "UDZO", "-ov",
        ])
        .arg("-srcfolder")
        .arg(&staging)
        .arg(image);
    let made = run_tool(hdiutil, "hdiutil create");
    let _ = std::fs::remove_dir_all(&staging);
    made
}

/// The icon drawn as a PNG of `side` pixels, the drawing taking `body` of
/// the canvas, in its middle.
pub(crate) fn draw_icon(svg: &[u8], side: u32, body: f32) -> Result<Vec<u8>> {
    use resvg::{tiny_skia, usvg};
    let tree = usvg::Tree::from_data(svg, &usvg::Options::default())
        .context("reading the icon's drawing")?;
    let drawn = tree.size().width().max(tree.size().height());
    let mut pixmap = tiny_skia::Pixmap::new(side, side).context("no room for the icon")?;
    let inner = side as f32 * body;
    let scale = inner / drawn;
    let inset = (side as f32 - inner) / 2.0;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_row(scale, 0.0, 0.0, scale, inset, inset),
        &mut pixmap.as_mut(),
    );
    pixmap.encode_png().context("writing the icon's picture")
}

/// The icon as an `.icns`: the drawing at every size of
/// [`ICON_ENTRIES`], as PNG pictures.
fn icns(svg: &[u8]) -> Result<Vec<u8>> {
    let mut entries = Vec::new();
    for (kind, side) in ICON_ENTRIES {
        entries.push((*kind, draw_icon(svg, side, ICON_BODY)?));
    }
    Ok(icns_of(&entries))
}

/// An `.icns` of these entries: `icns` and the whole length, then each
/// entry's type, its length with its header, and its picture.
fn icns_of(entries: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let length = |data: usize| u32::try_from(data + 8).unwrap_or(u32::MAX);
    let total: usize = 8 + entries
        .iter()
        .map(|(_, data)| data.len() + 8)
        .sum::<usize>();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(b"icns");
    out.extend_from_slice(&u32::try_from(total).unwrap_or(u32::MAX).to_be_bytes());
    for (kind, data) in entries {
        out.extend_from_slice(kind);
        out.extend_from_slice(&length(data.len()).to_be_bytes());
        out.extend_from_slice(data);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_icns_is_its_header_and_its_entries() {
        let out = icns_of(&[(*b"ic07", vec![1, 2, 3]), (*b"icp4", vec![])]);
        assert_eq!(&out[..4], b"icns");
        assert_eq!(u32::from_be_bytes(out[4..8].try_into().unwrap()), 27);
        assert_eq!(out.len(), 27);
        assert_eq!(&out[8..12], b"ic07");
        assert_eq!(u32::from_be_bytes(out[12..16].try_into().unwrap()), 11);
        assert_eq!(&out[16..19], &[1, 2, 3]);
        assert_eq!(&out[19..23], b"icp4");
        assert_eq!(u32::from_be_bytes(out[23..27].try_into().unwrap()), 8);
    }

    #[test]
    fn the_icon_is_drawn_at_every_size() {
        let svg = std::fs::read(crate::root().join("brand/icon.svg")).unwrap();
        let out = icns(&svg).unwrap();
        let mut at = 8;
        let mut kinds = Vec::new();
        while at < out.len() {
            let kind: [u8; 4] = out[at..at + 4].try_into().unwrap();
            let length = u32::from_be_bytes(out[at + 4..at + 8].try_into().unwrap()) as usize;
            // Each picture is a PNG.
            assert_eq!(&out[at + 8..at + 16], b"\x89PNG\r\n\x1a\n");
            kinds.push(kind);
            at += length;
        }
        assert_eq!(at, out.len());
        assert_eq!(kinds.len(), ICON_ENTRIES.len());
    }

    #[test]
    fn options_are_read_and_odd_ones_refused() {
        let read =
            |args: &[&str]| options(&args.iter().map(|a| (*a).to_owned()).collect::<Vec<_>>());
        let default = read(&[]).unwrap();
        assert_eq!(default.profile, "dist");
        assert!(default.build && !default.universal && !default.dmg);
        assert_eq!(default.identity, "-");
        assert_eq!(default.git, None);
        let all = read(&[
            "--profile",
            "release",
            "--universal",
            "--dmg",
            "--no-build",
            "--git",
            "/tmp/git",
            "--sign",
            "Developer ID Application: Someone (TEAM123456)",
        ])
        .unwrap();
        assert_eq!(all.profile, "release");
        assert!(!all.build && all.universal && all.dmg);
        assert_eq!(all.git, Some(PathBuf::from("/tmp/git")));
        assert_eq!(
            all.identity,
            "Developer ID Application: Someone (TEAM123456)"
        );
        assert!(read(&["--profile"]).is_err());
        assert!(read(&["--sign"]).is_err());
        assert!(read(&["--sign", " "]).is_err());
        assert!(read(&["--git"]).is_err());
        assert!(read(&["--notarise"]).is_err());
        assert_eq!(profile_dir("dev"), "debug");
        assert_eq!(profile_dir("dist"), "dist");
    }

    #[test]
    fn a_git_to_ship_brings_its_licence_and_its_sources_note() {
        let dir = std::env::temp_dir().join(format!("bitbraid-git-check-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("bin")).unwrap();
        std::fs::write(dir.join("bin/git"), b"").unwrap();
        let said = check_git(&dir).unwrap_err().to_string();
        assert!(said.contains("no COPYING, SOURCE.txt there"), "{said}");
        std::fs::write(dir.join("COPYING"), b"").unwrap();
        std::fs::write(dir.join("SOURCE.txt"), b"").unwrap();
        let checked = check_git(&dir);
        let _ = std::fs::remove_dir_all(&dir);
        checked.unwrap();
        assert!(check_git(Path::new("/nowhere/at/all")).is_err());
    }

    #[test]
    fn mach_o_files_are_known_by_their_magic() {
        let dir = std::env::temp_dir().join(format!("bitbraid-macho-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("libexec/git-core")).unwrap();
        std::fs::write(dir.join("git"), [0xcf, 0xfa, 0xed, 0xfe, 7, 0, 0, 1]).unwrap();
        std::fs::write(
            dir.join("libexec/git-core/git-remote-http"),
            [0xca, 0xfe, 0xba, 0xbe, 0],
        )
        .unwrap();
        std::fs::write(dir.join("libexec/git-core/git-sh-setup"), b"#!/bin/sh\n").unwrap();
        std::fs::write(dir.join("short"), [0xcf]).unwrap();
        let found = mach_o_files(&dir).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        let names: Vec<_> = found
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        // The deepest first.
        assert_eq!(names, ["git-remote-http", "git"]);
    }
}
