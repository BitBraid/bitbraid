//! Stash requests: the menus of the graph and of the references panel and
//! the toolbar make them, the workspace asks where it has to and runs them.
//! A stash is named by its commit id, which stays the same while positions
//! in the list shift.
//!
//! Sources: gpui-kit documentation (popup menu).

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum StashRequest {
    /// Set the local changes aside, after asking for a message.
    Push,
    /// Set every local change aside, untracked files too, without asking.
    PushNow,
    /// Bring a stash's changes back and keep it. None is the newest.
    Apply(Option<String>),
    /// Bring a stash's changes back and drop it. None is the newest.
    Pop(Option<String>),
    /// Drop a stash, after asking.
    Drop(String),
    /// Ask for a name, create that branch where the stash was made and pop
    /// the stash there.
    Branch(String),
    /// Ask for another message for a stash, and give it that one.
    Reword(String),
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = stash, no_json)]
pub struct RunStash {
    /// What to do.
    pub request: StashRequest,
}

fn entry(request: StashRequest) -> Box<dyn Action> {
    Box::new(RunStash { request })
}

/// Menu entries for one stash, named by its commit id: apply, pop and
/// drop, then popping it on a new branch.
pub fn stash_entries(menu: PopupMenu, stash: &str) -> PopupMenu {
    let id = || stash.to_owned();
    menu.menu(
        say!("stashes-apply"),
        entry(StashRequest::Apply(Some(id()))),
    )
    .menu(say!("stashes-pop"), entry(StashRequest::Pop(Some(id()))))
    .menu(say!("stashes-drop"), entry(StashRequest::Drop(id())))
    .menu(
        say!("stashes-pop-on-branch"),
        entry(StashRequest::Branch(id())),
    )
}

/// The entry that gives a stash another message.
pub fn reword_entry(menu: PopupMenu, stash: &str) -> PopupMenu {
    menu.menu(
        say!("stashes-edit-message"),
        entry(StashRequest::Reword(stash.to_owned())),
    )
}

/// The menu entry of the row of uncommitted work.
pub fn push_entry(menu: PopupMenu) -> PopupMenu {
    menu.menu(say!("stashes-push"), entry(StashRequest::Push))
}
