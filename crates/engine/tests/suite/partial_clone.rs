//! Partial clones against `git clone --filter`: a clone that
//! leaves the content of files, or trees too, on the remote has the same
//! references, the same objects missing and the same settings as git's
//! own; a fetch keeps the filter, so what it brings leaves out what the
//! twin's `git fetch` leaves out; and a remote that does not filter is
//! said to have sent everything.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    process::Command,
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir, scratch_root};
use serde_json::{Value, json};

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

fn hermetic(dir: &Path) -> Git {
    let git = Git::new(dir, fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git
}

/// A directory of this test's own, empty.
fn playground(name: &str) -> PathBuf {
    let dir = scratch_root().join(format!("partial-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    bitbraid_core::paths::canonical(dir).unwrap()
}

struct Engine {
    registry: Registry,
    ctx: CommandContext,
}

impl Engine {
    fn new() -> Self {
        Self {
            registry: builtin_registry(),
            ctx: CommandContext::default(),
        }
    }
    fn at(path: &Path) -> Self {
        let mut engine = Self::new();
        engine.run("repo.open", json!({ "path": path }));
        engine
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

/// The objects history refers to that are not here.
fn missing(dir: &Path) -> BTreeSet<String> {
    plain_git(dir, &["rev-list", "--objects", "--all", "--missing=print"])
        .lines()
        .filter_map(|line| line.strip_prefix('?'))
        .map(str::to_owned)
        .collect()
}

fn references(dir: &Path) -> Vec<String> {
    plain_git(dir, &["for-each-ref", "--format=%(refname) %(objectname)"])
        .lines()
        .map(str::to_owned)
        .collect()
}

/// What makes a clone partial: the settings of its remote.
fn settings(dir: &Path) -> Vec<String> {
    plain_git(dir, &["config", "--list", "--local"])
        .lines()
        .filter(|line| {
            line.starts_with("remote.origin.")
                || line.starts_with("extensions.")
                || line.contains("sparse")
        })
        .filter(|line| !line.starts_with("remote.origin.url="))
        .map(str::to_owned)
        .collect()
}

/// A source that lets a filter be asked for.
fn source() -> Scratch {
    let source = Scratch::of("layout").unwrap();
    source
        .git()
        .run(["config", "uploadpack.allowFilter", "true"])
        .unwrap();
    source
}

#[test]
fn a_partial_clone_is_what_git_makes() {
    let source = source();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("filters");
    let mut engine = Engine::new();
    for (filter, sparse) in [("blob:none", false), ("tree:0", false), ("blob:none", true)] {
        let name = format!("{}-{sparse}", filter.replace(':', "-"));
        let ours = dir.join(format!("ours-{name}"));
        let twin = dir.join(format!("twin-{name}"));
        let mut params = json!({ "url": url, "path": ours, "filter": filter });
        if sparse {
            params["sparse"] = json!(true);
        }
        let out = engine.run("repo.clone", params);
        assert_eq!(out["filter"], filter);
        assert_eq!(out["partial"], true);
        let mut args = vec!["clone", "-q", "--no-local"];
        let filter_arg = format!("--filter={filter}");
        args.push(&filter_arg);
        if sparse {
            args.push("--sparse");
        }
        args.extend([url.as_str(), twin.to_str().unwrap()]);
        plain_git(&dir, &args);
        assert_eq!(references(&ours), references(&twin), "{name}");
        assert_eq!(settings(&ours), settings(&twin), "{name}");
        assert_eq!(missing(&ours), missing(&twin), "{name}");
        assert!(!missing(&ours).is_empty(), "{name}: nothing was left out");
        assert_eq!(
            plain_git(&ours, &["ls-files", "-t"]),
            plain_git(&twin, &["ls-files", "-t"]),
            "{name}"
        );
        if sparse {
            assert!(!ours.join("src").exists());
            assert!(ours.join("README.md").exists());
        }
    }

    // A fetch keeps the filter: what it brings that history needs only
    // later is left out, as git's own fetch leaves it out.
    let source_git = source.git();
    source
        .write("src/core/lib.rs", "pub fn core() { new(); }")
        .unwrap();
    source.write("docs/new.md", "new").unwrap();
    source_git.run(["add", "-A"]).unwrap();
    source_git.run(["commit", "-q", "-m", "more"]).unwrap();
    source_git
        .run(["commit", "-q", "--allow-empty", "-m", "after"])
        .unwrap();
    let ours = dir.join("ours-blob-none-false");
    let twin = dir.join("twin-blob-none-false");
    Engine::at(&ours).run("remote.fetch", json!({ "remote": "origin" }));
    plain_git(&twin, &["fetch", "-q", "origin"]);
    assert_eq!(references(&ours), references(&twin));
    assert_eq!(missing(&ours), missing(&twin));
    let new_blob = hermetic(source.path())
        .run(["rev-parse", "HEAD:docs/new.md"])
        .unwrap();
    assert!(missing(&ours).contains(new_blob.trim()));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_remote_that_does_not_filter_is_said_to_have_sent_everything() {
    // No `uploadpack.allowFilter`: git warns and clones everything.
    let source = Scratch::of("layout").unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("unfiltered");
    let ours = dir.join("ours");
    let out = Engine::new().run(
        "repo.clone",
        json!({ "url": url, "path": ours, "filter": "blob:none" }),
    );
    assert_eq!(out["partial"], false);
    assert!(missing(&ours).is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

/// What a diff says of each file: path, lines added, lines taken out.
fn numstat_by_us(diff: &Value) -> Vec<(String, u64, u64)> {
    let mut out: Vec<_> = diff
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap().to_owned(),
                file["additions"].as_u64().unwrap(),
                file["deletions"].as_u64().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn numstat_by_git(dir: &Path, old: &str, new: &str) -> Vec<(String, u64, u64)> {
    let mut out: Vec<_> = plain_git(dir, &["diff", "--numstat", "--no-renames", old, new])
        .lines()
        .map(|line| {
            let mut words = line.split('\t');
            let added = words.next().unwrap().parse().unwrap();
            let removed = words.next().unwrap().parse().unwrap();
            (words.next().unwrap().to_owned(), added, removed)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn what_a_diff_needs_is_fetched_from_the_remote() {
    let source = source();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("lazy");
    for filter in ["blob:none", "tree:0"] {
        let ours = dir.join(filter.replace(':', "-"));
        Engine::new().run(
            "repo.clone",
            json!({ "url": url, "path": ours, "filter": filter }),
        );
        let before = missing(&ours);
        assert!(!before.is_empty());
        let mut engine = Engine::at(&ours);
        // The guide as it was laid out is not here: only what is checked
        // out came with the clone.
        let diff = engine.run(
            "repo.diff",
            json!({ "old": "HEAD~4", "new": "HEAD~3", "detect_renames": false }),
        );
        assert_eq!(
            numstat_by_us(&diff),
            numstat_by_git(&ours, "HEAD~4", "HEAD~3"),
            "{filter}"
        );
        assert!(missing(&ours).len() < before.len(), "{filter}");
        // And a commit's files with renames looked for, as the details
        // panel asks.
        let diff = engine.run("repo.diff", json!({ "old": "HEAD~3", "new": "HEAD~2" }));
        assert_eq!(
            numstat_by_us(&diff),
            numstat_by_git(&ours, "HEAD~3", "HEAD~2"),
            "{filter}"
        );
    }
    // A file's history and its blame, from a clone that has no old
    // content at all.
    let ours = dir.join("history");
    Engine::new().run(
        "repo.clone",
        json!({ "url": url, "path": ours, "filter": "blob:none" }),
    );
    let mut engine = Engine::at(&ours);
    // A file as the first commit had it, which only the remote has.
    let bytes = engine.run(
        "file.bytes",
        json!({ "path": "docs/guide.md", "side": "HEAD~4" }),
    );
    let first = plain_git(&ours, &["show", "HEAD~4:docs/guide.md"]);
    assert_eq!(bytes["exists"], true);
    assert_eq!(bytes["size"], json!(first.len()));
    // The first commit against the worktree.
    let mixed = engine.run(
        "repo.diff",
        json!({ "old": "HEAD~4", "new": "worktree", "detect_renames": false }),
    );
    let mut by_git: Vec<(String, u64, u64)> =
        plain_git(&ours, &["diff", "--numstat", "--no-renames", "HEAD~4"])
            .lines()
            .map(|line| {
                let mut words = line.split('\t');
                let added = words.next().unwrap().parse().unwrap();
                let removed = words.next().unwrap().parse().unwrap();
                (words.next().unwrap().to_owned(), added, removed)
            })
            .collect();
    by_git.sort();
    assert_eq!(numstat_by_us(&mixed), by_git);
    let history = engine.run("repo.history", json!({ "path": "docs/guide.md" }));
    let ours_commits: Vec<String> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["commit"]["id"].as_str().unwrap().to_owned())
        .collect();
    let git_commits: Vec<String> = plain_git(
        &ours,
        &["log", "--format=%H", "--follow", "--", "docs/guide.md"],
    )
    .lines()
    .map(str::to_owned)
    .collect();
    assert_eq!(ours_commits, git_commits);
    let blame = engine.run(
        "repo.blame",
        json!({ "path": "docs/guide.md", "rev": "HEAD", "ignore_revs": false }),
    );
    let ours_lines: Vec<String> = blame
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["commit"].as_str().unwrap().to_owned())
        .collect();
    let git_lines: Vec<String> =
        plain_git(&ours, &["blame", "-s", "-l", "HEAD", "--", "docs/guide.md"])
            .lines()
            .map(|line| line.get(..40).unwrap().trim_start_matches('^').to_owned())
            .collect();
    assert_eq!(ours_lines.len(), git_lines.len());
    for (ours, git) in ours_lines.iter().zip(&git_lines) {
        assert!(ours.starts_with(git.as_str()), "{ours} {git}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// A partial clone whose remote is gone: what is not here cannot be
/// fetched, and a read that needs it fails.
#[test]
fn a_partial_clone_without_its_remote_says_what_is_missing() {
    let source = source();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("orphaned");
    let ours = dir.join("ours");
    Engine::new().run(
        "repo.clone",
        json!({ "url": url, "path": ours, "filter": "blob:none" }),
    );
    plain_git(
        &ours,
        &[
            "remote",
            "set-url",
            "origin",
            dir.join("gone").to_str().unwrap(),
        ],
    );
    let mut engine = Engine::at(&ours);
    for (id, params) in [
        ("repo.diff", json!({ "old": "HEAD~4", "new": "HEAD~3" })),
        ("repo.diff", json!({ "old": "HEAD~4", "new": "worktree" })),
        (
            "file.bytes",
            json!({ "path": "docs/guide.md", "side": "HEAD~4" }),
        ),
        (
            "repo.blame",
            json!({ "path": "docs/guide.md", "rev": "HEAD~3" }),
        ),
    ] {
        assert!(engine.try_run(id, params.clone()).is_err(), "{id} {params}");
    }
    // What is here is read as ever: the worktree against HEAD, and a
    // file's history, which needs its trees and no content while
    // nothing was moved.
    engine.run("repo.diff", json!({ "old": "HEAD", "new": "worktree" }));
    engine.run("repo.history", json!({ "path": "docs/guide.md" }));
    let _ = std::fs::remove_dir_all(&dir);
}

/// The packs of a repository: how many there are.
#[cfg(unix)]
fn packs(dir: &Path) -> usize {
    std::fs::read_dir(dir.join(".git/objects/pack"))
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
        .count()
}

/// Have the remote of the clone at `ours` note every time it is asked for
/// something, in `log`: its `upload-pack` is a script that writes a line
/// there first (git-config(1) `remote.<name>.uploadpack`).
#[cfg(unix)]
fn count_asking(ours: &Path, log: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    let script = log.with_extension("sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\necho asked >> '{}'\nexec git upload-pack \"$@\"\n",
            log.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    plain_git(
        ours,
        &[
            "config",
            "remote.origin.uploadpack",
            script.to_str().unwrap(),
        ],
    );
}

/// How many times the remote was asked for something.
#[cfg(unix)]
fn asked(log: &Path) -> usize {
    std::fs::read_to_string(log)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

/// What nobody asked for reaches no remote: in a clone that left the trees
/// of history on its remote, the trial merge the conflict marks run for
/// every local branch answers that it cannot tell, the files of an old
/// commit say that they were left there, and the size is measured from what
/// is here, without the remote being asked once and without a pack being
/// added. The same trial merge asked for by the user fetches what it needs,
/// as git does; done for the marks in the background, that would add packs
/// all the time.
#[cfg(unix)]
#[test]
fn what_nobody_asked_for_reaches_no_remote() {
    let source = source();
    // A branch beside the checked-out one, which changes a file in a
    // folder: its trial merge needs trees a treeless clone does not have.
    let git = source.git();
    git.run(["checkout", "-q", "-b", "side", "HEAD~2"]).unwrap();
    std::fs::write(
        source.path().join("src/core/lib.rs"),
        "pub fn core() {}\npub fn side() {}\n",
    )
    .unwrap();
    git.run(["commit", "-q", "-am", "a change on the side"])
        .unwrap();
    git.run(["checkout", "-q", "main"]).unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("unasked");
    let ours = dir.join("ours");
    Engine::new().run(
        "repo.clone",
        json!({ "url": url, "path": ours, "filter": "tree:0" }),
    );
    let log = dir.join("asked.log");
    count_asking(&ours, &log);
    let before = packs(&ours);
    let mut engine = Engine::at(&ours);
    let mut unasked = |id: &str, params: Value| {
        engine
            .registry
            .run_unasked(id, &mut engine.ctx, &params)
            .map_err(|err| err.to_string())
    };
    let marked = unasked(
        "branch.predict",
        json!({ "target": "origin/side", "aside": true }),
    )
    .unwrap();
    assert_eq!(marked["known"], false, "{marked}");
    assert_eq!(asked(&log), 0, "the trial merge asked the remote");
    let listed = unasked("repo.diff", json!({ "old": "HEAD~3", "new": "HEAD~2" })).unwrap_err();
    assert!(
        listed.contains(bitbraid_git::promisor::left_on_remote()),
        "{listed}"
    );
    assert_eq!(
        asked(&log),
        0,
        "the files of an old commit asked the remote"
    );
    let measured = unasked("repo.size", json!({})).unwrap();
    assert!(
        measured["objects"]["packed"].as_u64().unwrap() > 0,
        "{measured}"
    );
    // Where the largest came in: as far as history is here, and said so.
    assert_eq!(measured["walk_left_on_remote"], true, "{measured}");
    assert_eq!(measured["walk_complete"], false, "{measured}");
    assert_eq!(asked(&log), 0, "measuring asked the remote");
    assert_eq!(packs(&ours), before, "no pack came");
    // Asked for, the trial merge fetches what it needs, as git would.
    let foretold = engine.run(
        "branch.predict",
        json!({ "target": "origin/side", "aside": true }),
    );
    assert_eq!(foretold["known"], true, "{foretold}");
    assert!(asked(&log) > 0, "the remote was asked");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn hostile_filters_are_refused_and_make_nothing() {
    let source = source();
    let url = source.path().to_str().unwrap().to_owned();
    let dir = playground("hostile");
    let mut engine = Engine::new();
    let ours = dir.join("ours");
    for filter in [
        json!(""),
        json!("blob:limit=1"),
        json!("tree:1"),
        json!("BLOB:NONE"),
        json!("--upload-pack=touch /tmp/x"),
        json!("blob:none --bare"),
        json!(5),
        json!(true),
        json!(["blob:none"]),
        json!({ "blob": "none" }),
    ] {
        assert!(
            engine
                .try_run(
                    "repo.clone",
                    json!({ "url": url, "path": ours, "filter": filter })
                )
                .is_err(),
            "{filter}"
        );
        assert!(!ours.exists(), "{filter}");
    }
    for sparse in [json!("yes"), json!(1), json!([])] {
        assert!(
            engine
                .try_run(
                    "repo.clone",
                    json!({ "url": url, "path": ours, "sparse": sparse })
                )
                .is_err()
        );
    }
    assert!(!ours.exists());
    let _ = std::fs::remove_dir_all(&dir);
}
