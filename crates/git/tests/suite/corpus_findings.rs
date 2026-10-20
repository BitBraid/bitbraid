//! Cases the corpus run (`cargo xtask corpus`, `tests/suite/corpus.rs`)
//! met on real repositories, each as a small history made here, so that
//! they are tested without a network:
//!
//! - a symbolic link is diffed as a link, in the worktree too, and not as
//!   nothing or as the file it leads to (git.git, `git-subtree`);
//! - renames are found among more than about thirty files moved and
//!   edited in one commit (cargo, the compiler moved a directory up);
//! - a subject ends where the line between it and the body holds spaces
//!   (git.git, 2005 and 2006);
//! - file history leaves out what a merge did not take, and merges when it
//!   follows renames, as git does (cargo);
//! - the graph's order with a parent dated after its child is held to
//!   `--date-order`.

use std::collections::BTreeSet;

use bitbraid_git::{
    DiffOptions, DiffSide, HistoryOptions, LineKind, LogOptions, Oid, Repository, Tips,
    apply_hunks, diff_file, diff_trees, file_history, log, walk,
};
use bitbraid_testkit::{Git, Scratch, oracle::CLOCK_EPOCH};

/// A commit of everything, dated `day` days into the fixture clock.
fn commit_on_day(git: &Git, message: &str, day: i64) {
    let date = format!("{} +0000", CLOCK_EPOCH + 400 * 86_400 + day * 86_400);
    git.run(["add", "-A"]).unwrap();
    git.run_with(
        ["commit", "-q", "--allow-empty", "-m", message],
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    )
    .unwrap();
}

/// Hunks as git prints them, headers cut after the ranges.
fn hunk_text(hunks: &[bitbraid_git::Hunk]) -> Vec<String> {
    let mut out = Vec::new();
    for hunk in hunks {
        out.push(hunk.header());
        for line in &hunk.lines {
            let mark = match line.kind {
                LineKind::Context => ' ',
                LineKind::Added => '+',
                LineKind::Removed => '-',
            };
            out.push(format!("{mark}{}", line.text));
            if line.no_newline {
                out.push("\\ No newline at end of file".to_owned());
            }
        }
    }
    out
}

fn git_hunks(git: &Git, args: &[&str]) -> Vec<String> {
    let mut command = vec!["diff", "--no-color", "--no-ext-diff"];
    command.extend_from_slice(args);
    let text = git.run(command).unwrap();
    text.strip_suffix('\n')
        .unwrap_or(&text)
        .split('\n')
        .skip_while(|line| !line.starts_with("@@"))
        .map(|line| match line.strip_prefix("@@") {
            Some(rest) => match rest.split_once("@@") {
                Some((ranges, _)) => format!("@@{ranges}@@"),
                None => line.to_owned(),
            },
            None => line.to_owned(),
        })
        .collect()
}

#[test]
fn a_symbolic_link_is_compared_by_where_it_points() {
    let symlink = |to: &str, link: &std::path::Path| bitbraid_testkit::symlink_file(to, link);
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    // Git for Windows keeps a link as one where it is told it may.
    if cfg!(windows) {
        git.run(["config", "core.symlinks", "true"]).unwrap();
    }
    let link = scratch.path().join("link");
    symlink("a.txt", &link).unwrap();
    commit_on_day(&git, "add a link", 1);
    std::fs::remove_file(&link).unwrap();
    symlink("b.txt", &link).unwrap();
    commit_on_day(&git, "point it elsewhere", 2);
    let repo = Repository::open(scratch.path()).unwrap();
    let side = |rev: &str| DiffSide::parse(&repo, rev).unwrap();

    // Between commits: one line out, one line in, neither with an end.
    for (old, new) in [("HEAD~1", "HEAD"), ("HEAD~2", "HEAD~1"), ("HEAD", "HEAD~2")] {
        let files = diff_trees(&repo, &side(old), &side(new), &DiffOptions::default()).unwrap();
        let file = files.iter().find(|f| f.path == "link").expect("the link");
        assert!(!file.binary);
        assert_eq!(
            hunk_text(&file.hunks),
            git_hunks(&git, &[old, new, "--", "link"]),
            "{old}..{new}"
        );
        let numstat = git
            .run(["diff", "--numstat", old, new, "--", "link"])
            .unwrap();
        assert_eq!(
            numstat,
            format!("{}\t{}\tlink\n", file.additions, file.deletions),
            "{old}..{new}"
        );
        // And one path alone gives the same.
        let one = diff_file(&repo, &side(old), &side(new), "link", 3)
            .unwrap()
            .expect("the link");
        assert_eq!(hunk_text(&one.hunks), hunk_text(&file.hunks));
    }
    let files = diff_trees(
        &repo,
        &side("HEAD~1"),
        &side("HEAD"),
        &DiffOptions::default(),
    )
    .unwrap();
    let file = files.iter().find(|f| f.path == "link").unwrap();
    assert_eq!(apply_hunks("a.txt", &file.hunks).as_deref(), Some("b.txt"));
    assert_eq!(
        (file.old_mode.as_deref(), file.new_mode.as_deref()),
        (Some("120000"), Some("120000"))
    );

    // In the worktree: where it points now, not what is in the file there.
    std::fs::remove_file(&link).unwrap();
    symlink("README.md", &link).unwrap();
    let unstaged = diff_file(&repo, &DiffSide::Index, &DiffSide::Worktree, "link", 3)
        .unwrap()
        .expect("the link changed");
    assert_eq!(hunk_text(&unstaged.hunks), git_hunks(&git, &["--", "link"]));
    assert_eq!(
        hunk_text(&unstaged.hunks),
        [
            "@@ -1 +1 @@",
            "-b.txt",
            "\\ No newline at end of file",
            "+README.md",
            "\\ No newline at end of file"
        ]
    );
    // One that leads nowhere is still a link that says something.
    std::fs::remove_file(&link).unwrap();
    symlink("nowhere/at all", &link).unwrap();
    let dangling = diff_file(&repo, &DiffSide::Index, &DiffSide::Worktree, "link", 3)
        .unwrap()
        .expect("the link changed");
    assert_eq!(hunk_text(&dangling.hunks), git_hunks(&git, &["--", "link"]));
    // And one that leads to a directory is not that directory.
    std::fs::remove_file(&link).unwrap();
    symlink(".", &link).unwrap();
    let to_dir = diff_file(&repo, &DiffSide::Index, &DiffSide::Worktree, "link", 3)
        .unwrap()
        .expect("the link changed");
    assert_eq!(hunk_text(&to_dir.hunks), git_hunks(&git, &["--", "link"]));
}

#[test]
fn renames_are_found_among_many_files_moved_and_edited() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    // Forty against forty is more pairs than gitoxide compares unasked.
    let body = |n: usize| {
        (0..14)
            .map(|line| format!("file {n} line {line}: {}\n", "x".repeat(n % 7 + 3)))
            .collect::<String>()
    };
    for n in 0..40 {
        scratch
            .write(&format!("old/dir/f{n}.txt"), &body(n))
            .unwrap();
    }
    commit_on_day(&git, "forty files", 1);
    for n in 0..40 {
        let text = body(n).replace("line 5:", "line five:");
        std::fs::remove_file(scratch.path().join(format!("old/dir/f{n}.txt"))).unwrap();
        scratch.write(&format!("new/f{n}.txt"), &text).unwrap();
    }
    commit_on_day(&git, "moved and edited", 2);

    let repo = Repository::open(scratch.path()).unwrap();
    let side = |rev: &str| DiffSide::parse(&repo, rev).unwrap();
    let files = diff_trees(
        &repo,
        &side("HEAD~1"),
        &side("HEAD"),
        &DiffOptions::default(),
    )
    .unwrap();
    let ours: BTreeSet<(String, String)> = files
        .iter()
        .filter(|f| f.kind == bitbraid_git::diff::FileChangeKind::Renamed)
        .map(|f| (f.old_path.clone().unwrap(), f.path.clone()))
        .collect();
    let status = git
        .run([
            "diff-tree",
            "-r",
            "-M",
            "--name-status",
            "-z",
            "HEAD~1",
            "HEAD",
        ])
        .unwrap();
    let mut parts = status.split('\0').filter(|part| !part.is_empty());
    let mut gits: BTreeSet<(String, String)> = BTreeSet::new();
    while let Some(letter) = parts.next() {
        assert!(letter.starts_with('R'), "git pairs every file: {letter}");
        gits.insert((
            parts.next().unwrap().to_owned(),
            parts.next().unwrap().to_owned(),
        ));
    }
    assert_eq!(gits.len(), 40);
    assert_eq!(ours, gits);
    assert_eq!(files.len(), 40, "nothing is left over as added or deleted");
    for file in &files {
        assert_eq!((file.additions, file.deletions), (1, 1), "{}", file.path);
    }

    // The history of one of them goes on under its old name.
    let followed = file_history(&repo, "new/f7.txt", &HistoryOptions::default()).unwrap();
    let want = git
        .lines(["log", "--follow", "--format=%H", "--", "new/f7.txt"])
        .unwrap();
    assert_eq!(want.len(), 2);
    assert_eq!(
        followed
            .iter()
            .map(|e| e.commit.id.to_hex())
            .collect::<Vec<_>>(),
        want
    );
    assert_eq!(followed[0].previous_path.as_deref(), Some("old/dir/f7.txt"));
}

#[test]
fn a_subject_ends_at_a_line_that_only_holds_spaces() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let messages = [
        "subject\n \nbody after a line of one space\n",
        "subject over\ntwo lines  \n\t\nbody after a line of a tab\n\nand more\n",
        "\n\n  leading blank lines and an indent\n\nbody\n",
        "no body at all",
        "Date:   Thu Apr 14 08:26:38 2005 +0200\nAuthor: Someone <someone@example.invalid>\n \n[patch] the real subject\n",
    ];
    for message in messages {
        let file = scratch.path().join(".git/corpus-message");
        std::fs::write(&file, message).unwrap();
        git.run([
            "commit".as_ref(),
            "-q".as_ref(),
            "--allow-empty".as_ref(),
            "--cleanup=verbatim".as_ref(),
            "-F".as_ref(),
            file.as_os_str(),
        ])
        .unwrap();
        let repo = Repository::open(scratch.path()).unwrap();
        let head = log(
            &repo,
            &LogOptions {
                tips: Tips::Head,
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        let subject = git.run(["log", "-1", "--format=%s"]).unwrap();
        let body = git.run(["log", "-1", "--format=%b"]).unwrap();
        assert_eq!(
            head[0].summary,
            subject.strip_suffix('\n').unwrap(),
            "{message:?}"
        );
        assert_eq!(head[0].body, body.trim(), "{message:?}");
    }
}

/// The history the two walks differ on: a side branch that changes the
/// file and takes the change back, a merge that keeps the file as it was,
/// then a merge that leaves the file unlike both its parents.
fn history_with_merges() -> (Scratch, Git) {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch.write("f.txt", "one\ntwo\nthree\n").unwrap();
    commit_on_day(&git, "add f", 1);
    git.run(["checkout", "-q", "-b", "side"]).unwrap();
    scratch.write("f.txt", "one\nTWO\nthree\n").unwrap();
    commit_on_day(&git, "side: change f", 2);
    scratch.write("f.txt", "one\ntwo\nthree\n").unwrap();
    commit_on_day(&git, "side: take it back", 3);
    git.run(["checkout", "-q", "main"]).unwrap();
    scratch.write("other.txt", "other\n").unwrap();
    commit_on_day(&git, "main: something else", 4);
    let day = |n: i64| format!("{} +0000", CLOCK_EPOCH + (400 + n) * 86_400);
    git.run_with(
        [
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "merge side: f as it was",
            "side",
        ],
        &[
            ("GIT_AUTHOR_DATE", &day(5)),
            ("GIT_COMMITTER_DATE", &day(5)),
        ],
    )
    .unwrap();
    // Both sides change the same line; the merge writes a third thing.
    git.run(["checkout", "-q", "-b", "left"]).unwrap();
    scratch.write("f.txt", "one\nleft\nthree\n").unwrap();
    commit_on_day(&git, "left: change f", 6);
    git.run(["checkout", "-q", "main"]).unwrap();
    scratch.write("f.txt", "one\nright\nthree\n").unwrap();
    commit_on_day(&git, "main: change f", 7);
    let _ = git.run(["merge", "-q", "--no-ff", "left"]); // stops, by design
    scratch.write("f.txt", "one\nboth\nthree\n").unwrap();
    git.run(["add", "f.txt"]).unwrap();
    git.run_with(
        ["commit", "-q", "-m", "merge left: f unlike either"],
        &[
            ("GIT_AUTHOR_DATE", &day(8)),
            ("GIT_COMMITTER_DATE", &day(8)),
        ],
    )
    .unwrap();
    git.run(["mv", "f.txt", "g.txt"]).unwrap();
    commit_on_day(&git, "rename f to g", 9);
    scratch.write("g.txt", "one\nboth\nthree\nfour\n").unwrap();
    commit_on_day(&git, "change g", 10);
    (scratch, git)
}

#[test]
fn file_history_is_git_log_for_the_path() {
    let (scratch, git) = history_with_merges();
    let repo = Repository::open(scratch.path()).unwrap();
    let plain = HistoryOptions {
        follow: false,
        ..Default::default()
    };
    let subjects = |entries: &[bitbraid_git::HistoryEntry]| -> Vec<String> {
        entries.iter().map(|e| e.commit.summary.clone()).collect()
    };
    for path in ["g.txt", "f.txt", "other.txt", "a.txt", "nothing-here.txt"] {
        let ours = file_history(&repo, path, &plain).unwrap();
        let want = git.lines(["log", "--format=%H", "--", path]).unwrap();
        assert_eq!(
            ours.iter()
                .map(|e| e.commit.id.to_hex())
                .collect::<Vec<_>>(),
            want,
            "{path}: {:?}",
            subjects(&ours)
        );
    }
    // What that means here: the merge that kept f is not listed, nor what
    // the side it merged did and undid; the merge that wrote f anew is.
    let f = file_history(&repo, "f.txt", &plain).unwrap();
    assert_eq!(
        subjects(&f),
        [
            "rename f to g",
            "merge left: f unlike either",
            "main: change f",
            "left: change f",
            "add f"
        ]
    );
    assert_eq!(f[0].kind, bitbraid_git::history::HistoryKind::Deleted);
    assert_eq!(f[1].kind, bitbraid_git::history::HistoryKind::Modified);
}

/// Both histories of every path, against git's, in the repository as it
/// is now.
fn histories_are_gits(scratch: &Scratch, git: &Git, paths: &[&str], when: &str) {
    // Opened anew: what a repository knows of a commit-graph file it
    // knows from when it was opened.
    let repo = Repository::open(scratch.path()).unwrap();
    let plain = HistoryOptions {
        follow: false,
        ..Default::default()
    };
    for path in paths {
        for (options, flags) in [
            (&plain, &["log", "--format=%H"][..]),
            (
                &HistoryOptions::default(),
                &["log", "--follow", "--format=%H"][..],
            ),
        ] {
            // Git follows one file, not a directory.
            if options.follow && (path.ends_with('/') || !path.contains('.')) {
                continue;
            }
            let ours: Vec<String> = file_history(&repo, path, options)
                .unwrap()
                .iter()
                .map(|entry| entry.commit.id.to_hex())
                .collect();
            let mut asked: Vec<&str> = flags.to_vec();
            asked.extend(["--", path]);
            let want = git.lines(asked).unwrap();
            assert_eq!(ours, want, "{when}: {path}, follow {}", options.follow);
        }
    }
}

#[test]
fn file_history_is_the_same_with_a_commit_graph_and_where_none_may_be_used() {
    let (scratch, git) = history_with_merges();
    // Files some directories down, changed by some commits and left alone
    // by others, on a branch that is merged and on the trunk: what the
    // walk remembers of a tree is what it read of that tree.
    scratch.write("deep/er/still/x.txt", "x\n").unwrap();
    scratch.write("deep/er/y.txt", "y\n").unwrap();
    scratch.write("deep/z.txt", "z\n").unwrap();
    commit_on_day(&git, "three files, deep", 11);
    git.run(["checkout", "-q", "-b", "deeper"]).unwrap();
    scratch.write("deep/er/still/x.txt", "x\nmore\n").unwrap();
    commit_on_day(&git, "deeper: change x", 12);
    scratch.write("deep/z.txt", "z\nmore\n").unwrap();
    commit_on_day(&git, "deeper: change z", 13);
    git.run(["checkout", "-q", "main"]).unwrap();
    scratch.write("deep/er/y.txt", "y\nmore\n").unwrap();
    commit_on_day(&git, "main: change y", 14);
    let day = format!("{} +0000", CLOCK_EPOCH + (400 + 15) * 86_400);
    git.run_with(
        ["merge", "-q", "--no-ff", "-m", "merge deeper", "deeper"],
        &[("GIT_AUTHOR_DATE", &day), ("GIT_COMMITTER_DATE", &day)],
    )
    .unwrap();
    // A file where a directory was, and the other way round.
    scratch.remove("deep/z.txt").unwrap();
    scratch.write("deep/z.txt/inside.txt", "inside\n").unwrap();
    commit_on_day(&git, "z becomes a directory", 16);

    let paths = [
        "g.txt",
        "f.txt",
        "other.txt",
        "deep/er/still/x.txt",
        "deep/er/y.txt",
        "deep/z.txt",
        "deep/z.txt/inside.txt",
        "deep/er",
        "deep/er/",
        "deep",
        "deep/nothing/here.txt",
        "g.txt/not-under-a-file.txt",
        "nothing-here.txt",
    ];
    histories_are_gits(&scratch, &git, &paths, "no commit-graph");

    // With the file git writes for itself: the commits are taken from it.
    git.run(["commit-graph", "write", "--reachable"]).unwrap();
    assert!(
        scratch
            .path()
            .join(".git/objects/info/commit-graph")
            .is_file()
    );
    histories_are_gits(&scratch, &git, &paths, "a commit-graph");

    // History goes on after the file was written: the new commits are not
    // in it, the old ones are.
    scratch
        .write("deep/er/still/x.txt", "x\nmore\nand more\n")
        .unwrap();
    commit_on_day(&git, "change x after the graph", 17);
    scratch
        .write("g.txt", "one\nboth\nthree\nfour\nfive\n")
        .unwrap();
    commit_on_day(&git, "change g after the graph", 18);
    histories_are_gits(
        &scratch,
        &git,
        &paths,
        "a commit-graph older than the history",
    );

    // A commit is replaced by one with another parent. The commit-graph
    // file was written without knowing of it, so git leaves the file
    // alone then: with the file and without it the histories are the same.
    let changed_y = git
        .lines(["log", "--format=%H", "--grep=main: change y"])
        .unwrap();
    let root = git.lines(["rev-list", "--max-parents=0", "HEAD"]).unwrap();
    git.run(["replace", "--graft", &changed_y[0], &root[0]])
        .unwrap();
    let ours = |scratch: &Scratch| -> Vec<Vec<String>> {
        let repo = Repository::open(scratch.path()).unwrap();
        paths
            .iter()
            .map(|path| {
                let plain = HistoryOptions {
                    follow: false,
                    ..Default::default()
                };
                file_history(&repo, path, &plain)
                    .unwrap()
                    .iter()
                    .map(|entry| entry.commit.id.to_hex())
                    .collect()
            })
            .collect()
    };
    let with_the_file = ours(&scratch);
    std::fs::remove_file(scratch.path().join(".git/objects/info/commit-graph")).unwrap();
    assert_eq!(with_the_file, ours(&scratch));
    assert!(with_the_file.iter().any(|history| !history.is_empty()));
    git.run(["replace", "-d", &changed_y[0]]).unwrap();
    histories_are_gits(&scratch, &git, &paths, "the replacement gone again");

    // A shallow clone ends where it was cut, and its histories are git's
    // there too, with a commit-graph file left over from before or not.
    let clone = scratch.path().join(".git").join("shallow-clone");
    let url = format!("file://{}", scratch.path().display());
    git.run([
        "clone",
        "-q",
        "--depth=4",
        url.as_str(),
        clone.to_string_lossy().as_ref(),
    ])
    .unwrap();
    let shallow = Git::new(&clone, git.home());
    let repo = Repository::open(&clone).unwrap();
    for path in ["g.txt", "deep/er/still/x.txt", "deep/er/y.txt", "deep"] {
        let plain = HistoryOptions {
            follow: false,
            ..Default::default()
        };
        let got: Vec<String> = file_history(&repo, path, &plain)
            .unwrap()
            .iter()
            .map(|entry| entry.commit.id.to_hex())
            .collect();
        let want = shallow.lines(["log", "--format=%H", "--", path]).unwrap();
        assert_eq!(got, want, "shallow: {path}");
    }
}

/// The author's date and the commit's date of a history run against each
/// other: work written long ago and committed late (a rebase, a patch by
/// mail), on two lines that are merged. By author date the order is
/// another than by commit date, and it is git's.
#[test]
fn the_author_date_order_is_gits_where_the_two_dates_disagree() {
    use bitbraid_git::{LogOptions, Order, Tips, walk};
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let at = |day: i64| format!("{} +0000", CLOCK_EPOCH + (400 + day) * 86_400);
    let commit = |message: &str, written: i64, committed: i64| {
        git.run_with(
            ["commit", "-q", "--allow-empty", "-m", message],
            &[
                ("GIT_AUTHOR_DATE", &at(written)),
                ("GIT_COMMITTER_DATE", &at(committed)),
            ],
        )
        .unwrap();
    };
    // On the trunk: written in one order, committed in the other.
    commit("trunk: written last, committed first", 30, 1);
    git.run(["checkout", "-q", "-b", "old-work"]).unwrap();
    commit("old work: written first, committed late", 2, 20);
    commit("old work: written early, committed later", 3, 21);
    git.run(["checkout", "-q", "main"]).unwrap();
    commit("trunk: written in between", 10, 22);
    commit("trunk: the same author date as another", 3, 23);
    git.run_with(
        ["merge", "-q", "--no-ff", "-m", "merge old work", "old-work"],
        &[("GIT_AUTHOR_DATE", &at(5)), ("GIT_COMMITTER_DATE", &at(24))],
    )
    .unwrap();
    git.run(["checkout", "-q", "-b", "later"]).unwrap();
    commit("later: written before the merge was", 4, 25);
    // Two more tips that nothing orders but their dates, which disagree.
    git.run(["checkout", "-q", "-b", "committed-last", "main"])
        .unwrap();
    commit("written first of all, committed last of all", 1, 40);
    git.run(["checkout", "-q", "-b", "committed-first", "main"])
        .unwrap();
    commit("written last of all, committed before that", 39, 30);

    let repo = Repository::open(scratch.path()).unwrap();
    let ids = |order: Order, tips: Tips| -> Vec<String> {
        walk(
            &repo,
            &LogOptions {
                tips,
                order,
                ..Default::default()
            },
        )
        .unwrap()
        .iter()
        .map(|commit| commit.id.to_hex())
        .collect()
    };
    let by_author = git
        .lines(["rev-list", "--all", "--author-date-order"])
        .unwrap();
    let by_date = git.lines(["rev-list", "--all", "--date-order"]).unwrap();
    assert_ne!(by_author, by_date, "the history tells the two orders apart");
    assert_eq!(ids(Order::AuthorDate, Tips::All), by_author);
    assert_eq!(ids(Order::Date, Tips::All), by_date);
    // From one tip as well.
    assert_eq!(
        ids(Order::AuthorDate, Tips::Head),
        git.lines(["rev-list", "HEAD", "--author-date-order"])
            .unwrap()
    );
    // A commit never comes after one of its parents, whatever the dates.
    let all = walk(
        &repo,
        &LogOptions {
            tips: Tips::All,
            order: Order::AuthorDate,
            ..Default::default()
        },
    )
    .unwrap();
    let mut seen = BTreeSet::new();
    for commit in &all {
        for parent in &commit.parents {
            assert!(!seen.contains(parent), "a parent before its child");
        }
        seen.insert(commit.id);
    }
    // In pieces nothing is settled early by this order: it comes whole.
    assert!(bitbraid_git::pieces::stable_rows(&all, Order::AuthorDate).is_empty());
}

#[test]
fn followed_file_history_is_git_log_follow() {
    let (scratch, git) = history_with_merges();
    let repo = Repository::open(scratch.path()).unwrap();
    for path in ["g.txt", "other.txt", "a.txt", "nothing-here.txt"] {
        let ours = file_history(&repo, path, &HistoryOptions::default()).unwrap();
        let want = git
            .lines(["log", "--follow", "--format=%H", "--", path])
            .unwrap();
        assert_eq!(
            ours.iter()
                .map(|e| e.commit.id.to_hex())
                .collect::<Vec<_>>(),
            want,
            "{path}"
        );
    }
    // Every commit that touched the file under either name, also on the
    // side a merge did not take from; no merge.
    let g = file_history(&repo, "g.txt", &HistoryOptions::default()).unwrap();
    let subjects: Vec<&str> = g.iter().map(|e| e.commit.summary.as_str()).collect();
    assert_eq!(
        subjects,
        [
            "change g",
            "rename f to g",
            "main: change f",
            "left: change f",
            "side: take it back",
            "side: change f",
            "add f"
        ]
    );
    assert_eq!(g[1].kind, bitbraid_git::history::HistoryKind::Renamed);
    assert_eq!(g[1].previous_path.as_deref(), Some("f.txt"));
    assert_eq!(g[2].path, "f.txt");

    // Limits: of entries, and of commits looked at.
    let two = file_history(
        &repo,
        "g.txt",
        &HistoryOptions {
            max_entries: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(two.len(), 2);
    let looked = file_history(
        &repo,
        "g.txt",
        &HistoryOptions {
            log: LogOptions {
                tips: Tips::Head,
                limit: Some(1),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(looked.len(), 1);
    // First parents only: the side branches are not walked.
    let first = file_history(
        &repo,
        "g.txt",
        &HistoryOptions {
            log: LogOptions {
                tips: Tips::Head,
                first_parent: true,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    let want = git
        .lines([
            "log",
            "--follow",
            "--first-parent",
            "--format=%H",
            "--",
            "g.txt",
        ])
        .unwrap();
    assert_eq!(
        first
            .iter()
            .map(|e| e.commit.id.to_hex())
            .collect::<Vec<_>>(),
        want
    );
}

/// An executable file that became a symbolic link has changed its type, as
/// git says, and is not only modified (git.git's
/// `t/perf/p6010-merge-base.sh` with a link put in its place). A file that
/// is not executable is checked too.
#[cfg(unix)]
#[test]
fn an_executable_file_that_became_a_link_has_changed_its_type() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch.write("run.sh", "#!/bin/sh\nexit 0\n").unwrap();
    scratch.write("plain.txt", "plain\n").unwrap();
    std::fs::set_permissions(
        scratch.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    commit_on_day(&git, "a script and a file", 1);
    assert!(
        git.run(["ls-files", "-s", "run.sh"])
            .unwrap()
            .starts_with("100755"),
        "the script is recorded as executable"
    );
    for name in ["run.sh", "plain.txt"] {
        std::fs::remove_file(scratch.path().join(name)).unwrap();
        symlink("README.md", scratch.path().join(name)).unwrap();
    }
    // The other way round, and a bit alone, for good measure.
    let readme = scratch.path().join("README.md");
    std::fs::set_permissions(&readme, std::fs::Permissions::from_mode(0o755)).unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let ours: Vec<(String, char)> = bitbraid_git::status(&repo)
        .unwrap()
        .into_iter()
        .map(|entry| (entry.path, entry.unstaged.map_or('.', |kind| kind.letter())))
        .collect();
    let gits: Vec<(String, char)> = git
        .lines(["status", "--porcelain=v2"])
        .unwrap()
        .into_iter()
        .map(|line| {
            let parts: Vec<&str> = line.splitn(9, ' ').collect();
            (parts[8].to_owned(), parts[1].chars().nth(1).unwrap())
        })
        .collect();
    assert_eq!(ours, gits);
    assert_eq!(
        ours,
        [
            ("README.md".to_owned(), 'M'),
            ("plain.txt".to_owned(), 'T'),
            ("run.sh".to_owned(), 'T')
        ]
    );
}

/// A worktree in which an executable file is a link that leads nowhere
/// (git.git, after a stash was taken back): gitoxide gives no status at all
/// ("No such file or directory") once it has to look at the content (the
/// link's length is the file's size). Git is asked then.
#[cfg(unix)]
#[test]
fn a_status_gitoxide_cannot_read_is_read_from_git() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    // Fourteen bytes, like the fourteen letters the link will hold.
    scratch.write("run.sh", "#!/bin/sh\n:;:\n").unwrap();
    std::fs::set_permissions(
        scratch.path().join("run.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    commit_on_day(&git, "a script", 1);
    std::fs::remove_file(scratch.path().join("run.sh")).unwrap();
    symlink("somewhere-else", scratch.path().join("run.sh")).unwrap();
    scratch.write("new.txt", "new\n").unwrap();
    scratch.write("a.txt", "changed\n").unwrap();
    git.run(["mv", "b.txt", "moved.txt"]).unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let ours: Vec<(String, char, char, Option<String>)> = bitbraid_git::status(&repo)
        .expect("a status all the same")
        .into_iter()
        .map(|entry| {
            (
                entry.path,
                entry.staged.map_or('.', |kind| kind.letter()),
                entry.unstaged.map_or('.', |kind| kind.letter()),
                entry.original_path,
            )
        })
        .collect();
    let rows =
        |rows: &[(&str, char, char, Option<&str>)]| -> Vec<(String, char, char, Option<String>)> {
            rows.iter()
                .map(|(path, staged, unstaged, from)| {
                    (
                        (*path).to_owned(),
                        *staged,
                        *unstaged,
                        from.map(str::to_owned),
                    )
                })
                .collect()
        };
    assert_eq!(
        ours,
        rows(&[
            ("a.txt", '.', 'M', None),
            ("moved.txt", 'R', '.', Some("b.txt")),
            ("new.txt", '.', '?', None),
            ("run.sh", '.', 'T', None),
        ])
    );
    // Which is what git says, letter for letter.
    let porcelain = git.run(["status", "--porcelain=v1", "-z"]).unwrap();
    for (path, staged, unstaged, _) in &ours {
        let line = match (*staged, *unstaged) {
            ('.', '?') => format!("?? {path}"),
            (staged, unstaged) => format!(
                "{}{} {path}",
                if staged == '.' { ' ' } else { staged },
                if unstaged == '.' { ' ' } else { unstaged }
            ),
        };
        assert!(
            porcelain.split('\0').any(|record| record == line),
            "{line:?} is not in {porcelain:?}"
        );
    }
}

/// `git log --follow` looks for where a new file came from among every
/// file there was, the ones the commit left alone included: a file made
/// from another one has that one's history behind it (cargo's tests are
/// made that way, one directory from the last).
#[test]
fn followed_history_goes_on_in_the_file_a_copy_was_made_of() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let text = |changed: &[usize]| {
        (0..24)
            .map(|line| {
                if changed.contains(&line) {
                    format!("line {line} changed\n")
                } else {
                    format!("line {line} of the original\n")
                }
            })
            .collect::<String>()
    };
    scratch.write("deep/down/orig.txt", &text(&[])).unwrap();
    // Alike too, but less: not the one.
    scratch
        .write("decoy.txt", &text(&[1, 2, 3, 4, 5, 6, 7, 8]))
        .unwrap();
    commit_on_day(&git, "add orig", 1);
    scratch.write("deep/down/orig.txt", &text(&[3])).unwrap();
    commit_on_day(&git, "change orig", 2);
    // Made from it and edited, in a commit that leaves it alone.
    scratch.write("copy.txt", &text(&[3, 10])).unwrap();
    scratch.write("unrelated.txt", "nothing alike\n").unwrap();
    commit_on_day(&git, "copy orig, edited", 3);
    scratch.write("copy.txt", &text(&[3, 10, 11])).unwrap();
    commit_on_day(&git, "change the copy", 4);
    scratch
        .write("deep/down/orig.txt", &text(&[3, 20]))
        .unwrap();
    commit_on_day(&git, "change orig again", 5);
    // The same bytes under another name, twice: the same file name wins.
    let readme = std::fs::read_to_string(scratch.path().join("README.md")).unwrap();
    scratch.write("elsewhere/README.md", &readme).unwrap();
    scratch.write("elsewhere/other-name.md", &readme).unwrap();
    commit_on_day(&git, "the readme once more", 6);

    let repo = Repository::open(scratch.path()).unwrap();
    for path in [
        "copy.txt",
        "elsewhere/README.md",
        "unrelated.txt",
        "decoy.txt",
    ] {
        let ours = file_history(&repo, path, &HistoryOptions::default()).unwrap();
        let want = git
            .lines(["log", "--follow", "--format=%H", "--", path])
            .unwrap();
        assert_eq!(
            ours.iter()
                .map(|e| e.commit.id.to_hex())
                .collect::<Vec<_>>(),
            want,
            "{path}: {:?}",
            ours.iter().map(|e| &e.commit.summary).collect::<Vec<_>>()
        );
    }
    let copy = file_history(&repo, "copy.txt", &HistoryOptions::default()).unwrap();
    let subjects: Vec<&str> = copy.iter().map(|e| e.commit.summary.as_str()).collect();
    assert_eq!(
        subjects,
        [
            "change the copy",
            "copy orig, edited",
            "change orig",
            "add orig"
        ]
    );
    assert_eq!(copy[1].kind, bitbraid_git::history::HistoryKind::Copied);
    assert_eq!(copy[1].previous_path.as_deref(), Some("deep/down/orig.txt"));
    assert_eq!(copy[2].path, "deep/down/orig.txt");
    let readme = file_history(&repo, "elsewhere/README.md", &HistoryOptions::default()).unwrap();
    assert_eq!(readme[0].previous_path.as_deref(), Some("README.md"));
    assert_eq!(readme.len(), 2);
    // Not followed: where it appears is where it starts.
    let plain = file_history(
        &repo,
        "copy.txt",
        &HistoryOptions {
            follow: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(plain.len(), 2);
    assert_eq!(plain[1].kind, bitbraid_git::history::HistoryKind::Added);
}

/// The name a file is followed by belongs to each commit: a branch that
/// still had the file under its old name when it was merged is read by
/// the old name, also when the commit that brought the new name found
/// nothing to have made it from (cargo's lint tests: the old file was
/// deleted two commits before the new one came).
#[test]
fn a_merged_branch_is_followed_by_the_name_the_file_has_there() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let text = |changed: &[usize]| {
        (0..20)
            .map(|line| {
                if changed.contains(&line) {
                    format!("line {line}, changed\n")
                } else {
                    format!("line {line} as it was written\n")
                }
            })
            .collect::<String>()
    };
    let day = |n: i64| format!("{} +0000", CLOCK_EPOCH + (400 + n) * 86_400);
    scratch.write("old/t.txt", &text(&[])).unwrap();
    commit_on_day(&git, "add old", 1);
    git.run(["checkout", "-q", "-b", "stale"]).unwrap();
    scratch.write("old/t.txt", &text(&[2])).unwrap();
    commit_on_day(&git, "stale: change old", 2);
    scratch.write("elsewhere.txt", "elsewhere\n").unwrap();
    commit_on_day(&git, "stale: something else", 3);
    git.run(["checkout", "-q", "main"]).unwrap();
    scratch.remove("old/t.txt").unwrap();
    commit_on_day(&git, "main: delete old", 4);
    scratch.write("new/t.txt", &text(&[2, 7])).unwrap();
    commit_on_day(&git, "main: add new, out of nothing", 5);
    // Old is gone on one side and changed on the other: git stops, and the
    // merge keeps it gone.
    let _ = git.run(["merge", "-q", "--no-ff", "--no-commit", "stale"]);
    let _ = git.run(["rm", "-q", "-f", "--ignore-unmatch", "old/t.txt"]);
    git.run_with(
        ["commit", "-q", "-m", "merge stale"],
        &[
            ("GIT_AUTHOR_DATE", &day(6)),
            ("GIT_COMMITTER_DATE", &day(6)),
        ],
    )
    .unwrap();
    scratch.write("new/t.txt", &text(&[2, 7, 9])).unwrap();
    commit_on_day(&git, "main: change new", 7);

    let repo = Repository::open(scratch.path()).unwrap();
    let ours = file_history(&repo, "new/t.txt", &HistoryOptions::default()).unwrap();
    let want = git
        .lines(["log", "--follow", "--format=%H", "--", "new/t.txt"])
        .unwrap();
    let subjects: Vec<&str> = ours.iter().map(|e| e.commit.summary.as_str()).collect();
    let ids: Vec<String> = ours.iter().map(|e| e.commit.id.to_hex()).collect();
    // Git follows a file by the name it has on each side of a merge since
    // 2.56 ("better handle non-linear history"). An older git keeps one
    // name for the whole walk and changes it only where it finds a
    // rename; here it finds none, and stops where the new name came: the
    // first two of the list. The list held to is the newer git's.
    if follows_each_side(&bitbraid_testkit::oracle::git_version().unwrap()) {
        assert_eq!(ids, want, "{subjects:?}");
    } else {
        assert_eq!(ids.get(..2), Some(want.as_slice()), "{subjects:?}");
    }
    assert_eq!(
        subjects,
        [
            "main: change new",
            "main: add new, out of nothing",
            "stale: change old",
            "add old"
        ]
    );
    assert_eq!(ours[1].kind, bitbraid_git::history::HistoryKind::Added);
    assert_eq!(ours[2].path, "old/t.txt");
    // From the commit that added it alone there is nothing before it.
    let added: Oid = git.rev_parse("HEAD~2").unwrap().parse().unwrap();
    let alone = file_history(
        &repo,
        "new/t.txt",
        &HistoryOptions {
            log: LogOptions {
                tips: Tips::Ids(vec![added]),
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(alone.len(), 1);
}

/// Whether the git that `git --version` names follows a file by the name
/// it has on each side of a merge: 2.56 and later.
fn follows_each_side(version: &str) -> bool {
    let numbers: Vec<u32> = version
        .trim_start_matches("git version ")
        .split('.')
        .take(2)
        .map_while(|number| number.parse().ok())
        .collect();
    numbers.as_slice() >= [2, 56].as_slice()
}

/// Commits made with plumbing, so that a parent can be dated after its
/// child: `(name, parents, day)`. Returns the ids by name.
fn dated_history(git: &Git, commits: &[(&str, &[&str], i64)]) -> Vec<(String, String)> {
    let tree = git.rev_parse("HEAD^{tree}").unwrap();
    let mut ids: Vec<(String, String)> = Vec::new();
    for (name, parents, day) in commits {
        let date = format!("{} +0000", CLOCK_EPOCH + 800 * 86_400 + day * 3600);
        let mut args = vec![
            "commit-tree".to_owned(),
            tree.clone(),
            "-m".to_owned(),
            (*name).to_owned(),
        ];
        for parent in *parents {
            let id = &ids.iter().find(|(known, _)| known == parent).unwrap().1;
            args.push("-p".to_owned());
            args.push(id.clone());
        }
        let id = git
            .run_with(
                &args,
                &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
            )
            .unwrap()
            .trim()
            .to_owned();
        ids.push(((*name).to_owned(), id));
    }
    ids
}

#[test]
fn the_graph_order_is_date_order_when_clocks_were_wrong() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    // Parents dated after their children (five and six), several tips.
    let ids = dated_history(
        &git,
        &[
            ("nine", &[], 1),
            ("five", &["nine"], 90),
            ("six", &["nine"], 95),
            ("three", &["nine"], 30),
            ("one", &["five"], 100),
            ("four", &["five"], 35),
            ("two", &["six"], 40),
            ("seven", &["one", "two", "three"], 20),
            ("eight", &["four", "seven"], 10),
        ],
    );
    for (name, id) in &ids {
        git.run(["update-ref", &format!("refs/heads/skew/{name}"), id])
            .unwrap();
    }
    let tips: Vec<Oid> = ids.iter().map(|(_, id)| id.parse().unwrap()).collect();
    let repo = Repository::open(scratch.path()).unwrap();
    let ours = walk(
        &repo,
        &LogOptions {
            tips: Tips::Ids(tips),
            ..Default::default()
        },
    )
    .unwrap();
    let mut args = vec!["rev-list".to_owned(), "--date-order".to_owned()];
    args.extend(ids.iter().map(|(_, id)| id.clone()));
    let want = git.lines(&args).unwrap();
    let name = |hex: &str| ids.iter().find(|(_, id)| id == hex).unwrap().0.clone();
    assert_eq!(
        ours.iter()
            .map(|c| name(&c.id.to_hex()))
            .collect::<Vec<_>>(),
        want.iter().map(|id| name(id)).collect::<Vec<_>>()
    );
    // And all of it, with the fixture's own commits.
    let all = walk(&repo, &LogOptions::default()).unwrap();
    let want = git
        .lines(["rev-list", "--date-order", "HEAD", "--branches", "--tags"])
        .unwrap();
    assert_eq!(all.iter().map(|c| c.id.to_hex()).collect::<Vec<_>>(), want);
}
