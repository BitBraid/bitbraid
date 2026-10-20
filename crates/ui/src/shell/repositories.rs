//! The window's side of the page of repositories: its pinned tab, the first
//! at the left end of the tab bar, shows it, as its key and the palette do,
//! and what is done on it is done here: a repository opened, browsed for,
//! cloned or made, a tab closed, a workspace's repositories opened with its
//! defaults. The page goes away when a tab is chosen, a new one is made or
//! a repository is opened, and with Escape.
//!
//! Sources: none (original design).

use gpui_kit::component::IconName;
use gpui_kit::*;

use std::path::PathBuf;

use bitbraid_engine::workspace::Layout;

use super::{Shell, TabKind, WindowSheet};
use crate::{
    prefs::{Prefs, RepoState},
    views::repositories::{RepositoriesEvent, RepositoriesView, Section},
};

impl Shell {
    /// Show the page in place of the tab's content. Asked for again while
    /// it is on show, it stays as it is, as a shown tab does.
    pub(super) fn show_repositories(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sheet != Some(WindowSheet::Repositories) {
            self.leave_inbox(cx);
            let page = self.repositories_page(window, cx);
            page.update(cx, |page, cx| page.show(window, cx));
            self.sheet = Some(WindowSheet::Repositories);
        }
        self.take_focus(window, cx);
        cx.notify();
    }

    /// The page's tab, the first of the pinned tabs at the left end of the
    /// tab bar (`super::pinned`): its mark alone. Shown as the shown tab
    /// while the page is on show.
    pub(super) fn render_repositories_tab(&self, cx: &mut Context<Self>) -> AnyElement {
        let weak = cx.entity().downgrade();
        super::pinned::pinned_tab(
            "repositories-tab",
            "repositories-tab-mark",
            IconName::FolderOpen,
            say!("shell-repositories-tab").into(),
            self.sheet == Some(WindowSheet::Repositories),
            true,
            None,
            move |_, window, cx| {
                let _ = weak.update(cx, |this, cx| this.show_repositories(window, cx));
            },
            cx,
        )
    }

    /// The page, once it was shown: what the off-screen tests ask about
    /// and act on.
    pub fn repositories_view(&self) -> Option<Entity<RepositoriesView>> {
        self.repositories.clone()
    }

    /// Put the page away, if it is shown: a tab is to be looked at.
    pub(super) fn leave_repositories(&mut self) {
        if self.sheet == Some(WindowSheet::Repositories) {
            self.sheet = None;
        }
    }

    /// The page, made the first time it is asked for.
    pub(super) fn repositories_page(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<RepositoriesView> {
        if let Some(page) = &self.repositories {
            return page.clone();
        }
        let page = cx.new(|cx| RepositoriesView::new(window, cx));
        let subscription = cx.subscribe_in(
            &page,
            window,
            |this, _, event: &RepositoriesEvent, window, cx| {
                this.on_repositories(event, window, cx)
            },
        );
        self._subscriptions.push(subscription);
        self.repositories = Some(page.clone());
        page
    }

    fn on_repositories(
        &mut self,
        event: &RepositoriesEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            RepositoriesEvent::Open(path) => {
                self.open_repository(path.clone(), window, cx);
                self.take_focus(window, cx);
            }
            RepositoriesEvent::OpenMany { paths, workspace } => {
                self.open_with_defaults(paths, workspace, window, cx);
                self.take_focus(window, cx);
            }
            RepositoriesEvent::Browse => self.browse_for_repository(window, cx),
            RepositoriesEvent::Clone => self.prompt_clone(window, cx),
            RepositoriesEvent::Init => self.prompt_init(window, cx),
            RepositoriesEvent::Close(path) => {
                let tab = self.tabs.iter().position(|tab| {
                    matches!(tab.kind, TabKind::Repo(_)) && tab.path.as_deref() == Some(path)
                });
                if let Some(ix) = tab {
                    self.close_tab(ix, window, cx);
                }
            }
        }
        cx.notify();
    }

    /// Show the page with a workspace's section unfolded and in sight:
    /// what a workspace on the start page leads to.
    pub(super) fn reveal_workspace(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.show_repositories(window, cx);
        let section = Section::Workspace(id.to_owned());
        if let Some(page) = &self.repositories {
            page.update(cx, |page, cx| page.reveal(&section, cx));
        }
    }

    /// Open repositories of a workspace, each in a tab, with its defaults.
    /// A repository the application has not opened before takes them
    /// (`workspace.defaults`): it commits as the workspace's profile unless
    /// it does already, and its tab starts from the workspace's layout. One
    /// that was opened before is as it was left, and opens at once.
    fn open_with_defaults(
        &mut self,
        paths: &[PathBuf],
        workspace: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let repos = &cx.global::<Prefs>().state.repos;
        let (fresh, known): (Vec<PathBuf>, Vec<PathBuf>) = paths
            .iter()
            .cloned()
            .partition(|path| !repos.contains_key(&path.to_string_lossy().into_owned()));
        for path in known {
            self.open_repository(path, window, cx);
        }
        for path in fresh {
            let run = crate::session::run_standalone(
                "workspace.defaults",
                serde_json::json!({ "id": workspace, "path": path.display().to_string() }),
            );
            cx.spawn_in(window, async move |this, cx| {
                let reply = run.reply.await;
                let _ = this.update_in(cx, |this, window, cx| {
                    if let Ok(crate::session::Reply::Command(value)) = &reply {
                        this.take_defaults(&path, value, window, cx);
                    }
                    this.open_repository(path.clone(), window, cx);
                });
            })
            .detach();
        }
    }

    /// Take what `workspace.defaults` answered for a repository about to
    /// be opened: its layout to start from, and what became of the
    /// profile, said when it could not be written.
    fn take_defaults(
        &mut self,
        path: &std::path::Path,
        value: &serde_json::Value,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let layout = value
            .get("layout")
            .cloned()
            .and_then(|layout| serde_json::from_value::<Layout>(layout).ok())
            .filter(|layout| !layout.is_empty());
        if let Some(layout) = layout {
            let prefs = cx.global_mut::<Prefs>();
            let key = path.to_string_lossy().into_owned();
            if let std::collections::hash_map::Entry::Vacant(entry) = prefs.state.repos.entry(key) {
                entry.insert(state_of(&layout));
                prefs.save_state();
            }
        }
        let error = value
            .pointer("/profile/error")
            .and_then(serde_json::Value::as_str)
            .filter(|error| !error.is_empty());
        if let Some(error) = error {
            crate::notices::push(
                window,
                cx,
                crate::notices::Notice::warning(say!(
                    "shell-workspace-profile-not-taken",
                    repository = crate::shell::repo_title(path),
                    error
                )),
            );
        }
    }
}

/// A workspace's layout as a repository's remembered one starts.
fn state_of(layout: &Layout) -> RepoState {
    RepoState {
        left: layout.left_width,
        right: layout.right_width,
        left_panel: layout.left_panel,
        right_panel: layout.right_panel,
        closed_sections: layout.closed_sections.clone(),
        hidden_sections: layout.hidden_sections.clone(),
        section_order: layout.section_order.clone(),
        section_sizes: layout.section_sizes.clone(),
        maximised_section: layout.maximised_section.clone(),
        ..RepoState::default()
    }
}
