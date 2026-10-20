//! Performance budgets on a large synthetic repository. Ignored by default;
//! run with `cargo xtask bench [--commits N]`.
#![allow(clippy::print_stdout)]

use std::time::Instant;

use bitbraid_git::{LogOptions, Repository, details, layout, snapshot, status, walk};
use bitbraid_testkit::{CorpusSpec, Git, build_synthetic, build_wide, fixtures_out_dir};

#[test]
#[ignore = "slow; run via cargo xtask bench"]
fn walk_and_layout_on_synthetic_corpus() {
    let commits: u64 = std::env::var("BITBRAID_CORPUS_COMMITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);
    let spec = CorpusSpec {
        commits,
        ..Default::default()
    };
    let t = Instant::now();
    let dir = build_synthetic(&spec).unwrap();
    println!(
        "BENCH corpus build_or_reuse commits={commits} took={:?}",
        t.elapsed()
    );

    let t = Instant::now();
    let repo = Repository::open(&dir).unwrap();
    let snap = snapshot(&repo).unwrap();
    println!(
        "BENCH snapshot branches={} took={:?}",
        snap.branches.len(),
        t.elapsed()
    );

    let t = Instant::now();
    let walked = walk(&repo, &LogOptions::default()).unwrap();
    let walk_time = t.elapsed();
    println!("BENCH walk commits={} took={:?}", walked.len(), walk_time);
    assert_eq!(walked.len() as u64, commits);

    let t = Instant::now();
    let rows = layout(&walked);
    let layout_time = t.elapsed();
    let width = rows.iter().map(|r| r.width).max().unwrap_or(0);
    println!(
        "BENCH layout rows={} max_width={width} took={:?}",
        rows.len(),
        layout_time
    );
    println!("BENCH total walk+layout took={:?}", walk_time + layout_time);

    // The same with two branches pinned to the left: the last two by
    // name, so that the lanes do move.
    let pins: Vec<_> = snap
        .branches
        .iter()
        .rev()
        .take(2)
        .map(|b| b.target)
        .collect();
    let t = Instant::now();
    let pinned = bitbraid_git::layout_pinned(&walked, &pins);
    let pinned_time = t.elapsed();
    let moved = rows
        .iter()
        .zip(&pinned)
        .filter(|(a, b)| a.lane != b.lane)
        .count();
    println!(
        "BENCH layout pinned={} rows={} moved={moved} max_width={} took={:?}",
        pins.len(),
        pinned.len(),
        pinned.iter().map(|r| r.width).max().unwrap_or(0),
        pinned_time
    );

    let t = Instant::now();
    let window: Vec<_> = walked.iter().take(200).map(|c| c.id).collect();
    let decoded = details(&repo, &window).unwrap();
    println!(
        "BENCH details window={} took={:?}",
        decoded.len(),
        t.elapsed()
    );
}

/// A large history handed out in pieces: how soon the first rows are
/// there, what the pieces cost on top of the walk, and that every piece is
/// the beginning of the whole.
#[test]
#[ignore = "slow; run via cargo xtask bench"]
fn pieces_on_synthetic_corpus() {
    use bitbraid_git::{layout_piece, pieces};
    let commits: u64 = std::env::var("BITBRAID_CORPUS_COMMITS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200_000);
    let spec = CorpusSpec {
        commits,
        ..Default::default()
    };
    let dir = build_synthetic(&spec).unwrap();
    let repo = Repository::open(&dir).unwrap();
    let options = LogOptions::default();

    let t = Instant::now();
    let first = pieces::first(&repo, &options, pieces::FIRST_PIECE).unwrap();
    let first_time = t.elapsed();
    let pieces::First::Piece { rows, walked } = first else {
        println!("BENCH pieces: {commits} commits are one piece (took {first_time:?})");
        return;
    };
    let t = Instant::now();
    let (_, laid) = layout_piece(&rows, None, &[]);
    println!(
        "BENCH pieces first rows={} walked={walked} walk={first_time:?} layout={:?}",
        rows.len(),
        t.elapsed()
    );
    assert_eq!(laid.len(), rows.len());

    let t = Instant::now();
    let whole = walk(&repo, &options).unwrap();
    let whole_time = t.elapsed();
    assert_eq!(
        rows,
        whole[..rows.len()],
        "the first piece begins the whole"
    );

    let t = Instant::now();
    let mut handed: Vec<(usize, usize, std::time::Duration)> = Vec::new();
    let all = pieces::walk_in_pieces(
        &repo,
        &options,
        pieces::FIRST_PIECE,
        &|| true,
        &mut |rows, walked| {
            assert_eq!(rows, whole[..rows.len()], "a piece begins the whole");
            handed.push((rows.len(), walked, t.elapsed()));
        },
    )
    .unwrap()
    .unwrap();
    let pieces_time = t.elapsed();
    assert_eq!(all, whole);
    for (rows, walked, at) in &handed {
        println!("BENCH pieces piece rows={rows} walked={walked} at={at:?}");
    }
    println!(
        "BENCH pieces whole commits={} in_one_go={whole_time:?} in_pieces={pieces_time:?} \
         (the checks against the whole included)",
        whole.len()
    );
}

#[test]
#[ignore = "slow; run via cargo xtask bench"]
fn status_on_wide_worktree() {
    let files: u64 = std::env::var("BITBRAID_WIDE_FILES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let t = Instant::now();
    let dir = build_wide(files).unwrap();
    println!(
        "BENCH wide build_or_reuse files={files} took={:?}",
        t.elapsed()
    );
    let repo = Repository::open(&dir).unwrap();
    let t = Instant::now();
    let cold = status(&repo).unwrap();
    println!(
        "BENCH status cold entries={} took={:?}",
        cold.len(),
        t.elapsed()
    );
    let t = Instant::now();
    let warm = status(&repo).unwrap();
    println!(
        "BENCH status warm entries={} took={:?}",
        warm.len(),
        t.elapsed()
    );
    assert!(warm.is_empty(), "a fresh checkout is clean");
}

#[test]
#[ignore = "needs a clone of git.git; run via cargo xtask bench"]
fn walk_and_layout_on_git_git() {
    let Some(path) = std::env::var_os("BITBRAID_GIT_GIT") else {
        println!("BENCH git.git skipped (set BITBRAID_GIT_GIT to a bare clone)");
        return;
    };
    let repo = Repository::open(&path).unwrap();
    let t = Instant::now();
    let walked = walk(&repo, &LogOptions::default()).unwrap();
    let walk_time = t.elapsed();
    let t = Instant::now();
    let rows = layout(&walked);
    let layout_time = t.elapsed();
    let width = rows.iter().map(|r| r.width).max().unwrap_or(0);
    println!(
        "BENCH git.git commits={} walk={walk_time:?} layout={layout_time:?} max_width={width} total={:?}",
        walked.len(),
        walk_time + layout_time
    );
}

#[test]
#[ignore = "needs a clone of git.git; run via cargo xtask bench"]
fn git_git_order_and_refs_match_git() {
    use std::collections::{BTreeMap, BTreeSet};
    let Some(path) = std::env::var_os("BITBRAID_GIT_GIT") else {
        println!("BENCH git.git oracle skipped (set BITBRAID_GIT_GIT to a bare clone)");
        return;
    };
    let path = std::path::PathBuf::from(path);
    let repo = Repository::open(&path).unwrap();
    let git = Git::new(&path, fixtures_out_dir().join(".home"));

    // References.
    let snap = snapshot(&repo).unwrap();
    let want: BTreeMap<String, String> = git
        .lines([
            "for-each-ref",
            "--format=%(refname:short) %(objectname)",
            "refs/heads",
        ])
        .unwrap()
        .into_iter()
        .map(|l| {
            let (n, id) = l.split_once(' ').unwrap();
            (n.to_owned(), id.to_owned())
        })
        .collect();
    let got: BTreeMap<String, String> = snap
        .branches
        .iter()
        .map(|b| (b.name.clone(), b.target.to_hex()))
        .collect();
    assert_eq!(got, want, "git.git branches");
    let want_tags = git
        .lines(["for-each-ref", "--format=%(refname:short)", "refs/tags"])
        .unwrap()
        .len();
    assert_eq!(snap.tags.len(), want_tags, "git.git tag count");

    // Commit set, parents and ordering.
    let walked = walk(&repo, &LogOptions::default()).unwrap();
    let want_order = git.lines(["rev-list", "--all", "--date-order"]).unwrap();
    let want_set: BTreeSet<&str> = want_order.iter().map(String::as_str).collect();
    let got_order: Vec<String> = walked.iter().map(|c| c.id.to_hex()).collect();
    let got_set: BTreeSet<&str> = got_order.iter().map(String::as_str).collect();
    assert_eq!(got_set.len(), want_set.len(), "git.git commit count");
    assert_eq!(got_set, want_set, "git.git commit set");
    let mut seen = BTreeSet::new();
    for c in &walked {
        for p in &c.parents {
            assert!(!seen.contains(p), "parent {p} before child {}", c.id);
        }
        seen.insert(c.id);
    }
    let differing = got_order
        .iter()
        .zip(&want_order)
        .filter(|(a, b)| a != b)
        .count();
    println!(
        "BENCH git.git oracle commits={} positions_differing_from_git_date_order={differing} ({:.2}%)",
        walked.len(),
        differing as f64 * 100.0 / walked.len().max(1) as f64
    );
    // Ties in commit time are ordered differently by gix and git; a small
    // disagreement is expected, a large one would indicate a real bug.
    assert!(
        differing * 100 < walked.len() * 5,
        "more than 5% of positions differ from git"
    );
}

/// Time every stage of opening a real repository, the way the app does it.
/// `BITBRAID_BENCH_REPO=<path> cargo test -p bitbraid-git --test suite --
/// --ignored --nocapture bench_corpus::open_stages`.
#[test]
#[ignore = "needs BITBRAID_BENCH_REPO; run by hand"]
fn open_stages_on_a_real_repository() {
    let Some(path) = std::env::var_os("BITBRAID_BENCH_REPO") else {
        println!("BENCH open stages skipped (set BITBRAID_BENCH_REPO to a repository)");
        return;
    };
    let total = Instant::now();
    let t = Instant::now();
    let repo = Repository::open(&path).unwrap();
    println!("BENCH stage open took={:?}", t.elapsed());
    let t = Instant::now();
    let _summary = repo.summary().unwrap();
    println!("BENCH stage summary took={:?}", t.elapsed());
    // The watcher returns at once; its first change set says the watches stand.
    let t = Instant::now();
    let watch_started = t;
    let (watching_tx, watching_rx) = std::sync::mpsc::channel();
    let watcher = bitbraid_git::Watcher::spawn(
        repo.git_dir(),
        repo.workdir(),
        std::time::Duration::from_millis(250),
        move |_| {
            let _ = watching_tx.send(Instant::now());
        },
    );
    println!(
        "BENCH stage watcher spawn ok={} took={:?}",
        watcher.is_ok(),
        t.elapsed()
    );
    let t = Instant::now();
    let entries = status(&repo).unwrap();
    println!(
        "BENCH stage status entries={} took={:?}",
        entries.len(),
        t.elapsed()
    );
    let t = Instant::now();
    let snap = snapshot(&repo).unwrap();
    println!(
        "BENCH stage snapshot branches={} remote_branches={} tags={} stashes={} worktrees={} took={:?}",
        snap.branches.len(),
        snap.remotes.iter().map(|r| r.branches.len()).sum::<usize>(),
        snap.tags.len(),
        snap.stashes.len(),
        snap.worktrees.len(),
        t.elapsed()
    );
    let t = Instant::now();
    let walked = walk(&repo, &LogOptions::default()).unwrap();
    println!(
        "BENCH stage walk commits={} took={:?}",
        walked.len(),
        t.elapsed()
    );
    let t = Instant::now();
    let rows = layout(&walked);
    let width = rows.iter().map(|r| r.width).max().unwrap_or(0);
    println!(
        "BENCH stage layout max_width={width} took={:?}",
        t.elapsed()
    );
    let window: Vec<_> = walked.iter().take(40).map(|c| c.id).collect();
    let t = Instant::now();
    let decoded = details(&repo, &window).unwrap();
    println!(
        "BENCH stage details window={} took={:?}",
        decoded.len(),
        t.elapsed()
    );
    println!("BENCH stage total took={:?}", total.elapsed());
    match watching_rx.recv_timeout(std::time::Duration::from_secs(60)) {
        Ok(at) => println!(
            "BENCH stage watcher watching after={:?} (in the background)",
            at.duration_since(watch_started)
        ),
        Err(_) => println!("BENCH stage watcher never reported"),
    }
}

/// What a key typed into the graph's search costs on a million commits,
/// for the part that asks every commit whether its id begins with what
/// was typed, two ways: each id written out as text and compared, and the
/// digits compared in place. `cargo test --release -p
/// bitbraid-git --test suite -- --ignored --nocapture
/// bench_corpus::id_matching`.
#[test]
#[ignore = "slow; run by hand"]
fn id_matching_on_synthetic_corpus() {
    let dir = build_synthetic(&CorpusSpec::default()).expect("corpus");
    let repo = Repository::open(&dir).unwrap();
    let commits = walk(&repo, &LogOptions::default()).unwrap();
    // A word that is the beginning of some ids, and one that is of none.
    for typed in ["a", "abc", "abc1234", "merge"] {
        let needle = typed.to_lowercase();
        let mut written = (0usize, std::time::Duration::MAX);
        let mut in_place = (0usize, std::time::Duration::MAX);
        // The best of five: other things run on this machine.
        for _ in 0..5 {
            let t = Instant::now();
            let found = commits
                .iter()
                .filter(|commit| commit.id.to_hex().starts_with(&needle))
                .count();
            written = (found, written.1.min(t.elapsed()));
            let t = Instant::now();
            let digits = bitbraid_git::oid::hex_digits(&needle);
            let found = commits
                .iter()
                .filter(|commit| {
                    digits
                        .as_deref()
                        .is_some_and(|digits| commit.id.begins_with(digits))
                })
                .count();
            in_place = (found, in_place.1.min(t.elapsed()));
        }
        assert_eq!(written.0, in_place.0, "{typed}: the two ways agree");
        println!(
            "BENCH id matching {typed:?} commits={} found={} written_out={:?} in_place={:?}",
            commits.len(),
            in_place.0,
            written.1,
            in_place.1
        );
    }
}

/// Time the plain history of some files of a real repository against
/// `git log -- <path>`, and check that the two list the same commits.
/// `BITBRAID_BENCH_REPO=<path> [BITBRAID_BENCH_FILES=a,b,c] cargo test
/// --release -p bitbraid-git --test suite -- --ignored --nocapture
/// bench_corpus::file_history_on`. Without the list, 24 files spread over
/// what HEAD holds.
#[test]
#[ignore = "needs BITBRAID_BENCH_REPO; run by hand"]
fn file_history_on_a_real_repository() {
    use bitbraid_git::{HistoryOptions, file_history};

    let Some(path) = std::env::var_os("BITBRAID_BENCH_REPO") else {
        println!("BENCH file history skipped (set BITBRAID_BENCH_REPO to a repository)");
        return;
    };
    let path = std::path::PathBuf::from(path);
    let git = Git::new(&path, fixtures_out_dir().join(".home"));
    let files: Vec<String> = match std::env::var("BITBRAID_BENCH_FILES") {
        Ok(listed) => listed.split(',').map(str::to_owned).collect(),
        Err(_) => {
            let all = git.lines(["ls-tree", "-r", "--name-only", "HEAD"]).unwrap();
            let step = (all.len() / 24).max(1);
            all.into_iter().step_by(step).take(24).collect()
        }
    };
    let repo = Repository::open(&path).unwrap();
    let plain = HistoryOptions {
        follow: false,
        ..Default::default()
    };
    let (mut ours, mut gits) = (std::time::Duration::ZERO, std::time::Duration::ZERO);
    let (mut listed, mut same) = (0usize, 0usize);
    for file in &files {
        let t = Instant::now();
        let got = file_history(&repo, file, &plain).unwrap();
        let took = t.elapsed();
        ours += took;
        let t = Instant::now();
        let want = git
            .lines(["log", "--format=%H", "HEAD", "--", file.as_str()])
            .unwrap();
        let git_took = t.elapsed();
        gits += git_took;
        listed += got.len();
        let got: Vec<String> = got.iter().map(|entry| entry.commit.id.to_hex()).collect();
        same += usize::from(got == want);
        println!(
            "BENCH file history {file}: {} commits, ours {took:?}, git {git_took:?}",
            got.len()
        );
    }
    println!(
        "BENCH file history files={} commits={listed} as_git={same} ours={ours:?} git={gits:?} ratio={:.2}",
        files.len(),
        ours.as_secs_f64() / gits.as_secs_f64().max(1e-9)
    );
    assert_eq!(same, files.len(), "a history differs from git's");
}
