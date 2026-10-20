//! When the program dies, what its commands started dies with it: a push
//! that is waiting for its `pre-push` hook, and whatever the hook
//! started, must not go on for an application that was killed or has
//! crashed. An upload by git-lfs sits exactly there.
//!
//! The program is this binary running `remote.push`; it is killed with
//! the one signal nothing can catch, so nothing in it gets to clean up.
//! What ends the rest is the leash of `crates/git/src/exe/follow.rs`.
//!
//! Sources: githooks(5) (`pre-push`), git-config(1) (`core.hooksPath`),
//! kill(1) (`-KILL`, and `-0`: whether a process is there).
#![cfg(unix)]

use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};

fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < limit, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn pid_in(file: &Path) -> Option<u32> {
    std::fs::read_to_string(file)
        .ok()
        .filter(|text| text.ends_with('\n'))
        .and_then(|text| text.trim().parse().ok())
}

/// A scratch repository with something to push, a bare remote beside it,
/// and a `pre-push` hook that starts something long and waits for it.
/// `before` goes first in the hook.
fn site(before: &str) -> (Scratch, PathBuf, PathBuf) {
    let copy = Scratch::of("linear").unwrap();
    let bare = copy.path().with_extension("origin.git");
    let dir = copy.path().with_extension("scripts");
    for old in [&bare, &dir] {
        let _ = std::fs::remove_dir_all(old);
    }
    std::fs::create_dir_all(dir.join("hooks")).unwrap();
    let git = copy.git();
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run([
        "config",
        "core.hooksPath",
        dir.join("hooks").to_str().unwrap(),
    ])
    .unwrap();
    copy.write("more.txt", "more\n").unwrap();
    git.run(["add", "more.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "something to push"])
        .unwrap();
    let hook = dir.join("hooks/pre-push");
    std::fs::write(
        &hook,
        format!(
            "#!/bin/sh\n{before}\nsleep 600 &\necho $! > '{0}/child'\necho $$ > '{0}/hook'\nwait\n",
            dir.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    (copy, bare, dir)
}

/// The binary pushing, seeing only the fixtures' configuration.
fn pushing(copy: &Scratch) -> std::process::Child {
    let home = fixtures_out_dir().join(".home");
    Command::new(env!("CARGO_BIN_EXE_bitbraid-cli"))
        .current_dir(copy.path())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "BITBRAID_CONFIG_DIR",
            copy.path().with_extension("scripts").join("config"),
        )
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .args(["--repo", copy.path().to_str().unwrap()])
        .args([
            "run",
            "remote.push",
            "--params",
            r#"{"remote":"origin","branch":"main"}"#,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap()
}

fn killed_with_its_push(before: &str, within: Duration) {
    let (copy, bare, dir) = site(before);
    let served = Git::new(&bare, fixtures_out_dir().join(".home"));
    let main = served.rev_parse("refs/heads/main").unwrap();
    let mut program = pushing(&copy);
    wait_for(
        "the hook to start its upload",
        Duration::from_secs(60),
        || pid_in(&dir.join("hook")).is_some() && pid_in(&dir.join("child")).is_some(),
    );
    let (hook, child) = (
        pid_in(&dir.join("hook")).unwrap(),
        pid_in(&dir.join("child")).unwrap(),
    );
    assert!(alive(hook) && alive(child));

    // Dead on the spot: no handler runs, nothing is flushed or closed by it.
    program.kill().unwrap();
    program.wait().unwrap();
    let gone = Instant::now();
    wait_for("the hook and its upload to end too", within, || {
        !alive(hook) && !alive(child)
    });
    let took = gone.elapsed();
    assert_eq!(
        served.rev_parse("refs/heads/main").unwrap(),
        main,
        "nothing was pushed"
    );
    // Whatever a failed run left behind.
    for pid in [hook, child] {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stderr(Stdio::null())
            .status();
    }
    let _ = std::fs::remove_dir_all(&bare);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(took < within);
}

#[test]
fn what_a_push_started_ends_when_the_program_is_killed() {
    killed_with_its_push("", Duration::from_secs(5));
}

#[test]
fn also_what_would_not_end_when_asked() {
    // Asked first, and killed two seconds later.
    killed_with_its_push("trap '' TERM", Duration::from_secs(10));
}
