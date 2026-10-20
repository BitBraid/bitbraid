//! A workspace's section of the page of repositories: its heading in the
//! workspace's colour with its icon, name, how many repositories it has
//! and a menu, then a row per repository with a box to tick, its name,
//! its remote, what git says of it, and what can be done with it here.
//!
//! At the heading's right: add repositories, fetch, pull with its way of
//! pulling, open them all and a menu (hide, tick all or none, open, edit,
//! the colour, delete). Ticking rows turns the heading's buttons into a
//! bar whose buttons name how many they act on (`bulk.rs`). Besides: a
//! column of remotes whose width is dragged and kept, a repository found
//! missing that is located or cloned from its row, and the workspace
//! written to a file to be taken in elsewhere.
//!
//! What a workspace is and how its file is kept is the engine's
//! (`workspace.*`); nothing here changes a repository.
//!
//! Sources: none (a layout of this application's own).

use std::path::{Path, PathBuf};

use bitbraid_engine::workspace::{Workspace, WorkspaceRepository};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    menu::{DropdownMenu, PopupMenu, PopupMenuItem},
    tooltip::Tooltip,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use serde::Deserialize;
use serde_json::json;

use super::{
    ColumnGrip, GRIP_WIDTH, GripChip, Making, NAME_LEFT, ROW_BUTTON, ROW_BUTTON_GAP, ROW_HEIGHT,
    ROW_RIGHT, ROWS_BOTTOM, ROWS_TOP, RepositoriesEvent, RepositoriesView, Section, TICK_LEFT,
    chip, nothing, star_button,
};
use crate::{
    notices::Notice,
    prefs::Prefs,
    session::{Reply, run_standalone},
    zoom::px,
};

/// The colours a workspace can be given: ours, by a word each, kept as
/// the engine keeps a colour (`#rrggbb`). None is the heading's own.
pub const COLOURS: [(bitbraid_strings::Message, &str); 10] = [
    (message!("repositories-colour-blue"), "#4f8fd9"),
    (message!("repositories-colour-teal"), "#3aa6a0"),
    (message!("repositories-colour-green"), "#5aa35a"),
    (message!("repositories-colour-olive"), "#9c9a3c"),
    (message!("repositories-colour-amber"), "#d9a34f"),
    (message!("repositories-colour-orange"), "#d97c4f"),
    (message!("repositories-colour-red"), "#d65c5c"),
    (message!("repositories-colour-rose"), "#d4588f"),
    (message!("repositories-colour-violet"), "#9563d6"),
    (message!("repositories-colour-slate"), "#7b8794"),
];

/// The icons a workspace can be given, by the names the engine keeps
/// (lowercase words and dashes). The first is a workspace's when it was
/// given none.
pub const ICONS: [&str; 12] = [
    "layers",
    "folder",
    "package",
    "code-xml",
    "globe",
    "server",
    "book-open",
    "flask-conical",
    "users",
    "wrench",
    "star",
    "heart",
];

/// The icon of a name of [`ICONS`]; the first one's for any other.
pub fn icon(name: Option<&str>) -> Icon {
    use gpui_kit::assets::IconName as More;
    match name.unwrap_or_default() {
        "folder" => Icon::new(IconName::Folder),
        "package" => Icon::new(More::Package),
        "code-xml" => Icon::new(More::CodeXml),
        "globe" => Icon::new(IconName::Globe),
        "server" => Icon::new(More::Server),
        "book-open" => Icon::new(IconName::BookOpen),
        "flask-conical" => Icon::new(More::FlaskConical),
        "users" => Icon::new(More::Users),
        "wrench" => Icon::new(More::Wrench),
        "star" => Icon::new(IconName::Star),
        "heart" => Icon::new(IconName::Heart),
        _ => Icon::new(More::Layers),
    }
}

/// A colour as the engine keeps it (`#rrggbb`), when it is one.
pub fn colour(text: Option<&str>) -> Option<Hsla> {
    let digits = text?.trim().strip_prefix('#')?;
    if digits.len() != 6 || !digits.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(digits, 16)
        .ok()
        .map(|value| rgb(value).into())
}

/// What a colour is called among [`COLOURS`], when it is one of them.
pub fn colour_name(text: Option<&str>) -> Option<&'static str> {
    let text = text?;
    COLOURS
        .iter()
        .find(|(_, hex)| hex.eq_ignore_ascii_case(text))
        .map(|(name, _)| name.text())
}

/// A workspace as `workspace.list` gives it: what is kept of it, and
/// which of its repositories are not where it says.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Listed {
    /// The workspace.
    #[serde(flatten)]
    pub workspace: Workspace,
    /// The paths of its repositories whose folders are not there.
    #[serde(default)]
    pub missing: Vec<String>,
}

impl Listed {
    /// A workspace as the page would list it, its missing repositories
    /// looked for on the disk: for workspaces that are handed in.
    pub fn of(workspace: Workspace) -> Self {
        let missing = workspace
            .repositories
            .iter()
            .filter(|repository| !repository.is_present())
            .map(|repository| repository.path.clone())
            .collect();
        Self { workspace, missing }
    }
}

/// The workspaces of an answer of `workspace.list`; any that does not
/// read is passed over.
pub fn listed(value: &serde_json::Value) -> Vec<Listed> {
    value
        .get("workspaces")
        .and_then(serde_json::Value::as_array)
        .map(|all| {
            all.iter()
                .filter_map(|one| serde_json::from_value::<Listed>(one.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// A remote's address as a row shows it: without the scheme, a user's
/// name and `.git`.
pub(super) fn short_remote(url: &str) -> String {
    let rest = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
    let rest = rest.rsplit_once('@').map(|(_, rest)| rest).unwrap_or(rest);
    let rest = rest.strip_suffix(".git").unwrap_or(rest);
    rest.replacen(
        ':',
        "/",
        usize::from(!url.contains("://") && !url.starts_with('/')),
    )
}

/// The tint of a heading in a workspace's colour: at rest, and under the
/// pointer.
fn tint(colour: Option<Hsla>, cx: &App) -> Option<(Hsla, Hsla)> {
    let theme = cx.theme();
    let colour = colour?;
    Some((
        theme.secondary.blend(colour.opacity(0.16)),
        theme.secondary.blend(colour.opacity(0.32)),
    ))
}

impl RepositoriesView {
    /// The workspace of an id.
    pub(super) fn workspace(&self, id: &str) -> Option<&Listed> {
        self.workspaces
            .iter()
            .find(|listed| listed.workspace.id == id)
    }

    /// The paths of a workspace's rows that are ticked, in its order.
    pub fn ticked_in(&self, id: &str) -> Vec<String> {
        let Some(listed) = self.workspace(id) else {
            return Vec::new();
        };
        listed
            .workspace
            .repositories
            .iter()
            .filter(|repository| {
                self.ticked
                    .contains(&(id.to_owned(), repository.path.clone()))
            })
            .map(|repository| repository.path.clone())
            .collect()
    }

    /// What fetching and pulling act on: the targets whose folders are
    /// there.
    pub(super) fn present_targets(&self, id: &str) -> Vec<String> {
        let missing = self
            .workspace(id)
            .map(|listed| listed.missing.clone())
            .unwrap_or_default();
        self.targets(id)
            .into_iter()
            .filter(|path| !missing.contains(path))
            .collect()
    }

    /// What the heading's buttons act on: the ticked rows, or every row
    /// when none is.
    pub(super) fn targets(&self, id: &str) -> Vec<String> {
        let ticked = self.ticked_in(id);
        if !ticked.is_empty() {
            return ticked;
        }
        self.workspace(id)
            .map(|listed| {
                listed
                    .workspace
                    .repositories
                    .iter()
                    .map(|repository| repository.path.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Tick a row, or take its tick away.
    pub fn tick(&mut self, id: &str, path: &str, on: bool, cx: &mut Context<Self>) {
        let key = (id.to_owned(), path.to_owned());
        if on {
            self.ticked.insert(key);
        } else {
            self.ticked.remove(&key);
        }
        cx.notify();
    }

    /// Tick every row of a workspace, or none.
    pub fn tick_all(&mut self, id: &str, on: bool, cx: &mut Context<Self>) {
        self.ticked.retain(|(ticked, _)| ticked != id);
        if on && let Some(listed) = self.workspace(id) {
            let all: Vec<(String, String)> = listed
                .workspace
                .repositories
                .iter()
                .map(|repository| (id.to_owned(), repository.path.clone()))
                .collect();
            self.ticked.extend(all);
        }
        cx.notify();
    }

    /// A workspace's section: its heading, and its rows unless it is
    /// folded.
    pub(super) fn workspace_section(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        let Some(listed) = self.workspace(id).cloned() else {
            return div().into_any_element();
        };
        let theme = cx.theme().clone();
        let section = Section::Workspace(id.to_owned());
        let folded = self.folded.contains(&section.key());
        let shown_colour = colour(listed.workspace.colour.as_deref());
        let rows: Vec<&WorkspaceRepository> = listed
            .workspace
            .repositories
            .iter()
            .filter(|repository| self.searched(&repository.name(), Path::new(&repository.path)))
            .collect();
        let follows = listed.workspace.follows_folder.clone();
        let lead = h_flex()
            .gap(px(10.))
            .items_center()
            .child(
                icon(listed.workspace.icon.as_deref())
                    .size(px(16.))
                    .text_color(shown_colour.unwrap_or(theme.foreground)),
            )
            .child(Self::name_and_count(
                listed.workspace.name.clone(),
                listed.workspace.repositories.len(),
                cx,
            ))
            .child(
                h_flex()
                    .id(SharedString::from(format!("mark-{id}")))
                    .gap(px(4.))
                    .items_center()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::FolderOpen).size(px(14.)))
                    .child(say!("repositories-local"))
                    .tooltip(move |window, cx| {
                        Tooltip::new(match &follows {
                            Some(folder) => say!("repositories-local-following", folder),
                            None => say!("repositories-local-tip").to_owned(),
                        })
                        .build(window, cx)
                    }),
            )
            .into_any_element();
        let controls = self.workspace_controls(&listed, cx);
        let header = self.heading(&section, lead, tint(shown_colour, cx), controls, cx);
        let body = (!folded).then(|| {
            if rows.is_empty() {
                nothing(&theme)
            } else {
                v_flex_rows(
                    rows.iter()
                        .map(|repository| self.workspace_row(&listed, repository, cx))
                        .collect(),
                )
            }
        });
        gpui_kit::component::v_flex()
            .w_full()
            .flex_none()
            .mb(px(super::HEADER_GAP))
            .child(header)
            .children(body)
            .into_any_element()
    }

    /// The buttons at the right of a workspace's heading: with nothing
    /// ticked and nothing running, icons that act on every repository and
    /// the menu; else the bar that names how many it acts on.
    fn workspace_controls(&self, listed: &Listed, cx: &mut Context<Self>) -> AnyElement {
        let id = listed.workspace.id.clone();
        if !self.ticked_in(&id).is_empty() || self.runs.contains_key(&id) {
            return self.bulk_bar(listed, cx);
        }
        let icon_button = |what: &str, icon: Icon, tip: &'static str| {
            Button::new(SharedString::from(format!("{what}-{id}")))
                .accessibility_label(tip)
                .ghost()
                .xsmall()
                .w(px(ROW_BUTTON))
                .icon(icon)
                .tooltip(tip)
        };
        let (add_id, fetch_id, pull_id, open_id) = (id.clone(), id.clone(), id.clone(), id.clone());
        h_flex()
            .flex_none()
            .gap(px(ROW_BUTTON_GAP))
            .pr(px(ROW_RIGHT))
            .items_center()
            .child(
                icon_button("add", Icon::new(IconName::Plus), say!("repositories-add")).on_click(
                    cx.listener(move |this, _, window, cx| {
                        this.add_to_workspace(&add_id, window, cx)
                    }),
                ),
            )
            .child(
                icon_button(
                    "fetch",
                    Icon::new(IconName::RefreshCw),
                    say!("repositories-fetch-all"),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    let paths = this.present_targets(&fetch_id);
                    this.start_bulk(&fetch_id, super::bulk::Kind::Fetch, paths, window, cx)
                })),
            )
            .child(self.pull_buttons(&pull_id, None, cx))
            .child(
                icon_button(
                    "open-all",
                    Icon::new(IconName::ExternalLink),
                    say!("repositories-open-all"),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    let paths = this.targets(&open_id);
                    this.open_workspace_repositories(&open_id, paths, cx)
                })),
            )
            .child(self.workspace_menu(listed, cx))
            .into_any_element()
    }

    /// The pull button of a heading and the caret beside it, which picks
    /// how it pulls; labelled with how many when `count` says.
    pub(super) fn pull_buttons(
        &self,
        id: &str,
        count: Option<usize>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mode = pull_mode(cx);
        let page = cx.entity().downgrade();
        let for_pull = id.to_owned();
        let pull = Button::new(SharedString::from(format!("pull-{id}")))
            .ghost()
            .xsmall()
            .icon(gpui_kit::assets::IconName::ArrowDownToLine)
            .tooltip(pull_tip(&mode))
            .map(|button| match count {
                Some(count) => button
                    .outline()
                    .label(say!("repositories-bulk-pull", count)),
                None => button.w(px(ROW_BUTTON)),
            })
            .on_click(cx.listener(move |this, _, window, cx| {
                let paths = this.present_targets(&for_pull);
                this.start_bulk(&for_pull, super::bulk::Kind::Pull, paths, window, cx)
            }));
        let caret = Button::new(SharedString::from(format!("pull-mode-{id}")))
            .ghost()
            .xsmall()
            .w(px(16.))
            .icon(Icon::new(IconName::ChevronDown).size(px(11.)))
            .tooltip(say!("repositories-pull-mode"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let mut menu = menu.label(say!("repositories-pull-mode-heading"));
                for (word, label) in crate::views::workspace::PULL_MODES {
                    let page = page.clone();
                    menu = menu.item(
                        PopupMenuItem::new(label.text())
                            .checked(mode == word)
                            .on_click(move |_, _, cx| {
                                let _ = page.update(cx, |_, cx| {
                                    let prefs = cx.global_mut::<Prefs>();
                                    prefs.state.repositories_page.pull_mode = Some(word.to_owned());
                                    prefs.save_state();
                                    cx.notify();
                                });
                            }),
                    );
                }
                menu
            });
        h_flex()
            .flex_none()
            .items_center()
            .child(pull)
            .child(caret)
            .into_any_element()
    }

    /// The menu at the right of a workspace's heading.
    pub(super) fn workspace_menu(&self, listed: &Listed, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity().downgrade();
        let id = listed.workspace.id.clone();
        let ticked = self.ticked_in(&id).len();
        let missing = listed.missing.clone();
        let can_clone = listed.workspace.repositories.iter().any(|repository| {
            missing.contains(&repository.path) && repository.remote_url.is_some()
        });
        let current = listed.workspace.colour.clone();
        Button::new(SharedString::from(format!("menu-workspace-{id}")))
            .accessibility_label(say!("repositories-more"))
            .ghost()
            .xsmall()
            .w(px(ROW_BUTTON))
            .icon(IconName::EllipsisVertical)
            .tooltip(say!("repositories-more"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, window, cx| {
                let entry = |label: String, icon: Icon, run: Box<MenuRun>| {
                    let page = page.clone();
                    let run = std::rc::Rc::new(run);
                    PopupMenuItem::new(label)
                        .icon(icon)
                        .on_click(move |_, window, cx| {
                            let run = run.clone();
                            let _ = page.update(cx, |this, cx| run(this, window, cx));
                        })
                };
                let id = id.clone();
                let (a, b, c, d, e, f, g, h, i) = (
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                    id.clone(),
                );
                let mut menu = menu
                    .item(entry(
                        say!("repositories-hide").to_owned(),
                        Icon::new(IconName::EyeOff),
                        Box::new(move |this, _, cx| {
                            this.hide_section(&Section::Workspace(a.clone()), true, cx)
                        }),
                    ))
                    .separator()
                    .item(entry(
                        say!("repositories-tick-all").to_owned(),
                        Icon::new(gpui_kit::assets::IconName::SquareCheckBig),
                        Box::new(move |this, _, cx| this.tick_all(&b, true, cx)),
                    ))
                    .item(entry(
                        say!("repositories-tick-none").to_owned(),
                        Icon::new(IconName::Square),
                        Box::new(move |this, _, cx| this.tick_all(&c, false, cx)),
                    ))
                    .separator()
                    .item(entry(
                        if ticked > 0 {
                            say!("repositories-open-ticked-in-tabs", ticked)
                        } else {
                            say!("repositories-open-all-in-tabs").to_owned()
                        },
                        Icon::new(IconName::ExternalLink),
                        Box::new(move |this, _, cx| {
                            let paths = this.targets(&d);
                            this.open_workspace_repositories(&d, paths, cx)
                        }),
                    ));
                if can_clone {
                    menu = menu.item(entry(
                        say!("repositories-clone-missing").to_owned(),
                        Icon::new(gpui_kit::assets::IconName::ArrowDownToLine),
                        Box::new(move |this, window, cx| this.clone_missing(&e, window, cx)),
                    ));
                }
                if !missing.is_empty() {
                    let first = missing.first().cloned().unwrap_or_default();
                    menu = menu.item(entry(
                        say!("repositories-locate-missing").to_owned(),
                        Icon::new(IconName::Search),
                        Box::new(move |this, window, cx| this.locate(&f, &first, window, cx)),
                    ));
                }
                let page_for_colours = page.clone();
                let current = current.clone();
                let mut menu = menu
                    .separator()
                    .item(entry(
                        say!("repositories-edit").to_owned(),
                        Icon::new(gpui_kit::assets::IconName::Pencil),
                        Box::new(move |this, window, cx| {
                            this.open_form(Making::Edit(g.clone()), window, cx)
                        }),
                    ))
                    .submenu_with_icon(
                        Some(Icon::new(IconName::Palette)),
                        say!("repositories-colour"),
                        window,
                        cx,
                        move |menu: PopupMenu, _, _| {
                            colour_menu(menu, &page_for_colours, &h, current.as_deref())
                        },
                    )
                    .item(entry(
                        say!("repositories-export").to_owned(),
                        Icon::new(gpui_kit::assets::IconName::FileOutput),
                        Box::new(move |this, window, cx| this.export_workspace(&i, window, cx)),
                    ))
                    .separator();
                // Deleting first, then taking the ticked ones out.
                let for_delete = id.clone();
                menu = menu.item(entry(
                    say!("repositories-delete").to_owned(),
                    Icon::new(gpui_kit::assets::IconName::Trash),
                    Box::new(move |this, window, cx| {
                        this.ask_delete_workspace(&for_delete, window, cx)
                    }),
                ));
                if ticked > 0 {
                    menu = menu.item(entry(
                        say!("repositories-remove-ticked", ticked),
                        Icon::new(gpui_kit::assets::IconName::Trash),
                        Box::new(move |this, window, cx| {
                            let paths = this.ticked_in(&id);
                            this.remove_from_workspace(&id, paths, window, cx)
                        }),
                    ));
                }
                menu
            })
            .into_any_element()
    }

    /// A repository's row in a workspace's section.
    fn workspace_row(
        &self,
        listed: &Listed,
        repository: &WorkspaceRepository,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let id = listed.workspace.id.clone();
        let name = repository.name();
        let key = format!("ws-{id}-{name}");
        let element = |what: &str| SharedString::from(format!("{what}-{key}"));
        let path = PathBuf::from(&repository.path);
        let missing = listed.missing.contains(&repository.path);
        let ticked = self.ticked.contains(&(id.clone(), repository.path.clone()));
        let favourite = cx.global::<Prefs>().state.favourites.contains(&path);
        let (name_width, remote_width) = Self::columns(cx);
        let page = cx.entity().downgrade();
        let (tick_id, tick_path) = (id.clone(), repository.path.clone());
        let (open_path, name_path, editor_path) = (path.clone(), path.clone(), path.clone());
        let remote = repository
            .remote_url
            .clone()
            .map(|url| (short_remote(&url), url));
        let grip = |what: ColumnGrip, element: SharedString| {
            div()
                .id(element)
                .w(px(GRIP_WIDTH))
                .h_full()
                .flex_none()
                .cursor_col_resize()
                .on_drag(what, |_, _, _, cx| cx.new(|_| GripChip))
                .test_support()
        };
        let chips = match self.outcome_chips(&id, &repository.path, &key, cx) {
            Some(chips) => chips,
            None if missing => vec![
                chip("problem", &key, cx)
                    .text_color(theme.danger)
                    .child(Icon::new(IconName::TriangleAlert).size(px(11.)))
                    .child(say!("repositories-not-found"))
                    .tooltip({
                        let path = repository.path.clone();
                        move |window, cx| {
                            Tooltip::new(say!("repositories-not-there", path = path.as_str()))
                                .build(window, cx)
                        }
                    })
                    .into_any_element(),
            ],
            None => self.chips(&key, &path, cx),
        };
        h_flex()
            .id(element("repo"))
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .items_center()
            .rounded(theme.radius)
            .hover(|row| row.bg(theme.list_hover))
            .child(
                div().pl(px(TICK_LEFT)).w(px(NAME_LEFT)).flex_none().child(
                    Checkbox::new(element("tick"))
                        .accessibility_label(say!(
                            "repositories-choose",
                            path = repository.path.as_str()
                        ))
                        .checked(ticked)
                        .on_click(move |on: &bool, _, cx: &mut App| {
                            let on = *on;
                            let _ =
                                page.update(cx, |this, cx| this.tick(&tick_id, &tick_path, on, cx));
                        }),
                ),
            )
            .child(
                div()
                    .id(element("name"))
                    .w(px(name_width - GRIP_WIDTH))
                    .flex_none()
                    .pr(px(10.))
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .cursor_pointer()
                    .when(missing, |name| name.text_color(theme.muted_foreground))
                    .child(name.clone())
                    .tooltip({
                        let path = repository.path.clone();
                        move |window, cx| Tooltip::new(path.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.open_workspace_repositories(
                            &id_of(&name_path, this),
                            vec![name_path.display().to_string()],
                            cx,
                        )
                    }))
                    .test_support(),
            )
            .child(grip(ColumnGrip::Name, element("grip-name")))
            .child(
                div()
                    .id(element("remote"))
                    .w(px(remote_width - GRIP_WIDTH))
                    .flex_none()
                    .pr(px(10.))
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .children(remote.clone().map(|(short, _)| short))
                    .when_some(remote.map(|(_, url)| url), |cell, url| {
                        cell.tooltip(move |window, cx| Tooltip::new(url.clone()).build(window, cx))
                    })
                    .test_support(),
            )
            .child(grip(ColumnGrip::Remote, element("grip-remote")))
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(5.))
                    .overflow_hidden()
                    .children(chips),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(ROW_BUTTON_GAP))
                    .pr(px(ROW_RIGHT))
                    .child(
                        Button::new(element("open"))
                            .accessibility_label(say!("repositories-open"))
                            .ghost()
                            .xsmall()
                            .w(px(ROW_BUTTON))
                            .icon(IconName::ExternalLink)
                            .tooltip(say!("repositories-open"))
                            .disabled(missing)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.open_workspace_repositories(
                                    &id_of(&open_path, this),
                                    vec![open_path.display().to_string()],
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new(element("editor"))
                            .accessibility_label(say!("repositories-open-in-editor"))
                            .ghost()
                            .xsmall()
                            .w(px(ROW_BUTTON))
                            .icon(gpui_kit::assets::IconName::CodeXml)
                            .tooltip(say!("repositories-open-in-editor"))
                            .disabled(missing)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_in_editor(vec![editor_path.clone()], window, cx)
                            })),
                    )
                    .child(star_button(element("star"), favourite, path.clone(), cx))
                    .child(self.row_menu(listed, repository, &key, cx)),
            )
            .test_support()
            .into_any_element()
    }

    /// The menu at the right of a workspace's row.
    fn row_menu(
        &self,
        listed: &Listed,
        repository: &WorkspaceRepository,
        key: &str,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity().downgrade();
        let id = listed.workspace.id.clone();
        let path = repository.path.clone();
        let missing = listed.missing.contains(&path);
        let clonable = missing && repository.remote_url.is_some();
        Button::new(SharedString::from(format!("menu-{key}")))
            .accessibility_label(say!("repositories-more"))
            .ghost()
            .xsmall()
            .w(px(ROW_BUTTON))
            .icon(IconName::EllipsisVertical)
            .tooltip(say!("repositories-more"))
            .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                let entry = |label: &'static str, what: RowDo| {
                    let (page, id, path) = (page.clone(), id.clone(), path.clone());
                    PopupMenuItem::new(label).on_click(move |_, window, cx| {
                        let (id, path) = (id.clone(), path.clone());
                        let _ = page.update(cx, |this, cx| match what {
                            RowDo::Open => this.open_workspace_repositories(&id, vec![path], cx),
                            RowDo::Editor => {
                                this.open_in_editor(vec![PathBuf::from(path)], window, cx)
                            }
                            RowDo::Locate => this.locate(&id, &path, window, cx),
                            RowDo::Clone => {
                                let paths = vec![path];
                                this.start_bulk(&id, super::bulk::Kind::Clone, paths, window, cx)
                            }
                            RowDo::Remove => {
                                this.remove_from_workspace(&id, vec![path], window, cx)
                            }
                        });
                    })
                };
                let mut menu = menu;
                if missing {
                    menu = menu.item(entry(say!("repositories-row-locate"), RowDo::Locate));
                    if clonable {
                        menu = menu.item(entry(say!("repositories-clone"), RowDo::Clone));
                    }
                } else {
                    menu = menu
                        .item(entry(say!("repositories-open"), RowDo::Open))
                        .item(entry(say!("repositories-open-in-editor"), RowDo::Editor));
                }
                menu.separator()
                    .item(entry(say!("repositories-row-remove"), RowDo::Remove))
            })
            .into_any_element()
    }

    /// Open repositories of a workspace in tabs of their own, the ones
    /// that are not there left out.
    pub fn open_workspace_repositories(
        &mut self,
        id: &str,
        paths: Vec<String>,
        cx: &mut Context<Self>,
    ) {
        let missing = self
            .workspace(id)
            .map(|listed| listed.missing.clone())
            .unwrap_or_default();
        let paths: Vec<PathBuf> = paths
            .into_iter()
            .filter(|path| !missing.contains(path))
            .map(PathBuf::from)
            .collect();
        if paths.is_empty() {
            return;
        }
        cx.emit(RepositoriesEvent::OpenMany {
            paths,
            workspace: id.to_owned(),
        });
    }

    /// Open folders in the editor the preferences name.
    pub(super) fn open_in_editor(
        &mut self,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let editor = cx.global::<Prefs>().preferences.editor.clone();
        for path in paths {
            let run = run_standalone(
                "repo.open_editor",
                json!({ "command": editor, "path": path.display().to_string() }),
            );
            cx.spawn_in(window, async move |_, cx| {
                if let Ok(Reply::Error(err)) = run.reply.await {
                    let _ = cx.update(|window, cx| {
                        let said = if err.contains("no editor is named") {
                            say!("repositories-no-editor").to_owned()
                        } else {
                            sentence(&err)
                        };
                        crate::notices::push(window, cx, Notice::error(said));
                    });
                }
            })
            .detach();
        }
    }

    /// Run a workspace command that answers at once, say what went wrong
    /// if it did, and read the workspaces again; `then` is told the
    /// answer when it went well.
    pub(super) fn run_workspace_command(
        &mut self,
        id: &'static str,
        params: serde_json::Value,
        then: impl FnOnce(&mut Self, serde_json::Value, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let run = run_standalone(id, params);
        cx.spawn_in(window, async move |this, cx| {
            let reply = run.reply.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match reply {
                    Ok(Reply::Command(value)) => then(this, value, window, cx),
                    Ok(Reply::Error(err)) => {
                        crate::notices::push(window, cx, Notice::error(sentence(&err)))
                    }
                    _ => {}
                }
                this.look(window, cx);
            });
        })
        .detach();
    }

    /// Take repositories out of a workspace; their folders stay.
    pub fn remove_from_workspace(
        &mut self,
        id: &str,
        paths: Vec<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if paths.is_empty() {
            return;
        }
        for path in &paths {
            self.ticked.remove(&(id.to_owned(), path.clone()));
        }
        self.run_workspace_command(
            "workspace.remove",
            json!({ "id": id, "paths": paths }),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    /// Ask whether a workspace is to be deleted; its repositories stay.
    pub fn ask_delete_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self
            .workspace(id)
            .map(|listed| listed.workspace.name.clone())
        else {
            return;
        };
        let page = cx.entity().downgrade();
        let id = id.to_owned();
        crate::ask::show(
            crate::ask::Ask::new(say!("repositories-delete-ask", name = name.as_str())).danger(
                say!("repositories-delete"),
                move |_, window, cx| {
                    let id = id.clone();
                    let _ = page.update(cx, |this, cx| this.delete_workspace(&id, window, cx));
                },
            ),
            window,
            cx,
        );
    }

    /// Delete a workspace; its repositories stay as they are.
    pub fn delete_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.ticked.retain(|(ticked, _)| ticked != id);
        self.run_workspace_command(
            "workspace.delete",
            json!({ "id": id }),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    /// Give a workspace another colour, or its heading's own.
    pub fn set_colour(
        &mut self,
        id: &str,
        colour: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_workspace_command(
            "workspace.update",
            json!({ "id": id, "colour": colour.unwrap_or("") }),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    /// Ask for repositories to add to a workspace, and add them.
    fn add_to_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: true,
            prompt: Some(say!("repositories-add-prompt").into()),
        });
        let id = id.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.add_paths(&id, paths, window, cx)
                });
            }
        })
        .detach();
    }

    /// Add repositories to a workspace, by their folders.
    pub fn add_paths(
        &mut self,
        id: &str,
        paths: Vec<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let paths: Vec<String> = paths
            .iter()
            .map(|path| path.display().to_string())
            .collect();
        if paths.is_empty() {
            return;
        }
        self.run_workspace_command(
            "workspace.add",
            json!({ "id": id, "paths": paths }),
            |_, value, window, cx| {
                let already = value
                    .get("already")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                if already > 0 {
                    crate::notices::push(
                        window,
                        cx,
                        Notice::info(say!("repositories-already-in", already)),
                    );
                }
            },
            window,
            cx,
        );
    }

    /// Ask where a repository that is not where the workspace says is
    /// now, and point the workspace there.
    pub(super) fn locate(
        &mut self,
        id: &str,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = Path::new(path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_owned());
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(say!("repositories-locate-prompt", name).into()),
        });
        let (id, path) = (id.to_owned(), path.to_owned());
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(to) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.locate_at(&id, &path, &to, window, cx)
                });
            }
        })
        .detach();
    }

    /// Point a workspace's entry at the folder a repository is in now.
    pub fn locate_at(
        &mut self,
        id: &str,
        path: &str,
        to: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.run_workspace_command(
            "workspace.locate",
            json!({ "id": id, "path": path, "to": to.display().to_string() }),
            |_, _, _, _| {},
            window,
            cx,
        );
    }

    /// Clone a workspace's missing repositories where it says they are.
    fn clone_missing(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(listed) = self.workspace(id) else {
            return;
        };
        let ticked = self.ticked_in(id);
        let paths: Vec<String> = listed
            .workspace
            .repositories
            .iter()
            .filter(|repository| {
                listed.missing.contains(&repository.path)
                    && repository.remote_url.is_some()
                    && (ticked.is_empty() || ticked.contains(&repository.path))
            })
            .map(|repository| repository.path.clone())
            .collect();
        self.start_bulk(id, super::bulk::Kind::Clone, paths, window, cx);
    }

    /// Ask where a workspace's file goes, and write it there.
    fn export_workspace(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(name) = self
            .workspace(id)
            .map(|listed| listed.workspace.name.clone())
        else {
            return;
        };
        let start = dirs_home();
        let file_name = format!("{}.workspace.toml", file_safe(&name));
        let chosen = cx.prompt_for_new_path(&start, Some(&file_name));
        let id = id.to_owned();
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(file))) = chosen.await {
                let _ = this.update_in(cx, |this, window, cx| {
                    this.export_to(&id, &file, window, cx)
                });
            }
        })
        .detach();
    }

    /// Write a workspace's file.
    pub fn export_to(
        &mut self,
        id: &str,
        file: &Path,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = file.display().to_string();
        self.run_workspace_command(
            "workspace.export",
            json!({ "id": id, "file": shown, "overwrite": true }),
            move |_, _, window, cx| {
                crate::notices::push(
                    window,
                    cx,
                    Notice::success(say!("repositories-written", shown = shown.as_str())),
                )
            },
            window,
            cx,
        );
    }

    /// Ask for a workspace's file, and take the workspace in.
    pub(super) fn import_workspace(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let chosen = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(say!("repositories-import-prompt").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = chosen.await
                && let Some(file) = paths.into_iter().next()
            {
                let _ = this.update_in(cx, |this, window, cx| this.import_from(&file, window, cx));
            }
        })
        .detach();
    }

    /// Take in a workspace from its file: its repositories are looked for
    /// beside the file, where the file says they are.
    pub fn import_from(&mut self, file: &Path, window: &mut Window, cx: &mut Context<Self>) {
        self.run_workspace_command(
            "workspace.import",
            json!({ "file": file.display().to_string() }),
            |this, value, window, cx| {
                let name = value
                    .pointer("/workspace/name")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or(say!("repositories-the-workspace"))
                    .to_owned();
                let refused = value
                    .get("refused")
                    .and_then(serde_json::Value::as_array)
                    .map_or(0, Vec::len);
                let said = if refused > 0 {
                    Notice::warning(say!(
                        "repositories-imported-refused",
                        name = name.as_str(),
                        refused
                    ))
                } else {
                    Notice::success(say!("repositories-imported", name = name.as_str()))
                };
                crate::notices::push(window, cx, said);
                if let Some(id) = value
                    .pointer("/workspace/id")
                    .and_then(serde_json::Value::as_str)
                {
                    this.folded
                        .retain(|key| *key != Section::Workspace(id.to_owned()).key());
                }
            },
            window,
            cx,
        );
    }
}

/// What an entry of a workspace's menu does to the page.
type MenuRun = dyn Fn(&mut RepositoriesView, &mut Window, &mut Context<RepositoriesView>);

/// What a row's menu does.
#[derive(Debug, Clone, Copy)]
enum RowDo {
    Open,
    Editor,
    Locate,
    Clone,
    Remove,
}

/// The workspace a repository of a row is in, for opening it with that
/// workspace's defaults: the first that has it.
fn id_of(path: &Path, page: &RepositoriesView) -> String {
    let text = path.display().to_string();
    page.workspaces
        .iter()
        .find(|listed| {
            listed
                .workspace
                .repositories
                .iter()
                .any(|repository| repository.path == text)
        })
        .map(|listed| listed.workspace.id.clone())
        .unwrap_or_default()
}

/// The rows of a section.
fn v_flex_rows(rows: Vec<AnyElement>) -> AnyElement {
    gpui_kit::component::v_flex()
        .w_full()
        .pt(px(ROWS_TOP))
        .pb(px(ROWS_BOTTOM))
        .children(rows)
        .into_any_element()
}

/// The colours to pick from, the one a workspace has ticked.
fn colour_menu(
    menu: PopupMenu,
    page: &WeakEntity<RepositoriesView>,
    id: &str,
    current: Option<&str>,
) -> PopupMenu {
    let pick = |label: &'static str, colour: Option<&'static str>, menu: PopupMenu| {
        let (page, id) = (page.clone(), id.to_owned());
        let checked = match (colour, current) {
            (None, None) => true,
            (Some(colour), Some(current)) => colour.eq_ignore_ascii_case(current),
            _ => false,
        };
        let swatch = colour.and_then(|colour| self::colour(Some(colour)));
        menu.item(
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
                            .bg(swatch.unwrap_or(theme.secondary)),
                    )
                    .child(label)
            })
            .checked(checked)
            .on_click(move |_, window, cx| {
                let id = id.clone();
                let _ = page.update(cx, |this, cx| this.set_colour(&id, colour, window, cx));
            }),
        )
    };
    let mut menu = pick(say!("repositories-colour-none"), None, menu);
    for (label, hex) in COLOURS {
        menu = pick(label.text(), Some(hex), menu);
    }
    menu
}

/// How the pull buttons of the page pull: as chosen there, else as the
/// preferences say.
pub(super) fn pull_mode(cx: &App) -> String {
    let prefs = cx.global::<Prefs>();
    prefs
        .state
        .repositories_page
        .pull_mode
        .clone()
        .filter(|mode| {
            crate::views::workspace::PULL_MODES
                .iter()
                .any(|(word, _)| word == mode)
        })
        .unwrap_or_else(|| prefs.preferences.pull_mode.clone())
}

/// What the pull button says under the pointer.
fn pull_tip(mode: &str) -> &'static str {
    match mode {
        "ff_only" => say!("repositories-pull-ff-only"),
        "rebase" => say!("repositories-pull-rebase"),
        "merge" => say!("repositories-pull-merge"),
        _ => say!("repositories-pull-ff"),
    }
}

/// An error as a notification shows it: starting with a capital.
pub(super) fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// A name as part of a file's name: letters, digits, dashes and
/// underscores, the rest dashes.
fn file_safe(name: &str) -> String {
    let safe: String = name
        .chars()
        .take(64)
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    if safe.trim_matches('-').is_empty() {
        "workspace".to_owned()
    } else {
        safe
    }
}

/// Where a chooser of a file to write starts: the home folder.
fn dirs_home() -> PathBuf {
    bitbraid_core::paths::home().unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::{colour, colour_name, file_safe, short_remote};

    #[test]
    fn a_remote_is_shown_without_its_scheme_user_and_suffix() {
        assert_eq!(
            short_remote("https://github.com/example-org/tools.git"),
            "github.com/example-org/tools"
        );
        assert_eq!(
            short_remote("git@gitlab.com:group/project.git"),
            "gitlab.com/group/project"
        );
        assert_eq!(short_remote("ssh://git@host:2222/a/b"), "host:2222/a/b");
        assert_eq!(short_remote("/srv/git/beta.git"), "/srv/git/beta");
        assert_eq!(short_remote(""), "");
    }

    #[test]
    fn colours_are_read_as_the_engine_keeps_them() {
        assert!(colour(Some("#4f8fd9")).is_some());
        assert!(colour(Some("#4F8FD9")).is_some());
        for bad in ["4f8fd9", "#4f8fd", "#4f8fd9a", "#zzzzzz", "", "#é1234"] {
            assert!(colour(Some(bad)).is_none(), "{bad}");
        }
        assert!(colour(None).is_none());
        assert_eq!(colour_name(Some("#4F8FD9")), Some("Blue"));
        assert_eq!(colour_name(Some("#000000")), None);
    }

    #[test]
    fn a_file_name_is_made_of_a_workspace_name() {
        assert_eq!(file_safe("Team tools"), "Team-tools");
        assert_eq!(file_safe("../.."), "workspace");
        assert_eq!(file_safe("über_1"), "über_1");
    }
}
