//! Merging a pull request and its place in a stack.
//!
//! The merge box says whether it merges cleanly, is held back, or
//! conflicts (`mergeable`); a split button merges it in the way chosen
//! among those the repository allows (a merge commit, one squashed
//! commit, its commits replayed), each with a line that says what it
//! does; a box removes its branch afterwards. Merging is asked in the bar
//! first. In a stack (GitHub), the box lists the stack from the top down
//! to the branch the bottom one goes into, and offers to move the one
//! above this onto its base when it is merged (`pr.merge` with
//! `retarget_stack`); the heading says where in the stack it is, with
//! buttons to the one below and the one above.
//!
//! Sources: docs.github.com/en/rest/pulls/pulls ("Merge a pull request",
//! "Update a pull request" for the base);
//! docs.github.com/en/repositories/configuring-branches-and-merges-in-your-repository/configuring-pull-request-merges;
//! docs.gitlab.com/api/merge_requests ("Merge a merge request").

use bitbraid_integrations::model::{
    MergeMethod, Mergeable, Merged, ProviderKind, PullDetails, PullState, Stack, StackEntry,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{PullView, PullViewEvent};
use crate::{
    ask::Ask,
    notices::Notice,
    views::pulls::{
        calls,
        facts::{merge_line, number_text, shown_line},
    },
    zoom::px,
};

/// What a way to merge is called.
pub fn method_name(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => say!("pulls-method-merge"),
        MergeMethod::Squash => say!("pulls-method-squash"),
        MergeMethod::Rebase => say!("pulls-method-rebase"),
    }
}

/// What a way to merge does, in a line.
pub fn method_line(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => say!("pulls-method-merge-line"),
        MergeMethod::Squash => say!("pulls-method-squash-line"),
        MergeMethod::Rebase => say!("pulls-method-rebase-line"),
    }
}

/// What the merge button says for a way to merge.
pub fn merge_label(method: MergeMethod) -> &'static str {
    match method {
        MergeMethod::Merge => say!("pulls-merge"),
        MergeMethod::Squash => say!("pulls-merge-squash"),
        MergeMethod::Rebase => say!("pulls-merge-rebase"),
    }
}

/// What the bar asks before it merges.
fn merge_question(kind: ProviderKind, number: u64, base: &str, method: MergeMethod) -> String {
    let named = number_text(kind, number);
    let base = shown_line(base, 80);
    match method {
        MergeMethod::Merge => say!("pulls-merge-question", named, base),
        MergeMethod::Squash => say!("pulls-merge-question-squash", named, base),
        MergeMethod::Rebase => say!("pulls-merge-question-rebase", named, base),
    }
}

impl PullView {
    /// The way it is merged: the one chosen, while the repository allows
    /// it; else the first it allows.
    pub(super) fn method(&self) -> Option<MergeMethod> {
        let allowed = &self.details.as_ref()?.merge_methods;
        self.method
            .filter(|method| allowed.contains(method))
            .or_else(|| allowed.first().copied())
    }

    /// Choose how it is merged.
    pub fn choose_method(&mut self, method: MergeMethod, cx: &mut Context<Self>) {
        self.method = Some(method);
        cx.notify();
    }

    /// Whether its branch is removed after it is merged.
    pub fn set_delete_branch(&mut self, on: bool, cx: &mut Context<Self>) {
        self.delete_branch = on;
        cx.notify();
    }

    /// Whether the one above it in the stack is moved onto its base when
    /// it is merged.
    pub fn set_retarget(&mut self, on: bool, cx: &mut Context<Self>) {
        self.retarget = on;
        cx.notify();
    }

    /// The pull request above this one in its stack.
    pub(super) fn above(&self) -> Option<&StackEntry> {
        let stack = self.stack.as_ref()?;
        stack
            .entries
            .get(stack.position.checked_add(1)?)
            .filter(|entry| entry.state == PullState::Open)
    }

    /// The one below this one in its stack.
    fn below(&self) -> Option<&StackEntry> {
        let stack = self.stack.as_ref()?;
        stack.entries.get(stack.position.checked_sub(1)?)
    }

    /// Whether the merge button may be pressed, or why not.
    fn merge_refused(&self, details: &PullDetails) -> Option<&'static str> {
        if details.pull.state != PullState::Open {
            return Some(say!("pulls-merge-refused-not-open"));
        }
        if details.pull.draft {
            return Some(say!("pulls-merge-refused-draft"));
        }
        if details.mergeable == Mergeable::Conflicts {
            return Some(say!("pulls-merge-refused-conflicts"));
        }
        if self.method().is_none() {
            return Some(say!("pulls-merge-refused-no-method"));
        }
        if self.sending.is_some() {
            return Some(say!("pulls-merge-refused-busy"));
        }
        None
    }

    /// Ask in the bar, then merge.
    pub fn ask_merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(details), Some(method)) = (&self.details, self.method()) else {
            return;
        };
        if self.merge_refused(details).is_some() {
            return;
        }
        let question = merge_question(self.link.kind, self.number, &details.pull.base_ref, method);
        let view = cx.entity().downgrade();
        crate::ask::show(
            Ask::new(question).go(merge_label(method), move |_, window, cx| {
                let _ = view.update(cx, |this, cx| this.merge(window, cx));
            }),
            window,
            cx,
        );
    }

    /// Merge it at the provider, as chosen.
    pub fn merge(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(method) = self.method() else {
            return;
        };
        let retarget = self.retarget && self.above().is_some();
        let kind = self.link.kind;
        let number = self.number;
        let extra = json!({
            "method": method,
            "delete_branch": self.delete_branch,
            "retarget_stack": retarget,
        });
        self.send::<Merged>(
            "pr.merge",
            extra,
            say!("pulls-merging"),
            window,
            cx,
            move |this, merged, window, cx| {
                if !merged.merged {
                    this.write_error = Some(say!(
                        "pulls-merge-failed",
                        why = merged
                            .message
                            .as_deref()
                            .map(|said| shown_line(said, 300))
                            .unwrap_or_else(|| say!("pulls-merge-not-done").to_owned())
                    ));
                    return;
                }
                let mut said = say!("pulls-merged-notice", named = number_text(kind, number));
                if merged.branch_deleted {
                    said.push_str(say!("pulls-merged-branch-removed"));
                }
                if !merged.retargeted.is_empty() {
                    let moved: Vec<String> = merged
                        .retargeted
                        .iter()
                        .take(10)
                        .map(|number| number_text(kind, *number))
                        .collect();
                    said.push_str(&say!(
                        "pulls-merged-retargeted",
                        count = moved.len(),
                        moved = moved.join(", ")
                    ));
                }
                let notice = match merged
                    .message
                    .as_deref()
                    .filter(|said| !said.trim().is_empty())
                {
                    Some(more) => Notice::warning(format!("{said}. {}", shown_line(more, 300))),
                    None => Notice::success(said),
                };
                crate::notices::push(window, cx, notice);
                this.refresh(cx);
                this.stack = None;
                this.read_stack(cx);
                cx.emit(PullViewEvent::Changed);
            },
        );
    }

    /// Its stack, at GitHub.
    pub(super) fn read_stack(&mut self, cx: &mut Context<Self>) {
        if self.link.kind != ProviderKind::Github {
            return;
        }
        let asked = calls::ask::<Stack>(
            "pr.stack",
            json!({
                "account": self.link.account,
                "owner": self.link.owner,
                "name": self.link.name,
                "number": self.number,
            }),
        );
        let generation = self.generation;
        cx.spawn(async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if this.generation != generation {
                    return;
                }
                // A stack of one is no stack; what could not be read
                // shows nothing.
                this.stack = answer.ok().filter(|stack| {
                    stack.entries.len() > 1 && stack.position < stack.entries.len()
                });
                cx.notify();
            });
        })
        .detach();
    }

    /// Where in its stack it is, `2/3`, with buttons to the one below and
    /// the one above.
    pub(super) fn render_stack_place(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let stack = self.stack.as_ref()?;
        let theme = cx.theme().clone();
        let place = stack.position.saturating_add(1);
        let below = self.below().map(|entry| entry.number);
        let above = stack
            .entries
            .get(stack.position.saturating_add(1))
            .map(|entry| entry.number);
        let kind = self.link.kind;
        Some(
            h_flex()
                .id("pull-stack-place")
                .flex_none()
                .gap_0p5()
                .items_center()
                .child(
                    Button::new("pull-stack-down")
                        .accessibility_label(say!("pulls-stack-below"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronLeft)
                        .disabled(below.is_none())
                        .when_some(below, |button, number| {
                            button.tooltip(say!(
                                "pulls-stack-below-named",
                                named = number_text(kind, number)
                            ))
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if let Some(number) = below {
                                cx.emit(PullViewEvent::Open(number));
                            }
                        })),
                )
                .child(
                    div()
                        .px_1()
                        .rounded(px(3.))
                        .bg(theme.muted)
                        .text_xs()
                        .font_semibold()
                        .child(format!("{place}/{}", stack.entries.len())),
                )
                .child(
                    Button::new("pull-stack-up")
                        .accessibility_label(say!("pulls-stack-above"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::ChevronRight)
                        .disabled(above.is_none())
                        .when_some(above, |button, number| {
                            button.tooltip(say!(
                                "pulls-stack-above-named",
                                named = number_text(kind, number)
                            ))
                        })
                        .on_click(cx.listener(move |_, _, _, cx| {
                            if let Some(number) = above {
                                cx.emit(PullViewEvent::Open(number));
                            }
                        })),
                )
                .test_support()
                .into_any_element(),
        )
    }

    /// The stack in the merge box: from the top down, this one marked,
    /// then the branch the bottom one goes into.
    fn render_stack_list(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let stack = self.stack.as_ref()?;
        let theme = cx.theme().clone();
        let kind = self.link.kind;
        let base = stack
            .entries
            .first()
            .map(|entry| shown_line(&entry.base_ref, 80))?;
        let rows: Vec<AnyElement> =
            stack
                .entries
                .iter()
                .enumerate()
                .rev()
                .take(50)
                .map(|(at, entry)| {
                    let here = at == stack.position;
                    let number = entry.number;
                    let colour = match entry.state {
                        PullState::Open => theme.success,
                        PullState::Merged => theme.primary,
                        PullState::Closed => theme.danger,
                    };
                    h_flex()
                        .id(("pull-stack-entry", at))
                        .w_full()
                        .h(px(22.))
                        .px_1()
                        .gap_1p5()
                        .items_center()
                        .rounded(px(3.))
                        .text_xs()
                        .when(here, |row| row.bg(theme.list_active).font_semibold())
                        .when(!here, |row| {
                            row.cursor_pointer()
                                .hover(|style| style.bg(theme.accent))
                                .on_click(cx.listener(move |_, _, _, cx| {
                                    cx.emit(PullViewEvent::Open(number))
                                }))
                        })
                        .child(
                            Icon::new(gpui_kit::assets::IconName::GitPullRequest)
                                .size_3p5()
                                .flex_none()
                                .text_color(colour),
                        )
                        .child(
                            div()
                                .flex_none()
                                .text_color(theme.muted_foreground)
                                .child(number_text(kind, number)),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .child(shown_line(&entry.title, 200)),
                        )
                        .child(
                            div()
                                .flex_none()
                                .ml_auto()
                                .font_family(theme.mono_font_family.clone())
                                .text_color(theme.muted_foreground)
                                .child(shown_line(&entry.head_ref, 60)),
                        )
                        .test_support()
                        .into_any_element()
                })
                .collect();
        Some(
            v_flex()
                .id("pull-stack")
                .w_full()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child(say!("pulls-stack-heading")),
                )
                .children(rows)
                .child(
                    h_flex()
                        .h(px(22.))
                        .px_1()
                        .gap_1p5()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::ArrowDown).size_3p5().flex_none())
                        .child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .child(base),
                        ),
                )
                .into_any_element(),
        )
    }

    /// The merge box.
    pub(super) fn render_merge_box(
        &self,
        details: &PullDetails,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let open = details.pull.state == PullState::Open;
        let (line, good) = if open {
            merge_line(details.mergeable, &shown_line(&details.pull.base_ref, 120))
        } else {
            (
                match details.pull.state {
                    PullState::Merged => say!("pulls-was-merged").to_owned(),
                    _ => say!("pulls-was-closed").to_owned(),
                },
                None,
            )
        };
        let (icon, colour) = match (details.mergeable, good) {
            (Mergeable::Blocked, _) if open => (Icon::new(IconName::TriangleAlert), theme.warning),
            (_, Some(true)) => (Icon::new(IconName::CircleCheck), theme.success),
            (_, Some(false)) => (Icon::new(IconName::TriangleAlert), theme.danger),
            (_, None) => (Icon::new(IconName::Info), theme.muted_foreground),
        };
        let refused = self.merge_refused(details);
        let method = self.method();
        let allowed = details.merge_methods.clone();
        let view = cx.entity().downgrade();
        let head = shown_line(&details.pull.head_ref, 80);
        let base = shown_line(&details.pull.base_ref, 80);
        let above = self.above().map(|entry| entry.number);
        let kind = self.link.kind;
        let stack = self.render_stack_list(cx);
        let merge_box = v_flex()
            .id("pull-merge-box")
            .flex_1()
            .min_w_0()
            .p_3()
            .gap_2()
            .rounded(px(6.))
            .border_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .text_sm()
                    .child(icon.size_4().flex_none().text_color(colour))
                    .child(div().min_w_0().child(line)),
            )
            .children(stack)
            .when(open, |merge_box| {
                merge_box
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                h_flex()
                                    .flex_none()
                                    .child(
                                        Button::new("pull-merge")
                                            .primary()
                                            .small()
                                            .label(method.map_or(say!("pulls-merge"), merge_label))
                                            .loading(self.sending == Some(say!("pulls-merging")))
                                            .disabled(refused.is_some())
                                            .when_some(refused, |button, why| button.tooltip(why))
                                            .on_click(cx.listener(|this, _, window, cx| {
                                                this.ask_merge(window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new("pull-merge-how")
.accessibility_label(say!("pulls-merge-how"))
                                            .primary()
                                            .small()
                                            .icon(IconName::ChevronDown)
                                            .disabled(allowed.len() < 2 || refused.is_some())
                                            .tooltip(say!("pulls-merge-how"))
                                            .dropdown_menu(move |menu: PopupMenu, _, _| {
                                                let mut menu = menu.min_w(px(300.));
                                                for way in &allowed {
                                                    let (view, way) = (view.clone(), *way);
                                                    menu = menu.item(
                                                        PopupMenuItem::element(move |_, cx: &mut App| {
                                                            v_flex()
                                                                .py_0p5()
                                                                .child(div().font_semibold().child(method_name(way)))
                                                                .child(
                                                                    div()
                                                                        .text_xs()
                                                                        .text_color(cx.theme().muted_foreground)
                                                                        .child(method_line(way)),
                                                                )
                                                        })
                                                        .checked(method == Some(way))
                                                        .on_click(move |_, _, cx| {
                                                            let _ = view.update(cx, |this, cx| {
                                                                this.choose_method(way, cx)
                                                            });
                                                        }),
                                                    );
                                                }
                                                menu
                                            }),
                                    ),
                            )
                            .children(method.map(|method| {
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .whitespace_normal()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(method_line(method))
                            })),
                    )
                    .child(
                        Checkbox::new("pull-merge-delete")
                            .label(say!("pulls-merge-remove-branch", head = &head))
                            .checked(self.delete_branch)
                            .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                this.set_delete_branch(*checked, cx)
                            })),
                    )
                    .when_some(above, |merge_box, number| {
                        merge_box.child(
                            Checkbox::new("pull-merge-retarget")
                                .label(say!(
                                    "pulls-merge-retarget",
                                    named = number_text(kind, number),
                                    base = &base
                                ))
                                .checked(self.retarget)
                                .on_click(cx.listener(|this, checked: &bool, _, cx| {
                                    this.set_retarget(*checked, cx)
                                })),
                        )
                    })
            })
            .test_support();
        // The box stands in from the conversation with the mark of a pull
        // request beside it.
        h_flex()
            .w_full()
            .flex_none()
            .gap(px(MERGE_MARK_GAP))
            .items_start()
            .child(
                Icon::new(gpui_kit::assets::IconName::GitPullRequest)
                    .size(px(MERGE_MARK))
                    .flex_none()
                    .mt(px(MERGE_MARK_TOP))
                    .text_color(theme.muted_foreground),
            )
            .child(merge_box)
            .into_any_element()
    }
}

/// The mark beside the merge box: its size, the room after it and how far
/// down it stands.
const MERGE_MARK: f32 = 18.0;
const MERGE_MARK_GAP: f32 = 10.0;
const MERGE_MARK_TOP: f32 = 12.0;

#[cfg(test)]
mod tests {
    use bitbraid_integrations::model::{MergeMethod, ProviderKind};

    use super::{merge_label, merge_question};

    #[test]
    fn the_bar_says_how_it_merges() {
        assert_eq!(
            merge_question(ProviderKind::Gitlab, 7, "main", MergeMethod::Squash),
            "Merge !7 into main as one squashed commit? It cannot be taken back from here."
        );
        assert_eq!(merge_label(MergeMethod::Rebase), "Rebase and merge");
    }
}
