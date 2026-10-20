//! Working-tree status: one entry per path with its staged (HEAD to index)
//! and unstaged (index to worktree) change, matching `git status`.
//!
//! Read through gitoxide. When gitoxide cannot read it (one path it trips
//! over ends its whole answer), git is asked instead and its porcelain
//! output is read: a status that is slower is better than none.
//!
//! Sources: git-status(1) (porcelain v2 semantics, "Porcelain Format
//! Version 2", `-z`), gix status iterator docs.

use std::collections::BTreeMap;

use gix::bstr::ByteSlice;
use serde::{Deserialize, Serialize};

use crate::{Repository, Result, git_err};

/// Kind of change on one side (staged or unstaged).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    /// New file.
    Added,
    /// Content changed.
    Modified,
    /// Removed.
    Deleted,
    /// Moved (staged renames only, as git does).
    Renamed,
    /// Copied (when copy detection is on).
    Copied,
    /// File, symlink or submodule type changed.
    TypeChanged,
    /// Not tracked (unstaged side only).
    Untracked,
    /// Marked with `git add -N`.
    IntentToAdd,
    /// Unmerged after a conflict.
    Conflicted,
}

impl ChangeKind {
    /// The letter `git status --porcelain` uses.
    pub fn letter(self) -> char {
        match self {
            ChangeKind::Added => 'A',
            ChangeKind::Modified => 'M',
            ChangeKind::Deleted => 'D',
            ChangeKind::Renamed => 'R',
            ChangeKind::Copied => 'C',
            ChangeKind::TypeChanged => 'T',
            ChangeKind::Untracked => '?',
            ChangeKind::IntentToAdd => 'A',
            ChangeKind::Conflicted => 'U',
        }
    }
}

/// One path in the status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StatusEntry {
    /// Path relative to the worktree root.
    pub path: String,
    /// Previous path for renames and copies.
    pub original_path: Option<String>,
    /// Change between HEAD and the index.
    pub staged: Option<ChangeKind>,
    /// Change between the index and the worktree.
    pub unstaged: Option<ChangeKind>,
    /// The path has conflict entries in the index.
    pub conflict: bool,
}

fn mode_class(mode: gix::index::entry::Mode) -> u8 {
    use gix::index::entry::Mode;
    if mode.contains(Mode::COMMIT) {
        2
    } else if mode.contains(Mode::SYMLINK) {
        1
    } else {
        0
    }
}

/// Whether what is at `path` in the worktree is a symbolic link, on a
/// file system that keeps them.
fn became_a_link(repo: &Repository, path: &str) -> bool {
    repo.workdir().is_some_and(|workdir| {
        workdir
            .join(path)
            .symlink_metadata()
            .is_ok_and(|meta| meta.is_symlink())
    })
}

/// How many changed paths the interface takes from one status scan. A
/// worktree with more than this (a folder of a million files that is not
/// ignored, say) is listed up to here and said to hold more: the scan
/// stops, and nothing is sorted, shown or counted beyond it.
pub const STATUS_LIMIT: usize = 20_000;

/// Compute the status of the worktree. Bare repositories have none.
pub fn status(repo: &Repository) -> Result<Vec<StatusEntry>> {
    status_bounded(repo, usize::MAX).map(|(entries, _)| entries)
}

/// [`status`] that stops once `limit` paths were found to be changed. The
/// flag says that it stopped there and that there is more; what is
/// returned is in path order.
///
/// Which paths a scan finds first is not defined, and where there are more
/// than are taken they are usually files that are not tracked. So when the
/// scan was cut short, the tracked files are scanned once more by
/// themselves, which is the cheap part, and what is changed among them is
/// kept before anything untracked: a change to commit is not lost among
/// twenty thousand files that someone forgot to ignore.
pub fn status_bounded(repo: &Repository, limit: usize) -> Result<(Vec<StatusEntry>, bool)> {
    // Ours, not gitoxide's: a linked worktree of a bare repository has
    // files to have a status of.
    if repo.is_bare() {
        return Ok((Vec::new(), false));
    }
    // Inside a WSL distribution git runs there and reads the worktree as
    // its own; gitoxide would read it through the distribution's share, a
    // file at a time and many times slower.
    if inside_wsl(repo) {
        return by_git(repo).map(|listed| cut(listed, limit));
    }
    match by_gitoxide(repo, limit) {
        Ok(found) => Ok(found),
        // Gitoxide gives up on the whole worktree for one path it cannot
        // read (an executable file replaced by a link that leads nowhere,
        // when it has to look at the content: it reads through the link).
        // Git is asked then, and what it lists is cut to the limit; when
        // git cannot say either, the first failure is the one to report.
        Err(first) => by_git(repo)
            .map(|listed| cut(listed, limit))
            .map_err(|_| first),
    }
}

/// Whether the worktree is inside a WSL distribution, which only Windows
/// reaches through a share.
fn inside_wsl(repo: &Repository) -> bool {
    #[cfg(windows)]
    {
        repo.workdir()
            .is_some_and(|dir| bitbraid_core::wsl::WslPath::of(dir).is_some())
    }
    #[cfg(not(windows))]
    {
        let _ = repo;
        false
    }
}

/// What git listed, in path order and cut to `limit` ([`keep`]), and
/// whether there was more.
fn cut(listed: Vec<StatusEntry>, limit: usize) -> (Vec<StatusEntry>, bool) {
    let mut entries: BTreeMap<String, StatusEntry> = listed
        .into_iter()
        .map(|entry| (entry.path.clone(), entry))
        .collect();
    let more = entries.len() > limit;
    keep(&mut entries, limit);
    (entries.into_values().collect(), more)
}

/// Cut a status to `limit` paths: untracked ones go first, from the end,
/// so that what is changed among the tracked files stays.
fn keep(entries: &mut BTreeMap<String, StatusEntry>, limit: usize) {
    let over = entries.len().saturating_sub(limit);
    let untracked: Vec<String> = entries
        .iter()
        .rev()
        .filter(|(_, entry)| {
            entry.staged.is_none() && entry.unstaged == Some(ChangeKind::Untracked)
        })
        .take(over)
        .map(|(path, _)| path.clone())
        .collect();
    for path in untracked {
        entries.remove(&path);
    }
    while entries.len() > limit {
        entries.pop_last();
    }
}

/// The status as git prints it for programs.
fn by_git(repo: &Repository) -> Result<Vec<StatusEntry>> {
    let out = crate::exe::run(
        repo,
        [
            "-c",
            "status.renames=true",
            "-c",
            "core.quotePath=false",
            "status",
            "--porcelain=v2",
            "-z",
            "--untracked-files=all",
        ],
        None,
    )?;
    Ok(porcelain(&out.stdout))
}

/// Read `git status --porcelain=v2 -z`. Records end with a NUL; one for a
/// rename or copy is followed by the path it came from. What cannot be
/// read as a record is passed over.
fn porcelain(text: &str) -> Vec<StatusEntry> {
    fn kind(letter: char, staged: bool) -> Option<ChangeKind> {
        Some(match letter {
            'M' => ChangeKind::Modified,
            'A' if staged => ChangeKind::Added,
            // In the worktree column: announced with `git add -N`.
            'A' => ChangeKind::IntentToAdd,
            'D' => ChangeKind::Deleted,
            'R' => ChangeKind::Renamed,
            'C' => ChangeKind::Copied,
            'T' => ChangeKind::TypeChanged,
            'U' => ChangeKind::Conflicted,
            _ => return None,
        })
    }
    let mut entries: BTreeMap<String, StatusEntry> = BTreeMap::new();
    let mut records = text.split('\0');
    while let Some(record) = records.next() {
        let (tag, rest) = record.split_once(' ').unwrap_or((record, ""));
        let mut entry = StatusEntry {
            path: String::new(),
            original_path: None,
            staged: None,
            unstaged: None,
            conflict: false,
        };
        // The fields before the path, by kind of record.
        let fields = match tag {
            "1" => 8,
            "2" => 9,
            "u" => 10,
            "?" => 1,
            _ => continue,
        };
        let parts: Vec<&str> = rest.splitn(fields, ' ').collect();
        // A rename's record is followed by where it came from, whatever
        // else is wrong with it.
        let from = if tag == "2" { records.next() } else { None };
        let Some(path) = parts
            .last()
            .filter(|path| parts.len() == fields && !path.is_empty())
        else {
            continue;
        };
        entry.path = (*path).to_owned();
        match tag {
            "?" => entry.unstaged = Some(ChangeKind::Untracked),
            "u" => {
                entry.conflict = true;
                entry.unstaged = Some(ChangeKind::Conflicted);
            }
            _ => {
                let mut letters = parts.first().copied().unwrap_or_default().chars();
                entry.staged = letters.next().and_then(|letter| kind(letter, true));
                entry.unstaged = letters.next().and_then(|letter| kind(letter, false));
                entry.original_path = from.map(str::to_owned);
            }
        }
        entries.insert(entry.path.clone(), entry);
    }
    entries.into_values().collect()
}

/// The status as gitoxide computes it, cut at `limit` paths: see
/// [`status_bounded`].
fn by_gitoxide(repo: &Repository, limit: usize) -> Result<(Vec<StatusEntry>, bool)> {
    let (mut entries, more) = scan(repo, true, limit)?;
    if more {
        let (tracked, _) = scan(repo, false, limit)?;
        entries.extend(tracked);
        keep(&mut entries, limit);
    }
    Ok((entries.into_values().collect(), more))
}

/// One pass over the worktree, with or without the files that are not
/// tracked, until more than `limit` paths were found to be changed: the
/// paths by name, at most `limit` of them, and whether there were more.
fn scan(
    repo: &Repository,
    untracked: bool,
    limit: usize,
) -> Result<(BTreeMap<String, StatusEntry>, bool)> {
    let r = repo.gix();
    let platform = r
        .status(gix::progress::Discard)
        .map_err(git_err(say!("status-preparing")))?
        .untracked_files(if untracked {
            gix::status::UntrackedFiles::Files
        } else {
            gix::status::UntrackedFiles::None
        })
        .index_worktree_rewrites(None);
    let iter = platform
        .into_iter(Vec::<gix::bstr::BString>::new())
        .map_err(git_err(say!("status-starting")))?;

    let mut entries: BTreeMap<String, StatusEntry> = BTreeMap::new();
    fn entry(entries: &mut BTreeMap<String, StatusEntry>, path: String) -> &mut StatusEntry {
        entries.entry(path.clone()).or_insert_with(|| StatusEntry {
            path,
            original_path: None,
            staged: None,
            unstaged: None,
            conflict: false,
        })
    }

    let mut more = false;
    for item in iter {
        // One path more than is taken: there is more, and leaving the loop
        // ends the scan.
        if entries.len() > limit {
            more = true;
            break;
        }
        let item = item.map_err(git_err(say!("status-reading")))?;
        match item {
            gix::status::Item::TreeIndex(change) => {
                use gix::diff::index::Change;
                let (kind, path, original) = match &change {
                    Change::Addition { location, .. } => (
                        ChangeKind::Added,
                        location.to_str_lossy().into_owned(),
                        None,
                    ),
                    Change::Deletion { location, .. } => (
                        ChangeKind::Deleted,
                        location.to_str_lossy().into_owned(),
                        None,
                    ),
                    Change::Modification {
                        location,
                        previous_entry_mode,
                        entry_mode,
                        ..
                    } => {
                        let kind = if mode_class(*previous_entry_mode) != mode_class(*entry_mode) {
                            ChangeKind::TypeChanged
                        } else {
                            ChangeKind::Modified
                        };
                        (kind, location.to_str_lossy().into_owned(), None)
                    }
                    Change::Rewrite {
                        source_location,
                        location,
                        copy,
                        ..
                    } => (
                        if *copy {
                            ChangeKind::Copied
                        } else {
                            ChangeKind::Renamed
                        },
                        location.to_str_lossy().into_owned(),
                        Some(source_location.to_str_lossy().into_owned()),
                    ),
                };
                let e = entry(&mut entries, path);
                e.staged = Some(kind);
                if original.is_some() {
                    e.original_path = original;
                }
            }
            gix::status::Item::IndexWorktree(item) => {
                use gix::status::index_worktree::Item as W;
                use gix::status::plumbing::index_as_worktree::{Change, EntryStatus};
                match item {
                    W::Modification {
                        rela_path,
                        status,
                        entry: tracked,
                        ..
                    } => {
                        let path = rela_path.to_str_lossy().into_owned();
                        match status {
                            EntryStatus::Conflict { .. } => {
                                let e = entry(&mut entries, path);
                                e.conflict = true;
                                e.unstaged = Some(ChangeKind::Conflicted);
                            }
                            EntryStatus::Change(change) => {
                                let kind = match change {
                                    Change::Removed => ChangeKind::Deleted,
                                    Change::Type { .. } => ChangeKind::TypeChanged,
                                    // An executable file that became a
                                    // symbolic link is read by gitoxide
                                    // as a file that lost its executable
                                    // bit; git calls it what it is.
                                    Change::Modification { .. }
                                        if mode_class(tracked.mode) == 0
                                            && became_a_link(repo, &path) =>
                                    {
                                        ChangeKind::TypeChanged
                                    }
                                    Change::Modification { .. }
                                    | Change::SubmoduleModification(_) => ChangeKind::Modified,
                                };
                                entry(&mut entries, path).unstaged = Some(kind);
                            }
                            EntryStatus::IntentToAdd => {
                                entry(&mut entries, path).unstaged = Some(ChangeKind::IntentToAdd);
                            }
                            EntryStatus::NeedsUpdate(_) => {}
                        }
                    }
                    W::DirectoryContents {
                        entry: dir_entry, ..
                    } => {
                        if dir_entry.status == gix::dir::entry::Status::Untracked {
                            let path = dir_entry.rela_path.to_str_lossy().into_owned();
                            entry(&mut entries, path).unstaged = Some(ChangeKind::Untracked);
                        }
                    }
                    other => {
                        // Rewrites are disabled above; keep the path visible if one ever appears.
                        let path = other.rela_path().to_str_lossy().into_owned();
                        entry(&mut entries, path).unstaged = Some(ChangeKind::Renamed);
                    }
                }
            }
        }
    }
    while entries.len() > limit {
        more = true;
        entries.pop_last();
    }
    Ok((entries, more))
}

#[cfg(test)]
mod tests {
    use super::{ChangeKind, StatusEntry, porcelain};

    fn entry(path: &str, staged: Option<ChangeKind>, unstaged: Option<ChangeKind>) -> StatusEntry {
        StatusEntry {
            path: path.to_owned(),
            original_path: None,
            staged,
            unstaged,
            conflict: false,
        }
    }

    #[test]
    fn gits_porcelain_is_read_record_by_record() {
        let id = "1111111111111111111111111111111111111111";
        let text = [
            format!("1 .M N... 100644 100644 100644 {id} {id} changed.txt"),
            format!("1 A. N... 000000 100644 100644 {id} {id} dir/added and spaced.txt"),
            format!("1 .A N... 000000 000000 100644 {id} {id} announced.txt"),
            format!("1 MD N... 100644 100644 000000 {id} {id} staged then gone.txt"),
            format!("1 .T N... 100755 100755 120000 {id} {id} run.sh"),
            format!("2 R. N... 100644 100644 100644 {id} {id} R100 new name.txt"),
            "old name.txt".to_owned(),
            format!("u UU N... 100644 100644 100644 100644 {id} {id} {id} both.txt"),
            "? untracked/deep down.txt".to_owned(),
            "! ignored.o".to_owned(),
            "# branch.oid something".to_owned(),
            String::new(),
        ]
        .join("\0");
        use ChangeKind::*;
        let mut renamed = entry("new name.txt", Some(Renamed), None);
        renamed.original_path = Some("old name.txt".to_owned());
        let mut conflicted = entry("both.txt", None, Some(Conflicted));
        conflicted.conflict = true;
        assert_eq!(
            porcelain(&text),
            [
                entry("announced.txt", None, Some(IntentToAdd)),
                conflicted,
                entry("changed.txt", None, Some(Modified)),
                entry("dir/added and spaced.txt", Some(Added), None),
                renamed,
                entry("run.sh", None, Some(TypeChanged)),
                entry("staged then gone.txt", Some(Modified), Some(Deleted)),
                entry("untracked/deep down.txt", None, Some(Untracked)),
            ]
        );
    }

    #[test]
    fn what_is_no_record_is_passed_over() {
        for hostile in [
            "",
            "\0\0\0",
            "1",
            "1 ",
            "1 .M too few fields",
            "2 R. N... 100644 100644 100644 a b R100",
            "u UU short",
            "? ",
            "?",
            "x y z",
            "1 \u{fffd}\u{fffd} N... 1 2 3 a b ",
        ] {
            assert_eq!(porcelain(hostile), [], "{hostile:?}");
        }
        // Letters nobody knows are no change; the path is still a path.
        let odd = porcelain("1 ?! N... 1 2 3 a b odd.txt\0");
        assert_eq!(odd, [entry("odd.txt", None, None)]);
        // A rename cut off before its source keeps the rename.
        let cut = porcelain("2 R. N... 1 2 3 a b R100 to.txt");
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].original_path, None);
    }
}
