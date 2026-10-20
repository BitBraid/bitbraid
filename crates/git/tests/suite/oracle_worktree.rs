//! Differential tests for status, diffs, blame and file history against git.

use std::collections::BTreeMap;

use bitbraid_git::{
    DiffOptions, DiffSide, HistoryOptions, LogOptions, Repository, Tips, apply_hunks, blame,
    diff_file, diff_trees, file_history, log, status,
};
use bitbraid_testkit::{Git, Scratch, build_fixture, fixtures_out_dir, list_scripts};

/// Parse `git status --porcelain=v2` into path -> (staged letter, unstaged letter, original path).
pub(crate) fn porcelain(git: &Git) -> BTreeMap<String, (char, char, Option<String>)> {
    let mut out = BTreeMap::new();
    for line in git
        .lines([
            "status",
            "--porcelain=v2",
            "--untracked-files=all",
            "--no-renames",
        ])
        .unwrap()
    {
        let mut parts = line.splitn(2, ' ');
        let tag = parts.next().unwrap();
        let rest = parts.next().unwrap_or_default();
        match tag {
            "1" => {
                let fields: Vec<&str> = rest.splitn(8, ' ').collect();
                let xy: Vec<char> = fields[0].chars().collect();
                out.insert(fields[7].to_owned(), (xy[0], xy[1], None));
            }
            "2" => {
                let fields: Vec<&str> = rest.splitn(9, ' ').collect();
                let xy: Vec<char> = fields[0].chars().collect();
                let (path, orig) = fields[8].split_once('\t').unwrap();
                out.insert(path.to_owned(), (xy[0], xy[1], Some(orig.to_owned())));
            }
            "u" => {
                let fields: Vec<&str> = rest.splitn(10, ' ').collect();
                out.insert(fields[9].to_owned(), ('U', 'U', None));
            }
            "?" => {
                out.insert(rest.to_owned(), ('.', '?', None));
            }
            _ => {}
        }
    }
    out
}

pub(crate) fn ours(repo: &Repository) -> BTreeMap<String, (char, char, Option<String>)> {
    status(repo)
        .unwrap()
        .into_iter()
        .map(|e| {
            if e.conflict {
                return (e.path, ('U', 'U', None));
            }
            (
                e.path,
                (
                    e.staged.map(|k| k.letter()).unwrap_or('.'),
                    e.unstaged.map(|k| k.letter()).unwrap_or('.'),
                    e.original_path,
                ),
            )
        })
        .collect()
}

#[test]
fn clean_fixtures_have_empty_status() {
    for name in list_scripts().unwrap() {
        let path = build_fixture(&name).unwrap();
        let repo = Repository::open(&path).unwrap();
        assert!(
            status(&repo).unwrap().is_empty(),
            "{name}: expected a clean status"
        );
    }
}

#[test]
fn status_matches_porcelain_for_a_mixed_worktree() {
    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    scratch.write("a.txt", "a1\nchanged\n").unwrap(); // unstaged modify
    scratch.write("b.txt", "b1\nstaged\n").unwrap();
    git.run(["add", "b.txt"]).unwrap(); // staged modify
    scratch.write("new.txt", "untracked\n").unwrap(); // untracked
    scratch.write("dir/staged-new.txt", "added\n").unwrap();
    git.run(["add", "dir/staged-new.txt"]).unwrap(); // staged add
    scratch.remove("base.txt").unwrap(); // unstaged delete
    git.run(["rm", "-q", "README.md"]).unwrap(); // staged delete
    scratch.write("both.txt", "v1\n").unwrap();
    git.run(["add", "both.txt"]).unwrap();
    scratch.write("both.txt", "v2\n").unwrap(); // staged add + unstaged modify

    let repo = Repository::open(scratch.path()).unwrap();
    let got = ours(&repo);
    let want = porcelain(&git);
    assert_eq!(got, want);
}

/// A status that stops once enough changed paths were found.
#[test]
fn a_bounded_status_is_a_part_of_the_whole_and_says_that_there_is_more() {
    use bitbraid_git::status::status_bounded;

    let scratch = Scratch::of("branches-merges").unwrap();
    let git = scratch.git();
    for n in 0..60 {
        scratch
            .write(&format!("pile/untracked-{n:02}.txt"), "new\n")
            .unwrap();
    }
    scratch.write("a.txt", "a1\nchanged\n").unwrap();
    scratch.write("b.txt", "b1\nstaged\n").unwrap();
    git.run(["add", "b.txt"]).unwrap();
    scratch
        .write("b.txt", "b1\nstaged\nand changed again\n")
        .unwrap();

    let repo = Repository::open(scratch.path()).unwrap();
    let whole = status(&repo).unwrap();
    // Held to git first: 60 untracked files and two changed ones.
    assert_eq!(
        whole.len(),
        git.lines(["status", "--porcelain", "-uall"]).unwrap().len()
    );
    assert_eq!(whole.len(), 62);

    // No bound that is reached: everything, and nothing more to say.
    for limit in [62, 63, 1000, usize::MAX] {
        let (entries, more) = status_bounded(&repo, limit).unwrap();
        assert_eq!(entries, whole, "limit {limit}");
        assert!(!more, "limit {limit}");
    }
    // A bound that is: that many paths, each as the whole status has it,
    // in path order, and the flag.
    for limit in [0, 1, 10, 61] {
        let (entries, more) = status_bounded(&repo, limit).unwrap();
        assert_eq!(entries.len(), limit);
        assert!(more, "limit {limit}");
        assert!(entries.windows(2).all(|pair| pair[0].path < pair[1].path));
        for entry in &entries {
            let full = whole.iter().find(|e| e.path == entry.path).unwrap();
            // A path cut off between its two halves may lack one of them.
            assert!(entry.staged.is_none() || entry.staged == full.staged);
            assert!(entry.unstaged.is_none() || entry.unstaged == full.unstaged);
        }
        // What is changed among the tracked files is kept before anything
        // untracked, whole: it is what there is to commit.
        if limit >= 2 {
            for path in ["a.txt", "b.txt"] {
                let kept = entries.iter().find(|e| e.path == path);
                let full = whole.iter().find(|e| e.path == path);
                assert_eq!(kept, full, "limit {limit}: {path}");
                assert!(kept.is_some());
            }
        }
    }
    // A clean worktree has nothing to cut.
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "all of it"]).unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    assert_eq!(status_bounded(&repo, 0).unwrap(), (Vec::new(), false));
}

#[test]
fn status_reports_staged_renames_like_git() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    git.run(["mv", "a.txt", "moved.txt"]).unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    let entries = status(&repo).unwrap();
    let moved = entries
        .iter()
        .find(|e| e.path == "moved.txt")
        .expect("rename target listed");
    assert_eq!(moved.staged, Some(bitbraid_git::ChangeKind::Renamed));
    assert_eq!(moved.original_path.as_deref(), Some("a.txt"));
    // git agrees once rename detection is on.
    let line = git
        .lines(["status", "--porcelain=v2"])
        .unwrap()
        .into_iter()
        .find(|l| l.starts_with("2 R"))
        .expect("git reports a rename");
    assert!(line.ends_with("moved.txt\ta.txt"), "{line}");
}

#[test]
fn status_reports_conflicts() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch.write("a.txt", "main side\n").unwrap();
    git.run(["commit", "-qam", "main side"]).unwrap();
    git.run(["checkout", "-qb", "other", "HEAD~1"]).unwrap();
    scratch.write("a.txt", "other side\n").unwrap();
    git.run(["commit", "-qam", "other side"]).unwrap();
    git.run(["checkout", "-q", "main"]).unwrap();
    let _ = git.run(["merge", "other"]); // conflicts by design
    let repo = Repository::open(scratch.path()).unwrap();
    assert_eq!(ours(&repo), porcelain(&git));
    let entries = status(&repo).unwrap();
    assert!(entries.iter().any(|e| e.path == "a.txt" && e.conflict));
}

#[test]
fn tree_diffs_match_numstat_and_reproduce_new_blobs() {
    for name in list_scripts().unwrap() {
        let path = build_fixture(&name).unwrap();
        let repo = Repository::open(&path).unwrap();
        let git = Git::new(&path, fixtures_out_dir().join(".home"));
        let commits = log(&repo, &LogOptions::default()).unwrap();
        for commit in &commits {
            let parent = commit.parents.first().copied();
            let old = parent.map(DiffSide::Commit).unwrap_or(DiffSide::Empty);
            let files = diff_trees(
                &repo,
                &old,
                &DiffSide::Commit(commit.id),
                &DiffOptions::default(),
            )
            .unwrap();

            // numstat: additions, deletions, path (renames as "old => new").
            let parent_spec = parent.map(|p| p.to_hex()).unwrap_or_else(|| {
                git.run(["hash-object", "-t", "tree", "/dev/null"])
                    .unwrap()
                    .trim()
                    .to_owned()
            });
            let numstat = git
                .lines(["diff", "--numstat", "-M", &parent_spec, &commit.id.to_hex()])
                .unwrap();
            let mut want: BTreeMap<String, (u32, u32)> = BTreeMap::new();
            for line in numstat {
                let parts: Vec<&str> = line.splitn(3, '\t').collect();
                let path = parts[2];
                let path = path
                    .split(" => ")
                    .last()
                    .unwrap()
                    .trim_end_matches('}')
                    .to_owned();
                let path = path.replace('{', "");
                want.insert(
                    path,
                    (parts[0].parse().unwrap_or(0), parts[1].parse().unwrap_or(0)),
                );
            }
            let got: BTreeMap<String, (u32, u32)> = files
                .iter()
                .map(|f| (f.path.clone(), (f.additions, f.deletions)))
                .collect();
            assert_eq!(got, want, "{name}: numstat of {}", commit.id);

            // Hunks reproduce the new blob from the old blob.
            for file in &files {
                if file.binary {
                    continue;
                }
                let old_text = match file.old_id {
                    Some(id) => {
                        String::from_utf8(repo.gix().find_blob(*id.as_gix()).unwrap().data.clone())
                            .unwrap()
                    }
                    None => String::new(),
                };
                let new_text = match file.new_id {
                    Some(id) => {
                        String::from_utf8(repo.gix().find_blob(*id.as_gix()).unwrap().data.clone())
                            .unwrap()
                    }
                    None => String::new(),
                };
                assert_eq!(
                    apply_hunks(&old_text, &file.hunks).as_deref(),
                    Some(new_text.as_str()),
                    "{name}: hunks of {} in {}",
                    file.path,
                    commit.id
                );
                // Hunk headers equal git's.
                // Both paths of a rename must be in the pathspec, or git cannot pair them.
                let commit_hex = commit.id.to_hex();
                let mut args = vec![
                    "diff",
                    "-U3",
                    "-M",
                    parent_spec.as_str(),
                    commit_hex.as_str(),
                    "--",
                    file.path.as_str(),
                ];
                if let Some(old_path) = &file.old_path {
                    args.push(old_path);
                }
                let text = git.run(&args).unwrap();
                let want_headers: Vec<String> = text
                    .lines()
                    .filter(|l| l.starts_with("@@"))
                    .map(|l| l.split(" @@").next().unwrap().to_owned() + " @@")
                    .collect();
                let got_headers: Vec<String> = file.hunks.iter().map(|h| h.header()).collect();
                assert_eq!(
                    got_headers, want_headers,
                    "{name}: hunk headers of {} in {}",
                    file.path, commit.id
                );
            }
        }
    }
}

#[test]
fn rename_is_detected_in_tree_diff() {
    let path = build_fixture("renames").unwrap();
    let repo = Repository::open(&path).unwrap();
    let git = Git::new(&path, fixtures_out_dir().join(".home"));
    let rename_commit = git.rev_parse("HEAD~2").unwrap(); // "rename a to b"
    let before = git.rev_parse("HEAD~3").unwrap(); // "add a and keep"
    let files = diff_trees(
        &repo,
        &DiffSide::Commit(before.parse().unwrap()),
        &DiffSide::Commit(rename_commit.parse().unwrap()),
        &DiffOptions::default(),
    )
    .unwrap();
    let file = files
        .iter()
        .find(|f| f.path == "b.txt")
        .expect("rename target");
    assert_eq!(file.kind, bitbraid_git::diff::FileChangeKind::Renamed);
    assert_eq!(file.old_path.as_deref(), Some("a.txt"));
    assert!(file.hunks.is_empty(), "pure rename has no hunks");
}

#[test]
fn file_diffs_cover_index_and_worktree() {
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch.write("a.txt", "2\nstaged line\n").unwrap();
    git.run(["add", "a.txt"]).unwrap();
    scratch
        .write("a.txt", "2\nstaged line\nworktree line\n")
        .unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    let head = DiffSide::parse(&repo, "HEAD").unwrap();

    let staged = diff_file(&repo, &head, &DiffSide::Index, "a.txt", 3)
        .unwrap()
        .unwrap();
    assert_eq!((staged.additions, staged.deletions), (1, 0));
    let unstaged = diff_file(&repo, &DiffSide::Index, &DiffSide::Worktree, "a.txt", 3)
        .unwrap()
        .unwrap();
    assert_eq!((unstaged.additions, unstaged.deletions), (1, 0));
    let total = diff_file(&repo, &head, &DiffSide::Worktree, "a.txt", 3)
        .unwrap()
        .unwrap();
    assert_eq!((total.additions, total.deletions), (2, 0));
    assert!(
        diff_file(&repo, &head, &DiffSide::Worktree, "nope.txt", 3)
            .unwrap()
            .is_none()
    );

    // git agrees on the counts.
    let numstat = git
        .run(["diff", "--numstat", "--cached", "--", "a.txt"])
        .unwrap();
    assert!(numstat.starts_with("1\t0\t"), "{numstat}");
}

#[test]
fn blame_matches_git_blame_porcelain() {
    for name in ["linear", "renames", "branches-merges"] {
        let path = build_fixture(name).unwrap();
        let repo = Repository::open(&path).unwrap();
        let git = Git::new(&path, fixtures_out_dir().join(".home"));
        let file = if name == "renames" { "b.txt" } else { "a.txt" };
        let lines = blame(&repo, file, None).unwrap();
        let porcelain = git
            .run(["blame", "--porcelain", "HEAD", "--", file])
            .unwrap();
        let mut want: Vec<(u32, String, u32)> = Vec::new();
        for line in porcelain.lines() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts.len() >= 3
                && parts[0].len() == 40
                && parts[0].chars().all(|c| c.is_ascii_hexdigit())
            {
                want.push((
                    parts[2].parse().unwrap(),
                    parts[0].to_owned(),
                    parts[1].parse().unwrap(),
                ));
            }
        }
        want.sort();
        let got: Vec<(u32, String, u32)> = lines
            .iter()
            .map(|l| (l.line, l.commit.to_hex(), l.source_line))
            .collect();
        assert_eq!(got, want, "{name}: blame of {file}");
        let content = std::fs::read_to_string(path.join(file)).unwrap();
        assert_eq!(
            lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
            content.lines().collect::<Vec<_>>()
        );
    }
}

#[test]
fn file_history_follows_renames_like_git_log() {
    let path = build_fixture("renames").unwrap();
    let repo = Repository::open(&path).unwrap();
    let git = Git::new(&path, fixtures_out_dir().join(".home"));

    let follow = file_history(&repo, "b.txt", &HistoryOptions::default()).unwrap();
    let want = git
        .lines(["log", "--follow", "--format=%H", "--", "b.txt"])
        .unwrap();
    assert_eq!(
        follow
            .iter()
            .map(|e| e.commit.id.to_hex())
            .collect::<Vec<_>>(),
        want
    );
    assert_eq!(follow[1].kind, bitbraid_git::history::HistoryKind::Renamed);
    assert_eq!(follow[1].previous_path.as_deref(), Some("a.txt"));
    assert_eq!(follow[2].path, "a.txt");

    let plain = file_history(
        &repo,
        "b.txt",
        &HistoryOptions {
            follow: false,
            ..Default::default()
        },
    )
    .unwrap();
    let want = git.lines(["log", "--format=%H", "--", "b.txt"]).unwrap();
    assert_eq!(
        plain
            .iter()
            .map(|e| e.commit.id.to_hex())
            .collect::<Vec<_>>(),
        want
    );

    let deleted = file_history(&repo, "keep.txt", &HistoryOptions::default()).unwrap();
    let want = git.lines(["log", "--format=%H", "--", "keep.txt"]).unwrap();
    assert_eq!(
        deleted
            .iter()
            .map(|e| e.commit.id.to_hex())
            .collect::<Vec<_>>(),
        want
    );
    assert_eq!(deleted[0].kind, bitbraid_git::history::HistoryKind::Deleted);

    let linear = file_history(
        &repo,
        "b.txt",
        &HistoryOptions {
            log: LogOptions {
                tips: Tips::All,
                ..Default::default()
            },
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(linear.len(), 3);
}

/// A repository where a commit is replaced (`refs/replace/`, here a
/// graft that takes one commit out of the history): git reads the commit
/// with the parent the replacement names, gitoxide with the one that is
/// stored. The history of a file is git's there, with and without
/// following, and it is not the history of the commits as they are stored.
#[test]
fn file_history_is_gits_where_commits_are_replaced() {
    use bitbraid_git::history::{HistoryEntry, HistoryKind};
    let scratch = bitbraid_testkit::Scratch::of("renames").unwrap();
    let git = scratch.git();
    let ids = |args: &[&str]| git.lines(args.iter().copied()).unwrap();
    let ours = |repo: &Repository, path: &str, follow: bool| {
        file_history(
            repo,
            path,
            &HistoryOptions {
                follow,
                ..Default::default()
            },
        )
        .unwrap()
    };
    let hexes = |entries: &[HistoryEntry]| {
        entries
            .iter()
            .map(|entry| entry.commit.id.to_hex())
            .collect::<Vec<_>>()
    };

    // As it is stored: add a, rename a to b, modify b, delete keep.
    let stored_follow = ids(&["log", "--follow", "--format=%H", "--", "b.txt"]);
    let stored_plain = ids(&["log", "--format=%H", "--", "b.txt"]);
    assert_eq!(stored_follow.len(), 3);
    {
        let repo = Repository::open(scratch.path()).unwrap();
        assert_eq!(hexes(&ours(&repo, "b.txt", true)), stored_follow);
        assert_eq!(hexes(&ours(&repo, "b.txt", false)), stored_plain);
    }

    // "delete keep" is given the rename commit as its parent: "modify b"
    // is no longer in its history, and the change to b.txt is its own.
    git.run(["replace", "--graft", "HEAD", "HEAD~2"]).unwrap();
    assert_eq!(git.refs("refs/replace").unwrap().len(), 1);
    let repo = Repository::open(scratch.path()).unwrap();
    for (path, follow) in [
        ("b.txt", true),
        ("b.txt", false),
        ("a.txt", true),
        ("a.txt", false),
        ("keep.txt", true),
        ("keep.txt", false),
        ("never-there.txt", true),
    ] {
        let mut args = vec!["log", "--format=%H"];
        if follow {
            args.push("--follow");
        }
        args.extend(["--", path]);
        assert_eq!(
            hexes(&ours(&repo, path, follow)),
            ids(&args),
            "{path}, follow {follow}"
        );
    }
    // It is another history than the stored one, or this test saw nothing.
    let follow = ours(&repo, "b.txt", true);
    assert_ne!(hexes(&follow), stored_follow);
    // What happened in each is what git says happened: the change to
    // b.txt is in "delete keep" now, then the rename, then the beginning.
    assert_eq!(
        follow
            .iter()
            .map(|entry| (
                entry.commit.summary.as_str(),
                entry.kind,
                entry.path.as_str(),
                entry.previous_path.as_deref()
            ))
            .collect::<Vec<_>>(),
        [
            ("delete keep", HistoryKind::Modified, "b.txt", None),
            (
                "rename a to b",
                HistoryKind::Renamed,
                "b.txt",
                Some("a.txt")
            ),
            ("add a and keep", HistoryKind::Added, "a.txt", None),
        ]
    );
    assert_eq!(
        follow[0].blob.map(|blob| blob.to_hex()),
        Some(git.rev_parse("HEAD:b.txt").unwrap())
    );
    assert_eq!(
        follow[2].blob.map(|blob| blob.to_hex()),
        Some(git.rev_parse("HEAD~2:a.txt").unwrap())
    );
    // Without following, the file begins where it has that name.
    let plain = ours(&repo, "b.txt", false);
    assert_eq!(
        plain.iter().map(|entry| entry.kind).collect::<Vec<_>>(),
        [HistoryKind::Modified, HistoryKind::Added]
    );
    // A deletion, and no more of them than were asked for.
    let deleted = ours(&repo, "keep.txt", true);
    assert_eq!(deleted[0].kind, HistoryKind::Deleted);
    assert_eq!(deleted[0].blob, None);
    let one = file_history(
        &repo,
        "keep.txt",
        &HistoryOptions {
            max_entries: Some(1),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(one.len(), 1);

    // With the replacement taken away it is the stored history again.
    git.run(["replace", "-d", "HEAD"]).unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    assert_eq!(hexes(&ours(&repo, "b.txt", true)), stored_follow);

    // A graft of the old kind (`info/grafts`) is read the same way.
    let child = git.rev_parse("HEAD").unwrap();
    let parent = git.rev_parse("HEAD~2").unwrap();
    std::fs::create_dir_all(scratch.path().join(".git/info")).unwrap();
    std::fs::write(
        scratch.path().join(".git/info/grafts"),
        format!("{child} {parent}\n"),
    )
    .unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    let grafted = ours(&repo, "b.txt", true);
    assert_eq!(
        hexes(&grafted),
        ids(&["log", "--follow", "--format=%H", "--", "b.txt"])
    );
    assert_ne!(hexes(&grafted), stored_follow);
}

/// Hunks as git prints them, without the text after the second `@@`.
fn hunk_text(hunks: &[bitbraid_git::Hunk]) -> Vec<String> {
    let mut out = Vec::new();
    for hunk in hunks {
        out.push(hunk.header());
        for line in &hunk.lines {
            let mark = match line.kind {
                bitbraid_git::LineKind::Context => ' ',
                bitbraid_git::LineKind::Added => '+',
                bitbraid_git::LineKind::Removed => '-',
            };
            out.push(format!("{mark}{}", line.text));
        }
    }
    out
}

/// The hunks of `git diff <args>` for one path, headers cut after `@@`.
pub(crate) fn git_hunks(git: &Git, args: &[&str], path: &str) -> Vec<String> {
    let mut command = vec!["diff", "--no-color", "--no-ext-diff"];
    command.extend_from_slice(args);
    command.extend_from_slice(&["--", path]);
    let text = git.run(command).unwrap();
    // Not `lines()`: a carriage return at the end of a line is content.
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

/// Ignoring whitespace changes which lines differ, so it is done where the
/// lines are compared. The result has to be git's, line for line.
#[test]
fn whitespace_modes_match_git_diff() {
    use bitbraid_git::{Whitespace, diff_file_with};

    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    let old = "fn main() {\n    let a = 1;\n    let b = 2;   \n    call(a, b);\n}\n\n\
               fn middle() {\n    one();\n    two();\n    three();\n    four();\n}\n\n\
               fn tail() {}\n\tindented();\nlast\n";
    let new = "fn main() {\n        let a = 1;\n    let b = 2;\n    call( a,  b );\n    extra();\n}\n\n\
               fn middle() {\n    one();\n    two();\n    three();\n    four();\n}\n\n\
               fn tail() { }\n    indented();\nlast\r\n";
    scratch.write("ws.rs", old).unwrap();
    git.run(["add", "ws.rs"]).unwrap();
    scratch.write("ws.rs", new).unwrap();
    let repo = Repository::open(scratch.path()).unwrap();

    for (whitespace, flag) in [
        (Whitespace::Exact, None),
        (Whitespace::IgnoreTrailing, Some("--ignore-space-at-eol")),
        (Whitespace::IgnoreAll, Some("--ignore-all-space")),
    ] {
        let ours = diff_file_with(
            &repo,
            &DiffSide::Index,
            &DiffSide::Worktree,
            "ws.rs",
            3,
            whitespace,
        )
        .unwrap()
        .unwrap();
        let args: Vec<&str> = flag.into_iter().collect();
        assert_eq!(
            hunk_text(&ours.hunks),
            git_hunks(&git, &args, "ws.rs"),
            "{whitespace:?}"
        );
    }

    // Hostile input: a mode other than exact still declines binary data and
    // copes with empty sides and a missing final newline.
    assert!(bitbraid_git::diff_bytes_with(b"a\0b", b"c", 3, Whitespace::IgnoreAll).is_none());
    for whitespace in [Whitespace::IgnoreTrailing, Whitespace::IgnoreAll] {
        assert_eq!(
            bitbraid_git::diff_bytes_with(b"", b"", 3, whitespace),
            Some(Vec::new())
        );
        let hunks = bitbraid_git::diff_bytes_with(b"", b"x  \ny", 0, whitespace).unwrap();
        assert_eq!(hunk_text(&hunks), ["@@ -0,0 +1,2 @@", "+x  ", "+y"]);
        assert!(hunks[0].lines[1].no_newline);
        assert_eq!(
            bitbraid_git::diff_bytes_with(b"a \n", b"a\n", u32::MAX, whitespace),
            Some(Vec::new())
        );
    }
}

/// "The whole file" is asked for as the largest context there is. The hunk
/// builder must not overflow on it and take the calling thread down.
#[test]
fn a_huge_context_means_the_whole_file_and_does_not_overflow() {
    use bitbraid_git::{Whitespace, diff_bytes, diff_bytes_with};

    let old = b"one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\nnine\nten\n";
    let new = b"one\ntwo\nTHREE\nfour\nfive\nsix\nseven\neight\nnine\nTEN\n";
    for context in [
        u32::MAX,
        u32::MAX - 1,
        u32::MAX / 2,
        u32::MAX / 2 + 1,
        1 << 31,
    ] {
        let hunks = diff_bytes(old, new, context).unwrap();
        assert_eq!(hunks.len(), 1, "context {context}");
        assert_eq!(hunks[0].header(), "@@ -1,10 +1,10 @@");
        assert_eq!(hunks[0].lines.len(), 12);
        for whitespace in [Whitespace::IgnoreTrailing, Whitespace::IgnoreAll] {
            let loose = diff_bytes_with(old, new, context, whitespace).unwrap();
            assert_eq!(hunk_text(&loose), hunk_text(&hunks));
        }
    }
    // Empty and one-sided inputs under the same contexts.
    assert_eq!(diff_bytes(b"", b"", u32::MAX), Some(Vec::new()));
    assert_eq!(diff_bytes(b"", b"x\n", u32::MAX).unwrap().len(), 1);
    assert_eq!(diff_bytes(b"x\n", b"", u32::MAX).unwrap().len(), 1);
    // And it is what git shows with as much context.
    let scratch = Scratch::of("linear").unwrap();
    let git = scratch.git();
    scratch
        .write("ctx.txt", std::str::from_utf8(old).unwrap())
        .unwrap();
    git.run(["add", "ctx.txt"]).unwrap();
    scratch
        .write("ctx.txt", std::str::from_utf8(new).unwrap())
        .unwrap();
    assert_eq!(
        hunk_text(&diff_bytes(old, new, u32::MAX).unwrap()),
        git_hunks(&git, &["-U1000000"], "ctx.txt")
    );
}
