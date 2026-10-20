//! What the window does, as `behaviour.rs` asks it, for undo: what an
//! action that changes the repository leaves to undo, and what is said
//! when undoing cannot be done; and for what such an action may leave
//! behind, conflicts without an operation in progress.

use std::path::{Path, PathBuf};

use bitbraid_ui::keymap;

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_checkout_that_carried_changes_is_undone_carrying_them_back",
        run: a_checkout_that_carried_changes_is_undone_carrying_them_back,
    },
    Test {
        name: "a_conflict_without_an_operation_is_listed_to_be_resolved",
        run: a_conflict_without_an_operation_is_listed_to_be_resolved,
    },
];

const BASE: &str = "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\ntheta\n";

/// A file on `main`, a branch `other` that changes its first line, and on
/// `main` a change of its last line that is not committed.
fn checkout_repository() -> PathBuf {
    let repo = repos::fixture("linear", "checkout");
    std::fs::write(repo.join("file.txt"), BASE).unwrap();
    repos::git(&repo, &["add", "file.txt"]);
    repos::git(&repo, &["commit", "-q", "-m", "Checkout baseline"]);
    repos::git(&repo, &["switch", "-q", "-c", "other"]);
    std::fs::write(repo.join("file.txt"), BASE.replace("alpha", "other alpha")).unwrap();
    repos::git(&repo, &["commit", "-q", "-am", "Other first line"]);
    repos::git(&repo, &["switch", "-q", "main"]);
    std::fs::write(repo.join("file.txt"), BASE.replace("theta", "local theta")).unwrap();
    repo
}

fn head(repo: &Path) -> String {
    repos::git(repo, &["symbolic-ref", "--short", "HEAD"])
        .trim()
        .to_owned()
}

fn file(repo: &Path) -> String {
    std::fs::read_to_string(repo.join("file.txt")).unwrap()
}

/// Checked out with the local change carried in a stash, undo goes back
/// with the change carried the same way; and an undo that cannot be done is
/// said in a notification, not only in the activity log.
fn a_checkout_that_carried_changes_is_undone_carrying_them_back() {
    repos::fresh();
    let repo = checkout_repository();
    // The preferences say to carry changes without asking.
    let mut stage = Stage::open(Start::with(&repo).preferences("checkout_stash = true\n"));
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.checkout("other".to_owned(), window, cx)
        })
    });
    stage.until("on the other branch", |_| head(&repo) == "other");
    stage.settle();
    assert_eq!(
        file(&repo),
        "other alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\neta\nlocal theta\n"
    );
    let said = stage.notices().len();

    stage.act(keymap::Undo);
    stage.until("back on main", |_| head(&repo) == "main");
    stage.settle();
    assert_eq!(file(&repo), BASE.replace("theta", "local theta"));
    assert_eq!(repos::git(&repo, &["stash", "list"]), "");
    assert_eq!(
        repos::git(&repo, &["status", "--porcelain"]),
        " M file.txt\n"
    );
    let notices = stage.notices();
    assert!(
        notices
            .iter()
            .skip(said)
            .all(|notice| !notice.contains("failed")),
        "{notices:?}"
    );

    // Redone, then moved elsewhere by other means: undo refuses, and says
    // so where it is seen.
    stage.act(keymap::Redo);
    stage.until("on the other branch again", |_| head(&repo) == "other");
    stage.settle();
    repos::git(&repo, &["switch", "-q", "-c", "elsewhere"]);
    stage.until("the window saw it", |stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).head_label()) == "elsewhere"
    });
    let said = stage.notices().len();
    stage.act(keymap::Undo);
    stage.until("a notification", |stage| stage.notices().len() > said);
    let notices = stage.notices();
    assert!(
        notices
            .last()
            .is_some_and(|notice| notice.contains("HEAD is not where")),
        "{notices:?}"
    );
    assert_eq!(head(&repo), "elsewhere");
}

/// A patch applied with a three-way merge leaves a file unmerged and no
/// operation in progress. The panel of changes lists it with the files in
/// conflict, to be resolved there, and not nowhere.
fn a_conflict_without_an_operation_is_listed_to_be_resolved() {
    repos::fresh();
    let repo = checkout_repository();
    // A patch of `other`'s first line, applied on main where that line is
    // changed too.
    let patch = repos::work().join("first-line.diff");
    let diff = repos::git(&repo, &["diff", "main", "other", "--", "file.txt"]);
    std::fs::write(&patch, diff).unwrap();
    std::fs::write(repo.join("file.txt"), BASE.replace("alpha", "main alpha")).unwrap();
    repos::git(&repo, &["commit", "-q", "-am", "Main first line"]);
    let git = bitbraid_testkit::Git::new(&repo, repos::work().join(".home"));
    git.ensure_home().unwrap();
    let applied = git.run(["apply", "--3way", patch.to_str().unwrap()]);
    assert!(applied.is_err(), "it conflicts");
    assert!(repos::git(&repo, &["ls-files", "--unmerged"]).contains("file.txt"));

    let mut stage = Stage::open(Start::with(&repo));
    stage.with_workspace(|workspace, _, cx| {
        let graph = workspace.read(cx).graph();
        graph.update(cx, |graph, cx| graph.select_wip(cx));
    });
    stage.settle();
    assert!(stage.workspace().changes_shown);
    assert_eq!(stage.workspace().in_progress, None, "no operation");
    stage.until("the conflict is listed", |stage| {
        stage.shows(("conflict", 0usize))
    });
    // Opened from there, it is the merge tool's.
    stage.click(("conflict", 0usize));
    stage.settle();
    assert_eq!(stage.workspace().file_tab, "merge");
    assert_eq!(
        stage.workspace().file.map(|(path, _)| path).as_deref(),
        Some("file.txt")
    );
}
