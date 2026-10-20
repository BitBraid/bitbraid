//! The pinned tabs at the left end of the tab bar: the page of
//! repositories, then the inbox. Each shows a view of the window's own in
//! place of the tab's content, as wide as its mark (and the inbox's count),
//! on a ground tinted with the theme's accent, drawn as the shown tab while
//! its view is on show, and never closed. Their lines and the shown one's
//! accent are every tab's (`super::tab_edges`); the first of the
//! repositories' tabs follows the inbox's at once, the line between them
//! the inbox's, as between any two tabs: their tinted ground sets them
//! apart.
//!
//! Sources: none (original design).

use gpui_kit::component::{ActiveTheme, Icon, IconName, h_flex, tooltip::Tooltip};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::tab_edges::tab_edges;
use crate::zoom::px;

/// A pinned tab is as wide as what is in it: its mark this far in from
/// each side, and what follows the mark. It is never narrower than this,
/// so that alone the mark stands in a square.
const INSET: f32 = 11.;
const LEAST: f32 = 36.;
/// From the mark to what follows it.
const GAP: f32 = 5.;

/// One pinned tab: `id` to find it by, its mark (found by `mark_id`),
/// what it says in its tooltip and to assistive technology, whether its
/// view is on show, whether it is the first of the pinned tabs, what
/// follows the mark (the inbox's count), and what a click does.
#[allow(clippy::too_many_arguments)]
pub(super) fn pinned_tab(
    id: &'static str,
    mark_id: &'static str,
    icon: IconName,
    said: SharedString,
    shown: bool,
    leading: bool,
    after: Option<AnyElement>,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .relative()
        .h_full()
        .flex_none()
        .min_w(px(LEAST))
        .px(px(INSET))
        .gap(px(GAP))
        .justify_center()
        .items_center()
        .text_sm()
        .cursor_pointer()
        // A tab for assistive technology, selected while its view is on show.
        .role(Role::Tab)
        .aria_label(said.clone())
        .aria_selected(shown)
        .map(|tab| {
            if shown {
                // As a shown repository's tab: open towards what it
                // shows, the mark in the accent.
                tab.bg(theme.title_bar).text_color(theme.primary)
            } else {
                let hovered = theme.primary.opacity(0.14);
                let hover_text = theme.tab_active_foreground;
                tab.bg(theme.primary.opacity(0.08))
                    .text_color(theme.tab_foreground)
                    .hover(move |style| style.bg(hovered).text_color(hover_text))
            }
        })
        .child(
            div()
                .id(mark_id)
                .flex_none()
                .child(Icon::new(icon).size_3p5())
                .test_support(),
        )
        .children(after)
        .children(tab_edges(leading, shown, cx))
        .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
        .on_click(on_click)
        .test_support()
        .into_any_element()
}
