//! Stash rows in a walked history: where they go and what they leave out.

use bitbraid_git::{GraphCommit, LogOptions, Oid, Repository, Tips, layout, walk, with_stashes};
use bitbraid_testkit::{Git, Scratch};

fn open(scratch: &Scratch) -> Repository {
    Repository::open(scratch.path()).unwrap()
}

fn all(repo: &Repository) -> Vec<GraphCommit> {
    walk(repo, &LogOptions::default()).unwrap()
}

/// Make a stash dated `date` (seconds, as git takes them) and return it.
fn stash(scratch: &Scratch, git: &Git, file: &str, date: &str) -> Oid {
    scratch
        .write(file, &format!("stashed at {date}\n"))
        .unwrap();
    git.run_with(
        ["stash", "push", "--include-untracked"],
        &[("GIT_COMMITTER_DATE", date), ("GIT_AUTHOR_DATE", date)],
    )
    .unwrap();
    git.rev_parse("stash@{0}").unwrap().parse().unwrap()
}

fn row_of(commits: &[GraphCommit], id: Oid) -> usize {
    commits
        .iter()
        .position(|commit| commit.id == id)
        .unwrap_or_else(|| panic!("{id:?} has no row"))
}

#[test]
fn a_stash_hangs_off_the_commit_it_was_made_on() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    let head: Oid = git.rev_parse("HEAD").unwrap().parse().unwrap();
    // Newer than everything in the fixture.
    let made = stash(&scratch, &git, "new.txt", "2000000000 +0000");
    let repo = open(&scratch);

    let plain = all(&repo);
    let rows = with_stashes(&repo, plain.clone(), &[made]);
    assert_eq!(rows.len(), plain.len() + 1);
    // On top, with the commit it was made on as its only parent: the
    // commits that hold the index and the untracked files are not rows.
    assert_eq!(rows[0].id, made);
    assert_eq!(rows[0].parents, [head]);
    assert_eq!(&rows[1..], &plain[..]);
    let parents = git
        .run(["rev-list", "--parents", "-n", "1", "stash@{0}"])
        .unwrap();
    assert_eq!(
        parents.split_whitespace().count(),
        4,
        "a stash with untracked files"
    );

    // The layout takes it like any commit: a row of its own above its parent.
    let laid = layout(&rows);
    assert_eq!(laid.len(), rows.len());

    // No stashes, or ones that are not commits: the history as it was.
    assert_eq!(with_stashes(&repo, plain.clone(), &[]), plain);
    assert_eq!(with_stashes(&repo, plain.clone(), &[Oid::zero()]), plain);
    // Given twice, it is one row.
    assert_eq!(
        with_stashes(&repo, plain.clone(), &[made, made]).len(),
        rows.len()
    );
}

#[test]
fn rows_go_by_date_but_never_below_their_parent() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let tip: Oid = git.rev_parse("HEAD").unwrap().parse().unwrap();
    let old: Oid = git.rev_parse("HEAD~3").unwrap().parse().unwrap();

    // A stash made long ago, by its date, on the newest commit.
    let ancient = stash(&scratch, &git, "one.txt", "946684800 +0000");
    // A stash made on an old commit, dated after everything.
    git.run(["switch", "-q", "--detach", "HEAD~3"]).unwrap();
    let late = stash(&scratch, &git, "two.txt", "2000000000 +0000");
    git.run(["switch", "-q", "main"]).unwrap();

    let repo = open(&scratch);
    let rows = with_stashes(&repo, all(&repo), &[late, ancient]);
    // The late one is first; the ancient one sits right above its parent,
    // which its date alone would have put it far below.
    assert_eq!(rows[0].id, late);
    assert_eq!(rows[0].parents, [old]);
    assert_eq!(row_of(&rows, ancient) + 1, row_of(&rows, tip));
    for (ix, row) in rows.iter().enumerate() {
        for parent in &row.parents {
            assert!(row_of(&rows, *parent) > ix, "a parent above its child");
        }
    }
    // The order they are given in does not matter.
    assert_eq!(with_stashes(&repo, all(&repo), &[ancient, late]), rows);
}

#[test]
fn a_stash_whose_commit_is_not_shown_is_left_out() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    // A stash on a commit that only a side branch reaches.
    git.run(["switch", "-q", "--orphan", "island"]).unwrap();
    scratch.write("island.txt", "alone\n").unwrap();
    git.run(["add", "island.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "island"]).unwrap();
    let on_island = stash(&scratch, &git, "island.txt", "2000000000 +0000");
    git.run(["switch", "-q", "main"]).unwrap();
    let on_main = stash(&scratch, &git, "main-stash.txt", "2000000001 +0000");

    let repo = open(&scratch);
    let head_only = walk(
        &repo,
        &LogOptions {
            tips: Tips::Head,
            ..LogOptions::default()
        },
    )
    .unwrap();
    let rows = with_stashes(&repo, head_only.clone(), &[on_island, on_main]);
    assert_eq!(rows.len(), head_only.len() + 1);
    assert_eq!(rows[0].id, on_main);
    assert!(rows.iter().all(|row| row.id != on_island));

    // With every branch shown, both are there.
    let rows = with_stashes(&repo, all(&repo), &[on_island, on_main]);
    assert!(rows.iter().any(|row| row.id == on_island));
    assert!(rows.iter().any(|row| row.id == on_main));

    // A branch on the stash commit itself: the walk finds it with all its
    // parents, and it is still one row.
    git.run(["branch", "at-stash", &on_main.to_hex()]).unwrap();
    let repo = open(&scratch);
    let rows = with_stashes(&repo, all(&repo), &[on_main]);
    assert_eq!(rows.iter().filter(|row| row.id == on_main).count(), 1);
    assert!(rows[row_of(&rows, on_main)].parents.len() > 1);
}

/// A history handed out in pieces: the rows of a piece, stashes and all,
/// are the first rows of the whole.
#[test]
fn a_piece_ends_above_where_a_stash_could_still_come_in() {
    use bitbraid_git::pieces::hold_back_for_stashes;

    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let times: Vec<i64> = git
        .lines(["log", "--format=%ct"])
        .unwrap()
        .iter()
        .map(|line| line.parse().unwrap())
        .collect();
    assert_eq!(times.len(), 5);
    // Made on the newest commit long ago, by its date: it stands right
    // above that commit.
    let on_tip = stash(&scratch, &git, "one.txt", "946684800 +0000");
    // Made on the first commit, dated between the second and the third
    // newest: it goes between those two.
    git.run(["switch", "-q", "--detach", "HEAD~4"]).unwrap();
    let between = format!("{} +0000", (times[1] + times[2]) / 2);
    let mid = stash(&scratch, &git, "two.txt", &between);
    // Made on the second commit, dated after everything: the first row.
    git.run(["switch", "-q", "--detach", "main~3"]).unwrap();
    let late = stash(&scratch, &git, "three.txt", "2000000000 +0000");
    git.run(["switch", "-q", "main"]).unwrap();

    let repo = open(&scratch);
    let commits = all(&repo);
    let mut pieces_with_rows = 0;
    for stashes in [vec![on_tip, mid], vec![on_tip, mid, late], vec![late]] {
        let whole = with_stashes(&repo, commits.clone(), &stashes);
        assert_eq!(whole.len(), commits.len() + stashes.len());
        for size in 0..commits.len() {
            let mut piece = commits[..size].to_vec();
            hold_back_for_stashes(&repo, &mut piece, &stashes);
            assert_eq!(piece, commits[..piece.len()], "it is only ever cut");
            let rows = with_stashes(&repo, piece, &stashes);
            assert_eq!(
                rows,
                whole[..rows.len()],
                "{} stashes, {size} walked",
                stashes.len()
            );
            pieces_with_rows += usize::from(!rows.is_empty());
        }
    }
    // With the stash on the first commit open, a piece ends above the row
    // it would go to; with it and the one on the newest commit only, the
    // two newest commits and that stash are certain.
    let mut piece = commits[..4].to_vec();
    hold_back_for_stashes(&repo, &mut piece, &[on_tip, mid]);
    assert_eq!(piece, commits[..2]);
    assert!(pieces_with_rows >= 5, "{pieces_with_rows}");
    // Nothing to hold back for: no stashes, ones that are no commits, a
    // piece that holds the commits of all of them.
    let mut piece = commits[..3].to_vec();
    hold_back_for_stashes(&repo, &mut piece, &[]);
    hold_back_for_stashes(&repo, &mut piece, &[Oid::zero()]);
    hold_back_for_stashes(&repo, &mut piece, &[on_tip]);
    assert_eq!(piece, commits[..3]);
    let mut none: Vec<GraphCommit> = Vec::new();
    hold_back_for_stashes(&repo, &mut none, &[on_tip, mid, late]);
    assert!(none.is_empty());
}
