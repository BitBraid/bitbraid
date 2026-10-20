//! Keeping a repository in shape: git's own maintenance, run through
//! git, and how large the repository is and what takes the room.
//!
//! Nothing here deletes an object that git would keep. Each task is one
//! of git's own commands with git's own safety margins: `gc` and `prune`
//! keep unreachable objects younger than `gc.pruneExpire` (two weeks when
//! that is not set), and a repack puts unreachable objects into a pack or
//! a loose file of their own instead of dropping them, as `gc` does. The
//! check (`git fsck`) only reads.
//!
//! `git gc` and `git repack` say nothing about how far they are when they
//! do not write to a terminal, so for those the stage is said here and
//! git's own lines are passed on as far as it writes any.
//!
//! Measuring walks history to find where the largest files came in. In a
//! partial clone that left the trees of history on its remote, a measure
//! nobody asked to fetch for (`exe::local_only`) stops the walk where the
//! first tree is missing, rather than having git fetch every commit's tree
//! one at a time; where the rest came in is not known.
//!
//! Sources: git-gc(1) (`--aggressive`, `gc.pruneExpire`, `gc.cruftPacks`),
//! git-prune(1) (`--expire`, `--progress`), git-repack(1) (`-d`, `-A`,
//! `--cruft`), git-commit-graph(1) (`write --reachable --progress`),
//! git-fsck(1) (what it prints, `--progress`), git-count-objects(1) (`-v`),
//! git-cat-file(1) (`--batch-all-objects`, `%(objectsize:disk)`),
//! git-rev-list(1) (`--count`), git-log(1) and git-diff(1) ("Raw output
//! format"), git-worktree(1), git-lfs-config(5) (`lfs.storage`), git's
//! documentation of partial clone (`partial-clone.adoc`).

use std::{
    cmp::Reverse,
    collections::{BinaryHeap, HashMap},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, exe, remote::Reporter};

/// The largest blobs listed at most.
pub const MOST_BLOBS: usize = 100;
/// The largest blobs listed when nothing else is asked.
pub const DEFAULT_BLOBS: usize = 10;
/// Commits walked for where the largest blobs came in, when nothing else
/// is asked.
pub const DEFAULT_COMMITS: u64 = 100_000;
/// Commits walked at most.
pub const MOST_COMMITS: u64 = 2_000_000;
/// Files and folders looked at in one worktree or in the LFS store before
/// the count stops and says it is short.
pub const MOST_ENTRIES: u64 = 500_000;
/// Findings of a check kept, of each kind; the rest are counted.
pub const MOST_FINDINGS: usize = 500;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

/// The error of an operation that was stopped on request, as the runner
/// of git words it.
fn stopped() -> Error {
    exe::cancelled()
}

// ---------------------------------------------------------------------
// The tasks.

/// One of git's maintenance tasks.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Task {
    /// `git gc`: pack what is loose, drop what expired, tidy references
    /// and logs, write the commit graph.
    Gc,
    /// `git gc --aggressive`: the same, packing far harder; slow.
    GcAggressive,
    /// `git prune`: remove loose objects nothing reaches that are older
    /// than `gc.pruneExpire`.
    Prune,
    /// `git repack`: everything reachable into one pack, the rest into a
    /// pack of its own (or loose, where `gc.cruftPacks` is false), and
    /// the packs that made redundant removed.
    Repack,
    /// `git commit-graph write --reachable`: the file that makes walking
    /// history fast.
    CommitGraph,
}

impl Task {
    /// What is said while git runs, for the tasks git is silent about.
    fn stage(self) -> &'static str {
        match self {
            Task::Gc | Task::GcAggressive => say!("maintenance-stage-gc"),
            Task::Prune => say!("maintenance-stage-prune"),
            Task::Repack => say!("maintenance-stage-repack"),
            Task::CommitGraph => say!("maintenance-stage-commit-graph"),
        }
    }
}

/// How many objects there are and how much room they take, as
/// `git count-objects -v` counts them. Sizes are in bytes; git counts in
/// whole kibibytes, so they are multiples of 1024.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counts {
    /// Loose objects.
    pub loose: u64,
    /// The room they take.
    pub loose_bytes: u64,
    /// Objects in packs.
    pub packed: u64,
    /// Packs.
    pub packs: u64,
    /// The room the packs take, with their indexes.
    pub pack_bytes: u64,
    /// Loose objects that are in a pack too.
    pub prune_packable: u64,
    /// Files in the object store that are none of git's.
    pub garbage: u64,
    /// The room they take.
    pub garbage_bytes: u64,
}

impl Counts {
    /// All the room the object store takes.
    pub fn total_bytes(&self) -> u64 {
        self.loose_bytes
            .saturating_add(self.pack_bytes)
            .saturating_add(self.garbage_bytes)
    }
}

/// Read what `git count-objects -v` prints.
fn parse_counts(text: &str) -> Counts {
    let mut counts = Counts::default();
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let Ok(value) = value.trim().parse::<u64>() else {
            continue;
        };
        let kib = value.saturating_mul(1024);
        match key.trim() {
            "count" => counts.loose = value,
            "size" => counts.loose_bytes = kib,
            "in-pack" => counts.packed = value,
            "packs" => counts.packs = value,
            "size-pack" => counts.pack_bytes = kib,
            "prune-packable" => counts.prune_packable = value,
            "garbage" => counts.garbage = value,
            "size-garbage" => counts.garbage_bytes = kib,
            _ => {}
        }
    }
    counts
}

/// The objects of the repository, counted by git.
pub fn count_objects(repo: &Repository) -> Result<Counts> {
    let out = exe::run(repo, ["count-objects", "-v"], None)?;
    Ok(parse_counts(&out.stdout))
}

/// What a task did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Done {
    pub task: Task,
    /// The objects before it ran.
    pub before: Counts,
    /// And after.
    pub after: Counts,
    /// How many bytes the object store took less afterwards; below zero
    /// when it grew, which a repack that turns unreachable objects loose
    /// can make it do.
    pub freed: i64,
    /// What git said.
    pub output: String,
}

/// How far back `prune` keeps unreachable objects: as `gc` does.
fn prune_expiry(repo: &Repository) -> String {
    exe::config_get(repo, "gc.pruneExpire")
        .filter(|value| !value.starts_with('-') && !value.chars().any(char::is_control))
        .unwrap_or_else(|| "2.weeks.ago".to_owned())
}

/// Whether unreachable objects are packed into a pack of their own rather
/// than turned loose: git's default unless `gc.cruftPacks` says no.
fn cruft_packs(repo: &Repository) -> bool {
    !matches!(
        exe::run(repo, ["config", "--get", "--bool", "gc.cruftPacks"], None)
            .map(|out| out.stdout.trim().to_owned())
            .as_deref(),
        Ok("false")
    )
}

/// The arguments git is run with for `task`.
fn task_args(repo: &Repository, task: Task) -> Vec<String> {
    let words: Vec<String> = match task {
        Task::Gc => vec!["gc".into()],
        Task::GcAggressive => vec!["gc".into(), "--aggressive".into()],
        Task::Prune => vec![
            "prune".into(),
            "--progress".into(),
            format!("--expire={}", prune_expiry(repo)),
        ],
        Task::Repack => {
            let keep = if cruft_packs(repo) { "--cruft" } else { "-A" };
            vec!["repack".into(), "-d".into(), keep.into()]
        }
        Task::CommitGraph => vec![
            "commit-graph".into(),
            "write".into(),
            "--reachable".into(),
            "--progress".into(),
        ],
    };
    words
}

/// Run `task` through git, saying how far it is and stopping when asked.
/// The objects are counted before and after, which is what was freed.
pub fn run(repo: &Repository, task: Task, reporter: &mut Reporter<'_>) -> Result<Done> {
    let before = count_objects(repo)?;
    (reporter.on_progress)(crate::Progress {
        stage: task.stage().to_owned(),
        percent: None,
    });
    let args = task_args(repo, task);
    let cancel = reporter.cancel.clone();
    let mut on_line = |line: &str| {
        if let Some(progress) = crate::remote::parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    };
    let out = exe::run_streaming(repo, &args, &cancel, &mut on_line)?;
    let after = count_objects(repo)?;
    let freed = i128::from(before.total_bytes()) - i128::from(after.total_bytes());
    let freed = i64::try_from(freed).unwrap_or(if freed < 0 { i64::MIN } else { i64::MAX });
    Ok(Done {
        task,
        before,
        after,
        freed,
        output: said(&out),
    })
}

/// What git said, both streams, without git's meters, which are of no use
/// once they stand at their end.
fn said(out: &exe::Output) -> String {
    let mut text = String::new();
    for line in out.stdout.lines().chain(out.stderr.lines()) {
        let line = line.trim_end();
        if line.is_empty() || crate::remote::parse_progress(line).is_some() {
            continue;
        }
        text.push_str(line);
        text.push('\n');
    }
    text.trim_end().to_owned()
}

// ---------------------------------------------------------------------
// The check.

/// What kind of thing a check found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FindingKind {
    /// An object nothing refers to: harmless, and what `gc` removes once
    /// it is old enough.
    Dangling,
    /// An object that something refers to and that is not there.
    Missing,
    /// A reference from one object to another that is not there or not
    /// of the right type.
    BrokenLink,
    /// An object that is damaged or does not read.
    Error,
    /// Something git warns about: an object that reads but is not as it
    /// should be.
    Warning,
    /// Git could not go on.
    Fatal,
    /// Anything else git said that is not progress.
    Other,
}

/// One thing a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Finding {
    /// What kind of thing.
    pub kind: FindingKind,
    /// The object it is about, when git names one.
    pub id: Option<String>,
    /// The object's type, when git says it.
    pub object: Option<String>,
    /// What git said.
    pub text: String,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct Check {
    /// Git found nothing wrong: dangling objects are not wrong.
    pub ok: bool,
    /// Git checked everything; false when it stopped for a fatal error.
    pub finished: bool,
    /// What is wrong, the first [`MOST_FINDINGS`] of it.
    pub problems: Vec<Finding>,
    /// How many things are wrong in all.
    pub problem_count: u64,
    /// The dangling objects, the first [`MOST_FINDINGS`] of them.
    pub dangling: Vec<Finding>,
    /// How many dangling objects there are in all.
    pub dangling_count: u64,
}

/// A hexadecimal object id, as git prints one.
fn looks_like_id(word: &str) -> bool {
    (word.len() == 40 || word.len() == 64) && word.bytes().all(|b| b.is_ascii_hexdigit())
}

/// The object type and id at the start of `rest`: `blob 1234…`.
fn type_and_id(rest: &str) -> (Option<String>, Option<String>) {
    let mut words = rest.split_whitespace();
    let object = words.next().map(str::to_owned);
    let id = words
        .next()
        .map(|word| word.trim_end_matches([':', ',']))
        .filter(|word| looks_like_id(word))
        .map(str::to_owned);
    match id {
        Some(id) => (object, Some(id)),
        None => (None, None),
    }
}

/// Read one line of what `git fsck` says. None for progress and blank
/// lines; a line that goes on the one before (the `to …` of a broken
/// link) is returned as [`FindingKind::Other`] with no id, for the caller
/// to join.
fn parse_finding(line: &str) -> Option<Finding> {
    let text = line.trim();
    if text.is_empty() || crate::remote::parse_progress(text).is_some() {
        return None;
    }
    let (kind, rest) = if let Some(rest) = text.strip_prefix("dangling ") {
        (FindingKind::Dangling, rest)
    } else if let Some(rest) = text.strip_prefix("missing ") {
        (FindingKind::Missing, rest)
    } else if let Some(rest) = text.strip_prefix("broken link from") {
        (FindingKind::BrokenLink, rest)
    } else if let Some(rest) = text.strip_prefix("error in ") {
        (FindingKind::Error, rest)
    } else if let Some(rest) = text.strip_prefix("warning in ") {
        (FindingKind::Warning, rest)
    } else if let Some(rest) = text.strip_prefix("error: ") {
        (FindingKind::Error, rest)
    } else if let Some(rest) = text.strip_prefix("warning: ") {
        (FindingKind::Warning, rest)
    } else if let Some(rest) = text.strip_prefix("fatal: ") {
        (FindingKind::Fatal, rest)
    } else if text.starts_with("notice: ") || text.starts_with("Checking ") {
        // What git tells without anything being wrong: an unborn branch,
        // the stage it is at.
        return None;
    } else {
        (FindingKind::Other, "")
    };
    let (object, id) = type_and_id(rest);
    Some(Finding {
        kind,
        id,
        object,
        text: text.to_owned(),
    })
}

impl Check {
    fn take(&mut self, finding: Finding) {
        if finding.kind == FindingKind::Other
            && finding.text.starts_with("to ")
            && let Some(last) = self.problems.last_mut()
            && last.kind == FindingKind::BrokenLink
        {
            last.text.push(' ');
            last.text.push_str(&finding.text);
            return;
        }
        if finding.kind == FindingKind::Fatal {
            self.finished = false;
        }
        if finding.kind == FindingKind::Dangling {
            self.dangling_count = self.dangling_count.saturating_add(1);
            if self.dangling.len() < MOST_FINDINGS {
                self.dangling.push(finding);
            }
            return;
        }
        self.problem_count = self.problem_count.saturating_add(1);
        if self.problems.len() < MOST_FINDINGS {
            self.problems.push(finding);
        }
    }
}

/// Check the objects and their links with `git fsck`, which changes
/// nothing. What git finds is the answer, also when git ends badly for
/// it; an error is only what kept git from checking at all.
pub fn fsck(repo: &Repository, reporter: &mut Reporter<'_>) -> Result<Check> {
    let mut check = Check {
        finished: true,
        ..Check::default()
    };
    let cancel = reporter.cancel.clone();
    let ran = exe::run_followed(
        repo,
        ["fsck", "--progress"],
        &[],
        None,
        &cancel,
        &mut |said| {
            let line = match said {
                exe::Said::Out(line) | exe::Said::Err(line) | exe::Said::Reported(line) => line,
            };
            if let Some(progress) = crate::remote::parse_progress(line) {
                (reporter.on_progress)(progress);
                return;
            }
            if let Some(finding) = parse_finding(line) {
                check.take(finding);
            }
        },
    );
    if cancel.load(Ordering::SeqCst) {
        return Err(stopped());
    }
    match ran {
        Ok(_) => {}
        // Git ends badly when it finds something wrong: what it found is
        // the answer.
        Err(Error::Command { .. }) if check.problem_count > 0 => {}
        Err(err) => return Err(err),
    }
    check.ok = check.problem_count == 0 && check.finished;
    Ok(check)
}

// ---------------------------------------------------------------------
// The size of the repository.

/// How many of something there are and the room they take.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct Tally {
    /// How many.
    pub count: u64,
    /// The bytes they take.
    pub bytes: u64,
    /// Everything was counted; false when the count stopped at its limit.
    pub complete: bool,
}

/// The objects of each type, by the room they take in the store.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct ByType {
    pub commits: Tally,
    pub trees: Tally,
    /// Blobs: the content of files.
    pub blobs: Tally,
    /// Annotated tags.
    pub tags: Tally,
}

/// A worktree and the room its files take, its git directory left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WorktreeSize {
    /// Where it is.
    #[serde(serialize_with = "crate::path_text")]
    pub path: PathBuf,
    /// The branch checked out there.
    pub branch: Option<String>,
    /// The main worktree.
    pub is_main: bool,
    /// Its files.
    pub files: Tally,
}

/// One of the largest blobs in history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LargeBlob {
    /// The blob.
    pub id: String,
    /// Its size.
    pub size: u64,
    /// The room it takes in the store, compressed or as a delta.
    pub disk_size: u64,
    /// The path it had in the commit it came in by.
    pub path: Option<String>,
    /// The commit it came in by: the oldest of those the walk saw add it.
    pub commit: Option<Oid>,
    /// That commit's summary.
    pub summary: Option<String>,
    /// That commit's time, in seconds since 1970.
    pub time: Option<i64>,
}

/// How large the repository is and what takes the room.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SizeReport {
    /// The object store, as git counts it.
    pub objects: Counts,
    /// The objects by type.
    pub by_type: ByType,
    /// The content of LFS files kept here, when there is an LFS store.
    pub lfs: Option<Tally>,
    /// Every worktree.
    pub worktrees: Vec<WorktreeSize>,
    /// The largest blobs in history, largest first.
    pub largest: Vec<LargeBlob>,
    /// Commits walked to find where they came in.
    pub walked: u64,
    /// The walk saw every commit; false when it stopped at its limit, and
    /// then a blob it did not see come in is listed without a path.
    pub walk_complete: bool,
    /// The walk stopped where a partial clone left the trees of history
    /// on its remote, which nobody had asked to be fetched.
    pub walk_left_on_remote: bool,
}

/// What [`size`] is asked to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SizeOptions {
    /// How many of the largest blobs to list.
    pub blobs: usize,
    /// How many commits to walk at most for where they came in.
    pub commits: u64,
}

impl Default for SizeOptions {
    fn default() -> Self {
        Self {
            blobs: DEFAULT_BLOBS,
            commits: DEFAULT_COMMITS,
        }
    }
}

/// Run git and hand every line it writes on standard output to `on_line`
/// as it comes, keeping only the end of it: for listings as long as the
/// repository is large.
fn each_line<I, S>(
    repo: &Repository,
    args: I,
    cancel: &Arc<AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let ran = exe::run_followed(repo, args, &[], None, cancel, &mut |said| {
        if let exe::Said::Out(line) = said {
            on_line(line);
        }
    });
    if cancel.load(Ordering::SeqCst) {
        return Err(stopped());
    }
    ran.map(|_| ())
}

/// Count the files under `dir` and the bytes they hold, without following
/// links, leaving out what `skip` says and stopping at [`MOST_ENTRIES`]
/// or when asked.
fn tally_files(dir: &Path, skip: &dyn Fn(&Path) -> bool, cancel: &AtomicBool) -> Tally {
    let mut tally = Tally {
        complete: true,
        ..Tally::default()
    };
    let mut seen: u64 = 0;
    let mut waiting = vec![dir.to_path_buf()];
    while let Some(dir) = waiting.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            seen = seen.saturating_add(1);
            if seen > MOST_ENTRIES || cancel.load(Ordering::Relaxed) {
                tally.complete = false;
                return tally;
            }
            let path = entry.path();
            if skip(&path) {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                waiting.push(path);
            } else {
                tally.count = tally.count.saturating_add(1);
                tally.bytes = tally.bytes.saturating_add(meta.len());
            }
        }
    }
    tally
}

/// Where git-lfs keeps content: `lfs.storage` when that is set (relative
/// to the git directory), else `lfs` in it.
fn lfs_store(repo: &Repository) -> PathBuf {
    let common = repo.gix().common_dir().to_path_buf();
    match exe::config_get(repo, "lfs.storage") {
        Some(set) if Path::new(&set).is_absolute() => PathBuf::from(set),
        Some(set) => common.join(set),
        None => common.join("lfs"),
    }
    .join("objects")
}

/// The type of an object as `cat-file` names it, into the tally of it.
fn tally_of<'a>(by_type: &'a mut ByType, kind: &str) -> Option<&'a mut Tally> {
    match kind {
        "commit" => Some(&mut by_type.commits),
        "tree" => Some(&mut by_type.trees),
        "blob" => Some(&mut by_type.blobs),
        "tag" => Some(&mut by_type.tags),
        _ => None,
    }
}

/// A blob that may be among the largest.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Candidate {
    size: u64,
    id: String,
    disk_size: u64,
}

/// Where a candidate came in.
#[derive(Debug, Clone)]
struct CameIn {
    commit: Oid,
    path: String,
    summary: String,
    time: i64,
}

/// The commit a line of the walk starts: `\x01<id>\x01<time>\x01<summary>`.
fn parse_commit_line(line: &str) -> Option<(Oid, i64, String)> {
    let rest = line.strip_prefix('\u{1}')?;
    let mut parts = rest.splitn(3, '\u{1}');
    let id = parts.next()?.parse::<Oid>().ok()?;
    let time = parts.next()?.trim().parse::<i64>().ok()?;
    let summary = parts.next().unwrap_or("").to_owned();
    Some((id, time, summary))
}

/// One line of raw diff output: the new object and the path, for a change
/// that leaves something there (not a deletion).
fn parse_raw_line(line: &str) -> Option<(&str, String)> {
    let rest = line.strip_prefix(':')?;
    let (fields, path) = rest.split_once('\t')?;
    let mut words = fields.split(' ');
    let _old_mode = words.next()?;
    let _new_mode = words.next()?;
    let _old = words.next()?;
    let new = words.next()?;
    let status = words.next()?;
    if status.starts_with('D') || !looks_like_id(new) {
        return None;
    }
    let path = match path.strip_prefix('"') {
        Some(quoted) => crate::lfs::unquote(quoted).map(|(path, _)| path)?,
        None => path.to_owned(),
    };
    Some((new, path))
}

/// How large the repository is and what takes the room: the object store
/// as git counts it, the objects by type, the LFS store, every worktree's
/// files, and the largest blobs in history with the path and the commit
/// they came in by. The walk for those is bounded by `options.commits`;
/// everything stops when asked.
pub fn size(
    repo: &Repository,
    options: SizeOptions,
    reporter: &mut Reporter<'_>,
) -> Result<SizeReport> {
    if options.blobs > MOST_BLOBS {
        return Err(invalid(
            say!("maintenance-what-blobs"),
            say!("maintenance-most-blobs", most = MOST_BLOBS),
        ));
    }
    if options.commits > MOST_COMMITS {
        return Err(invalid(
            say!("maintenance-what-commits"),
            say!("maintenance-most-commits", most = MOST_COMMITS),
        ));
    }
    let cancel = reporter.cancel.clone();
    let mut say = |stage: &str, percent: Option<u8>| {
        (reporter.on_progress)(crate::Progress {
            stage: stage.to_owned(),
            percent,
        });
    };
    say(say!("maintenance-counting-objects"), None);
    let objects = count_objects(repo)?;

    // Every object once, by type, and the largest blobs among them. More
    // are kept than are asked for: some may not be in history at all.
    let keep = options.blobs.saturating_mul(4).max(options.blobs);
    let mut by_type = ByType {
        commits: Tally {
            complete: true,
            ..Tally::default()
        },
        ..ByType::default()
    };
    by_type.trees.complete = true;
    by_type.blobs.complete = true;
    by_type.tags.complete = true;
    let mut heap: BinaryHeap<Reverse<Candidate>> = BinaryHeap::new();
    say(say!("maintenance-measuring-objects"), None);
    each_line(
        repo,
        [
            "cat-file",
            "--batch-all-objects",
            "--batch-check=%(objecttype) %(objectname) %(objectsize) %(objectsize:disk)",
        ],
        &cancel,
        &mut |line| {
            let mut words = line.split(' ');
            let (Some(kind), Some(id), Some(size), Some(disk)) =
                (words.next(), words.next(), words.next(), words.next())
            else {
                return;
            };
            let (Ok(size), Ok(disk_size)) = (size.parse::<u64>(), disk.parse::<u64>()) else {
                return;
            };
            if let Some(tally) = tally_of(&mut by_type, kind) {
                tally.count = tally.count.saturating_add(1);
                tally.bytes = tally.bytes.saturating_add(disk_size);
            }
            if kind != "blob" || keep == 0 {
                return;
            }
            let smaller_than_all =
                heap.len() >= keep && heap.peek().is_some_and(|Reverse(least)| least.size >= size);
            if smaller_than_all {
                return;
            }
            heap.push(Reverse(Candidate {
                size,
                id: id.to_owned(),
                disk_size,
            }));
            if heap.len() > keep {
                heap.pop();
            }
        },
    )?;
    let mut candidates: Vec<Candidate> = heap.into_iter().map(|Reverse(one)| one).collect();
    candidates.sort_by(|one, other| other.cmp(one));

    // Where they came in: the walk goes from the newest commits back, so
    // the last commit seen to add a blob is the oldest that did.
    let mut came_in: HashMap<String, CameIn> = HashMap::new();
    let mut walked: u64 = 0;
    // Without blobs there is nothing to look for, and the answer is whole.
    let mut walk_complete = candidates.is_empty();
    let mut walk_left_on_remote = false;
    if !candidates.is_empty() && options.commits > 0 {
        let wanted: std::collections::HashSet<&str> =
            candidates.iter().map(|one| one.id.as_str()).collect();
        let limit = options.commits.to_string();
        // One more than the limit: whether there are more commits than
        // are walked.
        let beyond = options.commits.saturating_add(1).to_string();
        let counted = exe::run(
            repo,
            [
                "rev-list",
                "--count",
                "--all",
                "--max-count",
                beyond.as_str(),
            ],
            None,
        )
        .ok()
        .and_then(|out| out.stdout.trim().parse::<u64>().ok());
        let total = counted.unwrap_or(0).min(options.commits);
        say(say!("maintenance-walking-history"), Some(0));
        let mut current: Option<(Oid, i64, String)> = None;
        let mut last_percent: u8 = 0;
        let walk = each_line(
            repo,
            [
                "-c",
                "core.quotePath=false",
                "log",
                "--all",
                "--raw",
                "--no-abbrev",
                "--no-renames",
                "--no-color",
                "--format=%x01%H%x01%ct%x01%s",
                "--max-count",
                limit.as_str(),
            ],
            &cancel,
            &mut |line| {
                if let Some(commit) = parse_commit_line(line) {
                    walked = walked.saturating_add(1);
                    current = Some(commit);
                    if let Some(percent) = walked.saturating_mul(100).checked_div(total) {
                        let percent = u8::try_from(percent.min(100)).unwrap_or(100);
                        if percent != last_percent {
                            last_percent = percent;
                            say(say!("maintenance-walking-history"), Some(percent));
                        }
                    }
                    return;
                }
                let Some((new, path)) = parse_raw_line(line) else {
                    return;
                };
                if !wanted.contains(new) {
                    return;
                }
                if let Some((commit, time, summary)) = &current {
                    came_in.insert(
                        new.to_owned(),
                        CameIn {
                            commit: *commit,
                            path,
                            summary: summary.clone(),
                            time: *time,
                        },
                    );
                }
            },
        );
        walk_complete = match walk {
            Ok(()) => match counted {
                Some(counted) => counted <= options.commits,
                None => walked < options.commits,
            },
            // A partial clone that left the trees of history on its remote,
            // measured while nobody asked for them to be fetched:
            // where the largest came in is known as far as the walk got.
            Err(_)
                if !cancel.load(Ordering::SeqCst)
                    && exe::is_local_only()
                    && crate::promisor::is_partial(repo) =>
            {
                walk_left_on_remote = true;
                false
            }
            Err(err) => return Err(err),
        };
    }
    let mut largest = Vec::new();
    for candidate in candidates {
        if largest.len() >= options.blobs {
            break;
        }
        let found = came_in.remove(&candidate.id);
        // Not seen coming in by a walk that saw everything: not in
        // history (unreachable, or only in the index).
        if found.is_none() && walk_complete {
            continue;
        }
        largest.push(LargeBlob {
            id: candidate.id,
            size: candidate.size,
            disk_size: candidate.disk_size,
            path: found.as_ref().map(|one| one.path.clone()),
            commit: found.as_ref().map(|one| one.commit),
            summary: found.as_ref().map(|one| one.summary.clone()),
            time: found.as_ref().map(|one| one.time),
        });
    }

    say(say!("maintenance-measuring-files"), None);
    let store = lfs_store(repo);
    let lfs = store
        .is_dir()
        .then(|| tally_files(&store, &|_| false, &cancel));
    let listed = crate::worktree::list(repo, false)?;
    let places: Vec<PathBuf> = listed
        .iter()
        .filter(|one| !one.bare)
        .map(|one| one.path.clone())
        .collect();
    let mut worktrees = Vec::new();
    for one in listed.iter().filter(|one| !one.bare && !one.prunable) {
        let root = one.path.clone();
        let skip = |path: &Path| {
            // Its git directory, and the other worktrees inside it.
            (path.parent() == Some(root.as_path())
                && path.file_name().is_some_and(|name| name == ".git"))
                || places.iter().any(|place| place != &root && place == path)
        };
        let files = tally_files(&root, &skip, &cancel);
        worktrees.push(WorktreeSize {
            path: one.path.clone(),
            branch: one.branch.clone(),
            is_main: one.is_main,
            files,
        });
    }
    if cancel.load(Ordering::SeqCst) {
        return Err(stopped());
    }
    Ok(SizeReport {
        objects,
        by_type,
        lfs,
        worktrees,
        largest,
        walked,
        walk_complete,
        walk_left_on_remote,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_are_read_as_git_prints_them() {
        let counts = parse_counts(
            "count: 3\nsize: 12\nin-pack: 20\npacks: 1\nsize-pack: 979\nprune-packable: 2\ngarbage: 1\nsize-garbage: 4\n",
        );
        assert_eq!(counts.loose, 3);
        assert_eq!(counts.loose_bytes, 12 * 1024);
        assert_eq!(counts.packed, 20);
        assert_eq!(counts.packs, 1);
        assert_eq!(counts.pack_bytes, 979 * 1024);
        assert_eq!(counts.prune_packable, 2);
        assert_eq!(counts.garbage, 1);
        assert_eq!(counts.garbage_bytes, 4 * 1024);
        assert_eq!(counts.total_bytes(), (12 + 979 + 4) * 1024);
        // Nonsense is passed over, and a number too large saturates.
        let odd = parse_counts("count: x\n: 5\nsize: 18446744073709551615\nwhat\n");
        assert_eq!(odd.loose, 0);
        assert_eq!(odd.loose_bytes, u64::MAX);
    }

    #[test]
    fn findings_are_told_apart() {
        let id = "1234567890123456789012345678901234567890";
        let dangling = parse_finding(&format!("dangling commit {id}")).unwrap();
        assert_eq!(dangling.kind, FindingKind::Dangling);
        assert_eq!(dangling.id.as_deref(), Some(id));
        assert_eq!(dangling.object.as_deref(), Some("commit"));
        let missing = parse_finding(&format!("missing blob {id}")).unwrap();
        assert_eq!(missing.kind, FindingKind::Missing);
        let broken = parse_finding(&format!("broken link from    tree {id}")).unwrap();
        assert_eq!(broken.kind, FindingKind::BrokenLink);
        assert_eq!(broken.id.as_deref(), Some(id));
        let error = parse_finding(&format!("error in tree {id}: badTree: oops")).unwrap();
        assert_eq!(error.kind, FindingKind::Error);
        assert_eq!(error.id.as_deref(), Some(id));
        assert_eq!(
            parse_finding("error: object file .git/objects/ab/cd is empty")
                .unwrap()
                .kind,
            FindingKind::Error
        );
        assert!(parse_finding("Checking objects: 100% (20/20), done.").is_none());
        assert!(parse_finding("notice: HEAD points to an unborn branch (main)").is_none());
        assert!(parse_finding("   ").is_none());
        // The second line of a broken link joins the first.
        let mut check = Check::default();
        check.take(broken);
        check.take(parse_finding(&format!("              to    blob {id}")).unwrap());
        assert_eq!(check.problems.len(), 1);
        assert!(
            check.problems[0]
                .text
                .ends_with(&format!("to    blob {id}"))
        );
        // Nothing hostile reads as an id.
        assert_eq!(type_and_id("blob 12zz"), (None, None));
        assert_eq!(type_and_id(""), (None, None));
    }

    #[test]
    fn findings_are_kept_up_to_a_limit_and_counted_beyond() {
        let mut check = Check::default();
        for _ in 0..(MOST_FINDINGS + 7) {
            check.take(Finding {
                kind: FindingKind::Dangling,
                id: None,
                object: None,
                text: "dangling".into(),
            });
            check.take(Finding {
                kind: FindingKind::Missing,
                id: None,
                object: None,
                text: "missing".into(),
            });
        }
        assert_eq!(check.dangling.len(), MOST_FINDINGS);
        assert_eq!(check.problems.len(), MOST_FINDINGS);
        assert_eq!(check.dangling_count, (MOST_FINDINGS + 7) as u64);
        assert_eq!(check.problem_count, (MOST_FINDINGS + 7) as u64);
    }

    #[test]
    fn walk_lines_are_read() {
        let id = "1234567890123456789012345678901234567890";
        let (commit, time, summary) =
            parse_commit_line(&format!("\u{1}{id}\u{1}1700000000\u{1}a \u{1} summary")).unwrap();
        assert_eq!(commit.to_hex(), id);
        assert_eq!(time, 1_700_000_000);
        assert_eq!(summary, "a \u{1} summary");
        assert!(parse_commit_line("plain").is_none());
        assert!(parse_commit_line("\u{1}zz\u{1}1\u{1}x").is_none());
        let zero = "0".repeat(40);
        let added = format!(":000000 100644 {zero} {id} A\tdir/file name");
        assert_eq!(
            parse_raw_line(&added),
            Some((id, "dir/file name".to_owned()))
        );
        let quoted = format!(":100644 100644 {zero} {id} M\t\"tab\\there\"");
        assert_eq!(parse_raw_line(&quoted), Some((id, "tab\there".to_owned())));
        let deleted = format!(":100644 000000 {id} {zero} D\tgone");
        assert_eq!(parse_raw_line(&deleted), None);
        assert_eq!(parse_raw_line(":short"), None);
        assert_eq!(parse_raw_line(&format!(":1 2 3 {id} M\t\"open")), None);
    }
}
