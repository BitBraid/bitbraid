//! A shallow clone's commits as git has them: a commit at the edge, whose
//! parents were left on the remote, is a root to git. It lists no parent
//! and every file of it as added, here as well.

use bitbraid_git::{
    DiffOptions, Oid, Repository, Whitespace, commit_files, details, diff::FileChangeKind, parents,
};
use bitbraid_testkit::{Git, Scratch};

fn options() -> DiffOptions {
    DiffOptions {
        context: 3,
        paths: Vec::new(),
        detect_renames: true,
        whitespace: Whitespace::Exact,
        max_bytes: None,
    }
}

/// `git show --name-status` of a commit, as `A\tpath` lines.
fn gits_files(git: &Git, id: &str) -> Vec<String> {
    git.lines(["show", "--name-status", "--no-renames", "--format=", id])
        .unwrap()
        .into_iter()
        .filter(|line| !line.is_empty())
        .collect()
}

fn ours(repo: &Repository, id: Oid) -> Vec<String> {
    commit_files(repo, id, &options())
        .unwrap()
        .iter()
        .map(|file| {
            let letter = match file.kind {
                FileChangeKind::Added => "A",
                FileChangeKind::Deleted => "D",
                FileChangeKind::Modified => "M",
                other => panic!("unexpected {other:?} for {}", file.path),
            };
            format!("{letter}\t{}", file.path)
        })
        .collect()
}

#[test]
fn the_edge_of_a_shallow_clone_is_a_root_as_git_has_it() {
    let scratch = Scratch::of("renames").unwrap();
    let git = scratch.git();
    let url = format!("file://{}", scratch.path().display());
    for depth in ["1", "2"] {
        let clone = scratch.path().join(".git").join(format!("shallow-{depth}"));
        git.run([
            "clone",
            "-q",
            &format!("--depth={depth}"),
            url.as_str(),
            clone.to_string_lossy().as_ref(),
        ])
        .unwrap();
        let shallow = Git::new(&clone, git.home());
        let repo = Repository::open(&clone).unwrap();
        let ids = shallow.lines(["rev-list", "HEAD"]).unwrap();
        assert_eq!(ids.len().to_string(), depth);
        for id in &ids {
            let oid: Oid = id.parse().unwrap();
            let want_parents: Vec<String> = shallow
                .run(["log", "-1", "--format=%P", id])
                .unwrap()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let got: Vec<String> = parents(&repo, oid).iter().map(Oid::to_hex).collect();
            assert_eq!(got, want_parents, "parents of {id} at depth {depth}");
            let detailed: Vec<String> = details(&repo, &[oid]).unwrap()[0]
                .parents
                .iter()
                .map(Oid::to_hex)
                .collect();
            assert_eq!(detailed, want_parents, "details of {id} at depth {depth}");
            let want = gits_files(&shallow, id);
            assert!(!want.is_empty(), "{id} changes something");
            assert_eq!(ours(&repo, oid), want, "files of {id} at depth {depth}");
        }
        // The edge itself: every file of its tree is added.
        let edge = ids.last().unwrap();
        assert!(
            gits_files(&shallow, edge)
                .iter()
                .all(|l| l.starts_with("A\t"))
        );
    }
}
