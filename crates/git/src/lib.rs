//! bitbraid-git: the repository layer.
//!
//! Reads go through gitoxide (`gix`): the references and the snapshot the
//! references panel shows, the revision walk and the lane layout of the
//! commit graph ([`graph`], [`pieces`]), status, diffs, blame, file
//! history, search, configuration and the index. Index writes of single
//! hunks and lines go through libgit2 (`git2`, [`stage`]). Everything that
//! has to behave exactly like the user's git runs the git executable
//! ([`exe`]): commits, branches and tags, fetch, pull and push with the
//! answers to git's questions ([`askpass`]), merge, rebase and the
//! operations stopped in the middle, stashes, signing and the checking of
//! signatures, hooks, LFS, maintenance and the external diff and merge
//! tools. A watcher ([`watch`]) says when the repository changed on disk.
//! Each read and each change is held to the real git in the tests.
//!
//! Sources: gix documentation (docs.rs/gix), the `git2` crate's and
//! libgit2's documentation, git-rev-list(1) for ordering semantics,
//! git-for-each-ref(1), git-worktree(1), git-submodule(1), git-stash(1),
//! git-status(1), git-diff(1), git-blame(1); each module lists its own.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

// `say!` and `message!`, in every module: the words come from the catalog.
#[macro_use]
extern crate bitbraid_strings;

pub mod activity;
pub mod askpass;
pub mod blame;
pub mod blame_revs;
pub mod branch;
pub mod commit;
pub mod compare;
pub mod create;
pub mod diff;
pub mod exe;
pub mod files_change;
mod filters;
pub mod find;
pub mod flow;
pub mod glance;
pub mod graph;
pub mod history;
pub mod hold;
pub mod hooks;
mod hookwatch;
pub mod housekeeping;
pub mod identity;
pub mod ignore;
pub mod ignore_templates;
pub mod index_flags;
pub mod interactive;
pub mod issue_keys;
pub mod known_tools;
pub mod lfs;
pub mod log;
pub mod long_paths;
pub mod maintenance;
pub mod message;
pub mod oid;
pub mod patch;
pub mod pieces;
pub mod promisor;
pub mod remote;
pub mod repo;
pub mod rerere;
pub mod rerere_record;
pub mod rewrite;
pub mod rewrite_one;
pub mod search;
pub mod sequence;
pub mod settings;
pub mod shipped;
pub mod signature;
pub mod signers;
pub mod snapshot;
pub mod sparse;
pub mod stage;
pub mod stand_in;
pub mod stash;
pub mod status;
pub mod strings;
pub mod submodule;
pub mod system_config;
pub mod tag;
pub mod tools;
pub mod tracked;
pub mod version;
pub mod watch;
pub mod wip;
pub mod worktree;

pub use blame::{BlameLine, blame};
pub use branch::{CheckoutOutcome, DeletedBranch, Dirty, HeadState};
pub use commit::{
    CommitOutcome, CommitRequest, Person, commit, move_head, recent_people, template,
};
pub use create::{CloneFilter, CloneOptions, Cloned, InitOptions, clone_dir_name};
pub use diff::{
    DiffOptions, DiffSide, FileDiff, FileLineEndings, Hunk, Line, LineEndings, LineKind,
    Whitespace, apply_hunks, commit_files, diff_bytes, diff_bytes_with, diff_file, diff_file_with,
    diff_trees, file_bytes,
};
pub use graph::{
    GraphRow, LaneRef, MAX_PINNED, ascii, layout, layout_piece, layout_pinned, layout_with_wip,
    layout_with_wip_pinned,
};
pub use history::{HistoryEntry, HistoryOptions, file_history};
pub use ignore::{IgnoreTarget, IgnoreWhat, ignore_add};
pub use log::{
    CommitInfo, GraphCommit, GraphNode, LogOptions, Order, Signature, Tips, details, log, message,
    parents, topological_order, walk, with_stashes, with_stashes_in,
};
pub use oid::Oid;
pub use remote::{Force, Progress, PullMode, PushOptions, Reporter};
pub use repo::{RepoSummary, Repository};
pub use rewrite::{MergeMode, Moved, RebaseOptions, ResetMode};
pub use snapshot::{Snapshot, remote_fetch_urls, snapshot};
pub use stage::{
    Backup, HunkSelection, apply_selected, discard_hunks, discard_paths, restore_backup,
    restore_path_at, stage_all, stage_hunks, stage_paths, unapply_selected, unstage_hunks,
    unstage_paths, worktree_path,
};
pub use stash::{StashEntry, StashOptions};
pub use status::{ChangeKind, STATUS_LIMIT, StatusEntry, status, status_bounded};
pub use watch::{Area, ChangeSet, Watcher};

/// Errors from the repository model.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The path is not inside a git repository, or it cannot be opened.
    #[error("{}", say!("errors-open", path = .path.display().to_string(), source = .source.to_string()))]
    Open {
        /// Path that was tried.
        path: std::path::PathBuf,
        /// Underlying error.
        #[source]
        source: BoxedError,
    },
    /// A gitoxide operation failed. What it says goes down to why: the
    /// first message is often only the step that failed (see
    /// `with_causes`), and a reason that is in a cause, such as the object
    /// database having no room for the packs on disk
    /// ([`repo::is_out_of_room`]), is known by what is said.
    #[error("{context}: {}", explained(source.as_ref()))]
    Git {
        /// What was being done.
        context: &'static str,
        /// Underlying error.
        #[source]
        source: BoxedError,
    },
    /// Input that cannot be used.
    #[error("{}", say!("errors-invalid", what = *.what, detail = .detail.as_str()))]
    Invalid {
        /// Which input.
        what: &'static str,
        /// Why.
        detail: String,
    },
    /// Git ran and reported failure. Where what it said is that a path is
    /// too long, what to do about it follows ([`long_paths::advice`]).
    #[error("{}{}", say!("errors-command", command = .command.as_str(), status = .status.as_str(), stderr = .stderr.as_str()), long_paths::after(stderr))]
    Command {
        /// `git` and its arguments, without credentials.
        command: String,
        /// How the process ended.
        status: String,
        /// What it wrote to standard error.
        stderr: String,
    },
    /// An operation that was not carried out, for a reason the user can do
    /// something about. Nothing was changed.
    #[error("{what}: {detail}")]
    Refused {
        /// Which operation.
        what: &'static str,
        /// Why.
        detail: String,
    },
    /// A hook of the repository ended badly, and git ended badly for it:
    /// not a failure of git's own. The message says which hook,
    /// what it said, and what there is to do.
    #[error("{}", hookwatch::describe(hook, *code, said, *after))]
    Hook {
        /// The hook's name, as githooks(5) has it.
        hook: String,
        /// What the hook ended with.
        code: i32,
        /// The end of what was written while it ran.
        said: String,
        /// It runs after the fact (a `post-` hook): what was asked for
        /// was done all the same.
        after: bool,
    },
}

/// Boxed source error.
pub type BoxedError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Result alias for this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A path as text for a result, also when it is not UTF-8, which serde
/// would refuse to write: a repository in such a directory is still one
/// to work with.
pub(crate) fn path_text<S: serde::Serializer>(
    path: &std::path::Path,
    to: S,
) -> std::result::Result<S::Ok, S::Error> {
    to.serialize_str(&path.to_string_lossy())
}

/// [`path_text`] for a path that may be absent.
pub(crate) fn opt_path_text<S: serde::Serializer>(
    path: &Option<std::path::PathBuf>,
    to: S,
) -> std::result::Result<S::Ok, S::Error> {
    match path {
        Some(path) => to.serialize_some(&path.to_string_lossy()),
        None => to.serialize_none(),
    }
}

/// What an error says, followed by what each error it came from says
/// where that adds something: gitoxide's own message is often only the
/// step that failed ("delegate.traverse(…) failed"), and why is below it.
pub(crate) fn with_causes(err: &(dyn std::error::Error + 'static)) -> String {
    let mut said = err.to_string();
    let mut next = err.source();
    while let Some(cause) = next {
        let more = cause.to_string();
        if !more.is_empty() && !said.contains(&more) {
            said.push_str(": ");
            said.push_str(&more);
        }
        next = cause.source();
    }
    said
}

/// [`with_causes`], and when that says a path is too long, what to do
/// about it ([`long_paths::advice`]).
fn explained(err: &(dyn std::error::Error + 'static)) -> String {
    let said = with_causes(err);
    let advice = long_paths::after(&said);
    said + &advice
}

pub(crate) fn git_err<E>(context: &'static str) -> impl FnOnce(E) -> Error
where
    E: std::error::Error + Send + Sync + 'static,
{
    move |source| Error::Git {
        context,
        source: Box::new(source),
    }
}
