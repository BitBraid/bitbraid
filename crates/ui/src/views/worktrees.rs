//! Worktree requests: the menus of the references panel and of the graph
//! make them, the workspace asks where it has to and runs them.
//!
//! Sources: gpui-kit documentation (popup menu).

use std::path::{Path, PathBuf};

use gpui_kit::component::menu::PopupMenu;
use gpui_kit::*;
use serde::Deserialize;

/// One request.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub enum WorktreeRequest {
    /// Open the pane that adds a worktree. With a starting point the
    /// worktree checks that out, or a new branch that begins there; without
    /// one it gets a new branch at HEAD.
    New(Option<String>),
    /// Open the worktree at this path in a tab of its own.
    Open(String),
    /// Open the worktree at this path in the tab that is shown, in place
    /// of the worktree that is open there.
    OpenHere(String),
    /// Start a terminal in the worktree at this path.
    Terminal(String),
    /// Ask for a reason and lock the worktree at this path.
    Lock(String),
    /// Unlock the worktree at this path.
    Unlock(String),
    /// Ask for a folder and move the worktree at this path there.
    Move(String),
    /// Ask, then remove the worktree at this path; with its branch when
    /// that is set.
    Remove {
        /// Where it is.
        path: String,
        /// Delete the branch it has checked out too.
        with_branch: bool,
    },
    /// Forget the worktrees whose folder is gone.
    Prune,
}

/// The action a menu entry dispatches.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = worktree, no_json)]
pub struct RunWorktree {
    /// What to do.
    pub request: WorktreeRequest,
}

/// What is said of the changes counted in another worktree: they are of
/// its tracked files alone, since it is looked at without the search for
/// untracked files (`bitbraid_git::wip::LOOK`).
pub const TRACKED_ONLY: bitbraid_strings::Message = message!("worktrees-tracked-only");

fn entry(request: WorktreeRequest) -> Box<dyn Action> {
    Box::new(RunWorktree { request })
}

/// The last part of a path: what a worktree is called in a list.
pub fn folder_name(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_owned())
}

/// The entry that adds a worktree for `start`: a branch, or a commit.
pub fn new_worktree_entry(menu: PopupMenu, start: Option<&str>) -> PopupMenu {
    menu.menu(
        match start {
            Some(_) => say!("worktrees-new-here"),
            None => say!("worktrees-new"),
        },
        entry(WorktreeRequest::New(start.map(str::to_owned))),
    )
}

/// The entry that opens a worktree, by the name of its folder.
pub fn open_worktree_entry(menu: PopupMenu, path: &str) -> PopupMenu {
    menu.menu(
        say!("worktrees-open-named", name = folder_name(path)),
        entry(WorktreeRequest::Open(path.to_owned())),
    )
}

/// What can be done with one worktree, and why not when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WorktreeRow {
    /// The main worktree.
    pub is_main: bool,
    /// The worktree this tab shows.
    pub current: bool,
    pub locked: bool,
    /// It has a branch checked out.
    pub on_branch: bool,
}

/// Menu entries for one worktree, in groups: open it (here, or in a new
/// tab) and remove it (with its branch or not); lock or unlock it; then a
/// terminal there, moving it, a new worktree, forgetting those that are
/// gone. The one this tab shows has nothing of the first two groups, and
/// no moving.
pub fn worktree_entries(menu: PopupMenu, path: &str, row: WorktreeRow) -> PopupMenu {
    let at = || path.to_owned();
    let remove = |with_branch| {
        entry(WorktreeRequest::Remove {
            path: at(),
            with_branch,
        })
    };
    let fixed = if row.is_main {
        Some(say!("worktrees-why-main"))
    } else if row.locked {
        Some(say!("worktrees-why-locked"))
    } else {
        None
    };
    let menu = if row.current {
        menu
    } else {
        let menu = menu
            .menu(
                say!("worktrees-open-here"),
                entry(WorktreeRequest::OpenHere(at())),
            )
            .menu(
                say!("worktrees-open-in-tab"),
                entry(WorktreeRequest::Open(at())),
            )
            .menu_with_disabled(
                match fixed {
                    Some(why) => say!("worktrees-remove-not", why),
                    None => say!("worktrees-remove").to_owned(),
                },
                remove(false),
                fixed.is_some(),
            );
        let menu = if row.on_branch && fixed.is_none() {
            menu.menu(say!("worktrees-remove-with-branch"), remove(true))
        } else {
            menu
        };
        let menu = menu.separator();
        let menu = if row.locked {
            menu.menu(
                say!("worktrees-unlock"),
                entry(WorktreeRequest::Unlock(at())),
            )
        } else {
            menu.menu_with_disabled(
                if row.is_main {
                    say!("worktrees-lock-main")
                } else {
                    say!("worktrees-lock")
                },
                entry(WorktreeRequest::Lock(at())),
                row.is_main,
            )
        };
        menu.separator()
    };
    let menu = menu.menu(
        say!("worktrees-terminal"),
        entry(WorktreeRequest::Terminal(at())),
    );
    // What keeps a worktree from being removed keeps it where it is.
    let menu = if row.current {
        menu
    } else {
        menu.menu_with_disabled(
            match fixed {
                Some(why) => say!("worktrees-move-not", why),
                None => say!("worktrees-move").to_owned(),
            },
            entry(WorktreeRequest::Move(at())),
            fixed.is_some(),
        )
    };
    menu.separator()
        .menu(say!("worktrees-new"), entry(WorktreeRequest::New(None)))
        .menu(say!("worktrees-prune"), entry(WorktreeRequest::Prune))
}

/// The main worktree of a linked worktree, told from where the linked
/// one's git directory is (`<main>/.git/worktrees/<id>`). None for a
/// repository that is not a linked worktree, or whose main one is bare.
pub fn main_worktree_of(git_dir: &Path) -> Option<PathBuf> {
    let worktrees = git_dir.parent()?;
    let common = worktrees.parent()?;
    let named = |dir: &Path, name: &str| dir.file_name().is_some_and(|found| found == name);
    (named(worktrees, "worktrees") && named(common, ".git"))
        .then(|| common.parent().map(Path::to_path_buf))
        .flatten()
}

/// The folder a new worktree is suggested at: beside the repository, named
/// after it and after what is checked out there.
pub fn suggested_folder(repository: &Path, start: Option<&str>) -> String {
    let name = repository
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "worktree".to_owned());
    // The last part of a branch name; a commit id cut short.
    let tail = start
        .map(|start| {
            let hex = start.len() == 40 && start.chars().all(|c| c.is_ascii_hexdigit());
            let last = start.rsplit('/').next().unwrap_or(start);
            if hex {
                start.chars().take(7).collect::<String>()
            } else {
                last.chars()
                    .map(|c| {
                        if c.is_alphanumeric() || "-_.".contains(c) {
                            c
                        } else {
                            '-'
                        }
                    })
                    .collect()
            }
        })
        .filter(|tail| !tail.is_empty())
        .unwrap_or_else(|| "worktree".to_owned());
    let folder = format!("{name}-{tail}");
    match repository.parent() {
        Some(parent) => parent.join(folder).display().to_string(),
        None => folder,
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::path::Path;

    use super::{folder_name, main_worktree_of, suggested_folder};

    #[test]
    fn the_main_worktree_is_told_from_a_linked_ones_git_directory() {
        assert_eq!(
            main_worktree_of(Path::new("/work/project/.git/worktrees/topic")),
            Some("/work/project".into())
        );
        for not_linked in [
            "/work/project/.git",
            "/work/bare.git/worktrees/topic",
            "/work/project/.git/modules/lib",
            "/",
            "",
        ] {
            assert_eq!(
                main_worktree_of(Path::new(not_linked)),
                None,
                "{not_linked}"
            );
        }
    }

    #[test]
    fn a_new_worktree_is_suggested_beside_the_repository() {
        let repo = Path::new("/work/project");
        // Beside it, with the separator of the system.
        let beside = |name: &str| Path::new("/work").join(name).display().to_string();
        assert_eq!(suggested_folder(repo, None), beside("project-worktree"));
        assert_eq!(
            suggested_folder(repo, Some("feature/log in")),
            beside("project-log-in")
        );
        assert_eq!(
            suggested_folder(repo, Some("origin/topic")),
            beside("project-topic")
        );
        assert_eq!(
            suggested_folder(repo, Some("0123456789abcdef0123456789abcdef01234567")),
            beside("project-0123456")
        );
        assert_eq!(suggested_folder(Path::new("/"), Some("x")), "worktree-x");
        assert_eq!(folder_name("/work/project-topic"), "project-topic");
        assert_eq!(folder_name("/"), "/");
    }
}
