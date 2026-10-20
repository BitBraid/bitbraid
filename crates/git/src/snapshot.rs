//! A snapshot of everything the references panel shows: HEAD, local branches,
//! remotes with their branches, tags, stashes, worktrees and submodules.
//!
//! Sources: git-for-each-ref(1), git-stash(1) (stash list is the reflog of
//! `refs/stash`), git-worktree(1), gitrepository-layout(5) (the per-worktree
//! `HEAD` file), git-submodule(1), gix documentation.

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Oid, Repository, Result, git_err};

/// Where HEAD points.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Head {
    /// Detached HEAD (points at a commit, not a branch).
    pub detached: bool,
    /// Short branch name when HEAD is symbolic, even if unborn.
    pub branch: Option<String>,
    /// Commit id, absent when the branch is unborn.
    pub target: Option<Oid>,
    /// With a detached HEAD: the local branch it was on before, when that
    /// branch still exists. Where "go back" goes.
    pub previous: Option<String>,
}

/// A local branch.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Branch {
    /// Short name, for example `feature/a`.
    pub name: String,
    /// Full name, for example `refs/heads/feature/a`.
    pub full_name: String,
    /// Commit the branch points at.
    pub target: Oid,
    /// Configured upstream in short form, for example `origin/main`.
    pub upstream: Option<String>,
    /// Remote the upstream belongs to.
    pub remote: Option<String>,
    /// Whether HEAD is on this branch.
    pub is_head: bool,
}

/// A branch on a remote (`refs/remotes/<remote>/<name>`).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RemoteBranch {
    /// Name without the remote prefix.
    pub name: String,
    /// Full reference name.
    pub full_name: String,
    /// Commit the branch points at.
    pub target: Oid,
}

/// A configured remote and its tracking branches.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Remote {
    /// Remote name, for example `origin`.
    pub name: String,
    /// Fetch URL.
    pub fetch_url: Option<String>,
    /// Push URL when it differs from the fetch URL.
    pub push_url: Option<String>,
    /// The branch the remote's `HEAD` names here
    /// (`refs/remotes/<remote>/HEAD`), without the remote's name: its
    /// default branch as of the clone or the last `git remote set-head`.
    pub head: Option<String>,
    /// Tracking branches.
    pub branches: Vec<RemoteBranch>,
}

/// A tag.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Tag {
    /// Short name.
    pub name: String,
    /// Object the tag resolves to after peeling (usually a commit).
    pub target: Oid,
    /// Tag object id for annotated tags.
    pub tag_object: Option<Oid>,
    pub annotated: bool,
    /// Message of an annotated tag.
    pub message: Option<String>,
}

/// A stash entry, newest first (`stash@{0}` is index 0).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Stash {
    /// Position in the stash list.
    pub index: usize,
    /// The stash commit.
    pub target: Oid,
    /// Message as recorded in the reflog.
    pub message: String,
}

/// A worktree, including the main one.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Worktree {
    /// Worktree id (`main` for the main worktree, the directory name otherwise).
    pub id: String,
    /// Path of the working directory.
    #[serde(serialize_with = "crate::opt_path_text")]
    pub path: Option<PathBuf>,
    /// Commit checked out, if resolvable.
    pub head: Option<Oid>,
    /// Branch checked out, if any.
    pub branch: Option<String>,
    /// Locked against removal.
    pub locked: bool,
    /// Reason given for the lock.
    pub lock_reason: Option<String>,
    pub is_main: bool,
    /// Whether this is the worktree the repository was opened in.
    pub current: bool,
}

/// A submodule as declared in `.gitmodules`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Submodule {
    pub name: String,
    /// Path relative to the worktree.
    pub path: String,
    /// Configured URL.
    pub url: Option<String>,
    /// Commit checked out inside the submodule, when it is initialised.
    pub head: Option<Oid>,
    /// Commit recorded in the superproject's index.
    pub index: Option<Oid>,
    pub active: bool,
}

/// Everything the references panel needs.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct Snapshot {
    pub head: Head,
    /// Local branches, sorted by name.
    pub branches: Vec<Branch>,
    /// Remotes, sorted by name, each with its branches sorted by name.
    pub remotes: Vec<Remote>,
    /// Tags, sorted by name.
    pub tags: Vec<Tag>,
    /// Stashes, newest first.
    pub stashes: Vec<Stash>,
    /// Worktrees, main first.
    pub worktrees: Vec<Worktree>,
    /// Submodules, in `.gitmodules` order.
    pub submodules: Vec<Submodule>,
    /// Where a walk of the whole history starts, as
    /// [`crate::log::resolve_tips`] finds it for every reference: HEAD,
    /// then each branch, reference under `refs/remotes/` and tag that
    /// names a commit, in the order the references are read, each commit
    /// once. Read with the references, so that a graph laid out from this
    /// snapshot need not read them all again, which with thousands of loose
    /// references takes a quarter of a second on Windows.
    #[serde(skip)]
    pub walk_tips: Vec<Oid>,
}

/// The starting points of a walk of everything, each commit once, in the
/// order they come.
#[derive(Debug, Default)]
struct WalkTips {
    ids: Vec<Oid>,
    seen: std::collections::HashSet<Oid>,
}

impl WalkTips {
    fn add(&mut self, id: Oid) {
        if self.seen.insert(id) {
            self.ids.push(id);
        }
    }
}

fn head_info(repo: &gix::Repository) -> Result<Head> {
    let head = repo
        .head()
        .map_err(git_err(say!("snapshot-reading-head")))?;
    let detached = head.is_detached();
    Ok(Head {
        detached,
        branch: head.referent_name().map(|name| name.shorten().to_string()),
        target: head.id().map(|id| Oid::from(id.detach())),
        previous: if detached {
            previous_branch(repo)
        } else {
            None
        },
    })
}

/// The branch a checkout last left, from HEAD's reflog (`checkout: moving
/// from <branch> to <somewhere>`), if it is a local branch that still
/// exists. This is what `@{-1}` means to git.
fn previous_branch(repo: &gix::Repository) -> Option<String> {
    let head = repo.find_reference("HEAD").ok()?;
    let mut log = head.log_iter();
    let lines = log.rev().ok()??;
    // The last checkout is near the end; a log without one is not read whole.
    for line in lines.take(500) {
        let line = line.ok()?;
        let message = line.message.to_string();
        if let Some(moved) = message.strip_prefix("checkout: moving from ") {
            let (from, _) = moved.split_once(" to ")?;
            let exists = repo
                .try_find_reference(&format!("refs/heads/{from}"))
                .ok()
                .flatten()
                .is_some();
            return exists.then(|| from.to_owned());
        }
    }
    None
}

/// Commit a branch reference points at. Branches hold commits, so the target
/// is taken as it is: peeling would look every one of them up in the object
/// database, which adds up over thousands of remote branches.
fn branch_target(reference: &mut gix::Reference<'_>) -> Option<Oid> {
    match reference.target().try_id() {
        Some(id) => Some(Oid::from(id.to_owned())),
        // Symbolic: follow it.
        None => reference.peel_to_id().ok().map(|id| Oid::from(id.detach())),
    }
}

/// HEAD of a linked worktree as (commit, branch short name), read from the
/// `HEAD` file in its private git directory. Opening a repository per
/// worktree takes milliseconds each; this is one small file.
fn linked_worktree_head(repo: &gix::Repository, git_dir: &Path) -> (Option<Oid>, Option<String>) {
    let head = git_dir.join("HEAD");
    // A real HEAD is a line; refuse to slurp anything else.
    if !std::fs::metadata(&head).is_ok_and(|meta| meta.len() <= 4096) {
        return (None, None);
    }
    let Ok(text) = std::fs::read_to_string(&head) else {
        return (None, None);
    };
    let text = text.trim();
    match text.strip_prefix("ref:").map(str::trim) {
        Some("") => (None, None),
        Some(name) => {
            let target = repo
                .try_find_reference(name)
                .ok()
                .flatten()
                .and_then(|mut reference| reference.peel_to_id().ok())
                .map(|id| Oid::from(id.detach()));
            let short = name.strip_prefix("refs/heads/").unwrap_or(name);
            (target, Some(short.to_owned()))
        }
        None => (text.parse().ok(), None),
    }
}

/// Whether two paths name one directory, however each is written.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (
        bitbraid_core::paths::canonical(a),
        bitbraid_core::paths::canonical(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Where the remote `name` fetches from, and where it pushes to when that
/// is somewhere else.
fn remote_urls(r: &gix::Repository, name: &str) -> (Option<String>, Option<String>) {
    match r.find_remote(name) {
        Ok(remote) => {
            let fetch = remote
                .url(gix::remote::Direction::Fetch)
                .map(|url| url.to_bstring().to_string());
            let push = remote
                .url(gix::remote::Direction::Push)
                .map(|url| url.to_bstring().to_string())
                .filter(|push| Some(push) != fetch.as_ref());
            (fetch, push)
        }
        Err(_) => (None, None),
    }
}

/// The remotes by name, each with where it fetches from: what a snapshot
/// says of them ([`Remote::fetch_url`]), read from the configuration alone,
/// without a reference. In a repository with thousands of references a
/// snapshot takes tens of milliseconds, and the remotes' addresses are
/// asked for whenever the links to accounts are.
pub fn remote_fetch_urls(repo: &Repository) -> Vec<(String, Option<String>)> {
    let r = repo.gix();
    let mut remotes: Vec<(String, Option<String>)> = r
        .remote_names()
        .into_iter()
        .map(|name| {
            let name = name.to_string();
            let (fetch, _) = remote_urls(r, &name);
            (name, fetch)
        })
        .collect();
    remotes.sort_by(|a, b| a.0.cmp(&b.0));
    remotes
}

/// Take a snapshot of the repository's references and related state.
pub fn snapshot(repo: &Repository) -> Result<Snapshot> {
    let r = repo.gix();
    let head = head_info(r)?;
    let platform = r
        .references()
        .map_err(git_err(say!("snapshot-listing-references")))?;
    let mut walk_tips = WalkTips::default();
    if let Some(id) = head.target {
        walk_tips.add(id);
    }

    // Local branches.
    let mut branches = Vec::new();
    for reference in platform
        .prefixed("refs/heads/")
        .map_err(git_err(say!("snapshot-listing-local-branches")))?
    {
        let mut reference = reference.map_err(git_err(say!("snapshot-reading-branch")))?;
        let full_name = reference.name().as_bstr().to_string();
        let name = reference.name().shorten().to_string();
        // A walk takes a branch at its word, and a symbolic one not at all.
        if let Some(id) = reference.target().try_id() {
            walk_tips.add(Oid::from(id.to_owned()));
        }
        let Some(target) = branch_target(&mut reference) else {
            continue; // broken reference; nothing to show
        };
        let upstream = r
            .branch_remote_tracking_ref_name(reference.name(), gix::remote::Direction::Fetch)
            .and_then(|res| res.ok())
            .map(|tracking| tracking.shorten().to_string());
        let remote = r
            .branch_remote_name(reference.name().shorten(), gix::remote::Direction::Fetch)
            .map(|remote| remote.as_bstr().to_string());
        let is_head = !head.detached && head.branch.as_deref() == Some(name.as_str());
        branches.push(Branch {
            name,
            full_name,
            target,
            upstream,
            remote,
            is_head,
        });
    }
    branches.sort_by(|a, b| a.name.cmp(&b.name));

    // Remotes and their branches, from one reading of everything under
    // `refs/remotes/`: each reference goes to every remote whose folder it
    // is in, and one that names a commit is where a walk starts.
    let mut remotes: Vec<Remote> = r
        .remote_names()
        .into_iter()
        .map(|remote_name| {
            let name = remote_name.to_string();
            let (fetch_url, push_url) = remote_urls(r, &name);
            Remote {
                name,
                fetch_url,
                push_url,
                head: None,
                branches: Vec::new(),
            }
        })
        .collect();
    let prefixes: Vec<String> = remotes
        .iter()
        .map(|remote| format!("refs/remotes/{}/", remote.name))
        .collect();
    for reference in platform
        .prefixed("refs/remotes/")
        .map_err(git_err(say!("snapshot-listing-remote-branches")))?
    {
        let reference = reference.map_err(git_err(say!("snapshot-reading-remote-branch")))?;
        let full_name = reference.name().as_bstr().to_string();
        let direct = reference
            .target()
            .try_id()
            .map(|id| Oid::from(id.to_owned()));
        if let Some(id) = direct {
            walk_tips.add(id);
        }
        for (remote, prefix) in remotes.iter_mut().zip(&prefixes) {
            let Some(rest) = full_name.strip_prefix(prefix.as_str()) else {
                continue;
            };
            match direct {
                Some(target) => remote.branches.push(RemoteBranch {
                    name: rest.to_owned(),
                    full_name: full_name.clone(),
                    target,
                }),
                // Symbolic: refs/remotes/origin/HEAD names the default
                // branch, and is no branch itself.
                None if rest == "HEAD" => {
                    if let Some(named) = reference.target().try_name() {
                        remote.head = named
                            .as_bstr()
                            .to_string()
                            .strip_prefix(prefix.as_str())
                            .map(str::to_owned);
                    }
                }
                None => {}
            }
        }
    }
    for remote in &mut remotes {
        remote.branches.sort_by(|a, b| a.name.cmp(&b.name));
    }
    remotes.sort_by(|a, b| a.name.cmp(&b.name));

    // Tags, peeled against the packed references read once (see
    // `log::resolve_tips`).
    let packed = r.refs.cached_packed_buffer().ok().flatten();
    let mut tags = Vec::new();
    for reference in platform
        .prefixed("refs/tags/")
        .map_err(git_err(say!("snapshot-listing-tags")))?
    {
        let mut reference = reference.map_err(git_err(say!("snapshot-reading-tag")))?;
        let name = reference.name().shorten().to_string();
        let direct = reference
            .target()
            .try_id()
            .map(|id| Oid::from(id.to_owned()));
        let Ok(peeled) = reference.peel_to_id_packed(packed.as_ref().map(|buffer| &***buffer))
        else {
            continue;
        };
        let target = Oid::from(peeled.detach());
        // Tags of trees and blobs are no place for a walk to start.
        if r.find_header(*target.as_gix())
            .is_ok_and(|header| header.kind() == gix::object::Kind::Commit)
        {
            walk_tips.add(target);
        }
        let (annotated, tag_object, message) = match direct {
            Some(direct) if direct != target => {
                let message = r
                    .find_object(*direct.as_gix())
                    .ok()
                    .and_then(|object| object.try_into_tag().ok())
                    .and_then(|tag| tag.decode().ok().map(|tag| tag.message.to_string()));
                (true, Some(direct), message)
            }
            _ => (false, None, None),
        };
        tags.push(Tag {
            name,
            target,
            tag_object,
            annotated,
            message,
        });
    }
    tags.sort_by(|a, b| a.name.cmp(&b.name));

    // Stashes: the reflog of refs/stash, newest first.
    let mut stashes = Vec::new();
    if let Ok(Some(reference)) = r.try_find_reference("refs/stash") {
        let mut log = reference.log_iter();
        if let Ok(Some(lines)) = log.rev() {
            for (index, line) in lines.enumerate() {
                let Ok(line) = line else { break };
                stashes.push(Stash {
                    index,
                    target: Oid::from(line.new_oid),
                    message: line.message.to_string(),
                });
            }
        }
    }

    // Worktrees: the main one first, then linked ones. Opened in a linked
    // worktree, the main one is the one beside the shared git directory.
    let mut worktrees = Vec::new();
    let in_linked = !same_dir(r.git_dir(), r.common_dir());
    if in_linked {
        // Paths as the system writes them: on Windows git writes the
        // ones that lead from a worktree to the repository with `/`.
        let main_dir = r
            .main_repo()
            .ok()
            .and_then(|main| main.workdir().map(Path::to_path_buf))
            .map(bitbraid_core::paths::native);
        if let Some(path) = main_dir {
            let (main_head, main_branch) = linked_worktree_head(r, r.common_dir());
            worktrees.push(Worktree {
                id: "main".to_owned(),
                path: Some(path),
                head: main_head,
                branch: main_branch,
                locked: false,
                lock_reason: None,
                is_main: true,
                current: false,
            });
        }
    } else if let Some(workdir) = r.workdir() {
        worktrees.push(Worktree {
            id: "main".to_owned(),
            path: Some(workdir.to_path_buf()),
            head: head.target,
            branch: head.branch.clone(),
            locked: false,
            lock_reason: None,
            is_main: true,
            current: true,
        });
    }
    for proxy in r
        .worktrees()
        .map_err(git_err(say!("snapshot-listing-worktrees")))?
    {
        let id = proxy.id().map(|id| id.to_string()).unwrap_or_default();
        let path = proxy.base().ok().map(bitbraid_core::paths::native);
        let locked = proxy.is_locked();
        let lock_reason = proxy.lock_reason().map(|reason| reason.to_string());
        let (wt_head, wt_branch) = linked_worktree_head(r, proxy.git_dir());
        worktrees.push(Worktree {
            id,
            path,
            head: wt_head,
            branch: wt_branch,
            locked,
            lock_reason,
            is_main: false,
            current: in_linked && same_dir(proxy.git_dir(), r.git_dir()),
        });
    }

    // Submodules.
    let mut submodules = Vec::new();
    if let Some(iter) = r
        .submodules()
        .map_err(git_err(say!("snapshot-listing-submodules")))?
    {
        for sm in iter {
            submodules.push(Submodule {
                name: sm.name().to_string(),
                path: sm.path().map(|path| path.to_string()).unwrap_or_default(),
                url: sm.url().ok().map(|url| url.to_bstring().to_string()),
                head: sm.head_id().ok().flatten().map(Oid::from),
                index: sm.index_id().ok().flatten().map(Oid::from),
                active: sm.is_active().unwrap_or(false),
            });
        }
    }

    Ok(Snapshot {
        head,
        branches,
        remotes,
        tags,
        stashes,
        worktrees,
        submodules,
        walk_tips: walk_tips.ids,
    })
}
