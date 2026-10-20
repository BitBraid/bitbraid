//! What rerere has recorded: every conflict it has a record of, the newest
//! first, with the two sides of the conflict, what it was resolved to, and
//! a way to forget it.
//!
//! Git keeps no path with a record, so a record is shown by what it
//! holds. The one git is waiting to write a resolution to, of a file that
//! is being resolved right now, says which file and cannot be forgotten:
//! there is nothing in it yet. A resolution git has just put into a file
//! again is forgotten from that file's row in the changes panel.
//!
//! Sources: git-rerere(1).

use gpui_kit::component::{
    ActiveTheme, IconName, Sizable, StyledExt, WindowExt,
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

/// Events from the view.
#[derive(Debug, Clone)]
pub enum RerereEvent {
    /// Put the view away.
    Close,
    /// A record was forgotten, or could not be; what to say about it.
    Said {
        /// For a notification.
        text: String,
        /// It worked.
        ok: bool,
    },
}

/// One record, as `rerere.records` answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    /// The name git gives the conflict.
    pub id: String,
    /// A resolution is recorded.
    pub resolved: bool,
    /// When, in unix seconds.
    pub time: i64,
    /// How many conflicts the file had.
    pub conflicts: usize,
    /// Our side of the first conflict.
    pub ours: Vec<String>,
    /// Their side of it.
    pub theirs: Vec<String>,
    /// What it was resolved to, where that can be told.
    pub resolution: Option<Vec<String>>,
    /// The file it is in, when that file is being resolved now.
    pub path: Option<String>,
}

fn lines(value: Option<&serde_json::Value>) -> Option<Vec<String>> {
    Some(
        value?
            .as_array()?
            .iter()
            .filter_map(|line| line.as_str().map(str::to_owned))
            .collect(),
    )
}

/// The records of an answer, and whether rerere is on.
pub fn records(value: &serde_json::Value) -> (bool, Vec<Record>) {
    let enabled = value.get("enabled").and_then(|flag| flag.as_bool()) == Some(true);
    let records = value
        .get("records")
        .and_then(|records| records.as_array())
        .map(|records| {
            records
                .iter()
                .filter_map(|record| {
                    Some(Record {
                        id: record.get("id")?.as_str()?.to_owned(),
                        resolved: record.get("resolved").and_then(|flag| flag.as_bool())
                            == Some(true),
                        time: record
                            .get("time")
                            .and_then(|time| time.as_i64())
                            .unwrap_or(0),
                        conflicts: record
                            .get("conflicts")
                            .and_then(|count| count.as_u64())
                            .and_then(|count| usize::try_from(count).ok())
                            .unwrap_or(0),
                        ours: lines(record.get("ours")).unwrap_or_default(),
                        theirs: lines(record.get("theirs")).unwrap_or_default(),
                        resolution: lines(record.get("resolution")),
                        path: record
                            .get("path")
                            .and_then(|path| path.as_str())
                            .map(str::to_owned),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (enabled, records)
}

/// What the records come to, for the line at the top: how many hold a
/// resolution, and how many are of files being resolved now.
pub fn summary(enabled: bool, records: &[Record]) -> String {
    let recorded = records.iter().filter(|record| record.resolved).count();
    let waiting = records
        .iter()
        .filter(|record| record.path.is_some())
        .count();
    let mut parts = Vec::new();
    if !enabled {
        parts.push(say!("rerere-off").to_owned());
    }
    parts.push(if enabled {
        say!("rerere-recorded", count = recorded)
    } else {
        say!("rerere-recorded-before", count = recorded)
    });
    if waiting > 0 {
        parts.push(say!("rerere-being-resolved", count = waiting));
    }
    parts.join(" \u{b7} ")
}

/// What a record says of itself in a line.
pub fn headline(record: &Record) -> String {
    match (&record.path, record.resolved) {
        (Some(path), _) => say!("rerere-being-resolved-in", path),
        (None, false) => say!("rerere-never-resolved").to_owned(),
        (None, true) => say!("rerere-resolution", conflicts = record.conflicts),
    }
}

/// The view.
pub struct RerereView {
    session: SessionHandle,
    enabled: bool,
    records: Vec<Record>,
    loaded: bool,
    error: Option<String>,
    now: i64,
}

impl EventEmitter<RerereEvent> for RerereView {}

impl RerereView {
    /// Create the view.
    pub fn new(session: SessionHandle) -> Self {
        Self {
            session,
            enabled: false,
            records: Vec::new(),
            loaded: false,
            error: None,
            now: now(),
        }
    }

    /// Read the records again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.now = now();
        let request = Request::Command {
            id: "rerere.records".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    (this.enabled, this.records) = records(&value);
                    this.error = None;
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            this.loaded = true;
            cx.notify();
        });
    }

    /// Ask, and then forget one record.
    fn ask_forget(&mut self, id: String, window: &mut Window, cx: &mut Context<Self>) {
        let view = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, _| {
            let (view, id) = (view.clone(), id.clone());
            alert
                .title(say!("rerere-forget-title"))
                .description(say!("rerere-forget-description"))
                .ok_text(say!("rerere-forget"))
                .show_cancel(true)
                .on_ok(move |_, _, cx| {
                    let id = id.clone();
                    let _ = view.update(cx, |this, cx| this.forget(id, cx));
                    true
                })
        });
    }

    fn forget(&mut self, id: String, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "rerere.forget_record".to_owned(),
            params: json!({ "id": id }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    let forgotten =
                        value.get("forgotten").and_then(|flag| flag.as_bool()) == Some(true);
                    cx.emit(RerereEvent::Said {
                        text: if forgotten {
                            say!("rerere-forgot").to_owned()
                        } else {
                            say!("rerere-gone-already").to_owned()
                        },
                        ok: true,
                    });
                }
                Reply::Error(err) => cx.emit(RerereEvent::Said {
                    text: err,
                    ok: false,
                }),
                _ => {}
            }
            this.reload(cx);
        });
    }

    fn render_record(&self, ix: usize, record: &Record, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let id = record.id.clone();
        let side = |label: &'static str, lines: &[String], colour: Hsla| {
            v_flex()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .children(lines.iter().map(|line| {
                    div()
                        .h(px(18.))
                        .px_1()
                        .truncate()
                        .whitespace_nowrap()
                        .bg(colour.opacity(0.12))
                        .font_family(theme.mono_font_family.clone())
                        .text_size(theme.mono_font_size)
                        .child(if line.is_empty() {
                            " ".to_owned()
                        } else {
                            line.clone()
                        })
                }))
                .when(lines.is_empty(), |side| {
                    side.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("rerere-side-empty")),
                    )
                })
        };
        v_flex()
            .w_full()
            .px_3()
            .py_2()
            .gap_1()
            .border_b_1()
            .border_color(theme.border)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_none()
                            .font_family(theme.mono_font_family.clone())
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(record.id.chars().take(10).collect::<String>()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_sm()
                            .child(headline(record)),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(relative_time(record.time, self.now)),
                    )
                    // Not the one git is about to write to.
                    .when(record.path.is_none(), |row| {
                        row.child(
                            Button::new(("rerere-forget", ix))
                                .ghost()
                                .xsmall()
                                .label(say!("rerere-forget"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.ask_forget(id.clone(), window, cx)
                                })),
                        )
                    }),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_start()
                    .child(side(say!("rerere-ours"), &record.ours, theme.blue))
                    .child(side(say!("rerere-theirs"), &record.theirs, theme.yellow))
                    .when_some(record.resolution.as_ref(), |row, resolution| {
                        row.child(side(say!("rerere-resolved-to"), resolution, theme.green))
                    }),
            )
            .into_any_element()
    }
}

impl Render for RerereView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows: Vec<AnyElement> = self
            .records
            .iter()
            .enumerate()
            .map(|(ix, record)| self.render_record(ix, record, cx))
            .collect();
        let nothing = rows.is_empty();
        let summary = if self.loaded {
            summary(self.enabled, &self.records)
        } else {
            say!("rerere-loading").to_owned()
        };
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
                    .child(div().font_semibold().child(say!("rerere-heading")))
                    .child(
                        div()
                            .flex_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    )
                    .child(
                        Button::new("rerere-close")
                            .accessibility_label(say!("rerere-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RerereEvent::Close))),
                    ),
            )
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .child(
                v_flex()
                    .id("rerere-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(nothing && self.loaded, |d| {
                        d.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(say!("rerere-nothing")),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{headline, records, summary};

    #[test]
    fn records_are_read_and_say_what_they_are() {
        let (enabled, read) = records(&json!({
            "enabled": true,
            "records": [
                { "id": "ab".repeat(20), "resolved": true, "time": 100, "conflicts": 1,
                  "ours": ["main's a"], "theirs": ["topic's a"], "resolution": ["both"],
                  "path": null },
                { "id": "cd".repeat(20), "resolved": false, "time": 50, "conflicts": 3,
                  "ours": [], "theirs": [], "resolution": null, "path": "src/a.rs" },
                { "resolved": true },
                7,
            ],
        }));
        assert!(enabled);
        assert_eq!(read.len(), 2);
        assert_eq!(
            read[0].resolution.as_deref(),
            Some(&["both".to_owned()][..])
        );
        assert_eq!(headline(&read[0]), "a resolution");
        assert_eq!(headline(&read[1]), "being resolved now, in src/a.rs");
        let mut left = read[1].clone();
        left.path = None;
        assert_eq!(headline(&left), "a conflict seen and never resolved");
        let mut many = read[0].clone();
        many.conflicts = 4;
        assert_eq!(headline(&many), "a resolution of 4 conflicts in one file");
        assert_eq!(
            summary(true, &read),
            "1 resolution recorded \u{b7} 1 being resolved now"
        );
        assert_eq!(
            summary(false, &read[..1]),
            "rerere is off here \u{b7} 1 recorded from before"
        );
        assert_eq!(summary(true, &[]), "nothing recorded yet");
        assert_eq!(
            summary(false, &[]),
            "rerere is off here \u{b7} nothing recorded"
        );
        // An answer that is none lists nothing.
        for odd in [json!(null), json!({ "records": 3 }), json!("x")] {
            let (enabled, read) = records(&odd);
            assert!(!enabled && read.is_empty(), "{odd}");
        }
    }
}
