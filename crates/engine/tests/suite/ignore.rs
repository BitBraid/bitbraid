//! Ignoring against git: a pattern made from a path ignores
//! that path and nothing beside it, the three files a pattern can go to
//! are the ones git reads, a tracked file that is covered is named and
//! stops being tracked the way `git rm --cached` stops it.
//!
//! The engine runs git with the developer's own configuration, so every
//! copy here names an ignore file of its own (`core.excludesFile`): the
//! "global" target then is a scratch file, and nobody's real one is read
//! or written.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

/// The scratch file that stands for the user's own ignore file.
fn own_ignore_file(copy: &Scratch) -> PathBuf {
    copy.path().with_extension("ignore")
}

/// A copy of a fixture whose "global" ignore file is a scratch file.
fn copy_of(fixture: &str) -> Scratch {
    let copy = Scratch::of(fixture).unwrap();
    let own = own_ignore_file(&copy);
    std::fs::write(&own, "").unwrap();
    copy.git()
        .run(["config", "core.excludesFile", own.to_str().unwrap()])
        .unwrap();
    copy
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = copy_of(fixture);
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
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(own_ignore_file(&self.scratch));
    }
}

fn ignored(git: &Git, path: &str) -> bool {
    git.run(["check-ignore", "-q", "--no-index", "--", path])
        .is_ok()
}

/// Everything git says about the files: staged, changed, untracked, ignored.
fn status(git: &Git) -> Vec<String> {
    git.lines([
        "status",
        "--porcelain=v2",
        "--untracked-files=all",
        "--ignored=matching",
    ])
    .unwrap()
}

fn index(git: &Git) -> Vec<String> {
    git.lines(["ls-files", "-s"]).unwrap()
}

#[test]
fn a_pattern_made_from_a_path_ignores_that_path_and_nothing_beside_it() {
    let mut h = Harness::new("linear");
    // (the path, something close to it that must stay unignored)
    let cases = [
        ("notes.txt", "sub/notes.txt"),
        ("sub/deep/file.rs", "other/sub/deep/file.rs"),
        ("#hash", "hash"),
        ("!bang", "bang"),
        ("star*name", "starXname"),
        ("quest?on", "question"),
        ("[abc]", "a"),
        ("back\\slash", "backslash"),
        ("ends in blank ", "ends in blank"),
        ("  starts with blanks", "starts with blanks"),
        ("ünï/çödé.txt", "uni/code.txt"),
        ("a b/c d.txt", "a b/c e.txt"),
    ];
    // Windows takes no `*` or `?` in a name and none that ends in a blank,
    // and `\` is a separator there: those names cannot be files there.
    let cases: Vec<(&str, &str)> = cases
        .into_iter()
        .filter(|(path, _)| {
            !cfg!(windows) || !(path.contains(['*', '?', '\\']) || path.ends_with(' '))
        })
        .collect();
    for &(path, near) in &cases {
        for file in [path, near] {
            h.scratch.write(file, "x\n").unwrap();
        }
        assert!(!ignored(&h.git(), path), "{path:?} before");
        let out = h.run("ignore.add", json!({ "path": path, "what": "file" }));
        assert_eq!(out["added"], true, "{path:?}");
        assert!(ignored(&h.git(), path), "{path:?} with {}", out["pattern"]);
        assert!(!ignored(&h.git(), near), "{near:?} with {}", out["pattern"]);
    }
    // Asked again, nothing is written twice.
    let before = std::fs::read(h.scratch.path().join(".gitignore")).unwrap();
    for &(path, _) in &cases {
        let out = h.run("ignore.add", json!({ "path": path }));
        assert_eq!(out["added"], false, "{path:?}");
    }
    assert_eq!(
        std::fs::read(h.scratch.path().join(".gitignore")).unwrap(),
        before
    );
}

#[test]
fn a_folder_and_an_extension_are_ignored_as_the_same_line_written_by_hand() {
    let mut h = Harness::new("linear");
    let twin = copy_of("linear");
    for copy in [&h.scratch, &twin] {
        for file in [
            "build/out/a.o",
            "build/out/deep/b.o",
            "build/keep.txt",
            "other/build/out/c.o",
            "logs/x.log",
            "y.log",
            "y.log.txt",
            "odd [dir]/z.tmp",
        ] {
            copy.write(file, "x\n").unwrap();
        }
    }
    let out = h.run(
        "ignore.add",
        json!({ "path": "build/out", "what": "folder" }),
    );
    assert_eq!(out["pattern"], "/build/out/");
    let out = h.run(
        "ignore.add",
        json!({ "path": "logs/x.log", "what": "extension" }),
    );
    assert_eq!(out["pattern"], "*.log");
    let out = h.run(
        "ignore.add",
        json!({ "path": "odd [dir]/", "what": "folder" }),
    );
    assert_eq!(out["pattern"], "/odd \\[dir]/");
    twin.write(".gitignore", "/build/out/\n*.log\n/odd \\[dir]/\n")
        .unwrap();
    assert_eq!(
        std::fs::read(h.scratch.path().join(".gitignore")).unwrap(),
        std::fs::read(twin.path().join(".gitignore")).unwrap()
    );
    assert_eq!(status(&h.git()), status(&twin.git()));
    let said = status(&h.git()).join("\n");
    assert!(said.contains("! build/out/"), "{said}");
    assert!(said.contains("? build/keep.txt"), "{said}");
    assert!(said.contains("? other/build/out/c.o"), "{said}");
    assert!(
        said.contains("! y.log") && said.contains("? y.log.txt"),
        "{said}"
    );
    let _ = std::fs::remove_file(own_ignore_file(&twin));

    // What has no extension has none to ignore.
    for path in ["Makefile", ".hidden", "dir.d/Makefile"] {
        let err = h
            .try_run("ignore.add", json!({ "path": path, "what": "extension" }))
            .unwrap_err();
        assert!(err.contains("no extension"), "{path}: {err}");
    }
}

#[test]
fn each_target_is_the_file_git_reads_for_it() {
    let mut h = Harness::new("linear");
    let twin = copy_of("linear");
    for copy in [&h.scratch, &twin] {
        for file in ["one.shared", "two.mine", "three.everywhere", "four.kept"] {
            copy.write(file, "x\n").unwrap();
        }
    }
    let shared = h.run("ignore.add", json!({ "pattern": "*.shared" }));
    let mine = h.run(
        "ignore.add",
        json!({ "pattern": "*.mine", "target": "local" }),
    );
    let everywhere = h.run(
        "ignore.add",
        json!({ "pattern": "*.everywhere", "target": "global" }),
    );
    assert_eq!(
        PathBuf::from(shared["file"].as_str().unwrap()),
        h.scratch.path().join(".gitignore")
    );
    assert_eq!(
        PathBuf::from(mine["file"].as_str().unwrap()),
        h.scratch.path().join(".git/info/exclude")
    );
    assert_eq!(
        PathBuf::from(everywhere["file"].as_str().unwrap()),
        own_ignore_file(&h.scratch)
    );
    // The twin's three files written by hand.
    twin.write(".gitignore", "*.shared\n").unwrap();
    let exclude = twin.path().join(".git/info/exclude");
    std::fs::create_dir_all(exclude.parent().unwrap()).unwrap();
    let mut text = std::fs::read_to_string(&exclude).unwrap_or_default();
    text.push_str("*.mine\n");
    std::fs::write(&exclude, text).unwrap();
    std::fs::write(own_ignore_file(&twin), "*.everywhere\n").unwrap();
    assert_eq!(status(&h.git()), status(&twin.git()));
    let said = status(&h.git()).join("\n");
    for gone in ["! one.shared", "! two.mine", "! three.everywhere"] {
        assert!(said.contains(gone), "{said}");
    }
    assert!(said.contains("? four.kept"), "{said}");
    // Only the shared one shows as a change to commit.
    assert!(said.contains("? .gitignore"), "{said}");
    assert_eq!(
        std::fs::read_to_string(own_ignore_file(&h.scratch)).unwrap(),
        "*.everywhere\n"
    );
    let _ = std::fs::remove_file(own_ignore_file(&twin));
}

#[test]
fn the_local_target_of_a_linked_worktree_is_the_file_its_git_reads() {
    let h = Harness::new("linear");
    let linked = h.scratch.path().with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    h.git()
        .run([
            "worktree",
            "add",
            "-q",
            "--detach",
            linked.to_str().unwrap(),
        ])
        .unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": linked }))
        .unwrap();
    std::fs::write(linked.join("scratch.mine"), "x\n").unwrap();
    let home = h.git().home().to_path_buf();
    let there = Git::new(&linked, home);
    assert!(!ignored(&there, "scratch.mine"));
    let out = registry
        .run(
            "ignore.add",
            &mut ctx,
            &json!({ "pattern": "*.mine", "target": "local" }),
        )
        .unwrap();
    // Not the worktree's own directory under `worktrees/`, which git does
    // not read ignore rules from.
    assert_eq!(
        PathBuf::from(out["file"].as_str().unwrap()),
        h.scratch.path().join(".git/info/exclude")
    );
    assert!(ignored(&there, "scratch.mine"));
    assert!(ignored(&h.git(), "anything.mine"));
    h.git()
        .run(["worktree", "remove", "--force", linked.to_str().unwrap()])
        .unwrap();
    let _ = std::fs::remove_dir_all(&linked);
}

#[test]
fn what_an_ignore_file_holds_is_kept_byte_for_byte() {
    let mut h = Harness::new("linear");
    let file = h.scratch.path().join(".gitignore");
    // Not text in any encoding, DOS line endings, and no line end at the
    // end: all of it stays, and the new line ends as the others do.
    let mut held = b"# caf\xe9 \xff\xfe\r\n*.o\r\nlast".to_vec();
    std::fs::write(&file, &held).unwrap();
    let out = h.run("ignore.add", json!({ "pattern": "*.tmp" }));
    assert_eq!(out["added"], true);
    held.extend_from_slice(b"\r\n*.tmp\r\n");
    assert_eq!(std::fs::read(&file).unwrap(), held);
    // A pattern that is there, DOS ending or not, is not added again.
    for there in ["*.o", "last", "*.tmp", "  *.tmp  "] {
        let out = h.run("ignore.add", json!({ "pattern": there }));
        assert_eq!(out["added"], false, "{there:?}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), held);
    assert!(ignored(&h.git(), "x.tmp") && ignored(&h.git(), "x.o"));

    // An ignore file that cannot be read is not written over.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir(&file).unwrap();
    let err = h
        .try_run("ignore.add", json!({ "pattern": "*.bak" }))
        .unwrap_err();
    assert!(err.contains("ignore file"), "{err}");
    assert!(file.is_dir());
}

#[test]
fn a_tracked_file_that_is_covered_is_named_and_stops_being_tracked_as_git_stops_it() {
    let mut h = Harness::new("linear");
    let twin = copy_of("linear");
    for copy in [&h.scratch, &twin] {
        copy.write("gen/one.txt", "1\n").unwrap();
        copy.write("gen/two.txt", "2\n").unwrap();
        copy.write("gen/sub/three.txt", "3\n").unwrap();
        copy.git().run(["add", "--", "gen"]).unwrap();
        copy.git()
            .run(["commit", "-q", "-m", "generated files, by mistake"])
            .unwrap();
        // One of them was changed since.
        copy.write("gen/two.txt", "2\nchanged\n").unwrap();
    }
    let listed = |git: &Git| {
        git.lines(["ls-files", "--cached", "--ignored", "--exclude-standard"])
            .unwrap()
    };
    assert_eq!(h.run("ignore.tracked", json!({}))["count"], 0);

    // The pattern covers three tracked files; they are named, and stay.
    let out = h.run("ignore.add", json!({ "path": "gen", "what": "folder" }));
    twin.write(".gitignore", "/gen/\n").unwrap();
    assert_eq!(out["tracked_count"], 3);
    assert_eq!(
        out["tracked"],
        json!(["gen/one.txt", "gen/sub/three.txt", "gen/two.txt"])
    );
    let tracked = h.run("ignore.tracked", json!({}));
    assert_eq!(tracked["paths"], json!(listed(&twin.git())));
    assert_eq!(tracked["count"], 3);
    assert_eq!(
        h.run("ignore.tracked", json!({ "limit": 1 })),
        json!({ "paths": ["gen/one.txt"], "count": 3 })
    );
    assert_eq!(index(&h.git()), index(&twin.git()));
    // A second pattern names only what it brought.
    let out = h.run("ignore.add", json!({ "pattern": "a.txt" }));
    assert_eq!(out["tracked"], json!(["a.txt"]));
    let original = index(&h.git());

    // Stop tracking: one file, then the folder with what is left in it.
    let out = h.run("ignore.untrack", json!({ "paths": ["gen/one.txt"] }));
    assert_eq!(out["untracked"], json!(["gen/one.txt"]));
    twin.write(".gitignore", "/gen/\na.txt\n").unwrap();
    twin.git()
        .run(["rm", "--cached", "-q", "--", "gen/one.txt"])
        .unwrap();
    assert_eq!(index(&h.git()), index(&twin.git()));
    let out = h.run("ignore.untrack", json!({ "paths": ["gen", "a.txt"] }));
    assert_eq!(
        out["untracked"],
        json!(["a.txt", "gen/sub/three.txt", "gen/two.txt"])
    );
    assert_eq!(out["count"], 3);
    twin.git()
        .run(["rm", "--cached", "-r", "-q", "--", "gen", "a.txt"])
        .unwrap();
    assert_eq!(index(&h.git()), index(&twin.git()));
    assert_eq!(status(&h.git()), status(&twin.git()));
    assert_eq!(h.run("ignore.tracked", json!({}))["count"], 0);
    // The files are where they were, with what they held.
    for (file, text) in [
        ("gen/one.txt", "1\n"),
        ("gen/two.txt", "2\nchanged\n"),
        ("gen/sub/three.txt", "3\n"),
    ] {
        assert_eq!(
            std::fs::read_to_string(h.scratch.path().join(file)).unwrap(),
            text
        );
    }

    // It is staged like any deletion: unstaging takes it back.
    h.run(
        "unstage.paths",
        json!({ "paths": ["gen/one.txt", "gen/two.txt", "gen/sub/three.txt", "a.txt"] }),
    );
    assert_eq!(index(&h.git()), original);
    let _ = std::fs::remove_file(own_ignore_file(&twin));
}

#[test]
fn stop_tracking_refuses_what_git_refuses_and_changes_nothing_then() {
    let mut h = Harness::new("linear");
    let before = index(&h.git());
    // Not tracked, not there, outside, no paths at all.
    h.scratch.write("new.txt", "n\n").unwrap();
    for paths in [
        json!(["new.txt"]),
        json!(["nowhere.txt"]),
        json!(["a.txt", "nowhere.txt"]),
        json!(["../outside"]),
        json!(["/etc/passwd"]),
        json!([""]),
        json!([]),
    ] {
        let err = h.try_run("ignore.untrack", json!({ "paths": paths }));
        assert!(err.is_err(), "{paths}: {err:?}");
        assert_eq!(index(&h.git()), before, "{paths}");
    }
    // Staged content that is in neither the commit nor the file would be
    // lost: git refuses, and so does this.
    h.scratch.write("a.txt", "staged\n").unwrap();
    h.git().run(["add", "--", "a.txt"]).unwrap();
    h.scratch.write("a.txt", "and changed again\n").unwrap();
    let staged = index(&h.git());
    let err = h
        .try_run("ignore.untrack", json!({ "paths": ["a.txt"] }))
        .unwrap_err();
    assert!(err.contains("staged content"), "{err}");
    assert_eq!(index(&h.git()), staged);
}

#[test]
fn hostile_parameters_are_refused_or_taken_as_text_and_reach_no_other_file() {
    let mut h = Harness::new("linear");
    let head = h.git().rev_parse("HEAD").unwrap();
    let staged = index(&h.git());
    let hostile = [
        Value::Null,
        json!(""),
        json!("   "),
        json!("-"),
        json!("--all"),
        json!("--output=/tmp/bb-injected"),
        json!("../../../../etc/passwd"),
        json!("/etc/passwd"),
        json!("a\u{0}b"),
        json!("one\ntwo\r\n"),
        json!("#comment"),
        json!("!"),
        json!("\u{202e}\u{feff}名前 🦑"),
        json!("x".repeat(70_000)),
        json!(-1),
        json!(u64::MAX),
        json!(1e308),
        json!(true),
        json!([]),
        json!({}),
        json!(["--force", "", null, 7, ["nested"]]),
    ];
    for value in &hostile {
        for params in [
            json!({ "pattern": value }),
            json!({ "pattern": value, "target": "local" }),
            json!({ "path": value }),
            json!({ "path": value, "what": "folder" }),
            json!({ "path": value, "what": "extension" }),
            json!({ "path": "a.txt", "what": value }),
            json!({ "pattern": "*.x", "target": value }),
            json!({ "pattern": "*.x", "path": value }),
        ] {
            // Refused, or taken as a line of text for an ignore file.
            let _ = h.try_run("ignore.add", params);
        }
        let _ = h.try_run("ignore.tracked", json!({ "limit": value }));
        let _ = h.try_run("ignore.untrack", json!({ "paths": value }));
        let _ = h.try_run("ignore.untrack", json!({ "paths": [value] }));
        let _ = h.try_run("ignore.untrack", json!({ "paths": ["a.txt", value] }));
    }
    for shape in [Value::Null, json!([]), json!("text"), json!(7)] {
        for id in ["ignore.add", "ignore.untrack"] {
            assert!(h.try_run(id, shape.clone()).is_err(), "{id} {shape}");
        }
        let _ = h.try_run("ignore.tracked", shape);
    }
    // No commit moved, nothing left the index, and every line that was
    // written is one line in one of the two files a pattern was sent to.
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(index(&h.git()), staged);
    assert!(!std::path::Path::new("/tmp/bb-injected").exists());
    assert_eq!(
        std::fs::read_to_string(own_ignore_file(&h.scratch)).unwrap(),
        "",
        "nothing asked for the user's own file"
    );
    let written = std::fs::read_to_string(h.scratch.path().join(".gitignore")).unwrap();
    for line in written.lines() {
        assert!(!line.is_empty() && !line.starts_with('#'), "{line:?}");
    }
    // Both of a pattern and a path is neither.
    let err = h
        .try_run("ignore.add", json!({ "pattern": "*.x", "path": "a.txt" }))
        .unwrap_err();
    assert!(err.contains("not both"), "{err}");
    let err = h.try_run("ignore.add", json!({})).unwrap_err();
    assert!(err.contains("pattern"), "{err}");
}
