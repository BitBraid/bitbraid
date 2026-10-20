//! The remotes' addresses read without the references
//! (`remote_fetch_urls`), against git and against the snapshot, which
//! reads them with everything else.

use bitbraid_git::{Repository, remote_fetch_urls, snapshot};
use bitbraid_testkit::Scratch;

#[test]
fn the_remotes_addresses_are_git_s_and_the_snapshot_s() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    git.run(["remote", "add", "origin", "https://example.invalid/one.git"])
        .unwrap();
    git.run([
        "remote",
        "add",
        "upstream",
        "https://example.invalid/two.git",
    ])
    .unwrap();
    // Pushed somewhere else: the fetch address is still the one asked for.
    git.run([
        "remote",
        "set-url",
        "--push",
        "upstream",
        "git@example.invalid:two.git",
    ])
    .unwrap();
    // A remote with no address at all, which git lists all the same.
    git.run([
        "config",
        "remote.bare.fetch",
        "+refs/heads/*:refs/remotes/bare/*",
    ])
    .unwrap();
    // A reference of a remote changes nothing here.
    let head = git.rev_parse("HEAD").unwrap();
    git.run(["update-ref", "refs/remotes/origin/main", head.as_str()])
        .unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let got = remote_fetch_urls(&repo);

    let mut want: Vec<(String, Option<String>)> = git
        .lines(["remote"])
        .unwrap()
        .into_iter()
        .map(|name| {
            // As configured: `git remote get-url` would give a remote
            // without an address its own name.
            let url = git
                .run(["config", "--get", &format!("remote.{name}.url")])
                .ok()
                .map(|url| url.trim().to_owned())
                .filter(|url| !url.is_empty());
            (name, url)
        })
        .collect();
    want.sort();
    assert_eq!(got, want);
    assert_eq!(
        got.iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["bare", "origin", "upstream"]
    );

    let from_snapshot: Vec<(String, Option<String>)> = snapshot(&repo)
        .unwrap()
        .remotes
        .into_iter()
        .map(|remote| (remote.name, remote.fetch_url))
        .collect();
    assert_eq!(got, from_snapshot);

    // None at all.
    let plain = Scratch::of("linear").unwrap();
    let repo = Repository::open(plain.path()).unwrap();
    assert!(remote_fetch_urls(&repo).is_empty());
}
