//! Worktree commands against `git worktree` on twin scratch copies: the
//! command runs on one copy, git on the other, and the list of worktrees,
//! what each has checked out, the references and the branch configuration
//! have to come out the same.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

/// One copy and the directories its worktrees go to, which are beside it
/// and removed with it.
struct Side {
    copy: Scratch,
}

impl Side {
    fn new() -> Self {
        let side = Self {
            copy: Scratch::of("branches-merges").unwrap(),
        };
        side.sweep();
        side
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    /// Where the worktree called `name` goes.
    fn wt(&self, name: &str) -> PathBuf {
        self.copy.path().with_extension(format!("wt-{name}"))
    }
    fn git_in(&self, name: &str) -> Git {
        Git::new(self.wt(name), fixtures_out_dir().join(".home"))
    }
    /// A remote named `origin`: a bare clone beside the copy.
    fn add_remote(&self) {
        let bare = self.wt("origin.git");
        let git = self.git();
        git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
            .unwrap();
        git.run(["remote", "add", "origin", bare.to_str().unwrap()])
            .unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
    }
    /// Remove every directory beside the copy that is one of its own.
    fn sweep(&self) {
        let (Some(parent), Some(name)) = (self.copy.path().parent(), self.copy.path().file_name())
        else {
            return;
        };
        let prefix = format!("{}.wt-", name.to_string_lossy());
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    /// With the copy's own path taken out, so that two copies compare: as
    /// the system writes it and as git does (`C:/...` on Windows).
    fn neutral(&self, text: &str) -> String {
        let path = self.copy.path().to_str().unwrap();
        text.replace(path, "<copy>")
            .replace(&git_form(path), "<copy>")
    }
    /// Everything a worktree command can change, as text.
    fn state(&self) -> Vec<String> {
        let git = self.git();
        let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
        let config = git
            .run(["config", "--get-regexp", r"^branch\."])
            .unwrap_or_default();
        out.extend(config.lines().map(|line| format!("config {line}")));
        let listed = git.run(["worktree", "list", "--porcelain"]).unwrap();
        for line in listed.lines().filter(|line| !line.is_empty()) {
            out.push(self.neutral(line));
            // What is in each worktree that is there.
            if let Some(path) = line.strip_prefix("worktree ")
                && Path::new(path).is_dir()
                && Path::new(path) != self.copy.path()
            {
                let inside = Git::new(path, fixtures_out_dir().join(".home"));
                for change in inside.lines(["status", "--porcelain"]).unwrap() {
                    out.push(format!("  {change}"));
                }
                out.push(format!(
                    "  tree {}",
                    inside.rev_parse("HEAD^{tree}").unwrap()
                ));
            }
        }
        out
    }
}

impl Drop for Side {
    fn drop(&mut self) {
        self.sweep();
    }
}

struct Harness {
    ours: Side,
    twin: Side,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let ours = Side::new();
        let twin = Side::new();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": ours.copy.path() }))
            .expect("open scratch");
        Self {
            ours,
            twin,
            registry,
            ctx,
        }
    }
    fn both(&self, f: impl Fn(&Side)) {
        f(&self.ours);
        f(&self.twin);
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    fn refused(&mut self, id: &str, params: Value, because: &str) {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(because), "{id} {params}: {err}");
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(self.ours.state(), self.twin.state(), "{what}");
    }
    fn path(&self, name: &str) -> String {
        self.ours.wt(name).to_str().unwrap().to_owned()
    }
}

/// A full path as git prints it: on Windows with `/` between folders.
fn git_form(path: &str) -> String {
    if cfg!(windows) {
        path.replace('\\', "/")
    } else {
        path.to_owned()
    }
}

/// A full path git printed, as the system writes it, which is how the
/// commands answer with it: on Windows with `\` between folders.
fn system_form(path: &str) -> String {
    if cfg!(windows) {
        path.replace('/', "\\")
    } else {
        path.to_owned()
    }
}

/// `git worktree list --porcelain` read by the test itself: one map of
/// attributes per worktree.
fn listed_by_git(git: &Git) -> Vec<Vec<(String, String)>> {
    let text = git.run(["worktree", "list", "--porcelain"]).unwrap();
    text.split("\n\n")
        .filter(|record| !record.trim().is_empty())
        .map(|record| {
            record
                .lines()
                .map(|line| match line.split_once(' ') {
                    Some((word, value)) => (word.to_owned(), value.to_owned()),
                    None => (line.to_owned(), String::new()),
                })
                .collect()
        })
        .collect()
}

fn attribute<'a>(record: &'a [(String, String)], word: &str) -> Option<&'a str> {
    record
        .iter()
        .find(|(name, _)| name == word)
        .map(|(_, value)| value.as_str())
}

#[test]
fn the_list_says_what_git_says_and_counts_the_changes() {
    let mut h = Harness::new();
    let git = h.ours.git();
    let base = h.ours.copy.path().to_path_buf();
    let wt = |name: &str| base.with_extension(format!("wt-{name}"));
    git.run([
        "worktree",
        "add",
        "-q",
        wt("a").to_str().unwrap(),
        "feature/a",
    ])
    .unwrap();
    git.run([
        "worktree",
        "add",
        "-q",
        "--detach",
        wt("det").to_str().unwrap(),
        "HEAD~1",
    ])
    .unwrap();
    git.run([
        "worktree",
        "add",
        "-q",
        wt("gone").to_str().unwrap(),
        "feature/b",
    ])
    .unwrap();
    git.run([
        "worktree",
        "lock",
        "--reason",
        "on a stick that is unplugged",
        wt("det").to_str().unwrap(),
    ])
    .unwrap();
    std::fs::remove_dir_all(wt("gone")).unwrap();
    // Changes: two in the linked one, one in the main one.
    std::fs::write(wt("a").join("a.txt"), "changed\n").unwrap();
    std::fs::write(wt("a").join("new.txt"), "new\n").unwrap();
    h.ours.copy.write("base.txt", "changed\n").unwrap();

    let ours = h.run("worktree.list", json!({}));
    let ours = ours.as_array().unwrap();
    let theirs = listed_by_git(&git);
    assert_eq!(ours.len(), theirs.len());
    assert_eq!(ours.len(), 4);
    for (index, (mine, record)) in ours.iter().zip(&theirs).enumerate() {
        let path = attribute(record, "worktree").unwrap();
        assert_eq!(mine["path"], system_form(path));
        assert_eq!(mine["head"], attribute(record, "HEAD").unwrap());
        assert_eq!(
            mine["branch"]
                .as_str()
                .map(|name| format!("refs/heads/{name}")),
            attribute(record, "branch").map(str::to_owned),
            "{path}"
        );
        assert_eq!(mine["detached"], attribute(record, "detached").is_some());
        assert_eq!(mine["locked"], attribute(record, "locked").is_some());
        assert_eq!(mine["prunable"], attribute(record, "prunable").is_some());
        assert_eq!(mine["is_main"], index == 0);
        assert_eq!(mine["current"], index == 0);
        assert_eq!(mine["bare"], false);
        // As many changes as `git status` has lines there.
        let counted = Path::new(path).is_dir().then(|| {
            Git::new(path, fixtures_out_dir().join(".home"))
                .lines(["status", "--porcelain"])
                .unwrap()
                .len()
        });
        assert_eq!(
            mine["changes"].as_u64().map(|count| count as usize),
            counted,
            "{path}"
        );
    }
    assert_eq!(ours[0]["changes"], 1);
    assert_eq!(ours[1]["changes"], 2);
    assert_eq!(ours[2]["lock_reason"], "on a stick that is unplugged");
    assert!(ours[3]["changes"].is_null(), "its directory is gone");
    assert!(
        ours[3]["prune_reason"]
            .as_str()
            .is_some_and(|reason| !reason.is_empty())
    );

    // Not counted when not asked for.
    let plain = h.run("worktree.list", json!({ "changes": false }));
    assert!(
        plain
            .as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["changes"].is_null())
    );

    // The snapshot the references panel is built from knows the same
    // linked worktrees.
    let refs = h.run("repo.refs", json!({}));
    let known: Vec<(&str, Option<&str>, bool)> = refs["worktrees"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["is_main"] == false)
        .map(|entry| {
            (
                entry["path"].as_str().unwrap_or(""),
                entry["branch"].as_str(),
                entry["locked"].as_bool().unwrap(),
            )
        })
        .collect();
    let mut listed: Vec<(&str, Option<&str>, bool)> = ours
        .iter()
        .skip(1)
        .filter(|entry| entry["prunable"] == false)
        .map(|entry| {
            (
                entry["path"].as_str().unwrap(),
                entry["branch"].as_str(),
                entry["locked"].as_bool().unwrap(),
            )
        })
        .collect();
    let mut known: Vec<_> = known
        .into_iter()
        .filter(|(path, _, _)| Path::new(path).is_dir())
        .collect();
    known.sort();
    listed.sort();
    assert_eq!(known, listed);

    // Opened in a linked worktree, that one is the current one.
    let registry = builtin_registry();
    let mut linked = CommandContext::default();
    registry
        .run("repo.open", &mut linked, &json!({ "path": wt("a") }))
        .unwrap();
    let seen = registry
        .run("worktree.list", &mut linked, &json!({ "changes": false }))
        .unwrap();
    let current: Vec<bool> = seen
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["current"].as_bool().unwrap())
        .collect();
    assert_eq!(current, [false, true, false, false]);
}

#[test]
fn a_bare_repository_lists_its_worktrees_and_a_worktree_of_it_is_worked_in() {
    let side = Side::new();
    let bare = side.wt("bare.git");
    side.git()
        .run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    let bare_git = Git::new(&bare, fixtures_out_dir().join(".home"));
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": bare }))
        .unwrap();
    let listed = registry.run("worktree.list", &mut ctx, &json!({})).unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0]["bare"], true);
    assert_eq!(listed[0]["is_main"], true);
    assert_eq!(listed[0]["current"], true);
    assert!(listed[0]["changes"].is_null());

    // The bare repository itself is opened read-only: nothing
    // is added to it or taken from it here.
    for (command, params) in [
        (
            "worktree.add",
            json!({ "path": side.wt("refused"), "branch": "feature/a" }),
        ),
        ("worktree.remove", json!({ "path": bare })),
        ("worktree.prune", json!({})),
    ] {
        let err = registry
            .run(command, &mut ctx, &params)
            .unwrap_err()
            .to_string();
        assert!(err.contains("read-only"), "{command}: {err}");
    }
    assert!(!side.wt("refused").exists());
    assert!(!bare.join("worktrees").exists());

    // A worktree of it, made by git, is listed there, and opened itself
    // it is a repository like any other: it has files, and what changes
    // it runs.
    let from_bare = side.wt("from-bare");
    bare_git
        .run([
            "worktree",
            "add",
            "-q",
            from_bare.to_str().unwrap(),
            "feature/a",
        ])
        .unwrap();
    let listed = registry.run("worktree.list", &mut ctx, &json!({})).unwrap();
    let listed = listed.as_array().unwrap();
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[1]["branch"], "feature/a");
    let mut inside = CommandContext::default();
    let summary = registry
        .run("repo.open", &mut inside, &json!({ "path": from_bare }))
        .unwrap();
    assert_eq!(summary["bare"], false);
    // Its files have a status, as git has it, although the configuration
    // it shares says `bare`.
    std::fs::write(from_bare.join("a.txt"), "changed in the worktree\n").unwrap();
    std::fs::write(from_bare.join("new.txt"), "new\n").unwrap();
    let status = registry
        .run("repo.status", &mut inside, &json!({}))
        .unwrap();
    let mut paths: Vec<&str> = status
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    paths.sort_unstable();
    let from_bare_git = Git::new(&from_bare, fixtures_out_dir().join(".home"));
    let mut want: Vec<String> = from_bare_git
        .run(["status", "--porcelain"])
        .unwrap()
        .lines()
        .map(|line| line[3..].to_owned())
        .collect();
    want.sort_unstable();
    assert_eq!(paths, want);
    assert_eq!(paths, ["a.txt", "new.txt"]);
    from_bare_git
        .run(["checkout", "-q", "--", "a.txt"])
        .unwrap();
    std::fs::remove_file(from_bare.join("new.txt")).unwrap();
    registry
        .run(
            "branch.create",
            &mut inside,
            &json!({ "name": "made-here" }),
        )
        .unwrap();
    let made = registry
        .run(
            "worktree.add",
            &mut inside,
            &json!({ "path": side.wt("second"), "branch": "made-here" }),
        )
        .unwrap();
    assert_eq!(made["branch"], "made-here");
    assert!(side.wt("second").join("a.txt").is_file());
    assert_eq!(
        bare_git.rev_parse("made-here").unwrap(),
        bare_git.rev_parse("feature/a").unwrap()
    );
}

#[test]
fn add_matches_git_worktree_add() {
    let mut h = Harness::new();
    h.both(Side::add_remote);
    h.assert_same("before");

    // A branch that exists.
    let made = h.run(
        "worktree.add",
        json!({ "path": h.path("a"), "branch": "feature/a" }),
    );
    assert_eq!(made["branch"], "feature/a");
    assert_eq!(made["path"], h.path("a"));
    assert_eq!(made["is_main"], false);
    let twin = h.twin.wt("a");
    h.twin
        .git()
        .run(["worktree", "add", "-q", twin.to_str().unwrap(), "feature/a"])
        .unwrap();
    h.assert_same("an existing branch");

    // A new branch at a commit: a plain branch, following nothing.
    let made = h.run(
        "worktree.add",
        json!({ "path": h.path("topic"), "new_branch": "topic/one", "start": "HEAD~1" }),
    );
    assert_eq!(made["branch"], "topic/one");
    assert_eq!(made["head"], h.ours.git().rev_parse("HEAD~1").unwrap());
    let twin = h.twin.wt("topic");
    h.twin
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            "--no-track",
            "-b",
            "topic/one",
            twin.to_str().unwrap(),
            "HEAD~1",
        ])
        .unwrap();
    h.assert_same("a new branch at a commit");

    // A new branch at HEAD when nothing is said.
    h.run(
        "worktree.add",
        json!({ "path": h.path("here"), "new_branch": "here" }),
    );
    let twin = h.twin.wt("here");
    h.twin
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            "--no-track",
            "-b",
            "here",
            twin.to_str().unwrap(),
        ])
        .unwrap();
    h.assert_same("a new branch at HEAD");

    // A new branch from a branch of the remote follows it.
    h.run(
        "worktree.add",
        json!({ "path": h.path("b"), "new_branch": "b-local", "start": "origin/feature/b" }),
    );
    let twin = h.twin.wt("b");
    h.twin
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            "--track",
            "-b",
            "b-local",
            twin.to_str().unwrap(),
            "origin/feature/b",
        ])
        .unwrap();
    h.assert_same("a new branch that follows a remote one");
    assert_eq!(
        h.ours
            .git()
            .run(["rev-parse", "--abbrev-ref", "b-local@{upstream}"])
            .unwrap()
            .trim(),
        "origin/feature/b"
    );

    // Detached, at a tag; the path relative to the repository, and the
    // directory there already, empty.
    std::fs::create_dir_all(h.ours.wt("det")).unwrap();
    let relative = format!(
        "../{}",
        h.ours.wt("det").file_name().unwrap().to_str().unwrap()
    );
    let made = h.run(
        "worktree.add",
        json!({ "path": relative, "detach": true, "start": "v1.0" }),
    );
    assert_eq!(made["detached"], true);
    assert!(made["branch"].is_null());
    let twin = h.twin.wt("det");
    h.twin
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            "--detach",
            twin.to_str().unwrap(),
            "v1.0",
        ])
        .unwrap();
    h.assert_same("detached at a tag");

    // Detached at HEAD when nothing is said.
    h.run(
        "worktree.add",
        json!({ "path": h.path("head"), "detach": true }),
    );
    let twin = h.twin.wt("head");
    h.twin
        .git()
        .run(["worktree", "add", "-q", "--detach", twin.to_str().unwrap()])
        .unwrap();
    h.assert_same("detached at HEAD");

    // Nothing of this is an action to undo.
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn add_refuses_what_cannot_work_and_leaves_nothing_behind() {
    let mut h = Harness::new();
    let git = h.ours.git();
    git.run([
        "worktree",
        "add",
        "-q",
        h.ours.wt("a").to_str().unwrap(),
        "feature/a",
    ])
    .unwrap();
    std::fs::create_dir_all(h.ours.wt("full")).unwrap();
    std::fs::write(h.ours.wt("full").join("kept.txt"), "kept\n").unwrap();
    std::fs::write(h.ours.wt("file"), "a file\n").unwrap();
    let before = h.ours.state();

    let fresh = h.path("fresh");
    for (params, because) in [
        (
            json!({ "path": h.path("full"), "new_branch": "x" }),
            "not an empty directory",
        ),
        (
            json!({ "path": h.path("file"), "new_branch": "x" }),
            "not an empty directory",
        ),
        (
            json!({ "path": h.path("a"), "new_branch": "x" }),
            "not an empty directory",
        ),
        // Checked out in the main worktree, and in a linked one.
        (
            json!({ "path": fresh, "branch": "main" }),
            "is checked out in",
        ),
        (
            json!({ "path": fresh, "branch": "feature/a" }),
            "is checked out in",
        ),
        (
            json!({ "path": fresh, "branch": "no-such" }),
            "there is no branch",
        ),
        // A branch of a remote is not a branch to check out here.
        (
            json!({ "path": fresh, "branch": "origin/main" }),
            "there is no branch",
        ),
        (
            json!({ "path": fresh, "new_branch": "feature/b" }),
            "exists already",
        ),
        (
            json!({ "path": fresh, "new_branch": "x", "start": "no-such" }),
            "does not name a commit",
        ),
        (
            json!({ "path": fresh, "detach": true, "start": "HEAD^{tree}" }),
            "does not name a commit",
        ),
        (json!({ "path": fresh }), "say what to check out"),
        (
            json!({ "path": fresh, "branch": "feature/b", "new_branch": "x" }),
            "give one of",
        ),
        (
            json!({ "path": fresh, "branch": "feature/b", "detach": true }),
            "give one of",
        ),
        (
            json!({ "path": fresh, "branch": "feature/b", "start": "HEAD" }),
            "start goes with",
        ),
    ] {
        h.refused("worktree.add", params, because);
        assert_eq!(h.ours.state(), before);
        assert!(!h.ours.wt("fresh").exists());
    }
    assert_eq!(
        std::fs::read_to_string(h.ours.wt("full").join("kept.txt")).unwrap(),
        "kept\n"
    );

    // Git itself gives up, at a path it cannot make a directory at (under
    // a file): the branch it was to make is not left behind.
    let under_file = h.ours.wt("file").join("below");
    let err = h
        .try_run(
            "worktree.add",
            json!({ "path": under_file, "new_branch": "never" }),
        )
        .unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(h.ours.state(), before, "no branch is left of a failed add");
}

#[test]
fn remove_matches_git_and_keeps_work_unless_forced() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        for (name, args) in [
            ("clean", vec!["feature/a"]),
            ("dirty", vec!["feature/b"]),
            ("locked", vec!["--detach"]),
        ] {
            let path = side.wt(name);
            let mut command = vec!["worktree", "add", "-q", path.to_str().unwrap()];
            command.extend(args);
            git.run(command).unwrap();
        }
        git.run(["worktree", "lock", side.wt("locked").to_str().unwrap()])
            .unwrap();
        std::fs::write(side.wt("dirty").join("b.txt"), "work\n").unwrap();
        std::fs::write(side.wt("dirty").join("untracked.txt"), "more\n").unwrap();
    });
    h.assert_same("before");

    // A clean one goes.
    let out = h.run("worktree.remove", json!({ "path": h.path("clean") }));
    assert_eq!(out["branch"], "feature/a");
    assert!(out["deleted"].is_null() && out["branch_kept"].is_null());
    h.twin
        .git()
        .run(["worktree", "remove", h.twin.wt("clean").to_str().unwrap()])
        .unwrap();
    h.assert_same("a clean worktree");
    assert!(!h.ours.wt("clean").exists());

    // One with work in it stays, until it is said twice.
    h.refused(
        "worktree.remove",
        json!({ "path": h.path("dirty") }),
        "2 changed files; they would be lost",
    );
    h.assert_same("refused with changes");
    assert!(h.ours.wt("dirty").join("untracked.txt").is_file());
    h.run(
        "worktree.remove",
        json!({ "path": h.path("dirty"), "force": true }),
    );
    h.twin
        .git()
        .run([
            "worktree",
            "remove",
            "--force",
            h.twin.wt("dirty").to_str().unwrap(),
        ])
        .unwrap();
    h.assert_same("forced with changes");

    // A locked one stays even when forced, until it is unlocked.
    for force in [false, true] {
        h.refused(
            "worktree.remove",
            json!({ "path": h.path("locked"), "force": force }),
            "is locked; unlock it first",
        );
    }
    h.assert_same("refused while locked");
    h.run("worktree.unlock", json!({ "path": h.path("locked") }));
    h.run("worktree.remove", json!({ "path": h.path("locked") }));
    let twin = h.twin.git();
    let locked = h.twin.wt("locked");
    twin.run(["worktree", "unlock", locked.to_str().unwrap()])
        .unwrap();
    twin.run(["worktree", "remove", locked.to_str().unwrap()])
        .unwrap();
    h.assert_same("unlocked, then removed");

    // The main one never.
    h.refused(
        "worktree.remove",
        json!({ "path": h.ours.copy.path(), "force": true }),
        "the main worktree cannot be removed",
    );
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn removing_with_the_branch_is_undone_as_a_branch_deletion() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        let merged = side.wt("merged");
        git.run([
            "worktree",
            "add",
            "-q",
            merged.to_str().unwrap(),
            "feature/a",
        ])
        .unwrap();
        // A branch with a commit that is nowhere else.
        let lone = side.wt("lone");
        git.run([
            "worktree",
            "add",
            "-q",
            "-b",
            "lone",
            lone.to_str().unwrap(),
        ])
        .unwrap();
        std::fs::write(lone.join("lone.txt"), "lone\n").unwrap();
        let inside = side.git_in("lone");
        inside.run(["add", "lone.txt"]).unwrap();
        inside.run(["commit", "-q", "-m", "lone work"]).unwrap();
    });
    let tip = h.ours.git().rev_parse("feature/a").unwrap();

    let out = h.run(
        "worktree.remove",
        json!({ "path": h.path("merged"), "delete_branch": true }),
    );
    assert_eq!(out["deleted"]["name"], "feature/a");
    let twin = h.twin.git();
    twin.run(["worktree", "remove", h.twin.wt("merged").to_str().unwrap()])
        .unwrap();
    twin.run(["branch", "-d", "feature/a"]).unwrap();
    h.assert_same("worktree and branch");
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Delete branch feature/a"
    );

    // Undo brings the branch back, where it was; the worktree stays gone.
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.git().rev_parse("feature/a").unwrap(), tip);
    assert!(!h.ours.wt("merged").exists());
    h.run("redo.last", json!({}));
    h.assert_same("redo of the deletion");

    // A branch that is merged nowhere stays, and the answer says why.
    let out = h.run(
        "worktree.remove",
        json!({ "path": h.path("lone"), "delete_branch": true }),
    );
    assert!(out["deleted"].is_null());
    assert!(
        out["branch_kept"]
            .as_str()
            .is_some_and(|why| why.contains("not fully merged")),
        "{out}"
    );
    twin.run(["worktree", "remove", h.twin.wt("lone").to_str().unwrap()])
        .unwrap();
    h.assert_same("worktree gone, unmerged branch kept");
    assert!(h.ours.git().rev_parse("lone").is_ok());
}

#[test]
fn a_worktree_does_not_remove_itself() {
    let side = Side::new();
    let path = side.wt("self");
    side.git()
        .run(["worktree", "add", "-q", path.to_str().unwrap(), "feature/a"])
        .unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": path }))
        .unwrap();
    let err = registry
        .run("worktree.remove", &mut ctx, &json!({ "path": path }))
        .unwrap_err()
        .to_string();
    assert!(err.contains("is the worktree that is open"), "{err}");
    assert!(path.join("a.txt").is_file());
    // From there the main one is still the main one.
    let err = registry
        .run(
            "worktree.remove",
            &mut ctx,
            &json!({ "path": side.copy.path() }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("main worktree"), "{err}");
}

#[test]
fn lock_and_unlock_match_git() {
    let mut h = Harness::new();
    h.both(|side| {
        for name in ["one", "two"] {
            let path = side.wt(name);
            side.git()
                .run(["worktree", "add", "-q", "--detach", path.to_str().unwrap()])
                .unwrap();
        }
    });
    let reason = "on the laptop; \"do not\" prune";

    let out = h.run(
        "worktree.lock",
        json!({ "path": h.path("one"), "reason": reason }),
    );
    assert_eq!(out["locked"], true);
    assert_eq!(out["lock_reason"], reason);
    // An empty reason is no reason.
    let out = h.run(
        "worktree.lock",
        json!({ "path": h.path("two"), "reason": "  " }),
    );
    assert!(out["lock_reason"].is_null());
    let twin = h.twin.git();
    twin.run([
        "worktree",
        "lock",
        "--reason",
        reason,
        h.twin.wt("one").to_str().unwrap(),
    ])
    .unwrap();
    twin.run(["worktree", "lock", h.twin.wt("two").to_str().unwrap()])
        .unwrap();
    h.assert_same("locked");

    h.refused(
        "worktree.lock",
        json!({ "path": h.path("one") }),
        "is locked already",
    );
    h.refused(
        "worktree.lock",
        json!({ "path": h.ours.copy.path() }),
        "the main worktree cannot be locked",
    );
    h.assert_same("refused locks");

    let out = h.run("worktree.unlock", json!({ "path": h.path("one") }));
    assert_eq!(out["locked"], false);
    twin.run(["worktree", "unlock", h.twin.wt("one").to_str().unwrap()])
        .unwrap();
    h.assert_same("unlocked");
    h.refused(
        "worktree.unlock",
        json!({ "path": h.path("one") }),
        "is not locked",
    );
    h.refused(
        "worktree.unlock",
        json!({ "path": h.ours.copy.path() }),
        "is not locked",
    );
}

#[test]
fn move_matches_git_worktree_move() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        for name in ["one", "kept", "busy"] {
            let path = side.wt(name);
            git.run(["worktree", "add", "-q", "--detach", path.to_str().unwrap()])
                .unwrap();
        }
        git.run(["worktree", "lock", side.wt("kept").to_str().unwrap()])
            .unwrap();
        // Work in the one that moves goes with it.
        std::fs::write(side.wt("one").join("a.txt"), "changed here\n").unwrap();
        std::fs::write(side.wt("one").join("new.txt"), "new here\n").unwrap();
        std::fs::write(side.wt("busy").join("there.txt"), "in the way\n").unwrap();
    });

    let out = h.run(
        "worktree.move",
        json!({ "path": h.path("one"), "to": h.path("moved") }),
    );
    assert_eq!(out["path"], h.path("moved"));
    let twin = h.twin.git();
    twin.run([
        "worktree",
        "move",
        h.twin.wt("one").to_str().unwrap(),
        h.twin.wt("moved").to_str().unwrap(),
    ])
    .unwrap();
    h.assert_same("moved");
    assert!(!h.ours.wt("one").exists());
    assert_eq!(
        std::fs::read_to_string(h.ours.wt("moved").join("new.txt")).unwrap(),
        "new here\n"
    );
    // Git works in it where it is now.
    assert_eq!(
        h.ours
            .git_in("moved")
            .run(["status", "--porcelain"])
            .unwrap(),
        h.twin
            .git_in("moved")
            .run(["status", "--porcelain"])
            .unwrap()
    );

    // What stays where it is, each with its reason, and nothing changes.
    let before = h.ours.state();
    h.refused(
        "worktree.move",
        json!({ "path": h.path("kept"), "to": h.path("elsewhere") }),
        "is locked",
    );
    h.refused(
        "worktree.move",
        json!({ "path": h.ours.copy.path(), "to": h.path("elsewhere") }),
        "the main worktree cannot be moved",
    );
    h.refused(
        "worktree.move",
        json!({ "path": h.path("moved"), "to": h.path("busy") }),
        "there is something at",
    );
    h.refused(
        "worktree.move",
        json!({ "path": h.path("nowhere"), "to": h.path("elsewhere") }),
        "is not a worktree of this repository",
    );
    for to in ["", "a\nb", "x\u{0}y"] {
        assert!(
            h.try_run(
                "worktree.move",
                json!({ "path": h.path("moved"), "to": to })
            )
            .is_err(),
            "{to:?}"
        );
    }
    assert!(
        h.try_run("worktree.move", json!({ "path": 7, "to": [] }))
            .is_err()
    );
    assert_eq!(h.ours.state(), before);
    assert!(!h.ours.wt("elsewhere").exists());
}

#[test]
fn prune_forgets_what_is_gone_and_not_locked() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        for name in ["gone", "gone-locked", "there"] {
            let path = side.wt(name);
            git.run(["worktree", "add", "-q", "--detach", path.to_str().unwrap()])
                .unwrap();
        }
        git.run(["worktree", "lock", side.wt("gone-locked").to_str().unwrap()])
            .unwrap();
        std::fs::remove_dir_all(side.wt("gone")).unwrap();
        std::fs::remove_dir_all(side.wt("gone-locked")).unwrap();
    });
    // What is gone is not removed, it is pruned.
    h.refused(
        "worktree.remove",
        json!({ "path": h.path("gone") }),
        "pruning removes what is left of it",
    );

    let out = h.run("worktree.prune", json!({}));
    assert_eq!(out["pruned"], json!([h.path("gone")]));
    h.twin.git().run(["worktree", "prune"]).unwrap();
    h.assert_same("pruned");
    let left = h.run("worktree.list", json!({ "changes": false }));
    assert_eq!(left.as_array().unwrap().len(), 3);

    // Nothing more to prune.
    assert_eq!(h.run("worktree.prune", json!({}))["pruned"], json!([]));
    h.assert_same("pruned again");
}

#[test]
fn hostile_worktree_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let git = h.ours.git();
    git.run([
        "worktree",
        "add",
        "-q",
        h.ours.wt("kept").to_str().unwrap(),
        "feature/a",
    ])
    .unwrap();
    let before = h.ours.state();
    let long = "w".repeat(5000);
    let other = Scratch::of("linear").unwrap();

    // A path that names no worktree of this repository.
    for path in [
        "",
        "-f",
        "--force",
        "a\nb",
        "nul\u{0}",
        ".",
        "..",
        "/",
        "nowhere",
        "../../../../etc",
        other.path().to_str().unwrap(),
        long.as_str(),
    ] {
        for (command, params) in [
            ("worktree.remove", json!({ "path": path, "force": true })),
            (
                "worktree.remove",
                json!({ "path": path, "delete_branch": true }),
            ),
            ("worktree.lock", json!({ "path": path })),
            ("worktree.unlock", json!({ "path": path })),
        ] {
            // The copy itself is a worktree: its own refusals apply.
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the path {path:?}"
            );
        }
    }
    // A path nothing can be made at.
    for path in ["", "a\nb", "nul\u{0}", "\u{7}bell", long.as_str()] {
        for params in [
            json!({ "path": path, "new_branch": "fine" }),
            json!({ "path": path, "detach": true }),
        ] {
            assert!(
                h.try_run("worktree.add", params).is_err(),
                "worktree.add accepted the path {path:?}"
            );
        }
    }
    let fresh = h.path("fresh");
    for name in [
        "",
        "-d",
        "--force",
        "a..b",
        "a b",
        "a\nb",
        "HEAD",
        "@",
        "@{-1}",
        "x.lock",
        "/lead",
        "trail/",
        long.as_str(),
    ] {
        for params in [
            json!({ "path": fresh, "branch": name }),
            json!({ "path": fresh, "new_branch": name }),
        ] {
            assert!(
                h.try_run("worktree.add", params).is_err(),
                "worktree.add accepted the branch {name:?}"
            );
        }
    }
    for start in [
        "",
        "-x",
        "--orphan",
        "no-such-rev",
        "HEAD^{tree}",
        "a\nb",
        long.as_str(),
    ] {
        for params in [
            json!({ "path": fresh, "new_branch": "fine", "start": start }),
            json!({ "path": fresh, "detach": true, "start": start }),
        ] {
            assert!(
                h.try_run("worktree.add", params).is_err(),
                "worktree.add accepted the start {start:?}"
            );
        }
    }
    for reason in ["nul\u{0}", long.as_str()] {
        assert!(
            h.try_run(
                "worktree.lock",
                json!({ "path": h.path("kept"), "reason": reason })
            )
            .is_err(),
            "worktree.lock accepted a reason of {} bytes",
            reason.len()
        );
    }
    for (command, params) in [
        ("worktree.add", json!({})),
        ("worktree.add", json!({ "path": 3, "detach": true })),
        ("worktree.add", json!({ "path": fresh, "detach": "yes" })),
        ("worktree.add", json!({ "path": fresh, "branch": ["a"] })),
        ("worktree.remove", json!({})),
        ("worktree.remove", json!({ "path": ["a"] })),
        (
            "worktree.remove",
            json!({ "path": h.path("kept"), "force": "yes" }),
        ),
        (
            "worktree.lock",
            json!({ "path": h.path("kept"), "reason": 3 }),
        ),
        ("worktree.unlock", json!({ "path": null })),
        ("worktree.list", json!({ "changes": "all" })),
    ] {
        assert!(
            h.try_run(command, params.clone()).is_err(),
            "{command} {params}"
        );
    }
    assert_eq!(h.ours.state(), before);
    assert!(!h.ours.wt("fresh").exists());
    assert!(h.ours.wt("kept").join("a.txt").is_file());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn a_repository_that_is_not_as_expected_is_read_without_a_crash() {
    let mut h = Harness::new();
    let git = h.ours.git();
    git.run([
        "worktree",
        "add",
        "-q",
        "--detach",
        h.ours.wt("odd").to_str().unwrap(),
    ])
    .unwrap();
    let admin = h.ours.copy.path().join(".git/worktrees");
    let entry = std::fs::read_dir(&admin)
        .unwrap()
        .flatten()
        .next()
        .unwrap()
        .path();

    // What git keeps about the worktree, damaged in turn.
    for (file, content) in [
        ("HEAD", ""),
        ("HEAD", "ref: refs/heads/\n"),
        ("HEAD", "garbage that is no id\n"),
        ("gitdir", ""),
        ("gitdir", "/nowhere/at/all/.git\n"),
        ("locked", "\u{0}\u{1}\u{2}"),
    ] {
        let path = entry.join(file);
        let saved = std::fs::read(&path).ok();
        std::fs::write(&path, content).unwrap();
        // An answer or a refusal, never a crash.
        let _ = h.try_run("worktree.list", json!({}));
        let _ = h.try_run("worktree.lock", json!({ "path": h.path("odd") }));
        let _ = h.try_run("worktree.unlock", json!({ "path": h.path("odd") }));
        match saved {
            Some(bytes) => std::fs::write(&path, bytes).unwrap(),
            // An unlock that went through has removed it already.
            None => drop(std::fs::remove_file(&path)),
        }
    }
    // The directory of the worktree replaced by a file.
    std::fs::remove_dir_all(h.ours.wt("odd")).unwrap();
    std::fs::write(h.ours.wt("odd"), "not a directory\n").unwrap();
    let listed = h.run("worktree.list", json!({}));
    assert_eq!(listed.as_array().unwrap().len(), 2);
    let _ = h.try_run("worktree.remove", json!({ "path": h.path("odd") }));
    let _ = h.try_run("worktree.prune", json!({}));
}

/// The changed tracked files of a worktree by kind, as the test reads them
/// out of git's other status format: (modified, added, deleted, renamed).
/// Untracked files are not counted: the other worktrees are looked at
/// without the search for them.
fn kinds_by_git(dir: &Path) -> (u64, u64, u64, u64) {
    let git = Git::new(dir, fixtures_out_dir().join(".home"));
    let (mut modified, mut added, mut deleted, mut renamed) = (0, 0, 0, 0);
    for line in git
        .lines(["status", "--porcelain=v2", "--untracked-files=no"])
        .unwrap()
    {
        let mut words = line.split(' ');
        let (kind, xy) = (words.next().unwrap_or(""), words.next().unwrap_or(""));
        let (staged, unstaged) = (xy.chars().next(), xy.chars().nth(1));
        match kind {
            "?" => added += 1,
            "2" => renamed += 1,
            "u" => modified += 1,
            "1" => match (staged, unstaged) {
                (Some('A'), _) => added += 1,
                (Some('D'), _) | (Some('.'), Some('D')) => deleted += 1,
                _ => modified += 1,
            },
            _ => {}
        }
    }
    (modified, added, deleted, renamed)
}

#[test]
fn the_changes_of_other_worktrees_are_counted_by_kind_and_not_twice() {
    let mut h = Harness::new();
    let git = h.ours.git();
    for (name, branch) in [("a", "feature/a"), ("b", "feature/b")] {
        git.run([
            "worktree",
            "add",
            "-q",
            h.ours.wt(name).to_str().unwrap(),
            branch,
        ])
        .unwrap();
    }
    git.run([
        "worktree",
        "add",
        "-q",
        "--detach",
        h.ours.wt("clean").to_str().unwrap(),
        "HEAD~1",
    ])
    .unwrap();
    // In `a`: one file changed, one new, one staged as new, one moved, one
    // removed. In `b`: one file changed. The open worktree has a change of
    // its own, which is not what is asked about.
    let (a, b) = (h.ours.wt("a"), h.ours.wt("b"));
    let in_a = h.ours.git_in("a");
    let tracked: Vec<String> = in_a.lines(["ls-files"]).unwrap();
    assert!(
        tracked.len() >= 3,
        "the fixture has files to change: {tracked:?}"
    );
    std::fs::write(a.join(&tracked[0]), "changed\n").unwrap();
    std::fs::write(a.join("new.txt"), "new\n").unwrap();
    std::fs::write(a.join("staged.txt"), "staged\n").unwrap();
    in_a.run(["add", "staged.txt"]).unwrap();
    in_a.run(["mv", &tracked[1], "moved.txt"]).unwrap();
    std::fs::remove_file(a.join(&tracked[2])).unwrap();
    let in_b = h.ours.git_in("b");
    let tracked_b: Vec<String> = in_b.lines(["ls-files"]).unwrap();
    std::fs::write(b.join(&tracked_b[0]), "changed here too\n").unwrap();
    h.ours
        .copy
        .write("base.txt", "changed in the open one\n")
        .unwrap();

    let first = h.run("worktree.changes", json!({}));
    let first = first.as_array().unwrap().clone();
    let by_path = |entries: &[Value], dir: &Path| -> Value {
        entries
            .iter()
            .find(|entry| entry["path"] == dir.to_str().unwrap())
            .cloned()
            .unwrap_or_else(|| panic!("{} is not listed", dir.display()))
    };
    // The other worktrees and only those, each with what git has there.
    assert_eq!(first.len(), 3);
    for name in ["a", "b", "clean"] {
        let dir = h.ours.wt(name);
        let entry = by_path(&first, &dir);
        let (modified, added, deleted, renamed) = kinds_by_git(&dir);
        assert_eq!(
            entry["counts"],
            json!({ "modified": modified, "added": added, "deleted": deleted, "renamed": renamed }),
            "{name}"
        );
        assert_eq!(entry["scanned"], true);
        assert_eq!(entry["waiting"], false);
        let lines = Git::new(&dir, fixtures_out_dir().join(".home"))
            .lines(["status", "--porcelain", "--untracked-files=no"])
            .unwrap()
            .len() as u64;
        assert_eq!(modified + added + deleted + renamed, lines, "{name}");
        assert_eq!(
            entry["head"],
            Git::new(&dir, fixtures_out_dir().join(".home"))
                .rev_parse("HEAD")
                .unwrap()
        );
    }
    // `new.txt` is not tracked: not counted.
    assert_eq!(
        by_path(&first, &a)["counts"],
        json!({ "modified": 1, "added": 1, "deleted": 1, "renamed": 1 })
    );
    assert_eq!(by_path(&first, &a)["branch"], "feature/a");
    assert!(by_path(&first, &h.ours.wt("clean"))["branch"].is_null());

    // The test's own `git status` above refreshed the indexes it read (ours
    // never does), so the stamps are taken once more.
    let first = h.run("worktree.changes", json!({}));
    let first = first.as_array().unwrap().clone();
    // Told what it said, it looks at nothing again: no counts, the stamps
    // as they were.
    let known = |entries: &[Value]| -> Value {
        Value::Object(
            entries
                .iter()
                .map(|entry| {
                    (
                        entry["path"].as_str().unwrap().to_owned(),
                        entry["stamp"].clone(),
                    )
                })
                .collect(),
        )
    };
    let again = h.run("worktree.changes", json!({ "known": known(&first) }));
    for entry in again.as_array().unwrap() {
        assert_eq!(entry["scanned"], false, "{entry}");
        assert!(entry["counts"].is_null());
        assert_eq!(
            entry["stamp"],
            by_path(&first, Path::new(entry["path"].as_str().unwrap()))["stamp"]
        );
    }
    // Something staged in `b`, a commit in `a`: those two are looked at
    // again, the third is not.
    std::thread::sleep(std::time::Duration::from_millis(20));
    in_b.run(["add", "-A"]).unwrap();
    in_a.run(["add", "-A"]).unwrap();
    in_a.run(["commit", "-q", "-m", "work in a"]).unwrap();
    let third = h.run("worktree.changes", json!({ "known": known(&first) }));
    let third = third.as_array().unwrap().clone();
    assert_eq!(by_path(&third, &a)["scanned"], true);
    assert_eq!(
        by_path(&third, &a)["counts"],
        json!({ "modified": 0, "added": 0, "deleted": 0, "renamed": 0 })
    );
    assert_eq!(by_path(&third, &b)["scanned"], true);
    assert_eq!(by_path(&third, &b)["counts"]["modified"], 1);
    assert_eq!(by_path(&third, &h.ours.wt("clean"))["scanned"], false);
    // An edit that touches neither HEAD nor the index does not change the
    // stamp: it is seen when asked with nothing known.
    let clean_files = h.ours.git_in("clean").lines(["ls-files"]).unwrap();
    std::fs::write(h.ours.wt("clean").join(&clean_files[0]), "edited\n").unwrap();
    let stale = h.run("worktree.changes", json!({ "known": known(&third) }));
    assert_eq!(
        by_path(stale.as_array().unwrap(), &h.ours.wt("clean"))["scanned"],
        false
    );
    let fresh = h.run("worktree.changes", json!({}));
    assert_eq!(
        by_path(fresh.as_array().unwrap(), &h.ours.wt("clean"))["counts"]["modified"],
        1
    );

    // Hostile parameters: refused, or taken as knowing nothing.
    for bad in [
        json!({ "known": "everything" }),
        json!({ "known": ["a", "b"] }),
        json!({ "known": { "a": 5 } }),
        json!({ "known": { "a": null } }),
        json!({ "known": 7 }),
        json!({ "skip": "a" }),
        json!({ "skip": [1, 2] }),
        json!({ "skip": { "a": true } }),
        json!({ "at_most": -1 }),
        json!({ "at_most": "one" }),
        json!({ "at_most": 1.5 }),
        json!({ "at_most": [1] }),
    ] {
        assert!(h.try_run("worktree.changes", bad.clone()).is_err(), "{bad}");
    }
    let many: serde_json::Map<String, Value> = (0..5000)
        .map(|n| (format!("/nowhere/{n}"), json!("stamp")))
        .collect();
    assert!(
        h.try_run("worktree.changes", json!({ "known": many }))
            .is_err()
    );
    let many: Vec<String> = (0..5000).map(|n| format!("/nowhere/{n}")).collect();
    assert!(
        h.try_run("worktree.changes", json!({ "skip": many }))
            .is_err()
    );
    // As many as there can be: no limit at all.
    let all = h.run("worktree.changes", json!({ "at_most": u64::MAX }));
    assert_eq!(all.as_array().unwrap().len(), 3);
    let odd = json!({ "known": { "": "", "\u{0}": "\u{0}", "../..": "x", a.to_str().unwrap(): "not its stamp" } });
    let listed = h.run("worktree.changes", odd);
    assert_eq!(listed.as_array().unwrap().len(), 3);
    assert_eq!(by_path(listed.as_array().unwrap(), &a)["scanned"], true);

    // A worktree whose directory is gone is not listed.
    std::fs::remove_dir_all(&b).unwrap();
    let after = h.run("worktree.changes", json!({}));
    let after = after.as_array().unwrap().clone();
    assert!(
        after
            .iter()
            .all(|entry| entry["path"] != b.to_str().unwrap())
    );
    assert_eq!(after.len(), 2);

    // Stopped before it starts: an error, not half an answer.
    h.ctx
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    assert!(h.try_run("worktree.changes", json!({})).is_err());
    h.ctx
        .cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);
}

/// The look at another worktree leaves out the search for untracked files
/// and nothing else: on a worktree with changes of every kind to tracked
/// files and untracked files all over (in folders git has files in, in
/// folders it has none in, ignored ones), what it counts is what git's own
/// status counts of the tracked files, and git's full status has only the
/// untracked files more.
#[test]
fn the_cheap_look_counts_the_tracked_changes_git_counts() {
    let mut h = Harness::new();
    let dir = h.ours.wt("busy");
    h.ours
        .git()
        .run(["worktree", "add", "-q", dir.to_str().unwrap(), "feature/a"])
        .unwrap();
    let git = h.ours.git_in("busy");
    // One more file to change than the branch has.
    std::fs::write(dir.join("doomed.txt"), "to be removed\n").unwrap();
    git.run(["add", "doomed.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "a file to remove"]).unwrap();
    let tracked: Vec<String> = git.lines(["ls-files"]).unwrap();
    assert!(tracked.len() >= 4, "{tracked:?}");
    // Tracked: changed and not staged, changed, staged and changed again,
    // staged as new and then changed, moved, removed.
    std::fs::write(dir.join(&tracked[0]), "changed\n").unwrap();
    std::fs::write(dir.join(&tracked[1]), "staged\n").unwrap();
    git.run(["add", &tracked[1]]).unwrap();
    std::fs::write(dir.join(&tracked[1]), "staged, then changed\n").unwrap();
    std::fs::write(dir.join("added.txt"), "added\n").unwrap();
    git.run(["add", "added.txt"]).unwrap();
    std::fs::write(dir.join("added.txt"), "added, then changed\n").unwrap();
    git.run(["mv", &tracked[2], "moved.txt"]).unwrap();
    std::fs::remove_file(dir.join(&tracked[3])).unwrap();
    // Untracked: beside tracked files, in a new folder, deep down, and
    // ignored (by the repository's own exclude file, which every worktree
    // reads).
    let exclude = h.ours.copy.path().join(".git/info/exclude");
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    std::fs::write(&exclude, "*.log\nbuild/\n").unwrap();
    for n in 0..20 {
        std::fs::write(dir.join(format!("scratch-{n}.md")), "notes\n").unwrap();
        let deep = dir.join(format!("new-folder/{n}/deeper"));
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("file.txt"), "new\n").unwrap();
        std::fs::write(dir.join(format!("run-{n}.log")), "log\n").unwrap();
        let build = dir.join(format!("build/{n}"));
        std::fs::create_dir_all(&build).unwrap();
        std::fs::write(build.join("out.o"), "obj\n").unwrap();
    }

    let listed = h.run("worktree.changes", json!({}));
    let entry = listed
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == dir.to_str().unwrap())
        .cloned()
        .expect("the worktree is listed");
    let (modified, added, deleted, renamed) = kinds_by_git(&dir);
    assert_eq!(
        entry["counts"],
        json!({ "modified": modified, "added": added, "deleted": deleted, "renamed": renamed })
    );
    // What the kinds are here: two changed, one new, one moved, one gone.
    assert_eq!((modified, added, deleted, renamed), (2, 1, 1, 1));
    // Git's full status lists the same tracked entries and the untracked
    // ones besides; none of those is counted.
    let full = git
        .lines(["status", "--porcelain", "--untracked-files=normal"])
        .unwrap();
    let tracked_only = git
        .lines(["status", "--porcelain", "--untracked-files=no"])
        .unwrap();
    let untracked = full.iter().filter(|line| line.starts_with("??")).count();
    // The twenty notes and the new folder; nothing ignored.
    assert_eq!(untracked, 21, "{full:?}");
    assert_eq!(full.len() - untracked, tracked_only.len());
    assert_eq!(
        (modified + added + deleted + renamed) as usize,
        tracked_only.len()
    );
}

/// How far one look goes: worktrees the caller says not to look at now and
/// those beyond as many as it allows are listed as waiting, with no counts
/// and without git started for them, under the stamp they have now; those
/// whose stamp is the known one are passed over whatever the caller says.
#[test]
fn a_look_goes_as_far_as_it_is_let_and_says_what_waits() {
    let mut h = Harness::new();
    let names = ["a", "b", "c"];
    for name in names {
        h.ours
            .git()
            .run([
                "worktree",
                "add",
                "-q",
                "--detach",
                h.ours.wt(name).to_str().unwrap(),
                "HEAD",
            ])
            .unwrap();
    }
    let paths: Vec<String> = names
        .iter()
        .map(|name| h.ours.wt(name).to_str().unwrap().to_owned())
        .collect();
    let path = |name: &str| paths[names.iter().position(|n| *n == name).unwrap()].clone();
    let entry = |answer: &Value, name: &str| -> Value {
        answer
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["path"] == path(name))
            .cloned()
            .unwrap()
    };
    let flags = |answer: &Value| -> Vec<(bool, bool, bool)> {
        names
            .iter()
            .map(|name| {
                let entry = entry(answer, name);
                (
                    entry["scanned"] == true,
                    entry["waiting"] == true,
                    entry["counts"].is_object(),
                )
            })
            .collect()
    };
    let looked_at = |flags: &[(bool, bool, bool)]| -> Vec<usize> {
        flags
            .iter()
            .enumerate()
            .filter(|(_, (scanned, _, _))| *scanned)
            .map(|(at, _)| at)
            .collect()
    };
    // Nothing known: one looked at (the first git lists), with its counts;
    // the others waiting, without.
    let one = flags(&h.run("worktree.changes", json!({ "at_most": 1 })));
    let first = looked_at(&one);
    assert_eq!(first.len(), 1, "{one:?}");
    for (at, flags) in one.iter().enumerate() {
        let looked = first.contains(&at);
        assert_eq!(*flags, (looked, !looked, looked));
    }
    // None at all: stamps only.
    let none = h.run("worktree.changes", json!({ "at_most": 0 }));
    assert_eq!(flags(&none), [(false, true, false); 3]);
    // That one left out: another is looked at.
    let skipped = flags(&h.run(
        "worktree.changes",
        json!({ "skip": [path(names[first[0]])], "at_most": 1 }),
    ));
    let next = looked_at(&skipped);
    assert_eq!(next.len(), 1, "{skipped:?}");
    assert_ne!(next, first);
    assert_eq!(skipped[first[0]], (false, true, false));
    // Known by their stamps: passed over, left out or not.
    let known: serde_json::Map<String, Value> = names
        .iter()
        .map(|name| (path(name), entry(&none, name)["stamp"].clone()))
        .collect();
    let same = h.run(
        "worktree.changes",
        json!({ "known": known.clone(), "skip": [path("b")], "at_most": 0 }),
    );
    assert_eq!(flags(&same), [(false, false, false); 3]);
    for name in names {
        assert_eq!(entry(&same, name)["stamp"], entry(&none, name)["stamp"]);
    }
    // Something staged in `c`: it waits while it is left out, and is
    // looked at when it is not.
    std::fs::write(h.ours.wt("c").join("staged.txt"), "staged\n").unwrap();
    h.ours.git_in("c").run(["add", "staged.txt"]).unwrap();
    let left = h.run(
        "worktree.changes",
        json!({ "known": known.clone(), "skip": [path("c")] }),
    );
    assert_eq!(
        flags(&left),
        [
            (false, false, false),
            (false, false, false),
            (false, true, false)
        ]
    );
    assert_ne!(entry(&left, "c")["stamp"], entry(&none, "c")["stamp"]);
    let looked = h.run("worktree.changes", json!({ "known": known }));
    assert_eq!(entry(&looked, "c")["counts"]["added"], 1);
    assert_eq!(entry(&looked, "c")["waiting"], false);
}

/// Asking must not write. `git status` refreshes the index it reads when
/// it is let, and the index of a linked worktree is inside the git
/// directory the application watches: every question would then look like
/// a change and be asked again, for ever.
#[test]
fn asking_about_worktrees_writes_nothing_a_watcher_sees() {
    use std::{sync::mpsc, time::Duration};

    let mut h = Harness::new();
    let git = h.ours.git();
    let linked = h.ours.wt("watched");
    git.run([
        "worktree",
        "add",
        "-q",
        linked.to_str().unwrap(),
        "feature/a",
    ])
    .unwrap();
    // Files written again with what they held: their times no longer match
    // the index, which is what makes a status want to refresh it.
    for (dir, file) in [
        (&linked, "a.txt"),
        (&h.ours.copy.path().to_path_buf(), "base.txt"),
    ] {
        let content = std::fs::read(dir.join(file)).unwrap();
        std::fs::write(dir.join(file), content).unwrap();
    }
    // On Windows the first look into a folder that was just made writes
    // its time of last access, and a watcher hears of that once: the
    // worktree's folder in the git directory, looked into by the first
    // `git worktree list`. That is the system's doing and not git's, so
    // those folders are looked into before watching.
    for entry in std::fs::read_dir(h.ours.copy.path().join(".git").join("worktrees")).unwrap() {
        let _ = std::fs::read_dir(entry.unwrap().path()).map(Iterator::count);
    }

    let (tx, rx) = mpsc::channel();
    let _watcher = bitbraid_git::Watcher::spawn(
        &h.ours.copy.path().join(".git"),
        Some(h.ours.copy.path()),
        Duration::from_millis(100),
        move |set| {
            let _ = tx.send(set);
        },
    )
    .unwrap();
    // The first set says the watches stand; whatever the setup stirred up
    // comes with it or right after.
    rx.recv_timeout(Duration::from_secs(10)).expect("watching");
    while rx.recv_timeout(Duration::from_millis(500)).is_ok() {}

    for _ in 0..3 {
        let listed = h.run("worktree.list", json!({}));
        assert_eq!(listed.as_array().unwrap().len(), 2);
        let changed = h.run("worktree.changes", json!({}));
        assert_eq!(changed.as_array().unwrap().len(), 1);
        h.run("submodule.list", json!({}));
        h.run("branch.tracking", json!({}));
        h.run("repo.status", json!({}));
    }
    let seen = rx.recv_timeout(Duration::from_millis(900));
    assert!(seen.is_err(), "a question changed something: {seen:?}");
}
