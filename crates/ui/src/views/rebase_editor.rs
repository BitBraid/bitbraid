//! The interactive rebase editor: the commits from one commit up to the
//! current branch, each with what is to happen to it.
//!
//! The list reads like the graph, newest on top, each commit with its
//! author's picture; a commit that is squashed melts into the one below
//! it. Every commit is kept, reworded, squashed, fixed up, stopped at or
//! dropped: from the choice at the left of its row or with a single key;
//! and moved by dragging or with Ctrl (or Alt) and the arrows. Rewording
//! opens a small form under the row for the new message. Under the list,
//! at the left, what the keys do; at the right: start over, give up,
//! start (what there will be is the last one's tooltip). Nothing happens
//! to the repository until the plan is started.
//!
//! Sources: git-rebase(1) (interactive mode); gpui-kit documentation
//! (virtual list, text area, drag and drop, key events, popup menu).

use bitbraid_git::interactive::{self, Action, Entry, Range, Resulting, Step};
use bitbraid_strings::Message;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::DropdownMenu,
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    session::{Reply, Request, SessionHandle},
    views::{avatar::avatar, now, relative_time, rewrite::DragChip},
};

const ROW_HEIGHT: f32 = 32.0;
/// Height of the row that says what is planned.
const TITLE_HEIGHT: f32 = 36.0;
/// The space left and right of the list, and over it.
const MARGIN: f32 = 20.0;
/// Width and height of a row's choice of what happens to its commit.
const CHOICE_WIDTH: f32 = 90.0;
const CHOICE_HEIGHT: f32 = 24.0;
/// Width of the cell a commit's mark stands in, and the mark's size.
const MARK_CELL: f32 = 58.0;
const MARK: f32 = 22.0;
/// Height of the buttons that reset, give up and start.
const BUTTON_HEIGHT: f32 = 30.0;
/// Most resulting commits listed in the preview.
const PREVIEWED: usize = 20;
/// The form under a row that is reworded: how far from the list's left
/// it stands, how wide and how high it is (with the space over it).
const FORM_LEFT: f32 = 187.0;
const FORM_WIDTH: f32 = 400.0;
const FORM_HEIGHT: f32 = 216.0;

/// Events from the editor.
#[derive(Debug, Clone)]
pub enum RebaseEvent {
    /// A commit of the plan was selected: the details beside the plan are
    /// about it.
    Selected(bitbraid_git::Oid),
    /// The plan was given up.
    Cancelled,
    /// Carry the plan out.
    Run {
        /// The oldest commit of the plan.
        first: String,
        /// One step per commit, in the order they are applied.
        steps: Vec<Step>,
        /// The base they are replayed on, when it is not where they are.
        onto: Option<String>,
        /// Without the hook that could refuse the rebase.
        no_verify: bool,
    },
    /// Give one commit another message, where no plan for more can be
    /// made: merge commits after it stay as they are.
    Reword {
        /// The commit.
        commit: String,
        /// Its new message.
        message: String,
    },
}

/// Why only the message of a commit can change here and no plan for the
/// commits after it can be made: what `commit.rewrite_way` said.
pub(crate) fn why_alone(merge: bool, first: bool) -> &'static str {
    if merge {
        say!("rebase-alone-merge")
    } else if first {
        say!("rebase-alone-first")
    } else {
        say!("rebase-alone-behind-merge")
    }
}

/// One commit of the plan.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    entry: Entry,
    action: Action,
    /// The new message of a commit that is reworded.
    message: Option<String>,
}

/// The plan as it is edited: the commits, oldest first.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PlanModel {
    rows: Vec<Row>,
}

impl PlanModel {
    /// The plan that changes nothing.
    pub fn new(range: &Range) -> Self {
        Self {
            rows: range
                .commits
                .iter()
                .map(|entry| Row {
                    entry: entry.clone(),
                    action: Action::Pick,
                    message: None,
                })
                .collect(),
        }
    }

    /// How many commits it has.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether it has none.
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    /// What is to happen to the commit at `row`. A commit that is to be
    /// reworded starts from the message it has.
    pub fn set_action(&mut self, row: usize, action: Action) {
        if let Some(row) = self.rows.get_mut(row) {
            row.action = action;
            if action == Action::Reword {
                if row.message.is_none() {
                    row.message = Some(row.entry.message.clone());
                }
            } else {
                row.message = None;
            }
        }
    }

    /// The new message of the commit at `row`, when it is reworded.
    pub fn set_message(&mut self, row: usize, message: String) {
        if let Some(row) = self.rows.get_mut(row)
            && row.action == Action::Reword
        {
            row.message = Some(message);
        }
    }

    /// Give a commit back the new message it had, or none.
    pub fn restore_message(&mut self, row: usize, message: Option<String>) {
        if let Some(row) = self.rows.get_mut(row) {
            row.message = message;
        }
    }

    /// Move the commit at `from` so that it is at `to`; where it ended up.
    pub fn place(&mut self, from: usize, to: usize) -> usize {
        if from >= self.rows.len() {
            return from;
        }
        let to = to.min(self.rows.len().saturating_sub(1));
        if from != to {
            let row = self.rows.remove(from);
            self.rows.insert(to, row);
        }
        to
    }

    /// Move the commit at `row` one place: later in the history or
    /// earlier. Where it ended up.
    pub fn shift(&mut self, row: usize, later: bool) -> usize {
        let to = if later {
            row.saturating_add(1)
        } else {
            row.saturating_sub(1)
        };
        self.place(row, to)
    }

    /// The plan as the engine takes it.
    pub fn steps(&self) -> Vec<Step> {
        self.rows
            .iter()
            .map(|row| Step {
                id: row.entry.id,
                action: row.action,
                message: row.message.clone(),
            })
            .collect()
    }

    /// How many commits that the upstream already has would be made anew:
    /// those from the first change of the plan on.
    pub fn published_rewritten(&self, range: &Range) -> usize {
        let untouched = interactive::untouched(range);
        let steps = self.steps();
        let first_change = steps
            .iter()
            .zip(&untouched)
            .position(|(step, original)| step != original);
        match first_change {
            Some(from) => self
                .rows
                .iter()
                .skip(from)
                .filter(|row| row.entry.published)
                .count(),
            None => 0,
        }
    }
}

/// What the editor holds.
enum State {
    Empty,
    Loading,
    /// The commits cannot be planned for, and why.
    Refused(String),
    Open(Range),
}

/// A row picked up to be dropped somewhere else in the list.
#[derive(Debug, Clone, Copy)]
struct DraggedRow(usize);

/// What a row's own choice was set to: the commit of the plan (oldest
/// first) and which of [`ACTIONS`].
#[derive(Action, Clone, Debug, PartialEq, Eq, serde::Deserialize)]
#[action(namespace = rebase_editor, no_json)]
struct Choose {
    row: usize,
    action: usize,
}

/// The interactive rebase editor.
pub struct RebaseEditor {
    session: SessionHandle,
    first: String,
    /// The new base of the plan, when it moves the commits somewhere else.
    onto: Option<String>,
    state: State,
    model: PlanModel,
    /// Set when the plan is one commit by itself, to be reworded and
    /// nothing else: why no plan for more can be made.
    alone: Option<&'static str>,
    /// The selected commit, as an index into the plan (oldest first).
    selected: usize,
    /// The message of the selected commit, to be changed when it is
    /// reworded.
    message: Entity<TextareaState>,
    /// What the plan leads to, or what is wrong with it.
    outline: Result<Vec<Resulting>, String>,
    /// The hook that could refuse the rebase, when it is there: what the
    /// switch beside the buttons leaves out.
    refusing_hooks: Vec<String>,
    /// Start without it. For this plan: a plan opened anew runs the hook.
    skip_hooks: bool,
    /// The row whose new message is being written in the form under it,
    /// with what it was to have before the form opened.
    rewording: Option<(usize, Action, Option<String>)>,
    sizes: crate::zoom::Sizes,
    scroll: VirtualListScrollHandle,
    focus_handle: FocusHandle,
    _subscription: Subscription,
}

impl EventEmitter<RebaseEvent> for RebaseEditor {}

/// What `commit.rewrite_way` answered, when it says that no plan can be
/// made from the commit: the commit by itself, and why.
fn alone_of(value: &serde_json::Value) -> Option<(Range, &'static str)> {
    if value.get("plain").and_then(|plain| plain.as_bool()) != Some(false) {
        return None;
    }
    let flag = |name: &str| value.get(name).and_then(|flag| flag.as_bool()) == Some(true);
    let range: Range = serde_json::from_value(value.get("alone")?.clone()).ok()?;
    (range.commits.len() == 1).then(|| (range, why_alone(flag("merge"), flag("first"))))
}

/// What a row's choice says for an action: its name among [`ACTIONS`].
fn name_of(action: Action) -> &'static str {
    ACTIONS
        .iter()
        .find(|(candidate, ..)| *candidate == action)
        .map_or(say!("rebase-action-pick"), |(_, _, name, _)| name.text())
}

/// A refusal as it is shown: starting with a capital.
fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// The keys said under the list, in this order.
const KEYS_SHOWN: [Action; 6] = [
    Action::Pick,
    Action::Squash,
    Action::Reword,
    Action::Drop,
    Action::Fixup,
    Action::Edit,
];

/// The actions with their keys, in the order a row's choice shows them.
const ACTIONS: [(Action, &str, Message, Message); 6] = [
    (
        Action::Pick,
        "p",
        message!("rebase-action-pick"),
        message!("rebase-action-pick-note"),
    ),
    (
        Action::Reword,
        "r",
        message!("rebase-action-reword"),
        message!("rebase-action-reword-note"),
    ),
    (
        Action::Squash,
        "s",
        message!("rebase-action-squash"),
        message!("rebase-action-squash-note"),
    ),
    (
        Action::Fixup,
        "f",
        message!("rebase-action-fixup"),
        message!("rebase-action-fixup-note"),
    ),
    (
        Action::Edit,
        "e",
        message!("rebase-action-edit"),
        message!("rebase-action-edit-note"),
    ),
    (
        Action::Drop,
        "d",
        message!("rebase-action-drop"),
        message!("rebase-action-drop-note"),
    ),
];

impl RebaseEditor {
    /// Create the view.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let message = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("rebase-message-placeholder"))
                // Tab goes on to the next control: a text that grows,
                // unlike one of fixed rows, is not indented.
                .auto_grow(8, 8)
        });
        let subscription = cx.subscribe(&message, |this, state, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                let text = state.read(cx).value().to_string();
                let selected = this.selected;
                let before = this.model.clone();
                this.model.set_message(selected, text);
                if this.model != before {
                    this.check();
                    cx.notify();
                }
            }
        });
        Self {
            session,
            first: String::new(),
            onto: None,
            state: State::Empty,
            model: PlanModel::default(),
            alone: None,
            selected: 0,
            message,
            outline: Ok(Vec::new()),
            refusing_hooks: Vec::new(),
            skip_hooks: false,
            rewording: None,
            sizes: crate::zoom::Sizes::default(),
            scroll: VirtualListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
            _subscription: subscription,
        }
    }

    /// Plan for the commits from `first` up to the current branch. With
    /// `reword`, the first of them starts out as the one to be reworded.
    pub fn open(
        &mut self,
        first: String,
        reword: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.first = first.clone();
        self.onto = None;
        self.alone = None;
        self.state = State::Loading;
        self.model = PlanModel::default();
        self.outline = Ok(Vec::new());
        self.sizes = crate::zoom::Sizes::default();
        // A reword asks first whether a plan can be made from the commit:
        // behind a merge commit, of one, or of the first commit of all,
        // only the message changes and the commit is shown by itself.
        let way = reword.then(|| {
            self.session.send(Request::Command {
                id: "commit.rewrite_way".to_owned(),
                params: json!({ "commit": first }),
            })
        });
        let session = self.session.clone();
        cx.spawn_in(window, async move |this, cx| {
            if let Some(way) = way {
                let Ok(reply) = way.await else { return };
                let alone = match reply {
                    Reply::Command(value) => alone_of(&value),
                    Reply::Error(err) => {
                        let _ = this.update_in(cx, |this, _, cx| {
                            if this.first == first {
                                this.state = State::Refused(err);
                                cx.notify();
                            }
                        });
                        return;
                    }
                    _ => None,
                };
                if let Some((range, why)) = alone {
                    let _ = this.update_in(cx, |this, window, cx| {
                        if this.first == first {
                            this.alone = Some(why);
                            this.loaded(range, true, window, cx);
                            cx.notify();
                        }
                    });
                    return;
                }
            }
            let reply = session.send(Request::Command {
                id: "rebase.plan".to_owned(),
                params: json!({ "first": first }),
            });
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.first != first {
                    return;
                }
                match reply {
                    Reply::Command(value) => match serde_json::from_value::<Range>(value) {
                        Ok(range) => this.loaded(range, reword, window, cx),
                        Err(err) => this.state = State::Refused(err.to_string()),
                    },
                    Reply::Error(err) => this.state = State::Refused(err),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Plan for the commits the current branch has and `onto` does not,
    /// to be replayed on `onto`: an interactive rebase onto another base.
    pub fn open_onto(&mut self, onto: String, window: &mut Window, cx: &mut Context<Self>) {
        self.first = String::new();
        self.onto = Some(onto.clone());
        self.alone = None;
        self.state = State::Loading;
        self.model = PlanModel::default();
        self.outline = Ok(Vec::new());
        self.sizes = crate::zoom::Sizes::default();
        let reply = self.session.send(Request::Command {
            id: "rebase.plan".to_owned(),
            params: json!({ "onto": onto }),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else { return };
            let _ = this.update_in(cx, |this, window, cx| {
                if this.onto.as_deref() != Some(onto.as_str()) {
                    return;
                }
                match reply {
                    Reply::Command(value) => {
                        // Where the plan begins is the engine's to say.
                        let first = value
                            .get("first")
                            .and_then(|first| first.as_str())
                            .map(str::to_owned);
                        match (first, serde_json::from_value::<Range>(value)) {
                            (Some(first), Ok(range)) => {
                                this.first = first;
                                this.loaded(range, false, window, cx);
                            }
                            (_, Err(err)) => this.state = State::Refused(err.to_string()),
                            (None, _) => {
                                this.state = State::Refused(say!("rebase-no-beginning").into())
                            }
                        }
                    }
                    Reply::Error(err) => this.state = State::Refused(err),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    fn loaded(&mut self, range: Range, reword: bool, window: &mut Window, cx: &mut Context<Self>) {
        self.model = PlanModel::new(&range);
        // The oldest when it is the one to reword, else the newest, which
        // is the top of the list.
        self.selected = if reword {
            0
        } else {
            self.model.len().saturating_sub(1)
        };
        if reword {
            self.model.set_action(0, Action::Reword);
        }
        self.rewording = None;
        self.state = State::Open(range);
        self.check();
        self.say_selected(cx);
        self.show_message(window, cx);
        if reword {
            self.open_form(Action::Pick, None, window, cx);
        } else {
            self.focus_handle.focus(window, cx);
        }
        self.resize();
    }

    /// The heights of the rows (the form of a row that is reworded
    /// stands over the rows under it, and takes no room).
    fn resize(&mut self) {
        self.sizes = crate::zoom::Sizes::new(vec![size(px(0.), px(ROW_HEIGHT)); self.model.len()]);
    }

    /// Open the form under the selected row for its new message; what it
    /// was to be before is kept for the form's cancel.
    fn open_form(
        &mut self,
        before: Action,
        message: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.rewording = Some((self.selected, before, message));
        self.show_message(window, cx);
        // The cursor at the end of the summary, to go on typing it.
        let summary = self
            .model
            .rows
            .get(self.selected)
            .and_then(|row| row.message.as_deref())
            .and_then(|message| message.lines().next())
            .map(|line| u32::try_from(line.chars().count()).unwrap_or(u32::MAX))
            .unwrap_or(0);
        self.message.update(cx, |state, cx| {
            state.set_cursor_position(
                gpui_kit::component::input::Position {
                    line: 0,
                    character: summary,
                },
                window,
                cx,
            )
        });
        self.resize();
        cx.notify();
    }

    /// Close the form: with what was typed, or giving it up, which puts
    /// the commit back to what it was to be before.
    fn close_form(&mut self, keep: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some((row, before, message)) = self.rewording.take() else {
            return;
        };
        if !keep {
            self.model.set_action(row, before);
            self.model.restore_message(row, message);
            self.check();
            self.show_message(window, cx);
        }
        self.focus_handle.focus(window, cx);
        self.resize();
        cx.notify();
    }

    /// Check the plan and work out what it leads to.
    fn check(&mut self) {
        self.outline = match &self.state {
            State::Open(range) => {
                interactive::outline(range, &self.model.steps()).map_err(|err| err.to_string())
            }
            _ => Ok(Vec::new()),
        };
    }

    /// Put the selected commit's message into the field: the new one when
    /// it is reworded, else the one it has.
    fn show_message(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let text = self
            .model
            .rows
            .get(self.selected)
            .map(|row| {
                row.message
                    .clone()
                    .unwrap_or_else(|| row.entry.message.clone())
            })
            .unwrap_or_default();
        self.message
            .update(cx, |state, cx| state.set_value(text, window, cx));
    }

    fn select(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) {
        if row >= self.model.len() {
            return;
        }
        // Another row: the form keeps what was typed and closes.
        if self.rewording.as_ref().is_some_and(|(at, ..)| *at != row) {
            self.close_form(true, window, cx);
        }
        self.selected = row;
        self.show_message(window, cx);
        self.scroll
            .scroll_to_item(self.shown_index(row), ScrollStrategy::Center);
        self.say_selected(cx);
        cx.notify();
    }

    /// Say which commit is selected, for the details beside the plan.
    fn say_selected(&mut self, cx: &mut Context<Self>) {
        if let Some(row) = self.model.rows.get(self.selected) {
            cx.emit(RebaseEvent::Selected(row.entry.id));
        }
    }

    /// Where a commit of the plan is in the list, which is newest first.
    fn shown_index(&self, row: usize) -> usize {
        self.model.len().saturating_sub(1).saturating_sub(row)
    }

    fn act(&mut self, action: Action, window: &mut Window, cx: &mut Context<Self>) {
        // A commit by itself is reworded and nothing else.
        if self.alone.is_some() && action != Action::Reword {
            return;
        }
        let before = self
            .model
            .rows
            .get(self.selected)
            .map(|row| (row.action, row.message.clone()));
        self.model.set_action(self.selected, action);
        self.check();
        self.show_message(window, cx);
        match before {
            Some((before, message)) if action == Action::Reword => {
                self.open_form(before, message, window, cx)
            }
            // Something else than rewording: the form has no business.
            _ if self.rewording.is_some() => {
                self.rewording = None;
                self.resize();
            }
            _ => {}
        }
        cx.notify();
    }

    /// Move the selected commit one place up or down the list.
    fn shift(&mut self, up: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.alone.is_some() {
            return;
        }
        self.close_form(true, window, cx);
        // Up the list is later in the history.
        let to = self.model.shift(self.selected, up);
        self.check();
        self.select(to, window, cx);
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let State::Open(range) = &self.state {
            self.model = PlanModel::new(range);
            self.selected = self.selected.min(self.model.len().saturating_sub(1));
            self.rewording = None;
            if self.alone.is_some() {
                self.model.set_action(0, Action::Reword);
                self.rewording = Some((0, Action::Reword, None));
            }
            self.check();
            self.show_message(window, cx);
            self.resize();
            cx.notify();
        }
    }

    /// Whether the plan differs from leaving everything as it is.
    fn changed(&self) -> bool {
        match &self.state {
            // A commit by itself: when its message is another one.
            State::Open(_) if self.alone.is_some() => self.reworded().is_some(),
            State::Open(range) => self.model.steps() != interactive::untouched(range),
            _ => false,
        }
    }

    /// For a commit by itself: the commit and its new message, when that
    /// is not the one it has.
    fn reworded(&self) -> Option<(String, String)> {
        self.alone?;
        let row = self.model.rows.first()?;
        let message = row.message.as_ref()?;
        let tidy = |text: &str| text.trim_matches(['\n', '\r', ' ', '\t']).to_owned();
        (tidy(message) != tidy(&row.entry.message))
            .then(|| (row.entry.id.to_hex(), message.clone()))
    }

    /// The hooks that could refuse the rebase and are there, told before
    /// a plan is opened. The switch starts off for every plan.
    pub fn set_refusing_hooks(&mut self, hooks: Vec<String>) {
        self.refusing_hooks = hooks;
        self.skip_hooks = false;
    }

    /// Whether the plan is to be started without the hook that could
    /// refuse it: switched on, with such a hook there.
    fn skips_hooks(&self) -> bool {
        self.skip_hooks && !self.refusing_hooks.is_empty()
    }

    /// Whether there is something to carry out: a plan that changes a
    /// commit, or any plan at all when it moves them to another base.
    fn runnable(&self) -> bool {
        self.changed() || (self.onto.is_some() && matches!(self.state, State::Open(_)))
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.alone.is_some() {
            if let (true, Some((commit, message))) = (self.outline.is_ok(), self.reworded()) {
                cx.emit(RebaseEvent::Reword { commit, message });
            }
            return;
        }
        if self.outline.is_ok() && self.runnable() {
            cx.emit(RebaseEvent::Run {
                first: self.first.clone(),
                steps: self.model.steps(),
                onto: self.onto.clone(),
                no_verify: self.skips_hooks(),
            });
        }
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Only with the list itself in focus: in the message field the
        // same keys are letters.
        if !self.focus_handle.is_focused(window) || !matches!(self.state, State::Open(_)) {
            return;
        }
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        let plain = !modifiers.control && !modifiers.alt && !modifiers.platform;
        let key = keystroke.key.as_str();
        // The list is newest first: up is the next commit in the history.
        match key {
            "up" if modifiers.alt || modifiers.control => self.shift(true, window, cx),
            "down" if modifiers.alt || modifiers.control => self.shift(false, window, cx),
            "up" if plain => self.select(self.selected.saturating_add(1), window, cx),
            "down" if plain => {
                if let Some(row) = self.selected.checked_sub(1) {
                    self.select(row, window, cx);
                }
            }
            "enter" if modifiers.control || modifiers.platform => self.start(cx),
            "escape" => cx.emit(RebaseEvent::Cancelled),
            _ if plain => {
                let Some((action, ..)) = ACTIONS.iter().find(|(_, k, ..)| *k == key) else {
                    return;
                };
                self.act(*action, window, cx);
            }
            _ => return,
        }
        cx.stop_propagation();
    }

    fn render_row(&self, shown: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        let index = self.model.len().checked_sub(1)?.checked_sub(shown)?;
        let row = self.model.rows.get(index)?;
        let selected = index == self.selected;
        let colour = match row.action {
            Action::Pick => theme.green,
            Action::Reword => theme.blue,
            Action::Squash | Action::Fixup => theme.yellow,
            Action::Edit => theme.cyan,
            Action::Drop => theme.red,
        };
        let dropped = row.action == Action::Drop;
        let melts = matches!(row.action, Action::Squash | Action::Fixup);
        let summary = match (&row.message, row.action) {
            (Some(message), Action::Reword) => message.lines().next().unwrap_or("").to_owned(),
            _ => row.entry.summary.clone(),
        };
        let dragged_label = format!("{} {}", row.entry.id.short(7), row.entry.summary);
        let lane = crate::theme::lane_color(0, cx);
        let alone = self.alone.is_some();
        // What happens to the commit: the choice at the left of its row.
        let choice = Button::new(("plan-choice", index))
            .outline()
            .xsmall()
            .w(px(CHOICE_WIDTH))
            .h(px(CHOICE_HEIGHT))
            .border_color(colour)
            .label(name_of(row.action))
            .dropdown_caret(true)
            .dropdown_menu(move |menu, _, _| {
                ACTIONS
                    .iter()
                    .enumerate()
                    // A commit by itself is reworded and nothing else.
                    .filter(|(_, (action, ..))| !alone || *action == Action::Reword)
                    .fold(menu, |menu, (action, (_, key, name, _))| {
                        menu.menu(
                            format!("{name}  ({})", key.to_uppercase()),
                            Box::new(Choose { row: index, action }),
                        )
                    })
            });
        // The commit's mark, as in the graph: a commit that melts into
        // the one below, or is left out, stands aside.
        let aside = melts || dropped;
        let mark = h_flex()
            .relative()
            .w(px(MARK_CELL))
            .h_full()
            .flex_none()
            .items_center()
            .child(
                div()
                    .absolute()
                    .left(px(MARK / 2.0))
                    .right_0()
                    .top(px(4.))
                    .bottom(px(4.))
                    .bg(lane.opacity(if selected { 0.38 } else { 0.16 })),
            )
            .when(melts, |cell| {
                cell.child(
                    div()
                        .absolute()
                        .left(px(6.))
                        .text_xs()
                        .text_color(theme.foreground)
                        .child("↓"),
                )
            })
            // The author's picture in a ring of the line's colour.
            .child(
                div()
                    .relative()
                    .ml(px(if aside { MARK + 2.0 } else { 0.0 }))
                    .size(px(MARK))
                    .flex_none()
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .overflow_hidden()
                    .border_2()
                    .border_color(lane)
                    .bg(theme.background)
                    .when(dropped, |mark| mark.opacity(0.45))
                    .child(avatar(
                        &row.entry.email,
                        px(MARK - 6.0),
                        theme.is_dark(),
                        cx,
                    )),
            );
        let line = h_flex()
            .id(("plan-row", index))
            .w_full()
            .h(px(ROW_HEIGHT))
            .px(px(MARGIN))
            .gap(px(MARGIN))
            .items_center()
            .text_sm()
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                let again = this.selected == index && this.rewording.is_none();
                this.select(index, window, cx);
                this.focus_handle.focus(window, cx);
                // A reworded commit clicked again: its form opens.
                let reworded = this
                    .model
                    .rows
                    .get(index)
                    .filter(|row| row.action == Action::Reword)
                    .map(|row| row.message.clone());
                if let (true, Some(message)) = (again, reworded) {
                    this.open_form(Action::Reword, message, window, cx);
                }
            }))
            .on_drag(DraggedRow(index), move |_, _, _, cx| {
                cx.new(|_| DragChip(dragged_label.clone()))
            })
            .drag_over::<DraggedRow>(|style, _, _, cx| {
                style.border_t_2().border_color(cx.theme().primary)
            })
            .on_drop(cx.listener(move |this, dragged: &DraggedRow, window, cx| {
                this.close_form(true, window, cx);
                let to = this.model.place(dragged.0, index);
                this.check();
                this.select(to, window, cx);
            }))
            .child(choice)
            // From the mark to the row's end is what is tinted when
            // the commit is the selected one.
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .h(px(CHOICE_HEIGHT))
                    .gap_2()
                    .items_center()
                    .when(selected, |row| row.bg(theme.primary.opacity(0.24)))
                    .hover(|style| style.bg(theme.list_hover))
                    // The mark's cell ends in the bar of its line; the
                    // two are one piece.
                    .child(
                        h_flex()
                            .flex_none()
                            .h_full()
                            .child(mark)
                            .child(div().flex_none().w(px(2.)).h_full().bg(lane)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .pl_0p5()
                            .truncate()
                            .when(melts, |cell| cell.text_color(theme.muted_foreground))
                            .when(dropped, |cell| {
                                cell.line_through().text_color(theme.muted_foreground)
                            })
                            .child(summary),
                    )
                    .when(row.entry.published, |row| {
                        row.child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.yellow)
                                .child(say!("rebase-pushed")),
                        )
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(relative_time(row.entry.time, now())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .pr_2()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(row.entry.id.short(7)),
                    ),
            );
        Some(line.into_any_element())
    }

    /// The form under a row that is reworded: the new message, how many
    /// characters its first line has left of 72, and the buttons that give
    /// it up or take it.
    fn render_form(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let first = self
            .model
            .rows
            .get(self.selected)
            .and_then(|row| row.message.as_deref())
            .and_then(|message| message.lines().next())
            .map(|line| line.chars().count())
            .unwrap_or(0);
        let left = 72_i64.saturating_sub(i64::try_from(first).unwrap_or(i64::MAX));
        v_flex()
            .id("reword-form")
            .occlude()
            .w(px(FORM_WIDTH))
            .h(px(FORM_HEIGHT - 5.))
            .p(px(10.))
            .gap(px(6.))
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .text_sm()
            .child(
                h_flex()
                    .child(div().flex_1().child(say!("rebase-new-message")))
                    .child(
                        div()
                            .text_color(if left < 0 {
                                theme.danger
                            } else {
                                theme.muted_foreground
                            })
                            .child(left.to_string()),
                    ),
            )
            .child(Textarea::new(&self.message).h(px(132.)))
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("reword-cancel")
                            .danger()
                            .outline()
                            .xsmall()
                            .label(say!("rebase-form-cancel"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.close_form(false, window, cx)
                            })),
                    )
                    .child(
                        Button::new("reword-update")
                            .primary()
                            .xsmall()
                            .label(say!("rebase-update-message"))
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.close_form(true, window, cx)
                                }),
                            ),
                    ),
            )
            .test_support()
            .into_any_element()
    }

    /// What there will be when the plan is carried out, newest first: a
    /// commit a line, and what it is made of when that is not one commit.
    fn preview(&self) -> String {
        let result = match &self.outline {
            Ok(result) => result,
            Err(err) => return err.clone(),
        };
        if result.is_empty() {
            return say!("rebase-preview-nothing").to_owned();
        }
        let mut lines = vec![say!("rebase-preview").to_owned()];
        for commit in result.iter().rev().take(PREVIEWED) {
            let summary = commit.message.lines().next().unwrap_or("");
            lines.push(match (commit.from.len(), commit.stops) {
                (1, false) => summary.to_owned(),
                (1, true) => say!("rebase-preview-stops", summary),
                (count, false) => say!("rebase-preview-from", summary, count),
                (count, true) => say!("rebase-preview-from-stops", summary, count),
            });
        }
        let more = result.len().saturating_sub(PREVIEWED);
        if more > 0 {
            lines.push(say!("rebase-preview-more", more));
        }
        lines.join("\n")
    }
}

impl Focusable for RebaseEditor {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RebaseEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let note = |text: String, colour: Hsla| {
            div()
                .p_3()
                .text_sm()
                .text_color(colour)
                .child(text)
                .into_any_element()
        };
        // What is rebased, onto what when it moves, and how much it is.
        let nothing = || (String::new(), None, String::new());
        let (about, body): ((String, Option<String>, String), AnyElement) = match &self.state {
            State::Empty => (nothing(), note(String::new(), theme.muted_foreground)),
            State::Loading => (
                nothing(),
                note(say!("rebase-loading").to_owned(), theme.muted_foreground),
            ),
            State::Refused(why) => (nothing(), note(sentence(why), theme.danger)),
            State::Open(range) => {
                let published = self.model.published_rewritten(range);
                let alone = self.alone;
                // Under the list, at the left: what each key does to the
                // selected commit, under a caption, the key's letter of
                // each word stressed. For a commit by itself there is
                // nothing to choose, and why is said.
                let word = |key: &str, rest: &str| {
                    h_flex()
                        .flex_none()
                        .child(div().font_semibold().child(key.to_owned()))
                        .child(
                            div()
                                .text_color(theme.muted_foreground)
                                .child(rest.to_owned()),
                        )
                };
                let keys =
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .text_xs()
                        .when(alone.is_none(), |keys| {
                            keys.child(
                                h_flex()
                                    .gap_1()
                                    .items_center()
                                    .text_color(theme.muted_foreground)
                                    .child(Icon::new(gpui_kit::assets::IconName::Keyboard).size_3())
                                    .child(say!("rebase-shortcuts")),
                            )
                            .child(
                                h_flex()
                                    .flex_wrap()
                                    .gap_x(px(8.))
                                    .children(KEYS_SHOWN.iter().filter_map(|action| {
                                        let (_, key, name, _) = ACTIONS
                                            .iter()
                                            .find(|(candidate, ..)| candidate == action)?;
                                        let rest = name.text().get(1..).unwrap_or_default();
                                        Some(word(&key.to_uppercase(), rest))
                                    }))
                                    .child(word(
                                        &move_key("up"),
                                        &format!(" {}", say!("rebase-move-up")),
                                    ))
                                    .child(word(
                                        &move_key("down"),
                                        &format!(" {}", say!("rebase-move-down")),
                                    )),
                            )
                        });
                let list = v_flex()
                    .id("plan-list")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .pt(px(MARGIN))
                    .child(
                        v_virtual_list(
                            cx.entity().clone(),
                            "plan-rows",
                            self.sizes.get(),
                            |this, range, _, cx| {
                                range
                                    .filter_map(|shown| this.render_row(shown, cx))
                                    .collect::<Vec<_>>()
                            },
                        )
                        .track_scroll(&self.scroll),
                    )
                    .scrollbar(&self.scroll, ScrollbarAxis::Vertical);
                let warning = (published > 0).then(|| {
                    let upstream = range.upstream.clone().unwrap_or_default();
                    div()
                        .w_full()
                        .px_2()
                        .py_1()
                        .border_b_1()
                        .border_color(theme.border)
                        .bg(theme.yellow.opacity(0.12))
                        .text_xs()
                        .child(say!(
                            "rebase-published-warning",
                            count = published,
                            upstream,
                            branch = &range.branch
                        ))
                });
                // Over the list, for a commit by itself: why there is
                // nothing to plan, and what stays as it is.
                let only = alone.map(|why| {
                    div()
                        .w_full()
                        .px_2()
                        .py_1()
                        .border_b_1()
                        .border_color(theme.border)
                        .bg(theme.list_head)
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(why)
                });
                let count = say!("rebase-commits", count = self.model.len());
                let changed = self.changed();
                let valid = self.outline.is_ok();
                let foot = h_flex()
                    .w_full()
                    .flex_none()
                    .px(px(MARGIN))
                    .py_3()
                    .gap(px(MARGIN))
                    .items_center()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(keys)
                    .child(
                        Button::new("plan-reset")
                            .outline()
                            .h(px(BUTTON_HEIGHT))
                            .px_3()
                            .label(say!("rebase-reset"))
                            .tooltip(say!("rebase-reset-tooltip"))
                            .disabled(!changed)
                            .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx))),
                    )
                    .child(
                        Button::new("plan-cancel-foot")
                            .danger()
                            .outline()
                            .h(px(BUTTON_HEIGHT))
                            .px_3()
                            .label(if alone.is_some() {
                                say!("rebase-cancel")
                            } else {
                                say!("rebase-cancel-rebase")
                            })
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RebaseEvent::Cancelled))),
                    )
                    .child(
                        Button::new("plan-start")
                            .primary()
                            .h(px(BUTTON_HEIGHT))
                            .px_3()
                            .label(if alone.is_some() {
                                say!("rebase-reword")
                            } else if self.skips_hooks() {
                                say!("rebase-start-without-hooks")
                            } else {
                                say!("rebase-start")
                            })
                            .disabled(!self.runnable() || !valid)
                            .tooltip(self.preview())
                            .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                    );
                // Offered where there is a hook that could refuse the
                // rebase, in a line of its own over the buttons (the keys
                // beside them need the width); a commit that is only
                // reworded asks none.
                let switch = (alone.is_none() && !self.refusing_hooks.is_empty()).then(|| {
                    h_flex()
                        .w_full()
                        .flex_none()
                        .px(px(MARGIN))
                        .py_2()
                        .justify_end()
                        .child(
                            Checkbox::new("plan-skip-hooks")
                                .label(crate::views::workspace::hooks::switch_label(
                                    &self.refusing_hooks,
                                ))
                                .checked(self.skip_hooks)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.skip_hooks = *checked;
                                    cx.notify();
                                })),
                        )
                });
                (
                    (range.branch.clone(), self.onto.clone(), count),
                    v_flex()
                        .size_full()
                        .children(only)
                        .children(warning)
                        .child(v_flex().flex_1().min_h_0().w_full().child(list))
                        // What is wrong with the plan, when something is.
                        .children(self.outline.as_ref().err().map(|err| {
                            div()
                                .w_full()
                                .px(px(MARGIN))
                                .py_1()
                                .text_xs()
                                .text_color(theme.danger)
                                .child(err.clone())
                        }))
                        .children(switch)
                        .child(foot)
                        .into_any_element(),
                )
            }
        };
        let (branch, onto, count) = about;
        let chip = |text: String| {
            div()
                .flex_none()
                .max_w(px(260.))
                .truncate()
                .px_1p5()
                .rounded(theme.radius)
                .bg(theme.primary.opacity(0.25))
                .text_sm()
                .child(text)
        };
        let form = self.rewording.as_ref().map(|(row, ..)| {
            let list = self.scroll.bounds();
            let shown = self.shown_index(*row);
            let down = px(ROW_HEIGHT * shown.saturating_add(1) as f32) + self.scroll.offset().y;
            let at = point(list.origin.x + px(FORM_LEFT), list.origin.y + down);
            deferred(
                anchored()
                    .position(at)
                    .snap_to_window_with_margin(px(8.))
                    .child(self.render_form(cx)),
            )
            .with_priority(1)
        });
        v_flex()
            .id("rebase-editor")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
            .on_action(cx.listener(|this, choose: &Choose, window, cx| {
                let Some((action, ..)) = ACTIONS.get(choose.action) else {
                    return;
                };
                // A choice for a row that is no longer there (the plan
                // was loaded anew under the menu) is for no commit.
                if choose.row >= this.model.len() {
                    return;
                }
                this.select(choose.row, window, cx);
                this.act(*action, window, cx);
            }))
            .size_full()
            .bg(theme.background)
            // The title, what is rebased, and the way out at the far end.
            .child(
                h_flex()
                    .w_full()
                    .h(px(TITLE_HEIGHT))
                    .flex_none()
                    .px(px(MARGIN))
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_none().pr_3().text_sm().font_semibold().child(
                        if self.alone.is_some() {
                            say!("rebase-title-reword")
                        } else {
                            say!("rebase-title")
                        },
                    ))
                    .when(!branch.is_empty(), |header| {
                        header
                            .child(
                                div()
                                    .flex_none()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(if self.alone.is_some() {
                                        say!("rebase-on")
                                    } else {
                                        say!("rebase-rebasing")
                                    }),
                            )
                            .child(chip(branch))
                    })
                    .when_some(onto, |header, onto| {
                        header
                            .child(
                                div()
                                    .flex_none()
                                    .text_sm()
                                    .text_color(theme.muted_foreground)
                                    .child(say!("rebase-onto")),
                            )
                            .child(chip(onto))
                    })
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(count),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("plan-cancel")
                            .danger()
                            .outline()
                            .xsmall()
                            .label(if self.alone.is_some() {
                                say!("rebase-cancel")
                            } else {
                                say!("rebase-cancel-rebase")
                            })
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RebaseEvent::Cancelled))),
                    ),
            )
            .child(div().flex_1().min_h_0().child(body))
            // The form of the row that is reworded, under that row and over
            // the rows that follow it.
            .children(form)
    }
}

/// The key that moves a line, as shown: Ctrl and an arrow, or on macOS
/// Option and an arrow (Ctrl+↑ is the system's Mission Control there).
/// Both are taken everywhere.
fn move_key(arrow: &str) -> String {
    crate::keymap::display_key(&match crate::keymap::System::now() {
        crate::keymap::System::Mac => format!("alt-{arrow}"),
        crate::keymap::System::Other => format!("ctrl-{arrow}"),
    })
}

#[cfg(test)]
mod tests {
    use super::{PlanModel, alone_of, why_alone};
    use bitbraid_git::{
        Oid,
        interactive::{Action, Entry, Range, outline, untouched},
    };

    fn id(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    fn range(published: usize) -> Range {
        Range {
            base: id(0),
            branch: "main".to_owned(),
            upstream: Some("origin/main".to_owned()),
            commits: (1u8..=4)
                .map(|n| Entry {
                    id: id(n),
                    summary: format!("c{n}"),
                    message: format!("c{n}\n\nbody"),
                    author: "Ada".to_owned(),
                    email: "ada@example.invalid".to_owned(),
                    time: 0,
                    published: usize::from(n) <= published,
                })
                .collect(),
        }
    }

    fn order(model: &PlanModel) -> Vec<Oid> {
        model.steps().iter().map(|step| step.id).collect()
    }

    #[test]
    fn a_fresh_plan_changes_nothing() {
        let range = range(0);
        let model = PlanModel::new(&range);
        assert_eq!(model.len(), 4);
        assert!(!model.is_empty());
        assert_eq!(model.steps(), untouched(&range));
        assert_eq!(model.published_rewritten(&range), 0);
        assert!(PlanModel::default().is_empty());
    }

    #[test]
    fn rows_move_and_stay_inside_the_list() {
        let range = range(0);
        let mut model = PlanModel::new(&range);
        assert_eq!(model.shift(0, true), 1);
        assert_eq!(order(&model), [id(2), id(1), id(3), id(4)]);
        assert_eq!(model.shift(1, false), 0);
        assert_eq!(order(&model), [id(1), id(2), id(3), id(4)]);
        // At the ends there is nowhere to go.
        assert_eq!(model.shift(0, false), 0);
        assert_eq!(model.shift(3, true), 3);
        assert_eq!(order(&model), [id(1), id(2), id(3), id(4)]);
        // Dropped somewhere else, and past the end.
        assert_eq!(model.place(0, 2), 2);
        assert_eq!(order(&model), [id(2), id(3), id(1), id(4)]);
        assert_eq!(model.place(1, 99), 3);
        assert_eq!(order(&model), [id(2), id(1), id(4), id(3)]);
        // A row that is not there moves nothing.
        assert_eq!(model.place(9, 0), 9);
        assert_eq!(model.shift(usize::MAX, true), usize::MAX);
        assert_eq!(order(&model), [id(2), id(1), id(4), id(3)]);
        // Every commit is still there once: the engine takes the plan.
        assert!(outline(&range, &model.steps()).is_ok());
    }

    #[test]
    fn a_reword_starts_from_the_message_there_is() {
        let range = range(0);
        let mut model = PlanModel::new(&range);
        model.set_action(1, Action::Reword);
        assert_eq!(model.steps()[1].message.as_deref(), Some("c2\n\nbody"));
        model.set_message(1, "better".to_owned());
        assert_eq!(model.steps()[1].message.as_deref(), Some("better"));
        // Keeps what was typed when reword is chosen again.
        model.set_action(1, Action::Reword);
        assert_eq!(model.steps()[1].message.as_deref(), Some("better"));
        // Another action forgets it, and a message is only taken for a
        // reword.
        model.set_action(1, Action::Squash);
        assert_eq!(model.steps()[1].message, None);
        model.set_message(1, "ignored".to_owned());
        model.set_message(9, "nowhere".to_owned());
        model.set_action(9, Action::Drop);
        assert_eq!(model.steps()[1].message, None);
        let result = outline(&range, &model.steps()).unwrap();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].message, "c1\n\nbody\n\nc2\n\nbody");
    }

    #[test]
    fn only_what_comes_after_the_first_change_is_made_anew() {
        let range = range(3);
        let mut model = PlanModel::new(&range);
        // The newest is not pushed: nothing pushed is touched.
        model.set_action(3, Action::Drop);
        assert_eq!(model.published_rewritten(&range), 0);
        // From the second on: two pushed commits are rewritten.
        model.set_action(1, Action::Edit);
        assert_eq!(model.published_rewritten(&range), 2);
        // Moving the oldest touches all three.
        let mut model = PlanModel::new(&range);
        model.shift(0, true);
        assert_eq!(model.published_rewritten(&range), 3);
    }
    #[test]
    fn a_commit_that_cannot_be_planned_from_is_shown_by_itself() {
        let entry = serde_json::json!({
            "id": "11".repeat(20), "summary": "one", "message": "one\n\nbody",
            "author": "Ada", "time": 7, "published": true,
        });
        let alone = serde_json::json!({
            "base": "22".repeat(20), "branch": "main", "upstream": "origin/main",
            "commits": [entry],
        });
        let way = |plain: bool, merge: bool, first: bool| serde_json::json!({ "plain": plain, "merge": merge, "first": first, "alone": alone });
        // A plain list leads up to HEAD: the plan is made as ever.
        assert!(alone_of(&way(true, false, false)).is_none());
        let (range, why) = alone_of(&way(false, false, false)).unwrap();
        assert_eq!(range.commits.len(), 1);
        assert_eq!(range.commits[0].message, "one\n\nbody");
        assert!(range.commits[0].published);
        assert_eq!(range.upstream.as_deref(), Some("origin/main"));
        assert!(why.starts_with("A merge commit comes after this one"));
        // Each reason in its own words, and each says what stays.
        let reasons = [
            why_alone(true, false),
            why_alone(false, true),
            why_alone(false, false),
        ];
        assert!(reasons[0].contains("is a merge commit"));
        assert!(reasons[1].contains("first commit of the history"));
        assert!(reasons.iter().all(|why| why.contains("stay as they are")));
        assert_eq!(alone_of(&way(false, true, false)).unwrap().1, reasons[0]);
        assert_eq!(alone_of(&way(false, false, true)).unwrap().1, reasons[1]);
        // An answer that is not one: nothing is made of it.
        for bad in [
            serde_json::json!({}),
            serde_json::json!({ "plain": false }),
            serde_json::json!({ "plain": false, "alone": { "commits": [] } }),
            serde_json::json!({ "plain": "no", "alone": alone }),
            serde_json::json!(7),
        ] {
            assert!(alone_of(&bad).is_none(), "{bad}");
        }
        let mut two = alone.clone();
        two["commits"] = serde_json::json!([entry, entry]);
        assert!(alone_of(&serde_json::json!({ "plain": false, "alone": two })).is_none());
    }
}
