//! The page of repositories looks at each from outside it:
//! `repo.glance` says the branch, how far it is ahead and behind its
//! upstream, and the uncommitted files by kind, as git says them; and why
//! a folder could not be looked at. What it is given is checked first.

use std::path::Path;

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

fn glance(paths: &[&Path]) -> Vec<Value> {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let paths: Vec<_> = paths.iter().map(|path| path.to_str().unwrap()).collect();
    let answer = registry
        .run("repo.glance", &mut ctx, &json!({ "paths": paths }))
        .unwrap();
    answer.as_array().unwrap().clone()
}

#[test]
fn a_repository_is_seen_as_git_sees_it() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let bare = copy.path().with_extension("remote.git");
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run(["fetch", "-q", "origin"]).unwrap();
    git.run(["branch", "-q", "--set-upstream-to=origin/main"])
        .unwrap();
    // Two behind, one ahead.
    git.run(["reset", "-q", "--hard", "HEAD~2"]).unwrap();
    copy.write("c.txt", "c").unwrap();
    git.run(["add", "c.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "add c"]).unwrap();
    // One of each kind, uncommitted.
    copy.write("README.md", "# changed").unwrap();
    copy.remove("a.txt").unwrap();
    copy.write("d.txt", "new").unwrap();
    git.run(["mv", "c.txt", "e.txt"]).unwrap();

    let counted = git
        .run(["rev-list", "--left-right", "--count", "main...origin/main"])
        .unwrap();
    let (ahead, behind) = counted.trim().split_once('\t').unwrap();
    let status = git.run(["status", "--porcelain"]).unwrap();
    let kind = |letters: &str| {
        status
            .lines()
            .filter(|line| {
                letters.contains(line.chars().next().unwrap())
                    || letters.contains(line.chars().nth(1).unwrap())
            })
            .count()
    };

    let found = glance(&[copy.path(), &bare]);
    let seen = &found[0];
    assert_eq!(seen["path"], copy.path().to_str().unwrap());
    assert_eq!(seen["branch"], "main");
    assert_eq!(seen["ahead"].as_u64().unwrap().to_string(), ahead);
    assert_eq!(seen["behind"].as_u64().unwrap().to_string(), behind);
    assert_eq!(
        (seen["ahead"].as_u64(), seen["behind"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(seen["counts"]["renamed"], kind("R"));
    assert_eq!(seen["counts"]["deleted"], kind("D"));
    assert_eq!(seen["counts"]["added"], kind("?"));
    assert_eq!(seen["counts"]["modified"], kind("M"));
    assert!(seen["problem"].is_null(), "{seen}");
    // The bare one has a branch and no files.
    let remote = &found[1];
    assert_eq!(remote["branch"], "main");
    assert_eq!(remote["counts"]["modified"], 0);
    assert!(remote["problem"].is_null(), "{remote}");
    std::fs::remove_dir_all(&bare).unwrap();
}

#[test]
fn without_an_upstream_or_a_branch_there_is_nothing_to_count() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let found = glance(&[copy.path()]);
    assert_eq!(found[0]["branch"], "main");
    assert!(found[0]["ahead"].is_null() && found[0]["behind"].is_null());
    git.run(["checkout", "-q", "--detach", "HEAD~1"]).unwrap();
    let found = glance(&[copy.path()]);
    assert!(found[0]["branch"].is_null(), "{}", found[0]);
    assert!(found[0]["problem"].is_null(), "{}", found[0]);
}

#[test]
fn a_folder_that_is_gone_or_no_repository_says_so() {
    let copy = Scratch::of("linear").unwrap();
    let gone = copy.path().join("not-there");
    let plain = copy.path().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    // A repository whose own folder is garbage, and one that is a file.
    let garbage = copy.path().join("garbage");
    std::fs::create_dir_all(garbage.join(".git")).unwrap();
    std::fs::write(garbage.join(".git/HEAD"), "\u{0}\u{1}not a ref").unwrap();
    let pointer = copy.path().join("pointer");
    std::fs::create_dir_all(&pointer).unwrap();
    std::fs::write(pointer.join(".git"), "gitdir: /nowhere/at/all\n").unwrap();
    let file = copy.path().join("README.md");
    // A folder inside a repository is not that repository.
    let found = glance(&[&gone, &plain, &garbage, &pointer, &file]);
    assert_eq!(found.len(), 5);
    for seen in &found {
        assert!(seen["problem"].is_string(), "{seen}");
        assert!(seen["branch"].is_null(), "{seen}");
    }
}

/// When the file git says `FETCH_HEAD` is was written, in seconds.
fn written(dir: &Path, git_path: &str) -> i64 {
    let file = if Path::new(git_path).is_absolute() {
        std::path::PathBuf::from(git_path)
    } else {
        dir.join(git_path)
    };
    let modified = std::fs::metadata(file).unwrap().modified().unwrap();
    i64::try_from(
        modified
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs(),
    )
    .unwrap()
}

/// The last fetch is when git last wrote `FETCH_HEAD`, where git keeps
/// it: none before the first fetch; of a linked worktree the later of its
/// own and the repository's.
#[test]
fn the_last_fetch_is_when_git_wrote_fetch_head() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let found = glance(&[copy.path()]);
    assert!(
        found[0]["last_fetch"].is_null(),
        "never fetched: {}",
        found[0]
    );

    let bare = copy.path().with_extension("fetched.git");
    git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", bare.to_str().unwrap()])
        .unwrap();
    git.run(["fetch", "-q", "origin"]).unwrap();
    let where_git_keeps_it = git.run(["rev-parse", "--git-path", "FETCH_HEAD"]).unwrap();
    let main = written(copy.path(), where_git_keeps_it.trim());
    let found = glance(&[copy.path()]);
    assert_eq!(found[0]["last_fetch"], main, "{}", found[0]);

    // A linked worktree, never fetched from itself: the repository's.
    let linked = copy.path().with_extension("linked");
    git.run([
        "worktree",
        "add",
        "-q",
        "--detach",
        linked.to_str().unwrap(),
    ])
    .unwrap();
    let found = glance(&[&linked]);
    assert_eq!(found[0]["last_fetch"], main, "{}", found[0]);
    // Fetched from there, its own, where git keeps that.
    let there = bitbraid_testkit::Git::new(&linked, copy.git().home());
    there.run(["fetch", "-q", "origin"]).unwrap();
    let kept = there
        .run(["rev-parse", "--git-path", "FETCH_HEAD"])
        .unwrap();
    let own = written(&linked, kept.trim());
    let found = glance(&[&linked]);
    assert_eq!(found[0]["last_fetch"], own.max(main), "{}", found[0]);
    git.run(["worktree", "remove", "--force", linked.to_str().unwrap()])
        .unwrap();
    std::fs::remove_dir_all(&bare).unwrap();
}

#[test]
fn what_it_is_given_is_checked() {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let many: Vec<String> = (0..65).map(|n| format!("/tmp/{n}")).collect();
    for params in [
        json!({}),
        json!({ "paths": "/tmp" }),
        json!({ "paths": [1, 2] }),
        json!({ "paths": ["relative/path"] }),
        json!({ "paths": ["-c", "core.pager=x"] }),
        json!({ "paths": ["/tmp/a\nb"] }),
        json!({ "paths": [format!("/{}", "a".repeat(5000))] }),
        json!({ "paths": many }),
    ] {
        assert!(
            registry.run("repo.glance", &mut ctx, &params).is_err(),
            "{params}"
        );
    }
    // None at all is no work.
    let answer = registry
        .run("repo.glance", &mut ctx, &json!({ "paths": [] }))
        .unwrap();
    assert_eq!(answer, json!([]));
}
