//! Operations that stop on a conflict: what is reported, resolving file by
//! file, continuing, skipping and aborting, each against the same thing
//! done by hand with git on a twin copy.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// The first-parent history as (tree, number of parents, subject): commit
/// ids carry the date, these do not.
fn shape(git: &Git) -> Vec<String> {
    git.lines(["log", "--first-parent", "--format=%T %P|%s", "HEAD"])
        .unwrap()
        .into_iter()
        .map(|line| {
            let (ids, subject) = line.split_once('|').unwrap();
            let mut ids = ids.split(' ');
            let tree = ids.next().unwrap().to_owned();
            format!("{tree} {} {subject}", ids.count())
        })
        .collect()
}

fn status(git: &Git) -> String {
    git.run(["status", "--porcelain"]).unwrap()
}

/// Both lines of a history that conflict in every way a file can:
/// `a.txt` changed on both, `b.txt` changed on main and deleted on topic,
/// `README.md` deleted on main and changed on topic, `new.txt` added on
/// both, and `calm.txt` only on topic, which merges cleanly. Topic has a
/// second commit that touches nothing main does.
fn diverge(copy: &Scratch) {
    let git = copy.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("a.txt", "topic version\n").unwrap();
    copy.write("README.md", "topic's readme\n").unwrap();
    copy.write("new.txt", "topic's new file\n").unwrap();
    copy.write("calm.txt", "only on topic\n").unwrap();
    git.run(["rm", "-q", "b.txt"]).unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "topic: everything at once"])
        .unwrap();
    copy.write("later.txt", "a second commit\n").unwrap();
    git.run(["add", "later.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "topic: something else"])
        .unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write("a.txt", "main version\n").unwrap();
    copy.write("b.txt", "main's b\n").unwrap();
    copy.write("new.txt", "main's new file\n").unwrap();
    git.run(["rm", "-q", "README.md"]).unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "main: everything at once"])
        .unwrap();
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        diverge(&scratch);
        diverge(&twin);
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
        assert_eq!(
            shape(&self.git()),
            shape(&self.twin.git()),
            "{what}: history"
        );
        assert_eq!(
            status(&self.git()),
            status(&self.twin.git()),
            "{what}: status"
        );
        assert_eq!(
            self.git().run(["ls-files", "--stage"]).unwrap(),
            self.twin.git().run(["ls-files", "--stage"]).unwrap(),
            "{what}: index"
        );
    }
    fn note(&self) -> std::path::PathBuf {
        self.scratch.path().join(".git/bitbraid/operation.json")
    }
    /// Conflicted paths with their kinds, as the engine reports them.
    fn kinds(&mut self) -> Vec<(String, String)> {
        let out = self.run("op.status", json!({}));
        out["operation"]["conflicts"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                (
                    c["path"].as_str().unwrap().to_owned(),
                    c["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }
}

/// Resolve the four conflicts of merging topic into main, by hand.
fn resolve_by_hand(copy: &Scratch) {
    let git = copy.git();
    git.run(["checkout", "--theirs", "--", "a.txt"]).unwrap();
    git.run(["add", "a.txt"]).unwrap();
    git.run(["rm", "-q", "--", "b.txt"]).unwrap();
    git.run(["checkout", "--theirs", "--", "README.md"])
        .unwrap();
    git.run(["add", "README.md"]).unwrap();
    copy.write("new.txt", "both, put together\n").unwrap();
    git.run(["add", "new.txt"]).unwrap();
}

/// The same through the commands.
fn resolve_here(h: &mut Harness) {
    h.run(
        "conflict.take",
        json!({ "path": "a.txt", "side": "theirs" }),
    );
    // Their side deleted it: taking theirs removes the file.
    h.run(
        "conflict.take",
        json!({ "path": "b.txt", "side": "theirs" }),
    );
    h.run(
        "conflict.take",
        json!({ "path": "README.md", "side": "theirs" }),
    );
    h.scratch.write("new.txt", "both, put together\n").unwrap();
    h.run("conflict.resolve", json!({ "path": "new.txt" }));
}

#[test]
fn a_merge_stops_on_its_conflicts_and_is_finished_file_by_file() {
    let mut h = Harness::new();
    let start = h.git().rev_parse("HEAD").unwrap();
    assert!(h.run("op.status", json!({}))["operation"].is_null());

    let out = h.run("branch.merge", json!({ "target": "topic" }));
    assert!(out["stopped"].is_string(), "{out}");
    assert!(h.twin.git().run(["merge", "--no-edit", "topic"]).is_err());
    h.assert_same("stopped merge");
    assert!(
        h.run("undo.peek", json!({}))["undo"].is_null(),
        "nothing journaled yet"
    );
    assert!(h.note().is_file());

    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "merge");
    // The two sides by the branch at them and their short ids.
    let short = |rev: &str| h.git().rev_parse(rev).unwrap()[..7].to_owned();
    assert_eq!(operation["ours"], format!("main at {}", short("HEAD")));
    assert_eq!(operation["theirs"], format!("topic at {}", short("topic")));
    assert_eq!(operation["can_skip"], false);
    assert_eq!(operation["current"], h.git().rev_parse("topic").unwrap());
    assert_eq!(
        h.kinds(),
        [
            ("README.md".to_owned(), "deleted_by_us".to_owned()),
            ("a.txt".to_owned(), "both_modified".to_owned()),
            ("b.txt".to_owned(), "deleted_by_them".to_owned()),
            ("new.txt".to_owned(), "both_added".to_owned()),
        ]
    );
    assert_eq!(
        out["conflicts"],
        json!(["README.md", "a.txt", "b.txt", "new.txt"])
    );

    // Not yet: files are in conflict, and a merge has nothing to skip.
    let err = h.try_run("op.continue", json!({})).unwrap_err();
    assert!(err.contains("4 files are still in conflict"), "{err}");
    assert!(h.try_run("op.skip", json!({})).is_err());
    // Marking a file resolved while git's markers are in it is a mistake
    // more often than not.
    let err = h
        .try_run("conflict.resolve", json!({ "path": "new.txt" }))
        .unwrap_err();
    assert!(err.contains("conflict markers"), "{err}");

    resolve_here(&mut h);
    resolve_by_hand(&h.twin);
    h.assert_same("resolved, before the commit");
    assert!(h.kinds().is_empty());

    // Changed my mind about one: back in conflict, then our side after all.
    h.run("conflict.reopen", json!({ "path": "a.txt" }));
    assert_eq!(
        h.kinds(),
        [("a.txt".to_owned(), "both_modified".to_owned())]
    );
    assert!(h.scratch.path().join("a.txt").exists());
    h.run("conflict.take", json!({ "path": "a.txt", "side": "ours" }));
    h.twin.git().run(["checkout", "-m", "--", "a.txt"]).unwrap();
    h.twin
        .git()
        .run(["checkout", "--ours", "--", "a.txt"])
        .unwrap();
    h.twin.git().run(["add", "a.txt"]).unwrap();

    let done = h.run("op.continue", json!({}));
    assert_eq!(done["finished"], true);
    assert!(done["operation"].is_null());
    h.twin.git().run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("finished merge");
    assert!(!h.note().exists());

    // Now it is one step of the journal, like a merge that never stopped.
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Merge topic");
    let merged = h.git().rev_parse("HEAD").unwrap();
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    h.run("redo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), merged);
}

#[test]
fn a_rebase_stops_commit_by_commit_and_can_skip() {
    for skip in [false, true] {
        let mut h = Harness::new();
        for copy in [&h.scratch, &h.twin] {
            copy.git().run(["switch", "-q", "topic"]).unwrap();
        }
        let start = h.git().rev_parse("HEAD").unwrap();
        let out = h.run("branch.rebase", json!({ "onto": "main" }));
        assert!(out["stopped"].is_string(), "{out}");
        assert!(h.twin.git().run(["rebase", "main"]).is_err());

        let operation = h.run("op.status", json!({}))["operation"].clone();
        assert_eq!(operation["kind"], "rebase");
        assert_eq!(operation["step"], json!([1, 2]));
        assert_eq!(operation["branch"], "topic");
        assert_eq!(operation["onto"], h.git().rev_parse("main").unwrap());
        assert_eq!(operation["original_head"], start);
        assert_eq!(operation["can_skip"], true);
        // In a rebase "ours" is what the commits are replayed onto, and
        // "theirs" the commit being replayed, each by its branch and id.
        let short = |rev: &str| h.git().rev_parse(rev).unwrap()[..7].to_owned();
        assert_eq!(operation["ours"], format!("main at {}", short("main")));
        let replayed = operation["current"].as_str().unwrap()[..7].to_owned();
        assert_eq!(operation["theirs"], format!("topic at {replayed}"));
        assert_eq!(h.kinds().len(), 4);

        if skip {
            let done = h.run("op.skip", json!({}));
            assert_eq!(done["finished"], true, "{done}");
            h.twin.git().run(["rebase", "--skip"]).unwrap();
        } else {
            // Keep the replayed commit's version of everything.
            for path in ["a.txt", "b.txt", "README.md", "new.txt"] {
                h.run("conflict.take", json!({ "path": path, "side": "theirs" }));
            }
            let twin = h.twin.git();
            twin.run([
                "checkout",
                "--theirs",
                "--",
                "a.txt",
                "README.md",
                "new.txt",
            ])
            .unwrap();
            twin.run(["add", "a.txt", "README.md", "new.txt"]).unwrap();
            twin.run(["rm", "-q", "--", "b.txt"]).unwrap();
            let done = h.run("op.continue", json!({}));
            assert_eq!(done["finished"], true, "{done}");
            twin.run_with(["rebase", "--continue"], &[("GIT_EDITOR", "true")])
                .unwrap();
        }
        h.assert_same(if skip { "rebase with a skip" } else { "rebase" });
        assert_eq!(
            h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
            "topic\n"
        );
        assert_eq!(h.run("undo.peek", json!({}))["undo"], "Rebase onto main");
        h.run("undo.last", json!({}));
        assert_eq!(h.git().rev_parse("topic").unwrap(), start);
        assert_eq!(status(&h.git()), "");
    }
}

#[test]
fn picks_and_reverts_stop_too_and_abort_goes_back() {
    let mut h = Harness::new();
    let git = h.git();
    let start = git.rev_parse("HEAD").unwrap();
    let pick = git.rev_parse("topic~1").unwrap();
    let journal_before = h.run("undo.peek", json!({}));

    let out = h.run("commit.cherry_pick", json!({ "commits": [pick] }));
    assert!(out["stopped"].is_string(), "{out}");
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "cherry_pick");
    assert_eq!(operation["current"], pick);
    assert!(h.note().is_file());
    // Nothing else starts meanwhile, undo included.
    let err = h
        .try_run("branch.merge", json!({ "target": "topic" }))
        .unwrap_err();
    assert!(err.contains("in progress"), "{err}");

    h.run("op.abort", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&git), "");
    assert!(h.run("op.status", json!({}))["operation"].is_null());
    assert!(!h.note().exists());
    assert_eq!(h.run("undo.peek", json!({})), journal_before);
    for command in ["op.continue", "op.skip", "op.abort"] {
        let err = h.try_run(command, json!({})).unwrap_err();
        assert!(err.contains("nothing is in progress"), "{command}: {err}");
    }

    // A revert of main's own commit after another edit of the same file.
    for copy in [&h.scratch, &h.twin] {
        copy.write("a.txt", "edited again\n").unwrap();
        copy.git()
            .run(["commit", "-q", "-am", "main: a again"])
            .unwrap();
    }
    let out = h.run("commit.revert", json!({ "commits": ["HEAD~1"] }));
    assert!(out["stopped"].is_string(), "{out}");
    assert!(h.twin.git().run(["revert", "--no-edit", "HEAD~1"]).is_err());
    assert_eq!(h.run("op.status", json!({}))["operation"]["kind"], "revert");
    let conflicted: Vec<String> = h.kinds().into_iter().map(|(path, _)| path).collect();
    for path in &conflicted {
        h.run("conflict.take", json!({ "path": path, "side": "ours" }));
        // By hand, "ours" of a file our side deleted is its removal.
        let twin = h.twin.git();
        if twin.run(["checkout", "--ours", "--", path]).is_ok() {
            twin.run(["add", "--", path]).unwrap();
        } else {
            twin.run(["rm", "-q", "--", path]).unwrap();
        }
    }
    let after_resolving = status(&h.git());
    assert_eq!(after_resolving, status(&h.twin.git()));
    // Everything taken from our side: the revert changes nothing, which git
    // will not commit. That is said, and the way out is to skip.
    match h.try_run("op.continue", json!({})) {
        Ok(done) => {
            assert_eq!(done["finished"], true);
            h.twin
                .git()
                .run_with(["revert", "--continue"], &[("GIT_EDITOR", "true")])
                .unwrap();
        }
        Err(_) => {
            let done = h.run("op.skip", json!({}));
            assert_eq!(done["finished"], true);
            h.twin.git().run(["revert", "--skip"]).unwrap();
        }
    }
    h.assert_same("revert resolved to our side");
}

#[test]
fn an_operation_begun_in_a_shell_is_seen_and_finished_but_not_journaled() {
    let mut h = Harness::new();
    let git = h.git();
    assert!(git.run(["merge", "--no-edit", "topic"]).is_err());
    assert!(h.twin.git().run(["merge", "--no-edit", "topic"]).is_err());
    assert_eq!(h.run("op.status", json!({}))["operation"]["kind"], "merge");
    resolve_here(&mut h);
    resolve_by_hand(&h.twin);
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    h.twin.git().run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("merge begun in a shell");
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // A note left by an operation that was ended somewhere else is not
    // taken for the next one's.
    let mut h = Harness::new();
    let git = h.git();
    h.run("branch.merge", json!({ "target": "topic" }));
    assert!(h.note().is_file());
    git.run(["merge", "--abort"]).unwrap();
    h.scratch.write("unrelated.txt", "x\n").unwrap();
    git.run(["add", "unrelated.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "main moved on"]).unwrap();
    assert!(git.run(["merge", "--no-edit", "topic"]).is_err());
    resolve_here(&mut h);
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    assert!(!h.note().exists());
}

/// The text a file gets when every conflict is taken from one side, or
/// from both, out of the regions the merge tool is given.
fn rebuilt(detail: &Value, take: &str) -> String {
    let lines = |value: &Value| -> String {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|line| line.as_str().unwrap())
            .collect()
    };
    detail["regions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|region| match region["kind"].as_str().unwrap() {
            "same" => lines(&region["lines"]),
            _ if take == "union" => lines(&region["ours"]) + &lines(&region["theirs"]),
            _ => lines(&region[take]),
        })
        .collect()
}

#[test]
fn the_merge_tool_gets_the_file_cut_where_git_cuts_it() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    // A longer file: both sides change the same lines in two places and
    // different lines elsewhere, and one line looks like a conflict marker.
    let base: Vec<String> = (1..=40).map(|n| format!("line {n}")).collect();
    let write = |lines: &[String]| {
        scratch
            .write("long.txt", &(lines.join("\n") + "\n"))
            .unwrap()
    };
    write(&base);
    git.run(["add", "long.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "long file"]).unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    let mut topic = base.clone();
    topic[4] = "line 5, by topic".to_owned();
    topic[19] = "<<<<<<< this line is text, not a marker".to_owned();
    topic[29] = "line 30, by topic".to_owned();
    topic.insert(30, "added by topic".to_owned());
    topic[38] = "line 38, only topic touched this".to_owned();
    write(&topic);
    git.run(["commit", "-q", "-am", "topic edits"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    let mut main = base.clone();
    main[4] = "line 5, by main".to_owned();
    main[11] = "line 12, only main touched this".to_owned();
    main[29] = "line 30, by main".to_owned();
    write(&main);
    git.run(["commit", "-q", "-am", "main edits"]).unwrap();

    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .unwrap();
    let mut run = |id: &str, params: Value| registry.run(id, &mut ctx, &params);
    run("branch.merge", json!({ "target": "topic" })).unwrap();
    let detail = run("conflict.detail", json!({ "path": "long.txt" })).unwrap();
    assert!(detail["whole_only"].is_null());
    let conflicts: Vec<&Value> = detail["regions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|region| region["kind"] == "conflict")
        .collect();
    assert_eq!(conflicts.len(), 2, "{detail}");
    assert_eq!(conflicts[0]["ours"], json!(["line 5, by main\n"]));
    assert_eq!(conflicts[0]["base"], json!(["line 5\n"]));
    assert_eq!(conflicts[0]["theirs"], json!(["line 5, by topic\n"]));

    // Taking every conflict from one side, or from both, gives what
    // git merge-file gives when told the same.
    for (take, flag) in [
        ("ours", "--ours"),
        ("theirs", "--theirs"),
        ("union", "--union"),
    ] {
        let dir = scratch.path().join("tmp-merge");
        std::fs::create_dir_all(&dir).unwrap();
        for (name, rev) in [("ours", "HEAD"), ("base", "HEAD~1"), ("theirs", "topic")] {
            let text = git.run(["show", &format!("{rev}:long.txt")]).unwrap();
            std::fs::write(dir.join(name), text).unwrap();
        }
        let expected = git
            .run([
                "merge-file",
                "-p",
                flag,
                "tmp-merge/ours",
                "tmp-merge/base",
                "tmp-merge/theirs",
            ])
            .unwrap();
        assert_eq!(rebuilt(&detail, take), expected, "{take}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
    // Nothing was left behind in the git directory.
    let leftovers: Vec<_> = std::fs::read_dir(scratch.path().join(".git/bitbraid"))
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("merge-"))
        .collect();
    assert!(leftovers.is_empty());

    // Saving a result resolves the file; one with markers left is refused.
    let half = rebuilt(&detail, "ours") + "<<<<<<< HEAD\nforgotten\n>>>>>>> topic\n";
    let err = run(
        "conflict.write",
        json!({ "path": "long.txt", "text": half }),
    )
    .unwrap_err()
    .to_string();
    assert!(err.contains("conflict markers"), "{err}");
    assert!(
        git.run(["status", "--porcelain"])
            .unwrap()
            .starts_with("UU long.txt")
    );
    // Their side has a line that only looks like a marker: the check
    // cannot tell, so saving it has to be insisted on.
    let result = rebuilt(&detail, "theirs");
    assert!(
        run(
            "conflict.write",
            json!({ "path": "long.txt", "text": result })
        )
        .is_err()
    );
    run(
        "conflict.write",
        json!({ "path": "long.txt", "text": result, "allow_markers": true }),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(scratch.path().join("long.txt")).unwrap(),
        result
    );
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "M  long.txt\n");
    assert_eq!(run("op.continue", json!({})).unwrap()["finished"], true);
    // Resolved: there is nothing to take apart or to write any more.
    assert!(run("conflict.detail", json!({ "path": "long.txt" })).is_err());
    assert!(run("conflict.write", json!({ "path": "long.txt", "text": "x" })).is_err());
}

#[test]
fn files_that_cannot_be_taken_apart_say_so() {
    let mut h = Harness::new();
    // A picture changed on both sides, on top of the usual scenario.
    {
        let copy = &h.scratch;
        let git = copy.git();
        std::fs::write(copy.path().join("pic.bin"), [0u8, 1, 2, 3]).unwrap();
        git.run(["add", "pic.bin"]).unwrap();
        git.run(["commit", "-q", "-m", "a binary"]).unwrap();
        git.run(["switch", "-q", "topic"]).unwrap();
        git.run(["cherry-pick", "main"]).unwrap();
        std::fs::write(copy.path().join("pic.bin"), [0u8, 9, 9, 9]).unwrap();
        git.run(["commit", "-q", "-am", "topic's binary"]).unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
        std::fs::write(copy.path().join("pic.bin"), [0u8, 7, 7, 7]).unwrap();
        git.run(["commit", "-q", "-am", "main's binary"]).unwrap();
    }
    h.run("branch.merge", json!({ "target": "topic" }));
    for path in ["pic.bin", "b.txt", "README.md"] {
        let detail = h.run("conflict.detail", json!({ "path": path }));
        assert!(detail["whole_only"].is_string(), "{path}: {detail}");
        assert_eq!(detail["regions"], json!([]), "{path}");
    }
    // Both added: there is no base, and it is taken apart all the same.
    let detail = h.run("conflict.detail", json!({ "path": "new.txt" }));
    assert!(detail["whole_only"].is_null());
    assert_eq!(detail["regions"][0]["base"], json!([]));
    // The binary is resolved whole.
    h.run(
        "conflict.take",
        json!({ "path": "pic.bin", "side": "theirs" }),
    );
    assert_eq!(
        std::fs::read(h.scratch.path().join("pic.bin")).unwrap(),
        [0u8, 9, 9, 9]
    );
    for path in ["", "../x", "calm.txt", "nope"] {
        assert!(
            h.try_run("conflict.detail", json!({ "path": path }))
                .is_err(),
            "{path:?}"
        );
        assert!(
            h.try_run("conflict.write", json!({ "path": path, "text": "x" }))
                .is_err(),
            "{path:?}"
        );
    }
    assert!(
        h.try_run("conflict.write", json!({ "path": "a.txt" }))
            .is_err()
    );
    assert!(
        h.try_run("conflict.write", json!({ "path": "a.txt", "text": 5 }))
            .is_err()
    );
}

#[test]
fn squash_and_pick_without_commit_are_still_taken_back() {
    let mut h = Harness::new();
    let git = h.git();
    let head = git.rev_parse("HEAD").unwrap();
    let pick = git.rev_parse("topic~1").unwrap();
    for (command, params) in [
        (
            "branch.merge",
            json!({ "target": "topic", "mode": "squash" }),
        ),
        (
            "commit.cherry_pick",
            json!({ "commits": [pick], "no_commit": true }),
        ),
    ] {
        let err = h.try_run(command, params).unwrap_err();
        assert!(err.contains("nothing was changed"), "{command}: {err}");
        assert_eq!(git.rev_parse("HEAD").unwrap(), head);
        assert_eq!(status(&git), "");
        assert!(h.run("op.status", json!({}))["operation"].is_null());
    }
}

#[test]
fn hostile_paths_and_broken_state_are_refused_or_reported_never_a_crash() {
    let mut h = Harness::new();
    let git = h.git();
    h.run("branch.merge", json!({ "target": "topic" }));
    let before = (status(&git), git.run(["ls-files", "--stage"]).unwrap());
    let long = "p".repeat(5000);
    for path in [
        "",
        "../outside",
        "/etc/hostname",
        "calm.txt",
        "no-such-file",
        ":(top)a.txt",
        "-x",
        "a.txt\0b",
        long.as_str(),
    ] {
        for (command, params) in [
            ("conflict.take", json!({ "path": path, "side": "ours" })),
            ("conflict.take", json!({ "path": path, "side": "theirs" })),
            ("conflict.resolve", json!({ "path": path })),
            (
                "conflict.resolve",
                json!({ "path": path, "allow_markers": true }),
            ),
            ("conflict.reopen", json!({ "path": path })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted {path:?}"
            );
        }
    }
    for params in [
        json!({}),
        json!({ "path": "a.txt" }),
        json!({ "path": "a.txt", "side": "both" }),
        json!({ "path": "a.txt", "side": 2 }),
        json!({ "path": 7, "side": "ours" }),
    ] {
        assert!(
            h.try_run("conflict.take", params.clone()).is_err(),
            "{params}"
        );
    }
    assert_eq!(
        (status(&git), git.run(["ls-files", "--stage"]).unwrap()),
        before
    );
    h.run("op.abort", json!({}));

    // State files that are missing, empty or nonsense: whatever can be
    // read is reported, and going on is git's to refuse.
    let dir = h.scratch.path().join(".git");
    std::fs::write(dir.join("MERGE_HEAD"), "not an id\n").unwrap();
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "merge");
    assert!(operation["current"].is_null());
    let _ = h.try_run("op.continue", json!({}));
    let _ = h.try_run("op.abort", json!({}));
    let _ = std::fs::remove_file(dir.join("MERGE_HEAD"));
    let _ = git.run(["reset", "-q", "--hard"]);

    std::fs::create_dir_all(dir.join("rebase-merge")).unwrap();
    std::fs::write(dir.join("rebase-merge/msgnum"), "seven").unwrap();
    std::fs::write(dir.join("rebase-merge/end"), vec![0xffu8; 64]).unwrap();
    std::fs::write(dir.join("rebase-merge/head-name"), "").unwrap();
    std::fs::write(dir.join("rebase-merge/onto"), "\n\n\n").unwrap();
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "rebase");
    assert!(operation["step"].is_null());
    assert!(operation["onto"].is_null());
    for command in ["op.continue", "op.skip", "op.abort"] {
        let _ = h.try_run(command, json!({}));
    }
    let _ = std::fs::remove_dir_all(dir.join("rebase-merge"));
    // And a note that is not a note.
    std::fs::create_dir_all(dir.join("bitbraid")).unwrap();
    std::fs::write(h.note(), "{ not json").unwrap();
    assert!(git.run(["merge", "--no-edit", "topic"]).is_err());
    resolve_here(&mut h);
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
}

#[test]
fn conflicts_are_foretold_without_touching_anything() {
    let mut h = Harness::new();
    let git = h.git();
    // Two branches whose merge is clean and makes a tree that is not
    // there yet, for telling whether a trial merge wrote anything.
    for name in ["calm-one", "calm-two"] {
        git.run(["switch", "-q", "-c", name, "main~1"]).unwrap();
        h.scratch.write(&format!("{name}.txt"), name).unwrap();
        git.run(["add", "."]).unwrap();
        git.run(["commit", "-q", "-m", name]).unwrap();
    }
    git.run(["switch", "-q", "main"]).unwrap();
    // Work in the tree and in the index plays no part and is left alone.
    h.scratch.write("local.txt", "not committed\n").unwrap();
    h.scratch.write("staged.txt", "staged\n").unwrap();
    git.run(["add", "staged.txt"]).unwrap();
    let before = (
        status(&git),
        git.run(["rev-parse", "HEAD"]).unwrap(),
        git.run(["ls-files", "--stage"]).unwrap(),
    );

    let out = h.run("branch.predict", json!({ "target": "topic" }));
    assert_eq!(out["known"], true);
    let mut foretold: Vec<String> = out["conflicts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|path| path.as_str().unwrap().to_owned())
        .collect();
    foretold.sort();
    // What git leaves unmerged when the merge is really done.
    let twin = h.twin.git();
    assert!(twin.run(["merge", "topic"]).is_err());
    let mut unmerged = twin
        .lines(["diff", "--name-only", "--diff-filter=U"])
        .unwrap();
    unmerged.sort();
    assert_eq!(foretold, unmerged);
    assert_eq!(foretold, ["README.md", "a.txt", "b.txt", "new.txt"]);

    // Looking ahead writes nothing under `.git`, not even the objects of
    // the merge it tries, and says the same.
    let everything = |dir: &std::path::Path| -> Vec<String> {
        let mut found = Vec::new();
        let mut stack = vec![dir.to_path_buf()];
        while let Some(at) = stack.pop() {
            for entry in std::fs::read_dir(&at).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let meta = entry.metadata().unwrap();
                    found.push(format!("{} {}", path.display(), meta.len()));
                }
            }
        }
        found.sort();
        found
    };
    let git_dir = h.scratch.path().join(".git");
    // The activity log of the run is the application's and not under it.
    let under_git = everything(&git_dir);
    let aside = h.run(
        "branch.predict",
        json!({ "target": "topic", "aside": true }),
    );
    assert_eq!(aside, out);
    for target in ["HEAD", "main~1", "calm-one"] {
        let aside = h.run("branch.predict", json!({ "target": target, "aside": true }));
        assert_eq!(aside, json!({ "known": true, "conflicts": [] }), "{target}");
    }
    assert_eq!(everything(&git_dir), under_git, "something was written");
    // Asked the plain way, the merged tree is written: the look above
    // would have seen it.
    h.run("branch.predict", json!({ "target": "calm-two" }));
    assert_ne!(everything(&git_dir), under_git);

    // A commit id and a tag do as well as a branch name.
    let tip = git.run(["rev-parse", "topic"]).unwrap();
    let by_id = h.run("branch.predict", json!({ "target": tip.trim() }));
    assert_eq!(by_id["conflicts"], out["conflicts"]);

    // What merges cleanly, what is merged already and the commit itself.
    git.run(["branch", "calm", "main~1"]).unwrap();
    for target in ["calm", "HEAD", "main~1"] {
        let out = h.run("branch.predict", json!({ "target": target }));
        assert_eq!(out, json!({ "known": true, "conflicts": [] }), "{target}");
    }

    // Histories with nothing in common: there is no telling.
    git.run(["switch", "-q", "--orphan", "island"]).unwrap();
    git.run(["rm", "-rfq", "--cached", "."]).unwrap();
    h.scratch.write("a.txt", "from nowhere\n").unwrap();
    git.run(["add", "a.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "island"]).unwrap();
    git.run(["switch", "-q", "-f", "main"]).unwrap();
    git.run(["add", "staged.txt"]).unwrap();
    let out = h.run("branch.predict", json!({ "target": "island" }));
    assert_eq!(out, json!({ "known": false, "conflicts": [] }));

    // What does not name a commit is refused, whatever it looks like.
    for target in [
        "",
        "--abort",
        "-h",
        "no-such-branch",
        "topic\nmain",
        "HEAD:a.txt",
        "topic^{tree}",
    ] {
        assert!(
            h.try_run("branch.predict", json!({ "target": target }))
                .is_err(),
            "{target:?}"
        );
    }
    assert!(h.try_run("branch.predict", json!({})).is_err());
    assert!(h.try_run("branch.predict", json!({ "target": 7 })).is_err());

    // Nothing moved in all of that.
    let after = (
        status(&git),
        git.run(["rev-parse", "HEAD"]).unwrap(),
        git.run(["ls-files", "--stage"]).unwrap(),
    );
    assert_eq!(before, after);
    assert!(h.run("op.status", json!({}))["operation"].is_null());
}

/// Between two tips of which neither is checked out, as a pull request
/// of one into the other meets them: the files are those git
/// leaves unmerged when the one is checked out and the other merged, and
/// nothing is touched, aside or not.
#[test]
fn conflicts_between_two_branches_elsewhere_are_foretold() {
    let mut h = Harness::new();
    let git = h.git();
    git.run(["branch", "base", "main"]).unwrap();
    git.run(["branch", "calm", "main~1"]).unwrap();
    git.run(["switch", "-q", "--detach", "main~1"]).unwrap();
    let before = (
        status(&git),
        git.run(["rev-parse", "HEAD"]).unwrap(),
        git.run(["ls-files", "--stage"]).unwrap(),
    );
    let out = h.run(
        "branch.predict",
        json!({ "ours": "base", "target": "topic", "aside": true }),
    );
    assert_eq!(out["known"], true);
    let twin = h.twin.git();
    twin.run(["switch", "-q", "main"]).unwrap();
    assert!(twin.run(["merge", "topic"]).is_err());
    let mut unmerged = twin
        .lines(["diff", "--name-only", "--diff-filter=U"])
        .unwrap();
    unmerged.sort();
    assert_eq!(out["conflicts"], json!(unmerged));
    assert_eq!(
        h.run(
            "branch.predict",
            json!({ "ours": "topic", "target": "base" })
        )["conflicts"],
        out["conflicts"],
        "the same either way round"
    );
    assert_eq!(
        h.run(
            "branch.predict",
            json!({ "ours": "calm", "target": "base" })
        ),
        json!({ "known": true, "conflicts": [] })
    );
    for ours in ["", "-h", "no-such-branch", "topic\nmain"] {
        assert!(
            h.try_run("branch.predict", json!({ "ours": ours, "target": "topic" }))
                .is_err(),
            "{ours:?}"
        );
    }
    assert_eq!(
        before,
        (
            status(&git),
            git.run(["rev-parse", "HEAD"]).unwrap(),
            git.run(["ls-files", "--stage"]).unwrap(),
        )
    );
}

#[test]
fn two_renames_of_one_file_are_three_paths_to_settle() {
    let mut h = Harness::new();
    // On top of the usual scenario: each side gives `later.txt`'s
    // neighbour another name.
    for copy in [&h.scratch, &h.twin] {
        let git = copy.git();
        copy.write("moved.txt", "a file to rename\n").unwrap();
        git.run(["add", "moved.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "a file to rename"]).unwrap();
        git.run(["switch", "-q", "-c", "left"]).unwrap();
        git.run(["mv", "moved.txt", "left-name.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "left: rename"]).unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
        git.run(["mv", "moved.txt", "main-name.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "main: rename"]).unwrap();
    }
    let out = h.run("branch.merge", json!({ "target": "left" }));
    assert!(out["stopped"].is_string(), "{out}");
    assert!(h.twin.git().run(["merge", "--no-edit", "left"]).is_err());
    h.assert_same("stopped on the renames");
    assert_eq!(
        h.kinds(),
        [
            ("left-name.txt".to_owned(), "added_by_them".to_owned()),
            ("main-name.txt".to_owned(), "added_by_us".to_owned()),
            ("moved.txt".to_owned(), "both_deleted".to_owned()),
        ]
    );
    // None of the three can be taken apart line by line.
    for path in ["left-name.txt", "main-name.txt", "moved.txt"] {
        let detail = h.run("conflict.detail", json!({ "path": path }));
        assert!(detail["whole_only"].is_string(), "{path}");
    }
    // Our name is kept: the old name and their name go.
    for path in ["moved.txt", "main-name.txt", "left-name.txt"] {
        h.run("conflict.take", json!({ "path": path, "side": "ours" }));
    }
    let twin = h.twin.git();
    twin.run(["rm", "-q", "moved.txt"]).unwrap();
    twin.run(["add", "main-name.txt"]).unwrap();
    twin.run(["rm", "-q", "-f", "left-name.txt"]).unwrap();
    h.assert_same("renames settled");
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    twin.run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("merge of the renames");
    assert!(h.scratch.path().join("main-name.txt").is_file());
    assert!(!h.scratch.path().join("left-name.txt").exists());
}

#[test]
fn a_submodule_in_conflict_is_settled_by_choosing_a_commit() {
    let mut h = Harness::new();
    // A repository to be the submodule, with two commits that diverge.
    let source = Scratch::of("linear").unwrap();
    let src = source.git();
    src.run(["config", "user.name", "Ada Fixture"]).unwrap();
    src.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    let tip = |branch: &str| {
        src.run(["switch", "-q", "-c", branch, "main"]).unwrap();
        source.write("side.txt", branch).unwrap();
        src.run(["add", "side.txt"]).unwrap();
        src.run(["commit", "-q", "-m", branch]).unwrap();
        src.rev_parse("HEAD").unwrap()
    };
    let (theirs, ours) = (tip("one"), tip("two"));
    // What a clone checks out: neither of the two.
    src.run(["switch", "-q", "main"]).unwrap();
    let url = source.path().display().to_string();

    for copy in [&h.scratch, &h.twin] {
        let git = copy.git();
        git.run([
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "--quiet",
            "add",
            url.as_str(),
            "sub",
        ])
        .unwrap();
        git.run(["commit", "-q", "-m", "add the submodule"])
            .unwrap();
        let sub = Git::new(copy.path().join("sub"), git.home());
        let point = |branch: &str, at: &str| {
            git.run(["switch", "-q", branch]).unwrap();
            sub.run(["checkout", "-q", at]).unwrap();
            git.run(["add", "sub"]).unwrap();
            git.run(["commit", "-q", "-m", "move the submodule"])
                .unwrap();
        };
        git.run(["branch", "pointer"]).unwrap();
        point("pointer", &theirs);
        point("main", &ours);
    }
    let out = h.run("branch.merge", json!({ "target": "pointer" }));
    assert_eq!(out["conflicts"], json!(["sub"]), "{out}");
    assert!(h.twin.git().run(["merge", "--no-edit", "pointer"]).is_err());
    let operation = h.run("op.status", json!({}))["operation"].clone();
    let conflict = &operation["conflicts"][0];
    assert_eq!(conflict["path"], "sub");
    assert_eq!(conflict["submodule"], true);
    assert_eq!(conflict["ours"].as_str().unwrap(), ours);
    assert_eq!(conflict["theirs"].as_str().unwrap(), theirs);
    let detail = h.run("conflict.detail", json!({ "path": "sub" }));
    assert!(
        detail["whole_only"].as_str().unwrap().contains("submodule"),
        "{detail}"
    );
    // There is no text to write for it.
    assert!(
        h.try_run("conflict.write", json!({ "path": "sub", "text": "x" }))
            .is_err()
    );

    h.run("conflict.take", json!({ "path": "sub", "side": "theirs" }));
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    // By hand: move the submodule there and record it.
    let twin = h.twin.git();
    Git::new(h.twin.path().join("sub"), twin.home())
        .run(["checkout", "-q", theirs.as_str()])
        .unwrap();
    twin.run(["add", "sub"]).unwrap();
    twin.run(["commit", "-q", "--no-edit"]).unwrap();
    assert_eq!(shape(&h.git()), shape(&twin), "history");
    assert_eq!(
        h.git().run(["ls-files", "--stage"]).unwrap(),
        twin.run(["ls-files", "--stage"]).unwrap(),
        "index"
    );
    assert_eq!(
        h.git().run(["rev-parse", "HEAD:sub"]).unwrap().trim(),
        theirs
    );
    // The submodule's own checkout is left where it was.
    assert_eq!(
        Git::new(h.scratch.path().join("sub"), h.git().home())
            .rev_parse("HEAD")
            .unwrap(),
        ours
    );
}

/// What `op.status` says of the operation: whether continuing would go
/// through as things are.
fn ready(h: &mut Harness) -> bool {
    let operation = h.run("op.status", json!({}))["operation"].clone();
    operation["ready"]
        .as_bool()
        .unwrap_or_else(|| panic!("{operation}"))
}

/// Every conflict settled with one side.
fn take_all(h: &mut Harness, side: &str) {
    let conflicted: Vec<String> = h.kinds().into_iter().map(|(path, _)| path).collect();
    assert!(!conflicted.is_empty());
    for path in &conflicted {
        h.run("conflict.take", json!({ "path": path, "side": side }));
    }
    assert!(h.kinds().is_empty());
}

#[test]
fn a_stopped_operation_says_when_it_is_ready_to_go_on() {
    // A merge: not while files are in conflict; then whatever is staged,
    // even nothing, makes the merge commit.
    let mut h = Harness::new();
    h.run("branch.merge", json!({ "target": "topic" }));
    assert!(!ready(&mut h), "in conflict");
    take_all(&mut h, "ours");
    assert!(
        ready(&mut h),
        "a merge resolved to our side is still a merge"
    );
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);

    // A pick makes a commit of what is staged: resolved to their side
    // something is, and it goes on.
    let mut h = Harness::new();
    let pick = h.git().rev_parse("topic~1").unwrap();
    h.run("commit.cherry_pick", json!({ "commits": [pick] }));
    assert!(!ready(&mut h), "in conflict");
    take_all(&mut h, "theirs");
    assert!(ready(&mut h), "the pick's changes are staged");
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);

    // Resolved to our side nothing is staged, and git refuses to go on:
    // the state says so before it is tried. `calm.txt` came in without a
    // conflict and is taken out again, so that nothing is left.
    let mut h = Harness::new();
    h.run("commit.cherry_pick", json!({ "commits": [pick] }));
    take_all(&mut h, "ours");
    assert!(ready(&mut h), "calm.txt is still staged");
    h.git().run(["rm", "-q", "-f", "--", "calm.txt"]).unwrap();
    assert_eq!(status(&h.git()), "");
    assert!(!ready(&mut h), "nothing is staged");
    assert!(h.try_run("op.continue", json!({})).is_err());
    assert!(!ready(&mut h), "still there, still with nothing to commit");
    // Something staged for it, and it is ready and goes through.
    h.scratch.write("calm.txt", "after all\n").unwrap();
    h.git().run(["add", "calm.txt"]).unwrap();
    assert!(ready(&mut h));
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);

    // A revert likewise.
    let mut h = Harness::new();
    h.scratch.write("a.txt", "edited again\n").unwrap();
    h.git()
        .run(["commit", "-q", "-am", "main: a again"])
        .unwrap();
    h.run("commit.revert", json!({ "commits": ["HEAD~1"] }));
    assert!(!ready(&mut h), "in conflict");
    take_all(&mut h, "ours");
    let nothing_staged = h.git().run(["diff", "--cached", "--quiet"]).is_ok();
    assert_eq!(ready(&mut h), !nothing_staged);
    assert_eq!(
        h.try_run("op.continue", json!({})).is_ok(),
        !nothing_staged,
        "what the state said is what git does"
    );

    // A rebase that replays commits goes on with nothing staged: the
    // commit is left out.
    let mut h = Harness::new();
    h.git().run(["switch", "-q", "topic"]).unwrap();
    h.run("branch.rebase", json!({ "onto": "main" }));
    assert!(!ready(&mut h), "in conflict");
    take_all(&mut h, "ours");
    h.git().run(["rm", "-q", "-f", "--", "calm.txt"]).unwrap();
    assert_eq!(status(&h.git()), "");
    assert!(ready(&mut h), "a rebase goes on without the commit");
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);

    // One begun in a shell that applies patches does not, like a mailbox.
    let mut h = Harness::new();
    let git = h.git();
    git.run(["switch", "-q", "topic"]).unwrap();
    assert!(git.run(["rebase", "--apply", "main"]).is_err());
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "rebase", "{operation}");
    assert!(!ready(&mut h), "in conflict");
    take_all(&mut h, "ours");
    // A rebase that applies patches leaves what did not conflict to the
    // patch: nothing of it is staged.
    if git.run(["diff", "--cached", "--quiet"]).is_ok() {
        assert!(!ready(&mut h), "nothing is staged");
        assert!(h.try_run("op.continue", json!({})).is_err());
        h.scratch.write("calm.txt", "by hand\n").unwrap();
        git.run(["add", "calm.txt"]).unwrap();
    }
    assert!(ready(&mut h), "something is staged");
    let done = h.run("op.continue", json!({}));
    assert_ne!(done["operation"]["step"], operation["step"], "{done}");
}
