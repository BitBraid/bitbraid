//! Large synthetic repositories for performance budgets, generated with
//! `git fast-import` (orders of magnitude faster than one commit per `git
//! commit`). Deterministic: same parameters, same object ids.
//!
//! A corpus is built once and reused, by tests that run side by side in one
//! process and by several processes at once (`cargo test` of two crates,
//! two checkouts sharing a target directory). So a builder never works
//! where the corpus is looked for: it builds into a directory beside it,
//! under a name of its own, writes the mark that says "whole" into that
//! as the last thing, and then renames the directory into place, which the
//! file system does at once or not at all. What stands at the place of a
//! corpus and carries the mark is therefore always whole. Two builders at
//! once both build; the first rename wins and the other throws its copy
//! away (the copies are the same, byte for byte). Within one process a
//! lock lets the second wait for the first instead. What a builder that
//! died left beside the corpus is removed when it is some hours old.
//!
//! Sources: git-fast-import(1); rename(2) (a directory is replaced only
//! when it is empty, and atomically).

use std::{
    fmt::Write as _,
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Stdio,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime},
};

use anyhow::{Context, Result, bail};

use crate::{
    Git, fixtures_out_dir,
    oracle::{CLOCK_EPOCH, clock_at},
};

/// Bump when the generated stream changes; stale corpora are rebuilt.
const GENERATOR_VERSION: u32 = 3;

/// The seconds between one commit of a synthetic history and the next:
/// an hour, as the fixtures have it, unless the last commit would then be
/// dated past the last second a 32-bit unsigned number holds (February
/// 2106), and then as many as keep it before. Git for Windows reads a raw
/// date in `fast-import` into a `long`, which is 32 bits there, and stops
/// at the first commit beyond: an hour apart from 2020, the millionth
/// commit is in 2134.
fn seconds_between(commits: u64) -> u64 {
    const HOUR: u64 = 3600;
    let room = u64::from(u32::MAX).saturating_sub(u64::try_from(CLOCK_EPOCH).unwrap_or(0));
    if commits.saturating_mul(HOUR) <= room {
        HOUR
    } else {
        (room / commits.max(1)).max(1)
    }
}

/// The date of the commit at `tick` of a synthetic history whose commits
/// are `step` seconds apart, as fast-import takes it.
fn synthetic_date(tick: u64, step: u64) -> String {
    if step == 3600 {
        return clock_at(tick);
    }
    let since = i64::try_from(tick.saturating_mul(step)).unwrap_or(i64::MAX);
    format!("{} +0000", CLOCK_EPOCH.saturating_add(since))
}

/// Shape of a synthetic repository.
#[derive(Debug, Clone, Copy)]
pub struct CorpusSpec {
    /// Total commits.
    pub commits: u64,
    /// Concurrent branches that merge back into `main` periodically.
    pub lanes: u64,
    /// Commits per branch between merges.
    pub merge_every: u64,
    /// Distinct files touched.
    pub files: u64,
}

impl Default for CorpusSpec {
    fn default() -> Self {
        Self {
            commits: 1_000_000,
            lanes: 8,
            merge_every: 50,
            files: 1000,
        }
    }
}

/// Directory of corpus repositories.
pub fn corpus_dir() -> PathBuf {
    fixtures_out_dir()
        .parent()
        .map(|p| p.join("corpus"))
        .unwrap_or_else(|| PathBuf::from("target/corpus"))
}

fn fast_import_stream(spec: &CorpusSpec) -> String {
    // Branch `main` carries the trunk; lanes `lane-N` branch off and merge
    // back every `merge_every` commits, giving the graph real merges.
    let mut out = String::with_capacity((spec.commits as usize).saturating_mul(160));
    let mut mark: u64 = 0;
    let mut lane_heads: Vec<Option<u64>> = vec![None; spec.lanes as usize];
    let mut tick: u64 = 0;
    let mut emitted: u64 = 0;
    let step = seconds_between(spec.commits);

    let commit = |out: &mut String,
                  branch: &str,
                  parents: &[u64],
                  file: u64,
                  msg: &str,
                  mark: &mut u64,
                  tick: &mut u64|
     -> u64 {
        *mark += 1;
        *tick += 1;
        let (name, email) = crate::oracle::author_for(*tick);
        let date = synthetic_date(*tick, step);
        let _ = writeln!(out, "commit refs/heads/{branch}");
        let _ = writeln!(out, "mark :{}", *mark);
        let _ = writeln!(out, "author {name} <{email}> {date}");
        let _ = writeln!(out, "committer {name} <{email}> {date}");
        let _ = writeln!(out, "data {}\n{msg}", msg.len());
        for (i, p) in parents.iter().enumerate() {
            let _ = writeln!(out, "{} :{p}", if i == 0 { "from" } else { "merge" });
        }
        let content = format!("file {file} at tick {}\n", *tick);
        let _ = writeln!(out, "M 100644 inline dir{}/file{file}.txt", file % 32);
        let _ = writeln!(out, "data {}\n{content}", content.len());
        out.push('\n');
        *mark
    };

    // Root commit.
    let mut main_head: Option<u64> = Some(commit(
        &mut out,
        "main",
        &[],
        0,
        "root",
        &mut mark,
        &mut tick,
    ));
    emitted += 1;
    let mut rounds: u64 = 0;

    while emitted < spec.commits {
        for lane in 0..spec.lanes {
            if emitted >= spec.commits {
                break;
            }
            let branch = format!("lane-{lane}");
            let lane_ix = lane as usize;
            let base = lane_heads.get(lane_ix).copied().flatten().or(main_head);
            let parents: Vec<u64> = base.into_iter().collect();
            let file = (emitted.wrapping_mul(7919)) % spec.files;
            let m = commit(
                &mut out,
                &branch,
                &parents,
                file,
                &format!("lane {lane} commit {emitted}"),
                &mut mark,
                &mut tick,
            );
            if let Some(slot) = lane_heads.get_mut(lane_ix) {
                *slot = Some(m);
            }
            emitted += 1;
        }
        rounds += 1;
        if rounds.is_multiple_of(spec.merge_every.max(1)) || emitted >= spec.commits {
            // Merge every lane into main (octopus when lanes > 1), then restart lanes from main.
            let mut parents: Vec<u64> = main_head.into_iter().collect();
            parents.extend(lane_heads.iter().flatten().copied());
            if parents.len() > 1 && emitted < spec.commits {
                let m = commit(
                    &mut out,
                    "main",
                    &parents,
                    emitted % spec.files,
                    &format!("merge lanes at {emitted}"),
                    &mut mark,
                    &mut tick,
                );
                main_head = Some(m);
                emitted += 1;
                for slot in lane_heads.iter_mut() {
                    *slot = None;
                }
            }
        }
    }
    if let Some(m) = main_head {
        let _ = writeln!(out, "reset refs/heads/main\nfrom :{m}\n");
    }
    out
}

/// One builder at a time in this process: the second waits and finds what
/// the first made.
static BUILDING: Mutex<()> = Mutex::new(());

/// Tells the directories of this process's builders apart.
static BUILDERS: AtomicU64 = AtomicU64::new(0);

/// What marks a directory beside a corpus as a builder's own.
const BUILDING_MARK: &str = ".building-";

/// What marks one that was in the way and is being removed.
const STALE_MARK: &str = ".stale-";

/// How old what a builder left behind has to be before it is removed: no
/// builder that is still at work is that old.
const LEFT_BEHIND: Duration = Duration::from_secs(6 * 60 * 60);

/// A name beside `name` that no other builder has: of this process and of
/// this call.
fn beside(root: &Path, name: &str, mark: &str) -> PathBuf {
    let n = BUILDERS.fetch_add(1, Ordering::Relaxed);
    root.join(format!("{name}{mark}{}-{n}", std::process::id()))
}

/// Remove a directory with what is in it, or a file: whatever it is that
/// stands in a corpus's way.
fn remove(path: &Path) {
    if fs::remove_dir_all(path).is_err() {
        let _ = fs::remove_file(path);
    }
}

/// Remove what builders that died left in `root`.
fn sweep(root: &Path) {
    let Ok(entries) = fs::read_dir(root) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.contains(BUILDING_MARK) && !name.contains(STALE_MARK) {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|at| SystemTime::now().duration_since(at).ok())
            .is_some_and(|age| age > LEFT_BEHIND);
        if old {
            remove(&entry.path());
        }
    }
}

/// Move a directory that was built whole to where it is looked for. What
/// is there already and is whole stays, and ours goes: another builder was
/// first. What is there and is not whole (left by a builder that died, or
/// made by an older generator) is moved out of the way first.
fn put_in_place(built: &Path, dir: &Path, whole: &dyn Fn(&Path) -> bool) -> Result<()> {
    let (Some(root), Some(name)) = (dir.parent(), dir.file_name()) else {
        bail!("{} is no place for a corpus", dir.display());
    };
    let name = name.to_string_lossy();
    let mut last = None;
    for _ in 0..16 {
        match fs::rename(built, dir) {
            Ok(()) => return Ok(()),
            Err(_) if whole(dir) => {
                remove(built);
                return Ok(());
            }
            Err(err) => {
                if dir.symlink_metadata().is_err() {
                    // Nothing is in the way: it failed for another reason.
                    remove(built);
                    return Err(err)
                        .with_context(|| format!("moving a corpus to {}", dir.display()));
                }
                // Another builder may be moving the same thing away: then
                // one of the two renames finds it gone, which is as good.
                let away = beside(root, &name, STALE_MARK);
                if fs::rename(dir, &away).is_ok() {
                    remove(&away);
                }
                last = Some(err);
            }
        }
    }
    remove(built);
    match last {
        Some(err) => Err(err).with_context(|| format!("moving a corpus to {}", dir.display())),
        None => bail!("moving a corpus to {}", dir.display()),
    }
}

/// Run `git fast-import` on a stream.
fn fast_import(git: &Git, stream: &str) -> Result<()> {
    let mut child = git
        .command()
        .args(["fast-import", "--quiet", "--done"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawning git fast-import")?;
    // What it says is read while it is fed: when it stops early, the pipe
    // it no longer reads is the least of it, and its words say why.
    let mut stderr = child.stderr.take().context("fast-import stderr")?;
    let said = std::thread::spawn(move || {
        let mut text = Vec::new();
        let _ = std::io::Read::read_to_end(&mut stderr, &mut text);
        text
    });
    let fed = {
        let mut stdin = child.stdin.take().context("fast-import stdin")?;
        stdin
            .write_all(stream.as_bytes())
            .and_then(|()| stdin.write_all(b"done\n"))
    };
    let status = child.wait()?;
    let said = said.join().unwrap_or_default();
    if !status.success() || fed.is_err() {
        bail!(
            "git fast-import failed ({status}{}): {}",
            fed.err()
                .map(|err| format!(", fed until: {err}"))
                .unwrap_or_default(),
            String::from_utf8_lossy(&said)
        );
    }
    Ok(())
}

/// Build a corpus beside its place and move it there: `fill` makes the
/// repository in the directory it is given, `whole` says whether what
/// stands at a place is a finished corpus, and `mark` writes what `whole`
/// looks for, as the last thing before the move.
fn build_in_place(
    dir: &Path,
    whole: &dyn Fn(&Path) -> bool,
    fill: &dyn Fn(&Git) -> Result<()>,
    mark: &dyn Fn(&Path) -> Result<()>,
) -> Result<()> {
    if whole(dir) {
        return Ok(());
    }
    let (Some(root), Some(name)) = (dir.parent(), dir.file_name()) else {
        bail!("{} is no place for a corpus", dir.display());
    };
    fs::create_dir_all(root)?;
    sweep(root);
    let building = beside(root, &name.to_string_lossy(), BUILDING_MARK);
    let built = (|| -> Result<()> {
        fs::create_dir_all(&building)?;
        let git = Git::new(&building, fixtures_out_dir().join(".home"));
        git.ensure_home()?;
        git.run(["init", "-q", "-b", "main"])?;
        fill(&git)?;
        mark(&building)
    })();
    if let Err(err) = built {
        remove(&building);
        return Err(err);
    }
    put_in_place(&building, dir, whole)
}

/// Build (or reuse) the synthetic corpus for `spec`; returns its path. Safe
/// to call from several threads and several processes at once.
pub fn build_synthetic(spec: &CorpusSpec) -> Result<PathBuf> {
    let _one = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    synthetic_in(&corpus_dir(), spec)
}

/// The synthetic corpus for `spec` under `root`, built when it is not
/// there whole.
fn synthetic_in(root: &Path, spec: &CorpusSpec) -> Result<PathBuf> {
    let dir = root.join(format!(
        "synthetic-{}-l{}-m{}-f{}",
        spec.commits, spec.lanes, spec.merge_every, spec.files
    ));
    let want = format!("generator {GENERATOR_VERSION}");
    let stamp = |dir: &Path| dir.join(".corpus-ok");
    build_in_place(
        &dir,
        &|dir| fs::read_to_string(stamp(dir)).ok().as_deref() == Some(want.as_str()),
        &|git| {
            fast_import(git, &fast_import_stream(spec))?;
            // A commit-graph file is what a real repository would have
            // after `git gc`.
            let _ = git.run(["commit-graph", "write", "--reachable"]);
            Ok(())
        },
        &|dir| Ok(fs::write(stamp(dir), &want)?),
    )?;
    Ok(dir)
}

/// Build (or reuse) a repository with one commit containing `files` small
/// files spread over directories, checked out, for status benchmarks. Safe
/// to call from several threads and several processes at once.
pub fn build_wide(files: u64) -> Result<PathBuf> {
    let _one = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    wide_in(&corpus_dir(), files)
}

/// The wide corpus of `files` files under `root`, built when it is not
/// there whole.
fn wide_in(root: &Path, files: u64) -> Result<PathBuf> {
    let dir = root.join(format!("wide-{files}"));
    let want = format!("generator {GENERATOR_VERSION} wide");
    // The mark is under `.git`, where it moves with the repository and
    // leaves the worktree clean.
    let stamp = |dir: &Path| dir.join(".git").join("corpus-ok");
    let whole = |dir: &Path| fs::read_to_string(stamp(dir)).ok().as_deref() == Some(want.as_str());
    build_in_place(
        &dir,
        &whole,
        &|git| {
            let mut stream = String::with_capacity((files as usize).saturating_mul(96));
            let (name, email) = crate::oracle::author_for(1);
            let date = clock_at(1);
            let _ = writeln!(
                stream,
                "commit refs/heads/main\nmark :1\nauthor {name} <{email}> {date}\ncommitter {name} <{email}> {date}\ndata 4\nwide"
            );
            for i in 0..files {
                let content = format!("file {i}\n");
                let _ = writeln!(
                    stream,
                    "M 100644 inline d{}/d{}/f{i}.txt",
                    i % 97,
                    (i / 97) % 53
                );
                let _ = writeln!(stream, "data {}\n{content}", content.len());
            }
            stream.push('\n');
            fast_import(git, &stream)?;
            git.run(["checkout", "-q", "-f", "main"])?;
            Ok(())
        },
        &|dir| Ok(fs::write(stamp(dir), &want)?),
    )?;
    Ok(dir)
}

/// Shape of a repository with many references and worktrees (by default
/// 33,000 commits, 4,300 references of which 3,548 remote branches, 19
/// worktrees, 14,000 files), on which start-up, scrolling and the
/// references panel are measured: made so that they can be measured
/// anywhere.
#[derive(Debug, Clone, Copy)]
pub struct ManyRefsSpec {
    /// Total commits, on lanes that merge back into `main`.
    pub commits: u64,
    /// Files in the tree, each changed again from time to time.
    pub files: u64,
    /// Local branches, `main` and the lanes among them.
    pub local_branches: u64,
    /// Branches of the remote `origin`.
    pub remote_branches: u64,
    pub tags: u64,
    /// Linked worktrees, each on a local branch of its own, checked out.
    pub worktrees: u64,
}

impl Default for ManyRefsSpec {
    fn default() -> Self {
        Self {
            commits: 33_000,
            files: 14_000,
            local_branches: 400,
            remote_branches: 3_548,
            tags: 352,
            worktrees: 19,
        }
    }
}

/// Bump when the many-references repository's stream changes.
const MANY_REFS_VERSION: u32 = 2;

/// Lanes of the many-references repository, and how often they merge.
const MANY_REFS_LANES: u64 = 6;
const MANY_REFS_MERGE_EVERY: u64 = 20;

/// Where the linked worktrees of the many-references repository are,
/// inside it (and left out of its status by `info/exclude`).
const MANY_REFS_WORKTREES: &str = ".worktrees";

/// The commit a reference of number `k` among `count` points at: spread
/// over the history, the same each time.
fn spread(k: u64, commits: u64) -> u64 {
    1 + k.wrapping_mul(2_654_435_761) % commits.max(1)
}

/// The commits of the many-references repository after its first: the
/// lanes' history, marked 1 and on.
fn many_refs_history(spec: &ManyRefsSpec) -> u64 {
    spec.commits.saturating_sub(1).max(1)
}

/// The first commit of the many-references repository, on `main`: every
/// file, where the lanes' commits change them later. (A merge made by
/// `fast-import` has its first parent's tree, so `main` would otherwise
/// hold only the files its own merges named.) Unmarked: the history's
/// first commit, which names no parent, follows it on `main`.
fn many_refs_first(spec: &ManyRefsSpec) -> String {
    let files = spec.files.max(1);
    let mut out = String::with_capacity((files as usize).saturating_mul(64));
    let (name, email) = crate::oracle::author_for(0);
    let date = clock_at(0);
    let _ = writeln!(
        out,
        "commit refs/heads/main\nauthor {name} <{email}> {date}\ncommitter {name} <{email}> {date}\ndata 11\nevery file"
    );
    for file in 0..files {
        let content = format!("file {file}\n");
        let _ = writeln!(
            out,
            "M 100644 inline dir{}/file{file}.txt\ndata {}\n{content}",
            file % 32,
            content.len()
        );
    }
    out.push('\n');
    out
}

/// The references of the many-references repository, as `fast-import`
/// resets after the commits: local branches beyond `main` and the lanes
/// (the worktrees' first), the remote's branches and tags.
fn many_refs_stream(spec: &ManyRefsSpec) -> String {
    let mut out = String::new();
    let history = many_refs_history(spec);
    let own = spec.local_branches.saturating_sub(MANY_REFS_LANES + 1);
    for k in 0..own {
        let name = if k < spec.worktrees {
            format!("wt/branch-{k}")
        } else {
            format!("topic/area-{}/change-{k}", k % 20)
        };
        let _ = writeln!(
            out,
            "reset refs/heads/{name}\nfrom :{}\n",
            spread(k, history)
        );
    }
    for k in 0..spec.remote_branches {
        let name = if k == 0 {
            "main".to_owned()
        } else {
            format!("feature/team-{}/item-{k}", k % 40)
        };
        let _ = writeln!(
            out,
            "reset refs/remotes/origin/{name}\nfrom :{}\n",
            spread(k.wrapping_add(7_919), history)
        );
    }
    for k in 0..spec.tags {
        let at = (history.saturating_mul(k + 1) / (spec.tags + 1)).max(1);
        let _ = writeln!(
            out,
            "reset refs/tags/v{}.{}.{}\nfrom :{at}\n",
            k / 100,
            (k / 10) % 10,
            k % 10
        );
    }
    out
}

/// Build (or reuse) the repository with many references and worktrees for
/// `spec`; returns its path. Its worktrees are under `.worktrees` in it.
/// Safe to call from several threads and several processes at once.
pub fn build_many_refs(spec: &ManyRefsSpec) -> Result<PathBuf> {
    let _one = BUILDING.lock().unwrap_or_else(PoisonError::into_inner);
    many_refs_in(&corpus_dir(), spec)
}

/// The many-references repository for `spec` under `root`.
fn many_refs_in(root: &Path, spec: &ManyRefsSpec) -> Result<PathBuf> {
    let dir = root.join(format!(
        "many-refs-c{}-f{}-l{}-r{}-t{}-w{}",
        spec.commits,
        spec.files,
        spec.local_branches,
        spec.remote_branches,
        spec.tags,
        spec.worktrees
    ));
    let want = format!("generator {GENERATOR_VERSION} many-refs {MANY_REFS_VERSION}");
    let stamp = |dir: &Path| dir.join(".git").join("corpus-ok");
    let whole = |dir: &Path| fs::read_to_string(stamp(dir)).ok().as_deref() == Some(want.as_str());
    build_in_place(
        &dir,
        &whole,
        &|git| {
            let commits = CorpusSpec {
                commits: many_refs_history(spec),
                lanes: MANY_REFS_LANES,
                merge_every: MANY_REFS_MERGE_EVERY,
                files: spec.files.max(1),
            };
            let mut stream = many_refs_first(spec);
            stream.push_str(&fast_import_stream(&commits));
            stream.push_str(&many_refs_stream(spec));
            fast_import(git, &stream)?;
            git.run([
                "remote",
                "add",
                "origin",
                "https://example.invalid/many-refs.git",
            ])?;
            git.run([
                "symbolic-ref",
                "refs/remotes/origin/HEAD",
                "refs/remotes/origin/main",
            ])?;
            git.run(["checkout", "-q", "-f", "main"])?;
            fs::write(
                git.cwd().join(".git").join("info").join("exclude"),
                format!("/{MANY_REFS_WORKTREES}/\n"),
            )?;
            for k in 0..spec.worktrees {
                let path = git.cwd().join(MANY_REFS_WORKTREES).join(format!("wt-{k}"));
                let path = path
                    .to_str()
                    .with_context(|| format!("{} is not UTF-8", path.display()))?;
                git.run(["worktree", "add", "-q", path, &format!("wt/branch-{k}")])?;
            }
            // What a real repository has after `git gc`.
            let _ = git.run(["commit-graph", "write", "--reachable"]);
            Ok(())
        },
        &|dir| Ok(fs::write(stamp(dir), &want)?),
    )?;
    // Built beside its place and moved there: the worktrees and the
    // repository are told where the others are now.
    let worktrees: Vec<String> = (0..spec.worktrees)
        .map(|k| {
            dir.join(MANY_REFS_WORKTREES)
                .join(format!("wt-{k}"))
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    if !worktrees.is_empty() {
        let git = Git::new(&dir, fixtures_out_dir().join(".home"));
        let mut args = vec!["worktree".to_owned(), "repair".to_owned()];
        args.extend(worktrees);
        git.run(args)?;
    }
    Ok(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_million_commits_are_dated_before_2106_and_small_histories_an_hour_apart() {
        // The last commit of the corpus the benchmarks open: within what a
        // 32-bit unsigned number holds, where Git for Windows' fast-import
        // reads it.
        let commits = CorpusSpec::default().commits;
        let step = seconds_between(commits);
        let last: u64 = synthetic_date(commits, step)
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap();
        assert!(last <= u64::from(u32::MAX), "{last}");
        assert!(step > 60, "{step}");
        // The histories of the fixtures and the tests keep their dates,
        // and so their ids.
        for commits in [61, 1200, 33_000, 700_000] {
            assert_eq!(seconds_between(commits), 3600);
            assert_eq!(synthetic_date(commits, 3600), clock_at(commits));
        }
        assert_eq!(seconds_between(0), 3600);
        assert_eq!(seconds_between(u64::MAX), 1);
    }

    #[test]
    fn a_small_many_references_repository_has_its_references_and_worktrees() {
        let spec = ManyRefsSpec {
            commits: 120,
            files: 30,
            local_branches: 12,
            remote_branches: 9,
            tags: 4,
            worktrees: 2,
        };
        let root = fresh_root("many-refs");
        let dir = many_refs_in(&root, &spec).unwrap();
        let git = Git::new(&dir, fixtures_out_dir().join(".home"));
        assert_eq!(git.rev_list_count("--all").unwrap(), 120);
        // Every file on main, and in the worktrees.
        assert_eq!(git.lines(["ls-files"]).unwrap().len(), 30);
        let count = |prefix: &str| {
            git.lines(["for-each-ref", "--format=%(refname)", prefix])
                .unwrap()
                .len()
        };
        assert_eq!(count("refs/heads"), 12);
        // The remote's branches and its HEAD.
        assert_eq!(count("refs/remotes"), 10);
        assert_eq!(count("refs/tags"), 4);
        let listed = git.lines(["worktree", "list", "--porcelain"]).unwrap();
        let worktrees = listed
            .iter()
            .filter(|line| line.starts_with("worktree "))
            .count();
        assert_eq!(worktrees, 3, "{listed:?}");
        assert!(dir.join(".worktrees/wt-1/.git").is_file());
        // Checked out and clean, the worktrees left out of its status.
        assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "");
        // Asked again, it is the same repository.
        assert_eq!(many_refs_in(&root, &spec).unwrap(), dir);
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn small_synthetic_corpus_has_the_requested_commit_count_and_merges() {
        let spec = CorpusSpec {
            commits: 500,
            lanes: 4,
            merge_every: 10,
            files: 20,
        };
        let dir = build_synthetic(&spec).unwrap();
        let git = Git::new(&dir, fixtures_out_dir().join(".home"));
        let count = git.rev_list_count("--all").unwrap();
        assert_eq!(count, 500);
        let merges = git.lines(["rev-list", "--all", "--merges"]).unwrap();
        assert!(!merges.is_empty(), "expected merge commits");
    }

    /// A place for corpora that no run before this one has built in.
    fn fresh_root(name: &str) -> PathBuf {
        let root = crate::scratch_root().join(format!(
            "corpus-{name}-{}-{}",
            std::process::id(),
            BUILDERS.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&root);
        root
    }

    fn small() -> CorpusSpec {
        CorpusSpec {
            commits: 300,
            lanes: 3,
            merge_every: 7,
            files: 10,
        }
    }

    /// What is in a place for corpora, by name.
    fn held(root: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(root)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// A folder opened so that its times can be set. Windows opens a
    /// folder only when asked to in so many words, and for writing.
    fn open_folder(dir: &Path) -> fs::File {
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            // FILE_FLAG_BACKUP_SEMANTICS: the handle may be a folder's.
            fs::OpenOptions::new()
                .write(true)
                .custom_flags(0x0200_0000)
                .open(dir)
                .unwrap()
        }
        #[cfg(not(windows))]
        {
            fs::File::open(dir).unwrap()
        }
    }

    fn commits(dir: &Path) -> u64 {
        Git::new(dir, fixtures_out_dir().join(".home"))
            .rev_list_count("--all")
            .unwrap()
    }

    #[test]
    fn builders_at_once_end_with_one_whole_corpus() {
        // Each thread builds as a process of its own would: without the
        // lock that makes the builders of one process take turns. They
        // start together, on a place where nothing is built yet.
        let root = fresh_root("race");
        let spec = small();
        let together = std::sync::Barrier::new(6);
        let dirs: Vec<PathBuf> = std::thread::scope(|scope| {
            let builders: Vec<_> = (0..6)
                .map(|_| {
                    scope.spawn(|| {
                        together.wait();
                        synthetic_in(&root, &spec).unwrap()
                    })
                })
                .collect();
            builders
                .into_iter()
                .map(|builder| builder.join().unwrap())
                .collect()
        });
        // All of them were given the same place, and it holds the whole.
        assert!(dirs.iter().all(|dir| *dir == dirs[0]), "{dirs:?}");
        assert_eq!(commits(&dirs[0]), 300);
        assert!(dirs[0].join(".corpus-ok").is_file());
        // Nothing of the builders that lost is left beside it.
        assert_eq!(held(&root), ["synthetic-300-l3-m7-f10"]);
        // And the next one reuses it: the directory is the same one.
        let made = fs::metadata(&dirs[0]).unwrap().modified().unwrap();
        assert_eq!(synthetic_in(&root, &spec).unwrap(), dirs[0]);
        assert_eq!(fs::metadata(&dirs[0]).unwrap().modified().unwrap(), made);
        fs::remove_dir_all(&root).unwrap();
    }

    /// Tests of the interface may ask for one corpus at the same moment in
    /// a target directory that does not have it yet.
    #[test]
    fn a_corpus_asked_for_from_several_threads_at_once_is_built_once() {
        // A shape nothing else asks for, taken away first so that it is
        // built here whatever an earlier run left.
        let spec = CorpusSpec {
            commits: 61,
            lanes: 2,
            merge_every: 4,
            files: 3,
        };
        let dir = corpus_dir().join("synthetic-61-l2-m4-f3");
        let _ = fs::remove_dir_all(&dir);
        let built: Vec<_> = (0..8)
            .map(|_| std::thread::spawn(move || build_synthetic(&spec)))
            .collect();
        for thread in built {
            let path = thread.join().unwrap().unwrap();
            assert_eq!(path, dir);
        }
        let git = Git::new(&dir, fixtures_out_dir().join(".home"));
        assert_eq!(git.rev_list_count("--all").unwrap(), 61);
    }

    #[test]
    fn the_builders_of_one_process_take_turns() {
        // Through the door every caller uses, from several threads, as the
        // tests of the interface that share one corpus do: none may find
        // it half built.
        let spec = CorpusSpec {
            commits: 240,
            lanes: 3,
            merge_every: 5,
            files: 8,
        };
        let together = std::sync::Barrier::new(4);
        let counts: Vec<u64> = std::thread::scope(|scope| {
            let builders: Vec<_> = (0..4)
                .map(|_| {
                    scope.spawn(|| {
                        together.wait();
                        commits(&build_synthetic(&spec).unwrap())
                    })
                })
                .collect();
            builders
                .into_iter()
                .map(|builder| builder.join().unwrap())
                .collect()
        });
        assert_eq!(counts, [240, 240, 240, 240]);
    }

    #[test]
    fn what_is_half_built_or_old_is_not_taken_for_whole() {
        let root = fresh_root("half");
        let spec = small();
        let dir = root.join("synthetic-300-l3-m7-f10");
        // Where the corpus belongs, a directory with no mark.
        fs::create_dir_all(dir.join(".git")).unwrap();
        fs::write(
            dir.join(".git/HEAD"),
            "ref: refs/heads/main
",
        )
        .unwrap();
        assert_eq!(synthetic_in(&root, &spec).unwrap(), dir);
        assert_eq!(commits(&dir), 300);
        // Made by another generator: built again.
        fs::write(dir.join(".corpus-ok"), "generator 0").unwrap();
        fs::write(dir.join("left-over"), "of the old one").unwrap();
        synthetic_in(&root, &spec).unwrap();
        assert!(!dir.join("left-over").exists());
        assert_eq!(commits(&dir), 300);
        // A file where the directory belongs is moved away like the rest.
        fs::remove_dir_all(&dir).unwrap();
        fs::write(&dir, "not a directory").unwrap();
        synthetic_in(&root, &spec).unwrap();
        assert_eq!(commits(&dir), 300);
        assert_eq!(held(&root), ["synthetic-300-l3-m7-f10"]);

        // What a builder left beside the corpus goes when it is old, and
        // stays while it may still be at work.
        let young = root.join(format!("synthetic-1{BUILDING_MARK}1-1"));
        let old = root.join(format!("synthetic-2{BUILDING_MARK}1-2"));
        let stale = root.join(format!("synthetic-2{STALE_MARK}1-3"));
        for left in [&young, &old, &stale] {
            fs::create_dir_all(left).unwrap();
        }
        let long_ago = SystemTime::now() - LEFT_BEHIND - Duration::from_secs(60);
        for left in [&old, &stale] {
            open_folder(left).set_modified(long_ago).unwrap();
        }
        sweep(&root);
        assert!(young.is_dir() && !old.exists() && !stale.exists());
        // A place that is not there is nothing to sweep.
        sweep(&root.join("nowhere"));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn a_corpus_that_cannot_be_built_leaves_nothing_behind() {
        let root = fresh_root("fails");
        fs::create_dir_all(&root).unwrap();
        let dir = root.join("never");
        let failed = build_in_place(
            &dir,
            &|dir| dir.join("mark").is_file(),
            &|_| bail!("no such generator"),
            &|dir| Ok(fs::write(dir.join("mark"), "")?),
        );
        assert!(failed.is_err());
        assert!(held(&root).is_empty(), "{:?}", held(&root));
        // No place at all.
        assert!(build_in_place(Path::new("/"), &|_| false, &|_| Ok(()), &|_| Ok(())).is_err());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_wide_corpus_is_marked_under_git_and_its_worktree_is_clean() {
        let root = fresh_root("wide");
        let dir = wide_in(&root, 40).unwrap();
        let git = Git::new(&dir, fixtures_out_dir().join(".home"));
        assert_eq!(git.lines(["ls-files"]).unwrap().len(), 40);
        assert!(git.lines(["status", "--porcelain"]).unwrap().is_empty());
        assert_eq!(held(&root), ["wide-40"]);
        // One that is marked is taken as it is.
        fs::write(dir.join("untracked"), "proof that it was not rebuilt").unwrap();
        assert_eq!(wide_in(&root, 40).unwrap(), dir);
        assert!(dir.join("untracked").is_file());
        assert_eq!(held(&root), ["wide-40"]);
        // Without its mark it is built again.
        fs::remove_file(dir.join(".git/corpus-ok")).unwrap();
        wide_in(&root, 40).unwrap();
        assert!(!dir.join("untracked").exists());
        fs::remove_dir_all(&root).unwrap();
    }
}
