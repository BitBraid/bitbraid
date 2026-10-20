//! The two controls at the toolbar's left: the repository and the branch.
//!
//! Each is a block of its own width, as high as the toolbar: a small
//! caption over the name and a caret; a long name is cut inside its block,
//! so the branch's block never moves. A click drops a panel 250 wide from
//! the toolbar's bottom edge, under the block: a field to find one by
//! typing (it has the keys), then the list, a row each. The repository's
//! lists the recent repositories but the one on show (a click opens it, or
//! goes to its tab), then what ours has besides (opening one, the ways out
//! to other programs, and its maintenance); the branch's lists the local
//! branches flat, the current one first and filled (a click checks out).
//! Enter takes the first row found.
//!
//! Sources: none (original design).

use std::path::PathBuf;

use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputEvent, InputState},
    popover::Popover,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use super::{Shell, TOOLBAR_HEIGHT, repo_title};
use crate::{prefs::Prefs, views::workspace::Outside};

/// From a block's left edge to its caption and name.
const BLOCK_INSET: f32 = 20.0;
/// Width of the panel a block drops.
const PANEL_WIDTH: f32 = 250.0;
/// Height of the field at the panel's top.
const FIELD_HEIGHT: f32 = 34.0;
/// Height of a repository's row.
const REPO_ROW: f32 = 25.0;
/// Height of a branch's row.
const BRANCH_ROW: f32 = 27.0;
/// How many recent repositories the panel lists at most.
const MOST_RECENT: usize = 30;

/// Which of the two panels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Which {
    Repository,
    Branch,
}

/// A panel's state: whether it is open, and what is typed in its field.
pub(super) struct Finder {
    pub(super) open: bool,
    pub(super) search: Entity<InputState>,
}

impl Finder {
    /// A closed panel; Enter in its field takes the first row found.
    pub(super) fn new(
        which: Which,
        placeholder: &'static str,
        subscriptions: &mut Vec<Subscription>,
        window: &mut Window,
        cx: &mut Context<Shell>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        subscriptions.push(cx.subscribe_in(
            &search,
            window,
            move |this, _, event: &InputEvent, window, cx| match event {
                InputEvent::PressEnter { .. } => this.take_first(which, window, cx),
                InputEvent::Change => cx.notify(),
                _ => {}
            },
        ));
        Self {
            open: false,
            search,
        }
    }

    /// What is typed, as it is looked for.
    fn typed(&self, cx: &App) -> String {
        self.search.read(cx).value().trim().to_lowercase()
    }
}

impl Shell {
    fn finder(&mut self, which: Which) -> &mut Finder {
        match which {
            Which::Repository => &mut self.repo_finder,
            Which::Branch => &mut self.branch_finder,
        }
    }

    /// Open or close a panel; it opens with nothing typed.
    pub(super) fn set_finder(
        &mut self,
        which: Which,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let finder = self.finder(which);
        if open && !finder.open {
            let search = finder.search.clone();
            search.update(cx, |state, cx| state.set_value("", window, cx));
        }
        self.finder(which).open = open;
        cx.notify();
    }

    /// The recent repositories whose name has what is typed, the one on
    /// show left out.
    fn repos_found(&self, cx: &App) -> Vec<PathBuf> {
        let typed = self.repo_finder.typed(cx);
        let shown = self.active_path();
        cx.global::<Prefs>()
            .state
            .recent
            .iter()
            .filter(|path| Some(path.as_path()) != shown.as_deref())
            .filter(|path| typed.is_empty() || repo_title(path).to_lowercase().contains(&typed))
            .take(MOST_RECENT)
            .cloned()
            .collect()
    }

    /// The local branches whose name has what is typed, the current one
    /// first, and whether each is the current one.
    fn branches_found(&self, cx: &App) -> Vec<(String, bool)> {
        let Some(workspace) = self.active_workspace() else {
            return Vec::new();
        };
        let workspace = workspace.read(cx);
        let current = workspace.head_label();
        let typed = self.branch_finder.typed(cx);
        let mut found: Vec<(String, bool)> = workspace
            .local_branch_names()
            .into_iter()
            .filter(|name| typed.is_empty() || name.to_lowercase().contains(&typed))
            .map(|name| {
                let is_current = name == current;
                (name, is_current)
            })
            .collect();
        found.sort_by_key(|(_, is_current)| !*is_current);
        found
    }

    /// Enter in a panel's field: the first row found.
    fn take_first(&mut self, which: Which, window: &mut Window, cx: &mut Context<Self>) {
        match which {
            Which::Repository => {
                if let Some(path) = self.repos_found(cx).into_iter().next() {
                    self.pick_repo(path, window, cx);
                }
            }
            Which::Branch => {
                if let Some((name, _)) = self.branches_found(cx).into_iter().next() {
                    self.pick_branch(name, window, cx);
                }
            }
        }
    }

    /// Open a repository from the panel, or go to its tab.
    fn pick_repo(&mut self, path: PathBuf, window: &mut Window, cx: &mut Context<Self>) {
        self.set_finder(Which::Repository, false, window, cx);
        self.open_repository(path, window, cx);
    }

    /// Check a branch out from the panel; the current one is left as it is.
    fn pick_branch(&mut self, name: String, window: &mut Window, cx: &mut Context<Self>) {
        self.set_finder(Which::Branch, false, window, cx);
        if self.active_read_only(cx) {
            return;
        }
        if let Some(workspace) = self.active_workspace().cloned() {
            let current = workspace.read(cx).head_label();
            if name != current {
                workspace.update(cx, |w, cx| w.checkout(name, window, cx));
            }
        }
    }

    /// The two blocks at the toolbar's left, with their panels, each
    /// `width` wide (narrower than their own in a narrow window).
    pub(super) fn render_crumbs(&self, width: f32, cx: &mut Context<Self>) -> AnyElement {
        let Some(workspace) = self.active_workspace() else {
            return div().into_any_element();
        };
        let (repo_name, branch_name) = {
            let workspace = workspace.read(cx);
            (workspace.repo_name(), workspace.head_label())
        };
        let read_only = self.active_read_only(cx);
        let weak = cx.entity().downgrade();
        let theme = cx.theme().clone();

        let repos = self.repos_found(cx);
        let repo_search = self.repo_finder.search.clone();
        let repo_weak = weak.clone();
        let repo = Popover::new("repo-finder")
            .anchor(Anchor::TopLeft)
            .offset(px(1.))
            .open(self.repo_finder.open)
            .track_focus(&self.repo_finder.search.focus_handle(cx))
            .on_open_change({
                let weak = weak.clone();
                move |open, window, cx| {
                    let _ = weak.update(cx, |shell, cx| {
                        shell.set_finder(Which::Repository, *open, window, cx)
                    });
                }
            })
            .trigger(block(
                "crumb-repo",
                say!("shell-finder-repository"),
                repo_name,
                true,
                width,
                cx,
            ))
            .p_0()
            .content(move |_, _, cx| {
                repo_panel(&repos, &repo_search, &repo_weak, cx).into_any_element()
            });

        let branches = self.branches_found(cx);
        let branch_search = self.branch_finder.search.clone();
        let branch_weak = weak.clone();
        let branch = Popover::new("branch-finder")
            .anchor(Anchor::TopLeft)
            .offset(px(1.))
            .open(self.branch_finder.open)
            .track_focus(&self.branch_finder.search.focus_handle(cx))
            .on_open_change(move |open, window, cx| {
                let _ = weak.update(cx, |shell, cx| {
                    shell.set_finder(Which::Branch, *open, window, cx)
                });
            })
            .trigger(block(
                "crumb-branch",
                say!("shell-finder-branch"),
                branch_name,
                false,
                width,
                cx,
            ))
            .p_0()
            .content(move |_, _, cx| {
                branch_panel(&branches, read_only, &branch_search, &branch_weak, cx)
                    .into_any_element()
            });

        h_flex()
            .flex_none()
            .h_full()
            .items_center()
            .child(repo)
            .child(
                div()
                    .w(px(super::toolbar_fit::CHEVRON_WIDTH))
                    .flex_none()
                    .flex()
                    .justify_center()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::ChevronRight).size_3p5()),
            )
            .child(branch)
            .into_any_element()
    }
}

/// A block: a small caption over the name and a caret, as high as the
/// toolbar and `width` wide whatever the name.
fn block(
    id: &'static str,
    caption: &'static str,
    name: String,
    bold: bool,
    width: f32,
    cx: &App,
) -> Button {
    let muted = cx.theme().muted_foreground;
    Button::new(id)
        .accessibility_label(format!("{caption}: {name}"))
        .ghost()
        .w(px(width))
        .h(px(TOOLBAR_HEIGHT - 1.))
        .px_0()
        .rounded_none()
        .child(
            v_flex()
                .w_full()
                .pl(px(BLOCK_INSET))
                .pr(px(6.))
                .gap(px(3.))
                .child(
                    div()
                        .text_size(px(11.))
                        .line_height(px(13.))
                        .text_color(muted)
                        .child(caption),
                )
                .child(
                    h_flex()
                        .w_full()
                        .gap_1()
                        .items_center()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .text_size(px(15.))
                                .line_height(px(18.))
                                .when(bold, |name| name.font_semibold())
                                .child(name),
                        )
                        .child(
                            Icon::new(IconName::ChevronDown)
                                .size_3()
                                .flex_none()
                                .text_color(muted),
                        ),
                ),
        )
}

/// The field at a panel's top.
fn field(search: &Entity<InputState>) -> Div {
    div()
        .w_full()
        .p(px(4.))
        .child(Input::new(search).h(px(FIELD_HEIGHT)))
}

/// A row of a panel: a button as wide as the panel.
fn row(id: impl Into<ElementId>, height: f32) -> Button {
    Button::new(id)
        .ghost()
        .w_full()
        .h(px(height))
        .px_3()
        .gap_1p5()
        .justify_start()
        .rounded_none()
}

/// The repository's panel: the recent ones, then the ways to open one and
/// out to other programs.
fn repo_panel(
    repos: &[PathBuf],
    search: &Entity<InputState>,
    shell: &WeakEntity<Shell>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    let muted = theme.muted_foreground;
    let elsewhere = |ix: usize, label: &'static str, what: Elsewhere| {
        let shell = shell.clone();
        row(("repo-finder-also", ix), REPO_ROW)
            .accessibility_label(label)
            .child(div().flex_1().text_sm().child(label))
            .on_click(move |_, window, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.set_finder(Which::Repository, false, window, cx);
                    let workspace = shell.active_workspace().cloned();
                    match (what, workspace) {
                        (Elsewhere::Browse, _) => shell.browse_for_repository(window, cx),
                        (Elsewhere::Outside(what), Some(workspace)) => {
                            workspace.update(cx, |w, cx| w.open_outside(what, window, cx))
                        }
                        (Elsewhere::Terminal, Some(workspace)) => {
                            workspace.update(cx, |w, cx| w.start_terminal(None, window, cx))
                        }
                        (Elsewhere::Maintenance, Some(_)) => shell.show_preferences(
                            crate::views::preferences::Section::Maintenance,
                            window,
                            cx,
                        ),
                        (_, None) => {}
                    }
                });
            })
    };
    v_flex()
        .w(px(PANEL_WIDTH))
        .pb_1()
        .child(field(search))
        .child(
            h_flex()
                .h(px(REPO_ROW))
                .px_3()
                .gap_1p5()
                .items_center()
                .text_xs()
                .text_color(muted)
                .child(Icon::new(gpui_kit::assets::IconName::Clock).size_3())
                .child(say!("shell-finder-recent")),
        )
        .children(repos.iter().enumerate().map(|(ix, path)| {
            let (shell, path) = (shell.clone(), path.clone());
            row(("repo-finder-row", ix), REPO_ROW)
                .accessibility_label(repo_title(&path))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_sm()
                        .child(repo_title(&path)),
                )
                .on_click(move |_, window, cx| {
                    let path = path.clone();
                    let _ = shell.update(cx, |shell, cx| shell.pick_repo(path, window, cx));
                })
        }))
        .when(repos.is_empty(), |panel| {
            panel.child(
                div()
                    .h(px(REPO_ROW))
                    .px_3()
                    .text_sm()
                    .text_color(muted)
                    .child(say!("shell-finder-no-other")),
            )
        })
        .child(div().my_1().h(px(1.)).bg(theme.border))
        .child(elsewhere(0, say!("shell-finder-open"), Elsewhere::Browse))
        .child(elsewhere(
            1,
            say!("shell-finder-file-manager"),
            Elsewhere::Outside(Outside::FileManager),
        ))
        .child(elsewhere(
            2,
            say!("shell-finder-editor"),
            Elsewhere::Outside(Outside::Editor),
        ))
        // The terminal in the window, and the system's own, told apart by
        // their words.
        .child(elsewhere(
            3,
            say!("shell-finder-terminal"),
            Elsewhere::Terminal,
        ))
        .child(elsewhere(
            4,
            say!("shell-finder-system-terminal"),
            Elsewhere::Outside(Outside::Terminal),
        ))
        // Its size, git's tasks and the check.
        .child(div().my_1().h(px(1.)).bg(theme.border))
        .child(elsewhere(
            5,
            say!("shell-finder-maintenance"),
            Elsewhere::Maintenance,
        ))
}

/// What a row under the repositories does.
#[derive(Debug, Clone, Copy)]
enum Elsewhere {
    /// Ask for a repository's folder.
    Browse,
    /// Open the repository's folder in a program outside.
    Outside(Outside),
    /// Start a terminal in the window, in the repository's folder.
    Terminal,
    /// Show the repository's maintenance section.
    Maintenance,
}

/// The branch's panel: the local branches, the current one first and
/// filled.
fn branch_panel(
    branches: &[(String, bool)],
    read_only: bool,
    search: &Entity<InputState>,
    shell: &WeakEntity<Shell>,
    cx: &App,
) -> Div {
    let theme = cx.theme();
    v_flex()
        .w(px(PANEL_WIDTH))
        .pb_1()
        .child(field(search))
        .when(read_only, |panel| {
            panel.child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(crate::read_only::IN_SHORT.text()),
            )
        })
        .children(branches.iter().enumerate().map(|(ix, (name, current))| {
            let (shell, name) = (shell.clone(), name.clone());
            let label = name.clone();
            row(("branch-finder-row", ix), BRANCH_ROW)
                .accessibility_label(label.clone())
                .when(*current, |row| row.bg(theme.accent))
                .disabled(read_only)
                .child(
                    Icon::new(gpui_kit::assets::IconName::GitBranch)
                        .size_3p5()
                        .flex_none()
                        .text_color(theme.muted_foreground),
                )
                .child(div().flex_1().min_w_0().truncate().text_sm().child(label))
                .on_click(move |_, window, cx| {
                    let name = name.clone();
                    let _ = shell.update(cx, |shell, cx| shell.pick_branch(name, window, cx));
                })
        }))
}
