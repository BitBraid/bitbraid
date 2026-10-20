//! Partial clones: what a partial clone left on its remote is fetched
//! when something here needs it.
//!
//! Git does that by itself whenever it misses an object. What is read
//! here goes through gitoxide, which does not: a diff of an old commit
//! fails on content that never came. So a read that fails in a partial
//! clone has git do what the same read in git would need (a `git diff`
//! that counts the lines of the files it compares, which fetches the
//! trees on the way and the content of those files in one batch), and is
//! tried once more. Outside a partial clone a read that fails is not tried
//! again.
//!
//! Only what somebody asked for is fetched. Work nobody asked for (the
//! views kept current, the repository looked over in the background) runs
//! under [`exe::local_only`]: git fetches nothing for it, and a read here
//! that misses what a partial clone left out says so instead of fetching
//! ([`left_on_remote`]).
//!
//! Sources: git's documentation of partial clone (`partial-clone.adoc`:
//! promisor remotes, objects fetched on demand, the batch fetch a diff
//! makes before it compares), git-config(1) (`remote.<name>.promisor`),
//! git-diff(1) (`--numstat`, `--cached`, `-M`).

use crate::{DiffSide, Error, Repository, Result, exe};

/// Whether the repository is a partial clone: some remote is one that
/// git fetches missing objects from.
pub fn is_partial(repo: &Repository) -> bool {
    exe::run(
        repo,
        [
            "config",
            "--get-regexp",
            r"^remote\..*\.promisor$",
            "^true$",
        ],
        None,
    )
    .is_ok_and(|out| !out.stdout.trim().is_empty())
}

/// A side of a diff as git names it on its command line, when it names a
/// tree; the index and the worktree have nothing to fetch.
fn tree_word(repo: &Repository, side: &DiffSide) -> Option<String> {
    match side {
        DiffSide::Commit(id) | DiffSide::Tree(id) => Some(id.to_hex()),
        DiffSide::Empty => Some(repo.gix().empty_tree().id.to_string()),
        DiffSide::Index | DiffSide::Worktree => None,
    }
}

/// Have git fetch what comparing `old` with `new` needs, for `paths`
/// only when some are given.
pub(crate) fn fetch_for_diff(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    paths: &[&str],
) -> Result<()> {
    let mut args: Vec<String> = ["diff", "--numstat", "-M", "--no-ext-diff", "--no-textconv"]
        .map(str::to_owned)
        .to_vec();
    // A tree against the index or the worktree reads the same objects
    // whichever side it is on.
    let (tree, other) = match (tree_word(repo, old), tree_word(repo, new)) {
        (Some(old), Some(new)) => {
            args.extend([old, new]);
            (None, None)
        }
        (Some(tree), None) => (Some(tree), Some(new)),
        (None, Some(tree)) => (Some(tree), Some(old)),
        // The index against the worktree: everything is here.
        (None, None) => return Ok(()),
    };
    if let Some(tree) = tree {
        if matches!(other, Some(DiffSide::Index)) {
            args.push("--cached".to_owned());
        }
        args.push(tree);
    }
    if !paths.is_empty() {
        args.push("--".to_owned());
        args.extend(paths.iter().map(|path| (*path).to_owned()));
    }
    exe::run(repo, &args, None).map(|_| ())
}

/// Have git fetch what the history of `path` needs: each of its versions,
/// and what finding where it was moved from compares.
pub(crate) fn fetch_for_history(repo: &Repository, path: &str) -> Result<()> {
    exe::run(
        repo,
        [
            "log",
            "--all",
            "--format=%H",
            "--numstat",
            "-M",
            "--follow",
            "--",
            path,
        ],
        None,
    )
    .map(|_| ())
}

/// Have git fetch what the blame of `path` as of `rev` needs.
pub(crate) fn fetch_for_blame(repo: &Repository, path: &str, rev: Option<&str>) -> Result<()> {
    let rev = rev.unwrap_or("HEAD");
    exe::run(repo, ["blame", "-s", rev, "--", path], None).map(|_| ())
}

/// What is said of a read that failed in a partial clone while it may
/// read only what is here (see [`exe::local_only`]). The interface looks
/// for these words to offer to fetch.
pub fn left_on_remote() -> &'static str {
    say!("promisor-left-on-remote")
}

/// Read with `read`; in a partial clone, when that fails, have `fetch`
/// bring what it needs and read once more. The first error is the one
/// that is given when the fetch fails too.
///
/// Work nobody asked for ([`exe::local_only`]) fetches nothing: a read
/// that fails in a partial clone then says that what it needs was left on
/// the remote, which is not a repository that is broken. A read that
/// failed because the open handle has no room for the packs on disk
/// ([`crate::repo::is_out_of_room`]) is no missing object either: no fetch
/// mends it, and it is handed back for the handle to be opened again.
pub(crate) fn fetching<T>(
    repo: &Repository,
    read: impl Fn() -> Result<T>,
    fetch: impl FnOnce() -> Result<()>,
) -> Result<T> {
    match read() {
        Ok(found) => Ok(found),
        Err(err) => {
            if crate::repo::is_out_of_room(&err.to_string()) || !is_partial(repo) {
                return Err(err);
            }
            if exe::is_local_only() {
                return Err(Error::Refused {
                    what: say!("promisor-what-not-on-this-machine"),
                    detail: say!(
                        "promisor-fetched-only-when-asked",
                        left = left_on_remote(),
                        error = err.to_string()
                    ),
                });
            }
            if fetch().is_err() {
                return Err(err);
            }
            read()
        }
    }
}
