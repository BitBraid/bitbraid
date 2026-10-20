//! The list of keyboard shortcuts: a dialog over the window.
//!
//! A dialog 673 wide and 517 high, centred, 83 under the window's top, the
//! same at every size of the window. A title row 46 high with the key that
//! opens the list, a field that finds a key by what it does or by the key,
//! and a cross. Then the keys by where they apply, each place a heading
//! with a rule under it, each key a row 30 high: what it does from 40, the
//! key from 390 further on, a cap for every part of it, `or` between the
//! keys of one action. Only what has a key is listed. Changing a key is
//! not done here but on the page of keys the settings open.
//!
//! Sources: none (original design).

use gpui_kit::component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::{
    keymap::{BINDINGS, Keys, display_key},
    views::shortcuts::matches,
};

/// Width of the dialog.
pub const WIDTH: f32 = 673.0;
/// Height of the dialog.
pub const HEIGHT: f32 = 517.0;
/// From the window's top to the dialog.
pub const TOP: f32 = 83.0;
/// Height of the title row.
const TITLE_HEIGHT: f32 = 46.0;
/// Height of a key's row.
const ROW_HEIGHT: f32 = 30.0;
/// From the dialog's left edge to what a key does.
const LABEL_LEFT: f32 = 40.0;
/// From what a key does to the key.
const KEY_COLUMN: f32 = 390.0;

/// What the dialog asks of the window.
#[derive(Debug, Clone)]
pub enum KeyListEvent {
    /// Put the dialog away.
    Close,
}

impl EventEmitter<KeyListEvent> for KeyListView {}

/// The list of keyboard shortcuts.
pub struct KeyListView {
    filter: Entity<InputState>,
    query: String,
    _subscriptions: Vec<Subscription>,
}

impl KeyListView {
    /// Create the view.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(say!("shortcuts-filter")));
        let typed = cx.subscribe_in(&filter, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        Self {
            filter,
            query: String::new(),
            _subscriptions: vec![typed],
        }
    }

    /// Put the cursor in the field, with nothing typed.
    pub fn focus_filter(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.filter.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.query.clear();
    }
}

/// A key as caps: one for every part of every stroke.
fn caps(key: &str, cx: &App) -> Div {
    let theme = cx.theme();
    let (border, ground) = (theme.border, theme.secondary);
    let shown = display_key(key);
    h_flex()
        .gap_1()
        .items_center()
        .children(shown.split(' ').enumerate().map(|(n, stroke)| {
            h_flex()
                .gap(px(3.))
                .when(n > 0, |stroke| stroke.ml_1())
                .children(
                    stroke
                        .split('+')
                        .filter(|part| !part.is_empty())
                        .map(|part| {
                            div()
                                .px(px(5.))
                                .h(px(20.))
                                .line_height(px(18.))
                                .rounded(px(3.))
                                .border_1()
                                .border_color(border)
                                .bg(ground)
                                .text_sm()
                                .child(part.to_owned())
                        }),
                )
        }))
}

/// The keys of one action, `or` between them.
fn keys_of(keys: &[String], cx: &App) -> Div {
    let muted = cx.theme().muted_foreground;
    let mut row = h_flex().gap_2().items_center();
    for (n, key) in keys.iter().enumerate() {
        if n > 0 {
            row = row.child(
                div()
                    .text_sm()
                    .text_color(muted)
                    .child(say!("shortcuts-or")),
            );
        }
        row = row.child(caps(key, cx));
    }
    row
}

impl Render for KeyListView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let resolved = cx.global::<Keys>().resolved.clone();
        let opens = resolved.keys_of("help.shortcuts").to_vec();
        let mut list: Vec<AnyElement> = Vec::new();
        let mut place: Option<&'static str> = None;
        for bound in &resolved.bound {
            let Some(binding) = BINDINGS.get(bound.index) else {
                continue;
            };
            if bound.keys.is_empty() || !matches(binding, &bound.keys, &self.query) {
                continue;
            }
            if place != Some(binding.place()) {
                place = Some(binding.place());
                list.push(
                    div()
                        .flex_none()
                        .mt(px(if list.is_empty() { 8. } else { 16. }))
                        .pb_2()
                        .mb_1()
                        .border_b_1()
                        .border_color(theme.border)
                        .text_lg()
                        .child(binding.place())
                        .into_any_element(),
                );
            }
            list.push(
                h_flex()
                    .h(px(ROW_HEIGHT))
                    .flex_none()
                    .items_center()
                    .text_sm()
                    .child(
                        div()
                            .w(px(KEY_COLUMN))
                            .flex_none()
                            .pr_3()
                            .truncate()
                            .child(binding.label.text().trim_end_matches('…')),
                    )
                    .child(keys_of(&bound.keys, cx))
                    .into_any_element(),
            );
        }
        let none = list.is_empty();
        v_flex()
            .id("key-list")
            .w(px(WIDTH))
            .h(px(HEIGHT))
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(px(TITLE_HEIGHT))
                    .flex_none()
                    .pl(px(20.))
                    .pr_3()
                    .gap_3()
                    .items_center()
                    .bg(theme.title_bar)
                    .child(div().text_lg().child(say!("shortcuts-title")))
                    .child(keys_of(&opens, cx))
                    .child(div().flex_1())
                    .child(div().w(px(180.)).child(Input::new(&self.filter).small()))
                    .child(
                        Button::new("key-list-close")
                            .accessibility_label(say!("shortcuts-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(KeyListEvent::Close))),
                    ),
            )
            .child(
                v_flex()
                    .id("key-list-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .pl(px(LABEL_LEFT))
                    .pr(px(LABEL_LEFT))
                    .pb_4()
                    .overflow_y_scroll()
                    .children(list)
                    .when(none, |list| {
                        list.child(
                            div()
                                .pt_4()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(say!("shortcuts-no-key-does-that")),
                        )
                    }),
            )
            // Its place is asked for in the tests.
            .test_support()
    }
}
