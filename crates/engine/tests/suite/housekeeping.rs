//! Branch housekeeping against git: the overview is compared with what
//! `git branch --merged`, `git for-each-ref` and `git log` say about each
//! branch, descriptions and deletions with the same thing done by hand on
//! a twin copy.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir, oracle::CLOCK_EPOCH};
use serde_json::{Value, json};

const DAY: i64 = 86_400;
/// When `wip` got its commit: long after everything else in the fixture.
const WIP_TIME: i64 = CLOCK_EPOCH + 80 * DAY;

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Everything a housekeeping command can change, as text.
fn state(git: &Git) -> Vec<String> {
    let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
    let head = git
        .run(["symbolic-ref", "--quiet", "HEAD"])
        .or_else(|_| git.run(["rev-parse", "HEAD"]))
        .unwrap();
    out.push(format!("HEAD {}", head.trim()));
    // Sorted: a branch that is deleted and brought back has its settings
    // further down in the file than before.
    let mut config: Vec<String> = git
        .run(["config", "--local", "--get-regexp", r"^(branch|gitflow)\."])
        .unwrap_or_default()
        .lines()
        .map(|line| format!("config {line}"))
        .collect();
    config.sort();
    out.extend(config);
    out
}

/// A directory beside a scratch copy, removed with it.
fn beside(copy: &Scratch, extension: &str) -> PathBuf {
    let path = copy.path().with_extension(extension);
    let _ = std::fs::remove_dir_all(&path);
    path
}

fn at(path: &Path) -> Git {
    Git::new(path, fixtures_out_dir().join(".home"))
}

/// Branches of every kind there is to tell apart, on top of the fixture's
/// `main` with `feature/a` and `feature/b` merged into it:
/// merged ones (`done/one`, `done/two` which follows main, `shielded`,
/// `held` which a second worktree has checked out, `here` which is the
/// current one), unmerged ones (`wip`, with a late commit), and two that
/// follow a branch that is gone, one merged and one not.
fn untidy(copy: &Scratch) {
    let git = copy.git();
    let late = format!("{WIP_TIME} +0000");
    let dated = [
        ("GIT_AUTHOR_DATE", late.as_str()),
        ("GIT_COMMITTER_DATE", late.as_str()),
    ];
    git.run(["branch", "done/one", "main~1"]).unwrap();
    git.run(["branch", "done/two", "main"]).unwrap();
    git.run(["branch", "-q", "-u", "main", "done/two"]).unwrap();
    git.run(["branch", "shielded", "main~2"]).unwrap();
    git.run(["branch", "held", "main~1"]).unwrap();
    git.run(["branch", "vanishing", "main"]).unwrap();
    git.run(["branch", "gone/merged", "main~1"]).unwrap();
    git.run(["branch", "-q", "-u", "vanishing", "gone/merged"])
        .unwrap();
    git.run(["switch", "-q", "-c", "wip", "main"]).unwrap();
    copy.write("wip.txt", "wip\n").unwrap();
    git.run(["add", "wip.txt"]).unwrap();
    git.run_with(["commit", "-q", "-m", "wip"], &dated).unwrap();
    git.run(["switch", "-q", "-c", "gone/unmerged", "main~1"])
        .unwrap();
    copy.write("other.txt", "other\n").unwrap();
    git.run(["add", "other.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "not merged anywhere"])
        .unwrap();
    git.run(["branch", "-q", "-u", "vanishing", "gone/unmerged"])
        .unwrap();
    git.run(["branch", "-q", "-D", "vanishing"]).unwrap();
    git.run(["switch", "-q", "-c", "here", "main"]).unwrap();
    let other = beside(copy, "wt");
    git.run(["worktree", "add", "-q", other.to_str().unwrap(), "held"])
        .unwrap();
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("branches-merges").unwrap();
        let twin = Scratch::of("branches-merges").unwrap();
        untidy(&scratch);
        untidy(&twin);
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
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.git()), state(&self.twin.git()), "{what}");
    }
    fn peek(&mut self) -> Value {
        self.run("undo.peek", json!({}))["undo"].clone()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        for copy in [&self.scratch, &self.twin] {
            for extension in ["wt", "wt2", "origin.git"] {
                let _ = std::fs::remove_dir_all(copy.path().with_extension(extension));
            }
        }
    }
}

/// The local branches by their full names, short.
fn branches(git: &Git) -> Vec<String> {
    git.lines(["for-each-ref", "--format=%(refname)", "refs/heads"])
        .unwrap()
        .into_iter()
        .map(|name| name.strip_prefix("refs/heads/").unwrap().to_owned())
        .collect()
}

/// What git calls merged into `base`.
fn merged(git: &Git, base: &str) -> BTreeSet<String> {
    git.lines(["branch", "--merged", base, "--format=%(refname)"])
        .unwrap()
        .into_iter()
        .map(|name| name.strip_prefix("refs/heads/").unwrap().to_owned())
        .collect()
}

/// One answer of the overview, held against git one branch at a time.
fn check_overview(git: &Git, out: &Value, base: Option<&str>, days: i64, now: i64) {
    let merged = base.map(|base| merged(git, base)).unwrap_or_default();
    let tracking: BTreeMap<String, (String, String)> = git
        .lines([
            "for-each-ref",
            "--format=%(refname)|%(upstream:short)|%(upstream:track)",
            "refs/heads",
        ])
        .unwrap()
        .into_iter()
        .map(|line| {
            let mut parts = line.splitn(3, '|');
            let name = parts.next().unwrap().strip_prefix("refs/heads/").unwrap();
            (
                name.to_owned(),
                (
                    parts.next().unwrap().to_owned(),
                    parts.next().unwrap().to_owned(),
                ),
            )
        })
        .collect();
    let current = git
        .run(["branch", "--show-current"])
        .unwrap()
        .trim()
        .to_owned();
    // Where the other worktrees are, and on which branch.
    let listed = git.run(["worktree", "list", "--porcelain"]).unwrap();
    let mut elsewhere: BTreeMap<String, String> = BTreeMap::new();
    let mut path = String::new();
    for line in listed.lines() {
        if let Some(found) = line.strip_prefix("worktree ") {
            path = found.to_owned();
        } else if let Some(branch) = line.strip_prefix("branch refs/heads/")
            && branch != current
        {
            elsewhere.insert(branch.to_owned(), path.clone());
        }
    }

    assert_eq!(out["base"], json!(base));
    assert_eq!(out["stale_days"], days);
    assert_eq!(out["now"], now);
    let found = out["branches"].as_array().unwrap();
    let names: Vec<&str> = found.iter().map(|b| b["name"].as_str().unwrap()).collect();
    assert_eq!(names, branches(git));
    for branch in found {
        let name = branch["name"].as_str().unwrap();
        let reference = format!("refs/heads/{name}");
        let time: i64 = git
            .run(["log", "-1", "--format=%ct", &reference])
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        let (upstream, track) = &tracking[name];
        let description = git
            .run(["config", "--get", &format!("branch.{name}.description")])
            .ok()
            .map(|text| text.strip_suffix('\n').unwrap_or(&text).to_owned());
        assert_eq!(branch["tip"], git.rev_parse(&reference).unwrap(), "{name}");
        assert_eq!(branch["time"], time, "{name}: time");
        assert_eq!(branch["stale"], time < now - days * DAY, "{name}: stale");
        assert_eq!(branch["merged"], merged.contains(name), "{name}: merged");
        assert_eq!(branch["gone"], track == "[gone]", "{name}: gone");
        assert_eq!(
            branch["upstream"],
            json!((!upstream.is_empty()).then_some(upstream)),
            "{name}: upstream"
        );
        assert_eq!(branch["current"], name == current, "{name}: current");
        assert_eq!(
            branch["worktree"],
            json!(elsewhere.get(name)),
            "{name}: worktree"
        );
        assert_eq!(branch["description"], json!(description), "{name}");
    }
}

fn flagged<'a>(out: &'a Value, flag: &str) -> Vec<&'a str> {
    out["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b[flag] == true)
        .map(|b| b["name"].as_str().unwrap())
        .collect()
}

#[test]
fn overview_says_of_each_branch_what_git_says() {
    let mut h = Harness::new();
    let git = h.git();
    git.run([
        "config",
        "branch.feature/a.description",
        "First of the two\n\nand a second paragraph",
    ])
    .unwrap();
    let before = state(&git);
    let now = CLOCK_EPOCH + 100 * DAY;

    // Compared with main, a hundred days after the fixture was made:
    // everything is stale but the branch with the late commit.
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, Some("main"), 90, now);
    assert_eq!(
        flagged(&out, "merged"),
        [
            "done/one",
            "done/two",
            "feature/a",
            "feature/b",
            "gone/merged",
            "held",
            "here",
            "main",
            "shielded"
        ]
    );
    assert_eq!(flagged(&out, "gone"), ["gone/merged", "gone/unmerged"]);
    assert_eq!(flagged(&out, "current"), ["here"]);
    let fresh: Vec<&str> = out["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["stale"] == false)
        .map(|b| b["name"].as_str().unwrap())
        .collect();
    assert_eq!(fresh, ["wip"]);
    let held = out["branches"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "held")
        .unwrap();
    assert!(held["worktree"].as_str().unwrap().ends_with(".wt"));

    // Other spans of time. Exactly that long ago is not yet stale.
    for (days, stale) in [(10, 11), (1000, 0), (0, 11)] {
        let out = h.run("branch.overview", json!({ "now": now, "stale_days": days }));
        check_overview(&git, &out, Some("main"), days, now);
        assert_eq!(flagged(&out, "stale").len(), stale, "{days} days");
    }
    let edge = WIP_TIME + 90 * DAY;
    let out = h.run("branch.overview", json!({ "now": edge }));
    assert!(!flagged(&out, "stale").contains(&"wip"));
    let out = h.run("branch.overview", json!({ "now": edge + 1 }));
    assert!(flagged(&out, "stale").contains(&"wip"));
    // By the clock, the fixture is years old.
    let out = h.run("branch.overview", json!({}));
    let clock = out["now"].as_i64().unwrap();
    assert!(clock > 1_750_000_000, "{clock}");
    check_overview(&git, &out, Some("main"), 90, clock);
    assert_eq!(flagged(&out, "stale").len(), 11);

    // Compared with something else: a branch, and gitflow's main branch
    // when there is one.
    for base in ["feature/a", "wip", "done/one"] {
        let out = h.run("branch.overview", json!({ "base": base, "now": now }));
        check_overview(&git, &out, Some(base), 90, now);
    }
    git.run(["config", "gitflow.branch.master", "feature/b"])
        .unwrap();
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, Some("feature/b"), 90, now);
    // One that names no branch is not taken up.
    git.run(["config", "gitflow.branch.master", "nowhere"])
        .unwrap();
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, Some("main"), 90, now);
    git.run(["config", "--unset", "gitflow.branch.master"])
        .unwrap();
    assert_eq!(state(&git), before, "asking changes nothing");

    // A branch of a remote. Main has moved on since it was fetched, so
    // less is merged there than into main.
    let bare = beside(&h.scratch, "origin.git");
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run(["fetch", "-q", "origin"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("more.txt", "more\n").unwrap();
    git.run(["add", "more.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "main moves on"]).unwrap();
    git.run(["branch", "-q", "-f", "done/two", "main"]).unwrap();
    let out = h.run(
        "branch.overview",
        json!({ "base": "origin/main", "now": now }),
    );
    check_overview(&git, &out, Some("origin/main"), 90, now);
    assert!(!flagged(&out, "merged").contains(&"done/two"));
    assert!(flagged(&out, "merged").contains(&"here"));

    // Neither main nor master: the current branch. Nowhere: nothing to
    // compare with, and nothing counts as merged.
    git.run(["branch", "-m", "main", "trunk"]).unwrap();
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, Some("trunk"), 90, now);
    git.run(["branch", "master", "trunk~3"]).unwrap();
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, Some("master"), 90, now);
    git.run(["branch", "-D", "master"]).unwrap();
    git.run(["switch", "-q", "--detach"]).unwrap();
    let out = h.run("branch.overview", json!({ "now": now }));
    check_overview(&git, &out, None, 90, now);
    assert!(flagged(&out, "merged").is_empty());
    assert!(flagged(&out, "current").is_empty());
}

#[test]
fn describe_sets_a_description_as_it_is_given_and_removes_it() {
    let mut h = Harness::new();
    let key = "branch.feature/a.description";
    let exact = |git: &Git| git.run(["config", "-z", "--get", key]).unwrap();
    let text = "Rework of the login form\n\n  Second paragraph, with a\ttab.\n";

    let out = h.run(
        "branch.describe",
        json!({ "name": "feature/a", "description": text }),
    );
    assert_eq!(out["branch"], "feature/a");
    assert_eq!(out["description"], text);
    assert!(out["before"].is_null());
    h.twin.git().run(["config", key, text]).unwrap();
    h.assert_same("describe");
    assert_eq!(exact(&h.git()), format!("{text}\0"), "kept as it is");
    let overview = h.run("branch.overview", json!({}));
    let described: Vec<&Value> = overview["branches"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| !b["description"].is_null())
        .collect();
    assert_eq!(described.len(), 1);
    assert_eq!(described[0]["name"], "feature/a");
    assert_eq!(described[0]["description"], text);

    // Another text takes its place, whatever it begins with.
    for next in [
        "-not an option",
        "  indented, and \"quoted\" # not a comment",
    ] {
        let was = exact(&h.git());
        let out = h.run(
            "branch.describe",
            json!({ "name": "feature/a", "description": next }),
        );
        assert_eq!(out["description"], next);
        assert_eq!(format!("{}\0", out["before"].as_str().unwrap()), was);
        h.twin.git().run(["config", "--", key, next]).unwrap();
        h.assert_same("describe again");
        assert_eq!(exact(&h.git()), format!("{next}\0"));
    }

    // Empty, blank or absent: the description goes.
    for (n, params) in [
        json!({ "name": "feature/a", "description": "" }),
        json!({ "name": "feature/a", "description": " \n\t" }),
        json!({ "name": "feature/a" }),
        json!({ "name": "feature/a", "description": null }),
    ]
    .into_iter()
    .enumerate()
    {
        h.run(
            "branch.describe",
            json!({ "name": "feature/a", "description": "to go" }),
        );
        let out = h.run("branch.describe", params);
        assert!(out["description"].is_null(), "{n}");
        assert_eq!(out["before"], "to go", "{n}");
        assert!(h.git().run(["config", "--get", key]).is_err(), "{n}");
    }
    h.twin.git().run(["config", "--unset", key]).unwrap();
    h.assert_same("description removed");
    // Nothing there: nothing to remove.
    let out = h.run(
        "branch.describe",
        json!({ "name": "feature/a", "description": "" }),
    );
    assert!(out["before"].is_null() && out["description"].is_null());

    // What else a branch has configured stays through all of it.
    let follows = |git: &Git| {
        git.run(["config", "--get", "branch.done/two.merge"])
            .unwrap()
    };
    h.run(
        "branch.describe",
        json!({ "name": "done/two", "description": "follows main" }),
    );
    h.run("branch.describe", json!({ "name": "done/two" }));
    assert_eq!(follows(&h.git()), "refs/heads/main\n");
    h.assert_same("the upstream stays");

    // Only a branch that is there can be described.
    let before = state(&h.git());
    for name in ["nowhere", "origin/main", "v1.0", "refs/heads/main"] {
        let err = h
            .try_run(
                "branch.describe",
                json!({ "name": name, "description": "x" }),
            )
            .unwrap_err();
        assert!(err.contains("no branch"), "{name}: {err}");
    }
    assert_eq!(state(&h.git()), before);

    // A description is journalled like any change: undone, it
    // is what it was; redone, what the command made it. One that changed
    // nothing left no entry.
    assert_eq!(h.peek(), "Remove the description of branch done/two");
    h.run("undo.last", json!({}));
    assert_eq!(
        description(&h.git(), "done/two").as_deref(),
        Some("follows main")
    );
    assert_eq!(h.peek(), "Describe branch done/two");
    h.run("undo.last", json!({}));
    assert_eq!(description(&h.git(), "done/two"), None);
    assert_eq!(follows(&h.git()), "refs/heads/main\n", "the upstream stays");
    h.run("redo.last", json!({}));
    assert_eq!(
        description(&h.git(), "done/two").as_deref(),
        Some("follows main")
    );
    // Changed by hand since: undo leaves it alone and says so.
    h.git()
        .run(["config", "branch.done/two.description", "written by hand"])
        .unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("not what this action left it as"), "{err}");
    assert_eq!(
        description(&h.git(), "done/two").as_deref(),
        Some("written by hand")
    );
    // A text of several lines comes back as it was, to the byte.
    let long = "first line\n\n  indented, with \"quotes\" and a # sign\nlast\n";
    h.git()
        .run(["config", "branch.done/two.description", long])
        .unwrap();
    h.run(
        "branch.describe",
        json!({ "name": "done/two", "description": "short" }),
    );
    h.run("undo.last", json!({}));
    assert_eq!(description(&h.git(), "done/two").as_deref(), Some(long));
}

/// The description of a branch as git has it.
fn description(git: &Git, branch: &str) -> Option<String> {
    git.run([
        "config",
        "--local",
        "--get",
        &format!("branch.{branch}.description"),
    ])
    .ok()
    .map(|text| text.strip_suffix('\n').unwrap_or(&text).to_owned())
}

/// Names out of a list of branches in an answer.
fn names(list: &Value) -> Vec<String> {
    list.as_array()
        .unwrap()
        .iter()
        .map(|b| b["name"].as_str().unwrap().to_owned())
        .collect()
}

fn reasons(out: &Value) -> BTreeMap<String, String> {
    out["kept"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| {
            (
                b["name"].as_str().unwrap().to_owned(),
                b["reason"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn delete_merged_takes_what_git_calls_merged_minus_the_exceptions() {
    let mut h = Harness::new();
    let before = state(&h.git());
    let protected = json!(["shielded", "not-a-branch-here"]);
    // What git says is merged, without the base, the current branch, the
    // one another worktree holds and the protected one.
    let expected: Vec<String> = merged(&h.twin.git(), "main")
        .into_iter()
        .filter(|name| !["main", "here", "held", "shielded"].contains(&name.as_str()))
        .collect();
    assert_eq!(
        expected,
        [
            "done/one",
            "done/two",
            "feature/a",
            "feature/b",
            "gone/merged"
        ]
    );
    let kept: BTreeMap<String, String> = [
        ("main", "base"),
        ("here", "current"),
        ("held", "worktree"),
        ("shielded", "protected"),
        ("wip", "unmerged"),
        ("gone/unmerged", "unmerged"),
    ]
    .into_iter()
    .map(|(name, reason)| (name.to_owned(), reason.to_owned()))
    .collect();

    // First only asked: the answer, and nothing done.
    let asked = h.run(
        "branch.delete_merged",
        json!({ "protected": protected, "dry_run": true }),
    );
    assert_eq!(asked["base"], "main");
    assert_eq!(asked["dry_run"], true);
    assert_eq!(names(&asked["deleted"]), expected);
    assert_eq!(reasons(&asked), kept);
    assert_eq!(state(&h.git()), before);
    assert!(h.peek().is_null());
    for branch in asked["deleted"].as_array().unwrap() {
        let name = branch["name"].as_str().unwrap();
        assert_eq!(branch["target"], h.git().rev_parse(name).unwrap());
    }
    let of = |list: &Value, name: &str| -> Value {
        list.as_array()
            .unwrap()
            .iter()
            .find(|b| b["name"] == name)
            .unwrap()
            .clone()
    };
    assert_eq!(of(&asked["deleted"], "done/two")["upstream"], "main");
    assert_eq!(of(&asked["deleted"], "gone/merged")["gone"], true);
    assert_eq!(of(&asked["deleted"], "done/one")["gone"], false);
    // Its upstream is gone, but it is not merged: it is not touched.
    assert_eq!(of(&asked["kept"], "gone/unmerged")["gone"], true);
    assert!(
        of(&asked["kept"], "held")["detail"]
            .as_str()
            .unwrap()
            .ends_with(".wt")
    );
    assert!(of(&asked["kept"], "main")["detail"].is_null());

    // Then done.
    let done = h.run("branch.delete_merged", json!({ "protected": protected }));
    assert_eq!(done["dry_run"], false);
    assert_eq!(done["deleted"], asked["deleted"]);
    assert_eq!(done["kept"], asked["kept"]);
    for name in &expected {
        h.twin.git().run(["branch", "-q", "-D", name]).unwrap();
    }
    h.assert_same("delete merged");
    assert_eq!(
        branches(&h.git()),
        ["gone/unmerged", "held", "here", "main", "shielded", "wip"]
    );
    let deleted = state(&h.git());

    // Nothing merged is left: again does nothing.
    let again = h.run("branch.delete_merged", json!({ "protected": protected }));
    assert_eq!(again["deleted"], json!([]));
    assert_eq!(state(&h.git()), deleted);

    // Each deletion is a step of the journal: they come back one by one,
    // the last deleted first, with the upstream when it is still there.
    for name in expected.iter().rev() {
        assert_eq!(h.peek(), format!("Delete branch {name}"));
        assert!(h.git().rev_parse(&format!("refs/heads/{name}")).is_err());
        h.run("undo.last", json!({}));
        assert_eq!(
            h.git().rev_parse(name).unwrap(),
            of(&asked["deleted"], name)["target"]
        );
    }
    assert!(h.peek().is_null());
    // An upstream that is gone cannot be set again, so that one setting
    // is what a deletion and its undo cost.
    let without = |state: Vec<String>| -> Vec<String> {
        state
            .into_iter()
            .filter(|line| !line.starts_with("config branch.gone/merged."))
            .collect()
    };
    assert_eq!(without(state(&h.git())), without(before));
    for _ in &expected {
        h.run("redo.last", json!({}));
    }
    assert_eq!(state(&h.git()), deleted);
}

#[test]
fn delete_merged_keeps_what_is_not_its_to_take() {
    let mut h = Harness::new();
    let git = h.git();

    // Gitflow's two branches stay, merged or not, asked for or not.
    git.run(["branch", "develop", "main~1"]).unwrap();
    git.run(["config", "gitflow.branch.master", "main"])
        .unwrap();
    git.run(["config", "gitflow.branch.develop", "develop"])
        .unwrap();
    let out = h.run("branch.delete_merged", json!({ "dry_run": true }));
    assert_eq!(reasons(&out)["develop"], "flow");
    assert_eq!(reasons(&out)["main"], "base");
    assert!(names(&out["deleted"]).contains(&"shielded".to_owned()));
    let out = h.run(
        "branch.delete_merged",
        json!({ "base": "done/two", "dry_run": true }),
    );
    assert_eq!(reasons(&out)["main"], "flow");
    assert_eq!(reasons(&out)["done/two"], "base");
    git.run(["config", "--remove-section", "gitflow.branch"])
        .unwrap();
    git.run(["branch", "-D", "develop"]).unwrap();

    // Compared with another branch, it is that one's question.
    let out = h.run(
        "branch.delete_merged",
        json!({ "base": "feature/a", "dry_run": true }),
    );
    let expected: Vec<String> = merged(&git, "feature/a")
        .into_iter()
        .filter(|name| name != "feature/a")
        .collect();
    assert_eq!(names(&out["deleted"]), expected);
    assert_eq!(reasons(&out)["main"], "unmerged");

    // Compared with a branch of a remote, the local branch of that name
    // is the base as far as deleting goes.
    let bare = beside(&h.scratch, "origin.git");
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run(["fetch", "-q", "origin"]).unwrap();
    git.run(["branch", "-q", "-u", "origin/main", "done/one"])
        .unwrap();
    git.run(["branch", "tracker", "origin/main"]).unwrap();
    git.run(["branch", "-q", "-u", "origin/main", "tracker"])
        .unwrap();
    let out = h.run(
        "branch.delete_merged",
        json!({ "base": "origin/main", "dry_run": true }),
    );
    assert_eq!(out["base"], "origin/main");
    assert_eq!(reasons(&out)["main"], "base");
    assert_eq!(reasons(&out)["tracker"], "base", "it follows the base");
    assert_eq!(reasons(&out)["done/one"], "base", "and so does this one");
    let expected: Vec<String> = merged(&git, "origin/main")
        .into_iter()
        .filter(|name| !["main", "tracker", "done/one", "here", "held"].contains(&name.as_str()))
        .collect();
    assert_eq!(names(&out["deleted"]), expected);

    // Nothing to compare with: it has to be said.
    let before = state(&git);
    git.run(["branch", "-m", "main", "trunk"]).unwrap();
    git.run(["switch", "-q", "--detach"]).unwrap();
    let err = h.try_run("branch.delete_merged", json!({})).unwrap_err();
    assert!(err.contains("say which"), "{err}");
    assert!(h.peek().is_null());
    git.run(["switch", "-q", "here"]).unwrap();
    git.run(["branch", "-m", "trunk", "main"]).unwrap();
    assert_eq!(state(&git), before);
}

#[test]
fn a_branch_git_will_not_let_go_is_kept_and_the_others_go() {
    let mut h = Harness::new();
    let git = h.git();
    // A second worktree bisects on `busy`: HEAD is detached there, so no
    // listing shows the branch as checked out, and git still holds it.
    git.run(["branch", "busy", "main~1"]).unwrap();
    let other = beside(&h.scratch, "wt2");
    git.run(["worktree", "add", "-q", other.to_str().unwrap(), "busy"])
        .unwrap();
    let there = at(&other);
    let root = git
        .run(["rev-list", "--max-parents=0", "main"])
        .unwrap()
        .trim()
        .to_owned();
    there.run(["bisect", "start", "busy", &root]).unwrap();
    assert!(there.run(["symbolic-ref", "-q", "HEAD"]).is_err());

    let out = h.run("branch.delete_merged", json!({}));
    let kept = reasons(&out);
    assert_eq!(kept["busy"], "failed", "{out}");
    let busy = out["kept"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["name"] == "busy")
        .unwrap();
    assert!(busy["detail"].as_str().unwrap().contains("busy"), "{busy}");
    assert_eq!(
        names(&out["deleted"]),
        [
            "done/one",
            "done/two",
            "feature/a",
            "feature/b",
            "gone/merged",
            "shielded"
        ]
    );
    assert!(git.rev_parse("refs/heads/busy").is_ok());
    assert_eq!(h.peek(), "Delete branch shielded");
    assert_eq!(
        branches(&git),
        ["busy", "gone/unmerged", "held", "here", "main", "wip"]
    );
}

#[test]
fn hostile_housekeeping_parameters_and_repositories_are_survived() {
    let mut h = Harness::new();
    let git = h.git();
    let before = state(&git);
    let long = "x".repeat(5000);
    let huge = "d".repeat(100_000);

    for name in [
        "",
        "-D",
        "--force",
        "-",
        "a..b",
        "a b",
        "a\nb",
        "a\u{0}b",
        "\u{1b}[2J",
        "end.lock",
        "refs/heads/../escape",
        "/leading",
        "trailing/",
        "@{-1}",
        "@{upstream}",
        "main@{1}",
        "HEAD",
        "@",
        "main~1",
        "main^",
        "main^{tree}",
        "a:b",
        "a?b",
        "*",
        "a[b",
        "a\\b",
        "nowhere",
        long.as_str(),
    ] {
        for (command, params) in [
            (
                "branch.describe",
                json!({ "name": name, "description": "text" }),
            ),
            ("branch.describe", json!({ "name": name })),
            ("branch.overview", json!({ "base": name })),
            ("branch.delete_merged", json!({ "base": name })),
            (
                "branch.delete_merged",
                json!({ "base": name, "dry_run": true }),
            ),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted {name:?}"
            );
        }
        assert_eq!(state(&git), before, "after {name:?}");
    }
    for description in [
        "nul\u{0}",
        "\u{1b}]0;title\u{7}",
        "bell\u{7}",
        huge.as_str(),
    ] {
        assert!(
            h.try_run(
                "branch.describe",
                json!({ "name": "feature/a", "description": description })
            )
            .is_err(),
            "{:?}",
            description.get(..10)
        );
    }
    assert_eq!(state(&git), before);
    for params in [
        json!({ "stale_days": -1 }),
        json!({ "stale_days": 1.5 }),
        json!({ "stale_days": "90" }),
        json!({ "stale_days": 4_294_967_296_u64 }),
        json!({ "now": "today" }),
        json!({ "now": 1e300 }),
        json!({ "base": 3 }),
        json!({ "base": ["main"] }),
    ] {
        assert!(
            h.try_run("branch.overview", params.clone()).is_err(),
            "{params}"
        );
    }
    // Numbers at the ends of their range are counted with, not crashed on.
    for params in [
        json!({ "stale_days": 4_294_967_295_u32, "now": i64::MIN }),
        json!({ "stale_days": 4_294_967_295_u32, "now": i64::MAX }),
        json!({ "stale_days": 0, "now": i64::MIN }),
        json!({ "stale_days": 0, "now": 0 }),
    ] {
        let out = h.run("branch.overview", params.clone());
        assert_eq!(out["branches"].as_array().unwrap().len(), 11, "{params}");
    }
    for params in [
        json!({ "protected": "main" }),
        json!({ "protected": [1, 2] }),
        json!({ "dry_run": "yes" }),
        json!({ "base": {} }),
    ] {
        assert!(
            h.try_run("branch.delete_merged", params.clone()).is_err(),
            "{params}"
        );
    }
    for params in [
        json!({}),
        json!({ "name": 3 }),
        json!({ "name": ["feature/a"] }),
        json!({ "name": "feature/a", "description": 7 }),
        json!({ "name": "feature/a", "description": ["x"] }),
    ] {
        assert!(
            h.try_run("branch.describe", params.clone()).is_err(),
            "{params}"
        );
    }
    // Protected names are only ever compared with: anything may stand
    // there, and what it names stays.
    let everything: Vec<String> = branches(&git);
    let mut protected: Vec<Value> = everything.iter().map(|name| json!(name)).collect();
    protected.extend([json!("-D"), json!("a\nb"), json!(""), json!(long)]);
    let out = h.run("branch.delete_merged", json!({ "protected": protected }));
    assert_eq!(out["deleted"], json!([]));
    assert_eq!(state(&git), before);
    assert!(h.peek().is_null());

    // A repository that is not as expected. Looking changes nothing, and
    // with every branch protected nothing is deleted either.
    let every = |h: &mut Harness, what: &str| {
        let git = h.git();
        let refs = |git: &Git| git.user_refs("%(refname) %(objectname)").unwrap();
        let before = refs(&git);
        let config = git.run(["config", "--local", "--list"]).unwrap();
        let _ = h.try_run("branch.overview", json!({}));
        let _ = h.try_run("branch.overview", json!({ "base": "feature/a" }));
        let _ = h.try_run("branch.delete_merged", json!({ "dry_run": true }));
        let _ = h.try_run(
            "branch.delete_merged",
            json!({ "protected": protected, "base": "main" }),
        );
        assert_eq!(refs(&git), before, "{what}");
        assert_eq!(
            git.run(["config", "--local", "--list"]).unwrap(),
            config,
            "{what}"
        );
        // Describing writes; the references stay as they are, and undo
        // takes the description back however odd the rest is.
        let _ = h.try_run(
            "branch.describe",
            json!({ "name": "feature/a", "description": "x" }),
        );
        let _ = h.try_run("branch.describe", json!({ "name": "feature/a" }));
        assert_eq!(refs(&git), before, "{what}: describe");
        for _ in 0..2 {
            if !h.peek().is_null() {
                h.run("undo.last", json!({}));
            }
        }
        assert!(h.peek().is_null(), "{what}");
        // The same settings with the same values, each name's in the
        // order they were in; where in the file a name stands is not
        // kept (git writes a value at the end of its section).
        let by_name = |listed: &str| -> BTreeMap<String, Vec<String>> {
            let mut by_name: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for line in listed.lines() {
                let (name, value) = line.split_once('=').unwrap_or((line, "true"));
                by_name
                    .entry(name.to_owned())
                    .or_default()
                    .push(value.to_owned());
            }
            by_name
        };
        assert_eq!(
            by_name(&git.run(["config", "--local", "--list"]).unwrap()),
            by_name(&config),
            "{what}: describe undone"
        );
    };
    for (key, value) in [
        ("branch.feature/a.merge", "garbage"),
        ("branch.feature/a.merge", "refs/heads/../../etc"),
        ("branch.feature/a.remote", "nowhere"),
        ("branch.feature/a.remote", "-x"),
        (
            "branch.nowhere.description",
            "of a branch that is not there",
        ),
        ("branch.-x.description", "of a name no branch can have"),
        ("gitflow.branch.master", "-x"),
        ("gitflow.branch.master", "main~1"),
        ("gitflow.branch.develop", "a\nb"),
    ] {
        git.run(["config", "--local", "--add", key, value]).unwrap();
        every(&mut h, &format!("{key} = {value:?}"));
        git.run(["config", "--local", "--unset-all", key]).unwrap();
    }
    // A description set twice, and one without a value.
    for value in ["one", "two"] {
        git.run([
            "config",
            "--local",
            "--add",
            "branch.feature/a.description",
            value,
        ])
        .unwrap();
    }
    assert_eq!(
        h.run("branch.overview", json!({}))["branches"][2]["description"],
        "two",
        "the last one is the one git reads"
    );
    every(&mut h, "a description set twice");
    let config = h.scratch.path().join(".git/config");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        format!("{text}[branch \"feature/b\"]\n\tdescription\n"),
    )
    .unwrap();
    every(&mut h, "a description without a value");
    std::fs::write(&config, &text).unwrap();

    let dot = h.scratch.path().join(".git");
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    for (file, content) in [
        ("MERGE_HEAD", String::new()),
        ("MERGE_HEAD", "not an id\n".to_owned()),
        ("refs/heads/broken", "not an id\n".to_owned()),
        ("refs/heads/empty", String::new()),
        ("refs/heads/a-tree", format!("{tree}\n")),
        ("refs/remotes/origin/broken", "garbage\n".to_owned()),
        ("worktrees/ghost/gitdir", "\u{0}garbage".to_owned()),
        (
            "worktrees/ghost/HEAD",
            "ref: refs/heads/feature/a\n".to_owned(),
        ),
        ("bitbraid/journal.json", "{".to_owned()),
    ] {
        let path = dot.join(file);
        let had = std::fs::read(&path).ok();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, content).unwrap();
        every(&mut h, file);
        match had {
            Some(bytes) => std::fs::write(&path, bytes).unwrap(),
            None => std::fs::remove_file(&path).unwrap(),
        }
    }
    let _ = std::fs::remove_dir_all(dot.join("worktrees/ghost"));

    // And one git itself no longer reads: errors, nothing more.
    std::fs::write(dot.join("HEAD"), "garbage\n").unwrap();
    for (command, params) in [
        ("branch.overview", json!({})),
        (
            "branch.describe",
            json!({ "name": "main", "description": "x" }),
        ),
        ("branch.delete_merged", json!({})),
    ] {
        let _ = h.try_run(command, params);
    }
}
