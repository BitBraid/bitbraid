//! Paths longer than Windows takes without long paths: a file more than
//! 260 characters deep is listed, compared, staged, committed, checked
//! out, blamed and followed through history as git does it (on Windows
//! with git's long paths on), mutations held to git on a twin clone. What
//! is too long for Windows is told, and git's long paths are turned on
//! (and taken back by undo).

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

/// Five folders of 52 characters and a name of 40: 305 characters below
/// the worktree, which is beyond 260 wherever the worktree is.
fn deep_path() -> String {
    let mut parts: Vec<String> = (0..5).map(|n| format!("{n}{}", "d".repeat(51))).collect();
    parts.push(format!("{}.txt", "f".repeat(36)));
    parts.join("/")
}

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .unwrap();
        Self {
            scratch,
            registry,
            ctx,
        }
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn git(&self) -> Git {
        self.scratch.git()
    }
}

/// The index as git lists it: mode, blob and path of every entry.
fn index(git: &Git) -> String {
    git.run(["ls-files", "--stage"]).unwrap()
}

#[test]
fn a_file_deeper_than_windows_takes_is_worked_with_as_git_does() {
    let deep = deep_path();
    assert!(deep.len() > 300, "{}", deep.len());
    let mut h = Harness::new();
    let twin = Scratch::of("linear").unwrap();
    let (ours, theirs) = (h.git(), twin.git());
    assert!(h.scratch.path().join(&deep).to_string_lossy().len() > 260);
    // On Windows git takes such a path only with its long paths on, which
    // the banner turns on (the next test); without, it does not see the
    // file at all.
    if cfg!(windows) {
        for git in [&ours, &theirs] {
            git.run(["config", "core.longpaths", "true"]).unwrap();
        }
    }

    // A new file: untracked, as git lists it.
    for scratch in [&h.scratch, &twin] {
        scratch.write(&deep, "one\ntwo\nthree\n").unwrap();
    }
    let status = h.run("repo.status", json!({}));
    let entry = status
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == deep.as_str())
        .unwrap_or_else(|| panic!("{deep} is not in {status}"));
    assert_eq!(entry["unstaged"], "untracked", "{entry}");
    assert!(
        ours.run(["status", "--porcelain", "-uall"])
            .unwrap()
            .contains(&format!("?? {deep}"))
    );

    // Staged and committed: the same index and tree as git makes.
    h.run("stage.paths", json!({ "paths": [deep] }));
    theirs.run(["add", "--", &deep]).unwrap();
    assert_eq!(index(&ours), index(&theirs));
    h.run("commit.create", json!({ "message": "a deep file" }));
    theirs.run(["commit", "-q", "-m", "a deep file"]).unwrap();
    let tree = |git: &Git| git.rev_parse("HEAD^{tree}").unwrap();
    assert_eq!(tree(&ours), tree(&theirs));

    // Changed, compared as git compares it, and staged hunk by hunk.
    for scratch in [&h.scratch, &twin] {
        scratch.write(&deep, "one\nTWO\nthree\nfour\n").unwrap();
    }
    let diff = h.run(
        "repo.diff",
        json!({ "old": "index", "new": "worktree", "paths": [deep] }),
    );
    let file = &diff[0];
    assert_eq!(file["path"], deep.as_str());
    assert_eq!(
        (file["additions"].as_u64(), file["deletions"].as_u64()),
        (Some(2), Some(1))
    );
    let numstat = ours.run(["diff", "--numstat", "--", &deep]).unwrap();
    assert!(numstat.starts_with("2\t1\t"), "{numstat}");
    let changed: Vec<usize> = file["hunks"][0]["lines"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
        .filter(|(_, line)| line["kind"] != "context")
        .map(|(at, _)| at)
        .collect();
    h.run(
        "stage.hunks",
        json!({ "path": deep, "selection": [{ "hunk": 0, "lines": changed }] }),
    );
    theirs.run(["add", "--", &deep]).unwrap();
    assert_eq!(index(&ours), index(&theirs));
    h.run(
        "commit.create",
        json!({ "message": "the deep file changed" }),
    );
    theirs
        .run(["commit", "-q", "-m", "the deep file changed"])
        .unwrap();
    assert_eq!(tree(&ours), tree(&theirs));

    // Who wrote each line, and the file's history, as git has them.
    let blame = h.run("repo.blame", json!({ "path": deep }));
    let porcelain = ours.run(["blame", "--porcelain", "--", &deep]).unwrap();
    let mut wanted = Vec::new();
    for line in porcelain.lines() {
        let mut words = line.split(' ');
        if let (Some(id), Some(_), Some(_)) = (words.next(), words.next(), words.next())
            && id.len() == 40
            && id.chars().all(|ch| ch.is_ascii_hexdigit())
        {
            wanted.push(id.to_owned());
        }
    }
    let got: Vec<String> = blame
        .as_array()
        .unwrap()
        .iter()
        .map(|line| line["commit"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(got, wanted);
    let history = h.run("repo.history", json!({ "path": deep }));
    let got: Vec<&str> = history
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["commit"]["id"].as_str().unwrap())
        .collect();
    let log = ours.run(["log", "--format=%H", "--", &deep]).unwrap();
    let wanted: Vec<&str> = log.lines().collect();
    assert_eq!(got, wanted);

    // Checked out without it and with it again.
    h.run(
        "branch.create",
        json!({ "name": "before-deep", "start": "HEAD~2" }),
    );
    h.run("branch.checkout", json!({ "target": "before-deep" }));
    assert!(!h.scratch.path().join(&deep).exists());
    h.run("branch.checkout", json!({ "target": "main" }));
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join(&deep)).unwrap(),
        "one\nTWO\nthree\nfour\n"
    );
    assert_eq!(ours.run(["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn what_is_too_long_for_windows_is_told_and_long_paths_are_turned_on() {
    let deep = deep_path();
    let mut h = Harness::new();
    let report = h.run("repo.long_paths", json!({}));
    assert_eq!(report["beyond"], 0, "{report}");
    assert_eq!(report["enabled"], false);
    assert_eq!(report["applies"], cfg!(windows));
    assert_eq!(report["blocks_git"], false);

    h.scratch.write(&deep, "deep\n").unwrap();
    h.run("stage.paths", json!({ "paths": [deep] }));
    let report = h.run("repo.long_paths", json!({}));
    assert_eq!(report["beyond"], 1, "{report}");
    assert_eq!(report["longest"], deep.as_str());
    let root = h.scratch.path().to_string_lossy().encode_utf16().count();
    assert_eq!(
        report["longest_length"].as_u64(),
        u64::try_from(root + 1 + deep.len()).ok()
    );
    assert_eq!(report["blocks_git"], cfg!(windows));

    let turned = h.run("repo.long_paths_enable", json!({}));
    assert_eq!(turned["enabled"], true, "{turned}");
    assert_eq!(turned["beyond"], 1, "{turned}");
    assert_eq!(
        h.git()
            .run(["config", "--local", "--get", "core.longpaths"])
            .unwrap()
            .trim(),
        "true"
    );
    // A setting of the repository's: undo takes it back.
    h.run("undo.last", json!({}));
    assert!(
        h.git()
            .run(["config", "--local", "--get", "core.longpaths"])
            .is_err()
    );
}

/// Hostile parameters and a repository of no worktree: refusals, no
/// panic, and nothing written to the user's configuration.
#[test]
fn the_windows_commands_take_hostile_input() {
    let mut h = Harness::new();
    for params in [
        json!({ "scope": "nowhere" }),
        json!({ "scope": 5 }),
        json!({ "scope": null, "extra": [1, 2] }),
        json!([]),
        json!("text"),
    ] {
        let _ = h.try_run("repo.long_paths_enable", params.clone());
        let _ = h.try_run("repo.long_paths", params.clone());
        let _ = h.try_run("repo.wsl", params);
    }
    let refused = h
        .try_run("repo.long_paths_enable", json!({ "scope": "nowhere" }))
        .unwrap_err();
    assert!(refused.contains("scope"), "{refused}");
    // Not inside WSL: a path of this system never is one.
    assert_eq!(h.run("repo.wsl", json!({})), json!({ "inside": false }));

    // A bare repository: nothing there to be too long, and the
    // repository's own configuration is not written.
    let bare = Scratch::of("bare").unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": bare.path() }))
        .unwrap();
    let report = registry
        .run("repo.long_paths", &mut ctx, &json!({}))
        .unwrap();
    assert_eq!(report["beyond"], 0);
    assert!(
        registry
            .run("repo.long_paths_enable", &mut ctx, &json!({}))
            .is_err()
    );
    assert_eq!(
        registry.run("repo.wsl", &mut ctx, &json!({})).unwrap()["inside"],
        false
    );
}

/// `base` with folders below it to `length` UTF-16 units in all, none of
/// them longer than 50 characters.
#[cfg(windows)]
fn folder_of(base: &std::path::Path, length: usize) -> std::path::PathBuf {
    let mut folder = base.to_path_buf();
    loop {
        let now = folder.to_string_lossy().encode_utf16().count();
        if now >= length {
            return folder;
        }
        let left = length - now - 1;
        folder.push("q".repeat(if left > 52 { 50 } else { left }));
    }
}

/// A worktree in a folder Git for Windows makes none in, long paths on or
/// not, is refused before git is asked, in our words, and nothing is made;
/// one at the longest it takes is made, and git works in it.
#[cfg(windows)]
#[test]
fn a_worktree_folder_too_long_for_git_on_windows_is_refused_in_our_words() {
    let mut h = Harness::new();
    h.git().run(["config", "core.longpaths", "true"]).unwrap();
    let base = h.scratch.path().with_extension("wt");
    let before = h.git().run(["worktree", "list", "--porcelain"]).unwrap();

    let deep = folder_of(&base, 216);
    let refused = h
        .try_run(
            "worktree.add",
            json!({ "path": deep, "new_branch": "deep" }),
        )
        .unwrap_err();
    assert!(refused.contains("216 characters"), "{refused}");
    assert!(refused.contains("even with long paths"), "{refused}");
    assert!(!base.exists());
    assert_eq!(
        h.git().run(["worktree", "list", "--porcelain"]).unwrap(),
        before
    );
    assert!(h.git().rev_parse("refs/heads/deep").is_err());

    let fits = folder_of(&base, 215);
    h.run(
        "worktree.add",
        json!({ "path": fits, "new_branch": "fits" }),
    );
    assert!(fits.join(".git").is_file());
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&fits)
        .args(["status", "--porcelain", "--branch"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(String::from_utf8_lossy(&out.stdout).starts_with("## fits"));
    h.run("worktree.remove", json!({ "path": fits }));
    let _ = std::fs::remove_dir_all(&base);
}

/// With git's long paths off, what Windows' limit keeps git from is
/// refused and said in our words; once such a file is checked out the
/// report the banner reads says git is kept from it; turned on from
/// there, the same work goes through.
#[cfg(windows)]
#[test]
fn with_long_paths_off_git_is_refused_in_our_words_until_they_are_turned_on() {
    let deep = deep_path();
    let mut h = Harness::new();
    let git = h.git();
    git.run(["config", "core.longpaths", "false"]).unwrap();
    // A branch with the deep file, made by git with long paths on for once.
    git.run(["switch", "-q", "-c", "deep"]).unwrap();
    h.scratch.write(&deep, "deep\n").unwrap();
    git.run(["-c", "core.longpaths=true", "add", "--", &deep])
        .unwrap();
    git.run(["-c", "core.longpaths=true", "commit", "-q", "-m", "deep"])
        .unwrap();
    git.run(["-c", "core.longpaths=true", "switch", "-q", "main"])
        .unwrap();
    assert!(!h.scratch.path().join(&deep).exists());

    // Checked out with them off: git makes nothing, and it is said.
    let refused = h
        .try_run("branch.checkout", json!({ "target": "deep" }))
        .unwrap_err();
    assert!(refused.contains("Filename too long"), "{refused}");
    assert!(refused.contains("core.longpaths true"), "{refused}");
    assert_eq!(
        git.run(["branch", "--show-current"]).unwrap().trim(),
        "main"
    );

    // Where it is checked out all the same, the banner's report says git
    // is kept from it, and staging a change to it is refused and said.
    git.run(["-c", "core.longpaths=true", "switch", "-q", "deep"])
        .unwrap();
    let report = h.run("repo.long_paths", json!({}));
    assert_eq!(report["beyond"], 1, "{report}");
    assert_eq!(report["enabled"], false, "{report}");
    assert_eq!(report["blocks_git"], true, "{report}");
    h.scratch.write(&deep, "changed\n").unwrap();
    let refused = h
        .try_run("stage.paths", json!({ "paths": [deep] }))
        .unwrap_err();
    assert!(refused.contains("core.longpaths true"), "{refused}");

    // Turned on from the banner: staged, and checked out both ways.
    let turned = h.run("repo.long_paths_enable", json!({}));
    assert_eq!(turned["enabled"], true, "{turned}");
    h.run("stage.paths", json!({ "paths": [deep] }));
    assert!(
        git.run([
            "-c",
            "core.longpaths=true",
            "diff",
            "--cached",
            "--name-only"
        ])
        .unwrap()
        .contains("ffff")
    );
    h.run("commit.create", json!({ "message": "changed deep" }));
    h.run("branch.checkout", json!({ "target": "main" }));
    assert!(!h.scratch.path().join(&deep).exists());
    h.run("branch.checkout", json!({ "target": "deep" }));
    assert_eq!(
        std::fs::read_to_string(h.scratch.path().join(&deep)).unwrap(),
        "changed\n"
    );
}

/// A worktree whose folder grew longer than git works in on Windows after
/// it was made (moved deeper by the file system): neither git nor
/// gitoxide reads it, so opening it says why in our words, and one at the
/// longest that works opens.
#[cfg(windows)]
#[test]
fn a_worktree_whose_folder_grew_too_long_is_said_so_when_opened() {
    let mut h = Harness::new();
    let base = h.scratch.path().with_extension("grown");
    let short = base.join("wt");
    h.run(
        "worktree.add",
        json!({ "path": short, "new_branch": "grown" }),
    );
    let open = |path: &std::path::Path| {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": path }))
            .map(|_| ())
            .map_err(|err| err.to_string())
    };

    let fits = folder_of(&base.join("a"), 254);
    std::fs::create_dir_all(fits.parent().unwrap()).unwrap();
    std::fs::rename(&short, &fits).unwrap();
    open(&fits).unwrap();

    let grown = folder_of(&base.join("b"), 270);
    std::fs::create_dir_all(grown.parent().unwrap()).unwrap();
    std::fs::rename(&fits, &grown).unwrap();
    let refused = open(&grown).unwrap_err();
    assert!(refused.contains("270 characters long"), "{refused}");
    assert!(refused.contains("move it to a shorter folder"), "{refused}");
    // Git can do nothing there either.
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(&grown)
        .arg("status")
        .output()
        .unwrap();
    assert!(!out.status.success());
    let _ = std::fs::remove_dir_all(&base);
}
