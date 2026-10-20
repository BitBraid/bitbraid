//! An issue's view: it takes the graph's place and the right panel's,
//! opened from a row of the ISSUES section or from an issue key in a
//! commit's message, and goes away with its close button. It is read
//! only: issues are changed at the provider.
//!
//! At the top a heading 52 high with the provider's name and buttons that
//! read it again, open it in the system's browser and close the view.
//! Under it a column at most 1200 wide, centred, that scrolls as a whole:
//! the number over the title, a pill of where it stands, and who opened
//! it and when; then at the left the description drawn from its Markdown
//! and the comments, oldest first, each in a box under its author; at the
//! right, 300 wide, where it stands as a bar, who it is assigned to, its
//! labels, the local branches made for it (whose names hold its number)
//! with a button that makes one and one that checks each out, and its
//! milestone, each under its heading and over a rule.
//!
//! Sources: docs.github.com/en/rest/issues; docs.gitlab.com/api/issues.

use bitbraid_integrations::model::{Comment, IssueDetails, IssueState, RepoLink};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{
    card::{self, Known, state_icon},
    facts::{branches_for, number_text, state_word},
};
use crate::{
    session::{Reply, SessionHandle},
    views::pulls::{
        calls,
        card::{picture, who},
        facts::{age, due_date, may_open, provider_name, shown_line},
        markdown,
        section::provider_mark,
    },
    zoom::px,
};

/// The view's measures: the heading's height; the column the issue is
/// drawn in, at most this wide and this far from either side; the room
/// between its two columns and the right one's width; the height of a
/// part's heading.
const HEADING_HEIGHT: f32 = 52.0;
const COLUMN_WIDTH: f32 = 1200.0;
const COLUMN_SIDE: f32 = 40.0;
const COLUMN_GAP: f32 = 40.0;
const SIDE_WIDTH: f32 = 300.0;
const PART_HEADING: f32 = 24.0;
/// The size of the heading's buttons.
const HEADING_ICON: f32 = 18.0;
/// From the heading to the number, the number's line, the title's, the
/// room over the pill and its height, and from the pill to the rule under
/// them; then from that rule to the two columns.
const COLUMN_TOP: f32 = 21.0;
const NUMBER_LINE: f32 = 20.0;
const TITLE_LINE: f32 = 34.0;
const PILL_ABOVE: f32 = 7.0;
const PILL_HEIGHT: f32 = 19.0;
const TITLE_UNDER: f32 = 5.0;
const UNDER_TITLE: f32 = 6.0;
/// From a part's heading to what is under it; from the description's rule
/// to the comments' heading; from one comment to the next, and from their
/// heading to the first; from what a part of the right column holds to its
/// rule; the height of the state's bar, and of the button that makes a
/// branch.
const PART_UNDER_HEADING: f32 = 11.0;
const COMMENTS_ABOVE: f32 = 19.0;
const COMMENT_GAP: f32 = 19.0;
const COMMENTS_UNDER_HEADING: f32 = 6.0;
const SIDE_PART_UNDER: f32 = 21.0;
const BAR_HEIGHT: f32 = 20.0;
const BRANCH_BUTTON: f32 = 30.0;
/// The most comments and entries of a list that are drawn.
const MOST_LISTED: usize = 200;

/// What the view asks of the window.
#[derive(Debug, Clone)]
pub enum IssueViewEvent {
    /// Put the view away: the graph comes back.
    Close,
    /// Make a branch for the issue: the bar that asks names it.
    Branch {
        /// Where it is.
        link: RepoLink,
        issue: Box<bitbraid_integrations::model::Issue>,
    },
    /// Check out this local branch.
    Checkout(String),
}

/// What the view has on show, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueViewProbe {
    /// The issue's number, and the repository's `owner/name`.
    pub number: u64,
    pub repository: String,
    /// Its title, once read.
    pub title: Option<String>,
    /// Where it stands, once read.
    pub state: Option<String>,
    /// Whether it is being read.
    pub loading: bool,
    /// What went wrong, when it could not be read.
    pub error: Option<String>,
    /// How many comments it has.
    pub comments: usize,
    /// The local branches made for it.
    pub branches: Vec<String>,
    /// What its heading says when it was kept from before because the
    /// provider could not be reached; none when it was read.
    pub offline: Option<String>,
}

/// The view.
pub struct IssueView {
    link: RepoLink,
    number: u64,
    details: Option<IssueDetails>,
    error: Option<String>,
    loading: bool,
    /// The details were given (tests): nothing is read.
    given: bool,
    /// The description and the comments, made ready to be drawn.
    body: SharedString,
    comments: Vec<SharedString>,
    /// The repository's local branches, and the one checked out.
    local: Vec<String>,
    current: Option<String>,
    /// Which reading the answer belongs to.
    generation: u64,
    now: i64,
    scroll: ScrollHandle,
}

impl EventEmitter<IssueViewEvent> for IssueView {}

impl IssueView {
    /// The view of issue `number` of the repository `link` names, among
    /// these local branches; it is read at once.
    pub fn new(
        link: RepoLink,
        number: u64,
        local: Vec<String>,
        current: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self {
            link,
            number,
            details: None,
            error: None,
            loading: false,
            given: false,
            body: SharedString::default(),
            comments: Vec::new(),
            local,
            current,
            generation: 0,
            now: crate::views::now(),
            scroll: ScrollHandle::new(),
        };
        // What a card read already shows at once, and is read again.
        if let Some(Known::Read(details)) = cx
            .try_global::<card::Kept>()
            .and_then(|kept| kept.get(&card::issue_ref(&view.link, number)))
            .cloned()
        {
            view.take(*details);
        }
        view.load(false, cx);
        view
    }

    /// Which issue it shows.
    pub fn shows(&self) -> (&RepoLink, u64) {
        (&self.link, self.number)
    }

    /// The repository's local branches changed.
    pub fn set_branches(
        &mut self,
        local: Vec<String>,
        current: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if local != self.local || current != self.current {
            self.local = local;
            self.current = current;
            cx.notify();
        }
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
        let reply = calls::run(
            "issue.show",
            json!({
                "account": self.link.account,
                "owner": self.link.owner,
                "name": self.link.name,
                "number": self.number,
                "fresh": fresh,
            }),
        );
        SessionHandle::deliver(
            reply,
            "issue.show".to_owned(),
            cx,
            move |this, reply, cx| {
                if this.generation != generation {
                    return;
                }
                this.loading = false;
                match reply {
                    Reply::Command(value) => match serde_json::from_value::<IssueDetails>(value) {
                        Ok(details) => {
                            card::keep(
                                &this.link,
                                this.number,
                                Known::Read(Box::new(details.clone())),
                                cx,
                            );
                            this.take(details);
                        }
                        Err(_) => this.error = Some(say!("issues-reply-unreadable").to_owned()),
                    },
                    Reply::Error(err) => this.error = Some(crate::providers::said(&err)),
                    _ => {}
                }
                cx.notify();
            },
        );
        cx.notify();
    }

    /// Read it again, past what was kept.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load(true, cx);
    }

    /// Hold what was read, and make its texts ready to be drawn.
    fn take(&mut self, details: IssueDetails) {
        self.error = None;
        self.body = markdown::prepared(&details.body);
        self.comments = details
            .comments
            .iter()
            .take(MOST_LISTED)
            .map(|comment| markdown::prepared(&comment.body))
            .collect();
        self.details = Some(details);
    }

    /// Show these details in place of what the provider would answer, and
    /// read nothing: for the off-screen tests.
    pub fn give(&mut self, details: IssueDetails, cx: &mut Context<Self>) {
        self.given = true;
        self.generation = self.generation.wrapping_add(1);
        self.loading = false;
        self.now = crate::views::now();
        self.take(details);
        cx.notify();
    }

    /// The local branches made for the issue.
    fn branches(&self) -> Vec<String> {
        branches_for(&self.local, self.number)
    }

    /// Ask for a branch for the issue, once it was read: what the button
    /// of the right column does.
    pub fn ask_branch(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(details) = &self.details else {
            return false;
        };
        cx.emit(IssueViewEvent::Branch {
            link: self.link.clone(),
            issue: Box::new(details.issue.clone()),
        });
        true
    }

    /// What the view has on show.
    pub fn probe(&self) -> IssueViewProbe {
        let details = self.details.as_ref();
        IssueViewProbe {
            number: self.number,
            repository: format!("{}/{}", self.link.owner, self.link.name),
            title: details.map(|details| details.issue.title.clone()),
            state: details.map(|details| state_word(details.issue.state).to_owned()),
            loading: self.loading,
            error: self.error.clone(),
            comments: details.map_or(0, |details| details.comments.len()),
            branches: self.branches(),
            offline: self.offline(),
        }
    }

    /// What the heading says when what is shown was kept from before.
    fn offline(&self) -> Option<String> {
        crate::views::pulls::facts::offline_said(&self.details.as_ref()?.freshness, self.now)
    }

    fn render_heading(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let offline = self.offline();
        let url = self
            .details
            .as_ref()
            .map(|details| details.issue.web_url.clone())
            .filter(|url| may_open(url));
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
            .child(div().text_xl().child(match self.link.kind {
                bitbraid_integrations::model::ProviderKind::Github => say!("issues-view-github"),
                bitbraid_integrations::model::ProviderKind::Gitlab => say!("issues-view-gitlab"),
            }))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(format!(
                        "{}/{}",
                        shown_line(&self.link.owner, 120),
                        shown_line(&self.link.name, 120),
                    )),
            )
            .child(div().flex_1())
            .when_some(offline, |heading, said| {
                heading.child(crate::views::pulls::view::offline_mark(
                    "issue-offline",
                    said,
                    cx,
                ))
            })
            .child(
                Button::new("issue-refresh")
                    .accessibility_label(say!("issues-refresh"))
                    .ghost()
                    .icon(Icon::new(IconName::RefreshCw).size(px(HEADING_ICON)))
                    .tooltip(say!("issues-refresh"))
                    .disabled(self.loading)
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .child(
                Button::new("issue-browse")
                    .accessibility_label(say!(
                        "issues-open-at",
                        provider = provider_name(self.link.kind)
                    ))
                    .ghost()
                    .icon(Icon::new(IconName::ExternalLink).size(px(HEADING_ICON)))
                    .tooltip(say!(
                        "issues-open-at",
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
                Button::new("issue-close")
                    .accessibility_label(say!("issues-back-to-graph"))
                    .ghost()
                    .icon(Icon::new(IconName::Close).size(px(HEADING_ICON)))
                    .tooltip(say!("issues-back-to-graph"))
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(IssueViewEvent::Close))),
            )
            .into_any_element()
    }

    /// The number over the title, the title, and a line with where it
    /// stands and who opened it when.
    fn render_title(&self, details: &IssueDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let issue = &details.issue;
        let (_, colour) = state_icon(issue.state, cx);
        v_flex()
            .w_full()
            .flex_none()
            .pb(px(TITLE_UNDER))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .text_sm()
                    .line_height(px(NUMBER_LINE))
                    .text_color(theme.muted_foreground)
                    .child(number_text(issue.number)),
            )
            .child(
                div()
                    .id("issue-title")
                    .w_full()
                    .text_2xl()
                    .line_height(px(TITLE_LINE))
                    .whitespace_normal()
                    .line_clamp(3)
                    .child(shown_line(&issue.title, 1024))
                    .test_support(),
            )
            .child(
                h_flex()
                    .mt(px(PILL_ABOVE))
                    .gap(px(10.))
                    .items_center()
                    .flex_wrap()
                    .text_sm()
                    .child(
                        div()
                            .id("issue-state")
                            .flex_none()
                            .h(px(PILL_HEIGHT))
                            .line_height(px(PILL_HEIGHT))
                            .px(px(8.))
                            .rounded(px(3.))
                            .bg(colour)
                            .text_color(theme.background)
                            .child(state_word(issue.state)),
                    )
                    .child(picture(&issue.author, 18., cx))
                    .child(div().child(who(&issue.author)))
                    .child(div().text_color(theme.muted_foreground).child(say!(
                        "issues-opened-it",
                        age = age(&issue.created, self.now)
                    ))),
            )
            .into_any_element()
    }

    /// A heading of a part of the view.
    fn part_heading(title: &'static str, count: Option<usize>, cx: &App) -> AnyElement {
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
            .into_any_element()
    }

    /// A comment: its author's picture, name and age, and under them what
    /// it says in a box of its own.
    fn render_comment(
        &self,
        at: usize,
        comment: &Comment,
        text: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        v_flex()
            .id(("issue-comment", at))
            .w_full()
            .flex_none()
            .gap(px(4.))
            .child(
                h_flex()
                    .gap(px(10.))
                    .items_center()
                    .child(picture(&comment.author, 24., cx))
                    .child(div().child(who(&comment.author)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(theme.muted_foreground)
                            .child(age(&comment.created, self.now)),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .px(px(20.))
                    .py(px(7.))
                    .rounded(px(3.))
                    .bg(theme.muted)
                    .child(markdown::view(
                        ElementId::Name(format!("issue-comment-text:{at}").into()),
                        text,
                    )),
            )
            .into_any_element()
    }

    /// The description and the comments.
    fn render_left(&self, details: &IssueDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let body_empty = details.body.trim().is_empty();
        let comments: Vec<AnyElement> = details
            .comments
            .iter()
            .enumerate()
            .take(MOST_LISTED)
            .map(|(at, comment)| {
                let text = self.comments.get(at).cloned().unwrap_or_default();
                self.render_comment(at, comment, text, cx)
            })
            .collect();
        let count = details.comments.len();
        v_flex()
            .id("issue-left")
            .flex_1()
            .min_w_0()
            .child(
                v_flex()
                    .id("issue-description")
                    .w_full()
                    .flex_none()
                    .pb(px(18.))
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(Self::part_heading(say!("issues-description"), None, cx))
                    .child(if body_empty {
                        div()
                            .text_color(theme.muted_foreground)
                            .child(say!("issues-no-description"))
                            .into_any_element()
                    } else {
                        div()
                            .w_full()
                            .child(markdown::view("issue-body", self.body.clone()))
                            .into_any_element()
                    })
                    .test_support(),
            )
            .child(div().mt(px(COMMENTS_ABOVE)).child(Self::part_heading(
                say!("issues-comments"),
                (count > 0).then_some(count),
                cx,
            )))
            .when(count == 0, |left| {
                left.child(
                    div()
                        .mt(px(PART_UNDER_HEADING))
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(say!("issues-no-comments")),
                )
            })
            .child(
                v_flex()
                    .mt(px(COMMENTS_UNDER_HEADING))
                    .gap(px(COMMENT_GAP))
                    .children(comments),
            )
            .into_any_element()
    }

    /// A part of the right column: its heading, what is under it, and a
    /// rule under that.
    fn side_part(
        title: &'static str,
        count: Option<usize>,
        lines: Vec<AnyElement>,
        cx: &App,
    ) -> AnyElement {
        // An empty part has its heading alone.
        let theme = cx.theme();
        v_flex()
            .w_full()
            .pb(px(SIDE_PART_UNDER))
            .gap(px(PART_UNDER_HEADING))
            .border_b_1()
            .border_color(theme.border)
            .child(Self::part_heading(title, count, cx))
            .children(lines)
            .into_any_element()
    }

    fn render_side(&self, details: &IssueDetails, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let issue = &details.issue;
        let (_, colour) = state_icon(issue.state, cx);
        // Where it stands, as a bar across the column in its colour.
        let state = vec![
            div()
                .w_full()
                .h(px(BAR_HEIGHT))
                .line_height(px(BAR_HEIGHT))
                .text_sm()
                .px(px(8.))
                .rounded(px(2.))
                .bg(colour)
                .text_color(theme.background)
                .child(match issue.state {
                    IssueState::Open => say!("issues-state-open").to_owned(),
                    IssueState::Closed => {
                        say!("issues-closed-updated", age = age(&issue.updated, self.now))
                    }
                })
                .into_any_element(),
        ];
        let assignees: Vec<AnyElement> = issue
            .assignees
            .iter()
            .take(MOST_LISTED)
            .map(|user| {
                // The name, and the login after it, quiet; the login
                // alone without one.
                h_flex()
                    .gap(px(10.))
                    .items_center()
                    .min_w_0()
                    .child(picture(user, 24., cx))
                    .child(
                        div().min_w_0().truncate().child(
                            user.name
                                .as_deref()
                                .map_or_else(|| who(user), |name| shown_line(name, 60)),
                        ),
                    )
                    .when(user.name.is_some(), |line| {
                        line.child(
                            div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child(format!("({})", who(user))),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let labels: Vec<AnyElement> = issue
            .labels
            .iter()
            .take(MOST_LISTED)
            .map(|label| card::label_bar(label, cx))
            .collect();
        let milestone: Vec<AnyElement> = issue
            .milestone
            .iter()
            .map(|milestone| {
                v_flex()
                    .w_full()
                    .px(px(8.))
                    .py(px(6.))
                    .rounded(px(2.))
                    .bg(theme.muted)
                    .child(
                        h_flex()
                            .gap(px(6.))
                            .items_center()
                            .child(
                                Icon::new(gpui_kit::assets::IconName::Milestone)
                                    .size_3p5()
                                    .flex_none(),
                            )
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .child(shown_line(&milestone.title, 120)),
                            ),
                    )
                    .children(milestone.due.as_deref().and_then(due_date).map(|due| {
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("issues-due", due))
                    }))
                    .into_any_element()
            })
            .collect();
        let made = self.branches();
        let mut branches: Vec<AnyElement> = made
            .iter()
            .take(MOST_LISTED)
            .enumerate()
            .map(|(at, name)| {
                let here = self.current.as_deref() == Some(name.as_str());
                let target = name.clone();
                h_flex()
                    .id(("issue-branch", at))
                    .w_full()
                    .h(px(24.))
                    .gap_1p5()
                    .items_center()
                    .text_sm()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::GitBranch)
                            .size_3p5()
                            .flex_none()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .font_family(theme.mono_font_family.clone())
                            .when(here, |name| name.font_semibold())
                            .child(shown_line(name, 200)),
                    )
                    .child(if here {
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("issues-branch-checked-out"))
                            .into_any_element()
                    } else {
                        Button::new(("issue-branch-checkout", at))
                            .ghost()
                            .xsmall()
                            .label(say!("issues-branch-check-out"))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(IssueViewEvent::Checkout(target.clone()))
                            }))
                            .into_any_element()
                    })
                    .into_any_element()
            })
            .collect();
        branches.push(
            h_flex()
                .child(
                    Button::new("issue-create-branch")
                        .outline()
                        .small()
                        .h(px(BRANCH_BUTTON))
                        .icon(Icon::new(gpui_kit::assets::IconName::GitBranch))
                        .label(say!("issues-create-branch"))
                        .tooltip(say!("issues-create-branch-tooltip"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ask_branch(cx);
                        })),
                )
                .into_any_element(),
        );
        v_flex()
            .id("issue-side")
            .w(px(SIDE_WIDTH))
            .flex_none()
            .gap(px(16.))
            .child(Self::side_part(say!("issues-status"), None, state, cx))
            .child(Self::side_part(
                say!("issues-assignees"),
                None,
                assignees,
                cx,
            ))
            .child(Self::side_part(say!("issues-labels"), None, labels, cx))
            .child(Self::side_part(
                say!("issues-branches"),
                (!made.is_empty()).then_some(made.len()),
                branches,
                cx,
            ))
            .child(Self::side_part(
                say!("issues-milestone"),
                None,
                milestone,
                cx,
            ))
            .into_any_element()
    }
}

impl Render for IssueView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let heading = self.render_heading(cx);
        let details = self.details.clone();
        // One column at most 1200 wide, centred, 40 in from either side,
        // that scrolls as a whole; the title over two columns, the right
        // one 300 wide.
        let column: AnyElement = match details {
            Some(details) => v_flex()
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
                )
                .into_any_element(),
            None => div()
                .w_full()
                .max_w(px(COLUMN_WIDTH))
                .pt(px(COLUMN_TOP))
                .text_sm()
                .text_color(if self.error.is_some() {
                    theme.danger
                } else {
                    theme.muted_foreground
                })
                .child(match &self.error {
                    Some(error) => shown_line(error, 600),
                    None => say!("issues-view-reading", named = number_text(self.number)),
                })
                .into_any_element(),
        };
        v_flex()
            .id("issue-view")
            .size_full()
            .bg(theme.background)
            .child(heading)
            .when_some(
                self.error.clone().filter(|_| self.details.is_some()),
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
            .child(
                div()
                    .id("issue-room")
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        div()
                            .id("issue-scroll")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .px(px(COLUMN_SIDE))
                            .flex()
                            .justify_center()
                            // The column keeps its own height, so that the
                            // page scrolls.
                            .items_start()
                            .child(column),
                    )
                    .vertical_scrollbar(&self.scroll),
            )
            .test_support()
    }
}
