//! The changes panel: unstaged and staged files as a list of paths or a tree
//! of folders, with selection, keyboard staging, and stage, unstage, discard
//! and ignore actions.
//!
//! Sources: git-status(1) for what the two lists mean; gpui-kit
//! documentation (virtual list, scroll mask).

use std::{collections::BTreeSet, rc::Rc};

use bitbraid_git::{
    ChangeKind, StatusEntry,
    sequence::{Conflict, ConflictKind, InProgress},
};
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, VirtualListScrollHandle,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt, PopupMenu},
    scroll::{ScrollableElement, ScrollableMask, ScrollbarAxis},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde::Deserialize;
use serde_json::json;

use crate::{
    keymap::{
        CHANGES_CONTEXT, DiscardAll, DiscardSelected, MarkResolved, OpenSelected, RowMenu,
        SelectAll, SelectFirst, SelectLast, SelectNext, SelectPrev, StageAll, StageSelected,
        TakeOurs, TakeTheirs, ToggleTreeView, UnstageAll, UnstageSelected,
    },
    prefs::Prefs,
    session::{Reply, Request, SessionHandle},
    views::{
        chosen,
        drag_paths::{self, DraggedPaths},
        file_actions::{self, FileAction, RunFileAction},
        file_tree,
    },
};

/// Height of the row that opens or closes every folder of a tree.
const ROW_HEIGHT: f32 = 30.0;
/// Height of an open folder of the tree (a closed one is as high as a
/// file).
const OPEN_FOLDER_HEIGHT: f32 = 25.0;
/// How much deeper each level of the tree is.
const INDENT: f32 = 15.0;
/// Height of a list's heading, its rule included.
const HEADING_HEIGHT: f32 = 36.0;
/// Height of a file of the two lists.
const FILE_ROW_HEIGHT: f32 = 32.0;

/// Which list a path is shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Deserialize)]
pub enum Side {
    /// Index to worktree.
    Unstaged,
    /// HEAD to index.
    Staged,
}

/// Events from the changes panel.
#[derive(Debug, Clone)]
pub enum ChangesEvent {
    /// A file was chosen; show its diff for this side.
    Open { path: String, side: Side },
    /// A file was clicked: shown, or closed again when it is the one that
    /// is shown.
    Clicked { path: String, side: Side },
    /// Several files are selected: none of them is the one shown.
    Several,
    /// The repository changed through this panel; refresh status.
    Mutated,
    /// A conflicted file was chosen; open it to be resolved.
    OpenConflict { path: String },
    /// Something is to be ignored, or to stop being tracked.
    Ignore(IgnoreRequest),
    /// Something was picked from a file's menu that the workspace does:
    /// the editor, the file manager, the file's history or its blame.
    File(FileAction),
    /// The unstaged changes to these files are to be thrown away, once
    /// that has been asked.
    Discard(Vec<String>),
}

/// What a file's menu asks about ignoring. The workspace carries it out,
/// since it asks for a pattern and says what came of it.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub enum IgnoreRequest {
    /// A pattern made from a path goes into `.gitignore` at once. `what`
    /// is `file`, `folder` or `extension`, as `ignore.add` takes it.
    Path {
        /// The path, relative to the worktree.
        path: String,
        /// What of it to ignore.
        what: String,
    },
    /// A pattern is typed, starting from `start`, and goes to `target`
    /// (`repository`, `local` or `global`).
    Typed {
        /// What the field starts with.
        start: String,
        /// Which ignore file.
        target: String,
    },
    /// These paths stop being tracked and stay on disk.
    Untrack {
        /// Files or folders, relative to the worktree.
        paths: Vec<String>,
    },
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct AskIgnore {
    request: IgnoreRequest,
}

/// The folders above a file, nearest first: `a/b/c.txt` gives `a/b`, `a`.
pub fn folders_above(path: &str) -> Vec<&str> {
    let mut folders = Vec::new();
    let mut rest = path;
    while let Some((folder, _)) = rest.rsplit_once('/') {
        if !folder.is_empty() {
            folders.push(folder);
        }
        rest = folder;
    }
    folders
}

/// The entries about ignoring a path: itself, its extension, the folders
/// above it (the nearest three), and a pattern typed by hand for each of
/// the three ignore files; and, for a file git tracks, an end to that.
fn ignore_entries(menu: PopupMenu, path: &str, folder: bool, tracked: bool) -> PopupMenu {
    use bitbraid_git::{IgnoreWhat, ignore::pattern_for};
    let ask = |request: IgnoreRequest| -> Box<dyn Action> { Box::new(AskIgnore { request }) };
    let by_path = |path: &str, what: &str| {
        ask(IgnoreRequest::Path {
            path: path.to_owned(),
            what: what.to_owned(),
        })
    };
    let mut menu = menu;
    let own = if folder {
        IgnoreWhat::Folder
    } else {
        IgnoreWhat::File
    };
    // What a pattern cannot name (a line break in the path) is not offered.
    let own_pattern = pattern_for(path, own);
    if own_pattern.is_some() {
        menu = menu.menu(
            if folder {
                say!("changes-ignore-this-folder")
            } else {
                say!("changes-ignore-this-file")
            },
            by_path(path, if folder { "folder" } else { "file" }),
        );
    }
    let extension = (!folder)
        .then(|| pattern_for(path, IgnoreWhat::Extension))
        .flatten();
    if let Some(extension) = &extension {
        menu = menu.menu(
            say!("changes-ignore-extension", extension),
            by_path(path, "extension"),
        );
    }
    for above in folders_above(path).into_iter().take(3) {
        if pattern_for(above, IgnoreWhat::Folder).is_some() {
            menu = menu.menu(
                say!("changes-ignore-folder-above", above),
                by_path(above, "folder"),
            );
        }
    }
    let typed = |start: &Option<String>, target: &str| {
        ask(IgnoreRequest::Typed {
            start: start.clone().unwrap_or_default(),
            target: target.to_owned(),
        })
    };
    menu = menu
        .separator()
        .menu(
            say!("changes-ignore-pattern"),
            typed(&own_pattern, "repository"),
        )
        .menu(
            say!("changes-ignore-pattern-local"),
            typed(&own_pattern, "local"),
        )
        // A path of this repository means little to the others: the
        // extension is what is offered to start from.
        .menu(
            say!("changes-ignore-pattern-global"),
            typed(&extension, "global"),
        );
    if tracked {
        menu = menu.separator().menu(
            if folder {
                say!("changes-untrack-folder")
            } else {
                say!("changes-untrack-file")
            },
            ask(IgnoreRequest::Untrack {
                paths: vec![path.to_owned()],
            }),
        );
    }
    menu
}

/// The menu of a file in conflict: the same from the pointer and from the
/// menu key. First what resolves it (the merge tool, either side whole,
/// marking it resolved), then its past, the ways to open it and its path.
fn conflict_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    path: &str,
    ours: &str,
    theirs: &str,
    reused: bool,
) -> PopupMenu {
    let resolve = |how| -> Box<dyn Action> {
        Box::new(ResolveConflict {
            path: path.to_owned(),
            how,
        })
    };
    let menu = menu
        .menu(
            say!("changes-open-merge-tool"),
            Box::new(RunFileAction {
                action: FileAction::MergeTool {
                    path: path.to_owned(),
                },
            }),
        )
        .menu(say!("changes-take-ours", ours), resolve(Resolution::Ours))
        .menu(
            say!("changes-take-theirs", theirs),
            resolve(Resolution::Theirs),
        )
        .menu(say!("changes-mark-resolved"), resolve(Resolution::AsItIs))
        .menu(
            say!("changes-mark-resolved-markers"),
            resolve(Resolution::WithMarkers),
        );
    // What rerere put into the file is one it recorded before: it can be
    // forgotten, for the conflict to be resolved afresh and that to be
    // recorded.
    let menu = if reused {
        menu.menu(
            say!("changes-forget-resolution"),
            Box::new(ForgetResolution {
                path: path.to_owned(),
            }),
        )
    } else {
        menu
    };
    let ignored = path.to_owned();
    let menu = menu.submenu(say!("changes-ignore"), window, cx, move |menu, _, _| {
        ignore_entries(menu, &ignored, false, true)
    });
    file_actions::entries(menu.separator(), path, file_actions::Listed::Worktree)
}

/// The menu of a changed file in one of the two lists: the same from the
/// pointer and from the menu key.
fn file_menu(
    menu: PopupMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
    path: &str,
    side: Side,
    untracked: bool,
    was_in_conflict: bool,
) -> PopupMenu {
    let p = path.to_owned();
    // A file that was resolved in the operation under way can be looked
    // at again as the conflict it was.
    let menu = if was_in_conflict {
        menu.menu(
            say!("changes-reopen-conflict"),
            Box::new(ReopenConflict { path: p.clone() }),
        )
        .separator()
    } else {
        menu
    };
    // What is done to the file: staging or unstaging it, throwing its
    // changes away (a file that is new has none to throw away: deleting
    // it is at the end), ignoring it.
    let menu = match side {
        Side::Unstaged => menu.menu(
            say!("changes-stage"),
            Box::new(StagePath { path: p.clone() }),
        ),
        Side::Staged => menu.menu(
            say!("changes-unstage"),
            Box::new(UnstagePath { path: p.clone() }),
        ),
    };
    let menu = match side {
        Side::Unstaged if untracked => menu,
        Side::Unstaged => menu.menu(
            say!("changes-discard"),
            Box::new(DiscardPath { path: p.clone() }),
        ),
        // A staged file goes back to what the last commit has.
        Side::Staged => menu.menu(
            say!("changes-discard"),
            Box::new(DiscardStaged { path: p.clone() }),
        ),
    };
    let ignored_path = p.clone();
    let menu = menu.submenu(say!("changes-ignore"), window, cx, move |menu, _, _| {
        ignore_entries(menu, &ignored_path, false, !untracked)
    });
    // History and blame, the ways to open it, the path: the same entries
    // the files of a commit have.
    let listed = match side {
        Side::Unstaged if untracked => file_actions::Listed::Untracked,
        Side::Unstaged => file_actions::Listed::Worktree,
        Side::Staged => file_actions::Listed::Staged,
    };
    let menu = file_actions::entries(menu.separator(), &p, listed);
    if untracked {
        return menu.separator().menu(
            say!("changes-delete-file"),
            Box::new(DiscardPath { path: p.clone() }),
        );
    }
    // A tracked file can be put out of git's sight; it then no longer
    // shows here, and is found among the index flags.
    if side == Side::Staged {
        return menu;
    }
    let hide = |flag: &str| -> Box<dyn Action> {
        Box::new(HidePath {
            path: p.clone(),
            flag: flag.to_owned(),
        })
    };
    menu.separator()
        .menu(say!("changes-skip-worktree"), hide("skip_worktree"))
        .menu(say!("changes-assume-unchanged"), hide("assume_unchanged"))
}

/// The menu of several selected files of one list: what is done to all
/// of them.
fn selection_menu(menu: PopupMenu, side: Side, paths: Vec<String>) -> PopupMenu {
    let n = paths.len();
    let menu = match side {
        Side::Unstaged => menu.menu(
            say!("changes-selection-stage", count = n),
            Box::new(StageMany {
                paths: paths.clone(),
            }),
        ),
        Side::Staged => menu.menu(
            say!("changes-selection-unstage", count = n),
            Box::new(UnstageMany {
                paths: paths.clone(),
            }),
        ),
    };
    menu.menu(
        say!("changes-selection-discard", count = n),
        Box::new(DiscardMany {
            side,
            paths: paths.clone(),
        }),
    )
    .menu(
        say!("changes-selection-ignore", count = n),
        Box::new(IgnoreMany { paths }),
    )
}

/// Where the keys are among the rows: on a file in conflict, by its place
/// in that list, or on a file of the two lists under it, by its row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    Conflict(usize),
    File(usize),
}

/// The spot `delta` steps from `current` among `conflicts` files in
/// conflict followed by the file rows `files`: no further than the ends.
/// From nowhere, one step up is the last and one step down the first; all
/// the way up is still the first and all the way down the last.
fn spot_after(
    current: Option<Spot>,
    delta: isize,
    conflicts: usize,
    files: &[usize],
) -> Option<Spot> {
    let count = conflicts.saturating_add(files.len());
    let last = count.checked_sub(1)?;
    let at = current.and_then(|spot| match spot {
        Spot::Conflict(ix) => (ix < conflicts).then_some(ix),
        Spot::File(row) => files
            .iter()
            .position(|file| *file == row)
            .map(|ix| ix.saturating_add(conflicts)),
    });
    let next = match at {
        Some(at) => at.saturating_add_signed(delta).min(last),
        None => match delta {
            ..=-2 => 0,
            -1 => last,
            0 | 1 => 0,
            _ => last,
        },
    };
    match next.checked_sub(conflicts) {
        None => Some(Spot::Conflict(next)),
        Some(ix) => files.get(ix).copied().map(Spot::File),
    }
}

/// A menu that was opened by key, where it stands, and the watch for its
/// end.
struct KeyMenu {
    menu: Entity<PopupMenu>,
    position: Point<Pixels>,
    _closed: Subscription,
}

/// How a conflicted file is resolved as a whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
enum Resolution {
    /// Our side's version.
    Ours,
    /// Their side's version.
    Theirs,
    /// What the worktree holds.
    AsItIs,
    /// What the worktree holds, conflict markers included.
    WithMarkers,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct ResolveConflict {
    path: String,
    how: Resolution,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct StagePath {
    path: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct UnstagePath {
    path: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct DiscardPath {
    path: String,
}

/// Stage these files.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct StageMany {
    paths: Vec<String>,
}

/// Unstage these files.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct UnstageMany {
    paths: Vec<String>,
}

/// Throw away the changes of these files of one list, once asked.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct DiscardMany {
    side: Side,
    paths: Vec<String>,
}

/// Ignore each of these files.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct IgnoreMany {
    paths: Vec<String>,
}

/// Put a staged file back to what the last commit has: out of the index
/// first, then its changes thrown away.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct DiscardStaged {
    path: String,
}

/// Stage what is changed in a folder.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct StageFolder {
    folder: String,
}

/// Unstage what is staged in a folder.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct UnstageFolder {
    folder: String,
}

/// Throw away what is not staged in a folder, once asked.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct DiscardFolder {
    folder: String,
}

/// The icon and the colour of a kind of change, for a folder's counts.
pub(crate) fn tree_mark(kind: file_tree::Kind, theme: &gpui_kit::component::Theme) -> (Icon, Hsla) {
    match kind {
        file_tree::Kind::Added => (Icon::new(IconName::Plus), theme.green),
        file_tree::Kind::Deleted => (Icon::new(IconName::Minus), theme.red),
        file_tree::Kind::Renamed => (Icon::new(gpui_kit::assets::IconName::FilePen), theme.blue),
        file_tree::Kind::Modified => (Icon::new(gpui_kit::assets::IconName::Pencil), theme.yellow),
    }
}

/// Put a file that was resolved back in conflict.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct ReopenConflict {
    path: String,
}

/// Forget the resolution rerere recorded for the conflict a file is in.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct ForgetResolution {
    path: String,
}

/// The paths `conflict.resolved` answers with.
pub fn resolved_paths(value: &serde_json::Value) -> BTreeSet<String> {
    value
        .get("paths")
        .and_then(|paths| paths.as_array())
        .map(|paths| {
            paths
                .iter()
                .filter_map(|path| path.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Tell git not to look at a file any more: `assume_unchanged` or
/// `skip_worktree`.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct HidePath {
    path: String,
    flag: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct OpenPath {
    path: String,
}

#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = changes, no_json)]
struct CopyPath {
    path: String,
}

/// A file in one of the two lists.
type Key = (Side, String);

/// One row of the panel.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Row {
    /// The header of a list with its file count.
    Header { side: Side, count: usize },
    /// The first row of a list shown as a tree: it opens every folder, or
    /// closes them all once every one is open.
    Expand { side: Side, all_open: bool },
    /// A folder of the tree; `path` is its full path, `label` its name;
    /// closed, it says how many files of each kind are in it.
    Folder {
        side: Side,
        path: String,
        label: String,
        depth: usize,
        open: bool,
        counts: file_tree::Counts,
    },
    /// A changed file; `entry` indexes the status entries.
    File {
        side: Side,
        entry: usize,
        depth: usize,
        /// File name in the tree, file name with the folder apart in the list.
        label: String,
        /// The folder, shown muted beside the name in the list of paths.
        folder: Option<String>,
    },
}

fn change_of(entry: &StatusEntry, side: Side) -> Option<ChangeKind> {
    match side {
        Side::Unstaged => entry.unstaged,
        Side::Staged => entry.staged,
    }
}

/// Split a path into its folder and file name.
fn split_path(path: &str) -> (Option<&str>, &str) {
    match path.rsplit_once('/') {
        Some((folder, name)) => (Some(folder), name),
        None => (None, path),
    }
}

/// What happened to a file on one side, as a folder counts it.
fn tree_kind(kind: ChangeKind) -> file_tree::Kind {
    match kind {
        ChangeKind::Added | ChangeKind::Untracked | ChangeKind::IntentToAdd => {
            file_tree::Kind::Added
        }
        ChangeKind::Deleted => file_tree::Kind::Deleted,
        ChangeKind::Renamed | ChangeKind::Copied => file_tree::Kind::Renamed,
        ChangeKind::Modified | ChangeKind::TypeChanged | ChangeKind::Conflicted => {
            file_tree::Kind::Modified
        }
    }
}

/// The folders of one side's files, as the open set keeps them.
fn side_folders(entries: &[StatusEntry], side: Side) -> BTreeSet<Key> {
    file_tree::folders(
        entries
            .iter()
            .filter(|entry| !entry.conflict && change_of(entry, side).is_some())
            .map(|entry| entry.path.as_str()),
    )
    .into_iter()
    .map(|folder| (side, folder))
    .collect()
}

/// The rows of the panel: for each side a header, then its files as a list
/// of paths or as a tree, by path A to Z (or Z to A), without regard to
/// case. In the tree, the folders in `open` are open.
fn build_rows(
    entries: &[StatusEntry],
    tree: bool,
    open: &BTreeSet<Key>,
    reversed: bool,
) -> Vec<Row> {
    let mut rows = Vec::new();
    for side in [Side::Unstaged, Side::Staged] {
        let files: Vec<usize> = entries
            .iter()
            .enumerate()
            // Conflicted files have a list of their own.
            .filter(|(_, entry)| !entry.conflict && change_of(entry, side).is_some())
            .map(|(ix, _)| ix)
            .collect();
        let paths: Vec<&str> = files
            .iter()
            .filter_map(|ix| entries.get(*ix).map(|entry| entry.path.as_str()))
            .collect();
        let files: Vec<usize> = file_tree::order(&paths, reversed)
            .into_iter()
            .filter_map(|place| files.get(place).copied())
            .collect();
        rows.push(Row::Header {
            side,
            count: files.len(),
        });
        if files.is_empty() {
            // An empty list is its heading alone.
        } else if tree {
            let items: Vec<(usize, &str, file_tree::Kind)> = files
                .iter()
                .filter_map(|ix| {
                    let entry = entries.get(*ix)?;
                    Some((*ix, entry.path.as_str(), tree_kind(change_of(entry, side)?)))
                })
                .collect();
            let folders = side_folders(entries, side);
            if !folders.is_empty() {
                rows.push(Row::Expand {
                    side,
                    all_open: folders.is_subset(open),
                });
            }
            let is_open = |path: &str| open.contains(&(side, path.to_owned()));
            for node in file_tree::tree(&items, &is_open) {
                rows.push(match node {
                    file_tree::Node::Folder {
                        path,
                        name,
                        depth,
                        open,
                        counts,
                    } => Row::Folder {
                        side,
                        path,
                        label: name,
                        depth,
                        open,
                        counts,
                    },
                    file_tree::Node::File { item, depth } => {
                        let name = entries
                            .get(item)
                            .map(|entry| split_path(&entry.path).1.to_owned())
                            .unwrap_or_default();
                        Row::File {
                            side,
                            entry: item,
                            depth,
                            label: name,
                            folder: None,
                        }
                    }
                });
            }
        } else {
            for ix in files {
                let Some(entry) = entries.get(ix) else {
                    continue;
                };
                // A moved file is where it is now; its mark says it moved.
                let (folder, name) = split_path(&entry.path);
                rows.push(Row::File {
                    side,
                    entry: ix,
                    depth: 0,
                    label: name.to_owned(),
                    folder: folder.map(str::to_owned),
                });
            }
        }
    }
    rows
}

/// How high a row is.
fn row_height(row: &Row) -> f32 {
    match row {
        Row::File { .. } => FILE_ROW_HEIGHT,
        Row::Folder { open: true, .. } => OPEN_FOLDER_HEIGHT,
        Row::Folder { open: false, .. } => FILE_ROW_HEIGHT,
        Row::Header { .. } | Row::Expand { .. } => ROW_HEIGHT,
    }
}

/// The rows without what is in the lists that are folded: their headings
/// stay.
fn fold_rows(rows: Vec<Row>, folded: &BTreeSet<Side>) -> Vec<Row> {
    rows.into_iter()
        .filter(|row| match row {
            Row::Header { .. } => true,
            Row::Expand { side, .. } | Row::Folder { side, .. } | Row::File { side, .. } => {
                !folded.contains(side)
            }
        })
        .collect()
}

/// One of the two lists: its side, how many files it has, and which rows
/// are under its heading (the heading's own row is not among them).
#[derive(Debug, Clone, PartialEq)]
struct Section {
    side: Side,
    count: usize,
    rows: std::ops::Range<usize>,
    /// How high each of those rows is.
    sizes: crate::zoom::Sizes,
}

/// The lists of the rows: each heading with the rows up to the next one.
fn sections(rows: &[Row]) -> Vec<Section> {
    let mut out: Vec<Section> = Vec::new();
    for (ix, row) in rows.iter().enumerate() {
        if let Row::Header { side, count } = row {
            if let Some(last) = out.last_mut() {
                last.rows.end = ix;
            }
            out.push(Section {
                side: *side,
                count: *count,
                rows: ix.saturating_add(1)..rows.len(),
                sizes: crate::zoom::Sizes::default(),
            });
        }
    }
    for section in &mut out {
        section.sizes = crate::zoom::Sizes::new(
            rows.get(section.rows.clone())
                .unwrap_or_default()
                .iter()
                .map(|row| size(px(0.), px(row_height(row))))
                .collect(),
        );
    }
    out
}

/// The paths with, for each file that was moved on `side`, the place it was
/// moved from: a move is one row, and git keeps the two paths apart, so
/// that unstaging the new one alone would leave the old one's deletion
/// staged.
fn with_rename_sources(entries: &[StatusEntry], side: Side, paths: Vec<String>) -> Vec<String> {
    let mut out = paths.clone();
    for path in &paths {
        let from = entries
            .iter()
            .find(|entry| {
                entry.path == *path && change_of(entry, side) == Some(ChangeKind::Renamed)
            })
            .and_then(|entry| entry.original_path.clone());
        if let Some(from) = from
            && !out.contains(&from)
        {
            out.push(from);
        }
    }
    out
}

/// The file a row stands for.
fn row_key(row: &Row, entries: &[StatusEntry]) -> Option<Key> {
    match row {
        Row::File { side, entry, .. } => Some((*side, entries.get(*entry)?.path.clone())),
        _ => None,
    }
}

/// What a click on a row did.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Clicked {
    /// The row is not a file: nothing.
    Nothing,
    /// The selection changed, and no diff is opened for it.
    Selected,
    /// This file alone is selected now, and its diff is to be opened.
    Opened(Key),
}

/// Apply a click on `row` to the selection and the cursor.
///
/// A plain click selects the file alone, puts the cursor on it and opens
/// its diff. With the secondary modifier (Ctrl, Cmd on macOS) the file
/// joins or leaves the selection and takes the cursor. With Shift, every
/// file between the cursor and the click joins the selection, across the
/// two lists when they lie on either side of a header, and the cursor
/// stays where it is, so that the next range starts from the same end.
/// Shift without a cursor that still has a row (none yet, or its file
/// left the list or is in a folder that was closed) is a click without
/// Shift.
fn click_on(
    rows: &[Row],
    entries: &[StatusEntry],
    selection: &mut BTreeSet<Key>,
    cursor: &mut Option<Key>,
    row: usize,
    shift: bool,
    secondary: bool,
) -> Clicked {
    let key_at = |ix: usize| rows.get(ix).and_then(|row| row_key(row, entries));
    let Some(key) = key_at(row) else {
        return Clicked::Nothing;
    };
    if shift
        && let Some(anchor) = cursor.as_ref().and_then(|cursor| {
            rows.iter()
                .position(|row| row_key(row, entries).as_ref() == Some(cursor))
        })
    {
        // Everything between the cursor and the click joins the selection.
        let (from, to) = (anchor.min(row), anchor.max(row));
        for ix in from..=to {
            if let Some(key) = key_at(ix) {
                selection.insert(key);
            }
        }
        return Clicked::Selected;
    }
    if secondary {
        if !selection.remove(&key) {
            selection.insert(key.clone());
        }
        *cursor = Some(key);
        return Clicked::Selected;
    }
    *selection = BTreeSet::from([key.clone()]);
    *cursor = Some(key.clone());
    Clicked::Opened(key)
}

pub struct ChangesView {
    session: SessionHandle,
    entries: Vec<StatusEntry>,
    /// The branch the changes are on, for the heading.
    branch: Option<String>,
    /// Files in conflict in the operation in progress.
    conflicts: Vec<Conflict>,
    /// Files that were in conflict in the operation under way and are
    /// resolved now.
    resolved: BTreeSet<String>,
    /// What the operation's two sides are in words.
    conflict_sides: (String, String),
    /// What comes in and what it comes into, by name, and the operation's
    /// kind, for the line under the heading.
    side_names: Option<(String, String)>,
    stopped_kind: Option<bitbraid_git::sequence::Kind>,
    /// The operation that stopped part way: what it says about itself, and
    /// whether it cannot go on as things are. It heads the panel then.
    stopped: Option<(String, bool)>,
    /// Rows to show, rebuilt when the status, the layout or a folder's
    /// collapsed state changes.
    rows: Rc<Vec<Row>>,
    /// The two lists of those rows, each under its heading.
    sections: Rc<Vec<Section>>,
    /// The folders of the trees that are open; the others are closed.
    open_folders: BTreeSet<Key>,
    /// The lists that are folded to their headings, and whether the one of
    /// the files in conflict is.
    folded: BTreeSet<Side>,
    conflicts_folded: bool,
    /// Files the next action applies to.
    selection: BTreeSet<Key>,
    /// The file last clicked or moved to: the one whose diff is open, and
    /// one end of a shift-click range.
    cursor: Option<Key>,
    /// Row of the cursor before an action, to land near it afterwards.
    cursor_row: Option<usize>,
    /// The file in conflict the keys are on, when they are on one: the
    /// cursor of the list above the two others. Never both this and
    /// `cursor`.
    conflict_cursor: Option<String>,
    conflict_scroll: ScrollHandle,
    /// The menu of a row, opened by key.
    key_menu: Option<KeyMenu>,
    busy: bool,
    /// How the checkout stands: for the menus of the folders, and the bar
    /// over the lists while it is sparse.
    sparse: Option<crate::views::sparse::SparseMark>,
    error: Option<String>,
    /// Each list scrolls by itself.
    unstaged_scroll: VirtualListScrollHandle,
    staged_scroll: VirtualListScrollHandle,
    focus_handle: FocusHandle,
}

impl EventEmitter<ChangesEvent> for ChangesView {}

impl ChangesView {
    pub fn new(session: SessionHandle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut view = Self {
            session,
            entries: Vec::new(),
            branch: None,
            conflicts: Vec::new(),
            resolved: BTreeSet::new(),
            conflict_sides: (String::new(), String::new()),
            side_names: None,
            stopped_kind: None,
            stopped: None,
            rows: Rc::new(Vec::new()),
            sections: Rc::new(Vec::new()),
            open_folders: BTreeSet::new(),
            folded: BTreeSet::new(),
            conflicts_folded: false,
            selection: BTreeSet::new(),
            cursor: None,
            cursor_row: None,
            conflict_cursor: None,
            conflict_scroll: ScrollHandle::new(),
            key_menu: None,
            busy: false,
            sparse: None,
            error: None,
            unstaged_scroll: VirtualListScrollHandle::new(),
            staged_scroll: VirtualListScrollHandle::new(),
            focus_handle: cx.focus_handle(),
        };
        view.rebuild(cx);
        view
    }

    /// The branch that is checked out, for the heading.
    pub fn set_branch(&mut self, branch: Option<String>, cx: &mut Context<Self>) {
        if self.branch != branch {
            self.branch = branch;
            cx.notify();
        }
    }

    /// Replace the status entries.
    pub fn set_status(&mut self, entries: Vec<StatusEntry>, cx: &mut Context<Self>) {
        self.entries = entries;
        self.busy = false;
        let exists = |key: &Key, entries: &[StatusEntry]| {
            entries
                .iter()
                .any(|e| e.path == key.1 && change_of(e, key.0).is_some())
        };
        let entries = &self.entries;
        self.selection.retain(|key| exists(key, entries));
        let cursor_gone = self
            .cursor
            .as_ref()
            .is_some_and(|key| !exists(key, entries));
        self.rebuild(cx);
        if cursor_gone {
            // The file under the cursor left its list (staged, say): land on
            // the file that took its place, so that the same key goes on.
            let landing = self.cursor_row.take().and_then(|row| {
                let side = self.cursor.as_ref().map(|key| key.0);
                self.file_row_near(row, side)
            });
            self.cursor = None;
            match landing {
                Some(row) => self.move_to_row(row, cx),
                None => self.selection.clear(),
            }
        }
        cx.notify();
    }

    /// Nothing is selected and the keys are on no file: the file that
    /// was shown was closed by a click on it.
    pub fn unmark(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        self.cursor = None;
        cx.notify();
    }

    /// What the line under the heading calls the two sides, for the
    /// off-screen tests.
    pub fn side_names(&self) -> Option<(String, String)> {
        self.side_names.clone()
    }

    /// How the checkout stands.
    pub fn set_sparse(
        &mut self,
        mark: Option<crate::views::sparse::SparseMark>,
        cx: &mut Context<Self>,
    ) {
        if self.sparse != mark {
            self.sparse = mark;
            cx.notify();
        }
    }

    /// What the two sides of the stopped operation are called: what comes
    /// in, and what it comes into.
    pub fn set_side_names(&mut self, names: Option<(String, String)>, cx: &mut Context<Self>) {
        if self.side_names != names {
            self.side_names = names;
            cx.notify();
        }
    }

    /// The row the keys are on, for the off-screen tests: whether it is a
    /// file in conflict, and its path.
    pub fn keys_on(&self) -> Option<(bool, String)> {
        match &self.conflict_cursor {
            Some(path) => Some((true, path.clone())),
            None => self.cursor.as_ref().map(|(_, path)| (false, path.clone())),
        }
    }

    /// Whether the menu of a row is open, opened by key.
    pub fn key_menu_open(&self) -> bool {
        self.key_menu.is_some()
    }

    /// What the last action that failed said.
    pub fn refusal(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Counts for the row of uncommitted work: (staged, unstaged incl.
    /// untracked).
    pub fn counts(&self) -> (usize, usize) {
        (
            self.entries.iter().filter(|e| e.staged.is_some()).count(),
            self.entries.iter().filter(|e| e.unstaged.is_some()).count(),
        )
    }

    /// The operation in progress changed, or ended: show its conflicts.
    /// `part_of` is what a merge belongs to when it is a step of something
    /// larger (a gitflow finish).
    pub fn set_conflicts(
        &mut self,
        operation: Option<&InProgress>,
        part_of: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let (conflicts, sides) = match operation {
            Some(operation) => (
                operation.conflicts.clone(),
                (operation.ours.clone(), operation.theirs.clone()),
            ),
            None => (Vec::new(), (String::new(), String::new())),
        };
        let stopped = operation.map(|operation| {
            (
                crate::views::stopped::sentence(operation, part_of),
                crate::views::stopped::blocked(operation),
            )
        });
        self.stopped_kind = operation.map(|operation| operation.kind);
        if self.conflicts != conflicts || self.conflict_sides != sides || self.stopped != stopped {
            // The file under the keys was resolved: they land on the one
            // that took its place, so that the same key goes on.
            let was_at = self.conflict_cursor.as_ref().and_then(|path| {
                self.conflicts
                    .iter()
                    .position(|conflict| conflict.path == *path)
            });
            self.conflicts = conflicts;
            self.conflict_sides = sides;
            self.stopped = stopped;
            if let Some(path) = &self.conflict_cursor
                && !self.conflicts.iter().any(|conflict| conflict.path == *path)
            {
                self.conflict_cursor = was_at
                    .and_then(|at| {
                        let last = self.conflicts.len().checked_sub(1)?;
                        self.conflicts.get(at.min(last))
                    })
                    .map(|conflict| conflict.path.clone());
            }
            cx.notify();
        }
        // The files that were in conflict and are resolved: they can be
        // put back in conflict for as long as the operation lasts.
        if operation.is_none() {
            if !self.resolved.is_empty() {
                self.resolved.clear();
                cx.notify();
            }
            return;
        }
        let request = Request::Command {
            id: "conflict.resolved".to_owned(),
            params: json!({}),
        };
        self.session.call(request, cx, |this, reply, cx| {
            let resolved = match reply {
                Reply::Command(value) => resolved_paths(&value),
                _ => BTreeSet::new(),
            };
            if this.resolved != resolved {
                this.resolved = resolved;
                cx.notify();
            }
        });
    }

    /// Files in conflict while no operation is in progress: a patch applied
    /// with a three-way merge leaves them, and so may a command run in a
    /// shell. They are listed as an operation's are, without its sentence
    /// and its two sides; an operation's own list goes first.
    pub fn set_loose_conflicts(&mut self, conflicts: Vec<Conflict>, cx: &mut Context<Self>) {
        if self.stopped.is_some() || self.conflicts == conflicts {
            return;
        }
        self.conflicts = conflicts;
        if let Some(path) = &self.conflict_cursor
            && !self.conflicts.iter().any(|conflict| conflict.path == *path)
        {
            self.conflict_cursor = None;
        }
        cx.notify();
    }

    /// Mark these files resolved as they are now, one after the other; a
    /// file still holding conflict markers is refused and stops there.
    fn resolve_each(&mut self, mut paths: Vec<String>, cx: &mut Context<Self>) {
        if paths.is_empty() {
            self.busy = false;
            cx.emit(ChangesEvent::Mutated);
            cx.notify();
            return;
        }
        let path = paths.remove(0);
        self.busy = true;
        self.error = None;
        cx.notify();
        let request = Request::Command {
            id: "conflict.resolve".to_owned(),
            params: json!({ "path": path }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            if let Reply::Error(err) = reply {
                this.busy = false;
                this.error = Some(err);
                cx.emit(ChangesEvent::Mutated);
                cx.notify();
                return;
            }
            this.resolve_each(paths, cx);
        });
    }

    fn resolve(&mut self, path: &str, how: Resolution, cx: &mut Context<Self>) {
        let (id, params) = match how {
            Resolution::Ours => ("conflict.take", json!({ "path": path, "side": "ours" })),
            Resolution::Theirs => ("conflict.take", json!({ "path": path, "side": "theirs" })),
            Resolution::AsItIs => ("conflict.resolve", json!({ "path": path })),
            Resolution::WithMarkers => (
                "conflict.resolve",
                json!({ "path": path, "allow_markers": true }),
            ),
        };
        self.run(id, params, cx);
    }

    /// The files in conflict, above the two lists: each with its kind and
    /// the ways to resolve it whole. Clicking one opens it.
    fn render_conflicts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.conflicts.is_empty() {
            return None;
        }
        let theme = cx.theme().clone();
        let (ours, theirs) = self.conflict_sides.clone();
        let rows: Vec<AnyElement> = self
            .conflicts
            .iter()
            .enumerate()
            .map(|(ix, conflict)| {
                let path = conflict.path.clone();
                // Rerere has put its remembered resolution in the file: it
                // is no open conflict any more, and waits to be accepted.
                let reused = conflict.reused;
                let kind = match conflict.kind {
                    _ if reused => say!("changes-conflict-reused"),
                    _ if conflict.submodule => say!("changes-conflict-submodule"),
                    ConflictKind::BothModified => say!("changes-conflict-both-modified"),
                    ConflictKind::BothAdded => say!("changes-conflict-both-added"),
                    ConflictKind::DeletedByThem => say!("changes-conflict-deleted-by-them"),
                    ConflictKind::DeletedByUs => say!("changes-conflict-deleted-by-us"),
                    ConflictKind::AddedByUs => say!("changes-conflict-added-by-us"),
                    ConflictKind::AddedByThem => say!("changes-conflict-added-by-them"),
                    ConflictKind::BothDeleted => say!("changes-conflict-both-deleted"),
                };
                let take = |id: &'static str, label: &'static str, how: Resolution| {
                    let path = path.clone();
                    Button::new((id, ix))
                        .ghost()
                        .xsmall()
                        .label(label)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            cx.stop_propagation();
                            this.resolve(&path, how, cx)
                        }))
                };
                let (open, menu_path) = (path.clone(), path.clone());
                let (ours, theirs) = (ours.clone(), theirs.clone());
                let is_cursor = self.conflict_cursor.as_deref() == Some(path.as_str());
                let tip = SharedString::from(path.clone());
                h_flex()
                    .id(("conflict", ix))
                    .test_support()
                    .role(Role::ListItem)
                    .aria_label(crate::access::conflict_row(&path))
                    // Taking a side whole shows while the pointer is
                    // on the row.
                    .group("conflict-row")
                    .w_full()
                    .h(px(FILE_ROW_HEIGHT))
                    .pl(px(21.))
                    .pr(px(10.))
                    .gap(px(5.))
                    .items_center()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                    // The row the keys are on, marked as in the lists
                    // under this one.
                    .when(is_cursor, |d| d.border_1().border_color(theme.ring))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.put_keys_on_conflict(&open, cx);
                        this.focus_handle.focus(window, cx);
                        cx.emit(ChangesEvent::OpenConflict { path: open.clone() })
                    }))
                    .child(
                        Icon::new(if reused {
                            IconName::Check
                        } else {
                            IconName::TriangleAlert
                        })
                        .size_3()
                        .flex_none()
                        .text_color(theme.warning),
                    )
                    // The folder dimmed, then the name.
                    .child({
                        let (folder, name) = split_path(&path);
                        h_flex()
                            .flex_none()
                            .max_w_full()
                            .text_sm()
                            .children(folder.map(|folder| {
                                div()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(format!("{folder}/"))
                            }))
                            .child(div().truncate().child(name.to_owned()))
                    })
                    .child(
                        div()
                            .id(("conflict-kind", ix))
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(if reused {
                                theme.warning
                            } else {
                                theme.muted_foreground
                            })
                            .when(reused, |text| {
                                text.tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(say!(
                                        "changes-conflict-reused-tooltip"
                                    ))
                                    .build(window, cx)
                                })
                            })
                            .child(kind),
                    )
                    // What a remembered resolution waits for comes first.
                    .when(reused, |row| {
                        row.child(
                            take("take-as-is", say!("changes-accept"), Resolution::AsItIs)
                                .tooltip(say!("changes-accept-tooltip")),
                        )
                    })
                    .child(
                        h_flex()
                            .flex_none()
                            .gap_1()
                            .invisible()
                            .group_hover("conflict-row", |style| style.visible())
                            .child(take("take-ours", say!("changes-ours"), Resolution::Ours))
                            .child(take(
                                "take-theirs",
                                say!("changes-theirs"),
                                Resolution::Theirs,
                            )),
                    )
                    .when_some(self.dragged(None, &path), |row, dragged| {
                        row.on_drag(dragged, drag_paths::chip)
                    })
                    .context_menu(move |menu, window, cx| {
                        conflict_menu(menu, window, cx, &menu_path, &ours, &theirs, reused)
                    })
                    .into_any_element()
            })
            .collect();
        let folded = self.conflicts_folded;
        let heading = self.render_heading(
            "heading-conflicts",
            conflicts_heading(
                self.conflicts.len(),
                self.conflicts
                    .iter()
                    .filter(|conflict| conflict.reused)
                    .count(),
            ),
            folded,
            |this, cx| {
                this.conflicts_folded = !this.conflicts_folded;
                cx.notify();
            },
            // Every file marked resolved as it is now, one after the other.
            Some(
                Button::new("resolve-all")
                    .outline()
                    .xsmall()
                    .label(say!("changes-resolve-all"))
                    .tooltip(say!("changes-resolve-all-tooltip"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        cx.stop_propagation();
                        let paths: Vec<String> = this
                            .conflicts
                            .iter()
                            .map(|conflict| conflict.path.clone())
                            .collect();
                        this.resolve_each(paths, cx)
                    })),
            ),
            cx,
        );
        // Headed like the two lists under it, and sharing the height with
        // them.
        Some(
            list_section(v_flex().id("conflict-list"), folded)
                .text_sm()
                .child(heading)
                .when(!folded, |section| {
                    section.child(
                        div()
                            .id("conflict-area")
                            .relative()
                            .flex_1()
                            .min_h_0()
                            .child(
                                v_flex()
                                    .id("conflict-rows")
                                    .size_full()
                                    .overflow_y_scroll()
                                    .track_scroll(&self.conflict_scroll)
                                    .children(rows),
                            )
                            .scrollbar(&self.conflict_scroll, ScrollbarAxis::Vertical)
                            // The wheel moves the list before the panel
                            // around it (`wheel_first`).
                            .child(wheel_first("conflict-wheel", &self.conflict_scroll)),
                    )
                })
                .into_any_element(),
        )
    }

    fn rebuild(&mut self, cx: &mut Context<Self>) {
        let tree = cx.global::<Prefs>().preferences.changes_tree;
        let reversed = cx.global::<Prefs>().preferences.changes_reversed;
        let rows = fold_rows(
            build_rows(&self.entries, tree, &self.open_folders, reversed),
            &self.folded,
        );
        self.sections = Rc::new(sections(&rows));
        self.rows = Rc::new(rows);
        cx.notify();
    }

    /// How low the panel can be: its top and every list's heading and
    /// `rows` rows (two is what the form's grip leaves them; one, what a
    /// window that has no more room does), so that the commit form under
    /// it is what gives way.
    pub fn least_height(&self, rows: f32) -> f32 {
        let open = |folded: bool| {
            if folded {
                HEADING_HEIGHT
            } else {
                HEADING_HEIGHT + rows * FILE_ROW_HEIGHT + 1.
            }
        };
        let lists: f32 = self
            .sections
            .iter()
            .map(|section| open(self.folded.contains(&section.side)))
            .sum();
        let conflicts = if self.conflicts.is_empty() {
            0.
        } else {
            open(self.conflicts_folded)
        };
        // The heading, the row of the two sides while an operation is
        // stopped, and the Path / Tree row.
        let sides = if self.stopped.is_some() { 26. } else { 0. };
        HEADING_HEIGHT + sides + 40. + conflicts + lists
    }

    /// The scroll of a list.
    fn scroll_of(&self, side: Side) -> &VirtualListScrollHandle {
        match side {
            Side::Unstaged => &self.unstaged_scroll,
            Side::Staged => &self.staged_scroll,
        }
    }

    /// The list a row is in.
    fn section_of(&self, row: usize) -> Option<&Section> {
        self.sections
            .iter()
            .find(|section| section.rows.contains(&row))
    }

    /// Fold a list to its heading, or open it again. What was selected in
    /// it is no longer selected: the keys would act on files out of sight.
    fn toggle_folded(&mut self, side: Side, cx: &mut Context<Self>) {
        if !self.folded.remove(&side) {
            self.folded.insert(side);
            self.selection.retain(|key| key.0 != side);
            if self.cursor.as_ref().is_some_and(|key| key.0 == side) {
                self.cursor = None;
            }
        }
        self.rebuild(cx);
    }

    fn key_at(&self, row: usize) -> Option<Key> {
        row_key(self.rows.get(row)?, &self.entries)
    }

    fn row_of(&self, key: &Key) -> Option<usize> {
        self.rows
            .iter()
            .position(|row| row_key(row, &self.entries).as_ref() == Some(key))
    }

    /// What dragging the row of `path` picks up, by full paths:
    /// every selected file when it is one of several selected, else
    /// itself. None without a worktree.
    fn dragged(&self, key: Option<&Key>, path: &str) -> Option<DraggedPaths> {
        let worktree = self.session.summary.workdir.as_deref();
        match key {
            Some(key) if self.selection.len() > 1 && self.selection.contains(key) => {
                DraggedPaths::in_worktree(worktree, self.selection.iter().map(|key| key.1.as_str()))
            }
            _ => DraggedPaths::in_worktree(worktree, [path]),
        }
    }

    /// The file row at `row` or the nearest one after or before it,
    /// preferring `side`.
    fn file_row_near(&self, row: usize, side: Option<Side>) -> Option<usize> {
        let is_file = |ix: &usize| self.key_at(*ix).is_some();
        let on_side = |ix: &usize| {
            self.key_at(*ix)
                .is_some_and(|key| side.is_none_or(|side| key.0 == side))
        };
        let last = self.rows.len().saturating_sub(1);
        let row = row.min(last);
        (row..=last)
            .find(on_side)
            .or_else(|| (0..row).rev().find(on_side))
            .or_else(|| (row..=last).find(is_file))
            .or_else(|| (0..row).rev().find(is_file))
    }

    /// Put the cursor and the selection on one file row and open its diff.
    fn move_to_row(&mut self, row: usize, cx: &mut Context<Self>) {
        let Some(key) = self.key_at(row) else {
            return;
        };
        self.selection = BTreeSet::from([key.clone()]);
        self.cursor = Some(key.clone());
        if let Some(section) = self.section_of(row) {
            self.scroll_of(section.side)
                .scroll_to_item(row.saturating_sub(section.rows.start), ScrollStrategy::Top);
        }
        cx.emit(ChangesEvent::Open {
            path: key.1,
            side: key.0,
        });
        cx.notify();
    }

    /// Where the keys are now.
    fn spot(&self) -> Option<Spot> {
        if let Some(path) = &self.conflict_cursor {
            return self
                .conflicts
                .iter()
                .position(|conflict| conflict.path == *path)
                .map(Spot::Conflict);
        }
        self.cursor
            .as_ref()
            .and_then(|key| self.row_of(key))
            .map(Spot::File)
    }

    /// Put the keys on a file in conflict: the lists under it have no
    /// cursor and no selection then.
    fn put_keys_on_conflict(&mut self, path: &str, cx: &mut Context<Self>) {
        let Some(ix) = self
            .conflicts
            .iter()
            .position(|conflict| conflict.path == path)
        else {
            return;
        };
        self.conflict_cursor = Some(path.to_owned());
        self.cursor = None;
        self.selection.clear();
        self.conflict_scroll.scroll_to_item(ix);
        cx.notify();
    }

    /// The file in conflict the keys are on.
    fn conflict_under_keys(&self) -> Option<&Conflict> {
        let path = self.conflict_cursor.as_ref()?;
        self.conflicts
            .iter()
            .find(|conflict| conflict.path == *path)
    }

    /// The keys go up or down: through the files in conflict first, then
    /// through the two lists under them.
    fn move_cursor(&mut self, delta: isize, cx: &mut Context<Self>) {
        let files: Vec<usize> = (0..self.rows.len())
            .filter(|ix| self.key_at(*ix).is_some())
            .collect();
        match spot_after(self.spot(), delta, self.conflicts.len(), &files) {
            Some(Spot::Conflict(ix)) => {
                if let Some(path) = self.conflicts.get(ix).map(|conflict| conflict.path.clone()) {
                    self.put_keys_on_conflict(&path, cx);
                }
            }
            Some(Spot::File(row)) => {
                self.conflict_cursor = None;
                self.move_to_row(row, cx);
            }
            None => {}
        }
    }

    /// Resolve the file in conflict the keys are on, whole.
    fn resolve_under_keys(&mut self, how: Resolution, cx: &mut Context<Self>) {
        if let Some(path) = self.conflict_under_keys().map(|c| c.path.clone()) {
            self.resolve(&path, how, cx);
        }
    }

    /// The menu key: open the menu of the row the keys are on, at the row.
    fn open_row_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        type Builder = Box<dyn Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu>;
        // `down` is how far the row's lower edge is under the list's top
        // when nothing is scrolled.
        let under = |list: Bounds<Pixels>, offset: Pixels, down: f32| {
            let down = px(down) + offset;
            let y = (list.origin.y + down)
                .max(list.origin.y)
                .min(list.origin.y + list.size.height);
            point(list.origin.x + px(24.), y)
        };
        let (builder, position): (Builder, Point<Pixels>) = match self.spot() {
            Some(Spot::Conflict(ix)) => {
                let Some(conflict) = self.conflicts.get(ix) else {
                    return;
                };
                let (path, reused) = (conflict.path.clone(), conflict.reused);
                let (ours, theirs) = self.conflict_sides.clone();
                (
                    Box::new(move |menu, window, cx| {
                        conflict_menu(menu, window, cx, &path, &ours, &theirs, reused)
                    }),
                    under(
                        self.conflict_scroll.bounds(),
                        self.conflict_scroll.offset().y,
                        FILE_ROW_HEIGHT * ix.saturating_add(1) as f32,
                    ),
                )
            }
            Some(Spot::File(row)) => {
                let Some(Row::File { side, entry, .. }) = self.rows.get(row) else {
                    return;
                };
                let Some(entry) = self.entries.get(*entry) else {
                    return;
                };
                let Some(kind) = change_of(entry, *side) else {
                    return;
                };
                let (side, path) = (*side, entry.path.clone());
                let untracked = kind == ChangeKind::Untracked;
                let was_in_conflict = self.resolved.contains(&entry.path);
                (
                    Box::new(move |menu, window, cx| {
                        file_menu(menu, window, cx, &path, side, untracked, was_in_conflict)
                    }),
                    {
                        let (scroll, first) = match self.section_of(row) {
                            Some(section) => (self.scroll_of(section.side), section.rows.start),
                            None => (&self.unstaged_scroll, 0),
                        };
                        under(
                            scroll.bounds(),
                            scroll.offset().y,
                            self.rows
                                .get(first..=row)
                                .unwrap_or_default()
                                .iter()
                                .map(row_height)
                                .sum(),
                        )
                    },
                )
            }
            None => return,
        };
        // What the menu runs is handled by the panel, which the keyboard
        // goes back to when the menu is gone.
        let panel = self.focus_handle.clone();
        let menu = PopupMenu::build(window, cx, |menu, window, cx| {
            builder(menu.action_context(panel), window, cx)
        });
        let closed = cx.subscribe_in(&menu, window, |this, _, _: &DismissEvent, window, cx| {
            this.key_menu = None;
            this.focus_handle.focus(window, cx);
            cx.notify();
        });
        menu.focus_handle(cx).focus(window, cx);
        self.key_menu = Some(KeyMenu {
            menu,
            position,
            _closed: closed,
        });
        cx.notify();
    }

    fn click_file(&mut self, row: usize, modifiers: Modifiers, cx: &mut Context<Self>) {
        self.conflict_cursor = None;
        let clicked = click_on(
            &self.rows,
            &self.entries,
            &mut self.selection,
            &mut self.cursor,
            row,
            modifiers.shift,
            modifiers.secondary(),
        );
        match clicked {
            Clicked::Nothing => {}
            Clicked::Selected => {
                if self.selection.len() > 1 {
                    cx.emit(ChangesEvent::Several);
                }
                cx.notify()
            }
            Clicked::Opened(key) => {
                cx.emit(ChangesEvent::Clicked {
                    path: key.1,
                    side: key.0,
                });
                cx.notify();
            }
        }
    }

    fn run(&mut self, id: &str, params: serde_json::Value, cx: &mut Context<Self>) {
        self.busy = true;
        self.error = None;
        self.cursor_row = self.cursor.as_ref().and_then(|key| self.row_of(key));
        cx.notify();
        let request = Request::Command {
            id: id.to_owned(),
            params,
        };
        self.session.call(request, cx, |this, reply, cx| {
            this.busy = false;
            if let Reply::Error(err) = reply {
                this.error = Some(err);
            }
            cx.emit(ChangesEvent::Mutated);
            cx.notify();
        });
    }

    /// Paths on `side`, all of them or only the selected ones.
    fn paths(&self, side: Side, selected_only: bool) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| change_of(entry, side).is_some())
            .filter(|entry| !selected_only || self.selection.contains(&(side, entry.path.clone())))
            .map(|entry| entry.path.clone())
            .collect()
    }

    /// Paths on `side` inside the folder `folder`.
    fn paths_under(&self, side: Side, folder: &str) -> Vec<String> {
        self.entries
            .iter()
            .filter(|entry| change_of(entry, side).is_some())
            .filter(|entry| {
                entry
                    .path
                    .strip_prefix(folder)
                    .is_some_and(|rest| rest.starts_with('/'))
            })
            .map(|entry| entry.path.clone())
            .collect()
    }

    fn act(&mut self, command: &str, paths: Vec<String>, cx: &mut Context<Self>) {
        let side = match command {
            "unstage.paths" => Side::Staged,
            _ => Side::Unstaged,
        };
        let paths = with_rename_sources(&self.entries, side, paths);
        if !paths.is_empty() {
            self.run(command, json!({ "paths": paths }), cx);
        }
    }

    /// Put staged files back to what the last commit has: out of the
    /// index, then, once that is done, their changes thrown away. A new
    /// file is deleted. Undo brings each step back.
    fn discard_staged(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let paths = with_rename_sources(&self.entries, Side::Staged, paths);
        if paths.is_empty() {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let request = Request::Command {
            id: "unstage.paths".to_owned(),
            params: json!({ "paths": paths.clone() }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            if let Reply::Error(err) = reply {
                this.busy = false;
                this.error = Some(err);
                cx.emit(ChangesEvent::Mutated);
                cx.notify();
                return;
            }
            this.run("discard.paths", json!({ "paths": paths }), cx);
        });
    }

    /// Hand files whose unstaged changes are to go to the workspace, which
    /// asks first; a moved file takes its old place along.
    fn ask_discard(&mut self, paths: Vec<String>, cx: &mut Context<Self>) {
        let paths = with_rename_sources(&self.entries, Side::Unstaged, paths);
        if !paths.is_empty() {
            cx.emit(ChangesEvent::Discard(paths));
        }
    }

    /// Select every file of the list the cursor is in (the unstaged one
    /// without a cursor).
    fn select_all(&mut self, cx: &mut Context<Self>) {
        let side = self
            .cursor
            .as_ref()
            .map(|key| key.0)
            .unwrap_or(Side::Unstaged);
        self.selection = self
            .paths(side, false)
            .into_iter()
            .map(|path| (side, path))
            .collect();
        cx.notify();
    }

    fn toggle_tree(&mut self, cx: &mut Context<Self>) {
        let prefs = cx.global_mut::<Prefs>();
        prefs.preferences.changes_tree = !prefs.preferences.changes_tree;
        prefs.save_preferences();
        self.rebuild(cx);
    }

    /// The heading of a list: a chevron and its name, which fold the list
    /// to the heading and open it again, and at its right the button that
    /// acts on the whole list, when there is one. A rule under it.
    fn render_heading(
        &self,
        id: &'static str,
        title: String,
        folded: bool,
        toggle: impl Fn(&mut Self, &mut Context<Self>) + 'static,
        button: Option<Button>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .id(id)
            .w_full()
            .h(px(HEADING_HEIGHT))
            .flex_none()
            .pl(px(13.))
            .pr(px(15.))
            .gap(px(5.))
            .items_center()
            .text_sm()
            .border_b_1()
            .border_color(theme.border)
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, _, cx| toggle(this, cx)))
            .child(
                Icon::new(if folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size_3()
                .flex_none()
                .text_color(theme.muted_foreground),
            )
            .child(div().flex_1().min_w_0().truncate().child(title))
            .children(button.map(|button| button.h(px(22.))))
            .test_support()
            .into_any_element()
    }

    /// The heading of one of the two lists. Its button stages or unstages
    /// what of that list is selected, or all of it; a list without files
    /// has none.
    fn render_header(&self, side: Side, count: usize, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selection.iter().filter(|key| key.0 == side).count();
        let button = (count > 0).then(|| {
            let (id, command) = match side {
                Side::Unstaged => ("stage-all", "stage.paths"),
                Side::Staged => ("unstage-all", "unstage.paths"),
            };
            Button::new(id)
                .outline()
                .xsmall()
                .label(heading_button(side, selected))
                .on_click(cx.listener(move |this, _, _, cx| {
                    cx.stop_propagation();
                    let paths = this.paths(side, selected > 0);
                    this.act(command, paths, cx)
                }))
        });
        self.render_heading(
            match side {
                Side::Unstaged => "heading-unstaged",
                Side::Staged => "heading-staged",
            },
            match side {
                Side::Unstaged => say!("changes-unstaged-heading", count),
                Side::Staged => say!("changes-staged-heading", count),
            },
            self.folded.contains(&side),
            move |this, cx| this.toggle_folded(side, cx),
            button,
            cx,
        )
    }

    /// The top of the panel: discard everything, how many files are
    /// changed and on which branch, and the list or tree switch.
    fn render_top(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let tree = cx.global::<Prefs>().preferences.changes_tree;
        let files = self.entries.len();
        let unstaged = self
            .entries
            .iter()
            .filter(|entry| entry.unstaged.is_some())
            .count();
        let selected = self
            .selection
            .iter()
            .filter(|key| key.0 == Side::Unstaged)
            .count();
        v_flex()
            .w_full()
            .flex_none()
            .child(
                h_flex()
                    .w_full()
                    // Taller only when the sentence about a stopped
                    // operation needs a second line.
                    .min_h(px(HEADING_HEIGHT))
                    .py_1()
                    .px(px(10.))
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    // What is not staged is thrown away, of the files that
                    // are selected (their number beside the icon) or of
                    // all; it is asked first. Not while an operation is
                    // stopped, which has its own way back.
                    .when(self.stopped.is_none(), |row| {
                        row.child(
                            Button::new("discard-all")
                                .accessibility_label(say!("changes-discard-all"))
                                .outline()
                                .xsmall()
                                .h(px(22.))
                                .min_w(px(23.))
                                .icon(Icon::new(gpui_kit::assets::IconName::Trash))
                                .when(selected > 0, |button| button.label(selected.to_string()))
                                .tooltip(if selected > 0 {
                                    say!("changes-discard-selected-tooltip")
                                } else {
                                    say!("changes-discard-all-tooltip")
                                })
                                .when(unstaged == 0, |button| button.opacity(0.4))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let paths = this.paths(Side::Unstaged, selected > 0);
                                    this.ask_discard(paths, cx)
                                })),
                        )
                    })
                    .when(self.stopped.is_some(), |row| {
                        row.child(div().w(px(24.)).flex_none())
                    })
                    // While an operation is stopped part way, the panel is
                    // headed by what it waits for.
                    .when_some(self.stopped.clone(), |row, (said, blocked)| {
                        row.child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                // A warning while it cannot go on; a tick
                                // once it can.
                                .child(
                                    Icon::new(if blocked {
                                        IconName::TriangleAlert
                                    } else {
                                        IconName::Check
                                    })
                                    .small()
                                    .flex_none()
                                    .text_color(if blocked {
                                        theme.warning
                                    } else {
                                        theme.success
                                    }),
                                )
                                // What to do next is the end of the
                                // sentence: it goes on a second line
                                // rather than be cut off.
                                .child(div().min_w_0().whitespace_normal().child(said)),
                        )
                    })
                    .when(self.stopped.is_none(), |row| {
                        row.child(
                            h_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_2()
                                .items_center()
                                .justify_center()
                                .text_sm()
                                .child(say!("changes-files-on", count = files))
                                .children(self.branch.clone().map(|branch| {
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .px_1p5()
                                        .rounded(theme.radius)
                                        .bg(theme.primary.opacity(0.25))
                                        .child(branch)
                                })),
                        )
                    })
                    // As wide as the button at the other end, so that the
                    // text stands in the middle.
                    .child(div().w(px(24.)).flex_none()),
            )
            // Under it, which side comes into which: by name, each in a
            // frame of its own.
            .when(
                self.stopped.is_some() && !self.conflict_sides.0.is_empty(),
                |top| {
                    let side = |text: &str| {
                        div()
                            .min_w_0()
                            .truncate()
                            .px_1p5()
                            .rounded(theme.radius)
                            .bg(theme.primary.opacity(0.25))
                            .child(text.to_owned())
                    };
                    let (incoming, into) = self.side_names.clone().unwrap_or_else(|| {
                        (self.conflict_sides.1.clone(), self.conflict_sides.0.clone())
                    });
                    let (verb, joiner) = sides_words(self.stopped_kind);
                    let word = |text: &'static str| {
                        div()
                            .flex_none()
                            .text_color(theme.muted_foreground)
                            .child(text)
                    };
                    top.child(
                        h_flex()
                            .w_full()
                            .h(px(33.))
                            .px_2()
                            .gap(px(5.))
                            .items_center()
                            .justify_center()
                            .text_sm()
                            .child(word(verb))
                            .child(side(&incoming))
                            .child(word(joiner))
                            .child(side(&into)),
                    )
                },
            )
            // The switch stands 11 under the rule, in a row 40 high, centred;
            // the button that turns the order round stands at the left.
            .child(
                h_flex()
                    .relative()
                    .w_full()
                    .h(px(40.))
                    .pt(px(11.))
                    .items_start()
                    .justify_center()
                    .child(div().absolute().left(px(8.)).top(px(11.)).child(
                        sort_button("changes-sort", cx).on_click(cx.listener(|this, _, _, cx| {
                            toggle_order(cx);
                            this.rebuild(cx);
                        })),
                    ))
                    .child(
                        ButtonGroup::new("changes-layout")
                            .outline()
                            .small()
                            .child(
                                chosen(
                                    Button::new("layout-path")
                                        .icon(Icon::new(gpui_kit::assets::IconName::List))
                                        .label(say!("changes-layout-path")),
                                    !tree,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if cx.global::<Prefs>().preferences.changes_tree {
                                            this.toggle_tree(cx)
                                        }
                                    },
                                )),
                            )
                            .child(
                                chosen(
                                    Button::new("layout-tree")
                                        .icon(Icon::new(gpui_kit::assets::IconName::ListTree))
                                        .label(say!("changes-layout-tree")),
                                    tree,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    |this, _, _, cx| {
                                        if !cx.global::<Prefs>().preferences.changes_tree {
                                            this.toggle_tree(cx)
                                        }
                                    },
                                )),
                            ),
                    ),
            )
    }

    /// The first row of a tree: a word that opens every folder of the
    /// list, or closes them all once every one is open.
    fn render_expand(
        &self,
        ix: usize,
        side: Side,
        all_open: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .id(("changes-row", ix))
            .w_full()
            .h(px(ROW_HEIGHT))
            .pl(px(20.))
            .items_center()
            .text_xs()
            .cursor_pointer()
            .child(div().hover(|style| style.underline()).child(if all_open {
                say!("changes-collapse-all")
            } else {
                say!("changes-expand-all")
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                let folders = side_folders(&this.entries, side);
                if all_open {
                    this.open_folders.retain(|key| key.0 != side);
                } else {
                    this.open_folders.extend(folders);
                }
                this.rebuild(cx);
            }))
            .into_any_element()
    }

    /// A folder of the tree: a chevron and its name, dimmed; while it is
    /// closed, how many files of each kind are in it. A click opens it (a
    /// folder that holds nothing but a folder opens with that one) or
    /// closes it.
    #[allow(clippy::too_many_arguments)]
    fn render_folder(
        &self,
        ix: usize,
        side: Side,
        path: &str,
        label: &str,
        depth: usize,
        open: bool,
        counts: file_tree::Counts,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let folder = path.to_owned();
        let menu_folder = path.to_owned();
        let whole_path = SharedString::from(format!("{path}/"));
        // Something in it is tracked unless all of it is new.
        let tracked = self.entries.iter().any(|entry| {
            entry.unstaged != Some(ChangeKind::Untracked)
                && entry
                    .path
                    .strip_prefix(path)
                    .is_some_and(|rest| rest.starts_with('/'))
        });
        let row = h_flex()
            .id(("changes-row", ix))
            .test_support()
            .role(Role::ListItem)
            .aria_label(crate::access::folder_row(path))
            .aria_expanded(open)
            .w_full()
            .h(px(if open {
                OPEN_FOLDER_HEIGHT
            } else {
                FILE_ROW_HEIGHT
            }))
            .pl(px(15. + INDENT * depth as f32))
            .pr(px(10.))
            .gap(px(4.))
            .items_center()
            .text_sm()
            .cursor_pointer()
            .hover(|s| s.bg(theme.accent))
            .tooltip(move |window, cx| Tooltip::new(whole_path.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                if open {
                    this.open_folders.remove(&(side, folder.clone()));
                } else {
                    let paths: Vec<String> = this
                        .entries
                        .iter()
                        .filter(|entry| change_of(entry, side).is_some())
                        .map(|entry| entry.path.clone())
                        .collect();
                    for path in file_tree::opens_with(&folder, paths.iter().map(String::as_str)) {
                        this.open_folders.insert((side, path));
                    }
                }
                this.rebuild(cx);
            }))
            .child(
                Icon::new(if open {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .size_3()
                .flex_none()
                .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_none()
                    .max_w_full()
                    .truncate()
                    .text_color(theme.muted_foreground)
                    .child(label.to_owned()),
            )
            .when(!open, |row| {
                row.child(div().w(px(18.)).flex_none()).children(
                    file_tree::Kind::ALL
                        .into_iter()
                        .filter(|kind| file_tree::count_of(&counts, *kind) > 0)
                        .map(|kind| {
                            let (icon, colour) = tree_mark(kind, &theme);
                            h_flex()
                                .flex_none()
                                .gap_1()
                                .mr_2()
                                .items_center()
                                .text_xs()
                                .child(icon.size_3().text_color(colour))
                                .child(file_tree::count_of(&counts, kind).to_string())
                        }),
                )
            });
        // Dragged out as its folder.
        let row = match self.dragged(None, path) {
            Some(dragged) => row.on_drag(dragged, drag_paths::chip),
            None => row,
        };
        let sparse = self.sparse.clone();
        row.context_menu(move |menu, window, cx| {
            let menu = crate::views::menu_fit::fit(menu, window);
            let folder = menu_folder.clone();
            let ignored = folder.clone();
            let (verb, staging): (&str, Box<dyn Action>) = match side {
                Side::Unstaged => (
                    say!("changes-stage-folder"),
                    Box::new(StageFolder {
                        folder: folder.clone(),
                    }),
                ),
                Side::Staged => (
                    say!("changes-unstage-folder"),
                    Box::new(UnstageFolder {
                        folder: folder.clone(),
                    }),
                ),
            };
            let menu = menu
                .menu(verb, staging)
                .menu(
                    say!("changes-discard"),
                    Box::new(DiscardFolder {
                        folder: folder.clone(),
                    }),
                )
                .submenu(say!("changes-ignore"), window, cx, move |menu, _, _| {
                    ignore_entries(menu, &ignored, true, tracked)
                })
                .separator()
                .menu(
                    say!("changes-open-terminal"),
                    Box::new(RunFileAction {
                        action: FileAction::Terminal {
                            path: folder.clone(),
                        },
                    }),
                )
                .menu(
                    say!("changes-open-folder"),
                    Box::new(RunFileAction {
                        action: FileAction::OpenFolder {
                            path: folder.clone(),
                        },
                    }),
                );
            crate::views::sparse::folder_entries(menu, &folder, sparse.as_ref(), window, cx)
        })
        .into_any_element()
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file(
        &self,
        ix: usize,
        side: Side,
        entry: &StatusEntry,
        depth: usize,
        label: &str,
        folder: Option<&str>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let Some(kind) = change_of(entry, side) else {
            return div().h(px(FILE_ROW_HEIGHT)).into_any_element();
        };
        // A file that was in conflict and is resolved has a tick, unless
        // it only came in with the operation.
        let resolved_here = side == Side::Staged
            && self.resolved.contains(&entry.path)
            && kind != ChangeKind::Added;
        let color = match kind {
            _ if resolved_here => theme.success,
            ChangeKind::Added | ChangeKind::Untracked | ChangeKind::IntentToAdd => theme.green,
            ChangeKind::Deleted => theme.red,
            ChangeKind::Modified | ChangeKind::TypeChanged | ChangeKind::Conflicted => theme.yellow,
            _ => theme.blue,
        };
        let mark = match kind {
            _ if resolved_here => Icon::new(IconName::CircleCheck),
            ChangeKind::Added | ChangeKind::Untracked | ChangeKind::IntentToAdd => {
                Icon::new(IconName::Plus)
            }
            ChangeKind::Deleted => Icon::new(IconName::Minus),
            ChangeKind::Renamed | ChangeKind::Copied => {
                Icon::new(gpui_kit::assets::IconName::FilePen)
            }
            ChangeKind::Conflicted => Icon::new(IconName::TriangleAlert),
            _ => Icon::new(gpui_kit::assets::IconName::Pencil),
        };
        let key = (side, entry.path.clone());
        let selected = self.selection.contains(&key);
        let is_cursor = self.cursor.as_ref() == Some(&key);
        let path = entry.path.clone();
        let menu_path = path.clone();
        let whole_path = SharedString::from(path.clone());
        let row = h_flex()
            .id(("changes-row", ix))
            .test_support()
            .role(Role::ListItem)
            .aria_label(crate::access::file_row(
                &path,
                kind,
                side == Side::Staged,
                resolved_here,
            ))
            .aria_selected(selected)
            .when(is_cursor && self.conflict_cursor.is_none(), |row| {
                row.aria_active_descendant()
            })
            // The button at the row's end shows while the pointer is on
            // the row.
            .group("changes-file")
            .w_full()
            .h(px(FILE_ROW_HEIGHT))
            .pl(px(21. + INDENT * depth as f32))
            .pr(px(10.))
            .gap(px(5.))
            .items_center()
            .text_sm()
            .cursor_pointer()
            .when(selected, |d| d.bg(theme.list_active))
            .when(!selected, |d| d.hover(|s| s.bg(theme.accent)))
            .when(is_cursor, |d| d.border_1().border_color(theme.ring))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus_handle.focus(window, cx);
                this.click_file(ix, event.modifiers(), cx);
            }))
            // The whole path, which the row may have had to cut.
            .tooltip(move |window, cx| Tooltip::new(whole_path.clone()).build(window, cx))
            .child(mark.size_3().flex_none().text_color(color))
            // The folder first and dimmed, then the name.
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .flex_shrink(1.0)
                            .min_w_0()
                            .truncate()
                            .text_color(theme.muted_foreground)
                            .child(match folder {
                                Some(folder) if !folder.is_empty() => format!("{folder}/"),
                                _ => String::new(),
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w_full()
                            .truncate()
                            .child(label.to_owned()),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .invisible()
                    .group_hover("changes-file", |style| style.visible())
                    .child(
                        Button::new(("act-file", ix))
                            .outline()
                            .xsmall()
                            .h(px(22.))
                            .label(if side == Side::Unstaged {
                                say!("changes-stage-file")
                            } else {
                                say!("changes-unstage-file")
                            })
                            .on_click(cx.listener(move |this, _, _, cx| {
                                cx.stop_propagation();
                                let command = if side == Side::Unstaged {
                                    "stage.paths"
                                } else {
                                    "unstage.paths"
                                };
                                this.act(command, vec![path.clone()], cx);
                            })),
                    ),
            );
        // Dragged out by its full path, with the others selected.
        let row = match self.dragged(Some(&key), &entry.path) {
            Some(dragged) => row.on_drag(dragged, drag_paths::chip),
            None => row,
        };
        let untracked = kind == ChangeKind::Untracked;
        let was_in_conflict = self.resolved.contains(&entry.path);
        // A row among several selected of its list: the menu is about them.
        let together: Vec<String> = if selected {
            self.selection
                .iter()
                .filter(|key| key.0 == side)
                .map(|key| key.1.clone())
                .collect()
        } else {
            Vec::new()
        };
        row.context_menu(move |menu, window, cx| {
            // A file's menu is longer than a small window is high.
            let menu = crate::views::menu_fit::fit(menu, window);
            if together.len() > 1 {
                return selection_menu(menu, side, together.clone());
            }
            file_menu(
                menu,
                window,
                cx,
                &menu_path,
                side,
                untracked,
                was_in_conflict,
            )
        })
        .into_any_element()
    }

    fn render_row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        match row {
            // A heading is drawn over its list, not in it.
            Row::Header { .. } => div().into_any_element(),
            Row::Expand { side, all_open } => self.render_expand(ix, *side, *all_open, cx),
            Row::Folder {
                side,
                path,
                label,
                depth,
                open,
                counts,
            } => self.render_folder(ix, *side, path, label, *depth, *open, *counts, cx),
            Row::File {
                side,
                entry,
                depth,
                label,
                folder,
            } => match self.entries.get(*entry) {
                Some(entry) => {
                    self.render_file(ix, *side, entry, *depth, label, folder.as_deref(), cx)
                }
                None => div().h(px(FILE_ROW_HEIGHT)).into_any_element(),
            },
        }
    }
}

impl Focusable for ChangesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ChangesView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        // The lists, one under the other: each its heading, and under it
        // its rows, which scroll by themselves.
        let sections = self.sections.clone();
        let lists: Vec<AnyElement> = sections
            .iter()
            .map(|section| {
                let side = section.side;
                let folded = self.folded.contains(&side);
                let first = section.rows.start;
                let rows = self.rows.clone();
                let (list_id, rows_id, virtual_id, wheel_id) = match side {
                    Side::Unstaged => (
                        "unstaged-list",
                        "unstaged-rows",
                        "unstaged-virtual",
                        "unstaged-wheel",
                    ),
                    Side::Staged => (
                        "staged-list",
                        "staged-rows",
                        "staged-virtual",
                        "staged-wheel",
                    ),
                };
                let heading = self.render_header(side, section.count, cx);
                let scroll = self.scroll_of(side).clone();
                list_section(v_flex().id(list_id), folded)
                    .child(heading)
                    .when(!folded, |list| {
                        list.child(
                            div()
                                .id(rows_id)
                                .relative()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    v_virtual_list(
                                        cx.entity().clone(),
                                        virtual_id,
                                        section.sizes.get(),
                                        move |this, range, _window, cx| {
                                            range
                                                .filter_map(|ix| {
                                                    let ix = ix.saturating_add(first);
                                                    Some(this.render_row(ix, rows.get(ix)?, cx))
                                                })
                                                .collect::<Vec<_>>()
                                        },
                                    )
                                    .track_scroll(&scroll),
                                )
                                .scrollbar(&scroll, ScrollbarAxis::Vertical)
                                // The wheel moves the list before the
                                // panel around it (`wheel_first`).
                                .child(wheel_first(wheel_id, &scroll))
                                .test_support(),
                        )
                    })
                    .into_any_element()
            })
            .collect();

        let conflicts = self.render_conflicts(cx);
        v_flex()
            .id("changes")
            .test_support()
            .key_context(CHANGES_CONTEXT)
            .track_focus(&self.focus_handle)
            // A list for assistive technology, the row the keys are on its
            // active one.
            .role(Role::List)
            .aria_label(crate::access::CHANGES_NAME.text())
            .on_action(cx.listener(|this, a: &StagePath, _, cx| {
                this.act("stage.paths", vec![a.path.clone()], cx)
            }))
            .on_action(
                cx.listener(|this, a: &ResolveConflict, _, cx| this.resolve(&a.path, a.how, cx)),
            )
            .on_action(cx.listener(|this, a: &UnstagePath, _, cx| {
                this.act("unstage.paths", vec![a.path.clone()], cx)
            }))
            .on_action(cx.listener(|this, a: &DiscardPath, _, cx| {
                this.act("discard.paths", vec![a.path.clone()], cx)
            }))
            .on_action(cx.listener(|this, a: &StageMany, _, cx| {
                this.act("stage.paths", a.paths.clone(), cx)
            }))
            .on_action(cx.listener(|this, a: &UnstageMany, _, cx| {
                this.act("unstage.paths", a.paths.clone(), cx)
            }))
            .on_action(cx.listener(|this, a: &DiscardMany, _, cx| match a.side {
                Side::Unstaged => this.ask_discard(a.paths.clone(), cx),
                Side::Staged => this.discard_staged(a.paths.clone(), cx),
            }))
            .on_action(cx.listener(|_, a: &IgnoreMany, _, cx| {
                for path in &a.paths {
                    cx.emit(ChangesEvent::Ignore(IgnoreRequest::Path {
                        path: path.clone(),
                        what: "file".to_owned(),
                    }));
                }
            }))
            .on_action(cx.listener(|this, a: &DiscardStaged, _, cx| {
                this.discard_staged(vec![a.path.clone()], cx)
            }))
            .on_action(cx.listener(|this, a: &StageFolder, _, cx| {
                let paths = this.paths_under(Side::Unstaged, &a.folder);
                this.act("stage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, a: &UnstageFolder, _, cx| {
                let paths = this.paths_under(Side::Staged, &a.folder);
                this.act("unstage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, a: &DiscardFolder, _, cx| {
                let paths = this.paths_under(Side::Unstaged, &a.folder);
                this.ask_discard(paths, cx)
            }))
            .on_action(cx.listener(|this, a: &ReopenConflict, _, cx| {
                this.run("conflict.reopen", json!({ "path": a.path.clone() }), cx)
            }))
            .on_action(cx.listener(|this, a: &ForgetResolution, _, cx| {
                this.run("rerere.forget", json!({ "path": a.path.clone() }), cx)
            }))
            .on_action(cx.listener(|_, ask: &AskIgnore, _, cx| {
                cx.emit(ChangesEvent::Ignore(ask.request.clone()))
            }))
            .on_action(cx.listener(|this, a: &HidePath, _, cx| {
                this.run(
                    "index.set_flag",
                    json!({ "paths": [a.path], "flag": a.flag, "on": true }),
                    cx,
                )
            }))
            .on_action(cx.listener(|_, run: &RunFileAction, _, cx| {
                cx.emit(ChangesEvent::File(run.action.clone()))
            }))
            .on_action(cx.listener(|_, a: &OpenPath, _, cx| {
                cx.emit(ChangesEvent::File(FileAction::Edit {
                    path: a.path.clone(),
                }))
            }))
            .on_action(cx.listener(|_, a: &CopyPath, _, cx| {
                cx.emit(ChangesEvent::File(FileAction::CopyPath {
                    path: a.path.clone(),
                }))
            }))
            .on_action(cx.listener(|this, _: &SelectNext, _, cx| this.move_cursor(1, cx)))
            .on_action(cx.listener(|this, _: &SelectPrev, _, cx| this.move_cursor(-1, cx)))
            .on_action(
                cx.listener(|this, _: &SelectFirst, _, cx| this.move_cursor(isize::MIN / 2, cx)),
            )
            .on_action(
                cx.listener(|this, _: &SelectLast, _, cx| this.move_cursor(isize::MAX / 2, cx)),
            )
            .on_action(cx.listener(|this, _: &StageSelected, _, cx| {
                let paths = this.paths(Side::Unstaged, true);
                this.act("stage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, _: &UnstageSelected, _, cx| {
                let paths = this.paths(Side::Staged, true);
                this.act("unstage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, _: &DiscardSelected, _, cx| {
                let paths = this.paths(Side::Unstaged, true);
                this.act("discard.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, _: &StageAll, _, cx| {
                let paths = this.paths(Side::Unstaged, false);
                this.act("stage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, _: &UnstageAll, _, cx| {
                let paths = this.paths(Side::Staged, false);
                this.act("unstage.paths", paths, cx)
            }))
            .on_action(cx.listener(|this, _: &DiscardAll, _, cx| {
                let paths = this.paths(Side::Unstaged, false);
                this.ask_discard(paths, cx)
            }))
            .on_action(cx.listener(|this, _: &SelectAll, _, cx| this.select_all(cx)))
            .on_action(cx.listener(|this, _: &OpenSelected, _, cx| {
                // A file in conflict opens to be resolved, another one
                // as its diff.
                if let Some(path) = this.conflict_under_keys().map(|c| c.path.clone()) {
                    cx.emit(ChangesEvent::OpenConflict { path });
                } else if let Some((side, path)) = this.cursor.clone() {
                    cx.emit(ChangesEvent::Open { path, side });
                }
            }))
            .on_action(cx.listener(|this, _: &TakeOurs, _, cx| {
                this.resolve_under_keys(Resolution::Ours, cx)
            }))
            .on_action(cx.listener(|this, _: &TakeTheirs, _, cx| {
                this.resolve_under_keys(Resolution::Theirs, cx)
            }))
            .on_action(cx.listener(|this, _: &MarkResolved, _, cx| {
                this.resolve_under_keys(Resolution::AsItIs, cx)
            }))
            .on_action(cx.listener(|this, _: &RowMenu, window, cx| this.open_row_menu(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTreeView, _, cx| this.toggle_tree(cx)))
            .size_full()
            .child(self.render_top(cx))
            // Only part of the files is checked out.
            .children(
                self.sparse
                    .as_ref()
                    .filter(|mark| mark.enabled)
                    .map(|mark| crate::views::sparse::bar(mark, cx)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_h_0()
                    .overflow_hidden()
                    .children(conflicts)
                    .children(lists),
            )
            // A menu opened by key stands over everything, at its row.
            .children(self.key_menu.as_ref().map(|open| {
                deferred(
                    anchored()
                        .position(open.position)
                        .snap_to_window_with_margin(px(8.))
                        .child(open.menu.clone()),
                )
                .with_priority(1)
            }))
            .when_some(self.error.clone(), |d, err| {
                d.child(div().p_2().text_xs().text_color(theme.danger).child(err))
            })
            .when(self.busy, |d| {
                d.child(
                    div()
                        .p_2()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(say!("changes-working")),
                )
            })
    }
}

/// The words of the line that says which side comes into which: what is
/// going on, and how the two are joined.
fn sides_words(kind: Option<bitbraid_git::sequence::Kind>) -> (&'static str, &'static str) {
    use bitbraid_git::sequence::Kind;
    match kind {
        Some(Kind::Rebase) => (say!("changes-sides-rebasing"), say!("changes-sides-onto")),
        Some(Kind::CherryPick) => (say!("changes-sides-picking"), say!("changes-sides-into")),
        Some(Kind::Revert) => (say!("changes-sides-reverting"), say!("changes-sides-in")),
        Some(Kind::Mailbox) => (say!("changes-sides-applying"), say!("changes-sides-to")),
        Some(Kind::Merge) | None => (say!("changes-sides-merging"), say!("changes-sides-into")),
    }
}

/// The button that turns the order of the files round: A to Z, Z to A.
/// The order is one for every list of files.
pub(crate) fn sort_button(id: &'static str, cx: &App) -> Button {
    let reversed = cx.global::<Prefs>().preferences.changes_reversed;
    Button::new(id)
        .accessibility_label(say!("changes-order"))
        .ghost()
        .xsmall()
        .h(px(24.))
        .icon(Icon::new(if reversed {
            gpui_kit::assets::IconName::ArrowDownZA
        } else {
            gpui_kit::assets::IconName::ArrowDownAZ
        }))
        .tooltip(if reversed {
            say!("changes-order-reversed")
        } else {
            say!("changes-order-forward")
        })
}

/// Turn the order of the files round, for every list of them.
pub(crate) fn toggle_order(cx: &mut App) {
    let prefs = cx.global_mut::<Prefs>();
    prefs.preferences.changes_reversed = !prefs.preferences.changes_reversed;
    prefs.save_preferences();
}

/// What makes a list take the wheel before the panel it is in. In a low
/// window (a large zoom) the panel of the changes and the form scrolls as
/// a whole, and the toolkit hands a wheel to every scrolling area under
/// the pointer, which would move the list and the panel together. This
/// takes the wheel while the list can move that way, and hands it on to
/// the panel at the list's end or when the list has nothing to scroll. It
/// is the toolkit's mask: a child of the list's area after the list,
/// covering it.
pub(crate) fn wheel_first<H>(id: &'static str, scroll: &H) -> ScrollableMask<H>
where
    H: gpui_kit::component::scroll::ScrollbarHandle + Clone,
{
    ScrollableMask::new(Axis::Vertical, scroll).id(id)
}

/// A list's place in the panel: open, it shares the height with the other
/// open lists and is never shorter than its heading and a row; folded, it
/// is its heading. Two rows each is what the form's grip leaves them.
fn list_section<E: Styled>(element: E, folded: bool) -> E {
    if folded {
        element.w_full().flex_none()
    } else {
        element
            .w_full()
            .flex_grow(1.)
            .flex_shrink(1.)
            .flex_basis(px(0.))
            .min_h(px(HEADING_HEIGHT + FILE_ROW_HEIGHT))
    }
}

/// What the button on a list's heading says: what it does to the files of
/// the list that are selected, or to all of them.
pub(crate) fn heading_button(side: Side, selected: usize) -> String {
    match (side, selected) {
        (Side::Unstaged, 0) => say!("changes-stage-all").to_owned(),
        (Side::Staged, 0) => say!("changes-unstage-all").to_owned(),
        (Side::Unstaged, count) => say!("changes-stage-files", count),
        (Side::Staged, count) => say!("changes-unstage-files", count),
    }
}

/// The heading over the files in conflict: how many there are, and how
/// many of them hold a resolution that was remembered and only wait to be
/// accepted.
pub(crate) fn conflicts_heading(all: usize, reused: usize) -> String {
    match reused {
        0 => say!("changes-conflicts-heading", all),
        n if n >= all => say!("changes-conflicts-heading-all-reused", all),
        n => say!("changes-conflicts-heading-reused", all, reused = n),
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::collections::BTreeSet;

    use bitbraid_git::{ChangeKind, StatusEntry};

    use super::{
        Clicked, Key, Row, Side, Spot, click_on, fold_rows, heading_button, sections, side_folders,
        spot_after, with_rename_sources,
    };

    /// The rows A to Z, with the folders in `open` open.
    fn build_rows(entries: &[StatusEntry], tree: bool, open: &BTreeSet<Key>) -> Vec<Row> {
        super::build_rows(entries, tree, open, false)
    }

    /// Every folder of both lists, open.
    fn all_open(entries: &[StatusEntry]) -> BTreeSet<Key> {
        let mut open = side_folders(entries, Side::Unstaged);
        open.extend(side_folders(entries, Side::Staged));
        open
    }

    fn entry(path: &str, staged: bool, unstaged: bool) -> StatusEntry {
        StatusEntry {
            path: path.to_owned(),
            original_path: None,
            staged: staged.then_some(ChangeKind::Modified),
            unstaged: unstaged.then_some(ChangeKind::Modified),
            conflict: false,
        }
    }

    /// Rows as text: indentation by depth, `/` after folders.
    fn sketch(rows: &[Row], entries: &[StatusEntry]) -> Vec<String> {
        rows.iter()
            .map(|row| match row {
                Row::Header { side, count } => format!("{side:?} {count}"),
                Row::Expand { all_open, .. } => if *all_open {
                    "  (collapse all)"
                } else {
                    "  (expand all)"
                }
                .to_owned(),
                Row::Folder {
                    label,
                    depth,
                    open,
                    counts,
                    ..
                } => format!(
                    "{}{label}/{}",
                    "  ".repeat(*depth + 1),
                    if *open {
                        String::new()
                    } else {
                        format!(" closed {counts:?}")
                    }
                ),
                Row::File {
                    entry,
                    depth,
                    label,
                    folder,
                    ..
                } => format!(
                    "{}{label}{} <{}>",
                    "  ".repeat(*depth + 1),
                    folder
                        .as_ref()
                        .map(|f| format!(" in {f}"))
                        .unwrap_or_default(),
                    entries[*entry].path
                ),
            })
            .collect()
    }

    fn entries() -> Vec<StatusEntry> {
        vec![
            entry("CHANGES.md", false, true),
            entry("crates/git/src/log.rs", true, true),
            entry("crates/git/src/watch.rs", false, true),
            entry("crates/ui/src/views/graph.rs", false, true),
            entry("docs/a/b/c/deep.md", true, false),
        ]
    }

    #[test]
    fn the_list_of_paths_shows_the_name_and_the_folder_apart() {
        let entries = entries();
        let rows = build_rows(&entries, false, &BTreeSet::new());
        assert_eq!(
            sketch(&rows, &entries),
            [
                "Unstaged 4",
                "  CHANGES.md <CHANGES.md>",
                "  log.rs in crates/git/src <crates/git/src/log.rs>",
                "  watch.rs in crates/git/src <crates/git/src/watch.rs>",
                "  graph.rs in crates/ui/src/views <crates/ui/src/views/graph.rs>",
                "Staged 2",
                "  log.rs in crates/git/src <crates/git/src/log.rs>",
                "  deep.md in docs/a/b/c <docs/a/b/c/deep.md>",
            ]
        );
    }

    #[test]
    fn the_tree_starts_closed_and_has_a_row_for_every_folder() {
        let entries = entries();
        let rows = build_rows(&entries, true, &BTreeSet::new());
        assert_eq!(
            sketch(&rows, &entries),
            [
                "Unstaged 4",
                "  (expand all)",
                "  crates/ closed [3, 0, 0, 0]",
                "  CHANGES.md <CHANGES.md>",
                "Staged 2",
                "  (expand all)",
                "  crates/ closed [1, 0, 0, 0]",
                "  docs/ closed [1, 0, 0, 0]",
            ]
        );
        let rows = build_rows(&entries, true, &all_open(&entries));
        assert_eq!(
            sketch(&rows, &entries),
            [
                "Unstaged 4",
                "  (collapse all)",
                "  crates/",
                "    git/",
                "      src/",
                "        log.rs <crates/git/src/log.rs>",
                "        watch.rs <crates/git/src/watch.rs>",
                "    ui/",
                "      src/",
                "        views/",
                "          graph.rs <crates/ui/src/views/graph.rs>",
                "  CHANGES.md <CHANGES.md>",
                "Staged 2",
                "  (collapse all)",
                "  crates/",
                "    git/",
                "      src/",
                "        log.rs <crates/git/src/log.rs>",
                "  docs/",
                "    a/",
                "      b/",
                "        c/",
                "          deep.md <docs/a/b/c/deep.md>",
            ]
        );
    }

    #[test]
    fn a_closed_folder_hides_what_is_in_it_on_its_side_only() {
        let entries = entries();
        let mut open = all_open(&entries);
        open.remove(&(Side::Unstaged, "crates".to_owned()));
        let rows = build_rows(&entries, true, &open);
        assert_eq!(
            sketch(&rows, &entries),
            [
                "Unstaged 4",
                "  (expand all)",
                "  crates/ closed [3, 0, 0, 0]",
                "  CHANGES.md <CHANGES.md>",
                "Staged 2",
                "  (collapse all)",
                "  crates/",
                "    git/",
                "      src/",
                "        log.rs <crates/git/src/log.rs>",
                "  docs/",
                "    a/",
                "      b/",
                "        c/",
                "          deep.md <docs/a/b/c/deep.md>",
            ]
        );
        // Z to A, as a list of paths.
        let rows = super::build_rows(&entries, false, &open, true);
        let paths: Vec<String> = sketch(&rows, &entries)
            .into_iter()
            .filter_map(|line| {
                line.split_once('<')
                    .map(|(_, path)| path.trim_end_matches('>').to_owned())
            })
            .collect();
        assert_eq!(
            paths,
            [
                "crates/ui/src/views/graph.rs",
                "crates/git/src/watch.rs",
                "crates/git/src/log.rs",
                "CHANGES.md",
                "docs/a/b/c/deep.md",
                "crates/git/src/log.rs",
            ]
        );
    }

    #[test]
    fn each_list_is_its_heading_and_the_rows_up_to_the_next() {
        let entries = entries();
        let rows = build_rows(&entries, false, &BTreeSet::new());
        let lists = sections(&rows);
        let shape: Vec<(Side, usize, std::ops::Range<usize>, usize)> = lists
            .iter()
            .map(|list| (list.side, list.count, list.rows.clone(), list.sizes.len()))
            .collect();
        assert_eq!(
            shape,
            [(Side::Unstaged, 4, 1..5, 4), (Side::Staged, 2, 6..8, 2)]
        );
        // A file's row is 32 high.
        assert!(lists.iter().all(|list| {
            list.sizes
                .get()
                .iter()
                .all(|size| f32::from(size.height) == 32.)
        }));
        // Folded, a list keeps its heading and loses its rows, and the
        // other list's rows are found where they now are.
        let folded = fold_rows(rows, &[Side::Unstaged].into());
        let lists = sections(&folded);
        let shape: Vec<(Side, usize, std::ops::Range<usize>)> = lists
            .iter()
            .map(|list| (list.side, list.count, list.rows.clone()))
            .collect();
        assert_eq!(shape, [(Side::Unstaged, 4, 1..1), (Side::Staged, 2, 2..4)]);
        assert_eq!(
            sketch(&folded, &entries),
            [
                "Unstaged 4",
                "Staged 2",
                "  log.rs in crates/git/src <crates/git/src/log.rs>",
                "  deep.md in docs/a/b/c <docs/a/b/c/deep.md>",
            ]
        );
        assert!(sections(&[]).is_empty());
    }

    #[test]
    fn a_moved_file_is_acted_on_with_the_place_it_was_moved_from() {
        let mut moved = entry("web/add.js", true, false);
        moved.staged = Some(ChangeKind::Renamed);
        moved.original_path = Some("web/sum.js".to_owned());
        let entries = vec![moved, entry("CHANGES.md", true, true)];
        let paths = |side, list: &[&str]| {
            with_rename_sources(
                &entries,
                side,
                list.iter().map(|path| (*path).to_owned()).collect(),
            )
        };
        assert_eq!(
            paths(Side::Staged, &["CHANGES.md", "web/add.js"]),
            ["CHANGES.md", "web/add.js", "web/sum.js"]
        );
        // Named already, it is not named twice.
        assert_eq!(
            paths(Side::Staged, &["web/sum.js", "web/add.js"]),
            ["web/sum.js", "web/add.js"]
        );
        // On the side where it is no move, nothing is added.
        assert_eq!(paths(Side::Unstaged, &["web/add.js"]), ["web/add.js"]);
        assert!(paths(Side::Staged, &[]).is_empty());
    }

    #[test]
    fn the_button_of_a_heading_says_what_it_takes() {
        assert_eq!(heading_button(Side::Unstaged, 0), "Stage all");
        assert_eq!(heading_button(Side::Unstaged, 1), "Stage 1 file");
        assert_eq!(heading_button(Side::Staged, 0), "Unstage all");
        assert_eq!(heading_button(Side::Staged, 3), "Unstage 3 files");
    }

    #[test]
    fn an_empty_list_is_its_heading_and_odd_paths_do_not_break_the_tree() {
        let rows = build_rows(&[], true, &BTreeSet::new());
        assert_eq!(sketch(&rows, &[]), ["Unstaged 0", "Staged 0"]);
        // Empty components, a trailing slash, a file and a folder of one name.
        let entries = vec![
            entry("a//b", false, true),
            entry("a/", false, true),
            entry("a", false, true),
            entry("", false, true),
        ];
        for tree in [false, true] {
            let rows = build_rows(&entries, tree, &all_open(&entries));
            let files = rows
                .iter()
                .filter(|row| matches!(row, Row::File { .. }))
                .count();
            assert_eq!(files, 4, "every entry keeps a row (tree: {tree})");
        }
    }

    // Clicks. The rows of `entries()` as a list of paths:
    //   0 Unstaged 4      1 CHANGES.md  2 log.rs   3 watch.rs   4 graph.rs
    //   5 Staged 2        6 log.rs      7 deep.md

    const README: &str = "CHANGES.md";
    const LOG: &str = "crates/git/src/log.rs";
    const WATCH: &str = "crates/git/src/watch.rs";
    const GRAPH: &str = "crates/ui/src/views/graph.rs";
    const DEEP: &str = "docs/a/b/c/deep.md";

    fn unstaged(path: &str) -> Key {
        (Side::Unstaged, path.to_owned())
    }

    fn staged(path: &str) -> Key {
        (Side::Staged, path.to_owned())
    }

    /// A panel's rows with its selection and cursor, to click in.
    struct Panel {
        entries: Vec<StatusEntry>,
        rows: Vec<Row>,
        selection: BTreeSet<Key>,
        cursor: Option<Key>,
    }

    impl Panel {
        fn new(tree: bool, collapsed: &BTreeSet<Key>) -> Self {
            let entries = entries();
            let rows = build_rows(&entries, tree, collapsed);
            Self {
                entries,
                rows,
                selection: BTreeSet::new(),
                cursor: None,
            }
        }
        fn list() -> Self {
            Self::new(false, &BTreeSet::new())
        }
        fn press(&mut self, row: usize, shift: bool, secondary: bool) -> Clicked {
            click_on(
                &self.rows,
                &self.entries,
                &mut self.selection,
                &mut self.cursor,
                row,
                shift,
                secondary,
            )
        }
        fn click(&mut self, row: usize) -> Clicked {
            self.press(row, false, false)
        }
        fn ctrl_click(&mut self, row: usize) -> Clicked {
            self.press(row, false, true)
        }
        fn shift_click(&mut self, row: usize) -> Clicked {
            self.press(row, true, false)
        }
        /// The status came anew, or a folder was opened or closed.
        fn relaid(&mut self, entries: Vec<StatusEntry>, tree: bool, collapsed: &BTreeSet<Key>) {
            self.rows = build_rows(&entries, tree, collapsed);
            self.entries = entries;
        }
        fn selected(&self) -> Vec<Key> {
            self.selection.iter().cloned().collect()
        }
    }

    #[test]
    fn a_click_selects_one_file_and_opens_it() {
        let mut panel = Panel::list();
        assert_eq!(panel.click(2), Clicked::Opened(unstaged(LOG)));
        assert_eq!(panel.selected(), [unstaged(LOG)]);
        assert_eq!(panel.cursor, Some(unstaged(LOG)));
        // Another click replaces the selection, whatever it was; the same
        // path in the other list is another file.
        panel.ctrl_click(4);
        assert_eq!(panel.click(6), Clicked::Opened(staged(LOG)));
        assert_eq!(panel.selected(), [staged(LOG)]);
        assert_eq!(panel.cursor, Some(staged(LOG)));
        // Clicking it again opens it again and leaves it selected.
        assert_eq!(panel.click(6), Clicked::Opened(staged(LOG)));
        assert_eq!(panel.selected(), [staged(LOG)]);

        // What is not a file takes no click: a header, a row that is not
        // there, with or without a key held.
        for row in [0, 5, 8, usize::MAX] {
            for (shift, secondary) in [(false, false), (true, false), (false, true), (true, true)] {
                assert_eq!(panel.press(row, shift, secondary), Clicked::Nothing);
            }
        }
        assert_eq!(panel.selected(), [staged(LOG)]);
        assert_eq!(panel.cursor, Some(staged(LOG)));
        // Nor the note of an empty list.
        let mut empty = Panel::list();
        empty.relaid(Vec::new(), false, &BTreeSet::new());
        for row in 0..4 {
            assert_eq!(empty.click(row), Clicked::Nothing);
            assert_eq!(empty.shift_click(row), Clicked::Nothing);
        }
        assert!(empty.selection.is_empty() && empty.cursor.is_none());
    }

    #[test]
    fn a_click_with_the_secondary_key_adds_a_file_or_takes_it_out() {
        let mut panel = Panel::list();
        panel.click(1);
        // Added, and the cursor goes with it; no diff is opened.
        assert_eq!(panel.ctrl_click(3), Clicked::Selected);
        assert_eq!(panel.selected(), [unstaged(README), unstaged(WATCH)]);
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));
        // In the other list too: the two sides of one path are apart.
        assert_eq!(panel.ctrl_click(6), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(README), unstaged(WATCH), staged(LOG)]
        );
        assert!(!panel.selection.contains(&unstaged(LOG)));
        // Taken out again; the cursor is on it all the same.
        assert_eq!(panel.ctrl_click(3), Clicked::Selected);
        assert_eq!(panel.selected(), [unstaged(README), staged(LOG)]);
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));
        // Down to nothing.
        panel.ctrl_click(1);
        panel.ctrl_click(6);
        assert!(panel.selection.is_empty());
        assert_eq!(panel.cursor, Some(staged(LOG)));
        // With nothing selected and no cursor, it selects the one file.
        let mut fresh = Panel::list();
        assert_eq!(fresh.ctrl_click(4), Clicked::Selected);
        assert_eq!(fresh.selected(), [unstaged(GRAPH)]);
        assert_eq!(fresh.cursor, Some(unstaged(GRAPH)));
    }

    #[test]
    fn a_click_with_shift_selects_the_range_from_the_cursor() {
        // Forwards.
        let mut panel = Panel::list();
        panel.click(1);
        assert_eq!(panel.shift_click(3), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(README), unstaged(LOG), unstaged(WATCH)]
        );
        // The cursor stays at the end the range began at, so the next
        // range begins there too.
        assert_eq!(panel.cursor, Some(unstaged(README)));
        assert_eq!(panel.shift_click(4), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [
                unstaged(README),
                unstaged(LOG),
                unstaged(WATCH),
                unstaged(GRAPH)
            ]
        );
        // A range joins the selection: a shorter one takes nothing out.
        assert_eq!(panel.shift_click(2), Clicked::Selected);
        assert_eq!(panel.selection.len(), 4);
        assert_eq!(panel.cursor, Some(unstaged(README)));

        // Backwards.
        let mut panel = Panel::list();
        panel.click(4);
        assert_eq!(panel.shift_click(2), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(LOG), unstaged(WATCH), unstaged(GRAPH)]
        );
        assert_eq!(panel.cursor, Some(unstaged(GRAPH)));

        // On the cursor itself: that one file.
        let mut panel = Panel::list();
        panel.click(3);
        assert_eq!(panel.shift_click(3), Clicked::Selected);
        assert_eq!(panel.selected(), [unstaged(WATCH)]);

        // What was selected apart from the range stays, and the range
        // starts from the file that was added last, which has the cursor.
        let mut panel = Panel::list();
        panel.click(1);
        panel.ctrl_click(3);
        assert_eq!(panel.shift_click(4), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(README), unstaged(WATCH), unstaged(GRAPH)]
        );
        // Shift wins over the secondary key when both are held.
        assert_eq!(panel.press(2, true, true), Clicked::Selected);
        assert_eq!(panel.selection.len(), 4);
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));
    }

    #[test]
    fn a_range_reaches_across_the_two_lists_and_over_folders() {
        // From the unstaged list into the staged one: the header between
        // them is no file, and the files on both sides are taken.
        let mut panel = Panel::list();
        panel.click(3);
        assert_eq!(panel.shift_click(7), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(WATCH), unstaged(GRAPH), staged(LOG), staged(DEEP)]
        );
        // And back from the staged list into the unstaged one.
        let mut panel = Panel::list();
        panel.click(6);
        assert_eq!(panel.shift_click(4), Clicked::Selected);
        assert_eq!(panel.selected(), [unstaged(GRAPH), staged(LOG)]);

        // In the tree the folders in between are passed over:
        //   0 Unstaged 4   1 (collapse all)   2 crates/   3 git/   4 src/
        //   5 log.rs   6 watch.rs   7 ui/   8 src/   9 views/   10 graph.rs
        //   11 CHANGES.md   12 Staged 2 ...
        let mut panel = Panel::new(true, &all_open(&entries()));
        assert_eq!(panel.click(5), Clicked::Opened(unstaged(LOG)));
        assert_eq!(panel.shift_click(11), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [
                unstaged(README),
                unstaged(LOG),
                unstaged(WATCH),
                unstaged(GRAPH)
            ]
        );
        // A folder is no end of a range, nor is the row that opens them.
        assert_eq!(panel.shift_click(2), Clicked::Nothing);
        assert_eq!(panel.shift_click(1), Clicked::Nothing);
        assert_eq!(panel.selection.len(), 4);
    }

    #[test]
    fn shift_without_a_cursor_in_the_list_is_a_plain_click() {
        // No cursor yet.
        let mut panel = Panel::list();
        assert_eq!(panel.shift_click(3), Clicked::Opened(unstaged(WATCH)));
        assert_eq!(panel.selected(), [unstaged(WATCH)]);
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));

        // The file under the cursor left its list (it was staged whole):
        // there is no row to start a range from, so the click selects the
        // file alone and the cursor moves to it.
        let mut panel = Panel::list();
        panel.click(1);
        panel.ctrl_click(3);
        let mut entries = entries();
        entries[2] = entry(WATCH, true, false);
        panel.relaid(entries, false, &BTreeSet::new());
        // Now: 0 Unstaged 3, 1 README, 2 log.rs, 3 graph.rs, 4 Staged 3,
        // 5 log.rs, 6 watch.rs, 7 deep.md.
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));
        assert_eq!(panel.shift_click(3), Clicked::Opened(unstaged(GRAPH)));
        assert_eq!(panel.selected(), [unstaged(GRAPH)]);
        assert_eq!(panel.cursor, Some(unstaged(GRAPH)));
        // From there a range works again, into the list the file went to.
        assert_eq!(panel.shift_click(6), Clicked::Selected);
        assert_eq!(
            panel.selected(),
            [unstaged(GRAPH), staged(LOG), staged(WATCH)]
        );

        // With the secondary key held as well, the file is added or taken
        // out, as without Shift. (What the selection still holds of files
        // that are gone is the view's to drop when the status arrives.)
        let mut panel = Panel::list();
        panel.click(1);
        panel.relaid(
            vec![entry(LOG, false, true), entry(WATCH, false, true)],
            false,
            &BTreeSet::new(),
        );
        assert_eq!(panel.press(2, true, true), Clicked::Selected);
        assert_eq!(panel.selected(), [unstaged(README), unstaged(WATCH)]);
        assert_eq!(panel.cursor, Some(unstaged(WATCH)));

        // The file under the cursor is in a folder that was closed: it has
        // no row either.
        let mut panel = Panel::new(true, &all_open(&self::entries()));
        panel.click(5);
        let mut open = all_open(&self::entries());
        open.remove(&(Side::Unstaged, "crates".to_owned()));
        panel.relaid(self::entries(), true, &open);
        // Now: 0 Unstaged 4, 1 (expand all), 2 crates/ (closed),
        // 3 CHANGES.md, 4 Staged 2 ...
        assert_eq!(panel.shift_click(3), Clicked::Opened(unstaged(README)));
        assert_eq!(panel.selected(), [unstaged(README)]);
    }

    #[test]
    fn the_keys_go_through_the_conflicts_and_then_through_the_files() {
        // Rows 0, 3 and 4 are headings; two files are in conflict.
        let files = [1usize, 2, 5];
        let step = |from, delta| spot_after(from, delta, 2, &files);
        // From nowhere: down is the first there is, up the last; all the
        // way up and all the way down are the two ends.
        assert_eq!(step(None, 1), Some(Spot::Conflict(0)));
        assert_eq!(step(None, -1), Some(Spot::File(5)));
        assert_eq!(step(None, isize::MIN / 2), Some(Spot::Conflict(0)));
        assert_eq!(step(None, isize::MAX / 2), Some(Spot::File(5)));
        // Down through the conflicts into the files, and back up.
        assert_eq!(step(Some(Spot::Conflict(0)), 1), Some(Spot::Conflict(1)));
        assert_eq!(step(Some(Spot::Conflict(1)), 1), Some(Spot::File(1)));
        assert_eq!(step(Some(Spot::File(1)), -1), Some(Spot::Conflict(1)));
        assert_eq!(step(Some(Spot::File(2)), 1), Some(Spot::File(5)));
        // No further than the ends.
        assert_eq!(step(Some(Spot::File(5)), 1), Some(Spot::File(5)));
        assert_eq!(step(Some(Spot::Conflict(0)), -1), Some(Spot::Conflict(0)));
        assert_eq!(
            step(Some(Spot::File(2)), isize::MIN / 2),
            Some(Spot::Conflict(0))
        );
        // A spot that is no longer there counts as nowhere.
        assert_eq!(step(Some(Spot::Conflict(7)), 1), Some(Spot::Conflict(0)));
        assert_eq!(step(Some(Spot::File(4)), -1), Some(Spot::File(5)));
        // Without conflicts the files are all there is.
        assert_eq!(spot_after(None, 1, 0, &files), Some(Spot::File(1)));
        assert_eq!(
            spot_after(Some(Spot::File(1)), -1, 0, &files),
            Some(Spot::File(1))
        );
        // Only conflicts; and nothing at all.
        assert_eq!(spot_after(None, -1, 3, &[]), Some(Spot::Conflict(2)));
        assert_eq!(spot_after(None, 1, 0, &[]), None);
        assert_eq!(spot_after(Some(Spot::File(1)), 1, 0, &[]), None);
    }
}
