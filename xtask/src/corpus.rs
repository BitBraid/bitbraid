//! `cargo xtask corpus`: the oracle comparison on real repositories.
//!
//! The generated fixtures are small and regular. The corpus is what other
//! people's history looks like: clock skew, octopus merges, several roots,
//! tags of blobs, messages in other encodings. The repositories are bare
//! clones kept under the bench target directory (`<target>/corpus/`),
//! fetched the first time they are asked for (with the commit-graph file
//! git's maintenance would write) and never updated behind anyone's back;
//! nothing of them enters the checkout. The comparison itself is
//! `crates/git/tests/suite/corpus.rs`, run in release mode.
//!
//! This needs the network once per repository, so it is no part of
//! `cargo xtask ci`.
//!
//! Sources: git-clone(1), git-commit-graph(1).

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};
use bitbraid_testkit::Git;

/// A repository of the corpus: what it is called here, the directory it is
/// kept in, and where it comes from.
struct Known {
    name: &'static str,
    dir: &'static str,
    url: &'static str,
}

const KNOWN: &[Known] = &[
    Known {
        name: "cargo",
        dir: "cargo.git",
        url: "https://github.com/rust-lang/cargo",
    },
    Known {
        name: "git",
        dir: "git.git",
        url: "https://github.com/git/git",
    },
];

const USAGE: &str = "\
cargo xtask corpus [--offline] [--list] [--repo <path>]... [name]...

  Runs the oracle comparison (references, graph, diffs, blames, file
  histories, statuses against git) on the corpus repositories, cloning the
  ones that are missing into <bench target>/corpus. Without names: all of
  them.

  --offline      never clone; a missing repository is an error
  --list         print the corpus and where each repository is, and stop
  --repo <path>  also compare this repository (it is only read; a scratch
                 clone is made of it for the status checks)
";

/// The target directory benchmarks and corpus runs build into and keep
/// their repositories under, always absolute: the tests run with their
/// crate's directory as the current one, so a relative path would name a
/// different place there (and regenerate a million commits in it).
pub(crate) fn bench_target() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR").filter(|dir| !dir.is_empty()) {
        Some(dir) => {
            let dir = PathBuf::from(dir);
            if dir.is_absolute() {
                dir
            } else {
                crate::root().join(dir)
            }
        }
        None => crate::root().join("target-bench"),
    }
}

/// Where the corpus repositories are kept.
pub(crate) fn corpus_dir() -> PathBuf {
    bench_target().join("corpus")
}

pub fn run(args: &[String]) -> Result<()> {
    let mut offline = false;
    let mut list = false;
    let mut names: Vec<&str> = Vec::new();
    let mut extra: Vec<PathBuf> = Vec::new();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--offline" => offline = true,
            "--list" => list = true,
            "--repo" => {
                let path = rest.next().context("--repo needs a path")?;
                let path = std::fs::canonicalize(path)
                    .with_context(|| format!("no repository at {path}"))?;
                extra.push(path);
            }
            "--help" | "-h" => {
                print!("{USAGE}");
                return Ok(());
            }
            flag if flag.starts_with('-') => bail!("unknown option {flag:?}\n{USAGE}"),
            name => names.push(name),
        }
    }
    let wanted: Vec<&Known> = if names.is_empty() && extra.is_empty() {
        KNOWN.iter().collect()
    } else {
        names
            .iter()
            .map(|name| {
                KNOWN
                    .iter()
                    .find(|known| known.name == *name)
                    .with_context(|| {
                        format!(
                            "no corpus repository called {name:?} (there are: {})",
                            KNOWN
                                .iter()
                                .map(|known| known.name)
                                .collect::<Vec<_>>()
                                .join(", ")
                        )
                    })
            })
            .collect::<Result<_>>()?
    };

    let dir = corpus_dir();
    if list {
        for known in KNOWN {
            let path = dir.join(known.dir);
            let state = if is_repository(&path) {
                "present"
            } else {
                "missing"
            };
            println!("{}\t{}\t{}\t{state}", known.name, known.url, path.display());
        }
        return Ok(());
    }

    let mut repositories: Vec<(String, PathBuf)> = Vec::new();
    for known in wanted {
        let path = ensure(&dir, known, offline)?;
        repositories.push((known.name.to_owned(), path));
    }
    for path in extra {
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "repository".to_owned());
        repositories.push((name, path));
    }
    if repositories.is_empty() {
        bail!("nothing to compare\n{USAGE}");
    }

    let listed = repositories
        .iter()
        .map(|(name, path)| format!("{name}\t{}", path.display()))
        .collect::<Vec<_>>()
        .join("\n");
    println!(
        "\n==> oracle comparison on {} repositories",
        repositories.len()
    );
    let status = Command::new("cargo")
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
            "--test-threads=1",
            // The tests of the module `corpus`; a filter is a part of the
            // name, which those of `bench_corpus` also have.
            "corpus::",
            "--skip",
            "bench_corpus::",
        ])
        .env("BITBRAID_CORPUS_REPOS", listed)
        .env("CARGO_TARGET_DIR", bench_target())
        .current_dir(crate::root())
        .status()
        .context("spawning cargo test")?;
    if !status.success() {
        bail!("the corpus comparison failed with {status}");
    }
    println!("\ncorpus: every comparison passed");
    Ok(())
}

/// A bare repository, or the top of one with a working tree.
fn is_repository(path: &Path) -> bool {
    path.join("HEAD").is_file() || path.join(".git").exists()
}

/// The clone of `known` under `dir`, made now if it is not there. A clone
/// that was interrupted leaves a directory with another name behind, never
/// half a repository under the real one.
fn ensure(dir: &Path, known: &Known, offline: bool) -> Result<PathBuf> {
    let path = dir.join(known.dir);
    if is_repository(&path) {
        println!("corpus: {} is at {}", known.name, path.display());
        return Ok(path);
    }
    if offline {
        bail!(
            "{} is not at {} and --offline forbids cloning it from {}",
            known.name,
            path.display(),
            known.url
        );
    }
    std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    let partial = dir.join(format!("{}.partial", known.dir));
    if partial.exists() {
        std::fs::remove_dir_all(&partial)
            .with_context(|| format!("removing {}", partial.display()))?;
    }
    println!(
        "corpus: cloning {} from {} into {}",
        known.name,
        known.url,
        path.display()
    );
    // The hermetic runner: no credentials, no identity but the fixtures',
    // nothing of the developer's configuration goes along.
    let git = Git::new(dir, bench_target().join("fixtures").join(".home"));
    git.ensure_home()?;
    git.run([
        "clone".as_ref(),
        "--quiet".as_ref(),
        "--bare".as_ref(),
        known.url.as_ref(),
        partial.as_os_str(),
    ])
    .with_context(|| format!("cloning {}", known.url))?;
    // A clone has no commit-graph file until git's maintenance writes one;
    // a repository in use has one, and so does the synthetic corpus. The
    // walk of git.git takes many times as long without it, which would
    // measure the clone and not the repository.
    git.run([
        "-C".as_ref(),
        partial.as_os_str(),
        "commit-graph".as_ref(),
        "write".as_ref(),
        "--reachable".as_ref(),
    ])
    .with_context(|| format!("writing the commit-graph of {}", known.name))?;
    std::fs::rename(&partial, &path)
        .with_context(|| format!("moving the clone to {}", path.display()))?;
    Ok(path)
}
