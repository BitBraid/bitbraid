//! The menu bar of macOS: the application's menu, File, Edit, View,
//! Repository, Window and Help, each entry one of the window's actions, so
//! that the keys a menu shows are the keys in force (they are set again
//! when the keys change). Elsewhere the window has no menu bar and nothing
//! here is installed.
//!
//! Edit's entries are those of the text field that has the keys: undoing
//! there undoes typing, never a git action, which is Repository's. Entries
//! nothing can do at the moment are greyed by the system.
//!
//! The system's own entries stand where macOS has them: Services, hiding,
//! the window list (in Window, which the system fills in), and entering
//! full screen (which it adds to View).
//!
//! Sources: gpui's `Menu`, `MenuItem`, `OsAction`, `SystemMenuType`,
//! `App::set_menus`, `App::hide`, `App::hide_other_apps`,
//! `App::unhide_other_apps`, `Window::minimize_window`,
//! `Window::zoom_window`; Apple's Human Interface
//! Guidelines, "The menu bar" (the menus every application has, their
//! order, and what each holds) and "Mac keyboard shortcuts" (Cmd+H,
//! Option+Cmd+H, Cmd+M).

use gpui_kit::{
    Action, App, KeyBinding, Menu, MenuItem, OsAction, SystemMenuType, actions, component::input,
};

use crate::{brand, keymap};

actions!(
    bitbraid,
    [
        HideApplication,
        HideOthers,
        ShowAll,
        MinimizeWindow,
        ZoomWindow
    ]
);

/// Whether the system has a menu bar for the application.
pub const MENU_BAR: bool = cfg!(target_os = "macos");

/// How an entry's action is made.
type MakeAction = fn() -> Box<dyn Action>;

/// The keys of what the system's menus do the same in every application:
/// hiding and minimising.
const SYSTEM_KEYS: [(&str, MakeAction); 3] = [
    ("cmd-h", || Box::new(HideApplication)),
    ("cmd-alt-h", || Box::new(HideOthers)),
    ("cmd-m", || Box::new(MinimizeWindow)),
];

/// Install the menu bar, what its own entries do, and their keys.
pub fn init(cx: &mut App) {
    if !MENU_BAR {
        return;
    }
    cx.on_action(|_: &HideApplication, cx| cx.hide());
    cx.on_action(|_: &HideOthers, cx| cx.hide_other_apps());
    cx.on_action(|_: &ShowAll, cx| cx.unhide_other_apps());
    // What is done to the window waits until the action is through: while
    // it is dispatched the window is busy, and could not be changed.
    cx.on_action(|_: &MinimizeWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, _| window.minimize_window());
            }
        });
    });
    cx.on_action(|_: &ZoomWindow, cx| {
        cx.defer(|cx| {
            if let Some(window) = cx.active_window() {
                let _ = window.update(cx, |_, window, _| window.zoom_window());
            }
        });
    });
    let keys: Vec<KeyBinding> = SYSTEM_KEYS
        .iter()
        .filter_map(|(key, action)| {
            KeyBinding::load(
                key,
                action(),
                None,
                false,
                None,
                &gpui_kit::DummyKeyboardMapper,
            )
            .ok()
        })
        .collect();
    cx.bind_keys(keys);
    refresh(cx);
}

/// Set the menus again, with the keys in force now.
pub fn refresh(cx: &mut App) {
    if MENU_BAR {
        cx.set_menus(menus());
    }
}

/// An entry that does one of the window's actions.
fn entry(name: &str, action: impl Action) -> MenuItem {
    MenuItem::action(name.to_owned(), action)
}

/// The menus, in the order the bar shows them.
pub fn menus() -> Vec<Menu> {
    vec![
        Menu::new(brand::NAME).items([
            entry(say!("menus-about"), keymap::ShowAbout),
            MenuItem::separator(),
            entry(say!("menus-settings"), keymap::ShowPreferences),
            entry(say!("menus-keyboard-shortcuts"), keymap::ShowShortcuts),
            MenuItem::separator(),
            MenuItem::os_submenu(say!("menus-services"), SystemMenuType::Services),
            MenuItem::separator(),
            entry(say!("menus-hide"), HideApplication),
            entry(say!("menus-hide-others"), HideOthers),
            entry(say!("menus-show-all"), ShowAll),
            MenuItem::separator(),
            entry(say!("menus-quit"), keymap::Quit),
        ]),
        Menu::new(say!("menus-file")).items([
            entry(say!("menus-new-tab"), keymap::NewTab),
            entry(say!("menus-open-repository"), keymap::OpenRepository),
            entry(say!("menus-clone-repository"), keymap::CloneRepository),
            entry(say!("menus-new-repository"), keymap::InitRepository),
            MenuItem::separator(),
            entry(
                say!("menus-repositories-and-workspaces"),
                keymap::ShowRepositories,
            ),
            MenuItem::separator(),
            entry(say!("menus-close-tab"), keymap::CloseTab),
        ]),
        Menu::new(say!("menus-edit")).items([
            MenuItem::os_action(say!("menus-undo"), input::Undo, OsAction::Undo),
            MenuItem::os_action(say!("menus-redo"), input::Redo, OsAction::Redo),
            MenuItem::separator(),
            MenuItem::os_action(say!("menus-cut"), input::Cut, OsAction::Cut),
            MenuItem::os_action(say!("menus-copy"), input::Copy, OsAction::Copy),
            MenuItem::os_action(say!("menus-paste"), input::Paste, OsAction::Paste),
            MenuItem::os_action(
                say!("menus-select-all"),
                input::SelectAll,
                OsAction::SelectAll,
            ),
            MenuItem::separator(),
            entry(say!("menus-find"), keymap::FocusSearch),
        ]),
        Menu::new(say!("menus-view")).items([
            entry(say!("menus-command-palette"), keymap::TogglePalette),
            MenuItem::separator(),
            entry(say!("menus-references-panel"), keymap::ToggleLeftPanel),
            entry(say!("menus-details-panel"), keymap::ToggleRightPanel),
            entry(say!("menus-terminal"), keymap::ToggleTerminal),
            MenuItem::separator(),
            entry(say!("menus-activity-log"), keymap::ShowActivity),
            entry(say!("menus-undo-history"), keymap::ShowUndoHistory),
            entry(say!("menus-notifications"), keymap::ShowNotifications),
            MenuItem::separator(),
            entry(say!("menus-next-theme"), keymap::CycleTheme),
            MenuItem::separator(),
            entry(say!("menus-zoom-in"), keymap::ZoomIn),
            entry(say!("menus-zoom-out"), keymap::ZoomOut),
            entry(say!("menus-actual-size"), keymap::ZoomReset),
        ]),
        Menu::new(say!("menus-repository")).items([
            entry(say!("menus-fetch"), keymap::Fetch),
            entry(say!("menus-pull"), keymap::Pull),
            entry(say!("menus-push"), keymap::Push),
            MenuItem::separator(),
            entry(say!("menus-new-branch"), keymap::NewBranch),
            entry(say!("menus-stash-changes"), keymap::StashChanges),
            entry(say!("menus-pop-stash"), keymap::PopStash),
            MenuItem::separator(),
            entry(say!("menus-undo-last-action"), keymap::Undo),
            entry(say!("menus-redo-last-action"), keymap::Redo),
            MenuItem::separator(),
            entry(say!("menus-show-in-finder"), keymap::OpenInFileManager),
            entry(say!("menus-open-in-terminal"), keymap::OpenInTerminal),
            entry(say!("menus-open-in-editor"), keymap::OpenInEditor),
            MenuItem::separator(),
            entry(say!("menus-refresh"), keymap::Refresh),
        ]),
        Menu::new(say!("menus-window")).items([
            entry(say!("menus-minimize"), MinimizeWindow),
            entry(say!("menus-zoom"), ZoomWindow),
            MenuItem::separator(),
            entry(say!("menus-show-next-tab"), keymap::NextTab),
            entry(say!("menus-show-previous-tab"), keymap::PrevTab),
        ]),
        Menu::new(say!("menus-help")).items([
            entry(say!("menus-keyboard-shortcuts"), keymap::ShowShortcuts),
            entry(say!("menus-command-palette"), keymap::TogglePalette),
        ]),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_menu_has_entries_and_the_bar_its_order() {
        let menus = menus();
        let names: Vec<&str> = menus.iter().map(|menu| menu.name.as_ref()).collect();
        assert_eq!(
            names,
            [
                brand::NAME,
                "File",
                "Edit",
                "View",
                "Repository",
                "Window",
                "Help"
            ]
        );
        for menu in &menus {
            assert!(!menu.items.is_empty(), "{}", menu.name);
            // No separator at either end, nor two in a row.
            let separators: Vec<bool> = menu
                .items
                .iter()
                .map(|item| matches!(item, MenuItem::Separator))
                .collect();
            assert!(
                !separators.first().copied().unwrap_or(true),
                "{}",
                menu.name
            );
            assert!(!separators.last().copied().unwrap_or(true), "{}", menu.name);
            assert!(
                !separators.windows(2).any(|pair| pair == [true, true]),
                "{}",
                menu.name
            );
        }
    }

    #[test]
    fn the_systems_keys_can_be_bound() {
        for (key, action) in SYSTEM_KEYS {
            assert!(
                KeyBinding::load(
                    key,
                    action(),
                    None,
                    false,
                    None,
                    &gpui_kit::DummyKeyboardMapper
                )
                .is_ok(),
                "{key}"
            );
        }
    }
}
