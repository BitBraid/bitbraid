//! `git.info` against `git --version`, and `git.use`: every git child the
//! engine starts afterwards is the chosen executable.
//!
//! The choice is one for the whole process, so everything here is one test,
//! and this file is a test binary of its own.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Scratch, scratch_root};
use serde_json::{Value, json};

struct Engine {
    registry: Registry,
    ctx: CommandContext,
}

impl Engine {
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }
}

#[cfg(unix)]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt as _;
    let path = dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    // A script that was just written can be "busy" for a moment, while a
    // child forked meanwhile still holds it open for writing.
    for _ in 0..200 {
        match std::process::Command::new(&path)
            .arg("--version")
            .env("BITBRAID_TEST_SETTLING", "1")
            .output()
        {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                std::thread::sleep(std::time::Duration::from_millis(10))
            }
            _ => break,
        }
    }
    path
}

/// Elsewhere (Windows) what is handed out is the batch file that runs the
/// script with Git's `sh`.
#[cfg(not(unix))]
fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
    bitbraid_testkit::script(dir, name, &format!("#!/bin/sh\n{body}\n")).unwrap()
}

/// The subcommands a log of invocations holds, one line each.
fn logged(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_chosen_executable_runs_every_git_child_and_hostile_choices_change_nothing() {
    let dir = scratch_root().join(format!("git-executable-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let dir = bitbraid_core::paths::canonical(dir).unwrap();
    let mut engine = Engine {
        registry: builtin_registry(),
        ctx: CommandContext::default(),
    };

    // Asked, it says what `git --version` says.
    let oracle = bitbraid_testkit::oracle::git_version().unwrap();
    let info = engine.run("git.info", json!({}));
    assert_eq!(info["version_text"], oracle.trim());
    assert_eq!(info["chosen"], false);
    assert_eq!(info["supported"], true, "the tests need a current git");
    assert!(info["lacks"].as_array().unwrap().is_empty());
    assert!(Path::new(info["path"].as_str().unwrap()).is_file());
    let real = info["path"].as_str().unwrap().to_owned();
    let numbers = format!(
        "{}.{}.{}",
        info["version"]["major"], info["version"]["minor"], info["version"]["patch"]
    );
    assert!(oracle.contains(&numbers), "{oracle} / {numbers}");

    // A wrapper that notes what it was started with and then is git.
    let log = dir.join("invocations.log");
    let wrapper = script(
        &dir,
        "wrapped-git",
        &format!(
            "[ -n \"$BITBRAID_TEST_SETTLING\" ] || printf '%s\\n' \"$1\" >> '{}'\nexec '{real}' \"$@\"",
            log.display()
        ),
    );

    // Hostile choices are refused, and the git in use stays the one it was.
    let other = script(&dir, "not-git", "echo 'something else 1.0'");
    for hostile in [
        json!({ "program": "/definitely/not/there/git" }),
        json!({ "program": dir }),
        json!({ "program": other }),
        json!({ "program": "a\u{0}b" }),
        json!({ "program": "git\n--exec-path=/tmp" }),
        json!({ "program": "x".repeat(10_000) }),
        json!({ "program": 7 }),
        json!({ "program": ["git"] }),
        json!({ "program": "git", "verify": "yes" }),
    ] {
        assert!(
            engine.try_run("git.use", hostile.clone()).is_err(),
            "{hostile} should be refused"
        );
        assert!(
            engine.try_run("git.info", hostile.clone()).is_err() || hostile["verify"].is_string(),
            "{hostile} should not be described"
        );
        assert_eq!(engine.run("git.info", json!({}))["chosen"], false);
        assert!(logged(&log).is_empty());
    }
    // Another executable can be asked about without switching to it.
    let asked = engine.run("git.info", json!({ "program": wrapper }));
    assert_eq!(asked["version_text"], oracle.trim());
    assert_eq!(asked["chosen"], false);
    assert_eq!(logged(&log), ["--version"]);
    std::fs::remove_file(&log).unwrap();

    // Chosen: from now on it is what runs.
    let out = engine.run("git.use", json!({ "program": wrapper }));
    assert_eq!(out["program"], wrapper.to_str().unwrap());
    assert_eq!(out["info"]["version_text"], oracle.trim());
    let info = engine.run("git.info", json!({}));
    assert_eq!(info["chosen"], true);
    assert_eq!(info["path"], wrapper.to_str().unwrap());
    std::fs::remove_file(&log).unwrap();

    let copy = Scratch::of("branches-merges").unwrap();
    let git = copy.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    engine.run("repo.open", json!({ "path": copy.path() }));
    // One of each way git is started: in a repository, where there is none
    // yet, while it reports, and by the large-file commands.
    engine.run("worktree.list", json!({}));
    copy.write("notes-of-the-test.txt", "one\n").unwrap();
    engine.run("stage.paths", json!({ "paths": ["notes-of-the-test.txt"] }));
    engine.run(
        "commit.create",
        json!({ "message": "a commit by the wrapper" }),
    );
    let made = dir.join("made");
    engine.run("repo.init", json!({ "path": made }));
    let cloned = dir.join("cloned");
    engine.run("repo.clone", json!({ "url": copy.path(), "path": cloned }));
    let _ = engine.try_run("lfs.status", json!({}));
    let seen = logged(&log);
    for subcommand in ["worktree", "commit", "init", "clone"] {
        assert!(
            seen.iter().any(|line| line == subcommand),
            "{subcommand} did not go through the chosen executable: {seen:?}"
        );
    }
    assert_eq!(
        git.run(["log", "-1", "--format=%s"]).unwrap(),
        "a commit by the wrapper\n"
    );

    // Unverified, as at start-up: taken as it is, and found out when used.
    let out = engine.run(
        "git.use",
        json!({ "program": "/definitely/not/there/git", "verify": false }),
    );
    assert!(out["info"].is_null());
    assert!(engine.try_run("worktree.list", json!({})).is_err());
    assert!(engine.try_run("git.info", json!({})).is_err());

    // And back to the default.
    let before = logged(&log).len();
    let out = engine.run("git.use", json!({ "program": "  " }));
    assert!(out["program"].is_null());
    assert_eq!(out["info"]["version_text"], oracle.trim());
    engine.run("worktree.list", json!({}));
    assert_eq!(logged(&log).len(), before, "the wrapper is out of use");
    assert_eq!(engine.run("git.info", json!({}))["chosen"], false);
    let out = engine.run("git.use", json!({ "program": wrapper }));
    assert_eq!(out["program"], wrapper.to_str().unwrap());
    engine.run("git.use", json!({}));
    assert_eq!(engine.run("git.info", json!({}))["chosen"], false);

    drop(copy);
    let _ = std::fs::remove_dir_all(&dir);
}
