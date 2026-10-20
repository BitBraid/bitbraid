//! Maintenance against git on twin copies: each task runs as a
//! command on one copy and as git itself on the other, and the two have
//! to hold the same references and the same objects afterwards; what the
//! command says was there before and after is what `git count-objects`
//! says. The check is held to `git fsck` on sound and on damaged copies,
//! and the size of the repository to what git lists of its objects and
//! history.
//!
//! The commands run git as whoever runs the tests has it set up, and so
//! does the twin: both read the same configuration.

use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, atomic::Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir, scratch_root};
use serde_json::{Value, json};

/// Git as the commands run it: with the configuration of whoever runs the
/// tests, in `dir` and nowhere above it.
fn plain_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// What git says and how it ended, also when it ends badly.
fn plain_git_code(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn hermetic(dir: &Path) -> Git {
    let git = Git::new(dir, fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git
}

struct Engine {
    registry: Registry,
    ctx: CommandContext,
}

impl Engine {
    fn at(path: &Path) -> Self {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": path }))
            .unwrap();
        Self { registry, ctx }
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }
}

/// Every object a repository has, reachable or not.
fn objects(dir: &Path) -> BTreeSet<String> {
    plain_git(
        dir,
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objectname)",
        ],
    )
    .lines()
    .map(str::to_owned)
    .collect()
}

fn references(dir: &Path) -> Vec<String> {
    plain_git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// `git count-objects -v` as a map of its numbers.
fn counted(dir: &Path) -> BTreeMap<String, u64> {
    plain_git(dir, &["count-objects", "-v"])
        .lines()
        .filter_map(|line| {
            let (key, value) = line.split_once(':')?;
            Some((key.trim().to_owned(), value.trim().parse().ok()?))
        })
        .collect()
}

/// A copy of `layout` with something nothing reaches: a commit that was
/// made and then dropped, with a blob of its own, and a blob that was only
/// written.
fn with_unreachable() -> Scratch {
    let copy = Scratch::of("layout").unwrap();
    let git = copy.git();
    copy.write("dropped.txt", "a file of a commit that was dropped")
        .unwrap();
    git.run(["add", "dropped.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "dropped"]).unwrap();
    git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    git.run(["reflog", "expire", "--expire=now", "--all"])
        .unwrap();
    let mut child = git
        .command()
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"a blob nothing refers to")
        .unwrap();
    assert!(child.wait().unwrap().success());
    copy
}

/// What the twin runs for each task.
fn by_git(task: &str) -> Vec<Vec<String>> {
    let words = |line: &[&str]| line.iter().map(|word| (*word).to_owned()).collect();
    match task {
        "gc" => vec![words(&["gc", "--quiet"])],
        "gc_aggressive" => vec![words(&["gc", "--quiet", "--aggressive"])],
        "prune" => vec![words(&["prune", "--expire=2.weeks.ago"])],
        "repack" => vec![words(&["repack", "-q", "-d", "--cruft"])],
        "commit_graph" => vec![words(&["commit-graph", "write", "--reachable"])],
        other => panic!("no such task {other}"),
    }
}

#[test]
fn each_task_leaves_what_git_leaves() {
    for task in ["gc", "gc_aggressive", "prune", "repack", "commit_graph"] {
        for expire_now in [false, true] {
            let ours = with_unreachable();
            let twin = with_unreachable();
            if expire_now {
                // What nothing reaches may go at once.
                for copy in [&ours, &twin] {
                    copy.git().run(["config", "gc.pruneExpire", "now"]).unwrap();
                }
            }
            // The twin's prune expires what the configuration says, as the
            // command's does.
            let mut twin_steps = by_git(task);
            if task == "prune" && expire_now {
                twin_steps = vec![vec!["prune".to_owned(), "--expire=now".to_owned()]];
            }
            if task == "repack"
                && plain_git_code(ours.path(), &["config", "--get", "--bool", "gc.cruftPacks"]).1
                    == "false\n"
            {
                twin_steps = vec![vec![
                    "repack".to_owned(),
                    "-q".to_owned(),
                    "-d".to_owned(),
                    "-A".to_owned(),
                ]];
            }
            let before = counted(ours.path());
            let mut engine = Engine::at(ours.path());
            let done = engine.run("repo.maintenance", json!({ "task": task }));
            for step in twin_steps {
                let step: Vec<&str> = step.iter().map(String::as_str).collect();
                plain_git(twin.path(), &step);
            }
            let what = format!("{task} (expire now: {expire_now})");
            assert_eq!(references(ours.path()), references(twin.path()), "{what}");
            assert_eq!(objects(ours.path()), objects(twin.path()), "{what}");
            let after = counted(ours.path());
            let twin_after = counted(twin.path());
            for key in ["count", "in-pack", "packs", "prune-packable", "garbage"] {
                assert_eq!(after.get(key), twin_after.get(key), "{what}: {key}");
            }
            // What it says was there is what git counts.
            assert_eq!(done["task"], task);
            assert_eq!(done["before"]["loose"], json!(before["count"]), "{what}");
            assert_eq!(done["before"]["packed"], json!(before["in-pack"]), "{what}");
            assert_eq!(
                done["before"]["loose_bytes"],
                json!(before["size"] * 1024),
                "{what}"
            );
            assert_eq!(done["after"]["loose"], json!(after["count"]), "{what}");
            assert_eq!(done["after"]["packs"], json!(after["packs"]), "{what}");
            let total = |counts: &BTreeMap<String, u64>| {
                (counts["size"] + counts["size-pack"] + counts["size-garbage"]) as i64 * 1024
            };
            assert_eq!(
                done["freed"].as_i64().unwrap(),
                total(&before) - total(&after),
                "{what}"
            );
            if task == "commit_graph" {
                let graph = ours.path().join(".git/objects/info");
                assert!(
                    graph.join("commit-graph").exists() || graph.join("commit-graphs").is_dir(),
                    "no commit graph was written"
                );
            }
        }
    }
}

#[test]
fn gc_with_expiry_now_frees_what_nothing_reaches() {
    let ours = with_unreachable();
    ours.git().run(["config", "gc.pruneExpire", "now"]).unwrap();
    let unreachable_before = objects(ours.path()).len();
    let stages = Arc::new(Mutex::new(Vec::new()));
    let mut engine = Engine::at(ours.path());
    let sink = stages.clone();
    engine.ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
        sink.lock().unwrap().push(progress.stage);
    }));
    let done = engine.run("repo.maintenance", json!({ "task": "gc" }));
    assert!(objects(ours.path()).len() < unreachable_before);
    assert_eq!(done["after"]["loose"], 0);
    assert!(done["after"]["packs"].as_u64().unwrap() >= 1);
    // It said what it was doing, though git says nothing.
    assert!(
        stages
            .lock()
            .unwrap()
            .iter()
            .any(|stage| stage == "Collecting garbage")
    );
}

/// The ids of the lines of `git fsck` that start with `word`.
fn ids_after(text: &str, word: &str) -> BTreeSet<String> {
    text.lines()
        .filter_map(|line| line.strip_prefix(word))
        .filter_map(|rest| rest.split_whitespace().nth(1))
        .map(str::to_owned)
        .collect()
}

fn ids_in(findings: &Value, kind: &str) -> BTreeSet<String> {
    findings
        .as_array()
        .unwrap()
        .iter()
        .filter(|finding| finding["kind"] == kind)
        .filter_map(|finding| finding["id"].as_str())
        .map(str::to_owned)
        .collect()
}

#[test]
fn the_check_finds_what_git_fsck_finds() {
    // Sound, with dangling objects.
    let copy = with_unreachable();
    let mut engine = Engine::at(copy.path());
    let check = engine.run("repo.fsck", json!({}));
    let (code, said, _) = plain_git_code(copy.path(), &["fsck"]);
    assert_eq!(code, 0);
    assert_eq!(check["ok"], true, "{check}");
    assert_eq!(check["finished"], true);
    assert_eq!(check["problem_count"], 0);
    assert_eq!(
        ids_in(&check["dangling"], "dangling"),
        ids_after(&said, "dangling ")
    );
    assert_eq!(
        check["dangling_count"].as_u64().unwrap() as usize,
        ids_after(&said, "dangling ").len()
    );
    assert!(check["dangling_count"].as_u64().unwrap() >= 2);

    // A blob of history taken away: the fixture keeps its objects loose.
    let blob = hermetic(copy.path())
        .run(["rev-parse", "HEAD:README.md"])
        .unwrap();
    let blob = blob.trim();
    let file = copy
        .path()
        .join(".git/objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    assert!(file.exists(), "the fixture's objects are loose");
    std::fs::remove_file(&file).unwrap();
    let check = engine.run("repo.fsck", json!({}));
    let (code, said, said_err) = plain_git_code(copy.path(), &["fsck"]);
    assert_ne!(code, 0);
    assert_eq!(check["ok"], false);
    let all = format!("{said}{said_err}");
    assert_eq!(
        ids_in(&check["problems"], "missing"),
        ids_after(&all, "missing ")
    );
    assert!(ids_in(&check["problems"], "missing").contains(blob));

    // An object file that is empty: damaged, not missing.
    let tree = hermetic(copy.path())
        .run(["rev-parse", "HEAD^{tree}"])
        .unwrap();
    let tree = tree.trim();
    let file = copy
        .path()
        .join(".git/objects")
        .join(&tree[..2])
        .join(&tree[2..]);
    let mut permissions = std::fs::metadata(&file).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    std::fs::set_permissions(&file, permissions).unwrap();
    std::fs::write(&file, b"").unwrap();
    let check = engine.run("repo.fsck", json!({}));
    assert_eq!(check["ok"], false);
    assert!(check["problem_count"].as_u64().unwrap() >= 2, "{check}");
}

/// Every blob of history with its size, largest first, as git lists them.
fn blobs_by_git(dir: &Path) -> Vec<(u64, String)> {
    let listed = plain_git(dir, &["rev-list", "--objects", "--all"]);
    let ids: String = listed
        .lines()
        .filter_map(|line| line.split(' ').next())
        .map(|id| format!("{id}\n"))
        .collect();
    let mut child = Command::new("git")
        .current_dir(dir)
        .args([
            "cat-file",
            "--batch-check=%(objecttype) %(objectname) %(objectsize)",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(ids.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    let mut blobs: Vec<(u64, String)> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            let mut words = line.split(' ');
            (words.next()? == "blob").then_some(())?;
            let id = words.next()?.to_owned();
            Some((words.next()?.parse().ok()?, id))
        })
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    blobs.sort_by(|one, other| other.cmp(one));
    blobs
}

#[test]
fn the_size_is_what_git_counts_and_lists() {
    let copy = with_unreachable();
    // A large blob that is in no commit is no blob of history.
    let mut child = hermetic(copy.path())
        .command()
        .args(["hash-object", "-w", "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&[b'x'; 5000])
        .unwrap();
    assert!(child.wait().unwrap().success());
    // A linked worktree, and an LFS store with two objects in it.
    let linked = copy.path().with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    hermetic(copy.path())
        .run([
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "feature/ui",
        ])
        .unwrap();
    let store = copy.path().join(".git/lfs/objects/ab/cd");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join(format!("abcd{}", "0".repeat(60))), [1u8; 300]).unwrap();
    std::fs::write(store.join(format!("abcd{}", "1".repeat(60))), [2u8; 200]).unwrap();

    let mut engine = Engine::at(copy.path());
    let size = engine.run("repo.size", json!({ "blobs": 4 }));
    let counts = counted(copy.path());
    assert_eq!(size["objects"]["loose"], json!(counts["count"]));
    assert_eq!(size["objects"]["packed"], json!(counts["in-pack"]));
    assert_eq!(
        size["objects"]["pack_bytes"],
        json!(counts["size-pack"] * 1024)
    );

    // Objects by type, as git lists every object there is.
    let mut by_type: BTreeMap<String, u64> = BTreeMap::new();
    for line in plain_git(
        copy.path(),
        &[
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objecttype)",
        ],
    )
    .lines()
    {
        *by_type.entry(line.to_owned()).or_default() += 1;
    }
    for (kind, field) in [
        ("commit", "commits"),
        ("tree", "trees"),
        ("blob", "blobs"),
        ("tag", "tags"),
    ] {
        assert_eq!(
            size["by_type"][field]["count"],
            json!(by_type.get(kind).copied().unwrap_or(0)),
            "{kind}"
        );
    }

    // The largest blobs of history, and where each came in.
    let by_git: Vec<(u64, String)> = blobs_by_git(copy.path()).into_iter().take(4).collect();
    let ours: Vec<(u64, String)> = size["largest"]
        .as_array()
        .unwrap()
        .iter()
        .map(|blob| {
            (
                blob["size"].as_u64().unwrap(),
                blob["id"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(ours, by_git);
    assert_eq!(size["walk_complete"], true);
    assert_eq!(
        size["walked"],
        json!(hermetic(copy.path()).rev_list_count("--all").unwrap())
    );
    for blob in size["largest"].as_array().unwrap() {
        let id = blob["id"].as_str().unwrap();
        let commits = plain_git(
            copy.path(),
            &[
                "log",
                "--all",
                "--no-renames",
                "--format=%H",
                &format!("--find-object={id}"),
            ],
        );
        let oldest = commits.lines().last().unwrap();
        assert_eq!(blob["commit"], oldest, "{id}");
        // The path it has there is one the blob is at.
        let path = blob["path"].as_str().unwrap();
        let at = plain_git(copy.path(), &["rev-parse", &format!("{oldest}:{path}")]);
        assert_eq!(at.trim(), id);
        let summary = plain_git(copy.path(), &["log", "-1", "--format=%s", oldest]);
        assert_eq!(blob["summary"], summary.trim_end());
    }
    // The largest is the big view, which came in on the branch.
    assert_eq!(size["largest"][0]["path"], "src/ui/big.rs");

    // The worktrees and their files, and the LFS store.
    let worktrees = size["worktrees"].as_array().unwrap();
    assert_eq!(worktrees.len(), 2);
    for (place, tree) in [
        (copy.path().to_path_buf(), &worktrees[0]),
        (linked.clone(), &worktrees[1]),
    ] {
        let tracked = hermetic(&place).lines(["ls-files"]).unwrap();
        let bytes: u64 = tracked
            .iter()
            .map(|file| std::fs::metadata(place.join(file)).unwrap().len())
            .sum();
        assert_eq!(
            tree["files"]["count"],
            json!(tracked.len()),
            "{}",
            place.display()
        );
        assert_eq!(tree["files"]["bytes"], json!(bytes), "{}", place.display());
        assert_eq!(tree["files"]["complete"], true);
    }
    assert_eq!(worktrees[0]["is_main"], true);
    assert_eq!(worktrees[1]["branch"], "feature/ui");
    assert_eq!(size["lfs"]["count"], 2);
    assert_eq!(size["lfs"]["bytes"], 500);
    let _ = std::fs::remove_dir_all(&linked);
}

#[test]
fn the_walk_stops_at_its_limit_and_says_so() {
    let copy = Scratch::of("layout").unwrap();
    let mut engine = Engine::at(copy.path());
    let size = engine.run("repo.size", json!({ "blobs": 2, "commits": 1 }));
    assert_eq!(size["walked"], 1);
    assert_eq!(size["walk_complete"], false);
    assert_eq!(size["largest"].as_array().unwrap().len(), 2);
    // No walk at all: the largest blobs without where they came in.
    let size = engine.run("repo.size", json!({ "blobs": 3, "commits": 0 }));
    assert_eq!(size["walked"], 0);
    assert_eq!(size["largest"].as_array().unwrap().len(), 3);
    assert!(size["largest"][0]["path"].is_null());
    let none = engine.run("repo.size", json!({ "blobs": 0 }));
    assert!(none["largest"].as_array().unwrap().is_empty());
}

#[test]
fn a_task_or_a_measure_that_is_stopped_says_so() {
    let copy = with_unreachable();
    let mut engine = Engine::at(copy.path());
    for (id, params) in [
        ("repo.size", json!({})),
        ("repo.fsck", json!({})),
        ("repo.maintenance", json!({ "task": "gc" })),
    ] {
        // Stopped as soon as it says what it is doing.
        let cancel = engine.ctx.cancel.clone();
        engine.ctx.progress = Some(Arc::new(move |_: bitbraid_git::Progress| {
            cancel.store(true, Ordering::SeqCst)
        }));
        let err = engine.try_run(id, params).unwrap_err();
        assert!(err.contains("cancelled"), "{id}: {err}");
        engine.ctx.cancel.store(false, Ordering::SeqCst);
        engine.ctx.progress = None;
    }
    // Nothing was lost on the way.
    let check = engine.run("repo.fsck", json!({}));
    assert_eq!(check["ok"], true, "{check}");
}

/// A place of this test's own.
fn playground(name: &str) -> PathBuf {
    let dir = scratch_root().join(format!("maintenance-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn hostile_parameters_and_repositories_do_no_harm() {
    let copy = Scratch::of("layout").unwrap();
    let before = (references(copy.path()), objects(copy.path()));
    let mut engine = Engine::at(copy.path());
    for task in [
        json!(""),
        json!("GC"),
        json!("gc --aggressive"),
        json!("--auto"),
        json!("fsck"),
        json!(5),
        json!(null),
        json!({ "gc": true }),
        json!(["gc"]),
    ] {
        assert!(
            engine
                .try_run("repo.maintenance", json!({ "task": task }))
                .is_err(),
            "{task}"
        );
    }
    assert!(engine.try_run("repo.maintenance", json!({})).is_err());
    for (blobs, commits) in [
        (json!(-1), json!(10)),
        (json!(101), json!(10)),
        (json!(1e30), json!(10)),
        (json!("ten"), json!(10)),
        (json!(5), json!(-5)),
        (json!(5), json!(u64::MAX)),
        (json!(5), json!(3_000_000)),
        (json!(5), json!("all")),
    ] {
        assert!(
            engine
                .try_run("repo.size", json!({ "blobs": blobs, "commits": commits }))
                .is_err(),
            "{blobs} {commits}"
        );
    }
    assert_eq!((references(copy.path()), objects(copy.path())), before);

    // An empty repository: nothing to measure, nothing to check.
    let dir = playground("empty");
    plain_git(&dir, &["init", "-q", "empty"]);
    let mut engine = Engine::at(&dir.join("empty"));
    let size = engine.run("repo.size", json!({}));
    assert!(size["largest"].as_array().unwrap().is_empty());
    assert_eq!(size["walked"], 0);
    assert_eq!(engine.run("repo.fsck", json!({}))["ok"], true);
    for task in ["gc", "prune", "repack", "commit_graph"] {
        engine.run("repo.maintenance", json!({ "task": task }));
    }

    // Garbage in the object store, and an object file that is empty.
    let broken = Scratch::of("linear").unwrap();
    let objects_dir = broken.path().join(".git/objects");
    std::fs::write(objects_dir.join("garbage.txt"), b"not an object").unwrap();
    std::fs::create_dir_all(objects_dir.join("zz")).unwrap();
    std::fs::write(objects_dir.join("zz/notahexname"), b"").unwrap();
    std::fs::create_dir_all(objects_dir.join("ab")).unwrap();
    std::fs::write(objects_dir.join("ab").join("c".repeat(38)), b"").unwrap();
    // What git counts as garbage: a file among the objects that is none.
    std::fs::write(objects_dir.join("ab/not-an-object"), b"garbage").unwrap();
    let mut engine = Engine::at(broken.path());
    let size = engine.try_run("repo.size", json!({}));
    if let Ok(size) = &size {
        assert!(size["objects"]["garbage"].as_u64().unwrap() >= 1, "{size}");
    }
    let check = engine.run("repo.fsck", json!({}));
    assert_eq!(check["ok"], false, "{check}");
    for task in ["gc", "gc_aggressive", "prune", "repack", "commit_graph"] {
        // Git may refuse; nothing may panic.
        let _ = engine.try_run("repo.maintenance", json!({ "task": task }));
    }
    let _ = std::fs::remove_dir_all(&dir);
}
