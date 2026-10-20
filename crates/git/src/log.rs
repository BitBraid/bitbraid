//! Commit listing in an order fit for drawing a graph: newest first by commit
//! time, repaired so that no parent appears before any of its children; or,
//! when asked ([`Order::Topo`]), with each line of history kept together.
//!
//! Two layers: [`walk`] yields ids, parents and times for every commit
//! without decoding objects (the commit-graph file serves it when present),
//! and [`details`] decodes author, committer and summary for a window of ids.
//! [`log`] combines both for small repositories and the CLI.
//!
//! Sources: git-rev-list(1) (`--date-order` and `--topo-order` semantics),
//! gix revision walk documentation.

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap, HashSet},
};

use gix::bstr::ByteSlice;
use serde::{Deserialize, Serialize};

use crate::{Oid, Repository, Result, git_err};

/// Which commits to start walking from.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Tips {
    /// HEAD, all local branches, all remote branches and all tags.
    #[default]
    All,
    /// HEAD only.
    Head,
    /// Explicit commit ids.
    Ids(Vec<Oid>),
}

/// How the commits of a walk are ordered. A commit comes before all of its
/// parents either way.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "snake_case")]
pub enum Order {
    /// Newest first by commit date, as `git log --date-order`.
    #[default]
    Date,
    /// Each line of history together, not mixed with the others by their
    /// dates, as `git log --topo-order`.
    Topo,
    /// Newest first by the date the change was written, as `git log
    /// --author-date-order`. That date is in the commit and nowhere else,
    /// so every commit of the walk is read for it, as git reads them: a
    /// large history takes that much longer, and comes whole, not in
    /// pieces.
    AuthorDate,
}

/// Options for [`walk`] and [`log`].
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LogOptions {
    /// Starting points.
    #[serde(default)]
    pub tips: Tips,
    /// Stop after this many commits (in time order, before topological repair).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Follow only first parents.
    #[serde(default)]
    pub first_parent: bool,
    /// The order the commits come in.
    #[serde(default)]
    pub order: Order,
}

/// Author or committer identity with time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Signature {
    pub name: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub seconds: i64,
    /// Timezone offset in seconds east of UTC.
    pub offset: i32,
}

/// The minimum the graph needs per commit: identity, parents and time.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GraphCommit {
    /// Commit id.
    pub id: Oid,
    /// Parent ids, first parent first.
    pub parents: Vec<Oid>,
    /// Committer time in seconds since the Unix epoch, when known.
    pub seconds: Option<i64>,
}

/// One commit with its decoded metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitInfo {
    /// Commit id.
    pub id: Oid,
    /// Parent ids, first parent first.
    pub parents: Vec<Oid>,
    pub author: Signature,
    pub committer: Signature,
    /// First line of the message.
    pub summary: String,
    /// The rest of the message after the summary, trimmed; empty when absent.
    #[serde(default)]
    pub body: String,
}

/// Anything with an id and parents can be ordered and laid out as a graph.
pub trait GraphNode {
    /// Commit id.
    fn id(&self) -> Oid;
    /// Parent ids, first parent first.
    fn parents(&self) -> &[Oid];
    /// Committer time in seconds since the Unix epoch, when known: what
    /// decides which of two commits that may both come next comes first.
    fn seconds(&self) -> Option<i64> {
        None
    }
}

impl GraphNode for GraphCommit {
    fn id(&self) -> Oid {
        self.id
    }
    fn parents(&self) -> &[Oid] {
        &self.parents
    }
    fn seconds(&self) -> Option<i64> {
        self.seconds
    }
}

impl GraphNode for CommitInfo {
    fn id(&self) -> Oid {
        self.id
    }
    fn parents(&self) -> &[Oid] {
        &self.parents
    }
    fn seconds(&self) -> Option<i64> {
        Some(self.committer.seconds)
    }
}

fn signature(sig: gix::actor::SignatureRef<'_>) -> Signature {
    let time = sig.time().ok();
    Signature {
        name: sig.name.to_string(),
        email: sig.email.to_string(),
        seconds: time.map(|t| t.seconds).unwrap_or_default(),
        offset: time.map(|t| t.offset).unwrap_or_default(),
    }
}

/// Resolve the starting points of a walk to object ids, deduplicated, in
/// reference order. Branches are taken at their word: a branch holds a
/// commit, and looking thousands of them up in the object database to make
/// sure costs more than the walk itself. Tags are peeled and checked, since
/// tags of trees and blobs are legitimate. [`walk`] recovers when a branch
/// turns out to hold something else.
pub fn resolve_tips(repo: &Repository, tips: &Tips) -> Result<Vec<Oid>> {
    let r = repo.gix();
    let mut out: Vec<Oid> = Vec::new();
    let mut seen: HashSet<Oid> = HashSet::new();
    let mut push = |id: Oid| {
        if seen.insert(id) {
            out.push(id);
        }
    };
    match tips {
        Tips::Ids(ids) => ids.iter().copied().for_each(&mut push),
        Tips::Head => {
            if let Ok(id) = r.head_id() {
                push(Oid::from(id.detach()));
            }
        }
        Tips::All => {
            if let Ok(id) = r.head_id() {
                push(Oid::from(id.detach()));
            }
            let platform = r
                .references()
                .map_err(git_err(say!("log-listing-references")))?;
            // Peeled against the packed references read once: asked for
            // each tag, gitoxide looks at the file's date every time, which
            // on Windows takes most of the time spent here.
            let packed = r.refs.cached_packed_buffer().ok().flatten();
            for prefix in ["refs/heads/", "refs/remotes/", "refs/tags/"] {
                for reference in platform
                    .prefixed(prefix)
                    .map_err(git_err(say!("log-listing-references")))?
                {
                    let Ok(mut reference) = reference else {
                        continue;
                    };
                    let Some(direct) = reference.target().try_id().map(ToOwned::to_owned) else {
                        continue; // symbolic
                    };
                    if prefix != "refs/tags/" {
                        push(Oid::from(direct));
                        continue;
                    }
                    let Ok(id) =
                        reference.peel_to_id_packed(packed.as_ref().map(|buffer| &***buffer))
                    else {
                        continue;
                    };
                    let id = id.detach();
                    // Tags may point at trees or blobs; only commits can be
                    // walked. What an object is, is in its header.
                    if r.find_header(id)
                        .is_ok_and(|header| header.kind() == gix::object::Kind::Commit)
                    {
                        push(Oid::from(id));
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Walk commits reachable from the tips, newest first, child-before-parent,
/// without decoding commit bodies. This is what the graph is built from.
pub fn walk(repo: &Repository, options: &LogOptions) -> Result<Vec<GraphCommit>> {
    let tips = resolve_tips(repo, &options.tips)?;
    match walk_from(repo, &tips, options) {
        Ok(commits) => Ok(commits),
        // A starting point that is not a commit (a branch pointing at a tree,
        // say) stops the walk. Drop such tips and walk again.
        Err(first) => match commit_tips(repo, &tips) {
            Some(commits_only) => walk_from(repo, &commits_only, options),
            None => Err(first),
        },
    }
}

fn walk_from(repo: &Repository, tips: &[Oid], options: &LogOptions) -> Result<Vec<GraphCommit>> {
    let commits = walk_by_time(repo, tips, options, &mut |_| true)?.unwrap_or_default();
    Ok(ordered(repo, commits, options.order))
}

/// Put commits that are in the order the walk came to them into the order
/// asked for.
pub(crate) fn ordered(
    repo: &Repository,
    commits: Vec<GraphCommit>,
    order: Order,
) -> Vec<GraphCommit> {
    match order {
        Order::Date => topological_repair(commits),
        Order::Topo => topological_order(commits),
        Order::AuthorDate => {
            let written = author_seconds(repo, &commits);
            repaired_by(commits, |at, _| written.get(at).copied().flatten())
        }
    }
}

/// When each commit was written, by its author's date: read from the
/// commit, which is the only place that has it. None for a commit that
/// cannot be read or whose date cannot.
fn author_seconds(repo: &Repository, commits: &[GraphCommit]) -> Vec<Option<i64>> {
    let r = repo.gix();
    commits
        .iter()
        .map(|commit| {
            let read = r.find_commit(*commit.id.as_gix()).ok()?;
            Some(read.author().ok()?.time().ok()?.seconds)
        })
        .collect()
}

/// The commits reachable from `tips` in the order the walk comes to them:
/// newest first by commit time, not yet repaired or reordered. `each` is
/// called after every commit with all of them so far; when it says no, the
/// walk stops and there is no result.
pub(crate) fn walk_by_time(
    repo: &Repository,
    tips: &[Oid],
    options: &LogOptions,
    each: &mut dyn FnMut(&[GraphCommit]) -> bool,
) -> Result<Option<Vec<GraphCommit>>> {
    let r = repo.gix();
    if tips.is_empty() {
        return Ok(Some(Vec::new()));
    }
    let mut platform = r.rev_walk(tips.iter().map(|id| *id.as_gix())).sorting(
        gix::revision::walk::Sorting::ByCommitTime(
            gix::traverse::commit::simple::CommitTimeOrder::NewestFirst,
        ),
    );
    if options.first_parent {
        platform = platform.first_parent_only();
    }
    let walk = platform.all().map_err(git_err(say!("log-starting-walk")))?;

    let mut commits = Vec::new();
    for info in walk {
        let info = info.map_err(git_err(say!("log-walking")))?;
        commits.push(GraphCommit {
            id: Oid::from(info.id),
            parents: info.parent_ids.iter().map(|id| Oid::from(*id)).collect(),
            seconds: info.commit_time,
        });
        if !each(&commits) {
            return Ok(None);
        }
        if options.limit.is_some_and(|limit| commits.len() >= limit) {
            break;
        }
    }
    Ok(Some(commits))
}

/// The starting points of a walk that are commits, when some are not: a
/// branch pointing at a tree stops a walk. None when all of them are.
pub(crate) fn commit_tips(repo: &Repository, tips: &[Oid]) -> Option<Vec<Oid>> {
    let commits_only: Vec<Oid> = tips
        .iter()
        .copied()
        .filter(|id| repo.gix().find_commit(*id.as_gix()).is_ok())
        .collect();
    (commits_only.len() != tips.len()).then_some(commits_only)
}

/// Add a row per stash to a walked history.
///
/// A stash is a commit with two or three parents: the commit it was made
/// on, then commits holding the index and the untracked files. Its row
/// keeps only the first, so it hangs off the commit it was made on and the
/// other two stay out of the graph. A row goes where its date puts it and
/// never below its parent. A stash whose parent is not among `commits` is
/// left out.
pub fn with_stashes(
    repo: &Repository,
    commits: Vec<GraphCommit>,
    stashes: &[Oid],
) -> Vec<GraphCommit> {
    with_stashes_in(repo, commits, stashes, Order::Date)
}

/// [`with_stashes`] for commits in the given order. In [`Order::Topo`] and
/// [`Order::AuthorDate`] the rows are not sorted by the commit's date, so
/// a stash has no place by its date: it goes right above the commit it
/// was made on.
pub fn with_stashes_in(
    repo: &Repository,
    commits: Vec<GraphCommit>,
    stashes: &[Oid],
    order: Order,
) -> Vec<GraphCommit> {
    struct Row {
        commit: GraphCommit,
        placed: bool,
        parent_seen: bool,
    }
    let r = repo.gix();
    let mut rows: Vec<Row> = Vec::new();
    for id in stashes {
        if rows.iter().any(|row| row.commit.id == *id) {
            continue;
        }
        let Ok(commit) = r.find_commit(*id.as_gix()) else {
            continue;
        };
        let Some(parent) = commit.parent_ids().next() else {
            continue;
        };
        rows.push(Row {
            commit: GraphCommit {
                id: *id,
                parents: vec![Oid::from(parent.detach())],
                seconds: commit.time().ok().map(|time| time.seconds),
            },
            placed: false,
            parent_seen: false,
        });
    }
    if rows.is_empty() {
        return commits;
    }
    // Newest first, like the commits.
    rows.sort_by_key(|row| Reverse(row.commit.seconds));
    let mut by_parent: HashMap<Oid, Vec<usize>> = HashMap::new();
    let mut by_id: HashMap<Oid, usize> = HashMap::new();
    for (ix, row) in rows.iter().enumerate() {
        by_id.insert(row.commit.id, ix);
        if let Some(parent) = row.commit.parents.first() {
            by_parent.entry(*parent).or_default().push(ix);
        }
    }

    let mut out = Vec::with_capacity(commits.len().saturating_add(rows.len()));
    let mut next = 0usize;
    for commit in commits {
        // Something else points at the stash commit and the walk found it:
        // that row stands, with all its parents.
        if let Some(row) = by_id.get(&commit.id).and_then(|ix| rows.get_mut(*ix)) {
            if !row.placed {
                row.placed = true;
                row.parent_seen = true;
                out.push(commit);
            }
            continue;
        }
        // By date: every stash not older than this commit goes above it.
        while let Some(row) = rows.get_mut(next) {
            if order != Order::Date || row.commit.seconds < commit.seconds {
                break;
            }
            if !row.placed {
                row.placed = true;
                out.push(row.commit.clone());
            }
            next = next.saturating_add(1);
        }
        // By parent: an older stash still goes above the commit it is on.
        if let Some(children) = by_parent.get(&commit.id) {
            for ix in children {
                if let Some(row) = rows.get_mut(*ix) {
                    row.parent_seen = true;
                    if !row.placed {
                        row.placed = true;
                        out.push(row.commit.clone());
                    }
                }
            }
        }
        out.push(commit);
    }
    // A row placed by its date whose parent never came has nothing to hang
    // off.
    let adrift: Vec<Oid> = rows
        .iter()
        .filter(|row| row.placed && !row.parent_seen)
        .map(|row| row.commit.id)
        .collect();
    if !adrift.is_empty() {
        out.retain(|commit| !adrift.contains(&commit.id));
    }
    out
}

/// The commits at the edge of a shallow clone: their parents were left
/// on the remote, and git takes each of them for a root (`git log
/// --format=%P` lists no parent, `git show` lists every file as added).
/// Empty in a repository that is not shallow.
fn shallow_edge(r: &gix::Repository) -> HashSet<gix::ObjectId> {
    match r.shallow_commits() {
        Ok(Some(edge)) => edge.iter().copied().collect(),
        _ => HashSet::new(),
    }
}

/// A commit's parents as git has them: what the commit names, except at
/// the edge of a shallow clone, where it has none.
fn parents_of(commit: &gix::Commit<'_>, edge: &HashSet<gix::ObjectId>) -> Vec<Oid> {
    if edge.contains(&commit.id) {
        return Vec::new();
    }
    commit.parent_ids().map(|p| Oid::from(p.detach())).collect()
}

/// The parents of a commit as git has them: none for a commit at the
/// edge of a shallow clone (see [`details`]). Empty when `id` names no
/// commit that is here.
pub fn parents(repo: &Repository, id: Oid) -> Vec<Oid> {
    let r = repo.gix();
    match r.find_commit(*id.as_gix()) {
        Ok(commit) => parents_of(&commit, &shallow_edge(r)),
        Err(_) => Vec::new(),
    }
}

/// Decode metadata for the given commits, in the given order. Ids that do
/// not resolve to a commit are skipped. A commit at the edge of a shallow
/// clone has no parents, as git lists it.
pub fn details(repo: &Repository, ids: &[Oid]) -> Result<Vec<CommitInfo>> {
    let r = repo.gix();
    let edge = shallow_edge(r);
    let mut out = Vec::with_capacity(ids.len());
    for id in ids {
        let Ok(commit) = r.find_commit(*id.as_gix()) else {
            continue;
        };
        let author = commit
            .author()
            .map_err(git_err(say!("log-decoding-author")))?;
        let committer = commit
            .committer()
            .map_err(git_err(say!("log-decoding-committer")))?;
        let (summary, body) = subject_and_body(
            commit
                .message_raw()
                .map_err(git_err(say!("log-decoding-message")))?,
        );
        out.push(CommitInfo {
            id: *id,
            parents: parents_of(&commit, &edge),
            author: signature(author),
            committer: signature(committer),
            summary,
            body,
        });
    }
    Ok(out)
}

/// A commit message as git shows it: the subject is the first paragraph
/// on one line, the body is what follows it. A paragraph ends at a blank
/// line, and a line of nothing but spaces and tabs is blank: old history
/// separates subject and body with such lines, and the whole message would
/// otherwise come out as its subject.
pub(crate) fn subject_and_body(raw: &[u8]) -> (String, String) {
    let space = |byte: &u8| matches!(byte, b' ' | b'\t' | b'\r' | b'\n');
    let blank = |line: &[u8]| line.iter().all(space);
    let mut lines = raw.split(|byte| *byte == b'\n').peekable();
    while lines.peek().is_some_and(|line| blank(line)) {
        lines.next();
    }
    let mut subject: Vec<u8> = Vec::new();
    for line in lines.by_ref() {
        if blank(line) {
            break;
        }
        if !subject.is_empty() {
            subject.push(b' ');
        }
        let end = line
            .iter()
            .rposition(|byte| !space(byte))
            .map_or(0, |last| last.saturating_add(1));
        subject.extend(line.iter().take(end));
    }
    let body = lines.collect::<Vec<_>>().join(&b'\n');
    (
        subject.to_str_lossy().into_owned(),
        body.to_str_lossy().trim().to_owned(),
    )
}

/// The full raw message of a commit.
pub fn message(repo: &Repository, id: Oid) -> Result<String> {
    let commit = repo
        .gix()
        .find_commit(*id.as_gix())
        .map_err(git_err(say!("log-loading-commit")))?;
    Ok(commit
        .message_raw()
        .map_err(git_err(say!("log-decoding-message")))?
        .to_str_lossy()
        .into_owned())
}

/// List commits reachable from the tips with full metadata, newest first,
/// child-before-parent. Decodes every commit; prefer [`walk`] plus
/// [`details`] on the visible window for large repositories.
pub fn log(repo: &Repository, options: &LogOptions) -> Result<Vec<CommitInfo>> {
    let walked = walk(repo, options)?;
    let ids: Vec<Oid> = walked.iter().map(|c| c.id).collect();
    details(repo, &ids)
}

/// Reorder so that every commit precedes all of its parents and, among the
/// commits that may come next (all their children are out), the newest
/// one does: what `git rev-list --date-order` does. Commits of one date
/// come in the order they became free to, the ones no other commit points
/// at first, in the incoming order; a commit without a date goes by its
/// place in the incoming order alone, after the dated ones.
///
/// The incoming order is newest first as the walk found the commits, which
/// is this order already unless a clock was wrong somewhere: a parent
/// dated after its child is found early and has to wait, and what is
/// free to come meanwhile is then chosen by date, not by where the walk
/// happened to find it.
pub fn topological_repair<T: GraphNode>(commits: Vec<T>) -> Vec<T> {
    repaired_by(commits, |_, commit| commit.seconds())
}

/// [`topological_repair`] by another date than the commit's own:
/// `seconds` says it for the commit at a place in the incoming order.
/// With the author's date this is `git rev-list --author-date-order`.
pub(crate) fn repaired_by<T: GraphNode>(
    commits: Vec<T>,
    seconds: impl Fn(usize, &T) -> Option<i64>,
) -> Vec<T> {
    let n = commits.len();
    let index: crate::oid::OidMap<usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id(), i))
        .collect();
    let mut remaining_children = vec![0usize; n];
    for commit in &commits {
        for parent in commit.parents() {
            if let Some(&j) = index.get(parent)
                && let Some(count) = remaining_children.get_mut(j)
            {
                *count += 1;
            }
        }
    }
    // Newest first, then first come first served.
    let key = |commit: usize, turn: usize| {
        let seconds = commits.get(commit).and_then(|found| seconds(commit, found));
        (seconds.is_some(), seconds, Reverse(turn), commit)
    };
    let mut ready: BinaryHeap<(bool, Option<i64>, Reverse<usize>, usize)> = remaining_children
        .iter()
        .enumerate()
        .filter(|(_, count)| **count == 0)
        .map(|(i, _)| key(i, i))
        .collect();
    let mut turn = n;
    let mut order = Vec::with_capacity(n);
    while let Some((_, _, _, i)) = ready.pop() {
        order.push(i);
        let Some(commit) = commits.get(i) else {
            continue;
        };
        for parent in commit.parents() {
            if let Some(&j) = index.get(parent)
                && let Some(count) = remaining_children.get_mut(j)
            {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push(key(j, turn));
                    turn = turn.saturating_add(1);
                }
            }
        }
    }
    if order.len() != n {
        // Only possible with a corrupt (cyclic) history; keep the time order.
        return commits;
    }
    let mut slots: Vec<Option<T>> = commits.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|i| slots.get_mut(i).and_then(Option::take))
        .collect()
}

/// Reorder so that every commit precedes all of its parents and a line of
/// history stays together: after a commit comes the parent that became free
/// last, and a line that started goes on until it reaches a commit that
/// still has children to come. The commits without children start, in the
/// incoming (time) order. This is what `git rev-list --topo-order` does.
pub fn topological_order<T: GraphNode>(commits: Vec<T>) -> Vec<T> {
    let n = commits.len();
    let index: crate::oid::OidMap<usize> = commits
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id(), i))
        .collect();
    let mut remaining_children = vec![0usize; n];
    for commit in &commits {
        for parent in commit.parents() {
            if let Some(&j) = index.get(parent)
                && let Some(count) = remaining_children.get_mut(j)
            {
                *count += 1;
            }
        }
    }
    // Taken from the end: the first of the incoming order goes first.
    let mut ready: Vec<usize> = remaining_children
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, count)| **count == 0)
        .map(|(i, _)| i)
        .collect();
    let mut order = Vec::with_capacity(n);
    while let Some(i) = ready.pop() {
        order.push(i);
        let Some(commit) = commits.get(i) else {
            continue;
        };
        for parent in commit.parents() {
            if let Some(&j) = index.get(parent)
                && let Some(count) = remaining_children.get_mut(j)
            {
                *count = count.saturating_sub(1);
                if *count == 0 {
                    ready.push(j);
                }
            }
        }
    }
    if order.len() != n {
        // Only possible with a corrupt (cyclic) history; keep the time order.
        return commits;
    }
    let mut slots: Vec<Option<T>> = commits.into_iter().map(Some).collect();
    order
        .into_iter()
        .filter_map(|i| slots.get_mut(i).and_then(Option::take))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commit(id: u8, parents: &[u8]) -> CommitInfo {
        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        CommitInfo {
            id: oid(id),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            author: Signature::default(),
            committer: Signature::default(),
            summary: format!("c{id}"),
            body: String::new(),
        }
    }

    #[test]
    fn moves_parents_after_children() {
        // Time order lists parent 2 before its child 3 (clock skew).
        let input = vec![
            commit(1, &[2]),
            commit(2, &[4]),
            commit(3, &[2]),
            commit(4, &[]),
        ];
        let out = topological_repair(input);
        let ids: Vec<String> = out.iter().map(|c| c.summary.clone()).collect();
        assert_eq!(ids, ["c1", "c3", "c2", "c4"]);
    }

    #[test]
    fn keeps_time_order_when_already_valid() {
        let input = vec![commit(1, &[2]), commit(2, &[3]), commit(3, &[])];
        let out = topological_repair(input.clone());
        assert_eq!(out, input);
    }

    #[test]
    fn tolerates_parents_outside_the_window() {
        let input = vec![commit(1, &[9]), commit(2, &[9])];
        assert_eq!(topological_repair(input.clone()), input);
    }

    #[test]
    fn topological_order_keeps_a_line_together() {
        // By time: 6 = merge(5, 4); 5 -> 3; 4 -> 2; 3 -> 1; 2 -> 1; 1 root.
        // The two lines alternate by time; in topological order the line
        // that was merged in comes whole, then the one merged into.
        let input = vec![
            commit(6, &[5, 4]),
            commit(5, &[3]),
            commit(4, &[2]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        let summaries = |commits: &[CommitInfo]| -> Vec<String> {
            commits.iter().map(|c| c.summary.clone()).collect()
        };
        assert_eq!(
            summaries(&topological_repair(input.clone())),
            ["c6", "c5", "c4", "c3", "c2", "c1"]
        );
        assert_eq!(
            summaries(&topological_order(input)),
            ["c6", "c4", "c2", "c5", "c3", "c1"]
        );
        // Two tips: the first of the incoming order starts, and its line
        // runs until it meets a commit the other tip still leads to.
        let two = vec![
            commit(4, &[2]),
            commit(3, &[1]),
            commit(2, &[1]),
            commit(1, &[]),
        ];
        assert_eq!(summaries(&topological_order(two)), ["c4", "c2", "c3", "c1"]);
    }

    #[test]
    fn topological_order_survives_hostile_input() {
        // A parent listed twice, a parent outside the window, a commit that
        // is its own parent, a cycle of two, and nothing at all.
        let twice = vec![commit(2, &[1, 1]), commit(1, &[])];
        assert_eq!(topological_order(twice.clone()), twice);
        let outside = vec![commit(1, &[9]), commit(2, &[9])];
        assert_eq!(topological_order(outside.clone()), outside);
        let own = vec![commit(1, &[1])];
        assert_eq!(topological_order(own.clone()), own);
        let cycle = vec![commit(1, &[2]), commit(2, &[1]), commit(3, &[])];
        assert_eq!(topological_order(cycle.clone()), cycle);
        let none: Vec<CommitInfo> = Vec::new();
        assert!(topological_order(none).is_empty());
    }

    #[test]
    fn graph_commits_order_too() {
        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        let input = vec![
            GraphCommit {
                id: oid(1),
                parents: vec![oid(2)],
                seconds: Some(3),
            },
            GraphCommit {
                id: oid(2),
                parents: vec![],
                seconds: Some(9),
            },
            GraphCommit {
                id: oid(3),
                parents: vec![oid(2)],
                seconds: Some(1),
            },
        ];
        let out = topological_repair(input);
        assert_eq!(
            out.iter().map(|c| c.id).collect::<Vec<_>>(),
            [oid(1), oid(3), oid(2)]
        );
    }

    fn dated(id: u8, parents: &[u8], seconds: Option<i64>) -> GraphCommit {
        let oid = |b: u8| -> Oid { format!("{b:02x}").repeat(20).parse().unwrap() };
        GraphCommit {
            id: oid(id),
            parents: parents.iter().map(|p| oid(*p)).collect(),
            seconds,
        }
    }

    fn order_of(input: Vec<GraphCommit>) -> Vec<u8> {
        topological_repair(input)
            .iter()
            .map(|c| c.id.as_gix().as_bytes().first().copied().unwrap())
            .collect()
    }

    /// Among the commits free to come next the newest comes, wherever the
    /// walk found it: a parent dated after its child (5) is found early,
    /// has to wait for its other child (4), and what is free meanwhile is
    /// taken by date.
    #[test]
    fn what_may_come_next_comes_by_date() {
        let input = vec![
            dated(1, &[5], Some(100)),
            dated(5, &[9], Some(90)),
            dated(2, &[6], Some(40)),
            dated(6, &[9], Some(95)),
            dated(3, &[9], Some(30)),
            dated(4, &[5], Some(35)),
            dated(9, &[], Some(1)),
        ];
        // 1; then 2 (40) of 2, 3, 4; that frees 6 (95), the newest; then
        // 4 (35), which frees 5 (90), before 3 (30) that came in ahead of
        // both; the root last.
        assert_eq!(order_of(input), [1, 2, 6, 4, 5, 3, 9]);
        // Out of order altogether, and nothing to hold anything back.
        let roots = vec![
            dated(1, &[], Some(10)),
            dated(2, &[], Some(30)),
            dated(3, &[], Some(20)),
        ];
        assert_eq!(order_of(roots), [2, 3, 1]);
    }

    /// One date: first free, first out; the ones nothing points at in
    /// the order they came in. No date: after the dated ones, as they came.
    #[test]
    fn commits_of_one_date_keep_their_turn() {
        let input = vec![
            dated(1, &[4], Some(7)),
            dated(2, &[5], Some(7)),
            dated(3, &[], Some(7)),
            dated(4, &[], Some(7)),
            dated(5, &[], Some(7)),
        ];
        assert_eq!(order_of(input), [1, 2, 3, 4, 5]);
        let undated = vec![
            dated(1, &[], None),
            dated(2, &[], Some(-5)),
            dated(3, &[], None),
            dated(4, &[], Some(i64::MIN)),
        ];
        assert_eq!(order_of(undated), [2, 4, 1, 3]);
    }

    #[test]
    fn the_subject_is_the_first_paragraph_as_git_has_it() {
        let split = |raw: &str| {
            let (subject, body) = subject_and_body(raw.as_bytes());
            (subject, body)
        };
        let both = |subject: &str, body: &str| (subject.to_owned(), body.to_owned());
        assert_eq!(split("subject\n\nbody\n"), both("subject", "body"));
        // A line of spaces or tabs ends the subject like an empty one.
        assert_eq!(
            split("subject\n \nbody one\nbody two\n"),
            both("subject", "body one\nbody two")
        );
        assert_eq!(split("subject\n\t \t\nbody"), both("subject", "body"));
        // A subject over several lines is one line; what trails a line goes.
        assert_eq!(
            split("first  \nsecond\t\n\nbody\n\nmore\n"),
            both("first second", "body\n\nmore")
        );
        // Blank lines before it are not it; what leads a line stays.
        assert_eq!(
            split("\n  \n  indented\n\nbody"),
            both("  indented", "body")
        );
        assert_eq!(split("subject\r\n\r\nbody\r\n"), both("subject", "body"));
        assert_eq!(split("only a subject"), both("only a subject", ""));
        assert_eq!(split(""), both("", ""));
        assert_eq!(split("\n\n \n"), both("", ""));
        // Bytes that are no text are shown as what can be shown of them.
        let (subject, _) = subject_and_body(b"caf\xe9\n\nbody");
        assert_eq!(subject, "caf\u{fffd}");
    }
}
