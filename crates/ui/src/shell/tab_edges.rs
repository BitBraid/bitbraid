//! What every tab of the tab bar draws over its own box: the lines at its
//! sides and along its bottom, and the accent of the shown tab. They lie
//! over the box and take none of its room, so that showing a tab, passing
//! over one or showing a pinned tab's page changes colours and never where
//! anything is.
//!
//! Every tab has a line down its right edge, and the first of the bar
//! (the page of repositories' pinned tab) one down its left edge too: the
//! line between two tabs is drawn once, by the left one, whichever of
//! them is shown. The first of the repositories' tabs drawn follows the
//! inbox's pinned tab directly and shares its line. A tab that is not
//! shown has the bar's line along its bottom; the shown one is open
//! towards what it shows and has an accent along its top, inside its box.
//! The lines are one of the toolkit's pixels thick at every zoom, as the
//! bar's other lines are.
//!
//! Sources: none (original design).

use gpui_kit::component::ActiveTheme;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

/// How thick the shown tab's accent is, in points.
const ACCENT: f32 = 2.;

/// The id of a tab's side line: what the tests find the lines by.
const SIDE: &str = "tab-side";
/// The id of a tab's bottom line, and of the shown tab's accent.
const BASE: &str = "tab-base";
const ACCENT_ID: &str = "tab-accent";

/// What a tab draws over its box: `leading` when it is the first of the
/// bar, `shown` while it is the shown tab.
pub(super) fn tab_edges(leading: bool, shown: bool, cx: &App) -> Vec<AnyElement> {
    let theme = cx.theme();
    let line = gpui_kit::px(1.);
    let side = |left: bool| {
        div()
            .id((SIDE, usize::from(left)))
            .absolute()
            .top_0()
            .bottom_0()
            .w(line)
            .bg(theme.border)
            .map(|rule| if left { rule.left_0() } else { rule.right_0() })
            .test_support()
            .into_any_element()
    };
    let mut edges = Vec::with_capacity(3);
    if leading {
        edges.push(side(true));
    }
    edges.push(side(false));
    if shown {
        // Between the side lines, from the top down.
        edges.push(
            div()
                .id(ACCENT_ID)
                .absolute()
                .top_0()
                .left(if leading { line } else { gpui_kit::px(0.) })
                .right(line)
                .h(crate::zoom::px(ACCENT))
                .bg(theme.primary)
                .test_support()
                .into_any_element(),
        );
    } else {
        edges.push(
            div()
                .id(BASE)
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(line)
                .bg(theme.border)
                .test_support()
                .into_any_element(),
        );
    }
    edges
}
