//! File history: the commits that changed a path, following renames.
//!
//! Two walks, each the one git makes, so that the list is git's list in
//! git's order:
//!
//! - Without following, `git log -- <path>`: a commit is listed when the
//!   path differs from every parent. A merge that has the path as one of
//!   its parents has it changed nothing, and only that parent is walked
//!   on: what happened on the other side did not get into the result (a
//!   change that was reverted there, or the same change made twice).
//! - Following, `git log --follow -- <path>`: every commit is walked, a
//!   merge is never listed, and a commit with one parent is when the path
//!   differs from it. Where the path appears, the file it was renamed or
//!   copied from is looked for among all the files of the parent, and the
//!   parent is walked under that name. A name belongs to a commit, not to
//!   the walk: each parent of a merge gets the name the file has on its
//!   side, so that a branch that still had the old name when it was
//!   merged is read by the old name.
//!
//! Commits come newest first as the walk reaches them: the newest of the
//! commits whose child was walked, which is git's default order and not
//! the graph's (a parent dated after its child comes late here, early in
//! the graph).
//!
//! What the walk costs is reading: every commit that is walked, and the
//! trees on the way to the path in each. Two things keep that down, both
//! of them what git does. Where the repository has a commit-graph file,
//! a commit's tree, parents and date are taken from it and the commit
//! itself is not read. And a tree is read once: what a name is in a tree
//! is remembered by the tree's id, so the directories a commit did not
//! touch, which have the id they had in its parent, are not read again.
//! What is left is the root tree of each commit, which nearly every
//! commit changes.
//!
//! One kind of repository is not walked here at all: one where commits
//! are replaced (`refs/replace/`) or grafted (`info/grafts`). Git reads
//! such a commit as its replacement says, with other parents or another
//! tree; gitoxide (0.89) reads it as it is stored, so a walk of ours
//! would list another history than git does. There git itself is asked
//! (`git log --raw`, with `--follow` or without), and its list is the
//! answer. What each listed commit says of itself (author, message,
//! parents) is still read by gitoxide, from the commit as it is stored.
//!
//! Sources: git-log(1) (`--follow`, `--raw`, `-z`, `--diff-merges`,
//! `--stdin`), git-rev-list(1) ("History Simplification", "Commit
//! Ordering"), git-diff(1) ("Raw output format"), git-replace(1),
//! gitrepository-layout(5) (`info/grafts`), gitformat-commit-graph(5)
//! (what the file holds, and that git leaves it alone in a shallow
//! repository and where objects are replaced).

use gix::bstr::ByteSlice;
use serde::{Deserialize, Serialize};

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
};

use crate::{
    CommitInfo, LogOptions, Oid, Repository, Result, exe, git_err,
    log::{details, resolve_tips},
};

/// How the path changed in a listed commit.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HistoryKind {
    /// First appearance.
    Added,
    /// Content or mode changed.
    Modified,
    Deleted,
    /// Appeared under this name, previously known as `previous_path`.
    Renamed,
    /// Appeared as a copy of `previous_path`, which stayed where it was.
    Copied,
}

/// One entry of a file's history.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryEntry {
    pub commit: CommitInfo,
    /// Path of the file in this commit.
    pub path: String,
    /// What happened.
    pub kind: HistoryKind,
    /// Path before a rename, or the path this was copied from.
    pub previous_path: Option<String>,
    /// Blob id in this commit, absent for deletions.
    pub blob: Option<Oid>,
}

/// Options for [`file_history`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HistoryOptions {
    /// Walk options (tips, limit of commits examined, first-parent).
    #[serde(flatten)]
    pub log: LogOptions,
    /// Follow renames.
    #[serde(default = "default_true")]
    pub follow: bool,
    /// Stop after this many history entries.
    #[serde(default)]
    pub max_entries: Option<usize>,
}

fn default_true() -> bool {
    true
}

impl Default for HistoryOptions {
    fn default() -> Self {
        Self {
            log: LogOptions {
                tips: crate::Tips::Head,
                ..Default::default()
            },
            follow: true,
            max_entries: None,
        }
    }
}

/// What a path is in a tree: the object and its mode.
type Entry = Option<(Oid, u16)>;

/// A commit as the walk needs it.
#[derive(Debug, Clone)]
struct Node {
    tree: gix::ObjectId,
    parents: Vec<Oid>,
    seconds: i64,
}

/// The commits walked so far and what the followed path is in each.
struct Walked<'r> {
    r: &'r gix::Repository,
    /// The commit-graph file, where there is one that may be used.
    graph: Option<gix::commitgraph::Graph>,
    nodes: HashMap<Oid, Option<Node>>,
    /// What a path is in a commit, by the number of the path's name.
    entries: HashMap<(Oid, usize), Entry>,
    /// What the part of a name at some depth is in a tree: the tree, the
    /// number of the name and how many parts stand before this one.
    in_tree: HashMap<(gix::ObjectId, usize, usize), Option<(gix::ObjectId, u16)>>,
    /// Commits waiting their turn: newest first, then first come.
    waiting: BinaryHeap<(i64, Reverse<u64>, Oid)>,
    queued: HashSet<Oid>,
    turn: u64,
}

impl<'r> Walked<'r> {
    fn new(r: &'r gix::Repository) -> Self {
        Self {
            r,
            graph: usable_graph(r),
            nodes: HashMap::new(),
            entries: HashMap::new(),
            in_tree: HashMap::new(),
            waiting: BinaryHeap::new(),
            queued: HashSet::new(),
            turn: 0,
        }
    }

    /// The commit, or nothing when the id names no commit that is here: a
    /// tag of a tree, or a parent beyond the edge of a shallow clone,
    /// which is where history ends as far as anyone can tell.
    fn node(&mut self, id: Oid) -> Option<&Node> {
        let (r, graph) = (self.r, self.graph.as_ref());
        self.nodes
            .entry(id)
            .or_insert_with(|| {
                // From the commit-graph where the commit is in it: it was
                // written at some time, and what came after is read as an
                // object.
                if let Some(node) = graph.and_then(|graph| from_graph(graph, id)) {
                    return Some(node);
                }
                let commit = r.find_commit(*id.as_gix()).ok()?;
                Some(Node {
                    tree: commit.tree_id().ok()?.detach(),
                    parents: commit.parent_ids().map(|p| Oid::from(p.detach())).collect(),
                    seconds: commit.time().ok()?.seconds,
                })
            })
            .as_ref()
    }

    /// What `path`, the name numbered `name`, is in the commit.
    fn entry(&mut self, id: Oid, name: usize, path: &str) -> Result<Entry> {
        if let Some(known) = self.entries.get(&(id, name)) {
            return Ok(*known);
        }
        let Some(tree) = self.node(id).map(|node| node.tree) else {
            return Ok(None);
        };
        let entry = self.lookup(tree, name, path)?;
        self.entries.insert((id, name), entry);
        Ok(entry)
    }

    /// What `path`, the name numbered `name`, is under the tree `root`:
    /// down the directories of the path, each of them read once however
    /// many commits have it.
    fn lookup(&mut self, root: gix::ObjectId, name: usize, path: &str) -> Result<Entry> {
        let mut tree = root;
        let mut parts = path.split('/').filter(|part| !part.is_empty()).peekable();
        let mut depth = 0usize;
        let mut found = None;
        while let Some(part) = parts.next() {
            let key = (tree, name, depth);
            let entry = match self.in_tree.get(&key) {
                Some(known) => *known,
                None => {
                    let read = self
                        .r
                        .find_tree(tree)
                        .map_err(git_err(say!("history-loading-tree")))?;
                    let entry = read
                        .iter()
                        .filter_map(|entry| entry.ok())
                        .find(|entry| entry.filename() == part.as_bytes())
                        .map(|entry| (entry.oid().to_owned(), entry.mode().value()));
                    self.in_tree.insert(key, entry);
                    entry
                }
            };
            let Some((id, mode)) = entry else {
                return Ok(None);
            };
            if parts.peek().is_none() {
                found = Some((Oid::from(id), mode));
                break;
            }
            // What stands in the way of the rest of the path and is no
            // directory: the path is not there.
            if !gix::object::tree::EntryMode::try_from(u32::from(mode))
                .is_ok_and(|mode| mode.is_tree())
            {
                return Ok(None);
            }
            tree = id;
            depth = depth.saturating_add(1);
        }
        Ok(found)
    }

    /// Have the commit walked, once.
    fn queue(&mut self, id: Oid) {
        if self.queued.contains(&id) {
            return;
        }
        let Some(seconds) = self.node(id).map(|node| node.seconds) else {
            return;
        };
        self.queued.insert(id);
        self.waiting.push((seconds, Reverse(self.turn), id));
        self.turn = self.turn.saturating_add(1);
    }

    /// The parents of a commit that are here to be walked.
    fn parents(&mut self, id: Oid, first_only: bool) -> Vec<Oid> {
        let listed = self
            .node(id)
            .map(|node| node.parents.clone())
            .unwrap_or_default();
        listed
            .into_iter()
            .filter(|parent| self.node(*parent).is_some())
            .take(if first_only { 1 } else { usize::MAX })
            .collect()
    }
}

/// The commit-graph file of a repository, when it says what the objects
/// say. Git does not use it in a shallow repository, nor where objects are
/// replaced or grafted (the file was written without knowing of those),
/// and neither is it used here then.
fn usable_graph(r: &gix::Repository) -> Option<gix::commitgraph::Graph> {
    // When it cannot be told whether it is shallow, it is taken for it.
    if r.is_shallow().unwrap_or(true) {
        return None;
    }
    // Nor when it cannot be told whether anything is replaced.
    if replaced(r)? {
        return None;
    }
    r.commit_graph_if_enabled().ok().flatten()
}

/// Whether commits of this repository are read by git as something else
/// than what is stored: replaced by a reference under `refs/replace/`, or
/// grafted. None when the references cannot be listed.
fn replaced(r: &gix::Repository) -> Option<bool> {
    let references = r.references().ok()?;
    let by_reference = references.prefixed("refs/replace/").ok()?.next().is_some();
    Some(by_reference || r.common_dir().join("info").join("grafts").exists())
}

/// One commit of `git log -z --raw --format=%x1e%H`: the commit, the
/// path it has the file under, what happened to it, the name it had
/// before and the blob it is now.
type Listed = (Oid, String, HistoryKind, Option<String>, Option<Oid>);

/// Read what `git log -z --raw --format=%x1e%H` printed for one path. A
/// commit is a record separator, its id and a NUL; what happened to the
/// path follows as `:<mode> <mode> <blob> <blob> <letter>`, a NUL, the
/// path and a NUL, with a second path after a rename or a copy. A commit
/// git lists without saying what happened (it does not for a root commit
/// it was told nothing about) is listed as a change of `asked`.
fn parse_raw_log(printed: &str, asked: &str) -> Vec<Listed> {
    let mut listed = Vec::new();
    for record in printed.split('\u{1e}').skip(1) {
        let mut fields = record.split('\0');
        let Some(id) = fields.next().and_then(|id| id.trim().parse::<Oid>().ok()) else {
            continue;
        };
        let mut entry: Listed = (id, asked.to_owned(), HistoryKind::Modified, None, None);
        while let Some(field) = fields.next() {
            let Some(what) = field.trim_start_matches('\n').strip_prefix(':') else {
                continue;
            };
            let mut parts = what.split(' ');
            let blob = parts.nth(3).and_then(|blob| blob.parse::<Oid>().ok());
            let letter = parts.next().and_then(|status| status.chars().next());
            let Some(first) = fields.next() else { break };
            let (kind, moved) = match letter {
                Some('A') => (HistoryKind::Added, false),
                Some('D') => (HistoryKind::Deleted, false),
                Some('R') => (HistoryKind::Renamed, true),
                Some('C') => (HistoryKind::Copied, true),
                _ => (HistoryKind::Modified, false),
            };
            let (path, previous) = if moved {
                match fields.next() {
                    Some(now) => (now, Some(first.to_owned())),
                    None => break,
                }
            } else {
                (first, None)
            };
            let blob = blob.filter(|_| kind != HistoryKind::Deleted);
            entry = (id, path.to_owned(), kind, previous, blob);
            break;
        }
        listed.push(entry);
    }
    listed
}

/// The history of `path` as git lists it, for a repository whose commits
/// git reads as something else than what is stored. `limit` (how many
/// commits are looked at) has no counterpart there and is left out; what
/// is asked for at most (`max_entries`) holds.
fn by_git(repo: &Repository, path: &str, options: &HistoryOptions) -> Result<Vec<HistoryEntry>> {
    let tips = resolve_tips(repo, &options.log.tips)?;
    if tips.is_empty() {
        return Ok(Vec::new());
    }
    let mut args: Vec<String> = [
        // The path is a path, whatever characters it has.
        "--literal-pathspecs",
        // What a user set up for reading a log by hand is kept out.
        "-c",
        "log.follow=false",
        "-c",
        "log.showSignature=false",
        "log",
        "-z",
        "--raw",
        "--no-abbrev",
        "--no-color",
        // A merge is held against its first parent, as the walk here does.
        "--diff-merges=first-parent",
        "--format=%x1e%H",
        "--stdin",
    ]
    .iter()
    .map(|arg| (*arg).to_owned())
    .collect();
    if options.follow {
        args.push("--follow".to_owned());
    }
    if options.log.first_parent {
        args.push("--first-parent".to_owned());
    }
    if let Some(most) = options.max_entries {
        args.push(format!("--max-count={most}"));
    }
    args.push("--".to_owned());
    args.push(path.to_owned());
    // The starting points on standard input: there may be thousands.
    let mut from = String::new();
    for tip in tips {
        from.push_str(&tip.to_hex());
        from.push('\n');
    }
    let printed = exe::run(repo, &args, Some(from.as_bytes()))?;
    let listed = parse_raw_log(&printed.stdout, path);
    let ids: Vec<Oid> = listed.iter().map(|(id, ..)| *id).collect();
    let mut decoded: HashMap<Oid, CommitInfo> = details(repo, &ids)?
        .into_iter()
        .map(|commit| (commit.id, commit))
        .collect();
    Ok(listed
        .into_iter()
        .filter_map(|(id, path, kind, previous_path, blob)| {
            Some(HistoryEntry {
                // A commit listed twice would be one that git walked twice.
                commit: decoded.remove(&id)?,
                path,
                kind,
                previous_path,
                blob,
            })
        })
        .collect())
}

/// A commit as the commit-graph file has it; nothing when it is not in the
/// file, or the file cannot be read there.
fn from_graph(graph: &gix::commitgraph::Graph, id: Oid) -> Option<Node> {
    let commit = graph.commit_by_id(id.as_gix())?;
    let tree = commit.root_tree_id().to_owned();
    let seconds = i64::try_from(commit.committer_timestamp()).ok()?;
    let mut parents = Vec::new();
    for parent in commit.iter_parents() {
        parents.push(Oid::from(graph.id_at(parent.ok()?).to_owned()));
    }
    Some(Node {
        tree,
        parents,
        seconds,
    })
}

/// One new file is compared for likeness with as many files at most, the
/// way `diff.renameLimit` counts pairs (1000 a side).
const ORIGIN_PAIRS: usize = 1000 * 1000;
/// A line longer than this counts in pieces of this length, as git has it.
const LONGEST_PIECE: usize = 64;

/// How much of `source` is in `target`, in bytes: every line (or piece of
/// a long line) of the source counts as often as the target has it too,
/// wherever it stands there. This is the measure git ranks renames and
/// copies by, so that the same file comes out as the origin.
fn bytes_shared(source: &[u8], target: &[u8]) -> usize {
    /// A piece and whether the line ends with it; a carriage return
    /// before the end of a line is no part of it.
    fn pieces(bytes: &[u8]) -> impl Iterator<Item = (&[u8], bool)> {
        bytes
            .split_inclusive(|byte| *byte == b'\n')
            .flat_map(|line| {
                let (text, ended) = match line.strip_suffix(b"\n") {
                    Some(text) => (text.strip_suffix(b"\r").unwrap_or(text), true),
                    None => (line, false),
                };
                let whole = text.len().saturating_add(usize::from(ended));
                (0..whole.div_ceil(LONGEST_PIECE)).map(move |piece| {
                    let from = piece.saturating_mul(LONGEST_PIECE);
                    let to = from.saturating_add(LONGEST_PIECE).min(whole);
                    (
                        text.get(from..to.min(text.len())).unwrap_or_default(),
                        ended && to == whole,
                    )
                })
            })
    }
    let weight = |(text, ended): (&[u8], bool)| text.len().saturating_add(usize::from(ended));
    let mut in_source: HashMap<(&[u8], bool), usize> = HashMap::new();
    for piece in pieces(source) {
        let bytes = in_source.entry(piece).or_default();
        *bytes = bytes.saturating_add(weight(piece));
    }
    let mut shared = 0usize;
    for piece in pieces(target) {
        if let Some(left) = in_source.get_mut(&piece) {
            let taken = (*left).min(weight(piece));
            *left = left.saturating_sub(taken);
            shared = shared.saturating_add(taken);
        }
    }
    shared
}

/// How alike two files are, by their ids, once it was worked out.
type Alike = HashMap<(gix::ObjectId, gix::ObjectId), usize>;

/// As many files with the very content of the new one are looked at.
const IDENTICAL_LOOKED_AT: usize = 100;
/// As many of the most alike are kept to choose from.
const ALIKE_KEPT: usize = 4;

/// Where a file that appears in `child` at `path` came from: the file of
/// the parent it is a rename or a copy of, and whether that one stayed (a
/// copy). As `git log --follow` looks for it, among every file of the
/// parent:
///
/// - one with the same content first: of the first hundred by path, one
///   that is gone from the child before one that stayed, and one with the
///   same file name before another;
/// - else one at least half of which the new file shares: of the four
///   most alike (the same file name first among equals, then the first by
///   path), the first that is gone from the child, else the first.
///
/// In a tree of more files than git would pair with one new file, among
/// the ones with the same content only.
fn origin_of(
    r: &gix::Repository,
    alike: &mut Alike,
    parent: Oid,
    child: Oid,
    path: &str,
    (blob, mode): (Oid, u16),
) -> Result<Option<(String, bool)>> {
    let tree_of = |commit: Oid| {
        r.find_commit(*commit.as_gix())
            .map_err(git_err(say!("history-loading-commit")))?
            .tree()
            .map_err(git_err(say!("history-loading-tree")))
    };
    let (old_tree, new_tree) = (tree_of(parent)?, tree_of(child)?);
    let mut candidates = old_tree
        .traverse()
        .breadthfirst
        .files()
        .map_err(git_err(say!("history-listing-tree")))?;
    candidates.retain(|entry| entry.mode.is_blob() || entry.mode.is_link());
    candidates.sort_by(|a, b| a.filepath.cmp(&b.filepath));
    let alike_too = candidates.len() <= ORIGIN_PAIRS;

    fn name(path: &[u8]) -> &[u8] {
        path.rsplit(|byte| *byte == b'/').next().unwrap_or(path)
    }
    let wanted = name(path.as_bytes());
    // A rename when the file it came from is gone, a copy when it stayed.
    let stayed = |found: &gix::bstr::BStr| -> Result<bool> {
        Ok(new_tree
            .lookup_entry_by_path(found.to_path_lossy())
            .map_err(git_err(say!("history-looking-up-path")))?
            .is_some())
    };
    let answer = |found: &gix::bstr::BStr, stayed: bool| {
        Ok(Some((found.to_str_lossy().into_owned(), stayed)))
    };
    // A file, as against a link (or a file marked executable, which is a
    // file all the same).
    let regular = mode & 0o170_000 == 0o100_000;

    // The same content under another name. What is not a file on both
    // sides has to be the same kind of thing.
    let mut best: Option<(u8, bool, &gix::bstr::BStr)> = None;
    for entry in candidates
        .iter()
        .filter(|entry| entry.oid == *blob.as_gix())
        .filter(|entry| (regular && entry.mode.is_blob()) || entry.mode.value() == mode)
        .take(IDENTICAL_LOOKED_AT)
    {
        let found: &gix::bstr::BStr = entry.filepath.as_ref();
        let kept = stayed(found)?;
        let score = u8::from(!kept).saturating_add(u8::from(name(found) == wanted));
        if best.is_none_or(|(most, _, _)| score > most) {
            best = Some((score, kept, found));
            if score == 2 {
                break;
            }
        }
    }
    if let Some((_, kept, found)) = best {
        return answer(found, kept);
    }
    if !alike_too || !regular {
        return Ok(None);
    }

    // The most alike, among files. A size apart by more than half of the
    // larger cannot share half.
    let target = r
        .find_blob(*blob.as_gix())
        .map_err(git_err(say!("history-loading-blob")))?;
    let target = &target.data;
    if target.is_empty() || crate::diff::looks_binary(target) {
        return Ok(None);
    }
    // In sixty-thousandths, as git counts: equal scores are equal there.
    // Nothing for a file that cannot be the one.
    let score_of = |source: gix::ObjectId| -> usize {
        let Ok(header) = r.find_header(source) else {
            return 0;
        };
        let size = usize::try_from(header.size()).unwrap_or(usize::MAX);
        let larger = size.max(target.len());
        if size == 0 || larger.abs_diff(size.min(target.len())) > larger / 2 {
            return 0;
        }
        let Ok(source) = r.find_blob(source) else {
            return 0;
        };
        if crate::diff::looks_binary(&source.data) {
            return 0;
        }
        bytes_shared(&source.data, target)
            .min(larger)
            .saturating_mul(60_000)
            / larger
    };
    let mut most: Vec<(usize, bool, &gix::bstr::BStr)> = Vec::with_capacity(ALIKE_KEPT + 1);
    for entry in candidates.iter().filter(|entry| entry.mode.is_blob()) {
        // Most files of a tree are the files of the last tree asked
        // about: each is compared with this one once.
        let score = *alike
            .entry((entry.oid, *blob.as_gix()))
            .or_insert_with(|| score_of(entry.oid));
        if score < 30_000 {
            continue;
        }
        let found: &gix::bstr::BStr = entry.filepath.as_ref();
        let rank = (score, name(found) == wanted);
        // Before the first that is less: equals stay in the order they
        // came, which is by path.
        let at = most
            .iter()
            .position(|(score, named, _)| (*score, *named) < rank)
            .unwrap_or(most.len());
        if at < ALIKE_KEPT {
            most.insert(at, (rank.0, rank.1, found));
            most.truncate(ALIKE_KEPT);
        }
    }
    for (_, _, found) in &most {
        if !stayed(found)? {
            return answer(found, false);
        }
    }
    match most.first() {
        Some((_, _, found)) => answer(found, true),
        None => Ok(None),
    }
}

/// History of `path`. In a partial clone what it needs and was left on
/// the remote is fetched ([`crate::promisor`]).
pub fn file_history(
    repo: &Repository,
    path: &str,
    options: &HistoryOptions,
) -> Result<Vec<HistoryEntry>> {
    crate::promisor::fetching(
        repo,
        || file_history_here(repo, path, options),
        || crate::promisor::fetch_for_history(repo, path),
    )
}

/// [`file_history`] with what is here.
fn file_history_here(
    repo: &Repository,
    path: &str,
    options: &HistoryOptions,
) -> Result<Vec<HistoryEntry>> {
    struct Found {
        commit: Oid,
        path: String,
        kind: HistoryKind,
        previous_path: Option<String>,
        blob: Option<Oid>,
    }
    let r = repo.gix();
    // Where commits are replaced or grafted, git reads other parents and
    // trees than gitoxide does: its own list is the history then. Should
    // git not answer, the walk below is what there is.
    if replaced(r) == Some(true)
        && let Ok(entries) = by_git(repo, path, options)
    {
        return Ok(entries);
    }
    let mut walk = Walked::new(r);
    for tip in resolve_tips(repo, &options.log.tips)? {
        walk.queue(tip);
    }
    // The names the file goes by, and which of them each commit is read
    // by: the one its child handed it, the last child to be walked if
    // several did. The tips are read by the name that was asked for.
    let mut alike = Alike::new();
    let mut names: Vec<String> = vec![path.to_owned()];
    let mut name_of: HashMap<Oid, usize> = HashMap::new();
    let mut found: Vec<Found> = Vec::new();
    let mut examined = 0usize;
    while let Some((_, _, id)) = walk.waiting.pop() {
        if options.log.limit.is_some_and(|limit| examined >= limit)
            || options.max_entries.is_some_and(|max| found.len() >= max)
        {
            break;
        }
        examined = examined.saturating_add(1);
        let name = name_of.get(&id).copied().unwrap_or(0);
        let Some(current) = names.get(name).cloned() else {
            continue;
        };
        let here = walk.entry(id, name, &current)?;
        let (parents, before) = if options.follow {
            let parents = walk.parents(id, options.log.first_parent);
            let mut before = Vec::with_capacity(parents.len());
            for parent in &parents {
                before.push(walk.entry(*parent, name, &current)?);
            }
            (parents, before)
        } else {
            // The parents in turn, up to the first that has the path as it
            // is here: then nothing happened in this commit, only that
            // parent is walked on, and the parents after it are not looked
            // at, as git does not look at them. That is a commit and a
            // tree not read for every merge of a branch that left the
            // path alone, and there are many of those.
            let listed = walk
                .node(id)
                .map(|node| node.parents.clone())
                .unwrap_or_default();
            let (mut parents, mut before) = (Vec::new(), Vec::new());
            let mut same = None;
            for parent in listed {
                // Beyond the edge of a shallow clone there is nothing.
                if walk.node(parent).is_none() {
                    continue;
                }
                let entry = walk.entry(parent, name, &current)?;
                if entry == here {
                    same = Some(parent);
                    break;
                }
                parents.push(parent);
                before.push(entry);
                if options.log.first_parent {
                    break;
                }
            }
            if let Some(parent) = same {
                walk.queue(parent);
                continue;
            }
            (parents, before)
        };
        if options.follow && parents.len() > 1 {
            // A merge is never the commit that changed the file. Each of
            // its parents is read by the name the file has there: where
            // the file is not, by the name of what it came from.
            for (parent, entry) in parents.iter().zip(&before) {
                let theirs = match (here, entry) {
                    (Some(now), None) => {
                        match origin_of(r, &mut alike, *parent, id, &current, now)? {
                            Some((source, _)) => {
                                names.push(source);
                                names.len().saturating_sub(1)
                            }
                            None => name,
                        }
                    }
                    _ => name,
                };
                name_of.insert(*parent, theirs);
                walk.queue(*parent);
            }
            continue;
        }
        let first = before.first().copied().flatten();
        let mut theirs = name;
        let change = match (here, first) {
            (Some(now), Some(then)) if now == then => None,
            (Some(_), Some(_)) => Some((HistoryKind::Modified, None)),
            (Some(now), None) => {
                let origin = match parents.first() {
                    Some(parent) if options.follow => {
                        origin_of(r, &mut alike, *parent, id, &current, now)?
                    }
                    _ => None,
                };
                Some(match origin {
                    Some((source, stayed)) => {
                        names.push(source.clone());
                        theirs = names.len().saturating_sub(1);
                        let kind = if stayed {
                            HistoryKind::Copied
                        } else {
                            HistoryKind::Renamed
                        };
                        (kind, Some(source))
                    }
                    None => (HistoryKind::Added, None),
                })
            }
            (None, Some(_)) => Some((HistoryKind::Deleted, None)),
            (None, None) => None,
        };
        for parent in &parents {
            if options.follow {
                name_of.insert(*parent, theirs);
            }
            walk.queue(*parent);
        }
        if let Some((kind, previous_path)) = change {
            found.push(Found {
                commit: id,
                path: current,
                kind,
                previous_path,
                blob: here.map(|(blob, _)| blob),
            });
        }
    }
    let ids: Vec<Oid> = found.iter().map(|entry| entry.commit).collect();
    let mut decoded: HashMap<Oid, CommitInfo> = details(repo, &ids)?
        .into_iter()
        .map(|commit| (commit.id, commit))
        .collect();
    Ok(found
        .into_iter()
        .filter_map(|entry| {
            Some(HistoryEntry {
                commit: decoded.remove(&entry.commit)?,
                path: entry.path,
                kind: entry.kind,
                previous_path: entry.previous_path,
                blob: entry.blob,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_git_prints_for_a_path_is_read_commit_by_commit() {
        let (one, two, three) = ("1".repeat(40), "2".repeat(40), "3".repeat(40));
        let (old, new, none) = ("a".repeat(40), "b".repeat(40), "0".repeat(40));
        let printed = format!(
            "\u{1e}{one}\0\n:100644 100644 {old} {new} M\0src/b.txt\0\
             \u{1e}{two}\0\n:100644 100644 {old} {old} R100\0a.txt\0src/b.txt\0\
             \u{1e}{three}\0\n:000000 100644 {none} {old} A\0a.txt\0"
        );
        let listed = parse_raw_log(&printed, "src/b.txt");
        let id = |hex: &str| hex.parse::<Oid>().unwrap();
        assert_eq!(
            listed,
            vec![
                (
                    id(&one),
                    "src/b.txt".to_owned(),
                    HistoryKind::Modified,
                    None,
                    Some(id(&new))
                ),
                (
                    id(&two),
                    "src/b.txt".to_owned(),
                    HistoryKind::Renamed,
                    Some("a.txt".to_owned()),
                    Some(id(&old))
                ),
                (
                    id(&three),
                    "a.txt".to_owned(),
                    HistoryKind::Added,
                    None,
                    Some(id(&old))
                ),
            ]
        );
        // A deletion has no blob; a copy names where it came from; a
        // commit git says nothing about is a change of the path asked for.
        let printed = format!(
            "\u{1e}{one}\0\n:100644 000000 {old} {none} D\0gone.txt\0\
             \u{1e}{two}\0\n:100644 100644 {old} {new} C75\0from.txt\0to.txt\0\
             \u{1e}{three}\0"
        );
        let listed = parse_raw_log(&printed, "to.txt");
        assert_eq!(
            listed
                .iter()
                .map(|(_, path, kind, previous, blob)| (
                    path.as_str(),
                    *kind,
                    previous.as_deref(),
                    blob.is_some()
                ))
                .collect::<Vec<_>>(),
            [
                ("gone.txt", HistoryKind::Deleted, None, false),
                ("to.txt", HistoryKind::Copied, Some("from.txt"), true),
                ("to.txt", HistoryKind::Modified, None, false),
            ]
        );
        // Nothing, and nonsense, list nothing and take nothing down.
        assert!(parse_raw_log("", "x").is_empty());
        for odd in [
            "no separator at all",
            "\u{1e}",
            "\u{1e}not an id\0:1 2 3 4 M\0x\0",
            "\u{1e}\0\0\0",
            "\u{1e}1111\0:",
            "\u{1e}\u{1e}\u{1e}",
        ] {
            assert!(parse_raw_log(odd, "x").is_empty(), "{odd:?}");
        }
        // Cut off in the middle of a commit: what is whole is kept.
        let cut = format!("\u{1e}{one}\0\n:100644 100644 {old} {new} R90\0a.txt");
        assert_eq!(parse_raw_log(&cut, "b.txt").len(), 1);
        let cut = format!("\u{1e}{one}\0\n:100644 100644 {old}");
        assert_eq!(parse_raw_log(&cut, "b.txt").len(), 1);
    }

    #[test]
    fn what_two_files_share_is_counted_in_bytes_of_lines() {
        // The same: all of it. Nothing in common: nothing.
        assert_eq!(bytes_shared(b"one\ntwo\nthree\n", b"one\ntwo\nthree\n"), 14);
        assert_eq!(bytes_shared(b"one\ntwo\n", b"three\nfour\n"), 0);
        assert_eq!(bytes_shared(b"", b"anything\n"), 0);
        assert_eq!(bytes_shared(b"anything\n", b""), 0);
        // Wherever the lines stand, and each as often as both have it.
        assert_eq!(bytes_shared(b"a\nb\nc\n", b"c\nx\na\n"), 4);
        assert_eq!(bytes_shared(b"a\na\na\n", b"a\na\n"), 4);
        assert_eq!(bytes_shared(b"a\na\n", b"a\na\na\n"), 4);
        // A line without its end is another line; a carriage return
        // before the end is none.
        assert_eq!(bytes_shared(b"last", b"last\n"), 0);
        assert_eq!(bytes_shared(b"one\r\ntwo\r\n", b"one\ntwo\n"), 8);
        // A long line counts in pieces, so that one changed end does not
        // lose all of it.
        let long = "x".repeat(200);
        let other = format!("{}y", "x".repeat(199));
        assert_eq!(bytes_shared(long.as_bytes(), long.as_bytes()), 200);
        assert_eq!(bytes_shared(long.as_bytes(), other.as_bytes()), 192);
        // Bytes that are no text are bytes.
        assert_eq!(bytes_shared(&[0xff, b'\n', 0x00], &[0xff, b'\n']), 2);
    }
}
