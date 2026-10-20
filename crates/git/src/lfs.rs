//! Git LFS: whether it is installed and set up in a repository, the
//! patterns it tracks, the LFS files of a commit, moving their content to
//! and from a remote, and the locks a remote's server keeps.
//!
//! All of it is `git lfs`, run like every other git command, so it is in
//! the activity log. Two things are read here without it, so that a
//! repository can be asked how it stands on every refresh, and also where
//! git-lfs is not installed: the patterns, from the attribute files, and
//! how many files are LFS files, from the index and the pointers in the
//! object database.
//!
//! What an attribute file says is not trusted: it is read up to a bound,
//! never through a link, and nothing is written through one either.
//!
//! Git-lfs writes its hooks without being asked to, where git looks for
//! hooks, whenever it tracks or untracks a pattern and whenever its filter
//! runs; a hook of the user's own it leaves alone. That is its doing and
//! is left to it. Only [`install`] is asked for hooks, and it keeps them
//! inside the repository.
//!
//! Moving content takes as long as the content is large. While it runs it
//! says how far it is (see [`report`]), and asked to stop it ends with
//! everything it started: git, git-lfs and the transfer programs of
//! git-lfs (see [`exe::run_followed`]). What had arrived whole by then
//! stays in the local store, as git-lfs leaves it.
//!
//! Sources: git-lfs(1), git-lfs-install(1), git-lfs-track(1),
//! git-lfs-untrack(1), git-lfs-ls-files(1), git-lfs-fetch(1),
//! git-lfs-pull(1), git-lfs-push(1), git-lfs-checkout(1),
//! git-lfs-locks(1), git-lfs-lock(1),
//! git-lfs-unlock(1), git-lfs-config(5); the Git LFS specification (what a
//! pointer file is) and its file locking API (what a lock has);
//! gitattributes(5) (the lines of an attribute file, quoting, macros,
//! where they are read from); gitglossary(7) (the `attr` pathspec magic);
//! git-ls-files(1); githooks(5); git-config(1) (`--get-regexp`,
//! `http.sslCAInfo`). What the pages leave open (what a command prints,
//! in which order it writes) was found by running git-lfs 3.8.0, and what
//! a real server answers (who may list, verify, lock and force, a
//! certificate that is not accepted) by running it against GitLab CE 19.4
//! and github.com.

use std::{
    collections::HashMap,
    ffi::OsString,
    io::Read as _,
    path::{Component, Path, PathBuf},
    sync::Mutex,
    time::{Duration, Instant},
};

use serde::Serialize;
use serde_json::Value;

use crate::{Error, Oid, Reporter, Repository, Result, branch, exe, remote};

mod report;

/// The longest pattern, path and revision that are taken.
const LONGEST: usize = 4096;
/// More patterns than this are not listed.
const MOST_PATTERNS: usize = 2000;
/// More files and locks than this are not listed.
const MOST_LISTED: usize = 50_000;
/// More attribute files than this are not read.
const MOST_SOURCES: usize = 200;
/// More of one attribute file than this is not read for the patterns.
const READ_OF_ONE: u64 = 1024 * 1024;
/// More than this is not read for the patterns from all attribute files
/// together.
const READ_OF_ALL: u64 = 4 * 1024 * 1024;
/// A `.gitattributes` larger than this is not changed from here.
const LARGEST_CHANGED: u64 = 16 * 1024 * 1024;
/// As much of a hook is read to see whether it calls git-lfs.
const READ_OF_HOOK: u64 = 64 * 1024;
/// A pointer file is smaller than this.
const POINTER_LIMIT: u64 = 1024;
/// This many files are opened to see that they hold a pointer; beyond
/// that, a file the filter applies to is counted as it is.
const POINTERS_READ: usize = 5000;
/// How long an answer of `git lfs version` is taken to hold.
const VERSION_HELD: Duration = Duration::from_secs(30);
/// A server that refused within this time is asked again (who holds the
/// lock, or the list of locks alone); one that took longer to fail is not.
const ASKED_AGAIN_WITHIN: Duration = Duration::from_secs(5);
/// As much of what git-lfs said is passed on as the reason of a refusal.
const LONGEST_REASON: usize = 2000;
/// What git-lfs prints as the time of a lock that came without one: the
/// first moment of its calendar (3.8.0 does).
const NO_TIME: &str = "0001-01-01T00:00:00Z";
/// The hooks git-lfs installs.
const HOOKS: [&str; 4] = ["pre-push", "post-checkout", "post-commit", "post-merge"];
/// Where a repository's own patterns are.
const ATTRIBUTES: &str = ".gitattributes";

/// A pattern whose files go through LFS.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfsPattern {
    /// The pattern, seen from the top of the worktree: one that a folder's
    /// own attribute file declares has that folder before it.
    pub pattern: String,
    /// The attribute file that declares it, relative to the worktree.
    pub source: String,
    /// Its files are meant to be locked before they are edited.
    pub lockable: bool,
}

/// How LFS stands in a repository.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfsStatus {
    /// Git has git-lfs.
    pub installed: bool,
    /// Its version.
    pub version: Option<String>,
    /// The pre-push hook that will run here calls git-lfs.
    pub hooks: bool,
    /// The `filter.lfs` commands are set for this repository, in whichever
    /// configuration.
    pub filters: bool,
    /// The patterns that are tracked.
    pub patterns: Vec<LfsPattern>,
    /// How many files in the index are LFS files: the filter applies to
    /// them and what is stored for them is a pointer. Nothing is counted
    /// while no pattern is tracked.
    pub files: usize,
    /// A pattern is tracked.
    pub active: bool,
}

/// What setting LFS up here did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Installed {
    /// How LFS stands now.
    #[serde(flatten)]
    pub status: LfsStatus,
    /// Why no hooks were written, when they were left alone.
    pub hooks_skipped: Option<String>,
}

/// What tracking a pattern did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Tracked {
    /// The pattern as it was given.
    pub pattern: String,
    /// Whether it is lockable now.
    pub lockable: bool,
    /// `.gitattributes` was changed; not when the pattern was tracked like
    /// this already.
    pub changed: bool,
}

/// What untracking a pattern did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Untracked {
    /// The pattern as it was given.
    pub pattern: String,
    /// `.gitattributes` was changed; not when the pattern was not tracked.
    pub changed: bool,
}

/// An LFS file of a commit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfsFile {
    /// Where it is.
    pub path: String,
    /// The id of its content: a SHA-256, in hex.
    pub oid: String,
    /// How large its content is, in bytes.
    pub size: u64,
    /// The content is in the local store; without it there is only the
    /// pointer.
    pub present: bool,
    /// The file in the worktree holds the content rather than the pointer.
    pub checked_out: bool,
}

/// The LFS files of a commit.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LfsFiles {
    /// The commit that was looked at; none on a branch without commits.
    pub commit: Option<Oid>,
    /// Its LFS files.
    pub files: Vec<LfsFile>,
    /// There are more than are listed.
    pub more: bool,
}

/// What a fetch, pull or push of LFS content did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Transferred {
    /// The remote it went to or came from.
    pub remote: String,
    /// What git-lfs said.
    pub output: String,
}

/// What putting LFS content in place did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct CheckedOut {
    /// What git-lfs said: how many files it put in place, and which it
    /// passed over because their content is not here.
    pub output: String,
}

/// A lock on a file, as the remote's server keeps it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Lock {
    /// The server's id for it.
    pub id: String,
    /// The file.
    pub path: String,
    /// Who holds it.
    pub owner: Option<String>,
    /// Since when, as the server writes it.
    pub locked_at: Option<String>,
    /// It is held by this user; absent when the server did not say.
    pub ours: Option<bool>,
}

/// What unlocking a file did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Unlocked {
    /// The file.
    pub path: String,
    /// It is no longer locked.
    pub unlocked: bool,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("lfs-what"),
        detail: detail.into(),
    }
}

/// A relative path that names something inside the worktree: no way up,
/// no way to the root, nothing git could read as an option.
fn inside(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= LONGEST
        && !text.starts_with('-')
        && !text.chars().any(char::is_control)
        && text
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(text)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn workdir(repo: &Repository) -> Result<&Path> {
    repo.workdir()
        .ok_or_else(|| refused(say!("lfs-bare-no-files")))
}

// ---------------------------------------------------------------------
// Whether git-lfs is there.

/// What `git lfs version` last said: when, to which git, and the answer.
type Known = (Instant, OsString, Option<String>);

/// Asked on every refresh, so an answer is taken to hold for a while. A
/// command that needs git-lfs asks again and renews it.
static VERSION: Mutex<Option<Known>> = Mutex::new(None);

/// The version in what `git lfs version` prints, which begins
/// `git-lfs/3.8.0 (…)`.
fn parse_version(said: &str) -> String {
    let word = said.split_whitespace().next().unwrap_or("");
    word.strip_prefix("git-lfs/")
        .unwrap_or(word)
        .chars()
        .take(40)
        .collect()
}

/// The version of git-lfs, when git has it. Unless `fresh`, an answer of
/// a moment ago is given again.
fn version(repo: &Repository, fresh: bool) -> Option<String> {
    let git = exe::git_binary();
    if !fresh {
        // A poisoned lock only means asking again.
        let held = VERSION.lock().ok().and_then(|known| known.clone());
        if let Some((asked, binary, version)) = held
            && binary == git
            && asked.elapsed() < VERSION_HELD
        {
            return version;
        }
    }
    // Without git-lfs, git says that `lfs` is no command of its own.
    let version = match exe::run_with_code(repo, ["lfs", "version"], None) {
        Ok((Some(0), out)) => Some(parse_version(&out.stdout)),
        _ => None,
    };
    if let Ok(mut known) = VERSION.lock() {
        *known = Some((Instant::now(), git, version.clone()));
    }
    version
}

/// Refuse when git-lfs is not installed.
fn require(repo: &Repository) -> Result<()> {
    if version(repo, true).is_some() {
        return Ok(());
    }
    Err(refused(say!("lfs-not-installed")))
}

/// Refuse to run git-lfs for the first time in a bare repository.
///
/// A command of git-lfs that works on a repository writes
/// `lfs.repositoryformatversion` into that repository's configuration
/// when it is not there yet, also one that only lists (3.8.0 does). A
/// bare repository is opened read-only, so there git-lfs is asked only
/// where it has been before and has nothing left to write.
fn leaves_bare_alone(repo: &Repository) -> Result<()> {
    if !repo.is_bare() {
        return Ok(());
    }
    let known = exe::run(
        repo,
        ["config", "--local", "--get", "lfs.repositoryformatversion"],
        None,
    )
    .is_ok_and(|out| !out.stdout.trim().is_empty());
    if known {
        return Ok(());
    }
    Err(refused(say!("lfs-bare-untouched")))
}

// ---------------------------------------------------------------------
// Attribute files.

/// What a list of attributes says about LFS, as far as it says anything.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Says {
    /// The filter is LFS, or it is expressly something else or nothing.
    filter: Option<bool>,
    /// Lockable, or expressly not.
    lockable: Option<bool>,
}

/// A line of an attribute file: a pattern and what it says.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    pattern: String,
    says: Says,
}

/// A pattern in double quotes, written the way C writes a string: the
/// text after the opening quote gives the pattern, and what follows the
/// closing quote.
pub(crate) fn unquote(rest: &str) -> Option<(String, &str)> {
    let mut out: Vec<u8> = Vec::new();
    let mut bytes = rest.bytes().enumerate();
    while let Some((at, byte)) = bytes.next() {
        match byte {
            b'"' => {
                let after = rest.get(at.saturating_add(1)..)?;
                return Some((String::from_utf8_lossy(&out).into_owned(), after));
            }
            b'\\' => {
                let (_, escaped) = bytes.next()?;
                match escaped {
                    b'a' => out.push(0x07),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'v' => out.push(0x0b),
                    b'0'..=b'3' => {
                        // A byte as three octal digits.
                        let mut value = u32::from(escaped.saturating_sub(b'0'));
                        for _ in 0..2 {
                            let (_, digit) = bytes.next()?;
                            if !(b'0'..=b'7').contains(&digit) {
                                return None;
                            }
                            value = value
                                .saturating_mul(8)
                                .saturating_add(u32::from(digit.saturating_sub(b'0')));
                        }
                        out.push(u8::try_from(value).ok()?);
                    }
                    other => out.push(other),
                }
            }
            other => out.push(other),
        }
    }
    None
}

/// What the attributes of one line say about LFS. A macro stands for what
/// it was defined as.
fn says(attributes: &str, macros: &HashMap<String, Says>) -> Says {
    let mut out = Says::default();
    for word in attributes.split_ascii_whitespace() {
        // `-name` takes an attribute away, `!name` leaves it unsaid; both
        // undo what an earlier word of the line set.
        let (name, value, set) = match word.strip_prefix(['-', '!']) {
            Some(name) => (name, None, false),
            None => match word.split_once('=') {
                Some((name, value)) => (name, Some(value), true),
                None => (word, None, true),
            },
        };
        match name {
            "filter" => out.filter = Some(set && value == Some("lfs")),
            "lockable" => out.lockable = Some(set && value.is_none()),
            _ => {
                if set
                    && value.is_none()
                    && let Some(meant) = macros.get(name)
                {
                    out.filter = meant.filter.or(out.filter);
                    out.lockable = meant.lockable.or(out.lockable);
                }
            }
        }
    }
    out
}

/// The lines of an attribute file. With `define`, the macros it defines
/// are added to `macros`; a file in a folder cannot define any.
fn rules(text: &str, macros: &mut HashMap<String, Says>, define: bool) -> Vec<Rule> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim_start_matches([' ', '\t']).trim_end_matches('\r');
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let (pattern, attributes) = match line.strip_prefix('"') {
            Some(rest) => match unquote(rest) {
                Some(found) => found,
                None => continue,
            },
            None => match line.split_once([' ', '\t']) {
                Some((pattern, attributes)) => (pattern.to_owned(), attributes),
                None => (line.to_owned(), ""),
            },
        };
        if let Some(name) = pattern.strip_prefix("[attr]") {
            if define && !name.is_empty() {
                let meant = says(attributes, macros);
                macros.insert(name.to_owned(), meant);
            }
            continue;
        }
        // Git reads no pattern that takes attributes away from paths.
        if pattern.is_empty() || pattern.starts_with('!') || pattern.len() > LONGEST {
            continue;
        }
        out.push(Rule {
            pattern,
            says: says(attributes, macros),
        });
    }
    out
}

/// The beginning of the file at `path`: of a regular file only, and
/// through a link only when `follow`. Anything else, a pipe say, is not
/// opened at all.
fn read_start(path: &Path, most: u64, follow: bool) -> Option<Vec<u8>> {
    let found = if follow {
        std::fs::metadata(path)
    } else {
        std::fs::symlink_metadata(path)
    };
    if !found.ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(most)
        .read_to_end(&mut bytes)
        .ok()?;
    Some(bytes)
}

/// Whether an entry of the index is a file, rather than a link or a
/// submodule.
fn is_file(mode: gix::index::entry::Mode) -> bool {
    use gix::index::entry::Mode;
    mode == Mode::FILE || mode == Mode::FILE_EXECUTABLE
}

/// The attribute files in folders of the worktree, as far as the index
/// knows them: one that was never added is not found.
fn nested_sources(repo: &Repository) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Ok(index) = repo.gix().index_or_empty() else {
        return out;
    };
    let suffix = format!("/{ATTRIBUTES}");
    for entry in index.entries() {
        let path = entry.path(&index);
        if !path.ends_with(suffix.as_bytes()) || !is_file(entry.mode) {
            continue;
        }
        let Ok(path) = std::str::from_utf8(path) else {
            continue;
        };
        // In a conflict a path is there once per side.
        if !inside(path) || out.last().is_some_and(|last| last == path) {
            continue;
        }
        if out.len() >= MOST_SOURCES {
            break;
        }
        out.push(path.to_owned());
    }
    out
}

/// The attribute file at `path` in the worktree `dir`, up to `most`
/// bytes. One that is not checked out is read as the index has it, which
/// is where git looks then; one of those that is larger is left out.
fn attribute_file(repo: &Repository, dir: &Path, path: &str, most: u64) -> Option<Vec<u8>> {
    use gix::bstr::ByteSlice as _;
    let file = dir.join(path);
    match file.symlink_metadata() {
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        _ => return read_start(&file, most, false),
    }
    let repo = repo.gix();
    let index = repo.index_or_empty().ok()?;
    let entry = index.entry_by_path(path.as_bytes().as_bstr())?;
    if !is_file(entry.mode) || repo.find_header(entry.id).ok()?.size() > most {
        return None;
    }
    Some(repo.find_blob(entry.id).ok()?.data.clone())
}

/// What `read` gives when it is allowed as many bytes as are `left` to
/// read, as text.
fn within(left: &mut u64, read: impl FnOnce(u64) -> Option<Vec<u8>>) -> Option<String> {
    let bytes = read((*left).min(READ_OF_ONE))?;
    *left = left.saturating_sub(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// The patterns that are tracked: in `.gitattributes` at the top of the
/// worktree, in those of its folders, and in the repository's own
/// `info/attributes`. Those of the user's and the system's attribute
/// files are not looked for.
fn patterns(repo: &Repository) -> Vec<LfsPattern> {
    let mut left = READ_OF_ALL;
    let top = repo.workdir().and_then(|dir| {
        within(&mut left, |most| {
            attribute_file(repo, dir, ATTRIBUTES, most)
        })
    });
    let info = repo.gix().common_dir().join("info").join("attributes");
    let info = within(&mut left, |most| read_start(&info, most, true));

    // Macros count wherever they are used, so they are gathered first.
    let mut macros = HashMap::new();
    for text in [&info, &top].into_iter().flatten() {
        let _ = rules(text, &mut macros, true);
    }

    let mut out: Vec<LfsPattern> = Vec::new();
    let mut take = |source: &str, folder: Option<&str>, text: &str| {
        for rule in rules(text, &mut macros, false) {
            if rule.says.filter != Some(true) || out.len() >= MOST_PATTERNS {
                continue;
            }
            let pattern = match folder {
                // A pattern is anchored where its file is.
                Some(folder) => format!("{folder}/{}", rule.pattern.trim_start_matches('/')),
                None => rule.pattern,
            };
            out.push(LfsPattern {
                pattern,
                source: source.to_owned(),
                lockable: rule.says.lockable == Some(true),
            });
        }
    };
    if let Some(text) = &top {
        take(ATTRIBUTES, None, text);
    }
    if let Some(dir) = repo.workdir() {
        for source in nested_sources(repo) {
            let read = |most| attribute_file(repo, dir, &source, most);
            let Some(text) = within(&mut left, read) else {
                continue;
            };
            let folder = source
                .strip_suffix(ATTRIBUTES)
                .map(|folder| folder.trim_end_matches('/'));
            take(&source, folder, &text);
        }
    }
    if let Some(text) = &info {
        take(".git/info/attributes", None, text);
    }
    out
}

// ---------------------------------------------------------------------
// The status.

/// Whether these bytes are a pointer file: a few lines of a key and a
/// value, the version of the format first, with the id and the size of the
/// content among them.
fn is_pointer(bytes: &[u8]) -> bool {
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) >= POINTER_LIMIT {
        return false;
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut lines = text.lines();
    // The second address is the one the format had before it was released.
    if !matches!(
        lines.next(),
        Some(
            "version https://git-lfs.github.com/spec/v1"
                | "version https://hawser.github.com/spec/v1"
        )
    ) {
        return false;
    }
    let (mut id, mut size) = (false, false);
    for line in lines {
        match line.split_once(' ') {
            Some(("oid", value)) => {
                id = value.strip_prefix("sha256:").is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
            }
            Some(("size", value)) => {
                size = !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit());
            }
            _ => {}
        }
    }
    id && size
}

/// Whether the blob `id` is a pointer file. Its size is asked first, so
/// that content which was never turned into a pointer is not loaded.
fn is_pointer_blob(repo: &gix::Repository, id: Oid) -> bool {
    let id = *id.as_gix();
    let small = repo
        .find_header(id)
        .is_ok_and(|header| header.size() < POINTER_LIMIT);
    small && repo.find_blob(id).is_ok_and(|blob| is_pointer(&blob.data))
}

/// How many files in the index the LFS filter applies to with a pointer
/// stored for them. Git picks the files by their attributes; the pointers
/// are read here.
fn count_files(repo: &Repository) -> usize {
    if repo.workdir().is_none() {
        return 0;
    }
    let Ok((Some(0), out)) = exe::run_with_code(
        repo,
        ["ls-files", "-s", "-z", "--", ":(attr:filter=lfs)"],
        None,
    ) else {
        return 0;
    };
    let (mut count, mut read) = (0usize, 0usize);
    for record in out.stdout.split('\0') {
        // `<mode> <id> <stage>`, a tab, the path.
        let Some((head, _)) = record.split_once('\t') else {
            continue;
        };
        let mut parts = head.split(' ');
        let (Some("100644" | "100755"), Some(id), Some("0")) =
            (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if read < POINTERS_READ {
            read = read.saturating_add(1);
            if !id
                .parse::<Oid>()
                .is_ok_and(|id| is_pointer_blob(repo.gix(), id))
            {
                continue;
            }
        }
        count = count.saturating_add(1);
    }
    count
}

/// What the configuration in effect says that matters here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Settings {
    /// The LFS filter has its commands.
    filters: bool,
    /// Hooks are looked for in a directory of the user's choosing.
    hooks_path: bool,
}

fn settings(repo: &Repository) -> Settings {
    let mut found: HashMap<String, String> = HashMap::new();
    // Exit code 1 says that nothing is set.
    if let Ok((Some(0), out)) = exe::run_with_code(
        repo,
        [
            "config",
            "-z",
            "--get-regexp",
            r"^(filter\.lfs\.(clean|smudge|process)|core\.hookspath)$",
        ],
        None,
    ) {
        // Each setting is its key, a line feed, its value and a NUL; the
        // one that comes last is the one in effect.
        for record in out.stdout.split('\0') {
            let (key, value) = record.split_once('\n').unwrap_or((record, ""));
            found.insert(key.to_owned(), value.to_owned());
        }
    }
    let set = |key: &str| found.get(key).is_some_and(|value| !value.trim().is_empty());
    Settings {
        filters: set("filter.lfs.clean") && (set("filter.lfs.process") || set("filter.lfs.smudge")),
        hooks_path: found.contains_key("core.hookspath"),
    }
}

/// Where git looks for hooks. Only when `core.hooksPath` is set does git
/// have to be asked.
fn hooks_dir(repo: &Repository, configured: bool) -> PathBuf {
    if configured && let Ok(out) = exe::run(repo, ["rev-parse", "--git-path", "hooks"], None) {
        let said = out.stdout.strip_suffix('\n').unwrap_or(&out.stdout);
        if !said.is_empty() {
            // Relative to where git ran.
            return repo.workdir().unwrap_or_else(|| repo.git_dir()).join(said);
        }
    }
    repo.gix().common_dir().join("hooks")
}

/// Whether the hook at `path` calls git-lfs.
fn calls_lfs(path: &Path) -> bool {
    read_start(path, READ_OF_HOOK, true).is_some_and(|bytes| {
        let text = String::from_utf8_lossy(&bytes);
        text.contains("git lfs") || text.contains("git-lfs")
    })
}

/// Whether git would run this file as a hook: marked as executable for
/// someone.
#[cfg(unix)]
fn runs(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Whether git would run this file as a hook: where files carry no such
/// mark, being there is enough.
#[cfg(not(unix))]
fn runs(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

fn status_with(repo: &Repository, version: Option<String>) -> LfsStatus {
    let settings = settings(repo);
    let pre_push = hooks_dir(repo, settings.hooks_path).join("pre-push");
    let patterns = patterns(repo);
    let active = !patterns.is_empty();
    LfsStatus {
        installed: version.is_some(),
        version,
        hooks: runs(&pre_push) && calls_lfs(&pre_push),
        filters: settings.filters,
        // Without a pattern there is nothing the filter applies to.
        files: if active { count_files(repo) } else { 0 },
        patterns,
        active,
    }
}

/// How LFS stands here. This never fails: what cannot be found out reads
/// as not there. Without git-lfs the patterns and the files are still
/// told.
///
/// It is asked on every refresh, so it is kept cheap. Without a tracked
/// pattern it runs one `git config`, reads the attribute files and the
/// pre-push hook, and looks through the index for attribute files in
/// folders; `git lfs version` is asked once in half a minute. While a
/// pattern is tracked there is one `git ls-files` on top, which goes over
/// the whole index, and a pointer read per LFS file, up to a bound.
pub fn status(repo: &Repository) -> LfsStatus {
    status_with(repo, version(repo, false))
}

// ---------------------------------------------------------------------
// Setting it up.

/// `path` with links followed, as far as it exists; what is not there yet
/// is taken as named.
fn settled(path: &Path) -> Option<PathBuf> {
    let mut missing: Vec<OsString> = Vec::new();
    let mut at = path;
    loop {
        if let Ok(real) = bitbraid_core::paths::canonical(at) {
            let mut out = real;
            out.extend(missing.iter().rev());
            return Some(out);
        }
        missing.push(at.file_name()?.to_owned());
        at = at.parent()?;
    }
}

/// Whether `path` is in the repository: under its git directory or in
/// its worktree.
fn in_repository(repo: &Repository, path: &Path) -> bool {
    let Some(path) = settled(path) else {
        return false;
    };
    [Some(repo.gix().common_dir()), repo.workdir()]
        .into_iter()
        .flatten()
        .filter_map(settled)
        .any(|root| path.starts_with(root))
}

/// Set LFS up for this repository: the filter's commands in its own
/// configuration, and the hooks. Nothing outside the repository is
/// written: where the hooks are kept elsewhere (`core.hooksPath`, or a
/// link), they are left alone and the answer says so. A hook of the
/// user's own that does not call git-lfs is in the way, and then nothing
/// is done; one that does call it is kept as it is.
pub fn install(repo: &Repository) -> Result<Installed> {
    require(repo)?;
    let hooks = hooks_dir(repo, settings(repo).hooks_path);
    let ours = in_repository(repo, &hooks);
    // Hooks that are there and call git-lfs, and one that is a link.
    let (mut there, mut linked) = (false, false);
    if ours {
        for name in HOOKS {
            let hook = hooks.join(name);
            let Ok(meta) = hook.symlink_metadata() else {
                continue;
            };
            if !calls_lfs(&hook) {
                return Err(refused(say!(
                    "lfs-hook-in-way",
                    name,
                    path = hook.display().to_string()
                )));
            }
            there = true;
            linked |= meta.file_type().is_symlink();
        }
    }
    let with_hooks = ["lfs", "install", "--local"];
    let filters_only = ["lfs", "install", "--local", "--skip-repo"];
    let mut hooks_skipped = None;
    if !ours {
        exe::run(repo, filters_only, None)?;
        hooks_skipped = Some(say!(
            "lfs-hooks-elsewhere",
            path = hooks.display().to_string()
        ));
    } else if linked {
        // Not written through, wherever it leads; it calls git-lfs.
        exe::run(repo, filters_only, None)?;
    } else if let Err(err) = exe::run(repo, with_hooks, None) {
        // A hook that calls git-lfs but was not written by it stops
        // git-lfs just the same, after the filter is set. That hook does
        // its part, so this makes sure of the filter and leaves it there.
        if !there {
            return Err(err);
        }
        exe::run(repo, filters_only, None)?;
    }
    Ok(Installed {
        status: status_with(repo, version(repo, false)),
        hooks_skipped,
    })
}

// ---------------------------------------------------------------------
// Tracking.

/// Check a pattern before git-lfs writes it into `.gitattributes`.
fn check_pattern(pattern: &str) -> Result<()> {
    let bad = |why: String| Err(invalid(say!("lfs-what-pattern"), why));
    let quoted = || format!("{pattern:?}");
    if pattern.trim().is_empty() {
        return bad(say!("lfs-pattern-empty", pattern = quoted()));
    }
    if pattern.len() > LONGEST {
        return Err(invalid(say!("lfs-what-pattern"), say!("lfs-too-long")));
    }
    if pattern.chars().any(char::is_control) {
        return bad(say!("lfs-pattern-control", pattern = quoted()));
    }
    if pattern.starts_with('-') {
        return bad(say!("lfs-pattern-dash", pattern = quoted()));
    }
    // The two that follow are what gitattributes(5) says cannot work.
    if pattern.starts_with('!') {
        return bad(say!("lfs-pattern-negated", pattern = quoted()));
    }
    if pattern.ends_with('/') {
        return bad(say!("lfs-pattern-folder", pattern = quoted()));
    }
    if pattern.split('/').any(|part| part == "..") {
        return bad(say!("lfs-pattern-outside", pattern = quoted()));
    }
    Ok(())
}

/// A pattern as git-lfs writes it into `.gitattributes`.
fn escape(pattern: &str) -> String {
    pattern
        .replace('\\', "\\\\")
        .replace(' ', "[[:space:]]")
        .replace('#', "\\#")
}

/// What [`escape`] was given, for a pattern as `.gitattributes` has it.
fn unescape(written: &str) -> String {
    let spaced = written.replace("[[:space:]]", " ");
    let mut out = String::with_capacity(spaced.len());
    let mut chars = spaced.chars().peekable();
    while let Some(c) = chars.next() {
        match (c, chars.peek()) {
            ('\\', Some(&next @ ('\\' | '#'))) => {
                out.push(next);
                let _ = chars.next();
            }
            _ => out.push(c),
        }
    }
    out
}

/// What `.gitattributes` at the top of the worktree holds, when it is
/// there. It has to be a file of its own: git reads no attributes through
/// a link, and git-lfs would write through one to wherever it leads.
fn own_attributes(file: &Path) -> Result<Option<Vec<u8>>> {
    let meta = match file.symlink_metadata() {
        Ok(meta) => meta,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => {
            return Err(refused(say!(
                "lfs-attributes-unreadable",
                file = ATTRIBUTES,
                err = err.to_string()
            )));
        }
    };
    if meta.file_type().is_symlink() {
        return Err(refused(say!("lfs-attributes-link", file = ATTRIBUTES)));
    }
    if !meta.is_file() {
        return Err(refused(say!("lfs-attributes-not-file", file = ATTRIBUTES)));
    }
    if meta.len() > LARGEST_CHANGED {
        return Err(refused(say!("lfs-attributes-too-large", file = ATTRIBUTES)));
    }
    std::fs::read(file).map(Some).map_err(|err| {
        refused(say!(
            "lfs-attributes-unreadable",
            file = ATTRIBUTES,
            err = err.to_string()
        ))
    })
}

/// Put `.gitattributes` back as it was.
fn restore(file: &Path, before: Option<&[u8]>) {
    let _ = match before {
        Some(bytes) => std::fs::write(file, bytes),
        None => std::fs::remove_file(file),
    };
}

/// How the attribute file `content` tracks `pattern`, when it does:
/// whether it is lockable there.
fn tracked_in(content: Option<&[u8]>, pattern: &str) -> Option<bool> {
    let text = String::from_utf8_lossy(content?);
    let mut macros = HashMap::new();
    let _ = rules(&text, &mut macros, true);
    let escaped = escape(pattern);
    rules(&text, &mut macros, false)
        .into_iter()
        .rev()
        .find(|rule| rule.pattern == pattern || rule.pattern == escaped)
        .filter(|rule| rule.says.filter == Some(true))
        .map(|rule| rule.says.lockable == Some(true))
}

/// Run a git-lfs command that edits `.gitattributes`, and say whether it
/// changed it. When the command fails the file is put back as it was.
fn editing(
    repo: &Repository,
    file: &Path,
    before: Option<&[u8]>,
    args: &[&str],
) -> Result<(exe::Output, Option<Vec<u8>>)> {
    let ran = exe::run(repo, args, None);
    let after = read_start(file, LARGEST_CHANGED.saturating_add(1), false);
    match ran {
        Ok(out) => Ok((out, after)),
        Err(err) => {
            if after.as_deref() != before {
                restore(file, before);
            }
            Err(err)
        }
    }
}

/// Track `pattern` with LFS: `.gitattributes` at the top of the worktree
/// is changed and left for the user to commit. `lockable` marks its files
/// as to be locked or takes the mark away; absent, the mark stays as it is.
/// A pattern that is tracked like this already is not an error.
pub fn track(repo: &Repository, pattern: &str, lockable: Option<bool>) -> Result<Tracked> {
    let dir = workdir(repo)?;
    check_pattern(pattern)?;
    require(repo)?;
    let file = dir.join(ATTRIBUTES);
    let before = own_attributes(&file)?;
    let was = tracked_in(before.as_deref(), pattern);

    let mut args = vec!["lfs", "track"];
    match lockable {
        Some(true) => args.push("--lockable"),
        Some(false) => args.push("--not-lockable"),
        None => {}
    }
    args.extend(["--", pattern]);
    let (out, after) = editing(repo, &file, before.as_deref(), &args)?;
    let changed = after != before;
    // When it cannot write the file, git-lfs says so and still ends well.
    if !changed && was.is_none() && !out.stdout.contains("already supported") {
        let said = format!("{}{}", out.stdout, out.stderr);
        return Err(refused(say!(
            "lfs-not-written",
            file = ATTRIBUTES,
            said = said.trim()
        )));
    }
    Ok(Tracked {
        pattern: pattern.to_owned(),
        lockable: tracked_in(after.as_deref(), pattern).unwrap_or(lockable == Some(true)),
        changed,
    })
}

/// Stop tracking `pattern`: its line leaves `.gitattributes` at the top of
/// the worktree, which is left for the user to commit. A pattern that is
/// not tracked there is not an error. The pattern is taken as it was
/// tracked or as the file has it.
pub fn untrack(repo: &Repository, pattern: &str) -> Result<Untracked> {
    let dir = workdir(repo)?;
    check_pattern(pattern)?;
    require(repo)?;
    let file = dir.join(ATTRIBUTES);
    let answer = |changed| Untracked {
        pattern: pattern.to_owned(),
        changed,
    };
    let Some(before) = own_attributes(&file)? else {
        return Ok(answer(false));
    };
    let (_, after) = editing(
        repo,
        &file,
        Some(&before),
        &["lfs", "untrack", "--", pattern],
    )?;
    if after.as_deref() != Some(before.as_slice()) {
        return Ok(answer(true));
    }
    // Git-lfs compares what it is given with the file's patterns after
    // escaping it once more, so one taken from the file has to be given
    // the way it was first typed.
    let typed = unescape(pattern);
    if typed == pattern || check_pattern(&typed).is_err() {
        return Ok(answer(false));
    }
    let (_, after) = editing(
        repo,
        &file,
        Some(&before),
        &["lfs", "untrack", "--", typed.as_str()],
    )?;
    Ok(answer(after.as_deref() != Some(before.as_slice())))
}

// ---------------------------------------------------------------------
// The files of a commit.

/// The files in what `git lfs ls-files --json` printed, and whether there
/// were more than are listed.
fn parse_files(text: &str) -> Result<(Vec<LfsFile>, bool)> {
    let said: Value = serde_json::from_str(text).map_err(|err| {
        invalid(
            say!("lfs-what-answer"),
            say!("lfs-not-file-list", err = err.to_string()),
        )
    })?;
    // Without any file the list is `null`.
    let listed = said
        .get("files")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let files = listed
        .iter()
        .filter_map(|file| {
            let flag = |name: &str| file.get(name).and_then(Value::as_bool).unwrap_or(false);
            Some(LfsFile {
                path: file.get("name")?.as_str()?.to_owned(),
                oid: file
                    .get("oid")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                size: file.get("size").and_then(Value::as_u64).unwrap_or(0),
                present: flag("downloaded"),
                checked_out: flag("checkout"),
            })
        })
        .take(MOST_LISTED)
        .collect();
    Ok((files, listed.len() > MOST_LISTED))
}

/// The LFS files in the commit `rev` names (HEAD when absent): where each
/// is, the id and the size of its content, and whether that content is
/// here.
pub fn files(repo: &Repository, rev: Option<&str>) -> Result<LfsFiles> {
    let named = rev.unwrap_or("HEAD");
    if named.len() > LONGEST {
        return Err(invalid(say!("lfs-what-revision"), say!("lfs-too-long")));
    }
    require(repo)?;
    leaves_bare_alone(repo)?;
    let commit = branch::resolve(repo, named);
    // A branch without commits has no files; any other name has to be one
    // of a commit.
    if commit.is_none() && rev.is_some() {
        return Err(invalid(
            say!("lfs-what-revision"),
            say!("lfs-not-commit", named = format!("{named:?}")),
        ));
    }
    let Some(commit) = commit else {
        return Ok(LfsFiles {
            commit: None,
            files: Vec::new(),
            more: false,
        });
    };
    let hex = commit.to_hex();
    let out = exe::run(
        repo,
        ["lfs", "ls-files", "--json", "--", hex.as_str()],
        None,
    )
    .map_err(|err| match &err {
        Error::Command { stderr, .. } if stderr.contains("unknown flag") => {
            refused(say!("lfs-too-old"))
        }
        _ => err,
    })?;
    let (files, more) = parse_files(&out.stdout)?;
    Ok(LfsFiles {
        commit: Some(commit),
        files,
        more,
    })
}

// ---------------------------------------------------------------------
// Content to and from a remote.

fn remotes(repo: &Repository) -> Result<Vec<String>> {
    let out = exe::run(repo, ["remote"], None)?;
    Ok(out
        .stdout
        .lines()
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .collect())
}

/// The remote an operation goes to: the one that is named, which has to
/// be configured; without a name the one the current branch follows, else
/// `origin`, else the only one there is.
fn remote_for(repo: &Repository, named: Option<&str>) -> Result<String> {
    if let Some(name) = named {
        remote::check_remote_name(repo, name)?;
    }
    let known = remotes(repo)?;
    if let Some(name) = named {
        if !known.iter().any(|known| known == name) {
            return Err(invalid(
                say!("lfs-what-remote"),
                say!("lfs-no-such-remote", name = format!("{name:?}")),
            ));
        }
        return Ok(name.to_owned());
    }
    if known.is_empty() {
        return Err(refused(say!("lfs-no-remote")));
    }
    let followed = branch::head_state(repo)?
        .branch
        .and_then(|branch| exe::config_get(repo, &format!("branch.{branch}.remote")))
        .filter(|followed| known.contains(followed));
    match followed {
        Some(followed) => Ok(followed),
        None => remote::default_remote(repo),
    }
}

/// Run a git-lfs command that takes its time: one that talks to a remote,
/// or moves content. What it reports is passed on as it comes, and asked
/// to stop it is ended together with git-lfs and what git-lfs started.
///
/// With `moving`, the command moves content: it is asked for its meter,
/// which it would not write without a terminal, and for its report of
/// the bytes of each file, and both are read into the progress that is
/// passed on (see [`report`]). The report is a file in the user's
/// runtime directory, or where there is none in the repository's git
/// directory, for as long as the command runs, so this is not for what
/// only reads. Without `moving`, what the command prints is an answer, or
/// why it failed, and not looked at here: a line such as `Error while
/// retrieving locks: …` reads like a stage of progress and is none.
fn talking(
    repo: &Repository,
    args: &[&str],
    moving: bool,
    reporter: &mut Reporter<'_>,
) -> Result<exe::Output> {
    let cancel = reporter.cancel.clone();
    let file = if moving {
        report::Report::begin(repo)
    } else {
        None
    };
    let mut env: Vec<(&str, &std::ffi::OsStr)> = Vec::new();
    if moving {
        env.push((report::METER, "1".as_ref()));
    }
    if let Some(file) = &file {
        env.push((report::REPORT, file.path().as_os_str()));
    }
    let mut meter = report::Meter::default();
    let out = exe::run_followed(
        repo,
        args,
        &env,
        file.as_ref().map(report::Report::path),
        &cancel,
        &mut |said| {
            let progress = match said {
                exe::Said::Reported(line) => meter.reported(line),
                exe::Said::Err(line) | exe::Said::Out(line) if moving => meter.said(line),
                exe::Said::Err(_) | exe::Said::Out(_) => None,
            };
            if let Some(progress) = progress {
                (reporter.on_progress)(progress);
            }
        },
    )?;
    for progress in meter.ended_well() {
        (reporter.on_progress)(progress);
    }
    Ok(out)
}

/// What git-lfs said while it moved content and ended well, of both
/// kinds, as one text. Its meter is in it as it stood at the end; where
/// it did not get to write that, the meter is left out rather than left
/// standing at less.
fn said(out: &exe::Output) -> String {
    let mut text = String::new();
    for line in out.stdout.lines().chain(out.stderr.lines()) {
        if !report::is_stale_meter(line) {
            text.push_str(line);
            text.push('\n');
        }
    }
    text.trim().to_owned()
}

fn transfer(
    repo: &Repository,
    remote: String,
    args: &[&str],
    reporter: &mut Reporter<'_>,
) -> Result<Transferred> {
    // A refusal that is known (the server's certificate) is said in words
    // before git-lfs's own, as for git's fetch and push.
    let out = talking(repo, args, true, reporter).map_err(remote::explained)?;
    Ok(Transferred {
        remote,
        output: said(&out),
    })
}

/// Download the content of the LFS files of what is checked out from
/// `remote`; with `all`, of every commit that any reference there
/// reaches. Files in the worktree are left as they are.
pub fn fetch(
    repo: &Repository,
    remote: Option<&str>,
    all: bool,
    reporter: &mut Reporter<'_>,
) -> Result<Transferred> {
    require(repo)?;
    let remote = remote_for(repo, remote)?;
    let mut args = vec!["lfs", "fetch"];
    if all {
        args.push("--all");
    }
    args.extend(["--", remote.as_str()]);
    transfer(repo, remote.clone(), &args, reporter)
}

/// Download the content of the LFS files of what is checked out from
/// `remote` and put it in place of the pointers in the worktree. That
/// needs the filter: without it the pointers would stay, so it is refused.
pub fn pull(
    repo: &Repository,
    remote: Option<&str>,
    reporter: &mut Reporter<'_>,
) -> Result<Transferred> {
    workdir(repo)?;
    require(repo)?;
    let remote = remote_for(repo, remote)?;
    if !settings(repo).filters {
        return Err(refused(say!("lfs-not-set-up-pull")));
    }
    let args = ["lfs", "pull", "--", remote.as_str()];
    transfer(repo, remote.clone(), &args, reporter)
}

/// Upload to `remote` the content of the LFS files of the current branch
/// that it does not have; with `all`, of every commit that any local
/// reference reaches.
pub fn push(
    repo: &Repository,
    remote: Option<&str>,
    all: bool,
    reporter: &mut Reporter<'_>,
) -> Result<Transferred> {
    require(repo)?;
    let remote = remote_for(repo, remote)?;
    // Without `--all` git-lfs wants to be told what to push.
    let current = branch::head_state(repo)?
        .branch
        .unwrap_or_else(|| "HEAD".to_owned());
    let args = if all {
        vec!["lfs", "push", "--all", "--", remote.as_str()]
    } else {
        vec!["lfs", "push", "--", remote.as_str(), current.as_str()]
    };
    transfer(repo, remote.clone(), &args, reporter)
}

/// Put the content of the LFS files that are checked out in place of
/// their pointers in the worktree, as far as it is in the local store.
/// Nothing is downloaded: content that is not here is passed over, and
/// git-lfs says for which files. A file that was changed is left as it
/// is. Like [`pull`] this needs the filter, without which git-lfs does
/// nothing and says so while ending well.
pub fn checkout(repo: &Repository, reporter: &mut Reporter<'_>) -> Result<CheckedOut> {
    workdir(repo)?;
    require(repo)?;
    if !settings(repo).filters {
        return Err(refused(say!("lfs-not-set-up-checkout")));
    }
    let out = talking(repo, &["lfs", "checkout"], true, reporter)?;
    Ok(CheckedOut { output: said(&out) })
}

// ---------------------------------------------------------------------
// Locks.

fn lock_from(value: &Value, ours: Option<bool>) -> Option<Lock> {
    let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
    Some(Lock {
        id: text(value.get("id")).unwrap_or_default(),
        path: text(value.get("path"))?,
        owner: text(value.get("owner").and_then(|owner| owner.get("name"))),
        // A server that gave no time has git-lfs print one all the same.
        locked_at: text(value.get("locked_at")).filter(|at| at != NO_TIME),
        ours,
    })
}

/// The locks in what git-lfs printed: a list, or, when the server was
/// asked whose they are, a list of ours and one of theirs.
fn parse_locks(text: &str) -> Result<Vec<Lock>> {
    let said: Value = serde_json::from_str(text).map_err(|err| {
        invalid(
            say!("lfs-what-answer"),
            say!("lfs-not-lock-list", err = err.to_string()),
        )
    })?;
    let list = |value: Option<&Value>, ours: Option<bool>| -> Vec<Lock> {
        value
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter_map(|lock| lock_from(lock, ours))
            .collect()
    };
    let mut out = if said.is_array() {
        list(Some(&said), None)
    } else {
        let mut out = list(said.get("ours"), Some(true));
        out.extend(list(said.get("theirs"), Some(false)));
        out
    };
    out.truncate(MOST_LISTED);
    Ok(out)
}

/// Locks are kept by the LFS server of a remote: without a remote, and
/// without an address set for LFS by itself, there is nobody to ask.
fn require_server(repo: &Repository) -> Result<()> {
    let by_itself = exe::config_get(repo, "lfs.url").is_some()
        || repo
            .workdir()
            .is_some_and(|dir| dir.join(".lfsconfig").is_file());
    if by_itself || !remotes(repo)?.is_empty() {
        return Ok(());
    }
    Err(refused(say!("lfs-no-remote-for-locks")))
}

/// The locks the server of the default remote keeps, with whether each is
/// this user's; where the server lets this user read the list but not
/// verify it (a reporter at GitLab), without that.
///
/// Asked whose the locks are, git-lfs keeps what the server answered in the
/// git directory, each time (`lfs/lockcache.db` and a file under
/// `lfs/cache/locks`; 3.8.0 does). So this is not something to ask on every
/// refresh, and a bare repository, which is opened read-only, is not asked
/// at all. The engine counts the command as one that changes the repository
/// for the same reason (`lfs.locks` in its table of access) and refuses it
/// there first; the refusal here is for whoever calls this directly.
pub fn locks(repo: &Repository, reporter: &mut Reporter<'_>) -> Result<Vec<Lock>> {
    require(repo)?;
    if repo.is_bare() {
        return Err(refused(say!("lfs-bare-locks")));
    }
    require_server(repo)?;
    let began = Instant::now();
    match listed(repo, true, reporter) {
        Ok(found) => Ok(found),
        Err(err) => {
            // Telling ours from theirs takes the right to push, where the list
            // itself takes only the right to read: GitLab refuses a reporter
            // the first and gives them the second (19.4 does). A server that
            // refused at once is asked for the list alone, without whose each
            // lock is; one that took long to fail or never answered, or one
            // that was refused for a known reason (its certificate, the
            // credentials), is not asked a second time. That the account may
            // not write is a known reason, and the very one the list alone is
            // for.
            let again = began.elapsed() < ASKED_AGAIN_WITHIN
                && matches!(&err, Error::Command { stderr, .. }
                    if remote::meaning_of(stderr)
                        .is_none_or(|meaning| meaning == remote::NOT_ALLOWED.text()));
            if again && let Ok(found) = listed(repo, false, reporter) {
                return Ok(found);
            }
            Err(not_listed(err))
        }
    }
}

/// The locks, asked whose each is (`--verify`) or not; a failure as git
/// ended with it.
fn listed(repo: &Repository, verify: bool, reporter: &mut Reporter<'_>) -> Result<Vec<Lock>> {
    let plain: &[&str] = if verify {
        &["lfs", "locks", "--verify"]
    } else {
        &["lfs", "locks"]
    };
    let mut args = plain.to_vec();
    args.push("--json");
    let out = talking(repo, &args, false, reporter)?;
    let found = parse_locks(&out.stdout)?;
    if found.is_empty() {
        // Asked for JSON, git-lfs prints an empty list and ends well also
        // when the server could not be asked (3.8.0 does). Asked in plain
        // text it fails as it should, so that is how "none" is confirmed.
        talking(repo, plain, false, reporter)?;
    }
    Ok(found)
}

/// The refusal for locks that could not be listed, from what git-lfs gave
/// as the reason; any other failure as it is.
fn not_listed(err: Error) -> Error {
    match err {
        Error::Command { stderr, .. } if !reason(&stderr).is_empty() => {
            let said = reason(&stderr);
            let said = said
                .strip_prefix("Error while retrieving locks: ")
                .unwrap_or(&said);
            refused(with_meaning(say!("lfs-locks-not-listed"), said))
        }
        err => err,
    }
}

/// `what` did not happen, and why, as git-lfs gave it: with what a
/// refusal that is known means before git-lfs's own words (a certificate
/// that was not accepted, which git-lfs says in Go's words).
fn with_meaning(what: &str, said: &str) -> String {
    match remote::meaning_of(said) {
        Some(meaning) => say!("lfs-with-meaning", what, meaning, said),
        None => format!("{what}: {said}"),
    }
}

/// Check the path of a file to lock or unlock: a plain path inside the
/// worktree, and not one inside the git directory, where no file of the
/// user's is.
fn check_locked_path(path: &str) -> Result<()> {
    let in_git_dir = path
        .split('/')
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(".git"));
    if !inside(path) || in_git_dir {
        return Err(invalid(
            say!("lfs-what-path"),
            say!("lfs-not-worktree-file", path = format!("{path:?}")),
        ));
    }
    Ok(())
}

/// What git-lfs wrote when it ended badly, as one line of bounded length.
fn reason(stderr: &str) -> String {
    let mut text = stderr.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.len() > LONGEST_REASON {
        let mut end = LONGEST_REASON;
        while !text.is_char_boundary(end) {
            end = end.saturating_sub(1);
        }
        text.truncate(end);
    }
    text
}

/// The locks the server keeps on `path`, when it can be asked. Asked for
/// one path and not whose the locks are, git-lfs writes nothing into the
/// repository (3.8.0 does not).
fn locks_on(repo: &Repository, path: &str, reporter: &mut Reporter<'_>) -> Option<Vec<Lock>> {
    let only = format!("--path={path}");
    let out = talking(
        repo,
        &["lfs", "locks", "--json", only.as_str()],
        false,
        reporter,
    )
    .ok()?;
    parse_locks(&out.stdout).ok()
}

/// The lock the server keeps on `path`, when it can be asked and keeps
/// one.
fn held_on(repo: &Repository, path: &str, reporter: &mut Reporter<'_>) -> Option<Lock> {
    locks_on(repo, path, reporter)?
        .into_iter()
        .find(|lock| lock.path == path)
}

/// Why a file that is locked already was not locked again: who holds the
/// lock and since when, as far as the server says.
fn already_locked(held: &Lock) -> Error {
    let path = held.path.as_str();
    let owner = held.owner.as_deref().filter(|owner| !owner.is_empty());
    let since = held.locked_at.as_deref().filter(|since| !since.is_empty());
    refused(match (owner, since) {
        (Some(owner), Some(since)) => say!("lfs-already-locked-by-since", path, owner, since),
        (Some(owner), None) => say!("lfs-already-locked-by", path, owner),
        (None, Some(since)) => say!("lfs-already-locked-since", path, since),
        (None, None) => say!("lfs-already-locked", path),
    })
}

/// Lock the file at `path` on the server of the default remote, so that
/// others are kept from pushing changes to it.
///
/// A server that keeps a lock on the file already refuses, in words of its
/// own that do not say whose it is; git-lfs passes them on and ends badly
/// (3.8.0 does). The server is then asked for the lock on that path, and
/// the refusal names who holds it.
pub fn lock(repo: &Repository, path: &str, reporter: &mut Reporter<'_>) -> Result<Lock> {
    let dir = workdir(repo)?;
    check_locked_path(path)?;
    require(repo)?;
    if !dir
        .join(path)
        .symlink_metadata()
        .is_ok_and(|meta| meta.is_file())
    {
        return Err(refused(say!("lfs-not-a-file-here", path)));
    }
    require_server(repo)?;
    let began = Instant::now();
    let out = match talking(
        repo,
        &["lfs", "lock", "--json", "--", path],
        false,
        reporter,
    ) {
        Ok(out) => out,
        Err(Error::Command {
            command,
            status,
            stderr,
        }) => {
            // A server that took long to fail, or never answered, is not
            // asked a second time, nor one refused for a known reason.
            if began.elapsed() < ASKED_AGAIN_WITHIN
                && remote::meaning_of(&stderr).is_none()
                && let Some(held) = held_on(repo, path, reporter)
            {
                return Err(already_locked(&held));
            }
            let said = reason(&stderr);
            if said.is_empty() {
                return Err(Error::Command {
                    command,
                    status,
                    stderr,
                });
            }
            let said = said
                .strip_prefix(&format!("Locking {path} failed: "))
                .unwrap_or(&said);
            return Err(refused(with_meaning(
                &say!("lfs-was-not-locked", path),
                said,
            )));
        }
        Err(err) => return Err(err),
    };
    parse_locks(&out.stdout)?
        .into_iter()
        .next()
        .map(|lock| Lock {
            ours: Some(true),
            ..lock
        })
        .ok_or_else(|| invalid(say!("lfs-what-answer"), say!("lfs-nothing-locked")))
}

/// Give up the lock on the file at `path`. With `force`, also a lock that
/// someone else holds, and also when the file has changes.
pub fn unlock(
    repo: &Repository,
    path: &str,
    force: bool,
    reporter: &mut Reporter<'_>,
) -> Result<Unlocked> {
    let dir = workdir(repo)?;
    check_locked_path(path)?;
    require(repo)?;
    require_server(repo)?;
    let mut args = vec!["lfs", "unlock"];
    if force {
        args.push("--force");
    }
    args.extend(["--json", "--", path]);
    let out = match talking(repo, &args, false, reporter) {
        Ok(out) => out,
        // Git-lfs ends badly when it did not unlock, having said why on
        // both of its outputs (3.8.0 does); only the one for errors is
        // kept of a command that ended badly.
        Err(Error::Command { stderr, .. }) if !reason(&stderr).is_empty() => {
            let why = reason(&stderr);
            // The git-lfs of Git for Windows (3.7.1) gives up the lock on
            // a file that is not here and then fails over the file, which
            // it looks for to make it writable again. Whether the server
            // still keeps the lock says which it was.
            let gone = force
                && dir.join(path).symlink_metadata().is_err()
                && !why.ends_with("no matching locks found")
                && remote::meaning_of(&why).is_none()
                && locks_on(repo, path, reporter)
                    .is_some_and(|locks| !locks.iter().any(|lock| lock.path == path));
            if gone {
                return Ok(Unlocked {
                    path: path.to_owned(),
                    unlocked: true,
                });
            }
            return Err(not_unlocked(path, &why));
        }
        Err(err) => return Err(err),
    };
    // One entry per path: `unlocked`, and a `reason` when it is not, for
    // a git-lfs that says so and ends well.
    let said: Option<Value> = serde_json::from_str(&out.stdout).ok();
    let entry = said.as_ref().and_then(|said| said.get(0));
    if entry
        .and_then(|entry| entry.get("unlocked"))
        .and_then(Value::as_bool)
        != Some(true)
    {
        let why = entry
            .and_then(|entry| entry.get("reason"))
            .and_then(Value::as_str)
            .map(reason)
            .filter(|why| !why.is_empty())
            .unwrap_or_else(|| say!("lfs-not-said-unlocked").to_owned());
        return Err(not_unlocked(path, &why));
    }
    Ok(Unlocked {
        path: path.to_owned(),
        unlocked: true,
    })
}

/// The refusal for a file that was not unlocked, from what git-lfs gave
/// as the reason: its own (the file has changes that are not committed),
/// or the server's (the lock is someone else's).
fn not_unlocked(path: &str, why: &str) -> Error {
    // Git-lfs asks the server for the lock on the path first, and says
    // this when the server keeps none.
    if why.ends_with("no matching locks found") {
        return refused(say!("lfs-not-locked", path));
    }
    refused(with_meaning(&say!("lfs-was-not-unlocked", path), why))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracked(text: &str) -> Vec<(String, bool)> {
        let mut macros = HashMap::new();
        let _ = rules(text, &mut macros, true);
        rules(text, &mut macros, false)
            .into_iter()
            .filter(|rule| rule.says.filter == Some(true))
            .map(|rule| (rule.pattern, rule.says.lockable == Some(true)))
            .collect()
    }

    #[test]
    fn attribute_lines_are_read_as_git_reads_them() {
        let text = concat!(
            "# a comment\n",
            "\n",
            "*.bin filter=lfs diff=lfs merge=lfs -text\n",
            "*.psd filter=lfs diff=lfs merge=lfs -text lockable\n",
            "   \t*.indented\t\tfilter=lfs   \n",
            "*.crlf filter=lfs\r\n",
            "my[[:space:]]file.dat filter=lfs\n",
            "\"quoted name.bin\" filter=lfs\n",
            "\"tab\\there\\\\and\\\"quote\\101\" filter=lfs lockable\n",
            "*.txt text\n",
            "*.other filter=other\n",
            "*.bare filter\n",
            "*.unset -filter\n",
            "*.unsaid !filter\n",
            "*.caps FILTER=lfs\n",
            "*.undone filter=lfs -filter\n",
            "*.redone -filter filter=lfs\n",
            "*.notlock filter=lfs -lockable\n",
            "!negative filter=lfs\n",
            "\"unclosed filter=lfs\n",
            "alone\n",
        );
        assert_eq!(
            tracked(text),
            [
                ("*.bin".to_owned(), false),
                ("*.psd".to_owned(), true),
                ("*.indented".to_owned(), false),
                ("*.crlf".to_owned(), false),
                ("my[[:space:]]file.dat".to_owned(), false),
                ("quoted name.bin".to_owned(), false),
                ("tab\there\\and\"quoteA".to_owned(), true),
                ("*.redone".to_owned(), false),
                ("*.notlock".to_owned(), false),
            ]
        );
    }

    #[test]
    fn a_macro_stands_for_its_attributes() {
        let text = concat!(
            "*.early big\n",
            "[attr]big filter=lfs diff=lfs merge=lfs -text\n",
            "[attr]locked big lockable\n",
            "[attr]plain text\n",
            "*.mac big\n",
            "*.both locked\n",
            "*.plain plain\n",
            "*.not -big\n",
            "*.valued big=yes\n",
            "*.over big -filter\n",
        );
        assert_eq!(
            tracked(text),
            [
                ("*.early".to_owned(), false),
                ("*.mac".to_owned(), false),
                ("*.both".to_owned(), true),
            ]
        );
        // A file in a folder defines none: the name is then just a name.
        let mut macros = HashMap::new();
        let found = rules("[attr]big filter=lfs\n*.mac big\n", &mut macros, false);
        assert!(macros.is_empty());
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].says, Says::default());
    }

    #[test]
    fn garbage_in_an_attribute_file_is_only_garbage() {
        let mut macros = HashMap::new();
        for text in [
            "",
            "\n\n\n",
            "\"",
            "\"\\",
            "\"\\4",
            "\"\\37",
            "\"\\399\" filter=lfs",
            "[attr]",
            "[attr] filter=lfs",
            "\u{0}\u{1}\u{2} filter=lfs",
            "= == =filter=lfs -- !! -=! filter=",
            &"x".repeat(10_000),
            &"*.a filter=lfs\n".repeat(5000),
        ] {
            let _ = rules(text, &mut macros, true);
            let _ = rules(text, &mut macros, false);
        }
        // A pattern too long to be one is left out.
        let long = format!("{} filter=lfs\n", "p".repeat(LONGEST + 1));
        assert!(rules(&long, &mut macros, false).is_empty());
    }

    #[test]
    fn a_pattern_is_found_as_git_lfs_wrote_it() {
        for (typed, written) in [
            ("*.bin", "*.bin"),
            ("my file.dat", "my[[:space:]]file.dat"),
            ("a#b", "a\\#b"),
            ("back\\slash", "back\\\\slash"),
            (
                " lead and trail ",
                "[[:space:]]lead[[:space:]]and[[:space:]]trail[[:space:]]",
            ),
            ("mixed \\# all", "mixed[[:space:]]\\\\\\#[[:space:]]all"),
        ] {
            assert_eq!(escape(typed), written, "{typed:?}");
            assert_eq!(unescape(written), typed, "{written:?}");
        }
        // What is not an escape stays.
        assert_eq!(unescape("a\\b"), "a\\b");
        assert_eq!(unescape("end\\"), "end\\");

        let file = b"*.bin filter=lfs -text\nmy[[:space:]]file.dat filter=lfs lockable\n*.gone filter=lfs\n*.gone -filter\n";
        assert_eq!(tracked_in(Some(file), "*.bin"), Some(false));
        assert_eq!(tracked_in(Some(file), "my file.dat"), Some(true));
        assert_eq!(tracked_in(Some(file), "my[[:space:]]file.dat"), Some(true));
        assert_eq!(tracked_in(Some(file), "*.gone"), None);
        assert_eq!(tracked_in(Some(file), "*.other"), None);
        assert_eq!(tracked_in(None, "*.bin"), None);
    }

    #[test]
    fn patterns_that_cannot_work_are_not_taken() {
        for good in [
            "*.bin",
            "/top.bin",
            "dir/**",
            "my file.dat",
            "a#b",
            "ä/ö*.x",
            "a..b",
        ] {
            assert!(check_pattern(good).is_ok(), "{good:?}");
        }
        let long = "p".repeat(LONGEST + 1);
        for bad in [
            "",
            " ",
            "\t",
            "-x",
            "--lockable",
            "--",
            "!negative",
            "dir/",
            "/",
            "..",
            "../up.bin",
            "a/../b.bin",
            "a\nb",
            "nul\0",
            "tab\tx",
            long.as_str(),
        ] {
            assert!(check_pattern(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn only_plain_relative_paths_count_as_inside() {
        for good in ["a", "assets/my file.bin", "a/b/c", "ä/ö"] {
            assert!(inside(good), "{good:?}");
        }
        let long = "p/".repeat(3000);
        for bad in [
            "",
            "-x",
            "--force",
            "/abs",
            "..",
            "a/../b",
            "../up",
            "./here",
            "a//b",
            "a/",
            "a\nb",
            "nul\0",
            long.as_str(),
        ] {
            assert!(!inside(bad), "{bad:?}");
            assert!(check_locked_path(bad).is_err(), "{bad:?}");
        }
        // Nothing in the git directory is a file to lock.
        for bad in [".git", ".git/config", ".GIT/hooks/pre-push"] {
            assert!(check_locked_path(bad).is_err(), "{bad:?}");
        }
        for good in ["a", ".gitattributes", "sub/.git", "git/config"] {
            assert!(check_locked_path(good).is_ok(), "{good:?}");
        }
    }

    #[test]
    fn a_pointer_is_told_from_other_small_files() {
        let pointer = concat!(
            "version https://git-lfs.github.com/spec/v1\n",
            "oid sha256:dace82371525b81729329d6d579bc783ef95cd6a25f635cf1efc594862dbe52c\n",
            "size 11\n",
        );
        assert!(is_pointer(pointer.as_bytes()));
        // With more keys, which the format allows, and its older address.
        let extended = pointer.replace("oid ", "ext-0-foo sha256:abc\noid ");
        assert!(is_pointer(extended.as_bytes()));
        assert!(is_pointer(pointer.replace("git-lfs", "hawser").as_bytes()));

        let padded = format!("{pointer}{}", "x".repeat(1024));
        for other in [
            "",
            "version https://git-lfs.github.com/spec/v1\n",
            "just some text\n",
            &pointer.replace("spec/v1", "spec/v2"),
            &pointer.replace("sha256:", "md5:"),
            &pointer.replace("dace", "DAC"),
            &pointer.replace("size 11", "size eleven"),
            &pointer.replace("size 11", "size "),
            &pointer.replacen("version", " version", 1),
            padded.as_str(),
        ] {
            assert!(!is_pointer(other.as_bytes()), "{other:?}");
        }
        assert!(!is_pointer(&[0xff, 0xfe, 0x00]));
    }

    #[test]
    fn the_version_is_read_from_what_git_lfs_prints() {
        assert_eq!(
            parse_version("git-lfs/3.8.0 (GitHub; linux amd64; go 1.27.0)\n"),
            "3.8.0"
        );
        assert_eq!(parse_version("git-lfs/2.13.2\n"), "2.13.2");
        assert_eq!(parse_version("something else\n"), "something");
        assert_eq!(parse_version(""), "");
        assert_eq!(parse_version(&"9".repeat(500)).len(), 40);
    }

    #[test]
    fn listed_files_are_read_from_json() {
        let text = r#"{ "files": [
            { "name": "one.bin", "size": 11, "checkout": true, "downloaded": true,
              "oid_type": "sha256", "oid": "dace", "version": "https://git-lfs.github.com/spec/v1" },
            { "name": "sub/two.bin", "size": 18446744073709551615, "checkout": false, "downloaded": false, "oid": "d5ab" },
            { "size": 3 },
            { "name": 7 },
            { "name": "bare" },
            "not an object"
        ] }"#;
        let (files, more) = parse_files(text).unwrap();
        assert!(!more);
        assert_eq!(
            files,
            [
                LfsFile {
                    path: "one.bin".to_owned(),
                    oid: "dace".to_owned(),
                    size: 11,
                    present: true,
                    checked_out: true,
                },
                LfsFile {
                    path: "sub/two.bin".to_owned(),
                    oid: "d5ab".to_owned(),
                    size: u64::MAX,
                    present: false,
                    checked_out: false,
                },
                LfsFile {
                    path: "bare".to_owned(),
                    oid: String::new(),
                    size: 0,
                    present: false,
                    checked_out: false,
                },
            ]
        );
        // None at all is `null`; a number that is no size is no size.
        for none in [
            r#"{ "files": null }"#,
            "{}",
            "[]",
            "3",
            r#"{ "files": "many" }"#,
        ] {
            assert_eq!(parse_files(none).unwrap(), (Vec::new(), false), "{none}");
        }
        let odd = parse_files(
            r#"{ "files": [{ "name": "n", "size": -4 }, { "name": "f", "size": 1.5 }] }"#,
        )
        .unwrap();
        assert!(odd.0.iter().all(|file| file.size == 0));
        for garbage in ["", "{", "not json", "\u{0}"] {
            assert!(parse_files(garbage).is_err(), "{garbage:?}");
        }
    }

    #[test]
    fn locks_are_read_from_either_form_of_the_list() {
        let lock = |id: &str, path: &str, owner: Option<&str>, ours: Option<bool>| Lock {
            id: id.to_owned(),
            path: path.to_owned(),
            owner: owner.map(str::to_owned),
            locked_at: Some("2026-10-01T10:00:00Z".to_owned()),
            ours,
        };
        let plain = r#"[
            {"id":"7","path":"theirs.bin","owner":{"name":"Bob"},"locked_at":"2026-10-01T10:00:00Z"},
            {"id":"11","path":"d ir/sp ace.bin","locked_at":"2026-10-01T10:00:00Z"},
            {"id":"12"},
            7
        ]"#;
        assert_eq!(
            parse_locks(plain).unwrap(),
            [
                lock("7", "theirs.bin", Some("Bob"), None),
                lock("11", "d ir/sp ace.bin", None, None),
            ]
        );
        let verified = r#"{
            "ours": [{"id":"11","path":"mine.bin","owner":{"name":"Ada"},"locked_at":"2026-10-01T10:00:00Z"}],
            "theirs": [{"id":"7","path":"theirs.bin","owner":{"name":"Bob"},"locked_at":"2026-10-01T10:00:00Z"}]
        }"#;
        assert_eq!(
            parse_locks(verified).unwrap(),
            [
                lock("11", "mine.bin", Some("Ada"), Some(true)),
                lock("7", "theirs.bin", Some("Bob"), Some(false)),
            ]
        );
        for none in [
            "[]",
            "{}",
            r#"{"ours":[],"theirs":[]}"#,
            r#"{"ours":3}"#,
            "null",
        ] {
            assert!(parse_locks(none).unwrap().is_empty(), "{none}");
        }
        for garbage in ["", "[", "Locking failed"] {
            assert!(parse_locks(garbage).is_err(), "{garbage:?}");
        }
        // A lock the server gave no time for: git-lfs prints the first
        // moment of its calendar, which is nobody's "since when".
        let timeless = r#"[{"id":"98","path":"one.bin","locked_at":"0001-01-01T00:00:00Z"}]"#;
        assert_eq!(
            parse_locks(timeless).unwrap(),
            [Lock {
                id: "98".to_owned(),
                path: "one.bin".to_owned(),
                owner: None,
                locked_at: None,
                ours: None,
            }]
        );
    }

    #[test]
    fn a_refusal_passes_on_what_git_lfs_said_in_one_bounded_line() {
        assert_eq!(reason(""), "");
        assert_eq!(reason(" \n\t\n"), "");
        assert_eq!(
            reason("warning: one\n  Locking a.bin failed:\tnot now \n"),
            "warning: one Locking a.bin failed: not now"
        );
        // Bounded, and cut between characters.
        let long = "é".repeat(LONGEST_REASON);
        let cut = reason(&long);
        assert!(cut.len() <= LONGEST_REASON, "{}", cut.len());
        assert!(cut.chars().all(|c| c == 'é'));

        let text = |err: Error| err.to_string();
        // Git-lfs found no lock on the path: the file is not locked.
        assert_eq!(
            text(not_unlocked(
                "a.bin",
                "unable to get lock ID: no matching locks found"
            )),
            "lfs: a.bin is not locked"
        );
        // Any other reason is passed on as it is.
        assert_eq!(
            text(not_unlocked(
                "a.bin",
                "Cannot unlock file with uncommitted changes"
            )),
            "lfs: a.bin was not unlocked: Cannot unlock file with uncommitted changes"
        );

        // Locks that were not listed say why, without the command line;
        // a failure that said nothing, and any other error, stay as they
        // are.
        let failed = |stderr: &str| Error::Command {
            command: "git lfs locks --verify".to_owned(),
            status: "exit status: 2".to_owned(),
            stderr: stderr.to_owned(),
        };
        assert_eq!(
            text(not_listed(failed(
                "Error while retrieving locks: internal server error\n"
            ))),
            "lfs: the locks could not be listed: internal server error"
        );
        assert_eq!(
            text(not_listed(failed("missing protocol: \"\""))),
            "lfs: the locks could not be listed: missing protocol: \"\""
        );
        assert!(matches!(not_listed(failed(" \n")), Error::Command { .. }));
        assert!(matches!(
            not_listed(invalid("operation", "cancelled")),
            Error::Invalid { .. }
        ));

        let held = |owner: Option<&str>, since: Option<&str>| Lock {
            id: "3".to_owned(),
            path: "d ir/a.bin".to_owned(),
            owner: owner.map(str::to_owned),
            locked_at: since.map(str::to_owned),
            ours: None,
        };
        assert_eq!(
            text(already_locked(&held(
                Some("Bob"),
                Some("2026-10-01T10:00:00Z")
            ))),
            "lfs: d ir/a.bin is already locked by Bob since 2026-10-01T10:00:00Z"
        );
        assert_eq!(
            text(already_locked(&held(Some("Bob"), None))),
            "lfs: d ir/a.bin is already locked by Bob"
        );
        assert_eq!(
            text(already_locked(&held(Some(""), Some("")))),
            "lfs: d ir/a.bin is already locked"
        );
    }

    /// What git-lfs 3.8.0 wrote when a server's certificate was not
    /// accepted: said in words before git-lfs's own, for a list, a lock,
    /// an unlock and a transfer.
    #[test]
    fn a_certificate_that_is_not_accepted_is_said_in_words() {
        let text = |err: Error| err.to_string();
        let go = "tls: failed to verify certificate: x509: certificate signed by unknown authority";
        let failed = |stderr: String| Error::Command {
            command: "git lfs locks --verify".to_owned(),
            status: "exit status: 2".to_owned(),
            stderr,
        };
        let listed = text(not_listed(failed(format!(
            "Error while retrieving locks: Post \"https://127.0.0.1:1/g/p.git/info/lfs/locks/verify\": {go}\n"
        ))));
        assert!(
            listed.starts_with(
                "lfs: the locks could not be listed: The server's certificate was not accepted"
            ),
            "{listed}"
        );
        assert!(listed.contains("http.sslCAInfo"), "{listed}");
        assert!(
            listed.ends_with(&format!(
                "Git-lfs said: Post \"https://127.0.0.1:1/g/p.git/info/lfs/locks/verify\": {go}"
            )),
            "{listed}"
        );
        let unlocked = text(not_unlocked(
            "a.bin",
            &format!("unable to get lock ID: Get \"https://127.0.0.1:1/x\": {go}"),
        ));
        assert!(
            unlocked.starts_with("lfs: a.bin was not unlocked: The server's certificate"),
            "{unlocked}"
        );
        assert!(
            with_meaning("a.bin was not locked", go)
                .starts_with("a.bin was not locked: The server's certificate"),
        );
        // Another reason is passed on as it is.
        assert_eq!(
            with_meaning(
                "a.bin was not locked",
                "Access forbidden. Check your access level."
            ),
            "a.bin was not locked: Access forbidden. Check your access level."
        );
        // github.com's 403 to a new lock has no words, and git-lfs's own
        // for it come after ours.
        let github = "Authorization error: https://u@github.com/o/r.git/info/lfs/locks \
                      Check that you have proper access to the repository";
        assert_eq!(
            with_meaning("a.bin was not locked", github),
            format!(
                "a.bin was not locked: {} Git-lfs said: {github}",
                remote::NOT_ALLOWED
            )
        );
        // A transfer: the words first, then all git-lfs said but its meter.
        let moved = text(remote::explained(Error::Command {
            command: "git lfs fetch -- origin".to_owned(),
            status: "exit status: 2".to_owned(),
            stderr: format!(
                "Downloading LFS objects:  50% (1/2), 800 KB | 183 KB/s\n\
                 batch response: Post \"https://127.0.0.1:1/g/p.git/info/lfs/objects/batch\": {go}\n\
                 error: failed to fetch some objects from 'https://127.0.0.1:1/g/p.git/info/lfs'"
            ),
        }));
        assert!(
            moved.contains("The server's certificate was not accepted"),
            "{moved}"
        );
        assert!(moved.contains(go), "{moved}");
        assert!(!moved.contains("Downloading LFS objects"), "{moved}");
    }

    /// Whatever git-lfs writes, telling what it means stays within bounds
    /// and never fails.
    #[test]
    fn hostile_words_of_git_lfs_are_read_without_harm() {
        let mut hostile: Vec<String> = vec![
            String::new(),
            "\0\u{1b}[31m\r\n".to_owned(),
            "x509:".to_owned(),
            "SSL certificate problem".repeat(10_000),
            "é".repeat(LONGEST_REASON * 3),
            "Error while retrieving locks: ".to_owned(),
            format!("{}tls: failed to verify certificate", "\n".repeat(10_000)),
        ];
        hostile.push((0u32..2000).filter_map(char::from_u32).collect::<String>());
        for said in &hostile {
            let bounded = reason(said);
            assert!(bounded.len() <= LONGEST_REASON);
            let _ = with_meaning("a.bin was not locked", &bounded);
            let _ = not_listed(Error::Command {
                command: "git lfs locks".to_owned(),
                status: "exit status: 2".to_owned(),
                stderr: said.clone(),
            })
            .to_string();
            let _ = not_unlocked("a.bin", &bounded).to_string();
            let _ = remote::explained(Error::Command {
                command: "git lfs push".to_owned(),
                status: "exit status: 2".to_owned(),
                stderr: said.clone(),
            })
            .to_string();
        }
    }
}
