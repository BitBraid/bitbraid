//! Where HEAD is and where it was: the branch a detached HEAD can go back
//! to, against git's own `@{-1}`.

use bitbraid_git::{Repository, snapshot};
use bitbraid_testkit::Scratch;

fn head(scratch: &Scratch) -> bitbraid_git::snapshot::Head {
    snapshot(&Repository::open(scratch.path()).unwrap())
        .unwrap()
        .head
}

#[test]
fn a_detached_head_knows_the_branch_it_left() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    // On a branch there is nowhere to go back to.
    let on_branch = head(&scratch);
    assert!(!on_branch.detached);
    assert_eq!(on_branch.previous, None);

    git.run(["switch", "-q", "--detach", "HEAD~1"]).unwrap();
    let detached = head(&scratch);
    assert!(detached.detached);
    assert_eq!(
        detached.previous.as_deref(),
        Some(
            git.run(["rev-parse", "--abbrev-ref", "@{-1}"])
                .unwrap()
                .trim()
        )
    );
    assert_eq!(detached.previous.as_deref(), Some("main"));

    // From one detached commit to another: git's `@{-1}` is the commit
    // left last, which is no branch to go back to.
    git.run(["switch", "-q", "--detach", "HEAD~1"]).unwrap();
    assert_eq!(head(&scratch).previous, None);

    // Through a branch with a slash in its name, then away from it again.
    git.run(["switch", "-q", "-c", "feature/with/slashes"])
        .unwrap();
    git.run(["switch", "-q", "--detach"]).unwrap();
    assert_eq!(
        head(&scratch).previous.as_deref(),
        Some("feature/with/slashes")
    );
    // The branch is gone: nothing to offer.
    git.run(["branch", "-q", "-D", "feature/with/slashes"])
        .unwrap();
    assert_eq!(head(&scratch).previous, None);
}

#[test]
fn a_remote_says_which_branch_its_head_names() {
    // A clone knows the default branch of what it was cloned from.
    let source = Scratch::of("branches-merges").unwrap();
    let clone = source.path().with_extension("clone");
    let _ = std::fs::remove_dir_all(&clone);
    source
        .git()
        .run([
            "clone",
            "-q",
            source.path().to_str().unwrap(),
            clone.to_str().unwrap(),
        ])
        .unwrap();
    let remotes = || {
        snapshot(&Repository::open(&clone).unwrap())
            .unwrap()
            .remotes
    };
    let git = bitbraid_testkit::Git::new(&clone, source.path().with_extension("home"));
    let named = |git: &bitbraid_testkit::Git| {
        git.run(["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
            .ok()
            .and_then(|name| name.trim().strip_prefix("origin/").map(str::to_owned))
    };
    assert_eq!(remotes()[0].head, named(&git));
    assert_eq!(remotes()[0].head.as_deref(), Some("main"));
    // It is no branch of the remote.
    assert!(remotes()[0].branches.iter().all(|b| b.name != "HEAD"));
    // Named otherwise, and not at all.
    git.run(["remote", "set-head", "origin", "feature/a"])
        .unwrap();
    assert_eq!(remotes()[0].head, named(&git));
    assert_eq!(remotes()[0].head.as_deref(), Some("feature/a"));
    git.run(["remote", "set-head", "origin", "--delete"])
        .unwrap();
    assert_eq!(remotes()[0].head, None);
    let _ = std::fs::remove_dir_all(&clone);
    let _ = std::fs::remove_dir_all(source.path().with_extension("home"));
}

#[test]
fn a_reflog_that_is_missing_or_odd_is_no_reason_to_fail() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    git.run(["switch", "-q", "--detach"]).unwrap();
    let log = scratch.path().join(".git/logs/HEAD");
    // Lines that are not reflog lines, and a checkout message cut short.
    std::fs::write(
        &log,
        "garbage\n\n0000 1111 nobody <x> 0 +0000\tcheckout: moving from \n",
    )
    .unwrap();
    assert!(head(&scratch).detached);
    let _ = head(&scratch).previous;
    std::fs::remove_file(&log).unwrap();
    assert_eq!(head(&scratch).previous, None);
    std::fs::write(&log, vec![0xffu8; 4096]).unwrap();
    assert_eq!(head(&scratch).previous, None);
}
