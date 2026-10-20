//! The terminal panel of one repository or worktree: what the bottom of
//! the graph's column shows while the terminal is on show.
//!
//! A rule round it; a heading 28 high with a terminal icon 10 in from the
//! left and the word Terminal beside it, and at the right three buttons 25
//! apart, the last one's middle 18 in from the right edge: minimise (a
//! chevron, up while minimised), end the session (a bin) and hide (a
//! cross); a rule 2 high; then the text, 12 in from the left border and 12
//! under the rule. A right click in the text opens a menu of what can be
//! done to it and to the panel.
//!
//! When a worktree has more than one terminal, they are tabs in the
//! heading after its word, compact, each named by what its shell calls
//! itself or by its number, with a cross while the pointer is on it; the
//! middle button closes one. A plus after them (after the word while there
//! is one) starts another. Where the panel stands, how high it is and
//! whether it is minimised is the workspace's to lay out
//! (`workspace/terminals.rs`).
//!
//! Sources: none (original design).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use bitbraid_engine::completion::Words;
use bitbraid_strings::Message;

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::look::Look;
use super::text::{TerminalText, TerminalTextEvent};
use crate::prefs::Prefs;
use crate::theme::TerminalColors;
use crate::zoom::px;

/// The heading's height.
pub const HEADER_HEIGHT: f32 = 28.0;
/// The rule between the heading and the text.
pub const RULE_HEIGHT: f32 = 2.0;
/// How high the panel is at first, and the least it can be dragged to:
/// the same at every window size.
pub const LEAST_HEIGHT: f32 = 275.0;
/// What the graph keeps above the panel at the most it can be dragged
/// to: its heading and about two rows.
pub const GRAPH_KEPT: f32 = 70.0;
/// A minimised panel: its heading between two rules.
pub const MINIMISED_HEIGHT: f32 = HEADER_HEIGHT + 2.0;
/// From the panel's left edge to where the heading's icon is drawn: 10
/// to the sign itself, which stands inside its box.
const ICON_INSET: f32 = 9.0;
/// How large the heading's icon is.
const ICON_SIZE: f32 = 16.0;
/// The heading's buttons: how far apart their middles are, and how far
/// the last one's middle is from the right edge.
const BUTTON_PITCH: f32 = 25.0;
const LAST_BUTTON_INSET: f32 = 18.0;
/// How wide a button of the heading is.
const BUTTON_WIDTH: f32 = 20.0;
/// The longest a tab of the heading grows.
const TAB_MOST: f32 = 180.0;
/// What a tab is called at the most, in characters.
const TITLE_MOST: usize = 60;

/// What the text of a terminal is.
pub enum Body {
    /// Waiting to be told what to start.
    Starting,
    /// It could not be started, and why.
    Failed(String),
    /// A shell runs, shown by its text (which pads itself 12 from the
    /// left and the top).
    Text(Entity<TerminalText>),
}

/// A shell that was started: its text, and what carries the session's
/// news to it (`sessions::open_text`).
pub struct Running {
    pub text: Entity<TerminalText>,
    pub forward: Task<()>,
}

/// One terminal of a worktree.
pub struct Terminal {
    /// Unique in the window: what its tab and its text are found by.
    id: u64,
    /// Its number in its worktree, from one: what it is called while its
    /// shell gives it no name.
    number: usize,
    /// What its shell calls it, cleaned and cut.
    title: Option<String>,
    /// The folder it was started in.
    folder: PathBuf,
    body: Body,
    /// What carries the session's news to the text, and what the text
    /// says, heard here.
    _forward: Option<Task<()>>,
    _heard: Option<Subscription>,
}

impl Terminal {
    /// What its tab says.
    pub fn name(&self) -> String {
        match &self.title {
            Some(title) => title.clone(),
            None => say!("terminal-tab-name", number = self.number),
        }
    }

    /// What it is known by in the window.
    pub fn id(&self) -> u64 {
        self.id
    }

    /// The folder it was started in.
    pub fn folder(&self) -> &Path {
        &self.folder
    }

    /// Whether its shell runs.
    pub fn running(&self, cx: &App) -> bool {
        match &self.body {
            Body::Text(text) => !text.read(cx).exited(),
            Body::Starting | Body::Failed(_) => false,
        }
    }

    /// Its text, once its shell runs.
    pub fn text(&self) -> Option<&Entity<TerminalText>> {
        match &self.body {
            Body::Text(text) => Some(text),
            Body::Starting | Body::Failed(_) => None,
        }
    }

    /// Why it could not be started, when it could not.
    pub fn failed(&self) -> Option<&str> {
        match &self.body {
            Body::Failed(why) => Some(why),
            Body::Starting | Body::Text(_) => None,
        }
    }

    /// The handle the keys go to while it has them.
    fn focus(&self, cx: &App) -> Option<FocusHandle> {
        self.text().map(|text| text.read(cx).focus_handle(cx))
    }
}

/// What the panel tells the window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PanelEvent {
    /// What the workspace draws of it changed: shown, minimised, the list.
    Changed,
    /// Another terminal is wanted, in this folder (none: the worktree's).
    Wanted(Option<PathBuf>),
    /// A command ended in one of its shells: what it did may show in the
    /// graph.
    CommandFinished,
    /// It went away while it had the keys: they go back to the graph.
    Left,
}

/// A command of the panel's: the menu's, the heading's and the keys'.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelDo {
    Copy,
    Paste,
    SelectAll,
    DeleteLine,
    Clear,
    Minimise,
    End,
    Hide,
}

/// The menu that is open over the text, and where.
struct OpenMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _closed: Subscription,
}

/// The terminals of one repository or worktree.
pub struct TerminalPanel {
    /// The repository's or worktree's folder.
    path: PathBuf,
    terminals: Vec<Terminal>,
    /// Which of them is on show.
    active: usize,
    /// The highest number given so far.
    numbered: usize,
    menu: Option<OpenMenu>,
    /// What the terminals were last drawn with: when the preferences or
    /// the theme change it, it is handed to each again.
    applied: Option<Look>,
    /// The repository's names that git commands typed in its terminals
    /// are completed with, handed to each.
    words: Option<Arc<Words>>,
}

impl EventEmitter<PanelEvent> for TerminalPanel {}

/// Ids that are unique in the window, for the terminals' elements.
fn next_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// A title as a tab shows it: without control characters, cut to what
/// fits, none when nothing is left.
pub fn clean_title(title: &str) -> Option<String> {
    let cleaned: String = title
        .chars()
        .filter(|c| !c.is_control())
        .take(TITLE_MOST.saturating_add(1))
        .collect();
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.chars().count() > TITLE_MOST {
        let mut cut: String = trimmed.chars().take(TITLE_MOST.saturating_sub(1)).collect();
        cut.push('…');
        Some(cut)
    } else {
        Some(trimmed.to_owned())
    }
}

impl TerminalPanel {
    /// The panel of the repository or worktree at `path`, with nothing in
    /// it yet.
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            terminals: Vec::new(),
            active: 0,
            numbered: 0,
            menu: None,
            applied: None,
            words: None,
        }
    }

    /// The repository's names to complete git commands with, for every
    /// terminal of the panel and those started later.
    pub fn set_words(&mut self, words: Arc<Words>, cx: &mut Context<Self>) {
        for text in self.terminals.iter().filter_map(|t| t.text().cloned()) {
            let given = words.clone();
            text.update(cx, |text, _| text.set_completion_words(given));
        }
        self.words = Some(words);
    }

    /// The names its terminals complete with, once they came.
    pub fn words(&self) -> Option<&Arc<Words>> {
        self.words.as_ref()
    }

    /// What the terminals were last drawn with.
    pub fn look(&self) -> Option<&Look> {
        self.applied.as_ref()
    }

    /// Hand the look of the preferences and the theme to every terminal,
    /// when it is not what they have.
    fn apply_look(&mut self, cx: &mut Context<Self>) -> Look {
        let look = super::look::current(cx);
        if self.applied.as_ref() != Some(&look) {
            self.applied = Some(look.clone());
            let texts: Vec<Entity<TerminalText>> = self
                .terminals
                .iter()
                .filter_map(|t| t.text().cloned())
                .collect();
            for text in texts {
                let given = super::look::terminal_look(&look);
                text.update(cx, |text, cx| text.set_look(given, cx));
            }
        }
        look
    }

    /// The folder it is of.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Its terminals, in the order of their tabs.
    pub fn terminals(&self) -> &[Terminal] {
        &self.terminals
    }

    /// Which terminal is on show.
    pub fn active(&self) -> usize {
        self.active
    }

    /// Whether it has no terminal.
    pub fn is_empty(&self) -> bool {
        self.terminals.is_empty()
    }

    /// Whether the menu over the text is open.
    pub fn menu_open(&self) -> bool {
        self.menu.is_some()
    }

    /// Whether the panel is to be on show: the repository's state says so
    /// and it has a terminal.
    pub fn shown(&self, cx: &App) -> bool {
        !self.terminals.is_empty() && cx.global::<Prefs>().repo_state(&self.path).terminal_shown
    }

    /// Whether only its heading shows.
    pub fn minimised(&self, cx: &App) -> bool {
        cx.global::<Prefs>()
            .repo_state(&self.path)
            .terminal_minimised
    }

    fn remember(&self, cx: &mut App, change: impl FnOnce(&mut crate::prefs::RepoState)) {
        cx.global_mut::<Prefs>()
            .update_repo_state(&self.path, change);
    }

    /// Show or hide the panel; the terminals stay either way.
    pub fn set_shown(&mut self, shown: bool, cx: &mut Context<Self>) {
        self.remember(cx, |state| state.terminal_shown = shown);
        if !shown {
            self.menu = None;
        }
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// Show only the heading, or all of it again.
    pub fn set_minimised(&mut self, minimised: bool, cx: &mut Context<Self>) {
        self.remember(cx, |state| state.terminal_minimised = minimised);
        self.menu = None;
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// Add a terminal that is about to be started in `folder` and show it.
    /// Its id, for [`Self::started`].
    pub fn add(&mut self, folder: PathBuf, cx: &mut Context<Self>) -> u64 {
        let id = next_id();
        self.numbered = self.numbered.saturating_add(1);
        self.terminals.push(Terminal {
            id,
            number: self.numbered,
            title: None,
            folder,
            body: Body::Starting,
            _forward: None,
            _heard: None,
        });
        self.active = self.terminals.len().saturating_sub(1);
        // A terminal that is asked for is seen: the panel comes up whole.
        self.remember(cx, |state| {
            state.terminal_shown = true;
            state.terminal_minimised = false;
        });
        cx.emit(PanelEvent::Changed);
        cx.notify();
        id
    }

    /// The terminal of this id could not be started, and why.
    pub fn failed(&mut self, id: u64, why: String, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.iter_mut().find(|t| t.id == id) {
            terminal.body = Body::Failed(why);
        }
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// The shell of the terminal of this id runs: show its text, drawn as
    /// the others are, and hear what it says. One that was ended while it
    /// was being started is ended at once.
    pub fn attach(
        &mut self,
        id: u64,
        running: Running,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Running { text, forward } = running;
        if let Some(look) = self.applied.clone() {
            let given = super::look::terminal_look(&look);
            text.update(cx, |text, cx| text.set_look(given, cx));
        }
        if let Some(words) = self.words.clone() {
            text.update(cx, |text, _| text.set_completion_words(words));
        }
        let heard = cx.subscribe_in(
            &text,
            window,
            move |this, _, event: &TerminalTextEvent, window, cx| match event {
                TerminalTextEvent::Title(title) => this.retitle(id, title, cx),
                // A shell that ends takes its tab with it, and the last
                // one the panel.
                TerminalTextEvent::Exited(_) => this.close(id, window, cx),
                TerminalTextEvent::CommandFinished => this.command_finished(cx),
                TerminalTextEvent::ContextMenu(position) => this.open_menu(*position, window, cx),
            },
        );
        match self.terminals.iter_mut().find(|t| t.id == id) {
            Some(terminal) => {
                terminal.body = Body::Text(text);
                terminal._forward = Some(forward);
                terminal._heard = Some(heard);
            }
            None => text.read(cx).session().kill(),
        }
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// Its shell gave the terminal of this id a name.
    pub fn retitle(&mut self, id: u64, title: &str, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminals.iter_mut().find(|t| t.id == id) {
            terminal.title = clean_title(title);
        }
        cx.notify();
    }

    /// A command ended in a shell of this panel.
    pub fn command_finished(&mut self, cx: &mut Context<Self>) {
        cx.emit(PanelEvent::CommandFinished);
    }

    /// Show the terminal at `index`.
    pub fn select(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if index < self.terminals.len() {
            self.active = index;
            self.menu = None;
            self.focus_active(window, cx);
            cx.notify();
        }
    }

    /// Show the next terminal, or the previous one, going round.
    pub fn step(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let count = self.terminals.len();
        if count < 2 {
            return;
        }
        let next = if forward {
            self.active.saturating_add(1) % count
        } else {
            self.active
                .checked_sub(1)
                .unwrap_or(count.saturating_sub(1))
        };
        self.select(next, window, cx);
    }

    /// Give the keys to the terminal on show.
    pub fn focus_active(&self, window: &mut Window, cx: &mut App) {
        if let Some(handle) = self.terminals.get(self.active).and_then(|t| t.focus(cx)) {
            handle.focus(window, cx);
        }
    }

    /// End the terminal of this id: its shell is ended and its tab goes.
    /// The last one takes the panel with it.
    pub fn close(&mut self, id: u64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(at) = self.terminals.iter().position(|t| t.id == id) else {
            return;
        };
        let had_keys = self.active_has_keys(window, cx);
        let ended = self.terminals.remove(at);
        end(ended, cx);
        if at < self.active || self.active >= self.terminals.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.menu = None;
        if self.terminals.is_empty() {
            // The panel closes with its last shell, and the button
            // starts a new one.
            self.numbered = 0;
            self.remember(cx, |state| state.terminal_shown = false);
            if had_keys {
                cx.emit(PanelEvent::Left);
            }
        } else if had_keys {
            self.focus_active(window, cx);
        }
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// End every terminal of the panel, as the window does when nothing
    /// shows its worktree any more.
    pub fn end_all(&mut self, cx: &mut Context<Self>) {
        for terminal in std::mem::take(&mut self.terminals) {
            end(terminal, cx);
        }
        self.active = 0;
        self.numbered = 0;
        self.menu = None;
        cx.emit(PanelEvent::Changed);
        cx.notify();
    }

    /// Whether the terminal on show has the keys.
    pub fn active_has_keys(&self, window: &Window, cx: &App) -> bool {
        self.terminals
            .get(self.active)
            .and_then(|t| t.focus(cx))
            .is_some_and(|handle| handle.contains_focused(window, cx))
    }

    /// Do one of the panel's commands to the terminal on show.
    pub fn run(&mut self, what: PanelDo, window: &mut Window, cx: &mut Context<Self>) {
        self.menu = None;
        let text = self
            .terminals
            .get(self.active)
            .and_then(|t| t.text().cloned());
        match what {
            // The text's own.
            PanelDo::Copy
            | PanelDo::Paste
            | PanelDo::SelectAll
            | PanelDo::DeleteLine
            | PanelDo::Clear => {
                if let Some(text) = text {
                    text.update(cx, |text, cx| match what {
                        PanelDo::Copy => text.copy(cx),
                        PanelDo::Paste => text.paste(window, cx),
                        PanelDo::SelectAll => text.select_all(cx),
                        PanelDo::DeleteLine => text.delete_line(cx),
                        _ => text.clear(cx),
                    });
                }
                self.focus_active(window, cx);
            }
            PanelDo::Minimise => {
                let minimised = self.minimised(cx);
                if !minimised && self.active_has_keys(window, cx) {
                    cx.emit(PanelEvent::Left);
                }
                self.set_minimised(!minimised, cx);
                if minimised {
                    self.focus_active(window, cx);
                }
            }
            PanelDo::End => {
                if let Some(id) = self.terminals.get(self.active).map(|t| t.id) {
                    self.close(id, window, cx);
                }
            }
            PanelDo::Hide => {
                if self.active_has_keys(window, cx) {
                    cx.emit(PanelEvent::Left);
                }
                self.set_shown(false, cx);
            }
        }
        cx.notify();
    }

    /// Whether the terminal on show has text selected.
    fn has_selection(&self, cx: &App) -> bool {
        self.terminals
            .get(self.active)
            .and_then(Terminal::text)
            .is_some_and(|text| text.read(cx).has_selection())
    }

    /// Open the menu of the text where the pointer is.
    pub fn open_menu(
        &mut self,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = cx.entity().downgrade();
        let selection = self.has_selection(cx);
        let menu = PopupMenu::build(window, cx, move |menu, _, _| {
            let entry = |label: &'static str, key: Option<SharedString>, what: PanelDo| {
                let panel = panel.clone();
                menu_entry(label, key).on_click(move |_, window, cx| {
                    let _ = panel.update(cx, |panel, cx| panel.run(what, window, cx));
                })
            };
            // In groups: the text, then the line and the screen, then
            // the panel. Copy only with a selection.
            menu.min_w(px(MENU_WIDTH))
                .item(
                    entry(say!("terminal-menu-copy"), Some(copy_key()), PanelDo::Copy)
                        .disabled(!selection),
                )
                .item(entry(
                    say!("terminal-menu-paste"),
                    Some(paste_key()),
                    PanelDo::Paste,
                ))
                .item(entry(
                    say!("terminal-menu-select-all"),
                    None,
                    PanelDo::SelectAll,
                ))
                .separator()
                .item(entry(
                    say!("terminal-menu-delete-line"),
                    Some(delete_line_key()),
                    PanelDo::DeleteLine,
                ))
                .item(entry(
                    say!("terminal-menu-clear"),
                    Some(clear_key()),
                    PanelDo::Clear,
                ))
                .separator()
                .item(entry(
                    say!("terminal-menu-minimise"),
                    None,
                    PanelDo::Minimise,
                ))
                .item(entry(say!("terminal-menu-end"), None, PanelDo::End))
                .item(entry(say!("terminal-menu-hide"), None, PanelDo::Hide))
        });
        let closed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.menu = None;
            this.focus_active(window, cx);
            cx.notify();
        });
        // Once the click that opened it is done: the text takes the keys
        // on a click of any button.
        let focused = menu.focus_handle(cx);
        cx.defer_in(window, move |_, window, cx| focused.focus(window, cx));
        self.menu = Some(OpenMenu {
            menu,
            position,
            _closed: closed,
        });
        cx.notify();
    }

    /// The labels of the menu that is open, for the tests: empty while
    /// none is.
    pub fn menu_labels(&self) -> Vec<&'static str> {
        if self.menu.is_none() {
            return Vec::new();
        }
        MENU_LABELS.iter().map(Message::text).collect()
    }

    fn render_header(&self, minimised: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let several = self.terminals.len() > 1;
        let tabs = several.then(|| {
            h_flex()
                .id("terminal-tabs")
                .test_support()
                .min_w_0()
                .flex_shrink(1.)
                .ml(px(10.))
                .gap(px(2.))
                .overflow_x_scroll()
                .children(
                    self.terminals
                        .iter()
                        .enumerate()
                        .map(|(index, terminal)| self.render_tab(index, terminal, cx)),
                )
        });
        // The sign is a child, not the button's icon, which the button
        // sizes itself.
        let button = |id: &'static str, icon: Icon, tip: &'static str| {
            Button::new(id)
                .accessibility_label(tip)
                .ghost()
                .xsmall()
                .w(px(BUTTON_WIDTH))
                .h(px(BUTTON_WIDTH))
                .px_0()
                .child(
                    div()
                        .size_full()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(icon),
                )
                .tooltip(tip)
        };
        h_flex()
            .id("terminal-heading")
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .w_full()
            .items_center()
            .pl(px(ICON_INSET))
            .pr(px(LAST_BUTTON_INSET - BUTTON_WIDTH / 2.))
            .text_size(px(13.))
            .child(
                Icon::new(gpui_kit::assets::IconName::Terminal)
                    .size(px(ICON_SIZE))
                    .flex_none()
                    .text_color(theme.foreground),
            )
            .child(
                div()
                    .ml(px(4.))
                    .flex_none()
                    .text_color(theme.foreground)
                    .child(say!("terminal-heading")),
            )
            .children(tabs)
            .child(
                Button::new("terminal-new")
                    .accessibility_label(say!("terminal-new"))
                    .ghost()
                    .xsmall()
                    .ml(px(if several { 4. } else { 8. }))
                    .icon(IconName::Plus)
                    .tooltip(say!("terminal-new"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PanelEvent::Wanted(None)))),
            )
            .child(div().flex_1())
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(BUTTON_PITCH - BUTTON_WIDTH))
                    .child(
                        button(
                            "terminal-minimise",
                            // The signs: a chevron 10 wide, a bin 10 by
                            // 12, a cross 8.
                            Icon::new(if minimised {
                                IconName::ChevronUp
                            } else {
                                IconName::ChevronDown
                            })
                            .size(px(20.)),
                            if minimised {
                                say!("terminal-restore")
                            } else {
                                say!("terminal-minimise")
                            },
                        )
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.run(PanelDo::Minimise, window, cx)
                        })),
                    )
                    .child(
                        button(
                            "terminal-end",
                            Icon::new(gpui_kit::assets::IconName::Trash).size(px(14.)),
                            say!("terminal-end"),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.run(PanelDo::End, window, cx)),
                        ),
                    )
                    .child(
                        button(
                            "terminal-hide",
                            Icon::new(IconName::Close).size(px(16.)),
                            say!("terminal-hide"),
                        )
                        .on_click(
                            cx.listener(|this, _, window, cx| this.run(PanelDo::Hide, window, cx)),
                        ),
                    ),
            )
    }

    fn render_tab(&self, index: usize, terminal: &Terminal, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active = index == self.active;
        let id = terminal.id;
        let group: SharedString = format!("terminal-tab-{id}").into();
        h_flex()
            .id(("terminal-tab", id))
            .group(group.clone())
            .h(px(22.))
            .max_w(px(TAB_MOST))
            .min_w_0()
            .flex_shrink(1.)
            .pl(px(8.))
            .pr(px(2.))
            .gap(px(2.))
            .items_center()
            .rounded(px(4.))
            .text_size(px(12.))
            .cursor_pointer()
            .map(|tab| {
                if active {
                    tab.bg(theme.secondary).text_color(theme.foreground)
                } else {
                    tab.text_color(theme.muted_foreground)
                        .hover(|style| style.bg(theme.secondary.opacity(0.5)))
                }
            })
            .on_click(cx.listener(move |this, _, window, cx| this.select(index, window, cx)))
            // The middle button closes the one it is over.
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.close(id, window, cx);
                }),
            )
            .child(div().min_w_0().truncate().child(terminal.name()))
            .child(
                div()
                    .size(px(16.))
                    .flex_none()
                    .invisible()
                    .group_hover(group, |style| style.visible())
                    .child(
                        Button::new(("terminal-tab-close", id))
                            .accessibility_label(say!("terminal-tab-close"))
                            .ghost()
                            .xsmall()
                            .w(px(16.))
                            .h(px(16.))
                            .icon(Icon::new(IconName::Close).size_3())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.close(id, window, cx);
                            })),
                    ),
            )
            .test_support()
            .into_any_element()
    }

    fn render_body(&self, colors: &TerminalColors, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(terminal) = self.terminals.get(self.active) else {
            return div().into_any_element();
        };
        let id = terminal.id;
        let inner: AnyElement = match &terminal.body {
            Body::Starting => div().into_any_element(),
            // Said where the text would begin.
            Body::Failed(why) => div()
                .pl(px(12.))
                .pt(px(12.))
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(why.clone())
                .into_any_element(),
            // The text pads itself; what redraws the window for something
            // else reuses what it drew.
            Body::Text(text) => div()
                .id(("terminal-text", id))
                .size_full()
                .child(AnyView::from(text.clone()).cached(StyleRefinement::default().size_full()))
                .test_support()
                .into_any_element(),
        };
        div()
            .id(("terminal-body", id))
            .flex_1()
            .min_h_0()
            .w_full()
            // The panels beside the column draw the rules at its sides.
            .bg(colors.background)
            .child(inner)
            .into_any_element()
    }
}

/// How wide the menu over the text is at the least.
const MENU_WIDTH: f32 = 260.0;

/// The keys the menu names beside its entries: what the text does with
/// them. Ctrl+C copies while there is a selection and stops what runs
/// otherwise; Ctrl+Shift+C always copies. On macOS Cmd+C and Cmd+V copy
/// and paste. The last two are the shell's.
fn copy_key() -> SharedString {
    copy_or_paste("ctrl-c", "cmd-c")
}

fn paste_key() -> SharedString {
    copy_or_paste("ctrl-shift-v", "cmd-v")
}

fn copy_or_paste(elsewhere: &str, mac: &str) -> SharedString {
    crate::keymap::display_key(match crate::keymap::System::now() {
        crate::keymap::System::Mac => mac,
        crate::keymap::System::Other => elsewhere,
    })
    .into()
}

fn delete_line_key() -> SharedString {
    crate::keymap::display_key("ctrl-u").into()
}

fn clear_key() -> SharedString {
    crate::keymap::display_key("ctrl-l").into()
}

/// The menu's entries, in order.
const MENU_LABELS: [Message; 8] = [
    message!("terminal-menu-copy"),
    message!("terminal-menu-paste"),
    message!("terminal-menu-select-all"),
    message!("terminal-menu-delete-line"),
    message!("terminal-menu-clear"),
    message!("terminal-menu-minimise"),
    message!("terminal-menu-end"),
    message!("terminal-menu-hide"),
];

/// An entry of the menu: its label, and its key at the right.
fn menu_entry(label: &'static str, key: Option<SharedString>) -> PopupMenuItem {
    PopupMenuItem::element(move |_, cx| {
        h_flex()
            .w_full()
            .gap_6()
            .justify_between()
            .child(label)
            .children(key.clone().map(|key| {
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(key)
            }))
    })
}

/// End a terminal's shell and its group.
fn end(terminal: Terminal, cx: &App) {
    if let Some(text) = terminal.text() {
        text.read(cx).session().kill();
    }
}

impl Render for TerminalPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let colors = self.apply_look(cx).colors;
        let minimised = self.minimised(cx);

        let menu = self.menu.as_ref().map(|open| {
            deferred(
                anchored()
                    .position(open.position)
                    .snap_to_window_with_margin(px(8.))
                    .child(open.menu.clone()),
            )
            .with_priority(1)
        });
        v_flex()
            .id("terminal-panel")
            .size_full()
            .overflow_hidden()
            .bg(colors.background)
            .border_t_1()
            .border_color(theme.border)
            .child(self.render_header(minimised, cx))
            .map(|panel| {
                if minimised {
                    panel.border_b_1()
                } else {
                    panel
                        .child(div().h(px(RULE_HEIGHT)).flex_none().bg(theme.border))
                        .child(self.render_body(&colors, cx))
                }
            })
            .children(menu)
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::clean_title;

    #[test]
    fn a_title_is_cleaned_and_cut() {
        assert_eq!(clean_title("vim notes.md").as_deref(), Some("vim notes.md"));
        assert_eq!(clean_title("  \u{7}\u{1b}]0;  ").as_deref(), Some("]0;"));
        assert_eq!(clean_title(""), None);
        assert_eq!(clean_title("\u{1b}\u{7}\n"), None);
        let long = "x".repeat(10_000);
        let cut = clean_title(&long).unwrap();
        assert_eq!(cut.chars().count(), super::TITLE_MOST);
        assert!(cut.ends_with('…'));
        // Wide characters are characters, not bytes.
        assert_eq!(clean_title(&"é".repeat(70)).unwrap().chars().count(), 60);
    }
}
