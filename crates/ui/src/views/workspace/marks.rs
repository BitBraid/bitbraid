//! Marks on the local branches that would conflict with the one that is
//! checked out, worked out ahead of any merge.
//!
//! Bounded so that it slows nothing: local branches only, at most
//! [`MARKED`] of them (the first by name), only while they are listed
//! open, one trial merge at a time on a thread of its own with a pause
//! between two, and each pair of commits once. The trial merges write
//! nothing into the repository (`branch.predict {aside}`), so the watcher
//! is not woken by them. Nor do they fetch anything: nobody asked for
//! them, so they stay local (`crate::session`), and in a partial clone a
//! merge that needs what the clone left on its remote answers that it
//! cannot tell, which is no mark.
//!
//! Sources: git-merge-tree(1); git's documentation of partial clone
//! (`partial-clone.adoc`).

use std::collections::{HashMap, VecDeque};

use bitbraid_git::Oid;
use gpui_kit::*;
use serde_json::json;

use super::Workspace;
use crate::session::{Reply, Request};

/// Most local branches that are tried against the checked-out one.
pub(super) const MARKED: usize = 50;

/// The pause between two trial merges.
const GAP: std::time::Duration = std::time::Duration::from_millis(120);

/// What is known and what is still to ask.
#[derive(Debug, Default)]
pub(super) struct ConflictMarks {
    /// The commit that is checked out, which every answer is about.
    head: Option<Oid>,
    /// By branch: the tip that was tried and how many files conflicted.
    known: HashMap<String, (Oid, usize)>,
    /// Branches still to try, with their tips.
    queue: VecDeque<(String, Oid)>,
    /// A trial merge is on its way.
    running: bool,
}

impl ConflictMarks {
    /// Whether trial merges are still to come or on their way.
    pub(super) fn under_way(&self) -> bool {
        self.running || !self.queue.is_empty()
    }

    /// How many branches have been answered for at the tips they are at.
    pub(super) fn answered(&self) -> usize {
        self.known.len()
    }
}

/// The branches to try, in the order given: those that are not checked
/// out, are not at the checked-out commit and have not been tried at the
/// tip they are at; no more than `limit` branches are looked at.
pub(super) fn to_try(
    head: Oid,
    branches: &[(String, Oid, bool)],
    known: &HashMap<String, (Oid, usize)>,
    limit: usize,
) -> VecDeque<(String, Oid)> {
    branches
        .iter()
        .take(limit)
        .filter(|(name, tip, is_head)| {
            !is_head && *tip != head && known.get(name).is_none_or(|(tried, _)| tried != tip)
        })
        .map(|(name, tip, _)| (name.clone(), *tip))
        .collect()
}

impl Workspace {
    /// The references are in, or the local branches came into view: see
    /// what is to try, and begin when nothing is on its way.
    pub(super) fn mark_conflicts(&mut self, cx: &mut Context<Self>) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let head = snapshot.head.target.filter(|_| !self.read_only());
        let open = self.refs_view.read(cx).local_open();
        let (Some(head), true) = (head, open) else {
            // Nothing is tried while nobody looks; what is known stays
            // for when they do, as long as HEAD is where it was.
            self.conflict_marks.queue.clear();
            return;
        };
        if self.conflict_marks.head != Some(head) {
            self.conflict_marks.known.clear();
            self.conflict_marks.head = Some(head);
        }
        let branches: Vec<(String, Oid, bool)> = snapshot
            .branches
            .iter()
            .map(|branch| (branch.name.clone(), branch.target, branch.is_head))
            .collect();
        // What is no longer a branch, or has moved, is no longer known.
        self.conflict_marks.known.retain(|name, (tip, _)| {
            branches
                .iter()
                .any(|(branch, at, _)| branch == name && at == tip)
        });
        self.conflict_marks.queue = to_try(head, &branches, &self.conflict_marks.known, MARKED);
        self.show_conflict_marks(cx);
        self.try_next_conflict(cx);
    }

    fn show_conflict_marks(&mut self, cx: &mut Context<Self>) {
        let marks: HashMap<String, usize> = self
            .conflict_marks
            .known
            .iter()
            .filter(|(_, (_, files))| *files > 0)
            .map(|(name, (_, files))| (name.clone(), *files))
            .collect();
        self.refs_view
            .update(cx, |view, cx| view.set_conflicts(marks, cx));
    }

    fn try_next_conflict(&mut self, cx: &mut Context<Self>) {
        if self.conflict_marks.running {
            return;
        }
        let Some((name, tip)) = self.conflict_marks.queue.pop_front() else {
            return;
        };
        let Some(head) = self.conflict_marks.head else {
            return;
        };
        self.conflict_marks.running = true;
        let reply = self.session.send(Request::Command {
            id: "branch.predict".to_owned(),
            params: json!({ "target": tip.to_hex(), "aside": true }),
        });
        cx.spawn(async move |this, cx| {
            let reply = reply.await;
            cx.background_executor().timer(GAP).await;
            let _ = this.update(cx, |this, cx| {
                this.conflict_marks.running = false;
                // An answer about another HEAD is about nothing.
                if this.conflict_marks.head == Some(head)
                    && let Ok(Reply::Command(value)) = reply
                {
                    let files = value
                        .get("conflicts")
                        .and_then(|files| files.as_array())
                        .map_or(0, Vec::len);
                    this.conflict_marks.known.insert(name, (tip, files));
                    this.show_conflict_marks(cx);
                }
                this.try_next_conflict(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use bitbraid_git::Oid;

    use super::to_try;

    fn id(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    #[test]
    fn only_what_is_not_known_yet_is_tried() {
        let branches = vec![
            ("a".to_owned(), id(1), false),
            ("b".to_owned(), id(2), false),
            ("main".to_owned(), id(9), true),
            ("same".to_owned(), id(9), false),
            ("z".to_owned(), id(3), false),
        ];
        let mut known = HashMap::new();
        let names = |known: &HashMap<String, (Oid, usize)>, limit| {
            to_try(id(9), &branches, known, limit)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        };
        // Not the checked-out branch, nor one at the same commit.
        assert_eq!(names(&known, 50), ["a", "b", "z"]);
        // What was tried at its tip is not tried again; what moved is.
        known.insert("a".to_owned(), (id(1), 0));
        known.insert("b".to_owned(), (id(7), 2));
        assert_eq!(names(&known, 50), ["b", "z"]);
        // No more than the bound are looked at, tried or not.
        assert_eq!(names(&known, 2), ["b"]);
        assert!(names(&known, 0).is_empty());
    }
}
