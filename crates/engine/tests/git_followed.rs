//! A repository that is open when another git executable is chosen
//! follows it: the next command reads with that git's system
//! configuration, as the git children it starts do.
//!
//! What is looked at is the status, which gitoxide computes, and in it the
//! files a system-wide ignore file leaves out: each of two gits has a
//! system configuration that names an ignore file of its own. The same
//! status is asked of the git itself each time.
//!
//! The choice is one for the whole process, so this is one test in a test
//! binary of its own.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

/// A git installed elsewhere: the real one with a system configuration of
/// its own, which it reads although the tests keep git children from the
/// machine's (`GIT_CONFIG_NOSYSTEM`).
fn installed_git(dir: &Path, name: &str, system: &Path) -> PathBuf {
    bitbraid_testkit::script(
        dir,
        name,
        &format!(
            "#!/bin/sh\nunset GIT_CONFIG_NOSYSTEM\nGIT_CONFIG_SYSTEM='{}' exec git \"$@\"\n",
            system.display()
        ),
    )
    .unwrap()
}

/// The paths `repo.status` lists, through a context that stays open.
fn listed(registry: &Registry, ctx: &mut CommandContext) -> BTreeSet<String> {
    let status = registry.run("repo.status", ctx, &json!({})).unwrap();
    let entries = status
        .get("entries")
        .and_then(Value::as_array)
        .or_else(|| status.as_array())
        .unwrap_or_else(|| panic!("no entries in {status}"));
    entries
        .iter()
        .map(|entry| entry["path"].as_str().unwrap().to_owned())
        .collect()
}

/// The paths `git status` lists, asked of `program` in `repo`.
fn by_git(program: &Path, repo: &Path) -> BTreeSet<String> {
    let out = Command::new(program)
        .args(["status", "--porcelain", "--untracked-files=all"])
        .current_dir(repo)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout)
        .unwrap()
        .lines()
        .map(|line| line[3..].to_owned())
        .collect()
}

fn names(paths: &[&str]) -> BTreeSet<String> {
    paths.iter().map(|path| (*path).to_owned()).collect()
}

#[test]
fn an_open_repository_follows_the_git_that_is_chosen() {
    let scratch = Scratch::of("linear").unwrap();
    let repo = scratch.path().to_path_buf();
    let tools = repo.join(".git").join("bitbraid-test-tools");
    fs::create_dir_all(&tools).unwrap();
    // Two files nobody tracks, of kinds no ignore file of a developer's
    // own would name.
    fs::write(repo.join("left.bitbraid-first-kind"), "x").unwrap();
    fs::write(repo.join("left.bitbraid-second-kind"), "x").unwrap();
    // Each git ignores one of the kinds, system-wide.
    let mut gits = Vec::new();
    for kind in ["first", "second"] {
        let ignore = tools.join(format!("{kind}.ignore"));
        fs::write(&ignore, format!("*.bitbraid-{kind}-kind\n")).unwrap();
        let system = tools.join(format!("{kind}.gitconfig"));
        // A backslash in a value of git's configuration begins an escape:
        // a path of Windows is written with `/` there.
        fs::write(
            &system,
            format!(
                "[core]\n\texcludesFile = {}\n",
                ignore.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        gits.push(installed_git(&tools, &format!("{kind}-git"), &system));
    }
    let (first, second) = (gits[0].clone(), gits[1].clone());
    let plain = PathBuf::from("git");

    let registry = builtin_registry();
    // The context of a repository that stays open, as a tab's is.
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": repo }))
        .unwrap();
    // The preferences choose a git through a context of their own.
    let choose = |program: Option<&Path>| {
        let mut standalone = CommandContext::default();
        registry
            .run(
                "git.use",
                &mut standalone,
                &json!({ "program": program.map(|program| program.to_string_lossy()) }),
            )
            .unwrap();
    };

    let both = names(&["left.bitbraid-first-kind", "left.bitbraid-second-kind"]);
    assert_eq!(listed(&registry, &mut ctx), both);
    assert_eq!(by_git(&plain, &repo), both);

    // The first git is chosen while the repository is open: its next
    // status leaves out what that git ignores, as that git's own does.
    choose(Some(&first));
    let under_first = names(&["left.bitbraid-second-kind"]);
    assert_eq!(by_git(&first, &repo), under_first);
    assert_eq!(listed(&registry, &mut ctx), under_first);
    assert!(ctx.repo.as_ref().unwrap().follows_git());

    // And the second, the other way round.
    choose(Some(&second));
    let under_second = names(&["left.bitbraid-first-kind"]);
    assert_eq!(by_git(&second, &repo), under_second);
    assert_eq!(listed(&registry, &mut ctx), under_second);

    // Chosen again, nothing is opened anew: the handle is the same one.
    let before = ctx.repo.as_ref().unwrap().git_dir().to_path_buf();
    choose(Some(&second));
    assert!(ctx.repo.as_ref().unwrap().follows_git());
    assert_eq!(listed(&registry, &mut ctx), under_second);
    assert_eq!(ctx.repo.as_ref().unwrap().git_dir(), before);

    // Back to the default: both are listed again.
    choose(None);
    assert_eq!(listed(&registry, &mut ctx), both);
    assert_eq!(by_git(&plain, &repo), both);

    // A context without a repository has nothing to follow, and a choice
    // that is no git at all takes nothing down.
    let mut none = CommandContext::default();
    none.follow_git();
    exe_is(&registry, "/bin/false");
    assert!(registry.run("repo.status", &mut ctx, &json!({})).is_ok());
    choose(None);
}

/// Choose an executable without asking it what it is.
fn exe_is(registry: &Registry, program: &str) {
    let mut standalone = CommandContext::default();
    registry
        .run(
            "git.use",
            &mut standalone,
            &json!({ "program": program, "verify": false }),
        )
        .unwrap();
}
