//! One session thread per open repository.
//!
//! `gix::Repository` is not `Send`, so the repository lives on its own
//! thread together with the engine context. The UI sends typed requests and
//! awaits typed replies; the filesystem watcher reports changes as events.
//!
//! What nobody asked for stays on this machine. A
//! request is sent either as asked by the user ([`SessionHandle::ask`]) or
//! not ([`SessionHandle::send`]): what keeps the views current, which most
//! requests are. One that was not asked for and only reads runs under
//! `bitbraid_git::exe::local_only`, so that git fetches nothing a partial
//! clone left on its remote, and a read that needs what was left there
//! fails saying so, or answers that it cannot tell. Operations, and
//! commands that change the repository, are the user's in any case.
//!
//! Sources: git's documentation of partial clone (`partial-clone.adoc`),
//! git(1) (`GIT_NO_LAZY_FETCH`).

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
    time::Duration,
};

use bitbraid_engine::{CommandContext, Registry};
use bitbraid_git::{
    ChangeSet, CommitInfo, GraphCommit, GraphRow, Oid, Order, RepoSummary, Repository, Snapshot,
    StatusEntry, Tips, Watcher,
};
use bitbraid_integrations::registry as composed_registry;
use futures::channel::{mpsc as fmpsc, oneshot};
use gpui_kit::*;

mod graphs;

/// What kind of reference a badge represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BadgeKind {
    /// HEAD (detached or the current branch).
    Head,
    /// Local branch.
    Local,
    /// Remote-tracking branch.
    Remote,
    Tag,
    Stash,
}

/// A reference drawn next to a commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Badge {
    pub kind: BadgeKind,
    /// Short name.
    pub name: String,
    /// Full reference name, when it is a reference.
    pub full_name: Option<String>,
}

/// Another worktree of the repository, for the mark on the commit it has
/// checked out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeMark {
    /// Where it is.
    pub path: String,
    /// The commit it has checked out.
    pub head: Oid,
    /// The branch it is on, when it is on one.
    pub branch: Option<String>,
}

/// The graph the views render, built once per load on the session thread.
#[derive(Debug)]
pub struct GraphData {
    /// Commits in row order.
    pub commits: Vec<GraphCommit>,
    /// Lane layout per row.
    pub rows: Vec<GraphRow>,
    /// Layout of the row for uncommitted work, present when the worktree was
    /// dirty as of the last status request. Its lane runs down to HEAD.
    pub wip: Option<GraphRow>,
    /// Row index by commit id.
    pub index: HashMap<Oid, usize>,
    /// Per row, the nearest row above it holding a child of its commit;
    /// `u32::MAX` for a commit without children in the graph.
    pub child: Vec<u32>,
    /// Badges by commit id.
    pub badges: HashMap<Oid, Vec<Badge>>,
    /// The worktrees other than the one that is open.
    pub worktrees: Vec<WorktreeMark>,
    /// The rows that stand for uncommitted work in another worktree: by
    /// the id of the row, which is no commit's, the worktree's place in
    /// `worktrees`.
    pub worktree_rows: HashMap<Oid, usize>,
    /// What each local branch follows, where it follows something: the
    /// branch's short name and its upstream's (`origin/main`).
    pub upstreams: HashMap<String, String>,
    /// HEAD commit.
    pub head: Option<Oid>,
    /// Widest row, in lanes.
    pub max_width: u16,
    /// The order the rows are in.
    pub order: Order,
    /// For the leading rows of a history that is still being walked: how
    /// many commits were walked so far. None for a whole history.
    pub walked: Option<usize>,
    /// The number of the request this graph was built for. The pieces of
    /// a history and its whole graph have the same.
    pub generation: u64,
}

impl GraphData {
    /// Whether this graph should take the place of `shown`: not when it is
    /// a smaller part of the same history, which a late answer can be.
    pub fn follows(&self, shown: &GraphData) -> bool {
        if self.generation != shown.generation {
            return true;
        }
        match (self.walked, shown.walked) {
            (None, _) => true,
            (Some(_), None) => false,
            (Some(now), Some(before)) => now >= before,
        }
    }
}

/// Whether a status has anything to commit or stage: what puts the
/// uncommitted-work row on top of the graph.
pub fn is_dirty(status: &[StatusEntry]) -> bool {
    status
        .iter()
        .any(|entry| entry.staged.is_some() || entry.unstaged.is_some())
}

/// A request to the session thread.
#[derive(Debug, Clone)]
pub enum Request {
    /// The references snapshot.
    Refs,
    /// The graph for the given tips (hidden references already excluded).
    Graph {
        /// Starting points.
        tips: Tips,
        /// Full names of references to exclude from the walk.
        exclude: Vec<String>,
        /// Full names of the branches pinned to the left, in the order of
        /// their lanes. One that is not in the graph has none.
        pinned: Vec<String>,
        /// The order of the rows.
        order: Order,
        /// Paths of the other worktrees that have uncommitted work: each
        /// gets a row above the commit it has checked out.
        worktree_rows: Vec<String>,
    },
    /// Decoded details for commits.
    Details(Vec<Oid>),
    /// Details for the rows the graph handed out last shows first when it
    /// opens: asked with the first graph, ahead of the window, so that its
    /// first frame has them (see [`first_shown`]).
    FirstDetails(Shown),
    /// Full message of one commit.
    Message(Oid),
    /// Working tree status.
    Status,
    /// First-parent ancestry of a commit, for hover highlighting.
    Ancestry(Oid),
    /// Diff of one path between two sides (`HEAD`, `index`, `worktree`,
    /// `empty`, a revision).
    DiffFile {
        /// Old side.
        old: String,
        /// New side.
        new: String,
        /// Path relative to the worktree.
        path: String,
        /// How whitespace counts.
        whitespace: bitbraid_git::Whitespace,
        /// Unchanged lines around each change; `u32::MAX` for the whole file.
        context: u32,
        /// Where the file is on the old side, when it was moved.
        old_path: Option<String>,
        /// A side larger than this many bytes is measured and not
        /// compared; none compares whatever the size.
        max_bytes: Option<u64>,
    },
    /// Files changed by a commit against its first parent (or the empty tree).
    CommitFiles(Oid),
    /// Files that differ between two commits; without an old one, against
    /// the empty tree.
    RangeFiles {
        /// The older end.
        old: Option<Oid>,
        /// The newer end.
        new: Oid,
    },
    /// Run any engine command by id with JSON parameters.
    Command {
        /// Command id, for example `stage.paths`.
        id: String,
        /// Parameters.
        params: serde_json::Value,
    },
    /// Run an engine command that can take a while (it talks to a remote,
    /// or runs hooks) as an operation: on the operations thread, one at a
    /// time, reporting progress as events, stoppable.
    Operation {
        /// Command id, for example `remote.fetch`.
        id: String,
        /// Parameters.
        params: serde_json::Value,
    },
    /// Stop the operation that is running.
    CancelOperation,
    /// Stop the slow reads of this command that are running: a search
    /// that is no longer wanted, a count of files that is
    /// asked for afresh. They answer with an error that says so.
    CancelReads(String),
}

/// Where a graph stands when it opens, as it was left: the commit at the
/// top of its list and the selected one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Shown {
    /// The commit that was at the top of the list.
    pub top: Option<Oid>,
    /// The commit that was selected.
    pub selected: Option<Oid>,
}

impl Shown {
    /// From the ids a repository's remembered state keeps, as text.
    pub fn remembered(top: Option<&str>, selected: Option<&str>) -> Self {
        Self {
            top: top.and_then(|id| id.parse().ok()),
            selected: selected.and_then(|id| id.parse().ok()),
        }
    }
}

/// How many rows are read ahead from where a graph opens: more than a tall
/// window shows, and as many as the graph asks for around the selection.
const FIRST_ROWS: usize = 96;

/// The commits a graph shows first when it opens where `shown` says: from
/// the row of the commit that was at the top, or from the first row, and,
/// without a top, around the commit it selects, which its list is centred
/// on: the one that was selected, else HEAD's (as `GraphView::set_graph`
/// chooses; one that is no longer in the graph selects the first row).
/// Rows that stand for another worktree's uncommitted work are no commits.
pub fn first_shown(graph: &GraphData, shown: Shown) -> Vec<Oid> {
    let row_of = |id: Option<Oid>| id.and_then(|id| graph.index.get(&id).copied());
    let top = row_of(shown.top);
    let mut rows: Vec<usize> = (top.unwrap_or(0)..).take(FIRST_ROWS).collect();
    if top.is_none()
        && let Some(at) = row_of(shown.selected.or(graph.head))
    {
        let half = FIRST_ROWS / 2;
        rows.extend(at.saturating_sub(half)..at.saturating_add(half));
    }
    rows.sort_unstable();
    rows.dedup();
    rows.into_iter()
        .filter_map(|ix| graph.commits.get(ix))
        .map(|commit| commit.id)
        .filter(|id| !graph.worktree_rows.contains_key(id))
        .collect()
}

/// A reply from the session thread.
#[derive(Debug)]
pub enum Reply {
    Refs(Snapshot),
    Graph(Arc<GraphData>),
    Details(Vec<CommitInfo>),
    Message(String),
    /// Status, and whether there are more changed paths than it holds
    /// (it is cut at `bitbraid_git::STATUS_LIMIT`).
    Status(Vec<StatusEntry>, bool),
    /// Ancestry as a set of commit ids.
    Ancestry(Vec<Oid>),
    /// A file diff, absent when the path exists on neither side.
    DiffFile(Option<bitbraid_git::FileDiff>),
    /// Files changed by a commit.
    CommitFiles(Vec<bitbraid_git::FileDiff>),
    /// Output of an engine command.
    Command(serde_json::Value),
    /// The request failed.
    Error(String),
}

/// Something happened on the session thread.
#[derive(Debug, Clone)]
pub enum Event {
    /// The repository changed on disk.
    Changed(ChangeSet),
    /// More of a large history has been walked: a larger piece of the
    /// graph that was handed out, or all of it.
    Graph(Arc<GraphData>),
    /// The walk of a large history failed after its first rows were
    /// handed out.
    GraphFailed {
        /// The number of the request the walk was for.
        generation: u64,
        /// What went wrong.
        error: String,
    },
    /// The running operation says where it is.
    Progress {
        /// Command id of the operation.
        command: String,
        /// What git is doing, for example `Receiving objects`.
        stage: String,
        /// How far along, when known.
        percent: Option<u8>,
    },
    /// The session thread began a command that changes the repository,
    /// by its label, or answered it (none). Everything asked of the thread
    /// after it waits meanwhile.
    Busy(Option<String>),
}

/// A request on its way to a session thread, and where its reply goes.
struct Envelope {
    request: Request,
    /// Whether the user asked for it (see [`SessionHandle::ask`]).
    asked: bool,
    reply: ReplySender,
}

/// How many requests the threads here have been sent and have not answered
/// yet, whichever thread they went to. A reply wakes the task that waits
/// for it before it is counted off, so "none owed, and nothing left to run
/// on the main thread" means the interface is at rest. That is what the
/// off-screen tests wait for instead of for a length of time.
static OWED: Owed = Owed {
    count: std::sync::Mutex::new(0),
    changed: std::sync::Condvar::new(),
};

struct Owed {
    count: std::sync::Mutex<usize>,
    changed: std::sync::Condvar,
}

impl Owed {
    fn add(&self) {
        let mut count = match self.count.lock() {
            Ok(count) => count,
            Err(poisoned) => poisoned.into_inner(),
        };
        *count = count.saturating_add(1);
    }

    fn take(&self) {
        let mut count = match self.count.lock() {
            Ok(count) => count,
            Err(poisoned) => poisoned.into_inner(),
        };
        *count = count.saturating_sub(1);
        self.changed.notify_all();
    }
}

/// How many requests are waiting for their reply.
pub fn unanswered() -> usize {
    match OWED.count.lock() {
        Ok(count) => *count,
        Err(poisoned) => *poisoned.into_inner(),
    }
}

/// Wait until fewer requests are waiting than `known`, or for `limit` at
/// the most. Returns how many are waiting then.
pub fn wait_for_reply(known: usize, limit: Duration) -> usize {
    let count = match OWED.count.lock() {
        Ok(count) => count,
        Err(poisoned) => poisoned.into_inner(),
    };
    match OWED
        .changed
        .wait_timeout_while(count, limit, |count| *count >= known && *count > 0)
    {
        Ok((count, _)) => *count,
        Err(poisoned) => *poisoned.into_inner().0,
    }
}

/// The end a reply is given at. It counts as owed from the moment it is
/// made until it was used or dropped.
#[derive(Debug)]
struct ReplySender(Option<oneshot::Sender<Reply>>);

impl ReplySender {
    fn new(tx: oneshot::Sender<Reply>) -> Self {
        OWED.add();
        Self(Some(tx))
    }

    /// Give the reply. False when nobody waits for it any more.
    fn send(mut self, reply: Reply) -> bool {
        self.0.take().is_some_and(|tx| tx.send(reply).is_ok())
        // Counted off as `self` goes, which is after the task was woken.
    }
}

impl Drop for ReplySender {
    fn drop(&mut self) {
        // Dropping the sender wakes the task too: it sees the request
        // cancelled.
        drop(self.0.take());
        OWED.take();
    }
}

/// Handle to a session thread. Cloneable; dropping the last handle ends the thread.
#[derive(Clone)]
pub struct SessionHandle {
    /// Path the repository was opened with.
    pub path: PathBuf,
    /// Summary at open time.
    pub summary: RepoSummary,
    tx: mpsc::Sender<Envelope>,
}

impl std::fmt::Debug for SessionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHandle")
            .field("path", &self.path)
            .finish()
    }
}

impl SessionHandle {
    /// Open a repository on a new session thread. Blocks until the repository
    /// is open (fast, but for the first one of a process, which waits for
    /// the git in use to say where its system configuration is) and returns
    /// the handle plus the event stream.
    pub fn open(path: &Path) -> Result<(Self, fmpsc::UnboundedReceiver<Event>), String> {
        Opening::start(path)?.wait()
    }

    /// Send a request nobody asked for: one that keeps a view current.
    /// When it only reads, it reads what is on this machine and nothing
    /// else (see the module's documentation). The receiver resolves with
    /// the reply.
    pub fn send(&self, request: Request) -> oneshot::Receiver<Reply> {
        send_to(&self.tx, request, false)
    }

    /// Send a request the user asked for (a commit they selected, a file's
    /// diff, history or blame they opened): it may fetch what a partial
    /// clone left on its remote, as git would. The receiver resolves with
    /// the reply.
    pub fn ask(&self, request: Request) -> oneshot::Receiver<Reply> {
        send_to(&self.tx, request, true)
    }

    /// Send a request nobody asked for, as [`Self::send`], and deliver the
    /// reply to an entity.
    pub fn call<V: 'static>(
        &self,
        request: Request,
        cx: &mut Context<V>,
        on_reply: impl FnOnce(&mut V, Reply, &mut Context<V>) + 'static,
    ) {
        self.call_as(false, request, cx, on_reply);
    }

    /// Send a request the user asked for, as [`Self::ask`], and deliver
    /// the reply to an entity.
    pub fn call_asked<V: 'static>(
        &self,
        request: Request,
        cx: &mut Context<V>,
        on_reply: impl FnOnce(&mut V, Reply, &mut Context<V>) + 'static,
    ) {
        self.call_as(true, request, cx, on_reply);
    }

    /// [`Self::call_asked`] when `asked`, else [`Self::call`].
    pub fn call_as<V: 'static>(
        &self,
        asked: bool,
        request: Request,
        cx: &mut Context<V>,
        on_reply: impl FnOnce(&mut V, Reply, &mut Context<V>) + 'static,
    ) {
        let name = request_name(&request);
        let rx = send_to(&self.tx, request, asked);
        Self::deliver(rx, name, cx, on_reply);
    }

    /// Deliver the reply to a request already sent with [`Self::send`].
    pub fn deliver<V: 'static>(
        rx: oneshot::Receiver<Reply>,
        name: String,
        cx: &mut Context<V>,
        on_reply: impl FnOnce(&mut V, Reply, &mut Context<V>) + 'static,
    ) {
        cx.spawn(async move |this, cx| match rx.await {
            Ok(reply) => {
                debug(|| format!("reply for {name}: {}", reply_name(&reply)));
                let updated = this.update(cx, |view, cx| on_reply(view, reply, cx));
                if updated.is_err() {
                    debug(|| format!("reply for {name} dropped: entity gone"));
                }
            }
            Err(_) => debug(|| format!("reply for {name} cancelled")),
        })
        .detach();
    }
}

/// Put a request in a session thread's queue, as the user's when `asked`;
/// the receiver resolves with the reply.
fn send_to(tx: &mpsc::Sender<Envelope>, request: Request, asked: bool) -> oneshot::Receiver<Reply> {
    let (reply_tx, reply_rx) = oneshot::channel();
    debug(|| format!("send {}", request_name(&request)));
    let envelope = Envelope {
        request,
        asked,
        reply: ReplySender::new(reply_tx),
    };
    if tx.send(envelope).is_err() {
        debug(|| "send failed: session thread gone".to_owned());
    }
    reply_rx
}

/// A session thread that was started and has not been waited for: it is
/// opening its repository. Requests can be queued meanwhile; they are
/// answered once it is open, and dropped unanswered if it does not open.
pub struct Opening {
    path: PathBuf,
    tx: mpsc::Sender<Envelope>,
    events: fmpsc::UnboundedReceiver<Event>,
    ready: mpsc::Receiver<Result<RepoSummary, String>>,
}

impl std::fmt::Debug for Opening {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // not shown: the type's name, for developers
        f.debug_struct("Opening").field("path", &self.path).finish()
    }
}

impl Opening {
    /// Start a session thread for `path` without waiting for it.
    pub fn start(path: &Path) -> Result<Self, String> {
        let (tx, rx) = mpsc::channel::<Envelope>();
        let (events_tx, events_rx) = fmpsc::unbounded::<Event>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<RepoSummary, String>>();
        let path_owned = path.to_path_buf();
        let spawned = thread::Builder::new()
            // not shown: the thread's name
            .name(format!("bitbraid-session {}", path_owned.display()))
            .spawn(move || session_main(path_owned, rx, events_tx, ready_tx));
        if let Err(err) = spawned {
            return Err(say!("session-cannot-start", error = err.to_string()));
        }
        Ok(Self {
            path: path.to_path_buf(),
            tx,
            events: events_rx,
            ready: ready_rx,
        })
    }

    /// Queue a request nobody asked for (see [`SessionHandle::send`]) for
    /// the session that is opening.
    pub fn send(&self, request: Request) -> oneshot::Receiver<Reply> {
        send_to(&self.tx, request, false)
    }

    /// Wait until the repository is open: the handle and the event stream,
    /// or why it did not open.
    pub fn wait(self) -> Result<(SessionHandle, fmpsc::UnboundedReceiver<Event>), String> {
        let summary = self
            .ready
            .recv()
            .map_err(|_| say!("session-ended-before-opening").to_owned())??;
        Ok((
            SessionHandle {
                path: self.path,
                summary,
                tx: self.tx,
            },
            self.events,
        ))
    }
}

/// Replies to the requests every workspace starts with, sent before the
/// workspace exists.
#[derive(Debug)]
pub struct StartupReplies {
    /// Reply to [`Request::Status`].
    pub status: oneshot::Receiver<Reply>,
    /// Reply to [`Request::Refs`].
    pub refs: oneshot::Receiver<Reply>,
    /// Reply to the [`Request::Graph`] for the remembered visibility.
    pub graph: oneshot::Receiver<Reply>,
    /// Reply to the [`Request::FirstDetails`] of that graph.
    pub details: oneshot::Receiver<Reply>,
}

/// A session opened ahead of its window. Creating a window takes several
/// hundred milliseconds, most of it in the graphics driver; a repository
/// opened first is read on its session thread in the meantime.
#[derive(Debug)]
pub struct Preload {
    pub session: SessionHandle,
    /// Its event stream.
    pub events: fmpsc::UnboundedReceiver<Event>,
    /// Startup requests already on their way, when the graph request could
    /// be told without the references.
    pub replies: Option<StartupReplies>,
}

/// A session started ahead of its window, and the startup requests queued
/// for it.
#[derive(Debug)]
struct Ahead {
    opening: Opening,
    replies: Option<StartupReplies>,
}

/// Sessions opened ahead of their window, by path as the shell asks for it.
#[derive(Debug, Default)]
pub struct Preloaded(HashMap<PathBuf, Ahead>);

impl Global for Preloaded {}

impl Preloaded {
    /// Start opening `path` and queue its startup requests. `graph` is the
    /// graph request, absent when it depends on the references. Status goes
    /// first: the graph is laid out from the last status. The details of
    /// the rows the graph shows first, where `shown` says it stands, come
    /// last, so that the first frame has the commits' text and not only
    /// their lanes.
    ///
    /// Nothing here waits for the repository to be open: the first one a
    /// process opens asks the git in use where its system configuration is,
    /// which is a child process and tens of milliseconds, and
    /// the window is asked for meanwhile.
    pub fn open(&mut self, path: &Path, graph: Option<Request>, shown: Shown) {
        if self.0.contains_key(path) {
            return;
        }
        let Ok(opening) = Opening::start(path) else {
            return; // the shell opens it again and reports the error
        };
        let replies = graph.map(|graph| StartupReplies {
            status: opening.send(Request::Status),
            refs: opening.send(Request::Refs),
            graph: opening.send(graph),
            details: opening.send(Request::FirstDetails(shown)),
        });
        self.0
            .insert(path.to_path_buf(), Ahead { opening, replies });
    }

    /// Hand over the session started for `path`, if any, waiting for it to
    /// be open if it is not yet. None also when it did not open: the shell
    /// opens it again and reports the error.
    pub fn take(&mut self, path: &Path) -> Option<Preload> {
        let Ahead { opening, replies } = self.0.remove(path)?;
        let (session, events) = opening.wait().ok()?;
        Some(Preload {
            session,
            events,
            replies,
        })
    }
}

/// Where the application keeps its own files, for the commands that keep
/// some there (profiles). Set when the preferences are loaded: once in the
/// application, and anew for every window the off-screen tests open, each
/// with a directory of its own.
static CONFIG_DIR: std::sync::RwLock<Option<PathBuf>> = std::sync::RwLock::new(None);

/// Tell the sessions where the application's configuration directory is.
/// Threads that are running keep the one they started with.
pub fn set_config_dir(dir: Option<PathBuf>) {
    // A poisoned lock still holds a usable value: take it over.
    let mut known = match CONFIG_DIR.write() {
        Ok(known) => known,
        Err(poisoned) => poisoned.into_inner(),
    };
    *known = dir;
}

/// The application's configuration directory, when there is one.
fn config_dir() -> Option<PathBuf> {
    match CONFIG_DIR.read() {
        Ok(known) => known.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// What the threads of every session are running, for the moment the
/// application is quit: the flags that stop an operation, and how many
/// commands are in flight.
#[derive(Debug)]
pub struct Running {
    cancels: std::sync::Mutex<Vec<std::sync::Weak<std::sync::atomic::AtomicBool>>>,
    busy: std::sync::atomic::AtomicUsize,
    quitting: std::sync::atomic::AtomicBool,
}

/// One command in flight; it is over when this is dropped.
#[derive(Debug)]
pub struct Busy<'a>(&'a Running);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        // Never below zero, whatever happened in between.
        let _ = self.0.busy.try_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |busy| Some(busy.saturating_sub(1)),
        );
    }
}

impl Default for Running {
    fn default() -> Self {
        Self::new()
    }
}

impl Running {
    /// Nothing running.
    pub const fn new() -> Self {
        Self {
            cancels: std::sync::Mutex::new(Vec::new()),
            busy: std::sync::atomic::AtomicUsize::new(0),
            quitting: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Have `flag` set when everything is stopped. Flags nobody holds any
    /// more are forgotten on the way.
    pub fn watch(&self, flag: &Arc<std::sync::atomic::AtomicBool>) {
        if let Ok(mut cancels) = self.cancels.lock() {
            cancels.retain(|known| known.strong_count() > 0);
            cancels.push(Arc::downgrade(flag));
        }
    }

    /// A command is about to run. None once everything was stopped: what
    /// was still waiting its turn does not start.
    pub fn begin(&self) -> Option<Busy<'_>> {
        use std::sync::atomic::Ordering;
        self.busy.fetch_add(1, Ordering::SeqCst);
        let busy = Busy(self);
        // Counted first and asked second: `stop` either sees the count or
        // has said that it is over.
        (!self.quitting.load(Ordering::SeqCst)).then_some(busy)
    }

    /// Stop every operation and wait, for at most `limit`, until nothing
    /// is in flight. True when everything ended in time.
    pub fn stop(&self, limit: Duration) -> bool {
        use std::sync::atomic::Ordering;
        self.quitting.store(true, Ordering::SeqCst);
        if let Ok(cancels) = self.cancels.lock() {
            for flag in cancels.iter().filter_map(std::sync::Weak::upgrade) {
                flag.store(true, Ordering::SeqCst);
            }
        }
        let started = std::time::Instant::now();
        while self.busy.load(Ordering::SeqCst) > 0 {
            if started.elapsed() >= limit {
                return false;
            }
            thread::sleep(Duration::from_millis(10));
        }
        true
    }
}

/// What this process is running.
static RUNNING: Running = Running::new();

/// The application is being quit: stop the operations that are running
/// (a fetch, a transfer of large files and the git processes under them
/// would otherwise go on without it), let what is in flight end, and have
/// the last entries of the activity log written. Bounded: a quit never
/// hangs on a process that does not die.
pub fn quit() {
    let ended = RUNNING.stop(Duration::from_secs(2));
    let written = bitbraid_git::activity::flush(Duration::from_secs(2));
    debug(|| format!("quit: operations ended {ended}, activity log written {written}"));
}

/// Debug logging to stderr when `BITBRAID_DEBUG` is set.
#[allow(clippy::print_stderr)]
pub(crate) fn debug(message: impl FnOnce() -> String) {
    if std::env::var_os("BITBRAID_DEBUG").is_some() {
        eprintln!("[session] {}", message());
    }
}

/// Log a startup milestone with the time since the first milestone (the
/// start of `run`), when `BITBRAID_DEBUG` is set.
pub(crate) fn milestone(what: &str) {
    static START: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
    let start = START.get_or_init(std::time::Instant::now);
    debug(|| format!("milestone {what} at {:?}", start.elapsed()));
}

fn request_name(request: &Request) -> String {
    match request {
        Request::Refs => "refs".to_owned(),
        Request::Graph { .. } => "graph".to_owned(),
        Request::Details(ids) => format!("details({})", ids.len()),
        Request::FirstDetails(_) => "first_details".to_owned(),
        Request::Message(id) => format!("message({})", id.short(7)),
        Request::Status => "status".to_owned(),
        Request::Ancestry(id) => format!("ancestry({})", id.short(7)),
        // not shown: what the debug notes call a request
        Request::DiffFile { old, new, path, .. } => format!("diff_file({old}..{new} {path})"),
        Request::CommitFiles(id) => format!("commit_files({})", id.short(7)),
        Request::RangeFiles { old, new } => format!(
            "range_files({}..{})",
            old.map(|id| id.short(7))
                .unwrap_or_else(|| "empty".to_owned()),
            new.short(7)
        ),
        Request::Command { id, .. } => format!("command({id})"),
        Request::Operation { id, .. } => format!("operation({id})"),
        Request::CancelOperation => "cancel_operation".to_owned(),
        Request::CancelReads(id) => format!("cancel_reads({id})"),
    }
}

fn reply_name(reply: &Reply) -> String {
    match reply {
        Reply::Refs(_) => "refs".to_owned(),
        // not shown: what the debug notes call a reply
        Reply::Graph(g) => format!("graph({} rows)", g.rows.len()),
        Reply::Details(d) => format!("details({})", d.len()),
        Reply::Message(_) => "message".to_owned(),
        Reply::Status(s, more) => {
            // not shown: as above
            format!("status({}{})", s.len(), if *more { ", cut" } else { "" })
        }
        Reply::Ancestry(a) => format!("ancestry({})", a.len()),
        Reply::DiffFile(d) => format!(
            "diff_file({})",
            d.as_ref().map(|d| d.hunks.len()).unwrap_or(0)
        ),
        Reply::CommitFiles(f) => format!("commit_files({})", f.len()),
        Reply::Command(_) => "command".to_owned(),
        // not shown: as above
        Reply::Error(e) => format!("error: {e}"),
    }
}

struct SessionState {
    ctx: CommandContext,
    registry: Registry,
    /// The graphs handed out: the last whole one, which a reload that
    /// finds the references where they were gets again, and a large
    /// history that is still being walked.
    graphs: graphs::Graphs,
    /// Dirtiness of the worktree as of the last status request.
    dirty: bool,
    /// The references as the request handled last read them, when it was
    /// [`Request::Refs`]: the graph asked for right after them (a reload
    /// asks for both, one after the other) is laid out from them instead
    /// of reading every reference again. Any other request in between
    /// drops them. A read of thousands of loose references can take a
    /// quarter of a second on Windows.
    refs_just_read: Option<bitbraid_git::Snapshot>,
    _watcher: Option<Watcher>,
}

fn session_main(
    path: PathBuf,
    rx: mpsc::Receiver<Envelope>,
    events_tx: fmpsc::UnboundedSender<Event>,
    ready_tx: mpsc::Sender<Result<RepoSummary, String>>,
) {
    let repo = match Repository::open(&path) {
        Ok(repo) => repo,
        Err(err) => {
            let _ = ready_tx.send(Err(err.to_string()));
            return;
        }
    };
    let summary = match repo.summary() {
        Ok(summary) => summary,
        Err(err) => {
            let _ = ready_tx.send(Err(err.to_string()));
            return;
        }
    };
    let watcher = {
        let events_tx = events_tx.clone();
        Watcher::spawn(
            repo.git_dir(),
            repo.workdir(),
            Duration::from_millis(250),
            move |set| {
                let _ = events_tx.unbounded_send(Event::Changed(set));
            },
        )
        .ok()
    };
    let _ = ready_tx.send(Ok(summary));
    let operations = Operations::start(&path, events_tx.clone());
    let reads = Reads::default();

    let mut state = SessionState {
        ctx: CommandContext {
            journal: Some(bitbraid_engine::Journal::load(&repo)),
            repo: Some(repo),
            config_dir: config_dir(),
            ..Default::default()
        },
        registry: composed_registry(),
        graphs: graphs::Graphs::new(&path, events_tx.clone()),
        dirty: false,
        refs_just_read: None,
        _watcher: watcher,
    };
    for envelope in rx {
        let name = request_name(&envelope.request);
        if let Request::CancelReads(command) = &envelope.request {
            let stopped = reads.stop(command);
            envelope
                .reply
                .send(Reply::Command(serde_json::json!({ "stopped": stopped })));
            continue;
        }
        let Some(envelope) = read_elsewhere(&path, &reads, envelope) else {
            continue;
        };
        let Some(Envelope {
            request,
            asked,
            reply: reply_tx,
        }) = operations.take(envelope)
        else {
            continue;
        };
        let started = std::time::Instant::now();
        // In flight until it has answered: a quit waits a moment for it,
        // so that what it did is in the activity log.
        let busy = RUNNING.begin();
        // A change to the repository holds up what is asked after it: the
        // window says so when it takes a while.
        let changing = changing(&state.registry, &request);
        if changing.is_some() {
            let _ = events_tx.unbounded_send(Event::Busy(changing.clone()));
        }
        let reply = {
            // What nobody asked for and only reads stays on this machine.
            let _here =
                stays_here(&state.registry, &request, asked).then(bitbraid_git::exe::local_only);
            // A read answered here is tried again on the repository opened
            // anew, should the handle have no room for the packs git added
            // since it was opened; the engine sees to its commands itself.
            let again = (!matches!(request, Request::Command { .. })).then(|| request.clone());
            let reply = handle(&mut state, request);
            match (again, reply) {
                (Some(again), Reply::Error(said))
                    if bitbraid_git::repo::is_out_of_room(&said) && reopened(&mut state) =>
                {
                    handle(&mut state, again)
                }
                (_, reply) => reply,
            }
        };
        drop(busy);
        if changing.is_some() {
            let _ = events_tx.unbounded_send(Event::Busy(None));
        }
        debug(|| {
            format!(
                "handled {name} -> {} in {:?}",
                reply_name(&reply),
                started.elapsed()
            )
        });
        if !reply_tx.send(reply) {
            debug(|| format!("reply to {name} had no receiver"));
        }
    }
}

/// A file with a side larger than this is not compared, and so not laid
/// out, until that is asked for ("Load anyway"): 2 MiB. One of that size
/// rewritten whole takes about half a second, and the time grows with the
/// size.
pub const DIFF_GUARD_BYTES: u64 = 2 * 1024 * 1024;

/// Read commands slow enough to stall everything queued behind them: the
/// history of a file takes most of a second on a big repository. They touch
/// nothing the session keeps, so each runs on a thread of its own with its
/// own repository handle.
const SLOW_READS: &[&str] = &[
    "repo.history",
    "repo.blame",
    // One `git status` for each worktree.
    "worktree.list",
    "submodule.list",
    "submodule.changes",
    // As long as git's own log takes over the same history.
    "search.commits",
    "search.content",
    // Every file of a commit's tree.
    "search.files",
    // One `git status` for each worktree that is looked at.
    "worktree.changes",
    "sign.verify",
    // A merge in memory, for every local branch that is looked ahead at.
    "branch.predict",
    // A question to the LFS server.
    "lfs.locks",
    // Several questions to git each, asked with every reload.
    "flow.status",
    "branch.overview",
    "lfs.status",
    // Every object counted, and history walked for the largest.
    "repo.size",
    // A few questions to git, and every file of the index counted when
    // the checkout is sparse.
    "sparse.status",
    // A walk of the reflog and of recent history, asked when a tab opens.
    "commit.messages",
    "commit.trailer_keys",
    // A commit against the working tree: every file that differs.
    "repo.diff",
];

/// The slow reads of a session that are running, by the command they
/// run: the flags that stop them. A read has a flag of its own, which its
/// command looks at as it goes (and which ends the git process it waits
/// for); it is set when the reads of that command are stopped, and when
/// the application is quit.
#[derive(Debug, Default)]
struct Reads(std::sync::Mutex<Vec<(String, std::sync::Weak<std::sync::atomic::AtomicBool>)>>);

impl Reads {
    /// A read of `command` begins: the flag that stops it.
    fn begin(&self, command: &str) -> Arc<std::sync::atomic::AtomicBool> {
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if let Ok(mut running) = self.0.lock() {
            // Reads that have ended hold their flag no more.
            running.retain(|(_, known)| known.strong_count() > 0);
            running.push((command.to_owned(), Arc::downgrade(&flag)));
        }
        flag
    }

    /// Stop every read of `command` that is running; how many there were.
    fn stop(&self, command: &str) -> usize {
        let Ok(mut running) = self.0.lock() else {
            return 0;
        };
        let mut stopped = 0usize;
        running.retain(|(known, flag)| {
            let Some(flag) = flag.upgrade() else {
                return false;
            };
            if known == command {
                flag.store(true, std::sync::atomic::Ordering::SeqCst);
                stopped = stopped.saturating_add(1);
                return false;
            }
            true
        });
        stopped
    }
}

/// Run a slow read on its own thread, where it can be stopped. Gives the
/// request back when it is not one, or when no thread could be started.
fn read_elsewhere(path: &Path, reads: &Reads, envelope: Envelope) -> Option<Envelope> {
    let Request::Command { id, params } = &envelope.request else {
        return Some(envelope);
    };
    if !SLOW_READS.contains(&id.as_str()) {
        return Some(envelope);
    }
    let (id, params, path) = (id.clone(), params.clone(), path.to_path_buf());
    let asked = envelope.asked;
    let Envelope {
        request,
        reply: reply_tx,
        ..
    } = envelope;
    // Known before the thread runs, so that a stop that comes right after
    // the request finds it.
    let cancel = reads.begin(&id);
    RUNNING.watch(&cancel);
    // The sender goes to the thread; a failed spawn hands it back.
    let slot = std::sync::Arc::new(std::sync::Mutex::new(Some(reply_tx)));
    let for_thread = slot.clone();
    let spawned = thread::Builder::new()
        // not shown: the thread's name
        .name(format!("bitbraid-read {id}"))
        .spawn(move || {
            let Some(reply_tx) = for_thread.lock().ok().and_then(|mut slot| slot.take()) else {
                return;
            };
            // In flight until it has answered; none starts once the
            // application is being quit.
            let Some(_busy) = RUNNING.begin() else {
                // not shown: the engine's words for a stop, which are looked for
                reply_tx.send(Reply::Error("operation: cancelled".to_owned()));
                return;
            };
            let started = std::time::Instant::now();
            let registry = composed_registry();
            let mut ctx = CommandContext {
                cancel,
                config_dir: config_dir(),
                ..Default::default()
            };
            let opened = registry.run("repo.open", &mut ctx, &serde_json::json!({ "path": path }));
            // What nobody asked for and only reads stays on this machine.
            let here = !asked && reads_only(&registry, &id, &params);
            let read = |ctx: &mut CommandContext| {
                if here {
                    registry.run_unasked(&id, ctx, &params)
                } else {
                    registry.run(&id, ctx, &params)
                }
            };
            let reply = match opened.and_then(|_| read(&mut ctx)) {
                Ok(output) => Reply::Command(output),
                Err(err) => Reply::Error(err.to_string()),
            };
            debug(|| format!("read {id} elsewhere in {:?}", started.elapsed()));
            reply_tx.send(reply);
        });
    match spawned {
        Ok(_) => None,
        Err(_) => {
            let reply = slot.lock().ok().and_then(|mut slot| slot.take())?;
            Some(Envelope {
                request,
                asked,
                reply,
            })
        }
    }
}

/// Whether a request is to stay on this machine (see the module's
/// documentation): nobody asked for it, and it only reads. Operations (a
/// fetch, a pull, a push: they talk to a remote because that is what was
/// asked) and commands that change the repository are the user's in any
/// case.
fn stays_here(registry: &Registry, request: &Request, asked: bool) -> bool {
    if asked {
        return false;
    }
    match request {
        Request::Operation { .. } | Request::CancelOperation => false,
        Request::Command { id, params } => reads_only(registry, id, params),
        _ => true,
    }
}

/// The label of a command that changes the repository with these
/// parameters. One that is not known fails at once.
fn changing(registry: &Registry, request: &Request) -> Option<String> {
    match request {
        Request::Command { id, params } if !reads_only(registry, id, params) => {
            registry.get(id).map(|spec| spec.label.to_owned())
        }
        _ => None,
    }
}

/// Open the session's repository again, in place of the handle that has
/// no room for the packs on disk (`bitbraid_git::repo::is_out_of_room`).
/// Whether it was.
fn reopened(state: &mut SessionState) -> bool {
    state
        .ctx
        .repo
        .as_mut()
        .is_some_and(|repo| repo.reopen().is_ok())
}

/// Whether a command only reads: it changes nothing in the repository
/// with these parameters (and one that is not known is taken for one
/// that does).
fn reads_only(registry: &Registry, id: &str, params: &serde_json::Value) -> bool {
    registry
        .access(id)
        .is_some_and(|access| !access.changes_repository(params))
}

/// A command that needs no open repository (init, clone), running on a
/// thread of its own.
pub struct Standalone {
    /// The command's result.
    pub reply: oneshot::Receiver<Reply>,
    /// What git reports while it runs.
    pub progress: fmpsc::UnboundedReceiver<bitbraid_git::Progress>,
    /// Set to stop it.
    pub cancel: Arc<std::sync::atomic::AtomicBool>,
}

/// Start a command that needs no open repository, as the user asked for
/// it: a clone, a fetch of every repository of a workspace.
pub fn run_standalone(id: &str, params: serde_json::Value) -> Standalone {
    standalone(id, params, true)
}

/// Start a command that needs no open repository and that nobody asked
/// for: a look over the repositories of a list that keeps it current. It
/// stays on this machine (see the module's documentation).
pub fn run_standalone_unasked(id: &str, params: serde_json::Value) -> Standalone {
    standalone(id, params, false)
}

fn standalone(id: &str, params: serde_json::Value, asked: bool) -> Standalone {
    let (reply_tx, reply) = oneshot::channel();
    let reply_tx = ReplySender::new(reply_tx);
    let (progress_tx, progress) = fmpsc::unbounded();
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    RUNNING.watch(&cancel);
    let (command, flag) = (id.to_owned(), cancel.clone());
    let spawned = thread::Builder::new()
        // not shown: the thread's name
        .name(format!("bitbraid-standalone {id}"))
        .spawn(move || {
            let Some(_busy) = RUNNING.begin() else {
                // not shown: the engine's words for a stop, which are looked for
                reply_tx.send(Reply::Error("operation: cancelled".to_owned()));
                return;
            };
            let registry = composed_registry();
            let mut ctx = CommandContext {
                cancel: flag,
                config_dir: config_dir(),
                ..Default::default()
            };
            ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
                let _ = progress_tx.unbounded_send(progress);
            }));
            let started = std::time::Instant::now();
            let ran = if asked {
                registry.run(&command, &mut ctx, &params)
            } else {
                registry.run_unasked(&command, &mut ctx, &params)
            };
            let reply = match ran {
                Ok(output) => Reply::Command(output),
                Err(err) => Reply::Error(err.to_string()),
            };
            debug(|| {
                format!(
                    "standalone {command} -> {} in {:?}",
                    reply_name(&reply),
                    started.elapsed()
                )
            });
            reply_tx.send(reply);
        });
    if spawned.is_err() {
        debug(|| "no thread for a standalone command; it will fail".to_owned());
    }
    Standalone {
        reply,
        progress,
        cancel,
    }
}

/// The thread that runs operations: commands that talk to a remote or run
/// for a while. It has its own repository handle and takes them one at a
/// time, in the order they were asked for, while the session thread goes
/// on answering reads.
struct Operations {
    queue: mpsc::Sender<(String, serde_json::Value, ReplySender)>,
    cancel: Arc<std::sync::atomic::AtomicBool>,
}

impl Operations {
    fn start(path: &Path, events: fmpsc::UnboundedSender<Event>) -> Self {
        let (queue, waiting) = mpsc::channel::<(String, serde_json::Value, ReplySender)>();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        RUNNING.watch(&cancel);
        let (path, flag) = (path.to_path_buf(), cancel.clone());
        let spawned = thread::Builder::new()
            // not shown: the thread's name
            .name(format!("bitbraid-operations {}", path.display()))
            .spawn(move || {
                let registry = composed_registry();
                let mut ctx = CommandContext {
                    cancel: flag,
                    config_dir: config_dir(),
                    ..Default::default()
                };
                let opened = registry
                    .run("repo.open", &mut ctx, &serde_json::json!({ "path": path }))
                    .map_err(|err| err.to_string());
                for (id, params, reply_tx) in waiting {
                    if let Err(err) = &opened {
                        reply_tx.send(Reply::Error(err.clone()));
                        continue;
                    }
                    // A stop was meant for the operation before this one.
                    ctx.cancel.store(false, std::sync::atomic::Ordering::SeqCst);
                    // Unless it was the application being quit: then this
                    // one does not start. Asked after the flag was reset,
                    // so a quit in between is seen here or sets the flag.
                    let Some(_busy) = RUNNING.begin() else {
                        // not shown: the engine's words for a stop, which are looked for
                        reply_tx.send(Reply::Error("operation: cancelled".to_owned()));
                        continue;
                    };
                    let (command, events) = (id.clone(), events.clone());
                    ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
                        let _ = events.unbounded_send(Event::Progress {
                            command: command.clone(),
                            stage: progress.stage,
                            percent: progress.percent,
                        });
                    }));
                    // The session thread writes the journal too.
                    ctx.reload_journal();
                    let started = std::time::Instant::now();
                    let reply = match registry.run(&id, &mut ctx, &params) {
                        Ok(output) => Reply::Command(output),
                        Err(err) => Reply::Error(err.to_string()),
                    };
                    debug(|| {
                        format!(
                            "operation {id} -> {} in {:?}",
                            reply_name(&reply),
                            started.elapsed()
                        )
                    });
                    reply_tx.send(reply);
                }
            });
        if spawned.is_err() {
            debug(|| "no thread for operations; they will fail".to_owned());
        }
        Self { queue, cancel }
    }

    /// Queue an operation or stop the running one. Gives back any other
    /// request.
    fn take(&self, envelope: Envelope) -> Option<Envelope> {
        let Envelope {
            request,
            asked,
            reply: reply_tx,
        } = envelope;
        match request {
            Request::Operation { id, params } => {
                if let Err(mpsc::SendError((_, _, reply_tx))) =
                    self.queue.send((id, params, reply_tx))
                {
                    reply_tx.send(Reply::Error(
                        say!("session-operations-cannot-run").to_owned(),
                    ));
                }
                None
            }
            Request::CancelOperation => {
                self.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
                reply_tx.send(Reply::Command(serde_json::Value::Null));
                None
            }
            other => Some(Envelope {
                request: other,
                asked,
                reply: reply_tx,
            }),
        }
    }
}

fn handle(state: &mut SessionState, request: Request) -> Reply {
    // With the configuration of the git that is run now.
    state.ctx.follow_git();
    // Good for the graph asked for next, and for nothing after it.
    let refs_just_read = state.refs_just_read.take();
    if let Request::Command { id, params } = request {
        // The operations thread writes the journal too.
        state.ctx.reload_journal();
        return match state.registry.run(&id, &mut state.ctx, &params) {
            Ok(output) => Reply::Command(output),
            Err(err) => Reply::Error(err.to_string()),
        };
    }
    let Some(repo) = state.ctx.repo.as_ref() else {
        return Reply::Error(say!("session-no-repository").to_owned());
    };
    match request {
        Request::Command { .. }
        | Request::Operation { .. }
        | Request::CancelOperation
        | Request::CancelReads(_) => Reply::Error("unreachable".to_owned()),
        Request::DiffFile {
            old,
            new,
            path,
            whitespace,
            context,
            old_path,
            max_bytes,
        } => {
            let sides =
                (|| -> bitbraid_git::Result<(bitbraid_git::DiffSide, bitbraid_git::DiffSide)> {
                    // The parent of a root commit does not exist: everything
                    // in that commit is new.
                    let old_side = match bitbraid_git::DiffSide::parse(repo, &old) {
                        Err(_) if old.ends_with('^') => bitbraid_git::DiffSide::Empty,
                        other => other?,
                    };
                    Ok((old_side, bitbraid_git::DiffSide::parse(repo, &new)?))
                })();
            let diff = sides.and_then(|(old, new)| {
                bitbraid_git::diff::diff_file_guarded(
                    repo,
                    &old,
                    &new,
                    old_path.as_deref(),
                    &path,
                    context,
                    whitespace,
                    max_bytes,
                )
            });
            // A file that is on neither side may be one of the untracked
            // files a stash carries, which are in a commit of their own:
            // it is then new, whole.
            let diff = match diff {
                Ok(None) => match new
                    .parse::<Oid>()
                    .ok()
                    .and_then(|id| bitbraid_git::stash::untracked_part(repo, id))
                {
                    Some(untracked) => bitbraid_git::diff::diff_file_guarded(
                        repo,
                        &bitbraid_git::DiffSide::Empty,
                        &bitbraid_git::DiffSide::Commit(untracked),
                        None,
                        &path,
                        context,
                        whitespace,
                        max_bytes,
                    ),
                    None => Ok(None),
                },
                other => other,
            };
            match diff {
                Ok(diff) => Reply::DiffFile(diff),
                Err(err) => Reply::Error(err.to_string()),
            }
        }
        Request::CommitFiles(id) => {
            let options = bitbraid_git::DiffOptions {
                context: 3,
                paths: Vec::new(),
                detect_renames: true,
                whitespace: bitbraid_git::Whitespace::Exact,
                max_bytes: Some(DIFF_GUARD_BYTES),
            };
            match bitbraid_git::commit_files(repo, id, &options) {
                Ok(files) => Reply::CommitFiles(files),
                Err(err) => Reply::Error(err.to_string()),
            }
        }
        Request::RangeFiles { old, new } => {
            let old = old
                .map(bitbraid_git::DiffSide::Commit)
                .unwrap_or(bitbraid_git::DiffSide::Empty);
            match bitbraid_git::diff_trees(
                repo,
                &old,
                &bitbraid_git::DiffSide::Commit(new),
                &bitbraid_git::DiffOptions {
                    context: 3,
                    paths: Vec::new(),
                    detect_renames: true,
                    whitespace: bitbraid_git::Whitespace::Exact,
                    max_bytes: Some(DIFF_GUARD_BYTES),
                },
            ) {
                Ok(files) => Reply::CommitFiles(files),
                Err(err) => Reply::Error(err.to_string()),
            }
        }
        Request::Refs => match bitbraid_git::snapshot(repo) {
            Ok(snapshot) => {
                state.refs_just_read = Some(snapshot.clone());
                Reply::Refs(snapshot)
            }
            Err(err) => Reply::Error(err.to_string()),
        },
        Request::Graph {
            tips,
            exclude,
            pinned,
            order,
            worktree_rows,
        } => {
            let snapshot = match refs_just_read.map_or_else(|| bitbraid_git::snapshot(repo), Ok) {
                Ok(snapshot) => snapshot,
                Err(err) => return Reply::Error(err.to_string()),
            };
            let key = graphs::GraphKey {
                snapshot,
                tips,
                exclude,
                pinned,
                order,
                worktree_rows,
                dirty: state.dirty,
            };
            match state.graphs.request(repo, key) {
                Ok(graph) => Reply::Graph(graph),
                Err(err) => Reply::Error(err.to_string()),
            }
        }
        Request::Details(ids) => match bitbraid_git::details(repo, &ids) {
            Ok(details) => Reply::Details(details),
            Err(err) => Reply::Error(err.to_string()),
        },
        Request::FirstDetails(shown) => {
            let ids = state
                .graphs
                .current()
                .map(|graph| first_shown(graph, shown))
                .unwrap_or_default();
            match bitbraid_git::details(repo, &ids) {
                Ok(details) => Reply::Details(details),
                Err(err) => Reply::Error(err.to_string()),
            }
        }
        Request::Message(id) => match bitbraid_git::message(repo, id) {
            Ok(message) => Reply::Message(message),
            Err(err) => Reply::Error(err.to_string()),
        },
        // Bounded: a worktree with more changed paths than anyone reads
        // is listed up to there.
        Request::Status => match bitbraid_git::status_bounded(repo, bitbraid_git::STATUS_LIMIT) {
            Ok((status, more)) => {
                state.dirty = is_dirty(&status);
                Reply::Status(status, more)
            }
            Err(err) => Reply::Error(err.to_string()),
        },
        Request::Ancestry(id) => {
            let Some(graph) = state.graphs.current() else {
                return Reply::Ancestry(Vec::new());
            };
            let mut out = Vec::new();
            let mut current = Some(id);
            while let Some(id) = current {
                out.push(id);
                current = graph
                    .index
                    .get(&id)
                    .and_then(|ix| graph.commits.get(*ix))
                    .and_then(|c| c.parents.first().copied());
                if out.len() > 100_000 {
                    break;
                }
            }
            Reply::Ancestry(out)
        }
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::{
        sync::{Arc, atomic::AtomicBool, atomic::Ordering},
        time::Duration,
    };

    use super::Running;

    #[test]
    fn quitting_stops_what_runs_and_waits_for_it_to_end() {
        let running = Arc::new(Running::new());
        let flag = Arc::new(AtomicBool::new(false));
        running.watch(&flag);
        // A flag nobody holds any more is no trouble.
        running.watch(&Arc::new(AtomicBool::new(false)));
        let (theirs, seen) = (running.clone(), flag.clone());
        let (started, wait) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let busy = theirs.begin();
            let _ = started.send(busy.is_some());
            // An operation: it ends when its flag is set.
            while !seen.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(2));
            }
            drop(busy);
        });
        assert_eq!(wait.recv().ok(), Some(true));
        assert!(running.stop(Duration::from_secs(10)), "it ended in time");
        assert!(flag.load(Ordering::SeqCst), "the flag was set");
        assert!(worker.join().is_ok());
        // What was waiting its turn does not start any more.
        assert!(running.begin().is_none());
        assert!(
            running.stop(Duration::from_millis(50)),
            "nothing is counted twice"
        );
    }

    #[test]
    fn the_reads_of_a_command_are_stopped_and_no_others() {
        use super::Reads;
        let reads = Reads::default();
        let search = reads.begin("search.commits");
        let second = reads.begin("search.commits");
        let history = reads.begin("repo.history");
        // One that has ended holds its flag no more.
        drop(reads.begin("search.commits"));
        assert_eq!(reads.stop("search.commits"), 2);
        assert!(search.load(Ordering::SeqCst) && second.load(Ordering::SeqCst));
        assert!(!history.load(Ordering::SeqCst));
        // Stopped once: there is nothing left to stop, and a read that
        // begins afterwards is not stopped by what was asked before.
        assert_eq!(reads.stop("search.commits"), 0);
        let later = reads.begin("search.commits");
        assert!(!later.load(Ordering::SeqCst));
        assert_eq!(reads.stop("no.such"), 0);
        assert_eq!(reads.stop(""), 0);
        assert_eq!(reads.stop("repo.history"), 1);
        assert!(history.load(Ordering::SeqCst));
    }

    #[test]
    fn a_search_that_is_no_longer_wanted_is_stopped() {
        use bitbraid_testkit::Scratch;
        use futures::executor::block_on;

        use super::{Reply, Request, SessionHandle};
        let scratch = Scratch::of("search").unwrap();
        let (session, _events) = SessionHandle::open(scratch.path()).unwrap();
        let search = |query: &str| Request::Command {
            id: "search.commits".to_owned(),
            params: serde_json::json!({ "query": query, "zone": "UTC" }),
        };
        // Asked and stopped right away: the session takes the two in
        // turn, so the stop finds the read before its command begins.
        let unwanted = session.send(search("change:needle"));
        let stop = session.send(Request::CancelReads("search.commits".to_owned()));
        match block_on(stop) {
            Ok(Reply::Command(value)) => assert_eq!(value["stopped"], 1),
            other => panic!("the stop was not answered: {other:?}"),
        }
        match block_on(unwanted) {
            Ok(Reply::Error(err)) => assert!(err.contains("cancelled"), "{err}"),
            other => panic!("the search was not stopped: {other:?}"),
        }
        // Nothing is running: nothing to stop. And the next search is
        // answered as if nothing had been.
        match block_on(session.send(Request::CancelReads("search.commits".to_owned()))) {
            Ok(Reply::Command(value)) => assert_eq!(value["stopped"], 0),
            other => panic!("{other:?}"),
        }
        let want = scratch
            .git()
            .lines(["log", "--all", "--format=%H", "-Sneedle"])
            .unwrap();
        match block_on(session.send(search("change:needle"))) {
            Ok(Reply::Command(value)) => {
                let mut ids: Vec<String> = value["ids"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|id| id.as_str().unwrap().to_owned())
                    .collect();
                let mut want = want.clone();
                ids.sort();
                want.sort();
                assert_eq!(ids, want);
                assert!(!ids.is_empty());
            }
            other => panic!("the search after it failed: {other:?}"),
        }
        // A stop for another command leaves a search alone.
        let kept = session.send(search("needle"));
        let _ = block_on(session.send(Request::CancelReads("repo.history".to_owned())));
        assert!(matches!(block_on(kept), Ok(Reply::Command(_))));
    }

    #[test]
    fn a_session_is_started_without_waiting_and_answers_what_was_queued() {
        use bitbraid_testkit::Scratch;
        use futures::executor::block_on;

        use super::{Opening, Preloaded, Reply, Request, Shown};
        let scratch = Scratch::of("linear").unwrap();
        // Asked before the repository is open, answered once it is.
        let opening = Opening::start(scratch.path()).unwrap();
        let status = opening.send(Request::Status);
        let refs = opening.send(Request::Refs);
        let (session, _events) = opening.wait().unwrap();
        assert_eq!(session.path, scratch.path());
        assert!(session.summary.head.is_some());
        assert!(matches!(block_on(status), Ok(Reply::Status(..))));
        assert!(matches!(block_on(refs), Ok(Reply::Refs(_))));
        // And asked afterwards, as ever.
        assert!(matches!(
            block_on(session.send(Request::Status)),
            Ok(Reply::Status(..))
        ));

        // What is no repository: the wait says so, and what was queued is
        // dropped unanswered rather than left waiting.
        let nowhere = scratch.path().join("no-such-directory");
        let opening = Opening::start(&nowhere).unwrap();
        let unanswered = opening.send(Request::Status);
        assert!(opening.wait().is_err());
        assert!(block_on(unanswered).is_err());

        // Ahead of the window: handed over open, once; what did not open
        // is not handed over, so that the shell opens it and reports why.
        let mut ahead = Preloaded::default();
        ahead.open(scratch.path(), None, Shown::default());
        ahead.open(scratch.path(), None, Shown::default());
        ahead.open(&nowhere, None, Shown::default());
        let preload = ahead.take(scratch.path()).unwrap();
        assert!(preload.replies.is_none());
        assert!(matches!(
            block_on(preload.session.send(Request::Refs)),
            Ok(Reply::Refs(_))
        ));
        assert!(ahead.take(scratch.path()).is_none());
        assert!(ahead.take(&nowhere).is_none());
    }

    #[test]
    fn the_details_of_the_first_rows_are_read_with_the_first_graph() {
        use bitbraid_git::{Oid, Order, Tips};
        use bitbraid_testkit::{CorpusSpec, build_synthetic};
        use futures::executor::block_on;

        use super::{GraphData, Preloaded, Reply, Request, SessionHandle, Shown, first_shown};
        let dir = build_synthetic(&CorpusSpec {
            commits: 300,
            lanes: 3,
            merge_every: 7,
            files: 10,
        })
        .unwrap();
        let graph = || Request::Graph {
            tips: Tips::All,
            exclude: Vec::new(),
            pinned: Vec::new(),
            order: Order::Date,
            worktree_rows: Vec::new(),
        };
        let (session, _events) = SessionHandle::open(&dir).unwrap();
        let Ok(Reply::Graph(shown)) = block_on(session.send(graph())) else {
            panic!("no graph");
        };
        let rows: Vec<Oid> = shown.commits.iter().map(|commit| commit.id).collect();
        assert_eq!(rows.len(), 300);
        let read_ahead = |shown: Shown| -> Vec<Oid> {
            let mut ahead = Preloaded::default();
            ahead.open(&dir, Some(graph()), shown);
            let replies = ahead.take(&dir).unwrap().replies.unwrap();
            assert!(matches!(block_on(replies.graph), Ok(Reply::Graph(_))));
            let Ok(Reply::Details(details)) = block_on(replies.details) else {
                panic!("no details");
            };
            details.iter().map(|commit| commit.id).collect()
        };
        // The rows in these ranges, once each, in order.
        let of = |ranges: &[std::ops::Range<usize>]| -> Vec<Oid> {
            let mut at: Vec<usize> = ranges.iter().flat_map(|range| range.clone()).collect();
            at.sort_unstable();
            at.dedup();
            at.into_iter()
                .filter_map(|ix| rows.get(ix).copied())
                .collect()
        };
        let head = shown.index[&shown.head.unwrap()];
        let around_head = head.saturating_sub(48)..head + 48;
        // Opened where nothing was remembered: from the first row, as many
        // as a tall window shows and more, and around HEAD, which is
        // selected and which the list is centred on.
        assert_eq!(
            read_ahead(Shown::default()),
            of(&[0..96, around_head.clone()])
        );
        // From the commit that was at the top of the list.
        let at_top = Shown {
            top: Some(rows[150]),
            selected: Some(rows[20]),
        };
        assert_eq!(read_ahead(at_top), rows[150..246].to_vec());
        // Without one, the list is centred on the selected commit, and the
        // first rows are read as well.
        let selected = Shown {
            top: None,
            selected: Some(rows[200]),
        };
        let mut wanted = rows[..96].to_vec();
        wanted.extend_from_slice(&rows[152..248]);
        assert_eq!(read_ahead(selected), wanted);
        // A top that is no longer in the graph counts for nothing; a
        // selected commit that is not selects the first row, as the graph
        // does; at the end of the history there are fewer rows to read.
        let gone = Shown::remembered(Some(&"0".repeat(40)), Some("not an id"));
        assert!(gone.top.is_some() && gone.selected.is_none());
        assert_eq!(read_ahead(gone), of(&[0..96, around_head]));
        let not_there = Shown {
            top: None,
            selected: gone.top,
        };
        assert_eq!(read_ahead(not_there), rows[..96].to_vec());
        let at_end = Shown {
            top: Some(rows[280]),
            selected: None,
        };
        assert_eq!(read_ahead(at_end), rows[280..].to_vec());

        // HEAD further down, behind newer commits of other branches: with
        // nothing remembered the list is centred on it, and its rows are
        // read with the first ones.
        let moved = GraphData {
            commits: shown.commits.clone(),
            rows: shown.rows.clone(),
            wip: shown.wip.clone(),
            index: shown.index.clone(),
            child: shown.child.clone(),
            badges: shown.badges.clone(),
            worktrees: shown.worktrees.clone(),
            worktree_rows: shown.worktree_rows.clone(),
            upstreams: shown.upstreams.clone(),
            head: Some(rows[120]),
            max_width: shown.max_width,
            order: shown.order,
            walked: shown.walked,
            generation: shown.generation,
        };
        let mut wanted = rows[..96].to_vec();
        wanted.extend_from_slice(&rows[96..168]);
        assert_eq!(first_shown(&moved, Shown::default()), wanted);
        // A row that stands for another worktree's uncommitted work is no
        // commit, and is not read.
        let with_row = GraphData {
            worktree_rows: [(rows[3], 0)].into_iter().collect(),
            ..moved
        };
        let ids = first_shown(&with_row, Shown::default());
        assert_eq!(ids.len(), 167);
        assert!(!ids.contains(&rows[3]));
    }

    #[test]
    fn a_quit_does_not_hang_on_what_does_not_end() {
        let running = Running::new();
        let stuck = running.begin();
        assert!(stuck.is_some());
        assert!(!running.stop(Duration::from_millis(40)));
        drop(stuck);
        assert!(running.stop(Duration::from_millis(40)));
    }
}
