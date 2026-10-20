//! What the diff viewer asks beyond a plain diff, against git: a file that
//! was moved compared between its two places, a file too large to compare
//! measured and left alone, a binary file told by size and id on both
//! sides, and a commit compared with the working tree and with the index.

use std::collections::BTreeMap;

use bitbraid_git::{
    DiffOptions, DiffSide, Repository, Whitespace, apply_hunks,
    compare::diff_mixed,
    diff::{FileChangeKind, diff_file_guarded},
    diff_trees,
};
use bitbraid_testkit::{Git, Scratch};

fn open(copy: &Scratch) -> Repository {
    Repository::open(copy.path()).unwrap()
}

fn side(repo: &Repository, rev: &str) -> DiffSide {
    DiffSide::parse(repo, rev).unwrap()
}

fn commit_all(git: &Git, message: &str) {
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", message]).unwrap();
}

/// `git diff --numstat` of two blobs: lines added and removed.
fn numstat(git: &Git, old: &str, new: &str) -> (u32, u32) {
    let out = git.run(["diff", "--numstat", old, new]).unwrap();
    let mut fields = out.split_whitespace();
    (
        fields.next().unwrap().parse().unwrap(),
        fields.next().unwrap().parse().unwrap(),
    )
}

#[test]
fn a_moved_file_is_compared_between_its_two_places() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let lines: String = (1..=40).map(|n| format!("line {n}\n")).collect();
    copy.write("src/old name.txt", &lines).unwrap();
    commit_all(&git, "a file to move");
    std::fs::create_dir_all(copy.path().join("moved")).unwrap();
    git.run(["mv", "src/old name.txt", "moved/new name.txt"])
        .unwrap();
    let edited = lines
        .replace("line 7\n", "line seven\n")
        .replace("line 30\n", "");
    copy.write("moved/new name.txt", &edited).unwrap();
    commit_all(&git, "move and edit it");
    let repo = open(&copy);
    let (before, after) = (side(&repo, "HEAD~1"), side(&repo, "HEAD"));

    for (whitespace, context) in [
        (Whitespace::Exact, 3),
        (Whitespace::Exact, u32::MAX),
        (Whitespace::IgnoreAll, 3),
    ] {
        let diff = diff_file_guarded(
            &repo,
            &before,
            &after,
            Some("src/old name.txt"),
            "moved/new name.txt",
            context,
            whitespace,
            None,
        )
        .unwrap()
        .unwrap();
        assert_eq!(diff.kind, FileChangeKind::Renamed);
        assert_eq!(diff.old_path.as_deref(), Some("src/old name.txt"));
        assert_eq!(diff.path, "moved/new name.txt");
        assert_eq!(
            (diff.additions, diff.deletions),
            numstat(&git, "HEAD~1:src/old name.txt", "HEAD:moved/new name.txt"),
            "{whitespace:?} with {context} lines of context"
        );
        assert_eq!(
            apply_hunks(&lines, &diff.hunks).as_deref(),
            Some(edited.as_str())
        );
        assert_eq!(
            diff.old_id.map(|id| id.to_hex()),
            Some(git.rev_parse("HEAD~1:src/old name.txt").unwrap())
        );
        assert_eq!(
            diff.new_id.map(|id| id.to_hex()),
            Some(git.rev_parse("HEAD:moved/new name.txt").unwrap())
        );
    }
    // The same path on both sides is no move, and without the old place
    // the file reads as new.
    let alone = diff_file_guarded(
        &repo,
        &before,
        &after,
        None,
        "moved/new name.txt",
        3,
        Whitespace::Exact,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(alone.kind, FileChangeKind::Added);
    assert_eq!(alone.additions, 39);
    // A place outside the worktree is refused, one that is nowhere is no
    // old side.
    for hostile in ["../outside", "/etc/passwd", ""] {
        assert!(
            diff_file_guarded(
                &repo,
                &before,
                &after,
                Some(hostile),
                "moved/new name.txt",
                3,
                Whitespace::Exact,
                None
            )
            .is_err(),
            "{hostile:?}"
        );
    }
    let nowhere = diff_file_guarded(
        &repo,
        &before,
        &after,
        Some("never/was.txt"),
        "moved/new name.txt",
        3,
        Whitespace::Exact,
        None,
    )
    .unwrap()
    .unwrap();
    assert_eq!(nowhere.kind, FileChangeKind::Added);
}

#[test]
fn a_file_over_the_bound_is_measured_and_not_compared() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let big: String = (0..20_000)
        .map(|n| format!("row {n} of the big file\n"))
        .collect();
    copy.write("data/big.txt", &big).unwrap();
    copy.write("small.txt", "one\n").unwrap();
    commit_all(&git, "a big file and a small one");
    let bigger = format!("{big}and one more row\n");
    copy.write("data/big.txt", &bigger).unwrap();
    copy.write("small.txt", "one\ntwo\n").unwrap();
    commit_all(&git, "both grow");
    copy.write("data/big.txt", &format!("{bigger}still growing\n"))
        .unwrap();
    let repo = open(&copy);
    let size_of = |spec: &str| -> u64 {
        git.run(["cat-file", "-s", spec])
            .unwrap()
            .trim()
            .parse()
            .unwrap()
    };

    // Between two commits.
    let options = |max_bytes| DiffOptions {
        max_bytes,
        ..Default::default()
    };
    let bounded = diff_trees(
        &repo,
        &side(&repo, "HEAD~1"),
        &side(&repo, "HEAD"),
        &options(Some(10_000)),
    )
    .unwrap();
    let by_path: BTreeMap<&str, _> = bounded
        .iter()
        .map(|file| (file.path.as_str(), file))
        .collect();
    let large = by_path["data/big.txt"];
    assert!(large.too_large && large.hunks.is_empty() && !large.binary);
    assert_eq!(large.old_size, Some(size_of("HEAD~1:data/big.txt")));
    assert_eq!(large.new_size, Some(size_of("HEAD:data/big.txt")));
    assert_eq!(large.kind, FileChangeKind::Modified);
    assert_eq!(
        large.new_id.map(|id| id.to_hex()),
        Some(git.rev_parse("HEAD:data/big.txt").unwrap())
    );
    let small = by_path["small.txt"];
    assert!(!small.too_large);
    assert_eq!((small.additions, small.deletions), (1, 0));
    assert_eq!(small.new_size, Some(size_of("HEAD:small.txt")));
    // Without the bound, and with one it is under, it is compared.
    for max in [None, Some(10_000_000)] {
        let all = diff_trees(
            &repo,
            &side(&repo, "HEAD~1"),
            &side(&repo, "HEAD"),
            &options(max),
        )
        .unwrap();
        let large = all.iter().find(|file| file.path == "data/big.txt").unwrap();
        assert!(!large.too_large);
        assert_eq!((large.additions, large.deletions), (1, 0));
        assert_eq!(large.new_size, Some(size_of("HEAD:data/big.txt")));
    }

    // One path, on every pair of sides the viewer shows.
    let on_disk = std::fs::metadata(copy.path().join("data/big.txt"))
        .unwrap()
        .len();
    for (old, new, old_size, new_size) in [
        ("HEAD", "worktree", size_of("HEAD:data/big.txt"), on_disk),
        ("index", "worktree", size_of(":data/big.txt"), on_disk),
        (
            "HEAD~1",
            "HEAD",
            size_of("HEAD~1:data/big.txt"),
            size_of("HEAD:data/big.txt"),
        ),
        (
            "HEAD~1",
            "index",
            size_of("HEAD~1:data/big.txt"),
            size_of(":data/big.txt"),
        ),
    ] {
        let guarded = diff_file_guarded(
            &repo,
            &side(&repo, old),
            &side(&repo, new),
            None,
            "data/big.txt",
            3,
            Whitespace::Exact,
            Some(10_000),
        )
        .unwrap()
        .unwrap();
        assert!(
            guarded.too_large && guarded.hunks.is_empty(),
            "{old}..{new}"
        );
        assert_eq!(
            (guarded.old_size, guarded.new_size),
            (Some(old_size), Some(new_size)),
            "{old}..{new}"
        );
        let whole = diff_file_guarded(
            &repo,
            &side(&repo, old),
            &side(&repo, new),
            None,
            "data/big.txt",
            3,
            Whitespace::Exact,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(!whole.too_large && !whole.hunks.is_empty(), "{old}..{new}");
        assert_eq!(
            (whole.old_size, whole.new_size),
            (Some(old_size), Some(new_size)),
            "{old}..{new}"
        );
    }
    // A file that came, and one that went, over the bound.
    let came = diff_file_guarded(
        &repo,
        &side(&repo, "HEAD~2"),
        &side(&repo, "HEAD"),
        None,
        "data/big.txt",
        3,
        Whitespace::Exact,
        Some(10),
    )
    .unwrap()
    .unwrap();
    assert_eq!((came.kind, came.too_large), (FileChangeKind::Added, true));
    assert_eq!(came.old_size, None);
    let went = diff_file_guarded(
        &repo,
        &side(&repo, "HEAD"),
        &side(&repo, "HEAD~2"),
        None,
        "data/big.txt",
        3,
        Whitespace::Exact,
        Some(10),
    )
    .unwrap()
    .unwrap();
    assert_eq!((went.kind, went.too_large), (FileChangeKind::Deleted, true));
    // A bound of nothing guards everything that is not empty, and no
    // bound is too large for a number.
    for max in [0, u64::MAX] {
        let diff = diff_file_guarded(
            &repo,
            &side(&repo, "HEAD~1"),
            &side(&repo, "HEAD"),
            None,
            "small.txt",
            3,
            Whitespace::Exact,
            Some(max),
        )
        .unwrap()
        .unwrap();
        assert_eq!(diff.too_large, max == 0);
    }
}

#[test]
fn a_binary_file_is_told_by_its_size_and_id_on_both_sides() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let file = copy.path().join("blob.bin");
    std::fs::write(&file, [0u8, 1, 2, 3, 255, 0, 9]).unwrap();
    commit_all(&git, "a binary file");
    std::fs::write(&file, [0u8, 1, 2, 3, 255, 0, 9, 7, 7, 7]).unwrap();
    commit_all(&git, "it grows");
    std::fs::write(&file, [9u8, 0, 0, 0, 1]).unwrap();
    let repo = open(&copy);

    let committed = diff_trees(
        &repo,
        &side(&repo, "HEAD~1"),
        &side(&repo, "HEAD"),
        &DiffOptions::default(),
    )
    .unwrap();
    let diff = &committed[0];
    assert!(diff.binary && diff.hunks.is_empty());
    assert_eq!((diff.old_size, diff.new_size), (Some(7), Some(10)));
    assert_eq!(
        diff.old_id.map(|id| id.to_hex()),
        Some(git.rev_parse("HEAD~1:blob.bin").unwrap())
    );
    assert_eq!(
        diff.new_id.map(|id| id.to_hex()),
        Some(git.rev_parse("HEAD:blob.bin").unwrap())
    );

    // The file of the worktree has the id git would give its bytes.
    let hashed = git
        .run(["hash-object", "--no-filters", "--", "blob.bin"])
        .unwrap()
        .trim()
        .to_owned();
    for old in ["HEAD", "index"] {
        let diff = diff_file_guarded(
            &repo,
            &side(&repo, old),
            &DiffSide::Worktree,
            None,
            "blob.bin",
            3,
            Whitespace::Exact,
            None,
        )
        .unwrap()
        .unwrap();
        assert!(diff.binary);
        assert_eq!((diff.old_size, diff.new_size), (Some(10), Some(5)));
        assert_eq!(
            diff.old_id.map(|id| id.to_hex()),
            Some(git.rev_parse("HEAD:blob.bin").unwrap())
        );
        assert_eq!(diff.new_id.map(|id| id.to_hex()), Some(hashed.clone()));
    }
}

/// A path that leads out of the worktree is not read as a file of the
/// worktree, which would show anything the user can read as an added file.
#[test]
fn a_path_that_leads_out_of_the_worktree_is_refused_not_read() {
    let copy = Scratch::of("linear").unwrap();
    let outside = copy.path().with_extension("secret");
    std::fs::write(&outside, "not the repository's to show\n").unwrap();
    let name = outside.file_name().unwrap().to_str().unwrap().to_owned();
    let repo = open(&copy);
    for path in [
        format!("../{name}"),
        format!("src/../../{name}"),
        outside.to_str().unwrap().to_owned(),
        String::new(),
    ] {
        for (old, new) in [
            ("HEAD", "worktree"),
            ("index", "worktree"),
            ("HEAD~1", "HEAD"),
        ] {
            let result =
                bitbraid_git::diff_file(&repo, &side(&repo, old), &side(&repo, new), &path, 3);
            assert!(result.is_err(), "{path:?} {old}..{new}: {result:?}");
        }
        let listed = diff_mixed(
            &repo,
            &side(&repo, "HEAD"),
            &DiffSide::Worktree,
            &DiffOptions {
                paths: vec![path.clone()],
                ..Default::default()
            },
        );
        assert!(listed.is_err(), "{path:?}: {listed:?}");
    }
    let _ = std::fs::remove_file(&outside);
}

/// `git diff --name-status` as path -> letter.
fn name_status(git: &Git, args: &[&str]) -> BTreeMap<String, char> {
    let mut all = vec!["diff", "--no-renames", "--name-status"];
    all.extend_from_slice(args);
    git.lines(all)
        .unwrap()
        .into_iter()
        .map(|line| {
            let (letter, path) = line.split_once('\t').unwrap();
            (path.to_owned(), letter.chars().next().unwrap())
        })
        .collect()
}

fn letter(kind: FileChangeKind) -> char {
    match kind {
        FileChangeKind::Added => 'A',
        FileChangeKind::Deleted => 'D',
        FileChangeKind::Modified => 'M',
        FileChangeKind::Renamed => 'R',
        FileChangeKind::Copied => 'C',
        FileChangeKind::TypeChanged => 'T',
    }
}

#[test]
fn a_commit_is_compared_with_the_working_tree_and_the_index_as_git_compares_them() {
    let copy = Scratch::of("branches-merges").unwrap();
    let git = copy.git();
    // More history first, so that old commits differ from HEAD in files
    // the worktree has not touched.
    copy.write("later.txt", "added late\n").unwrap();
    copy.write("a.txt", "a1\nrewritten since\n").unwrap();
    commit_all(&git, "later changes");
    let back = git.run(["show", "HEAD~1:a.txt"]).unwrap();
    // The worktree: one file put back to what it was a commit ago, one
    // edited, one deleted, one staged, one new and not tracked, a binary.
    copy.write("a.txt", &back).unwrap();
    copy.write("base.txt", "edited in the worktree\n").unwrap();
    copy.remove("later.txt").unwrap();
    copy.write("staged.txt", "staged\n").unwrap();
    git.run(["add", "--", "staged.txt"]).unwrap();
    copy.write("untracked dir/new file.txt", "new\n").unwrap();
    std::fs::write(copy.path().join("pic.bin"), [0u8, 159, 146, 150]).unwrap();
    let repo = open(&copy);
    let untracked: Vec<String> = git
        .lines(["ls-files", "--others", "--exclude-standard"])
        .unwrap();
    assert_eq!(untracked.len(), 2);

    let commits = git.lines(["rev-list", "--all"]).unwrap();
    assert!(commits.len() >= 4);
    for commit in &commits {
        // Against the worktree: what git lists, and the files it does not
        // track yet as added.
        let ours: BTreeMap<String, char> = diff_mixed(
            &repo,
            &side(&repo, commit),
            &DiffSide::Worktree,
            &DiffOptions::default(),
        )
        .unwrap()
        .into_iter()
        .map(|file| (file.path, letter(file.kind)))
        .collect();
        let mut expected = name_status(&git, &[commit.as_str()]);
        for path in &untracked {
            expected.insert(path.clone(), 'A');
        }
        assert_eq!(ours, expected, "{commit} against the worktree");

        // Lines added and removed, file by file.
        let counted: BTreeMap<String, (u32, u32)> = diff_mixed(
            &repo,
            &side(&repo, commit),
            &DiffSide::Worktree,
            &DiffOptions::default(),
        )
        .unwrap()
        .into_iter()
        .filter(|file| !file.binary && !untracked.contains(&file.path))
        .map(|file| (file.path, (file.additions, file.deletions)))
        .collect();
        let git_counted: BTreeMap<String, (u32, u32)> = git
            .lines(["diff", "--no-renames", "--numstat", commit.as_str()])
            .unwrap()
            .into_iter()
            .filter_map(|line| {
                let mut fields = line.split('\t');
                let added = fields.next()?.parse().ok()?;
                let removed = fields.next()?.parse().ok()?;
                Some((fields.next()?.to_owned(), (added, removed)))
            })
            .collect();
        assert_eq!(counted, git_counted, "{commit} against the worktree");

        // Against the index.
        let staged: BTreeMap<String, char> = diff_mixed(
            &repo,
            &side(&repo, commit),
            &DiffSide::Index,
            &DiffOptions::default(),
        )
        .unwrap()
        .into_iter()
        .map(|file| (file.path, letter(file.kind)))
        .collect();
        assert_eq!(
            staged,
            name_status(&git, &["--cached", commit.as_str()]),
            "{commit} against the index"
        );
    }
    // The file that was put back is no difference to the commit it was
    // put back to, though git's status has it as changed.
    let since = diff_mixed(
        &repo,
        &side(&repo, "HEAD~1"),
        &DiffSide::Worktree,
        &DiffOptions::default(),
    )
    .unwrap();
    assert!(since.iter().all(|file| file.path != "a.txt"), "{since:?}");
    // Over the bound, a file is listed and not read.
    let guarded = diff_mixed(
        &repo,
        &side(&repo, "HEAD"),
        &DiffSide::Worktree,
        &DiffOptions {
            max_bytes: Some(3),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(guarded.iter().any(|file| file.too_large));
    assert!(
        guarded
            .iter()
            .all(|file| !file.too_large || file.hunks.is_empty())
    );
    // Only the paths that are asked for.
    let only = diff_mixed(
        &repo,
        &side(&repo, "HEAD"),
        &DiffSide::Worktree,
        &DiffOptions {
            paths: vec!["base.txt".to_owned(), "../outside".to_owned()],
            ..Default::default()
        },
    );
    assert!(only.is_err(), "a path outside is refused");
    let only = diff_mixed(
        &repo,
        &side(&repo, "HEAD"),
        &DiffSide::Worktree,
        &DiffOptions {
            paths: vec!["base.txt".to_owned(), "nowhere.txt".to_owned()],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(only.len(), 1);
    assert_eq!(only[0].path, "base.txt");
}
