//! Issue keys drawn as links in a text: in a commit's message in the
//! details panel and in the summaries of the graph's rows.
//! A key (`#12`, `owner/name#12`, as `bitbraid_git::issue_keys` reads
//! them) of a repository the open one is linked to is drawn in the colour
//! of links and underlined; the pointer resting on it shows the issue's
//! card (`issue.show`, asked once and kept); a click opens the issue's
//! view. Without a linked remote a text is drawn as it is.
//!
//! Sources: gpui's interactive text (click ranges and tooltips).

use std::rc::Rc;

use bitbraid_integrations::model::RepoLink;
use gpui_kit::component::{ActiveTheme, tooltip::Tooltip};
use gpui_kit::*;

use super::{card::key_card, facts::key_links};

/// What a click on a key does: open the issue of this repository.
pub type Open = Rc<dyn Fn(RepoLink, u64, &mut Window, &mut App)>;

/// A text with its issue keys as links, or the text alone when it has
/// none that name an issue of a linked repository.
pub fn text(
    id: impl Into<ElementId>,
    text: impl Into<SharedString>,
    links: &[RepoLink],
    open: Open,
    cx: &App,
) -> AnyElement {
    let text: SharedString = text.into();
    let keys = key_links(&text, links);
    if keys.is_empty() {
        return text.into_any_element();
    }
    let colour = cx.theme().link;
    let highlights: Vec<(std::ops::Range<usize>, HighlightStyle)> = keys
        .iter()
        .map(|key| {
            (
                key.range.clone(),
                HighlightStyle {
                    color: Some(colour),
                    underline: Some(UnderlineStyle {
                        thickness: crate::zoom::px(1.),
                        color: Some(colour),
                        wavy: false,
                    }),
                    ..HighlightStyle::default()
                },
            )
        })
        .collect();
    let ranges: Vec<std::ops::Range<usize>> = keys.iter().map(|key| key.range.clone()).collect();
    let keys = Rc::new(keys);
    let clicked = keys.clone();
    InteractiveText::new(id, StyledText::new(text).with_highlights(highlights))
        .on_click(ranges, move |at, window, cx| {
            if let Some(key) = clicked.get(at) {
                open(key.link.clone(), key.number, window, cx);
            }
        })
        .tooltip(move |at, window, cx| {
            let key = keys.iter().find(|key| key.range.contains(&at))?.clone();
            Some(
                Tooltip::element(move |_, cx| key_card(&key.link, key.number, cx))
                    .build(window, cx),
            )
        })
        .into_any_element()
}
