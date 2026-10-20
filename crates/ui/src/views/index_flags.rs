//! Index flags: the files git has been told not to look at (assume
//! unchanged, skip worktree) or to expect (intent to add). They do not show
//! in the status, so this is where they are seen and brought back: one at
//! a time, by its button or with the arrows and Enter, or all at once.
//!
//! Sources: git-update-index(1) (`--assume-unchanged`, `--skip-worktree`),
//! git-add(1) (`--intent-to-add`).

use gpui_kit::component::{
    ActiveTheme, Disableable, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::session::{Reply, Request, SessionHandle};

/// Events from the index flags view.
#[derive(Debug, Clone)]
pub enum FlagsEvent {
    /// Put the view away.
    Close,
    /// A flag was taken off; the status may show the file again.
    Changed(String),
    /// It was refused; why.
    Refused(String),
}

/// One flagged entry: its path and which flags it carries.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Flagged {
    path: String,
    intent_to_add: bool,
    skip_worktree: bool,
    assume_unchanged: bool,
}

impl Flagged {
    /// The marks that hide the file and can be taken off here, as the
    /// command names them.
    fn hiding(&self) -> Vec<&'static str> {
        let mut flags = Vec::new();
        if self.assume_unchanged {
            flags.push("assume_unchanged");
        }
        if self.skip_worktree {
            flags.push("skip_worktree");
        }
        flags
    }

    /// What the row says the file is marked as.
    fn said(&self) -> &'static str {
        match (
            self.assume_unchanged,
            self.skip_worktree,
            self.intent_to_add,
        ) {
            (true, true, _) => say!("index-flags-assumed-and-skipped"),
            (true, false, _) => say!("index-flags-assumed"),
            (false, true, _) => say!("index-flags-skipped"),
            (false, false, true) => say!("index-flags-intent-to-add"),
            (false, false, false) => "",
        }
    }
}

fn flagged_from(value: &serde_json::Value) -> (Vec<Flagged>, bool) {
    let entries = value
        .get("entries")
        .and_then(|list| list.as_array())
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let flag = |key: &str| entry.get(key).and_then(|flag| flag.as_bool()) == Some(true);
            Some(Flagged {
                path: entry.get("path")?.as_str()?.to_owned(),
                intent_to_add: flag("intent_to_add"),
                skip_worktree: flag("skip_worktree"),
                assume_unchanged: flag("assume_unchanged"),
            })
        })
        .collect();
    let truncated = value.get("truncated").and_then(|flag| flag.as_bool()) == Some(true);
    (entries, truncated)
}

/// A path as the folders it is in, with their last slash, and its name:
/// the name is what is looked for, and stays whole when the row is narrow.
fn parted(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(at) => path.split_at(at.saturating_add(1)),
        None => ("", path),
    }
}

/// The row the cursor goes to from `from`, `by` rows on, among `count`
/// rows: the first or the last from nowhere, and no further than the ends.
fn stepped(count: usize, from: Option<usize>, by: isize) -> Option<usize> {
    let last = count.checked_sub(1)?;
    Some(match from {
        Some(at) => at.min(last).saturating_add_signed(by).min(last),
        None if by < 0 => last,
        None => 0,
    })
}

/// What has to be asked so that git looks at every hidden file again: for
/// each mark, the paths that carry it.
fn all_hidden(entries: &[Flagged]) -> Vec<(&'static str, Vec<String>)> {
    ["assume_unchanged", "skip_worktree"]
        .into_iter()
        .filter_map(|flag| {
            let paths: Vec<String> = entries
                .iter()
                .filter(|entry| entry.hiding().contains(&flag))
                .map(|entry| entry.path.clone())
                .collect();
            (!paths.is_empty()).then_some((flag, paths))
        })
        .collect()
}

/// The index flags view.
pub struct FlagsView {
    session: SessionHandle,
    focus_handle: FocusHandle,
    entries: Vec<Flagged>,
    truncated: bool,
    error: Option<String>,
    /// The row the arrows are on.
    cursor: Option<usize>,
    scroll: ScrollHandle,
}

impl EventEmitter<FlagsEvent> for FlagsView {}

impl Focusable for FlagsView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FlagsView {
    /// Create the view.
    pub fn new(session: SessionHandle, cx: &mut Context<Self>) -> Self {
        Self {
            session,
            focus_handle: cx.focus_handle(),
            entries: Vec::new(),
            truncated: false,
            error: None,
            cursor: None,
            scroll: ScrollHandle::new(),
        }
    }

    /// Read the flags again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let request = Request::Command {
            id: "index.flags".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            match reply {
                Reply::Command(value) => {
                    (this.entries, this.truncated) = flagged_from(&value);
                    this.error = None;
                    // The cursor stays on a row that is there.
                    this.cursor = this
                        .cursor
                        .and_then(|at| stepped(this.entries.len(), Some(at), 0));
                }
                Reply::Error(err) => this.error = Some(err),
                _ => {}
            }
            cx.notify();
        });
    }

    /// Take marks off paths, one request for each mark, and say what was
    /// done when the last has answered.
    fn clear(
        &mut self,
        asks: Vec<(&'static str, Vec<String>)>,
        done: String,
        cx: &mut Context<Self>,
    ) {
        let last = asks.len().saturating_sub(1);
        for (at, (flag, paths)) in asks.into_iter().enumerate() {
            let request = Request::Command {
                id: "index.set_flag".to_owned(),
                params: json!({ "paths": paths, "flag": flag, "on": false }),
            };
            let done = done.clone();
            self.session.call(request, cx, move |this, reply, cx| {
                match reply {
                    Reply::Command(_) if at == last => cx.emit(FlagsEvent::Changed(done)),
                    Reply::Error(err) => cx.emit(FlagsEvent::Refused(err)),
                    _ => {}
                }
                if at == last {
                    this.reload(cx);
                }
            });
        }
    }

    /// Have git look at the file of a row again.
    fn clear_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.get(row) else {
            return;
        };
        let asks: Vec<(&'static str, Vec<String>)> = entry
            .hiding()
            .into_iter()
            .map(|flag| (flag, vec![entry.path.clone()]))
            .collect();
        if asks.is_empty() {
            return;
        }
        let done = say!("index-flags-cleared", path = entry.path.as_str());
        self.clear(asks, done, cx);
    }

    /// Have git look at every hidden file again.
    fn clear_all(&mut self, cx: &mut Context<Self>) {
        let asks = all_hidden(&self.entries);
        let files = self
            .entries
            .iter()
            .filter(|entry| !entry.hiding().is_empty())
            .count();
        if asks.is_empty() {
            return;
        }
        let done = say!("index-flags-cleared-all", files);
        self.clear(asks, done, cx);
    }

    /// A key while the view has the focus: the arrows move over the rows,
    /// Enter takes the marks off the row they are on.
    fn on_key(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let by = match event.keystroke.unparse().as_str() {
            "down" | "j" => 1,
            "up" | "k" => -1,
            "home" => isize::MIN,
            "end" => isize::MAX,
            "enter" | "space" => {
                if let Some(row) = self.cursor {
                    self.clear_row(row, cx);
                }
                cx.stop_propagation();
                return;
            }
            _ => return,
        };
        self.cursor = stepped(self.entries.len(), self.cursor, by);
        if let Some(row) = self.cursor {
            // The explanation stands above the rows.
            self.scroll.scroll_to_item(row.saturating_add(1));
        }
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for FlagsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let hidden = self
            .entries
            .iter()
            .filter(|entry| !entry.hiding().is_empty())
            .count();
        let rows: Vec<AnyElement> = self
            .entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| {
                let (folders, name) = parted(&entry.path);
                let can_clear = !entry.hiding().is_empty();
                h_flex()
                    .id(("flag-row", ix))
                    .w_full()
                    .h(px(30.))
                    .px_3()
                    .gap_2()
                    .items_center()
                    .when(self.cursor == Some(ix), |row| row.bg(theme.list_active))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.cursor = Some(ix);
                        this.focus_handle.focus(window, cx);
                        cx.notify();
                    }))
                    // The folders give way before the name does.
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .flex_shrink(1.)
                                    .min_w_0()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(folders.to_owned()),
                            )
                            .child(
                                div()
                                    .flex_none()
                                    .max_w_full()
                                    .truncate()
                                    .child(name.to_owned()),
                            ),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(entry.said()),
                    )
                    .when(can_clear, |row| {
                        row.child(
                            Button::new(("flag-clear", ix))
                                .outline()
                                .xsmall()
                                .label(say!("index-flags-look-again"))
                                .tooltip(say!("index-flags-look-again-tooltip"))
                                .disabled(self.session.summary.bare)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    cx.stop_propagation();
                                    this.clear_row(ix, cx)
                                })),
                        )
                    })
                    .into_any_element()
            })
            .collect();
        let none = rows.is_empty();
        v_flex()
            .id("index-flags")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key))
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
                            .child(say!("index-flags-heading")),
                    )
                    .when(hidden > 1, |bar| {
                        bar.child(
                            Button::new("flags-clear-all")
                                .outline()
                                .small()
                                .label(say!("index-flags-look-again-all", hidden))
                                .disabled(self.session.summary.bare)
                                .on_click(cx.listener(|this, _, _, cx| this.clear_all(cx))),
                        )
                    })
                    .child(
                        Button::new("flags-close")
                            .accessibility_label(say!("index-flags-close"))
                            .ghost()
                            .small()
                            .icon(IconName::Close)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(FlagsEvent::Close))),
                    ),
            )
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_sm().text_color(theme.danger).child(err))
            })
            .child(
                v_flex()
                    .id("flags-rows")
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .text_sm()
                    .child(
                        div()
                            .p_3()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(say!("index-flags-explained")),
                    )
                    .children(rows)
                    .when(none, |d| {
                        d.child(div().px_3().child(say!("index-flags-none")))
                    })
                    .when(self.truncated, |d| {
                        d.child(
                            div()
                                .px_3()
                                .py_1()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(say!("index-flags-truncated")),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{all_hidden, flagged_from, parted, stepped};

    #[test]
    fn flagged_entries_are_read() {
        let (entries, truncated) = flagged_from(&json!({
            "entries": [
                { "path": "config.local", "intent_to_add": false, "skip_worktree": true, "assume_unchanged": false },
                { "path": "new.txt", "intent_to_add": true, "skip_worktree": false, "assume_unchanged": false },
                { "skip_worktree": true },
            ],
            "counts": { "intent_to_add": 1, "skip_worktree": 1, "assume_unchanged": 0 },
            "truncated": true,
        }));
        assert_eq!(entries.len(), 2);
        assert!(entries[0].skip_worktree && !entries[0].assume_unchanged);
        assert!(entries[1].intent_to_add && truncated);
        assert_eq!(flagged_from(&json!("nonsense")), (Vec::new(), false));
    }

    #[test]
    fn a_row_says_what_the_file_is_marked_as_and_what_can_be_taken_off() {
        let (entries, _) = flagged_from(&json!({ "entries": [
            { "path": "a", "assume_unchanged": true },
            { "path": "b", "skip_worktree": true },
            { "path": "c", "assume_unchanged": true, "skip_worktree": true },
            { "path": "d", "intent_to_add": true },
            { "path": "e" },
        ]}));
        let said: Vec<&str> = entries.iter().map(|entry| entry.said()).collect();
        assert_eq!(
            said,
            [
                "assumed unchanged",
                "skipped",
                "assumed unchanged, and skipped",
                "to be added; unstage it to take that back",
                "",
            ]
        );
        assert_eq!(entries[0].hiding(), ["assume_unchanged"]);
        assert_eq!(entries[2].hiding(), ["assume_unchanged", "skip_worktree"]);
        // A file that is only to be added hides nothing: there is nothing
        // to take off here.
        assert!(entries[3].hiding().is_empty() && entries[4].hiding().is_empty());
        // All at once: one request for each mark, with every path that
        // carries it.
        assert_eq!(
            all_hidden(&entries),
            [
                ("assume_unchanged", vec!["a".to_owned(), "c".to_owned()]),
                ("skip_worktree", vec!["b".to_owned(), "c".to_owned()]),
            ]
        );
        assert!(all_hidden(&entries[3..]).is_empty());
        assert!(all_hidden(&[]).is_empty());
    }

    #[test]
    fn a_path_is_parted_into_its_folders_and_its_name() {
        assert_eq!(parted("a/b/c.txt"), ("a/b/", "c.txt"));
        assert_eq!(parted("c.txt"), ("", "c.txt"));
        assert_eq!(parted(""), ("", ""));
        assert_eq!(parted("a/"), ("a/", ""));
        assert_eq!(parted("/"), ("/", ""));
        assert_eq!(parted("größe/ß.txt"), ("größe/", "ß.txt"));
    }

    #[test]
    fn the_cursor_walks_the_rows_and_stops_at_the_ends() {
        assert_eq!(stepped(0, None, 1), None);
        assert_eq!(stepped(0, Some(3), -1), None);
        // From nowhere: the first going down, the last going up.
        assert_eq!(stepped(3, None, 1), Some(0));
        assert_eq!(stepped(3, None, -1), Some(2));
        assert_eq!(stepped(3, Some(0), 1), Some(1));
        assert_eq!(stepped(3, Some(2), 1), Some(2));
        assert_eq!(stepped(3, Some(0), -1), Some(0));
        assert_eq!(stepped(3, Some(1), isize::MAX), Some(2));
        assert_eq!(stepped(3, Some(1), isize::MIN), Some(0));
        // A row that is gone (the list got shorter) is the last one.
        assert_eq!(stepped(3, Some(9), 0), Some(2));
        assert_eq!(stepped(1, Some(usize::MAX), -1), Some(0));
    }
}
