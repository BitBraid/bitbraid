//! What rerere has recorded, listed and forgotten, and the files of an
//! operation that can be put back in conflict, against git on twin copies:
//! a record is what git wrote under `rr-cache`, and a record that was
//! forgotten no longer resolves the conflict it was of, as when git forgets
//! it or its directory is gone.

use std::path::Path;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Two lines of history that conflict in `a.txt`, with rerere on.
fn diverge(copy: &Scratch) {
    let git = copy.git();
    for (key, value) in [
        ("user.name", "Ada Fixture"),
        ("user.email", "ada@fixture.invalid"),
        ("commit.gpgsign", "false"),
        ("merge.conflictStyle", "merge"),
        ("rerere.enabled", "true"),
        ("rerere.autoUpdate", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    copy.write("a.txt", "top\nmiddle\nbottom\n").unwrap();
    git.run(["commit", "-q", "-am", "three lines"]).unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("a.txt", "top\ntopic's middle\nbottom\n")
        .unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write("a.txt", "top\nmain's middle\nbottom\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
}

/// Merge, resolve the conflict by hand and commit: rerere records it.
fn resolve_once(copy: &Scratch) {
    let git = copy.git();
    assert!(git.run(["merge", "-q", "topic"]).is_err());
    copy.write("a.txt", "top\nboth middles\nbottom\n").unwrap();
    git.run(["commit", "-q", "-am", "merged by hand"]).unwrap();
    // Back to before the merge, for it to be made again.
    git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
}

/// The names under `rr-cache`, with what each directory holds.
fn cache(copy: &Scratch) -> Vec<(String, Vec<String>)> {
    let dir = copy.path().join(".git/rr-cache");
    let mut out: Vec<(String, Vec<String>)> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|entry| {
                    let mut files: Vec<String> = std::fs::read_dir(entry.path())
                        .map(|files| {
                            files
                                .flatten()
                                .map(|file| file.file_name().to_string_lossy().into_owned())
                                .collect()
                        })
                        .unwrap_or_default();
                    files.sort();
                    (entry.file_name().to_string_lossy().into_owned(), files)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        diverge(&scratch);
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
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.scratch.path().join(rel)).unwrap()
    }
}

#[test]
fn a_record_is_what_git_wrote_and_says_what_was_resolved_to_what() {
    let mut h = Harness::new();
    let none = h.run("rerere.records", json!({}));
    assert_eq!(none["enabled"], true);
    assert_eq!(none["records"], json!([]));

    resolve_once(&h.scratch);
    let written = cache(&h.scratch);
    assert_eq!(written.len(), 1, "{written:?}");
    let listed = h.run("rerere.records", json!({}));
    let records = listed["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record["id"], written[0].0);
    assert_eq!(record["resolved"], true);
    assert_eq!(record["conflicts"], 1);
    // Git writes the two sides in an order of its own; they are the two.
    let mut sides = vec![record["ours"].clone(), record["theirs"].clone()];
    sides.sort_by_key(Value::to_string);
    assert_eq!(sides, [json!(["main's middle"]), json!(["topic's middle"])]);
    assert_eq!(record["resolution"], json!(["both middles"]));
    assert_eq!(record["path"], Value::Null);
    assert!(record["time"].as_i64().unwrap() > 1_600_000_000);

    // The conflict comes up again: git resolves it from the record,
    // which is the same one.
    assert!(h.git().run(["merge", "-q", "topic"]).is_err());
    assert_eq!(h.read("a.txt"), "top\nboth middles\nbottom\n");
    let during = h.run("rerere.records", json!({}));
    assert_eq!(during["records"].as_array().unwrap().len(), 1);
    assert_eq!(during["records"][0]["id"], written[0].0);
    assert_eq!(
        h.run("rerere.records", json!({ "limit": 0 }))["records"],
        json!([])
    );
    h.git().run(["merge", "--abort"]).unwrap();
}

#[test]
fn a_conflict_that_is_being_resolved_is_listed_with_its_file_and_left_alone() {
    let mut h = Harness::new();
    assert!(h.git().run(["merge", "-q", "topic"]).is_err());
    // Git has seen the conflict and waits to record how it is resolved.
    let listed = h.run("rerere.records", json!({}));
    let records = listed["records"].as_array().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0]["resolved"], false);
    assert_eq!(records[0]["path"], "a.txt");
    assert_eq!(records[0]["resolution"], Value::Null);
    let id = records[0]["id"].as_str().unwrap().to_owned();
    assert_eq!(cache(&h.scratch)[0].0, id);
    let before = cache(&h.scratch);
    let err = h
        .try_run("rerere.forget_record", json!({ "id": id }))
        .unwrap_err();
    assert!(
        err.contains("a.txt") && err.contains("being resolved"),
        "{err}"
    );
    assert_eq!(cache(&h.scratch), before);
    // Resolved and committed, git records it, as it would have anyway.
    h.scratch
        .write("a.txt", "top\nboth middles\nbottom\n")
        .unwrap();
    h.git().run(["commit", "-q", "-am", "merged"]).unwrap();
    let listed = h.run("rerere.records", json!({}));
    assert_eq!(listed["records"][0]["resolved"], true);
    assert_eq!(listed["records"][0]["path"], Value::Null);
    assert_eq!(listed["records"][0]["resolution"], json!(["both middles"]));
}

#[test]
fn a_forgotten_record_no_longer_resolves_as_when_its_directory_is_gone() {
    let mut h = Harness::new();
    let twin = Scratch::of("linear").unwrap();
    diverge(&twin);
    resolve_once(&h.scratch);
    resolve_once(&twin);
    let id = cache(&h.scratch)[0].0.clone();
    assert_eq!(cache(&twin)[0].0, id, "the same conflict has the same name");

    let out = h.run("rerere.forget_record", json!({ "id": id }));
    assert_eq!(out["forgotten"], true);
    std::fs::remove_dir_all(twin.path().join(".git/rr-cache").join(&id)).unwrap();
    assert_eq!(cache(&h.scratch), cache(&twin));
    assert_eq!(h.run("rerere.records", json!({}))["records"], json!([]));
    // Forgotten twice is forgotten.
    assert_eq!(
        h.run("rerere.forget_record", json!({ "id": id }))["forgotten"],
        false
    );

    // The conflict is one again, in both.
    assert!(h.git().run(["merge", "-q", "topic"]).is_err());
    assert!(twin.git().run(["merge", "-q", "topic"]).is_err());
    let ours = h.read("a.txt");
    assert!(ours.contains("<<<<<<<"), "{ours}");
    assert_eq!(
        ours,
        std::fs::read_to_string(twin.path().join("a.txt")).unwrap()
    );
    assert_eq!(cache(&h.scratch), cache(&twin));
}

#[test]
fn resolved_files_are_the_ones_git_can_put_back_in_conflict() {
    let mut h = Harness::new();
    assert_eq!(h.run("conflict.resolved", json!({}))["paths"], json!([]));
    assert!(h.git().run(["merge", "-q", "topic"]).is_err());
    assert_eq!(h.run("conflict.resolved", json!({}))["paths"], json!([]));
    h.scratch.write("a.txt", "top\nresolved\nbottom\n").unwrap();
    h.run("conflict.resolve", json!({ "path": "a.txt" }));
    let resolved = h.run("conflict.resolved", json!({}));
    assert_eq!(resolved["paths"], json!(["a.txt"]));
    // What git itself remembers of it.
    let undo = h.git().run(["ls-files", "--resolve-undo"]).unwrap();
    assert!(undo.lines().all(|line| line.ends_with("\ta.txt")) && !undo.is_empty());
    // Put back, it is in conflict and no longer listed.
    h.run("conflict.reopen", json!({ "path": "a.txt" }));
    assert!(h.read("a.txt").contains("<<<<<<<"));
    assert_eq!(h.run("conflict.resolved", json!({}))["paths"], json!([]));
    assert!(
        h.git()
            .lines(["diff", "--name-only", "--diff-filter=U"])
            .unwrap()
            .contains(&"a.txt".to_owned())
    );
    h.git().run(["merge", "--abort"]).unwrap();
    assert_eq!(h.run("conflict.resolved", json!({}))["paths"], json!([]));
}

#[test]
fn hostile_ids_forget_nothing_and_reach_nothing_outside_the_record() {
    let mut h = Harness::new();
    resolve_once(&h.scratch);
    let before = cache(&h.scratch);
    let kept = h.scratch.path().join(".git/kept-elsewhere");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("file"), "stays\n").unwrap();
    // A link with the name of a record, to somewhere else.
    let linked = "cd".repeat(20);
    bitbraid_testkit::symlink_dir(&kept, h.scratch.path().join(".git/rr-cache").join(&linked))
        .unwrap();
    for hostile in [
        json!(""),
        json!("."),
        json!(".."),
        json!("../kept-elsewhere"),
        json!("../../.git"),
        json!("/etc"),
        json!("--all"),
        json!("g".repeat(40)),
        json!("a".repeat(39)),
        json!("a".repeat(41)),
        json!(format!("{}/..", "a".repeat(40))),
        json!("a\u{0}b"),
        json!("x".repeat(70_000)),
        json!(7),
        json!(null),
        json!(["a"]),
        json!({ "id": "a" }),
    ] {
        let result = h.try_run("rerere.forget_record", json!({ "id": hostile }));
        assert!(result.is_err(), "{hostile}: {result:?}");
    }
    // A name that is one, and names nothing; and the link, which is not
    // followed.
    for id in ["ab".repeat(20), linked.clone(), "ab".repeat(32)] {
        let out = h.run("rerere.forget_record", json!({ "id": id }));
        assert_eq!(out["forgotten"], false, "{id}");
    }
    assert!(Path::new(&kept.join("file")).is_file());
    let mut after = cache(&h.scratch);
    after.retain(|(name, _)| *name != linked);
    assert_eq!(after, before);
    for hostile in [json!("x"), json!(-1), json!([1]), json!(1e308)] {
        let _ = h.try_run("rerere.records", json!({ "limit": hostile }));
    }
    assert_eq!(
        h.run("rerere.records", json!({}))["records"]
            .as_array()
            .unwrap()
            .len(),
        1,
        "the link is no record"
    );
}
