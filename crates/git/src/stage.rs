//! Index and worktree mutations: stage and unstage paths, hunks and lines,
//! discard with a backup, ignore. Paths, hunks and lines are written to the
//! index through libgit2 (`git2`), which applies the same rules as git for
//! modes, stat data and sorting; staging everything and putting whole files
//! back run git itself.
//!
//! Sources: git-add(1), git-reset(1), git-restore(1), gitignore(5), libgit2
//! index documentation.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Oid, Repository, Result,
    diff::{DiffSide, Hunk, LineKind, diff_file},
};

fn git2_repo(repo: &Repository) -> Result<git2::Repository> {
    git2::Repository::open(repo.git_dir()).map_err(|err| Error::Git {
        context: say!("stage-opening-libgit2"),
        source: Box::new(err),
    })
}

fn g2<E: std::error::Error + Send + Sync + 'static>(
    context: &'static str,
) -> impl FnOnce(E) -> Error {
    move |err| Error::Git {
        context,
        source: Box::new(err),
    }
}

pub(crate) fn check_rel(path: &str) -> Result<()> {
    if path.is_empty() || !bitbraid_core::paths::stays_inside(path) {
        return Err(Error::Invalid {
            what: say!("stage-what-path"),
            detail: say!("stage-path-outside", path = format!("{path:?}")),
        });
    }
    Ok(())
}

fn workdir(repo: &Repository) -> Result<&Path> {
    repo.workdir().ok_or(Error::Invalid {
        what: say!("stage-what-repository"),
        detail: say!("stage-bare").to_owned(),
    })
}

/// Stage whole paths (adds, modifications and deletions), like `git add -A -- paths`.
pub fn stage_paths(repo: &Repository, paths: &[String]) -> Result<()> {
    let wd = workdir(repo)?;
    let g = git2_repo(repo)?;
    let mut index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
    for path in paths {
        check_rel(path)?;
        if wd.join(path).symlink_metadata().is_ok() {
            index
                .add_path(Path::new(path))
                .map_err(g2(say!("stage-staging-a-path")))?;
        } else {
            // Deleted in the worktree: stage the deletion.
            index
                .remove_path(Path::new(path))
                .map_err(g2(say!("stage-staging-a-deletion")))?;
        }
    }
    index.write().map_err(g2(say!("stage-writing-the-index")))?;
    Ok(())
}

/// Unstage whole paths, like `git reset -- paths` (or `git rm --cached` on an unborn branch).
pub fn unstage_paths(repo: &Repository, paths: &[String]) -> Result<()> {
    let g = git2_repo(repo)?;
    for path in paths {
        check_rel(path)?;
    }
    match g.head().ok().and_then(|h| h.target()) {
        Some(head) => {
            let commit = g
                .find_object(head, None)
                .map_err(g2(say!("stage-loading-head")))?;
            g.reset_default(Some(&commit), paths.iter().map(Path::new))
                .map_err(g2(say!("stage-unstaging-paths")))?;
        }
        None => {
            let mut index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
            for path in paths {
                index
                    .remove_path(Path::new(path))
                    .map_err(g2(say!("stage-unstaging-a-path")))?;
            }
            index.write().map_err(g2(say!("stage-writing-the-index")))?;
        }
    }
    Ok(())
}

/// Which lines of a hunk to take: all, or the listed 0-based indexes into
/// `hunk.lines` (context lines are always kept).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HunkSelection {
    /// Index of the hunk in the file diff.
    pub hunk: usize,
    /// Selected change lines (indexes into `Hunk::lines`); `None` means the whole hunk.
    #[serde(default)]
    pub lines: Option<Vec<usize>>,
}

/// Apply a subset of the hunks of a diff (old -> new) onto `old`, producing
/// the text that has only the selected changes. Unselected change lines keep
/// the old side.
pub fn apply_selected(old: &str, hunks: &[Hunk], selection: &[HunkSelection]) -> Option<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0usize;
    for (hunk_ix, hunk) in hunks.iter().enumerate() {
        let selected = selection.iter().find(|s| s.hunk == hunk_ix);
        let start = if hunk.old_lines == 0 {
            usize::try_from(hunk.old_start).ok()?
        } else {
            usize::try_from(hunk.old_start).ok()?.saturating_sub(1)
        };
        if start < cursor {
            return None;
        }
        for line in old_lines.get(cursor..start)? {
            out.push_str(line);
        }
        cursor = start;
        for (line_ix, line) in hunk.lines.iter().enumerate() {
            let take = match selected {
                None => false,
                Some(sel) => sel.lines.as_ref().is_none_or(|l| l.contains(&line_ix)),
            };
            match line.kind {
                LineKind::Context => {
                    let actual = old_lines.get(cursor)?;
                    if actual.strip_suffix('\n').unwrap_or(actual) != line.text {
                        return None;
                    }
                    out.push_str(actual);
                    cursor = cursor.saturating_add(1);
                }
                LineKind::Removed => {
                    let actual = old_lines.get(cursor)?;
                    if actual.strip_suffix('\n').unwrap_or(actual) != line.text {
                        return None;
                    }
                    if !take {
                        out.push_str(actual); // not selected: the old line stays
                    }
                    cursor = cursor.saturating_add(1);
                }
                LineKind::Added => {
                    if take {
                        out.push_str(&line.text);
                        if !line.no_newline {
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    for line in old_lines.get(cursor..)? {
        out.push_str(line);
    }
    Some(out)
}

/// Undo a subset of the changes of a diff (old -> new) in `new`, producing
/// the text in which the selected changes did not happen. Unselected changes
/// stay as `new` has them. The mirror of [`apply_selected`]: unstaging and
/// discarding take their selection from the same old -> new diff a viewer
/// shows, so a line index means the line the user picked.
///
/// A removed line that is brought back goes where the hunk lists it, before
/// the added lines of its run that stay.
pub fn unapply_selected(new: &str, hunks: &[Hunk], selection: &[HunkSelection]) -> Option<String> {
    let new_lines: Vec<&str> = new.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0usize;
    for (hunk_ix, hunk) in hunks.iter().enumerate() {
        let selected = selection.iter().find(|s| s.hunk == hunk_ix);
        let start = if hunk.new_lines == 0 {
            usize::try_from(hunk.new_start).ok()?
        } else {
            usize::try_from(hunk.new_start).ok()?.saturating_sub(1)
        };
        if start < cursor {
            return None;
        }
        for line in new_lines.get(cursor..start)? {
            out.push_str(line);
        }
        cursor = start;
        for (line_ix, line) in hunk.lines.iter().enumerate() {
            let take = match selected {
                None => false,
                Some(sel) => sel.lines.as_ref().is_none_or(|l| l.contains(&line_ix)),
            };
            match line.kind {
                LineKind::Context | LineKind::Added => {
                    let actual = new_lines.get(cursor)?;
                    if actual.strip_suffix('\n').unwrap_or(actual) != line.text {
                        return None;
                    }
                    // A selected addition is undone: the line goes.
                    if line.kind == LineKind::Context || !take {
                        out.push_str(actual);
                    }
                    cursor = cursor.saturating_add(1);
                }
                LineKind::Removed => {
                    if take {
                        out.push_str(&line.text);
                        if !line.no_newline {
                            out.push('\n');
                        }
                    }
                }
            }
        }
    }
    for line in new_lines.get(cursor..)? {
        out.push_str(line);
    }
    Some(out)
}

fn read_text(bytes: Vec<u8>) -> Result<String> {
    String::from_utf8(bytes).map_err(|_| Error::Invalid {
        what: say!("stage-what-file"),
        detail: say!("stage-not-text").to_owned(),
    })
}

fn side_text(repo: &Repository, side: &DiffSide, path: &str) -> Result<String> {
    let g = git2_repo(repo)?;
    match side {
        DiffSide::Index => {
            let index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
            match index.get_path(Path::new(path), 0) {
                Some(entry) => {
                    let blob = g
                        .find_blob(entry.id)
                        .map_err(g2(say!("stage-loading-the-index-blob")))?;
                    read_text(blob.content().to_vec())
                }
                None => Ok(String::new()),
            }
        }
        DiffSide::Worktree => {
            let file = workdir(repo)?.join(path);
            // As git would store it, as the diff it is lined up with has it.
            match fs::read(&file) {
                Ok(bytes) => read_text(crate::filters::to_git(repo, path, bytes)),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(String::new()),
                Err(err) => Err(Error::Git {
                    context: say!("stage-reading-a-worktree-file"),
                    source: Box::new(err),
                }),
            }
        }
        DiffSide::Commit(id) | DiffSide::Tree(id) => {
            let tree = match side {
                DiffSide::Commit(_) => g
                    .find_commit(
                        git2::Oid::from_bytes(id.as_gix().as_bytes())
                            .map_err(g2(say!("stage-converting-an-id")))?,
                    )
                    .map_err(g2(say!("stage-loading-a-commit")))?
                    .tree()
                    .map_err(g2(say!("stage-loading-a-tree")))?,
                _ => g
                    .find_tree(
                        git2::Oid::from_bytes(id.as_gix().as_bytes())
                            .map_err(g2(say!("stage-converting-an-id")))?,
                    )
                    .map_err(g2(say!("stage-loading-a-tree")))?,
            };
            match tree.get_path(Path::new(path)) {
                Ok(entry) => {
                    let blob = g
                        .find_blob(entry.id())
                        .map_err(g2(say!("stage-loading-a-blob")))?;
                    read_text(blob.content().to_vec())
                }
                Err(_) => Ok(String::new()),
            }
        }
        DiffSide::Empty => Ok(String::new()),
    }
}

fn worktree_mode(repo: &Repository, path: &str) -> Result<u32> {
    let file = workdir(repo)?.join(path);
    let meta = file.symlink_metadata().map_err(|err| Error::Git {
        context: say!("stage-reading-file-metadata"),
        source: Box::new(err),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if meta.file_type().is_symlink() {
            return Ok(0o120000);
        }
        Ok(if meta.permissions().mode() & 0o111 != 0 {
            0o100755
        } else {
            0o100644
        })
    }
    // No executable mark here: the mode the index has stays, as git keeps
    // it with core.fileMode off (an executable script stays one, a link
    // checked out as a file stays a link); a new file is a plain one.
    #[cfg(not(unix))]
    {
        if meta.file_type().is_symlink() {
            return Ok(0o120000);
        }
        let g = git2_repo(repo)?;
        let index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
        Ok(index
            .get_path(Path::new(path), 0)
            .map(|entry| entry.mode)
            .filter(|mode| matches!(mode, 0o100644 | 0o100755 | 0o120000))
            .unwrap_or(0o100644))
    }
}

fn write_index_entry(repo: &Repository, path: &str, content: &str, mode: u32) -> Result<()> {
    let g = git2_repo(repo)?;
    let mut index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
    let existing = index.get_path(Path::new(path), 0);
    let entry = git2::IndexEntry {
        ctime: git2::IndexTime::new(0, 0),
        mtime: git2::IndexTime::new(0, 0),
        dev: 0,
        ino: 0,
        mode,
        uid: 0,
        gid: 0,
        file_size: 0,
        id: git2::Oid::ZERO_SHA1,
        flags: existing.as_ref().map(|e| e.flags & !0x0fff).unwrap_or(0),
        flags_extended: 0,
        path: path.as_bytes().to_vec(),
    };
    index
        .add_frombuffer(&entry, content.as_bytes())
        .map_err(g2(say!("stage-writing-a-staged-blob")))?;
    index.write().map_err(g2(say!("stage-writing-the-index")))?;
    Ok(())
}

/// Stage selected hunks or lines of `path` (index -> worktree diff).
pub fn stage_hunks(
    repo: &Repository,
    path: &str,
    selection: &[HunkSelection],
    context: u32,
) -> Result<()> {
    check_rel(path)?;
    let Some(diff) = diff_file(repo, &DiffSide::Index, &DiffSide::Worktree, path, context)? else {
        return Ok(());
    };
    if diff.binary {
        return Err(Error::Invalid {
            what: say!("stage-what-file"),
            detail: say!("stage-binary-stage").to_owned(),
        });
    }
    let index_text = side_text(repo, &DiffSide::Index, path)?;
    let new_text = apply_selected(&index_text, &diff.hunks, selection).ok_or(Error::Invalid {
        what: say!("stage-what-selection"),
        detail: say!("stage-hunks-stale-file").to_owned(),
    })?;
    let mode = worktree_mode(repo, path)?;
    write_index_entry(repo, path, &new_text, mode)
}

/// Unstage selected hunks or lines of `path`. The selection refers to the
/// HEAD -> index diff, the one a view of staged changes shows.
pub fn unstage_hunks(
    repo: &Repository,
    path: &str,
    selection: &[HunkSelection],
    context: u32,
) -> Result<()> {
    check_rel(path)?;
    let g = git2_repo(repo)?;
    let head = match g.head().ok().and_then(|h| h.target()) {
        Some(id) => DiffSide::Commit(Oid::from(
            gix::ObjectId::try_from(id.as_bytes()).map_err(g2(say!("stage-converting-an-id")))?,
        )),
        None => DiffSide::Empty,
    };
    let Some(diff) = diff_file(repo, &head, &DiffSide::Index, path, context)? else {
        return Ok(());
    };
    if diff.binary {
        return Err(Error::Invalid {
            what: say!("stage-what-file"),
            detail: say!("stage-binary-unstage").to_owned(),
        });
    }
    // Undoing the chosen changes in the index moves it back towards HEAD
    // for exactly those lines.
    let index_text = side_text(repo, &DiffSide::Index, path)?;
    let new_text = unapply_selected(&index_text, &diff.hunks, selection).ok_or(Error::Invalid {
        what: say!("stage-what-selection"),
        detail: say!("stage-hunks-stale-index").to_owned(),
    })?;
    let index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
    let mode = index
        .get_path(Path::new(path), 0)
        .map(|e| e.mode)
        .unwrap_or(0o100644);
    write_index_entry(repo, path, &new_text, mode)
}

/// A backup taken before a discard, enough to restore the file exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Backup {
    /// Path relative to the worktree.
    pub path: String,
    /// Blob holding the previous content, absent when the file did not exist.
    pub blob: Option<Oid>,
    /// Previous mode.
    pub mode: Option<u32>,
}

/// The mode git records for a symbolic link.
const LINK: u32 = 0o120000;
/// As many paths are handed to one `git restore`, and as many bytes of
/// them: a command line has an end, and a short one on some systems.
const RESTORED_AT_ONCE: usize = 200;
const RESTORED_BYTES: usize = 16 * 1024;

/// What stands in the way of `path` inside the worktree `wd`: the first
/// part of it, short of the last, that is there and is not a folder. A
/// symbolic link is not a folder, wherever it leads: what is behind one
/// is not in the worktree, and may be anywhere.
fn in_the_way(wd: &Path, path: &str) -> Option<String> {
    let mut at = wd.to_path_buf();
    let mut behind = String::new();
    let mut parts = path.split('/').peekable();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            break;
        }
        at.push(part);
        if !behind.is_empty() {
            behind.push('/');
        }
        behind.push_str(part);
        match at.symlink_metadata() {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Some(behind),
            // Nothing there: nothing leads anywhere.
            Err(_) => return None,
        }
    }
    None
}

/// Refuse a path that names its file in a roundabout way (`./a`, `a//b`,
/// `a/`). The index knows a file by one name; asked for by another it
/// would be taken for a file git does not know, which a discard removes.
fn check_plain(path: &str) -> Result<()> {
    if path.split('/').any(|part| part.is_empty() || part == ".") {
        return Err(Error::Invalid {
            what: say!("stage-what-path"),
            detail: say!("stage-path-not-plain", path = format!("{path:?}")),
        });
    }
    Ok(())
}

/// Refuse a path that is reached through a symbolic link, or through a
/// file: writing or removing it would be done somewhere else.
fn check_way(wd: &Path, path: &str, what: &'static str) -> Result<()> {
    match in_the_way(wd, path) {
        Some(behind) => Err(Error::Refused {
            what,
            detail: say!("stage-behind-link", path, behind),
        }),
        None => Ok(()),
    }
}

/// What a symbolic link says it leads to, as git stores it.
fn link_text(file: &Path) -> Result<Vec<u8>> {
    let target = fs::read_link(file).map_err(|err| Error::Git {
        context: say!("stage-reading-a-link-for-backup"),
        source: Box::new(err),
    })?;
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        Ok(target.as_os_str().as_bytes().to_vec())
    }
    #[cfg(not(unix))]
    {
        Ok(target.to_string_lossy().into_owned().into_bytes())
    }
}

/// Put a symbolic link that leads to `target` at `file`. Where links are
/// not to be had, a file with the target as its text, as git writes one.
fn make_link(target: &[u8], file: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        std::os::unix::fs::symlink(std::ffi::OsStr::from_bytes(target), file)
    }
    #[cfg(not(unix))]
    {
        fs::write(file, target)
    }
}

fn backup_of(repo: &Repository, path: &str) -> Result<Backup> {
    let wd = workdir(repo)?;
    let file = wd.join(path);
    let nothing = || Backup {
        path: path.to_owned(),
        blob: None,
        mode: None,
    };
    // What is behind a link is not a file of the worktree: to git, and
    // so here, there is no file at such a path.
    if in_the_way(wd, path).is_some() {
        return Ok(nothing());
    }
    let Ok(meta) = file.symlink_metadata() else {
        return Ok(nothing());
    };
    // A link is what it says, not what it leads to.
    let bytes = if meta.file_type().is_symlink() {
        link_text(&file)?
    } else {
        fs::read(&file).map_err(|err| Error::Git {
            context: say!("stage-reading-a-file-for-backup"),
            source: Box::new(err),
        })?
    };
    let g = git2_repo(repo)?;
    let id = g
        .blob(&bytes)
        .map_err(g2(say!("stage-writing-a-backup-blob")))?;
    let oid = Oid::from(
        gix::ObjectId::try_from(id.as_bytes()).map_err(g2(say!("stage-converting-an-id")))?,
    );
    Ok(Backup {
        path: path.to_owned(),
        blob: Some(oid),
        mode: Some(worktree_mode(repo, path)?),
    })
}

/// Stage everything: changes, deletions and untracked files that are not
/// ignored. This is `git add --all`, run by git so that filters and
/// `.gitattributes` apply as they do in a shell.
pub fn stage_all(repo: &Repository) -> Result<()> {
    workdir(repo)?;
    crate::exe::run(repo, ["add", "--all", "--", "."], None)?;
    Ok(())
}

/// Back up the current worktree content of `paths`, for an operation that
/// is about to overwrite them.
pub fn backup_paths(repo: &Repository, paths: &[String]) -> Result<Vec<Backup>> {
    let mut backups = Vec::with_capacity(paths.len());
    for path in paths {
        check_rel(path)?;
        backups.push(backup_of(repo, path)?);
    }
    Ok(backups)
}

/// Restore a backup to the worktree: the file as it was, a symbolic link
/// as the link it was. A link that is at the path now is replaced, never
/// written through, and nothing is restored behind a link.
pub fn restore_backup(repo: &Repository, backup: &Backup) -> Result<()> {
    check_rel(&backup.path)?;
    let wd = workdir(repo)?;
    check_way(wd, &backup.path, say!("stage-what-undo"))?;
    let file = wd.join(&backup.path);
    match backup.blob {
        None => {
            if file.symlink_metadata().is_ok() {
                fs::remove_file(&file).map_err(|err| Error::Git {
                    context: say!("stage-removing-a-file"),
                    source: Box::new(err),
                })?;
            }
        }
        Some(blob) => {
            let g = git2_repo(repo)?;
            let id = git2::Oid::from_bytes(blob.as_gix().as_bytes())
                .map_err(g2(say!("stage-converting-an-id")))?;
            let content = g
                .find_blob(id)
                .map_err(g2(say!("stage-loading-a-backup-blob")))?
                .content()
                .to_vec();
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(|err| Error::Git {
                    context: say!("stage-creating-directories"),
                    source: Box::new(err),
                })?;
            }
            let was_link = backup.mode == Some(LINK);
            // A link at the path would be written through, and a file
            // where a link goes is in its way: either is taken away
            // first.
            let there = file.symlink_metadata().ok();
            if there
                .as_ref()
                .is_some_and(|meta| meta.file_type().is_symlink() || (was_link && meta.is_file()))
            {
                fs::remove_file(&file).map_err(|err| Error::Git {
                    context: say!("stage-removing-in-place"),
                    source: Box::new(err),
                })?;
            }
            if was_link {
                return make_link(&content, &file).map_err(|err| Error::Git {
                    context: say!("stage-restoring-a-link"),
                    source: Box::new(err),
                });
            }
            fs::write(&file, content).map_err(|err| Error::Git {
                context: say!("stage-writing-a-restored-file"),
                source: Box::new(err),
            })?;
            #[cfg(unix)]
            if let Some(mode) = backup.mode {
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(
                    &file,
                    fs::Permissions::from_mode(if mode == 0o100755 { 0o755 } else { 0o644 }),
                );
            }
        }
    }
    Ok(())
}

/// Discard worktree changes of whole paths: tracked files return to what
/// the index has, untracked files are deleted. Returns backups for undo.
///
/// What comes back is what a checkout writes, and git is who writes it
/// (`git restore --worktree`): content through the filters the attributes
/// name (an LFS file as its content rather than its pointer, line endings
/// as they are checked out), a link as a link, the mode git recorded. A
/// path that is a link in the worktree is replaced, never written through.
///
/// A submodule is not a file to put back: among other paths it is left as
/// it is, as `git restore` leaves it, and asked for by itself it is refused
/// with what to do instead. A folder is refused, and so is a path behind a
/// link or a file (it is not in the worktree) and a file in conflict (it
/// has no one version to go back to); nothing is discarded then.
pub fn discard_paths(repo: &Repository, paths: &[String]) -> Result<Vec<Backup>> {
    let wd = workdir(repo)?;
    let g = git2_repo(repo)?;
    let index = g.index().map_err(g2(say!("stage-opening-the-index")))?;
    const GITLINK: u32 = 0o160000;
    let refuse = |detail: String| Error::Refused {
        what: say!("stage-what-discard"),
        detail,
    };
    // Each file with whether the index has it.
    let mut files: Vec<(&str, bool)> = Vec::new();
    let mut submodule = None;
    for path in paths {
        check_rel(path)?;
        check_plain(path)?;
        let at = Path::new(path);
        let recorded = index.get_path(at, 0);
        if recorded.as_ref().is_some_and(|entry| entry.mode == GITLINK) {
            submodule = Some(path);
            continue;
        }
        check_way(wd, path, say!("stage-what-discard"))?;
        // What is inside a submodule is that repository's to discard: here
        // it would be taken for a file git does not know, and removed.
        let inside = path
            .match_indices('/')
            .filter_map(|(end, _)| path.get(..end))
            .find(|above| {
                index
                    .get_path(Path::new(above), 0)
                    .is_some_and(|entry| entry.mode == GITLINK)
            });
        if let Some(above) = inside {
            return Err(refuse(say!("stage-inside-submodule", path, above)));
        }
        // A folder itself, not a link that leads to one.
        if wd
            .join(path)
            .symlink_metadata()
            .is_ok_and(|meta| meta.is_dir())
        {
            return Err(refuse(say!("stage-folder", path)));
        }
        if recorded.is_none() && (1..=3).any(|stage| index.get_path(at, stage).is_some()) {
            return Err(refuse(say!("stage-in-conflict", path)));
        }
        files.push((path.as_str(), recorded.is_some()));
    }
    if let (true, Some(path)) = (files.is_empty(), submodule) {
        return Err(refuse(say!("stage-submodule", path)));
    }
    let mut backups = Vec::with_capacity(files.len());
    for (path, _) in &files {
        backups.push(backup_of(repo, path)?);
    }
    let put_back = || -> Result<()> {
        // A few at a time: a command line has an end. Literal: a path is
        // a path, also when it starts like pathspec magic.
        let tracked: Vec<&str> = files
            .iter()
            .filter(|(_, tracked)| *tracked)
            .map(|(path, _)| *path)
            .collect();
        let mut rest = tracked.as_slice();
        while !rest.is_empty() {
            let mut taken = 0usize;
            let mut length = 0usize;
            for path in rest {
                length = length.saturating_add(path.len()).saturating_add(1);
                if taken > 0 && (taken >= RESTORED_AT_ONCE || length > RESTORED_BYTES) {
                    break;
                }
                taken = taken.saturating_add(1);
            }
            let (now, later) = rest.split_at(taken.min(rest.len()));
            let mut args = vec!["--literal-pathspecs", "restore", "--worktree", "--"];
            args.extend(now);
            crate::exe::run(repo, args, None)?;
            rest = later;
        }
        for (path, tracked) in &files {
            let file = wd.join(path);
            // The link itself goes, not what it leads to.
            if !tracked && file.symlink_metadata().is_ok() {
                fs::remove_file(&file).map_err(|err| Error::Git {
                    context: say!("stage-removing-an-untracked-file"),
                    source: Box::new(err),
                })?;
            }
        }
        Ok(())
    };
    if let Err(err) = put_back() {
        // Nothing is discarded then: what was put back so far is taken
        // back, as far as that goes.
        for backup in &backups {
            let _ = restore_backup(repo, backup);
        }
        return Err(err);
    }
    Ok(backups)
}

/// Discard selected hunks or lines of `path` in the worktree. The selection
/// refers to the index -> worktree diff, the one a view of unstaged changes
/// shows. Returns the backup.
pub fn discard_hunks(
    repo: &Repository,
    path: &str,
    selection: &[HunkSelection],
    context: u32,
) -> Result<Backup> {
    check_rel(path)?;
    check_plain(path)?;
    let wd = workdir(repo)?;
    check_way(wd, path, say!("stage-what-discard"))?;
    // Writing the lines back would write through the link.
    if wd
        .join(path)
        .symlink_metadata()
        .is_ok_and(|meta| meta.file_type().is_symlink())
    {
        return Err(Error::Refused {
            what: say!("stage-what-discard"),
            detail: say!("stage-symbolic-link", path),
        });
    }
    let Some(diff) = diff_file(repo, &DiffSide::Index, &DiffSide::Worktree, path, context)? else {
        return backup_of(repo, path);
    };
    if diff.binary {
        return Err(Error::Invalid {
            what: say!("stage-what-file"),
            detail: say!("stage-binary-discard").to_owned(),
        });
    }
    // Undoing the chosen changes in the worktree file moves it back to the
    // index content for exactly those lines.
    let worktree_text = side_text(repo, &DiffSide::Worktree, path)?;
    let new_text =
        unapply_selected(&worktree_text, &diff.hunks, selection).ok_or(Error::Invalid {
            what: say!("stage-what-selection"),
            detail: say!("stage-hunks-stale-file").to_owned(),
        })?;
    let backup = backup_of(repo, path)?;
    let file = wd.join(path);
    // Back as git would check it out: with CRLF again where it had them.
    let new_bytes = crate::filters::to_worktree(repo, path, new_text.into_bytes());
    fs::write(&file, new_bytes).map_err(|err| Error::Git {
        context: say!("stage-writing-a-file"),
        source: Box::new(err),
    })?;
    Ok(backup)
}

/// The absolute path of a worktree-relative `path`. Refuses absolute paths
/// and anything that climbs out of the worktree.
pub fn worktree_path(repo: &Repository, path: &str) -> Result<PathBuf> {
    check_rel(path)?;
    Ok(bitbraid_core::paths::join(workdir(repo)?, path))
}

/// The id git gives a blob with this content, without writing it anywhere.
pub fn blob_id(bytes: &[u8]) -> Option<Oid> {
    let id = git2::Oid::hash_object(git2::ObjectType::Blob, bytes).ok()?;
    Some(Oid::from(gix::ObjectId::try_from(id.as_bytes()).ok()?))
}

/// Largest text [`write_text`] takes, in bytes.
pub const MAX_EDIT: usize = 16 * 1024 * 1024;

/// Replace the content of an existing worktree file with `text`: the quick
/// fix made in the application. With `base`, only when the file still has
/// the content of that blob id, so that what somebody else wrote in the
/// meantime is not overwritten. Returns backups of the file before and
/// after, for undo and redo.
///
/// Only a regular file that is reached without passing a link is written,
/// through a temporary file beside it, and it keeps its mode.
pub fn write_text(
    repo: &Repository,
    path: &str,
    text: &str,
    base: Option<Oid>,
) -> Result<(Backup, Backup)> {
    let refuse = |detail: String| Error::Refused {
        what: say!("stage-what-edit"),
        detail,
    };
    check_rel(path)?;
    if text.len() > MAX_EDIT {
        return Err(refuse(say!("stage-too-large", path)));
    }
    let root = workdir(repo)?;
    // A link on the way could lead anywhere, out of the worktree included.
    let mut at = root.to_path_buf();
    let parts: Vec<&str> = path.split('/').collect();
    for (ix, part) in parts.iter().enumerate() {
        at.push(part);
        let last = ix.saturating_add(1) == parts.len();
        let meta = at
            .symlink_metadata()
            .map_err(|_| refuse(say!("stage-edit-missing", path)))?;
        let fits = if last { meta.is_file() } else { meta.is_dir() };
        if !fits {
            return Err(refuse(say!("stage-edit-not-plain", path)));
        }
    }
    let file = at;
    let before = backup_of(repo, path)?;
    if base.is_some() && before.blob != base {
        return Err(refuse(say!("stage-edit-changed", path)));
    }
    let io = |context: &'static str| {
        move |err: std::io::Error| Error::Git {
            context,
            source: Box::new(err),
        }
    };
    let name = file
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = file.with_file_name(format!(".{name}.bitbraid-{}.tmp", std::process::id()));
    fs::write(&tmp, text).map_err(io(say!("stage-writing-the-edited-file")))?;
    let finish = || -> std::io::Result<()> {
        fs::set_permissions(&tmp, fs::metadata(&file)?.permissions())?;
        fs::rename(&tmp, &file)
    };
    if let Err(err) = finish() {
        let _ = fs::remove_file(&tmp);
        return Err(io(say!("stage-replacing-the-edited-file"))(err));
    }
    let after = backup_of(repo, path)?;
    Ok((before, after))
}

/// Put the content of `to` in place of a file that still has the content
/// of `expected`: undo and redo of an edit, which must not overwrite what
/// was written since.
pub fn swap_backup(repo: &Repository, expected: &Backup, to: &Backup) -> Result<()> {
    check_rel(&expected.path)?;
    let now = backup_of(repo, &expected.path)?;
    if now.blob != expected.blob {
        return Err(Error::Refused {
            what: say!("stage-what-undo"),
            detail: say!("stage-changed-since", path = expected.path.as_str()),
        });
    }
    restore_backup(repo, to)
}

/// Put `path` in the worktree back to what it is in the commit `rev`,
/// leaving the index alone (`git restore --source=<rev> --worktree`).
/// Returns the backup of what was there. Fails when `rev` has no such path.
pub fn restore_path_at(repo: &Repository, path: &str, rev: Oid) -> Result<Backup> {
    check_rel(path)?;
    let backup = backup_of(repo, path)?;
    // Literal: a path is a path, also when it starts like pathspec magic.
    crate::exe::run(
        repo,
        [
            "--literal-pathspecs",
            "restore",
            "--source",
            rev.to_hex().as_str(),
            "--worktree",
            "--",
            path,
        ],
        None,
    )?;
    Ok(backup)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diff::diff_bytes;

    #[test]
    fn selecting_no_hunks_keeps_the_old_text() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let hunks = diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
        assert_eq!(apply_selected(old, &hunks, &[]).as_deref(), Some(old));
    }

    #[test]
    fn selecting_the_whole_hunk_reproduces_the_new_text() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let hunks = diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
        let all: Vec<HunkSelection> = (0..hunks.len())
            .map(|h| HunkSelection {
                hunk: h,
                lines: None,
            })
            .collect();
        assert_eq!(apply_selected(old, &hunks, &all).as_deref(), Some(new));
    }

    #[test]
    fn selecting_single_lines_takes_only_those() {
        let old = "a\nb\nc\n";
        let new = "a\nB\nc\nd\n";
        let hunks = diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
        // One hunk: a, -b, +B, c, +d. Take only "+d" (the last line of the hunk).
        let last = hunks[0].lines.len() - 1;
        let sel = vec![HunkSelection {
            hunk: 0,
            lines: Some(vec![last]),
        }];
        assert_eq!(
            apply_selected(old, &hunks, &sel).as_deref(),
            Some("a\nb\nc\nd\n")
        );
        // Take only the removal of b (keeps B out).
        let removed = hunks[0]
            .lines
            .iter()
            .position(|l| l.kind == LineKind::Removed)
            .unwrap();
        let sel = vec![HunkSelection {
            hunk: 0,
            lines: Some(vec![removed]),
        }];
        assert_eq!(apply_selected(old, &hunks, &sel).as_deref(), Some("a\nc\n"));
    }

    #[test]
    fn rejects_paths_escaping_the_worktree() {
        assert!(check_rel("../x").is_err());
        assert!(check_rel("/abs").is_err());
        assert!(check_rel("").is_err());
        assert!(check_rel("a/b.txt").is_ok());
    }
}
