//! What the window does, as `behaviour.rs` asks it, while work goes on
//! around a repository all the time: agents and editors in its other
//! worktrees.

use std::time::Duration;

use crate::{
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "work_in_another_worktree_is_counted_without_reading_this_one_again",
        run: work_in_another_worktree_is_counted_without_reading_this_one_again,
    },
    Test {
        name: "busy_worktrees_are_looked_at_once_a_minute_one_at_a_time",
        run: busy_worktrees_are_looked_at_once_a_minute_one_at_a_time,
    },
];

/// What is uncommitted in the other worktrees, by path.
fn other_worktrees(stage: &mut Stage) -> Vec<(String, usize)> {
    stage.workspace().other_worktrees
}

/// Work going on in another worktree writes its index under
/// `.git/worktrees` again and again (each `git add`, and each `git status`
/// that refreshes it): its row and its count follow, at most once a minute
/// (see the test below), and this worktree's references and history are not
/// read again for it, or a repository of twenty worktrees with agents at
/// work in a few of them would be read and drawn anew several times a
/// second.
fn work_in_another_worktree_is_counted_without_reading_this_one_again() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let other = repo
        .parent()
        .expect("the project is in a folder")
        .join("project-hotfix");
    let mut stage = Stage::open(Start::with(&repo));
    // The first look at the other worktrees waits a moment after opening.
    stage.pass(Duration::from_secs(3));
    stage.until("the other worktree is looked at", |stage| {
        stage.settle();
        !other_worktrees(stage).is_empty()
    });
    let seen = other_worktrees(&mut stage);
    let path = other.to_string_lossy().into_owned();
    assert_eq!(
        seen.iter().find(|(at, _)| *at == path).map(|(_, n)| *n),
        Some(0),
        "{seen:?}"
    );
    let reloads = stage.workspace().reloads;

    // An agent at work there: a file written and added, three times.
    for round in 0..3 {
        std::fs::write(other.join(format!("agent-{round}.txt")), "notes\n").unwrap();
        repos::git(&other, &["add", "-A"]);
        stage.pass(Duration::from_secs(16));
        stage.settle();
    }
    let counted = |stage: &mut Stage| {
        other_worktrees(stage)
            .iter()
            .any(|(at, count)| *at == path && *count == 3)
    };
    stage.until(
        "the other worktree's changes are counted, or this one read again",
        |stage| {
            stage.pass(Duration::from_secs(16));
            stage.settle();
            counted(stage) || stage.workspace().reloads != reloads
        },
    );
    assert_eq!(
        stage.workspace().reloads,
        reloads,
        "nothing of this worktree was read again for work in another"
    );
    assert!(counted(&mut stage), "{:?}", other_worktrees(&mut stage));
}

/// Agents at work in two other worktrees of a repository that is open in
/// two tabs, the project's and its hotfix worktree's, each staging a new
/// file every ten seconds for six minutes: in the background each worktree
/// is looked at at most once a minute, whichever tab looks, one look at a
/// time in the whole application with a pause between two, and none while
/// another runs; what was found reaches the tabs within a minute and a
/// look.
fn busy_worktrees_are_looked_at_once_a_minute_one_at_a_time() {
    repos::fresh();
    let repo = repos::project_with_everything();
    let folder = repo.parent().expect("the project is in a folder");
    let hotfix = folder.join("project-hotfix");
    let agent = folder.join("project-agent");
    repos::git(
        &repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "agent",
            agent.to_str().unwrap(),
            "main",
        ],
    );
    let state = format!("open = [{repo:?}, {hotfix:?}]\nactive = 0\n");
    let mut stage = Stage::open(Start::with(&repo).state(state));
    // Both tabs are up, the project's shown.
    stage.click(("tab-name", 1usize));
    stage.settle();
    stage.click(("tab-name", 0usize));
    stage.settle();
    let (path, agent_path) = (
        hotfix.to_string_lossy().into_owned(),
        agent.to_string_lossy().into_owned(),
    );
    let steps = 36;
    for step in 0..steps {
        for (dir, name) in [(&agent, "agent"), (&hotfix, "fix")] {
            std::fs::write(dir.join(format!("{name}-{step}.txt")), "notes\n").unwrap();
            repos::git(dir, &["add", "-A"]);
        }
        // The watcher sees it on its own thread, a moment later.
        std::thread::sleep(Duration::from_millis(350));
        for _ in 0..2 {
            stage.pass(Duration::from_secs(5));
            stage.settle();
        }
    }
    // A minute and a look later, what the agents did is counted. (A look
    // takes as long on this clock as it is moved on while git runs, and
    // the pause after it as long again: it is moved on a little at a time.)
    for _ in 0..16 {
        stage.pass(Duration::from_secs(5));
        stage.settle();
    }
    let probe = stage.workspace();
    let looks = probe.worktree_looks.clone();
    assert_eq!(probe.worktree_look_overlaps, 0, "{looks:#?}");
    // One at a time, a pause between two.
    for pair in looks.windows(2) {
        let (before, after) = (&pair[0], &pair[1]);
        assert!(
            after.began >= before.ended,
            "a look began before the one before it ended: {looks:#?}"
        );
        if !after.asked {
            assert!(
                after.began >= before.ended + Duration::from_secs(2),
                "no pause between two looks: {looks:#?}"
            );
        }
    }
    // Each worktree at most once a minute in the background, and at
    // least every other minute while it is busy.
    for watched in [&agent_path, &path] {
        let began: Vec<Duration> = looks
            .iter()
            .filter(|look| !look.asked && look.paths.contains(watched))
            .map(|look| look.began)
            .collect();
        for pair in began.windows(2) {
            assert!(
                pair[1] - pair[0] >= Duration::from_secs(60),
                "{watched} was looked at again {:?} after a look: {looks:#?}",
                pair[1] - pair[0]
            );
        }
        assert!(
            began.len() >= 4,
            "{watched} was looked at {} times in six minutes: {looks:#?}",
            began.len()
        );
    }
    let counted = |stage: &mut Stage, path: &str| {
        stage
            .workspace()
            .other_worktrees
            .iter()
            .find(|(at, _)| at == path)
            .map(|(_, count)| *count)
    };
    assert_eq!(counted(&mut stage, &agent_path), Some(steps));
    assert_eq!(counted(&mut stage, &path), Some(steps));
    // The other tab knows as much of the agent's worktree.
    stage.click(("tab-name", 1usize));
    stage.settle();
    assert_eq!(counted(&mut stage, &agent_path), Some(steps));
}
