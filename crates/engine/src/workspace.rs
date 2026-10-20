//! Workspaces: named groups of repositories, kept in
//! `workspaces.toml` in the application's configuration directory. This
//! module is what a workspace is, how it is checked, and how the file is
//! read and written; the commands are in `workspace_cmds.rs`.
//!
//! A workspace never changes a repository by being made, edited or
//! deleted: it is a list of folders with a name, a colour, an icon, a
//! description, the profile its repositories are opened with and the
//! layout their tabs start from. A repository that is not on this machine
//! (yet) is an entry with the address to clone it from.
//!
//! The file is someone's text that may have been edited by hand or come
//! from elsewhere: it is read with bounds on its size, on the number of
//! workspaces and repositories and on the length of every text, and a
//! file that cannot be read whole is a refusal that says why and is never
//! written over. Keys it has that this version does not know are passed
//! over; a file written by a newer version (a higher `version`) is read
//! and not written over, so nothing in it is lost.
//!
//! The shapes here are what the window (the page of repositories) and
//! the inbox read: [`Workspace`], [`WorkspaceRepository`], [`Layout`], and
//! for the commands that work on many repositories at once,
//! [`BulkOutcome`] and [`RepositoryOutcome`].
//!
//! Sources: the TOML specification (toml.io) for the file.

use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::Read as _,
    path::{Component, Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use crate::command::CommandError;

/// The file the workspaces are kept in, in the application's
/// configuration directory.
pub const FILE: &str = "workspaces.toml";
/// The version of the file this build writes. A file with a higher one is
/// read and not written over.
pub const VERSION: u32 = 1;
/// As many workspaces as are kept.
pub const MAX_WORKSPACES: usize = 256;
/// As many repositories as one workspace holds.
pub const MAX_REPOSITORIES: usize = 1000;
/// A file larger than this is not one that was written here, and is not
/// read.
pub const MAX_FILE: u64 = 4 * 1024 * 1024;
/// The longest name of a workspace, in characters.
pub const MAX_NAME: usize = 64;
/// The longest description, in characters.
pub const MAX_DESCRIPTION: usize = 2000;
/// The longest icon name, in characters.
pub const MAX_ICON: usize = 32;
/// The longest path, in bytes.
pub const MAX_PATH: usize = 4096;
/// The longest remote address, in bytes.
pub const MAX_URL: usize = 2048;
/// The longest id, in characters.
pub const MAX_ID: usize = 48;
/// As many names as one list of the layout holds.
pub const MAX_SECTIONS: usize = 64;
/// The widest a panel is kept, in pixels.
pub const MAX_WIDTH: f32 = 100_000.0;

/// A named group of repositories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Workspace {
    /// What it is known by in commands: made from its name when it is
    /// made, and never changed after, also when it is renamed.
    pub id: String,
    /// What it is called. No two have the same name, in whatever case it
    /// is written.
    pub name: String,
    /// Its colour, as `#rrggbb`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub colour: Option<String>,
    /// The name of its icon: lowercase letters, digits and dashes. Which
    /// names there are is the window's business.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// A few words about it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The profile (`profiles.toml`) its repositories are opened with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// A folder it keeps in step with: every repository found in it is
    /// one of the workspace's (`workspace.sync_folder`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub follows_folder: Option<String>,
    /// The layout a tab of one of its repositories starts from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Layout>,
    /// Its repositories, in the order they were added.
    #[serde(default)]
    pub repositories: Vec<WorkspaceRepository>,
}

/// One repository of a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkspaceRepository {
    /// Its folder, by its full path. A repository that is not there (not
    /// cloned yet, or moved) keeps the path it is to have.
    pub path: String,
    /// Where it can be cloned from: its `origin`, or its only remote, as
    /// it was when it was added; or the address it came with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_url: Option<String>,
}

impl WorkspaceRepository {
    /// The last part of its path: what it is called in a list.
    pub fn name(&self) -> String {
        Path::new(&self.path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.clone())
    }

    /// Whether its folder is there.
    pub fn is_present(&self) -> bool {
        Path::new(&self.path).is_dir()
    }
}

/// What a tab of a repository looks like when it is opened: the panels
/// and the sections of the references panel, as the window keeps them
/// for each repository. Everything that is not said is as the window
/// starts out.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Layout {
    /// The width of the references panel, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_width: Option<f32>,
    /// The width of the details panel, in pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_width: Option<f32>,
    /// Whether the references panel is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left_panel: Option<bool>,
    /// Whether the details panel is shown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right_panel: Option<bool>,
    /// The sections of the references panel that are closed; none while
    /// they are as they start out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub closed_sections: Option<Vec<String>>,
    /// The sections that are not shown.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hidden_sections: Vec<String>,
    /// The sections in the order they are shown; empty while they are in
    /// the order they start out in.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub section_order: Vec<String>,
    /// How the open sections share the height of the panel: a weight for
    /// each.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub section_sizes: BTreeMap<String, f32>,
    /// The section that has the room of all the others, if one has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximised_section: Option<String>,
}

impl Layout {
    /// Whether it says nothing: then there is no layout to keep.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

/// Where one repository is in a command that works on many at once.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryState {
    /// Waiting for its turn.
    Queued,
    /// Its command is running.
    Running,
    /// Its command ended well.
    Done,
    /// Its command ended badly: the error says why.
    Failed,
    /// Its command was not run: the error says why (stopped before its
    /// turn, nothing to clone it from, a folder in the way).
    Skipped,
}

impl RepositoryState {
    /// The state as one word, as the progress lines have it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Running => "running",
            Self::Done => "done",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }

    /// The state a word names.
    pub fn parse(word: &str) -> Option<Self> {
        Some(match word {
            "queued" => Self::Queued,
            "running" => Self::Running,
            "done" => Self::Done,
            "failed" => Self::Failed,
            "skipped" => Self::Skipped,
            _ => return None,
        })
    }

    /// Whether it is over for this repository.
    pub fn is_final(self) -> bool {
        matches!(self, Self::Done | Self::Failed | Self::Skipped)
    }
}

/// What became of one repository in a command that works on many.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RepositoryOutcome {
    /// The repository, by the path the workspace has for it.
    pub path: String,
    /// Where it is.
    pub state: RepositoryState,
    /// The last thing git said it was doing, as `Receiving objects`.
    #[serde(default)]
    pub progress: Option<String>,
    /// How far along that is, when git says.
    #[serde(default)]
    pub percent: Option<u8>,
    /// Why it failed or was skipped.
    #[serde(default)]
    pub error: Option<String>,
    /// What its command answered, when it ended well.
    #[serde(default)]
    pub output: Option<serde_json::Value>,
}

impl RepositoryOutcome {
    /// A repository waiting for its turn.
    pub fn queued(path: &str) -> Self {
        Self {
            path: path.to_owned(),
            state: RepositoryState::Queued,
            progress: None,
            percent: None,
            error: None,
            output: None,
        }
    }

    /// This as a progress report: what a command that works on many
    /// repositories hands its progress sink. Its stage is three fields
    /// with a tab between them: the path, the state and the line of
    /// progress or the error. A path has no tab (`workspace` refuses
    /// control characters in paths), and the line has its control
    /// characters made blanks, so it reads back with
    /// [`RepositoryOutcome::from_progress`].
    pub fn to_progress(&self) -> bitbraid_git::Progress {
        let said = self
            .error
            .as_deref()
            .or(self.progress.as_deref())
            .unwrap_or("");
        let said: String = said
            .chars()
            .take(500)
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        bitbraid_git::Progress {
            stage: format!("{}\t{}\t{}", self.path, self.state.as_str(), said),
            percent: self.percent,
        }
    }

    /// A progress report of a command that works on many repositories, as
    /// [`RepositoryOutcome::to_progress`] made it; none for any other.
    /// What went wrong and the line of progress come back in `error` or
    /// `progress` by the state.
    pub fn from_progress(progress: &bitbraid_git::Progress) -> Option<Self> {
        let mut parts = progress.stage.splitn(3, '\t');
        let path = parts.next()?;
        let state = RepositoryState::parse(parts.next()?)?;
        let said = parts.next()?;
        let said = Some(said.to_owned()).filter(|said| !said.is_empty());
        let failed = matches!(state, RepositoryState::Failed | RepositoryState::Skipped);
        Some(Self {
            path: path.to_owned(),
            state,
            progress: if failed { None } else { said.clone() },
            percent: progress.percent,
            error: if failed { said } else { None },
            output: None,
        })
    }
}

/// What a command that works on many repositories did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BulkOutcome {
    /// The workspace, by its id.
    pub workspace: String,
    /// The command each repository was given, as `remote.fetch`.
    pub command: String,
    /// What became of each repository, in the workspace's order.
    pub repositories: Vec<RepositoryOutcome>,
    /// How many ended well, failed, and were skipped.
    pub done: usize,
    pub failed: usize,
    pub skipped: usize,
    /// Whether it was stopped before every repository had its turn.
    pub stopped: bool,
    /// As many repositories as were worked on at once.
    pub parallel: usize,
    /// How long it took, in milliseconds.
    pub elapsed_ms: u64,
}

/// The file as it is read. Keys that are not known are passed over.
#[derive(Debug, Default, Deserialize)]
struct Stored {
    #[serde(default)]
    version: Option<u32>,
    #[serde(default)]
    workspaces: Vec<Workspace>,
}

/// The file as it is written.
#[derive(Serialize)]
struct ToStore<'a> {
    version: u32,
    workspaces: &'a [Workspace],
}

/// What was read: the workspaces, and whether the file may be written
/// over (not when a newer version wrote it).
#[derive(Debug, Default)]
pub struct Loaded {
    /// The workspaces, in the file's order.
    pub workspaces: Vec<Workspace>,
    /// Why it is not to be written over, when it is not.
    pub read_only: Option<String>,
}

pub(crate) fn refused(detail: impl Into<String>) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what: say!("workspace-refused"),
        detail: detail.into(),
    })
}

/// The beginning of a text, for an error message.
pub(crate) fn cut(text: &str) -> String {
    const SHOWN: usize = 200;
    match text.char_indices().nth(SHOWN) {
        Some((end, _)) => format!("{}…", text.get(..end).unwrap_or("")),
        None => text.to_owned(),
    }
}

/// A short text as it is kept: without blanks around it, not empty, not
/// longer than `most` characters, and without control characters.
pub fn short_text(text: &str, most: usize) -> Result<String, String> {
    let kept = text.trim();
    if kept.is_empty() {
        return Err(say!("workspace-empty").to_owned());
    }
    if kept.chars().count() > most {
        return Err(say!("workspace-longer-than-characters", most));
    }
    if kept.chars().any(char::is_control) {
        return Err(say!("workspace-control-characters").to_owned());
    }
    Ok(kept.to_owned())
}

/// A name for comparing: two workspaces are one when this is the same.
pub fn folded(name: &str) -> String {
    name.to_lowercase()
}

/// A colour as it is kept: `#rrggbb`, in lower case.
pub fn colour(text: &str) -> Result<String, String> {
    let kept = text.trim().to_ascii_lowercase();
    let digits = kept.strip_prefix('#').unwrap_or("");
    if digits.len() == 6 && digits.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(kept)
    } else {
        Err(say!("workspace-colour-form").to_owned())
    }
}

/// An icon's name as it is kept: lowercase letters, digits and dashes.
pub fn icon(text: &str) -> Result<String, String> {
    let kept = text.trim();
    if kept.is_empty() || kept.chars().count() > MAX_ICON {
        return Err(say!("workspace-icon-length", most = MAX_ICON));
    }
    if !kept
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(say!("workspace-icon-characters").to_owned());
    }
    Ok(kept.to_owned())
}

/// A description as it is kept: without blanks around it, not longer than
/// [`MAX_DESCRIPTION`], and without control characters other than line
/// breaks and tabs. An empty one is none.
pub fn description(text: &str) -> Result<Option<String>, String> {
    let kept = text.trim();
    if kept.is_empty() {
        return Ok(None);
    }
    if kept.chars().count() > MAX_DESCRIPTION {
        return Err(say!(
            "workspace-longer-than-characters",
            most = MAX_DESCRIPTION
        ));
    }
    if kept
        .chars()
        .any(|c| c.is_control() && c != '\n' && c != '\t')
    {
        return Err(say!("workspace-control-characters").to_owned());
    }
    Ok(Some(kept.to_owned()))
}

/// A full path as it is kept: absolute, without control characters, not
/// longer than [`MAX_PATH`], without `..`, and without `.` parts or a
/// separator at its end.
pub fn full_path(text: &str) -> Result<String, String> {
    if text.is_empty() {
        return Err(say!("workspace-empty").to_owned());
    }
    if text.len() > MAX_PATH {
        return Err(say!("workspace-longer-than-bytes", most = MAX_PATH));
    }
    if text.chars().any(char::is_control) {
        return Err(say!("workspace-control-characters").to_owned());
    }
    let path = Path::new(text);
    if !path.is_absolute() || text.starts_with('-') {
        return Err(say!("workspace-not-full-path").to_owned());
    }
    let mut kept = PathBuf::new();
    for component in path.components() {
        match component {
            Component::ParentDir => return Err(say!("workspace-goes-up").to_owned()),
            Component::CurDir => {}
            other => kept.push(other),
        }
    }
    kept.to_str()
        .map(str::to_owned)
        .ok_or_else(|| say!("workspace-not-text").to_owned())
}

/// A remote address as it is kept: without blanks around it, not longer
/// than [`MAX_URL`], without control characters, and not something git
/// would take for an option.
pub fn remote_url(text: &str) -> Result<String, String> {
    let kept = text.trim();
    if kept.is_empty() {
        return Err(say!("workspace-empty").to_owned());
    }
    if kept.len() > MAX_URL {
        return Err(say!("workspace-longer-than-bytes", most = MAX_URL));
    }
    if kept.chars().any(char::is_control) {
        return Err(say!("workspace-control-characters").to_owned());
    }
    if kept.starts_with('-') {
        return Err(say!("workspace-starts-with-dash").to_owned());
    }
    Ok(kept.to_owned())
}

/// A layout as it is kept: widths that are numbers within bounds, names
/// of sections that are short words, and no list longer than
/// [`MAX_SECTIONS`].
pub fn layout(layout: Layout) -> Result<Layout, String> {
    let width = |width: Option<f32>, what: &str| match width {
        Some(width) if !width.is_finite() || !(0.0..=MAX_WIDTH).contains(&width) => {
            Err(say!("workspace-width-bounds", what, most = MAX_WIDTH))
        }
        other => Ok(other),
    };
    let section =
        |name: &str| short_text(name, MAX_NAME).map_err(|why| say!("workspace-section-name", why));
    let names = |list: Vec<String>, what: &str| {
        if list.len() > MAX_SECTIONS {
            return Err(say!("workspace-too-many-names", what, most = MAX_SECTIONS));
        }
        list.iter()
            .map(|name| section(name))
            .collect::<Result<Vec<_>, _>>()
    };
    if layout.section_sizes.len() > MAX_SECTIONS {
        return Err(say!("workspace-too-many-sizes", most = MAX_SECTIONS));
    }
    let mut sizes = BTreeMap::new();
    for (name, weight) in layout.section_sizes {
        if !weight.is_finite() || !(0.0..=MAX_WIDTH).contains(&weight) {
            return Err(say!(
                "workspace-not-a-weight",
                name = format!("{:?}", cut(&name))
            ));
        }
        sizes.insert(section(&name)?, weight);
    }
    Ok(Layout {
        left_width: width(layout.left_width, say!("workspace-left-panel-width"))?,
        right_width: width(layout.right_width, say!("workspace-right-panel-width"))?,
        left_panel: layout.left_panel,
        right_panel: layout.right_panel,
        closed_sections: layout
            .closed_sections
            .map(|list| names(list, "closed_sections"))
            .transpose()?,
        hidden_sections: names(layout.hidden_sections, "hidden_sections")?,
        section_order: names(layout.section_order, "section_order")?,
        section_sizes: sizes,
        maximised_section: layout
            .maximised_section
            .map(|name| section(&name))
            .transpose()?,
    })
}

/// An id as it is kept: lowercase letters, digits and dashes, not
/// starting or ending with a dash.
pub fn id(text: &str) -> Result<String, String> {
    let ok = !text.is_empty()
        && text.chars().count() <= MAX_ID
        && text
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !text.starts_with('-')
        && !text.ends_with('-');
    if ok {
        Ok(text.to_owned())
    } else {
        Err(say!("workspace-id-form", most = MAX_ID))
    }
}

/// A new id for a workspace called `name`: its letters and digits in lower
/// case with dashes between the words, and a number after it when another
/// has that id already.
pub fn new_id(name: &str, taken: &[Workspace]) -> String {
    let mut slug = String::new();
    for c in name.chars().flat_map(char::to_lowercase) {
        if c.is_ascii_lowercase() || c.is_ascii_digit() {
            slug.push(c);
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() >= 32 {
            break;
        }
    }
    let slug = slug.trim_matches('-');
    let slug = if slug.is_empty() { "workspace" } else { slug };
    let used = |candidate: &str| taken.iter().any(|workspace| workspace.id == candidate);
    if !used(slug) {
        return slug.to_owned();
    }
    let mut number = 2usize;
    loop {
        let candidate = format!("{slug}-{number}");
        if !used(&candidate) {
            return candidate;
        }
        number = number.saturating_add(1);
    }
}

/// A repository entry as it is kept, or why it cannot be.
pub fn repository(entry: WorkspaceRepository) -> Result<WorkspaceRepository, String> {
    let path = full_path(&entry.path).map_err(|why| say!("workspace-its-path", why))?;
    let remote_url = entry
        .remote_url
        .filter(|url| !url.trim().is_empty())
        .map(|url| remote_url(&url).map_err(|why| say!("workspace-its-remote-url", why)))
        .transpose()?;
    Ok(WorkspaceRepository { path, remote_url })
}

/// A workspace as it is kept: every field checked, blanks around texts
/// gone, an empty text is none, an empty layout is none, and a
/// repository is in it once.
pub fn tidy(workspace: Workspace) -> Result<Workspace, String> {
    let id = id(&workspace.id).map_err(|why| say!("workspace-its-id", why))?;
    let name =
        short_text(&workspace.name, MAX_NAME).map_err(|why| say!("workspace-its-name", why))?;
    let filled = |text: Option<String>| text.filter(|text| !text.trim().is_empty());
    let colour = filled(workspace.colour)
        .map(|text| colour(&text).map_err(|why| say!("workspace-its-colour", why)))
        .transpose()?;
    let icon = filled(workspace.icon)
        .map(|text| icon(&text).map_err(|why| say!("workspace-its-icon", why)))
        .transpose()?;
    let description = match workspace.description {
        Some(text) => description(&text).map_err(|why| say!("workspace-its-description", why))?,
        None => None,
    };
    let default_profile = filled(workspace.default_profile)
        .map(|text| {
            short_text(&text, crate::profile_cmds::MAX_NAME)
                .map_err(|why| say!("workspace-its-default-profile", why))
        })
        .transpose()?;
    let follows_folder = filled(workspace.follows_folder)
        .map(|text| full_path(&text).map_err(|why| say!("workspace-its-follows-folder", why)))
        .transpose()?;
    let layout = workspace
        .layout
        .map(|kept| layout(kept).map_err(|why| say!("workspace-its-layout", why)))
        .transpose()?
        .filter(|kept| !kept.is_empty());
    if workspace.repositories.len() > MAX_REPOSITORIES {
        return Err(say!(
            "workspace-too-many-repositories",
            most = MAX_REPOSITORIES
        ));
    }
    let mut seen = HashSet::new();
    let mut repositories = Vec::with_capacity(workspace.repositories.len());
    for (index, entry) in workspace.repositories.into_iter().enumerate() {
        let entry = repository(entry).map_err(|why| {
            say!(
                "workspace-its-repository",
                number = index.saturating_add(1),
                why
            )
        })?;
        if seen.insert(entry.path.clone()) {
            repositories.push(entry);
        }
    }
    Ok(Workspace {
        id,
        name,
        colour,
        icon,
        description,
        default_profile,
        follows_folder,
        layout,
        repositories,
    })
}

/// One reader or writer of the file at a time in this process, so that
/// two commands that change it one after the other both have their way.
static FILE_LOCK: Mutex<()> = Mutex::new(());

/// Hold the file for a change: read it, let `change` make what is to be
/// written and answered, write that. Nothing is written when `change`
/// refuses or when the file is not to be written over.
pub(crate) fn changing<T>(
    dir: &Path,
    change: impl FnOnce(&mut Vec<Workspace>) -> Result<T, CommandError>,
) -> Result<T, CommandError> {
    // A command that panicked while it held the lock left the file whole
    // (it is written beside and moved), so the lock is taken all the same.
    let _held = FILE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let loaded = load(dir)?;
    if let Some(why) = loaded.read_only {
        return Err(refused(why));
    }
    let mut workspaces = loaded.workspaces;
    let answer = change(&mut workspaces)?;
    store(dir, &workspaces)?;
    Ok(answer)
}

/// A profile was given another name: every workspace that names it as its
/// default profile, in whatever case (profiles are told apart as
/// workspaces are), names the new one. How many did. Nothing is written
/// when none names it; a file that is not to be written over is refused.
pub fn rename_profile(dir: &Path, from: &str, to: &str) -> Result<usize, CommandError> {
    retarget_profile(dir, from, Some(to))
}

/// A profile was deleted: every workspace that names it as its default
/// profile, in whatever case, names none from then on. How many did; as
/// for a rename, nothing is written when none names it.
pub fn forget_profile(dir: &Path, name: &str) -> Result<usize, CommandError> {
    retarget_profile(dir, name, None)
}

/// The workspaces that name the profile `from` name `to` instead, or none.
fn retarget_profile(dir: &Path, from: &str, to: Option<&str>) -> Result<usize, CommandError> {
    let key = folded(from);
    let names = |workspace: &Workspace| {
        workspace
            .default_profile
            .as_deref()
            .is_some_and(|name| folded(name) == key && Some(name) != to)
    };
    if !load(dir)?.workspaces.iter().any(names) {
        return Ok(0);
    }
    changing(dir, |workspaces| {
        let mut changed = 0usize;
        for workspace in workspaces.iter_mut().filter(|workspace| names(workspace)) {
            workspace.default_profile = to.map(str::to_owned);
            changed = changed.saturating_add(1);
        }
        Ok(changed)
    })
}

/// Read the workspaces. No file is no workspaces. A file that cannot be
/// read as workspaces, whatever the reason, is a refusal that says so,
/// and that is what keeps it from being written over.
pub fn load(dir: &Path) -> Result<Loaded, CommandError> {
    let path = dir.join(FILE);
    let unusable = |why: String| {
        refused(say!(
            "workspace-file-unusable",
            path = path.display().to_string(),
            why
        ))
    };
    let meta = match fs::metadata(&path) {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Loaded::default()),
        Err(err) => {
            return Err(unusable(say!(
                "workspace-cannot-be-read",
                error = err.to_string()
            )));
        }
    };
    if !meta.is_file() {
        return Err(unusable(say!("workspace-not-a-file").to_owned()));
    }
    let too_large = || unusable(say!("workspace-larger-than", kib = MAX_FILE / 1024));
    if meta.len() > MAX_FILE {
        return Err(too_large());
    }
    // Not a byte more than the bound, whatever the file has grown to
    // since it was asked.
    let mut bytes = Vec::new();
    fs::File::open(&path)
        .and_then(|file| {
            file.take(MAX_FILE.saturating_add(1))
                .read_to_end(&mut bytes)
        })
        .map_err(|err| unusable(say!("workspace-cannot-be-read", error = err.to_string())))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_FILE {
        return Err(too_large());
    }
    let text =
        String::from_utf8(bytes).map_err(|_| unusable(say!("workspace-not-text").to_owned()))?;
    let stored = parse(&text).map_err(unusable)?;
    Ok(stored)
}

/// The workspace of an id, as it is kept now in the configuration
/// directory `dir`.
pub fn find(dir: &Path, id: &str) -> Result<Workspace, CommandError> {
    load(dir)?
        .workspaces
        .into_iter()
        .find(|workspace| workspace.id == id)
        .ok_or_else(|| refused(say!("workspace-no-such", id = format!("{id:?}"))))
}

/// The workspaces of the text of a file, or why they cannot be read.
pub fn parse(text: &str) -> Result<Loaded, String> {
    let stored: Stored = toml::from_str(text).map_err(|err| {
        let line = err
            .span()
            .and_then(|span| text.get(..span.start))
            .map(|before| before.matches('\n').count().saturating_add(1));
        let said = cut(err.message());
        match line {
            Some(line) => say!("workspace-not-understood-at", line, said),
            None => say!("workspace-not-understood", said),
        }
    })?;
    if stored.workspaces.len() > MAX_WORKSPACES {
        return Err(say!(
            "workspace-file-too-many",
            count = stored.workspaces.len(),
            most = MAX_WORKSPACES
        ));
    }
    let read_only = match stored.version {
        Some(version) if version > VERSION => Some(say!("workspace-file-newer", version)),
        _ => None,
    };
    let mut ids = HashSet::new();
    let mut names = HashSet::new();
    let mut workspaces = Vec::with_capacity(stored.workspaces.len());
    for (index, workspace) in stored.workspaces.into_iter().enumerate() {
        let number = index.saturating_add(1);
        let workspace = tidy(workspace)
            .map_err(|why| say!("workspace-file-unusable-workspace", number, why))?;
        if !ids.insert(workspace.id.clone()) {
            return Err(say!("workspace-file-same-id", number));
        }
        if !names.insert(folded(&workspace.name)) {
            return Err(say!("workspace-file-same-name", number));
        }
        workspaces.push(workspace);
    }
    Ok(Loaded {
        workspaces,
        read_only,
    })
}

/// The text of the file for these workspaces.
pub fn to_text(workspaces: &[Workspace]) -> Result<String, String> {
    toml::to_string_pretty(&ToStore {
        version: VERSION,
        workspaces,
    })
    .map_err(|err| err.to_string())
}

/// Write `text` to `path`: beside it first, then in its place, so that
/// the file is whole or is the one before.
pub(crate) fn write_whole(path: &Path, text: &str) -> Result<(), String> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let beside = path.with_file_name(format!(
        ".{name}.{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&beside, text)
        .map_err(|err| say!("workspace-cannot-be-written", error = err.to_string()))?;
    fs::rename(&beside, path).map_err(|err| {
        let _ = fs::remove_file(&beside);
        say!("workspace-cannot-be-placed", error = err.to_string())
    })
}

/// Write the workspaces.
fn store(dir: &Path, workspaces: &[Workspace]) -> Result<(), CommandError> {
    let text =
        to_text(workspaces).map_err(|why| refused(say!("workspace-not-written-down", why)))?;
    // What could not be read back is not written.
    if u64::try_from(text.len()).unwrap_or(u64::MAX) > MAX_FILE {
        return Err(refused(say!(
            "workspace-too-large-to-keep",
            kib = MAX_FILE / 1024
        )));
    }
    fs::create_dir_all(dir).map_err(|err| {
        refused(say!(
            "workspace-directory-not-made",
            error = err.to_string()
        ))
    })?;
    write_whole(&dir.join(FILE), &text).map_err(|why| refused(say!("workspace-file-failed", why)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A full path of this system for one written the Unix way: on
    /// Windows on drive C, with its separators.
    fn full(path: &str) -> String {
        if cfg!(windows) {
            format!("C:{}", path.replace('/', "\\"))
        } else {
            path.to_owned()
        }
    }

    fn workspace(id: &str, name: &str) -> Workspace {
        Workspace {
            id: id.to_owned(),
            name: name.to_owned(),
            colour: None,
            icon: None,
            description: None,
            default_profile: None,
            follows_folder: None,
            layout: None,
            repositories: Vec::new(),
        }
    }

    #[test]
    fn ids_are_made_from_names_and_never_twice() {
        assert_eq!(new_id("Web Services", &[]), "web-services");
        assert_eq!(new_id("  Ünï cödé 人 ", &[]), "n-c-d");
        assert_eq!(new_id("人人", &[]), "workspace");
        assert_eq!(new_id("--", &[]), "workspace");
        let taken = [workspace("web", "Web"), workspace("web-2", "Web two")];
        assert_eq!(new_id("WEB", &taken), "web-3");
        let long = new_id(&"a".repeat(500), &[]);
        assert!(long.len() <= 32 && id(&long).is_ok(), "{long}");
        for made in ["Web Services", "a-b", "x y z", "9"] {
            assert!(id(&new_id(made, &taken)).is_ok(), "{made}");
        }
        for bad in ["", "-a", "a-", "A", "a b", "a_b", &"a".repeat(MAX_ID + 1)] {
            assert!(id(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn paths_are_full_and_plain() {
        assert_eq!(full_path(&full("/a/b/")).unwrap(), full("/a/b"));
        assert_eq!(full_path(&full("/a/./b")).unwrap(), full("/a/b"));
        assert_eq!(full_path(&full("/a//b")).unwrap(), full("/a/b"));
        for bad in [
            "",
            "a/b",
            "./a",
            &full("/a/../b"),
            &full("/a\nb"),
            &full("/a\0b"),
            "-x",
            &full(&format!("/{}", "a".repeat(MAX_PATH))),
        ] {
            assert!(full_path(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn colours_icons_and_addresses_are_checked() {
        assert_eq!(colour(" #A0B1C2 ").unwrap(), "#a0b1c2");
        for bad in ["", "red", "#abc", "#abcdeg", "a0b1c2", "#a0b1c2ff"] {
            assert!(colour(bad).is_err(), "{bad:?}");
        }
        assert_eq!(icon("folder-git-2").unwrap(), "folder-git-2");
        for bad in ["", "Folder", "a b", "a/b", &"a".repeat(MAX_ICON + 1)] {
            assert!(icon(bad).is_err(), "{bad:?}");
        }
        assert_eq!(
            remote_url(" git@example.invalid:a/b.git ").unwrap(),
            "git@example.invalid:a/b.git"
        );
        for bad in [
            "",
            "  ",
            "--upload-pack=x",
            "a\nb",
            &"u".repeat(MAX_URL + 1),
        ] {
            assert!(remote_url(bad).is_err(), "{bad:?}");
        }
        assert_eq!(description("  ").unwrap(), None);
        assert_eq!(
            description(" two\nlines\t ").unwrap().as_deref(),
            Some("two\nlines")
        );
        assert!(description("a\u{1b}[2J").is_err());
    }

    #[test]
    fn a_layout_is_numbers_and_short_names() {
        let fine = Layout {
            left_width: Some(260.0),
            closed_sections: Some(vec!["tags".to_owned()]),
            section_sizes: [("local".to_owned(), 2.0)].into_iter().collect(),
            ..Layout::default()
        };
        assert_eq!(layout(fine.clone()).unwrap(), fine);
        for bad in [
            Layout {
                left_width: Some(f32::NAN),
                ..Layout::default()
            },
            Layout {
                right_width: Some(-1.0),
                ..Layout::default()
            },
            Layout {
                hidden_sections: vec!["a\nb".to_owned()],
                ..Layout::default()
            },
            Layout {
                section_order: vec!["x".to_owned(); MAX_SECTIONS + 1],
                ..Layout::default()
            },
            Layout {
                section_sizes: [("x".to_owned(), f32::INFINITY)].into_iter().collect(),
                ..Layout::default()
            },
        ] {
            assert!(layout(bad.clone()).is_err(), "{bad:?}");
        }
        assert!(Layout::default().is_empty());
    }

    #[test]
    fn a_workspace_is_tidied_before_it_is_kept() {
        let typed = Workspace {
            name: "  Web \t".to_owned(),
            colour: Some(" #ABCDEF".to_owned()),
            icon: Some("".to_owned()),
            description: Some("   ".to_owned()),
            default_profile: Some(" Work ".to_owned()),
            layout: Some(Layout::default()),
            repositories: vec![
                WorkspaceRepository {
                    path: full("/src/a/"),
                    remote_url: Some(" ".to_owned()),
                },
                WorkspaceRepository {
                    path: full("/src/a"),
                    remote_url: Some("/elsewhere".to_owned()),
                },
            ],
            ..workspace("web", "")
        };
        let kept = tidy(typed).unwrap();
        assert_eq!(kept.name, "Web");
        assert_eq!(kept.colour.as_deref(), Some("#abcdef"));
        assert_eq!(kept.icon, None);
        assert_eq!(kept.description, None);
        assert_eq!(kept.default_profile.as_deref(), Some("Work"));
        assert_eq!(kept.layout, None);
        // One path is one repository: the first is kept.
        assert_eq!(
            kept.repositories,
            [WorkspaceRepository {
                path: full("/src/a"),
                remote_url: None,
            }]
        );
        assert_eq!(tidy(kept.clone()).unwrap(), kept);
    }

    #[test]
    fn the_file_reads_back_what_was_written() {
        let all = [
            Workspace {
                colour: Some("#112233".to_owned()),
                icon: Some("star".to_owned()),
                description: Some("quoted \"text\" \\ and\nlines".to_owned()),
                default_profile: Some("Work".to_owned()),
                follows_folder: Some(full("/src")),
                layout: Some(Layout {
                    left_width: Some(300.5),
                    left_panel: Some(false),
                    closed_sections: Some(Vec::new()),
                    section_sizes: [("tags".to_owned(), 1.5)].into_iter().collect(),
                    ..Layout::default()
                }),
                repositories: vec![WorkspaceRepository {
                    path: full("/src/it's $HOME 人"),
                    remote_url: Some("https://example.invalid/a.git".to_owned()),
                }],
                ..workspace("web", "Web")
            },
            workspace("empty", "Empty"),
        ];
        let text = to_text(&all).unwrap();
        let back = parse(&text).unwrap();
        assert_eq!(back.workspaces, all);
        assert!(back.read_only.is_none());
        assert!(parse("").unwrap().workspaces.is_empty());
    }

    #[test]
    fn what_cannot_be_used_is_said_and_unknown_keys_are_passed_over() {
        let known = parse(&format!(
            "version = 1\nlater = {{ a = 1 }}\n[[workspaces]]\nid = \"a\"\nname = \"A\"\n\
             tint = \"red\"\n[[workspaces.repositories]]\npath = '{}'\nkind = \"x\"\n",
            full("/a")
        ))
        .unwrap();
        assert_eq!(known.workspaces.len(), 1);
        assert_eq!(known.workspaces[0].repositories[0].path, full("/a"));
        let newer = parse("version = 7\n").unwrap();
        assert!(newer.read_only.is_some());
        for text in [
            "workspaces = \"none\"\n",
            "[workspaces]\nid = \"a\"\n",
            "[[workspaces]]\nid = \"a\"\n",
            "[[workspaces]]\nid = \"A\"\nname = \"a\"\n",
            "[[workspaces]]\nid = \"a\"\nname = \"a\"\ncolour = \"red\"\n",
            "[[workspaces]]\nid = \"a\"\nname = \"a\"\n[[workspaces]]\nid = \"a\"\nname = \"b\"\n",
            "[[workspaces]]\nid = \"a\"\nname = \"a\"\n[[workspaces]]\nid = \"b\"\nname = \"A\"\n",
            "[[workspaces]]\nid = \"a\"\nname = \"a\"\n[[workspaces.repositories]]\npath = \"rel\"\n",
            "version = -1\n",
            "version = \"one\"\n",
        ] {
            assert!(parse(text).is_err(), "{text}");
        }
        let many: String = (0..=MAX_WORKSPACES)
            .map(|n| format!("[[workspaces]]\nid = \"w{n}\"\nname = \"W{n}\"\n"))
            .collect();
        assert!(parse(&many).unwrap_err().contains("no more than"));
    }

    #[test]
    fn a_progress_line_names_its_repository() {
        let mut outcome = RepositoryOutcome::queued("/src/a b");
        outcome.state = RepositoryState::Running;
        outcome.progress = Some("Receiving objects".to_owned());
        outcome.percent = Some(42);
        let progress = outcome.to_progress();
        assert_eq!(progress.stage, "/src/a b\trunning\tReceiving objects");
        assert_eq!(
            RepositoryOutcome::from_progress(&progress).unwrap(),
            outcome
        );
        let failed = RepositoryOutcome {
            state: RepositoryState::Failed,
            error: Some("fatal:\tno\nway".to_owned()),
            progress: None,
            percent: None,
            ..outcome
        };
        let back = RepositoryOutcome::from_progress(&failed.to_progress()).unwrap();
        assert_eq!(back.error.as_deref(), Some("fatal: no way"));
        assert_eq!(back.state, RepositoryState::Failed);
        for stage in ["Receiving objects", "", "/a\tnone\tx", "/a\trunning"] {
            let progress = bitbraid_git::Progress {
                stage: stage.to_owned(),
                percent: None,
            };
            assert!(
                RepositoryOutcome::from_progress(&progress).is_none(),
                "{stage}"
            );
        }
    }
}
