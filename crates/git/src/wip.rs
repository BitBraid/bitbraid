//! Uncommitted work in the other worktrees, for the rows that stand for it
//! in the graph.
//!
//! Finding it costs a `git status` in each worktree, so a worktree is not
//! looked at again while nothing says it changed. [`scan`] is told what the
//! caller already knows as a stamp per worktree: the commit it has checked
//! out and the size and time of its index. A worktree whose stamp is still
//! the same is passed over; the others are asked, one after the other. An
//! edit to a file that touches neither HEAD nor the index leaves the stamp
//! as it was: the caller decides when to ask afresh, which it does by
//! saying that it knows nothing.
//!
//! The caller also says how far one scan may go ([`Scope`]): which
//! worktrees are not to be looked at this time (looked at a moment ago),
//! and how many may be. A worktree that changed but is left for later is
//! said to be waiting, so that the caller asks again when it may.
//!
//! The look is the cheap one ([`LOOK`]): what is changed in the files git
//! tracks, staged or not, without the search for untracked files. That
//! search reads every folder of the working tree and matches what it finds
//! against the ignore rules, which in a large tree costs as much as the
//! rest or more; untracked files of other worktrees are not counted.
//!
//! [`with_rows`] puts a row for each such worktree into a walked history,
//! directly above the commit the worktree has checked out and as a child
//! of it, so that the layout draws it like the tip of a branch. The row's
//! id ([`row_id`]) is made from the worktree's path and names no object.
//!
//! Sources: git-worktree(1), git-status(1) (the short format and
//! `--untracked-files`), gitrepository-layout(5) (the `gitdir:` file of a
//! linked worktree and its `index`).

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
};

use serde::{Deserialize, Serialize};

use crate::{GraphCommit, Oid, Repository, Result, exe, worktree};

/// At most this many worktrees are looked at; a repository with more has
/// rows for the first ones git lists.
pub const MAX_SCANNED: usize = 64;

/// The changed files of a worktree by what happened to them. A file counts
/// once: what is staged says it when something is.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Counts {
    /// Changed in place (or unmerged, or of another type).
    #[serde(default)]
    pub modified: usize,
    /// New, tracked or not.
    #[serde(default)]
    pub added: usize,
    /// Removed.
    #[serde(default)]
    pub deleted: usize,
    /// Moved or copied.
    #[serde(default)]
    pub renamed: usize,
}

impl Counts {
    /// How many files there are in all.
    pub fn total(&self) -> usize {
        self.modified
            .saturating_add(self.added)
            .saturating_add(self.deleted)
            .saturating_add(self.renamed)
    }
}

/// One worktree other than the open one, and what is uncommitted in it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorktreeChanges {
    /// Its working directory.
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// The commit it has checked out, absent on a branch without commits.
    pub head: Option<Oid>,
    /// The branch it is on, by its short name.
    pub branch: Option<String>,
    /// What it is told apart by the next time: see [`stamp`].
    pub stamp: String,
    /// Its changed files by kind. Absent when the stamp is the one the
    /// caller knew (nothing was looked at), when it is waiting, or when
    /// git could not say.
    pub counts: Option<Counts>,
    /// Whether it was looked at.
    pub scanned: bool,
    /// Its stamp is not the one the caller knew, but it was not looked at:
    /// the caller said not now, or as many as it allowed were looked at
    /// before it. The caller keeps what it had and asks again.
    pub waiting: bool,
}

/// How far one [`scan`] may go.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Scope {
    /// Worktrees not to be looked at this time, by path, whatever their
    /// stamp: one that changed is said to be waiting.
    pub skip: HashSet<String>,
    /// How many may be looked at, at most; no limit when absent.
    pub at_most: Option<usize>,
}

/// What git is asked in a worktree that is looked at: what is changed in
/// its tracked files, staged or not, in the short format, without the
/// search for untracked files.
pub const LOOK: [&str; 3] = ["status", "--porcelain", "--untracked-files=no"];

/// Count the entries of `git status --porcelain` (version 1, one entry a
/// line) by kind. A line is two letters, a space and a path; the first
/// letter is what is staged, the second what is not.
pub fn count_status(porcelain: &str) -> Counts {
    let mut counts = Counts::default();
    for line in porcelain.lines() {
        let mut letters = line.chars();
        let (Some(staged), Some(unstaged)) = (letters.next(), letters.next()) else {
            continue;
        };
        // Not an entry: the branch line, a line of another format, noise.
        let known = |letter: char| " MTADRCU?!".contains(letter);
        if letters.next() != Some(' ') || !known(staged) || !known(unstaged) {
            continue;
        }
        // What is staged says it, as on the row of the open worktree.
        let letter = if staged != ' ' && staged != '?' {
            staged
        } else {
            unstaged
        };
        let slot = match letter {
            '?' | 'A' => &mut counts.added,
            'D' => &mut counts.deleted,
            'R' | 'C' => &mut counts.renamed,
            '!' | ' ' => continue, // ignored files are not changes
            _ => &mut counts.modified,
        };
        *slot = slot.saturating_add(1);
    }
    counts
}

/// The directory git keeps a worktree's own files in: `.git` itself for
/// the main worktree, what the `gitdir:` line of the `.git` file names for
/// a linked one.
fn git_dir_of(worktree: &Path) -> Option<PathBuf> {
    let dot_git = worktree.join(".git");
    let meta = std::fs::symlink_metadata(&dot_git).ok()?;
    if meta.is_dir() {
        return Some(dot_git);
    }
    // A pointer file is a line; one that is longer than a path can be is
    // not read.
    if meta.len() > 16 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(&dot_git).ok()?;
    let target = text.lines().next()?.strip_prefix("gitdir:")?.trim();
    if target.is_empty() {
        return None;
    }
    let target = Path::new(target);
    Some(if target.is_absolute() {
        target.to_path_buf()
    } else {
        worktree.join(target)
    })
}

/// What a worktree is told apart by between two looks at it: the commit it
/// has checked out, and the size and time of its index. Staging, a commit,
/// a checkout, a reset and a stash all change one of them. Cheap: one small
/// file read and one `stat`. It ends with what kind of look it goes with,
/// so that what a look that also counted untracked files found is not
/// taken for what this one would find.
pub fn stamp(worktree: &Path, head: Option<Oid>) -> String {
    let index = git_dir_of(worktree)
        .and_then(|dir| std::fs::metadata(dir.join("index")).ok())
        .map(|meta| {
            let modified = meta
                .modified()
                .ok()
                .and_then(|at| at.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_nanos())
                .unwrap_or(0);
            format!("{}.{modified}", meta.len())
        })
        .unwrap_or_else(|| "-".to_owned());
    let head = head.map(|id| id.to_hex()).unwrap_or_else(|| "-".to_owned());
    format!("{head} {index} tracked")
}

/// What is uncommitted in each worktree other than the open one. `known`
/// maps a worktree's path to the stamp it had when it was last looked at:
/// one that still has it is listed without counts and without git being
/// started for it. Of the others, those `scope` leaves out are listed as
/// waiting, also without git. Stops with an error when `cancel` is set.
pub fn scan(
    repo: &Repository,
    known: &HashMap<String, String>,
    scope: &Scope,
    cancel: &Arc<AtomicBool>,
) -> Result<Vec<WorktreeChanges>> {
    let mut out = Vec::new();
    let listed = worktree::list(repo, false)?;
    let others = listed
        .into_iter()
        .filter(|info| !info.current && !info.bare && !info.prunable)
        .take(MAX_SCANNED);
    let mut left = scope.at_most.unwrap_or(usize::MAX);
    for info in others {
        let stamp = stamp(&info.path, info.head);
        let path = info.path.to_string_lossy().into_owned();
        let same = known.get(&path).is_some_and(|was| *was == stamp);
        let waiting = !same && (left == 0 || scope.skip.contains(&path));
        let looked = !same && !waiting;
        let counts = if !looked || !info.path.is_dir() {
            None
        } else {
            left = left.saturating_sub(1);
            match exe::run_streaming_in(&info.path, LOOK, cancel, &mut |_| {}) {
                Ok(said) => Some(count_status(&said.stdout)),
                Err(err) if cancel.load(std::sync::atomic::Ordering::SeqCst) => return Err(err),
                // A worktree git cannot read is listed without counts.
                Err(_) => None,
            }
        };
        out.push(WorktreeChanges {
            path: info.path,
            head: info.head,
            branch: info.branch,
            stamp,
            scanned: looked,
            waiting,
            counts,
        });
    }
    Ok(out)
}

/// The id of the row that stands for the uncommitted work of the worktree
/// at `path`. It names no object: it starts with eight zeros, which no
/// commit a repository holds is expected to, and the rest is made from the
/// path, so that the row is the same row from one graph to the next.
pub fn row_id(path: &Path) -> Oid {
    let mut first = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut first);
    let first = first.finish();
    let mut second = std::collections::hash_map::DefaultHasher::new();
    (first, "bitbraid worktree row").hash(&mut second);
    let second = second.finish();
    format!("00000000{first:016x}{second:016x}")
        .parse()
        .unwrap_or_else(|_| Oid::zero())
}

/// Add a row for the uncommitted work of other worktrees to a walked
/// history. `rows` holds, per worktree, the id of its row and the commit it
/// has checked out. A row goes directly above that commit and has it for
/// its only parent; a worktree whose commit is not among `commits` (it is
/// hidden, or left out while only some references are shown) gets none,
/// and of two entries with the same row id the first counts. The order of
/// rows above one commit is the order given.
pub fn with_rows(commits: Vec<GraphCommit>, rows: &[(Oid, Oid)]) -> Vec<GraphCommit> {
    if rows.is_empty() {
        return commits;
    }
    let mut above: HashMap<Oid, Vec<Oid>> = HashMap::new();
    let mut seen: Vec<Oid> = Vec::new();
    for (id, head) in rows.iter().take(MAX_SCANNED) {
        if seen.contains(id) {
            continue;
        }
        seen.push(*id);
        above.entry(*head).or_default().push(*id);
    }
    let mut out = Vec::with_capacity(commits.len().saturating_add(seen.len()));
    for commit in commits {
        // A commit with the id of a row would be a row twice: the commit
        // stands and the row is left out.
        if seen.contains(&commit.id) {
            above.retain(|_, ids| {
                ids.retain(|id| *id != commit.id);
                !ids.is_empty()
            });
        }
        if let Some(ids) = above.remove(&commit.id) {
            for id in ids {
                out.push(GraphCommit {
                    id,
                    parents: vec![commit.id],
                    seconds: None,
                });
            }
        }
        out.push(commit);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oid(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    fn commit(id: u8, parents: &[u8]) -> GraphCommit {
        GraphCommit {
            id: oid(id),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            seconds: Some(i64::from(id)),
        }
    }

    #[test]
    fn status_lines_are_counted_by_kind_once_per_file() {
        let text = " M changed.txt\nM  staged.txt\nMM both.txt\nA  new.txt\n?? untracked.txt\n\
                    ?? dir/\n D gone.txt\nD  removed.txt\nR  old.txt -> new-name.txt\n\
                    C  a.txt -> b.txt\nUU conflict.txt\nT  type.txt\nAM added-then-edited.txt\n\
                    !! ignored.log\n";
        assert_eq!(
            count_status(text),
            Counts {
                modified: 5,
                added: 4,
                deleted: 2,
                renamed: 2,
            }
        );
        assert_eq!(count_status(text).total(), 13);
    }

    #[test]
    fn what_is_no_status_line_is_not_counted() {
        for noise in [
            "",
            "\n\n",
            "M",
            "MM",
            "## main...origin/main",
            "1 .M N... 100644 100644 100644 abc def file",
            "\u{1F600}\u{1F600}\u{1F600}",
            "\0\0\0",
        ] {
            assert_eq!(count_status(noise), Counts::default(), "{noise:?}");
        }
        // A path may hold anything after the two letters and the space.
        assert_eq!(count_status(" M  \u{1F600} -> x\n").modified, 1);
        assert_eq!(Counts::default().total(), 0);
        let full = Counts {
            modified: usize::MAX,
            added: 1,
            deleted: 1,
            renamed: 1,
        };
        assert_eq!(full.total(), usize::MAX);
    }

    #[test]
    fn a_row_goes_right_above_the_commit_its_worktree_has() {
        let history = || {
            vec![
                commit(4, &[3]),
                commit(3, &[2]),
                commit(2, &[1]),
                commit(1, &[]),
            ]
        };
        let (a, b, c) = (oid(0xa0), oid(0xb0), oid(0xc0));
        let ids = |commits: &[GraphCommit]| -> Vec<Oid> { commits.iter().map(|c| c.id).collect() };
        // Nothing to add: the same vector back.
        assert_eq!(with_rows(history(), &[]), history());
        // Two worktrees on one commit, one on another, one on a commit that
        // is not in the graph, and one row given twice.
        let rows = [
            (a, oid(2)),
            (b, oid(4)),
            (c, oid(2)),
            (oid(0xd0), oid(9)),
            (a, oid(1)),
        ];
        let out = with_rows(history(), &rows);
        assert_eq!(ids(&out), [b, oid(4), oid(3), a, c, oid(2), oid(1)]);
        for row in out.iter().filter(|commit| [a, b, c].contains(&commit.id)) {
            assert_eq!(row.parents.len(), 1);
            assert_eq!(row.seconds, None);
        }
        assert_eq!(out[3].parents, [oid(2)]);
        // A commit before its parents still.
        let mut seen = Vec::new();
        for commit in &out {
            assert!(commit.parents.iter().all(|parent| !seen.contains(parent)));
            seen.push(commit.id);
        }
    }

    #[test]
    fn a_row_never_stands_in_for_a_commit() {
        // A row id that is a commit of the history: the commit stays what
        // it is and no row is added for it.
        let history = vec![commit(2, &[1]), commit(1, &[])];
        let out = with_rows(history.clone(), &[(oid(2), oid(1)), (oid(1), oid(1))]);
        assert_eq!(out, history);
        // More rows than are ever taken.
        let many: Vec<(Oid, Oid)> = (0..200u8)
            .map(|n| (oid(n.wrapping_add(10)), oid(1)))
            .collect();
        let out = with_rows(history, &many);
        assert_eq!(out.len(), 2 + MAX_SCANNED);
        assert!(with_rows(Vec::new(), &many).is_empty());
    }

    #[test]
    fn row_ids_are_stable_distinct_and_no_commit() {
        let a = row_id(Path::new("/work/a"));
        assert_eq!(a, row_id(Path::new("/work/a")));
        assert_ne!(a, row_id(Path::new("/work/b")));
        assert_ne!(a, Oid::zero());
        assert!(a.to_hex().starts_with("00000000"));
        assert_ne!(row_id(Path::new("")), Oid::zero());
    }

    #[test]
    fn a_stamp_tells_a_changed_index_and_another_head_apart() {
        let dir = std::env::temp_dir().join(format!("bitbraid-wip-stamp-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let git_dir = dir.join("elsewhere/worktrees/w");
        std::fs::create_dir_all(&git_dir).unwrap();
        let tree = dir.join("tree");
        std::fs::create_dir_all(&tree).unwrap();
        // No `.git` at all: still a stamp, by the head alone.
        let bare = stamp(&tree, Some(oid(1)));
        assert_ne!(bare, stamp(&tree, Some(oid(2))));
        assert_ne!(bare, stamp(&tree, None));
        // A linked worktree: `.git` is a file that says where its files are.
        std::fs::write(
            tree.join(".git"),
            format!("gitdir: {}\n", git_dir.display()),
        )
        .unwrap();
        std::fs::write(git_dir.join("index"), b"one").unwrap();
        let first = stamp(&tree, Some(oid(1)));
        assert_ne!(first, bare);
        // Not the stamp of a look that also counted untracked files.
        assert!(first.ends_with(" tracked"), "{first}");
        assert_eq!(first, stamp(&tree, Some(oid(1))));
        std::fs::write(git_dir.join("index"), b"longer than before").unwrap();
        assert_ne!(first, stamp(&tree, Some(oid(1))));
        // A main worktree: `.git` is the directory.
        let main = dir.join("main");
        std::fs::create_dir_all(main.join(".git")).unwrap();
        std::fs::write(main.join(".git/index"), b"x").unwrap();
        assert!(stamp(&main, None).starts_with("- 1."));
        // A pointer that points nowhere, or is no pointer, is no trouble.
        std::fs::write(tree.join(".git"), "gitdir:\n").unwrap();
        assert_eq!(stamp(&tree, Some(oid(1))), bare);
        std::fs::write(tree.join(".git"), [0xff, 0xfe, 0x00]).unwrap();
        assert_eq!(stamp(&tree, Some(oid(1))), bare);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
