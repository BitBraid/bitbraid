//! Submodule commands against `git submodule` on twin scratch copies: the
//! command runs on one copy, git on the other, and `.gitmodules`, the
//! index, the status, the configuration and the clones git keeps have to
//! come out the same. The submodules are small repositories built beside
//! each copy, the same on both sides.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

/// Git with clones from local paths allowed, as the twin needs it.
const LOCAL: [&str; 2] = ["-c", "protocol.file.allow=always"];

fn git_at(dir: impl Into<PathBuf>) -> Git {
    Git::new(dir, fixtures_out_dir().join(".home"))
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
        side.make_lib();
        side
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    /// A directory beside the copy that is removed with it.
    fn beside(&self, name: &str) -> PathBuf {
        self.copy.path().with_extension(format!("sm-{name}"))
    }
    fn lib(&self) -> String {
        self.beside("lib").to_str().unwrap().to_owned()
    }
    /// The repository the submodules are cloned from: three commits on
    /// `main` and one on `side` that leaves after the first.
    fn make_lib(&self) {
        let dir = self.beside("lib");
        std::fs::create_dir_all(&dir).unwrap();
        let git = git_at(&dir);
        git.run(["init", "-q", "-b", "main"]).unwrap();
        let commit = |file: &str, content: &str, message: &str| {
            std::fs::write(dir.join(file), content).unwrap();
            git.run(["add", file]).unwrap();
            git.run(["commit", "-q", "-m", message]).unwrap();
        };
        commit("lib.txt", "one\n", "lib one");
        commit("lib.txt", "two\n", "lib two");
        git.run(["checkout", "-q", "-b", "side", "HEAD~1"]).unwrap();
        commit("side.txt", "side\n", "lib side");
        git.run(["checkout", "-q", "main"]).unwrap();
        commit("lib.txt", "three\n", "lib three");
    }
    fn lib_rev(&self, rev: &str) -> String {
        git_at(self.beside("lib")).rev_parse(rev).unwrap()
    }
    /// Add the library as a submodule at `path` with git and commit that.
    fn add_with_git(&self, path: &str) {
        let git = self.git();
        let lib = self.lib();
        let mut args = LOCAL.to_vec();
        args.extend(["submodule", "--quiet", "add", "--", lib.as_str(), path]);
        git.run(args).unwrap();
        git.run(["commit", "-q", "-m", &format!("add {path}")])
            .unwrap();
    }
    fn sub(&self, path: &str) -> Git {
        git_at(self.copy.path().join(path))
    }
    fn sweep(&self) {
        let (Some(parent), Some(name)) = (self.copy.path().parent(), self.copy.path().file_name())
        else {
            return;
        };
        let prefix = format!("{}.sm-", name.to_string_lossy());
        for entry in std::fs::read_dir(parent).into_iter().flatten().flatten() {
            if entry.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_dir_all(entry.path());
            }
        }
    }
    /// With the copy's own path taken out: as it is, and as a
    /// configuration file holds it, where a `\` (Windows' separator) is
    /// written twice.
    fn neutral(&self, text: &str) -> String {
        let path = self.copy.path().to_str().unwrap();
        text.replace(&path.replace('\\', "\\\\"), "<copy>")
            .replace(path, "<copy>")
    }
    /// The clones git keeps, by where they are under `.git/modules`.
    fn clones(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            if dir.join("HEAD").is_file() {
                // With `/` between folders, as git names a submodule.
                let relative = dir.strip_prefix(base).unwrap().to_string_lossy();
                out.push(relative.replace(std::path::MAIN_SEPARATOR, "/"));
                return;
            }
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                if entry.path().is_dir() {
                    walk(&entry.path(), base, out);
                }
            }
        }
        let base = self.copy.path().join(".git/modules");
        let mut out = Vec::new();
        walk(&base, &base, &mut out);
        out.sort();
        out
    }
    /// Everything a submodule command can change, as text.
    fn state(&self) -> Vec<String> {
        let git = self.git();
        let mut out = Vec::new();
        let modules = std::fs::read_to_string(self.copy.path().join(".gitmodules"))
            .unwrap_or_else(|_| "(no .gitmodules)".to_owned());
        out.extend(modules.lines().map(|line| self.neutral(line)));
        // `.gitmodules` holds a path of the copy's own, so its id differs;
        // what it says is compared above.
        out.extend(
            git.lines(["ls-files", "-s"])
                .unwrap()
                .into_iter()
                .filter(|line| !line.ends_with("\t.gitmodules")),
        );
        out.extend(git.lines(["status", "--porcelain"]).unwrap());
        out.extend(
            git.lines(["submodule", "status"])
                .unwrap_or_default()
                .into_iter()
                .map(|line| format!("status {line}")),
        );
        let config = git
            .run(["config", "--local", "--get-regexp", r"^submodule\."])
            .unwrap_or_default();
        out.extend(config.lines().map(|line| self.neutral(line)));
        out.extend(
            self.clones()
                .into_iter()
                .map(|clone| format!("clone {clone}")),
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
}

/// The entry of a list for `path`.
fn entry<'a>(list: &'a Value, path: &str) -> &'a Value {
    list.as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == path)
        .unwrap_or_else(|| panic!("{path} is not in {list}"))
}

/// The state `git submodule status` marks a path with.
fn marked_by_git(git: &Git, path: &str) -> &'static str {
    let lines = git.run(["submodule", "status"]).unwrap();
    let line = lines
        .lines()
        .find(|line| line.get(42..).is_some_and(|rest| rest.starts_with(path)))
        .unwrap_or_else(|| panic!("{path} is not in {lines}"));
    match line.chars().next() {
        Some(' ') => "in_step",
        Some('-') => "not_initialised",
        Some('+') => "moved",
        Some('U') => "conflicted",
        other => panic!("unknown mark {other:?}"),
    }
}

#[test]
fn the_list_says_what_git_submodule_status_says() {
    let h = Harness::new();
    let side = &h.ours;
    let git = side.git();
    for path in ["step", "deps/my lib", "moved", "absent", "dirty"] {
        side.add_with_git(path);
    }
    side.sub("moved").run(["checkout", "-q", "HEAD~1"]).unwrap();
    git.run(["submodule", "--quiet", "deinit", "--", "absent"])
        .unwrap();
    std::fs::write(side.copy.path().join("dirty/lib.txt"), "edited\n").unwrap();
    std::fs::write(side.copy.path().join("deps/my lib/new.txt"), "new\n").unwrap();
    // The branch that `.gitmodules` says one follows.
    git.run([
        "config",
        "--file",
        ".gitmodules",
        "submodule.step.branch",
        "side",
    ])
    .unwrap();

    let list = h.run("submodule.list", json!({}));
    assert_eq!(list.as_array().unwrap().len(), 5);
    let side = &h.ours;
    for path in ["step", "deps/my lib", "moved", "absent", "dirty"] {
        let mine = entry(&list, path);
        assert_eq!(mine["name"], path);
        assert_eq!(mine["state"], marked_by_git(&git, path), "{path}");
        assert_eq!(mine["url"], side.lib());
        let recorded = git.run(["ls-files", "-s", "--", path]).unwrap();
        assert_eq!(
            mine["recorded"],
            recorded.split_whitespace().nth(1).unwrap(),
            "{path}"
        );
        let head = side
            .copy
            .path()
            .join(path)
            .join(".git")
            .exists()
            .then(|| side.sub(path).rev_parse("HEAD").unwrap());
        assert_eq!(mine["head"].as_str().map(str::to_owned), head, "{path}");
    }
    assert_eq!(entry(&list, "step")["branch"], "side");
    assert!(entry(&list, "moved")["branch"].is_null());
    assert_eq!(entry(&list, "moved")["head"], side.lib_rev("main~1"));
    assert_eq!(entry(&list, "dirty")["modified"], true);
    assert_eq!(entry(&list, "dirty")["untracked"], false);
    assert_eq!(entry(&list, "deps/my lib")["untracked"], true);
    assert_eq!(entry(&list, "deps/my lib")["modified"], false);
    assert_eq!(entry(&list, "step")["modified"], false);

    // The snapshot the references panel is built from knows the same.
    let refs = h.run("repo.refs", json!({}));
    let mut known: Vec<&str> = refs["submodules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    known.sort();
    assert_eq!(known, ["absent", "deps/my lib", "dirty", "moved", "step"]);
}

#[test]
fn a_submodule_in_a_conflict_is_listed_as_one() {
    let h = Harness::new();
    let side = &h.ours;
    let git = side.git();
    side.add_with_git("sub");
    // One branch records the commit on the library's side branch, the
    // other an older one of its main branch: neither follows the other.
    let record = |branch: &str, rev: &str| {
        git.run(["checkout", "-q", "-b", branch, "main"]).unwrap();
        side.sub("sub").run(["checkout", "-q", rev]).unwrap();
        git.run(["commit", "-q", "-am", &format!("sub to {rev}")])
            .unwrap();
    };
    record("left", "origin/side");
    record("right", "origin/main~1");
    assert!(git.run(["merge", "-q", "left"]).is_err());

    let list = h.run("submodule.list", json!({}));
    let mine = entry(&list, "sub");
    assert_eq!(mine["state"], "conflicted");
    assert_eq!(marked_by_git(&git, "sub"), "conflicted");
    assert!(mine["recorded"].is_null(), "nothing is recorded at stage 0");
    h.refused(
        "submodule.remove",
        json!({ "path": "sub", "force": true }),
        "is in a conflict",
    );
}

#[test]
fn add_matches_git_submodule_add() {
    let h = Harness::new();
    h.assert_same("before");

    let added = h.run(
        "submodule.add",
        json!({ "url": h.ours.lib(), "path": "sub", "allow_local": true }),
    );
    assert_eq!(added["path"], "sub");
    assert_eq!(added["state"], "in_step");
    assert_eq!(added["head"], h.ours.lib_rev("main"));
    let twin = h.twin.git();
    let lib = h.twin.lib();
    let mut args = LOCAL.to_vec();
    args.extend(["submodule", "--quiet", "add", "--", lib.as_str(), "sub"]);
    twin.run(args).unwrap();
    h.assert_same("a submodule at the top");

    // Deeper, with a space in the path, following a branch; the path
    // given with a slash at its end.
    let added = h.run(
        "submodule.add",
        json!({
            "url": h.ours.lib(),
            "path": "deps/my lib/",
            "branch": "side",
            "allow_local": true,
        }),
    );
    assert_eq!(added["path"], "deps/my lib");
    assert_eq!(added["branch"], "side");
    assert_eq!(added["head"], h.ours.lib_rev("side"));
    let mut args = LOCAL.to_vec();
    args.extend([
        "submodule",
        "--quiet",
        "add",
        "--branch",
        "side",
        "--",
        lib.as_str(),
        "deps/my lib",
    ]);
    twin.run(args).unwrap();
    h.assert_same("a submodule in a folder, on a branch");

    // It is staged, not committed, and not an action to undo.
    assert!(
        h.ours
            .git()
            .lines(["status", "--porcelain"])
            .unwrap()
            .contains(&"A  sub".to_owned())
    );
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

/// Whether the configuration this user has lets git clone from local
/// paths anyway. The commands run git as the user has it.
fn local_clones_allowed_anyway() -> bool {
    std::process::Command::new("git")
        .args(["config", "--get", "protocol.file.allow"])
        .output()
        .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "always")
}

#[test]
fn add_refuses_what_cannot_work_and_leaves_nothing_behind() {
    let h = Harness::new();
    h.ours.add_with_git("taken");
    h.ours.copy.write("full/kept.txt", "kept\n").unwrap();
    std::fs::create_dir_all(h.ours.copy.path().join("empty")).unwrap();
    let before = h.ours.state();
    let lib = h.ours.lib();

    for (params, because) in [
        (
            json!({ "url": lib, "path": "taken", "allow_local": true }),
            "is a submodule already",
        ),
        (
            json!({ "url": lib, "path": "full", "allow_local": true }),
            "exists already",
        ),
        // Git takes no directory that is there, an empty one neither.
        (
            json!({ "url": lib, "path": "empty", "allow_local": true }),
            "exists already",
        ),
        // A file of the fixture.
        (
            json!({ "url": lib, "path": "a.txt", "allow_local": true }),
            "exists already",
        ),
        (
            json!({ "url": lib, "path": "new", "branch": "no-such-branch", "allow_local": true }),
            "",
        ),
        (
            json!({ "url": h.ours.beside("nowhere"), "path": "new", "allow_local": true }),
            "",
        ),
    ] {
        h.refused("submodule.add", params, because);
        assert_eq!(h.ours.state(), before);
        assert!(!h.ours.copy.path().join("new").exists());
    }
    // Git does not clone from a local path unless it is told to.
    if !local_clones_allowed_anyway() {
        h.refused("submodule.add", json!({ "url": lib, "path": "new" }), "");
        assert_eq!(h.ours.state(), before);
        assert!(!h.ours.copy.path().join("new").exists());
    }
    assert_eq!(
        std::fs::read_to_string(h.ours.copy.path().join("full/kept.txt")).unwrap(),
        "kept\n"
    );
}

#[test]
fn update_and_sync_match_git() {
    // The superproject records the library at two commits; it is cloned
    // without its submodule, and everything is done in the clones.
    let build = |side: &Side| {
        side.add_with_git("sub");
        side.sub("sub").run(["checkout", "-q", "HEAD~1"]).unwrap();
        side.git()
            .run(["commit", "-q", "-am", "sub back by one"])
            .unwrap();
        let clone = side.beside("clone");
        side.git()
            .run(["clone", "-q", ".", clone.to_str().unwrap()])
            .unwrap();
    };
    let (ours, twin) = (Side::new(), Side::new());
    build(&ours);
    build(&twin);
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run(
            "repo.open",
            &mut ctx,
            &json!({ "path": ours.beside("clone") }),
        )
        .unwrap();
    let mut run = |id: &str, params: Value| {
        registry
            .run(id, &mut ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };
    let (our_clone, twin_clone) = (git_at(ours.beside("clone")), git_at(twin.beside("clone")));
    let state = |side: &Side, git: &Git| -> Vec<String> {
        let mut out = git.lines(["submodule", "status"]).unwrap();
        out.extend(git.lines(["status", "--porcelain"]).unwrap());
        let config = git
            .run(["config", "--local", "--get-regexp", r"^submodule\."])
            .unwrap_or_default();
        out.extend(config.lines().map(|line| side.neutral(line)));
        if git.cwd().join("sub/.git").exists() {
            let inside = git_at(git.cwd().join("sub"));
            out.push(side.neutral(&inside.run(["remote", "get-url", "origin"]).unwrap()));
            out.push(inside.rev_parse("HEAD").unwrap());
        }
        out
    };
    let same = |what: &str| {
        assert_eq!(
            state(&ours, &our_clone),
            state(&twin, &twin_clone),
            "{what}"
        );
    };
    same("cloned without the submodule");
    assert_eq!(
        entry(&run("submodule.list", json!({})), "sub")["state"],
        "not_initialised"
    );

    // Not there yet: update without init leaves it so, with init clones it.
    let after = run("submodule.update", json!({ "allow_local": true }));
    assert_eq!(entry(&after, "sub")["state"], "not_initialised");
    let after = run(
        "submodule.update",
        json!({ "init": true, "allow_local": true }),
    );
    assert_eq!(entry(&after, "sub")["state"], "in_step");
    assert_eq!(entry(&after, "sub")["head"], ours.lib_rev("main~1"));
    let mut args = LOCAL.to_vec();
    args.extend(["submodule", "--quiet", "update", "--init"]);
    twin_clone.run(args).unwrap();
    same("initialised and updated");

    // The superproject goes back to where it recorded the newer commit:
    // the submodule is behind what is recorded until it is updated.
    for git in [&our_clone, &twin_clone] {
        git.run(["checkout", "-q", "HEAD~1"]).unwrap();
    }
    assert_eq!(
        entry(&run("submodule.list", json!({})), "sub")["state"],
        "moved"
    );
    same("moved");
    let after = run(
        "submodule.update",
        json!({ "paths": ["sub"], "recursive": true, "allow_local": true }),
    );
    assert_eq!(entry(&after, "sub")["state"], "in_step");
    assert_eq!(entry(&after, "sub")["head"], ours.lib_rev("main"));
    let mut args = LOCAL.to_vec();
    args.extend(["submodule", "--quiet", "update", "--recursive", "--", "sub"]);
    twin_clone.run(args).unwrap();
    same("updated to what is recorded");

    // The library moves: `.gitmodules` is edited, sync takes that over.
    for (side, git) in [(&ours, &our_clone), (&twin, &twin_clone)] {
        let moved = side.beside("lib-moved");
        std::fs::rename(side.beside("lib"), &moved).unwrap();
        git.run([
            "config",
            "--file",
            ".gitmodules",
            "submodule.sub.url",
            moved.to_str().unwrap(),
        ])
        .unwrap();
    }
    let after = run("submodule.sync", json!({ "paths": ["sub"] }));
    assert_eq!(
        entry(&after, "sub")["url"],
        ours.beside("lib-moved").to_str().unwrap()
    );
    twin_clone
        .run(["submodule", "--quiet", "sync", "--", "sub"])
        .unwrap();
    same("synced");
    assert!(
        state(&ours, &our_clone)
            .iter()
            .any(|line| line.ends_with("sm-lib-moved")),
        "the clone's own remote was repointed"
    );
    run("submodule.sync", json!({ "recursive": true }));
    same("synced again, all of them");
}

#[test]
fn remove_matches_git_and_keeps_work_unless_forced() {
    let h = Harness::new();
    h.both(|side| {
        for path in ["clean", "dirty", "extra", "moved", "purged"] {
            side.add_with_git(path);
        }
        std::fs::write(side.copy.path().join("dirty/lib.txt"), "work\n").unwrap();
        std::fs::write(side.copy.path().join("extra/untracked.txt"), "more\n").unwrap();
        side.sub("moved").run(["checkout", "-q", "HEAD~1"]).unwrap();
    });
    h.assert_same("before");
    let twin = h.twin.git();

    // A clean one goes: staged, the clone kept, as git leaves it.
    let out = h.run("submodule.remove", json!({ "path": "clean" }));
    assert_eq!(
        out,
        json!({ "name": "clean", "path": "clean", "purged": false })
    );
    twin.run(["submodule", "--quiet", "deinit", "--", "clean"])
        .unwrap();
    twin.run(["rm", "-q", "--", "clean"]).unwrap();
    h.assert_same("a clean submodule");
    assert!(h.ours.clones().contains(&"clean".to_owned()));
    assert!(!h.ours.copy.path().join("clean").exists());
    assert!(
        h.ours
            .git()
            .lines(["status", "--porcelain"])
            .unwrap()
            .contains(&"D  clean".to_owned())
    );
    // The clone that was kept is in the way of another one there.
    h.refused(
        "submodule.add",
        json!({ "url": h.ours.lib(), "path": "clean", "allow_local": true }),
        "still keeps a clone",
    );

    // Work in it, or a commit that is not the recorded one, keeps it.
    for (path, because) in [
        ("dirty", "has changes of its own"),
        ("extra", "has changes of its own"),
        ("moved", "is not the one recorded"),
    ] {
        h.refused("submodule.remove", json!({ "path": path }), because);
    }
    h.assert_same("refused");
    assert!(h.ours.copy.path().join("extra/untracked.txt").is_file());
    for path in ["dirty", "extra", "moved"] {
        h.run("submodule.remove", json!({ "path": path, "force": true }));
        twin.run(["submodule", "--quiet", "deinit", "--force", "--", path])
            .unwrap();
        twin.run(["rm", "-q", "--force", "--", path]).unwrap();
    }
    h.assert_same("forced");

    // For good: the clone goes too, and the place is free again.
    let out = h.run(
        "submodule.remove",
        json!({ "path": "purged", "purge": true }),
    );
    assert_eq!(out["purged"], true);
    twin.run(["submodule", "--quiet", "deinit", "--", "purged"])
        .unwrap();
    twin.run(["rm", "-q", "--", "purged"]).unwrap();
    std::fs::remove_dir_all(h.twin.copy.path().join(".git/modules/purged")).unwrap();
    h.assert_same("purged");
    assert!(!h.ours.clones().contains(&"purged".to_owned()));
    h.both(|side| {
        side.git().run(["commit", "-q", "-m", "removed"]).unwrap();
    });
    h.run(
        "submodule.add",
        json!({ "url": h.ours.lib(), "path": "purged", "allow_local": true }),
    );
    let lib = h.twin.lib();
    let mut args = LOCAL.to_vec();
    args.extend(["submodule", "--quiet", "add", "--", lib.as_str(), "purged"]);
    twin.run(args).unwrap();
    h.assert_same("added again where one was purged");
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn forget_deletes_the_clone_a_removal_left_and_nothing_else() {
    let h = Harness::new();
    h.both(|side| {
        for path in ["gone", "deps/deep", "stays"] {
            side.add_with_git(path);
        }
    });
    let twin = h.twin.git();
    // Removed the usual way, on both sides: the clones stay.
    for path in ["gone", "deps/deep"] {
        h.run("submodule.remove", json!({ "path": path }));
        twin.run(["submodule", "--quiet", "deinit", "--", path])
            .unwrap();
        twin.run(["rm", "-q", "--", path]).unwrap();
    }
    h.both(|side| {
        side.git().run(["commit", "-q", "-m", "removed"]).unwrap();
    });
    h.assert_same("removed, the clones kept");
    assert_eq!(h.ours.clones(), ["deps/deep", "gone", "stays"]);
    h.refused(
        "submodule.add",
        json!({ "url": h.ours.lib(), "path": "gone", "allow_local": true }),
        "still keeps a clone",
    );

    // What cannot be forgotten: one that is still here, a folder that
    // holds clones without being one, a name there is no clone of.
    for (name, because) in [
        ("stays", "is a submodule of this repository"),
        ("deps", "keeps no clone"),
        ("never", "keeps no clone"),
        ("gone/objects", "keeps no clone"),
    ] {
        h.refused("submodule.forget", json!({ "name": name }), because);
    }
    h.assert_same("refused");
    assert_eq!(h.ours.clones(), ["deps/deep", "gone", "stays"]);

    // Forgotten: the same as deleting the directory by hand, and the
    // place is free for another submodule.
    let out = h.run("submodule.forget", json!({ "name": "gone" }));
    assert_eq!(out, json!({ "name": "gone", "clone": "modules/gone" }));
    std::fs::remove_dir_all(h.twin.copy.path().join(".git/modules/gone")).unwrap();
    h.assert_same("one clone forgotten");
    assert_eq!(h.ours.clones(), ["deps/deep", "stays"]);
    assert_eq!(h.twin.clones(), ["deps/deep", "stays"]);
    let out = h.run("submodule.forget", json!({ "name": "deps/deep/" }));
    assert_eq!(out["name"], "deps/deep");
    std::fs::remove_dir_all(h.twin.copy.path().join(".git/modules/deps/deep")).unwrap();
    h.assert_same("a clone in a folder forgotten");
    assert_eq!(h.ours.clones(), ["stays"]);
    // The one that is still here works as before.
    assert_eq!(
        h.ours.sub("stays").rev_parse("HEAD").unwrap(),
        h.twin.sub("stays").rev_parse("HEAD").unwrap()
    );
    assert!(
        h.ours
            .git()
            .run(["submodule", "status", "--", "stays"])
            .unwrap()
            .starts_with(' '),
        "in step with what is recorded"
    );
    // Twice is refused: there is nothing left to forget.
    h.refused(
        "submodule.forget",
        json!({ "name": "gone" }),
        "keeps no clone",
    );

    h.run(
        "submodule.add",
        json!({ "url": h.ours.lib(), "path": "gone", "allow_local": true }),
    );
    let lib = h.twin.lib();
    let mut args = LOCAL.to_vec();
    args.extend(["submodule", "--quiet", "add", "--", lib.as_str(), "gone"]);
    twin.run(args).unwrap();
    h.assert_same("added again where one was forgotten");
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn forget_and_purge_do_not_follow_a_link_out_of_the_repository() {
    let h = Harness::new();
    let copy = h.ours.copy.path().to_path_buf();
    // Somebody else's repository, with something that must stay.
    let outside = h.ours.beside("precious");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(outside.join("b")).unwrap();
    for file in ["HEAD", "b/HEAD"] {
        std::fs::write(outside.join(file), "ref: refs/heads/main\n").unwrap();
    }
    std::fs::write(outside.join("b/precious.txt"), "precious\n").unwrap();
    let modules = copy.join(".git/modules");
    std::fs::create_dir_all(&modules).unwrap();
    // `modules/a` leads out of the repository; `a/b` is asked for.
    bitbraid_testkit::symlink_dir(&outside, modules.join("a")).unwrap();
    bitbraid_testkit::symlink_dir(outside.join("b"), modules.join("direct")).unwrap();
    let long = "n".repeat(5000);
    for name in [
        "a/b",
        "a",
        "direct",
        "",
        "..",
        "../..",
        "a/../..",
        "/etc",
        "-rf",
        "a\nb",
        "a\u{0}b",
        ".",
        "a//b",
        long.as_str(),
    ] {
        assert!(
            h.try_run("submodule.forget", json!({ "name": name }))
                .is_err(),
            "submodule.forget accepted {name:?}"
        );
        assert_eq!(
            std::fs::read_to_string(outside.join("b/precious.txt")).unwrap(),
            "precious\n",
            "after {name:?}"
        );
        assert!(copy.join(".git/HEAD").is_file(), "after {name:?}");
    }
    // Parameters of the wrong kind are refused, not unwrapped.
    for params in [json!({}), json!({ "name": 7 }), json!({ "name": ["a"] })] {
        assert!(h.try_run("submodule.forget", params).is_err());
    }

    // A submodule whose kept clone was swapped for such a link: removed
    // for good, the link is not followed.
    h.ours.add_with_git("real");
    let kept = modules.join("real");
    let moved = h.ours.beside("real-clone");
    let _ = std::fs::remove_dir_all(&moved);
    std::fs::rename(&kept, &moved).unwrap();
    std::fs::write(moved.join("precious.txt"), "precious\n").unwrap();
    bitbraid_testkit::symlink_dir(&moved, &kept).unwrap();
    let _ = h.try_run(
        "submodule.remove",
        json!({ "path": "real", "force": true, "purge": true }),
    );
    assert_eq!(
        std::fs::read_to_string(moved.join("precious.txt")).unwrap(),
        "precious\n"
    );
    assert!(moved.join("HEAD").is_file(), "the clone it led to is whole");
    let _ = std::fs::remove_dir_all(&outside);
    let _ = std::fs::remove_dir_all(&moved);
}

/// The subjects `git diff --submodule=log` lists as gained and as lost.
fn logged_by_git(git: &Git, args: &[&str]) -> (Vec<String>, Vec<String>) {
    let mut command = vec!["diff", "--submodule=log"];
    command.extend(args);
    let text = git.run(command).unwrap();
    let pick = |mark: &str| {
        text.lines()
            .filter_map(|line| line.strip_prefix(mark))
            .map(str::to_owned)
            .collect::<Vec<_>>()
    };
    (pick("  > "), pick("  < "))
}

fn subjects(changes: &Value, which: &str) -> Vec<String> {
    changes[which]
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["summary"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn changes_say_what_git_diff_submodule_log_says() {
    let h = Harness::new();
    let side = &h.ours;
    let git = side.git();
    side.add_with_git("sub");
    let at = |rev: &str, message: &str| {
        side.sub("sub").run(["checkout", "-q", rev]).unwrap();
        git.run(["commit", "-q", "-am", message]).unwrap();
    };
    at("origin/main~2", "back by two");
    at("origin/side", "sideways");
    at("origin/main", "forward again");
    let (main, side_tip, first) = (
        side.lib_rev("main"),
        side.lib_rev("side"),
        side.lib_rev("main~2"),
    );

    // Between commits of the superproject: back, sideways, forward.
    for (old, new) in [
        ("HEAD~3", "HEAD~2"),
        ("HEAD~2", "HEAD~1"),
        ("HEAD~1", "HEAD"),
        ("HEAD~3", "HEAD"),
        ("HEAD~2", "HEAD"),
    ] {
        let mine = h.run(
            "submodule.changes",
            json!({ "path": "sub", "old": old, "new": new }),
        );
        let (gained, lost) = logged_by_git(&git, &[old, new, "--", "sub"]);
        assert_eq!(mine["known"], true, "{old}..{new}");
        assert_eq!(subjects(&mine, "gained"), gained, "{old}..{new}");
        assert_eq!(subjects(&mine, "lost"), lost, "{old}..{new}");
        assert_eq!(mine["more_gained"], false);
    }
    let back = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~3", "new": "HEAD~2" }),
    );
    assert_eq!(back["old"], main);
    assert_eq!(back["new"], first);
    assert_eq!(subjects(&back, "lost"), ["lib three", "lib two"]);
    assert!(subjects(&back, "gained").is_empty());
    // What the two recorded commits say, as git logs them.
    let lib = git_at(side.beside("lib"));
    for (key, rev) in [("old_summary", "main"), ("new_summary", "main~2")] {
        let said = lib.run(["log", "-1", "--format=%s", rev]).unwrap();
        assert_eq!(back[key], said.trim(), "{key}");
    }
    let added_too = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~3^", "new": "HEAD~3" }),
    );
    assert!(
        added_too["old_summary"].is_null(),
        "nothing was recorded before"
    );
    assert_eq!(added_too["new_summary"], back["old_summary"]);
    let sideways = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~1", "new": "HEAD" }),
    );
    assert_eq!(sideways["old"], side_tip);
    assert_eq!(subjects(&sideways, "gained"), ["lib three", "lib two"]);
    assert_eq!(subjects(&sideways, "lost"), ["lib side"]);
    assert_eq!(sideways["gained"][0]["id"], main);

    // Where it was added there is no old side, and `^` of a commit names
    // its parent as it does for any diff.
    let added = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~3^", "new": "HEAD~3" }),
    );
    assert!(added["old"].is_null());
    assert_eq!(added["new"], main);
    assert_eq!(added["known"], false);
    assert!(subjects(&added, "gained").is_empty());

    // Uncommitted: checked out elsewhere, then staged.
    side.sub("sub")
        .run(["checkout", "-q", "origin/main~1"])
        .unwrap();
    let unstaged = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "index", "new": "worktree" }),
    );
    let (gained, lost) = logged_by_git(&git, &["--", "sub"]);
    assert_eq!(subjects(&unstaged, "gained"), gained);
    assert_eq!(subjects(&unstaged, "lost"), lost);
    assert_eq!(lost, ["lib three"]);
    // The sides that are meant when none is named.
    assert_eq!(
        h.run("submodule.changes", json!({ "path": "sub" })),
        unstaged
    );
    git.run(["add", "sub"]).unwrap();
    let staged = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD", "new": "index" }),
    );
    let (gained, lost) = logged_by_git(&git, &["--cached", "--", "sub"]);
    assert_eq!(subjects(&staged, "gained"), gained);
    assert_eq!(subjects(&staged, "lost"), lost);
    let none = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "index", "new": "worktree" }),
    );
    assert_eq!(none["old"], none["new"]);
    assert!(subjects(&none, "gained").is_empty() && subjects(&none, "lost").is_empty());

    // Not checked out any more: the clone git keeps still answers.
    git.run(["submodule", "--quiet", "deinit", "--force", "--", "sub"])
        .unwrap();
    let kept = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~1", "new": "HEAD" }),
    );
    assert_eq!(kept["known"], true);
    assert_eq!(subjects(&kept, "lost"), ["lib side"]);
    // Without it, the two ids are all there is.
    std::fs::remove_dir_all(h.ours.copy.path().join(".git/modules/sub")).unwrap();
    let bare = h.run(
        "submodule.changes",
        json!({ "path": "sub", "old": "HEAD~1", "new": "HEAD" }),
    );
    assert_eq!(bare["known"], false);
    assert_eq!(bare["old"], side_tip);
    assert_eq!(bare["new"], main);
    assert!(subjects(&bare, "gained").is_empty());

    // A path that is no submodule records nothing on either side.
    let plain = h.run(
        "submodule.changes",
        json!({ "path": "a.txt", "old": "HEAD~1", "new": "HEAD" }),
    );
    assert!(plain["old"].is_null() && plain["new"].is_null());
    assert_eq!(plain["known"], false);
}

#[test]
fn a_long_way_between_two_commits_is_cut_and_says_so() {
    let h = Harness::new();
    let side = &h.ours;
    side.add_with_git("sub");
    // Far more commits in the library than a list shows.
    let lib = git_at(side.beside("lib"));
    for n in 0..230 {
        lib.run(["commit", "-q", "--allow-empty", "-m", &format!("more {n}")])
            .unwrap();
    }
    let sub = side.sub("sub");
    sub.run(["-c", "protocol.file.allow=always", "fetch", "-q"])
        .unwrap();
    sub.run(["checkout", "-q", "origin/main"]).unwrap();
    let changes = h.run("submodule.changes", json!({ "path": "sub" }));
    assert_eq!(changes["gained"].as_array().unwrap().len(), 200);
    assert_eq!(changes["more_gained"], true);
    assert_eq!(changes["more_lost"], false);
    assert_eq!(changes["gained"][0]["summary"], "more 229");
}

#[test]
fn a_moved_submodule_is_staged_like_a_file_and_not_discarded_like_one() {
    let h = Harness::new();
    h.both(|side| {
        side.add_with_git("sub");
        side.sub("sub").run(["checkout", "-q", "HEAD~1"]).unwrap();
        side.copy.write("a.txt", "edited\n").unwrap();
    });
    let twin = h.twin.git();

    // The status and the diff name it, with the two commits and no hunks.
    let status = h.run("repo.status", json!({}));
    let listed: Vec<&str> = status
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap())
        .collect();
    assert_eq!(listed, ["a.txt", "sub"]);
    let diff = h.run("repo.diff", json!({ "old": "index", "new": "worktree" }));
    let sub = diff
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "sub")
        .expect("the submodule is in the diff");
    assert_eq!(sub["old_mode"], "160000");
    assert_eq!(sub["old_id"], h.ours.lib_rev("main"));
    assert_eq!(sub["new_id"], h.ours.lib_rev("main~1"));
    assert_eq!(sub["hunks"], json!([]));

    h.run("stage.paths", json!({ "paths": ["sub"] }));
    twin.run(["add", "sub"]).unwrap();
    h.assert_same("staged");
    h.run("unstage.paths", json!({ "paths": ["sub"] }));
    twin.run(["restore", "--staged", "sub"]).unwrap();
    h.assert_same("unstaged");

    // By itself it is not discarded: that is what updating it is for.
    h.refused(
        "discard.paths",
        json!({ "paths": ["sub"] }),
        "is a submodule; update it",
    );
    h.refused("discard.paths", json!({ "paths": ["sub/"] }), "");
    h.assert_same("refused");
    // Among files it is left alone, as `git restore` leaves it.
    h.run("discard.paths", json!({ "paths": ["a.txt", "sub"] }));
    twin.run(["restore", "--", "a.txt", "sub"]).unwrap();
    h.assert_same("the file discarded, the submodule left");
    assert_eq!(
        h.ours.sub("sub").rev_parse("HEAD").unwrap(),
        h.ours.lib_rev("main~1")
    );
    h.run("undo.last", json!({}));
    assert_eq!(
        std::fs::read_to_string(h.ours.copy.path().join("a.txt")).unwrap(),
        "edited\n"
    );
    // A folder is not a file either.
    h.ours.copy.write("dir/inner.txt", "x\n").unwrap();
    h.refused(
        "discard.paths",
        json!({ "paths": ["a.txt", "dir"] }),
        "is a folder",
    );
    assert_eq!(
        std::fs::read_to_string(h.ours.copy.path().join("a.txt")).unwrap(),
        "edited\n",
        "nothing is discarded when one path is refused"
    );
}

#[test]
fn hostile_submodule_parameters_are_refused_and_change_nothing() {
    let h = Harness::new();
    h.ours.add_with_git("kept");
    let before = h.ours.state();
    let lib = h.ours.lib();
    let long = "s".repeat(5000);
    let outside = h.ours.beside("outside");

    for path in [
        "",
        "-f",
        "--force",
        "/",
        "/abs",
        "..",
        "../up",
        "a/../../up",
        "./kept",
        "kept/.",
        "kept//",
        "a\nb",
        "nul\u{0}",
        "nowhere",
        ".git",
        "a.txt",
        outside.to_str().unwrap(),
        long.as_str(),
    ] {
        for (command, params) in [
            ("submodule.remove", json!({ "path": path, "force": true })),
            ("submodule.remove", json!({ "path": path, "purge": true })),
            ("submodule.update", json!({ "paths": [path] })),
            ("submodule.update", json!({ "paths": ["kept", path] })),
            ("submodule.sync", json!({ "paths": [path] })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the path {path:?}"
            );
        }
    }
    // Nothing is added outside the repository or where git would read an
    // option.
    for path in [
        "",
        "-f",
        "--force",
        "/abs",
        "..",
        "../up",
        "a/../../up",
        "./here",
        "a//b",
        "a\nb",
        "nul\u{0}",
        outside.to_str().unwrap(),
        long.as_str(),
    ] {
        assert!(
            h.try_run(
                "submodule.add",
                json!({ "url": lib, "path": path, "allow_local": true })
            )
            .is_err(),
            "submodule.add accepted the path {path:?}"
        );
        assert!(
            h.try_run(
                "submodule.changes",
                json!({ "path": path, "old": "HEAD", "new": "index" })
            )
            .is_err(),
            "submodule.changes accepted the path {path:?}"
        );
    }
    for url in [
        "",
        "-u",
        "--upload-pack=touch pwned",
        "a\nb",
        "nul\u{0}",
        long.as_str(),
    ] {
        assert!(
            h.try_run(
                "submodule.add",
                json!({ "url": url, "path": "new", "allow_local": true })
            )
            .is_err(),
            "submodule.add accepted the URL {url:?}"
        );
    }
    for branch in [
        "",
        "-b",
        "--force",
        "a..b",
        "a b",
        "HEAD",
        "@{-1}",
        long.as_str(),
    ] {
        assert!(
            h.try_run(
                "submodule.add",
                json!({ "url": lib, "path": "new", "branch": branch, "allow_local": true })
            )
            .is_err(),
            "submodule.add accepted the branch {branch:?}"
        );
    }
    for side in ["", "-x", "no-such-rev", "a\nb", long.as_str()] {
        for params in [
            json!({ "path": "kept", "old": side, "new": "HEAD" }),
            json!({ "path": "kept", "old": "HEAD", "new": side }),
        ] {
            assert!(
                h.try_run("submodule.changes", params).is_err(),
                "submodule.changes accepted the side {side:?}"
            );
        }
    }
    for (command, params) in [
        ("submodule.add", json!({})),
        ("submodule.add", json!({ "url": lib })),
        ("submodule.add", json!({ "url": 3, "path": "new" })),
        (
            "submodule.add",
            json!({ "url": lib, "path": "new", "allow_local": "yes" }),
        ),
        ("submodule.update", json!({ "paths": "kept" })),
        ("submodule.update", json!({ "init": 1 })),
        ("submodule.sync", json!({ "paths": [3] })),
        ("submodule.remove", json!({})),
        (
            "submodule.remove",
            json!({ "path": "kept", "force": "yes" }),
        ),
        ("submodule.changes", json!({})),
        ("submodule.changes", json!({ "path": ["kept"] })),
    ] {
        assert!(
            h.try_run(command, params.clone()).is_err(),
            "{command} {params}"
        );
    }
    assert_eq!(h.ours.state(), before);
    assert!(!h.ours.copy.path().join("new").exists());
    assert!(!h.ours.copy.path().join("pwned").exists());
    assert!(!outside.exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

#[test]
fn a_gitmodules_file_that_lies_is_not_followed() {
    let h = Harness::new();
    h.ours.add_with_git("real");
    let copy = h.ours.copy.path().to_path_buf();
    let outside = h.ours.beside("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("precious.txt"), "precious\n").unwrap();
    let escape = format!("../{}", outside.file_name().unwrap().to_str().unwrap());

    for content in [
        // Paths that leave the worktree, or are no paths.
        format!("[submodule \"evil\"]\n\tpath = {escape}\n\turl = x\n"),
        format!(
            "[submodule \"evil\"]\n\tpath = {}\n\turl = x\n",
            outside.display()
        ),
        "[submodule \"evil\"]\n\tpath = -rf\n\turl = x\n".to_owned(),
        "[submodule \"evil\"]\n\tpath = a/../../b\n\turl = x\n".to_owned(),
        "[submodule \"evil\"]\n\tpath = \n\turl = x\n".to_owned(),
        // A name that leaves the directory git keeps clones in.
        "[submodule \"../../../evil\"]\n\tpath = real\n\turl = x\n".to_owned(),
        "[submodule \"-x\"]\n\tpath = real\n\turl = x\n".to_owned(),
        // Not a configuration file at all.
        String::new(),
        "\u{0}\u{1}\u{2} binary".to_owned(),
        "[submodule".to_owned(),
        "[submodule \"a\"]\n".repeat(5000),
    ] {
        std::fs::write(copy.join(".gitmodules"), &content).unwrap();
        // An answer or a refusal, never a crash, and nothing listed that
        // is not a plain path inside.
        if let Ok(list) = h.try_run("submodule.list", json!({})) {
            for listed in list.as_array().unwrap() {
                let (path, name) = (
                    listed["path"].as_str().unwrap(),
                    listed["name"].as_str().unwrap(),
                );
                assert!(
                    !path.contains("..") && !path.starts_with(['/', '-']) && !name.contains(".."),
                    "{listed}"
                );
            }
        }
        for (command, params) in [
            (
                "submodule.remove",
                json!({ "path": escape, "force": true, "purge": true }),
            ),
            (
                "submodule.remove",
                json!({ "path": outside, "force": true, "purge": true }),
            ),
            (
                "submodule.remove",
                json!({ "path": "real", "force": true, "purge": true }),
            ),
            ("submodule.update", json!({ "init": true })),
            ("submodule.sync", json!({})),
            ("submodule.changes", json!({ "path": "real" })),
        ] {
            let _ = h.try_run(command, params);
        }
        assert_eq!(
            std::fs::read_to_string(outside.join("precious.txt")).unwrap(),
            "precious\n",
            "{content:?}"
        );
        assert!(copy.join(".git/HEAD").is_file());
    }
    // `.gitmodules` as a directory.
    std::fs::remove_file(copy.join(".gitmodules")).unwrap();
    std::fs::create_dir_all(copy.join(".gitmodules")).unwrap();
    let list = h.run("submodule.list", json!({}));
    assert_eq!(list, json!([]));

    // A bare repository has none to work with.
    let bare = h.ours.beside("bare.git");
    h.ours
        .git()
        .run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": bare }))
        .unwrap();
    assert_eq!(
        registry
            .run("submodule.list", &mut ctx, &json!({}))
            .unwrap(),
        json!([])
    );
    for (command, params) in [
        ("submodule.add", json!({ "url": "x", "path": "new" })),
        ("submodule.update", json!({})),
        ("submodule.sync", json!({})),
        ("submodule.remove", json!({ "path": "real" })),
    ] {
        assert!(
            registry.run(command, &mut ctx, &params).is_err(),
            "{command}"
        );
    }
}
