//! The commit box: summary and description with a count of how wide they
//! are, amend, the messages of commits made here lately to take one back
//! from, trailers added by git's rules, and commit.
//!
//! On a branch whose name holds an issue (`12-fix-quotes`, read by
//! `issue.key_from_branch`), an empty description starts as the trailer
//! `Refs: #12`, unless the preferences say not to: a trailer rather than a
//! prefix of the summary, so that the summary stays the change's own and
//! its fifty characters are not spent, and `Refs` rather than `Fixes`, so
//! that a commit never closes the issue by surprise; both providers draw
//! `#12` as a link wherever it stands.
//!
//! Sources: git-commit(1) ("DISCUSSION" for the 50 characters of a
//! summary), git-interpret-trailers(1); docs.github.com "Autolinked
//! references and URLs" and "Linking a pull request to an issue" (closing
//! words); docs.gitlab.com "Crosslinking issues" and "Closing issues
//! automatically".

use std::{cell::Cell, rc::Rc};

use bitbraid_git::{Person, message::RecentMessage};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenu},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde::Deserialize;
use serde_json::json;

use crate::session::{Reply, Request, SessionHandle};

/// Events from the commit box.
#[derive(Debug, Clone)]
pub enum CommitEvent {
    /// A commit or amend succeeded; with `push` the branch is to be pushed
    /// next.
    Committed {
        /// Push the branch now.
        push: bool,
        /// The hooks that are there and were left out because the commit
        /// was made without them.
        skipped_hooks: Vec<String>,
    },
    /// A trailer is to be typed: ask for its value, and for its key when
    /// there is none yet, and hand both to [`CommitBox::add_trailer`].
    AskTrailer {
        /// The key that was chosen; empty when it is to be typed too.
        key: String,
    },
    /// What was staged was stashed from the stash tab.
    Stashed,
    /// Go on with the operation that stopped.
    Continue,
    /// Leave out the commit the operation stopped at.
    Skip,
    /// Give the operation that stopped up.
    Abort,
}

/// An operation that stopped part way (a merge, a rebase, a pick): while
/// there is one, the form ends in the two ways out of it, side by side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The word for the operation in a sentence.
    pub noun: String,
    /// How many files are still in conflict; it goes on when none is.
    pub conflicts: usize,
    /// Whether the commit it stopped at can be left out.
    pub can_skip: bool,
}

/// What the button that goes on says, and whether it commits what the
/// form holds rather than what the operation prepared: a merge with a
/// summary typed is concluded under that message.
pub fn going_on(stopped: &Stopped, summary_typed: bool) -> (String, bool) {
    if stopped.noun == "merge" && summary_typed {
        (say!("commit-merge").to_owned(), true)
    } else {
        (say!("commit-continue", noun = stopped.noun.as_str()), false)
    }
}

/// What the button that commits says, and whether it can be pressed. It
/// waits for a summary, then for something staged, and then says how many
/// files it commits. Amending needs nothing staged: the message alone can
/// be what changes.
pub fn commit_button(
    summary_typed: bool,
    busy: bool,
    amend: bool,
    staged: usize,
    skipping: bool,
) -> (String, bool) {
    if !summary_typed {
        return (say!("commit-type-summary").to_owned(), false);
    }
    if amend {
        let label = if skipping {
            say!("commit-amend-without-hooks")
        } else {
            say!("commit-amend")
        };
        return (label.to_owned(), !busy);
    }
    match staged {
        0 => (say!("commit-stage-a-file").to_owned(), false),
        count if skipping => (say!("commit-files-without-hooks", count), !busy),
        count => (say!("commit-files", count), !busy),
    }
}

/// The tabs over the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormTab {
    /// The commit's form.
    Commit,
    /// A title and a description for a stash of what is staged.
    Stash,
}

/// The least height of the message's frame on the commit tab, and on the
/// stash tab, borders included.
const FRAME_HEIGHT: f32 = 138.;
const STASH_FRAME: f32 = 129.;
/// How low the frame can be made by a window that has no more room: the
/// summary and two lines of the description.
const FRAME_LEAST: f32 = 80.;

/// What the message's frame gives up of its height so that a form that
/// would be `natural` high, with the frame as tall as it wants
/// (`wanted`), fits in `room`: nothing while it fits, and never more than
/// leaves the frame its least. A room or a height that is no number takes
/// nothing.
fn squeezed(natural: f32, room: f32, wanted: f32) -> f32 {
    let over = natural - room;
    if over.is_finite() {
        over.clamp(0., (wanted - FRAME_LEAST).max(0.))
    } else {
        0.
    }
}

/// The grip over the form, while it is dragged.
#[derive(Debug, Clone, Copy)]
struct FormGrip;

/// Nothing follows the pointer while the grip is dragged.
struct GripChip;

impl Render for GripChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// What the button of the stash tab says, and whether it can be pressed:
/// it stashes what is staged, and waits for something to be.
pub fn stash_button(staged: usize, busy: bool) -> (String, bool) {
    match staged {
        0 => (say!("commit-stash-stage-a-file").to_owned(), false),
        count => (say!("commit-stash-files", count), !busy),
    }
}

/// A summary longer than this is long: git's own advice.
pub const SUMMARY_SOFT: usize = 50;
/// A summary longer than this is cut off where commits are listed.
pub const SUMMARY_HARD: usize = 72;
/// A line of the description longer than this wraps in a terminal.
pub const BODY_WIDTH: usize = 72;
/// How many messages the history offers.
const HISTORY: usize = 15;
/// How many trailer keys the menu offers.
const KEYS: usize = 10;

/// Commit in one of the ways the menu beside the button offers.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = commit_box, no_json)]
struct CommitWith {
    /// Stage everything first.
    stage_all: bool,
    /// Push afterwards.
    push: bool,
}

/// Add a trailer. Without a value it is asked for; `Signed-off-by` without
/// one signs off as whoever git commits as.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = commit_box, no_json)]
struct AddTrailer {
    key: String,
    value: String,
}

/// Put a message of the history into the box; the index past the last one
/// stands for what the box held before one was taken.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = commit_box, no_json)]
struct TakeMessage {
    index: usize,
}

pub struct CommitBox {
    session: SessionHandle,
    summary: Entity<InputState>,
    description: Entity<TextareaState>,
    summary_text: String,
    /// The description as last typed, for the count of its widest line.
    description_text: String,
    /// The messages of the commits made here lately.
    recent: Vec<RecentMessage>,
    /// What the box held when a message was taken from the history.
    draft: Option<(String, String)>,
    /// The trailer keys this repository uses.
    trailer_keys: Vec<String>,
    amend: bool,
    /// What the box held when amending was ticked: given back when it is
    /// unticked.
    before_amend: Option<(String, String)>,
    /// Which tab of the form is on show.
    tab: FormTab,
    /// Whether the row of options under the message is unfolded.
    options_open: bool,
    /// Push the branch once the commit is made.
    push_after: bool,
    /// How much taller than its least the message's frame was dragged,
    /// and how much taller it was drawn the last time (the lists may have
    /// left it less).
    extra: f32,
    frame_drawn: Rc<Cell<f32>>,
    /// The room the form has, as the window says it: how high its top
    /// may go (the lists keep their least above it) and where the panel
    /// ends; and how tall it was drawn the last time.
    room: Rc<Cell<(f32, f32)>>,
    height_drawn: Rc<Cell<f32>>,
    /// What the message's frame gave up of its height when it was last
    /// drawn, in a panel too low for the whole form (`squeezed`).
    taken: Rc<Cell<f32>>,
    /// How high the form's top goes with the lists over it at their very
    /// least, a row each, as the window says it: what it has before its
    /// message gives way.
    tightest: Rc<Cell<f32>>,
    /// How far under the grip's top the pointer took hold of it.
    grab: Rc<Cell<f32>>,
    /// Whether the repository signs commits unless told otherwise, whether
    /// a key is set at all, and what the user chose for the next commit
    /// when that is not what the repository does by itself.
    sign_default: bool,
    sign_key: bool,
    sign: Option<bool>,
    /// The hooks that can refuse a commit and are there: what committing
    /// without hooks leaves out.
    hooks: Vec<String>,
    /// Every hook that is there, by name: what the switches of the other
    /// operations that can go without theirs are about.
    present: Vec<String>,
    /// Commit without them. Kept while the tab is open, said on the button
    /// and under it for as long as it is on, and never written anywhere: a
    /// new start runs the hooks again.
    skip_hooks: bool,
    /// How many files are staged: what the button says it commits, and
    /// without any there is nothing for it to commit.
    staged: usize,
    busy: bool,
    /// The operation that stopped part way, when there is one.
    stopped: Option<Stopped>,
    error: Option<String>,
    people: Vec<Person>,
    pending_set: Option<(String, String)>,
    pending_clear: bool,
    /// The branch checked out, and the issue its name holds once
    /// `issue.key_from_branch` answered.
    issue_branch: Option<String>,
    issue_number: Option<u64>,
    /// The trailer naming that issue is to be put into an empty
    /// description when the box is drawn next.
    issue_trailer_due: bool,
    /// Whether the preferences wanted that trailer when they were last
    /// looked at.
    trailer_wanted: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<CommitEvent> for CommitBox {}

impl CommitBox {
    /// Create the view and ask for the template, the people, the recent
    /// messages, how the repository signs and its hooks.
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let summary = cx.new(|cx| InputState::new(window, cx).placeholder(say!("commit-summary")));
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("commit-description"))
                // Tab goes on to the next control: a text that grows,
                // unlike one of fixed rows, is not indented.
                .auto_grow(4, 4)
        });
        let subscription = cx.subscribe_in(
            &summary,
            window,
            |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.summary_text = state.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );
        let described = cx.subscribe_in(
            &description,
            window,
            |this, state, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.description_text = state.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );
        let mut this = Self {
            session,
            summary,
            description,
            summary_text: String::new(),
            description_text: String::new(),
            recent: Vec::new(),
            draft: None,
            trailer_keys: Vec::new(),
            amend: false,
            before_amend: None,
            tab: FormTab::Commit,
            options_open: false,
            push_after: false,
            extra: 0.,
            frame_drawn: Rc::new(Cell::new(0.)),
            room: Rc::new(Cell::new((f32::NEG_INFINITY, f32::INFINITY))),
            height_drawn: Rc::new(Cell::new(0.)),
            taken: Rc::new(Cell::new(0.)),
            tightest: Rc::new(Cell::new(f32::NEG_INFINITY)),
            grab: Rc::new(Cell::new(0.)),
            sign_default: false,
            sign_key: false,
            sign: None,
            hooks: Vec::new(),
            present: Vec::new(),
            skip_hooks: false,
            staged: 0,
            busy: false,
            stopped: None,
            error: None,
            people: Vec::new(),
            pending_set: None,
            pending_clear: false,
            issue_branch: None,
            issue_number: None,
            issue_trailer_due: false,
            trailer_wanted: cx.global::<crate::prefs::Prefs>().preferences.issue_trailer,
            focus_handle: cx.focus_handle(),
            _subscriptions: vec![subscription, described],
        };
        // The preference for the trailer is followed while the box is open.
        let followed = cx.observe_global_in::<crate::prefs::Prefs>(window, |this, window, cx| {
            this.follow_trailer_preference(window, cx)
        });
        this._subscriptions.push(followed);
        this.load_template(cx);
        this.load_people(cx);
        this.load_history(cx);
        this.load_signing(cx);
        this.load_hooks(cx);
        this
    }

    /// The field of the summary: what is typed in it shows in the graph's
    /// row of uncommitted work as well.
    pub fn summary_field(&self) -> Entity<InputState> {
        self.summary.clone()
    }

    /// Put the cursor in the summary (from the field in the graph's row of
    /// uncommitted work).
    pub fn focus_summary(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.summary.update(cx, |state, cx| state.focus(window, cx));
    }

    /// The hooks that are there, as last asked: every one, by name.
    pub fn hooks_present(&self) -> &[String] {
        &self.present
    }

    /// What the stash tab's button says and whether it can be pressed, for
    /// the off-screen tests.
    pub fn stash_button(&self) -> (String, bool) {
        stash_button(self.staged, self.busy)
    }

    /// The room the form has, for the window to say: how high its top
    /// may go, the lists over it keeping their least, and where the panel
    /// ends.
    pub fn room(&self) -> Rc<Cell<(f32, f32)>> {
        self.room.clone()
    }

    /// How high the form's top goes with the lists at a row each, for
    /// the window to say: the message gives way only below it.
    pub fn tightest(&self) -> Rc<Cell<f32>> {
        self.tightest.clone()
    }

    /// The summary as it stands, for the off-screen tests.
    pub fn summary(&self) -> &str {
        &self.summary_text
    }

    /// The description as it was last typed or put there.
    pub fn description(&self) -> &str {
        &self.description_text
    }

    /// The tab that is on show.
    pub fn tab(&self) -> FormTab {
        self.tab
    }

    /// Whether the stash tab is offered: not while a rebase is stopped,
    /// which replays commits and has nothing of its own to set aside.
    fn stash_offered(&self) -> bool {
        self.stopped
            .as_ref()
            .is_none_or(|stopped| stopped.noun != "rebase")
    }

    /// Stash what is staged, under the title and the description.
    fn stash(&mut self, cx: &mut Context<Self>) {
        if self.busy || self.staged == 0 {
            return;
        }
        let title = self.summary_text.trim();
        let description = self.description.read(cx).value().trim().to_owned();
        let message = if description.is_empty() {
            title.to_owned()
        } else {
            format!("{title}\n\n{description}")
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        let request = Request::Command {
            id: "stash.push".into(),
            params: json!({ "message": message.trim(), "staged_only": true }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.busy = false;
            match reply {
                Reply::Error(err) => this.error = Some(err),
                _ => {
                    this.pending_clear = true;
                    this.before_amend = None;
                    cx.emit(CommitEvent::Stashed);
                }
            }
            cx.notify();
        });
    }

    /// The row of tabs over the form: the commit's and the stash's. The
    /// one on show has its word and an open bottom; the other is an icon
    /// on the rule that runs to the panel's edge.
    fn render_tabs(&self, stash_tab: bool, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tab = |id: &'static str, icon: Icon, word: &'static str, on: bool, which: FormTab| {
            h_flex()
                .id(id)
                .h_full()
                .gap(px(5.))
                .px(px(10.))
                .items_center()
                .justify_center()
                .text_sm()
                .cursor_pointer()
                .when(on, |tab| {
                    tab.min_w(px(96.))
                        .border_1()
                        .border_b_0()
                        .border_color(theme.border)
                        .rounded_t(theme.radius)
                })
                .when(!on, |tab| {
                    tab.border_b_1()
                        .border_color(theme.border)
                        .text_color(theme.muted_foreground)
                })
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(word).build(window, cx)
                })
                .test_support()
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.tab = which;
                    cx.notify();
                }))
                .child(icon.size_4())
                .when(on, |tab| tab.child(word))
        };
        h_flex()
            .w_full()
            .h(px(31.))
            .pl(px(10.))
            .items_end()
            .child(tab(
                "tab-commit",
                Icon::new(gpui_kit::assets::IconName::GitCommitHorizontal),
                say!("commit-tab-commit"),
                !stash_tab,
                FormTab::Commit,
            ))
            .when(self.stash_offered(), |row| {
                row.child(tab(
                    "tab-stash",
                    Icon::new(gpui_kit::assets::IconName::Archive),
                    say!("commit-tab-stash"),
                    stash_tab,
                    FormTab::Stash,
                ))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .border_b_1()
                    .border_color(theme.border),
            )
    }

    /// How many files are staged now.
    pub fn set_staged(&mut self, staged: usize, cx: &mut Context<Self>) {
        if self.staged != staged {
            self.staged = staged;
            cx.notify();
        }
    }

    /// What the button says and whether it can be pressed, for the
    /// off-screen tests.
    pub fn button(&self) -> (String, bool) {
        commit_button(
            !self.summary_text.trim().is_empty(),
            self.busy,
            self.amend,
            self.staged,
            self.skip_hooks && !self.hooks.is_empty(),
        )
    }

    /// Ask which of the hooks that can refuse a commit are there. Without
    /// one there is nothing to leave out, and the switch is not shown.
    pub fn load_hooks(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "hook.list".into(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                this.hooks = refusing_hooks(&value);
                this.present = hook_names(&value);
                if this.hooks.is_empty() {
                    this.skip_hooks = false;
                }
                cx.notify();
            }
        });
    }

    /// Ask how the repository signs: whether commits are signed by
    /// default, and whether there is a key to sign with.
    pub fn load_signing(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "sign.status".into(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                this.sign_default =
                    value.get("commits").and_then(|flag| flag.as_bool()) == Some(true);
                this.sign_key = value.get("key").is_some_and(|key| key.is_string());
                cx.notify();
            }
        });
    }

    fn load_template(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "commit.template".into(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply
                && let Some(template) = value.get("template").and_then(|t| t.as_str())
                && this.summary_text.is_empty()
            {
                // Template lines starting with '#' are comments git strips; keep the rest.
                let body: String = template
                    .lines()
                    .filter(|l| !l.starts_with('#'))
                    .collect::<Vec<_>>()
                    .join("\n");
                this.pending_set = Some((String::new(), body.trim().to_owned()));
                cx.notify();
            }
        });
    }

    /// Ask for the messages of the commits made here lately and for the
    /// trailer keys in use (when the box is made, and after commits).
    pub fn load_history(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "commit.messages".into(),
            params: json!({ "limit": HISTORY }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply
                && let Ok(recent) = serde_json::from_value::<Vec<RecentMessage>>(value)
            {
                this.recent = recent;
                cx.notify();
            }
        });
        let request = Request::Command {
            id: "commit.trailer_keys".into(),
            params: json!({ "limit": 300 }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                this.trailer_keys = offered_keys(&value);
                cx.notify();
            }
        });
    }

    /// Reload the co-author candidates (after commits).
    pub fn load_people(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "commit.people".into(),
            params: json!({ "limit": 200 }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply
                && let Ok(people) = serde_json::from_value::<Vec<Person>>(value)
            {
                this.people = people;
                cx.notify();
            }
        });
    }

    /// Tick or untick amending. Ticked, the last commit's message takes
    /// the box's place; unticked, what the box held before comes back.
    fn set_amend(&mut self, amend: bool, cx: &mut Context<Self>) {
        self.amend = amend;
        cx.notify();
        if !amend {
            if let Some(before) = self.before_amend.take() {
                self.pending_set = Some(before);
            }
            return;
        }
        self.before_amend = Some((self.summary_text.clone(), self.description_text.clone()));
        {
            // Prefill from the tip commit's message.
            let request = Request::Command {
                id: "repo.summary".into(),
                params: json!({}),
            };
            self.session.call(request, cx, |this, reply, cx| {
                let head = match reply {
                    Reply::Command(value) => value
                        .get("head")
                        .and_then(|h| h.as_str())
                        .and_then(|h| h.parse().ok()),
                    _ => None,
                };
                let Some(head) = head else { return };
                this.session
                    .call(Request::Message(head), cx, |this, reply, cx| {
                        if let Reply::Message(message) = reply {
                            let mut parts = message.splitn(2, "\n\n");
                            let summary = parts.next().unwrap_or_default().trim().to_owned();
                            let body = parts.next().unwrap_or_default().trim().to_owned();
                            this.pending_set = Some((summary, body));
                            cx.notify();
                        }
                    });
            });
        }
    }

    /// Commit with the message that is in the box (from a key or the
    /// palette). Without a summary the cursor goes into its field instead.
    pub fn commit_now(&mut self, stage_all: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.summary_text.trim().is_empty() {
            self.summary.update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        self.commit(stage_all, false, cx);
    }

    fn commit(&mut self, stage_all: bool, push: bool, cx: &mut Context<Self>) {
        let summary = self.summary_text.trim().to_owned();
        if summary.is_empty() || self.busy {
            return;
        }
        let description = self.description.read(cx).value().trim().to_owned();
        let message = if description.is_empty() {
            summary
        } else {
            format!("{summary}\n\n{description}")
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        let id = if self.amend {
            "commit.amend"
        } else {
            "commit.create"
        };
        // Signing is said only when it is not what the repository does by
        // itself.
        let sign = self.sign.filter(|sign| *sign != self.sign_default);
        let request = Request::Command {
            id: id.into(),
            params: json!({
                "message": message,
                "keep_author": true,
                "stage_all": stage_all,
                "sign": sign,
                "no_verify": self.skip_hooks,
            }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            this.busy = false;
            match reply {
                Reply::Error(err) => this.error = Some(err),
                reply => {
                    let skipped_hooks = match &reply {
                        Reply::Command(value) => texts(value.get("skipped_hooks")),
                        _ => Vec::new(),
                    };
                    this.pending_clear = true;
                    this.issue_trailer_due = true;
                    this.amend = false;
                    this.before_amend = None;
                    cx.emit(CommitEvent::Committed {
                        push,
                        skipped_hooks,
                    });
                    this.draft = None;
                    this.load_people(cx);
                    this.load_history(cx);
                    // A hook may have been added or taken away meanwhile.
                    this.load_hooks(cx);
                }
            }
            cx.notify();
        });
    }

    /// The branch checked out changed: ask which issue its name holds, for
    /// the trailer that names it.
    pub fn set_issue_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        if branch == self.issue_branch {
            return;
        }
        self.issue_branch = branch.clone();
        self.issue_number = None;
        let Some(branch) = branch else {
            return;
        };
        let reply =
            crate::views::pulls::calls::run("issue.key_from_branch", json!({ "branch": branch }));
        SessionHandle::deliver(
            reply,
            "issue.key_from_branch".to_owned(),
            cx,
            move |this, reply, cx| {
                if this.issue_branch.as_deref() != Some(branch.as_str()) {
                    return;
                }
                this.issue_number = match reply {
                    Reply::Command(value) => value.get("number").and_then(|number| number.as_u64()),
                    _ => None,
                };
                this.issue_trailer_due = this.issue_number.is_some();
                cx.notify();
            },
        );
    }

    /// The issue the checked-out branch's name holds, once known.
    pub fn issue_number(&self) -> Option<u64> {
        self.issue_number
    }

    /// The preference that names the branch's issue in the description
    /// was turned on or off while the box is open: on, an empty
    /// description takes the trailer now; off, a description that is that
    /// trailer alone is emptied again, without the tab having to be left
    /// and shown again first.
    fn follow_trailer_preference(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted = cx.global::<crate::prefs::Prefs>().preferences.issue_trailer;
        if wanted == self.trailer_wanted {
            return;
        }
        self.trailer_wanted = wanted;
        if wanted {
            self.issue_trailer_due = true;
        } else if let Some(number) = self.issue_number
            && self.description_text.trim()
                == format!("{}: #{number}", crate::views::issues::facts::TRAILER_KEY)
        {
            self.description_text.clear();
            self.description
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        cx.notify();
    }

    /// The trailer that names the branch's issue, for a description that
    /// is empty, when the preferences want it and the box is not amending.
    fn issue_trailer(&self, cx: &App) -> Option<String> {
        let number = self.issue_number?;
        let wanted = cx.global::<crate::prefs::Prefs>().preferences.issue_trailer;
        let named = crate::views::issues::facts::names_issue(&self.summary_text, number);
        (wanted && !self.amend && !named && self.description_text.trim().is_empty())
            .then(|| format!("{}: #{number}", crate::views::issues::facts::TRAILER_KEY))
    }

    /// The operation in progress changed, or ended.
    pub fn set_stopped(&mut self, stopped: Option<Stopped>, cx: &mut Context<Self>) {
        if self.stopped != stopped {
            self.stopped = stopped;
            cx.notify();
        }
    }

    /// Add a trailer to the message the way git adds one: into the block
    /// of trailers at its end when there is one, and by the repository's
    /// own trailer settings. `Signed-off-by` without a value signs off as
    /// whoever git commits as.
    pub fn add_trailer(&mut self, key: &str, value: &str, cx: &mut Context<Self>) {
        let description = self.description.read(cx).value().to_string();
        // The summary may not be written yet; the trailers go under the
        // description either way.
        let summary = match self.summary_text.trim() {
            "" => "summary",
            typed => typed,
        };
        let sign_off = key.eq_ignore_ascii_case("Signed-off-by") && value.trim().is_empty();
        let trailers = if sign_off {
            json!([])
        } else {
            json!([{ "key": key, "value": value }])
        };
        let request = Request::Command {
            id: "commit.trailer".into(),
            params: json!({
                "message": format!("{summary}\n\n{}", description.trim_end()),
                "trailers": trailers,
                "sign_off": sign_off,
            }),
        };
        self.error = None;
        self.session.call(request, cx, move |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    if let Some(message) = value.get("message").and_then(|m| m.as_str()) {
                        let (_, body) = split_message(message);
                        this.pending_set = Some((this.summary_text.clone(), body));
                    }
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
    }

    /// Put a message of the history into the box. What the box held is
    /// kept, and offered at the end of the history until the next commit.
    fn take_message(&mut self, index: usize, cx: &mut Context<Self>) {
        let taken = match self.recent.get(index) {
            Some(recent) => split_message(&recent.message),
            None => match self.draft.clone() {
                Some(draft) => draft,
                None => return,
            },
        };
        let held = (
            self.summary_text.trim().to_owned(),
            self.description.read(cx).value().trim().to_owned(),
        );
        let from_history = self
            .recent
            .iter()
            .any(|recent| split_message(&recent.message) == held);
        if !(held.0.is_empty() && held.1.is_empty()) && !from_history && held != taken {
            self.draft = Some(held);
        }
        self.pending_set = Some(taken);
        cx.notify();
    }
}

/// A message as the box holds it: the first line, and what follows the
/// blank line after it.
pub fn split_message(message: &str) -> (String, String) {
    let message = message.trim();
    match message.split_once('\n') {
        Some((summary, rest)) => (summary.trim().to_owned(), rest.trim().to_owned()),
        None => (message.to_owned(), String::new()),
    }
}

/// How wide the description is: its widest line in characters, with the
/// number of that line. Lines git strips as comments do not count.
pub fn widest_line(description: &str) -> (usize, usize) {
    description
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.starts_with('#'))
        .map(|(number, line)| (line.trim_end().chars().count(), number.saturating_add(1)))
        .fold(
            (0, 0),
            |widest, line| if line.0 > widest.0 { line } else { widest },
        )
}

/// What a line of the history shows of a message: its summary, cut where
/// a menu would grow too wide, and a mark when there is more under it.
pub fn history_label(summary: &str, has_body: bool) -> String {
    const SHOWN: usize = 56;
    let mut label: String = summary.chars().take(SHOWN).collect();
    if summary.chars().count() > SHOWN {
        label.push('…');
    }
    if has_body {
        label.push_str("  ¶");
    }
    label
}

/// The trailer keys of `commit.trailer_keys` that get an entry of their
/// own: not the two the menu always has.
pub fn offered_keys(listed: &serde_json::Value) -> Vec<String> {
    listed
        .as_array()
        .map(|keys| {
            keys.iter()
                .filter_map(|key| key.get("key").and_then(|key| key.as_str()))
                .filter(|key| {
                    !key.eq_ignore_ascii_case("Co-authored-by")
                        && !key.eq_ignore_ascii_case("Signed-off-by")
                })
                .take(KEYS)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a trailer of this key names a person, so that the people of
/// recent history are offered for it.
pub fn names_a_person(key: &str) -> bool {
    key.to_ascii_lowercase().ends_with("-by")
}

/// The people of recent history as entries that add a trailer naming one.
fn people_entries(menu: PopupMenu, key: &str, people: &[Person]) -> PopupMenu {
    let mut menu = menu;
    if people.is_empty() {
        menu = menu.label(say!("commit-no-people"));
    }
    for person in people.iter().take(20) {
        menu = menu.menu(
            format!("{} <{}>", person.name, person.email),
            Box::new(AddTrailer {
                key: key.to_owned(),
                value: format!("{} <{}>", person.name, person.email),
            }),
        );
    }
    menu.separator().menu(
        say!("commit-someone-else"),
        Box::new(AddTrailer {
            key: key.to_owned(),
            value: String::new(),
        }),
    )
}

/// The texts of a JSON list; nothing for anything else.
fn texts(value: Option<&serde_json::Value>) -> Vec<String> {
    value
        .and_then(|value| value.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// The hooks `hook.list` answers with, by name.
pub fn hook_names(listed: &serde_json::Value) -> Vec<String> {
    listed
        .get("hooks")
        .and_then(|hooks| hooks.as_array())
        .map(|hooks| {
            hooks
                .iter()
                .filter_map(|hook| hook.get("name").and_then(|name| name.as_str()))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Of the hooks `hook.list` answers with, the ones a commit made without
/// hooks leaves out: the two that can refuse it.
pub fn refusing_hooks(listed: &serde_json::Value) -> Vec<String> {
    hook_names(listed)
        .into_iter()
        .filter(|name| bitbraid_git::hooks::COMMIT_NO_VERIFY.contains(&name.as_str()))
        .collect()
}

/// "pre-commit", "pre-commit and commit-msg": names as a phrase.
pub fn name_list(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [one] => one.clone(),
        [first @ .., last] => say!(
            "commit-name-list",
            first = first.join(", "),
            last = last.as_str()
        ),
    }
}

/// What is said under the button while hooks are left out.
pub fn skip_warning(hooks: &[String]) -> String {
    say!(
        "commit-skip-warning",
        count = hooks.len(),
        hooks = name_list(hooks)
    )
}

impl Focusable for CommitBox {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for CommitBox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some((summary, body)) = self.pending_set.take() {
            if summary != self.summary_text {
                self.summary_text = summary.clone();
                self.summary
                    .update(cx, |state, cx| state.set_value(summary, window, cx));
            }
            self.description_text = body.clone();
            self.description
                .update(cx, |state, cx| state.set_value(body, window, cx));
        }
        if self.pending_clear {
            self.pending_clear = false;
            self.summary_text.clear();
            self.description_text.clear();
            self.summary
                .update(cx, |state, cx| state.set_value("", window, cx));
            self.description
                .update(cx, |state, cx| state.set_value("", window, cx));
        }
        if self.issue_trailer_due {
            self.issue_trailer_due = false;
            if let Some(trailer) = self.issue_trailer(cx) {
                self.description_text = trailer.clone();
                self.description
                    .update(cx, |state, cx| state.set_value(trailer, window, cx));
            }
        }
        let theme = cx.theme();
        let len = self.summary_text.chars().count();
        let (widest, widest_at) = widest_line(&self.description_text);
        let recent: Vec<(String, bool)> = self
            .recent
            .iter()
            .map(|recent| (recent.summary.clone(), recent.message.trim().contains('\n')))
            .collect();
        let draft = self.draft.clone();
        let keys = self.trailer_keys.clone();
        let (commit_label, commit_on) = self.button();
        // Staging everything first needs nothing staged yet.
        let can_commit_more = !self.summary_text.trim().is_empty() && !self.busy;
        let amending = self.amend;
        let skipping = self.skip_hooks && !self.hooks.is_empty();
        let people = self.people.clone();
        let stopped = self.stopped.clone();
        let summary_typed = !self.summary_text.trim().is_empty();
        let stash_tab = self.tab == FormTab::Stash && self.stash_offered();
        let least = if stash_tab { STASH_FRAME } else { FRAME_HEIGHT };
        // A window made smaller leaves the lists less: the message gives
        // back what it was dragged taller by, down to its least. With the
        // lists down to a row each, its description gives way, down to two
        // lines, before the form runs out of the panel. How high the form
        // was the last time with the message as tall as it wants is what it
        // was drawn and what was taken off it then.
        let (ceiling, floor) = self.room.get();
        let mut natural = self.height_drawn.get() + self.taken.get();
        let over = natural - (floor - ceiling);
        if over.is_finite() && over > 0.5 && self.extra > 0. {
            let given = over.min(self.extra);
            self.extra -= given;
            natural -= given;
        }
        let tightest = self.tightest.get();
        let taken = squeezed(natural, floor - tightest, least + self.extra);
        self.taken.set(taken);
        let left = SUMMARY_HARD as i64 - len as i64;
        let frame_drawn = self.frame_drawn.clone();
        let extra = self.extra - taken;
        // The message: summary and description in one frame, as tall as
        // it was dragged; it gives way before the lists do. `least` is
        // its height undragged.
        let message_frame = |least: f32, count: bool| {
            v_flex()
                .id(if count {
                    "commit-message"
                } else {
                    "stash-message"
                })
                .relative()
                .mx(px(11.))
                .flex_basis(px(least + extra))
                .flex_shrink(1.)
                // In a window too low for the lists and all of the form,
                // the description is what gives way.
                .min_h(px(FRAME_LEAST))
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                // A long description scrolls inside the frame; it does
                // not run out over what is under it.
                .overflow_hidden()
                .child({
                    let frame_drawn = frame_drawn.clone();
                    canvas(
                        move |bounds, _, _| {
                            frame_drawn.set(crate::zoom::unpx(bounds.size.height) - least)
                        },
                        |_, _, _, _| {},
                    )
                    .absolute()
                    .size_full()
                })
                .child(
                    h_flex()
                        .gap_2()
                        .pr_2()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .child(Input::new(&self.summary).appearance(false)),
                        )
                        // How many characters are left of the 72 where
                        // lists cut a summary off: past the 50 git advises
                        // a warning, past 72 wrong.
                        .when(count, |row| {
                            row.child(
                                div()
                                    .text_sm()
                                    .text_color(if left < 0 {
                                        theme.danger
                                    } else if len > SUMMARY_SOFT {
                                        theme.warning
                                    } else {
                                        theme.muted_foreground
                                    })
                                    .child(left.to_string()),
                            )
                        }),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(Textarea::new(&self.description).appearance(false).h_full()),
                )
        };
        let mut form: Vec<AnyElement> = Vec::new();
        if stash_tab {
            // What is staged, set aside under the title and description.
            let (label, on) = self.stash_button();
            form.push(
                message_frame(STASH_FRAME, false)
                    .mt(px(10.))
                    .test_support()
                    .into_any_element(),
            );
            form.push(
                h_flex()
                    .mt(px(10.))
                    .mx(px(11.))
                    .child(
                        Button::new("stash-staged")
                            .primary()
                            .flex_1()
                            .h(px(40.))
                            .label(label)
                            .disabled(!on)
                            .on_click(cx.listener(|this, _, _, cx| this.stash(cx))),
                    )
                    .into_any_element(),
            );
        } else {
            // Amending is chosen before the message is written.
            form.push(
                h_flex()
                    .mt(px(10.))
                    .h(px(24.))
                    .pl(px(11.))
                    .items_center()
                    .child(
                        Checkbox::new("amend")
                            .label(say!("commit-amend-last"))
                            .checked(self.amend)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_amend(*checked, cx)
                            })),
                    )
                    .into_any_element(),
            );
            form.push(
                message_frame(FRAME_HEIGHT, true)
                    .test_support()
                    .into_any_element(),
            );
            // Under the message, folded: the options of the next commit.
            // At its right a message to take back, a trailer to add, how
            // wide the description has become, and the other ways to
            // commit.
            let open = self.options_open;
            form.push(
                h_flex()
                    .id("commit-options")
                    .mt(px(6.))
                    .h(px(24.))
                    .pl(px(13.))
                    .pr(px(11.))
                    .gap_1()
                    .items_center()
                    .child(
                        h_flex()
                            .id("commit-options-fold")
                            .test_support()
                            .gap(px(5.))
                            .items_center()
                            .cursor_pointer()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.options_open = !this.options_open;
                                cx.notify();
                            }))
                            .child(
                                Icon::new(if open {
                                    IconName::ChevronDown
                                } else {
                                    IconName::ChevronRight
                                })
                                .size_3(),
                            )
                            .child(say!("commit-options")),
                    )
                    .child(div().flex_1())
                    .when(widest > 0, |row| {
                        row.child(
                            div()
                                .text_xs()
                                .text_color(if widest > BODY_WIDTH {
                                    theme.warning
                                } else {
                                    theme.muted_foreground
                                })
                                .child(if widest > BODY_WIDTH {
                                    say!(
                                        "commit-line-too-wide",
                                        line = widest_at,
                                        widest,
                                        limit = BODY_WIDTH
                                    )
                                } else {
                                    say!("commit-widest-line", widest, limit = BODY_WIDTH)
                                }),
                        )
                    })
                    .child(
                        Button::new("message-history")
                            .ghost()
                            .xsmall()
                            .label(say!("commit-history"))
                            .dropdown_menu(move |menu, _, _| {
                                let mut menu = menu;
                                if recent.is_empty() && draft.is_none() {
                                    menu = menu.label(say!("commit-history-none"));
                                }
                                for (index, (summary, has_body)) in recent.iter().enumerate() {
                                    menu = menu.menu(
                                        history_label(summary, *has_body),
                                        Box::new(TakeMessage { index }),
                                    );
                                }
                                if let Some((summary, body)) = &draft {
                                    let shown = if summary.is_empty() { body } else { summary };
                                    menu = menu.separator().menu(
                                        say!(
                                            "commit-history-typed",
                                            typed = history_label(
                                                shown.lines().next().unwrap_or_default(),
                                                false
                                            )
                                        ),
                                        Box::new(TakeMessage { index: usize::MAX }),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        Button::new("add-trailer")
                            .ghost()
                            .xsmall()
                            .label(say!("commit-add-trailer"))
                            .dropdown_menu(move |menu, window, cx| {
                                let trailer = |key: &str, value: &str| -> Box<dyn Action> {
                                    Box::new(AddTrailer {
                                        key: key.to_owned(),
                                        value: value.to_owned(),
                                    })
                                };
                                let for_people = people.clone();
                                let mut menu = menu
                                    .menu(say!("commit-sign-off-me"), trailer("Signed-off-by", ""))
                                    .submenu("Co-authored-by", window, cx, move |menu, _, _| {
                                        people_entries(menu, "Co-authored-by", &for_people)
                                    });
                                if !keys.is_empty() {
                                    menu = menu.separator();
                                }
                                for key in &keys {
                                    if names_a_person(key) {
                                        let (key, for_people) = (key.clone(), people.clone());
                                        menu = menu.submenu(
                                            key.clone(),
                                            window,
                                            cx,
                                            move |menu, _, _| {
                                                people_entries(menu, &key, &for_people)
                                            },
                                        );
                                    } else {
                                        menu = menu.menu(format!("{key}…"), trailer(key, ""));
                                    }
                                }
                                menu.separator()
                                    .menu(say!("commit-another-trailer"), trailer("", ""))
                            }),
                    )
                    // The other ways to commit: with everything staged
                    // first, with a push afterwards, or both. Here, so
                    // that the button under it has the whole width.
                    .child(
                        Button::new("commit-more")
                            .accessibility_label(say!("commit-more"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::ChevronDown)
                            .tooltip(say!("commit-more"))
                            .disabled(!can_commit_more)
                            .dropdown_menu(move |menu, _, _| {
                                let with = |stage_all, push| -> Box<dyn Action> {
                                    Box::new(CommitWith { stage_all, push })
                                };
                                let (staging, pushing, both) = if amending {
                                    (
                                        say!("commit-stage-all-amend"),
                                        say!("commit-amend-push"),
                                        say!("commit-stage-all-amend-push"),
                                    )
                                } else {
                                    (
                                        say!("commit-stage-all-commit"),
                                        say!("commit-commit-push"),
                                        say!("commit-stage-all-commit-push"),
                                    )
                                };
                                menu.menu(staging, with(true, false))
                                    .menu(pushing, with(false, true))
                                    .menu(both, with(true, true))
                            }),
                    )
                    .into_any_element(),
            );
            if open {
                // Pushing afterwards and signing at the left, leaving the
                // hooks out at the right, each where it applies.
                form.push(
                    h_flex()
                        .h(px(25.))
                        .pl(px(16.))
                        .pr(px(11.))
                        .gap_3()
                        .items_center()
                        .child(
                            Checkbox::new("push-after")
                                .label(say!("commit-push-after"))
                                .checked(self.push_after)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.push_after = *checked;
                                    cx.notify();
                                })),
                        )
                        // Offered once there is a key, or when the
                        // repository signs anyway.
                        .when(self.sign_key || self.sign_default, |row| {
                            row.child(
                                Checkbox::new("sign")
                                    .label(say!("commit-sign"))
                                    .checked(self.sign.unwrap_or(self.sign_default))
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.sign = Some(*checked);
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(div().flex_1())
                        // Offered where there is a hook that could refuse.
                        .when(!self.hooks.is_empty(), |row| {
                            row.child(
                                Checkbox::new("skip-hooks")
                                    .label(say!("commit-skip-hooks"))
                                    .checked(self.skip_hooks)
                                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                        this.skip_hooks = *checked;
                                        cx.notify();
                                    })),
                            )
                        })
                        .into_any_element(),
                );
            }
            // The button is as wide as the frame, and says what it will
            // do.
            form.push(
                div()
                    // The two ways out of a stopped operation stand closer.
                    .mt(px(if stopped.is_some() { 11. } else { 16. }))
                    .mx(px(11.))
                    .child(
                        h_flex()
                            .w_full()
                            .gap_1()
                            .items_center()
                            // While an operation is stopped part way: the two ways
                            // out of it, each half the width, going on first.
                            .when_some(stopped.clone(), |row, stopped| {
                                let (label, commits) = going_on(&stopped, summary_typed);
                                let busy = self.busy;
                                row.gap(px(5.))
                                    .child(
                                        Button::new("op-continue")
                                            .primary()
                                            .flex_1()
                                            .h(px(40.))
                                            .label(label)
                                            .disabled(stopped.conflicts > 0 || busy)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if commits {
                                                    this.commit(false, false, cx)
                                                } else {
                                                    cx.emit(CommitEvent::Continue)
                                                }
                                            })),
                                    )
                                    .child(
                                        Button::new("op-abort")
                                            .danger()
                                            .flex_1()
                                            .h(px(40.))
                                            .label(say!(
                                                "commit-abort",
                                                noun = stopped.noun.as_str()
                                            ))
                                            .on_click(cx.listener(|_, _, _, cx| {
                                                cx.emit(CommitEvent::Abort)
                                            })),
                                    )
                            })
                            .when(stopped.is_none(), |row| {
                                row.child(
                                    Button::new("commit")
                                        .primary()
                                        .flex_1()
                                        .h(px(40.))
                                        .label(commit_label)
                                        .disabled(!commit_on)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            let push = this.push_after;
                                            this.commit(false, push, cx)
                                        })),
                                )
                            }),
                    )
                    .into_any_element(),
            );
        }
        v_flex()
            .id("commit-box")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(|this, action: &AddTrailer, _, cx| {
                if action.value.trim().is_empty()
                    && !action.key.eq_ignore_ascii_case("Signed-off-by")
                {
                    cx.emit(CommitEvent::AskTrailer {
                        key: action.key.clone(),
                    });
                } else {
                    this.add_trailer(&action.key, &action.value, cx);
                }
            }))
            .on_action(
                cx.listener(|this, action: &TakeMessage, _, cx| {
                    this.take_message(action.index, cx)
                }),
            )
            .on_action(cx.listener(|this, action: &CommitWith, _, cx| {
                this.commit(action.stage_all, action.push, cx)
            }))
            // The grip over the form makes the message taller and the
            // lists shorter, or the other way.
            .on_drag_move(cx.listener(|this, event: &DragMoveEvent<FormGrip>, _, cx| {
                // Where the grip's top would be, under the pointer.
                let pointer = (crate::zoom::unpx(event.event.position.y) - this.grab.get())
                    .max(this.room.get().0);
                let up = crate::zoom::unpx(event.bounds.top()) - pointer;
                let drawn = this.frame_drawn.get().max(0.);
                let extra = (drawn + up).max(0.);
                if (extra - this.extra).abs() >= 0.5 {
                    this.extra = extra;
                    cx.notify();
                }
            }))
            .relative()
            .w_full()
            .flex_shrink(1.)
            .pb(px(10.))
            .bg(theme.background)
            .child({
                let (height_drawn, room) = (self.height_drawn.clone(), self.room.clone());
                let tightest = self.tightest.clone();
                let dragged = self.extra > 0.;
                let wanted = least + self.extra;
                canvas(
                    move |bounds, window, _| {
                        let height = crate::zoom::unpx(bounds.size.height);
                        height_drawn.set(height);
                        // Drawn taller than the room it has: drawn again,
                        // with the message given back what it needs to.
                        // So also when the description would give way by
                        // another amount than it did.
                        let (ceiling, floor) = room.get();
                        let tight = floor - tightest.get();
                        // On the next frame: the window does not hear a
                        // refresh while it draws, and nothing else might
                        // draw it again (a window made smaller, once).
                        if (dragged && height - (floor - ceiling) > 0.5)
                            || (squeezed(height + taken, tight, wanted) - taken).abs() > 0.5
                        {
                            window.request_animation_frame();
                        }
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .size_full()
            })
            .child(
                div()
                    .id("form-grip")
                    .w_full()
                    .h(px(10.))
                    .flex()
                    .justify_center()
                    .cursor_row_resize()
                    .on_drag(FormGrip, {
                        let grab = self.grab.clone();
                        move |_, offset, _, cx| {
                            grab.set(crate::zoom::unpx(offset.y));
                            cx.new(|_| GripChip)
                        }
                    })
                    .child(div().w(px(100.)).h(px(4.)).rounded_full().bg(theme.border))
                    .test_support(),
            )
            .child(self.render_tabs(stash_tab, cx))
            .children(form)
            // A commit that can be left out: under the two buttons.
            .when(
                stopped.as_ref().is_some_and(|stopped| stopped.can_skip),
                |d| {
                    d.child(
                        h_flex().w_full().justify_end().child(
                            Button::new("op-skip")
                                .ghost()
                                .xsmall()
                                .label(say!("commit-skip-this"))
                                .on_click(cx.listener(|_, _, _, cx| cx.emit(CommitEvent::Skip))),
                        ),
                    )
                },
            )
            .when(skipping, |d| {
                d.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(skip_warning(&self.hooks)),
                )
            })
            .when_some(self.error.clone(), |d, err| {
                d.child(div().text_xs().text_color(theme.danger).child(err))
            })
    }
}

#[cfg(test)]
mod button_tests {
    use super::{FRAME_HEIGHT, FRAME_LEAST, commit_button, squeezed, stash_button};

    #[test]
    fn the_message_gives_way_only_as_far_as_its_least() {
        // It fits: nothing is taken.
        assert_eq!(squeezed(300., 400., FRAME_HEIGHT), 0.);
        // Over by 20: 20 off the frame.
        assert_eq!(squeezed(420., 400., FRAME_HEIGHT), 20.);
        // Over by more than the frame has to give: what it has.
        assert_eq!(
            squeezed(900., 400., FRAME_HEIGHT),
            FRAME_HEIGHT - FRAME_LEAST
        );
        // No room known yet, or no number: nothing.
        assert_eq!(squeezed(900., f32::INFINITY, FRAME_HEIGHT), 0.);
        assert_eq!(squeezed(f32::NAN, 400., FRAME_HEIGHT), 0.);
        assert_eq!(squeezed(900., 400., 10.), 0.);
    }

    #[test]
    fn the_stash_button_waits_for_something_staged_and_counts_it() {
        assert_eq!(
            stash_button(0, false),
            ("Stage a file to stash".to_owned(), false)
        );
        assert_eq!(stash_button(1, false), ("Stash 1 file".to_owned(), true));
        assert_eq!(stash_button(3, false), ("Stash 3 files".to_owned(), true));
        assert_eq!(stash_button(3, true), ("Stash 3 files".to_owned(), false));
    }

    fn said(
        summary: bool,
        busy: bool,
        amend: bool,
        staged: usize,
        skipping: bool,
    ) -> (String, bool) {
        commit_button(summary, busy, amend, staged, skipping)
    }

    #[test]
    fn the_button_waits_for_a_summary_then_for_something_staged() {
        // Without a summary it says so, whatever is staged.
        for staged in [0, 3] {
            assert_eq!(
                said(false, false, false, staged, false),
                ("Type a summary to commit".to_owned(), false)
            );
        }
        // With one and nothing staged there is nothing to commit.
        assert_eq!(
            said(true, false, false, 0, false),
            ("Stage a file to commit".to_owned(), false)
        );
    }

    #[test]
    fn the_button_says_how_many_files_it_commits() {
        assert_eq!(
            said(true, false, false, 1, false),
            ("Commit 1 file".to_owned(), true)
        );
        assert_eq!(
            said(true, false, false, 4, false),
            ("Commit 4 files".to_owned(), true)
        );
        assert_eq!(
            said(true, false, false, 4, true),
            ("Commit 4 files without hooks".to_owned(), true)
        );
        // While a commit is being made it cannot be pressed again.
        assert_eq!(
            said(true, true, false, 4, false),
            ("Commit 4 files".to_owned(), false)
        );
    }

    #[test]
    fn amending_needs_nothing_staged() {
        assert_eq!(
            said(true, false, true, 0, false),
            ("Amend".to_owned(), true)
        );
        assert_eq!(
            said(true, false, true, 2, true),
            ("Amend without hooks".to_owned(), true)
        );
        assert_eq!(
            said(true, true, true, 0, false),
            ("Amend".to_owned(), false)
        );
        // The summary comes first all the same.
        assert_eq!(
            said(false, false, true, 2, false),
            ("Type a summary to commit".to_owned(), false)
        );
    }
}

#[cfg(test)]
mod stopped_tests {
    use super::{Stopped, going_on};

    #[test]
    fn a_merge_is_concluded_under_what_was_typed_and_anything_else_goes_on() {
        let stopped = |noun: &str| Stopped {
            noun: noun.to_owned(),
            conflicts: 0,
            can_skip: false,
        };
        assert_eq!(
            going_on(&stopped("merge"), false),
            ("Continue merge".to_owned(), false)
        );
        assert_eq!(
            going_on(&stopped("merge"), true),
            ("Commit merge".to_owned(), true)
        );
        // A rebase replays commits that have their messages already.
        assert_eq!(
            going_on(&stopped("rebase"), true),
            ("Continue rebase".to_owned(), false)
        );
        assert_eq!(
            going_on(&stopped("cherry-pick"), false),
            ("Continue cherry-pick".to_owned(), false)
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        history_label, name_list, names_a_person, offered_keys, refusing_hooks, skip_warning,
        split_message, widest_line,
    };
    use serde_json::json;

    #[test]
    fn a_message_is_split_into_what_the_two_fields_hold() {
        assert_eq!(split_message("summary"), ("summary".into(), String::new()));
        assert_eq!(
            split_message("summary\n\nBody line one.\nLine two.\n\nRefs: #1\n"),
            (
                "summary".into(),
                "Body line one.\nLine two.\n\nRefs: #1".into()
            )
        );
        // Without the blank line the rest is the body all the same.
        assert_eq!(
            split_message("summary\nstraight on\n"),
            ("summary".into(), "straight on".into())
        );
        assert_eq!(split_message("  \n "), (String::new(), String::new()));
    }

    #[test]
    fn the_widest_line_is_counted_in_characters_and_named_by_its_number() {
        assert_eq!(widest_line(""), (0, 0));
        assert_eq!(widest_line("abc\nabcdef\nab"), (6, 2));
        // Characters, not bytes; blanks at the end and comments do not count.
        assert_eq!(
            widest_line("größe\n# a comment that is much longer\nab   "),
            (5, 1)
        );
        assert_eq!(widest_line(&"x".repeat(80)), (80, 1));
        // The first of two that are as wide.
        assert_eq!(widest_line("ab\ncd"), (2, 1));
    }

    #[test]
    fn a_line_of_the_history_is_a_summary_cut_to_fit() {
        assert_eq!(history_label("fix the parser", false), "fix the parser");
        assert_eq!(history_label("fix the parser", true), "fix the parser  ¶");
        let long = "ü".repeat(80);
        let label = history_label(&long, false);
        assert_eq!(label.chars().count(), 57);
        assert!(label.ends_with('…'));
        assert_eq!(history_label("", false), "");
    }

    #[test]
    fn the_keys_in_use_are_offered_but_for_the_two_that_always_are() {
        let listed = json!([
            { "key": "Ticket", "count": 0, "configured": true },
            { "key": "Co-authored-by", "count": 9, "configured": false },
            { "key": "signed-off-by", "count": 8, "configured": false },
            { "key": "Reviewed-by", "count": 3, "configured": false },
            { "count": 3 },
            7,
        ]);
        assert_eq!(offered_keys(&listed), ["Ticket", "Reviewed-by"]);
        assert!(offered_keys(&json!({ "key": "x" })).is_empty());
        let many: Vec<_> = (0..40)
            .map(|n| json!({ "key": format!("Key-{n}") }))
            .collect();
        assert_eq!(offered_keys(&json!(many)).len(), 10);
        assert!(names_a_person("Reviewed-by") && names_a_person("ACKED-BY"));
        assert!(!names_a_person("Fixes") && !names_a_person("by"));
    }

    #[test]
    fn only_the_hooks_that_can_refuse_a_commit_are_offered_to_be_skipped() {
        let listed = json!({
            "directory": "/r/.git/hooks",
            "configured": false,
            "hooks": [
                { "name": "pre-commit", "path": "/r/.git/hooks/pre-commit" },
                { "name": "prepare-commit-msg", "path": "/r/.git/hooks/prepare-commit-msg" },
                { "name": "commit-msg", "path": "/r/.git/hooks/commit-msg" },
                { "name": "post-commit", "path": "/r/.git/hooks/post-commit" },
                { "name": "pre-push", "path": "/r/.git/hooks/pre-push" },
            ],
        });
        assert_eq!(refusing_hooks(&listed), ["pre-commit", "commit-msg"]);
        assert!(refusing_hooks(&json!({ "hooks": [] })).is_empty());
        // Anything that is not the answer of the command names nothing.
        for odd in [
            json!(null),
            json!([]),
            json!({ "hooks": 3 }),
            json!({ "hooks": [1, {}] }),
        ] {
            assert!(refusing_hooks(&odd).is_empty(), "{odd}");
        }
    }

    #[test]
    fn the_warning_names_what_is_left_out() {
        let one = vec!["pre-commit".to_owned()];
        let two = vec!["pre-commit".to_owned(), "commit-msg".to_owned()];
        assert_eq!(name_list(&[]), "");
        assert_eq!(name_list(&one), "pre-commit");
        assert_eq!(name_list(&two), "pre-commit and commit-msg");
        assert_eq!(
            skip_warning(&one),
            "The pre-commit hook does not run and cannot refuse the commit."
        );
        assert_eq!(
            skip_warning(&two),
            "The pre-commit and commit-msg hooks do not run and cannot refuse the commit."
        );
    }
}
