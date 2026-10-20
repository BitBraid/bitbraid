//! The page of repositories, behind the first pinned tab left of the
//! tabs: every repository the application knows, in the sections it is
//! known in (the ones open in a tab, the favourites, the recent ones, each
//! workspace's, and all of them once), each on a row with the branch it
//! is on, how far that is ahead of and behind what it follows, and what
//! is uncommitted in it. From here a repository is browsed for, cloned or
//! made, opened, made a favourite or taken out of a list, and workspaces
//! are made, imported and worked on.
//!
//! Laid out as a heading, the buttons to begin with, a row with folding,
//! the search and the box for the changes, then the sections, which
//! scroll, are put in order by dragging their headings and are hidden
//! from their menus. A row says nothing of a repository until git has
//! been asked about it (`repo.glance`), which is done each time the page
//! is shown; the workspaces are read from their file then too
//! (`workspace.list`).
//!
//! Sources: git-status(1) (what the branch line and the short format
//! say).

pub mod bulk;
mod making;
pub mod sections;
mod workspaces;

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use bitbraid_engine::workspace::RepositoryOutcome;
use bitbraid_git::glance::{Glance, MAX_GLANCED};
use gpui_kit::component::{
    ActiveTheme, Icon, IconName, Sizable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::{ScrollableElement, ScrollbarAxis},
    tooltip::Tooltip,
    v_flex,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

use crate::prefs::{Prefs, RepoList};

pub use making::{Candidate, Making, WorkspaceForm, candidates};
pub use sections::Section;
pub use workspaces::{
    COLOURS, ICONS, Listed, colour as workspace_colour, icon as workspace_icon, listed,
};

/// From the sides of the window to the heading, the buttons and the row
/// under them.
const SIDE: f32 = 45.;
/// From the sides of the window to the sections.
const SECTION_SIDE: f32 = 48.;
/// Above the heading, and the heading's line.
const TOP: f32 = 16.;
const HEADING_LINE: f32 = 24.;
/// The buttons to begin with, under the heading.
const BUTTONS_TOP: f32 = 8.;
const BUTTON_HEIGHT: f32 = 30.;
/// The row with folding, the search and the box, under the buttons.
const TOOLS_TOP: f32 = 12.;
const TOOLS_HEIGHT: f32 = 34.;
/// From that row to the first section.
const SECTIONS_TOP: f32 = 20.;
/// The box for the changes, at the right end of that row, and the space
/// left of it.
const SUMMARY_WIDTH: f32 = 110.;
const SUMMARY_GAP: f32 = 14.;
/// A section's heading, and the space between two folded ones.
const HEADER_HEIGHT: f32 = 44.;
const HEADER_GAP: f32 = 3.;
/// A repository's row, and the space above and under the rows.
const ROW_HEIGHT: f32 = 34.;
const ROWS_TOP: f32 = 15.;
const ROWS_BOTTOM: f32 = 8.;
/// Under the words that say a section has nothing.
const EMPTY_BOTTOM: f32 = 14.;
/// From a section's left edge: its arrow, its name; a repository's box
/// to tick, its name, and what is known of it.
const ARROW_LEFT: f32 = 16.;
const TICK_LEFT: f32 = 41.;
const NAME_LEFT: f32 = 77.;
const NOTHING_LEFT: f32 = 31.;
/// The columns of a row: the names, and in a workspace's rows the
/// remotes, as wide as they start out and as narrow and wide as they are
/// dragged. A row of the other sections has no remote: its name takes
/// both, so that what is known of a repository begins at the same place
/// in every section (577 in).
pub const NAME_WIDTH: f32 = 300.;
pub const REMOTE_WIDTH: f32 = 200.;
const NAME_LEAST: f32 = 120.;
const NAME_MOST: f32 = 900.;
const REMOTE_LEAST: f32 = 60.;
const REMOTE_MOST: f32 = 700.;
/// The grip between two columns, under the pointer.
const GRIP_WIDTH: f32 = 9.;
/// The buttons at the right end of a row: their width, the space between
/// them, and the space right of the last one.
const ROW_BUTTON: f32 = 24.;
const ROW_BUTTON_GAP: f32 = 11.;
const ROW_RIGHT: f32 = 13.;
/// What is known of a repository is shown in small boxes this high.
const CHIP_HEIGHT: f32 = 18.;
/// Room at the end of the list, where a heading dropped goes last.
const END_ZONE: f32 = 40.;

/// Events from the page, for the window to act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RepositoriesEvent {
    /// Open this repository in a tab, or show its tab.
    Open(PathBuf),
    /// Open these repositories of a workspace, each in a tab of its own,
    /// with the workspace's defaults.
    OpenMany {
        /// The repositories, in the order their tabs are to be.
        paths: Vec<PathBuf>,
        /// The workspace they are of, by its id.
        workspace: String,
    },
    /// Show the directory chooser.
    Browse,
    /// Ask what to clone and where to.
    Clone,
    /// Ask where the new repository goes.
    Init,
    /// Close the tab this repository is open in.
    Close(PathBuf),
}

/// The sections of the page that it always has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Part {
    /// Open in a tab.
    Open,
    /// Made a favourite.
    Favourites,
    /// Opened before.
    Recent,
    /// Every repository of the others, once.
    All,
}

impl Part {
    /// In the order they are shown when nothing else was chosen.
    pub const ALL: [Part; 4] = [Part::Open, Part::Favourites, Part::Recent, Part::All];

    /// The word for it in element ids, and the key it is kept under.
    pub fn key(self) -> &'static str {
        match self {
            Part::Open => "open",
            Part::Favourites => "favourites",
            Part::Recent => "recent",
            Part::All => "all",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Part::Open => say!("repositories-section-open"),
            Part::Favourites => say!("repositories-section-favourites"),
            Part::Recent => say!("repositories-section-recent"),
            Part::All => say!("repositories-section-all"),
        }
    }
}

/// A repository on the page: what it is called, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    path: PathBuf,
}

/// A section's heading, picked up to be put elsewhere.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DraggedSection {
    section: Section,
    title: String,
}

/// The grip between two columns of the rows, while it is dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnGrip {
    /// The right edge of the names.
    Name,
    /// The right edge of the remotes.
    Remote,
}

/// Nothing follows the pointer while a grip is dragged.
struct GripChip;

impl Render for GripChip {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The page.
pub struct RepositoriesView {
    focus_handle: FocusHandle,
    search: Entity<InputState>,
    /// What is typed in the search, in lower case.
    query: String,
    /// The sections that are folded, by their keys.
    folded: Vec<String>,
    /// Whether a row says what is uncommitted.
    show_changes: bool,
    /// What git said of each repository, the last time it was asked.
    seen: HashMap<PathBuf, Glance>,
    /// Which asking is the latest: an older answer is not taken.
    asked: u64,
    /// The workspaces, as their file had them the last time it was read.
    workspaces: Vec<Listed>,
    /// Why the workspaces could not be read, when they could not.
    workspaces_problem: Option<String>,
    /// The rows that are ticked: a workspace's id and a repository's path.
    ticked: HashSet<(String, String)>,
    /// The commands running on many repositories, by workspace.
    runs: HashMap<String, bulk::Run>,
    /// What became of each repository in the last of them, by workspace
    /// and path, and which command that was.
    outcomes: HashMap<(String, String), RepositoryOutcome>,
    bulk_kinds: HashMap<String, bulk::Kind>,
    /// The dialog that makes or edits a workspace, while it is open.
    form: Option<Entity<WorkspaceForm>>,
    /// The section to bring into sight once it is on the page.
    revealing: Option<Section>,
    /// Whether a column's grip was dragged since the button went down:
    /// the widths are kept when it comes up.
    resized: bool,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<RepositoriesEvent> for RepositoriesView {}

impl RepositoriesView {
    /// Create the page.
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(say!("repositories-search")));
        let typed = cx.subscribe_in(&search, window, |this, state, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::Change) {
                this.query = state.read(cx).value().to_lowercase();
                cx.notify();
            }
        });
        Self {
            focus_handle: cx.focus_handle(),
            search,
            query: String::new(),
            folded: Vec::new(),
            show_changes: true,
            seen: HashMap::new(),
            asked: 0,
            workspaces: Vec::new(),
            workspaces_problem: None,
            ticked: HashSet::new(),
            runs: HashMap::new(),
            outcomes: HashMap::new(),
            bulk_kinds: HashMap::new(),
            form: None,
            revealing: None,
            resized: false,
            scroll: ScrollHandle::new(),
            _subscriptions: vec![typed],
        }
    }

    /// Show the page: the workspaces are read and git is asked about
    /// every repository on it.
    pub fn show(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.forget_done();
        self.look(window, cx);
        cx.notify();
    }

    /// Read the workspaces and ask git about the repositories again, what
    /// came of the last commands on many left as it is: a command run
    /// elsewhere (the palette) may have changed what the page shows.
    pub fn read_again(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.look(window, cx);
        cx.notify();
    }

    /// Put the focus in the search, as the page is shown.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |state, cx| state.focus(window, cx));
    }

    /// The workspaces as the page has them.
    pub fn workspaces(&self) -> &[Listed] {
        &self.workspaces
    }

    /// The sections in the order they are shown, the hidden ones left out.
    pub fn shown_sections(&self, cx: &App) -> Vec<Section> {
        let kept = &cx.global::<Prefs>().state.repositories_page;
        self.sections(cx)
            .into_iter()
            .filter(|section| !kept.hidden_sections.contains(&section.key()))
            .collect()
    }

    /// Every section, hidden or not, in the order they are shown.
    fn sections(&self, cx: &App) -> Vec<Section> {
        let kept = &cx.global::<Prefs>().state.repositories_page;
        let ids: Vec<String> = self
            .workspaces
            .iter()
            .map(|listed| listed.workspace.id.clone())
            .collect();
        sections::ordered(&kept.section_order, &ids)
    }

    /// Ask git about the repositories on the page, off the main thread:
    /// the workspaces are read first, since their repositories are on it.
    fn look(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.asked = self.asked.wrapping_add(1);
        let asked = self.asked;
        let following: Vec<String> = self
            .workspaces
            .iter()
            .filter(|listed| listed.workspace.follows_folder.is_some())
            .map(|listed| listed.workspace.id.clone())
            .collect();
        cx.spawn_in(window, async move |this, cx| {
            // A workspace that follows a folder takes in what is new in
            // it first. One that cannot is left as it was: it is said
            // when the workspaces are read.
            for id in following {
                let run = crate::session::run_standalone_unasked(
                    "workspace.sync_folder",
                    serde_json::json!({ "id": id }),
                );
                let _ = run.reply.await;
            }
            let run =
                crate::session::run_standalone_unasked("workspace.list", serde_json::json!({}));
            let listed = run.reply.await;
            let paths = this
                .update(cx, |this, cx| {
                    if this.asked != asked {
                        return None;
                    }
                    match listed {
                        Ok(crate::session::Reply::Command(value)) => {
                            this.workspaces = workspaces::listed(&value);
                            this.workspaces_problem = None;
                        }
                        Ok(crate::session::Reply::Error(err)) => {
                            this.workspaces_problem = Some(err);
                        }
                        _ => {}
                    }
                    this.forget_gone();
                    this.scroll_to_revealed(cx);
                    cx.notify();
                    Some(this.paths_to_glance(cx))
                })
                .ok()
                .flatten();
            let Some(paths) = paths else {
                return;
            };
            for chunk in paths.chunks(MAX_GLANCED) {
                let run = crate::session::run_standalone_unasked(
                    "repo.glance",
                    serde_json::json!({ "paths": chunk }),
                );
                let Ok(crate::session::Reply::Command(value)) = run.reply.await else {
                    continue;
                };
                let Ok(found) = serde_json::from_value::<Vec<Glance>>(value) else {
                    continue;
                };
                let current = this
                    .update(cx, |this, cx| {
                        if this.asked != asked {
                            return false;
                        }
                        for glance in found {
                            this.seen.insert(glance.path.clone(), glance);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !current {
                    return;
                }
            }
        })
        .detach();
    }

    /// Every repository on the page once, as git is to be asked about
    /// them.
    fn paths_to_glance(&self, cx: &App) -> Vec<String> {
        let mut paths: Vec<String> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        for entry in self.entries(Part::All, cx) {
            let text = entry.path.display().to_string();
            if seen.insert(text.clone()) {
                paths.push(text);
            }
        }
        for listed in &self.workspaces {
            for repository in &listed.workspace.repositories {
                if !listed.missing.contains(&repository.path)
                    && seen.insert(repository.path.clone())
                {
                    paths.push(repository.path.clone());
                }
            }
        }
        paths
    }

    /// Ticks and outcomes of workspaces and repositories that are no
    /// longer there are forgotten.
    fn forget_gone(&mut self) {
        let there: HashSet<(String, String)> = self
            .workspaces
            .iter()
            .flat_map(|listed| {
                listed
                    .workspace
                    .repositories
                    .iter()
                    .map(|repository| (listed.workspace.id.clone(), repository.path.clone()))
            })
            .collect();
        self.ticked.retain(|key| there.contains(key));
        self.outcomes.retain(|key, _| there.contains(key));
    }

    /// The repositories of a section that is one of the four, before the
    /// search.
    fn entries(&self, part: Part, cx: &App) -> Vec<Entry> {
        let state = &cx.global::<Prefs>().state;
        let known = |paths: &[PathBuf]| -> Vec<Entry> {
            paths.iter().map(|path| entry_of(path, cx)).collect()
        };
        match part {
            Part::Open => known(&state.open),
            Part::Favourites => known(&state.favourites),
            Part::Recent => known(&state.recent),
            Part::All => {
                let mut all: Vec<Entry> = Vec::new();
                let in_workspaces: Vec<PathBuf> = self
                    .workspaces
                    .iter()
                    .flat_map(|listed| {
                        listed
                            .workspace
                            .repositories
                            .iter()
                            .filter(|repository| !listed.missing.contains(&repository.path))
                            .map(|repository| PathBuf::from(&repository.path))
                    })
                    .collect();
                for entry in known(&state.open)
                    .into_iter()
                    .chain(known(&state.favourites))
                    .chain(known(&state.recent))
                    .chain(known(&in_workspaces))
                {
                    if !all.iter().any(|seen| seen.path == entry.path) {
                        all.push(entry);
                    }
                }
                all
            }
        }
    }

    /// Whether the search leaves a repository of this name and path.
    fn searched(&self, name: &str, path: &Path) -> bool {
        self.query.is_empty()
            || name.to_lowercase().contains(&self.query)
            || path
                .display()
                .to_string()
                .to_lowercase()
                .contains(&self.query)
    }

    /// The repositories of a section that the search leaves.
    fn listed(&self, part: Part, cx: &App) -> Vec<Entry> {
        let mut entries = self.entries(part, cx);
        entries.retain(|entry| self.searched(&entry.name, &entry.path));
        entries
    }

    fn fold(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.folded.iter().any(|folded| folded == key) {
            self.folded.retain(|folded| folded != key);
        } else {
            self.folded.push(key.to_owned());
        }
        cx.notify();
    }

    fn fold_all(&mut self, fold: bool, cx: &mut Context<Self>) {
        self.folded = if fold {
            self.sections(cx).iter().map(Section::key).collect()
        } else {
            Vec::new()
        };
        cx.notify();
    }

    /// Unfold a section and bring it into sight: what a workspace on the
    /// start page leads to.
    pub fn reveal(&mut self, section: &Section, cx: &mut Context<Self>) {
        let key = section.key();
        self.folded.retain(|folded| *folded != key);
        let prefs = cx.global_mut::<Prefs>();
        let page = &mut prefs.state.repositories_page;
        if page.hidden_sections.contains(&key) {
            page.hidden_sections.retain(|hidden| *hidden != key);
            prefs.save_state();
        }
        // A workspace not read yet is brought into sight once it is.
        self.revealing = Some(section.clone());
        self.scroll_to_revealed(cx);
        cx.notify();
    }

    /// Bring the section asked for into sight, when it is on the page.
    fn scroll_to_revealed(&mut self, cx: &App) {
        let Some(section) = self.revealing.clone() else {
            return;
        };
        let at = self
            .shown_sections(cx)
            .iter()
            .position(|shown| *shown == section);
        if let Some(at) = at {
            // The line that says the workspaces could not be read comes
            // first, when it is there.
            let at = at.saturating_add(usize::from(self.workspaces_problem.is_some()));
            self.scroll.scroll_to_top_of_item(at);
            self.revealing = None;
        }
    }

    /// Put a section before another, or last; the order is kept.
    pub fn move_section(
        &mut self,
        moved: &Section,
        before: Option<&Section>,
        cx: &mut Context<Self>,
    ) {
        let shown = self.sections(cx);
        if let Some(order) = sections::moved(&shown, moved, before) {
            let prefs = cx.global_mut::<Prefs>();
            prefs.state.repositories_page.section_order = order;
            prefs.save_state();
            cx.notify();
        }
    }

    /// Hide a section, or show it again; which are hidden is kept.
    pub fn hide_section(&mut self, section: &Section, hide: bool, cx: &mut Context<Self>) {
        let there = self.sections(cx);
        let prefs = cx.global_mut::<Prefs>();
        let page = &mut prefs.state.repositories_page;
        page.hidden_sections =
            sections::hidden_with(&page.hidden_sections, &section.key(), hide, &there);
        prefs.save_state();
        cx.notify();
    }

    /// Show every hidden section again.
    fn show_all_sections(&mut self, cx: &mut Context<Self>) {
        let prefs = cx.global_mut::<Prefs>();
        prefs.state.repositories_page.hidden_sections.clear();
        prefs.save_state();
        cx.notify();
    }

    /// The widths of the column of names and of remotes, as kept.
    fn columns(cx: &App) -> (f32, f32) {
        let page = &cx.global::<Prefs>().state.repositories_page;
        (
            page.name_width
                .unwrap_or(NAME_WIDTH)
                .clamp(NAME_LEAST, NAME_MOST),
            page.remote_width
                .unwrap_or(REMOTE_WIDTH)
                .clamp(REMOTE_LEAST, REMOTE_MOST),
        )
    }

    /// A column's grip was dragged to `x` in the list's own measure.
    fn drag_column(&mut self, grip: ColumnGrip, x: f32, cx: &mut Context<Self>) {
        let (name, remote) = Self::columns(cx);
        let page = &mut cx.global_mut::<Prefs>().state.repositories_page;
        match grip {
            ColumnGrip::Name => {
                let width =
                    (x - SECTION_SIDE - NAME_LEFT + GRIP_WIDTH / 2.).clamp(NAME_LEAST, NAME_MOST);
                if (width - name).abs() >= 0.5 {
                    page.name_width = Some(width);
                    cx.notify();
                }
            }
            ColumnGrip::Remote => {
                let width = (x - SECTION_SIDE - NAME_LEFT - name + GRIP_WIDTH / 2.)
                    .clamp(REMOTE_LEAST, REMOTE_MOST);
                if (width - remote).abs() >= 0.5 {
                    page.remote_width = Some(width);
                    cx.notify();
                }
            }
        }
    }

    /// The top of the page: the heading, the ways to begin, and the row
    /// with folding, the search and the box for the changes.
    fn top(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let button = |id: &'static str, label: &'static str, width: f32| {
            Button::new(id)
                .outline()
                .small()
                .h(px(BUTTON_HEIGHT))
                .w(px(width))
                .label(label)
        };
        let show_changes = self.show_changes;
        let page = cx.entity().downgrade();
        let hidden: Vec<Section> = {
            let kept = &cx.global::<Prefs>().state.repositories_page;
            self.sections(cx)
                .into_iter()
                .filter(|section| kept.hidden_sections.contains(&section.key()))
                .collect()
        };
        let hidden_titles: Vec<(Section, String)> = hidden
            .iter()
            .map(|section| (section.clone(), self.title_of(section)))
            .collect();
        let _ = window;
        v_flex()
            .w_full()
            .flex_none()
            .px(px(SIDE))
            .pt(px(TOP))
            .child(
                div()
                    .h(px(HEADING_LINE))
                    .line_height(px(HEADING_LINE))
                    .text_base()
                    .font_semibold()
                    .child(say!("repositories-heading")),
            )
            .child(
                h_flex()
                    .mt(px(BUTTONS_TOP))
                    .h(px(BUTTON_HEIGHT))
                    .gap(px(5.))
                    .child(
                        button("repositories-browse", say!("repositories-browse"), 91.)
                            .icon(IconName::FolderOpen)
                            .on_click(
                                cx.listener(|_, _, _, cx| cx.emit(RepositoriesEvent::Browse)),
                            ),
                    )
                    .child(
                        button("repositories-clone", say!("repositories-clone"), 81.)
                            .icon(gpui_kit::assets::IconName::ArrowDownToLine)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RepositoriesEvent::Clone))),
                    )
                    .child(
                        button("repositories-init", say!("repositories-init"), 67.)
                            .icon(IconName::Plus)
                            .on_click(cx.listener(|_, _, _, cx| cx.emit(RepositoriesEvent::Init))),
                    )
                    .child(
                        button(
                            "repositories-new-workspace",
                            say!("repositories-new-workspace"),
                            156.,
                        )
                        .icon(gpui_kit::assets::IconName::Layers)
                        .tooltip(say!("repositories-new-workspace-tip"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.open_form(Making::New, window, cx)
                        })),
                    )
                    .child(
                        button(
                            "repositories-import-workspace",
                            say!("repositories-import"),
                            85.,
                        )
                        .icon(gpui_kit::assets::IconName::FileInput)
                        .tooltip(say!("repositories-import-tip"))
                        .on_click(
                            cx.listener(|this, _, window, cx| this.import_workspace(window, cx)),
                        ),
                    ),
            )
            .child(
                h_flex()
                    .mt(px(TOOLS_TOP))
                    .h(px(TOOLS_HEIGHT))
                    .w_full()
                    .gap(px(4.))
                    .child(
                        Button::new("repositories-collapse")
                            .ghost()
                            .small()
                            .icon(gpui_kit::assets::IconName::ChevronsDownUp)
                            .label(say!("repositories-collapse"))
                            .on_click(cx.listener(|this, _, _, cx| this.fold_all(true, cx))),
                    )
                    .child(
                        Button::new("repositories-expand")
                            .ghost()
                            .small()
                            .icon(IconName::ChevronsUpDown)
                            .label(say!("repositories-expand"))
                            .on_click(cx.listener(|this, _, _, cx| this.fold_all(false, cx))),
                    )
                    .child(
                        div().ml(px(4.)).flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .id("repositories-search")
                                .prefix(Icon::new(IconName::Search).size(px(16.)))
                                .min_h(px(TOOLS_HEIGHT))
                                .w_full(),
                        ),
                    )
                    .child(
                        h_flex()
                            .ml(px(SUMMARY_GAP - 4.))
                            .w(px(SUMMARY_WIDTH))
                            .flex_none()
                            .child(
                                Checkbox::new("repositories-changes")
                                    .label(say!("repositories-changes"))
                                    .checked(show_changes)
                                    .on_click(move |on: &bool, _, cx: &mut App| {
                                        let on = *on;
                                        let _ = page.update(cx, |this, cx| {
                                            this.show_changes = on;
                                            cx.notify();
                                        });
                                    }),
                            ),
                    )
                    .when(!hidden_titles.is_empty(), |row| {
                        let count = hidden_titles.len();
                        let page = cx.entity().downgrade();
                        row.child(
                            Button::new("repositories-hidden")
                                .ghost()
                                .small()
                                .flex_none()
                                .icon(IconName::EyeOff)
                                .label(count.to_string())
                                .tooltip(say!("repositories-hidden"))
                                .dropdown_menu(move |menu, _, _| {
                                    let shown_all = page.clone();
                                    let mut menu = menu.item(
                                        PopupMenuItem::new(say!("repositories-show-all"))
                                            .icon(IconName::Eye)
                                            .on_click(move |_, _, cx| {
                                                let _ = shown_all.update(cx, |this, cx| {
                                                    this.show_all_sections(cx)
                                                });
                                            }),
                                    );
                                    menu = menu.separator();
                                    for (section, title) in &hidden_titles {
                                        let (page, section) = (page.clone(), section.clone());
                                        menu =
                                            menu.item(PopupMenuItem::new(title.clone()).on_click(
                                                move |_, _, cx| {
                                                    let _ = page.update(cx, |this, cx| {
                                                        this.hide_section(&section, false, cx)
                                                    });
                                                },
                                            ));
                                    }
                                    menu
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// What a section is called.
    fn title_of(&self, section: &Section) -> String {
        match section {
            Section::Part(part) => part.title().to_owned(),
            Section::Workspace(id) => self
                .workspaces
                .iter()
                .find(|listed| listed.workspace.id == *id)
                .map(|listed| listed.workspace.name.clone())
                .unwrap_or_else(|| id.clone()),
        }
    }

    /// A section's heading, whichever it is: it folds the section, is
    /// dragged onto another to go before it, and has a menu at its right.
    #[allow(clippy::too_many_arguments)]
    fn heading(
        &self,
        section: &Section,
        lead: AnyElement,
        tint: Option<(Hsla, Hsla)>,
        controls: AnyElement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let key = section.key();
        let folded = self.folded.contains(&key);
        let (for_fold, for_drop) = (key.clone(), section.clone());
        let dragged = DraggedSection {
            section: section.clone(),
            title: self.title_of(section),
        };
        let drop_line = theme.drop_target;
        h_flex()
            .id(SharedString::from(format!("part-{key}")))
            .w_full()
            .h(px(HEADER_HEIGHT))
            .flex_none()
            .pl(px(ARROW_LEFT))
            .gap(px(15.))
            .items_center()
            .rounded(theme.radius)
            .border_t_2()
            .border_color(transparent_black())
            .map(|heading| match tint {
                Some((rest, hovered)) => heading.bg(rest).hover(move |style| style.bg(hovered)),
                None => heading.bg(theme.secondary),
            })
            .cursor_pointer()
            .child(
                Icon::new(if folded {
                    IconName::ChevronRight
                } else {
                    IconName::ChevronDown
                })
                .size(px(16.)),
            )
            .child(div().flex_1().min_w_0().child(lead))
            // A press on the buttons at the right is theirs: it neither
            // folds the section nor picks the heading up.
            .child(
                div()
                    .flex_none()
                    .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                    .child(controls),
            )
            .on_click(cx.listener(move |this, _, _, cx| this.fold(&for_fold, cx)))
            .on_drag(dragged, |dragged, _, _, cx| {
                cx.new(|_| crate::views::rewrite::DragChip(dragged.title.clone()))
            })
            .drag_over::<DraggedSection>(move |style, _, _, _| style.border_color(drop_line))
            .on_drop(cx.listener(move |this, dragged: &DraggedSection, _, cx| {
                this.move_section(&dragged.section, Some(&for_drop), cx)
            }))
            .test_support()
            .into_any_element()
    }

    /// The name of a section and how many it has, as its heading shows
    /// them.
    fn name_and_count(title: String, count: usize, cx: &App) -> Div {
        let theme = cx.theme();
        h_flex()
            .gap(px(10.))
            .items_center()
            .child(div().text_sm().font_semibold().child(title))
            .child(
                div()
                    .h(px(CHIP_HEIGHT))
                    .min_w(px(CHIP_HEIGHT))
                    .px(px(5.))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(theme.muted)
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(count.to_string()),
            )
    }

    /// The menu of one of the four sections' headings: hide it.
    fn part_menu(&self, part: Part, cx: &mut Context<Self>) -> AnyElement {
        let page = cx.entity().downgrade();
        h_flex()
            .flex_none()
            .pr(px(ROW_RIGHT))
            .child(
                Button::new(SharedString::from(format!("menu-{}", part.key())))
                    .accessibility_label(say!("repositories-more"))
                    .ghost()
                    .xsmall()
                    .w(px(ROW_BUTTON))
                    .icon(IconName::EllipsisVertical)
                    .tooltip(say!("repositories-more"))
                    .dropdown_menu(move |menu, _, _| {
                        let page = page.clone();
                        menu.item(
                            PopupMenuItem::new(say!("repositories-hide"))
                                .icon(IconName::EyeOff)
                                .on_click(move |_, _, cx| {
                                    let _ = page.update(cx, |this, cx| {
                                        this.hide_section(&Section::Part(part), true, cx)
                                    });
                                }),
                        )
                    }),
            )
            .into_any_element()
    }

    /// One of the four sections: its heading, and its rows unless it is
    /// folded.
    fn section(&self, part: Part, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let entries = self.listed(part, cx);
        let folded = self.folded.iter().any(|key| key == part.key());
        let lead =
            Self::name_and_count(part.title().to_owned(), entries.len(), cx).into_any_element();
        let controls = self.part_menu(part, cx);
        let header = self.heading(&Section::Part(part), lead, None, controls, cx);
        let body = (!folded).then(|| {
            if entries.is_empty() {
                nothing(&theme)
            } else {
                v_flex()
                    .w_full()
                    .pt(px(ROWS_TOP))
                    .pb(px(ROWS_BOTTOM))
                    .children(entries.iter().map(|entry| self.row(part, entry, cx)))
                    .into_any_element()
            }
        });
        v_flex()
            .w_full()
            .flex_none()
            .mb(px(HEADER_GAP))
            .child(header)
            .children(body)
            .into_any_element()
    }

    /// A repository's row: its name, what is known of it, and at the right
    /// what can be done with it here.
    fn row(&self, part: Part, entry: &Entry, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let key = format!("{}-{}", part.key(), entry.name);
        let id = |what: &str| SharedString::from(format!("{what}-{key}"));
        let favourite = cx.global::<Prefs>().state.favourites.contains(&entry.path);
        let (open_path, name_path, star_path, out_path) = (
            entry.path.clone(),
            entry.path.clone(),
            entry.path.clone(),
            entry.path.clone(),
        );
        let (name_width, remote_width) = Self::columns(cx);
        let take_out = match part {
            Part::Open => Some((say!("repositories-close-tab"), Part::Open)),
            Part::Favourites => Some((say!("repositories-unfavourite"), Part::Favourites)),
            Part::Recent => Some((say!("repositories-take-out-recent"), Part::Recent)),
            Part::All => None,
        };
        h_flex()
            .id(id("repo"))
            .w_full()
            .h(px(ROW_HEIGHT))
            .flex_none()
            .items_center()
            .rounded(theme.radius)
            .hover(|row| row.bg(theme.list_hover))
            .child(
                div()
                    .id(id("name"))
                    .pl(px(NAME_LEFT))
                    .w(px(NAME_LEFT + name_width + remote_width))
                    .flex_none()
                    .pr(px(10.))
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .cursor_pointer()
                    .child(entry.name.clone())
                    .tooltip({
                        let path = entry.path.display().to_string();
                        move |window, cx| Tooltip::new(path.clone()).build(window, cx)
                    })
                    .on_click(cx.listener(move |_, _, _, cx| {
                        cx.emit(RepositoriesEvent::Open(name_path.clone()))
                    }))
                    .test_support(),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(5.))
                    .overflow_hidden()
                    .children(self.chips(&key, &entry.path, cx)),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(px(ROW_BUTTON_GAP))
                    .pr(px(ROW_RIGHT))
                    .child(
                        Button::new(id("open"))
                            .accessibility_label(say!("repositories-open"))
                            .ghost()
                            .xsmall()
                            .w(px(ROW_BUTTON))
                            .icon(IconName::ExternalLink)
                            .tooltip(say!("repositories-open"))
                            .on_click(cx.listener(move |_, _, _, cx| {
                                cx.emit(RepositoriesEvent::Open(open_path.clone()))
                            })),
                    )
                    .child(star_button(id("star"), favourite, star_path, cx))
                    .child(match take_out {
                        Some((tip, from)) => Button::new(id("take-out"))
                            .accessibility_label(tip)
                            .ghost()
                            .xsmall()
                            .w(px(ROW_BUTTON))
                            .icon(IconName::Close)
                            .tooltip(tip)
                            .on_click(cx.listener(move |_, _, _, cx| {
                                let path = out_path.clone();
                                match from {
                                    Part::Open => cx.emit(RepositoriesEvent::Close(path)),
                                    Part::Favourites => change(cx, &|state| {
                                        state.remove_entry(RepoList::Favourites, &path)
                                    }),
                                    Part::Recent | Part::All => change(cx, &|state| {
                                        state.remove_entry(RepoList::Recent, &path)
                                    }),
                                }
                            }))
                            .into_any_element(),
                        None => div().w(px(ROW_BUTTON)).into_any_element(),
                    }),
            )
            .test_support()
            .into_any_element()
    }

    /// What is known of a repository, in small boxes: its branch, how far
    /// that is ahead and behind, what is uncommitted and when it was last
    /// fetched; or why it could not be looked at.
    fn chips(&self, key: &str, path: &Path, cx: &App) -> Vec<AnyElement> {
        let theme = cx.theme();
        let Some(seen) = self.seen.get(path) else {
            return Vec::new();
        };
        let chip = |what: &str| chip(what, key, cx);
        let count = |icon: Icon, number: usize, colour: Hsla| {
            h_flex()
                .gap(px(2.))
                .items_center()
                .child(icon.size(px(11.)).text_color(colour))
                .child(number.to_string())
        };
        let mut chips = Vec::new();
        if let Some(problem) = seen.problem.clone() {
            let words = if path.is_dir() {
                say!("repositories-not-readable")
            } else {
                say!("repositories-not-found")
            };
            chips.push(
                chip("problem")
                    .text_color(theme.danger)
                    .child(Icon::new(IconName::TriangleAlert).size(px(11.)))
                    .child(words)
                    .tooltip(move |window, cx| Tooltip::new(problem.clone()).build(window, cx))
                    .into_any_element(),
            );
            return chips;
        }
        chips.push(
            chip("branch")
                .child(
                    Icon::new(gpui_kit::assets::IconName::GitBranch)
                        .size(px(11.))
                        .text_color(theme.muted_foreground),
                )
                .child(
                    div().max_w(px(160.)).truncate().child(
                        seen.branch
                            .clone()
                            .unwrap_or_else(|| say!("repositories-detached").to_owned()),
                    ),
                )
                .into_any_element(),
        );
        let (ahead, behind) = (seen.ahead.unwrap_or(0), seen.behind.unwrap_or(0));
        if ahead > 0 || behind > 0 {
            chips.push(
                chip("ahead")
                    .when(ahead > 0, |chip| {
                        chip.child(count(Icon::new(IconName::ArrowUp), ahead, theme.success))
                    })
                    .when(behind > 0, |chip| {
                        chip.child(count(Icon::new(IconName::ArrowDown), behind, theme.warning))
                    })
                    .into_any_element(),
            );
        }
        let counts = seen.counts;
        if self.show_changes && counts.total() > 0 {
            let kinds = [
                (
                    Icon::new(gpui_kit::assets::IconName::Pencil),
                    counts.modified,
                    theme.warning,
                ),
                (Icon::new(IconName::Plus), counts.added, theme.success),
                (Icon::new(IconName::Minus), counts.deleted, theme.danger),
                (
                    Icon::new(gpui_kit::assets::IconName::ArrowRightLeft),
                    counts.renamed,
                    theme.info,
                ),
            ];
            chips.push(
                chip("changes")
                    .gap(px(7.))
                    .children(
                        kinds
                            .into_iter()
                            .filter(|(_, number, _)| *number > 0)
                            .map(|(icon, number, colour)| count(icon, number, colour)),
                    )
                    .into_any_element(),
            );
        }
        // When it was last fetched: when git last wrote its FETCH_HEAD.
        if let Some(then) = seen.last_fetch {
            let said = crate::views::relative_time(then, crate::views::now());
            let whole = say!(
                "repositories-last-fetched",
                said = said.as_str(),
                time = crate::views::absolute_time(then, 0)
            );
            chips.push(
                chip("fetched")
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(gpui_kit::assets::IconName::CloudDownload).size(px(11.)))
                    .child(said)
                    .tooltip(move |window, cx| Tooltip::new(whole.clone()).build(window, cx))
                    .into_any_element(),
            );
        }
        chips
    }
}

/// A repository as a row names it: by the name its tab was given, else by
/// its folder's.
fn entry_of(path: &Path, cx: &App) -> Entry {
    let state = &cx.global::<Prefs>().state;
    Entry {
        name: state
            .aliases
            .get(&path.to_string_lossy().into_owned())
            .map(|alias| alias.trim().to_owned())
            .filter(|alias| !alias.is_empty())
            .unwrap_or_else(|| crate::shell::repo_title(path)),
        path: path.to_path_buf(),
    }
}

/// A small box of what is known of a repository.
fn chip(what: &str, key: &str, cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    let theme = cx.theme();
    h_flex()
        .id(SharedString::from(format!("{what}-{key}")))
        .h(px(CHIP_HEIGHT))
        .flex_none()
        .px(px(4.))
        .gap(px(3.))
        .items_center()
        .rounded(px(3.))
        .bg(theme.muted)
        .text_xs()
        .test_support()
}

/// The line that says a section has nothing in it.
fn nothing(theme: &gpui_kit::component::Theme) -> AnyElement {
    div()
        .w_full()
        .pt(px(ROWS_TOP))
        .pb(px(EMPTY_BOTTOM))
        .child(
            div()
                .h(px(ROW_HEIGHT))
                .pl(px(NOTHING_LEFT))
                .flex()
                .items_center()
                .text_sm()
                .italic()
                .text_color(theme.muted_foreground)
                .child(say!("repositories-none")),
        )
        .into_any_element()
}

/// The star of a row, which makes the repository a favourite or not.
fn star_button(
    id: SharedString,
    favourite: bool,
    path: PathBuf,
    cx: &mut Context<RepositoriesView>,
) -> Button {
    Button::new(id)
        .accessibility_label(if favourite {
            say!("repositories-unfavourite")
        } else {
            say!("repositories-favourite")
        })
        .ghost()
        .xsmall()
        .w(px(ROW_BUTTON))
        .icon(if favourite {
            IconName::StarFill
        } else {
            IconName::Star
        })
        .tooltip(if favourite {
            say!("repositories-unfavourite")
        } else {
            say!("repositories-favourite")
        })
        .on_click(cx.listener(move |_, _, _, cx| {
            let path = path.clone();
            change(cx, &|state| {
                if state.favourites.contains(&path) {
                    state.remove_entry(RepoList::Favourites, &path)
                } else {
                    state.favourites.push(path.clone());
                    true
                }
            });
        }))
}

/// Change what is remembered, save it when it changed, and draw the page
/// again.
fn change(cx: &mut Context<RepositoriesView>, change: &dyn Fn(&mut crate::prefs::State) -> bool) {
    let prefs = cx.global_mut::<Prefs>();
    if change(&mut prefs.state) {
        prefs.save_state();
    }
    cx.notify();
}

impl Focusable for RepositoriesView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RepositoriesView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown = self.shown_sections(cx);
        let sections: Vec<AnyElement> = shown
            .iter()
            .map(|section| match section {
                Section::Part(part) => self.section(*part, cx),
                Section::Workspace(id) => self.workspace_section(id, cx),
            })
            .collect();
        let drop_line = cx.theme().drop_target;
        let problem = self.workspaces_problem.clone();
        v_flex()
            .id("repositories-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .child(self.top(window, cx))
            .child(
                div()
                    .relative()
                    .mt(px(SECTIONS_TOP))
                    .flex_1()
                    .min_h_0()
                    .w_full()
                    .child(
                        v_flex()
                            .id("repositories-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.scroll)
                            .px(px(SECTION_SIDE))
                            .pb(px(24.))
                            // A grip between two columns, dragged.
                            .on_drag_move(cx.listener(
                                |this, event: &DragMoveEvent<ColumnGrip>, _, cx| {
                                    let x = crate::zoom::unpx(event.event.position.x)
                                        - crate::zoom::unpx(event.bounds.left());
                                    this.resized = true;
                                    this.drag_column(*event.drag(cx), x, cx);
                                },
                            ))
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(|this, _, _, cx| {
                                    // The widths a drag left are kept.
                                    if std::mem::take(&mut this.resized) {
                                        cx.global::<Prefs>().save_state();
                                    }
                                }),
                            )
                            .when_some(problem, |list, problem| {
                                list.child(
                                    div()
                                        .pb(px(10.))
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(say!("repositories-workspaces-unreadable", problem)),
                                )
                            })
                            .children(sections)
                            .child(
                                div()
                                    .id("repositories-end")
                                    .w_full()
                                    .h(px(END_ZONE))
                                    .flex_none()
                                    .border_t_2()
                                    .border_color(transparent_black())
                                    .drag_over::<DraggedSection>(move |style, _, _, _| {
                                        style.border_color(drop_line)
                                    })
                                    .on_drop(cx.listener(
                                        |this, dragged: &DraggedSection, _, cx| {
                                            this.move_section(&dragged.section, None, cx)
                                        },
                                    )),
                            ),
                    )
                    .scrollbar(&self.scroll, ScrollbarAxis::Vertical),
            )
    }
}
