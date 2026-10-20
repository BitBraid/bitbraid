//! The undo journal as a whole: listing it, and going back and forward
//! several entries at once, compared with taking the same steps one by one
//! on a twin scratch copy.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Side {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Side {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let git = scratch.git();
        // Commands run git with the real environment: what a commit needs
        // is set here, and the developer's own signing is kept out.
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
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
    /// References, HEAD, index and worktree, without what differs between
    /// two copies that did the same at different seconds: the ids of the
    /// commits made here.
    fn state(&self) -> Vec<String> {
        let git = self.git();
        let mut lines = git
            .user_refs("%(refname) %(objecttype) %(tree) %(contents:subject)")
            .unwrap();
        let head = git.run(["rev-parse", "--abbrev-ref", "HEAD"]).unwrap();
        lines.push(format!("HEAD {}", head.trim()));
        lines.extend(git.lines(["log", "-1", "--format=at %T %s"]).unwrap());
        lines.extend(
            git.lines(["status", "--porcelain=v2", "--untracked-files=all"])
                .unwrap(),
        );
        lines.extend(git.lines(["ls-files", "-s"]).unwrap());
        lines
    }
    /// References and HEAD to the id.
    fn exact(&self) -> Vec<String> {
        let git = self.git();
        let mut lines = git
            .user_refs("%(objectname) %(objecttype)%09%(refname)")
            .unwrap();
        lines.push(git.rev_parse("HEAD").unwrap());
        lines.push(git.run(["rev-parse", "--abbrev-ref", "HEAD"]).unwrap());
        lines
    }
    fn tag(&self, name: &str) -> Option<String> {
        self.git().rev_parse(&format!("refs/tags/{name}")).ok()
    }
    fn journal_file(&self) -> PathBuf {
        self.scratch
            .path()
            .join(".git")
            .join("bitbraid")
            .join("journal.json")
    }
    /// One stack of the journal as (id, label, kind), in the order listed.
    fn stack(&mut self, which: &str) -> Vec<(u64, String, String)> {
        self.run("undo.journal", json!({}))[which]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| {
                (
                    row["id"].as_u64().unwrap(),
                    row["label"].as_str().unwrap().to_owned(),
                    row["kind"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }
    /// The id of the entry with this label.
    fn id_of(&mut self, label: &str) -> u64 {
        let mut rows = self.stack("undo");
        rows.extend(self.stack("redo"));
        rows.iter()
            .find(|(_, row, _)| row.starts_with(label))
            .unwrap_or_else(|| panic!("no entry {label:?} in {rows:?}"))
            .0
    }
}

fn labels(value: &Value) -> Vec<&str> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label.as_str().unwrap())
        .collect()
}

/// A label without the id of a commit made here: two copies that commit
/// the same at different seconds get different ids.
fn plain(label: &str) -> String {
    let summary = label
        .strip_prefix("Commit ")
        .and_then(|rest| rest.split_once(':'));
    match summary {
        Some((_, summary)) => format!("Commit *:{summary}"),
        None => label.to_owned(),
    }
}

/// A stack with its labels made comparable between two copies.
fn comparable(stack: Vec<(u64, String, String)>) -> Vec<(u64, String, String)> {
    stack
        .into_iter()
        .map(|(id, label, kind)| (id, plain(&label), kind))
        .collect()
}

/// Take one step on the twin and hold its label against ours.
fn step(twin: &mut Side, command: &str, key: &str, ours: &str) {
    let out = twin.run(command, json!({}));
    assert_eq!(plain(out[key].as_str().unwrap()), plain(ours), "{command}");
}

/// Several kinds of change, one after another: two commits, branches with
/// and without a checkout, tags of both kinds, a rename and a deletion.
fn act(side: &mut Side) {
    side.scratch.write("a.txt", "2\none\n").unwrap();
    side.run(
        "commit.create",
        json!({ "message": "one", "stage_all": true }),
    );
    side.run("branch.create", json!({ "name": "topic" }));
    side.run("tag.create", json!({ "name": "v1" }));
    side.scratch.write("b.txt", "bb\ntwo\n").unwrap();
    side.run(
        "commit.create",
        json!({ "message": "two", "stage_all": true }),
    );
    side.run("tag.create", json!({ "name": "v2", "message": "notes" }));
    side.run("branch.create", json!({ "name": "side", "checkout": true }));
    side.run(
        "branch.rename",
        json!({ "name": "topic", "new_name": "renamed" }),
    );
    side.run("tag.delete", json!({ "name": "v1" }));
}

#[test]
fn the_journal_lists_both_stacks_newest_first() {
    let mut h = Side::new();
    let empty = h.run("undo.journal", json!({}));
    assert_eq!(
        empty,
        json!({ "undo": [], "redo": [], "next_undo": null, "next_redo": null })
    );

    let before = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    act(&mut h);
    let listed = h.run("undo.journal", json!({ "junk": [1] }));
    let rows = listed["undo"].as_array().unwrap();
    let kinds: Vec<&str> = rows.iter().map(|r| r["kind"].as_str().unwrap()).collect();
    assert_eq!(
        kinds,
        [
            "tag_delete",
            "branch_rename",
            "branch_create",
            "tag_create",
            "commit",
            "tag_create",
            "branch_create",
            "commit"
        ]
    );
    let ids: Vec<u64> = rows.iter().map(|r| r["id"].as_u64().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "{ids:?}");
    assert_eq!(rows[0]["label"], "Delete tag v1");
    assert_eq!(rows[5]["label"], "Tag v1");
    assert_eq!(rows[6]["label"], "Create branch topic");
    assert!(rows[7]["label"].as_str().unwrap().ends_with(": one"));
    for row in rows {
        let time = row["time"].as_i64().unwrap();
        assert!((before..=before + 600).contains(&time), "{row}");
        assert_eq!(row.as_object().unwrap().len(), 4, "id, label, time, kind");
    }
    assert_eq!(listed["redo"], json!([]));
    assert_eq!(listed["next_undo"], ids[0]);
    assert_eq!(listed["next_redo"], Value::Null);
    assert_eq!(h.run("undo.peek", json!({}))["undo"], rows[0]["label"]);

    // Undone entries move over. Read as one list, redo first, it is still
    // every entry from the newest to the oldest; redo takes the one next
    // to where the repository is, which is the last of its list.
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    let listed = h.run("undo.journal", json!({}));
    let all: Vec<u64> = listed["redo"]
        .as_array()
        .unwrap()
        .iter()
        .chain(listed["undo"].as_array().unwrap())
        .map(|row| row["id"].as_u64().unwrap())
        .collect();
    assert_eq!(all, ids);
    assert_eq!(listed["redo"].as_array().unwrap().len(), 3);
    assert_eq!(listed["redo"][0]["kind"], "tag_delete");
    assert_eq!(listed["next_redo"], ids[2]);
    assert_eq!(listed["next_undo"], ids[3]);
    let peek = h.run("undo.peek", json!({}));
    assert_eq!(peek["redo"], listed["redo"][2]["label"]);
    assert_eq!(peek["undo"], listed["undo"][0]["label"]);

    // It is the journal on disk: another session on the repository lists
    // the same.
    let mut again = CommandContext::default();
    h.registry
        .run(
            "repo.open",
            &mut again,
            &json!({ "path": h.scratch.path() }),
        )
        .unwrap();
    let reopened = h.registry.run("undo.journal", &mut again, &json!({}));
    assert_eq!(reopened.unwrap(), listed);
}

#[test]
fn undo_to_and_redo_to_match_single_steps_on_a_twin() {
    let mut ours = Side::new();
    let mut twin = Side::new();
    let start = ours.exact();
    assert_eq!(start, twin.exact());
    act(&mut ours);
    act(&mut twin);
    assert_eq!(ours.state(), twin.state());
    let done = ours.exact();
    let journal = ours.stack("undo");
    assert_eq!(journal.len(), 8);
    let newest = journal[0].0;
    let oldest = journal[7].0;

    // Back to before the first tag: six entries of five kinds.
    let target = ours.id_of("Tag v1");
    assert_eq!(target, journal[5].0);
    let out = ours.run("undo.to", json!({ "id": target }));
    let expected: Vec<&str> = journal[..6].iter().map(|row| row.1.as_str()).collect();
    assert_eq!(labels(&out["undone"]), expected);
    assert_eq!(out["complete"], true);
    assert_eq!(out["error"], Value::Null);
    assert_eq!(out["failed"], Value::Null);
    for label in &expected {
        step(&mut twin, "undo.last", "undone", label);
    }
    assert_eq!(ours.state(), twin.state(), "after undo.to");
    for which in ["undo", "redo"] {
        assert_eq!(
            comparable(ours.stack(which)),
            comparable(twin.stack(which)),
            "{which}"
        );
    }
    assert_eq!(ours.stack("undo"), journal[6..]);
    assert_eq!(ours.stack("redo"), journal[..6]);
    // What it came to: the first commit and the branch made after it.
    let git = ours.git();
    assert_eq!(git.refs("refs/heads").unwrap(), ["main", "topic"]);
    assert!(git.refs("refs/tags").unwrap().is_empty());
    assert_eq!(git.run(["log", "-1", "--format=%s"]).unwrap().trim(), "one");

    // Forward again, all the way: the same six, the other way round.
    let out = ours.run("redo.to", json!({ "id": newest }));
    let expected: Vec<&str> = journal[..6].iter().rev().map(|r| r.1.as_str()).collect();
    assert_eq!(labels(&out["redone"]), expected);
    assert_eq!(out["complete"], true);
    assert_eq!(out["error"], Value::Null);
    for label in &expected {
        step(&mut twin, "redo.last", "redone", label);
    }
    assert_eq!(ours.state(), twin.state(), "after redo.to");
    assert_eq!(
        ours.exact(),
        done,
        "the same commits and the same tag object"
    );
    assert_eq!(ours.stack("undo"), journal);
    assert!(ours.stack("redo").is_empty());

    // One entry is one step, as with undo.last.
    let out = ours.run("undo.to", json!({ "id": newest }));
    assert_eq!(labels(&out["undone"]), ["Delete tag v1"]);
    step(&mut twin, "undo.last", "undone", "Delete tag v1");
    assert_eq!(ours.state(), twin.state());

    // Everything: the references are where they were before any of it.
    let out = ours.run("undo.to", json!({ "id": oldest }));
    assert_eq!(labels(&out["undone"]).len(), 7);
    assert_eq!(out["complete"], true);
    while !twin.run("undo.last", json!({}))["undone"].is_null() {}
    assert_eq!(ours.state(), twin.state(), "after undoing everything");
    assert_eq!(ours.exact(), start);
    assert!(ours.stack("undo").is_empty());
    assert_eq!(ours.stack("redo"), journal);
    // And part of the way forward: redo.to stops at the entry named.
    let out = ours.run("redo.to", json!({ "id": journal[6].0 }));
    assert_eq!(
        labels(&out["redone"]),
        [journal[7].1.as_str(), journal[6].1.as_str()]
    );
    step(&mut twin, "redo.last", "redone", &journal[7].1);
    step(&mut twin, "redo.last", "redone", &journal[6].1);
    assert_eq!(ours.state(), twin.state(), "after redoing two");
    assert_eq!(
        comparable(ours.stack("redo")),
        comparable(twin.stack("redo"))
    );
}

#[test]
fn undo_to_stops_where_a_step_cannot_be_taken() {
    let mut h = Side::new();
    let git = h.git();
    h.run("tag.create", json!({ "name": "first" }));
    h.run("tag.create", json!({ "name": "moving" }));
    h.run("branch.create", json!({ "name": "topic" }));
    h.run("tag.create", json!({ "name": "last" }));
    let (first, moving) = (h.id_of("Tag first"), h.id_of("Tag moving"));
    let (topic, last) = (h.id_of("Create branch topic"), h.id_of("Tag last"));
    let first_value = h.tag("first");

    // A tag made here and moved by hand since: undo must not delete it.
    git.run(["tag", "-f", "moving", "HEAD~2"]).unwrap();
    let moved = h.tag("moving");
    let out = h.run("undo.to", json!({ "id": first }));
    assert_eq!(labels(&out["undone"]), ["Tag last", "Create branch topic"]);
    assert_eq!(out["complete"], false);
    assert_eq!(
        out["failed"],
        json!({ "id": moving, "label": "Tag moving" })
    );
    let error = out["error"].as_str().unwrap();
    assert!(error.contains("not where this action left it"), "{error}");
    // What was undone stays undone; the rest is as it was.
    assert_eq!(h.tag("last"), None);
    assert_eq!(git.refs("refs/heads").unwrap(), ["main"]);
    assert_eq!(h.tag("moving"), moved);
    assert_eq!(h.tag("first"), first_value);
    let listed = h.run("undo.journal", json!({}));
    assert_eq!(listed["next_undo"], moving);
    assert_eq!(listed["next_redo"], topic);
    let ids = |rows: &Value| -> Vec<u64> {
        let rows = rows.as_array().unwrap();
        rows.iter().map(|row| row["id"].as_u64().unwrap()).collect()
    };
    assert_eq!(ids(&listed["undo"]), [moving, first]);
    assert_eq!(ids(&listed["redo"]), [last, topic]);

    // The step that fails being the first one: nothing is undone, and it
    // is still an answer.
    let refs = h.exact();
    let out = h.run("undo.to", json!({ "id": moving }));
    assert_eq!(out["undone"], json!([]));
    assert_eq!(out["complete"], false);
    assert_eq!(out["failed"]["id"], moving);
    assert_eq!(h.exact(), refs);

    // Forward again, to the end.
    let out = h.run("redo.to", json!({ "id": last }));
    assert_eq!(labels(&out["redone"]), ["Create branch topic", "Tag last"]);
    assert_eq!(out["complete"], true);
    assert!(h.tag("last").is_some());
    assert_eq!(git.refs("refs/heads").unwrap(), ["main", "topic"]);

    // Redo stops the same way: the branch it would make is there already,
    // somewhere else.
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    git.run(["branch", "topic", "HEAD~1"]).unwrap();
    let by_hand = git.rev_parse("refs/heads/topic").unwrap();
    let out = h.run("redo.to", json!({ "id": last }));
    assert_eq!(out["redone"], json!([]));
    assert_eq!(out["complete"], false);
    assert_eq!(
        out["failed"],
        json!({ "id": topic, "label": "Create branch topic" })
    );
    assert!(!out["error"].as_str().unwrap().is_empty());
    assert_eq!(git.rev_parse("refs/heads/topic").unwrap(), by_hand);
    assert_eq!(h.tag("last"), None);
    assert_eq!(h.run("undo.journal", json!({}))["next_redo"], topic);
    // Out of the way, it goes through.
    git.run(["branch", "-D", "topic"]).unwrap();
    let out = h.run("redo.to", json!({ "id": last }));
    assert_eq!(out["complete"], true);
    assert_ne!(git.rev_parse("refs/heads/topic").unwrap(), by_hand);
    assert!(h.tag("last").is_some());
}

#[test]
fn hostile_ids_and_a_broken_journal_change_nothing() {
    let mut h = Side::new();
    let git = h.git();
    h.run("tag.create", json!({ "name": "one" }));
    h.run("tag.create", json!({ "name": "two" }));
    h.run("tag.create", json!({ "name": "three" }));
    h.run("undo.last", json!({}));
    let (one, three) = (h.id_of("Tag one"), h.id_of("Tag three"));
    let refs = h.exact();
    let file = h.journal_file();
    let stored = std::fs::read_to_string(&file).unwrap();

    // Ids that are no ids, or name nothing on the stack asked about.
    for id in [
        json!(null),
        json!("1"),
        json!(-1),
        json!(1.5),
        json!(1e30),
        json!([1]),
        json!({ "id": 1 }),
        json!(true),
        json!(0),
        json!(4),
        json!(999_999),
        json!(u64::MAX),
    ] {
        for command in ["undo.to", "redo.to"] {
            assert!(
                h.try_run(command, json!({ "id": id })).is_err(),
                "{command} took the id {id}"
            );
        }
    }
    for params in [json!({}), json!(null), json!([1]), json!("1"), json!(1)] {
        for command in ["undo.to", "redo.to"] {
            assert!(h.try_run(command, params.clone()).is_err(), "{params}");
        }
    }
    // An entry of the other stack is refused before anything is done.
    let err = h.try_run("undo.to", json!({ "id": three })).unwrap_err();
    assert!(err.contains("nothing was changed"), "{err}");
    let err = h.try_run("redo.to", json!({ "id": one })).unwrap_err();
    assert!(err.contains("nothing was changed"), "{err}");
    assert_eq!(h.exact(), refs);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), stored);

    // No repository open.
    let mut nothing = CommandContext::default();
    for command in ["undo.journal", "undo.to", "redo.to"] {
        let out = h.registry.run(command, &mut nothing, &json!({ "id": 1 }));
        assert!(out.is_err(), "{command}");
    }

    // A journal that cannot be read is an empty one.
    let empty = json!({ "undo": [], "redo": [], "next_undo": null, "next_redo": null });
    for garbage in ["", "{{{{ not json", "[]", "null", "{\"done\": 3}", "\0\0\0"] {
        std::fs::write(&file, garbage).unwrap();
        h.ctx.reload_journal();
        assert_eq!(h.run("undo.journal", json!({})), empty, "{garbage:?}");
        assert!(h.try_run("undo.to", json!({ "id": one })).is_err());
        assert!(h.try_run("redo.to", json!({ "id": three })).is_err());
    }
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    h.ctx.reload_journal();
    assert_eq!(h.run("undo.journal", json!({})), empty);
    std::fs::remove_dir(&file).unwrap();

    // A journal somebody wrote by hand: entries that reverse nothing that
    // is there, ids that repeat, numbers at their limits. Every step is
    // refused by the checks a single undo makes, and nothing moves.
    let zero = "0".repeat(40);
    let long = "l".repeat(100_000);
    let entry = |id: u64, change: Value| json!({ "id": id, "label": format!("hostile {id}"), "time": i64::MIN, "change": change });
    let hostile = vec![
        entry(
            7,
            json!({ "kind": "tag_create", "name": "one", "value": zero }),
        ),
        entry(
            7,
            json!({ "kind": "tag_create", "name": "-d\nx", "value": zero }),
        ),
        entry(
            0,
            json!({ "kind": "tag_delete", "name": "one", "value": zero }),
        ),
        entry(
            u64::MAX,
            json!({ "kind": "commit", "branch": null, "previous": null, "id": zero, "amend": false }),
        ),
        entry(
            3,
            json!({ "kind": "branch_create", "name": "../../outside", "target": zero, "switched_from": null }),
        ),
        entry(
            4,
            json!({ "kind": "branch_rename", "name": "--force", "new_name": "-D" }),
        ),
        entry(
            5,
            json!({ "kind": "branch_delete", "name": "main", "target": zero, "upstream": "-x/y" }),
        ),
        entry(
            6,
            json!({ "kind": "branch_move", "name": "main", "before": zero, "after": zero }),
        ),
        json!({ "id": 8, "label": long, "time": i64::MAX, "change": { "kind": "upstream", "name": "-x", "before": "--exec=x", "after": null } }),
    ];
    let written = json!({ "next_id": u64::MAX, "done": hostile, "undone": hostile });
    std::fs::write(&file, serde_json::to_string(&written).unwrap()).unwrap();
    h.ctx.reload_journal();
    let listed = h.run("undo.journal", json!({}));
    assert_eq!(listed["undo"].as_array().unwrap().len(), hostile.len());
    assert_eq!(listed["redo"].as_array().unwrap().len(), hostile.len());
    assert_eq!(listed["next_undo"], 8);
    for id in [8, 6, 5, 4, 3, u64::MAX, 0, 7] {
        for (command, key) in [("undo.to", "undone"), ("redo.to", "redone")] {
            let out = h.run(command, json!({ "id": id }));
            assert_eq!(out[key], json!([]), "{command} {id}: {out}");
            assert_eq!(out["complete"], false, "{command} {id}");
            assert_eq!(out["failed"]["id"], 8, "{command} {id}");
            assert!(!out["error"].as_str().unwrap().is_empty());
        }
    }
    assert_eq!(h.exact(), refs);
    assert_eq!(git.lines(["status", "--porcelain"]).unwrap(), [""; 0]);
    assert!(!h.scratch.path().join("x").exists());

    // The same with the entries one by one on top, so that each is the
    // step that is tried.
    for at in 0..hostile.len() {
        let stack = &hostile[..=at];
        let written = json!({ "next_id": 9, "done": stack, "undone": stack });
        std::fs::write(&file, serde_json::to_string(&written).unwrap()).unwrap();
        h.ctx.reload_journal();
        let id = hostile[at]["id"].clone();
        for (command, key) in [("undo.to", "undone"), ("redo.to", "redone")] {
            let out = h.run(command, json!({ "id": id }));
            assert_eq!(out[key], json!([]), "{command} of entry {at}: {out}");
            assert_eq!(out["complete"], false, "{command} of entry {at}");
        }
        assert_eq!(h.exact(), refs, "entry {at}");
    }

    // Very many entries: listed, and gone through no further than the
    // first that fails.
    let many: Vec<Value> = (0..5000u64)
        .map(|id| {
            entry(
                id,
                json!({ "kind": "tag_create", "name": "absent", "value": zero }),
            )
        })
        .collect();
    let written = json!({ "next_id": 1, "done": many, "undone": [] });
    std::fs::write(&file, serde_json::to_string(&written).unwrap()).unwrap();
    h.ctx.reload_journal();
    assert_eq!(
        h.run("undo.journal", json!({}))["undo"]
            .as_array()
            .unwrap()
            .len(),
        5000
    );
    let out = h.run("undo.to", json!({ "id": 0 }));
    assert_eq!(out["undone"], json!([]));
    assert_eq!(out["failed"]["id"], 4999);
    assert_eq!(h.exact(), refs);
}
