//! Stopping a fetch or a push ends everything it started: git, the hook
//! git ran, and what the hook ran, which is where an upload by git-lfs
//! sits. Killing git alone would leave the rest running, and the command
//! waiting for it.
//!
//! The hook and the transport are scripts of the test's own, set in the
//! scratch repository's configuration (`core.hooksPath`,
//! `core.sshCommand`); they write down their process numbers and then
//! wait far longer than any test.
//!
//! Sources: githooks(5) (`pre-push`), git-config(1) (`core.hooksPath`,
//! `core.sshCommand`), kill(1) (`-0`: whether a process is there).
#![cfg(unix)]

use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

fn script(path: &Path, body: &str) {
    std::fs::write(path, body).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

/// Whether a process with this number is there.
fn alive(pid: u32) -> bool {
    Command::new("kill")
        .args(["-0", &pid.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) -> Duration {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < limit, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
    started.elapsed()
}

/// The number a script wrote into a file, once it is there whole.
fn pid_in(file: &Path) -> Option<u32> {
    std::fs::read_to_string(file)
        .ok()
        .filter(|text| text.ends_with('\n'))
        .and_then(|text| text.trim().parse().ok())
}

/// A scratch repository with a bare remote beside it and a directory for
/// the scripts and what they write.
struct Site {
    copy: Scratch,
    bare: PathBuf,
    dir: PathBuf,
}

impl Site {
    fn new() -> Self {
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
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("core.hooksPath", dir.join("hooks").to_str().unwrap()),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        copy.write("more.txt", "more\n").unwrap();
        git.run(["add", "more.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "something to push"])
            .unwrap();
        Self { copy, bare, dir }
    }

    fn remote_main(&self) -> String {
        Git::new(&self.bare, fixtures_out_dir().join(".home"))
            .rev_parse("refs/heads/main")
            .unwrap()
    }

    /// Run a command, set the cancel flag once `started` says so, and
    /// return what the command ended with and how long after the flag.
    fn stopped(
        &self,
        id: &str,
        params: Value,
        started: impl Fn() -> bool,
    ) -> (Result<Value, String>, Duration) {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": self.copy.path() }))
            .expect("open scratch");
        let cancel = ctx.cancel.clone();
        let id = id.to_owned();
        let running = std::thread::spawn(move || {
            registry
                .run(&id, &mut ctx, &params)
                .map_err(|err| err.to_string())
        });
        wait_for("the command to get going", Duration::from_secs(60), started);
        let asked = Instant::now();
        cancel.store(true, Ordering::SeqCst);
        let result = running.join().unwrap();
        (result, asked.elapsed())
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        // Whatever a failed test left running.
        for file in ["hook", "child", "transport"] {
            if let Some(pid) = pid_in(&self.dir.join(file)) {
                let _ = Command::new("kill")
                    .args(["-KILL", &pid.to_string()])
                    .stderr(Stdio::null())
                    .status();
            }
        }
        let _ = std::fs::remove_dir_all(&self.bare);
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A `pre-push` hook that starts something long, says nothing and waits
/// for it: git-lfs uploading, as far as git can tell.
fn hook_that_uploads(site: &Site, before: &str) {
    script(
        &site.dir.join("hooks/pre-push"),
        &format!(
            "#!/bin/sh\n{before}\nsleep 600 &\necho $! > '{0}/child'\necho $$ > '{0}/hook'\nwait\n",
            site.dir.display()
        ),
    );
}

#[test]
fn stopping_a_push_ends_the_hook_and_what_the_hook_started() {
    let site = Site::new();
    hook_that_uploads(&site, "");
    let before = site.remote_main();
    let (hook, child) = (site.dir.join("hook"), site.dir.join("child"));
    let (result, took) = site.stopped(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
        || pid_in(&hook).is_some() && pid_in(&child).is_some(),
    );
    let err = result.expect_err("a stopped push is not a push");
    assert!(err.contains("cancelled"), "{err}");
    // At once: asked to end, everything did.
    assert!(took < Duration::from_millis(1500), "{took:?}");
    let (hook, child) = (pid_in(&hook).unwrap(), pid_in(&child).unwrap());
    wait_for(
        "the hook and its upload to be gone",
        Duration::from_secs(5),
        || !alive(hook) && !alive(child),
    );
    assert_eq!(site.remote_main(), before, "nothing was pushed");

    // Left alone, the same push goes through: the hook is what held it.
    script(&site.dir.join("hooks/pre-push"), "#!/bin/sh\nexit 0\n");
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": site.copy.path() }))
        .unwrap();
    registry
        .run(
            "remote.push",
            &mut ctx,
            &json!({ "remote": "origin", "branch": "main" }),
        )
        .unwrap();
    assert_eq!(
        site.remote_main(),
        site.copy.git().rev_parse("HEAD").unwrap()
    );
}

#[test]
fn what_will_not_end_when_asked_is_killed_a_moment_later() {
    let site = Site::new();
    // The hook and what it starts shrug off the request to end.
    hook_that_uploads(&site, "trap '' TERM");
    let (hook, child) = (site.dir.join("hook"), site.dir.join("child"));
    let (result, took) = site.stopped(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
        || pid_in(&hook).is_some() && pid_in(&child).is_some(),
    );
    let err = result.expect_err("a stopped push is not a push");
    assert!(err.contains("cancelled"), "{err}");
    // Given its two seconds, then killed; not waited for any longer.
    assert!(
        took > Duration::from_millis(1500) && took < Duration::from_secs(8),
        "{took:?}"
    );
    let (hook, child) = (pid_in(&hook).unwrap(), pid_in(&child).unwrap());
    wait_for(
        "the hook and its upload to be gone",
        Duration::from_secs(5),
        || !alive(hook) && !alive(child),
    );
}

#[test]
fn stopping_a_fetch_ends_the_transport_it_waits_for() {
    let site = Site::new();
    // What stands in for ssh connects to nothing and says nothing.
    let transport = site.dir.join("transport.sh");
    script(
        &transport,
        &format!(
            "#!/bin/sh\necho $$ > '{0}/transport'\nexec sleep 600\n",
            site.dir.display()
        ),
    );
    let git = site.copy.git();
    git.run(["config", "core.sshCommand", transport.to_str().unwrap()])
        .unwrap();
    git.run(["config", "ssh.variant", "simple"]).unwrap();
    git.run(["remote", "add", "far", "ssh://nobody@host.invalid/repo.git"])
        .unwrap();
    let refs = git.lines(["for-each-ref"]).unwrap();
    let pid = site.dir.join("transport");
    let (result, took) = site.stopped("remote.fetch", json!({ "remote": "far" }), || {
        pid_in(&pid).is_some()
    });
    let err = result.expect_err("a stopped fetch is not a fetch");
    assert!(err.contains("cancelled"), "{err}");
    assert!(took < Duration::from_millis(1500), "{took:?}");
    let pid = pid_in(&pid).unwrap();
    wait_for("the transport to be gone", Duration::from_secs(5), || {
        !alive(pid)
    });
    assert_eq!(git.lines(["for-each-ref"]).unwrap(), refs);
}

/// The leash changes nothing about a command that is left alone: how it
/// ended and what it said come back as they were.
#[test]
fn a_command_on_a_leash_still_says_how_it_ended() {
    let site = Site::new();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": site.copy.path() }))
        .unwrap();
    // A remote that is not there: git's own failure, with its own number.
    site.copy
        .git()
        .run([
            "remote",
            "add",
            "gone",
            site.dir.join("no-such.git").to_str().unwrap(),
        ])
        .unwrap();
    let err = registry
        .run("remote.fetch", &mut ctx, &json!({ "remote": "gone" }))
        .expect_err("nothing to fetch from")
        .to_string();
    assert!(err.contains("exit status: 128"), "{err}");
    assert!(
        err.contains("does not appear to be a git repository"),
        "{err}"
    );
    assert!(err.starts_with("git fetch"), "{err}");
    // A hook that refuses is still told apart from git.
    script(
        &site.dir.join("hooks/pre-push"),
        "#!/bin/sh\necho 'not on a Friday' >&2\nexit 3\n",
    );
    let err = registry
        .run(
            "remote.push",
            &mut ctx,
            &json!({ "remote": "origin", "branch": "main" }),
        )
        .expect_err("the hook refuses")
        .to_string();
    assert!(
        err.contains("pre-push") && err.contains("not on a Friday"),
        "{err}"
    );
    // And a fetch that works, works.
    registry
        .run("remote.fetch", &mut ctx, &json!({ "remote": "origin" }))
        .unwrap();
}
