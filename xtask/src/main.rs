//! `cargo xtask <command>`: repository automation.
//!
//! This is a developer tool and is exempt from the workspace's never-crash
//! lints, but it still reports errors through `Result` rather than panicking.

mod appbench;
mod archive;
mod bundle;
mod ci;
mod corpus;
mod feed;
mod fixtures;
mod layers;
mod package;
mod pkgtest;
mod rpm;
mod sbom;
mod snapshot;

use std::process::ExitCode;

use anyhow::Result;

const USAGE: &str = "\
cargo xtask <command> [args]

  ci                   fmt, clippy, tests, snapshot --check, layers, fixtures, deny
  layers               check that crates only depend on strictly lower layers
  fixtures [--clean]   build and verify fixture repositories from fixtures/*.toml
  snapshot [--check]   draw every view off screen and store the pictures; --check compares them
  bench [--commits N]  run the corpus benchmarks one at a time (release build, prints BENCH lines)
  corpus [name...]     compare with git on real repositories (clones them once; not part of ci)
  appbench [options]   run the application off screen on a million commits: fps and peak memory
  bundle [options]     macOS: BitBraid.app, signed ad hoc or --sign; --dmg a disk image, --universal both processors, --git a shipped git
  package <kind>...    a release's packages: tarball appimage deb rpm aur (all), zip wxs (Windows)
  package-test [sys]   the Linux packages installed and started in containers (Docker; not part of ci)
  sbom [--out <dir>]   the bill of materials (CycloneDX) of each system's binary
  feed <dir> [--key k] a release's SHA256SUMS, minisign signatures and update feed (latest.json)
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.split_first() {
        Some((cmd, rest)) => dispatch(cmd, rest),
        None => {
            eprint!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cmd: &str, rest: &[String]) -> Result<()> {
    match cmd {
        "ci" => ci::run(rest),
        "layers" => layers::run(),
        "snapshot" => snapshot::run(rest),
        "fixtures" => fixtures::run(rest),
        "bench" => bench(rest),
        "corpus" => corpus::run(rest),
        "appbench" => appbench::run(rest),
        "bundle" => bundle::run(rest),
        "package" => package::run(rest),
        "package-test" => pkgtest::run(rest),
        "sbom" => sbom::run(rest),
        "feed" => feed::run(rest),
        "help" | "--help" | "-h" => {
            print!("{USAGE}");
            Ok(())
        }
        other => anyhow::bail!("unknown command {other:?}\n{USAGE}"),
    }
}

/// The workspace root, as the testkit finds it.
pub(crate) fn root() -> std::path::PathBuf {
    bitbraid_testkit::workspace_root()
}

fn bench(args: &[String]) -> Result<()> {
    let commits = args
        .iter()
        .position(|a| a == "--commits")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "1000000".to_owned());
    let status = std::process::Command::new("cargo")
        .args([
            "test",
            "--release",
            "--package",
            "bitbraid-git",
            "--test",
            "suite",
            "--",
            "--ignored",
            "--nocapture",
            // One at a time: side by side they share the processor and
            // the disk, and each reads slower than it is.
            "--test-threads=1",
            "bench_corpus::",
        ])
        .env("BITBRAID_CORPUS_COMMITS", commits)
        .env(
            "BITBRAID_GIT_GIT",
            std::env::var_os("BITBRAID_GIT_GIT").unwrap_or_else(|| {
                let default = corpus::corpus_dir().join("git.git");
                if default.is_dir() {
                    default.into_os_string()
                } else {
                    std::ffi::OsString::new()
                }
            }),
        )
        // Absolute, or the tests (which run in their crate's directory)
        // would look for the corpora somewhere else and generate a million
        // commits anew there.
        .env("CARGO_TARGET_DIR", corpus::bench_target())
        .current_dir(root())
        .status()?;
    if !status.success() {
        anyhow::bail!("benchmark failed with {status}");
    }
    Ok(())
}
