//! The dialog that adds a filter to the pull requests section: a name
//! for its group and a query in GitHub's search words, with what the
//! words are and, as it is typed, what a provider leaves out of it
//! (GitLab is asked by fewer). The filter is kept with the repository's
//! state and has a group of its own under the four every repository has.
//!
//! Sources: docs.github.com/en/search-github (searching issues and pull
//! requests); docs.gitlab.com/api/merge_requests (list parameters).

use bitbraid_integrations::model::ProviderKind;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use super::{
    PullsView,
    facts::{QUALIFIERS, SavedFilter, filter_problem, left_out},
};
use crate::zoom::px;

/// How wide the dialog is.
const WIDTH: f32 = 560.0;

/// What the words of a query are, in a line.
pub fn words_line() -> String {
    let words: Vec<String> = QUALIFIERS.iter().map(|word| format!("{word}:")).collect();
    let (last, rest) = words
        .split_last()
        .map_or((String::new(), &[][..]), |(last, rest)| {
            (last.clone(), rest)
        });
    say!("pulls-filter-words", words = rest.join(", "), last)
}

/// Open the dialog. What it saves goes to `view`; the subscriptions that
/// redraw it as it is typed in are kept by the view while it is open.
pub fn open(
    view: WeakEntity<PullsView>,
    kinds: Vec<ProviderKind>,
    others: Vec<SavedFilter>,
    window: &mut Window,
    cx: &mut Context<PullsView>,
) -> Vec<Subscription> {
    let name =
        cx.new(|cx| InputState::new(window, cx).placeholder(say!("pulls-filter-name-placeholder")));
    let query = cx
        .new(|cx| InputState::new(window, cx).placeholder(say!("pulls-filter-query-placeholder")));
    let subscriptions = [&name, &query]
        .into_iter()
        .map(|input| {
            cx.subscribe_in(input, window, |_, _, event: &InputEvent, window, _| {
                if matches!(event, InputEvent::Change) {
                    window.refresh();
                }
            })
        })
        .collect();
    let mut kinds = kinds;
    kinds.sort();
    kinds.dedup();
    let first = name.clone();
    window.open_dialog(cx, move |dialog, _, cx| {
        let theme = cx.theme().clone();
        let typed_name = name.read(cx).value().to_string();
        let typed_query = query.read(cx).value().to_string();
        let problem = filter_problem(&typed_name, &typed_query, &others);
        let mut left: Vec<String> = Vec::new();
        for line in kinds.iter().flat_map(|kind| left_out(&typed_query, *kind)) {
            if !left.contains(&line) {
                left.push(line);
            }
        }
        let words = |text: &'static str| div().text_sm().pb(px(4.)).child(text);
        let body = v_flex()
            .w_full()
            .gap(px(14.))
            .child(
                v_flex()
                    .child(words(say!("pulls-filter-name")))
                    .child(Input::new(&name).id("pull-filter-name").w_full()),
            )
            .child(
                v_flex()
                    .child(words(say!("pulls-filter-query")))
                    .child(Input::new(&query).id("pull-filter-query").w_full())
                    .child(
                        div()
                            .pt(px(6.))
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .whitespace_normal()
                            .child(words_line()),
                    ),
            )
            .when(!left.is_empty(), |body| {
                body.child(
                    v_flex()
                        .id("pull-filter-left-out")
                        .gap_0p5()
                        .text_xs()
                        .text_color(theme.warning)
                        .children(left.into_iter().map(|line| div().child(line)))
                        .test_support(),
                )
            })
            .when_some(
                problem
                    .clone()
                    .filter(|_| !typed_name.is_empty() || !typed_query.is_empty()),
                |body, problem| {
                    body.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(problem),
                    )
                },
            );
        let (view, name_field, query_field) = (view.clone(), name.clone(), query.clone());
        dialog
            .title(
                h_flex()
                    .gap_2()
                    .child(Icon::new(gpui_kit::assets::IconName::ListFilterPlus).size(px(18.)))
                    .child(say!("pulls-add-filter")),
            )
            .width(px(WIDTH))
            .child(body)
            .footer(
                h_flex()
                    .w_full()
                    .gap(px(10.))
                    .justify_end()
                    .items_center()
                    .child(
                        Button::new("pull-filter-cancel")
                            .outline()
                            .small()
                            .label(say!("pulls-cancel"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("pull-filter-save")
                            .primary()
                            .small()
                            .label(say!("pulls-filter-add"))
                            .disabled(problem.is_some())
                            .on_click(move |_, window, cx| {
                                let filter = SavedFilter {
                                    name: name_field.read(cx).value().to_string(),
                                    query: query_field.read(cx).value().to_string(),
                                };
                                let _ = view.update(cx, |this, cx| this.save_filter(filter, cx));
                                window.close_dialog(cx);
                            }),
                    ),
            )
    });
    cx.defer_in(window, move |_, window, cx| {
        first.update(cx, |input, cx| input.focus(window, cx));
    });
    subscriptions
}

#[cfg(test)]
mod tests {
    use super::words_line;

    #[test]
    fn the_words_are_listed() {
        let line = words_line();
        assert!(line.contains("review-requested:"));
        assert!(line.contains("status:"));
    }
}
