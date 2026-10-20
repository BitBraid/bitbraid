//! What is asked about a submodule: where one comes from, where it goes
//! and which branch it follows when it is added (in the docked pane,
//! `docked.rs`); and, when one is removed, whether the clone git keeps of
//! it goes too.
//!
//! Sources: git-submodule(1) (`add --branch`, `deinit`); gitsubmodules(7)
//! (a submodule's repository lives under the superproject's
//! `.git/modules`, and stays there when the submodule is removed).

use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    Workspace,
    form::{Check, Form},
    refuses_local_clone,
};
use crate::session::{Reply, Request};

/// What is asked when a submodule is added.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) struct Adding {
    /// Where it is cloned from.
    pub url: String,
    /// The folder it goes to, inside the repository.
    pub path: String,
    /// The branch it follows; none for the default of where it comes from.
    pub branch: String,
}

impl Adding {
    /// The parameters of `submodule.add`. No branch is not an empty one.
    pub(super) fn params(&self, allow_local: bool) -> Value {
        let mut params = json!({
            "url": self.url,
            "path": self.path,
            "allow_local": allow_local,
        });
        if !self.branch.is_empty()
            && let Some(fields) = params.as_object_mut()
        {
            fields.insert("branch".to_owned(), json!(self.branch));
        }
        params
    }

    /// The folder a repository is usually cloned into: the last part of
    /// where it comes from, without `.git`.
    pub(super) fn usual_folder(url: &str) -> String {
        let url = url.trim().trim_end_matches('/');
        let last = url.rsplit(['/', ':']).next().unwrap_or_default();
        last.strip_suffix(".git").unwrap_or(last).to_owned()
    }

    /// What there is to say while it is typed: `Err` for what cannot be,
    /// `Ok` with what is worth knowing. `submodules` are the paths of the
    /// ones there are.
    pub(super) fn reviewed(&self, submodules: &[String]) -> Result<Option<String>, String> {
        if !bitbraid_core::paths::stays_inside(&self.path) {
            return Err(say!("dialogs-submodule-folder-outside").to_owned());
        }
        if submodules.contains(&self.path) {
            return Err(say!("dialogs-submodule-exists", path = self.path.as_str()));
        }
        if self.branch.starts_with('-') || self.branch.chars().any(char::is_whitespace) {
            return Err(say!(
                "dialogs-submodule-branch-refused",
                branch = format!("{:?}", self.branch)
            ));
        }
        if self.path.is_empty() && !self.url.is_empty() {
            let usual = Self::usual_folder(&self.url);
            if !usual.is_empty() {
                return Ok(Some(say!("dialogs-submodule-folder-unnamed", usual)));
            }
        }
        if self.url.is_empty() || self.path.is_empty() {
            return Ok(None);
        }
        Ok(Some(if self.branch.is_empty() {
            say!(
                "dialogs-submodule-cloned-default",
                path = self.path.as_str()
            )
        } else {
            say!(
                "dialogs-submodule-cloned-branch",
                path = self.path.as_str(),
                branch = self.branch.as_str()
            )
        }))
    }
}

/// What the switch of the removal says it deletes.
pub(super) fn purge_label(name: &str) -> String {
    say!("dialogs-submodule-purge", name)
}

/// What is said once a submodule is removed.
pub(super) fn removed_text(path: &str, purge: bool) -> String {
    if purge {
        say!("dialogs-submodule-removed-purged", path)
    } else {
        say!("dialogs-submodule-removed", path)
    }
}

/// Whether adding was refused because a clone of an earlier submodule is
/// still kept at that place.
pub(super) fn refuses_kept_clone(err: &str) -> bool {
    err.contains("still keeps a clone")
}

impl Workspace {
    /// The paths of the submodules there are.
    pub(super) fn submodule_paths(&self) -> Vec<String> {
        self.snapshot
            .as_ref()
            .map(|snapshot| {
                snapshot
                    .submodules
                    .iter()
                    .map(|submodule| submodule.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Add a submodule. Git's refusal to clone from a local path, and a
    /// clone that is still kept of an earlier submodule there, are each
    /// asked about and then gone on from.
    pub(super) fn add_submodule(
        &mut self,
        adding: Adding,
        allow_local: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let again = adding.clone();
        self.run_operation_with(
            "submodule.add",
            adding.params(allow_local),
            say!("dialogs-submodule-adding"),
            match adding.branch.as_str() {
                "" => say!("dialogs-submodule-added", path = adding.path.as_str()),
                branch => say!(
                    "dialogs-submodule-added-on",
                    path = adding.path.as_str(),
                    branch
                ),
            },
            move |this, err, window, cx| {
                if refuses_kept_clone(err) {
                    this.ask_forget_clone(again.clone(), allow_local, window, cx);
                    return true;
                }
                if allow_local || !refuses_local_clone(err) {
                    return false;
                }
                let adding = again.clone();
                this.ask_local_clone(
                    move |this, window, cx| this.add_submodule(adding.clone(), true, window, cx),
                    window,
                    cx,
                );
                true
            },
            window,
            cx,
        );
    }

    /// A clone of a submodule that was removed is in the way of the new
    /// one: ask whether it goes, delete it and add again.
    fn ask_forget_clone(
        &mut self,
        adding: Adding,
        allow_local: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let path = adding.path.clone();
        self.ask_in_bar(
            say!("dialogs-submodule-old-clone-question", path),
            say!("dialogs-delete-it"),
            true,
            move |this, window, cx| {
                this.forget_clone_then_add(adding.clone(), allow_local, window, cx)
            },
            window,
            cx,
        );
    }

    fn forget_clone_then_add(
        &mut self,
        adding: Adding,
        allow_local: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.refused_here("submodule.forget", window, cx) {
            return;
        }
        let reply = self.session.send(Request::Command {
            id: "submodule.forget".to_owned(),
            params: json!({ "name": adding.path }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Reply::Error(err) => {
                    let notice = this.failed_notice(&err, cx);
                    crate::notices::push(window, cx, notice);
                }
                _ => this.add_submodule(adding, allow_local, window, cx),
            });
        })
        .detach();
    }

    /// Ask whether the submodule at `path` is to go, and whether the
    /// clone git keeps of it goes with it.
    pub(super) fn prompt_remove_submodule(
        &mut self,
        path: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The clone is kept under the submodule's name, which is its
        // path unless it was moved or named by hand.
        let name = self
            .snapshot
            .as_ref()
            .and_then(|snapshot| {
                snapshot
                    .submodules
                    .iter()
                    .find(|submodule| submodule.path == path)
            })
            .map(|submodule| submodule.name.clone())
            .unwrap_or_else(|| path.clone());
        let mut form = Form::new(
            say!("dialogs-submodule-remove-title", path = path.as_str()),
            say!("workspace-remove"),
        );
        form.lead = Some(say!("dialogs-submodule-remove-lead").to_owned());
        form.checks = vec![Check::new(purge_label(&name), false)];
        self.form(
            form,
            move |this, answers, window, cx| {
                this.remove_submodule(path.clone(), false, answers.check(0), window, cx);
            },
            window,
            cx,
        );
    }

    /// Remove a submodule; one with work in it is asked about once more.
    pub(super) fn remove_submodule(
        &mut self,
        path: String,
        force: bool,
        purge: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let again = path.clone();
        self.run_mutation(
            "submodule.remove",
            json!({ "path": path, "force": force, "purge": purge }),
            removed_text(&path, purge),
            move |this, err, window, cx| {
                if force || !err.contains("would be lost") {
                    return false;
                }
                let path = again.clone();
                this.ask_in_bar(
                    say!(
                        "dialogs-submodule-remove-anyway-question",
                        path = path.as_str()
                    ),
                    say!("workspace-remove"),
                    true,
                    move |this, window, cx| {
                        this.remove_submodule(path.clone(), true, purge, window, cx)
                    },
                    window,
                    cx,
                );
                true
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{Adding, purge_label, refuses_kept_clone, removed_text};

    fn adding(url: &str, path: &str, branch: &str) -> Adding {
        Adding {
            url: url.to_owned(),
            path: path.to_owned(),
            branch: branch.to_owned(),
        }
    }

    #[test]
    fn a_branch_is_sent_only_when_one_is_named() {
        assert_eq!(
            adding("https://example.invalid/lib.git", "deps/lib", "").params(false),
            json!({
                "url": "https://example.invalid/lib.git", "path": "deps/lib",
                "allow_local": false,
            })
        );
        assert_eq!(
            adding("../lib", "deps/lib", "stable").params(true),
            json!({ "url": "../lib", "path": "deps/lib", "allow_local": true, "branch": "stable" })
        );
    }

    #[test]
    fn what_is_typed_for_a_submodule_is_looked_at() {
        let there = ["deps/lib".to_owned()];
        assert_eq!(adding("", "", "").reviewed(&there), Ok(None));
        // A folder is suggested from where it comes from.
        assert_eq!(
            adding("git@example.invalid:team/parser.git", "", "").reviewed(&there),
            Ok(Some(
                "A folder for it is still to be named: parser, for one.".to_owned()
            ))
        );
        assert_eq!(
            adding("../lib", "deps/lib", "").reviewed(&there),
            Err("deps/lib is a submodule already.".to_owned())
        );
        for outside in ["/tmp/x", "../x", "a/../../x"] {
            assert_eq!(
                adding("../lib", outside, "").reviewed(&there),
                Err("The folder has to be inside this repository.".to_owned())
            );
        }
        for bad in ["-b", "two words"] {
            assert!(adding("../lib", "deps/new", bad).reviewed(&there).is_err());
        }
        assert_eq!(
            adding("../lib", "deps/new", "").reviewed(&there),
            Ok(Some(
                "Cloned into deps/new at the default branch of where it comes from; that is \
                 staged."
                    .to_owned()
            ))
        );
        assert_eq!(
            adding("../lib", "deps/new", "stable").reviewed(&there),
            Ok(Some(
                "Cloned into deps/new at stable, which it is set to follow; that is staged."
                    .to_owned()
            ))
        );
    }

    #[test]
    fn the_usual_folder_is_the_last_part_of_the_address() {
        for (url, folder) in [
            ("https://example.invalid/team/parser.git", "parser"),
            ("git@example.invalid:team/parser.git", "parser"),
            ("git@example.invalid:parser", "parser"),
            ("../lib/", "lib"),
            ("/srv/git/lib.git/", "lib"),
            ("", ""),
        ] {
            assert_eq!(Adding::usual_folder(url), folder, "{url:?}");
        }
    }

    #[test]
    fn the_removal_says_what_goes() {
        assert!(purge_label("deps/lib").contains(".git/modules/deps/lib"));
        assert_eq!(
            removed_text("deps/lib", false),
            "Removed the submodule deps/lib; that is staged"
        );
        assert!(removed_text("deps/lib", true).contains("and the clone git kept of it"));
        assert!(refuses_kept_clone(
            "submodule: git still keeps a clone of a submodule that was at deps/lib; remove it \
             for good first"
        ));
        assert!(!refuses_kept_clone(
            "submodule: deps/lib is a submodule already"
        ));
    }
}
