//! Gitflow commands against the same steps done by hand with plain git on
//! a twin copy: git-flow itself is not needed and not used. Commits made
//! on one side carry today's date and on the other the fixture's, so where
//! ids cannot match, trees, parents and messages do.

use std::collections::HashMap;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// Everything gitflow can change, without the ids of commits made on the
/// way: per branch its first-parent history as (tree, trees of the
/// parents, subject), the tags with what they mark, HEAD, the settings,
/// and the status.
fn state(git: &Git) -> Vec<String> {
    let trees: HashMap<String, String> = git
        .lines(["log", "--all", "--format=%H %T"])
        .unwrap()
        .into_iter()
        .filter_map(|line| {
            line.split_once(' ')
                .map(|(id, tree)| (id.to_owned(), tree.to_owned()))
        })
        .collect();
    let mut out = Vec::new();
    // By their full names: a short one is not the branch's own when a tag
    // shares it.
    for reference in git
        .lines(["for-each-ref", "--format=%(refname)", "refs/heads"])
        .unwrap()
    {
        let branch = reference.strip_prefix("refs/heads/").unwrap();
        for line in git
            .lines(["log", "--first-parent", "--format=%T|%P|%s", &reference])
            .unwrap()
        {
            let mut parts = line.splitn(3, '|');
            let tree = parts.next().unwrap();
            let parents: Vec<&str> = parts
                .next()
                .unwrap()
                .split_whitespace()
                .map(|id| trees[id].as_str())
                .collect();
            let subject = parts.next().unwrap();
            out.push(format!("{branch}: {tree} {parents:?} {subject}"));
        }
    }
    for line in git
        .lines([
            "for-each-ref",
            "--format=%(refname)|%(objecttype)|%(tree)%(*tree)|%(contents:subject)",
            "refs/tags",
        ])
        .unwrap()
    {
        out.push(format!("tag {line}"));
    }
    let head = git
        .run(["symbolic-ref", "--quiet", "HEAD"])
        .or_else(|_| git.run(["rev-parse", "HEAD^{tree}"]))
        .unwrap();
    out.push(format!("HEAD {}", head.trim()));
    let mut config: Vec<String> = git
        .run(["config", "--local", "--get-regexp", r"^(gitflow|branch)\."])
        .unwrap_or_default()
        .lines()
        .map(|line| format!("config {line}"))
        .collect();
    config.sort();
    out.extend(config);
    out.push(format!(
        "status {:?}",
        git.run(["status", "--porcelain"]).unwrap()
    ));
    out
}

/// A commit of one file, the same on whichever copy it is made.
fn commit(copy: &Scratch, file: &str, content: &str, message: &str) {
    copy.write(file, content).unwrap();
    let git = copy.git();
    git.run(["add", "--", file]).unwrap();
    git.run(["commit", "-q", "-m", message]).unwrap();
}

/// What `flow.init` does, by hand, with the usual prefixes for bugfix and
/// support branches.
fn init_by_hand(copy: &Scratch, main: &str, develop: &str, prefixes: [&str; 4]) {
    let [feature, release, hotfix, tag] = prefixes;
    init_all_by_hand(
        copy,
        main,
        develop,
        [feature, "bugfix/", release, hotfix, "support/", tag],
    );
}

/// What `flow.init` does, by hand: the prefixes are those of feature,
/// bugfix, release, hotfix and support branches, and of the tags.
fn init_all_by_hand(copy: &Scratch, main: &str, develop: &str, prefixes: [&str; 6]) {
    let git = copy.git();
    if git.rev_parse(&format!("refs/heads/{develop}")).is_err() {
        git.run(["branch", "--no-track", develop, main]).unwrap();
    }
    for (key, value) in [
        ("gitflow.branch.master", main),
        ("gitflow.branch.develop", develop),
        ("gitflow.prefix.feature", prefixes[0]),
        ("gitflow.prefix.bugfix", prefixes[1]),
        ("gitflow.prefix.release", prefixes[2]),
        ("gitflow.prefix.hotfix", prefixes[3]),
        ("gitflow.prefix.support", prefixes[4]),
        ("gitflow.prefix.versiontag", prefixes[5]),
    ] {
        git.run(["config", key, value]).unwrap();
    }
}

const DEFAULTS: [&str; 4] = ["feature/", "release/", "hotfix/", ""];

/// The steps of a finish with how each went.
fn steps(out: &Value) -> Vec<(String, String)> {
    out["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|step| {
            (
                step["step"].as_str().unwrap().to_owned(),
                step["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

fn expect_steps(out: &Value, expected: &[(&str, &str)]) {
    let expected: Vec<(String, String)> = expected
        .iter()
        .map(|(step, state)| ((*step).to_owned(), (*state).to_owned()))
        .collect();
    assert_eq!(steps(out), expected, "{out}");
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        let twin = Scratch::of(fixture).unwrap();
        for copy in [&scratch, &twin] {
            // Commands run git with the real environment; merges and
            // annotated tags need a name.
            let git = copy.git();
            git.run(["config", "user.name", "Ada Fixture"]).unwrap();
            git.run(["config", "user.email", "ada@fixture.invalid"])
                .unwrap();
        }
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
    /// Gitflow set up with the defaults on both copies.
    fn set_up(fixture: &str) -> Self {
        let mut h = Self::new(fixture);
        h.run("flow.init", json!({}));
        init_by_hand(&h.twin, "main", "develop", DEFAULTS);
        h.assert_same("set up");
        h
    }
    fn git(&self) -> Git {
        self.scratch.git()
    }
    fn both(&self, f: impl Fn(&Scratch)) {
        f(&self.scratch);
        f(&self.twin);
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
    /// Undo until the journal says `until`, and say what was undone.
    fn undo_back_to(&mut self, until: &Value) -> Vec<String> {
        let mut undone = Vec::new();
        while self.peek() != *until {
            let out = self.run("undo.last", json!({}));
            undone.push(out["undone"].as_str().unwrap().to_owned());
            assert!(undone.len() < 50, "undo does not get anywhere");
        }
        undone
    }
}

#[test]
fn init_writes_the_settings_git_flow_reads_and_creates_develop() {
    let mut h = Harness::new("linear");
    let out = h.run("flow.status", json!({}));
    assert_eq!(out["initialized"], false);
    assert!(out["reason"].as_str().unwrap().contains("not set up"));
    assert!(out["main"].is_null() && out["develop"].is_null());
    assert_eq!(
        out["prefixes"],
        json!({
            "feature": "feature/", "bugfix": "bugfix/", "release": "release/",
            "hotfix": "hotfix/", "support": "support/", "tag": "",
        })
    );
    let before = state(&h.git());

    // Nothing given: main is the main branch, develop is made from it.
    let out = h.run("flow.init", json!({}));
    assert_eq!(out["initialized"], true);
    assert!(out["reason"].is_null());
    assert_eq!(out["main"], "main");
    assert_eq!(out["develop"], "develop");
    assert_eq!(out["created"], "develop");
    init_by_hand(&h.twin, "main", "develop", DEFAULTS);
    h.assert_same("init");
    assert_eq!(
        h.git().rev_parse("develop").unwrap(),
        h.git().rev_parse("main").unwrap()
    );
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "main\n",
        "nothing is checked out"
    );
    assert_eq!(h.run("flow.status", json!({}))["initialized"], true);

    // One entry holds the branch and the settings: undone, the repository
    // is what it was before gitflow was set up.
    assert_eq!(h.peek(), "Set up gitflow: create branch develop");
    h.run("undo.last", json!({}));
    assert!(h.git().rev_parse("refs/heads/develop").is_err());
    assert_eq!(state(&h.git()), before, "undo of init");
    let out = h.run("flow.status", json!({}));
    assert_eq!(out["initialized"], false);
    assert!(out["reason"].as_str().unwrap().contains("not set up"));
    h.run("redo.last", json!({}));
    h.assert_same("redo of init");
    assert_eq!(h.run("flow.status", json!({}))["initialized"], true);

    // Again: the same settings, nothing to create, nothing journalled.
    let out = h.run("flow.init", json!({}));
    assert!(out["created"].is_null());
    assert_eq!(out["initialized"], true);
    h.assert_same("init twice");
    assert_eq!(h.peek(), "Set up gitflow: create branch develop");

    // Other names and prefixes; what is not given again stays.
    let custom = json!({
        "develop": "dev", "feature": "feat/", "release": "rel/", "hotfix": "fix/", "tag": "v"
    });
    let out = h.run("flow.init", custom);
    assert_eq!(out["created"], "dev");
    assert_eq!(out["develop"], "dev");
    init_by_hand(&h.twin, "main", "dev", ["feat/", "rel/", "fix/", "v"]);
    h.assert_same("init with other names");
    let out = h.run("flow.init", json!({ "tag": "" }));
    assert_eq!(out["develop"], "dev");
    assert_eq!(
        out["prefixes"],
        json!({
            "feature": "feat/", "bugfix": "bugfix/", "release": "rel/",
            "hotfix": "fix/", "support": "support/", "tag": "",
        })
    );
    h.twin
        .git()
        .run(["config", "gitflow.prefix.versiontag", ""])
        .unwrap();
    h.assert_same("init keeps what is configured");

    // What cannot be: nothing is written, nothing is created.
    let settled = state(&h.git());
    let peek = h.peek();
    for (params, says) in [
        (json!({ "develop": "main" }), "cannot both be"),
        (json!({ "main": "dev" }), "cannot both be"),
        (json!({ "main": "no-such-branch" }), "no branch"),
        (json!({ "feature": "" }), "cannot begin"),
        (json!({ "release": "a b/" }), "cannot begin"),
        (json!({ "hotfix": "-x/" }), "cannot begin"),
        (json!({ "hotfix": "x..y/" }), "cannot begin"),
        (json!({ "feature": "same/", "release": "same/" }), "both"),
        (json!({ "bugfix": "fix/" }), "both"),
        (json!({ "support": "feat/" }), "both"),
        (json!({ "bugfix": "" }), "cannot begin"),
        (json!({ "support": "a b/" }), "cannot begin"),
        (json!({ "tag": "a b" }), "tag"),
        (json!({ "tag": "v~" }), "tag"),
        (json!({ "develop": "a..b" }), "valid"),
        (json!({ "develop": "new one" }), "valid"),
    ] {
        let err = h.try_run("flow.init", params.clone()).unwrap_err();
        assert!(err.contains(says), "{params}: {err}");
        assert_eq!(state(&h.git()), settled, "{params}");
    }
    assert_eq!(h.peek(), peek);
    assert_ne!(settled, before);
}

#[test]
fn init_finds_master_and_refuses_what_it_cannot_set_up() {
    // Only master: that is the main branch.
    let mut h = Harness::new("linear");
    h.both(|copy| {
        copy.git().run(["branch", "-m", "main", "master"]).unwrap();
    });
    let out = h.run("flow.init", json!({}));
    assert_eq!(out["main"], "master");
    assert_eq!(out["created"], "develop");
    init_by_hand(&h.twin, "master", "develop", DEFAULTS);
    h.assert_same("init on master");

    // Both: main. Neither: it has to be said.
    let mut h = Harness::new("linear");
    h.git().run(["branch", "master", "HEAD~1"]).unwrap();
    assert_eq!(h.run("flow.init", json!({}))["main"], "main");
    let mut h = Harness::new("linear");
    h.git().run(["branch", "-m", "main", "trunk"]).unwrap();
    let before = state(&h.git());
    let err = h.try_run("flow.init", json!({})).unwrap_err();
    assert!(err.contains("say which"), "{err}");
    assert_eq!(state(&h.git()), before);
    assert_eq!(
        h.run("flow.init", json!({ "main": "trunk" }))["main"],
        "trunk"
    );

    // A develop branch that is only on the remote is not made anew.
    let mut h = Harness::new("linear");
    let git = h.git();
    let bare = h.scratch.path().with_extension("origin.git");
    let _ = std::fs::remove_dir_all(&bare);
    git.run(["branch", "develop", "HEAD~1"]).unwrap();
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["branch", "-D", "develop"]).unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run(["fetch", "-q", "origin"]).unwrap();
    let before = state(&git);
    let err = h.try_run("flow.init", json!({})).unwrap_err();
    assert!(err.contains("origin/develop"), "{err}");
    assert_eq!(state(&git), before);
    git.run(["branch", "--track", "develop", "origin/develop"])
        .unwrap();
    let out = h.run("flow.init", json!({}));
    assert!(out["created"].is_null());
    assert_eq!(
        git.rev_parse("develop").unwrap(),
        git.rev_parse("HEAD~1").unwrap()
    );
    let _ = std::fs::remove_dir_all(&bare);

    // No commit yet: there is no main branch to speak of.
    let empty = h.scratch.path().with_extension("empty");
    let _ = std::fs::remove_dir_all(&empty);
    std::fs::create_dir_all(&empty).unwrap();
    let git = Git::new(&empty, fixtures_out_dir().join(".home"));
    git.run(["init", "-q"]).unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": empty }))
        .unwrap();
    let err = registry
        .run("flow.init", &mut ctx, &json!({}))
        .unwrap_err()
        .to_string();
    assert!(err.contains("no commit yet"), "{err}");
    let out = registry.run("flow.status", &mut ctx, &json!({})).unwrap();
    assert_eq!(out["initialized"], false);
    assert!(
        git.run(["config", "--local", "--get-regexp", r"^gitflow\."])
            .is_err(),
        "nothing was written"
    );
    for (command, params) in [
        ("flow.start", json!({ "kind": "feature", "name": "x" })),
        ("flow.finish", json!({ "kind": "feature", "name": "x" })),
    ] {
        assert!(registry.run(command, &mut ctx, &params).is_err());
    }
    let _ = std::fs::remove_dir_all(&empty);
}

#[test]
fn start_matches_git_switch_and_undoes() {
    let mut h = Harness::set_up("linear");
    let begin = state(&h.git());
    let peek = h.peek();

    // A feature from develop.
    let out = h.run("flow.start", json!({ "kind": "feature", "name": "login" }));
    assert_eq!(out["branch"], "feature/login");
    assert_eq!(out["base"], "develop");
    assert_eq!(out["target"], h.git().rev_parse("develop").unwrap());
    h.twin
        .git()
        .run([
            "switch",
            "-q",
            "--no-track",
            "-c",
            "feature/login",
            "develop",
        ])
        .unwrap();
    h.assert_same("start a feature");
    let status = h.run("flow.status", json!({}));
    assert_eq!(
        status["features"],
        json!([{
            "name": "login",
            "branch": "feature/login",
            "tip": h.git().rev_parse("develop").unwrap(),
            "current": true,
        }])
    );
    assert_eq!(status["releases"], json!([]));
    assert_eq!(h.peek(), "Start feature login");

    // Journalled like a branch created and checked out: one step back.
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.git()), begin);
    h.run("redo.last", json!({}));
    h.assert_same("redo of start");

    // A release from develop, a hotfix from main, which has moved on.
    h.both(|copy| {
        copy.git().run(["switch", "-q", "main"]).unwrap();
        commit(copy, "main.txt", "main\n", "on main");
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "develop.txt", "develop\n", "on develop");
    });
    let out = h.run("flow.start", json!({ "kind": "release", "name": "1.2" }));
    assert_eq!(out["branch"], "release/1.2");
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/1.2", "develop"])
        .unwrap();
    h.assert_same("start a release");
    let out = h.run("flow.start", json!({ "kind": "hotfix", "name": "1.1.1" }));
    assert_eq!(out["base"], "main");
    assert_eq!(out["target"], h.git().rev_parse("main").unwrap());
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "hotfix/1.1.1", "main"])
        .unwrap();
    h.assert_same("start a hotfix");

    // From somewhere else, when told so.
    let out = h.run(
        "flow.start",
        json!({ "kind": "feature", "name": "older", "base": "main~2" }),
    );
    assert_eq!(out["base"], "main~2");
    h.twin
        .git()
        .run([
            "switch",
            "-q",
            "--no-track",
            "-c",
            "feature/older",
            "main~2",
        ])
        .unwrap();
    h.assert_same("start from a given base");
    let status = h.run("flow.status", json!({}));
    let names = |list: &Value| -> Vec<String> {
        list.as_array()
            .unwrap()
            .iter()
            .map(|b| b["name"].as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(names(&status["features"]), ["login", "older"]);
    assert_eq!(names(&status["releases"]), ["1.2"]);
    assert_eq!(names(&status["hotfixes"]), ["1.1.1"]);

    // Taken names, a tag that is in the way of the release's own, a base
    // that is nothing: refused, nothing changed.
    h.both(|copy| copy.git().run(["tag", "2.0", "main"]).unwrap().clear());
    let settled = state(&h.git());
    let peek_now = h.peek();
    for (params, says) in [
        (json!({ "kind": "feature", "name": "login" }), "exists"),
        (json!({ "kind": "release", "name": "1.2" }), "exists"),
        (json!({ "kind": "release", "name": "2.0" }), "tag 2.0"),
        (json!({ "kind": "hotfix", "name": "2.0" }), "tag 2.0"),
        (
            json!({ "kind": "feature", "name": "x", "base": "nowhere" }),
            "does not name a commit",
        ),
    ] {
        let err = h.try_run("flow.start", params.clone()).unwrap_err();
        assert!(err.contains(says), "{params}: {err}");
        assert_eq!(state(&h.git()), settled, "{params}");
    }
    // A tag of that name is no obstacle to a feature.
    h.run("flow.start", json!({ "kind": "feature", "name": "2.0" }));
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.git()), settled);
    assert_eq!(h.peek(), peek_now);
    assert_ne!(peek_now, peek);

    // Not set up: refused.
    let mut bare = Harness::new("linear");
    let before = state(&bare.git());
    let err = bare
        .try_run("flow.start", json!({ "kind": "feature", "name": "x" }))
        .unwrap_err();
    assert!(err.contains("not set up"), "{err}");
    assert_eq!(state(&bare.git()), before);
}

#[test]
fn a_bugfix_starts_from_develop_and_finishes_into_it_like_a_feature() {
    let mut h = Harness::set_up("linear");
    // Main moves on: a bugfix is not started from there.
    h.both(|copy| {
        commit(copy, "main.txt", "main\n", "on main");
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "develop.txt", "develop\n", "on develop");
    });
    let begin = state(&h.git());
    let peek = h.peek();

    let out = h.run("flow.start", json!({ "kind": "bugfix", "name": "typo" }));
    assert_eq!(out["branch"], "bugfix/typo");
    assert_eq!(out["base"], "develop");
    assert_eq!(out["target"], h.git().rev_parse("develop").unwrap());
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "bugfix/typo", "develop"])
        .unwrap();
    h.assert_same("start a bugfix");
    let status = h.run("flow.status", json!({}));
    assert_eq!(
        status["bugfixes"],
        json!([{
            "name": "typo",
            "branch": "bugfix/typo",
            "tip": h.git().rev_parse("develop").unwrap(),
            "current": true,
        }])
    );
    assert_eq!(status["features"], json!([]));
    assert_eq!(h.peek(), "Start bugfix typo");
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.git()), begin);
    h.run("redo.last", json!({}));
    h.assert_same("redo of the start");

    // A tag of that name is no obstacle: a bugfix leaves none.
    h.both(|copy| copy.git().run(["tag", "typo", "main"]).unwrap().clear());
    h.both(|copy| {
        commit(copy, "typo.txt", "fixed\n", "bugfix: typo");
    });
    let before = state(&h.git());
    let started = h.peek();

    // Finished into develop with a merge commit, then deleted; main and
    // the tags are left alone.
    let main_before = h.git().rev_parse("main").unwrap();
    let out = h.run("flow.finish", json!({ "kind": "bugfix", "name": "typo" }));
    assert_eq!(out["finished"], true);
    assert_eq!(out["branch"], "bugfix/typo");
    assert!(out["tag"].is_null());
    expect_steps(
        &out,
        &[
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "done"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "bugfix/typo"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "bugfix/typo"]).unwrap();
    h.assert_same("finish a bugfix");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'bugfix/typo' into develop\n"
    );
    assert_eq!(h.git().rev_parse("main").unwrap(), main_before);
    assert_eq!(h.run("flow.status", json!({}))["bugfixes"], json!([]));
    let finished = state(&h.git());

    // Step by step in the journal, and back.
    assert_eq!(
        h.undo_back_to(&started),
        [
            "Finish bugfix typo: delete bugfix/typo",
            "Finish bugfix typo: merge into develop",
            "Finish bugfix typo: check out develop",
        ]
    );
    assert_eq!(state(&h.git()), before);
    for _ in 0..3 {
        h.run("redo.last", json!({}));
    }
    assert_eq!(state(&h.git()), finished);
    assert_ne!(h.peek(), peek);

    // Kept when asked, and then there is nothing left to do.
    h.run("flow.start", json!({ "kind": "bugfix", "name": "kept" }));
    commit(&h.scratch, "kept.txt", "kept\n", "bugfix: kept");
    let out = h.run(
        "flow.finish",
        json!({ "kind": "bugfix", "name": "kept", "keep": true }),
    );
    expect_steps(
        &out,
        &[
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "skipped"),
        ],
    );
    assert!(h.git().rev_parse("refs/heads/bugfix/kept").is_ok());

    // A bugfix that conflicts in develop stops like any merge.
    h.run("flow.start", json!({ "kind": "bugfix", "name": "clash" }));
    commit(
        &h.scratch,
        "develop.txt",
        "from the bugfix\n",
        "bugfix: clash",
    );
    h.git().run(["switch", "-q", "develop"]).unwrap();
    commit(
        &h.scratch,
        "develop.txt",
        "from develop\n",
        "develop: clash",
    );
    let out = h.run("flow.finish", json!({ "kind": "bugfix", "name": "clash" }));
    assert_eq!(out["finished"], false);
    assert_eq!(out["stopped_at"], "merge_develop");
    assert_eq!(out["conflicts"], json!(["develop.txt"]));
    expect_steps(
        &out,
        &[
            ("checkout_develop", "already"),
            ("merge_develop", "stopped"),
            ("delete", "pending"),
        ],
    );
    h.scratch.write("develop.txt", "both\n").unwrap();
    h.git().run(["add", "develop.txt"]).unwrap();
    h.run("op.continue", json!({}));
    let out = h.run("flow.finish", json!({ "kind": "bugfix", "name": "clash" }));
    assert_eq!(out["finished"], true);
    expect_steps(
        &out,
        &[
            ("checkout_develop", "already"),
            ("merge_develop", "already"),
            ("delete", "done"),
        ],
    );
    assert!(h.git().rev_parse("refs/heads/bugfix/clash").is_err());
}

#[test]
fn a_support_branch_starts_from_main_and_is_never_finished() {
    let mut h = Harness::set_up("linear");
    h.both(|copy| {
        commit(copy, "main.txt", "main\n", "on main");
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "develop.txt", "develop\n", "on develop");
    });
    let begin = state(&h.git());

    // From main, whatever is checked out.
    let out = h.run("flow.start", json!({ "kind": "support", "name": "1.x" }));
    assert_eq!(out["branch"], "support/1.x");
    assert_eq!(out["base"], "main");
    assert_eq!(out["target"], h.git().rev_parse("main").unwrap());
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "support/1.x", "main"])
        .unwrap();
    h.assert_same("start a support branch");
    assert_eq!(h.peek(), "Start support 1.x");
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.git()), begin);
    h.run("redo.last", json!({}));
    h.assert_same("redo of the start");

    // From the release it carries on, when that is said; a tag of its own
    // name is no obstacle, since it leaves none.
    h.both(|copy| copy.git().run(["tag", "0.9", "main~2"]).unwrap().clear());
    let out = h.run(
        "flow.start",
        json!({ "kind": "support", "name": "0.9", "base": "0.9" }),
    );
    assert_eq!(out["base"], "0.9");
    assert_eq!(out["target"], h.git().rev_parse("main~2").unwrap());
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "support/0.9", "0.9"])
        .unwrap();
    h.assert_same("start a support branch from a tag");
    let status = h.run("flow.status", json!({}));
    let names: Vec<&str> = status["supports"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| branch["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["0.9", "1.x"]);
    assert_eq!(status["hotfixes"], json!([]));

    // It is never finished: refused whatever is asked, nothing changed.
    commit(&h.scratch, "old.txt", "mended\n", "support: mend");
    commit(&h.twin, "old.txt", "mended\n", "support: mend");
    let settled = state(&h.git());
    let peek = h.peek();
    for params in [
        json!({ "kind": "support", "name": "0.9" }),
        json!({ "kind": "support", "name": "1.x", "keep": true }),
        json!({ "kind": "support", "name": "no-such" }),
        json!({ "kind": "support", "name": "0.9", "message": "never" }),
    ] {
        let err = h.try_run("flow.finish", params.clone()).unwrap_err();
        assert!(err.contains("is not finished"), "{params}: {err}");
        assert_eq!(state(&h.git()), settled, "{params}");
    }
    assert_eq!(h.peek(), peek);
    h.assert_same("a support branch that was not finished");

    // A name that is taken is refused like any other.
    let err = h
        .try_run("flow.start", json!({ "kind": "support", "name": "1.x" }))
        .unwrap_err();
    assert!(err.contains("exists"), "{err}");
    assert_eq!(state(&h.git()), settled);
}

#[test]
fn the_prefixes_of_all_five_kinds_are_changed_and_changed_back() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "bugfix", "name": "typo" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "bugfix/typo", "develop"])
        .unwrap();
    let before = state(&h.git());
    let peek = h.peek();

    // Every prefix another one: what is asked is what is written.
    let out = h.run(
        "flow.init",
        json!({
            "feature": "feat/", "bugfix": "fix/", "release": "rel/",
            "hotfix": "hot/", "support": "lts/", "tag": "v",
        }),
    );
    assert!(out["created"].is_null());
    assert_eq!(
        out["prefixes"],
        json!({
            "feature": "feat/", "bugfix": "fix/", "release": "rel/",
            "hotfix": "hot/", "support": "lts/", "tag": "v",
        })
    );
    init_all_by_hand(
        &h.twin,
        "main",
        "develop",
        ["feat/", "fix/", "rel/", "hot/", "lts/", "v"],
    );
    h.assert_same("six prefixes changed");
    // The branch under the old prefix is no bugfix any more.
    assert_eq!(out["bugfixes"], json!([]));

    // New branches take the new prefixes.
    for (kind, name, branch, from) in [
        ("bugfix", "crash", "fix/crash", "develop"),
        ("support", "2.x", "lts/2.x", "main"),
        ("feature", "search", "feat/search", "develop"),
    ] {
        let out = h.run("flow.start", json!({ "kind": kind, "name": name }));
        assert_eq!(out["branch"], branch);
        h.twin
            .git()
            .run(["switch", "-q", "--no-track", "-c", branch, from])
            .unwrap();
        h.assert_same(branch);
    }
    let status = h.run("flow.status", json!({}));
    assert_eq!(status["bugfixes"][0]["branch"], "fix/crash");
    assert_eq!(status["supports"][0]["branch"], "lts/2.x");
    assert_eq!(status["features"][0]["branch"], "feat/search");

    // The change of settings is one step of the journal: taken back, the
    // prefixes are what they were, and the old bugfix is one again.
    for _ in 0..3 {
        h.run("undo.last", json!({}));
    }
    assert_eq!(h.peek(), "Set up gitflow");
    h.run("undo.last", json!({}));
    assert_eq!(h.peek(), peek);
    assert_eq!(state(&h.git()), before);
    let status = h.run("flow.status", json!({}));
    assert_eq!(status["prefixes"]["bugfix"], "bugfix/");
    assert_eq!(status["prefixes"]["support"], "support/");
    assert_eq!(status["bugfixes"][0]["branch"], "bugfix/typo");

    // One prefix alone, the others as they are.
    let out = h.run("flow.init", json!({ "support": "old/" }));
    assert_eq!(out["prefixes"]["support"], "old/");
    assert_eq!(out["prefixes"]["bugfix"], "bugfix/");
    assert_eq!(out["prefixes"]["feature"], "feature/");

    // A repository git-flow set up without these two keys reads with the
    // usual prefixes for them.
    let git = h.git();
    for key in ["gitflow.prefix.bugfix", "gitflow.prefix.support"] {
        git.run(["config", "--unset", key]).unwrap();
    }
    let status = h.run("flow.status", json!({}));
    assert_eq!(status["initialized"], true);
    assert_eq!(status["prefixes"]["bugfix"], "bugfix/");
    assert_eq!(status["prefixes"]["support"], "support/");
    assert_eq!(status["bugfixes"][0]["branch"], "bugfix/typo");
}

#[test]
fn start_treats_local_changes_the_way_a_checkout_does() {
    let mut h = Harness::set_up("linear");
    // A change git can carry along comes along.
    h.both(|copy| copy.write("a.txt", "local change\n").unwrap());
    h.run(
        "flow.start",
        json!({ "kind": "feature", "name": "carried" }),
    );
    h.twin
        .git()
        .run([
            "switch",
            "-q",
            "--no-track",
            "-c",
            "feature/carried",
            "develop",
        ])
        .unwrap();
    h.assert_same("start with a change that comes along");
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("a.txt")).unwrap(),
        "local change\n"
    );

    // One that the checkout would overwrite refuses it, and the branch
    // that was made for it is not left behind.
    let before = state(&h.git());
    let peek = h.peek();
    let err = h
        .try_run(
            "flow.start",
            json!({ "kind": "feature", "name": "blocked", "base": "main~3" }),
        )
        .unwrap_err();
    assert!(err.contains("a.txt"), "{err}");
    assert_eq!(state(&h.git()), before);
    assert_eq!(h.peek(), peek);
    assert!(h.git().rev_parse("refs/heads/feature/blocked").is_err());
}

#[test]
fn finishing_a_feature_matches_the_steps_by_hand_and_undoes_step_by_step() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "feature", "name": "login" }));
    h.twin
        .git()
        .run([
            "switch",
            "-q",
            "--no-track",
            "-c",
            "feature/login",
            "develop",
        ])
        .unwrap();
    h.both(|copy| {
        commit(copy, "login.txt", "form\n", "login: form");
        commit(copy, "login.txt", "form\ncheck\n", "login: check");
    });
    let before = state(&h.git());
    let peek = h.peek();

    let out = h.run("flow.finish", json!({ "kind": "feature", "name": "login" }));
    assert_eq!(out["finished"], true);
    assert_eq!(out["changed"], true);
    assert_eq!(out["branch"], "feature/login");
    assert!(out["tag"].is_null());
    assert!(out["stopped"].is_null() && out["stopped_at"].is_null());
    expect_steps(
        &out,
        &[
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "done"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "feature/login"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "feature/login"]).unwrap();
    h.assert_same("finish a feature");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'feature/login' into develop\n",
        "git's own wording"
    );
    assert_eq!(
        h.git().rev_list_count("HEAD^2").unwrap(),
        7,
        "a merge commit although develop could have moved forward"
    );
    let finished = state(&h.git());

    // Each step is in the journal as what it is, under the finish's name.
    assert_eq!(
        h.undo_back_to(&peek),
        [
            "Finish feature login: delete feature/login",
            "Finish feature login: merge into develop",
            "Finish feature login: check out develop",
        ]
    );
    assert_eq!(state(&h.git()), before);
    for _ in 0..3 {
        h.run("redo.last", json!({}));
    }
    assert_eq!(state(&h.git()), finished);

    // Again: the branch is gone, and so there is nothing to finish.
    let err = h
        .try_run("flow.finish", json!({ "kind": "feature", "name": "login" }))
        .unwrap_err();
    assert!(err.contains("no branch feature/login"), "{err}");
    assert_eq!(state(&h.git()), finished);

    // A feature nothing was committed on has nothing to merge: it goes.
    h.run("flow.start", json!({ "kind": "feature", "name": "empty" }));
    let out = h.run("flow.finish", json!({ "kind": "feature", "name": "empty" }));
    expect_steps(
        &out,
        &[
            ("checkout_develop", "done"),
            ("merge_develop", "already"),
            ("delete", "done"),
        ],
    );
    assert_eq!(state(&h.git()), finished);
}

#[test]
fn finishing_a_release_and_a_hotfix_matches_the_steps_by_hand() {
    let mut h = Harness::new("linear");
    h.run("flow.init", json!({ "tag": "v" }));
    init_by_hand(
        &h.twin,
        "main",
        "develop",
        ["feature/", "release/", "hotfix/", "v"],
    );

    // A release: develop has work main lacks, the release adds to it.
    h.both(|copy| {
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "work.txt", "work\n", "work for the release");
    });
    h.run("flow.start", json!({ "kind": "release", "name": "1.2" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/1.2", "develop"])
        .unwrap();
    h.both(|copy| commit(copy, "VERSION", "1.2\n", "release: version 1.2"));
    // Develop moves on meanwhile, somewhere else.
    h.both(|copy| {
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "next.txt", "next\n", "work for the one after");
        copy.git().run(["switch", "-q", "release/1.2"]).unwrap();
    });
    let before = state(&h.git());
    let peek = h.peek();

    let out = h.run(
        "flow.finish",
        json!({ "kind": "release", "name": "1.2", "message": "One point two\n\nwith notes" }),
    );
    assert_eq!(out["finished"], true);
    assert_eq!(out["tag"], "v1.2");
    assert_eq!(out["main"], "main");
    assert_eq!(out["develop"], "develop");
    expect_steps(
        &out,
        &[
            ("checkout_main", "done"),
            ("merge_main", "done"),
            ("tag", "done"),
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "done"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/1.2"])
        .unwrap();
    twin.run(["tag", "-a", "-m", "One point two\n\nwith notes", "v1.2"])
        .unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/1.2"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "release/1.2"]).unwrap();
    h.assert_same("finish a release");
    let git = h.git();
    assert_eq!(
        git.run(["cat-file", "-t", "refs/tags/v1.2"]).unwrap(),
        "tag\n"
    );
    assert_eq!(
        git.rev_parse("v1.2^{commit}").unwrap(),
        git.rev_parse("main").unwrap(),
        "the tag is on main"
    );
    assert_eq!(
        git.run(["log", "-1", "--format=%s", "main"]).unwrap(),
        "Merge branch 'release/1.2'\n"
    );
    assert_eq!(
        git.run(["log", "-1", "--format=%s", "develop"]).unwrap(),
        "Merge branch 'release/1.2' into develop\n"
    );
    let finished = state(&git);

    // All of it back, one step at a time, and forward again.
    assert_eq!(
        h.undo_back_to(&peek),
        [
            "Finish release 1.2: delete release/1.2",
            "Finish release 1.2: merge into develop",
            "Finish release 1.2: check out develop",
            "Finish release 1.2: tag v1.2",
            "Finish release 1.2: merge into main",
            "Finish release 1.2: check out main",
        ]
    );
    assert_eq!(state(&h.git()), before);
    for _ in 0..6 {
        h.run("redo.last", json!({}));
    }
    assert_eq!(state(&h.git()), finished);

    // A hotfix: from main, into main with a tag, and into develop. The
    // tag's message names it when none is given.
    h.run("flow.start", json!({ "kind": "hotfix", "name": "1.2.1" }));
    twin.run(["switch", "-q", "--no-track", "-c", "hotfix/1.2.1", "main"])
        .unwrap();
    h.both(|copy| commit(copy, "VERSION", "1.2.1\n", "hotfix: version 1.2.1"));
    let out = h.run(
        "flow.finish",
        json!({ "kind": "hotfix", "name": "1.2.1", "message": "  " }),
    );
    assert_eq!(out["finished"], true);
    assert_eq!(out["tag"], "v1.2.1");
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "hotfix/1.2.1"])
        .unwrap();
    twin.run(["tag", "-a", "-m", "Hotfix 1.2.1", "v1.2.1"])
        .unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "hotfix/1.2.1"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "hotfix/1.2.1"]).unwrap();
    h.assert_same("finish a hotfix");

    // Finished and gone: a second finish has no branch, and says that the
    // tag is there.
    let finished = state(&h.git());
    let err = h
        .try_run("flow.finish", json!({ "kind": "hotfix", "name": "1.2.1" }))
        .unwrap_err();
    assert!(err.contains("no branch hotfix/1.2.1"), "{err}");
    assert!(err.contains("tag v1.2.1 exists"), "{err}");
    assert_eq!(state(&h.git()), finished);
}

#[test]
fn keep_leaves_the_branch_and_a_finish_run_again_does_what_is_left() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "release", "name": "3.0" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/3.0", "develop"])
        .unwrap();
    h.both(|copy| commit(copy, "VERSION", "3.0\n", "release: version 3.0"));

    let params = json!({ "kind": "release", "name": "3.0", "keep": true });
    let out = h.run("flow.finish", params.clone());
    assert_eq!(out["finished"], true);
    expect_steps(
        &out,
        &[
            ("checkout_main", "done"),
            ("merge_main", "done"),
            ("tag", "done"),
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "skipped"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/3.0"])
        .unwrap();
    twin.run(["tag", "-a", "-m", "Release 3.0", "3.0"]).unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/3.0"])
        .unwrap();
    h.assert_same("finish and keep");
    assert!(h.git().rev_parse("refs/heads/release/3.0").is_ok());

    // Twice in a row: everything is found done, and the answer says so.
    let kept = state(&h.git());
    let peek = h.peek();
    let out = h.run("flow.finish", params);
    assert_eq!(out["finished"], true);
    assert_eq!(out["changed"], false);
    expect_steps(
        &out,
        &[
            ("checkout_main", "already"),
            ("merge_main", "already"),
            ("tag", "already"),
            ("checkout_develop", "already"),
            ("merge_develop", "already"),
            ("delete", "skipped"),
        ],
    );
    assert_eq!(state(&h.git()), kept);
    assert_eq!(h.peek(), peek);

    // Without keep, from the branch itself: only what is left is done,
    // and HEAD has to leave the branch first. Nothing is merged any
    // more, so a local change is the checkout's to carry along.
    h.both(|copy| {
        copy.git().run(["switch", "-q", "release/3.0"]).unwrap();
        copy.write("a.txt", "being worked on\n").unwrap();
    });
    let out = h.run("flow.finish", json!({ "kind": "release", "name": "3.0" }));
    assert_eq!(out["changed"], true);
    expect_steps(
        &out,
        &[
            ("checkout_main", "already"),
            ("merge_main", "already"),
            ("tag", "already"),
            ("checkout_develop", "done"),
            ("merge_develop", "already"),
            ("delete", "done"),
        ],
    );
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["branch", "-q", "-d", "release/3.0"]).unwrap();
    h.assert_same("finish what was kept");
    assert_eq!(
        h.git().run(["status", "--porcelain"]).unwrap(),
        " M a.txt\n"
    );
    assert_eq!(
        h.undo_back_to(&peek),
        [
            "Finish release 3.0: delete release/3.0",
            "Finish release 3.0: check out develop",
        ]
    );
}

#[test]
fn a_finish_that_fails_half_way_says_where_and_goes_on_when_run_again() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "release", "name": "2.0" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/2.0", "develop"])
        .unwrap();
    h.both(|copy| commit(copy, "VERSION", "2.0\n", "release: version 2.0"));
    // Nothing a look at the repository would call wrong, and still git
    // cannot make the tag `2.0` while there is a tag `2.0/rc`.
    h.git().run(["tag", "2.0/rc", "main"]).unwrap();
    let params = json!({ "kind": "release", "name": "2.0" });

    let err = h.try_run("flow.finish", params.clone()).unwrap_err();
    assert!(err.contains("failed at the tag"), "{err}");
    assert!(err.contains("2.0/rc"), "{err}");
    // What was done before it stays done, and the journal has it.
    assert_eq!(h.peek(), "Finish release 2.0: merge into main");
    assert_eq!(
        h.git().run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "main\n"
    );
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap(),
        "Merge branch 'release/2.0'\n"
    );

    // With that out of the way, the same finish does the rest.
    h.git().run(["tag", "-d", "2.0/rc"]).unwrap();
    let out = h.run("flow.finish", params);
    assert_eq!(out["finished"], true);
    expect_steps(
        &out,
        &[
            ("checkout_main", "already"),
            ("merge_main", "already"),
            ("tag", "done"),
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "done"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/2.0"])
        .unwrap();
    twin.run(["tag", "-a", "-m", "Release 2.0", "2.0"]).unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/2.0"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "release/2.0"]).unwrap();
    h.assert_same("finished after a failure");
}

#[test]
fn a_release_that_conflicts_in_develop_stops_and_goes_on_when_run_again() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "release", "name": "1.2" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "release/1.2", "develop"])
        .unwrap();
    // The release and develop change the same line.
    h.both(|copy| {
        commit(copy, "a.txt", "release\n", "release: a");
        copy.git().run(["switch", "-q", "develop"]).unwrap();
        commit(copy, "a.txt", "develop\n", "develop: a");
    });
    let before = state(&h.git());
    let peek = h.peek();
    let params = json!({ "kind": "release", "name": "1.2" });

    // Main and the tag go through; the way back into develop stops.
    let out = h.run("flow.finish", params.clone());
    assert_eq!(out["finished"], false);
    assert_eq!(out["changed"], true);
    assert_eq!(out["stopped_at"], "merge_develop");
    assert!(out["stopped"].is_string(), "{out}");
    assert_eq!(out["conflicts"], json!(["a.txt"]));
    assert_eq!(out["before"]["branch"], "develop");
    assert_eq!(out["before"], out["after"]);
    expect_steps(
        &out,
        &[
            ("checkout_main", "done"),
            ("merge_main", "done"),
            ("tag", "done"),
            ("checkout_develop", "done"),
            ("merge_develop", "stopped"),
            ("delete", "pending"),
        ],
    );
    let twin = h.twin.git();
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "release/1.2"])
        .unwrap();
    twin.run(["tag", "-a", "-m", "Release 1.2", "1.2"]).unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    assert!(
        twin.run(["merge", "--no-ff", "--no-edit", "release/1.2"])
            .is_err()
    );
    h.assert_same("stopped in the merge into develop");
    assert_eq!(h.run("op.status", json!({}))["operation"]["kind"], "merge");
    // The merge is not in the journal until it is committed.
    assert_eq!(h.peek(), "Finish release 1.2: check out develop");

    // While it is stopped, a finish does not begin again.
    let stopped = state(&h.git());
    let err = h.try_run("flow.finish", params.clone()).unwrap_err();
    assert!(err.contains("a merge is in progress"), "{err}");
    assert_eq!(state(&h.git()), stopped);

    // Resolved and committed like any merge.
    h.run(
        "conflict.take",
        json!({ "path": "a.txt", "side": "theirs" }),
    );
    assert_eq!(h.run("op.continue", json!({}))["finished"], true);
    twin.run(["checkout", "--theirs", "--", "a.txt"]).unwrap();
    twin.run(["add", "a.txt"]).unwrap();
    twin.run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("the merge committed");
    assert_eq!(h.peek(), "Finish release 1.2: merge into develop");

    // The same finish again does the rest.
    let out = h.run("flow.finish", params);
    assert_eq!(out["finished"], true);
    assert!(out["stopped"].is_null());
    assert_eq!(out["conflicts"], json!([]));
    expect_steps(
        &out,
        &[
            ("checkout_main", "already"),
            ("merge_main", "already"),
            ("tag", "already"),
            ("checkout_develop", "already"),
            ("merge_develop", "already"),
            ("delete", "done"),
        ],
    );
    twin.run(["branch", "-q", "-d", "release/1.2"]).unwrap();
    h.assert_same("finished after the conflict");

    // And the whole of it is taken back through the journal.
    assert_eq!(
        h.undo_back_to(&peek),
        [
            "Finish release 1.2: delete release/1.2",
            "Finish release 1.2: merge into develop",
            "Finish release 1.2: check out develop",
            "Finish release 1.2: tag 1.2",
            "Finish release 1.2: merge into main",
            "Finish release 1.2: check out main",
        ]
    );
    assert_eq!(state(&h.git()), before);
}

#[test]
fn a_hotfix_that_conflicts_in_main_can_be_aborted_and_finished_later() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "hotfix", "name": "0.9.1" }));
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "hotfix/0.9.1", "main"])
        .unwrap();
    h.both(|copy| {
        commit(copy, "b.txt", "hotfix\n", "hotfix: b");
        copy.git().run(["switch", "-q", "main"]).unwrap();
        commit(copy, "b.txt", "main moved\n", "main: b");
    });
    let before = state(&h.git());
    let peek = h.peek();
    let params = json!({ "kind": "hotfix", "name": "0.9.1" });
    let stopped_steps = [
        ("checkout_main", "already"),
        ("merge_main", "stopped"),
        ("tag", "pending"),
        ("checkout_develop", "pending"),
        ("merge_develop", "pending"),
        ("delete", "pending"),
    ];

    let out = h.run("flow.finish", params.clone());
    assert_eq!(out["stopped_at"], "merge_main");
    assert_eq!(out["conflicts"], json!(["b.txt"]));
    expect_steps(&out, &stopped_steps);
    let twin = h.twin.git();
    assert!(
        twin.run(["merge", "--no-ff", "--no-edit", "hotfix/0.9.1"])
            .is_err()
    );
    h.assert_same("stopped in the merge into main");
    assert_eq!(h.peek(), peek, "nothing is journaled yet");

    // Given up: as if the finish had not been asked for.
    h.run("op.abort", json!({}));
    twin.run(["merge", "--abort"]).unwrap();
    assert_eq!(state(&h.git()), before);

    // Asked again, resolved this time, and then the rest.
    let out = h.run("flow.finish", params.clone());
    expect_steps(&out, &stopped_steps);
    assert!(
        twin.run(["merge", "--no-ff", "--no-edit", "hotfix/0.9.1"])
            .is_err()
    );
    h.run("conflict.take", json!({ "path": "b.txt", "side": "ours" }));
    h.run("op.continue", json!({}));
    twin.run(["checkout", "--ours", "--", "b.txt"]).unwrap();
    twin.run(["add", "b.txt"]).unwrap();
    twin.run(["commit", "-q", "--no-edit"]).unwrap();
    h.assert_same("the merge into main committed");

    let out = h.run("flow.finish", params);
    assert_eq!(out["finished"], true);
    expect_steps(
        &out,
        &[
            ("checkout_main", "already"),
            ("merge_main", "already"),
            ("tag", "done"),
            ("checkout_develop", "done"),
            ("merge_develop", "done"),
            ("delete", "done"),
        ],
    );
    twin.run(["tag", "-a", "-m", "Hotfix 0.9.1", "0.9.1"])
        .unwrap();
    twin.run(["switch", "-q", "develop"]).unwrap();
    twin.run(["merge", "-q", "--no-ff", "--no-edit", "hotfix/0.9.1"])
        .unwrap();
    twin.run(["branch", "-q", "-d", "hotfix/0.9.1"]).unwrap();
    h.assert_same("finished after the conflict in main");
    assert_eq!(
        h.git().rev_parse("0.9.1^{commit}").unwrap(),
        h.git().rev_parse("main").unwrap()
    );
    assert_eq!(h.undo_back_to(&peek).len(), 5);
    assert_eq!(state(&h.git()), before);
}

#[test]
fn a_finish_refuses_before_it_changes_anything() {
    // Not set up.
    let mut h = Harness::new("branches-merges");
    let before = state(&h.git());
    let err = h
        .try_run("flow.finish", json!({ "kind": "feature", "name": "a" }))
        .unwrap_err();
    assert!(err.contains("not set up"), "{err}");
    assert_eq!(state(&h.git()), before);

    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "feature", "name": "work" }));
    commit(&h.scratch, "new.txt", "from the feature\n", "feature: new");
    h.run("flow.start", json!({ "kind": "release", "name": "1.0" }));
    commit(&h.scratch, "VERSION", "1.0\n", "release: version");
    let git = h.git();
    git.run(["switch", "-q", "develop"]).unwrap();
    let feature = json!({ "kind": "feature", "name": "work" });
    let release = json!({ "kind": "release", "name": "1.0" });
    let peek = h.peek();
    let refuse = |h: &mut Harness, params: &Value, says: &str| {
        let before = state(&h.git());
        let err = h.try_run("flow.finish", params.clone()).unwrap_err();
        assert!(err.contains(says), "{params}: {err}");
        assert_eq!(state(&h.git()), before, "{params}: {says}");
        assert_eq!(h.peek(), peek, "{params}: {says}");
    };

    // No such branch.
    refuse(
        &mut h,
        &json!({ "kind": "feature", "name": "nothing" }),
        "no branch feature/nothing",
    );
    refuse(
        &mut h,
        &json!({ "kind": "hotfix", "name": "work" }),
        "no branch hotfix/work",
    );

    // Local changes: to a tracked file, staged or not, and an untracked
    // file where the feature has one.
    h.scratch.write("b.txt", "edited\n").unwrap();
    refuse(&mut h, &feature, "local changes to b.txt");
    git.run(["add", "b.txt"]).unwrap();
    refuse(&mut h, &release, "local changes to b.txt");
    git.run(["reset", "-q", "--hard"]).unwrap();
    h.scratch.write("new.txt", "mine\n").unwrap();
    refuse(&mut h, &feature, "new.txt");
    h.scratch.remove("new.txt").unwrap();
    // An untracked file nothing writes to is no obstacle; see below.
    h.scratch.write("notes.txt", "mine\n").unwrap();

    // An operation in progress.
    let merge_head = h.scratch.path().join(".git/MERGE_HEAD");
    std::fs::write(&merge_head, format!("{}\n", git.rev_parse("main").unwrap())).unwrap();
    refuse(&mut h, &feature, "a merge is in progress");
    std::fs::remove_file(&merge_head).unwrap();

    // A tag of the release's name on something that is not the release.
    git.run(["tag", "1.0", "main~2"]).unwrap();
    refuse(&mut h, &release, "tag 1.0 exists already");
    git.run(["tag", "-d", "1.0"]).unwrap();

    // The name means something else as well: git would merge that.
    git.run(["tag", "feature/work", "main~2"]).unwrap();
    refuse(&mut h, &feature, "ambiguous");
    git.run(["tag", "-d", "feature/work"]).unwrap();

    // A branch another worktree holds: the one to merge into, and the
    // one to delete.
    let other = h.scratch.path().with_extension("wt");
    let _ = std::fs::remove_dir_all(&other);
    git.run(["worktree", "add", "-q", other.to_str().unwrap(), "main"])
        .unwrap();
    refuse(&mut h, &release, "main is checked out in the worktree");
    let there = Git::new(&other, fixtures_out_dir().join(".home"));
    there.run(["switch", "-q", "feature/work"]).unwrap();
    refuse(
        &mut h,
        &feature,
        "feature/work is checked out in the worktree",
    );
    git.run(["worktree", "remove", "--force", other.to_str().unwrap()])
        .unwrap();

    // A branch with no history in common cannot be merged.
    git.run(["switch", "-q", "--orphan", "feature/alone"])
        .unwrap();
    commit(&h.scratch, "alone.txt", "alone\n", "from nowhere");
    git.run(["switch", "-q", "develop"]).unwrap();
    refuse(
        &mut h,
        &json!({ "kind": "feature", "name": "alone" }),
        "no history in common",
    );

    // The main and develop branches are not something to finish.
    git.run(["config", "gitflow.prefix.feature", "dev"])
        .unwrap();
    refuse(
        &mut h,
        &json!({ "kind": "feature", "name": "elop" }),
        "one of the two branches",
    );
    git.run(["config", "gitflow.prefix.feature", "feature/"])
        .unwrap();

    // With all of that out of the way both go through, the untracked
    // file untouched.
    assert_eq!(h.run("flow.finish", feature)["finished"], true);
    assert_eq!(h.run("flow.finish", release)["finished"], true);
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join("notes.txt")).unwrap(),
        "mine\n"
    );
    assert_eq!(
        git.run(["status", "--porcelain"]).unwrap(),
        "?? notes.txt\n"
    );
}

#[test]
fn hostile_flow_parameters_and_repositories_are_survived() {
    let mut h = Harness::set_up("linear");
    h.run("flow.start", json!({ "kind": "feature", "name": "real" }));
    commit(&h.scratch, "real.txt", "real\n", "feature: real");
    h.git().run(["switch", "-q", "develop"]).unwrap();
    let before = state(&h.git());
    let peek = h.peek();
    let long = "n".repeat(5000);
    let huge = "m".repeat(200_000);

    for name in [
        "",
        "-d",
        "--force",
        "-",
        "a..b",
        "a b",
        "a\nb",
        "a\u{0}b",
        "\u{1b}[31m",
        "end.lock",
        "../escape",
        "/leading",
        "trailing/",
        "@{-1}",
        "a@{1}",
        "a~1",
        "a^",
        "a:b",
        "a?b",
        "a*",
        "a[b",
        "a\\b",
        long.as_str(),
    ] {
        for kind in ["feature", "bugfix", "release", "hotfix", "support"] {
            for (command, params) in [
                ("flow.start", json!({ "kind": kind, "name": name })),
                ("flow.finish", json!({ "kind": kind, "name": name })),
                (
                    "flow.finish",
                    json!({ "kind": kind, "name": name, "keep": true }),
                ),
            ] {
                assert!(
                    h.try_run(command, params).is_err(),
                    "{command} accepted the {kind} name {name:?}"
                );
            }
        }
        for key in ["main", "develop"] {
            assert!(
                h.try_run("flow.init", json!({ key: name })).is_err(),
                "flow.init accepted {name:?} as {key}"
            );
        }
        assert_eq!(state(&h.git()), before, "after the name {name:?}");
    }
    // A prefix is not a name: `trailing/` is how one usually looks.
    for prefix in [
        "",
        "-x/",
        "--",
        "a b/",
        "a\nb/",
        "..",
        "a../",
        "x~/",
        "/x/",
        "@{",
        long.as_str(),
    ] {
        for key in ["feature", "bugfix", "release", "hotfix", "support"] {
            assert!(
                h.try_run("flow.init", json!({ key: prefix })).is_err(),
                "flow.init accepted the {key} prefix {prefix:?}"
            );
        }
        assert_eq!(state(&h.git()), before, "after the prefix {prefix:?}");
    }
    for base in [
        "",
        "-x",
        "--orphan",
        "nowhere",
        "a\nb",
        "HEAD^{tree}",
        long.as_str(),
    ] {
        assert!(
            h.try_run(
                "flow.start",
                json!({ "kind": "feature", "name": "fine", "base": base })
            )
            .is_err(),
            "flow.start accepted the base {base:?}"
        );
    }
    for tag in ["-x", "a b", "a\nb", "x..", "~", long.as_str()] {
        assert!(
            h.try_run("flow.init", json!({ "tag": tag })).is_err(),
            "flow.init accepted the tag prefix {tag:?}"
        );
    }
    for params in [
        json!({}),
        json!({ "kind": "feature" }),
        json!({ "name": "real" }),
        json!({ "kind": "support", "name": "real" }),
        json!({ "kind": "Feature", "name": "real" }),
        json!({ "kind": 1, "name": "real" }),
        json!({ "kind": "feature", "name": 7 }),
        json!({ "kind": "feature", "name": ["real"] }),
        json!({ "kind": "feature", "name": "real", "keep": "yes" }),
        json!({ "kind": "feature", "name": "real", "base": 3 }),
        json!({ "kind": "feature", "name": "real", "message": 3 }),
    ] {
        for command in ["flow.start", "flow.finish"] {
            // `base` and `message` belong to one of the two each; the
            // other does not read them and refuses for its own reasons.
            if h.try_run(command, params.clone()).is_ok() {
                h.undo_back_to(&peek);
            }
        }
        assert_eq!(state(&h.git()), before, "after {params}");
    }
    for params in [
        json!({ "main": 3 }),
        json!({ "develop": ["x"] }),
        json!({ "feature": true }),
        json!({ "tag": {} }),
    ] {
        assert!(h.try_run("flow.init", params.clone()).is_err(), "{params}");
    }
    // A tag message that cannot be a tag message, found out before the
    // release is merged anywhere.
    h.run("flow.start", json!({ "kind": "release", "name": "9.9" }));
    let started = state(&h.git());
    for message in ["nul\u{0}", huge.as_str()] {
        assert!(
            h.try_run(
                "flow.finish",
                json!({ "kind": "release", "name": "9.9", "message": message })
            )
            .is_err()
        );
        assert_eq!(state(&h.git()), started);
    }
    h.undo_back_to(&peek);
    assert_eq!(state(&h.git()), before);
    assert_eq!(h.peek(), peek);

    // Settings no tool would write. Whatever the commands make of them,
    // a refusal leaves the repository alone. References are compared as
    // they are from here on: some of them will not be commits.
    let state = |git: &Git| {
        let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
        out.push(git.run(["symbolic-ref", "HEAD"]).unwrap());
        out.push(git.run(["config", "--local", "--list"]).unwrap());
        out.push(git.run(["status", "--porcelain"]).unwrap());
        out
    };
    let git = h.git();
    let every = |h: &mut Harness, what: &str| {
        let git = h.git();
        let before = state(&git);
        let peek = h.peek();
        let out = h.try_run("flow.status", json!({}));
        assert!(out.is_ok(), "{what}: {out:?}");
        assert_eq!(state(&git), before, "{what}: status");
        for (command, params) in [
            ("flow.start", json!({ "kind": "feature", "name": "probe" })),
            ("flow.start", json!({ "kind": "release", "name": "0.0" })),
            ("flow.finish", json!({ "kind": "feature", "name": "real" })),
            ("flow.finish", json!({ "kind": "hotfix", "name": "real" })),
            ("flow.init", json!({})),
        ] {
            match h.try_run(command, params) {
                Err(_) => assert_eq!(state(&git), before, "{what}: {command}"),
                Ok(_) if command == "flow.init" => {}
                Ok(_) => {
                    h.undo_back_to(&peek);
                }
            }
        }
    };
    for (key, values) in [
        (
            "gitflow.branch.master",
            vec![
                "",
                "-x",
                "main~1",
                "refs/heads/main",
                "a b",
                "develop",
                "HEAD",
                "nowhere",
                long.as_str(),
            ],
        ),
        (
            "gitflow.branch.develop",
            vec!["", "--orphan", "develop^", "main", "@{-1}", "a\nb"],
        ),
        (
            "gitflow.prefix.feature",
            vec!["", "-", "..", "a b/", "\n", "feature", "release/", "~"],
        ),
        (
            "gitflow.prefix.versiontag",
            vec!["-", "a b", "..", "\n", "^", long.as_str()],
        ),
    ] {
        let was = git.run(["config", "--local", "--get", key]).unwrap();
        let was = was.strip_suffix('\n').unwrap_or(&was);
        for value in values {
            git.run(["config", "--local", key, value]).unwrap();
            every(&mut h, &format!("{key} = {value:?}"));
            git.run(["config", "--local", key, was]).unwrap();
        }
    }
    // One key set twice, and a key without a value.
    git.run([
        "config",
        "--local",
        "--add",
        "gitflow.branch.master",
        "main",
    ])
    .unwrap();
    every(&mut h, "a key set twice");
    let config = h.scratch.path().join(".git/config");
    let text = std::fs::read_to_string(&config).unwrap();
    std::fs::write(&config, format!("{text}[gitflow \"prefix\"]\n\thotfix\n")).unwrap();
    every(&mut h, "a key without a value");
    std::fs::write(&config, &text).unwrap();

    // State files that are not what they should be.
    let dot = h.scratch.path().join(".git");
    for (file, content) in [
        ("MERGE_HEAD", ""),
        ("MERGE_HEAD", "not an id\n"),
        ("CHERRY_PICK_HEAD", "\u{0}\u{1}\u{2}"),
        ("bitbraid/operation.json", "{ not json"),
        ("bitbraid/journal.json", "[1, 2"),
        ("refs/heads/feature/broken", "not an id\n"),
        ("refs/heads/feature/empty", ""),
        ("refs/heads/release/tree", ""),
        ("refs/tags/real", "garbage\n"),
    ] {
        let path = dot.join(file);
        let had = std::fs::read(&path).ok();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let content = if file.ends_with("/tree") {
            // A branch that points at a tree.
            format!("{}\n", git.rev_parse("HEAD^{tree}").unwrap())
        } else {
            content.to_owned()
        };
        std::fs::write(&path, content).unwrap();
        every(&mut h, file);
        match had {
            Some(bytes) => std::fs::write(&path, bytes).unwrap(),
            None => std::fs::remove_file(&path).unwrap(),
        }
    }
    std::fs::create_dir_all(dot.join("rebase-merge")).unwrap();
    every(&mut h, "an empty rebase-merge directory");
    std::fs::remove_dir_all(dot.join("rebase-merge")).unwrap();

    // And a repository git itself no longer reads: errors, nothing more.
    std::fs::write(dot.join("HEAD"), "garbage\n").unwrap();
    for (command, params) in [
        ("flow.status", json!({})),
        ("flow.init", json!({})),
        ("flow.start", json!({ "kind": "feature", "name": "probe" })),
        ("flow.finish", json!({ "kind": "feature", "name": "real" })),
    ] {
        let _ = h.try_run(command, params);
    }
}
