//! Discarding whole paths against `git restore` on a twin
//! scratch copy: what comes back has to be what a checkout writes, with
//! the filters the attributes name applied, a link as a link and the mode
//! git recorded, and nothing outside the worktree may be touched, also
//! when a link in the worktree leads there. Undo and redo bring back and
//! take away exactly what was there.
//!
//! Writing the bytes of the index entry over whatever is at the path goes
//! wrong in each case tested here: a symbolic link in the worktree would be
//! written through, and the file it leads to overwritten, outside the
//! worktree too; a path behind a linked folder would be removed or written
//! where the link leads; a file in conflict would be taken for an
//! untracked one and deleted; an LFS file would come back as its pointer
//! and a file checked out with CRLF with LF.
//!
//! Sources: git-restore(1), gitattributes(5) (`eol`, `filter`),
//! git-update-index(1) (`--chmod`), git-lfs(1).
#![cfg(unix)]
#![allow(clippy::print_stderr)]

use std::{
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

/// A scratch copy, and a folder beside it that is not in any worktree.
struct Side {
    copy: Scratch,
    outside: PathBuf,
}

impl Side {
    fn new() -> Self {
        let copy = Scratch::of("linear").unwrap();
        let outside = copy.path().with_extension("outside");
        let _ = std::fs::remove_dir_all(&outside);
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("victim.txt"), "untouched\n").unwrap();
        std::fs::write(outside.join("in.txt"), "outside too\n").unwrap();
        Self { copy, outside }
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    fn at(&self, file: &str) -> PathBuf {
        self.copy.path().join(file)
    }
    fn link(&self, target: &Path, file: &str) {
        let at = self.at(file);
        if at.symlink_metadata().is_ok() {
            std::fs::remove_file(&at).unwrap();
        }
        std::os::unix::fs::symlink(target, at).unwrap();
    }
    /// Every file of the worktree: what kind it is and what it holds.
    fn tree(&self) -> Vec<String> {
        fn walk(dir: &Path, base: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap().flatten() {
                let path = entry.path();
                let name = path.strip_prefix(base).unwrap().display().to_string();
                let meta = path.symlink_metadata().unwrap();
                if name == ".git" {
                    continue;
                }
                if meta.file_type().is_symlink() {
                    // Where it leads, without the part that differs
                    // between the two copies.
                    let target = std::fs::read_link(&path).unwrap();
                    let target = target
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    out.push(format!("{name} -> …/{target}"));
                } else if meta.is_dir() {
                    out.push(format!("{name}/"));
                    walk(&path, base, out);
                } else {
                    let runs = meta.permissions().mode() & 0o111 != 0;
                    out.push(format!(
                        "{name}{} {:?}",
                        if runs { " (runs)" } else { "" },
                        String::from_utf8_lossy(&std::fs::read(&path).unwrap())
                    ));
                }
            }
        }
        let mut out = Vec::new();
        walk(self.copy.path(), self.copy.path(), &mut out);
        out.sort();
        out
    }
    /// What is outside, which nothing may change.
    fn untouched(&self, what: &str) {
        for (file, text) in [("victim.txt", "untouched\n"), ("in.txt", "outside too\n")] {
            assert_eq!(
                std::fs::read_to_string(self.outside.join(file)).unwrap(),
                text,
                "{what}: {file} outside the worktree"
            );
        }
        let mut names: Vec<String> = std::fs::read_dir(&self.outside)
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(names, ["in.txt", "victim.txt"], "{what}");
    }
    fn porcelain(&self) -> Vec<String> {
        self.git().lines(["status", "--porcelain"]).unwrap()
    }
}

impl Drop for Side {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.outside);
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
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn refused(&mut self, id: &str, params: Value, why: &str) {
        let err = self
            .registry
            .run(id, &mut self.ctx, &params)
            .expect_err(&format!("{id} {params} should be refused"))
            .to_string();
        assert!(err.contains(why), "{id} {params}: {err}");
    }
    fn same(&self, what: &str) {
        assert_eq!(self.ours.tree(), self.twin.tree(), "{what}");
        assert_eq!(self.ours.porcelain(), self.twin.porcelain(), "{what}");
        self.ours.untouched(what);
        self.twin.untouched(what);
    }
}

/// Files of every kind a checkout writes differently, committed.
fn committed(side: &Side) {
    let copy = &side.copy;
    copy.write(".gitattributes", "*.crlf text eol=crlf\n")
        .unwrap();
    copy.write("text.crlf", "one\r\ntwo\r\n").unwrap();
    copy.write("run.sh", "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(side.at("run.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    copy.write("plain.txt", "plain\n").unwrap();
    copy.write("gone.txt", "to be deleted\n").unwrap();
    copy.write("dir/in.txt", "inside\n").unwrap();
    side.link(Path::new("plain.txt"), "link");
    side.git().run(["add", "-A"]).unwrap();
    side.git()
        .run(["commit", "-q", "-m", "files of every kind"])
        .unwrap();
}

/// Each of them changed in the way its kind can be, and two files that
/// are new. Links lead out of the worktree wherever one can be made.
fn changed(side: &Side) {
    let victim = side.outside.join("victim.txt");
    side.copy.write("text.crlf", "changed\r\n").unwrap();
    side.copy.write("run.sh", "#!/bin/sh\nexit 1\n").unwrap();
    std::fs::set_permissions(side.at("run.sh"), std::fs::Permissions::from_mode(0o644)).unwrap();
    // A file replaced by a link, and a link that leads elsewhere now.
    side.link(&victim, "plain.txt");
    side.link(&victim, "link");
    side.copy.remove("gone.txt").unwrap();
    side.copy.write("dir/in.txt", "inside, changed\n").unwrap();
    side.copy.write("new.txt", "not known to git\n").unwrap();
    side.link(&victim, "newlink");
}

const TRACKED: [&str; 6] = [
    "text.crlf",
    "run.sh",
    "plain.txt",
    "link",
    "gone.txt",
    "dir/in.txt",
];
const UNTRACKED: [&str; 2] = ["new.txt", "newlink"];

#[test]
fn discard_puts_back_what_git_restore_puts_back_and_undo_takes_it_away() {
    let mut h = Harness::new();
    h.both(committed);
    h.same("committed");
    let clean = h.ours.tree();
    h.both(changed);
    h.same("changed");
    let dirty = h.ours.tree();
    assert_ne!(dirty, clean);

    // Every file at once on one copy; git on the other.
    let all: Vec<&str> = TRACKED.iter().chain(&UNTRACKED).copied().collect();
    let out = h.run("discard.paths", json!({ "paths": all }));
    assert_eq!(out["discarded"], json!(all));
    let mut args = vec!["restore", "--worktree", "--"];
    args.extend(TRACKED);
    h.twin.git().run(args).unwrap();
    for file in UNTRACKED {
        std::fs::remove_file(h.twin.at(file)).unwrap();
    }
    h.same("discarded");
    assert_eq!(h.ours.tree(), clean);
    assert_eq!(h.ours.porcelain(), Vec::<String>::new());
    // Said once more, each for itself: the line endings of a checkout,
    // the mode, a file where a link had taken its place, the link as it
    // was recorded, and the link that was new gone without its target.
    let read = |file: &str| std::fs::read_to_string(h.ours.at(file)).unwrap();
    assert_eq!(read("text.crlf"), "one\r\ntwo\r\n");
    assert_eq!(read("run.sh"), "#!/bin/sh\nexit 0\n");
    let mode = |file: &str| h.ours.at(file).metadata().unwrap().permissions().mode();
    assert_ne!(mode("run.sh") & 0o111, 0);
    assert!(h.ours.at("plain.txt").symlink_metadata().unwrap().is_file());
    assert_eq!(read("plain.txt"), "plain\n");
    assert_eq!(
        std::fs::read_link(h.ours.at("link")).unwrap(),
        Path::new("plain.txt")
    );
    assert!(h.ours.at("newlink").symlink_metadata().is_err());
    assert_eq!(read("gone.txt"), "to be deleted\n");

    // Undo brings every change back as it was, links as links.
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Discard 8 files");
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.tree(), dirty);
    h.ours.untouched("undone");
    assert_eq!(
        std::fs::read_link(h.ours.at("plain.txt")).unwrap(),
        h.ours.outside.join("victim.txt")
    );
    // And redo takes them away again.
    h.run("redo.last", json!({}));
    assert_eq!(h.ours.tree(), clean);
    h.ours.untouched("redone");
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.tree(), dirty);

    // One at a time comes to the same.
    for file in all {
        h.run("discard.paths", json!({ "paths": [file] }));
        h.ours.untouched(file);
    }
    assert_eq!(h.ours.tree(), clean);
    h.same("discarded one by one");
}

#[test]
fn nothing_is_discarded_behind_a_link_through_one_or_in_conflict() {
    let mut h = Harness::new();
    committed(&h.ours);
    let side_outside = h.ours.outside.clone();

    // A file named in a roundabout way is not the file git lists: it is
    // refused, not taken for one git does not know and removed.
    let clean = h.ours.tree();
    for path in [
        "./plain.txt",
        "dir//in.txt",
        "dir/./in.txt",
        "plain.txt/",
        ".",
        "dir/",
        "dir/../plain.txt",
        "/plain.txt",
        "",
    ] {
        h.refused("discard.paths", json!({ "paths": [path] }), "path");
        h.refused(
            "discard.paths",
            json!({ "paths": ["gone.txt", path] }),
            "path",
        );
        h.refused(
            "discard.hunks",
            json!({ "path": path, "selection": [{ "hunk": 0 }] }),
            "path",
        );
    }
    assert_eq!(h.ours.tree(), clean);
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // What is inside a submodule is that repository's to discard: it is
    // refused, not taken for a file git does not know and removed.
    let git = h.ours.git();
    let head = git.rev_parse("HEAD").unwrap();
    git.run([
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("160000,{head},sub"),
    ])
    .unwrap();
    h.ours
        .copy
        .write("sub/deeper/kept.txt", "the submodule's own\n")
        .unwrap();
    for paths in [
        json!(["sub/deeper/kept.txt"]),
        json!(["gone.txt", "sub/deeper/kept.txt"]),
        json!(["sub/nothing.txt"]),
    ] {
        h.refused(
            "discard.paths",
            json!({ "paths": paths }),
            "is inside the submodule sub",
        );
    }
    assert_eq!(
        std::fs::read_to_string(h.ours.at("sub/deeper/kept.txt")).unwrap(),
        "the submodule's own\n"
    );
    git.run(["update-index", "--force-remove", "--", "sub"])
        .unwrap();
    std::fs::remove_dir_all(h.ours.at("sub")).unwrap();
    assert_eq!(h.ours.tree(), clean);

    // A link to a folder outside, which git knows nothing of: what is
    // behind it is no file of the worktree, there or not.
    h.ours.link(&side_outside, "way");
    for path in ["way/victim.txt", "way/in.txt", "way/nothing.txt"] {
        h.refused("discard.paths", json!({ "paths": [path] }), "is behind way");
        h.ours.untouched(path);
    }
    // Among files that could be discarded, it is in their way: nothing
    // is discarded then.
    h.ours.copy.write("plain.txt", "changed\n").unwrap();
    h.refused(
        "discard.paths",
        json!({ "paths": ["plain.txt", "way/victim.txt"] }),
        "is behind way",
    );
    assert_eq!(
        std::fs::read_to_string(h.ours.at("plain.txt")).unwrap(),
        "changed\n"
    );
    // The link itself is a file like any other that is new: it goes, and
    // what it led to stays.
    h.run("discard.paths", json!({ "paths": ["way", "plain.txt"] }));
    assert!(h.ours.at("way").symlink_metadata().is_err());
    h.ours.untouched("the link discarded");
    h.run("undo.last", json!({}));
    assert_eq!(std::fs::read_link(h.ours.at("way")).unwrap(), side_outside);
    h.ours.untouched("the link brought back");
    std::fs::remove_file(h.ours.at("way")).unwrap();
    h.run("discard.paths", json!({ "paths": ["plain.txt"] }));

    // A folder git knows, replaced by a link that leads outside: its
    // file is behind the link now.
    std::fs::remove_dir_all(h.ours.at("dir")).unwrap();
    h.ours.link(&side_outside, "dir");
    h.refused(
        "discard.paths",
        json!({ "paths": ["dir/in.txt"] }),
        "is behind dir",
    );
    h.refused(
        "discard.hunks",
        json!({ "path": "dir/in.txt", "selection": [{ "hunk": 0 }] }),
        "is behind dir",
    );
    h.ours.untouched("behind a folder that became a link");
    // With the link out of the way the file comes back.
    h.run("discard.paths", json!({ "paths": ["dir"] }));
    h.run("discard.paths", json!({ "paths": ["dir/in.txt"] }));
    assert_eq!(
        std::fs::read_to_string(h.ours.at("dir/in.txt")).unwrap(),
        "inside\n"
    );
    h.ours.untouched("the folder back");

    // Lines are not discarded through a link either.
    let victim = side_outside.join("victim.txt");
    h.ours.link(&victim, "plain.txt");
    h.refused(
        "discard.hunks",
        json!({ "path": "plain.txt", "selection": [{ "hunk": 0 }] }),
        "is a symbolic link",
    );
    h.ours.untouched("lines through a link");
    h.run("discard.paths", json!({ "paths": ["plain.txt"] }));
    // A folder is refused, and so is a link's target folder taken for
    // one: the link is a file.
    h.refused("discard.paths", json!({ "paths": ["dir"] }), "is a folder");

    // A file in conflict has no one version to go back to: it stays, with
    // its markers, and is not taken for a file git does not know and
    // deleted.
    let git = h.ours.git();
    git.run(["checkout", "-q", "-b", "other"]).unwrap();
    h.ours.copy.write("a.txt", "other side\n").unwrap();
    git.run(["commit", "-q", "-am", "other side"]).unwrap();
    git.run(["checkout", "-q", "main"]).unwrap();
    h.ours.copy.write("a.txt", "main side\n").unwrap();
    git.run(["commit", "-q", "-am", "main side"]).unwrap();
    assert!(git.run(["merge", "other"]).is_err());
    let marked = std::fs::read_to_string(h.ours.at("a.txt")).unwrap();
    assert!(marked.contains("<<<<<<<"), "{marked}");
    let before = h.run("undo.peek", json!({}));
    h.refused(
        "discard.paths",
        json!({ "paths": ["a.txt"] }),
        "a.txt is in conflict",
    );
    assert_eq!(std::fs::read_to_string(h.ours.at("a.txt")).unwrap(), marked);
    assert_eq!(h.run("undo.peek", json!({})), before);
    h.ours.untouched("in conflict");
}

/// A discard of lines is journalled as such: as a discard of its path,
/// redoing it would discard every change the file has.
#[test]
fn redoing_a_discard_of_lines_discards_those_lines_only() {
    let mut h = Harness::new();
    let read = |h: &Harness| std::fs::read_to_string(h.ours.at("a.txt")).unwrap();
    let committed = read(&h);
    // A line before what is there and a line after it.
    let changed = format!("0\n{committed}3\n");
    h.ours.copy.write("a.txt", &changed).unwrap();
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["a.txt"] }),
    );
    let lines = diff[0]["hunks"][0]["lines"].as_array().unwrap();
    let zero = lines
        .iter()
        .position(|line| line["kind"] == "added" && line["text"] == "0")
        .unwrap();
    h.run(
        "discard.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": [zero] }] }),
    );
    let without = format!("{committed}3\n");
    assert_eq!(read(&h), without);
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Discard lines in a.txt"
    );

    h.run("undo.last", json!({}));
    assert_eq!(read(&h), changed);
    // The one line goes again, and the other change stays.
    h.run("redo.last", json!({}));
    assert_eq!(read(&h), without);
    h.run("undo.last", json!({}));
    assert_eq!(read(&h), changed);
    h.run("redo.last", json!({}));

    // What was written to the file since is not overwritten by an undo.
    let later = format!("{without}4\n");
    h.ours.copy.write("a.txt", &later).unwrap();
    h.refused("undo.last", json!({}), "changed since");
    assert_eq!(read(&h), later);
}

/// An LFS file comes back as its content, where the bytes of the index
/// are its pointer. Skipped where git-lfs is not installed.
#[test]
fn a_discarded_lfs_file_comes_back_as_its_content() {
    let found = Command::new("git")
        .args(["lfs", "version"])
        .output()
        .is_ok_and(|out| out.status.success());
    if !found {
        eprintln!(
            "skipped a_discarded_lfs_file_comes_back_as_its_content: git-lfs is not installed"
        );
        return;
    }
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        // Whoever runs the tests may keep hooks in a place of their own,
        // and git-lfs writes its hooks where git looks for them.
        let theirs = Command::new("git")
            .current_dir(side.copy.path())
            .args(["config", "--get", "core.hooksPath"])
            .output()
            .is_ok_and(|out| out.status.success() && !out.stdout.is_empty());
        if theirs {
            let hooks = side.at(".git/hooks");
            git.run(["config", "core.hooksPath", hooks.to_str().unwrap()])
                .unwrap();
        }
        git.run(["lfs", "install", "--local"]).unwrap();
        git.run(["lfs", "track", "*.bin"]).unwrap();
        side.copy.write("large.bin", "the content\n").unwrap();
        git.run(["add", "-A"]).unwrap();
        git.run(["commit", "-q", "-m", "an lfs file"]).unwrap();
        side.copy.write("large.bin", "changed\n").unwrap();
    });
    // What the index has for it is a pointer.
    let pointer = h
        .ours
        .git()
        .run(["cat-file", "blob", ":large.bin"])
        .unwrap();
    assert!(pointer.starts_with("version https://git-lfs"), "{pointer}");

    h.run("discard.paths", json!({ "paths": ["large.bin"] }));
    h.twin
        .git()
        .run(["restore", "--worktree", "--", "large.bin"])
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(h.ours.at("large.bin")).unwrap(),
        "the content\n"
    );
    h.same("an lfs file discarded");
    h.run("undo.last", json!({}));
    assert_eq!(
        std::fs::read_to_string(h.ours.at("large.bin")).unwrap(),
        "changed\n"
    );
}
