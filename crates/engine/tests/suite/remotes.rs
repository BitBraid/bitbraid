//! Fetch, pull, push and remote management against git on twin scratch
//! copies. The remotes are bare repositories on the local file system, so
//! nothing here needs a network.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, atomic::Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Side {
    copy: Scratch,
    bare: PathBuf,
}

impl Side {
    fn new(fixture: &str) -> Self {
        let copy = Scratch::of(fixture).unwrap();
        let bare = copy.path().with_extension("origin.git");
        let _ = std::fs::remove_dir_all(&bare);
        let git = copy.git();
        git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
            .unwrap();
        git.run(["remote", "add", "origin", bare.to_str().unwrap()])
            .unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        git.run(["branch", "-q", "--set-upstream-to=origin/main", "main"])
            .unwrap();
        // Commands run git with the real environment; merges need a name.
        git.run(["config", "user.name", "Ada Fixture"]).unwrap();
        git.run(["config", "user.email", "ada@fixture.invalid"])
            .unwrap();
        Self { copy, bare }
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    fn remote(&self) -> Git {
        Git::new(&self.bare, fixtures_out_dir().join(".home"))
    }
    /// Someone else pushes a commit to `branch` on the remote.
    fn upstream_commit(&self, branch: &str, file: &str, content: &str) {
        let other = self.copy.path().with_extension("other");
        let _ = std::fs::remove_dir_all(&other);
        self.git()
            .run([
                "clone",
                "-q",
                self.bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ])
            .unwrap();
        let git = Git::new(&other, fixtures_out_dir().join(".home"));
        git.run(["switch", "-q", branch]).unwrap();
        std::fs::write(other.join(file), content).unwrap();
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "-m", &format!("upstream: {file}")])
            .unwrap();
        git.run(["push", "-q", "origin", branch]).unwrap();
        let _ = std::fs::remove_dir_all(&other);
    }
    /// A local commit, with the fixture's fixed dates so that both sides
    /// make the same one.
    fn local_commit(&self, file: &str, content: &str) {
        self.copy.write(file, content).unwrap();
        self.git().run(["add", file]).unwrap();
        self.git()
            .run(["commit", "-q", "-m", &format!("local: {file}")])
            .unwrap();
    }
    fn refs(&self) -> Vec<String> {
        self.git()
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap()
    }
    fn remote_refs(&self) -> Vec<String> {
        self.remote()
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap()
    }
}

struct Harness {
    ours: Side,
    twin: Side,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let ours = Side::new("branches-merges");
        let twin = Side::new("branches-merges");
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": ours.copy.path() }))
            .expect("open scratch");
        Self {
            ours,
            twin,
            registry,
            ctx,
        }
    }
    fn both(&self, f: impl Fn(&Side)) {
        f(&self.ours);
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
        assert_eq!(self.ours.refs(), self.twin.refs(), "{what}: local refs");
        assert_eq!(
            self.ours.remote_refs(),
            self.twin.remote_refs(),
            "{what}: remote refs"
        );
    }
}

#[test]
fn fetch_matches_git_fetch_including_prune() {
    let mut h = Harness::new();
    h.both(|side| side.upstream_commit("main", "up.txt", "one\n"));
    h.run("remote.fetch", json!({ "remote": "origin" }));
    h.twin.git().run(["fetch", "-q", "origin"]).unwrap();
    h.assert_same("fetch one remote");

    // A branch deleted on the remote stays without prune and goes with it.
    h.both(|side| {
        side.remote()
            .run(["update-ref", "-d", "refs/heads/feature/b"])
            .unwrap();
    });
    h.run("remote.fetch", json!({}));
    h.twin.git().run(["fetch", "-q", "--all"]).unwrap();
    h.assert_same("fetch all without prune");
    assert!(
        h.ours
            .refs()
            .iter()
            .any(|r| r.starts_with("refs/remotes/origin/feature/b "))
    );
    h.run("remote.fetch", json!({ "prune": true }));
    h.twin
        .git()
        .run(["fetch", "-q", "--all", "--prune"])
        .unwrap();
    h.assert_same("fetch with prune");
    assert!(
        !h.ours
            .refs()
            .iter()
            .any(|r| r.starts_with("refs/remotes/origin/feature/b "))
    );

    assert!(
        h.try_run("remote.fetch", json!({ "remote": "nowhere" }))
            .is_err()
    );
    h.assert_same("a failed fetch changes nothing");
}

#[test]
fn pull_modes_match_git_pull() {
    // Fast-forward.
    let mut h = Harness::new();
    h.both(|side| side.upstream_commit("main", "up.txt", "one\n"));
    let out = h.run("remote.pull", json!({ "mode": "ff_only" }));
    assert_ne!(out["before"]["commit"], out["after"]["commit"]);
    h.twin.git().run(["pull", "-q", "--ff-only"]).unwrap();
    h.assert_same("fast-forward pull");

    // Diverged: fast-forward only refuses and leaves everything alone.
    h.both(|side| {
        side.local_commit("mine.txt", "mine\n");
        side.upstream_commit("main", "up2.txt", "two\n");
    });
    let before = h.ours.refs();
    let head = h.ours.git().rev_parse("HEAD").unwrap();
    assert!(
        h.try_run("remote.pull", json!({ "mode": "ff_only" }))
            .is_err()
    );
    assert_eq!(h.ours.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(
        h.ours
            .refs()
            .iter()
            .filter(|r| r.starts_with("refs/heads/"))
            .collect::<Vec<_>>(),
        before
            .iter()
            .filter(|r| r.starts_with("refs/heads/"))
            .collect::<Vec<_>>()
    );

    // Merge: a commit with the local tip first and the upstream second.
    // Its id carries today's date, so trees and parents are compared.
    let shape = |side: &Side| {
        let git = side.git();
        (
            git.rev_parse("HEAD^{tree}").unwrap(),
            git.run(["rev-list", "--parents", "-n", "1", "HEAD"])
                .unwrap()
                .split_whitespace()
                .skip(1)
                .map(str::to_owned)
                .collect::<Vec<_>>(),
        )
    };
    h.run("remote.pull", json!({ "mode": "merge" }));
    h.twin
        .git()
        .run(["pull", "-q", "--no-rebase", "--no-ff", "--no-edit"])
        .unwrap();
    assert_eq!(shape(&h.ours), shape(&h.twin), "merge pull");
    assert_eq!(shape(&h.ours).1.len(), 2);

    // Rebase: the local commit replayed on top of the upstream.
    let mut h = Harness::new();
    h.both(|side| {
        side.local_commit("mine.txt", "mine\n");
        side.upstream_commit("main", "up.txt", "one\n");
    });
    h.run("remote.pull", json!({ "mode": "rebase" }));
    h.twin.git().run(["pull", "-q", "--rebase"]).unwrap();
    assert_eq!(shape(&h.ours), shape(&h.twin), "rebase pull");
    assert_eq!(
        shape(&h.ours).1,
        [h.ours.git().rev_parse("origin/main").unwrap()],
        "one parent: the upstream tip"
    );
}

#[test]
fn pull_is_undone_and_a_conflicting_pull_stops_to_be_resolved() {
    let mut h = Harness::new();
    h.ours.upstream_commit("main", "up.txt", "one\n");
    let git = h.ours.git();
    let start = git.rev_parse("HEAD").unwrap();
    h.run("remote.pull", json!({ "mode": "ff_only" }));
    let pulled = git.rev_parse("HEAD").unwrap();
    assert_ne!(pulled, start);
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Pull");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "");
    assert_eq!(
        git.rev_parse("origin/main").unwrap(),
        pulled,
        "fetched stays"
    );
    h.run("redo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), pulled);

    // Up to date: nothing moved, nothing to undo twice.
    h.run("remote.pull", json!({}));
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    h.run("redo.last", json!({}));

    // Both sides write the same file differently: the pull's merge or
    // rebase stops, in progress, and aborting it goes back.
    h.ours.local_commit("up.txt", "mine\n");
    h.ours.upstream_commit("main", "up.txt", "theirs\n");
    let head = git.rev_parse("HEAD").unwrap();
    for mode in ["merge", "ff", "rebase"] {
        let out = h.run("remote.pull", json!({ "mode": mode }));
        assert!(out["stopped"].is_string(), "{mode}: {out}");
        assert_eq!(out["conflicts"], json!(["up.txt"]), "{mode}");
        let operation = h.run("op.status", json!({}))["operation"].clone();
        assert_eq!(
            operation["kind"],
            if mode == "rebase" { "rebase" } else { "merge" },
            "{mode}"
        );
        h.run("op.abort", json!({}));
        assert_eq!(git.rev_parse("HEAD").unwrap(), head, "{mode}");
        assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "", "{mode}");
        assert_eq!(
            git.run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
            "main\n"
        );
        for marker in ["MERGE_HEAD", "rebase-merge", "rebase-apply"] {
            assert!(
                !h.ours.copy.path().join(".git").join(marker).exists(),
                "{mode}: {marker} left behind"
            );
        }
    }
    // Still the earlier pull: none of the stopped ones was journalled.
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Pull");

    // Resolved and continued, a pull is one step to undo like any other.
    h.run("remote.pull", json!({ "mode": "merge" }));
    h.run(
        "conflict.take",
        json!({ "path": "up.txt", "side": "theirs" }),
    );
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    assert_eq!(
        std::fs::read_to_string(h.ours.copy.path().join("up.txt")).unwrap(),
        "theirs\n"
    );
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), head);
}

#[test]
fn a_branch_that_is_not_checked_out_fast_forwards_to_its_upstream() {
    let mut h = Harness::new();
    // A branch following origin/main, left behind by two commits there.
    h.both(|side| {
        let git = side.git();
        git.run(["branch", "-q", "--track", "follower", "origin/main"])
            .unwrap();
        side.upstream_commit("main", "up.txt", "one\n");
        side.upstream_commit("main", "up2.txt", "two\n");
        git.run(["fetch", "-q", "origin"]).unwrap();
    });
    let git = h.ours.git();
    let start = git.rev_parse("follower").unwrap();
    let head = git.rev_parse("HEAD").unwrap();

    let out = h.run("branch.fast_forward", json!({ "name": "follower" }));
    // What `git fetch . a:b` does, which only ever fast-forwards.
    h.twin
        .git()
        .run([
            "fetch",
            "-q",
            ".",
            "refs/remotes/origin/main:refs/heads/follower",
        ])
        .unwrap();
    h.assert_same("fast-forward of a branch that is not checked out");
    assert_eq!(out["before"], start);
    assert_eq!(out["after"], git.rev_parse("origin/main").unwrap());
    assert_eq!(git.rev_parse("HEAD").unwrap(), head, "HEAD stays");
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "");

    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Fast-forward follower"
    );
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("follower").unwrap(), start);
    h.run("redo.last", json!({}));
    h.assert_same("redo of the fast-forward");

    // Up to date: nothing moves, nothing more to undo.
    let again = h.run("branch.fast_forward", json!({ "name": "follower" }));
    assert_eq!(again["before"], again["after"]);
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("follower").unwrap(), start);

    // Moved by hand since: redo leaves it alone.
    git.run(["branch", "-q", "-f", "follower", "HEAD~1"])
        .unwrap();
    assert!(h.try_run("redo.last", json!({})).is_err());
    assert_eq!(
        git.rev_parse("follower").unwrap(),
        git.rev_parse("HEAD~1").unwrap()
    );
}

#[test]
fn a_fast_forward_that_is_not_one_is_refused() {
    let mut h = Harness::new();
    let git = h.ours.git();
    let refs = |h: &Harness| h.ours.refs();

    // Checked out here: that is what pull is for.
    h.ours.upstream_commit("main", "up.txt", "one\n");
    git.run(["fetch", "-q", "origin"]).unwrap();
    let before = refs(&h);
    let err = h
        .try_run("branch.fast_forward", json!({ "name": "main" }))
        .unwrap_err();
    assert!(err.contains("checked out"), "{err}");

    // Checked out in another worktree: the same.
    git.run(["branch", "-q", "--track", "elsewhere", "origin/main"])
        .unwrap();
    git.run(["reset", "-q", "--hard", "HEAD"]).unwrap();
    let other = h.ours.copy.path().with_extension("wt");
    let _ = std::fs::remove_dir_all(&other);
    git.run([
        "worktree",
        "add",
        "-q",
        other.to_str().unwrap(),
        "elsewhere",
    ])
    .unwrap();
    git.run(["update-ref", "refs/heads/elsewhere", "HEAD~1"])
        .unwrap();
    let before_wt = refs(&h);
    let err = h
        .try_run("branch.fast_forward", json!({ "name": "elsewhere" }))
        .unwrap_err();
    assert!(err.contains("checked out at"), "{err}");
    assert_eq!(refs(&h), before_wt);
    git.run(["worktree", "remove", "--force", other.to_str().unwrap()])
        .unwrap();
    git.run(["branch", "-q", "-D", "elsewhere"]).unwrap();

    // Its own commit that the upstream does not have: not a fast-forward.
    git.run(["branch", "-q", "--track", "diverged", "origin/main"])
        .unwrap();
    git.run(["switch", "-q", "diverged"]).unwrap();
    git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    h.ours.local_commit("mine.txt", "mine\n");
    git.run(["switch", "-q", "main"]).unwrap();
    let diverged = git.rev_parse("diverged").unwrap();
    let err = h
        .try_run("branch.fast_forward", json!({ "name": "diverged" }))
        .unwrap_err();
    assert!(err.contains("cannot simply move forward"), "{err}");
    assert_eq!(git.rev_parse("diverged").unwrap(), diverged);

    // No upstream, no such branch, and names that are not names.
    git.run(["branch", "-q", "loner", "HEAD~1"]).unwrap();
    let err = h
        .try_run("branch.fast_forward", json!({ "name": "loner" }))
        .unwrap_err();
    assert!(err.contains("no upstream"), "{err}");
    let long = "b".repeat(5000);
    for name in [
        "",
        "-f",
        "nope",
        "a..b",
        "HEAD",
        "@{-1}",
        "origin/main",
        long.as_str(),
    ] {
        assert!(
            h.try_run("branch.fast_forward", json!({ "name": name }))
                .is_err(),
            "accepted {name:?}"
        );
    }
    assert!(h.try_run("branch.fast_forward", json!({})).is_err());
    let _ = before;
    assert_eq!(
        git.rev_parse("loner").unwrap(),
        git.rev_parse("HEAD~1").unwrap()
    );
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn push_matches_git_push_and_respects_the_remote() {
    let mut h = Harness::new();
    h.both(|side| side.local_commit("mine.txt", "mine\n"));
    h.run("remote.push", json!({}));
    h.twin.git().run(["push", "-q"]).unwrap();
    h.assert_same("plain push");

    // The remote moved: a plain push is refused and the remote keeps its tip.
    h.both(|side| {
        side.upstream_commit("main", "up.txt", "one\n");
        side.local_commit("mine2.txt", "more\n");
    });
    let remote_before = h.ours.remote_refs();
    let err = h.try_run("remote.push", json!({})).unwrap_err();
    assert!(
        err.contains("rejected") || err.contains("fetch first"),
        "{err}"
    );
    assert_eq!(h.ours.remote_refs(), remote_before);

    // With lease and a stale view of the remote it is refused as well.
    assert!(
        h.try_run("remote.push", json!({ "force": "lease" }))
            .is_err()
    );
    assert_eq!(h.ours.remote_refs(), remote_before);
    // After a fetch the lease holds and the push overwrites.
    h.run("remote.fetch", json!({ "remote": "origin" }));
    h.twin.git().run(["fetch", "-q", "origin"]).unwrap();
    h.run("remote.push", json!({ "force": "lease" }));
    h.twin
        .git()
        .run(["push", "-q", "--force-with-lease"])
        .unwrap();
    h.assert_same("push with lease");

    // First push of a new branch sets its upstream.
    h.both(|side| {
        side.git().run(["switch", "-q", "-c", "topic"]).unwrap();
        side.local_commit("topic.txt", "t\n");
    });
    h.run(
        "remote.push",
        json!({ "branch": "topic", "set_upstream": true }),
    );
    h.twin
        .git()
        .run(["push", "-q", "--set-upstream", "origin", "topic"])
        .unwrap();
    h.assert_same("first push of a branch");
    let upstream = |side: &Side| {
        side.git()
            .run(["rev-parse", "--abbrev-ref", "topic@{upstream}"])
            .unwrap()
    };
    assert_eq!(upstream(&h.ours), "origin/topic\n");
    assert_eq!(upstream(&h.ours), upstream(&h.twin));

    // Tags travel when asked.
    h.both(|side| {
        side.git().run(["tag", "v2.0"]).unwrap();
    });
    h.run("remote.push", json!({ "tags": true }));
    h.twin.git().run(["push", "-q", "--tags"]).unwrap();
    h.assert_same("push with tags");
}

#[test]
fn a_forced_push_says_what_it_overwrites_and_a_lease_holds_to_what_was_shown() {
    let mut h = Harness::new();
    h.both(|side| {
        side.upstream_commit("main", "up.txt", "one\n");
        side.local_commit("mine.txt", "mine\n");
        side.git().run(["fetch", "-q", "origin"]).unwrap();
    });
    // What would go, as git counts and lists it.
    let preview = h.run("remote.push_preview", json!({}));
    let counted = h
        .ours
        .git()
        .run(["rev-list", "--count", "--left-right", "main...origin/main"])
        .unwrap();
    let mut counted = counted.split_whitespace();
    assert_eq!(
        preview["ahead"].as_u64().unwrap().to_string(),
        counted.next().unwrap()
    );
    assert_eq!(
        preview["behind"].as_u64().unwrap().to_string(),
        counted.next().unwrap()
    );
    assert_eq!(preview["behind"], 1);
    assert_eq!(preview["branch"], "main");
    assert_eq!(preview["remote"], "origin");
    let listed: Vec<String> = preview["lost"]
        .as_array()
        .unwrap()
        .iter()
        .map(|lost| {
            format!(
                "{} {}",
                lost["id"].as_str().unwrap(),
                lost["summary"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        listed,
        h.ours
            .git()
            .lines(["log", "--format=%H %s", "main..origin/main"])
            .unwrap()
    );
    let shown = preview["there"].as_str().unwrap().to_owned();
    assert_eq!(
        shown,
        h.ours
            .git()
            .run(["rev-parse", "origin/main"])
            .unwrap()
            .trim()
    );

    // Somebody pushes again, and something fetches in between (a timer):
    // the remote-tracking branch is where the remote is, so a bare lease
    // would overwrite what nobody was shown. A lease on what was shown
    // refuses, like git's own with that commit named.
    h.both(|side| {
        side.upstream_commit("main", "up2.txt", "two\n");
        side.git().run(["fetch", "-q", "origin"]).unwrap();
    });
    let remote_before = h.ours.remote_refs();
    let err = h
        .try_run("remote.push", json!({ "force": "lease", "expect": shown }))
        .unwrap_err();
    assert!(
        err.contains("stale info") || err.contains("rejected"),
        "{err}"
    );
    assert_eq!(h.ours.remote_refs(), remote_before);
    let lease = format!("--force-with-lease=refs/heads/main:{shown}");
    assert!(
        h.twin
            .git()
            .run(["push", "-q", lease.as_str(), "origin", "main"])
            .is_err()
    );
    h.assert_same("a lease on a commit the remote has left");

    // Shown again, the lease holds and the push overwrites.
    let preview = h.run("remote.push_preview", json!({ "branch": "main" }));
    assert_eq!(preview["behind"], 2);
    let shown = preview["there"].as_str().unwrap().to_owned();
    h.run(
        "remote.push",
        json!({ "force": "lease", "expect": shown, "branch": "main" }),
    );
    let lease = format!("--force-with-lease=refs/heads/main:{shown}");
    h.twin
        .git()
        .run(["push", "-q", lease.as_str(), "origin", "main"])
        .unwrap();
    h.assert_same("push with a lease on what was shown");
    assert_eq!(h.run("remote.push_preview", json!({}))["behind"], 0);

    // By force, what arrived unseen goes too.
    h.both(|side| {
        side.upstream_commit("main", "up3.txt", "three\n");
        side.local_commit("mine3.txt", "more\n");
    });
    h.run("remote.push", json!({ "force": "force" }));
    h.twin.git().run(["push", "-q", "--force"]).unwrap();
    h.assert_same("push by force");

    // A branch the remote does not have: nothing to overwrite.
    h.both(|side| {
        side.git().run(["switch", "-q", "-c", "fresh"]).unwrap();
    });
    let preview = h.run("remote.push_preview", json!({}));
    assert_eq!(preview["branch"], "fresh");
    assert!(preview["there"].is_null());
    assert_eq!(preview["behind"], 0);
    assert!(preview["lost"].as_array().unwrap().is_empty());
}

#[test]
fn a_branch_is_pushed_to_the_remote_it_follows() {
    let mut h = Harness::new();
    // A second remote, and a branch that follows it.
    h.both(|side| {
        let backup = side.copy.path().with_extension("backup.git");
        let _ = std::fs::remove_dir_all(&backup);
        let git = side.git();
        git.run(["init", "-q", "--bare", backup.to_str().unwrap()])
            .unwrap();
        git.run(["remote", "add", "backup", backup.to_str().unwrap()])
            .unwrap();
        git.run(["switch", "-q", "-c", "side"]).unwrap();
        git.run(["push", "-q", "--set-upstream", "backup", "side"])
            .unwrap();
        side.local_commit("side.txt", "side\n");
        git.run(["switch", "-q", "main"]).unwrap();
    });
    let backup_refs = |side: &Side| {
        Git::new(
            side.copy.path().with_extension("backup.git"),
            fixtures_out_dir().join(".home"),
        )
        .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
        .unwrap()
    };
    assert_eq!(
        h.run("remote.push_preview", json!({ "branch": "side" }))["remote"],
        "backup"
    );
    // It goes to `backup`, not to `origin`, as `git push` from the branch
    // would send it.
    h.run("remote.push", json!({ "branch": "side" }));
    h.twin.git().run(["push", "-q", "backup", "side"]).unwrap();
    h.assert_same("push of a branch that follows another remote");
    assert_eq!(backup_refs(&h.ours), backup_refs(&h.twin));
    assert!(
        !h.ours
            .remote_refs()
            .iter()
            .any(|line| line.starts_with("refs/heads/side "))
    );
    // The setting that says where pushes go wins over what is followed.
    h.both(|side| {
        side.git()
            .run(["config", "branch.side.pushRemote", "origin"])
            .unwrap();
    });
    h.run("remote.push", json!({ "branch": "side" }));
    h.twin.git().run(["push", "-q", "origin", "side"]).unwrap();
    h.assert_same("push to the branch's push remote");
}

#[test]
fn a_branch_is_pushed_to_a_branch_of_another_name() {
    let mut h = Harness::new();
    h.both(|side| {
        side.git().run(["switch", "-q", "-c", "mine"]).unwrap();
        side.local_commit("mine.txt", "mine\n");
    });
    // Onto a branch the remote does not have yet, and then onto main.
    h.run(
        "remote.push",
        json!({ "branch": "mine", "remote": "origin", "to": "theirs" }),
    );
    h.twin
        .git()
        .run(["push", "-q", "origin", "mine:refs/heads/theirs"])
        .unwrap();
    h.assert_same("push to a branch of another name");
    assert!(
        h.ours
            .remote_refs()
            .iter()
            .any(|line| line.starts_with("refs/heads/theirs "))
    );
    h.run(
        "remote.push",
        json!({ "branch": "mine", "remote": "origin", "to": "main" }),
    );
    h.twin
        .git()
        .run(["push", "-q", "origin", "mine:refs/heads/main"])
        .unwrap();
    h.assert_same("push onto the remote's main");
    // Not forward: refused like any push, and the remote keeps its branch.
    h.both(|side| {
        side.git().run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
        side.local_commit("other.txt", "other\n");
    });
    let before = h.ours.remote_refs();
    assert!(
        h.try_run(
            "remote.push",
            json!({ "branch": "mine", "remote": "origin", "to": "main" })
        )
        .is_err()
    );
    assert_eq!(h.ours.remote_refs(), before);
    // A name git could read as something else.
    for to in ["", "-f", "a..b", "x:y", "main:main", "refs/heads/*"] {
        assert!(
            h.try_run(
                "remote.push",
                json!({ "branch": "mine", "remote": "origin", "to": to })
            )
            .is_err(),
            "{to:?}"
        );
    }
    assert_eq!(h.ours.remote_refs(), before);
}

#[test]
fn a_branch_moves_forward_to_a_commit_that_is_named() {
    let mut h = Harness::new();
    h.both(|side| {
        side.git().run(["branch", "behind", "HEAD~2"]).unwrap();
        side.git().run(["branch", "aside", "HEAD~2"]).unwrap();
    });
    let start = h.ours.git().rev_parse("behind").unwrap();
    // As git moves a branch that is not checked out forward.
    h.run(
        "branch.fast_forward",
        json!({ "name": "behind", "to": "main~1" }),
    );
    h.twin
        .git()
        .run(["push", "-q", ".", "main~1:refs/heads/behind"])
        .unwrap();
    h.assert_same("fast-forward to a commit");
    assert_eq!(
        h.ours.git().rev_parse("behind").unwrap(),
        h.ours.git().rev_parse("main~1").unwrap()
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Fast-forward behind");
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.git().rev_parse("behind").unwrap(), start);
    h.run("redo.last", json!({}));
    h.assert_same("redo of the fast-forward");

    // Backwards, or sideways, is not forward: refused, nothing moves.
    let at = h.ours.git().rev_parse("behind").unwrap();
    let err = h
        .try_run(
            "branch.fast_forward",
            json!({ "name": "behind", "to": "main~2" }),
        )
        .unwrap_err();
    assert!(err.contains("cannot simply move forward"), "{err}");
    h.both(|side| {
        side.git().run(["switch", "-q", "aside"]).unwrap();
        side.local_commit("aside.txt", "aside\n");
        side.git().run(["switch", "-q", "main"]).unwrap();
    });
    assert!(
        h.try_run(
            "branch.fast_forward",
            json!({ "name": "aside", "to": "main" })
        )
        .is_err()
    );
    // The checked-out branch is not moved under its worktree, and what
    // names no commit moves nothing.
    for (name, to) in [
        ("main", "main"),
        ("behind", "no-such-rev"),
        ("behind", "-x"),
        ("behind", ""),
        ("no-such-branch", "main"),
    ] {
        assert!(
            h.try_run("branch.fast_forward", json!({ "name": name, "to": to }))
                .is_err(),
            "{name} to {to:?}"
        );
    }
    assert_eq!(h.ours.git().rev_parse("behind").unwrap(), at);
    h.assert_same("refused fast-forwards");
}

/// A server that answers every request with "who are you?": it counts the
/// requests and notes whether one came with credentials.
struct Locked {
    port: u16,
    requests: Arc<Mutex<(usize, bool)>>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Locked {
    fn start() -> Self {
        use std::io::{Read as _, Write as _};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        listener.set_nonblocking(true).unwrap();
        let requests = Arc::new(Mutex::new((0, false)));
        let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let (seen, done) = (requests.clone(), stop.clone());
        let thread = std::thread::spawn(move || {
            while !done.load(Ordering::SeqCst) {
                let Ok((mut stream, _)) = listener.accept() else {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                    continue;
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut request = Vec::new();
                let mut buffer = [0u8; 1024];
                while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                    match stream.read(&mut buffer) {
                        Ok(0) | Err(_) => break,
                        Ok(read) => request.extend_from_slice(&buffer[..read]),
                    }
                }
                let text = String::from_utf8_lossy(&request).to_lowercase();
                {
                    let mut seen = seen.lock().unwrap();
                    seen.0 += 1;
                    seen.1 |= text.contains("authorization:");
                }
                let _ = stream.write_all(
                    b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"scratch\"\r\n\
                      Content-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        Self {
            port,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for Locked {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[test]
fn a_quiet_fetch_asks_nobody_and_says_which_remote_wants_an_answer() {
    let mut h = Harness::new();
    // Quiet, a remote that needs no answer is fetched as git fetches it.
    h.both(|side| side.upstream_commit("main", "up.txt", "one\n"));
    h.run(
        "remote.fetch",
        json!({ "remote": "origin", "quiet": true, "prune": true }),
    );
    h.twin
        .git()
        .run(["fetch", "-q", "--prune", "origin"])
        .unwrap();
    h.assert_same("a quiet fetch");

    // A remote that wants a user name. Whatever could ask is a script
    // that leaves a mark; no credential helper is consulted.
    let server = Locked::start();
    let side = h.ours.copy.path().with_extension("side");
    let _ = std::fs::remove_dir_all(&side);
    std::fs::create_dir_all(&side).unwrap();
    let mark = side.join("asked");
    let asker = bitbraid_testkit::script(
        &side,
        "askpass.sh",
        &format!("#!/bin/sh\necho asked >> '{}'\nexit 1\n", mark.display()),
    )
    .unwrap();
    let git = h.ours.git();
    let url = format!("http://127.0.0.1:{}/locked.git", server.port);
    git.run(["remote", "add", "locked", url.as_str()]).unwrap();
    git.run(["config", "credential.helper", ""]).unwrap();
    git.run(["config", "http.proxy", ""]).unwrap();
    git.run(["config", "core.askPass", asker.to_str().unwrap()])
        .unwrap();

    let err = h
        .try_run("remote.fetch", json!({ "remote": "locked", "quiet": true }))
        .unwrap_err();
    assert!(
        err.contains("locked wants an answer") && err.contains("nobody was asked"),
        "{err}"
    );
    {
        let seen = server.requests.lock().unwrap();
        assert!(seen.0 >= 1, "the server was never asked");
        assert!(!seen.1, "credentials were sent");
    }
    assert!(!mark.exists(), "something was asked");

    // All remotes at once: the one that needs nothing is fetched, the
    // other is named.
    h.ours.upstream_commit("main", "up2.txt", "two\n");
    let before = h.ours.git().run(["rev-parse", "origin/main"]).unwrap();
    let err = h
        .try_run("remote.fetch", json!({ "quiet": true }))
        .unwrap_err();
    assert!(err.contains("locked wants an answer"), "{err}");
    assert_ne!(
        h.ours.git().run(["rev-parse", "origin/main"]).unwrap(),
        before,
        "origin was not fetched"
    );
    assert!(!mark.exists(), "something was asked");
    assert!(!server.requests.lock().unwrap().1, "credentials were sent");
}

#[test]
fn remotes_are_added_pointed_and_removed_like_git_remote() {
    let mut h = Harness::new();
    let config = |side: &Side| {
        side.git()
            .run(["config", "--get-regexp", r"^remote\.second\."])
            .unwrap_or_default()
    };
    h.run(
        "remote.add",
        json!({ "name": "second", "url": "https://example.invalid/second.git" }),
    );
    h.twin
        .git()
        .run([
            "remote",
            "add",
            "second",
            "https://example.invalid/second.git",
        ])
        .unwrap();
    assert_eq!(config(&h.ours), config(&h.twin));
    assert!(config(&h.ours).contains("example.invalid/second.git"));

    h.run(
        "remote.set_url",
        json!({ "name": "second", "url": "https://example.invalid/moved.git" }),
    );
    h.run(
        "remote.set_url",
        json!({ "name": "second", "url": "ssh://example.invalid/push.git", "push": true }),
    );
    let git = h.twin.git();
    git.run([
        "remote",
        "set-url",
        "second",
        "https://example.invalid/moved.git",
    ])
    .unwrap();
    git.run([
        "remote",
        "set-url",
        "--push",
        "second",
        "ssh://example.invalid/push.git",
    ])
    .unwrap();
    assert_eq!(config(&h.ours), config(&h.twin));

    h.run("remote.remove", json!({ "name": "second" }));
    h.twin.git().run(["remote", "remove", "second"]).unwrap();
    assert_eq!(config(&h.ours), "");
    assert_eq!(config(&h.ours), config(&h.twin));

    // Existing and missing names are errors that change nothing.
    assert!(
        h.try_run("remote.add", json!({ "name": "origin", "url": "x" }))
            .is_err()
    );
    assert!(
        h.try_run("remote.remove", json!({ "name": "second" }))
            .is_err()
    );
    h.assert_same("refused remote changes");
}

#[test]
fn progress_is_reported_and_cancel_stops_before_anything_happens() {
    let mut h = Harness::new();
    h.ours.upstream_commit("main", "up.txt", "one\n");
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = seen.clone();
    h.ctx.progress = Some(Arc::new(move |progress| {
        sink.lock().unwrap().push(progress.stage);
    }));

    // Asked to stop before it starts: nothing is fetched.
    let before = h.ours.refs();
    h.ctx.cancel.store(true, Ordering::SeqCst);
    let err = h
        .try_run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_err();
    assert!(err.contains("cancelled"), "{err}");
    assert_eq!(h.ours.refs(), before);
    for (command, params) in [("remote.pull", json!({})), ("remote.push", json!({}))] {
        assert!(
            h.try_run(command, params)
                .unwrap_err()
                .contains("cancelled")
        );
    }
    assert_eq!(h.ours.refs(), before);

    // Allowed to run, it fetches; whatever stages git named were passed on.
    h.ctx.cancel.store(false, Ordering::SeqCst);
    h.run("remote.fetch", json!({ "remote": "origin" }));
    assert_ne!(h.ours.refs(), before);
    for stage in seen.lock().unwrap().iter() {
        assert!(
            stage.chars().next().unwrap().is_ascii_uppercase(),
            "{stage}"
        );
    }
}

#[test]
fn hostile_remote_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let before = (h.ours.refs(), h.ours.remote_refs());
    let config_before = h.ours.git().run(["config", "--list", "--local"]).unwrap();
    let long = "r".repeat(5000);
    for name in [
        "",
        "-x",
        "--upload-pack=touch pwned",
        "a/b",
        "a b",
        "..",
        "a\nb",
        "a\u{0}b",
        "x.lock",
        long.as_str(),
    ] {
        for (command, params) in [
            ("remote.fetch", json!({ "remote": name })),
            ("remote.push", json!({ "remote": name, "branch": "main" })),
            (
                "remote.push_preview",
                json!({ "remote": name, "branch": "main" }),
            ),
            (
                "remote.add",
                json!({ "name": name, "url": "https://example.invalid/x.git" }),
            ),
            ("remote.remove", json!({ "name": name })),
            (
                "remote.set_url",
                json!({ "name": name, "url": "https://example.invalid/x.git" }),
            ),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted {name:?}"
            );
        }
    }
    for url in ["", "-oProxyCommand=touch pwned", "--upload-pack=x", "a\nb"] {
        assert!(
            h.try_run("remote.add", json!({ "name": "evil", "url": url }))
                .is_err(),
            "{url:?}"
        );
        assert!(
            h.try_run("remote.set_url", json!({ "name": "origin", "url": url }))
                .is_err(),
            "{url:?}"
        );
    }
    for branch in ["-f", "--all", "a..b", "@{-1}", "", "no-such-branch"] {
        assert!(
            h.try_run("remote.push", json!({ "branch": branch }))
                .is_err(),
            "{branch:?}"
        );
        assert!(
            h.try_run("remote.push_preview", json!({ "branch": branch }))
                .is_err(),
            "{branch:?}"
        );
    }
    // A lease on something that is no commit id.
    for expect in [json!("main"), json!("--force"), json!(7)] {
        assert!(
            h.try_run(
                "remote.push",
                json!({ "force": "lease", "expect": expect, "branch": "main" })
            )
            .is_err(),
            "{expect}"
        );
    }
    // And on a commit the remote's branch is not at: with nothing to push
    // git has nothing to refuse, and nothing changes either way.
    let _ = h.try_run(
        "remote.push",
        json!({
            "force": "lease",
            "expect": "0000000000000000000000000000000000000001",
            "branch": "main",
        }),
    );
    for params in [
        json!({ "quiet": "very" }),
        json!({ "quiet": true, "remote": "nowhere" }),
    ] {
        assert!(
            h.try_run("remote.fetch", params.clone()).is_err(),
            "{params}"
        );
    }
    for params in [
        json!({ "mode": "sideways" }),
        json!({ "force": "very" }),
        json!({ "prune": "yes" }),
        json!({ "remote": 7 }),
    ] {
        for command in ["remote.fetch", "remote.pull", "remote.push"] {
            let _ = h.try_run(command, params.clone());
        }
    }
    assert_eq!((h.ours.refs(), h.ours.remote_refs()), before);
    assert_eq!(
        h.ours.git().run(["config", "--list", "--local"]).unwrap(),
        config_before
    );
    assert!(!h.ours.copy.path().join("pwned").exists());
}
