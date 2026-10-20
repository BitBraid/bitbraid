//! The window's side of the inbox: its tab at the left end of the
//! tab bar, which shows it in place of the tab's content and puts the
//! repository tabs in the background while it is on show; the status
//! bar's summary of what waits most, whose panel opens the inbox on a
//! group; and what is done for it: a repository opened, a pull request
//! checked out or reviewed in its repository's tab, a worktree for its
//! branch, a branch for an issue, a clone or a folder chosen.
//!
//! Sources: none (original design).

use bitbraid_integrations::inbox::PullGroup;
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    popover::Popover,
    v_flex,
};
use gpui_kit::*;

use super::{CLONE_FROM, Shell, WindowSheet};
use crate::{
    views::inbox::{InboxEvent, InboxView, NAME, facts},
    views::workspace::Workspace,
    zoom::px,
};

/// The count: a pill this high, its figures this large.
const COUNT_HEIGHT: f32 = 16.;
const COUNT_TEXT: f32 = 10.;
/// Past this the count says that there are more.
const COUNT_MOST: usize = 99;
/// The status bar's panel: its width, and a line of it.
const SUMMARY_WIDTH: f32 = 436.;
const SUMMARY_LINE: f32 = 34.;

/// What the inbox's tab says in its tooltip and to assistive technology:
/// its name and what waits, when the inbox has an answer.
fn inbox_said(waiting: Option<(usize, usize)>) -> String {
    match waiting {
        None => NAME.text().to_owned(),
        Some((0, 0)) => say!("shell-inbox-nothing-waits", name = NAME.text()),
        Some((pulls, 0)) => say!("shell-inbox-pulls-wait", name = NAME.text(), pulls),
        Some((0, issues)) => say!("shell-inbox-issues-wait", name = NAME.text(), issues),
        Some((pulls, issues)) => say!("shell-inbox-both-wait", name = NAME.text(), pulls, issues),
    }
}

/// The count on the inbox's tab: the number, or that there are more than
/// it has room for.
fn count_said(count: usize) -> String {
    if count > COUNT_MOST {
        format!("{COUNT_MOST}+")
    } else {
        count.to_string()
    }
}

impl Shell {
    /// The inbox, made with the window.
    pub(super) fn make_inbox(
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Entity<InboxView>, Vec<Subscription>) {
        let view = cx.new(|cx| InboxView::new(window, cx));
        let events = cx.subscribe_in(&view, window, |this, _, event: &InboxEvent, window, cx| {
            this.on_inbox(event, window, cx)
        });
        // The status bar says what it says anew when the answer changes.
        let redraw = cx.observe(&view, |_, _, cx| cx.notify());
        (view, vec![events, redraw])
    }

    /// The inbox, for the tests.
    pub fn inbox(&self) -> Entity<InboxView> {
        self.inbox.clone()
    }

    /// Show the inbox in place of the tab's content.
    pub fn show_inbox(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sheet = Some(WindowSheet::Inbox);
        self.inbox.update(cx, |view, cx| view.set_shown(true, cx));
        self.take_focus(window, cx);
        cx.notify();
    }

    /// Show the inbox on one group of its pull requests.
    pub fn show_inbox_group(
        &mut self,
        group: PullGroup,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inbox.update(cx, |view, cx| view.open_group(group, cx));
        self.show_inbox(window, cx);
    }

    /// Put the inbox away, if it is shown: a tab is to be looked at.
    pub(super) fn leave_inbox(&mut self, cx: &mut Context<Self>) {
        if self.sheet == Some(WindowSheet::Inbox) {
            self.sheet = None;
            self.inbox.update(cx, |view, cx| view.set_shown(false, cx));
        }
    }

    /// The workspace of a repository's tab, the tab opened (or shown)
    /// first.
    fn workspace_of(
        &mut self,
        path: &std::path::Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<Workspace>> {
        self.leave_inbox(cx);
        self.open_repository(path.to_path_buf(), window, cx);
        self.take_focus(window, cx);
        self.active_workspace()
            .filter(|_| {
                self.tabs
                    .get(self.active)
                    .is_some_and(|tab| tab.path.as_deref() == Some(path))
            })
            .cloned()
    }

    fn on_inbox(&mut self, event: &InboxEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            InboxEvent::OpenRepository(path) => {
                let _ = self.workspace_of(path, window, cx);
            }
            InboxEvent::CheckOut {
                path,
                link,
                number,
                fork,
            } => {
                if let Some(workspace) = self.workspace_of(path, window, cx) {
                    let (link, number, fork) = (link.clone(), *number, fork.clone());
                    workspace.update(cx, |workspace, cx| {
                        workspace.checkout_pull_request(link, number, fork, window, cx)
                    });
                }
            }
            InboxEvent::Review { path, link, number } => {
                if let Some(workspace) = self.workspace_of(path, window, cx) {
                    let (link, number) = (link.clone(), *number);
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_pull(link, number, window, cx)
                    });
                }
            }
            InboxEvent::Worktree { path, link, number } => {
                if let Some(workspace) = self.workspace_of(path, window, cx) {
                    let (link, number) = (link.clone(), *number);
                    workspace.update(cx, |workspace, cx| {
                        workspace.add_pull_worktree(link, number, window, cx)
                    });
                }
            }
            InboxEvent::ShowIssue { path, link, number } => {
                if let Some(workspace) = self.workspace_of(path, window, cx) {
                    let (link, number) = (link.clone(), *number);
                    workspace.update(cx, |workspace, cx| {
                        workspace.open_issue(link, number, window, cx)
                    });
                }
            }
            InboxEvent::IssueBranch { path, link, issue } => {
                if let Some(workspace) = self.workspace_of(path, window, cx) {
                    let link = link.clone();
                    workspace.update(cx, |workspace, cx| {
                        workspace.ask_issue_branch(link, issue, window, cx)
                    });
                }
            }
            InboxEvent::Clone { url } => {
                self.prompt_clone(window, cx);
                super::form::set_row(self, CLONE_FROM, url.clone(), window, cx);
                self.suggest_clone_path(window, cx);
            }
            InboxEvent::Locate => self.browse_for_repository(window, cx),
            InboxEvent::OpenSettings => {
                self.show_preferences(crate::views::preferences::Section::Integrations, window, cx)
            }
        }
        cx.notify();
    }

    /// How many pull requests and issues wait in the inbox: what its tab
    /// of everything counts with nothing chosen, the snoozed left out.
    /// None until it has an answer.
    fn waiting(&self, cx: &App) -> Option<(usize, usize)> {
        let answer = self.inbox.read(cx).answer()?;
        let counts = facts::counts(answer, &facts::Choices::default(), crate::views::now());
        let count = |wanted: facts::Tab| {
            counts
                .iter()
                .find(|(tab, _)| *tab == wanted)
                .map_or(0, |(_, count)| *count)
        };
        Some((count(facts::Tab::Pulls), count(facts::Tab::Issues)))
    }

    /// The inbox's tab, the second of the pinned tabs at the left end of
    /// the tab bar (`super::pinned`): its mark and how many things wait,
    /// as wide as they are. Shown as the shown tab while the inbox is on
    /// show.
    pub(super) fn render_inbox_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let active = self.sheet == Some(WindowSheet::Inbox);
        let waiting = self.waiting(cx);
        let said: SharedString = inbox_said(waiting).into();
        let count = waiting
            .map(|(pulls, issues)| pulls.saturating_add(issues))
            .filter(|count| *count > 0)
            .map(|count| {
                let (ground, figures) = if active {
                    (theme.primary, theme.primary_foreground)
                } else {
                    (
                        theme.tab_active_foreground.opacity(0.13),
                        theme.tab_active_foreground,
                    )
                };
                div()
                    .id("inbox-count")
                    .flex_none()
                    .min_w(px(COUNT_HEIGHT))
                    .h(px(COUNT_HEIGHT))
                    .px(px(5.))
                    .rounded_full()
                    .bg(ground)
                    .text_color(figures)
                    .text_size(px(COUNT_TEXT))
                    .line_height(px(COUNT_HEIGHT))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_center()
                    .child(count_said(count))
                    .test_support()
                    .into_any_element()
            });
        let weak = cx.entity().downgrade();
        super::pinned::pinned_tab(
            "inbox-tab",
            "inbox-tab-mark",
            IconName::Inbox,
            said,
            active,
            false,
            count,
            move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.show_inbox(window, cx));
            },
            cx,
        )
    }

    /// The status bar's summary at its left end: the most urgent pull
    /// request and what it waits for, which opens a panel with every
    /// urgent group; or that no account is connected.
    pub(super) fn render_inbox_summary(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = cx.theme().clone();
        if crate::providers::accounts(cx).is_empty() {
            return Some(
                Button::new("inbox-summary-connect")
                    .ghost()
                    .xsmall()
                    .icon(Icon::new(IconName::Inbox).size_3())
                    .label(say!("shell-inbox-connect"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.show_preferences(
                            crate::views::preferences::Section::Integrations,
                            window,
                            cx,
                        )
                    }))
                    .into_any_element(),
            );
        }
        let now = crate::views::now();
        let urgent = facts::urgent(self.inbox.read(cx).answer()?, now);
        let line = facts::status_line(&urgent)?;
        let shell = cx.entity().downgrade();
        let lines: Vec<facts::Urgent> = urgent.clone();
        let reading = self.inbox.clone();
        Some(
            Popover::new("inbox-summary")
                .anchor(Anchor::BottomLeft)
                .trigger(
                    Button::new("inbox-summary-button")
                        .ghost()
                        .xsmall()
                        .icon(
                            Icon::new(IconName::Inbox)
                                .size_3()
                                .text_color(theme.primary),
                        )
                        .label(line),
                )
                .p_0()
                .content(move |_, _, cx| {
                    let theme = cx.theme().clone();
                    let rows = lines.iter().map(|urgent| {
                        let group = urgent.group;
                        let shell = shell.clone();
                        h_flex()
                            .id(SharedString::from(format!("inbox-summary-{}", group.key())))
                            .h(px(SUMMARY_LINE))
                            .px(px(14.))
                            .gap(px(10.))
                            .items_center()
                            .text_sm()
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.list_hover))
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(facts::urgent_line(urgent)),
                            )
                            .child(div().text_color(theme.muted_foreground).child("·"))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(urgent.first.clone()),
                            )
                            .on_click(move |_, window, cx| {
                                let _ = shell.update(cx, |shell, cx| {
                                    shell.show_inbox_group(group, window, cx)
                                });
                            })
                            .test_support()
                    });
                    let reading = reading.clone();
                    v_flex()
                        .w(px(SUMMARY_WIDTH))
                        .py(px(6.))
                        .child(
                            h_flex()
                                .h(px(40.))
                                .px(px(14.))
                                .gap(px(8.))
                                .items_center()
                                .border_b_1()
                                .border_color(theme.border)
                                .child(Icon::new(IconName::Inbox).size(px(16.)))
                                .child(div().flex_1().text_base().child(NAME.text()))
                                .child(
                                    Button::new("inbox-summary-refresh")
                                        .accessibility_label(say!("shell-inbox-refresh"))
                                        .ghost()
                                        .xsmall()
                                        .icon(IconName::RefreshCw)
                                        .tooltip(say!("shell-inbox-refresh"))
                                        .on_click(move |_, _, cx| {
                                            reading.update(cx, |view, cx| view.read(true, cx));
                                        }),
                                ),
                        )
                        .children(rows)
                        .into_any_element()
                })
                .into_any_element(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{count_said, inbox_said};

    #[test]
    fn the_inbox_tab_says_what_waits_and_counts_up_to_ninety_nine() {
        assert_eq!(inbox_said(None), "Inbox");
        assert_eq!(inbox_said(Some((0, 0))), "Inbox: nothing waits for you");
        assert_eq!(inbox_said(Some((1, 0))), "Inbox: 1 pull request");
        assert_eq!(inbox_said(Some((0, 3))), "Inbox: 3 issues");
        assert_eq!(
            inbox_said(Some((12, 1))),
            "Inbox: 12 pull requests and 1 issue"
        );
        assert_eq!(count_said(7), "7");
        assert_eq!(count_said(99), "99");
        assert_eq!(count_said(100), "99+");
        assert_eq!(count_said(usize::MAX), "99+");
    }
}
