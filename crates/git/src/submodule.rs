//! Submodules: what is declared and in what state each is, adding,
//! updating, syncing and removing them, forgetting the clone git keeps of
//! one that was removed, and the commits between two commits a
//! superproject has recorded for one.
//!
//! What `.gitmodules` says is not trusted: a path or a name from it is only
//! used once it is a plain relative path that stays inside the worktree.
//!
//! Sources: git-submodule(1), gitmodules(5), gitsubmodules(7),
//! git-ls-files(1) (`-s`), git-status(1) (porcelain v2, the submodule
//! field), git-diff(1) (`--submodule=log`, for what a submodule's diff
//! shows), git-config(1) (`protocol.allow`), git-rev-parse(1)
//! (`--git-path`).

use std::{
    collections::HashMap,
    ffi::OsString,
    path::{Component, Path, PathBuf},
};

use serde::Serialize;

use crate::{
    DiffSide, Error, Oid, Reporter, Repository, Result, branch, exe, remote::parse_progress,
};

/// More submodules than this are not looked at.
const MOST: usize = 2000;
/// The longest URL and path that are taken.
const LONGEST: usize = 4096;
/// How many commits a direction of [`changes`] lists.
const LISTED: usize = 200;

/// Where a submodule stands against what the superproject records.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SubmoduleState {
    /// Declared, but not cloned and checked out here.
    NotInitialised,
    /// Checked out at the commit that is recorded.
    InStep,
    /// Checked out at another commit than the one recorded.
    Moved,
    /// The superproject is in a conflict over which commit to record.
    Conflicted,
}

/// One submodule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SubmoduleInfo {
    /// Its name in `.gitmodules`.
    pub name: String,
    /// Where it is, relative to the worktree.
    pub path: String,
    /// Where it is cloned from, as `.gitmodules` has it.
    pub url: Option<String>,
    /// The branch `.gitmodules` says it follows.
    pub branch: Option<String>,
    /// The commit the superproject's index records.
    pub recorded: Option<Oid>,
    /// The commit checked out in it.
    pub head: Option<Oid>,
    /// Where it stands.
    pub state: SubmoduleState,
    /// Tracked files of its own are changed.
    pub modified: bool,
    /// It has untracked files of its own.
    pub untracked: bool,
}

/// What removing a submodule did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Removed {
    pub name: String,
    /// Where it was.
    pub path: String,
    /// The clone git kept of it under its own directory is gone too.
    pub purged: bool,
}

/// What forgetting the kept clone of a removed submodule did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Forgotten {
    /// The name the submodule had.
    pub name: String,
    /// Where the clone was, as git names the place.
    pub clone: String,
}

/// A commit of a submodule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SubmoduleCommit {
    /// Its id, in the submodule's repository.
    pub id: Oid,
    /// The first line of its message.
    pub summary: String,
}

/// What lies between two commits recorded for a submodule.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SubmoduleChanges {
    /// The submodule's path.
    pub path: String,
    /// The commit recorded on the old side, absent when there was none.
    pub old: Option<Oid>,
    /// The commit recorded on the new side.
    pub new: Option<Oid>,
    /// Both commits could be read in the submodule's own repository. When
    /// not, the two ids are all there is.
    pub known: bool,
    /// Commits the new side has and the old one has not, newest first.
    pub gained: Vec<SubmoduleCommit>,
    /// Commits the old side had and the new one has not: it went back or
    /// sideways.
    pub lost: Vec<SubmoduleCommit>,
    /// There are more gained commits than are listed.
    pub more_gained: bool,
    /// There are more lost commits than are listed.
    pub more_lost: bool,
    /// The first line of the message of the commit recorded on each side,
    /// when the submodule's own repository has it.
    pub old_summary: Option<String>,
    /// See `old_summary`.
    pub new_summary: Option<String>,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("submodule-what-submodule"),
        detail: detail.into(),
    }
}

/// A relative path that names something inside the worktree: no way up,
/// no way to the root, nothing git could read as an option.
fn inside(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= LONGEST
        && !text.starts_with('-')
        && !text.chars().any(char::is_control)
        && text
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
        && Path::new(text)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn workdir(repo: &Repository) -> Result<&Path> {
    repo.workdir()
        .ok_or_else(|| refused(say!("submodule-bare")))
}

/// A submodule as `.gitmodules` declares it.
struct Declared {
    name: String,
    path: String,
    url: Option<String>,
    branch: Option<String>,
}

/// What `.gitmodules` declares, in its order. Entries without a usable
/// name or path are left out.
fn declared(repo: &Repository) -> Result<Vec<Declared>> {
    let Some(dir) = repo.workdir() else {
        return Ok(Vec::new());
    };
    if !dir.join(".gitmodules").is_file() {
        return Ok(Vec::new());
    }
    // Exit code 1 says that nothing matched.
    let (code, out) = exe::run_with_code(
        repo,
        [
            "config",
            "-z",
            "--file",
            ".gitmodules",
            "--get-regexp",
            r"^submodule\.",
        ],
        None,
    )?;
    if code != Some(0) {
        return Ok(Vec::new());
    }
    let mut order: Vec<String> = Vec::new();
    let mut settings: HashMap<String, HashMap<String, String>> = HashMap::new();
    // Each setting is its key, a line feed, its value and a NUL.
    for record in out.stdout.split('\0') {
        let Some((key, value)) = record.split_once('\n') else {
            continue;
        };
        let Some((name, variable)) = key
            .strip_prefix("submodule.")
            .and_then(|rest| rest.rsplit_once('.'))
        else {
            continue;
        };
        if !settings.contains_key(name) {
            if order.len() >= MOST {
                continue;
            }
            order.push(name.to_owned());
        }
        settings
            .entry(name.to_owned())
            .or_default()
            .insert(variable.to_owned(), value.to_owned());
    }
    let mut out = Vec::new();
    for name in order {
        let Some(mut found) = settings.remove(&name) else {
            continue;
        };
        let Some(path) = found.remove("path").filter(|path| inside(path)) else {
            continue;
        };
        if !inside(&name) {
            continue;
        }
        let text = |value: Option<String>| value.filter(|text| !text.is_empty());
        out.push(Declared {
            name,
            path,
            url: text(found.remove("url")),
            branch: text(found.remove("branch")),
        });
    }
    Ok(out)
}

/// The commits the index records for these paths, by stage. A path in a
/// conflict has stages 1 to 3 instead of stage 0.
fn recorded(repo: &Repository, paths: &[&str]) -> Result<HashMap<String, Vec<(u8, Oid)>>> {
    let mut found: HashMap<String, Vec<(u8, Oid)>> = HashMap::new();
    if paths.is_empty() {
        return Ok(found);
    }
    let mut args = vec!["ls-files", "-s", "-z", "--"];
    args.extend(paths);
    let out = exe::run(repo, args, None)?;
    for record in out.stdout.split('\0') {
        // `<mode> <id> <stage>`, a tab, the path.
        let Some((head, path)) = record.split_once('\t') else {
            continue;
        };
        let mut parts = head.split(' ');
        let (Some("160000"), Some(id), Some(stage)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        if let (Ok(id), Ok(stage)) = (id.parse::<Oid>(), stage.parse::<u8>()) {
            found.entry(path.to_owned()).or_default().push((stage, id));
        }
    }
    Ok(found)
}

/// For these paths, whether the submodule there has changed tracked files
/// and whether it has untracked ones, as `git status` tells it.
fn dirt(repo: &Repository, paths: &[&str]) -> Result<HashMap<String, (bool, bool)>> {
    let mut found = HashMap::new();
    if paths.is_empty() {
        return Ok(found);
    }
    let mut args = vec!["status", "--porcelain=v2", "-z", "--"];
    args.extend(paths);
    let out = exe::run(repo, args, None)?;
    let mut records = out.stdout.split('\0');
    while let Some(record) = records.next() {
        // An ordinary entry has eight fields before its path, an unmerged
        // one ten; a renamed one is followed by the path it had.
        let fields = match record.as_bytes().first() {
            Some(b'1') => 9,
            Some(b'u') => 11,
            Some(b'2') => {
                let _ = records.next();
                continue;
            }
            _ => continue,
        };
        let mut parts = record.splitn(fields, ' ');
        let sub = parts.nth(2).unwrap_or("");
        let Some(path) = parts.last() else { continue };
        // `S`, then a letter or a dot each for: the commit changed,
        // tracked files changed, untracked files.
        let mut flags = sub.chars();
        if flags.next() != Some('S') {
            continue;
        }
        let _commit = flags.next();
        let modified = flags.next() == Some('M');
        let untracked = flags.next() == Some('U');
        found.insert(path.to_owned(), (modified, untracked));
    }
    Ok(found)
}

/// The commit checked out in the repository at `dir`, when there is one
/// there. Without a `.git` of its own git would answer for the
/// superproject, so that is looked at first.
fn head_in(dir: &Path) -> Option<Oid> {
    if !dir.join(".git").exists() {
        return None;
    }
    exe::run_in(dir, ["rev-parse", "--verify", "--quiet", "HEAD"])
        .ok()
        .and_then(|out| out.stdout.trim().parse().ok())
}

/// Every submodule `.gitmodules` declares, with where it stands.
pub fn list(repo: &Repository) -> Result<Vec<SubmoduleInfo>> {
    let declared = declared(repo)?;
    let Some(dir) = repo.workdir() else {
        return Ok(Vec::new());
    };
    let paths: Vec<&str> = declared.iter().map(|one| one.path.as_str()).collect();
    let recorded = recorded(repo, &paths)?;
    let dirt = dirt(repo, &paths)?;
    let mut out = Vec::new();
    for Declared {
        name,
        path,
        url,
        branch,
    } in &declared
    {
        let stages = recorded.get(path).map(Vec::as_slice).unwrap_or(&[]);
        let conflicted = stages.iter().any(|(stage, _)| *stage != 0);
        let in_index = stages
            .iter()
            .find(|(stage, _)| *stage == 0)
            .map(|(_, id)| *id);
        let initialised = dir.join(path).join(".git").exists();
        let head = head_in(&dir.join(path));
        let state = if conflicted {
            SubmoduleState::Conflicted
        } else if !initialised {
            SubmoduleState::NotInitialised
        } else if head == in_index {
            SubmoduleState::InStep
        } else {
            SubmoduleState::Moved
        };
        let (modified, untracked) = dirt.get(path).copied().unwrap_or((false, false));
        out.push(SubmoduleInfo {
            name: name.clone(),
            path: path.clone(),
            url: url.clone(),
            branch: branch.clone(),
            recorded: in_index,
            head,
            state,
            modified,
            untracked,
        });
    }
    Ok(out)
}

/// The listed submodule at `path`.
fn find(repo: &Repository, path: &str) -> Result<SubmoduleInfo> {
    let not_one = || {
        invalid(
            say!("submodule-what-submodule"),
            say!("submodule-not-one", path = format!("{path:?}")),
        )
    };
    let wanted = path.strip_suffix('/').unwrap_or(path);
    if !inside(wanted) {
        return Err(not_one());
    }
    list(repo)?
        .into_iter()
        .find(|info| info.path == wanted)
        .ok_or_else(not_one)
}

/// The paths asked for, each a listed submodule; none means all of them.
fn listed_paths(repo: &Repository, paths: &[String]) -> Result<Vec<String>> {
    paths
        .iter()
        .map(|path| find(repo, path).map(|info| info.path))
        .collect()
}

/// Git for a submodule command: with clones from local paths allowed when
/// that was asked for, which git refuses by itself.
fn submodule_args(allow_local: bool, rest: &[&str]) -> Vec<OsString> {
    let mut args: Vec<OsString> = Vec::new();
    if allow_local {
        args.extend(["-c".into(), "protocol.file.allow=always".into()]);
    }
    args.push("submodule".into());
    args.extend(rest.iter().map(OsString::from));
    args
}

fn run_reporting(repo: &Repository, args: &[OsString], reporter: &mut Reporter<'_>) -> Result<()> {
    let cancel = reporter.cancel.clone();
    exe::run_streaming(repo, args, &cancel, &mut |line| {
        if let Some(progress) = parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    })?;
    Ok(())
}

/// Where git keeps the clone of the submodule called `name`.
fn clone_dir(repo: &Repository, name: &str) -> Option<PathBuf> {
    if !inside(name) {
        return None;
    }
    let asked = format!("modules/{name}");
    let out = exe::run(repo, ["rev-parse", "--git-path", asked.as_str()], None).ok()?;
    let answer = out.stdout.trim_end_matches(['\n', '\r']);
    if answer.is_empty() {
        return None;
    }
    let path = Path::new(answer);
    Some(if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.workdir()?.join(path)
    })
}

/// The clone git keeps of the submodule called `name`, when there is one
/// and it is safe to delete: a directory of its own with a `HEAD` in it,
/// that really lies under the repository's `modules` directory. A link
/// anywhere on the way there (`modules/a` pointing out of the repository
/// while `a/b` is asked for) would have a deletion follow it; such a
/// place is not a clone of ours.
fn kept_clone(repo: &Repository, name: &str) -> Option<PathBuf> {
    let kept = clone_dir(repo, name)?;
    if kept.is_symlink() || !kept.is_dir() || !kept.join("HEAD").is_file() {
        return None;
    }
    let root = bitbraid_core::paths::canonical(clone_dir(repo, "x")?.parent()?).ok()?;
    let real = bitbraid_core::paths::canonical(&kept).ok()?;
    let rest = real.strip_prefix(&root).ok()?;
    // As many steps below `modules` as the name has, and the same ones.
    (rest == Path::new(name)).then_some(kept)
}

/// Add a submodule cloned from `url` at `path`, which must not exist,
/// following `branch` when one is given. The result is staged, as git
/// leaves it.
pub fn add(
    repo: &Repository,
    url: &str,
    path: &str,
    branch_name: Option<&str>,
    allow_local: bool,
    reporter: &mut Reporter<'_>,
) -> Result<SubmoduleInfo> {
    let dir = workdir(repo)?;
    if url.is_empty()
        || url.len() > LONGEST
        || url.starts_with('-')
        || url.chars().any(char::is_control)
    {
        return Err(invalid(
            say!("submodule-what-url"),
            say!("submodule-url-unusable", url = format!("{url:?}")),
        ));
    }
    let path = path.strip_suffix('/').unwrap_or(path);
    if !inside(path) {
        return Err(invalid(
            say!("submodule-what-path"),
            say!("submodule-path-outside", path = format!("{path:?}")),
        ));
    }
    if let Some(name) = branch_name {
        branch::check_branch_name(repo, name)?;
    }
    if declared(repo)?
        .iter()
        .any(|one| one.name == path || one.path == path)
    {
        return Err(refused(say!("submodule-already", path)));
    }
    // Git takes a path that exists only when a repository is there
    // already; nothing is adopted here.
    let target = dir.join(path);
    if target.symlink_metadata().is_ok() {
        return Err(refused(say!("submodule-exists", path)));
    }
    let tracked = exe::run(repo, ["ls-files", "-z", "--", path], None)?;
    if !tracked.stdout.is_empty() {
        return Err(refused(say!("submodule-tracked", path)));
    }
    // A clone left from a submodule that was removed would be used again
    // as it is, whatever URL is given now.
    let kept = clone_dir(repo, path);
    if kept.as_deref().is_some_and(Path::exists) {
        return Err(refused(say!("submodule-clone-kept", path)));
    }

    let mut args = submodule_args(allow_local, &["add", "--progress"]);
    if let Some(name) = branch_name {
        args.extend(["--branch".into(), name.into()]);
    }
    args.extend(["--".into(), url.into(), path.into()]);
    if let Err(err) = run_reporting(repo, &args, reporter) {
        // Stopped or failed half-way: leave nothing of it behind. Neither
        // the directory nor the clone was there before.
        if target.is_dir() {
            let _ = std::fs::remove_dir_all(&target);
        }
        if let Some(kept) = kept
            && kept.is_dir()
        {
            let _ = std::fs::remove_dir_all(kept);
        }
        return Err(err);
    }
    find(repo, path)
}

/// Bring submodules to the commits the superproject records: all of them,
/// or those at `paths`. With `init`, those not cloned yet are cloned.
pub fn update(
    repo: &Repository,
    paths: &[String],
    init: bool,
    recursive: bool,
    allow_local: bool,
    reporter: &mut Reporter<'_>,
) -> Result<Vec<SubmoduleInfo>> {
    workdir(repo)?;
    let paths = listed_paths(repo, paths)?;
    let mut args = submodule_args(allow_local, &["update", "--progress"]);
    if init {
        args.push("--init".into());
    }
    if recursive {
        args.push("--recursive".into());
    }
    args.push("--".into());
    args.extend(paths.iter().map(OsString::from));
    run_reporting(repo, &args, reporter)?;
    list(repo)
}

/// Take the URLs from `.gitmodules` over into the configuration the clones
/// use: for all submodules, or those at `paths`.
pub fn sync(repo: &Repository, paths: &[String], recursive: bool) -> Result<Vec<SubmoduleInfo>> {
    workdir(repo)?;
    let paths = listed_paths(repo, paths)?;
    let mut args = submodule_args(false, &["sync"]);
    if recursive {
        args.push("--recursive".into());
    }
    args.push("--".into());
    args.extend(paths.iter().map(OsString::from));
    exe::run(repo, args, None)?;
    list(repo)
}

/// Remove the submodule at `path`: its checkout, its entry in the index
/// and in `.gitmodules`; that is left staged. Without `force` one with
/// changes of its own, or at another commit than the one recorded, is
/// refused. With `purge` the clone git keeps of it goes too, and with it
/// every commit that was only there.
pub fn remove(repo: &Repository, path: &str, force: bool, purge: bool) -> Result<Removed> {
    workdir(repo)?;
    let info = find(repo, path)?;
    let path = info.path.as_str();
    if info.state == SubmoduleState::Conflicted {
        return Err(refused(say!("submodule-in-conflict", path)));
    }
    if !force {
        if info.modified || info.untracked {
            return Err(refused(say!("submodule-changes-of-its-own", path)));
        }
        if info.state == SubmoduleState::Moved {
            return Err(refused(say!("submodule-moved", path)));
        }
    }
    let kept = kept_clone(repo, &info.name);
    let flag: &[&str] = if force { &["--force"] } else { &[] };
    let mut deinit = vec!["submodule", "deinit"];
    deinit.extend(flag);
    deinit.extend(["--", path]);
    exe::run(repo, deinit, None)?;
    let mut rm = vec!["rm", "--quiet"];
    rm.extend(flag);
    rm.extend(["--", path]);
    exe::run(repo, rm, None)?;

    let mut purged = false;
    if purge && let Some(kept) = kept {
        purged = std::fs::remove_dir_all(&kept).is_ok();
    }
    Ok(Removed {
        name: info.name,
        path: info.path,
        purged,
    })
}

/// Delete the clone git still keeps, under its own directory, of a
/// submodule that is no longer part of this repository: what a removal
/// without `purge` leaves behind, and what stands in the way of another
/// submodule at the same place. Every commit that was only in that clone
/// is gone with it. A submodule that is still declared here is refused;
/// that one is removed, not forgotten.
pub fn forget(repo: &Repository, name: &str) -> Result<Forgotten> {
    workdir(repo)?;
    let name = name.strip_suffix('/').unwrap_or(name);
    if !inside(name) {
        return Err(invalid(
            say!("submodule-what-name"),
            say!("submodule-not-a-name", name = format!("{name:?}")),
        ));
    }
    // A name is a path under `modules`: neither the submodule itself nor
    // one that lives further down may still be in use.
    let below = format!("{name}/");
    if let Some(live) = declared(repo)?
        .iter()
        .find(|one| one.name == name || one.name.starts_with(&below))
    {
        return Err(refused(say!(
            "submodule-still-declared",
            name = live.name.as_str()
        )));
    }
    let Some(kept) = kept_clone(repo, name) else {
        return Err(refused(say!("submodule-no-clone", name)));
    };
    std::fs::remove_dir_all(&kept).map_err(|err| Error::Git {
        context: say!("submodule-deleting-clone"),
        source: Box::new(err),
    })?;
    Ok(Forgotten {
        name: name.to_owned(),
        clone: format!("modules/{name}"),
    })
}

/// The commit a side of a diff records for the submodule at `path`.
fn recorded_at(repo: &Repository, side: &DiffSide, path: &str) -> Result<Option<Oid>> {
    let in_index = |repo: &Repository| -> Result<Option<Oid>> {
        Ok(recorded(repo, &[path])?
            .get(path)
            .and_then(|stages| stages.iter().find(|(stage, _)| *stage == 0))
            .map(|(_, id)| *id))
    };
    match side {
        DiffSide::Empty => Ok(None),
        DiffSide::Index => in_index(repo),
        // What is checked out; a submodule that is not there stands where
        // the index has it.
        DiffSide::Worktree => match repo.workdir().and_then(|dir| head_in(&dir.join(path))) {
            Some(head) => Ok(Some(head)),
            None => in_index(repo),
        },
        DiffSide::Commit(id) | DiffSide::Tree(id) => {
            let hex = id.to_hex();
            let out = exe::run(repo, ["ls-tree", "-z", hex.as_str(), "--", path], None)?;
            // `160000 commit <id>`, a tab, the path.
            Ok(out.stdout.split('\0').find_map(|record| {
                let (head, _) = record.split_once('\t')?;
                let mut parts = head.split(' ');
                match (parts.next(), parts.next(), parts.next()) {
                    (Some("160000"), Some("commit"), Some(id)) => id.parse().ok(),
                    _ => None,
                }
            }))
        }
    }
}

/// The first line of the message of a commit of a submodule's own
/// repository, when it has the commit.
fn summary_of(dir: &Path, id: Oid) -> Option<String> {
    let commit = format!("{}^{{commit}}", id.to_hex());
    let out = exe::run_in(dir, ["log", "-1", "--format=%s", commit.as_str(), "--"]).ok()?;
    let line = out.stdout.lines().next()?.to_owned();
    Some(line)
}

/// The commits in `dir` that `to` has and `from` has not, newest first,
/// and whether there are more than are listed.
fn between(dir: &Path, from: Oid, to: Oid) -> (Vec<SubmoduleCommit>, bool) {
    let range = format!("{}..{}", from.to_hex(), to.to_hex());
    let most = format!("--max-count={}", LISTED.saturating_add(1));
    let Ok(out) = exe::run_in(
        dir,
        [
            "log",
            "--format=%H%x00%s",
            most.as_str(),
            range.as_str(),
            "--",
        ],
    ) else {
        return (Vec::new(), false);
    };
    let mut commits: Vec<SubmoduleCommit> = out
        .stdout
        .lines()
        .filter_map(|line| {
            let (id, summary) = line.split_once('\0')?;
            Some(SubmoduleCommit {
                id: id.parse().ok()?,
                summary: summary.to_owned(),
            })
        })
        .collect();
    let more = commits.len() > LISTED;
    commits.truncate(LISTED);
    (commits, more)
}

/// What lies between the commits two sides of a diff record for the
/// submodule at `path`. The sides are named as for a diff: a revision of
/// the superproject, `index`, `worktree` or `empty`.
pub fn changes(repo: &Repository, path: &str, old: &str, new: &str) -> Result<SubmoduleChanges> {
    let path = path.strip_suffix('/').unwrap_or(path);
    if !inside(path) {
        return Err(invalid(
            say!("submodule-what-path"),
            say!("submodule-path-outside", path = format!("{path:?}")),
        ));
    }
    // The parent of a first commit does not exist: nothing was recorded.
    let old_side = match DiffSide::parse(repo, old) {
        Err(_) if old.ends_with('^') => DiffSide::Empty,
        other => other?,
    };
    let new_side = DiffSide::parse(repo, new)?;
    let old = recorded_at(repo, &old_side, path)?;
    let new = recorded_at(repo, &new_side, path)?;
    let mut out = SubmoduleChanges {
        path: path.to_owned(),
        old,
        new,
        known: false,
        gained: Vec::new(),
        lost: Vec::new(),
        more_gained: false,
        more_lost: false,
        old_summary: None,
        new_summary: None,
    };
    if old.is_none() && new.is_none() {
        return Ok(out);
    }
    // Its own repository: the checkout when it is there, else the clone
    // git keeps, found by the name it is declared under.
    let checkout = repo
        .workdir()
        .map(|dir| dir.join(path))
        .filter(|dir| dir.join(".git").exists());
    let dir = match checkout {
        Some(dir) => Some(dir),
        None => declared(repo)?
            .into_iter()
            .find(|one| one.path == path)
            .and_then(|one| clone_dir(repo, &one.name))
            .filter(|dir| dir.join("HEAD").is_file()),
    };
    let Some(dir) = dir else {
        return Ok(out);
    };
    // What each recorded commit says, as far as the clone has it: also
    // for a submodule that was only added or only removed.
    out.old_summary = old.and_then(|id| summary_of(&dir, id));
    out.new_summary = new.and_then(|id| summary_of(&dir, id));
    let (Some(old), Some(new)) = (old, new) else {
        return Ok(out);
    };
    let has = |id: Oid| {
        let commit = format!("{}^{{commit}}", id.to_hex());
        exe::run_in(&dir, ["cat-file", "-e", commit.as_str()]).is_ok()
    };
    if !has(old) || !has(new) {
        return Ok(out);
    }
    out.known = true;
    (out.gained, out.more_gained) = between(&dir, old, new);
    (out.lost, out.more_lost) = between(&dir, new, old);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_plain_relative_paths_count_as_inside() {
        for good in ["a", "deps/my lib", "a/b/c", "weird name-ok", "ä/ö"] {
            assert!(inside(good), "{good:?}");
        }
        let long = "p/".repeat(3000);
        for bad in [
            "",
            "-x",
            "--force",
            "/abs",
            "..",
            "a/../b",
            "../up",
            "./here",
            "a/./b",
            "a\nb",
            "nul\0",
            long.as_str(),
        ] {
            assert!(!inside(bad), "{bad:?}");
        }
    }

    #[test]
    fn local_clones_are_allowed_only_when_asked_for() {
        let plain = submodule_args(false, &["update", "--init"]);
        assert_eq!(plain, ["submodule", "update", "--init"]);
        let allowed = submodule_args(true, &["add"]);
        assert_eq!(
            allowed,
            ["-c", "protocol.file.allow=always", "submodule", "add"]
        );
    }
}
