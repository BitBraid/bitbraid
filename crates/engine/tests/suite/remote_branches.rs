//! Deleting a branch on a remote, against `git push --delete`
//! on twin scratch copies, each with a bare repository beside it for a
//! remote: the references of the remote and of the clone have to come out
//! the same, undo has to put back exactly what was there, and a branch
//! somebody pushed to in the meantime has to survive.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

fn remote_of(copy: &Scratch) -> PathBuf {
    copy.path().with_extension("remote.git")
}

/// The references of the clone and of its remote, as text.
fn state(copy: &Scratch) -> Vec<String> {
    let git = copy.git();
    let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
    let remote = remote_of(copy);
    let there = git
        .run([
            "--git-dir",
            remote.to_str().unwrap(),
            "for-each-ref",
            "--format=remote %(refname) %(objectname)",
        ])
        .unwrap();
    out.extend(there.lines().map(str::to_owned));
    out
}

impl Harness {
    /// Both copies with a remote `origin` that has every branch, and a
    /// branch `gone` that exists on the remote only.
    fn new() -> Self {
        let scratch = Scratch::of("branches-merges").unwrap();
        let twin = Scratch::of("branches-merges").unwrap();
        for copy in [&scratch, &twin] {
            let git = copy.git();
            let bare = remote_of(copy);
            let _ = std::fs::remove_dir_all(&bare);
            git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
                .unwrap();
            git.run(["remote", "add", "origin", bare.to_str().unwrap()])
                .unwrap();
            git.run(["push", "-q", "origin", "feature/a:refs/heads/gone"])
                .unwrap();
            git.run(["fetch", "-q", "origin"]).unwrap();
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
    /// Our copy and its remote look like git's copy and its remote. The
    /// two remotes are different folders with the same content, so the
    /// texts compare as they are.
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.scratch), state(&self.twin), "{what}");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        for copy in [&self.scratch, &self.twin] {
            let _ = std::fs::remove_dir_all(remote_of(copy));
        }
    }
}

#[test]
fn deleting_on_the_remote_matches_git_push_delete_and_undoes() {
    let mut h = Harness::new();
    h.assert_same("to begin with");
    let before = state(&h.scratch);
    let tip = h.git().run(["rev-parse", "origin/gone"]).unwrap();

    let out = h.run(
        "branch.delete_remote",
        json!({ "remote": "origin", "branch": "gone" }),
    );
    assert_eq!(out["deleted"], "origin/gone");
    assert_eq!(out["target"], tip.trim());
    h.twin
        .git()
        .run(["push", "-q", "--delete", "origin", "gone"])
        .unwrap();
    h.assert_same("after the delete");
    assert!(
        !state(&h.scratch)
            .iter()
            .any(|line| line.contains("refs/heads/gone") || line.contains("origin/gone")),
        "the branch and its remote-tracking branch are gone"
    );

    // With the reflogs expired, a prune must not take what undo needs.
    h.git()
        .run(["reflog", "expire", "--expire=now", "--all"])
        .unwrap();
    h.git().run(["gc", "-q", "--prune=now"]).unwrap();

    // Undo pushes it back where it was, and says so in what peek tells.
    let peek = h.run("undo.peek", json!({}));
    assert_eq!(peek["undo"], "Delete origin/gone on origin");
    assert_eq!(peek["undo_remote"], true);
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.scratch), before, "undo puts back what was there");
    let peek = h.run("undo.peek", json!({}));
    assert_eq!(peek["redo_remote"], true);
    assert_eq!(peek["undo_remote"], false);

    // Redo deletes it again, as git did on the twin.
    h.run("redo.last", json!({}));
    h.assert_same("after redo");
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.scratch), before);
}

#[test]
fn a_local_branch_of_the_same_name_stays() {
    let mut h = Harness::new();
    for copy in [&h.scratch, &h.twin] {
        copy.git()
            .run(["push", "-q", "origin", "feature/a"])
            .unwrap();
        copy.git().run(["fetch", "-q", "origin"]).unwrap();
        copy.git()
            .run([
                "branch",
                "-q",
                "--set-upstream-to=origin/feature/a",
                "feature/a",
            ])
            .unwrap();
    }
    h.run(
        "branch.delete_remote",
        json!({ "remote": "origin", "branch": "feature/a" }),
    );
    h.twin
        .git()
        .run(["push", "-q", "--delete", "origin", "feature/a"])
        .unwrap();
    h.assert_same("a branch with a slash in its name, followed by a local one");
    assert!(
        h.git()
            .run(["rev-parse", "--verify", "refs/heads/feature/a"])
            .is_ok(),
        "the local branch is still there"
    );
}

#[test]
fn a_branch_somebody_pushed_to_since_is_not_deleted() {
    let mut h = Harness::new();
    // Somebody else moves the branch on the remote; here nothing is fetched.
    let remote = remote_of(&h.scratch);
    let newer = h.git().run(["rev-parse", "main"]).unwrap();
    h.git()
        .run([
            "--git-dir",
            remote.to_str().unwrap(),
            "update-ref",
            "refs/heads/gone",
            newer.trim(),
        ])
        .unwrap();
    let before = state(&h.scratch);
    let err = h
        .try_run(
            "branch.delete_remote",
            json!({ "remote": "origin", "branch": "gone" }),
        )
        .unwrap_err();
    assert!(err.contains("somebody pushed to it"), "{err}");
    assert!(err.contains("Fetch origin"), "{err}");
    assert_eq!(state(&h.scratch), before, "nothing was deleted");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], Value::Null);

    // Fetched, it is what is known here, and goes.
    h.git().run(["fetch", "-q", "origin"]).unwrap();
    h.run(
        "branch.delete_remote",
        json!({ "remote": "origin", "branch": "gone" }),
    );
    // Somebody makes a branch of that name again: undo leaves it alone.
    let other = h.git().run(["rev-parse", "feature/b"]).unwrap();
    h.git()
        .run([
            "--git-dir",
            remote.to_str().unwrap(),
            "update-ref",
            "refs/heads/gone",
            other.trim(),
        ])
        .unwrap();
    let before = state(&h.scratch);
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("has a branch gone again"), "{err}");
    assert_eq!(state(&h.scratch), before);
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Delete origin/gone on origin",
        "the entry stays for when the way is clear"
    );
}

#[test]
fn hostile_remotes_and_branches_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let before = state(&h.scratch);
    let long = "x".repeat(70_000);
    let cases = [
        json!({}),
        json!({ "remote": "origin" }),
        json!({ "branch": "gone" }),
        json!({ "remote": 7, "branch": "gone" }),
        json!({ "remote": "origin", "branch": ["gone"] }),
        json!({ "remote": "", "branch": "gone" }),
        json!({ "remote": "origin", "branch": "" }),
        json!({ "remote": "--receive-pack=touch pwned", "branch": "gone" }),
        json!({ "remote": "-origin", "branch": "gone" }),
        json!({ "remote": "nowhere", "branch": "gone" }),
        json!({ "remote": "origin", "branch": "--all" }),
        json!({ "remote": "origin", "branch": "-d" }),
        json!({ "remote": "origin", "branch": "no such branch" }),
        json!({ "remote": "origin", "branch": "never-fetched" }),
        json!({ "remote": "origin", "branch": "gone\u{0}x" }),
        json!({ "remote": "origin", "branch": "gone\nmain" }),
        json!({ "remote": "origin", "branch": "gone:main" }),
        json!({ "remote": "origin", "branch": "refs/heads/gone" }),
        json!({ "remote": "origin", "branch": "+gone" }),
        json!({ "remote": "origin", "branch": long }),
        json!({ "remote": "origin", "branch": "gone", "protected": "gone" }),
        // A protected name is refused before anything is asked of git.
        json!({ "remote": "origin", "branch": "gone", "protected": ["gone"] }),
        json!({ "remote": "origin", "branch": "main", "protected": ["main", "master"] }),
    ];
    for params in cases {
        let result = h.try_run("branch.delete_remote", params.clone());
        assert!(result.is_err(), "{params} was taken: {result:?}");
        assert_eq!(state(&h.scratch), before, "after {params}");
    }
    assert!(!h.scratch.path().join("pwned").exists());
    assert_eq!(h.run("undo.peek", json!({}))["undo"], Value::Null);
}
