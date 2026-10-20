//! What the workspace does with a file that was picked from a list: start
//! the editor or the file manager on it, copy its path, or bring the file
//! pane to its history or its blame.
//!
//! Sources: `crates/ui/src/views/file_actions.rs`.

use bitbraid_git::Oid;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{FileTab, RightPane, Workspace, sentence};
use crate::{
    notices::Notice,
    prefs::Prefs,
    session::{Reply, Request},
    views::{changes::Side, diff::DiffSource, file_actions::FileAction},
};

impl Workspace {
    /// Wrap the long lines of the diff, or stop wrapping them (from a key
    /// or the palette, wherever the focus is).
    pub fn toggle_diff_wrap(&mut self, cx: &mut Context<Self>) {
        self.diff_view.update(cx, |view, cx| view.toggle_wrap(cx));
    }

    /// List what differs between the commit that is selected and the
    /// working tree, in the details panel (from the palette).
    pub fn compare_with_worktree(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let has_commit = self.details_view.read(cx).has_commit();
        if self.right != RightPane::Commit || !has_commit {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("workspace-compare-select-one")),
            );
            return;
        }
        self.details_view
            .update(cx, |view, cx| view.toggle_against_worktree(cx));
    }

    /// The same for a commit that is named (the one a branch is at, from
    /// the menu of its row): it is selected in the graph first, and
    /// compared once the details panel shows it, which it does when the
    /// graph's word of the selection has arrived.
    pub(super) fn compare_commit_with_worktree(
        &mut self,
        id: Oid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let found = self
            .graph_view
            .update(cx, |view, cx| view.reveal(id, window, cx));
        if !found {
            crate::notices::push(
                window,
                cx,
                Notice::info(say!("workspace-not-in-graph", id = id.short(10))),
            );
            return;
        }
        cx.defer_in(window, |this, window, cx| {
            this.compare_with_worktree(window, cx)
        });
    }

    /// Open a submodule in a tab of its own and show one of its commits
    /// there: its checkout when it has one, else the clone git keeps for
    /// it, which opens read-only.
    pub(super) fn open_submodule_commit(
        &mut self,
        path: &str,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let workdir = self
            .session
            .summary
            .workdir
            .clone()
            .unwrap_or_else(|| self.session.path.clone());
        let found = submodule_folder(&workdir, &self.session.summary.git_dir, path, |folder| {
            folder.exists()
        });
        match found {
            Some(folder) => cx.emit(super::WorkspaceEvent::OpenAt {
                path: folder,
                commit: id.to_owned(),
            }),
            None => crate::notices::push(
                window,
                cx,
                Notice::info(say!("workspace-submodule-not-cloned", path)),
            ),
        }
    }

    /// Carry out what was picked from a file's menu.
    pub fn file_action(&mut self, action: FileAction, window: &mut Window, cx: &mut Context<Self>) {
        let preferences = cx.global::<Prefs>().preferences.clone();
        match action {
            FileAction::Edit { path } => self.start(
                "file.open",
                json!({ "path": path, "command": preferences.editor }),
                window,
                cx,
            ),
            FileAction::EditAt { path, rev } => self.start(
                "file.open",
                json!({ "path": path, "rev": rev.to_hex(), "command": preferences.editor }),
                window,
                cx,
            ),
            FileAction::Reveal { path } => self.start(
                "file.reveal",
                json!({ "path": path, "command": preferences.file_manager }),
                window,
                cx,
            ),
            FileAction::DiffTool {
                path,
                staged,
                commit,
            } => self.start(
                "tool.diff",
                json!({
                    "path": path,
                    "staged": staged,
                    "commit": commit.map(|id| id.to_hex()),
                    "command": preferences.external_diff,
                }),
                window,
                cx,
            ),
            FileAction::OpenDefault { path } => {
                self.start("file.open", json!({ "path": path }), window, cx)
            }
            FileAction::Restore { path, rev } => self.run_mutation(
                "file.restore",
                json!({ "path": path, "rev": rev.to_hex() }),
                say!(
                    "workspace-file-restored",
                    path = path.as_str(),
                    commit = rev.short(7)
                ),
                |_, _, _, _| false,
                window,
                cx,
            ),
            FileAction::MergeTool { path } => self.run_operation(
                "tool.merge",
                json!({ "path": path, "command": preferences.external_merge }),
                say!("workspace-merge-tool-waiting"),
                say!("workspace-merge-tool-ended").to_owned(),
                window,
                cx,
            ),
            FileAction::OpenFolder { path } => self.start(
                "repo.open_folder",
                json!({ "path": path, "command": preferences.file_manager }),
                window,
                cx,
            ),
            FileAction::Terminal { path } => {
                let root = self
                    .session
                    .summary
                    .workdir
                    .clone()
                    .unwrap_or_else(|| self.session.path.clone());
                self.start_terminal(Some(root.join(path)), window, cx)
            }
            FileAction::CopyPath { path } => {
                cx.write_to_clipboard(ClipboardItem::new_string(path));
            }
            FileAction::SparseOnly { path } => self.run_sparse(
                "sparse.set",
                json!({ "directories": [path] }),
                say!("workspace-sparse-only-running", path = path.as_str()),
                say!("workspace-sparse-only-done", path = path.as_str()),
                window,
                cx,
            ),
            FileAction::SparseAdd { path } => self.run_sparse(
                "sparse.add",
                json!({ "directories": [path] }),
                say!("workspace-sparse-add-running", path = path.as_str()),
                say!("workspace-sparse-add-done", path = path.as_str()),
                window,
                cx,
            ),

            FileAction::History { path, rev } => {
                self.show_past_of(path, rev, FileTab::History, window, cx)
            }
            FileAction::Blame { path, rev } => {
                self.show_past_of(path, rev, FileTab::Blame, window, cx)
            }
        }
    }

    /// A file's history or its blame, picked from a list: the view of its
    /// past, with the keyboard in the list of commits. Closing it goes
    /// back to the file when the pane was already about it, else to the
    /// graph the list was beside.
    fn show_past_of(
        &mut self,
        path: String,
        rev: Option<Oid>,
        tab: FileTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let was_open = self.show_diff && self.file.as_ref() == Some(&(path.clone(), rev));
        self.show_file(path, rev, cx);
        self.set_file_tab(tab, cx);
        if !was_open {
            self.past_back = None;
        }
        self.history_view
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
    }

    /// Run a command that starts a program outside and changes nothing
    /// here; what goes wrong is said.
    fn start(&mut self, id: &str, params: Value, window: &mut Window, cx: &mut Context<Self>) {
        // The user asked for it: a version of a file from a partial clone
        // is fetched for the program that shows it.
        let reply = self.session.ask(Request::Command {
            id: id.to_owned(),
            params,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Reply::Error(err)) = reply.await {
                let _ = this.update_in(cx, |_, window, cx| {
                    crate::notices::push(window, cx, Notice::error(sentence(&err)));
                });
            }
        })
        .detach();
    }

    /// Make the file pane be about this file, unless it is already: a
    /// file of a commit as that commit changed it, a file of the worktree
    /// from the side it is changed on.
    fn show_file(&mut self, path: String, rev: Option<Oid>, cx: &mut Context<Self>) {
        if self.file.as_ref() == Some(&(path.clone(), rev)) && self.show_diff {
            return;
        }
        let source = match rev {
            Some(id) => DiffSource::Revision {
                id,
                path: path.clone(),
                old_path: None,
            },
            None => DiffSource::Working {
                path: path.clone(),
                side: worktree_side(&self.status, &path),
            },
        };
        self.open_file(path, rev, source, cx);
    }
}

/// Where the repository of a submodule is: its checkout in the worktree
/// when that is one (it has a `.git`), else the clone git keeps under
/// `modules` in the repository's own directory. `exists` says whether a
/// place is there.
pub fn submodule_folder(
    workdir: &std::path::Path,
    git_dir: &std::path::Path,
    path: &str,
    exists: impl Fn(&std::path::Path) -> bool,
) -> Option<std::path::PathBuf> {
    // A path of the repository, not a way out of it.
    if path.is_empty()
        || path.starts_with('/')
        || path.split('/').any(|part| part == ".." || part.is_empty())
        || !bitbraid_core::paths::stays_inside(path)
    {
        return None;
    }
    let checkout = workdir.join(path);
    if exists(&checkout.join(".git")) {
        return Some(checkout);
    }
    let kept = git_dir.join("modules").join(path);
    exists(&kept.join("HEAD")).then_some(kept)
}

/// The side of the staging area a worktree file is shown from: where it
/// has changes that are not staged, that side; the staged side otherwise.
pub fn worktree_side(status: &[bitbraid_git::StatusEntry], path: &str) -> Side {
    match status.iter().find(|entry| entry.path == path) {
        Some(entry) if entry.unstaged.is_none() && entry.staged.is_some() => Side::Staged,
        _ => Side::Unstaged,
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::{ChangeKind, StatusEntry};

    use std::path::{Path, PathBuf};

    use super::{submodule_folder, worktree_side};
    use crate::views::changes::Side;

    #[test]
    fn a_submodule_is_opened_where_its_repository_is() {
        let (workdir, git_dir) = (Path::new("/r"), Path::new("/r/.git"));
        let there = |places: &'static [&'static str]| {
            move |place: &Path| places.iter().any(|known| Path::new(known) == place)
        };
        // Checked out: the folder in the worktree.
        assert_eq!(
            submodule_folder(workdir, git_dir, "deps/lib", there(&["/r/deps/lib/.git"])),
            Some(PathBuf::from("/r/deps/lib"))
        );
        // Not checked out: the clone git keeps.
        assert_eq!(
            submodule_folder(
                workdir,
                git_dir,
                "deps/lib",
                there(&["/r/.git/modules/deps/lib/HEAD"])
            ),
            Some(PathBuf::from("/r/.git/modules/deps/lib"))
        );
        // Neither, and paths that are no place in the repository.
        assert_eq!(
            submodule_folder(workdir, git_dir, "deps/lib", there(&[])),
            None
        );
        for hostile in ["", "/etc", "../outside", "a/../../b", "a//b"] {
            assert_eq!(
                submodule_folder(workdir, git_dir, hostile, |_| true),
                None,
                "{hostile:?}"
            );
        }
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

    #[test]
    fn a_worktree_file_is_shown_from_the_side_it_is_changed_on() {
        let status = [
            entry("both.rs", true, true),
            entry("staged.rs", true, false),
            entry("unstaged.rs", false, true),
        ];
        assert_eq!(worktree_side(&status, "both.rs"), Side::Unstaged);
        assert_eq!(worktree_side(&status, "staged.rs"), Side::Staged);
        assert_eq!(worktree_side(&status, "unstaged.rs"), Side::Unstaged);
        // A file without changes has nothing on either side.
        assert_eq!(worktree_side(&status, "clean.rs"), Side::Unstaged);
    }
}
