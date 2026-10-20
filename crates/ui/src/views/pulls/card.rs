//! The card that shows beside a pull request's row while the pointer
//! rests on it: its title and number; who opened it and which branch is
//! to go into which; its checks summed up and one a line; its reviewers
//! with what each said, and its assignees; its labels in their colours;
//! and when it was opened and last changed. A GitLab merge request has
//! the same card, with what GitLab answered.
//!
//! It is 500 wide, its left edge a little over the panel's right edge,
//! its middle at the row's middle (kept inside the window), the parts one
//! under the other without rules between them.
//!
//! Also the parts of it the section's rows and the pull request's view
//! draw too: a person's picture, a label's chip, the mark of a row.
//!
//! Sources: gpui-kit documentation (hover card).

use bitbraid_integrations::model::{
    CommitStatus, Label, ProviderKind, PullRequest, Reviewer, User,
};
use gpui_kit::base::HoverCard;
use gpui_kit::component::{ActiveTheme, Icon, IconName, ThemeStyled, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{
    checks::{check_lines, ci_icon},
    facts::{
        StateMark, age, checks_summary, ci_word, label_colours, number_text, review_word,
        shown_line,
    },
};
use crate::zoom::px;

/// How wide a card is, and the room inside its edges.
pub const CARD_WIDTH: f32 = 500.0;
const CARD_PADDING_X: f32 = 20.0;
const CARD_PADDING_Y: f32 = 16.0;
/// How far a card stands over the right edge of the row it is for.
const CARD_OVERLAP: f32 = 8.0;
/// The room between the parts of a card.
const CARD_GAP: f32 = 14.0;

/// A row with its card: after the pointer rests on the row a moment, the
/// card stands beside it, its left edge a little over the row's right
/// edge and its middle at the row's middle; it stays while the pointer
/// is on the row or on the card.
pub fn with_card(
    id: impl Into<ElementId>,
    row: impl IntoElement + 'static,
    content: impl FnOnce(&mut Window, &mut App) -> AnyElement + 'static,
) -> AnyElement {
    let theme_card = |cx: &App| {
        div()
            .w(px(CARD_WIDTH))
            .px(px(CARD_PADDING_X))
            .py(px(CARD_PADDING_Y))
            .popover_style(cx)
    };
    HoverCard::new(id)
        .anchor(Anchor::LeftCenter)
        .trigger(row)
        .content(move |_, window, cx| {
            let inner = content(window, cx);
            // The toolkit puts this frame's left edge at the row's right
            // edge; the card in it reaches back over the row by a margin,
            // which the frame's own box does not count.
            div().id("hover-card-frame").child(
                theme_card(cx)
                    .id("hover-card")
                    .ml(px(-CARD_OVERLAP))
                    .child(inner)
                    .test_support(),
            )
        })
        .into_any_element()
}

/// The width a card's content has inside its edges.
pub fn content_width() -> f32 {
    CARD_WIDTH - 2. * CARD_PADDING_X
}

/// A person's picture: the provider's, where pictures from the providers
/// are asked for and it is in (`crate::avatars`); else the pattern made
/// on this machine from their name at the provider, in a circle.
pub fn picture(user: &User, size: f32, cx: &App) -> AnyElement {
    let dark = cx.theme().is_dark();
    if let Some(found) = user
        .avatar_url
        .as_deref()
        .and_then(|address| crate::avatars::picture_at(address, size > 24., cx))
    {
        return img(found)
            .size(px(size))
            .flex_none()
            .rounded_full()
            .into_any_element();
    }
    div()
        .size(px(size))
        .flex_none()
        .rounded_full()
        .overflow_hidden()
        .bg(cx.theme().muted)
        .flex()
        .items_center()
        .justify_center()
        .child(crate::views::avatar::identicon(
            &user.login,
            px(size * 0.7),
            dark,
        ))
        .into_any_element()
}

/// A person's name as it is shown: their login.
pub fn who(user: &User) -> String {
    shown_line(&user.login, 60)
}

/// A label as a chip in its colour, its name in a colour that reads on
/// it; one without a colour is outlined.
pub fn label_chip(label: &Label, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let colours = label.color.as_deref().and_then(label_colours);
    div()
        .flex_none()
        .h(px(18.))
        .px(px(6.))
        .rounded(px(9.))
        .flex()
        .items_center()
        .text_xs()
        .max_w(px(160.))
        .map(|chip| match colours {
            Some((fill, text)) => chip.bg(rgb(fill)).text_color(rgb(text)),
            None => chip
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

/// The mark of a row: an icon and its colour.
pub fn state_icon(mark: StateMark, cx: &App) -> (Icon, Hsla) {
    use gpui_kit::assets::IconName as Extra;
    let theme = cx.theme();
    match mark {
        // A tick, a hollow circle and a cross; the colours are the
        // theme's.
        StateMark::Ready => (Icon::new(IconName::Check), theme.success),
        StateMark::Waiting => (Icon::new(Extra::Circle), theme.warning),
        StateMark::Failing => (Icon::new(IconName::Close), theme.danger),
        StateMark::Merged => (Icon::new(Extra::GitMerge), theme.primary),
        StateMark::Closed => (Icon::new(Extra::GitPullRequestClosed), theme.danger),
    }
}

/// Which branch goes into which, each with its repository:
/// `octo/tools/feature` into `octo/tools/main`, a branch of a fork with
/// the fork.
pub fn branches_line(pull: &PullRequest, _kind: ProviderKind) -> (String, String) {
    let head_repo = pull.head_repo.as_deref().unwrap_or(&pull.repo);
    (
        format!(
            "{}/{}",
            shown_line(head_repo, 120),
            shown_line(&pull.head_ref, 120)
        ),
        format!(
            "{}/{}",
            shown_line(&pull.repo, 120),
            shown_line(&pull.base_ref, 120)
        ),
    )
}

/// A branch's name set apart, as a chip in the code face.
pub fn branch_chip(name: String, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .min_w_0()
        .px(px(5.))
        .py(px(1.))
        .rounded(px(3.))
        .bg(theme.muted)
        .font_family(theme.mono_font_family.clone())
        .text_xs()
        .truncate()
        .child(name)
        .into_any_element()
}

/// One person of a list: picture, name, and what they said.
fn person_line(user: &User, said: Option<&'static str>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .gap(px(10.))
        .items_center()
        .min_w_0()
        .text_sm()
        .child(picture(user, 18., cx))
        .child(div().min_w_0().truncate().child(who(user)))
        .children(said.map(|said| {
            div()
                .flex_none()
                .text_color(theme.muted_foreground)
                .child(said)
        }))
        .into_any_element()
}

/// People under a heading in the rows' colour, one under another;
/// nothing when there is no one.
fn people(heading: &'static str, lines: Vec<AnyElement>) -> Option<AnyElement> {
    (!lines.is_empty()).then(|| {
        v_flex()
            .flex_1()
            .flex_basis(relative(0.))
            .min_w_0()
            .gap(px(8.))
            .child(div().text_base().child(heading))
            .children(lines)
            .into_any_element()
    })
}

/// The reviewers and the assignees side by side, each in half the card;
/// one alone keeps the left half.
fn people_row(columns: Vec<AnyElement>) -> Option<AnyElement> {
    let alone = columns.len() == 1;
    (!columns.is_empty()).then(|| {
        h_flex()
            .w_full()
            .gap(px(20.))
            .items_start()
            .children(columns)
            .when(alone, |row| {
                row.child(div().flex_1().flex_basis(relative(0.)))
            })
            .into_any_element()
    })
}

/// The reviewers of a pull request as lines of a column.
pub fn reviewer_lines(reviewers: &[Reviewer], cx: &App) -> Vec<AnyElement> {
    reviewers
        .iter()
        .take(12)
        .map(|reviewer| person_line(&reviewer.user, Some(review_word(reviewer.state)), cx))
        .collect()
}

/// The assignees of a pull request as lines of a column.
pub fn assignee_lines(assignees: &[User], cx: &App) -> Vec<AnyElement> {
    assignees
        .iter()
        .take(12)
        .map(|user| person_line(user, None, cx))
        .collect()
}

/// What the card of a pull request holds, with the checks of its head
/// commit when they are known: the title in large type with the number
/// after it, quiet; who wants which branch merged into which; the checks
/// summed up and one a line; the reviewers and the assignees side by side,
/// each under a heading when there are any; the labels under theirs; when
/// it was opened and when it last changed, a line each.
pub fn card(
    pull: &PullRequest,
    kind: ProviderKind,
    checks: Option<&CommitStatus>,
    now: i64,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let (head, base) = branches_line(pull, kind);
    let (ci, ci_colour) = ci_icon(pull.ci, cx);
    let states: Vec<_> = checks
        .map(|status| status.checks.iter().map(|check| check.state).collect())
        .unwrap_or_default();
    let summary = if states.is_empty() {
        say!("pulls-checks-word", word = ci_word(pull.ci))
    } else {
        checks_summary(&states)
    };
    v_flex()
        .w(px(content_width()))
        .gap(px(CARD_GAP))
        // Title and number.
        .child(
            h_flex()
                .w_full()
                .gap(px(6.))
                .items_baseline()
                .child(
                    div()
                        .min_w_0()
                        .text_lg()
                        .line_clamp(3)
                        .child(shown_line(&pull.title, 300)),
                )
                .child(
                    div()
                        .flex_none()
                        .text_sm()
                        .text_color(theme.muted_foreground)
                        .child(number_text(kind, pull.number)),
                ),
        )
        // Who wants what merged into what.
        .child(
            h_flex()
                .gap(px(6.))
                .items_center()
                .flex_wrap()
                .text_sm()
                .child(picture(&pull.author, 18., cx))
                .child(div().child(who(&pull.author)))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-wants-to-merge")),
                )
                .child(branch_chip(head, cx))
                .child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-into")),
                )
                .child(branch_chip(base, cx)),
        )
        // Checks.
        .child(
            v_flex()
                .gap_1()
                .child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .text_sm()
                        .child(ci.size_3p5().flex_none().text_color(ci_colour))
                        .child(div().child(summary)),
                )
                .children(
                    checks
                        .map(|status| check_lines(&status.checks, now, cx))
                        .unwrap_or_default(),
                ),
        )
        .children(people_row(
            people(say!("pulls-reviewers"), reviewer_lines(&pull.reviewers, cx))
                .into_iter()
                .chain(people(
                    say!("pulls-assignees"),
                    assignee_lines(&pull.assignees, cx),
                ))
                .collect(),
        ))
        .when(!pull.labels.is_empty(), |card| {
            card.child(
                v_flex()
                    .gap(px(8.))
                    .child(div().text_base().child(say!("pulls-labels")))
                    .child(
                        h_flex().gap_1().flex_wrap().children(
                            pull.labels
                                .iter()
                                .take(20)
                                .map(|label| label_chip(label, cx)),
                        ),
                    ),
            )
        })
        .child(
            v_flex()
                .gap(px(4.))
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(say!("pulls-card-opened", age = age(&pull.created, now)))
                .child(say!("pulls-card-updated", age = age(&pull.updated, now))),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::{CiState, ProviderKind, PullState};

    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use bitbraid_integrations::model::{PullRequest, User};

    use super::branches_line;

    #[test]
    fn the_branches_are_named_with_their_repositories() {
        let mut pull = PullRequest {
            repo: "group/sub/tools".to_owned(),
            number: 1,
            title: String::new(),
            state: PullState::Open,
            draft: false,
            author: User::default(),
            head_ref: "topic".to_owned(),
            head_repo: None,
            head_sha: String::new(),
            base_ref: "main".to_owned(),
            created: String::new(),
            updated: String::new(),
            labels: Vec::new(),
            reviewers: Vec::new(),
            assignees: Vec::new(),
            ci: CiState::None,
            comments: None,
            web_url: String::new(),
        };
        for kind in [ProviderKind::Github, ProviderKind::Gitlab] {
            assert_eq!(
                branches_line(&pull, kind),
                (
                    "group/sub/tools/topic".to_owned(),
                    "group/sub/tools/main".to_owned()
                )
            );
        }
        pull.head_repo = Some("me/tools".to_owned());
        assert_eq!(
            branches_line(&pull, ProviderKind::Github).0,
            "me/tools/topic"
        );
        pull.repo = "odd".to_owned();
        pull.head_repo = None;
        assert_eq!(branches_line(&pull, ProviderKind::Github).1, "odd/main");
    }
}
