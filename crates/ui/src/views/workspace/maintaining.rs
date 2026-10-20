//! The repository's upkeep in its tab: git's maintenance tasks and the
//! check of the objects run from the maintenance section, and the sparse
//! checkout the lists of files show and change.
//!
//! A task, the check and a change to the sparse checkout each run as an
//! operation: one at a time, in the status bar with what git says of how
//! far it is, stoppable. What a task or the check answers goes back to
//! the maintenance view; how the checkout stands is read again after a
//! change to it, and handed to the lists of files.
//!
//! Sources: `crates/ui/src/views/maintenance.rs`, `crates/ui/src/views/sparse.rs`.

use gpui_kit::*;
use serde_json::{Value, json};

use super::{Workspace, sentence};
use crate::{
    notices::Notice,
    session::{Reply, Request},
    views::{
        maintenance::{Checked, Job, MaintenanceEvent, MaintenanceView, freed_said},
        sparse::SparseMark,
    },
};

/// What the tab keeps for its upkeep.
pub(super) struct Upkeep {
    /// The maintenance section's view, which the preferences show.
    view: Entity<MaintenanceView>,
    /// How the checkout stands, as last asked; none until it was, and for
    /// a repository without a worktree.
    sparse: Option<SparseMark>,
    _subscription: Subscription,
}

impl Upkeep {
    /// The view, and what the tab does with its events.
    pub(super) fn new(
        session: &crate::session::SessionHandle,
        window: &mut Window,
        cx: &mut Context<Workspace>,
    ) -> Self {
        let view = cx.new(|_| MaintenanceView::new(session.clone()));
        let subscription = cx.subscribe_in(
            &view,
            window,
            |this, _, event: &MaintenanceEvent, window, cx| match *event {
                MaintenanceEvent::Run(job) => this.run_upkeep(job, window, cx),
                MaintenanceEvent::Stop => this.cancel_operation(cx),
            },
        );
        Self {
            view,
            sparse: None,
            _subscription: subscription,
        }
    }
}

/// What came of an operation, as the views take it.
fn answered(reply: Result<Reply, futures::channel::oneshot::Canceled>) -> Result<Value, String> {
    match reply {
        Ok(Reply::Command(value)) => Ok(value),
        Ok(Reply::Error(err)) => Err(err),
        Ok(_) => Err(say!("dialogs-answer-not-one").to_owned()),
        // not shown: what the engine says when it is stopped, which is looked for
        Err(_) => Err("operation: cancelled".to_owned()),
    }
}

impl Workspace {
    /// The maintenance section of the preferences: this repository's.
    pub fn maintenance_view(&self) -> Entity<MaintenanceView> {
        self.upkeep.view.clone()
    }

    /// How the checkout stands, as last asked.
    pub fn sparse_mark(&self) -> Option<&SparseMark> {
        self.upkeep.sparse.as_ref()
    }

    /// Where a running task or check is, for the maintenance view.
    pub(super) fn upkeep_progress(
        &mut self,
        command: &str,
        stage: &str,
        percent: Option<u8>,
        cx: &mut Context<Self>,
    ) {
        if matches!(command, "repo.maintenance" | "repo.fsck") {
            let stage = stage.to_owned();
            self.upkeep
                .view
                .update(cx, |view, cx| view.progress(stage, percent, cx));
        }
    }

    /// Run one of git's maintenance tasks, or the check, as an operation;
    /// what it answers goes to the maintenance view and, in a sentence, to
    /// the notices.
    pub(super) fn run_upkeep(&mut self, job: Job, window: &mut Window, cx: &mut Context<Self>) {
        let (id, params, label) = match job {
            Job::Task(task) => (
                "repo.maintenance",
                json!({ "task": task.word() }),
                task.doing(),
            ),
            Job::Check => ("repo.fsck", json!({}), say!("dialogs-checking-objects")),
        };
        if self.refused_here(id, window, cx) {
            return;
        }
        let reply = self.session.send(Request::Operation {
            id: id.to_owned(),
            params,
        });
        self.operation_began(label, Some(id));
        self.upkeep.view.update(cx, |view, cx| view.began(job, cx));
        cx.spawn_in(window, async move |this, cx| {
            let reply = answered(reply.await);
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                let notice = match (&reply, job) {
                    (Err(err), _) if err.contains("cancelled") => {
                        Notice::info(say!("workspace-operation-stopped", label))
                    }
                    (Err(err), _) => Notice::error(sentence(err)),
                    (Ok(value), Job::Task(_)) => Notice::success(freed_said(value)),
                    (Ok(value), Job::Check) => {
                        let checked = Checked::from_json(value);
                        if checked.ok {
                            Notice::success(checked.said())
                        } else {
                            Notice::warning(checked.said())
                        }
                    }
                };
                crate::notices::push(window, cx, notice);
                this.upkeep
                    .view
                    .update(cx, |view, cx| view.finished(job, reply, cx));
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Ask how the checkout stands, and hand it to the lists of files.
    pub(super) fn load_sparse(&mut self, cx: &mut Context<Self>) {
        // Without a worktree nothing is checked out at all.
        if self.session.summary.workdir.is_none() {
            return;
        }
        let request = Request::Command {
            id: "sparse.status".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let Reply::Command(value) = reply else {
                return;
            };
            let mark = SparseMark::from_json(&value);
            if this.upkeep.sparse.as_ref() != Some(&mark) {
                this.upkeep.sparse = Some(mark.clone());
                let for_details = mark.clone();
                this.changes_view
                    .update(cx, |view, cx| view.set_sparse(Some(mark), cx));
                this.details_view
                    .update(cx, |view, cx| view.set_sparse(Some(for_details), cx));
                cx.notify();
            }
        });
    }

    /// Change the sparse checkout as an operation, and read how it stands
    /// afterwards. What git says of files it left where they were (they
    /// had changes) is passed on.
    pub(super) fn run_sparse(
        &mut self,
        id: &'static str,
        params: Value,
        label: String,
        done: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here(id, window, cx) {
            return;
        }
        let reply = self.session.send(Request::Operation {
            id: id.to_owned(),
            params,
        });
        self.operation_began(&label, Some(id));
        cx.spawn_in(window, async move |this, cx| {
            let reply = answered(reply.await);
            let _ = this.update_in(cx, |this, window, cx| {
                this.operation_ended();
                let notice = match &reply {
                    Err(err) if err.contains("cancelled") => {
                        Notice::info(say!("workspace-operation-stopped", label = label.as_str()))
                    }
                    Err(err) => Notice::error(sentence(err)),
                    Ok(value) => {
                        let said = value
                            .get("output")
                            .and_then(Value::as_str)
                            .filter(|said| said.contains("not up to date"));
                        match said {
                            Some(_) => Notice::warning(say!(
                                "dialogs-sparse-files-left",
                                done = done.as_str()
                            )),
                            None => Notice::success(done.clone()),
                        }
                    }
                };
                crate::notices::push(window, cx, notice);
                this.load_sparse(cx);
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// Check out every file again (from the bar over the changes, a
    /// folder's menu or the palette).
    pub fn sparse_disable(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.upkeep.sparse.as_ref().is_some_and(|mark| mark.enabled) {
            crate::notices::push(window, cx, Notice::info(say!("dialogs-sparse-all-already")));
            return;
        }
        self.run_sparse(
            "sparse.disable",
            json!({}),
            say!("dialogs-sparse-disabling").to_owned(),
            say!("dialogs-sparse-disabled").to_owned(),
            window,
            cx,
        );
    }
}
