//! What the commit box asks of the engine beyond a plain commit, against
//! git on a twin copy: committing without the hooks that can refuse.

use std::path::Path;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
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
}

/// Put a hook in place. It appends its name to `hooks.log` beside the
/// repository's files each time it runs and ends with `code`. Asked to with
/// `GS_PROBE`, it does nothing: that is how it is waited for to be
/// runnable, since a script that was just written can be "busy" for a
/// moment while a child another test's thread forked still holds it open.
/// Where files carry no executable mark (Windows), git runs a hook with the
/// shell it brings along, and being there is enough.
fn hook(repo: &Path, name: &str, code: i32) {
    let hooks = repo.join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let file = hooks.join(name);
    std::fs::write(
        &file,
        format!(
            "#!/bin/sh\n[ -n \"$GS_PROBE\" ] && exit 0\necho {name} >> \"$(git rev-parse --show-toplevel)/hooks.log\"\nexit {code}\n"
        ),
    )
    .unwrap();
    make_runnable(&file);
}

#[cfg(unix)]
fn make_runnable(file: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
    for _ in 0..300 {
        match std::process::Command::new(file)
            .env("GS_PROBE", "1")
            .output()
        {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            other => {
                other.unwrap();
                return;
            }
        }
    }
    panic!("{} stayed busy", file.display());
}

#[cfg(not(unix))]
fn make_runnable(_file: &Path) {}

fn ran(repo: &Path) -> Vec<String> {
    std::fs::read_to_string(repo.join("hooks.log"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The four hooks of a commit: the two that can refuse it do, the two
/// that cannot are there to be seen running.
fn four_hooks(repo: &Path) {
    hook(repo, "pre-commit", 1);
    hook(repo, "commit-msg", 1);
    hook(repo, "prepare-commit-msg", 0);
    hook(repo, "post-commit", 0);
}

#[test]
fn a_commit_without_hooks_leaves_out_what_git_leaves_out_and_says_which() {
    let mut h = Harness::new("linear");
    let twin = Scratch::of("linear").unwrap();
    for copy in [&h.scratch, &twin] {
        four_hooks(copy.path());
        copy.write("a.txt", "2\nhooked\n").unwrap();
        copy.git().run(["add", "--", "a.txt"]).unwrap();
    }

    // With the hooks, the commit is refused by the first of them, in both.
    let refused = h
        .try_run("commit.create", json!({ "message": "blocked" }))
        .unwrap_err();
    assert!(refused.contains("pre-commit"), "{refused}");
    assert!(twin.git().run(["commit", "-q", "-m", "blocked"]).is_err());
    assert_eq!(ran(h.scratch.path()), ran(twin.path()));
    assert_eq!(ran(twin.path()), ["pre-commit"]);

    // Without them it is made, the other two hooks run all the same, and
    // the answer names what was left out.
    let out = h.run(
        "commit.create",
        json!({ "message": "allowed", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!(["pre-commit", "commit-msg"]));
    twin.git()
        .run(["commit", "-q", "--no-verify", "-m", "allowed"])
        .unwrap();
    assert_eq!(ran(h.scratch.path()), ran(twin.path()));
    assert_eq!(
        ran(twin.path()),
        ["pre-commit", "prepare-commit-msg", "post-commit"]
    );
    let shape = |git: &Git| git.run(["log", "-2", "--format=%T %s"]).unwrap();
    assert_eq!(shape(&h.git()), shape(&twin.git()));

    // Amending likewise.
    for copy in [&h.scratch, &twin] {
        copy.write("b.txt", "bb\namended\n").unwrap();
        copy.git().run(["add", "--", "b.txt"]).unwrap();
    }
    assert!(
        h.try_run("commit.amend", json!({ "message": "amended" }))
            .is_err()
    );
    assert!(
        twin.git()
            .run(["commit", "-q", "--amend", "-m", "amended"])
            .is_err()
    );
    let out = h.run(
        "commit.amend",
        json!({ "message": "amended", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!(["pre-commit", "commit-msg"]));
    twin.git()
        .run(["commit", "-q", "--amend", "--no-verify", "-m", "amended"])
        .unwrap();
    assert_eq!(ran(h.scratch.path()), ran(twin.path()));
    assert_eq!(shape(&h.git()), shape(&twin.git()));
    assert_eq!(
        h.git().rev_list_count("HEAD").unwrap(),
        twin.git().rev_list_count("HEAD").unwrap()
    );

    // Undo takes the amended commit back like any other.
    let amended = h.git().rev_parse("HEAD").unwrap();
    h.run("undo.last", json!({}));
    assert_ne!(h.git().rev_parse("HEAD").unwrap(), amended);
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap().trim(),
        "allowed"
    );
}

#[test]
fn only_hooks_that_are_there_and_would_run_are_named_as_skipped() {
    let mut h = Harness::new("linear");
    // No hooks at all: nothing was skipped, and the answer has no list.
    h.scratch.write("a.txt", "2\none\n").unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "one", "stage_all": true, "no_verify": true }),
    );
    assert!(out.get("skipped_hooks").is_none(), "{out}");

    // One that git would not run (not executable) is not one either.
    // Where files carry no such mark (Windows), git runs every hook that
    // is there, and both are named.
    hook(h.scratch.path(), "commit-msg", 1);
    let idle = h.scratch.path().join(".git/hooks/pre-commit");
    std::fs::write(&idle, "#!/bin/sh\nexit 1\n").unwrap();
    h.scratch.write("a.txt", "2\ntwo\n").unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "two", "stage_all": true, "no_verify": true }),
    );
    if cfg!(unix) {
        assert_eq!(out["skipped_hooks"], json!(["commit-msg"]));
    } else {
        assert_eq!(out["skipped_hooks"], json!(["pre-commit", "commit-msg"]));
    }

    // A commit that runs its hooks names none, whatever is there. Where
    // git runs the hook that is not marked (Windows), it lets the commit
    // through.
    std::fs::remove_file(h.scratch.path().join(".git/hooks/commit-msg")).unwrap();
    if !cfg!(unix) {
        std::fs::write(&idle, "#!/bin/sh\nexit 0\n").unwrap();
    }
    h.scratch.write("a.txt", "2\nthree\n").unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "three", "stage_all": true }),
    );
    assert!(out.get("skipped_hooks").is_none(), "{out}");

    // Hooks kept somewhere else (core.hooksPath) are the ones looked at.
    let elsewhere = h.scratch.path().join("my-hooks");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::rename(&idle, elsewhere.join("pre-commit")).unwrap();
    make_runnable(&elsewhere.join("pre-commit"));
    h.git()
        .run(["config", "core.hooksPath", "my-hooks"])
        .unwrap();
    h.scratch.write("a.txt", "2\nfour\n").unwrap();
    h.git().run(["add", "--", "a.txt"]).unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "four", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!(["pre-commit"]));
}

#[test]
fn hostile_values_for_the_hooks_switch_are_refused_without_a_commit() {
    let mut h = Harness::new("linear");
    h.scratch.write("a.txt", "2\nstaged\n").unwrap();
    h.git().run(["add", "--", "a.txt"]).unwrap();
    let before = h.git().rev_parse("HEAD").unwrap();
    for hostile in [
        json!("yes"),
        json!(1),
        json!([true]),
        json!({ "no_verify": true }),
        json!("\u{0}"),
    ] {
        for id in ["commit.create", "commit.amend"] {
            let err = h
                .try_run(id, json!({ "message": "m", "no_verify": hostile }))
                .unwrap_err();
            assert!(err.contains("no_verify"), "{id}: {err}");
        }
    }
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), before);
    // A hooks directory that is not one does not bring the commit down.
    h.git()
        .run(["config", "core.hooksPath", "/nonexistent/\u{1b}[31m"])
        .unwrap();
    let out = h.run(
        "commit.create",
        json!({ "message": "still made", "no_verify": true }),
    );
    assert!(out.get("skipped_hooks").is_none(), "{out}");
}
