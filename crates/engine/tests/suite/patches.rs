//! Patch commands against git on twin scratch copies: what is exported is
//! byte for byte what `git format-patch` and `git diff` write, what is
//! applied leaves the files, the index and the history as `git apply` and
//! `git am` leave them, and a mailbox that stops is continued, skipped and
//! aborted as git does it.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

/// A scratch copy set up the way every copy in these tests is: commands
/// run git with the real environment, so what a commit and a merge need is
/// in the copy's own configuration, and rerere stays out of it.
fn copy() -> Scratch {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    for (key, value) in [
        ("user.name", "Ada Fixture"),
        ("user.email", "ada@fixture.invalid"),
        ("commit.gpgsign", "false"),
        ("merge.conflictStyle", "merge"),
        ("rerere.enabled", "false"),
        ("am.threeWay", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    scratch
}

/// A copy with the commands run on it.
struct Side {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Side {
    fn new() -> Self {
        let scratch = copy();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.scratch.git()
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
    fn file(&self, rel: &str) -> PathBuf {
        self.scratch.path().join(rel)
    }
    fn note(&self) -> PathBuf {
        self.file(".git/bitbraid/operation.json")
    }
}

/// A directory beside a scratch copy for patches to go to, removed with
/// the test.
struct Outbox(PathBuf);

impl Outbox {
    fn new(of: &Scratch) -> Self {
        let path = of.path().with_extension("patches");
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn at(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn text(&self, name: &str) -> String {
        self.at(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Outbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The files of a directory by name, with what is in them.
fn files_in(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files: Vec<(String, Vec<u8>)> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| {
            (
                entry.file_name().to_string_lossy().into_owned(),
                std::fs::read(entry.path()).unwrap(),
            )
        })
        .collect();
    files.sort();
    files
}

fn names_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn status(git: &Git) -> String {
    git.run(["status", "--porcelain"]).unwrap()
}

fn index(git: &Git) -> String {
    git.run(["ls-files", "--stage"]).unwrap()
}

/// The history as (tree, number of parents, author, author date, subject):
/// commit ids carry the committer's date, these do not.
fn shape(git: &Git) -> Vec<String> {
    git.lines(["log", "--format=%T %P|%an <%ae> %at|%s", "HEAD"])
        .unwrap()
        .into_iter()
        .map(|line| {
            let (ids, rest) = line.split_once('|').unwrap();
            let mut ids = ids.split(' ');
            let tree = ids.next().unwrap().to_owned();
            format!("{tree} {} {rest}", ids.count())
        })
        .collect()
}

fn assert_same(ours: &Git, twin: &Git, what: &str) {
    assert_eq!(shape(ours), shape(twin), "{what}: history");
    assert_eq!(status(ours), status(twin), "{what}: status");
    assert_eq!(index(ours), index(twin), "{what}: index");
}

/// Uncommitted work of every kind a patch has to carry: a changed file, a
/// new one whose text is not UTF-8, one that is not text at all and one
/// that is gone, all staged; and a change that is not staged.
fn work(copy: &Scratch) {
    let git = copy.git();
    copy.write("a.txt", "changed and staged\nwith a second line\n")
        .unwrap();
    std::fs::write(copy.path().join("latin.txt"), b"caf\xe9 au lait\n").unwrap();
    std::fs::write(copy.path().join("bin.dat"), [0u8, 159, 146, 150, 0, 1, 2]).unwrap();
    git.run(["rm", "-q", "README.md"]).unwrap();
    git.run(["add", "a.txt", "latin.txt", "bin.dat"]).unwrap();
    copy.write("b.txt", "changed, not staged\n").unwrap();
}

/// No directory of ours is left where patches were written.
fn nothing_left_behind(dir: &Path) {
    for entry in std::fs::read_dir(dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        assert!(!name.starts_with(".bitbraid"), "{name} was left in {dir:?}");
        if entry.path().is_dir() {
            nothing_left_behind(&entry.path());
        }
    }
}

#[test]
fn exported_patches_are_byte_for_byte_what_git_writes() {
    let mut h = Side::new();
    let twin = copy();
    let ours = Outbox::new(&h.scratch);
    let theirs = Outbox::new(&twin);
    let twin_git = twin.git();

    // A range into a directory that is made for it: one file per commit.
    let out = h.run(
        "patch.export",
        json!({ "from": "HEAD~3", "dir": ours.text("range") }),
    );
    twin_git
        .run([
            "format-patch",
            "-q",
            "-o",
            theirs.text("range").as_str(),
            "HEAD~3..HEAD",
        ])
        .unwrap();
    assert_eq!(
        files_in(&ours.at("range")),
        files_in(&theirs.at("range")),
        "a range"
    );
    let names = [
        "0001-update-a.patch",
        "0002-add-b.patch",
        "0003-update-b.patch",
    ];
    assert_eq!(names_in(&ours.at("range")), names);
    let written: Vec<String> = names
        .iter()
        .map(|name| ours.at("range").join(name).to_str().unwrap().to_owned())
        .collect();
    assert_eq!(out["files"], json!(written), "in order");
    assert_eq!(out["patches"], 3);
    assert_eq!(out["skipped"], json!([]));

    // Commits one by one, in the order they are named.
    let newest = h.git().rev_parse("HEAD").unwrap();
    let older = h.git().rev_parse("HEAD~3").unwrap();
    let out = h.run(
        "patch.export",
        json!({ "commits": [newest, "HEAD~3"], "dir": ours.text("each") }),
    );
    for (number, id) in [(1, &newest), (2, &older)] {
        twin_git
            .run([
                "format-patch",
                "-q",
                "-o",
                theirs.text("each").as_str(),
                format!("--start-number={number}").as_str(),
                "-1",
                id.as_str(),
            ])
            .unwrap();
    }
    assert_eq!(
        files_in(&ours.at("each")),
        files_in(&theirs.at("each")),
        "single commits"
    );
    assert_eq!(
        names_in(&ours.at("each")),
        ["0001-update-b.patch", "0002-add-a.patch"]
    );
    assert_eq!(out["patches"], 2);

    // A mailbox of a range, and of single commits.
    let out = h.run(
        "patch.export",
        json!({ "from": "HEAD~3", "to": "HEAD~1", "file": ours.text("range.mbox") }),
    );
    assert_eq!(out["files"], json!([ours.text("range.mbox")]));
    assert_eq!(out["patches"], 2);
    assert_eq!(
        std::fs::read(ours.at("range.mbox")).unwrap(),
        twin_git
            .run(["format-patch", "--stdout", "HEAD~3..HEAD~1"])
            .unwrap()
            .into_bytes(),
        "a mailbox"
    );
    h.run(
        "patch.export",
        json!({ "commits": [newest, older], "file": ours.text("each.mbox") }),
    );
    let mut expected = String::new();
    for id in [&newest, &older] {
        expected.push_str(
            &twin_git
                .run(["format-patch", "--stdout", "-1", id.as_str()])
                .unwrap(),
        );
    }
    assert_eq!(
        std::fs::read(ours.at("each.mbox")).unwrap(),
        expected.into_bytes(),
        "a mailbox of single commits"
    );
    // The first commit of all has a patch too.
    let out = h.run(
        "patch.export",
        json!({ "commits": ["HEAD~4"], "file": ours.text("root.mbox") }),
    );
    assert_eq!(out["patches"], 1);
    assert_eq!(
        std::fs::read(ours.at("root.mbox")).unwrap(),
        twin_git
            .run(["format-patch", "--stdout", "-1", "HEAD~4"])
            .unwrap()
            .into_bytes()
    );

    // Uncommitted work, with what is not text and what is not UTF-8.
    work(&h.scratch);
    work(&twin);
    for (source, flag, name) in [
        ("staged", Some("--cached"), "staged.diff"),
        ("worktree", None, "worktree.diff"),
    ] {
        let out = h.run(
            "patch.export",
            json!({ source: true, "file": ours.text(name) }),
        );
        assert_eq!(out["files"], json!([ours.text(name)]));
        let mut args = vec!["diff".to_owned(), "--binary".to_owned()];
        args.extend(flag.map(str::to_owned));
        args.push(format!("--output={}", theirs.text(name)));
        twin_git.run(&args).unwrap();
        let written = std::fs::read(ours.at(name)).unwrap();
        assert_eq!(written, std::fs::read(theirs.at(name)).unwrap(), "{source}");
        assert!(!written.is_empty());
    }
    let staged = std::fs::read(ours.at("staged.diff")).unwrap();
    assert!(staged.windows(4).any(|bytes| bytes == b"caf\xe9"));
    assert!(String::from_utf8_lossy(&staged).contains("GIT binary patch"));

    // A file that is there is not written over unless that is asked for.
    h.refused(
        "patch.export",
        json!({ "worktree": true, "file": ours.text("staged.diff") }),
        "exists already",
    );
    assert_eq!(std::fs::read(ours.at("staged.diff")).unwrap(), staged);
    // In a directory it is all of them or none: the second of these two
    // is there already, and the first is not written either.
    h.refused(
        "patch.export",
        json!({ "commits": ["HEAD~4", "HEAD~1"], "dir": ours.text("range") }),
        "0002-add-b.patch exists already",
    );
    assert_eq!(files_in(&ours.at("range")), files_in(&theirs.at("range")));
    h.run(
        "patch.export",
        json!({ "worktree": true, "file": ours.text("staged.diff"), "overwrite": true }),
    );
    assert_eq!(
        std::fs::read(ours.at("staged.diff")).unwrap(),
        std::fs::read(theirs.at("worktree.diff")).unwrap()
    );
    h.run(
        "patch.export",
        json!({ "from": "HEAD~3", "dir": ours.text("range"), "overwrite": true }),
    );
    assert_eq!(files_in(&ours.at("range")), files_in(&theirs.at("range")));

    // Exporting changes nothing in the repository and leaves nothing of
    // its own beside the patches.
    assert_eq!(status(&h.git()), status(&twin_git));
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    nothing_left_behind(&ours.0);
}

#[test]
fn a_merge_commit_is_left_out_and_said_to_be() {
    let mut h = Side::new();
    let twin = copy();
    let ours = Outbox::new(&h.scratch);
    let theirs = Outbox::new(&twin);
    for copy in [&h.scratch, &twin] {
        let git = copy.git();
        git.run(["switch", "-q", "-c", "side", "HEAD~2"]).unwrap();
        copy.write("side.txt", "on the side\n").unwrap();
        git.run(["add", "side.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "side work"]).unwrap();
        git.run(["switch", "-q", "main"]).unwrap();
        git.run(["merge", "-q", "--no-ff", "-m", "merge side", "side"])
            .unwrap();
    }
    let merge = h.git().rev_parse("HEAD").unwrap();
    let out = h.run(
        "patch.export",
        json!({ "from": "HEAD~1", "dir": ours.text("range") }),
    );
    twin.git()
        .run([
            "format-patch",
            "-q",
            "-o",
            theirs.text("range").as_str(),
            "HEAD~1..HEAD",
        ])
        .unwrap();
    assert_eq!(files_in(&ours.at("range")), files_in(&theirs.at("range")));
    assert_eq!(names_in(&ours.at("range")), ["0001-side-work.patch"]);
    assert_eq!(out["skipped"], json!([merge]));

    // Named by itself it has nothing to write, alone or among others.
    h.refused(
        "patch.export",
        json!({ "commits": [merge], "file": ours.text("merge.mbox") }),
        "merge commit",
    );
    h.refused(
        "patch.export",
        json!({ "commits": [merge], "dir": ours.text("merge") }),
        "merge commit",
    );
    assert!(!ours.at("merge.mbox").exists());
    assert!(
        !ours.at("merge").exists(),
        "the directory made for it is gone"
    );
    let out = h.run(
        "patch.export",
        json!({ "commits": [merge, "side"], "dir": ours.text("some") }),
    );
    assert_eq!(names_in(&ours.at("some")), ["0001-side-work.patch"]);
    assert_eq!(out["skipped"], json!([merge]));
    assert_eq!(out["patches"], 1);
    nothing_left_behind(&ours.0);
}

#[test]
fn what_is_exported_applies_on_a_fresh_copy_to_the_same_tree() {
    let mut source = Side::new();
    let mut fresh = Side::new();
    let outbox = Outbox::new(&source.scratch);

    // Commits: a mailbox from one copy, applied where they are missing.
    source.scratch.write("c.txt", "a third file\n").unwrap();
    source.git().run(["add", "c.txt"]).unwrap();
    source.git().run(["commit", "-q", "-m", "add c"]).unwrap();
    source.run(
        "patch.export",
        json!({ "from": "HEAD~3", "file": outbox.text("three.mbox") }),
    );
    fresh
        .git()
        .run(["reset", "-q", "--hard", "HEAD~2"])
        .unwrap();
    let out = fresh.run(
        "patch.mailbox",
        json!({ "file": outbox.text("three.mbox") }),
    );
    assert_eq!(out["commits"], 3);
    assert!(out.get("stopped").is_none(), "{out}");
    assert_eq!(shape(&fresh.git()), shape(&source.git()));
    assert_eq!(status(&fresh.git()), "");

    // Uncommitted work: what is staged, then what is not.
    work(&source.scratch);
    source.run(
        "patch.export",
        json!({ "staged": true, "file": outbox.text("staged.diff") }),
    );
    source.run(
        "patch.export",
        json!({ "worktree": true, "file": outbox.text("worktree.diff") }),
    );
    let out = fresh.run(
        "patch.apply",
        json!({ "file": outbox.text("staged.diff"), "index": true }),
    );
    assert_eq!(out["index"], true);
    fresh.run(
        "patch.apply",
        json!({ "file": outbox.text("worktree.diff") }),
    );
    assert_eq!(status(&fresh.git()), status(&source.git()));
    assert_eq!(index(&fresh.git()), index(&source.git()));
    assert_eq!(
        fresh.git().run(["write-tree"]).unwrap(),
        source.git().run(["write-tree"]).unwrap()
    );
    for file in ["a.txt", "b.txt", "latin.txt", "bin.dat"] {
        assert_eq!(
            std::fs::read(fresh.file(file)).unwrap(),
            std::fs::read(source.file(file)).unwrap(),
            "{file}"
        );
    }
    assert!(!fresh.file("README.md").exists());
}

/// The numbers `git apply --numstat` gives, as the command answers them.
fn numstat(git: &Git, patch: &str) -> Value {
    let lines = git.lines(["apply", "--numstat", patch]).unwrap();
    let files: Vec<Value> = lines
        .iter()
        .map(|line| {
            let mut parts = line.splitn(3, '\t');
            let added = parts.next().unwrap().parse::<u64>().ok();
            let deleted = parts.next().unwrap().parse::<u64>().ok();
            json!({
                "path": parts.next().unwrap(),
                "added": added,
                "deleted": deleted,
                "binary": added.is_none(),
            })
        })
        .collect();
    json!(files)
}

#[test]
fn a_patch_is_applied_as_git_applies_it_and_undone_and_redone() {
    for with_index in [false, true] {
        let mut h = Side::new();
        let twin = copy();
        let maker = copy();
        let outbox = Outbox::new(&h.scratch);
        let patch = outbox.text("work.diff");
        work(&maker);
        maker
            .git()
            .run([
                "diff",
                "--binary",
                "HEAD",
                format!("--output={patch}").as_str(),
            ])
            .unwrap();
        let before = (status(&h.git()), index(&h.git()));
        assert_eq!(before.0, "");
        // A file written again with what it had: the index remembers it
        // otherwise, and git by itself would take it for changed and
        // refuse a patch that goes to the index.
        for file in ["a.txt", "b.txt", "README.md"] {
            let same = std::fs::read(h.file(file)).unwrap();
            std::fs::remove_file(h.file(file)).unwrap();
            std::fs::write(h.file(file), same).unwrap();
        }

        // What it would do, before it does it.
        let checked = h.run("patch.check", json!({ "file": patch, "index": with_index }));
        assert_eq!(checked["applies"], true, "{checked}");
        assert!(checked["reason"].is_null());
        assert_eq!(checked["files"], numstat(&twin.git(), &patch));
        assert_eq!(checked["total"], 5);
        assert_eq!((status(&h.git()), index(&h.git())), before, "only asked");

        let out = h.run("patch.apply", json!({ "file": patch, "index": with_index }));
        // By hand the index is looked over first: a fresh copy's files are
        // newer than what its index remembers of them.
        twin.git().run(["update-index", "-q", "--refresh"]).unwrap();
        let mut args = vec!["apply"];
        if with_index {
            args.push("--index");
        }
        args.push(patch.as_str());
        twin.git().run(&args).unwrap();
        let applied = (status(&h.git()), index(&h.git()));
        assert_eq!(applied, (status(&twin.git()), index(&twin.git())));
        assert_ne!(applied.0, "");
        for file in ["a.txt", "b.txt", "latin.txt", "bin.dat"] {
            assert_eq!(
                std::fs::read(h.file(file)).unwrap(),
                std::fs::read(twin.path().join(file)).unwrap(),
                "{file}"
            );
        }
        assert!(!h.file("README.md").exists());
        assert_eq!(out["index"], with_index);
        assert_eq!(out["reverse"], false);
        assert_eq!(out["undoable"], true);
        assert_eq!(out["conflicts"], json!([]));
        assert_eq!(out["files"], checked["files"]);
        // The patch is kept in the repository, byte for byte.
        let kept = out["patch"].as_str().unwrap().to_owned();
        assert_eq!(
            kept,
            h.git()
                .run(["hash-object", "--no-filters", patch.as_str()])
                .unwrap()
                .trim()
        );
        assert_eq!(
            h.git().run(["cat-file", "-t", kept.as_str()]).unwrap(),
            "blob\n"
        );

        // Undo is the patch the other way round; redo is the patch again.
        assert_eq!(
            h.run("undo.peek", json!({}))["undo"],
            "Apply patch work.diff"
        );
        let listed = h.run("undo.journal", json!({}));
        assert_eq!(listed["undo"][0]["kind"], "patch_apply");
        h.run("undo.last", json!({}));
        assert_eq!((status(&h.git()), index(&h.git())), before, "undone");
        assert!(h.file("README.md").is_file());
        assert!(!h.file("latin.txt").exists());
        // Redo is refused the same way when the patch no longer goes in:
        // a file it would make is there already.
        h.scratch.write("latin.txt", "in the way\n").unwrap();
        let in_the_way = (status(&h.git()), index(&h.git()));
        let err = h.try_run("redo.last", json!({})).unwrap_err();
        assert!(err.contains("does not apply that way any more"), "{err}");
        assert_eq!((status(&h.git()), index(&h.git())), in_the_way);
        assert_eq!(
            h.run("undo.peek", json!({}))["redo"],
            "Apply patch work.diff"
        );
        std::fs::remove_file(h.file("latin.txt")).unwrap();
        h.run("redo.last", json!({}));
        assert_eq!((status(&h.git()), index(&h.git())), applied, "redone");

        // Once a file it touches was changed again, the patch no longer
        // comes out, and undo says so and leaves everything as it is.
        let kept_text = std::fs::read(h.file("a.txt")).unwrap();
        h.scratch.write("a.txt", "edited since\n").unwrap();
        let edited = (status(&h.git()), index(&h.git()));
        let err = h.try_run("undo.last", json!({})).unwrap_err();
        assert!(err.contains("does not apply that way any more"), "{err}");
        assert!(err.contains("nothing was changed"), "{err}");
        assert_eq!((status(&h.git()), index(&h.git())), edited);
        assert_eq!(
            std::fs::read_to_string(h.file("a.txt")).unwrap(),
            "edited since\n"
        );
        assert!(h.file("latin.txt").is_file());
        assert_eq!(
            h.run("undo.peek", json!({}))["undo"],
            "Apply patch work.diff"
        );
        std::fs::write(h.file("a.txt"), kept_text).unwrap();

        // The other way round on request: an entry of its own, whose undo
        // puts the change back.
        let out = h.run(
            "patch.apply",
            json!({ "file": patch, "index": with_index, "reverse": true }),
        );
        assert_eq!(out["reverse"], true);
        assert_eq!((status(&h.git()), index(&h.git())), before, "reversed");
        assert_eq!(
            h.run("undo.peek", json!({}))["undo"],
            "Apply patch work.diff in reverse"
        );
        h.run("undo.last", json!({}));
        assert_eq!((status(&h.git()), index(&h.git())), applied);
        h.run("undo.last", json!({}));
        assert_eq!((status(&h.git()), index(&h.git())), before);
        assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    }
}

/// A patch that changes `a.txt` from what the fixture has, made in a copy
/// of its own.
fn patch_of_a(outbox: &Outbox) -> String {
    let maker = copy();
    let patch = outbox.text("a.diff");
    maker.write("a.txt", "from the patch\n").unwrap();
    maker.write("fine.txt", "applies anywhere\n").unwrap();
    maker.git().run(["add", "-A"]).unwrap();
    maker
        .git()
        .run([
            "diff",
            "--binary",
            "--cached",
            format!("--output={patch}").as_str(),
        ])
        .unwrap();
    patch
}

#[test]
fn a_patch_that_does_not_apply_changes_nothing_unless_it_is_merged() {
    let mut h = Side::new();
    let twin = copy();
    let outbox = Outbox::new(&h.scratch);
    let patch = patch_of_a(&outbox);
    for copy in [&h.scratch, &twin] {
        copy.write("a.txt", "changed here\n").unwrap();
        copy.git().run(["commit", "-q", "-am", "local a"]).unwrap();
    }
    let before = (status(&h.git()), index(&h.git()));

    // Not as it is: said as an answer when asked, refused when told to.
    let checked = h.run("patch.check", json!({ "file": patch }));
    assert_eq!(checked["applies"], false);
    let reason = checked["reason"].as_str().unwrap();
    assert!(reason.contains("a.txt"), "{reason}");
    assert_eq!(checked["total"], 2, "what it touches is known all the same");
    for index in [false, true] {
        let err = h
            .try_run("patch.apply", json!({ "file": patch, "index": index }))
            .unwrap_err();
        assert!(err.contains("does not apply"), "{err}");
        assert!(err.contains("nothing was changed"), "{err}");
    }
    assert_eq!((status(&h.git()), index(&h.git())), before);
    // All or nothing: the file that would have applied was not made.
    assert!(!h.file("fine.txt").exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    // Taken out it does not come either: it was never in.
    let err = h
        .try_run("patch.apply", json!({ "file": patch, "reverse": true }))
        .unwrap_err();
    assert!(err.contains("does not apply"), "{err}");

    // Merged, it applies and says where it left a conflict.
    let checked = h.run("patch.check", json!({ "file": patch, "three_way": true }));
    assert_eq!(checked["applies"], true, "{checked}");
    assert_eq!((status(&h.git()), index(&h.git())), before, "only asked");
    let out = h.run("patch.apply", json!({ "file": patch, "three_way": true }));
    assert!(twin.git().run(["apply", "--3way", patch.as_str()]).is_err());
    assert_eq!(out["conflicts"], json!(["a.txt"]));
    assert_eq!(out["index"], true, "a merge takes the index along");
    let same_as_twin = |h: &Side| {
        assert_eq!(status(&h.git()), status(&twin.git()));
        assert_eq!(index(&h.git()), index(&twin.git()));
        for file in ["a.txt", "fine.txt"] {
            assert_eq!(
                std::fs::read(h.file(file)).ok(),
                std::fs::read(twin.path().join(file)).ok(),
                "{file}"
            );
        }
    };
    same_as_twin(&h);
    assert_eq!(status(&h.git()), "UU a.txt\nA  fine.txt\n");
    // What the merge did is undone, conflict and all, as git's
    // `reset --merge` takes it back, and done again as it was.
    assert_eq!(out["undoable"], true);
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Apply patch a.diff");
    h.run("undo.last", json!({}));
    twin.git().run(["reset", "-q", "--merge"]).unwrap();
    same_as_twin(&h);
    assert_eq!((status(&h.git()), index(&h.git())), before);
    h.run("redo.last", json!({}));
    assert!(twin.git().run(["apply", "--3way", patch.as_str()]).is_err());
    same_as_twin(&h);
    assert_eq!(status(&h.git()), "UU a.txt\nA  fine.txt\n");

    // No operation is in progress, and the conflict is listed and
    // resolved like any other.
    assert!(h.run("op.status", json!({}))["operation"].is_null());
    let listed = h.run("conflict.list", json!({}));
    assert_eq!(listed["conflicts"].as_array().unwrap().len(), 1);
    assert_eq!(listed["conflicts"][0]["path"], "a.txt");
    assert_eq!(listed["conflicts"][0]["kind"], "both_modified");
    assert_eq!(listed["conflicts"][0]["reused"], false);
    h.run(
        "conflict.take",
        json!({ "path": "a.txt", "side": "theirs" }),
    );
    assert_eq!(
        std::fs::read_to_string(h.file("a.txt")).unwrap(),
        "from the patch\n"
    );
    assert_eq!(h.run("conflict.list", json!({}))["conflicts"], json!([]));
}

/// Three commits on a branch `topic` and the same as a mailbox: `a.txt`
/// changed, a file added, `a.txt` changed again. Then main moves on with
/// a change of its own to `a.txt` when `diverge` says so.
fn series(copy: &Scratch, mailbox: Option<&str>, diverge: bool) {
    let git = copy.git();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("a.txt", "topic's a\n").unwrap();
    git.run(["commit", "-q", "-am", "topic: change a"]).unwrap();
    copy.write("c.txt", "a new file\n").unwrap();
    git.run(["add", "c.txt"]).unwrap();
    git.run_with(
        ["commit", "-q", "-m", "topic: add c"],
        &[
            ("GIT_AUTHOR_NAME", "Bob Fixture"),
            ("GIT_AUTHOR_EMAIL", "bob@fixture.invalid"),
        ],
    )
    .unwrap();
    copy.write("a.txt", "topic's a, again\n").unwrap();
    git.run(["commit", "-q", "-am", "topic: change a again"])
        .unwrap();
    if let Some(mailbox) = mailbox {
        git.run([
            "format-patch",
            format!("--output={mailbox}").as_str(),
            "main..topic",
        ])
        .unwrap();
    }
    git.run(["switch", "-q", "main"]).unwrap();
    if diverge {
        copy.write("a.txt", "main's a\n").unwrap();
        git.run(["commit", "-q", "-am", "main: change a"]).unwrap();
    }
}

struct Mail {
    h: Side,
    twin: Scratch,
    _outbox: Outbox,
    mailbox: String,
    start: String,
}

impl Mail {
    fn new(diverge: bool) -> Self {
        let h = Side::new();
        let twin = copy();
        let outbox = Outbox::new(&h.scratch);
        let mailbox = outbox.text("series.mbox");
        series(&h.scratch, Some(&mailbox), diverge);
        series(&twin, None, diverge);
        let start = h.git().rev_parse("HEAD").unwrap();
        assert_eq!(start, twin.git().rev_parse("HEAD").unwrap());
        Self {
            h,
            twin,
            _outbox: outbox,
            mailbox,
            start,
        }
    }
    fn same(&self, what: &str) {
        assert_same(&self.h.git(), &self.twin.git(), what);
    }
    fn operation(&mut self) -> Value {
        self.h.run("op.status", json!({}))["operation"].clone()
    }
}

#[test]
fn a_mailbox_is_applied_as_git_am_applies_it() {
    let mut m = Mail::new(false);
    let out = m.h.run("patch.mailbox", json!({ "file": m.mailbox }));
    m.twin.git().run(["am", m.mailbox.as_str()]).unwrap();
    m.same("a mailbox");
    assert_eq!(out["commits"], 3);
    assert_eq!(out["before"]["commit"], m.start);
    assert_eq!(out["after"]["commit"], m.h.git().rev_parse("HEAD").unwrap());
    assert!(out.get("stopped").is_none());
    assert!(m.operation().is_null());
    // The commits are the ones of the branch they were made from, by
    // their authors.
    assert_eq!(
        m.h.git().rev_parse("HEAD^{tree}").unwrap(),
        m.h.git().rev_parse("topic^{tree}").unwrap()
    );
    assert_eq!(
        m.h.git().lines(["log", "--format=%an|%s", "-3"]).unwrap(),
        [
            "Ada Fixture|topic: change a again",
            "Bob Fixture|topic: add c",
            "Ada Fixture|topic: change a",
        ]
    );

    // One step of the journal, like any other move of the branch.
    let applied = m.h.git().rev_parse("HEAD").unwrap();
    assert_eq!(
        m.h.run("undo.peek", json!({}))["undo"],
        "Apply mailbox series.mbox"
    );
    m.h.run("undo.last", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), m.start);
    assert_eq!(status(&m.h.git()), "");
    m.h.run("redo.last", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), applied);

    // Applied once more there is nothing to it: git stops at the first
    // patch, which changes nothing now, and it is given up.
    let out = m.h.run("patch.mailbox", json!({ "file": m.mailbox }));
    assert!(out["stopped"].is_string(), "{out}");
    assert_eq!(m.operation()["kind"], "mailbox");
    m.h.run("op.abort", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), applied);
    assert!(m.operation().is_null());
}

#[test]
fn a_mailbox_that_conflicts_stops_and_is_continued() {
    let mut m = Mail::new(true);
    let out = m.h.run(
        "patch.mailbox",
        json!({ "file": m.mailbox, "three_way": true }),
    );
    assert!(
        m.twin
            .git()
            .run(["am", "--3way", m.mailbox.as_str()])
            .is_err()
    );
    m.same("stopped at the first patch");
    let said = out["stopped"].as_str().unwrap();
    assert!(said.contains("Patch failed at 0001"), "{said}");
    assert_eq!(out["conflicts"], json!(["a.txt"]));
    assert_eq!(out["commits"], 0);
    assert!(m.h.note().is_file());
    assert!(
        m.h.run("undo.peek", json!({}))["undo"].is_null(),
        "nothing journaled yet"
    );

    // It is its own kind of operation, not a rebase.
    let operation = m.operation();
    assert_eq!(operation["kind"], "mailbox");
    assert_eq!(operation["step"], json!([1, 3]));
    assert_eq!(operation["original_head"], m.start);
    assert_eq!(operation["can_skip"], true);
    assert_eq!(operation["editing"], false);
    assert!(operation["current"].is_null());
    assert!(operation["branch"].is_null());
    assert_eq!(operation["theirs"], "the patch \"topic: change a\"");
    assert_eq!(operation["conflicts"][0]["path"], "a.txt");
    assert_eq!(operation["conflicts"][0]["kind"], "both_modified");
    assert_eq!(operation["ready"], false, "a file is in conflict");
    // Nothing else begins meanwhile.
    m.h.refused("branch.merge", json!({ "target": "topic" }), "in progress");
    m.h.refused("patch.mailbox", json!({ "file": m.mailbox }), "in progress");
    m.h.refused("op.continue", json!({}), "1 file is still in conflict");

    // Resolved with the patch's side, it goes on to the end.
    m.h.run(
        "conflict.take",
        json!({ "path": "a.txt", "side": "theirs" }),
    );
    let twin = m.twin.git();
    twin.run(["checkout", "--theirs", "--", "a.txt"]).unwrap();
    twin.run(["add", "a.txt"]).unwrap();
    // Nothing is in conflict and what the patch changes is staged: the
    // state says it can go on, and it does.
    let operation = m.operation();
    assert_eq!(operation["conflicts"], json!([]));
    assert_eq!(operation["ready"], true, "{operation}");
    let done = m.h.run("op.continue", json!({}));
    twin.run(["am", "--continue"]).unwrap();
    assert_eq!(done["finished"], true, "{done}");
    assert!(done["operation"].is_null());
    m.same("continued to the end");
    assert_eq!(
        m.h.git()
            .rev_list_count(&format!("{}..HEAD", m.start))
            .unwrap(),
        3
    );
    assert!(!m.h.note().exists());

    // Finished from here, it is one step of the journal.
    assert_eq!(
        m.h.run("undo.peek", json!({}))["undo"],
        "Apply mailbox series.mbox"
    );
    m.h.run("undo.last", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), m.start);
    assert_eq!(status(&m.h.git()), "");
}

#[test]
fn a_mailbox_that_conflicts_is_skipped_patch_by_patch() {
    let mut m = Mail::new(true);
    m.h.run(
        "patch.mailbox",
        json!({ "file": m.mailbox, "three_way": true }),
    );
    let twin = m.twin.git();
    assert!(twin.run(["am", "--3way", m.mailbox.as_str()]).is_err());

    // Without the first patch the second applies and the third, which
    // builds on the first, conflicts in its turn.
    let next = m.h.run("op.skip", json!({}));
    assert!(twin.run(["am", "--skip"]).is_err());
    assert_eq!(next["finished"], false, "{next}");
    let said = next["message"].as_str().unwrap();
    assert!(said.contains("Patch failed at 0003"), "{said}");
    assert_eq!(next["operation"]["step"], json!([3, 3]));
    assert_eq!(next["operation"]["conflicts"][0]["path"], "a.txt");
    m.same("skipped to the third patch");
    assert_eq!(
        m.h.git()
            .rev_list_count(&format!("{}..HEAD", m.start))
            .unwrap(),
        1
    );

    let done = m.h.run("op.skip", json!({}));
    twin.run(["am", "--skip"]).unwrap();
    assert_eq!(done["finished"], true, "{done}");
    m.same("skipped to the end");
    assert_eq!(status(&m.h.git()), "");
    assert_eq!(
        m.h.git().lines(["log", "--format=%s", "-2"]).unwrap(),
        ["topic: add c", "main: change a"]
    );
    // What was applied on the way is journalled and comes off again.
    assert_eq!(
        m.h.run("undo.peek", json!({}))["undo"],
        "Apply mailbox series.mbox"
    );
    m.h.run("undo.last", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), m.start);
}

#[test]
fn a_mailbox_that_stops_is_aborted_back_to_where_it_began() {
    let mut m = Mail::new(true);
    let journal = m.h.run("undo.peek", json!({}));
    m.h.run(
        "patch.mailbox",
        json!({ "file": m.mailbox, "three_way": true }),
    );
    let twin = m.twin.git();
    assert!(twin.run(["am", "--3way", m.mailbox.as_str()]).is_err());
    // Past the first patch, so that there is a commit to take back.
    m.h.run("op.skip", json!({}));
    assert!(twin.run(["am", "--skip"]).is_err());
    assert_ne!(m.h.git().rev_parse("HEAD").unwrap(), m.start);

    let out = m.h.run("op.abort", json!({}));
    twin.run(["am", "--abort"]).unwrap();
    assert_eq!(out["aborted"], true);
    m.same("aborted");
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), m.start);
    assert_eq!(status(&m.h.git()), "");
    assert!(m.operation().is_null());
    assert!(!m.h.note().exists());
    assert_eq!(m.h.run("undo.peek", json!({})), journal);
    for command in ["op.continue", "op.skip", "op.abort"] {
        m.h.refused(command, json!({}), "nothing is in progress");
    }
}

#[test]
fn a_patch_of_a_mailbox_that_does_not_apply_stops_it_without_a_conflict() {
    let mut m = Mail::new(true);
    // Not merged: the patch is not applied at all, and no file is touched.
    let out = m.h.run(
        "patch.mailbox",
        json!({ "file": m.mailbox, "three_way": false }),
    );
    let twin = m.twin.git();
    assert!(twin.run(["am", "--no-3way", m.mailbox.as_str()]).is_err());
    m.same("stopped without a conflict");
    assert_eq!(out["conflicts"], json!([]));
    let said = out["stopped"].as_str().unwrap();
    assert!(said.contains("Patch failed at 0001"), "{said}");
    assert!(said.contains("a.txt"), "{said}");
    let operation = m.operation();
    assert_eq!(operation["kind"], "mailbox");
    assert_eq!(operation["step"], json!([1, 3]));
    assert_eq!(operation["conflicts"], json!([]));
    assert_eq!(status(&m.h.git()), "");

    // There is nothing to go on with until something is staged for it,
    // and the state says so before git does.
    assert_eq!(operation["ready"], false, "{operation}");
    let err = m.h.try_run("op.continue", json!({})).unwrap_err();
    assert!(err.contains("No changes"), "{err}");
    assert_eq!(m.operation()["step"], json!([1, 3]));
    assert_eq!(m.operation()["ready"], false);
    // A change that is not staged is not what the patch commits either.
    m.h.scratch.write("a.txt", "not staged\n").unwrap();
    assert_eq!(m.operation()["ready"], false);
    m.h.git().run(["checkout", "-q", "--", "a.txt"]).unwrap();

    // Skipping goes on: the second patch is applied, the third stops it
    // again, and that is an answer, not an error.
    let next = m.h.run("op.skip", json!({}));
    assert!(twin.run(["am", "--skip"]).is_err());
    assert_eq!(next["finished"], false, "{next}");
    assert_eq!(next["operation"]["step"], json!([3, 3]));
    assert_eq!(next["operation"]["conflicts"], json!([]));
    assert!(
        next["message"]
            .as_str()
            .unwrap()
            .contains("Patch failed at 0003"),
        "{next}"
    );
    m.same("skipped to a patch that does not apply");

    // Made to fit by hand, staged, and continued.
    assert_eq!(next["operation"]["ready"], false, "{next}");
    for copy in [&m.h.scratch, &m.twin] {
        copy.write("a.txt", "both, by hand\n").unwrap();
        copy.git().run(["add", "a.txt"]).unwrap();
    }
    assert_eq!(m.operation()["ready"], true, "staged: ready to continue");
    let done = m.h.run("op.continue", json!({}));
    twin.run(["am", "--continue"]).unwrap();
    assert_eq!(done["finished"], true, "{done}");
    m.same("continued after fitting the patch by hand");
    assert_eq!(
        m.h.git().lines(["log", "--format=%s", "-2"]).unwrap(),
        ["topic: change a again", "topic: add c"]
    );
    m.h.run("undo.last", json!({}));
    assert_eq!(m.h.git().rev_parse("HEAD").unwrap(), m.start);
}

#[test]
fn a_mailbox_does_not_begin_over_staged_work_or_another_operation() {
    let mut m = Mail::new(false);
    m.h.scratch.write("b.txt", "staged\n").unwrap();
    m.h.git().run(["add", "b.txt"]).unwrap();
    m.h.refused(
        "patch.mailbox",
        json!({ "file": m.mailbox }),
        "staged changes",
    );
    assert_eq!(status(&m.h.git()), "M  b.txt\n");
    assert!(!m.h.file(".git/rebase-apply").exists());
    assert!(m.operation().is_null());
    m.h.git().run(["reset", "-q", "--hard"]).unwrap();

    // Work that is not staged and that the patches do not touch stays.
    m.h.scratch.write("b.txt", "not staged\n").unwrap();
    m.twin.write("b.txt", "not staged\n").unwrap();
    let out = m.h.run("patch.mailbox", json!({ "file": m.mailbox }));
    m.twin.git().run(["am", m.mailbox.as_str()]).unwrap();
    assert_eq!(out["commits"], 3);
    m.same("over unstaged work");
    assert_eq!(status(&m.h.git()), " M b.txt\n");
}

/// Files no patch should be read from, in an outbox.
struct Hostile {
    garbage: String,
    binary: String,
    empty: String,
    huge: String,
    escaping: String,
    escaping_mail: String,
}

fn hostile_files(outbox: &Outbox, maker: &Scratch) -> Hostile {
    let write = |name: &str, bytes: &[u8]| {
        std::fs::write(outbox.at(name), bytes).unwrap();
        outbox.text(name)
    };
    let garbage = write("garbage.patch", b"this is no patch\nat all\n");
    let noise: Vec<u8> = (0u32..4096)
        .map(|n| (n.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let binary = write("binary.patch", &noise);
    let empty = write("empty.patch", b"");
    // Larger than is read, without filling the disk: a hole.
    let huge = outbox.text("huge.patch");
    std::fs::File::create(&huge)
        .unwrap()
        .set_len(bitbraid_git::patch::MAX_MAILBOX + 1)
        .unwrap();
    // A patch whose paths lead out of the repository.
    let escaping = write(
        "escaping.patch",
        b"diff --git a/../escaped.txt b/../escaped.txt\nnew file mode 100644\n--- /dev/null\n+++ b/../escaped.txt\n@@ -0,0 +1 @@\n+out of the repository\n",
    );
    // The same in a mailbox: a real patch with its paths rewritten.
    let git = maker.git();
    maker
        .write("inside.txt", "out of the repository\n")
        .unwrap();
    git.run(["add", "inside.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "add inside"]).unwrap();
    let mail = git
        .run(["format-patch", "--stdout", "-1", "HEAD"])
        .unwrap()
        .replace("inside.txt", "../escaped.txt");
    let escaping_mail = write("escaping.mbox", mail.as_bytes());
    Hostile {
        garbage,
        binary,
        empty,
        huge,
        escaping,
        escaping_mail,
    }
}

#[test]
fn hostile_patch_files_are_answered_or_refused_and_change_nothing() {
    let mut h = Side::new();
    let maker = copy();
    let outbox = Outbox::new(&h.scratch);
    let files = hostile_files(&outbox, &maker);
    let before = (status(&h.git()), index(&h.git()));
    let head = h.git().rev_parse("HEAD").unwrap();
    let escaped = h.scratch.path().parent().unwrap().join("escaped.txt");
    let _ = std::fs::remove_file(&escaped);
    let long = "p".repeat(5000);
    let dir = h.scratch.path().to_str().unwrap().to_owned();

    // Something that is not a patch is an answer when asked about, and
    // nothing to apply.
    for file in [&files.garbage, &files.binary, &files.empty] {
        let checked = h.run("patch.check", json!({ "file": file }));
        assert_eq!(checked["applies"], false, "{file}");
        assert!(checked["reason"].is_string(), "{file}");
        assert_eq!(checked["files"], json!([]));
        for params in [
            json!({ "file": file }),
            json!({ "file": file, "index": true }),
            json!({ "file": file, "three_way": true }),
            json!({ "file": file, "reverse": true }),
        ] {
            h.refused("patch.apply", params, "does not apply");
        }
        h.refused(
            "patch.mailbox",
            json!({ "file": file }),
            "git does not take it",
        );
        assert!(!h.file(".git/rebase-apply").exists(), "{file}");
    }

    // A patch that names a path outside the repository: git refuses it,
    // and that is what is answered.
    let checked = h.run("patch.check", json!({ "file": files.escaping }));
    assert_eq!(checked["applies"], false);
    assert!(
        checked["reason"].as_str().unwrap().contains("invalid path"),
        "{checked}"
    );
    assert_eq!(checked["files"][0]["path"], "../escaped.txt");
    for params in [
        json!({ "file": files.escaping }),
        json!({ "file": files.escaping, "index": true }),
        json!({ "file": files.escaping, "three_way": true }),
    ] {
        h.refused("patch.apply", params, "invalid path");
    }
    assert!(!escaped.exists());
    // In a mailbox git stops at it, in progress; given up, nothing is left.
    let out = h.run("patch.mailbox", json!({ "file": files.escaping_mail }));
    assert!(
        out["stopped"].as_str().unwrap().contains("invalid path"),
        "{out}"
    );
    assert_eq!(
        h.run("op.status", json!({}))["operation"]["kind"],
        "mailbox"
    );
    assert!(!escaped.exists());
    h.run("op.abort", json!({}));
    assert!(!escaped.exists());

    // What is no file to read a patch from.
    for file in [
        "",
        "a\nb",
        "nul\u{0}",
        "\u{7}bell",
        "no-such-file.patch",
        "../no-such-file.patch",
        "/dev/null",
        "/dev/zero",
        ".",
        "..",
        dir.as_str(),
        long.as_str(),
        files.huge.as_str(),
    ] {
        for command in ["patch.check", "patch.apply", "patch.mailbox"] {
            assert!(
                h.try_run(command, json!({ "file": file })).is_err(),
                "{command} took {:?}",
                file.get(..file.len().min(40))
            );
        }
    }
    for (command, params) in [
        ("patch.check", json!({})),
        ("patch.check", json!({ "file": 3 })),
        ("patch.check", json!({ "file": ["a"] })),
        (
            "patch.check",
            json!({ "file": files.garbage, "index": "yes" }),
        ),
        ("patch.apply", json!({})),
        ("patch.apply", json!({ "file": null })),
        (
            "patch.apply",
            json!({ "file": files.garbage, "three_way": 1 }),
        ),
        (
            "patch.apply",
            json!({ "file": files.garbage, "reverse": [] }),
        ),
        ("patch.mailbox", json!({})),
        ("patch.mailbox", json!({ "file": {} })),
        (
            "patch.mailbox",
            json!({ "file": files.garbage, "three_way": "no" }),
        ),
    ] {
        assert!(
            h.try_run(command, params.clone()).is_err(),
            "{command} {params}"
        );
    }

    assert_eq!((status(&h.git()), index(&h.git())), before);
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), head);
    assert!(h.run("op.status", json!({}))["operation"].is_null());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    assert!(!h.file(".git/rebase-apply").exists());
}

#[test]
fn hostile_export_parameters_are_refused_and_write_nothing() {
    let mut h = Side::new();
    let outbox = Outbox::new(&h.scratch);
    work(&h.scratch);
    let before = (status(&h.git()), index(&h.git()));
    let long = "e".repeat(5000);
    let fine = outbox.text("fine.mbox");
    let pwned = outbox.text("pwned");
    std::fs::write(outbox.at("taken.diff"), "already here\n").unwrap();
    std::fs::write(outbox.at("a-file"), "not a directory\n").unwrap();
    std::fs::create_dir(outbox.at("a-dir")).unwrap();

    // What does not name commits.
    let option = format!("--output={pwned}");
    for rev in [
        "",
        "-1",
        "--all",
        option.as_str(),
        "no-such-rev",
        "HEAD^{tree}",
        "HEAD:a.txt",
        "a\nb",
        "nul\u{0}",
        long.as_str(),
    ] {
        for params in [
            json!({ "commits": [rev], "file": fine }),
            json!({ "commits": ["HEAD", rev], "dir": outbox.text("made") }),
            json!({ "from": rev, "file": fine }),
            json!({ "from": "HEAD~1", "to": rev, "file": fine }),
        ] {
            assert!(
                h.try_run("patch.export", params.clone()).is_err(),
                "patch.export took the revision {:?}",
                rev.get(..rev.len().min(40))
            );
        }
    }
    // More than one export takes, and nothing at all.
    let many: Vec<&str> =
        std::iter::repeat_n("HEAD", bitbraid_git::patch::MAX_EXPORT + 1).collect();
    h.refused(
        "patch.export",
        json!({ "commits": many, "file": fine }),
        "at most",
    );
    h.refused(
        "patch.export",
        json!({ "commits": [], "file": fine }),
        "no commits",
    );
    h.refused(
        "patch.export",
        json!({ "from": "HEAD", "file": fine }),
        "no commit with a patch",
    );
    h.refused(
        "patch.export",
        json!({ "from": "HEAD", "to": "HEAD~2", "dir": outbox.text("made") }),
        "no commit with a patch",
    );

    // Where nothing may or can be written.
    let inside = h.file(".git").to_str().unwrap().to_owned();
    for place in [
        "",
        "a\nb",
        "nul\u{0}",
        long.as_str(),
        ".git/exported",
        ".git/hooks/exported",
        ".git/config",
        inside.as_str(),
        "no/such/directory/deep/down",
    ] {
        for params in [
            json!({ "from": "HEAD~1", "file": place, "overwrite": true }),
            json!({ "from": "HEAD~1", "dir": place, "overwrite": true }),
            json!({ "staged": true, "file": place, "overwrite": true }),
        ] {
            assert!(
                h.try_run("patch.export", params.clone()).is_err(),
                "patch.export wrote to {:?}",
                place.get(..place.len().min(40))
            );
        }
    }
    h.refused(
        "patch.export",
        json!({ "staged": true, "file": outbox.text("taken.diff") }),
        "exists already",
    );
    h.refused(
        "patch.export",
        json!({ "from": "HEAD~1", "file": outbox.text("a-dir"), "overwrite": true }),
        "is a directory",
    );
    h.refused(
        "patch.export",
        json!({ "from": "HEAD~1", "dir": outbox.text("a-file"), "overwrite": true }),
        "is not a directory",
    );
    h.refused(
        "patch.export",
        json!({ "from": "HEAD~1", "dir": outbox.text("not/there") }),
        "does not exist",
    );
    h.refused(
        "patch.export",
        json!({ "staged": true, "dir": outbox.text("made") }),
        "one file",
    );

    // One source and one destination, of the right shape.
    for params in [
        json!({}),
        json!({ "file": fine }),
        json!({ "from": "HEAD~1" }),
        json!({ "to": "HEAD", "file": fine }),
        json!({ "commits": ["HEAD"], "to": "HEAD", "file": fine }),
        json!({ "commits": ["HEAD"], "from": "HEAD~1", "file": fine }),
        json!({ "commits": ["HEAD"], "staged": true, "file": fine }),
        json!({ "staged": true, "worktree": true, "file": fine }),
        json!({ "from": "HEAD~1", "worktree": true, "file": fine }),
        json!({ "from": "HEAD~1", "file": fine, "dir": outbox.text("made") }),
        json!({ "commits": "HEAD", "file": fine }),
        json!({ "commits": [3], "file": fine }),
        json!({ "from": ["HEAD~1"], "file": fine }),
        json!({ "staged": "yes", "file": fine }),
        json!({ "from": "HEAD~1", "file": 3 }),
        json!({ "from": "HEAD~1", "dir": [] }),
        json!({ "from": "HEAD~1", "file": fine, "overwrite": "yes" }),
    ] {
        assert!(
            h.try_run("patch.export", params.clone()).is_err(),
            "patch.export {params}"
        );
    }

    // Stopped, an export has written nothing, and the directory that was
    // made for it is gone again.
    h.ctx
        .cancel
        .store(true, std::sync::atomic::Ordering::SeqCst);
    for params in [
        json!({ "from": "HEAD~3", "dir": outbox.text("made") }),
        json!({ "commits": ["HEAD", "HEAD~1"], "dir": outbox.text("made") }),
        json!({ "from": "HEAD~3", "file": fine }),
        json!({ "commits": ["HEAD"], "file": fine }),
        json!({ "staged": true, "file": fine }),
    ] {
        h.refused("patch.export", params, "cancelled");
    }
    h.ctx
        .cancel
        .store(false, std::sync::atomic::Ordering::SeqCst);

    // Nothing was written anywhere, and nothing was changed.
    assert_eq!(
        names_in(&outbox.0),
        ["a-dir", "a-file", "taken.diff"],
        "the outbox has what the test put there"
    );
    assert_eq!(
        std::fs::read_to_string(outbox.at("taken.diff")).unwrap(),
        "already here\n"
    );
    assert!(names_in(&outbox.at("a-dir")).is_empty());
    for inside in ["exported", "hooks/exported"] {
        assert!(!h.file(".git").join(inside).exists());
    }
    assert!(
        std::fs::read_to_string(h.file(".git/config"))
            .unwrap()
            .contains("[core]"),
        "the configuration is what it was"
    );
    nothing_left_behind(h.scratch.path());
    assert_eq!((status(&h.git()), index(&h.git())), before);
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // Uncommitted work has to be there to be written.
    h.git().run(["reset", "-q", "--hard"]).unwrap();
    h.refused(
        "patch.export",
        json!({ "staged": true, "file": fine }),
        "nothing is staged",
    );
    h.refused(
        "patch.export",
        json!({ "worktree": true, "file": fine }),
        "no tracked file is changed",
    );
    assert!(!outbox.at("fine.mbox").exists());
}

/// Put `entries` into the journal file as its undo and redo stacks, and
/// have the context read it.
fn write_journal(h: &mut Side, done: Value, undone: Value) {
    let dir = h.file(".git/bitbraid");
    std::fs::create_dir_all(&dir).unwrap();
    let journal = json!({ "next_id": 50, "done": done, "undone": undone });
    std::fs::write(dir.join("journal.json"), journal.to_string()).unwrap();
    h.ctx.reload_journal();
}

fn patch_entry(id: u64, patch: &str) -> Value {
    json!({
        "id": id,
        "label": "Apply patch from a journal written by hand",
        "time": 0,
        "change": { "kind": "patch_apply", "patch": patch, "index": false, "reverse": false },
    })
}

#[test]
fn a_journal_entry_that_names_no_patch_is_refused_not_acted_on() {
    let mut h = Side::new();
    let git = h.git();
    h.scratch.write("b.txt", "local work\n").unwrap();
    let before = (status(&git), index(&git));
    let head = git.rev_parse("HEAD").unwrap();
    let zeros = "0".repeat(40);
    let missing = "0123456789abcdef0123456789abcdef01234567".to_owned();
    let commit = head.clone();
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    let tag = {
        git.run(["tag", "-a", "-m", "annotated", "v1"]).unwrap();
        git.rev_parse("v1").unwrap()
    };
    // A blob, but of a file and not of a patch.
    let not_a_patch = git.rev_parse("HEAD:a.txt").unwrap();
    let before_refs = git
        .run(["for-each-ref", "--format=%(refname) %(objectname)"])
        .unwrap();

    for id in [&zeros, &missing, &commit, &tree, &tag, &not_a_patch] {
        write_journal(&mut h, json!([patch_entry(7, id)]), json!([]));
        let err = h.try_run("undo.last", json!({})).unwrap_err();
        assert!(err.contains("nothing was changed"), "{id}: {err}");
        // It stays where it is, to be seen.
        assert_eq!(
            h.run("undo.peek", json!({}))["undo"],
            "Apply patch from a journal written by hand"
        );

        write_journal(&mut h, json!([]), json!([patch_entry(7, id)]));
        let err = h.try_run("redo.last", json!({})).unwrap_err();
        assert!(err.contains("nothing was changed"), "{id}: {err}");

        // Going through the whole journal stops at it, and says so.
        write_journal(&mut h, json!([patch_entry(7, id)]), json!([]));
        let out = h.run("undo.to", json!({ "id": 7 }));
        assert_eq!(out["complete"], false, "{id}: {out}");
        assert_eq!(out["undone"], json!([]));
    }
    // An id that is no id at all does not make it into a journal.
    write_journal(&mut h, json!([patch_entry(7, "--output=pwned")]), json!([]));
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    assert!(h.run("undo.last", json!({}))["undone"].is_null());

    assert_eq!((status(&git), index(&git)), before);
    assert_eq!(git.rev_parse("HEAD").unwrap(), head);
    assert_eq!(
        git.run(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap(),
        before_refs
    );
    assert_eq!(
        std::fs::read_to_string(h.file("b.txt")).unwrap(),
        "local work\n"
    );
    assert!(!h.file("pwned").exists());
}

#[test]
fn a_mailbox_whose_state_is_not_as_expected_is_read_without_a_crash() {
    let mut m = Mail::new(true);
    m.h.run(
        "patch.mailbox",
        json!({ "file": m.mailbox, "three_way": true }),
    );
    assert_eq!(m.operation()["kind"], "mailbox");
    let state = m.h.file(".git/rebase-apply");
    let big = "x".repeat(2 * 1024 * 1024);

    // What git keeps about the mailbox, damaged in turn. HEAD, the
    // objects and the references are left alone: git has to go on taking
    // this for a repository.
    for (file, content) in [
        ("next", ""),
        ("next", "garbage\n"),
        ("next", "-1\n"),
        ("next", "99999999999999999999\n"),
        ("last", ""),
        ("last", "0\n"),
        ("last", "\u{0}\u{1}\u{2}"),
        ("final-commit", ""),
        ("final-commit", "\u{0}\u{1}\u{2}\n\n\n"),
        ("final-commit", big.as_str()),
        ("../ORIG_HEAD", ""),
        ("../ORIG_HEAD", "garbage that is no id\n"),
        ("../MERGE_RR", "\u{0}\u{0}garbage"),
    ] {
        let path = state.join(file);
        let saved = std::fs::read(&path).ok();
        std::fs::write(&path, content).unwrap();
        // An answer, never a crash; and still a mailbox.
        let operation = m.operation();
        assert_eq!(operation["kind"], "mailbox", "{file} {content:?}");
        let _ = m.h.try_run("conflict.list", json!({}));
        let _ = m.h.try_run("rerere.status", json!({}));
        // A note whose operation began somewhere else is not this one's.
        match saved {
            Some(bytes) => std::fs::write(&path, bytes).unwrap(),
            None => drop(std::fs::remove_file(&path)),
        }
    }
    // The subject of a patch is shown in part, however long it is.
    std::fs::write(state.join("final-commit"), "s".repeat(5000)).unwrap();
    let theirs = m.operation()["theirs"].as_str().unwrap().to_owned();
    assert!(theirs.len() < 100, "{}", theirs.len());

    // Without its marker the directory is a rebase's, and without
    // anything in it nothing can be told; neither is a crash.
    std::fs::remove_file(state.join("applying")).unwrap();
    assert_eq!(m.operation()["kind"], "rebase");
    std::fs::write(state.join("applying"), "").unwrap();
    assert_eq!(m.operation()["kind"], "mailbox");
    for file in [
        "next",
        "last",
        "final-commit",
        "patch",
        "info",
        "author-script",
    ] {
        let _ = std::fs::remove_file(state.join(file));
    }
    let operation = m.operation();
    assert_eq!(operation["kind"], "mailbox");
    assert!(operation["step"].is_null());
    assert_eq!(operation["theirs"], "the patch being applied");
    // Going on is an answer or a refusal; giving up is git's to do.
    let _ = m.h.try_run("op.continue", json!({}));
    let _ = m.h.try_run("op.skip", json!({}));
    let _ = m.h.try_run("op.abort", json!({}));
    let _ = m.h.try_run("op.status", json!({}));
}
