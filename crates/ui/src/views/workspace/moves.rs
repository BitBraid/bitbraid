//! What is asked before the current branch is moved in one of the ways
//! that have a choice in them: how to merge, what to rebase and onto
//! what, and which parent a merge commit is picked or reverted against.
//!
//! Sources: git-merge(1) (`--ff-only`, `--no-ff`, `--squash`,
//! `--no-commit`), git-rebase(1) (`--onto`, `--autostash`),
//! git-cherry-pick(1) and git-revert(1) (`--mainline`).

use bitbraid_git::{ChangeKind, hooks::Unverified};
use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    PromptField, Workspace,
    form::{Check, Form, Pick, Suggest},
    sentence,
};
use crate::{
    notices::Notice,
    session::{Reply, Request},
    views::rewrite::{MERGE_MODES, Rewrite},
};

/// A commit id cut for reading; a name stays as it is.
fn shown(rev: &str) -> String {
    let is_id = rev.len() == 40 && rev.bytes().all(|byte| byte.is_ascii_hexdigit());
    if is_id {
        rev.chars().take(7).collect()
    } else {
        rev.to_owned()
    }
}

/// The parents of a merge commit as a dialog lists them: which one it
/// is, its id, and what it says. `details` is what `repo.commits` answers
/// for the parents, which may leave some out.
pub(super) fn parent_options(parents: &[String], details: &Value) -> Vec<(String, String)> {
    let summary_of = |id: &str| {
        details
            .as_array()
            .into_iter()
            .flatten()
            .find(|commit| commit.get("id").and_then(|found| found.as_str()) == Some(id))
            .and_then(|commit| commit.get("summary")?.as_str())
            .unwrap_or("")
            .to_owned()
    };
    parents
        .iter()
        .enumerate()
        .map(|(ix, id)| {
            let number = ix.saturating_add(1);
            let short: String = id.chars().take(7).collect();
            let summary = summary_of(id);
            (
                if ix == 0 {
                    say!("dialogs-parent-first", number)
                } else {
                    say!("dialogs-parent-merged-in", number)
                },
                if summary.is_empty() {
                    short
                } else {
                    format!("{short} {summary}")
                },
            )
        })
        .collect()
}

/// The parents `repo.commits` gives for one commit.
pub(super) fn parents_of(details: &Value) -> Vec<String> {
    details
        .as_array()
        .and_then(|commits| commits.first())
        .and_then(|commit| commit.get("parents")?.as_array())
        .into_iter()
        .flatten()
        .filter_map(|parent| parent.as_str().map(str::to_owned))
        .take(16)
        .collect()
}

impl Workspace {
    /// Open the dialog a request needs before it can run, when it is one
    /// that asks first. True when it was: nothing more is to be done with
    /// the request, the dialog goes on from here.
    pub(super) fn ask_how(
        &mut self,
        rewrite: &Rewrite,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match rewrite {
            Rewrite::MergeHow { target } => {
                self.ask_merge(target.clone(), window, cx);
                true
            }
            Rewrite::RebaseHow { onto, after } => {
                self.ask_rebase(onto.clone(), after.clone(), window, cx);
                true
            }
            // Planned in the editor, on another base.
            Rewrite::InteractiveOnto { onto } => {
                let onto = onto.clone();
                self.planning = true;
                let hooks = self.hooks_there(Unverified::Rebase, cx);
                self.rebase_editor.update(cx, |editor, cx| {
                    editor.set_refusing_hooks(hooks);
                    editor.open_onto(onto, window, cx)
                });
                cx.notify();
                true
            }
            // The panel that makes a pull request (`pulling.rs`).
            Rewrite::StartPull { head, base } => {
                self.start_pull(head.clone(), base.clone(), window, cx);
                true
            }
            Rewrite::PushTo { branch, onto } => {
                self.push_onto(branch.clone(), onto.clone(), window, cx);
                true
            }
            Rewrite::CherryPick {
                commit,
                merge: true,
                staged,
            } => {
                self.ask_mainline(commit.clone(), false, *staged, window, cx);
                true
            }
            Rewrite::Revert {
                commit,
                merge: true,
            } => {
                self.ask_mainline(commit.clone(), true, false, window, cx);
                true
            }
            _ => false,
        }
    }

    /// Push a local branch to a branch of a remote, named `remote/branch`
    /// (a drop on its label): plainly, so that it is refused where the
    /// remote's branch would lose commits.
    fn push_onto(
        &mut self,
        branch: String,
        onto: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((remote, there)) = super::split_remote(&self.remote_names(), &onto) else {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("dialogs-push-onto-unknown", onto = onto.as_str())),
            );
            return;
        };
        let to = (there != branch).then(|| there.clone());
        self.run_operation_with(
            "remote.push",
            json!({ "branch": branch, "remote": remote, "to": to }),
            say!("workspace-pushing"),
            say!(
                "dialogs-pushed-onto",
                branch = branch.as_str(),
                remote = remote.as_str(),
                there = there.as_str()
            ),
            move |_, err, window, cx| {
                if !super::push_rejected(err) {
                    return false;
                }
                crate::notices::push(
                    window,
                    cx,
                    Notice::error(say!(
                        "dialogs-push-onto-refused",
                        onto = onto.as_str(),
                        branch = branch.as_str()
                    )),
                );
                true
            },
            window,
            cx,
        );
    }

    /// The branch that is checked out, as menus and dialogs call it.
    fn here(&self) -> String {
        self.snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.head.branch.clone())
            .unwrap_or_else(|| "HEAD".to_owned())
    }

    fn ask_merge(&mut self, target: String, window: &mut Window, cx: &mut Context<Self>) {
        let mut form = Form::new(
            say!(
                "dialogs-merge-title",
                target = shown(&target),
                here = self.here()
            ),
            say!("dialogs-merge"),
        );
        form.pick = Some(Pick {
            options: MERGE_MODES
                .iter()
                .map(|(_, label, more)| ((*label).to_owned(), (*more).to_owned()))
                .collect(),
            picked: 0,
        });
        // Offered where there is a hook that could refuse the merge.
        let hooks = self.hooks_there(Unverified::Merge, cx);
        if !hooks.is_empty() {
            form.checks = vec![Check::new(super::hooks::switch_label(&hooks), false)];
        }
        self.form(
            form,
            move |this, answers, window, cx| {
                let Some((mode, _, _)) = MERGE_MODES.get(answers.picked) else {
                    return;
                };
                this.rewrite(
                    Rewrite::MergeAs {
                        target: target.clone(),
                        mode: *mode,
                        no_verify: answers.check(0),
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Every branch and tag by the name git takes: what a new base can be
    /// picked from.
    fn base_names(&self) -> Vec<String> {
        let Some(snapshot) = &self.snapshot else {
            return Vec::new();
        };
        let mut names: Vec<String> = snapshot
            .branches
            .iter()
            .filter(|branch| !branch.is_head)
            .map(|branch| branch.name.clone())
            .collect();
        names.extend(self.remote_branch_names());
        names.extend(snapshot.tags.iter().map(|tag| tag.name.clone()));
        names
    }

    fn ask_rebase(
        &mut self,
        onto: Option<String>,
        after: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Changes git would refuse to rebase over: set aside by default.
        let dirty = self.status.iter().any(|entry| {
            (entry.staged.is_some() || entry.unstaged.is_some())
                && entry.unstaged != Some(ChangeKind::Untracked)
        });
        let mut form = Form::new(
            say!("dialogs-rebase-title", here = self.here()),
            say!("dialogs-rebase"),
        );
        form.lead = Some(match &after {
            Some(_) => say!("dialogs-rebase-lead-after").to_owned(),
            None => say!("dialogs-rebase-lead").to_owned(),
        });
        form.fields = vec![
            PromptField {
                placeholder: say!("dialogs-rebase-base-placeholder"),
                initial: onto.clone().unwrap_or_default(),
                required: true,
            },
            PromptField {
                placeholder: say!("dialogs-rebase-after-placeholder"),
                initial: after.unwrap_or_default(),
                required: false,
            },
        ];
        // A base that is given needs no list to pick it from.
        if onto.is_none() {
            form.suggest = Some(Suggest {
                names: self.base_names(),
                strict: false,
            });
        }
        form.checks = vec![Check::new(say!("dialogs-rebase-autostash"), dirty)];
        // Offered where there is a hook that could refuse the rebase.
        let hooks = self.hooks_there(Unverified::Rebase, cx);
        if !hooks.is_empty() {
            form.checks
                .push(Check::new(super::hooks::switch_label(&hooks), false));
        }
        self.form(
            form,
            |this, answers, window, cx| {
                let after = answers.text(1);
                this.rewrite(
                    Rewrite::RebaseWith {
                        onto: answers.text(0),
                        after: (!after.is_empty()).then_some(after),
                        autostash: answers.check(0),
                        no_verify: answers.check(1),
                    },
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Ask which parent a merge commit is picked or reverted against,
    /// with what each parent is, and go on with the one chosen.
    fn ask_mainline(
        &mut self,
        commit: String,
        revert: bool,
        staged: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let about = self.session.send(Request::Command {
            id: "repo.commits".to_owned(),
            params: json!({ "ids": [commit] }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let parents = match about.await {
                Ok(Reply::Command(value)) => parents_of(&value),
                Ok(Reply::Error(err)) => {
                    let _ = this.update_in(cx, |_, window, cx| {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)))
                    });
                    return;
                }
                _ => return,
            };
            let asked = this.update(cx, |this, _| {
                this.session.send(Request::Command {
                    id: "repo.commits".to_owned(),
                    params: json!({ "ids": parents }),
                })
            });
            let details = match asked {
                Ok(reply) => match reply.await {
                    Ok(Reply::Command(value)) => value,
                    _ => Value::Null,
                },
                Err(_) => return,
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let options = parent_options(&parents, &details);
                if options.len() < 2 {
                    // Not a merge commit after all: as any other commit.
                    let plain = if revert {
                        Rewrite::Revert {
                            commit: commit.clone(),
                            merge: false,
                        }
                    } else {
                        Rewrite::CherryPick {
                            commit: commit.clone(),
                            merge: false,
                            staged,
                        }
                    };
                    this.rewrite(plain, window, cx);
                    return;
                }
                let short = shown(&commit);
                let mut form = Form::new(
                    if revert {
                        say!("dialogs-revert-merge-title", short = short.as_str())
                    } else {
                        say!("dialogs-cherry-pick-merge-title", short = short.as_str())
                    },
                    if revert {
                        say!("dialogs-revert")
                    } else {
                        say!("dialogs-cherry-pick")
                    },
                );
                form.lead = Some(if revert {
                    say!("dialogs-revert-merge-lead").to_owned()
                } else {
                    say!("dialogs-cherry-pick-merge-lead").to_owned()
                });
                form.pick = Some(Pick { options, picked: 0 });
                self::open_mainline(this, form, commit.clone(), revert, staged, window, cx);
            });
        })
        .detach();
    }
}

fn open_mainline(
    this: &mut Workspace,
    form: Form,
    commit: String,
    revert: bool,
    staged: bool,
    window: &mut Window,
    cx: &mut Context<Workspace>,
) {
    this.form(
        form,
        move |this, answers, window, cx| {
            let mainline = u32::try_from(answers.picked.saturating_add(1)).unwrap_or(1);
            this.rewrite(
                Rewrite::OfMerge {
                    commit: commit.clone(),
                    mainline,
                    revert,
                    staged,
                },
                window,
                cx,
            );
        },
        window,
        cx,
    );
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{parent_options, parents_of, shown};

    #[test]
    fn the_parents_of_a_merge_are_listed_with_what_they_are() {
        let (first, second) = ("a".repeat(40), "b".repeat(40));
        let commit = json!([{ "id": "c".repeat(40), "parents": [first, second] }]);
        let parents = parents_of(&commit);
        assert_eq!(parents, [first.clone(), second.clone()]);
        let details = json!([
            { "id": second, "summary": "topic work" },
            { "id": first, "summary": "main work" },
        ]);
        assert_eq!(
            parent_options(&parents, &details),
            [
                (
                    "Parent 1, the line it was merged into".to_owned(),
                    "aaaaaaa main work".to_owned()
                ),
                (
                    "Parent 2, what was merged in".to_owned(),
                    "bbbbbbb topic work".to_owned()
                ),
            ]
        );
        // A parent nothing is known about is its id alone.
        assert_eq!(
            parent_options(&parents, &json!(null))[1].1,
            "bbbbbbb".to_owned()
        );
        // What is no answer has no parents.
        assert!(parents_of(&json!("nonsense")).is_empty());
        assert!(parents_of(&json!([])).is_empty());
        assert!(parents_of(&json!([{ "parents": [7, null] }])).is_empty());
    }

    #[test]
    fn ids_are_cut_and_names_are_not() {
        assert_eq!(shown(&"ab".repeat(20)), "abababa");
        assert_eq!(shown("feature/x"), "feature/x");
        let wide = "é".repeat(40);
        assert_eq!(shown(&wide), wide);
    }
}
