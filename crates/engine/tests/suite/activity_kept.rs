//! The activity log between runs: what a command did is in a
//! file of the application's own for the repository, never in the
//! repository; it is read back by a process that did not write it, bounded,
//! and a file that is not what was written is survived.
//!
//! A second run is stood in for by forgetting what the running process
//! remembers (`bitbraid_git::activity::forget_repository`): what the list
//! has then, it has from the file. The command line's tests run two
//! processes for real.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_git::activity;
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    /// The application's configuration directory: beside the copy.
    config: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let config = scratch.path().with_extension("config");
        let _ = std::fs::remove_dir_all(&config);
        let git = scratch.git();
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        let registry = builtin_registry();
        let mut ctx = CommandContext {
            config_dir: Some(config.clone()),
            ..Default::default()
        };
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            config,
            registry,
            ctx,
        }
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    /// Wait for the writing thread, then list.
    fn log(&mut self, params: Value) -> Vec<Value> {
        assert!(activity::flush(Duration::from_secs(20)));
        self.run("activity.list", params)["entries"]
            .as_array()
            .unwrap()
            .clone()
    }
    /// As a process started now would see it: nothing remembered.
    fn another_run(&self) {
        assert!(activity::flush(Duration::from_secs(20)));
        activity::forget_repository(self.ctx.repo.as_ref().unwrap());
    }
    /// The files the log is kept in.
    fn files(&self) -> Vec<PathBuf> {
        let mut found: Vec<PathBuf> = std::fs::read_dir(self.config.join("activity"))
            .map(|listing| listing.flatten().map(|entry| entry.path()).collect())
            .unwrap_or_default();
        found.sort();
        found
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.config);
    }
}

fn commands(entries: &[Value]) -> Vec<&str> {
    entries
        .iter()
        .map(|entry| entry["command"].as_str().unwrap())
        .collect()
}

/// Every file under `dir`, as relative paths.
fn listing(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                // With `/` between folders, as git writes paths.
                let relative = path.strip_prefix(root).unwrap().to_string_lossy();
                out.push(relative.replace(std::path::MAIN_SEPARATOR, "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn what_was_done_is_there_in_the_next_run_and_what_was_asked_is_not() {
    let mut h = Harness::new();
    let in_repository = listing(h.scratch.path());
    h.run("tag.create", json!({ "name": "kept-one" }));
    h.run("branch.create", json!({ "name": "kept-two" }));
    h.run("repo.status", json!({}));
    let first = h.log(json!({ "questions": true, "limit": 2000 }));
    let done: Vec<Value> = first
        .iter()
        .filter(|entry| entry["question"] == false)
        .cloned()
        .collect();
    assert!(done.len() >= 2, "{:?}", commands(&first));
    assert!(first.len() > done.len(), "there were questions too");
    assert!(commands(&done).iter().any(|c| c.contains("kept-one")));

    // One file, in the application's directory; nothing in the repository
    // but what the two commands made there.
    let files = h.files();
    assert_eq!(files.len(), 1, "{files:?}");
    assert!(files[0].starts_with(&h.config));
    let now: Vec<String> = listing(h.scratch.path())
        .into_iter()
        .filter(|path| !in_repository.contains(path))
        .collect();
    assert!(
        now.iter().all(|path| {
            path.starts_with(".git/refs/")
                || path.starts_with(".git/logs/")
                || path == ".git/bitbraid/journal.json"
        }),
        "{now:?}"
    );

    // The next run: what did something, entry for entry as it was; what
    // was only asked was the running application's.
    h.another_run();
    let second = h.log(json!({ "questions": true, "limit": 2000 }));
    let kept: Vec<Value> = second
        .iter()
        .filter(|entry| done.iter().any(|before| before["id"] == entry["id"]))
        .cloned()
        .collect();
    assert_eq!(kept, done, "the same entries, in the same order");
    let asked_before: Vec<&Value> = first
        .iter()
        .filter(|entry| entry["question"] == true)
        .collect();
    assert!(
        asked_before
            .iter()
            .all(|asked| second.iter().all(|entry| entry["id"] != asked["id"])),
        "questions are not kept"
    );
    assert_eq!(kept[0]["origin"], "branch.create");
    assert_eq!(kept[0]["end"], "exited");
    assert_eq!(kept[0]["code"], 0);

    // The filter and the limit go over what is kept like over the rest.
    let found = h.log(json!({ "text": "KEPT-ONE" }));
    assert_eq!(found.len(), 1, "{:?}", commands(&found));
    assert_eq!(h.log(json!({ "limit": 1 })).len(), 1);

    // What comes now is later than what was kept, and listed first.
    h.run("tag.delete", json!({ "name": "kept-one" }));
    let third = h.log(json!({}));
    assert!(third[0]["command"].as_str().unwrap().contains("update-ref"));
    let ids: Vec<u64> = third.iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "{ids:?}");
    assert!(third.len() > kept.len());
}

#[test]
fn clearing_forgets_what_was_kept_too() {
    let mut h = Harness::new();
    h.run("tag.create", json!({ "name": "one" }));
    h.run("tag.create", json!({ "name": "two" }));
    let listed = h.log(json!({ "questions": true, "limit": 2000 })).len();
    h.another_run();
    h.run("tag.create", json!({ "name": "three" }));
    let before = h.log(json!({ "questions": true, "limit": 2000 })).len();
    assert!(before >= 3 && before < listed + 20);

    let out = h.run("activity.clear", json!({}));
    assert_eq!(out["cleared"], before);
    assert!(h.log(json!({ "questions": true })).is_empty());
    assert!(h.files().is_empty(), "the file is gone");
    assert_eq!(h.run("activity.clear", json!({}))["cleared"], 0);

    // And the log goes on from there.
    h.run("tag.delete", json!({ "name": "one" }));
    h.another_run();
    let after = h.log(json!({}));
    assert_eq!(after.len(), 1, "{:?}", commands(&after));
    assert_eq!(h.files().len(), 1);
}

#[test]
fn a_file_that_is_not_what_was_written_is_survived() {
    let mut h = Harness::new();
    h.run("tag.create", json!({ "name": "before" }));
    assert_eq!(h.log(json!({})).len(), 1);
    let file = h.files().remove(0);
    let good = std::fs::read(&file).unwrap();
    h.another_run();

    for garbage in [
        Vec::new(),
        vec![0u8, 159, 146, 150, 255],
        b"not json at all\n{\"id\": \"nor this\"}\n[]\nnull\n".to_vec(),
        b"{".repeat(100_000),
        vec![b'\n'; 50_000],
    ] {
        std::fs::write(&file, &garbage).unwrap();
        let listed = h.log(json!({ "questions": true }));
        assert!(
            listed.iter().all(|entry| entry["question"] == true),
            "{:?}",
            commands(&listed)
        );
    }
    // Half of an entry, then a whole one: the whole one is read.
    let mut torn = good[..good.len() / 2].to_vec();
    torn.push(b'\n');
    torn.extend(&good);
    std::fs::write(&file, &torn).unwrap();
    assert_eq!(commands(&h.log(json!({}))).len(), 1);

    // A directory where the file should be, and no directory at all:
    // nothing is kept, and nothing fails.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    h.run("tag.create", json!({ "name": "unkept" }));
    assert_eq!(h.log(json!({})).len(), 1, "the running process has it");
    h.another_run();
    assert!(h.log(json!({})).is_empty());
    std::fs::remove_dir(&file).unwrap();
    // Garbage without an end of line: the next entry starts a line of
    // its own and is read back.
    std::fs::write(&file, b"garbage that ends nowhere").unwrap();
    h.run("tag.create", json!({ "name": "after" }));
    h.another_run();
    let listed = h.log(json!({}));
    assert_eq!(listed.len(), 1, "{:?}", commands(&listed));
    assert!(listed[0]["command"].as_str().unwrap().contains("after"));
}

#[test]
fn without_a_directory_of_the_applications_nothing_is_kept() {
    let scratch = Scratch::of("linear").unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
        .unwrap();
    registry
        .run("tag.create", &mut ctx, &json!({ "name": "unkept" }))
        .unwrap();
    assert!(activity::flush(Duration::from_secs(20)));
    let list = |ctx: &mut CommandContext| {
        registry.run("activity.list", ctx, &json!({})).unwrap()["entries"]
            .as_array()
            .unwrap()
            .len()
    };
    assert_eq!(list(&mut ctx), 1);
    activity::forget_repository(ctx.repo.as_ref().unwrap());
    assert_eq!(list(&mut ctx), 0);
    // Nor did anything appear in the repository for it.
    assert!(
        listing(scratch.path())
            .iter()
            .all(|path| !path.contains("activity") && !path.ends_with(".jsonl"))
    );
}

#[test]
fn init_and_clone_are_in_the_log_of_the_repository_they_made() {
    let scratch = Scratch::of("linear").unwrap();
    let config = scratch.path().with_extension("config");
    let made = scratch.path().with_extension("made");
    let cloned = scratch.path().with_extension("cloned");
    for dir in [&config, &made, &cloned] {
        let _ = std::fs::remove_dir_all(dir);
    }
    let registry = builtin_registry();
    // No repository is open: this is the start page.
    let mut ctx = CommandContext {
        config_dir: Some(config.clone()),
        ..Default::default()
    };
    registry
        .run(
            "repo.init",
            &mut ctx,
            &json!({ "path": made, "branch": "trunk" }),
        )
        .unwrap();
    registry
        .run(
            "repo.clone",
            &mut ctx,
            &json!({ "url": scratch.path(), "path": cloned }),
        )
        .unwrap();
    // One that fails leaves no log for a repository that never was.
    let nowhere = scratch.path().with_extension("nowhere");
    assert!(
        registry
            .run(
                "repo.clone",
                &mut ctx,
                &json!({ "url": scratch.path().with_extension("no-such"), "path": nowhere }),
            )
            .is_err()
    );
    assert!(activity::flush(Duration::from_secs(20)));
    let kept = std::fs::read_dir(config.join("activity")).unwrap().count();
    assert_eq!(kept, 2, "one file for each repository that was made");

    for (path, what) in [(&made, "git init"), (&cloned, "git clone")] {
        let mut ctx = CommandContext {
            config_dir: Some(config.clone()),
            ..Default::default()
        };
        registry
            .run("repo.open", &mut ctx, &json!({ "path": path }))
            .unwrap();
        let list = |ctx: &mut CommandContext| -> Vec<Value> {
            registry.run("activity.list", ctx, &json!({})).unwrap()["entries"]
                .as_array()
                .unwrap()
                .clone()
        };
        // In the run that made it, and in the next.
        for run in ["this run", "the next run"] {
            let listed = list(&mut ctx);
            let entry = listed
                .iter()
                .find(|entry| entry["command"].as_str().unwrap().starts_with(what))
                .unwrap_or_else(|| panic!("{run}: no {what} in {:?}", commands(&listed)));
            assert_eq!(entry["end"], "exited", "{run}");
            assert_eq!(entry["code"], 0, "{run}");
            let origin = if what == "git init" {
                "repo.init"
            } else {
                "repo.clone"
            };
            assert_eq!(entry["origin"], origin, "{run}");
            activity::forget_repository(ctx.repo.as_ref().unwrap());
        }
    }
    // The clone ran beside the repository, not in it: it is the
    // repository's all the same, and nobody else's.
    let mut ours = CommandContext {
        config_dir: Some(config.clone()),
        ..Default::default()
    };
    registry
        .run("repo.open", &mut ours, &json!({ "path": scratch.path() }))
        .unwrap();
    let source = registry
        .run("activity.list", &mut ours, &json!({}))
        .unwrap();
    assert!(
        commands(source["entries"].as_array().unwrap())
            .iter()
            .all(|command| !command.starts_with("git clone")),
        "{source}"
    );
    for dir in [&config, &made, &cloned] {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[test]
fn what_runs_inside_a_submodule_and_in_another_worktree_is_the_repositorys() {
    let mut h = Harness::new();
    let home = fixtures_out_dir().join(".home");
    // A submodule, cloned from a copy on this disk, and a second worktree.
    let source = Scratch::of("linear").unwrap();
    let git = h.scratch.git();
    git.run([
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "add",
        "-q",
        source.path().to_str().unwrap(),
        "sub",
    ])
    .unwrap();
    git.run(["commit", "-q", "-m", "add a submodule"]).unwrap();
    let tree = h.scratch.path().with_extension("tree");
    let _ = std::fs::remove_dir_all(&tree);
    git.run([
        "worktree",
        "add",
        "-q",
        tree.to_str().unwrap(),
        "-b",
        "tree",
    ])
    .unwrap();
    assert!(Git::new(&tree, &home).rev_parse("HEAD").is_ok());

    h.run("submodule.list", json!({}));
    h.run("worktree.list", json!({}));
    let listed = h.log(json!({ "questions": true, "limit": 2000 }));
    let ran_in = |dir: &Path| -> Vec<&Value> {
        let dir = dir.canonicalize().unwrap();
        listed
            .iter()
            .filter(|entry| {
                Path::new(entry["directory"].as_str().unwrap())
                    .canonicalize()
                    .is_ok_and(|ran| ran == dir)
            })
            .collect()
    };
    let inside = ran_in(&h.scratch.path().join("sub"));
    assert!(!inside.is_empty(), "nothing ran inside the submodule");
    assert!(
        inside
            .iter()
            .all(|entry| entry["origin"] == "submodule.list")
    );
    let elsewhere = ran_in(&tree);
    assert!(!elsewhere.is_empty(), "nothing ran in the other worktree");
    assert!(
        elsewhere
            .iter()
            .all(|entry| entry["origin"] == "worktree.list")
    );
    // Each says which repository it was run for.
    let git_dir = bitbraid_core::paths::canonical(h.scratch.path().join(".git")).unwrap();
    for entry in inside.iter().chain(&elsewhere) {
        assert_eq!(
            entry["repository"].as_str().map(Path::new),
            Some(git_dir.as_path()),
            "{entry}"
        );
    }
    let _ = std::fs::remove_dir_all(&tree);
}
