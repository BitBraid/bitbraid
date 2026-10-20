//! Mutations against git: staging, discarding, committing, ignoring, undo
//! and the file commands, held to git plumbing on scratch copies.

use std::collections::BTreeMap;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

fn index_of(git: &Git) -> BTreeMap<String, (String, String)> {
    git.lines(["ls-files", "-s"])
        .unwrap()
        .into_iter()
        .map(|line| {
            let (meta, path) = line.split_once('\t').unwrap();
            let parts: Vec<&str> = meta.split(' ').collect();
            (path.to_owned(), (parts[0].to_owned(), parts[1].to_owned()))
        })
        .collect()
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
    fn porcelain(&self) -> Vec<String> {
        self.git()
            .lines(["status", "--porcelain=v2", "--untracked-files=all"])
            .unwrap()
    }
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.scratch.path().join(rel)).unwrap()
    }
}

#[test]
fn stage_and_unstage_whole_paths_match_git() {
    let mut h = Harness::new("branches-merges");
    h.scratch.write("a.txt", "a1\nchanged\n").unwrap();
    h.scratch.write("new.txt", "new\n").unwrap();
    h.scratch.remove("base.txt").unwrap();

    h.run(
        "stage.paths",
        json!({ "paths": ["a.txt", "new.txt", "base.txt"] }),
    );
    let ours = index_of(&h.git());
    let twin = Scratch::of("branches-merges").unwrap();
    twin.write("a.txt", "a1\nchanged\n").unwrap();
    twin.write("new.txt", "new\n").unwrap();
    twin.remove("base.txt").unwrap();
    twin.git().run(["add", "-A"]).unwrap();
    assert_eq!(ours, index_of(&twin.git()), "index after staging");
    assert!(
        h.porcelain()
            .iter()
            .all(|l| l.starts_with("1 A.") || l.starts_with("1 M.") || l.starts_with("1 D.")),
        "{:?}",
        h.porcelain()
    );

    h.run(
        "unstage.paths",
        json!({ "paths": ["a.txt", "new.txt", "base.txt"] }),
    );
    let status = h.porcelain();
    assert!(
        status
            .iter()
            .any(|l| l.starts_with("1 .M") && l.ends_with("a.txt")),
        "{status:?}"
    );
    assert!(
        status.iter().any(|l| l.starts_with("? new.txt")),
        "{status:?}"
    );
    assert!(
        status
            .iter()
            .any(|l| l.starts_with("1 .D") && l.ends_with("base.txt")),
        "{status:?}"
    );
}

#[test]
fn stage_lines_then_unstage_lines_round_trip() {
    let mut h = Harness::new("linear");
    h.scratch.write("a.txt", "0\n2\n3\n").unwrap();
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["a.txt"] }),
    );
    let hunks = diff[0]["hunks"].as_array().unwrap();
    assert_eq!(hunks.len(), 1);
    let lines = hunks[0]["lines"].as_array().unwrap();
    let first_added = lines.iter().position(|l| l["kind"] == "added").unwrap();
    h.run(
        "stage.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": [first_added] }] }),
    );
    let staged = h.run(
        "repo.diff",
        json!({ "old": "HEAD", "new": "index", "paths": ["a.txt"] }),
    );
    assert_eq!(staged[0]["additions"], 1);
    assert_eq!(h.git().run(["show", ":a.txt"]).unwrap(), "0\n2\n");
    assert_eq!(h.read("a.txt"), "0\n2\n3\n", "worktree untouched");

    // Unstaging picks its lines from the diff a staged view shows.
    let staged = h.run(
        "repo.diff",
        json!({ "old": "HEAD", "new": "index", "paths": ["a.txt"] }),
    );
    let lines = staged[0]["hunks"][0]["lines"].as_array().unwrap();
    let added = lines.iter().position(|l| l["kind"] == "added").unwrap();
    h.run(
        "unstage.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": [added] }] }),
    );
    assert_eq!(h.git().run(["show", ":a.txt"]).unwrap(), "2\n");
    assert!(
        h.git().run(["diff", "--cached", "--quiet"]).is_ok(),
        "nothing staged"
    );
}

/// Git for Windows checks text out with CRLF (`core.autocrlf`, on in its
/// system configuration). A line changed in such a file is one line
/// changed, as `git diff` says; staging it stores LF, as `git add` would;
/// and discarding it leaves the file with CRLF, as git would check it out.
#[test]
fn a_file_checked_out_with_crlf_is_diffed_staged_and_discarded_line_by_line() {
    let mut h = Harness::new("linear");
    let git = h.git();
    git.run(["config", "core.autocrlf", "true"]).unwrap();
    h.scratch.write("lines.txt", "one\ntwo\nthree\n").unwrap();
    git.run(["add", "--", "lines.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "lines"]).unwrap();
    let file = h.scratch.path().join("lines.txt");
    std::fs::remove_file(&file).unwrap();
    git.run(["checkout", "--", "lines.txt"]).unwrap();
    assert_eq!(h.read("lines.txt"), "one\r\ntwo\r\nthree\r\n");
    assert!(h.porcelain().is_empty(), "{:?}", h.porcelain());

    std::fs::write(&file, "ONE\r\ntwo\r\nTHREE\r\n").unwrap();
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["lines.txt"] }),
    );
    assert_eq!(diff[0]["additions"], 2, "{diff}");
    assert_eq!(diff[0]["deletions"], 2, "{diff}");
    let line = |diff: &Value, kind: &str, text: &str| -> usize {
        diff[0]["hunks"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .position(|l| l["kind"] == kind && l["text"] == text)
            .unwrap_or_else(|| panic!("no {kind} {text:?} in {diff}"))
    };

    // The first change is staged, without a CR.
    let picked = [line(&diff, "removed", "one"), line(&diff, "added", "ONE")];
    h.run(
        "stage.hunks",
        json!({ "path": "lines.txt", "selection": [{ "hunk": 0, "lines": picked }] }),
    );
    assert_eq!(
        git.run(["show", ":lines.txt"]).unwrap(),
        "ONE\ntwo\nthree\n"
    );
    assert_eq!(h.read("lines.txt"), "ONE\r\ntwo\r\nTHREE\r\n");

    // The other is discarded: the file is as git would check it out.
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["lines.txt"] }),
    );
    let picked = [
        line(&diff, "removed", "three"),
        line(&diff, "added", "THREE"),
    ];
    h.run(
        "discard.hunks",
        json!({ "path": "lines.txt", "selection": [{ "hunk": 0, "lines": picked }] }),
    );
    assert_eq!(h.read("lines.txt"), "ONE\r\ntwo\r\nthree\r\n");
    assert!(
        git.run(["diff", "--quiet", "--", "lines.txt"]).is_ok(),
        "nothing left unstaged"
    );
}

/// A filter driver (git-lfs) is a program of the repository's: it is not
/// run to show a diff or to stage part of a file. Here the driver's clean
/// step would make any file `cleaned`; the diff and the index have the
/// file's own lines.
#[test]
fn a_filter_driver_is_not_run_to_diff_or_stage_lines() {
    let mut h = Harness::new("linear");
    let git = h.git();
    h.scratch.write("lines.txt", "one\ntwo\n").unwrap();
    git.run(["add", "--", "lines.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "lines"]).unwrap();
    git.run(["config", "filter.shout.clean", "echo cleaned"])
        .unwrap();
    h.scratch
        .write(".gitattributes", "lines.txt filter=shout\n")
        .unwrap();
    h.scratch.write("lines.txt", "one\nTWO\n").unwrap();

    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["lines.txt"] }),
    );
    let lines: Vec<(String, String)> = diff[0]["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|l| l["kind"] != "context")
        .map(|l| (l["kind"].to_string(), l["text"].to_string()))
        .collect();
    assert_eq!(
        lines,
        [
            (r#""removed""#.to_owned(), r#""two""#.to_owned()),
            (r#""added""#.to_owned(), r#""TWO""#.to_owned()),
        ],
        "{diff}"
    );
    let picked: Vec<usize> = diff[0]["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, l)| l["kind"] != "context")
        .map(|(at, _)| at)
        .collect();
    h.run(
        "stage.hunks",
        json!({ "path": "lines.txt", "selection": [{ "hunk": 0, "lines": picked }] }),
    );
    assert_eq!(git.run(["show", ":lines.txt"]).unwrap(), "one\nTWO\n");
}

/// Where files carry no executable mark (Windows), staging part of an
/// executable script keeps it executable, as git keeps it there.
#[test]
fn staging_lines_of_an_executable_keeps_it_executable() {
    let mut h = Harness::new("linear");
    let git = h.git();
    h.scratch.write("run.sh", "#!/bin/sh\necho one\n").unwrap();
    git.run(["add", "--", "run.sh"]).unwrap();
    git.run(["update-index", "--chmod=+x", "--", "run.sh"])
        .unwrap();
    git.run(["commit", "-q", "-m", "script"]).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let file = h.scratch.path().join("run.sh");
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    h.scratch
        .write("run.sh", "#!/bin/sh\necho one\necho two\n")
        .unwrap();
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["run.sh"] }),
    );
    let added = diff[0]["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .position(|l| l["kind"] == "added")
        .unwrap();
    h.run(
        "stage.hunks",
        json!({ "path": "run.sh", "selection": [{ "hunk": 0, "lines": [added] }] }),
    );
    assert_eq!(index_of(&git)["run.sh"].0, "100755");
}

#[test]
fn discard_paths_and_hunks_are_undoable() {
    let mut h = Harness::new("linear");
    h.scratch.write("a.txt", "2\nextra\n").unwrap();
    h.scratch.write("junk.txt", "junk\n").unwrap();

    h.run("discard.paths", json!({ "paths": ["a.txt", "junk.txt"] }));
    assert_eq!(h.read("a.txt"), "2\n");
    assert!(!h.scratch.path().join("junk.txt").exists());
    assert!(h.porcelain().is_empty());

    let peek = h.run("undo.peek", json!({}));
    assert_eq!(peek["undo"], "Discard 2 files");
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), "2\nextra\n");
    assert_eq!(h.read("junk.txt"), "junk\n");
    h.run("redo.last", json!({}));
    assert_eq!(h.read("a.txt"), "2\n");
    assert!(!h.scratch.path().join("junk.txt").exists());

    h.scratch.write("a.txt", "0\n2\n3\n").unwrap();
    // Discarding picks its lines from the diff an unstaged view shows.
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": ["a.txt"] }),
    );
    let lines = diff[0]["hunks"][0]["lines"].as_array().unwrap();
    let added_zero = lines
        .iter()
        .position(|l| l["kind"] == "added" && l["text"] == "0")
        .unwrap();
    h.run(
        "discard.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": [added_zero] }] }),
    );
    assert_eq!(h.read("a.txt"), "2\n3\n");
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), "0\n2\n3\n");
}

#[test]
fn commit_amend_undo_redo_move_the_branch_like_git() {
    let mut h = Harness::new("linear");
    let before = h.git().rev_parse("HEAD").unwrap();
    h.scratch.write("a.txt", "2\ncommitted\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["a.txt"] }));
    let out = h.run(
        "commit.create",
        json!({ "message": "add a line\n\nBody text.\n" }),
    );
    let new_head = h.git().rev_parse("HEAD").unwrap();
    assert_eq!(out["id"], new_head);
    assert_eq!(out["previous"], before);
    assert_eq!(out["branch"], "main");
    assert_eq!(
        h.git().run(["log", "-1", "--format=%B"]).unwrap().trim(),
        "add a line\n\nBody text."
    );
    assert!(h.porcelain().is_empty());

    h.scratch.write("b.txt", "bb\namended\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["b.txt"] }));
    let amended = h.run("commit.amend", json!({ "message": "add a line (amended)" }));
    let amended_head = h.git().rev_parse("HEAD").unwrap();
    assert_eq!(amended["id"], amended_head);
    assert_eq!(
        h.git().rev_parse("HEAD~1").unwrap(),
        before,
        "amend keeps the parent"
    );
    assert_eq!(h.git().rev_list_count("HEAD").unwrap(), 6);

    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), new_head);
    let status = h.porcelain();
    assert!(
        status
            .iter()
            .any(|l| l.starts_with("1 M.") && l.ends_with("b.txt")),
        "{status:?}"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), before);
    let status = h.porcelain();
    assert!(
        status
            .iter()
            .any(|l| l.starts_with("1 M.") && l.ends_with("a.txt")),
        "{status:?}"
    );
    h.run("redo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), new_head);
    h.run("redo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), amended_head);

    h.git()
        .run(["commit", "-q", "--allow-empty", "-m", "someone else"])
        .unwrap();
    assert!(
        h.try_run("undo.last", json!({})).is_err(),
        "compare-and-swap refuses"
    );
}

#[test]
fn commit_with_stage_all_commits_what_git_add_all_would() {
    let mut h = Harness::new("linear");
    let twin = Scratch::of("linear").unwrap();
    for copy in [&h.scratch, &twin] {
        // A change, a deletion, a new file in a new folder, and an ignored
        // file that must stay out.
        copy.write("a.txt", "changed\n").unwrap();
        std::fs::remove_file(copy.path().join("b.txt")).unwrap();
        copy.write("sub/new.txt", "new\n").unwrap();
        copy.write(".gitignore", "*.log\n").unwrap();
        copy.write("build.log", "noise\n").unwrap();
    }
    h.run(
        "commit.create",
        json!({ "message": "everything at once", "stage_all": true }),
    );
    let git = twin.git();
    git.run(["add", "--all"]).unwrap();
    git.run(["commit", "-q", "-m", "everything at once"])
        .unwrap();
    // Commit ids carry the date; the trees do not.
    assert_eq!(
        h.git().rev_parse("HEAD^{tree}").unwrap(),
        git.rev_parse("HEAD^{tree}").unwrap()
    );
    let short = |git: &Git| git.run(["status", "--porcelain"]).unwrap();
    assert_eq!(short(&h.git()), short(&git));
    assert!(h.scratch.path().join("build.log").exists());

    // Undo takes the commit back and leaves the work staged.
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_list_count("HEAD").unwrap(), 5);
    assert!(
        short(&h.git()).contains("A  sub/new.txt"),
        "{}",
        short(&h.git())
    );

    // Without the flag only the index is committed.
    let mut h = Harness::new("linear");
    h.scratch.write("a.txt", "not staged\n").unwrap();
    assert!(
        h.try_run("commit.create", json!({ "message": "nothing staged" }))
            .is_err()
    );
    assert!(
        h.try_run(
            "commit.create",
            json!({ "message": "wrong type", "stage_all": "yes" })
        )
        .is_err()
    );
    assert_eq!(short(&h.git()), " M a.txt\n");
}

#[test]
fn commit_rejects_empty_messages_and_respects_no_verify() {
    let mut h = Harness::new("linear");
    assert!(
        h.try_run("commit.create", json!({ "message": "   " }))
            .is_err()
    );
    let hooks = h.scratch.path().join(".git/hooks");
    std::fs::create_dir_all(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 1\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    h.scratch.write("a.txt", "2\nhooked\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["a.txt"] }));
    assert!(
        h.try_run("commit.create", json!({ "message": "blocked" }))
            .is_err()
    );
    h.run(
        "commit.create",
        json!({ "message": "allowed", "no_verify": true }),
    );
    assert_eq!(
        h.git().run(["log", "-1", "--format=%s"]).unwrap().trim(),
        "allowed"
    );
}

#[test]
fn ignore_add_and_people_and_template() {
    let mut h = Harness::new("branches-merges");
    let out = h.run("ignore.add", json!({ "pattern": "*.log" }));
    assert!(out["file"].as_str().unwrap().ends_with(".gitignore"));
    assert_eq!(h.read(".gitignore"), "*.log\n");
    h.run("ignore.add", json!({ "pattern": "*.log" }));
    assert_eq!(h.read(".gitignore"), "*.log\n", "no duplicates");
    h.run(
        "ignore.add",
        json!({ "pattern": "tmp/", "target": "local" }),
    );
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join(".git/info/exclude"))
            .unwrap()
            .lines()
            .last(),
        Some("tmp/")
    );

    let people = h.run("commit.people", json!({ "limit": 50 }));
    let names: Vec<&str> = people
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"Ada Fixture") && names.contains(&"Bob Fixture"),
        "{names:?}"
    );

    assert_eq!(h.run("commit.template", json!({}))["template"], Value::Null);
    h.scratch
        .write("tpl.txt", "Summary:\n\nDetails:\n")
        .unwrap();
    h.git()
        .run(["config", "commit.template", "tpl.txt"])
        .unwrap();
    assert_eq!(
        h.run("commit.template", json!({}))["template"],
        "Summary:\n\nDetails:\n"
    );
}

/// `repo.diff` compares lines the way its `whitespace` parameter says, for
/// worktree diffs and for diffs between commits, and rejects other values.
#[test]
fn repo_diff_honours_the_whitespace_parameter() {
    let mut h = Harness::new("linear");
    let git = h.git();
    h.scratch.write("ws.txt", "one\n  two\nthree   \n").unwrap();
    git.run(["add", "ws.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "ws"]).unwrap();
    h.scratch
        .write("ws.txt", "one\n      two\nthree\nfour\n")
        .unwrap();

    let counts = |h: &mut Harness, old: &str, new: &str, whitespace: Option<&str>| {
        let mut params = json!({ "old": old, "new": new, "paths": ["ws.txt"] });
        if let Some(whitespace) = whitespace {
            params["whitespace"] = json!(whitespace);
        }
        let files = h.run("repo.diff", params);
        let file = &files.as_array().unwrap()[0];
        (
            file["additions"].as_u64().unwrap(),
            file["deletions"].as_u64().unwrap(),
        )
    };
    assert_eq!(counts(&mut h, "HEAD", "worktree", None), (3, 2));
    assert_eq!(counts(&mut h, "HEAD", "worktree", Some("exact")), (3, 2));
    assert_eq!(
        counts(&mut h, "HEAD", "worktree", Some("ignore_trailing")),
        (2, 1)
    );
    assert_eq!(
        counts(&mut h, "HEAD", "worktree", Some("ignore_all")),
        (1, 0)
    );

    // Between two commits the tree diff takes the same parameter.
    git.run(["commit", "-q", "-am", "ws again"]).unwrap();
    assert_eq!(counts(&mut h, "HEAD~1", "HEAD", None), (3, 2));
    assert_eq!(counts(&mut h, "HEAD~1", "HEAD", Some("ignore_all")), (1, 0));

    for hostile in [json!("nonsense"), json!(7), json!({ "a": 1 })] {
        let err = h
            .try_run(
                "repo.diff",
                json!({ "old": "HEAD~1", "new": "HEAD", "whitespace": hostile }),
            )
            .unwrap_err();
        assert!(err.contains("whitespace"), "{err}");
    }
}

/// Hostile numbers: a context as large as the type allows must come back as
/// the whole file from every command that cuts a file into hunks.
#[test]
fn a_huge_context_parameter_does_not_bring_a_command_down() {
    let mut h = Harness::new("linear");
    h.scratch.write("a.txt", "changed\nand more\n").unwrap();
    for context in [u32::MAX, u32::MAX / 2 + 1] {
        let files = h.run(
            "repo.diff",
            json!({ "old": "HEAD", "new": "worktree", "paths": ["a.txt"], "context": context }),
        );
        assert_eq!(files.as_array().unwrap().len(), 1);
        let between = h.run(
            "repo.diff",
            json!({ "old": "HEAD~1", "new": "HEAD", "context": context }),
        );
        assert!(between.is_array());
    }
    // Staging by hunk under the same context stages the file's changes.
    h.run(
        "stage.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": null }], "context": u32::MAX }),
    );
    // Porcelain v2: `M.` is staged, `.M` unstaged.
    assert!(h.porcelain()[0].starts_with("1 M. "), "{:?}", h.porcelain());
    h.run(
        "unstage.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": null }], "context": u32::MAX }),
    );
    assert!(h.porcelain()[0].starts_with("1 .M "), "{:?}", h.porcelain());
    h.run(
        "discard.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0, "lines": null }], "context": u32::MAX }),
    );
    assert!(h.porcelain().is_empty());
}

/// A line picked in the viewer is the line that gets unstaged or discarded,
/// also in a hunk that both removes and adds. Read against the reversed
/// diff, the same index would be another line.
#[test]
fn lines_picked_in_a_mixed_hunk_are_the_lines_acted_on() {
    let mut h = Harness::new("linear");
    let git = h.git();
    h.scratch
        .write("m.txt", "fn main() {\n    old();\n}\n")
        .unwrap();
    git.run(["add", "m.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "m"]).unwrap();
    let edited = "fn main() {\n    new();\n    extra();\n}\n";
    h.scratch.write("m.txt", edited).unwrap();

    let line_of = |h: &mut Harness, old: &str, new: &str, kind: &str, text: &str| -> usize {
        let diff = h.run(
            "repo.diff",
            json!({ "old": old, "new": new, "paths": ["m.txt"] }),
        );
        diff[0]["hunks"][0]["lines"]
            .as_array()
            .unwrap()
            .iter()
            .position(|l| l["kind"] == kind && l["text"] == text)
            .unwrap()
    };

    // Discard the one added line `extra();`: nothing else moves.
    let extra = line_of(&mut h, "index", "worktree", "added", "    extra();");
    h.run(
        "discard.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": [extra] }] }),
    );
    assert_eq!(h.read("m.txt"), "fn main() {\n    new();\n}\n");
    h.run("undo.last", json!({}));
    assert_eq!(h.read("m.txt"), edited);

    // Discard the removal of `old();`: it comes back before what was added.
    let old_line = line_of(&mut h, "index", "worktree", "removed", "    old();");
    h.run(
        "discard.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": [old_line] }] }),
    );
    assert_eq!(
        h.read("m.txt"),
        "fn main() {\n    old();\n    new();\n    extra();\n}\n"
    );
    h.run("undo.last", json!({}));

    // Discarding every line of the hunk one by one equals discarding the hunk.
    h.run(
        "discard.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": [1, 2, 3] }] }),
    );
    assert_eq!(h.read("m.txt"), "fn main() {\n    old();\n}\n");
    h.run("undo.last", json!({}));
    assert_eq!(h.read("m.txt"), edited);

    // The same for unstaging: stage everything, then unstage one added line.
    h.run("stage.paths", json!({ "paths": ["m.txt"] }));
    let extra = line_of(&mut h, "HEAD", "index", "added", "    extra();");
    h.run(
        "unstage.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": [extra] }] }),
    );
    assert_eq!(
        git.run(["show", ":m.txt"]).unwrap(),
        "fn main() {\n    new();\n}\n"
    );
    assert_eq!(h.read("m.txt"), edited, "worktree untouched");
    // Unstage the removal: the index has both lines again.
    let old_line = line_of(&mut h, "HEAD", "index", "removed", "    old();");
    h.run(
        "unstage.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": [old_line] }] }),
    );
    assert_eq!(
        git.run(["show", ":m.txt"]).unwrap(),
        "fn main() {\n    old();\n    new();\n}\n"
    );
    // Unstaging the whole remaining hunk leaves nothing staged.
    h.run(
        "unstage.hunks",
        json!({ "path": "m.txt", "selection": [{ "hunk": 0, "lines": null }] }),
    );
    assert!(git.run(["diff", "--cached", "--quiet"]).is_ok());
}

/// `file.restore` puts the worktree file back to a commit's version like
/// `git restore --source`, leaves the index alone and can be undone.
#[test]
fn file_restore_matches_git_restore_and_is_undoable() {
    let mut h = Harness::new("linear");
    let git = h.git();
    // a.txt was "1" two commits before its current "2"; keep a local edit.
    h.scratch.write("a.txt", "local edit\n").unwrap();
    let old = git.rev_parse("HEAD~3").unwrap();
    let wanted = git.run(["show", &format!("{old}:a.txt")]).unwrap();

    let out = h.run("file.restore", json!({ "path": "a.txt", "rev": old }));
    assert_eq!(out["restored"], "a.txt");
    assert_eq!(h.read("a.txt"), wanted);
    assert_eq!(
        git.run(["show", ":a.txt"]).unwrap(),
        git.run(["show", "HEAD:a.txt"]).unwrap(),
        "the index stays"
    );
    // The twin: git's own restore from the same commit gives the same bytes.
    git.run(["restore", "--source", &old, "--worktree", "--", "a.txt"])
        .unwrap();
    assert_eq!(h.read("a.txt"), wanted);

    let peek = h.run("undo.peek", json!({}));
    assert!(
        peek["undo"]
            .as_str()
            .unwrap()
            .starts_with("Restore a.txt from "),
        "{peek}"
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), "local edit\n");
    h.run("redo.last", json!({}));
    assert_eq!(h.read("a.txt"), wanted);
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), "local edit\n");

    // A revision name works too.
    h.run("file.restore", json!({ "path": "a.txt", "rev": "HEAD" }));
    assert_eq!(h.read("a.txt"), git.run(["show", "HEAD:a.txt"]).unwrap());
    // In a commit from before the file existed, the file is absent, and so
    // it is afterwards, as with git; undo brings the local version back.
    h.scratch.write("b.txt", "mine\n").unwrap();
    let first = git.rev_parse("HEAD~4").unwrap();
    h.run("file.restore", json!({ "path": "b.txt", "rev": first }));
    assert!(!h.scratch.path().join("b.txt").exists());
    h.run("undo.last", json!({}));
    assert_eq!(h.read("b.txt"), "mine\n");
    // A path no commit and no index knows is an error that changes nothing.
    h.scratch.write("stranger.txt", "untracked\n").unwrap();
    assert!(
        h.try_run(
            "file.restore",
            json!({ "path": "stranger.txt", "rev": "HEAD" })
        )
        .is_err()
    );
    assert_eq!(h.read("stranger.txt"), "untracked\n");
}

/// Hostile parameters to the file commands: paths outside the worktree,
/// pathspec magic, revisions that are not commits, commands that do not
/// exist. Errors, never a panic and never a touched file.
fn decoded(value: &Value) -> Vec<u8> {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD
        .decode(value["base64"].as_str().unwrap())
        .unwrap()
}

#[test]
fn file_bytes_are_the_file_on_each_side() {
    let mut h = Harness::new("linear");
    let git = h.git();
    let at_head = git.run(["show", "HEAD:a.txt"]).unwrap();
    h.scratch.write("a.txt", "staged\n").unwrap();
    git.run(["add", "a.txt"]).unwrap();
    h.scratch.write("a.txt", "on disk\n").unwrap();

    for (side, want) in [
        ("HEAD", at_head.as_str()),
        ("index", "staged\n"),
        ("worktree", "on disk\n"),
        (
            "HEAD~1",
            git.run(["show", "HEAD~1:a.txt"]).unwrap().as_str(),
        ),
    ] {
        let out = h.run("file.bytes", json!({ "path": "a.txt", "side": side }));
        assert_eq!(out["exists"], true, "{side}");
        assert_eq!(decoded(&out), want.as_bytes(), "{side}");
        assert_eq!(out["size"], want.len(), "{side}");
    }
    // The worktree is the side when none is named, and the id is git's.
    let out = h.run("file.bytes", json!({ "path": "a.txt" }));
    assert_eq!(decoded(&out), b"on disk\n");
    assert_eq!(out["id"], git.run(["hash-object", "a.txt"]).unwrap().trim());

    // Bytes that are not text come back as they are.
    let raw: Vec<u8> = (0..=255u8).collect();
    std::fs::write(h.scratch.path().join("raw.bin"), &raw).unwrap();
    assert_eq!(
        decoded(&h.run("file.bytes", json!({ "path": "raw.bin" }))),
        raw
    );

    // Not there, or not a file: said so, not an error.
    for (path, side) in [
        ("nope.txt", "worktree"),
        ("nope.txt", "HEAD"),
        ("raw.bin", "HEAD"),
        ("raw.bin", "index"),
    ] {
        let out = h.run("file.bytes", json!({ "path": path, "side": side }));
        assert_eq!(out["exists"], false, "{path} at {side}");
    }
    std::fs::create_dir(h.scratch.path().join("folder")).unwrap();
    assert_eq!(
        h.run("file.bytes", json!({ "path": "folder" }))["exists"],
        false
    );

    // Too large to hand out, without reading it first.
    let big = std::fs::File::create(h.scratch.path().join("big.bin")).unwrap();
    big.set_len(33 * 1024 * 1024).unwrap();
    let err = h
        .try_run("file.bytes", json!({ "path": "big.bin" }))
        .unwrap_err();
    assert!(err.contains("too large"), "{err}");

    for params in [
        json!({}),
        json!({ "path": "" }),
        json!({ "path": "../outside" }),
        json!({ "path": "/etc/hostname" }),
        json!({ "path": "a/../../b" }),
        json!({ "path": "../outside", "side": "HEAD" }),
        json!({ "path": "a.txt", "side": "no-such-rev" }),
        json!({ "path": "a.txt", "side": "-x" }),
        json!({ "path": "a.txt", "side": 3 }),
        json!({ "path": 3 }),
    ] {
        assert!(h.try_run("file.bytes", params.clone()).is_err(), "{params}");
    }
}

#[test]
fn file_write_replaces_text_and_is_undoable() {
    let mut h = Harness::new("linear");
    let git = h.git();
    let file = h.scratch.path().join("a.txt");
    let original = std::fs::read_to_string(&file).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let opened = h.run("file.bytes", json!({ "path": "a.txt" }));

    let out = h.run(
        "file.write",
        json!({ "path": "a.txt", "text": "fixed\r\nin place\n", "base": opened["id"] }),
    );
    assert_eq!(std::fs::read(&file).unwrap(), b"fixed\r\nin place\n");
    assert_eq!(out["id"], git.run(["hash-object", "a.txt"]).unwrap().trim());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&file).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "the file keeps its mode");
    }
    // Nothing else was left beside it.
    let stray: Vec<_> = std::fs::read_dir(h.scratch.path())
        .unwrap()
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains(".tmp"))
        .collect();
    assert!(stray.is_empty(), "{stray:?}");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Edit a.txt");

    h.run("undo.last", json!({}));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    h.run("redo.last", json!({}));
    assert_eq!(std::fs::read(&file).unwrap(), b"fixed\r\nin place\n");

    // Written by somebody else since: neither a save from the stale text
    // nor undo goes over it.
    std::fs::write(&file, "theirs\n").unwrap();
    let err = h
        .try_run(
            "file.write",
            json!({ "path": "a.txt", "text": "mine\n", "base": out["id"] }),
        )
        .unwrap_err();
    assert!(err.contains("changed on disk"), "{err}");
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("changed since"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");

    // The same text again is no change and nothing to undo twice.
    h.run("file.write", json!({ "path": "a.txt", "text": "theirs\n" }));
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Edit a.txt");
}

#[test]
fn file_write_only_touches_plain_files_inside_the_worktree() {
    let mut h = Harness::new("linear");
    let root = h.scratch.path().to_path_buf();
    // Somewhere outside the worktree, and links into it.
    let outside = root.with_extension("outside");
    let _ = std::fs::remove_dir_all(&outside);
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.txt"), "untouched\n").unwrap();
    std::fs::create_dir(root.join("folder")).unwrap();
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, root.join("way-out")).unwrap();
        std::os::unix::fs::symlink(outside.join("secret.txt"), root.join("link.txt")).unwrap();
    }
    let long = "x".repeat(5000);
    let huge = "y".repeat(16 * 1024 * 1024 + 1);
    for (path, text) in [
        ("way-out/secret.txt", "pwned"),
        ("link.txt", "pwned"),
        ("folder", "pwned"),
        ("not-there.txt", "new files are not made here"),
        ("../linear.outside/secret.txt", "pwned"),
        ("/tmp/bitbraid-should-not-exist", "pwned"),
        ("", "pwned"),
        (long.as_str(), "pwned"),
        ("a.txt", huge.as_str()),
    ] {
        assert!(
            h.try_run("file.write", json!({ "path": path, "text": text }))
                .is_err(),
            "wrote to {path:?}"
        );
    }
    for params in [
        json!({ "path": "a.txt" }),
        json!({ "text": "x" }),
        json!({ "path": "a.txt", "text": 3 }),
        json!({ "path": "a.txt", "text": "x", "base": "not an id" }),
        json!({ "path": "a.txt", "text": "x", "base": "0000000000000000000000000000000000000000" }),
    ] {
        assert!(h.try_run("file.write", params.clone()).is_err(), "{params}");
    }
    assert_eq!(
        std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
        "untouched\n"
    );
    assert!(!root.join("not-there.txt").exists());
    assert!(!std::path::Path::new("/tmp/bitbraid-should-not-exist").exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn file_commands_refuse_hostile_parameters() {
    let mut h = Harness::new("linear");
    for path in ["", "/etc/passwd", "../outside", "a/../../outside", "x/.."] {
        for command in ["file.open", "file.reveal", "file.restore"] {
            let err = h
                .try_run(
                    command,
                    json!({ "path": path, "rev": "HEAD", "command": "true" }),
                )
                .unwrap_err();
            assert!(err.contains("path"), "{command} {path:?}: {err}");
        }
    }
    // Pathspec magic is a file name here, and there is no such file.
    assert!(
        h.try_run(
            "file.restore",
            json!({ "path": ":(top)a.txt", "rev": "HEAD" })
        )
        .is_err()
    );
    assert_eq!(h.read("a.txt"), "2\n");
    for rev in [
        json!("no-such-rev"),
        json!("index"),
        json!("worktree"),
        json!(7),
    ] {
        assert!(
            h.try_run("file.restore", json!({ "path": "a.txt", "rev": rev }))
                .is_err()
        );
    }
    assert!(
        h.try_run("file.restore", json!({ "path": "a.txt" }))
            .is_err()
    );
    assert!(
        h.try_run(
            "file.open",
            json!({ "path": "a.txt", "command": "no-such-program-bitbraid-test {path}" })
        )
        .is_err()
    );
    assert!(h.porcelain().is_empty(), "nothing changed");
}

/// `file.open` runs the given command with the file's absolute path as one
/// argument, without a shell.
#[cfg(unix)]
#[test]
fn file_open_runs_the_editor_command_on_the_file() {
    let mut h = Harness::new("linear");
    h.scratch.write("with space.txt", "x\n").unwrap();
    // `cp <file> <file>.copy`: the copy appears only if each path arrived as
    // one argument, space and all.
    let out = h.run(
        "file.open",
        json!({ "path": "with space.txt", "command": "cp {path} {path}.copy" }),
    );
    assert_eq!(out["opened"], "with space.txt");
    let copy = h.scratch.path().join("with space.txt.copy");
    // The program is not waited for: the copy is there a moment later, and
    // its content a moment after the file is (seen empty on a busy machine).
    for _ in 0..250 {
        if std::fs::read_to_string(&copy).is_ok_and(|text| !text.is_empty()) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(&copy).unwrap(), "x\n");
}

/// An editor command quoted the way a command for a shell is
/// gets the path and the line without the quotes, the path with its blank
/// as one argument, and a quoted word of its own as one.
#[cfg(unix)]
#[test]
fn file_open_takes_the_quotes_off_a_quoted_editor_command() {
    let mut h = Harness::new("linear");
    h.scratch.write("with space.txt", "x\n").unwrap();
    // The shell started writes its other arguments next to the file it
    // was given first, one a line.
    let out = h.run(
        "file.open",
        json!({
            "path": "with space.txt",
            "line": 7,
            "command": r#"sh -c 'printf "%s\n" "$@" > "$0.args"' "{path}" "{line}" 'two words'"#,
        }),
    );
    assert_eq!(out["opened"], "with space.txt");
    let written = h.scratch.path().join("with space.txt.args");
    for _ in 0..250 {
        if std::fs::read_to_string(&written).is_ok_and(|text| text.ends_with("words\n")) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(std::fs::read_to_string(&written).unwrap(), "7\ntwo words\n");
    // A quote that is not closed starts nothing.
    let err = h
        .try_run(
            "file.open",
            json!({ "path": "with space.txt", "command": "touch \"{path}.never" }),
        )
        .unwrap_err();
    assert!(err.contains("not closed"), "{err}");
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!h.scratch.path().join("with space.txt.never").exists());
}
