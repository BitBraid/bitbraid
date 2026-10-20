//! Actions, the default keys, and keys of the user's own.
//!
//! Everything a key can be bound to is one row of [`BINDINGS`]: a stable
//! id, what it does, where its keys apply, the keys it has by default and
//! the action it stands for. The same table feeds the palette and the
//! sheet of shortcuts.
//!
//! `keybindings.toml` in the configuration directory changes keys by id:
//!
//! ```toml
//! [keys]
//! "remote.pull" = "ctrl-alt-p"        # one key
//! "remote.push" = ["f9", "ctrl-u"]    # several
//! "theme.cycle" = []                  # none
//! ```
//!
//! A key is written as the toolkit writes it: modifiers `ctrl`, `alt`,
//! `shift`, `super` (`cmd` on macOS), then the key, joined by `-`; several
//! keys in a row (`ctrl-k ctrl-s`) are separated by a space. What the file
//! says wins over the defaults; two actions that end up on one key in one
//! place are a conflict, which is reported and settled (see [`resolve`]).
//!
//! The table holds the keys of Linux and Windows. On macOS a default has
//! Cmd where they have Ctrl, but for the keys the system keeps with Cmd
//! (see [`on_mac`]), and keys are shown with the signs macOS menus use.
//!
//! Sources: gpui's keymap documentation (`KeyBinding`, `Keystroke::parse`,
//! `Unbind`); Apple's "Mac keyboard shortcuts" (support.apple.com/HT201236:
//! what the system and every application use Cmd for, Ctrl+Cmd+F for full
//! screen) and the Human Interface Guidelines, "Keyboards" (the order of
//! the modifier signs).

use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

use bitbraid_strings::Message;
use gpui_kit::*;
use serde::Deserialize;

actions!(
    bitbraid,
    [
        NewTab,
        CloseTab,
        NextTab,
        PrevTab,
        OpenRepository,
        ToggleLeftPanel,
        ToggleRightPanel,
        FocusSearch,
        FocusGraph,
        SelectNext,
        SelectPrev,
        ExtendNext,
        ExtendPrev,
        SelectFirst,
        SelectLast,
        NextBranch,
        PrevBranch,
        PageDown,
        PageUp,
        TogglePalette,
        ShowShortcuts,
        ShowAbout,
        CycleTheme,
        Refresh,
        Undo,
        Redo,
        ShowActivity,
        ShowUndoHistory,
        ShowSigning,
        ShowProfiles,
        ShowIntegrations,
        ShowInbox,
        ShowRepositories,
        StartPullRequest,
        ShowPreferences,
        ShowNotifications,
        OpenInFileManager,
        OpenInEditor,
        OpenInTerminal,
        ShowLfs,
        ShowIndexFlags,
        ApplyPatch,
        ApplyMailbox,
        SaveChangesAsPatch,
        ToggleRerere,
        IgnorePattern,
        UntrackIgnored,
        AddCommitTrailer,
        ToggleWrap,
        CompareWithWorktree,
        SaveStagedAsPatch,
        ShowRerereRecords,
        ContinueOperation,
        SkipOperation,
        AbortOperation,
        RegisterLinks,
        ReloadKeys,
        CloneRepository,
        InitRepository,
        ShowTabList,
        ChangeKeys,
        ResetKeys,
        ToggleFullscreen,
        MoveTabLeft,
        MoveTabRight,
        RenameTab,
        JumpTo,
        ToggleGraphOrder,
        ToggleGraphDates,
        ToggleOnlyMatches,
        FindFile,
        CopyCommitId,
        ZoomIn,
        ZoomOut,
        ZoomReset,
        Fetch,
        Pull,
        Push,
        PushWithLease,
        ForcePush,
        SkipPushHook,
        PushAllTags,
        NewBranch,
        StashChanges,
        PopStash,
        CommitStaged,
        CommitAll,
        StageSelected,
        UnstageSelected,
        DiscardSelected,
        StageAll,
        UnstageAll,
        DiscardAll,
        SelectAll,
        OpenSelected,
        ToggleTreeView,
        NextChange,
        PrevChange,
        SaveFile,
        Quit,
        FilterReferences,
        FocusReferences,
        ExpandRow,
        CollapseRow,
        RevealRow,
        RowMenu,
        FlowSettings,
        FlowStart,
        FlowFinish,
        AllowSigner,
        ChooseSignersFile,
        TakeOurs,
        TakeTheirs,
        MarkResolved,
        ToggleSelected,
        WalkNext,
        WalkPrev,
        MovePinLeft,
        MovePinRight,
        ToggleTerminal,
        NewTerminal,
        NextTerminal,
        PrevTerminal,
        CloseTerminal,
        ShowMaintenance,
        CheckOutEverything,
        CopyText,
    ]
);

/// Jump to a tab by index.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = bitbraid, no_json)]
pub struct JumpToTab {
    /// Zero-based tab index.
    pub index: usize,
}

/// Key context of the graph list.
pub const GRAPH_CONTEXT: &str = "GraphView";

/// Key context of the rows of the references panel.
pub const REFS_CONTEXT: &str = "RefsView";

/// Key context of the changes panel.
pub const CHANGES_CONTEXT: &str = "ChangesView";

/// Key context of the diff viewer.
pub const DIFF_CONTEXT: &str = "DiffView";

/// Key context of the in-place editor.
pub const EDIT_CONTEXT: &str = "EditView";

/// Key context of the blame, and of the file as it was in a file's past.
pub const BLAME_CONTEXT: &str = "BlameView";

/// Where a key that would otherwise be typed, or would edit what is being
/// typed, does not apply: inside a text field.
const OUTSIDE_TEXT: &str = "!Input";

/// Whose keys: macOS has Cmd where the others have Ctrl, leaves to the
/// system what it takes with Cmd, types characters with Option, and writes
/// keys with signs of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    /// macOS.
    Mac,
    /// Linux, Windows and the rest.
    Other,
}

impl System {
    /// The system this build is for.
    pub const HERE: Self = if cfg!(target_os = "macos") {
        Self::Mac
    } else {
        Self::Other
    };

    /// Whose keys are in force: this system's. In this crate's own tests,
    /// the one the test says, Linux's and Windows' unless it says another.
    pub fn now() -> Self {
        #[cfg(test)]
        {
            tests::SYSTEM.with(std::cell::Cell::get)
        }
        #[cfg(not(test))]
        {
            Self::HERE
        }
    }
}

/// A key's modifiers and the key itself: `ctrl-shift-p` is `ctrl-shift`
/// and `p`; the key may be `-` (`ctrl--`).
fn split_stroke(stroke: &str) -> (&str, &str) {
    match stroke.strip_suffix("--") {
        Some(modifiers) => (modifiers, "-"),
        None => match stroke.rsplit_once('-') {
            Some((modifiers, name)) if !name.is_empty() => (modifiers, name),
            _ => ("", stroke),
        },
    }
}

/// A default key of the table as macOS has it: Cmd for Ctrl, but where the
/// system keeps Cmd with the key for itself (Cmd+Tab and Cmd+` switch
/// applications and windows) and where macOS has a key of its own for the
/// same thing (Ctrl+Cmd+F for full screen; Cmd+↑ and Cmd+↓ to the first
/// and the last, there being no Home and End on most of its keyboards).
pub fn on_mac(key: &str) -> String {
    key.split_whitespace()
        .map(|stroke| match stroke {
            "ctrl-tab" | "ctrl-shift-tab" | "ctrl-`" | "ctrl-~" | "ctrl-pageup"
            | "ctrl-pagedown" => stroke.to_owned(),
            "ctrl-shift-f" => "ctrl-cmd-f".to_owned(),
            "ctrl-home" => "cmd-up".to_owned(),
            "ctrl-end" => "cmd-down".to_owned(),
            _ => {
                let (modifiers, name) = split_stroke(stroke);
                if modifiers.is_empty() {
                    return stroke.to_owned();
                }
                let modifiers: Vec<&str> = modifiers
                    .split('-')
                    .map(|part| if part == "ctrl" { "cmd" } else { part })
                    .collect();
                format!("{}-{name}", modifiers.join("-"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The default keys of a row, as the system in force has them.
pub fn default_keys(binding: &Binding) -> Vec<String> {
    binding
        .keys
        .iter()
        .map(|key| match System::now() {
            System::Mac => on_mac(key),
            System::Other => (*key).to_owned(),
        })
        .collect()
}

/// A key as it is written on the system in force: `ctrl` of a key of the
/// table is `cmd` on macOS. For keys that are the same everywhere but for
/// Ctrl, written once (the text of a hint, a key of a view's own).
pub fn here(key: &str) -> String {
    match System::now() {
        System::Mac => on_mac(key),
        System::Other => key.to_owned(),
    }
}

/// How the action of a binding is made.
#[derive(Debug, Clone, Copy)]
pub enum Make {
    /// An action without parameters.
    Unit(fn() -> Box<dyn Action>),
    /// Go to the tab with this index.
    Tab(usize),
}

/// Something a key can be bound to.
#[derive(Debug, Clone, Copy)]
pub struct Binding {
    /// The name it has in `keybindings.toml`.
    pub id: &'static str,
    /// What it does.
    pub label: Message,
    /// Further words the palette finds it by; none for the keys of a
    /// single view, which the palette does not offer.
    pub keywords: Option<Message>,
    /// Where its keys apply: everywhere, or in one view.
    pub context: Option<&'static str>,
    /// The keys it has when the user gave it none.
    pub keys: &'static [&'static str],
    /// The engine command it runs, when it is a way to run one.
    pub command: Option<&'static str>,
    /// Whether the palette offers it: movements inside a list are not
    /// something to pick from a list.
    pub palette: bool,
    /// How its action is made.
    pub make: Make,
}

impl Binding {
    /// The action.
    pub fn action(&self) -> Box<dyn Action> {
        match self.make {
            Make::Unit(make) => make(),
            Make::Tab(index) => Box::new(JumpToTab { index }),
        }
    }

    /// The name of the view its keys apply in, for the sheet.
    pub fn place(&self) -> &'static str {
        match self.context {
            None => say!("keymap-place-everywhere"),
            Some(GRAPH_CONTEXT) => say!("keymap-place-graph"),
            Some(REFS_CONTEXT) => say!("keymap-place-references"),
            Some(CHANGES_CONTEXT) => say!("keymap-place-changes"),
            Some(DIFF_CONTEXT) => say!("keymap-place-diff"),
            Some(EDIT_CONTEXT) => say!("keymap-place-editor"),
            Some(BLAME_CONTEXT) => say!("keymap-place-blame"),
            Some(_) => say!("keymap-place-elsewhere"),
        }
    }
}

/// A row of the table: id, label, keywords, keys, action; optionally the
/// view it belongs to and the engine command it runs. A row of a view has
/// no keywords: the palette does not offer it.
macro_rules! bind {
    ($id:literal, $label:expr, $keywords:expr, [$($key:literal),*], $action:ident) => {
        bind!(@row $id, $label, Some($keywords), None, [$($key),*], None, true, $action)
    };
    ($id:literal, $label:expr, $keywords:expr, [$($key:literal),*], $action:ident, runs $command:literal) => {
        bind!(@row $id, $label, Some($keywords), None, [$($key),*], Some($command), true, $action)
    };
    ($id:literal, $label:expr, [$($key:literal),*], $action:ident, in $context:expr) => {
        bind!(@row $id, $label, None, Some($context), [$($key),*], None, false, $action)
    };
    (@row $id:literal, $label:expr, $keywords:expr, $context:expr, [$($key:literal),*], $command:expr, $palette:literal, $action:ident) => {
        Binding {
            id: $id,
            label: $label,
            keywords: $keywords,
            context: $context,
            keys: &[$($key),*],
            command: $command,
            palette: $palette,
            make: Make::Unit(|| Box::new($action)),
        }
    };
}

/// A row for one of the nine "go to tab" keys.
const fn tab(
    id: &'static str,
    label: Message,
    key: &'static [&'static str],
    index: usize,
) -> Binding {
    Binding {
        id,
        label,
        keywords: None,
        context: None,
        keys: key,
        command: None,
        palette: false,
        make: Make::Tab(index),
    }
}

/// Everything a key can be bound to, in the order the sheet lists it.
pub static BINDINGS: &[Binding] = &[
    // The window and its tabs.
    Binding {
        palette: false,
        ..bind!(
            "palette.open",
            message!("keymap-palette-open"),
            message!("keymap-palette-open-keywords"),
            ["ctrl-p"],
            TogglePalette
        )
    },
    bind!(
        "repo.open",
        message!("keymap-repo-open"),
        message!("keymap-repo-open-keywords"),
        ["ctrl-shift-o", "ctrl-o"],
        OpenRepository
    ),
    bind!(
        "tab.new",
        message!("keymap-tab-new"),
        message!("keymap-tab-new-keywords"),
        ["ctrl-t"],
        NewTab
    ),
    bind!(
        "repo.clone",
        message!("keymap-repo-clone"),
        message!("keymap-repo-clone-keywords"),
        ["ctrl-n"],
        CloneRepository
    ),
    bind!(
        "repo.init",
        message!("keymap-repo-init"),
        message!("keymap-repo-init-keywords"),
        ["ctrl-i"],
        InitRepository
    ),
    bind!(
        "tab.list",
        message!("keymap-tab-list"),
        message!("keymap-tab-list-keywords"),
        ["ctrl-shift-a"],
        ShowTabList
    ),
    bind!(
        "tab.close",
        message!("keymap-tab-close"),
        message!("keymap-tab-close-keywords"),
        ["ctrl-w"],
        CloseTab
    ),
    bind!(
        "tab.next",
        message!("keymap-tab-next"),
        message!("keymap-tab-next-keywords"),
        ["ctrl-tab"],
        NextTab
    ),
    bind!(
        "tab.previous",
        message!("keymap-tab-previous"),
        message!("keymap-tab-previous-keywords"),
        ["ctrl-shift-tab"],
        PrevTab
    ),
    bind!(
        "tab.move_left",
        message!("keymap-tab-move-left"),
        message!("keymap-tab-move-left-keywords"),
        ["ctrl-shift-pageup"],
        MoveTabLeft
    ),
    bind!(
        "tab.move_right",
        message!("keymap-tab-move-right"),
        message!("keymap-tab-move-right-keywords"),
        ["ctrl-shift-pagedown"],
        MoveTabRight
    ),
    bind!(
        "tab.rename",
        message!("keymap-tab-rename"),
        message!("keymap-tab-rename-keywords"),
        [],
        RenameTab
    ),
    tab("tab.go_1", message!("keymap-tab-go-1"), &["ctrl-1"], 0),
    tab("tab.go_2", message!("keymap-tab-go-2"), &["ctrl-2"], 1),
    tab("tab.go_3", message!("keymap-tab-go-3"), &["ctrl-3"], 2),
    tab("tab.go_4", message!("keymap-tab-go-4"), &["ctrl-4"], 3),
    tab("tab.go_5", message!("keymap-tab-go-5"), &["ctrl-5"], 4),
    tab("tab.go_6", message!("keymap-tab-go-6"), &["ctrl-6"], 5),
    tab("tab.go_7", message!("keymap-tab-go-7"), &["ctrl-7"], 6),
    tab("tab.go_8", message!("keymap-tab-go-8"), &["ctrl-8"], 7),
    tab("tab.go_9", message!("keymap-tab-go-9"), &["ctrl-9"], 8),
    bind!(
        "panel.references",
        message!("keymap-panel-references"),
        message!("keymap-panel-references-keywords"),
        ["ctrl-j"],
        ToggleLeftPanel
    ),
    bind!(
        "panel.details",
        message!("keymap-panel-details"),
        message!("keymap-panel-details-keywords"),
        ["ctrl-k"],
        ToggleRightPanel
    ),
    bind!(
        "view.refresh",
        message!("keymap-view-refresh"),
        message!("keymap-view-refresh-keywords"),
        ["f5"],
        Refresh
    ),
    bind!(
        "theme.cycle",
        message!("keymap-theme-cycle"),
        message!("keymap-theme-cycle-keywords"),
        ["alt-shift-t"],
        CycleTheme
    ),
    bind!(
        "zoom.in",
        message!("keymap-zoom-in"),
        message!("keymap-zoom-in-keywords"),
        ["ctrl-=", "ctrl-plus"],
        ZoomIn
    ),
    bind!(
        "zoom.out",
        message!("keymap-zoom-out"),
        message!("keymap-zoom-out-keywords"),
        ["ctrl--"],
        ZoomOut
    ),
    bind!(
        "zoom.reset",
        message!("keymap-zoom-reset"),
        message!("keymap-zoom-reset-keywords"),
        ["ctrl-0"],
        ZoomReset
    ),
    bind!(
        "view.fullscreen",
        message!("keymap-view-fullscreen"),
        message!("keymap-view-fullscreen-keywords"),
        ["ctrl-shift-f", "f11", "ctrl-alt-enter"],
        ToggleFullscreen
    ),
    bind!(
        "preferences.open",
        message!("keymap-preferences-open"),
        message!("keymap-preferences-open-keywords"),
        ["ctrl-,"],
        ShowPreferences
    ),
    bind!(
        "help.shortcuts",
        message!("keymap-help-shortcuts"),
        message!("keymap-help-shortcuts-keywords"),
        ["ctrl-/"],
        ShowShortcuts
    ),
    bind!(
        "help.about",
        message!("keymap-help-about"),
        message!("keymap-help-about-keywords"),
        [],
        ShowAbout
    ),
    bind!(
        "view.notifications",
        message!("keymap-view-notifications"),
        message!("keymap-view-notifications-keywords"),
        ["ctrl-shift-n"],
        ShowNotifications
    ),
    bind!(
        "keys.change",
        message!("keymap-keys-change"),
        message!("keymap-keys-change-keywords"),
        [],
        ChangeKeys
    ),
    bind!(
        "keys.reload",
        message!("keymap-keys-reload"),
        message!("keymap-keys-reload-keywords"),
        [],
        ReloadKeys
    ),
    bind!(
        "keys.reset",
        message!("keymap-keys-reset"),
        message!("keymap-keys-reset-keywords"),
        [],
        ResetKeys
    ),
    bind!(
        "app.quit",
        message!("keymap-app-quit"),
        message!("keymap-app-quit-keywords"),
        ["ctrl-q"],
        Quit
    ),
    // The repository.
    bind!(
        "remote.fetch",
        message!("keymap-remote-fetch"),
        message!("keymap-remote-fetch-keywords"),
        ["ctrl-l"],
        Fetch,
        runs "remote.fetch"
    ),
    bind!(
        "remote.pull",
        message!("keymap-remote-pull"),
        message!("keymap-remote-pull-keywords"),
        ["ctrl-shift-down"],
        Pull,
        runs "remote.pull"
    ),
    bind!(
        "remote.push",
        message!("keymap-remote-push"),
        message!("keymap-remote-push-keywords"),
        ["ctrl-shift-up"],
        Push,
        runs "remote.push"
    ),
    bind!(
        "remote.push_lease",
        message!("keymap-remote-push-lease"),
        message!("keymap-remote-push-lease-keywords"),
        [],
        PushWithLease,
        runs "remote.push"
    ),
    bind!(
        "remote.push_force",
        message!("keymap-remote-push-force"),
        message!("keymap-remote-push-force-keywords"),
        [],
        ForcePush,
        runs "remote.push"
    ),
    bind!(
        "pr.create",
        message!("keymap-pr-create"),
        message!("keymap-pr-create-keywords"),
        [],
        StartPullRequest,
        runs "pr.create"
    ),
    bind!(
        "remote.push_skip_hook",
        message!("keymap-remote-push-skip-hook"),
        message!("keymap-remote-push-skip-hook-keywords"),
        [],
        SkipPushHook,
        runs "remote.push"
    ),
    bind!(
        "tag.push_all",
        message!("keymap-tag-push-all"),
        message!("keymap-tag-push-all-keywords"),
        [],
        PushAllTags,
        runs "tag.push"
    ),
    bind!(
        "branch.create",
        message!("keymap-branch-create"),
        message!("keymap-branch-create-keywords"),
        ["ctrl-b"],
        NewBranch,
        runs "branch.create"
    ),
    bind!(
        "stash.push",
        message!("keymap-stash-push"),
        message!("keymap-stash-push-keywords"),
        ["alt-shift-s"],
        StashChanges,
        runs "stash.push"
    ),
    bind!(
        "stash.pop",
        message!("keymap-stash-pop"),
        message!("keymap-stash-pop-keywords"),
        [],
        PopStash,
        runs "stash.pop"
    ),
    bind!(
        "commit.create",
        message!("keymap-commit-create"),
        message!("keymap-commit-create-keywords"),
        ["ctrl-enter"],
        CommitStaged,
        runs "commit.create"
    ),
    bind!(
        "commit.all",
        message!("keymap-commit-all"),
        message!("keymap-commit-all-keywords"),
        ["ctrl-shift-enter"],
        CommitAll
    ),
    bind!(
        "undo.last",
        message!("keymap-undo-last"),
        message!("keymap-undo-last-keywords"),
        ["ctrl-z"],
        Undo,
        runs "undo.last"
    ),
    bind!(
        "redo.last",
        message!("keymap-redo-last"),
        message!("keymap-redo-last-keywords"),
        ["ctrl-y", "ctrl-shift-z"],
        Redo,
        runs "redo.last"
    ),
    bind!(
        "search.commits",
        message!("keymap-search-commits"),
        message!("keymap-search-commits-keywords"),
        ["ctrl-f"],
        FocusSearch
    ),
    bind!(
        "search.only_matches",
        message!("keymap-search-only-matches"),
        message!("keymap-search-only-matches-keywords"),
        ["alt-shift-f"],
        ToggleOnlyMatches
    ),
    bind!(
        "search.files",
        message!("keymap-search-files"),
        message!("keymap-search-files-keywords"),
        ["ctrl-shift-h", "alt-p"],
        FindFile,
        runs "search.files"
    ),
    bind!(
        "graph.jump",
        message!("keymap-graph-jump"),
        message!("keymap-graph-jump-keywords"),
        ["ctrl-g"],
        JumpTo
    ),
    bind!(
        "refs.filter",
        message!("keymap-refs-filter"),
        message!("keymap-refs-filter-keywords"),
        ["ctrl-alt-f"],
        FilterReferences
    ),
    bind!(
        "graph.focus",
        message!("keymap-graph-focus"),
        message!("keymap-graph-focus-keywords"),
        ["escape"],
        FocusGraph
    ),
    bind!(
        "graph.order",
        message!("keymap-graph-order"),
        message!("keymap-graph-order-keywords"),
        [],
        ToggleGraphOrder
    ),
    bind!(
        "graph.dates",
        message!("keymap-graph-dates"),
        message!("keymap-graph-dates-keywords"),
        [],
        ToggleGraphDates
    ),
    bind!(
        "repo.open_folder",
        message!("keymap-repo-open-folder"),
        message!("keymap-repo-open-folder-keywords"),
        ["alt-o"],
        OpenInFileManager,
        runs "repo.open_folder"
    ),
    bind!(
        "repo.open_terminal",
        message!("keymap-repo-open-terminal"),
        message!("keymap-repo-open-terminal-keywords"),
        ["alt-t"],
        OpenInTerminal,
        runs "repo.open_terminal"
    ),
    // The terminal inside the window. Its keys reach the window
    // while the shell has the keys (`views::terminal::text::WINDOW_KEYS`).
    bind!(
        "terminal.toggle",
        message!("keymap-terminal-toggle"),
        message!("keymap-terminal-toggle-keywords"),
        ["ctrl-`"],
        ToggleTerminal
    ),
    bind!(
        "terminal.new",
        message!("keymap-terminal-new"),
        message!("keymap-terminal-new-keywords"),
        ["ctrl-~"],
        NewTerminal
    ),
    bind!(
        "terminal.next",
        message!("keymap-terminal-next"),
        message!("keymap-terminal-next-keywords"),
        ["ctrl-pagedown"],
        NextTerminal
    ),
    bind!(
        "terminal.previous",
        message!("keymap-terminal-previous"),
        message!("keymap-terminal-previous-keywords"),
        ["ctrl-pageup"],
        PrevTerminal
    ),
    bind!(
        "terminal.close",
        message!("keymap-terminal-close"),
        message!("keymap-terminal-close-keywords"),
        [],
        CloseTerminal
    ),
    bind!(
        "repo.open_editor",
        message!("keymap-repo-open-editor"),
        message!("keymap-repo-open-editor-keywords"),
        ["ctrl-shift-e", "alt-e"],
        OpenInEditor,
        runs "repo.open_editor"
    ),
    bind!(
        "diff.next_change",
        message!("keymap-diff-next-change"),
        message!("keymap-diff-next-change-keywords"),
        ["f7"],
        NextChange
    ),
    bind!(
        "diff.previous_change",
        message!("keymap-diff-previous-change"),
        message!("keymap-diff-previous-change-keywords"),
        ["shift-f7"],
        PrevChange
    ),
    bind!(
        "diff.wrap",
        message!("keymap-diff-wrap"),
        message!("keymap-diff-wrap-keywords"),
        ["alt-z"],
        ToggleWrap
    ),
    bind!(
        "diff.against_worktree",
        message!("keymap-diff-against-worktree"),
        message!("keymap-diff-against-worktree-keywords"),
        [],
        CompareWithWorktree
    ),
    // Views that take the centre.
    bind!(
        "view.activity",
        message!("keymap-view-activity"),
        message!("keymap-view-activity-keywords"),
        ["ctrl-shift-l"],
        ShowActivity
    ),
    bind!(
        "view.undo_history",
        message!("keymap-view-undo-history"),
        message!("keymap-view-undo-history-keywords"),
        ["alt-shift-h"],
        ShowUndoHistory
    ),
    bind!(
        "view.signing",
        message!("keymap-view-signing"),
        message!("keymap-view-signing-keywords"),
        [],
        ShowSigning
    ),
    bind!(
        "view.profiles",
        message!("keymap-view-profiles"),
        message!("keymap-view-profiles-keywords"),
        [],
        ShowProfiles
    ),
    bind!(
        "view.integrations",
        message!("keymap-view-integrations"),
        message!("keymap-view-integrations-keywords"),
        [],
        ShowIntegrations
    ),
    bind!(
        "view.repositories",
        message!("keymap-view-repositories"),
        message!("keymap-view-repositories-keywords"),
        ["ctrl-alt-o"],
        ShowRepositories
    ),
    bind!(
        "view.inbox",
        message!("keymap-view-inbox"),
        message!("keymap-view-inbox-keywords"),
        [],
        ShowInbox
    ),
    bind!(
        "view.lfs",
        message!("keymap-view-lfs"),
        message!("keymap-view-lfs-keywords"),
        [],
        ShowLfs
    ),
    bind!(
        "view.maintenance",
        message!("keymap-view-maintenance"),
        message!("keymap-view-maintenance-keywords"),
        [],
        ShowMaintenance
    ),
    bind!(
        "sparse.disable",
        message!("keymap-sparse-disable"),
        message!("keymap-sparse-disable-keywords"),
        [],
        CheckOutEverything,
        runs "sparse.disable"
    ),
    bind!(
        "view.index_flags",
        message!("keymap-view-index-flags"),
        message!("keymap-view-index-flags-keywords"),
        [],
        ShowIndexFlags
    ),
    bind!(
        "patch.apply",
        message!("keymap-patch-apply"),
        message!("keymap-patch-apply-keywords"),
        [],
        ApplyPatch
    ),
    bind!(
        "patch.mailbox",
        message!("keymap-patch-mailbox"),
        message!("keymap-patch-mailbox-keywords"),
        [],
        ApplyMailbox
    ),
    bind!(
        "patch.save",
        message!("keymap-patch-save"),
        message!("keymap-patch-save-keywords"),
        [],
        SaveChangesAsPatch
    ),
    bind!(
        "patch.save_staged",
        message!("keymap-patch-save-staged"),
        message!("keymap-patch-save-staged-keywords"),
        [],
        SaveStagedAsPatch
    ),
    bind!(
        "rerere.toggle",
        message!("keymap-rerere-toggle"),
        message!("keymap-rerere-toggle-keywords"),
        [],
        ToggleRerere
    ),
    // What the banner of an operation that stopped offers, for the palette
    // and for keys of the user's own.
    bind!(
        "op.continue",
        message!("keymap-op-continue"),
        message!("keymap-op-continue-keywords"),
        [],
        ContinueOperation,
        runs "op.continue"
    ),
    bind!(
        "op.skip",
        message!("keymap-op-skip"),
        message!("keymap-op-skip-keywords"),
        [],
        SkipOperation,
        runs "op.skip"
    ),
    bind!(
        "op.abort",
        message!("keymap-op-abort"),
        message!("keymap-op-abort-keywords"),
        [],
        AbortOperation,
        runs "op.abort"
    ),
    bind!(
        "view.rerere",
        message!("keymap-view-rerere"),
        message!("keymap-view-rerere-keywords"),
        [],
        ShowRerereRecords
    ),
    bind!(
        "flow.settings",
        message!("keymap-flow-settings"),
        message!("keymap-flow-settings-keywords"),
        [],
        FlowSettings,
        runs "flow.init"
    ),
    bind!(
        "flow.start",
        message!("keymap-flow-start"),
        message!("keymap-flow-start-keywords"),
        [],
        FlowStart,
        runs "flow.start"
    ),
    bind!(
        "flow.finish",
        message!("keymap-flow-finish"),
        message!("keymap-flow-finish-keywords"),
        [],
        FlowFinish,
        runs "flow.finish"
    ),
    bind!(
        "sign.allow",
        message!("keymap-sign-allow"),
        message!("keymap-sign-allow-keywords"),
        [],
        AllowSigner,
        runs "sign.allow"
    ),
    bind!(
        "sign.signers_file",
        message!("keymap-sign-signers-file"),
        message!("keymap-sign-signers-file-keywords"),
        [],
        ChooseSignersFile
    ),
    bind!(
        "commit.trailer",
        message!("keymap-commit-trailer"),
        message!("keymap-commit-trailer-keywords"),
        [],
        AddCommitTrailer
    ),
    bind!(
        "ignore.pattern",
        message!("keymap-ignore-pattern"),
        message!("keymap-ignore-pattern-keywords"),
        [],
        IgnorePattern
    ),
    bind!(
        "ignore.untrack",
        message!("keymap-ignore-untrack"),
        message!("keymap-ignore-untrack-keywords"),
        [],
        UntrackIgnored
    ),
    bind!(
        "link.register",
        message!("keymap-link-register"),
        message!("keymap-link-register-keywords"),
        [],
        RegisterLinks
    ),
    // The graph.
    bind!("graph.select_next", message!("keymap-graph-select-next"), ["down", "j"], SelectNext, in GRAPH_CONTEXT),
    bind!("graph.select_previous", message!("keymap-graph-select-previous"), ["up", "k"], SelectPrev, in GRAPH_CONTEXT),
    bind!("graph.extend_next", message!("keymap-graph-extend-next"), ["shift-down"], ExtendNext, in GRAPH_CONTEXT),
    bind!("graph.extend_previous", message!("keymap-graph-extend-previous"), ["shift-up"], ExtendPrev, in GRAPH_CONTEXT),
    bind!("graph.select_first", message!("keymap-graph-select-first"), ["ctrl-home"], SelectFirst, in GRAPH_CONTEXT),
    bind!("graph.select_last", message!("keymap-graph-select-last"), ["ctrl-end"], SelectLast, in GRAPH_CONTEXT),
    bind!("graph.next_branch", message!("keymap-graph-next-branch"), ["alt-down", "]"], NextBranch, in GRAPH_CONTEXT),
    bind!("graph.previous_branch", message!("keymap-graph-previous-branch"), ["alt-up", "["], PrevBranch, in GRAPH_CONTEXT),
    bind!("graph.page_down", message!("keymap-graph-page-down"), ["pagedown"], PageDown, in GRAPH_CONTEXT),
    bind!("graph.page_up", message!("keymap-graph-page-up"), ["pageup"], PageUp, in GRAPH_CONTEXT),
    bind!("graph.copy_id", message!("keymap-graph-copy-id"), ["ctrl-c"], CopyCommitId, in GRAPH_CONTEXT),
    // The changes panel.
    bind!("changes.select_next", message!("keymap-changes-select-next"), ["down", "j"], SelectNext, in CHANGES_CONTEXT),
    bind!("changes.select_previous", message!("keymap-changes-select-previous"), ["up", "k"], SelectPrev, in CHANGES_CONTEXT),
    bind!("changes.select_first", message!("keymap-changes-select-first"), ["home"], SelectFirst, in CHANGES_CONTEXT),
    bind!("changes.select_last", message!("keymap-changes-select-last"), ["end"], SelectLast, in CHANGES_CONTEXT),
    bind!("changes.stage", message!("keymap-changes-stage"), ["s"], StageSelected, in CHANGES_CONTEXT),
    bind!("changes.unstage", message!("keymap-changes-unstage"), ["u"], UnstageSelected, in CHANGES_CONTEXT),
    bind!("changes.discard", message!("keymap-changes-discard"), ["d"], DiscardSelected, in CHANGES_CONTEXT),
    bind!("changes.stage_all", message!("keymap-changes-stage-all"), ["ctrl-shift-s", "shift-s"], StageAll, in CHANGES_CONTEXT),
    bind!("changes.unstage_all", message!("keymap-changes-unstage-all"), ["shift-u"], UnstageAll, in CHANGES_CONTEXT),
    bind!("changes.discard_all", message!("keymap-changes-discard-all"), ["shift-d"], DiscardAll, in CHANGES_CONTEXT),
    bind!("changes.select_all", message!("keymap-changes-select-all"), ["ctrl-a"], SelectAll, in CHANGES_CONTEXT),
    bind!("changes.open", message!("keymap-changes-open"), ["enter"], OpenSelected, in CHANGES_CONTEXT),
    bind!("changes.tree", message!("keymap-changes-tree"), ["t"], ToggleTreeView, in CHANGES_CONTEXT),
    bind!("changes.take_ours", message!("keymap-changes-take-ours"), ["o"], TakeOurs, in CHANGES_CONTEXT),
    bind!("changes.take_theirs", message!("keymap-changes-take-theirs"), ["h"], TakeTheirs, in CHANGES_CONTEXT),
    bind!("changes.mark_resolved", message!("keymap-changes-mark-resolved"), ["r"], MarkResolved, in CHANGES_CONTEXT),
    bind!("changes.menu", message!("keymap-changes-menu"), ["shift-f10", "menu"], RowMenu, in CHANGES_CONTEXT),
    // The diff and the editor.
    bind!("diffview.next_change", message!("keymap-diffview-next-change"), ["n"], NextChange, in DIFF_CONTEXT),
    bind!("diffview.previous_change", message!("keymap-diffview-previous-change"), ["p"], PrevChange, in DIFF_CONTEXT),
    bind!("diffview.copy", message!("keymap-diffview-copy"), ["ctrl-c"], CopyText, in DIFF_CONTEXT),
    bind!("blame.copy", message!("keymap-blame-copy"), ["ctrl-c"], CopyText, in BLAME_CONTEXT),
    bind!("edit.save", message!("keymap-edit-save"), ["ctrl-s"], SaveFile, in EDIT_CONTEXT),
];

/// What is done with the pointer and has no key, for the sheet: how it is
/// done, and what it does.
pub const POINTER_NOTES: &[(Message, Message)] = &[
    (
        message!("keymap-pointer-several"),
        message!("keymap-pointer-several-does"),
    ),
    (
        message!("keymap-pointer-label-twice"),
        message!("keymap-pointer-label-twice-does"),
    ),
    (
        message!("keymap-pointer-label-right"),
        message!("keymap-pointer-label-right-does"),
    ),
    (
        message!("keymap-pointer-tab-middle"),
        message!("keymap-tab-close"),
    ),
    (
        message!("keymap-pointer-tab-drag"),
        message!("keymap-pointer-tab-drag-does"),
    ),
    (
        message!("keymap-pointer-branch-drag"),
        message!("keymap-pointer-branch-drag-does"),
    ),
    (
        message!("keymap-pointer-heading-drag"),
        message!("keymap-pointer-heading-drag-does"),
    ),
    (
        message!("keymap-pointer-edge-drag"),
        message!("keymap-pointer-edge-drag-does"),
    ),
];

/// The binding with this id.
pub fn binding(id: &str) -> Option<&'static Binding> {
    BINDINGS.iter().find(|binding| binding.id == id)
}

/// A key as it is compared and shown to the toolkit: parsed and written
/// out again, so that `Ctrl-Shift-P` and `shift-ctrl-p` are one key. None
/// for something that is no key.
pub fn normal_key(typed: &str) -> Option<String> {
    let strokes: Vec<&str> = typed.split_whitespace().collect();
    if strokes.is_empty() || strokes.len() > 4 {
        return None;
    }
    let mut out = Vec::new();
    for stroke in strokes {
        // The toolkit takes modifiers in lower case only.
        let lowered = stroke.to_lowercase();
        let parsed = Keystroke::parse(&lowered).ok()?;
        // A modifier is held with a key; it is not one.
        const MODIFIERS: &[&str] = &[
            "ctrl", "control", "alt", "shift", "cmd", "super", "win", "fn", "platform",
        ];
        if parsed.key.is_empty()
            || parsed.key.chars().any(char::is_control)
            || MODIFIERS.contains(&parsed.key.as_str())
        {
            return None;
        }
        out.push(parsed.unparse());
    }
    Some(out.join(" "))
}

/// A key as people write it: `Ctrl+Shift+P`; on macOS as its menus do,
/// `⇧⌘P`.
pub fn display_key(key: &str) -> String {
    match System::now() {
        System::Mac => display_on_mac(key),
        System::Other => display_elsewhere(key),
    }
}

/// A key as macOS writes it: the signs of the modifiers in the order its
/// menus have them (Control, Option, Shift, Command), then the key.
fn display_on_mac(key: &str) -> String {
    key.split_whitespace()
        .map(|stroke| {
            let (modifiers, name) = split_stroke(stroke);
            let held: Vec<&str> = modifiers.split('-').collect();
            let mut out = String::new();
            for (names, sign) in [
                (&["ctrl", "control"][..], "⌃"),
                (&["alt", "option"][..], "⌥"),
                (&["shift"][..], "⇧"),
                (&["cmd", "super", "win", "platform"][..], "⌘"),
                (&["fn"][..], "fn "),
            ] {
                if held.iter().any(|part| names.contains(part)) {
                    out.push_str(sign);
                }
            }
            out.push_str(&match name {
                "up" => "↑".to_owned(),
                "down" => "↓".to_owned(),
                "left" => "←".to_owned(),
                "right" => "→".to_owned(),
                "enter" => "↩".to_owned(),
                "escape" => "⎋".to_owned(),
                "tab" => "⇥".to_owned(),
                "space" => "Space".to_owned(),
                "pageup" => "⇞".to_owned(),
                "pagedown" => "⇟".to_owned(),
                "home" => "↖".to_owned(),
                "end" => "↘".to_owned(),
                "backspace" => "⌫".to_owned(),
                "delete" => "⌦".to_owned(),
                other => other.to_uppercase(),
            });
            out
        })
        .reduce(|first, next| say!("keymap-key-then", first, next))
        .unwrap_or_default()
}

/// A key as Linux and Windows write it: `Ctrl+Shift+P`.
fn display_elsewhere(key: &str) -> String {
    key.split_whitespace()
        .map(|stroke| {
            // The key itself may be `-`: split off what stands before the
            // last part only.
            let (modifiers, name) = split_stroke(stroke);
            let mut parts: Vec<String> = modifiers
                .split('-')
                .filter(|part| !part.is_empty())
                .map(|part| match part {
                    "ctrl" => "Ctrl".to_owned(),
                    "alt" => "Alt".to_owned(),
                    "shift" => "Shift".to_owned(),
                    "cmd" | "super" | "win" | "platform" => "Super".to_owned(),
                    "fn" => "Fn".to_owned(),
                    other => other.to_owned(),
                })
                .collect();
            parts.push(match name {
                "up" => "↑".to_owned(),
                "down" => "↓".to_owned(),
                "left" => "←".to_owned(),
                "right" => "→".to_owned(),
                "enter" => "Enter".to_owned(),
                "escape" => "Esc".to_owned(),
                "tab" => "Tab".to_owned(),
                "space" => "Space".to_owned(),
                "pageup" => "PgUp".to_owned(),
                "pagedown" => "PgDn".to_owned(),
                "home" => "Home".to_owned(),
                "end" => "End".to_owned(),
                "backspace" => "Backspace".to_owned(),
                "delete" => "Del".to_owned(),
                other => {
                    let mut chars = other.chars();
                    match chars.next() {
                        Some(first) => first.to_uppercase().chain(chars).collect(),
                        None => String::new(),
                    }
                }
            });
            parts.join("+")
        })
        .reduce(|first, next| say!("keymap-key-then", first, next))
        .unwrap_or_default()
}

/// Whether a key that applies everywhere has to keep out of text fields:
/// one that types a character, or that a text field uses to edit. On
/// macOS Option with a key types a character too (`ø`; `@`, `[` and `|`
/// on many layouts), and a text field edits with Cmd and Option.
fn belongs_to_text(key: &str) -> bool {
    /// What a text field of macOS does with Cmd or Option held.
    const EDITING_ON_MAC: &[&str] = &[
        "cmd-z",
        "cmd-shift-z",
        "cmd-y",
        "cmd-a",
        "cmd-c",
        "cmd-x",
        "cmd-v",
        "cmd-left",
        "cmd-right",
        "cmd-up",
        "cmd-down",
        "cmd-shift-left",
        "cmd-shift-right",
        "cmd-shift-up",
        "cmd-shift-down",
        "cmd-backspace",
        "cmd-delete",
        "alt-left",
        "alt-right",
        "alt-up",
        "alt-down",
        "alt-shift-left",
        "alt-shift-right",
        "alt-shift-up",
        "alt-shift-down",
        "alt-backspace",
        "alt-delete",
    ];
    let mac = System::now() == System::Mac;
    if mac
        && key.split_whitespace().next().is_some_and(|first| {
            // Compared as the toolkit writes them: Cmd is `super` on Linux,
            // where the tests of macOS's keys run too.
            let first_normal = normal_key(first);
            EDITING_ON_MAC
                .iter()
                .any(|editing| first_normal.is_some() && normal_key(editing) == first_normal)
                || Keystroke::parse(first).is_ok_and(|stroke| {
                    let held = stroke.modifiers;
                    held.alt && !held.control && !held.platform && stroke.key.chars().count() == 1
                })
        })
    {
        return true;
    }
    belongs_to_text_elsewhere(key)
}

/// Whether a key keeps out of text fields, as Linux and Windows type.
fn belongs_to_text_elsewhere(key: &str) -> bool {
    /// What a text field does with Ctrl held.
    const EDITING: &[&str] = &[
        "ctrl-z",
        "ctrl-y",
        "ctrl-shift-z",
        "ctrl-a",
        "ctrl-c",
        "ctrl-x",
        "ctrl-v",
        "ctrl-e",
        "ctrl-left",
        "ctrl-right",
        "ctrl-shift-left",
        "ctrl-shift-right",
        "ctrl-home",
        "ctrl-end",
        "ctrl-shift-home",
        "ctrl-shift-end",
        "ctrl-backspace",
        "ctrl-delete",
    ];
    /// Keys that type nothing even without Ctrl or Alt.
    const NOT_TYPED: &[&str] = &[
        "escape", "f1", "f2", "f3", "f4", "f5", "f6", "f7", "f8", "f9", "f10", "f11", "f12",
    ];
    key.split_whitespace().next().is_some_and(|first| {
        if EDITING.contains(&first) {
            return true;
        }
        let Ok(stroke) = Keystroke::parse(first) else {
            return false;
        };
        let modifiers = stroke.modifiers;
        let held = modifiers.control || modifiers.alt || modifiers.platform;
        !held && !NOT_TYPED.contains(&stroke.key.as_str())
    })
}

/// The keys of the user's own, as `keybindings.toml` holds them.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct UserKeys {
    /// Keys by binding id.
    #[serde(default)]
    pub keys: BTreeMap<String, OneOrMany>,
}

/// One key, or a list of them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(untagged)]
pub enum OneOrMany {
    /// `"ctrl-p"`.
    One(String),
    /// `["ctrl-p", "f1"]`; an empty list takes every key away.
    Many(Vec<String>),
}

impl OneOrMany {
    fn list(&self) -> Vec<String> {
        match self {
            Self::One(key) => vec![key.clone()],
            Self::Many(keys) => keys.clone(),
        }
    }
}

/// A binding with the keys it ends up with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bound {
    /// Index into [`BINDINGS`].
    pub index: usize,
    /// Its keys, normalised.
    pub keys: Vec<String>,
    /// Whether the user's file set them.
    pub own: bool,
    /// Keys it would have had and lost to another action.
    pub lost: Vec<String>,
}

/// The whole keymap as it is in force.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Resolved {
    /// One entry per row of [`BINDINGS`], in that order.
    pub bound: Vec<Bound>,
    /// What is wrong with the user's file: ids that do not exist, keys that
    /// are no keys.
    pub problems: Vec<String>,
    /// Keys two actions wanted in one place, and who got them.
    pub conflicts: Vec<String>,
}

impl Resolved {
    /// The keys of the binding with this id.
    pub fn keys_of(&self, id: &str) -> &[String] {
        self.bound
            .iter()
            .find(|bound| BINDINGS.get(bound.index).is_some_and(|b| b.id == id))
            .map(|bound| bound.keys.as_slice())
            .unwrap_or(&[])
    }

    /// The first key of the way to run an engine command, as people write
    /// it, when there is one.
    pub fn key_for_command(&self, command: &str) -> Option<String> {
        self.bound.iter().find_map(|bound| {
            let binding = BINDINGS.get(bound.index)?;
            (binding.command == Some(command))
                .then(|| bound.keys.first().map(|key| display_key(key)))
                .flatten()
        })
    }
}

/// Put the user's keys over the defaults and settle who gets a key that
/// two actions want in the same place: an action the user gave the key to
/// keeps it over one that only had it by default, and otherwise the one
/// that comes first in the table. The loser is left without that key and
/// the conflict is reported. A key that applies everywhere and one that
/// applies in a single view do not conflict: in that view the view's own
/// wins, as it does for the defaults.
pub fn resolve(user: &UserKeys) -> Resolved {
    let mut problems = Vec::new();
    let mut own: HashMap<&'static str, Vec<String>> = HashMap::new();
    for (id, keys) in &user.keys {
        let Some(binding) = binding(id) else {
            problems.push(say!("keymap-not-bindable", id = shortened(id)));
            continue;
        };
        let mut normal = Vec::new();
        for key in keys.list() {
            match normal_key(&key) {
                Some(key) if !normal.contains(&key) => normal.push(key),
                Some(_) => {}
                None => problems.push(say!(
                    "keymap-not-a-key",
                    key = shortened(&key),
                    id = binding.id
                )),
            }
        }
        own.insert(binding.id, normal);
    }

    let mut bound: Vec<Bound> = BINDINGS
        .iter()
        .enumerate()
        .map(|(index, binding)| match own.get(binding.id) {
            Some(keys) => Bound {
                index,
                keys: keys.clone(),
                own: true,
                lost: Vec::new(),
            },
            None => Bound {
                index,
                keys: default_keys(binding)
                    .iter()
                    .filter_map(|key| normal_key(key))
                    .collect(),
                own: false,
                lost: Vec::new(),
            },
        })
        .collect();

    // Who holds each key in each place: the user's choices first.
    let mut holders: HashMap<(Option<&'static str>, String), usize> = HashMap::new();
    let mut conflicts = Vec::new();
    let order: Vec<usize> = (0..bound.len())
        .filter(|ix| bound.get(*ix).is_some_and(|b| b.own))
        .chain((0..bound.len()).filter(|ix| bound.get(*ix).is_some_and(|b| !b.own)))
        .collect();
    for ix in order {
        let Some(binding) = BINDINGS.get(ix) else {
            continue;
        };
        let keys = bound.get(ix).map(|b| b.keys.clone()).unwrap_or_default();
        let mut kept = Vec::new();
        let mut lost = Vec::new();
        for key in keys {
            match holders.get(&(binding.context, key.clone())) {
                Some(holder) => {
                    let winner = BINDINGS.get(*holder).map(|b| b.id).unwrap_or("?");
                    conflicts.push(say!(
                        "keymap-conflict",
                        key = display_key(&key),
                        winner,
                        id = binding.id,
                        place = binding.place().to_lowercase(),
                    ));
                    lost.push(key);
                }
                None => {
                    holders.insert((binding.context, key.clone()), ix);
                    kept.push(key);
                }
            }
        }
        if let Some(entry) = bound.get_mut(ix) {
            entry.keys = kept;
            entry.lost = lost;
        }
    }
    Resolved {
        bound,
        problems,
        conflicts,
    }
}

/// What giving a key to an action would do, said before it is saved.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct KeyCheck {
    /// Rows of [`BINDINGS`] that have the key in the same place now and
    /// would lose it.
    pub taken_from: Vec<usize>,
    /// For an action that applies everywhere: rows of single views that
    /// have the key, and keep it there.
    pub kept_by: Vec<usize>,
    /// For an action of one view: rows that apply everywhere and have the
    /// key, which this view would no longer reach by it.
    pub hides: Vec<usize>,
    /// The key types something or edits text: it will not apply while a
    /// text field is typed in.
    pub typed: bool,
}

impl KeyCheck {
    /// The check as sentences, the conflict first.
    pub fn said(&self, key: &str) -> Vec<String> {
        let shown = display_key(key);
        let name = |index: &usize| BINDINGS.get(*index).map(|binding| binding.label.text());
        let mut said = Vec::new();
        for action in self.taken_from.iter().filter_map(name) {
            said.push(say!("keymap-check-taken", key = &shown, action));
        }
        for (index, action) in self
            .kept_by
            .iter()
            .filter_map(|index| Some((index, name(index)?)))
        {
            let place = BINDINGS
                .get(*index)
                .map_or("", Binding::place)
                .to_lowercase();
            said.push(say!("keymap-check-kept", place, key = &shown, action));
        }
        for action in self.hides.iter().filter_map(name) {
            said.push(say!("keymap-check-hides", key = &shown, action));
        }
        if self.typed {
            said.push(say!("keymap-check-typed", key = &shown));
        }
        said
    }
}

/// What would come of giving `key` (normalised) to the row `index`.
pub fn check_key(resolved: &Resolved, index: usize, key: &str) -> KeyCheck {
    let Some(binding) = BINDINGS.get(index) else {
        return KeyCheck::default();
    };
    let mut check = KeyCheck {
        typed: binding.context.is_none() && belongs_to_text(key),
        ..KeyCheck::default()
    };
    for bound in &resolved.bound {
        if bound.index == index || !bound.keys.iter().any(|held| held == key) {
            continue;
        }
        let Some(other) = BINDINGS.get(bound.index) else {
            continue;
        };
        match (binding.context, other.context) {
            (mine, theirs) if mine == theirs => check.taken_from.push(bound.index),
            (None, Some(_)) => check.kept_by.push(bound.index),
            (Some(_), None) => check.hides.push(bound.index),
            _ => {}
        }
    }
    check
}

/// A change to the user's file: the id of an action, and its new keys, or
/// none to take what the file says about it out again.
pub type KeyChange = (String, Option<Vec<String>>);

/// What has to be written so that the row `index` has exactly `keys`
/// (normalised): its own entry, and one for every action in the same place
/// that holds one of them, without it. The loser is written down so that
/// no conflict is left to settle the next time the file is read.
pub fn changes_for(resolved: &Resolved, index: usize, keys: &[String]) -> Vec<KeyChange> {
    let Some(binding) = BINDINGS.get(index) else {
        return Vec::new();
    };
    let mut changes = vec![(binding.id.to_owned(), Some(keys.to_vec()))];
    for bound in &resolved.bound {
        let Some(other) = BINDINGS.get(bound.index) else {
            continue;
        };
        if bound.index == index || other.context != binding.context {
            continue;
        }
        if bound.keys.iter().any(|held| keys.contains(held)) {
            let left: Vec<String> = bound
                .keys
                .iter()
                .filter(|held| !keys.contains(held))
                .cloned()
                .collect();
            changes.push((other.id.to_owned(), Some(left)));
        }
    }
    changes
}

/// Read the user's file to change it: its text as a document that keeps
/// comments and layout. A file that is not there is an empty one; one that
/// the application itself could not read is left alone and said.
fn editable(path: &Path) -> Result<toml_edit::DocumentMut, String> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8(bytes)
            .map_err(|_| say!("keymap-file-not-text-to-change").to_owned())?,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(say!("keymap-file-unreadable", error = err.to_string())),
    };
    let wrong = |why: String| {
        say!(
            "keymap-file-broken-to-change",
            why = why.lines().next().unwrap_or("").trim()
        )
    };
    // What the application reads it as has to hold, or the file is not ours
    // to rewrite.
    toml::from_str::<UserKeys>(&text).map_err(|err| wrong(err.to_string()))?;
    text.parse::<toml_edit::DocumentMut>()
        .map_err(|err| wrong(err.to_string()))
}

/// Write the user's file: beside it first and then over it, so that a
/// write that fails halfway leaves the old one. A link to a file elsewhere
/// stays a link.
fn store(path: &Path, text: &str) -> Result<(), String> {
    let said = |err: std::io::Error| say!("keymap-file-unwritable", error = err.to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(said)?;
    }
    let target = bitbraid_core::paths::canonical(path).unwrap_or_else(|_| path.to_path_buf());
    let mut beside = target.clone().into_os_string();
    beside.push(format!(".{}.new", std::process::id()));
    let beside = PathBuf::from(beside);
    std::fs::write(&beside, text).map_err(said)?;
    std::fs::rename(&beside, &target).map_err(|err| {
        let _ = std::fs::remove_file(&beside);
        said(err)
    })
}

/// Change keys in the user's file and leave the rest of it as it is.
pub fn edit_user_keys(path: &Path, changes: &[KeyChange]) -> Result<(), String> {
    let mut document = editable(path)?;
    let keys = document
        .entry("keys")
        .or_insert(toml_edit::Item::Table(toml_edit::Table::new()));
    let Some(table) = keys.as_table_like_mut() else {
        return Err(say!("keymap-file-keys-not-table").to_owned());
    };
    for (id, change) in changes {
        match change {
            None => {
                table.remove(id);
            }
            Some(list) => {
                let value = match list.as_slice() {
                    [one] => toml_edit::Value::from(one.as_str()),
                    several => toml_edit::Value::Array(
                        several
                            .iter()
                            .map(String::as_str)
                            .collect::<toml_edit::Array>(),
                    ),
                };
                // The comment in front of an entry that is there stays.
                match table.get_mut(id).and_then(toml_edit::Item::as_value_mut) {
                    Some(existing) => {
                        let decor = existing.decor().clone();
                        *existing = value;
                        *existing.decor_mut() = decor;
                    }
                    None => {
                        table.insert(id, toml_edit::Item::Value(value));
                    }
                }
            }
        }
    }
    store(path, &document.to_string())
}

/// Take every key of the user's own out of the file; what else is in it
/// stays.
pub fn clear_user_keys(path: &Path) -> Result<(), String> {
    if path.symlink_metadata().is_err() {
        return Ok(());
    }
    let mut document = editable(path)?;
    if let Some(table) = document
        .get_mut("keys")
        .and_then(toml_edit::Item::as_table_like_mut)
    {
        let ids: Vec<String> = table.iter().map(|(id, _)| id.to_owned()).collect();
        for id in ids {
            table.remove(&id);
        }
    }
    store(path, &document.to_string())
}

/// The start of a name, for a message.
fn shortened(text: &str) -> String {
    let mut cut: String = text.chars().take(40).collect();
    if cut.len() < text.len() {
        cut.push('…');
    }
    cut
}

/// Read `keybindings.toml`. A file that is not there is no keys of the
/// user's own; one that cannot be read is said and counts as none.
pub fn read_user_keys(path: &Path) -> (UserKeys, Option<String>) {
    let text = match std::fs::read(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            return (UserKeys::default(), None);
        }
        Err(err) => {
            return (
                UserKeys::default(),
                Some(say!("keymap-file-unreadable", error = err.to_string())),
            );
        }
        Ok(bytes) => match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                return (
                    UserKeys::default(),
                    Some(say!("keymap-file-not-text-defaults").to_owned()),
                );
            }
        },
    };
    match toml::from_str::<UserKeys>(&text) {
        Ok(keys) => (keys, None),
        Err(err) => (
            UserKeys::default(),
            Some(say!(
                "keymap-file-broken-defaults",
                why = err.to_string().lines().next().unwrap_or("").trim()
            )),
        ),
    }
}

/// What a file looked like when it was looked at: enough to tell that it
/// changed since, without reading it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileStamp {
    /// Its length, none when it is not there.
    length: Option<u64>,
    /// When it was last written.
    modified: Option<std::time::SystemTime>,
}

impl FileStamp {
    /// Look at a file.
    pub fn of(path: &Path) -> Self {
        match std::fs::metadata(path) {
            Ok(meta) => Self {
                length: Some(meta.len()),
                modified: meta.modified().ok(),
            },
            Err(_) => Self {
                length: None,
                modified: None,
            },
        }
    }
}

/// The keymap in force, a GPUI global.
#[derive(Debug, Default)]
pub struct Keys {
    /// Where the user's keys are read from, when there is a place.
    pub file: Option<PathBuf>,
    /// What is in force.
    pub resolved: Resolved,
    /// What was bound last time, to be taken back before binding anew:
    /// the keys, where they apply, and the row of the table.
    bound: Vec<(String, Option<&'static str>, usize)>,
}

impl Global for Keys {}

impl Keys {
    /// Everything there is to say about the user's file: what is wrong in
    /// it, then the conflicts.
    pub fn complaints(&self) -> Vec<String> {
        self.resolved
            .problems
            .iter()
            .chain(&self.resolved.conflicts)
            .cloned()
            .collect()
    }
}

/// The context a key is bound in: the view's, or everywhere but in text
/// fields for keys those use themselves.
fn bound_context(binding: &Binding, key: &str) -> Option<&'static str> {
    match binding.context {
        Some(context) => Some(context),
        None if belongs_to_text(key) => Some(OUTSIDE_TEXT),
        None => None,
    }
}

/// Install the keys: the defaults with the user's over them.
pub fn init(cx: &mut App) {
    let file = cx
        .global::<crate::prefs::Prefs>()
        .dir()
        .map(|dir| dir.join("keybindings.toml"));
    cx.set_global(Keys {
        file,
        ..Keys::default()
    });
    let _ = reload(cx);
}

/// Read the user's file again and bind what it says now. What was bound
/// before is taken back first, action by action, so that keys of the
/// toolkit's own views are left alone. Returns what there is to say.
pub fn reload(cx: &mut App) -> Vec<String> {
    let file = cx.global::<Keys>().file.clone();
    let (user, unreadable) = match &file {
        Some(file) => read_user_keys(file),
        None => (UserKeys::default(), None),
    };
    let mut resolved = resolve(&user);
    if let Some(unreadable) = unreadable {
        resolved.problems.insert(0, unreadable);
    }

    let mut bindings: Vec<KeyBinding> = Vec::new();
    let before = std::mem::take(&mut cx.global_mut::<Keys>().bound);
    for (key, context, index) in &before {
        let Some(binding) = BINDINGS.get(*index) else {
            continue;
        };
        let name: SharedString = binding.action().name().to_owned().into();
        if let Some(unbind) = key_binding(key, Box::new(Unbind(name)), *context) {
            bindings.push(unbind);
        }
    }
    let mut now = Vec::new();
    for bound in &resolved.bound {
        let Some(binding) = BINDINGS.get(bound.index) else {
            continue;
        };
        for key in &bound.keys {
            let context = bound_context(binding, key);
            if let Some(made) = key_binding(key, binding.action(), context) {
                bindings.push(made);
                now.push((key.clone(), context, bound.index));
            }
        }
    }
    cx.bind_keys(bindings);
    let keys = cx.global_mut::<Keys>();
    keys.bound = now;
    keys.resolved = resolved;
    let said = keys.complaints();
    // The menus show the keys anew.
    crate::menus::refresh(cx);
    cx.refresh_windows();
    said
}

/// A key binding from parts that may be anything: none when the key or the
/// context does not parse (the toolkit's own constructor panics on those).
fn key_binding(key: &str, action: Box<dyn Action>, context: Option<&str>) -> Option<KeyBinding> {
    let predicate = match context {
        Some(context) => Some(KeyBindingContextPredicate::parse(context).ok()?.into()),
        None => None,
    };
    KeyBinding::load(key, action, predicate, false, None, &DummyKeyboardMapper).ok()
}

#[cfg(test)]
pub(crate) mod tests {
    use std::cell::Cell;
    use std::collections::{BTreeMap, HashSet};

    thread_local! {
        /// Whose keys a test of this crate has: Linux's and Windows' unless
        /// it says another with [`as_on`].
        pub(super) static SYSTEM: Cell<super::System> = const { Cell::new(super::System::Other) };
    }

    /// Have the keys of `system` for the rest of this test.
    pub(crate) fn as_on(system: super::System) {
        SYSTEM.with(|cell| cell.set(system));
    }

    use super::{
        BINDINGS, KeyCheck, OUTSIDE_TEXT, OneOrMany, UserKeys, belongs_to_text, binding,
        bound_context, changes_for, check_key, clear_user_keys, display_key, edit_user_keys,
        key_binding, normal_key, read_user_keys, resolve,
    };

    fn at(id: &str) -> usize {
        BINDINGS.iter().position(|row| row.id == id).unwrap()
    }

    fn keys(list: &[&str]) -> Vec<String> {
        list.iter().map(|key| (*key).to_owned()).collect()
    }

    fn user(pairs: &[(&str, &[&str])]) -> UserKeys {
        UserKeys {
            keys: pairs
                .iter()
                .map(|(id, keys)| {
                    (
                        (*id).to_owned(),
                        OneOrMany::Many(keys.iter().map(|key| (*key).to_owned()).collect()),
                    )
                })
                .collect::<BTreeMap<_, _>>(),
        }
    }

    #[test]
    fn the_table_is_sound() {
        let mut ids = HashSet::new();
        for row in BINDINGS {
            assert!(ids.insert(row.id), "{} is there twice", row.id);
            assert!(!row.label.text().is_empty(), "{}", row.id);
            assert!(
                row.id.split('.').count() == 2 && row.id == row.id.to_lowercase(),
                "{}",
                row.id
            );
            for key in row.keys {
                let normal = normal_key(key).unwrap_or_else(|| panic!("{key} of {}", row.id));
                // Every default can be bound for real.
                let context = bound_context(row, &normal);
                assert!(
                    key_binding(&normal, row.action(), context).is_some(),
                    "{key} of {}",
                    row.id
                );
            }
        }
        // The defaults do not fight among themselves.
        let resolved = resolve(&UserKeys::default());
        assert_eq!(resolved.conflicts, Vec::<String>::new());
        assert!(resolved.problems.is_empty());
        assert_eq!(resolved.bound.len(), BINDINGS.len());
        // The everyday commands have keys.
        for id in [
            "remote.pull",
            "remote.push",
            "remote.fetch",
            "commit.create",
            "commit.all",
            "branch.create",
            "stash.push",
        ] {
            assert!(!resolved.keys_of(id).is_empty(), "{id} has no key");
        }
        // Never one a desktop keeps for itself.
        for row in BINDINGS {
            assert!(!row.keys.contains(&"ctrl-shift-u"), "{}", row.id);
        }
    }

    #[test]
    fn macos_has_cmd_where_the_others_have_ctrl() {
        use super::{System, on_mac};
        assert_eq!(on_mac("ctrl-p"), "cmd-p");
        assert_eq!(on_mac("ctrl-shift-o"), "cmd-shift-o");
        assert_eq!(on_mac("ctrl-alt-f"), "cmd-alt-f");
        assert_eq!(on_mac("ctrl--"), "cmd--");
        assert_eq!(on_mac("ctrl-k ctrl-s"), "cmd-k cmd-s");
        for same in ["alt-o", "f5", "shift-down", "s", "]"] {
            assert_eq!(on_mac(same), same);
        }
        // What the system keeps with Cmd stays with Ctrl; where macOS has
        // a key of its own for the same thing, that one.
        assert_eq!(on_mac("ctrl-tab"), "ctrl-tab");
        assert_eq!(on_mac("ctrl-shift-tab"), "ctrl-shift-tab");
        assert_eq!(on_mac("ctrl-`"), "ctrl-`");
        assert_eq!(on_mac("ctrl-shift-f"), "ctrl-cmd-f");
        assert_eq!(on_mac("ctrl-home"), "cmd-up");
        assert_eq!(on_mac("ctrl-end"), "cmd-down");

        as_on(System::Mac);
        let resolved = super::resolve(&UserKeys::default());
        assert_eq!(resolved.conflicts, Vec::<String>::new());
        assert!(resolved.problems.is_empty());
        let normal = |key: &str| normal_key(key).unwrap();
        for (id, key) in [
            ("palette.open", "cmd-p"),
            ("app.quit", "cmd-q"),
            ("tab.close", "cmd-w"),
            ("preferences.open", "cmd-,"),
            ("tab.next", "ctrl-tab"),
            ("view.fullscreen", "ctrl-cmd-f"),
            ("graph.select_first", "cmd-up"),
        ] {
            assert_eq!(resolved.keys_of(id).first(), Some(&normal(key)), "{id}");
        }
        // Never a key the system or every application keeps for itself.
        for bound in &resolved.bound {
            let row = &BINDINGS[bound.index];
            for key in &bound.keys {
                for kept in [
                    "cmd-h",
                    "cmd-alt-h",
                    "cmd-m",
                    "cmd-tab",
                    "cmd-shift-tab",
                    "cmd-`",
                    "cmd-space",
                    "ctrl-space",
                    "cmd-shift-3",
                    "cmd-shift-4",
                    "cmd-shift-5",
                    "cmd-alt-escape",
                    "ctrl-up",
                    "ctrl-down",
                ] {
                    assert_ne!(key, &normal(kept), "{}", row.id);
                }
                // And each can be bound for real.
                let context = bound_context(row, key);
                assert!(key_binding(key, row.action(), context).is_some(), "{key}");
            }
        }
    }

    #[test]
    fn macos_shows_keys_with_its_signs() {
        as_on(super::System::Mac);
        assert_eq!(display_key("cmd-shift-p"), "⇧⌘P");
        assert_eq!(display_key("ctrl-cmd-f"), "⌃⌘F");
        assert_eq!(display_key("cmd-alt-f"), "⌥⌘F");
        assert_eq!(display_key("alt-shift-ctrl-cmd-x"), "⌃⌥⇧⌘X");
        assert_eq!(display_key("cmd--"), "⌘-");
        assert_eq!(display_key("cmd-enter"), "⌘↩");
        assert_eq!(display_key("cmd-up"), "⌘↑");
        assert_eq!(display_key("cmd-k cmd-s"), "⌘K, then ⌘S");
        assert_eq!(display_key("f5"), "F5");
        assert_eq!(display_key(""), "");
    }

    #[test]
    fn on_macos_option_types_and_cmd_edits() {
        as_on(super::System::Mac);
        // Option with a key types a character: ø, and on many layouts @,
        // [ and |.
        assert!(belongs_to_text("alt-o"));
        assert!(belongs_to_text("alt-shift-s"));
        assert!(!belongs_to_text("alt-f5"));
        // A text field's own keys.
        assert!(belongs_to_text("cmd-a"));
        assert!(belongs_to_text("cmd-z"));
        assert!(belongs_to_text("cmd-shift-up"));
        assert!(belongs_to_text("alt-left"));
        assert!(!belongs_to_text("cmd-p"));
        assert!(!belongs_to_text("cmd-alt-f"));
        assert!(!belongs_to_text("cmd-shift-l"));
        // As elsewhere.
        assert!(belongs_to_text("s"));
        assert!(belongs_to_text("enter"));
        assert!(!belongs_to_text("escape"));
        // Undo, and pull and push (Cmd+Shift+↑ and ↓ select in a field),
        // keep out of text fields.
        let resolved = super::resolve(&UserKeys::default());
        for id in ["undo.last", "redo.last", "remote.pull", "remote.push"] {
            let row = binding(id).unwrap();
            for key in resolved.keys_of(id) {
                assert_eq!(bound_context(row, key), Some(OUTSIDE_TEXT), "{id} {key}");
            }
        }
    }

    #[test]
    fn undo_and_typed_keys_stay_out_of_text_fields() {
        // Ctrl+Z in a field that is being typed in undoes the typing, not
        // the last git action.
        for id in ["undo.last", "redo.last"] {
            let row = binding(id).unwrap();
            for key in row.keys {
                assert_eq!(bound_context(row, key), Some(OUTSIDE_TEXT), "{id} {key}");
            }
        }
        assert!(belongs_to_text("s"));
        assert!(belongs_to_text("shift-s"));
        assert!(belongs_to_text("enter"));
        assert!(belongs_to_text("ctrl-a"));
        assert!(!belongs_to_text("ctrl-p"));
        assert!(!belongs_to_text("f5"));
        assert!(!belongs_to_text("escape"));
        assert!(!belongs_to_text("alt-o"));
        assert!(!belongs_to_text("ctrl-shift-down"));
        // A key of a view's own stays the view's.
        let stage = binding("changes.stage").unwrap();
        assert_eq!(bound_context(stage, "s"), stage.context);
        let palette = binding("palette.open").unwrap();
        assert_eq!(bound_context(palette, "ctrl-p"), None);
    }

    #[test]
    fn keys_are_normalised_and_shown() {
        assert_eq!(normal_key("Ctrl-Shift-P").as_deref(), Some("ctrl-shift-p"));
        assert_eq!(normal_key("shift-ctrl-p").as_deref(), Some("ctrl-shift-p"));
        assert_eq!(
            normal_key("  ctrl-k   ctrl-s ").as_deref(),
            Some("ctrl-k ctrl-s")
        );
        for bad in [
            "",
            "   ",
            "ctrl-",
            "ctrl-shift",
            "hyper-x-y-z",
            "a b c d e",
            "ctrl-\u{0}",
        ] {
            assert_eq!(normal_key(bad), None, "{bad:?}");
        }
        assert_eq!(display_key("ctrl-shift-p"), "Ctrl+Shift+P");
        assert_eq!(display_key("ctrl--"), "Ctrl+-");
        assert_eq!(display_key("ctrl-="), "Ctrl+=");
        assert_eq!(display_key("f5"), "F5");
        assert_eq!(display_key("shift-down"), "Shift+↓");
        assert_eq!(display_key("ctrl-k ctrl-s"), "Ctrl+K, then Ctrl+S");
        assert_eq!(display_key(""), "");
    }

    #[test]
    fn the_users_keys_replace_the_defaults() {
        let resolved = resolve(&user(&[
            ("remote.pull", &["Ctrl-Alt-P"]),
            ("remote.push", &["f9", "ctrl-u", "f9"]),
            ("theme.cycle", &[]),
        ]));
        assert_eq!(resolved.keys_of("remote.pull"), ["ctrl-alt-p"]);
        assert_eq!(resolved.keys_of("remote.push"), ["f9", "ctrl-u"]);
        assert!(resolved.keys_of("theme.cycle").is_empty());
        // Everything else is as it was.
        assert_eq!(resolved.keys_of("palette.open"), ["ctrl-p"]);
        assert!(resolved.problems.is_empty() && resolved.conflicts.is_empty());
        assert_eq!(
            resolved.key_for_command("remote.pull").as_deref(),
            Some("Ctrl+Alt+P")
        );
        assert_eq!(resolved.key_for_command("stash.pop"), None);
        assert_eq!(resolved.key_for_command("no.such"), None);
        let own: Vec<&str> = resolved
            .bound
            .iter()
            .filter(|bound| bound.own)
            .map(|bound| BINDINGS[bound.index].id)
            .collect();
        assert_eq!(own, ["theme.cycle", "remote.pull", "remote.push"]);
    }

    #[test]
    fn conflicts_are_found_and_settled() {
        // The user's choice takes a key from a default.
        let resolved = resolve(&user(&[("remote.pull", &["ctrl-p"])]));
        assert_eq!(resolved.keys_of("remote.pull"), ["ctrl-p"]);
        assert!(resolved.keys_of("palette.open").is_empty());
        assert_eq!(resolved.conflicts.len(), 1);
        assert!(
            resolved.conflicts[0].contains("Ctrl+P")
                && resolved.conflicts[0].contains("remote.pull keeps it"),
            "{:?}",
            resolved.conflicts
        );
        let palette = resolved
            .bound
            .iter()
            .find(|bound| BINDINGS[bound.index].id == "palette.open")
            .unwrap();
        assert_eq!(palette.lost, ["ctrl-p"]);

        // Two of the user's own on one key: the first in the table keeps it.
        let resolved = resolve(&user(&[
            ("remote.push", &["f9"]),
            ("remote.pull", &["f9", "f10"]),
        ]));
        assert_eq!(resolved.keys_of("remote.pull"), ["f9", "f10"]);
        assert!(resolved.keys_of("remote.push").is_empty());
        assert_eq!(resolved.conflicts.len(), 1);

        // A key everywhere and the same key in one view are not a conflict;
        // the same key twice in one view is.
        let resolved = resolve(&user(&[("remote.pull", &["s"])]));
        assert!(resolved.conflicts.is_empty());
        let resolved = resolve(&user(&[("changes.unstage", &["s"])]));
        assert_eq!(resolved.conflicts.len(), 1);
        assert_eq!(resolved.keys_of("changes.unstage"), ["s"]);
        assert!(resolved.keys_of("changes.stage").is_empty());
    }

    #[test]
    fn a_key_is_checked_before_it_is_saved() {
        let resolved = resolve(&UserKeys::default());
        // Nothing has it.
        let free = check_key(&resolved, at("remote.pull"), "ctrl-alt-p");
        assert_eq!(free, KeyCheck::default());
        assert!(free.said("ctrl-alt-p").is_empty());
        // Another action has it in the same place.
        let taken = check_key(&resolved, at("remote.pull"), "ctrl-p");
        assert_eq!(taken.taken_from, [at("palette.open")]);
        let said = taken.said("ctrl-p");
        assert_eq!(said.len(), 1, "{said:?}");
        assert!(
            said[0].contains("Ctrl+P")
                && said[0].contains("Command palette")
                && said[0].contains("takes it away"),
            "{said:?}"
        );
        // A key the action has already is not a conflict with itself.
        assert_eq!(
            check_key(&resolved, at("palette.open"), "ctrl-p"),
            KeyCheck::default()
        );
        // An action everywhere on a key a view has: the view keeps it, and
        // a key that types is said to keep out of text fields.
        let typed = check_key(&resolved, at("remote.pull"), "s");
        assert!(typed.taken_from.is_empty());
        assert_eq!(typed.kept_by, [at("changes.stage")]);
        assert!(typed.typed);
        let said = typed.said("s");
        assert!(
            said[0].contains("changes view") && said[1].contains("typing"),
            "{said:?}"
        );
        // An action of one view on a key that applies everywhere.
        let hides = check_key(&resolved, at("changes.stage"), "ctrl-p");
        assert_eq!(hides.hides, [at("palette.open")]);
        assert!(hides.taken_from.is_empty() && !hides.typed);
        // The same key in two views is nobody's business.
        assert_eq!(
            check_key(&resolved, at("graph.select_next"), "home"),
            KeyCheck::default()
        );
        // A row that is not there.
        assert_eq!(
            check_key(&resolved, usize::MAX, "ctrl-p"),
            KeyCheck::default()
        );
    }

    #[test]
    fn saving_a_key_writes_down_who_loses_it() {
        let resolved = resolve(&UserKeys::default());
        assert_eq!(
            changes_for(&resolved, at("remote.pull"), &keys(&["ctrl-p"])),
            [
                ("remote.pull".to_owned(), Some(keys(&["ctrl-p"]))),
                ("palette.open".to_owned(), Some(Vec::new())),
            ]
        );
        // An action with two keys keeps the other one.
        assert_eq!(
            changes_for(&resolved, at("remote.pull"), &keys(&["f9", "ctrl-y"])),
            [
                ("remote.pull".to_owned(), Some(keys(&["f9", "ctrl-y"]))),
                ("redo.last".to_owned(), Some(keys(&["ctrl-shift-z"]))),
            ]
        );
        // A view's key is not taken by an action that applies everywhere.
        assert_eq!(
            changes_for(&resolved, at("remote.pull"), &keys(&["s"])).len(),
            1
        );
        assert!(changes_for(&resolved, usize::MAX, &keys(&["s"])).is_empty());
        // What is written leaves nothing to settle when it is read back.
        let mut user = UserKeys::default();
        for (id, change) in changes_for(&resolved, at("remote.pull"), &keys(&["ctrl-p"])) {
            user.keys.insert(id, OneOrMany::Many(change.unwrap()));
        }
        let after = resolve(&user);
        assert!(after.conflicts.is_empty(), "{:?}", after.conflicts);
        assert_eq!(after.keys_of("remote.pull"), ["ctrl-p"]);
        assert!(after.keys_of("palette.open").is_empty());
    }

    #[test]
    fn the_file_is_changed_in_place_and_keeps_what_else_is_in_it() {
        let dir = std::env::temp_dir().join(format!("bitbraid-keyedit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        // No file and no folder yet: both are made.
        let file = dir.join("deeper").join("keybindings.toml");
        edit_user_keys(
            &file,
            &[("remote.pull".to_owned(), Some(keys(&["ctrl-alt-p"])))],
        )
        .unwrap();
        let (read, said) = read_user_keys(&file);
        assert_eq!(said, None);
        assert_eq!(resolve(&read).keys_of("remote.pull"), ["ctrl-alt-p"]);
        // Nothing is left beside it.
        assert_eq!(std::fs::read_dir(dir.join("deeper")).unwrap().count(), 1);

        // A file written by hand keeps its comments, its order and what is
        // not ours.
        let by_hand = concat!(
            "# my keys\n\n[keys]\n# pull with one hand\n",
            "\"remote.pull\" = \"ctrl-alt-p\" # was ctrl-shift-down\n",
            "\"theme.cycle\" = []\n\n[other]\nkept = true\n"
        );
        std::fs::write(&file, by_hand).unwrap();
        edit_user_keys(
            &file,
            &[
                ("remote.pull".to_owned(), Some(keys(&["f9", "ctrl-u"]))),
                ("theme.cycle".to_owned(), None),
                ("zoom.in".to_owned(), Some(keys(&["f2"]))),
                ("stash.pop".to_owned(), Some(Vec::new())),
                ("never.there".to_owned(), None),
            ],
        )
        .unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        for kept in [
            "# my keys\n",
            "# pull with one hand\n\"remote.pull\" = [\"f9\", \"ctrl-u\"] # was ctrl-shift-down\n",
            "[other]\nkept = true\n",
        ] {
            assert!(text.contains(kept), "{kept:?} in {text:?}");
        }
        assert!(!text.contains("theme.cycle"), "{text}");
        let (read, said) = read_user_keys(&file);
        assert_eq!(said, None);
        let resolved = resolve(&read);
        assert_eq!(resolved.keys_of("remote.pull"), ["f9", "ctrl-u"]);
        assert_eq!(resolved.keys_of("zoom.in"), ["f2"]);
        assert_eq!(resolved.keys_of("theme.cycle"), ["alt-shift-t"]);
        assert!(resolved.keys_of("stash.pop").is_empty());
        assert!(resolved.bound[at("stash.pop")].own);
        assert!(resolved.problems.is_empty(), "{:?}", resolved.problems);

        // Every key goes; the rest of the file stays.
        clear_user_keys(&file).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(
            text.contains("# my keys\n") && text.contains("[other]\nkept = true\n"),
            "{text}"
        );
        assert_eq!(read_user_keys(&file).0, UserKeys::default());

        // Keys written as one line are changed there.
        std::fs::write(&file, "keys = { \"remote.pull\" = \"f9\" }\n").unwrap();
        edit_user_keys(&file, &[("zoom.in".to_owned(), Some(keys(&["f2"])))]).unwrap();
        let resolved = resolve(&read_user_keys(&file).0);
        assert_eq!(resolved.keys_of("remote.pull"), ["f9"]);
        assert_eq!(resolved.keys_of("zoom.in"), ["f2"]);

        // A name that is anything at all is written so that it reads back.
        let odd = "a\"b\nc = [1]\u{0}".to_owned();
        edit_user_keys(&file, &[(odd.clone(), Some(keys(&["f3"])))]).unwrap();
        let (read, said) = read_user_keys(&file);
        assert_eq!(said, None);
        assert!(read.keys.contains_key(&odd));

        // A file the application cannot read is the user's to put right:
        // it is not written over, and it is said.
        for broken in [
            &b"[keys\n"[..],
            b"[keys]\n\"remote.pull\" = 7\n",
            b"keys = \"all of them\"\n",
            b"\xff\xfe",
        ] {
            std::fs::write(&file, broken).unwrap();
            let change = [("zoom.in".to_owned(), Some(keys(&["f2"])))];
            let said = edit_user_keys(&file, &change).unwrap_err();
            assert!(
                said.contains("keybindings.toml") && said.len() < 300,
                "{said}"
            );
            assert!(clear_user_keys(&file).is_err());
            assert_eq!(std::fs::read(&file).unwrap(), broken);
        }

        // A file that is a link to one kept elsewhere stays a link.
        #[cfg(unix)]
        {
            let real = dir.join("kept-elsewhere.toml");
            std::fs::write(&real, "[keys]\n").unwrap();
            std::fs::remove_file(&file).unwrap();
            std::os::unix::fs::symlink(&real, &file).unwrap();
            edit_user_keys(&file, &[("zoom.in".to_owned(), Some(keys(&["f2"])))]).unwrap();
            assert!(file.symlink_metadata().unwrap().file_type().is_symlink());
            assert!(std::fs::read_to_string(&real).unwrap().contains("zoom.in"));
        }

        // Nothing to clear where there is no file, and none is made.
        let nowhere = dir.join("none").join("keybindings.toml");
        clear_user_keys(&nowhere).unwrap();
        assert!(!nowhere.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_hostile_file_is_reported_and_binds_nothing_odd() {
        let resolved = resolve(&user(&[
            ("no.such.thing", &["ctrl-p"]),
            (
                "remote.pull",
                &["", "ctrl-", "not a key at all really", "ctrl-alt-p"],
            ),
            (&"x".repeat(500), &["f1"]),
        ]));
        assert_eq!(resolved.keys_of("remote.pull"), ["ctrl-alt-p"]);
        assert_eq!(resolved.keys_of("palette.open"), ["ctrl-p"]);
        assert_eq!(resolved.problems.len(), 5, "{:?}", resolved.problems);
        assert!(resolved.problems.iter().all(|said| said.len() < 200));

        let dir = std::env::temp_dir().join(format!("bitbraid-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("keybindings.toml");
        assert_eq!(read_user_keys(&file), (UserKeys::default(), None));
        std::fs::write(
            &file,
            "[keys]\n\"remote.pull\" = \"ctrl-alt-p\"\n\"theme.cycle\" = []\n\"zoom.in\" = [\"f2\"]\n",
        )
        .unwrap();
        let (keys, said) = read_user_keys(&file);
        assert_eq!(said, None);
        assert_eq!(keys.keys.len(), 3);
        assert_eq!(resolve(&keys).keys_of("zoom.in"), ["f2"]);
        for broken in [
            &b"[keys\n"[..],
            b"[keys]\n\"remote.pull\" = 7\n",
            b"keys = \"all of them\"\n",
            b"\xff\xfe",
        ] {
            std::fs::write(&file, broken).unwrap();
            let (keys, said) = read_user_keys(&file);
            assert_eq!(keys, UserKeys::default());
            assert!(said.is_some(), "{broken:?}");
            // The file is the user's: it stays as it is.
            assert_eq!(std::fs::read(&file).unwrap(), broken);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
