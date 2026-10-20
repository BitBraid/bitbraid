//! External diff and merge tools. No real tool is started: a command that
//! writes down what it was given stands in for one, and what it was given
//! is compared with what git has for each side.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Tools configured for the user running the tests must not be started,
/// and git skips a tool whose name is empty: so every name is emptied for
/// this repository.
fn no_tools(git: &Git) {
    for key in ["diff.tool", "diff.guitool", "merge.tool", "merge.guitool"] {
        git.run(["config", key, ""]).unwrap();
    }
    // No backup files next to a resolved one.
    git.run(["config", "mergetool.keepBackup", "false"])
        .unwrap();
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let git = scratch.git();
        git.run(["config", "user.name", "Ada Fixture"]).unwrap();
        git.run(["config", "user.email", "ada@fixture.invalid"])
            .unwrap();
        no_tools(&git);
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.scratch.git()
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    /// Somewhere the stand-in tool can write that is not part of the
    /// worktree.
    fn out(&self, name: &str) -> PathBuf {
        let dir = self.scratch.path().join(".git").join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}

/// A diff tool that copies what it is shown into `out` and says when it
/// is done.
fn recorder(out: &Path) -> String {
    let out = out.display();
    format!(
        "cat \"$LOCAL\" > '{out}/local' && cat \"$REMOTE\" > '{out}/remote' && touch '{out}/done'"
    )
}

/// What the tool wrote, once it has: the two versions it was shown.
fn recorded(out: &Path) -> (String, String) {
    let started = std::time::Instant::now();
    while !out.join("done").exists() {
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "the tool never ran"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let read = |name: &str| std::fs::read_to_string(out.join(name)).unwrap();
    (read("local"), read("remote"))
}

fn status(git: &Git) -> String {
    git.run(["status", "--porcelain"]).unwrap()
}

/// `a.txt` changed on both sides of a merge that is then begun.
fn conflict(h: &mut Harness) {
    let git = h.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "theirs\n").unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "ours\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
    let out = h.run("branch.merge", json!({ "target": "topic" }));
    assert_eq!(out["conflicts"], json!(["a.txt"]));
}

#[test]
fn a_diff_tool_is_shown_the_two_versions() {
    let mut h = Harness::new();
    let git = h.git();
    let show = |rev: &str| git.run(["show", rev]).unwrap();

    // Unstaged: the index and the file.
    h.scratch.write("a.txt", "in the worktree\n").unwrap();
    let before = status(&git);
    let out = h.out("tool-unstaged");
    let started = h.run(
        "tool.diff",
        json!({ "path": "a.txt", "command": recorder(&out) }),
    );
    assert!(started["started"].is_string());
    assert_eq!(
        recorded(&out),
        (show(":a.txt"), "in the worktree\n".to_owned())
    );

    // Staged: the last commit and the index.
    git.run(["add", "a.txt"]).unwrap();
    h.scratch.write("a.txt", "changed again\n").unwrap();
    let out = h.out("tool-staged");
    h.run(
        "tool.diff",
        json!({ "path": "a.txt", "staged": true, "command": recorder(&out) }),
    );
    assert_eq!(
        recorded(&out),
        (show("HEAD:a.txt"), "in the worktree\n".to_owned())
    );

    // A commit: its parent and itself. "update a" is HEAD~2.
    let commit = git.rev_parse("HEAD~2").unwrap();
    let out = h.out("tool-commit");
    h.run(
        "tool.diff",
        json!({ "path": "a.txt", "commit": commit, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (show("HEAD~3:a.txt"), show("HEAD~2:a.txt")));

    // A file the commit added, and the first commit, which has no parent:
    // there is nothing on the old side.
    let added = git.rev_parse("HEAD~1").unwrap();
    let out = h.out("tool-added");
    h.run(
        "tool.diff",
        json!({ "path": "b.txt", "commit": added, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (String::new(), show("HEAD~1:b.txt")));
    let root = git.rev_parse("HEAD~4").unwrap();
    let out = h.out("tool-root");
    h.run(
        "tool.diff",
        json!({ "path": "README.md", "commit": root, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (String::new(), show("HEAD~4:README.md")));

    // Two commits that were picked: what differs between them,
    // the older first. "update a" is HEAD~2 and the newest touches b.txt.
    let (old, new) = (
        git.rev_parse("HEAD~3").unwrap(),
        git.rev_parse("HEAD").unwrap(),
    );
    let out = h.out("tool-range");
    h.run(
        "tool.diff",
        json!({ "path": "a.txt", "from": old, "to": new, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (show("HEAD~3:a.txt"), show("HEAD:a.txt")));
    // What git itself calls different between the two is what is shown.
    let by_git = git.lines(["diff", "--name-only", &old, &new]).unwrap();
    assert!(by_git.contains(&"a.txt".to_owned()) && by_git.contains(&"b.txt".to_owned()));
    // A file that is in the newer one only, and no older commit at all.
    let out = h.out("tool-range-added");
    h.run(
        "tool.diff",
        json!({ "path": "b.txt", "from": old, "to": new, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (String::new(), show("HEAD:b.txt")));
    let out = h.out("tool-range-all");
    h.run(
        "tool.diff",
        json!({ "path": "README.md", "to": new, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (String::new(), show("HEAD:README.md")));
    // The other way round is the other way round.
    let out = h.out("tool-range-back");
    h.run(
        "tool.diff",
        json!({ "path": "a.txt", "from": new, "to": old, "command": recorder(&out) }),
    );
    assert_eq!(recorded(&out), (show("HEAD:a.txt"), show("HEAD~3:a.txt")));

    // Looking changes nothing.
    assert_eq!(
        status(&git).replace("MM", " M"),
        before,
        "apart from what the test staged"
    );
    // Two commits are compared by themselves, and both have to be commits.
    let pwned = h.scratch.path().join("pwned-range");
    let touch = format!("touch '{}'", pwned.display());
    for params in [
        json!({ "path": "a.txt", "from": old }),
        json!({ "path": "a.txt", "from": old, "to": new, "commit": new }),
        json!({ "path": "a.txt", "from": old, "to": new, "staged": true }),
        json!({ "path": "a.txt", "from": "--output=pwned", "to": new }),
        json!({ "path": "a.txt", "from": old, "to": "HEAD" }),
        json!({ "path": "a.txt", "from": old, "to": "" }),
        json!({ "path": "a.txt", "from": 7, "to": new }),
        json!({ "path": "../outside", "from": old, "to": new }),
        json!({ "from": old, "to": new }),
    ] {
        let mut params = params;
        params["command"] = json!(touch);
        assert!(h.try_run("tool.diff", params.clone()).is_err(), "{params}");
    }
    assert!(!pwned.exists(), "nothing was started for what was refused");
}

#[test]
fn without_a_tool_nothing_is_started() {
    let mut h = Harness::new();
    assert_eq!(
        h.run("tool.list", json!({})),
        json!({ "diff": null, "merge": null })
    );
    h.scratch.write("a.txt", "changed\n").unwrap();
    let err = h
        .try_run("tool.diff", json!({ "path": "a.txt" }))
        .unwrap_err();
    assert!(err.contains("no diff tool is configured"), "{err}");
    // A command that is only blanks is no command.
    let err = h
        .try_run("tool.diff", json!({ "path": "a.txt", "command": "  " }))
        .unwrap_err();
    assert!(err.contains("no diff tool is configured"), "{err}");
    h.git().run(["checkout", "-q", "--", "a.txt"]).unwrap();
    conflict(&mut h);
    let err = h
        .try_run("tool.merge", json!({ "path": "a.txt" }))
        .unwrap_err();
    assert!(err.contains("no merge tool is configured"), "{err}");
    assert_eq!(status(&h.git()), "UU a.txt\n");

    // What cannot be a path or a command is refused before git is asked.
    let pwned = h.scratch.path().join("pwned");
    for (path, command) in [
        ("", "true"),
        ("../outside", "true"),
        ("/etc/passwd", "true"),
        ("a.txt", "one\ntwo"),
        ("a.txt", "nul\0"),
    ] {
        for id in ["tool.diff", "tool.merge"] {
            assert!(
                h.try_run(id, json!({ "path": path, "command": command }))
                    .is_err(),
                "{id} {path:?} {command:?}"
            );
        }
    }
    assert!(h.try_run("tool.diff", json!({})).is_err());
    assert!(
        h.try_run(
            "tool.diff",
            json!({ "path": "a.txt", "commit": "--output=pwned" })
        )
        .is_err()
    );
    // A file that is not in conflict has nothing to merge, whatever the
    // command.
    let touch = format!("touch '{}'", pwned.display());
    assert!(
        h.try_run("tool.merge", json!({ "path": "b.txt", "command": touch }))
            .is_err()
    );
    assert!(!pwned.exists());
    assert_eq!(status(&h.git()), "UU a.txt\n");
}

#[test]
fn a_merge_tool_resolves_the_file_or_leaves_it() {
    let mut h = Harness::new();
    conflict(&mut h);
    let git = h.git();

    // A tool that gives up: nothing is resolved, nothing is left behind.
    let out = h.run(
        "tool.merge",
        json!({ "path": "a.txt", "command": "exit 1" }),
    );
    assert_eq!(out["resolved"], false);
    assert_eq!(status(&git), "UU a.txt\n");

    // A tool that ends well and writes nothing (closed without saving):
    // nothing is resolved either.
    let out = h.run("tool.merge", json!({ "path": "a.txt", "command": "true" }));
    assert_eq!(out["resolved"], false);
    assert_eq!(status(&git), "UU a.txt\n");
    assert!(
        std::fs::read_to_string(h.scratch.path().join("a.txt"))
            .unwrap()
            .contains("<<<<<<<")
    );

    // A tool that writes the result from the three versions it is given
    // at once, within the second it was started: git judges by the file's
    // time, which some systems' shells compare to the second only, and
    // would take it for a tool that wrote nothing.
    let out = h.run(
        "tool.merge",
        json!({
            "path": "a.txt",
            "command": "cat \"$BASE\" \"$LOCAL\" \"$REMOTE\" > \"$MERGED\"",
        }),
    );
    assert_eq!(out["resolved"], true);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        format!(
            "{}ours\ntheirs\n",
            git.run(["show", "main~1:a.txt"]).unwrap()
        )
    );
    // Staged as resolved, and no stray files beside it.
    assert_eq!(status(&git), "M  a.txt\n");
    assert!(
        h.run("op.status", json!({}))["operation"]["conflicts"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    assert_eq!(status(&git), "");
}

#[test]
fn the_tool_git_is_configured_with_is_used() {
    let mut h = Harness::new();
    let git = h.git();
    let out = h.out("tool-configured");
    git.run(["config", "diff.tool", "recorder"]).unwrap();
    git.run(["config", "difftool.recorder.cmd", recorder(&out).as_str()])
        .unwrap();
    git.run(["config", "merge.tool", "theirs"]).unwrap();
    git.run([
        "config",
        "mergetool.theirs.cmd",
        "cat \"$REMOTE\" > \"$MERGED\"",
    ])
    .unwrap();
    git.run(["config", "mergetool.theirs.trustExitCode", "true"])
        .unwrap();
    assert_eq!(
        h.run("tool.list", json!({})),
        json!({ "diff": "recorder", "merge": "theirs" })
    );
    h.scratch.write("b.txt", "edited\n").unwrap();
    let started = h.run("tool.diff", json!({ "path": "b.txt" }));
    assert_eq!(started["started"], "recorder");
    assert_eq!(
        recorded(&out),
        (
            git.run(["show", "HEAD:b.txt"]).unwrap(),
            "edited\n".to_owned()
        )
    );
    git.run(["checkout", "-q", "--", "b.txt"]).unwrap();

    conflict(&mut h);
    let out = h.run("tool.merge", json!({ "path": "a.txt" }));
    assert_eq!(out["resolved"], true);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "theirs\n"
    );
    assert_eq!(status(&git), "M  a.txt\n");
}

/// One file of several in conflict is handed to an external tool, what
/// the tool made of it is read back into the worktree and marked
/// resolved, and the other files in conflict are not touched.
#[test]
fn one_file_in_conflict_goes_to_an_external_tool_and_its_result_comes_back() {
    let mut h = Harness::new();
    let git = h.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "theirs a\n").unwrap();
    h.scratch.write("b.txt", "theirs b\n").unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "ours a\n").unwrap();
    h.scratch.write("b.txt", "ours b\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
    let out = h.run("branch.merge", json!({ "target": "topic" }));
    assert_eq!(out["conflicts"], json!(["a.txt", "b.txt"]));
    let other = h.scratch.path().join("b.txt");
    let other_before = std::fs::read(&other).unwrap();
    let stages = |path: &str| git.run(["ls-files", "--unmerged", "--", path]).unwrap();
    let other_stages = stages("b.txt");
    assert_eq!(other_stages.lines().count(), 3);

    // The tool: it writes down the three versions it is handed, and
    // leaves a result of its own.
    let seen = h.out("tool-conflict");
    let dir = seen.display();
    let command = format!(
        "cat \"$BASE\" > '{dir}/base' && cat \"$LOCAL\" > '{dir}/local' && \
         cat \"$REMOTE\" > '{dir}/remote' && printf 'made in the tool\\n' > \"$MERGED\""
    );
    let out = h.run("tool.merge", json!({ "path": "a.txt", "command": command }));
    assert_eq!(out["resolved"], true);

    // It was handed that one file, in its three versions.
    let handed = |name: &str| std::fs::read_to_string(seen.join(name)).unwrap();
    assert_eq!(handed("base"), git.run(["show", "main~1:a.txt"]).unwrap());
    assert_eq!(handed("local"), "ours a\n");
    assert_eq!(handed("remote"), "theirs a\n");

    // Its result is in the worktree, and in the index as the resolution.
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "made in the tool\n"
    );
    assert_eq!(git.run(["show", ":a.txt"]).unwrap(), "made in the tool\n");
    assert_eq!(stages("a.txt"), "");

    // The other file is as it was: in conflict, with the same versions
    // in the index and the same text, markers and all, in the worktree.
    assert_eq!(status(&git), "M  a.txt\nUU b.txt\n");
    assert_eq!(stages("b.txt"), other_stages);
    assert_eq!(std::fs::read(&other).unwrap(), other_before);
    let conflicts = h.run("op.status", json!({}))["operation"]["conflicts"].clone();
    assert_eq!(conflicts.as_array().unwrap().len(), 1);
    assert_eq!(conflicts[0]["path"], "b.txt");
    let err = h.try_run("op.continue", json!({})).unwrap_err();
    assert!(err.contains("1 file is still in conflict"), "{err}");
}

#[test]
fn waiting_for_a_merge_tool_can_be_stopped() {
    let mut h = Harness::new();
    conflict(&mut h);
    let cancel = h.ctx.cancel.clone();
    let stopper = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let started = std::time::Instant::now();
    let err = h
        .try_run(
            "tool.merge",
            json!({ "path": "a.txt", "command": "sleep 6" }),
        )
        .unwrap_err();
    stopper.join().unwrap();
    assert!(err.contains("cancelled"), "{err}");
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "it did not wait for the tool to end"
    );
    // The file is as it was, and nothing of the tool's is in the worktree.
    assert_eq!(status(&h.git()), "UU a.txt\n");
    let leftovers: Vec<_> = std::fs::read_dir(h.scratch.path().join(".git/bitbraid"))
        .map(|dir| {
            dir.filter_map(|entry| entry.ok())
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .filter(|name| name.starts_with("tool-") && name.ends_with(".log"))
                .collect()
        })
        .unwrap_or_default();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

/// Whether a process of this number is still there (a zombie that its
/// parent has not collected counts as gone).
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    let state = std::process::Command::new("ps")
        .args(["-o", "stat=", "-p", &pid.to_string()])
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default();
    !state.is_empty() && !state.starts_with('Z')
}

/// Stopping the wait for a merge tool ends the tool, not only git: a
/// window left open would resolve the file behind the application's back
/// when it is saved later. One that does not end when asked is killed a
/// moment later.
#[cfg(unix)]
#[test]
fn stopping_a_merge_tool_ends_the_tool() {
    for (name, tool) in [
        ("ends when asked", "exec sleep 60"),
        (
            "will not end when asked",
            "trap \"\" TERM; while :; do sleep 1; done",
        ),
    ] {
        let mut h = Harness::new();
        conflict(&mut h);
        let out = h.out("tool-stopped");
        let pid_file = out.join("pid");
        let command = format!("sh -c 'echo $$ > \"{}\"; {tool}'", pid_file.display());
        let cancel = h.ctx.cancel.clone();
        let watched = pid_file.clone();
        let stopper = std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let pid = loop {
                if let Some(pid) = std::fs::read_to_string(&watched)
                    .ok()
                    .and_then(|text| text.trim().parse::<u32>().ok())
                {
                    break pid;
                }
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(20),
                    "the tool never started"
                );
                std::thread::sleep(std::time::Duration::from_millis(20));
            };
            cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            pid
        });
        let err = h
            .try_run("tool.merge", json!({ "path": "a.txt", "command": command }))
            .unwrap_err();
        let pid = stopper.join().unwrap();
        assert!(err.contains("cancelled"), "{name}: {err}");
        let stopped = std::time::Instant::now();
        while alive(pid) {
            assert!(
                stopped.elapsed() < std::time::Duration::from_secs(8),
                "{name}: the tool is still there after it was stopped"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(status(&h.git()), "UU a.txt\n", "{name}");
    }
}

/// The copy with the conflict markers that git keeps beside a file a tool
/// resolved (`a.txt.orig`) is not left in the worktree, unless the user's
/// configuration asks for it.
#[test]
fn a_merge_tool_leaves_no_copy_beside_the_file_unless_asked() {
    let mut h = Harness::new();
    let git = h.git();
    git.run(["config", "--unset", "mergetool.keepBackup"])
        .unwrap();
    conflict(&mut h);
    let theirs = "cat \"$REMOTE\" > \"$MERGED\"".to_owned();
    let out = h.run("tool.merge", json!({ "path": "a.txt", "command": theirs }));
    assert_eq!(out["resolved"], true);
    assert_eq!(status(&git), "M  a.txt\n");
    assert!(!h.scratch.path().join("a.txt.orig").exists());

    // Asked for, it is kept, as git keeps it.
    h.run("op.abort", json!({}));
    git.run(["config", "mergetool.keepBackup", "true"]).unwrap();
    let out = h.run("branch.merge", json!({ "target": "topic" }));
    assert_eq!(out["conflicts"], json!(["a.txt"]));
    let out = h.run("tool.merge", json!({ "path": "a.txt", "command": theirs }));
    assert_eq!(out["resolved"], true);
    assert!(h.scratch.path().join("a.txt.orig").exists());
}

/// A diff tool that gives up at once (a library it lacks, an option it
/// does not take) says so with what it wrote, rather than being taken
/// for started.
#[test]
fn a_diff_tool_that_gives_up_at_once_says_why() {
    let mut h = Harness::new();
    h.scratch.write("a.txt", "changed\n").unwrap();
    let err = h
        .try_run(
            "tool.diff",
            json!({ "path": "a.txt", "command": "echo 'cannot open the display' >&2; exit 3" }),
        )
        .unwrap_err();
    assert!(err.contains("cannot open the display"), "{err}");
}
