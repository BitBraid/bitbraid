//! Confirmations asked in the bar that takes the toolbar's place
//! (`crate::ask`): removing a remote, a worktree or a submodule, deleting
//! a branch or a tag here or on a remote, dropping a stash or a commit,
//! resetting a branch, finishing a gitflow branch. Those that list what
//! would be lost are dialogs.
//!
//! Sources: `crates/ui/src/ask.rs`.

use gpui_kit::*;

use super::Workspace;

impl Workspace {
    /// Ask in the bar whether to go ahead: `said` is the question,
    /// `label` its button, in the colour of danger for what cannot be had
    /// back but by undo. Giving up does nothing.
    pub(super) fn ask_in_bar(
        &mut self,
        said: impl Into<SharedString>,
        label: &'static str,
        danger: bool,
        run: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workspace = cx.entity().downgrade();
        let go = move |_: Vec<String>, window: &mut Window, cx: &mut App| {
            let _ = workspace.update(cx, |this, cx| run(this, window, cx));
        };
        let ask = crate::ask::Ask::new(said);
        let ask = if danger {
            ask.danger(label, go)
        } else {
            ask.go(label, go)
        };
        crate::ask::show(ask, window, cx);
    }
}
