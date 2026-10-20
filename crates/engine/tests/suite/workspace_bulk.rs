//! Commands on many repositories of a workspace at once, held
//! to git: each repository of the workspace has a twin, and git does to
//! the twin what the workspace command does to the repository (`git
//! fetch --all`, `git pull`, `git clone`); afterwards the two have the
//! same references. The remotes are bare repositories on the local file
//! system. One that fails never stops the others, every repository's
//! progress names it, and a stop keeps the rest from starting.
//!
//! The benchmark at the end (ignored; `cargo test -p bitbraid-engine
//! --test suite workspace_bulk::fifty -- --ignored --nocapture`) fetches
//! a workspace of fifty repositories from fifty remotes.
#![allow(clippy::print_stdout)]

use std::{
    collections::BTreeSet,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use bitbraid_engine::{
    CommandContext, Registry, builtin_registry,
    workspace::{RepositoryOutcome, RepositoryState},
};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

use crate::workspaces::{Dir, full};

/// A copy of a fixture following a bare remote of its own.
struct Side {
    copy: Scratch,
    bare: PathBuf,
}

impl Side {
    fn new() -> Self {
        let copy = Scratch::of("linear").unwrap();
        let bare = copy.path().with_extension("origin.git");
        let _ = std::fs::remove_dir_all(&bare);
        let git = copy.git();
        git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
            .unwrap();
        git.run(["remote", "add", "origin", bare.to_str().unwrap()])
            .unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        git.run(["branch", "-q", "--set-upstream-to=origin/main", "main"])
            .unwrap();
        // Commands run git with the real environment; merges need a name.
        git.run(["config", "user.name", "Ada Fixture"]).unwrap();
        git.run(["config", "user.email", "ada@fixture.invalid"])
            .unwrap();
        Self { copy, bare }
    }
    fn path(&self) -> String {
        self.copy.path().to_str().unwrap().to_owned()
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    /// Someone else pushes a commit to `main` on the remote.
    fn upstream_commit(&self, file: &str, content: &str) {
        let other = self.copy.path().with_extension("other");
        let _ = std::fs::remove_dir_all(&other);
        self.git()
            .run([
                "clone",
                "-q",
                self.bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ])
            .unwrap();
        let git = Git::new(&other, fixtures_out_dir().join(".home"));
        std::fs::write(other.join(file), content).unwrap();
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "-m", &format!("upstream: {file}")])
            .unwrap();
        git.run(["push", "-q", "origin", "main"]).unwrap();
        let _ = std::fs::remove_dir_all(&other);
    }
    fn local_commit(&self, file: &str, content: &str) {
        self.copy.write(file, content).unwrap();
        self.git().run(["add", file]).unwrap();
        self.git()
            .run(["commit", "-q", "-m", &format!("local: {file}")])
            .unwrap();
    }
    fn refs(&self) -> Vec<String> {
        self.git()
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap()
    }
}

impl Drop for Side {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.bare);
    }
}

/// A repository of the workspace and its twin, each with a remote of its
/// own that gets the same commits.
struct Pair {
    ours: Side,
    twin: Side,
}

impl Pair {
    fn new() -> Self {
        Self {
            ours: Side::new(),
            twin: Side::new(),
        }
    }
    fn both(&self, f: impl Fn(&Side)) {
        f(&self.ours);
        f(&self.twin);
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(self.ours.refs(), self.twin.refs(), "{what}");
    }
}

/// The progress reports a command made, as they came.
#[derive(Clone, Default)]
struct Reports(Arc<Mutex<Vec<bitbraid_git::Progress>>>);

impl Reports {
    fn outcomes(&self) -> Vec<RepositoryOutcome> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|progress| {
                RepositoryOutcome::from_progress(progress)
                    .unwrap_or_else(|| panic!("a report that names no repository: {progress:?}"))
            })
            .collect()
    }
    /// The states one repository went through, in order, each once.
    fn states_of(&self, path: &str) -> Vec<RepositoryState> {
        let mut states: Vec<RepositoryState> = Vec::new();
        for outcome in self.outcomes().into_iter().filter(|o| o.path == path) {
            if states.last() != Some(&outcome.state) {
                states.push(outcome.state);
            }
        }
        states
    }
}

struct Harness {
    dir: Dir,
    registry: Registry,
    ctx: CommandContext,
    reports: Reports,
}

impl Harness {
    fn new() -> Self {
        let dir = Dir::new("workspace-bulk");
        let reports = Reports::default();
        let kept = reports.clone();
        let ctx = CommandContext {
            config_dir: Some(dir.join("config")),
            progress: Some(Arc::new(move |progress| {
                kept.0.lock().unwrap().push(progress);
            })),
            ..CommandContext::default()
        };
        Self {
            dir,
            registry: builtin_registry(),
            ctx,
            reports,
        }
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.reports.0.lock().unwrap().clear();
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
}

fn states(out: &Value) -> Vec<String> {
    out["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| outcome["state"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn fetch_and_pull_of_a_workspace_match_git_in_each_repository() {
    let mut h = Harness::new();
    let pairs: Vec<Pair> = (0..4).map(|_| Pair::new()).collect();
    for (n, pair) in pairs.iter().enumerate() {
        pair.both(|side| side.upstream_commit(&format!("up{n}.txt"), "one\n"));
    }
    // The third has lost its remote: its fetch fails, in both.
    pairs[2].both(|side| std::fs::remove_dir_all(&side.bare).unwrap());
    let paths: Vec<String> = pairs.iter().map(|pair| pair.ours.path()).collect();
    h.run(
        "workspace.create",
        json!({ "name": "Four", "paths": paths }),
    );

    let out = h.run("workspace.fetch", json!({ "id": "four", "parallel": 2 }));
    for pair in &pairs {
        let _ = pair.twin.git().run(["fetch", "-q", "--all"]);
    }
    for (n, pair) in pairs.iter().enumerate() {
        pair.assert_same(&format!("fetch of repository {n}"));
    }
    assert_eq!(states(&out), ["done", "done", "failed", "done"], "{out}");
    assert_eq!(
        (out["done"].as_u64(), out["failed"].as_u64()),
        (Some(3), Some(1))
    );
    assert_eq!(out["command"], "remote.fetch");
    assert_eq!(out["parallel"], 2);
    assert_eq!(out["stopped"], false);
    let failed = &out["repositories"][2];
    assert_eq!(failed["path"], paths[2].as_str());
    assert!(!failed["error"].as_str().unwrap().is_empty(), "{failed}");
    // Every repository's progress named it, from its turn to its end.
    for (n, path) in paths.iter().enumerate() {
        let went = h.reports.states_of(path);
        let last = if n == 2 {
            RepositoryState::Failed
        } else {
            RepositoryState::Done
        };
        assert_eq!(
            went,
            [RepositoryState::Queued, RepositoryState::Running, last],
            "{path}"
        );
    }

    // Fast-forward pulls; the one without a remote fails again, alone.
    for (n, pair) in pairs.iter().enumerate() {
        if n != 2 {
            pair.both(|side| side.upstream_commit(&format!("again{n}.txt"), "two\n"));
        }
    }
    let out = h.run(
        "workspace.pull",
        json!({ "id": "four", "mode": "ff_only", "parallel": 3 }),
    );
    for pair in &pairs {
        let _ = pair.twin.git().run(["pull", "-q", "--ff-only"]);
    }
    for (n, pair) in pairs.iter().enumerate() {
        pair.assert_same(&format!("pull of repository {n}"));
    }
    assert_eq!(states(&out), ["done", "done", "failed", "done"], "{out}");
    assert_ne!(
        out["repositories"][0]["output"]["before"],
        out["repositories"][0]["output"]["after"]
    );

    // Only those named; a rebase where there is a commit of one's own,
    // and a fast-forward that cannot be, which fails by itself.
    pairs[0].both(|side| {
        side.local_commit("mine.txt", "mine\n");
        side.upstream_commit("theirs.txt", "theirs\n");
    });
    pairs[1].both(|side| {
        side.local_commit("mine.txt", "mine\n");
        side.upstream_commit("theirs.txt", "theirs\n");
    });
    let out = h.run(
        "workspace.pull",
        json!({ "id": "four", "paths": [paths[1], paths[0]], "mode": "rebase" }),
    );
    // In the workspace's order, and only those two.
    let named: Vec<&str> = out["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| outcome["path"].as_str().unwrap())
        .collect();
    assert_eq!(named, [paths[0].as_str(), paths[1].as_str()]);
    assert_eq!(states(&out), ["done", "done"], "{out}");
    for pair in &pairs[..2] {
        pair.twin
            .git()
            .run_with(["pull", "-q", "--rebase"], &[])
            .unwrap();
        // The rebased commit carries today's date: trees and messages.
        let shape = |side: &Side| {
            let git = side.git();
            (
                git.rev_parse("HEAD^{tree}").unwrap(),
                git.lines(["log", "--format=%s %T"]).unwrap(),
                git.rev_parse("origin/main").unwrap(),
            )
        };
        assert_eq!(shape(&pair.ours), shape(&pair.twin));
    }
    // Undo is each repository's: the pull is the last thing in its journal.
    let mut ctx = CommandContext::default();
    h.registry
        .run("repo.open", &mut ctx, &json!({ "path": paths[0] }))
        .unwrap();
    let peek = h.registry.run("undo.peek", &mut ctx, &json!({})).unwrap();
    assert_eq!(peek["undo"], "Pull", "{peek}");
}

#[test]
fn a_bare_repository_is_fetched_and_never_pulled() {
    let mut h = Harness::new();
    let pair = Pair::new();
    pair.both(|side| side.upstream_commit("up.txt", "one\n"));
    let bare = pair.ours.bare.to_str().unwrap().to_owned();
    h.run(
        "workspace.create",
        json!({ "name": "Mixed", "paths": [pair.ours.path(), bare] }),
    );
    let before = Git::new(&pair.ours.bare, fixtures_out_dir().join(".home"))
        .lines(["for-each-ref"])
        .unwrap();
    let out = h.run("workspace.pull", json!({ "id": "mixed" }));
    assert_eq!(states(&out), ["done", "failed"], "{out}");
    let said = out["repositories"][1]["error"].as_str().unwrap();
    assert!(said.contains("read-only"), "{said}");
    pair.twin.git().run(["pull", "-q"]).unwrap();
    pair.assert_same("pull beside a bare repository");
    let after = Git::new(&pair.ours.bare, fixtures_out_dir().join(".home"))
        .lines(["for-each-ref"])
        .unwrap();
    assert_eq!(before, after);
}

#[test]
fn missing_repositories_are_cloned_as_git_clones_them() {
    let mut h = Harness::new();
    let first = Side::new();
    let second = Side::new();
    first.upstream_commit("up.txt", "one\n");
    let present = Side::new();
    let gone = |name: &str| h.dir.text(name);
    let (a, b, nowhere) = (gone("a"), gone("b"), gone("no-address"));
    h.run(
        "workspace.create",
        json!({
            "name": "Clones",
            "paths": [present.path()],
            "repositories": [
                { "path": a, "remote_url": first.bare.to_str().unwrap() },
                { "path": b, "remote_url": second.bare.to_str().unwrap() },
                { "path": nowhere, "remote_url": h.dir.text("no-such-remote.git") },
            ],
        }),
    );
    // The one that is there is not asked about; the one whose remote is
    // not there fails, alone.
    let out = h.run("workspace.clone_missing", json!({ "id": "clones" }));
    let paths: Vec<&str> = out["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|outcome| outcome["path"].as_str().unwrap())
        .collect();
    assert_eq!(paths, [a.as_str(), b.as_str(), nowhere.as_str()]);
    assert_eq!(states(&out), ["done", "done", "failed"], "{out}");
    assert!(
        !std::path::Path::new(&nowhere).exists(),
        "a failed clone leaves nothing"
    );
    for (path, side) in [(&a, &first), (&b, &second)] {
        let twin = h.dir.join(&format!(
            "twin-{}",
            side.copy.path().file_name().unwrap().to_str().unwrap()
        ));
        side.git()
            .run([
                "clone",
                "-q",
                side.bare.to_str().unwrap(),
                twin.to_str().unwrap(),
            ])
            .unwrap();
        let refs = |at: &std::path::Path| {
            Git::new(at, fixtures_out_dir().join(".home"))
                .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
                .unwrap()
        };
        assert_eq!(refs(std::path::Path::new(path)), refs(&twin), "{path}");
    }
    let listed = h.run("workspace.list", json!({ "id": "clones" }));
    assert_eq!(listed["workspaces"][0]["missing"], json!([nowhere]));

    // Named, the one that is there is said to be; into a folder, the
    // workspace has it where it went.
    let into = h.dir.join("into");
    std::fs::create_dir_all(&into).unwrap();
    let out = h.run(
        "workspace.clone_missing",
        json!({ "id": "clones", "paths": [present.path(), nowhere], "folder": into }),
    );
    assert_eq!(states(&out), ["skipped", "failed"], "{out}");
    assert_eq!(out["repositories"][0]["error"], "it is there already");
    std::fs::remove_dir_all(&a).unwrap();
    let out = h.run(
        "workspace.clone_missing",
        json!({ "id": "clones", "paths": [a], "folder": into }),
    );
    assert_eq!(states(&out), ["done"], "{out}");
    let moved_to = into.join("a").to_str().unwrap().to_owned();
    assert_eq!(out["moved"], json!([{ "from": a, "to": moved_to }]));
    let listed = h.run("workspace.list", json!({ "id": "clones" }));
    let kept: Vec<&str> = listed["workspaces"][0]["repositories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        kept,
        [
            present.path().as_str(),
            moved_to.as_str(),
            b.as_str(),
            nowhere.as_str()
        ]
    );

    // Without an address there is nothing to clone from: such an entry
    // comes from a file written elsewhere.
    let config = h.dir.join("config");
    let mut kept = bitbraid_engine::workspace::load(&config)
        .unwrap()
        .workspaces;
    kept[0]
        .repositories
        .push(bitbraid_engine::workspace::WorkspaceRepository {
            path: h.dir.text("c"),
            remote_url: None,
        });
    std::fs::write(
        config.join(bitbraid_engine::workspace::FILE),
        bitbraid_engine::workspace::to_text(&kept).unwrap(),
    )
    .unwrap();
    let out = h.run(
        "workspace.clone_missing",
        json!({ "id": "clones", "paths": [h.dir.text("c")] }),
    );
    assert_eq!(states(&out), ["skipped"], "{out}");
    assert_eq!(
        out["repositories"][0]["error"],
        "there is no address to clone it from"
    );
}

#[test]
fn a_stop_keeps_the_rest_from_starting() {
    let mut h = Harness::new();
    let sides: Vec<Side> = (0..3).map(|_| Side::new()).collect();
    for side in &sides {
        side.upstream_commit("up.txt", "one\n");
    }
    let paths: Vec<String> = sides.iter().map(Side::path).collect();
    h.run(
        "workspace.create",
        json!({ "name": "Stop", "paths": paths }),
    );
    let before: Vec<Vec<String>> = sides.iter().map(Side::refs).collect();

    // Stopped before it starts: nothing runs.
    h.ctx.cancel.store(true, Ordering::SeqCst);
    let out = h.run("workspace.fetch", json!({ "id": "stop" }));
    assert_eq!(states(&out), ["skipped", "skipped", "skipped"], "{out}");
    assert_eq!(out["stopped"], true);
    assert_eq!(out["repositories"][0]["error"], "stopped before its turn");
    assert_eq!(sides.iter().map(Side::refs).collect::<Vec<_>>(), before);

    // Stopped as the first one starts, one at a time: the others do not.
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    h.ctx.cancel = cancel;
    h.ctx.progress = Some(Arc::new(move |progress| {
        if RepositoryOutcome::from_progress(&progress)
            .is_some_and(|outcome| outcome.state == RepositoryState::Running)
        {
            flag.store(true, Ordering::SeqCst);
        }
    }));
    let out = h.run("workspace.fetch", json!({ "id": "stop", "parallel": 1 }));
    let went = states(&out);
    assert_eq!(&went[1..], ["skipped", "skipped"], "{out}");
    assert_eq!(out["stopped"], true);
    assert_eq!(sides[1].refs(), before[1]);
    assert_eq!(sides[2].refs(), before[2]);
}

#[test]
fn what_a_bulk_command_is_given_is_checked() {
    let mut h = Harness::new();
    let side = Side::new();
    h.run(
        "workspace.create",
        json!({ "name": "One", "paths": [side.path()] }),
    );
    for (id, params, expected) in [
        ("workspace.fetch", json!({}), "id"),
        ("workspace.fetch", json!({ "id": "two" }), "no workspace"),
        (
            "workspace.fetch",
            json!({ "id": "one", "parallel": 0 }),
            "from 1",
        ),
        (
            "workspace.fetch",
            json!({ "id": "one", "parallel": 33 }),
            "from 1",
        ),
        (
            "workspace.fetch",
            json!({ "id": "one", "parallel": -1 }),
            "parallel",
        ),
        (
            "workspace.fetch",
            json!({ "id": "one", "paths": [full("/elsewhere")] }),
            "not one of",
        ),
        (
            "workspace.fetch",
            json!({ "id": "one", "paths": ["rel"] }),
            "full path",
        ),
        (
            "workspace.pull",
            json!({ "id": "one", "mode": "sideways" }),
            "mode",
        ),
        (
            "workspace.clone_missing",
            json!({ "id": "one", "folder": "rel" }),
            "full path",
        ),
        (
            "workspace.clone_missing",
            json!({ "id": "one", "folder": h.dir.text("nowhere") }),
            "not a folder",
        ),
    ] {
        let said = h
            .registry
            .run(id, &mut h.ctx, &params)
            .map(|out| out.to_string())
            .map_or_else(
                |err| err.to_string(),
                |out| panic!("{id} {params} was taken: {out}"),
            );
        assert!(said.contains(expected), "{id} {params}: {said}");
    }
    // A workspace that has none of its repositories here does nothing.
    std::fs::remove_dir_all(side.copy.path()).unwrap();
    let out = h.run("workspace.fetch", json!({ "id": "one" }));
    assert_eq!(states(&out), ["skipped"]);
    assert!(
        out["repositories"][0]["error"]
            .as_str()
            .unwrap()
            .contains("clone it first")
    );
}

/// Fifty repositories, each with a remote of its own that has a commit
/// it has not: fetched eight at once, and one at a time, timed. With
/// eight, several have to run at once.
#[test]
#[ignore = "a benchmark; run it by name with --ignored --nocapture"]
fn fifty_repositories_fetch_in_parallel() {
    let mut h = Harness::new();
    let sides: Vec<Side> = (0..50).map(|_| Side::new()).collect();
    let paths: Vec<String> = sides.iter().map(Side::path).collect();
    h.run(
        "workspace.create",
        json!({ "name": "Fifty", "paths": paths }),
    );
    let mut timings = Vec::new();
    for (round, parallel) in [8usize, 1].into_iter().enumerate() {
        for side in &sides {
            side.upstream_commit(&format!("round{round}.txt"), "x\n");
        }
        // How many were running at once, at most.
        let running: Arc<Mutex<(BTreeSet<String>, usize)>> = Arc::default();
        let seen = running.clone();
        h.ctx.progress = Some(Arc::new(move |progress| {
            if let Some(outcome) = RepositoryOutcome::from_progress(&progress) {
                let mut seen = seen.lock().unwrap();
                match outcome.state {
                    RepositoryState::Running => {
                        seen.0.insert(outcome.path);
                    }
                    state if state.is_final() => {
                        seen.0.remove(&outcome.path);
                    }
                    _ => {}
                }
                seen.1 = seen.1.max(seen.0.len());
            }
        }));
        let started = std::time::Instant::now();
        let out = h.run(
            "workspace.fetch",
            json!({ "id": "fifty", "parallel": parallel }),
        );
        let took = started.elapsed();
        assert_eq!(out["done"], 50, "{out}");
        let most = running.lock().unwrap().1;
        println!(
            "BENCH workspace.fetch repositories=50 parallel={parallel} took={took:?} \
             elapsed_ms={} most_at_once={most}",
            out["elapsed_ms"]
        );
        if parallel > 1 {
            assert!(most > 1, "only {most} ran at once");
        } else {
            assert_eq!(most, 1);
        }
        timings.push(took);
        for side in &sides {
            let refs = side.refs();
            assert!(
                refs.iter()
                    .any(|line| line.starts_with("refs/remotes/origin/main ")),
                "{refs:?}"
            );
        }
    }
    println!(
        "BENCH workspace.fetch speed-up of eight at once over one: {:.1}x",
        timings[1].as_secs_f64() / timings[0].as_secs_f64().max(1e-9)
    );
}
