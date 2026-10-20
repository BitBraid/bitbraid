//! The activity log: every git process the application started for this
//! repository, newest first, with its command line, how it ended, how long
//! it took and what it wrote. Entries can be filtered and copied; what a
//! hook printed is in the entry of the command that ran the hook.
//!
//! Sources: none (original design).

use std::collections::BTreeSet;

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Selectable, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::session::{Reply, Request, SessionHandle};

/// How many entries are asked for.
const LIMIT: usize = 300;

/// Events from the activity view.
#[derive(Debug, Clone)]
pub enum ActivityEvent {
    /// Put the view away.
    Close,
}

/// One git process, as `activity.list` answers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    id: u64,
    command: String,
    /// Unix seconds.
    started: i64,
    duration_ms: u64,
    /// How it ended: `exited`, `killed`, `cancelled`, `not_started`,
    /// `lost`, `running`.
    end: String,
    code: Option<i64>,
    stdout: String,
    stderr: String,
    cut: bool,
    question: bool,
    /// The engine command it ran under.
    origin: Option<String>,
}

impl Row {
    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .unwrap_or_default()
                .to_owned()
        };
        let flag = |key: &str| value.get(key).and_then(|flag| flag.as_bool()) == Some(true);
        Some(Self {
            id: value.get("id")?.as_u64()?,
            command: value.get("command")?.as_str()?.to_owned(),
            started: value
                .get("started")
                .and_then(|ms| ms.as_u64())
                .and_then(|ms| i64::try_from(ms / 1000).ok())
                .unwrap_or(0),
            duration_ms: value
                .get("duration_ms")
                .and_then(|ms| ms.as_u64())
                .unwrap_or(0),
            end: text("end"),
            code: value.get("code").and_then(|code| code.as_i64()),
            stdout: text("stdout"),
            stderr: text("stderr"),
            cut: flag("stdout_cut") || flag("stderr_cut"),
            question: flag("question"),
            origin: value
                .get("origin")
                .and_then(|origin| origin.as_str())
                .map(str::to_owned),
        })
    }

    /// Whether it ended the way a command should.
    fn went_well(&self) -> bool {
        self.end == "exited" && self.code == Some(0)
    }

    /// How it ended, in a few words.
    fn outcome(&self) -> String {
        match (self.end.as_str(), self.code) {
            ("exited", Some(0)) => say!("activity-ended-ok").to_owned(),
            ("exited", Some(code)) => say!("activity-ended-exit", code),
            ("killed", _) => say!("activity-ended-killed").to_owned(),
            ("cancelled", _) => say!("activity-ended-stopped").to_owned(),
            ("not_started", _) => say!("activity-ended-not-started").to_owned(),
            ("running", _) => say!("activity-ended-running").to_owned(),
            _ => say!("activity-ended-unknown").to_owned(),
        }
    }

    /// What goes to the clipboard: the command line and what it wrote.
    fn copied(&self) -> String {
        let mut text = self.command.clone();
        for output in [&self.stdout, &self.stderr] {
            if !output.is_empty() {
                text.push('\n');
                text.push_str(output);
            }
        }
        text
    }
}

/// How long something took, for a column a few characters wide.
fn duration_text(ms: u64) -> String {
    match ms {
        0..=999 => say!("activity-milliseconds", ms),
        1000..=59_999 => say!(
            "activity-seconds",
            seconds = ms / 1000,
            tenths = (ms % 1000) / 100
        ),
        _ => say!("activity-minutes", minutes = ms / 60_000),
    }
}

pub struct ActivityView {
    session: SessionHandle,
    rows: Vec<Row>,
    /// How many entries matched before the limit.
    matched: usize,
    /// Entries whose output is shown.
    open: BTreeSet<u64>,
    filter: Entity<InputState>,
    /// Show the questions the application asks git all the time as well.
    questions: bool,
    /// The hooks that run in this repository, by name.
    hooks: Vec<String>,
    error: Option<String>,
    /// The clock the times of day are read on: this machine's.
    zone: bitbraid_git::search::Zone,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ActivityEvent> for ActivityView {}

impl ActivityView {
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| InputState::new(window, cx).placeholder(say!("activity-filter")));
        let subscription =
            cx.subscribe_in(&filter, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload(cx);
                }
            });
        Self {
            session,
            rows: Vec::new(),
            matched: 0,
            open: BTreeSet::new(),
            filter,
            questions: false,
            hooks: Vec::new(),
            error: None,
            zone: bitbraid_git::search::Zone::system(),
            _subscriptions: vec![subscription],
        }
    }

    /// Read the log again, and which hooks there are.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let text = self.filter.read(cx).value().trim().to_owned();
        let request = Request::Command {
            id: "activity.list".to_owned(),
            params: json!({ "limit": LIMIT, "questions": self.questions, "text": text }),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    this.rows = value
                        .get("entries")
                        .and_then(|entries| entries.as_array())
                        .map(|entries| entries.iter().filter_map(Row::from_json).collect())
                        .unwrap_or_default();
                    this.matched = value
                        .get("matched")
                        .and_then(|count| count.as_u64())
                        .and_then(|count| usize::try_from(count).ok())
                        .unwrap_or(this.rows.len());
                    this.error = None;
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
        let request = Request::Command {
            id: "hook.list".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            if let Reply::Command(value) = reply {
                this.hooks = value
                    .get("hooks")
                    .and_then(|hooks| hooks.as_array())
                    .map(|hooks| {
                        hooks
                            .iter()
                            .filter_map(|hook| hook.get("name")?.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                cx.notify();
            }
        });
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "activity.clear".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, _, cx| {
            this.open.clear();
            this.reload(cx);
        });
    }

    fn render_row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let open = self.open.contains(&row.id);
        let id = row.id;
        let colour = if row.went_well() {
            theme.muted_foreground
        } else if row.end == "running" || row.end == "cancelled" {
            theme.warning
        } else {
            theme.danger
        };
        let head = h_flex()
            .id(("activity-row", ix))
            .w_full()
            .min_h(px(26.))
            .px_3()
            .gap_2()
            .items_center()
            .cursor_pointer()
            .hover(|style| style.bg(theme.accent))
            .on_click(cx.listener(move |this, _, _, cx| {
                if !this.open.remove(&id) {
                    this.open.insert(id);
                }
                cx.notify();
            }))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size_3()
                .flex_none()
                .text_color(theme.muted_foreground),
            )
            // When, first, by the clock of this machine.
            .child(
                div()
                    .flex_none()
                    .w(px(64.))
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .text_color(theme.muted_foreground)
                    .child(clock_time(row.started, self.zone.offset_at(row.started))),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .when(row.question, |d| d.text_color(theme.muted_foreground))
                    .child(row.command.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(colour)
                    .child(row.outcome()),
            )
            .when_some(row.origin.clone(), |d, origin| {
                d.child(
                    div()
                        .flex_none()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(origin),
                )
            })
            // How long, right-aligned at the end.
            .child(
                div()
                    .flex_none()
                    .w(px(62.))
                    .text_right()
                    .font_family(theme.mono_font_family.clone())
                    .text_size(theme.mono_font_size)
                    .text_color(theme.muted_foreground)
                    .child(duration_text(row.duration_ms)),
            );
        if !open {
            return head.into_any_element();
        }
        let copied = row.copied();
        let output = |text: &str| {
            div()
                .w_full()
                .px_2()
                .py_1()
                .rounded(theme.radius)
                .bg(theme.secondary)
                .font_family(theme.mono_font_family.clone())
                .text_size(theme.mono_font_size)
                .children(text.lines().map(|line| {
                    div()
                        .min_h(px(18.))
                        .whitespace_nowrap()
                        .child(line.to_owned())
                }))
        };
        v_flex()
            .w_full()
            .child(head)
            .child(
                v_flex()
                    .w_full()
                    .pl(px(34.))
                    .pr_3()
                    .pb_2()
                    .gap_1()
                    .overflow_hidden()
                    .when(row.cut, |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(say!("activity-cut")),
                        )
                    })
                    .when(!row.stdout.is_empty(), |d| d.child(output(&row.stdout)))
                    .when(!row.stderr.is_empty(), |d| d.child(output(&row.stderr)))
                    .when(row.stdout.is_empty() && row.stderr.is_empty(), |d| {
                        d.child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(say!("activity-wrote-nothing")),
                        )
                    })
                    .child(
                        h_flex().child(
                            Button::new(("activity-copy", ix))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .label(say!("activity-copy"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copied.clone(),
                                    ));
                                }),
                        ),
                    ),
            )
            .into_any_element()
    }
}

impl Render for ActivityView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let rows: Vec<AnyElement> = self
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| self.render_row(ix, row, cx))
            .collect();
        let shown = self.rows.len();
        let summary = if self.matched > shown {
            say!("activity-summary-last", shown, matched = self.matched)
        } else {
            say!("activity-entries", count = shown)
        };
        let hooks = if self.hooks.is_empty() {
            say!("activity-no-hooks").to_owned()
        } else {
            say!("activity-hooks", hooks = self.hooks.join(", "))
        };
        v_flex()
            .id("activity-dialog")
            .size_full()
            .bg(theme.background)
            .border_1()
            .border_color(theme.border)
            .rounded(theme.radius_lg)
            .overflow_hidden()
            .child(
                h_flex()
                    .w_full()
                    .h(px(TITLE_HEIGHT))
                    .flex_none()
                    .px_5()
                    .gap_2()
                    .items_center()
                    .bg(theme.title_bar)
                    .border_b_1()
                    .border_color(theme.border)
                    .child(div().flex_none().text_lg().child(say!("activity-heading")))
                    .child(div().w(px(280.)).child(Input::new(&self.filter).small()))
                    .child(
                        Button::new("activity-questions")
                            .ghost()
                            .small()
                            .label(say!("activity-questions"))
                            .tooltip(say!("activity-questions-tooltip"))
                            .selected(self.questions)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.questions = !this.questions;
                                this.reload(cx);
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("activity-reload")
                            .accessibility_label(say!("activity-reload"))
                            .ghost()
                            .small()
                            .icon(IconName::RefreshCw)
                            .tooltip(say!("activity-reload"))
                            .on_click(cx.listener(|this, _, _, cx| this.reload(cx))),
                    )
                    .child(
                        Button::new("activity-clear")
                            .ghost()
                            .small()
                            .label(say!("activity-clear"))
                            .tooltip(say!("activity-clear-tooltip"))
                            .on_click(cx.listener(|this, _, _, cx| this.clear(cx))),
                    )
                    .child(
                        Button::new("activity-close")
                            .accessibility_label(say!("activity-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(ActivityEvent::Close))),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .h(px(24.))
                    .flex_none()
                    .px_3()
                    .gap_3()
                    .items_center()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(summary)
                    .child(hooks),
            )
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .child(
                v_flex()
                    .id("activity-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .children(rows)
                    .when(shown == 0 && self.error.is_none(), |d| {
                        d.child(
                            div()
                                .p_3()
                                .text_sm()
                                .text_color(theme.muted_foreground)
                                .child(say!("activity-nothing")),
                        )
                    }),
            )
            // Its place is asked for in the tests.
            .test_support()
    }
}

/// Height of the dialog's title row.
const TITLE_HEIGHT: f32 = 52.0;
/// From the window's left, right and top edge to the dialog.
pub const INSET: f32 = 51.0;
/// From the window's bottom to the dialog.
pub const BELOW: f32 = 70.0;

/// A time of day, `HH:MM:SS`, for a Unix timestamp and an offset east of
/// UTC in seconds.
fn clock_time(seconds: i64, offset: i32) -> String {
    let secs = seconds.saturating_add(i64::from(offset)).rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        secs / 3600,
        (secs % 3600) / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{Row, duration_text};

    #[test]
    fn an_entry_is_read_and_says_how_it_ended() {
        let row = Row::from_json(&json!({
            "id": 7,
            "command": "git commit --file=-",
            "started": 1_700_000_123_456u64,
            "duration_ms": 1540,
            "end": "exited",
            "code": 1,
            "stdout": "",
            "stderr": "hook says no",
            "stdout_cut": false,
            "stderr_cut": true,
            "question": false,
            "origin": "commit.create",
        }))
        .unwrap();
        assert_eq!(row.started, 1_700_000_123);
        assert_eq!(row.outcome(), "exit 1");
        assert!(!row.went_well() && row.cut);
        assert_eq!(row.origin.as_deref(), Some("commit.create"));
        assert_eq!(row.copied(), "git commit --file=-\nhook says no");

        let ended = |end: &str, code: Option<i64>| {
            Row::from_json(&json!({ "id": 1, "command": "git x", "end": end, "code": code }))
                .unwrap()
                .outcome()
        };
        assert_eq!(ended("exited", Some(0)), "ok");
        assert_eq!(ended("killed", None), "killed");
        assert_eq!(ended("cancelled", None), "stopped");
        assert_eq!(ended("not_started", None), "not started");
        assert_eq!(ended("running", None), "running");
        assert_eq!(ended("lost", None), "unknown");
        // Without an id or a command it is no entry.
        assert!(Row::from_json(&json!({ "command": "git x" })).is_none());
        assert!(Row::from_json(&json!({ "id": 1 })).is_none());
        assert!(Row::from_json(&json!("nonsense")).is_none());
    }

    #[test]
    fn a_time_of_day_is_read_on_the_clock_of_the_zone() {
        use super::clock_time;
        assert_eq!(clock_time(0, 0), "00:00:00");
        assert_eq!(clock_time(3_723, 0), "01:02:03");
        assert_eq!(clock_time(3_723, 3_600), "02:02:03");
        // Before the day began in UTC, and at the ends of what fits.
        assert_eq!(clock_time(0, -3_600), "23:00:00");
        assert_eq!(clock_time(i64::MAX, i32::MAX).len(), 8);
        assert_eq!(clock_time(i64::MIN, i32::MIN).len(), 8);
    }

    #[test]
    fn durations_fit_their_column() {
        assert_eq!(duration_text(0), "0 ms");
        assert_eq!(duration_text(999), "999 ms");
        assert_eq!(duration_text(1540), "1.5 s");
        assert_eq!(duration_text(59_999), "59.9 s");
        assert_eq!(duration_text(600_000), "10 min");
    }
}
