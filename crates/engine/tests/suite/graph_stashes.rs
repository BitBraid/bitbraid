//! `repo.graph` with a row per stash, against git: each stash
//! of `git stash list` is a row whose one parent is the commit it was made
//! on (`stash@{n}^`), above that commit; without the flag there are none,
//! and the commits of the walk stay as they were.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

fn open(scratch: &Scratch) -> (Registry, CommandContext) {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .expect("open scratch");
    (registry, ctx)
}

fn ids(graph: &Value) -> Vec<String> {
    graph["commits"]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn every_stash_hangs_above_the_commit_it_was_made_on() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    // One stash on the tip, one a commit lower.
    scratch.write("a.txt", "on the tip").unwrap();
    git.run(["stash", "push", "-q", "-m", "on the tip"])
        .unwrap();
    git.run(["checkout", "-q", "HEAD~1"]).unwrap();
    scratch.write("a.txt", "one lower").unwrap();
    git.run(["stash", "push", "-q", "-m", "one lower"]).unwrap();
    git.run(["checkout", "-q", "main"]).unwrap();

    let (registry, mut ctx) = open(&scratch);
    let plain = registry.run("repo.graph", &mut ctx, &json!({})).unwrap();
    let with = registry
        .run("repo.graph", &mut ctx, &json!({ "stashes": true }))
        .unwrap();
    let stashes = git.lines(["rev-parse", "stash@{0}", "stash@{1}"]).unwrap();
    let plain_ids = ids(&plain);
    let with_ids = ids(&with);
    for stash in &stashes {
        assert!(!plain_ids.contains(stash), "{stash} without the flag");
    }
    // The walk's commits are all there, in their order.
    let walked: Vec<&String> = with_ids.iter().filter(|id| !stashes.contains(id)).collect();
    assert_eq!(walked, plain_ids.iter().collect::<Vec<_>>());
    assert_eq!(with["rows"].as_array().unwrap().len(), with_ids.len());

    for (index, stash) in stashes.iter().enumerate() {
        let base = git.rev_parse(&format!("stash@{{{index}}}^")).unwrap();
        let base = base.trim();
        let row = with["commits"]
            .as_array()
            .unwrap()
            .iter()
            .find(|commit| commit["id"] == json!(stash))
            .unwrap_or_else(|| panic!("no row for stash@{{{index}}}"));
        assert_eq!(row["parents"], json!([base]), "stash@{{{index}}}");
        let above = with_ids.iter().position(|id| id == stash).unwrap();
        let below = with_ids.iter().position(|id| id == base).unwrap();
        assert!(above < below, "stash@{{{index}}} is above its commit");
    }
}

#[test]
fn a_repository_without_stashes_has_the_same_graph_either_way() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let (registry, mut ctx) = open(&scratch);
    let plain = registry.run("repo.graph", &mut ctx, &json!({})).unwrap();
    let with = registry
        .run("repo.graph", &mut ctx, &json!({ "stashes": true }))
        .unwrap();
    assert_eq!(plain, with);
}

#[test]
fn a_stash_whose_commit_is_not_walked_is_left_out() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    git.run(["checkout", "-q", "HEAD~3"]).unwrap();
    scratch.write("a.txt", "low").unwrap();
    git.run(["stash", "push", "-q"]).unwrap();
    git.run(["checkout", "-q", "main"]).unwrap();
    let stash = git.rev_parse("stash@{0}").unwrap();
    let (registry, mut ctx) = open(&scratch);
    // Two commits: the stash's is the fourth from the top.
    let graph = registry
        .run(
            "repo.graph",
            &mut ctx,
            &json!({ "stashes": true, "limit": 2 }),
        )
        .unwrap();
    assert!(!ids(&graph).contains(&stash.trim().to_owned()));
}

#[test]
fn the_flag_takes_a_boolean_only() {
    let scratch = Scratch::of("linear").unwrap();
    let (registry, mut ctx) = open(&scratch);
    for hostile in [json!("yes"), json!(1), json!([true]), json!({})] {
        assert!(
            registry
                .run("repo.graph", &mut ctx, &json!({ "stashes": hostile }))
                .is_err(),
            "{hostile}"
        );
    }
}
