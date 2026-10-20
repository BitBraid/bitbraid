//! Paths too long for Windows.
//!
//! Windows takes paths of fewer than 260 characters from a program that
//! does not ask for more. Git for Windows asks for more when `core.longpaths`
//! is on; while it is off, a file whose full path is longer
//! (`bitbraid_core::paths::beyond_windows_limit`) is one git cannot check
//! out, add or read, and git says "Filename too long". The engine's own
//! reads and writes go through the standard library, which hands a long
//! path to the system in the form that takes it, so what is too long
//! matters for git, and libgit2, which reads `core.longpaths` too.
//!
//! Here: which tracked files of a repository are beyond the limit at the
//! place its worktree is, and whether `core.longpaths` is on ([`report`]);
//! turning it on for the repository ([`enable`]); and what a person is told
//! when git, libgit2 or the system said a path is too long ([`advice`]),
//! which every error of this crate that carries such a message adds to it.
//!
//! Sources: Git for Windows' documentation and release notes of
//! `core.longpaths` ("Filename too long" while it is off); git-config(1);
//! libgit2's `core.longpaths` ("path too long"); Microsoft Learn,
//! "Maximum Path Length Limitation" and "System Error Codes"
//! (`ERROR_FILENAME_EXCED_RANGE`, 206).

use gix::bstr::ByteSlice as _;
use serde::Serialize;

use crate::{Repository, Result, exe, rerere::Scope};

/// What of a repository is too long for Windows.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct LongPaths {
    /// The limit holds on this system: it is Windows.
    pub applies: bool,
    /// Git's `core.longpaths` is on: git for Windows takes long paths.
    pub enabled: bool,
    /// How many tracked files are beyond the limit where the worktree is.
    pub beyond: usize,
    /// The longest of them, as git names it.
    pub longest: Option<String>,
    /// How long its full path is, in UTF-16 units, as Windows counts.
    pub longest_length: usize,
}

impl LongPaths {
    /// Whether git cannot work with every file here: the limit holds,
    /// files are beyond it, and `core.longpaths` is off.
    pub fn blocks_git(&self) -> bool {
        self.applies && !self.enabled && self.beyond > 0
    }
}

/// Which tracked files of `repo` are beyond what Windows takes without
/// long paths, at the place its worktree is, and whether `core.longpaths`
/// is on. A bare repository has no files there.
pub fn report(repo: &Repository) -> Result<LongPaths> {
    let enabled = repo
        .gix()
        .config_snapshot()
        .boolean("core.longpaths")
        .unwrap_or(false);
    let mut found = LongPaths {
        applies: cfg!(windows),
        enabled,
        ..LongPaths::default()
    };
    let Some(workdir) = repo.workdir() else {
        return Ok(found);
    };
    let root = bitbraid_core::paths::simplified(workdir)
        .to_string_lossy()
        .into_owned();
    let index = repo
        .gix()
        .index_or_empty()
        .map_err(crate::git_err(say!("repo-loading-index")))?;
    for entry in index.entries() {
        let path = entry.path(&index).to_str_lossy();
        let full = format!("{root}\\{}", path.replace('/', "\\"));
        if !bitbraid_core::paths::beyond_windows_limit(&full) {
            continue;
        }
        found.beyond = found.beyond.saturating_add(1);
        let length = bitbraid_core::paths::windows_length(&full);
        if length > found.longest_length {
            found.longest_length = length;
            found.longest = Some(path.into_owned());
        }
    }
    Ok(found)
}

/// Turn git's long paths on (`core.longpaths`), in this repository's own
/// configuration or in the user's, which every clone made afterwards
/// reads too: Git for Windows then takes paths of any length. Elsewhere
/// git reads the setting and does nothing with it. What is too long here
/// then, read again.
pub fn enable(repo: &Repository, scope: Scope) -> Result<LongPaths> {
    let file = match scope {
        Scope::Repo => "--local",
        Scope::User => "--global",
    };
    exe::run(repo, ["config", file, "core.longpaths", "true"], None)?;
    // What gitoxide read when the repository was opened is from before:
    // git is asked what holds now (the repository's own `false` still
    // wins over the user's `true`).
    let now = exe::run(
        repo,
        ["config", "--type=bool", "--get", "core.longpaths"],
        None,
    )
    .is_ok_and(|out| out.stdout.trim() == "true");
    let mut found = report(repo)?;
    found.enabled = now;
    Ok(found)
}

/// What a person is told when `said`, what git, libgit2 or the system
/// said, is that a path is too long: on Windows what to set; on another
/// system what the limit is. None when it says nothing of the kind.
pub fn advice(said: &str) -> Option<&'static str> {
    advice_for(said, cfg!(windows))
}

/// [`advice`] as it is on Windows (`windows`) or on another system.
pub fn advice_for(said: &str, windows: bool) -> Option<&'static str> {
    const SAID: [&str; 4] = [
        // Git for Windows, where the C library calls ENAMETOOLONG so.
        "filename too long",
        // libgit2.
        "path too long",
        // Windows itself (ERROR_FILENAME_EXCED_RANGE).
        "the filename or extension is too long",
        // Linux and macOS.
        "file name too long",
    ];
    let said = said.to_lowercase();
    if !SAID.iter().any(|known| said.contains(known)) {
        return None;
    }
    Some(if windows {
        say!("long-paths-advice-windows")
    } else {
        say!("long-paths-advice")
    })
}

/// [`advice`] as it follows what was said in an error: on a line of its
/// own, or nothing.
pub(crate) fn after(said: &str) -> String {
    match advice(said) {
        Some(advice) => format!("\n{advice}"),
        None => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_too_long_is_explained_in_our_words() {
        let windows = [
            "error: unable to create file a/very/long/path.txt: Filename too long",
            "fatal: cannot create directory at 'deep/folder': Filename too long",
            "warning: could not open directory 'x/': Filename too long",
            "path too long: 'C:/work/deep/file'",
            "spawning git: The filename or extension is too long. (os error 206)",
        ];
        for said in windows {
            let advice = advice_for(said, true).unwrap();
            assert!(advice.contains("core.longpaths true"), "{said}");
        }
        let advice = advice_for("open: File name too long (os error 36)", false).unwrap();
        assert!(advice.contains("255 bytes"));
        assert!(!advice.contains("longpaths"));
        for said in [
            "",
            "fatal: not a git repository",
            "path is fine",
            "too long ago",
        ] {
            assert_eq!(advice_for(said, true), None, "{said}");
        }
    }

    #[test]
    fn git_is_blocked_only_on_windows_with_long_paths_off_and_files_beyond() {
        let paths = |applies, enabled, beyond| LongPaths {
            applies,
            enabled,
            beyond,
            ..LongPaths::default()
        };
        assert!(paths(true, false, 1).blocks_git());
        assert!(!paths(true, true, 1).blocks_git());
        assert!(!paths(true, false, 0).blocks_git());
        assert!(!paths(false, false, 9).blocks_git());
    }
}
