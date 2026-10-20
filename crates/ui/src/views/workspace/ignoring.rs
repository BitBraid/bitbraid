//! Ignoring from the workspace: what a file's menu and the palette ask
//! for is carried out here, because a pattern may have to be typed first
//! and what came of it is said in a notice. A pattern does not stop git
//! from tracking a file, so a notice about a pattern that covers tracked
//! files offers to stop tracking them.
//!
//! Sources: `crates/engine/src/ignore_cmds.rs`; gitignore(5).

use gpui_kit::component::WindowExt;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{PromptField, Workspace, sentence};
use crate::{
    notices::Notice,
    session::{Reply, Request},
    views::changes::IgnoreRequest,
};

/// How many paths a sentence names before it only counts the rest.
const NAMED: usize = 3;

/// What an ignore file is called in a sentence: its name where that says
/// which one it is, its place otherwise.
pub fn file_label(file: &str) -> String {
    let path = std::path::Path::new(file);
    if path.file_name().is_some_and(|name| name == ".gitignore") {
        ".gitignore".to_owned()
    } else if path.ends_with("info/exclude") {
        ".git/info/exclude".to_owned()
    } else {
        file.to_owned()
    }
}

/// "a.txt", "a.txt and b.txt", "a.txt, b.txt, c.txt and 4 more".
pub fn named(paths: &[String], count: usize) -> String {
    let shown: Vec<&str> = paths.iter().take(NAMED).map(String::as_str).collect();
    let rest = count.saturating_sub(shown.len());
    match (shown.as_slice(), rest) {
        ([], _) => say!("dialogs-files-count", count),
        ([one], 0) => (*one).to_owned(),
        ([first @ .., last], 0) => say!(
            "dialogs-named-and-last",
            first = first.join(", "),
            last = *last
        ),
        (shown, rest) => say!(
            "workspace-names-and-more",
            names = shown.join(", "),
            more = rest
        ),
    }
}

/// What `ignore.add` answered, as what to say about it: the sentence,
/// whether it is a warning, and the tracked files the pattern covers.
pub fn ignored_summary(answer: &Value) -> (String, bool, Vec<String>) {
    let text = |key: &str| {
        answer
            .get(key)
            .and_then(|value| value.as_str())
            .unwrap_or_default()
    };
    let pattern = text("pattern");
    let file = file_label(text("file"));
    if answer.get("added").and_then(|added| added.as_bool()) == Some(false) {
        return (
            say!("dialogs-ignore-already", pattern, file = file.as_str()),
            false,
            Vec::new(),
        );
    }
    let tracked: Vec<String> = answer
        .get("tracked")
        .and_then(|tracked| tracked.as_array())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| path.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    let count = answer
        .get("tracked_count")
        .and_then(|count| count.as_u64())
        .and_then(|count| usize::try_from(count).ok())
        .unwrap_or(tracked.len())
        .max(tracked.len());
    if count == 0 {
        return (
            say!("dialogs-ignoring", pattern, file = file.as_str()),
            false,
            tracked,
        );
    }
    (
        say!(
            "dialogs-ignoring-tracked",
            pattern,
            file = file.as_str(),
            named = named(&tracked, count),
            count
        ),
        true,
        tracked,
    )
}

/// What is asked before the tracked files that are ignored stop being
/// tracked.
pub fn untrack_question(paths: &[String], count: usize) -> String {
    say!(
        "dialogs-untrack-question",
        named = named(paths, count),
        count
    )
}

impl Workspace {
    /// Carry out what was asked about ignoring.
    pub fn ignore(&mut self, request: IgnoreRequest, window: &mut Window, cx: &mut Context<Self>) {
        match request {
            IgnoreRequest::Path { path, what } => {
                self.ignore_add(json!({ "path": path, "what": what }), window, cx)
            }
            IgnoreRequest::Typed { start, target } => {
                if self.refused_here("ignore.add", window, cx) {
                    return;
                }
                let title = match target.as_str() {
                    "local" => say!("dialogs-ignore-local-title"),
                    "global" => say!("dialogs-ignore-global-title"),
                    _ => say!("dialogs-ignore-title"),
                };
                self.prompt(
                    title.to_owned(),
                    say!("dialogs-ignore"),
                    vec![PromptField::required(
                        say!("dialogs-ignore-placeholder"),
                        &start,
                    )],
                    move |this, texts, window, cx| {
                        let pattern = texts.into_iter().next().unwrap_or_default();
                        this.ignore_add(
                            json!({ "pattern": pattern, "target": target }),
                            window,
                            cx,
                        );
                    },
                    window,
                    cx,
                );
            }
            IgnoreRequest::Untrack { paths } => self.untrack(paths, window, cx),
        }
    }

    /// Add a pattern and say what came of it; where it covers files that
    /// are tracked, offer to stop tracking them.
    fn ignore_add(&mut self, params: Value, window: &mut Window, cx: &mut Context<Self>) {
        if self.refused_here("ignore.add", window, cx) {
            return;
        }
        let reply = self.session.send(Request::Command {
            id: "ignore.add".to_owned(),
            params,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let _ = this.update_in(cx, |this, window, cx| {
                let notice = match reply {
                    Reply::Error(err) => Notice::error(sentence(&err)),
                    Reply::Command(answer) => {
                        let (text, warn, tracked) = ignored_summary(&answer);
                        if warn {
                            let workspace = cx.entity().downgrade();
                            Notice::warning(text).autohide(false).action(
                                say!("dialogs-stop-tracking"),
                                move |window, cx| {
                                    let paths = tracked.clone();
                                    let _ = workspace
                                        .update(cx, |this, cx| this.untrack(paths, window, cx));
                                },
                            )
                        } else {
                            Notice::success(text)
                        }
                    }
                    _ => return,
                };
                crate::notices::push(window, cx, notice);
                this.after_mutation(cx);
                this.reload(cx);
            });
        })
        .detach();
    }

    /// Stop tracking paths; they stay on disk.
    pub fn untrack(&mut self, paths: Vec<String>, window: &mut Window, cx: &mut Context<Self>) {
        if paths.is_empty() {
            return;
        }
        let what = named(&paths, paths.len());
        self.run_mutation(
            "ignore.untrack",
            json!({ "paths": paths }),
            say!("dialogs-untracked", what),
            |_, _, _, _| false,
            window,
            cx,
        );
    }

    /// Ask for a pattern to put into `.gitignore` (from the palette).
    pub fn prompt_ignore(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.ignore(
            IgnoreRequest::Typed {
                start: String::new(),
                target: "repository".to_owned(),
            },
            window,
            cx,
        );
    }

    /// Find the tracked files that ignore rules cover and, once that is
    /// confirmed, stop tracking them (from the palette).
    pub fn untrack_ignored(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.refused_here("ignore.untrack", window, cx) {
            return;
        }
        let reply = self.session.send(Request::Command {
            id: "ignore.tracked".to_owned(),
            params: json!({}),
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(reply) = reply.await else {
                return;
            };
            let _ = this.update_in(cx, |_, window, cx| {
                let answer = match reply {
                    Reply::Error(err) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)));
                        return;
                    }
                    Reply::Command(answer) => answer,
                    _ => return,
                };
                let paths: Vec<String> = answer
                    .get("paths")
                    .and_then(|paths| paths.as_array())
                    .map(|paths| {
                        paths
                            .iter()
                            .filter_map(|path| path.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                if paths.is_empty() {
                    crate::notices::push(window, cx, Notice::info(say!("dialogs-untrack-none")));
                    return;
                }
                let count = answer
                    .get("count")
                    .and_then(|count| count.as_u64())
                    .and_then(|count| usize::try_from(count).ok())
                    .unwrap_or(paths.len());
                let workspace = cx.entity().downgrade();
                let title = say!("dialogs-untrack-ignored-title", count);
                let description = untrack_question(&paths, count);
                window.open_alert_dialog(cx, move |alert, _, _| {
                    let (workspace, paths) = (workspace.clone(), paths.clone());
                    alert
                        .title(title.clone())
                        .description(description.clone())
                        .show_cancel(true)
                        .on_ok(move |_, window, cx| {
                            let paths = paths.clone();
                            let _ =
                                workspace.update(cx, |this, cx| this.untrack(paths, window, cx));
                            true
                        })
                });
            });
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::{file_label, ignored_summary, named, untrack_question};
    use serde_json::json;

    fn paths(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn an_ignore_file_is_called_by_what_tells_it_apart() {
        assert_eq!(file_label("/work/repo/.gitignore"), ".gitignore");
        assert_eq!(
            file_label("/work/repo/.git/info/exclude"),
            ".git/info/exclude"
        );
        assert_eq!(
            file_label("/home/u/.config/git/ignore"),
            "/home/u/.config/git/ignore"
        );
        assert_eq!(file_label(""), "");
    }

    #[test]
    fn a_few_paths_are_named_and_the_rest_counted() {
        assert_eq!(named(&paths(&["a"]), 1), "a");
        assert_eq!(named(&paths(&["a", "b"]), 2), "a and b");
        assert_eq!(named(&paths(&["a", "b", "c"]), 3), "a, b and c");
        assert_eq!(
            named(&paths(&["a", "b", "c", "d"]), 4),
            "a, b, c and 1 more"
        );
        // The engine lists so many and counts them all.
        assert_eq!(
            named(&paths(&["a", "b", "c"]), 9000),
            "a, b, c and 8997 more"
        );
        assert_eq!(named(&[], 7), "7 files");
    }

    #[test]
    fn the_question_before_tracking_stops_counts_its_files() {
        assert_eq!(
            untrack_question(&paths(&["gen/a.txt"]), 1),
            "gen/a.txt. Git tracks it although an ignore rule covers it. It stays on disk; \
             leaving the index is staged, and unstaging takes it back."
        );
        assert!(
            untrack_question(&paths(&["a", "b", "c", "d"]), 40)
                .starts_with("a, b, c and 37 more. Git tracks them although ignore rules cover them. They stay on disk;")
        );
    }

    #[test]
    fn what_came_of_a_pattern_is_said_and_tracked_files_are_a_warning() {
        let fresh = json!({
            "pattern": "*.log", "file": "/r/.gitignore", "added": true,
            "tracked": [], "tracked_count": 0,
        });
        assert_eq!(
            ignored_summary(&fresh),
            ("Ignoring *.log (.gitignore)".to_owned(), false, vec![])
        );
        let there = json!({ "pattern": "*.log", "file": "/r/.git/info/exclude", "added": false });
        assert_eq!(
            ignored_summary(&there).0,
            "*.log is in .git/info/exclude already"
        );
        let one = json!({
            "pattern": "/gen/", "file": "/r/.gitignore", "added": true,
            "tracked": ["gen/a.txt"], "tracked_count": 1,
        });
        let (text, warn, tracked) = ignored_summary(&one);
        assert_eq!(
            text,
            "Ignoring /gen/ (.gitignore). gen/a.txt is tracked and stays so until tracking stops"
        );
        assert!(warn);
        assert_eq!(tracked, ["gen/a.txt"]);
        let many = json!({
            "pattern": "/gen/", "file": "/r/.gitignore", "added": true,
            "tracked": ["gen/a", "gen/b", "gen/c", "gen/d"], "tracked_count": 12,
        });
        let (text, warn, tracked) = ignored_summary(&many);
        assert!(
            text.ends_with(
                "gen/a, gen/b, gen/c and 9 more are tracked and stay so until tracking stops"
            ),
            "{text}"
        );
        assert!(warn && tracked.len() == 4);
        // An answer that is none says little and breaks nothing.
        for odd in [
            json!(null),
            json!([]),
            json!({ "tracked": 3, "tracked_count": "x" }),
        ] {
            let (_, warn, tracked) = ignored_summary(&odd);
            assert!(!warn && tracked.is_empty(), "{odd}");
        }
    }
}
