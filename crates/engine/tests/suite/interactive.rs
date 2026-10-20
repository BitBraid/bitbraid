//! Interactive rebase: plans carried out by the engine against the same
//! history made by hand with git on a twin copy (reset, cherry-pick and
//! amend, never a rebase), the guard rails, and plans that stop half-way.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Five commits on top of the fixture: `one` adds a file that `four`
/// changes, the others each add a file of their own. `two` has a body and
/// `#3 three` begins like a comment.
fn work(copy: &Scratch) {
    let git = copy.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    let commit = |file: &str, content: &str, message: &str| {
        copy.write(file, content).unwrap();
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "--cleanup=verbatim", "-m", message])
            .unwrap();
    };
    commit("one.txt", "1\n", "one");
    commit("two.txt", "2\n", "two\n\nbody of two");
    commit("three.txt", "3\n", "#3 three");
    commit("one.txt", "1\n4\n", "four");
    commit("five.txt", "5\n", "five");
}

/// The commit with this subject in the history of HEAD.
fn id_of(git: &Git, subject: &str) -> String {
    git.lines(["log", "--format=%H %s", "HEAD"])
        .unwrap()
        .into_iter()
        .find_map(|line| {
            let (id, s) = line.split_once(' ')?;
            (s == subject).then(|| id.to_owned())
        })
        .unwrap_or_else(|| panic!("no commit called {subject:?}"))
}

/// Everything about the history that does not depend on when it was made:
/// tree, number of parents, author, author date and the whole message.
fn shape(git: &Git) -> String {
    git.run(["log", "--format=%T %p|%an <%ae> %ad%n%B%x00", "HEAD"])
        .unwrap()
        .split('\0')
        .map(|record| {
            let (head, rest) = record.trim_start().split_once('|').unwrap_or(("", record));
            let mut ids = head.split(' ');
            let tree = ids.next().unwrap_or("");
            format!("{tree} {} {rest}", ids.filter(|p| !p.is_empty()).count())
        })
        .collect::<Vec<_>>()
        .join("\n--\n")
}

fn status(git: &Git) -> String {
    git.run(["status", "--porcelain"]).unwrap()
}

fn step(id: &str, action: &str) -> Value {
    json!({ "id": id, "action": action })
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        work(&scratch);
        work(&twin);
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
    /// Ids of the five commits in the scratch copy, oldest first.
    fn ids(&self) -> Vec<String> {
        let git = self.git();
        ["one", "two", "#3 three", "four", "five"]
            .iter()
            .map(|subject| id_of(&git, subject))
            .collect()
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(
            shape(&self.git()),
            shape(&self.twin.git()),
            "{what}: history"
        );
        assert_eq!(
            status(&self.git()),
            status(&self.twin.git()),
            "{what}: status"
        );
        assert_eq!(
            self.git().run(["ls-files", "--stage"]).unwrap(),
            self.twin.git().run(["ls-files", "--stage"]).unwrap(),
            "{what}: index"
        );
    }
    fn plan_dir(&self) -> std::path::PathBuf {
        self.scratch.path().join(".git/bitbraid/rebase")
    }
}

#[test]
fn the_range_is_what_git_log_lists() {
    let mut h = Harness::new();
    let git = h.git();
    let ids = h.ids();
    let plan = h.run("rebase.plan", json!({ "first": ids[0] }));
    assert_eq!(plan["branch"], "main");
    assert!(plan["upstream"].is_null());
    assert_eq!(
        plan["base"].as_str().unwrap(),
        git.rev_parse("HEAD~5").unwrap()
    );
    let commits = plan["commits"].as_array().unwrap();
    let listed: Vec<&str> = commits.iter().map(|c| c["id"].as_str().unwrap()).collect();
    let mut logged = git.lines(["log", "--format=%H", "HEAD~5..HEAD"]).unwrap();
    logged.reverse();
    assert_eq!(listed, logged, "oldest first");
    assert_eq!(commits[1]["summary"], "two");
    assert_eq!(commits[1]["message"], "two\n\nbody of two");
    assert_eq!(commits[2]["summary"], "#3 three");
    assert_eq!(commits[0]["author"], "Ada Fixture");
    assert!(commits.iter().all(|c| c["published"] == false));
    // By name, by a shorter id, and from HEAD itself.
    let by_name = h.run("rebase.plan", json!({ "first": "HEAD~4" }));
    assert_eq!(by_name, plan);
    let short = &ids[0][..10];
    assert_eq!(h.run("rebase.plan", json!({ "first": short })), plan);
    let last = h.run("rebase.plan", json!({ "first": "HEAD" }));
    assert_eq!(last["commits"].as_array().unwrap().len(), 1);

    // What the upstream has already is marked.
    git.run(["branch", "up", ids[1].as_str()]).unwrap();
    git.run(["branch", "-q", "--set-upstream-to=up"]).unwrap();
    let plan = h.run("rebase.plan", json!({ "first": ids[0] }));
    assert_eq!(plan["upstream"], "up");
    let published: Vec<bool> = plan["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["published"].as_bool().unwrap())
        .collect();
    assert_eq!(published, [true, true, false, false, false]);
}

#[test]
fn a_plan_is_carried_out_like_the_same_done_by_hand() {
    let mut h = Harness::new();
    let ids = h.ids();
    let start = h.git().rev_parse("HEAD").unwrap();
    let reworded = "ONE\n\nwith a body\n# and a line like a comment";
    let steps = json!([
        step(&ids[2], "pick"),
        { "id": ids[0], "action": "reword", "message": reworded },
        step(&ids[1], "squash"),
        step(&ids[4], "fixup"),
        step(&ids[3], "pick"),
    ]);

    // What it leads to, before anything is done.
    let outline = h.run("rebase.outline", json!({ "first": ids[0], "steps": steps }));
    let result = outline["result"].as_array().unwrap();
    assert_eq!(result.len(), 3);
    assert_eq!(result[0]["message"], "#3 three");
    assert_eq!(
        result[1]["message"],
        format!("{reworded}\n\ntwo\n\nbody of two")
    );
    assert_eq!(result[1]["from"], json!([ids[0], ids[1], ids[4]]));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);

    let out = h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    assert!(out.get("stopped").is_none(), "{out}");

    // By hand on the twin: reset, pick, amend. No rebase.
    let twin = h.twin.git();
    let t: Vec<String> = ["one", "two", "#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", "HEAD~5"]).unwrap();
    twin.run(["cherry-pick", t[2].as_str()]).unwrap();
    twin.run(["cherry-pick", t[0].as_str()]).unwrap();
    twin.run(["cherry-pick", "-n", t[1].as_str()]).unwrap();
    twin.run(["cherry-pick", "-n", t[4].as_str()]).unwrap();
    let message = format!("{reworded}\n\ntwo\n\nbody of two\n");
    twin.run([
        "commit",
        "-q",
        "--amend",
        "--cleanup=verbatim",
        "-m",
        message.as_str(),
    ])
    .unwrap();
    twin.run(["cherry-pick", t[3].as_str()]).unwrap();
    h.assert_same("the plan");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%B", "HEAD~1"]).unwrap(),
        format!("{message}\n"),
        "the line that looks like a comment is still there"
    );
    assert!(!h.plan_dir().exists(), "nothing is left behind");
    assert!(h.run("op.status", json!({}))["operation"].is_null());

    // One step for undo, and back again.
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Interactive rebase of 5 commits"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    h.run("redo.last", json!({}));
    h.assert_same("redo");
}

#[test]
fn a_plan_is_replayed_on_another_base() {
    let mut h = Harness::new();
    // Another line of history from where the five commits begin.
    for copy in [&h.scratch, &h.twin] {
        let git = copy.git();
        git.run(["switch", "-q", "-c", "elsewhere", "HEAD~5"])
            .unwrap();
        copy.write("else.txt", "else\n").unwrap();
        git.run(["add", "else.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "elsewhere"]).unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
    }
    let ids = h.ids();
    let start = h.git().rev_parse("HEAD").unwrap();
    let base = h.git().rev_parse("elsewhere").unwrap();

    // The plan for a new base is for the commits that base does not have.
    let plan = h.run("rebase.plan", json!({ "onto": "elsewhere" }));
    assert_eq!(plan["first"], ids[0]);
    assert_eq!(plan["onto"], base);
    let planned: Vec<&str> = plan["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["summary"].as_str().unwrap())
        .collect();
    assert_eq!(
        planned,
        h.git()
            .lines(["log", "--reverse", "--format=%s", "elsewhere..HEAD"])
            .unwrap()
    );

    let steps = json!([
        step(&ids[0], "pick"),
        step(&ids[1], "pick"),
        step(&ids[2], "drop"),
        step(&ids[3], "pick"),
        step(&ids[4], "fixup"),
    ]);
    let out = h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps, "onto": "elsewhere" }),
    );
    assert!(out.get("stopped").is_none(), "{out}");

    // By hand on the twin: the branch put on the new base, then picks.
    let twin = h.twin.git();
    let t: Vec<String> = ["one", "two", "#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", "elsewhere"]).unwrap();
    twin.run(["cherry-pick", t[0].as_str()]).unwrap();
    twin.run(["cherry-pick", t[1].as_str()]).unwrap();
    twin.run(["cherry-pick", t[3].as_str()]).unwrap();
    twin.run(["cherry-pick", "-n", t[4].as_str()]).unwrap();
    twin.run(["commit", "-q", "--amend", "--no-edit"]).unwrap();
    h.assert_same("the plan on another base");
    assert_eq!(h.git().rev_parse("HEAD~3").unwrap(), base);
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Interactive rebase of 5 commits onto elsewhere"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");

    // A plan that changes no commit still moves them to the new base.
    let untouched: Vec<Value> = ids.iter().map(|id| step(id, "pick")).collect();
    h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": untouched, "onto": "elsewhere" }),
    );
    twin.run(["reset", "-q", "--hard", "elsewhere"]).unwrap();
    for id in &t {
        twin.run(["cherry-pick", id.as_str()]).unwrap();
    }
    h.assert_same("every commit, on another base");
    h.run("undo.last", json!({}));

    // Onto where they are is no move, and a base that has everything
    // leaves nothing to plan.
    let before = h.run("undo.peek", json!({}));
    h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": untouched, "onto": "HEAD~5" }),
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(h.run("undo.peek", json!({})), before);
    let err = h
        .try_run("rebase.plan", json!({ "onto": "HEAD" }))
        .unwrap_err();
    assert!(err.contains("nothing to rebase"), "{err}");
    let err = h.try_run("rebase.plan", json!({})).unwrap_err();
    assert!(err.contains("first"), "{err}");
    // What does not name a commit, or could be read as an option.
    for onto in ["", "-x", "--exec=touch pwned", "no-such-rev", "a\nb"] {
        assert!(
            h.try_run("rebase.plan", json!({ "onto": onto })).is_err(),
            "{onto:?}"
        );
        assert!(
            h.try_run(
                "rebase.interactive",
                json!({ "first": ids[0], "steps": untouched, "onto": onto })
            )
            .is_err(),
            "{onto:?}"
        );
    }
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    assert!(!h.scratch.path().join("pwned").exists());
}

#[test]
fn a_plan_that_changes_nothing_does_nothing() {
    let mut h = Harness::new();
    let ids = h.ids();
    let start = h.git().rev_parse("HEAD").unwrap();
    let steps: Vec<Value> = ids.iter().map(|id| step(id, "pick")).collect();
    let out = h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    assert_eq!(out["before"], out["after"]);
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    // A reword to the words it has is the same plan to git, and the commit
    // ids show it: nothing is rewritten.
    let mut steps = steps;
    steps[1] = json!({ "id": ids[1], "action": "reword", "message": "two\n\nbody of two\n" });
    h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
}

#[test]
fn merges_the_first_commit_and_other_histories_are_refused() {
    let mut h = Harness::new();
    let git = h.git();
    let ids = h.ids();
    let root = git
        .run(["rev-list", "--max-parents=0", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();

    // From the first commit of the history.
    let err = h
        .try_run("rebase.plan", json!({ "first": root }))
        .unwrap_err();
    assert!(err.contains("first commit of the history"), "{err}");
    assert!(err.contains(&root[..7]), "{err}");

    // A commit that is not in the branch's history.
    git.run(["switch", "-q", "-c", "other", "HEAD~3"]).unwrap();
    h.scratch.write("other.txt", "o\n").unwrap();
    git.run(["add", "other.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "elsewhere"]).unwrap();
    let elsewhere = git.rev_parse("HEAD").unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    let err = h
        .try_run("rebase.plan", json!({ "first": elsewhere }))
        .unwrap_err();
    assert!(err.contains("not in the history of main"), "{err}");

    // A merge commit in the range, and as the place to begin.
    git.run(["merge", "-q", "--no-ff", "--no-edit", "other"])
        .unwrap();
    let merge = git.rev_parse("HEAD").unwrap();
    for first in [ids[0].as_str(), merge.as_str()] {
        let err = h
            .try_run("rebase.plan", json!({ "first": first }))
            .unwrap_err();
        assert!(err.contains("is a merge commit"), "{err}");
        assert!(err.contains(&merge[..7]), "{err}");
    }
    let steps: Vec<Value> = ids.iter().map(|id| step(id, "drop")).collect();
    assert!(
        h.try_run(
            "rebase.interactive",
            json!({ "first": ids[0], "steps": steps })
        )
        .is_err()
    );
    assert_eq!(git.rev_parse("HEAD").unwrap(), merge);

    // After the merge, plans are fine again.
    h.scratch.write("after.txt", "a\n").unwrap();
    git.run(["add", "after.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "after the merge"]).unwrap();
    let plan = h.run("rebase.plan", json!({ "first": "HEAD" }));
    assert_eq!(plan["commits"].as_array().unwrap().len(), 1);

    // Not on a branch.
    git.run(["switch", "-q", "--detach"]).unwrap();
    let err = h
        .try_run("rebase.plan", json!({ "first": "HEAD" }))
        .unwrap_err();
    assert!(err.contains("not on a branch"), "{err}");
}

#[test]
fn hostile_plans_change_nothing() {
    let mut h = Harness::new();
    let git = h.git();
    let ids = h.ids();
    let start = git.rev_parse("HEAD").unwrap();
    let picks: Vec<Value> = ids.iter().map(|id| step(id, "pick")).collect();
    let with = |index: usize, value: Value| {
        let mut steps = picks.clone();
        steps[index] = value;
        Value::Array(steps)
    };
    let pwned = h.scratch.path().join("pwned");
    let exec = format!("--exec=touch {}", pwned.display());
    for first in [
        "",
        "-i",
        "--root",
        exec.as_str(),
        "HEAD\n",
        "no-such",
        "HEAD:one.txt",
    ] {
        assert!(
            h.try_run("rebase.plan", json!({ "first": first })).is_err(),
            "{first:?}"
        );
        assert!(
            h.try_run(
                "rebase.interactive",
                json!({ "first": first, "steps": picks })
            )
            .is_err(),
            "{first:?}"
        );
    }
    let foreign = "ab".repeat(20);
    let many: Vec<Value> = (0..5000).map(|_| step(&ids[0], "pick")).collect();
    for steps in [
        json!("drop everything"),
        json!(null),
        json!([]),
        json!([{}]),
        json!([{ "id": "zz", "action": "pick" }]),
        json!(picks[..4]),
        Value::Array(many),
        with(1, step(&ids[0], "pick")),
        with(1, step(&foreign, "pick")),
        with(1, step(&ids[1], "explode")),
        with(
            1,
            json!({ "id": ids[1], "action": "exec", "message": "touch pwned" }),
        ),
        with(0, step(&ids[0], "squash")),
        with(0, step(&ids[0], "fixup")),
        with(2, step(&ids[2], "reword")),
        with(
            2,
            json!({ "id": ids[2], "action": "reword", "message": "" }),
        ),
        with(2, json!({ "id": ids[2], "action": "reword", "message": 7 })),
        with(
            2,
            json!({ "id": ids[2], "action": "reword", "message": "x".repeat((1 << 20) + 1) }),
        ),
    ] {
        let params = json!({ "first": ids[0], "steps": steps });
        assert!(
            h.try_run("rebase.interactive", params.clone()).is_err(),
            "{steps}"
        );
        assert!(h.try_run("rebase.outline", params).is_err());
    }
    assert!(h.try_run("rebase.interactive", json!({})).is_err());
    // A message cannot smuggle a line into the todo list.
    let sly = format!("fine\nexec touch {}\n", pwned.display());
    h.run(
        "rebase.interactive",
        json!({ "first": ids[4], "steps": [{ "id": ids[4], "action": "reword", "message": sly }] }),
    );
    assert!(!pwned.exists());
    assert_eq!(
        git.run(["log", "-1", "--format=%B"]).unwrap().trim_end(),
        sly.trim_end()
    );
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&git), "");

    // Local changes: git refuses, and they are still there.
    h.scratch.write("one.txt", "not committed\n").unwrap();
    let steps = with(4, step(&ids[4], "drop"));
    assert!(
        h.try_run(
            "rebase.interactive",
            json!({ "first": ids[0], "steps": steps })
        )
        .is_err()
    );
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&git), " M one.txt\n");
    assert!(h.run("op.status", json!({}))["operation"].is_null());
}

#[test]
fn a_plan_that_conflicts_stops_and_goes_on() {
    let mut h = Harness::new();
    let ids = h.ids();
    let start = h.git().rev_parse("HEAD").unwrap();
    // Without `one`, `four` changes a file that is not there.
    let steps = json!([
        step(&ids[0], "drop"),
        { "id": ids[1], "action": "reword", "message": "TWO" },
        step(&ids[2], "pick"),
        step(&ids[3], "pick"),
        { "id": ids[4], "action": "reword", "message": "FIVE" },
    ]);
    let out = h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    assert!(out["stopped"].is_string(), "{out}");
    assert_eq!(out["conflicts"], json!(["one.txt"]));
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "rebase");
    assert_eq!(operation["branch"], "main");
    assert_eq!(operation["current"].as_str().unwrap(), ids[3]);
    assert_eq!(operation["editing"], false);
    assert!(h.plan_dir().join("message-1").exists(), "still needed");
    // Nothing else can begin on top of it.
    assert!(
        h.try_run(
            "rebase.interactive",
            json!({ "first": "HEAD", "steps": [step("HEAD", "drop")] })
        )
        .is_err()
    );

    h.run(
        "conflict.take",
        json!({ "path": "one.txt", "side": "theirs" }),
    );
    let out = h.run("op.continue", json!({}));
    assert_eq!(out["finished"], true, "{out}");

    let twin = h.twin.git();
    let t: Vec<String> = ["one", "two", "#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", "HEAD~5"]).unwrap();
    twin.run(["cherry-pick", t[1].as_str()]).unwrap();
    twin.run(["commit", "-q", "--amend", "-m", "TWO"]).unwrap();
    twin.run(["cherry-pick", t[2].as_str()]).unwrap();
    assert!(twin.run(["cherry-pick", t[3].as_str()]).is_err());
    twin.run(["add", "one.txt"]).unwrap();
    twin.run(["-c", "core.editor=true", "cherry-pick", "--continue"])
        .unwrap();
    twin.run(["cherry-pick", t[4].as_str()]).unwrap();
    twin.run(["commit", "-q", "--amend", "-m", "FIVE"]).unwrap();
    h.assert_same("after the conflict");

    // The whole of it is one step for undo.
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Interactive rebase of 5 commits"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);

    // And one that is given up leaves no trace.
    h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    h.run("op.abort", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(status(&h.git()), "");
    assert!(h.run("op.status", json!({}))["operation"].is_null());
}

#[test]
fn a_commit_to_be_edited_stops_the_plan_there() {
    let mut h = Harness::new();
    let ids = h.ids();
    let mut steps: Vec<Value> = ids.iter().map(|id| step(id, "pick")).collect();
    steps[1] = step(&ids[1], "edit");
    let out = h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": steps }),
    );
    let stopped = out["stopped"].as_str().unwrap();
    assert!(stopped.contains("to be amended"), "{stopped}");
    assert_eq!(out["conflicts"], json!([]));
    let operation = h.run("op.status", json!({}))["operation"].clone();
    assert_eq!(operation["kind"], "rebase");
    assert_eq!(operation["conflicts"], json!([]));
    assert_eq!(operation["editing"], true);
    // HEAD is the commit that is to be edited.
    assert_eq!(h.git().run(["log", "-1", "--format=%s"]).unwrap(), "two\n");

    let amend = |copy: &Scratch| {
        copy.write("extra.txt", "x\n").unwrap();
        let git = copy.git();
        git.run(["add", "extra.txt"]).unwrap();
        git.run(["commit", "-q", "--amend", "--no-edit"]).unwrap();
    };
    amend(&h.scratch);
    let out = h.run("op.continue", json!({}));
    assert_eq!(out["finished"], true, "{out}");

    let twin = h.twin.git();
    let t: Vec<String> = ["one", "two", "#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", t[1].as_str()]).unwrap();
    amend(&h.twin);
    twin.run(["cherry-pick", t[2].as_str(), t[3].as_str(), t[4].as_str()])
        .unwrap();
    h.assert_same("after the edit");
}

#[test]
fn one_commit_is_reworded_dropped_or_squashed() {
    let mut h = Harness::new();
    let twin = h.twin.git();

    // Reword in the middle.
    let three = id_of(&h.git(), "#3 three");
    h.run(
        "commit.reword",
        json!({ "commit": three, "message": "three, said differently\n\n#3" }),
    );
    let t: Vec<String> = ["#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", t[0].as_str()]).unwrap();
    twin.run([
        "commit",
        "-q",
        "--amend",
        "--cleanup=verbatim",
        "-m",
        "three, said differently\n\n#3\n",
    ])
    .unwrap();
    twin.run(["cherry-pick", t[1].as_str(), t[2].as_str()])
        .unwrap();
    h.assert_same("reword");
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        format!("Reword {}", &three[..7])
    );

    // Drop the last one: a plan that keeps nothing.
    let five = id_of(&h.git(), "five");
    h.run("commit.drop", json!({ "commit": five }));
    twin.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    h.assert_same("drop of the tip");

    // Drop one in the middle.
    let two = id_of(&h.git(), "two");
    h.run("commit.drop", json!({ "commit": two }));
    let t: Vec<String> = ["one", "three, said differently", "four"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["reset", "-q", "--hard", t[0].as_str()]).unwrap();
    twin.run(["cherry-pick", t[1].as_str(), t[2].as_str()])
        .unwrap();
    h.assert_same("drop in the middle");

    // One commit is squashed into the one before it.
    let said = id_of(&h.git(), "three, said differently");
    h.run("commit.squash", json!({ "commits": [said] }));
    twin.run(["reset", "-q", "--hard", t[0].as_str()]).unwrap();
    twin.run(["cherry-pick", "-n", t[1].as_str()]).unwrap();
    twin.run([
        "commit",
        "-q",
        "--amend",
        "--cleanup=verbatim",
        "-m",
        "one\n\nthree, said differently\n\n#3\n",
    ])
    .unwrap();
    twin.run(["cherry-pick", t[2].as_str()]).unwrap();
    h.assert_same("squash of one");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Squash 2 commits");

    // Several that follow one another, named in any order.
    let (git, start) = (h.git(), h.git().rev_parse("HEAD").unwrap());
    for (file, message) in [("p.txt", "p"), ("q.txt", "q"), ("r.txt", "r")] {
        for copy in [&h.scratch, &h.twin] {
            copy.write(file, "x\n").unwrap();
            copy.git().run(["add", file]).unwrap();
            copy.git().run(["commit", "-q", "-m", message]).unwrap();
        }
    }
    let (four, p, q) = (id_of(&git, "four"), id_of(&git, "p"), id_of(&git, "q"));
    // Not next to one another, one twice, one from nowhere: refused.
    for commits in [
        json!([four, q]),
        json!([p, p]),
        json!([p, q, p]),
        json!([p, "no-such"]),
        json!([]),
        json!("p"),
        json!([p, "--root"]),
    ] {
        let before = git.rev_parse("HEAD").unwrap();
        assert!(
            h.try_run("commit.squash", json!({ "commits": commits }))
                .is_err(),
            "{commits}"
        );
        assert_eq!(git.rev_parse("HEAD").unwrap(), before);
    }
    h.run("commit.squash", json!({ "commits": [q, four, p] }));
    twin.run(["reset", "-q", "--soft", "HEAD~3"]).unwrap();
    // `r` comes back on top of the three made one.
    twin.run(["reset", "-q", "HEAD", "--", "r.txt"]).unwrap();
    twin.run([
        "commit",
        "-q",
        "--amend",
        "--cleanup=verbatim",
        "-m",
        "four\n\np\n\nq\n",
    ])
    .unwrap();
    twin.run(["add", "r.txt"]).unwrap();
    twin.run(["commit", "-q", "-m", "r"]).unwrap();
    assert_eq!(
        h.git().lines(["log", "--format=%T %s", "-3"]).unwrap(),
        twin.lines(["log", "--format=%T %s", "-3"]).unwrap()
    );
    assert_eq!(
        h.git().run(["log", "-1", "--format=%B", "HEAD~1"]).unwrap(),
        twin.run(["log", "-1", "--format=%B", "HEAD~1"]).unwrap()
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Squash 3 commits");
    h.run("undo.last", json!({}));
    assert_ne!(h.git().rev_parse("HEAD").unwrap(), start);
    assert_eq!(h.git().run(["log", "-1", "--format=%s"]).unwrap(), "r\n");
}

#[test]
fn a_plan_matches_a_todo_list_written_by_hand() {
    let mut h = Harness::new();
    let ids = h.ids();
    h.run(
        "rebase.interactive",
        json!({ "first": ids[0], "steps": [
            step(&ids[2], "pick"),
            step(&ids[0], "pick"),
            step(&ids[1], "fixup"),
            step(&ids[4], "drop"),
            step(&ids[3], "edit"),
        ] }),
    );
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);

    // The same list, typed out, through git's own interactive rebase.
    let twin = h.twin.git();
    let t: Vec<String> = ["one", "two", "#3 three", "four", "five"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    let list = h.twin.path().join(".git").join("by-hand");
    std::fs::write(
        &list,
        format!(
            "pick {}\npick {}\nfixup {}\ndrop {}\nedit {}\n",
            t[2], t[0], t[1], t[4], t[3]
        ),
    )
    .unwrap();
    let editor = format!("cp '{}'", list.display());
    let env = [
        ("GIT_SEQUENCE_EDITOR", editor.as_str()),
        ("GIT_EDITOR", "true"),
    ];
    twin.run_with(["rebase", "-i", "HEAD~5"], &env).unwrap();
    twin.run_with(["rebase", "--continue"], &env).unwrap();
    h.assert_same("the hand-written list");
}

#[test]
fn commits_move_one_place_like_the_same_done_by_hand() {
    let mut h = Harness::new();
    let twin = h.twin.git();
    let t = |subject: &str| id_of(&twin, subject);

    // `two` up past `#3 three`.
    let two = id_of(&h.git(), "two");
    h.run("commit.move", json!({ "commits": [two], "up": true }));
    let (one, two_by_hand) = (t("one"), t("two"));
    let (three, four, five) = (t("#3 three"), t("four"), t("five"));
    twin.run(["reset", "-q", "--hard", one.as_str()]).unwrap();
    twin.run([
        "cherry-pick",
        three.as_str(),
        two_by_hand.as_str(),
        four.as_str(),
        five.as_str(),
    ])
    .unwrap();
    h.assert_same("one up");
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        format!("Move {} up", &two[..7])
    );

    // `four` and `five` (named the other way round) down past `two`.
    let (four, five) = (id_of(&h.git(), "four"), id_of(&h.git(), "five"));
    h.run(
        "commit.move",
        json!({ "commits": [five, four], "up": false }),
    );
    let (three, two, four, five) = (t("#3 three"), t("two"), t("four"), t("five"));
    twin.run(["reset", "-q", "--hard", three.as_str()]).unwrap();
    twin.run(["cherry-pick", four.as_str(), five.as_str(), two.as_str()])
        .unwrap();
    h.assert_same("two down");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Move 2 commits down");

    // Undone, both, and done again.
    let top = h.git().rev_parse("HEAD").unwrap();
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    assert_eq!(
        h.git().lines(["log", "--format=%s", "HEAD"]).unwrap()[..5],
        ["five", "four", "#3 three", "two", "one"]
    );
    h.run("redo.last", json!({}));
    h.run("redo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), top);

    // Nothing to move past, not one after another, the first commit of
    // the history, unknown, hostile: refused, nothing changes.
    let (newest, oldest_here) = (id_of(&h.git(), "two"), id_of(&h.git(), "#3 three"));
    let (one, four) = (id_of(&h.git(), "one"), id_of(&h.git(), "four"));
    let base = h.git().rev_parse("HEAD").unwrap();
    for params in [
        json!({ "commits": [newest], "up": true }),
        json!({ "commits": [one, four], "up": true }),
        json!({ "commits": [oldest_here, oldest_here], "up": true }),
        json!({ "commits": ["no-such"], "up": true }),
        json!({ "commits": ["--root"], "up": false }),
        json!({ "commits": [], "up": true }),
        json!({ "commits": four, "up": true }),
        json!({ "commits": [four] }),
        json!({ "commits": [four], "up": "yes" }),
    ] {
        assert!(
            h.try_run("commit.move", params.clone()).is_err(),
            "{params}"
        );
        assert_eq!(h.git().rev_parse("HEAD").unwrap(), base, "{params}");
    }
    // The first commit of the fixture cannot go down: there is nothing
    // under it that a plan can begin at.
    let root = h
        .git()
        .run(["rev-list", "--max-parents=0", "HEAD"])
        .unwrap();
    assert!(
        h.try_run(
            "commit.move",
            json!({ "commits": [root.trim()], "up": false })
        )
        .is_err()
    );
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), base);
    assert_eq!(status(&h.git()), "");
}
