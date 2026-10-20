//! Stashes: set work aside, bring it back, drop it.
//!
//! A stash is named by its commit id here, never by its position, which
//! shifts whenever another stash is made or dropped; the position is looked
//! up at the moment git needs it.
//!
//! Applying a stash is a merge and can conflict. The merge is tried in
//! memory first (`git merge-tree`, which takes trees from git 2.45 on) and
//! a stash that would conflict is refused before anything changes. With an
//! older git there is no telling, and git's own outcome stands.
//!
//! A stash's message is changed by making its commit anew with the other
//! message (the same tree, parents, author and dates) and putting the new
//! commit in the old one's place in the list.
//!
//! Sources: git-stash(1), git-merge-tree(1), git-write-tree(1),
//! git-commit-tree(1).

use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, branch, exe, stage};

/// What to stash and how.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StashOptions {
    /// What the stash is called; git's "WIP on …" when absent.
    #[serde(default)]
    pub message: Option<String>,
    /// Take untracked files along.
    #[serde(default)]
    pub include_untracked: bool,
    /// Leave what is staged in place (it is stashed as well).
    #[serde(default)]
    pub keep_index: bool,
    /// Stash only what is staged.
    #[serde(default)]
    pub staged_only: bool,
}

/// One entry of the stash list.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StashEntry {
    /// The stash commit.
    pub id: Oid,
    /// Its message in the list.
    pub message: String,
}

fn invalid(detail: impl Into<String>) -> Error {
    Error::Invalid {
        what: say!("stash-what-stash"),
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("stash-what-stash"),
        detail: detail.into(),
    }
}

/// The stash list, newest first.
pub fn stash_list(repo: &Repository) -> Result<Vec<StashEntry>> {
    // `git stash` wants a worktree even to list; without one there is
    // nothing that could have been stashed.
    if repo.is_bare() {
        return Ok(Vec::new());
    }
    let out = exe::run(repo, ["stash", "list", "--format=%H %gs"], None)?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let (id, message) = line.split_once(' ').unwrap_or((line, ""));
            Some(StashEntry {
                id: id.parse().ok()?,
                message: message.to_owned(),
            })
        })
        .collect())
}

/// Where a stash is in the list right now, as git names it.
fn position(repo: &Repository, stash: Oid) -> Result<String> {
    stash_list(repo)?
        .iter()
        .position(|entry| entry.id == stash)
        .map(|index| format!("stash@{{{index}}}"))
        .ok_or_else(|| invalid(say!("stash-not-in-list", stash = stash.short(10))))
}

/// The stash a caller means: the one given, or the newest.
pub fn resolve(repo: &Repository, stash: Option<Oid>) -> Result<Oid> {
    match stash {
        Some(id) => position(repo, id).map(|_| id),
        None => stash_list(repo)?
            .first()
            .map(|entry| entry.id)
            .ok_or_else(|| refused(say!("stash-none"))),
    }
}

/// Stash the local changes. Returns the new stash, or none when there was
/// nothing to stash.
pub fn stash_push(repo: &Repository, options: &StashOptions) -> Result<Option<Oid>> {
    if let Some(message) = &options.message
        && message.chars().any(char::is_control)
    {
        return Err(invalid(say!("stash-message-control")));
    }
    let newest = |repo: &Repository| -> Result<Option<Oid>> {
        Ok(stash_list(repo)?.first().map(|entry| entry.id))
    };
    if options.staged_only && options.include_untracked {
        // Git refuses the two together, in words about its options.
        return Err(invalid(say!("stash-staged-and-untracked")));
    }
    let before = newest(repo)?;
    let mut args = vec!["stash", "push"];
    if options.include_untracked {
        args.push("--include-untracked");
    }
    if options.keep_index {
        args.push("--keep-index");
    }
    if options.staged_only {
        args.push("--staged");
    }
    // An empty message is no message.
    let message = options.message.as_deref().map(str::trim).unwrap_or("");
    if !message.is_empty() {
        args.extend(["--message", message]);
    }
    exe::run(repo, args, None)?;
    let after = newest(repo)?;
    Ok(after.filter(|_| after != before))
}

/// Files the stash's untracked part would write that exist already. Git
/// stops there, after bringing back some of the others.
fn untracked_in_the_way(repo: &Repository, stash: Oid) -> Vec<String> {
    let third = format!("{}^3", stash.to_hex());
    let Ok(out) = exe::run(
        repo,
        ["ls-tree", "-r", "--name-only", "-z", third.as_str()],
        None,
    ) else {
        // No third parent: the stash has no untracked files.
        return Vec::new();
    };
    out.stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .filter(|path| {
            stage::worktree_path(repo, path).is_ok_and(|file| file.symlink_metadata().is_ok())
        })
        .map(str::to_owned)
        .collect()
}

/// The files applying `stash` would leave in conflict, by merging in
/// memory the three trees `git stash apply` merges: the stash's base, the
/// index as it is, and the stash. None when that cannot be told (an index
/// with unmerged paths, a git without `merge-tree --merge-base` on trees).
fn predicted_conflicts(repo: &Repository, stash: Oid) -> Option<Vec<String>> {
    let ours = exe::run(repo, ["write-tree"], None).ok()?;
    let ours = ours.stdout.trim();
    let hex = stash.to_hex();
    let base = format!("{hex}^1^{{tree}}");
    let theirs = format!("{hex}^{{tree}}");
    let (code, out) = exe::run_with_code(
        repo,
        [
            "merge-tree",
            "--write-tree",
            "--name-only",
            "--no-messages",
            "-z",
            "--merge-base",
            base.as_str(),
            ours,
            theirs.as_str(),
        ],
        None,
    )
    .ok()?;
    match code {
        Some(0) => Some(Vec::new()),
        // The merged tree's id comes first, then the files in conflict.
        Some(1) => Some(
            out.stdout
                .split('\0')
                .skip(1)
                .filter(|path| !path.is_empty())
                .map(str::to_owned)
                .collect(),
        ),
        _ => None,
    }
}

/// Bring a stash's changes back; `pop` also drops it from the list, and
/// `index` restores what was staged as staged. Refused, with nothing
/// changed, when it would conflict or an untracked file is in the way.
pub fn stash_apply(repo: &Repository, stash: Oid, pop: bool, index: bool) -> Result<()> {
    let at = position(repo, stash)?;
    let blocked = untracked_in_the_way(repo, stash);
    if !blocked.is_empty() {
        return Err(refused(say!(
            "stash-untracked-in-the-way",
            files = blocked.join(", ")
        )));
    }
    if let Some(files) = predicted_conflicts(repo, stash)
        && !files.is_empty()
    {
        return Err(refused(say!(
            "stash-would-conflict",
            files = files.join(", ")
        )));
    }
    let mut args = vec!["stash", if pop { "pop" } else { "apply" }, "--quiet"];
    if index {
        args.push("--index");
    }
    args.push(at.as_str());
    exe::run(repo, args, None)?;
    Ok(())
}

/// Remove a stash from the list. Returns its message, with which
/// [`stash_store`] puts it back.
pub fn stash_drop(repo: &Repository, stash: Oid) -> Result<String> {
    let list = stash_list(repo)?;
    let (index, entry) = list
        .iter()
        .enumerate()
        .find(|(_, entry)| entry.id == stash)
        .ok_or_else(|| invalid(say!("stash-not-in-list", stash = stash.short(10))))?;
    let at = format!("stash@{{{index}}}");
    exe::run(repo, ["stash", "drop", "--quiet", at.as_str()], None)?;
    Ok(entry.message.clone())
}

/// Put a dropped stash back in the list, as the newest. Its commit is
/// still in the object database until git collects garbage.
pub fn stash_store(repo: &Repository, stash: Oid, message: &str) -> Result<()> {
    if stash_list(repo)?.iter().any(|entry| entry.id == stash) {
        return Ok(());
    }
    if message.chars().any(char::is_control) {
        return Err(invalid(say!("stash-message-control")));
    }
    let hex = stash.to_hex();
    exe::run(
        repo,
        ["stash", "store", "--message", message, hex.as_str()],
        None,
    )
    .map_err(|_| refused(say!("stash-gone-nothing-changed", stash = stash.short(10))))?;
    Ok(())
}

/// The first half of bringing a stash back on a branch of its own, which
/// is the way for one that no longer fits the branch it came from: create
/// a branch where the stash was made and check it out, as `git stash
/// branch` does before it applies. Returns where the branch points and
/// where HEAD was. The second half is a pop there, with what was staged
/// staged again ([`stash_apply_recorded`]); the two are apart so that
/// each can be taken back as what it is.
///
/// Local changes come along as with any checkout; when they are in its
/// way the branch is not left behind.
pub fn stash_branch_start(
    repo: &Repository,
    stash: Oid,
    name: &str,
) -> Result<(Oid, branch::HeadState)> {
    branch::check_branch_name(repo, name)?;
    // It has to be in the list: that is where the pop takes it from.
    position(repo, stash)?;
    let made_on = format!("{}^1", stash.to_hex());
    let target = branch::branch_create(repo, name, Some(&made_on))?;
    match branch::checkout(repo, name, crate::Dirty::Refuse) {
        Ok(outcome) => Ok((target, outcome.before)),
        Err(err) => {
            // Half done is worse than not done: take the branch back.
            let _ = branch::branch_move(repo, name, None, Some(target));
            Err(err)
        }
    }
}

/// The commit that holds a stash's untracked files: the third parent of
/// a commit that has the shape git gives a stash made with them (a third
/// parent that has no parent of its own). None for any other commit.
pub fn untracked_part(repo: &Repository, stash: Oid) -> Option<Oid> {
    let hex = stash.to_hex();
    let out = exe::run(
        repo,
        ["rev-list", "--parents", "-n", "1", hex.as_str()],
        None,
    )
    .ok()?;
    let mut ids = out.stdout.split_whitespace().skip(1);
    let (_base, _index, third) = (ids.next()?, ids.next()?, ids.next()?);
    if ids.next().is_some() {
        return None;
    }
    let own = exe::run(repo, ["rev-list", "--parents", "-n", "1", third], None).ok()?;
    let third: Oid = third.parse().ok()?;
    (own.stdout.split_whitespace().count() == 1).then_some(third)
}

/// The files a stash brings back, as a commit's files are listed: what it
/// changes in tracked files against the commit it was made on, and the
/// untracked files it carries, each as a file that is added.
/// By path, as `git stash show --include-untracked` lists them.
pub fn stash_files(
    repo: &Repository,
    stash: Oid,
    options: &crate::DiffOptions,
) -> Result<Vec<crate::FileDiff>> {
    let base = format!("{}^1", stash.to_hex());
    let base = crate::rewrite::commit_of(repo, say!("stash-what-stash"), &base)?;
    let mut files = crate::diff_trees(
        repo,
        &crate::DiffSide::Commit(base),
        &crate::DiffSide::Commit(stash),
        options,
    )?;
    if let Some(untracked) = untracked_part(repo, stash) {
        files.extend(crate::diff_trees(
            repo,
            &crate::DiffSide::Empty,
            &crate::DiffSide::Commit(untracked),
            options,
        )?);
        files.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(files)
}

/// An entry of the index for one path, as far as undoing an application
/// needs it: for a stash nothing in conflict, so one entry a path; for
/// what [`crate::files_change`] records, every stage of a path in
/// conflict.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Staged {
    pub path: String,
    /// Its mode as git prints it.
    pub mode: String,
    pub blob: Oid,
    /// Its stage: 0, or 1 to 3 in a conflict (the common ancestor's, ours,
    /// theirs).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub stage: u8,
}

fn is_zero(stage: &u8) -> bool {
    *stage == 0
}

/// What applying a stash changed, enough to take it back and to tell
/// whether it still can be: for every path the stash touches, the file
/// and the index entry before and after.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Applied {
    pub stash: Oid,
    /// It was popped: dropped from the list once applied.
    pub popped: bool,
    /// What was staged in it was staged again.
    pub index: bool,
    /// Its message in the list, with which a popped one is put back.
    pub message: String,
    /// The files it touches, as they were before.
    pub files_before: Vec<stage::Backup>,
    /// The same files, as applying left them.
    pub files_after: Vec<stage::Backup>,
    /// The index entries of those paths before; a path without one had
    /// none.
    pub index_before: Vec<Staged>,
    /// The same, as applying left them.
    pub index_after: Vec<Staged>,
}

/// Most paths a stash may touch for its application to be recorded for
/// undo: each is read before and after.
pub const MAX_RECORDED: usize = 5000;

/// The paths applying `stash` can write: what it changes against the
/// commit it was made on, what was staged in it, and its untracked files.
fn touched(repo: &Repository, stash: Oid) -> Result<Vec<String>> {
    let hex = stash.to_hex();
    let (base, staged) = (format!("{hex}^1"), format!("{hex}^2"));
    let mut paths: Vec<String> = Vec::new();
    for other in [hex.as_str(), staged.as_str()] {
        let out = exe::run(
            repo,
            [
                "diff",
                "--name-only",
                "--no-renames",
                "-z",
                base.as_str(),
                other,
                "--",
            ],
            None,
        )?;
        paths.extend(
            out.stdout
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_owned),
        );
    }
    if let Some(untracked) = untracked_part(repo, stash) {
        let out = exe::run(
            repo,
            [
                "ls-tree",
                "-r",
                "--name-only",
                "-z",
                untracked.to_hex().as_str(),
            ],
            None,
        )?;
        paths.extend(
            out.stdout
                .split('\0')
                .filter(|path| !path.is_empty())
                .map(str::to_owned),
        );
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

/// The index entries of `paths`, for those that have one.
fn staged_entries(repo: &Repository, paths: &[String]) -> Result<Vec<Staged>> {
    let mut entries = Vec::new();
    // In pieces: a command line has an end.
    for some in paths.chunks(200) {
        let mut args = vec!["--literal-pathspecs", "ls-files", "--stage", "-z", "--"];
        args.extend(some.iter().map(String::as_str));
        let out = exe::run(repo, args, None)?;
        for record in out.stdout.split('\0').filter(|record| !record.is_empty()) {
            let Some((about, path)) = record.split_once('\t') else {
                continue;
            };
            let mut about = about.split(' ');
            let (Some(mode), Some(blob), Some(stage)) = (about.next(), about.next(), about.next())
            else {
                continue;
            };
            if stage != "0" {
                return Err(refused(say!("stash-path-in-conflict", path)));
            }
            let Ok(blob) = blob.parse() else { continue };
            entries.push(Staged {
                path: path.to_owned(),
                mode: mode.to_owned(),
                blob,
                stage: 0,
            });
        }
    }
    Ok(entries)
}

/// [`stash_apply`], keeping what it takes to undo it. Nothing is kept,
/// and the stash is applied all the same, when it touches more files
/// than are read one by one for that ([`MAX_RECORDED`]).
pub fn stash_apply_recorded(
    repo: &Repository,
    stash: Oid,
    pop: bool,
    index: bool,
) -> Result<Option<Applied>> {
    let message = stash_list(repo)?
        .into_iter()
        .find(|entry| entry.id == stash)
        .map(|entry| entry.message)
        .ok_or_else(|| invalid(say!("stash-not-in-list", stash = stash.short(10))))?;
    let paths = touched(repo, stash)?;
    if paths.len() > MAX_RECORDED {
        stash_apply(repo, stash, pop, index)?;
        return Ok(None);
    }
    let index_before = staged_entries(repo, &paths)?;
    let files_before = stage::backup_paths(repo, &paths)?;
    stash_apply(repo, stash, pop, index)?;
    Ok(Some(Applied {
        stash,
        popped: pop,
        index,
        message,
        files_before,
        files_after: stage::backup_paths(repo, &paths)?,
        index_before,
        index_after: staged_entries(repo, &paths)?,
    }))
}

/// Take an applied stash out again: every file and index entry it touched
/// goes back to what it was, and a stash that was popped is back in the
/// list. Only while each of them is still as applying left it; otherwise
/// nothing is changed, and the error names what was changed since.
pub fn stash_unapply(repo: &Repository, applied: &Applied) -> Result<()> {
    crate::rewrite::ensure_idle(repo, say!("stash-what-undo"))?;
    let paths: Vec<String> = applied
        .files_after
        .iter()
        .map(|backup| backup.path.clone())
        .collect();
    let since = |path: &str| refused(say!("stash-changed-since-applied", path));
    let now = stage::backup_paths(repo, &paths)?;
    for (now, then) in now.iter().zip(&applied.files_after) {
        if now.blob != then.blob {
            return Err(since(&then.path));
        }
    }
    let staged_now = staged_entries(repo, &paths)?;
    if staged_now != applied.index_after {
        let moved = paths
            .iter()
            .find(|path| {
                let entry = |entries: &[Staged]| {
                    entries
                        .iter()
                        .find(|entry| &entry.path == *path)
                        .map(|entry| (entry.mode.clone(), entry.blob))
                };
                entry(&staged_now) != entry(&applied.index_after)
            })
            .map_or(say!("stash-the-index"), String::as_str);
        return Err(since(moved));
    }
    for before in &applied.files_before {
        stage::restore_backup(repo, before)?;
    }
    // The index: each path back to the entry it had, or to none.
    let mut records = String::new();
    for path in &paths {
        match applied
            .index_before
            .iter()
            .find(|entry| &entry.path == path)
        {
            Some(entry) => {
                records.push_str(&format!(
                    "{} {} 0\t{}\0",
                    entry.mode,
                    entry.blob.to_hex(),
                    entry.path
                ));
            }
            None => records.push_str(&format!("0 {}\t{path}\0", "0".repeat(40))),
        }
    }
    exe::run(
        repo,
        ["update-index", "-z", "--index-info"],
        Some(records.as_bytes()),
    )?;
    if applied.popped {
        stash_store(repo, applied.stash, &applied.message)?;
    }
    Ok(())
}

/// Most bytes a stash's new message has.
const MAX_STASH_MESSAGE: usize = 4096;

/// The message a stash's commit says for `message`, made on the branch the
/// old one says it was made on: git's "On <branch>: " before it, as `git
/// stash push --message` writes it.
fn stash_subject(old: &str, message: &str) -> String {
    let first = old.lines().next().unwrap_or("");
    let branch = first
        .strip_prefix("On ")
        .or_else(|| first.strip_prefix("WIP on "))
        .and_then(|rest| rest.split_once(": "))
        .map(|(branch, _)| branch);
    match branch {
        Some(branch) => format!("On {branch}: {message}"),
        None => message.to_owned(),
    }
}

/// Give the stash `stash` another message, in its place in the list.
/// Returns the stash it is now (a new commit) and what the list said of
/// it before; [`stash_replace`] takes it back with those.
pub fn stash_reword(repo: &Repository, stash: Oid, message: &str) -> Result<(Oid, String)> {
    let message = message.trim();
    if message.is_empty() {
        return Err(invalid(say!("stash-message-empty")));
    }
    if message.len() > MAX_STASH_MESSAGE || message.chars().any(char::is_control) {
        return Err(invalid(say!("stash-message-too-long")));
    }
    let list = stash_list(repo)?;
    let entry = list
        .iter()
        .find(|entry| entry.id == stash)
        .ok_or_else(|| invalid(say!("stash-not-in-list", stash = stash.short(10))))?;
    let old = entry.message.clone();
    let hex = stash.to_hex();
    // Who made it and when, kept as they were.
    let said = exe::run(
        repo,
        [
            "log",
            "-1",
            "--date=raw",
            "--format=%an%x00%ae%x00%ad%x00%cn%x00%ce%x00%cd%x00%B",
            hex.as_str(),
        ],
        None,
    )?;
    let fields: Vec<&str> = said.stdout.splitn(7, '\0').collect();
    let [name, email, date, c_name, c_email, c_date, body] = fields.as_slice() else {
        return Err(refused(say!("stash-unreadable", stash = stash.short(10))));
    };
    let subject = stash_subject(body, message);
    let tree = format!("{hex}^{{tree}}");
    let parents = exe::run(repo, ["rev-parse", format!("{hex}^@").as_str()], None)?;
    let mut args: Vec<String> = vec![
        "commit-tree".to_owned(),
        tree,
        "-m".to_owned(),
        subject.clone(),
    ];
    for parent in parents.stdout.lines().filter(|line| !line.is_empty()) {
        args.push("-p".to_owned());
        args.push(parent.to_owned());
    }
    let made = exe::run_with_env(
        repo,
        &args,
        &[
            ("GIT_AUTHOR_NAME", name),
            ("GIT_AUTHOR_EMAIL", email),
            ("GIT_AUTHOR_DATE", date),
            ("GIT_COMMITTER_NAME", c_name),
            ("GIT_COMMITTER_EMAIL", c_email),
            ("GIT_COMMITTER_DATE", c_date.trim()),
        ],
    )?;
    let new: Oid = made
        .stdout
        .trim()
        .parse()
        .map_err(|_| refused(say!("stash-no-new-commit")))?;
    stash_replace(repo, stash, new, &subject)?;
    Ok((new, old))
}

/// Put the stash `to` in the place of `from` in the stash list, said in
/// the list as `message`: the stashes newer than it are taken off and put
/// back on in their order. What could not be put back is said, and the
/// commits stay in the object database.
pub fn stash_replace(repo: &Repository, from: Oid, to: Oid, message: &str) -> Result<()> {
    if message.chars().any(char::is_control) {
        return Err(invalid(say!("stash-message-control")));
    }
    let list = stash_list(repo)?;
    let at = list
        .iter()
        .position(|entry| entry.id == from)
        .ok_or_else(|| invalid(say!("stash-not-in-list", stash = from.short(10))))?;
    // Newest first: the ones above it, and it.
    let taken: Vec<StashEntry> = list.into_iter().take(at.saturating_add(1)).collect();
    for _ in &taken {
        exe::run(repo, ["stash", "drop", "--quiet", "stash@{0}"], None)?;
    }
    // Oldest first back on: in its place the new one, then those above.
    let mut back: Vec<(Oid, String)> = vec![(to, message.to_owned())];
    back.extend(
        taken
            .iter()
            .rev()
            .skip(1)
            .map(|entry| (entry.id, entry.message.clone())),
    );
    for (index, (id, said)) in back.iter().enumerate() {
        if let Err(err) = stash_store_anew(repo, *id, said) {
            // The old one in its place again where the new one would not
            // go, and the rest on top as far as they go.
            let rest = back.iter().skip(index.saturating_add(1));
            if index == 0 {
                let _ = stash_store_anew(
                    repo,
                    from,
                    &taken.last().map(|e| e.message.clone()).unwrap_or_default(),
                );
            }
            for (id, said) in rest {
                let _ = stash_store_anew(repo, *id, said);
            }
            return Err(err);
        }
    }
    Ok(())
}

/// `git stash store`: `id` on top of the list, as `message`.
fn stash_store_anew(repo: &Repository, id: Oid, message: &str) -> Result<()> {
    let hex = id.to_hex();
    exe::run(
        repo,
        ["stash", "store", "--message", message, hex.as_str()],
        None,
    )
    .map_err(|_| refused(say!("stash-gone", stash = id.short(10))))?;
    Ok(())
}
