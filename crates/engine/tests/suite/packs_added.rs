//! A repository that stays open in a context (a tab's session, the
//! operations thread) keeps answering while git adds packs to it: more of
//! them than its handle has room for, and the handle is opened again and
//! the read asked once more, instead of the files of a commit failing with
//! "too few index slots" for as long as the tab is open.

use std::path::Path;

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::json;

/// `count` packs more: copies of a pack that is there, each under a name
/// of its own, as many small fetches leave them.
fn more_packs(repo: &Path, count: usize) {
    let dir = repo.join(".git/objects/pack");
    let pack = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "pack"))
        .expect("a pack to copy");
    for n in 0..count {
        let name = format!("pack-{:040x}", 0x00fe_ed00_0000_usize.saturating_add(n));
        std::fs::copy(&pack, dir.join(format!("{name}.pack"))).unwrap();
        std::fs::copy(pack.with_extension("idx"), dir.join(format!("{name}.idx"))).unwrap();
    }
}

#[test]
fn a_read_after_more_packs_than_room_opens_the_repository_again() {
    let copy = Scratch::of("linear").unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    // A commit made from outside, in a pack of its own, and a thousand
    // and more packs beside it.
    let git = copy.git();
    copy.write("added/later.txt", "made while the repository was open\n")
        .unwrap();
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "made from outside"])
        .unwrap();
    git.run(["repack", "-d", "-q"]).unwrap();
    more_packs(copy.path(), 1100);
    let diff = registry
        .run(
            "repo.diff",
            &mut ctx,
            &json!({ "old": "HEAD~1", "new": "HEAD" }),
        )
        .unwrap();
    let paths: Vec<&str> = diff
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|file| file["path"].as_str())
        .collect();
    assert_eq!(paths, ["added/later.txt"]);
    // And from then on without opening it again.
    registry
        .run(
            "repo.diff",
            &mut ctx,
            &json!({ "old": "HEAD~2", "new": "HEAD" }),
        )
        .unwrap();
}
