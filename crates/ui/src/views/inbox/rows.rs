//! The list of the inbox: the pull requests in their groups, each a
//! heading that folds and a table under it; the flat tables of the other
//! tabs; the work in progress; what is said when there is nothing, or no
//! account, or an account could not be asked.
//!
//! A table has these columns: at the left the pin and the snooze (shown
//! on the row under the pointer, the pin always on a pinned row), how
//! long ago the row changed, its state (the pull request's, its
//! reviews', its checks'), the title with the number, the lines it adds
//! and removes and its labels and milestone under it; at the right, each
//! as wide at every window size, the author, the others in it, the
//! repository with the branch, and the button that does what the row
//! most needs with the rest in its menu. Only the title's column grows
//! with the window.
//!
//! Sources: none (original design).

use std::path::PathBuf;

use bitbraid_integrations::{
    inbox::{Approval, InboxIssue, InboxPull, PullGroup, WorkInProgress},
    model::{CiState, Label, Milestone, ProviderKind, ReviewState, User},
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    menu::{DropdownMenu, PopupMenuItem},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{
    InboxEvent, InboxView, Tab,
    facts::{self, FirstAction, Row},
};
use crate::{
    views::pulls::{
        card::{label_chip, picture},
        checks::ci_icon,
        facts::{parse_time, provider_name, shown_line},
    },
    zoom::px,
};

/// From the window's edges to the tables.
pub(super) const LIST_LEFT: f32 = 45.;
pub(super) const LIST_RIGHT: f32 = 57.;
/// A group's heading, the room under it before its table, and after the
/// table before the next heading.
const GROUP_HEIGHT: f32 = 40.;
const GROUP_GAP: f32 = 7.;
const TABLE_TOP: f32 = 5.;
const TABLE_BOTTOM: f32 = 13.;
/// The table's heading row and a row.
const HEAD_HEIGHT: f32 = 28.;
pub(super) const ROW_HEIGHT: f32 = 57.;
/// The columns from the table's left edge: the pin and the snooze, the
/// age, the state, the title's.
const MARKS_WIDTH: f32 = 45.;
const AGE_WIDTH: f32 = 58.;
const STATE_WIDTH: f32 = 70.;
/// The columns from the right edge, each as wide at every size.
const AUTHOR_WIDTH: f32 = 88.;
const PEOPLE_WIDTH: f32 = 208.;
const REPO_WIDTH: f32 = 258.;
const ACTION_WIDTH: f32 = 254.;
/// The least the title's column keeps, and with it the least a table is
/// wide: a window narrower than that (1280 at 150 % is 853 points) scrolls
/// the list sideways rather than putting the title under the author and
/// the buttons out of the window.
const TITLE_LEAST: f32 = 160.;
pub(super) const TABLE_LEAST: f32 = MARKS_WIDTH
    + AGE_WIDTH
    + STATE_WIDTH
    + TITLE_LEAST
    + AUTHOR_WIDTH
    + PEOPLE_WIDTH
    + REPO_WIDTH
    + ACTION_WIDTH;
/// The button of a row: its two parts.
const ACTION_MAIN: f32 = 160.;
const ACTION_CARET: f32 = 33.;
const ACTION_HEIGHT: f32 = 31.;
/// From the way to copy a row's link to the table's right edge.
const COPY_RIGHT: f32 = 11.;
/// Pictures of people.
const AUTHOR_PICTURE: f32 = 28.;
const PERSON_PICTURE: f32 = 26.;
/// The empty state's card.
const CARD_LEFT: f32 = 15.;
const CARD_WIDTH: f32 = 902.;

/// Where a row is drawn: its group, when it is in one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Place {
    group: Option<PullGroup>,
}

impl Place {
    fn key(self) -> &'static str {
        self.group.map_or("list", PullGroup::key)
    }
}

impl InboxView {
    /// What the list holds, for the tab on show.
    pub(super) fn body(&self, _window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let accounts = !crate::providers::accounts(cx).is_empty();
        let Some(answer) = self.answer.clone() else {
            if !accounts && !self.given {
                return self.no_account(cx);
            }
            return match &self.error {
                Some(error) => {
                    self.card("inbox-error", say!("inbox-error"), error.clone(), None, cx)
                }
                None => self.card(
                    "inbox-reading",
                    say!("inbox-reading"),
                    say!("inbox-reading-text").to_owned(),
                    None,
                    cx,
                ),
            };
        };
        if answer.accounts.is_empty() && !accounts && answer.pulls.is_empty() {
            return self.no_account(cx);
        }
        let mut parts: Vec<AnyElement> = Vec::new();
        // An account that could not be asked says so above the rest.
        for state in answer.accounts.iter().filter(|state| state.error.is_some()) {
            let theme = cx.theme();
            parts.push(
                h_flex()
                    .id(SharedString::from(format!(
                        "inbox-account-error-{}",
                        state.account
                    )))
                    .w_full()
                    .mb(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .gap(px(8.))
                    .rounded(theme.radius)
                    .bg(theme.danger.opacity(0.12))
                    .text_sm()
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .size(px(14.))
                            .text_color(theme.danger),
                    )
                    .child(div().min_w_0().truncate().child(say!(
                        "inbox-account-failed",
                        login = shown_line(&state.login, 60),
                        provider = provider_name(state.kind),
                        error = crate::providers::said(&shown_line(
                            state.error.as_deref().unwrap_or_default(),
                            300
                        ))
                    )))
                    .test_support()
                    .into_any_element(),
            );
        }
        if let Some(problem) = &answer.marks_problem {
            let theme = cx.theme();
            parts.push(
                div()
                    .id("inbox-marks-problem")
                    .mb(px(8.))
                    .px(px(12.))
                    .py(px(8.))
                    .rounded(theme.radius)
                    .bg(theme.warning.opacity(0.12))
                    .text_sm()
                    .child(say!(
                        "inbox-marks-problem",
                        problem = shown_line(problem, 300)
                    ))
                    .into_any_element(),
            );
        }
        match self.tab {
            Tab::Pulls => {
                let groups = facts::grouped(&answer, &self.choices, self.now);
                if groups.is_empty() {
                    parts.push(self.nothing(cx));
                }
                for (group, rows) in groups {
                    parts.push(self.group(group, &rows, cx));
                }
            }
            Tab::Work => {
                let work = facts::work(&answer, &self.choices);
                if work.is_empty() {
                    parts.push(self.card(
                        "inbox-nothing",
                        say!("inbox-work-nothing"),
                        say!("inbox-work-nothing-text").to_owned(),
                        None,
                        cx,
                    ));
                } else {
                    parts.push(self.work_table(&work, cx));
                }
            }
            tab => {
                let rows = facts::flat(&answer, tab, &self.choices, self.now);
                if rows.is_empty() {
                    parts.push(self.nothing(cx));
                } else {
                    parts.push(self.table(Place { group: None }, &rows, cx));
                }
            }
        }
        v_flex().w_full().children(parts).into_any_element()
    }

    /// The card that says why there is nothing.
    fn card(
        &self,
        id: &'static str,
        title: &'static str,
        text: String,
        button: Option<AnyElement>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .id(id)
            .ml(px(CARD_LEFT))
            .mt(px(40.))
            .w(px(CARD_WIDTH))
            .max_w_full()
            .p(px(40.))
            .gap(px(14.))
            .rounded(theme.radius_lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(div().text_size(px(24.)).child(title))
            .child(div().text_color(theme.muted_foreground).child(text))
            .children(button)
            .test_support()
            .into_any_element()
    }

    fn no_account(&self, cx: &mut Context<Self>) -> AnyElement {
        let button = Button::new("inbox-connect")
            .outline()
            .small()
            .w(px(220.))
            .icon(gpui_kit::assets::IconName::Plug)
            .label(say!("inbox-connect"))
            .on_click(cx.listener(|_, _, _, cx| cx.emit(InboxEvent::OpenSettings)))
            .into_any_element();
        self.card(
            "inbox-no-account",
            say!("inbox-no-account"),
            say!("inbox-no-account-text").to_owned(),
            Some(button),
            cx,
        )
    }

    fn nothing(&self, cx: &mut Context<Self>) -> AnyElement {
        let (title, text) = match self.tab {
            Tab::Snoozed => (
                say!("inbox-snoozed-nothing"),
                say!("inbox-snoozed-nothing-text"),
            ),
            Tab::Issues => (
                say!("inbox-issues-nothing"),
                say!("inbox-issues-nothing-text"),
            ),
            _ => (say!("inbox-nothing"), say!("inbox-nothing-text")),
        };
        self.card("inbox-nothing", title, text.to_owned(), None, cx)
    }

    /// A group of the pull requests' tab: its heading, and its table
    /// unless it is folded.
    fn group(&self, group: PullGroup, rows: &[&InboxPull], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let folded = self.folded.contains(&group);
        let (icon, colour) = group_icon(group, cx);
        let rows: Vec<Row<'_>> = rows.iter().map(|pull| Row::Pull(pull)).collect();
        v_flex()
            .w_full()
            .child(
                h_flex()
                    .id(SharedString::from(format!("inbox-group-{}", group.key())))
                    .w_full()
                    .h(px(GROUP_HEIGHT))
                    .pl(px(14.))
                    .gap(px(14.))
                    .items_center()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.list_hover))
                    .child(
                        Icon::new(if folded {
                            IconName::ChevronRight
                        } else {
                            IconName::ChevronDown
                        })
                        .size(px(16.)),
                    )
                    .child(
                        h_flex()
                            .gap(px(8.))
                            .items_center()
                            .child(icon.size(px(14.)).text_color(colour))
                            .child(div().text_sm().child(facts::group_title(group)))
                            .child(count_chip(rows.len(), cx)),
                    )
                    .on_click(cx.listener(move |this, _, _, cx| this.fold(group, cx)))
                    .test_support(),
            )
            .map(|this| {
                if folded {
                    this.mb(px(GROUP_GAP))
                } else {
                    this.child(
                        div()
                            .mt(px(TABLE_TOP))
                            .mb(px(TABLE_BOTTOM))
                            .child(self.table(Place { group: Some(group) }, &rows, cx)),
                    )
                }
            })
            .into_any_element()
    }

    /// A table: its heading row and its rows.
    fn table(&self, place: Place, rows: &[Row<'_>], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let heading = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let head = h_flex()
            .w_full()
            .h(px(HEAD_HEIGHT))
            .flex_none()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .w(px(MARKS_WIDTH))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Pin)
                            .size(px(13.))
                            .text_color(theme.muted_foreground),
                    ),
            )
            .child(
                div()
                    .w(px(AGE_WIDTH))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::Clock)
                            .size(px(13.))
                            .text_color(theme.muted_foreground),
                    ),
            )
            .child(
                div()
                    .w(px(STATE_WIDTH))
                    .flex_none()
                    .child(heading(say!("inbox-column-state"))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(heading(say!("inbox-column-item"))),
            )
            .child(
                div()
                    .w(px(AUTHOR_WIDTH))
                    .flex_none()
                    .child(heading(say!("inbox-column-author"))),
            )
            .child(
                div()
                    .w(px(PEOPLE_WIDTH))
                    .flex_none()
                    .child(heading(say!("inbox-column-with"))),
            )
            .child(
                div()
                    .w(px(REPO_WIDTH))
                    .flex_none()
                    .child(heading(say!("inbox-column-repository"))),
            )
            .child(
                div()
                    .w(px(ACTION_WIDTH))
                    .flex_none()
                    .child(heading(say!("inbox-column-action"))),
            );
        v_flex()
            .id(SharedString::from(format!("inbox-table-{}", place.key())))
            .w_full()
            .overflow_hidden()
            // A group's table is a box of its own; a flat list is rows on
            // the page.
            .when(place.group.is_some(), |table| {
                table
                    .rounded(theme.radius)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.secondary.opacity(0.5))
            })
            .child(head)
            .children(rows.iter().enumerate().map(|(at, row)| {
                let last = at + 1 == rows.len();
                match row {
                    Row::Pull(pull) => self.pull_row(place, pull, last, cx),
                    Row::Issue(issue) => self.issue_row(place, issue, last, cx),
                }
            }))
            .test_support()
            .into_any_element()
    }

    /// The pin and the snooze at a row's left: shown under the pointer,
    /// the pin always while it holds; in the snoozed tab, the way back.
    fn marks(
        &self,
        key: &str,
        pinned: bool,
        snoozed_now: bool,
        group: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let busy = self.marking.contains(key);
        let id = |what: &str| SharedString::from(format!("inbox-{what}-{group}-{key}"));
        let pin_key = key.to_owned();
        let pin = Button::new(id("pin"))
            .accessibility_label(if pinned {
                say!("inbox-unpin")
            } else {
                say!("inbox-pin")
            })
            .ghost()
            .xsmall()
            .disabled(busy)
            .icon(
                Icon::new(gpui_kit::assets::IconName::Pin).text_color(if pinned {
                    theme.primary
                } else {
                    theme.muted_foreground
                }),
            )
            .tooltip(if pinned {
                say!("inbox-unpin")
            } else {
                say!("inbox-pin")
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                let command = if pinned { "inbox.unpin" } else { "inbox.pin" };
                this.mark(pin_key.clone(), command, None, window, cx);
            }));
        let snooze_key = key.to_owned();
        let snooze = if snoozed_now {
            Button::new(id("unsnooze"))
                .accessibility_label(say!("inbox-unsnooze"))
                .ghost()
                .xsmall()
                .disabled(busy)
                .icon(
                    Icon::new(gpui_kit::assets::IconName::AlarmClockOff).text_color(theme.primary),
                )
                .tooltip(say!("inbox-unsnooze"))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.mark(snooze_key.clone(), "inbox.unsnooze", None, window, cx);
                }))
                .into_any_element()
        } else {
            let view = cx.entity().downgrade();
            Button::new(id("snooze"))
                .accessibility_label(say!("inbox-snooze"))
                .ghost()
                .xsmall()
                .disabled(busy)
                .icon(
                    Icon::new(gpui_kit::assets::IconName::AlarmClock)
                        .text_color(theme.muted_foreground),
                )
                .tooltip(say!("inbox-snooze"))
                .dropdown_menu(move |menu, _, _| {
                    let choices: [(&'static str, Option<&'static str>); 4] = [
                        (say!("inbox-snooze-until-back"), None),
                        (say!("inbox-snooze-hour"), Some("hour")),
                        (say!("inbox-snooze-four-hours"), Some("four_hours")),
                        (say!("inbox-snooze-tomorrow"), Some("tomorrow")),
                    ];
                    choices.into_iter().fold(menu, |menu, (label, length)| {
                        let (view, key) = (view.clone(), snooze_key.clone());
                        menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.mark(key.clone(), "inbox.snooze", length, window, cx)
                            });
                        }))
                    })
                })
                .into_any_element()
        };
        let row_group = SharedString::from(format!("inbox-row-{group}-{key}"));
        h_flex()
            .w(px(MARKS_WIDTH))
            .flex_none()
            .pl(px(4.))
            .items_center()
            .child(
                div()
                    .when(!pinned, |pin_place| {
                        pin_place
                            .invisible()
                            .group_hover(row_group.clone(), |style| style.visible())
                    })
                    .child(pin),
            )
            .child(
                div()
                    .when(!snoozed_now, |place| {
                        place
                            .invisible()
                            .group_hover(row_group.clone(), |style| style.visible())
                    })
                    .child(snooze),
            )
            .into_any_element()
    }

    /// A pull request's row.
    fn pull_row(
        &self,
        place: Place,
        pull: &InboxPull,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let row = &pull.pull;
        let group = place.key();
        let row_group = SharedString::from(format!("inbox-row-{group}-{}", pull.key));
        let snoozed_now = facts::snoozed(pull.snoozed.as_ref(), self.now);
        let number = number_text(pull.kind, row.number);
        let action = facts::first_action(pull, place.group);
        let provider = provider_name(pull.kind);
        // Under the title: the lines, the labels, the milestone, and in
        // the snoozed tab when it comes back.
        let lines = lines_text(pull.additions, pull.deletions, cx);
        let under = h_flex()
            .h(px(18.))
            .gap(px(8.))
            .items_center()
            .min_w_0()
            .overflow_hidden()
            .when_some(pull.snoozed.filter(|_| snoozed_now), |line, snoozed| {
                line.child(snooze_line(&snoozed, self.now, cx))
            })
            .children(lines)
            .children(labels_line(&row.labels, cx))
            .children(milestone_line(pull.milestone.as_ref(), cx));
        let others = people(
            row.reviewers
                .iter()
                .map(|reviewer| &reviewer.user)
                .chain(row.assignees.iter())
                .filter(|user| user.login != row.author.login),
            cx,
        );
        h_flex()
            .id(SharedString::from(format!(
                "inbox-pull-{group}-{}",
                pull.key
            )))
            .group(row_group)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .items_center()
            .when(!last, |row| row.border_b_1().border_color(theme.border))
            .hover(|style| style.bg(theme.list_hover))
            .child(self.marks(&pull.key, pull.pinned, snoozed_now, group, cx))
            .child(age_cell(&row.updated, self.now, cx))
            .child(
                h_flex()
                    .w(px(STATE_WIDTH))
                    .flex_none()
                    .gap(px(9.))
                    .items_center()
                    .child(
                        Icon::new(if row.draft {
                            gpui_kit::assets::IconName::GitPullRequestDraft
                        } else {
                            gpui_kit::assets::IconName::GitPullRequest
                        })
                        .size(px(14.))
                        .text_color(if row.draft {
                            theme.muted_foreground
                        } else {
                            theme.success
                        }),
                    )
                    .child(review_mark(pull, cx))
                    .child(checks_mark(row.ci, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .pr(px(12.))
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(px(10.))
                            .items_center()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(shown_line(&row.title, 300)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "inbox-number-{group}-{}",
                                        pull.key
                                    )))
                                    .flex_none()
                                    .text_sm()
                                    .text_color(theme.primary)
                                    .cursor_pointer()
                                    .child(number)
                                    .on_click({
                                        let url = row.web_url.clone();
                                        cx.listener(move |this, _, window, cx| {
                                            this.open_page(&url, window, cx)
                                        })
                                    }),
                            ),
                    )
                    .child(under),
            )
            .child(
                div()
                    .w(px(AUTHOR_WIDTH))
                    .flex_none()
                    .pl(px(2.))
                    .child(person(&row.author, AUTHOR_PICTURE, cx)),
            )
            .child(div().w(px(PEOPLE_WIDTH)).flex_none().child(others))
            .child(repo_cell(
                &row.repo,
                Some(&row.head_ref),
                pull.path.is_some(),
                None,
                cx,
            ))
            .child(self.pull_action(place, pull, action, provider, cx))
            .test_support()
            .into_any_element()
    }

    /// The button of a pull request's row, and its menu.
    fn pull_action(
        &self,
        place: Place,
        pull: &InboxPull,
        first: FirstAction,
        provider: &'static str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id =
            |what: &str| SharedString::from(format!("inbox-{what}-{}-{}", place.key(), pull.key));
        let view = cx.entity().downgrade();
        let do_it = Do::of_pull(pull);
        let main = {
            let (view, do_it) = (view.clone(), do_it.clone());
            Button::new(id("action"))
                .outline()
                .small()
                .w(px(ACTION_MAIN))
                .h(px(ACTION_HEIGHT))
                .label(first.label(provider))
                .on_click(move |_, window, cx| {
                    let what = match first {
                        FirstAction::Merge => What::Merge,
                        FirstAction::Review => What::Review,
                        FirstAction::OpenRepository => What::OpenRepository,
                        FirstAction::OpenAtProvider => What::OpenAtProvider,
                    };
                    let _ = view.update(cx, |this, cx| do_it.run(this, what, window, cx));
                })
        };
        let here = pull.path.is_some();
        let branch = shown_line(&pull.pull.head_ref, 40);
        let name = pull
            .pull
            .repo
            .rsplit_once('/')
            .map_or_else(|| pull.pull.repo.clone(), |(_, name)| name.to_owned());
        let caret = Button::new(id("more"))
            .accessibility_label(say!("inbox-more"))
            .outline()
            .small()
            .w(px(ACTION_CARET))
            .h(px(ACTION_HEIGHT))
            .icon(IconName::ChevronDown)
            .dropdown_menu(move |menu, _, _| {
                let mut entries: Vec<(String, What)> = vec![
                    (say!("inbox-action-open-at", provider), What::OpenAtProvider),
                    (say!("inbox-menu-merge").to_owned(), What::Merge),
                ];
                if here {
                    entries.extend([
                        (
                            say!("inbox-action-open-repository").to_owned(),
                            What::OpenRepository,
                        ),
                        (say!("inbox-menu-review").to_owned(), What::Review),
                        (
                            say!("inbox-menu-check-out", branch = branch.as_str()),
                            What::CheckOut,
                        ),
                        (say!("inbox-menu-worktree").to_owned(), What::Worktree),
                    ]);
                } else {
                    entries.extend([
                        (
                            say!("inbox-menu-clone", name = shown_line(&name, 40)),
                            What::Clone,
                        ),
                        (say!("inbox-menu-locate").to_owned(), What::Locate),
                    ]);
                }
                entries.push((say!("inbox-menu-copy-link").to_owned(), What::CopyLink));
                entries.into_iter().fold(menu, |menu, (label, what)| {
                    let (view, do_it) = (view.clone(), do_it.clone());
                    menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| do_it.run(this, what, window, cx));
                    }))
                })
            });
        // The way to copy its link stands in the column too, at its end.
        h_flex()
            .w(px(ACTION_WIDTH))
            .flex_none()
            .items_center()
            .child(main)
            .child(caret)
            .child(div().flex_1())
            .child(copy_link(
                &format!("{}-{}", place.key(), pull.key),
                &pull.pull.web_url,
                cx,
            ))
            .into_any_element()
    }

    /// An issue's row.
    fn issue_row(
        &self,
        place: Place,
        issue: &InboxIssue,
        last: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let row = &issue.issue;
        let group = place.key();
        let row_group = SharedString::from(format!("inbox-row-{group}-{}", issue.key));
        let snoozed_now = facts::snoozed(issue.snoozed.as_ref(), self.now);
        let provider = provider_name(issue.kind);
        let here = issue.path.is_some();
        let do_it = Do::of_issue(issue);
        let view = cx.entity().downgrade();
        let id = |what: &str| SharedString::from(format!("inbox-{what}-{group}-{}", issue.key));
        let counts = h_flex()
            .gap(px(10.))
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(
                h_flex()
                    .gap(px(3.))
                    .items_center()
                    .child(Icon::new(gpui_kit::assets::IconName::MessageSquare).size(px(12.)))
                    .child(row.comments.to_string()),
            )
            .when_some(issue.reactions, |line, reactions| {
                line.child(
                    h_flex()
                        .gap(px(3.))
                        .items_center()
                        .child(Icon::new(IconName::ThumbsUp).size(px(12.)))
                        .child(reactions.to_string()),
                )
            });
        let under = h_flex()
            .h(px(18.))
            .gap(px(8.))
            .items_center()
            .min_w_0()
            .overflow_hidden()
            .when_some(issue.snoozed.filter(|_| snoozed_now), |line, snoozed| {
                line.child(snooze_line(&snoozed, self.now, cx))
            })
            .child(counts)
            .children(labels_line(&row.labels, cx))
            .children(milestone_line(row.milestone.as_ref(), cx));
        let branch_link = {
            let (view, do_it) = (view.clone(), do_it.clone());
            div()
                .id(id("branch"))
                .text_xs()
                .when(here, |link| {
                    link.text_color(theme.primary).cursor_pointer().on_click(
                        move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| {
                                do_it.run(this, What::IssueBranch, window, cx)
                            });
                        },
                    )
                })
                .when(!here, |link| link.text_color(theme.muted_foreground))
                .child(say!("inbox-issue-branch"))
                .into_any_element()
        };
        let main = {
            let (view, do_it) = (view.clone(), do_it.clone());
            let what = if here {
                What::IssueBranch
            } else {
                What::OpenAtProvider
            };
            Button::new(id("action"))
                .outline()
                .small()
                .w(px(ACTION_MAIN))
                .h(px(ACTION_HEIGHT))
                .label(if here {
                    say!("inbox-issue-create-branch").to_owned()
                } else {
                    say!("inbox-action-open-at", provider)
                })
                .on_click(move |_, window, cx| {
                    let _ = view.update(cx, |this, cx| do_it.run(this, what, window, cx));
                })
        };
        let name = row
            .repo
            .rsplit_once('/')
            .map_or_else(|| row.repo.clone(), |(_, name)| name.to_owned());
        let caret = {
            let (view, do_it) = (view.clone(), do_it.clone());
            Button::new(id("more"))
                .accessibility_label(say!("inbox-more"))
                .outline()
                .small()
                .w(px(ACTION_CARET))
                .h(px(ACTION_HEIGHT))
                .icon(IconName::ChevronDown)
                .dropdown_menu(move |menu, _, _| {
                    let mut entries: Vec<(String, What)> =
                        vec![(say!("inbox-action-open-at", provider), What::OpenAtProvider)];
                    if here {
                        entries.extend([
                            (say!("inbox-menu-show-issue").to_owned(), What::ShowIssue),
                            (
                                say!("inbox-action-open-repository").to_owned(),
                                What::OpenRepository,
                            ),
                            (
                                say!("inbox-menu-issue-branch").to_owned(),
                                What::IssueBranch,
                            ),
                        ]);
                    } else {
                        entries.extend([
                            (
                                say!("inbox-menu-clone", name = shown_line(&name, 40)),
                                What::Clone,
                            ),
                            (say!("inbox-menu-locate").to_owned(), What::Locate),
                        ]);
                    }
                    entries.push((say!("inbox-menu-copy-link").to_owned(), What::CopyLink));
                    entries.into_iter().fold(menu, |menu, (label, what)| {
                        let (view, do_it) = (view.clone(), do_it.clone());
                        menu.item(PopupMenuItem::new(label).on_click(move |_, window, cx| {
                            let _ = view.update(cx, |this, cx| do_it.run(this, what, window, cx));
                        }))
                    })
                })
        };
        h_flex()
            .id(SharedString::from(format!(
                "inbox-issue-{group}-{}",
                issue.key
            )))
            .group(row_group)
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .items_center()
            .when(!last, |row| row.border_b_1().border_color(theme.border))
            .hover(|style| style.bg(theme.list_hover))
            .child(self.marks(&issue.key, issue.pinned, snoozed_now, group, cx))
            .child(age_cell(&row.updated, self.now, cx))
            .child(
                h_flex()
                    .w(px(STATE_WIDTH))
                    .flex_none()
                    .items_center()
                    .child(
                        Icon::new(gpui_kit::assets::IconName::CircleDot)
                            .size(px(14.))
                            .text_color(theme.success),
                    ),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .pr(px(12.))
                    .gap(px(6.))
                    .child(
                        h_flex()
                            .min_w_0()
                            .gap(px(10.))
                            .items_center()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .child(shown_line(&row.title, 300)),
                            )
                            .child(
                                div()
                                    .id(id("number"))
                                    .flex_none()
                                    .text_sm()
                                    .text_color(theme.primary)
                                    .cursor_pointer()
                                    .child(format!("#{}", row.number))
                                    .on_click({
                                        let url = row.web_url.clone();
                                        cx.listener(move |this, _, window, cx| {
                                            this.open_page(&url, window, cx)
                                        })
                                    }),
                            ),
                    )
                    .child(under),
            )
            .child(
                div()
                    .w(px(AUTHOR_WIDTH))
                    .flex_none()
                    .pl(px(2.))
                    .child(person(&row.author, AUTHOR_PICTURE, cx)),
            )
            .child(
                div()
                    .w(px(PEOPLE_WIDTH))
                    .flex_none()
                    .child(people(row.assignees.iter(), cx)),
            )
            .child(repo_cell(&row.repo, None, here, Some(branch_link), cx))
            .child(
                h_flex()
                    .w(px(ACTION_WIDTH))
                    .flex_none()
                    .items_center()
                    .child(main)
                    .child(caret)
                    .child(div().flex_1())
                    .child(copy_link(
                        &format!("{group}-{}", issue.key),
                        &row.web_url,
                        cx,
                    )),
            )
            .test_support()
            .into_any_element()
    }

    /// The table of the work in progress: one row for each repository
    /// with uncommitted changes.
    fn work_table(&self, work: &[&WorkInProgress], cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let heading = |text: &'static str| {
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let rows = work.iter().enumerate().map(|(at, wip)| {
            let path = PathBuf::from(&wip.path);
            let last = at + 1 == work.len();
            let counts = wip.counts;
            let kinds = [
                (
                    gpui_kit::assets::IconName::Pencil,
                    counts.modified,
                    theme.warning,
                    say!("inbox-work-changed", count = counts.modified),
                ),
                (
                    gpui_kit::assets::IconName::FilePen,
                    counts.added,
                    theme.success,
                    say!("inbox-work-new", count = counts.added),
                ),
                (
                    gpui_kit::assets::IconName::Trash,
                    counts.deleted,
                    theme.danger,
                    say!("inbox-work-removed", count = counts.deleted),
                ),
                (
                    gpui_kit::assets::IconName::ArrowRightLeft,
                    counts.renamed,
                    theme.info,
                    say!("inbox-work-moved", count = counts.renamed),
                ),
            ];
            let open = path.clone();
            h_flex()
                .id(SharedString::from(format!("inbox-work-{}", wip.path)))
                .w_full()
                .h(px(ROW_HEIGHT))
                .flex_none()
                .items_center()
                .when(!last, |row| row.border_b_1().border_color(theme.border))
                .hover(|style| style.bg(theme.list_hover))
                .child(div().w(px(MARKS_WIDTH + AGE_WIDTH)).flex_none())
                .child(
                    h_flex().w(px(STATE_WIDTH)).flex_none().child(
                        Icon::new(gpui_kit::assets::IconName::FolderGit2)
                            .size(px(14.))
                            .text_color(theme.warning),
                    ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .pr(px(12.))
                        .gap(px(6.))
                        .child(div().text_sm().truncate().child(shown_line(&wip.name, 120)))
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .truncate()
                                .child(shown_line(&wip.path, 300)),
                        ),
                )
                .child(
                    h_flex()
                        .w(px(AUTHOR_WIDTH + PEOPLE_WIDTH))
                        .flex_none()
                        .gap(px(12.))
                        .text_xs()
                        .children(kinds.into_iter().filter(|(_, count, _, _)| *count > 0).map(
                            |(icon, _, colour, text)| {
                                h_flex()
                                    .gap(px(4.))
                                    .items_center()
                                    .child(Icon::new(icon).size(px(12.)).text_color(colour))
                                    .child(text)
                            },
                        )),
                )
                .child(
                    v_flex()
                        .w(px(REPO_WIDTH))
                        .flex_none()
                        .gap(px(4.))
                        .child(branch_chip(
                            wip.branch
                                .clone()
                                .unwrap_or_else(|| say!("inbox-work-detached").to_owned()),
                            cx,
                        ))
                        .when(
                            wip.ahead.unwrap_or(0) > 0 || wip.behind.unwrap_or(0) > 0,
                            |cell| {
                                cell.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(say!(
                                            "inbox-work-ahead-behind",
                                            ahead = wip.ahead.unwrap_or(0),
                                            behind = wip.behind.unwrap_or(0)
                                        )),
                                )
                            },
                        ),
                )
                .child(
                    h_flex().w(px(ACTION_WIDTH)).flex_none().child(
                        Button::new(SharedString::from(format!("inbox-work-open-{}", wip.path)))
                            .outline()
                            .small()
                            .w(px(ACTION_MAIN + ACTION_CARET))
                            .h(px(ACTION_HEIGHT))
                            .label(say!("inbox-action-open-repository"))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(InboxEvent::OpenRepository(open.clone()))
                            })),
                    ),
                )
                .test_support()
        });
        v_flex()
            .id("inbox-table-work")
            .w_full()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary.opacity(0.5))
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(px(HEAD_HEIGHT))
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .w(px(MARKS_WIDTH + AGE_WIDTH + STATE_WIDTH))
                            .flex_none(),
                    )
                    .child(
                        div()
                            .flex_1()
                            .child(heading(say!("inbox-column-repository"))),
                    )
                    .child(
                        div()
                            .w(px(AUTHOR_WIDTH + PEOPLE_WIDTH))
                            .flex_none()
                            .child(heading(say!("inbox-column-changes"))),
                    )
                    .child(
                        div()
                            .w(px(REPO_WIDTH))
                            .flex_none()
                            .child(heading(say!("inbox-column-branch"))),
                    )
                    .child(
                        div()
                            .w(px(ACTION_WIDTH))
                            .flex_none()
                            .child(heading(say!("inbox-column-action"))),
                    ),
            )
            .children(rows)
            .test_support()
            .into_any_element()
    }
}

/// What a row's button or menu entry does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum What {
    Merge,
    Review,
    OpenRepository,
    OpenAtProvider,
    CheckOut,
    Worktree,
    Clone,
    Locate,
    CopyLink,
    IssueBranch,
    ShowIssue,
}

/// What a row's actions need to know of it.
#[derive(Debug, Clone)]
struct Do {
    key: String,
    url: String,
    path: Option<PathBuf>,
    link: Option<bitbraid_integrations::model::RepoLink>,
    number: u64,
    fork: Option<String>,
    clone_url: String,
    issue: Option<bitbraid_integrations::model::Issue>,
}

/// Where a repository at a provider is cloned from, by its page.
fn clone_address(web_url: &str, repo: &str) -> String {
    // The pull request's or issue's page is on the repository's site.
    let host_end = web_url.find("://").and_then(|scheme| {
        web_url
            .get(scheme + 3..)?
            .find('/')
            .map(|at| at + scheme + 3)
    });
    match host_end.and_then(|end| web_url.get(..end)) {
        Some(site) => format!("{site}/{repo}.git"),
        None => String::new(),
    }
}

impl Do {
    fn of_pull(pull: &InboxPull) -> Self {
        Self {
            key: pull.key.clone(),
            url: pull.pull.web_url.clone(),
            path: pull.path.as_ref().map(PathBuf::from),
            link: InboxView::link(
                &pull.account,
                pull.kind,
                &pull.pull.repo,
                pull.remote.as_deref(),
            ),
            number: pull.pull.number,
            fork: pull.pull.head_repo.clone(),
            clone_url: clone_address(&pull.pull.web_url, &pull.pull.repo),
            issue: None,
        }
    }

    fn of_issue(issue: &InboxIssue) -> Self {
        Self {
            key: issue.key.clone(),
            url: issue.issue.web_url.clone(),
            path: issue.path.as_ref().map(PathBuf::from),
            link: InboxView::link(
                &issue.account,
                issue.kind,
                &issue.issue.repo,
                issue.remote.as_deref(),
            ),
            number: issue.issue.number,
            fork: None,
            clone_url: clone_address(&issue.issue.web_url, &issue.issue.repo),
            issue: Some(issue.issue.clone()),
        }
    }

    fn run(
        &self,
        view: &mut InboxView,
        what: What,
        window: &mut Window,
        cx: &mut Context<InboxView>,
    ) {
        match what {
            What::Merge => view.ask_merge(self.key.clone(), cx),
            What::OpenAtProvider => view.open_page(&self.url, window, cx),
            What::CopyLink => {
                if crate::providers::openable(&self.url) {
                    cx.write_to_clipboard(ClipboardItem::new_string(self.url.clone()));
                }
            }
            What::Clone => {
                if !self.clone_url.is_empty() {
                    cx.emit(InboxEvent::Clone {
                        url: self.clone_url.clone(),
                    });
                }
            }
            What::Locate => cx.emit(InboxEvent::Locate),
            What::OpenRepository => {
                if let Some(path) = &self.path {
                    cx.emit(InboxEvent::OpenRepository(path.clone()));
                }
            }
            What::Review => {
                if let (Some(path), Some(link)) = (&self.path, &self.link) {
                    cx.emit(InboxEvent::Review {
                        path: path.clone(),
                        link: link.clone(),
                        number: self.number,
                    });
                }
            }
            What::CheckOut => {
                if let (Some(path), Some(link)) = (&self.path, &self.link) {
                    cx.emit(InboxEvent::CheckOut {
                        path: path.clone(),
                        link: link.clone(),
                        number: self.number,
                        fork: self.fork.clone(),
                    });
                }
            }
            What::Worktree => {
                if let (Some(path), Some(link)) = (&self.path, &self.link) {
                    cx.emit(InboxEvent::Worktree {
                        path: path.clone(),
                        link: link.clone(),
                        number: self.number,
                    });
                }
            }
            What::ShowIssue => {
                if let (Some(path), Some(link)) = (&self.path, &self.link) {
                    cx.emit(InboxEvent::ShowIssue {
                        path: path.clone(),
                        link: link.clone(),
                        number: self.number,
                    });
                }
            }
            What::IssueBranch => {
                if let (Some(path), Some(link), Some(issue)) = (&self.path, &self.link, &self.issue)
                {
                    cx.emit(InboxEvent::IssueBranch {
                        path: path.clone(),
                        link: link.clone(),
                        issue: Box::new(issue.clone()),
                    });
                }
            }
        }
    }
}

/// `#12` at GitHub, `!12` at GitLab.
fn number_text(kind: ProviderKind, number: u64) -> String {
    crate::views::pulls::facts::number_text(kind, number)
}

/// A count in a small rounded box.
fn count_chip(count: usize, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .h(px(18.))
        .min_w(px(20.))
        .px(px(5.))
        .flex()
        .items_center()
        .justify_center()
        .rounded_full()
        .bg(theme.muted)
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(count.to_string())
        .into_any_element()
}

/// A group's mark and its colour.
fn group_icon(group: PullGroup, cx: &App) -> (Icon, Hsla) {
    use gpui_kit::assets::IconName as Extra;
    let theme = cx.theme();
    match group {
        PullGroup::ReadyToMerge => (Icon::new(Extra::GitMerge), theme.success),
        PullGroup::ReviewMine => (Icon::new(IconName::Eye), theme.primary),
        PullGroup::NoReviewers => (Icon::new(Extra::UserPlus), theme.danger),
        PullGroup::FailingChecks => (Icon::new(IconName::CircleX), theme.danger),
        PullGroup::Conflicts => (Icon::new(IconName::TriangleAlert), theme.warning),
        PullGroup::ChangesRequested => (Icon::new(Extra::FilePen), theme.warning),
        PullGroup::Commented => (Icon::new(Extra::MessageSquare), theme.foreground),
        PullGroup::WaitingForReview => (Icon::new(Extra::Hourglass), theme.muted_foreground),
        PullGroup::Draft => (
            Icon::new(Extra::GitPullRequestDraft),
            theme.muted_foreground,
        ),
        PullGroup::Other => (Icon::new(Extra::Circle), theme.muted_foreground),
    }
}

/// How long ago a row changed, in the fewest letters.
fn age_cell(updated: &str, now: i64, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let text = parse_time(updated)
        .map(|at| facts::short_age(at, now))
        .unwrap_or_default();
    div()
        .w(px(AGE_WIDTH))
        .flex_none()
        .flex()
        .justify_center()
        .text_sm()
        .text_color(theme.muted_foreground)
        .child(text)
        .into_any_element()
}

/// A dash, where there is nothing to mark.
fn dash(cx: &App) -> AnyElement {
    div()
        .w(px(14.))
        .flex()
        .justify_center()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child("–")
        .into_any_element()
}

/// What a pull request's reviews add up to, as a mark.
fn review_mark(pull: &InboxPull, cx: &App) -> AnyElement {
    use gpui_kit::assets::IconName as Extra;
    let theme = cx.theme();
    let any = |state: ReviewState| {
        pull.pull
            .reviewers
            .iter()
            .any(|reviewer| reviewer.state == state)
    };
    let (icon, colour, said) =
        if any(ReviewState::ChangesRequested) || pull.approval == Approval::ChangesRequested {
            (
                Icon::new(Extra::FilePen),
                theme.danger,
                say!("inbox-review-changes-asked"),
            )
        } else if pull.approval == Approval::Approved || any(ReviewState::Approved) {
            (
                Icon::new(IconName::CircleCheck),
                theme.success,
                say!("inbox-review-approved"),
            )
        } else if any(ReviewState::Pending) || pull.approval == Approval::Required {
            (
                Icon::new(Extra::Hourglass),
                theme.warning,
                say!("inbox-review-awaited"),
            )
        } else if any(ReviewState::Commented) {
            (
                Icon::new(Extra::MessageSquare),
                theme.foreground,
                say!("inbox-review-commented"),
            )
        } else {
            return dash(cx);
        };
    div()
        .id(SharedString::from(format!(
            "inbox-review-mark-{}",
            pull.key
        )))
        .child(icon.size(px(14.)).text_color(colour))
        .tooltip(move |window, cx| Tooltip::new(said).build(window, cx))
        .into_any_element()
}

/// The checks summed up, as a mark.
fn checks_mark(state: CiState, cx: &App) -> AnyElement {
    if state == CiState::None {
        return dash(cx);
    }
    let (icon, colour) = ci_icon(state, cx);
    icon.size(px(14.)).text_color(colour).into_any_element()
}

/// `+12 / −3` and the blocks that say how large that is.
fn lines_text(additions: Option<u64>, deletions: Option<u64>, cx: &App) -> Option<AnyElement> {
    let theme = cx.theme();
    let blocks = facts::size_blocks(additions, deletions)?;
    let colour = match blocks {
        1 => theme.success,
        2 => theme.info,
        3 => theme.warning,
        _ => theme.danger,
    };
    Some(
        h_flex()
            .flex_none()
            .gap(px(4.))
            .items_center()
            .text_xs()
            .child(
                div()
                    .text_color(theme.success)
                    .child(format!("+{}", additions.unwrap_or(0))),
            )
            .child(div().text_color(theme.muted_foreground).child("/"))
            .child(
                div()
                    .text_color(theme.danger)
                    .child(format!("−{}", deletions.unwrap_or(0))),
            )
            .child(
                h_flex()
                    .ml(px(4.))
                    .gap(px(2.))
                    .children((0..blocks).map(|_| div().size(px(7.)).bg(colour))),
            )
            .into_any_element(),
    )
}

/// The labels, the first three and a count of the rest.
fn labels_line(labels: &[Label], cx: &App) -> Vec<AnyElement> {
    let mut out: Vec<AnyElement> = labels
        .iter()
        .take(3)
        .map(|label| label_chip(label, cx))
        .collect();
    if labels.len() > 3 {
        out.push(count_chip(labels.len() - 3, cx));
    }
    out
}

/// A milestone's mark and title.
fn milestone_line(milestone: Option<&Milestone>, cx: &App) -> Option<AnyElement> {
    let milestone = milestone?;
    let theme = cx.theme();
    Some(
        h_flex()
            .flex_none()
            .gap(px(4.))
            .items_center()
            .text_xs()
            .text_color(theme.muted_foreground)
            .child(Icon::new(gpui_kit::assets::IconName::Milestone).size(px(12.)))
            .child(shown_line(&milestone.title, 40))
            .into_any_element(),
    )
}

/// When a snoozed row comes back.
fn snooze_line(snoozed: &bitbraid_integrations::inbox::Snoozed, now: i64, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let offset = bitbraid_git::search::Zone::system().offset_at(now);
    h_flex()
        .flex_none()
        .gap(px(4.))
        .items_center()
        .text_xs()
        .text_color(theme.primary)
        .child(Icon::new(gpui_kit::assets::IconName::AlarmClock).size(px(12.)))
        .child(facts::snooze_end(snoozed, now, offset))
        .into_any_element()
}

/// A person's picture, with their name under the pointer.
fn person(user: &User, size: f32, cx: &App) -> AnyElement {
    let name = crate::views::pulls::card::who(user);
    div()
        .id(SharedString::from(format!(
            "inbox-person-{}-{size}",
            user.login
        )))
        .child(picture(user, size, cx))
        .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
        .into_any_element()
}

/// The others in a row: the first few, overlapping.
fn people<'a>(users: impl Iterator<Item = &'a User>, cx: &App) -> AnyElement {
    let mut seen: Vec<&User> = Vec::new();
    for user in users {
        if !user.login.is_empty() && !seen.iter().any(|known| known.login == user.login) {
            seen.push(user);
        }
    }
    h_flex()
        .children(seen.iter().take(5).enumerate().map(|(at, user)| {
            div()
                .when(at > 0, |picture| picture.ml(px(-6.)))
                .child(person(user, PERSON_PICTURE, cx))
        }))
        .into_any_element()
}

/// A branch's name in a chip, after a branch's mark.
fn branch_chip(name: String, cx: &App) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .max_w(px(REPO_WIDTH - 12.))
        .min_w_0()
        .px(px(6.))
        .py(px(1.))
        .gap(px(4.))
        .items_center()
        .rounded(px(3.))
        .bg(theme.muted)
        .text_xs()
        .child(
            Icon::new(gpui_kit::assets::IconName::GitBranch)
                .size(px(12.))
                .text_color(theme.muted_foreground),
        )
        .child(div().min_w_0().truncate().child(shown_line(&name, 120)))
        .into_any_element()
}

/// The repository's name over its branch (or what goes in its place),
/// marked when the repository is here.
fn repo_cell(
    repo: &str,
    branch: Option<&str>,
    here: bool,
    under: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let name = repo.rsplit_once('/').map_or(repo, |(_, name)| name);
    let full = repo.to_owned();
    v_flex()
        .id(SharedString::from(format!(
            "inbox-repo-{repo}-{}",
            branch.unwrap_or("")
        )))
        .w(px(REPO_WIDTH))
        .flex_none()
        .pr(px(10.))
        .gap(px(4.))
        .child(
            h_flex()
                .gap(px(4.))
                .items_center()
                .text_xs()
                .text_color(theme.muted_foreground)
                .when(here, |line| {
                    line.child(
                        Icon::new(gpui_kit::assets::IconName::HardDrive)
                            .size(px(11.))
                            .text_color(theme.success),
                    )
                })
                .child(div().min_w_0().truncate().child(shown_line(name, 80))),
        )
        .children(branch.map(|branch| branch_chip(branch.to_owned(), cx)))
        .children(under)
        .tooltip(move |window, cx| {
            Tooltip::new(if here {
                say!("inbox-repo-here", repo = full.as_str())
            } else {
                say!("inbox-repo-not-here", repo = full.as_str())
            })
            .build(window, cx)
        })
        .into_any_element()
}

/// The way to copy a row's link, shown under the pointer.
fn copy_link(key: &str, url: &str, cx: &mut Context<InboxView>) -> AnyElement {
    let url = url.to_owned();
    div()
        .w(px(30.))
        .mr(px(COPY_RIGHT))
        .flex_none()
        .flex()
        .justify_center()
        .invisible()
        .group_hover(SharedString::from(format!("inbox-row-{key}")), |style| {
            style.visible()
        })
        .child(
            Button::new(SharedString::from(format!("inbox-copy-{key}")))
                .accessibility_label(say!("inbox-copy-link"))
                .ghost()
                .xsmall()
                .icon(IconName::Copy)
                .tooltip(say!("inbox-copy-link"))
                .on_click(cx.listener(move |_, _, _, cx| {
                    if crate::providers::openable(&url) {
                        cx.write_to_clipboard(ClipboardItem::new_string(url.clone()));
                    }
                })),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_repository_is_cloned_from_its_site() {
        assert_eq!(
            super::clone_address("https://github.com/o/r/pull/3", "o/r"),
            "https://github.com/o/r.git"
        );
        assert_eq!(
            super::clone_address(
                "https://gitlab.example.org/g/s/p/-/merge_requests/1",
                "g/s/p"
            ),
            "https://gitlab.example.org/g/s/p.git"
        );
        assert_eq!(super::clone_address("nothing", "o/r"), "");
    }
}
