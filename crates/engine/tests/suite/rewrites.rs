//! Merge, rebase, reset, cherry-pick and revert against git on twin scratch
//! copies. New commits carry today's date on one side and the fixture's on
//! the other, so where ids cannot match, trees, parents and messages do.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// The first-parent history of HEAD as (tree, number of parents, subject),
/// which does not depend on dates.
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

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        let twin = Scratch::of(fixture).unwrap();
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
    }
}

/// A branch `topic` with one commit, and one more commit on main, on both
/// copies: the two lines have diverged without touching the same file.
fn diverge(copy: &Scratch) {
    let git = copy.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("topic.txt", "topic\n").unwrap();
    git.run(["add", "topic.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "on topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write("main.txt", "main\n").unwrap();
    git.run(["add", "main.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "on main"]).unwrap();
}

#[test]
fn merge_modes_match_git_merge_and_undo() {
    let mut h = Harness::new("linear");
    h.both(diverge);
    let start = h.git().rev_parse("HEAD").unwrap();

    // Diverged: fast-forward only refuses, nothing moves.
    assert!(
        h.try_run(
            "branch.merge",
            json!({ "target": "topic", "mode": "ff_only" })
        )
        .is_err()
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);

    h.run("branch.merge", json!({ "target": "topic" }));
    h.twin
        .git()
        .run(["merge", "-q", "--no-edit", "topic"])
        .unwrap();
    h.assert_same("merge");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'topic'\n",
        "git's own wording"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Merge topic");
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    h.run("redo.last", json!({}));
    h.assert_same("redo of merge");
    h.run("undo.last", json!({}));
    h.twin
        .git()
        .run(["reset", "-q", "--hard", "HEAD~1"])
        .unwrap();
    h.assert_same("undo of merge");

    // Squash: the changes staged, HEAD where it was, and undo takes them
    // back (held to git in `undo_files.rs`).
    h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "squash" }),
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert!(status(&h.git()).contains("topic.txt"));
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Squash-merge topic");
    h.run("undo.last", json!({}));
    assert_eq!(status(&h.git()), "");

    // Fast-forward: from topic's base, main is simply ahead.
    h.both(|copy| {
        copy.git().run(["switch", "-q", "topic"]).unwrap();
        copy.git().run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    });
    h.run(
        "branch.merge",
        json!({ "target": "main", "mode": "ff_only" }),
    );
    h.twin
        .git()
        .run(["merge", "-q", "--ff-only", "main"])
        .unwrap();
    h.assert_same("fast-forward merge");
    assert_eq!(
        h.git().rev_parse("HEAD").unwrap(),
        h.git().rev_parse("main").unwrap()
    );
}

#[test]
fn rebase_matches_git_rebase_and_undoes() {
    let mut h = Harness::new("linear");
    h.both(diverge);
    h.both(|copy| copy.git().run(["switch", "-q", "topic"]).unwrap().clear());
    let start = h.git().rev_parse("HEAD").unwrap();
    h.run("branch.rebase", json!({ "onto": "main" }));
    h.twin.git().run(["rebase", "-q", "main"]).unwrap();
    h.assert_same("rebase");
    assert_eq!(
        h.git().rev_parse("HEAD~1").unwrap(),
        h.git().rev_parse("main").unwrap()
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    h.run("redo.last", json!({}));
    h.assert_same("redo of rebase");

    // Detached, there is no branch to rebase.
    h.git().run(["switch", "-q", "--detach"]).unwrap();
    assert!(
        h.try_run("branch.rebase", json!({ "onto": "main" }))
            .is_err()
    );
}

#[test]
fn reset_modes_match_git_reset_and_undo_brings_everything_back() {
    for mode in ["soft", "mixed", "hard"] {
        let mut h = Harness::new("linear");
        h.both(|copy| {
            copy.write("a.txt", "local edit\n").unwrap();
            copy.write("new.txt", "untracked\n").unwrap();
        });
        let start = (h.git().rev_parse("HEAD").unwrap(), status(&h.git()));
        h.run("branch.reset", json!({ "target": "HEAD~2", "mode": mode }));
        h.twin
            .git()
            .run(["reset", "-q", &format!("--{mode}"), "HEAD~2"])
            .unwrap();
        h.assert_same(&format!("reset --{mode}"));
        assert!(
            h.run("undo.peek", json!({}))["undo"]
                .as_str()
                .unwrap()
                .ends_with(&format!("({mode})"))
        );

        h.run("undo.last", json!({}));
        assert_eq!(h.git().rev_parse("HEAD").unwrap(), start.0, "{mode}");
        assert_eq!(
            std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
            "local edit\n",
            "{mode}: the edit is back"
        );
        if mode != "soft" {
            // A soft reset stages what the dropped commits changed; after
            // the others the status is what it was.
            assert_eq!(status(&h.git()), start.1, "{mode}");
        }
        h.run("redo.last", json!({}));
        h.assert_same(&format!("redo of reset --{mode}"));
        h.run("undo.last", json!({}));
        assert_eq!(h.git().rev_parse("HEAD").unwrap(), start.0);
        assert_eq!(
            std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
            "local edit\n"
        );
    }
}

#[test]
fn cherry_pick_and_revert_match_git() {
    let mut h = Harness::new("linear");
    h.both(diverge);
    let start = h.git().rev_parse("HEAD").unwrap();
    let topic = h.git().rev_parse("topic").unwrap();

    h.run("commit.cherry_pick", json!({ "commits": [topic] }));
    h.twin.git().run(["cherry-pick", "topic"]).unwrap();
    h.assert_same("cherry-pick");
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Cherry-pick 1 commit"
    );

    // Revert what was just picked, then two older commits at once.
    h.run("commit.revert", json!({ "commits": ["HEAD"] }));
    h.twin.git().run(["revert", "--no-edit", "HEAD"]).unwrap();
    h.assert_same("revert");
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);

    // Picked without committing: staged, HEAD stays.
    h.run(
        "commit.cherry_pick",
        json!({ "commits": [topic], "no_commit": true }),
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert!(status(&h.git()).contains("topic.txt"));
    h.git().run(["reset", "-q", "--hard"]).unwrap();

    // A merge commit needs to be told which parent to keep.
    let mut h = Harness::new("branches-merges");
    assert!(
        h.try_run("commit.revert", json!({ "commits": ["HEAD"] }))
            .is_err()
    );
    assert_eq!(status(&h.git()), "");
    h.run(
        "commit.revert",
        json!({ "commits": ["HEAD"], "mainline": 1 }),
    );
    h.twin
        .git()
        .run(["revert", "--no-edit", "--mainline", "1", "HEAD"])
        .unwrap();
    h.assert_same("revert of a merge");

    // The same for picking one: onto its first parent's parent, the merge
    // brings in what the other side did.
    let merge = h.git().rev_parse("HEAD~1").unwrap();
    h.both(|copy| {
        copy.git()
            .run(["switch", "-q", "--detach", "HEAD~1^1"])
            .unwrap();
    });
    assert!(
        h.try_run("commit.cherry_pick", json!({ "commits": [merge] }))
            .is_err()
    );
    assert_eq!(status(&h.git()), "");
    h.run(
        "commit.cherry_pick",
        json!({ "commits": [merge], "mainline": 1 }),
    );
    h.twin
        .git()
        .run(["cherry-pick", "--mainline", "1", &merge])
        .unwrap();
    h.assert_same("cherry-pick of a merge");
}

#[test]
fn a_merge_without_commit_stops_before_the_commit_like_git() {
    let mut h = Harness::new("linear");
    h.both(diverge);
    let start = h.git().rev_parse("HEAD").unwrap();
    let index = |git: &Git| git.run(["ls-files", "-s"]).unwrap();
    let merge_head = |git: &Git| git.run(["rev-parse", "--verify", "-q", "MERGE_HEAD"]);

    let before = h.run("undo.peek", json!({}));
    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_commit" }),
    );
    h.twin
        .git()
        .run(["merge", "-q", "--no-ff", "--no-commit", "topic"])
        .unwrap();
    assert!(
        out["stopped"]
            .as_str()
            .unwrap()
            .contains("stopped before the commit"),
        "{out}"
    );
    assert!(out["conflicts"].as_array().unwrap().is_empty());
    // Nothing is committed: HEAD stays, the merge is in progress, and
    // index and files hold what was merged, as with git.
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    h.assert_same("merge without commit");
    assert_eq!(index(&h.git()), index(&h.twin.git()));
    assert_eq!(
        merge_head(&h.git()).unwrap(),
        merge_head(&h.twin.git()).unwrap()
    );
    let operation = h.run("op.status", json!({}));
    assert_eq!(operation["operation"]["kind"], "merge");
    assert_eq!(h.run("undo.peek", json!({})), before, "not journalled yet");

    // Continuing makes the merge commit, and the whole is one step to undo.
    h.run("op.continue", json!({}));
    h.twin.git().run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("the merge commit");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'topic'\n"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Merge topic");
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");

    // Begun again and aborted: as it was.
    h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_commit" }),
    );
    h.run("op.abort", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    assert!(merge_head(&h.git()).is_err());

    // Where the branch could simply move forward it stops as well: there
    // would be nothing to look at otherwise.
    h.twin.git().run(["reset", "-q", "--hard", &start]).unwrap();
    h.both(|copy| {
        copy.git().run(["switch", "-q", "topic"]).unwrap();
        copy.git().run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    });
    let base = h.git().rev_parse("HEAD").unwrap();
    let out = h.run(
        "branch.merge",
        json!({ "target": "main", "mode": "no_commit" }),
    );
    h.twin
        .git()
        .run(["merge", "-q", "--no-ff", "--no-commit", "main"])
        .unwrap();
    assert!(out["stopped"].is_string(), "{out}");
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), base);
    h.assert_same("merge without commit where a fast-forward would do");
    assert_eq!(index(&h.git()), index(&h.twin.git()));
    h.run("op.abort", json!({}));

    // With nothing to merge, nothing is left in progress.
    let out = h.run(
        "branch.merge",
        json!({ "target": "HEAD~1", "mode": "no_commit" }),
    );
    assert!(out.get("stopped").is_none(), "{out}");
    assert!(merge_head(&h.git()).is_err());
    assert_eq!(h.run("op.status", json!({}))["operation"], Value::Null);
}

#[test]
fn a_range_is_rebased_onto_another_base_and_local_changes_are_set_aside() {
    let mut h = Harness::new("linear");
    h.both(|copy| {
        diverge(copy);
        let git = copy.git();
        // Whatever a global configuration says about it.
        git.run(["config", "rebase.autoStash", "false"]).unwrap();
        git.run(["switch", "-q", "topic"]).unwrap();
        for name in ["two", "three"] {
            copy.write(&format!("{name}.txt"), name).unwrap();
            git.run(["add", "."]).unwrap();
            git.run(["commit", "-q", "-m", &format!("topic {name}")])
                .unwrap();
        }
    });
    let start = h.git().rev_parse("HEAD").unwrap();
    let subjects = |git: &Git| git.lines(["log", "--format=%s", "main..HEAD"]).unwrap();

    // Only the commits after the first one of the branch go onto main.
    h.run(
        "branch.rebase",
        json!({ "onto": "main", "after": "topic~2" }),
    );
    h.twin
        .git()
        .run(["rebase", "-q", "--onto", "main", "topic~2"])
        .unwrap();
    h.assert_same("rebase of a range");
    assert_eq!(subjects(&h.git()), ["topic three", "topic two"]);
    assert_eq!(
        h.git().rev_parse("HEAD~2").unwrap(),
        h.git().rev_parse("main").unwrap()
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Rebase onto main");
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    h.twin.git().run(["reset", "-q", "--hard", &start]).unwrap();

    // A start that is not in the branch's history is refused before
    // anything moves: main has a commit topic does not have.
    let err = h
        .try_run("branch.rebase", json!({ "onto": "main", "after": "main" }))
        .unwrap_err();
    assert!(err.contains("not in the history"), "{err}");
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);

    // A local change: git refuses to rebase over it, and with autostash
    // sets it aside and brings it back.
    h.both(|copy| copy.write("a.txt", "local edit\n").unwrap());
    assert!(
        h.try_run("branch.rebase", json!({ "onto": "main" }))
            .is_err()
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), status(&h.twin.git()));
    let out = h.run(
        "branch.rebase",
        json!({ "onto": "main", "autostash": true }),
    );
    h.twin
        .git()
        .run(["rebase", "-q", "--autostash", "main"])
        .unwrap();
    h.assert_same("rebase with autostash");
    assert!(out.get("stash_kept").is_none(), "{out}");
    assert_eq!(status(&h.git()), " M a.txt\n");
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "local edit\n"
    );
    assert!(h.git().lines(["stash", "list"]).unwrap().is_empty());
    // Undo moves the branch back and leaves the change where it is.
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), " M a.txt\n");

    // A change that does not fit on the new base stays in the stash, as
    // git leaves it, and the answer says so.
    h.twin.git().run(["reset", "-q", "--keep", &start]).unwrap();
    h.both(|copy| {
        let git = copy.git();
        git.run(["stash", "-q"]).unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
        copy.write("a.txt", "changed on main\n").unwrap();
        git.run(["commit", "-q", "-am", "main changes a"]).unwrap();
        git.run(["switch", "-q", "topic"]).unwrap();
        git.run(["stash", "pop", "-q"]).unwrap();
    });
    let out = h.run(
        "branch.rebase",
        json!({ "onto": "main", "autostash": true }),
    );
    let _ = h.twin.git().run(["rebase", "-q", "--autostash", "main"]);
    assert_eq!(out["stash_kept"], true, "{out}");
    h.assert_same("rebase whose autostash did not fit");
    assert_eq!(
        h.git().lines(["stash", "list"]).unwrap().len(),
        h.twin.git().lines(["stash", "list"]).unwrap().len()
    );
    assert_eq!(h.git().lines(["stash", "list"]).unwrap().len(), 1);
}

#[test]
fn a_merge_commit_is_picked_and_reverted_against_the_parent_that_is_named() {
    // HEAD merges feature/b (its second parent) into main.
    let mut h = Harness::new("branches-merges");
    let merge = h.git().rev_parse("HEAD").unwrap();
    h.run(
        "commit.revert",
        json!({ "commits": ["HEAD"], "mainline": 2 }),
    );
    h.twin
        .git()
        .run(["revert", "--no-edit", "--mainline", "2", "HEAD"])
        .unwrap();
    h.assert_same("revert of a merge against its second parent");
    // What is left is the second parent's tree: the other line is undone.
    assert_eq!(
        h.git().rev_parse("HEAD^{tree}").unwrap(),
        h.git().rev_parse(&format!("{merge}^2^{{tree}}")).unwrap()
    );
    h.run("undo.last", json!({}));
    h.twin
        .git()
        .run(["reset", "-q", "--hard", "HEAD~1"])
        .unwrap();
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), merge);

    // Picked onto its second parent, against that parent: what the first
    // parent's line brought.
    h.both(|copy| {
        copy.git()
            .run(["switch", "-q", "--detach", "HEAD^2"])
            .unwrap();
    });
    h.run(
        "commit.cherry_pick",
        json!({ "commits": [merge], "mainline": 2 }),
    );
    h.twin
        .git()
        .run(["cherry-pick", "--mainline", "2", &merge])
        .unwrap();
    h.assert_same("cherry-pick of a merge against its second parent");
    h.run("undo.last", json!({}));
    h.twin
        .git()
        .run(["reset", "-q", "--hard", "HEAD~1"])
        .unwrap();

    // The same without a commit: staged, HEAD where it was, like git.
    let at = h.git().rev_parse("HEAD").unwrap();
    h.run(
        "commit.cherry_pick",
        json!({ "commits": [merge], "mainline": 2, "no_commit": true }),
    );
    h.twin
        .git()
        .run(["cherry-pick", "--no-commit", "--mainline", "2", &merge])
        .unwrap();
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), at);
    h.assert_same("cherry-pick of a merge without commit");
    assert_eq!(
        h.git().run(["ls-files", "-s"]).unwrap(),
        h.twin.git().run(["ls-files", "-s"]).unwrap()
    );
    assert!(status(&h.git()).starts_with("A  "), "{}", status(&h.git()));
    h.both(|copy| copy.git().run(["reset", "-q", "--hard"]).unwrap().clear());

    // A parent the commit does not have: refused, nothing changed.
    for command in ["commit.cherry_pick", "commit.revert"] {
        assert!(
            h.try_run(command, json!({ "commits": [merge], "mainline": 3 }))
                .is_err()
        );
        assert_eq!(status(&h.git()), "");
        assert_eq!(h.git().rev_parse("HEAD").unwrap(), at);
    }
}

/// Everything under the worktree and the index, as a comparable string.
fn everything(h: &Harness) -> String {
    let git = h.git();
    format!(
        "{}\n{}\n{}",
        git.run(["status", "--porcelain=v2", "--branch"]).unwrap(),
        git.run(["ls-files", "--stage"]).unwrap(),
        git.run(["diff"]).unwrap()
    )
}

#[test]
fn nothing_starts_on_top_of_an_unfinished_operation() {
    let mut h = Harness::new("linear");
    let git = h.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "topic version\n").unwrap();
    git.run(["commit", "-q", "-am", "topic edits a"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "main version\n").unwrap();
    git.run(["commit", "-q", "-am", "main edits a"]).unwrap();
    let topic = git.rev_parse("topic").unwrap();

    // The user's own merge, stopped on a conflict and half resolved.
    assert!(git.run(["merge", "topic"]).is_err());
    h.scratch.write("a.txt", "resolved by hand\n").unwrap();
    let merge_head = h.scratch.path().join(".git/MERGE_HEAD");
    assert!(merge_head.exists());
    let before = everything(&h);

    for (command, params) in [
        ("branch.merge", json!({ "target": "topic" })),
        (
            "branch.merge",
            json!({ "target": "topic", "mode": "squash" }),
        ),
        ("branch.rebase", json!({ "onto": "topic" })),
        (
            "branch.reset",
            json!({ "target": "HEAD~1", "mode": "hard" }),
        ),
        (
            "branch.reset",
            json!({ "target": "HEAD~1", "mode": "soft" }),
        ),
        ("commit.cherry_pick", json!({ "commits": [topic] })),
        (
            "commit.cherry_pick",
            json!({ "commits": [topic], "no_commit": true }),
        ),
        ("commit.revert", json!({ "commits": ["HEAD"] })),
    ] {
        let err = h.try_run(command, params.clone()).unwrap_err();
        assert!(err.contains("a merge is in progress"), "{command}: {err}");
        assert!(merge_head.exists(), "{command} {params} ended the merge");
        assert_eq!(everything(&h), before, "{command} {params}");
    }
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "resolved by hand\n"
    );

    // Conflicts with no operation behind them (a stash that did not apply)
    // are no place to start either.
    git.run(["merge", "--abort"]).unwrap();
    h.scratch.write("a.txt", "local\n").unwrap();
    git.run(["stash", "-q"]).unwrap();
    h.scratch.write("a.txt", "other\n").unwrap();
    git.run(["commit", "-q", "-am", "third edit"]).unwrap();
    assert!(git.run(["stash", "pop"]).is_err());
    let before = everything(&h);
    let err = h
        .try_run(
            "branch.reset",
            json!({ "target": "HEAD~1", "mode": "hard" }),
        )
        .unwrap_err();
    assert!(err.contains("unresolved conflicts"), "{err}");
    assert_eq!(everything(&h), before);
}

#[test]
fn staged_work_survives_a_squash_or_pick_that_cannot_run() {
    let mut h = Harness::new("linear");
    let git = h.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "topic version\n").unwrap();
    git.run(["commit", "-q", "-am", "topic edits a"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    // Diverged, so the merge is a real one: a fast-forward carries staged
    // work along, which is fine and is git's business.
    h.scratch.write("main.txt", "main\n").unwrap();
    git.run(["add", "main.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "on main"]).unwrap();
    let topic = git.rev_parse("topic").unwrap();

    // Staged work in the very file the other side changes, and in another.
    for path in ["a.txt", "b.txt"] {
        h.scratch.write(path, "staged work\n").unwrap();
        git.run(["add", path]).unwrap();
        let before = everything(&h);
        for (command, params) in [
            (
                "branch.merge",
                json!({ "target": "topic", "mode": "squash" }),
            ),
            (
                "commit.cherry_pick",
                json!({ "commits": [topic], "no_commit": true }),
            ),
            ("branch.merge", json!({ "target": "topic" })),
            ("commit.cherry_pick", json!({ "commits": [topic] })),
        ] {
            assert!(
                h.try_run(command, params.clone()).is_err(),
                "{command} {params} ran over staged work in {path}"
            );
            assert_eq!(everything(&h), before, "{command} {params} with {path}");
        }
        git.run(["reset", "-q", "--hard"]).unwrap();
    }

    // Unstaged work in another file rides along, as with git.
    h.scratch.write("b.txt", "unstaged work\n").unwrap();
    h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "squash" }),
    );
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("b.txt")).unwrap(),
        "unstaged work\n"
    );
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "topic version\n"
    );
}

#[test]
fn a_hard_reset_gives_back_the_untracked_file_it_overwrote() {
    let mut h = Harness::new("linear");
    let git = h.git();
    // A later commit adds new.txt and dir/inner.txt; go back before it and
    // put untracked files where they will land.
    h.scratch.write("new.txt", "committed\n").unwrap();
    h.scratch.write("dir/inner.txt", "committed\n").unwrap();
    git.run(["add", "."]).unwrap();
    git.run(["commit", "-q", "-m", "add new"]).unwrap();
    let tip = git.rev_parse("HEAD").unwrap();
    git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    h.scratch
        .write("new.txt", "mine, never committed\n")
        .unwrap();
    h.scratch
        .write("dir", "a file where a directory goes\n")
        .unwrap();
    h.scratch.write("other.txt", "bystander\n").unwrap();
    let start = git.rev_parse("HEAD").unwrap();

    h.run("branch.reset", json!({ "target": tip, "mode": "hard" }));
    assert_eq!(git.rev_parse("HEAD").unwrap(), tip);
    assert_eq!(status(&git), "?? other.txt\n");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    let root = h.scratch.path().to_path_buf();
    let read = |name: &str| std::fs::read_to_string(root.join(name)).unwrap();
    assert_eq!(read("new.txt"), "mine, never committed\n");
    assert_eq!(read("dir"), "a file where a directory goes\n");
    assert_eq!(read("other.txt"), "bystander\n");

    // And again through redo and undo.
    h.run("redo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), tip);
    assert_eq!(read("new.txt"), "committed\n");
    h.run("undo.last", json!({}));
    assert_eq!(read("new.txt"), "mine, never committed\n");
    assert_eq!(read("dir"), "a file where a directory goes\n");
}

#[test]
fn undo_refuses_rather_than_lose_work() {
    let mut h = Harness::new("linear");
    h.both(diverge);
    let git = h.git();
    let start = git.rev_parse("HEAD").unwrap();

    // HEAD moved since: a commit made in a shell after the merge.
    h.run("branch.merge", json!({ "target": "topic" }));
    let merged = git.rev_parse("HEAD").unwrap();
    h.scratch.write("later.txt", "later\n").unwrap();
    git.run(["add", "later.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "later"]).unwrap();
    let later = git.rev_parse("HEAD").unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("not where this action left it"), "{err}");
    assert_eq!(git.rev_parse("HEAD").unwrap(), later);
    git.run(["reset", "-q", "--hard", &merged]).unwrap();

    // A local change to a file the merge brought in is in the way.
    h.scratch.write("topic.txt", "edited since\n").unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("topic.txt"), "{err}");
    assert_eq!(git.rev_parse("HEAD").unwrap(), merged);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("topic.txt")).unwrap(),
        "edited since\n"
    );
    git.run(["checkout", "-q", "--", "topic.txt"]).unwrap();

    // A change elsewhere is not, and is still there afterwards.
    h.scratch.write("a.txt", "unrelated edit\n").unwrap();
    h.scratch.write("scratch.txt", "untracked\n").unwrap();
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&git), " M a.txt\n?? scratch.txt\n");

    // Redo would write topic.txt; an untracked file of that name is in the
    // way.
    h.scratch.write("topic.txt", "untracked now\n").unwrap();
    let err = h.try_run("redo.last", json!({})).unwrap_err();
    assert!(err.contains("topic.txt"), "{err}");
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("topic.txt")).unwrap(),
        "untracked now\n"
    );
    std::fs::remove_file(h.scratch.path().join("topic.txt")).unwrap();
    h.run("redo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), merged);
    assert_eq!(status(&git), " M a.txt\n?? scratch.txt\n");
}

#[test]
fn conflicts_stop_the_operation_and_abort_leaves_the_repository_as_it_was() {
    let mut h = Harness::new("linear");
    // Both lines change a.txt differently.
    let git = h.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "topic version\n").unwrap();
    git.run(["commit", "-q", "-am", "topic edits a"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "main version\n").unwrap();
    git.run(["commit", "-q", "-am", "main edits a"]).unwrap();
    let topic = git.rev_parse("topic").unwrap();

    let untouched = |h: &Harness, head: &str, what: &str| {
        assert_eq!(h.git().rev_parse("HEAD").unwrap(), head, "{what}: HEAD");
        assert_eq!(status(&h.git()), "", "{what}: status");
        for marker in [
            "MERGE_HEAD",
            "CHERRY_PICK_HEAD",
            "REVERT_HEAD",
            "rebase-merge",
        ] {
            assert!(
                !h.scratch.path().join(".git").join(marker).exists(),
                "{what}: {marker} left behind"
            );
        }
    };
    let head = git.rev_parse("HEAD").unwrap();
    // The ones that keep their state stop, in progress, and say on what.
    let older = git
        .run(["log", "--format=%H", "--grep=^update a$", "HEAD"])
        .unwrap();
    for (command, params) in [
        ("branch.merge", json!({ "target": "topic" })),
        ("commit.cherry_pick", json!({ "commits": [topic] })),
        // Reverting the fixture's edit of a.txt conflicts with the newer one.
        ("commit.revert", json!({ "commits": [older.trim()] })),
    ] {
        let out = h.run(command, params.clone());
        assert!(out["stopped"].is_string(), "{command} {params}: {out}");
        assert_eq!(out["conflicts"], json!(["a.txt"]), "{command}");
        assert!(status(&h.git()).contains("a.txt"), "{command}");
        h.run("op.abort", json!({}));
        untouched(&h, &head, command);
    }
    // The ones that keep none are taken back at once.
    for (command, params) in [
        (
            "branch.merge",
            json!({ "target": "topic", "mode": "squash" }),
        ),
        (
            "commit.cherry_pick",
            json!({ "commits": [topic], "no_commit": true }),
        ),
    ] {
        let err = h.try_run(command, params.clone()).unwrap_err();
        assert!(err.contains("a.txt"), "{command} {params}: {err}");
        untouched(&h, &head, command);
    }

    git.run(["switch", "-q", "topic"]).unwrap();
    let head = git.rev_parse("HEAD").unwrap();
    let out = h.run("branch.rebase", json!({ "onto": "main" }));
    assert!(out["stopped"].is_string(), "{out}");
    h.run("op.abort", json!({}));
    untouched(&h, &head, "rebase");
    assert_eq!(
        git.run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "topic\n"
    );
    // Nothing was journalled for any of it.
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn hostile_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new("branches-merges");
    let head = h.git().rev_parse("HEAD").unwrap();
    let long = "z".repeat(5000);
    for bad in [
        "",
        "-X",
        "--abort",
        "--exec=touch pwned",
        "no-such-rev",
        "a\nb",
        "HEAD^{tree}",
        long.as_str(),
    ] {
        for (command, params) in [
            ("branch.merge", json!({ "target": bad })),
            ("branch.rebase", json!({ "onto": bad })),
            ("branch.rebase", json!({ "onto": "HEAD~1", "after": bad })),
            (
                "branch.merge",
                json!({ "target": bad, "mode": "no_commit" }),
            ),
            ("branch.reset", json!({ "target": bad, "mode": "hard" })),
            ("commit.cherry_pick", json!({ "commits": [bad] })),
            ("commit.cherry_pick", json!({ "commits": ["HEAD", bad] })),
            ("commit.revert", json!({ "commits": [bad] })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted {bad:?}"
            );
        }
    }
    for params in [
        json!({}),
        json!({ "commits": [] }),
        json!({ "commits": "HEAD" }),
        json!({ "target": "HEAD", "mode": "sideways" }),
        json!({ "commits": ["HEAD"], "mainline": -1 }),
        json!({ "commits": ["HEAD"], "mainline": 99 }),
        json!({ "onto": "HEAD", "autostash": "yes" }),
        json!({ "onto": "HEAD", "after": 7 }),
    ] {
        for command in [
            "branch.merge",
            "branch.rebase",
            "branch.reset",
            "commit.cherry_pick",
            "commit.revert",
        ] {
            let _ = h.try_run(command, params.clone());
        }
    }
    // Whatever the last loop managed to run legitimately is undone.
    while h.run("undo.peek", json!({}))["undo"].is_string() {
        if h.try_run("undo.last", json!({})).is_err() {
            break;
        }
    }
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(status(&h.git()), "");
    assert!(!h.scratch.path().join("pwned").exists());
}
