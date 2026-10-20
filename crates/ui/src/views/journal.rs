//! The undo history: everything that can be undone and redone, newest
//! first, with the place the repository is at in between. Any entry can be
//! gone back to, or forward to, in one step.
//!
//! Sources: none (original design).

use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::{
    session::{Reply, Request, SessionHandle},
    views::{now, relative_time},
};

/// Events from the undo history.
#[derive(Debug, Clone)]
pub enum JournalEvent {
    /// Put the view away.
    Close,
    /// Entries were undone or redone; what to say about it, and whether
    /// all of what was asked for was done.
    Walked {
        /// For a notification.
        said: String,
        /// False when it stopped half-way.
        complete: bool,
    },
}

/// One entry of the journal, as `undo.journal` answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Row {
    id: u64,
    label: String,
    time: i64,
}

fn rows(value: &serde_json::Value, key: &str) -> Vec<Row> {
    value
        .get(key)
        .and_then(|list| list.as_array())
        .map(|list| {
            list.iter()
                .filter_map(|row| {
                    Some(Row {
                        id: row.get("id")?.as_u64()?,
                        label: row.get("label")?.as_str()?.to_owned(),
                        time: row.get("time").and_then(|time| time.as_i64()).unwrap_or(0),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// What to say after `undo.to` or `redo.to`: how many steps were taken,
/// and where and why it stopped when it did.
fn walked(value: &serde_json::Value, undo: bool) -> (String, bool) {
    let key = if undo { "undone" } else { "redone" };
    let taken = value
        .get(key)
        .and_then(|taken| taken.as_array())
        .map_or(0, Vec::len);
    let complete = value.get("complete").and_then(|flag| flag.as_bool()) == Some(true);
    let done = if undo {
        say!("journal-undid", count = taken)
    } else {
        say!("journal-redid", count = taken)
    };
    if complete {
        return (done, true);
    }
    let failed = value
        .get("failed")
        .and_then(|failed| failed.get("label"))
        .and_then(|label| label.as_str())
        .unwrap_or(say!("journal-next-one"));
    let why = value
        .get("error")
        .and_then(|error| error.as_str())
        .unwrap_or(say!("journal-could-not"));
    (say!("journal-stopped", done, failed, why), false)
}

/// The undo history view.
pub struct JournalView {
    session: SessionHandle,
    undo: Vec<Row>,
    redo: Vec<Row>,
    error: Option<String>,
    now: i64,
}

impl EventEmitter<JournalEvent> for JournalView {}

impl JournalView {
    /// Create the view.
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            undo: Vec::new(),
            redo: Vec::new(),
            error: None,
            now: now(),
        }
    }

    /// Read the journal again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.now = now();
        let request = Request::Command {
            id: "undo.journal".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    this.undo = rows(&value, "undo");
                    this.redo = rows(&value, "redo");
                    this.error = None;
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
    }

    /// Undo back to an entry, or redo up to one.
    fn walk(&mut self, id: u64, undo: bool, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: if undo { "undo.to" } else { "redo.to" }.to_owned(),
            params: json!({ "id": id }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    let (said, complete) = walked(&value, undo);
                    cx.emit(JournalEvent::Walked { said, complete });
                }
                Reply::Error(err) => cx.emit(JournalEvent::Walked {
                    said: err,
                    complete: false,
                }),
                _ => {}
            }
            this.reload(cx);
        });
    }

    fn render_row(&self, row: &Row, undo: bool, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = row.id;
        h_flex()
            .w_full()
            .h(px(30.))
            .px_3()
            .gap_2()
            .items_center()
            .when(!undo, |d| d.text_color(theme.muted_foreground))
            .child(div().flex_1().min_w_0().truncate().child(row.label.clone()))
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(relative_time(row.time, self.now)),
            )
            .child(
                Button::new((if undo { "undo-to" } else { "redo-to" }, row.id))
                    .ghost()
                    .xsmall()
                    .label(if undo {
                        say!("journal-undo-to-here")
                    } else {
                        say!("journal-redo-to-here")
                    })
                    .on_click(cx.listener(move |this, _, _, cx| this.walk(id, undo, cx))),
            )
            .into_any_element()
    }
}

impl Render for JournalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // Redo above undo reads as one list from newest to oldest.
        let redo: Vec<AnyElement> = self
            .redo
            .iter()
            .map(|row| self.render_row(row, false, cx))
            .collect();
        let undo: Vec<AnyElement> = self
            .undo
            .iter()
            .map(|row| self.render_row(row, true, cx))
            .collect();
        let nothing = redo.is_empty() && undo.is_empty();
        v_flex()
            .size_full()
            .bg(theme.background)
            .child(
                h_flex()
                    .w_full()
                    .h(px(36.))
                    .flex_none()
                    .px_3()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        div()
                            .flex_1()
                            .font_semibold()
                            .child(say!("journal-heading")),
                    )
                    .child(
                        Button::new("journal-close")
                            .accessibility_label(say!("journal-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(JournalEvent::Close))),
                    ),
            )
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .child(
                v_flex()
                    .id("journal-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .text_sm()
                    .children(redo)
                    .when(!nothing, |d| {
                        d.child(
                            h_flex()
                                .w_full()
                                .h(px(24.))
                                .px_3()
                                .gap_2()
                                .items_center()
                                .text_xs()
                                .text_color(theme.primary)
                                .child(say!("journal-here"))
                                .child(div().flex_1().h(px(1.)).bg(theme.primary.opacity(0.5))),
                        )
                    })
                    .children(undo)
                    .when(nothing, |d| {
                        d.child(
                            div()
                                .p_3()
                                .text_color(theme.muted_foreground)
                                .child(say!("journal-nothing")),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{rows, walked};

    #[test]
    fn the_journal_is_read_and_a_walk_is_told() {
        let journal = json!({
            "undo": [
                { "id": 3, "label": "Tag v2", "time": 30, "kind": "tag_create" },
                { "id": 2, "label": "Commit abc: fix", "time": 20, "kind": "commit" },
                { "label": "no id" },
            ],
            "redo": [],
        });
        let undo = rows(&journal, "undo");
        assert_eq!(undo.len(), 2);
        assert_eq!((undo[0].id, undo[0].label.as_str()), (3, "Tag v2"));
        assert!(rows(&journal, "redo").is_empty());
        assert!(rows(&json!("nonsense"), "undo").is_empty());

        let all = json!({ "undone": ["a", "b"], "complete": true, "error": null, "failed": null });
        assert_eq!(walked(&all, true), ("Undid 2 actions".to_owned(), true));
        let one = json!({ "redone": ["a"], "complete": true });
        assert_eq!(walked(&one, false), ("Redid 1 action".to_owned(), true));
        let half = json!({
            "undone": ["a"],
            "complete": false,
            "error": "tag: v1 is not where this action left it",
            "failed": { "id": 4, "label": "Tag v1" },
        });
        assert_eq!(
            walked(&half, true),
            (
                "Undid 1 action, then stopped at \"Tag v1\": tag: v1 is not where this action left it"
                    .to_owned(),
                false
            )
        );
    }
}
