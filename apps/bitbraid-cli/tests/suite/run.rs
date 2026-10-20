//! `bitbraid-cli run <id>` as a program: what needs no repository runs
//! outside one, a long command reports on standard error while standard
//! output stays the JSON result, and Ctrl-C stops it through the engine's
//! cancel flag.
//!
//! The binary is run with a home and a git configuration of the fixtures'
//! own, so nothing of whoever runs the tests is read or written.
//!
//! Ctrl-C is tried on Unix only: it is a signal sent with `kill` there,
//! and the program hears it through signal-hook's Unix signals.

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};
#[cfg(unix)]
use std::{
    os::unix::{fs::PermissionsExt as _, process::ExitStatusExt as _},
    process::Child,
    time::{Duration, Instant},
};

use bitbraid_testkit::{Git, Scratch, build_fixture, fixtures_out_dir, scratch_root};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory of the test's own that is in no repository.
fn playground(name: &str) -> PathBuf {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = scratch_root().join(format!("cli-{name}-{}-{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    bitbraid_core::paths::canonical(dir).unwrap()
}

fn home() -> PathBuf {
    let git = Git::new(fixtures_out_dir(), fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git.home().to_path_buf()
}

/// The binary, to be run in `dir`, seeing only the fixtures' configuration.
fn cli(dir: &Path) -> Command {
    let home = home();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_bitbraid-cli"));
    cmd.current_dir(dir)
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("BITBRAID_CONFIG_DIR", dir.join("bitbraid-config"))
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

fn run_in(dir: &Path, args: &[&str]) -> Output {
    cli(dir).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// `run <id> --params <params>` in `dir`: the JSON it printed.
fn ok(dir: &Path, id: &str, params: Value) -> Value {
    let out = run_in(dir, &["run", id, "--params", &params.to_string()]);
    assert!(out.status.success(), "{id} {params}: {}", text(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|err| {
        panic!(
            "{id}: standard output is not JSON ({err}): {}",
            text(&out.stdout)
        )
    })
}

/// The same, expected to fail: what it wrote to standard error.
fn failed(dir: &Path, args: &[&str]) -> String {
    let out = run_in(dir, args);
    assert_eq!(
        out.status.code(),
        Some(1),
        "{args:?}: {}",
        text(&out.stdout)
    );
    assert!(out.stdout.is_empty(), "{args:?}: {}", text(&out.stdout));
    text(&out.stderr)
}

#[test]
fn commands_that_need_no_repository_run_outside_one() {
    let dir = playground("outside");
    let source = build_fixture("branches-merges").unwrap();

    // Making repositories.
    let made = dir.join("made");
    let out = ok(
        &dir,
        "repo.init",
        json!({ "path": made, "branch": "trunk" }),
    );
    assert_eq!(out["path"], made.to_str().unwrap());
    let git = Git::new(&made, home());
    assert_eq!(
        git.run(["symbolic-ref", "--short", "HEAD"]).unwrap(),
        "trunk\n"
    );
    let cloned = dir.join("cloned");
    let out = ok(&dir, "repo.clone", json!({ "url": source, "path": cloned }));
    assert_eq!(out["path"], cloned.to_str().unwrap());
    assert_eq!(
        Git::new(&cloned, home()).rev_parse("HEAD").unwrap(),
        Git::new(&source, home()).rev_parse("HEAD").unwrap()
    );

    // Links: made and read with no repository anywhere.
    let link = ok(
        &dir,
        "link.make",
        json!({ "path": "/srv/project", "branch": "main" }),
    );
    let link = link["link"].as_str().unwrap();
    let parsed = ok(&dir, "link.parse", json!({ "url": link }));
    assert_eq!(parsed["repository"]["path"], "/srv/project");
    assert_eq!(
        parsed["target"],
        json!({ "kind": "branch", "value": "main" })
    );
    // One that would mean the open repository says that none is open,
    // which is the command's answer and not a failure to open.
    let err = failed(
        &dir,
        &["run", "link.make", "--params", r#"{"branch":"main"}"#],
    );
    assert!(err.contains("none is open"), "{err}");
    assert!(!err.contains("opening repository"), "{err}");

    // Profiles are the application's, kept where it is told to.
    assert_eq!(ok(&dir, "profile.list", json!({}))["profiles"], json!([]));
    let profile = json!({ "name": "Work", "user_name": "Ada", "user_email": "ada@work.invalid" });
    ok(&dir, "profile.save", json!({ "profile": profile }));
    assert_eq!(
        ok(&dir, "profile.list", json!({}))["profiles"][0]["name"],
        "Work"
    );
    assert!(dir.join("bitbraid-config/profiles.toml").is_file());
    // Without a repository the identity is the user's.
    let current = ok(&dir, "profile.current", json!({}));
    assert_eq!(current["scope"], "user");
    assert_eq!(current["identity"]["user_email"], "ada@fixture.invalid");
    ok(&dir, "profile.delete", json!({ "name": "Work" }));

    // A query is read without anything to search in.
    let query = ok(
        &dir,
        "search.parse",
        json!({ "query": "author:ada is:merge" }),
    );
    assert!(query["error"].is_null(), "{query}");

    // What does need a repository says where it looked.
    for args in [
        vec!["run", "repo.log"],
        vec!["run", "branch.create", "--params", r#"{"name":"x"}"#],
        vec!["summary"],
        vec!["status"],
    ] {
        let err = failed(&dir, &args);
        assert!(err.contains("opening repository at ."), "{args:?}: {err}");
    }
    let err = failed(&dir, &["run", "no.such"]);
    assert!(err.contains("unknown command"), "{err}");
    assert!(!err.contains("opening repository"), "{err}");
    // The listing needs none either.
    let listed = run_in(&dir, &["commands", "--json"]);
    assert!(listed.status.success());
    assert!(
        serde_json::from_slice::<Value>(&listed.stdout)
            .unwrap()
            .is_array()
    );
}

/// The listing says of each command what it needs of a repository and
/// what it may do to one, as the engine's table has it, so that whoever
/// drives the program can tell before running anything.
#[test]
fn the_listing_says_what_each_command_needs_and_does() {
    let dir = playground("listing");
    let listed = run_in(&dir, &["commands", "--json"]);
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let listed: Value = serde_json::from_slice(&listed.stdout).unwrap();
    let rows = listed.as_array().unwrap();
    assert!(rows.len() > 100, "{}", rows.len());
    let access = |id: &str| {
        rows.iter()
            .find(|row| row["id"] == id)
            .unwrap_or_else(|| panic!("{id} is not listed"))["access"]
            .clone()
    };
    let says =
        |repository: &str, effect: &str| json!({ "repository": repository, "effect": effect });
    // One that needs no repository, one that reads, one that writes.
    assert_eq!(access("repo.init"), says("nothing", "reads"));
    assert_eq!(access("repo.log"), says("repository", "reads"));
    assert_eq!(access("branch.create"), says("repository", "writes"));
    // One that uses a repository when one is open, and one that writes
    // configuration where its `scope` says.
    assert_eq!(access("link.make"), says("repository_if_open", "reads"));
    assert_eq!(access("sign.configure"), says("repository", "writes_scope"));
    assert_eq!(
        access("profile.use"),
        says("repository_if_open", "writes_scope")
    );
    // Every row has it, in these words and no others, beside its label,
    // description and parameters.
    for row in rows {
        let id = row["id"].as_str().unwrap();
        let access = row["access"].as_object().unwrap_or_else(|| panic!("{id}"));
        assert_eq!(access.len(), 2, "{id}: {row}");
        assert!(
            ["repository", "repository_if_open", "nothing"]
                .contains(&access["repository"].as_str().unwrap()),
            "{id}: {row}"
        );
        assert!(
            ["reads", "writes", "writes_scope"].contains(&access["effect"].as_str().unwrap()),
            "{id}: {row}"
        );
        for field in ["label", "description", "params"] {
            assert!(row.get(field).is_some(), "{id} has no {field}");
        }
    }
    // What the listing says is what running does: a command that needs
    // nothing runs here, where no repository is, and one that needs a
    // repository says that there is none.
    assert!(ok(&dir, "search.parse", json!({ "query": "is:merge" }))["error"].is_null());
    assert_eq!(access("search.parse"), says("nothing", "reads"));
    let err = failed(&dir, &["run", "repo.log"]);
    assert!(err.contains("opening repository at ."), "{err}");
}

#[test]
fn a_repository_is_used_when_there_is_one_and_required_when_it_is_named() {
    let dir = playground("named");
    let copy = Scratch::of("linear").unwrap();
    let path = copy.path().to_str().unwrap();

    // In a repository, a command that can use one does.
    let out = cli(copy.path())
        .args(["run", "link.make", "--params", r#"{"branch":"main"}"#])
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", text(&out.stderr));
    let link: Value = serde_json::from_slice(&out.stdout).unwrap();
    let parsed = ok(&dir, "link.parse", json!({ "url": link["link"] }));
    assert_eq!(
        Path::new(parsed["repository"]["path"].as_str().unwrap())
            .canonicalize()
            .unwrap(),
        copy.path().canonicalize().unwrap()
    );
    // Named from elsewhere: the same.
    let out = run_in(&dir, &["--repo", path, "run", "link.make"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    let current = run_in(&dir, &["--repo", path, "run", "profile.current"]);
    let current: Value = serde_json::from_slice(&current.stdout).unwrap();
    assert_eq!(current["scope"], "effective");

    // A repository that is named and is not there is an error also for a
    // command that could do without one: that one was meant.
    let nowhere = dir.join("nowhere");
    let err = failed(
        &dir,
        &[
            "--repo",
            nowhere.to_str().unwrap(),
            "run",
            "link.make",
            "--params",
            r#"{"path":"/x"}"#,
        ],
    );
    assert!(err.contains("opening repository at"), "{err}");
    // And is of no interest to a command that takes no notice of one.
    let out = run_in(
        &dir,
        &[
            "--repo",
            nowhere.to_str().unwrap(),
            "run",
            "search.parse",
            "--params",
            r#"{"query":"x"}"#,
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
}

/// The activity log between runs, with runs that are runs: each is a
/// process of its own, and the second lists what the first did.
#[test]
fn what_one_run_did_the_next_one_lists() {
    let dir = playground("log");
    let copy = Scratch::of("linear").unwrap();
    let path = copy.path().to_str().unwrap();
    let list = |repo: &str| -> Vec<Value> {
        let out = run_in(&dir, &["--repo", repo, "run", "activity.list"]);
        assert!(out.status.success(), "{}", text(&out.stderr));
        let listed: Value = serde_json::from_slice(&out.stdout).unwrap();
        listed["entries"].as_array().unwrap().clone()
    };
    assert!(list(path).is_empty());

    let tag = r#"{"name":"from-the-first-run"}"#;
    let out = run_in(
        &dir,
        &["--repo", path, "run", "tag.create", "--params", tag],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let listed = list(path);
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0]["command"]
            .as_str()
            .unwrap()
            .contains("from-the-first-run")
    );
    assert_eq!(listed[0]["origin"], "tag.create");
    assert_eq!(listed[0]["code"], 0);

    // Another run adds to it; the newest is first.
    let out = run_in(
        &dir,
        &["--repo", path, "run", "tag.delete", "--params", tag],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let listed = list(path);
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert_eq!(listed[0]["origin"], "tag.delete");
    assert!(listed[0]["id"].as_u64() > listed[1]["id"].as_u64());

    // It is kept with the application's own files, not in the repository.
    let kept: Vec<_> = fs::read_dir(dir.join("bitbraid-config/activity"))
        .unwrap()
        .flatten()
        .collect();
    assert_eq!(kept.len(), 1);
    assert!(!copy.path().join(".git/bitbraid/activity").exists());

    // A repository that a run made has that in its log.
    let made = dir.join("made");
    ok(&dir, "repo.init", json!({ "path": made }));
    let listed = list(made.to_str().unwrap());
    assert_eq!(listed.len(), 1, "{listed:?}");
    assert!(
        listed[0]["command"]
            .as_str()
            .unwrap()
            .starts_with("git init")
    );
    assert_eq!(listed[0]["origin"], "repo.init");

    // Cleared in one run, gone in the next.
    let out = run_in(&dir, &["--repo", path, "run", "activity.clear"]);
    let cleared: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(cleared["cleared"], 2);
    assert!(list(path).is_empty());
    assert_eq!(list(made.to_str().unwrap()).len(), 1, "the other log stays");
}

#[test]
fn a_long_command_reports_on_standard_error_and_answers_on_standard_output() {
    let dir = playground("progress");
    let source = build_fixture("branches-merges").unwrap();
    // Over the pack protocol, which reports; a path alone is copied. (On
    // Windows the address has the drive after a slash and `/` between
    // folders.)
    let url = if cfg!(windows) {
        format!(
            "file:///{}",
            source.display().to_string().replace('\\', "/")
        )
    } else {
        format!("file://{}", source.display())
    };

    let out = run_in(
        &dir,
        &[
            "--compact",
            "run",
            "repo.clone",
            "--params",
            &json!({ "url": url, "path": dir.join("one") }).to_string(),
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    // One line of JSON and nothing else.
    let stdout = text(&out.stdout);
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    let answer: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(answer["url"], url);
    // The stages git went through, a line each; not a terminal, so none
    // is rewritten and there is no escape sequence.
    let stderr = text(&out.stderr);
    assert!(stderr.contains("Receiving objects: 100%"), "{stderr}");
    assert!(
        !stderr.contains('\r') && !stderr.contains('\u{1b}'),
        "{stderr:?}"
    );
    for line in stderr.lines() {
        let stage = line.split(':').next().unwrap();
        assert!(
            !stage.is_empty() && stage.chars().all(|c| c.is_ascii_alphabetic() || c == ' '),
            "not a progress line: {line:?}"
        );
    }
    let stages: Vec<&str> = stderr.lines().collect();
    let mut once = stages.clone();
    once.dedup();
    assert_eq!(stages, once, "a line is not written twice");

    // Asked to be quiet, it is.
    let out = run_in(
        &dir,
        &[
            "--no-progress",
            "run",
            "repo.clone",
            "--params",
            &json!({ "url": url, "path": dir.join("two") }).to_string(),
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    assert!(out.stderr.is_empty(), "{}", text(&out.stderr));
    assert!(dir.join("two/.git").is_dir());
}

#[cfg(unix)]
fn script(path: &Path, body: &str) {
    fs::write(path, body).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(unix)]
fn wait_for(what: &str, limit: Duration, mut done: impl FnMut() -> bool) {
    let started = Instant::now();
    while !done() {
        assert!(started.elapsed() < limit, "waited too long for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[cfg(unix)]
fn signal(child: &Child, name: &str) {
    let status = Command::new("kill")
        .args([name, &child.id().to_string()])
        .status()
        .unwrap();
    assert!(status.success());
}

/// Wait for the process to end, and say how long that took.
#[cfg(unix)]
fn ended(mut child: Child, limit: Duration) -> (Output, Duration) {
    let started = Instant::now();
    wait_for("the command line to end", limit, || {
        child.try_wait().unwrap().is_some()
    });
    let took = started.elapsed();
    (child.wait_with_output().unwrap(), took)
}

/// A repository whose remote `slow` never answers: what stands in for
/// git-upload-pack writes down its process number when it is started and
/// then waits for as long as the git that started it is there.
#[cfg(unix)]
fn with_a_remote_that_hangs(dir: &Path) -> Scratch {
    let copy = Scratch::of("linear").unwrap();
    let remote = dir.join("slow.git");
    let git = copy.git();
    git.run(["clone", "-q", "--bare", ".", remote.to_str().unwrap()])
        .unwrap();
    let pack = dir.join("upload-pack.sh");
    script(
        &pack,
        &format!(
            "#!/bin/sh\necho $$ > '{0}/started'\ncat > /dev/null\n",
            dir.display()
        ),
    );
    git.run(["remote", "add", "slow", remote.to_str().unwrap()])
        .unwrap();
    git.run(["config", "remote.slow.uploadpack", pack.to_str().unwrap()])
        .unwrap();
    copy
}

#[cfg(unix)]
#[test]
fn ctrl_c_stops_a_running_command_through_the_engine() {
    let dir = playground("interrupt");
    let copy = with_a_remote_that_hangs(&dir);
    let refs = copy.git().lines(["for-each-ref"]).unwrap();

    let child = cli(&dir)
        .args(["--repo", copy.path().to_str().unwrap()])
        .args(["run", "remote.fetch", "--params", r#"{"remote":"slow"}"#])
        .spawn()
        .unwrap();
    wait_for("the fetch to start", Duration::from_secs(30), || {
        dir.join("started").exists()
    });
    // To this process alone: what stops git is the engine, not the
    // terminal handing the signal to everything in the foreground.
    signal(&child, "-INT");
    let (out, took) = ended(child, Duration::from_secs(20));
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(130), "{stderr}");
    assert!(stderr.ends_with("error: stopped\n"), "{stderr}");
    assert!(out.stdout.is_empty(), "{}", text(&out.stdout));
    // At once: well inside the moment given to commands that do not watch.
    assert!(took < Duration::from_millis(2500), "{took:?}");
    // Git is gone with it, and so is what git had started; nothing was
    // fetched.
    let transport: u32 = fs::read_to_string(dir.join("started"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    wait_for(
        "git's transport to be gone",
        Duration::from_secs(10),
        || {
            !Command::new("kill")
                .args(["-0", &transport.to_string()])
                .stderr(Stdio::null())
                .status()
                .unwrap()
                .success()
        },
    );
    assert_eq!(copy.git().lines(["for-each-ref"]).unwrap(), refs);
    assert!(
        !copy.path().join(".git/FETCH_HEAD").exists()
            || fs::read_to_string(copy.path().join(".git/FETCH_HEAD"))
                .unwrap()
                .is_empty()
    );
}

/// A repository in which a commit takes as long as the test lets it: the
/// hook waits for a file to appear, or for some seconds at most.
#[cfg(unix)]
fn with_a_hook_that_waits(dir: &Path) -> Scratch {
    let copy = Scratch::of("linear").unwrap();
    script(
        &copy.path().join(".git/hooks/pre-commit"),
        &format!(
            "#!/bin/sh\n: > '{0}/started'\nn=0\nwhile [ ! -e '{0}/go' ] && [ $n -lt 150 ]; do sleep 0.1; n=$((n+1)); done\nexit 1\n",
            dir.display()
        ),
    );
    copy
}

#[cfg(unix)]
#[test]
fn a_command_that_cannot_be_stopped_is_left_after_a_moment_and_at_once_the_second_time() {
    let commit = [
        "run",
        "commit.create",
        "--params",
        r#"{"message":"waits","allow_empty":true}"#,
    ];

    // Once: the command line waits a moment for it, then ends.
    let dir = playground("left");
    let copy = with_a_hook_that_waits(&dir);
    let head = copy.git().rev_parse("HEAD").unwrap();
    let child = cli(&dir)
        .args(["--repo", copy.path().to_str().unwrap()])
        .args(commit)
        .spawn()
        .unwrap();
    wait_for("the hook to start", Duration::from_secs(30), || {
        dir.join("started").exists()
    });
    signal(&child, "-INT");
    let (out, took) = ended(child, Duration::from_secs(20));
    let stderr = text(&out.stderr);
    assert_eq!(out.status.code(), Some(130), "{stderr}");
    assert!(stderr.contains("did not end by itself"), "{stderr}");
    assert!(took >= Duration::from_millis(2500), "{took:?}");
    // The hook is let go, refuses the commit, and nothing was committed.
    fs::write(dir.join("go"), "").unwrap();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(copy.git().rev_parse("HEAD").unwrap(), head);

    // Twice: the second one ends the process as a signal does.
    let dir = playground("twice");
    let copy = with_a_hook_that_waits(&dir);
    let child = cli(&dir)
        .args(["--repo", copy.path().to_str().unwrap()])
        .args(commit)
        .spawn()
        .unwrap();
    wait_for("the hook to start", Duration::from_secs(30), || {
        dir.join("started").exists()
    });
    signal(&child, "-INT");
    std::thread::sleep(Duration::from_millis(300));
    signal(&child, "-INT");
    let (out, took) = ended(child, Duration::from_secs(20));
    assert_eq!(out.status.signal(), Some(2), "{:?}", out.status);
    assert!(took < Duration::from_millis(2000), "{took:?}");
    fs::write(dir.join("go"), "").unwrap();
    std::thread::sleep(Duration::from_millis(400));
}
