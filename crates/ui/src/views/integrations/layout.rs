//! Where the parts of a provider's page stand. The integrations pages are
//! laid out apart from the other settings: the heading 20 from the column
//! of providers; under it a body 35 in from that column and 15 short of
//! the window's edge, at most 870 wide, with the cards centred in it and
//! the rows of a form whose words end a third of the way across, less 16.
//! The controls of the rows that connect an account (a server's address,
//! a token) begin 30 after the words, those of the SSH key 20 after them.
//! Fields reach to 15 short of the body's end; one row stands 15 under
//! the one before it.
//!
//! Sources: gpui-kit documentation (scroll).

use gpui_kit::component::{
    h_flex,
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

/// The column of providers, and a provider's row in it.
pub(super) const COLUMN_WIDTH: f32 = 260.;
pub(super) const SITE_HEIGHT: f32 = 43.;
/// The heading, from the column.
const HEADING_LEFT: f32 = 20.;
/// The heading's box and how far under the top it starts.
const HEADING_TOP: f32 = 16.;
const HEADING_LINE: f32 = 32.;
/// From the heading's box to the body.
const AFTER_HEADING: f32 = 17.;
/// The body: from the column, from the page's right edge, and the widest
/// it gets.
const BODY_LEFT: f32 = 35.;
const BODY_RIGHT: f32 = 15.;
const BODY_WIDTH: f32 = 870.;
/// The share of the body that is the column of words, what is left
/// between the words and that column's end, and where the controls begin
/// after it: in the rows of the SSH key, and in the form that connects an
/// account.
const WORDS_SHARE: f32 = 0.335;
const WORDS_END: f32 = 16.;
const CONTROL_IN: f32 = 4.;
const FORM_CONTROL_IN: f32 = 14.;
/// What a field leaves at the body's end.
const CONTROL_END: f32 = 15.;
/// From one row (or card) to the next.
pub(super) const GAP: f32 = 15.;
/// A field, and a button in a row of the form.
pub(super) const FIELD_HEIGHT: f32 = 34.;
pub(super) const BUTTON_HEIGHT: f32 = 31.;
/// A line of the words.
const WORDS_LINE: f32 = 20.;

/// A provider's page: its heading, and under it the body, which scrolls
/// with it.
pub(super) fn page(
    heading: SharedString,
    body: Vec<AnyElement>,
    scroll: &ScrollHandle,
) -> AnyElement {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(
            v_flex()
                .id("integrations-page")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .pt(px(HEADING_TOP))
                .pb(px(48.))
                .child(
                    div()
                        .pl(px(HEADING_LEFT))
                        .h(px(HEADING_LINE))
                        .flex_none()
                        .text_2xl()
                        .line_height(px(HEADING_LINE))
                        .font_weight(FontWeight::MEDIUM)
                        .child(heading),
                )
                .child(
                    div().w_full().pl(px(BODY_LEFT)).pr(px(BODY_RIGHT)).child(
                        v_flex()
                            .id("integrations-body")
                            .w_full()
                            .max_w(px(BODY_WIDTH))
                            .mt(px(AFTER_HEADING))
                            .gap(px(GAP))
                            .children(body),
                    ),
                ),
        )
        .scrollbar(scroll, ScrollbarAxis::Vertical)
        .into_any_element()
}

/// Something centred in the body: a card.
pub(super) fn centred(element: impl IntoElement) -> AnyElement {
    h_flex()
        .w_full()
        .justify_center()
        .child(element)
        .into_any_element()
}

/// A row of the SSH key's part: the words right-aligned in their column
/// (none for a row that continues the one above), the control 20 after
/// them, as wide as the body lets it be when `fill`, and what is said
/// under it in small print.
pub(super) fn row(
    words: impl Into<SharedString>,
    control: AnyElement,
    fill: bool,
    under: Option<AnyElement>,
    muted: Hsla,
) -> AnyElement {
    row_at(CONTROL_IN, words, control, fill, under, muted)
}

/// A row of the form that connects an account: as [`row`], the control 30
/// after the words.
pub(super) fn form_row(
    words: impl Into<SharedString>,
    control: AnyElement,
    fill: bool,
    under: Option<AnyElement>,
    muted: Hsla,
) -> AnyElement {
    row_at(FORM_CONTROL_IN, words, control, fill, under, muted)
}

fn row_at(
    control_in: f32,
    words: impl Into<SharedString>,
    control: AnyElement,
    fill: bool,
    under: Option<AnyElement>,
    muted: Hsla,
) -> AnyElement {
    h_flex()
        .w_full()
        .items_start()
        .child(
            div()
                .w(relative(WORDS_SHARE))
                .flex_none()
                .min_h(px(FIELD_HEIGHT))
                .pt(px((FIELD_HEIGHT - WORDS_LINE) / 2.))
                .pr(px(WORDS_END))
                .text_sm()
                .line_height(px(WORDS_LINE))
                .text_right()
                .whitespace_normal()
                .child(words.into()),
        )
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .pl(px(control_in))
                .pr(px(CONTROL_END))
                .gap(px(6.))
                .child(
                    h_flex()
                        .min_h(px(FIELD_HEIGHT))
                        .items_center()
                        .when(fill, |line| line.w_full())
                        .child(control),
                )
                .children(under.map(|under| {
                    div()
                        .text_xs()
                        .line_height(px(17.5))
                        .text_color(muted)
                        .whitespace_normal()
                        .child(under)
                })),
        )
        .into_any_element()
}
