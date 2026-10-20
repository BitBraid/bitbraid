//! Find a file by its path: the files of a commit's tree, the ones whose
//! path holds the letters that were typed, the best first.
//!
//! The match is the one people expect from a file finder: the letters of
//! the query appear in the path in that order, not necessarily next to one
//! another, whatever their case. Among the paths that match, one is better
//! when its letters are in the file's name rather than in its folders,
//! when they stand together, when they start a word (after a slash, a dot,
//! a dash or an underscore, or at a capital), and when the path is short.
//!
//! Bounded: at most [`MAX_SCANNED`] files of the tree are looked at (the
//! answer says when there were more), at most `limit` are returned, and
//! the walk stops when the flag it is given is set.
//!
//! Sources: none (the scoring is ours; that a file finder matches letters
//! in order is common to every editor). gix documentation (trees).

use std::sync::atomic::{AtomicBool, Ordering};

use gix::bstr::ByteSlice;
use serde::Serialize;

use crate::{DiffSide, Error, Repository, Result, git_err};

/// At most this many files of a tree are looked at.
pub const MAX_SCANNED: usize = 500_000;
/// The longest query that is taken.
pub const MAX_QUERY: usize = 256;
/// The most files one answer holds.
pub const MAX_LIMIT: usize = 1000;
/// Folders nested deeper than this are not gone into.
const MAX_DEPTH: usize = 256;

/// One file that matches.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileHit {
    /// Its path in the tree.
    pub path: String,
    /// How well it matches: higher is better.
    pub score: i64,
}

/// What a search for a file found.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Files {
    /// The best matches, the best first.
    pub files: Vec<FileHit>,
    /// How many files matched in all.
    pub matched: usize,
    /// How many files were looked at.
    pub scanned: usize,
    /// The tree has more files than were looked at.
    pub more: bool,
}

/// How well `path` matches `query`, or none when it does not: the letters
/// of the query, without its blanks, have to appear in the path in order.
/// An empty query matches everything alike.
///
/// Of the ways the letters can be found in the path, the best one counts:
/// "parser" is in `parser.rs` as the six letters of the name, not as five
/// of them and the `r` of the ending.
pub fn score(query: &str, path: &str) -> Option<i64> {
    let wanted: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect();
    if wanted.is_empty() {
        return Some(0);
    }
    let letters: Vec<char> = path.chars().collect();
    if wanted.len() > letters.len() {
        return None;
    }
    // The letters in order at all? Most paths are told apart here.
    let mut rest = wanted.iter();
    let mut next = rest.next();
    for letter in &letters {
        if let Some(want) = next
            && same_letter(*letter, *want)
        {
            next = rest.next();
        }
    }
    if next.is_some() {
        return None;
    }
    // Where the file's name begins.
    let name_at = letters
        .iter()
        .rposition(|c| *c == '/')
        .map_or(0, |slash| slash.saturating_add(1));
    // What a letter of the query is worth at each place of the path.
    let worth: Vec<i64> = letters
        .iter()
        .enumerate()
        .map(|(place, here)| {
            let before = place.checked_sub(1).and_then(|at| letters.get(at)).copied();
            // The start of a word.
            let starts = match before {
                None => true,
                Some(before) => {
                    matches!(before, '/' | '.' | '-' | '_' | ' ')
                        || (before.is_lowercase() && here.is_uppercase())
                }
            };
            // A letter in the file's name counts for more than one in a
            // folder.
            (if place >= name_at { 16 } else { 4 }) + if starts { 10 } else { 0 }
        })
        .collect();
    // best[place]: the most the letters so far can come to with the last
    // of them at this place; none where it cannot stand.
    let mut best: Vec<Option<i64>> = vec![None; letters.len()];
    for (nth, want) in wanted.iter().enumerate() {
        let mut row: Vec<Option<i64>> = vec![None; letters.len()];
        // The most the letters before this one come to, ending anywhere
        // left of the place looked at.
        let mut before_any: Option<i64> = None;
        for (place, letter) in letters.iter().enumerate() {
            let left = place
                .checked_sub(1)
                .and_then(|at| best.get(at).copied().flatten());
            if nth > 0 {
                before_any = before_any.max(left);
            }
            if !same_letter(*letter, *want) {
                continue;
            }
            let here = worth.get(place).copied().unwrap_or(0);
            let value = if nth == 0 {
                // The query begins the name.
                Some(here + if place == name_at { 20 } else { 0 })
            } else {
                // Right after the letter before it, or anywhere after.
                let together = left.map(|left| left.saturating_add(16));
                together
                    .max(before_any)
                    .map(|so_far| so_far.saturating_add(here))
            };
            if let Some(slot) = row.get_mut(place) {
                *slot = value;
            }
        }
        best = row;
    }
    let total = best.into_iter().flatten().max()?;
    // Short paths before long ones, among equals.
    let length = i64::try_from(letters.len()).unwrap_or(i64::MAX);
    Some(total.saturating_mul(4).saturating_sub(length.min(400)))
}

/// Whether a letter of a path is the letter that was typed, whatever its
/// case. The typed one is in lower case already.
fn same_letter(letter: char, want: char) -> bool {
    letter == want || letter.to_lowercase().eq(std::iter::once(want))
}

fn invalid(detail: impl Into<String>) -> Error {
    Error::Invalid {
        what: say!("find-what-file-search"),
        detail: detail.into(),
    }
}

/// The files of the tree of `rev` (HEAD when absent) whose path matches
/// `query`, the best `limit` of them. A repository without commits has
/// none. Stops with an error when `cancel` is set.
pub fn files(
    repo: &Repository,
    rev: Option<&str>,
    query: &str,
    limit: usize,
    cancel: &AtomicBool,
) -> Result<Files> {
    if query.len() > MAX_QUERY {
        return Err(invalid(say!("find-query-too-long", max = MAX_QUERY)));
    }
    if query.chars().any(char::is_control) {
        return Err(invalid(say!("find-query-control")));
    }
    let limit = limit.min(MAX_LIMIT);
    let r = repo.gix();
    let root = match rev {
        Some(rev) => match DiffSide::parse(repo, rev)? {
            DiffSide::Commit(id) => r
                .find_commit(*id.as_gix())
                .map_err(git_err(say!("find-loading-commit")))?
                .tree()
                .map_err(git_err(say!("find-loading-tree")))?,
            DiffSide::Tree(id) => r
                .find_tree(*id.as_gix())
                .map_err(git_err(say!("find-loading-tree")))?,
            _ => return Err(invalid(say!("find-revision-not-commit"))),
        },
        None => match r.head_commit() {
            Ok(commit) => commit.tree().map_err(git_err(say!("find-loading-tree")))?,
            // A branch without commits has no files yet.
            Err(_) => return Ok(Files::default()),
        },
    };

    let mut out = Files::default();
    let mut best: Vec<FileHit> = Vec::new();
    // Folders still to go into, the next one last: (tree, its path, depth).
    let mut pending = vec![(root.id, String::new(), 0usize)];
    'walk: while let Some((tree_id, prefix, depth)) = pending.pop() {
        if cancel.load(Ordering::SeqCst) {
            return Err(invalid(say!("find-cancelled")));
        }
        let Ok(tree) = r.find_tree(tree_id) else {
            continue; // a folder whose tree is missing has no files to find
        };
        let mut folders: Vec<(gix::ObjectId, String, usize)> = Vec::new();
        for entry in tree.iter() {
            let Ok(entry) = entry else { break };
            let name = entry.filename().to_str_lossy();
            let path = if prefix.is_empty() {
                name.into_owned()
            } else {
                format!("{prefix}/{name}")
            };
            let mode = entry.mode();
            if mode.is_tree() {
                if depth < MAX_DEPTH {
                    folders.push((entry.object_id(), path, depth.saturating_add(1)));
                }
                continue;
            }
            // A submodule is a folder of another repository, not a file.
            if mode.is_commit() {
                continue;
            }
            if out.scanned >= MAX_SCANNED {
                out.more = true;
                break 'walk;
            }
            out.scanned = out.scanned.saturating_add(1);
            if let Some(score) = score(query, &path) {
                out.matched = out.matched.saturating_add(1);
                best.push(FileHit { path, score });
                // Kept small as it goes: a tree of half a million files is
                // not sorted whole for the fifty that are wanted.
                if best.len() >= limit.saturating_mul(4).max(256) {
                    keep_best(&mut best, limit);
                }
            }
        }
        // In the order they stand in the tree.
        pending.extend(folders.into_iter().rev());
    }
    keep_best(&mut best, limit);
    out.files = best;
    Ok(out)
}

/// Sort by score, the best first, the path deciding among equals, and keep
/// `limit`.
fn keep_best(hits: &mut Vec<FileHit>, limit: usize) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.path.cmp(&b.path)));
    hits.truncate(limit);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The paths in the order the scores put them, best first.
    fn ranked<'a>(query: &str, paths: &[&'a str]) -> Vec<&'a str> {
        let mut hits: Vec<(&str, i64)> = paths
            .iter()
            .filter_map(|path| Some((*path, score(query, path)?)))
            .collect();
        hits.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
        hits.into_iter().map(|(path, _)| path).collect()
    }

    #[test]
    fn letters_match_in_order_whatever_their_case() {
        assert!(score("grs", "crates/ui/src/views/graph.rs").is_some());
        assert!(score("GRAPH", "crates/ui/src/views/graph.rs").is_some());
        assert!(score("graph", "crates/UI/GRAPH.RS").is_some());
        assert!(score("uiviews", "crates/ui/src/views/graph.rs").is_some());
        assert!(
            score("g r s", "graph.rs").is_some(),
            "blanks are no letters"
        );
        // Not in that order, or not there.
        assert_eq!(score("srg", "graph.rs"), None);
        assert_eq!(score("graphx", "graph.rs"), None);
        assert_eq!(score("x", ""), None);
        // Nothing typed: everything, alike.
        assert_eq!(score("", "a/b"), Some(0));
        assert_eq!(score("   ", "a/b"), Some(0));
        // Letters beyond ASCII, and a path of nothing but them.
        assert!(score("é", "docs/résumé.md").is_some());
        assert!(score("É", "docs/résumé.md").is_some());
        assert!(score("日本", "docs/日本語.txt").is_some());
        assert_eq!(score("本日", "docs/日本語.txt"), None);
    }

    #[test]
    fn the_file_name_counts_for_more_than_the_folders() {
        let paths = [
            "crates/graph/src/lib.rs",
            "crates/ui/src/views/graph.rs",
            "docs/photography/readme.md",
            "g/r/a/p/h/notes.txt",
        ];
        let order = ranked("graph", &paths);
        assert_eq!(order[0], "crates/ui/src/views/graph.rs");
        assert_eq!(order.len(), 4);
        // Letters together before letters apart, a word's start before
        // its middle, and the shorter path among equals.
        assert_eq!(
            ranked("log", &["src/catalog.rs", "src/log.rs", "src/l_o_g.rs"])[0],
            "src/log.rs"
        );
        assert_eq!(
            ranked("sc", &["src/search_cmds.rs", "src/discard.rs"])[0],
            "src/search_cmds.rs"
        );
        assert_eq!(
            ranked("gv", &["src/GraphView.tsx", "src/segv.txt"])[0],
            "src/GraphView.tsx"
        );
        assert_eq!(
            ranked("mod", &["a/very/long/way/down/mod.rs", "a/mod.rs"])[0],
            "a/mod.rs"
        );
        // The best way the letters can be found counts: the six letters of
        // the name, not five of them and the `r` of the ending.
        assert_eq!(
            ranked("parser", &["docs/par_ser.md", "src/deep/down/parser.rs"])[0],
            "src/deep/down/parser.rs"
        );
        assert!(score("parser", "parser.rs") > score("parser", "pa/r/s/e/r.rs"));
        assert!(score("ab", "ab") > score("ab", "a/b"));
    }

    #[test]
    fn hostile_paths_and_queries_only_score() {
        let long = "a/".repeat(5000);
        assert!(score("aaa", &long).is_some());
        assert!(score(&"a".repeat(300), &long).is_some());
        assert_eq!(score(&"b".repeat(300), &long), None);
        for path in ["", "/", "//", "\u{0}", "a\nb", "\u{202e}evil", "🦀/🦀.rs"] {
            for query in ["", "a", "/", "🦀", "\u{0}"] {
                let _ = score(query, path);
            }
        }
        assert!(score("🦀rs", "🦀/🦀.rs").is_some());
    }

    #[test]
    fn the_best_are_kept_in_order() {
        let mut hits: Vec<FileHit> = [("b", 5), ("a", 5), ("c", 9), ("d", 1)]
            .into_iter()
            .map(|(path, score)| FileHit {
                path: path.to_owned(),
                score,
            })
            .collect();
        keep_best(&mut hits, 3);
        let paths: Vec<&str> = hits.iter().map(|hit| hit.path.as_str()).collect();
        assert_eq!(paths, ["c", "a", "b"]);
        keep_best(&mut hits, 0);
        assert!(hits.is_empty());
    }
}
