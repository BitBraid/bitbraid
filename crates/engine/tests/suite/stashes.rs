//! Stash commands against `git stash` on twin scratch copies.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Index, worktree and stash list as one comparable string. Stash commits
/// carry the date, so they are compared by their trees: the worktree, the
/// index, and the untracked files when the stash has them.
fn state(git: &Git) -> String {
    let mut out = format!(
        "{}\n--\n{}\n--\n{}\n--\n",
        git.run(["status", "--porcelain=v2", "--branch"]).unwrap(),
        git.run(["ls-files", "--stage"]).unwrap(),
        git.run(["diff"]).unwrap(),
    );
    let stashes = git.lines(["stash", "list", "--format=%gd %gs"]).unwrap();
    for (index, line) in stashes.iter().enumerate() {
        out.push_str(line);
        // The parent is resolved first: asking for the tree of a third
        // parent that is not there gets an answer from git all the same.
        for parent in ["", "^2", "^3"] {
            let tree = git
                .run([
                    "rev-parse",
                    "--verify",
                    "--quiet",
                    &format!("stash@{{{index}}}{parent}"),
                ])
                .and_then(|commit| git.rev_parse(&format!("{}^{{tree}}", commit.trim())))
                .unwrap_or_else(|_| "none".to_owned());
            out.push_str(&format!(" {}", tree.trim()));
        }
        out.push('\n');
    }
    out
}

/// A staged change, an unstaged change and an untracked file.
fn dirty(copy: &Scratch) {
    copy.write("a.txt", "edited, not staged\n").unwrap();
    copy.write("b.txt", "edited and staged\n").unwrap();
    copy.git().run(["add", "b.txt"]).unwrap();
    copy.write("new.txt", "untracked\n").unwrap();
}

fn newest(git: &Git) -> String {
    git.rev_parse("stash@{0}").unwrap()
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        for copy in [&scratch, &twin] {
            let git = copy.git();
            git.run(["config", "user.name", "Ada Fixture"]).unwrap();
            git.run(["config", "user.email", "ada@fixture.invalid"])
                .unwrap();
        }
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            twin,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.scratch.git()
    }
    fn both(&self, f: impl Fn(&Scratch)) {
        f(&self.scratch);
        f(&self.twin);
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
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.git()), state(&self.twin.git()), "{what}");
    }
}

#[test]
fn push_matches_git_stash_push_in_every_variant() {
    for (params, flags) in [
        (json!({}), vec![]),
        (
            json!({ "include_untracked": true }),
            vec!["--include-untracked"],
        ),
        (json!({ "keep_index": true }), vec!["--keep-index"]),
        (json!({ "staged_only": true }), vec!["--staged"]),
        (
            json!({ "message": "half of the parser", "include_untracked": true }),
            vec!["--include-untracked", "-m", "half of the parser"],
        ),
        // An empty message is git's own.
        (json!({ "message": "  " }), vec![]),
    ] {
        let mut h = Harness::new();
        h.both(dirty);
        let out = h.run("stash.push", params.clone());
        let mut args = vec!["stash", "push"];
        args.extend(flags);
        h.twin.git().run(args).unwrap();
        h.assert_same(&format!("push {params}"));
        assert_eq!(out["stash"], newest(&h.git()), "{params}");
        assert_eq!(h.run("undo.peek", json!({}))["undo"], "Stash");
    }

    // Nothing to stash: no stash, nothing to undo.
    let mut h = Harness::new();
    let out = h.run("stash.push", json!({}));
    assert!(out["stash"].is_null());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    assert_eq!(h.run("stash.list", json!({}))["stashes"], json!([]));
}

#[test]
fn apply_and_pop_match_git_with_and_without_the_index() {
    for (command, params, args) in [
        ("stash.apply", json!({}), vec!["stash", "apply"]),
        (
            "stash.apply",
            json!({ "index": true }),
            vec!["stash", "apply", "--index"],
        ),
        ("stash.pop", json!({}), vec!["stash", "pop"]),
        (
            "stash.pop",
            json!({ "index": true }),
            vec!["stash", "pop", "--index"],
        ),
    ] {
        let mut h = Harness::new();
        h.both(|copy| {
            dirty(copy);
            copy.git().run(["stash", "push", "-u"]).unwrap();
        });
        h.run(command, params.clone());
        h.twin.git().run(args).unwrap();
        h.assert_same(&format!("{command} {params}"));
        assert!(h.scratch.path().join("new.txt").exists());
    }
}

#[test]
fn a_stash_is_named_by_its_commit_whatever_its_position() {
    let mut h = Harness::new();
    let git = h.git();
    h.scratch.write("a.txt", "first stash\n").unwrap();
    git.run(["stash", "push", "-m", "first"]).unwrap();
    let first = newest(&git);
    h.scratch.write("b.txt", "second stash\n").unwrap();
    git.run(["stash", "push", "-m", "second"]).unwrap();
    let second = newest(&git);

    let list = h.run("stash.list", json!({}));
    assert_eq!(list["stashes"][0]["id"], second);
    assert_eq!(list["stashes"][1]["id"], first);
    assert_eq!(list["stashes"][1]["message"], "On main: first");

    // The older one, by id: the newer stays, and is the only one left.
    h.run("stash.pop", json!({ "stash": first }));
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "first stash\n"
    );
    assert_eq!(
        git.lines(["stash", "list", "--format=%H"]).unwrap(),
        std::slice::from_ref(&second)
    );
    // It is gone from the list, so it cannot be used again.
    assert!(h.try_run("stash.apply", json!({ "stash": first })).is_err());
    assert!(h.try_run("stash.drop", json!({ "stash": first })).is_err());
}

#[test]
fn a_stash_that_would_conflict_is_refused_and_nothing_changes() {
    let mut h = Harness::new();
    let git = h.git();
    h.scratch.write("a.txt", "stashed version\n").unwrap();
    h.scratch.write("b.txt", "stashed too\n").unwrap();
    git.run(["stash", "push"]).unwrap();
    h.scratch.write("a.txt", "committed since\n").unwrap();
    git.run(["commit", "-q", "-am", "another edit of a"])
        .unwrap();
    // Unrelated local work that must not be disturbed.
    h.scratch.write("README.md", "local\n").unwrap();
    h.scratch.write("staged.txt", "staged\n").unwrap();
    git.run(["add", "staged.txt"]).unwrap();
    let before = state(&git);

    for (command, params) in [
        ("stash.apply", json!({})),
        ("stash.pop", json!({})),
        ("stash.pop", json!({ "index": true })),
    ] {
        let err = h.try_run(command, params).unwrap_err();
        assert!(err.contains("would conflict in a.txt"), "{command}: {err}");
        assert_eq!(state(&git), before, "{command}");
    }
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "committed since\n",
        "no conflict markers"
    );

    // On a branch made where the stash was made it fits; git stash branch
    // does the same on the twin.
    let mut h = Harness::new();
    h.both(|copy| {
        let git = copy.git();
        copy.write("a.txt", "stashed version\n").unwrap();
        git.run(["stash", "push"]).unwrap();
        copy.write("a.txt", "committed since\n").unwrap();
        git.run(["commit", "-q", "-am", "another edit of a"])
            .unwrap();
    });
    let before = state(&h.git());
    let listed = h.git().run(["stash", "list"]).unwrap();
    let undo_before = h.run("undo.peek", json!({}))["undo_id"].clone();
    h.run("stash.branch", json!({ "name": "rescued" }));
    h.twin.git().run(["stash", "branch", "rescued"]).unwrap();
    h.assert_same("stash to a new branch");
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "rescued\n"
    );
    let done = state(&h.git());

    // It is two steps of the journal, the pop and the branch, and taken
    // back in that order the repository is what it was: the stash in the
    // list again, the branch gone, the branch it was begun from checked
    // out.
    let undone = |h: &mut Harness| {
        h.run("undo.last", json!({}))["undone"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(undone(&mut h), "Stash on a new branch: pop onto rescued");
    assert_eq!(h.git().run(["stash", "list"]).unwrap(), listed);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        format!("{}\n", base_of_a(&h)),
        "the file is what the branch has"
    );
    assert_eq!(undone(&mut h), "Stash on a new branch: create rescued");
    assert_eq!(state(&h.git()), before);
    assert!(h.git().rev_parse("refs/heads/rescued").is_err());
    assert_eq!(h.run("undo.peek", json!({}))["undo_id"], undo_before);
    // And done again, step by step.
    h.run("redo.last", json!({}));
    h.run("redo.last", json!({}));
    assert_eq!(state(&h.git()), done);
    h.assert_same("redone");

    // A name that is taken, and a stash that is none: refused with
    // nothing made and nothing journalled.
    let peek = h.run("undo.peek", json!({}))["undo_id"].clone();
    for params in [
        json!({ "name": "rescued" }),
        json!({ "name": "main" }),
        json!({ "name": "other", "stash": "0000000000000000000000000000000000000000" }),
    ] {
        assert!(
            h.try_run("stash.branch", params.clone()).is_err(),
            "{params}"
        );
        assert_eq!(state(&h.git()), done, "{params}");
    }
    assert!(h.git().rev_parse("refs/heads/other").is_err());
    assert_eq!(h.run("undo.peek", json!({}))["undo_id"], peek);

    // Local changes that are in the way of the checkout: the branch that
    // was made for it is not left behind, and the stash stays.
    let mut h = Harness::new();
    let git = h.git();
    h.scratch.write("a.txt", "stashed version\n").unwrap();
    git.run(["stash", "push"]).unwrap();
    h.scratch.write("a.txt", "committed since\n").unwrap();
    git.run(["commit", "-q", "-am", "another edit of a"])
        .unwrap();
    h.scratch.write("a.txt", "and edited again\n").unwrap();
    let before = state(&git);
    let err = h
        .try_run("stash.branch", json!({ "name": "blocked" }))
        .unwrap_err();
    assert!(err.contains("a.txt"), "{err}");
    assert_eq!(state(&git), before);
    assert!(git.rev_parse("refs/heads/blocked").is_err());
    assert_eq!(git.lines(["stash", "list"]).unwrap().len(), 1);
}

/// What `a.txt` holds on the commit the stash of the test above was made
/// on: what the worktree has again once the pop is taken back, on the new
/// branch that stands on that commit.
fn base_of_a(h: &Harness) -> String {
    h.git()
        .run(["show", "rescued:a.txt"])
        .unwrap()
        .trim_end_matches('\n')
        .to_owned()
}

#[test]
fn an_untracked_file_in_the_way_stops_the_stash_before_it_starts() {
    let mut h = Harness::new();
    let git = h.git();
    h.scratch.write("a.txt", "edited\n").unwrap();
    h.scratch.write("new.txt", "stashed untracked\n").unwrap();
    h.scratch.write("other.txt", "also stashed\n").unwrap();
    git.run(["stash", "push", "-u"]).unwrap();
    h.scratch.write("new.txt", "written since\n").unwrap();
    let before = state(&git);

    for command in ["stash.apply", "stash.pop"] {
        let err = h.try_run(command, json!({})).unwrap_err();
        assert!(err.contains("new.txt"), "{command}: {err}");
        assert_eq!(state(&git), before, "{command}");
        assert!(
            !h.scratch.path().join("other.txt").exists(),
            "{command} brought back half of the untracked files"
        );
    }
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("new.txt")).unwrap(),
        "written since\n"
    );
}

#[test]
fn stashing_is_undone_by_putting_everything_back() {
    for params in [
        json!({}),
        json!({ "include_untracked": true }),
        json!({ "keep_index": true }),
        json!({ "staged_only": true }),
    ] {
        let mut h = Harness::new();
        dirty(&h.scratch);
        let git = h.git();
        let before = state(&git);
        h.run("stash.push", params.clone());
        assert_ne!(state(&git), before, "{params}");
        h.run("undo.last", json!({}));
        assert_eq!(state(&git), before, "undo of push {params}");
        h.run("redo.last", json!({}));
        assert_eq!(git.lines(["stash", "list"]).unwrap().len(), 1, "{params}");
        h.run("undo.last", json!({}));
        assert_eq!(state(&git), before, "second undo of push {params}");
    }

    // Another stash on top: the first one no longer goes back by undo.
    let mut h = Harness::new();
    dirty(&h.scratch);
    let git = h.git();
    h.run("stash.push", json!({}));
    h.scratch.write("a.txt", "more work\n").unwrap();
    git.run(["stash", "push"]).unwrap();
    let before = state(&git);
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("no longer the newest"), "{err}");
    assert_eq!(state(&git), before);
}

#[test]
fn a_dropped_stash_comes_back_with_undo() {
    let mut h = Harness::new();
    let git = h.git();
    h.both(|copy| {
        copy.write("a.txt", "first\n").unwrap();
        copy.git().run(["stash", "push", "-m", "first"]).unwrap();
        copy.write("b.txt", "second\n").unwrap();
        copy.git().run(["stash", "push", "-m", "second"]).unwrap();
    });
    let first = git.rev_parse("stash@{1}").unwrap();
    let second = git.rev_parse("stash@{0}").unwrap();

    h.run("stash.drop", json!({ "stash": first }));
    h.twin.git().run(["stash", "drop", "stash@{1}"]).unwrap();
    h.assert_same("drop of the older stash");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Drop stash");

    h.run("undo.last", json!({}));
    let list = h.run("stash.list", json!({}));
    // Back in the list (as the newest), the same commit under its message.
    assert_eq!(list["stashes"][0]["id"], first);
    assert_eq!(list["stashes"][0]["message"], "On main: first");
    assert_eq!(list["stashes"][1]["id"], second);
    h.run("redo.last", json!({}));
    h.assert_same("redo of the drop");

    // Without a stash given, the newest goes.
    h.run("stash.drop", json!({}));
    assert_eq!(h.run("stash.list", json!({}))["stashes"], json!([]));
    assert!(h.try_run("stash.drop", json!({})).is_err());
    assert!(h.try_run("stash.pop", json!({})).is_err());
    h.run("undo.last", json!({}));
    assert_eq!(h.run("stash.list", json!({}))["stashes"][0]["id"], second);
}

#[test]
fn applying_and_popping_are_undone_file_by_file() {
    for (command, params, args, label) in [
        (
            "stash.apply",
            json!({}),
            vec!["stash", "apply"],
            "Apply stash",
        ),
        (
            "stash.apply",
            json!({ "index": true }),
            vec!["stash", "apply", "--index"],
            "Apply stash",
        ),
        ("stash.pop", json!({}), vec!["stash", "pop"], "Pop stash"),
        (
            "stash.pop",
            json!({ "index": true }),
            vec!["stash", "pop", "--index"],
            "Pop stash",
        ),
    ] {
        let mut h = Harness::new();
        // Git brings the index back only into an index without changes.
        let with_index = params.get("index").is_some();
        h.both(|copy| {
            dirty(copy);
            copy.git().run(["stash", "push", "-u"]).unwrap();
            // Work since, in a file the stash does not touch: one part
            // of it staged where that is allowed, one not, and a file of
            // its own.
            copy.write("README.md", "# since\n").unwrap();
            if !with_index {
                copy.git().run(["add", "README.md"]).unwrap();
            }
            copy.write("README.md", "# since, and more\n").unwrap();
            copy.write("other.txt", "since\n").unwrap();
        });
        // The twin never applies: that is what undo has to come back to.
        let never = state(&h.twin.git());
        assert_eq!(state(&h.git()), never);

        h.run(command, params.clone());
        assert_eq!(h.run("undo.peek", json!({}))["undo"], label, "{params}");
        assert!(h.scratch.path().join("new.txt").exists());
        h.run("undo.last", json!({}));
        assert_eq!(state(&h.git()), never, "undo of {command} {params}");
        assert!(!h.scratch.path().join("new.txt").exists());
        assert_eq!(
            std::fs::read_to_string(h.scratch.path().join("other.txt")).unwrap(),
            "since\n"
        );

        // Redo is the application again, as git does it.
        h.run("redo.last", json!({}));
        h.twin.git().run(args).unwrap();
        h.assert_same(&format!("redo of {command} {params}"));
        h.run("undo.last", json!({}));
        assert_eq!(state(&h.git()), never, "undo after redo");
    }
}

#[test]
fn undo_of_an_applied_stash_refuses_once_its_files_changed() {
    let mut h = Harness::new();
    let git = h.git();
    dirty(&h.scratch);
    git.run(["stash", "push", "-u"]).unwrap();
    let stashed = state(&git);
    h.run("stash.pop", json!({}));
    let popped = state(&git);

    // A file the stash brought back was worked on since.
    h.scratch.write("a.txt", "edited again\n").unwrap();
    let changed = state(&git);
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("a.txt was changed since"), "{err}");
    assert_eq!(state(&git), changed, "nothing was changed");
    assert_eq!(h.run("stash.list", json!({}))["stashes"], json!([]));

    // Or staged since.
    h.scratch.write("a.txt", "edited, not staged\n").unwrap();
    assert_eq!(state(&git), popped);
    git.run(["add", "new.txt"]).unwrap();
    let staged = state(&git);
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("new.txt was changed since"), "{err}");
    assert_eq!(state(&git), staged);

    // As popping left it, it goes back, and the stash is in the list.
    git.run(["reset", "-q", "--", "new.txt"]).unwrap();
    h.run("undo.last", json!({}));
    assert_eq!(state(&git), stashed);
}

#[test]
fn what_a_stash_holds_is_what_git_lists_untracked_files_included() {
    let mut h = Harness::new();
    let git = h.git();
    dirty(&h.scratch);
    std::fs::remove_file(h.scratch.path().join("README.md")).unwrap();
    h.scratch.write("dir/deep.txt", "one\ntwo\n").unwrap();
    git.run(["stash", "push", "-u"]).unwrap();
    let letter = |kind: &str| match kind {
        "added" => "A",
        "deleted" => "D",
        "modified" => "M",
        other => panic!("{other}"),
    };
    let listed = |out: &Value| -> (Vec<String>, Vec<String>) {
        let files = out["files"].as_array().unwrap();
        (
            files
                .iter()
                .map(|file| {
                    format!(
                        "{}\t{}",
                        letter(file["kind"].as_str().unwrap()),
                        file["path"].as_str().unwrap()
                    )
                })
                .collect(),
            files
                .iter()
                .map(|file| {
                    format!(
                        "{}\t{}\t{}",
                        file["additions"],
                        file["deletions"],
                        file["path"].as_str().unwrap()
                    )
                })
                .collect(),
        )
    };
    let out = h.run("stash.show", json!({}));
    let (names, counts) = listed(&out);
    assert_eq!(
        names,
        git.lines([
            "stash",
            "show",
            "--include-untracked",
            "--name-status",
            "stash@{0}"
        ])
        .unwrap()
    );
    assert_eq!(
        counts,
        git.lines([
            "stash",
            "show",
            "--include-untracked",
            "--numstat",
            "stash@{0}"
        ])
        .unwrap()
    );
    assert!(names.contains(&"A\tnew.txt".to_owned()), "{names:?}");
    assert!(names.contains(&"A\tdir/deep.txt".to_owned()), "{names:?}");
    // The untracked file's lines are there to read.
    let deep = out["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "dir/deep.txt")
        .unwrap();
    assert_eq!(deep["additions"], 2);
    assert!(!deep["hunks"].as_array().unwrap().is_empty());

    // A stash without untracked files is its tracked changes alone.
    h.scratch.write("a.txt", "again\n").unwrap();
    git.run(["stash", "push"]).unwrap();
    let id = newest(&git);
    let out = h.run("stash.show", json!({ "stash": id }));
    assert_eq!(
        listed(&out).0,
        git.lines(["stash", "show", "--name-status", "stash@{0}"])
            .unwrap()
    );
}

#[test]
fn hostile_stash_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let git = h.git();
    dirty(&h.scratch);
    git.run(["stash", "push", "-u", "-m", "kept"]).unwrap();
    h.scratch.write("a.txt", "local\n").unwrap();
    let head = git.rev_parse("HEAD").unwrap();
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    let before = state(&git);
    let long = "f".repeat(5000);

    for bad in [
        json!(""),
        json!("-x"),
        json!("--index"),
        json!("stash@{0}"),
        json!("stash@{99}"),
        json!("HEAD"),
        json!(head),
        json!(tree),
        json!("0000000000000000000000000000000000000000"),
        json!(long),
        json!(7),
        json!(["a"]),
    ] {
        for command in ["stash.apply", "stash.pop", "stash.drop", "stash.show"] {
            assert!(
                h.try_run(command, json!({ "stash": bad })).is_err(),
                "{command} accepted {bad}"
            );
        }
        assert!(
            h.try_run("stash.branch", json!({ "name": "fine", "stash": bad }))
                .is_err(),
            "stash.branch accepted {bad}"
        );
    }
    for name in ["", "-x", "a..b", "a b", "HEAD", "@{-1}", long.as_str()] {
        assert!(
            h.try_run("stash.branch", json!({ "name": name })).is_err(),
            "stash.branch accepted the name {name:?}"
        );
    }
    for message in ["line\nbreak", "bell\u{7}", "nul\u{0}"] {
        assert!(
            h.try_run("stash.push", json!({ "message": message }))
                .is_err(),
            "{message:?}"
        );
    }
    for params in [
        json!({ "message": 3 }),
        json!({ "include_untracked": "yes" }),
        json!({ "staged_only": true, "include_untracked": true }),
    ] {
        assert!(h.try_run("stash.push", params.clone()).is_err(), "{params}");
    }
    assert_eq!(state(&git), before);
    assert_eq!(git.lines(["branch", "--list"]).unwrap().len(), 1);
}

#[test]
fn a_stash_gets_another_message_in_its_place_and_undoes() {
    let mut h = Harness::new();
    let git = h.git();
    h.both(|copy| {
        copy.write("a.txt", "first\n").unwrap();
        copy.git().run(["stash", "push", "-m", "first"]).unwrap();
        copy.write("b.txt", "second\n").unwrap();
        copy.git().run(["stash", "push", "-m", "second"]).unwrap();
    });
    let first = git.rev_parse("stash@{1}").unwrap();
    let second = git.rev_parse("stash@{0}").unwrap();
    let made = |rev: &str| {
        git.run(["log", "-1", "--format=%an %ae %ad %cd", rev])
            .unwrap()
    };
    let when = made("stash@{1}");

    let out = h.run(
        "stash.reword",
        json!({ "stash": first, "message": "renamed" }),
    );
    let now = out["stash"].as_str().unwrap().to_owned();
    assert_ne!(now, first);
    // The same with git: the two taken off, the older one put back under
    // its new message, then the newer one.
    let twin = h.twin.git();
    let (t0, t1) = (
        twin.rev_parse("stash@{0}").unwrap(),
        twin.rev_parse("stash@{1}").unwrap(),
    );
    twin.run(["stash", "drop", "stash@{0}"]).unwrap();
    twin.run(["stash", "drop", "stash@{0}"]).unwrap();
    twin.run(["stash", "store", "-m", "On main: renamed", t1.as_str()])
        .unwrap();
    twin.run(["stash", "store", "-m", "On main: second", t0.as_str()])
        .unwrap();
    h.assert_same("reword");
    // In its place, its own commit saying so, made by whom and when the
    // old one was.
    assert_eq!(git.rev_parse("stash@{1}").unwrap(), now);
    assert_eq!(git.rev_parse("stash@{0}").unwrap(), second);
    assert_eq!(
        git.run(["log", "-1", "--format=%s", "stash@{1}"]).unwrap(),
        "On main: renamed\n"
    );
    assert_eq!(made("stash@{1}"), when);
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Edit stash message");

    // Undone: the old stash in its place; done again: the new one.
    h.run("undo.last", json!({}));
    let list = h.run("stash.list", json!({}));
    assert_eq!(list["stashes"][1]["id"], first);
    assert_eq!(list["stashes"][1]["message"], "On main: first");
    assert_eq!(list["stashes"][0]["id"], second);
    h.run("redo.last", json!({}));
    let list = h.run("stash.list", json!({}));
    assert_eq!(list["stashes"][1]["id"], now);
    assert_eq!(list["stashes"][1]["message"], "On main: renamed");
    // It is applied like the stash it was.
    h.run("stash.apply", json!({ "stash": now }));
    h.twin.git().run(["stash", "apply", "stash@{1}"]).unwrap();
    h.assert_same("apply of the reworded stash");
    h.both(|copy| {
        copy.git().run(["checkout", "--", "."]).unwrap();
    });

    // Without a stash named, the newest.
    h.run("stash.reword", json!({ "message": "on top" }));
    assert_eq!(
        h.run("stash.list", json!({}))["stashes"][0]["message"],
        "On main: on top"
    );

    // Hostile messages and stashes: refused, nothing changes.
    let before = state(&git);
    let long = "m".repeat(5000);
    for message in [
        "",
        "  ",
        "line\nbreak",
        "bell\u{7}",
        "nul\u{0}",
        long.as_str(),
    ] {
        assert!(
            h.try_run("stash.reword", json!({ "stash": now, "message": message }))
                .is_err(),
            "{message:?}"
        );
    }
    let head = git.rev_parse("HEAD").unwrap();
    for bad in [
        json!(""),
        json!("-x"),
        json!("stash@{0}"),
        json!(head),
        json!("0000000000000000000000000000000000000000"),
        json!(7),
    ] {
        assert!(
            h.try_run("stash.reword", json!({ "stash": bad, "message": "m" }))
                .is_err(),
            "{bad}"
        );
    }
    assert!(h.try_run("stash.reword", json!({ "stash": now })).is_err());
    assert_eq!(state(&git), before);
}
