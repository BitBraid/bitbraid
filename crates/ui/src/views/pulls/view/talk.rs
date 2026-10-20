//! Talking on a pull request: the field at the bottom of the
//! conversation, with Comment (`pr.comment`) and a Review button that
//! approves, asks for changes or comments as a review (`pr.review`) with
//! what the field holds. Every comment has a menu that answers it in its
//! conversation, or quotes it in the field. What was sent is read again
//! from the provider, so that the conversation shows it as the provider
//! keeps it.
//!
//! Sources: docs.github.com/en/rest/pulls/reviews ("Create a review for
//! a pull request"), docs.github.com/en/rest/pulls/comments ("Create a
//! reply for a review comment"), docs.github.com/en/rest/issues/comments;
//! docs.gitlab.com/api/merge_request_approvals, docs.gitlab.com/api/discussions,
//! docs.gitlab.com/api/notes.

use bitbraid_integrations::model::{Comment, ProviderKind, Review};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{InputEvent, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::json;

use super::{PullView, PullViewEvent};
use crate::views::pulls::facts::shown_line;
use crate::zoom::px;

/// What a review says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// It may go in.
    Approve,
    /// It needs changes first.
    RequestChanges,
    /// Neither: what is written, as a review.
    Comment,
}

impl Verdict {
    fn event(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
            Self::Comment => "comment",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Approve => say!("pulls-verdict-approve"),
            Self::RequestChanges => say!("pulls-verdict-request-changes"),
            Self::Comment => say!("pulls-verdict-comment"),
        }
    }

    fn done(self) -> &'static str {
        match self {
            Self::Approve => say!("pulls-verdict-approved"),
            Self::RequestChanges => say!("pulls-verdict-changes-requested"),
            Self::Comment => say!("pulls-verdict-reviewed"),
        }
    }
}

/// A text quoted for an answer: each line after `> `, an empty line after.
pub fn quoted(text: &str) -> String {
    let mut out: String = text
        .trim_end()
        .lines()
        .take(200)
        .map(|line| format!("> {line}\n"))
        .collect();
    out.push('\n');
    out
}

impl PullView {
    /// Make the field the first time the view is drawn.
    pub(super) fn ensure_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.composer.is_some() {
            return;
        }
        let field = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("pulls-comment-placeholder"))
                .rows(4)
        });
        let typed = cx.subscribe_in(&field, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.composer_text = state.read(cx).value().to_string();
                cx.notify();
            }
        });
        self.composer_subscription = Some(typed);
        self.composer = Some(field);
    }

    fn set_composer(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.composer_text = text.clone();
        // The field is at the bottom of the conversation: in view.
        self.scroll.scroll_to_bottom();
        if let Some(field) = &self.composer {
            field.update(cx, |state, cx| {
                state.set_value(text, window, cx);
                state.focus(window, cx);
            });
        }
        cx.notify();
    }

    /// Answer a conversation: the comment goes into it.
    pub fn reply_to_thread(&mut self, at: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(thread) = self.details.as_ref().and_then(|d| d.threads.get(at)) else {
            return;
        };
        let who = thread
            .comments
            .first()
            .map(|comment| shown_line(&comment.author.login, 60))
            .unwrap_or_default();
        let said = match (&thread.path, thread.line) {
            (Some(path), Some(line)) => say!(
                "pulls-answering-on",
                who,
                place = format!("{}:{line}", shown_line(path, 120))
            ),
            (Some(path), None) => say!("pulls-answering-on", who, place = shown_line(path, 120)),
            _ => say!("pulls-answering", who),
        };
        self.reply_to = Some((thread.id.clone(), said));
        let text = self.composer_text.clone();
        self.set_composer(text, window, cx);
    }

    /// Quote a text in the field, after what is there; in the conversation
    /// it came from, when it came from one.
    pub fn quote(
        &mut self,
        text: &str,
        thread: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(at) = thread {
            self.reply_to_thread(at, window, cx);
        }
        let mut field = self.composer_text.clone();
        if !field.is_empty() && !field.ends_with("\n\n") {
            field.push_str(if field.ends_with('\n') { "\n" } else { "\n\n" });
        }
        field.push_str(&quoted(text));
        self.set_composer(field, window, cx);
    }

    /// Answer the whole of it again, not a conversation.
    pub fn clear_reply(&mut self, cx: &mut Context<Self>) {
        self.reply_to = None;
        cx.notify();
    }

    /// Send what the field holds as a comment.
    pub fn send_comment(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.composer_text.trim().to_owned();
        if body.is_empty() {
            return;
        }
        let mut extra = json!({ "body": body });
        if let (Some((id, _)), Some(object)) = (&self.reply_to, extra.as_object_mut()) {
            object.insert("reply_to".to_owned(), json!(id));
        }
        self.send::<Comment>(
            "pr.comment",
            extra,
            say!("pulls-commenting"),
            window,
            cx,
            |this, _, window, cx| {
                this.reply_to = None;
                this.set_composer(String::new(), window, cx);
                this.refresh(cx);
                cx.emit(PullViewEvent::Changed);
            },
        );
    }

    /// Send a review with what the field holds.
    pub fn send_review(&mut self, verdict: Verdict, window: &mut Window, cx: &mut Context<Self>) {
        let body = self.composer_text.trim().to_owned();
        if !self.verdict_allowed(verdict) {
            return;
        }
        let mut extra = json!({ "event": verdict.event() });
        if let (false, Some(object)) = (body.is_empty(), extra.as_object_mut()) {
            object.insert("body".to_owned(), json!(body));
        }
        self.send::<Review>(
            "pr.review",
            extra,
            verdict.done(),
            window,
            cx,
            |this, _, window, cx| {
                this.set_composer(String::new(), window, cx);
                this.refresh(cx);
                cx.emit(PullViewEvent::Changed);
            },
        );
    }

    /// Whether a verdict can be sent as things stand: a review that only
    /// comments needs words, and so does asking for changes at GitHub.
    fn verdict_allowed(&self, verdict: Verdict) -> bool {
        let empty = self.composer_text.trim().is_empty();
        match verdict {
            Verdict::Approve => true,
            Verdict::RequestChanges => !(empty && self.link.kind == ProviderKind::Github),
            Verdict::Comment => !empty,
        }
    }

    /// The menu of a comment: answer its conversation, quote it.
    pub(super) fn comment_menu(
        &self,
        id: ElementId,
        thread: Option<usize>,
        text: String,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let view = cx.entity().downgrade();
        Button::new(id)
            .accessibility_label(say!("pulls-comment-menu"))
            .ghost()
            .xsmall()
            .icon(IconName::EllipsisVertical)
            .dropdown_menu(move |menu: PopupMenu, _, _| {
                let mut menu = menu;
                if let Some(at) = thread {
                    let view = view.clone();
                    menu = menu.item(PopupMenuItem::new(say!("pulls-reply")).on_click(
                        move |_, window, cx| {
                            let _ =
                                view.update(cx, |this, cx| this.reply_to_thread(at, window, cx));
                        },
                    ));
                }
                let (view, text) = (view.clone(), text.clone());
                menu.item(
                    PopupMenuItem::new(say!("pulls-quote")).on_click(move |_, window, cx| {
                        let _ = view.update(cx, |this, cx| this.quote(&text, thread, window, cx));
                    }),
                )
            })
            .into_any_element()
    }

    /// The field at the bottom, with what it answers and its buttons.
    pub(super) fn render_composer(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(field) = &self.composer else {
            return div().into_any_element();
        };
        let empty = self.composer_text.trim().is_empty();
        let busy = self.sending.is_some();
        let view = cx.entity().downgrade();
        let allowed: Vec<(Verdict, bool)> =
            [Verdict::Approve, Verdict::RequestChanges, Verdict::Comment]
                .into_iter()
                .map(|verdict| (verdict, self.verdict_allowed(verdict)))
                .collect();
        v_flex()
            .id("pull-composer")
            .w_full()
            .flex_none()
            .gap_2()
            .when_some(self.reply_to.clone(), |composer, (_, said)| {
                composer.child(
                    h_flex()
                        .gap_1p5()
                        .items_center()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::ArrowRight).size_3p5())
                        .child(div().min_w_0().truncate().child(said))
                        .child(
                            Button::new("pull-reply-clear")
                                .accessibility_label(say!("pulls-reply-clear"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Close)
                                .tooltip(say!("pulls-reply-clear"))
                                .on_click(cx.listener(|this, _, _, cx| this.clear_reply(cx))),
                        ),
                )
            })
            .child(
                div()
                    .id("pull-comment-field")
                    .w_full()
                    .h(px(96.))
                    .overflow_hidden()
                    .child(Textarea::new(field).h_full())
                    .test_support(),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        Button::new("pull-review")
                            .small()
                            .label(say!("pulls-review"))
                            .dropdown_caret(true)
                            .disabled(busy)
                            .dropdown_menu(move |menu: PopupMenu, _, _| {
                                let mut menu = menu;
                                for (verdict, allowed) in &allowed {
                                    let (view, verdict) = (view.clone(), *verdict);
                                    menu = menu.item(
                                        PopupMenuItem::new(verdict.label())
                                            .disabled(!allowed)
                                            .on_click(move |_, window, cx| {
                                                let _ = view.update(cx, |this, cx| {
                                                    this.send_review(verdict, window, cx)
                                                });
                                            }),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(say!("pulls-review-hint")),
                    )
                    .child(
                        Button::new("pull-comment")
                            .primary()
                            .small()
                            .label(if self.reply_to.is_some() {
                                say!("pulls-reply")
                            } else {
                                say!("pulls-comment")
                            })
                            .loading(self.sending == Some(say!("pulls-commenting")))
                            .disabled(empty || busy)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.send_comment(window, cx)),
                            ),
                    ),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::quoted;

    #[test]
    fn a_quote_marks_each_line() {
        assert_eq!(quoted("a\nb\n"), "> a\n> b\n\n");
        assert_eq!(quoted(""), "\n");
    }
}
