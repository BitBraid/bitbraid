//! Gitflow's dialogs: the start of a branch of one of the five kinds. The
//! two branches and the prefixes are a section of the preferences.
//!
//! What can be told while it is typed is told there (`views/flow.rs`); the
//! engine checks all of it again.
//!
//! Sources: none (original design).

use gpui_kit::*;
use serde_json::json;

use super::{
    PromptField, Workspace,
    form::{Answers, Form, Pick, Verdict},
};
use crate::{
    notices::Notice,
    views::flow::{self, FlowRequest, KINDS},
};

/// A kind as a sentence begins with it.
fn capitalised(kind: &str) -> String {
    let mut word = kind.to_owned();
    if let Some(initial) = word.get_mut(..1) {
        initial.make_ascii_uppercase();
    }
    word
}

/// What a kind is called where it stands alone.
fn branch_word(kind: &str) -> String {
    if kind == "support" {
        say!("dialogs-flow-support-branch").to_owned()
    } else {
        kind.to_owned()
    }
}

impl Workspace {
    /// Ask for a name and start a branch: of `kind`, or of the kind that
    /// is chosen in the dialog when none is given.
    pub(super) fn flow_start(
        &mut self,
        kind: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.flow_state.initialized {
            // Nothing can be started yet: what is missing is asked first.
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("dialogs-flow-not-set-up-yet")),
            );
            cx.emit(super::WorkspaceEvent::OpenSettings(
                crate::views::preferences::Section::Gitflow,
            ));
            return;
        }
        let fixed = kind.filter(|kind| KINDS.contains(&kind.as_str()));
        let mut form = Form::new(
            match &fixed {
                Some(kind) => say!("dialogs-flow-start-title", kind = branch_word(kind)),
                None => say!("dialogs-flow-start-any-title").to_owned(),
            },
            say!("dialogs-flow-start"),
        );
        form.fields = vec![PromptField::required(
            say!("dialogs-flow-name-placeholder"),
            "",
        )];
        if fixed.is_none() {
            form.pick = Some(Pick {
                options: KINDS
                    .iter()
                    .map(|kind| {
                        (
                            capitalised(kind),
                            say!(
                                "dialogs-flow-kind-from",
                                prefix = self.flow_state.prefix_of(kind),
                                base = self.flow_state.base_of(kind)
                            ),
                        )
                    })
                    .collect(),
                picked: 0,
            });
        }
        let chosen = move |answers: &Answers| -> String {
            fixed
                .clone()
                .or_else(|| KINDS.get(answers.picked).map(|kind| (*kind).to_owned()))
                .unwrap_or_else(|| "feature".to_owned())
        };
        let for_review = chosen.clone();
        form.review = Some(std::rc::Rc::new(
            move |this: &Workspace, answers: &Answers| {
                let local = this.local_branch_names();
                let kind = for_review(answers);
                Some(
                    match flow::start_explained(&kind, &answers.text(0), &this.flow_state, &local) {
                        Ok(explained) => Verdict::Remark(explained),
                        Err(refusal) => Verdict::Refuse(refusal),
                    },
                )
            },
        ));
        self.form(
            form,
            move |this, answers, window, cx| {
                let (kind, name) = (chosen(&answers), answers.text(0));
                this.places_loaded = None;
                this.run_mutation(
                    "flow.start",
                    json!({ "kind": kind, "name": name }),
                    say!(
                        "dialogs-flow-started",
                        kind = kind.as_str(),
                        name = name.as_str()
                    ),
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Finish the branch that is checked out, when it is one of gitflow's
    /// and of a kind that is finished; say so when it is not.
    pub(super) fn flow_finish_current(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self
            .flow_state
            .branches
            .iter()
            .find(|branch| branch.current)
            .cloned();
        let said = match current {
            Some(branch) if flow::finishes(branch.kind) => {
                self.flow(
                    FlowRequest::Finish {
                        kind: branch.kind.to_owned(),
                        name: branch.name,
                        keep: false,
                    },
                    window,
                    cx,
                );
                return;
            }
            Some(branch) => say!("dialogs-flow-support-not-finished", branch = branch.branch),
            None if !self.flow_state.initialized => say!("dialogs-flow-not-set-up").to_owned(),
            None => say!("dialogs-flow-none-checked-out").to_owned(),
        };
        crate::notices::push(window, cx, Notice::info(said));
    }
}

#[cfg(test)]
mod tests {
    use super::{branch_word, capitalised};

    #[test]
    fn a_kind_is_worded_for_a_title_and_a_choice() {
        assert_eq!(capitalised("bugfix"), "Bugfix");
        assert_eq!(capitalised(""), "");
        assert_eq!(branch_word("support"), "support branch");
        assert_eq!(branch_word("hotfix"), "hotfix");
    }
}
