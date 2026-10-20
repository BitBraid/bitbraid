//! Paths that are not UTF-8: a repository whose git directory, or whose
//! worktree, has such a name is still one to work with, and nothing that
//! answers with a path may fall over it. (A path cannot be given that way,
//! parameters being JSON, so the repository is reached through one that
//! is.)
#![cfg(unix)]
#![allow(clippy::print_stderr)]

use std::{ffi::OsString, os::unix::ffi::OsStringExt, path::PathBuf};

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::json;

/// A path beside `scratch` whose name ends in a byte that is no UTF-8.
fn odd_beside(scratch: &Scratch, name: &str) -> PathBuf {
    let mut bytes = scratch.path().as_os_str().to_owned().into_encoded_bytes();
    bytes.extend_from_slice(format!(".odd-{name}-").as_bytes());
    bytes.push(0xff);
    PathBuf::from(OsString::from_vec(bytes))
}

/// Whether the file system holds a name that is not UTF-8. APFS, macOS's,
/// refuses one ("Illegal byte sequence"), and there is nothing to test.
fn odd_names_are_held(scratch: &Scratch, test: &str) -> bool {
    let probe = odd_beside(scratch, "probe");
    match std::fs::create_dir(&probe) {
        Ok(()) => {
            let _ = std::fs::remove_dir(&probe);
            true
        }
        Err(err) => {
            eprintln!("{test}: skipped, this file system takes no such name: {err}");
            false
        }
    }
}

#[test]
fn a_git_directory_with_a_name_that_is_not_utf8_is_worked_with() {
    let scratch = Scratch::of("linear").unwrap();
    if !odd_names_are_held(&scratch, "a git directory not named in UTF-8") {
        return;
    }
    // The repository's own directory moves away; a `.git` file points at it.
    let moved = odd_beside(&scratch, "git");
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(scratch.path().join(".git"), &moved).unwrap();
    let mut pointer = b"gitdir: ".to_vec();
    pointer.extend_from_slice(moved.as_os_str().as_encoded_bytes());
    pointer.push(b'\n');
    std::fs::write(scratch.path().join(".git"), pointer).unwrap();
    assert!(scratch.git().run(["status", "--porcelain"]).is_ok());

    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let summary = registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .expect("the repository opens");
    // The path is shown as well as it can be.
    let shown = summary["git_dir"].as_str().unwrap();
    assert!(
        shown.contains(".odd-git-") && shown.ends_with('\u{fffd}'),
        "{shown}"
    );
    for (command, params) in [
        ("repo.summary", json!({})),
        ("repo.refs", json!({})),
        ("repo.status", json!({})),
        ("worktree.list", json!({})),
        ("hook.list", json!({})),
        ("activity.list", json!({ "questions": true })),
    ] {
        registry
            .run(command, &mut ctx, &params)
            .unwrap_or_else(|err| panic!("{command}: {err}"));
    }
    // The file this writes is inside that directory.
    let out = registry
        .run(
            "ignore.add",
            &mut ctx,
            &json!({ "pattern": "*.log", "target": "local" }),
        )
        .expect("the pattern is added");
    assert!(out["file"].as_str().unwrap().ends_with("info/exclude"));
    assert!(
        std::fs::read_to_string(moved.join("info/exclude"))
            .unwrap()
            .contains("*.log")
    );
    drop(ctx);
    let _ = std::fs::remove_dir_all(&moved);
}

#[test]
fn a_worktree_with_a_name_that_is_not_utf8_is_listed_and_pruned() {
    let scratch = Scratch::of("branches-merges").unwrap();
    if !odd_names_are_held(&scratch, "a worktree not named in UTF-8") {
        return;
    }
    let odd = odd_beside(&scratch, "wt");
    let _ = std::fs::remove_dir_all(&odd);
    scratch
        .git()
        .run([
            OsString::from("worktree"),
            "add".into(),
            "-q".into(),
            odd.clone().into_os_string(),
            "feature/a".into(),
        ])
        .unwrap();

    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .unwrap();
    let listed = registry.run("worktree.list", &mut ctx, &json!({})).unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1]["branch"], "feature/a");
    // What git prints is read as text, so such a path is known only as
    // well as text can hold it: the worktree is listed, and not looked into.
    assert!(listed[1]["changes"].is_null());
    assert!(listed[1]["path"].as_str().unwrap().contains(".odd-wt-"));
    let refs = registry.run("repo.refs", &mut ctx, &json!({})).unwrap();
    assert_eq!(refs["worktrees"].as_array().unwrap().len(), 2);

    // Gone from the disk, it is pruned, and the answer names it.
    std::fs::remove_dir_all(&odd).unwrap();
    let out = registry
        .run("worktree.prune", &mut ctx, &json!({}))
        .unwrap();
    let pruned = out["pruned"].as_array().unwrap();
    assert_eq!(pruned.len(), 1);
    assert!(pruned[0].as_str().unwrap().contains(".odd-wt-"));
    assert_eq!(
        registry
            .run("worktree.list", &mut ctx, &json!({}))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
