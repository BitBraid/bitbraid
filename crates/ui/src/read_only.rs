//! A repository that is opened read-only, and what the interface leaves
//! out of it.
//!
//! A bare repository is read-only in the engine: `Registry::run`
//! refuses every command that could change it. This module is the one
//! place where the interface learns the same, so that it does not offer
//! what would only fail: which commands and which keys are refused comes
//! from the engine's own table ([`bitbraid_engine::access`]), every menu
//! of such a repository starts from [`menu`], and the words that say it
//! are here.
//!
//! Sources: gitrepository-layout(5), gitglossary(7) ("bare repository");
//! `crates/engine/src/access.rs`.

use bitbraid_engine::{Effect, access};
use gpui_kit::component::menu::PopupMenu;

use bitbraid_strings::Message;

use crate::keymap::Binding;

/// What a read-only repository is headed with while it is open.
pub const HEADING: Message = message!("app-read-only-heading");
/// And why, with the way out.
pub const EXPLANATION: Message = message!("app-read-only-explanation");
/// The same in a line, for a menu, a tooltip or a notice.
pub const IN_SHORT: Message = message!("app-read-only-in-short");

/// Keys of the window that change the repository without naming the one
/// engine command they run (they ask first, or run several).
const CHANGING_KEYS: &[&str] = &[
    "commit.all",
    "ignore.pattern",
    "ignore.untrack",
    "patch.apply",
    "patch.mailbox",
    "rerere.toggle",
];

/// Whether the engine refuses this command in a read-only repository. As
/// there, a command that is not known is taken to change it. The
/// registry is asked, so that a command of a higher layer (a provider's)
/// is judged by what it brought.
pub fn refuses(command: &str) -> bool {
    bitbraid_integrations::shared()
        .access(command)
        .unwrap_or_else(|| access::of(command))
        .effect
        != Effect::Reads
}

/// Whether what a key is bound to would be refused in a read-only
/// repository.
pub fn refuses_binding(binding: &Binding) -> bool {
    binding.command.is_some_and(refuses) || CHANGING_KEYS.contains(&binding.id)
}

/// The start of every menu in a read-only repository: a line that says
/// why the rest is missing. What only changes the view, or copies
/// something out, is added after it by the caller.
pub fn menu(menu: PopupMenu) -> PopupMenu {
    menu.label(IN_SHORT.text())
}

#[cfg(test)]
mod tests {
    use super::{CHANGING_KEYS, refuses, refuses_binding};
    use crate::keymap::{self, BINDINGS};

    #[test]
    fn what_changes_a_repository_is_refused_and_what_reads_it_is_not() {
        for command in [
            "branch.create",
            "branch.checkout",
            "remote.fetch",
            "remote.push",
            "stash.push",
            "undo.last",
            "commit.create",
            "pr.checkout",
            "issue.branch",
            "no.such_command",
        ] {
            assert!(refuses(command), "{command}");
        }
        for command in [
            "repo.open_folder",
            "repo.open_terminal",
            "branch.tracking",
            "search.commits",
            "activity.list",
            "pr.list",
            "account.list",
        ] {
            assert!(!refuses(command), "{command}");
        }
    }

    #[test]
    fn keys_that_change_the_repository_are_known() {
        let refused: Vec<&str> = BINDINGS
            .iter()
            .filter(|binding| refuses_binding(binding))
            .map(|binding| binding.id)
            .collect();
        for id in [
            "remote.fetch",
            "remote.pull",
            "remote.push",
            "branch.create",
            "stash.push",
            "stash.pop",
            "commit.create",
            "commit.all",
            "undo.last",
            "redo.last",
            "patch.apply",
            "patch.mailbox",
            "rerere.toggle",
        ] {
            assert!(refused.contains(&id), "{id} is not refused");
        }
        // What only looks, moves about or belongs to the window stays.
        for id in [
            "palette.open",
            "repo.open",
            "tab.close",
            "view.refresh",
            "search.commits",
            "graph.jump",
            "repo.open_folder",
            "view.activity",
            "patch.save",
            "graph.select_next",
            "app.quit",
        ] {
            assert!(!refused.contains(&id), "{id} is refused");
        }
        // Every key named here exists.
        for id in CHANGING_KEYS {
            assert!(keymap::binding(id).is_some(), "{id}");
        }
    }
}
