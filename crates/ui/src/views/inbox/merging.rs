//! The question a row's Merge asks before anything is merged: which pull
//! request into which branch, what stands in the way as far as is known
//! (changes asked, failed checks, conflicts), how (a merge commit, one
//! squashed commit, or its commits replayed), and whether its branch goes
//! afterwards. It lies over the inbox, 600 wide, near the top; `pr.merge`
//! does the rest.
//!
//! Sources: docs.github.com/en/rest "Pull requests" (merge);
//! docs.gitlab.com/api "Merge requests" (merge).

use bitbraid_integrations::{
    inbox::{Approval, PullGroup},
    model::{CiState, MergeMethod, Mergeable, ReviewState},
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{InboxView, facts};
use crate::{views::pulls::facts::shown_line, zoom::px};

/// The dialog's width and how far under the inbox's top it stands.
const WIDTH: f32 = 600.;
const TOP: f32 = 50.;

impl InboxView {
    pub(super) fn merge_dialog(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(merging) = &self.merging else {
            return div().into_any_element();
        };
        let Some(pull) = self.pull(&merging.key) else {
            return div().into_any_element();
        };
        let row = &pull.pull;
        // What is known to stand in the way, a line each.
        let changes = row
            .reviewers
            .iter()
            .any(|reviewer| reviewer.state == ReviewState::ChangesRequested)
            || pull.approval == Approval::ChangesRequested;
        let mut lines: Vec<(bool, String)> = vec![if changes {
            (false, say!("inbox-merge-changes-asked").to_owned())
        } else {
            (true, say!("inbox-merge-no-changes-asked").to_owned())
        }];
        lines.push(match row.ci {
            CiState::Failure | CiState::Cancelled => {
                (false, say!("inbox-merge-checks-failed").to_owned())
            }
            CiState::Pending | CiState::Running => {
                (false, say!("inbox-merge-checks-running").to_owned())
            }
            CiState::Success => (true, say!("inbox-merge-checks-passed").to_owned()),
            _ => (true, say!("inbox-merge-no-checks").to_owned()),
        });
        if pull.mergeable == Mergeable::Conflicts {
            lines.push((
                false,
                say!(
                    "inbox-merge-conflicts",
                    base = shown_line(&row.base_ref, 60)
                ),
            ));
        } else if !pull.groups.contains(&PullGroup::ReadyToMerge) {
            lines.push((false, say!("inbox-merge-may-refuse").to_owned()));
        }
        let methods = [
            (MergeMethod::Merge, say!("inbox-merge-method-merge")),
            (MergeMethod::Squash, say!("inbox-merge-method-squash")),
            (MergeMethod::Rebase, say!("inbox-merge-method-rebase")),
        ];
        let chosen = merging.method;
        let busy = merging.busy;
        let delete = merging.delete_branch;
        let view = cx.entity().downgrade();
        div()
            .id("inbox-merge-backdrop")
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .pt(px(TOP))
            .bg(theme.background.opacity(0.6))
            .occlude()
            .on_click(cx.listener(|this, _, _, cx| {
                if !this.merging.as_ref().is_some_and(|merging| merging.busy) {
                    this.merging = None;
                    cx.notify();
                }
            }))
            .child(
                v_flex()
                    .id("inbox-merge")
                    .w(px(WIDTH))
                    .rounded(theme.radius_lg)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.popover)
                    .shadow_lg()
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        h_flex()
                            .h(px(52.))
                            .px(px(18.))
                            .gap(px(10.))
                            .items_center()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(Icon::new(gpui_kit::assets::IconName::GitMerge).size(px(16.)))
                            .child(div().flex_1().min_w_0().truncate().text_base().child(say!(
                                "inbox-merge-heading",
                                pull = facts::short_name(pull),
                                base = shown_line(&row.base_ref, 60)
                            )))
                            .child(
                                Button::new("inbox-merge-close")
                                    .accessibility_label(say!("inbox-merge-close"))
                                    .ghost()
                                    .small()
                                    .icon(IconName::Close)
                                    .disabled(busy)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.merging = None;
                                        cx.notify();
                                    })),
                            ),
                    )
                    .child(
                        v_flex()
                            .p(px(20.))
                            .gap(px(14.))
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .child(shown_line(&row.title, 200)),
                            )
                            .children(lines.into_iter().map(|(fine, text)| {
                                h_flex()
                                    .gap(px(10.))
                                    .items_center()
                                    .text_sm()
                                    .child(
                                        Icon::new(if fine {
                                            IconName::CircleCheck
                                        } else {
                                            IconName::CircleX
                                        })
                                        .size(px(16.))
                                        .text_color(
                                            if fine { theme.success } else { theme.danger },
                                        ),
                                    )
                                    .child(text)
                            }))
                            .child(h_flex().gap(px(6.)).children(methods.into_iter().map(
                                |(method, label)| {
                                    crate::views::chosen(
                                        Button::new(SharedString::from(format!(
                                            "inbox-merge-method-{label}"
                                        )))
                                        .outline()
                                        .small()
                                        .label(label)
                                        .disabled(busy)
                                        .on_click(
                                            cx.listener(move |this, _, _, cx| {
                                                if let Some(merging) = this.merging.as_mut() {
                                                    merging.method = method;
                                                }
                                                cx.notify();
                                            }),
                                        ),
                                        method == chosen,
                                        cx,
                                    )
                                },
                            )))
                            .child(
                                Checkbox::new("inbox-merge-delete")
                                    .label(say!("inbox-merge-delete-branch"))
                                    .checked(delete)
                                    .disabled(busy)
                                    .on_click(move |on: &bool, _, cx: &mut App| {
                                        let on = *on;
                                        let _ = view.update(cx, |this, cx| {
                                            if let Some(merging) = this.merging.as_mut() {
                                                merging.delete_branch = on;
                                            }
                                            cx.notify();
                                        });
                                    }),
                            )
                            .when_some(merging.error.clone(), |body, error| {
                                body.child(
                                    div()
                                        .id("inbox-merge-error")
                                        .text_sm()
                                        .text_color(theme.danger)
                                        .child(shown_line(&error, 400))
                                        .test_support(),
                                )
                            })
                            .child(
                                h_flex()
                                    .gap(px(8.))
                                    .justify_end()
                                    .child(
                                        Button::new("inbox-merge-cancel")
                                            .ghost()
                                            .small()
                                            .label(say!("inbox-merge-cancel"))
                                            .disabled(busy)
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.merging = None;
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("inbox-merge-go")
                                            .primary()
                                            .small()
                                            .label(say!("inbox-merge-go"))
                                            .loading(busy)
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.merge(window, cx)
                                            })),
                                    ),
                            ),
                    ),
            )
            .into_any_element()
    }
}
