//! The dialog that stashes: a message, and what the stash takes.
//!
//! Sources: git-stash(1) (`--include-untracked`, `--keep-index`,
//! `--staged`).

use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    PromptField, Workspace,
    form::{Check, Form},
};

/// The parameters of `stash.push` for what the dialog says. Only what is
/// staged is a stash of its own kind: git takes neither untracked files
/// with it nor leaves the index as it is, so the other two do not apply.
pub(super) fn stash_params(
    message: &str,
    untracked: bool,
    keep_index: bool,
    staged_only: bool,
) -> Value {
    if staged_only {
        json!({ "message": message, "staged_only": true })
    } else {
        json!({
            "message": message,
            "include_untracked": untracked,
            "keep_index": keep_index,
        })
    }
}

/// What is said once the stash is made.
pub(super) fn stashed_text(untracked: bool, keep_index: bool, staged_only: bool) -> String {
    match (staged_only, keep_index, untracked) {
        (true, _, _) => say!("workspace-stashed-staged").to_owned(),
        (false, true, true) => say!("dialogs-stashed-keep-index-untracked").to_owned(),
        (false, true, false) => say!("dialogs-stashed-keep-index").to_owned(),
        (false, false, true) => say!("dialogs-stashed-untracked").to_owned(),
        (false, false, false) => say!("dialogs-stashed").to_owned(),
    }
}

impl Workspace {
    /// Stash every local change, untracked files too, without asking: what
    /// the toolbar's Stash does. git names the stash.
    pub(super) fn stash_now(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_mutation(
            "stash.push",
            stash_params("", true, false, false),
            stashed_text(true, false, false),
            |_, _, _, _| false,
            window,
            cx,
        );
    }

    /// Ask for a message and for what the stash takes, and stash.
    pub(super) fn prompt_stash(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut form = Form::new(say!("dialogs-stash-title"), say!("dialogs-stash"));
        // The message is optional: git names the stash otherwise.
        form.fields = vec![PromptField::optional(say!(
            "workspace-prompt-message-optional"
        ))];
        form.checks = vec![
            Check::new(say!("dialogs-stash-untracked"), true),
            Check::new(say!("dialogs-stash-keep-index"), false),
            Check::new(say!("dialogs-stash-staged-only"), false),
        ];
        self.form(
            form,
            |this, answers, window, cx| {
                let (untracked, keep_index, staged_only) =
                    (answers.check(0), answers.check(1), answers.check(2));
                this.run_mutation(
                    "stash.push",
                    stash_params(&answers.text(0), untracked, keep_index, staged_only),
                    stashed_text(untracked, keep_index, staged_only),
                    |_, _, _, _| false,
                    window,
                    cx,
                );
            },
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{stash_params, stashed_text};

    #[test]
    fn the_switches_become_the_engines_parameters() {
        assert_eq!(
            stash_params("wip", true, false, false),
            json!({ "message": "wip", "include_untracked": true, "keep_index": false })
        );
        assert_eq!(
            stash_params("", false, true, false),
            json!({ "message": "", "include_untracked": false, "keep_index": true })
        );
        // Only what is staged: the other two are not sent, which git
        // would refuse together with it.
        assert_eq!(
            stash_params("half", true, true, true),
            json!({ "message": "half", "staged_only": true })
        );
        assert!(stashed_text(true, true, true).starts_with("Stashed what was staged"));
        assert!(stashed_text(false, false, false).ends_with("untracked files stay"));
        assert!(stashed_text(true, true, false).contains("still staged"));
    }
}
