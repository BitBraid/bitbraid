//! A pull request's view: it takes the graph's place and the right
//! panel's, opened from a row of the PULL REQUESTS section, and goes away
//! with its close button.
//!
//! At the top a heading 52 high with the provider's name and buttons that
//! read it again, open it in the system's browser and close the view.
//! Under it one column at most 1200 wide, centred, that scrolls as a
//! whole, laid out as an issue's view: the number over the title, a pill
//! of where it stands and who wants to merge which branch into which,
//! then a rule and two columns. The left one has the description drawn
//! from its Markdown, then the conversation (comments on the whole of it,
//! and threads on a line of a file with that file's place in the diff,
//! oldest first, with the reviews that said something), then the merge
//! box, then the field to comment and review. The right one, 300 wide,
//! has the files changed (a file opens its changes in the left column,
//! drawn from the patch the provider sent, the right column beside it
//! scrolling on its own), the commits, the reviewers, the assignees, the
//! checks (each opens its page), the labels, the milestone, everyone who
//! took part and the branch it brings, each under its heading and over a
//! rule.
//!
//! What changes it is in the modules under this one: editing in place
//! (`editing`), merging and the stack (`merging`), commenting and
//! reviewing (`talk`), suggestions from the edits in the working tree to
//! a file of it (`suggesting`), offered over the file's changes. Two
//! clicks on the branch it brings check that out (`pr.checkout`, through
//! the window, which asks first when that adds a fork's remote); on the
//! branch it goes into, check that out.
//!
//! Sources: docs.github.com/en/rest/pulls; docs.gitlab.com/api/merge_requests.

mod editing;
mod merging;
mod suggesting;
mod talk;

use std::rc::Rc;

pub use editing::Listed;
pub use merging::{merge_label, method_line, method_name};
pub use suggesting::{Suggesting, lines_words};
pub use talk::Verdict;

use bitbraid_integrations::model::{
    Check, Comment, MergeMethod, PullDetails, PullFile, PullOptions, PullState, RepoLink, Review,
    Stack, Thread, User,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, Textarea, TextareaState},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{
    card::{assignee_lines, branch_chip, branches_line, picture, reviewer_lines, who},
    checks::ci_icon,
    facts::{
        age, checks_summary, ci_word, due_date, may_open, number_text, parse_time, provider_name,
        shown_line,
    },
    markdown,
    patch::{self, PatchLine},
    section::provider_mark,
};
use crate::{
    session::{Reply, SessionHandle},
    zoom::px,
};

/// The view's measures, as those of an issue's view
/// (`super::super::issues::view`): the heading and its buttons; the
/// column, at most so wide and so far from either side, the room between
/// its two columns and the right one's width; from the heading to the
/// number, the number's line, the title's, the room over the pill and its
/// height, from the pill to the rule under them and from that rule to the
/// two columns; a part's heading, from it to what it holds, from that to
/// the part's rule in the right column; from the description's rule to
/// the comments' heading, from that to the first comment, and from one
/// entry of the conversation to the next.
const HEADING_HEIGHT: f32 = 52.0;
const HEADING_ICON: f32 = 18.0;
const COLUMN_WIDTH: f32 = 1200.0;
const COLUMN_SIDE: f32 = 40.0;
const COLUMN_GAP: f32 = 40.0;
const SIDE_WIDTH: f32 = 300.0;
const COLUMN_TOP: f32 = 21.0;
const NUMBER_LINE: f32 = 20.0;
const TITLE_LINE: f32 = 34.0;
const PILL_ABOVE: f32 = 7.0;
const PILL_HEIGHT: f32 = 19.0;
const TITLE_UNDER: f32 = 5.0;
const UNDER_TITLE: f32 = 6.0;
const PART_HEADING: f32 = 24.0;
const PART_UNDER_HEADING: f32 = 11.0;
const SIDE_PART_UNDER: f32 = 21.0;
const COMMENTS_ABOVE: f32 = 19.0;
const COMMENTS_UNDER_HEADING: f32 = 6.0;
const ENTRY_GAP: f32 = 21.0;
/// What the right column leaves its scroll bar beside a file's changes.
const SIDE_SCROLL_ROOM: f32 = 12.0;
/// The most lines of a thread's place in the diff that are shown: the
/// last ones, which end at the line it is about.
const HUNK_LINES: usize = 8;
/// The most entries of a list in the right column that are drawn.
const MOST_LISTED: usize = 200;

/// What the view asks of the window.
#[derive(Debug, Clone)]
pub enum PullViewEvent {
    /// Put the view away: the graph comes back.
    Close,
    /// Show another pull request of the same repository: one of its
    /// stack.
    Open(u64),
    /// It was changed at the provider: the lists are read again.
    Changed,
    /// Check out the branch it brings (`pr.checkout`); `fork` names the
    /// repository that branch is in when it is another's, whose remote
    /// may have to be added first.
    Checkout {
        number: u64,
        /// The fork, `owner/name`, when it comes from one.
        fork: Option<String>,
    },
    /// Check out the branch it goes into, of the remote it is at.
    CheckoutBase(String),
}

/// One entry of the conversation, in the order they were written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Entry {
    /// A thread, by its place among the threads.
    Thread(usize),
    /// A review that said something, by its place among the reviews.
    Review(usize),
}

/// What the view has on show, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullViewProbe {
    /// The pull request's number.
    pub number: u64,
    /// Its title, once read.
    pub title: Option<String>,
    /// Whether it is being read.
    pub loading: bool,
    /// What went wrong, when it could not be read.
    pub error: Option<String>,
    /// How many files it changes, threads it has, checks its head has.
    pub files: usize,
    pub threads: usize,
    pub checks: usize,
    /// The file whose changes the left column shows, if one.
    pub file_open: Option<String>,
    /// What is edited in place: `title` or `body`.
    pub editing: Option<&'static str>,
    /// What is being sent to the provider.
    pub sending: Option<&'static str>,
    /// What went wrong with the last thing sent.
    pub write_error: Option<String>,
    /// Its place in its stack, counted from the bottom, and how many.
    pub stack: Option<(usize, usize)>,
    /// What the comment field holds, and what it answers.
    pub composer: String,
    pub reply_to: Option<String>,
    /// How it would be merged; whether its branch goes; whether the one
    /// above it moves onto its base.
    pub method: Option<MergeMethod>,
    pub delete_branch: bool,
    pub retarget: bool,
    /// Why the edits to the open file cannot be offered as suggestions;
    /// none when they can.
    pub suggest_refused: Option<&'static str>,
    /// The suggestions from the open file's edits, while they are on show.
    pub suggesting: Option<Suggesting>,
    /// What its heading says when it was kept from before because the
    /// provider could not be reached; none when it was read.
    pub offline: Option<String>,
}

/// The view.
pub struct PullView {
    link: RepoLink,
    number: u64,
    details: Option<PullDetails>,
    error: Option<String>,
    loading: bool,
    /// The details were given (tests): nothing is read.
    given: bool,
    /// The description and the texts of the conversation, made ready to
    /// be drawn: the description, each thread's comments, each review.
    body: SharedString,
    comments: Vec<Vec<SharedString>>,
    reviews: Vec<SharedString>,
    /// The conversation in the order it was written.
    entries: Vec<Entry>,
    /// The lines of each thread's place in the diff.
    hunks: Vec<Rc<Vec<PatchLine>>>,
    /// The file whose changes the left column shows: its place among the
    /// files, and its lines.
    file: Option<(usize, Rc<Vec<PatchLine>>)>,
    /// Which reading the answer belongs to.
    generation: u64,
    now: i64,
    scroll: ScrollHandle,
    side_scroll: ScrollHandle,
    patch_scroll: UniformListScrollHandle,
    /// What can be chosen for it, once asked (for the pencils).
    options: Option<PullOptions>,
    options_asked: bool,
    /// What is edited in place, and what saves it on Enter.
    editing: Option<editing::Editing>,
    edit_subscription: Option<Subscription>,
    /// The comment field at the bottom, made the first time it is drawn,
    /// what it holds, and the conversation it answers (its id and a line
    /// that says whose).
    composer: Option<Entity<TextareaState>>,
    composer_text: String,
    composer_subscription: Option<Subscription>,
    reply_to: Option<(String, String)>,
    /// How it is to be merged, whether its branch goes, whether the one
    /// above it in its stack moves onto its base.
    method: Option<MergeMethod>,
    delete_branch: bool,
    retarget: bool,
    /// What is being sent to the provider, while it is.
    sending: Option<&'static str>,
    /// What went wrong with the last thing sent.
    write_error: Option<String>,
    /// Its stack, once read (GitHub).
    stack: Option<Stack>,
    /// The open repository's queue, and the branch checked out there:
    /// what suggestions from local edits are made with.
    session: Option<SessionHandle>,
    checked_out: Option<String>,
    /// Suggestions from the edits to a file, while they are on show.
    suggesting: Option<Suggesting>,
}

impl EventEmitter<PullViewEvent> for PullView {}

/// When a thread was begun: its first comment's time.
fn thread_time(thread: &Thread) -> i64 {
    thread
        .comments
        .first()
        .and_then(|comment| parse_time(&comment.created))
        .unwrap_or(0)
}

/// The conversation in the order it was written: the threads, and the
/// reviews that said something.
fn entries_of(details: &PullDetails) -> Vec<Entry> {
    let mut timed: Vec<(i64, Entry)> = details
        .threads
        .iter()
        .enumerate()
        .filter(|(_, thread)| !thread.comments.is_empty())
        .map(|(at, thread)| (thread_time(thread), Entry::Thread(at)))
        .chain(
            details
                .reviews
                .iter()
                .enumerate()
                .filter(|(_, review)| !review.body.trim().is_empty())
                .map(|(at, review)| {
                    (
                        review
                            .submitted
                            .as_deref()
                            .and_then(parse_time)
                            .unwrap_or(0),
                        Entry::Review(at),
                    )
                }),
        )
        .collect();
    timed.sort_by_key(|(time, _)| *time);
    timed.into_iter().map(|(_, entry)| entry).collect()
}

/// Everyone who took part: the author, the reviewers, the assignees and
/// whoever commented, each once, in that order.
fn participants(details: &PullDetails) -> Vec<User> {
    let mut people: Vec<User> = Vec::new();
    let mut add = |user: &User| {
        if !user.login.is_empty() && !people.iter().any(|known| known.login == user.login) {
            people.push(user.clone());
        }
    };
    add(&details.pull.author);
    for reviewer in &details.pull.reviewers {
        add(&reviewer.user);
    }
    for user in &details.pull.assignees {
        add(user);
    }
    for review in &details.reviews {
        add(&review.author);
    }
    for thread in &details.threads {
        for comment in &thread.comments {
            add(&comment.author);
        }
    }
    people
}

/// The mark in the heading of a pull request's or an issue's view when
/// what it shows was kept from before: the refresh did not reach the
/// provider. `said` is [`super::facts::offline_said`]'s.
pub(crate) fn offline_mark(id: &'static str, said: String, cx: &App) -> AnyElement {
    h_flex()
        .id(id)
        .flex_none()
        .gap_1()
        .items_center()
        .text_xs()
        .text_color(cx.theme().warning)
        .child(Icon::new(gpui_kit::assets::IconName::CloudOff).size_3p5())
        .child(said)
        .tooltip(|window, cx| Tooltip::new(say!("pulls-offline-tooltip")).build(window, cx))
        .test_support()
        .into_any_element()
}

/// The word and the colour of the pill of where a pull request stands.
fn pill(details: &PullDetails, cx: &App) -> (&'static str, Hsla) {
    let theme = cx.theme();
    match (details.pull.state, details.pull.draft) {
        (PullState::Merged, _) => (say!("pulls-merged"), theme.primary),
        (PullState::Closed, _) => (say!("pulls-closed"), theme.danger),
        (PullState::Open, true) => (say!("pulls-draft"), theme.muted_foreground),
        (PullState::Open, false) => (say!("pulls-open"), theme.success),
    }
}

/// The letter of a file's change, and its colour.
fn file_mark(file: &PullFile, cx: &App) -> (&'static str, Hsla) {
    let theme = cx.theme();
    match file.status.as_str() {
        "added" => ("A", theme.success),
        "removed" => ("D", theme.danger),
        "renamed" => ("R", theme.primary),
        "copied" => ("C", theme.primary),
        _ => ("M", theme.warning),
    }
}

impl PullView {
    /// The view of pull request `number` of the repository `link` names;
    /// it is read at once.
    pub fn new(link: RepoLink, number: u64, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            link,
            number,
            details: None,
            error: None,
            loading: false,
            given: false,
            body: SharedString::default(),
            comments: Vec::new(),
            reviews: Vec::new(),
            entries: Vec::new(),
            hunks: Vec::new(),
            file: None,
            generation: 0,
            now: crate::views::now(),
            scroll: ScrollHandle::new(),
            side_scroll: ScrollHandle::new(),
            patch_scroll: UniformListScrollHandle::new(),
            options: None,
            options_asked: false,
            editing: None,
            edit_subscription: None,
            composer: None,
            composer_text: String::new(),
            composer_subscription: None,
            reply_to: None,
            method: None,
            delete_branch: false,
            retarget: true,
            sending: None,
            write_error: None,
            stack: None,
            session: None,
            checked_out: None,
            suggesting: None,
        };
        view.load(false, cx);
        view
    }

    /// Which pull request it shows.
    pub fn shows(&self) -> (&RepoLink, u64) {
        (&self.link, self.number)
    }

    /// Whether it is being read, and what went wrong when it could not be.
    pub fn reading(&self) -> (bool, Option<&str>) {
        (self.loading, self.error.as_deref())
    }

    /// Read it from the provider; past what was kept when `fresh`.
    fn load(&mut self, fresh: bool, cx: &mut Context<Self>) {
        if self.given {
            return;
        }
        self.loading = true;
        self.now = crate::views::now();
        self.generation = self.generation.wrapping_add(1);
        let generation = self.generation;
        self.read_stack(cx);
        let reply = super::calls::run(
            "pr.show",
            json!({
                "account": self.link.account,
                "owner": self.link.owner,
                "name": self.link.name,
                "number": self.number,
                "fresh": fresh,
            }),
        );
        SessionHandle::deliver(reply, "pr.show".to_owned(), cx, move |this, reply, cx| {
            if this.generation != generation {
                return;
            }
            this.loading = false;
            match reply {
                Reply::Command(value) => match serde_json::from_value::<PullDetails>(value) {
                    Ok(details) => this.take(details),
                    Err(_) => this.error = Some(say!("pulls-reply-unreadable").to_owned()),
                },
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
        cx.notify();
    }

    /// Read it again, past what was kept.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    /// Hold what was read, and make its texts ready to be drawn.
    fn take(&mut self, details: PullDetails) {
        self.error = None;
        self.body = markdown::prepared(&details.body);
        self.comments = details
            .threads
            .iter()
            .map(|thread| {
                thread
                    .comments
                    .iter()
                    .map(|comment| markdown::prepared(&comment.body))
                    .collect()
            })
            .collect();
        self.reviews = details
            .reviews
            .iter()
            .map(|review| markdown::prepared(&review.body))
            .collect();
        self.hunks = details
            .threads
            .iter()
            .map(|thread| {
                let lines = thread
                    .diff_hunk
                    .as_deref()
                    .map(patch::parse)
                    .unwrap_or_default();
                // The hunk's range, then the lines that end at the one
                // it is about.
                let skip = lines.len().saturating_sub(HUNK_LINES);
                Rc::new(lines.into_iter().skip(skip).collect())
            })
            .collect();
        self.entries = entries_of(&details);
        // A file that was open stays open while the pull request still
        // changes it.
        self.file = self.file.take().and_then(|(at, _)| {
            let file = details.files.get(at)?;
            Some((
                at,
                Rc::new(patch::parse(file.patch.as_deref().unwrap_or(""))),
            ))
        });
        self.details = Some(details);
    }

    /// Show these details in place of what the provider would answer, and
    /// read nothing: for the off-screen tests.
    pub fn give(&mut self, details: PullDetails, cx: &mut Context<Self>) {
        self.given = true;
        // What was being read is not taken when it comes.
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.now = crate::views::now();
        self.take(details);
        cx.notify();
    }

    /// Show the changes of a file in the left column, or the conversation
    /// again (none).
    pub fn open_file(&mut self, at: Option<usize>, cx: &mut Context<Self>) {
        if self.suggesting.as_ref().is_some_and(|s| s.sent.is_some()) {
            return;
        }
        self.suggesting = None;
        self.file = at.and_then(|at| {
            let file = self.details.as_ref()?.files.get(at)?;
            Some((
                at,
                Rc::new(patch::parse(file.patch.as_deref().unwrap_or(""))),
            ))
        });
        self.scroll.set_offset(point(px(0.), px(0.)));
        cx.notify();
    }

    /// What the view has on show.
    pub fn probe(&self) -> PullViewProbe {
        let details = self.details.as_ref();
        PullViewProbe {
            number: self.number,
            title: details.map(|details| details.pull.title.clone()),
            loading: self.loading,
            error: self.error.clone(),
            files: details.map_or(0, |details| details.files.len()),
            threads: details.map_or(0, |details| details.threads.len()),
            checks: details.map_or(0, |details| details.checks.len()),
            file_open: self
                .file
                .as_ref()
                .and_then(|(at, _)| Some(details?.files.get(*at)?.path.clone())),
            editing: self.editing.as_ref().map(|editing| match editing {
                editing::Editing::Title(_) => "title",
                editing::Editing::Body(_) => "body",
            }),
            sending: self.sending,
            write_error: self.write_error.clone(),
            stack: self.stack.as_ref().and_then(super::facts::stack_place),
            composer: self.composer_text.clone(),
            reply_to: self.reply_to.as_ref().map(|(id, _)| id.clone()),
            method: self.method(),
            delete_branch: self.delete_branch,
            retarget: self.retarget,
            suggest_refused: self.suggest_refused(),
            suggesting: self.suggesting.clone(),
            offline: self.offline(),
        }
    }

    /// What the heading says when what is shown was kept from before,
    /// because the provider could not be reached, with when it was read.
    fn offline(&self) -> Option<String> {
        super::facts::offline_said(&self.details.as_ref()?.freshness, self.now)
    }

    fn render_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let offline = self.offline();
        let url = self
            .details
            .as_ref()
            .map(|details| details.pull.web_url.clone())
            .filter(|url| may_open(url));
        let what = match self.link.kind {
            bitbraid_integrations::model::ProviderKind::Github => say!("pulls-view-github"),
            bitbraid_integrations::model::ProviderKind::Gitlab => say!("pulls-view-gitlab"),
        };
        h_flex()
            .w_full()
            .h(px(HEADING_HEIGHT))
            .flex_none()
            .pl(px(20.))
            .pr(px(10.))
            .gap(px(10.))
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(provider_mark(self.link.kind, cx))
            .child(div().text_xl().child(what))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{}/{} · {}",
                        shown_line(&self.link.owner, 120),
                        shown_line(&self.link.name, 120),
                        shown_line(&self.link.remote, 80)
                    )),
            )
            .child(div().flex_1())
            // Kept from before: the refresh did not reach the provider,
            // as the pull requests' section says it too.
            .when_some(offline, |heading, said| {
                heading.child(offline_mark("pull-offline", said, cx))
            })
            .child(
                Button::new("pull-refresh")
                    .accessibility_label(say!("pulls-refresh"))
                    .ghost()
                    .icon(Icon::new(IconName::RefreshCw).size(px(HEADING_ICON)))
                    .tooltip(say!("pulls-refresh"))
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .child(
                Button::new("pull-browse")
                    .accessibility_label(say!(
                        "pulls-open-at",
                        provider = provider_name(self.link.kind)
                    ))
                    .ghost()
                    .icon(Icon::new(IconName::ExternalLink).size(px(HEADING_ICON)))
                    .tooltip(say!(
                        "pulls-open-at",
                        provider = provider_name(self.link.kind)
                    ))
                    .disabled(url.is_none())
                    .on_click(move |_, _, cx| {
                        if let Some(url) = &url {
                            cx.open_url(url);
                        }
                    }),
            )
            .child(
                Button::new("pull-close")
                    .accessibility_label(say!("pulls-back-to-graph"))
                    .ghost()
                    .icon(Icon::new(IconName::Close).size(px(HEADING_ICON)))
                    .tooltip(say!("pulls-back-to-graph"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PullViewEvent::Close))),
            )
            .into_any_element()
    }

    fn render_title(&self, details: &PullDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let pull = &details.pull;
        let (word, colour) = pill(details, cx);
        let (head, base) = branches_line(pull, self.link.kind);
        let stack_place = self.render_stack_place(cx);
        let title_field = match &self.editing {
            Some(editing::Editing::Title(field)) => Some(field.clone()),
            _ => None,
        };
        let busy = self.sending.is_some();
        let checkout = PullViewEvent::Checkout {
            number: pull.number,
            fork: pull.head_repo.clone().filter(|repo| *repo != pull.repo),
        };
        let checkout_base = PullViewEvent::CheckoutBase(pull.base_ref.clone());
        let open = pull.state == PullState::Open;
        // Two clicks on a branch check it out.
        let branch =
            |id: &'static str, name: String, event: PullViewEvent, cx: &mut Context<Self>| {
                div()
                    .id(id)
                    .min_w_0()
                    .cursor_pointer()
                    .child(branch_chip(name, cx))
                    .tooltip(|window, cx| {
                        Tooltip::new(say!("pulls-branch-check-out")).build(window, cx)
                    })
                    .on_click(cx.listener(move |_, click: &ClickEvent, _, cx| {
                        if click.click_count() >= 2 {
                            cx.emit(event.clone());
                        }
                    }))
                    .test_support()
            };
        let head_chip = branch("pull-head-branch", head, checkout, cx);
        let base_chip = branch("pull-base-branch", base, checkout_base, cx);
        v_flex()
            .w_full()
            .flex_none()
            .pb(px(TITLE_UNDER))
            .border_b_1()
            .border_color(theme.border)
            // The number over the title (and the place in its stack).
            .child(
                h_flex().gap_2().items_center().children(stack_place).child(
                    div()
                        .flex_none()
                        .text_sm()
                        .line_height(px(NUMBER_LINE))
                        .text_color(theme.muted_foreground)
                        .child(number_text(self.link.kind, pull.number)),
                ),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_start()
                    .child(match title_field {
                        Some(field) => h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(Input::new(&field).id("pull-title-field").small()),
                            )
                            .child(
                                Button::new("pull-title-save")
                                    .primary()
                                    .small()
                                    .label(say!("pulls-save"))
                                    .loading(busy)
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_edit(window, cx)
                                    })),
                            )
                            .child(
                                Button::new("pull-title-cancel")
                                    .small()
                                    .label(say!("pulls-cancel"))
                                    .on_click(cx.listener(|this, _, _, cx| this.cancel_edit(cx))),
                            )
                            .into_any_element(),
                        None => h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_1()
                            .items_start()
                            .child(
                                div()
                                    .id("pull-title")
                                    .flex_1()
                                    .min_w_0()
                                    .text_2xl()
                                    .line_height(px(TITLE_LINE))
                                    .whitespace_normal()
                                    // A title of no end leaves the rest its room.
                                    .line_clamp(3)
                                    .child(shown_line(&pull.title, 1024))
                                    .test_support(),
                            )
                            .when(open, |row| {
                                row.child(
                                    Button::new("pull-title-edit")
                                        .accessibility_label(say!("pulls-change-title"))
                                        .ghost()
                                        .xsmall()
                                        .mt(px(4.))
                                        .icon(Icon::new(gpui_kit::assets::IconName::Pencil))
                                        .tooltip(say!("pulls-change-title"))
                                        .disabled(busy)
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.edit_title(window, cx)
                                        })),
                                )
                            })
                            .into_any_element(),
                    }),
            )
            .child(
                h_flex()
                    .mt(px(PILL_ABOVE))
                    .gap_1p5()
                    .items_center()
                    .flex_wrap()
                    .text_sm()
                    .child(
                        div()
                            .id("pull-state")
                            .flex_none()
                            .h(px(PILL_HEIGHT))
                            .line_height(px(PILL_HEIGHT))
                            .px(px(8.))
                            .rounded(px(3.))
                            .bg(colour)
                            .text_color(theme.background)
                            .child(word),
                    )
                    .child(picture(&pull.author, 18., cx))
                    .child(div().child(who(&pull.author)))
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(say!("pulls-wants-to-merge")),
                    )
                    .child(head_chip)
                    .child(
                        div()
                            .text_color(theme.muted_foreground)
                            .child(say!("pulls-into")),
                    )
                    .child(base_chip)
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "· {}",
                                say!("pulls-opened", age = age(&pull.created, self.now))
                            )),
                    ),
            )
            .into_any_element()
    }

    /// A comment: its author's picture, name, what they did and when, over
    /// a tinted box with what they said.
    #[allow(clippy::too_many_arguments)]
    fn render_comment(
        &self,
        id: ElementId,
        author: &User,
        when: &str,
        said: Option<String>,
        text: SharedString,
        menu: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .w_full()
            .gap(px(4.))
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_center()
                    .child(picture(author, 24., cx))
                    .child(div().child(who(author)))
                    .children(said.map(|said| div().text_color(theme.muted_foreground).child(said)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(age(when, self.now)),
                    )
                    .child(div().flex_1())
                    .children(menu),
            )
            .when(!text.is_empty(), |comment| {
                comment.child(
                    div()
                        .w_full()
                        .px(px(20.))
                        .py(px(7.))
                        .rounded(px(3.))
                        .bg(theme.muted)
                        .child(markdown::view(id, text)),
                )
            })
            .into_any_element()
    }

    fn render_thread(&self, at: usize, thread: &Thread, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let texts = self.comments.get(at);
        let hunk = self.hunks.get(at).cloned().unwrap_or_default();
        let place = thread.path.as_ref().map(|path| match thread.line {
            Some(line) => format!("{}:{line}", shown_line(path, 300)),
            None => shown_line(path, 300),
        });
        // A comment on the whole stands as it is; a thread on a line of a
        // file is framed with its place.
        let framed = place.is_some();
        v_flex()
            .id(("pull-thread", at))
            .w_full()
            .flex_none()
            .when(framed, |card| {
                card.rounded(px(6.))
                    .border_1()
                    .border_color(theme.border)
                    .overflow_hidden()
            })
            .when_some(place, |card, place| {
                card.child(
                    h_flex()
                        .w_full()
                        .h(px(28.))
                        .px_2()
                        .gap_1p5()
                        .items_center()
                        .bg(theme.muted)
                        .border_b_1()
                        .border_color(theme.border)
                        .text_xs()
                        .child(Icon::new(IconName::File).size_3p5().flex_none())
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .child(place),
                        )
                        .when(thread.resolved, |line| {
                            line.child(
                                div()
                                    .flex_none()
                                    .ml_auto()
                                    .text_color(theme.success)
                                    .child(say!("pulls-thread-resolved")),
                            )
                        }),
                )
                .when(!hunk.is_empty(), |card| {
                    card.child(
                        v_flex()
                            .w_full()
                            .border_b_1()
                            .border_color(theme.border)
                            .children(hunk.iter().map(|line| patch::render_line(line, cx))),
                    )
                })
            })
            .child(
                v_flex()
                    .w_full()
                    .when(framed, |comments| comments.p_2())
                    .gap(px(ENTRY_GAP))
                    .children(
                        thread
                            .comments
                            .iter()
                            .enumerate()
                            .take(200)
                            .map(|(number, comment): (usize, &Comment)| {
                                let text = texts
                                    .and_then(|texts| texts.get(number))
                                    .cloned()
                                    .unwrap_or_default();
                                let menu = self.comment_menu(
                                    ElementId::Name(
                                        format!("pull-comment-menu:{at}:{number}").into(),
                                    ),
                                    Some(at),
                                    comment.body.clone(),
                                    cx,
                                );
                                self.render_comment(
                                    ElementId::Name(format!("pull-comment:{at}:{number}").into()),
                                    &comment.author,
                                    &comment.created,
                                    None,
                                    text,
                                    Some(menu),
                                    cx,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
            )
            .into_any_element()
    }

    fn render_review(&self, at: usize, review: &Review, cx: &mut Context<Self>) -> AnyElement {
        let text = self.reviews.get(at).cloned().unwrap_or_default();
        let said = super::facts::review_word(review.state).to_owned();
        let menu = self.comment_menu(
            ElementId::Name(format!("pull-review-menu:{at}").into()),
            None,
            review.body.clone(),
            cx,
        );
        v_flex()
            .id(("pull-review", at))
            .w_full()
            .flex_none()
            .child(self.render_comment(
                ElementId::Name(format!("pull-review-text:{at}").into()),
                &review.author,
                review.submitted.as_deref().unwrap_or(""),
                Some(said),
                text,
                Some(menu),
                cx,
            ))
            .into_any_element()
    }

    fn render_left(&self, details: &PullDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        if self.suggesting.is_some() {
            return self.render_suggestions(cx);
        }
        if let Some((at, lines)) = &self.file {
            let file = details.files.get(*at);
            let path = file
                .map(|file| shown_line(&file.path, 400))
                .unwrap_or_default();
            let counts = file
                .map(|file| format!("+{} −{}", file.additions, file.deletions))
                .unwrap_or_default();
            let none = lines.is_empty();
            return v_flex()
                .id("pull-file")
                .size_full()
                .child(
                    h_flex()
                        .w_full()
                        .h(px(32.))
                        .flex_none()
                        .px_2()
                        .gap_2()
                        .items_center()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            Button::new("pull-file-back")
                                .accessibility_label(say!("pulls-back-to-conversation"))
                                .ghost()
                                .small()
                                .icon(IconName::ArrowLeft)
                                .tooltip(say!("pulls-back-to-conversation"))
                                .on_click(cx.listener(|this, _, _, cx| this.open_file(None, cx))),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family.clone())
                                .text_sm()
                                .child(path),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(counts),
                        )
                        .child(div().flex_1())
                        .when(!none, |heading| {
                            heading.child(self.render_suggest_button(cx))
                        }),
                )
                .child(if none {
                    div()
                        .p_3()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-file-no-patch"))
                        .into_any_element()
                } else {
                    div()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .child(
                            patch::render_list("pull-patch", lines.clone())
                                .size_full()
                                .track_scroll(&self.patch_scroll),
                        )
                        .into_any_element()
                })
                .into_any_element();
        }
        let body_empty = details.body.trim().is_empty();
        let body_field = match &self.editing {
            Some(editing::Editing::Body(field)) => Some(field.clone()),
            _ => None,
        };
        let busy = self.sending.is_some();
        let open = details.pull.state == PullState::Open;
        let entries: Vec<AnyElement> = self
            .entries
            .iter()
            .filter_map(|entry| match entry {
                Entry::Thread(at) => details
                    .threads
                    .get(*at)
                    .map(|thread| self.render_thread(*at, thread, cx)),
                Entry::Review(at) => details
                    .reviews
                    .get(*at)
                    .map(|review| self.render_review(*at, review, cx)),
            })
            .collect();
        let talk = entries.len();
        let pencil = (open && body_field.is_none()).then(|| {
            Button::new("pull-edit")
                .accessibility_label(say!("pulls-change-description"))
                .ghost()
                .xsmall()
                .icon(Icon::new(gpui_kit::assets::IconName::Pencil))
                .disabled(busy)
                .tooltip(say!("pulls-change-description"))
                .on_click(cx.listener(|this, _, window, cx| this.edit_body(window, cx)))
                .into_any_element()
        });
        // The description under its heading and over a rule, then the
        // conversation under its own, then the merge box and the field
        // that adds to it.
        v_flex()
            .id("pull-left")
            .flex_1()
            .min_w_0()
            .child(
                v_flex()
                    .id("pull-description")
                    .w_full()
                    .flex_none()
                    .pb(px(18.))
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Self::part_heading(
                        say!("pulls-description"),
                        None,
                        pencil,
                        cx,
                    ))
                    .child(if let Some(field) = &body_field {
                        v_flex()
                            .w_full()
                            .gap_2()
                            .child(
                                div()
                                    .id("pull-body-field")
                                    .w_full()
                                    .h(px(200.))
                                    .overflow_hidden()
                                    .child(Textarea::new(field).h_full())
                                    .test_support(),
                            )
                            .child(
                                h_flex()
                                    .gap_2()
                                    .justify_end()
                                    .child(
                                        Button::new("pull-body-cancel")
                                            .small()
                                            .label(say!("pulls-cancel"))
                                            .on_click(
                                                cx.listener(|this, _, _, cx| this.cancel_edit(cx)),
                                            ),
                                    )
                                    .child(
                                        Button::new("pull-body-save")
                                            .primary()
                                            .small()
                                            .label(say!("pulls-save"))
                                            .loading(busy)
                                            .disabled(busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.save_edit(window, cx)
                                            })),
                                    ),
                            )
                            .into_any_element()
                    } else if body_empty {
                        div()
                            .text_color(theme.muted_foreground)
                            .child(say!("pulls-no-description"))
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .child(markdown::view("pull-body", self.body.clone()))
                            .into_any_element()
                    })
                    .test_support(),
            )
            // The conversation.
            .child(div().mt(px(COMMENTS_ABOVE)).child(Self::part_heading(
                say!("pulls-comments"),
                (talk > 0).then_some(talk),
                None,
                cx,
            )))
            .child(
                v_flex()
                    .mt(px(COMMENTS_UNDER_HEADING))
                    .gap(px(ENTRY_GAP))
                    .when(talk == 0, |entries| {
                        entries.child(
                            div()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(say!("pulls-no-conversation")),
                        )
                    })
                    .children(entries)
                    .child(self.render_merge_box(details, cx))
                    .child(self.render_composer(cx)),
            )
            .test_support()
            .into_any_element()
    }

    /// A heading of a part of the view, with a count when it has one and
    /// a pencil at its end.
    fn part_heading(
        title: &'static str,
        count: Option<usize>,
        pencil: Option<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .w_full()
            .h(px(PART_HEADING))
            .gap(px(6.))
            .items_center()
            .font_weight(FontWeight::MEDIUM)
            .child(title)
            .children(count.map(|count| {
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!("{count}"))
            }))
            .when_some(pencil, |heading, pencil| {
                heading.child(div().flex_1()).child(pencil)
            })
            .into_any_element()
    }

    /// A part of the right column: its heading, with a count when it has
    /// one, and what is under it.
    fn side_part(
        title: &'static str,
        count: Option<usize>,
        lines: Vec<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        Self::side_part_with(title, count, None, lines, cx)
    }

    /// [`Self::side_part`], with a pencil at the end of its heading.
    fn side_part_with(
        title: &'static str,
        count: Option<usize>,
        pencil: Option<AnyElement>,
        lines: Vec<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        // Under its heading and over a rule, as every part of the right
        // column. An empty part has its heading alone.
        let theme = cx.theme();
        v_flex()
            .w_full()
            .pb(px(SIDE_PART_UNDER))
            .gap(px(PART_UNDER_HEADING))
            .border_b_1()
            .border_color(theme.border)
            .child(Self::part_heading(title, count, pencil, cx))
            .children(lines)
            .into_any_element()
    }

    fn render_check(at: usize, check: &Check, now: i64, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let (icon, colour) = ci_icon(check.state, cx);
        let url = check.url.clone().filter(|url| may_open(url));
        let opens = url.is_some();
        let when = check
            .finished
            .as_deref()
            .or(check.started.as_deref())
            .map(|time| age(time, now))
            .unwrap_or_default();
        h_flex()
            .id(("pull-check", at))
            .w_full()
            .gap_1p5()
            .items_center()
            .text_xs()
            .when(opens, |line| {
                line.cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .on_click(move |_, _, cx| {
                        if let Some(url) = &url {
                            cx.open_url(url);
                        }
                    })
            })
            .child(icon.size_3p5().flex_none().text_color(colour))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(shown_line(&check.name, 120)),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(say!(
                        "pulls-check-state-age",
                        state = ci_word(check.state),
                        age = when
                    )),
            )
            .when_some(check.description.clone(), |line, said| {
                let said = shown_line(&said, 300);
                line.tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
            })
            .test_support()
            .into_any_element()
    }

    fn render_side(&self, details: &PullDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open_at = self.file.as_ref().map(|(at, _)| *at);
        // What can be changed has a pencil while it is open.
        let open = details.pull.state == PullState::Open;
        let mut pencil = |listed: Listed| open.then(|| self.pencil(listed, cx));
        let pencils = [
            pencil(Listed::Reviewers),
            pencil(Listed::Assignees),
            pencil(Listed::Labels),
            pencil(Listed::Milestone),
        ];
        let [
            reviewers_pencil,
            assignees_pencil,
            labels_pencil,
            milestone_pencil,
        ] = pencils;
        let files: Vec<AnyElement> = details
            .files
            .iter()
            .enumerate()
            .take(MOST_LISTED)
            .map(|(at, file)| {
                let (letter, colour) = file_mark(file, cx);
                let name = file
                    .path
                    .rsplit('/')
                    .next()
                    .map(|name| shown_line(name, 200))
                    .unwrap_or_default();
                let folder = file
                    .path
                    .rsplit_once('/')
                    .map(|(folder, _)| shown_line(folder, 200));
                h_flex()
                    .id(("pull-file-row", at))
                    .w_full()
                    .h(px(22.))
                    .px_1()
                    .gap_1p5()
                    .items_center()
                    .rounded(px(3.))
                    .text_xs()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .when(open_at == Some(at), |row| row.bg(theme.list_active))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let next =
                            (this.file.as_ref().map(|(open, _)| *open) != Some(at)).then_some(at);
                        this.open_file(next, cx)
                    }))
                    .child(
                        div()
                            .w(px(12.))
                            .flex_none()
                            .font_semibold()
                            .text_color(colour)
                            .child(letter),
                    )
                    .child(div().flex_none().max_w(px(160.)).truncate().child(name))
                    .children(folder.map(|folder| {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(folder)
                    }))
                    .child(
                        h_flex()
                            .flex_none()
                            .ml_auto()
                            .gap_1()
                            .child(
                                div()
                                    .text_color(theme.success)
                                    .child(format!("+{}", file.additions)),
                            )
                            .child(
                                div()
                                    .text_color(theme.danger)
                                    .child(format!("−{}", file.deletions)),
                            ),
                    )
                    .test_support()
                    .into_any_element()
            })
            .collect();
        let commits: Vec<AnyElement> = details
            .commits
            .iter()
            .rev()
            .take(MOST_LISTED)
            .map(|commit| {
                h_flex()
                    .w_full()
                    .gap_1p5()
                    .items_center()
                    .text_xs()
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme.mono_font_family.clone())
                            .text_color(theme.muted_foreground)
                            .child(commit.sha.chars().take(7).collect::<String>()),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(shown_line(&commit.summary, 200)),
                    )
                    .into_any_element()
            })
            .collect();
        let states: Vec<_> = details.checks.iter().map(|check| check.state).collect();
        let mut checks: Vec<AnyElement> = Vec::new();
        if !details.checks.is_empty() {
            checks.push(
                div()
                    .text_xs()
                    .child(checks_summary(&states))
                    .into_any_element(),
            );
        }
        checks.extend(
            details
                .checks
                .iter()
                .enumerate()
                .take(MOST_LISTED)
                .map(|(at, check)| Self::render_check(at, check, self.now, cx)),
        );
        // Labels as bars across the column.
        let labels: Vec<AnyElement> = details
            .pull
            .labels
            .iter()
            .take(MOST_LISTED)
            .map(|label| crate::views::issues::card::label_bar(label, cx))
            .collect();
        let milestone: Vec<AnyElement> = details
            .milestone
            .iter()
            .map(|milestone| {
                // In a tinted box with its due day under it.
                v_flex()
                    .w_full()
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(2.))
                    .bg(theme.muted)
                    .text_sm()
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .child(shown_line(&milestone.title, 120)),
                    )
                    .children(milestone.due.as_deref().and_then(due_date).map(|due| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("pulls-due", due))
                    }))
                    .into_any_element()
            })
            .collect();
        let people = participants(details);
        let participants: Vec<AnyElement> = if people.is_empty() {
            Vec::new()
        } else {
            vec![
                h_flex()
                    .gap(px(6.))
                    .flex_wrap()
                    .children(people.iter().take(40).map(|user| {
                        let said = who(user);
                        div()
                            .id(ElementId::Name(
                                format!("pull-person:{}", user.login).into(),
                            ))
                            .child(picture(user, 24., cx))
                            .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
                    }))
                    .into_any_element(),
            ]
        };
        let branch = vec![
            div()
                .w_full()
                .h(px(20.))
                .px(px(8.))
                .rounded(px(2.))
                .bg(theme.muted)
                .flex()
                .items_center()
                .text_sm()
                .font_family(theme.mono_font_family.clone())
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .child(shown_line(&details.pull.head_ref, 200)),
                )
                .into_any_element(),
        ];
        v_flex()
            .id("pull-side")
            .w(px(SIDE_WIDTH))
            .flex_none()
            .gap(px(16.))
            .child(Self::side_part(
                say!("pulls-files-changed"),
                Some(details.files.len()),
                files,
                cx,
            ))
            .child(Self::side_part(
                say!("pulls-commits"),
                Some(details.commits.len()),
                commits,
                cx,
            ))
            .child(Self::side_part_with(
                say!("pulls-reviewers"),
                None,
                reviewers_pencil,
                reviewer_lines(&details.pull.reviewers, cx),
                cx,
            ))
            .child(Self::side_part_with(
                say!("pulls-assignees"),
                None,
                assignees_pencil,
                assignee_lines(&details.pull.assignees, cx),
                cx,
            ))
            .child(Self::side_part(
                say!("pulls-checks"),
                Some(details.checks.len()),
                checks,
                cx,
            ))
            .child(Self::side_part_with(
                say!("pulls-labels"),
                None,
                labels_pencil,
                labels,
                cx,
            ))
            .child(Self::side_part_with(
                say!("pulls-milestone"),
                None,
                milestone_pencil,
                milestone,
                cx,
            ))
            .child(Self::side_part(
                say!("pulls-participants"),
                Some(people.len()),
                participants,
                cx,
            ))
            .child(Self::side_part(say!("pulls-branch"), None, branch, cx))
            .into_any_element()
    }
}

impl Render for PullView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.ensure_composer(window, cx);
        let theme = cx.theme().clone();
        let heading = self.render_heading(cx);
        let details = self.details.clone();
        let body: AnyElement = match details {
            // A file's changes: the title over the file at the left, the
            // right column beside it scrolling on its own.
            Some(details) if self.file.is_some() => v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .px(px(COLUMN_SIDE))
                .pt(px(COLUMN_TOP))
                .gap(px(UNDER_TITLE))
                .child(self.render_title(&details, cx))
                .child(
                    h_flex()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .items_start()
                        .gap(px(COLUMN_GAP))
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .h_full()
                                .child(self.render_left(&details, cx)),
                        )
                        .child(
                            div()
                                .id("pull-side-room")
                                .relative()
                                .w(px(SIDE_WIDTH + SIDE_SCROLL_ROOM))
                                .h_full()
                                .flex_none()
                                .child(
                                    div()
                                        .id("pull-side-scroll")
                                        .size_full()
                                        .overflow_y_scroll()
                                        .track_scroll(&self.side_scroll)
                                        // Clear of the scroll bar.
                                        .pr(px(SIDE_SCROLL_ROOM))
                                        .child(self.render_side(&details, cx)),
                                )
                                .vertical_scrollbar(&self.side_scroll),
                        ),
                )
                .into_any_element(),
            // One column at most 1200 wide, centred, 40 in from either
            // side, that scrolls as a whole; the title over two columns, the
            // right one 300 wide.
            Some(details) => div()
                .id("pull-room")
                .relative()
                .flex_1()
                .min_h_0()
                .w_full()
                .child(
                    div()
                        .id("pull-page")
                        .size_full()
                        .overflow_y_scroll()
                        .track_scroll(&self.scroll)
                        .px(px(COLUMN_SIDE))
                        .flex()
                        .justify_center()
                        // The column keeps its own height, so that the page
                        // scrolls.
                        .items_start()
                        .child(
                            v_flex()
                                .w_full()
                                .max_w(px(COLUMN_WIDTH))
                                .pt(px(COLUMN_TOP))
                                .pb(px(40.))
                                .gap(px(UNDER_TITLE))
                                .child(self.render_title(&details, cx))
                                .child(
                                    h_flex()
                                        .w_full()
                                        .items_start()
                                        .gap(px(COLUMN_GAP))
                                        .child(self.render_left(&details, cx))
                                        .child(self.render_side(&details, cx)),
                                ),
                        )
                        .test_support(),
                )
                .vertical_scrollbar(&self.scroll)
                .into_any_element(),
            None => div()
                .p_4()
                .text_sm()
                .text_color(if self.error.is_some() {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(match &self.error {
                    Some(error) => shown_line(error, 600),
                    None => say!(
                        "pulls-view-reading",
                        named = number_text(self.link.kind, self.number)
                    ),
                })
                .into_any_element(),
        };
        v_flex()
            .id("pull-view")
            .size_full()
            .bg(theme.background)
            .child(heading)
            .when_some(
                self.write_error
                    .clone()
                    .or_else(|| self.error.clone().filter(|_| self.details.is_some())),
                |view, error| {
                    view.child(
                        div()
                            .px_4()
                            .py_1()
                            .text_xs()
                            .text_color(theme.danger)
                            .child(shown_line(&error, 600)),
                    )
                },
            )
            .child(body)
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use bitbraid_integrations::model::{
        CiState, Comment, Mergeable, PullDetails, PullRequest, PullState, Review, ReviewState,
        Thread, User,
    };

    use super::{Entry, entries_of, participants};

    fn user(login: &str) -> User {
        User {
            login: login.to_owned(),
            ..User::default()
        }
    }

    fn comment(login: &str, created: &str) -> Comment {
        Comment {
            id: created.to_owned(),
            author: user(login),
            body: "said".to_owned(),
            created: created.to_owned(),
            updated: None,
        }
    }

    fn details() -> PullDetails {
        PullDetails {
            pull: PullRequest {
                repo: "octo/tools".to_owned(),
                number: 7,
                title: "t".to_owned(),
                state: PullState::Open,
                draft: false,
                author: user("octo"),
                head_ref: "h".to_owned(),
                head_repo: None,
                head_sha: String::new(),
                base_ref: "main".to_owned(),
                created: "2020-01-01T00:00:00Z".to_owned(),
                updated: "2020-01-01T00:00:00Z".to_owned(),
                labels: Vec::new(),
                reviewers: Vec::new(),
                assignees: vec![user("ann")],
                ci: CiState::None,
                comments: None,
                web_url: String::new(),
            },
            body: String::new(),
            milestone: None,
            mergeable: Mergeable::Unknown,
            merge_methods: Vec::new(),
            checks: Vec::new(),
            commits: Vec::new(),
            files: Vec::new(),
            reviews: vec![
                Review {
                    id: "r1".to_owned(),
                    author: user("bob"),
                    state: ReviewState::Approved,
                    body: "Looks right".to_owned(),
                    submitted: Some("2020-01-02T00:00:00Z".to_owned()),
                },
                Review {
                    id: "r2".to_owned(),
                    author: user("bob"),
                    state: ReviewState::Commented,
                    body: "  ".to_owned(),
                    submitted: Some("2020-01-01T12:00:00Z".to_owned()),
                },
            ],
            threads: vec![
                Thread {
                    id: "t1".to_owned(),
                    path: None,
                    line: None,
                    diff_hunk: None,
                    resolved: false,
                    comments: vec![comment("cy", "2020-01-03T00:00:00Z")],
                },
                Thread {
                    id: "t2".to_owned(),
                    path: Some("src/a.rs".to_owned()),
                    line: Some(3),
                    diff_hunk: None,
                    resolved: true,
                    comments: vec![comment("octo", "2020-01-01T06:00:00Z")],
                },
                Thread {
                    id: "t3".to_owned(),
                    path: None,
                    line: None,
                    diff_hunk: None,
                    resolved: false,
                    comments: Vec::new(),
                },
            ],
            freshness: Default::default(),
        }
    }

    #[test]
    fn the_conversation_is_in_the_order_it_was_written() {
        assert_eq!(
            entries_of(&details()),
            vec![Entry::Thread(1), Entry::Review(0), Entry::Thread(0)]
        );
    }

    #[test]
    fn everyone_who_took_part_is_named_once() {
        let logins: Vec<String> = participants(&details())
            .into_iter()
            .map(|user| user.login)
            .collect();
        assert_eq!(logins, ["octo", "ann", "bob", "cy"]);
    }
}
