//! Where a walk of the whole history starts: the snapshot reads it with the
//! references (`Snapshot::walk_tips`), so that a graph laid out from a
//! snapshot does not read every reference a second time, and it has to be
//! what the walk finds by itself (`log::resolve_tips` with every
//! reference), commit for commit and in the same order: the order of the
//! starting points decides between commits of the same date.
//!
//! On every fixture, and on a repository with what could make the two
//! part: remotes whose names nest (`a` and `a/b`), references under
//! `refs/remotes/` of no remote, a remote's symbolic `HEAD`, a symbolic
//! branch, tags of a tree and of a blob, annotated and lightweight tags,
//! references packed and loose, and a detached HEAD.

use bitbraid_git::{Repository, Tips, log::resolve_tips, snapshot};
use bitbraid_testkit::{Scratch, list_scripts};

fn starts_as_the_walk_does(scratch: &Scratch) -> bitbraid_git::snapshot::Snapshot {
    let repo = Repository::open(scratch.path()).unwrap();
    let snap = snapshot(&repo).unwrap();
    let walk = resolve_tips(&repo, &Tips::All).unwrap();
    assert_eq!(snap.walk_tips, walk, "{}", scratch.path().display());
    snap
}

#[test]
fn the_snapshot_starts_a_walk_of_everything_where_the_walk_would() {
    for name in list_scripts().unwrap() {
        let Ok(scratch) = Scratch::of(&name) else {
            continue;
        };
        starts_as_the_walk_does(&scratch);
    }

    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    // `git remote add` refuses a name inside another's; written by hand,
    // the configuration has them both.
    for (name, url) in [
        ("a", "https://example.invalid/a.git"),
        ("a/b", "https://example.invalid/b.git"),
    ] {
        git.run(["config", &format!("remote.{name}.url"), url])
            .unwrap();
    }
    git.run(["update-ref", "refs/remotes/a/x", "HEAD~1"])
        .unwrap();
    git.run(["update-ref", "refs/remotes/a/b/y", "HEAD~2"])
        .unwrap();
    git.run(["update-ref", "refs/remotes/gone/z", "HEAD"])
        .unwrap();
    git.run(["symbolic-ref", "refs/remotes/a/HEAD", "refs/remotes/a/x"])
        .unwrap();
    git.run(["symbolic-ref", "refs/heads/alias", "refs/heads/main"])
        .unwrap();
    git.run(["tag", "of-a-tree", "HEAD^{tree}"]).unwrap();
    let blob = git
        .run(["hash-object", "-w", "--stdin"])
        .unwrap_or_default();
    let blob = blob.trim();
    if !blob.is_empty() {
        git.run(["tag", "of-a-blob", blob]).unwrap();
    }
    git.run(["tag", "-a", "-m", "annotated", "annotated", "HEAD~2"])
        .unwrap();
    git.run(["tag", "light", "HEAD~2"]).unwrap();
    git.run(["pack-refs", "--all"]).unwrap();
    // Loose ones beside the packed, one of them in place of a packed one.
    git.run(["update-ref", "refs/remotes/a/x", "HEAD~2"])
        .unwrap();
    git.run(["update-ref", "refs/remotes/a/later", "HEAD"])
        .unwrap();
    git.run(["tag", "-a", "-m", "loose", "loose-annotated", "HEAD~1"])
        .unwrap();
    git.run(["checkout", "-q", "--detach", "HEAD~1"]).unwrap();

    let snap = starts_as_the_walk_does(&scratch);
    // Each remote has every reference in its folder, a nested remote's
    // too: `refs/remotes/a/b/y` is `b/y` of `a` and `y` of `a/b`.
    let a = snap
        .remotes
        .iter()
        .find(|remote| remote.name == "a")
        .unwrap();
    let names: Vec<&str> = a
        .branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect();
    assert_eq!(names, ["b/y", "later", "x"]);
    assert_eq!(a.head.as_deref(), Some("x"));
    let ab = snap
        .remotes
        .iter()
        .find(|remote| remote.name == "a/b")
        .unwrap();
    let names: Vec<&str> = ab
        .branches
        .iter()
        .map(|branch| branch.name.as_str())
        .collect();
    assert_eq!(names, ["y"]);
    assert_eq!(ab.head, None);
    assert!(!snap.remotes.iter().any(|remote| remote.name == "gone"));
    // HEAD first; the tags of a tree and a blob are not where a walk starts.
    assert_eq!(snap.walk_tips.first().copied(), snap.head.target);
    for name in ["of-a-tree", "of-a-blob"] {
        if let Some(tag) = snap.tags.iter().find(|tag| tag.name == name) {
            assert!(!snap.walk_tips.contains(&tag.target), "{name}");
        }
    }
}
