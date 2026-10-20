//! A new branch or tag named in the graph: in a field in the label column
//! of the commit's row (`GraphView::name_new`). Enter makes it; a branch
//! made so is checked out, and an annotated tag's message is asked for
//! next, in the bar that asks in the toolbar's place. Where the graph does
//! not show the commit, the dialogs of `branches.rs` and `tagging.rs` ask
//! instead.
//!
//! Sources: git-branch(1), git-tag(1) (`-a`).

use gpui_kit::*;

use super::{Workspace, branches::judge_branch_name, form::Verdict, tagging::judge_tag_name};
use crate::{notices::Notice, views::graph::NameFor};

impl Workspace {
    /// Name what `what` is at `start` in the graph's label column. False
    /// when the graph does not show that commit.
    pub(super) fn name_in_graph(
        &mut self,
        what: NameFor,
        start: Option<String>,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        self.graph_view.update(cx, |graph, cx| {
            graph.name_new(what, start, initial, window, cx)
        })
    }

    /// The names of the tags there are.
    fn tag_names(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| snapshot.tags.iter().map(|tag| tag.name.clone()).collect())
            .unwrap_or_default()
    }

    /// A name was typed in the graph's label column: make what it names.
    /// A name git would refuse is said, and the field comes back with it.
    pub(super) fn named(
        &mut self,
        what: NameFor,
        start: Option<String>,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let verdict = match what {
            NameFor::Branch => {
                judge_branch_name(&name, &self.local_branch_names(), &self.tag_names())
            }
            NameFor::Tag | NameFor::AnnotatedTag => judge_tag_name(&name, &self.tag_names()),
        };
        if let Some(Verdict::Refuse(why)) = verdict {
            crate::notices::push(window, cx, Notice::error(why));
            self.name_in_graph(what, start, &name, window, cx);
            return;
        }
        let target = start.clone().unwrap_or_else(|| "HEAD".to_owned());
        match what {
            NameFor::Branch => self.create_branch(name, start, true, false, window, cx),
            NameFor::Tag => self.create_tag(name, target, String::new(), window, cx),
            NameFor::AnnotatedTag => {
                let workspace = cx.entity().downgrade();
                crate::ask::show(
                    crate::ask::Ask::new(say!("workspace-annotate-title", name = name.as_str()))
                        .field(say!("workspace-prompt-message"), "")
                        .go(say!("workspace-create-tag"), move |texts, window, cx| {
                            let message = texts.first().cloned().unwrap_or_default();
                            let (name, target) = (name.clone(), target.clone());
                            let _ = workspace.update(cx, |this, cx| {
                                this.create_tag(name, target, message, window, cx)
                            });
                        }),
                    window,
                    cx,
                );
            }
        }
    }

    /// Make the tag `name` at `target`, annotated when it has a message.
    fn create_tag(
        &mut self,
        name: String,
        target: String,
        message: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_mutation(
            "tag.create",
            super::tagging::tag_params(&name, &target, &message, false),
            say!("workspace-created-tag", name = name.as_str()),
            |_, _, _, _| false,
            window,
            cx,
        );
    }
}
