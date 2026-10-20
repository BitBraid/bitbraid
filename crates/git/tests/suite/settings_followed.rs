//! What git writes into a repository's configuration while it is open is
//! read at the next look: a remote added or pointed elsewhere by a git
//! child is in the next snapshot. gitoxide reads the files once, so the
//! handle is opened again when they were written; when they were not, it
//! is kept.

use bitbraid_git::{Repository, snapshot};
use bitbraid_testkit::Scratch;

fn remotes(repo: &Repository) -> Vec<(String, Option<String>)> {
    snapshot(repo)
        .unwrap()
        .remotes
        .into_iter()
        .map(|remote| (remote.name, remote.push_url))
        .collect()
}

#[test]
fn a_remote_git_adds_is_in_the_next_snapshot() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let mut repo = Repository::open(copy.path()).unwrap();
    assert!(remotes(&repo).is_empty());
    assert!(!repo.follow_git().unwrap(), "nothing was written");

    git.run([
        "remote",
        "add",
        "second",
        "https://example.invalid/second.git",
    ])
    .unwrap();
    assert!(repo.follow_git().unwrap(), "opened again");
    assert_eq!(remotes(&repo), [("second".to_owned(), None)]);
    assert!(!repo.follow_git().unwrap(), "and kept from then on");

    git.run([
        "remote",
        "set-url",
        "--push",
        "second",
        "https://example.invalid/pushed.git",
    ])
    .unwrap();
    assert!(repo.follow_git().unwrap());
    assert_eq!(
        remotes(&repo),
        [(
            "second".to_owned(),
            Some("https://example.invalid/pushed.git".to_owned())
        )]
    );
}

#[test]
fn a_repository_whose_git_directory_went_is_not_replaced_by_the_one_around_it() {
    // A repository inside another: when the inner one's `.git` is gone,
    // its configuration "changed", and opening again from its folder would
    // find the outer one. The handle keeps the inner repository instead.
    let outer = Scratch::of("linear").unwrap();
    let inner_path = outer.path().join("inner");
    std::fs::create_dir_all(&inner_path).unwrap();
    let init = std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&inner_path)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(init.success());
    let mut repo = Repository::open(&inner_path).unwrap();
    let inner_git = repo.git_dir().to_path_buf();
    std::fs::remove_dir_all(inner_path.join(".git")).unwrap();
    assert!(
        repo.follow_git().is_err(),
        "not opened as another repository"
    );
    assert_eq!(repo.git_dir(), inner_git.as_path());
    assert!(!repo.follow_git().unwrap_or(true), "and not tried again");
}
