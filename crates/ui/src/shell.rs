//! The window root: tab bar, toolbar, body, status bar.

use std::{
    cell::Cell,
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
};

use bitbraid_git::askpass::{Event as AskEvent, Kind as AskKind, Question};
use futures::StreamExt as _;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    command::{Command, CommandGroup, CommandItem, CommandState},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::ContextMenuExt,
    menu::DropdownMenu,
    popover::Popover,
    spinner::Spinner,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::{
    brand,
    gitexe::GitExecutable,
    keymap::{
        self, ApplyMailbox, ApplyPatch, CloseTab, CloseTerminal, CommitAll, CommitStaged,
        CycleTheme, Fetch, FilterReferences, FocusGraph, FocusReferences, JumpTo, JumpToTab, Keys,
        MoveTabLeft, MoveTabRight, NewBranch, NewTab, NewTerminal, NextTab, NextTerminal,
        OpenRepository, PopStash, PrevTab, PrevTerminal, Pull, Push, Quit, Redo, Refresh,
        RegisterLinks, ReloadKeys, RenameTab, SaveChangesAsPatch, ShowActivity, ShowIndexFlags,
        ShowLfs, ShowPreferences, ShowProfiles, ShowShortcuts, ShowSigning, ShowUndoHistory,
        StashChanges, ToggleLeftPanel, TogglePalette, ToggleRerere, ToggleRightPanel,
        ToggleTerminal, Undo, ZoomIn, ZoomOut, ZoomReset,
    },
    notices::Notice,
    prefs::Prefs,
    theme,
    views::{
        notices::{NoticesEvent, NoticesView},
        preferences::{PreferencesEvent, PreferencesView},
        refs::RemoteRequest,
        shortcuts::{ShortcutsEvent, ShortcutsView},
        start::{StartEvent, StartPage},
        stashes::StashRequest,
        workspace::{
            LfsTransfer, Outside, PULL_BUTTON, PushHow, Workspace, WorkspaceEvent,
            form::{self, Answers, Check, Choose, Form, FormHost, Forms, Row, Sheet, Verdict},
            terminals::TerminalKey,
        },
    },
};

mod finder;
mod inbox;
mod pinned;
mod probe;
mod profile;
mod providers;
mod repositories;
pub mod tab_bench;
mod tab_edges;
mod toolbar_fit;
mod updates;

pub use probe::{ShellProbe, TabProbe};

enum TabKind {
    Start(Entity<StartPage>),
    Repo(Entity<Workspace>),
}

struct TabEntry {
    title: SharedString,
    path: Option<PathBuf>,
    kind: TabKind,
    /// The repositories the tab showed before the one it shows: another
    /// worktree opened in its place. Their terminals are kept for when
    /// they are opened there again (AGENTS.md section 2).
    before: Vec<PathBuf>,
}

/// What a palette entry does when confirmed.
enum PaletteAction {
    /// One of the window's own actions, by its row in the table of keys.
    Binding(usize),
    OpenRepo(PathBuf),
    Tab(usize),
    /// An engine command by id: asked for its parameters, then run.
    Command(&'static str),
    /// Use this theme.
    Theme(crate::prefs::ThemeChoice),
    /// Switch to this profile.
    Profile(String),
}

struct PaletteEntry {
    label: SharedString,
    /// Said small beside the label: the id of a command.
    detail: Option<SharedString>,
    /// The key that does the same, as people write it.
    key: Option<SharedString>,
    keywords: Vec<SharedString>,
    action: PaletteAction,
}

/// The sections of the palette, in the order they are listed.
struct PaletteSection {
    heading: &'static str,
    entries: Vec<PaletteEntry>,
}

/// One parameter in the prompt a command is asked for its parameters in.
struct ParamField {
    spec: bitbraid_engine::ParamSpec,
    /// The field it is typed into; none for a flag or a choice.
    input: Option<Entity<InputState>>,
    /// What can be picked for a flag or a choice, and which is; the empty
    /// word stands for leaving the parameter out.
    options: Vec<&'static str>,
    picked: usize,
}

/// The prompt for a command's parameters.
struct ParamPrompt {
    id: &'static str,
    label: &'static str,
    fields: Vec<ParamField>,
    /// Why what was typed cannot be run.
    error: Option<String>,
}

/// A view of the window's own that takes the place of the tab's content
/// for as long as it is looked at: it needs no repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WindowSheet {
    /// The preferences.
    Preferences,
    /// Every key there is.
    Shortcuts,
    /// What was said in notifications.
    Notices,
    /// The list of keys, a dialog over the window.
    KeyList,
    /// What the application is, a dialog over the window
    /// (`views::about`).
    About,
    /// Every repository that is known (`views::repositories`).
    Repositories,
    /// The inbox: what waits for the user at the providers
    /// (`views::inbox`).
    Inbox,
}

/// The window root view.
pub struct Shell {
    focus_handle: FocusHandle,
    tabs: Vec<TabEntry>,
    active: usize,
    /// A view of the window's own shown over the tab's content.
    sheet: Option<WindowSheet>,
    preferences: Entity<PreferencesView>,
    shortcuts: Entity<ShortcutsView>,
    key_list: Entity<crate::views::key_list::KeyListView>,
    about: Entity<crate::views::about::AboutView>,
    notices: Entity<NoticesView>,
    /// The page of repositories, once it has been shown.
    repositories: Option<Entity<crate::views::repositories::RepositoriesView>>,
    /// The inbox, read from the start for the status bar's summary.
    inbox: Entity<crate::views::inbox::InboxView>,
    /// The field the new place of a repository that moved is typed into,
    /// which repository it is for, and why what was typed will not do.
    locate_input: Entity<InputState>,
    /// The list of the tabs at the right end of the tab bar: whether it is
    /// open, and what is typed to find a tab in it.
    tab_list_open: bool,
    tab_search: Entity<InputState>,
    /// A tab that is being dragged: where it was, and where it would land.
    tab_drag: Option<(usize, usize)>,
    /// The room the tabs had when the bar was last drawn: what they took
    /// and what was left after the plus, in points.
    tab_room: Rc<Cell<(f32, f32)>>,
    /// The first of the tabs drawn when the bar was last drawn, with more
    /// than fit: the next frame starts there too, so that showing a tab
    /// that is drawn moves none.
    tab_first: Cell<usize>,
    /// The tab the pointer is over: it shows its cross.
    hovered_tab: Option<usize>,
    /// The zoom the toasts' place was last measured at.
    toasts_zoom: f32,
    /// The zoom the window's buttons were last put in the middle of the
    /// bar of tabs for (macOS, `crate::chrome`).
    buttons_zoom: f32,
    /// The panels of the repository's and the branch's control.
    repo_finder: finder::Finder,
    branch_finder: finder::Finder,
    locating: Option<PathBuf>,
    locate_error: Option<String>,
    /// The profiles there are, by name, and the one the user's own git
    /// configuration is: what the menu beside the tabs lists and marks.
    profiles: Vec<String>,
    profile_now: Option<String>,
    /// The profiles in full, for the control beside the tabs and the
    /// preferences.
    profile_state: crate::views::profiles::Profiles,
    /// The engine's commands, for the palette.
    commands: Vec<bitbraid_engine::CommandSpec>,
    /// The prompt a command from the palette is asked for its parameters in.
    param_prompt: Option<ParamPrompt>,
    /// What the files of the user's own (keys, themes) looked like when
    /// they were last read: they are read again when that changes.
    config_seen: Option<crate::keymap::FileStamp>,
    themes_seen: crate::theme::FolderStamp,
    palette: Entity<CommandState>,
    palette_open: bool,
    /// The field of the dialog that answers git's questions.
    ask_input: Entity<InputState>,
    /// The question on screen, and the ones behind it.
    asking: Option<Question>,
    ask_queue: VecDeque<Question>,
    /// The forms the window itself asks in: what to clone and where to,
    /// and where a new repository goes (`views/workspace/form.rs`).
    forms: Forms<Shell>,
    /// The folder clones go into, and the destination last suggested in
    /// the clone form (a destination the user typed is kept).
    clone_parent: PathBuf,
    clone_suggested: String,
    /// The templates a new repository's `.gitignore` can start from: the
    /// name of each and what it is called.
    ignore_templates: Vec<(String, String)>,
    /// A clone or init that is running: it belongs to no tab yet.
    creating: Option<Creating>,
    /// What the accounts' places of the clone and init forms say
    /// (`shell/providers.rs`).
    provider_places: providers::Places,
    /// The terminals of every repository and worktree the tabs show or
    /// showed (AGENTS.md section 2).
    terminals: Entity<crate::views::terminal::sessions::Terminals>,
    _subscriptions: Vec<Subscription>,
}

/// Zoom to one of the sizes of the status bar's list, in percent.
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct ZoomTo {
    percent: u32,
}

/// The sizes the status bar's zoom offers, from the largest down.
const ZOOM_SIZES: [u32; 12] = [300, 250, 200, 175, 150, 140, 130, 120, 110, 100, 90, 80];

/// What the Pull button does here (the menu beside Pull).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct PullWith {
    mode: String,
}

/// Check out a local branch (the menu under the current branch's name).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct SwitchBranch {
    name: String,
}

/// Open a repository that was open before (the menu under the
/// repository's name).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct OpenRecent {
    path: PathBuf,
}

/// Push the current branch in one of the ways there are (the menu
/// beside Push).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct PushCurrent {
    how: crate::views::workspace::PushHow,
}

/// Fetch every remote (the menu beside Pull).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct FetchAll;

/// Switch to a profile: commit as it everywhere, with its own tabs (the
/// menu beside the tabs).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct SwitchProfile {
    name: String,
}

/// Move the content of large files (the toolbar's LFS group).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct LfsMove {
    what: crate::views::workspace::LfsTransfer,
}

/// Fetch one remote or add one (the menu beside Pull).
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct ToolbarRemote {
    request: RemoteRequest,
}

/// A button of the toolbar's middle group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
enum Tool {
    Undo,
    Redo,
    Pull,
    Push,
    Branch,
    Stash,
    Pop,
    Terminal,
}

impl Tool {
    /// What its button is known by.
    fn id(self) -> &'static str {
        match self {
            Tool::Undo => "undo",
            Tool::Redo => "redo",
            Tool::Pull => "pull",
            Tool::Push => "push",
            Tool::Branch => "new-branch",
            Tool::Stash => "stash",
            Tool::Pop => "pop",
            Tool::Terminal => "terminal",
        }
    }

    /// What its entry in the menu says.
    fn label(self) -> &'static str {
        match self {
            Tool::Undo => say!("shell-undo"),
            Tool::Redo => say!("shell-redo"),
            Tool::Pull => say!("shell-pull"),
            Tool::Push => say!("shell-push"),
            Tool::Branch => say!("shell-branch"),
            Tool::Stash => say!("shell-stash"),
            Tool::Pop => say!("shell-pop"),
            Tool::Terminal => say!("shell-terminal"),
        }
    }
}

/// Do what a button of the middle group does, from the menu that has the
/// buttons a narrow window has no room for.
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct ToolbarTool {
    tool: Tool,
}

/// What the menu of a tab asks for: the tab by its place.
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = shell, no_json)]
struct TabMenu {
    index: usize,
    what: TabDo,
}

/// The entries of a tab's menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize)]
enum TabDo {
    Rename,
    Close,
    CloseOthers,
    MoveLeft,
    MoveRight,
    CopyPath,
}

/// What goes with the pointer while a tab is dragged: the tab.
struct DragTab {
    title: SharedString,
}

impl Render for DragTab {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        h_flex()
            .h(px(TAB_BAR_HEIGHT))
            .pl(px(TAB_INSET))
            .pr(px(28.))
            .items_center()
            .bg(theme.title_bar)
            .border_1()
            .border_color(theme.border)
            .text_sm()
            .text_color(theme.tab_active_foreground)
            .child(self.title.clone())
    }
}

/// A tab that is being dragged to another place.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DraggedTab {
    index: usize,
    title: SharedString,
}

/// The tab a repository that is being opened takes the place of: the one
/// it was asked to take over, when there is such a tab; else the tab that
/// is shown when that is a start tab; else none, and it gets its own.
fn tab_to_replace(
    in_place_of: Option<usize>,
    tabs: usize,
    active: usize,
    active_is_start: bool,
) -> Option<usize> {
    in_place_of
        .filter(|ix| *ix < tabs)
        .or_else(|| (active_is_start && active < tabs).then_some(active))
}

/// Which of the repositories that are kept for the next time is the one
/// shown: its place among the tabs that have one (`kept`, a flag per tab).
/// The start page, which is not kept, counts as the repository before it.
fn kept_active(kept: &[bool], active: usize) -> usize {
    kept.iter()
        .take(active.saturating_add(1))
        .filter(|kept| **kept)
        .count()
        .saturating_sub(1)
}

/// Which of `count` tabs the bar draws when `fits` of them fit at their
/// least: all while they fit, else as many as fit from `first`, the first
/// drawn the last time, moved on as little as needed for the shown one
/// (`active`) to be among them. Showing a tab that is drawn moves none.
/// The others are in the list under the chevron.
fn tabs_drawn(count: usize, fits: usize, active: usize, first: usize) -> std::ops::Range<usize> {
    let fits = fits.max(1);
    if count <= fits {
        return 0..count;
    }
    let active = active.min(count.saturating_sub(1));
    let start = first.min(count.saturating_sub(fits));
    let start = if active < start {
        active
    } else if active >= start.saturating_add(fits) {
        active.saturating_add(1).saturating_sub(fits)
    } else {
        start
    };
    start..start.saturating_add(fits)
}

/// Where the active tab is after the tab at `from` was put at `to`.
fn active_after_move(active: usize, from: usize, to: usize) -> usize {
    if active == from {
        to
    } else if from < active && active <= to {
        active.saturating_sub(1)
    } else if to <= active && active < from {
        active.saturating_add(1)
    } else {
        active
    }
}

/// What a tab is called: the name it was given, else its folder's.
fn tab_title(path: &Path, aliases: &std::collections::HashMap<String, String>) -> String {
    aliases
        .get(&path.to_string_lossy().into_owned())
        .map(|alias| alias.trim())
        .filter(|alias| !alias.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| repo_title(path))
}

/// Heights of the bars around the content, at 100 %.
pub(crate) const TAB_BAR_HEIGHT: f32 = 34.0;
/// From a tab's left edge to what is in it.
const TAB_INSET: f32 = 20.0;
/// From a tab's left edge to what is in it, when there are more tabs
/// than fit at their own width.
const TAB_INSET_CROWDED: f32 = 8.0;
/// About how wide a tab is at its own width.
const TAB_ROOMY: f32 = 110.0;
/// What the tab bar holds besides the tabs: the pinned tabs, the plus
/// and the controls at the right end.
const TAB_BAR_FIXED: f32 = 360.0;
/// The narrowest a tab grows when there are more than fit.
const TAB_MIN_WIDTH: f32 = 56.0;
const TOOLBAR_HEIGHT: f32 = 48.0;
/// Height of the status bar, its top line included: 26 (a line and 25
/// under it).
pub const STATUS_BAR_HEIGHT: f32 = 26.0;
/// Width of the caret beside Pull: with it Push's middle is 66 after
/// Pull's, and the caret's own 31 after.
const TOOL_CARET_WIDTH: f32 = 14.0;
/// What the toolbar's middle group keeps free after Pop, the terminal's
/// button included. The group is centred between the bar's two ends with
/// it, which puts the middle of Undo 244 left of the window's. (162 for
/// that, less 2 for the bar's own edges, which are 0 and 2.)
const TOOL_GROUP_TRAIL: f32 = 160.0;
/// What the rule and the terminal's button after Pop take: the rule 3
/// after Pop's right edge, the button's middle 31 after the rule.
const TERMINAL_SPAN: f32 = 60.0;
/// From the right edge of Pop to the large-file control.
const LFS_GAP: f32 = 70.0;
/// Width of the large-file control.
const LFS_WIDTH: f32 = 46.0;
/// How far the group stands further left with the large-file control.
const LFS_SHIFT: f32 = 19.0;

/// A repository being made.
struct Creating {
    /// What the status bar says, for example `Cloning project`.
    label: String,
    /// What git reports.
    stage: Option<(String, Option<u8>)>,
    /// Set to stop it.
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Shell {
    /// Create the shell, restoring open repositories from the last session and
    /// opening `initial` when given.
    pub fn new(
        initial: Option<PathBuf>,
        link: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // The window exists from here on; what comes before the first
        // session is opened is the shell's own and should stay small.
        crate::session::milestone("shell building");
        // Whatever is measured while the views are built is at the zoom.
        crate::zoom::set(cx.global::<Prefs>().preferences.zoom);
        let focus_handle = cx.focus_handle();
        let palette = cx.new(|cx| CommandState::new(window, cx));
        let ask_input = cx.new(|cx| InputState::new(window, cx));
        let mut subscriptions = vec![cx.subscribe_in(
            &ask_input,
            window,
            |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.answer_question(true, true, window, cx);
                }
            },
        )];
        let (inbox, inbox_subscriptions) = Self::make_inbox(window, cx);
        subscriptions.extend(inbox_subscriptions);
        let preferences = cx.new(|cx| PreferencesView::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &preferences,
            window,
            |this, _, event: &PreferencesEvent, window, cx| match event {
                PreferencesEvent::Close => {
                    this.sheet = None;
                    this.take_focus(window, cx);
                    cx.notify();
                }
                PreferencesEvent::Shortcuts => {
                    this.sheet = Some(WindowSheet::Shortcuts);
                    this.take_focus(window, cx);
                    cx.notify();
                }
                PreferencesEvent::ReloadKeys => this.reload_keys(window, cx),
                PreferencesEvent::ResetKeys => {
                    this.sheet = Some(WindowSheet::Shortcuts);
                    this.take_focus(window, cx);
                    this.shortcuts
                        .update(cx, |view, cx| view.confirm_reset_all(window, cx));
                    cx.notify();
                }
                PreferencesEvent::ExampleTheme => this.write_example_theme(window, cx),
                // Every open repository is read again: its session follows
                // the git that is run now, and its configuration with it.
                PreferencesEvent::GitChanged => {
                    for tab in &this.tabs {
                        if let TabKind::Repo(workspace) = &tab.kind {
                            workspace.update(cx, |workspace, cx| workspace.refresh(cx));
                        }
                    }
                }
                PreferencesEvent::SwitchProfile(name) => {
                    this.switch_profile(name.clone(), window, cx)
                }
                PreferencesEvent::UseProfileHere(name) => {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        let name = name.clone();
                        workspace.update(cx, |w, cx| w.use_profile_here(name, window, cx));
                    }
                }
                PreferencesEvent::ProfilesChanged => this.load_profiles(window, cx),
                PreferencesEvent::Updates(ask) => this.on_update_ask(*ask, window, cx),
            },
        ));
        let notices = cx.new(NoticesView::new);
        subscriptions.push(cx.subscribe_in(
            &notices,
            window,
            |this, _, event: &NoticesEvent, window, cx| match event {
                NoticesEvent::Close => {
                    this.sheet = None;
                    this.take_focus(window, cx);
                    cx.notify();
                }
            },
        ));
        let tab_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("shell-find-tab")));
        subscriptions.push(cx.subscribe_in(
            &tab_search,
            window,
            |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => {
                    if let Some(first) = this.tabs_found(cx).first().map(|(ix, ..)| *ix) {
                        this.activate(first, cx);
                        this.set_tab_list(false, window, cx);
                    }
                }
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        ));
        let repo_finder = finder::Finder::new(
            finder::Which::Repository,
            say!("shell-find-repository"),
            &mut subscriptions,
            window,
            cx,
        );
        let branch_finder = finder::Finder::new(
            finder::Which::Branch,
            say!("shell-find-branch"),
            &mut subscriptions,
            window,
            cx,
        );
        let locate_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("shell-locate-placeholder")));
        let key_list = cx.new(|cx| crate::views::key_list::KeyListView::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &key_list,
            window,
            |this, _, event: &crate::views::key_list::KeyListEvent, window, cx| match event {
                crate::views::key_list::KeyListEvent::Close => {
                    if this.sheet == Some(WindowSheet::KeyList) {
                        this.sheet = None;
                    }
                    this.take_focus(window, cx);
                    cx.notify();
                }
            },
        ));
        let about = cx.new(crate::views::about::AboutView::new);
        subscriptions.push(cx.subscribe_in(
            &about,
            window,
            |this, _, event: &crate::views::about::AboutEvent, window, cx| match event {
                crate::views::about::AboutEvent::Close => {
                    if this.sheet == Some(WindowSheet::About) {
                        this.sheet = None;
                    }
                    this.take_focus(window, cx);
                    cx.notify();
                }
            },
        ));
        let shortcuts = cx.new(|cx| ShortcutsView::new(window, cx));
        subscriptions.push(cx.subscribe_in(
            &shortcuts,
            window,
            |this, _, event: &ShortcutsEvent, window, cx| match event {
                ShortcutsEvent::Close => {
                    this.sheet = None;
                    this.take_focus(window, cx);
                    cx.notify();
                }
                // The sheet wrote the file and read it: what the file looks
                // like now is known, and is not read a second time.
                ShortcutsEvent::Written => {
                    this.config_seen = cx
                        .global::<Keys>()
                        .file
                        .as_deref()
                        .map(crate::keymap::FileStamp::of);
                    cx.notify();
                }
            },
        ));
        let clone_parent = cx
            .global::<Prefs>()
            .state
            .clone_dir
            .clone()
            .filter(|dir| dir.is_dir())
            .or_else(home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        // Git's questions arrive from the askpass server's threads.
        if let Some(mut events) = cx
            .try_global::<crate::askpass::Askpass>()
            .is_some()
            .then(|| cx.global_mut::<crate::askpass::Askpass>().events.take())
            .flatten()
        {
            cx.spawn_in(window, async move |this, cx| {
                while let Some(event) = events.next().await {
                    let shown = this.update_in(cx, |this, window, cx| {
                        this.on_ask_event(event, window, cx);
                    });
                    if shown.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        // What later starts of the application hand over to this one.
        if let Some(mut incoming) = cx
            .try_global::<crate::instance::Incoming>()
            .is_some()
            .then(|| cx.global_mut::<crate::instance::Incoming>().events.take())
            .flatten()
        {
            cx.spawn_in(window, async move |this, cx| {
                while let Some(open) = incoming.next().await {
                    let shown = this.update_in(cx, |this, window, cx| {
                        this.open_incoming(open, window, cx);
                    });
                    if shown.is_err() {
                        break;
                    }
                }
            })
            .detach();
        }
        let mut shell = Self {
            focus_handle,
            tabs: Vec::new(),
            active: 0,
            sheet: None,
            preferences,
            shortcuts,
            key_list,
            about,
            notices,
            repositories: None,
            inbox,
            locate_input,
            tab_list_open: false,
            tab_search,
            tab_drag: None,
            tab_room: Rc::new(Cell::new((f32::NAN, f32::NAN))),
            tab_first: Cell::new(0),
            hovered_tab: None,
            // The theme placed them before the window was drawn at all.
            toasts_zoom: f32::NAN,
            buttons_zoom: f32::NAN,
            repo_finder,
            branch_finder,
            locating: None,
            locate_error: None,
            profiles: Vec::new(),
            profile_now: None,
            profile_state: Default::default(),
            commands: bitbraid_integrations::registry().iter().cloned().collect(),
            param_prompt: None,
            config_seen: None,
            themes_seen: Vec::new(),
            palette,
            palette_open: false,
            ask_input,
            asking: None,
            ask_queue: VecDeque::new(),
            forms: Forms::new(Vec::new()),
            clone_parent,
            clone_suggested: String::new(),
            ignore_templates: Vec::new(),
            creating: None,
            provider_places: providers::Places::default(),
            terminals: cx.new(|_| crate::views::terminal::sessions::Terminals::new()),
            _subscriptions: subscriptions,
        };
        let remembered = cx.global::<Prefs>().state.clone();
        for path in &remembered.open {
            if path.is_dir() {
                shell.open_repository(path.clone(), window, cx);
            }
        }
        if shell.tabs.is_empty() {
            shell.new_start_tab(window, cx);
        } else {
            // The tab of the repository that was shown, wherever the ones
            // that are gone left it; and kept as shown: opening each tab
            // writes down the last one opened as the one shown, which the
            // next start would otherwise take.
            let shown = remembered.open.get(remembered.active);
            shell.active = shell
                .tabs
                .iter()
                .position(|tab| tab.path.is_some() && tab.path.as_ref() == shown)
                .unwrap_or_else(|| remembered.active.min(shell.tabs.len().saturating_sub(1)));
            shell.persist_tabs(cx);
        }
        if let Some(path) = initial {
            let path = bitbraid_core::paths::canonical(&path).unwrap_or(path);
            shell.open_repository_with(path, true, None, window, cx);
        }
        if let Some(link) = link {
            // Once the window stands: following a link may have to ask or
            // to say something, and there is nothing to say it in before.
            cx.defer_in(window, move |shell, window, cx| {
                shell.open_link(&link, window, cx)
            });
        }
        // What went wrong while the preferences were read is said once, and
        // only now that there is a window to say it in.
        let problems = std::mem::take(&mut cx.global_mut::<Prefs>().problems);
        if !problems.is_empty() {
            cx.defer_in(window, move |_, window, cx| {
                for problem in problems {
                    crate::notices::push(window, cx, Notice::warning(problem).autohide(false));
                }
            });
        }
        shell.ask_git(window, cx);
        shell.load_profiles(window, cx);
        shell.load_ignore_templates(window, cx);
        // What is wrong with the keys of the user's own, likewise.
        let complaints = cx.global::<Keys>().complaints();
        if !complaints.is_empty() {
            cx.defer_in(window, move |_, window, cx| {
                crate::notices::push(window, cx, keys_notice(&complaints));
            });
        }
        let mut themes = cx.global::<theme::UserThemes>().problems.clone();
        let chosen = cx.global::<Prefs>().preferences.theme.clone();
        if !theme::exists(&chosen, cx) {
            themes.insert(
                0,
                say!("shell-theme-missing", theme = format!("{:?}", chosen.key())),
            );
        }
        if !themes.is_empty() {
            cx.defer_in(window, move |_, window, cx| {
                crate::notices::push(window, cx, themes_notice(&themes));
            });
        }
        shell.watch_config(window, cx);
        // Keys are for whatever has the focus, and to begin with nothing
        // has it; nor has anything once the view that had it is gone.
        let lost = cx.on_focus_lost(window, |shell, window, cx| shell.take_focus(window, cx));
        shell._subscriptions.push(lost);
        // What the palette ran may change what the pages outside the
        // repositories show.
        let ran = cx.observe_global_in::<CommandRan>(window, |shell, window, cx| {
            shell.after_command(window, cx)
        });
        shell._subscriptions.push(ran);
        // What this copy is, and a newer version while updates are on.
        shell.start_updates(window, cx);
        cx.defer_in(window, |shell, window, cx| shell.take_focus(window, cx));
        shell.start_tab_bench(window, cx);
        shell
    }

    /// A command run from the palette came back done: the pages that
    /// show what lies outside a repository read again what it may have
    /// changed. The page of repositories, when it is on show, reads its
    /// workspaces and asks git about its rows (any command may change a
    /// branch or what is uncommitted); the start pages and the inbox read
    /// the workspaces after a workspace's command, and the inbox reads its
    /// rows again after one of its own (a pin, a snooze).
    fn after_command(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = cx.try_global::<CommandRan>().map(|ran| ran.id.clone()) else {
            return;
        };
        if self.sheet == Some(WindowSheet::Repositories)
            && let Some(page) = self.repositories.clone()
        {
            page.update(cx, |page, cx| page.read_again(window, cx));
        }
        if id.starts_with("workspace.") {
            for tab in &self.tabs {
                if let TabKind::Start(start) = &tab.kind {
                    start.update(cx, |start, cx| start.read_workspaces(cx));
                }
            }
            self.inbox.update(cx, |inbox, cx| inbox.load_workspaces(cx));
        }
        if id.starts_with("inbox.") && !crate::providers::accounts(cx).is_empty() {
            self.inbox.update(cx, |inbox, cx| inbox.read(false, cx));
        }
    }

    /// Give the focus to what is on show: the window's own view, else the
    /// graph of the tab, else the window itself.
    fn take_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.sheet {
            Some(WindowSheet::Preferences) => {
                self.preferences.read(cx).focus_handle(cx).focus(window, cx);
                return;
            }
            Some(WindowSheet::Shortcuts) => {
                self.shortcuts
                    .update(cx, |view, cx| view.focus_filter(window, cx));
                return;
            }
            Some(WindowSheet::Notices) => {
                self.notices.read(cx).focus_handle(cx).focus(window, cx);
                return;
            }
            Some(WindowSheet::KeyList) => {
                self.key_list
                    .update(cx, |view, cx| view.focus_filter(window, cx));
                return;
            }
            Some(WindowSheet::About) => {
                self.about.read(cx).focus_handle(cx).focus(window, cx);
                return;
            }
            Some(WindowSheet::Repositories) => {
                if let Some(page) = self.repositories.clone() {
                    page.update(cx, |page, cx| page.focus_search(window, cx));
                    return;
                }
            }
            Some(WindowSheet::Inbox) => {
                self.inbox
                    .update(cx, |view, cx| view.focus_search(window, cx));
                return;
            }
            None => {}
        }
        let graph = self
            .active_workspace()
            .cloned()
            .is_some_and(|workspace| workspace.update(cx, |w, cx| w.focus_graph(window, cx)));
        if !graph {
            self.focus_handle.focus(window, cx);
        }
    }

    /// Ask the git that is run what version it is, and warn when there is
    /// none or it is too old. It is a process, so it is asked off the main
    /// thread and the window does not wait for it.
    fn ask_git(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let run = crate::session::run_standalone("git.info", serde_json::json!({}));
        cx.spawn_in(window, async move |_, cx| {
            let git = match run.reply.await {
                Ok(crate::session::Reply::Command(value)) => GitExecutable::from_json(&value),
                Ok(crate::session::Reply::Error(err)) => GitExecutable::failed(err),
                _ => return,
            };
            let _ = cx.update(|window, cx| {
                if let Some(warning) = git.warning() {
                    crate::notices::push(window, cx, Notice::warning(warning).autohide(false));
                }
                cx.set_global(git);
                cx.refresh_windows();
            });
        })
        .detach();
    }

    /// Read the profiles and which is the current one, for the control
    /// beside the tabs, the palette and the preferences. The first time
    /// there are none, one is made from git's identity.
    fn load_profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let remembered = cx.global::<Prefs>().state.profile.clone();
        cx.spawn_in(window, async move |this, cx| {
            let profiles = crate::views::profiles::read(remembered).await;
            let _ = this.update(cx, |this, cx| {
                this.profiles = profiles.names();
                this.profile_now = profiles.current.clone();
                this.preferences
                    .update(cx, |view, cx| view.set_profiles(profiles.clone(), cx));
                this.profile_state = profiles;
                cx.notify();
            });
        })
        .detach();
    }

    /// Switch to a profile: its identity becomes the user's own in git's
    /// configuration, the tabs that are open are put away under the
    /// profile that is left, and the tabs that were open under this one
    /// the last time come back.
    fn switch_profile(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        let run = crate::session::run_standalone(
            "profile.use",
            serde_json::json!({ "name": name, "scope": "user" }),
        );
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Ok(crate::session::Reply::Command(_)) => {
                    let open: Vec<PathBuf> = this
                        .tabs
                        .iter()
                        .filter_map(|tab| tab.path.clone())
                        .collect();
                    let active = this.active;
                    let prefs = cx.global_mut::<Prefs>();
                    let tabs = prefs.state.switch_profile(&name, open, active);
                    prefs.save_state();
                    this.profile_now = Some(name.clone());
                    // Its address, for the picture, is read with the rest.
                    this.load_profiles(window, cx);
                    if let Some(tabs) = tabs {
                        this.replace_tabs(tabs, window, cx);
                    }
                    // Who commits is said in every tab that is open.
                    for tab in &this.tabs {
                        if let TabKind::Repo(workspace) = &tab.kind {
                            workspace.update(cx, |workspace, cx| workspace.identity_changed(cx));
                        }
                    }
                    crate::notices::push(
                        window,
                        cx,
                        Notice::success(say!("shell-profile-switched", name = &name)),
                    );
                    cx.notify();
                }
                Ok(crate::session::Reply::Error(err)) => {
                    crate::notices::push(window, cx, Notice::error(sentence(&err)))
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Close every tab and open a set of tabs in their place.
    fn replace_tabs(
        &mut self,
        set: crate::prefs::TabSet,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tabs.clear();
        self.active = 0;
        for path in &set.open {
            if path.is_dir() {
                self.open_repository(path.clone(), window, cx);
            }
        }
        if self.tabs.is_empty() {
            self.new_start_tab(window, cx);
        } else {
            self.active = set.active.min(self.tabs.len().saturating_sub(1));
        }
        self.keep_terminals(cx);
        self.persist_tabs(cx);
        cx.notify();
    }

    /// End the terminals of the repositories no tab shows any more, nor
    /// shows again when a worktree is opened in its place.
    fn keep_terminals(&mut self, cx: &mut Context<Self>) {
        let kept: Vec<PathBuf> = self
            .tabs
            .iter()
            .flat_map(|tab| tab.before.iter().cloned().chain(tab.path.clone()))
            .collect();
        self.terminals
            .update(cx, |terminals, cx| terminals.end_unless(&kept, cx));
    }

    /// Run one of the terminal's commands in the tab that is shown.
    fn terminal_key(
        &mut self,
        key: crate::views::workspace::terminals::TerminalKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |w, cx| w.terminal_key(key, window, cx));
        }
    }

    /// Look every second and a half whether a file of the user's own
    /// changed, and read it again when it did. It is a `stat` of one file.
    fn watch_config(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.config_seen = cx
            .global::<Keys>()
            .file
            .as_deref()
            .map(crate::keymap::FileStamp::of);
        self.themes_seen = Self::themes_stamp(cx);
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(1500))
                    .await;
                let alive = this.update_in(cx, |shell, window, cx| {
                    let now = cx
                        .global::<Keys>()
                        .file
                        .as_deref()
                        .map(crate::keymap::FileStamp::of);
                    if now != shell.config_seen {
                        shell.config_seen = now;
                        shell.reload_keys(window, cx);
                    }
                    let themes = Self::themes_stamp(cx);
                    if themes != shell.themes_seen {
                        shell.themes_seen = themes;
                        shell.reload_themes(window, cx);
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        })
        .detach();
    }

    /// What the folder of the user's themes looks like now.
    fn themes_stamp(cx: &App) -> crate::theme::FolderStamp {
        cx.global::<theme::UserThemes>()
            .dir
            .as_deref()
            .map(theme::folder_stamp)
            .unwrap_or_default()
    }

    /// Read the themes of the user's own again; the one in use follows its
    /// file. What is wrong with a file is said, never more than that.
    fn reload_themes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let problems = theme::reload_user(cx);
        if problems.is_empty() {
            crate::notices::push(window, cx, Notice::success(say!("shell-themes-reloaded")));
        } else {
            crate::notices::push(window, cx, themes_notice(&problems));
        }
        cx.notify();
    }

    /// Write a theme to start from into the folder of themes, unless one of
    /// that name is there.
    fn write_example_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(dir) = cx.global::<theme::UserThemes>().dir.clone() else {
            return;
        };
        let file = dir.join("example.toml");
        let written = if file.symlink_metadata().is_ok() {
            Err(say!("shell-example-theme-there").to_owned())
        } else {
            std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&file, theme::example_file()))
                .map_err(|err| err.to_string())
        };
        match written {
            Ok(()) => crate::notices::push(
                window,
                cx,
                Notice::success(say!(
                    "shell-example-theme-written",
                    file = file.display().to_string()
                )),
            ),
            Err(why) => crate::notices::push(
                window,
                cx,
                Notice::warning(say!("shell-example-theme-not-written", why)),
            ),
        }
    }

    /// Read `keybindings.toml` again and say what came of it.
    fn reload_keys(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let complaints = keymap::reload(cx);
        if complaints.is_empty() {
            crate::notices::push(window, cx, Notice::success(say!("shell-keys-reloaded")));
        } else {
            crate::notices::push(window, cx, keys_notice(&complaints));
        }
        cx.notify();
    }

    /// What the preferences show of the repository whose tab is open, and
    /// which tab's sections are on show, for that tab to keep them current.
    fn preferences_follow_tab(&mut self, cx: &mut Context<Self>) {
        let open = self.sheet == Some(WindowSheet::Preferences);
        let shown = self.active_workspace().cloned().filter(|_| open);
        if open {
            let repo = self.tabs.get(self.active).and_then(|tab| match &tab.kind {
                TabKind::Repo(workspace) => Some(crate::views::preferences::RepoContext {
                    name: tab.title.clone(),
                    path: tab.path.clone().unwrap_or_default(),
                    workspace: workspace.downgrade(),
                }),
                TabKind::Start(_) => None,
            });
            self.preferences.update(cx, |view, _| view.set_repo(repo));
        }
        for tab in &self.tabs {
            if let TabKind::Repo(workspace) = &tab.kind {
                let want = shown.as_ref() == Some(workspace);
                if workspace.read(cx).settings_shown() != want {
                    workspace.update(cx, |workspace, cx| workspace.set_settings_shown(want, cx));
                }
            }
        }
    }

    /// Open the preferences at a section; they stay open when they are.
    pub fn show_preferences(
        &mut self,
        section: crate::views::preferences::Section,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.preferences
            .update(cx, |view, cx| view.show(section, cx));
        self.sheet = Some(WindowSheet::Preferences);
        self.take_focus(window, cx);
        cx.notify();
    }

    /// Show one of the window's own views over the tab, or put it away
    /// when it is what is shown.
    fn toggle_sheet(&mut self, sheet: WindowSheet, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = (self.sheet != Some(sheet)).then_some(sheet);
        if self.sheet == Some(WindowSheet::Notices) {
            cx.global_mut::<crate::notices::Notices>().seen();
        }
        self.take_focus(window, cx);
        cx.notify();
    }

    /// Open what another start of the application handed over: a link as
    /// one the application was started with, a path as a repository.
    fn open_incoming(&mut self, open: String, window: &mut Window, cx: &mut Context<Self>) {
        window.activate_window();
        cx.activate(true);
        // It is opened in a tab; a view of the window's own would hide it.
        self.sheet = None;
        if crate::instance::is_link(&open) {
            self.open_link(&open, window, cx);
        } else {
            self.open_repository(PathBuf::from(open), window, cx);
        }
        cx.notify();
    }

    /// Follow a `bitbraid://` link: open the repository it names and show
    /// what it names in it. A repository that was never open here is asked
    /// about first: opening one lets git act on what its settings say.
    pub fn open_link(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        use bitbraid_engine::link_cmds::{LinkRepository, LinkTarget, parse_link};
        let link = match parse_link(text) {
            Ok(link) => link,
            Err(err) => {
                crate::notices::push(
                    window,
                    cx,
                    Notice::error(say!("shell-link-broken", error = err.to_string())),
                );
                return;
            }
        };
        let jump = match &link.target {
            LinkTarget::None => None,
            // At most 18 digits, none of them a leading nought: a number.
            LinkTarget::PullRequest(number) => number.parse().ok().map(|number| LinkShows::Pull {
                number,
                url: match &link.repository {
                    LinkRepository::Url(url) => Some(url.clone()),
                    LinkRepository::Path(_) => None,
                },
            }),
            other => other.value().map(|name| LinkShows::Name(name.to_owned())),
        };
        let known: Vec<PathBuf> = {
            let state = &cx.global::<Prefs>().state;
            state
                .recent
                .iter()
                .chain(&state.favourites)
                .chain(&state.open)
                .cloned()
                .collect()
        };
        match link.repository {
            LinkRepository::Path(path) => {
                let path = PathBuf::from(path);
                if known.contains(&path) {
                    self.open_and_jump(path, jump, window, cx);
                    return;
                }
                let shell = cx.entity().downgrade();
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let (shell, path, jump) = (shell.clone(), path.clone(), jump.clone());
                    alert
                        .title(say!("shell-link-open-title"))
                        .description(say!(
                            "shell-link-open-description",
                            path = path.display().to_string()
                        ))
                        .show_cancel(true)
                        .on_ok(move |_, window, cx| {
                            let (path, jump) = (path.clone(), jump.clone());
                            let _ = shell.update(cx, |shell, cx| {
                                shell.open_and_jump(path, jump, window, cx)
                            });
                            true
                        })
                });
            }
            LinkRepository::Url(url) => match known.iter().find(|path| comes_from(path, &url)) {
                Some(path) => self.open_and_jump(path.clone(), jump, window, cx),
                None => {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::info(say!("shell-link-clone-first", url = &url)),
                    );
                    self.prompt_clone(window, cx);
                    form::set_row(self, CLONE_FROM, url, window, cx);
                }
            },
        }
    }

    /// Open a repository and, once its graph is there, show what `jump`
    /// names: a branch, a tag or a commit; or, once it is known which of
    /// its remotes are linked to an account, a pull request.
    fn open_and_jump(
        &mut self,
        path: PathBuf,
        jump: Option<LinkShows>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_repository(path, window, cx);
        let (Some(shows), Some(workspace)) = (jump, self.active_workspace().cloned()) else {
            return;
        };
        cx.spawn_in(window, async move |_, cx| {
            // The graph is read on another thread; look until it is in.
            for _ in 0..200 {
                cx.background_executor()
                    .timer(std::time::Duration::from_millis(50))
                    .await;
                let loaded =
                    workspace.read_with(cx, |workspace, _| workspace.commit_count().is_some());
                if loaded {
                    break;
                }
            }
            match shows {
                LinkShows::Name(text) => {
                    let _ = workspace.update_in(cx, |workspace, window, cx| {
                        workspace.jump_to(text, window, cx)
                    });
                }
                LinkShows::Pull { number, url } => {
                    // Which remotes are linked is asked on another thread
                    // too, from what is on this machine.
                    for _ in 0..200 {
                        let known = workspace.read_with(cx, |workspace, cx| {
                            workspace.pulls().read(cx).links_known()
                        });
                        if known {
                            break;
                        }
                        cx.background_executor()
                            .timer(std::time::Duration::from_millis(50))
                            .await;
                    }
                    let _ = workspace.update_in(cx, |workspace, window, cx| {
                        workspace.open_linked_pull(number, url.as_deref(), window, cx)
                    });
                }
            }
        })
        .detach();
    }

    /// A question from git or ssh arrived, or its asker went away.
    pub fn on_ask_event(&mut self, event: AskEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            AskEvent::Asked(question) => {
                self.ask_queue.push_back(question);
                if self.asking.is_none() {
                    self.show_next_question(window, cx);
                }
            }
            AskEvent::Gone(id) => {
                self.ask_queue.retain(|question| question.id != id);
                if self
                    .asking
                    .as_ref()
                    .is_some_and(|question| question.id == id)
                {
                    // Nobody is waiting for this answer any more.
                    self.asking = None;
                    self.clear_ask_input(window, cx);
                    window.close_dialog(cx);
                    cx.defer_in(window, |this, window, cx| {
                        if this.asking.is_none() {
                            this.show_next_question(window, cx);
                        }
                    });
                }
            }
        }
    }

    /// Whatever was typed does not stay in the field.
    fn clear_ask_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ask_input
            .update(cx, |state, cx| state.set_value("", window, cx));
    }

    /// Answer the question on screen with what is in the field, or decline
    /// it, and go on to the next one. `close` is false when the dialog is
    /// closing by itself (Enter, Escape, its corner button). Whichever way
    /// of answering comes first counts; the others find no question left.
    fn answer_question(
        &mut self,
        accept: bool,
        close: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(question) = self.asking.take() else {
            return;
        };
        let typed = self.ask_input.read(cx).value().to_string();
        self.clear_ask_input(window, cx);
        question.answer(accept.then_some(typed));
        if close {
            window.close_dialog(cx);
        }
        // After this dialog is gone, so that closing it cannot take the
        // next one with it.
        cx.defer_in(window, |this, window, cx| {
            if this.asking.is_none() {
                this.show_next_question(window, cx);
            }
        });
    }

    fn show_next_question(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(question) = self.ask_queue.pop_front() else {
            return;
        };
        let (kind, prompt) = (question.kind, question.prompt.clone());
        self.asking = Some(question);
        self.ask_input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.set_masked(kind == AskKind::Secret, window, cx);
        });
        // Which operation is asking, when one runs in the tab on screen.
        let during = self
            .active_workspace()
            .and_then(|workspace| workspace.read(cx).operation_text());
        let input = self.ask_input.clone();
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (for_cancel, for_ok, for_close) = (shell.clone(), shell.clone(), shell.clone());
            let for_enter = shell.clone();
            dialog
                .title(match kind {
                    AskKind::Secret => say!("shell-ask-secret-title"),
                    AskKind::Text => say!("shell-ask-title"),
                    AskKind::Confirm => say!("shell-ask-confirm-title"),
                })
                // A click beside the dialog must not decline for the user.
                .overlay_closable(false)
                .child(
                    v_flex()
                        .gap_2()
                        .text_sm()
                        // Git's own words, with names and addresses in
                        // them: letter by letter.
                        .child(crate::literal::said(prompt.trim().to_owned(), cx))
                        .when(kind != AskKind::Confirm, |column| {
                            column.child(Input::new(&input))
                        })
                        .child(div().text_xs().text_color(theme.muted_foreground).child(
                            match (&during, kind) {
                                (Some(during), AskKind::Secret) => {
                                    say!("shell-ask-secret-during", during)
                                }
                                (None, AskKind::Secret) => say!("shell-ask-secret-note").to_owned(),
                                (Some(during), _) => during.clone(),
                                (None, _) => String::new(),
                            },
                        )),
                )
                // Enter confirms the dialog, which then closes by itself;
                // this runs before `on_close` does.
                .on_ok(move |_, window, cx| {
                    let _ = for_enter.update(cx, |this, cx| {
                        this.answer_question(true, false, window, cx);
                    });
                    true
                })
                .on_close(move |_, window, cx| {
                    // Closed with Escape or the corner button: declined.
                    let _ = for_close.update(cx, |this, cx| {
                        this.answer_question(false, false, window, cx);
                    });
                })
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("ask-cancel")
                                .ghost()
                                .label(say!("shell-cancel"))
                                .on_click(move |_, window, cx| {
                                    let _ = for_cancel.update(cx, |this, cx| {
                                        this.answer_question(false, true, window, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("ask-ok")
                                .primary()
                                .label(if kind == AskKind::Confirm {
                                    say!("shell-ask-yes")
                                } else {
                                    say!("shell-ask-ok")
                                })
                                .on_click(move |_, window, cx| {
                                    let _ = for_ok.update(cx, |this, cx| {
                                        this.answer_question(true, true, window, cx);
                                    });
                                }),
                        ),
                )
        });
        // The dialog takes the focus when it opens; hand it to the field.
        cx.defer_in(window, |this, window, cx| {
            this.ask_input
                .update(cx, |state, cx| state.focus(window, cx));
        });
    }

    fn persist_tabs(&self, cx: &mut App) {
        let open: Vec<PathBuf> = self.tabs.iter().filter_map(|t| t.path.clone()).collect();
        // Which of those is shown: a start page before it is not kept, and
        // does not count.
        let kept: Vec<bool> = self.tabs.iter().map(|t| t.path.is_some()).collect();
        let prefs = cx.global_mut::<Prefs>();
        prefs.state.open = open;
        prefs.state.active = kept_active(&kept, self.active);
        prefs.save_state();
    }

    /// A new tab asked for (the plus, its key, the palette): made, and
    /// shown whatever was on show; the page of repositories and the inbox
    /// make way for it as for a tab chosen. A tab made because the last
    /// one closed is not this: the page it was closed from stays.
    fn new_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.leave_repositories();
        self.leave_inbox(cx);
        self.new_start_tab(window, cx);
        self.take_focus(window, cx);
    }

    fn new_start_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let start = cx.new(|cx| StartPage::new(window, cx));
        let subscription = cx.subscribe_in(
            &start,
            window,
            |this, _, event: &StartEvent, window, cx| match event {
                StartEvent::Open(path) => this.open_repository(path.clone(), window, cx),
                StartEvent::Browse => this.browse_for_repository(window, cx),
                StartEvent::Clone => this.prompt_clone(window, cx),
                StartEvent::Init => this.prompt_init(window, cx),
                StartEvent::Locate(path) => this.prompt_locate(path.clone(), window, cx),
                StartEvent::Workspace(id) => this.reveal_workspace(id, window, cx),
            },
        );
        self._subscriptions.push(subscription);
        self.tabs.push(TabEntry {
            title: say!("shell-new-tab").into(),
            path: None,
            kind: TabKind::Start(start),
            before: Vec::new(),
        });
        self.active = self.tabs.len().saturating_sub(1);
        cx.notify();
    }

    /// Open a repository in a tab (or activate its existing tab).
    pub fn open_repository(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.open_repository_with(path, false, None, window, cx);
    }

    /// Open a repository. `in_place_of` is a tab it takes the place of
    /// (another worktree taking over the tab it was opened from); without
    /// one it gets a tab of its own, or the start tab that is shown. A
    /// repository that is open already is shown where it is.
    fn open_repository_with(
        &mut self,
        path: PathBuf,
        bench: bool,
        in_place_of: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.leave_repositories();
        self.leave_inbox(cx);
        if let Some(ix) = self
            .tabs
            .iter()
            .position(|t| t.path.as_deref() == Some(path.as_path()))
        {
            self.active = ix;
            self.persist_tabs(cx);
            cx.notify();
            return;
        }
        crate::session::milestone("session opening");
        // Usually opened ahead of the window (see `app::run`).
        let opened = match cx
            .try_global::<crate::session::Preloaded>()
            .is_some()
            .then(|| cx.global_mut::<crate::session::Preloaded>().take(&path))
            .flatten()
        {
            Some(preload) => Ok((preload.session, preload.events, preload.replies)),
            None => crate::session::SessionHandle::open(&path)
                .map(|(session, events)| (session, events, None)),
        };
        match opened {
            Ok((session, events, mut startup)) => {
                crate::session::milestone("session open");
                // A linked worktree opened for the first time looks the way
                // the main one does. What was asked for ahead of the window
                // was asked for without that, and is asked for again.
                if let Some(main) =
                    crate::views::worktrees::main_worktree_of(&session.summary.git_dir)
                    && cx.global_mut::<Prefs>().inherit_repo_state(&path, &main)
                {
                    startup = None;
                }
                let workspace =
                    cx.new(|cx| Workspace::new(session, events, startup, bench, window, cx));
                // Its terminals are the window's.
                let terminals = self.terminals.clone();
                workspace.update(cx, |w, cx| w.set_terminals(terminals, window, cx));
                // A worktree or a submodule of it opens as a tab beside it.
                let subscription = cx.subscribe_in(
                    &workspace,
                    window,
                    |this, from, event: &WorkspaceEvent, window, cx| match event {
                        WorkspaceEvent::OpenSettings(section) => {
                            this.show_preferences(*section, window, cx)
                        }
                        WorkspaceEvent::Open(path) => {
                            this.open_repository(path.clone(), window, cx)
                        }
                        // In the tab it was asked from, wherever that is by
                        // now.
                        WorkspaceEvent::OpenInPlace(path) => {
                            let tab = this.tabs.iter().position(
                                |tab| matches!(&tab.kind, TabKind::Repo(open) if open == from),
                            );
                            this.open_repository_with(path.clone(), false, tab, window, cx)
                        }
                        WorkspaceEvent::OpenAt { path, commit } => this.open_and_jump(
                            path.clone(),
                            Some(LinkShows::Name(commit.clone())),
                            window,
                            cx,
                        ),
                    },
                );
                self._subscriptions.push(subscription);
                let title: SharedString =
                    tab_title(&path, &cx.global::<Prefs>().state.aliases).into();
                // In place of the tab that was named, or of a start tab
                // if that is what is active.
                let on_start = self
                    .tabs
                    .get(self.active)
                    .is_some_and(|t| matches!(t.kind, TabKind::Start(_)));
                let replace = tab_to_replace(in_place_of, self.tabs.len(), self.active, on_start);
                // What the tab showed before is kept with it.
                let before = replace
                    .and_then(|ix| self.tabs.get(ix))
                    .map(|tab| {
                        let mut before = tab.before.clone();
                        before.extend(tab.path.clone());
                        before.retain(|shown| *shown != path);
                        before
                    })
                    .unwrap_or_default();
                let entry = TabEntry {
                    title,
                    path: Some(path.clone()),
                    kind: TabKind::Repo(workspace),
                    before,
                };
                if let Some(ix) = replace {
                    if let Some(slot) = self.tabs.get_mut(ix) {
                        *slot = entry;
                    }
                    self.active = ix;
                } else {
                    self.tabs.push(entry);
                    self.active = self.tabs.len().saturating_sub(1);
                }
                cx.global_mut::<Prefs>().touch_recent(&path);
                self.persist_tabs(cx);
                cx.notify();
            }
            Err(err) => {
                // Said once the window stands. While it is being built
                // (a tab from last time, a path on the command line) there
                // is nothing to say it in yet, and saying it anyway takes
                // the application down.
                let said = say!(
                    "shell-cannot-open",
                    path = path.display().to_string(),
                    error = err
                );
                cx.defer_in(window, move |_, window, cx| {
                    crate::notices::push(window, cx, Notice::error(said))
                });
            }
        }
    }

    fn browse_for_repository(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("shell-open-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |shell, window, cx| {
                    shell.open_repository(path, window, cx)
                });
            }
        })
        .detach();
    }

    /// Ask where a repository whose folder is gone is now, and remember it
    /// there: its place in the lists, its layout, its tab's name.
    fn prompt_locate(&mut self, old: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.locating = Some(old.clone());
        self.locate_error = None;
        self.locate_input.update(cx, |state, cx| {
            state.set_value(old.display().to_string(), window, cx)
        });
        let input = self.locate_input.clone();
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let theme = cx.theme();
            let (for_ok, for_enter, for_browse) = (shell.clone(), shell.clone(), shell.clone());
            let error = shell
                .upgrade()
                .and_then(|shell| shell.read(cx).locate_error.clone());
            dialog
                .title(say!("shell-locate-title", name = repo_title(&old)))
                .width(px(560.))
                // Enter looks; a place that will not do keeps the dialog.
                .on_ok(move |_, window, cx| {
                    let _ = for_enter.update(cx, |shell, cx| shell.confirm_locate(window, cx));
                    false
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .text_color(theme.muted_foreground)
                                .child(say!(
                                    "shell-locate-description",
                                    path = old.display().to_string()
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().flex_1().child(Input::new(&input)))
                                .child(
                                    Button::new("locate-browse")
                                        .outline()
                                        .label(say!("shell-locate-browse"))
                                        .on_click(move |_, window, cx| {
                                            let _ = for_browse.update(cx, |shell, cx| {
                                                shell.browse_for_located(window, cx)
                                            });
                                        }),
                                ),
                        )
                        .children(error.map(|error| {
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .text_color(theme.danger)
                                .child(error)
                        })),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("locate-cancel")
                                .ghost()
                                .label(say!("shell-cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("locate-ok")
                                .primary()
                                .label(say!("shell-locate-ok"))
                                .on_click(move |_, window, cx| {
                                    let _ = for_ok
                                        .update(cx, |shell, cx| shell.confirm_locate(window, cx));
                                }),
                        ),
                )
        });
        cx.defer_in(window, |this, window, cx| {
            this.locate_input
                .update(cx, |state, cx| state.focus(window, cx));
        });
    }

    /// Choose the new place in the system's folder chooser.
    fn browse_for_located(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("shell-locate-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.locate_input.update(cx, |state, cx| {
                        state.set_value(path.display().to_string(), window, cx)
                    });
                });
            }
        })
        .detach();
    }

    /// Look whether there is a repository at the place that was typed and,
    /// when there is, move what was remembered about the old place to it.
    fn confirm_locate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(old) = self.locating.clone() else {
            return;
        };
        let typed = self.locate_input.read(cx).value().trim().to_owned();
        let new = expand_home(&typed);
        if typed.is_empty() || !new.is_absolute() {
            self.locate_error = Some(say!("shell-locate-not-full").to_owned());
            window.refresh();
            return;
        }
        // Whether a repository is there is the engine's to say.
        let run = crate::session::run_standalone("repo.open", serde_json::json!({ "path": new }));
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |this, window, cx| match reply {
                Ok(crate::session::Reply::Command(_)) => {
                    let new = bitbraid_core::paths::canonical(&new).unwrap_or(new);
                    let prefs = cx.global_mut::<Prefs>();
                    prefs.state.relocate(&old, &new);
                    prefs.save_state();
                    this.locating = None;
                    this.locate_error = None;
                    window.close_dialog(cx);
                    crate::notices::push(
                        window,
                        cx,
                        Notice::success(say!(
                            "shell-located",
                            name = repo_title(&old),
                            path = new.display().to_string()
                        )),
                    );
                    cx.refresh_windows();
                    cx.notify();
                }
                Ok(crate::session::Reply::Error(err)) => {
                    this.locate_error = Some(sentence(&err));
                    window.refresh();
                }
                _ => {}
            });
        })
        .detach();
    }

    /// Fill the destination from the URL: the folder clones go into, and the
    /// name git would pick. A destination the user typed is left alone.
    fn suggest_clone_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rows = form::row_texts(self, cx);
        let current = rows.get(CLONE_INTO).cloned().unwrap_or_default();
        if !current.is_empty() && current != self.clone_suggested {
            return;
        }
        let url = rows.get(CLONE_FROM).cloned().unwrap_or_default();
        let suggestion = bitbraid_git::clone_dir_name(&url)
            .map(|name| self.clone_parent.join(name).display().to_string())
            .unwrap_or_default();
        if suggestion != current {
            self.clone_suggested = suggestion.clone();
            form::set_row(self, CLONE_INTO, suggestion, window, cx);
        }
    }

    /// Ask what to clone and where to: the large form, with the folder
    /// first and then where it comes from; a shallow clone's branch and
    /// depth show once that is asked for.
    fn prompt_clone(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating.is_some() {
            crate::notices::push(window, cx, Notice::info(say!("shell-one-at-a-time")));
            return;
        }
        self.clone_suggested = String::new();
        let mut asked: Form<Shell> = Form::new(say!("shell-clone-title"), say!("shell-clone-ok"));
        asked.sheet = Some(Sheet {
            place: say!("shell-clone-from-address"),
            icon: gpui_kit::assets::IconName::Link,
            others: self.clone_places(window, cx),
        });
        asked.rows = vec![
            Row {
                placeholder: say!("shell-clone-into-placeholder"),
                button: Some((
                    say!("shell-browse"),
                    Rc::new(|this: &mut Shell, window, cx| {
                        this.browse_for_clone_parent(window, cx)
                    }),
                )),
                ..Row::required(say!("shell-clone-into"), "")
            },
            Row {
                placeholder: say!("shell-clone-from-placeholder"),
                ..Row::required(say!("shell-clone-from"), "")
            },
            Row {
                shown_by: Some(CLONE_SHALLOW),
                ..Row::optional(
                    say!("shell-clone-branch"),
                    "",
                    say!("shell-clone-branch-placeholder"),
                )
            },
            Row {
                placeholder: say!("shell-clone-depth-placeholder"),
                shown_by: Some(CLONE_SHALLOW),
                ..Row::required(say!("shell-clone-depth"), "50")
            },
        ];
        // The keyboard starts where the address goes; the folder follows
        // what is typed there until it is edited.
        asked.begin = CLONE_FROM;
        asked.typed = Some(Rc::new(|this: &mut Shell, window, cx| {
            this.suggest_clone_path(window, cx)
        }));
        asked.checks = vec![
            Check::new(say!("shell-clone-shallow"), false),
            Check::new(say!("shell-clone-submodules"), false),
            Check::new(say!("shell-clone-sparse"), false),
        ];
        // A partial clone, beside the depth of a shallow one.
        asked.choose = Some(Choose {
            label: say!("shell-clone-partial").to_owned(),
            none: say!("shell-clone-partial-none").to_owned(),
            options: CLONE_FILTERS
                .iter()
                .map(|(_, label)| label.text().to_owned())
                .collect(),
        });
        asked.review = Some(Rc::new(|_: &Shell, answers: &Answers| {
            clone_refusal(&answers.row(CLONE_INTO), &answers.row(CLONE_DEPTH)).map(Verdict::Refuse)
        }));
        form::open(
            self,
            asked,
            |this, answers, window, cx| this.confirm_clone(&answers, window, cx),
            window,
            cx,
        );
    }

    /// Choose the folder the clone goes into; the name stays the URL's.
    fn browse_for_clone_parent(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("shell-clone-into-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(parent) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.clone_parent = parent;
                    // Whatever was there came from the old folder.
                    this.clone_suggested = form::row_texts(this, cx)
                        .get(CLONE_INTO)
                        .cloned()
                        .unwrap_or_default();
                    this.suggest_clone_path(window, cx);
                });
            }
        })
        .detach();
    }

    /// Start the clone the form describes.
    fn confirm_clone(&mut self, answers: &Answers, window: &mut Window, cx: &mut Context<Self>) {
        let (typed, url) = (answers.row(CLONE_INTO), answers.row(CLONE_FROM));
        let path = expand_home(&typed);
        // The form held all of this to be so; a clone that began
        // meanwhile is the one thing it could not know.
        let Ok(depth) = clone_depth(&answers.row(CLONE_DEPTH)) else {
            return;
        };
        if self.creating.is_some() || !path.is_absolute() {
            return;
        }
        let branch = answers.row(CLONE_BRANCH);
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| url.clone());
        self.create(
            "repo.clone",
            serde_json::json!({
                "url": url,
                "path": path,
                "branch": (!branch.is_empty()).then_some(branch),
                "depth": depth,
                "filter": answers
                    .chosen
                    .and_then(|ix| CLONE_FILTERS.get(ix))
                    .map(|(filter, _)| *filter),
                "sparse": answers.check(CLONE_SPARSE),
                "recurse_submodules": answers.check(CLONE_SUBMODULES),
            }),
            say!("shell-cloning", name = &name),
            say!("shell-cloned", name = &name),
            path,
            true,
            window,
            cx,
        );
    }

    /// Ask which `.gitignore` templates there are, for the dialog a new
    /// repository is made in.
    fn load_ignore_templates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let run = crate::session::run_standalone("repo.ignore_templates", serde_json::json!({}));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(crate::session::Reply::Command(value)) = run.reply.await {
                let _ = this.update(cx, |this, _| this.ignore_templates = template_names(&value));
            }
        })
        .detach();
    }

    /// Ask what a new repository is called, which folder it is made in,
    /// what its first branch is called and what its `.gitignore` starts
    /// from, and create it: the large form, which shows the full path as
    /// it is typed.
    fn prompt_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating.is_some() {
            crate::notices::push(window, cx, Notice::info(say!("shell-one-at-a-time")));
            return;
        }
        let parent = self.clone_parent.display().to_string();
        let mut asked: Form<Shell> = Form::new(say!("shell-init-title"), say!("shell-init-ok"));
        asked.sheet = Some(Sheet {
            place: say!("shell-init-on-this-machine"),
            icon: gpui_kit::assets::IconName::HardDrive,
            others: self.init_places(window, cx),
        });
        asked.rows = vec![
            Row {
                placeholder: say!("shell-init-name-placeholder"),
                ..Row::required(say!("shell-init-name"), "")
            },
            Row {
                placeholder: say!("shell-init-in-placeholder"),
                button: Some((
                    say!("shell-browse"),
                    Rc::new(|this: &mut Shell, window, cx| this.browse_for_init(window, cx)),
                )),
                ..Row::required(say!("shell-init-in"), parent)
            },
            Row::derived(say!("shell-init-full-path"), |rows: &[String]| {
                let (name, folder) = (
                    rows.get(INIT_NAME).map_or("", String::as_str),
                    rows.get(INIT_IN).map_or("", String::as_str),
                );
                init_path(folder, name)
                    .map(|path| path.display().to_string())
                    .unwrap_or_default()
            }),
            Row::optional(
                say!("shell-init-branch"),
                "",
                say!("shell-init-branch-placeholder"),
            ),
        ];
        // The templates as they are called; their names go to the engine.
        let templates: Vec<String> = self
            .ignore_templates
            .iter()
            .map(|(name, _)| name.clone())
            .collect();
        if !templates.is_empty() {
            asked.choose = Some(Choose {
                label: ".gitignore".to_owned(),
                none: say!("shell-init-gitignore-none").to_owned(),
                options: self
                    .ignore_templates
                    .iter()
                    .map(|(_, title)| title.clone())
                    .collect(),
            });
        }
        asked.checks = vec![Check::new(say!("shell-init-first-commit"), true)];
        asked.review = Some(Rc::new(|_: &Shell, answers: &Answers| {
            init_refusal(&answers.row(INIT_IN), &answers.row(INIT_NAME)).map(Verdict::Refuse)
        }));
        form::open(
            self,
            asked,
            move |this, answers, window, cx| {
                let chosen: Vec<String> = answers
                    .chosen
                    .and_then(|ix| templates.get(ix))
                    .cloned()
                    .into_iter()
                    .collect();
                this.confirm_init(&answers, chosen, window, cx);
            },
            window,
            cx,
        );
    }

    /// Choose the folder of the new repository in the system's chooser.
    fn browse_for_init(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let rx = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("shell-init-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = rx.await
                && let Some(path) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    form::set_row(this, INIT_IN, path.display().to_string(), window, cx);
                });
            }
        })
        .detach();
    }

    /// Make the repository the form describes.
    fn confirm_init(
        &mut self,
        answers: &Answers,
        gitignore: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(path) = init_path(&answers.row(INIT_IN), &answers.row(INIT_NAME)) else {
            return;
        };
        if self.creating.is_some() {
            return;
        }
        let branch = answers.row(INIT_BRANCH);
        let name = repo_title(&path);
        self.create(
            "repo.init",
            serde_json::json!({
                "path": path,
                "branch": (!branch.is_empty()).then_some(branch),
                "first_commit": answers.check(0),
                "gitignore": gitignore,
            }),
            say!("shell-creating", name = &name),
            say!("shell-created", name = &name),
            path,
            false,
            window,
            cx,
        );
    }

    /// Run a command that makes a repository at `path`, show what git
    /// reports in the status bar, and open the repository when it is there.
    #[allow(clippy::too_many_arguments)]
    fn create(
        &mut self,
        id: &'static str,
        params: serde_json::Value,
        label: String,
        done: String,
        path: PathBuf,
        remember_parent: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Kept for a second try that allows what git refused the first.
        let again = (params.clone(), label.clone(), done.clone(), path.clone());
        let crate::session::Standalone {
            reply,
            mut progress,
            cancel,
        } = crate::session::run_standalone(id, params);
        self.creating = Some(Creating {
            label: label.clone(),
            stage: None,
            cancel,
        });
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            while let Some(step) = progress.next().await {
                let alive = this.update(cx, |this, cx| {
                    if let Some(creating) = &mut this.creating {
                        creating.stage = Some((step.stage, step.percent));
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    return;
                }
            }
        })
        .detach();
        cx.spawn_in(window, async move |this, cx| {
            let reply = reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.creating = None;
                cx.notify();
                match reply {
                    Ok(crate::session::Reply::Error(err)) if err.contains("cancelled") => {
                        crate::notices::push(window, cx, Notice::info(say!("shell-stopped", label)))
                    }
                    // Made, but without its first commit (no name and email
                    // configured, say): it is a repository all the same.
                    Ok(crate::session::Reply::Error(err))
                        if err.contains("the repository was created") =>
                    {
                        crate::notices::push(window, cx, Notice::warning(sentence(&err)));
                        this.open_repository(path, window, cx);
                    }
                    // A submodule that is a folder on this machine: git
                    // does not clone from one unless told it may.
                    Ok(crate::session::Reply::Error(err))
                        if id == "repo.clone"
                            && err.contains("transport 'file' not allowed")
                            && again.0.get("allow_local").is_none() =>
                    {
                        let shell = cx.entity().downgrade();
                        let (mut params, label, done, path) = again;
                        if let Some(params) = params.as_object_mut() {
                            params.insert("allow_local".to_owned(), true.into());
                        }
                        let notice = Notice::error(say!("shell-clone-local-submodule"))
                            .action(say!("shell-clone-allow-local"), move |window, cx| {
                                let (params, label, done, path) =
                                    (params.clone(), label.clone(), done.clone(), path.clone());
                                let _ = shell.update(cx, |this, cx| {
                                    this.create(
                                        "repo.clone",
                                        params,
                                        label,
                                        done,
                                        path,
                                        remember_parent,
                                        window,
                                        cx,
                                    )
                                });
                            })
                            .autohide(false);
                        crate::notices::push(window, cx, notice);
                    }
                    Ok(crate::session::Reply::Error(err)) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)))
                    }
                    Ok(answer) => {
                        if remember_parent && let Some(parent) = path.parent() {
                            this.clone_parent = parent.to_path_buf();
                            let prefs = cx.global_mut::<Prefs>();
                            prefs.state.clone_dir = Some(parent.to_path_buf());
                            prefs.save_state();
                        }
                        // A partial clone that the remote would not filter
                        // is a full one: said, not hidden.
                        let unfiltered = matches!(
                            &answer,
                            crate::session::Reply::Command(value)
                                if value.get("partial").and_then(|partial| partial.as_bool())
                                    == Some(false)
                        );
                        let notice = if unfiltered {
                            Notice::warning(say!("shell-clone-not-partial", done))
                        } else {
                            Notice::success(done)
                        };
                        crate::notices::push(window, cx, notice);
                        this.open_repository(path.clone(), window, cx);
                        // What follows it, for one made at a provider too.
                        this.after_created(&path, window, cx);
                    }
                    Err(_) => {}
                }
            });
        })
        .detach();
    }

    fn close_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        self.tabs.remove(ix);
        self.keep_terminals(cx);
        if self.tabs.is_empty() {
            self.new_start_tab(window, cx);
        } else if self.active >= self.tabs.len() {
            self.active = self.tabs.len().saturating_sub(1);
        } else if ix < self.active {
            self.active = self.active.saturating_sub(1);
        }
        self.persist_tabs(cx);
        cx.notify();
        // The keys go to the tab that is shown now.
        cx.defer_in(window, |shell, window, cx| shell.take_focus(window, cx));
    }

    /// Put the tab at `from` where the tab at `to` is; the ones between
    /// move over by one. The tab that is shown stays the one shown.
    fn move_tab(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if from == to || from >= self.tabs.len() || to >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(from);
        self.tabs.insert(to, tab);
        self.active = active_after_move(self.active, from, to);
        self.persist_tabs(cx);
        cx.notify();
    }

    /// Move the tab that is shown one place to the left or right.
    fn nudge_tab(&mut self, right: bool, cx: &mut Context<Self>) {
        let from = self.active;
        let to = if right {
            from.saturating_add(1)
        } else if from == 0 {
            return;
        } else {
            from.saturating_sub(1)
        };
        self.move_tab(from, to, cx);
    }

    /// Ask what a tab is to be called, in the bar: the name it has, to be
    /// changed, cleared or kept. The name belongs to the repository: it is
    /// the tab's name whenever that repository is open.
    fn prompt_rename_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self.tabs.get(ix).and_then(|tab| tab.path.clone()) else {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("shell-tab-name-no-repository")),
            );
            return;
        };
        let current = cx
            .global::<Prefs>()
            .state
            .aliases
            .get(&path.to_string_lossy().into_owned())
            .cloned()
            .unwrap_or_default();
        let shell = cx.entity().downgrade();
        let (for_save, for_clear) = (shell.clone(), shell);
        let (save_path, clear_path) = (path.clone(), path);
        crate::ask::show(
            crate::ask::Ask::new(say!("shell-tab-name"))
                .field(say!("shell-tab-name-placeholder"), &current)
                .changed()
                .go(say!("shell-tab-name-save"), move |texts, _, cx| {
                    let typed = texts.into_iter().next().unwrap_or_default();
                    let _ = for_save.update(cx, |shell, cx| {
                        shell.name_tab(&save_path, &typed, cx);
                    });
                })
                .other(say!("shell-tab-name-clear"), move |_, _, cx| {
                    let _ = for_clear.update(cx, |shell, cx| shell.name_tab(&clear_path, "", cx));
                }),
            window,
            cx,
        );
    }

    /// Give the tabs of a repository this name (none: its folder's), and
    /// remember it.
    fn name_tab(&mut self, path: &Path, typed: &str, cx: &mut Context<Self>) {
        let prefs = cx.global_mut::<Prefs>();
        prefs.set_alias(path, typed);
        let title: SharedString = tab_title(path, &prefs.state.aliases).into();
        for tab in &mut self.tabs {
            if tab.path.as_deref() == Some(path) {
                tab.title = title.clone();
            }
        }
        cx.notify();
    }

    /// Close every tab but one.
    fn close_other_tabs(&mut self, keep: usize, cx: &mut Context<Self>) {
        if keep >= self.tabs.len() {
            return;
        }
        let kept = self.tabs.remove(keep);
        self.tabs.clear();
        self.tabs.push(kept);
        self.active = 0;
        self.keep_terminals(cx);
        self.persist_tabs(cx);
        cx.notify();
    }

    fn on_tab_menu(&mut self, action: &TabMenu, window: &mut Window, cx: &mut Context<Self>) {
        let ix = action.index;
        match action.what {
            TabDo::Rename => self.prompt_rename_tab(ix, window, cx),
            TabDo::Close => self.close_tab(ix, window, cx),
            TabDo::CloseOthers => self.close_other_tabs(ix, cx),
            TabDo::MoveLeft if ix > 0 => self.move_tab(ix, ix.saturating_sub(1), cx),
            TabDo::MoveRight => self.move_tab(ix, ix.saturating_add(1), cx),
            TabDo::MoveLeft => {}
            TabDo::CopyPath => {
                if let Some(path) = self.tabs.get(ix).and_then(|tab| tab.path.as_ref()) {
                    cx.write_to_clipboard(ClipboardItem::new_string(path.display().to_string()));
                }
            }
        }
    }

    /// One tab: an icon for a repository's, its name, the cross that
    /// closes it (on the tab that is shown, a spinner while its repository
    /// loads), its path under the pointer, a menu, and the handle to drag
    /// it by. `pos` is where it stands in the bar now, which a drag may
    /// have changed; `hidden` keeps its place free under a dragged tab.
    /// The line at its left is the tab's before it, or the inbox's
    /// (`tab_edges`).
    fn render_tab(
        &self,
        ix: usize,
        pos: usize,
        entry: &TabEntry,
        hidden: bool,
        crowded: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        // While a pinned tab's view is on show, that is the shown tab.
        let shown = self.shown_tab();
        let active = shown == Some(ix);
        let weak = cx.entity().downgrade();
        let (close_weak, start_weak) = (weak.clone(), weak);
        let said: SharedString = match &entry.path {
            Some(path) => path.display().to_string().into(),
            None => say!("shell-start-tab-tooltip").into(),
        };
        let dragged = DraggedTab {
            index: ix,
            title: entry.title.clone(),
        };
        let has_path = entry.path.is_some();
        let (first, last) = (ix == 0, ix + 1 == self.tabs.len());
        let alone = self.tabs.len() <= 1;
        let loading = match &entry.kind {
            TabKind::Repo(workspace) => !workspace.read(cx).is_loaded(),
            TabKind::Start(_) => false,
        };
        // The shown tab and the one the pointer is over have a cross; every
        // tab keeps its place, so that no tab grows or moves when another
        // is shown or the pointer passes over it.
        let crossed = active || self.hovered_tab == Some(ix);
        let end = if loading {
            Some(
                div()
                    .size(px(16.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(Spinner::new().xsmall())
                    .into_any_element(),
            )
        } else if crossed {
            Some(
                Button::new(("close-tab", ix))
                    .accessibility_label(say!("shell-close-tab"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .on_click(move |_, window, cx| {
                        // Not a click on the tab as well: closing a tab
                        // that is not shown leaves the shown one shown.
                        cx.stop_propagation();
                        let _ = close_weak.update(cx, |shell, cx| shell.close_tab(ix, window, cx));
                    })
                    .into_any_element(),
            )
        } else {
            None
        };
        h_flex()
            .id(("tab", ix))
            .test_support()
            // A tab of a tab list for assistive technology.
            .role(Role::Tab)
            .aria_label(entry.title.clone())
            .aria_selected(active)
            .relative()
            .h_full()
            .flex_initial()
            .min_w(px(TAB_MIN_WIDTH))
            .max_w(px(240.))
            .pl(px(if crowded {
                TAB_INSET_CROWDED
            } else {
                TAB_INSET
            }))
            .pr(px(if crowded { 4. } else { 6. }))
            .gap(px(if crowded { 3. } else { 6. }))
            .items_center()
            .text_sm()
            .cursor_pointer()
            .when(hidden, |tab| tab.opacity(0.))
            // Shown, open towards the toolbar: its colour. Its lines and
            // its accent lie over its box (`tab_edges`): showing a tab or
            // passing over one changes colours, never where anything is.
            .map(|tab| {
                if active {
                    tab.bg(theme.title_bar)
                        .text_color(theme.tab_active_foreground)
                } else {
                    tab.bg(theme.tab)
                        .text_color(theme.tab_foreground)
                        .hover(|style| style.text_color(theme.tab_active_foreground))
                }
            })
            .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
            .on_hover(cx.listener(move |this, over: &bool, _, cx| {
                let now = if *over {
                    Some(ix)
                } else {
                    this.hovered_tab.filter(|hovered| *hovered != ix)
                };
                if now != this.hovered_tab {
                    this.hovered_tab = now;
                    cx.notify();
                }
            }))
            .on_click(cx.listener(move |this, _, _, cx| this.activate(ix, cx)))
            // The middle button closes the tab it is over.
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close_tab(ix, window, cx);
                }),
            )
            // Dragged, the tab itself goes with the pointer, and the others
            // make room where it would land.
            .on_drag(dragged, move |dragged, _, _, cx| {
                let _ = start_weak.update(cx, |shell, cx| {
                    shell.tab_drag = Some((ix, pos));
                    cx.notify();
                });
                let title = dragged.title.clone();
                cx.new(|_| DragTab { title })
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<DraggedTab>, _, cx| {
                    if !event.bounds.contains(&event.event.position) {
                        return;
                    }
                    if let Some((from, to)) = this.tab_drag
                        && to != pos
                    {
                        this.tab_drag = Some((from, pos));
                        cx.notify();
                    }
                }),
            )
            .when(has_path, |tab| {
                tab.child(
                    div()
                        .id(("tab-icon", ix))
                        .flex_none()
                        .child(
                            Icon::new(gpui_kit::assets::IconName::GitBranch)
                                .size_3()
                                .text_color(theme.muted_foreground),
                        )
                        // Its place is asked for in the tests.
                        .test_support(),
                )
            })
            .child(
                // The name takes the room it takes in bold whether it is
                // drawn bold (shown) or not: an unseen bold copy sets the
                // width, and the name lies over it.
                div()
                    .id(("tab-name", ix))
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .invisible()
                            .truncate()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(entry.title.clone()),
                    )
                    .child(
                        div()
                            .absolute()
                            .inset_0()
                            .truncate()
                            .when(active, |name| name.font_weight(FontWeight::SEMIBOLD))
                            .child(entry.title.clone()),
                    )
                    // Its place is asked for in the tests.
                    .test_support(),
            )
            // The cross's place is every tab's, the cross or not.
            .child(
                div()
                    .id(("tab-cross", ix))
                    .w(px(16.))
                    .h_full()
                    .flex_none()
                    .flex()
                    .items_center()
                    .children(end)
                    .test_support(),
            )
            .children(tab_edges::tab_edges(false, active, cx))
            .context_menu(move |menu, _, _| {
                let entry = |what| -> Box<dyn Action> { Box::new(TabMenu { index: ix, what }) };
                // Closing first, then the name; what is ours comes after.
                let mut menu = menu.menu(say!("shell-tab-menu-close"), entry(TabDo::Close));
                if !alone {
                    menu = menu.menu(
                        say!("shell-tab-menu-close-others"),
                        entry(TabDo::CloseOthers),
                    );
                }
                if has_path {
                    menu = menu
                        .menu(say!("shell-tab-menu-name"), entry(TabDo::Rename))
                        .separator()
                        .menu(say!("shell-tab-menu-copy-path"), entry(TabDo::CopyPath));
                }
                if !first || !last {
                    menu = menu.separator();
                }
                if !first {
                    menu = menu.menu(say!("shell-tab-menu-move-left"), entry(TabDo::MoveLeft));
                }
                if !last {
                    menu = menu.menu(say!("shell-tab-menu-move-right"), entry(TabDo::MoveRight));
                }
                menu
            })
            .into_any_element()
    }

    /// Open or close the list of tabs; it opens with nothing typed.
    fn set_tab_list(&mut self, open: bool, window: &mut Window, cx: &mut Context<Self>) {
        if open && !self.tab_list_open {
            self.tab_search
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.tab_list_open = open;
        cx.notify();
    }

    /// The tabs whose name has what is typed in the list's field, in the
    /// order the list shows them: the repositories' first.
    fn tabs_found(&self, cx: &App) -> Vec<(usize, SharedString, bool)> {
        let typed = self.tab_search.read(cx).value().trim().to_lowercase();
        let mut found: Vec<(usize, SharedString, bool)> = self
            .tabs
            .iter()
            .enumerate()
            .filter(|(_, tab)| typed.is_empty() || tab.title.to_lowercase().contains(&typed))
            .map(|(ix, tab)| (ix, tab.title.clone(), matches!(tab.kind, TabKind::Repo(_))))
            .collect();
        found.sort_by_key(|(ix, _, repo)| (!*repo, *ix));
        found
    }

    fn activate(&mut self, ix: usize, cx: &mut Context<Self>) {
        if ix < self.tabs.len() {
            self.leave_repositories();
            self.leave_inbox(cx);
            self.active = ix;
            self.persist_tabs(cx);
            cx.notify();
        }
    }

    /// The tab drawn as the shown one: none while the view of a pinned tab
    /// (the page of repositories, the inbox) is on show in its place.
    fn shown_tab(&self) -> Option<usize> {
        let pinned = matches!(
            self.sheet,
            Some(WindowSheet::Repositories | WindowSheet::Inbox)
        );
        (!pinned).then_some(self.active)
    }

    /// The repository of the tab that is shown, when there is one in it.
    fn active_path(&self) -> Option<PathBuf> {
        self.tabs
            .get(self.active)
            .filter(|tab| matches!(tab.kind, TabKind::Repo(_)))
            .and_then(|tab| tab.path.clone())
    }

    fn active_workspace(&self) -> Option<&Entity<Workspace>> {
        match self.tabs.get(self.active).map(|t| &t.kind) {
            Some(TabKind::Repo(workspace)) => Some(workspace),
            _ => None,
        }
    }

    /// Hand a gitflow request to the repository that is shown.
    fn flow(
        &mut self,
        request: crate::views::flow::FlowRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |w, cx| w.flow(request, window, cx));
        }
    }

    /// Whether the tab that is shown is a repository opened read-only.
    fn active_read_only(&self, cx: &App) -> bool {
        self.active_workspace()
            .is_some_and(|workspace| workspace.read(cx).read_only())
    }

    fn set_zoom(&mut self, zoom: f32, window: &mut Window, cx: &mut Context<Self>) {
        let zoom = zoom.clamp(0.5, 3.0);
        cx.global_mut::<Prefs>().preferences.zoom = zoom;
        cx.global::<Prefs>().save_preferences();
        // The scale itself is set where the window is drawn; every view is
        // drawn again at it.
        let _ = window;
        crate::zoom::set(zoom);
        cx.refresh_windows();
        cx.notify();
    }

    /// The bar of tabs, `width` wide (in points); on macOS the window's
    /// buttons take `buttons` (the toolkit's pixels) at its left.
    fn render_tab_bar(
        &self,
        width: f32,
        buttons: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let width = width - crate::zoom::unpx(buttons);
        // While the history is looked at nothing in it is new.
        let unseen = if self.sheet == Some(WindowSheet::Notices) {
            0
        } else {
            cx.global::<crate::notices::Notices>().unseen
        };
        let weak = cx.entity().downgrade();
        // While a tab is dragged it stands where it would land.
        let mut order: Vec<usize> = (0..self.tabs.len()).collect();
        let dragging = self.tab_drag.filter(|(from, _)| *from < order.len());
        if let Some((from, to)) = dragging {
            order.retain(|ix| *ix != from);
            order.insert(to.min(order.len()), from);
        }
        // With more tabs than fit at their own width, every tab gives up
        // most of its inset.
        let crowded = (self.tabs.len() as f32) * TAB_ROOMY > width - TAB_BAR_FIXED;
        // With more than fit at their least, as many as fit are drawn, the
        // shown one among them, so that none is drawn over the plus and the
        // controls at the right end. Not while one is dragged, which moves
        // among all of them. The room is the bar's last frame's.
        let (taken, left) = self.tab_room.get();
        let fits = if (taken + left).is_finite() && dragging.is_none() {
            ((taken + left) / TAB_MIN_WIDTH).floor().max(1.0) as usize
        } else {
            usize::MAX
        };
        let drawn = tabs_drawn(order.len(), fits, self.active, self.tab_first.get());
        if dragging.is_none() {
            self.tab_first.set(drawn.start);
        }
        let tabs: Vec<AnyElement> = order
            .iter()
            .enumerate()
            .filter(|(pos, _)| drawn.contains(pos))
            .filter_map(|(pos, ix)| {
                let entry = self.tabs.get(*ix)?;
                let hidden = dragging.is_some_and(|(from, _)| from == *ix);
                Some(self.render_tab(*ix, pos, entry, hidden, crowded, cx))
            })
            .collect();
        // What the tabs take and what is left after the plus, for the next
        // frame; a change draws the bar again.
        let measure = |room: Rc<Cell<(f32, f32)>>, which: bool| {
            canvas(
                move |bounds, window, _| {
                    let wide = crate::zoom::unpx(bounds.size.width);
                    let (taken, left) = room.get();
                    let now = if which { (wide, left) } else { (taken, wide) };
                    if (now.0 - taken).abs() > 0.5
                        || (now.1 - left).abs() > 0.5
                        || !(taken + left).is_finite()
                    {
                        room.set(now);
                        window.request_animation_frame();
                    }
                },
                |_, _, _, _| {},
            )
            .absolute()
            .size_full()
        };
        let new_weak = weak;
        // What belongs to the window rather than to a repository sits at
        // the far right of the tabs, 33 from one to the next: the list of
        // tabs, what was said, the preferences, and last who commits. The
        // activity log and the keys are in the status bar, the palette in
        // the toolbar; refreshing, the panels and the theme have their keys
        // and the palette.
        // The way to a tab the bar has no room to show: a panel under the
        // chevron, right-aligned to it, with a field to find a tab by its
        // name, the repositories' tabs under a caption, then the others.
        let found = self.tabs_found(cx);
        let search = self.tab_search.clone();
        let list_weak = cx.entity().downgrade();
        let tab_list = Popover::new("tab-list-panel")
            .anchor(Anchor::TopRight)
            .open(self.tab_list_open)
            .track_focus(&self.tab_search.focus_handle(cx))
            .on_open_change({
                let weak = list_weak.clone();
                move |open, window, cx| {
                    let _ = weak.update(cx, |shell, cx| shell.set_tab_list(*open, window, cx));
                }
            })
            .trigger(
                Button::new("tab-list")
                    .accessibility_label(say!("shell-tab-list"))
                    .ghost()
                    .small()
                    .icon(IconName::ChevronDown)
                    .tooltip(say!("shell-tab-list")),
            )
            .p_0()
            .content(move |_, _, cx| {
                tab_list_panel(&found, &search, &list_weak, cx).into_any_element()
            });
        // How many are new is a number in a small disc at the bell's top
        // right.
        let (badge, badge_text) = (cx.theme().primary, cx.theme().primary_foreground);
        // The history hangs from it: a panel 450 wide under the tabs,
        // right-aligned to the bell, down to 75 above the window's bottom.
        let history = self.notices.clone();
        let history_open = self.sheet == Some(WindowSheet::Notices);
        let shell = cx.entity().downgrade();
        let bell = div()
            .relative()
            .child(
                Popover::new("notices-panel")
                    .anchor(Anchor::TopRight)
                    .offset(px(7.5))
                    .open(history_open)
                    .track_focus(&self.notices.read(cx).focus_handle(cx))
                    // The bell opens and closes it, as a click outside and
                    // Escape close it.
                    .on_open_change(move |open, window, cx| {
                        let _ = shell.update(cx, |shell, cx| {
                            if *open != (shell.sheet == Some(WindowSheet::Notices)) {
                                shell.toggle_sheet(WindowSheet::Notices, window, cx);
                            }
                        });
                    })
                    .trigger(
                        Button::new("notices")
                            .accessibility_label(say!("shell-notifications"))
                            .ghost()
                            .small()
                            .icon(Icon::new(gpui_kit::assets::IconName::Bell))
                            .tooltip(say!("shell-notifications-new", count = unseen)),
                    )
                    .p_0()
                    .content(move |_, window, _| {
                        let height = crate::zoom::unpx(window.viewport_size().height)
                            - crate::views::notices::TOP
                            - crate::views::notices::BELOW;
                        div()
                            .w(px(crate::views::notices::WIDTH))
                            .h(px(height.max(120.)))
                            .child(history.clone())
                    }),
            )
            .when(unseen > 0, |bell| {
                bell.child(
                    div()
                        .absolute()
                        .top(px(-3.))
                        .right(px(-3.))
                        .min_w(px(14.))
                        .h(px(14.))
                        .px(px(3.))
                        .rounded_full()
                        .bg(badge)
                        .text_color(badge_text)
                        .text_size(px(9.))
                        .line_height(px(14.))
                        .text_center()
                        .child(if unseen > 99 {
                            "99+".to_owned()
                        } else {
                            unseen.to_string()
                        }),
                )
            });
        // Drawn pressed while the preferences are open.
        let gear = gpui_kit::component::Selectable::selected(
            Button::new("preferences")
                .accessibility_label(say!("shell-preferences"))
                .ghost()
                .small()
                .icon(IconName::Settings),
            self.sheet == Some(WindowSheet::Preferences),
        )
        .tooltip(say!("shell-preferences"))
        .on_click(cx.listener(|this, _, window, cx| {
            this.toggle_sheet(WindowSheet::Preferences, window, cx)
        }));
        let window_buttons = h_flex()
            .h_full()
            .pl_1()
            .pr(px(10.))
            .gap(px(9.))
            .items_center()
            .child(tab_list)
            .child(bell)
            .child(gear)
            .child(
                // Who commits: a panel with the profiles to switch to.
                profile::panel(&self.profile_state, cx),
            );
        // The line under the bar is every part's but the shown tab's,
        // which is open towards the toolbar.
        let border = cx.theme().border;
        h_flex()
            .w_full()
            .h(px(TAB_BAR_HEIGHT))
            .flex_none()
            .bg(cx.theme().tab_bar)
            // On macOS the window's buttons stand first, on the bar's
            // ground; the room they are in moves the window.
            .when(buttons > Pixels::ZERO, |bar| {
                bar.child(
                    div()
                        .flex_none()
                        .w(buttons)
                        .h_full()
                        .border_b_1()
                        .border_color(border)
                        .on_mouse_down(MouseButton::Left, |event, window, _| {
                            crate::chrome::bar_pressed(event, window)
                        }),
                )
            })
            // The pinned tabs stand first: the page of repositories, then
            // the inbox; the repositories' tabs follow them directly.
            .child(
                h_flex()
                    .id("pinned-tabs")
                    .test_support()
                    .role(Role::TabList)
                    .aria_label(crate::access::PINNED_TABS_NAME.text())
                    .h_full()
                    .flex_none()
                    .child(self.render_repositories_tab(cx))
                    .child(self.render_inbox_tab(cx)),
            )
            .child(
                // The tabs share the room there is: past it they all grow
                // narrower and cut their names, and nothing scrolls.
                h_flex()
                    .id("tabs")
                    .test_support()
                    .role(Role::TabList)
                    .aria_label(crate::access::TABS_NAME.text())
                    .relative()
                    .h_full()
                    .min_w_0()
                    .flex_initial()
                    // A tab that has no room is not drawn past the place
                    // the tabs have.
                    .overflow_hidden()
                    .child(measure(self.tab_room.clone(), true))
                    .on_drop(cx.listener(|this, _: &DraggedTab, _, cx| {
                        if let Some((from, to)) = this.tab_drag.take() {
                            this.move_tab(from, to, cx);
                        }
                        cx.notify();
                    }))
                    .children(tabs),
            )
            .child(
                h_flex()
                    .h_full()
                    .flex_none()
                    .pl(px(8.))
                    .pr_1()
                    .items_center()
                    .border_b_1()
                    .border_color(border)
                    .when(dragging.is_some(), |plus| plus.invisible())
                    .child(
                        Button::new("new-tab")
                            .accessibility_label(say!("shell-new-tab"))
                            .ghost()
                            .small()
                            .icon(IconName::Plus)
                            .on_click(move |_, window, cx| {
                                let _ = new_weak.update(cx, |shell, cx| shell.new_tab(window, cx));
                            }),
                    ),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .h_full()
                    .border_b_1()
                    .border_color(border)
                    // What is left of the bar moves the window where the
                    // window has no title bar of the system's.
                    .when(crate::chrome::IN_TABS, |room| {
                        room.on_mouse_down(MouseButton::Left, |event, window, _| {
                            crate::chrome::bar_pressed(event, window)
                        })
                    })
                    .child(measure(self.tab_room.clone(), false)),
            )
            .child(window_buttons.border_b_1().border_color(border))
    }

    /// One button of the toolbar's middle group: a caption above an icon,
    /// or the icon alone when the words are switched off.
    ///
    /// The caption's middle is 11 under the bar's top and the icon's 33;
    /// the caption takes the button's colour, a little dimmer, so that a
    /// button that cannot be used greys its words as well. While what the
    /// button started runs, a spinner stands where its icon was.
    fn tool_button(
        id: &'static str,
        caption: &'static str,
        icon: impl Into<Icon>,
        spinning: bool,
        labels: bool,
    ) -> Button {
        let icon: Icon = icon.into();
        let picture = if spinning {
            div()
                .size(px(20.))
                .flex()
                .items_center()
                .justify_center()
                .child(Spinner::new().small())
                .into_any_element()
        } else {
            icon.size_5().into_any_element()
        };
        Button::new(id)
            .accessibility_label(caption)
            .ghost()
            .w(px(if labels {
                toolbar_fit::BUTTON_WIDTH
            } else {
                toolbar_fit::BUTTON_NARROW
            }))
            .h(px(TOOLBAR_HEIGHT - 1.))
            .px_0()
            .child(
                v_flex()
                    .h_full()
                    .items_center()
                    .justify_center()
                    .gap(px(5.))
                    .when(labels, |column| {
                        column.pb(px(1.)).child(
                            div()
                                .text_size(px(12.))
                                .line_height(px(14.))
                                .opacity(0.75)
                                .child(caption),
                        )
                    })
                    .child(picture),
            )
    }

    /// A toolbar button that says nothing but its caption: with the words
    /// switched off, the caption is what shows under the pointer.
    fn plain_tool_button(
        id: &'static str,
        caption: &'static str,
        icon: impl Into<Icon>,
        spinning: bool,
        labels: bool,
    ) -> Button {
        let button = Self::tool_button(id, caption, icon, spinning, labels);
        if labels {
            button
        } else {
            button.tooltip(caption)
        }
    }

    /// What the toolbar's middle group can do now.
    pub fn toolbar_state(&self, cx: &App) -> ToolbarState {
        let Some(workspace) = self.active_workspace() else {
            return ToolbarState::default();
        };
        let workspace = workspace.read(cx);
        let running = workspace.running_command();
        ToolbarState {
            busy: running.is_some(),
            spin: running.as_deref().and_then(button_of_command),
            can_stash: workspace.can_stash(),
            can_pop: workspace.can_pop(),
        }
    }

    /// The toolbar in a window `width` points wide.
    fn render_toolbar(&self, width: f32, cx: &mut Context<Self>) -> impl IntoElement {
        let has_repo = self.active_workspace().is_some();
        let terminal_offered = self
            .active_workspace()
            .is_some_and(|workspace| workspace.read(cx).terminal_offered());
        // A repository that is opened read-only keeps the buttons, greyed:
        // what they do would be refused.
        let read_only = self.active_read_only(cx);
        let writable = has_repo && !read_only;
        // While something the user started runs, the middle group is
        // greyed and the button that started it spins.
        let ToolbarState {
            busy,
            spin,
            can_stash,
            can_pop,
        } = self.toolbar_state(cx);
        let lfs_active = self
            .active_workspace()
            .is_some_and(|workspace| workspace.read(cx).lfs_active());
        // A narrow window (a large zoom) keeps the parts apart: the room
        // after the middle group goes first, then the words, then the
        // blocks' width.
        let fit = toolbar_fit::fit(
            width,
            cx.global::<Prefs>().preferences.toolbar_labels,
            lfs_active,
        );
        let (undo_label, redo_label) = self
            .active_workspace()
            .map(|w| w.read(cx).undo_labels())
            .unwrap_or((None, None));
        let remotes = self
            .active_workspace()
            .map(|w| w.read(cx).remote_names())
            .unwrap_or_default();
        let pull_mode = self.active_workspace().map(|w| w.read(cx).pull_button(cx));
        let push_remotes = remotes.clone();
        // Whether pushes go without the pre-push hook; none where there
        // is no such hook to switch.
        let push_hook = self
            .active_workspace()
            .and_then(|w| w.read(cx).push_hook_skipped(cx));
        let push_branch = self
            .active_workspace()
            .map(|w| w.read(cx).head_label())
            .unwrap_or_default();

        // Left: where we are; each block drops a panel to go elsewhere.
        // Never narrower than the blocks, and never drawn over the middle.
        let left = h_flex()
            .flex_1()
            .min_w(px(if has_repo { fit.left } else { 0. }))
            .overflow_hidden()
            .h_full()
            .when(has_repo, |left| {
                left.child(self.render_crumbs(fit.block, cx))
            });

        // Middle: what is done to the repository. The buttons are 52 from
        // one to the next, and what the group keeps free at its end puts
        // it in the same place at any width of the window. In
        // a window too narrow for all of them, what has no room is in the
        // menu of a button after the rest.
        let kept = fit.middle;
        let more = (kept != toolbar_fit::Middle::All).then(|| {
            let disabled = [
                (Tool::Undo, busy || undo_label.is_none() || read_only),
                (Tool::Redo, busy || redo_label.is_none() || read_only),
                (Tool::Pull, busy || !writable),
                (Tool::Push, busy || !writable),
                (Tool::Branch, busy || !writable),
                (Tool::Stash, busy || !writable || !can_stash),
                (Tool::Pop, busy || !writable || !can_pop),
                (Tool::Terminal, !has_repo || !terminal_offered),
            ];
            Self::tool_button(
                "toolbar-more",
                say!("shell-toolbar-more"),
                IconName::Ellipsis,
                false,
                fit.labels,
            )
            .tooltip(say!("shell-toolbar-more-tooltip"))
            .dropdown_menu(move |menu, _, _| {
                let mut menu = menu.min_w(px(180.));
                for (tool, disabled) in disabled {
                    if !kept.shows(tool.id()) {
                        menu = menu.menu_with_disabled(
                            tool.label(),
                            Box::new(ToolbarTool { tool }),
                            disabled,
                        );
                    }
                }
                if lfs_active {
                    let go = |what| -> Box<dyn Action> { Box::new(LfsMove { what }) };
                    let off = busy || !writable;
                    menu = menu
                        .separator()
                        .menu_with_disabled(say!("shell-lfs-fetch"), go(LfsTransfer::Fetch), off)
                        .menu_with_disabled(
                            say!("shell-lfs-checkout"),
                            go(LfsTransfer::Checkout),
                            off,
                        )
                        .menu_with_disabled(say!("shell-lfs-pull"), go(LfsTransfer::Pull), off)
                        .menu_with_disabled(say!("shell-lfs-push"), go(LfsTransfer::Push), off)
                        .menu(say!("shell-lfs-patterns"), Box::new(ShowLfs));
                }
                menu
            })
        });
        let middle = h_flex()
            .flex_none()
            .gap_1()
            .items_center()
            .when(kept.shows("undo"), |middle| {
                middle.child(
                    Self::tool_button(
                        "undo",
                        say!("shell-undo"),
                        IconName::Undo2,
                        spin == Some("undo"),
                        fit.labels,
                    )
                    .tooltip(
                        undo_label
                            .clone()
                            .map(|label| say!("shell-undo-tooltip", action = shortened(&label, 60)))
                            .unwrap_or_else(|| say!("shell-nothing-to-undo").to_owned()),
                    )
                    .disabled(busy || undo_label.is_none() || read_only)
                    .on_click(cx.listener(|this, _, window, cx| this.undo(window, cx))),
                )
            })
            .when(kept.shows("redo"), |middle| {
                middle.child(
                    Self::tool_button(
                        "redo",
                        say!("shell-redo"),
                        IconName::Redo2,
                        spin == Some("redo"),
                        fit.labels,
                    )
                    .tooltip(
                        redo_label
                            .clone()
                            .map(|label| say!("shell-redo-tooltip", action = shortened(&label, 60)))
                            .unwrap_or_else(|| say!("shell-nothing-to-redo").to_owned()),
                    )
                    .disabled(busy || redo_label.is_none() || read_only)
                    .on_click(cx.listener(|this, _, window, cx| this.redo(window, cx))),
                )
            })
            .when(kept.shows("pull"), |middle| {
                middle.child(
                    h_flex()
                        .items_center()
                        .child(
                            Self::plain_tool_button(
                                "pull",
                                say!("shell-pull"),
                                Icon::new(gpui_kit::assets::IconName::ArrowDownToLine),
                                spin == Some("pull"),
                                fit.labels,
                            )
                            .disabled(busy || !writable)
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    if let Some(workspace) = this.active_workspace().cloned() {
                                        workspace.update(cx, |w, cx| w.pull_or_fetch(window, cx));
                                    }
                                },
                            )),
                        )
                        .child(
                            // The other ways to pull (the one picked is this
                            // repository's from then on), and fetching alone.
                            Button::new("pull-more")
                                .accessibility_label(say!("shell-pull-more"))
                                .ghost()
                                .xsmall()
                                .w(px(TOOL_CARET_WIDTH))
                                .px_0()
                                .icon(IconName::ChevronDown)
                                .disabled(busy || !writable)
                                .dropdown_menu(move |menu, _, _| {
                                    let run = |request| -> Box<dyn Action> {
                                        Box::new(ToolbarRemote { request })
                                    };
                                    // What the button does here: picked, not run.
                                    let mut menu =
                                        menu.min_w(px(250.)).label(say!("shell-pull-mode-heading"));
                                    for (mode, label) in PULL_BUTTON {
                                        menu = menu.menu_with_check(
                                            label.text(),
                                            pull_mode.as_deref() == Some(mode),
                                            Box::new(PullWith {
                                                mode: mode.to_owned(),
                                            }),
                                        );
                                    }
                                    // Fetching one remote, and adding one: ours.
                                    menu = menu.separator();
                                    for name in &remotes {
                                        menu = menu.menu(
                                            say!("shell-fetch-remote", name = name.as_str()),
                                            run(RemoteRequest::Fetch(name.clone())),
                                        );
                                    }
                                    menu = menu
                                        .menu(say!("shell-add-remote"), run(RemoteRequest::Add));
                                    menu
                                }),
                        ),
                )
            })
            .when(kept.shows("push"), |middle| {
                middle.child({
                    let push = Self::plain_tool_button(
                        "push",
                        // The button says it while the hook is off, in as many
                        // letters as its place has room for.
                        if push_hook == Some(true) {
                            say!("shell-push-no-hook")
                        } else {
                            say!("shell-push")
                        },
                        Icon::new(gpui_kit::assets::IconName::ArrowUpFromLine),
                        spin == Some("push"),
                        fit.labels,
                    )
                    .tooltip(say!("shell-push-tooltip"))
                    .disabled(busy || !writable)
                    .on_click(cx.listener(|this, _, window, cx| {
                        if let Some(workspace) = this.active_workspace().cloned() {
                            workspace.update(cx, |w, cx| w.push(window, cx));
                        }
                    }));
                    // Nothing stands beside Push. The pushes that overwrite,
                    // which ask first, the ones that take tags along and the
                    // ones to a remote by name are the menu of the button, on
                    // the pointer's other button.
                    let push = div().id("push-menu").child(push);
                    if writable {
                        push.context_menu(move |menu, _, _| {
                            push_menu(menu, &push_branch, &push_remotes, push_hook)
                        })
                        .into_any_element()
                    } else {
                        push.into_any_element()
                    }
                })
            })
            .when(kept == toolbar_fit::Middle::All, |middle| {
                middle
                    .child(
                        Self::plain_tool_button(
                            "new-branch",
                            say!("shell-branch"),
                            Icon::new(gpui_kit::assets::IconName::GitBranch),
                            spin == Some("new-branch"),
                            fit.labels,
                        )
                        .disabled(busy || !writable)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(workspace) = this.active_workspace().cloned() {
                                workspace.update(cx, |w, cx| w.prompt_new_branch(None, window, cx));
                            }
                        })),
                    )
                    .child(
                        Self::plain_tool_button(
                            "stash",
                            say!("shell-stash"),
                            Icon::new(gpui_kit::assets::IconName::Archive),
                            spin == Some("stash"),
                            fit.labels,
                        )
                        .disabled(busy || !writable || !can_stash)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(workspace) = this.active_workspace().cloned() {
                                workspace
                                    .update(cx, |w, cx| w.stash(StashRequest::PushNow, window, cx));
                            }
                        })),
                    )
                    .child(
                        Self::plain_tool_button(
                            "pop",
                            say!("shell-pop"),
                            Icon::new(gpui_kit::assets::IconName::ArchiveRestore),
                            spin == Some("pop"),
                            fit.labels,
                        )
                        .disabled(busy || !writable || !can_pop)
                        .on_click(cx.listener(|this, _, window, cx| {
                            if let Some(workspace) = this.active_workspace().cloned() {
                                workspace.update(cx, |w, cx| {
                                    w.stash(StashRequest::Pop(None), window, cx)
                                });
                            }
                        })),
                    )
                    // A rule, then the terminal inside the window: shown,
                    // started when there is none, or put away.
                    .child(
                        div()
                            .ml(px(-2.))
                            .mr(px(1.))
                            .mt(px(2.))
                            .w(px(1.))
                            .h(px(TOOLBAR_HEIGHT - 3.))
                            .flex_none()
                            .bg(cx.theme().border),
                    )
                    .child(
                        Self::tool_button(
                            "terminal",
                            say!("shell-terminal"),
                            Icon::new(gpui_kit::assets::IconName::Terminal),
                            false,
                            fit.labels,
                        )
                        .when(fit.labels, |button| {
                            button.w(px(toolbar_fit::TERMINAL_WIDTH))
                        })
                        .tooltip(
                            match cx
                                .global::<Keys>()
                                .resolved
                                .keys_of(crate::views::terminal::TOGGLE_ID)
                                .first()
                            {
                                Some(key) => {
                                    say!(
                                        "shell-terminal-tooltip-key",
                                        key = keymap::display_key(key)
                                    )
                                }
                                None => say!("shell-terminal-tooltip").to_owned(),
                            },
                        )
                        .disabled(!has_repo || !terminal_offered)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.terminal_key(TerminalKey::Toggle, window, cx)
                        })),
                    )
                    // Large files have a control of their own while a pattern is
                    // tracked: three letters and a caret, 70 after Pop (after the
                    // terminal's button), whose menu has what can be done with
                    // them. (The group is centred with it, so it stands half its
                    // width further left.)
                    .when(lfs_active, |middle| {
                        middle.child(
                            div().ml(px(LFS_GAP - TERMINAL_SPAN - 4.)).child(
                                Button::new("lfs")
                                    .accessibility_label(say!("shell-lfs"))
                                    .ghost()
                                    .small()
                                    .w(px(LFS_WIDTH))
                                    .px(px(4.))
                                    .child(
                                        h_flex()
                                            .gap_1()
                                            .items_center()
                                            .text_size(px(15.))
                                            .child(say!("shell-lfs-button"))
                                            .child(Icon::new(IconName::ChevronDown).size_3()),
                                    )
                                    .disabled(busy || !writable)
                                    .dropdown_menu(move |menu, _, _| {
                                        let go = |what| -> Box<dyn Action> {
                                            Box::new(LfsMove { what })
                                        };
                                        menu.menu(say!("shell-lfs-fetch"), go(LfsTransfer::Fetch))
                                            .menu(
                                                say!("shell-lfs-checkout"),
                                                go(LfsTransfer::Checkout),
                                            )
                                            .menu(say!("shell-lfs-pull"), go(LfsTransfer::Pull))
                                            .menu(say!("shell-lfs-push"), go(LfsTransfer::Push))
                                            .separator()
                                            .menu(say!("shell-lfs-patterns"), Box::new(ShowLfs))
                                    }),
                            ),
                        )
                    })
            })
            .children(more);

        // What the middle group keeps free at its end; it is what gives
        // way first in a narrow window.
        let trail = div().h_full().min_w_0().w(px(if lfs_active {
            TOOL_GROUP_TRAIL + 2.0 * LFS_SHIFT - LFS_GAP - LFS_WIDTH
        } else {
            TOOL_GROUP_TRAIL - TERMINAL_SPAN
        }));

        // Right: every command by its name, and finding commits. The last
        // button's middle is 26 from the window's edge.
        let right = h_flex()
            .flex_1()
            .min_w(px(fit.right))
            .justify_end()
            .gap_1()
            .child(
                Self::plain_tool_button(
                    "palette",
                    say!("shell-actions"),
                    Icon::new(gpui_kit::assets::IconName::SquareTerminal),
                    spin == Some("palette"),
                    fit.labels,
                )
                .on_click(cx.listener(|this, _, window, cx| this.open_palette(window, cx))),
            )
            .child(
                Self::plain_tool_button(
                    "search",
                    say!("shell-search"),
                    IconName::Search,
                    spin == Some("search"),
                    fit.labels,
                )
                .disabled(!has_repo)
                .on_click(cx.listener(|this, _, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.focus_search(window, cx));
                    }
                })),
            );

        h_flex()
            .w_full()
            .h(px(TOOLBAR_HEIGHT))
            .flex_none()
            .pr(px(2.))
            .items_center()
            .bg(cx.theme().title_bar)
            .border_b_1()
            .border_color(cx.theme().border)
            .child(left)
            .child(middle)
            .child(trail)
            .child(right)
    }

    fn render_status_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let has_repo = self.active_workspace().is_some();
        let (left, count, operation) = match self.active_workspace() {
            Some(workspace) => {
                let workspace = workspace.read(cx);
                (
                    workspace.status_line(),
                    workspace.commit_count(),
                    workspace.operation_text(),
                )
            }
            None => (say!("shell-no-repository").to_owned(), None, None),
        };
        let zoom = (cx.global::<Prefs>().preferences.zoom * 100.0).round();
        // Which git is run; in the colour of trouble when there is none or
        // it is too old.
        let git = cx.try_global::<GitExecutable>().and_then(|git| {
            git.short()
                .map(|short| (short, git.error.is_some() || !git.supported))
        });
        let creating = self.creating.as_ref().map(|creating| {
            let mut text = creating.label.clone();
            match &creating.stage {
                Some((stage, Some(percent))) => text.push_str(&format!(": {stage} {percent} %")),
                Some((stage, None)) => text.push_str(&format!(": {stage}")),
                None => text.push('…'),
            }
            text
        });
        h_flex()
            .w_full()
            .h(px(STATUS_BAR_HEIGHT))
            .flex_none()
            .px_2()
            .gap_3()
            .items_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .bg(cx.theme().title_bar)
            .border_t_1()
            .border_color(cx.theme().border)
            // What waits most at the providers, at the far left.
            .children(self.render_inbox_summary(cx))
            .child(left)
            .child(div().flex_1())
            .when_some(creating, |this, creating| {
                // A clone belongs to no tab yet, so it shows in all of them.
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(cx.theme().foreground)
                        .child(Spinner::new().xsmall())
                        .child(creating)
                        .child(
                            Button::new("cancel-creating")
                                .ghost()
                                .xsmall()
                                .label(say!("shell-stop"))
                                .on_click(cx.listener(|this, _, _, _| {
                                    if let Some(creating) = &this.creating {
                                        creating
                                            .cancel
                                            .store(true, std::sync::atomic::Ordering::SeqCst);
                                    }
                                })),
                        ),
                )
            })
            .when_some(operation, |this, operation| {
                // What is running, and a way to stop it.
                this.child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .text_color(cx.theme().foreground)
                        .child(Spinner::new().xsmall())
                        .child(operation)
                        .child(
                            Button::new("cancel-operation")
                                .ghost()
                                .xsmall()
                                .label(say!("shell-stop"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(workspace) = this.active_workspace().cloned() {
                                        workspace.update(cx, |w, cx| w.cancel_operation(cx));
                                    }
                                })),
                        ),
                )
            })
            .when_some(count, |this, count| this.child(commits_said(count)))
            .when_some(git, |this, (git, bad)| {
                this.child(
                    div()
                        .when(bad, |text| text.text_color(cx.theme().danger))
                        .child(git),
                )
            })
            // What git was run with, and every key: two buttons 33 apart,
            // before the zoom.
            .child(
                h_flex()
                    .gap(px(13.))
                    .items_center()
                    .child(
                        Button::new("activity")
                            .accessibility_label(say!("shell-activity"))
                            .ghost()
                            .w(px(20.))
                            .h(px(20.))
                            .px_0()
                            .child(Icon::new(gpui_kit::assets::IconName::Logs).size_3p5())
                            .tooltip(say!("shell-activity-tooltip"))
                            .disabled(!has_repo)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(workspace) = this.active_workspace().cloned() {
                                    workspace
                                        .update(cx, |workspace, cx| workspace.toggle_activity(cx));
                                }
                            })),
                    )
                    .child(
                        Button::new("cheat-sheet")
                            .accessibility_label(say!("shell-shortcuts"))
                            .ghost()
                            .w(px(20.))
                            .h(px(20.))
                            .px_0()
                            .child(Icon::new(gpui_kit::assets::IconName::Keyboard).size_3p5())
                            .tooltip(say!("shell-shortcuts-tooltip"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.toggle_sheet(WindowSheet::KeyList, window, cx)
                            })),
                    ),
            )
            // The zoom: a magnifier and the size, which open a list of
            // sizes above them, the one in use marked.
            .child(
                Button::new("zoom")
                    .ghost()
                    .xsmall()
                    .icon(IconName::Search)
                    .label(format!("{zoom:.0} %"))
                    .dropdown_menu_with_anchor(Anchor::BottomLeft, move |menu, _, _| {
                        let mut menu = menu;
                        for percent in ZOOM_SIZES {
                            menu = menu.menu_with_check(
                                format!("{percent} %"),
                                (zoom - percent as f32).abs() < 0.5,
                                Box::new(ZoomTo { percent }),
                            );
                        }
                        menu
                    }),
            )
            // Which version this is, in a frame of its own at the end.
            .child(
                div()
                    .px_1p5()
                    .border_1()
                    .border_color(cx.theme().border)
                    .rounded(cx.theme().radius)
                    .child(format!("{} {}", brand::NAME, env!("CARGO_PKG_VERSION"))),
            )
    }

    /// Everything the palette offers: the window's own actions, the other
    /// themes and profiles, the tabs, the recent repositories, and every
    /// command of the engine.
    fn palette_sections(&self, cx: &App) -> Vec<PaletteSection> {
        let keys = &cx.global::<Keys>().resolved;
        let words = |text: &str| -> Vec<SharedString> {
            text.split_whitespace()
                .map(|word| SharedString::from(word.to_owned()))
                .collect()
        };
        // In a repository that is opened read-only, what would be refused
        // is not listed.
        let read_only = self.active_read_only(cx);
        let mut actions = Vec::new();
        for bound in &keys.bound {
            let Some(binding) = keymap::BINDINGS.get(bound.index) else {
                continue;
            };
            if !binding.palette || (read_only && crate::read_only::refuses_binding(binding)) {
                continue;
            }
            let mut keywords = words(binding.keywords.map_or("", |more| more.text()));
            keywords.push(binding.id.into());
            actions.push(PaletteEntry {
                label: binding.label.text().into(),
                detail: None,
                key: bound
                    .keys
                    .first()
                    .map(|key| keymap::display_key(key).into()),
                keywords,
                action: PaletteAction::Binding(bound.index),
            });
        }
        let chosen = cx.global::<Prefs>().preferences.theme.clone();
        for choice in theme::choices(cx) {
            if choice == chosen {
                continue;
            }
            actions.push(PaletteEntry {
                label: say!("shell-palette-theme", theme = theme::label(&choice)).into(),
                detail: None,
                key: None,
                keywords: vec!["theme".into(), "colours".into(), "colors".into()],
                action: PaletteAction::Theme(choice),
            });
        }
        for name in &self.profiles {
            if self.profile_now.as_deref() == Some(name.as_str()) {
                continue;
            }
            actions.push(PaletteEntry {
                label: say!("shell-palette-profile", name).into(),
                detail: None,
                key: None,
                keywords: vec!["profile".into(), "identity".into(), "switch".into()],
                action: PaletteAction::Profile(name.clone()),
            });
        }
        let mut places = Vec::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            places.push(PaletteEntry {
                label: say!("shell-palette-tab", title = tab.title.as_str()).into(),
                detail: None,
                key: keys
                    .keys_of(&format!("tab.go_{}", ix.saturating_add(1)))
                    .first()
                    .map(|key| keymap::display_key(key).into()),
                keywords: vec!["tab".into(), "switch".into()],
                action: PaletteAction::Tab(ix),
            });
        }
        for path in &cx.global::<Prefs>().state.recent {
            places.push(PaletteEntry {
                label: say!("shell-palette-recent", path = path.display().to_string()).into(),
                detail: None,
                key: None,
                keywords: vec!["recent".into(), "open".into()],
                action: PaletteAction::OpenRepo(path.clone()),
            });
        }
        let commands = self
            .commands
            .iter()
            .filter(|spec| !(read_only && crate::read_only::refuses(spec.id)))
            .map(|spec| {
                let mut keywords = words(spec.description);
                keywords.push(spec.id.into());
                PaletteEntry {
                    label: spec.label.into(),
                    detail: Some(spec.id.into()),
                    key: keys.key_for_command(spec.id).map(SharedString::from),
                    keywords,
                    action: PaletteAction::Command(spec.id),
                }
            })
            .collect();
        vec![
            PaletteSection {
                heading: say!("shell-palette-window"),
                entries: actions,
            },
            PaletteSection {
                heading: say!("shell-palette-places"),
                entries: places,
            },
            PaletteSection {
                heading: say!("shell-palette-commands"),
                entries: commands,
            },
        ]
    }

    fn open_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // The key that opens it closes it as well.
        if self.palette_open {
            self.palette_open = false;
            window.close_dialog(cx);
            return;
        }
        self.palette_open = true;
        self.palette
            .update(cx, |palette, cx| palette.set_query("", window, cx));
        let sections: Rc<Vec<PaletteSection>> = Rc::new(self.palette_sections(cx));
        // What is listed for what is typed, as the palette numbers it: the
        // sections on show and the entries of each, best first. Worked out
        // when what is typed changes, not on every frame.
        type Listing = Option<(String, Vec<crate::palette::Listed>)>;
        let listed: Rc<std::cell::RefCell<Listing>> = Rc::new(std::cell::RefCell::new(None));
        let palette = self.palette.clone();
        let shell = cx.weak_entity();
        let focus_on_mount = Rc::new(Cell::new(true));
        window.open_dialog(cx, move |dialog, _, _| {
            let palette = palette.clone();
            let shell = shell.clone();
            let sections = sections.clone();
            let listed = listed.clone();
            let focus_on_mount = focus_on_mount.clone();
            let close_owner = shell.clone();
            // From the toolbar's bottom edge down, 600 wide and centred.
            dialog
                .close_button(false)
                .p_0()
                .width(px(PALETTE_WIDTH))
                .margin_top(px(PALETTE_TOP))
                .on_close(move |_, _, cx| {
                    let _ = close_owner.update(cx, |shell, _| shell.palette_open = false);
                })
                .content(move |content, window, cx| {
                    if focus_on_mount.replace(false) {
                        let palette = palette.clone();
                        window.defer(cx, move |window, cx| {
                            palette.read(cx).focus_handle(cx).focus(window, cx);
                        });
                    }
                    let confirm_shell = shell.clone();
                    let (confirm_sections, confirm_listed) = (sections.clone(), listed.clone());
                    let typed = palette.read(cx).query(cx).to_string();
                    let stale = listed.borrow().as_ref().is_none_or(|(of, _)| *of != typed);
                    if stale {
                        let now = list_palette(&sections, &typed);
                        *listed.borrow_mut() = Some((typed, now));
                    }
                    // The matching is ours, fuzzy and ranked; the toolkit's
                    // own keeps what has the letters in a row, as it comes.
                    // The field's text centred in its 40, and five rows of
                    // 40 on show (with the list's inset), the rest scrolled.
                    let mut command = Command::new(&palette)
                        .bordered(false)
                        .filterable(false)
                        .pt(px(PALETTE_FIELD_TOP))
                        .on_query(|_, window, _| window.refresh())
                        .placeholder(say!("shell-palette-placeholder"))
                        .max_h(px(PALETTE_ROW * PALETTE_ROWS + 8.));
                    if let Some((_, now)) = listed.borrow().as_ref() {
                        for (at, entries) in now {
                            let Some(section) = sections.get(*at) else {
                                continue;
                            };
                            command =
                                command.group(CommandGroup::new().label(section.heading).items(
                                    entries.iter().filter_map(|(entry, found)| {
                                        Some(palette_item(section.entries.get(*entry)?, found))
                                    }),
                                ));
                        }
                    }
                    // Escape closes the palette whatever is typed in it. The
                    // toolkit's own handling empties the field first and
                    // only closes on a second press, so the key is taken on
                    // its way down, before the field sees it.
                    // The key that opened it closes it as well: the dialog
                    // is not under the shell, so it is taken here.
                    let (escape_shell, toggle_shell) = (shell.clone(), shell.clone());
                    let closing = div()
                        .capture_action(
                            move |_: &gpui_kit::component::dialog::Cancel, window, cx| {
                                cx.stop_propagation();
                                let _ =
                                    escape_shell.update(cx, |shell, _| shell.palette_open = false);
                                window.close_dialog(cx);
                            },
                        )
                        .on_action(move |_: &TogglePalette, window, cx| {
                            let _ = toggle_shell.update(cx, |shell, _| shell.palette_open = false);
                            window.close_dialog(cx);
                        });
                    content.child(closing.child(command.on_confirm(move |index, window, cx| {
                        // The row that was picked, in what was listed last.
                        let picked = confirm_listed.borrow().as_ref().and_then(|(_, now)| {
                            let (section, entries) = now.get(index.section)?;
                            Some((*section, entries.get(index.row)?.0))
                        });
                        let Some(entry) = picked.and_then(|(section, entry)| {
                            confirm_sections.get(section)?.entries.get(entry)
                        }) else {
                            return;
                        };
                        let action = match &entry.action {
                            PaletteAction::Binding(ix) => PaletteAction::Binding(*ix),
                            PaletteAction::OpenRepo(path) => PaletteAction::OpenRepo(path.clone()),
                            PaletteAction::Tab(ix) => PaletteAction::Tab(*ix),
                            PaletteAction::Command(id) => PaletteAction::Command(id),
                            PaletteAction::Theme(choice) => PaletteAction::Theme(choice.clone()),
                            PaletteAction::Profile(name) => PaletteAction::Profile(name.clone()),
                        };
                        window.close_dialog(cx);
                        let _ = confirm_shell
                            .update(cx, |shell, cx| shell.run_palette_action(action, window, cx));
                    })))
                })
        });
    }

    fn run_palette_action(
        &mut self,
        action: PaletteAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.palette_open = false;
        match action {
            PaletteAction::Binding(ix) => {
                if let Some(binding) = keymap::BINDINGS.get(ix) {
                    // From what is on show: the palette had the focus, and
                    // it is gone. An action of the graph's own is the
                    // graph's to take.
                    self.take_focus(window, cx);
                    let action = binding.action();
                    cx.defer_in(window, move |_, window, cx| {
                        window.dispatch_action(action, cx)
                    });
                }
            }
            PaletteAction::OpenRepo(path) => self.open_repository(path, window, cx),
            PaletteAction::Tab(ix) => self.activate(ix, cx),
            PaletteAction::Theme(choice) => theme::choose(choice, cx),
            PaletteAction::Profile(name) => self.switch_profile(name, window, cx),
            // The updater's, the window's way: nothing is asked while
            // updates are off, and what comes of it is said.
            PaletteAction::Command("update.check") => self.look_for_updates(true, window, cx),
            PaletteAction::Command("update.install") => self.install_update(window, cx),
            PaletteAction::Command(id) => {
                // Once the palette's dialog is gone: the prompt is one too.
                cx.defer_in(window, move |shell, window, cx| {
                    shell.prompt_command(id, window, cx)
                });
            }
        }
    }

    /// Ask for the parameters of an engine command, in a prompt made from
    /// what the engine says the command takes, and run it. A command that
    /// takes nothing runs at once.
    fn prompt_command(&mut self, id: &'static str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(spec) = self.commands.iter().find(|spec| spec.id == id).cloned() else {
            return;
        };
        let schema = bitbraid_integrations::shared().params(id).unwrap_or(&[]);
        if schema.is_empty() {
            self.run_command(id, spec.label, serde_json::json!({}), window, cx);
            return;
        }
        let fields: Vec<ParamField> = schema
            .iter()
            .map(|param| {
                let options: Vec<&'static str> = match param.kind {
                    bitbraid_engine::ParamKind::Flag => vec!["yes", "no"],
                    bitbraid_engine::ParamKind::Choice(choices) => choices.to_vec(),
                    _ => Vec::new(),
                };
                let (input, options) = if options.is_empty() {
                    let input = cx.new(|cx| {
                        InputState::new(window, cx).placeholder(param_placeholder(param))
                    });
                    (Some(input), options)
                } else if param.required {
                    (None, options)
                } else {
                    // Left alone, the command decides.
                    (None, std::iter::once("").chain(options).collect())
                };
                ParamField {
                    spec: *param,
                    input,
                    options,
                    picked: 0,
                }
            })
            .collect();
        let first = fields.iter().find_map(|field| field.input.clone());
        self.param_prompt = Some(ParamPrompt {
            id,
            label: spec.label,
            fields,
            error: None,
        });
        let shell = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let (for_ok, for_close, for_enter) = (shell.clone(), shell.clone(), shell.clone());
            let rows = shell
                .upgrade()
                .map(|shell| Shell::param_rows(&shell, cx))
                .unwrap_or_default();
            let (title, description, error) = shell
                .upgrade()
                .and_then(|shell| {
                    let shell = shell.read(cx);
                    let prompt = shell.param_prompt.as_ref()?;
                    let description = shell
                        .commands
                        .iter()
                        .find(|spec| spec.id == prompt.id)
                        .map(|spec| spec.description)
                        .unwrap_or_default();
                    Some((
                        format!("{} ({})", prompt.label, prompt.id),
                        // What comes after "params:" is what the rows say.
                        description
                            .split("; params:")
                            .next()
                            .unwrap_or(description)
                            .to_owned(),
                        prompt.error.clone(),
                    ))
                })
                .unwrap_or_default();
            let theme = cx.theme();
            dialog
                .title(title)
                .width(px(560.))
                // Enter runs it; a prompt that cannot be right stays.
                .on_ok(move |_, window, cx| {
                    for_enter
                        .update(cx, |shell, cx| shell.confirm_param_prompt(window, cx))
                        .unwrap_or(true)
                })
                .on_close(move |_, _, cx| {
                    let _ = for_close.update(cx, |shell, _| shell.param_prompt = None);
                })
                .child(
                    v_flex()
                        .gap_2()
                        .child(
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .text_color(theme.muted_foreground)
                                .child(description),
                        )
                        .child(
                            v_flex()
                                .id("param-rows")
                                .gap_2()
                                .max_h(px(380.))
                                .overflow_y_scroll()
                                .children(rows),
                        )
                        .children(error.map(|error| {
                            div()
                                .text_sm()
                                .whitespace_normal()
                                .text_color(theme.danger)
                                .child(error)
                        })),
                )
                .footer(
                    h_flex()
                        .gap_2()
                        .justify_end()
                        .child(
                            Button::new("param-cancel")
                                .ghost()
                                .label(say!("shell-cancel"))
                                .on_click(|_, window, cx| window.close_dialog(cx)),
                        )
                        .child(
                            Button::new("param-ok")
                                .primary()
                                .label(say!("shell-param-run"))
                                .on_click(move |_, window, cx| {
                                    let done = for_ok
                                        .update(cx, |shell, cx| {
                                            shell.confirm_param_prompt(window, cx)
                                        })
                                        .unwrap_or(true);
                                    if done {
                                        window.close_dialog(cx);
                                    }
                                }),
                        ),
                )
        });
        if let Some(first) = first {
            cx.defer_in(window, move |_, window, cx| {
                first.update(cx, |state, cx| state.focus(window, cx));
            });
        }
    }

    /// The rows of the parameter prompt: a name, a few words, and a field
    /// or a button that goes through what can be picked.
    fn param_rows(shell: &Entity<Shell>, cx: &App) -> Vec<AnyElement> {
        let theme = cx.theme();
        let Some(prompt) = shell.read(cx).param_prompt.as_ref() else {
            return Vec::new();
        };
        prompt
            .fields
            .iter()
            .enumerate()
            .map(|(ix, field)| {
                let control: AnyElement = match &field.input {
                    Some(input) => Input::new(input).into_any_element(),
                    None => {
                        let picked = field.options.get(field.picked).copied().unwrap_or("");
                        let shell = shell.downgrade();
                        Button::new(("param-pick", ix))
                            .outline()
                            .small()
                            .label(if picked.is_empty() {
                                say!("shell-param-default")
                            } else {
                                picked
                            })
                            .on_click(move |_, window, cx| {
                                let _ = shell.update(cx, |shell, _| {
                                    if let Some(field) = shell
                                        .param_prompt
                                        .as_mut()
                                        .and_then(|prompt| prompt.fields.get_mut(ix))
                                    {
                                        field.picked = field
                                            .picked
                                            .saturating_add(1)
                                            .checked_rem(field.options.len())
                                            .unwrap_or(0);
                                    }
                                });
                                window.refresh();
                            })
                            .into_any_element()
                    }
                };
                v_flex()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_baseline()
                            .child(
                                div()
                                    .flex_none()
                                    .text_sm()
                                    .font_family(theme.mono_font_family.clone())
                                    .child(field.spec.name),
                            )
                            .when(field.spec.required, |row| {
                                row.child(
                                    div()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(theme.primary)
                                        .child(say!("shell-param-needed")),
                                )
                            })
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_xs()
                                    .whitespace_normal()
                                    .text_color(theme.muted_foreground)
                                    .child(field.spec.description.text()),
                            ),
                    )
                    .child(control)
                    .into_any_element()
            })
            .collect()
    }

    /// Turn what the prompt holds into parameters and run the command; a
    /// prompt that cannot be right says why and stays. True when the prompt
    /// is done with.
    fn confirm_param_prompt(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(prompt) = &self.param_prompt else {
            return true;
        };
        let typed: std::collections::BTreeMap<String, String> = prompt
            .fields
            .iter()
            .map(|field| {
                let text = match &field.input {
                    Some(input) => input.read(cx).value().to_string(),
                    None => field
                        .options
                        .get(field.picked)
                        .copied()
                        .unwrap_or("")
                        .to_owned(),
                };
                (field.spec.name.to_owned(), text)
            })
            .collect();
        let schema: Vec<bitbraid_engine::ParamSpec> =
            prompt.fields.iter().map(|field| field.spec).collect();
        let (id, label) = (prompt.id, prompt.label);
        match bitbraid_engine::params::from_text(&schema, &typed) {
            Ok(params) => {
                self.param_prompt = None;
                self.run_command(id, label, params, window, cx);
                true
            }
            Err(err) => {
                if let Some(prompt) = &mut self.param_prompt {
                    prompt.error = Some(sentence(&err.to_string()));
                }
                window.refresh();
                false
            }
        }
    }

    /// Run an engine command from the palette: in the repository of the
    /// tab, or by itself when no repository is open, and show what it
    /// answers.
    fn run_command(
        &mut self,
        id: &'static str,
        label: &'static str,
        params: serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |workspace, cx| {
                workspace.run_generic(id, label.to_owned(), params, window, cx)
            });
            return;
        }
        let run = crate::session::run_standalone(id, params);
        cx.spawn_in(window, async move |_, cx| {
            let reply = run.reply.await;
            let _ = cx.update(|window, cx| match reply {
                Ok(crate::session::Reply::Command(value)) => {
                    show_command_output(label, &value, window, cx);
                    command_ran(id, cx);
                }
                Ok(crate::session::Reply::Error(err)) => {
                    crate::notices::push(window, cx, Notice::error(sentence(&err)))
                }
                _ => {}
            });
        })
        .detach();
    }

    fn undo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |w, cx| w.undo(window, cx));
        }
    }

    fn redo(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |w, cx| w.redo(window, cx));
        }
    }

    /// What a button of the toolbar's middle group does, as its click
    /// does it: from the menu that has what a narrow window has no room
    /// for.
    fn run_tool(&mut self, tool: Tool, window: &mut Window, cx: &mut Context<Self>) {
        match tool {
            Tool::Undo => return self.undo(window, cx),
            Tool::Redo => return self.redo(window, cx),
            Tool::Terminal => return self.terminal_key(TerminalKey::Toggle, window, cx),
            _ => {}
        }
        let Some(workspace) = self.active_workspace().cloned() else {
            return;
        };
        workspace.update(cx, |w, cx| match tool {
            Tool::Pull => w.pull_or_fetch(window, cx),
            Tool::Push => w.push(window, cx),
            Tool::Branch => w.prompt_new_branch(None, window, cx),
            Tool::Stash => w.stash(StashRequest::PushNow, window, cx),
            Tool::Pop => w.stash(StashRequest::Pop(None), window, cx),
            Tool::Undo | Tool::Redo | Tool::Terminal => {}
        });
    }

    fn refresh(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if let Some(workspace) = self.active_workspace().cloned() {
            workspace.update(cx, |w, cx| w.refresh(cx));
        }
    }

    fn toggle_left(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // What the middle shows holds the panel folded, or has taken its
        // place: opening it is closing that, as the strip's button does.
        if let Some(workspace) = self.active_workspace().cloned()
            && workspace.read(cx).holds_references()
        {
            workspace.update(cx, |workspace, cx| workspace.open_references(window, cx));
            return;
        }
        // In the tab that is shown; with no repository in it, wherever
        // nothing else was chosen.
        let path = self.active_path();
        let prefs = cx.global_mut::<Prefs>();
        let shown = prefs.left_panel_shown(path.as_deref());
        prefs.set_panel_shown(path.as_deref(), true, !shown);
        cx.refresh_windows();
        cx.notify();
    }

    fn toggle_right(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let path = self.active_path();
        let prefs = cx.global_mut::<Prefs>();
        let shown = prefs.right_panel_shown(path.as_deref());
        prefs.set_panel_shown(path.as_deref(), false, !shown);
        cx.refresh_windows();
        cx.notify();
    }
}

impl Focusable for Shell {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// What a `bitbraid://` link shows once its repository is open.
#[derive(Debug, Clone)]
enum LinkShows {
    /// A branch, a tag or a commit, as the link names it.
    Name(String),
    /// A pull request by its number; of the remote at `url` when the link
    /// named its repository by that address.
    Pull { number: u64, url: Option<String> },
}

/// Whether the repository at `path` has a remote whose address is `url`,
/// give or take a `.git` and a slash at the end.
fn comes_from(path: &Path, url: &str) -> bool {
    let plain = |text: &str| {
        let text = text.trim_end_matches('/');
        text.strip_suffix(".git").unwrap_or(text).to_owned()
    };
    let wanted = plain(url);
    let Ok(repo) = bitbraid_git::Repository::open(path) else {
        return false;
    };
    let Ok(snapshot) = bitbraid_git::snapshot(&repo) else {
        return false;
    };
    snapshot.remotes.iter().any(|remote| {
        [&remote.fetch_url, &remote.push_url]
            .into_iter()
            .flatten()
            .any(|found| plain(found) == wanted)
    })
}

/// The user's home directory, where clones go until told otherwise.
fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
}

/// A typed path with a leading `~` standing for the home directory.
pub(crate) fn expand_home(typed: &str) -> PathBuf {
    match typed.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => {
            match home_dir() {
                Some(home) => home.join(rest.trim_start_matches(['/', '\\'])),
                None => PathBuf::from(typed),
            }
        }
        _ => PathBuf::from(typed),
    }
}

/// The menu beside Push: the ways to push `branch`, the checked-out one,
/// and where else it can go.
fn push_menu(
    menu: gpui_kit::component::menu::PopupMenu,
    branch: &str,
    remotes: &[String],
    skip_hook: Option<bool>,
) -> gpui_kit::component::menu::PopupMenu {
    let go = |how| -> Box<dyn Action> { Box::new(PushCurrent { how }) };
    // While the hook is switched off, the plain push says so.
    let plain = if skip_hook == Some(true) {
        say!("shell-push-branch-no-hook", branch)
    } else {
        say!("shell-push-branch", branch)
    };
    let mut menu = menu
        .menu(plain, go(PushHow::Plain))
        .menu(say!("shell-push-lease"), go(PushHow::Lease))
        .menu(say!("shell-push-force"), go(PushHow::Force))
        .separator()
        .menu(
            say!("shell-push-branch-tags", branch),
            go(PushHow::WithTags),
        )
        .menu(
            say!("shell-push-tags"),
            Box::new(crate::keymap::PushAllTags),
        );
    // With more than one remote, the others are a click away.
    if remotes.len() > 1 {
        menu = menu.separator();
        for remote in remotes.iter().take(12) {
            menu = menu.menu(
                say!("shell-push-branch-to", branch, remote),
                go(PushHow::To(remote.clone())),
            );
        }
    }
    // Offered where there is a hook that could refuse a push; it holds
    // for every push from this tab until it is switched back.
    if let Some(on) = skip_hook {
        menu = menu.separator().menu_with_check(
            say!("shell-push-skip-hook"),
            on,
            Box::new(crate::keymap::SkipPushHook),
        );
    }
    menu
}

/// Width of the list of tabs.
const TAB_LIST_WIDTH: f32 = 330.0;
/// Height of a tab's row in it.
const TAB_LIST_ROW: f32 = 47.0;

/// The panel of the list of tabs: a field to find one, then a row per tab
/// (the repositories' under a caption, the others after a rule), each with
/// a cross that closes it. A click on a row goes to the tab.
fn tab_list_panel(
    found: &[(usize, SharedString, bool)],
    search: &Entity<InputState>,
    shell: &WeakEntity<Shell>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let row = |(ix, title, repo): &(usize, SharedString, bool)| {
        let (ix, go, close) = (*ix, shell.clone(), shell.clone());
        Button::new(("tab-list-row", ix))
            .accessibility_label(title.clone())
            .ghost()
            .w_full()
            .h(px(TAB_LIST_ROW))
            .px_3()
            .gap_2()
            .justify_start()
            .rounded_none()
            .on_click(move |_, window, cx| {
                let _ = go.update(cx, |shell, cx| {
                    shell.activate(ix, cx);
                    shell.set_tab_list(false, window, cx);
                });
            })
            .child(div().w(px(16.)).flex_none().when(*repo, |slot| {
                slot.child(
                    Icon::new(gpui_kit::assets::IconName::GitBranch)
                        .size_3p5()
                        .text_color(theme.muted_foreground),
                )
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .child(title.clone()),
            )
            .child(
                Button::new(("tab-list-close", ix))
                    .accessibility_label(say!("shell-close-tab"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::Close)
                    .on_click(move |_, window, cx| {
                        cx.stop_propagation();
                        let _ = close.update(cx, |shell, cx| shell.close_tab(ix, window, cx));
                    }),
            )
    };
    let repos: Vec<_> = found.iter().filter(|(_, _, repo)| *repo).collect();
    let others: Vec<_> = found.iter().filter(|(_, _, repo)| !*repo).collect();
    v_flex()
        .w(px(TAB_LIST_WIDTH))
        .child(Input::new(search).h(px(34.)))
        .when(!repos.is_empty(), |panel| {
            panel
                .child(
                    div()
                        .px_3()
                        .pt_3()
                        .pb_1()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(say!("shell-tab-list-repositories")),
                )
                .children(repos.into_iter().map(row))
        })
        .when(!others.is_empty(), |panel| {
            panel
                .child(div().h(px(1.)).bg(theme.border))
                .children(others.into_iter().map(row))
        })
}

/// An error as a notification shows it: starting with a capital. A first
/// word that is written with capitals of its own (macOS, iCloud) is a name
/// and keeps its letters.
pub(crate) fn sentence(text: &str) -> String {
    let first_word = text
        .split(|c: char| !c.is_alphanumeric())
        .next()
        .unwrap_or_default();
    if first_word.chars().skip(1).any(char::is_uppercase) {
        return text.to_owned();
    }
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// How much history a clone gets, from what was typed: all of it when
/// nothing was, else a whole number of commits from one up.
pub(crate) fn clone_depth(typed: &str) -> Result<Option<u32>, String> {
    let typed = typed.trim();
    if typed.is_empty() {
        return Ok(None);
    }
    match typed.parse::<u32>() {
        Ok(depth) if depth >= 1 => Ok(Some(depth)),
        _ => Err(say!(
            "shell-clone-depth-not-number",
            typed = shortened(typed, 20)
        )),
    }
}

/// The templates `repo.ignore_templates` listed: name and title of each.
pub(crate) fn template_names(value: &serde_json::Value) -> Vec<(String, String)> {
    value
        .get("templates")
        .and_then(|templates| templates.as_array())
        .map(|templates| {
            templates
                .iter()
                .filter_map(|template| {
                    Some((
                        template.get("name")?.as_str()?.to_owned(),
                        template.get("title")?.as_str()?.to_owned(),
                    ))
                })
                .take(64)
                .collect()
        })
        .unwrap_or_default()
}

/// How many of the things dropped on the window are opened at once.
const MOST_DROPPED: usize = 8;

/// The folders to open for what was dropped on the window: each folder
/// itself, for a file the folder it is in, each once, in the order they
/// were dropped, and not more than a handful.
pub(crate) fn dropped_folders(paths: &[PathBuf], is_dir: impl Fn(&Path) -> bool) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    for path in paths {
        let folder = if is_dir(path) {
            path.clone()
        } else {
            match path.parent() {
                Some(parent) if is_dir(parent) => parent.to_path_buf(),
                _ => continue,
            }
        };
        if !folders.contains(&folder) {
            folders.push(folder);
        }
        if folders.len() >= MOST_DROPPED {
            break;
        }
    }
    folders
}

/// What the field of a parameter says while it is empty.
fn param_placeholder(param: &bitbraid_engine::ParamSpec) -> &'static str {
    use bitbraid_engine::ParamKind;
    match (param.kind, param.required) {
        (ParamKind::List, _) => say!("shell-param-list"),
        (ParamKind::Number, true) => say!("shell-param-number"),
        (ParamKind::Number, false) => say!("shell-param-number-optional"),
        (ParamKind::Structured, _) => say!("shell-param-structured"),
        (ParamKind::Path, true) => say!("shell-param-path"),
        (ParamKind::Path, false) => say!("shell-param-path-optional"),
        (ParamKind::Revision, true) => say!("shell-param-revision"),
        (ParamKind::Revision, false) => say!("shell-param-revision-optional"),
        (_, true) => "",
        (_, false) => say!("shell-param-optional"),
    }
}

/// What the palette lists for what was typed: the sections on show and
/// the entries of each, ranked (`crate::palette`).
fn list_palette(sections: &[PaletteSection], typed: &str) -> Vec<crate::palette::Listed> {
    let keywords: Vec<Vec<Vec<&str>>> = sections
        .iter()
        .map(|section| {
            section
                .entries
                .iter()
                .map(|entry| entry.keywords.iter().map(|word| word.as_ref()).collect())
                .collect()
        })
        .collect();
    let names: Vec<Vec<crate::palette::Names<'_>>> = sections
        .iter()
        .zip(&keywords)
        .map(|(section, keywords)| {
            section
                .entries
                .iter()
                .zip(keywords)
                .map(|(entry, keywords)| crate::palette::Names {
                    label: entry.label.as_ref(),
                    detail: entry.detail.as_deref(),
                    keywords,
                })
                .collect()
        })
        .collect();
    crate::palette::listing(&names, typed)
}

/// The command palette: where it begins (the toolbar's bottom edge), how
/// wide it is, the space over its field's text, a row's height and how
/// many rows are on show.
const PALETTE_TOP: f32 = 82.0;
const PALETTE_WIDTH: f32 = 600.0;
const PALETTE_FIELD_TOP: f32 = 4.0;
const PALETTE_ROW: f32 = 40.0;
const PALETTE_ROWS: f32 = 5.0;
/// What the toolkit puts above and below and beside what a row holds.
const PALETTE_ROW_INSET: (f32, f32) = (6.0, 8.0);

/// A row of the palette: the label, the id of a command beside it, and at
/// the right the key that does the same. The letters that matched what was
/// typed stand out. A row is 40 high with a rule under it: what it holds
/// reaches over the toolkit's inset, to the row's edges.
fn palette_item(entry: &PaletteEntry, found: &crate::palette::Found) -> CommandItem {
    let (label, detail, key) = (entry.label.clone(), entry.detail.clone(), entry.key.clone());
    let found = found.clone();
    let (above, beside) = PALETTE_ROW_INSET;
    CommandItem::new()
        .label(entry.label.clone())
        .child(move |_, cx| {
            let theme = cx.theme();
            h_flex()
                .w_full()
                .h(px(PALETTE_ROW))
                .my(px(-above))
                .mx(px(-beside))
                .px(px(beside))
                .border_b_1()
                .border_color(theme.border)
                .gap_2()
                .items_center()
                .child(div().min_w_0().truncate().child(crate::fuzzy::marked(
                    label.clone(),
                    &found.label,
                    theme.primary,
                )))
                .children(detail.clone().map(|detail| {
                    div()
                        .flex_none()
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(crate::fuzzy::marked(detail, &found.detail, theme.primary))
                }))
                .child(div().flex_1())
                .children(key.clone().map(|key| {
                    div()
                        .flex_none()
                        .px_1p5()
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border)
                        .font_family(theme.mono_font_family.clone())
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(key)
                }))
        })
}

/// One notification for everything that is wrong with the user's themes.
fn themes_notice(problems: &[String]) -> Notice {
    let listed = problems
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    let text = if problems.len() > 4 {
        say!(
            "shell-themes-problems-more",
            problems = listed,
            more = problems.len() - 4
        )
    } else {
        say!("shell-themes-problems", problems = listed)
    };
    Notice::warning(text).autohide(false)
}

/// One notification for everything that is wrong with the user's keys.
fn keys_notice(complaints: &[String]) -> Notice {
    let listed = complaints
        .iter()
        .take(4)
        .cloned()
        .collect::<Vec<_>>()
        .join("; ");
    let text = if complaints.len() > 4 {
        say!(
            "shell-keys-problems-more",
            complaints = listed,
            more = complaints.len() - 4
        )
    } else {
        say!("shell-keys-problems", complaints = listed)
    };
    Notice::warning(text).autohide(false)
}

/// What a command answered, as it is shown: the few words of a short
/// answer, or none when it is long enough to want a view of its own.
pub(crate) fn brief_output(value: &serde_json::Value) -> Option<String> {
    let text = match value {
        serde_json::Value::Null => return Some(String::new()),
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string(other).unwrap_or_default(),
    };
    (text.chars().count() <= 100 && !text.contains('\n')).then_some(text)
}

/// The answer of a command, written out to be read, cut where it becomes
/// too long to be.
pub(crate) fn full_output(value: &serde_json::Value) -> String {
    const LINES: usize = 400;
    let text = match value {
        serde_json::Value::String(text) => text.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_default(),
    };
    let mut out: Vec<String> = text
        .lines()
        .take(LINES)
        .map(|line| shortened(line, 400))
        .collect();
    let all = text.lines().count();
    if all > LINES {
        out.push(say!("shell-output-more-lines", count = all - LINES));
    }
    out.join("\n")
}

/// The command the palette ran last that came back done, by its id: the
/// shell looks at it as it changes ([`Shell::after_command`]).
pub(crate) struct CommandRan {
    id: String,
}

impl Global for CommandRan {}

/// Say that a command run from the palette came back done.
pub(crate) fn command_ran(id: &str, cx: &mut App) {
    cx.set_global(CommandRan { id: id.to_owned() });
}

/// Show what a command run from the palette answered: a short answer in a
/// notification, a long one in a dialog that scrolls.
pub(crate) fn show_command_output(
    label: &str,
    value: &serde_json::Value,
    window: &mut Window,
    cx: &mut App,
) {
    if let Some(brief) = brief_output(value) {
        let said = if brief.is_empty() || brief == "{}" {
            say!("shell-ran", label)
        } else {
            format!("{label}: {brief}")
        };
        crate::notices::push(window, cx, Notice::success(said));
        return;
    }
    let (title, text) = (label.to_owned(), SharedString::from(full_output(value)));
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme();
        let for_copy = text.clone();
        dialog
            .title(title.clone())
            .width(px(760.))
            .child(
                div()
                    .id("command-output")
                    .max_h(px(460.))
                    .overflow_y_scroll()
                    .p_2()
                    .rounded(theme.radius)
                    .bg(theme.muted)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .child(text.clone()),
            )
            .footer(
                h_flex()
                    .gap_2()
                    .justify_end()
                    .child(
                        Button::new("output-copy")
                            .ghost()
                            .label(say!("shell-output-copy"))
                            .on_click(move |_, _, cx| {
                                cx.write_to_clipboard(ClipboardItem::new_string(
                                    for_copy.to_string(),
                                ))
                            }),
                    )
                    .child(
                        Button::new("output-close")
                            .primary()
                            .label(say!("shell-close"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}

/// A label cut to `max` characters, with a mark where it was cut: the
/// toolbar has room for what an action was, not for a whole commit summary.
fn shortened(label: &str, max: usize) -> String {
    if label.chars().count() <= max {
        return label.to_owned();
    }
    let mut cut: String = label.chars().take(max.saturating_sub(1)).collect();
    cut.truncate(cut.trim_end().len());
    cut.push('…');
    cut
}

/// What the toolbar's middle group can do now.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolbarState {
    /// Something the user started runs: the whole group waits.
    pub busy: bool,
    /// The button that started it, which spins.
    pub spin: Option<&'static str>,
    /// There is something to stash.
    pub can_stash: bool,
    /// There is a stash to pop.
    pub can_pop: bool,
}

/// The toolbar's button that starts an engine command, by its id.
fn button_of_command(command: &str) -> Option<&'static str> {
    Some(match command {
        "remote.pull" | "remote.fetch" => "pull",
        "remote.push" | "tag.push" => "push",
        "branch.create" => "new-branch",
        "stash.push" => "stash",
        "stash.pop" | "stash.apply" => "pop",
        "undo.last" => "undo",
        "redo.last" => "redo",
        command if command.starts_with("lfs.") => "lfs",
        _ => return None,
    })
}

/// How many commits the graph holds, as the status bar says it: one is a
/// commit, not "1 commits".
fn commits_said(count: usize) -> String {
    // The figures go as text: a number in a message is a float, whose
    // digits past 2^53 are not the count's.
    say!("shell-commits", count, figures = count.to_string())
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // A tab that was dragged and let go anywhere but on the bar stays
        // where it was.
        if self.tab_drag.is_some() && !cx.has_active_drag() {
            self.tab_drag = None;
        }
        // The zoom is the scale everything is drawn at: the measures of
        // ours (`crate::zoom::px`) and the toolkit's em. Set here, before any
        // view of the window is drawn, it holds from the first frame and
        // follows the preference wherever that is changed.
        let drop_target = cx.theme().drop_target;
        crate::zoom::set(cx.global::<Prefs>().preferences.zoom);
        let rem = px(16.0);
        if window.rem_size() != rem {
            window.set_rem_size(rem);
        }
        // The toasts' place is the toolkit's, in its pixels.
        let zoom = crate::zoom::scale();
        if self.toasts_zoom != zoom {
            self.toasts_zoom = zoom;
            gpui_kit::component::Theme::update(cx, |theme| {
                crate::notices::place(&mut theme.notification, STATUS_BAR_HEIGHT)
            });
        }
        // The window's buttons keep their size; the bar grows with the zoom.
        if crate::chrome::IN_TABS && self.buttons_zoom != zoom {
            self.buttons_zoom = zoom;
            crate::chrome::place_buttons(window, px(TAB_BAR_HEIGHT));
        }
        self.preferences_follow_tab(cx);
        // The history of notifications hangs from the bell over whatever
        // is shown; the other views of the window's own take its place.
        let page_sheet = self.sheet.filter(|sheet| {
            !matches!(
                sheet,
                WindowSheet::Notices | WindowSheet::KeyList | WindowSheet::About
            )
        });
        let body: AnyElement = match (page_sheet, self.tabs.get(self.active).map(|t| &t.kind)) {
            (Some(WindowSheet::Preferences), _) => self.preferences.clone().into_any_element(),
            (Some(WindowSheet::Shortcuts), _) => self.shortcuts.clone().into_any_element(),
            (Some(WindowSheet::Repositories), _) => match &self.repositories {
                Some(page) => page.clone().into_any_element(),
                None => div().into_any_element(),
            },
            (Some(WindowSheet::Inbox), _) => self.inbox.clone().into_any_element(),
            (Some(WindowSheet::Notices | WindowSheet::KeyList | WindowSheet::About), _)
            | (None, None) => div().into_any_element(),
            (None, Some(TabKind::Start(start))) => start.clone().into_any_element(),
            (None, Some(TabKind::Repo(workspace))) => workspace.clone().into_any_element(),
        };
        // The toolbar is a repository's: the start page, which has none,
        // begins right under the tabs.
        let on_start = page_sheet.is_none()
            && matches!(
                self.tabs.get(self.active).map(|tab| &tab.kind),
                Some(TabKind::Start(_))
            );
        // The preferences take everything under the tabs: no toolbar and no
        // status bar while they are open.
        let in_preferences = self.sheet == Some(WindowSheet::Preferences);
        // A question takes the toolbar's place; where there is no toolbar
        // it lies over the top of the page (`crate::ask`).
        let bar = crate::ask::render(window, cx);
        let asking = bar.is_some();
        let without_toolbar = on_start
            || in_preferences
            || matches!(
                self.sheet,
                Some(WindowSheet::Repositories | WindowSheet::Inbox)
            );
        let (toolbar, over_page) = match (bar, without_toolbar) {
            (Some(bar), false) => (Some(bar), None),
            (Some(bar), true) => (None, Some(bar)),
            (None, false) => (
                Some(
                    self.render_toolbar(crate::zoom::unpx(window.viewport_size().width), cx)
                        .into_any_element(),
                ),
                None,
            ),
            (None, true) => (None, None),
        };
        let dimmed = cx.theme().background.opacity(0.55);
        // The list of keys lies over the window, which is dimmed under it:
        // 673 by 517, centred, 83 under the top. A click beside it puts it
        // away, as its cross and Escape do.
        let key_list = (self.sheet == Some(WindowSheet::KeyList)).then(|| {
            div()
                .id("key-list-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(crate::views::key_list::TOP))
                .bg(cx.theme().background.opacity(0.6))
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.sheet == Some(WindowSheet::KeyList) {
                        this.toggle_sheet(WindowSheet::KeyList, window, cx);
                    }
                }))
                .child(
                    div()
                        .id("key-list-frame")
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(self.key_list.clone()),
                )
                .test_support()
        });
        // What the application is lies over the window as the list of keys
        // does, the window dimmed under it, at the same distance from the
        // top. A click beside it puts it away, as its button and Escape do.
        let about = (self.sheet == Some(WindowSheet::About)).then(|| {
            div()
                .id("about-backdrop")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .flex()
                .flex_col()
                .items_center()
                .pt(px(crate::views::key_list::TOP))
                .bg(cx.theme().background.opacity(0.6))
                .occlude()
                .on_click(cx.listener(|this, _, window, cx| {
                    if this.sheet == Some(WindowSheet::About) {
                        this.toggle_sheet(WindowSheet::About, window, cx);
                    }
                }))
                .child(
                    div()
                        .id("about-frame")
                        .on_click(|_, _, cx| cx.stop_propagation())
                        .child(self.about.clone()),
                )
                .test_support()
        });
        // The activity log lies over the whole window, which is dimmed
        // under it: 51 from the left, right and top edge, 70 from the
        // bottom. A click beside it puts it away, as its cross does.
        let activity = self
            .active_workspace()
            .filter(|_| self.sheet.is_none())
            .and_then(|workspace| {
                let log = workspace.read(cx).activity_dialog()?;
                let workspace = workspace.downgrade();
                Some(
                    div()
                        .id("activity-backdrop")
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(cx.theme().background.opacity(0.6))
                        .occlude()
                        .on_click(move |_, _, cx| {
                            let _ = workspace.update(cx, |w, cx| w.close_activity(cx));
                        })
                        .child(
                            div()
                                .id("activity-frame")
                                .absolute()
                                .top(px(crate::views::activity::INSET))
                                .left(px(crate::views::activity::INSET))
                                .right(px(crate::views::activity::INSET))
                                .bottom(px(crate::views::activity::BELOW))
                                // A click inside is the log's.
                                .on_click(|_, _, cx| cx.stop_propagation())
                                .child(log),
                        )
                        // Clicked beside the log in the tests.
                        .test_support(),
                )
            });
        // In a repository that is opened read-only, a key for something
        // that would be refused says so instead of asking for a name first
        // and failing afterwards. These come before the handlers below, and
        // the first one to take an action keeps it.
        // Everything below is shown as it was written: no ligatures.
        let mut root = v_flex()
            .id("shell")
            .font_features(crate::literal::features())
            .track_focus(&self.focus_handle);
        if self.sheet.is_none() && self.active_read_only(cx) {
            for binding in keymap::BINDINGS
                .iter()
                .filter(|binding| crate::read_only::refuses_binding(binding))
            {
                let label = binding.label.text();
                root = root.on_boxed_action(binding.action().as_ref(), move |_, window, cx| {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::info(say!(
                            "shell-read-only-refused",
                            state = crate::read_only::IN_SHORT.text(),
                            action = label.trim_end_matches('…')
                        )),
                    );
                });
            }
        }
        root.on_action(cx.listener(|this, _: &NewTab, window, cx| this.new_tab(window, cx)))
            .on_action(cx.listener(|this, action: &ToolbarTool, window, cx| {
                this.run_tool(action.tool, window, cx)
            }))
            .on_action(cx.listener(|this, action: &PullWith, _, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.choose_pull(&action.mode, cx));
                }
            }))
            .on_action(cx.listener(|this, action: &SwitchBranch, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.checkout(action.name.clone(), window, cx));
                }
            }))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.open_repository(action.path.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, action: &PushCurrent, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    let how = action.how.clone();
                    workspace.update(cx, |w, cx| w.push_how(None, how, window, cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::SkipPushHook, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.toggle_push_hook(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::PushWithLease, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.push_how(None, PushHow::Lease, window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ForcePush, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.push_how(None, PushHow::Force, window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::PushAllTags, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| {
                            w.tag(crate::views::tags::TagRequest::PushAll, window, cx)
                        });
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &FetchAll, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.fetch(window, cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::OpenInFileManager, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace
                            .update(cx, |w, cx| w.open_outside(Outside::FileManager, window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::OpenInEditor, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.open_outside(Outside::Editor, window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::OpenInTerminal, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.open_outside(Outside::Terminal, window, cx));
                    }
                }),
            )
            // The terminal inside the window.
            .on_action(cx.listener(|this, _: &ToggleTerminal, window, cx| {
                this.terminal_key(TerminalKey::Toggle, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewTerminal, window, cx| {
                this.terminal_key(TerminalKey::New, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTerminal, window, cx| {
                this.terminal_key(TerminalKey::Next, window, cx)
            }))
            .on_action(cx.listener(|this, _: &PrevTerminal, window, cx| {
                this.terminal_key(TerminalKey::Previous, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTerminal, window, cx| {
                this.terminal_key(TerminalKey::Close, window, cx)
            }))
            .on_action(cx.listener(|this, action: &SwitchProfile, window, cx| {
                this.switch_profile(action.name.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, action: &LfsMove, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.lfs_transfer(action.what, window, cx));
                }
            }))
            .on_action(cx.listener(|this, action: &ToolbarRemote, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.remote(action.request.clone(), window, cx));
                }
            }))
            .on_action(cx.listener(|this, action: &TabMenu, window, cx| {
                this.on_tab_menu(action, window, cx)
            }))
            .on_action(cx.listener(|this, _: &MoveTabLeft, _, cx| this.nudge_tab(false, cx)))
            .on_action(cx.listener(|this, _: &MoveTabRight, _, cx| this.nudge_tab(true, cx)))
            .on_action(cx.listener(|this, _: &RenameTab, window, cx| {
                let ix = this.active;
                this.prompt_rename_tab(ix, window, cx)
            }))
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                let ix = this.active;
                this.close_tab(ix, window, cx);
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| {
                let n = this.tabs.len().max(1);
                this.activate((this.active + 1) % n, cx);
            }))
            .on_action(cx.listener(|this, _: &PrevTab, _, cx| {
                let n = this.tabs.len().max(1);
                this.activate((this.active + n - 1) % n, cx);
            }))
            .on_action(
                cx.listener(|this, action: &JumpToTab, _, cx| this.activate(action.index, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenRepository, window, cx| {
                this.browse_for_repository(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::CloneRepository, window, cx| {
                    this.prompt_clone(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::InitRepository, window, cx| {
                    this.prompt_init(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ShowTabList, window, cx| {
                    this.set_tab_list(true, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ToggleLeftPanel, window, cx| this.toggle_left(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ToggleRightPanel, window, cx| this.toggle_right(window, cx)),
            )
            .on_action(cx.listener(|_, _: &CycleTheme, _, cx| theme::cycle(cx)))
            // A file by its path, to see its history.
            .on_action(
                cx.listener(|this, _: &crate::keymap::FindFile, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |workspace, cx| workspace.find_file(window, cx));
                    }
                }),
            )
            // Only what the search found, or every commit again.
            .on_action(
                cx.listener(|this, _: &crate::keymap::ToggleOnlyMatches, _, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |workspace, cx| workspace.toggle_only_matches(cx));
                    }
                }),
            )
            // The order of the graph and the date of its rows:
            // preferences, switched from the palette as well. Every
            // workspace sees the other order and asks for its graph again.
            .on_action(
                cx.listener(|_, _: &crate::keymap::ToggleGraphOrder, _, cx| {
                    // The three orders in turn.
                    let prefs = cx.global_mut::<Prefs>();
                    prefs.preferences.graph_order = prefs.preferences.next_order().to_owned();
                    prefs.save_preferences();
                    cx.refresh_windows();
                }),
            )
            .on_action(
                cx.listener(|_, _: &crate::keymap::ToggleGraphDates, _, cx| {
                    let prefs = cx.global_mut::<Prefs>();
                    let commit = !prefs.preferences.commit_dates();
                    prefs.preferences.graph_date =
                        if commit { "commit" } else { "author" }.to_owned();
                    prefs.save_preferences();
                    cx.refresh_windows();
                }),
            )
            .on_action(cx.listener(|this, _: &Refresh, window, cx| this.refresh(window, cx)))
            .on_action(cx.listener(|this, _: &Undo, window, cx| this.undo(window, cx)))
            .on_action(cx.listener(|this, _: &JumpTo, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |workspace, cx| workspace.prompt_jump(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ShowLfs, window, cx| {
                if this.active_workspace().is_some() {
                    this.show_preferences(crate::views::preferences::Section::Lfs, window, cx);
                }
            }))
            // The repository's size, git's tasks and the check.
            .on_action(
                cx.listener(|this, _: &crate::keymap::ShowMaintenance, window, cx| {
                    if this.active_workspace().is_some() {
                        this.show_preferences(
                            crate::views::preferences::Section::Maintenance,
                            window,
                            cx,
                        );
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::CheckOutEverything, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.sparse_disable(window, cx));
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ShowIndexFlags, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |workspace, cx| workspace.toggle_flags(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ApplyPatch, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.prompt_apply_patch(false, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ApplyMailbox, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.prompt_apply_patch(true, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SaveChangesAsPatch, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.save_changes_as_patch(false, window, cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::SaveStagedAsPatch, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.save_changes_as_patch(true, window, cx));
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ToggleRerere, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.toggle_rerere(window, cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::AllowSigner, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.allow_signer(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ChooseSignersFile, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.choose_signers_file(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ContinueOperation, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.in_progress_asked(Some(false), window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::SkipOperation, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.in_progress_asked(Some(true), window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::AbortOperation, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.in_progress_asked(None, window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::FlowSettings, window, cx| {
                    this.flow(crate::views::flow::FlowRequest::SetUp, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::FlowStart, window, cx| {
                    this.flow(crate::views::flow::FlowRequest::StartAny, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::FlowFinish, window, cx| {
                    this.flow(crate::views::flow::FlowRequest::FinishCurrent, window, cx);
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ShowRerereRecords, _, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.toggle_rerere_records(cx));
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &crate::keymap::ToggleWrap, _, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.toggle_diff_wrap(cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::CompareWithWorktree, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.compare_with_worktree(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::AddCommitTrailer, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.ask_trailer(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::IgnorePattern, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.prompt_ignore(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::UntrackIgnored, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.untrack_ignored(window, cx));
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ShowProfiles, window, cx| {
                this.show_preferences(crate::views::preferences::Section::Profiles, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &keymap::ShowInbox, window, cx| this.show_inbox(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &keymap::ShowRepositories, window, cx| {
                    this.show_repositories(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &keymap::ShowIntegrations, window, cx| {
                    this.show_preferences(
                        crate::views::preferences::Section::Integrations,
                        window,
                        cx,
                    )
                }),
            )
            // What is running is stopped and the activity log written out
            // by what the application does whenever it is quit.
            .on_action(cx.listener(|_, _: &Quit, _, cx| cx.quit()))
            .on_action(cx.listener(|this, _: &FilterReferences, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.focus_references(true, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &FocusReferences, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.focus_references(false, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &RegisterLinks, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |workspace, cx| workspace.register_links(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ShowSigning, window, cx| {
                this.show_preferences(crate::views::preferences::Section::Signing, window, cx);
                // The keys that could sign take the arrows.
                if let Some(workspace) = this.active_workspace().cloned() {
                    let view = workspace.read(cx).signing_view();
                    view.update(cx, |view, cx| view.reload(cx));
                    view.read(cx).focus_handle(cx).focus(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &ShowActivity, _, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |workspace, cx| workspace.toggle_activity(cx));
                }
            }))
            .on_action(cx.listener(|this, _: &ShowUndoHistory, _, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |workspace, cx| workspace.toggle_journal(cx));
                }
            }))
            .on_action(
                cx.listener(|this, _: &keymap::StartPullRequest, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |workspace, cx| {
                            workspace.start_pull(String::new(), None, window, cx)
                        });
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &Redo, window, cx| this.redo(window, cx)))
            .on_action(
                cx.listener(|this, _: &TogglePalette, window, cx| this.open_palette(window, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowPreferences, window, cx| {
                this.toggle_sheet(WindowSheet::Preferences, window, cx)
            }))
            .on_action(cx.listener(|this, _: &FocusGraph, window, cx| {
                // Escape puts a view of the window's own away, and the
                // activity log; with none shown it is the graph's.
                let log = this
                    .active_workspace()
                    .cloned()
                    .filter(|w| w.read(cx).activity_dialog().is_some());
                if let Some(workspace) = log.filter(|_| this.sheet.is_none()) {
                    workspace.update(cx, |w, cx| w.close_activity(cx));
                    this.take_focus(window, cx);
                } else if this.sheet.is_some() {
                    this.sheet = None;
                    this.take_focus(window, cx);
                    cx.notify();
                } else if this.tab_list_open {
                    this.set_tab_list(false, window, cx);
                    this.take_focus(window, cx);
                } else if let Some(workspace) = this.active_workspace().cloned()
                    && workspace.update(cx, |w, cx| w.escape_closes_list(window, cx))
                {
                    // A list in the middle, with the keys outside it.
                } else {
                    cx.propagate();
                }
            }))
            // With the focus outside the graph, the search is still the
            // graph's: the graph takes this itself when it has the focus.
            .on_action(
                cx.listener(|this, _: &crate::keymap::FocusSearch, window, cx| {
                    if let Some(workspace) = this.active_workspace().cloned() {
                        workspace.update(cx, |w, cx| w.focus_search(window, cx));
                    }
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ShowNotifications, window, cx| {
                    this.toggle_sheet(WindowSheet::Notices, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ShowShortcuts, window, cx| {
                this.toggle_sheet(WindowSheet::KeyList, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &crate::keymap::ShowAbout, window, cx| {
                    this.about.update(cx, |about, cx| about.shown(cx));
                    this.toggle_sheet(WindowSheet::About, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &crate::keymap::ChangeKeys, window, cx| {
                    if this.sheet != Some(WindowSheet::Shortcuts) {
                        this.toggle_sheet(WindowSheet::Shortcuts, window, cx)
                    }
                }),
            )
            .on_action(cx.listener(|this, _: &ReloadKeys, window, cx| this.reload_keys(window, cx)))
            .on_action(
                cx.listener(|this, _: &crate::keymap::ResetKeys, window, cx| {
                    // Asked where the keys are listed, so that what comes
                    // of it is seen.
                    if this.sheet != Some(WindowSheet::Shortcuts) {
                        this.toggle_sheet(WindowSheet::Shortcuts, window, cx);
                    }
                    this.shortcuts
                        .update(cx, |view, cx| view.confirm_reset_all(window, cx));
                }),
            )
            .on_action(
                cx.listener(|_, _: &crate::keymap::ToggleFullscreen, window, _| {
                    window.toggle_fullscreen()
                }),
            )
            .on_action(cx.listener(|this, _: &Fetch, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.fetch(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Pull, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.pull(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &Push, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.push(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &NewBranch, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.prompt_new_branch(None, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &StashChanges, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.stash(StashRequest::Push, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &PopStash, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.stash(StashRequest::Pop(None), window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CommitStaged, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.commit_now(false, window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &CommitAll, window, cx| {
                if let Some(workspace) = this.active_workspace().cloned() {
                    workspace.update(cx, |w, cx| w.commit_now(true, window, cx));
                }
            }))
            .on_action(cx.listener(|this, action: &ZoomTo, window, cx| {
                this.set_zoom(action.percent as f32 / 100.0, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomIn, window, cx| {
                let zoom = cx.global::<Prefs>().preferences.zoom + 0.1;
                this.set_zoom(zoom, window, cx);
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, window, cx| {
                let zoom = cx.global::<Prefs>().preferences.zoom - 0.1;
                this.set_zoom(zoom, window, cx);
            }))
            .on_action(
                cx.listener(|this, _: &ZoomReset, window, cx| this.set_zoom(1.0, window, cx)),
            )
            // A folder dropped on the window is opened.
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_target))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                for folder in dropped_folders(paths.paths(), |path| path.is_dir()) {
                    this.open_repository(folder, window, cx);
                }
            }))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                // While a question stands the tabs are dimmed, and stay
                // where they are.
                div()
                    .relative()
                    .w_full()
                    .flex_none()
                    .child(self.render_tab_bar(
                        crate::zoom::unpx(window.viewport_size().width),
                        crate::chrome::buttons_room(window),
                        cx,
                    ))
                    .when(asking, |tabs| {
                        tabs.child(
                            div()
                                .id("tabs-dimmed")
                                .absolute()
                                .top_0()
                                .left_0()
                                .size_full()
                                .bg(dimmed)
                                .occlude(),
                        )
                    }),
            )
            .children(toolbar)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(body)
                    .children(
                        over_page.map(|bar| div().absolute().top_0().left_0().w_full().child(bar)),
                    ),
            )
            .when(!in_preferences, |root| {
                root.child(self.render_status_bar(cx))
            })
            .children(activity)
            .children(key_list)
            .children(about)
    }
}

/// The rows of the clone form: where it goes, where it comes from, which
/// branch and how much of its history; its switches: shallow (which shows
/// the branch and the depth), its submodules as well, and only the files
/// at the top checked out.
const CLONE_INTO: usize = 0;
const CLONE_FROM: usize = 1;
const CLONE_BRANCH: usize = 2;
const CLONE_DEPTH: usize = 3;
const CLONE_SHALLOW: usize = 0;
const CLONE_SUBMODULES: usize = 1;
const CLONE_SPARSE: usize = 2;
/// What a partial clone can leave on the remote: the engine's
/// word, and what the list says.
const CLONE_FILTERS: [(&str, bitbraid_strings::Message); 2] = [
    ("blob:none", message!("shell-clone-filter-blobs")),
    ("tree:0", message!("shell-clone-filter-trees")),
];
/// The rows of the form for a new repository: its name, the folder it is
/// made in, the full path the two make, its first branch.
const INIT_NAME: usize = 0;
const INIT_IN: usize = 1;
const INIT_BRANCH: usize = 3;

/// Where a new repository of this name in this folder goes: none while
/// either is missing, the folder is no full path or the name is no
/// folder's name.
fn init_path(folder: &str, name: &str) -> Option<PathBuf> {
    let (folder, name) = (folder.trim(), name.trim());
    if folder.is_empty() || name.is_empty() || name == "." || name == ".." {
        return None;
    }
    if name.contains(['/', '\\']) {
        return None;
    }
    let folder = expand_home(folder);
    folder.is_absolute().then(|| folder.join(name))
}

/// Why a new repository of this name in this folder cannot be made, if
/// it cannot: said in the form while it is typed.
fn init_refusal(folder: &str, name: &str) -> Option<String> {
    let (folder, name) = (folder.trim(), name.trim());
    if !folder.is_empty() && !expand_home(folder).is_absolute() {
        return Some(say!("shell-init-in-not-full").to_owned());
    }
    if name.contains(['/', '\\']) || name == "." || name == ".." {
        return Some(say!("shell-init-name-not-folder").to_owned());
    }
    None
}

/// Why what the clone form holds cannot be cloned, if it cannot: said in
/// the form while it is typed, and what keeps it open.
fn clone_refusal(folder: &str, depth: &str) -> Option<String> {
    if !folder.is_empty() && !expand_home(folder).is_absolute() {
        return Some(say!("shell-clone-into-not-full").to_owned());
    }
    clone_depth(depth).err()
}

impl FormHost for Shell {
    fn forms(&self) -> &Forms<Self> {
        &self.forms
    }

    fn forms_mut(&mut self) -> &mut Forms<Self> {
        &mut self.forms
    }
}

/// Display name for a repository path.
pub fn repo_title(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::shortened;

    #[test]
    fn a_sentence_starts_with_a_capital_unless_its_first_word_is_a_name() {
        use super::sentence;
        assert_eq!(sentence("the remote hung up"), "The remote hung up");
        assert_eq!(sentence("git failed"), "Git failed");
        assert_eq!(sentence("éclair"), "Éclair");
        assert_eq!(sentence(""), "");
        // The updater's word on a copy macOS translocated: not "MacOS".
        assert_eq!(
            sentence("macOS runs this copy from a place of its own"),
            "macOS runs this copy from a place of its own"
        );
        assert_eq!(sentence("iCloud: no room"), "iCloud: no room");
        assert_eq!(sentence("SHA-256 differs"), "SHA-256 differs");
    }

    #[test]
    fn as_many_tabs_as_fit_are_drawn_with_the_shown_one_among_them() {
        use super::tabs_drawn;
        // They fit: all of them.
        assert_eq!(tabs_drawn(4, 10, 3, 0), 0..4);
        // Ten of thirteen fit: from the first while the shown one is
        // among them, else ending at the shown one.
        assert_eq!(tabs_drawn(13, 10, 2, 0), 0..10);
        assert_eq!(tabs_drawn(13, 10, 12, 0), 3..13);
        assert_eq!(tabs_drawn(13, 10, 10, 0), 1..11);
        // From where the last frame started while the shown one is among
        // them: showing a tab that is drawn moves none.
        assert_eq!(tabs_drawn(13, 10, 5, 3), 3..13);
        assert_eq!(tabs_drawn(13, 10, 3, 3), 3..13);
        // Moved as little as needed for the shown one to be among them.
        assert_eq!(tabs_drawn(13, 10, 2, 3), 2..12);
        assert_eq!(tabs_drawn(13, 10, 12, 1), 3..13);
        // A start past what can be: as far on as the tabs go.
        assert_eq!(tabs_drawn(13, 10, 12, 50), 3..13);
        assert_eq!(tabs_drawn(13, 12, 0, usize::MAX), 0..12);
        // Room for none still draws one; a shown one past the end, the last.
        assert_eq!(tabs_drawn(5, 0, 4, 0), 4..5);
        assert_eq!(tabs_drawn(5, 2, 99, 0), 3..5);
        assert_eq!(tabs_drawn(0, 3, 0, 0), 0..0);
        assert_eq!(
            tabs_drawn(usize::MAX, 2, usize::MAX, usize::MAX),
            usize::MAX - 2..usize::MAX
        );
    }

    #[test]
    fn the_shown_repository_is_counted_among_the_kept_ones() {
        use super::kept_active;
        // Two repositories, the second shown.
        assert_eq!(kept_active(&[true, true], 1), 1);
        // A start page before it is not kept and does not count.
        assert_eq!(kept_active(&[false, true, true], 2), 1);
        // A start page shown counts as the repository before it, or the
        // first when there is none before it.
        assert_eq!(kept_active(&[true, false, true], 1), 0);
        assert_eq!(kept_active(&[false, true], 0), 0);
        // Out of range, or nothing: the last, or nought.
        assert_eq!(kept_active(&[true, true], usize::MAX), 1);
        assert_eq!(kept_active(&[], 3), 0);
    }

    #[test]
    fn the_shown_tab_stays_shown_when_tabs_move() {
        use super::active_after_move;
        // Tabs a b c d e, c shown (index 2).
        assert_eq!(active_after_move(2, 2, 4), 4, "the shown tab itself moved");
        assert_eq!(
            active_after_move(2, 0, 4),
            1,
            "one from its left went right of it"
        );
        assert_eq!(
            active_after_move(2, 4, 0),
            3,
            "one from its right went left of it"
        );
        assert_eq!(active_after_move(2, 0, 1), 2, "a move to its left only");
        assert_eq!(active_after_move(2, 3, 4), 2, "a move to its right only");
        assert_eq!(active_after_move(2, 0, 2), 1, "one put where it was");
        assert_eq!(active_after_move(2, 4, 2), 3);
        assert_eq!(active_after_move(0, 0, 0), 0);
        // Against moving a real list about.
        for from in 0..5usize {
            for to in 0..5usize {
                for active in 0..5usize {
                    let mut tabs = vec!['a', 'b', 'c', 'd', 'e'];
                    let shown = tabs[active];
                    let tab = tabs.remove(from);
                    tabs.insert(to, tab);
                    assert_eq!(
                        tabs[active_after_move(active, from, to)],
                        shown,
                        "{from} to {to}, {active} shown"
                    );
                }
            }
        }
    }

    #[test]
    fn a_repository_takes_over_the_tab_it_is_told_to_or_a_start_tab() {
        use super::tab_to_replace;
        // A worktree opened in place of the second of three tabs, while
        // the third is shown.
        assert_eq!(tab_to_replace(Some(1), 3, 2, false), Some(1));
        // No tab named: a tab of its own, unless a start tab is shown.
        assert_eq!(tab_to_replace(None, 3, 2, false), None);
        assert_eq!(tab_to_replace(None, 3, 2, true), Some(2));
        // A tab that is gone by now is not taken over.
        assert_eq!(tab_to_replace(Some(7), 3, 0, false), None);
        assert_eq!(tab_to_replace(Some(7), 3, 0, true), Some(0));
        assert_eq!(tab_to_replace(None, 0, 0, true), None);
    }

    #[test]
    fn a_tab_is_called_what_it_was_named_or_as_its_folder() {
        use super::tab_title;
        use std::{collections::HashMap, path::Path};
        let mut aliases = HashMap::new();
        assert_eq!(tab_title(Path::new("/work/project"), &aliases), "project");
        aliases.insert("/work/project".to_owned(), "  Client work ".to_owned());
        aliases.insert("/work/other".to_owned(), "   ".to_owned());
        assert_eq!(
            tab_title(Path::new("/work/project"), &aliases),
            "Client work"
        );
        assert_eq!(tab_title(Path::new("/work/other"), &aliases), "other");
        assert_eq!(tab_title(Path::new("/"), &aliases), "/");
    }

    #[test]
    fn the_depth_of_a_clone_is_a_number_from_one_up_or_nothing() {
        use super::clone_depth;
        assert_eq!(clone_depth(""), Ok(None));
        assert_eq!(clone_depth("   "), Ok(None));
        assert_eq!(clone_depth(" 1 "), Ok(Some(1)));
        assert_eq!(clone_depth("250"), Ok(Some(250)));
        for bad in ["0", "-1", "1.5", "all", "99999999999999999999", "1e3"] {
            assert!(clone_depth(bad).is_err(), "{bad}");
        }
        assert!(clone_depth(&"9".repeat(500)).unwrap_err().len() < 120);
    }

    #[test]
    fn the_templates_the_engine_lists_are_read() {
        use super::template_names;
        let listed = serde_json::json!({ "templates": [
            { "name": "rust", "title": "Rust" },
            { "name": "node", "title": "Node.js" },
            { "name": "untitled" },
            7,
        ], "text": null });
        assert_eq!(
            template_names(&listed),
            [
                ("rust".to_owned(), "Rust".to_owned()),
                ("node".to_owned(), "Node.js".to_owned())
            ]
        );
        assert!(template_names(&serde_json::json!([])).is_empty());
    }

    #[test]
    fn what_is_dropped_on_the_window_is_opened_as_folders() {
        use super::{MOST_DROPPED, dropped_folders};
        use std::path::{Path, PathBuf};
        let folders = ["/work/project", "/work/other", "/work"];
        let is_dir = |path: &Path| folders.iter().any(|folder| Path::new(folder) == path);
        let dropped = |paths: &[&str]| {
            dropped_folders(&paths.iter().map(PathBuf::from).collect::<Vec<_>>(), is_dir)
        };
        assert_eq!(
            dropped(&["/work/project"]),
            [PathBuf::from("/work/project")]
        );
        // A file stands for the folder it is in; nothing is opened twice.
        assert_eq!(
            dropped(&["/work/project/README.md", "/work/project", "/work/other"]),
            [PathBuf::from("/work/project"), PathBuf::from("/work/other")]
        );
        // What is nowhere is left out, and nothing at all is nothing.
        assert!(dropped(&["/gone/away/file.txt", "/gone", ""]).is_empty());
        assert!(dropped(&[]).is_empty());
        // A hundred folders dropped at once do not open a hundred tabs.
        let many: Vec<PathBuf> = (0..100)
            .map(|n| PathBuf::from(format!("/many/{n}")))
            .collect();
        assert_eq!(dropped_folders(&many, |_| true).len(), MOST_DROPPED);
    }

    #[test]
    fn one_commit_is_a_commit_and_any_other_number_is_commits() {
        use super::commits_said;
        assert_eq!(commits_said(1), "1 commit");
        assert_eq!(commits_said(0), "0 commits");
        assert_eq!(commits_said(2), "2 commits");
        assert_eq!(commits_said(40), "40 commits");
        assert_eq!(commits_said(usize::MAX), format!("{} commits", usize::MAX));
    }

    #[test]
    fn a_command_spins_the_button_that_starts_it() {
        use super::button_of_command;
        assert_eq!(button_of_command("remote.fetch"), Some("pull"));
        assert_eq!(button_of_command("remote.push"), Some("push"));
        assert_eq!(button_of_command("stash.push"), Some("stash"));
        assert_eq!(button_of_command("stash.pop"), Some("pop"));
        assert_eq!(button_of_command("lfs.pull"), Some("lfs"));
        assert_eq!(button_of_command("branch.rename"), None);
        assert_eq!(button_of_command(""), None);
    }

    #[test]
    fn long_labels_are_cut_at_a_character_and_marked() {
        assert_eq!(shortened("Stash", 32), "Stash");
        assert_eq!(shortened("", 32), "");
        let long = "Commit 5d171e0: one commit for everything at once";
        let cut = shortened(long, 32);
        assert_eq!(cut, "Commit 5d171e0: one commit for…");
        assert_eq!(cut.chars().count(), 31);
        // Exactly as long as allowed stays whole; wide characters count as one.
        assert_eq!(shortened(&"é".repeat(32), 32), "é".repeat(32));
        assert_eq!(shortened(&"é".repeat(33), 32).chars().count(), 32);
        assert_eq!(shortened("abc", 0), "…");
    }
}
