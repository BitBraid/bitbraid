//! What rerere has recorded, one conflict at a time, and forgetting one of
//! them whether or not its conflict is there now.
//!
//! Git keeps a directory per conflict under `rr-cache`, named by a hash of
//! the conflict: `preimage` is the file with its conflict markers, brought
//! to a form that does not depend on which side was whose, and `postimage`
//! the file as it was resolved. The path the conflict was in is not kept,
//! so a record is shown by what it holds: the two sides of its first
//! conflict and what was made of them.
//!
//! A record is forgotten by removing its directory, which is what `git
//! rerere gc` does to the ones that have grown old. The record of a
//! conflict that is being resolved right now (git lists it in `MERGE_RR`
//! until the file is resolved) has no resolution yet and is left alone;
//! a resolution git has just put into a file again is forgotten for that
//! file ([`crate::rerere::forget`], `git rerere forget`).
//!
//! Sources: git-rerere(1) (`forget`, `gc`), gitrepository-layout(5)
//! (`rr-cache`, `MERGE_RR`).

use std::{
    fs,
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

use serde::Serialize;

use crate::{Error, Repository, Result};

/// Records beyond this many are not listed.
const MOST: usize = 2000;
/// A side or a resolution is shown up to this many lines.
const SHOWN_LINES: usize = 12;
/// A record's file is read up to this size.
const MAX_READ: u64 = 1024 * 1024;

/// One conflict rerere has a record of.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Record {
    /// The name git gives the conflict: a hash of it.
    pub id: String,
    /// A resolution is recorded. Without one the conflict was seen and
    /// has not been resolved yet.
    pub resolved: bool,
    /// When the resolution was recorded (or the conflict first seen), in
    /// unix seconds.
    pub time: i64,
    /// How many conflicts the file had.
    pub conflicts: usize,
    /// The first conflict's one side, its first lines.
    pub ours: Vec<String>,
    /// Its other side.
    pub theirs: Vec<String>,
    /// What the first conflict was resolved to, where that can be told:
    /// the file had one conflict and was changed nowhere else.
    pub resolution: Option<Vec<String>>,
    /// The file the conflict is in, when it is being resolved right now
    /// and git is waiting to record how.
    pub path: Option<String>,
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("rerere-what-rerere"),
        detail: detail.into(),
    }
}

fn cache_dir(repo: &Repository) -> PathBuf {
    repo.gix().common_dir().join("rr-cache")
}

/// Whether a name is one git gives a conflict: a hash in hex.
pub fn is_id(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn read_small(file: &Path) -> Option<String> {
    let meta = file.symlink_metadata().ok()?;
    if !meta.is_file() || meta.len() > MAX_READ {
        return None;
    }
    Some(String::from_utf8_lossy(&fs::read(file).ok()?).into_owned())
}

fn modified(file: &Path) -> Option<i64> {
    let at = file.symlink_metadata().ok()?.modified().ok()?;
    i64::try_from(at.duration_since(UNIX_EPOCH).ok()?.as_secs()).ok()
}

/// A file with conflicts taken apart: what is before the first conflict,
/// its two sides, what follows it, and how many conflicts there are.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Taken<'a> {
    before: Vec<&'a str>,
    ours: Vec<&'a str>,
    theirs: Vec<&'a str>,
    after: Vec<&'a str>,
    conflicts: usize,
}

/// Take a recorded conflict apart. Git writes its markers bare in a
/// record: seven of `<`, `=`, `>` (and `|` before what both started from,
/// when that is kept), each on a line of its own.
pub fn take_apart(preimage: &str) -> Taken<'_> {
    #[derive(PartialEq)]
    enum At {
        Before,
        Ours,
        Base,
        Theirs,
        After,
    }
    let mut taken = Taken::default();
    let mut at = At::Before;
    for line in preimage.lines() {
        let marker = |mark: char| {
            line.len() >= 7
                && line.chars().take(7).all(|c| c == mark)
                && line.chars().nth(7).is_none_or(|c| c == ' ')
        };
        if marker('<') {
            taken.conflicts = taken.conflicts.saturating_add(1);
            if at == At::Before {
                at = At::Ours;
                continue;
            }
        }
        match at {
            At::Before => taken.before.push(line),
            At::Ours if marker('|') => at = At::Base,
            At::Ours | At::Base if marker('=') => at = At::Theirs,
            At::Ours => taken.ours.push(line),
            At::Base => {}
            At::Theirs if marker('>') => at = At::After,
            At::Theirs => taken.theirs.push(line),
            At::After => taken.after.push(line),
        }
    }
    taken
}

/// What the one conflict of a file was resolved to: the lines of the
/// resolved file between what stood before the conflict and what stood
/// after it. None when the file had more conflicts than one, or was
/// changed outside the conflict as well.
pub fn resolution_of(taken: &Taken<'_>, postimage: &str) -> Option<Vec<String>> {
    if taken.conflicts != 1 {
        return None;
    }
    let lines: Vec<&str> = postimage.lines().collect();
    let (head, tail) = (taken.before.len(), taken.after.len());
    let end = lines.len().checked_sub(tail)?;
    if head > end || lines.get(..head)? != taken.before.as_slice() {
        return None;
    }
    if lines.get(end..)? != taken.after.as_slice() {
        return None;
    }
    Some(
        lines
            .get(head..end)?
            .iter()
            .take(SHOWN_LINES)
            .map(|line| (*line).to_owned())
            .collect(),
    )
}

/// The conflicts git is keeping a record for in the merge that is under
/// way: the id of each and the path it is in.
fn in_progress(repo: &Repository) -> Vec<(String, String)> {
    let Ok(bytes) = fs::read(repo.git_dir().join("MERGE_RR")) else {
        return Vec::new();
    };
    String::from_utf8_lossy(&bytes)
        .split('\0')
        .filter_map(|entry| {
            let (id, path) = entry.split_once('\t')?;
            // A second way of resolving the same conflict is `id.N`.
            let id = id.split('.').next().unwrap_or(id);
            is_id(id).then(|| (id.to_owned(), path.to_owned()))
        })
        .collect()
}

/// What rerere has recorded, the newest first.
pub fn records(repo: &Repository, limit: usize) -> Vec<Record> {
    let Ok(entries) = fs::read_dir(cache_dir(repo)) else {
        return Vec::new();
    };
    let current = in_progress(repo);
    let mut records: Vec<Record> = Vec::new();
    for entry in entries.flatten().take(MOST) {
        let name = entry.file_name();
        let Some(id) = name.to_str().filter(|name| is_id(name)) else {
            continue;
        };
        let dir = entry.path();
        if !dir.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
            continue;
        }
        let (preimage, postimage) = (dir.join("preimage"), dir.join("postimage"));
        let resolved = postimage.is_file();
        let before = read_small(&preimage).unwrap_or_default();
        let taken = take_apart(&before);
        let shown = |lines: &[&str]| -> Vec<String> {
            lines
                .iter()
                .take(SHOWN_LINES)
                .map(|line| (*line).to_owned())
                .collect()
        };
        records.push(Record {
            id: id.to_owned(),
            resolved,
            time: modified(&postimage)
                .or_else(|| modified(&preimage))
                .unwrap_or(0),
            conflicts: taken.conflicts,
            ours: shown(&taken.ours),
            theirs: shown(&taken.theirs),
            resolution: read_small(&postimage).and_then(|after| resolution_of(&taken, &after)),
            path: current
                .iter()
                .find(|(known, _)| known == id)
                .map(|(_, path)| path.clone()),
        });
    }
    records.sort_by(|a, b| b.time.cmp(&a.time).then_with(|| a.id.cmp(&b.id)));
    records.truncate(limit);
    records
}

/// Forget one record. True when there was one.
pub fn forget_record(repo: &Repository, id: &str) -> Result<bool> {
    if !is_id(id) {
        return Err(Error::Invalid {
            what: say!("rerere-what-record"),
            detail: say!("rerere-not-a-record", id = format!("{id:?}")),
        });
    }
    let dir = cache_dir(repo).join(id);
    // Only a directory of the record itself: a link is not followed.
    if !dir.symlink_metadata().is_ok_and(|meta| meta.is_dir()) {
        return Ok(false);
    }
    // Git is waiting to record how this very conflict is resolved: there
    // is nothing recorded yet, and its directory is git's to write to.
    if let Some((_, path)) = in_progress(repo).into_iter().find(|(known, _)| known == id) {
        return Err(refused(say!("rerere-being-resolved", path)));
    }
    fs::remove_dir_all(&dir)
        .map_err(|err| refused(say!("rerere-not-removed", err = err.to_string())))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_recorded_conflict_is_taken_apart_at_its_bare_markers() {
        let preimage = "top\n<<<<<<<\nmain's a\n=======\ntopic's a\n>>>>>>>\nbottom\nend\n";
        let taken = take_apart(preimage);
        assert_eq!(taken.conflicts, 1);
        assert_eq!(taken.before, ["top"]);
        assert_eq!(taken.ours, ["main's a"]);
        assert_eq!(taken.theirs, ["topic's a"]);
        assert_eq!(taken.after, ["bottom", "end"]);
        // What both started from is left out; a second conflict is counted.
        let with_base = "<<<<<<<\none\n|||||||\nbase\n=======\ntwo\n>>>>>>>\nmid\n<<<<<<<\nx\n=======\ny\n>>>>>>>\n";
        let taken = take_apart(with_base);
        assert_eq!(taken.conflicts, 2);
        assert_eq!((taken.ours, taken.theirs), (vec!["one"], vec!["two"]));
        // A line that only starts like a marker is text.
        let text = "<<<<<<<<<< banner\n=======x\nplain\n";
        let taken = take_apart(text);
        assert_eq!(taken.conflicts, 0);
        assert_eq!(taken.before.len(), 3);
        assert_eq!(take_apart(""), Taken::default());
    }

    #[test]
    fn a_resolution_is_what_stands_where_the_conflict_stood() {
        let preimage = "top\n<<<<<<<\nmain's a\n=======\ntopic's a\n>>>>>>>\nbottom\n";
        let taken = take_apart(preimage);
        assert_eq!(
            resolution_of(&taken, "top\nboth of them\nand more\nbottom\n"),
            Some(vec!["both of them".to_owned(), "and more".to_owned()])
        );
        // Resolved to nothing at all.
        assert_eq!(resolution_of(&taken, "top\nbottom\n"), Some(vec![]));
        // Changed outside the conflict too: not told.
        assert_eq!(resolution_of(&taken, "TOP\nboth\nbottom\n"), None);
        assert_eq!(resolution_of(&taken, "top\n"), None);
        assert_eq!(resolution_of(&taken, ""), None);
        let two = take_apart("<<<<<<<\na\n=======\nb\n>>>>>>>\n<<<<<<<\nc\n=======\nd\n>>>>>>>\n");
        assert_eq!(resolution_of(&two, "a\nd\n"), None);
    }

    #[test]
    fn only_a_hash_names_a_record() {
        assert!(is_id(&"a1".repeat(20)));
        assert!(is_id(&"0f".repeat(32)));
        for bad in [
            "",
            "abc",
            "../../etc",
            &"g".repeat(40),
            &"a".repeat(41),
            "a/b",
            ".",
        ] {
            assert!(!is_id(bad), "{bad:?}");
        }
    }
}
