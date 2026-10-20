//! Leaving out the hooks that could refuse a merge, a rebase, a push or a
//! mailbox of patches, as the commit box leaves out a commit's.
//!
//! The switch is where each of them is started and is there only while
//! such a hook is: a line to tick in the dialogs that ask how to merge
//! and what to rebase, in the dialog that takes a mailbox and beside the
//! plan of an interactive rebase; an entry with a mark in the menu beside
//! Push. The four that are ticked hold for the one operation they are
//! ticked for. The one for pushing has no dialog to be in: it stays on
//! while the tab is open, the button and the menu's entries say so for as
//! long, and it can be switched from the palette. None of them is written
//! anywhere: a new start runs every hook again.
//!
//! What runs without its hooks says so while it runs ("Merging without
//! hooks") and when it is done, with the hooks the engine names as left
//! out. A merge that stops on a conflict is concluded by `git commit`,
//! which runs the hooks of a commit: nothing here changes that.
//!
//! Sources: githooks(5); git-merge(1), git-rebase(1), git-push(1),
//! git-am(1) (`--no-verify` of each, and which hooks it leaves out).

use bitbraid_git::hooks::Unverified;
use gpui_kit::*;
use serde_json::Value;

use super::Workspace;
use crate::{notices::Notice, views::commit_box::name_list};

/// What kind of command `id` is, when it is one that can go without the
/// hooks that could refuse it.
pub(crate) fn kind_of(id: &str) -> Option<Unverified> {
    match id {
        "branch.merge" => Some(Unverified::Merge),
        "branch.rebase" | "rebase.interactive" => Some(Unverified::Rebase),
        "remote.push" => Some(Unverified::Push),
        "patch.mailbox" => Some(Unverified::Mailbox),
        _ => None,
    }
}

/// Of the hooks that are there, the ones a command of this kind leaves
/// out when asked to, in git's order.
pub(crate) fn there(kind: Unverified, present: &[String]) -> Vec<String> {
    kind.hooks()
        .iter()
        .filter(|name| present.iter().any(|there| there == *name))
        .map(|name| (*name).to_owned())
        .collect()
}

/// What the switch says: which hooks it is about.
pub(crate) fn switch_label(there: &[String]) -> String {
    say!(
        "workspace-hooks-switch",
        hooks = name_list(there),
        count = there.len()
    )
}

/// The parameters of a command, with the hooks left out.
pub(crate) fn without(mut params: Value) -> Value {
    if let Some(fields) = params.as_object_mut() {
        fields.insert("no_verify".to_owned(), Value::Bool(true));
    }
    params
}

/// Whether the parameters of a command leave its hooks out.
pub(crate) fn asked(id: &str, params: &Value) -> bool {
    kind_of(id).is_some() && params.get("no_verify").and_then(Value::as_bool) == Some(true)
}

/// What an operation is called while it runs without its hooks.
pub(crate) fn running(label: &str) -> String {
    say!("workspace-hooks-running", label)
}

/// What is said when it is done: with the hooks the command's answer
/// names as left out. None when it left none out.
pub(crate) fn skipped_said(done: &str, answer: &Value) -> Option<String> {
    let skipped: Vec<String> = answer
        .get("skipped_hooks")?
        .as_array()?
        .iter()
        .filter_map(|name| name.as_str().map(str::to_owned))
        .collect();
    if skipped.is_empty() {
        return None;
    }
    Some(say!(
        "workspace-hooks-skipped",
        done,
        hooks = name_list(&skipped)
    ))
}

impl Workspace {
    /// The hooks that are there and that a command of this kind leaves
    /// out when asked to: empty when there is nothing to switch.
    pub(crate) fn hooks_there(&self, kind: Unverified, cx: &App) -> Vec<String> {
        there(kind, self.commit_box.read(cx).hooks_present())
    }

    /// Whether pushes go without the `pre-push` hook: switched on, and
    /// the hook is there. None when there is no such hook to switch.
    pub fn push_hook_skipped(&self, cx: &App) -> Option<bool> {
        (!self.hooks_there(Unverified::Push, cx).is_empty()).then_some(self.skip_push_hook)
    }

    /// Switch pushing without the `pre-push` hook on or off. It holds
    /// while the tab is open and is written nowhere.
    pub fn toggle_push_hook(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.hooks_there(Unverified::Push, cx).is_empty() {
            self.skip_push_hook = false;
            crate::notices::push(window, cx, Notice::info(say!("workspace-push-hook-none")));
            return;
        }
        self.skip_push_hook = !self.skip_push_hook;
        let notice = if self.skip_push_hook {
            Notice::warning(say!("workspace-push-hook-skipped"))
        } else {
            Notice::info(say!("workspace-push-hook-again"))
        };
        crate::notices::push(window, cx, notice);
        cx.notify();
    }

    /// The parameters a command is sent with: a push while its hook is
    /// switched off goes without it.
    pub(super) fn with_switches(&self, id: &str, params: Value, cx: &App) -> Value {
        if id == "remote.push" && self.push_hook_skipped(cx) == Some(true) {
            without(params)
        } else {
            params
        }
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::hooks::Unverified;
    use serde_json::json;

    use super::{asked, kind_of, running, skipped_said, switch_label, there, without};

    fn names(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_commands_that_can_go_without_their_hooks_are_known() {
        assert_eq!(kind_of("branch.merge"), Some(Unverified::Merge));
        assert_eq!(kind_of("branch.rebase"), Some(Unverified::Rebase));
        assert_eq!(kind_of("rebase.interactive"), Some(Unverified::Rebase));
        assert_eq!(kind_of("remote.push"), Some(Unverified::Push));
        assert_eq!(kind_of("patch.mailbox"), Some(Unverified::Mailbox));
        // The commit box has a switch of its own; the rest have none.
        for other in [
            "commit.create",
            "remote.pull",
            "patch.apply",
            "tag.push",
            "",
        ] {
            assert_eq!(kind_of(other), None, "{other}");
        }
    }

    #[test]
    fn the_switch_is_about_the_hooks_that_are_there() {
        let present = names(&["commit-msg", "pre-push", "post-merge", "applypatch-msg"]);
        assert_eq!(there(Unverified::Merge, &present), ["commit-msg"]);
        assert_eq!(there(Unverified::Push, &present), ["pre-push"]);
        assert_eq!(there(Unverified::Mailbox, &present), ["applypatch-msg"]);
        assert!(there(Unverified::Rebase, &present).is_empty());
        assert!(there(Unverified::Merge, &[]).is_empty());
        // In git's order, whatever order they were listed in.
        let both = names(&["commit-msg", "pre-merge-commit"]);
        assert_eq!(
            there(Unverified::Merge, &both),
            ["pre-merge-commit", "commit-msg"]
        );
        assert_eq!(
            switch_label(&there(Unverified::Merge, &both)),
            "Skip the pre-merge-commit and commit-msg hooks, which could refuse it"
        );
        assert_eq!(
            switch_label(&names(&["pre-push"])),
            "Skip the pre-push hook, which could refuse it"
        );
    }

    #[test]
    fn a_command_is_sent_without_its_hooks_and_says_so() {
        let params = without(json!({ "target": "topic" }));
        assert_eq!(params, json!({ "target": "topic", "no_verify": true }));
        assert!(asked("branch.merge", &params));
        assert!(!asked("branch.merge", &json!({ "target": "topic" })));
        assert!(!asked(
            "branch.merge",
            &json!({ "target": "topic", "no_verify": false })
        ));
        assert!(!asked("branch.merge", &json!({ "no_verify": "yes" })));
        // Of another command it is not this switch.
        assert!(!asked("commit.create", &json!({ "no_verify": true })));
        // What is no object stays what it is.
        assert_eq!(without(json!(null)), json!(null));
        assert_eq!(without(json!([1])), json!([1]));
        assert_eq!(running("Merging"), "Merging without hooks");
    }

    #[test]
    fn what_was_left_out_is_said_when_it_is_done() {
        let said = |answer| skipped_said("Merged topic", &answer);
        assert_eq!(
            said(json!({ "skipped_hooks": ["pre-merge-commit", "commit-msg"] })).as_deref(),
            Some("Merged topic, without running pre-merge-commit and commit-msg")
        );
        assert_eq!(
            said(json!({ "skipped_hooks": ["pre-push"] })).as_deref(),
            Some("Merged topic, without running pre-push")
        );
        // Asked for with no such hook there, not asked for, and answers
        // that are not as expected.
        assert_eq!(said(json!({ "skipped_hooks": [] })), None);
        assert_eq!(said(json!({})), None);
        assert_eq!(said(json!({ "skipped_hooks": "pre-push" })), None);
        assert_eq!(said(json!({ "skipped_hooks": [1, null] })), None);
        assert_eq!(said(json!(null)), None);
    }
}
