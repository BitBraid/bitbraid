//! Line endings held to git: how the lines of each side end, as
//! `git ls-files --eol` tells them; a change of line endings alone, as
//! `git diff --ignore-cr-at-eol` shows nothing of it; and the hunks of
//! both ways of comparing, line for line, between commits and between the
//! index and a worktree that `core.autocrlf` and the `eol` attribute
//! convert.

use std::collections::BTreeMap;

use bitbraid_git::{
    DiffOptions, DiffSide, FileDiff, LineEndings, LogOptions, Repository, Whitespace,
    diff_file_with, diff_trees, log,
};
use bitbraid_testkit::{Git, Scratch};

use super::oracle_worktree::{git_hunks, ours, porcelain};

/// Hunks as git prints them, without the text after the second `@@`, and
/// with git's line after a last line that has no line feed.
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
            if line.no_newline {
                out.push("\\ No newline at end of file".to_owned());
            }
        }
    }
    out
}

/// How git names the line endings of a text, in what `ls-files --eol`
/// prints: none for `none` (no line ends) and for nothing at all.
fn named(word: &str) -> Option<LineEndings> {
    match word {
        "lf" => Some(LineEndings::Lf),
        "crlf" => Some(LineEndings::Crlf),
        "mixed" => Some(LineEndings::Mixed),
        "none" | "" => None,
        other => panic!("git says {other:?} of a text"),
    }
}

/// `git ls-files --eol`: for each path, the line endings of what the
/// index holds and of the worktree's file.
fn eol_info(git: &Git) -> BTreeMap<String, (Option<LineEndings>, Option<LineEndings>)> {
    let text = git.run(["ls-files", "--eol"]).unwrap();
    text.lines()
        .map(|line| {
            let (info, path) = line.split_once('\t').unwrap();
            let mut words = info.split_whitespace();
            let index = words.next().unwrap().strip_prefix("i/").unwrap();
            let worktree = words.next().unwrap().strip_prefix("w/").unwrap();
            (path.to_owned(), (named(index), named(worktree)))
        })
        .collect()
}

fn options(whitespace: Whitespace) -> DiffOptions {
    DiffOptions {
        whitespace,
        ..DiffOptions::default()
    }
}

/// Every commit of the fixture against its parent: the line endings of
/// each side are what git says of that blob, a file whose change is only
/// its line endings is said to be one exactly where `--ignore-cr-at-eol`
/// leaves nothing of it, and the hunks of both ways are git's.
#[test]
fn line_endings_between_commits_are_gits() {
    let scratch = Scratch::of("line-endings").unwrap();
    let git = scratch.git();
    let repo = Repository::open(scratch.path()).unwrap();
    let commits = log(&repo, &LogOptions::default()).unwrap();
    assert_eq!(commits.len(), 5);
    // What git says of each blob, commit by commit: the index once the
    // commit is checked out.
    let mut said = BTreeMap::new();
    for commit in &commits {
        git.run(["checkout", "-q", &commit.id.to_hex()]).unwrap();
        said.insert(commit.id, eol_info(&git));
    }
    git.run(["checkout", "-q", "main"]).unwrap();

    let mut only_seen = Vec::new();
    for commit in &commits {
        let Some(parent) = commit.parents.first().copied() else {
            continue;
        };
        let (from, to) = (parent.to_hex(), commit.id.to_hex());
        let exact = diff_trees(
            &repo,
            &DiffSide::Commit(parent),
            &DiffSide::Commit(commit.id),
            &options(Whitespace::Exact),
        )
        .unwrap();
        let loose = diff_trees(
            &repo,
            &DiffSide::Commit(parent),
            &DiffSide::Commit(commit.id),
            &options(Whitespace::IgnoreCrAtEol),
        )
        .unwrap();
        assert_eq!(exact.len(), loose.len());
        for (file, loose) in exact.iter().zip(&loose) {
            let path = file.path.as_str();
            let at = |id| said[&id].get(path).and_then(|(index, _)| *index);
            assert_eq!(file.line_endings.old, at(parent), "{path} in {to}");
            assert_eq!(file.line_endings.new, at(commit.id), "{path} in {to}");
            assert_eq!(file.line_endings.worktree, None, "{path} in {to}");
            let plain = git_hunks(&git, &[&from, &to], path);
            let ignoring = git_hunks(&git, &["--ignore-cr-at-eol", &from, &to], path);
            assert_eq!(hunk_text(&file.hunks), plain, "{path} in {to}");
            assert_eq!(hunk_text(&loose.hunks), ignoring, "{path} in {to}");
            assert_eq!(
                file.line_endings.only_change,
                !plain.is_empty() && ignoring.is_empty(),
                "{path} in {to}"
            );
            assert_eq!(file.line_endings, loose.line_endings, "{path} in {to}");
            if file.line_endings.only_change {
                only_seen.push(path.to_owned());
            }
        }
    }
    // The fixture has one change of line endings alone; the edit of a
    // CRLF file, the LF line added to one and the file turned to LF with a
    // line changed are not.
    assert_eq!(only_seen, ["lf.txt"]);
}

/// The worktree against the index, the worktree's file converted as git
/// would store it: what git shows, with `--ignore-cr-at-eol` and without;
/// the endings of the index's blob and of the file on disk as
/// `git ls-files --eol` has them; and the status as git's.
#[test]
fn the_worktree_is_compared_as_git_stores_it() {
    let scratch = Scratch::of("line-endings").unwrap();
    let git = scratch.git();
    for autocrlf in ["false", "true", "input"] {
        git.run(["config", "core.autocrlf", autocrlf]).unwrap();
        git.run(["checkout", "-q", "--", "."]).unwrap();
        // lf.txt is stored with CRLF: here with LF, its line endings
        // alone change, which autocrlf leaves alone (a file stored with
        // CRLF is not converted).
        scratch.write("lf.txt", "one\ntwo\nthree\n").unwrap();
        // mixed.txt is stored with LF: here with CRLF and a line more.
        scratch.write("mixed.txt", "a\r\nB\r\nc\r\nd\r\n").unwrap();
        // The attribute says LF: CRLF here is LF to git.
        scratch.write("text.lf", "one\r\ntwo\r\nthree\r\n").unwrap();
        // The attribute says CRLF: a line changed, LF here.
        scratch.write("text.crlf", "one\nTWO\nthree\n").unwrap();
        // A new line ending at the end of a file that had none.
        scratch
            .write("crlf.txt", "one\r\nTWO\r\nthree\r\nfour")
            .unwrap();

        let repo = Repository::open(scratch.path()).unwrap();
        assert_eq!(ours(&repo), porcelain(&git), "autocrlf={autocrlf}");
        let eol = eol_info(&git);
        for path in ["lf.txt", "mixed.txt", "text.lf", "text.crlf", "crlf.txt"] {
            let diff = |whitespace| -> FileDiff {
                diff_file_with(
                    &repo,
                    &DiffSide::Index,
                    &DiffSide::Worktree,
                    path,
                    3,
                    whitespace,
                )
                .unwrap()
                .unwrap()
            };
            let (exact, loose) = (diff(Whitespace::Exact), diff(Whitespace::IgnoreCrAtEol));
            let plain = git_hunks(&git, &[], path);
            let ignoring = git_hunks(&git, &["--ignore-cr-at-eol"], path);
            let case = format!("{path} with autocrlf={autocrlf}");
            assert_eq!(hunk_text(&exact.hunks), plain, "{case}");
            assert_eq!(hunk_text(&loose.hunks), ignoring, "{case}");
            let endings = exact.line_endings;
            let (index, on_disk) = eol[path];
            assert_eq!(endings.old, index, "{case}");
            assert_eq!(endings.worktree.or(endings.new), on_disk, "{case}");
            // Told of the worktree only where git converts it.
            if let Some(worktree) = endings.worktree {
                assert_ne!(Some(worktree), endings.new, "{case}");
            }
            assert_eq!(
                endings.only_change,
                !plain.is_empty() && ignoring.is_empty(),
                "{case}"
            );
        }
        // CRLF that git turns into LF on the way in is no change to the
        // diff, though the status, as git's, lists the file: its size is
        // not the one the index recorded, and git takes that for a change
        // without reading it.
        let text_lf = diff_file_with(
            &repo,
            &DiffSide::Index,
            &DiffSide::Worktree,
            "text.lf",
            3,
            Whitespace::Exact,
        )
        .unwrap()
        .unwrap();
        assert!(text_lf.hunks.is_empty(), "autocrlf={autocrlf}");
        assert_eq!(text_lf.line_endings.worktree, Some(LineEndings::Crlf));
        assert!(!text_lf.line_endings.only_change);
    }
}

/// A change of line endings alone in the worktree, of a file git does not
/// convert: every line differs, and the diff says that is all.
#[test]
fn a_file_turned_to_crlf_is_a_change_of_line_endings_alone() {
    let scratch = Scratch::of("line-endings").unwrap();
    let git = scratch.git();
    scratch.write("mixed.txt", "a\r\nB\r\nc\r\n").unwrap();
    let repo = Repository::open(scratch.path()).unwrap();
    let file = diff_file_with(
        &repo,
        &DiffSide::Index,
        &DiffSide::Worktree,
        "mixed.txt",
        3,
        Whitespace::Exact,
    )
    .unwrap()
    .unwrap();
    assert_eq!((file.additions, file.deletions), (3, 3));
    assert!(file.line_endings.only_change);
    assert_eq!(file.line_endings.old, Some(LineEndings::Lf));
    assert_eq!(file.line_endings.new, Some(LineEndings::Crlf));
    assert_eq!(file.line_endings.worktree, None);
    assert!(git_hunks(&git, &["--ignore-cr-at-eol"], "mixed.txt").is_empty());
    // Ignoring whitespace at the ends of lines takes the CR as such.
    let trailing = diff_file_with(
        &repo,
        &DiffSide::Index,
        &DiffSide::Worktree,
        "mixed.txt",
        3,
        Whitespace::IgnoreTrailing,
    )
    .unwrap()
    .unwrap();
    assert!(trailing.hunks.is_empty());
    assert!(trailing.line_endings.only_change);
}
