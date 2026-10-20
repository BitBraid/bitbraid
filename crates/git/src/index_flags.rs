//! Flags on entries of the index that change how git treats a file:
//! intent-to-add (the file is known, its content not staged yet),
//! skip-worktree (the file in the worktree is left alone and not looked
//! at) and assume-unchanged (git does not check the file for changes).
//! Which entries carry one, and setting and clearing them.
//!
//! The list is read from the index through gitoxide, so that an index of
//! any size costs no process. It is decoded afresh each time: a flag that
//! git set a moment ago is there.
//!
//! Sources: git-update-index(1) (`--assume-unchanged`, `--skip-worktree`,
//! `--stdin`, `-z`), git-add(1) (`--intent-to-add`,
//! `--pathspec-from-file`), git-check-ignore(1), git-ls-files(1) (`-v`),
//! gitformat-index(5) (the flags of an entry), gix-index documentation.

use gix::index::entry::Flags as EntryFlags;
use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe, git_err, stage};

/// More flagged entries than this are counted and not listed: a sparse
/// checkout can have very many.
const MAX_LISTED: usize = 5000;
/// The most paths one change takes.
const MAX_PATHS: usize = 10_000;

/// A flag of an index entry.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Flag {
    /// The file will be added: it shows as new, with nothing staged.
    IntentToAdd,
    /// The file in the worktree is not git's business: it is not checked
    /// for changes and not written by a checkout.
    SkipWorktree,
    /// Git takes the file for unchanged without looking.
    AssumeUnchanged,
}

/// An index entry with at least one flag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Flagged {
    /// Path relative to the worktree.
    pub path: String,
    /// Marked as going to be added.
    pub intent_to_add: bool,
    /// Its worktree file is skipped.
    pub skip_worktree: bool,
    /// Assumed to be unchanged.
    pub assume_unchanged: bool,
}

/// How many entries carry each flag.
#[derive(Debug, Clone, Copy, Default, Serialize, PartialEq, Eq)]
pub struct Counts {
    /// Marked intent-to-add.
    pub intent_to_add: usize,
    /// Marked skip-worktree.
    pub skip_worktree: usize,
    /// Marked assume-unchanged.
    pub assume_unchanged: usize,
}

/// The flagged entries of the index.
#[derive(Debug, Clone, Default, Serialize, PartialEq, Eq)]
pub struct Flags {
    /// The entries, in index order; the first of them when there are very
    /// many.
    pub entries: Vec<Flagged>,
    /// How many there are of each kind, listed or not.
    pub counts: Counts,
    /// There are more flagged entries than are listed.
    pub truncated: bool,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("index-flags-what-flag"),
        detail: detail.into(),
    }
}

/// The index as it is on disk right now; none when there is no index yet.
fn read_index(repo: &Repository) -> Result<Option<gix::index::File>> {
    if !repo.gix().index_path().is_file() {
        return Ok(None);
    }
    repo.gix()
        .open_index()
        .map(Some)
        .map_err(git_err(say!("index-flags-reading-index")))
}

/// The entries that carry a flag.
pub fn list(repo: &Repository) -> Result<Flags> {
    let mut out = Flags::default();
    let Some(index) = read_index(repo)? else {
        return Ok(out);
    };
    let any = EntryFlags::INTENT_TO_ADD | EntryFlags::SKIP_WORKTREE | EntryFlags::ASSUME_VALID;
    for entry in index.entries() {
        // The versions of a file in conflict are not what a flag is about.
        if !entry.flags.intersects(any) || entry.stage_raw() != 0 {
            continue;
        }
        let flagged = Flagged {
            path: entry.path(&index).to_string(),
            intent_to_add: entry.flags.contains(EntryFlags::INTENT_TO_ADD),
            skip_worktree: entry.flags.contains(EntryFlags::SKIP_WORKTREE),
            assume_unchanged: entry.flags.contains(EntryFlags::ASSUME_VALID),
        };
        let counts = &mut out.counts;
        counts.intent_to_add = counts
            .intent_to_add
            .saturating_add(usize::from(flagged.intent_to_add));
        counts.skip_worktree = counts
            .skip_worktree
            .saturating_add(usize::from(flagged.skip_worktree));
        counts.assume_unchanged = counts
            .assume_unchanged
            .saturating_add(usize::from(flagged.assume_unchanged));
        if out.entries.len() < MAX_LISTED {
            out.entries.push(flagged);
        } else {
            out.truncated = true;
        }
    }
    Ok(out)
}

/// How a path stands in the index.
#[derive(PartialEq)]
enum Standing {
    Tracked,
    InConflict,
    Unknown,
}

fn standing(index: Option<&gix::index::File>, path: &str) -> Standing {
    match index.and_then(|index| index.entry_by_path(path.into())) {
        Some(entry) if entry.stage_raw() == 0 => Standing::Tracked,
        Some(_) => Standing::InConflict,
        None => Standing::Unknown,
    }
}

/// Paths one after the other, each ended by a NUL: what git reads from
/// standard input, whatever characters a name has.
fn nul_separated(paths: &[String]) -> Vec<u8> {
    let mut bytes = Vec::new();
    for path in paths {
        bytes.extend_from_slice(path.as_bytes());
        bytes.push(0);
    }
    bytes
}

/// Set or clear `flag` on `paths`. All of them or none: every path is
/// looked at before git is asked to change anything.
///
/// Skip-worktree and assume-unchanged are for files that are tracked.
/// Intent-to-add is for files that exist and are not tracked; it cannot be
/// cleared, since it ends when the file is staged or unstaged.
pub fn set(repo: &Repository, paths: &[String], flag: Flag, on: bool) -> Result<()> {
    if paths.is_empty() {
        return Err(invalid(
            say!("index-flags-what-paths"),
            say!("index-flags-no-paths"),
        ));
    }
    if paths.len() > MAX_PATHS {
        return Err(invalid(
            say!("index-flags-what-paths"),
            say!("index-flags-too-many", count = paths.len()),
        ));
    }
    if flag == Flag::IntentToAdd && !on {
        return Err(refused(say!("index-flags-intent-cannot-clear")));
    }
    let index = read_index(repo)?;
    for path in paths {
        let file = stage::worktree_path(repo, path)?;
        let standing = standing(index.as_ref(), path);
        if standing == Standing::InConflict {
            return Err(refused(say!(
                "index-flags-in-conflict",
                path = path.as_str()
            )));
        }
        match flag {
            Flag::SkipWorktree | Flag::AssumeUnchanged => {
                if standing != Standing::Tracked {
                    return Err(refused(say!(
                        "index-flags-not-tracked",
                        path = path.as_str()
                    )));
                }
            }
            Flag::IntentToAdd => {
                if standing == Standing::Tracked {
                    return Err(refused(say!("index-flags-tracked", path = path.as_str())));
                }
                // Git passes over its own directory without a word.
                if path
                    .split('/')
                    .any(|part| part.eq_ignore_ascii_case(".git"))
                {
                    return Err(refused(say!(
                        "index-flags-in-git-dir",
                        path = path.as_str()
                    )));
                }
                match file.symlink_metadata() {
                    Ok(meta) if meta.is_dir() => {
                        return Err(refused(say!("index-flags-folder", path = path.as_str())));
                    }
                    Ok(_) => {}
                    Err(_) => {
                        return Err(refused(say!("index-flags-missing", path = path.as_str())));
                    }
                }
            }
        }
    }
    let stdin = nul_separated(paths);
    match flag {
        Flag::IntentToAdd => {
            // Git adds what it can and then complains about an ignored
            // file: that is asked first, so that it is all or nothing.
            let (code, out) =
                exe::run_with_code(repo, ["check-ignore", "-z", "--stdin"], Some(&stdin))?;
            if code == Some(0)
                && let Some(ignored) = out.stdout.split('\0').find(|path| !path.is_empty())
            {
                return Err(refused(say!("index-flags-ignored", ignored)));
            }
            exe::run(
                repo,
                [
                    "--literal-pathspecs",
                    "add",
                    "--intent-to-add",
                    "--pathspec-from-file=-",
                    "--pathspec-file-nul",
                ],
                Some(&stdin),
            )?;
            // A path git has reasons of its own to pass over is not left
            // looking as if it had been marked.
            let index = read_index(repo)?;
            if let Some(left) = paths
                .iter()
                .find(|path| standing(index.as_ref(), path) != Standing::Tracked)
            {
                return Err(refused(say!("index-flags-not-taken", left = left.as_str())));
            }
        }
        Flag::SkipWorktree | Flag::AssumeUnchanged => {
            let option = match (flag, on) {
                (Flag::SkipWorktree, true) => "--skip-worktree",
                (Flag::SkipWorktree, false) => "--no-skip-worktree",
                (_, true) => "--assume-unchanged",
                (_, false) => "--no-assume-unchanged",
            };
            exe::run(
                repo,
                ["update-index", option, "-z", "--stdin"],
                Some(&stdin),
            )?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paths_go_to_git_ended_by_a_nul_each() {
        assert_eq!(
            nul_separated(&["a b.txt".to_owned(), "dir/new\nline".to_owned()]),
            b"a b.txt\0dir/new\nline\0"
        );
        assert!(nul_separated(&[]).is_empty());
    }

    #[test]
    fn a_flag_is_named_in_snake_case() {
        for (flag, name) in [
            (Flag::IntentToAdd, "\"intent_to_add\""),
            (Flag::SkipWorktree, "\"skip_worktree\""),
            (Flag::AssumeUnchanged, "\"assume_unchanged\""),
        ] {
            assert_eq!(serde_json::to_string(&flag).unwrap(), name);
            assert_eq!(serde_json::from_str::<Flag>(name).unwrap(), flag);
        }
        assert!(serde_json::from_str::<Flag>("\"--force\"").is_err());
    }
}
