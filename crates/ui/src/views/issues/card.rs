//! The card that shows over an issue while the pointer rests on it: on a
//! row of the ISSUES section, and on an issue key in a commit's message,
//! a summary in the graph or a branch's label. It has the issue's title
//! and number, the first lines of its description, where it stands, its
//! labels and milestone, who it is assigned to and who opened it.
//!
//! A key's card needs the issue read first: `issue.show` is asked once
//! for each issue, off the window's thread, and what it answered is kept
//! for the rest of the session ([`Kept`]), so the card shows at once the
//! next time. Until the answer is in, the card says it is being read.
//!
//! Sources: none (original design).

use std::collections::HashMap;

use bitbraid_integrations::model::{
    Issue, IssueDetails, IssueState, Label, ProviderKind, RepoLink,
};
use gpui_kit::component::{ActiveTheme, Icon, IconName, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::facts::{first_lines, number_text, state_word};
use crate::{
    views::pulls::{
        card::{CARD_WIDTH, content_width, who},
        facts::{age, due_date, label_colours, provider_name, shown_line},
    },
    zoom::px,
};

/// The most issues whose answers are kept; past it they are all let go.
const MOST_KEPT: usize = 512;
/// The column at a card's right.
const CARD_SIDE: f32 = 150.0;

/// An issue of a repository at an account, as the cards know it.
pub type IssueRef = (String, String, String, u64);

/// What is known of an issue whose card was asked for.
#[derive(Debug, Clone)]
pub enum Known {
    /// Asked, not answered yet.
    Reading,
    /// What `issue.show` answered.
    Read(Box<IssueDetails>),
    /// Why it could not be read.
    Failed(String),
}

/// The issues read for their cards this session, a GPUI global.
#[derive(Debug, Default)]
pub struct Kept {
    issues: HashMap<IssueRef, Known>,
}

impl Global for Kept {}

impl Kept {
    /// What is known of an issue, without asking for it.
    pub fn get(&self, issue: &IssueRef) -> Option<&Known> {
        self.issues.get(issue)
    }
}

/// What an issue of a link is known by.
pub fn issue_ref(link: &RepoLink, number: u64) -> IssueRef {
    (
        link.account.clone(),
        link.owner.clone(),
        link.name.clone(),
        number,
    )
}

/// Keep what is known of an issue (the issue's view and the tests give
/// what they read).
pub fn keep(link: &RepoLink, number: u64, known: Known, cx: &mut App) {
    let kept = cx.default_global::<Kept>();
    if kept.issues.len() >= MOST_KEPT {
        kept.issues.clear();
    }
    kept.issues.insert(issue_ref(link, number), known);
}

/// What is known of an issue, asking for it when nothing is yet. The
/// windows are drawn again when the answer comes.
pub fn known(link: &RepoLink, number: u64, cx: &mut App) -> Known {
    let key = issue_ref(link, number);
    if let Some(known) = cx
        .try_global::<Kept>()
        .and_then(|kept| kept.issues.get(&key))
    {
        return known.clone();
    }
    keep(link, number, Known::Reading, cx);
    let answer = crate::views::pulls::calls::ask::<IssueDetails>(
        "issue.show",
        json!({
            "account": link.account,
            "owner": link.owner,
            "name": link.name,
            "number": number,
        }),
    );
    let link = link.clone();
    cx.spawn(async move |cx| {
        let answer = answer.await;
        cx.update(|cx| {
            let known = match answer {
                Ok(details) => Known::Read(Box::new(details)),
                Err(problem) => Known::Failed(problem),
            };
            keep(&link, number, known, cx);
            cx.refresh_windows();
        });
    })
    .detach();
    Known::Reading
}

/// The mark of where an issue stands, and its colour.
pub fn state_icon(state: IssueState, cx: &App) -> (Icon, Hsla) {
    let theme = cx.theme();
    match state {
        IssueState::Open => (
            Icon::new(gpui_kit::assets::IconName::CircleDot),
            theme.success,
        ),
        IssueState::Closed => (Icon::new(IconName::CircleCheck), theme.primary),
    }
}

/// The card of an issue the list or `issue.show` answered: the number,
/// quiet, before the title in large type; then at the left the first lines
/// of its description when they are known, and at the right, 150 wide,
/// its labels as bars across that column, its milestone in a tinted box
/// with its due date, and who it is assigned to, each under a small
/// heading; under them where it stands and who opened it when.
pub fn card(
    issue: &Issue,
    body: Option<&str>,
    kind: ProviderKind,
    now: i64,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (_, colour) = state_icon(issue.state, cx);
    let lines = body
        .map(|body| first_lines(body, 4, 120))
        .unwrap_or_default();
    let heading = |text: &'static str| {
        div()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(text)
    };
    let part = |text: &'static str, lines: Vec<AnyElement>| {
        (!lines.is_empty()).then(|| {
            v_flex()
                .w_full()
                .gap(px(6.))
                .child(heading(text))
                .children(lines)
        })
    };
    let labels: Vec<AnyElement> = issue
        .labels
        .iter()
        .take(12)
        .map(|label| label_bar(label, cx))
        .collect();
    let milestone: Vec<AnyElement> = issue
        .milestone
        .iter()
        .map(|milestone| {
            v_flex()
                .w_full()
                .px(px(8.))
                .py(px(6.))
                .gap(px(2.))
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
                        .child(say!("issues-due", due))
                }))
                .into_any_element()
        })
        .collect();
    let assignees: Vec<AnyElement> = issue
        .assignees
        .iter()
        .take(12)
        .map(|user| {
            div()
                .min_w_0()
                .truncate()
                .text_sm()
                .child(shown_line(user.name.as_deref().unwrap_or(&user.login), 60))
                .into_any_element()
        })
        .collect();
    let side_empty = labels.is_empty() && milestone.is_empty() && assignees.is_empty();
    v_flex()
        .w(px(content_width()))
        .gap(px(14.))
        // Number and title.
        .child(
            h_flex()
                .w_full()
                .gap(px(6.))
                .items_baseline()
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(number_text(issue.number)),
                )
                .child(
                    div()
                        .min_w_0()
                        .text_lg()
                        .line_clamp(3)
                        .child(shown_line(&issue.title, 300)),
                ),
        )
        // The description at the left; labels, milestone and assignees
        // at the right.
        .when(!lines.is_empty() || !side_empty, |card| {
            card.child(
                h_flex()
                    .w_full()
                    .items_start()
                    .gap(px(20.))
                    .child(v_flex().flex_1().min_w_0().gap(px(6.)).when(
                        !lines.is_empty(),
                        |left| {
                            left.child(heading(say!("issues-description"))).child(
                                v_flex().gap(px(2.)).children(
                                    lines
                                        .into_iter()
                                        .map(|line| div().text_sm().truncate().child(line)),
                                ),
                            )
                        },
                    ))
                    .when(!side_empty, |row| {
                        row.child(
                            v_flex()
                                .w(px(CARD_SIDE))
                                .flex_none()
                                .gap(px(14.))
                                .children(part(say!("issues-labels"), labels))
                                .children(part(say!("issues-milestone"), milestone))
                                .children(part(say!("issues-assignees"), assignees)),
                        )
                    }),
            )
        })
        // Where it stands and who opened it.
        .child(
            h_flex()
                .gap_1p5()
                .items_center()
                .flex_wrap()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(
                    div()
                        .text_color(colour)
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(state_word(issue.state)),
                )
                .child(format!(
                    "· {} · {}",
                    say!(
                        "issues-opened-by",
                        who = who(&issue.author),
                        age = age(&issue.created, now)
                    ),
                    provider_name(kind),
                )),
        )
        .into_any_element()
}

/// A label as a bar across its column, in its colours.
pub fn label_bar(label: &Label, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let colours = label.color.as_deref().and_then(label_colours);
    div()
        .w_full()
        .h(px(20.))
        .px(px(8.))
        .rounded(px(2.))
        .flex()
        .items_center()
        .text_sm()
        .map(|bar| match colours {
            Some((fill, text)) => bar.bg(rgb(fill)).text_color(rgb(text)),
            None => bar
                .border_1()
                .border_color(theme.border)
                .text_color(theme.foreground),
        })
        .child(
            div()
                .min_w_0()
                .truncate()
                .child(shown_line(&label.name, 60)),
        )
        .into_any_element()
}

/// The card of the issue a key names: read once and kept; until then, a
/// line that says it is being read, or why it could not be.
pub fn key_card(link: &RepoLink, number: u64, cx: &mut App) -> AnyElement {
    let known = known(link, number, cx);
    let now = crate::views::now();
    let theme = cx.theme().clone();
    let line = |text: String| {
        div()
            .max_w(px(CARD_WIDTH))
            .py_1()
            .text_xs()
            .text_color(theme.muted_foreground)
            .whitespace_normal()
            .child(text)
            .into_any_element()
    };
    match known {
        Known::Read(details) => card(&details.issue, Some(&details.body), link.kind, now, cx),
        Known::Reading => line(say!(
            "issues-card-reading",
            named = number_text(number),
            owner = shown_line(&link.owner, 80),
            name = shown_line(&link.name, 80)
        )),
        Known::Failed(problem) => line(say!(
            "issues-card-failed",
            named = number_text(number),
            owner = shown_line(&link.owner, 80),
            name = shown_line(&link.name, 80),
            problem = shown_line(&problem, 300)
        )),
    }
}
