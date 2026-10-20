//! The watcher reports worktree and reference changes.

use std::{sync::mpsc, time::Duration};

use bitbraid_git::{Area, Watcher};
use bitbraid_testkit::Scratch;

#[test]
fn reports_worktree_and_ref_changes() {
    let scratch = Scratch::of("linear").unwrap();
    let git_dir = scratch.path().join(".git");
    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::spawn(
        &git_dir,
        Some(scratch.path()),
        Duration::from_millis(100),
        move |set| {
            let _ = tx.send(set);
        },
    )
    .unwrap();

    // The watches are put in place in the background; the first set says
    // they stand and names the references, the index, the worktree and the
    // git directory, for whatever went unseen until then.
    let first = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the catch-up change set");
    assert_eq!(
        first.areas,
        [Area::Refs, Area::Index, Area::Worktree, Area::GitDir]
    );
    assert!(first.paths.is_empty());

    scratch.write("watched.txt", "hello\n").unwrap();
    let set = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a worktree change set");
    assert!(set.areas.contains(&Area::Worktree), "{set:?}");
    assert!(
        set.paths.iter().any(|p| p.ends_with("watched.txt")),
        "{set:?}"
    );

    let git = scratch.git();
    git.run(["update-ref", "refs/heads/watched", "HEAD"])
        .unwrap();
    let mut saw_refs = false;
    for _ in 0..5 {
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(set) if set.areas.contains(&Area::Refs) => {
                saw_refs = true;
                break;
            }
            Ok(_) => continue,
            Err(_) => break,
        }
    }
    assert!(saw_refs, "expected a refs change set");
    drop(watcher);
}

/// A repository reached through a link is watched where it is: the events
/// of macOS name the resolved path (its `/tmp` and `/var` are links into
/// `/private`), and they still count as this repository's.
#[test]
fn a_repository_reached_through_a_link_reports_its_changes() {
    let scratch = Scratch::of("linear").unwrap();
    let link = scratch.path().with_extension("link");
    let _ = std::fs::remove_file(&link);
    bitbraid_testkit::symlink_dir(scratch.path(), &link).unwrap();
    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::spawn(
        &link.join(".git"),
        Some(&link),
        Duration::from_millis(100),
        move |set| {
            let _ = tx.send(set);
        },
    )
    .unwrap();
    rx.recv_timeout(Duration::from_secs(5))
        .expect("the catch-up change set");

    scratch.write("through-the-link.txt", "hello\n").unwrap();
    let set = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("a worktree change set");
    assert!(set.areas.contains(&Area::Worktree), "{set:?}");
    assert!(
        set.paths
            .iter()
            .any(|p| p == std::path::Path::new("through-the-link.txt")),
        "{set:?}"
    );
    drop(watcher);
    let _ = std::fs::remove_file(&link);
}

/// Collect change sets until none arrives for `quiet`.
fn drain(
    rx: &mpsc::Receiver<bitbraid_git::ChangeSet>,
    quiet: Duration,
) -> Vec<bitbraid_git::ChangeSet> {
    let mut sets = Vec::new();
    while let Ok(set) = rx.recv_timeout(quiet) {
        sets.push(set);
    }
    sets
}

/// Build output and dependencies are most of many trees. What git ignores
/// is not watched: no watch per directory, no refresh per written file.
/// (Where the whole tree is watched at once, as on Windows and macOS,
/// what is ignored is not reported.)
#[test]
fn ignored_directories_are_left_alone_and_new_ones_are_followed() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch.write(".gitignore", "build/\n").unwrap();
    scratch.write("build/deep/out.o", "x\n").unwrap();
    scratch.write("kept/tracked.txt", "tracked\n").unwrap();
    git.run(["add", ".gitignore", "kept/tracked.txt"]).unwrap();
    // An ignored directory holding a tracked file is still watched, also
    // one further down.
    scratch.write("vendor/forced.txt", "forced\n").unwrap();
    scratch
        .write("deps/vendored/forced.txt", "forced\n")
        .unwrap();
    scratch
        .write(".git/info/exclude", "vendor/\ndeps/vendored/\n")
        .unwrap();
    git.run(["add", "-f", "vendor/forced.txt", "deps/vendored/forced.txt"])
        .unwrap();

    let git_dir = scratch.path().join(".git");
    let (tx, rx) = mpsc::channel();
    let watcher = Watcher::spawn(
        &git_dir,
        Some(scratch.path()),
        Duration::from_millis(100),
        move |set| {
            let _ = tx.send(set);
        },
    )
    .unwrap();
    let first = rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the catch-up change set");
    assert_eq!(first.areas.len(), 4);

    scratch.write("build/deep/out.o", "rebuilt\n").unwrap();
    scratch.write("build/another.o", "new\n").unwrap();
    let sets = drain(&rx, Duration::from_millis(600));
    assert!(sets.is_empty(), "ignored output was reported: {sets:?}");

    scratch.write("kept/tracked.txt", "edited\n").unwrap();
    let sets = drain(&rx, Duration::from_millis(600));
    assert!(
        sets.iter()
            .any(|set| set.paths.iter().any(|p| p.ends_with("kept/tracked.txt"))),
        "{sets:?}"
    );

    for forced in ["vendor/forced.txt", "deps/vendored/forced.txt"] {
        scratch.write(forced, "edited\n").unwrap();
        let sets = drain(&rx, Duration::from_millis(600));
        assert!(
            sets.iter()
                .any(|set| set.paths.iter().any(|p| p.ends_with(forced))),
            "{forced}: {sets:?}"
        );
    }

    // A directory created later is watched from then on.
    std::fs::create_dir_all(scratch.path().join("fresh/inner")).unwrap();
    let sets = drain(&rx, Duration::from_millis(600));
    assert!(
        sets.iter().any(|set| set.areas.contains(&Area::Worktree)),
        "{sets:?}"
    );
    scratch.write("fresh/inner/file.txt", "hello\n").unwrap();
    let sets = drain(&rx, Duration::from_millis(600));
    assert!(
        sets.iter().any(|set| set
            .paths
            .iter()
            .any(|p| p.ends_with("fresh/inner/file.txt"))),
        "{sets:?}"
    );
    drop(watcher);
}
