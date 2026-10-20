//! The order of the graph and the bounded status
//! through the engine's commands, against git: `repo.log` and `repo.graph`
//! with `order` list the commits as `git rev-list --date-order` and
//! `--topo-order` do, and `repo.status` with `limit` is a part of what
//! `git status` lists.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Scratch, list_scripts};
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

fn ids(commits: &Value) -> Vec<String> {
    commits
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_order_asked_for_is_the_order_git_lists() {
    let mut told_apart = 0;
    for fixture in list_scripts().unwrap() {
        let (scratch, registry, mut ctx) = open(&fixture);
        let git = scratch.git();
        let by_date = git.lines(["rev-list", "--all", "--date-order"]).unwrap();
        let by_topology = git.lines(["rev-list", "--all", "--topo-order"]).unwrap();
        let by_author = git
            .lines(["rev-list", "--all", "--author-date-order"])
            .unwrap();
        told_apart += usize::from(by_date != by_topology);
        for (order, want) in [
            ("date", &by_date),
            ("topo", &by_topology),
            ("author_date", &by_author),
        ] {
            let log = registry
                .run("repo.log", &mut ctx, &json!({ "order": order }))
                .unwrap();
            assert_eq!(&ids(&log), want, "{fixture}: repo.log {order}");
            let graph = registry
                .run("repo.graph", &mut ctx, &json!({ "order": order }))
                .unwrap();
            assert_eq!(
                &ids(&graph["commits"]),
                want,
                "{fixture}: repo.graph {order}"
            );
            // One row of lanes for each commit, in that order.
            assert_eq!(graph["rows"].as_array().unwrap().len(), want.len());
        }
        // Without the word it is by date.
        let plain = registry.run("repo.log", &mut ctx, &json!({})).unwrap();
        assert_eq!(ids(&plain), by_date, "{fixture}");
        // From HEAD alone, first parents only.
        let want = git
            .lines(["rev-list", "HEAD", "--first-parent", "--topo-order"])
            .unwrap();
        let log = registry
            .run(
                "repo.log",
                &mut ctx,
                &json!({ "order": "topo", "tips": "head", "first_parent": true }),
            )
            .unwrap();
        assert_eq!(ids(&log), want, "{fixture}: first parents");
    }
    assert!(told_apart >= 2, "the fixtures tell the two orders apart");
}

#[test]
fn an_order_that_is_none_is_refused() {
    let (scratch, registry, mut ctx) = open("branches-merges");
    let before = scratch.git().user_refs("%(refname) %(objectname)").unwrap();
    for order in [
        json!("sideways"),
        json!(""),
        json!("Topo"),
        json!("date\u{0}"),
        json!(7),
        json!(["topo"]),
        json!({ "topo": true }),
        json!("t".repeat(5000)),
    ] {
        for command in ["repo.log", "repo.graph"] {
            let result = registry.run(command, &mut ctx, &json!({ "order": order }));
            assert!(result.is_err(), "{command} took {order}");
        }
    }
    // With everything else odd too, the order still holds.
    let graph = registry
        .run(
            "repo.graph",
            &mut ctx,
            &json!({ "order": "topo", "limit": 1, "pinned": ["main"], "ascii": true }),
        )
        .unwrap();
    assert_eq!(graph["commits"].as_array().unwrap().len(), 1);
    assert_eq!(
        scratch.git().user_refs("%(refname) %(objectname)").unwrap(),
        before
    );
}

#[test]
fn a_status_with_a_limit_is_part_of_the_whole_and_says_when_there_is_more() {
    let (scratch, registry, mut ctx) = open("branches-merges");
    let git = scratch.git();
    for n in 0..40 {
        scratch
            .write(&format!("pile/new-{n:02}.txt"), "new\n")
            .unwrap();
    }
    scratch.write("a.txt", "changed\n").unwrap();
    let by_git = git.lines(["status", "--porcelain", "-uall"]).unwrap();
    assert_eq!(by_git.len(), 41);

    // Without a limit: the list itself.
    let whole = registry.run("repo.status", &mut ctx, &json!({})).unwrap();
    let whole = whole.as_array().unwrap().clone();
    assert_eq!(whole.len(), by_git.len());
    // A limit that is not reached: all of it, and no more.
    let all = registry
        .run("repo.status", &mut ctx, &json!({ "limit": 41 }))
        .unwrap();
    assert_eq!(all["entries"].as_array().unwrap(), &whole);
    assert_eq!(all["more"], false);
    // One that is: that many of the entries, and the word.
    for limit in [0u64, 1, 7, 40] {
        let cut = registry
            .run("repo.status", &mut ctx, &json!({ "limit": limit }))
            .unwrap();
        let entries = cut["entries"].as_array().unwrap();
        assert_eq!(entries.len() as u64, limit);
        assert_eq!(cut["more"], true, "limit {limit}");
        for entry in entries {
            assert!(whole.contains(entry), "{entry} is not in the whole status");
        }
    }
    // Hostile limits are refused; an enormous one is no limit.
    for limit in [json!(-1), json!("many"), json!(1.5), json!([3]), json!({})] {
        let result = registry.run("repo.status", &mut ctx, &json!({ "limit": limit }));
        assert!(result.is_err(), "{limit} was taken");
    }
    let huge = registry
        .run("repo.status", &mut ctx, &json!({ "limit": u64::MAX }))
        .unwrap();
    assert_eq!(huge["entries"].as_array().unwrap().len(), 41);
    assert_eq!(huge["more"], false);
    // Asking changed nothing.
    assert_eq!(
        git.lines(["status", "--porcelain", "-uall"]).unwrap(),
        by_git
    );
}
