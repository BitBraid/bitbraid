//! Rerere against git on twin scratch copies: a conflict resolved once is
//! resolved by git the next time, the files it did that for are told from
//! the ones still to do, and forgetting and switching it off do what
//! `git rerere forget` and `rerere.enabled` do.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Two lines of history that conflict in `a.txt` and `b.txt`, which both
/// changed, and in `README.md`, which main deleted and topic changed: a
/// conflict rerere keeps no record of.
fn diverge(copy: &Scratch, rerere: Option<bool>) {
    let git = copy.git();
    // Commands run git with the real environment: what a merge needs is
    // set here, and whatever a global configuration sets for rerere is not
    // left to decide.
    for (key, value) in [
        ("user.name", "Ada Fixture"),
        ("user.email", "ada@fixture.invalid"),
        ("commit.gpgsign", "false"),
        ("merge.conflictStyle", "merge"),
        ("rerere.autoUpdate", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    if let Some(on) = rerere {
        git.run([
            "config",
            "rerere.enabled",
            if on { "true" } else { "false" },
        ])
        .unwrap();
    }
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("a.txt", "topic's a\n").unwrap();
    copy.write("b.txt", "topic's b\n").unwrap();
    copy.write("README.md", "topic's readme\n").unwrap();
    copy.write("calm.txt", "only on topic\n").unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write("a.txt", "main's a\n").unwrap();
    copy.write("b.txt", "main's b\n").unwrap();
    git.run(["rm", "-q", "README.md"]).unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "main"]).unwrap();
}

fn status(git: &Git) -> String {
    git.run(["status", "--porcelain"]).unwrap()
}

/// The first-parent history as (tree, number of parents, subject).
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

/// What rerere has under `rr-cache`: for each conflict, the names of the
/// files it keeps, without the copy of the file as it was last seen.
fn record(copy: &Scratch) -> Vec<String> {
    let dir = copy.path().join(".git/rr-cache");
    let mut out = Vec::new();
    for conflict in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let mut names: Vec<String> = std::fs::read_dir(conflict.path())
            .unwrap()
            .flatten()
            .map(|file| file.file_name().to_string_lossy().into_owned())
            .filter(|name| name != "thisimage")
            .collect();
        names.sort();
        out.push(format!(
            "{} {}",
            conflict.file_name().to_string_lossy(),
            names.join(",")
        ));
    }
    out.sort();
    out
}

fn resolutions(copy: &Scratch) -> usize {
    record(copy)
        .iter()
        .map(|line| line.matches("postimage").count())
        .sum()
}

impl Harness {
    fn with(rerere: Option<bool>) -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        diverge(&scratch, rerere);
        diverge(&twin, rerere);
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
    fn new() -> Self {
        Self::with(Some(true))
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
    fn refused(&mut self, id: &str, params: Value, because: &str) {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(because), "{id} {params}: {err}");
    }
    fn file(&self, rel: &str) -> PathBuf {
        self.scratch.path().join(rel)
    }
    fn both(&self, f: impl Fn(&Scratch)) {
        f(&self.scratch);
        f(&self.twin);
    }
    fn assert_same(&self, what: &str) {
        let (ours, twin) = (self.git(), self.twin.git());
        assert_eq!(shape(&ours), shape(&twin), "{what}: history");
        assert_eq!(status(&ours), status(&twin), "{what}: status");
        assert_eq!(
            ours.run(["ls-files", "--stage"]).unwrap(),
            twin.run(["ls-files", "--stage"]).unwrap(),
            "{what}: index"
        );
        for file in ["a.txt", "b.txt"] {
            assert_eq!(
                std::fs::read(self.file(file)).ok(),
                std::fs::read(self.twin.path().join(file)).ok(),
                "{what}: {file}"
            );
        }
        assert_eq!(record(&self.scratch), record(&self.twin), "{what}: record");
    }
    /// The conflicted paths with whether each is reported as reused, from
    /// the list of conflicts.
    fn reused(&mut self) -> Vec<(String, bool)> {
        let listed = self.run("conflict.list", json!({}));
        let pairs = |conflicts: &Value| -> Vec<(String, bool)> {
            conflicts
                .as_array()
                .unwrap()
                .iter()
                .map(|conflict| {
                    (
                        conflict["path"].as_str().unwrap().to_owned(),
                        conflict["reused"].as_bool().unwrap(),
                    )
                })
                .collect()
        };
        let from_list = pairs(&listed["conflicts"]);
        // The operation in progress lists the same.
        let operation = self.run("op.status", json!({}))["operation"].clone();
        if !operation.is_null() {
            assert_eq!(pairs(&operation["conflicts"]), from_list);
        }
        // And so does the status of rerere, sorted the other way.
        let status = self.run("rerere.status", json!({}));
        let named = |key: &str| -> Vec<String> {
            status[key]
                .as_array()
                .unwrap()
                .iter()
                .map(|path| path.as_str().unwrap().to_owned())
                .collect()
        };
        let pick = |reused: bool| -> Vec<String> {
            from_list
                .iter()
                .filter(|(_, flag)| *flag == reused)
                .map(|(path, _)| path.clone())
                .collect()
        };
        assert_eq!(named("reused"), pick(true));
        assert_eq!(named("remaining"), pick(false));
        from_list
    }
    /// Begin the merge on both copies; it stops on its conflicts.
    fn merge(&mut self) -> Value {
        let out = self.run("branch.merge", json!({ "target": "topic" }));
        assert!(out["stopped"].is_string(), "{out}");
        assert!(
            self.twin
                .git()
                .run(["merge", "--no-edit", "topic"])
                .is_err()
        );
        out
    }
    /// Resolve all three conflicts on both copies and finish the merge:
    /// `a.txt` and `b.txt` with text put together by hand, the readme with
    /// topic's version.
    fn resolve_and_finish(&mut self, a: &str) {
        self.both(|copy| {
            copy.write("a.txt", a).unwrap();
            copy.write("b.txt", "both b, by hand\n").unwrap();
        });
        self.run("conflict.resolve", json!({ "path": "a.txt" }));
        self.run("conflict.resolve", json!({ "path": "b.txt" }));
        self.run(
            "conflict.take",
            json!({ "path": "README.md", "side": "theirs" }),
        );
        let twin = self.twin.git();
        twin.run(["add", "a.txt", "b.txt"]).unwrap();
        twin.run(["checkout", "--theirs", "--", "README.md"])
            .unwrap();
        twin.run(["add", "README.md"]).unwrap();
        assert_eq!(self.run("op.continue", json!({}))["finished"], true);
        twin.run(["commit", "-q", "--no-edit"]).unwrap();
    }
    /// Take the finished merge back on both copies.
    fn take_back(&mut self) {
        self.run("undo.last", json!({}));
        self.twin
            .git()
            .run(["reset", "-q", "--hard", "HEAD~1"])
            .unwrap();
    }
}

fn pairs(list: &[(&str, bool)]) -> Vec<(String, bool)> {
    list.iter()
        .map(|(path, reused)| ((*path).to_owned(), *reused))
        .collect()
}

#[test]
fn a_resolution_recorded_once_is_reused_and_said_to_be() {
    let mut h = Harness::new();
    let status_at_rest = h.run("rerere.status", json!({}));
    assert_eq!(
        status_at_rest,
        json!({ "enabled": true, "recorded": 0, "reused": [], "remaining": [] })
    );

    // The first time: nothing is recorded, everything is still to do.
    let out = h.merge();
    assert_eq!(out["conflicts"], json!(["README.md", "a.txt", "b.txt"]));
    h.assert_same("the first merge, stopped");
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", false), ("b.txt", false)])
    );
    assert_eq!(
        h.git().lines(["rerere", "remaining"]).unwrap(),
        ["README.md", "a.txt", "b.txt"]
    );
    // Taken apart for the merge tool, a file nothing was remembered for
    // has no resolution to start from.
    let detail = h.run("conflict.detail", json!({ "path": "a.txt" }));
    assert!(detail["reused"].is_null(), "{detail}");
    // A file resolved by hand and not staged yet is not one rerere
    // resolved: it knows the conflict as its own to record.
    h.scratch.write("a.txt", "both a, by hand\n").unwrap();
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", false), ("b.txt", false)])
    );

    h.resolve_and_finish("both a, by hand\n");
    h.assert_same("the first merge, finished");
    let after = h.run("rerere.status", json!({}));
    assert_eq!(after["recorded"], 2);
    assert_eq!(resolutions(&h.twin), 2);
    assert_eq!(after["reused"], json!([]));
    assert_eq!(after["remaining"], json!([]));

    // Taken back and merged again, git puts the two resolutions in. The
    // files are still unmerged: it is for the user to accept them.
    h.take_back();
    let out = h.merge();
    assert_eq!(out["conflicts"], json!(["README.md", "a.txt", "b.txt"]));
    h.assert_same("the second merge, stopped");
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)])
    );
    assert_eq!(
        h.twin.git().lines(["rerere", "remaining"]).unwrap(),
        ["README.md"],
        "git's own list of what is left"
    );
    assert_eq!(
        std::fs::read_to_string(h.file("a.txt")).unwrap(),
        "both a, by hand\n"
    );
    assert_eq!(
        status(&h.git()),
        "DU README.md\nUU a.txt\nUU b.txt\nA  calm.txt\n",
        "unmerged all the same"
    );
    let kinds = h.run("conflict.list", json!({}));
    assert_eq!(kinds["conflicts"][1]["kind"], "both_modified");
    assert!(kinds["conflicts"][1]["ours"].is_string());
    // Taken apart for the merge tool, a file git resolved as last time
    // comes with that resolution, which is what the file holds, beside
    // the two sides it was made from; the one it did not resolve has none.
    let detail = h.run("conflict.detail", json!({ "path": "a.txt" }));
    assert_eq!(detail["reused"], "both a, by hand\n");
    assert_eq!(detail["regions"][0]["ours"], json!(["main's a\n"]));
    assert_eq!(detail["regions"][0]["theirs"], json!(["topic's a\n"]));
    let detail = h.run("conflict.detail", json!({ "path": "README.md" }));
    assert!(detail["reused"].is_null(), "{detail}");
    // Changed since, it is no longer what was remembered: the tool starts
    // from the two sides. Put back, it is again.
    h.scratch
        .write("a.txt", "<<<<<<< x\nmarkers again\n=======\ny\n>>>>>>> z\n")
        .unwrap();
    assert!(h.run("conflict.detail", json!({ "path": "a.txt" }))["reused"].is_null());
    h.scratch.write("a.txt", "both a, by hand\n").unwrap();

    // Accepted as they are, the merge is the one made by hand before.
    h.refused("op.continue", json!({}), "3 files are still in conflict");
    h.run("conflict.resolve", json!({ "path": "a.txt" }));
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("b.txt", true)]),
        "an accepted file is no longer in conflict"
    );
    h.run("conflict.resolve", json!({ "path": "b.txt" }));
    h.run(
        "conflict.take",
        json!({ "path": "README.md", "side": "theirs" }),
    );
    let twin = h.twin.git();
    twin.run(["add", "a.txt", "b.txt"]).unwrap();
    twin.run(["checkout", "--theirs", "--", "README.md"])
        .unwrap();
    twin.run(["add", "README.md"]).unwrap();
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    twin.run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("the second merge, finished");
    assert_eq!(h.reused(), pairs(&[]));
}

#[test]
fn a_resolution_is_reused_when_a_mailbox_meets_the_same_conflict() {
    // The same conflict met by another operation: git knows it again.
    let mut h = Harness::new();
    h.merge();
    h.resolve_and_finish("both a, by hand\n");
    h.take_back();
    h.both(|copy| {
        copy.git()
            .run(["format-patch", "-q", "--output=topic.mbox", "main..topic"])
            .unwrap();
    });
    let mailbox = h.file("topic.mbox").to_str().unwrap().to_owned();
    let out = h.run(
        "patch.mailbox",
        json!({ "file": mailbox, "three_way": true }),
    );
    assert!(out["stopped"].is_string(), "{out}");
    assert!(h.twin.git().run(["am", "--3way", "topic.mbox"]).is_err());
    assert_eq!(
        h.run("op.status", json!({}))["operation"]["kind"],
        "mailbox"
    );
    assert_eq!(
        status(&h.git()),
        status(&h.twin.git()),
        "the mailbox, stopped"
    );
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)])
    );
    assert_eq!(
        std::fs::read(h.file("a.txt")).unwrap(),
        std::fs::read(h.twin.path().join("a.txt")).unwrap()
    );
    h.run("op.abort", json!({}));
    h.twin.git().run(["am", "--abort"]).unwrap();
    assert_eq!(status(&h.git()), status(&h.twin.git()));
    assert_eq!(h.reused(), pairs(&[]));
}

#[test]
fn forgetting_a_resolution_leaves_the_file_to_be_resolved_afresh() {
    let mut h = Harness::new();
    h.merge();
    h.resolve_and_finish("both a, by hand\n");
    h.take_back();
    h.merge();
    assert_eq!(h.run("rerere.status", json!({}))["recorded"], 2);

    let out = h.run("rerere.forget", json!({ "path": "a.txt" }));
    h.twin.git().run(["rerere", "forget", "a.txt"]).unwrap();
    assert_eq!(out, json!({ "path": "a.txt", "forgotten": true }));
    h.assert_same("forgotten");
    assert_eq!(h.run("rerere.status", json!({}))["recorded"], 1);
    assert_eq!(resolutions(&h.twin), 1);
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", false), ("b.txt", true)])
    );
    // What rerere had put into the file is still there: it is the index
    // that says the file is to be resolved.
    assert_eq!(
        std::fs::read_to_string(h.file("a.txt")).unwrap(),
        "both a, by hand\n"
    );

    // Once more there is nothing left to forget, and that is said.
    let out = h.run("rerere.forget", json!({ "path": "a.txt" }));
    assert_eq!(out["forgotten"], false);
    // The readme's conflict is of a kind rerere records nothing for.
    let out = h.run("rerere.forget", json!({ "path": "README.md" }));
    assert_eq!(out["forgotten"], false);
    h.twin.git().run(["rerere", "forget", "a.txt"]).unwrap();
    h.twin.git().run(["rerere", "forget", "README.md"]).unwrap();
    h.assert_same("forgotten again");
    // A file that is in no conflict has no resolution to forget.
    h.refused(
        "rerere.forget",
        json!({ "path": "calm.txt" }),
        "is not in conflict",
    );

    // Resolved another way, that is what is recorded and reused from now on.
    h.resolve_and_finish("both a, the other way\n");
    h.assert_same("resolved afresh");
    assert_eq!(h.run("rerere.status", json!({}))["recorded"], 2);
    h.take_back();
    h.merge();
    h.assert_same("merged a third time");
    assert_eq!(
        std::fs::read_to_string(h.file("a.txt")).unwrap(),
        "both a, the other way\n"
    );
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)])
    );

    // A file that was accepted and staged can have its resolution
    // forgotten as well: git puts the conflict back together to find it.
    h.run("conflict.resolve", json!({ "path": "b.txt" }));
    h.twin.git().run(["add", "b.txt"]).unwrap();
    let staged = h.git().run(["ls-files", "--stage"]).unwrap();
    let out = h.run("rerere.forget", json!({ "path": "b.txt" }));
    h.twin.git().run(["rerere", "forget", "b.txt"]).unwrap();
    assert_eq!(out["forgotten"], true);
    assert_eq!(
        h.git().run(["ls-files", "--stage"]).unwrap(),
        staged,
        "the index is as it was"
    );
    h.assert_same("forgotten after staging");
}

/// Whether rerere is set one way or the other outside the repository, in
/// the environment the commands run git in. What holds "when nothing is
/// set" can only be shown where nothing is.
fn set_outside(dir: &Path) -> bool {
    std::process::Command::new(bitbraid_testkit::oracle::git_binary())
        .current_dir(dir)
        .args(["config", "--get", "rerere.enabled"])
        .env_remove("GIT_DIR")
        .output()
        .map(|out| !out.stdout.is_empty())
        .unwrap_or(true)
}

#[test]
fn rerere_is_switched_on_and_off_in_the_repository_s_configuration() {
    let mut h = Harness::with(None);
    let local = |git: &Git| {
        git.run(["config", "--local", "--get-all", "rerere.enabled"])
            .unwrap_or_default()
    };
    assert_eq!(local(&h.git()), "");
    let unset_means_unset = !set_outside(h.scratch.path());
    if unset_means_unset {
        // Nothing set, nothing recorded: off.
        assert_eq!(
            h.run("rerere.status", json!({})),
            json!({ "enabled": false, "recorded": 0, "reused": [], "remaining": [] })
        );
        h.merge();
        assert!(!h.file(".git/rr-cache").exists(), "nothing was recorded");
        h.run("op.abort", json!({}));
        h.twin.git().run(["merge", "--abort"]).unwrap();
    }

    // On, for this repository; the scope is the repository's when not
    // said.
    let out = h.run("rerere.enable", json!({ "on": true }));
    h.twin
        .git()
        .run(["config", "rerere.enabled", "true"])
        .unwrap();
    assert_eq!(out["enabled"], true);
    assert_eq!(local(&h.git()), "true\n");
    assert_eq!(local(&h.git()), local(&h.twin.git()));
    h.merge();
    h.resolve_and_finish("both a, by hand\n");
    h.assert_same("recorded once it was on");
    assert_eq!(h.run("rerere.status", json!({}))["recorded"], 2);

    // Off again: written as off, and nothing is replayed although the
    // record is there. The merge is taken back first: the switch is an
    // action of its own in the journal, and would be undone instead.
    h.take_back();
    let out = h.run("rerere.enable", json!({ "on": false, "scope": "repo" }));
    h.twin
        .git()
        .run(["config", "rerere.enabled", "false"])
        .unwrap();
    assert_eq!(out["enabled"], false);
    assert_eq!(out["recorded"], 2);
    assert_eq!(local(&h.git()), "false\n");
    // Undone it is on again, redone off.
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Switch rerere off");
    h.run("undo.last", json!({}));
    assert_eq!(local(&h.git()), "true\n");
    h.run("redo.last", json!({}));
    assert_eq!(local(&h.git()), "false\n");
    h.merge();
    h.assert_same("merged with rerere off");
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", false), ("b.txt", false)])
    );
    assert!(
        std::fs::read_to_string(h.file("a.txt"))
            .unwrap()
            .contains("<<<<<<< "),
        "the conflict is in the file"
    );
    // Not even a file put right by hand counts as rerere's then.
    h.scratch
        .write("a.txt", "by hand, rerere being off\n")
        .unwrap();
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", false), ("b.txt", false)])
    );
    h.refused("rerere.forget", json!({ "path": "a.txt" }), "is not on");
    h.run("op.abort", json!({}));
    h.twin.git().run(["merge", "--abort"]).unwrap();

    // With the setting gone git goes by the record: there is one, so it
    // is on, and the resolutions come back.
    if !unset_means_unset {
        return;
    }
    h.both(|copy| {
        copy.git()
            .run(["config", "--unset", "rerere.enabled"])
            .unwrap();
    });
    assert_eq!(h.run("rerere.status", json!({}))["enabled"], true);
    h.merge();
    h.assert_same("merged with the setting gone");
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)])
    );
}

fn config(git: &Git) -> String {
    git.run(["config", "--local", "--list"]).unwrap()
}

#[test]
fn hostile_rerere_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    h.merge();
    h.resolve_and_finish("both a, by hand\n");
    h.take_back();
    h.merge();
    let git = h.git();
    let before = (
        status(&git),
        git.run(["ls-files", "--stage"]).unwrap(),
        record(&h.scratch),
        config(&git),
    );
    let long = "r".repeat(5000);

    for path in [
        "",
        "/etc/passwd",
        "../a.txt",
        "x/../../a.txt",
        "nul\u{0}",
        "no-such-file.txt",
        "calm.txt",
        "-a",
        "--all",
        "--rerere-autoupdate",
        ".",
        "*.txt",
        ":(glob)**",
        ".git/MERGE_RR",
        long.as_str(),
    ] {
        assert!(
            h.try_run("rerere.forget", json!({ "path": path })).is_err(),
            "rerere.forget took {:?}",
            path.get(..path.len().min(40))
        );
    }
    // Only the repository's own configuration is ever asked for here: a
    // scope that is not one of the two is refused before anything is
    // written anywhere.
    for params in [
        json!({}),
        json!({ "on": "yes" }),
        json!({ "on": 1 }),
        json!({ "on": null }),
        json!({ "scope": "repo" }),
        json!({ "on": false, "scope": "" }),
        json!({ "on": false, "scope": "system" }),
        json!({ "on": false, "scope": "global" }),
        json!({ "on": false, "scope": "USER" }),
        json!({ "on": false, "scope": "--global" }),
        json!({ "on": false, "scope": "--file=/tmp/pwned" }),
        json!({ "on": false, "scope": 3 }),
        json!({ "on": false, "scope": ["repo"] }),
    ] {
        assert!(
            h.try_run("rerere.enable", params.clone()).is_err(),
            "rerere.enable {params}"
        );
    }
    for params in [
        json!({}),
        json!({ "path": 3 }),
        json!({ "path": ["a.txt"] }),
        json!({ "path": null }),
    ] {
        assert!(
            h.try_run("rerere.forget", params.clone()).is_err(),
            "rerere.forget {params}"
        );
    }
    assert_eq!(
        (
            status(&git),
            git.run(["ls-files", "--stage"]).unwrap(),
            record(&h.scratch),
            config(&git),
        ),
        before
    );
    assert_eq!(
        h.reused(),
        pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)])
    );
}

/// Something done to a file or directory git keeps for rerere.
type Damage<'a> = Box<dyn FnOnce(&Path) + 'a>;

/// Replace what is at `path` for the length of `during`, and put it back.
fn with_damage(path: &Path, damage: impl FnOnce(&Path), during: impl FnOnce()) {
    let aside = path.with_extension("aside");
    let was_there = path.symlink_metadata().is_ok();
    if was_there {
        std::fs::rename(path, &aside).unwrap();
    }
    damage(path);
    during();
    if path.is_dir() {
        std::fs::remove_dir_all(path).unwrap();
    } else if path.symlink_metadata().is_ok() {
        std::fs::remove_file(path).unwrap();
    }
    if was_there {
        std::fs::rename(&aside, path).unwrap();
    }
}

#[test]
fn a_record_that_is_not_as_expected_is_read_without_a_crash() {
    let mut h = Harness::new();
    h.merge();
    h.resolve_and_finish("both a, by hand\n");
    h.take_back();
    h.merge();
    let expected = pairs(&[("README.md", false), ("a.txt", true), ("b.txt", true)]);
    assert_eq!(h.reused(), expected);
    let merge_rr = h.file(".git/MERGE_RR");
    let cache = h.file(".git/rr-cache");
    let big = vec![b'x'; 2 * 1024 * 1024];

    // The list rerere keeps for the conflict at hand. HEAD, the objects
    // and the references are left alone: git has to go on taking this
    // for a repository.
    let damages: Vec<(&str, Damage<'_>)> = vec![
        ("gone", Box::new(|_| {})),
        (
            "garbage",
            Box::new(|path| std::fs::write(path, [0u8, 255, 1, 254, b'\n', 0]).unwrap()),
        ),
        (
            "no id",
            Box::new(|path| std::fs::write(path, "not an id\ta.txt\0").unwrap()),
        ),
        (
            "an id of nothing recorded",
            Box::new(|path| std::fs::write(path, format!("{}\ta.txt\0", "0".repeat(40))).unwrap()),
        ),
        (
            "a path that leaves the repository",
            Box::new(|path| {
                std::fs::write(path, format!("{}\t../../outside.txt\0", "a".repeat(40))).unwrap()
            }),
        ),
        ("huge", Box::new(|path| std::fs::write(path, &big).unwrap())),
        (
            "a directory",
            Box::new(|path| std::fs::create_dir(path).unwrap()),
        ),
    ];
    for (what, damage) in damages {
        let mut answered = None;
        with_damage(&merge_rr, damage, || {
            // An answer, never a crash.
            answered = Some((
                h.try_run("rerere.status", json!({})),
                h.try_run("conflict.list", json!({})),
                h.try_run("op.status", json!({})),
            ));
        });
        let (status, listed, operation) = answered.unwrap();
        let status = status.unwrap_or_else(|err| panic!("{what}: {err}"));
        assert_eq!(status["enabled"], true, "{what}");
        assert_eq!(status["recorded"], 2, "{what}");
        let listed = listed.unwrap_or_else(|err| panic!("{what}: {err}"));
        assert_eq!(listed["conflicts"].as_array().unwrap().len(), 3, "{what}");
        let operation = operation.unwrap_or_else(|err| panic!("{what}: {err}"));
        assert_eq!(operation["operation"]["kind"], "merge", "{what}");
    }
    assert_eq!(h.reused(), expected, "back as it was");

    // The record itself: not a directory, empty, or full of what is no
    // resolution. Only names are looked at, so what is in a file here
    // does not matter and is not read.
    let id = |n: u8| char::from(b'a' + n).to_string().repeat(40);
    let damages: Vec<(&str, usize, Damage<'_>)> = vec![
        ("gone", 0, Box::new(|_| {})),
        (
            "a file",
            0,
            Box::new(|path| std::fs::write(path, "not a directory\n").unwrap()),
        ),
        (
            "empty",
            0,
            Box::new(|path| std::fs::create_dir(path).unwrap()),
        ),
        (
            "names of every kind",
            4,
            Box::new(|path| {
                // Counted: a resolution, and two more ways of the same
                // conflict; and one under a name that is no id, since
                // names are all that is read.
                for (dir, files) in [
                    (id(0), vec!["preimage", "postimage", "thisimage"]),
                    (id(1), vec!["preimage.1", "postimage.1", "postimage.22"]),
                    // Not counted: no resolution, or no name of one.
                    (id(2), vec!["preimage"]),
                    // (Windows drops a dot at the end of a name and does
                    // not tell `P` from `p`: there those two would be the
                    // name of a resolution.)
                    (
                        id(3),
                        if cfg!(windows) {
                            vec!["postimage.x", "postimage1"]
                        } else {
                            vec!["postimage.", "postimage.x", "postimage1", "Postimage"]
                        },
                    ),
                    (id(4), vec![]),
                    ("not-an-id".to_owned(), vec!["postimage"]),
                ] {
                    std::fs::create_dir_all(path.join(&dir)).unwrap();
                    for file in files {
                        std::fs::write(path.join(&dir).join(file), [0u8, 255]).unwrap();
                    }
                }
                std::fs::write(path.join("a-file-among-them"), "x").unwrap();
            }),
        ),
    ];
    for (what, recorded, damage) in damages {
        let mut answered = None;
        with_damage(&cache, damage, || {
            answered = Some((
                h.try_run("rerere.status", json!({})),
                h.try_run("conflict.list", json!({})),
                h.try_run("rerere.forget", json!({ "path": "a.txt" })),
            ));
        });
        let (status, listed, _) = answered.unwrap();
        let status = status.unwrap_or_else(|err| panic!("{what}: {err}"));
        assert_eq!(status["recorded"], recorded, "{what}");
        // The setting says on, whatever has become of the record.
        assert_eq!(status["enabled"], true, "{what}");
        let listed = listed.unwrap_or_else(|err| panic!("{what}: {err}"));
        assert_eq!(listed["conflicts"].as_array().unwrap().len(), 3, "{what}");
    }
    assert_eq!(h.run("rerere.status", json!({}))["recorded"], 2);
    assert_eq!(h.run("op.status", json!({}))["operation"]["kind"], "merge");
}
