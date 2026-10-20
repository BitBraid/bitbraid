//! bitbraid-testkit: deterministic fixture repositories, the git CLI oracle
//! and stand-in servers, for the tests.
//!
//! Fixtures are described by TOML scripts in `fixtures/` at the workspace root
//! and materialised with the real `git` binary under `<target>/fixtures/<name>`.
//! Every step runs with a hermetic configuration and a fixed clock, so object
//! ids are reproducible across machines and runs.
//!
//! Sources: git(1) "Environment Variables", git-init(1), git-commit(1),
//! git-merge(1), git-tag(1), git-checkout(1) `--orphan`.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

pub mod corpus;
pub mod fixture;
pub mod oracle;
pub mod proxy;
pub mod real_proxy;
pub mod scratch;
pub mod script;
pub mod signer;
pub mod smart_http;
pub mod stand_in;
pub mod tls;

pub use corpus::{CorpusSpec, ManyRefsSpec, build_many_refs, build_synthetic, build_wide};
pub use fixture::{FixtureScript, Step, build_all, build_fixture, list_scripts};
pub use oracle::Git;
pub use proxy::{Outcome, Passage, Proxy, ProxyOptions};
pub use scratch::{Scratch, SocketDir, copy_fixture};
pub use script::{make_runnable, script};
pub use signer::Signer;
pub use smart_http::{Served, SmartHttp, SmartHttpOptions};
pub use stand_in::{Answer, Asked, StandIn};
pub use tls::Authority;

use std::path::{Path, PathBuf};

/// Workspace root, derived from this crate's manifest directory at compile time.
pub fn workspace_root() -> PathBuf {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .ancestors()
        .nth(2)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| manifest.to_path_buf())
}

/// Directory where fixture repositories are materialised.
pub fn fixtures_out_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        // Taken apart and put together again: on Windows a target
        // directory given with forward slashes (`D:/work/target`) comes
        // out with the system's, as the paths the engine reports do.
        Some(dir) => separated(Path::new(&dir)).join("fixtures"),
        None => workspace_root().join("target").join("fixtures"),
    }
}

/// `path` with the system's separators between its parts.
fn separated(path: &Path) -> PathBuf {
    path.components().collect()
}

/// Directory under which mutable copies of fixtures are made. It is not
/// inside this checkout, nor inside any other repository: a test that
/// breaks its copy so far that git no longer takes it for a repository
/// must find nothing above it to act on instead. Each target directory
/// has its own, so that builds side by side do not meet.
///
/// On Unix the temporary directory is taken as the system resolves it:
/// macOS's is under `/var`, a link to `/private/var`, and what git, a
/// shell's `pwd` and the system's file events say is the resolved path.
pub fn scratch_root() -> PathBuf {
    let key = fnv1a64(fixtures_out_dir().to_string_lossy().as_bytes());
    let temp = std::env::temp_dir();
    // Resolved, as git and the application write the paths they report:
    // on Windows in long names where TEMP has short ones (`RUNNER~1` for
    // `runneradmin` on a hosted runner), and without the `\\?\` that
    // resolving gives there.
    #[cfg(unix)]
    let temp = std::fs::canonicalize(&temp).unwrap_or(temp);
    #[cfg(windows)]
    let temp = dunce::canonicalize(&temp).unwrap_or(temp);
    temp.join("bitbraid-scratch").join(format!("{key:016x}"))
}

/// Directory holding the fixture scripts.
pub fn fixtures_src_dir() -> PathBuf {
    workspace_root().join("fixtures")
}

/// Make `link` a symbolic link to the directory `target`. On Windows a link
/// to a directory is a kind of its own, and making one takes Developer Mode
/// or an administrator.
pub fn symlink_dir(target: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// Make `link` a symbolic link to the file `target` (see [`symlink_dir`]).
pub fn symlink_file(target: impl AsRef<Path>, link: impl AsRef<Path>) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link)
    }
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_file(target, link)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = (target, link);
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

/// 64-bit FNV-1a hash, used to key fixture caches. Stable across Rust versions,
/// unlike `DefaultHasher`.
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    #[test]
    fn a_path_comes_out_with_the_systems_separators() {
        let given = Path::new("D:/work/project/target");
        let wanted = if cfg!(windows) {
            PathBuf::from(r"D:\work\project\target")
        } else {
            PathBuf::from("D:/work/project/target")
        };
        assert_eq!(super::separated(given), wanted);
        assert_eq!(
            super::separated(Path::new("/srv/target")),
            Path::new("/srv/target").components().collect::<PathBuf>()
        );
    }
}
