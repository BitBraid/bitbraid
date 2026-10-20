//! `repo.graph` with branches pinned to the left, against git:
//! the commits of a pinned branch are its tip and the first parents below
//! it, as `git rev-list --first-parent` lists them, and those have the
//! first lane.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, list_scripts};
use serde_json::{Value, json};

fn open(fixture: &str) -> (Scratch, Registry, CommandContext) {
    let scratch = Scratch::of(fixture).unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .expect("open scratch");
    (scratch, registry, ctx)
}

/// The lane of every commit, by id, in row order.
fn lanes(graph: &Value) -> Vec<(String, u64)> {
    let commits = graph["commits"].as_array().unwrap();
    let rows = graph["rows"].as_array().unwrap();
    assert_eq!(commits.len(), rows.len());
    commits
        .iter()
        .zip(rows)
        .map(|(commit, row)| {
            (
                commit["id"].as_str().unwrap().to_owned(),
                row["lane"].as_u64().unwrap(),
            )
        })
        .collect()
}

fn branches(git: &Git) -> Vec<String> {
    git.lines(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .unwrap()
}

#[test]
fn a_pinned_branch_has_the_first_lane_along_its_first_parents() {
    let mut moved = 0;
    for fixture in list_scripts().unwrap() {
        let (scratch, registry, mut ctx) = open(&fixture);
        let git = scratch.git();
        let plain = registry.run("repo.graph", &mut ctx, &json!({})).unwrap();
        for branch in branches(&git) {
            let pinned = registry
                .run("repo.graph", &mut ctx, &json!({ "pinned": [branch] }))
                .unwrap();
            // The same commits in the same order, whatever is pinned.
            assert_eq!(pinned["commits"], plain["commits"], "{fixture} {branch}");
            let line = git
                .lines(["rev-list", "--first-parent", branch.as_str()])
                .unwrap();
            let lanes = lanes(&pinned);
            let root = lanes
                .iter()
                .position(|(id, _)| id == line.last().unwrap())
                .unwrap();
            for (row, (id, lane)) in lanes.iter().enumerate() {
                if line.contains(id) {
                    assert_eq!(*lane, 0, "{fixture}: {branch} row {row}");
                } else if row <= root {
                    assert_ne!(*lane, 0, "{fixture}: {branch} row {row}");
                }
            }
            if pinned["rows"] != plain["rows"] {
                moved += 1;
            }
        }
    }
    // The fixtures have to have branches that pinning moves, or they test
    // nothing of it.
    assert!(moved >= 3, "pinning moved lanes for {moved} branches only");
}

#[test]
fn pins_come_in_the_order_they_are_named_and_show_in_the_drawing() {
    let (scratch, registry, mut ctx) = open("branches-merges");
    let git = scratch.git();
    let line = |branch: &str| git.lines(["rev-list", "--first-parent", branch]).unwrap();
    let run = |ctx: &mut CommandContext, pinned: Value| {
        registry
            .run(
                "repo.graph",
                ctx,
                &json!({ "pinned": pinned, "ascii": true }),
            )
            .unwrap()
    };
    let both = run(&mut ctx, json!(["feature/b", "feature/a"]));
    let lanes_of = lanes(&both);
    let lane = |id: &str| lanes_of.iter().find(|(known, _)| known == id).unwrap().1;
    // The first pin has the first lane all the way down; the second has
    // the next one down to where the two lines meet.
    let (first, second) = (line("feature/b"), line("feature/a"));
    for id in &first {
        assert_eq!(lane(id), 0, "{id}");
    }
    for id in second.iter().filter(|id| !first.contains(id)) {
        assert_eq!(lane(id), 1, "{id}");
    }
    // A revision that is no branch name pins the same way, and the text
    // drawing follows the lanes.
    let tip = git.run(["rev-parse", "feature/b"]).unwrap();
    let by_id = run(&mut ctx, json!([tip.trim(), "feature/a"]));
    assert_eq!(by_id["rows"], both["rows"]);
    assert_eq!(by_id["ascii"], both["ascii"]);
    let unpinned = run(&mut ctx, json!([]));
    assert_ne!(unpinned["ascii"], both["ascii"]);
}

#[test]
fn hostile_pins_are_refused_and_never_bring_it_down() {
    let (scratch, registry, mut ctx) = open("branches-merges");
    let before = scratch.git().user_refs("%(refname) %(objectname)").unwrap();
    let too_many: Vec<String> = (0..40).map(|n| format!("HEAD~{}", n % 3)).collect();
    let long = "x".repeat(100_000);
    for pinned in [
        json!("main"),
        json!(42),
        json!({ "ids": ["main"] }),
        json!(["no-such-branch"]),
        json!(["--help"]),
        json!(["-q"]),
        json!([""]),
        json!(["main\u{0}x"]),
        json!(["main\nfeature/a"]),
        json!([long]),
        json!([42]),
        json!([null]),
        json!(too_many),
        json!(["main", "main^{tree}"]),
    ] {
        let result = registry.run("repo.graph", &mut ctx, &json!({ "pinned": pinned }));
        assert!(result.is_err(), "{pinned} was taken");
    }
    // What names a commit twice, or one that is outside the walk, is
    // taken and pins what it can.
    let graph = registry
        .run(
            "repo.graph",
            &mut ctx,
            &json!({ "pinned": ["main", "main", "HEAD"], "tips": "head", "limit": 2 }),
        )
        .unwrap();
    assert_eq!(graph["rows"].as_array().unwrap().len(), 2);
    assert_eq!(
        scratch.git().user_refs("%(refname) %(objectname)").unwrap(),
        before
    );
}
