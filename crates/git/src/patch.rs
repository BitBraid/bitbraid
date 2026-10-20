//! Patches: commits or uncommitted work written out as patch files, a
//! patch file checked and applied to the files, and a mailbox of patches
//! applied as commits.
//!
//! What git writes goes to a directory of ours beside where it is wanted
//! and is put in place when all of it is there, so an export arrives whole
//! or not at all, and a file that exists is only replaced when that was
//! asked for. A patch to apply is read once, with a bound on its size, and
//! git is given those bytes: what was checked is what is applied, and what
//! is kept for undo.
//!
//! Sources: git-format-patch(1), git-diff(1) (`--binary`, `--output`),
//! git-apply(1) (`--check`, `--numstat`, `--index`, `--3way`,
//! `--reverse`), git-am(1), git-hash-object(1), git-rev-list(1).

use std::{
    ffi::OsString,
    fs,
    io::Read as _,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use serde::Serialize;

use crate::{Error, Moved, Oid, Repository, Result, branch, exe, git_err, rewrite, sequence};

/// The largest patch that is read to be checked or applied, in bytes.
pub const MAX_PATCH: u64 = 32 * 1024 * 1024;
/// The largest mailbox that is handed to `git am`, in bytes.
pub const MAX_MAILBOX: u64 = 256 * 1024 * 1024;
/// The most commits one export writes.
pub const MAX_EXPORT: usize = 500;
/// The most files of a patch that are listed.
const MAX_LISTED: usize = 5000;
/// The longest path that is taken.
const LONGEST: usize = 4096;

/// What an export takes its patches from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// These commits, each by itself, in the order given.
    Commits(Vec<String>),
    /// The commits `to` has and `from` has not, oldest first.
    Range {
        /// Where the range begins; its commit is not part of it.
        from: String,
        /// Where it ends.
        to: String,
    },
    /// What is staged: the last commit against the index.
    Staged,
    /// What is changed and not staged: the index against the files.
    Worktree,
}

/// Where an export writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Destination {
    /// One file per commit, in this directory.
    Dir(String),
    /// One file: a mailbox of the commits, or the diff of uncommitted work.
    File(String),
}

/// What an export wrote.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Exported {
    /// The files written, in order.
    pub files: Vec<PathBuf>,
    /// How many patches they hold.
    pub patches: usize,
    /// Commits that were left out: a merge commit has no patch.
    pub skipped: Vec<Oid>,
}

/// One file a patch touches.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Touched {
    /// Its path, as the patch has it.
    pub path: String,
    /// Lines added; absent for a binary change.
    pub added: Option<u64>,
    /// Lines deleted; absent for a binary change.
    pub deleted: Option<u64>,
    /// The change is binary.
    pub binary: bool,
}

/// What git says about a patch before it is applied.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Checked {
    /// It would apply.
    pub applies: bool,
    /// Why not, in git's words.
    pub reason: Option<String>,
    /// The files it touches; the first of them when there are very many.
    pub files: Vec<Touched>,
    /// How many files it touches.
    pub total: usize,
}

/// How a patch is applied.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// To the index as well as to the files.
    pub index: bool,
    /// Merge where it does not apply as it is, which takes the index along
    /// and may leave files in conflict.
    pub three_way: bool,
    /// The other way round: take the change out.
    pub reverse: bool,
}

/// What applying a patch did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Applied {
    /// The patch, kept as a blob in the object database.
    pub patch: Oid,
    /// The index was changed too.
    pub index: bool,
    /// It was applied the other way round.
    pub reverse: bool,
    /// The files it touches; the first of them when there are very many.
    pub files: Vec<Touched>,
    /// How many files it touches.
    pub total: usize,
    /// Files a three-way merge left in conflict.
    pub conflicts: Vec<String>,
    /// What a three-way merge did to the files and their index entries,
    /// when it could be read: what undo takes back where the patch the
    /// other way round would not (it left conflicts).
    #[serde(skip)]
    pub files_change: Option<crate::files_change::FilesChange>,
}

/// What applying a mailbox did.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Mailed {
    /// Where HEAD was and is, and what git said when it stopped half-way.
    pub moved: Moved,
    /// How many commits were made.
    pub commits: usize,
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

fn io(context: &'static str) -> impl FnOnce(std::io::Error) -> Error {
    move |err| Error::Git {
        context,
        source: Box::new(err),
    }
}

/// A path as it was typed: something, of a length a path can have, without
/// control characters. A relative one is taken from the worktree.
fn typed_path(repo: &Repository, what: &'static str, text: &str) -> Result<PathBuf> {
    if text.is_empty() {
        return Err(invalid(what, say!("patch-empty")));
    }
    if text.len() > LONGEST {
        return Err(invalid(what, say!("patch-too-long")));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("patch-control", text = format!("{text:?}")),
        ));
    }
    let path = Path::new(text);
    Ok(if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.workdir().unwrap_or_else(|| repo.git_dir()).join(path)
    })
}

/// The file a patch is read from: a plain file of at most `most` bytes.
fn patch_file(repo: &Repository, text: &str, most: u64) -> Result<PathBuf> {
    let path = typed_path(repo, say!("patch-what-patch-file"), text)?;
    let shown = path.display().to_string();
    let shown = shown.as_str();
    let meta = fs::metadata(&path).map_err(|_| {
        refused(
            say!("patch-what-patch"),
            say!("patch-unreadable", path = shown),
        )
    })?;
    if !meta.is_file() {
        return Err(refused(
            say!("patch-what-patch"),
            say!("patch-not-a-file", path = shown),
        ));
    }
    if meta.len() > most {
        return Err(refused(
            say!("patch-what-patch"),
            say!(
                "patch-larger-than-mib",
                path = shown,
                mib = most / (1024 * 1024)
            ),
        ));
    }
    Ok(path)
}

/// The bytes of a patch file, read once.
fn read_patch(repo: &Repository, text: &str) -> Result<Vec<u8>> {
    let path = patch_file(repo, text, MAX_PATCH)?;
    let file = fs::File::open(&path).map_err(io(say!("patch-opening")))?;
    let mut bytes = Vec::new();
    // One more than is taken, to see a file that grew since it was looked at.
    file.take(MAX_PATCH.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(io(say!("patch-reading")))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_PATCH {
        return Err(refused(
            say!("patch-what-patch"),
            say!("patch-larger-than-taken", path = path.display().to_string()),
        ));
    }
    Ok(bytes)
}

/// What git wrote to standard error, cut to what a person reads: without
/// the hints, and not more than a few lines.
fn said(stderr: &str) -> String {
    const LINES: usize = 6;
    let lines: Vec<&str> = stderr
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("hint:"))
        .map(|line| {
            line.strip_prefix("error: ")
                .or_else(|| line.strip_prefix("fatal: "))
                .unwrap_or(line)
        })
        .collect();
    if lines.is_empty() {
        return say!("patch-no-reason").to_owned();
    }
    let mut text = lines
        .iter()
        .take(LINES)
        .copied()
        .collect::<Vec<_>>()
        .join("; ");
    if lines.len() > LINES {
        text.push_str("; …");
    }
    text
}

/// The records of `git apply --numstat -z`: added, deleted and the path,
/// with `-` for the counts of a binary change. Returns the first files and
/// how many there are.
fn parse_numstat(text: &str) -> (Vec<Touched>, usize) {
    let mut files = Vec::new();
    let mut total = 0usize;
    for record in text.split('\0').filter(|record| !record.is_empty()) {
        let mut parts = record.splitn(3, '\t');
        let (Some(added), Some(deleted), Some(path)) = (parts.next(), parts.next(), parts.next())
        else {
            continue;
        };
        total = total.saturating_add(1);
        if files.len() < MAX_LISTED {
            files.push(Touched {
                path: path.to_owned(),
                added: added.parse().ok(),
                deleted: deleted.parse().ok(),
                binary: added == "-" && deleted == "-",
            });
        }
    }
    (files, total)
}

/// The options of `git apply` that say how.
fn how(options: Options) -> Vec<&'static str> {
    let mut args = Vec::new();
    if options.index {
        args.push("--index");
    }
    if options.three_way {
        args.push("--3way");
    }
    if options.reverse {
        args.push("--reverse");
    }
    args
}

/// Run `git apply` on the bytes of a patch.
fn run_apply(
    repo: &Repository,
    first: &[&'static str],
    options: Options,
    bytes: &[u8],
) -> Result<(Option<i32>, exe::Output)> {
    let mut args = vec!["apply"];
    args.extend_from_slice(first);
    args.extend(how(options));
    args.push("-");
    exe::run_with_code(repo, args, Some(bytes))
}

/// What a patch touches and whether it would apply.
fn examine(repo: &Repository, bytes: &[u8], options: Options) -> Result<Checked> {
    // What it touches first: something git cannot read as a patch touches
    // nothing, and that is the answer.
    let (code, out) = run_apply(repo, &["--numstat", "-z"], Options::default(), bytes)?;
    if code != Some(0) {
        return Ok(Checked {
            applies: false,
            reason: Some(said(&out.stderr)),
            files: Vec::new(),
            total: 0,
        });
    }
    let (files, total) = parse_numstat(&out.stdout);
    let reason = why_not(repo, bytes, options)?;
    Ok(Checked {
        applies: reason.is_none(),
        reason,
        files,
        total,
    })
}

/// Why a patch would not apply the way `options` say; nothing when it
/// would.
fn why_not(repo: &Repository, bytes: &[u8], options: Options) -> Result<Option<String>> {
    if options.index || options.three_way {
        // With the index in play git compares each file with what the
        // index remembers of it, and takes a file that was only touched
        // for one that was changed. Have it look first, as `git status`
        // does; that some files are changed is no failure here.
        let _ = exe::run_with_code(repo, ["update-index", "-q", "--refresh"], None)?;
    }
    let (code, out) = run_apply(repo, &["--check"], options, bytes)?;
    Ok((code != Some(0)).then(|| said(&out.stderr)))
}

/// Whether the patch in `file` would apply the way `options` say, and the
/// files it touches. A patch that does not apply, or is none, is an answer
/// and not an error.
pub fn check(repo: &Repository, file: &str, options: Options) -> Result<Checked> {
    let bytes = read_patch(repo, file)?;
    examine(repo, &bytes, options)
}

/// Paths that are unmerged in the index.
fn unmerged(repo: &Repository) -> Result<Vec<String>> {
    Ok(sequence::conflicts(repo)?
        .into_iter()
        .map(|conflict| conflict.path)
        .collect())
}

/// Every path a patch writes: the paths `--numstat` names, and for a file
/// it renames the old path too, which only the patch the other way round
/// names.
fn patch_paths(repo: &Repository, bytes: &[u8]) -> Result<Vec<String>> {
    let mut paths = Vec::new();
    for reverse in [false, true] {
        let options = Options {
            reverse,
            ..Options::default()
        };
        let (code, out) = run_apply(repo, &["--numstat", "-z"], options, bytes)?;
        if code != Some(0) {
            return Err(refused(say!("patch-what-apply"), said(&out.stderr)));
        }
        for record in out.stdout.split('\0').filter(|record| !record.is_empty()) {
            if let Some(path) = record
                .splitn(3, '\t')
                .nth(2)
                .filter(|path| !path.is_empty())
            {
                paths.push(path.to_owned());
            }
        }
    }
    Ok(paths)
}

/// Apply the patch in `file`. Refused, with nothing changed, when git says
/// it does not apply. With `three_way` a patch that only applies by merging
/// is applied too, and the files that merge left in conflict are named;
/// what it did to its files and their index entries is read before and
/// after, for undo ([`Applied::files_change`]).
pub fn apply(repo: &Repository, file: &str, options: Options) -> Result<Applied> {
    let bytes = read_patch(repo, file)?;
    let checked = examine(repo, &bytes, options)?;
    if let Some(reason) = checked.reason {
        return Err(refused(
            say!("patch-what-apply"),
            say!("patch-does-not-apply", reason),
        ));
    }
    let before = if options.three_way {
        unmerged(repo)?
    } else {
        Vec::new()
    };
    // Kept first: an applied patch that could not be kept could not be
    // taken back either.
    let patch = store(repo, &bytes)?;
    let run = || run_apply(repo, &[], options, &bytes);
    let paths = options
        .three_way
        .then(|| patch_paths(repo, &bytes).ok())
        .flatten();
    let ((code, out), files_change) = match paths {
        Some(paths) => crate::files_change::recorded(repo, paths, run)?,
        None => (run()?, None),
    };
    let conflicts: Vec<String> = if options.three_way {
        unmerged(repo)?
            .into_iter()
            .filter(|path| !before.contains(path))
            .collect()
    } else {
        Vec::new()
    };
    // A merge that left conflicts ends with 1 and has applied the rest.
    let done = code == Some(0) || (code == Some(1) && !conflicts.is_empty());
    if !done {
        return Err(refused(
            say!("patch-what-apply"),
            say!("patch-not-applied", said = said(&out.stderr)),
        ));
    }
    Ok(Applied {
        patch,
        index: options.index || options.three_way,
        reverse: options.reverse,
        files: checked.files,
        total: checked.total,
        conflicts,
        files_change,
    })
}

/// Keep the bytes of a patch as a blob. They go in as they are: no filter
/// of the repository's applies to what comes in on standard input.
fn store(repo: &Repository, bytes: &[u8]) -> Result<Oid> {
    let out = exe::run(repo, ["hash-object", "-w", "--stdin"], Some(bytes))?;
    out.stdout.trim().parse()
}

/// The bytes of a patch that was kept. The id comes from the journal, a
/// file anybody can write: it has to name a blob of a size a patch has.
fn stored(repo: &Repository, patch: Oid, what: &'static str) -> Result<Vec<u8>> {
    let gone = || refused(what, say!("patch-gone"));
    if patch.as_gix().is_null() {
        return Err(gone());
    }
    let header = repo
        .gix()
        .try_find_header(*patch.as_gix())
        .map_err(git_err(say!("patch-looking-for-kept")))?
        .ok_or_else(gone)?;
    if header.kind() != gix::object::Kind::Blob || header.size() > MAX_PATCH {
        return Err(refused(what, say!("patch-not-a-patch")));
    }
    let object = repo
        .gix()
        .find_object(*patch.as_gix())
        .map_err(git_err(say!("patch-reading-kept")))?;
    Ok(object.detach().data)
}

/// Apply a patch that was kept when it was first applied, for undo
/// (`reverse` turned round) and redo. Refused, with nothing changed, when
/// it does not apply that way any more.
pub fn apply_recorded(
    repo: &Repository,
    patch: Oid,
    index: bool,
    reverse: bool,
    what: &'static str,
) -> Result<()> {
    let bytes = stored(repo, patch, what)?;
    let options = Options {
        index,
        three_way: false,
        reverse,
    };
    if let Some(reason) = why_not(repo, &bytes, options)? {
        return Err(refused(what, say!("patch-no-longer-applies", reason)));
    }
    let (code, out) = run_apply(repo, &[], options, &bytes)?;
    if code != Some(0) {
        return Err(refused(
            what,
            say!("patch-not-applied", said = said(&out.stderr)),
        ));
    }
    Ok(())
}

/// How many commits `after` has that `before` has not.
fn commits_between(repo: &Repository, before: Option<Oid>, after: Option<Oid>) -> usize {
    let Some(after) = after.filter(|after| Some(*after) != before) else {
        return 0;
    };
    let range = match before {
        Some(before) => format!("{}..{}", before.to_hex(), after.to_hex()),
        None => after.to_hex(),
    };
    exe::run(repo, ["rev-list", "--count", range.as_str()], None)
        .ok()
        .and_then(|out| out.stdout.trim().parse().ok())
        .unwrap_or(0)
}

/// Apply the patches of a mailbox as commits on the current branch (`git
/// am`). `three_way` says whether a patch that does not apply as it is may
/// be merged; absent, git's own setting decides. A patch that conflicts or
/// does not apply stops it there, in progress, for [`crate::sequence`] to
/// continue, skip or abort.
pub fn mailbox(repo: &Repository, file: &str, three_way: Option<bool>) -> Result<Mailed> {
    mailbox_with(repo, file, three_way, false)
}

/// [`mailbox`], and with `no_verify` without the two hooks that can
/// refuse a patch of it: `applypatch-msg` and `pre-applypatch`
/// (`git am --no-verify`).
pub fn mailbox_with(
    repo: &Repository,
    file: &str,
    three_way: Option<bool>,
    no_verify: bool,
) -> Result<Mailed> {
    let what = say!("patch-what-apply-mailbox");
    // Git reads this one itself. It takes a directory for a mailbox of
    // another kind and finds nothing in it, without a word; a file it is.
    let path = patch_file(repo, file, MAX_MAILBOX)?;
    rewrite::ensure_idle(repo, what)?;
    // Git refuses a mailbox while anything is staged, and leaves its
    // directory of state behind when it does.
    if exe::run(repo, ["diff", "--cached", "--quiet"], None).is_err() {
        return Err(refused(what, say!("patch-staged-changes")));
    }
    let before = branch::head_state(repo)?;
    let mut args: Vec<OsString> = vec!["am".into()];
    match three_way {
        Some(true) => args.push("--3way".into()),
        Some(false) => args.push("--no-3way".into()),
        None => {}
    }
    if no_verify {
        args.push("--no-verify".into());
    }
    args.push("--".into());
    args.push(path.into_os_string());
    let stopped = match sequence::run_mailbox(repo, &args) {
        Ok(_) => None,
        Err(err) => {
            // A hook that refused a patch is said to be that, and that the
            // mailbox is still there to go on with.
            let words = Some(sequence::why_stopped(say!("patch-noun-mailbox"), &err))
                .filter(|words| !words.is_empty())
                .unwrap_or_else(|| say!("patch-no-reason-given").to_owned());
            let stopped = sequence::in_progress(repo)?
                .is_some_and(|operation| operation.kind == sequence::Kind::Mailbox);
            if !stopped {
                return Err(refused(what, say!("patch-not-taken", words)));
            }
            // It gave up before the first patch: there is nothing to go on
            // with, only its state to clear away.
            let state = repo.git_dir().join("rebase-apply");
            if state.join("dirtyindex").symlink_metadata().is_ok() {
                let _ = exe::run(repo, ["am", "--abort"], None);
                return Err(refused(what, words));
            }
            Some(words)
        }
    };
    let moved = rewrite::moved(repo, before, Vec::new(), stopped)?;
    let commits = commits_between(repo, moved.before.commit, moved.after.commit);
    Ok(Mailed { moved, commits })
}

/// What an export is to write, once it is known that it can.
enum Plan {
    /// Commits one by one, and those of the commits named that are merges
    /// and so have no patch.
    Each {
        ids: Vec<Oid>,
        merges: Vec<Oid>,
    },
    Range {
        from: Oid,
        to: Oid,
        count: usize,
    },
    Diff {
        cached: bool,
    },
}

fn range_of(from: Oid, to: Oid) -> String {
    format!("{}..{}", from.to_hex(), to.to_hex())
}

fn plan(repo: &Repository, source: &Source) -> Result<Plan> {
    let too_many = |n: usize| {
        refused(
            say!("patch-what-export"),
            say!("patch-too-many", count = n, most = MAX_EXPORT),
        )
    };
    match source {
        Source::Commits(revs) => {
            if revs.is_empty() {
                return Err(invalid(
                    say!("patch-what-commits"),
                    say!("patch-no-commits"),
                ));
            }
            if revs.len() > MAX_EXPORT {
                return Err(too_many(revs.len()));
            }
            let ids = revs
                .iter()
                .map(|rev| rewrite::commit_of(repo, say!("patch-what-commit"), rev))
                .collect::<Result<Vec<_>>>()?;
            // Asked for one commit that is a merge, git writes the newest
            // commit before it that is none. Merges are told first and
            // left out.
            let mut args = vec![
                "rev-list".to_owned(),
                "--no-walk".to_owned(),
                "--merges".to_owned(),
            ];
            args.extend(ids.iter().map(Oid::to_hex));
            let out = exe::run(repo, args, None)?;
            let found: Vec<Oid> = out
                .stdout
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .collect();
            let (merges, ids): (Vec<Oid>, Vec<Oid>) =
                ids.into_iter().partition(|id| found.contains(id));
            if ids.is_empty() {
                return Err(nothing_written());
            }
            Ok(Plan::Each { ids, merges })
        }
        Source::Range { from, to } => {
            let from = rewrite::commit_of(repo, say!("patch-what-from"), from)?;
            let to = rewrite::commit_of(repo, say!("patch-what-to"), to)?;
            let range = range_of(from, to);
            // A merge commit has no patch; git leaves it out.
            let out = exe::run(
                repo,
                ["rev-list", "--count", "--no-merges", range.as_str()],
                None,
            )?;
            let count: usize = out.stdout.trim().parse().unwrap_or(usize::MAX);
            if count == 0 {
                return Err(refused(
                    say!("patch-what-export"),
                    say!("patch-no-commit-between"),
                ));
            }
            if count > MAX_EXPORT {
                return Err(too_many(count));
            }
            Ok(Plan::Range { from, to, count })
        }
        Source::Staged => Ok(Plan::Diff { cached: true }),
        Source::Worktree => Ok(Plan::Diff { cached: false }),
    }
}

/// A directory of ours beside the destination, for git to write into. It
/// is gone again when the export is over, however it ends.
struct Workroom(PathBuf);

impl Workroom {
    fn beside(parent: &Path) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|since| since.subsec_nanos())
            .unwrap_or(0);
        let name = format!(".bitbraid-export-{}-{n}-{nanos}", std::process::id());
        let path = parent.join(name);
        fs::create_dir(&path).map_err(io(say!("patch-making-room")))?;
        Ok(Self(path))
    }
}

impl Drop for Workroom {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// The files in a directory, by name. Git numbers the patches it writes,
/// so that is their order.
fn listing(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map_err(io(say!("patch-listing")))?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect();
    files.sort();
    Ok(files)
}

/// An option with a path for its value.
fn with_path(option: &str, path: &Path) -> OsString {
    let mut arg = OsString::from(option);
    arg.push(path);
    arg
}

fn hex(id: Oid) -> OsString {
    id.to_hex().into()
}

/// The merge commits of a range, which have no patch.
fn merges_in(repo: &Repository, range: &str) -> Vec<Oid> {
    exe::run(repo, ["rev-list", "--merges", range], None)
        .map(|out| {
            out.stdout
                .lines()
                .filter_map(|line| line.trim().parse().ok())
                .take(MAX_EXPORT)
                .collect()
        })
        .unwrap_or_default()
}

/// Run git for an export, stoppable: what it writes goes to a directory
/// of ours, so that stopping it half-way leaves nothing where it counts.
fn run_stoppable(repo: &Repository, args: Vec<OsString>, cancel: &Arc<AtomicBool>) -> Result<()> {
    exe::run_streaming(repo, args, cancel, &mut |_| {})?;
    Ok(())
}

/// Have git write the commits of `plan` into `room`, one file each. Returns
/// the files in order and the commits that have none.
fn write_each(
    repo: &Repository,
    plan: &Plan,
    room: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(Vec<PathBuf>, Vec<Oid>)> {
    let into = |more: Vec<OsString>| -> Result<()> {
        let mut args: Vec<OsString> = vec![
            "format-patch".into(),
            "--quiet".into(),
            "-o".into(),
            room.into(),
        ];
        args.extend(more);
        run_stoppable(repo, args, cancel)
    };
    match plan {
        Plan::Range { from, to, .. } => {
            let range = range_of(*from, *to);
            into(vec![range.as_str().into()])?;
            Ok((listing(room)?, merges_in(repo, &range)))
        }
        Plan::Each { ids, merges } => {
            let (mut files, mut skipped) = (Vec::new(), merges.clone());
            for id in ids {
                // Numbered on from the last, so that the names sort in the
                // order the commits were given.
                let number = files.len().saturating_add(1);
                into(vec![
                    format!("--start-number={number}").into(),
                    "--no-cover-letter".into(),
                    "-1".into(),
                    hex(*id),
                ])?;
                let now = listing(room)?;
                if now.len() == files.len() {
                    skipped.push(*id);
                }
                files = now;
            }
            Ok((files, skipped))
        }
        Plan::Diff { .. } => Ok((Vec::new(), Vec::new())),
    }
}

fn length(file: &Path) -> u64 {
    file.metadata().map(|meta| meta.len()).unwrap_or(0)
}

/// Have git write the commits of `plan` as one mailbox in `room`. Returns
/// the file, how many patches it holds and the commits that have none.
fn write_mailbox(
    repo: &Repository,
    plan: &Plan,
    room: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<(PathBuf, usize, Vec<Oid>)> {
    let out = room.join("out");
    match plan {
        Plan::Range { from, to, count } => {
            let range = range_of(*from, *to);
            let args: Vec<OsString> = vec![
                "format-patch".into(),
                with_path("--output=", &out),
                range.as_str().into(),
            ];
            run_stoppable(repo, args, cancel)?;
            Ok((out, *count, merges_in(repo, &range)))
        }
        Plan::Each { ids, merges } => {
            // Each commit as git writes it by itself, one after the other.
            let one = room.join("one");
            let mut all = fs::File::create(&out).map_err(io(say!("patch-writing-mailbox")))?;
            let (mut patches, mut skipped) = (0usize, merges.clone());
            for id in ids {
                let _ = fs::remove_file(&one);
                let args: Vec<OsString> = vec![
                    "format-patch".into(),
                    with_path("--output=", &one),
                    "--no-cover-letter".into(),
                    "-1".into(),
                    hex(*id),
                ];
                run_stoppable(repo, args, cancel)?;
                if length(&one) == 0 {
                    skipped.push(*id);
                    continue;
                }
                let mut part = fs::File::open(&one).map_err(io(say!("patch-reading-patch")))?;
                std::io::copy(&mut part, &mut all).map_err(io(say!("patch-writing-mailbox")))?;
                patches = patches.saturating_add(1);
            }
            Ok((out, patches, skipped))
        }
        Plan::Diff { .. } => Ok((out, 0, Vec::new())),
    }
}

/// Have git write uncommitted work as one diff in `room`: with binary
/// changes in full, and in the form `git apply` takes whatever the
/// configuration says about prefixes, colours, external diff programs and
/// text conversion.
fn write_diff(
    repo: &Repository,
    cached: bool,
    room: &Path,
    cancel: &Arc<AtomicBool>,
) -> Result<PathBuf> {
    if !unmerged(repo)?.is_empty() {
        return Err(refused(say!("patch-what-export"), say!("patch-unresolved")));
    }
    let out = room.join("out");
    let mut args: Vec<OsString> = vec!["diff".into()];
    if cached {
        args.push("--cached".into());
    }
    args.extend(
        [
            "--binary",
            "--no-color",
            "--no-ext-diff",
            "--no-textconv",
            "--submodule=short",
            "--src-prefix=a/",
            "--dst-prefix=b/",
        ]
        .map(OsString::from),
    );
    args.push(with_path("--output=", &out));
    run_stoppable(repo, args, cancel)?;
    if length(&out) == 0 {
        return Err(refused(
            say!("patch-what-export"),
            if cached {
                say!("patch-nothing-staged")
            } else {
                say!("patch-nothing-changed")
            },
        ));
    }
    Ok(out)
}

/// Put a file git wrote where it is wanted. Without `overwrite` it is made
/// new or not at all, so a file that appeared meanwhile is not written
/// over.
fn place(from: &Path, to: &Path, overwrite: bool) -> Result<()> {
    if overwrite {
        return fs::rename(from, to).map_err(io(say!("patch-placing")));
    }
    let mut target = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(to)
        .map_err(|err| {
            if err.kind() == std::io::ErrorKind::AlreadyExists {
                taken(to)
            } else {
                io(say!("patch-placing"))(err)
            }
        })?;
    let copied =
        fs::File::open(from).and_then(|mut source| std::io::copy(&mut source, &mut target));
    if let Err(err) = copied {
        drop(target);
        let _ = fs::remove_file(to);
        return Err(io(say!("patch-placing"))(err));
    }
    Ok(())
}

fn taken(file: &Path) -> Error {
    refused(
        say!("patch-what-export"),
        say!("patch-exists", path = file.display().to_string()),
    )
}

/// Git's own directory is no place for an export: a file there can be one
/// the repository depends on.
fn outside_git_dir(repo: &Repository, place: &Path) -> Result<()> {
    let Ok(place) = bitbraid_core::paths::canonical(place) else {
        return Ok(());
    };
    for dir in [repo.git_dir(), repo.gix().common_dir()] {
        if bitbraid_core::paths::canonical(dir).is_ok_and(|dir| place.starts_with(dir)) {
            return Err(refused(say!("patch-what-export"), say!("patch-in-git-dir")));
        }
    }
    Ok(())
}

fn into_file(
    repo: &Repository,
    plan: &Plan,
    file: &Path,
    overwrite: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Exported> {
    let shown = file.display().to_string();
    let parent = file
        .parent()
        .filter(|parent| parent.is_dir())
        .ok_or_else(|| {
            refused(
                say!("patch-what-export"),
                say!("patch-no-directory", path = shown.as_str()),
            )
        })?;
    outside_git_dir(repo, parent)?;
    match file.symlink_metadata() {
        Ok(meta) if meta.is_dir() => {
            return Err(refused(
                say!("patch-what-export"),
                say!("patch-is-directory", path = shown.as_str()),
            ));
        }
        Ok(_) if !overwrite => return Err(taken(file)),
        _ => {}
    }
    let room = Workroom::beside(parent)?;
    let (out, patches, skipped) = match plan {
        Plan::Diff { cached } => (write_diff(repo, *cached, &room.0, cancel)?, 1, Vec::new()),
        commits => write_mailbox(repo, commits, &room.0, cancel)?,
    };
    if patches == 0 {
        return Err(nothing_written());
    }
    place(&out, file, overwrite)?;
    Ok(Exported {
        files: vec![file.to_path_buf()],
        patches,
        skipped,
    })
}

fn nothing_written() -> Error {
    refused(say!("patch-what-export"), say!("patch-no-patch"))
}

fn into_dir(
    repo: &Repository,
    plan: &Plan,
    dir: &Path,
    overwrite: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Exported> {
    let room = Workroom::beside(dir)?;
    let (written, skipped) = write_each(repo, plan, &room.0, cancel)?;
    if written.is_empty() {
        return Err(nothing_written());
    }
    let targets: Vec<PathBuf> = written
        .iter()
        .filter_map(|file| file.file_name().map(|name| dir.join(name)))
        .collect();
    // All of them or none: look before the first is put in place.
    if !overwrite && let Some(there) = targets.iter().find(|file| file.symlink_metadata().is_ok()) {
        return Err(taken(there));
    }
    for (from, to) in written.iter().zip(&targets) {
        place(from, to, overwrite)?;
    }
    Ok(Exported {
        patches: targets.len(),
        files: targets,
        skipped,
    })
}

/// Write patches out. Commits go to a directory, one file each named as
/// `git format-patch` names them, or to one mailbox file; uncommitted work
/// goes to one file. A file that exists is replaced only with `overwrite`,
/// and a directory that does not exist is made when the one it is in does.
/// Setting `cancel` stops it, and then nothing was written.
pub fn export(
    repo: &Repository,
    source: &Source,
    destination: &Destination,
    overwrite: bool,
    cancel: &Arc<AtomicBool>,
) -> Result<Exported> {
    match destination {
        Destination::File(text) => {
            let file = typed_path(repo, say!("patch-what-file"), text)?;
            let plan = plan(repo, source)?;
            into_file(repo, &plan, &file, overwrite, cancel)
        }
        Destination::Dir(text) => {
            let dir = typed_path(repo, say!("patch-what-directory"), text)?;
            if matches!(source, Source::Staged | Source::Worktree) {
                return Err(invalid(
                    say!("patch-what-destination"),
                    say!("patch-one-file"),
                ));
            }
            let plan = plan(repo, source)?;
            let shown = dir.display().to_string();
            let made = match dir.metadata() {
                Ok(meta) if meta.is_dir() => {
                    outside_git_dir(repo, &dir)?;
                    false
                }
                Ok(_) => {
                    return Err(refused(
                        say!("patch-what-export"),
                        say!("patch-not-directory", path = shown.as_str()),
                    ));
                }
                Err(_) => {
                    // One level only: in a directory that is there.
                    let parent =
                        dir.parent()
                            .filter(|parent| parent.is_dir())
                            .ok_or_else(|| {
                                refused(
                                    say!("patch-what-export"),
                                    say!("patch-no-directory", path = shown.as_str()),
                                )
                            })?;
                    outside_git_dir(repo, parent)?;
                    fs::create_dir(&dir).map_err(io(say!("patch-making-directory")))?;
                    true
                }
            };
            let exported = into_dir(repo, &plan, &dir, overwrite, cancel);
            if exported.is_err() && made {
                let _ = fs::remove_dir(&dir);
            }
            exported
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numstat_records_are_read_with_their_counts() {
        let records = ["3\t1\tsrc/a.rs", "-\t-\tlogo.png", "0\t7\twith\ttab", ""].join("\0");
        let (files, total) = parse_numstat(&records);
        assert_eq!(total, 3);
        assert_eq!(
            files,
            [
                Touched {
                    path: "src/a.rs".to_owned(),
                    added: Some(3),
                    deleted: Some(1),
                    binary: false,
                },
                Touched {
                    path: "logo.png".to_owned(),
                    added: None,
                    deleted: None,
                    binary: true,
                },
                Touched {
                    path: "with\ttab".to_owned(),
                    added: Some(0),
                    deleted: Some(7),
                    binary: false,
                },
            ]
        );
        // Nothing, and records that are none.
        assert_eq!(parse_numstat(""), (Vec::new(), 0));
        assert_eq!(parse_numstat("\0\0garbage\0one\ttwo\0"), (Vec::new(), 0));
        // Very many files are counted and not all listed.
        let many = "1\t0\tf\0".repeat(MAX_LISTED + 10);
        let (files, total) = parse_numstat(&many);
        assert_eq!((files.len(), total), (MAX_LISTED, MAX_LISTED + 10));
    }

    #[test]
    fn what_git_said_is_cut_to_what_a_person_reads() {
        assert_eq!(
            said(
                "error: patch failed: a.txt:1\nerror: a.txt: patch does not apply\nhint: try -3\n"
            ),
            "patch failed: a.txt:1; a.txt: patch does not apply"
        );
        assert_eq!(said("fatal: unrecognized input\n"), "unrecognized input");
        assert_eq!(said("  \nhint: only hints\n"), "git gave no reason");
        let long: String = (0..20).map(|n| format!("error: line {n}\n")).collect();
        assert!(said(&long).ends_with("line 5; …"), "{}", said(&long));
    }

    #[test]
    fn the_options_of_apply_are_the_ones_asked_for() {
        assert!(how(Options::default()).is_empty());
        assert_eq!(
            how(Options {
                index: true,
                three_way: true,
                reverse: true
            }),
            ["--index", "--3way", "--reverse"]
        );
    }
}
