//! Find a file by its path and open its history: a field to type letters of
//! the path into and the files that hold them in order, the best first, as
//! `search.files` finds them in the tree of HEAD.
//!
//! Every change of the text asks anew and stops the search that was still
//! running for the text before. Up and Down move through the list, Enter
//! or a click opens the file's history. Enter is for what is typed: pressed
//! while that is still being looked for, it opens the first file found for
//! it when the answer comes, not one found for the text before.
//!
//! Sources: `bitbraid_git::find` for what a match is.

use gpui_kit::component::{
    ActiveTheme,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde_json::json;

use crate::session::{Reply, Request, SessionHandle};

/// How many files are listed.
const LISTED: usize = 14;

/// What the finder asks of the workspace.
#[derive(Debug, Clone)]
pub enum FindFileEvent {
    /// Show the history of the file at this path.
    Open(String),
}

impl EventEmitter<FindFileEvent> for FindFile {}

/// What `search.files` answered.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Found {
    paths: Vec<String>,
    matched: usize,
    scanned: usize,
    more: bool,
}

/// Read the answer of `search.files`; what is not one is nothing found.
fn found_from(value: &serde_json::Value) -> Found {
    let count = |key: &str| {
        value
            .get(key)
            .and_then(|count| count.as_u64())
            .and_then(|count| usize::try_from(count).ok())
            .unwrap_or(0)
    };
    Found {
        paths: value
            .get("files")
            .and_then(|files| files.as_array())
            .into_iter()
            .flatten()
            .filter_map(|file| file.get("path")?.as_str().map(str::to_owned))
            .take(LISTED)
            .collect(),
        matched: count("matched"),
        scanned: count("scanned"),
        more: value.get("more").and_then(|more| more.as_bool()) == Some(true),
    }
}

/// A path as the list shows it: the file's name, and the folders it is in.
fn name_and_folder(path: &str) -> (&str, &str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (name, folder),
        None => (path, ""),
    }
}

/// What stands under the list: how many files match, of how many.
fn summary(found: &Found, typed: bool) -> String {
    let scanned = found.scanned;
    let mut text = match (typed, found.matched) {
        (_, 0) if scanned == 0 => say!("find-file-no-files").to_owned(),
        (true, 0) => say!("find-file-none-match", scanned),
        (true, matched) if matched > found.paths.len() => say!(
            "find-file-best-match",
            shown = found.paths.len(),
            matched,
            scanned
        ),
        (true, matched) => say!("find-file-match", matched, scanned),
        (false, _) => say!("find-file-all", scanned),
    };
    if found.more {
        text.push(' ');
        text.push_str(say!("find-file-more"));
    }
    text
}

/// The row the cursor is on after a step of `delta` among `rows` rows.
fn stepped(cursor: usize, delta: isize, rows: usize) -> usize {
    let Some(last) = rows.checked_sub(1) else {
        return 0;
    };
    let at = isize::try_from(cursor)
        .unwrap_or(isize::MAX)
        .saturating_add(delta);
    usize::try_from(at.max(0)).unwrap_or(0).min(last)
}

/// The file finder.
pub struct FindFile {
    session: SessionHandle,
    input: Entity<InputState>,
    found: Found,
    cursor: usize,
    /// How many times the engine was asked (the answer to an older text
    /// is not taken), and whether the answer is still out.
    asked: u64,
    pending: bool,
    /// Enter was pressed while the answer to what is typed was still out:
    /// the first file of that answer is opened when it comes.
    entered: bool,
    error: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl FindFile {
    pub fn new(session: SessionHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("find-file-placeholder")));
        let subscription =
            cx.subscribe_in(
                &input,
                window,
                |this, _, event: &InputEvent, _, cx| match event {
                    InputEvent::Change => this.ask(cx),
                    InputEvent::PressEnter { .. } => this.confirm(cx),
                    _ => {}
                },
            );
        Self {
            session,
            input,
            found: Found::default(),
            cursor: 0,
            asked: 0,
            pending: false,
            entered: false,
            error: None,
            _subscriptions: vec![subscription],
        }
    }

    /// Start afresh: an empty field with the cursor in it.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| {
            state.set_value("", window, cx);
            state.focus(window, cx);
        });
        self.ask(cx);
    }

    /// Put the cursor in the field.
    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |state, cx| state.focus(window, cx));
    }

    /// Ask the engine for the files that match what is typed.
    fn ask(&mut self, cx: &mut Context<Self>) {
        // What is still being looked for was typed before.
        if self.pending {
            drop(
                self.session
                    .send(Request::CancelReads("search.files".to_owned())),
            );
        }
        self.asked = self.asked.wrapping_add(1);
        let asked = self.asked;
        self.pending = true;
        // An Enter that waited was for the text before.
        self.entered = false;
        let query = self.input.read(cx).value().to_string();
        let request = Request::Command {
            id: "search.files".to_owned(),
            params: json!({ "query": query, "limit": LISTED }),
        };
        self.session
            .call_asked(request, cx, move |this, reply, cx| {
                if this.asked != asked {
                    return; // the text has changed since
                }
                this.pending = false;
                // Enter was pressed for this text: its first file, if it found
                // one. What is on show otherwise is still for the text before.
                let entered = std::mem::take(&mut this.entered);
                match reply {
                    Reply::Command(value) => {
                        this.found = found_from(&value);
                        this.cursor = 0;
                        this.error = None;
                        if entered {
                            this.take_cursor(cx);
                        }
                    }
                    Reply::Error(err) => {
                        this.found = Found::default();
                        this.error = Some(crate::shell::sentence(&err));
                    }
                    _ => {}
                }
                cx.notify();
            });
        cx.notify();
    }

    /// Enter: the history of the file the cursor is on, among the files
    /// found for what is typed. Those still being looked for, it waits for
    /// them: the list on show is for the text before, and its first file
    /// would be opened in place of the one asked for.
    fn confirm(&mut self, cx: &mut Context<Self>) {
        if self.pending {
            self.entered = true;
        } else {
            self.take_cursor(cx);
        }
    }

    /// Open the history of the file the cursor is on.
    fn take_cursor(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.found.paths.get(self.cursor) {
            cx.emit(FindFileEvent::Open(path.clone()));
        }
    }
}

impl Render for FindFile {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let typed = !self.input.read(cx).value().trim().is_empty();
        let rows: Vec<AnyElement> = self
            .found
            .paths
            .iter()
            .enumerate()
            .map(|(ix, path)| {
                let (name, folder) = name_and_folder(path);
                gpui_kit::component::h_flex()
                    .id(("found-file", ix))
                    .w_full()
                    .h(px(26.))
                    .px_2()
                    .gap_2()
                    .items_center()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .when(ix == self.cursor, |row| row.bg(theme.list_active))
                    .hover(|style| style.bg(theme.accent))
                    // A row on show is what is clicked, whatever is typed.
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.cursor = ix;
                        this.entered = false;
                        this.take_cursor(cx);
                    }))
                    .child(div().flex_none().text_sm().child(name.to_owned()))
                    .child(
                        div()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(folder.to_owned()),
                    )
                    .into_any_element()
            })
            .collect();
        let note = match (&self.error, self.pending) {
            (Some(err), _) => err.clone(),
            (None, true) if self.found.paths.is_empty() => say!("find-file-looking").to_owned(),
            _ => summary(&self.found, typed),
        };
        v_flex()
            .w_full()
            .gap_2()
            // Up and Down move through the list while the cursor stays in
            // the field: taken on their way down, before the field.
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                let delta = match event.keystroke.key.as_str() {
                    "down" => 1,
                    "up" => -1,
                    "pagedown" => 10,
                    "pageup" => -10,
                    _ => return,
                };
                this.cursor = stepped(this.cursor, delta, this.found.paths.len());
                cx.stop_propagation();
                cx.notify();
            }))
            .child(Input::new(&self.input))
            .child(v_flex().w_full().min_h(px(26.)).children(rows))
            .child(
                div()
                    .text_xs()
                    .text_color(if self.error.is_some() {
                        theme.danger
                    } else {
                        theme.muted_foreground
                    })
                    .child(note),
            )
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use serde_json::json;

    use super::{Found, LISTED, found_from, name_and_folder, stepped, summary};

    #[test]
    fn an_answer_is_read_and_what_is_none_is_nothing() {
        let found = found_from(&json!({
            "files": [
                { "path": "src/graph.rs", "score": 90 },
                { "path": "README.md", "score": 3 },
                { "score": 1 },
                "noise",
                { "path": 7 },
            ],
            "matched": 2, "scanned": 40, "more": false,
        }));
        assert_eq!(found.paths, ["src/graph.rs", "README.md"]);
        assert_eq!((found.matched, found.scanned, found.more), (2, 40, false));
        for odd in [
            json!(null),
            json!([]),
            json!("x"),
            json!({ "files": 3, "matched": -1 }),
        ] {
            assert_eq!(found_from(&odd), Found::default(), "{odd}");
        }
        // However many come, no more are listed than there are rows for.
        let many: Vec<_> = (0..500)
            .map(|n| json!({ "path": format!("f{n}") }))
            .collect();
        let found = found_from(&json!({ "files": many, "matched": 500, "more": true }));
        assert_eq!(found.paths.len(), LISTED);
        assert!(found.more);
    }

    #[test]
    fn a_path_is_shown_as_its_name_and_its_folders() {
        assert_eq!(
            name_and_folder("crates/ui/src/app.rs"),
            ("app.rs", "crates/ui/src")
        );
        assert_eq!(name_and_folder("README.md"), ("README.md", ""));
        assert_eq!(name_and_folder("dir/"), ("", "dir"));
        assert_eq!(name_and_folder(""), ("", ""));
    }

    #[test]
    fn the_note_says_how_many_match_of_how_many() {
        let found = |paths: usize, matched: usize, scanned: usize, more: bool| Found {
            paths: (0..paths).map(|n| format!("f{n}")).collect(),
            matched,
            scanned,
            more,
        };
        assert_eq!(
            summary(&found(0, 0, 0, false), false),
            "There are no files in the last commit."
        );
        assert_eq!(
            summary(&found(14, 900, 900, false), false),
            "900 files in the last commit. Type letters of a path."
        );
        assert_eq!(
            summary(&found(0, 0, 900, false), true),
            "None of 900 files has these letters in order."
        );
        assert_eq!(summary(&found(1, 1, 1, false), true), "1 of 1 file match.");
        assert_eq!(
            summary(&found(14, 60, 900, true), true),
            "The best 14 of 60 that match, among 900 files. The tree has more files than were \
             looked at."
        );
    }

    #[test]
    fn the_cursor_stays_in_the_list() {
        assert_eq!(stepped(0, 1, 5), 1);
        assert_eq!(stepped(4, 1, 5), 4);
        assert_eq!(stepped(0, -1, 5), 0);
        assert_eq!(stepped(2, 10, 5), 4);
        assert_eq!(stepped(2, -10, 5), 0);
        assert_eq!(stepped(usize::MAX, isize::MAX, 5), 4);
        assert_eq!(stepped(3, isize::MIN, 5), 0);
        assert_eq!(stepped(3, 1, 0), 0);
    }
}
