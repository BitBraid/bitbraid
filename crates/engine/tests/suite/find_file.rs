//! `search.files` against `git ls-tree`: the files it finds
//! are the files of the tree whose path holds the query's letters in
//! order, and the best of them come first.

use std::collections::BTreeSet;

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

fn paths(found: &Value) -> Vec<String> {
    found["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap().to_owned())
        .collect()
}

/// Whether `path` holds the letters of `query` in order, as the test
/// itself reads it: without case, blanks left out.
fn holds(path: &str, query: &str) -> bool {
    let mut letters = path.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .all(|want| letters.any(|have| have == want))
}

#[test]
fn the_files_found_are_the_files_git_lists_that_hold_the_letters() {
    let mut checked = 0;
    for fixture in list_scripts().unwrap() {
        let (scratch, registry, mut ctx) = open(&fixture);
        let git = scratch.git();
        for rev in ["HEAD", "HEAD~1"] {
            let Ok(listed) = git.lines(["ls-tree", "-r", "--name-only", rev]) else {
                continue; // a history of one commit has no HEAD~1
            };
            let listed: BTreeSet<String> = listed.into_iter().collect();
            for query in ["", "a", "rs", "txt", "src", "md", "E", "s r", "zzzz", "/"] {
                let found = registry
                    .run(
                        "search.files",
                        &mut ctx,
                        &json!({ "query": query, "rev": rev, "limit": 1000 }),
                    )
                    .unwrap();
                let want: BTreeSet<String> = listed
                    .iter()
                    .filter(|path| holds(path, query))
                    .cloned()
                    .collect();
                let got: BTreeSet<String> = paths(&found).into_iter().collect();
                assert_eq!(got, want, "{fixture} {rev} {query:?}");
                assert_eq!(found["matched"], want.len(), "{fixture} {rev} {query:?}");
                assert_eq!(found["scanned"], listed.len(), "{fixture} {rev}");
                assert_eq!(found["more"], false);
                // The best first: no score rises down the list.
                let scores: Vec<i64> = found["files"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|file| file["score"].as_i64().unwrap())
                    .collect();
                assert!(scores.windows(2).all(|pair| pair[0] >= pair[1]));
                checked += 1;
            }
        }
        // Without a revision it is HEAD.
        let head = registry
            .run("search.files", &mut ctx, &json!({ "query": "" }))
            .unwrap();
        let listed = git.lines(["ls-tree", "-r", "--name-only", "HEAD"]).unwrap();
        assert_eq!(head["scanned"], listed.len(), "{fixture}");
    }
    assert!(checked > 60, "{checked}");
}

#[test]
fn the_file_itself_comes_before_files_in_a_folder_of_that_name() {
    let (scratch, registry, mut ctx) = open("search");
    let git = scratch.git();
    for (path, content) in [
        ("parser/notes.txt", "notes\n"),
        ("src/deep/down/parser.rs", "fn parse() {}\n"),
        ("docs/par_ser.md", "words\n"),
        ("sub dir/file with spaces.txt", "spaces\n"),
        ("unicode/résumé.md", "é\n"),
    ] {
        scratch.write(path, content).unwrap();
    }
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "more files to find"])
        .unwrap();
    let mut find = |query: &str, limit: Value| -> Value {
        registry
            .run(
                "search.files",
                &mut ctx,
                &json!({ "query": query, "limit": limit }),
            )
            .unwrap()
    };
    let found = paths(&find("parser", json!(50)));
    assert_eq!(found[0], "src/deep/down/parser.rs", "{found:?}");
    assert!(found.contains(&"parser/notes.txt".to_owned()));
    assert!(found.contains(&"docs/par_ser.md".to_owned()));
    // A limit cuts the list and not the count.
    let cut = find("", json!(2));
    assert_eq!(paths(&cut).len(), 2);
    assert!(cut["matched"].as_u64().unwrap() > 2);
    assert_eq!(paths(&find("", json!(0))).len(), 0);
    // Blanks in the query are no letters; a path with blanks is found.
    assert_eq!(
        paths(&find("file with", json!(5)))[0],
        "sub dir/file with spaces.txt"
    );
    assert_eq!(paths(&find("RESUME", json!(5))).len(), 0, "é is not e");
    assert_eq!(paths(&find("résumé", json!(5))), ["unicode/résumé.md"]);
    // The worktree is not what is searched: a file that is not committed
    // is not found, and nothing was changed by looking.
    scratch.write("not-committed.txt", "x\n").unwrap();
    assert!(paths(&find("not-committed", json!(5))).is_empty());
    assert_eq!(
        git.run(["status", "--porcelain"]).unwrap(),
        "?? not-committed.txt\n"
    );
}

#[test]
fn hostile_parameters_are_refused_and_find_nothing() {
    let (scratch, registry, mut ctx) = open("search");
    let before = scratch.git().user_refs("%(refname) %(objectname)").unwrap();
    for params in [
        json!({}),
        json!({ "query": 7 }),
        json!({ "query": ["a"] }),
        json!({ "query": "a\nb" }),
        json!({ "query": "nul\u{0}" }),
        json!({ "query": "q".repeat(300) }),
        json!({ "query": "a", "rev": "--output=pwned" }),
        json!({ "query": "a", "rev": "no-such-branch" }),
        json!({ "query": "a", "rev": "" }),
        json!({ "query": "a", "rev": "index" }),
        json!({ "query": "a", "rev": "worktree" }),
        json!({ "query": "a", "limit": -1 }),
        json!({ "query": "a", "limit": "many" }),
    ] {
        let result = registry.run("search.files", &mut ctx, &params);
        assert!(result.is_err(), "{params} was taken");
    }
    // What is odd and harmless is taken: an enormous limit is the most
    // that is ever returned, a tree by its own name is a tree.
    let all = registry
        .run(
            "search.files",
            &mut ctx,
            &json!({ "query": "", "limit": u64::MAX }),
        )
        .unwrap();
    assert!(!paths(&all).is_empty());
    let tree = registry
        .run(
            "search.files",
            &mut ctx,
            &json!({ "query": "", "rev": "HEAD^{tree}" }),
        )
        .unwrap();
    assert_eq!(tree["scanned"], all["scanned"]);
    // Stopped before it starts.
    ctx.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(
        registry
            .run("search.files", &mut ctx, &json!({ "query": "a" }))
            .is_err()
    );
    ctx.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(!scratch.path().join("pwned").exists());
    assert_eq!(
        scratch.git().user_refs("%(refname) %(objectname)").unwrap(),
        before
    );
}

#[test]
fn a_repository_without_commits_has_no_files_to_find() {
    let (scratch, registry, mut ctx) = open("linear");
    let git = scratch.git();
    git.run(["switch", "-q", "--orphan", "empty"]).unwrap();
    let found = registry
        .run("search.files", &mut ctx, &json!({ "query": "" }))
        .unwrap();
    assert_eq!(found["scanned"], 0);
    assert!(paths(&found).is_empty());
}
