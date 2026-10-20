//! Worktrees: list them with what is checked out and how much is changed
//! in each, add one, remove one, lock and unlock one, and prune those
//! whose directory is gone. A worktree is named by its path, and that has
//! to be one git lists.
//!
//! Sources: git-worktree(1) (the commands, and the fields of `list
//! --porcelain -z`), git-status(1), gitrepository-layout(5).

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{
    DeletedBranch, Error, Oid, Reporter, Repository, Result, branch, exe, remote::parse_progress,
};

/// Changes are counted in this many worktrees; a repository with more has
/// the rest listed without a count.
const COUNTED: usize = 64;
/// The longest path and the longest lock reason that are taken.
const LONGEST: usize = 4096;
/// The longest folder Git for Windows makes a worktree in, in UTF-16
/// units, whatever `core.longpaths` says: it hands the new worktree its
/// `.git` file as `$GIT_DIR` and refuses one longer than its `PATH_MAX`
/// (260) less 40, and the folder is that path without its `/.git`.
const WINDOWS_WORKTREE_LIMIT: usize = 215;

/// One worktree as git lists it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct WorktreeInfo {
    /// Its working directory (the repository itself for a bare one).
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// The commit checked out, absent on a branch without commits.
    pub head: Option<Oid>,
    /// The branch checked out, by its short name.
    pub branch: Option<String>,
    /// HEAD is detached.
    pub detached: bool,
    /// A bare repository's own entry: there is no working directory.
    pub bare: bool,
    /// The main worktree, which is listed first.
    pub is_main: bool,
    /// The worktree this repository was opened in.
    pub current: bool,
    /// Locked against removal and pruning.
    pub locked: bool,
    /// Why, when a reason was given.
    pub lock_reason: Option<String>,
    /// Its directory is gone; pruning removes what git still keeps of it.
    pub prunable: bool,
    /// What git says is wrong with it.
    pub prune_reason: Option<String>,
    /// How many paths are changed or untracked in it. Absent when that was
    /// not asked for or could not be told.
    pub changes: Option<usize>,
}

/// What a new worktree checks out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// A local branch that exists and is checked out nowhere else.
    Branch(String),
    /// A branch made for it at `start` (HEAD when absent). When `start` is
    /// a branch of a remote, the new branch follows it.
    NewBranch {
        /// Short name of the branch to make.
        name: String,
        /// Where it begins.
        start: Option<String>,
    },
    /// A commit, with HEAD detached.
    Detached(String),
}

/// What removing a worktree did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Removed {
    /// The directory that is gone.
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// The branch it had checked out.
    pub branch: Option<String>,
    /// That branch, when it was asked to go too and went.
    pub deleted: Option<DeletedBranch>,
    /// Why the branch is still there although it was asked to go.
    pub branch_kept: Option<String>,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("worktree-what-worktree"),
        detail: detail.into(),
    }
}

/// Cut the listing into worktrees. Attributes end in a NUL and a worktree
/// ends in one more; an attribute is a word, or a word and its value.
fn parse_list(text: &str) -> Vec<WorktreeInfo> {
    let mut out: Vec<WorktreeInfo> = Vec::new();
    for record in text.split("\0\0") {
        let mut info: Option<WorktreeInfo> = None;
        for attribute in record.split('\0') {
            let (word, value) = match attribute.split_once(' ') {
                Some((word, value)) => (word, Some(value)),
                None => (attribute, None),
            };
            if word == "worktree" {
                info = value
                    .filter(|path| !path.is_empty())
                    .map(|path| WorktreeInfo {
                        // As the system writes it: on Windows git prints
                        // `C:/work/repo`.
                        path: bitbraid_core::paths::native(PathBuf::from(path)),
                        head: None,
                        branch: None,
                        detached: false,
                        bare: false,
                        is_main: out.is_empty(),
                        current: false,
                        locked: false,
                        lock_reason: None,
                        prunable: false,
                        prune_reason: None,
                        changes: None,
                    });
                continue;
            }
            let Some(info) = info.as_mut() else { continue };
            let text = || value.filter(|text| !text.is_empty()).map(str::to_owned);
            match word {
                "HEAD" => {
                    // An unborn branch is listed with an id of zeros.
                    info.head = value
                        .and_then(|hex| hex.parse::<Oid>().ok())
                        .filter(|id| *id != Oid::zero());
                }
                "branch" => {
                    info.branch = value
                        .map(|name| name.strip_prefix("refs/heads/").unwrap_or(name).to_owned());
                }
                "detached" => info.detached = true,
                "bare" => info.bare = true,
                "locked" => {
                    info.locked = true;
                    info.lock_reason = text();
                }
                "prunable" => {
                    info.prunable = true;
                    info.prune_reason = text();
                }
                _ => {} // an attribute a later git added
            }
        }
        out.extend(info);
    }
    out
}

/// The same place, whatever way it is written. A directory that is gone
/// cannot be resolved and is compared as it stands.
fn same_place(a: &Path, b: &Path) -> bool {
    match (
        bitbraid_core::paths::canonical(a),
        bitbraid_core::paths::canonical(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// How many paths `git status` lists in the worktree at `dir`.
fn count_changes(dir: &Path) -> Option<usize> {
    if !dir.is_dir() {
        return None;
    }
    // Without -z every entry is one line, a rename included.
    let out = exe::run_in(dir, ["status", "--porcelain", "--untracked-files=normal"]).ok()?;
    Some(out.stdout.lines().filter(|line| !line.is_empty()).count())
}

/// Every worktree, the main one first. With `changes`, each is asked how
/// many paths are changed in it, which starts git once per worktree.
pub fn list(repo: &Repository, changes: bool) -> Result<Vec<WorktreeInfo>> {
    let out = exe::run(repo, ["worktree", "list", "--porcelain", "-z"], None)?;
    let mut all = parse_list(&out.stdout);
    let here = repo.workdir();
    for (position, info) in all.iter_mut().enumerate() {
        info.current = match here {
            Some(here) => !info.bare && same_place(&info.path, here),
            None => info.bare,
        };
        if changes && position < COUNTED && !info.bare && !info.prunable {
            info.changes = count_changes(&info.path);
        }
    }
    Ok(all)
}

/// A path as it was typed: something, of a length a path can have, without
/// control characters. A relative one is taken from the working directory
/// git is run in, which also keeps it from being read as an option.
fn typed_path(repo: &Repository, text: &str) -> Result<PathBuf> {
    if text.is_empty() {
        return Err(invalid(
            say!("worktree-what-path"),
            say!("worktree-path-empty"),
        ));
    }
    if text.len() > LONGEST {
        return Err(invalid(
            say!("worktree-what-path"),
            say!("worktree-too-long"),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(
            say!("worktree-what-path"),
            say!("worktree-path-control", text = format!("{text:?}")),
        ));
    }
    let path = Path::new(text);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.workdir().unwrap_or_else(|| repo.git_dir()).join(path)
    })
}

/// Refuse a folder Git for Windows makes no worktree in
/// ([`WINDOWS_WORKTREE_LIMIT`]), before git is asked: it would stop
/// halfway with "'$GIT_DIR' too big" or "could not create leading
/// directories". `windows` says whether that limit holds here.
fn worktree_folder_fits(target: &Path, windows: bool) -> Result<()> {
    let text = target.to_string_lossy();
    let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let length = bitbraid_core::paths::windows_length(plain);
    if windows && length > WINDOWS_WORKTREE_LIMIT {
        return Err(refused(say!(
            "worktree-folder-too-long-for-windows",
            path = plain,
            length,
            most = WINDOWS_WORKTREE_LIMIT
        )));
    }
    Ok(())
}

/// The listed worktree a path names.
fn find(repo: &Repository, text: &str, changes: bool) -> Result<WorktreeInfo> {
    let path = typed_path(repo, text)?;
    list(repo, changes)?
        .into_iter()
        .find(|info| same_place(&info.path, &path))
        .ok_or_else(|| {
            invalid(
                say!("worktree-what-worktree"),
                say!("worktree-not-listed", text = format!("{text:?}")),
            )
        })
}

fn os(args: &[&str]) -> Vec<OsString> {
    args.iter().map(OsString::from).collect()
}

/// Run git while it reports what it is doing, stoppable.
fn run_reporting(repo: &Repository, args: &[OsString], reporter: &mut Reporter<'_>) -> Result<()> {
    let cancel = reporter.cancel.clone();
    exe::run_streaming(repo, args, &cancel, &mut |line| {
        if let Some(progress) = parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    })?;
    Ok(())
}

/// Add a worktree at `path`, which must not exist or be an empty
/// directory, and check `start` out in it.
pub fn add(
    repo: &Repository,
    path: &str,
    start: &Start,
    reporter: &mut Reporter<'_>,
) -> Result<WorktreeInfo> {
    let target = typed_path(repo, path)?;
    worktree_folder_fits(&target, cfg!(windows))?;
    let was_there = target.exists();
    if was_there {
        let empty = target.is_dir()
            && std::fs::read_dir(&target).is_ok_and(|mut entries| entries.next().is_none());
        if !empty {
            return Err(refused(say!(
                "worktree-not-empty",
                path = target.display().to_string()
            )));
        }
    }
    let existing = list(repo, false)?;
    if existing.iter().any(|info| same_place(&info.path, &target)) {
        return Err(refused(say!(
            "worktree-already",
            path = target.display().to_string()
        )));
    }

    let mut args = os(&["worktree", "add"]);
    let mut made_branch = None;
    let commit_ish = match start {
        Start::Branch(name) => {
            branch::check_branch_name(repo, name)?;
            if branch::branch_tip(repo, name).is_none() {
                return Err(refused(say!("worktree-no-branch", name = name.as_str())));
            }
            if let Some(holder) = existing
                .iter()
                .find(|info| info.branch.as_deref() == Some(name.as_str()))
            {
                return Err(refused(say!(
                    "worktree-checked-out-in",
                    name = name.as_str(),
                    path = holder.path.display().to_string()
                )));
            }
            // By its short name: that is what makes git check the branch
            // out rather than its commit.
            name.clone()
        }
        Start::NewBranch { name, start } => {
            branch::check_branch_name(repo, name)?;
            if branch::branch_tip(repo, name).is_some() {
                return Err(refused(say!(
                    "worktree-branch-exists",
                    name = name.as_str()
                )));
            }
            let start = start.as_deref().unwrap_or("HEAD");
            let commit = branch::resolve(repo, start).ok_or_else(|| {
                invalid(
                    say!("worktree-what-start-point"),
                    say!("worktree-names-no-commit", rev = format!("{start:?}")),
                )
            })?;
            let remote_branch = format!("refs/remotes/{start}");
            let follows = branch::resolve(repo, &remote_branch).is_some();
            args.extend(os(&[if follows { "--track" } else { "--no-track" }, "-b"]));
            args.push(name.into());
            made_branch = Some(name.clone());
            if follows {
                remote_branch
            } else {
                commit.to_hex()
            }
        }
        Start::Detached(rev) => {
            let commit = branch::resolve(repo, rev).ok_or_else(|| {
                invalid(
                    say!("worktree-what-start-point"),
                    say!("worktree-names-no-commit", rev = format!("{rev:?}")),
                )
            })?;
            args.push("--detach".into());
            commit.to_hex()
        }
    };
    args.push("--".into());
    args.push(target.clone().into_os_string());
    args.push(commit_ish.into());

    if let Err(err) = run_reporting(repo, &args, reporter) {
        // Stopped or failed half-way: leave nothing of it behind.
        let force = [
            "worktree".into(),
            "remove".into(),
            "--force".into(),
            "--".into(),
            target.clone().into_os_string(),
        ];
        let _ = exe::run(repo, force, None);
        if !was_there && target.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
        let _ = exe::run(repo, ["worktree", "prune"], None);
        if let Some(name) = made_branch
            && branch::branch_tip(repo, &name).is_some()
        {
            let _ = exe::run(repo, ["branch", "-D", "--", name.as_str()], None);
        }
        return Err(err);
    }
    list(repo, false)?
        .into_iter()
        .find(|info| same_place(&info.path, &target))
        .ok_or_else(|| {
            refused(say!(
                "worktree-not-made",
                path = target.display().to_string()
            ))
        })
}

/// Remove a linked worktree. The main one, the one this repository is open
/// in and a locked one are refused; one with changes is refused without
/// `force`. With `delete_branch` the branch it had checked out is deleted
/// afterwards, as [`branch::branch_delete`] does it (`force` lets an
/// unmerged one go).
pub fn remove(repo: &Repository, path: &str, force: bool, delete_branch: bool) -> Result<Removed> {
    let info = find(repo, path, true)?;
    let shown = info.path.display();
    if info.is_main {
        return Err(refused(say!("worktree-main-not-removed")));
    }
    if info.current {
        return Err(refused(say!(
            "worktree-open-not-removed",
            path = shown.to_string()
        )));
    }
    if info.locked {
        return Err(refused(match &info.lock_reason {
            Some(reason) => say!("worktree-locked-because", path = shown.to_string(), reason),
            None => say!("worktree-locked", path = shown.to_string()),
        }));
    }
    if info.prunable {
        return Err(refused(say!("worktree-gone", path = shown.to_string())));
    }
    if !force && let Some(count) = info.changes.filter(|count| *count > 0) {
        return Err(refused(say!(
            "worktree-has-changes",
            path = shown.to_string(),
            count
        )));
    }
    let mut args = os(&["worktree", "remove"]);
    if force {
        args.push("--force".into());
    }
    args.push("--".into());
    args.push(info.path.clone().into_os_string());
    exe::run(repo, args, None)?;

    let (mut deleted, mut branch_kept) = (None, None);
    if delete_branch && let Some(name) = &info.branch {
        match branch::branch_delete(repo, name, force) {
            Ok(gone) => deleted = Some(gone),
            Err(err) => branch_kept = Some(err.to_string()),
        }
    }
    Ok(Removed {
        path: info.path,
        branch: info.branch,
        deleted,
        branch_kept,
    })
}

/// Lock a linked worktree against removal and pruning, with a reason when
/// one is given.
pub fn lock(repo: &Repository, path: &str, reason: Option<&str>) -> Result<WorktreeInfo> {
    let info = find(repo, path, false)?;
    if info.is_main {
        return Err(refused(say!("worktree-main-not-locked")));
    }
    if info.locked {
        return Err(refused(say!(
            "worktree-locked-already",
            path = info.path.display().to_string()
        )));
    }
    let reason = reason.map(str::trim).filter(|reason| !reason.is_empty());
    if let Some(reason) = reason {
        if reason.len() > LONGEST {
            return Err(invalid(
                say!("worktree-what-lock-reason"),
                say!("worktree-too-long"),
            ));
        }
        if reason.contains('\0') {
            return Err(invalid(
                say!("worktree-what-lock-reason"),
                say!("worktree-reason-nul"),
            ));
        }
    }
    let mut args = os(&["worktree", "lock"]);
    if let Some(reason) = reason {
        args.push("--reason".into());
        args.push(reason.into());
    }
    args.push("--".into());
    args.push(info.path.into_os_string());
    exe::run(repo, args, None)?;
    find(repo, path, false)
}

/// Move a linked worktree to another folder, which must not be there
/// yet (git would move it into a folder that is). The main worktree, a
/// locked one and the one this repository is opened in stay where they
/// are, each with the reason; one that has submodules git refuses to
/// move, in its own words.
pub fn move_to(repo: &Repository, path: &str, to: &str) -> Result<WorktreeInfo> {
    let info = find(repo, path, false)?;
    if info.is_main {
        return Err(refused(say!("worktree-main-not-moved")));
    }
    if info.locked {
        return Err(refused(say!(
            "worktree-locked",
            path = info.path.display().to_string()
        )));
    }
    if repo
        .workdir()
        .is_some_and(|here| same_place(here, &info.path))
    {
        return Err(refused(say!("worktree-open-not-moved")));
    }
    let target = typed_path(repo, to)?;
    worktree_folder_fits(&target, cfg!(windows))?;
    if target.symlink_metadata().is_ok() {
        return Err(refused(say!(
            "worktree-something-there",
            path = target.display().to_string()
        )));
    }
    let mut args = os(&["worktree", "move", "--"]);
    args.push(info.path.into_os_string());
    args.push(target.clone().into_os_string());
    exe::run(repo, args, None)?;
    list(repo, false)?
        .into_iter()
        .find(|info| same_place(&info.path, &target))
        .ok_or_else(|| {
            invalid(
                say!("worktree-what-worktree"),
                say!("worktree-not-where-moved"),
            )
        })
}

/// Unlock a worktree.
pub fn unlock(repo: &Repository, path: &str) -> Result<WorktreeInfo> {
    let info = find(repo, path, false)?;
    if !info.locked {
        return Err(refused(say!(
            "worktree-not-locked",
            path = info.path.display().to_string()
        )));
    }
    let mut args = os(&["worktree", "unlock", "--"]);
    args.push(info.path.into_os_string());
    exe::run(repo, args, None)?;
    find(repo, path, false)
}

/// Forget the worktrees whose directory is gone, except locked ones.
/// Returns where they were.
pub fn prune(repo: &Repository) -> Result<Vec<PathBuf>> {
    let stale: Vec<PathBuf> = list(repo, false)?
        .into_iter()
        .filter(|info| info.prunable && !info.locked)
        .map(|info| info.path)
        .collect();
    exe::run(repo, ["worktree", "prune"], None)?;
    let left = list(repo, false)?;
    Ok(stale
        .into_iter()
        .filter(|path| !left.iter().any(|info| info.path == *path))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "ccf714f58d4705263a09ea7ce0edb835f7207994";

    #[test]
    fn a_folder_git_for_windows_makes_no_worktree_in_is_refused_there_alone() {
        let folder = |length: usize| {
            let base = r"D:\work\";
            PathBuf::from(format!("{base}{}", "w".repeat(length - base.len())))
        };
        for windows in [false, true] {
            assert!(worktree_folder_fits(&folder(215), windows).is_ok());
            assert!(worktree_folder_fits(&folder(216), false).is_ok());
        }
        let refused = worktree_folder_fits(&folder(216), true)
            .unwrap_err()
            .to_string();
        assert!(refused.contains("216 characters"), "{refused}");
        assert!(refused.contains("215"), "{refused}");
        // Written as a verbatim path, it is as long as without.
        let verbatim = PathBuf::from(format!(r"\\?\{}", folder(215).display()));
        assert!(worktree_folder_fits(&verbatim, true).is_ok());
    }

    #[test]
    fn the_listing_is_cut_into_worktrees_with_their_attributes() {
        let text = format!(
            "worktree /r/main\0HEAD {ID}\0branch refs/heads/main\0\0\
             worktree /r/det\0HEAD {ID}\0detached\0locked two\nlines \"q\"\0\0\
             worktree /r/gone\0HEAD {ID}\0branch refs/heads/topic/a\0prunable gitdir file points to non-existent location\0\0"
        );
        let all = parse_list(&text);
        assert_eq!(all.len(), 3);
        assert!(all[0].is_main && !all[1].is_main && !all[2].is_main);
        assert_eq!(all[0].path, PathBuf::from("/r/main"));
        assert_eq!(all[0].branch.as_deref(), Some("main"));
        assert_eq!(all[0].head, ID.parse().ok());
        assert!(all[1].detached && all[1].locked && all[1].branch.is_none());
        assert_eq!(all[1].lock_reason.as_deref(), Some("two\nlines \"q\""));
        assert_eq!(all[2].branch.as_deref(), Some("topic/a"));
        assert!(all[2].prunable && !all[2].locked);
        assert!(
            all[2]
                .prune_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("non-existent"))
        );
    }

    #[test]
    fn odd_listings_are_read_without_complaint() {
        assert!(parse_list("").is_empty());
        assert!(parse_list("\0\0\0\0").is_empty());
        // Attributes before any worktree, an empty path, words not known.
        assert!(parse_list("HEAD x\0locked\0\0worktree \0bare\0\0").is_empty());
        let zeros = "0".repeat(40);
        let all = parse_list(&format!(
            "worktree /bare\0bare\0\0worktree /new\0HEAD {zeros}\0branch refs/heads/unborn\0future thing\0locked\0"
        ));
        assert_eq!(all.len(), 2);
        assert!(all[0].bare && all[0].is_main);
        assert_eq!(all[1].head, None, "an unborn branch has no commit");
        assert!(all[1].locked && all[1].lock_reason.is_none());
        // A HEAD that is not an id is no id.
        assert_eq!(parse_list("worktree /x\0HEAD nonsense\0\0")[0].head, None);
    }
}
