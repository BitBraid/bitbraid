//! Git LFS commands against `git lfs` on twin scratch copies: the command
//! runs on one copy, git-lfs on the other, and `.gitattributes`, the
//! configuration, the hooks and the object stores have to come out the
//! same; what a command reads is compared with what git-lfs prints. The
//! remotes are bare repositories beside each copy, which git-lfs transfers
//! to and from without a server.
//!
//! The commands run git as whoever runs the tests has it set up, with or
//! without LFS filters of their own; the twin runs with a configuration of
//! its own. Nothing here writes outside the scratch directories.
//!
//! Without git-lfs installed there is nothing to compare with: each test
//! then says that it was skipped and passes.
#![allow(clippy::print_stderr)]

use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, atomic::Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

/// The hooks git-lfs installs.
const HOOKS: [&str; 4] = ["pre-push", "post-checkout", "post-commit", "post-merge"];

/// A pointer file, as git-lfs stores one for a file of eleven bytes.
/// (It and the two below are for the test without git-lfs, which runs on
/// Unix only: see there.)
#[cfg(unix)]
const POINTER: &str = "version https://git-lfs.github.com/spec/v1\noid sha256:dace82371525b81729329d6d579bc783ef95cd6a25f635cf1efc594862dbe52c\nsize 11\n";

/// Set in the copy of this test binary that is run without git-lfs.
#[cfg(unix)]
const WITHOUT: &str = "BITBRAID_TEST_WITHOUT_LFS";

fn git_at(dir: impl Into<PathBuf>) -> Git {
    Git::new(dir, fixtures_out_dir().join(".home"))
}

/// Whether the test called `test` has to be skipped: git has no git-lfs.
fn skipped(test: &str) -> bool {
    let found = Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        eprintln!("skipped {test}: git-lfs is not installed");
    }
    !found
}

/// Every file under `dir`, by where it is in it.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
            } else {
                out.push(path.strip_prefix(base).unwrap().display().to_string());
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// The LFS objects a repository at `git_dir` has.
fn store(git_dir: &Path) -> Vec<String> {
    files_under(&git_dir.join("lfs/objects"))
}

#[cfg(unix)]
fn make_executable(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_: &Path) {}

/// A setting as a command run in `dir` finds it: the commands run git with
/// the configuration of whoever runs the tests, but for the system's file,
/// which they do not read (the git crate's `hermetic` feature). A hosted
/// runner sets git-lfs's filter there.
fn found_in(dir: &Path, key: &str) -> Option<String> {
    let out = Command::new("git")
        .current_dir(dir)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .args(["config", "--get", key])
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Whoever runs the tests may keep hooks in a place of their own, for all
/// their repositories. Git-lfs writes its hooks where git looks for them,
/// not only when it is set up but whenever it tracks a pattern or its
/// filter runs. So every repository a command is run in has that place
/// set to its own directory, and theirs is left alone.
fn keep_hooks_here(dir: &Path, git_dir: &Path) {
    if found_in(dir, "core.hooksPath").is_some() {
        let hooks = git_dir.join("hooks");
        git_at(dir)
            .run(["config", "core.hooksPath", hooks.to_str().unwrap()])
            .unwrap();
    }
}

fn write_hook(path: &Path, content: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, content).unwrap();
    make_executable(path);
}

struct Side {
    copy: Scratch,
}

impl Side {
    fn new() -> Self {
        let side = Self {
            copy: Scratch::of("linear").unwrap(),
        };
        side.sweep();
        keep_hooks_here(side.path(), &side.path().join(".git"));
        side
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    fn path(&self) -> &Path {
        self.copy.path()
    }
    /// A directory beside the copy that is removed with it.
    fn beside(&self, name: &str) -> PathBuf {
        self.copy.path().with_extension(format!("lfs-{name}"))
    }
    fn sweep(&self) {
        let (Some(parent), Some(name)) = (self.copy.path().parent(), self.copy.path().file_name())
        else {
            return;
        };
        let prefix = format!("{}.lfs-", name.to_string_lossy());
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    /// A setting as the commands find it: they run git with the
    /// configuration of whoever runs the tests.
    fn found(&self, key: &str) -> Option<String> {
        found_in(self.path(), key)
    }
    /// Whether the commands find the LFS filter set, by the copy or by
    /// whoever runs the tests.
    fn filters_found(&self) -> bool {
        self.found("filter.lfs.clean").is_some()
            && (self.found("filter.lfs.process").is_some()
                || self.found("filter.lfs.smudge").is_some())
    }
    fn install(&self) {
        self.git().run(["lfs", "install", "--local"]).unwrap();
    }
    fn track(&self, pattern: &str) {
        self.git().run(["lfs", "track", "--", pattern]).unwrap();
    }
    fn commit(&self, message: &str) {
        self.git().run(["add", "-A"]).unwrap();
        self.git().run(["commit", "-q", "-m", message]).unwrap();
    }
    fn attributes(&self) -> Option<Vec<u8>> {
        std::fs::read(self.path().join(".gitattributes")).ok()
    }
    /// What the copy's own configuration says about LFS. Git-lfs writes
    /// its settings in no fixed order.
    fn lfs_config(&self) -> Vec<String> {
        let mut out = self
            .git()
            .lines(["config", "--local", "--get-regexp", r"^(filter\.lfs|lfs)\."])
            .unwrap_or_default();
        out.sort();
        out
    }
    /// The hooks in the copy's own directory of them, with what they hold.
    fn hooks(&self) -> Vec<(String, String)> {
        let dir = self.path().join(".git/hooks");
        files_under(&dir)
            .into_iter()
            .filter(|name| !name.ends_with(".sample"))
            .map(|name| {
                let content = std::fs::read(dir.join(&name)).unwrap();
                (name, String::from_utf8_lossy(&content).into_owned())
            })
            .collect()
    }
    /// An empty bare repository beside the copy as its remote `origin`.
    fn add_empty_remote(&self) -> PathBuf {
        let bare = self.beside("origin.git");
        let git = self.git();
        git.run(["init", "-q", "--bare", bare.to_str().unwrap()])
            .unwrap();
        keep_hooks_here(&bare, &bare);
        git.run(["remote", "add", "origin", bare.to_str().unwrap()])
            .unwrap();
        bare
    }
    /// Give the remote the commits of every branch and none of the LFS
    /// content, which the pre-push hook would upload if it were let, and
    /// have `main` follow it.
    fn push_commits(&self) {
        let git = self.git();
        git.run(["push", "-q", "--no-verify", "--all", "origin"])
            .unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        git.run(["branch", "-q", "--set-upstream-to=origin/main", "main"])
            .unwrap();
    }
    /// A remote `origin` that has the commits and none of the content.
    fn add_remote(&self) -> PathBuf {
        let bare = self.add_empty_remote();
        self.push_commits();
        bare
    }
    /// Everything an LFS command can change, as text.
    fn state(&self) -> Vec<String> {
        let git = self.git();
        let attributes = self
            .attributes()
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned());
        let mut out = vec![format!("attributes {attributes:?}")];
        out.extend(self.lfs_config());
        out.extend(
            self.hooks()
                .into_iter()
                .map(|(name, content)| format!("hook {name} {content:?}")),
        );
        out.extend(git.lines(["status", "--porcelain"]).unwrap());
        out.extend(git.lines(["ls-files", "-s"]).unwrap());
        out.extend(
            git.lines(["for-each-ref", "--format=%(refname) %(objectname)"])
                .unwrap(),
        );
        out.extend(
            store(&self.path().join(".git"))
                .into_iter()
                .map(|object| format!("object {object}")),
        );
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
    ctx: std::cell::RefCell<CommandContext>,
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
            ctx: std::cell::RefCell::new(ctx),
        }
    }
    fn both(&self, f: impl Fn(&Side)) {
        f(&self.ours);
        f(&self.twin);
    }
    fn run(&self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .map_err(|e| e.to_string())
    }
    fn refused(&self, id: &str, params: Value, because: &str) {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(because), "{id} {params}: {err}");
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(self.ours.state(), self.twin.state(), "{what}");
    }
    fn status(&self) -> Value {
        self.run("lfs.status", json!({}))
    }
}

/// Commands on another repository than the one a harness opens.
struct Opened {
    registry: Registry,
    ctx: std::cell::RefCell<CommandContext>,
}

impl Opened {
    fn at(path: &Path) -> Self {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": path }))
            .unwrap();
        Self {
            registry,
            ctx: std::cell::RefCell::new(ctx),
        }
    }
    fn run(&self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx.borrow_mut(), &params)
            .map_err(|e| e.to_string())
    }
}

/// The tracked patterns in a status: pattern, source, lockable.
fn tracked_by_us(status: &Value) -> Vec<(String, String, bool)> {
    let mut out: Vec<_> = status["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|pattern| {
            (
                pattern["pattern"].as_str().unwrap().to_owned(),
                pattern["source"].as_str().unwrap().to_owned(),
                pattern["lockable"].as_bool().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// The same as `git lfs track --json` lists them.
fn tracked_by_git_lfs(git: &Git) -> Vec<(String, String, bool)> {
    let said: Value = serde_json::from_str(&git.run(["lfs", "track", "--json"]).unwrap()).unwrap();
    let mut out: Vec<_> = said["patterns"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|pattern| pattern["tracked"] == true)
        .map(|pattern| {
            let source = pattern["source"].as_str().unwrap();
            // On Windows git-lfs names the file with the system's
            // separator; the repository's paths have `/` there.
            let source = if cfg!(windows) {
                source.replace('\\', "/")
            } else {
                source.to_owned()
            };
            (
                pattern["pattern"].as_str().unwrap().to_owned(),
                source,
                pattern["lockable"].as_bool().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// One LFS file: path, object id, size, in the store, in the worktree.
type Listed = (String, String, u64, bool, bool);

fn files_by_us(listed: &Value) -> Vec<Listed> {
    let mut out: Vec<_> = listed["files"]
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            (
                file["path"].as_str().unwrap().to_owned(),
                file["oid"].as_str().unwrap().to_owned(),
                file["size"].as_u64().unwrap(),
                file["present"].as_bool().unwrap(),
                file["checked_out"].as_bool().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

/// The same as `git lfs ls-files --json` lists them for `rev`.
fn files_by_git_lfs(git: &Git, rev: &str) -> Vec<Listed> {
    let said: Value =
        serde_json::from_str(&git.run(["lfs", "ls-files", "--json", rev]).unwrap()).unwrap();
    let mut out: Vec<_> = said["files"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[])
        .iter()
        .map(|file| {
            (
                file["name"].as_str().unwrap().to_owned(),
                file["oid"].as_str().unwrap().to_owned(),
                file["size"].as_u64().unwrap(),
                file["downloaded"].as_bool().unwrap(),
                file["checkout"].as_bool().unwrap(),
            )
        })
        .collect();
    out.sort();
    out
}

fn text(bytes: Option<Vec<u8>>) -> Option<String> {
    bytes.map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
}

#[test]
fn status_tells_how_lfs_stands_before_and_after_it_is_set_up() {
    if skipped("status_tells_how_lfs_stands_before_and_after_it_is_set_up") {
        return;
    }
    let h = Harness::new();
    let own_filters = h.ours.filters_found();

    let before = h.status();
    assert_eq!(before["installed"], true);
    let said = h.ours.git().run(["lfs", "version"]).unwrap();
    let version = before["version"].as_str().unwrap();
    assert!(
        said.starts_with(&format!("git-lfs/{version} ")),
        "{version} is not the version in {said}"
    );
    assert_eq!(before["filters"], own_filters);
    assert_eq!(before["hooks"], false);
    assert_eq!(before["patterns"], json!([]));
    assert_eq!(before["files"], 0);
    assert_eq!(before["active"], false);
    assert_eq!(
        before.as_object().unwrap().len(),
        7,
        "installed, version, hooks, filters, patterns, files, active"
    );
    h.assert_same("before");

    let installed = h.run("lfs.install", json!({}));
    h.twin.install();
    h.assert_same("set up");
    assert_eq!(installed["filters"], true);
    assert_eq!(installed["hooks"], true);
    assert!(installed["hooks_skipped"].is_null());
    assert_eq!(installed["active"], false);
    let names: Vec<String> = h.ours.hooks().into_iter().map(|(name, _)| name).collect();
    let mut expected = HOOKS.map(str::to_owned).to_vec();
    expected.sort();
    assert_eq!(names, expected);
    assert!(
        h.ours
            .lfs_config()
            .contains(&"filter.lfs.clean git-lfs clean -- %f".to_owned()),
        "{:?}",
        h.ours.lfs_config()
    );

    // The status says what setting up answered.
    let mut answered = installed.clone();
    answered.as_object_mut().unwrap().remove("hooks_skipped");
    assert_eq!(h.status(), answered);

    // Once more changes nothing, and none of it is an action to undo.
    h.run("lfs.install", json!({}));
    h.assert_same("set up twice");
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // A linked worktree shares the configuration and the hooks, and has
    // attributes of its own.
    h.ours
        .copy
        .write(
            ".gitattributes",
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
        )
        .unwrap();
    let linked = h.ours.beside("linked");
    h.ours
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            linked.to_str().unwrap(),
            "-b",
            "linked",
        ])
        .unwrap();
    let there = Opened::at(&linked).run("lfs.status", json!({}));
    assert_eq!(there["filters"], true);
    assert_eq!(there["hooks"], true);
    assert_eq!(there["active"], false, "the pattern is not committed");
    assert_eq!(h.status()["active"], true);
}

#[test]
fn setting_up_leaves_hooks_of_the_users_own_alone() {
    if skipped("setting_up_leaves_hooks_of_the_users_own_alone") {
        return;
    }
    let pre_push = |side: &Side| side.path().join(".git/hooks/pre-push");

    // A hook that knows nothing of git-lfs is in the way: nothing is done.
    let h = Harness::new();
    write_hook(&pre_push(&h.ours), "#!/bin/sh\necho mine\n");
    let before = h.ours.state();
    h.refused("lfs.install", json!({}), "pre-push is in the way");
    assert_eq!(h.ours.state(), before);
    assert_eq!(h.status()["hooks"], false);
    assert_eq!(h.status()["filters"], h.ours.filters_found());
    // Tracking goes on all the same, and leaves that hook as it is.
    assert_eq!(
        h.run("lfs.track", json!({ "pattern": "*.bin" }))["changed"],
        true
    );
    assert_eq!(
        h.ours.hooks(),
        [("pre-push".to_owned(), "#!/bin/sh\necho mine\n".to_owned())]
    );
    h.ours.copy.remove(".gitattributes").unwrap();

    // One that calls git-lfs is the user's to keep, and it does its part.
    let merged = "#!/bin/sh\necho mine\ngit lfs pre-push \"$@\"\n";
    write_hook(&pre_push(&h.ours), merged);
    let installed = h.run("lfs.install", json!({}));
    assert_eq!(installed["filters"], true);
    assert_eq!(installed["hooks"], true);
    assert!(installed["hooks_skipped"].is_null());
    assert_eq!(
        std::fs::read_to_string(pre_push(&h.ours)).unwrap(),
        merged,
        "the hook is as the user wrote it"
    );
    h.twin.install();
    assert_eq!(h.ours.lfs_config(), h.twin.lfs_config());
    // Without the mark that lets git run it, it does nothing.
    std::fs::write(pre_push(&h.ours), merged).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(pre_push(&h.ours), std::fs::Permissions::from_mode(0o644))
            .unwrap();
        assert_eq!(h.status()["hooks"], false);
    }

    // Hooks kept outside the repository are not written to.
    let h = Harness::new();
    let outside = h.ours.beside("hooks");
    std::fs::create_dir_all(&outside).unwrap();
    h.ours
        .git()
        .run(["config", "core.hooksPath", outside.to_str().unwrap()])
        .unwrap();
    let installed = h.run("lfs.install", json!({}));
    let why = installed["hooks_skipped"].as_str().unwrap();
    assert!(why.contains("outside this repository"), "{why}");
    assert_eq!(installed["filters"], true);
    assert_eq!(installed["hooks"], false);
    assert_eq!(files_under(&outside), Vec::<String>::new());
    assert_eq!(h.ours.hooks(), Vec::new(), "nor into the copy's own");
    // When the user has put git-lfs into the hook there, that counts.
    write_hook(&outside.join("pre-push"), merged);
    assert_eq!(h.status()["hooks"], true);
    // A link out of the repository is no way around that.
    #[cfg(unix)]
    {
        let h = Harness::new();
        let outside = h.ours.beside("linked-hooks");
        std::fs::create_dir_all(&outside).unwrap();
        let own = h.ours.path().join(".git/hooks");
        let _ = std::fs::remove_dir_all(&own);
        std::os::unix::fs::symlink(&outside, &own).unwrap();
        // Whoever keeps hooks elsewhere had the copy pointed at this
        // directory, which now leads out as well.
        let installed = h.run("lfs.install", json!({}));
        assert!(installed["hooks_skipped"].is_string());
        assert_eq!(files_under(&outside), Vec::<String>::new());

        // Nor is one hook that is a link out: one that calls git-lfs is
        // kept, one that does not is in the way.
        let h = Harness::new();
        let outside = h.ours.beside("linked-hook");
        write_hook(&outside.join("shared"), merged);
        std::fs::create_dir_all(h.ours.path().join(".git/hooks")).unwrap();
        std::os::unix::fs::symlink(outside.join("shared"), pre_push(&h.ours)).unwrap();
        let installed = h.run("lfs.install", json!({}));
        assert_eq!(installed["filters"], true);
        assert_eq!(installed["hooks"], true);
        assert!(installed["hooks_skipped"].is_null());
        assert_eq!(
            std::fs::read_to_string(outside.join("shared")).unwrap(),
            merged
        );
        assert!(pre_push(&h.ours).is_symlink());
        assert_eq!(files_under(&outside), ["shared"]);
        write_hook(&outside.join("shared"), "#!/bin/sh\necho mine\n");
        h.refused("lfs.install", json!({}), "pre-push is in the way");
    }

    // Kept in a folder of the worktree, they are written there.
    let h = Harness::new();
    h.both(|side| {
        side.git()
            .run(["config", "core.hooksPath", ".githooks"])
            .unwrap();
    });
    let installed = h.run("lfs.install", json!({}));
    h.twin.install();
    assert_eq!(installed["hooks"], true);
    assert!(installed["hooks_skipped"].is_null());
    for name in HOOKS {
        let hook = Path::new(".githooks").join(name);
        assert_eq!(
            std::fs::read(h.ours.path().join(&hook)).unwrap(),
            std::fs::read(h.twin.path().join(&hook)).unwrap(),
            "{name}"
        );
    }
    h.assert_same("hooks in a folder of the worktree");
}

#[test]
fn track_and_untrack_match_git_lfs() {
    if skipped("track_and_untrack_match_git_lfs") {
        return;
    }
    let h = Harness::new();
    let twin = h.twin.git();
    let same = |what: &str| {
        assert_eq!(
            text(h.ours.attributes()),
            text(h.twin.attributes()),
            "{what}"
        );
    };
    let track = |pattern: &str, lockable: Value| {
        h.run(
            "lfs.track",
            json!({ "pattern": pattern, "lockable": lockable }),
        )
    };
    let tracked = |pattern: &str, lockable: bool, changed: bool| json!({ "pattern": pattern, "lockable": lockable, "changed": changed });
    let untracked =
        |pattern: &str, changed: bool| json!({ "pattern": pattern, "changed": changed });
    // What the file holds already stays in it.
    let kept = "# kept\n*.txt text\n\n*.png binary\n";
    h.both(|side| side.copy.write(".gitattributes", kept).unwrap());

    assert_eq!(track("*.bin", Value::Null), tracked("*.bin", false, true));
    twin.run(["lfs", "track", "*.bin"]).unwrap();
    same("a pattern");
    // Git-lfs writes its hooks when it tracks a pattern, without being
    // asked to; it is git-lfs that tracks here too, so they are there.
    let names: Vec<String> = h.ours.hooks().into_iter().map(|(name, _)| name).collect();
    assert_eq!(
        names,
        ["post-checkout", "post-commit", "post-merge", "pre-push"]
    );
    assert_eq!(h.ours.hooks(), h.twin.hooks());
    assert_eq!(h.status()["hooks"], true);
    assert_eq!(h.status()["filters"], h.ours.filters_found());
    // Twice is not an error, and changes nothing.
    assert_eq!(track("*.bin", Value::Null), tracked("*.bin", false, false));
    twin.run(["lfs", "track", "*.bin"]).unwrap();
    same("the same pattern again");

    assert_eq!(track("*.psd", json!(true)), tracked("*.psd", true, true));
    twin.run(["lfs", "track", "--lockable", "*.psd"]).unwrap();
    same("a lockable pattern");
    // The mark stays unless something is said about it.
    assert_eq!(track("*.psd", Value::Null), tracked("*.psd", true, false));
    twin.run(["lfs", "track", "*.psd"]).unwrap();
    same("a lockable pattern again");
    assert_eq!(track("*.bin", json!(true)), tracked("*.bin", true, true));
    twin.run(["lfs", "track", "--lockable", "*.bin"]).unwrap();
    same("made lockable");
    assert_eq!(track("*.bin", json!(false)), tracked("*.bin", false, true));
    twin.run(["lfs", "track", "--not-lockable", "*.bin"])
        .unwrap();
    same("no longer lockable");
    assert_eq!(track("*.bin", json!(false)), tracked("*.bin", false, false));

    // Names that git-lfs has to write differently. On Windows git-lfs
    // takes `\` for a separator (`back/slash`), and writes such a pattern
    // again each time it is tracked: that name is tried elsewhere only.
    let backslash = !cfg!(windows);
    for pattern in [
        "my file.dat",
        "a#b",
        "back\\slash",
        "/top.raw",
        "assets/**",
        "ünï/*.x",
    ] {
        if pattern.contains('\\') && !backslash {
            continue;
        }
        assert_eq!(track(pattern, Value::Null), tracked(pattern, false, true));
        twin.run(["lfs", "track", "--", pattern]).unwrap();
        same(pattern);
        assert_eq!(
            track(pattern, Value::Null),
            tracked(pattern, false, false),
            "{pattern} is found tracked"
        );
    }
    let status = h.status();
    assert_eq!(status["active"], true);
    assert_eq!(tracked_by_us(&status), tracked_by_git_lfs(&twin));
    assert_eq!(tracked_by_us(&status).len(), if backslash { 8 } else { 7 });
    // Left changed and not staged, for the user to commit.
    for side in [&h.ours, &h.twin] {
        assert!(
            side.git()
                .lines(["status", "--porcelain"])
                .unwrap()
                .contains(&"?? .gitattributes".to_owned())
        );
    }

    assert_eq!(
        h.run("lfs.untrack", json!({ "pattern": "*.psd" })),
        untracked("*.psd", true)
    );
    twin.run(["lfs", "untrack", "*.psd"]).unwrap();
    same("untracked");
    // What is not tracked is not an error.
    for pattern in ["*.psd", "*.never"] {
        assert_eq!(
            h.run("lfs.untrack", json!({ "pattern": pattern })),
            untracked(pattern, false)
        );
        twin.run(["lfs", "untrack", pattern]).unwrap();
        same(pattern);
    }
    // As the file has it, which is how the status lists it, and as it
    // was typed.
    for (ours, theirs) in [
        ("my[[:space:]]file.dat", "my file.dat"),
        ("a\\#b", "a#b"),
        ("back\\\\slash", "back\\slash"),
        ("/top.raw", "/top.raw"),
        ("assets/**", "assets/**"),
        ("ünï/*.x", "ünï/*.x"),
        ("*.bin", "*.bin"),
    ] {
        if theirs.contains('\\') && !backslash {
            continue;
        }
        assert_eq!(
            h.run("lfs.untrack", json!({ "pattern": ours })),
            untracked(ours, true)
        );
        twin.run(["lfs", "untrack", "--", theirs]).unwrap();
        same(ours);
    }
    // What was there before is there still. (Git-lfs writes the file
    // anew when it changes a line, and leaves empty lines out.)
    assert_eq!(
        text(h.ours.attributes()).unwrap(),
        "# kept\n*.txt text\n*.png binary\n"
    );
    assert_eq!(h.status()["patterns"], json!([]));
    assert_eq!(h.status()["active"], false);

    // Without the file there is nothing to untrack, and none is made.
    h.both(|side| side.copy.remove(".gitattributes").unwrap());
    assert_eq!(
        h.run("lfs.untrack", json!({ "pattern": "*.bin" })),
        untracked("*.bin", false)
    );
    twin.run(["lfs", "untrack", "*.bin"]).unwrap();
    same("no file");
    assert!(h.ours.attributes().is_none());
    // And tracking makes it.
    assert_eq!(track("*.bin", Value::Null), tracked("*.bin", false, true));
    twin.run(["lfs", "track", "*.bin"]).unwrap();
    same("a new file");
    h.assert_same("at the end");
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // What git-lfs was run for is in the activity log, like any git call.
    let log = h.run("activity.list", json!({ "text": "lfs track", "limit": 1 }));
    let last = &log["entries"][0];
    assert_eq!(last["command"], "git lfs track -- *.bin");
    assert_eq!(last["origin"], "lfs.track");
    assert_eq!(last["question"], false);
    assert_eq!(last["code"], 0);
}

#[test]
fn status_lists_the_patterns_of_every_attribute_file() {
    if skipped("status_lists_the_patterns_of_every_attribute_file") {
        return;
    }
    let h = Harness::new();
    let side = &h.ours;
    let git = side.git();
    side.copy
        .write(
            ".gitattributes",
            concat!(
                "# written by hand\n",
                "*.bin filter=lfs diff=lfs merge=lfs -text\n",
                "*.psd\tfilter=lfs\tlockable\n",
                "   *.indented   filter=lfs   \n",
                "\"quoted name.dat\" filter=lfs\n",
                "[attr]big filter=lfs diff=lfs merge=lfs -text\n",
                "*.mac big\n",
                "*.txt text\n",
                "*.other filter=other\n",
                "*.unset -filter\n",
                "*.caps FILTER=lfs\n",
                "*.crlf filter=lfs\r\n",
            ),
        )
        .unwrap();
    side.copy
        .write(
            "assets/.gitattributes",
            concat!(
                "*.big filter=lfs lockable\n",
                "/anchored.raw filter=lfs\n",
                "deep/*.d filter=lfs -lockable\n",
                "**/any.x filter=lfs\n",
            ),
        )
        .unwrap();
    side.copy
        .write("a folder/in it/.gitattributes", "*.x filter=lfs\n")
        .unwrap();
    git.run(["add", "-A"]).unwrap();

    let status = h.status();
    let ours = tracked_by_us(&status);
    // Git-lfs lists them the same, but for the line that has tabs between
    // its words: git reads that one, the list of git-lfs leaves it out.
    let tabbed = ("*.psd".to_owned(), ".gitattributes".to_owned(), true);
    let by_git_lfs = || {
        let mut theirs = tracked_by_git_lfs(&git);
        assert!(!theirs.contains(&tabbed));
        theirs.push(tabbed.clone());
        theirs.sort();
        theirs
    };
    assert_eq!(ours, by_git_lfs());
    // What counts is what git makes of the files: a file is given to the
    // filter exactly when a pattern that is listed names it.
    for (pattern, file) in [
        ("*.bin", "x.bin"),
        ("*.psd", "x.psd"),
        ("*.indented", "x.indented"),
        ("quoted name.dat", "quoted name.dat"),
        ("*.mac", "x.mac"),
        ("*.crlf", "x.crlf"),
        ("*.txt", "x.txt"),
        ("*.other", "x.other"),
        ("*.unset", "x.unset"),
        ("*.caps", "x.caps"),
        ("assets/*.big", "assets/x.big"),
        ("assets/anchored.raw", "assets/anchored.raw"),
        ("assets/deep/*.d", "assets/deep/x.d"),
        ("assets/**/any.x", "assets/a/b/any.x"),
        ("a folder/in it/*.x", "a folder/in it/y.x"),
    ] {
        let said = |attribute: &str| {
            let line = git.run(["check-attr", attribute, "--", file]).unwrap();
            line.trim()
                .rsplit_once(": ")
                .map(|(_, value)| value.to_owned())
                .unwrap()
        };
        let listed = ours.iter().find(|(found, _, _)| found == pattern);
        assert_eq!(listed.is_some(), said("filter") == "lfs", "{pattern}");
        if let Some((_, _, lockable)) = listed {
            assert_eq!(*lockable, said("lockable") == "set", "{pattern}");
        }
    }
    let has = |pattern: &str, source: &str, lockable: bool| {
        ours.contains(&(pattern.to_owned(), source.to_owned(), lockable))
    };
    assert!(has("*.bin", ".gitattributes", false));
    assert!(has("*.psd", ".gitattributes", true));
    assert!(has("quoted name.dat", ".gitattributes", false));
    assert!(has("*.mac", ".gitattributes", false));
    assert!(has("assets/*.big", "assets/.gitattributes", true));
    assert!(has("assets/anchored.raw", "assets/.gitattributes", false));
    assert!(has(
        "a folder/in it/*.x",
        "a folder/in it/.gitattributes",
        false
    ));
    assert_eq!(ours.len(), 11);
    assert_eq!(status["active"], true);
    assert_eq!(status["files"], 0);
    // The top of the worktree first, then its folders.
    assert_eq!(status["patterns"][0]["pattern"], "*.bin");

    // The repository's own attributes count as well.
    std::fs::create_dir_all(side.path().join(".git/info")).unwrap();
    std::fs::write(
        side.path().join(".git/info/attributes"),
        "*.info filter=lfs lockable\n*.viamacro big\n",
    )
    .unwrap();
    let with_info = tracked_by_us(&h.status());
    assert_eq!(with_info.len(), 13);
    for pattern in ["*.info", "*.viamacro"] {
        assert!(
            with_info
                .iter()
                .any(|(found, source, lockable)| found == pattern
                    && source == ".git/info/attributes"
                    && *lockable == (pattern == "*.info")),
            "{with_info:?}"
        );
    }
    std::fs::remove_file(side.path().join(".git/info/attributes")).unwrap();

    // A folder's file that was never added is not found until it is.
    side.copy
        .write("later/.gitattributes", "*.late filter=lfs\n")
        .unwrap();
    assert_eq!(tracked_by_us(&h.status()), ours);
    git.run(["add", "later/.gitattributes"]).unwrap();
    assert_eq!(tracked_by_us(&h.status()), by_git_lfs());
    assert_eq!(tracked_by_us(&h.status()).len(), 12);

    // An attribute file that is not checked out counts as the index has
    // it: that is what git goes by then.
    side.commit("attribute files");
    let all = tracked_by_us(&h.status());
    let says = |file: &str| git.run(["check-attr", "filter", "--", file]).unwrap();
    for file in [".gitattributes", "assets/.gitattributes"] {
        side.copy.remove(file).unwrap();
    }
    assert_eq!(tracked_by_us(&h.status()), all);
    assert!(says("x.bin").contains("filter: lfs"));
    assert!(says("assets/x.big").contains("filter: lfs"));
    // Staged as gone, it is gone.
    git.run([
        "rm",
        "-q",
        "--cached",
        ".gitattributes",
        "assets/.gitattributes",
    ])
    .unwrap();
    let left: Vec<String> = tracked_by_us(&h.status())
        .into_iter()
        .map(|(_, source, _)| source)
        .collect();
    assert_eq!(
        left,
        ["a folder/in it/.gitattributes", "later/.gitattributes"]
    );
    assert!(says("x.bin").contains("filter: unspecified"));
    assert!(says("assets/x.big").contains("filter: unspecified"));
}

#[test]
fn files_are_listed_as_git_lfs_lists_them() {
    if skipped("files_are_listed_as_git_lfs_lists_them") {
        return;
    }
    let h = Harness::new();
    let side = &h.ours;
    let git = side.git();
    side.install();
    assert_eq!(
        h.run("lfs.track", json!({ "pattern": "*.bin" }))["changed"],
        true
    );
    side.copy.write("one.bin", "binary one\n").unwrap();
    side.copy.write("sub/two.bin", "binary two two\n").unwrap();
    side.copy.write("my file.bin", "with a space\n").unwrap();
    side.copy.write("empty.bin", "").unwrap();
    side.copy.write("notes.txt", "not for LFS\n").unwrap();
    side.commit("files through the filter");

    // What git stores is the pointer; the worktree has the content.
    let stored = git.run(["cat-file", "-p", "HEAD:one.bin"]).unwrap();
    assert!(
        stored.starts_with("version https://git-lfs.github.com/spec/v1\noid sha256:"),
        "{stored}"
    );
    assert_eq!(
        std::fs::read_to_string(side.path().join("one.bin")).unwrap(),
        "binary one\n"
    );

    let listed = h.run("lfs.files", json!({}));
    assert_eq!(listed["commit"], git.rev_parse("HEAD").unwrap());
    assert_eq!(listed["more"], false);
    let ours = files_by_us(&listed);
    assert_eq!(ours, files_by_git_lfs(&git, "HEAD"));
    let paths: Vec<&str> = ours.iter().map(|file| file.0.as_str()).collect();
    assert_eq!(paths, ["my file.bin", "one.bin", "sub/two.bin"]);
    let one = &ours[1];
    assert!(stored.contains(&format!("oid sha256:{}\n", one.1)));
    assert_eq!((one.2, one.3, one.4), (11, true, true));

    // A file the pattern matches that was stored as it is, is not one.
    side.copy.write("raw.bin", "stored as it is\n").unwrap();
    git.run([
        "-c",
        "filter.lfs.clean=",
        "-c",
        "filter.lfs.process=",
        "-c",
        "filter.lfs.required=false",
        "add",
        "raw.bin",
    ])
    .unwrap();
    git.run(["commit", "-q", "-m", "a file past the filter"])
        .unwrap();
    assert_eq!(
        git.run(["cat-file", "-p", "HEAD:raw.bin"]).unwrap(),
        "stored as it is\n"
    );
    let status = h.status();
    assert_eq!(status["active"], true);
    assert_eq!(status["files"], 3);
    assert_eq!(git.lines(["lfs", "ls-files"]).unwrap().len(), 3);

    // By revision: a name, a commit before, an id.
    let head = git.rev_parse("HEAD").unwrap();
    for rev in ["HEAD", "main", "HEAD~1", "HEAD~2", head.as_str()] {
        let listed = h.run("lfs.files", json!({ "rev": rev }));
        assert_eq!(listed["commit"], git.rev_parse(rev).unwrap(), "{rev}");
        assert_eq!(files_by_us(&listed), files_by_git_lfs(&git, rev), "{rev}");
    }
    assert_eq!(
        h.run("lfs.files", json!({ "rev": "HEAD~2" }))["files"],
        json!([])
    );

    // Content that is gone from the store, and a file that holds the
    // pointer again.
    let gone = side
        .path()
        .join(".git/lfs/objects")
        .join(&one.1[..2])
        .join(&one.1[2..4])
        .join(&one.1);
    std::fs::remove_file(&gone).unwrap();
    std::fs::write(
        side.path().join("sub/two.bin"),
        git.run(["cat-file", "-p", "HEAD:sub/two.bin"]).unwrap(),
    )
    .unwrap();
    let after = files_by_us(&h.run("lfs.files", json!({})));
    assert_eq!(after, files_by_git_lfs(&git, "HEAD"));
    assert_eq!((after[1].3, after[1].4), (false, true), "{:?}", after[1]);
    assert_eq!((after[2].3, after[2].4), (true, false), "{:?}", after[2]);

    // A branch without commits has none.
    git.run(["checkout", "-q", "--orphan", "empty"]).unwrap();
    git.run(["rm", "-rqf", "--cached", "."]).unwrap();
    let none = h.run("lfs.files", json!({}));
    assert_eq!(none, json!({ "commit": null, "files": [], "more": false }));
    assert!(h.try_run("lfs.files", json!({ "rev": "HEAD" })).is_err());
}

/// LFS set up, two LFS files on `main`, one more on `side`, and a remote
/// that has nothing yet.
fn with_remote(side: &Side) {
    side.install();
    side.track("*.bin");
    side.copy.write("one.bin", "binary one\n").unwrap();
    side.copy.write("sub/two.bin", "binary two two\n").unwrap();
    side.commit("lfs files on main");
    side.git().run(["checkout", "-q", "-b", "side"]).unwrap();
    side.copy
        .write("three.bin", "a third, on a branch\n")
        .unwrap();
    side.commit("an lfs file on side");
    side.git().run(["checkout", "-q", "main"]).unwrap();
    side.add_empty_remote();
}

#[test]
fn content_is_pushed_fetched_and_pulled_as_git_lfs_does_it() {
    if skipped("content_is_pushed_fetched_and_pulled_as_git_lfs_does_it") {
        return;
    }
    let h = Harness::new();
    h.both(with_remote);
    let remote_store = |side: &Side| store(&side.beside("origin.git"));
    assert_eq!(remote_store(&h.ours), Vec::<String>::new());
    assert_eq!(store(&h.ours.path().join(".git")).len(), 3);
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let sink = seen.clone();
    h.ctx.borrow_mut().progress = Some(Arc::new(move |progress| {
        sink.lock().unwrap().push(progress.stage);
    }));

    // The branch that is checked out, to the only remote there is. What
    // the remote is known to have already is not sent again, so this is
    // before it has the commits.
    let pushed = h.run("lfs.push", json!({}));
    assert_eq!(pushed["remote"], "origin");
    h.twin.git().run(["lfs", "push", "origin", "main"]).unwrap();
    assert_eq!(remote_store(&h.ours), remote_store(&h.twin));
    assert_eq!(remote_store(&h.ours).len(), 2);
    // What git-lfs reported was passed on.
    assert!(
        seen.lock()
            .unwrap()
            .contains(&"Uploading LFS objects".to_owned()),
        "{pushed}"
    );
    // Every branch.
    h.run("lfs.push", json!({ "remote": "origin", "all": true }));
    h.twin
        .git()
        .run(["lfs", "push", "--all", "origin"])
        .unwrap();
    assert_eq!(remote_store(&h.ours), remote_store(&h.twin));
    assert_eq!(remote_store(&h.ours), store(&h.ours.path().join(".git")));
    // With HEAD on no branch, what is checked out is pushed.
    h.ours.git().run(["checkout", "-q", "--detach"]).unwrap();
    h.run("lfs.push", json!({ "remote": "origin" }));
    h.ours.git().run(["checkout", "-q", "main"]).unwrap();
    h.assert_same("pushed");
    h.both(Side::push_commits);

    // A clone made without the filter has the pointers and no content.
    let clone = |side: &Side| {
        let dir = side.beside("clone");
        side.git()
            .run([
                "clone",
                "-q",
                side.beside("origin.git").to_str().unwrap(),
                dir.to_str().unwrap(),
            ])
            .unwrap();
        keep_hooks_here(&dir, &dir.join(".git"));
        git_at(dir)
    };
    let (ours, twin) = (clone(&h.ours), clone(&h.twin));
    let there = Opened::at(ours.cwd());
    let content = |git: &Git, file: &str| std::fs::read_to_string(git.cwd().join(file)).unwrap();
    let local = |git: &Git| store(&git.cwd().join(".git"));
    let flags = |listed: &Value| -> Vec<(bool, bool)> {
        files_by_us(listed)
            .into_iter()
            .map(|file| (file.3, file.4))
            .collect()
    };
    assert!(content(&ours, "one.bin").starts_with("version https://git-lfs"));
    let listed = there.run("lfs.files", json!({}));
    assert_eq!(files_by_us(&listed), files_by_git_lfs(&ours, "HEAD"));
    assert_eq!(flags(&listed), [(false, false), (false, false)]);
    let status = there.run("lfs.status", json!({}));
    assert_eq!(status["files"], 2);
    assert_eq!(status["active"], true);

    // Pulling needs the filter, or the pointers would stay.
    let own_filters = h.ours.filters_found();
    if !own_filters {
        let err = there.try_run("lfs.pull", json!({})).unwrap_err();
        assert!(err.contains("LFS is not set up"), "{err}");
        assert_eq!(local(&ours), Vec::<String>::new());
    }

    // Fetching brings the content and leaves the files alone.
    let fetched = there.run("lfs.fetch", json!({}));
    assert_eq!(fetched["remote"], "origin");
    twin.run(["lfs", "fetch", "origin"]).unwrap();
    assert_eq!(local(&ours), local(&twin));
    assert_eq!(local(&ours).len(), 2);
    assert!(content(&ours, "one.bin").starts_with("version https://git-lfs"));
    let listed = there.run("lfs.files", json!({}));
    assert_eq!(files_by_us(&listed), files_by_git_lfs(&ours, "HEAD"));
    assert_eq!(flags(&listed), [(true, false), (true, false)]);
    // Of every branch.
    there.run("lfs.fetch", json!({ "remote": "origin", "all": true }));
    twin.run(["lfs", "fetch", "--all", "origin"]).unwrap();
    assert_eq!(local(&ours), local(&twin));
    assert_eq!(local(&ours), remote_store(&h.ours));

    // Pulling puts the content in place, fetching it when it is not here.
    there.run("lfs.install", json!({}));
    twin.run(["lfs", "install", "--local"]).unwrap();
    for git in [&ours, &twin] {
        std::fs::remove_dir_all(git.cwd().join(".git/lfs/objects")).unwrap();
    }
    let pulled = there.run("lfs.pull", json!({ "remote": "origin" }));
    assert_eq!(pulled["remote"], "origin");
    twin.run(["lfs", "pull", "origin"]).unwrap();
    assert_eq!(local(&ours), local(&twin));
    assert_eq!(local(&ours).len(), 2);
    for (file, expected) in [
        ("one.bin", "binary one\n"),
        ("sub/two.bin", "binary two two\n"),
    ] {
        assert_eq!(content(&ours, file), expected);
        assert_eq!(content(&twin, file), expected);
    }
    assert_eq!(
        ours.lines(["status", "--porcelain"]).unwrap(),
        twin.lines(["status", "--porcelain"]).unwrap()
    );
    let listed = there.run("lfs.files", json!({}));
    assert_eq!(files_by_us(&listed), files_by_git_lfs(&ours, "HEAD"));
    assert_eq!(flags(&listed), [(true, true), (true, true)]);
    for stage in seen.lock().unwrap().iter() {
        assert!(
            stage.chars().next().unwrap().is_ascii_uppercase(),
            "{stage}"
        );
    }
}

#[test]
fn transfers_and_locks_refuse_what_cannot_work_and_stop_when_asked() {
    if skipped("transfers_and_locks_refuse_what_cannot_work_and_stop_when_asked") {
        return;
    }
    let h = Harness::new();
    let side = &h.ours;
    side.install();
    side.git()
        .run(["lfs", "track", "--lockable", "*.bin"])
        .unwrap();
    side.copy.write("one.bin", "binary one\n").unwrap();
    side.copy.write("folder/kept.txt", "kept\n").unwrap();
    side.commit("an lfs file");

    // Nowhere to go and nobody to ask.
    let before = side.state();
    for (command, params) in [
        ("lfs.fetch", json!({})),
        ("lfs.fetch", json!({ "all": true })),
        ("lfs.pull", json!({})),
        ("lfs.push", json!({})),
        ("lfs.push", json!({ "all": true })),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin" })),
    ] {
        h.refused(command, params, "has no remote");
    }
    for command in ["lfs.fetch", "lfs.pull", "lfs.push"] {
        h.refused(
            command,
            json!({ "remote": "origin" }),
            "there is no remote called",
        );
    }
    assert_eq!(side.state(), before);

    let bare = side.add_remote();
    let before = side.state();
    for command in ["lfs.fetch", "lfs.pull", "lfs.push"] {
        h.refused(
            command,
            json!({ "remote": "elsewhere" }),
            "there is no remote called",
        );
        // A remote is named, not addressed.
        h.refused(command, json!({ "remote": bare }), "");
    }

    // Asked to stop before it starts: nothing is run.
    h.ctx.borrow().cancel.store(true, Ordering::SeqCst);
    for (command, params) in [
        ("lfs.fetch", json!({})),
        ("lfs.pull", json!({})),
        ("lfs.checkout", json!({})),
        ("lfs.push", json!({})),
        ("lfs.push", json!({ "all": true })),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": true })),
    ] {
        h.refused(command, params, "cancelled");
    }
    h.ctx.borrow().cancel.store(false, Ordering::SeqCst);
    assert_eq!(store(&bare), Vec::<String>::new());
    assert_eq!(side.state(), before);

    // A repository on the file system keeps no locks: asking fails, and
    // that is an error rather than an empty list.
    for (command, params) in [
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": true })),
    ] {
        let err = h
            .try_run(command, params.clone())
            .expect_err(&format!("{command} {params}"));
        assert!(!err.contains("cancelled"), "{command}: {err}");
    }
    // Only a file that is there is locked.
    for path in ["nowhere.bin", "folder", "folder/nowhere.bin"] {
        h.refused("lfs.lock", json!({ "path": path }), "is not a file here");
    }
    assert_eq!(side.state(), before);

    // A remote that is not there: the failure comes back as one.
    let gone = side.beside("gone.git");
    side.git()
        .run(["remote", "add", "gone", gone.to_str().unwrap()])
        .unwrap();
    std::fs::remove_dir_all(side.path().join(".git/lfs/objects")).unwrap();
    assert!(h.try_run("lfs.fetch", json!({ "remote": "gone" })).is_err());
    assert!(h.try_run("lfs.pull", json!({ "remote": "gone" })).is_err());
    assert_eq!(store(&side.path().join(".git")), Vec::<String>::new());
    assert!(!gone.exists());
}

/// A transfer program for git-lfs that the tests can slow down. The remote
/// is the object store of a local bare repository, laid out as git-lfs
/// lays it out, so that what it moves can be compared with what git-lfs
/// moves by itself. Of each file it reports half, waits, and then moves
/// it. It notes who ran: itself, the git-lfs that started it, and what it
/// waits with.
///
/// Its arguments: the store, how many seconds to wait, a directory of its
/// own. It is run by `sh` and never as a program itself, so that no test
/// writes a file that is run.
#[cfg(unix)]
const AGENT: &str = r#"
store="$1"; pause="$2"; scratch="$3"
field() { printf '%s' "$1" | sed -n 's/.*"'"$2"'":"\([^"]*\)".*/\1/p'; }
number() { printf '%s' "$1" | sed -n 's/.*"'"$2"'":\([0-9]*\).*/\1/p'; }
place() {
  printf '%s/%s/%s/%s' "$store" "$(printf '%s' "$1" | cut -c1-2)" "$(printf '%s' "$1" | cut -c3-4)" "$1"
}
echo "$$" >> "$scratch/ran"
echo "$PPID" >> "$scratch/ran"
while IFS= read -r line; do
  case "$line" in
    *'"event":"init"'*) printf '{}\n' ;;
    *'"event":"terminate"'*) exit 0 ;;
    *'"event":"download"'*|*'"event":"upload"'*)
      oid=$(field "$line" oid); size=$(number "$line" size); half=$((size / 2))
      printf '{"event":"progress","oid":"%s","bytesSoFar":%s,"bytesSinceLast":%s}\n' "$oid" "$half" "$half"
      sleep "$pause" &
      echo "$!" >> "$scratch/ran"
      wait "$!"
      there=$(place "$oid")
      case "$line" in
        *'"event":"download"'*)
          cp "$there" "$scratch/$oid.$$" &&
            printf '{"event":"complete","oid":"%s","path":"%s"}\n' "$oid" "$scratch/$oid.$$" ;;
        *)
          mkdir -p "$(dirname "$there")" && cp "$(field "$line" path)" "$there" &&
            printf '{"event":"complete","oid":"%s"}\n' "$oid" ;;
      esac ;;
  esac
done
"#;

/// The transfer program above, set for one repository in that
/// repository's own configuration.
#[cfg(unix)]
struct Agent {
    git: Git,
    script: PathBuf,
    store: PathBuf,
    scratch: PathBuf,
}

#[cfg(unix)]
impl Agent {
    /// For the repository `git` runs in, with `remote` as the bare
    /// repository it moves content to and from, and `dir` to keep its own
    /// files in.
    fn set(git: Git, remote: &Path, dir: PathBuf) -> Self {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("agent.sh");
        std::fs::write(&script, AGENT).unwrap();
        let agent = Self {
            git,
            script,
            store: remote.join("lfs/objects"),
            scratch: dir,
        };
        for (key, value) in [
            ("lfs.customtransfer.slow.path", "sh"),
            ("lfs.customtransfer.slow.concurrent", "false"),
            ("lfs.standalonetransferagent", "slow"),
        ] {
            agent.git.run(["config", key, value]).unwrap();
        }
        agent.waits(0);
        agent
    }

    /// Have it wait this many seconds over each file.
    fn waits(&self, seconds: u32) {
        let args = format!(
            "'{}' '{}' {seconds} '{}'",
            self.script.display(),
            self.store.display(),
            self.scratch.display()
        );
        self.git
            .run(["config", "lfs.customtransfer.slow.args", &args])
            .unwrap();
    }

    /// Whether a process still runs; one that has ended and was not
    /// collected yet does not.
    fn runs(pid: &str) -> bool {
        std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| {
            let state = stat.rsplit(')').next().unwrap_or("").trim_start();
            !state.starts_with('Z') && !state.starts_with('X')
        })
    }

    /// The program ran, and nothing of it is left: not itself, not the
    /// git-lfs that started it, not what it waited with.
    fn assert_ran_and_is_gone(&self, what: &str) {
        let noted = self.scratch.join("ran");
        let ran = std::fs::read_to_string(&noted).unwrap_or_default();
        let ran: Vec<&str> = ran.lines().collect();
        assert!(ran.len() >= 3, "{what}: the agent did not run: {ran:?}");
        std::fs::remove_file(&noted).unwrap();
        if !Path::new("/proc/self/stat").exists() {
            return;
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let left: Vec<&&str> = ran.iter().filter(|pid| Self::runs(pid)).collect();
            if left.is_empty() {
                return;
            }
            assert!(
                std::time::Instant::now() < until,
                "{what}: still running: {left:?}"
            );
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

/// What was reported: each stage with its figure, in the order it came.
type Seen = Mutex<Vec<(String, Option<u8>)>>;

/// What the commands of a context report from now on. With `stop_in`, the
/// context is asked to stop as soon as that stage is at a figure between
/// nothing and everything.
fn watch(ctx: &mut CommandContext, stop_in: Option<&'static str>) -> Arc<Seen> {
    let seen: Arc<Seen> = Arc::default();
    let (sink, cancel) = (seen.clone(), ctx.cancel.clone());
    ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
        if stop_in == Some(progress.stage.as_str())
            && progress.percent.is_some_and(|at| at > 0 && at < 100)
        {
            cancel.store(true, Ordering::SeqCst);
        }
        sink.lock()
            .unwrap()
            .push((progress.stage, progress.percent));
    }));
    seen
}

/// The figures a stage was at, in the order they were reported.
fn figures(seen: &Seen, stage: &str) -> Vec<u8> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(reported, _)| reported == stage)
        .filter_map(|(_, percent)| *percent)
        .collect()
}

/// Nothing a transfer reports to is left in a git directory.
fn assert_no_report_left(git_dir: &Path) {
    let left: Vec<String> = files_under(&git_dir.join("bitbraid"))
        .into_iter()
        .filter(|name| name.ends_with(".progress"))
        .collect();
    assert_eq!(left, Vec::<String>::new());
}

#[cfg(unix)]
#[test]
fn a_transfer_says_how_far_it_is_and_ends_when_it_is_stopped() {
    let name = "a_transfer_says_how_far_it_is_and_ends_when_it_is_stopped";
    if skipped(name) {
        return;
    }
    const UP: &str = "Uploading LFS objects";
    const DOWN: &str = "Downloading LFS objects";
    const OUT: &str = "Checking out LFS objects";
    // Far longer than the test may take: a transfer that is not stopped
    // is one that fails here.
    const SLOW: u32 = 20;
    let in_time = std::time::Duration::from_secs(10);
    let h = Harness::new();
    h.both(with_remote);
    let remote_store = |side: &Side| store(&side.beside("origin.git"));
    let agent = Agent::set(
        h.ours.git(),
        &h.ours.beside("origin.git"),
        h.ours.beside("agent"),
    );

    // An upload that takes its time is stopped at the first figure in
    // between, which is of the bytes of a file that is under way.
    agent.waits(SLOW);
    let before = h.ours.state();
    let seen = watch(&mut h.ctx.borrow_mut(), Some(UP));
    let started = std::time::Instant::now();
    h.refused("lfs.push", json!({}), "cancelled");
    assert!(started.elapsed() < in_time, "{:?}", started.elapsed());
    let at = figures(&seen, UP);
    assert!(
        at.iter().any(|at| *at > 0 && *at < 100),
        "no figure in between: {:?}",
        seen.lock().unwrap()
    );
    assert!(at.is_sorted(), "{at:?}");
    agent.assert_ran_and_is_gone("push");
    assert_eq!(remote_store(&h.ours), Vec::<String>::new());
    assert_eq!(h.ours.state(), before);
    assert_no_report_left(&h.ours.path().join(".git"));
    // The run is in the activity log as one that was stopped.
    let log = h.run("activity.list", json!({}));
    let stopped = log["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| {
            entry["command"]
                .as_str()
                .is_some_and(|command| command.contains("lfs push"))
        })
        .unwrap_or_else(|| panic!("no push in {log}"));
    assert_eq!(stopped["end"], "cancelled", "{stopped}");

    // Left to itself it goes through, and says so up to the end. The
    // remote then has what git-lfs by itself gives it.
    h.ctx.borrow().cancel.store(false, Ordering::SeqCst);
    agent.waits(0);
    let seen = watch(&mut h.ctx.borrow_mut(), None);
    let pushed = h.run("lfs.push", json!({}));
    h.twin.git().run(["lfs", "push", "origin", "main"]).unwrap();
    assert_eq!(remote_store(&h.ours), remote_store(&h.twin));
    assert_eq!(remote_store(&h.ours).len(), 2);
    for object in remote_store(&h.ours) {
        let of = |side: &Side| {
            std::fs::read(side.beside("origin.git").join("lfs/objects").join(&object)).unwrap()
        };
        assert_eq!(of(&h.ours), of(&h.twin), "{object}");
    }
    let at = figures(&seen, UP);
    assert!(at.is_sorted(), "{at:?}");
    assert_eq!(at.last(), Some(&100), "{:?}", seen.lock().unwrap());
    // Of the line that was rewritten in place, only what it said last is
    // kept, and only when that is the end.
    let said = pushed["output"].as_str().unwrap();
    assert!(said.matches(UP).count() <= 1, "{said}");
    assert!(!said.contains(UP) || said.contains("100% (2/2)"), "{said}");
    agent.assert_ran_and_is_gone("push to the end");
    assert_no_report_left(&h.ours.path().join(".git"));
    h.both(Side::push_commits);

    // A clone with the pointers and no content, of each.
    let clone = |side: &Side| {
        let dir = side.beside("clone");
        side.git()
            .run([
                "clone",
                "-q",
                side.beside("origin.git").to_str().unwrap(),
                dir.to_str().unwrap(),
            ])
            .unwrap();
        keep_hooks_here(&dir, &dir.join(".git"));
        git_at(dir)
    };
    let (ours, twin) = (clone(&h.ours), clone(&h.twin));
    let there = Opened::at(ours.cwd());
    let content = |git: &Git, file: &str| std::fs::read_to_string(git.cwd().join(file)).unwrap();
    let local = |git: &Git| store(&git.cwd().join(".git"));
    let is_pointer = |git: &Git, file: &str| content(git, file).starts_with("version https://git");
    let agent = Agent::set(
        git_at(ours.cwd()),
        &h.ours.beside("origin.git"),
        h.ours.beside("agent-of-clone"),
    );

    // Putting content in place needs the filter, as pulling does.
    if !h.ours.filters_found() {
        let err = there.try_run("lfs.checkout", json!({})).unwrap_err();
        assert!(err.contains("LFS is not set up"), "{err}");
    }
    there.run("lfs.install", json!({}));
    twin.run(["lfs", "install", "--local"]).unwrap();
    // Content that is not here is not put in place, and is not fetched.
    let none = there.run("lfs.checkout", json!({}));
    assert!(
        none["output"].as_str().unwrap().contains("one.bin"),
        "{none}"
    );
    assert!(is_pointer(&ours, "one.bin"));
    assert_eq!(local(&ours), Vec::<String>::new());
    assert!(!agent.scratch.join("ran").exists());

    // A download that takes its time is stopped in the same way, whether
    // it is a fetch or a pull.
    agent.waits(SLOW);
    for command in ["lfs.fetch", "lfs.pull"] {
        let seen = watch(&mut there.ctx.borrow_mut(), Some(DOWN));
        let started = std::time::Instant::now();
        let err = there.try_run(command, json!({})).unwrap_err();
        assert!(err.contains("cancelled"), "{command}: {err}");
        assert!(started.elapsed() < in_time, "{:?}", started.elapsed());
        there.ctx.borrow().cancel.store(false, Ordering::SeqCst);
        let at = figures(&seen, DOWN);
        assert!(
            at.iter().any(|at| *at > 0 && *at < 100),
            "{command}: no figure in between: {:?}",
            seen.lock().unwrap()
        );
        agent.assert_ran_and_is_gone(command);
        assert_eq!(local(&ours), Vec::<String>::new(), "{command}");
        assert!(is_pointer(&ours, "one.bin"), "{command}");
        assert!(is_pointer(&ours, "sub/two.bin"), "{command}");
        assert_no_report_left(&ours.cwd().join(".git"));
    }

    // Left to itself the fetch brings what git-lfs by itself brings.
    agent.waits(0);
    let seen = watch(&mut there.ctx.borrow_mut(), None);
    let fetched = there.run("lfs.fetch", json!({}));
    twin.run(["lfs", "fetch", "origin"]).unwrap();
    assert_eq!(local(&ours), local(&twin));
    assert_eq!(local(&ours).len(), 2);
    assert_eq!(
        figures(&seen, DOWN).last(),
        Some(&100),
        "{:?} {fetched}",
        seen.lock().unwrap()
    );
    assert!(is_pointer(&ours, "one.bin"));

    // What is here is put in place, file by file, as git-lfs does it.
    let seen = watch(&mut there.ctx.borrow_mut(), None);
    let placed = there.run("lfs.checkout", json!({}));
    twin.run(["lfs", "checkout"]).unwrap();
    let said = placed["output"].as_str().unwrap();
    assert!(!said.contains(OUT) || said.contains("100% (2/2)"), "{said}");
    let at = figures(&seen, OUT);
    assert!(at.is_sorted(), "{at:?}");
    assert_eq!(at.last(), Some(&100), "{:?}", seen.lock().unwrap());
    let same = |what: &str| {
        for file in ["one.bin", "sub/two.bin"] {
            assert_eq!(
                std::fs::read(ours.cwd().join(file)).ok(),
                std::fs::read(twin.cwd().join(file)).ok(),
                "{what}: {file}"
            );
        }
        assert_eq!(
            ours.lines(["status", "--porcelain"]).unwrap(),
            twin.lines(["status", "--porcelain"]).unwrap(),
            "{what}"
        );
    };
    same("checked out");
    assert_eq!(content(&ours, "one.bin"), "binary one\n");
    assert_eq!(content(&ours, "sub/two.bin"), "binary two two\n");
    assert_eq!(
        ours.lines(["status", "--porcelain"]).unwrap(),
        Vec::<String>::new()
    );
    // Once more there is nothing to do, and a file that was changed is
    // not put back, while one that is missing is.
    there.run("lfs.checkout", json!({}));
    for git in [&ours, &twin] {
        std::fs::write(git.cwd().join("one.bin"), "changed by hand\n").unwrap();
        std::fs::remove_file(git.cwd().join("sub/two.bin")).unwrap();
    }
    there.run("lfs.checkout", json!({}));
    twin.run(["lfs", "checkout"]).unwrap();
    same("checked out over a change");
    assert_eq!(content(&ours, "one.bin"), "changed by hand\n");
    assert_eq!(content(&ours, "sub/two.bin"), "binary two two\n");
    assert_no_report_left(&ours.cwd().join(".git"));
}

/// Without a transfer program of the tests', between two directories,
/// git-lfs reports no bytes: its meter is what there is, and that is
/// passed on while it runs.
#[test]
fn a_transfer_between_directories_reports_what_the_meter_says() {
    if skipped("a_transfer_between_directories_reports_what_the_meter_says") {
        return;
    }
    let h = Harness::new();
    with_remote(&h.ours);
    let seen = watch(&mut h.ctx.borrow_mut(), None);
    let pushed = h.run("lfs.push", json!({ "all": true }));
    let at = figures(&seen, "Uploading LFS objects");
    assert!(at.is_sorted(), "{at:?}");
    assert_eq!(at.last(), Some(&100), "{:?}", seen.lock().unwrap());
    // Of the line that was rewritten in place, only what it said last is
    // kept, and only when that is the end.
    let said = pushed["output"].as_str().unwrap();
    let meters = said.matches("Uploading LFS objects").count();
    assert!(meters <= 1, "{said}");
    assert!(meters == 0 || said.contains("100% (3/3)"), "{said}");
    assert_no_report_left(&h.ours.path().join(".git"));
}

#[test]
fn hostile_lfs_parameters_are_refused_and_change_nothing() {
    if skipped("hostile_lfs_parameters_are_refused_and_change_nothing") {
        return;
    }
    let h = Harness::new();
    let side = &h.ours;
    side.install();
    side.track("*.bin");
    side.copy.write("one.bin", "binary one\n").unwrap();
    side.commit("an lfs file");
    let bare = side.add_remote();
    let before = side.state();
    let remote_before = files_under(&bare);
    let long = "p".repeat(5000);
    let outside = side.beside("outside");

    for pattern in [
        "",
        " ",
        "\t",
        "-x",
        "--lockable",
        "--not-lockable",
        "--",
        "--filename",
        "!negative",
        "folder/",
        "/",
        "..",
        "../up.bin",
        "a/../../up.bin",
        "a\nb",
        "*.x\n*.y filter=other",
        "nul\u{0}",
        long.as_str(),
    ] {
        for (command, params) in [
            ("lfs.track", json!({ "pattern": pattern })),
            ("lfs.track", json!({ "pattern": pattern, "lockable": true })),
            ("lfs.untrack", json!({ "pattern": pattern })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the pattern {pattern:?}"
            );
        }
    }
    for path in [
        "",
        "-f",
        "--force",
        "--json",
        "/",
        "/abs",
        "..",
        "../up",
        "a/../../up",
        "./one.bin",
        "one.bin/",
        "a//b",
        "a\nb",
        "nul\u{0}",
        ".git",
        ".git/config",
        outside.to_str().unwrap(),
        long.as_str(),
    ] {
        for (command, params) in [
            ("lfs.lock", json!({ "path": path })),
            ("lfs.unlock", json!({ "path": path })),
            ("lfs.unlock", json!({ "path": path, "force": true })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the path {path:?}"
            );
        }
    }
    for remote in [
        "",
        "-x",
        "--all",
        "--upload-pack=touch pwned",
        "a/b",
        "a b",
        "..",
        "a\nb",
        "nul\u{0}",
        "nowhere",
        bare.to_str().unwrap(),
        long.as_str(),
    ] {
        for (command, params) in [
            ("lfs.fetch", json!({ "remote": remote })),
            ("lfs.fetch", json!({ "remote": remote, "all": true })),
            ("lfs.pull", json!({ "remote": remote })),
            ("lfs.push", json!({ "remote": remote })),
            ("lfs.push", json!({ "remote": remote, "all": true })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the remote {remote:?}"
            );
        }
    }
    for rev in [
        "",
        "-x",
        "--all",
        "--json",
        "no-such-rev",
        "HEAD..main",
        "HEAD:one.bin",
        "a\nb",
        "nul\u{0}",
        long.as_str(),
    ] {
        assert!(
            h.try_run("lfs.files", json!({ "rev": rev })).is_err(),
            "lfs.files accepted the revision {rev:?}"
        );
    }
    for (command, params) in [
        ("lfs.track", json!({})),
        ("lfs.track", json!({ "pattern": 3 })),
        ("lfs.track", json!({ "pattern": ["*.x"] })),
        ("lfs.track", json!({ "pattern": "*.x", "lockable": "yes" })),
        ("lfs.untrack", json!({})),
        ("lfs.untrack", json!({ "pattern": { "a": 1 } })),
        ("lfs.files", json!({ "rev": 3 })),
        ("lfs.files", json!({ "rev": ["HEAD"] })),
        ("lfs.fetch", json!({ "remote": 3 })),
        ("lfs.fetch", json!({ "all": "yes" })),
        ("lfs.pull", json!({ "remote": ["origin"] })),
        ("lfs.push", json!({ "remote": true })),
        ("lfs.push", json!({ "all": 1 })),
        ("lfs.lock", json!({})),
        ("lfs.lock", json!({ "path": ["one.bin"] })),
        ("lfs.unlock", json!({})),
        ("lfs.unlock", json!({ "path": "one.bin", "force": "yes" })),
    ] {
        assert!(
            h.try_run(command, params.clone()).is_err(),
            "{command} {params}"
        );
    }
    // Parameters a command does not have are not in its way.
    assert_eq!(
        h.run("lfs.status", json!({ "anything": [1, 2] })),
        h.status()
    );

    assert_eq!(side.state(), before);
    assert_eq!(files_under(&bare), remote_before);
    assert!(!side.path().join("pwned").exists());
    assert!(!outside.exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

/// Every command once, with parameters that are in order. None may panic;
/// each answers or gives an error.
fn every_command(h: &Harness) {
    for (command, params) in [
        ("lfs.status", json!({})),
        ("lfs.track", json!({ "pattern": "*.hostile" })),
        ("lfs.track", json!({ "pattern": "*.bin", "lockable": true })),
        ("lfs.untrack", json!({ "pattern": "*.hostile" })),
        ("lfs.untrack", json!({ "pattern": "*.bin" })),
        ("lfs.files", json!({})),
        ("lfs.files", json!({ "rev": "HEAD~1" })),
        ("lfs.fetch", json!({})),
        ("lfs.fetch", json!({ "all": true })),
        ("lfs.pull", json!({})),
        ("lfs.checkout", json!({})),
        ("lfs.push", json!({})),
        ("lfs.push", json!({ "all": true })),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin", "force": true })),
        ("lfs.install", json!({})),
        ("lfs.status", json!({})),
    ] {
        let _ = h.try_run(command, params);
    }
}

#[test]
fn a_repository_that_is_not_as_expected_gets_an_answer_or_a_refusal() {
    if skipped("a_repository_that_is_not_as_expected_gets_an_answer_or_a_refusal") {
        return;
    }
    let h = Harness::new();
    let side = &h.ours;
    let copy = side.path().to_path_buf();
    side.install();
    side.track("*.bin");
    side.copy.write("one.bin", "binary one\n").unwrap();
    side.copy
        .write("deep/.gitattributes", "*.deep filter=lfs\n")
        .unwrap();
    side.commit("an lfs file");
    side.add_remote();
    side.git().run(["lfs", "push", "--all", "origin"]).unwrap();
    let outside = side.beside("outside");
    std::fs::create_dir_all(&outside).unwrap();
    let precious = outside.join("precious.txt");
    std::fs::write(&precious, "*.outside filter=lfs\n").unwrap();
    let attributes = copy.join(".gitattributes");
    let good = "*.bin filter=lfs diff=lfs merge=lfs -text\n";
    let intact = |what: &str| {
        assert_eq!(
            std::fs::read_to_string(&precious).unwrap(),
            "*.outside filter=lfs\n",
            "{what}"
        );
        assert!(copy.join(".git/HEAD").is_file(), "{what}");
        assert_eq!(
            side.git().rev_parse("HEAD").unwrap(),
            side.git().rev_parse("origin/main").unwrap(),
            "{what}"
        );
    };
    let well_formed = |status: &Value, what: &str| {
        for pattern in status["patterns"].as_array().unwrap() {
            let (text, source) = (
                pattern["pattern"].as_str().unwrap(),
                pattern["source"].as_str().unwrap(),
            );
            assert!(!text.is_empty() && text.len() <= 5000, "{what}: {pattern}");
            assert!(
                !source.contains("..") && !source.starts_with('/'),
                "{what}: {pattern}"
            );
            assert!(!text.contains("outside"), "{what}: {pattern}");
        }
    };

    // `.gitattributes` that is not what it should be.
    let binary: Vec<u8> = (0u8..=255).cycle().take(4096).collect();
    let many = "*.many filter=lfs\n".repeat(10_000);
    let long_line = format!("{} filter=lfs\n", "p".repeat(70_000));
    for content in [
        "".as_bytes(),
        b"\n\n\n",
        b"\"",
        b"\"unclosed filter=lfs\n",
        b"[attr]\n[attr] filter=lfs\n[attr]filter filter=lfs\n",
        b"= filter=lfs\n!x filter=lfs\n- filter=lfs\n",
        b"\x00\x01\x02 filter=lfs\n",
        b"\xff\xfe filter=lfs\n",
        b"../../escape filter=lfs\n/etc/passwd filter=lfs\n",
        binary.as_slice(),
        many.as_bytes(),
        long_line.as_bytes(),
    ] {
        let what = String::from_utf8_lossy(content.get(..30).unwrap_or(content)).into_owned();
        std::fs::write(&attributes, content).unwrap();
        well_formed(&h.status(), &what);
        for (command, params) in [
            ("lfs.track", json!({ "pattern": "*.hostile" })),
            (
                "lfs.track",
                json!({ "pattern": "*.hostile", "lockable": true }),
            ),
            ("lfs.files", json!({})),
            ("lfs.untrack", json!({ "pattern": "*.hostile" })),
            ("lfs.untrack", json!({ "pattern": "*.never" })),
        ] {
            let _ = h.try_run(command, params);
        }
        well_formed(&h.status(), &what);
        intact(&what);
    }
    std::fs::write(&attributes, &binary).unwrap();
    every_command(&h);
    intact("binary");
    std::fs::write(&attributes, many.as_str()).unwrap();
    assert_eq!(h.status()["patterns"].as_array().unwrap().len(), 2000);

    // Too large to be changed from here; still read, up to a bound.
    let huge = "*.huge filter=lfs\n".repeat(1_000_000);
    std::fs::write(&attributes, huge.as_str()).unwrap();
    assert_eq!(h.status()["active"], true);
    h.refused("lfs.track", json!({ "pattern": "*.x" }), "too large");
    h.refused("lfs.untrack", json!({ "pattern": "*.huge" }), "too large");
    assert_eq!(std::fs::metadata(&attributes).unwrap().len(), 18_000_000);
    intact("huge");

    // A directory.
    std::fs::remove_file(&attributes).unwrap();
    std::fs::create_dir_all(attributes.join("inner")).unwrap();
    let status = h.status();
    well_formed(&status, "a directory");
    h.refused("lfs.track", json!({ "pattern": "*.x" }), "is not a file");
    h.refused(
        "lfs.untrack",
        json!({ "pattern": "*.bin" }),
        "is not a file",
    );
    every_command(&h);
    assert!(attributes.join("inner").is_dir());
    intact("a directory");
    std::fs::remove_dir_all(&attributes).unwrap();

    #[cfg(unix)]
    {
        // A link that leads out: not read, and nothing is written through.
        std::os::unix::fs::symlink(&precious, &attributes).unwrap();
        let status = h.status();
        well_formed(&status, "a link");
        h.refused("lfs.track", json!({ "pattern": "*.x" }), "is a link");
        h.refused(
            "lfs.untrack",
            json!({ "pattern": "*.outside" }),
            "is a link",
        );
        intact("a link");
        std::fs::remove_file(&attributes).unwrap();

        // A pipe: nobody writes to it, and nothing here waits for that.
        // Git itself would, as soon as it is asked about a file, so there
        // is no other pattern that would have it asked, and only what
        // reads the file here is run.
        side.git()
            .run(["rm", "-q", "-f", "deep/.gitattributes"])
            .unwrap();
        let made = Command::new("mkfifo")
            .arg(&attributes)
            .status()
            .is_ok_and(|status| status.success());
        if made {
            assert_eq!(h.status()["patterns"], json!([]));
            h.refused("lfs.track", json!({ "pattern": "*.x" }), "is not a file");
            h.refused("lfs.untrack", json!({ "pattern": "*.x" }), "is not a file");
            std::fs::remove_file(&attributes).unwrap();
        }
        side.copy
            .write("deep/.gitattributes", "*.deep filter=lfs\n")
            .unwrap();
        side.git().run(["add", "deep/.gitattributes"]).unwrap();

        // A folder's own file that turned into a link out, or a folder.
        let nested = copy.join("deep/.gitattributes");
        std::fs::remove_file(&nested).unwrap();
        std::os::unix::fs::symlink(&precious, &nested).unwrap();
        well_formed(&h.status(), "a folder's link");
        std::fs::remove_file(&nested).unwrap();
        std::fs::create_dir_all(&nested).unwrap();
        well_formed(&h.status(), "a folder's folder");
        std::fs::remove_dir_all(&nested).unwrap();
        std::fs::write(&nested, "*.deep filter=lfs\n").unwrap();
    }
    std::fs::write(&attributes, good).unwrap();

    // A file that cannot be written: git-lfs says so and ends well, which
    // is not taken for success.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let read_only = |on: bool| {
            let mode = if on { 0o444 } else { 0o644 };
            std::fs::set_permissions(&attributes, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        read_only(true);
        // Whoever may write anything anyway is not stopped by that.
        if std::fs::OpenOptions::new()
            .append(true)
            .open(&attributes)
            .is_err()
        {
            h.refused(
                "lfs.track",
                json!({ "pattern": "*.readonly" }),
                "did not write",
            );
            assert_eq!(std::fs::read_to_string(&attributes).unwrap(), good);
        }
        read_only(false);
    }

    // The store: a file where the directory should be, then garbage in it.
    let lfs = copy.join(".git/lfs");
    std::fs::remove_dir_all(&lfs).unwrap();
    std::fs::write(&lfs, "not a directory\n").unwrap();
    well_formed(&h.status(), "the store is a file");
    assert_eq!(h.status()["files"], 1);
    // Git-lfs fails on that after it has written; the file is put back.
    assert!(h.try_run("lfs.track", json!({ "pattern": "*.x" })).is_err());
    assert_eq!(std::fs::read_to_string(&attributes).unwrap(), good);
    assert!(
        h.try_run("lfs.fetch", json!({ "remote": "origin" }))
            .is_err()
    );
    every_command(&h);
    intact("the store is a file");
    std::fs::remove_file(&lfs).unwrap();
    std::fs::write(&attributes, good).unwrap();
    std::fs::create_dir_all(&lfs).unwrap();
    std::fs::write(lfs.join("objects"), "not a directory either\n").unwrap();
    every_command(&h);
    intact("the objects are a file");
    std::fs::remove_file(lfs.join("objects")).unwrap();
    std::fs::write(&attributes, good).unwrap();
    // Content that is not what its name says.
    let listed = h.run("lfs.files", json!({}));
    let oid = listed["files"][0]["oid"].as_str().unwrap().to_owned();
    let object = lfs.join("objects").join(&oid[..2]).join(&oid[2..4]);
    std::fs::create_dir_all(&object).unwrap();
    std::fs::write(object.join(&oid), "garbage").unwrap();
    every_command(&h);
    intact("an object that is garbage");
    std::fs::write(&attributes, good).unwrap();

    // Hooks: one that is a directory, one that is binary, and no
    // directory for them at all.
    let hooks = copy.join(".git/hooks");
    let _ = std::fs::remove_dir_all(&hooks);
    std::fs::create_dir_all(hooks.join("pre-push")).unwrap();
    assert_eq!(h.status()["hooks"], false);
    h.refused("lfs.install", json!({}), "pre-push is in the way");
    every_command(&h);
    std::fs::remove_dir_all(&hooks).unwrap();
    std::fs::create_dir_all(&hooks).unwrap();
    std::fs::write(hooks.join("pre-push"), &binary).unwrap();
    make_executable(&hooks.join("pre-push"));
    assert_eq!(h.status()["hooks"], false);
    h.refused("lfs.install", json!({}), "pre-push is in the way");
    std::fs::remove_dir_all(&hooks).unwrap();
    std::fs::write(&hooks, "not a directory\n").unwrap();
    assert_eq!(h.status()["hooks"], false);
    assert!(h.try_run("lfs.install", json!({})).is_err());
    every_command(&h);
    intact("hooks");
    std::fs::remove_file(&hooks).unwrap();
    std::fs::write(&attributes, good).unwrap();

    // Settings of LFS in the worktree that are no settings.
    for content in [binary.as_slice(), b"[lfs]\n\turl = \n"] {
        std::fs::write(copy.join(".lfsconfig"), content).unwrap();
        well_formed(&h.status(), "lfsconfig");
        every_command(&h);
        intact("lfsconfig");
        std::fs::write(&attributes, good).unwrap();
    }
    std::fs::remove_file(copy.join(".lfsconfig")).unwrap();

    // A bare repository has no worktree to track in or to lock in.
    let bare = Opened::at(&side.beside("origin.git"));
    let status = bare.run("lfs.status", json!({}));
    assert_eq!(status["patterns"], json!([]));
    assert_eq!(status["files"], 0);
    assert_eq!(status["active"], false);
    for (command, params) in [
        ("lfs.track", json!({ "pattern": "*.x" })),
        ("lfs.untrack", json!({ "pattern": "*.x" })),
        ("lfs.pull", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin" })),
    ] {
        let err = bare.try_run(command, params).unwrap_err();
        assert!(err.contains("bare repository"), "{command}: {err}");
    }
    // It is opened read-only, and git-lfs writes its format
    // version into the configuration of a repository it first runs in:
    // there it is not asked, and the configuration stays as it is.
    let config = side.beside("origin.git").join("config");
    let before = std::fs::read(&config).unwrap();
    let err = bare
        .try_run("lfs.files", json!({ "rev": "main" }))
        .unwrap_err();
    assert!(err.contains("read-only"), "{err}");
    assert_eq!(std::fs::read(&config).unwrap(), before);
    // Where git-lfs has been before, it has nothing left to write.
    Git::new(side.beside("origin.git"), fixtures_out_dir().join(".home"))
        .run(["config", "lfs.repositoryformatversion", "0"])
        .unwrap();
    let settled = std::fs::read(&config).unwrap();
    let listed = bare.run("lfs.files", json!({ "rev": "main" }));
    assert_eq!(listed["files"].as_array().unwrap().len(), 1);
    assert_eq!(listed["files"][0]["present"], true);
    assert_eq!(std::fs::read(&config).unwrap(), settled);
}

/// What the test below asserts, in the copy of this binary that it runs
/// with a `PATH` that has git and no git-lfs.
#[cfg(unix)]
fn without_git_lfs() {
    let h = Harness::new();
    let git = h.ours.git();
    assert!(
        git.run(["lfs", "version"]).is_err(),
        "git-lfs is still found"
    );
    h.ours
        .copy
        .write(
            ".gitattributes",
            "*.bin filter=lfs diff=lfs merge=lfs -text\n*.psd filter=lfs lockable\n",
        )
        .unwrap();
    h.ours.copy.write("one.bin", POINTER).unwrap();
    h.ours.copy.write("small.bin", "no pointer\n").unwrap();
    h.ours.copy.write("notes.txt", POINTER).unwrap();
    h.ours.commit("a pointer written by hand");

    // What can still be told is told.
    assert_eq!(
        h.status(),
        json!({
            "installed": false,
            "version": null,
            "hooks": false,
            "filters": h.ours.filters_found(),
            "patterns": [
                { "pattern": "*.bin", "source": ".gitattributes", "lockable": false },
                { "pattern": "*.psd", "source": ".gitattributes", "lockable": true },
            ],
            "files": 1,
            "active": true,
        })
    );
    // Everything else says what is missing, and does nothing.
    let before = h.ours.state();
    for (command, params) in [
        ("lfs.install", json!({})),
        ("lfs.track", json!({ "pattern": "*.x" })),
        ("lfs.untrack", json!({ "pattern": "*.bin" })),
        ("lfs.files", json!({})),
        ("lfs.files", json!({ "rev": "HEAD" })),
        ("lfs.fetch", json!({})),
        ("lfs.pull", json!({})),
        ("lfs.checkout", json!({})),
        ("lfs.push", json!({})),
        ("lfs.push", json!({ "remote": "origin", "all": true })),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "one.bin" })),
        ("lfs.unlock", json!({ "path": "one.bin" })),
    ] {
        h.refused(command, params, "git-lfs is not installed");
    }
    assert_eq!(h.ours.state(), before);
}

/// Unix only: Git for Windows keeps git-lfs beside git and the libraries
/// git needs, so no `PATH` that lets git start hides it.
#[cfg(unix)]
#[test]
fn without_git_lfs_every_command_says_so_and_the_status_still_answers() {
    if std::env::var_os(WITHOUT).is_some() {
        return without_git_lfs();
    }
    // The test's name as the harness knows it: its path in the program,
    // without the program's own name.
    let module = module_path!()
        .split_once("::")
        .map_or("", |(_, module)| module);
    let name =
        format!("{module}::without_git_lfs_every_command_says_so_and_the_status_still_answers");
    // A test cannot change its own environment, so this one runs itself
    // once more with a `PATH` that holds git and nothing else.
    let git = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join("git"))
            .find(|candidate| candidate.is_file())
    });
    let Some(git) = git else {
        eprintln!("skipped {name}: git is not on the PATH");
        return;
    };
    // The fixture is built before two processes could build it at once.
    drop(Scratch::of("linear").unwrap());
    let dir = fixtures_out_dir()
        .join("scratch")
        .join(format!("without-lfs-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::os::unix::fs::symlink(&git, dir.join("git")).unwrap();
    // Where git-lfs is among git's own programs, the path does not hide it.
    let hidden = !Command::new(dir.join("git"))
        .args(["lfs", "version"])
        .env("PATH", &dir)
        .output()
        .is_ok_and(|out| out.status.success());
    if !hidden {
        eprintln!("skipped {name}: git finds git-lfs without the PATH");
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", &name, "--nocapture", "--test-threads=1"])
        .env("PATH", &dir)
        .env(WITHOUT, "1")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let said = format!(
        "{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    assert!(said.contains("1 passed"), "{said}");
}
