//! What the window says, as `behaviour.rs` asks it, while something it
//! was asked to do takes a while: git is held up (`Stage::hold_git`) and
//! the window is looked at meanwhile.

use std::time::Duration;

use bitbraid_ui::{
    keymap,
    views::worktrees::{RunWorktree, WorktreeRequest},
};

use crate::{
    hermetic, repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "a_worktree_being_removed_says_so_until_it_is_gone",
        run: a_worktree_being_removed_says_so_until_it_is_gone,
    },
    Test {
        name: "a_change_that_takes_a_while_shows_in_the_status_bar",
        run: a_change_that_takes_a_while_shows_in_the_status_bar,
    },
];

/// What the references panel's row of the worktree at `path` says at its
/// right, and whether it opens; none while it has no row.
fn worktree_said(stage: &mut Stage, path: &str) -> Option<(Option<String>, bool)> {
    stage.with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).worktree_said(path))
}

/// Open a section of the references panel with a click on its heading.
fn open_section(stage: &mut Stage, section: &str) {
    let (_, head, _) = stage
        .with_workspace(|workspace, _, cx| workspace.read(cx).refs().read(cx).sections_shown())
        .into_iter()
        .find(|(key, _, _)| *key == section)
        .expect("the section is on show");
    stage.click_at((f32::from(head.x), f32::from(head.y)));
    stage.settle();
}

/// Removing a large worktree takes as long as deleting its files, and git
/// says nothing meanwhile: the status bar says what is being removed,
/// with no stop, since git cannot stop half-way; its row says so too, and
/// neither opens nor offers anything until it is gone.
fn a_worktree_being_removed_says_so_until_it_is_gone() {
    if !hermetic::holds_git() {
        println!("skipped: git cannot be held up here");
        return;
    }
    repos::fresh();
    let repo = repos::project_with_everything();
    let other = repo
        .parent()
        .expect("the project is in a folder")
        .join("project-hotfix");
    let path = other.to_string_lossy().into_owned();
    let mut stage = Stage::open(Start::with(&repo));
    open_section(&mut stage, "worktrees");
    let (said, opens) = worktree_said(&mut stage, &path).expect("the worktree has a row");
    assert_ne!(said.as_deref(), Some("removing…"));
    assert!(opens);

    stage.hold_git("remove");
    // As the menu of its row does.
    stage.act(keymap::FocusReferences);
    stage.act(RunWorktree {
        request: WorktreeRequest::Remove {
            path: path.clone(),
            with_branch: false,
        },
    });
    // The question in the bar.
    stage.click(("ask-choice", 0usize));
    let probe = stage.workspace();
    assert_eq!(probe.removing, vec![path.clone()]);
    assert_eq!(
        probe.operation,
        Some(("Removing the worktree project-hotfix…".to_owned(), false))
    );
    assert!(!stage.shows("cancel-operation"));
    assert_eq!(
        worktree_said(&mut stage, &path),
        Some((Some("removing…".to_owned()), false))
    );
    assert!(other.exists(), "git is held up before it deletes anything");

    stage.release_git();
    stage.frame();
    let probe = stage.workspace();
    assert!(probe.removing.is_empty(), "{:?}", probe.removing);
    assert_eq!(probe.operation, None);
    assert!(!other.exists());
    assert_eq!(worktree_said(&mut stage, &path), None);
}

/// A command that changes the repository runs on the session thread, and
/// what is asked after it waits: once it has run for a moment the status
/// bar says what it is, with no stop, and when it is done it says nothing
/// again. What answers sooner shows nothing.
fn a_change_that_takes_a_while_shows_in_the_status_bar() {
    if !hermetic::holds_git() {
        println!("skipped: git cannot be held up here");
        return;
    }
    repos::fresh();
    let repo = repos::project();
    repos::git(&repo, &["branch", "topic"]);
    let mut stage = Stage::open(Start::with(&repo));

    stage.hold_git("switch");
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.checkout("topic".to_owned(), window, cx)
        })
    });
    stage.until("the checkout has begun", |stage| {
        stage.workspace().busy.is_some()
    });
    assert_eq!(stage.workspace().busy.as_deref(), Some("Check out"));
    assert_eq!(stage.workspace().operation, None);
    stage.pass(Duration::from_millis(299));
    assert_eq!(stage.workspace().operation, None, "not after 299 ms");
    stage.pass(Duration::from_millis(1));
    assert_eq!(
        stage.workspace().operation,
        Some(("Running “Check out”…".to_owned(), false))
    );
    assert!(!stage.shows("cancel-operation"));

    stage.release_git();
    let probe = stage.workspace();
    assert_eq!(probe.busy, None);
    assert_eq!(probe.operation, None);
    assert_eq!(
        repos::git(&repo, &["branch", "--show-current"]).trim(),
        "topic"
    );
}
