//! `cargo xtask package-test [--dir <dir>] [system...]`: the Linux
//! packages installed and started in containers of the distributions
//! they are for. Opt-in and never part of the gate: it needs
//! Docker, the network for each distribution's packages, and the packages
//! made first (`cargo xtask package all`, in `packaging/linux/Dockerfile`).
//!
//! In each container `packaging/linux/package-test.sh` installs the
//! system's package (the `.deb` on Debian and Ubuntu, the `.rpm` on
//! Fedora, the PKGBUILD built by makepkg from the tarball on Arch), asks
//! it for its version, checks that every library it needs is there, that
//! the desktop entries and the AppStream metadata pass their validators,
//! and starts it with a software Vulkan driver: in a virtual X display,
//! where its window has to come up, and in a headless Wayland compositor,
//! where the screen has to show it drawn (`package-draw.sh`); then the
//! AppImage the same way, the tarball where it runs from, and the package
//! taken out again. The pictures of the screen are kept in
//! `<target>/package-test/<system>-package.png` and `-appimage.png`, to
//! be looked at. Each container may use 2 GB of memory and is removed
//! afterwards.
//!
//! Sources: docker-run(1) (`--rm`, `--memory`, `-v ...:ro`); the
//! documentation of each distribution's package manager (apt-get(8),
//! dnf(8), pacman(8), makepkg(8)); cage(1) and grim(1), wlroots'
//! `WLR_BACKENDS` and `WLR_RENDERER`.

use std::{path::PathBuf, process::Command};

use anyhow::{Context, Result, bail};

use crate::bundle;

/// The systems tried: a name, the image, and how it installs packages.
const SYSTEMS: [(&str, &str, &str); 6] = [
    ("debian-12", "debian:bookworm", "deb"),
    ("debian-13", "debian:trixie", "deb"),
    ("ubuntu-22.04", "ubuntu:22.04", "deb"),
    ("ubuntu-24.04", "ubuntu:24.04", "deb"),
    ("fedora", "fedora:latest", "rpm"),
    ("arch", "archlinux:latest", "arch"),
];

const USAGE: &str = "cargo xtask package-test [--dir <packages>] [debian-12 debian-13 ubuntu-22.04 ubuntu-24.04 fedora arch]";

pub fn run(args: &[String]) -> Result<()> {
    let root = crate::root();
    let mut dir = bundle::target_dir(&root).join("packages");
    let mut wanted = Vec::new();
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => {
                dir = PathBuf::from(
                    args.next()
                        .with_context(|| format!("--dir wants a directory\n{USAGE}"))?,
                );
            }
            other if SYSTEMS.iter().any(|(name, _, _)| *name == other) => {
                wanted.push(other.to_owned())
            }
            other => bail!("package-test: {other}: not a system or an option\n{USAGE}"),
        }
    }
    let dir = std::fs::canonicalize(&dir)
        .with_context(|| format!("{}: no packages there", dir.display()))?;
    let version = bundle::workspace_version(&root)?;
    let scripts = root.join("packaging").join("linux");
    let pictures = bundle::target_dir(&root).join("package-test");
    std::fs::create_dir_all(&pictures)?;

    let mut results = Vec::new();
    for (name, image, family) in SYSTEMS {
        if !wanted.is_empty() && !wanted.iter().any(|w| w == name) {
            continue;
        }
        println!("\n==> {name} ({image})");
        let container = format!("bitbraid-package-test-{name}");
        // One left over from a run that was stopped.
        let _ = Command::new("docker")
            .args(["rm", "-f", &container])
            .output();
        let status = Command::new("docker")
            .args([
                "run",
                "--rm",
                "--name",
                &container,
                "--memory",
                "2g",
                "--memory-swap",
                "3g",
            ])
            .arg("-v")
            .arg(format!("{}:/packages:ro", dir.display()))
            .arg("-v")
            .arg(format!("{}:/test:ro", scripts.display()))
            .arg("-v")
            .arg(format!("{}:/out", pictures.display()))
            .args([image, "sh", "/test/package-test.sh", family, &version, name])
            .status()
            .context("starting docker")?;
        results.push((name, status.success()));
    }
    println!("\n==> package-test");
    for (name, passed) in &results {
        println!("  {name:<14} {}", if *passed { "passed" } else { "FAILED" });
    }
    if results.iter().any(|(_, passed)| !passed) {
        bail!("package-test: a system failed");
    }
    Ok(())
}
