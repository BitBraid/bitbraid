//! The top of the inbox: its heading, its tabs with their counts, the
//! row with folding, refresh and the search, and the row of choices
//! (which workspace, which account, labels, milestones, whose).
//!
//! The measures are the same at every window size: everything keeps its
//! place from the left, the search grows.
//!
//! Sources: none (original design).

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    spinner::Spinner,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{InboxView, NAME, Tab, Whose, facts};
use crate::zoom::px;

/// From the window's left edge to the tabs, the rows under them and the
/// heading; the search ends as far from the right edge.
pub(super) const SIDE: f32 = 40.;
/// Above the heading, and its line.
const HEADING_TOP: f32 = 37.;
const HEADING_HEIGHT: f32 = 32.;
/// The tabs' row under the heading, and the room at each side of a tab's
/// words.
const TABS_TOP: f32 = 26.;
const TABS_HEIGHT: f32 = 43.;
const TAB_PAD: f32 = 18.;
/// The row with folding and the search.
const TOOLS_TOP: f32 = 22.;
const TOOLS_HEIGHT: f32 = 34.;
/// The row of choices: each a pill this wide and high, this far apart.
const CHOICES_TOP: f32 = 7.;
const CHOICE_HEIGHT: f32 = 28.;
const CHOICE_WIDTH: f32 = 168.;
const CHOICE_GAP: f32 = 11.;
/// From the choices to the list.
pub(super) const LIST_TOP: f32 = 20.;

impl InboxView {
    /// The heading, the tabs, the tools and the choices.
    pub(super) fn top(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let counts = self
            .answer
            .as_ref()
            .map(|answer| facts::counts(answer, &self.choices, self.now))
            .unwrap_or_default();
        let tabs = Tab::ALL.into_iter().map(|tab| {
            let count = counts
                .iter()
                .find(|(known, _)| *known == tab)
                .map_or(0, |(_, count)| *count);
            let active = tab == self.tab;
            h_flex()
                .id(SharedString::from(format!("inbox-tab-{}", tab.key())))
                .h_full()
                .px(px(TAB_PAD))
                .gap(px(7.))
                .items_center()
                .cursor_pointer()
                .text_xs()
                .border_b_2()
                .map(|this| {
                    if active {
                        this.bg(theme.secondary)
                            .border_color(theme.foreground.opacity(0.6))
                            .text_color(theme.foreground)
                    } else {
                        this.border_color(transparent_black())
                            .text_color(theme.muted_foreground)
                            .hover(|style| style.text_color(theme.foreground))
                    }
                })
                .child(tab.title().to_uppercase())
                .child(
                    div()
                        .h(px(18.))
                        .min_w(px(20.))
                        .px(px(5.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(theme.muted)
                        .text_color(if active {
                            theme.primary
                        } else {
                            theme.muted_foreground
                        })
                        .child(count.to_string()),
                )
                .on_click(cx.listener(move |this, _, _, cx| this.set_tab(tab, cx)))
                .test_support()
        });
        let grouped = self.tab == Tab::Pulls;
        let offline = self
            .answer
            .as_ref()
            .is_some_and(|answer| answer.accounts.iter().any(|state| state.freshness.offline));
        let refresh = Button::new("inbox-refresh")
            .accessibility_label(say!("inbox-refresh"))
            .ghost()
            .small()
            .icon(IconName::RefreshCw)
            .tooltip(say!("inbox-refresh"))
            .loading(self.reading)
            .on_click(cx.listener(|this, _, _, cx| this.read(true, cx)));
        let tools = h_flex()
            .mt(px(TOOLS_TOP))
            .h(px(TOOLS_HEIGHT))
            .w_full()
            .pl(px(6.))
            .gap(px(4.))
            .items_center()
            .when(grouped, |row| {
                row.child(
                    Button::new("inbox-collapse")
                        .ghost()
                        .small()
                        .icon(gpui_kit::assets::IconName::ChevronsDownUp)
                        .label(say!("inbox-collapse-all"))
                        .on_click(cx.listener(|this, _, _, cx| this.fold_all(true, cx))),
                )
                .child(
                    Button::new("inbox-expand")
                        .ghost()
                        .small()
                        .icon(IconName::ChevronsUpDown)
                        .label(say!("inbox-expand-all"))
                        .on_click(cx.listener(|this, _, _, cx| this.fold_all(false, cx))),
                )
            })
            .child(refresh)
            .child(
                div().ml(px(8.)).flex_1().min_w_0().child(
                    Input::new(&self.search)
                        .id("inbox-search")
                        .prefix(Icon::new(IconName::Search).size(px(16.)))
                        .min_h(px(TOOLS_HEIGHT))
                        .w_full(),
                ),
            );
        v_flex()
            .w_full()
            .flex_none()
            .px(px(SIDE))
            .child(
                h_flex()
                    .mt(px(HEADING_TOP))
                    .h(px(HEADING_HEIGHT))
                    .pl(px(2.))
                    .gap(px(12.))
                    .items_center()
                    .child(
                        Icon::new(IconName::Inbox)
                            .size(px(24.))
                            .text_color(theme.primary),
                    )
                    .child(div().text_size(px(24.)).font_semibold().child(NAME.text()))
                    .when(self.reading, |row| row.child(Spinner::new().small()))
                    .when(offline, |row| {
                        // What a provider could not be asked for was kept
                        // from before.
                        row.child(
                            div()
                                .id("inbox-offline")
                                .child(
                                    Icon::new(gpui_kit::assets::IconName::CloudOff)
                                        .size(px(16.))
                                        .text_color(theme.warning),
                                )
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(say!(
                                        "inbox-offline"
                                    ))
                                    .build(window, cx)
                                })
                                .test_support(),
                        )
                    }),
            )
            .child(h_flex().mt(px(TABS_TOP)).h(px(TABS_HEIGHT)).children(tabs))
            .child(tools)
            .child(self.choices_row(cx))
            .into_any_element()
    }

    /// The row of choices of the tab on show.
    fn choices_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let view = cx.entity().downgrade();
        let answer = self.answer.clone().unwrap_or_default();
        let pill = |id: &'static str, icon: Option<Icon>, label: String| {
            Button::new(id)
                .outline()
                .small()
                .rounded_full()
                .w(px(CHOICE_WIDTH))
                .h(px(CHOICE_HEIGHT))
                .justify_start()
                .dropdown_caret(true)
                .when_some(icon, |button, icon| button.icon(icon))
                .label(label)
        };
        // Which workspace: none, or one of the local ones.
        let workspace_label = self
            .workspace
            .as_ref()
            .and_then(|id| self.workspaces.iter().find(|(known, _)| known == id))
            .map_or_else(
                || say!("inbox-workspace").to_owned(),
                |(_, name)| name.clone(),
            );
        let workspaces = self.workspaces.clone();
        let chosen = self.workspace.clone();
        let workspace = pill(
            "inbox-workspace",
            Some(Icon::new(gpui_kit::assets::IconName::Layers)),
            workspace_label,
        )
        .dropdown_menu({
            let view = view.clone();
            move |menu, _, _| {
                let pick = |id: Option<String>| {
                    let view = view.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = view.update(cx, |this, cx| this.set_workspace(id.clone(), cx));
                    }
                };
                let menu = menu.scrollable(true).max_h(px(320.)).item(
                    PopupMenuItem::new(say!("inbox-no-workspace"))
                        .checked(chosen.is_none())
                        .on_click(pick(None)),
                );
                workspaces.iter().fold(menu, |menu, (id, name)| {
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(chosen.as_ref() == Some(id))
                            .on_click(pick(Some(id.clone()))),
                    )
                })
            }
        });
        // Which account.
        let accounts = crate::providers::accounts(cx);
        let account_label = self
            .choices
            .account
            .as_ref()
            .and_then(|id| accounts.iter().find(|account| &account.id == id))
            .map_or_else(
                || say!("inbox-all-accounts").to_owned(),
                crate::providers::account_label,
            );
        let account_icon = self
            .choices
            .account
            .as_ref()
            .and_then(|id| accounts.iter().find(|account| &account.id == id))
            .map_or_else(
                || Icon::new(gpui_kit::assets::IconName::Users),
                |account| crate::providers::Site::of(account).icon(),
            );
        let chosen = self.choices.account.clone();
        let account = pill("inbox-account", Some(account_icon), account_label).dropdown_menu({
            let view = view.clone();
            move |menu, _, _| {
                let pick = |id: Option<String>| {
                    let view = view.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = view.update(cx, |this, cx| {
                            this.choices.account = id.clone();
                            cx.notify();
                        });
                    }
                };
                let menu = menu.item(
                    PopupMenuItem::new(say!("inbox-all-accounts"))
                        .checked(chosen.is_none())
                        .on_click(pick(None)),
                );
                accounts.iter().fold(menu, |menu, account| {
                    menu.item(
                        PopupMenuItem::new(crate::providers::account_label(account))
                            .icon(crate::providers::Site::of(account).icon())
                            .checked(chosen.as_ref() == Some(&account.id))
                            .on_click(pick(Some(account.id.clone()))),
                    )
                })
            }
        });
        let listed = |id: &'static str,
                      icon: Icon,
                      all: &'static str,
                      names: Vec<String>,
                      chosen: Option<String>,
                      set: fn(&mut InboxView, Option<String>)| {
            let view = view.clone();
            pill(
                id,
                Some(icon),
                chosen.clone().unwrap_or_else(|| all.to_owned()),
            )
            .dropdown_menu(move |menu, _, _| {
                let pick = |name: Option<String>| {
                    let view = view.clone();
                    move |_: &ClickEvent, _: &mut Window, cx: &mut App| {
                        let _ = view.update(cx, |this, cx| {
                            set(this, name.clone());
                            cx.notify();
                        });
                    }
                };
                let menu = menu.scrollable(true).max_h(px(320.)).item(
                    PopupMenuItem::new(all)
                        .checked(chosen.is_none())
                        .on_click(pick(None)),
                );
                names.iter().fold(menu, |menu, name| {
                    menu.item(
                        PopupMenuItem::new(name.clone())
                            .checked(chosen.as_ref() == Some(name))
                            .on_click(pick(Some(name.clone()))),
                    )
                })
            })
        };
        let labels = listed(
            "inbox-labels",
            Icon::new(gpui_kit::assets::IconName::Tag),
            say!("inbox-all-labels"),
            facts::labels(&answer),
            self.choices.label.clone(),
            |this, name| this.choices.label = name,
        );
        let milestones = listed(
            "inbox-milestones",
            Icon::new(gpui_kit::assets::IconName::Milestone),
            say!("inbox-all-milestones"),
            facts::milestones(&answer),
            self.choices.milestone.clone(),
            |this, name| this.choices.milestone = name,
        );
        let whose_choices: &'static [Whose] = match self.tab {
            Tab::Issues => &Whose::FOR_ISSUES,
            _ => &Whose::FOR_PULLS,
        };
        let chosen = self.choices.whose;
        let whose = pill(
            "inbox-whose",
            Some(Icon::new(gpui_kit::assets::IconName::ListFilter)),
            chosen.title().to_owned(),
        )
        .dropdown_menu(move |menu, _, _| {
            whose_choices.iter().fold(menu, |menu, whose| {
                let (view, whose) = (view.clone(), *whose);
                menu.item(
                    PopupMenuItem::new(whose.title())
                        .checked(chosen == whose)
                        .on_click(move |_, _, cx| {
                            let _ = view.update(cx, |this, cx| {
                                this.choices.whose = whose;
                                cx.notify();
                            });
                        }),
                )
            })
        });
        // In a window too narrow for them in one line (a large zoom), on
        // more than one, rather than cut off at the window's edge.
        h_flex()
            .mt(px(CHOICES_TOP))
            .min_h(px(CHOICE_HEIGHT))
            .flex_wrap()
            .gap(px(CHOICE_GAP))
            .child(workspace)
            .when(self.tab != Tab::Work, |row| {
                row.child(account).child(labels).child(milestones)
            })
            .when(matches!(self.tab, Tab::Pulls | Tab::Issues), |row| {
                row.child(whose)
            })
            .into_any_element()
    }
}
