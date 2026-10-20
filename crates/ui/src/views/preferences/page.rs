//! The page the preferences are: a column of sections at the left, and the
//! section that is chosen at the right, as a form under its heading.
//!
//! The page takes everything under the tab bar. The column is 300 wide at
//! every window size and scrolls as a whole: the way out at its top, who is
//! committed as, the sections every repository shares, and after them the
//! sections of the repository whose tab is open. The section scrolls on its
//! own, its heading with it. A section is groups of rows (`row.rs`), a
//! group under a heading of its own when it has one.
//!
//! Sources: gpui-kit documentation (scroll, icon).

use std::rc::Rc;

use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt, h_flex,
    scroll::{ScrollableElement, ScrollbarAxis},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

/// The column of sections.
pub(crate) const COLUMN_WIDTH: f32 = 300.;
/// From the column to the section's heading and its form.
pub(crate) const INSET: f32 = 55.;
/// A section's row in the column.
const SECTION_HEIGHT: f32 = 45.75;
/// The way out, the first row of the column.
const WAY_OUT_HEIGHT: f32 = 44.;
/// From the top of the section to its heading's box.
const HEADING_TOP: f32 = 16.;
/// The heading's box.
const HEADING_LINE: f32 = 32.;
/// From the heading to the first row, when no group heading comes first.
const AFTER_HEADING: f32 = 8.;
/// From the heading to a group heading that comes first.
const BEFORE_GROUP: f32 = 14.;
/// From the last row of a group to the next group's heading.
const BETWEEN_GROUPS: f32 = 34.;
/// A group heading's box.
const GROUP_LINE: f32 = 24.;
/// From a group heading to its first row.
const AFTER_GROUP: f32 = 9.;
/// From one row to the next.
pub(crate) const ROW_GAP: f32 = 8.;
/// The widest a paragraph or a rule across the form gets.
pub(crate) const WIDE: f32 = 1140.;
/// What is across the form leaves this much at the section's right edge:
/// as much as is left of the column.
const WIDE_END: f32 = INSET - 1.;

/// What draws something across the form.
type Across = Rc<dyn Fn(&mut Window, &mut App) -> AnyElement>;

/// What a section shows: rows, or something as wide as the form.
pub(crate) enum Block {
    /// A row of the form.
    Row(super::row::Item),
    /// Anything across the form: a paragraph, a list, a table.
    Wide(Across),
    /// Groups of a view of its own, which lays out its rows and headings
    /// with [`subheading`] and [`rows`].
    Own(Across),
}

/// Rows under a heading of their own, or under none.
pub(crate) struct Group {
    title: Option<SharedString>,
    /// A rule under the heading, across the form.
    rule: bool,
    blocks: Vec<Block>,
}

impl Group {
    /// A group without a heading.
    pub(crate) fn new() -> Self {
        Self {
            title: None,
            rule: false,
            blocks: Vec::new(),
        }
    }

    /// A group under a heading.
    pub(crate) fn titled(title: impl Into<SharedString>) -> Self {
        Self {
            title: Some(title.into()),
            ..Self::new()
        }
    }

    /// With a rule under the heading.
    pub(crate) fn ruled(mut self) -> Self {
        self.rule = true;
        self
    }

    /// A row.
    pub(crate) fn item(mut self, item: super::row::Item) -> Self {
        self.blocks.push(Block::Row(item));
        self
    }
}

impl Group {
    /// Groups a view draws itself, in the form's measures.
    pub(crate) fn own(
        mut self,
        render: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.blocks.push(Block::Own(Rc::new(render)));
        self
    }

    /// Something across the form.
    pub(crate) fn wide(
        mut self,
        render: impl Fn(&mut Window, &mut App) -> AnyElement + 'static,
    ) -> Self {
        self.blocks.push(Block::Wide(Rc::new(render)));
        self
    }
}

/// A paragraph across the form, in the size of the rows' words.
pub(crate) fn paragraph(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .w_full()
        .text_sm()
        .line_height(px(20.))
        .text_color(cx.theme().foreground)
        .whitespace_normal()
        .child(text.into())
        .into_any_element()
}

impl Default for Group {
    fn default() -> Self {
        Self::new()
    }
}

/// What is across the form, to the width it goes to.
pub(crate) fn across(element: AnyElement) -> AnyElement {
    div()
        .w_full()
        .pr(px(WIDE_END))
        .child(div().w_full().max_w(px(WIDE)).child(element))
        .into_any_element()
}

/// The heading of a group, a rule under it when asked.
pub(crate) fn subheading(title: impl Into<SharedString>, rule: bool, cx: &App) -> AnyElement {
    div()
        .w_full()
        .max_w(px(WIDE))
        .h(px(GROUP_LINE))
        .mb(px(AFTER_GROUP))
        .text_lg()
        .line_height(px(GROUP_LINE))
        .font_semibold()
        .when(rule, |this| {
            this.h(px(GROUP_LINE + 8.))
                .border_b_1()
                .border_color(cx.theme().border)
        })
        .child(title.into())
        .into_any_element()
}

/// The rows of a group, as far apart as rows are.
pub(crate) fn rows(rows: Vec<AnyElement>) -> Div {
    v_flex().w_full().gap(px(ROW_GAP)).children(rows)
}

/// A group of a view of its own: from the one before, its heading, its
/// rows.
pub(crate) fn own_group(
    title: Option<SharedString>,
    first: bool,
    blocks: Vec<AnyElement>,
    cx: &App,
) -> AnyElement {
    let gap = match (first, title.is_some()) {
        (true, false) => 0.,
        (true, true) => BEFORE_GROUP - AFTER_HEADING,
        (false, _) => BETWEEN_GROUPS,
    };
    v_flex()
        .w_full()
        .mt(px(gap))
        .when_some(title, |this, title| {
            this.child(subheading(title, false, cx))
        })
        .child(rows(blocks))
        .into_any_element()
}

/// The section that is chosen, under its heading.
pub(crate) fn section(
    heading: SharedString,
    groups: Vec<Group>,
    scroll: &ScrollHandle,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let mut column = v_flex()
        .id("preferences-section")
        .size_full()
        .overflow_y_scroll()
        .track_scroll(scroll)
        .pl(px(INSET))
        .pr(px(1.))
        .pt(px(HEADING_TOP))
        .pb(px(48.))
        .child(
            div()
                .h(px(HEADING_LINE))
                .flex_none()
                .text_2xl()
                .line_height(px(HEADING_LINE))
                .font_medium()
                .child(heading),
        );
    let mut first = true;
    for group in groups {
        let gap = match (first, group.title.is_some()) {
            (true, false) => AFTER_HEADING,
            (true, true) => BEFORE_GROUP,
            (false, _) => BETWEEN_GROUPS,
        };
        first = false;
        let blocks: Vec<AnyElement> = group
            .blocks
            .iter()
            .map(|block| match block {
                Block::Row(item) => item.render(window, cx),
                Block::Wide(render) => across(render(window, cx)),
                Block::Own(render) => render(window, cx),
            })
            .collect();
        column = column.child(
            v_flex()
                .w_full()
                .flex_none()
                .mt(px(gap))
                .when_some(group.title, |this, title| {
                    this.child(subheading(title, group.rule, cx))
                })
                .child(rows(blocks)),
        );
    }
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .h_full()
        .child(column)
        .scrollbar(scroll, ScrollbarAxis::Vertical)
        .into_any_element()
}

/// A row of the column: an icon and a name, filled when it is the one
/// that is shown.
pub(crate) fn section_row(
    id: SharedString,
    icon: Icon,
    name: SharedString,
    chosen: bool,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .w_full()
        .h(px(SECTION_HEIGHT))
        .flex_none()
        .pl(px(30.))
        .gap(px(21.))
        .items_center()
        .cursor_pointer()
        .text_lg()
        .when(chosen, |this| this.bg(theme.list_active))
        .when(!chosen, |this| this.hover(|this| this.bg(theme.list_hover)))
        .child(
            div()
                .w(px(22.))
                .flex_none()
                .flex()
                .justify_center()
                .child(icon.size(px(20.))),
        )
        .child(div().min_w_0().truncate().child(name))
        .on_click(on_click)
        .test_support()
        .into_any_element()
}

/// Room between the parts of the column.
pub(crate) fn space(height: f32) -> AnyElement {
    div().h(px(height)).flex_none().into_any_element()
}

/// A caption of the column.
pub(crate) fn caption(text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .px(px(10.))
        .text_sm()
        .line_height(px(20.))
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// The line under the caption of the repository's sections: its name.
pub(crate) fn repo_line(name: SharedString, cx: &App) -> AnyElement {
    h_flex()
        .px(px(10.))
        .pt(px(4.))
        .pb(px(8.))
        .gap(px(5.))
        .items_center()
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(gpui_kit::assets::IconName::Folder).size(px(14.)))
        .child(div().min_w_0().truncate().child(name))
        .into_any_element()
}

/// The way out, at the top of the column.
pub(crate) fn way_out(
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    h_flex()
        .id("preferences-close")
        .w_full()
        .h(px(WAY_OUT_HEIGHT))
        .flex_none()
        .pl(px(20.))
        .gap(px(7.))
        .items_center()
        .cursor_pointer()
        .text_lg()
        .hover(|this| this.bg(theme.list_hover))
        .child(Icon::new(gpui_kit::assets::IconName::ArrowLeftFromLine).size(px(18.)))
        .child(say!("preferences-close"))
        .on_click(on_click)
        .test_support()
        .into_any_element()
}

/// The column, with what is in it, scrolling as a whole.
pub(crate) fn column(rows: Vec<AnyElement>, scroll: &ScrollHandle, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .relative()
        .w(px(COLUMN_WIDTH))
        .h_full()
        .flex_none()
        .bg(theme.sidebar)
        .border_r_1()
        .border_color(theme.border)
        .child(
            v_flex()
                .id("preferences-column")
                .size_full()
                .overflow_y_scroll()
                .track_scroll(scroll)
                .pb(px(16.))
                .children(rows),
        )
        .scrollbar(scroll, ScrollbarAxis::Vertical)
        .into_any_element()
}
