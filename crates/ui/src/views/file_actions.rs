//! What can be done with a file from any list that shows one:
//! go to its history or its blame; show its changes in the diff tool; open
//! it in the editor, as it is now or as it was in a commit, or with the
//! system's own program; show it in the file manager; copy its path; and
//! for a file of a commit, put it back as the commit has it.
//!
//! The lists (the changes panel, the files of a commit) only put the
//! entries into their menus and pass on what was picked; the workspace
//! carries it out, since it owns the file pane and says what went wrong.
//!
//! Sources: none (original design).

use bitbraid_git::Oid;
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// Something to do with a file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub enum FileAction {
    /// Open the file of the worktree in the editor.
    Edit {
        /// Path relative to the worktree.
        path: String,
    },
    /// Open the file as it is in a commit, from a copy outside the
    /// repository.
    EditAt {
        /// Path in that commit.
        path: String,
        /// The commit.
        rev: Oid,
    },
    /// Show the file of the worktree in the file manager.
    Reveal {
        /// Path relative to the worktree.
        path: String,
    },
    /// Show the file's changes in the configured diff tool: what is not
    /// staged, what is staged, or what a commit changed.
    DiffTool {
        /// Path.
        path: String,
        /// The staged changes rather than the unstaged ones.
        staged: bool,
        /// The commit whose changes are shown.
        commit: Option<Oid>,
    },
    /// Open the file of the worktree with the system's own program for it.
    OpenDefault {
        /// Path relative to the worktree.
        path: String,
    },
    /// Put the file of the worktree back as it is in a commit.
    Restore {
        /// Path.
        path: String,
        /// The commit.
        rev: Oid,
    },
    /// Resolve a file in conflict in the configured merge tool.
    MergeTool {
        /// Path relative to the worktree.
        path: String,
    },
    /// Open a folder of the worktree in the file manager.
    OpenFolder {
        /// Path relative to the worktree.
        path: String,
    },
    /// Start a terminal in the window in a folder of the worktree.
    Terminal {
        /// Path relative to the worktree.
        path: String,
    },
    /// Check out only this folder and the files at the top.
    SparseOnly {
        /// Path of the folder relative to the worktree.
        path: String,
    },
    /// Check this folder out too, beside what a sparse checkout has.
    SparseAdd {
        /// Path of the folder relative to the worktree.
        path: String,
    },

    /// Copy the path.
    CopyPath {
        /// Path relative to the worktree.
        path: String,
    },
    /// The commits that changed the file.
    History {
        /// Path.
        path: String,
        /// The commit the file was listed in; none for a file of the
        /// worktree.
        rev: Option<Oid>,
    },
    /// Who last changed each line, as of a commit or of the last commit.
    Blame {
        /// Path.
        path: String,
        /// The commit to blame at.
        rev: Option<Oid>,
    },
}

/// The action a menu entry stands for.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = file_actions, no_json)]
pub struct RunFileAction {
    /// What to do.
    pub action: FileAction,
}

/// Where a file that is listed comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listed {
    /// The worktree: it is there to be opened as it is.
    Worktree,
    /// The worktree, the staged side of it.
    Staged,
    /// A file git does not track yet: it has no past.
    Untracked,
    /// A commit that has the file.
    Commit(Oid),
    /// A commit that deleted the file: it is in the commit before, which
    /// is the one named here when there is one.
    DeletedBy {
        /// The commit that deleted it.
        commit: Oid,
        /// The commit before, which still has it.
        before: Option<Oid>,
    },
}

/// What a file's menu offers, as labels with what each does; `None` is a
/// line between groups. Kept apart from the menu so that it can be read
/// without a window.
///
/// In groups: the file's past (history, blame); the ways to open it and
/// where it is (the diff tool, the editor, the system's program, the file
/// manager); its path; and for a file of a commit, putting it back as the
/// commit has it.
pub fn offered(path: &str, listed: Listed) -> Vec<Option<(&'static str, FileAction)>> {
    let path = || path.to_owned();
    let mut out = Vec::new();
    match listed {
        Listed::Untracked => {}
        Listed::Worktree | Listed::Staged => {
            out.push(Some((
                say!("file-actions-history"),
                FileAction::History {
                    path: path(),
                    rev: None,
                },
            )));
            out.push(Some((
                say!("file-actions-blame"),
                FileAction::Blame {
                    path: path(),
                    rev: None,
                },
            )));
        }
        Listed::Commit(rev) => {
            out.push(Some((
                say!("file-actions-history"),
                FileAction::History {
                    path: path(),
                    rev: Some(rev),
                },
            )));
            out.push(Some((
                say!("file-actions-blame-at"),
                FileAction::Blame {
                    path: path(),
                    rev: Some(rev),
                },
            )));
        }
        Listed::DeletedBy { commit, before } => {
            out.push(Some((
                say!("file-actions-history"),
                FileAction::History {
                    path: path(),
                    rev: Some(commit),
                },
            )));
            if before.is_some() {
                out.push(Some((
                    say!("file-actions-blame-before"),
                    FileAction::Blame {
                        path: path(),
                        rev: before,
                    },
                )));
            }
        }
    }
    let mut open = Vec::new();
    let diff_tool = |staged, commit| FileAction::DiffTool {
        path: path(),
        staged,
        commit,
    };
    match listed {
        Listed::Worktree | Listed::Untracked => {
            open.push(Some((
                say!("file-actions-diff-tool"),
                diff_tool(false, None),
            )));
            open.push(Some((
                say!("file-actions-edit"),
                FileAction::Edit { path: path() },
            )));
            open.push(Some((
                say!("file-actions-open-default"),
                FileAction::OpenDefault { path: path() },
            )));
        }
        Listed::Staged => {
            open.push(Some((
                say!("file-actions-diff-tool"),
                diff_tool(true, None),
            )));
            open.push(Some((
                say!("file-actions-edit"),
                FileAction::Edit { path: path() },
            )));
            open.push(Some((
                say!("file-actions-open-default"),
                FileAction::OpenDefault { path: path() },
            )));
        }
        Listed::Commit(rev) => {
            open.push(Some((
                say!("file-actions-diff-tool"),
                diff_tool(false, Some(rev)),
            )));
            open.push(Some((
                say!("file-actions-edit-at"),
                FileAction::EditAt { path: path(), rev },
            )));
            open.push(Some((
                say!("file-actions-edit-now"),
                FileAction::Edit { path: path() },
            )));
            open.push(Some((
                say!("file-actions-open-default"),
                FileAction::OpenDefault { path: path() },
            )));
        }
        Listed::DeletedBy { commit, before } => {
            open.push(Some((
                say!("file-actions-diff-tool"),
                diff_tool(false, Some(commit)),
            )));
            if let Some(rev) = before {
                open.push(Some((
                    say!("file-actions-edit-before"),
                    FileAction::EditAt { path: path(), rev },
                )));
            }
        }
    }
    // A file a commit deleted has no place to be shown at.
    if !matches!(listed, Listed::DeletedBy { .. }) {
        open.push(Some((
            say!("file-actions-reveal"),
            FileAction::Reveal { path: path() },
        )));
    }
    if !out.is_empty() {
        out.push(None);
    }
    out.append(&mut open);
    out.push(None);
    out.push(Some((
        say!("file-actions-copy-path"),
        FileAction::CopyPath { path: path() },
    )));
    // A file of a commit can be put back as the commit has it; one the
    // commit deleted, as it was before.
    let restore = match listed {
        Listed::Commit(rev) => Some((say!("file-actions-restore"), rev)),
        Listed::DeletedBy {
            before: Some(rev), ..
        } => Some((say!("file-actions-restore-before"), rev)),
        _ => None,
    };
    if let Some((label, rev)) = restore {
        out.push(None);
        out.push(Some((label, FileAction::Restore { path: path(), rev })));
    }
    out
}

/// What is offered for the place a file was moved from: the file as it
/// was there in the commit before, its history under that name and its
/// blame. Nothing without a commit before.
pub fn offered_for_old_path(old_path: &str, before: Option<Oid>) -> Vec<(String, FileAction)> {
    let Some(rev) = before else {
        return Vec::new();
    };
    let name = old_path.rsplit('/').next().unwrap_or(old_path);
    let path = || old_path.to_owned();
    vec![
        (
            say!("file-actions-old-edit", name),
            FileAction::EditAt { path: path(), rev },
        ),
        (
            say!("file-actions-old-history", old_path),
            FileAction::History {
                path: path(),
                rev: Some(rev),
            },
        ),
        (
            say!("file-actions-old-blame", name),
            FileAction::Blame {
                path: path(),
                rev: Some(rev),
            },
        ),
    ]
}

/// Add the entries for the place a file was moved from to a menu.
pub fn old_path_entries(menu: PopupMenu, old_path: &str, before: Option<Oid>) -> PopupMenu {
    let offered = offered_for_old_path(old_path, before);
    if offered.is_empty() {
        return menu;
    }
    let mut menu = menu.separator();
    for (label, action) in offered {
        menu = menu.menu(label, Box::new(RunFileAction { action }));
    }
    menu
}

/// Add the entries for a file to a menu.
pub fn entries(menu: PopupMenu, path: &str, listed: Listed) -> PopupMenu {
    let mut menu = menu;
    for entry in offered(path, listed) {
        menu = match entry {
            Some((label, action)) => menu.menu(label, Box::new(RunFileAction { action })),
            None => menu.separator(),
        };
    }
    menu
}

#[cfg(test)]
mod tests {
    use bitbraid_git::Oid;

    use super::{FileAction, Listed, offered, offered_for_old_path};

    #[test]
    fn the_place_a_file_was_moved_from_is_reached_in_the_commit_before() {
        let before = oid(0xcd);
        let offered = offered_for_old_path("old/dir/name.rs", Some(before));
        let labels: Vec<&str> = offered.iter().map(|(label, _)| label.as_str()).collect();
        assert_eq!(
            labels,
            [
                "Open name.rs as it was before the move",
                "History of old/dir/name.rs",
                "Blame name.rs before the move"
            ]
        );
        for (_, action) in &offered {
            let (path, rev) = match action {
                FileAction::EditAt { path, rev } => (path, Some(*rev)),
                FileAction::History { path, rev } | FileAction::Blame { path, rev } => (path, *rev),
                other => panic!("{other:?}"),
            };
            assert_eq!((path.as_str(), rev), ("old/dir/name.rs", Some(before)));
        }
        // A first commit moved nothing from anywhere.
        assert!(offered_for_old_path("old.rs", None).is_empty());
    }

    fn labels(path: &str, listed: Listed) -> Vec<&'static str> {
        offered(path, listed)
            .into_iter()
            .map(|entry| entry.map_or("-", |(label, _)| label))
            .collect()
    }

    fn oid(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    #[test]
    fn a_file_of_the_worktree_is_opened_as_it_is() {
        assert_eq!(
            labels("a/b.rs", Listed::Worktree),
            [
                "File history",
                "Blame",
                "-",
                "Open in diff tool",
                "Open in editor",
                "Open with default app",
                "Show in file manager",
                "-",
                "Copy path"
            ]
        );
        let worktree = offered("a/b.rs", Listed::Worktree);
        assert_eq!(
            worktree.get(1).cloned().flatten().map(|(_, action)| action),
            Some(FileAction::Blame {
                path: "a/b.rs".into(),
                rev: None
            })
        );
        // Staged, the diff tool shows what is staged; not tracked, there
        // is no past to show.
        assert!(
            offered("a/b.rs", Listed::Staged)
                .iter()
                .flatten()
                .any(|(_, action)| *action
                    == FileAction::DiffTool {
                        path: "a/b.rs".into(),
                        staged: true,
                        commit: None
                    })
        );
        assert_eq!(
            labels("new.txt", Listed::Untracked),
            [
                "Open in diff tool",
                "Open in editor",
                "Open with default app",
                "Show in file manager",
                "-",
                "Copy path"
            ]
        );
    }

    #[test]
    fn a_file_of_a_commit_is_also_opened_as_it_was_there_and_put_back() {
        let commit = oid(0xab);
        assert_eq!(
            labels("a/b.rs", Listed::Commit(commit)),
            [
                "File history",
                "Blame as of this commit",
                "-",
                "Open in diff tool",
                "Open as of this commit",
                "Open the file as it is now",
                "Open with default app",
                "Show in file manager",
                "-",
                "Copy path",
                "-",
                "Restore this version"
            ]
        );
        let all = offered("a/b.rs", Listed::Commit(commit));
        assert!(all.iter().flatten().any(|(_, action)| *action
            == FileAction::EditAt {
                path: "a/b.rs".into(),
                rev: commit
            }));
        assert!(all.iter().flatten().any(|(_, action)| *action
            == FileAction::Restore {
                path: "a/b.rs".into(),
                rev: commit
            }));
    }

    #[test]
    fn a_file_a_commit_deleted_is_opened_and_blamed_as_it_was_before() {
        let (commit, before) = (oid(0xab), oid(0xcd));
        let listed = Listed::DeletedBy {
            commit,
            before: Some(before),
        };
        assert_eq!(
            labels("gone.rs", listed),
            [
                "File history",
                "Blame as it was before this commit",
                "-",
                "Open in diff tool",
                "Open as it was before this commit",
                "-",
                "Copy path",
                "-",
                "Restore as it was before"
            ]
        );
        let all = offered("gone.rs", listed);
        assert!(all.iter().flatten().any(|(_, action)| *action
            == FileAction::EditAt {
                path: "gone.rs".into(),
                rev: before
            }));
        // History starts at the commit that deleted it, which lists it.
        assert!(all.iter().flatten().any(|(_, action)| *action
            == FileAction::History {
                path: "gone.rs".into(),
                rev: Some(commit)
            }));
        // A first commit deleted nothing; were it to, there is no before.
        let rootless = Listed::DeletedBy {
            commit,
            before: None,
        };
        assert_eq!(
            labels("gone.rs", rootless),
            ["File history", "-", "Open in diff tool", "-", "Copy path"]
        );
    }
}
