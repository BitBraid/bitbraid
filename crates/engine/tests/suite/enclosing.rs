//! A repository inside another one. When git no longer takes the inner one
//! for a repository (its `HEAD` is garbage, or gone), nothing meant for it
//! may reach the one around it: git, left to itself, would go up and act
//! there.
//!
//! Without that, a test that breaks its scratch copy on purpose makes
//! branches in the checkout the tests are run from, and switches it to one
//! of them.

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::{Git, Scratch, build_fixture, fixtures_out_dir};
use serde_json::json;

/// Everything of the outer repository a command could reach.
fn state(git: &Git) -> Vec<String> {
    let mut out = git
        .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
        .unwrap();
    out.push(git.run(["symbolic-ref", "HEAD"]).unwrap());
    out.push(git.run(["config", "--local", "--list"]).unwrap());
    out.push(git.run(["status", "--porcelain"]).unwrap());
    out.push(git.run(["stash", "list"]).unwrap());
    out.push(git.run(["worktree", "list", "--porcelain"]).unwrap());
    out
}

#[test]
fn a_repository_git_no_longer_reads_keeps_its_commands_from_the_one_around_it() {
    // The outer one has what every command below would be glad to find: a
    // branch called main, commits, tags.
    let outer = Scratch::of("branches-merges").unwrap();
    let outer_git = outer.git();
    let inner = outer.path().join("inner");
    let source = build_fixture("linear").unwrap();
    outer_git
        .run([
            "clone",
            "-q",
            source.to_str().unwrap(),
            inner.to_str().unwrap(),
        ])
        .unwrap();
    std::fs::write(inner.join("a.txt"), "edited\n").unwrap();
    let inner_git = Git::new(&inner, fixtures_out_dir().join(".home"));
    inner_git
        .run(["config", "user.name", "Ada Fixture"])
        .unwrap();
    inner_git
        .run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();

    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": inner }))
        .unwrap();
    let before = state(&outer_git);

    let commands = [
        (
            "branch.create",
            json!({ "name": "escaped", "checkout": true }),
        ),
        ("branch.checkout", json!({ "target": "feature/a" })),
        (
            "branch.describe",
            json!({ "name": "main", "description": "escaped" }),
        ),
        ("branch.delete_merged", json!({})),
        (
            "branch.rename",
            json!({ "name": "feature/b", "new_name": "escaped" }),
        ),
        (
            "branch.delete",
            json!({ "name": "feature/a", "force": true }),
        ),
        ("tag.create", json!({ "name": "escaped" })),
        ("tag.delete", json!({ "name": "v1.0" })),
        ("flow.init", json!({})),
        (
            "flow.start",
            json!({ "kind": "feature", "name": "escaped" }),
        ),
        ("stash.push", json!({ "message": "escaped" })),
        (
            "commit.create",
            json!({ "message": "escaped", "allow_empty": true }),
        ),
        ("remote.add", json!({ "name": "escaped", "url": "nowhere" })),
        ("worktree.prune", json!({})),
        (
            "worktree.add",
            json!({ "path": "../escaped-worktree", "new_branch": "escaped-wt" }),
        ),
        ("rerere.enable", json!({ "on": true })),
        ("sign.configure", json!({ "commits": true })),
        ("stage.paths", json!({ "paths": ["a.txt"] })),
        (
            "branch.reset",
            json!({ "target": "HEAD~1", "mode": "hard" }),
        ),
        ("branch.merge", json!({ "target": "feature/a" })),
    ];
    let head = inner.join(".git/HEAD");
    let kept = std::fs::read(&head).unwrap();
    // HEAD that is garbage, HEAD that is gone, and no `.git` at all.
    for broken in 0..3 {
        match broken {
            0 => std::fs::write(&head, "garbage\n").unwrap(),
            1 => std::fs::remove_file(&head).unwrap(),
            _ => std::fs::remove_dir_all(inner.join(".git")).unwrap(),
        }
        for (command, params) in &commands {
            let _ = registry.run(command, &mut ctx, params);
            assert_eq!(
                state(&outer_git),
                before,
                "{command} reached the repository around the broken one ({broken})"
            );
        }
        if broken == 0 {
            std::fs::write(&head, &kept).unwrap();
            // Put right again, it is worked with as before.
            registry
                .run("branch.create", &mut ctx, &json!({ "name": "back" }))
                .expect("the inner repository works again");
            assert_eq!(state(&outer_git), before);
        }
    }
    assert!(!outer.path().join("../escaped-worktree").exists());
}
