//! Operations that move the current branch: merge, rebase, reset,
//! cherry-pick and revert, through git's own sequencer.
//!
//! A merge, rebase, cherry-pick or revert that meets a conflict stops
//! there and says so in its result: the repository is left in the middle
//! of the operation for [`crate::sequence`] to go on from. A squash merge
//! and a pick without commit keep no such state, so they are taken back
//! instead and the error names the files. When git is already in the
//! middle of something, every operation here refuses to begin.
//!
//! Before a merge or a rebase begins, [`predict`] merges the two tips in
//! memory and names the files that would conflict.
//!
//! Sources: git-merge(1), git-rebase(1), git-reset(1), git-cherry-pick(1),
//! git-revert(1), git-merge-tree(1).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    Backup, ChangeKind, Error, Oid, Repository, Result, StatusEntry, branch, branch::HeadState,
    exe, stage,
};

/// How a merge is recorded.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum MergeMode {
    /// Only when the branch can move forward.
    FfOnly,
    /// Move forward when possible, a merge commit otherwise.
    #[default]
    Ff,
    /// Always a merge commit.
    NoFf,
    /// Bring the changes in as staged work, without a commit.
    Squash,
    /// Merge, and stop before the merge commit is made: the merge is left
    /// in progress, to be looked at and then continued or aborted.
    NoCommit,
}

/// What is said when a merge stopped before its commit because that was
/// asked for.
pub fn stopped_before_commit() -> &'static str {
    say!("rewrite-stopped-before-commit")
}

/// How a rebase is to go.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct RebaseOptions {
    /// Move only the commits after this one (a branch, a tag, a commit)
    /// instead of everything the new base does not have: a range of the
    /// branch, onto another base (`git rebase --onto <base> <after>`).
    #[serde(default)]
    pub after: Option<String>,
    /// Set local changes aside before and bring them back after.
    #[serde(default)]
    pub autostash: bool,
    /// Without the `pre-rebase` hook (`git rebase --no-verify`).
    #[serde(default)]
    pub no_verify: bool,
}

/// What a rebase did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rebased {
    /// Where HEAD went.
    pub moved: Moved,
    /// The local changes that were set aside did not fit back in: git put
    /// them in the stash.
    pub stash_kept: bool,
}

/// What a reset keeps.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum ResetMode {
    /// Move the branch; index and files stay.
    Soft,
    /// Move the branch and reset the index; files stay.
    #[default]
    Mixed,
    /// Move the branch, reset the index and the files.
    Hard,
    /// Move the branch and update the files, keeping local changes; refused
    /// when they are in the way.
    Keep,
}

impl ResetMode {
    fn flag(self) -> &'static str {
        match self {
            ResetMode::Soft => "--soft",
            ResetMode::Mixed => "--mixed",
            ResetMode::Hard => "--hard",
            ResetMode::Keep => "--keep",
        }
    }
}

/// What one of these operations did to HEAD.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Moved {
    /// HEAD before.
    pub before: HeadState,
    /// HEAD after.
    pub after: HeadState,
    /// Backups of files a hard reset overwrote.
    pub backups: Vec<Backup>,
    /// The operation stopped half-way (on a conflict, say) and is still in
    /// progress: what git said. `after` is then where HEAD is for now.
    #[serde(default)]
    pub stopped: Option<String>,
    /// What an operation that leaves HEAD where it is (a squash, a pick
    /// without commit) did to the files and the index, for undo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub files: Option<crate::files_change::FilesChange>,
}

/// What to do about an operation that stops half-way.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OnStop {
    /// The operation keeps its state in the git directory: leave it there
    /// to be continued, skipped or aborted.
    Leave,
    /// The operation only touches the index and the files (a squash, a pick
    /// without commit): reset what it merged.
    ResetIndex,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

/// The commit a revision names; refuses what could be read as an option.
pub(crate) fn commit_of(repo: &Repository, what: &'static str, rev: &str) -> Result<Oid> {
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("rewrite-cannot-be-used", rev = format!("{rev:?}")),
        ));
    }
    let spec = format!("{rev}^{{commit}}");
    exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", spec.as_str()],
        None,
    )
    .ok()
    .and_then(|out| out.stdout.trim().parse().ok())
    .ok_or_else(|| invalid(what, say!("rewrite-not-a-commit", rev = format!("{rev:?}"))))
}

/// Paths git lists as unmerged right now.
fn conflicted(repo: &Repository) -> Vec<String> {
    exe::run(repo, ["diff", "--name-only", "--diff-filter=U", "-z"], None)
        .map(|out| {
            out.stdout
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// What git is in the middle of in this worktree, if anything.
pub fn in_progress(repo: &Repository) -> Option<&'static str> {
    let dir = repo.git_dir();
    [
        ("MERGE_HEAD", message!("rewrite-in-progress-merge")),
        ("rebase-merge", message!("rewrite-in-progress-rebase")),
        ("rebase-apply", message!("rewrite-in-progress-rebase")),
        (
            "CHERRY_PICK_HEAD",
            message!("rewrite-in-progress-cherry-pick"),
        ),
        ("REVERT_HEAD", message!("rewrite-in-progress-revert")),
        ("sequencer", message!("rewrite-in-progress-sequencer")),
        ("BISECT_LOG", message!("rewrite-in-progress-bisect")),
    ]
    .into_iter()
    .find(|(name, _)| dir.join(name).symlink_metadata().is_ok())
    .map(|(_, what)| what.text())
}

/// Refuse to start `what` on top of an unfinished operation: aborting ours
/// would take the other one with it.
pub(crate) fn ensure_idle(repo: &Repository, what: &'static str) -> Result<()> {
    if let Some(other) = in_progress(repo) {
        return Err(refused(what, say!("rewrite-other-in-progress", other)));
    }
    if !conflicted(repo).is_empty() {
        return Err(refused(what, say!("rewrite-unresolved-conflicts")));
    }
    Ok(())
}

/// Run a sequencer command. When it stops half-way it is left in progress
/// and what git said is returned, or, where there is no state to leave, it
/// is taken back and the error names the files in conflict. A command that
/// refused to start left nothing behind and is an error as it is.
fn sequenced(
    repo: &Repository,
    args: &[&str],
    on_stop: OnStop,
    what: &'static str,
) -> Result<Option<String>> {
    ensure_idle(repo, what)?;
    let staged = |repo: &Repository| exe::run(repo, ["diff", "--cached", "--quiet"], None).is_err();
    let staged_before = staged(repo);
    if on_stop == OnStop::ResetIndex && staged_before {
        // Resetting the index is how these are taken back, and that would
        // take staged work with it.
        return Err(refused(what, say!("rewrite-staged-changes")));
    }
    let Err(err) = exe::run_hooked(repo, args, None) else {
        return Ok(None);
    };
    match stopped_or_failed(repo, on_stop, what, err) {
        // The commit of the operation could not be made (it could not be
        // signed, or a hook refused it) and git left nothing to continue
        // or to abort, only what the operation had staged: a revert does
        // that. Nothing was staged before, so what is staged now is the
        // operation's and is taken back: a refusal changes nothing.
        Err(err @ (Error::Refused { .. } | Error::Hook { after: false, .. }))
            if on_stop == OnStop::Leave && !staged_before && staged(repo) =>
        {
            let _ = exe::run(repo, ["reset", "--merge"], None);
            Err(refused(what, say!("rewrite-taken-back", told = told(&err))))
        }
        other => other,
    }
}

/// What an error says, without the operation it names in front: the
/// caller names its own.
fn told(err: &Error) -> String {
    match err {
        Error::Refused { detail, .. } => detail.clone(),
        other => other.to_string(),
    }
}

/// After a failed command that was started from an idle repository: either
/// it is in progress now (and that is the answer), or it failed. A
/// signature that could not be made is explained either way, and a hook
/// that refused is told apart from git (the command has to have been run
/// with [`exe::run_hooked`] or as a stream for that).
pub(crate) fn stopped_or_failed(
    repo: &Repository,
    on_stop: OnStop,
    what: &'static str,
    err: Error,
) -> Result<Option<String>> {
    let err = crate::signature::explain_failure(repo, err, what);
    match on_stop {
        OnStop::Leave => {
            if crate::sequence::in_progress(repo)?.is_some() {
                Ok(Some(crate::sequence::why_stopped(what, &err)))
            } else {
                Err(err)
            }
        }
        OnStop::ResetIndex => {
            let files = conflicted(repo);
            if files.is_empty() {
                return Err(err);
            }
            let _ = exe::run(repo, ["reset", "--merge"], None);
            let _ = std::fs::remove_file(repo.git_dir().join("SQUASH_MSG"));
            Err(refused(
                what,
                say!("rewrite-would-conflict", files = files.join(", ")),
            ))
        }
    }
}

pub(crate) fn is_tracked_change(entry: &StatusEntry) -> bool {
    (entry.staged.is_some() || entry.unstaged.is_some())
        && entry.unstaged != Some(ChangeKind::Untracked)
}

/// Paths that differ between two commits, or only the ones `to` adds. With
/// no `from` (an unborn branch) that is everything in `to`.
fn paths_between(
    repo: &Repository,
    from: Option<Oid>,
    to: Oid,
    added_only: bool,
) -> Result<Vec<String>> {
    let to = to.to_hex();
    let out = match from {
        Some(from) => {
            let from = from.to_hex();
            let mut args = vec!["diff", "--name-only", "--no-renames", "-z"];
            if added_only {
                args.push("--diff-filter=A");
            }
            args.extend([from.as_str(), to.as_str(), "--"]);
            exe::run(repo, args, None)?
        }
        None => exe::run(
            repo,
            ["ls-tree", "-r", "--name-only", "-z", to.as_str()],
            None,
        )?,
    };
    Ok(out
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Files on disk that are not git's and sit where moving from `from` to
/// `to` writes a file. A hard reset overwrites them without a word.
pub(crate) fn untracked_in_the_way(
    repo: &Repository,
    status: &[StatusEntry],
    from: Option<Oid>,
    to: Oid,
) -> Result<Vec<String>> {
    let known: HashSet<&str> = status
        .iter()
        .filter(|entry| is_tracked_change(entry))
        .map(|entry| entry.path.as_str())
        .collect();
    let mut seen = HashSet::new();
    let mut found = Vec::new();
    for path in paths_between(repo, from, to, true)? {
        // The file itself, and a file standing where one of its
        // directories has to be.
        let mut candidate = Some(path.as_str());
        while let Some(current) = candidate {
            candidate = current.rsplit_once('/').map(|(parent, _)| parent);
            if known.contains(current) || !seen.insert(current.to_owned()) {
                continue;
            }
            let Ok(file) = stage::worktree_path(repo, current) else {
                continue;
            };
            let Ok(meta) = file.symlink_metadata() else {
                continue;
            };
            if !meta.is_dir() {
                found.push(current.to_owned());
            } else if current == path {
                let inside = format!("{current}/");
                if status.iter().any(|entry| {
                    entry.unstaged == Some(ChangeKind::Untracked) && entry.path.starts_with(&inside)
                }) {
                    return Err(refused(
                        say!("rewrite-what-reset"),
                        say!("rewrite-untracked-in-the-way", directory = current),
                    ));
                }
            }
        }
    }
    Ok(found)
}

pub(crate) fn moved(
    repo: &Repository,
    before: HeadState,
    backups: Vec<Backup>,
    stopped: Option<String>,
) -> Result<Moved> {
    Ok(Moved {
        before,
        after: branch::head_state(repo)?,
        backups,
        stopped,
        files: None,
    })
}

/// Merge `target` into the current branch.
pub fn merge(repo: &Repository, target: &str, mode: MergeMode) -> Result<Moved> {
    merge_with(repo, target, mode, false)
}

/// [`merge`], and with `no_verify` without the two hooks that can refuse
/// the merge commit: `pre-merge-commit` and `commit-msg`
/// (`git merge --no-verify`).
pub fn merge_with(
    repo: &Repository,
    target: &str,
    mode: MergeMode,
    no_verify: bool,
) -> Result<Moved> {
    let merged = commit_of(repo, say!("rewrite-what-merge-target"), target)?;
    let before = branch::head_state(repo)?;
    // The name as given, so git words the merge message the way it would
    // in a shell.
    let how: &[&str] = match mode {
        MergeMode::FfOnly => &["--ff-only", "--no-edit"],
        MergeMode::Ff => &["--ff", "--no-edit"],
        MergeMode::NoFf => &["--no-ff", "--no-edit"],
        MergeMode::Squash => &["--squash", "--no-edit"],
        // Never a fast-forward: that would be done with nothing to look
        // at before it is.
        MergeMode::NoCommit => &["--no-ff", "--no-commit"],
    };
    let mut args = vec!["merge"];
    args.extend_from_slice(how);
    if no_verify {
        args.push("--no-verify");
    }
    args.push(target);
    let args = args.as_slice();
    let on_stop = if mode == MergeMode::Squash {
        OnStop::ResetIndex
    } else {
        OnStop::Leave
    };
    let run = || sequenced(repo, args, on_stop, say!("rewrite-what-merge"));
    // A squash leaves HEAD where it is: what it did to the files is what
    // undo takes back. It can write what differs between the two sides.
    let squashed = (mode == MergeMode::Squash)
        .then(|| crate::files_change::paths_between(repo, "HEAD", &merged.to_hex()).ok())
        .flatten();
    let (mut stopped, files) = match squashed {
        Some(paths) => crate::files_change::recorded(repo, paths, run)?,
        None => (run()?, None),
    };
    // Merged without a conflict and, as asked, not committed: git left
    // the merge in progress. Nothing is in progress when there was
    // nothing to merge.
    if mode == MergeMode::NoCommit
        && stopped.is_none()
        && crate::sequence::in_progress(repo)?.is_some()
    {
        stopped = Some(stopped_before_commit().to_owned());
    }
    let mut moved = moved(repo, before, Vec::new(), stopped)?;
    if moved.stopped.is_none() {
        moved.files = files;
    }
    Ok(moved)
}

/// Rebase the current branch onto `onto`.
pub fn rebase(repo: &Repository, onto: &str) -> Result<Moved> {
    Ok(rebase_with(repo, onto, &RebaseOptions::default())?.moved)
}

/// Rebase the current branch onto `onto`: all of it that `onto` does not
/// have, or only the commits after `options.after`; with local changes set
/// aside and brought back when `options.autostash` says so.
pub fn rebase_with(repo: &Repository, onto: &str, options: &RebaseOptions) -> Result<Rebased> {
    commit_of(repo, say!("rewrite-what-rebase-target"), onto)?;
    let before = branch::head_state(repo)?;
    if before.branch.is_none() {
        return Err(refused(
            say!("rewrite-what-rebase"),
            say!("rewrite-head-not-on-branch"),
        ));
    }
    let mut args = vec!["rebase"];
    // Not asked for, it is as the user's `rebase.autoStash` has it.
    if options.autostash {
        args.push("--autostash");
    }
    if options.no_verify {
        args.push("--no-verify");
    }
    let after;
    match &options.after {
        Some(rev) => {
            let start = commit_of(repo, say!("rewrite-what-range-start"), rev)?;
            // What is not in the branch's history has no commits after it
            // on the branch: git would take everything since the fork.
            let (code, _) = exe::run_with_code(
                repo,
                [
                    "merge-base",
                    "--is-ancestor",
                    start.to_hex().as_str(),
                    "HEAD",
                ],
                None,
            )?;
            if code != Some(0) {
                return Err(refused(
                    say!("rewrite-what-rebase"),
                    say!("rewrite-not-in-current-history", rev = rev.as_str()),
                ));
            }
            after = start.to_hex();
            args.extend(["--onto", onto, after.as_str()]);
        }
        None => args.push(onto),
    }
    ensure_idle(repo, say!("rewrite-what-rebase"))?;
    // Changes that were set aside and do not fit back in are put in the
    // stash by git, in words that differ from version to version: the
    // stash having a new entry when the rebase is over says it.
    let stash_before = branch::resolve(repo, "refs/stash");
    let stopped = match exe::run_hooked(repo, &args, None) {
        Ok(_) => {
            let stash_after = branch::resolve(repo, "refs/stash");
            return Ok(Rebased {
                moved: moved(repo, before, Vec::new(), None)?,
                stash_kept: stash_after.is_some() && stash_after != stash_before,
            });
        }
        Err(err) => stopped_or_failed(repo, OnStop::Leave, say!("rewrite-what-rebase"), err)?,
    };
    Ok(Rebased {
        moved: moved(repo, before, Vec::new(), stopped)?,
        stash_kept: false,
    })
}

/// What merging two tips in memory says.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Prediction {
    /// Whether it could be told at all: it takes a git that merges without
    /// a worktree (2.38) and two histories with something in common, and,
    /// for a merge nobody asked for (`exe::local_only`) in a partial
    /// clone, the trees and files it reads being here.
    pub known: bool,
    /// The files the merge would leave in conflict.
    pub conflicts: Vec<String>,
}

/// Most files a prediction names; a merge that conflicts in more than this
/// is not made clearer by the rest.
const MAX_PREDICTED: usize = 500;

/// The files that merging `target` into the current commit would leave in
/// conflict. The merge is done in memory: neither the index nor the files
/// are touched, and uncommitted changes play no part. For a rebase onto
/// `target` this is an estimate, since a rebase replays commits one at a
/// time and can conflict where the two tips would not.
pub fn predict(repo: &Repository, target: &str) -> Result<Prediction> {
    predict_with(repo, "HEAD", target, false)
}

/// Counts the scratch directories of [`predict_aside`] apart.
static ASIDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// [`predict`] for looking ahead, when nobody asked: the objects the
/// merge makes (its tree, the files it merged) are written to a directory
/// of their own outside the repository, which is removed afterwards, so
/// that nothing changes under `.git` and nothing that watches it is
/// woken.
pub fn predict_aside(repo: &Repository, target: &str) -> Result<Prediction> {
    predict_with(repo, "HEAD", target, true)
}

/// [`predict`] between two tips of which neither need be checked out:
/// the files merging `target` into `ours` would leave in conflict, as a
/// pull request of the one into the other would meet them. With `aside`,
/// it writes nothing into the repository, as [`predict_aside`].
pub fn predict_between(
    repo: &Repository,
    ours: &str,
    target: &str,
    aside: bool,
) -> Result<Prediction> {
    predict_with(repo, ours, target, aside)
}

fn predict_with(repo: &Repository, ours: &str, target: &str, aside: bool) -> Result<Prediction> {
    let theirs = commit_of(repo, say!("rewrite-what-target"), target)?;
    let what = if ours == "HEAD" {
        say!("rewrite-what-current-commit")
    } else {
        say!("rewrite-what-merged-into")
    };
    let ours = commit_of(repo, what, ours)?;
    let (ours, theirs) = (ours.to_hex(), theirs.to_hex());
    let args = [
        "merge-tree",
        "--write-tree",
        "--name-only",
        "--no-messages",
        "-z",
        ours.as_str(),
        theirs.as_str(),
    ];
    let (code, out) = if aside {
        // Git writes new objects where it is told to and reads the
        // repository's own as alternates of that place.
        let objects = exe::run(
            repo,
            [
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "objects",
            ],
            None,
        )?;
        let objects = std::path::PathBuf::from(objects.stdout.trim_end_matches('\n'));
        let scratch = std::env::temp_dir().join(format!(
            "bitbraid-merge-{}-{}",
            std::process::id(),
            ASIDE.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&scratch).map_err(|err| Error::Git {
            context: say!("rewrite-making-trial-directory"),
            source: Box::new(err),
        })?;
        let answer = exe::run_with_code_env(
            repo,
            args,
            &[
                ("GIT_OBJECT_DIRECTORY", scratch.as_os_str()),
                ("GIT_ALTERNATE_OBJECT_DIRECTORIES", objects.as_os_str()),
            ],
        );
        let _ = std::fs::remove_dir_all(&scratch);
        answer?
    } else {
        exe::run_with_code(repo, args, None)?
    };
    Ok(match code {
        Some(0) => Prediction {
            known: true,
            conflicts: Vec::new(),
        },
        // The merged tree's id comes first, then the files in conflict.
        Some(1) => {
            let mut conflicts: Vec<String> = out
                .stdout
                .split('\0')
                .skip(1)
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect();
            conflicts.sort();
            conflicts.dedup();
            conflicts.truncate(MAX_PREDICTED);
            Prediction {
                known: true,
                conflicts,
            }
        }
        // An older git, unrelated histories, or anything else git would
        // rather not merge: there is no telling.
        _ => Prediction {
            known: false,
            conflicts: Vec::new(),
        },
    })
}

/// Move the current branch to `target`. A hard reset backs up the files it
/// is about to overwrite: changed ones, and untracked ones in the way.
pub fn reset(repo: &Repository, target: &str, mode: ResetMode) -> Result<Moved> {
    let commit = commit_of(repo, say!("rewrite-what-reset-target"), target)?;
    ensure_idle(repo, say!("rewrite-what-reset"))?;
    let before = branch::head_state(repo)?;
    let mut backups = Vec::new();
    if mode == ResetMode::Hard {
        let status = crate::status(repo)?;
        let mut paths: Vec<String> = status
            .iter()
            .filter(|entry| is_tracked_change(entry))
            .map(|entry| entry.path.clone())
            .collect();
        paths.extend(untracked_in_the_way(repo, &status, before.commit, commit)?);
        backups = stage::backup_paths(repo, &paths)?;
    }
    exe::run(
        repo,
        ["reset", "--quiet", mode.flag(), commit.to_hex().as_str()],
        None,
    )?;
    moved(repo, before, backups, None)
}

fn commits_of(repo: &Repository, what: &'static str, revs: &[String]) -> Result<Vec<String>> {
    if revs.is_empty() {
        return Err(invalid(what, say!("rewrite-no-commits-given")));
    }
    revs.iter()
        .map(|rev| commit_of(repo, what, rev).map(|id| id.to_hex()))
        .collect()
}

/// Apply the changes of `commits`, in the order given, as new commits on
/// the current branch (or as staged work with `no_commit`). A merge commit
/// needs `mainline`: which of its parents to compare with (1 is the first).
pub fn cherry_pick(
    repo: &Repository,
    commits: &[String],
    mainline: Option<u32>,
    no_commit: bool,
) -> Result<Moved> {
    let ids = commits_of(repo, say!("rewrite-what-cherry-pick"), commits)?;
    let before = branch::head_state(repo)?;
    let mainline = mainline.map(|parent| parent.to_string());
    let mut args = vec!["cherry-pick"];
    if no_commit {
        args.push("--no-commit");
    }
    if let Some(parent) = &mainline {
        args.extend(["--mainline", parent.as_str()]);
    }
    args.extend(ids.iter().map(String::as_str));
    let on_stop = if no_commit {
        OnStop::ResetIndex
    } else {
        OnStop::Leave
    };
    let run = || sequenced(repo, &args, on_stop, say!("rewrite-what-cherry-pick"));
    // Without a commit HEAD stays: what it did to the files is what undo
    // takes back.
    let (stopped, files) = match no_commit
        .then(|| picked_paths(repo, &ids, mainline.as_deref()))
        .flatten()
    {
        Some(paths) => crate::files_change::recorded(repo, paths, run)?,
        None => (run()?, None),
    };
    let mut moved = moved(repo, before, Vec::new(), stopped)?;
    if moved.stopped.is_none() {
        moved.files = files;
    }
    Ok(moved)
}

/// The paths picking `ids` can write: what each changed against its
/// parent (the `mainline` one of a merge), every file of a root commit.
/// Nothing when that cannot be told.
fn picked_paths(repo: &Repository, ids: &[String], mainline: Option<&str>) -> Option<Vec<String>> {
    let mut paths = Vec::new();
    for id in ids {
        let parent = format!("{id}^{}", mainline.unwrap_or("1"));
        let changed = crate::files_change::paths_between(repo, &parent, id).or_else(|_| {
            exe::run(
                repo,
                ["ls-tree", "-r", "--name-only", "-z", id.as_str()],
                None,
            )
            .map(|out| {
                out.stdout
                    .split('\0')
                    .filter(|path| !path.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
        });
        paths.extend(changed.ok()?);
    }
    Some(paths)
}

/// Undo the changes of `commits` with new commits. A merge commit needs
/// `mainline`: which of its parents is the line to keep (1 is the first).
pub fn revert(repo: &Repository, commits: &[String], mainline: Option<u32>) -> Result<Moved> {
    let ids = commits_of(repo, say!("rewrite-what-revert"), commits)?;
    let before = branch::head_state(repo)?;
    let mainline = mainline.map(|parent| parent.to_string());
    let mut args = vec!["revert", "--no-edit"];
    if let Some(parent) = &mainline {
        args.extend(["--mainline", parent.as_str()]);
    }
    args.extend(ids.iter().map(String::as_str));
    let stopped = sequenced(repo, &args, OnStop::Leave, say!("rewrite-what-revert"))?;
    moved(repo, before, Vec::new(), stopped)
}

/// Put HEAD at `to` if it is still at `expected`. Used by undo and redo of
/// everything in this module, so it never costs work: where `mode` moved
/// the files, they move back only when no local change and no untracked
/// file is in the way.
pub fn move_back(
    repo: &Repository,
    expected: &HeadState,
    to: &HeadState,
    mode: ResetMode,
) -> Result<()> {
    ensure_idle(repo, say!("rewrite-what-undo"))?;
    if branch::head_state(repo)? != *expected {
        return Err(refused(
            say!("rewrite-what-undo"),
            say!("rewrite-head-moved"),
        ));
    }
    let commit = to.commit.ok_or_else(|| {
        refused(
            say!("rewrite-what-undo"),
            say!("rewrite-nothing-to-go-back-to"),
        )
    })?;
    let flag = match mode {
        ResetMode::Soft | ResetMode::Mixed => mode.flag(),
        ResetMode::Hard | ResetMode::Keep => {
            let status = crate::status(repo)?;
            let touched: HashSet<String> = paths_between(repo, expected.commit, commit, false)?
                .into_iter()
                .collect();
            let mut in_the_way: Vec<String> = status
                .iter()
                .filter(|entry| is_tracked_change(entry) && touched.contains(&entry.path))
                .map(|entry| entry.path.clone())
                .collect();
            in_the_way.extend(untracked_in_the_way(
                repo,
                &status,
                expected.commit,
                commit,
            )?);
            if !in_the_way.is_empty() {
                return Err(refused(
                    say!("rewrite-what-undo"),
                    say!("rewrite-changes-in-the-way", paths = in_the_way.join(", ")),
                ));
            }
            // Git checks once more, and refuses rather than overwrites.
            ResetMode::Keep.flag()
        }
    };
    exe::run(
        repo,
        ["reset", "--quiet", flag, commit.to_hex().as_str()],
        None,
    )?;
    Ok(())
}
