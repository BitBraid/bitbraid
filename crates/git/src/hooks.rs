//! The hooks git will run in a repository: for every hook git knows by
//! name, the program in the hooks directory, when there is one it can
//! execute.
//!
//! Where the hooks directory is, git says itself (`git rev-parse
//! --git-path hooks`): `core.hooksPath` when that is set, with `~`
//! expanded and a relative path taken from where hooks are run, else
//! `hooks` in the git directory, which the linked worktrees of a
//! repository share. What a hook prints is not read here: git passes it on
//! to its standard error, and that is in the activity log
//! ([`crate::activity`]) with the command that ran the hook.
//!
//! Hooks defined in configuration (`hook.<name>.command`, in recent
//! versions of git) are not listed.
//!
//! Sources: githooks(5), git-config(1) (`core.hooksPath`),
//! git-rev-parse(1) (`--git-path`), git-hook(1); for what `--no-verify`
//! leaves out of each command: git-commit(1), git-merge(1), git-rebase(1),
//! git-push(1), git-am(1).

use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::{Repository, Result, activity::lossy, exe};

/// The hooks git runs, as githooks(5) names them.
pub const NAMES: &[&str] = &[
    "applypatch-msg",
    "pre-applypatch",
    "post-applypatch",
    "pre-commit",
    "pre-merge-commit",
    "prepare-commit-msg",
    "commit-msg",
    "post-commit",
    "pre-rebase",
    "post-checkout",
    "post-merge",
    "pre-push",
    "pre-receive",
    "update",
    "proc-receive",
    "post-receive",
    "post-update",
    "reference-transaction",
    "push-to-checkout",
    "pre-auto-gc",
    "post-rewrite",
    "sendemail-validate",
    "fsmonitor-watchman",
    "p4-changelist",
    "p4-prepare-changelist",
    "p4-post-changelist",
    "p4-pre-submit",
    "post-index-change",
];

/// One hook that will run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hook {
    /// Its name: the event it runs on.
    pub name: &'static str,
    /// The program.
    #[serde(serialize_with = "lossy")]
    pub path: PathBuf,
}

/// The hooks of a repository.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Hooks {
    /// Where git looks for hooks.
    #[serde(serialize_with = "lossy")]
    pub directory: PathBuf,
    /// Whether that comes from `core.hooksPath` rather than being the
    /// repository's own hooks directory.
    pub configured: bool,
    /// The hooks that will run, in the order of githooks(5).
    pub hooks: Vec<Hook>,
}

/// The hooks git will run in this repository.
pub fn hooks(repo: &Repository) -> Result<Hooks> {
    let said = exe::run(repo, ["rev-parse", "--git-path", "hooks"], None)?.stdout;
    // The line's end is git's; anything else is part of the name.
    let said = said.strip_suffix('\n').unwrap_or(&said);
    // Relative to where git ran, which is also where it runs hooks; as the
    // system writes paths (git writes `.git/hooks` on Windows too).
    let directory =
        bitbraid_core::paths::native(repo.workdir().unwrap_or_else(|| repo.git_dir()).join(said));
    // An empty value is a value too: git then looks in the directory it
    // runs in.
    let (code, _) = exe::run_with_code(repo, ["config", "--get", "core.hooksPath"], None)?;
    let hooks = NAMES
        .iter()
        .copied()
        .filter_map(|name| {
            let path = directory.join(name);
            runs(&path).then_some(Hook { name, path })
        })
        .collect();
    Ok(Hooks {
        directory,
        configured: code == Some(0),
        hooks,
    })
}

/// The hooks `git commit --no-verify` leaves out: the two that can refuse
/// a commit. The others (`prepare-commit-msg`, `post-commit`) run all the
/// same.
pub const COMMIT_NO_VERIFY: &[&str] = &["pre-commit", "commit-msg"];

/// A command git takes `--no-verify` with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unverified {
    /// `git commit --no-verify`.
    Commit,
    /// `git merge --no-verify`.
    Merge,
    /// `git rebase --no-verify`.
    Rebase,
    /// `git push --no-verify`.
    Push,
    /// `git am --no-verify`.
    Mailbox,
}

impl Unverified {
    /// The hooks the command leaves out with `--no-verify`, as its manual
    /// page has them: the ones that can refuse it. Every other hook runs
    /// all the same (`post-merge`, `post-rewrite`, `post-applypatch`, and
    /// the hooks of the commits a rebase or a mailbox makes).
    pub const fn hooks(self) -> &'static [&'static str] {
        match self {
            Self::Commit => COMMIT_NO_VERIFY,
            Self::Merge => &["pre-merge-commit", "commit-msg"],
            Self::Rebase => &["pre-rebase"],
            Self::Push => &["pre-push"],
            Self::Mailbox => &["applypatch-msg", "pre-applypatch"],
        }
    }
}

/// Of the hooks `command` leaves out with `--no-verify`, the ones that
/// are there to be left out. Nothing when the hooks cannot be read: this
/// is for saying what was skipped, and nothing fails over it.
pub fn left_out(repo: &Repository, command: Unverified) -> Vec<String> {
    hooks(repo)
        .map(|all| {
            all.hooks
                .iter()
                .filter(|hook| command.hooks().contains(&hook.name))
                .map(|hook| hook.name.to_owned())
                .collect()
        })
        .unwrap_or_default()
}

/// Of the hooks a commit made with `--no-verify` leaves out, the ones
/// that are there to be left out.
pub fn left_out_by_no_verify(repo: &Repository) -> Vec<String> {
    left_out(repo, Unverified::Commit)
}

/// Whether git would run this file: it is there, a file, and marked as
/// executable for someone. Whether it is for this user, only running it
/// tells.
#[cfg(unix)]
fn runs(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

/// Whether git would run this file: where files carry no such mark, being
/// there is enough.
#[cfg(not(unix))]
fn runs(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}
