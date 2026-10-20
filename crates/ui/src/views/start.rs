//! The start page: the recent repositories, open / clone / init, and the
//! workspaces.
//!
//! Laid out from the top left: a heading, the three ways to begin as a row
//! of buttons, then the list, each repository on one line; right of a
//! rule, the logo alone. The favourites are on the page of repositories;
//! here a row's menu makes one. Under the recent ones, the workspaces, one
//! on a line in the same measure: its icon in its colour, its name, how
//! many repositories it has and what it is for; a click shows its section
//! on the page of repositories.
//!
//! Sources: `brand/logo.svg`.

use std::path::{Path, PathBuf};

use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::Button,
    h_flex,
    menu::{ContextMenuExt, PopupMenu, PopupMenuItem},
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::prefs::{Prefs, RepoList};

/// Width of the logo on the start page, in pixels.
const LOGO_WIDTH: f32 = 260.0;
/// Where the column of repositories begins, from the left edge.
const LEFT_MARGIN: f32 = 100.0;
/// The space above the heading.
const TOP_MARGIN: f32 = 39.0;
/// How much of the width the repositories take, less a little at the
/// window's right edge; the rest is right of the rule.
const LEFT_SHARE: f32 = 0.6;
const LEFT_SHARE_LESS: f32 = 16.0;
/// The buttons to begin with: under the heading, and how high.
const BUTTONS_TOP: f32 = 17.0;
const BUTTON_HEIGHT: f32 = 45.0;
/// From the buttons to the caption of the list.
const CAPTION_TOP: f32 = 16.0;
/// A repository's line: how high, where its name begins, and the space
/// between the name and where it is.
const ROW_HEIGHT: f32 = 22.0;
const ROW_INSET: f32 = 3.0;
const NAME_GAP: f32 = 12.0;

/// Where the rule stands in a window this wide: the repositories left of
/// it.
fn left_width(window: f32) -> f32 {
    ((window - LEFT_SHARE_LESS) * LEFT_SHARE).max(0.)
}

/// Events from the start page.
#[derive(Debug, Clone)]
pub enum StartEvent {
    /// Open this repository.
    Open(PathBuf),
    /// Show the directory chooser.
    Browse,
    /// Ask what to clone and where to.
    Clone,
    /// Ask where the new repository goes.
    Init,
    /// Ask where a repository whose folder is gone is now.
    Locate(PathBuf),
    /// Show this workspace's section of the page of repositories.
    Workspace(String),
}

/// The start page view.
pub struct StartPage {
    focus_handle: FocusHandle,
    /// The workspaces, as their file had them when they were read last.
    workspaces: Vec<crate::views::repositories::Listed>,
}

impl EventEmitter<StartEvent> for StartPage {}

impl StartPage {
    /// Create the view; the workspaces are read off the main thread.
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut page = Self {
            focus_handle: cx.focus_handle(),
            workspaces: Vec::new(),
        };
        page.read_workspaces(cx);
        page
    }

    /// Read the workspaces again, off the main thread: when the page is
    /// made, and when a command may have changed them.
    pub fn read_workspaces(&mut self, cx: &mut Context<Self>) {
        let run = crate::session::run_standalone_unasked("workspace.list", serde_json::json!({}));
        cx.spawn(async move |this, cx| {
            if let Ok(crate::session::Reply::Command(value)) = run.reply.await {
                let _ = this.update(cx, |this, cx| {
                    this.workspaces = crate::views::repositories::listed(&value);
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// The workspaces it lists.
    pub fn workspaces(&self) -> &[crate::views::repositories::Listed] {
        &self.workspaces
    }

    /// One workspace of the list: its icon in its colour, its name, how
    /// many repositories it has and what it is for. A click shows its
    /// section of the page of repositories.
    fn workspace_row(
        &self,
        listed: &crate::views::repositories::Listed,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        use crate::views::repositories::{workspace_colour, workspace_icon};
        let theme = cx.theme().clone();
        let workspace = &listed.workspace;
        let id = workspace.id.clone();
        let count = workspace.repositories.len();
        let said = match &workspace.description {
            None => say!("start-workspace-repositories", count),
            Some(description) => say!("start-workspace-repositories-described", count, description),
        };
        h_flex()
            .id(SharedString::from(format!("start-workspace-{id}")))
            .test_support()
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .pl(px(ROW_INSET))
            .gap(px(8.))
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| cx.emit(StartEvent::Workspace(id.clone()))))
            .child(
                workspace_icon(workspace.icon.as_deref())
                    .size(px(14.))
                    .text_color(
                        workspace_colour(workspace.colour.as_deref())
                            .unwrap_or(theme.muted_foreground),
                    ),
            )
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.5))
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .text_color(theme.link)
                    .child(workspace.name.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .ml(px(NAME_GAP - 8.))
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(said),
            )
            .into_any_element()
    }

    /// One repository of the list: its name and on the same line where it
    /// is, nothing else. What can be done with the entry (make it a
    /// favourite, move it, take it out) is in its menu, on the pointer's
    /// other button. One whose folder is gone says so; a click on it asks
    /// where it is now.
    fn repo_row(&self, path: &Path, missing: bool, cx: &mut Context<Self>) -> AnyElement {
        let list = RepoList::Recent;
        let theme = cx.theme().clone();
        let drop_target = theme.drop_target;
        let title = crate::shell::repo_title(path);
        let display = path.display().to_string();
        let key = SharedString::from(format!("rec-{display}"));
        let (click_path, drop_path) = (path.to_path_buf(), path.to_path_buf());
        let dragged = DraggedRepo {
            list,
            path: path.to_path_buf(),
            title: title.clone(),
        };
        let menu = row_menu(cx.entity().downgrade(), path.to_path_buf(), missing);
        h_flex()
            .id(key)
            .test_support()
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .pl(px(ROW_INSET))
            .gap(px(NAME_GAP))
            .items_center()
            .cursor_pointer()
            .on_click(cx.listener(move |_, _, _, cx| {
                cx.emit(if missing {
                    StartEvent::Locate(click_path.clone())
                } else {
                    StartEvent::Open(click_path.clone())
                })
            }))
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| crate::views::rewrite::DragChip(dragged.title.clone()))
            })
            .drag_over::<DraggedRepo>(move |style, dragged, _, _| {
                if dragged.list == list {
                    style.bg(drop_target)
                } else {
                    style
                }
            })
            .on_drop(cx.listener(move |_, dragged: &DraggedRepo, _, cx| {
                if dragged.list != list {
                    return;
                }
                let (from, to) = (dragged.path.clone(), drop_path.clone());
                change(cx, &|state| state.move_entry(list, &from, Some(&to)));
                cx.notify();
            }))
            .context_menu(move |popup, window, cx| menu(popup, window, cx))
            .child(
                div()
                    .flex_none()
                    .max_w(relative(0.5))
                    .truncate()
                    .text_sm()
                    .font_semibold()
                    .when(missing, |name| name.text_color(theme.muted_foreground))
                    .when(!missing, |name| name.text_color(theme.link))
                    .child(title),
            )
            .when(missing, |name| {
                name.child(
                    div()
                        .flex_none()
                        .px_1p5()
                        .rounded(theme.radius)
                        .bg(theme.danger.opacity(0.2))
                        .text_xs()
                        .text_color(theme.danger)
                        .child(say!("start-folder-gone")),
                )
            })
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(display),
            )
            .into_any_element()
    }
}

/// Change what is remembered, and save it when it changed.
fn change(cx: &mut App, change: &dyn Fn(&mut crate::prefs::State) -> bool) {
    let prefs = cx.global_mut::<Prefs>();
    if change(&mut prefs.state) {
        prefs.save_state();
    }
}

/// The menu of a repository of the list: open it (or say where it is
/// now), make it a favourite or not, move it up or down, take it out.
fn row_menu(
    page: WeakEntity<StartPage>,
    path: PathBuf,
    missing: bool,
) -> impl Fn(PopupMenu, &mut Window, &mut Context<PopupMenu>) -> PopupMenu + 'static {
    move |menu, _, cx| {
        let favourite = cx
            .global::<Prefs>()
            .state
            .favourites
            .iter()
            .any(|known| known == &path);
        // What an entry does, on the page.
        let entry = |label: &'static str, what: Do| {
            let (page, path) = (page.clone(), path.clone());
            PopupMenuItem::new(label).on_click(move |_, _, cx| {
                let path = path.clone();
                let _ = page.update(cx, |_, cx| {
                    match what {
                        Do::Open => cx.emit(StartEvent::Open(path)),
                        Do::Locate => cx.emit(StartEvent::Locate(path)),
                        Do::Star => change(cx, &|state| {
                            if state.favourites.contains(&path) {
                                state.remove_entry(RepoList::Favourites, &path)
                            } else {
                                state.favourites.push(path.clone());
                                true
                            }
                        }),
                        Do::Up => change(cx, &|state| {
                            state.nudge_entry(RepoList::Recent, &path, false)
                        }),
                        Do::Down => change(cx, &|state| {
                            state.nudge_entry(RepoList::Recent, &path, true)
                        }),
                        Do::TakeOut => {
                            change(cx, &|state| state.remove_entry(RepoList::Recent, &path))
                        }
                    }
                    cx.notify();
                });
            })
        };
        menu.item(if missing {
            entry(say!("start-menu-locate"), Do::Locate)
        } else {
            entry(say!("start-menu-open"), Do::Open)
        })
        .separator()
        .item(if favourite {
            entry(say!("start-menu-unfavourite"), Do::Star)
        } else {
            entry(say!("start-menu-favourite"), Do::Star)
        })
        .item(entry(say!("start-menu-move-up"), Do::Up))
        .item(entry(say!("start-menu-move-down"), Do::Down))
        .separator()
        .item(entry(say!("start-menu-take-out"), Do::TakeOut))
    }
}

/// What an entry of a row's menu does.
#[derive(Debug, Clone, Copy)]
enum Do {
    Open,
    Locate,
    Star,
    Up,
    Down,
    TakeOut,
}

/// A repository of one of the lists, picked up to be put elsewhere in it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DraggedRepo {
    list: RepoList,
    path: PathBuf,
    title: String,
}

/// The entries of a list as the page shows them, each with whether its
/// folder is gone; those in `leave_out` are not listed.
pub(crate) fn listed(
    entries: &[PathBuf],
    leave_out: &[PathBuf],
    exists: impl Fn(&Path) -> bool,
) -> Vec<(PathBuf, bool)> {
    entries
        .iter()
        .filter(|path| !leave_out.contains(path))
        .map(|path| (path.clone(), !exists(path)))
        .collect()
}

impl Focusable for StartPage {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for StartPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let recent = {
            let state = &cx.global::<Prefs>().state;
            listed(&state.recent, &[], |path: &Path| path.is_dir())
        };
        let recent_rows: Vec<AnyElement> = recent
            .iter()
            .map(|(path, missing)| self.repo_row(path, *missing, cx))
            .collect();
        let workspace_rows: Vec<AnyElement> = self
            .workspaces
            .clone()
            .iter()
            .map(|listed| self.workspace_row(listed, cx))
            .collect();
        let theme = cx.theme().clone();
        let muted = theme.muted_foreground;
        // One of the three things to begin with: a button 45 high and as
        // wide as its place, its icon before its word.
        let begin = |id: &'static str, icon: Icon, label: &'static str, width: f32| {
            Button::new(id)
                .outline()
                .large()
                .h(px(BUTTON_HEIGHT))
                .w(px(width))
                .px(px(8.))
                .accessibility_label(label)
                .child(
                    h_flex()
                        .gap(px(10.))
                        .items_center()
                        .child(icon.size(px(18.)))
                        .child(label),
                )
        };
        // The repositories at the left, from the top; what the application
        // is at the right of a rule. Nothing is centred: the list grows
        // downwards and the buttons stay where they are.
        let left = v_flex()
            .id("start-column")
            .w(px(left_width(crate::zoom::unpx(
                window.viewport_size().width,
            ))))
            .h_full()
            .flex_none()
            .pl(px(LEFT_MARGIN))
            .pr(px(20.))
            .pt(px(TOP_MARGIN))
            .pb_4()
            .overflow_y_scroll()
            .child(
                div()
                    .flex_none()
                    .h(px(26.))
                    .text_xl()
                    .line_height(px(26.))
                    .child(say!("start-heading")),
            )
            .child(
                h_flex()
                    .flex_none()
                    .mt(px(BUTTONS_TOP))
                    .gap(px(10.))
                    .child(
                        begin(
                            "browse",
                            Icon::new(IconName::FolderOpen),
                            say!("start-open"),
                            91.,
                        )
                        .tooltip(say!("start-open-tooltip"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(StartEvent::Browse))),
                    )
                    .child(
                        begin(
                            "clone",
                            Icon::new(gpui_kit::assets::IconName::CloudDownload),
                            say!("start-clone"),
                            95.,
                        )
                        .tooltip(say!("start-clone-tooltip"))
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(StartEvent::Clone))),
                    )
                    .child(
                        begin("init", Icon::new(IconName::Plus), say!("start-create"), 95.)
                            .tooltip(say!("start-create-tooltip"))
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(StartEvent::Init))),
                    ),
            )
            .when(!recent_rows.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .pt(px(CAPTION_TOP))
                        .text_sm()
                        .text_color(muted)
                        .child(say!("start-recent")),
                )
                .children(recent_rows)
            })
            .when(recent.is_empty(), |this| {
                this.child(
                    div()
                        .pt(px(22.))
                        .text_sm()
                        .text_color(muted)
                        .child(say!("start-empty")),
                )
            })
            .when(!workspace_rows.is_empty(), |this| {
                this.child(
                    div()
                        .flex_none()
                        .pt(px(CAPTION_TOP))
                        .text_sm()
                        .text_color(muted)
                        .child(say!("start-workspaces")),
                )
                .children(workspace_rows)
            });
        let right = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .pl(px(20.))
            .pr_4()
            .pt(px(TOP_MARGIN))
            .gap_3()
            // The logo at well over its smallest size, with the clear
            // space its rules ask for left around it, drawn at the pixels
            // of the screen it takes.
            .child(
                crate::brand::logo_image(theme.is_dark(), px(LOGO_WIDTH), window, cx)
                    .id("start-logo"),
            );
        h_flex()
            .id("start")
            .track_focus(&self.focus_handle)
            .size_full()
            .items_start()
            .child(left)
            .child(
                div()
                    .flex_none()
                    .w(px(1.))
                    .h_full()
                    .pt(px(42.))
                    .pb(px(40.))
                    .child(div().size_full().bg(theme.border)),
            )
            .child(right)
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::listed;

    #[test]
    fn a_repository_whose_folder_is_gone_is_listed_as_missing() {
        let path = |name: &str| PathBuf::from(format!("/work/{name}"));
        let there = |path: &Path| !path.ends_with("gone");
        let favourites = vec![path("fav"), path("gone")];
        let recent = vec![path("a"), path("fav"), path("gone"), path("b")];
        assert_eq!(
            listed(&favourites, &[], there),
            [(path("fav"), false), (path("gone"), true)]
        );
        // What is a favourite is not listed a second time.
        assert_eq!(
            listed(&recent, &favourites, there),
            [(path("a"), false), (path("b"), false)]
        );
        assert_eq!(
            listed(&recent, &[], there),
            [
                (path("a"), false),
                (path("fav"), false),
                (path("gone"), true),
                (path("b"), false)
            ]
        );
        assert!(listed(&[], &[], there).is_empty());
    }
}
