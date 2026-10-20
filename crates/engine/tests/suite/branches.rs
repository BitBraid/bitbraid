//! Branch commands against git on twin scratch copies: the command runs on
//! one copy, the equivalent git commands on the other, and references,
//! HEAD and branch configuration have to come out the same.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Everything a branch command can change, as text.
fn state(git: &Git) -> Vec<String> {
    let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
    let head = git
        .run(["symbolic-ref", "--quiet", "HEAD"])
        .or_else(|_| git.run(["rev-parse", "HEAD"]))
        .unwrap();
    out.push(format!("HEAD {}", head.trim()));
    let config = git
        .run(["config", "--get-regexp", r"^branch\."])
        .unwrap_or_default();
    out.extend(config.lines().map(|line| format!("config {line}")));
    out
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        let twin = Scratch::of(fixture).unwrap();
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
    /// A remote named `origin` for both copies: a bare clone beside each.
    fn with_remote(self) -> Self {
        for copy in [&self.scratch, &self.twin] {
            let git = copy.git();
            let bare = copy.path().with_extension("remote.git");
            let _ = std::fs::remove_dir_all(&bare);
            git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
                .unwrap();
            git.run(["remote", "add", "origin", bare.to_str().unwrap()])
                .unwrap();
            git.run(["fetch", "-q", "origin"]).unwrap();
        }
        self
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
    /// The copy the command ran on equals the copy git ran on. Remote URLs
    /// differ by path and are not part of the state compared.
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.git()), state(&self.twin.git()), "{what}");
    }
    fn undo(&mut self) -> Value {
        self.run("undo.last", json!({}))
    }
    fn redo(&mut self) -> Value {
        self.run("redo.last", json!({}))
    }
}

#[test]
fn create_matches_git_branch_and_undoes() {
    let mut h = Harness::new("branches-merges");
    let before = state(&h.git());
    h.run(
        "branch.create",
        json!({ "name": "topic", "start": "HEAD~1" }),
    );
    h.twin
        .git()
        .run(["branch", "--no-track", "topic", "HEAD~1"])
        .unwrap();
    h.assert_same("create at a start point");

    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Create branch topic");
    h.undo();
    assert_eq!(state(&h.git()), before);
    h.redo();
    h.assert_same("redo of create");

    // From a tag, and with checkout.
    h.run(
        "branch.create",
        json!({ "name": "release/next", "start": "v1.0", "checkout": true }),
    );
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/next", "v1.0"])
        .unwrap();
    h.assert_same("create and check out");
    h.undo();
    h.twin.git().run(["switch", "-q", "main"]).unwrap();
    h.twin.git().run(["branch", "-D", "release/next"]).unwrap();
    h.assert_same("undo of create and check out");

    // A name that exists, and a start that is nothing, change nothing.
    let before = state(&h.git());
    assert!(
        h.try_run("branch.create", json!({ "name": "main" }))
            .is_err()
    );
    assert!(
        h.try_run(
            "branch.create",
            json!({ "name": "x", "start": "no-such-thing" })
        )
        .is_err()
    );
    assert_eq!(state(&h.git()), before);
}

#[test]
fn rename_and_delete_match_git_and_undo() {
    let mut h = Harness::new("branches-merges");
    h.run(
        "branch.rename",
        json!({ "name": "feature/b", "new_name": "done/b" }),
    );
    h.twin
        .git()
        .run(["branch", "-m", "feature/b", "done/b"])
        .unwrap();
    h.assert_same("rename");
    h.undo();
    h.twin
        .git()
        .run(["branch", "-m", "done/b", "feature/b"])
        .unwrap();
    h.assert_same("undo of rename");
    h.redo();
    h.undo();
    h.assert_same("redo then undo of rename");

    // feature/a is merged into main: plain delete works.
    h.run("branch.delete", json!({ "name": "feature/a" }));
    h.twin.git().run(["branch", "-d", "feature/a"]).unwrap();
    h.assert_same("delete a merged branch");
    let before_undo = state(&h.git());
    h.undo();
    assert!(
        state(&h.git())
            .iter()
            .any(|line| line.starts_with("refs/heads/feature/a "))
    );
    h.redo();
    assert_eq!(state(&h.git()), before_undo);

    // An unmerged branch needs force, like git's -d and -D.
    h.git().run(["switch", "-q", "-c", "wip"]).unwrap();
    h.scratch.write("wip.txt", "wip\n").unwrap();
    h.git().run(["add", "wip.txt"]).unwrap();
    h.git().run(["commit", "-q", "-m", "wip"]).unwrap();
    // The current branch is refused whatever the force.
    assert!(
        h.try_run("branch.delete", json!({ "name": "wip", "force": true }))
            .is_err()
    );
    h.git().run(["switch", "-q", "main"]).unwrap();
    assert!(
        h.try_run("branch.delete", json!({ "name": "wip" }))
            .is_err()
    );
    let tip = h.git().rev_parse("wip").unwrap();
    h.run("branch.delete", json!({ "name": "wip", "force": true }));
    assert!(h.git().rev_parse("wip").is_err());
    h.undo();
    assert_eq!(h.git().rev_parse("wip").unwrap(), tip, "nothing was lost");

    // Protected names are refused before git is asked.
    let err = h
        .try_run(
            "branch.delete",
            json!({ "name": "wip", "force": true, "protected": ["main", "wip"] }),
        )
        .unwrap_err();
    assert!(err.contains("protected"), "{err}");
    assert_eq!(h.git().rev_parse("wip").unwrap(), tip);
}

#[test]
fn upstream_matches_git_and_survives_delete_and_undo() {
    let mut h = Harness::new("branches-merges").with_remote();
    h.run(
        "branch.set_upstream",
        json!({ "name": "feature/a", "upstream": "origin/feature/a" }),
    );
    h.twin
        .git()
        .run(["branch", "--set-upstream-to=origin/feature/a", "feature/a"])
        .unwrap();
    h.assert_same("set upstream");

    // Deleting and undoing brings the upstream back with the branch.
    let with_upstream = state(&h.git());
    h.run("branch.delete", json!({ "name": "feature/a" }));
    h.undo();
    assert_eq!(state(&h.git()), with_upstream);

    h.run(
        "branch.set_upstream",
        json!({ "name": "feature/a", "upstream": null }),
    );
    h.twin
        .git()
        .run(["branch", "--unset-upstream", "feature/a"])
        .unwrap();
    h.assert_same("clear upstream");
    h.undo();
    assert_eq!(state(&h.git()), with_upstream);
    h.redo();
    h.assert_same("redo of clearing");

    assert!(
        h.try_run(
            "branch.set_upstream",
            json!({ "name": "feature/a", "upstream": "origin/no-such-branch" })
        )
        .is_err()
    );
    h.assert_same("a refused upstream changes nothing");
}

#[test]
fn checkout_of_branches_remote_branches_and_commits_matches_git_switch() {
    let mut h = Harness::new("branches-merges").with_remote();
    h.run("branch.checkout", json!({ "target": "feature/a" }));
    h.twin.git().run(["switch", "-q", "feature/a"]).unwrap();
    h.assert_same("a local branch");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Check out feature/a");
    h.undo();
    h.twin.git().run(["switch", "-q", "main"]).unwrap();
    h.assert_same("undo goes back");
    h.redo();
    h.twin.git().run(["switch", "-q", "feature/a"]).unwrap();
    h.assert_same("redo goes there again");

    // A tag detaches.
    let out = h.run("branch.checkout", json!({ "target": "v1.0" }));
    assert!(out["after"]["branch"].is_null());
    h.twin
        .git()
        .run(["switch", "-q", "--detach", "v1.0"])
        .unwrap();
    h.assert_same("a tag, detached");

    // A remote branch with a local one of its name switches to that.
    h.run("branch.checkout", json!({ "target": "origin/feature/b" }));
    h.twin.git().run(["switch", "-q", "feature/b"]).unwrap();
    h.assert_same("a remote branch that has a local branch");

    // Without one it creates the tracking branch.
    for copy in [&h.scratch, &h.twin] {
        let git = copy.git();
        git.run(["switch", "-q", "main"]).unwrap();
        git.run(["branch", "-D", "feature/a"]).unwrap();
    }
    let out = h.run("branch.checkout", json!({ "target": "origin/feature/a" }));
    assert_eq!(out["created"], "feature/a");
    h.twin
        .git()
        .run([
            "switch",
            "-q",
            "--track",
            "-c",
            "feature/a",
            "origin/feature/a",
        ])
        .unwrap();
    h.assert_same("a remote branch, tracked");
    h.undo();
    h.twin.git().run(["switch", "-q", "main"]).unwrap();
    h.twin.git().run(["branch", "-D", "feature/a"]).unwrap();
    h.assert_same("undo takes the tracking branch away again");

    // Checking out where HEAD already is records nothing.
    let peek = h.run("undo.peek", json!({}));
    h.run("branch.checkout", json!({ "target": "main" }));
    assert_eq!(h.run("undo.peek", json!({})), peek);
}

#[test]
fn checkout_with_local_changes_refuses_stashes_or_discards() {
    let mut h = Harness::new("branches-merges");
    // a.txt differs between main and the commit before the merges.
    h.scratch.write("a.txt", "local change\n").unwrap();
    h.scratch.write("untracked.txt", "new\n").unwrap();
    let dirty = h.git().run(["status", "--porcelain"]).unwrap();
    let start = state(&h.git());

    // Git refuses to overwrite a.txt; nothing moves.
    let err = h
        .try_run("branch.checkout", json!({ "target": "HEAD~2" }))
        .unwrap_err();
    assert!(err.contains("a.txt"), "{err}");
    assert_eq!(state(&h.git()), start);
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), dirty);

    // Stash: the switch happens. The change cannot come back onto a file
    // that differs there, so it stays in the stash, the result says so, and
    // the worktree is a clean checkout: no conflict markers, no leftovers.
    let out = h.run(
        "branch.checkout",
        json!({ "target": "HEAD~2", "dirty": "stash" }),
    );
    assert!(out["after"]["branch"].is_null());
    assert_eq!(out["stash_kept"], true);
    assert_eq!(h.git().lines(["stash", "list"]).unwrap().len(), 1);
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");
    assert!(!h.scratch.path().join("untracked.txt").exists());
    // Back where it applies, the stash holds everything that was there.
    h.git().run(["switch", "-q", "main"]).unwrap();
    h.git().run(["stash", "pop", "-q"]).unwrap();
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), dirty);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "local change\n"
    );

    // Stash onto a branch where the file is the same: changes come along.
    h.git().run(["branch", "same", "main"]).unwrap();
    let out = h.run(
        "branch.checkout",
        json!({ "target": "same", "dirty": "stash" }),
    );
    assert_eq!(out["stash_kept"], false);
    assert_eq!(out["after"]["branch"], "same");
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), dirty);
    assert!(h.git().lines(["stash", "list"]).unwrap().is_empty());
    h.git().run(["switch", "-q", "main"]).unwrap();
    h.git().run(["branch", "-D", "same"]).unwrap();

    // Discard: the tracked change goes, and undo brings HEAD and it back.
    let out = h.run(
        "branch.checkout",
        json!({ "target": "HEAD~2", "dirty": "discard" }),
    );
    assert!(out["after"]["branch"].is_null());
    // a.txt does not exist that far back: the local version is gone.
    assert!(!h.scratch.path().join("a.txt").exists());
    assert!(
        h.scratch.path().join("untracked.txt").exists(),
        "untracked files are not in the way and stay"
    );
    h.undo();
    assert_eq!(state(&h.git()), start);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "local change\n"
    );
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), dirty);
}

/// A checkout that carried a local change over in a stash is
/// undone the same way: the change goes back with HEAD, carried in a
/// stash again, as git does it on the twin, and no stash is left.
#[test]
fn undoing_a_stash_and_checkout_carries_the_changes_back() {
    let mut h = Harness::new("linear");
    let base = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\n";
    for copy in [&h.scratch, &h.twin] {
        let git = copy.git();
        copy.write("file.txt", base).unwrap();
        git.run(["add", "file.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "baseline"]).unwrap();
        git.run(["switch", "-q", "-c", "other"]).unwrap();
        copy.write("file.txt", &base.replace("alpha", "other alpha"))
            .unwrap();
        git.run(["commit", "-q", "-am", "other first line"])
            .unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
        // The change on main touches a line the other branch leaves alone.
        copy.write("file.txt", &base.replace("theta", "local theta"))
            .unwrap();
    }
    let start = state(&h.git());
    let dirty = h.git().run(["status", "--porcelain"]).unwrap();
    // Git refuses the plain switch either way, which is why it is carried.
    assert!(h.twin.git().run(["switch", "-q", "other"]).is_err());
    let carry = |git: &Git, to: &str| {
        git.run(["stash", "push", "-q", "--include-untracked"])
            .unwrap();
        git.run(["switch", "-q", to]).unwrap();
        git.run(["stash", "pop", "-q"]).unwrap();
    };
    let worktree = |copy: &Scratch| {
        (
            std::fs::read_to_string(copy.path().join("file.txt")).unwrap(),
            copy.git().run(["status", "--porcelain"]).unwrap(),
            copy.git().lines(["stash", "list"]).unwrap(),
        )
    };

    let out = h.run(
        "branch.checkout",
        json!({ "target": "other", "dirty": "stash" }),
    );
    assert_eq!(out["stash_kept"], false);
    carry(&h.twin.git(), "other");
    h.assert_same("stash and check out");
    assert_eq!(worktree(&h.scratch), worktree(&h.twin));
    assert_eq!(
        worktree(&h.scratch).0,
        "other alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\nlocal theta\n"
    );

    let undone = h.undo();
    assert_eq!(undone["undone"], "Check out other");
    carry(&h.twin.git(), "main");
    h.assert_same("undo of stash and check out");
    assert_eq!(state(&h.git()), start);
    assert_eq!(worktree(&h.scratch), worktree(&h.twin));
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), dirty);
    assert!(h.git().lines(["stash", "list"]).unwrap().is_empty());

    // And done again, the same way once more.
    h.redo();
    carry(&h.twin.git(), "other");
    h.assert_same("redo of stash and check out");
    assert_eq!(worktree(&h.scratch), worktree(&h.twin));
}

#[test]
fn undo_refuses_when_things_moved_in_between() {
    let mut h = Harness::new("branches-merges");
    h.run("branch.checkout", json!({ "target": "feature/a" }));
    // The user went elsewhere by other means.
    h.git().run(["switch", "-q", "feature/b"]).unwrap();
    let there = state(&h.git());
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("HEAD"), "{err}");
    assert_eq!(state(&h.git()), there);

    // A created branch that got a commit is not deleted by undo.
    let mut h = Harness::new("branches-merges");
    h.run("branch.create", json!({ "name": "topic" }));
    h.git().run(["switch", "-q", "topic"]).unwrap();
    h.scratch.write("t.txt", "t\n").unwrap();
    h.git().run(["add", "t.txt"]).unwrap();
    h.git().run(["commit", "-q", "-m", "on topic"]).unwrap();
    h.git().run(["switch", "-q", "main"]).unwrap();
    let tip = h.git().rev_parse("topic").unwrap();
    assert!(h.try_run("undo.last", json!({})).is_err());
    assert_eq!(h.git().rev_parse("topic").unwrap(), tip);
}

#[test]
fn hostile_names_and_targets_are_refused_and_change_nothing() {
    let mut h = Harness::new("branches-merges");
    let before = state(&h.git());
    let long = "x".repeat(5000);
    let names = [
        "",
        "-D",
        "--force",
        "-",
        "a..b",
        "a b",
        "a\nb",
        "a\u{0}b",
        "end.lock",
        "refs/heads/../escape",
        "/leading",
        "trailing/",
        "@{-1}",
        "HEAD",
        "a~1",
        "a^",
        "a:b",
        "a?b",
        "a[b",
        "a\\b",
        long.as_str(),
    ];
    for name in names {
        for (command, params) in [
            ("branch.create", json!({ "name": name })),
            ("branch.create", json!({ "name": "ok", "start": name })),
            (
                "branch.rename",
                json!({ "name": "feature/a", "new_name": name }),
            ),
            ("branch.rename", json!({ "name": name, "new_name": "ok2" })),
            ("branch.delete", json!({ "name": name, "force": true })),
            (
                "branch.set_upstream",
                json!({ "name": name, "upstream": "x" }),
            ),
            (
                "branch.set_upstream",
                json!({ "name": "main", "upstream": name }),
            ),
        ] {
            // `start: "HEAD"`/`"@{-1}"`-style revisions can be legitimate
            // start points; whatever happens, it must not be a panic, and a
            // refusal must leave the repository alone.
            if h.try_run(command, params.clone()).is_ok() {
                for created in ["ok", "ok2"] {
                    let _ = h.git().run(["branch", "-D", created]);
                }
                let _ = h.git().run(["branch", "--unset-upstream", "main"]);
                let _ = h.git().run(["branch", "-m", "ok2", "feature/a"]);
            }
            let _ = h.try_run("undo.last", json!({}));
            while h.run("undo.peek", json!({}))["undo"].is_string() {
                if h.try_run("undo.last", json!({})).is_err() {
                    break;
                }
            }
        }
        assert_eq!(state(&h.git()), before, "after hostile name {name:?}");
    }
    for target in ["", "-h", "--orphan", "no-such-thing", "a\nb", "v1.0^{tree}"] {
        assert!(
            h.try_run("branch.checkout", json!({ "target": target }))
                .is_err(),
            "{target:?}"
        );
        assert_eq!(state(&h.git()), before, "after hostile target {target:?}");
    }
    // Wrong shapes.
    for params in [
        json!({}),
        json!({ "name": 7 }),
        json!({ "name": ["a"] }),
        json!({ "name": "x", "checkout": "yes" }),
        json!({ "target": "main", "dirty": "explode" }),
    ] {
        for command in [
            "branch.create",
            "branch.checkout",
            "branch.rename",
            "branch.delete",
            "branch.set_upstream",
        ] {
            let _ = h.try_run(command, params.clone());
        }
    }
}

#[test]
fn ahead_and_behind_are_what_git_counts() {
    let mut h = Harness::new("linear");
    let git = h.scratch.git();
    let before = state(&git);
    // Upstreams are local branches here, which git tracks all the same:
    // one level, one behind, one ahead, one both, one gone, one without.
    git.run(["branch", "base", "HEAD~2"]).unwrap();
    for (name, at, upstream) in [
        ("level", "HEAD", "main"),
        ("behind", "HEAD~2", "main"),
        ("ahead", "HEAD", "base"),
        ("both", "HEAD~3", "base"),
        ("gone", "HEAD", "base"),
        ("alone", "HEAD", ""),
    ] {
        git.run(["branch", name, at]).unwrap();
        if !upstream.is_empty() {
            git.run(["branch", "-q", "-u", upstream, name]).unwrap();
        }
    }
    // `both` gets a commit of its own, so it is ahead as well as behind.
    git.run(["switch", "-q", "both"]).unwrap();
    h.scratch.write("both.txt", "x").unwrap();
    git.run(["add", "both.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "on both"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    // `gone` follows a branch that is deleted under it.
    git.run(["branch", "vanishing", "HEAD~1"]).unwrap();
    git.run(["branch", "-q", "-u", "vanishing", "gone"])
        .unwrap();
    git.run(["branch", "-q", "-D", "vanishing"]).unwrap();

    let out = h
        .registry
        .run("branch.tracking", &mut h.ctx, &json!({}))
        .unwrap();
    let mut found: Vec<(String, u64, u64, bool)> = out["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["name"].as_str().unwrap().to_owned(),
                b["ahead"].as_u64().unwrap(),
                b["behind"].as_u64().unwrap(),
                b["gone"].as_bool().unwrap(),
            )
        })
        .collect();
    found.sort();
    // What git says, one branch at a time.
    let counted = |branch: &str, upstream: &str| {
        let out = git
            .run([
                "rev-list",
                "--left-right",
                "--count",
                &format!("{branch}...{upstream}"),
            ])
            .unwrap();
        let mut numbers = out.split_whitespace().map(|n| n.parse::<u64>().unwrap());
        (numbers.next().unwrap(), numbers.next().unwrap())
    };
    let (ahead, _) = counted("ahead", "base");
    let (_, behind) = counted("behind", "main");
    let (both_ahead, both_behind) = counted("both", "base");
    assert_eq!(
        found,
        [
            ("ahead".to_owned(), ahead, 0, false),
            ("behind".to_owned(), 0, behind, false),
            ("both".to_owned(), both_ahead, both_behind, false),
            ("gone".to_owned(), 0, 0, true),
        ],
        "level and alone are not listed"
    );
    assert_eq!((ahead, behind, both_ahead, both_behind), (2, 2, 1, 1));
    // Asking changed nothing beyond what the test set up itself.
    let after = state(&git);
    assert!(before.iter().all(|line| after.contains(line)));
}
