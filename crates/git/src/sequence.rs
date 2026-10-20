//! Operations that are in the middle: a merge, rebase, cherry-pick or
//! revert that stopped, or a mailbox of patches (`git am`) that did; what
//! it stopped on, and how to go on.
//!
//! The state is read from the files git keeps in its directory, so an
//! operation started in a shell looks the same as one started here.
//!
//! Sources: git-merge(1), git-rebase(1), git-cherry-pick(1),
//! git-revert(1), git-am(1), git-ls-files(1) (`-u`), git-checkout(1)
//! (`--ours`, `--theirs`, `--merge`), gitrepository-layout(5).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, exe};

/// Which operation is unfinished.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A merge waiting for its commit.
    Merge,
    /// A rebase between two of its commits.
    Rebase,
    /// A cherry-pick, possibly of several commits.
    CherryPick,
    /// A revert, possibly of several commits.
    Revert,
    /// A mailbox of patches being applied as commits (`git am`), stopped
    /// at a patch that conflicts or does not apply.
    Mailbox,
}

impl Kind {
    /// The word for it in a sentence.
    pub fn noun(self) -> &'static str {
        match self {
            Kind::Merge => say!("sequence-kind-merge"),
            Kind::Rebase => say!("sequence-kind-rebase"),
            Kind::CherryPick => say!("sequence-kind-cherry-pick"),
            Kind::Revert => say!("sequence-kind-revert"),
            Kind::Mailbox => say!("sequence-kind-mailbox"),
        }
    }
}

/// What kind of conflict a path is in, from the versions the index holds.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConflictKind {
    /// Changed on both sides.
    BothModified,
    /// Added on both sides, differently.
    BothAdded,
    /// Changed on our side, deleted on theirs.
    DeletedByThem,
    /// Deleted on our side, changed on theirs.
    DeletedByUs,
    /// Only our side has it (a rename met something else).
    AddedByUs,
    /// Only their side has it.
    AddedByThem,
    /// Deleted on both sides (two renames of one file).
    BothDeleted,
}

/// One conflicted path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Conflict {
    /// Path relative to the worktree.
    pub path: String,
    pub kind: ConflictKind,
    /// The version both sides started from, when there is one.
    pub base: Option<Oid>,
    /// Our version, when our side has the file.
    pub ours: Option<Oid>,
    /// Their version, when their side has the file.
    pub theirs: Option<Oid>,
    /// The path is a submodule: the versions are commits of the submodule,
    /// and it is resolved by choosing which of them it points at.
    #[serde(default)]
    pub submodule: bool,
    /// Rerere has put a resolution it had recorded into the file. The
    /// path is still unmerged: the resolution waits to be accepted.
    #[serde(default)]
    pub reused: bool,
}

/// An unfinished operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InProgress {
    pub kind: Kind,
    /// The commit being merged, replayed, picked or reverted right now. A
    /// patch from a mailbox is no commit yet.
    pub current: Option<Oid>,
    /// For a rebase or a mailbox: the step it is on and how many there
    /// are.
    pub step: Option<(u32, u32)>,
    /// For a pick or revert of several commits: how many are still to do,
    /// the current one included.
    pub remaining: Option<u32>,
    /// For a rebase: the branch being rebased, by its short name.
    pub branch: Option<String>,
    /// For a rebase: the commit it is rebased onto.
    pub onto: Option<Oid>,
    /// Where HEAD was when the operation began.
    pub original_head: Option<Oid>,
    /// What "ours" means in this operation, in words.
    pub ours: String,
    /// What "theirs" means.
    pub theirs: String,
    /// Paths still in conflict.
    pub conflicts: Vec<Conflict>,
    /// Whether a step can be skipped (not a merge).
    pub can_skip: bool,
    /// An interactive rebase stopped at a commit for it to be amended:
    /// nothing is wrong, and it goes on when asked to.
    #[serde(default)]
    pub editing: bool,
    /// Continuing would go through as things are: nothing is in conflict,
    /// and what the step commits is there. A merge and a rebase that
    /// replays commits go on with whatever is staged, even nothing. A
    /// series of patches, a rebase that applies patches, a pick and a
    /// revert make a commit of what is staged and refuse when that is
    /// nothing: they are ready once something is.
    #[serde(default)]
    pub ready: bool,
}

/// Whether the index holds anything HEAD does not: what a step that makes
/// a commit of the staged changes would commit. Nothing that cannot be
/// read counts as nothing staged.
fn something_staged(repo: &Repository) -> bool {
    matches!(
        exe::run_with_code(repo, ["diff", "--cached", "--quiet"], None),
        Ok((Some(1), _))
    )
}

/// What continuing or skipping led to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Outcome {
    /// The operation is over.
    pub finished: bool,
    /// When it stopped again: what git said.
    pub message: String,
}

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

/// A small state file, trimmed. Nothing that is missing, large or not text
/// is an error: the state is not known.
fn read(dir: &Path, name: &str) -> Option<String> {
    let file = dir.join(name);
    let meta = file.metadata().ok()?;
    if !meta.is_file() || meta.len() > 1024 * 1024 {
        return None;
    }
    let text = std::fs::read_to_string(file).ok()?;
    Some(text.trim().to_owned())
}

fn read_oid(dir: &Path, name: &str) -> Option<Oid> {
    read(dir, name)?.lines().next()?.trim().parse().ok()
}

fn read_number(dir: &Path, name: &str) -> Option<u32> {
    read(dir, name)?.parse().ok()
}

fn short(id: Option<Oid>) -> String {
    id.map(|id| id.short(7))
        .unwrap_or_else(|| say!("sequence-unknown-commit").to_owned())
}

/// A side of a stopped operation by name: the branch it is on and its
/// short id ("main at 1df6771"), or the short id alone when no branch is
/// at it.
fn named(id: Option<Oid>, branch: Option<String>) -> String {
    match (branch.filter(|name| !name.is_empty()), id) {
        (Some(branch), Some(id)) => say!("sequence-branch-at", branch, commit = id.short(7)),
        (Some(branch), None) => branch,
        (None, id) => short(id),
    }
}

/// The commit HEAD is at, and the branch checked out, if one is.
fn head_of(repo: &Repository) -> (Option<Oid>, Option<String>) {
    let id = exe::run(repo, ["rev-parse", "--verify", "--quiet", "HEAD"], None)
        .ok()
        .and_then(|out| out.stdout.trim().parse().ok());
    let branch = exe::run(repo, ["symbolic-ref", "--quiet", "--short", "HEAD"], None)
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|name| !name.is_empty());
    (id, branch)
}

/// A local branch whose tip is this commit, the first by name.
fn branch_at(repo: &Repository, id: Option<Oid>) -> Option<String> {
    let id = id?.to_hex();
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            "--points-at",
            id.as_str(),
            "--format=%(refname:short)",
            "refs/heads",
        ],
        None,
    )
    .ok()?;
    out.stdout
        .lines()
        .map(str::trim)
        .find(|name| !name.is_empty())
        .map(str::to_owned)
}

/// The paths in conflict, in the order the index has them.
pub fn conflicts(repo: &Repository) -> Result<Vec<Conflict>> {
    let out = exe::run(repo, ["ls-files", "--unmerged", "-z"], None)?;
    let mut found: Vec<Conflict> = Vec::new();
    for entry in out.stdout.split('\0').filter(|entry| !entry.is_empty()) {
        // `<mode> <object> <stage>\t<path>`
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut parts = meta.split(' ');
        let (Some(mode), Some(id), Some(stage)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let id: Option<Oid> = id.parse().ok();
        if found.last().is_none_or(|last| last.path != path) {
            found.push(Conflict {
                path: path.to_owned(),
                kind: ConflictKind::BothModified,
                base: None,
                ours: None,
                theirs: None,
                submodule: false,
                reused: false,
            });
        }
        if let Some(conflict) = found.last_mut() {
            conflict.submodule |= mode == "160000";
            match stage {
                "1" => conflict.base = id,
                "2" => conflict.ours = id,
                "3" => conflict.theirs = id,
                _ => {}
            }
        }
    }
    for conflict in &mut found {
        conflict.kind = match (
            conflict.base.is_some(),
            conflict.ours.is_some(),
            conflict.theirs.is_some(),
        ) {
            (_, true, true) if conflict.base.is_some() => ConflictKind::BothModified,
            (_, true, true) => ConflictKind::BothAdded,
            (true, true, false) => ConflictKind::DeletedByThem,
            (true, false, true) => ConflictKind::DeletedByUs,
            (false, true, false) => ConflictKind::AddedByUs,
            (false, false, true) => ConflictKind::AddedByThem,
            (_, false, false) => ConflictKind::BothDeleted,
        };
    }
    // What rerere resolved from its record is still unmerged, and is told
    // from the rest so that it can be accepted as it is.
    let paths: Vec<&str> = found
        .iter()
        .map(|conflict| conflict.path.as_str())
        .collect();
    let reused = crate::rerere::resolved_among(repo, &paths);
    for conflict in &mut found {
        conflict.reused = reused.contains(&conflict.path);
    }
    Ok(found)
}

/// The operation that is unfinished in this worktree, if any.
pub fn in_progress(repo: &Repository) -> Result<Option<InProgress>> {
    let dir = repo.git_dir();
    let exists = |name: &str| dir.join(name).symlink_metadata().is_ok();
    let branch_of = |sub: &Path| {
        read(sub, "head-name").map(|name| {
            name.strip_prefix("refs/heads/")
                .map(str::to_owned)
                .unwrap_or(name)
        })
    };
    // A rebase that replays commits goes on with nothing staged (the
    // commit is left out); one that applies patches does not.
    let replays = exists("rebase-merge");
    // A rebase first: the commit it is replaying leaves a CHERRY_PICK_HEAD
    // of its own.
    let found = if exists("rebase-merge") {
        let sub = dir.join("rebase-merge");
        let current = read_oid(&sub, "stopped-sha").or_else(|| read_oid(dir, "REBASE_HEAD"));
        let (onto, branch) = (read_oid(&sub, "onto"), branch_of(&sub));
        Some(InProgress {
            kind: Kind::Rebase,
            current,
            step: read_number(&sub, "msgnum").zip(read_number(&sub, "end")),
            remaining: None,
            branch: branch.clone(),
            onto,
            original_head: read_oid(&sub, "orig-head"),
            ours: named(onto, branch_at(repo, onto)),
            theirs: named(current, branch),
            conflicts: Vec::new(),
            can_skip: true,
            editing: sub.join("amend").symlink_metadata().is_ok(),
            ready: false,
        })
    } else if exists("rebase-apply/applying") {
        // `git am` keeps its state where a rebase that applies patches
        // does, and marks it as its own. It works on the branch as it is:
        // where that was before the first patch is in ORIG_HEAD.
        let sub = dir.join("rebase-apply");
        let subject = read(&sub, "final-commit")
            .and_then(|message| message.lines().next().map(str::to_owned))
            .map(|subject| subject.chars().take(72).collect::<String>())
            .filter(|subject| !subject.trim().is_empty());
        Some(InProgress {
            kind: Kind::Mailbox,
            current: None,
            step: read_number(&sub, "next").zip(read_number(&sub, "last")),
            remaining: None,
            branch: None,
            onto: None,
            original_head: read_oid(dir, "ORIG_HEAD"),
            ours: say!("sequence-current-branch").to_owned(),
            theirs: match subject {
                Some(subject) => say!("sequence-the-patch", subject),
                None => say!("sequence-patch-being-applied").to_owned(),
            },
            conflicts: Vec::new(),
            can_skip: true,
            editing: false,
            ready: false,
        })
    } else if exists("rebase-apply") {
        let sub = dir.join("rebase-apply");
        let current = read_oid(dir, "REBASE_HEAD").or_else(|| read_oid(&sub, "original-commit"));
        let (onto, branch) = (read_oid(&sub, "onto"), branch_of(&sub));
        Some(InProgress {
            kind: Kind::Rebase,
            current,
            step: read_number(&sub, "next").zip(read_number(&sub, "last")),
            remaining: None,
            branch: branch.clone(),
            onto,
            original_head: read_oid(&sub, "orig-head"),
            ours: named(onto, branch_at(repo, onto)),
            theirs: named(current, branch),
            conflicts: Vec::new(),
            can_skip: true,
            editing: false,
            ready: false,
        })
    } else if exists("MERGE_HEAD") {
        let current = read_oid(dir, "MERGE_HEAD");
        let (head, branch) = head_of(repo);
        Some(InProgress {
            kind: Kind::Merge,
            current,
            step: None,
            remaining: None,
            branch: None,
            onto: None,
            original_head: None,
            ours: named(head, branch),
            theirs: named(current, branch_at(repo, current)),
            conflicts: Vec::new(),
            can_skip: false,
            editing: false,
            ready: false,
        })
    } else if exists("CHERRY_PICK_HEAD") || exists("REVERT_HEAD") {
        let pick = exists("CHERRY_PICK_HEAD");
        let current = read_oid(
            dir,
            if pick {
                "CHERRY_PICK_HEAD"
            } else {
                "REVERT_HEAD"
            },
        );
        let sequencer = dir.join("sequencer");
        let remaining = read(&sequencer, "todo").map(|todo| {
            let lines = todo
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
                .count();
            u32::try_from(lines).unwrap_or(u32::MAX)
        });
        Some(InProgress {
            kind: if pick { Kind::CherryPick } else { Kind::Revert },
            current,
            step: None,
            remaining,
            branch: None,
            onto: None,
            original_head: read_oid(&sequencer, "head"),
            ours: {
                let (head, branch) = head_of(repo);
                named(head, branch)
            },
            theirs: if pick {
                named(current, branch_at(repo, current))
            } else {
                say!("sequence-undone", commit = short(current))
            },
            conflicts: Vec::new(),
            can_skip: true,
            editing: false,
            ready: false,
        })
    } else {
        None
    };
    match found {
        Some(mut operation) => {
            operation.conflicts = conflicts(repo)?;
            // A conflict in the commit that was being amended is a
            // conflict first.
            operation.editing = operation.editing && operation.conflicts.is_empty();
            operation.ready = operation.conflicts.is_empty()
                && (operation.editing
                    || operation.kind == Kind::Merge
                    || (operation.kind == Kind::Rebase && replays)
                    || something_staged(repo));
            Ok(Some(operation))
        }
        None => Ok(None),
    }
}

fn required(repo: &Repository, what: &'static str) -> Result<InProgress> {
    in_progress(repo)?.ok_or_else(|| refused(what, say!("sequence-nothing-in-progress")))
}

/// What git said, cut to what a person reads: its last lines, without the
/// hints.
pub(crate) fn last_words(err: &Error) -> String {
    let text = match err {
        Error::Command { stderr, .. } => stderr.clone(),
        other => other.to_string(),
    };
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .collect();
    let from = lines.len().saturating_sub(4);
    lines.get(from..).unwrap_or(&[]).join(" ")
}

/// Why an operation stopped with nothing in conflict, said so that it can
/// be acted on. A signature that could not be made (explained by
/// [`crate::signature::explain_failure`]) and a hook that refused
/// ([`Error::Hook`]) say what happened and that the operation is still
/// there to continue or abort; anything else is the end of what git said.
pub(crate) fn why_stopped(what: &str, err: &Error) -> String {
    match err {
        Error::Refused { detail, .. } => {
            say!("sequence-commit-not-made", what, detail = detail.as_str())
        }
        Error::Hook { after: false, .. } => {
            say!("sequence-hook-refused", err = err.to_string(), what)
        }
        other => last_words(other),
    }
}

/// Run `git am`. Where it stopped and why it says on standard output,
/// which the error of a failed command does not carry: that is put after
/// what it wrote to standard error, without its progress. With a watch on
/// the hooks, as every command that makes commits has: a patch that
/// `applypatch-msg` or `pre-applypatch` refused, or a commit of it that
/// `pre-commit`'s kin refused, is told apart from git.
pub(crate) fn run_mailbox<I, S>(repo: &Repository, args: I) -> Result<exe::Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    exe::run_hooked_with_code(repo, args, None, |code, out| {
        if code == Some(0) {
            return Ok(out);
        }
        let told: Vec<&str> = out
            .stdout
            .lines()
            .filter(|line| !line.starts_with("Applying: "))
            .collect();
        Err(Error::Command {
            command: "git am".to_owned(),
            status: match code {
                Some(code) => say!("sequence-exit-status", code),
                None => say!("sequence-ended-by-signal").to_owned(),
            },
            stderr: format!("{}\n{}", out.stderr.trim(), told.join("\n")),
        })
    })
}

/// Run one step of an operation of this kind. A step makes commits: a
/// signature that could not be made is explained, and a hook that
/// refused is told apart from git.
fn run_step(repo: &Repository, kind: Kind, args: &[&str]) -> Result<exe::Output> {
    match kind {
        Kind::Mailbox => run_mailbox(repo, args),
        _ => exe::run_hooked(repo, args, None)
            .map_err(|err| crate::signature::explain_failure(repo, err, kind.noun())),
    }
}

/// After a step: over, stopped again on something to resolve, or failed.
/// `before` is the operation as it was when the step began.
fn after(repo: &Repository, before: &InProgress, result: Result<exe::Output>) -> Result<Outcome> {
    match result {
        Ok(_) => {
            let finished = in_progress(repo)?.is_none();
            if finished && before.kind == Kind::Rebase {
                // The messages a plan kept for as long as it lasted.
                let _ = std::fs::remove_dir_all(crate::interactive::plan_dir(repo));
            }
            Ok(Outcome {
                finished,
                message: String::new(),
            })
        }
        Err(err) => match in_progress(repo)? {
            // Stopped on the next thing to resolve: that is not a failure.
            Some(operation) if !operation.conflicts.is_empty() => Ok(Outcome {
                finished: false,
                message: last_words(&err),
            }),
            // A mailbox that went on can stop at a later patch with
            // nothing in conflict: one that does not apply at all.
            Some(operation) if operation.kind == Kind::Mailbox && operation.step != before.step => {
                Ok(Outcome {
                    finished: false,
                    message: last_words(&err),
                })
            }
            // Still there with nothing in conflict: the step itself did
            // not go through (its commit could not be signed, a hook
            // refused it, the tree is not as the step needs it).
            Some(operation) => Err(refused(
                say!("sequence-what-continue"),
                why_stopped(operation.kind.noun(), &err),
            )),
            None => Err(err),
        },
    }
}

/// Go on with the operation after its conflicts are resolved.
pub fn continue_operation(repo: &Repository) -> Result<Outcome> {
    let operation = required(repo, say!("sequence-what-continue"))?;
    if !operation.conflicts.is_empty() {
        let count = operation.conflicts.len();
        return Err(refused(
            say!("sequence-what-continue"),
            say!("sequence-still-in-conflict", count),
        ));
    }
    let args: &[&str] = match operation.kind {
        // The message git prepared, as it is.
        Kind::Merge => &["commit", "--no-edit"],
        Kind::Rebase => &["rebase", "--continue"],
        Kind::CherryPick => &["cherry-pick", "--continue"],
        Kind::Revert => &["revert", "--continue"],
        Kind::Mailbox => &["am", "--continue"],
    };
    after(repo, &operation, run_step(repo, operation.kind, args))
}

/// Leave out the commit the operation stopped on and go on.
pub fn skip_operation(repo: &Repository) -> Result<Outcome> {
    let operation = required(repo, say!("sequence-what-skip"))?;
    let args: &[&str] = match operation.kind {
        Kind::Merge => {
            return Err(refused(
                say!("sequence-what-skip"),
                say!("sequence-merge-nothing-to-skip"),
            ));
        }
        Kind::Rebase => &["rebase", "--skip"],
        Kind::CherryPick => &["cherry-pick", "--skip"],
        Kind::Revert => &["revert", "--skip"],
        Kind::Mailbox => &["am", "--skip"],
    };
    after(repo, &operation, run_step(repo, operation.kind, args))
}

/// Give the operation up and go back to where it began.
pub fn abort_operation(repo: &Repository) -> Result<()> {
    let operation = required(repo, say!("sequence-what-abort"))?;
    let args: &[&str] = match operation.kind {
        Kind::Merge => &["merge", "--abort"],
        Kind::Rebase => &["rebase", "--abort"],
        Kind::CherryPick => &["cherry-pick", "--abort"],
        Kind::Revert => &["revert", "--abort"],
        Kind::Mailbox => &["am", "--abort"],
    };
    exe::run(repo, args, None)?;
    if operation.kind == Kind::Rebase {
        // What a plan kept for as long as it lasted.
        let _ = std::fs::remove_dir_all(crate::interactive::plan_dir(repo));
    }
    Ok(())
}

/// A side of a conflict.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    /// The side the operation is applied to.
    Ours,
    /// The side that is brought in.
    Theirs,
}

fn conflict_at(repo: &Repository, what: &'static str, path: &str) -> Result<Conflict> {
    crate::stage::worktree_path(repo, path)?;
    conflicts(repo)?
        .into_iter()
        .find(|conflict| conflict.path == path)
        .ok_or_else(|| refused(what, say!("sequence-not-in-conflict", path)))
}

/// Resolve a conflicted file by taking one side whole. Where that side
/// deleted the file, the file is removed.
pub fn take_side(repo: &Repository, path: &str, side: Side) -> Result<()> {
    let conflict = conflict_at(repo, say!("sequence-what-resolve"), path)?;
    let has = match side {
        Side::Ours => conflict.ours.is_some(),
        Side::Theirs => conflict.theirs.is_some(),
    };
    if conflict.submodule {
        // What is recorded is the commit the submodule points at. Its own
        // checkout is left as it is, to be brought along by whoever
        // updates submodules.
        let chosen = match side {
            Side::Ours => conflict.ours,
            Side::Theirs => conflict.theirs,
        };
        match chosen {
            Some(id) => {
                let entry = format!("160000,{},{path}", id.to_hex());
                exe::run(repo, ["update-index", "--cacheinfo", entry.as_str()], None)?;
            }
            None => {
                exe::run(repo, ["update-index", "--force-remove", "--", path], None)?;
            }
        }
        return Ok(());
    }
    if has {
        let flag = match side {
            Side::Ours => "--ours",
            Side::Theirs => "--theirs",
        };
        exe::run(
            repo,
            ["--literal-pathspecs", "checkout", flag, "--", path],
            None,
        )?;
        exe::run(repo, ["--literal-pathspecs", "add", "--", path], None)?;
    } else {
        exe::run(
            repo,
            [
                "--literal-pathspecs",
                "rm",
                "--quiet",
                "--force",
                "--",
                path,
            ],
            None,
        )?;
    }
    Ok(())
}

/// Lines git puts around the two sides of a conflict.
pub(crate) fn has_markers(text: &[u8]) -> bool {
    text.split(|byte| *byte == b'\n')
        .any(|line| line.starts_with(b"<<<<<<< ") || line.starts_with(b">>>>>>> "))
}

/// Resolve a conflicted file with what the worktree holds. A file that
/// still has conflict markers is refused unless `allow_markers` says the
/// lines are meant.
pub fn mark_resolved(repo: &Repository, path: &str, allow_markers: bool) -> Result<()> {
    conflict_at(repo, say!("sequence-what-resolve"), path)?;
    let file = crate::stage::worktree_path(repo, path)?;
    match file.symlink_metadata() {
        Ok(meta) if meta.is_file() => {
            let small = meta.len() <= u64::try_from(crate::stage::MAX_EDIT).unwrap_or(u64::MAX);
            if !allow_markers && small {
                let text = std::fs::read(&file).map_err(|err| Error::Git {
                    context: say!("sequence-reading-resolved-file"),
                    source: Box::new(err),
                })?;
                if has_markers(&text) {
                    return Err(refused(
                        say!("sequence-what-resolve"),
                        say!("sequence-markers-left", path),
                    ));
                }
            }
            exe::run(repo, ["--literal-pathspecs", "add", "--", path], None)?;
        }
        Ok(_) => {
            exe::run(repo, ["--literal-pathspecs", "add", "--", path], None)?;
        }
        // Not in the worktree: resolved as deleted.
        Err(_) => {
            exe::run(
                repo,
                [
                    "--literal-pathspecs",
                    "rm",
                    "--quiet",
                    "--cached",
                    "--",
                    path,
                ],
                None,
            )?;
        }
    }
    Ok(())
}

/// The files that were in conflict in the operation that is under way and
/// are resolved now: the ones git remembers the conflict of, and can put
/// back in conflict. Nothing while nothing is in progress.
pub fn resolved(repo: &Repository) -> Result<Vec<String>> {
    if repo.workdir().is_none() || in_progress(repo)?.is_none() {
        return Ok(Vec::new());
    }
    let listed = exe::run(repo, ["ls-files", "--resolve-undo", "-z"], None)?;
    let mut paths: Vec<String> = listed
        .stdout
        .split('\0')
        .filter_map(|entry| entry.split_once('\t'))
        .map(|(_, path)| path.to_owned())
        .collect();
    // One line for every side a file had.
    paths.dedup();
    Ok(paths)
}

/// Put a resolved file back in conflict, from what git remembers of it.
pub fn reopen(repo: &Repository, path: &str) -> Result<()> {
    crate::stage::worktree_path(repo, path)?;
    if in_progress(repo)?.is_none() {
        return Err(refused(
            say!("sequence-what-reopen"),
            say!("sequence-nothing-in-progress"),
        ));
    }
    // Only a file git remembers a resolution of. For any other path the
    // same command is a plain checkout, which would throw local changes
    // away.
    let known = resolved(repo)?.iter().any(|resolved| resolved == path);
    if !known {
        return Err(refused(
            say!("sequence-what-reopen"),
            say!("sequence-not-in-conflict-here", path),
        ));
    }
    exe::run(
        repo,
        ["--literal-pathspecs", "checkout", "--merge", "--", path],
        None,
    )
    .map_err(|err| refused(say!("sequence-what-reopen"), last_words(&err)))?;
    Ok(())
}

/// A stretch of a conflicted file, for the merge tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Region {
    /// Lines that merge cleanly. Each keeps its line ending.
    Same {
        /// The lines.
        lines: Vec<String>,
    },
    /// Lines the two sides changed differently.
    Conflict {
        /// What our side has there.
        ours: Vec<String>,
        /// What both started from.
        base: Vec<String>,
        /// What their side has there.
        theirs: Vec<String>,
    },
}

/// A conflicted file taken apart for the merge tool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Detail {
    pub path: String,
    pub kind: ConflictKind,
    /// The file in order, when both sides have it as text. Empty when
    /// `whole_only` says why not.
    pub regions: Vec<Region>,
    /// Set when the file can only be resolved as a whole: a side deleted
    /// it, or it is not text.
    pub whole_only: Option<String>,
    /// What rerere put into the file from its record, when it did: the
    /// text the file holds now. The path is still unmerged, and this is
    /// where its resolution starts from, not from the two sides again.
    #[serde(default)]
    pub reused: Option<String>,
}

/// The text rerere left in a conflicted file it resolved from its record.
/// Nothing when it did not, or when what is there is not text of a size
/// the merge tool takes.
fn reused_text(repo: &Repository, conflict: &Conflict) -> Option<String> {
    if !conflict.reused {
        return None;
    }
    let file = crate::stage::worktree_path(repo, &conflict.path).ok()?;
    let meta = file.symlink_metadata().ok()?;
    if !meta.is_file() || meta.len() > u64::try_from(MAX_MERGE_TEXT).unwrap_or(u64::MAX) {
        return None;
    }
    let text = String::from_utf8(std::fs::read(&file).ok()?).ok()?;
    (!text.contains('\0') && !has_markers(text.as_bytes())).then_some(text)
}

/// Largest version of a file taken apart for the merge tool.
const MAX_MERGE_TEXT: usize = 4 * 1024 * 1024;

/// The longest run of a marker character at the start of a line in `text`,
/// so that the markers asked of git can be longer than any of them.
fn longest_marker_run(text: &str) -> usize {
    text.lines()
        .map(|line| {
            let first = line.chars().next();
            match first {
                Some(ch @ ('<' | '=' | '>' | '|')) => {
                    line.chars().take_while(|other| *other == ch).count()
                }
                _ => 0,
            }
        })
        .max()
        .unwrap_or(0)
}

/// Cut the output of `git merge-file --diff3` into regions. `size` is the
/// marker size it was run with, longer than anything in the three inputs,
/// so a line that looks like a marker is one.
fn regions_of(merged: &str, size: usize) -> Vec<Region> {
    #[derive(PartialEq)]
    enum At {
        Same,
        Ours,
        Base,
        Theirs,
    }
    let marker = |line: &str, ch: char, labelled: bool| {
        let run: String = std::iter::repeat_n(ch, size).collect();
        let rest = line
            .trim_end_matches(['\n', '\r'])
            .strip_prefix(run.as_str());
        match rest {
            Some("") => true,
            Some(rest) => labelled && rest.starts_with(' '),
            None => false,
        }
    };
    let mut out: Vec<Region> = Vec::new();
    let mut at = At::Same;
    let (mut same, mut ours, mut base, mut theirs) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    for line in merged.split_inclusive('\n') {
        match at {
            At::Same if marker(line, '<', true) => {
                if !same.is_empty() {
                    out.push(Region::Same {
                        lines: std::mem::take(&mut same),
                    });
                }
                at = At::Ours;
            }
            At::Ours if marker(line, '|', true) => at = At::Base,
            At::Ours | At::Base if marker(line, '=', false) => at = At::Theirs,
            At::Theirs if marker(line, '>', true) => {
                out.push(Region::Conflict {
                    ours: std::mem::take(&mut ours),
                    base: std::mem::take(&mut base),
                    theirs: std::mem::take(&mut theirs),
                });
                at = At::Same;
            }
            At::Same => same.push(line.to_owned()),
            At::Ours => ours.push(line.to_owned()),
            At::Base => base.push(line.to_owned()),
            At::Theirs => theirs.push(line.to_owned()),
        }
    }
    // Output that ends inside a conflict was cut short; what was read is
    // kept as a conflict rather than dropped.
    if at != At::Same {
        out.push(Region::Conflict { ours, base, theirs });
    } else if !same.is_empty() {
        out.push(Region::Same { lines: same });
    }
    out
}

/// Take a conflicted file apart: the regions that merge and the ones that
/// do not, with each side's lines.
pub fn detail(repo: &Repository, path: &str) -> Result<Detail> {
    let conflict = conflict_at(repo, say!("sequence-what-resolve"), path)?;
    let whole = |why: &str| Detail {
        path: path.to_owned(),
        kind: conflict.kind,
        regions: Vec::new(),
        whole_only: Some(why.to_owned()),
        reused: None,
    };
    if conflict.submodule {
        return Ok(whole(say!("sequence-whole-submodule")));
    }
    let (Some(ours), Some(theirs)) = (conflict.ours, conflict.theirs) else {
        return Ok(whole(say!("sequence-whole-deleted")));
    };
    let blob = |id: Oid| -> Result<Option<String>> {
        let out = exe::run_with_code(repo, ["cat-file", "blob", id.to_hex().as_str()], None)?;
        let text = out.1.stdout;
        // Lossy decoding replaces what is not UTF-8; such a file is not
        // taken apart, since saving it would not give the bytes back.
        let fits = out.0 == Some(0)
            && text.len() <= MAX_MERGE_TEXT
            && !text.contains('\0')
            && !text.contains('\u{fffd}');
        Ok(fits.then_some(text))
    };
    let (Some(ours_text), Some(theirs_text)) = (blob(ours)?, blob(theirs)?) else {
        return Ok(whole(say!("sequence-whole-not-text")));
    };
    let base_text = match conflict.base {
        Some(id) => blob(id)?.unwrap_or_default(),
        None => String::new(),
    };
    // git merge-file works on files; they go next to the journal and are
    // removed again.
    let dir = repo
        .git_dir()
        .join("bitbraid")
        .join(format!("merge-{}", std::process::id()));
    let io = |context: &'static str| {
        move |err: std::io::Error| Error::Git {
            context,
            source: Box::new(err),
        }
    };
    std::fs::create_dir_all(&dir).map_err(io(say!("sequence-making-room")))?;
    let files = [
        (dir.join("ours"), &ours_text),
        (dir.join("base"), &base_text),
        (dir.join("theirs"), &theirs_text),
    ];
    let written = files
        .iter()
        .try_for_each(|(file, text)| std::fs::write(file, text));
    let size = [&ours_text, &base_text, &theirs_text]
        .iter()
        .map(|text| longest_marker_run(text))
        .max()
        .unwrap_or(0)
        .max(6)
        .saturating_add(1);
    let marker_size = format!("--marker-size={size}");
    let merged = written
        .map_err(io(say!("sequence-writing-version")))
        .and_then(|()| {
            let arg = |ix: usize| {
                files
                    .get(ix)
                    .map(|(file, _)| file.as_os_str().to_owned())
                    .unwrap_or_default()
            };
            exe::run_with_code(
                repo,
                [
                    "merge-file".into(),
                    "-p".into(),
                    "--diff3".into(),
                    marker_size.as_str().into(),
                    arg(0),
                    arg(1),
                    arg(2),
                ] as [std::ffi::OsString; 7],
                None,
            )
        });
    let _ = std::fs::remove_dir_all(&dir);
    let (code, out) = merged?;
    // The exit code is the number of conflicts; above 127 it is an error.
    if !code.is_some_and(|code| (0..=127).contains(&code)) {
        return Err(refused(
            say!("sequence-what-resolve"),
            say!("sequence-merge-failed", path, stderr = out.stderr.trim()),
        ));
    }
    Ok(Detail {
        path: path.to_owned(),
        kind: conflict.kind,
        regions: regions_of(&out.stdout, size),
        whole_only: None,
        reused: reused_text(repo, &conflict),
    })
}

/// Resolve a conflicted file with `text`: write it and mark the file
/// resolved. Text that still has conflict markers is refused unless
/// `allow_markers` says they are meant.
pub fn write_resolution(
    repo: &Repository,
    path: &str,
    text: &str,
    allow_markers: bool,
) -> Result<()> {
    conflict_at(repo, say!("sequence-what-resolve"), path)?;
    if !allow_markers && has_markers(text.as_bytes()) {
        return Err(refused(
            say!("sequence-what-resolve"),
            say!("sequence-result-has-markers", path),
        ));
    }
    crate::stage::write_text(repo, path, text, None)?;
    exe::run(repo, ["--literal-pathspecs", "add", "--", path], None)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Region, has_markers, longest_marker_run, regions_of};

    fn same(lines: &[&str]) -> Region {
        Region::Same {
            lines: lines.iter().map(|line| (*line).to_owned()).collect(),
        }
    }

    #[test]
    fn merged_text_is_cut_into_what_agrees_and_what_does_not() {
        let merged = "top\n<<<<<<< ours\nmine\n||||||| base\nwas\n=======\ntheirs 1\ntheirs 2\n>>>>>>> theirs\nbottom\n";
        assert_eq!(
            regions_of(merged, 7),
            [
                same(&["top\n"]),
                Region::Conflict {
                    ours: vec!["mine\n".to_owned()],
                    base: vec!["was\n".to_owned()],
                    theirs: vec!["theirs 1\n".to_owned(), "theirs 2\n".to_owned()],
                },
                same(&["bottom\n"]),
            ]
        );
        // Put back together from one side, the text is that side's.
        let rebuilt: String = regions_of(merged, 7)
            .iter()
            .flat_map(|region| match region {
                Region::Same { lines } => lines.clone(),
                Region::Conflict { theirs, .. } => theirs.clone(),
            })
            .collect();
        assert_eq!(rebuilt, "top\ntheirs 1\ntheirs 2\nbottom\n");

        // Lines that look like markers of another size are text.
        let tricky =
            "=======\n<<<<<<< not a marker\n<<<<<<<<< ours\na\n=========\nb\n>>>>>>>>> theirs\n";
        assert_eq!(
            regions_of(tricky, 9),
            [
                same(&["=======\n", "<<<<<<< not a marker\n"]),
                Region::Conflict {
                    ours: vec!["a\n".to_owned()],
                    base: Vec::new(),
                    theirs: vec!["b\n".to_owned()],
                },
            ]
        );
        // No conflict, no text, a last line without its break, and output
        // cut short in the middle of a conflict.
        assert_eq!(regions_of("a\nb", 7), [same(&["a\n", "b"])]);
        assert!(regions_of("", 7).is_empty());
        assert_eq!(
            regions_of("<<<<<<< ours\nmine\n", 7),
            [Region::Conflict {
                ours: vec!["mine\n".to_owned()],
                base: Vec::new(),
                theirs: Vec::new(),
            }]
        );
    }

    #[test]
    fn markers_are_made_longer_than_anything_in_the_file() {
        assert_eq!(longest_marker_run("plain\ntext\n"), 0);
        assert_eq!(longest_marker_run("a\n<<<<<<<<<< ten\n=== three\n"), 10);
        assert_eq!(longest_marker_run("  <<<< indented does not count\n"), 0);
        assert_eq!(longest_marker_run(""), 0);
    }

    #[test]
    fn conflict_markers_are_lines_that_start_like_git_writes_them() {
        assert!(has_markers(
            b"a\n<<<<<<< HEAD\nb\n=======\nc\n>>>>>>> topic\n"
        ));
        assert!(has_markers(b">>>>>>> topic"));
        // Seven of them inside a line, or without the space, are not.
        assert!(!has_markers(b"see <<<<<<< here\n"));
        assert!(!has_markers(b"<<<<<<<\n=======\n"));
        assert!(!has_markers(b""));
        assert!(!has_markers(&[0xff, b'\n', 0x00]));
    }
}
