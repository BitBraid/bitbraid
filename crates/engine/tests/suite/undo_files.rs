//! Undo of what merges into the files and the index and leaves HEAD where
//! it is: a cherry-pick without committing and a squash
//! merge. Each is done on one copy by the command and on a twin by git;
//! undo is held to `git reset --merge` on the twin, which takes back what
//! was merged and the message git prepared, and redo to the same command
//! run again.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Everything such an operation and its undo can change: HEAD, the index
/// with its stages, the files as status sees them, the untracked file
/// left alone, and the messages prepared for the next commit.
fn state(copy: &Scratch) -> Vec<String> {
    let git = copy.git();
    let mut out = vec![format!("HEAD {}", git.rev_parse("HEAD").unwrap())];
    out.extend(git.lines(["ls-files", "--stage"]).unwrap());
    out.extend(
        git.lines(["status", "--porcelain=v1", "--untracked-files=all"])
            .unwrap(),
    );
    for name in ["MERGE_MSG", "SQUASH_MSG"] {
        let file = copy.path().join(".git").join(name);
        out.push(format!("{name}: {:?}", std::fs::read_to_string(file).ok()));
    }
    for file in ["a.txt", "b.txt", "side.txt", "notes.txt"] {
        out.push(format!(
            "{file}: {:?}",
            std::fs::read_to_string(copy.path().join(file)).ok()
        ));
    }
    out
}

impl Harness {
    /// `linear`, with a branch `side` from its third commit that adds a
    /// file and changes `a.txt`, and a change to `b.txt` and an untracked
    /// file in the worktree, neither of which the side touches.
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        for copy in [&scratch, &twin] {
            let git = copy.git();
            git.run(["switch", "-q", "-c", "side", "HEAD~2"]).unwrap();
            copy.write("side.txt", "side\n").unwrap();
            copy.write("a.txt", "2\nfrom the side\n").unwrap();
            git.run(["add", "-A"]).unwrap();
            git.run(["commit", "-q", "-m", "Independent side"]).unwrap();
            git.run(["switch", "-q", "main"]).unwrap();
            copy.write("b.txt", "bb, changed here\n").unwrap();
            copy.write("notes.txt", "mine\n").unwrap();
        }
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .unwrap();
        Self {
            scratch,
            twin,
            registry,
            ctx,
        }
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }
    fn twin(&self) -> Git {
        self.twin.git()
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.scratch), state(&self.twin), "{what}");
    }
}

/// The operation, as the command and as git, then undo and redo of it.
fn round_trip(command: (&str, Value), git_args: &[&str], label: &str, merged_message: &str) {
    let mut h = Harness::new();
    let start = state(&h.scratch);
    h.assert_same("start");

    h.run(command.0, command.1.clone());
    h.twin().run(git_args).unwrap();
    h.assert_same(label);
    let done = state(&h.scratch);
    assert_ne!(done, start, "{label} changed something");
    assert!(
        done.iter()
            .any(|line| line.contains(merged_message) && line.contains("_MSG")),
        "{label}: git prepared a message: {done:?}"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], label);

    h.run("undo.last", json!({}));
    h.twin().run(["reset", "-q", "--merge"]).unwrap();
    h.assert_same(&format!("undo of {label}"));
    assert_eq!(state(&h.scratch), start);

    h.run("redo.last", json!({}));
    h.twin().run(git_args).unwrap();
    h.assert_same(&format!("redo of {label}"));

    // Changed since, a file it merged is not overwritten by undo.
    h.scratch.write("side.txt", "edited after\n").unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("side.txt"), "{err}");
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("side.txt")).unwrap(),
        "edited after\n"
    );
}

#[test]
fn a_pick_without_committing_is_undone_as_git_resets_it() {
    round_trip(
        (
            "commit.cherry_pick",
            json!({ "commits": ["side"], "no_commit": true }),
        ),
        &["cherry-pick", "--no-commit", "side"],
        "Cherry-pick 1 commit",
        "Independent side",
    );
}

#[test]
fn a_squash_merge_is_undone_as_git_resets_it() {
    round_trip(
        (
            "branch.merge",
            json!({ "target": "side", "mode": "squash" }),
        ),
        &["merge", "-q", "--squash", "side"],
        "Squash-merge side",
        "Independent side",
    );
}

/// The journal is a file anybody can write: an entry that names a path
/// outside the worktree, or an index line with something in it that is
/// no mode, is refused before anything is fed to git.
#[test]
fn a_hostile_journal_entry_changes_nothing() {
    type Spoil = fn(&mut Value);
    let spoilers: [Spoil; 4] = [
        |files| files["paths"][0] = json!("../outside.txt"),
        |files| files["paths"][0] = json!("/etc/passwd"),
        |files| files["index_before"][0]["mode"] = json!("100644 0\u{0}x"),
        |files| files["index_before"][0]["stage"] = json!(7),
    ];
    for spoil in spoilers {
        let mut h = Harness::new();
        h.run(
            "branch.merge",
            json!({ "target": "side", "mode": "squash" }),
        );
        let file = h
            .scratch
            .path()
            .join(".git")
            .join("bitbraid")
            .join("journal.json");
        let mut value: Value =
            serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        let entry = value["done"].as_array_mut().unwrap().last_mut().unwrap();
        spoil(&mut entry["change"]["files"]);
        std::fs::write(&file, serde_json::to_string(&value).unwrap()).unwrap();
        // Read again, as a new start reads it.
        let path = h.scratch.path().to_owned();
        h.run("repo.open", json!({ "path": path }));
        let there = state(&h.scratch);
        let result = h.try_run("undo.last", json!({}));
        assert!(result.is_err(), "{result:?}");
        assert_eq!(state(&h.scratch), there);
    }
}
