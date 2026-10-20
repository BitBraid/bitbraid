//! The checks of commits on the rows of the graph, and the mark at the
//! right end of a branch's label when that branch has an open pull
//! request. What is known and asked is `views::pulls::checks`; this is the
//! graph's side: which rows are in view, what a row draws, and the answers
//! of `ci.status`.
//!
//! Without a remote linked to an account nothing is asked, and a row
//! costs no more than one look at an empty table.
//!
//! Sources: none (original design).

use std::{collections::HashMap, rc::Rc, time::Instant};

use bitbraid_git::Oid;
use bitbraid_integrations::model::{CommitStatus, RepoLink};
use gpui_kit::component::{Icon, tooltip::Tooltip};
use gpui_kit::*;
use serde_json::json;

use super::GraphView;
use crate::{
    session::{Reply, SessionHandle, run_standalone},
    views::pulls::{checks::commit_mark, facts::PullMark},
};

/// The open pull request of a label's branch, when the label is of a
/// branch (here or on a remote) and that branch has one.
pub(super) fn pull_of<'a>(
    marks: &'a HashMap<String, PullMark>,
    name: &str,
    is_branch: bool,
) -> Option<&'a PullMark> {
    if !is_branch {
        return None;
    }
    marks.get(name)
}

/// The mark at the right end of a label whose branch has an open pull
/// request; its hover names the pull request.
pub(super) fn pull_mark(id: ElementId, mark: &PullMark, colour: Hsla) -> impl IntoElement + use<> {
    let said = mark.words();
    div()
        .id(id)
        .flex_none()
        .child(
            Icon::new(gpui_kit::assets::IconName::GitPullRequest)
                .size_3p5()
                .text_color(colour),
        )
        .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
        .test_support()
}

impl GraphView {
    /// Ask the checks of the commits of this repository at a provider, or
    /// none: what was known of another is forgotten.
    pub fn set_checks_source(&mut self, link: Option<RepoLink>, cx: &mut Context<Self>) {
        if self.checks.set_source(link) {
            cx.notify();
        }
    }

    /// Whether a remote of the repository is linked to an account: the
    /// labels' menus and drops offer to start a pull request.
    pub fn set_pulls_linked(&mut self, linked: bool, cx: &mut Context<Self>) {
        if self.checks.set_linked(linked) {
            cx.notify();
        }
    }

    /// The graph that is shown, once there is one.
    pub fn graph_data(&self) -> Option<std::sync::Arc<crate::session::GraphData>> {
        self.data.clone()
    }

    /// The branches with an open pull request, by name.
    pub fn set_pull_marks(&mut self, marks: HashMap<String, PullMark>, cx: &mut Context<Self>) {
        if *self.pull_marks != marks {
            self.pull_marks = Rc::new(marks);
            cx.notify();
        }
    }

    /// Hold these checks as known, without asking: for the off-screen
    /// tests.
    pub fn give_checks(&mut self, statuses: Vec<CommitStatus>, cx: &mut Context<Self>) {
        self.checks.give(statuses);
        cx.notify();
    }

    /// For the off-screen tests: whether checks are asked for, how many
    /// commits have a mark, and how many branches have a pull request.
    pub fn checks_probe(&self) -> (bool, usize, usize) {
        (
            self.checks.is_on(),
            self.checks.marked(),
            self.pull_marks.len(),
        )
    }

    /// The rows of the list in view were drawn: ask about the checks of
    /// their commits that are due, a batch at a time.
    pub(super) fn want_checks(&mut self, rows: &[usize], cx: &mut Context<Self>) {
        if !self.checks.is_on() {
            return;
        }
        let Some(data) = self.data.clone() else {
            return;
        };
        let shas: Vec<String> = rows
            .iter()
            .filter_map(|row| self.commit_ix(*row))
            .filter_map(|ix| data.commits.get(ix))
            .filter(|commit| !data.worktree_rows.contains_key(&commit.id))
            .map(|commit| commit.id.to_hex())
            .collect();
        let Some(batch) = self
            .checks
            .wanted(shas.iter().map(String::as_str), Instant::now())
        else {
            return;
        };
        let generation = batch.generation;
        let running = run_standalone(
            "ci.status",
            json!({
                "account": batch.source.account,
                "owner": batch.source.owner,
                "name": batch.source.name,
                "shas": batch.shas,
            }),
        );
        SessionHandle::deliver(
            running.reply,
            "ci.status".to_owned(),
            cx,
            move |this, reply, cx| match reply {
                Reply::Command(value) => {
                    let statuses: Vec<CommitStatus> =
                        serde_json::from_value(value).unwrap_or_default();
                    if this.checks.answered(generation, statuses) {
                        cx.notify();
                    }
                }
                _ => this.checks.failed(generation),
            },
        );
    }

    /// The mark of a commit's checks on its row, when they are known.
    pub(super) fn check_mark(
        &self,
        id: Oid,
        list_ix: usize,
        cx: &App,
    ) -> Option<impl IntoElement + use<>> {
        if self.checks.is_empty() {
            return None;
        }
        let status = self.checks.of(&id.to_hex())?;
        Some(commit_mark(
            ("checks", list_ix).into(),
            status,
            self.now,
            cx,
        ))
    }

    /// The branches with an open pull request, for the labels of a row:
    /// none while there are none, which costs a row nothing.
    pub(super) fn pull_marks_for_rows(&self) -> Option<Rc<HashMap<String, PullMark>>> {
        (!self.pull_marks.is_empty()).then(|| self.pull_marks.clone())
    }
}
