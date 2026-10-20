//! A repository at a glance, from outside it: the branch it is on, how far
//! that is ahead of and behind what it follows, and its uncommitted files
//! by kind. What the page of repositories shows of each one it lists.
//!
//! One `git status` with the branch line answers all of it, in the folder
//! of the repository, without opening it any further: the page lists
//! repositories that are not open in any tab.
//!
//! Sources: git-status(1) (the short format, `--branch`, the branch line
//! and what it says of the upstream).

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};

use crate::{
    exe,
    wip::{Counts, count_status},
};

/// As many repositories as one look takes.
pub const MAX_GLANCED: usize = 64;

/// What a look at one repository found.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct Glance {
    /// Its folder, as it was given.
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// The branch checked out; none on a detached HEAD.
    pub branch: Option<String>,
    /// How many commits the branch has that what it follows has not, and
    /// the other way round; none without an upstream.
    pub ahead: Option<usize>,
    pub behind: Option<usize>,
    /// Its uncommitted files by kind.
    pub counts: Counts,
    /// When it was last fetched, in seconds since the Unix epoch
    /// ([`last_fetch`]); none when it never was (or `FETCH_HEAD` is gone).
    pub last_fetch: Option<i64>,
    /// Why it could not be looked at, when it could not.
    pub problem: Option<String>,
}

/// What the branch line of `git status --porcelain --branch` says: the
/// branch, and how far it is ahead and behind its upstream.
pub fn branch_line(porcelain: &str) -> (Option<String>, Option<usize>, Option<usize>) {
    let Some(line) = porcelain
        .lines()
        .next()
        .and_then(|line| line.strip_prefix("## "))
    else {
        return (None, None, None);
    };
    // A branch without commits yet.
    if let Some(name) = line.strip_prefix("No commits yet on ") {
        return (Some(name.trim().to_owned()), None, None);
    }
    // Detached: nobody's branch.
    if line.starts_with("HEAD (no branch)") {
        return (None, None, None);
    }
    let (names, counted) = match line.split_once(" [") {
        Some((names, rest)) => (names, rest.strip_suffix(']')),
        None => (line, None),
    };
    let (branch, upstream) = match names.split_once("...") {
        Some((branch, upstream)) => (branch, Some(upstream)),
        None => (names, None),
    };
    let number = |word: &str| {
        counted.and_then(|counted| {
            counted
                .split(", ")
                .find_map(|part| part.strip_prefix(word)?.trim().parse::<usize>().ok())
        })
    };
    // An upstream that is there has counts, what is not said being nought;
    // one that is gone has none.
    let gone = counted == Some("gone");
    let (ahead, behind) = match upstream {
        Some(_) if !gone => (
            Some(number("ahead ").unwrap_or(0)),
            Some(number("behind ").unwrap_or(0)),
        ),
        _ => (None, None),
    };
    (
        Some(branch.to_owned()).filter(|name| !name.is_empty()),
        ahead,
        behind,
    )
}

/// Look at one repository.
pub fn glance(path: &Path, cancel: &Arc<AtomicBool>) -> Glance {
    let mut found = Glance {
        path: path.to_path_buf(),
        ..Glance::default()
    };
    if !path.is_dir() {
        found.problem = Some(say!("glance-folder-missing").to_owned());
        return found;
    }
    found.last_fetch = last_fetch(path);
    let args = [
        "status",
        "--porcelain",
        "--branch",
        "--untracked-files=normal",
    ];
    // A bare repository has no files to change: only its branch is said.
    if is_bare(path) {
        let args = ["symbolic-ref", "-q", "--short", "HEAD"];
        match exe::run_streaming_in(path, args, cancel, &mut |_| {}) {
            Ok(said) => {
                found.branch = Some(said.stdout.trim().to_owned()).filter(|name| !name.is_empty())
            }
            Err(err) => found.problem = Some(err.to_string()),
        }
        return found;
    }
    match exe::run_streaming_in(path, args, cancel, &mut |_| {}) {
        Ok(said) => {
            let (branch, ahead, behind) = branch_line(&said.stdout);
            found.branch = branch;
            found.ahead = ahead;
            found.behind = behind;
            found.counts = count_status(&said.stdout);
        }
        Err(err) => found.problem = Some(err.to_string()),
    }
    found
}

/// Most of a `.git` file or a `commondir` file that is read: either names
/// one folder.
const MOST_POINTER: u64 = 4096;

/// The first line of a small file that names a folder, read with a bound.
fn pointer_line(file: &Path) -> Option<String> {
    use std::io::Read as _;
    let mut text = String::new();
    std::fs::File::open(file)
        .ok()?
        .take(MOST_POINTER)
        .read_to_string(&mut text)
        .ok()?;
    let line = text.lines().next()?.trim();
    (!line.is_empty()).then(|| line.to_owned())
}

/// A folder a pointer file names, relative to the folder `base` when it is
/// not absolute.
fn pointed(base: &Path, named: &str) -> PathBuf {
    let named = PathBuf::from(named);
    if named.is_absolute() {
        named
    } else {
        base.join(named)
    }
}

/// When the repository in `path` was last fetched: when `FETCH_HEAD` was
/// last written, which every fetch writes (git-fetch(1)), from the files
/// alone and without asking git. It is kept per worktree
/// (gitrepository-layout(5)): of a linked worktree or a submodule (whose
/// `.git` is a file that names its folder) the later of its own and the
/// repository's. None when none was ever written.
pub fn last_fetch(path: &Path) -> Option<i64> {
    let dot_git = path.join(".git");
    let git_dir = if dot_git.is_dir() {
        dot_git
    } else if dot_git.is_file() {
        let line = pointer_line(&dot_git)?;
        pointed(path, line.strip_prefix("gitdir:")?.trim())
    } else if is_bare(path) {
        path.to_path_buf()
    } else {
        return None;
    };
    let common = pointer_line(&git_dir.join("commondir")).map(|named| pointed(&git_dir, &named));
    let written = |dir: &Path| {
        let modified = std::fs::metadata(dir.join("FETCH_HEAD"))
            .ok()?
            .modified()
            .ok()?;
        let since = modified.duration_since(std::time::UNIX_EPOCH).ok()?;
        i64::try_from(since.as_secs()).ok()
    };
    [Some(git_dir), common]
        .into_iter()
        .flatten()
        .filter_map(|dir| written(&dir))
        .max()
}

/// Whether `path` is the folder of a bare repository: no `.git` in it, and
/// what a repository's own folder has.
fn is_bare(path: &Path) -> bool {
    !path.join(".git").exists()
        && path.join("HEAD").is_file()
        && path.join("objects").is_dir()
        && path.join("refs").is_dir()
}

/// Look at each of `paths`, at most [`MAX_GLANCED`] of them, one after the
/// other. Stops when `cancel` is set, with what was found so far.
pub fn glance_all(paths: &[PathBuf], cancel: &Arc<AtomicBool>) -> Vec<Glance> {
    let mut out = Vec::new();
    for path in paths.iter().take(MAX_GLANCED) {
        if cancel.load(std::sync::atomic::Ordering::SeqCst) {
            break;
        }
        out.push(glance(path, cancel));
    }
    out
}

/// Whether `path` is the folder of a repository itself, and not a folder
/// somewhere inside one: it has a `.git` (a folder, or the file a linked
/// worktree or a submodule has), or it is a bare repository's own folder.
pub fn is_repository_folder(path: &Path) -> bool {
    let dot_git = path.join(".git");
    (dot_git.is_dir() && dot_git.join("HEAD").is_file()) || dot_git.is_file() || is_bare(path)
}

/// Where the repository in `path` is cloned from: the fetch address of
/// its `origin`, or of its only remote. None when it has no remote, or
/// several and none of them is `origin`, or it cannot be opened.
pub fn remote_url(path: &Path) -> Option<String> {
    let repo = crate::Repository::open(path).ok()?;
    let inner = repo.gix();
    let names: Vec<String> = inner
        .remote_names()
        .iter()
        .map(|name| name.to_string())
        .collect();
    let name = match names.as_slice() {
        [only] => only.as_str(),
        many if many.iter().any(|name| name == "origin") => "origin",
        _ => return None,
    };
    let remote = inner.find_remote(name).ok()?;
    remote
        .url(gix::remote::Direction::Fetch)
        .map(|url| url.to_bstring().to_string())
}

#[cfg(test)]
mod tests {
    use super::branch_line;

    #[test]
    fn the_branch_line_says_the_branch_and_how_far_it_is_from_its_upstream() {
        assert_eq!(
            branch_line("## main...origin/main [ahead 1, behind 2]\n M a\n"),
            (Some("main".to_owned()), Some(1), Some(2))
        );
        assert_eq!(
            branch_line("## main...origin/main [behind 3]"),
            (Some("main".to_owned()), Some(0), Some(3))
        );
        assert_eq!(
            branch_line("## main...origin/main"),
            (Some("main".to_owned()), Some(0), Some(0))
        );
        assert_eq!(
            branch_line("## feature/x...origin/feature/x [gone]"),
            (Some("feature/x".to_owned()), None, None)
        );
        assert_eq!(
            branch_line("## main"),
            (Some("main".to_owned()), None, None)
        );
        assert_eq!(
            branch_line("## No commits yet on trunk"),
            (Some("trunk".to_owned()), None, None)
        );
        assert_eq!(branch_line("## HEAD (no branch)"), (None, None, None));
        for nonsense in ["", "M a", "##", "## ...x [ahead nine]"] {
            let (_, ahead, _) = branch_line(nonsense);
            assert!(ahead.is_none_or(|ahead| ahead == 0), "{nonsense:?}");
        }
    }
}
