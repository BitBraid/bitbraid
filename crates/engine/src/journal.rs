//! The undo journal: what each mutation did and how to reverse it. Stored
//! under `.git/bitbraid/journal.json` so it survives restarts and never
//! touches tracked files.
//!
//! What an entry needs to be undone or redone is often an object that no
//! reference of the user's names any more: the content of a file that was
//! discarded, a stash that was dropped, the tip of a deleted branch. Git
//! prunes such objects, so every time the journal is written the objects
//! its entries need are held by references of our own
//! (`bitbraid_git::hold`), and let go when the entry leaves the journal
//! ([`Change::holds`], [`Journal::save`]).
//!
//! Sources: git-update-ref(1) (compare-and-swap semantics); git-gc(1)
//! (what is kept: what a reference or a reflog reaches).

use std::{fs, path::PathBuf};

use bitbraid_git::{
    Backup, Dirty, HeadState, Oid, Repository, ResetMode, StashOptions, remote::DroppedRef,
    settings::Setting,
};
use serde::{Deserialize, Serialize};

/// Bound on remembered entries.
pub const CAPACITY: usize = 100;

/// What an entry reverses.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Change {
    /// A commit was created (or amended): the tip moved from `previous` to `id`.
    Commit {
        /// Branch that moved, absent when detached.
        branch: Option<String>,
        /// Tip before.
        previous: Option<Oid>,
        /// Tip after.
        id: Oid,
        /// It was an amend.
        amend: bool,
    },
    /// Worktree content was discarded; backups restore it.
    Discard {
        /// One backup per path.
        backups: Vec<Backup>,
    },
    /// A worktree file was put back to what it was in a commit.
    Restore {
        /// What the file held before.
        backup: Backup,
        /// The commit its content was taken from.
        rev: Oid,
    },
    /// HEAD went to another branch or commit.
    Checkout {
        /// What was asked for, to do it again.
        target: String,
        /// How local changes were treated.
        dirty: Dirty,
        /// HEAD before.
        before: HeadState,
        /// HEAD after.
        after: HeadState,
        /// A local branch created to track a remote one.
        created: Option<String>,
        /// Backups of changes that were thrown away.
        backups: Vec<Backup>,
    },
    /// A branch was created, and checked out when `switched_from` is set.
    BranchCreate {
        /// Short name.
        name: String,
        /// Where it points.
        target: Oid,
        /// HEAD before the branch was checked out.
        switched_from: Option<HeadState>,
    },
    /// A branch was deleted.
    BranchDelete {
        /// Short name.
        name: String,
        /// Where it pointed.
        target: Oid,
        /// Its upstream as `remote/branch`.
        upstream: Option<String>,
    },
    /// A branch was renamed.
    BranchRename {
        /// Name before.
        name: String,
        /// Name after.
        new_name: String,
    },
    /// The current branch moved: a merge, rebase, reset, cherry-pick or
    /// revert.
    HeadMoved {
        /// HEAD before.
        before: HeadState,
        /// HEAD after.
        after: HeadState,
        /// How the files moved: the reset's own mode for a reset, keep for
        /// everything else (a merge, rebase, pick, revert or pull).
        mode: ResetMode,
        /// Backups of files a hard reset overwrote.
        backups: Vec<Backup>,
    },
    /// The upstream of a branch changed.
    Upstream {
        /// Short branch name.
        name: String,
        /// Upstream before as `remote/branch`.
        before: Option<String>,
        /// Upstream after.
        after: Option<String>,
    },
    /// Local changes were stashed.
    StashPush {
        /// The stash that was made.
        stash: Oid,
        /// How, to do it again.
        options: StashOptions,
    },
    /// A stash was dropped from the list.
    StashDrop {
        /// The stash commit, which stays in the object database.
        stash: Oid,
        /// Its message in the list.
        message: String,
    },
    /// A stash was given another message: its commit was made anew and
    /// put in its place in the list.
    StashReword {
        /// The stash before, which stays in the object database.
        before: Oid,
        /// What the list said of it.
        before_message: String,
        /// The stash it is now.
        after: Oid,
        /// What the list says of it now.
        after_message: String,
    },
    /// A stash was applied, or popped: its changes were brought back.
    StashApply {
        /// What it changed, file by file, and what it takes to put a
        /// popped one back in the list.
        applied: bitbraid_git::stash::Applied,
    },
    /// A worktree file was edited in the application.
    FileWrite {
        /// The file before.
        before: Backup,
        /// The file after.
        after: Backup,
    },
    /// A branch that is not checked out moved (a fast-forward to its
    /// upstream).
    BranchMove {
        /// Short name.
        name: String,
        /// Where it pointed.
        before: Oid,
        /// Where it points.
        after: Oid,
    },
    /// A tag was created.
    TagCreate {
        /// Short name.
        name: String,
        /// What the reference holds: a tag object or a commit.
        value: Oid,
    },
    /// A lightweight tag was given a message: made anew as an annotated
    /// tag at the same commit.
    TagAnnotate {
        /// Short name.
        name: String,
        /// What the reference held: the commit.
        before: Oid,
        /// What it holds: the tag object.
        after: Oid,
    },
    /// A tag was deleted.
    TagDelete {
        /// Short name.
        name: String,
        /// What the reference held.
        value: Oid,
    },
    /// A patch was applied to the files, and to the index when `index` is
    /// set.
    PatchApply {
        /// The patch, kept as a blob in the object database.
        patch: Oid,
        /// The index was changed along with the files.
        index: bool,
        /// It was applied the other way round.
        reverse: bool,
    },
    /// Changes were merged into the files and the index and HEAD stayed
    /// where it was: a cherry-pick without committing, a squash merge, a
    /// patch applied with a three-way merge that left files in conflict.
    FilesMerged {
        /// What it did to them, and what git prepared for the commit.
        files: bitbraid_git::files_change::FilesChange,
    },
    /// Settings of the repository's own configuration were changed: the
    /// signing settings, an identity, gitflow's names, the description of
    /// a branch, a remote that was added or pointed elsewhere.
    Config {
        /// Each setting that changed, with all its values before and
        /// after.
        settings: Vec<Setting>,
        /// A branch that was created along with them (gitflow's develop
        /// branch): its short name and where it points.
        #[serde(default)]
        branch: Option<(String, Oid)>,
    },
    /// A remote was removed.
    RemoteRemove {
        name: String,
        /// The settings that went with it: its own, and the upstream of
        /// the branches that followed it.
        settings: Vec<Setting>,
        /// Its remote-tracking references.
        references: Vec<DroppedRef>,
    },
    /// A signer was put into the allowed-signers file or taken out of it.
    Signers {
        /// The lines that came and went.
        edit: bitbraid_git::signers::Edit,
    },
    /// A branch was deleted on a remote.
    RemoteBranchDelete {
        remote: String,
        /// The branch there.
        branch: String,
        /// The commit it was at.
        target: Oid,
    },
}

impl Change {
    /// The objects this change needs that nothing else may be holding:
    /// for its undo while it can be undone, for its redo once it has
    /// been (`undone`). Blobs made as backups, and commits and tag
    /// objects whose reference the change took away. They are held for
    /// as long as the entry is in the journal on that side.
    ///
    /// What is done again makes its backups again, so an undone discard
    /// holds nothing; a tag that was deleted holds its object until the
    /// deletion is undone, and then the tag holds it; a tag that was
    /// created holds nothing until the creation is undone.
    ///
    /// Commits that only HEAD or a branch moved away from (a commit, an
    /// amend, a reset, a merge, a rebase, a checkout) are not among them:
    /// the reflog holds those, and a reference of ours would make them
    /// show in every `git log --all`.
    pub fn holds(&self, undone: bool) -> Vec<Oid> {
        let blobs = |backups: &[Backup]| -> Vec<Oid> {
            backups.iter().filter_map(|backup| backup.blob).collect()
        };
        match (self, undone) {
            (
                Self::Discard { backups }
                | Self::Checkout { backups, .. }
                | Self::HeadMoved { backups, .. },
                false,
            ) => blobs(backups),
            (Self::Restore { backup, .. }, false) => blobs(std::slice::from_ref(backup)),
            // Either way one of the two is written back.
            (Self::FileWrite { before, after }, _) => blobs(&[before.clone(), after.clone()]),
            (Self::PatchApply { patch, .. }, _) => vec![*patch],
            // A dropped stash has left the stash's reflog, a deleted
            // branch or tag its own.
            (Self::StashDrop { stash, .. }, false) => vec![*stash],
            // The stash that is not in the list on this side.
            (Self::StashReword { before, .. }, false) => vec![*before],
            (Self::StashReword { after, .. }, true) => vec![*after],
            // An annotation taken back leaves its tag object behind.
            (Self::TagAnnotate { after, .. }, true) => vec![*after],
            // What the files held before, to go back to; and a stash that
            // was popped has left the list. Undone, the stash is in the
            // list again and redo applies it afresh.
            (Self::StashApply { applied }, false) => {
                let mut held = blobs(&applied.files_before);
                if applied.popped {
                    held.push(applied.stash);
                }
                held
            }
            // What is put back: the files and the index entries (blobs of
            // a conflict's stages included) of the side that is not there.
            (Self::FilesMerged { files }, undone) => {
                let (backups, staged) = if undone {
                    (&files.files_after, &files.index_after)
                } else {
                    (&files.files_before, &files.index_before)
                };
                let mut held = blobs(backups);
                held.extend(staged.iter().map(|entry| entry.blob));
                held
            }
            (Self::BranchDelete { target, .. }, false) => vec![*target],
            (Self::TagDelete { value, .. }, false) | (Self::TagCreate { value, .. }, true) => {
                vec![*value]
            }
            (Self::RemoteRemove { references, .. }, false) => references
                .iter()
                .map(|reference| reference.object)
                .collect(),
            // The remote-tracking branch went with the branch, and nothing
            // else here may have the commit.
            (Self::RemoteBranchDelete { target, .. }, false) => vec![*target],
            (
                Self::Discard { .. }
                | Self::Checkout { .. }
                | Self::HeadMoved { .. }
                | Self::Restore { .. }
                | Self::StashDrop { .. }
                | Self::StashApply { .. }
                | Self::BranchDelete { .. }
                | Self::TagDelete { .. }
                | Self::TagCreate { .. }
                | Self::TagAnnotate { .. }
                | Self::RemoteRemove { .. }
                | Self::RemoteBranchDelete { .. }
                | Self::Commit { .. }
                | Self::BranchCreate { .. }
                | Self::BranchRename { .. }
                | Self::Upstream { .. }
                | Self::StashPush { .. }
                | Self::BranchMove { .. }
                | Self::Config { .. }
                | Self::Signers { .. },
                _,
            ) => Vec::new(),
        }
    }
}

impl Change {
    /// Whether undoing or redoing this talks to a remote, which can take
    /// as long as a push does: a caller that minds runs it where it does
    /// not hold up everything else.
    pub fn talks_to_remote(&self) -> bool {
        matches!(self, Self::RemoteBranchDelete { .. })
    }
}

/// One journal entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// Monotonic id.
    pub id: u64,
    /// Human label, for example "Commit 3f2a1c0: fix lanes".
    pub label: String,
    /// Unix time.
    pub time: i64,
    pub change: Change,
}

/// The journal with an undo stack and a redo stack.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Journal {
    next_id: u64,
    /// Entries that can be undone, oldest first.
    pub done: Vec<Entry>,
    /// Entries that were undone and can be redone, oldest first.
    pub undone: Vec<Entry>,
}

fn now() -> i64 {
    bitbraid_core::clock::unix_seconds()
}

impl Journal {
    fn path(repo: &Repository) -> PathBuf {
        repo.git_dir().join("bitbraid").join("journal.json")
    }

    /// Load the repository's journal, empty when absent or unreadable.
    pub fn load(repo: &Repository) -> Self {
        fs::read_to_string(Self::path(repo))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Persist atomically. Errors are swallowed: a journal that cannot be
    /// written must not block the operation it describes.
    pub fn save(&self, repo: &Repository) {
        let path = Self::path(repo);
        let Some(parent) = path.parent() else { return };
        if fs::create_dir_all(parent).is_err() {
            return;
        }
        let Ok(text) = serde_json::to_string_pretty(self) else {
            return;
        };
        let tmp = path.with_extension("json.tmp");
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &path);
        }
        // What the entries need is held, and what entries that are gone
        // needed is let go. Like the file: when it cannot be done, the
        // operation is not held up for it.
        let _ = bitbraid_git::hold::reconcile(repo, &self.held());
    }

    /// The objects the entries need, each with the entry that needs it.
    pub fn held(&self) -> Vec<(u64, Oid)> {
        let of = |entries: &[Entry], undone: bool| -> Vec<(u64, Oid)> {
            entries
                .iter()
                .flat_map(|entry| {
                    entry
                        .change
                        .holds(undone)
                        .into_iter()
                        .map(move |object| (entry.id, object))
                })
                .collect()
        };
        let mut held = of(&self.done, false);
        held.extend(of(&self.undone, true));
        held
    }

    /// Record a change; clears the redo stack.
    pub fn record(&mut self, label: impl Into<String>, change: Change) -> &Entry {
        self.next_id = self.next_id.wrapping_add(1);
        self.undone.clear();
        self.done.push(Entry {
            id: self.next_id,
            label: label.into(),
            time: now(),
            change,
        });
        if self.done.len() > CAPACITY {
            let excess = self.done.len() - CAPACITY;
            self.done.drain(..excess);
        }
        self.done.last().unwrap_or_else(|| unreachable_entry())
    }

    /// The entry that `undo` would reverse.
    pub fn peek_undo(&self) -> Option<&Entry> {
        self.done.last()
    }

    /// The entry that `redo` would replay.
    pub fn peek_redo(&self) -> Option<&Entry> {
        self.undone.last()
    }

    /// Move the last done entry to the redo stack.
    pub fn pop_undo(&mut self) -> Option<Entry> {
        let entry = self.done.pop()?;
        self.undone.push(entry.clone());
        Some(entry)
    }

    /// Move the last undone entry back to the done stack.
    pub fn pop_redo(&mut self) -> Option<Entry> {
        let entry = self.undone.pop()?;
        self.done.push(entry.clone());
        Some(entry)
    }
}

// `record` always pushes before reading `last`; this keeps the never-panic
// rule honest without an unwrap.
fn unreachable_entry() -> &'static Entry {
    static EMPTY: std::sync::OnceLock<Entry> = std::sync::OnceLock::new();
    EMPTY.get_or_init(|| Entry {
        id: 0,
        label: String::new(),
        time: 0,
        change: Change::Discard {
            backups: Vec::new(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_undo_redo_flow() {
        let mut j = Journal::default();
        j.record("one", Change::Discard { backups: vec![] });
        j.record("two", Change::Discard { backups: vec![] });
        assert_eq!(j.peek_undo().map(|e| e.label.as_str()), Some("two"));
        let e = j.pop_undo().unwrap();
        assert_eq!(e.label, "two");
        assert_eq!(j.peek_redo().map(|e| e.label.as_str()), Some("two"));
        j.pop_redo();
        assert_eq!(j.done.len(), 2);
        assert!(j.undone.is_empty());
        // A new record clears the redo stack.
        j.pop_undo();
        j.record("three", Change::Discard { backups: vec![] });
        assert!(j.undone.is_empty());
        assert_eq!(j.done.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 3]);
    }

    #[test]
    fn capacity_is_bounded() {
        let mut j = Journal::default();
        for i in 0..(CAPACITY + 10) {
            j.record(format!("{i}"), Change::Discard { backups: vec![] });
        }
        assert_eq!(j.done.len(), CAPACITY);
        assert_eq!(j.done.first().map(|e| e.label.as_str()), Some("10"));
    }

    #[test]
    fn round_trips_through_json() {
        let mut j = Journal::default();
        j.record(
            "commit",
            Change::Commit {
                branch: Some("main".into()),
                previous: None,
                id: Oid::zero(),
                amend: false,
            },
        );
        let text = serde_json::to_string(&j).unwrap();
        let back: Journal = serde_json::from_str(&text).unwrap();
        assert_eq!(back, j);
    }
}
