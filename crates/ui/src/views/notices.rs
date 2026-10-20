//! What the application said: every notification of this run, newest
//! first, with what each offered to do still on offer, and under them what
//! is kept of earlier runs.

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::notices::{Kind, Notices};

/// Width of the panel.
pub const WIDTH: f32 = 450.0;
/// From the window's top to the panel: just under the tabs.
pub const TOP: f32 = 36.0;
/// What the panel leaves free under it, to the window's bottom.
pub const BELOW: f32 = 75.0;
/// Height of the panel's header.
const HEADER: f32 = 52.0;

/// What the view asks of the window.
#[derive(Debug, Clone)]
pub enum NoticesEvent {
    /// Put the view away.
    Close,
}

impl EventEmitter<NoticesEvent> for NoticesView {}

/// The history of notifications.
pub struct NoticesView {
    focus_handle: FocusHandle,
}

impl NoticesView {
    /// Create the view.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
        }
    }
}

impl NoticesView {
    /// One notice: what kind of news, what it said, what it offers, when.
    fn row(
        &self,
        ix: usize,
        entry: crate::notices::Entry,
        now: i64,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let (word, colour) = match entry.kind {
            Kind::Info => (say!("notices-kind-info"), theme.info),
            Kind::Success => (say!("notices-kind-success"), theme.success),
            Kind::Warning => (say!("notices-kind-warning"), theme.warning),
            Kind::Error => (say!("notices-kind-error"), theme.danger),
        };
        let icon = match entry.kind {
            Kind::Info => IconName::Info,
            Kind::Success => IconName::CircleCheck,
            Kind::Warning => IconName::TriangleAlert,
            Kind::Error => IconName::CircleX,
        };
        h_flex()
            .w_full()
            .px_4()
            .py_3()
            .gap_3()
            .items_start()
            .border_b_1()
            .border_color(theme.border.opacity(0.5))
            .child(
                div()
                    .flex_none()
                    .size(px(32.))
                    .rounded_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .bg(colour.opacity(0.18))
                    .child(Icon::new(icon).size_4().text_color(colour)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_xs()
                            .child(div().font_semibold().text_color(colour).child(word))
                            .child(
                                div()
                                    .text_color(theme.muted_foreground)
                                    .child(crate::views::relative_time(entry.at, now)),
                            ),
                    )
                    .child(
                        div()
                            .whitespace_normal()
                            // Much of what is said is git's own, with
                            // options and names in it.
                            .child(crate::literal::said(entry.text.clone(), cx)),
                    )
                    .when(!entry.actions.is_empty(), |text| {
                        text.child(h_flex().pt_1().gap_2().flex_wrap().children(
                            entry.actions.iter().enumerate().map(|(n, action)| {
                                let run = action.run.clone();
                                Button::new(("notice-do", ix * 8 + n))
                                    .outline()
                                    .xsmall()
                                    .label(action.label.clone())
                                    .on_click(move |_, window, cx| run(window, cx))
                            }),
                        ))
                    }),
            )
            .into_any_element()
    }
}

impl Focusable for NoticesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for NoticesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let now = crate::views::now();
        let entries: Vec<crate::notices::Entry> = cx
            .global::<Notices>()
            .entries
            .iter()
            .rev()
            .cloned()
            .collect();
        let none = entries.is_empty();
        // Where the notices of earlier runs begin, when both kinds are
        // there: a line says so.
        let earlier_from = entries
            .iter()
            .position(|entry| entry.earlier)
            .filter(|at| *at > 0);
        let only_earlier = entries.first().is_some_and(|entry| entry.earlier);
        let earlier_heading = || {
            div()
                .px_3()
                .pt_3()
                .pb_1()
                .text_xs()
                .font_semibold()
                .text_color(theme.muted_foreground)
                .child(say!("notices-earlier-runs"))
                .into_any_element()
        };
        let rows: Vec<AnyElement> = entries
            .into_iter()
            .enumerate()
            .flat_map(|(ix, entry)| {
                let heading =
                    (earlier_from == Some(ix) || (ix == 0 && only_earlier)).then(earlier_heading);
                heading
                    .into_iter()
                    .chain(std::iter::once(self.row(ix, entry, now, cx)))
            })
            .collect();
        v_flex()
            .id("notices-history")
            .track_focus(&self.focus_handle)
            .size_full()
            .child(
                h_flex()
                    .w_full()
                    .h(px(HEADER))
                    .flex_none()
                    .px_4()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_1().text_base().child(say!("notices-heading")))
                    .child(
                        Button::new("notices-clear")
                            .ghost()
                            .small()
                            .label(say!("notices-clear"))
                            .on_click(cx.listener(|_, _, _, cx| {
                                cx.global_mut::<Notices>().clear();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("notices-close")
                            .accessibility_label(say!("notices-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(NoticesEvent::Close))),
                    ),
            )
            .child(
                v_flex()
                    .id("notice-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .text_sm()
                    .children(rows)
                    .when(none, |list| {
                        list.child(
                            div()
                                .p_3()
                                .text_color(theme.muted_foreground)
                                .child(say!("notices-nothing")),
                        )
                    }),
            )
            // Its place is asked for in the tests.
            .test_support()
    }
}
