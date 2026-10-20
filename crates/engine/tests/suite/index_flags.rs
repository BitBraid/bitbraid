//! Index flags against git on twin scratch copies: the list says what
//! `git ls-files -v` and `--debug` say, and a flag set or cleared here
//! leaves the index as `git update-index` and `git add -N` leave it.

use std::{collections::BTreeMap, path::PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

fn copy() -> Scratch {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    scratch
}

impl Harness {
    fn new() -> Self {
        let scratch = copy();
        let twin = copy();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            twin,
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
    fn refused(&mut self, params: Value, because: &str) {
        let err = self
            .try_run("index.set_flag", params.clone())
            .expect_err(&format!("{params} should be refused"));
        assert!(err.contains(because), "{params}: {err}");
    }
    fn both(&self, f: impl Fn(&Scratch)) {
        f(&self.scratch);
        f(&self.twin);
    }
    fn file(&self, rel: &str) -> PathBuf {
        self.scratch.path().join(rel)
    }
    /// The flagged entries as the command lists them: path to (intent to
    /// add, skip worktree, assume unchanged).
    fn listed(&mut self) -> BTreeMap<String, (bool, bool, bool)> {
        let out = self.run("index.flags", json!({}));
        let entries = out["entries"].as_array().unwrap();
        let map: BTreeMap<String, (bool, bool, bool)> = entries
            .iter()
            .map(|entry| {
                (
                    entry["path"].as_str().unwrap().to_owned(),
                    (
                        entry["intent_to_add"].as_bool().unwrap(),
                        entry["skip_worktree"].as_bool().unwrap(),
                        entry["assume_unchanged"].as_bool().unwrap(),
                    ),
                )
            })
            .collect();
        assert_eq!(map.len(), entries.len(), "each path once");
        // The counts are of what is listed.
        let count =
            |pick: fn(&(bool, bool, bool)) -> bool| map.values().filter(|f| pick(f)).count();
        assert_eq!(out["counts"]["intent_to_add"], count(|f| f.0));
        assert_eq!(out["counts"]["skip_worktree"], count(|f| f.1));
        assert_eq!(out["counts"]["assume_unchanged"], count(|f| f.2));
        assert_eq!(out["truncated"], false);
        map
    }
}

/// What git says about every entry, as text that two copies can be
/// compared by: the letter of `ls-files -v` and the flags of `--debug`.
fn flags_by_git(git: &Git) -> Vec<String> {
    let letters = git.run(["ls-files", "-v", "-z"]).unwrap();
    let debug = git.run(["ls-files", "--debug", "-z"]).unwrap();
    let mut out: Vec<String> = letters
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect();
    out.extend(
        debug
            .split('\n')
            .filter_map(|line| line.split_once("flags: "))
            .map(|(_, flags)| format!("flags {flags}")),
    );
    out
}

/// The flagged entries as git has them, read by the test: the letter of
/// `ls-files -v` is lower case for assume-unchanged and an S for
/// skip-worktree, and `--debug` shows the bit of intent-to-add.
fn flagged_by_git(git: &Git) -> BTreeMap<String, (bool, bool, bool)> {
    const INTENT_TO_ADD: u32 = 1 << 29;
    let letters = git.run(["ls-files", "-v", "-z"]).unwrap();
    let debug = git.run(["ls-files", "--debug", "-z"]).unwrap();
    // `--debug -z`: the path ends in a NUL, the lines about it follow.
    let bits: Vec<u32> = debug
        .split('\n')
        .filter_map(|line| line.split_once("flags: "))
        .map(|(_, hex)| u32::from_str_radix(hex.trim(), 16).unwrap())
        .collect();
    let entries: Vec<&str> = letters.split('\0').filter(|e| !e.is_empty()).collect();
    assert_eq!(entries.len(), bits.len(), "one set of flags an entry");
    entries
        .iter()
        .zip(bits)
        .filter_map(|(entry, bits)| {
            let (letter, path) = entry.split_once(' ').unwrap();
            let letter = letter.chars().next().unwrap();
            let flags = (
                bits & INTENT_TO_ADD != 0,
                letter.eq_ignore_ascii_case(&'s'),
                letter.is_ascii_lowercase(),
            );
            (flags != (false, false, false)).then(|| (path.to_owned(), flags))
        })
        .collect()
}

#[test]
fn the_list_is_what_git_says_of_each_entry() {
    let mut h = Harness::new();
    let git = h.git();
    assert!(h.listed().is_empty());
    assert_eq!(h.listed(), flagged_by_git(&git));

    // One of each, one with two, set with git itself.
    h.scratch.write("new.txt", "to be added\n").unwrap();
    h.scratch
        .write("dir/also new.txt", "to be added\n")
        .unwrap();
    git.run(["add", "-N", "new.txt", "dir/also new.txt"])
        .unwrap();
    git.run(["update-index", "--assume-unchanged", "a.txt", "README.md"])
        .unwrap();
    git.run(["update-index", "--skip-worktree", "b.txt", "README.md"])
        .unwrap();
    let listed = h.listed();
    assert_eq!(listed, flagged_by_git(&git));
    assert_eq!(
        listed,
        BTreeMap::from([
            ("README.md".to_owned(), (false, true, true)),
            ("a.txt".to_owned(), (false, false, true)),
            ("b.txt".to_owned(), (false, true, false)),
            ("dir/also new.txt".to_owned(), (true, false, false)),
            ("new.txt".to_owned(), (true, false, false)),
        ])
    );
    // Intent-to-add as git's status has it: new, with nothing staged.
    let status = git.run(["status", "--porcelain=v2"]).unwrap();
    assert!(
        status
            .lines()
            .any(|line| line.starts_with("1 .A ") && line.ends_with(" new.txt")),
        "{status}"
    );

    // Cleared with git, they are gone from the list at once.
    git.run([
        "update-index",
        "--no-assume-unchanged",
        "a.txt",
        "README.md",
    ])
    .unwrap();
    git.run(["update-index", "--no-skip-worktree", "b.txt"])
        .unwrap();
    git.run(["reset", "-q", "--", "new.txt"]).unwrap();
    let listed = h.listed();
    assert_eq!(listed, flagged_by_git(&git));
    assert_eq!(
        listed.keys().collect::<Vec<_>>(),
        ["README.md", "dir/also new.txt"]
    );

    // The versions of a file in conflict are not flagged entries.
    git.run(["reset", "-q", "--hard"]).unwrap();
    git.run(["update-index", "--no-skip-worktree", "README.md"])
        .unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "theirs\n").unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "ours\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
    assert!(git.run(["merge", "topic"]).is_err());
    assert!(h.listed().is_empty());
}

#[test]
fn flags_are_set_and_cleared_as_git_sets_them() {
    let mut h = Harness::new();
    // Names that need care: a blank, a leading dash, a line break, and
    // characters a pattern would be made of. Windows takes neither a line
    // break nor `*` in a name: there a name that is not ASCII, and `[a]`,
    // which a pattern takes for `a`, stand in for them.
    let (broken, starred) = if cfg!(windows) {
        ("lïne-bréak.txt", "st[a]r.txt")
    } else {
        ("line\nbreak.txt", "st*r.txt")
    };
    let odd = ["sp ace.txt", "-dash.txt", broken, starred];
    h.both(|copy| {
        for name in odd {
            copy.write(name, "tracked\n").unwrap();
        }
        copy.write("star.txt", "tracked too\n").unwrap();
        copy.git().run(["add", "-A"]).unwrap();
        copy.git().run(["commit", "-q", "-m", "odd names"]).unwrap();
    });
    let same = |h: &Harness, what: &str| {
        assert_eq!(
            flags_by_git(&h.git()),
            flags_by_git(&h.twin.git()),
            "{what}"
        );
        assert_eq!(
            h.git().run(["status", "--porcelain=v2"]).unwrap(),
            h.twin.git().run(["status", "--porcelain=v2"]).unwrap(),
            "{what}: status"
        );
    };

    for (flag, option) in [
        ("skip_worktree", "skip-worktree"),
        ("assume_unchanged", "assume-unchanged"),
    ] {
        let mut paths = vec!["a.txt"];
        paths.extend(odd);
        let out = h.run(
            "index.set_flag",
            json!({ "paths": paths, "flag": flag, "on": true }),
        );
        assert_eq!(out, json!({ "flag": flag, "on": true, "paths": paths }));
        let mut args = vec![
            "update-index".to_owned(),
            format!("--{option}"),
            "--".to_owned(),
        ];
        args.extend(paths.iter().map(|path| (*path).to_owned()));
        h.twin.git().run(&args).unwrap();
        same(&h, flag);
        let listed = h.listed();
        assert_eq!(listed, flagged_by_git(&h.git()));
        assert_eq!(listed.len(), 5);
        assert!(!listed.contains_key("star.txt"), "a name is not a pattern");
        assert!(listed.contains_key(broken));

        // What the flag is for: a change to the file is not seen.
        h.both(|copy| {
            copy.write("a.txt", "changed, and git is not to care\n")
                .unwrap()
        });
        same(&h, "a flagged file was changed");
        assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");

        // Off for some, then for the rest; the change shows again.
        h.run(
            "index.set_flag",
            json!({ "paths": ["a.txt", starred], "flag": flag, "on": false }),
        );
        h.twin
            .git()
            .run([
                "update-index",
                format!("--no-{option}").as_str(),
                "--",
                "a.txt",
                starred,
            ])
            .unwrap();
        same(&h, "cleared for two");
        assert_eq!(h.listed().len(), 3);
        assert_eq!(
            h.git().run(["status", "--porcelain"]).unwrap(),
            " M a.txt\n"
        );
        h.run(
            "index.set_flag",
            json!({ "paths": odd, "flag": flag, "on": false }),
        );
        let mut args = vec![
            "update-index".to_owned(),
            format!("--no-{option}"),
            "--".to_owned(),
        ];
        args.extend(odd.iter().map(|path| (*path).to_owned()));
        h.twin.git().run(&args).unwrap();
        same(&h, "cleared");
        assert!(h.listed().is_empty());
        // Setting what is set and clearing what is clear is no error.
        h.run(
            "index.set_flag",
            json!({ "paths": ["b.txt"], "flag": flag, "on": false }),
        );
        same(&h, "cleared again");
        h.both(|copy| {
            copy.git().run(["checkout", "-q", "--", "a.txt"]).unwrap();
        });
    }

    // Intent-to-add: files that are new, by their names and not as
    // patterns.
    let fresh = [
        "fresh.txt",
        "deep/down/fresh.txt",
        "-n.txt",
        if cfg!(windows) {
            "fr[o]sh.txt"
        } else {
            "fr*sh.txt"
        },
    ];
    h.both(|copy| {
        for name in fresh {
            copy.write(name, "new\n").unwrap();
        }
        copy.write("frosh.txt", "new, and not asked for\n").unwrap();
    });
    h.run(
        "index.set_flag",
        json!({ "paths": fresh, "flag": "intent_to_add", "on": true }),
    );
    let mut args: Vec<&str> = vec!["--literal-pathspecs", "add", "-N", "--"];
    args.extend(fresh);
    h.twin.git().run(&args).unwrap();
    same(&h, "intent to add");
    assert_eq!(
        h.git().run(["ls-files", "--stage"]).unwrap(),
        h.twin.git().run(["ls-files", "--stage"]).unwrap()
    );
    let listed = h.listed();
    assert_eq!(listed, flagged_by_git(&h.git()));
    assert_eq!(listed.len(), 4);
    assert!(listed.values().all(|flags| *flags == (true, false, false)));
    assert!(
        h.git()
            .run(["status", "--porcelain"])
            .unwrap()
            .contains("?? frosh.txt"),
        "a file that only matches as a pattern stays untracked"
    );
    // The file is part of a diff now, which is what the mark is for.
    assert!(
        h.git()
            .run(["diff", "--stat"])
            .unwrap()
            .contains("fresh.txt")
    );

    // It is not cleared as a flag: unstaging the file is what ends it.
    h.refused(
        json!({ "paths": ["fresh.txt"], "flag": "intent_to_add", "on": false }),
        "unstage it instead",
    );
    assert_eq!(h.listed().len(), 4);
    h.run("unstage.paths", json!({ "paths": fresh }));
    assert!(h.listed().is_empty());
    assert!(flagged_by_git(&h.git()).is_empty());
    assert!(h.file("fresh.txt").is_file(), "the file itself stays");
    // And staging it ends it the other way.
    h.run(
        "index.set_flag",
        json!({ "paths": ["fresh.txt"], "flag": "intent_to_add", "on": true }),
    );
    assert_eq!(h.listed().len(), 1);
    h.run("stage.paths", json!({ "paths": ["fresh.txt"] }));
    assert!(h.listed().is_empty());
    assert!(
        h.git()
            .run(["status", "--porcelain"])
            .unwrap()
            .contains("A  fresh.txt")
    );
}

#[test]
fn hostile_flag_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let git = h.git();
    // Something of every kind to be wrong about.
    h.scratch.write("untracked.txt", "not git's yet\n").unwrap();
    h.scratch.write("other.txt", "not git's yet\n").unwrap();
    h.scratch
        .write("folder/inside.txt", "in a folder\n")
        .unwrap();
    h.scratch.write("ignored.log", "ignored\n").unwrap();
    std::fs::create_dir_all(h.file(".git/info")).unwrap();
    std::fs::write(h.file(".git/info/exclude"), "*.log\n").unwrap();
    git.run(["update-index", "--assume-unchanged", "b.txt"])
        .unwrap();
    let before = (
        flags_by_git(&git),
        git.run(["status", "--porcelain=v2"]).unwrap(),
    );
    let long = "f".repeat(5000);
    let outside = h.scratch.path().parent().unwrap().join("outside.txt");
    std::fs::write(&outside, "not in the repository\n").unwrap();

    // What is no path of the worktree, for any flag either way.
    for path in [
        "",
        "/etc/passwd",
        "../outside.txt",
        "folder/../../outside.txt",
        outside.to_str().unwrap(),
        "no-such-file.txt",
        "nul\u{0}",
        ".git/config",
        ".git/index",
        "--force",
        "--chmod=+x",
        "-N",
        long.as_str(),
    ] {
        // Beside a path that is right for the flag, which must not be
        // flagged either.
        for (flag, fine) in [
            ("intent_to_add", "untracked.txt"),
            ("skip_worktree", "a.txt"),
            ("assume_unchanged", "a.txt"),
        ] {
            for on in [true, false] {
                assert!(
                    h.try_run(
                        "index.set_flag",
                        json!({ "paths": [fine, path], "flag": flag, "on": on })
                    )
                    .is_err(),
                    "{flag} {on} took the path {:?}",
                    path.get(..path.len().min(40))
                );
            }
        }
    }
    // The right kind of path for the wrong flag.
    for flag in ["skip_worktree", "assume_unchanged"] {
        for on in [true, false] {
            h.refused(
                json!({ "paths": ["a.txt", "untracked.txt"], "flag": flag, "on": on }),
                "untracked.txt is not tracked",
            );
            h.refused(
                json!({ "paths": ["folder"], "flag": flag, "on": on }),
                "is not tracked",
            );
        }
    }
    h.refused(
        json!({ "paths": ["untracked.txt", "a.txt"], "flag": "intent_to_add", "on": true }),
        "a.txt is tracked already",
    );
    h.refused(
        json!({ "paths": ["untracked.txt", "folder"], "flag": "intent_to_add", "on": true }),
        "folder is a folder",
    );
    // Git would add the first and then complain about the second.
    h.refused(
        json!({ "paths": ["untracked.txt", "ignored.log"], "flag": "intent_to_add", "on": true }),
        "ignored.log is ignored",
    );
    h.refused(
        json!({ "paths": ["untracked.txt"], "flag": "intent_to_add", "on": false }),
        "unstage it instead",
    );
    // Too many at once, none at all, and what is no request.
    let many: Vec<&str> = std::iter::repeat_n("a.txt", 10_001).collect();
    h.refused(
        json!({ "paths": many, "flag": "skip_worktree", "on": true }),
        "more than one change takes",
    );
    h.refused(
        json!({ "paths": [], "flag": "skip_worktree", "on": true }),
        "no paths",
    );
    for params in [
        json!({}),
        json!({ "paths": ["a.txt"] }),
        json!({ "paths": ["a.txt"], "flag": "skip_worktree" }),
        json!({ "paths": ["a.txt"], "on": true }),
        json!({ "paths": "a.txt", "flag": "skip_worktree", "on": true }),
        json!({ "paths": [3], "flag": "skip_worktree", "on": true }),
        json!({ "paths": ["a.txt"], "flag": "", "on": true }),
        json!({ "paths": ["a.txt"], "flag": "SkipWorktree", "on": true }),
        json!({ "paths": ["a.txt"], "flag": "--skip-worktree", "on": true }),
        json!({ "paths": ["a.txt"], "flag": "executable", "on": true }),
        json!({ "paths": ["a.txt"], "flag": 3, "on": true }),
        json!({ "paths": ["a.txt"], "flag": "skip_worktree", "on": "yes" }),
        json!({ "paths": ["a.txt"], "flag": "skip_worktree", "on": 1 }),
    ] {
        assert!(
            h.try_run("index.set_flag", params.clone()).is_err(),
            "{params}"
        );
    }
    assert_eq!(
        (
            flags_by_git(&git),
            git.run(["status", "--porcelain=v2"]).unwrap()
        ),
        before
    );
    assert_eq!(
        h.listed(),
        BTreeMap::from([("b.txt".to_owned(), (false, false, true))])
    );
    let _ = std::fs::remove_file(&outside);

    // A file in conflict carries no flag and takes none.
    git.run(["update-index", "--no-assume-unchanged", "b.txt"])
        .unwrap();
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    h.scratch.write("a.txt", "theirs\n").unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    h.scratch.write("a.txt", "ours\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
    assert!(git.run(["merge", "topic"]).is_err());
    let unmerged = git.run(["ls-files", "--stage"]).unwrap();
    for (flag, fine) in [
        ("intent_to_add", "untracked.txt"),
        ("skip_worktree", "b.txt"),
        ("assume_unchanged", "b.txt"),
    ] {
        h.refused(
            json!({ "paths": [fine, "a.txt"], "flag": flag, "on": true }),
            "a.txt is in conflict",
        );
    }
    assert_eq!(git.run(["ls-files", "--stage"]).unwrap(), unmerged);
    assert!(h.listed().is_empty());
}

#[test]
fn an_index_that_is_not_as_expected_is_an_answer_or_a_refusal() {
    let mut h = Harness::new();
    let git = h.git();
    // Written without its checksum, so that a changed byte is read as it
    // is and not caught by the checksum first.
    git.run(["config", "index.skipHash", "true"]).unwrap();
    git.run(["update-index", "--skip-worktree", "b.txt"])
        .unwrap();
    // Opened again, for the setting to be known here as well.
    h.run("repo.open", json!({ "path": h.scratch.path() }));
    let index = h.file(".git/index");
    let good = std::fs::read(&index).unwrap();
    assert_eq!(h.listed().len(), 1);

    let lie = |at: usize, bytes: &[u8]| {
        let mut bad = good.clone();
        bad[at..at + bytes.len()].copy_from_slice(bytes);
        bad
    };
    let noise: Vec<u8> = (0u32..good.len() as u32)
        .map(|n| (n.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    // HEAD, the objects and the references are left alone: git has to go
    // on taking this for a repository.
    for (what, bytes) in [
        ("empty", Vec::new()),
        ("only a signature", b"DIRC".to_vec()),
        ("cut short", good[..good.len() / 2].to_vec()),
        ("cut inside the header", good[..10].to_vec()),
        ("noise", noise),
        ("another signature", lie(0, b"NOPE")),
        ("a version there is none of", lie(4, &[0, 0, 0, 99])),
        ("more entries than there is room for", lie(8, &[0xff; 4])),
        ("one entry more than there is", {
            let count = u32::from_be_bytes(good[8..12].try_into().unwrap());
            lie(8, &(count + 1).to_be_bytes())
        }),
        ("no entries", lie(8, &[0; 4])),
    ] {
        std::fs::write(&index, &bytes).unwrap();
        // An answer or a refusal, never a crash; and nothing is written
        // where the index cannot be read.
        let listed = h.try_run("index.flags", json!({}));
        if let Ok(listed) = &listed {
            assert!(listed["entries"].is_array(), "{what}: {listed}");
        }
        let _ = h.try_run(
            "index.set_flag",
            json!({ "paths": ["a.txt"], "flag": "assume_unchanged", "on": true }),
        );
        h.scratch.write("fresh.txt", "new\n").unwrap();
        let _ = h.try_run(
            "index.set_flag",
            json!({ "paths": ["fresh.txt"], "flag": "intent_to_add", "on": true }),
        );
        std::fs::write(&index, &good).unwrap();
    }
    assert_eq!(h.listed().len(), 1, "back as it was");

    // No index at all: nothing is flagged, and nothing is tracked to flag.
    std::fs::remove_file(&index).unwrap();
    assert!(h.listed().is_empty());
    h.refused(
        json!({ "paths": ["a.txt"], "flag": "skip_worktree", "on": true }),
        "a.txt is not tracked",
    );
    assert!(!index.exists(), "asking did not make one");
    // A directory where the index should be.
    std::fs::create_dir(&index).unwrap();
    let _ = h.try_run("index.flags", json!({}));
    let _ = h.try_run(
        "index.set_flag",
        json!({ "paths": ["a.txt"], "flag": "skip_worktree", "on": true }),
    );
    std::fs::remove_dir(&index).unwrap();
    std::fs::write(&index, &good).unwrap();
    assert_eq!(h.listed().len(), 1);
}

#[test]
fn very_many_flagged_entries_are_counted_and_not_all_listed() {
    let mut h = Harness::new();
    let git = h.git();
    // More files than the list takes, written and added in one go.
    let dir = h.file("many");
    std::fs::create_dir_all(&dir).unwrap();
    for n in 0..5010 {
        std::fs::write(dir.join(format!("f{n:05}.txt")), "x\n").unwrap();
    }
    git.run(["add", "many"]).unwrap();
    let paths: Vec<String> = (0..5010).map(|n| format!("many/f{n:05}.txt")).collect();
    h.run(
        "index.set_flag",
        json!({ "paths": paths, "flag": "skip_worktree", "on": true }),
    );
    let out = h.run("index.flags", json!({}));
    assert_eq!(
        out["counts"],
        json!({ "intent_to_add": 0, "skip_worktree": 5010, "assume_unchanged": 0 })
    );
    assert_eq!(out["entries"].as_array().unwrap().len(), 5000);
    assert_eq!(out["truncated"], true);
    assert_eq!(
        git.lines(["ls-files", "-v"])
            .unwrap()
            .iter()
            .filter(|line| line.starts_with("S "))
            .count(),
        5010
    );
}
