//! The dialog that makes a workspace, or edits one.
//!
//! 598 wide, centred, 83 under the tabs; a title row 52 high with the
//! cross at its right; one row with the icon, the name and the colour,
//! then the description; then where the repositories come from (picked
//! one by one, a folder that is looked through, or a VS Code workspace
//! file), and a box that has the workspace follow the folder; the
//! repositories found, each with a cross, and a way to clear them; the
//! button that makes it at the bottom right, greyed until there is a name
//! and a repository. Editing has all but the repositories. Workspaces are
//! local only. The icon is one of a few of our icon set. The dialog also
//! has what a workspace's repositories open with: a profile, and the
//! layout of the tab on show; and what a folder looked through left out
//! and why.
//!
//! The engine makes and keeps the workspace (`workspace.create`,
//! `workspace.update`) and looks through folders and files
//! (`workspace.scan`, `workspace.import_code_workspace`); nothing here
//! reads a folder.
//!
//! Sources: the VS Code documentation on multi-root workspaces
//! (code.visualstudio.com/docs/editing/workspaces/multi-root-workspaces)
//! for what a `.code-workspace` file is.

use std::path::{Path, PathBuf};

use bitbraid_engine::workspace::{Layout, Workspace, WorkspaceRepository};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde_json::{Value, json};

use super::{
    RepositoriesView,
    workspaces::{COLOURS, ICONS, colour, colour_name, icon, sentence},
};
use crate::{
    prefs::{Prefs, RepoState},
    session::{Reply, run_standalone},
    zoom::px,
};

/// The dialog's width, and where it begins under the window's top.
const WIDTH: f32 = 598.;
const TOP: f32 = 83.;
/// Its title row, the title's place in it and the cross's.
const TITLE: f32 = 52.;
const TITLE_LEFT: f32 = 20.;
const CLOSE_RIGHT: f32 = 16.;
/// From its edges to what is in it.
const SIDE: f32 = 20.;
/// The box with the icon, and a field.
const ICON_BOX: f32 = 58.;
const FIELD: f32 = 32.;
/// The colour's list.
const COLOUR_WIDTH: f32 = 200.;
/// The description's box.
const DESCRIPTION: f32 = 63.;
/// A repository found, in the list of them, and the most rows the list
/// shows before it scrolls, where the window is high enough.
const FOUND_ROW: f32 = 40.;
const FOUND_ROWS: f32 = 5.;
/// What the rest of the dialog takes, for the list's height in a low
/// window.
const REST: f32 = 560.;
/// The words over a field, and the space between groups.
const CAPTION: f32 = 22.;
const GAP: f32 = 14.;

/// Which workspace the dialog is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Making {
    /// A new one.
    New,
    /// The one of this id.
    Edit(String),
}

/// A repository the new workspace is to have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Its folder.
    pub path: String,
    /// Where it can be cloned from, when that is known.
    pub remote_url: Option<String>,
}

/// The dialog's state.
pub struct WorkspaceForm {
    making: Making,
    page: WeakEntity<RepositoriesView>,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    icon: Option<String>,
    colour: Option<String>,
    /// The repositories found so far, for a new one.
    found: Vec<Candidate>,
    /// What a folder or file looked through left out, and why.
    skipped: Vec<String>,
    /// The last folder looked through, and whether the workspace is to
    /// follow it.
    folder: Option<String>,
    follow: bool,
    /// The profiles there are, and the one its repositories open with.
    profiles: Vec<String>,
    profile: Option<String>,
    /// The layout its repositories open with: the one it has, or the one
    /// of the tab on show, and whether it is kept.
    layout: Option<(String, Layout)>,
    keep_layout: bool,
    /// The names of the workspaces there are, which a new name must not
    /// be (in whatever case).
    taken: Vec<String>,
    /// What the engine said when it would not, or while it looks.
    refused: Option<String>,
    busy: bool,
    _subscriptions: Vec<Subscription>,
}

impl WorkspaceForm {
    fn new(
        making: Making,
        page: WeakEntity<RepositoriesView>,
        existing: Option<&Workspace>,
        taken: Vec<String>,
        current: Option<(String, Layout)>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(say!("repositories-form-name-placeholder"))
                .default_value(existing.map(|one| one.name.clone()).unwrap_or_default())
        });
        let description = cx.new(|cx| {
            TextareaState::new(window, cx)
                .placeholder(say!("repositories-form-description-placeholder"))
                // Tab goes on to the next control: a text that grows,
                // unlike one of fixed rows, is not indented.
                .auto_grow(2, 2)
                .default_value(
                    existing
                        .and_then(|one| one.description.clone())
                        .unwrap_or_default(),
                )
        });
        let typed = cx.subscribe(&name, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.refused = None;
                cx.notify();
            }
        });
        let layout = match existing {
            Some(one) => one
                .layout
                .clone()
                .filter(|layout| !layout.is_empty())
                .map(|layout| (say!("repositories-form-layout-own").to_owned(), layout))
                .or(current),
            None => current,
        };
        let keep_layout = existing.is_some_and(|one| one.layout.is_some());
        let mut form = Self {
            making,
            page,
            name,
            description,
            icon: existing.and_then(|one| one.icon.clone()),
            colour: existing.and_then(|one| one.colour.clone()),
            found: Vec::new(),
            skipped: Vec::new(),
            folder: existing.and_then(|one| one.follows_folder.clone()),
            follow: existing.is_some_and(|one| one.follows_folder.is_some()),
            profiles: Vec::new(),
            profile: existing.and_then(|one| one.default_profile.clone()),
            layout,
            keep_layout,
            taken,
            refused: None,
            busy: false,
            _subscriptions: vec![typed],
        };
        form.read_profiles(window, cx);
        form
    }

    /// The profiles there are, for the list of them.
    fn read_profiles(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let run = run_standalone("profile.list", json!({}));
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Reply::Command(value)) = run.reply.await {
                let names: Vec<String> = value
                    .get("profiles")
                    .and_then(Value::as_array)
                    .or_else(|| value.as_array())
                    .map(|all| {
                        all.iter()
                            .filter_map(|one| one.get("name").and_then(Value::as_str))
                            .map(str::to_owned)
                            .collect()
                    })
                    .unwrap_or_default();
                let _ = this.update(cx, |this, cx| {
                    this.profiles = names;
                    cx.notify();
                });
            }
        })
        .detach();
    }

    /// Which workspace it is for.
    pub fn making(&self) -> &Making {
        &self.making
    }

    /// The repositories found so far.
    pub fn found(&self) -> &[Candidate] {
        &self.found
    }

    /// What was left out, and why.
    pub fn skipped(&self) -> &[String] {
        &self.skipped
    }

    /// What the engine said when it would not.
    pub fn refused(&self) -> Option<&str> {
        self.refused.as_deref()
    }

    /// The name as typed.
    fn typed_name(&self, cx: &App) -> String {
        self.name.read(cx).value().trim().to_owned()
    }

    /// Why the dialog will not go ahead yet; none when it will.
    pub fn waits_for(&self, cx: &App) -> Option<&'static str> {
        let name = self.typed_name(cx);
        if name.is_empty() {
            return Some(say!("repositories-form-needs-name"));
        }
        let editing = match &self.making {
            Making::Edit(id) => Some(id.as_str()),
            Making::New => None,
        };
        let page = self.page.upgrade();
        let own = page.as_ref().and_then(|page| {
            editing.and_then(|id| {
                page.read(cx)
                    .workspace(id)
                    .map(|listed| listed.workspace.name.to_lowercase())
            })
        });
        if self.taken.iter().any(|taken| {
            taken.to_lowercase() == name.to_lowercase()
                && own.as_deref() != Some(&taken.to_lowercase())
        }) {
            return Some(say!("repositories-form-needs-free-name"));
        }
        if self.making == Making::New && self.found.is_empty() {
            return Some(say!("repositories-form-needs-repository"));
        }
        None
    }

    /// Take in repositories, as when they were picked one by one.
    pub fn take_found(&mut self, found: Vec<Candidate>, cx: &mut Context<Self>) {
        for candidate in found {
            if !self.found.iter().any(|known| known.path == candidate.path) {
                self.found.push(candidate);
            }
        }
        self.refused = None;
        cx.notify();
    }

    /// Take a repository out of the list.
    pub fn drop_found(&mut self, path: &str, cx: &mut Context<Self>) {
        self.found.retain(|known| known.path != path);
        cx.notify();
    }

    /// Clear the list.
    pub fn clear_found(&mut self, cx: &mut Context<Self>) {
        self.found.clear();
        self.skipped.clear();
        self.folder = None;
        self.follow = false;
        cx.notify();
    }

    /// Whether the workspace is to follow the folder looked through.
    pub fn set_follow(&mut self, follow: bool, cx: &mut Context<Self>) {
        self.follow = follow && self.folder.is_some();
        cx.notify();
    }

    /// Its colour, as the engine keeps it, or none.
    pub fn set_colour(&mut self, colour: Option<&str>, cx: &mut Context<Self>) {
        self.colour = colour.map(str::to_owned);
        cx.notify();
    }

    /// Its icon, by a name of [`ICONS`], or none.
    pub fn set_icon(&mut self, icon: Option<&str>, cx: &mut Context<Self>) {
        self.icon = icon.map(str::to_owned);
        cx.notify();
    }

    /// The profile its repositories open with, or none.
    pub fn set_profile(&mut self, profile: Option<&str>, cx: &mut Context<Self>) {
        self.profile = profile.map(str::to_owned);
        cx.notify();
    }

    /// Whether its repositories open with the layout on offer.
    pub fn set_keep_layout(&mut self, keep: bool, cx: &mut Context<Self>) {
        self.keep_layout = keep && self.layout.is_some();
        cx.notify();
    }

    /// Ask for folders: each one that is a repository is taken, and each
    /// that holds repositories is looked through for them.
    fn pick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(say!("repositories-add-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    for path in &paths {
                        this.scan(path, window, cx);
                    }
                });
            }
        })
        .detach();
    }

    /// Ask for a VS Code workspace file.
    fn pick_code_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(say!("repositories-form-code-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(file) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.read_code_workspace(&file, window, cx)
                });
            }
        })
        .detach();
    }

    /// Look through a folder for repositories (`workspace.scan`), and
    /// take in what is found; what is left out is said.
    pub fn scan(&mut self, folder: &Path, window: &mut Window, cx: &mut Context<Self>) {
        let shown = folder.display().to_string();
        self.ask_engine(
            "workspace.scan",
            json!({ "path": shown }),
            Some(shown),
            window,
            cx,
        );
    }

    /// Take the folders of a VS Code workspace file
    /// (`workspace.import_code_workspace`).
    pub fn read_code_workspace(
        &mut self,
        file: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ask_engine(
            "workspace.import_code_workspace",
            json!({ "file": file.display().to_string() }),
            None,
            window,
            cx,
        );
    }

    /// Ask the engine for repositories, and take in its answer.
    fn ask_engine(
        &mut self,
        id: &'static str,
        params: Value,
        folder: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.busy = true;
        self.refused = None;
        cx.notify();
        let run = run_standalone(id, params);
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update(cx, |this, cx| {
                this.busy = false;
                match reply {
                    Ok(Reply::Command(value)) => {
                        let (found, skipped) = answer(&value);
                        // A folder that holds repositories can be
                        // followed; one that is a repository is just that.
                        if let Some(folder) = &folder
                            && !found.iter().any(|candidate| candidate.path == *folder)
                        {
                            this.folder = Some(folder.clone());
                        }
                        for one in skipped {
                            if !this.skipped.contains(&one) {
                                this.skipped.push(one);
                            }
                        }
                        if found.is_empty() {
                            this.refused = Some(say!("repositories-form-none-found").to_owned());
                        }
                        this.take_found(found, cx);
                    }
                    Ok(Reply::Error(err)) => this.refused = Some(sentence(&err)),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// Make the workspace, or save what was changed; the dialog closes
    /// when the engine took it.
    pub fn submit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy || self.waits_for(cx).is_some() {
            return;
        }
        let name = self.typed_name(cx);
        let description = self.description.read(cx).value().trim().to_owned();
        let follows = if self.follow {
            self.folder.clone().unwrap_or_default()
        } else {
            String::new()
        };
        let layout = match (&self.layout, self.keep_layout) {
            (Some((_, layout)), true) => serde_json::to_value(layout).unwrap_or(Value::Null),
            _ => json!({}),
        };
        let (id, params) = match &self.making {
            Making::New => (
                "workspace.create",
                json!({
                    "name": name,
                    "colour": self.colour.clone().unwrap_or_default(),
                    "icon": self.icon.clone().unwrap_or_default(),
                    "description": description,
                    "default_profile": self.profile.clone().unwrap_or_default(),
                    "follows_folder": follows,
                    "layout": layout,
                    "repositories": self.found.iter().map(|candidate| json!({
                        "path": candidate.path,
                        "remote_url": candidate.remote_url,
                    })).collect::<Vec<_>>(),
                }),
            ),
            Making::Edit(id) => (
                "workspace.update",
                json!({
                    "id": id,
                    "name": name,
                    "colour": self.colour.clone().unwrap_or_default(),
                    "icon": self.icon.clone().unwrap_or_default(),
                    "description": description,
                    "default_profile": self.profile.clone().unwrap_or_default(),
                    "follows_folder": follows,
                    "layout": layout,
                }),
            ),
        };
        self.busy = true;
        cx.notify();
        let run = run_standalone(id, params);
        let page = self.page.clone();
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.busy = false;
                match reply {
                    Ok(Reply::Command(value)) => {
                        let made = value
                            .pointer("/workspace/id")
                            .and_then(Value::as_str)
                            .map(str::to_owned);
                        window.close_dialog(cx);
                        let _ = page.update(cx, |page, cx| {
                            page.form = None;
                            if let Some(id) = made {
                                let key = super::Section::Workspace(id).key();
                                page.folded.retain(|folded| *folded != key);
                            }
                            page.look(window, cx);
                        });
                    }
                    Ok(Reply::Error(err)) => this.refused = Some(sentence(&err)),
                    _ => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// The box with the workspace's icon, which drops the icons to pick.
    fn icon_box(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let form = cx.entity().downgrade();
        let current = self.icon.clone();
        let shown = colour(self.colour.as_deref()).unwrap_or(theme.foreground);
        Button::new("workspace-icon")
            .accessibility_label(say!("repositories-form-icon-tip"))
            .outline()
            .w(px(ICON_BOX))
            .h(px(ICON_BOX))
            .flex_none()
            .child(
                v_flex()
                    .items_center()
                    .gap(px(4.))
                    .child(icon(current.as_deref()).size(px(20.)).text_color(shown))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("repositories-form-icon")),
                    ),
            )
            .tooltip(say!("repositories-form-icon-tip"))
            .dropdown_menu(move |mut menu, _, _| {
                for name in ICONS {
                    let form = form.clone();
                    menu = menu.item(
                        PopupMenuItem::new(icon_label(name))
                            .icon(icon(Some(name)))
                            .checked(current.as_deref().unwrap_or(ICONS[0]) == name)
                            .on_click(move |_, _, cx| {
                                let _ = form.update(cx, |this, cx| this.set_icon(Some(name), cx));
                            }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// The list of colours.
    fn colour_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let form = cx.entity().downgrade();
        let current = self.colour.clone();
        let swatch = |fill: Hsla| {
            div()
                .size(px(14.))
                .flex_none()
                .rounded(px(3.))
                .border_1()
                .border_color(theme.border)
                .bg(fill)
        };
        let fill = colour(current.as_deref()).unwrap_or(theme.secondary);
        let named = colour_name(current.as_deref()).unwrap_or(say!("repositories-colour-none"));
        Button::new("workspace-colour")
            .accessibility_label(say!("repositories-form-colour-is", named))
            .outline()
            .w(px(COLOUR_WIDTH))
            .h(px(FIELD))
            .child(
                h_flex()
                    .w_full()
                    .gap(px(8.))
                    .items_center()
                    .child(swatch(fill))
                    .child(div().flex_1().text_sm().child(named))
                    .child(Icon::new(IconName::ChevronDown).size(px(14.))),
            )
            .dropdown_menu(move |mut menu, _, _| {
                let entries = std::iter::once((say!("repositories-colour-none"), None))
                    .chain(COLOURS.iter().map(|(name, hex)| (name.text(), Some(*hex))));
                for (name, hex) in entries {
                    let form = form.clone();
                    let checked = match (hex, current.as_deref()) {
                        (None, None) => true,
                        (Some(hex), Some(current)) => hex.eq_ignore_ascii_case(current),
                        _ => false,
                    };
                    let shown = hex.and_then(|hex| colour(Some(hex)));
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            let theme = cx.theme();
                            h_flex()
                                .gap(px(8.))
                                .items_center()
                                .child(
                                    div()
                                        .size(px(12.))
                                        .rounded(px(2.))
                                        .border_1()
                                        .border_color(theme.border)
                                        .bg(shown.unwrap_or(theme.secondary)),
                                )
                                .child(name)
                        })
                        .checked(checked)
                        .on_click(move |_, _, cx| {
                            let _ = form.update(cx, |this, cx| this.set_colour(hex, cx));
                        }),
                    );
                }
                menu
            })
            .into_any_element()
    }

    /// Where the repositories come from, and the ones found.
    fn repositories(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let form = cx.entity().downgrade();
        let follow_on = self.follow;
        let can_follow = self.folder.is_some();
        let list_height = (crate::zoom::unpx(window.viewport_size().height) - REST)
            .clamp(FOUND_ROW * 2., FOUND_ROW * FOUND_ROWS);
        let count = self.found.len();
        let rows = self.found.iter().enumerate().map(|(ix, candidate)| {
            let path = candidate.path.clone();
            let name = Path::new(&path)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.clone());
            let form = form.clone();
            h_flex()
                .id(("workspace-found", ix))
                .w_full()
                .h(px(FOUND_ROW))
                .flex_none()
                .px(px(8.))
                .gap(px(8.))
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .child(
                    Icon::new(gpui_kit::assets::IconName::FolderGit2)
                        .size(px(14.))
                        .text_color(theme.muted_foreground),
                )
                .child(div().flex_none().text_sm().child(name))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(path.clone()),
                )
                .child(
                    Button::new(("workspace-found-drop", ix))
                        .accessibility_label(say!("repositories-form-leave-out"))
                        .ghost()
                        .xsmall()
                        .icon(IconName::Close)
                        .tooltip(say!("repositories-form-leave-out"))
                        .on_click(move |_, _, cx| {
                            let path = path.clone();
                            let _ = form.update(cx, |this, cx| this.drop_found(&path, cx));
                        }),
                )
                .test_support()
        });
        let skipped = self.skipped.clone();
        let (pick, code) = (cx.entity().downgrade(), cx.entity().downgrade());
        let (follow_form, clear_form) = (form.clone(), form.clone());
        v_flex()
            .w_full()
            .gap(px(8.))
            .child(caption(say!("repositories-form-repositories"), cx))
            .child(
                h_flex()
                    .w_full()
                    .gap(px(10.))
                    .items_center()
                    .child(
                        Button::new("workspace-pick")
                            .outline()
                            .small()
                            .icon(IconName::FolderOpen)
                            .label(say!("repositories-browse"))
                            .tooltip(say!("repositories-form-browse-tip"))
                            .on_click(move |_, window, cx| {
                                let _ = pick.update(cx, |this, cx| this.pick(window, cx));
                            }),
                    )
                    .child(
                        Button::new("workspace-code")
                            .outline()
                            .small()
                            .icon(gpui_kit::assets::IconName::FileInput)
                            .label(say!("repositories-form-code"))
                            .tooltip(say!("repositories-form-code-tip"))
                            .on_click(move |_, window, cx| {
                                let _ = code
                                    .update(cx, |this, cx| this.pick_code_workspace(window, cx));
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        Checkbox::new("workspace-follow")
                            .label(say!("repositories-form-follow"))
                            .checked(follow_on)
                            .disabled(!can_follow)
                            .tooltip(say!("repositories-form-follow-tip"))
                            .on_click(move |on: &bool, _, cx: &mut App| {
                                let on = *on;
                                let _ = follow_form.update(cx, |this, cx| this.set_follow(on, cx));
                            }),
                    ),
            )
            .when(count > 0, |column| {
                column
                    .child(
                        h_flex()
                            .w_full()
                            .h(px(26.))
                            .items_center()
                            .justify_between()
                            .text_sm()
                            .child(
                                div()
                                    .id("workspace-count")
                                    .child(say!("repositories-form-count", count))
                                    .test_support(),
                            )
                            .child(
                                Button::new("workspace-clear")
                                    .outline()
                                    .xsmall()
                                    .label(say!("repositories-form-clear"))
                                    .on_click(move |_, _, cx| {
                                        let _ =
                                            clear_form.update(cx, |this, cx| this.clear_found(cx));
                                    }),
                            ),
                    )
                    .child(
                        div()
                            .id("workspace-found-list")
                            .w_full()
                            .max_h(px(list_height))
                            .overflow_y_scrollbar()
                            .rounded(theme.radius)
                            .border_1()
                            .border_color(theme.border)
                            .child(v_flex().w_full().children(rows)),
                    )
            })
            .when(!skipped.is_empty(), |column| {
                let all = skipped.join("\n");
                let first = skipped.first().cloned().unwrap_or_default();
                let more = skipped.len().saturating_sub(1);
                column.child(
                    div()
                        .id("workspace-skipped")
                        .w_full()
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(say!(
                            "repositories-form-left-out",
                            first = first.as_str(),
                            more
                        ))
                        .tooltip(move |window, cx| Tooltip::new(all.clone()).build(window, cx))
                        .test_support(),
                )
            })
            .into_any_element()
    }

    /// What its repositories open with.
    fn defaults(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let form = cx.entity().downgrade();
        let profiles = self.profiles.clone();
        let current = self.profile.clone();
        let layout = self.layout.as_ref().map(|(from, _)| from.clone());
        let keep = self.keep_layout;
        let layout_form = form.clone();
        h_flex()
            .w_full()
            .gap(px(12.))
            .items_center()
            .child(div().text_sm().child(say!("repositories-form-opens-with")))
            .child(
                Button::new("workspace-profile")
                    .outline()
                    .small()
                    .icon(gpui_kit::assets::IconName::UserRound)
                    .label(match &current {
                        Some(name) => say!("repositories-form-profile", name = name.as_str()),
                        None => say!("repositories-form-no-profile").to_owned(),
                    })
                    .tooltip(say!("repositories-form-profile-tip"))
                    .dropdown_menu(move |mut menu, _, _| {
                        let none = form.clone();
                        menu = menu.item(
                            PopupMenuItem::new(say!("repositories-form-no-profile"))
                                .checked(current.is_none())
                                .on_click(move |_, _, cx| {
                                    let _ = none.update(cx, |this, cx| this.set_profile(None, cx));
                                }),
                        );
                        for name in &profiles {
                            let (form, name) = (form.clone(), name.clone());
                            let checked = current.as_deref() == Some(name.as_str());
                            menu = menu.item(
                                PopupMenuItem::new(name.clone()).checked(checked).on_click(
                                    move |_, _, cx| {
                                        let _ = form.update(cx, |this, cx| {
                                            this.set_profile(Some(&name), cx)
                                        });
                                    },
                                ),
                            );
                        }
                        menu
                    }),
            )
            .child(
                Checkbox::new("workspace-layout")
                    .label(match &layout {
                        Some(from) => say!("repositories-form-layout", from = from.as_str()),
                        None => say!("repositories-form-no-layout").to_owned(),
                    })
                    .checked(keep)
                    .disabled(layout.is_none())
                    .on_click(move |on: &bool, _, cx: &mut App| {
                        let on = *on;
                        let _ = layout_form.update(cx, |this, cx| this.set_keep_layout(on, cx));
                    }),
            )
            .child(div().flex_1())
            .child(div().text_xs().text_color(theme.muted_foreground))
            .into_any_element()
    }
}

/// What an icon of [`ICONS`] is called in the list of them.
fn icon_label(name: &'static str) -> &'static str {
    match name {
        "layers" => say!("repositories-icon-layers"),
        "folder" => say!("repositories-icon-folder"),
        "package" => say!("repositories-icon-package"),
        "code-xml" => say!("repositories-icon-code-xml"),
        "globe" => say!("repositories-icon-globe"),
        "server" => say!("repositories-icon-server"),
        "book-open" => say!("repositories-icon-book-open"),
        "flask-conical" => say!("repositories-icon-flask-conical"),
        "users" => say!("repositories-icon-users"),
        "wrench" => say!("repositories-icon-wrench"),
        "star" => say!("repositories-icon-star"),
        "heart" => say!("repositories-icon-heart"),
        other => other,
    }
}

/// The words over a field.
fn caption(text: &'static str, cx: &App) -> Div {
    let _ = cx;
    div()
        .h(px(CAPTION))
        .flex()
        .items_end()
        .pb(px(4.))
        .text_sm()
        .child(text)
}

/// The repositories of an answer of `workspace.scan` or
/// `workspace.import_code_workspace`, and what they left out and why. A
/// repository is a path, or an object with one and its remote; what was
/// left out is a path, or one with the reason.
fn answer(value: &Value) -> (Vec<Candidate>, Vec<String>) {
    let list = |names: &[&str]| -> Vec<Value> {
        names
            .iter()
            .find_map(|name| value.get(*name).and_then(Value::as_array))
            .cloned()
            .unwrap_or_default()
    };
    let found = list(&["found", "repositories", "paths", "folders"])
        .iter()
        .filter_map(|one| match one {
            Value::String(path) => Some(Candidate {
                path: path.clone(),
                remote_url: None,
            }),
            Value::Object(_) => Some(Candidate {
                path: one.get("path")?.as_str()?.to_owned(),
                remote_url: one
                    .get("remote_url")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            }),
            _ => None,
        })
        .collect();
    let skipped = list(&["skipped", "missing"])
        .iter()
        .filter_map(|one| match one {
            Value::String(path) => Some(path.clone()),
            Value::Object(_) => {
                let path = one.get("path").and_then(Value::as_str).unwrap_or("?");
                match one
                    .get("why")
                    .or_else(|| one.get("reason"))
                    .and_then(Value::as_str)
                {
                    Some(why) => Some(format!("{path} ({why})")),
                    None => Some(path.to_owned()),
                }
            }
            _ => None,
        })
        .collect();
    (found, skipped)
}

/// The layout a tab of this repository has, as the engine keeps a
/// workspace's.
pub(super) fn layout_of(state: &RepoState) -> Layout {
    Layout {
        left_width: state.left,
        right_width: state.right,
        left_panel: state.left_panel,
        right_panel: state.right_panel,
        closed_sections: state.closed_sections.clone(),
        hidden_sections: state.hidden_sections.clone(),
        section_order: state.section_order.clone(),
        section_sizes: state.section_sizes.clone(),
        maximised_section: state.maximised_section.clone(),
    }
}

impl Render for WorkspaceForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme().clone();
        let waits = self.waits_for(cx);
        let busy = self.busy;
        let new = self.making == Making::New;
        let refused = self.refused.clone();
        let title = if new {
            say!("repositories-new-workspace")
        } else {
            say!("repositories-form-edit")
        };
        let submit = cx.entity().downgrade();
        let title_row = h_flex()
            .flex_none()
            .h(px(TITLE))
            .pl(px(TITLE_LEFT))
            .pr(px(CLOSE_RIGHT))
            .items_center()
            .justify_between()
            .rounded_t(theme.radius_lg)
            .bg(theme.sidebar)
            .border_b_1()
            .border_color(theme.border)
            .child(div().text_lg().child(title))
            .child(
                Button::new("workspace-form-close")
                    .accessibility_label(say!("repositories-form-close"))
                    .ghost()
                    .small()
                    .icon(IconName::Close)
                    .tooltip(say!("repositories-form-close"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            );
        let first_row = h_flex()
            .w_full()
            .gap(px(10.))
            .items_end()
            .child(self.icon_box(cx))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .child(caption(say!("repositories-form-name"), cx))
                    .child(
                        Input::new(&self.name)
                            .id("workspace-name")
                            .h(px(FIELD))
                            .w_full(),
                    ),
            )
            .child(
                v_flex()
                    .flex_none()
                    .child(caption(say!("repositories-colour"), cx))
                    .child(self.colour_list(cx)),
            );
        let description = v_flex()
            .w_full()
            .child(caption(say!("repositories-form-description"), cx))
            .child(Textarea::new(&self.description).h(px(DESCRIPTION)));
        let go = Button::new("workspace-form-go")
            .primary()
            .small()
            .label(if new {
                say!("repositories-form-create")
            } else {
                say!("repositories-form-save")
            })
            .loading(busy)
            .disabled(waits.is_some() || busy)
            .when_some(waits, |button, waits| {
                button.tooltip(say!("repositories-form-needs", waits))
            })
            .on_click(move |_, window, cx| {
                let _ = submit.update(cx, |this, cx| this.submit(window, cx));
            });
        v_flex()
            .id("workspace-form")
            .w_full()
            .child(title_row)
            .child(
                v_flex()
                    .w_full()
                    .px(px(SIDE))
                    .pt(px(SIDE))
                    .pb(px(SIDE))
                    .gap(px(GAP))
                    .child(first_row)
                    .child(description)
                    .when(new, |body| body.child(self.repositories(window, cx)))
                    .child(self.defaults(cx))
                    .when_some(refused, |body, refused| {
                        body.child(
                            div()
                                .id("workspace-refused")
                                .text_sm()
                                .text_color(theme.danger)
                                .child(refused)
                                .test_support(),
                        )
                    })
                    .child(h_flex().w_full().justify_end().child(go)),
            )
    }
}

impl RepositoriesView {
    /// Open the dialog that makes a workspace, or edits one.
    pub fn open_form(&mut self, making: Making, window: &mut Window, cx: &mut Context<Self>) {
        let existing = match &making {
            Making::Edit(id) => match self.workspace(id) {
                Some(listed) => Some(listed.workspace.clone()),
                None => return,
            },
            Making::New => None,
        };
        let taken: Vec<String> = self
            .workspaces
            .iter()
            .map(|listed| listed.workspace.name.clone())
            .collect();
        // The layout of the tab on show, to be kept for the workspace.
        let current = {
            let state = &cx.global::<Prefs>().state;
            state.open.get(state.active).and_then(|path| {
                let kept = state.repos.get(&path.to_string_lossy().into_owned())?;
                let layout = layout_of(kept);
                (!layout.is_empty()).then(|| (crate::shell::repo_title(path), layout))
            })
        };
        let page = cx.entity().downgrade();
        let form = cx.new(|cx| {
            WorkspaceForm::new(
                making,
                page.clone(),
                existing.as_ref(),
                taken,
                current,
                window,
                cx,
            )
        });
        self.form = Some(form.clone());
        let first = form.read(cx).name.clone();
        window.open_dialog(cx, move |dialog, _, _| {
            let page = page.clone();
            dialog
                .close_button(false)
                .p_0()
                .gap_0()
                .width(px(WIDTH))
                .margin_top(px(TOP))
                .on_ok({
                    let form = form.clone();
                    move |_, window, cx| {
                        form.update(cx, |form, cx| form.submit(window, cx));
                        false
                    }
                })
                .on_close(move |_, _, cx| {
                    let _ = page.update(cx, |page, _| page.form = None);
                })
                .child(form.clone())
        });
        first.update(cx, |state, cx| state.focus(window, cx));
    }

    /// The dialog that makes or edits a workspace, while it is open.
    pub fn form(&self) -> Option<Entity<WorkspaceForm>> {
        self.form.clone()
    }
}

/// Repositories for a new workspace, by their folders.
pub fn candidates(paths: &[PathBuf]) -> Vec<Candidate> {
    paths
        .iter()
        .map(|path| Candidate {
            path: path.display().to_string(),
            remote_url: None,
        })
        .collect()
}

/// A workspace's repository, as the dialog takes it in.
impl From<&WorkspaceRepository> for Candidate {
    fn from(repository: &WorkspaceRepository) -> Self {
        Self {
            path: repository.path.clone(),
            remote_url: repository.remote_url.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Candidate, answer, layout_of};
    use crate::prefs::RepoState;
    use serde_json::json;

    #[test]
    fn what_a_scan_answers_is_read_in_either_shape() {
        let (found, skipped) = answer(&json!({
            "found": ["/w/a", { "path": "/w/b", "remote_url": "https://h/b.git" }, 7, {}],
            "skipped": ["/w/c", { "path": "/w/d", "why": "a symbolic link" }, null],
        }));
        assert_eq!(
            found,
            [
                Candidate {
                    path: "/w/a".to_owned(),
                    remote_url: None
                },
                Candidate {
                    path: "/w/b".to_owned(),
                    remote_url: Some("https://h/b.git".to_owned())
                }
            ]
        );
        assert_eq!(skipped, ["/w/c", "/w/d (a symbolic link)"]);
        let (found, skipped) = answer(&json!({ "folders": ["/x"] }));
        assert_eq!(found.len(), 1);
        assert!(skipped.is_empty());
        assert_eq!(answer(&json!(null)), (Vec::new(), Vec::new()));
    }

    #[test]
    fn a_tab_s_layout_is_kept_as_a_workspace_s() {
        let state = RepoState {
            left: Some(280.),
            right_panel: Some(false),
            hidden_sections: vec!["tags".to_owned()],
            ..RepoState::default()
        };
        let layout = layout_of(&state);
        assert_eq!(layout.left_width, Some(280.));
        assert_eq!(layout.right_panel, Some(false));
        assert_eq!(layout.hidden_sections, ["tags"]);
        assert!(layout_of(&RepoState::default()).is_empty());
    }
}
