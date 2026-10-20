//! Suggestions from local edits: with the pull request's branch checked
//! out here, "Suggest my changes" over a file's changes asks the engine
//! what the edits to that file in the working tree would be sent as
//! (`pr.suggestions`, on the repository's queue, since it reads the
//! working tree), and shows that first in the file's place: for each, the
//! lines it is on, the lines as the pull request has them and what is
//! offered instead, and why one cannot be sent where it cannot. Send puts
//! each that can be on its lines with `pr.comment`, one after the other;
//! the conversation is read again then. Nothing is sent before Send.
//!
//! The view is laid out as the file's changes it takes the place of.
//!
//! Sources: `bitbraid_integrations::suggest` (what is sent and when it
//! cannot be); docs.github.com "Incorporating feedback in your pull
//! request"; docs.gitlab.com "Suggest changes".

use bitbraid_integrations::model::{Comment, PullState};
use bitbraid_integrations::suggest::{Suggestion, Suggestions};
use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::super::calls;
use super::{PullView, PullViewEvent};
use crate::session::SessionHandle;
use crate::views::pulls::facts::shown_line;
use crate::zoom::px;

/// The most lines of a suggestion drawn on each side.
const MOST_DRAWN: usize = 40;
/// How high a line of a suggestion is.
const LINE_HEIGHT: f32 = 20.0;

/// Suggestions being made from a file, read, or sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggesting {
    /// The file.
    pub path: String,
    /// What the engine said would be sent, once it said.
    pub answer: Option<Suggestions>,
    /// Why there are none to show.
    pub error: Option<String>,
    /// How many were sent so far, while they are sent.
    pub sent: Option<usize>,
}

impl Suggesting {
    /// The suggestions that can be sent.
    pub fn sendable(&self) -> Vec<&Suggestion> {
        self.answer
            .iter()
            .flat_map(|answer| answer.suggestions.iter())
            .filter(|suggestion| suggestion.why_not.is_none())
            .collect()
    }
}

/// The words of the lines a suggestion is on.
pub fn lines_words(suggestion: &Suggestion) -> String {
    if suggestion.start_line == suggestion.line {
        say!("pulls-suggestion-line", line = suggestion.line)
    } else {
        say!(
            "pulls-suggestion-lines",
            start = suggestion.start_line,
            end = suggestion.line
        )
    }
}

impl PullView {
    /// Where the view is: the repository's queue, and the branch checked
    /// out there (none when HEAD is on no branch).
    pub fn set_here(
        &mut self,
        session: Option<SessionHandle>,
        checked_out: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if self.checked_out != checked_out || session.is_some() != self.session.is_some() {
            self.session = session;
            self.checked_out = checked_out;
            cx.notify();
        }
    }

    /// Why the edits to the open file cannot be offered: none when they
    /// can.
    pub fn suggest_refused(&self) -> Option<&'static str> {
        let details = self.details.as_ref()?;
        if self.file.is_none() {
            return Some(say!("pulls-suggest-refused-no-file"));
        }
        if details.pull.state != PullState::Open {
            return Some(say!("pulls-suggest-refused-not-open"));
        }
        if self.session.is_none() {
            return Some(say!("pulls-suggest-refused-no-repository"));
        }
        if self.checked_out.as_deref() != Some(details.pull.head_ref.as_str()) {
            return Some(say!("pulls-suggest-refused-not-checked-out"));
        }
        None
    }

    /// Ask what the edits to the open file would be sent as, and show it.
    pub fn suggest(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.suggest_refused().is_some() || self.sending.is_some() {
            return;
        }
        let (Some(details), Some((at, _)), Some(session)) = (
            self.details.as_ref(),
            self.file.as_ref(),
            self.session.as_ref(),
        ) else {
            return;
        };
        let Some(file) = details.files.get(*at) else {
            return;
        };
        let path = file.path.clone();
        self.suggesting = Some(Suggesting {
            path: path.clone(),
            answer: None,
            error: None,
            sent: None,
        });
        let asked = calls::ask_queued::<Suggestions>(
            session,
            "pr.suggestions",
            json!({
                "account": self.link.account,
                "owner": self.link.owner,
                "name": self.link.name,
                "number": self.number,
                "path": path,
            }),
        );
        cx.spawn_in(window, async move |this, cx| {
            let answer = asked.await;
            let _ = this.update(cx, |this, cx| {
                if let Some(suggesting) = this.suggesting.as_mut()
                    && suggesting.path == path
                {
                    match answer {
                        Ok(answer) if answer.suggestions.is_empty() => {
                            suggesting.error = Some(say!("pulls-suggest-nothing").to_owned());
                        }
                        Ok(answer) => suggesting.answer = Some(answer),
                        Err(error) => suggesting.error = Some(error),
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Put the suggestions away: the file's changes again.
    pub fn close_suggestions(&mut self, cx: &mut Context<Self>) {
        if self.suggesting.as_ref().is_some_and(|s| s.sent.is_some()) {
            return;
        }
        self.suggesting = None;
        cx.notify();
    }

    /// Send every suggestion that can be, one after the other.
    pub fn send_suggestions(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sending.is_some() {
            return;
        }
        let Some(suggesting) = self.suggesting.as_mut() else {
            return;
        };
        let sendable: Vec<serde_json::Value> = suggesting
            .sendable()
            .into_iter()
            .map(|suggestion| {
                let mut params = json!({
                    "account": self.link.account,
                    "owner": self.link.owner,
                    "name": self.link.name,
                    "number": self.number,
                    "path": suggestion.path,
                    "line": suggestion.line,
                    "body": suggestion.body,
                });
                if suggestion.start_line < suggestion.line
                    && let Some(object) = params.as_object_mut()
                {
                    object.insert("start_line".to_owned(), json!(suggestion.start_line));
                }
                params
            })
            .collect();
        if sendable.is_empty() {
            return;
        }
        suggesting.sent = Some(0);
        self.sending = Some(say!("pulls-suggesting"));
        self.write_error = None;
        cx.spawn_in(window, async move |this, cx| {
            let mut failed = None;
            for (at, params) in sendable.into_iter().enumerate() {
                match calls::ask::<Comment>("pr.comment", params).await {
                    Ok(_) => {
                        let _ = this.update(cx, |this, cx| {
                            if let Some(suggesting) = this.suggesting.as_mut() {
                                suggesting.sent = Some(at.saturating_add(1));
                            }
                            cx.notify();
                        });
                    }
                    Err(error) => {
                        failed = Some(error);
                        break;
                    }
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.sending = None;
                match failed {
                    Some(error) => {
                        // What was sent is in the conversation; what was
                        // not stays on show.
                        this.write_error = Some(say!("pulls-suggesting-failed", error));
                        if let Some(suggesting) = this.suggesting.as_mut() {
                            suggesting.sent = None;
                        }
                    }
                    None => {
                        this.suggesting = None;
                        this.file = None;
                    }
                }
                this.refresh(cx);
                cx.emit(PullViewEvent::Changed);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// The button over a file's changes that offers its edits.
    pub(super) fn render_suggest_button(&self, cx: &mut Context<Self>) -> AnyElement {
        let refused = self.suggest_refused();
        Button::new("pull-file-suggest")
            .outline()
            .small()
            .label(say!("pulls-suggest"))
            .disabled(refused.is_some() || self.sending.is_some())
            .tooltip(refused.unwrap_or(say!("pulls-suggest-tooltip")))
            .on_click(cx.listener(|this, _, window, cx| this.suggest(window, cx)))
            .into_any_element()
    }

    /// What would be sent, in the file's place.
    pub(super) fn render_suggestions(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(suggesting) = self.suggesting.as_ref() else {
            return div().into_any_element();
        };
        let sendable = suggesting.sendable().len();
        let sending = suggesting.sent.is_some();
        let send_label = match (suggesting.sent, sendable) {
            (Some(sent), total) => say!("pulls-suggest-sending", sent, total),
            (None, count) => say!("pulls-suggest-send", count),
        };
        let heading = h_flex()
            .w_full()
            .h(px(32.))
            .flex_none()
            .px_2()
            .gap_2()
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(
                Button::new("pull-suggest-back")
                    .accessibility_label(say!("pulls-suggest-back"))
                    .ghost()
                    .small()
                    .icon(IconName::ArrowLeft)
                    .tooltip(say!("pulls-suggest-back-tooltip"))
                    .disabled(sending)
                    .on_click(cx.listener(|this, _, _, cx| this.close_suggestions(cx))),
            )
            .child(
                div()
                    .flex_none()
                    .text_sm()
                    .child(say!("pulls-suggestions-heading")),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(shown_line(&suggesting.path, 400)),
            );
        let body: AnyElement = match (&suggesting.answer, &suggesting.error) {
            (_, Some(error)) => div()
                .p_3()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(shown_line(error, 600))
                .into_any_element(),
            (None, None) => div()
                .p_3()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child(say!("pulls-suggest-comparing"))
                .into_any_element(),
            (Some(answer), None) => v_flex()
                .id("pull-suggestions")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .p_3()
                .gap_3()
                .children(
                    answer
                        .suggestions
                        .iter()
                        .enumerate()
                        .map(|(at, suggestion)| self.render_suggestion(at, suggestion, cx)),
                )
                .into_any_element(),
        };
        let footer = h_flex()
            .w_full()
            .flex_none()
            .px_3()
            .py_2()
            .gap_2()
            .items_center()
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(say!("pulls-suggestions-hint")),
            )
            .child(
                Button::new("pull-suggest-cancel")
                    .ghost()
                    .small()
                    .label(say!("pulls-cancel"))
                    .disabled(sending)
                    .on_click(cx.listener(|this, _, _, cx| this.close_suggestions(cx))),
            )
            .child(
                Button::new("pull-suggest-send")
                    .primary()
                    .small()
                    .label(send_label)
                    .disabled(sendable == 0 || sending || self.sending.is_some())
                    .on_click(cx.listener(|this, _, window, cx| this.send_suggestions(window, cx))),
            );
        v_flex()
            .id("pull-suggesting")
            .size_full()
            .child(heading)
            .child(body)
            .child(footer)
            .test_support()
            .into_any_element()
    }

    fn render_suggestion(
        &self,
        at: usize,
        suggestion: &Suggestion,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let mono = theme.mono_font_family.clone();
        let muted = theme.muted_foreground;
        let side = |lines: &[String], sign: &'static str, tint: Hsla| -> AnyElement {
            let shown = lines.len().min(MOST_DRAWN);
            let rows: Vec<AnyElement> = lines
                .iter()
                .take(shown)
                .map(|line| {
                    h_flex()
                        .w_full()
                        .h(px(LINE_HEIGHT))
                        .flex_none()
                        .bg(tint.opacity(0.14))
                        .font_family(mono.clone())
                        .text_size(px(12.))
                        .child(
                            div()
                                .w(px(20.))
                                .flex_none()
                                .text_center()
                                .text_color(tint)
                                .child(sign),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .whitespace_nowrap()
                                .child(shown_line(line, 1000)),
                        )
                        .into_any_element()
                })
                .collect();
            v_flex()
                .w_full()
                .children(rows)
                .when(lines.len() > shown, |list| {
                    list.child(
                        div()
                            .px_2()
                            .text_xs()
                            .text_color(muted)
                            .child(say!("pulls-suggestion-more", count = lines.len() - shown)),
                    )
                })
                .into_any_element()
        };
        let replaced = side(&suggestion.replaced, "−", theme.danger);
        let offered = side(&suggestion.suggested, "+", theme.success);
        v_flex()
            .id(("pull-suggestion", at))
            .w_full()
            .flex_none()
            .rounded(theme.radius)
            .border_1()
            .border_color(theme.border)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(px(30.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .bg(theme.muted)
                    .text_sm()
                    .child(div().flex_none().child(lines_words(suggestion)))
                    .children(suggestion.why_not.as_ref().map(|why| {
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.warning)
                            .child(shown_line(why, 300))
                    })),
            )
            .child(replaced)
            .child(offered)
            .when(suggestion.why_not.is_some(), |card| card.opacity(0.6))
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_integrations::suggest::{Suggestion, Suggestions};

    use super::{Suggesting, lines_words};

    fn suggestion(start: u64, end: u64, why_not: Option<&str>) -> Suggestion {
        Suggestion {
            path: "a".to_owned(),
            start_line: start,
            line: end,
            replaced: vec!["x".to_owned()],
            suggested: vec!["y".to_owned()],
            body: "```suggestion\ny\n```".to_owned(),
            why_not: why_not.map(str::to_owned),
        }
    }

    #[test]
    fn only_those_without_a_reason_are_sent() {
        let suggesting = Suggesting {
            path: "a".to_owned(),
            answer: Some(Suggestions {
                head_sha: "h".to_owned(),
                checked_out: None,
                suggestions: vec![suggestion(1, 1, None), suggestion(4, 6, Some("no"))],
            }),
            error: None,
            sent: None,
        };
        assert_eq!(suggesting.sendable().len(), 1);
        assert_eq!(lines_words(&suggestion(3, 3, None)), "Line 3");
        assert_eq!(lines_words(&suggestion(4, 6, None)), "Lines 4 to 6");
    }
}
