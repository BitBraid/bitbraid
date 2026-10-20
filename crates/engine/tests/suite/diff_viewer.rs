//! What the diff viewer asks of the commands: a commit compared with the
//! working tree as git compares them, the bound over which a file is
//! listed and not compared, and a file's bytes with a limit of the
//! caller's. The comparisons themselves are held to
//! git in `crates/git/tests/suite/diff_guard.rs`; this is the way through the
//! commands, and what they do with hostile parameters.

use std::collections::BTreeMap;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
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

fn letters(files: &Value) -> BTreeMap<String, char> {
    files
        .as_array()
        .unwrap()
        .iter()
        .map(|file| {
            let letter = match file["kind"].as_str().unwrap() {
                "added" => 'A',
                "deleted" => 'D',
                "modified" => 'M',
                other => panic!("{other}"),
            };
            (file["path"].as_str().unwrap().to_owned(), letter)
        })
        .collect()
}

fn git_letters(git: &Git, args: &[&str]) -> BTreeMap<String, char> {
    let mut all = vec!["diff", "--no-renames", "--name-status"];
    all.extend_from_slice(args);
    git.lines(all)
        .unwrap()
        .into_iter()
        .map(|line| {
            let (letter, path) = line.split_once('\t').unwrap();
            (path.to_owned(), letter.chars().next().unwrap())
        })
        .collect()
}

#[test]
fn a_commit_against_the_worktree_lists_every_file_that_differs() {
    let mut h = Harness::new("linear");
    // The worktree is changed in one file only; an old commit differs
    // from it in more than that.
    h.scratch.write("b.txt", "bb\nand a line\n").unwrap();
    h.scratch.write("fresh.txt", "not tracked\n").unwrap();
    for commit in h.git().lines(["rev-list", "HEAD"]).unwrap() {
        let ours = letters(&h.run("repo.diff", json!({ "old": commit, "new": "worktree" })));
        let mut expected = git_letters(&h.git(), &[commit.as_str()]);
        expected.insert("fresh.txt".to_owned(), 'A');
        assert_eq!(ours, expected, "{commit}");
        let staged = letters(&h.run("repo.diff", json!({ "old": commit, "new": "index" })));
        assert_eq!(
            staged,
            git_letters(&h.git(), &["--cached", commit.as_str()]),
            "{commit} against the index"
        );
    }
    // The first commit knows neither a.txt nor b.txt: both are new to it.
    let root = h.git().lines(["rev-list", "HEAD"]).unwrap().pop().unwrap();
    let ours = letters(&h.run("repo.diff", json!({ "old": root, "new": "worktree" })));
    assert_eq!(ours.get("a.txt"), Some(&'A'));
    assert_eq!(ours.get("b.txt"), Some(&'A'));
    assert_eq!(ours.len(), 3);
    // Without sides it is HEAD against the worktree.
    let plain = letters(&h.run("repo.diff", json!({})));
    assert_eq!(
        plain,
        BTreeMap::from([("b.txt".to_owned(), 'M'), ("fresh.txt".to_owned(), 'A')])
    );
}

#[test]
fn a_file_over_the_bound_is_listed_with_its_sizes_and_not_compared() {
    let mut h = Harness::new("linear");
    let big: String = (0..5000).map(|n| format!("row {n}\n")).collect();
    h.scratch.write("big.txt", &big).unwrap();
    h.git().run(["add", "-A"]).unwrap();
    h.git().run(["commit", "-q", "-m", "big"]).unwrap();
    h.scratch.write("big.txt", &format!("{big}more\n")).unwrap();
    h.scratch.write("b.txt", "bb\nmore\n").unwrap();
    let size: u64 = h
        .git()
        .run(["cat-file", "-s", "HEAD:big.txt"])
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    for (old, new) in [("HEAD", "worktree"), ("HEAD~1", "HEAD")] {
        let files = h.run(
            "repo.diff",
            json!({ "old": old, "new": new, "max_bytes": 1000 }),
        );
        let large = files
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "big.txt")
            .unwrap();
        assert_eq!(large["too_large"], true, "{old}..{new}");
        assert_eq!(large["hunks"], json!([]));
        let side = if new == "HEAD" {
            "new_size"
        } else {
            "old_size"
        };
        assert_eq!(large[side], size, "{old}..{new}");
        let whole = h.run("repo.diff", json!({ "old": old, "new": new }));
        let large = whole
            .as_array()
            .unwrap()
            .iter()
            .find(|file| file["path"] == "big.txt")
            .unwrap();
        assert_eq!(large["too_large"], false);
        assert!(!large["hunks"].as_array().unwrap().is_empty());
    }
    // The small file beside it is compared either way.
    let files = h.run("repo.diff", json!({ "max_bytes": 1000 }));
    let small = files
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == "b.txt")
        .unwrap();
    assert_eq!(
        (&small["too_large"], &small["additions"]),
        (&json!(false), &json!(1))
    );
}

#[test]
fn a_files_bytes_are_refused_over_the_callers_limit() {
    use std::fmt::Write as _;
    let mut h = Harness::new("linear");
    let mut text = String::new();
    for n in 0..300 {
        writeln!(text, "line {n}").unwrap();
    }
    h.scratch.write("a.txt", &text).unwrap();
    let committed: usize = h
        .git()
        .run(["cat-file", "-s", "HEAD:a.txt"])
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    for side in ["worktree", "HEAD"] {
        let size = if side == "worktree" {
            text.len()
        } else {
            committed
        };
        let within = h.run(
            "file.bytes",
            json!({ "path": "a.txt", "side": side, "max": size }),
        );
        assert_eq!(within["size"], size, "{side}");
        let err = h
            .try_run(
                "file.bytes",
                json!({ "path": "a.txt", "side": side, "max": size - 1 }),
            )
            .unwrap_err();
        assert!(err.contains("too large"), "{side}: {err}");
    }
    // No limit of the caller's raises the one there is.
    let all = h.run("file.bytes", json!({ "path": "a.txt", "max": u64::MAX }));
    assert_eq!(all["size"], text.len());
}

#[test]
fn hostile_parameters_read_nothing_from_outside_and_change_nothing() {
    let mut h = Harness::new("linear");
    let outside = h.scratch.path().with_extension("secret");
    std::fs::write(&outside, "not the repository's to show\n").unwrap();
    let name = outside.file_name().unwrap().to_str().unwrap().to_owned();
    let head = h.git().rev_parse("HEAD").unwrap();
    // A path that leads out of the worktree is refused, not read as an
    // added file.
    for path in [
        format!("../{name}"),
        format!("x/../../{name}"),
        outside.to_str().unwrap().to_owned(),
    ] {
        for (old, new) in [
            ("HEAD", "worktree"),
            ("index", "worktree"),
            ("HEAD~1", "index"),
        ] {
            let result = h.try_run(
                "repo.diff",
                json!({ "old": old, "new": new, "paths": [path] }),
            );
            assert!(result.is_err(), "{path} {old}..{new}: {result:?}");
        }
        assert!(
            h.try_run("file.bytes", json!({ "path": path })).is_err(),
            "{path}"
        );
    }
    for hostile in [
        Value::Null,
        json!(""),
        json!("--all"),
        json!("--output=/tmp/bb-injected-diff"),
        json!("a\u{0}b"),
        json!("x".repeat(70_000)),
        json!(-1),
        json!(1e308),
        json!(true),
        json!([]),
        json!({}),
        json!(["--force", "", null, 7]),
    ] {
        for params in [
            json!({ "old": hostile, "new": "worktree" }),
            json!({ "old": "HEAD~2", "new": hostile }),
            json!({ "old": "HEAD~2", "new": "worktree", "max_bytes": hostile }),
            json!({ "old": "HEAD~2", "new": "worktree", "paths": hostile }),
            json!({ "old": "HEAD~2", "new": "index", "paths": [hostile] }),
            json!({ "old": "HEAD~2", "new": "worktree", "context": hostile }),
        ] {
            let _ = h.try_run("repo.diff", params);
        }
        let _ = h.try_run("file.bytes", json!({ "path": "a.txt", "max": hostile }));
    }
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");
    assert!(!std::path::Path::new("/tmp/bb-injected-diff").exists());
    let _ = std::fs::remove_file(&outside);
}
