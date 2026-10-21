//! What runs for a while says so in the status bar. An operation does from
//! the moment it is handed to the operations thread; a command that
//! changes the repository on the session thread does once it has run for a
//! moment, since everything asked of that thread after it waits. A
//! worktree that is being removed is marked where it has a row as well,
//! since git says nothing while it deletes its files.
//!
//! Sources: gpui-kit documentation (timers).

use std::{collections::BTreeSet, time::Duration};

use gpui_kit::*;

use super::Workspace;

/// A command on the session thread is on show once it has run this long:
/// one that answers sooner would only flicker.
pub(super) const SHOWN_AFTER: Duration = Duration::from_millis(300);

/// The operations git cannot stop half-way: it deletes or moves a
/// worktree's files and says nothing until it is done.
const UNSTOPPABLE: &[&str] = &["worktree.remove", "worktree.move"];

/// What the session thread is doing for a command that changes the
/// repository.
#[derive(Debug, Default)]
pub(super) struct Busy {
    /// The command's label, while it runs.
    label: Option<String>,
    /// Whether it has run long enough to be on show.
    shown: bool,
    /// How many have begun: the wait of one that has answered is not taken
    /// for the next one's.
    begun: u64,
}

/// Whether the stop of the status bar can stop this operation.
pub(super) fn stoppable(command: Option<&str>) -> bool {
    command.is_none_or(|command| !UNSTOPPABLE.contains(&command))
}

impl Workspace {
    /// The session thread began a command that changes the repository, or
    /// answered it.
    pub(super) fn on_busy(&mut self, label: Option<String>, cx: &mut Context<Self>) {
        let Some(label) = label else {
            let was_shown = std::mem::take(&mut self.busy.shown);
            self.busy.label = None;
            if was_shown {
                cx.notify();
            }
            return;
        };
        self.busy.begun = self.busy.begun.wrapping_add(1);
        self.busy.label = Some(label);
        self.busy.shown = false;
        let begun = self.busy.begun;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SHOWN_AFTER).await;
            let _ = this.update(cx, |this, cx| {
                if this.busy.begun == begun && this.busy.label.is_some() {
                    this.busy.shown = true;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// What the status bar says of the command on the session thread, once
    /// it has run for a moment.
    pub(super) fn busy_text(&self) -> Option<String> {
        let label = self.busy.label.as_deref().filter(|_| self.busy.shown)?;
        Some(format!(
            "{}…",
            say!("workspace-command-running", command = label)
        ))
    }

    /// The command that changes the repository on the session thread, by
    /// its label, whether it is on show yet or not.
    pub(super) fn busy_label(&self) -> Option<&str> {
        self.busy.label.as_deref()
    }

    /// Whether the status bar offers to stop what it shows as running.
    pub fn operation_stoppable(&self) -> bool {
        self.operation
            .as_ref()
            .is_some_and(|operation| stoppable(operation.command.as_deref()))
    }

    /// A worktree is being removed, or no longer: its rows say so.
    pub(super) fn set_removing(&mut self, path: &str, removing: bool, cx: &mut Context<Self>) {
        let changed = if removing {
            self.removing.insert(path.to_owned())
        } else {
            self.removing.remove(path)
        };
        if !changed {
            return;
        }
        let removing: BTreeSet<String> = self.removing.clone();
        self.refs_view
            .update(cx, |view, cx| view.set_removing(removing.clone(), cx));
        self.graph_view
            .update(cx, |view, cx| view.set_removing(removing, cx));
        cx.notify();
    }
}
