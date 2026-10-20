//! Preferences and session state on disk.
//!
//! `preferences.toml` holds what the user chose; `state.toml` holds what the
//! app remembers (open tabs, recents, per-repository layout). Both live in
//! the platform configuration directory and are written atomically.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use gpui_kit::{App, Global};
use serde::{Deserialize, Serialize};

/// Theme choice: one of ours, or one of the user's own by its name. In
/// the file it is a word: `dark`, `light`, `contrast`, `dusk`, `ember`,
/// `graphite`, `paper`, `sage`, or the name.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum ThemeChoice {
    /// Our dark theme.
    #[default]
    Dark,
    /// Our light theme.
    Light,
    /// High-contrast theme.
    Contrast,
    /// Dark, plum grounds and a violet accent.
    Dusk,
    /// Dark, warm charcoal and an amber accent.
    Ember,
    /// Dark, neutral greys and an indigo accent.
    Graphite,
    /// Light, warm paper and a terracotta accent.
    Paper,
    /// Light, green-grey and a leaf-green accent.
    Sage,
    /// A theme from the `themes` folder.
    User(String),
}

impl ThemeChoice {
    /// Our themes, in the order the menus list them.
    pub const BUILT_IN: [ThemeChoice; 8] = [
        Self::Dark,
        Self::Light,
        Self::Contrast,
        Self::Dusk,
        Self::Ember,
        Self::Graphite,
        Self::Paper,
        Self::Sage,
    ];

    /// The word the file has for it.
    pub fn key(&self) -> &str {
        match self {
            Self::Dark => "dark",
            Self::Light => "light",
            Self::Contrast => "contrast",
            Self::Dusk => "dusk",
            Self::Ember => "ember",
            Self::Graphite => "graphite",
            Self::Paper => "paper",
            Self::Sage => "sage",
            Self::User(name) => name,
        }
    }

    /// The choice a word of the file stands for.
    pub fn from_key(key: &str) -> Self {
        Self::BUILT_IN
            .into_iter()
            .find(|ours| ours.key() == key)
            .unwrap_or_else(|| Self::User(key.to_owned()))
    }
}

impl Serialize for ThemeChoice {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.key())
    }
}

impl<'de> Deserialize<'de> for ThemeChoice {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        Ok(Self::from_key(&key))
    }
}

/// A graph column.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum Column {
    /// Commit summary.
    Summary,
    /// Author avatar and name.
    Author,
    /// Relative or absolute date.
    Date,
    /// Short id.
    Id,
}

impl Column {
    /// All columns in default order.
    pub const ALL: [Column; 4] = [Column::Summary, Column::Author, Column::Date, Column::Id];

    /// Label for menus and headers.
    pub fn label(self) -> &'static str {
        match self {
            Column::Summary => say!("prefs-column-summary"),
            Column::Author => say!("prefs-column-author"),
            Column::Date => say!("prefs-column-date"),
            Column::Id => say!("prefs-column-id"),
        }
    }
}

/// Graph column configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Columns {
    /// Visible columns in display order (summary is always present).
    #[serde(default = "default_columns")]
    pub order: Vec<Column>,
    /// Show the date and time in full instead of how long ago.
    #[serde(default = "default_true")]
    pub absolute_dates: bool,
    /// Width of the message column in pixels, where another column follows
    /// it: the last column shown takes what is left.
    #[serde(default = "default_summary_width")]
    pub summary_width: f32,
    /// Width of the author column in pixels.
    #[serde(default = "default_author_width")]
    pub author_width: f32,
    /// Width of the branch and tag column in pixels.
    #[serde(default = "default_refs_width")]
    pub refs_width: f32,
    /// Width of the graph column in pixels, once its border was dragged;
    /// wider graphs scroll horizontally.
    #[serde(default = "default_graph_width")]
    pub graph_width: f32,
    /// The graph column is as wide as the graph's lanes need, up to a
    /// limit, until its border is dragged.
    #[serde(default = "default_true")]
    pub graph_auto: bool,
    /// The branch and tag column is shown.
    #[serde(default = "default_true")]
    pub show_refs: bool,
    /// The graph column is shown.
    #[serde(default = "default_true")]
    pub show_graph: bool,
    /// Mark where one day, week or month of history ends and the next
    /// begins, with a line and a label across the rows.
    #[serde(default = "default_true")]
    pub date_dividers: bool,
    /// Width of the date column in pixels; none while it is as wide as its
    /// dates need (dates in full need more than "3 days ago").
    #[serde(default)]
    pub date_width: Option<f32>,
    /// Width of the id column in pixels.
    #[serde(default = "default_id_width")]
    pub id_width: f32,
}

impl Columns {
    /// How wide the date column is.
    pub fn date_width(&self) -> f32 {
        self.date_width
            .unwrap_or(if self.absolute_dates { 175.0 } else { 84.0 })
    }

    /// How wide a column is when another one follows it; the last one
    /// shown takes what is left.
    pub fn width_of(&self, column: Column) -> f32 {
        match column {
            Column::Summary => self.summary_width,
            Column::Author => self.author_width,
            Column::Date => self.date_width(),
            Column::Id => self.id_width,
        }
    }
}

fn default_columns() -> Vec<Column> {
    vec![Column::Summary]
}
fn default_author_width() -> f32 {
    175.0
}
fn default_summary_width() -> f32 {
    400.0
}
fn default_refs_width() -> f32 {
    140.0
}
fn default_graph_width() -> f32 {
    172.0
}
fn default_id_width() -> f32 {
    72.0
}

impl Default for Columns {
    fn default() -> Self {
        Self {
            order: default_columns(),
            absolute_dates: true,
            summary_width: default_summary_width(),
            author_width: default_author_width(),
            refs_width: default_refs_width(),
            graph_width: default_graph_width(),
            graph_auto: true,
            show_refs: true,
            show_graph: true,
            date_dividers: true,
            date_width: None,
            id_width: default_id_width(),
        }
    }
}

/// How diffs are shown: what was last chosen in the viewer, which is
/// what the next diff starts with.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffPrefs {
    /// `hunks` (the changed parts only), or the whole file in
    /// `one_column` or `two_columns`.
    #[serde(default = "default_diff_layout")]
    pub layout: String,
    /// How whitespace counts when lines are compared: `exact`,
    /// `ignore_trailing` (at line ends), `ignore_all` or `ignore_cr_at_eol`
    /// (a carriage return at a line's end).
    #[serde(default = "default_diff_whitespace")]
    pub whitespace: String,
    /// Long lines go on under themselves instead of off to the right.
    #[serde(default)]
    pub wrap: bool,
    /// Inside a changed line, what changed is marked.
    #[serde(default = "default_true")]
    pub words: bool,
}

/// The layouts of a diff: the changed parts only, or the whole file in
/// one column or in two.
pub const DIFF_LAYOUTS: [&str; 3] = ["hunks", "one_column", "two_columns"];
/// The ways whitespace can count in a diff.
pub const DIFF_WHITESPACE: [&str; 4] =
    ["exact", "ignore_trailing", "ignore_all", "ignore_cr_at_eol"];

fn default_diff_layout() -> String {
    "hunks".to_owned()
}
fn default_diff_whitespace() -> String {
    "exact".to_owned()
}

impl Default for DiffPrefs {
    fn default() -> Self {
        Self {
            layout: default_diff_layout(),
            whitespace: default_diff_whitespace(),
            wrap: false,
            words: true,
        }
    }
}

impl DiffPrefs {
    /// A word the file holds that is none of the known ones is the
    /// default again.
    fn sanitize(&mut self) {
        if !DIFF_LAYOUTS.contains(&self.layout.as_str()) {
            self.layout = default_diff_layout();
        }
        if !DIFF_WHITESPACE.contains(&self.whitespace.as_str()) {
            self.whitespace = default_diff_whitespace();
        }
    }
}

/// How the terminal inside the window looks, and what it starts.
/// The section of the preferences called Terminal.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TerminalPrefs {
    /// The family of the text; none is the theme's monospace font.
    #[serde(default)]
    pub font_family: Option<String>,
    /// The size of the text, in points.
    #[serde(default = "default_terminal_size")]
    pub font_size: f32,
    /// How high a line is, as a multiple of the size.
    #[serde(default = "default_terminal_line_height")]
    pub line_height: f32,
    /// The cursor: `bar`, `block` or `underline`.
    #[serde(default = "default_terminal_cursor")]
    pub cursor: String,
    /// The text is dimmed while the keys are elsewhere.
    #[serde(default = "default_true")]
    pub dim_unfocused: bool,
    /// The command line of the shell to start; none is the system's (the
    /// user's login shell). Handed to `terminal.shell`.
    #[serde(default)]
    pub shell: Option<String>,
    /// Git commands typed at the shell's prompt are completed.
    #[serde(default = "default_true")]
    pub completion: bool,
    /// What Tab does while completions are on show: `shell` (it goes to
    /// the shell), `accept` (it takes the one marked), `next` (it marks
    /// the next one).
    #[serde(default = "default_completion_tab")]
    pub completion_tab: String,
}

/// The cursor styles of the terminal, with what the preferences call them.
pub const TERMINAL_CURSORS: [(&str, bitbraid_strings::Message); 3] = [
    ("bar", message!("prefs-cursor-bar")),
    ("block", message!("prefs-cursor-block")),
    ("underline", message!("prefs-cursor-underline")),
];
/// What Tab can do while completions are on show, with what the
/// preferences call it.
pub const COMPLETION_TABS: [(&str, bitbraid_strings::Message); 3] = [
    ("shell", message!("prefs-tab-shell")),
    ("accept", message!("prefs-tab-accept")),
    ("next", message!("prefs-tab-next")),
];

fn default_completion_tab() -> String {
    "shell".to_owned()
}

/// The least and the most the terminal's text and lines can be.
pub const TERMINAL_SIZES: (f32, f32) = (6.0, 48.0);
pub const TERMINAL_LINE_HEIGHTS: (f32, f32) = (1.0, 3.0);

fn default_terminal_size() -> f32 {
    12.0
}
fn default_terminal_line_height() -> f32 {
    1.2
}
fn default_terminal_cursor() -> String {
    "bar".to_owned()
}

impl Default for TerminalPrefs {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size: default_terminal_size(),
            line_height: default_terminal_line_height(),
            cursor: default_terminal_cursor(),
            dim_unfocused: true,
            shell: None,
            completion: true,
            completion_tab: default_completion_tab(),
        }
    }
}

impl TerminalPrefs {
    /// Numbers out of range and words that are none of the choices are
    /// the defaults again; an empty name is no name.
    fn sanitize(&mut self) {
        let within = |value: f32, (least, most): (f32, f32), default: f32| {
            if value.is_finite() {
                value.clamp(least, most)
            } else {
                default
            }
        };
        self.font_size = within(self.font_size, TERMINAL_SIZES, default_terminal_size());
        self.line_height = within(
            self.line_height,
            TERMINAL_LINE_HEIGHTS,
            default_terminal_line_height(),
        );
        if !TERMINAL_CURSORS
            .iter()
            .any(|(word, _)| *word == self.cursor)
        {
            self.cursor = default_terminal_cursor();
        }
        if !COMPLETION_TABS
            .iter()
            .any(|(word, _)| *word == self.completion_tab)
        {
            self.completion_tab = default_completion_tab();
        }
        for value in [&mut self.font_family, &mut self.shell] {
            if value
                .as_deref()
                .is_some_and(|text| text.trim().is_empty() || text.chars().count() > 1000)
            {
                *value = None;
            }
        }
    }
}

/// User preferences.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Preferences {
    #[serde(default)]
    pub theme: ThemeChoice,
    /// UI scale factor, 1.0 = 100 %.
    #[serde(default = "default_zoom")]
    pub zoom: f32,
    /// Show the references panel.
    #[serde(default = "default_true")]
    pub left_panel: bool,
    /// Show the details panel.
    #[serde(default = "default_true")]
    pub right_panel: bool,
    /// Graph columns.
    #[serde(default)]
    pub columns: Columns,
    /// Show changed files as a tree of folders instead of a list of paths.
    #[serde(default)]
    pub changes_tree: bool,
    /// List changed files Z to A instead of A to Z.
    #[serde(default)]
    pub changes_reversed: bool,
    /// Command that opens a file for editing, for example
    /// `code --goto {path}:{line}`. Without one the system's default
    /// application for the file is used.
    #[serde(default)]
    pub editor: Option<String>,
    /// Branches that are never deleted from here.
    #[serde(default = "default_protected")]
    pub protected_branches: Vec<String>,
    /// How pull brings the upstream in: `ff_only`, `ff`, `rebase` or `merge`.
    #[serde(default = "default_pull_mode")]
    pub pull_mode: String,
    /// Command that shows two versions of a file, written as git writes
    /// tool commands, for example `meld "$LOCAL" "$REMOTE"`. Without one
    /// the diff tool git is configured with is used.
    #[serde(default)]
    pub external_diff: Option<String>,
    /// Command that resolves a conflicted file, for example
    /// `meld "$LOCAL" "$BASE" "$REMOTE" --output "$MERGED"`. Without one
    /// the merge tool git is configured with is used.
    #[serde(default)]
    pub external_merge: Option<String>,
    /// The git executable to run, as a name on `PATH` or a path. Without
    /// one it is `git`.
    #[serde(default)]
    pub git_executable: Option<String>,
    /// Whether the buttons of the toolbar say what they are, above their
    /// icons. Without the words they say it under the pointer.
    #[serde(default = "default_true")]
    pub toolbar_labels: bool,
    /// Stop what moves: dialogs, notifications and menus
    /// appear at once, and the marks that turn while something runs stand
    /// still. Off, motion is as the system's own setting asks.
    #[serde(default)]
    pub reduce_motion: bool,
    /// Command that starts a terminal, for example `kitty --directory
    /// {path}`; it is started in the repository's folder either way.
    /// Without one, the terminal that is found on the system.
    #[serde(default)]
    pub terminal: Option<String>,
    /// Command that shows a folder, for example `nautilus {path}`. Without
    /// one, whatever the system opens folders with.
    #[serde(default)]
    pub file_manager: Option<String>,
    /// Fetch every remote of an open repository this often, in minutes,
    /// without being asked; nought is never.
    #[serde(default)]
    pub auto_fetch_minutes: u32,
    /// Fetch every tag of a remote, not only the ones on what is fetched.
    #[serde(default)]
    pub fetch_tags: bool,
    /// Forget, with each fetch, the branches that are gone on the remote.
    #[serde(default = "default_true")]
    pub fetch_prune: bool,
    /// When local changes are in the way of a checkout: stash them, check
    /// out and bring them back, without asking.
    #[serde(default)]
    pub checkout_stash: bool,
    /// Days without a commit after which a local branch counts as stale.
    #[serde(default = "default_stale_days")]
    pub stale_days: u32,
    /// The order of the graph's rows: `date`, newest first; `author`, by
    /// when each change was written, newest first; or `topo`, each line of
    /// history together. A commit comes before its parents in every one.
    #[serde(default = "default_graph_order")]
    pub graph_order: String,
    /// Which of a commit's two dates its row shows: `author`, when the
    /// change was written, or `commit`, when the commit was made (which a
    /// rebase or an amend renews).
    #[serde(default = "default_graph_date")]
    pub graph_date: String,
    /// How diffs are shown.
    #[serde(default)]
    pub diff: DiffPrefs,
    /// Ask a picture service for the pictures of authors. Off
    /// unless the user turns it on: on, a hash of the e-mail address of
    /// each author that comes into view is sent to that service.
    #[serde(default)]
    pub gravatar: bool,
    /// Another service that answers as Gravatar does, as the address a
    /// hash is added to (`https://host/avatar`). Without one, Gravatar's.
    #[serde(default)]
    pub gravatar_base: Option<String>,
    /// Ask the providers of the connected accounts for pictures:
    /// of the accounts, of the owners of the remotes linked to them, and
    /// of the authors whose e-mail addresses a provider knows. Off unless
    /// the user turns it on.
    #[serde(default)]
    pub provider_pictures: bool,
    /// Name the issue a branch's name holds in the messages committed on
    /// it, as a `Refs: #123` trailer the commit box adds.
    #[serde(default = "default_true")]
    pub issue_trailer: bool,
    /// How a branch made for an issue is named, with `{number}` and
    /// `{title}`.
    #[serde(default = "default_issue_branch")]
    pub issue_branch: String,
    /// The terminal inside the window.
    #[serde(default)]
    pub terminal_panel: TerminalPrefs,
    /// Look for a newer version of BitBraid: at the start and
    /// once a day, one request to the feed published with its releases.
    /// Off unless the user turns it on; off, nothing is asked.
    #[serde(default)]
    pub updates: bool,
}

/// The orders the graph can be in, with what the preferences call them.
pub const GRAPH_ORDERS: &[(&str, bitbraid_strings::Message)] = &[
    ("date", message!("prefs-order-date")),
    ("author", message!("prefs-order-author")),
    ("topo", message!("prefs-order-topo")),
];

/// The dates a row can show, with what the preferences call them.
pub const GRAPH_DATES: &[(&str, bitbraid_strings::Message)] = &[
    ("author", message!("prefs-date-author")),
    ("commit", message!("prefs-date-commit")),
];

fn default_graph_order() -> String {
    "date".to_owned()
}

fn default_graph_date() -> String {
    "author".to_owned()
}

fn default_stale_days() -> u32 {
    90
}

fn default_pull_mode() -> String {
    "ff".to_owned()
}

fn default_protected() -> Vec<String> {
    vec!["main".to_owned(), "master".to_owned()]
}

fn default_zoom() -> f32 {
    1.0
}
fn default_true() -> bool {
    true
}

/// How a branch for an issue is named unless the user says otherwise.
pub fn default_issue_branch() -> String {
    bitbraid_git::issue_keys::DEFAULT_BRANCH_TEMPLATE.to_owned()
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Dark,
            zoom: 1.0,
            left_panel: true,
            right_panel: true,
            columns: Columns::default(),
            changes_tree: false,
            changes_reversed: false,
            editor: None,
            protected_branches: default_protected(),
            pull_mode: default_pull_mode(),
            external_diff: None,
            external_merge: None,
            git_executable: None,
            toolbar_labels: true,
            reduce_motion: false,
            terminal: None,
            file_manager: None,
            auto_fetch_minutes: 0,
            fetch_tags: false,
            fetch_prune: true,
            checkout_stash: false,
            stale_days: default_stale_days(),
            graph_order: default_graph_order(),
            graph_date: default_graph_date(),
            diff: DiffPrefs::default(),
            gravatar: false,
            gravatar_base: None,
            provider_pictures: false,
            issue_trailer: true,
            issue_branch: default_issue_branch(),
            terminal_panel: TerminalPrefs::default(),
            updates: false,
        }
    }
}

impl Preferences {
    /// The order the graph is walked in.
    pub fn order(&self) -> bitbraid_git::Order {
        match self.graph_order.as_str() {
            "topo" => bitbraid_git::Order::Topo,
            "author" => bitbraid_git::Order::AuthorDate,
            _ => bitbraid_git::Order::Date,
        }
    }

    /// The order after the one in use, as the preferences call it: the
    /// three in turn, for what switches the order with one key.
    pub fn next_order(&self) -> &'static str {
        let at = GRAPH_ORDERS
            .iter()
            .position(|(word, _)| *word == self.graph_order)
            .unwrap_or(0);
        GRAPH_ORDERS
            .get(at.saturating_add(1))
            .or(GRAPH_ORDERS.first())
            .map_or("date", |(word, _)| *word)
    }

    /// Whether rows show the commit date rather than the author date.
    pub fn commit_dates(&self) -> bool {
        self.graph_date == "commit"
    }

    /// Bring numbers a hand-edited file may hold into the range the
    /// interface can show: a zoom of a million per cent is not one.
    pub fn sanitize(&mut self) {
        self.zoom = if self.zoom.is_finite() {
            self.zoom.clamp(0.5, 3.0)
        } else {
            1.0
        };
        let width = |value: &mut f32, default: f32| {
            *value = if value.is_finite() {
                value.clamp(40.0, 2000.0)
            } else {
                default
            };
        };
        width(&mut self.columns.author_width, default_author_width());
        width(&mut self.columns.summary_width, default_summary_width());
        width(&mut self.columns.refs_width, default_refs_width());
        width(&mut self.columns.graph_width, default_graph_width());
        width(&mut self.columns.id_width, default_id_width());
        // A date column of no sensible width is as wide as its dates need.
        self.columns.date_width = self
            .columns
            .date_width
            .filter(|width| width.is_finite())
            .map(|width| width.clamp(40.0, 2000.0));
        // The message can be put away, but not every column with it.
        if self.columns.order.is_empty() {
            self.columns.order.push(Column::Summary);
        }
        let mut seen = Vec::new();
        self.columns.order.retain(|column| {
            let first = !seen.contains(column);
            seen.push(*column);
            first
        });
        let blank = |value: &mut Option<String>| {
            if value.as_deref().is_some_and(|text| text.trim().is_empty()) {
                *value = None;
            }
        };
        if let ThemeChoice::User(name) = &self.theme
            && (name.trim().is_empty() || name.chars().count() > 60)
        {
            self.theme = ThemeChoice::Dark;
        }
        blank(&mut self.editor);
        blank(&mut self.external_diff);
        blank(&mut self.external_merge);
        blank(&mut self.git_executable);
        blank(&mut self.terminal);
        blank(&mut self.file_manager);
        blank(&mut self.gravatar_base);
        // Once a day is the least often a timer is worth having.
        self.auto_fetch_minutes = self.auto_fetch_minutes.min(24 * 60);
        // A day at least, ten years at most.
        self.stale_days = self.stale_days.clamp(1, 3650);
        // A word that is none of the choices is the default.
        if !GRAPH_ORDERS
            .iter()
            .any(|(word, _)| *word == self.graph_order)
        {
            self.graph_order = default_graph_order();
        }
        if !GRAPH_DATES.iter().any(|(word, _)| *word == self.graph_date) {
            self.graph_date = default_graph_date();
        }
        self.diff.sanitize();
        self.terminal_panel.sanitize();
    }
}

/// Per-repository remembered layout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct RepoState {
    /// Width of the references panel in pixels.
    pub left: Option<f32>,
    /// Width of the details panel in pixels.
    pub right: Option<f32>,
    /// Hidden references (full names).
    #[serde(default)]
    pub hidden: Vec<String>,
    /// The references the graph shows alone, with their history (full
    /// names); empty means none.
    #[serde(default)]
    pub only: Vec<String>,
    /// Branches pinned to the left of the graph (full names), in the
    /// order of their lanes.
    #[serde(default)]
    pub pinned: Vec<String>,
    /// Selected commit.
    pub selected: Option<String>,
    /// How this repository pulls, when not the way the preferences say.
    #[serde(default)]
    pub pull_mode: Option<String>,
    /// The commit at the top of the graph as it was last scrolled.
    #[serde(default)]
    pub top: Option<String>,
    /// How far into that commit's row the graph was scrolled, in points:
    /// the part of the row out of sight over the list. None for none.
    #[serde(default)]
    pub top_into: Option<f32>,
    /// The file that was open in the middle: its path, and the commit it
    /// was opened from (none for a file of the worktree).
    #[serde(default)]
    pub open_file: Option<OpenFile>,
    /// Which sections of the references panel are closed; none while the
    /// panel is as it starts out.
    #[serde(default)]
    pub closed_sections: Option<Vec<String>>,
    /// The folders of the references panel that are closed, each as
    /// `section:path` (a remote's row is the folder of its branches).
    #[serde(default)]
    pub closed_folders: Vec<String>,
    /// The sections of the references panel in the order they are shown;
    /// empty while they are in the order they start out in.
    #[serde(default)]
    pub section_order: Vec<String>,
    /// The sections of the references panel that are not shown.
    #[serde(default)]
    pub hidden_sections: Vec<String>,
    /// How the open sections share the height of the panel, once the edge
    /// between two of them was dragged: a weight for each.
    #[serde(default)]
    pub section_sizes: std::collections::BTreeMap<String, f32>,
    /// The section that has the room of all the others, if one has.
    #[serde(default)]
    pub maximised_section: Option<String>,
    /// Whether the references panel shows in this repository's tab, when
    /// that was chosen there; the preference otherwise.
    #[serde(default)]
    pub left_panel: Option<bool>,
    /// The same for the details panel.
    #[serde(default)]
    pub right_panel: Option<bool>,
    /// The other worktrees as they were last looked at: what is
    /// uncommitted in each, and the stamp that says whether it has to be
    /// looked at again. Kept so that opening the repository
    /// does not start with a `git status` in every one of them.
    #[serde(default)]
    pub worktrees_seen: Vec<SeenWorktree>,
    /// Whether blame passes over the revisions the repository lists for
    /// that, when that was chosen here; as git does it otherwise.
    #[serde(default)]
    pub blame_ignore_revs: Option<bool>,
    /// Whether the terminal panel is on show under the graph.
    #[serde(default)]
    pub terminal_shown: bool,
    /// Whether it shows only its heading.
    #[serde(default)]
    pub terminal_minimised: bool,
    /// How high it was dragged; none while it is as high as it starts.
    #[serde(default)]
    pub terminal_height: Option<f32>,
    /// The filters made in the pull requests section here.
    #[serde(default)]
    pub pull_filters: Vec<crate::views::pulls::facts::SavedFilter>,
    /// The groups of the pull requests section that are closed, by key,
    /// and the remotes closed in them (`group/remote`).
    #[serde(default)]
    pub closed_pull_groups: Vec<String>,
    /// The remote whose issues the issues section shows, when one was
    /// chosen there.
    #[serde(default)]
    pub issue_repo: Option<String>,
    /// The provider's query the issues are asked with here, if any.
    #[serde(default)]
    pub issue_query: Option<String>,
    /// The groups of the issues section that are closed, by key.
    #[serde(default)]
    pub closed_issue_groups: Vec<String>,
}

/// One of the other worktrees of a repository as it was last looked at.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SeenWorktree {
    /// Its working directory.
    pub path: String,
    /// Its HEAD and the size and time of its index when it was looked at.
    #[serde(default)]
    pub stamp: String,
    /// Its changed files by kind.
    #[serde(default, flatten)]
    pub counts: bitbraid_git::wip::Counts,
}

/// How many branches can be pinned: each is a column at the left of the
/// graph for as long as its branch runs.
pub const MAX_PINNED: usize = 8;

impl RepoState {
    /// The other worktrees that had uncommitted work when they were last
    /// looked at, by path: the ones that get a row in the graph.
    pub fn worktree_rows(&self) -> Vec<String> {
        self.worktrees_seen
            .iter()
            .take(bitbraid_git::wip::MAX_SCANNED)
            .filter(|seen| seen.counts.total() > 0)
            .map(|seen| seen.path.clone())
            .collect()
    }

    /// Pin a branch to the left of the graph, behind what is pinned
    /// already, or unpin it. False when there is no room for one more.
    pub fn toggle_pinned(&mut self, full_name: &str) -> bool {
        if self.pinned.iter().any(|name| name == full_name) {
            self.pinned.retain(|name| name != full_name);
            return true;
        }
        // What a hand-edited file holds beyond the limit is cut here.
        self.pinned.truncate(MAX_PINNED);
        if self.pinned.len() >= MAX_PINNED {
            return false;
        }
        self.pinned.push(full_name.to_owned());
        true
    }
}

/// A file that was open in the middle of a tab.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct OpenFile {
    /// Path relative to the worktree.
    pub path: String,
    /// The commit it was opened from; none for a file of the worktree.
    #[serde(default)]
    pub rev: Option<String>,
}

/// Remembered application state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct State {
    /// Open repositories in tab order.
    #[serde(default)]
    pub open: Vec<PathBuf>,
    /// Active tab index.
    #[serde(default)]
    pub active: usize,
    /// Recently opened repositories, newest first.
    #[serde(default)]
    pub recent: Vec<PathBuf>,
    /// Favourite repositories.
    #[serde(default)]
    pub favourites: Vec<PathBuf>,
    /// The folder the last clone went into, offered for the next one.
    #[serde(default)]
    pub clone_dir: Option<PathBuf>,
    /// Per-repository layout keyed by path.
    #[serde(default)]
    pub repos: HashMap<String, RepoState>,
    /// What a repository's tab is called when it was given a name of its
    /// own, keyed by path.
    #[serde(default)]
    pub aliases: HashMap<String, String>,
    /// The profile the application was last switched to, when it was.
    #[serde(default)]
    pub profile: Option<String>,
    /// The tabs that were open under each profile when it was left, by
    /// profile name; the tabs from before any profile was switched to are
    /// under the empty name.
    #[serde(default)]
    pub profile_tabs: HashMap<String, TabSet>,
    /// How the page of repositories was left: the order of its sections,
    /// the hidden ones, the widths of its columns.
    #[serde(default)]
    pub repositories_page: RepositoriesPage,
}

/// How the page of repositories is laid out, as it was left.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default)]
pub struct RepositoriesPage {
    /// The sections by their keys (`open`, `favourites`, `recent`, `all`,
    /// `workspace:<id>`), in the order they are shown; a section that is
    /// not named comes after the ones that are.
    pub section_order: Vec<String>,
    /// The sections that are not shown, by their keys.
    pub hidden_sections: Vec<String>,
    /// The width of the column of names, and of the column of remotes in
    /// a workspace's rows, in pixels; none while they are as they start.
    pub name_width: Option<f32>,
    pub remote_width: Option<f32>,
    /// How the pull button of a workspace pulls (`remote.pull`'s modes),
    /// when another way than the preferences' was chosen there.
    pub pull_mode: Option<String>,
}

/// A set of tabs: the repositories that were open and which was shown.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct TabSet {
    /// Open repositories in tab order.
    #[serde(default)]
    pub open: Vec<PathBuf>,
    /// Which of them was shown.
    #[serde(default)]
    pub active: usize,
}

/// One of the two lists of repositories on the start page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepoList {
    /// The starred ones.
    Favourites,
    /// The ones opened last.
    Recent,
}

impl State {
    fn list_mut(&mut self, list: RepoList) -> &mut Vec<PathBuf> {
        match list {
            RepoList::Favourites => &mut self.favourites,
            RepoList::Recent => &mut self.recent,
        }
    }

    /// Put the entry `path` of a list where the entry `before` is (or at
    /// the end when `before` is absent or not in the list). False when
    /// nothing moved.
    pub fn move_entry(&mut self, list: RepoList, path: &Path, before: Option<&Path>) -> bool {
        let entries = self.list_mut(list);
        let Some(from) = entries.iter().position(|entry| entry == path) else {
            return false;
        };
        let to = before
            .and_then(|before| entries.iter().position(|entry| entry == before))
            .unwrap_or(entries.len().saturating_sub(1));
        if from == to {
            return false;
        }
        let entry = entries.remove(from);
        entries.insert(to.min(entries.len()), entry);
        true
    }

    /// Move an entry of a list one place up or down. False at the edge.
    pub fn nudge_entry(&mut self, list: RepoList, path: &Path, down: bool) -> bool {
        let entries = self.list_mut(list);
        let Some(from) = entries.iter().position(|entry| entry == path) else {
            return false;
        };
        let to = if down {
            from.saturating_add(1)
        } else if from == 0 {
            return false;
        } else {
            from.saturating_sub(1)
        };
        if to >= entries.len() {
            return false;
        }
        entries.swap(from, to);
        true
    }

    /// Take a repository out of a list. What is remembered about it (its
    /// layout, its tab's name) stays for when it is opened again.
    pub fn remove_entry(&mut self, list: RepoList, path: &Path) -> bool {
        let entries = self.list_mut(list);
        let before = entries.len();
        entries.retain(|entry| entry != path);
        entries.len() != before
    }

    /// A repository's folder is somewhere else now: everything that was
    /// remembered under the old path is remembered under the new one. A
    /// list that has both keeps one. False when nothing knew the old path.
    pub fn relocate(&mut self, old: &Path, new: &Path) -> bool {
        if old == new {
            return false;
        }
        let mut known = false;
        let mut lists: Vec<&mut Vec<PathBuf>> =
            vec![&mut self.recent, &mut self.favourites, &mut self.open];
        lists.extend(self.profile_tabs.values_mut().map(|tabs| &mut tabs.open));
        for list in lists {
            if !list.iter().any(|entry| entry == old) {
                continue;
            }
            known = true;
            if list.iter().any(|entry| entry == new) {
                list.retain(|entry| entry != old);
            } else {
                for entry in list.iter_mut().filter(|entry| entry.as_path() == old) {
                    *entry = new.to_path_buf();
                }
            }
        }
        let (old_key, new_key) = (
            old.to_string_lossy().into_owned(),
            new.to_string_lossy().into_owned(),
        );
        if let Some(state) = self.repos.remove(&old_key) {
            known = true;
            self.repos.entry(new_key.clone()).or_insert(state);
        }
        if let Some(alias) = self.aliases.remove(&old_key) {
            known = true;
            self.aliases.entry(new_key).or_insert(alias);
        }
        known
    }

    /// A profile was given another name: what was kept under its old name
    /// goes with it, the tabs that were open under it and that it is the
    /// one switched to. A profile it took the place of (one of the new
    /// name) leaves its tabs to it.
    pub fn rename_profile(&mut self, from: &str, to: &str) {
        if from == to {
            return;
        }
        if let Some(tabs) = self.profile_tabs.remove(from) {
            self.profile_tabs.insert(to.to_owned(), tabs);
        }
        if self.profile.as_deref() == Some(from) {
            self.profile = Some(to.to_owned());
        }
    }

    /// Switch to another profile: the tabs that are open are put away
    /// under the profile that is left, and the tabs that were open under
    /// the one switched to are handed back, when it has been used before.
    /// Switching to the profile that is current changes nothing.
    pub fn switch_profile(
        &mut self,
        to: &str,
        open: Vec<PathBuf>,
        active: usize,
    ) -> Option<TabSet> {
        let leaving = self.profile.clone().unwrap_or_default();
        if leaving == to {
            return None;
        }
        self.profile_tabs.insert(leaving, TabSet { open, active });
        self.profile = Some(to.to_owned());
        // At most 64 sets are kept: past that, sets go by name, never the
        // current profile's nor the one of no profile.
        if self.profile_tabs.len() > 64 {
            let current = to.to_owned();
            let mut names: Vec<String> = self.profile_tabs.keys().cloned().collect();
            names.sort();
            for name in names {
                if self.profile_tabs.len() <= 64 {
                    break;
                }
                if name != current && !name.is_empty() {
                    self.profile_tabs.remove(&name);
                }
            }
        }
        self.profile_tabs.get(to).cloned()
    }
}

/// The preferences store, a GPUI global.
#[derive(Debug)]
pub struct Prefs {
    dir: Option<PathBuf>,
    pub preferences: Preferences,
    pub state: State,
    /// What went wrong while the files were read, to be said once there
    /// is a window to say it in.
    pub problems: Vec<String>,
}

impl Global for Prefs {}

/// Read a file of ours. One that is not there gives the defaults and says
/// nothing. One that is there and cannot be read as what it should be is
/// moved aside under another name, so that the next save does not write
/// over what the user had, and the defaults are used; the second value
/// then says what happened.
fn read_toml<T: serde::de::DeserializeOwned + Default>(path: &Path) -> (T, Option<String>) {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let error = match fs::read(path) {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return (T::default(), None),
        Err(err) => err.to_string(),
        Ok(bytes) => match String::from_utf8(bytes) {
            Err(_) => say!("prefs-not-text").to_owned(),
            Ok(text) => match toml::from_str(&text) {
                Ok(value) => return (value, None),
                Err(err) => first_line(&err.to_string()),
            },
        },
    };
    let said = match set_aside(path) {
        Some(kept) => say!(
            "prefs-unreadable-kept",
            name,
            error,
            kept = kept.display().to_string()
        ),
        None => say!("prefs-unreadable-not-moved", name, error),
    };
    (T::default(), Some(said))
}

/// The first line of an error, which for a TOML error says what and where.
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_owned()
}

/// Move a file that cannot be read out of the way: `preferences.toml`
/// becomes `preferences.broken.toml`, or `preferences.broken-2.toml` when
/// that name is taken, and so on. Where it went, when it could be moved.
fn set_aside(path: &Path) -> Option<PathBuf> {
    let stem = path.file_stem()?.to_string_lossy().into_owned();
    let extension = path
        .extension()
        .map(|extension| extension.to_string_lossy().into_owned())
        .unwrap_or_else(|| "toml".to_owned());
    let candidates = std::iter::once(format!("{stem}.broken.{extension}"))
        .chain((2..=99).map(|n| format!("{stem}.broken-{n}.{extension}")));
    for name in candidates {
        let target = path.with_file_name(name);
        if target.symlink_metadata().is_ok() {
            continue;
        }
        return fs::rename(path, &target).ok().map(|()| target);
    }
    None
}

fn write_toml<T: Serialize>(path: &Path, value: &T) {
    let Ok(text) = toml::to_string_pretty(value) else {
        return;
    };
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let tmp = path.with_extension("toml.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, path);
    }
}

/// The platform's configuration directory for the application
/// (`~/.config/bitbraid` on Linux), when the platform has one.
pub fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "bitbraid").map(|d| d.config_dir().to_path_buf())
}

/// The platform's cache directory for the application
/// (`~/.cache/bitbraid` on Linux): for what can be had again.
pub fn cache_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("", "", "bitbraid").map(|d| d.cache_dir().to_path_buf())
}

impl Prefs {
    /// Load from the platform config directory (`~/.config/bitbraid` on Linux).
    pub fn load() -> Self {
        Self::load_from(config_dir())
    }

    /// Load from a given directory, or with nothing kept anywhere.
    pub fn load_from(dir: Option<PathBuf>) -> Self {
        let mut problems = Vec::new();
        let (mut preferences, state): (Preferences, State) = match &dir {
            Some(dir) => {
                let (preferences, problem) = read_toml(&dir.join("preferences.toml"));
                problems.extend(problem);
                let (state, problem) = read_toml(&dir.join("state.toml"));
                problems.extend(problem);
                (preferences, state)
            }
            None => (Preferences::default(), State::default()),
        };
        preferences.sanitize();
        Self {
            dir,
            preferences,
            state,
            problems,
        }
    }

    /// Where files are stored.
    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// Persist preferences.
    pub fn save_preferences(&self) {
        if let Some(dir) = &self.dir {
            write_toml(&dir.join("preferences.toml"), &self.preferences);
        }
    }

    /// Persist state.
    pub fn save_state(&self) {
        if let Some(dir) = &self.dir {
            write_toml(&dir.join("state.toml"), &self.state);
        }
    }

    /// Record a repository as recently opened (most recent first, 20 kept).
    pub fn touch_recent(&mut self, path: &Path) {
        self.state.recent.retain(|p| p != path);
        self.state.recent.insert(0, path.to_path_buf());
        self.state.recent.truncate(20);
        self.save_state();
    }

    /// Give a repository's tab a name of its own; an empty name takes it
    /// away again. Kept to what fits a tab.
    pub fn set_alias(&mut self, path: &Path, name: &str) {
        let key = path.to_string_lossy().into_owned();
        let name: String = name
            .trim()
            .chars()
            .filter(|c| !c.is_control())
            .take(60)
            .collect();
        if name.is_empty() {
            self.state.aliases.remove(&key);
        } else {
            self.state.aliases.insert(key, name);
        }
        self.save_state();
    }

    /// Whether the references panel shows in the tab of this repository:
    /// what was chosen there, else the preference.
    pub fn left_panel_shown(&self, path: Option<&Path>) -> bool {
        path.and_then(|path| self.state.repos.get(&path.to_string_lossy().into_owned()))
            .and_then(|state| state.left_panel)
            .unwrap_or(self.preferences.left_panel)
    }

    /// The same for the details panel.
    pub fn right_panel_shown(&self, path: Option<&Path>) -> bool {
        path.and_then(|path| self.state.repos.get(&path.to_string_lossy().into_owned()))
            .and_then(|state| state.right_panel)
            .unwrap_or(self.preferences.right_panel)
    }

    /// Show or hide a panel in the tab of a repository, or, with no
    /// repository, wherever nothing else was chosen.
    pub fn set_panel_shown(&mut self, path: Option<&Path>, left: bool, shown: bool) {
        match path {
            Some(path) => self.update_repo_state(path, |state| {
                if left {
                    state.left_panel = Some(shown);
                } else {
                    state.right_panel = Some(shown);
                }
            }),
            None => {
                if left {
                    self.preferences.left_panel = shown;
                } else {
                    self.preferences.right_panel = shown;
                }
                self.save_preferences();
            }
        }
    }

    /// Layout state for a repository.
    pub fn repo_state(&self, path: &Path) -> RepoState {
        self.state
            .repos
            .get(&path.to_string_lossy().into_owned())
            .cloned()
            .unwrap_or_default()
    }

    /// Give a repository that has no layout state yet the state of
    /// another one: a linked worktree starts out looking like the main
    /// one. The selected commit is not taken over. True when it was done.
    pub fn inherit_repo_state(&mut self, path: &Path, from: &Path) -> bool {
        let key = path.to_string_lossy().into_owned();
        if self.state.repos.contains_key(&key) {
            return false;
        }
        let Some(state) = self.state.repos.get(&from.to_string_lossy().into_owned()) else {
            return false;
        };
        // What is its own: the commit it was on, where it was scrolled to
        // and the file it had open.
        let state = RepoState {
            selected: None,
            top: None,
            top_into: None,
            open_file: None,
            ..state.clone()
        };
        self.state.repos.insert(key, state);
        self.save_state();
        true
    }

    /// Update layout state for a repository.
    pub fn update_repo_state(&mut self, path: &Path, f: impl FnOnce(&mut RepoState)) {
        let entry = self
            .state
            .repos
            .entry(path.to_string_lossy().into_owned())
            .or_default();
        f(entry);
        self.save_state();
    }
}

/// Install the global, loaded from the platform's configuration directory.
pub fn init(cx: &mut App) {
    install(Prefs::load(), cx);
}

/// Install preferences that were loaded already.
pub fn install(prefs: Prefs, cx: &mut App) {
    // The engine keeps the profiles beside the preferences.
    crate::session::set_config_dir(prefs.dir.clone());
    cx.set_global(prefs);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferences_round_trip_through_toml() {
        let prefs = Preferences {
            theme: ThemeChoice::Light,
            zoom: 1.25,
            left_panel: false,
            right_panel: true,
            columns: Columns::default(),
            changes_tree: true,
            changes_reversed: true,
            editor: Some("code --goto {path}:{line}".to_owned()),
            protected_branches: vec!["trunk".to_owned()],
            pull_mode: "rebase".to_owned(),
            external_diff: Some("meld \"$LOCAL\" \"$REMOTE\"".to_owned()),
            external_merge: None,
            git_executable: Some("/opt/git/bin/git".to_owned()),
            toolbar_labels: false,
            reduce_motion: true,
            terminal: Some("kitty --directory {path}".to_owned()),
            file_manager: Some("nautilus {path}".to_owned()),
            auto_fetch_minutes: 15,
            fetch_tags: true,
            fetch_prune: false,
            checkout_stash: true,
            stale_days: 30,
            graph_order: "topo".to_owned(),
            graph_date: "commit".to_owned(),
            diff: DiffPrefs {
                layout: "two_columns".to_owned(),
                whitespace: "ignore_trailing".to_owned(),
                wrap: true,
                words: false,
            },
            gravatar: true,
            gravatar_base: Some("https://pictures.example.org/avatar".to_owned()),
            provider_pictures: true,
            issue_trailer: false,
            issue_branch: "fix/{number}-{title}".to_owned(),
            terminal_panel: TerminalPrefs {
                font_family: Some("Iosevka".to_owned()),
                font_size: 14.0,
                line_height: 1.5,
                cursor: "underline".to_owned(),
                dim_unfocused: false,
                shell: Some("fish --login".to_owned()),
                completion: false,
                completion_tab: "next".to_owned(),
            },
            updates: true,
        };
        let text = toml::to_string(&prefs).unwrap();
        let back: Preferences = toml::from_str(&text).unwrap();
        assert_eq!(back, prefs);
        let defaults: Preferences = toml::from_str("").unwrap();
        assert_eq!(defaults, Preferences::default());
        // Nobody is asked for pictures unless the file says so.
        assert!(!defaults.gravatar);
        assert_eq!(defaults.gravatar_base, None);
        // Nor about updates.
        assert!(!defaults.updates);
    }

    #[test]
    fn a_file_that_cannot_be_read_is_kept_aside_and_said() {
        let dir = std::env::temp_dir().join(format!("bitbraid-broken-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let file = dir.join("preferences.toml");

        // Nothing there: the defaults, and nothing to say.
        let prefs = Prefs::load_from(Some(dir.clone()));
        assert_eq!(prefs.preferences, Preferences::default());
        assert!(prefs.problems.is_empty());

        // Not TOML, the wrong type for a value, not text at all: each is
        // kept under a name of its own.
        let broken: [&[u8]; 3] = [
            b"theme = \"light\"\nzoom = = 2\n",
            b"zoom = \"large\"\n",
            b"\xff\xfe\x00 not text",
        ];
        let names = [
            "preferences.broken.toml",
            "preferences.broken-2.toml",
            "preferences.broken-3.toml",
        ];
        for (content, name) in broken.iter().zip(names) {
            fs::write(&file, content).unwrap();
            let prefs = Prefs::load_from(Some(dir.clone()));
            assert_eq!(prefs.preferences, Preferences::default());
            assert_eq!(prefs.problems.len(), 1, "{:?}", prefs.problems);
            assert!(prefs.problems[0].contains(name), "{:?}", prefs.problems);
            assert!(prefs.problems[0].starts_with("preferences.toml could not be read"));
            assert!(!file.exists(), "moved aside");
            assert_eq!(fs::read(dir.join(name)).unwrap(), *content);
            // Saving afterwards writes a fresh file and leaves the kept one.
            prefs.save_preferences();
            assert!(file.is_file());
            assert_eq!(fs::read(dir.join(name)).unwrap(), *content);
            let again = Prefs::load_from(Some(dir.clone()));
            assert!(again.problems.is_empty());
        }

        // The state file is treated the same, and both can be broken at once.
        fs::write(&file, "zoom = [").unwrap();
        fs::write(dir.join("state.toml"), "active = \"second\"").unwrap();
        let prefs = Prefs::load_from(Some(dir.clone()));
        assert_eq!(prefs.problems.len(), 2);
        assert!(prefs.problems[1].contains("state.broken.toml"));
        assert_eq!(prefs.state, State::default());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_that_still_names_the_graphs_minimap_is_read_all_the_same() {
        // A key the application does not know is passed over, and the rest
        // of what the file says counts.
        let prefs: Preferences =
            toml::from_str("[columns]\nminimap = false\nid_width = 90.0\n").unwrap();
        assert_eq!(prefs.columns.id_width, 90.0);
        // What is written back does not name it.
        let written = toml::to_string(&prefs).unwrap();
        assert!(!written.contains("minimap"), "{written}");
    }

    #[test]
    fn numbers_from_a_hand_edited_file_are_brought_into_range() {
        let mut prefs: Preferences = toml::from_str(
            "zoom = 1e30\neditor = \"  \"\n[columns]\norder = [\"id\", \"id\", \"date\"]\n\
             author_width = -5.0\ngraph_width = nan\nrefs_width = 1e9\n",
        )
        .unwrap();
        prefs.sanitize();
        assert_eq!(prefs.zoom, 3.0);
        assert_eq!(prefs.editor, None);
        // Each column once; the message can be put away like the others.
        assert_eq!(prefs.columns.order, [Column::Id, Column::Date]);
        let mut none: Preferences = toml::from_str("[columns]\norder = []\n").unwrap();
        none.sanitize();
        assert_eq!(none.columns.order, [Column::Summary], "not every column");
        assert_eq!(prefs.columns.author_width, 40.0);
        assert_eq!(prefs.columns.graph_width, default_graph_width());
        assert_eq!(prefs.columns.refs_width, 2000.0);
        // The date and id columns have widths too: the date
        // column's is its dates' until it is dragged, the date and time in
        // full unless how long ago is asked for.
        assert_eq!(prefs.columns.date_width(), 175.0);
        assert_eq!(prefs.columns.id_width, 72.0);
        let mut ago: Preferences = toml::from_str("[columns]\nabsolute_dates = false\n").unwrap();
        ago.sanitize();
        assert_eq!(ago.columns.date_width(), 84.0);
        let mut wide: Preferences =
            toml::from_str("[columns]\nabsolute_dates = true\ndate_width = nan\nid_width = -1.0\n")
                .unwrap();
        wide.sanitize();
        assert_eq!(wide.columns.date_width, None);
        assert_eq!(wide.columns.date_width(), 175.0);
        assert_eq!(wide.columns.id_width, 40.0);
        let mut set: Preferences =
            toml::from_str("[columns]\ndate_width = 1e9\nid_width = 300.0\n").unwrap();
        set.sanitize();
        assert_eq!(set.columns.date_width(), 2000.0);
        assert_eq!(set.columns.width_of(Column::Id), 300.0);
        // The message has one too, for when a column follows it.
        assert_eq!(set.columns.width_of(Column::Summary), 400.0);
        assert_eq!(
            set.columns.width_of(Column::Author),
            set.columns.author_width
        );
        let mut tiny: Preferences = toml::from_str("zoom = -inf").unwrap();
        tiny.sanitize();
        assert_eq!(tiny.zoom, 1.0);
        let mut small: Preferences = toml::from_str("zoom = 0.01").unwrap();
        small.sanitize();
        assert_eq!(small.zoom, 0.5);
        // The timer is off unless it is asked for, and at most a day.
        assert_eq!(small.auto_fetch_minutes, 0);
        assert!(!small.fetch_tags);
        assert!(small.fetch_prune);
        let mut eager: Preferences =
            toml::from_str("auto_fetch_minutes = 4000000000\nfetch_tags = true").unwrap();
        eager.sanitize();
        assert_eq!(eager.auto_fetch_minutes, 1440);
        assert_eq!(eager.stale_days, 90);
        let mut never: Preferences = toml::from_str("stale_days = 0").unwrap();
        never.sanitize();
        assert_eq!(never.stale_days, 1);
        assert!(eager.fetch_tags);
    }

    #[test]
    fn the_order_and_the_date_of_the_graph_are_words_with_defaults() {
        // Nothing in the file: by date, author dates.
        let plain: Preferences = toml::from_str("").unwrap();
        assert_eq!(plain.order(), bitbraid_git::Order::Date);
        assert!(!plain.commit_dates());
        let chosen: Preferences =
            toml::from_str("graph_order = \"topo\"\ngraph_date = \"commit\"\n").unwrap();
        assert_eq!(chosen.order(), bitbraid_git::Order::Topo);
        // By the author's date, and the three in turn.
        let mut turning: Preferences = toml::from_str("graph_order = \"author\"\n").unwrap();
        turning.sanitize();
        assert_eq!(turning.order(), bitbraid_git::Order::AuthorDate);
        assert_eq!(turning.next_order(), "topo");
        turning.graph_order = turning.next_order().to_owned();
        assert_eq!(turning.next_order(), "date");
        turning.graph_order = turning.next_order().to_owned();
        assert_eq!(turning.next_order(), "author");
        turning.graph_order = "sideways".to_owned();
        assert_eq!(turning.next_order(), "author");
        assert!(chosen.commit_dates());
        // What is written comes back.
        let text = toml::to_string_pretty(&chosen).unwrap();
        let back: Preferences = toml::from_str(&text).unwrap();
        assert_eq!(back, chosen);
        // A word that is none of the choices does not cost the file: it is
        // the default from then on.
        let mut odd: Preferences =
            toml::from_str("graph_order = \"sideways\"\ngraph_date = \"\"\n").unwrap();
        assert_eq!(odd.order(), bitbraid_git::Order::Date);
        assert!(!odd.commit_dates());
        odd.sanitize();
        assert_eq!(
            (odd.graph_order.as_str(), odd.graph_date.as_str()),
            ("date", "author")
        );
        for (word, _) in GRAPH_ORDERS.iter().chain(GRAPH_DATES) {
            assert!(!word.is_empty());
        }
    }

    #[test]
    fn how_diffs_are_shown_is_kept_and_odd_words_are_the_defaults() {
        // A file that says nothing about diffs.
        let old: Preferences = toml::from_str("zoom = 1.0").unwrap();
        assert_eq!(old.diff, DiffPrefs::default());
        assert_eq!(
            (old.diff.layout.as_str(), old.diff.whitespace.as_str()),
            ("hunks", "exact")
        );
        assert!(old.diff.words && !old.diff.wrap);
        // What was chosen comes back.
        let chosen = Preferences {
            diff: DiffPrefs {
                layout: "two_columns".to_owned(),
                whitespace: "ignore_all".to_owned(),
                wrap: true,
                words: false,
            },
            ..Default::default()
        };
        let text = toml::to_string_pretty(&chosen).unwrap();
        let back: Preferences = toml::from_str(&text).unwrap();
        assert_eq!(back.diff, chosen.diff);
        // Part of it, and words nobody knows.
        let mut odd: Preferences =
            toml::from_str("[diff]\nlayout = \"diagonal\"\nwhitespace = \"\"\nwrap = true\n")
                .unwrap();
        odd.sanitize();
        assert_eq!(
            odd.diff,
            DiffPrefs {
                wrap: true,
                ..Default::default()
            }
        );
        for layout in DIFF_LAYOUTS {
            for whitespace in DIFF_WHITESPACE {
                let mut known = DiffPrefs {
                    layout: layout.to_owned(),
                    whitespace: whitespace.to_owned(),
                    ..Default::default()
                };
                let before = known.clone();
                known.sanitize();
                assert_eq!(known, before);
            }
        }
    }

    #[test]
    fn a_tab_name_is_kept_trimmed_and_taken_away_when_empty() {
        let mut prefs = Prefs::load_from(None);
        let path = Path::new("/work/project");
        prefs.set_alias(path, "  Client\twork\n ");
        assert_eq!(prefs.state.aliases["/work/project"], "Clientwork");
        prefs.set_alias(path, &"x".repeat(500));
        assert_eq!(prefs.state.aliases["/work/project"].len(), 60);
        prefs.set_alias(path, "   ");
        assert!(prefs.state.aliases.is_empty());
        // It is in the state file, and a file without it reads.
        prefs.set_alias(path, "Named");
        let text = toml::to_string(&prefs.state).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(back.aliases["/work/project"], "Named");
        let old: State = toml::from_str("active = 1").unwrap();
        assert!(old.aliases.is_empty());
    }

    #[test]
    fn a_tab_keeps_its_own_panels_scroll_and_file() {
        let mut prefs = Prefs::load_from(None);
        let (one, other) = (Path::new("/work/one"), Path::new("/work/other"));
        // Nothing chosen: the preference, for every tab and for none.
        assert!(prefs.left_panel_shown(Some(one)) && prefs.right_panel_shown(None));
        prefs.set_panel_shown(Some(one), true, false);
        prefs.set_panel_shown(Some(one), false, false);
        assert!(!prefs.left_panel_shown(Some(one)) && !prefs.right_panel_shown(Some(one)));
        assert!(prefs.left_panel_shown(Some(other)) && prefs.right_panel_shown(Some(other)));
        // The preference is what tabs without a choice of their own follow.
        prefs.set_panel_shown(None, true, false);
        assert!(!prefs.left_panel_shown(Some(other)) && !prefs.left_panel_shown(None));
        prefs.set_panel_shown(Some(one), true, true);
        assert!(prefs.left_panel_shown(Some(one)));

        prefs.update_repo_state(one, |state| {
            state.top = Some("abc".to_owned());
            state.top_into = Some(12.5);
            state.open_file = Some(OpenFile {
                path: "src/a b.rs".to_owned(),
                rev: None,
            });
            state.closed_sections = Some(vec!["tags".to_owned()]);
        });
        let text = toml::to_string(&prefs.state).unwrap();
        let back: State = toml::from_str(&text).unwrap();
        assert_eq!(back.repos["/work/one"], prefs.repo_state(one));
        assert_eq!(back.repos["/work/one"].left_panel, Some(true));
        // A state file without any of this reads as none of it.
        let old: State = toml::from_str("[repos.\"/work/one\"]\nleft = 240.0\n").unwrap();
        let state = &old.repos["/work/one"];
        assert_eq!((state.top.clone(), state.open_file.clone()), (None, None));
        assert_eq!(state.top_into, None);
        assert_eq!(
            (state.closed_sections.clone(), state.left_panel),
            (None, None)
        );
        // A worktree that takes the main one's state over starts at its own
        // place.
        assert!(prefs.inherit_repo_state(Path::new("/work/linked"), one));
        let linked = prefs.repo_state(Path::new("/work/linked"));
        assert_eq!(
            (linked.top, linked.top_into, linked.open_file),
            (None, None, None)
        );
        assert_eq!(linked.closed_sections, Some(vec!["tags".to_owned()]));
    }

    #[test]
    fn each_profile_keeps_its_own_tabs() {
        let mut state = State::default();
        let paths = |names: &[&str]| -> Vec<PathBuf> {
            names
                .iter()
                .map(|name| PathBuf::from(format!("/work/{name}")))
                .collect()
        };
        // The first switch: nothing was open under "work" before, so the
        // tabs stay; what was open is kept under no profile.
        assert_eq!(state.switch_profile("work", paths(&["a", "b"]), 1), None);
        assert_eq!(state.profile.as_deref(), Some("work"));
        assert_eq!(state.profile_tabs[""].open, paths(&["a", "b"]));
        // Under "work" other tabs are opened, then "home" is switched to.
        assert_eq!(state.switch_profile("home", paths(&["client"]), 0), None);
        // Back to "work": its tabs come back.
        let back = state
            .switch_profile("work", paths(&["garden", "recipes"]), 1)
            .unwrap();
        assert_eq!(back.open, paths(&["client"]));
        assert_eq!(back.active, 0);
        assert_eq!(
            state.profile_tabs["home"].open,
            paths(&["garden", "recipes"])
        );
        assert_eq!(state.profile_tabs["home"].active, 1);
        // Switching to where one is changes nothing.
        let before = state.clone();
        assert_eq!(state.switch_profile("work", paths(&["zzz"]), 0), None);
        assert_eq!(state, before);
        // It is all in the state file, and a file without it reads.
        let text = toml::to_string(&state).unwrap();
        let read: State = toml::from_str(&text).unwrap();
        assert_eq!(read, state);
        let old: State = toml::from_str("active = 1").unwrap();
        assert!(old.profile.is_none() && old.profile_tabs.is_empty());
        // Sets of profiles long gone do not pile up for ever.
        for n in 0..200 {
            state.switch_profile(&format!("p{n}"), Vec::new(), 0);
        }
        assert!(
            state.profile_tabs.len() <= 65,
            "{}",
            state.profile_tabs.len()
        );
        assert!(state.profile_tabs.contains_key(""));
    }

    /// A profile that is given another name keeps its tabs, and is still
    /// the one switched to when it was.
    #[test]
    fn a_renamed_profile_keeps_its_tabs() {
        let mut state = State::default();
        let paths = |names: &[&str]| -> Vec<PathBuf> {
            names
                .iter()
                .map(|name| PathBuf::from(format!("/work/{name}")))
                .collect()
        };
        // Two tabs under Alternate, one under Default, which is current.
        state.switch_profile("Default", Vec::new(), 0);
        state.switch_profile("Alternate", paths(&["patches"]), 0);
        state.switch_profile("Default", paths(&["patches", "tree0"]), 1);
        // The one that is not current is renamed.
        state.rename_profile("Alternate", "QA alternate");
        assert!(!state.profile_tabs.contains_key("Alternate"));
        let back = state
            .switch_profile("QA alternate", paths(&["patches"]), 0)
            .unwrap();
        assert_eq!(back.open, paths(&["patches", "tree0"]));
        assert_eq!(back.active, 1);
        // The current one is renamed: it is still the current one, and
        // what is open is put away under its new name.
        state.rename_profile("QA alternate", "Second");
        assert_eq!(state.profile.as_deref(), Some("Second"));
        let default = state.switch_profile("Default", paths(&["x"]), 0).unwrap();
        assert_eq!(default.open, paths(&["patches"]));
        assert_eq!(state.profile_tabs["Second"].open, paths(&["x"]));
        // The same name changes nothing; a name nothing was kept under
        // has nothing to take along.
        let before = state.clone();
        state.rename_profile("Default", "Default");
        state.rename_profile("Nobody", "Somebody");
        assert_eq!(state, before);
    }

    #[test]
    fn the_lists_of_repositories_are_reordered_and_entries_taken_out() {
        let path = |name: &str| PathBuf::from(format!("/work/{name}"));
        let mut state = State {
            recent: vec![path("a"), path("b"), path("c"), path("d")],
            favourites: vec![path("x"), path("y")],
            ..State::default()
        };
        // Dropped on another entry, an entry takes its place.
        assert!(state.move_entry(RepoList::Recent, &path("d"), Some(&path("b"))));
        assert_eq!(state.recent, [path("a"), path("d"), path("b"), path("c")]);
        assert!(state.move_entry(RepoList::Recent, &path("a"), Some(&path("c"))));
        assert_eq!(state.recent, [path("d"), path("b"), path("c"), path("a")]);
        // Dropped past the last, it goes to the end.
        assert!(state.move_entry(RepoList::Recent, &path("d"), None));
        assert_eq!(state.recent, [path("b"), path("c"), path("a"), path("d")]);
        // Onto itself, or something that is not there: nothing.
        assert!(!state.move_entry(RepoList::Recent, &path("b"), Some(&path("b"))));
        assert!(!state.move_entry(RepoList::Recent, &path("zz"), Some(&path("b"))));
        assert!(!state.move_entry(RepoList::Favourites, &path("b"), None));
        // One place at a time, and not past an edge.
        assert!(state.nudge_entry(RepoList::Favourites, &path("y"), false));
        assert_eq!(state.favourites, [path("y"), path("x")]);
        assert!(!state.nudge_entry(RepoList::Favourites, &path("y"), false));
        assert!(!state.nudge_entry(RepoList::Favourites, &path("x"), true));
        assert!(state.nudge_entry(RepoList::Favourites, &path("y"), true));
        assert!(!state.nudge_entry(RepoList::Favourites, &path("nope"), true));
        // Taken out of one list, it stays in the other.
        state.favourites.push(path("b"));
        assert!(state.remove_entry(RepoList::Recent, &path("b")));
        assert!(!state.remove_entry(RepoList::Recent, &path("b")));
        assert_eq!(state.recent, [path("c"), path("a"), path("d")]);
        assert!(state.favourites.contains(&path("b")));
    }

    #[test]
    fn a_repository_that_moved_keeps_what_was_remembered_about_it() {
        let path = |name: &str| PathBuf::from(format!("/work/{name}"));
        let mut prefs = Prefs::load_from(None);
        prefs.state.recent = vec![path("a"), path("old"), path("new")];
        prefs.state.favourites = vec![path("old")];
        prefs.state.open = vec![path("old"), path("a")];
        prefs.update_repo_state(&path("old"), |state| state.left = Some(300.0));
        prefs.set_alias(&path("old"), "Client");
        prefs.state.profile_tabs.insert(
            "Work".to_owned(),
            TabSet {
                open: vec![path("old")],
                active: 0,
            },
        );
        assert!(prefs.state.relocate(&path("old"), &path("new")));
        // Where the new place was already listed, there is one entry.
        assert_eq!(prefs.state.recent, [path("a"), path("new")]);
        assert_eq!(prefs.state.favourites, [path("new")]);
        assert_eq!(prefs.state.open, [path("new"), path("a")]);
        assert_eq!(prefs.state.profile_tabs["Work"].open, [path("new")]);
        assert_eq!(prefs.repo_state(&path("new")).left, Some(300.0));
        assert_eq!(prefs.repo_state(&path("old")), RepoState::default());
        assert_eq!(prefs.state.aliases["/work/new"], "Client");
        assert!(!prefs.state.aliases.contains_key("/work/old"));
        // Nothing knows the old place any more.
        assert!(!prefs.state.relocate(&path("old"), &path("elsewhere")));
        assert!(!prefs.state.relocate(&path("new"), &path("new")));
        // What the new place had remembered already is kept.
        prefs.update_repo_state(&path("b"), |state| state.left = Some(111.0));
        prefs.update_repo_state(&path("c"), |state| state.left = Some(222.0));
        assert!(prefs.state.relocate(&path("b"), &path("c")));
        assert_eq!(prefs.repo_state(&path("c")).left, Some(222.0));
    }

    #[test]
    fn pinned_branches_are_a_list() {
        let new: State =
            toml::from_str("[repos.\"/r\"]\npinned = [\"refs/heads/main\", \"refs/heads/next\"]\n")
                .unwrap();
        assert_eq!(
            new.repos["/r"].pinned,
            ["refs/heads/main", "refs/heads/next"]
        );
        let none: State = toml::from_str("[repos.\"/r\"]\nhidden = []\n").unwrap();
        assert!(none.repos["/r"].pinned.is_empty());
        // Written and read back.
        let text = toml::to_string(&new).unwrap();
        assert_eq!(toml::from_str::<State>(&text).unwrap(), new);

        // Pinned behind what is pinned, unpinned by the same entry, and
        // no more than there is room for.
        let mut state = RepoState::default();
        assert!(state.toggle_pinned("refs/heads/a"));
        assert!(state.toggle_pinned("refs/heads/b"));
        assert_eq!(state.pinned, ["refs/heads/a", "refs/heads/b"]);
        assert!(state.toggle_pinned("refs/heads/a"));
        assert_eq!(state.pinned, ["refs/heads/b"]);
        for n in 0..20 {
            state.toggle_pinned(&format!("refs/heads/more-{n}"));
        }
        assert_eq!(state.pinned.len(), super::MAX_PINNED);
        assert!(!state.toggle_pinned("refs/heads/one-too-many"));
        assert!(
            state.toggle_pinned("refs/heads/b"),
            "unpinning always works"
        );
        // A file with more than fit is cut when it is next changed.
        let mut state = RepoState {
            pinned: (0..40).map(|n| format!("refs/heads/{n}")).collect(),
            ..RepoState::default()
        };
        assert!(!state.toggle_pinned("refs/heads/x"));
        assert_eq!(state.pinned.len(), super::MAX_PINNED);
    }

    #[test]
    fn state_tolerates_missing_fields() {
        let state: State = toml::from_str("active = 2").unwrap();
        assert_eq!(state.active, 2);
        assert!(state.open.is_empty());
    }

    #[test]
    fn a_repository_without_state_takes_over_another_ones_once() {
        let mut prefs = Prefs::load_from(None);
        let (main, linked) = (Path::new("/work/main"), Path::new("/work/linked"));
        // Nothing to take over yet.
        assert!(!prefs.inherit_repo_state(linked, main));
        prefs.update_repo_state(main, |state| {
            state.hidden = vec!["refs/heads/old".to_owned()];
            state.left = Some(240.0);
            state.selected = Some("abc".to_owned());
        });
        assert!(prefs.inherit_repo_state(linked, main));
        let taken = prefs.repo_state(linked);
        assert_eq!(taken.hidden, ["refs/heads/old"]);
        assert_eq!(taken.left, Some(240.0));
        assert_eq!(taken.selected, None, "the selection is its own");
        // It has its own now, and keeps it.
        prefs.update_repo_state(linked, |state| state.hidden.clear());
        assert!(!prefs.inherit_repo_state(linked, main));
        assert!(prefs.repo_state(linked).hidden.is_empty());
    }

    #[test]
    fn a_theme_is_a_word_in_the_file() {
        for (word, choice) in [
            ("dark", ThemeChoice::Dark),
            ("light", ThemeChoice::Light),
            ("contrast", ThemeChoice::Contrast),
            ("ember", ThemeChoice::Ember),
            ("Harbour", ThemeChoice::User("Harbour".to_owned())),
            // The words are ours only as written.
            ("Ember", ThemeChoice::User("Ember".to_owned())),
        ] {
            let prefs: Preferences = toml::from_str(&format!("theme = \"{word}\"")).unwrap();
            assert_eq!(prefs.theme, choice);
            let text = toml::to_string(&prefs).unwrap();
            assert!(text.contains(&format!("theme = \"{word}\"")), "{text}");
        }
        // Every one of ours has a word of its own.
        for ours in ThemeChoice::BUILT_IN {
            assert_eq!(ThemeChoice::from_key(ours.key()), ours);
        }
        // No name at all, or an absurd one, is the default.
        let mut prefs: Preferences = toml::from_str("theme = \"  \"").unwrap();
        prefs.sanitize();
        assert_eq!(prefs.theme, ThemeChoice::Dark);
        assert!(toml::from_str::<Preferences>("theme = 3").is_err());
    }
}
