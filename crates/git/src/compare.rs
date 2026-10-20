//! A commit compared with the working tree or the index: every file that
//! differs between the two, not only the files that have uncommitted
//! changes.
//!
//! The candidates are the paths that differ between the commit and HEAD
//! (a tree diff, without rename detection, so that both the old and the
//! new place of a moved file are looked at) together with the paths git's
//! status lists. Each is then compared as it is in the commit with what
//! the worktree (or the index) holds, and the ones that are the same on
//! both sides are left out: a file changed since the commit and changed
//! back is no difference.
//!
//! Like `git diff <commit>`, with two things said: a moved file shows as
//! one file deleted and one added, and files git does not track yet are
//! listed as added when the worktree is the other side (`git diff` leaves
//! them out).
//!
//! Sources: git-diff(1) (`git diff <commit>`, `git diff --cached
//! <commit>`), git-status(1).

use std::collections::BTreeSet;

use crate::{
    DiffOptions, DiffSide, FileDiff, Repository, Result,
    diff::{FileChangeKind, diff_file_guarded, diff_trees},
};

/// Whether a side is a tree: a commit, a tree or the empty tree.
fn is_tree(side: &DiffSide) -> bool {
    !matches!(side, DiffSide::Index | DiffSide::Worktree)
}

/// Whether a diff of one path says the two sides differ.
pub fn differs(file: &FileDiff) -> bool {
    if file.kind != FileChangeKind::Modified {
        return true;
    }
    if file.too_large {
        // Not looked into: the blob ids say, where both are known.
        return match (file.old_id, file.new_id) {
            (Some(old), Some(new)) => old != new,
            _ => true,
        };
    }
    if file.is_submodule() || file.binary {
        return file.old_id != file.new_id;
    }
    !file.hunks.is_empty()
}

/// The files that differ between a tree and the index or the worktree
/// (in either order), or between the index and the worktree. In a
/// partial clone what the diff needs and was left on the remote is
/// fetched ([`crate::promisor`]).
pub fn diff_mixed(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    options: &DiffOptions,
) -> Result<Vec<FileDiff>> {
    crate::promisor::fetching(
        repo,
        || diff_mixed_here(repo, old, new, options),
        || {
            let paths: Vec<&str> = options.paths.iter().map(String::as_str).collect();
            crate::promisor::fetch_for_diff(repo, old, new, &paths)
        },
    )
}

/// [`diff_mixed`] with what is here.
fn diff_mixed_here(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    options: &DiffOptions,
) -> Result<Vec<FileDiff>> {
    let mut paths: BTreeSet<String> = BTreeSet::new();
    if options.paths.is_empty() {
        // What differs between the commit and HEAD, by path alone.
        if let Some(tree) = [old, new].into_iter().find(|side| is_tree(side))
            && let Ok(head) = DiffSide::parse(repo, "HEAD")
            && *tree != head
        {
            let by_path = DiffOptions {
                context: 0,
                paths: Vec::new(),
                detect_renames: false,
                whitespace: Default::default(),
                // Only the paths are wanted here, not what is in them.
                max_bytes: Some(0),
            };
            for file in diff_trees(repo, tree, &head, &by_path)? {
                paths.insert(file.path);
            }
        }
        // And what is changed since HEAD, staged or not.
        for entry in crate::status(repo)? {
            paths.extend(entry.original_path);
            paths.insert(entry.path);
        }
    } else {
        paths.extend(options.paths.iter().cloned());
    }
    let mut files = Vec::new();
    for path in paths {
        if let Some(file) = diff_file_guarded(
            repo,
            old,
            new,
            None,
            &path,
            options.context,
            options.whitespace,
            options.max_bytes,
        )? && differs(&file)
        {
            files.push(file);
        }
    }
    Ok(files)
}
