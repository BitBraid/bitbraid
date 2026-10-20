//! The graphs a session hands out: built on the session thread when the
//! history is walked in one go, and for a large history in pieces, the
//! first by the session thread and the rest by a thread of its own, while
//! the session goes on answering.
//!
//! A piece holds the leading rows of the graph and is laid out as the
//! whole will be (see `bitbraid_git::pieces`), so rows that are on screen
//! stay where they are when more arrives. Pieces and the whole graph reach
//! the views as events, each with the number of the request it belongs to:
//! a view takes none that is older than the graph it shows.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use bitbraid_git::{
    GraphCommit, LogOptions, Oid, Order, Repository, Snapshot, Tips,
    pieces::{self, First},
};
use futures::channel::mpsc as fmpsc;

use super::{Badge, BadgeKind, Event, GraphData, RUNNING, WorktreeMark, debug};

/// Everything a graph is built from. The commits behind a reference never
/// change, so equal keys mean an equal graph.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct GraphKey {
    pub snapshot: Snapshot,
    pub tips: Tips,
    pub exclude: Vec<String>,
    pub pinned: Vec<String>,
    pub order: Order,
    pub worktree_rows: Vec<String>,
    pub dirty: bool,
}

/// How much of the history the commits of a graph are.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Extent {
    /// All of it.
    Whole,
    /// Its leading rows, with this many commits walked so far.
    Piece(usize),
}

impl GraphKey {
    /// Whether a reference is hidden: by its own name, or as a branch of
    /// a remote that is hidden as a whole.
    fn hidden(&self, full: &str) -> bool {
        self.exclude.iter().any(|e| e == full)
            || crate::views::refs::under_remote(&self.exclude, full)
    }

    /// Where the walk starts and how it goes: the tips with what is hidden
    /// left out. A pinned branch is not walked for being pinned: hidden,
    /// it is out of the graph like any other.
    pub(super) fn walk_options(&self) -> LogOptions {
        let snapshot = &self.snapshot;
        let tips = match self.tips.clone() {
            Tips::All if !self.exclude.is_empty() => {
                let excluded = |full: &str| self.hidden(full);
                // Each commit once, in the order met: with thousands of
                // references a list alone would be searched for every one.
                let mut ids: Vec<Oid> = Vec::new();
                let mut seen = std::collections::HashSet::new();
                let mut add = |id: Oid| {
                    if seen.insert(id) {
                        ids.push(id);
                    }
                };
                if let Some(head) = snapshot.head.target {
                    add(head);
                }
                for branch in &snapshot.branches {
                    if !excluded(&branch.full_name) {
                        add(branch.target);
                    }
                }
                for remote in &snapshot.remotes {
                    if excluded(&format!("refs/remotes/{}", remote.name)) {
                        continue;
                    }
                    for branch in &remote.branches {
                        if !excluded(&branch.full_name) {
                            add(branch.target);
                        }
                    }
                }
                for tag in &snapshot.tags {
                    if !excluded(&format!("refs/tags/{}", tag.name)) {
                        add(tag.target);
                    }
                }
                Tips::Ids(ids)
            }
            // Everything: where the snapshot found a walk of everything
            // starts, rather than every reference read again.
            Tips::All if !snapshot.walk_tips.is_empty() => Tips::Ids(snapshot.walk_tips.clone()),
            other => other,
        };
        LogOptions {
            tips,
            limit: None,
            first_parent: false,
            order: self.order,
        }
    }

    /// The labels of the graph: what points at which commit.
    fn badges(&self) -> HashMap<Oid, Vec<Badge>> {
        let snapshot = &self.snapshot;
        let mut badges: HashMap<Oid, Vec<Badge>> = HashMap::new();
        let mut push = |id: Oid, badge: Badge| badges.entry(id).or_default().push(badge);
        if let Some(head) = snapshot.head.target {
            let name = snapshot
                .head
                .branch
                .clone()
                .unwrap_or_else(|| "HEAD".to_owned());
            push(
                head,
                Badge {
                    kind: BadgeKind::Head,
                    name,
                    full_name: None,
                },
            );
        }
        // What is hidden has no label either, also on a commit that stays
        // in the graph because something else reaches it. A remote can be
        // hidden as a whole.
        for branch in &snapshot.branches {
            if branch.is_head || self.hidden(&branch.full_name) {
                continue; // already shown as HEAD, or not shown
            }
            push(
                branch.target,
                Badge {
                    kind: BadgeKind::Local,
                    name: branch.name.clone(),
                    full_name: Some(branch.full_name.clone()),
                },
            );
        }
        for remote in &snapshot.remotes {
            for branch in &remote.branches {
                if self.hidden(&branch.full_name) {
                    continue;
                }
                push(
                    branch.target,
                    Badge {
                        kind: BadgeKind::Remote,
                        name: format!("{}/{}", remote.name, branch.name),
                        full_name: Some(branch.full_name.clone()),
                    },
                );
            }
        }
        for tag in &snapshot.tags {
            let full_name = format!("refs/tags/{}", tag.name);
            if self.hidden(&full_name) {
                continue;
            }
            push(
                tag.target,
                Badge {
                    kind: BadgeKind::Tag,
                    name: tag.name.clone(),
                    full_name: Some(full_name),
                },
            );
        }
        for stash in &snapshot.stashes {
            push(
                stash.target,
                Badge {
                    kind: BadgeKind::Stash,
                    name: format!("stash@{{{}}}", stash.index),
                    full_name: None,
                },
            );
        }
        badges
    }

    /// The graph for `commits`, which are in graph order and are the whole
    /// history or its leading rows.
    pub(super) fn graph(
        &self,
        repo: &Repository,
        mut commits: Vec<GraphCommit>,
        extent: Extent,
        generation: u64,
    ) -> GraphData {
        let snapshot = &self.snapshot;
        let order = self.order;
        let piece = matches!(extent, Extent::Piece(_));
        // The commits the pinned branches are at, in pin order. A name that
        // is no branch any more pins nothing, and in a piece neither does
        // one that is hidden: its tip will not come.
        let pins: Vec<Oid> = self
            .pinned
            .iter()
            .take(bitbraid_git::MAX_PINNED)
            .filter(|full| !piece || !self.exclude.contains(*full))
            .filter_map(|full| {
                snapshot
                    .branches
                    .iter()
                    .find(|branch| branch.full_name == *full)
                    .map(|branch| branch.target)
                    .or_else(|| {
                        snapshot
                            .remotes
                            .iter()
                            .flat_map(|remote| remote.branches.iter())
                            .find(|branch| branch.full_name == *full)
                            .map(|branch| branch.target)
                    })
            })
            .collect();

        // A row per stash, hidden like any other reference.
        let stashes: Vec<Oid> = snapshot
            .stashes
            .iter()
            .filter(|stash| {
                let name = format!("stash@{{{}}}", stash.index);
                !self.exclude.contains(&name)
            })
            .map(|stash| stash.target)
            .collect();
        if piece && order == Order::Date && !stashes.is_empty() {
            pieces::hold_back_for_stashes(repo, &mut commits, &stashes);
        }
        let commits = bitbraid_git::with_stashes_in(repo, commits, &stashes, order);

        // The other worktrees, and a row for each that has uncommitted work.
        let worktrees: Vec<WorktreeMark> = snapshot
            .worktrees
            .iter()
            .filter(|worktree| !worktree.current)
            .filter_map(|worktree| {
                Some(WorktreeMark {
                    path: worktree.path.as_ref()?.display().to_string(),
                    head: worktree.head?,
                    branch: worktree.branch.clone(),
                })
            })
            .collect();
        let mut worktree_ids: HashMap<Oid, usize> = HashMap::new();
        let mut wanted: Vec<(Oid, Oid)> = Vec::new();
        for path in self
            .worktree_rows
            .iter()
            .take(bitbraid_git::wip::MAX_SCANNED)
        {
            if let Some(at) = worktrees.iter().position(|mark| mark.path == *path)
                && let Some(mark) = worktrees.get(at)
            {
                let id = bitbraid_git::wip::row_id(Path::new(path));
                if worktree_ids.insert(id, at).is_none() {
                    wanted.push((id, mark.head));
                }
            }
        }
        let commits = bitbraid_git::wip::with_rows(commits, &wanted);

        let head = snapshot.head.target;
        let (wip, rows) = match (self.dirty, piece) {
            (dirty, true) => bitbraid_git::layout_piece(&commits, dirty.then_some(head), &pins),
            (true, false) => {
                let (wip, rows) = bitbraid_git::layout_with_wip_pinned(&commits, head, &pins);
                (Some(wip), rows)
            }
            (false, false) => (None, bitbraid_git::layout_pinned(&commits, &pins)),
        };
        let index: HashMap<Oid, usize> =
            commits.iter().enumerate().map(|(i, c)| (c.id, i)).collect();
        // Children come before their parents, so the last child to claim a
        // parent is the nearest one above it.
        let mut child = vec![u32::MAX; commits.len()];
        for (row, commit) in commits.iter().enumerate() {
            // Uncommitted work is on no branch that could be the nearest
            // one, and neither is a stash (its row has one parent; a stash
            // commit a branch points at has all of its own).
            if !worktree_ids.is_empty() && worktree_ids.contains_key(&commit.id) {
                continue;
            }
            if commit.parents.len() == 1 && stashes.contains(&commit.id) {
                continue;
            }
            for parent in &commit.parents {
                if let Some(slot) = index.get(parent).and_then(|ix| child.get_mut(*ix)) {
                    *slot = u32::try_from(row).unwrap_or(u32::MAX);
                }
            }
        }
        let max_width = rows
            .iter()
            .chain(wip.as_ref())
            .map(|r| r.width)
            .max()
            .unwrap_or(1);
        // Only the rows that made it into the graph are rows.
        worktree_ids.retain(|id, _| index.contains_key(id));
        GraphData {
            commits,
            rows,
            wip,
            index,
            child,
            badges: self.badges(),
            worktrees,
            worktree_rows: worktree_ids,
            upstreams: snapshot
                .branches
                .iter()
                .filter_map(|branch| Some((branch.name.clone(), branch.upstream.clone()?)))
                .collect(),
            head,
            max_width,
            order,
            walked: match extent {
                Extent::Whole => None,
                Extent::Piece(walked) => Some(walked),
            },
            generation,
        }
    }
}

/// Whether two graphs are of the same commits in the same order: the same
/// walk, whatever is added to its rows and however they are laid out.
fn same_walk(a: &GraphKey, b: &GraphKey) -> bool {
    a.tips == b.tips
        && a.exclude == b.exclude
        && a.order == b.order
        && a.snapshot.head.target == b.snapshot.head.target
        && a.snapshot.branches == b.snapshot.branches
        && a.snapshot.remotes == b.snapshot.remotes
        && a.snapshot.tags == b.snapshot.tags
}

/// The commits a whole graph was walked as: its rows without the ones that
/// were added for stashes and for the uncommitted work of other worktrees.
fn walked_commits(graph: &GraphData, key: &GraphKey) -> Vec<GraphCommit> {
    let stashes: Vec<Oid> = key
        .snapshot
        .stashes
        .iter()
        .map(|stash| stash.target)
        .collect();
    graph
        .commits
        .iter()
        .filter(|commit| !graph.worktree_rows.contains_key(&commit.id))
        // The row of a stash has one parent; a stash commit the walk came
        // to by a branch has all of its own, which are two or three.
        .filter(|commit| commit.parents.len() != 1 || !stashes.contains(&commit.id))
        .cloned()
        .collect()
}

/// A graph the worker has built, with the number of the request it is for.
type Built = Arc<Mutex<Option<(u64, Arc<GraphData>)>>>;

/// The thread that walks a large history to its end.
struct Worker {
    /// The number of a request, the size of its first piece, and what its
    /// graph is built from.
    jobs: mpsc::Sender<(u64, usize, GraphKey)>,
    /// The request that counts: a walk for an older one stops.
    newest: Arc<AtomicU64>,
    /// The last graph it built.
    built: Built,
}

impl Worker {
    fn start(path: &Path, events: fmpsc::UnboundedSender<Event>) -> Self {
        let (jobs, waiting) = mpsc::channel::<(u64, usize, GraphKey)>();
        let newest = Arc::new(AtomicU64::new(0));
        let built: Built = Arc::new(Mutex::new(None));
        // Set when the application is quit.
        let stop = Arc::new(AtomicBool::new(false));
        RUNNING.watch(&stop);
        let (path, counts, slot) = (path.to_path_buf(), newest.clone(), built.clone());
        let spawned = thread::Builder::new()
            // not shown: the thread's name
            .name(format!("bitbraid-graph {}", path.display()))
            .spawn(move || walk_jobs(&path, &waiting, &counts, &slot, &stop, &events));
        if spawned.is_err() {
            debug(|| "no thread to walk a large history on; its first rows stay".to_owned());
        }
        Self {
            jobs,
            newest,
            built,
        }
    }
}

/// The worker's loop: one history after the other, the newest request only.
fn walk_jobs(
    path: &Path,
    waiting: &mpsc::Receiver<(u64, usize, GraphKey)>,
    newest: &AtomicU64,
    built: &Built,
    stop: &AtomicBool,
    events: &fmpsc::UnboundedSender<Event>,
) {
    // A graph keeps a view current, which nobody asks for: what is read
    // for it (the rows of the other worktrees among it) stays on this
    // machine.
    let _here = bitbraid_git::exe::local_only();
    // Its own handle: a repository does not cross threads. Opened when
    // there is a first history to walk.
    let mut repo: Option<Repository> = None;
    for (generation, first, key) in waiting {
        let go_on = || newest.load(Ordering::SeqCst) == generation && !stop.load(Ordering::SeqCst);
        if !go_on() {
            continue;
        }
        let Some(_busy) = RUNNING.begin() else {
            continue;
        };
        // Anew when another git was chosen since: its configuration is
        // the one to read with.
        if !repo.as_ref().is_some_and(Repository::follows_git) {
            repo = Repository::open(path).ok();
        }
        let Some(handle) = repo.as_ref() else {
            let _ = events.unbounded_send(Event::GraphFailed {
                generation,
                error: say!("session-cannot-reopen").to_owned(),
            });
            continue;
        };
        let publish = |graph: GraphData| {
            let graph = Arc::new(graph);
            if let Ok(mut slot) = built.lock() {
                *slot = Some((generation, graph.clone()));
            }
            let _ = events.unbounded_send(Event::Graph(graph));
        };
        let mut walked = walk_once(handle, &key, first, generation, &go_on, &publish);
        // Git added more packs since the handle was opened than it has
        // room for: opened again, it walks once more.
        if walked
            .as_ref()
            .is_err_and(|err| bitbraid_git::repo::is_out_of_room(&err.to_string()))
            && let Some(open) = repo.as_mut()
            && open.reopen().is_ok()
        {
            walked = walk_once(open, &key, first, generation, &go_on, &publish);
        }
        if let Err(err) = walked {
            let _ = events.unbounded_send(Event::GraphFailed {
                generation,
                error: err.to_string(),
            });
        }
    }
}

/// Walk one history and hand out its graph: in pieces as it goes, and
/// whole at the end unless it was stopped. An error is the walk's.
fn walk_once(
    repo: &Repository,
    key: &GraphKey,
    first: usize,
    generation: u64,
    go_on: &dyn Fn() -> bool,
    publish: &dyn Fn(GraphData),
) -> bitbraid_git::Result<()> {
    let started = std::time::Instant::now();
    let walked = pieces::walk_in_pieces(
        repo,
        &key.walk_options(),
        first,
        go_on,
        &mut |rows, walked| {
            debug(|| format!("graph piece: {} rows of {walked} walked", rows.len()));
            publish(key.graph(repo, rows, Extent::Piece(walked), generation));
        },
    )?;
    match walked {
        Some(commits) => {
            let graph = key.graph(repo, commits, Extent::Whole, generation);
            debug(|| {
                format!(
                    "graph whole: {} rows in {:?}",
                    graph.commits.len(),
                    started.elapsed()
                )
            });
            if go_on() {
                publish(graph);
            }
        }
        None => debug(|| format!("graph walk {generation} stopped")),
    }
    Ok(())
}

/// The graphs of one session.
pub(super) struct Graphs {
    /// The last whole graph and what it was built from. A reload that
    /// finds the references where they were hands the same graph out again.
    whole: Option<(GraphKey, Arc<GraphData>)>,
    /// A history that is still being walked: what its graph is built from,
    /// the number of its request and the piece handed out for it.
    walking: Option<(GraphKey, u64, Arc<GraphData>)>,
    /// The number of the last request that built anything.
    generation: u64,
    /// How many commits are walked for the first piece of a history.
    first: usize,
    path: PathBuf,
    events: fmpsc::UnboundedSender<Event>,
    /// Started when the first large history comes.
    worker: Option<Worker>,
}

impl Graphs {
    pub(super) fn new(path: &Path, events: fmpsc::UnboundedSender<Event>) -> Self {
        Self {
            whole: None,
            walking: None,
            generation: 0,
            first: pieces::FIRST_PIECE,
            path: path.to_path_buf(),
            events,
            worker: None,
        }
    }

    /// The graph handed out last.
    pub(super) fn current(&self) -> Option<&Arc<GraphData>> {
        self.walking
            .as_ref()
            .map(|(_, _, graph)| graph)
            .or(self.whole.as_ref().map(|(_, graph)| graph))
    }

    /// Take what the worker has built for the history that is being
    /// walked: a larger piece, or the whole, which ends the walking.
    fn catch_up(&mut self) {
        let Some((key, generation, latest)) = self.walking.as_mut() else {
            return;
        };
        let built = self
            .worker
            .as_ref()
            .and_then(|worker| worker.built.lock().ok()?.clone())
            .filter(|(of, _)| *of == *generation)
            .map(|(_, graph)| graph);
        let Some(graph) = built else { return };
        if graph.walked.is_none() {
            self.whole = Some((key.clone(), graph));
            self.walking = None;
        } else {
            *latest = graph;
        }
    }

    /// The graph for `key`: the one at hand when nothing it is built from
    /// has changed, else a new one, whole, or the first piece of a large
    /// history with the rest on its way as events.
    pub(super) fn request(
        &mut self,
        repo: &Repository,
        key: GraphKey,
    ) -> bitbraid_git::Result<Arc<GraphData>> {
        self.catch_up();
        if let Some((known, graph)) = &self.whole
            && *known == key
        {
            let graph = graph.clone();
            // Whatever is being walked was asked for before this.
            self.stop_walking();
            return Ok(graph);
        }
        if let Some((known, _, latest)) = &self.walking
            && *known == key
        {
            return Ok(latest.clone());
        }
        self.stop_walking();
        self.generation = self.generation.wrapping_add(1).max(1);
        let generation = self.generation;
        // The same commits, shown otherwise (uncommitted work came or went,
        // a branch was pinned, a worktree got a row): laid out again
        // without another walk.
        if let Some((known, graph)) = &self.whole
            && same_walk(known, &key)
        {
            let commits = walked_commits(graph, known);
            let graph = Arc::new(key.graph(repo, commits, Extent::Whole, generation));
            self.whole = Some((key, graph.clone()));
            return Ok(graph);
        }
        match pieces::first(repo, &key.walk_options(), self.first)? {
            First::Whole(commits) => {
                let graph = Arc::new(key.graph(repo, commits, Extent::Whole, generation));
                self.whole = Some((key, graph.clone()));
                Ok(graph)
            }
            First::Piece { rows, walked } => {
                let graph = Arc::new(key.graph(repo, rows, Extent::Piece(walked), generation));
                self.walking = Some((key.clone(), generation, graph.clone()));
                if self.worker.is_none() {
                    self.worker = Some(Worker::start(&self.path, self.events.clone()));
                }
                if let Some(worker) = &self.worker {
                    worker.newest.store(generation, Ordering::SeqCst);
                    if worker.jobs.send((generation, self.first, key)).is_err() {
                        debug(|| "the thread that walks large histories is gone".to_owned());
                    }
                }
                Ok(graph)
            }
        }
    }

    /// Whatever the worker walks is no longer wanted.
    fn stop_walking(&mut self) {
        self.walking = None;
        if let Some(worker) = &self.worker {
            // A number no request has: the walk in progress stops.
            worker.newest.store(0, Ordering::SeqCst);
        }
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };

    use bitbraid_git::{Order, Repository, Tips, snapshot, walk};
    use bitbraid_testkit::{CorpusSpec, build_synthetic};
    use futures::channel::mpsc as fmpsc;

    use super::{Event, Extent, GraphData, GraphKey, Graphs};

    /// A history of some weeks, a commit an hour, with lines that merge
    /// into the trunk again and again: long enough to come in pieces when
    /// the first piece is made small.
    fn history() -> (std::path::PathBuf, Repository) {
        let spec = CorpusSpec {
            commits: 1200,
            lanes: 4,
            merge_every: 5,
            files: 10,
        };
        let dir = build_synthetic(&spec).unwrap();
        let repo = Repository::open(&dir).unwrap();
        (dir, repo)
    }

    fn key(repo: &Repository) -> GraphKey {
        GraphKey {
            snapshot: snapshot(repo).unwrap(),
            tips: Tips::All,
            exclude: Vec::new(),
            pinned: Vec::new(),
            order: Order::Date,
            worktree_rows: Vec::new(),
            dirty: false,
        }
    }

    /// The graph built in one go, to hold the others to.
    fn direct(repo: &Repository, key: &GraphKey) -> GraphData {
        let commits = walk(repo, &key.walk_options()).unwrap();
        key.graph(repo, commits, Extent::Whole, 0)
    }

    /// Every graph the walk of request `generation` sends, the whole one
    /// last.
    fn until_whole(
        events: &mut fmpsc::UnboundedReceiver<Event>,
        generation: u64,
    ) -> Vec<Arc<GraphData>> {
        let started = Instant::now();
        let mut seen = Vec::new();
        loop {
            match events.try_recv() {
                Ok(Event::Graph(graph)) => {
                    if graph.generation != generation {
                        continue;
                    }
                    let whole = graph.walked.is_none();
                    seen.push(graph);
                    if whole {
                        return seen;
                    }
                }
                Ok(Event::GraphFailed { error, .. }) => panic!("the walk failed: {error}"),
                Ok(_) => {}
                // Nothing yet.
                Err(_) => {
                    assert!(
                        started.elapsed() < Duration::from_secs(120),
                        "the whole graph never came"
                    );
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
    }

    /// A piece is the whole cut short: rows, lanes, the row of uncommitted
    /// work, and where the nearest child of each row is.
    fn begins(piece: &GraphData, whole: &GraphData, what: &str) {
        let rows = piece.commits.len();
        assert!(rows <= whole.commits.len(), "{what}");
        assert_eq!(piece.commits[..], whole.commits[..rows], "{what}: rows");
        assert_eq!(piece.rows[..], whole.rows[..rows], "{what}: lanes");
        assert_eq!(piece.wip, whole.wip, "{what}: uncommitted work");
        assert_eq!(piece.child[..], whole.child[..rows], "{what}: children");
        assert_eq!(piece.head, whole.head);
        assert_eq!(piece.badges, whole.badges);
        for (id, row) in &piece.index {
            assert_eq!(whole.index.get(id), Some(row), "{what}");
        }
        assert!(piece.max_width <= whole.max_width);
    }

    fn same(a: &GraphData, b: &GraphData, what: &str) {
        assert_eq!(a.commits.len(), b.commits.len(), "{what}");
        begins(a, b, what);
        assert_eq!(a.max_width, b.max_width, "{what}");
        assert_eq!(a.worktree_rows, b.worktree_rows, "{what}");
    }

    #[test]
    fn a_large_history_comes_in_pieces_that_begin_the_whole() {
        let (dir, repo) = history();
        let mut tested = 0;
        // Plain; with uncommitted work and two pinned lines; in topological
        // order; with the row of another worktree deep in the history.
        let plain = key(&repo);
        let mut pinned = key(&repo);
        pinned.dirty = true;
        pinned.pinned = vec!["refs/heads/lane-2".to_owned(), "refs/heads/main".to_owned()];
        let mut topo = key(&repo);
        topo.order = Order::Topo;
        let mut other = key(&repo);
        let deep = walk(&repo, &other.walk_options()).unwrap()[900].id;
        other
            .snapshot
            .worktrees
            .push(bitbraid_git::snapshot::Worktree {
                id: "other".to_owned(),
                path: Some("/work/other".into()),
                head: Some(deep),
                branch: None,
                locked: false,
                lock_reason: None,
                is_main: false,
                current: false,
            });
        other.worktree_rows = vec!["/work/other".to_owned(), "/work/unknown".to_owned()];
        other.dirty = true;

        for (what, key) in [
            ("plain", plain),
            ("pinned", pinned),
            ("topo", topo),
            ("worktree", other),
        ] {
            let (tx, mut events) = fmpsc::unbounded();
            let mut graphs = Graphs::new(&dir, tx);
            graphs.first = 150;
            let want = direct(&repo, &key);
            let first = graphs.request(&repo, key.clone()).unwrap();
            assert_eq!(first.walked, Some(150), "{what}: the first piece");
            begins(&first, &want, what);
            // Asked again meanwhile: what there is so far, and no second walk.
            let again = graphs.request(&repo, key.clone()).unwrap();
            assert_eq!(again.generation, first.generation);
            begins(&again, &want, what);

            let sent = until_whole(&mut events, first.generation);
            let whole = sent.last().unwrap();
            same(whole, &want, what);
            for piece in &sent {
                begins(piece, &want, what);
                assert!(piece.follows(&first), "{what}: it is no step back");
                tested += 1;
            }
            // Each is more than the one before.
            assert!(sent.windows(2).all(|pair| pair[1].follows(&pair[0])));
            assert!(!first.follows(whole), "a late first piece is not taken");
            // From now on the whole graph is what is handed out.
            let kept = graphs.request(&repo, key.clone()).unwrap();
            assert!(Arc::ptr_eq(&kept, whole), "{what}");
            if what == "worktree" {
                let row = bitbraid_git::wip::row_id(std::path::Path::new("/work/other"));
                let at = whole.index[&row];
                assert_eq!(whole.commits[at + 1].id, deep);
                assert_eq!(whole.commits[at].parents, [deep]);
                assert_eq!(whole.worktree_rows.len(), 1, "the unknown path has no row");
                // The row is no child that a nearest branch is found by.
                assert_ne!(whole.child[at + 1] as usize, at);
            }
            if what == "plain" {
                assert!(first.commits.len() >= 64, "{} rows", first.commits.len());
                assert!(sent.len() >= 2, "a piece on the way to the whole");
            }
        }
        assert!(tested >= 5);
    }

    #[test]
    fn the_same_commits_are_laid_out_again_without_another_walk() {
        let (dir, repo) = history();
        let (tx, mut events) = fmpsc::unbounded();
        let mut graphs = Graphs::new(&dir, tx);
        graphs.first = 150;
        let clean = key(&repo);
        let first = graphs.request(&repo, clean.clone()).unwrap();
        let whole = until_whole(&mut events, first.generation).pop().unwrap();

        // Uncommitted work appears: the whole graph at once, with its row.
        let mut dirty = clean.clone();
        dirty.dirty = true;
        let with_wip = graphs.request(&repo, dirty.clone()).unwrap();
        assert_eq!(
            with_wip.walked, None,
            "no pieces for commits that are known"
        );
        assert!(with_wip.generation > whole.generation);
        same(&with_wip, &direct(&repo, &dirty), "uncommitted work");
        assert!(with_wip.wip.is_some());
        // A branch is pinned.
        let mut pinned = dirty.clone();
        pinned.pinned = vec!["refs/heads/lane-1".to_owned()];
        let moved = graphs.request(&repo, pinned.clone()).unwrap();
        assert_eq!(moved.walked, None);
        same(&moved, &direct(&repo, &pinned), "a pinned branch");
        assert_ne!(moved.rows, with_wip.rows, "the pin moves lanes");
        // And back: what is asked for twice in a row is the same graph.
        let back = graphs.request(&repo, clean.clone()).unwrap();
        same(&back, &whole, "back to where it was");
        let twice = graphs.request(&repo, clean.clone()).unwrap();
        assert!(Arc::ptr_eq(&back, &twice));

        // Other commits (two lines hidden): walked anew, in pieces.
        let mut hidden = clean.clone();
        hidden.exclude = vec![
            "refs/heads/lane-0".to_owned(),
            "refs/heads/lane-3".to_owned(),
        ];
        let piece = graphs.request(&repo, hidden.clone()).unwrap();
        assert!(piece.walked.is_some());
        let sent = until_whole(&mut events, piece.generation);
        same(
            sent.last().unwrap(),
            &direct(&repo, &hidden),
            "two lines hidden",
        );
        // A walk that is no longer wanted is stopped by the next request,
        // and what that one asks for is what comes.
        let mut other = hidden.clone();
        other.exclude = vec!["refs/heads/lane-1".to_owned()];
        let abandoned = graphs.request(&repo, other).unwrap();
        assert!(abandoned.walked.is_some());
        let wanted = graphs.request(&repo, clean.clone()).unwrap();
        assert!(wanted.generation > abandoned.generation);
        let sent = until_whole(&mut events, wanted.generation);
        same(sent.last().unwrap(), &whole, "after an abandoned walk");
    }

    #[test]
    fn a_small_history_is_handed_out_whole_at_once() {
        let (dir, repo) = history();
        let (tx, mut events) = fmpsc::unbounded();
        let mut graphs = Graphs::new(&dir, tx);
        // As the application has it: 1,200 commits are one piece.
        let whole = graphs.request(&repo, key(&repo)).unwrap();
        assert_eq!(whole.walked, None);
        assert_eq!(whole.commits.len(), 1200);
        same(&whole, &direct(&repo, &key(&repo)), "in one go");
        assert!(graphs.worker.is_none(), "no thread is started for it");
        assert!(events.try_recv().is_err(), "and nothing is sent after it");
        assert!(Arc::ptr_eq(graphs.current().unwrap(), &whole));
    }
}
