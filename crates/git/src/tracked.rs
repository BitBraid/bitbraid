//! The paths the index tracks, as `git ls-files` lists them: what the
//! terminal completes paths from.
//!
//! Sources: git-ls-files(1); gitformat-index(5) (entries sorted by path,
//! a conflicted path once per stage); gix documentation
//! (`Repository::index_or_empty`, `State::entries`).

use crate::{Repository, Result, git_err};

/// The paths the index tracks, in its order (by path), each once, at
/// most `most` of them, and whether it tracks more. A path that is not
/// UTF-8 is left out: it cannot be typed as text.
pub fn tracked_paths(repo: &Repository, most: usize) -> Result<(Vec<String>, bool)> {
    let index = repo
        .gix()
        .index_or_empty()
        .map_err(git_err(say!("repo-loading-index")))?;
    let mut out: Vec<String> = Vec::new();
    for entry in index.entries() {
        let Ok(path) = std::str::from_utf8(entry.path(&index)) else {
            continue;
        };
        // In a conflict a path is there once per side.
        if out.last().is_some_and(|last| last == path) {
            continue;
        }
        if out.len() >= most {
            return Ok((out, true));
        }
        out.push(path.to_owned());
    }
    Ok((out, false))
}
