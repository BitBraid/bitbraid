//! What an operation that leaves HEAD where it is did to some files and
//! their entries in the index, kept to take it back and to do it again:
//! a cherry-pick without committing and a squash merge, which stage what
//! they merged, and a patch applied with a three-way merge, which may
//! leave files in conflict.
//!
//! The paths the operation can write are named before it runs, and each
//! one's file and index entries (every stage of one in conflict) are read
//! before and after. Taking it back puts the files and the entries back as
//! they were, doing it again as the operation left them; each only while
//! every one of those paths is still as the other side has it, or nothing
//! is changed and the error names the path that was changed since. An
//! operation that changed the index where it was not expected to (a merge
//! that follows a renamed folder may) is not recorded, and neither is one
//! that names more paths than are read one by one.
//!
//! Sources: git-update-index(1) (`--index-info`: a mode of 0 takes a path
//! out, then its stages go in), git-ls-files(1) (`--stage`), git-diff(1)
//! (`--name-only`, `--no-renames`, `--cached`), git-cherry-pick(1)
//! (`--no-commit`), git-merge(1) (`--squash`), git-apply(1) (`--3way`).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::{
    Error, Repository, Result, exe,
    stage::{self, Backup},
    stash::Staged,
};

/// Most paths an operation may name for it to be recorded: each is read
/// before and after.
pub const MAX_RECORDED: usize = crate::stash::MAX_RECORDED;

/// What an operation did to the files and the index entries of its paths.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FilesChange {
    /// The paths, sorted.
    pub paths: Vec<String>,
    /// Their files before; one for each path, in the same order.
    pub files_before: Vec<Backup>,
    /// Their files after.
    pub files_after: Vec<Backup>,
    /// Their index entries before, every stage; a path without one had
    /// none.
    pub index_before: Vec<Staged>,
    /// Their index entries after.
    pub index_after: Vec<Staged>,
    /// The messages git had prepared for the next commit before.
    #[serde(default)]
    pub prepared_before: Prepared,
    /// The same after: a pick without commit prepares the picked commit's
    /// message, a squash its own.
    #[serde(default)]
    pub prepared_after: Prepared,
}

/// The messages git prepares in the git directory for the commit that is
/// to follow (`MERGE_MSG`, `SQUASH_MSG`), which go and come back with what
/// prepared them, as `git reset --merge` takes them away with it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Prepared {
    /// `MERGE_MSG`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merge: Option<String>,
    /// `SQUASH_MSG`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub squash: Option<String>,
}

/// Longest prepared message that is kept.
const MAX_MESSAGE: u64 = 1024 * 1024;

const MERGE_MSG: &str = "MERGE_MSG";
const SQUASH_MSG: &str = "SQUASH_MSG";

/// The prepared messages there are now.
fn prepared(repo: &Repository) -> Prepared {
    let read = |name: &str| {
        let file = repo.git_dir().join(name);
        std::fs::symlink_metadata(&file)
            .ok()
            .filter(|meta| meta.is_file() && meta.len() <= MAX_MESSAGE)
            .and_then(|_| std::fs::read(&file).ok())
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    };
    Prepared {
        merge: read(MERGE_MSG),
        squash: read(SQUASH_MSG),
    }
}

/// Have the prepared messages be `wanted`.
fn put_prepared(repo: &Repository, wanted: &Prepared) -> Result<()> {
    let failed = |err: std::io::Error| Error::Git {
        context: say!("files-change-writing-prepared"),
        source: Box::new(err),
    };
    for (name, text) in [(MERGE_MSG, &wanted.merge), (SQUASH_MSG, &wanted.squash)] {
        let file = repo.git_dir().join(name);
        match text {
            Some(text) => std::fs::write(&file, text).map_err(failed)?,
            None => {
                if std::fs::symlink_metadata(&file).is_ok() {
                    std::fs::remove_file(&file).map_err(failed)?;
                }
            }
        }
    }
    Ok(())
}

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

fn split_z(text: &str) -> Vec<String> {
    text.split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The paths that differ between two commits, a renamed file as both of
/// its paths.
pub fn paths_between(repo: &Repository, old: &str, new: &str) -> Result<Vec<String>> {
    let out = exe::run(
        repo,
        [
            "diff",
            "--name-only",
            "--no-renames",
            "-z",
            "--end-of-options",
            old,
            new,
            "--",
        ],
        None,
    )?;
    Ok(split_z(&out.stdout))
}

/// The index entries of `paths`, every stage of each, as git lists them.
fn entries(repo: &Repository, paths: &[String]) -> Result<Vec<Staged>> {
    let mut found = Vec::new();
    // In pieces: a command line has an end.
    for some in paths.chunks(200) {
        let mut args = vec!["--literal-pathspecs", "ls-files", "--stage", "-z", "--"];
        args.extend(some.iter().map(String::as_str));
        let out = exe::run(repo, args, None)?;
        for record in out.stdout.split('\0').filter(|record| !record.is_empty()) {
            let Some((about, path)) = record.split_once('\t') else {
                continue;
            };
            let mut about = about.split(' ');
            let (Some(mode), Some(blob), Some(stage)) = (about.next(), about.next(), about.next())
            else {
                continue;
            };
            let (Ok(blob), Ok(stage)) = (blob.parse(), stage.parse::<u8>()) else {
                continue;
            };
            found.push(Staged {
                path: path.to_owned(),
                mode: mode.to_owned(),
                blob,
                stage,
            });
        }
    }
    found.sort_by(|a, b| (&a.path, a.stage).cmp(&(&b.path, b.stage)));
    Ok(found)
}

/// The paths whose index entries differ from HEAD's, or are in conflict.
fn changed_in_index(repo: &Repository) -> Result<BTreeSet<String>> {
    let out = exe::run(
        repo,
        [
            "diff",
            "--cached",
            "--name-only",
            "--no-renames",
            "-z",
            "HEAD",
            "--",
        ],
        None,
    )?;
    Ok(split_z(&out.stdout).into_iter().collect())
}

/// Run `op`, reading the files and index entries of `paths` before it and
/// after: what it did to them. Nothing is recorded when it changed nothing
/// there, when it changed the index outside them, when they are more than
/// [`MAX_RECORDED`], or when they could not be read after it ran (it ran
/// all the same, and that is not taken back by an error).
pub fn recorded<T>(
    repo: &Repository,
    mut paths: Vec<String>,
    op: impl FnOnce() -> Result<T>,
) -> Result<(T, Option<FilesChange>)> {
    paths.sort();
    paths.dedup();
    if paths.is_empty() || paths.len() > MAX_RECORDED {
        return Ok((op()?, None));
    }
    // What cannot be read before (a folder where a file is to be) leaves
    // the operation unrecorded, not undone.
    let before =
        (|| -> Result<_> { Ok((stage::backup_paths(repo, &paths)?, entries(repo, &paths)?)) })();
    let Ok((files_before, index_before)) = before else {
        return Ok((op()?, None));
    };
    let index_was = changed_in_index(repo).ok();
    let prepared_before = prepared(repo);
    let out = op()?;
    let after = || -> Result<Option<FilesChange>> {
        let Some(was) = &index_was else {
            return Ok(None);
        };
        let outside = changed_in_index(repo)?
            .into_iter()
            .any(|path| !was.contains(&path) && paths.binary_search(&path).is_err());
        if outside {
            return Ok(None);
        }
        let files_after = stage::backup_paths(repo, &paths)?;
        let index_after = entries(repo, &paths)?;
        if files_after == files_before && index_after == index_before {
            return Ok(None);
        }
        Ok(Some(FilesChange {
            paths: paths.clone(),
            files_before: files_before.clone(),
            files_after,
            index_before: index_before.clone(),
            index_after,
            prepared_before: prepared_before.clone(),
            prepared_after: prepared(repo),
        }))
    };
    Ok((out, after().unwrap_or(None)))
}

/// Put the paths' files and index entries as `files` and `index` have
/// them, if they are as `now_files` and `now_index` have them; `since`
/// names what that was ("the patch was applied").
fn put(
    repo: &Repository,
    paths: &[String],
    (now_files, now_index): (&[Backup], &[Staged]),
    (files, index, messages): (&[Backup], &[Staged], &Prepared),
    what: &'static str,
    since: &str,
) -> Result<()> {
    if let Some(other) = crate::rewrite::in_progress(repo) {
        return Err(refused(what, say!("files-change-in-progress", other)));
    }
    let changed = |path: &str| refused(what, say!("files-change-changed-since", path, since));
    // What the journal says comes from a file anybody can write: it is
    // to hold together before any of it is fed to git.
    let sound_entry = |entry: &Staged| {
        entry.stage <= 3
            && !entry.mode.is_empty()
            && entry.mode.len() <= 6
            && entry.mode.bytes().all(|byte| (b'0'..=b'7').contains(&byte))
            && paths.contains(&entry.path)
    };
    let sound = paths.len() == now_files.len()
        && paths.len() == files.len()
        && paths
            .iter()
            .all(|path| !path.contains('\0') && stage::check_rel(path).is_ok())
        && index.iter().all(sound_entry)
        && now_index.iter().all(sound_entry);
    if !sound {
        return Err(refused(what, say!("files-change-unsound")));
    }
    for (there, expected) in stage::backup_paths(repo, paths)?.iter().zip(now_files) {
        if there.blob != expected.blob || there.path != expected.path {
            return Err(changed(&expected.path));
        }
    }
    let staged = entries(repo, paths)?;
    if staged != now_index {
        let of = |entries: &[Staged], path: &str| -> Vec<Staged> {
            entries
                .iter()
                .filter(|entry| entry.path == path)
                .cloned()
                .collect()
        };
        let moved = paths
            .iter()
            .find(|path| of(&staged, path) != of(now_index, path))
            .map_or(say!("files-change-the-index"), String::as_str);
        return Err(changed(moved));
    }
    for file in files {
        stage::restore_backup(repo, file)?;
    }
    // Each path out of the index, then the entries it is to have.
    let null = repo.gix().object_hash().null().to_hex().to_string();
    let mut records = String::new();
    for path in paths {
        records.push_str(&format!("0 {null}\t{path}\0"));
        for entry in index.iter().filter(|entry| &entry.path == path) {
            records.push_str(&format!(
                "{} {} {}\t{}\0",
                entry.mode,
                entry.blob.to_hex(),
                entry.stage,
                entry.path
            ));
        }
    }
    exe::run(
        repo,
        ["update-index", "-z", "--index-info"],
        Some(records.as_bytes()),
    )?;
    put_prepared(repo, messages)
}

/// Take the change back: the files and index entries as they were before
/// the operation, while they are still as it left them, and the messages
/// git had prepared then. `since` names the operation for an error ("the
/// cherry-pick").
pub fn take_back(repo: &Repository, change: &FilesChange, since: &str) -> Result<()> {
    put(
        repo,
        &change.paths,
        (&change.files_after, &change.index_after),
        (
            &change.files_before,
            &change.index_before,
            &change.prepared_before,
        ),
        say!("files-change-what-undo"),
        since,
    )
}

/// Do the change again: the files and index entries as the operation left
/// them, while they are still as they were before it.
pub fn do_again(repo: &Repository, change: &FilesChange, since: &str) -> Result<()> {
    put(
        repo,
        &change.paths,
        (&change.files_before, &change.index_before),
        (
            &change.files_after,
            &change.index_after,
            &change.prepared_after,
        ),
        say!("files-change-what-redo"),
        since,
    )
}
