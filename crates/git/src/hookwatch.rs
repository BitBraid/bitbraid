//! Telling a hook that failed from git that failed.
//!
//! When a hook ends a command, git says what the hook printed and ends
//! with a status of its own; nothing in that says "this was your
//! pre-commit hook". Some hooks cannot even stop anything, and still turn
//! a command that worked into one that ended badly (`post-checkout`).
//!
//! Git's event trace does say it. A command that may run hooks is started
//! with `GIT_TRACE2_EVENT` naming a file of its own; git and every git it
//! starts append one line of JSON for each event, and a hook is a child
//! that is started (`child_start` with `child_class` `hook` and its
//! `hook_name`) and ends (`child_exit` with its `code`). After a command
//! that failed, the trace is read for a hook that ended badly; after one
//! that went well it is only removed.
//!
//! The file is in `bitbraid/` in the repository's git directory, where
//! the journal is: nobody else writes there, which the temporary
//! directory of the system could not promise.
//!
//! Sources: githooks(5) (which hooks there are, which of them can stop a
//! command, what their exit status does); git's
//! Documentation/technical/api-trace2 (the event format, the events
//! `child_start` and `child_exit` and their fields); git(1)
//! (`GIT_TRACE2_EVENT`: an absolute path is appended to); git-commit(1),
//! git-merge(1), git-rebase(1), git-push(1), git-am(1) (`--no-verify`).

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs,
    io::{Read as _, Seek as _, SeekFrom},
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use crate::{Error, Repository};

/// The variable that names the file git traces its events to.
pub(crate) const TRACE: &str = "GIT_TRACE2_EVENT";

/// How much of a trace is read, from its end: the hook that ended a
/// command is among the last things in it.
const MOST: u64 = 1024 * 1024;

/// Told apart by a number: two commands of one process at once have two
/// files.
static NEXT: AtomicU64 = AtomicU64::new(0);

/// A hook that ended with something other than success.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Failed {
    /// Its name, as githooks(5) has it.
    pub(crate) hook: String,
    /// What it ended with.
    pub(crate) code: i32,
}

/// The file one command's events go to. Removed when this is dropped.
#[derive(Debug)]
pub(crate) struct Watch {
    file: PathBuf,
}

impl Watch {
    /// A file for the events of one command in this repository. None
    /// where it cannot be made; the command then runs unwatched, and a
    /// hook's failure stays git's.
    pub(crate) fn begin(repo: &Repository) -> Option<Self> {
        let dir = std::path::absolute(repo.git_dir()).ok()?.join("bitbraid");
        fs::create_dir_all(&dir).ok()?;
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let file = dir.join(format!("hooks-{}-{n}.trace", std::process::id()));
        // One of an earlier process with this number that was left behind.
        let _ = fs::remove_file(&file);
        Some(Self { file })
    }

    /// What the command is started with in its environment.
    pub(crate) fn env(&self) -> (&'static str, &OsStr) {
        (TRACE, self.file.as_os_str())
    }

    /// The hook that ended badly while the command ran, when one did.
    pub(crate) fn failed(&self) -> Option<Failed> {
        let mut open = fs::File::open(&self.file).ok()?;
        let length = open.metadata().ok()?.len();
        if length > MOST {
            open.seek(SeekFrom::Start(length.saturating_sub(MOST)))
                .ok()?;
        }
        let mut bytes = Vec::new();
        open.take(MOST).read_to_end(&mut bytes).ok()?;
        failed_in(&String::from_utf8_lossy(&bytes))
    }

    /// What became of a command, with a hook's failure told apart from
    /// git's own: an error that git gave because a hook ended badly says
    /// which hook, what it ended with and what it said.
    pub(crate) fn explain<T>(&self, done: crate::Result<T>) -> crate::Result<T> {
        self.explain_with(done, "")
    }

    /// [`Self::explain`] with what the command wrote on its standard
    /// output. Git hands the other hooks' output to its standard error,
    /// but a `pre-push` hook writes to git's own standard output, which a
    /// push otherwise leaves empty: that is where git-lfs lists the locked
    /// files that stopped a push, and who holds each, before one line on
    /// standard error (git 2.56 and git-lfs 3.8.0 do). For that hook what
    /// it wrote there comes before what was written on standard error.
    pub(crate) fn explain_with<T>(&self, done: crate::Result<T>, stdout: &str) -> crate::Result<T> {
        let Err(Error::Command { stderr, .. }) = &done else {
            return done;
        };
        match self.failed() {
            Some(failed) => {
                let said = if failed.hook != "pre-push" || stdout.trim().is_empty() {
                    last_words(stderr)
                } else {
                    last_words(&format!("{stdout}\n{stderr}"))
                };
                Err(Error::Hook {
                    after: runs_after(&failed.hook),
                    said,
                    hook: failed.hook,
                    code: failed.code,
                })
            }
            None => done,
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.file);
    }
}

/// The last hook in a trace that ended with something other than
/// success. A line that is no event, or an event of another kind, is
/// passed over: the trace is git's to write, and other programs that a
/// hook starts write into it too.
fn failed_in(trace: &str) -> Option<Failed> {
    // A hook is known by the process that started it and the number that
    // process gave it.
    let mut running: HashMap<(String, u64), String> = HashMap::new();
    let mut last = None;
    for line in trace.lines().filter(|line| line.contains("\"child_")) {
        let Ok(event) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let text = |key: &str| event.get(key).and_then(|value| value.as_str());
        let (Some(kind), Some(process), Some(child)) = (
            text("event"),
            text("sid"),
            event.get("child_id").and_then(|id| id.as_u64()),
        ) else {
            continue;
        };
        let key = (process.to_owned(), child);
        match kind {
            "child_start" if text("child_class") == Some("hook") => {
                if let Some(name) = text("hook_name").filter(|name| plain(name)) {
                    running.insert(key, name.to_owned());
                }
            }
            "child_exit" => {
                if let Some(hook) = running.remove(&key) {
                    let code = event
                        .get("code")
                        .and_then(|code| code.as_i64())
                        .and_then(|code| i32::try_from(code).ok())
                        .unwrap_or(-1);
                    if code != 0 {
                        last = Some(Failed { hook, code });
                    }
                }
            }
            _ => {}
        }
    }
    last
}

/// A hook's name as git has them: a few lowercase words with dashes.
fn plain(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// Whether a hook runs after what it is about is done, so that its
/// failure takes nothing back (githooks(5): the `post-` hooks).
fn runs_after(hook: &str) -> bool {
    hook.starts_with("post-")
}

/// The end of what was written, for a message: its last lines without
/// git's hints, in one line.
fn last_words(said: &str) -> String {
    let lines: Vec<&str> = said
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .collect();
    let from = lines.len().saturating_sub(6);
    let mut text = lines.get(from..).unwrap_or(&[]).join(" ");
    if text.len() > 600 {
        let mut cut = 600;
        while !text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// What a failed hook is to the user, and what there is to do about it.
/// `after` hooks have stopped nothing.
pub(crate) fn describe(hook: &str, code: i32, said: &str, after: bool) -> String {
    let what_it_said = if said.is_empty() {
        say!("hookwatch-printed-nothing").to_owned()
    } else {
        say!("hookwatch-said", said)
    };
    if after {
        return say!("hookwatch-failed-after", hook, code, what_it_said);
    }
    let way_round = match hook {
        "pre-commit" | "commit-msg" => Some(say!("hookwatch-way-round-commit")),
        "pre-merge-commit" => Some(say!("hookwatch-way-round-merge")),
        "pre-rebase" => Some(say!("hookwatch-way-round-rebase")),
        "pre-push" => Some(say!("hookwatch-way-round-push")),
        "applypatch-msg" | "pre-applypatch" => Some(say!("hookwatch-way-round-am")),
        _ => None,
    };
    // After the sentence before it, with a blank between.
    let way_round = way_round.map(|text| format!(" {text}")).unwrap_or_default();
    say!("hookwatch-stopped", hook, code, what_it_said, way_round)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "20261010T100144.498140Z-Habd66376-P0028de25";

    fn start(sid: &str, child: u64, class: &str, hook: &str) -> String {
        format!(
            r#"{{"event":"child_start","sid":"{sid}","thread":"main","child_id":{child},"child_class":"{class}","hook_name":"{hook}","use_shell":false,"argv":[".git/hooks/{hook}"]}}"#
        )
    }

    fn exit(sid: &str, child: u64, code: i64) -> String {
        format!(
            r#"{{"event":"child_exit","sid":"{sid}","thread":"main","child_id":{child},"pid":1,"code":{code},"t_rel":0.001}}"#
        )
    }

    fn failed(hook: &str, code: i32) -> Option<Failed> {
        Some(Failed {
            hook: hook.to_owned(),
            code,
        })
    }

    #[test]
    fn a_hook_that_ended_badly_is_found_in_the_trace() {
        let trace = [
            r#"{"event":"version","sid":"x","evt":"3","exe":"2.56.0"}"#.to_owned(),
            start(SID, 0, "hook", "pre-commit"),
            exit(SID, 0, 1),
            r#"{"event":"exit","sid":"x","code":1}"#.to_owned(),
        ]
        .join("\n");
        assert_eq!(failed_in(&trace), failed("pre-commit", 1));
    }

    #[test]
    fn only_hooks_count_and_only_the_ones_that_failed() {
        // A child that is no hook fails, a hook succeeds, a hook of
        // another process with the same number fails later: the last.
        let other = "20261010T100144.498140Z-Habd66376-P0028de25/20261010T100145.1Z-H1-P2";
        let trace = [
            start(SID, 0, "?", "pre-commit"),
            exit(SID, 0, 128),
            start(SID, 1, "hook", "pre-commit"),
            exit(SID, 1, 0),
            start(SID, 2, "hook", "prepare-commit-msg"),
            start(other, 2, "hook", "commit-msg"),
            exit(SID, 2, 0),
            exit(other, 2, 3),
            start(SID, 3, "hook", "post-commit"),
        ]
        .join("\n");
        assert_eq!(failed_in(&trace), failed("commit-msg", 3));
        // Nothing failed; nothing in it; a hook that never ended.
        let fine = [start(SID, 0, "hook", "pre-commit"), exit(SID, 0, 0)].join("\n");
        assert_eq!(failed_in(&fine), None);
        assert_eq!(failed_in(""), None);
        assert_eq!(failed_in(&start(SID, 0, "hook", "pre-commit")), None);
        // Two fail one after the other: the one that ended the command.
        let both = [
            start(SID, 0, "hook", "pre-commit"),
            exit(SID, 0, 1),
            start(SID, 1, "hook", "post-checkout"),
            exit(SID, 1, 7),
        ]
        .join("\n");
        assert_eq!(failed_in(&both), failed("post-checkout", 7));
    }

    #[test]
    fn a_trace_that_is_not_one_is_read_without_harm() {
        let long = "x".repeat(100_000);
        for trace in [
            "not json at all \"child_start\"",
            r#"{"event":"child_start"}"#,
            r#"{"event":"child_exit","sid":7,"child_id":"x","code":1}"#,
            r#"["child_start"]"#,
            r#"{"event":"child_exit","sid":"s","child_id":0,"code":1}"#,
            "\"child_\0\u{1}\u{2}",
            long.as_str(),
        ] {
            assert_eq!(failed_in(trace), None, "{:?}", trace.get(..40));
        }
        // A name that is no hook's is not taken for one, and a code that
        // is no number is still a failure.
        let odd = [
            start(SID, 0, "hook", "../../etc/passwd"),
            exit(SID, 0, 1),
            start(SID, 1, "hook", "pre-push"),
            r#"{"event":"child_exit","sid":"20261010T100144.498140Z-Habd66376-P0028de25","child_id":1,"code":"one"}"#.to_owned(),
        ]
        .join("\n");
        assert_eq!(failed_in(&odd), failed("pre-push", -1));
    }

    #[test]
    fn what_is_said_names_the_hook_and_what_to_do() {
        let stopped = describe("pre-commit", 1, "lint failed in a.rs", false);
        assert!(stopped.contains("the pre-commit hook"), "{stopped}");
        assert!(stopped.contains("status 1"), "{stopped}");
        assert!(stopped.contains("lint failed in a.rs"), "{stopped}");
        assert!(stopped.contains("--no-verify"), "{stopped}");
        assert!(stopped.contains("Git itself had nothing against it"));
        let after = describe("post-checkout", 7, "", true);
        assert!(after.contains("post-checkout") && after.contains("status 7"));
        assert!(after.contains("is done"), "{after}");
        assert!(after.contains("printed nothing"), "{after}");
        assert!(describe("pre-push", 1, "x", false).contains("--no-verify"));
        assert!(!describe("reference-transaction", 1, "x", false).contains("--no-verify"));
        assert!(runs_after("post-merge") && !runs_after("pre-rebase"));
        // What a hook said is cut to its end.
        let chatty = (0..50).map(|n| format!("line {n}\n")).collect::<String>();
        let said = last_words(&format!("{chatty}hint: not this\n\nthe last line\n"));
        assert!(said.ends_with("the last line") && !said.contains("line 10 "));
        assert!(!said.contains("hint:"));
        assert!(last_words(&"é".repeat(2000)).chars().count() <= 601);
    }
}
