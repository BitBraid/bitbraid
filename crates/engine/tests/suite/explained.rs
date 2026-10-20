//! Failures that are not git's own are said to be what they are: a hook
//! of the repository that refused, with its name, what it ended with and
//! what it said; and a signature that could not be made in the middle of
//! a merge, a rebase, a cherry-pick or a revert, with what is wrong with
//! the key and what state that leaves.
//!
//! Hooks are scripts in a directory of the test's own that
//! `core.hooksPath` points at, and the signing program is a script that
//! fails the way gpg does without a key; both are set in the scratch
//! copy's own configuration, so nothing of the developer's is asked.
//! Git runs a script by its first line, on Windows too, with the `sh` that
//! Git for Windows brings along.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("branches-merges").unwrap();
        let twin = Scratch::of("branches-merges").unwrap();
        for copy in [&scratch, &twin] {
            let git = copy.git();
            for (key, value) in [
                ("user.name", "Ada Fixture"),
                ("user.email", "ada@fixture.invalid"),
                ("commit.gpgsign", "false"),
                ("tag.gpgsign", "false"),
            ] {
                git.run(["config", key, value]).unwrap();
            }
            // Something to merge, pick, rebase and revert: the fixture's
            // branches are all merged. `topic` has a commit main has
            // not, and main one that `topic` has not, in files of their
            // own. Made with the fixtures' clock, so the two copies have
            // the same commits.
            git.run(["switch", "-q", "-c", "topic"]).unwrap();
            copy.write("t.txt", "on topic\n").unwrap();
            git.run(["add", "t.txt"]).unwrap();
            git.run(["commit", "-q", "-m", "topic work"]).unwrap();
            git.run(["switch", "-q", "main"]).unwrap();
            copy.write("m.txt", "on main\n").unwrap();
            git.run(["add", "m.txt"]).unwrap();
            git.run(["commit", "-q", "-m", "main work"]).unwrap();
        }
        // The hooks that run are the ones of this directory, whatever
        // the developer has set for themselves.
        let hooks = scratch.path().join(".git/test-hooks");
        std::fs::create_dir_all(&hooks).unwrap();
        scratch
            .git()
            .run(["config", "core.hooksPath", hooks.to_str().unwrap()])
            .unwrap();
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
    /// Install a hook that says something and ends with a status.
    fn hook(&self, name: &str, says: &str, status: i32) -> PathBuf {
        let path = self.scratch.path().join(".git/test-hooks").join(name);
        let body = if says.is_empty() {
            format!("#!/bin/sh\nexit {status}\n")
        } else {
            format!("#!/bin/sh\necho '{says}' >&2\nexit {status}\n")
        };
        write_script(&path, &body);
        path
    }
    fn unhook(&self, name: &str) {
        let _ = std::fs::remove_file(self.scratch.path().join(".git/test-hooks").join(name));
    }
    /// Have every commit signed, by a program that has no key: it fails
    /// as gpg does.
    fn break_signing(&self) {
        let program = self.scratch.path().join(".git/no-key-gpg.sh");
        write_script(
            &program,
            "#!/bin/sh\necho 'gpg: skipped \"KEY\": No secret key' >&2\n\
             echo '[GNUPG:] INV_SGNR 9 KEY' >&2\n\
             echo 'gpg: signing failed: No secret key' >&2\nexit 2\n",
        );
        let git = self.git();
        for (key, value) in [
            ("gpg.program", program.to_str().unwrap()),
            ("gpg.format", "openpgp"),
            ("user.signingkey", "KEY"),
            ("commit.gpgsign", "true"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
    }
    fn mend_signing(&self) {
        self.git()
            .run(["config", "commit.gpgsign", "false"])
            .unwrap();
    }
    fn head(&self) -> String {
        self.git().rev_parse("HEAD").unwrap()
    }
    fn status(&self) -> String {
        self.git().run(["status", "--porcelain=v2"]).unwrap()
    }
    fn operation(&mut self) -> Value {
        self.run("op.status", json!({}))["operation"].clone()
    }
    /// Tree and parents of HEAD here and in the twin: the same commit but
    /// for when it was made.
    fn assert_same_commit(&self, what: &str) {
        let shape = |git: &Git| {
            (
                git.rev_parse("HEAD^{tree}").unwrap(),
                git.run(["log", "-1", "--format=%P|%s"]).unwrap(),
                git.run(["status", "--porcelain=v2"]).unwrap(),
            )
        };
        assert_eq!(shape(&self.git()), shape(&self.twin.git()), "{what}");
    }
}

/// Write a script and make it one that runs. Written through a child
/// process: a file this process has open for writing while another
/// thread starts a program is "busy" to the one that then runs it.
#[cfg(unix)]
fn write_script(path: &std::path::Path, body: &str) {
    use std::{io::Write as _, os::unix::fs::PermissionsExt as _};
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("cat > \"$0\"")
        .arg(path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(body.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Elsewhere (Windows) files carry no mark that lets them run, and a child
/// another test starts does not hold a file that is being written.
#[cfg(not(unix))]
fn write_script(path: &std::path::Path, body: &str) {
    std::fs::write(path, body).unwrap();
}

#[test]
fn a_hook_that_refuses_a_commit_is_named_with_what_it_said() {
    let mut h = Harness::new();
    let head = h.head();
    h.scratch.write("a.txt", "a1\nchanged\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["a.txt"] }));
    let staged = h.status();

    h.hook("pre-commit", "lint: a.txt has a tab in it", 1);
    let err = h
        .try_run("commit.create", json!({ "message": "blocked" }))
        .unwrap_err();
    assert!(err.contains("the pre-commit hook"), "{err}");
    assert!(err.contains("stopped this"), "{err}");
    assert!(err.contains("status 1"), "{err}");
    assert!(err.contains("lint: a.txt has a tab in it"), "{err}");
    assert!(err.contains("Git itself had nothing against it"), "{err}");
    assert!(err.contains("--no-verify"), "{err}");
    assert_eq!((h.head(), h.status()), (head.clone(), staged.clone()));
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // Another hook, another status, and one that says nothing.
    h.unhook("pre-commit");
    h.hook("commit-msg", "", 3);
    let err = h
        .try_run("commit.create", json!({ "message": "blocked again" }))
        .unwrap_err();
    assert!(err.contains("the commit-msg hook"), "{err}");
    assert!(err.contains("status 3"), "{err}");
    assert!(err.contains("printed nothing"), "{err}");
    assert_eq!(h.head(), head);
    // Amending runs the same hooks and says the same.
    let err = h
        .try_run("commit.amend", json!({ "message": "amended" }))
        .unwrap_err();
    assert!(err.contains("the commit-msg hook"), "{err}");
    assert_eq!(h.head(), head);

    // Without the checks, as the message says, it goes through; the twin
    // commits with git.
    h.hook("pre-commit", "still against it", 1);
    h.run(
        "commit.create",
        json!({ "message": "through", "no_verify": true }),
    );
    h.twin.write("a.txt", "a1\nchanged\n").unwrap();
    h.twin
        .git()
        .run(["commit", "-q", "-am", "through"])
        .unwrap();
    h.assert_same_commit("committed without the hooks");
}

#[test]
fn git_failing_by_itself_is_not_blamed_on_a_hook() {
    let mut h = Harness::new();
    // Hooks that are content, and nothing to commit: git's own refusal.
    h.hook("pre-commit", "", 0);
    h.hook("commit-msg", "", 0);
    let err = h
        .try_run("commit.create", json!({ "message": "nothing staged" }))
        .unwrap_err();
    assert!(!err.contains("hook"), "{err}");
    // A hook that fails after the fact does not fail a commit at all:
    // git does not pass its status on.
    h.hook("post-commit", "the post-commit hook grumbles", 9);
    h.scratch.write("a.txt", "a1\nmore\n").unwrap();
    h.run(
        "commit.create",
        json!({ "message": "made", "stage_all": true }),
    );
    assert_eq!(h.git().run(["log", "-1", "--format=%s"]).unwrap(), "made\n");
    // A branch that is not there: git's refusal, with a hook installed
    // that would have run had the checkout happened.
    h.hook("post-checkout", "never runs", 1);
    let err = h
        .try_run("branch.checkout", json!({ "target": "no-such-branch" }))
        .unwrap_err();
    assert!(!err.contains("hook"), "{err}");
}

#[test]
fn a_hook_that_fails_after_a_checkout_does_not_undo_the_checkout() {
    let mut h = Harness::new();
    let before = h.head();
    h.hook("post-checkout", "git-lfs was not found on your path", 2);

    // Git ends badly here although HEAD has moved; that is a checkout
    // that happened, and it is journalled like one.
    let out = h.run("branch.checkout", json!({ "target": "feature/a" }));
    assert_eq!(out["after"]["branch"], "feature/a");
    // The answer says what the hook had against it, for whoever asked.
    let said = out["hook"].as_str().unwrap();
    assert!(
        said.contains("post-checkout") && said.contains("git-lfs was not found"),
        "{said}"
    );
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "feature/a\n"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Check out feature/a");
    // What the hook said is with the command in the activity log.
    let log = h.run("activity.list", json!({ "text": "git-lfs was not found" }));
    assert_eq!(log["entries"].as_array().unwrap().len(), 1, "{log}");
    assert_eq!(log["entries"][0]["origin"], "branch.checkout");
    assert_ne!(log["entries"][0]["code"], 0);

    // Undo goes back through the same hook, and redo forward again.
    h.run("undo.last", json!({}));
    assert_eq!(h.head(), before);
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "main\n"
    );
    h.run("redo.last", json!({}));
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "feature/a\n"
    );
    // A new branch that is checked out, a detached commit.
    let out = h.run("branch.create", json!({ "name": "made", "checkout": true }));
    assert_eq!(out["checked_out"], true);
    assert!(out["hook"].as_str().unwrap().contains("post-checkout"));
    // A hook that has nothing against it says nothing.
    h.hook("post-checkout", "", 0);
    let out = h.run("branch.checkout", json!({ "target": "main" }));
    assert!(out["hook"].is_null(), "{out}");
    h.hook("post-checkout", "git-lfs was not found on your path", 2);
    let out = h.run("branch.checkout", json!({ "target": before }));
    assert!(out["after"]["branch"].is_null());
    assert_eq!(h.head(), before);
}

#[test]
fn hooks_that_refuse_a_merge_a_rebase_and_a_push_are_named() {
    let mut h = Harness::new();
    let head = h.head();

    // A rebase that its hook does not let begin: nothing is in progress.
    h.hook("pre-rebase", "not on a Friday", 1);
    let err = h
        .try_run("branch.rebase", json!({ "onto": "topic" }))
        .unwrap_err();
    assert!(err.contains("the pre-rebase hook"), "{err}");
    assert!(err.contains("not on a Friday"), "{err}");
    assert!(err.contains("git rebase --no-verify"), "{err}");
    assert_eq!(h.head(), head);
    assert!(h.operation().is_null());
    h.unhook("pre-rebase");

    // A merge whose commit a hook refuses: the merge is made and not
    // committed, and says who stopped it and that it can go on.
    h.hook("pre-merge-commit", "the tests have not run", 4);
    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_ff" }),
    );
    let stopped = out["stopped"].as_str().unwrap();
    assert!(stopped.contains("the pre-merge-commit hook"), "{stopped}");
    assert!(stopped.contains("status 4"), "{stopped}");
    assert!(stopped.contains("the tests have not run"), "{stopped}");
    assert!(stopped.contains("left where it stopped"), "{stopped}");
    assert_eq!(h.head(), head);
    assert_eq!(h.operation()["kind"], "merge");
    // With the hook out of the way it is finished, as git finishes it.
    h.unhook("pre-merge-commit");
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    h.twin
        .git()
        .run(["merge", "-q", "--no-ff", "--no-edit", "topic"])
        .unwrap();
    h.assert_same_commit("the merge the hook held up");

    // A push that `pre-push` refuses: the remote has nothing new.
    let remote = h.scratch.path().with_extension("remote.git");
    let _ = std::fs::remove_dir_all(&remote);
    let git = h.git();
    git.run(["init", "-q", "--bare", remote.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", remote.to_str().unwrap()])
        .unwrap();
    h.hook("pre-push", "protected: ask first", 1);
    let err = h
        .try_run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main" }),
        )
        .unwrap_err();
    assert!(err.contains("the pre-push hook"), "{err}");
    assert!(err.contains("protected: ask first"), "{err}");
    assert!(err.contains("git push --no-verify"), "{err}");
    let there = Git::new(&remote, fixtures_out_dir().join(".home"));
    assert!(there.lines(["for-each-ref"]).unwrap().is_empty());
    // Without the hook the same push goes through.
    h.unhook("pre-push");
    h.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    );
    assert_eq!(there.rev_parse("refs/heads/main").unwrap(), h.head());
    let _ = std::fs::remove_dir_all(&remote);
}

/// A `pre-push` hook writes to git's standard output: git-lfs, refusing
/// a push of files somebody else has locked (`lfs.<url>.locksverify`),
/// lists them there and says only its last line on standard error
/// (git-lfs 3.8.0 against GitLab). Which files, and whose, is what
/// the refusal is about, so it is part of what the error says.
#[test]
fn a_pre_push_hook_that_lists_locked_files_on_its_output_is_heard() {
    let mut h = Harness::new();
    let remote = h.scratch.path().with_extension("locked.git");
    let _ = std::fs::remove_dir_all(&remote);
    let git = h.git();
    git.run(["init", "-q", "--bare", remote.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", remote.to_str().unwrap()])
        .unwrap();
    write_script(
        &h.scratch.path().join(".git/test-hooks/pre-push"),
        "#!/bin/sh\n\
         echo 'Unable to push locked files:'\n\
         echo '* assets/a.bin - Bea Holder (refs: main)'\n\
         echo 'Cannot update locked files.' >&2\n\
         exit 2\n",
    );
    let err = h
        .try_run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main" }),
        )
        .unwrap_err();
    assert!(err.contains("the pre-push hook"), "{err}");
    assert!(err.contains("ended with status 2"), "{err}");
    assert!(
        err.contains("Unable to push locked files: * assets/a.bin - Bea Holder (refs: main) Cannot update locked files."),
        "{err}"
    );
    let there = Git::new(&remote, fixtures_out_dir().join(".home"));
    assert!(there.lines(["for-each-ref"]).unwrap().is_empty());
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn hooks_that_would_refuse_a_merge_are_left_out_only_when_asked() {
    let mut h = Harness::new();
    let head = h.head();
    h.hook("pre-merge-commit", "the tests have not run", 4);
    h.hook("commit-msg", "no ticket in the message", 1);
    // A hook that cannot refuse leaves a mark when it runs.
    let mark = h.scratch.path().join(".git/post-merge-ran");
    write_script(
        &h.scratch.path().join(".git/test-hooks/post-merge"),
        &format!("#!/bin/sh\n: > '{}'\n", mark.display()),
    );

    // Not asked for, and asked not to: the hook holds the merge up.
    for params in [
        json!({ "target": "topic", "mode": "no_ff" }),
        json!({ "target": "topic", "mode": "no_ff", "no_verify": false }),
    ] {
        let out = h.run("branch.merge", params);
        let stopped = out["stopped"].as_str().unwrap();
        assert!(stopped.contains("the pre-merge-commit hook"), "{stopped}");
        assert!(stopped.contains("git merge --no-verify"), "{stopped}");
        assert!(out.get("skipped_hooks").is_none(), "{out}");
        assert_eq!(h.head(), head);
        h.run("op.abort", json!({}));
        assert!(h.operation().is_null());
        assert!(!mark.exists());
    }

    // Asked for: merged as git merges it, and the two that were left out
    // are named. What cannot refuse ran all the same.
    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_ff", "no_verify": true }),
    );
    assert!(out.get("stopped").is_none(), "{out}");
    assert_eq!(
        out["skipped_hooks"],
        json!(["pre-merge-commit", "commit-msg"])
    );
    assert!(mark.exists(), "post-merge ran");
    h.twin
        .git()
        .run(["merge", "-q", "--no-ff", "--no-edit", "topic"])
        .unwrap();
    h.assert_same_commit("merged without the hooks that refuse");

    // It is undone like any merge, and only the hooks that are there are
    // named the next time.
    h.run("undo.last", json!({}));
    assert_eq!(h.head(), head);
    h.unhook("pre-merge-commit");
    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_ff", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!(["commit-msg"]));
    h.assert_same_commit("merged again");
}

#[test]
fn the_hook_that_would_refuse_a_rebase_is_left_out_only_when_asked() {
    let mut h = Harness::new();
    let head = h.head();
    h.hook("pre-rebase", "not on a Friday", 1);
    for params in [
        json!({ "onto": "topic" }),
        json!({ "onto": "topic", "no_verify": false }),
    ] {
        let err = h.try_run("branch.rebase", params).unwrap_err();
        assert!(err.contains("the pre-rebase hook"), "{err}");
        assert_eq!(h.head(), head);
        assert!(h.operation().is_null());
    }
    let out = h.run(
        "branch.rebase",
        json!({ "onto": "topic", "no_verify": true }),
    );
    assert!(out.get("stopped").is_none(), "{out}");
    assert_eq!(out["skipped_hooks"], json!(["pre-rebase"]));
    h.twin.git().run(["rebase", "-q", "topic"]).unwrap();
    h.assert_same_commit("rebased without the hook that refuses");

    // A plan of one's own is a rebase too, with the same hook before it.
    let tip = h.head();
    let plan = |no_verify: Option<bool>| {
        let mut params = json!({
            "first": tip,
            "steps": [{ "id": tip, "action": "reword", "message": "main work, said again" }],
        });
        if let Some(no_verify) = no_verify {
            params["no_verify"] = json!(no_verify);
        }
        params
    };
    for asked in [None, Some(false)] {
        let err = h.try_run("rebase.interactive", plan(asked)).unwrap_err();
        assert!(err.contains("pre-rebase"), "{err}");
        assert_eq!(h.head(), tip);
        assert!(h.operation().is_null());
    }
    let out = h.run("rebase.interactive", plan(Some(true)));
    assert!(out.get("stopped").is_none(), "{out}");
    assert_eq!(out["skipped_hooks"], json!(["pre-rebase"]));
    h.twin
        .git()
        .run(["commit", "-q", "--amend", "-m", "main work, said again"])
        .unwrap();
    h.assert_same_commit("reworded without the hook that refuses");

    // No such hook: nothing to name, and nothing goes differently.
    h.unhook("pre-rebase");
    h.run("undo.last", json!({}));
    assert_eq!(h.head(), tip);
    let out = h.run("rebase.interactive", plan(Some(true)));
    assert_eq!(out["skipped_hooks"], json!([]));
    h.assert_same_commit("reworded again");
}

#[test]
fn the_hook_that_would_refuse_a_push_is_left_out_only_when_asked() {
    let mut h = Harness::new();
    let remote = h.scratch.path().with_extension("unverified.git");
    let _ = std::fs::remove_dir_all(&remote);
    let git = h.git();
    git.run(["init", "-q", "--bare", remote.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", remote.to_str().unwrap()])
        .unwrap();
    let there = Git::new(&remote, fixtures_out_dir().join(".home"));
    h.hook("pre-push", "protected: ask first", 1);
    for params in [
        json!({ "remote": "origin", "branch": "main" }),
        json!({ "remote": "origin", "branch": "main", "no_verify": false }),
    ] {
        let err = h.try_run("remote.push", params).unwrap_err();
        assert!(err.contains("the pre-push hook"), "{err}");
        assert!(there.lines(["for-each-ref"]).unwrap().is_empty());
    }
    let out = h.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!(["pre-push"]));
    assert_eq!(there.rev_parse("refs/heads/main").unwrap(), h.head());
    // Still refused when not asked for, with the same hook.
    let err = h
        .try_run(
            "remote.push",
            json!({ "remote": "origin", "branch": "topic" }),
        )
        .unwrap_err();
    assert!(err.contains("the pre-push hook"), "{err}");
    assert!(there.rev_parse("refs/heads/topic").is_err());
    // No such hook: nothing to name.
    h.unhook("pre-push");
    let out = h.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "topic", "no_verify": true }),
    );
    assert_eq!(out["skipped_hooks"], json!([]));
    assert_eq!(
        there.rev_parse("refs/heads/topic").unwrap(),
        h.git().rev_parse("topic").unwrap()
    );
    let _ = std::fs::remove_dir_all(&remote);
}

#[test]
fn the_hooks_of_a_mailbox_are_listed_named_and_left_out_only_when_asked() {
    let mut h = Harness::new();
    let head = h.head();
    // The commit of `topic` as a mailbox, beside the repository.
    let mailbox = h.scratch.path().with_extension("topic.mbox");
    let patch = h
        .git()
        .run(["format-patch", "--stdout", "main..topic"])
        .unwrap();
    std::fs::write(&mailbox, patch).unwrap();
    let file = mailbox.to_str().unwrap().to_owned();

    // The two that can refuse a patch are among the hooks that will run.
    h.hook("applypatch-msg", "no patches by mail", 1);
    h.hook("pre-applypatch", "it does not build", 3);
    let listed = h.run("hook.list", json!({}));
    let names: Vec<&str> = listed["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hook| hook["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["applypatch-msg", "pre-applypatch"]);

    // Each of them refusing is said to be that hook, with what it said;
    // the mailbox is left in progress and nothing is committed.
    for (hook, said, params) in [
        (
            "applypatch-msg",
            "no patches by mail",
            json!({ "file": file }),
        ),
        (
            "pre-applypatch",
            "it does not build",
            json!({ "file": file, "no_verify": false }),
        ),
    ] {
        let out = h.run("patch.mailbox", params);
        let stopped = out["stopped"].as_str().unwrap();
        assert!(stopped.contains(&format!("the {hook} hook")), "{stopped}");
        assert!(stopped.contains(said), "{stopped}");
        assert!(stopped.contains("git am --no-verify"), "{stopped}");
        assert!(stopped.contains("left where it stopped"), "{stopped}");
        assert!(out.get("skipped_hooks").is_none(), "{out}");
        assert_eq!(h.head(), head);
        assert_eq!(h.operation()["kind"], "mailbox");
        h.run("op.abort", json!({}));
        assert!(h.operation().is_null());
        assert_eq!(h.status(), "");
        // The next round meets the second hook.
        h.unhook("applypatch-msg");
    }

    // Asked for: applied as git applies it, and both are named.
    h.hook("applypatch-msg", "no patches by mail", 1);
    let out = h.run("patch.mailbox", json!({ "file": file, "no_verify": true }));
    assert!(out.get("stopped").is_none(), "{out}");
    assert_eq!(out["commits"], 1);
    assert_eq!(
        out["skipped_hooks"],
        json!(["applypatch-msg", "pre-applypatch"])
    );
    h.twin.git().run(["am", "-q", file.as_str()]).unwrap();
    h.assert_same_commit("the mailbox applied without the hooks that refuse");
    let _ = std::fs::remove_file(&mailbox);
}

/// Whether the git the tests run is `major.minor` or later.
fn git_at_least(major: u32, minor: u32) -> bool {
    let version = bitbraid_testkit::oracle::git_version().unwrap();
    let numbers: Vec<u32> = version
        .trim_start_matches("git version ")
        .split('.')
        .take(2)
        .map_while(|number| number.parse().ok())
        .collect();
    numbers.as_slice() >= [major, minor].as_slice()
}

/// What a signature that could not be made is said to be.
fn assert_explains_the_key(said: &str) {
    assert!(said.contains("no secret key"), "{said}");
    assert!(said.contains("\"KEY\""), "{said}");
    assert!(said.contains("user.signingkey"), "{said}");
    // Not what git printed.
    assert!(!said.contains("gpg failed to sign the data"), "{said}");
    assert!(!said.contains("failed to write commit object"), "{said}");
}

#[test]
fn a_merge_that_cannot_be_signed_says_why_and_can_be_finished() {
    let mut h = Harness::new();
    let head = h.head();
    h.break_signing();

    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_ff" }),
    );
    let stopped = out["stopped"].as_str().unwrap();
    assert_explains_the_key(stopped);
    assert!(
        stopped.contains("the merge could not make its commit"),
        "{stopped}"
    );
    assert!(stopped.contains("continue, or abort"), "{stopped}");
    assert_eq!(h.head(), head);
    assert_eq!(h.operation()["kind"], "merge");
    assert!(h.operation()["conflicts"].as_array().unwrap().is_empty());

    // Going on while it is still wrong says the same, and stays put.
    let err = h.try_run("op.continue", json!({})).unwrap_err();
    assert_explains_the_key(&err);
    assert!(err.contains("the merge could not make its commit"), "{err}");
    assert_eq!(h.head(), head);
    assert_eq!(h.operation()["kind"], "merge");

    // Put right, it is finished; the twin merged with git, unsigned.
    h.mend_signing();
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    h.twin
        .git()
        .run(["merge", "-q", "--no-ff", "--no-edit", "topic"])
        .unwrap();
    h.assert_same_commit("merged once it could be");
    // And undone as the merge it is.
    h.run("undo.last", json!({}));
    assert_eq!(h.head(), head);

    // Or given up: back where it began, with nothing left behind.
    h.break_signing();
    let out = h.run(
        "branch.merge",
        json!({ "target": "topic", "mode": "no_ff" }),
    );
    assert!(out["stopped"].is_string());
    h.run("op.abort", json!({}));
    assert_eq!(h.head(), head);
    assert_eq!(h.status(), "");
    assert!(h.operation().is_null());
}

#[test]
fn a_cherry_pick_and_a_rebase_that_cannot_be_signed_say_why() {
    let mut h = Harness::new();
    let head = h.head();
    let pick = h.git().rev_parse("topic").unwrap();
    h.break_signing();

    let out = h.run("commit.cherry_pick", json!({ "commits": [pick] }));
    let stopped = out["stopped"].as_str().unwrap();
    assert_explains_the_key(stopped);
    assert!(
        stopped.contains("the cherry-pick could not make its commit"),
        "{stopped}"
    );
    assert_eq!(h.operation()["kind"], "cherry_pick");
    assert_eq!(h.head(), head);
    // Mended and continued: the commit git would have picked.
    h.mend_signing();
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    h.twin.git().run(["cherry-pick", &pick]).unwrap();
    h.assert_same_commit("picked once it could be signed");
    h.run("undo.last", json!({}));
    h.twin
        .git()
        .run(["reset", "-q", "--hard", "HEAD~1"])
        .unwrap();
    assert_eq!(h.head(), head);

    // A rebase stops at the first commit it has to make.
    h.break_signing();
    h.run("branch.checkout", json!({ "target": "topic" }));
    let tip = h.head();
    let out = h.run("branch.rebase", json!({ "onto": "main" }));
    let stopped = out["stopped"].as_str().unwrap();
    assert_explains_the_key(stopped);
    assert!(
        stopped.contains("the rebase could not make its commit"),
        "{stopped}"
    );
    assert_eq!(h.operation()["kind"], "rebase");
    let err = h.try_run("op.continue", json!({})).unwrap_err();
    // A git before 2.56 (2.45 and 2.55 were tried) puts a pick whose
    // commit it could not make back in its list and leaves what the pick
    // staged without a message: going on, it refuses over the staged
    // changes and does not try to sign again. A later git stops there as
    // for a conflict, and going on meets the key once more.
    if git_at_least(2, 56) {
        assert_explains_the_key(&err);
    } else {
        assert!(err.contains("git rebase --continue"), "{err}");
    }
    h.run("op.abort", json!({}));
    assert_eq!(h.head(), tip);
    assert_eq!(h.status(), "");
    // Mended, the whole rebase is what git makes of it.
    h.mend_signing();
    let out = h.run("branch.rebase", json!({ "onto": "main" }));
    assert!(out["stopped"].is_null(), "{out}");
    let twin = h.twin.git();
    twin.run(["switch", "-q", "topic"]).unwrap();
    twin.run(["rebase", "-q", "main"]).unwrap();
    h.assert_same_commit("rebased once it could be signed");
}

#[test]
fn a_revert_that_cannot_be_signed_says_why_and_changes_nothing() {
    let mut h = Harness::new();
    let head = h.head();
    // Local work that is nobody's business stays through it.
    h.scratch.write("untracked.txt", "mine\n").unwrap();
    let before = h.status();
    h.break_signing();

    // Git leaves a revert whose commit it could not make with the
    // revert staged and nothing to continue or abort. That is taken
    // back: refused means nothing was changed.
    let err = h
        .try_run("commit.revert", json!({ "commits": [head] }))
        .unwrap_err();
    assert_explains_the_key(&err);
    assert!(err.contains("nothing was changed"), "{err}");
    assert_eq!(h.head(), head);
    assert_eq!(h.status(), before);
    assert!(h.operation().is_null());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // Mended, it reverts as git does.
    h.mend_signing();
    h.run("commit.revert", json!({ "commits": [head] }));
    h.twin.git().run(["revert", "--no-edit", &head]).unwrap();
    std::fs::remove_file(h.scratch.path().join("untracked.txt")).unwrap();
    h.assert_same_commit("reverted once it could be signed");

    // A plain commit says the same.
    h.break_signing();
    h.scratch.write("a.txt", "a1\nto sign\n").unwrap();
    let err = h
        .try_run(
            "commit.create",
            json!({ "message": "signed", "stage_all": true }),
        )
        .unwrap_err();
    assert_explains_the_key(&err);
}
