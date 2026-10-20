//! The record of the git processes the application starts: what was run,
//! where, when, for how long, how it ended and what it wrote. The activity
//! log shows it, and the output of a hook is found here, in the entry of
//! the command that ran the hook.
//!
//! The record is a ring of the last [`CAPACITY`] entries, one for the
//! process and shared by all its threads. [`crate::exe`] writes an entry
//! where a git process ends. Recording never fails the call it records:
//! with a poisoned lock the entry is dropped.
//!
//! An entry is a question when git was only asked something (see
//! [`asks`]), so that a list can leave out what the application runs all
//! the time. The engine says which of its commands a thread is running
//! ([`under`]), and the entries made meanwhile carry that.
//!
//! An entry belongs to a repository when git ran in its working directory
//! or its git directory, and also when the engine was working for that
//! repository at the time ([`during`]), wherever git ran: inside a
//! submodule, in another worktree, in no repository at all. A clone and an
//! init are about the repository they make ([`about`]), and belong to it
//! once it is there.
//!
//! The ring is the running process's. What outlives it is kept by
//! [`store`], in a file per repository in a directory the application
//! names ([`during`] again): entries that did something, written by a
//! thread of their own after the process they describe has ended.
//! [`history`] is the two together.
//!
//! Sources: git(1) (the options that come before a command); gitcli(7)
//! (`--`, shortened long options); for telling a question from a change,
//! git-config(1), git-symbolic-ref(1), git-stash(1), git-remote(1),
//! git-tag(1), git-branch(1), git-diff(1) (`--output`), git-grep(1),
//! git-apply(1), git-merge-file(1), git-merge-tree(1), git-worktree(1),
//! git-submodule(1), git-rerere(1), git-lfs(1); RFC 3986 section 3.2.1
//! (user information in a URL).

use std::{
    borrow::Cow,
    cell::{Cell, RefCell},
    collections::{BTreeMap, VecDeque},
    ffi::OsStr,
    marker::PhantomData,
    path::{Path, PathBuf},
    process::{Command, ExitStatus},
    sync::{Arc, Mutex},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

use crate::Repository;

pub mod store;

pub use store::flush;

/// How many entries are kept; the oldest goes when one more comes.
pub const CAPACITY: usize = 2000;

/// How much of what a process wrote is kept, per stream, in bytes.
pub const OUTPUT_LIMIT: usize = 16 * 1024;

/// How much of a command line is kept. One with a thousand paths on it is
/// not read to its end.
const COMMAND_LIMIT: usize = 4 * 1024;

/// How a process ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum End {
    /// It ran to its end; the entry has its exit code.
    Exited,
    /// A signal ended it, and not on our request.
    Killed,
    /// It was stopped because the operation was cancelled.
    Cancelled,
    /// It could not be started; the entry's standard error says why.
    NotStarted,
    /// It was started and how it ended is not known: the pipe to it broke,
    /// or waiting for it failed.
    Lost,
    /// A tool that was started and left to the user; it has not ended yet.
    Running,
}

/// One git process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// A running number: higher is later, also from one run of the
    /// application to the next (see [`first_id`]).
    pub id: u64,
    /// The directory git ran in.
    #[serde(serialize_with = "lossy")]
    pub directory: PathBuf,
    /// The git directory of the repository it was run for, when the
    /// engine said so: the one that was open, or the one a clone or an
    /// init was making.
    #[serde(default, serialize_with = "lossy_or_none")]
    pub repository: Option<PathBuf>,
    /// `git` and its arguments, without environment and credentials.
    pub command: String,
    /// When it was started, in milliseconds since the unix epoch.
    pub started: u64,
    /// How long it took, in milliseconds. For a tool that is still
    /// running, how long it was watched.
    pub duration_ms: u64,
    /// How it ended.
    pub end: End,
    /// Its exit code, when it ran to its end.
    pub code: Option<i32>,
    /// The end of what it wrote to standard output.
    pub stdout: String,
    /// Whether the beginning of `stdout` was cut off.
    pub stdout_cut: bool,
    /// The end of what it wrote to standard error: git's own messages,
    /// progress, and whatever a hook printed.
    pub stderr: String,
    /// Whether the beginning of `stderr` was cut off.
    pub stderr_cut: bool,
    /// Git was only asked something (see [`asks`]).
    pub question: bool,
    /// The id of the engine command it ran under, when there was one.
    #[serde(default)]
    pub origin: Option<Cow<'static, str>>,
}

/// A path as text, also when its name is not UTF-8, which serde would
/// refuse to write.
pub(crate) fn lossy<S: serde::Serializer>(path: &Path, to: S) -> Result<S::Ok, S::Error> {
    to.serialize_str(&path.to_string_lossy())
}

/// [`lossy`] for a path that may be absent.
fn lossy_or_none<S: serde::Serializer>(path: &Option<PathBuf>, to: S) -> Result<S::Ok, S::Error> {
    match path {
        Some(path) => to.serialize_some(&path.to_string_lossy()),
        None => to.serialize_none(),
    }
}

/// The number before the first entry of this process: the time it is
/// made at, in microseconds since the unix epoch. Entries of an earlier
/// run are lower and those of a later one higher, so that an entry keeps
/// its number in the file that outlives the process and a list of both is
/// in order by number. The command line is run many times a second, which
/// is why it is not the second. (Two processes at once whose numbers meet,
/// because one made more entries than the other started microseconds
/// later, are told apart by when an entry was started; a clock set back
/// makes an order of numbers that is not the order in time. Nothing worse
/// comes of either.)
fn first_id() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| u64::try_from(since.as_micros()).unwrap_or(u64::MAX / 2))
        .unwrap_or(0)
}

#[derive(Debug)]
struct Ring {
    last_id: u64,
    entries: VecDeque<Arc<Entry>>,
}

impl Ring {
    const fn new() -> Self {
        Self {
            last_id: 0,
            entries: VecDeque::new(),
        }
    }

    /// Add an entry under the next number, and give it back as it is
    /// kept.
    fn push(&mut self, mut entry: Entry) -> Arc<Entry> {
        self.last_id = self.last_id.wrapping_add(1);
        entry.id = self.last_id;
        while self.entries.len() >= CAPACITY {
            self.entries.pop_front();
        }
        let entry = Arc::new(entry);
        self.entries.push_back(entry.clone());
        entry
    }

    /// Put `entry` in the place of the one numbered `id`, and give it
    /// back as it is kept. One that has left the ring meanwhile is still
    /// the entry numbered `id`, only no longer in the ring.
    fn replace(&mut self, id: u64, mut entry: Entry) -> Arc<Entry> {
        entry.id = id;
        let entry = Arc::new(entry);
        // Newest first: what is replaced was added a moment or a tool's
        // lifetime ago.
        if let Some(slot) = self.entries.iter_mut().rev().find(|slot| slot.id == id) {
            *slot = entry.clone();
        }
        entry
    }
}

static RING: Mutex<Ring> = Mutex::new(Ring::new());

/// The process's ring, numbering from [`first_id`]. None with a poisoned
/// lock: the entry is then dropped, and the git call it describes goes
/// on.
fn ring() -> Option<std::sync::MutexGuard<'static, Ring>> {
    let mut ring = RING.lock().ok()?;
    if ring.last_id == 0 {
        ring.last_id = first_id();
    }
    Some(ring)
}

/// Everything in the record, oldest first.
pub fn snapshot() -> Vec<Arc<Entry>> {
    RING.lock()
        .map(|ring| ring.entries.iter().cloned().collect())
        .unwrap_or_default()
}

/// The entries of this repository in the running process, oldest first:
/// those of git processes that ran in its working directory or its git
/// directory, or that the engine ran for it elsewhere.
pub fn of_repository(repo: &Repository) -> Vec<Arc<Entry>> {
    let mut places = Places::of(repo);
    snapshot()
        .into_iter()
        .filter(|entry| places.owns(entry))
        .collect()
}

/// The entries of this repository, oldest first: what is kept in
/// `kept_in` from earlier runs and from this one, and what the running
/// process has that is not kept (questions, a tool that is still open,
/// what the writing thread has not come to yet).
pub fn history(repo: &Repository, kept_in: Option<&Path>) -> Vec<Arc<Entry>> {
    let live = of_repository(repo);
    let Some(dir) = kept_in else {
        return live;
    };
    let kept = store::kept(&store::file_for(dir, &home_of(repo)));
    if kept.is_empty() {
        return live;
    }
    // An entry that is both kept and here is listed once: as it is here,
    // which is the same. It is the same entry when number and start are.
    let here: std::collections::BTreeSet<(u64, u64)> =
        live.iter().map(|entry| (entry.id, entry.started)).collect();
    let mut all: Vec<Arc<Entry>> = kept
        .iter()
        .filter(|entry| !here.contains(&(entry.id, entry.started)))
        .cloned()
        .collect();
    all.extend(live);
    all.sort_by_key(|entry| (entry.id, entry.started));
    all
}

/// Forget the entries of this repository in the running process; those
/// of others stay. Returns how many went.
pub fn forget_repository(repo: &Repository) -> usize {
    let mut places = Places::of(repo);
    // Which entries are this repository's is found out before the lock
    // is taken: it asks the file system.
    let ids: std::collections::BTreeSet<u64> = snapshot()
        .iter()
        .filter(|entry| places.owns(entry))
        .map(|entry| entry.id)
        .collect();
    let Ok(mut ring) = RING.lock() else { return 0 };
    let before = ring.entries.len();
    ring.entries.retain(|entry| !ids.contains(&entry.id));
    before.saturating_sub(ring.entries.len())
}

/// Forget the entries of this repository, those kept in `kept_in` too.
/// Returns how many a list had before.
pub fn forget(repo: &Repository, kept_in: Option<&Path>) -> usize {
    let listed = history(repo, kept_in).len();
    forget_repository(repo);
    if let Some(dir) = kept_in {
        // After what is on its way to the file, and done before a list
        // is asked for again.
        store::clear(store::file_for(dir, &home_of(repo)));
        store::flush(std::time::Duration::from_secs(5));
    }
    listed
}

/// What a repository is known by in the record: its git directory, with
/// links on the way resolved, so that two spellings are one repository.
fn home_of(repo: &Repository) -> PathBuf {
    let git_dir = absolute(repo.git_dir());
    bitbraid_core::paths::canonical(&git_dir).unwrap_or(git_dir)
}

/// The git directory a repository at `path` has, or will have once it is
/// made: `path` may not be there yet, so what of it exists is resolved
/// and the rest is taken as it is written.
fn home_at(path: &Path) -> PathBuf {
    let path = absolute(path);
    let existing = path
        .ancestors()
        .find(|part| part.exists())
        .unwrap_or(&path)
        .to_path_buf();
    let rest = path.strip_prefix(&existing).unwrap_or(Path::new(""));
    bitbraid_core::paths::canonical(&existing)
        .unwrap_or(existing)
        .join(rest)
        .join(".git")
}

/// The directories git is run in for one repository, and which of the
/// directories seen so far are the same ones under another spelling.
struct Places {
    given: Vec<PathBuf>,
    real: Vec<PathBuf>,
    seen: BTreeMap<PathBuf, bool>,
    /// What an entry that was run for this repository names.
    home: PathBuf,
}

impl Places {
    fn of(repo: &Repository) -> Self {
        let given: Vec<PathBuf> = repo
            .workdir()
            .into_iter()
            .chain([repo.git_dir()])
            .map(absolute)
            .collect();
        let real = given
            .iter()
            .filter_map(|path| bitbraid_core::paths::canonical(path).ok())
            .collect();
        Self {
            given,
            real,
            seen: BTreeMap::new(),
            home: home_of(repo),
        }
    }

    /// Whether an entry is this repository's: git ran in one of its
    /// directories, or was run for it.
    fn owns(&mut self, entry: &Entry) -> bool {
        entry.repository.as_deref() == Some(self.home.as_path()) || self.has(&entry.directory)
    }

    fn has(&mut self, directory: &Path) -> bool {
        if self.given.iter().any(|path| path == directory) {
            return true;
        }
        if let Some(known) = self.seen.get(directory) {
            return *known;
        }
        // A symbolic link on the way, or a relative path made absolute.
        let same =
            bitbraid_core::paths::canonical(directory).is_ok_and(|real| self.real.contains(&real));
        self.seen.insert(directory.to_path_buf(), same);
        same
    }
}

fn absolute(path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
    }
}

thread_local! {
    /// The engine command this thread is running.
    static UNDER: Cell<Option<&'static str>> = const { Cell::new(None) };
}

/// Names the engine command a thread is running, until it is dropped.
#[derive(Debug)]
#[must_use = "the command is named for as long as this is kept"]
pub struct Under {
    previous: Option<&'static str>,
    // Dropped on the thread it was made on, or it would rename another's.
    here: PhantomData<*const ()>,
}

/// Say that this thread runs the engine command `command` from now until
/// what is returned is dropped; the name there was before is then put
/// back, so a command that runs another gets its own name back after it.
pub fn under(command: &'static str) -> Under {
    let previous = UNDER
        .try_with(|cell| cell.replace(Some(command)))
        .ok()
        .flatten();
    Under {
        previous,
        here: PhantomData,
    }
}

impl Drop for Under {
    fn drop(&mut self) {
        let _ = UNDER.try_with(|cell| cell.set(self.previous));
    }
}

/// What a thread's entries are made for.
#[derive(Debug, Clone, Default)]
struct Scope {
    /// The git directory of the repository git is run for.
    repository: Option<Arc<Path>>,
    /// The directory entries are kept in between runs.
    keep: Option<Arc<Path>>,
}

/// An entry on its way to a file, and the file.
type Kept = (PathBuf, Arc<Entry>);

thread_local! {
    /// What this thread's entries are made for.
    static SCOPE: RefCell<Scope> = RefCell::new(Scope::default());
    /// Entries that are held back from their file until it is known that
    /// what they are about came to be (see [`about`]).
    static HELD: RefCell<Option<Vec<Kept>>> = const { RefCell::new(None) };
}

/// Names the engine command a thread is running and the repository it
/// runs it for, until it is dropped.
#[derive(Debug)]
#[must_use = "the command is named for as long as this is kept"]
pub struct During {
    _named: Under,
    previous: Scope,
}

/// Say that this thread runs the engine command `command` from now until
/// what is returned is dropped, for `repository` when one is open, and
/// that what it does is to be kept in the directory `keep` when the
/// application has one for it. What was said before is then put back.
///
/// Entries made meanwhile belong to that repository wherever git runs,
/// and those that did something are written to its file in `keep`.
pub fn during(
    command: &'static str,
    repository: Option<&Repository>,
    keep: Option<&Path>,
) -> During {
    let scope = Scope {
        repository: repository.map(|repo| Arc::from(home_of(repo))),
        keep: keep.map(Arc::from),
    };
    let previous = SCOPE
        .try_with(|cell| cell.replace(scope))
        .unwrap_or_default();
    During {
        _named: under(command),
        previous,
    }
}

impl Drop for During {
    fn drop(&mut self) {
        let previous = std::mem::take(&mut self.previous);
        let _ = SCOPE.try_with(|cell| cell.replace(previous));
    }
}

/// Says which repository a thread's entries are about, until it is
/// dropped: the one a clone or an init is making.
#[derive(Debug)]
#[must_use = "the entries are about the repository for as long as this is kept"]
pub struct About {
    previous: Option<Arc<Path>>,
    outer: Option<Vec<Kept>>,
    // Dropped on the thread it was made on.
    here: PhantomData<*const ()>,
}

/// Say that what this thread runs from now until what is returned is
/// dropped is about the repository at `path`, which may not be there yet.
/// The entries made meanwhile belong to that repository, and they are
/// written to its file only once [`About::came_to_be`] says that there is
/// such a repository: a clone that failed leaves no file for a repository
/// that never was.
pub fn about(path: &Path) -> About {
    let home: Arc<Path> = Arc::from(home_at(path));
    let previous = SCOPE
        .try_with(|cell| cell.borrow_mut().repository.replace(home))
        .ok()
        .flatten();
    let outer = HELD
        .try_with(|cell| cell.replace(Some(Vec::new())))
        .ok()
        .flatten();
    About {
        previous,
        outer,
        here: PhantomData,
    }
}

impl About {
    /// The repository is there: what was run to make it is kept.
    pub fn came_to_be(self) {
        let held = HELD
            .try_with(|cell| cell.borrow_mut().as_mut().map(std::mem::take))
            .ok()
            .flatten()
            .unwrap_or_default();
        for (file, entry) in held {
            store::keep(file, entry);
        }
    }
}

impl Drop for About {
    fn drop(&mut self) {
        let previous = self.previous.take();
        let _ = SCOPE.try_with(|cell| cell.borrow_mut().repository = previous);
        let outer = self.outer.take();
        let _ = HELD.try_with(|cell| cell.replace(outer));
    }
}

/// Hand an entry over to be written to `file`, or hold it back while a
/// repository is still being made.
fn keep(file: &Path, entry: Arc<Entry>) {
    let held = HELD
        .try_with(|cell| match cell.borrow_mut().as_mut() {
            Some(held) => {
                held.push((file.to_path_buf(), entry.clone()));
                true
            }
            None => false,
        })
        .unwrap_or(false);
    if !held {
        store::keep(file.to_path_buf(), entry);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing recorded yet.
    Open,
    /// Recorded as running under this number.
    Running(u64),
    /// Recorded for good.
    Done,
}

/// A git process on its way into the record: made right before the
/// process is started, told how it ended. When it is dropped without
/// having been told (an error on the way out of the function that ran
/// git) the entry says that the process was lost.
#[derive(Debug)]
pub(crate) struct Started {
    directory: PathBuf,
    /// The git directory of the repository it is run for.
    repository: Option<PathBuf>,
    /// The file the entry is kept in once the process has ended.
    keep: Option<PathBuf>,
    command: String,
    question: bool,
    origin: Option<&'static str>,
    started: u64,
    clock: Instant,
    state: Cell<State>,
}

/// How long a process took, in milliseconds, as the log has it. Under a
/// clock that is held still (`bitbraid_core::clock`) no time passes: an
/// entry begins and ends at the moment the clock is held at, so that what
/// is drawn of the log is the same on every run and every machine.
fn took_ms(elapsed: std::time::Duration, held: bool) -> u64 {
    if held {
        0
    } else {
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }
}

/// Note a git process that is about to be started. `rendered` is its
/// command line as [`crate::exe`] renders it for error messages.
pub(crate) fn begin(cmd: &Command, rendered: &str) -> Started {
    let lossy: Vec<Cow<'_, str>> = cmd.get_args().map(OsStr::to_string_lossy).collect();
    let args: Vec<&str> = lossy.iter().map(AsRef::as_ref).collect();
    let directory = match cmd.get_current_dir() {
        Some(dir) => absolute(dir),
        None => std::env::current_dir().unwrap_or_default(),
    };
    // By the clock everything shown with an age goes by, when that is
    // held still; to the millisecond otherwise.
    let started = match bitbraid_core::clock::held() {
        Some(held) => u64::try_from(held).unwrap_or(0).saturating_mul(1000),
        None => SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|since| u64::try_from(since.as_millis()).unwrap_or(u64::MAX))
            .unwrap_or(0),
    };
    let scope = SCOPE
        .try_with(|cell| cell.borrow().clone())
        .unwrap_or_default();
    let question = asks(&args);
    // What git was only asked is not kept between runs: there are
    // hundreds of such entries for every one that did something.
    let keep = match (&scope.keep, &scope.repository) {
        (Some(dir), Some(home)) if !question => Some(store::file_for(dir, home)),
        _ => None,
    };
    Started {
        directory,
        repository: scope.repository.as_deref().map(Path::to_path_buf),
        keep,
        command: head(&scrub(rendered, false), COMMAND_LIMIT),
        question,
        origin: UNDER.try_with(Cell::get).ok().flatten(),
        started,
        clock: Instant::now(),
        state: Cell::new(State::Open),
    }
}

impl Started {
    /// The process ended by itself or by a signal.
    pub(crate) fn exited(&self, status: ExitStatus, stdout: &str, stderr: &str) {
        let end = if status.code().is_some() {
            End::Exited
        } else {
            End::Killed
        };
        self.put(end, status.code(), stdout, stderr);
    }

    /// The process was stopped on request.
    pub(crate) fn cancelled(&self, stdout: &str, stderr: &str) {
        self.put(End::Cancelled, None, stdout, stderr);
    }

    /// The process could not be started.
    pub(crate) fn not_started(&self, why: &std::io::Error) {
        self.put(End::NotStarted, None, "", &why.to_string());
    }

    /// The process is left running; [`Started::exited`] completes the
    /// entry when it is collected.
    pub(crate) fn running(&self) {
        self.put(End::Running, None, "", "");
    }

    fn put(&self, end: End, code: Option<i32>, stdout: &str, stderr: &str) {
        let state = self.state.get();
        if state == State::Done {
            return;
        }
        let took = took_ms(self.clock.elapsed(), bitbraid_core::clock::held().is_some());
        let (stdout, stdout_cut) = tail(stdout);
        let (stderr, stderr_cut) = tail(stderr);
        let entry = Entry {
            id: 0,
            directory: self.directory.clone(),
            repository: self.repository.clone(),
            command: self.command.clone(),
            started: self.started,
            duration_ms: took,
            end,
            code,
            stdout,
            stdout_cut,
            stderr,
            stderr_cut,
            question: self.question,
            origin: self.origin.map(Cow::Borrowed),
        };
        // A poisoned lock: the entry is dropped, the git call goes on.
        let recorded = ring().map(|mut ring| match state {
            State::Running(id) => ring.replace(id, entry),
            _ => ring.push(entry),
        });
        let mut next = State::Done;
        if let Some(entry) = recorded {
            if end == End::Running {
                next = State::Running(entry.id);
            } else if let Some(file) = &self.keep {
                // It has ended: this is the entry for good, and it goes
                // to the file from another thread.
                keep(file, entry);
            }
        }
        self.state.set(next);
    }
}

impl Drop for Started {
    fn drop(&mut self) {
        self.put(End::Lost, None, "", "");
    }
}

/// The end of what a process wrote: at most [`OUTPUT_LIMIT`] bytes, cut
/// at a character, without the credentials in its URLs and without the
/// white space after its last line. Says whether the beginning was cut
/// off.
fn tail(text: &str) -> (String, bool) {
    let text = text.trim_end();
    if text.len() <= OUTPUT_LIMIT {
        return (scrub(text, false), false);
    }
    let mut start = text.len().saturating_sub(OUTPUT_LIMIT);
    while !text.is_char_boundary(start) {
        start = start.saturating_add(1);
    }
    (scrub(text.get(start..).unwrap_or(""), true), true)
}

/// The beginning of `text`, at most `limit` bytes of it, marked when
/// something was cut off.
fn head(text: &str, limit: usize) -> String {
    if text.len() <= limit {
        return text.to_owned();
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end = end.saturating_sub(1);
    }
    let mut kept = text.get(..end).unwrap_or("").to_owned();
    kept.push('…');
    kept
}

/// `text` without the credentials in its URLs: of `scheme://user:secret@host`
/// the part before the `@` is left out, wherever such a URL stands. A text
/// whose beginning was `cut` off may begin inside one, with the scheme
/// gone; its first word is then treated as if it did.
fn scrub(text: &str, cut: bool) -> String {
    if !cut && scheme_end(text).is_none() {
        return text.to_owned();
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    let mut in_authority = cut;
    loop {
        if in_authority {
            // User information ends at the last `@` before the path, or
            // before the word ends.
            let end = rest
                .bytes()
                .position(|byte| byte == b'/' || byte <= b' ' || byte == 0x7f)
                .unwrap_or(rest.len());
            let authority = rest.get(..end).unwrap_or("");
            match authority.rfind('@') {
                Some(at) => {
                    out.push_str("***");
                    out.push_str(authority.get(at..).unwrap_or(""));
                }
                None => out.push_str(authority),
            }
            rest = rest.get(end..).unwrap_or("");
        }
        let Some(after) = scheme_end(rest) else {
            out.push_str(rest);
            return out;
        };
        out.push_str(rest.get(..after).unwrap_or(""));
        rest = rest.get(after..).unwrap_or("");
        in_authority = true;
    }
}

/// Where the first `://` in `text` ends. The colon is looked for first: a
/// single byte is found many times faster than three, and most of what
/// git writes has few.
fn scheme_end(text: &str) -> Option<usize> {
    let mut from = 0usize;
    loop {
        let colon = from.saturating_add(text.get(from..)?.find(':')?);
        if text.get(colon..)?.starts_with("://") {
            return Some(colon.saturating_add(3));
        }
        from = colon.saturating_add(1);
    }
}

/// Whether `arg` is the option `name`, alone or with a value after `=`.
fn is(arg: &str, name: &str) -> bool {
    arg.strip_prefix(name)
        .is_some_and(|value| value.is_empty() || value.starts_with('='))
}

/// Whether `arg` could be the option `name` in another spelling: a long
/// option shortened, which git accepts while it is unambiguous, or a
/// short one among others in one word or with its value attached.
fn could_be(arg: &str, name: &str) -> bool {
    if is(arg, name) {
        return true;
    }
    if name.starts_with("--") {
        let spelled = arg.split('=').next().unwrap_or(arg);
        spelled.len() > 2 && spelled.starts_with("--") && name.starts_with(spelled)
    } else {
        let letter = name.chars().nth(1);
        arg.starts_with('-')
            && !arg.starts_with("--")
            && letter.is_some_and(|letter| arg.contains(letter))
    }
}

/// Whether a git command line only asks: `args` are the arguments after
/// `git`. A question changes no reference, no entry of the index, no file
/// of the user's and no configuration, and it talks to no remote. Objects
/// that nothing refers to do not count (`merge-tree` writes the tree it
/// compares with), and neither does git refreshing what it remembers
/// about files (`status`).
///
/// This is told from the command and its options alone, and carefully:
/// what is not known to be a question is not one.
pub fn asks(args: &[&str]) -> bool {
    // What comes before the command.
    let mut rest = args;
    let command = loop {
        let Some((first, more)) = rest.split_first() else {
            return false;
        };
        rest = more;
        match *first {
            "--version" | "-v" | "--exec-path" => return true,
            "--literal-pathspecs"
            | "--glob-pathspecs"
            | "--noglob-pathspecs"
            | "--icase-pathspecs"
            | "--no-pager"
            | "-P"
            | "--no-optional-locks"
            | "--no-replace-objects"
            | "--no-lazy-fetch"
            | "--no-advice" => {}
            // These take a value: a setting for this run, a directory.
            "-c" | "-C" => rest = rest.get(1..).unwrap_or(&[]),
            option if option.starts_with('-') => return false,
            command => break command,
        }
    };
    // Options stand before `--`; after it there are only paths.
    let end = rest
        .iter()
        .position(|arg| *arg == "--")
        .unwrap_or(rest.len());
    let before = rest.get(..end).unwrap_or(rest);
    let after = rest.get(end.saturating_add(1)..).unwrap_or(&[]);
    let first = before.first().copied();
    let has = |names: &[&str]| {
        before
            .iter()
            .any(|arg| names.iter().any(|name| is(arg, name)))
    };
    let could = |names: &[&str]| {
        before
            .iter()
            .any(|arg| names.iter().any(|name| could_be(arg, name)))
    };
    match command {
        "rev-parse" | "rev-list" | "merge-base" | "check-ref-format" | "check-ignore"
        | "check-attr" | "check-mailmap" | "for-each-ref" | "show-ref" | "ls-files" | "ls-tree"
        | "cat-file" | "merge-tree" | "write-tree" | "hash-object" | "status" | "describe"
        | "name-rev" | "blame" | "count-objects" | "var" | "version" | "verify-commit"
        | "verify-tag" => true,
        // These can write what they find to a file.
        "diff" | "diff-tree" | "diff-index" | "diff-files" | "log" | "show" => {
            !could(&["--output"])
        }
        // This one can open what it finds in a program.
        "grep" => !could(&["-O", "--open-files-in-pager"]),
        // With one name it reads; with two, or to delete, it writes.
        "symbolic-ref" => {
            let names = before.iter().filter(|arg| !arg.starts_with('-')).count();
            names.saturating_add(after.len()) == 1 && !could(&["-d", "--delete", "-m"])
        }
        "config" => {
            (matches!(first, Some("get" | "list"))
                || has(&[
                    "--get",
                    "--get-all",
                    "--get-regexp",
                    "--get-urlmatch",
                    "--get-color",
                    "--get-colorbool",
                    "--list",
                    "-l",
                ]))
                && !could(&[
                    "--add",
                    "--replace-all",
                    "--unset",
                    "--unset-all",
                    "--rename-section",
                    "--remove-section",
                    "--edit",
                    "-e",
                ])
        }
        "tag" => {
            rest.is_empty()
                || (has(&["-l", "--list", "-v", "--verify"])
                    && !could(&[
                        "-d",
                        "--delete",
                        "-a",
                        "--annotate",
                        "-s",
                        "--sign",
                        "-u",
                        "--local-user",
                        "-f",
                        "--force",
                        "-m",
                        "--message",
                        "-F",
                        "--file",
                        "-e",
                        "--edit",
                    ]))
        }
        "branch" => {
            has(&["--show-current", "--list", "-l"])
                && !could(&[
                    "-d",
                    "-D",
                    "--delete",
                    "-m",
                    "-M",
                    "--move",
                    "-c",
                    "-C",
                    "--copy",
                    "-f",
                    "--force",
                    "-u",
                    "--set-upstream-to",
                    "--unset-upstream",
                    "-t",
                    "--track",
                    "--no-track",
                    "--edit-description",
                    "--create-reflog",
                    "--recurse-submodules",
                ])
        }
        "remote" => {
            first == Some("get-url")
                || (after.is_empty() && before.iter().all(|arg| matches!(*arg, "-v" | "--verbose")))
        }
        "stash" => matches!(first, Some("list" | "show")),
        "worktree" => first == Some("list"),
        "submodule" => rest.is_empty() || matches!(first, Some("status" | "summary")),
        "rerere" => matches!(first, Some("status" | "diff" | "remaining")),
        "lfs" => matches!(first, Some("ls-files" | "env" | "version" | "status")),
        "apply" => has(&["--check"]) && !could(&["--apply"]),
        "merge-file" => has(&["-p", "--stdout"]),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_time_passes_under_a_clock_that_is_held() {
        let took = std::time::Duration::from_millis(1540);
        assert_eq!(took_ms(took, false), 1540);
        assert_eq!(took_ms(took, true), 0);
        assert_eq!(took_ms(std::time::Duration::ZERO, false), 0);
        // Longer than a count of milliseconds holds: the most there is.
        assert_eq!(took_ms(std::time::Duration::MAX, false), u64::MAX);
    }

    fn entry(directory: &str, command: &str) -> Entry {
        Entry {
            id: 0,
            directory: PathBuf::from(directory),
            repository: None,
            command: command.to_owned(),
            started: 0,
            duration_ms: 0,
            end: End::Exited,
            code: Some(0),
            stdout: String::new(),
            stdout_cut: false,
            stderr: String::new(),
            stderr_cut: false,
            question: false,
            origin: None,
        }
    }

    #[test]
    fn the_ring_keeps_the_newest_entries() {
        let mut ring = Ring::new();
        let extra = 500;
        for n in 0..CAPACITY + extra {
            assert_eq!(
                ring.push(entry("/somewhere", &format!("git {n}"))).id,
                n as u64 + 1
            );
        }
        assert_eq!(ring.entries.len(), CAPACITY);
        let ids: Vec<u64> = ring.entries.iter().map(|entry| entry.id).collect();
        let expected: Vec<u64> = (extra as u64 + 1..=(CAPACITY + extra) as u64).collect();
        assert_eq!(ids, expected, "oldest first, the first {extra} gone");
        assert_eq!(ring.entries[0].command, format!("git {extra}"));
        assert_eq!(
            ring.entries[CAPACITY - 1].command,
            format!("git {}", CAPACITY + extra - 1)
        );

        // An entry is completed in place, under its number; one that is
        // gone is not brought back.
        let last = ring.last_id;
        assert_eq!(ring.replace(last, entry("/somewhere", "git done")).id, last);
        assert_eq!(ring.replace(1, entry("/somewhere", "git gone")).id, 1);
        assert_eq!(ring.entries.len(), CAPACITY);
        assert_eq!(ring.entries[CAPACITY - 1].command, "git done");
        assert_eq!(ring.entries[CAPACITY - 1].id, last);
        assert!(ring.entries.iter().all(|entry| entry.command != "git gone"));
    }

    #[test]
    fn credentials_in_urls_are_taken_out_of_text() {
        for (given, shown) in [
            ("", ""),
            ("nothing to see", "nothing to see"),
            (
                "fatal: unable to access 'https://ada:hunter2@example.invalid/a.git/': no",
                "fatal: unable to access 'https://***@example.invalid/a.git/': no",
            ),
            // Several in one text, and one at its very end.
            (
                "from ssh://git:s3cret@one.invalid:22/a to https://token@two.invalid",
                "from ssh://***@one.invalid:22/a to https://***@two.invalid",
            ),
            // Two in one word, as in a setting that rewrites URLs.
            (
                "url.https://u:p@a.invalid/.insteadOf=https://v:q@b.invalid/",
                "url.https://***@a.invalid/.insteadOf=https://***@b.invalid/",
            ),
            // A password with an `@` in it goes as a whole.
            ("https://u:p@ss@h.invalid/a", "https://***@h.invalid/a"),
            // No user information: an `@` in the path, an address later on.
            (
                "https://example.invalid/a@b.git by ada@fixture.invalid",
                "https://example.invalid/a@b.git by ada@fixture.invalid",
            ),
            (
                "https://example.invalid ada@fixture.invalid",
                "https://example.invalid ada@fixture.invalid",
            ),
            (
                "https://example.invalid\nada@fixture.invalid",
                "https://example.invalid\nada@fixture.invalid",
            ),
            ("git@example.invalid:a/b.git", "git@example.invalid:a/b.git"),
            ("://", "://"),
            ("://://", "://://"),
            ("x://@", "x://***@"),
            ("é://ü:ß@ö/", "é://***@ö/"),
        ] {
            assert_eq!(scrub(given, false), shown, "{given}");
        }
        // A text that was cut may begin inside a URL: what could be the end
        // of a secret goes too.
        assert_eq!(scrub("cret@h.invalid/a b", true), "***@h.invalid/a b");
        assert_eq!(scrub("h.invalid/a x@y", true), "h.invalid/a x@y");
        assert_eq!(scrub("", true), "");
    }

    #[test]
    fn output_is_cut_at_a_character_and_says_so() {
        assert_eq!(tail("short\n\n"), ("short".to_owned(), false));
        assert_eq!(tail(""), (String::new(), false));
        let exact = "x".repeat(OUTPUT_LIMIT);
        assert_eq!(tail(&exact), (exact.clone(), false));

        // Two bytes a character, and a cut that would fall inside one.
        let long = format!("a{}THE END\n", "é".repeat(OUTPUT_LIMIT));
        let (kept, cut) = tail(&long);
        assert!(cut);
        assert!(kept.len() <= OUTPUT_LIMIT, "{}", kept.len());
        assert!(kept.len() >= OUTPUT_LIMIT - 4, "{}", kept.len());
        assert!(kept.starts_with('é') && kept.ends_with("éTHE END"));

        // A secret that the cut went through does not show.
        let secret = "s".repeat(100);
        let padding = "p".repeat(OUTPUT_LIMIT - 60);
        let long = format!("see https://ada:{secret}@example.invalid/a.git\n{padding}");
        let (kept, cut) = tail(&long);
        assert!(cut);
        assert!(!kept.contains("sss"), "{}", &kept[..80]);
        assert!(kept.starts_with("***@example.invalid/a.git\n"));

        // A command line is cut at its end, and marked.
        assert_eq!(head("git status", 100), "git status");
        let cut = head(&"é".repeat(100), 51);
        assert_eq!(cut, format!("{}…", "é".repeat(25)));
    }

    #[test]
    fn questions_are_told_from_changes() {
        let asks = |line: &str| {
            let args: Vec<&str> = line.split(' ').filter(|arg| !arg.is_empty()).collect();
            asks(&args)
        };
        for question in [
            "--version",
            "version",
            "rev-parse --verify --quiet refs/tags/v1",
            "rev-parse --git-path hooks",
            "rev-list --parents -n 1 HEAD",
            "merge-base --is-ancestor a b",
            "check-ref-format refs/heads/x",
            "for-each-ref --format=%(refname) refs/heads",
            "ls-files --unmerged -z",
            "ls-tree -r --name-only -z HEAD",
            "cat-file blob 1234",
            "merge-tree --write-tree --name-only -z a b",
            "write-tree",
            "hash-object -t tree --stdin",
            "status --porcelain=v2",
            "--no-optional-locks status",
            "diff --name-only --diff-filter=U -z",
            "diff --cached --quiet",
            "diff --output-indicator-new=> HEAD",
            "--literal-pathspecs diff -- --output=x",
            "log -z --reverse --format=%H a..b",
            "log -S--output=x",
            "show --name-only --format= -z stash@{0}^3",
            "-c core.quotepath=false show HEAD",
            "-C /somewhere log -1",
            "symbolic-ref --quiet --short HEAD",
            "symbolic-ref HEAD",
            "config --get user.name",
            "config --get --type=path core.hooksPath",
            "config --get-regexp ^gitflow\\.",
            "config --list --show-origin",
            "config -l",
            "config get user.name",
            "config list",
            "stash list --format=%H",
            "stash show -p stash@{0}",
            "remote",
            "remote -v",
            "remote get-url --push origin",
            "tag",
            "tag -l v*",
            "tag --list --sort=-creatordate --format=%(refname)",
            "tag -v v1",
            "branch --show-current",
            "branch --list -a --format=%(refname)",
            "worktree list --porcelain",
            "submodule",
            "submodule status --recursive",
            "rerere status",
            "lfs ls-files",
            "apply --check --index patch",
            "merge-file -p ours base theirs",
            "grep -n -e needle HEAD",
            "blame --porcelain -- a.txt",
            "verify-commit --raw HEAD",
        ] {
            assert!(asks(question), "{question:?} only asks");
        }
        for change in [
            "",
            "--",
            "-c",
            "--git-dir=/elsewhere status",
            "--exec-path=/elsewhere status",
            "--unknown-option status",
            "commit --quiet --message x",
            "tag -- v1 1234",
            "tag v1",
            "tag -l -d v1",
            "tag -ld v1",
            "tag --list --del v1",
            "tag -a -m notes v1",
            "update-ref -d refs/tags/v1 1234",
            "branch --no-track -- topic 1234",
            "branch -D -- topic",
            "branch --list --delete topic",
            "branch",
            "switch -- main",
            "--literal-pathspecs add -- a.txt",
            "--literal-pathspecs checkout --ours -- a.txt",
            "reset --quiet --hard HEAD",
            "stash",
            "stash push --include-untracked",
            "stash drop --quiet stash@{0}",
            "symbolic-ref HEAD refs/heads/other",
            "symbolic-ref -m reason HEAD refs/heads/other",
            "symbolic-ref --delete HEAD",
            "symbolic-ref -d HEAD",
            "symbolic-ref -- HEAD refs/heads/other",
            "symbolic-ref",
            "config user.name Ada",
            "config user.name",
            "config --get --unset user.name",
            "config --get --uns user.name",
            "config --add user.name Ada",
            "config -le",
            "config set user.name Ada",
            "config --global get user.name",
            "remote add -- origin https://example.invalid/a.git",
            "remote remove -- origin",
            "remote -v show origin",
            "remote show origin",
            "diff --output=patch HEAD",
            "diff --output patch HEAD",
            "diff --outp=patch HEAD",
            "log --output=patch -1",
            "show --output=patch HEAD",
            "grep -Oless needle",
            "grep --open-files-in-pager needle",
            "fetch --prune origin",
            "ls-remote origin",
            "push origin main",
            "pull --ff-only",
            "clone https://example.invalid/a.git",
            "worktree add ../other",
            "worktree prune",
            "submodule update --init",
            "submodule --quiet status",
            "rerere",
            "rerere forget a.txt",
            "lfs track *.bin",
            "lfs pull",
            "apply patch",
            "apply --check --apply patch",
            "merge-file ours base theirs",
            "update-index --refresh",
            "difftool --tool=x",
            "-c difftool.x.cmd=y difftool --tool=x",
            "rebase --interactive HEAD~2",
            "no-such-command",
            "help status",
        ] {
            assert!(!asks(change), "{change:?} is not known to only ask");
        }
    }

    #[test]
    fn the_command_a_thread_runs_is_named_and_nests() {
        let named = || UNDER.with(Cell::get);
        assert_eq!(named(), None);
        {
            let _outer = under("outer.command");
            assert_eq!(named(), Some("outer.command"));
            {
                let _inner = under("inner.command");
                assert_eq!(named(), Some("inner.command"));
                // Another thread has no name from this one.
                assert_eq!(std::thread::spawn(named).join().unwrap(), None);
            }
            assert_eq!(named(), Some("outer.command"));
        }
        assert_eq!(named(), None);
    }

    /// A directory nothing else records under, for looking at the record
    /// all tests of this binary share.
    fn own_directory(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("bitbraid-activity-{name}-{}", std::process::id()))
    }

    fn mine(directory: &Path) -> Vec<Arc<Entry>> {
        snapshot()
            .into_iter()
            .filter(|entry| entry.directory == directory)
            .collect()
    }

    #[test]
    fn a_process_is_recorded_once_however_it_ends() {
        let directory = own_directory("ends");
        let mut cmd = Command::new("git");
        cmd.current_dir(&directory)
            .args(["push", "https://ada:hunter2@example.invalid/a.git"]);
        let rendered = "git push https://ada:hunter2@example.invalid/a.git";

        // Told how it ended: once, whatever is said afterwards.
        let _named = under("test.push");
        let started = begin(&cmd, rendered);
        started.cancelled("", "stopped half-way\n");
        started.cancelled("", "said twice");
        drop(started);
        // Not told: the entry says so.
        drop(begin(&cmd, rendered));
        // Not started at all.
        let started = begin(&cmd, rendered);
        started.not_started(&std::io::Error::other("no such program"));
        drop(started);

        let entries = mine(&directory);
        let ends: Vec<End> = entries.iter().map(|entry| entry.end).collect();
        assert_eq!(ends, [End::Cancelled, End::Lost, End::NotStarted]);
        assert!(entries.windows(2).all(|pair| pair[0].id < pair[1].id));
        let first = &entries[0];
        assert_eq!(first.command, "git push https://***@example.invalid/a.git");
        assert_eq!(first.stderr, "stopped half-way");
        assert_eq!(first.code, None);
        assert!(!first.question);
        assert_eq!(first.origin.as_deref(), Some("test.push"));
        assert_eq!(first.repository, None, "nobody said for which repository");
        // Numbered from the moment the process first recorded something,
        // in microseconds, and so later than anything an earlier run made.
        assert!(
            first.id > first.started * 1000 - 3_600_000_000,
            "{}",
            first.id
        );
        assert!(first.id < (first.started + 1000) * 1000, "{}", first.id);
        assert!(first.started > 1_600_000_000_000, "{}", first.started);
        assert_eq!(entries[2].stderr, "no such program");
    }

    #[test]
    fn a_tool_left_running_is_one_entry_completed_when_it_is_collected() {
        let directory = own_directory("tool");
        let mut cmd = Command::new("git");
        cmd.current_dir(&directory).args(["difftool", "--tool=x"]);

        let started = begin(&cmd, "git difftool --tool=x");
        started.running();
        let entries = mine(&directory);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].end, End::Running);
        let id = entries[0].id;

        // Collected on another thread, as a tool is.
        std::thread::spawn(move || {
            started.exited(ExitStatus::default(), "", "the tool said this\n");
        })
        .join()
        .unwrap();
        let entries = mine(&directory);
        assert_eq!(entries.len(), 1, "completed in place");
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].end, End::Exited);
        assert_eq!(entries[0].code, Some(0));
        assert_eq!(entries[0].stderr, "the tool said this");

        // One that is never collected is not left running for ever.
        let started = begin(&cmd, "git difftool --tool=y");
        started.running();
        drop(started);
        let entries = mine(&directory);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[1].end, End::Lost);
    }

    #[test]
    fn a_relative_directory_is_recorded_as_an_absolute_one() {
        let mut cmd = Command::new("git");
        let name = format!("bitbraid-activity-relative-{}", std::process::id());
        cmd.current_dir(&name).arg("status");
        let started = begin(&cmd, "git status");
        started.running();
        drop(started);
        let here = std::env::current_dir().unwrap().join(&name);
        let entries = mine(&here);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].question);
    }
}
