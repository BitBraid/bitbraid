//! Completing git commands typed in the terminal: the names
//! read for it are git's, and a line is completed with them.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

struct Harness {
    copy: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    /// The fixture with two feature branches and a tag, plus a remote
    /// with branches of its own, a stash, a change, a staged file, a new
    /// file, and a branch whose name a shell has to read quoted.
    fn new() -> Self {
        let copy = Scratch::of("branches-merges").unwrap();
        let git = copy.git();
        let path = copy.path().display().to_string();
        git.run(["remote", "add", "origin", &path]).unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        git.run(["branch", "we'ird"]).unwrap();
        copy.write("a.txt", "stashed").unwrap();
        git.run(["stash", "push", "-q", "-m", "half done"]).unwrap();
        copy.write("README.md", "# changed").unwrap();
        copy.write("docs/new.md", "new").unwrap();
        copy.write("staged.txt", "staged").unwrap();
        git.run(["add", "staged.txt"]).unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .unwrap();
        Self {
            copy,
            registry,
            ctx,
        }
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

    fn labels(&mut self, line: &str) -> Vec<String> {
        let out = self.run("terminal.complete", json!({ "line": line }));
        out["candidates"]
            .as_array()
            .unwrap()
            .iter()
            .map(|candidate| candidate["label"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn texts(value: &Value) -> Vec<String> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item.as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn the_names_are_the_ones_git_lists() {
    let mut h = Harness::new();
    let git = h.copy.git();
    let words = h.run("terminal.completion_words", json!({}));
    let mut branches = texts(&words["branches"]);
    branches.sort();
    assert_eq!(branches, git.refs("refs/heads").unwrap());
    let mut tags = texts(&words["tags"]);
    tags.sort();
    assert_eq!(tags, git.lines(["tag"]).unwrap());
    let remotes: Vec<String> = words["remotes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|remote| remote["name"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(remotes, git.lines(["remote"]).unwrap());
    let mut theirs: Vec<String> = texts(&words["remotes"][0]["branches"])
        .into_iter()
        .map(|branch| format!("origin/{branch}"))
        .collect();
    theirs.sort();
    let mut listed: Vec<String> = git
        .refs("refs/remotes/origin")
        .unwrap()
        .into_iter()
        .filter(|name| name != "origin/HEAD" && name != "origin")
        .collect();
    listed.sort();
    assert_eq!(theirs, listed);
    assert_eq!(
        words["stashes"].as_array().unwrap().len(),
        git.lines(["stash", "list"]).unwrap().len()
    );
    assert_eq!(texts(&words["tracked"]), git.lines(["ls-files"]).unwrap());
    assert_eq!(words["more_tracked"], false);
    let mut changed: Vec<String> = words["changes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|change| change["path"].as_str().unwrap().to_owned())
        .collect();
    changed.sort();
    let mut porcelain: Vec<String> = git
        .run(["status", "--porcelain", "--untracked-files=all"])
        .unwrap()
        .lines()
        .filter_map(|line| line.get(3..))
        .map(str::to_owned)
        .collect();
    porcelain.sort();
    assert_eq!(changed, porcelain);
    assert_eq!(words["head"], "main");
    // A cap on the paths, and a word that it was reached.
    let capped = h.run("terminal.completion_words", json!({ "most_paths": 1 }));
    assert_eq!(capped["tracked"].as_array().unwrap().len(), 1);
    assert_eq!(capped["more_tracked"], true);
    let without = h.run("terminal.completion_words", json!({ "changes": false }));
    assert_eq!(without["changes"], json!([]));
}

#[test]
fn a_line_is_completed_with_the_repositorys_names() {
    let mut h = Harness::new();
    let git = h.copy.git();
    // Every name of a commit, after the options.
    let names: Vec<String> = h
        .labels("git checkout ")
        .into_iter()
        .filter(|label| !label.starts_with('-'))
        .collect();
    let mut expected: Vec<String> = git.refs("refs/heads").unwrap();
    expected.extend(
        git.refs("refs/remotes/origin")
            .unwrap()
            .into_iter()
            .filter(|name| name != "origin/HEAD" && name != "origin"),
    );
    expected.extend(git.lines(["tag"]).unwrap());
    let mut sorted = names.clone();
    sorted.sort();
    expected.sort();
    assert_eq!(sorted, expected);
    // Commands, remotes, stashes, changed paths.
    assert_eq!(h.labels("git chec")[..1], ["checkout"]);
    assert_eq!(h.labels("git push or"), ["origin"]);
    assert_eq!(h.labels("git stash pop st"), ["stash@{0}"]);
    assert_eq!(h.labels("git add README"), ["README.md"]);
    assert_eq!(h.labels("git add do"), ["docs/new.md", "docs/"]);
    assert_eq!(h.labels("git restore --staged st"), ["staged.txt"]);
    // From a folder below the top.
    let out = h.run(
        "terminal.complete",
        json!({ "line": "git add n", "folder": "docs" }),
    );
    assert_eq!(out["candidates"][0]["label"], "new.md");
    assert_eq!(out["start"], 8);
    assert_eq!(out["word"], "n");
    // A name a shell reads only quoted is written quoted.
    let out = h.run("terminal.complete", json!({ "line": "git branch -D we" }));
    let insert = out["candidates"][0]["insert"].as_str().unwrap().to_owned();
    assert_ne!(insert, "we'ird");
    // (On Windows by the sh of Git for Windows.)
    #[cfg(windows)]
    let sh = bitbraid_testkit::script::git_sh().unwrap();
    #[cfg(not(windows))]
    let sh = std::path::PathBuf::from("/bin/sh");
    let echoed = std::process::Command::new(sh)
        .args(["-c", &format!("printf %s {insert}")])
        .output()
        .unwrap();
    assert_eq!(String::from_utf8_lossy(&echoed.stdout), "we'ird");
    // Not git: nothing.
    let out = h.run("terminal.complete", json!({ "line": "ls -l" }));
    assert_eq!(out, Value::Null);
}

#[test]
fn hostile_parameters_are_refused() {
    let mut h = Harness::new();
    let long = format!("git checkout {}", "x".repeat(5000));
    assert!(
        h.try_run("terminal.complete", json!({ "line": long }))
            .is_err()
    );
    assert!(h.try_run("terminal.complete", json!({})).is_err());
    for folder in ["../outside", "a/../../b", "./x", "a\u{7}"] {
        assert!(
            h.try_run(
                "terminal.complete",
                json!({ "line": "git add ", "folder": folder })
            )
            .is_err(),
            "{folder}"
        );
    }
    assert!(
        h.try_run(
            "terminal.complete",
            json!({ "line": "git add ", "shell": "tcsh" })
        )
        .is_err()
    );
    assert!(
        h.try_run(
            "terminal.completion_words",
            json!({ "most_paths": 10_000_000 })
        )
        .is_err()
    );
    assert!(
        h.try_run("terminal.completion_words", json!({ "most_paths": -1 }))
            .is_err()
    );
    // Lines no shell would type are answered, not crashed on.
    for line in [
        "git \u{0}\u{1b}[31m",
        "git checkout \u{202e}evil",
        "git 'unclosed",
        "git \\",
        "git -C",
        "git stash -- --",
        "git checkout -- ../../../etc/",
    ] {
        h.run("terminal.complete", json!({ "line": line }));
    }
}
