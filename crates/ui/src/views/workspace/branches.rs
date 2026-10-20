//! The questions about branches that carry a choice: a new branch that is
//! checked out or not, with its name looked at as it is typed; what a
//! branch is to follow, picked from the branches the remotes have; and a
//! branch deleted here and, in the same go, on its remote.
//!
//! Sources: git-branch(1), git-check-ref-format(1), git-switch(1).

use gpui_kit::*;
use serde_json::json;

use super::{
    PromptField, Workspace,
    form::{Check, Form, Suggest, Verdict},
    split_remote,
};
use crate::{
    notices::Notice,
    prefs::Prefs,
    session::{Reply, Request},
};

/// What there is to say about `typed` as the name of a new branch, with
/// the local branches and the tags there are: git's rules first, then a
/// name that is taken, then a tag of the same name, which is allowed and
/// worth knowing.
pub(super) fn judge_branch_name(
    typed: &str,
    existing: &[String],
    tags: &[String],
) -> Option<Verdict> {
    if let Some(problem) = bitbraid_git::branch::name_problem(typed) {
        return Some(Verdict::Refuse(problem));
    }
    if existing.iter().any(|name| name == typed) {
        return Some(Verdict::Refuse(say!("dialogs-branch-exists", name = typed)));
    }
    // Git keeps a branch as a file: one name cannot be both a branch and
    // the folder of others.
    let clash = existing.iter().find(|name| {
        name.strip_prefix(typed)
            .is_some_and(|rest| rest.starts_with('/'))
            || typed
                .strip_prefix(name.as_str())
                .is_some_and(|rest| rest.starts_with('/'))
    });
    if let Some(other) = clash
        && !typed.is_empty()
    {
        return Some(Verdict::Refuse(say!(
            "dialogs-branch-folder-clash",
            other,
            typed
        )));
    }
    if tags.iter().any(|tag| tag == typed) {
        return Some(Verdict::Remark(say!("dialogs-branch-tag-too", typed)));
    }
    None
}

/// What a branch that is being given an upstream starts the field with:
/// what it follows now, else the branch of its name on a remote when
/// there is one, else nothing.
pub(super) fn upstream_start(
    name: &str,
    now: Option<&str>,
    on_offer: &[String],
    remotes: &[String],
) -> String {
    if let Some(now) = now {
        return now.to_owned();
    }
    on_offer
        .iter()
        .find(|offer| split_remote(remotes, offer).is_some_and(|(_, branch)| branch == name))
        .cloned()
        .unwrap_or_default()
}

impl Workspace {
    /// The branches of every remote as git names them, `remote/branch`.
    pub(super) fn remote_branch_names(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .remotes
                    .iter()
                    .flat_map(|remote| {
                        remote
                            .branches
                            .iter()
                            .map(|branch| format!("{}/{}", remote.name, branch.name))
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Ask for a name and create a branch at `start` (HEAD when absent),
    /// checked out or not. The name is looked at while it is typed.
    pub fn prompt_new_branch(
        &mut self,
        start: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Named in the graph's label column, on the commit's row, where
        // the graph shows it; in a dialog where it does not.
        if self.name_in_graph(
            crate::views::graph::NameFor::Branch,
            start.clone(),
            "",
            window,
            cx,
        ) {
            return;
        }
        let title = match &start {
            Some(start) if start.len() == 40 && start.chars().all(|c| c.is_ascii_hexdigit()) => {
                say!(
                    "dialogs-new-branch-at",
                    start = start.chars().take(7).collect::<String>()
                )
            }
            Some(start) => say!("dialogs-new-branch-at", start),
            None => say!("dialogs-new-branch-at-head").to_owned(),
        };
        let mut form = Form::new(title, say!("dialogs-create-branch"));
        form.fields = vec![PromptField::required(say!("workspace-prompt-name"), "")];
        form.judge = Some(std::rc::Rc::new(|this: &Self, typed: &str, _: &[bool]| {
            let tags: Vec<String> = this
                .snapshot
                .as_ref()
                .map(|snapshot| snapshot.tags.iter().map(|tag| tag.name.clone()).collect())
                .unwrap_or_default();
            judge_branch_name(typed, &this.local_branch_names(), &tags)
        }));
        form.checks = vec![Check::new(say!("dialogs-check-it-out"), true)];
        self.form(
            form,
            move |this, answers, window, cx| {
                this.create_branch(
                    answers.text(0),
                    start.clone(),
                    answers.check(0),
                    false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }

    /// Create a branch and, when asked, check it out. Local changes in
    /// the way of that checkout are carried over through the stash when
    /// the preferences say so (`stash`), and otherwise the branch is not
    /// made and the notice says how it can be.
    pub(super) fn create_branch(
        &mut self,
        name: String,
        start: Option<String>,
        checkout: bool,
        stash: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut params = json!({ "name": name, "start": start, "checkout": checkout });
        if let (true, Some(params)) = (stash, params.as_object_mut()) {
            params.insert("dirty".to_owned(), json!("stash"));
        }
        let again = (name.clone(), start.clone());
        self.run_mutation(
            "branch.create",
            params,
            if checkout {
                say!("dialogs-created-branch-checked-out", name = name.as_str())
            } else {
                say!(
                    "dialogs-created-branch-not-checked-out",
                    name = name.as_str()
                )
            },
            move |this, err, window, cx| {
                if stash || !err.contains("would be overwritten") {
                    return false;
                }
                let (name, start) = again.clone();
                if cx.global::<Prefs>().preferences.checkout_stash {
                    this.create_branch(name, start, true, true, window, cx);
                } else {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::error(say!("dialogs-create-branch-dirty", name)),
                    );
                }
                true
            },
            window,
            cx,
        );
    }

    /// Ask what a local branch is to follow, with the branches of the
    /// remotes to pick from; what is typed narrows them down.
    pub(super) fn prompt_upstream(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snapshot) = self.snapshot.as_ref() else {
            return;
        };
        let now = snapshot
            .branches
            .iter()
            .find(|branch| branch.name == name)
            .and_then(|branch| branch.upstream.clone());
        let on_offer = self.remote_branch_names();
        if on_offer.is_empty() {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("dialogs-upstream-none-known", name = name.as_str())),
            );
            return;
        }
        let mut form = Form::new(
            say!("dialogs-upstream-title", name = name.as_str()),
            say!("dialogs-set-upstream"),
        );
        form.fields = vec![PromptField::required(
            say!("dialogs-upstream-placeholder"),
            &upstream_start(&name, now.as_deref(), &on_offer, &self.remote_names()),
        )];
        form.suggest = Some(Suggest {
            names: on_offer,
            strict: true,
        });
        self.form(
            form,
            move |this, answers, window, cx| {
                let upstream = answers.text(0);
                this.run_mutation(
                    "branch.set_upstream",
                    json!({ "name": name, "upstream": upstream }),
                    say!(
                        "dialogs-upstream-set",
                        name = name.as_str(),
                        upstream = upstream.as_str()
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

    /// Ask, then delete a local branch; when it follows a branch of a
    /// remote, that one can go in the same breath.
    pub(super) fn confirm_delete_branch(
        &mut self,
        name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let remotes = self.remote_names();
        let followed = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.branches.iter().find(|branch| branch.name == name))
            .and_then(|branch| branch.upstream.clone())
            .and_then(|upstream| split_remote(&remotes, &upstream));
        // Asked in the bar: deleting it here, or here and on its remote.
        let workspace = cx.entity().downgrade();
        let here = {
            let (workspace, name) = (workspace.clone(), name.clone());
            move |_: Vec<String>, window: &mut Window, cx: &mut App| {
                let _ = workspace.update(cx, |this, cx| {
                    this.delete_branch_and(name.clone(), false, None, window, cx)
                });
            }
        };
        let ask = match &followed {
            Some((remote, branch)) => {
                let both = (followed.clone(), name.clone());
                crate::ask::Ask::new(say!(
                    "dialogs-delete-branch-or-both",
                    name = name.as_str(),
                    remote,
                    branch
                ))
                .danger(say!("workspace-delete"), here)
                .other(say!("dialogs-delete-both"), move |_, window, cx| {
                    let (on_remote, name) = both.clone();
                    let _ = workspace.update(cx, |this, cx| {
                        this.delete_branch_and(name, false, on_remote, window, cx)
                    });
                })
            }
            None => {
                crate::ask::Ask::new(say!("dialogs-delete-branch-question", name = name.as_str()))
                    .danger(say!("workspace-delete"), here)
            }
        };
        crate::ask::show(ask, window, cx);
    }

    /// Delete a local branch and then, when one is named, the branch it
    /// followed on its remote. Each is its own step for undo. A branch
    /// with commits that are nowhere else is asked about once more, and
    /// nothing goes on the remote unless the local branch went.
    fn delete_branch_and(
        &mut self,
        name: String,
        force: bool,
        on_remote: Option<(String, String)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here("branch.delete", window, cx) {
            return;
        }
        let protected = cx.global::<Prefs>().preferences.protected_branches.clone();
        let reply = self.session.send(Request::Command {
            id: "branch.delete".to_owned(),
            params: json!({ "name": name, "force": force, "protected": protected }),
        });
        let before = self.undo_id;
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let made = Self::journalled(&this, cx, before).await;
            let _ = this.update_in(cx, |this, window, cx| {
                match reply {
                    // git refuses a branch whose commits are nowhere else.
                    Reply::Error(err) if !force && err.contains("not fully merged") => {
                        this.ask_in_bar(
                            say!("dialogs-delete-unmerged-question", name = name.as_str()),
                            say!("workspace-delete"),
                            true,
                            move |this, window, cx| {
                                this.delete_branch_and(
                                    name.clone(),
                                    true,
                                    on_remote.clone(),
                                    window,
                                    cx,
                                )
                            },
                            window,
                            cx,
                        );
                        return;
                    }
                    Reply::Error(err) => {
                        let notice = this.failed_notice(&err, cx);
                        crate::notices::push(window, cx, notice);
                    }
                    _ => {
                        let notice = this.done_notice(
                            say!("dialogs-deleted-branch", name = name.as_str()),
                            made,
                            cx,
                        );
                        crate::notices::push(window, cx, notice);
                        if let Some((remote, branch)) = on_remote {
                            this.run_operation(
                                "branch.delete_remote",
                                json!({
                                    "remote": remote,
                                    "branch": branch,
                                    "protected": protected,
                                }),
                                say!("workspace-deleting-on-remote"),
                                say!(
                                    "workspace-deleted-on-remote",
                                    branch = branch.as_str(),
                                    remote = remote.as_str()
                                ),
                                window,
                                cx,
                            );
                        }
                    }
                }
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{judge_branch_name, upstream_start};
    use crate::views::workspace::form::Verdict;

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_new_branch_name_is_judged_as_it_is_typed() {
        let existing = names(&["main", "topic/login", "release"]);
        let tags = names(&["v1.0", "main"]);
        let judge_branch_name = |typed| judge_branch_name(typed, &existing, &tags);
        // Nothing typed, and a name that will do: nothing to say.
        assert_eq!(judge_branch_name(""), None);
        assert_eq!(judge_branch_name("topic/logout"), None);
        assert_eq!(judge_branch_name("mainline"), None);
        // A tag of that name is no obstacle, and is said.
        assert!(matches!(
            judge_branch_name("v1.0"),
            Some(Verdict::Remark(said)) if said.starts_with("There is a tag v1.0")
        ));
        // Git's rules, in git's order of importance.
        assert_eq!(
            judge_branch_name("my branch"),
            Some(Verdict::Refuse(
                "The name cannot contain a space.".to_owned()
            ))
        );
        assert!(matches!(
            judge_branch_name("a..b"),
            Some(Verdict::Refuse(_))
        ));
        // A name that is taken.
        assert_eq!(
            judge_branch_name("main"),
            Some(Verdict::Refuse(
                "There is a branch main already.".to_owned()
            ))
        );
        // A name that would be the folder of a branch, or in the folder
        // that a branch is.
        let Some(Verdict::Refuse(said)) = judge_branch_name("topic") else {
            panic!("topic was let through");
        };
        assert!(said.starts_with("There is a branch topic/login"), "{said}");
        let Some(Verdict::Refuse(said)) = judge_branch_name("release/1.0") else {
            panic!("release/1.0 was let through");
        };
        assert!(said.starts_with("There is a branch release"), "{said}");
    }

    #[test]
    fn the_upstream_field_starts_with_what_is_likeliest() {
        let offers = names(&["origin/main", "backup/topic", "origin/topic/login"]);
        let remotes = names(&["origin", "backup"]);
        let start = |name, now| upstream_start(name, now, &offers, &remotes);
        assert_eq!(start("topic", Some("origin/main")), "origin/main");
        assert_eq!(start("topic", None), "backup/topic");
        assert_eq!(start("topic/login", None), "origin/topic/login");
        // `origin/topic/login` is the branch `topic/login`, not `login`.
        assert_eq!(start("login", None), "");
        assert_eq!(start("nothing", None), "");
        assert_eq!(upstream_start("main", None, &[], &remotes), "");
    }
}
