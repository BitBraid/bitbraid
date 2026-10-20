//! Rerere: git's record of how conflicts were resolved, which it replays
//! when the same conflict comes up again. Whether it is on, how much it
//! has recorded, which of the files in conflict it has resolved from that
//! record, and forgetting what it recorded for one.
//!
//! A file rerere resolved is still unmerged in the index (unless
//! `rerere.autoUpdate` had it staged): git leaves it to be looked at. Git
//! also takes such a file off the list of conflicts it keeps for rerere in
//! `MERGE_RR`, so it is told by what is left: it is in conflict, and it is
//! not among the files `git rerere remaining` lists as still to do.
//!
//! Sources: git-rerere(1) (`remaining`, `forget`), git-config(1)
//! (`rerere.enabled`, `rerere.autoUpdate`), gitrepository-layout(5)
//! (`rr-cache`, `MERGE_RR`), git-ls-files(1) (`--resolve-undo`).

use std::{fs, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe, sequence, stage};

/// More recorded conflicts than this are not counted.
const MOST: usize = 100_000;
/// More ways of resolving one conflict than this are not counted.
const MOST_EACH: usize = 1000;

/// Where rerere stands in this repository.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Status {
    /// It is on: `rerere.enabled` says so, or says nothing and there is a
    /// record from before.
    pub enabled: bool,
    /// How many resolutions are recorded.
    pub recorded: usize,
    /// Files in conflict that were resolved from the record and wait to be
    /// accepted.
    pub reused: Vec<String>,
    /// Files in conflict that are still to be resolved.
    pub remaining: Vec<String>,
}

/// Which configuration a setting is written to.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// This repository's.
    #[default]
    Repo,
    /// The user's, for every repository.
    User,
}

/// What forgetting did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Forgotten {
    /// The file.
    pub path: String,
    /// A recorded resolution is gone. False when there was none.
    pub forgotten: bool,
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("rerere-what-rerere"),
        detail: detail.into(),
    }
}

/// Where the record is kept. Worktrees share it.
fn cache_dir(repo: &Repository) -> PathBuf {
    repo.gix().common_dir().join("rr-cache")
}

/// Whether rerere is on, as git decides it: the setting when there is one,
/// and otherwise whether anything was ever recorded here.
pub fn enabled(repo: &Repository) -> bool {
    let setting = exe::run(
        repo,
        ["config", "--type=bool", "--get", "rerere.enabled"],
        None,
    )
    .ok()
    .map(|out| out.stdout.trim().to_owned());
    match setting.as_deref() {
        Some("true") => true,
        Some("false") => false,
        _ => cache_dir(repo).is_dir(),
    }
}

/// How many resolutions are recorded, from the names in the record alone.
/// A conflict has a directory; a resolution of it is a `postimage` there,
/// or `postimage.N` where the same conflict was resolved in another way.
pub fn recorded(repo: &Repository) -> usize {
    let Ok(conflicts) = fs::read_dir(cache_dir(repo)) else {
        return 0;
    };
    let mut count = 0usize;
    for conflict in conflicts.flatten().take(MOST) {
        let Ok(files) = fs::read_dir(conflict.path()) else {
            continue;
        };
        for file in files.flatten().take(MOST_EACH) {
            let name = file.file_name();
            let name = name.to_string_lossy();
            let numbered = name.strip_prefix("postimage.").is_some_and(|number| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            });
            if name == "postimage" || numbered {
                count = count.saturating_add(1);
            }
        }
    }
    count
}

/// The file has no conflict left in it: it is a plain file, small enough
/// to look at, without git's markers.
fn settled(repo: &Repository, path: &str) -> bool {
    let Ok(file) = stage::worktree_path(repo, path) else {
        return false;
    };
    let small = file.symlink_metadata().is_ok_and(|meta| {
        meta.is_file() && meta.len() <= u64::try_from(stage::MAX_EDIT).unwrap_or(u64::MAX)
    });
    small && fs::read(&file).is_ok_and(|text| !sequence::has_markers(&text))
}

/// Those of the paths in conflict that rerere has resolved from its
/// record. Costs no process unless rerere has been at work here.
pub(crate) fn resolved_among(repo: &Repository, unmerged: &[&str]) -> Vec<String> {
    if unmerged.is_empty() {
        return Vec::new();
    }
    // Rerere writes MERGE_RR when it looks at a conflict, and replays from
    // what is under rr-cache: without the two it has resolved nothing.
    if !repo.git_dir().join("MERGE_RR").is_file() || !cache_dir(repo).is_dir() {
        return Vec::new();
    }
    let Ok(out) = exe::run(repo, ["rerere", "remaining"], None) else {
        return Vec::new();
    };
    let remaining: Vec<&str> = out.stdout.lines().collect();
    let candidates: Vec<&str> = unmerged
        .iter()
        .copied()
        // One path a line is all git gives: a path with a line break in it
        // cannot be told from two, and is left alone.
        .filter(|path| !path.contains('\n') && !remaining.contains(path))
        .collect();
    // Switched off, rerere lists nothing as remaining, having done nothing.
    if candidates.is_empty() || !enabled(repo) {
        return Vec::new();
    }
    candidates
        .into_iter()
        .filter(|path| settled(repo, path))
        .map(str::to_owned)
        .collect()
}

/// Whether rerere is on, how many resolutions it has recorded, and of the
/// files in conflict right now, which it resolved from the record and
/// which are still to do.
pub fn status(repo: &Repository) -> Result<Status> {
    let (mut reused, mut remaining) = (Vec::new(), Vec::new());
    if repo.workdir().is_some() {
        for conflict in sequence::conflicts(repo)? {
            if conflict.reused {
                reused.push(conflict.path);
            } else {
                remaining.push(conflict.path);
            }
        }
    }
    Ok(Status {
        enabled: enabled(repo),
        recorded: recorded(repo),
        reused,
        remaining,
    })
}

/// Switch rerere on or off, in this repository's configuration or the
/// user's. Off is written as off: left unset, rerere stays on wherever it
/// has recorded something before.
pub fn enable(repo: &Repository, on: bool, scope: Scope) -> Result<Status> {
    let file = match scope {
        Scope::Repo => "--local",
        Scope::User => "--global",
    };
    let value = if on { "true" } else { "false" };
    exe::run(repo, ["config", file, "rerere.enabled", value], None)?;
    status(repo)
}

/// Forget what rerere recorded for the conflict `path` is in, so that it
/// is resolved afresh and that is what is recorded. The file itself is not
/// touched: what rerere put into it stays until it is resolved again.
pub fn forget(repo: &Repository, path: &str) -> Result<Forgotten> {
    stage::worktree_path(repo, path)?;
    if !enabled(repo) {
        return Err(refused(say!("rerere-not-on")));
    }
    // Git forgets the resolution of a conflict that is there, or was and
    // can be put back; for any other path it does nothing and says so to
    // nobody.
    let in_conflict = sequence::conflicts(repo)?
        .iter()
        .any(|conflict| conflict.path == path);
    let was = || -> Result<bool> {
        let resolved = exe::run(repo, ["ls-files", "--resolve-undo", "-z"], None)?;
        Ok(resolved
            .stdout
            .split('\0')
            .filter_map(|entry| entry.split_once('\t'))
            .any(|(_, resolved_path)| resolved_path == path))
    };
    if !in_conflict && !was()? {
        return Err(refused(say!("rerere-not-in-conflict", path)));
    }
    let before = recorded(repo);
    exe::run(
        repo,
        ["--literal-pathspecs", "rerere", "forget", "--", path],
        None,
    )?;
    Ok(Forgotten {
        path: path.to_owned(),
        forgotten: recorded(repo) < before,
    })
}
