//! Filesystem watcher: debounced notifications when references, the index or
//! the working tree change, so views refresh without polling.
//!
//! Where a watch is per directory (inotify), directories git ignores are
//! left out: build output and installed dependencies can be most of a tree,
//! cost a watch each and report changes nobody needs to hear about. Where
//! one watch takes the whole tree (Windows, macOS), what changes in them is
//! passed over instead. When the system says that it may have missed
//! changes, or a watch fails, everything is read again.
//!
//! Sources: notify crate documentation (`Event::need_rescan`); inotify(7);
//! Microsoft Learn, "ReadDirectoryChangesW"; Apple's File System Events
//! Programming Guide (events name real paths); gitrepository-layout(5)
//! for which paths mean what; gitignore(5); gix documentation (exclude
//! stack).

use std::{
    path::{Path, PathBuf},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use notify::{RecursiveMode, Watcher as _};
use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// What changed, coarsely.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum Area {
    /// References, HEAD or packed-refs.
    Refs,
    /// The index.
    Index,
    /// Files in the working tree.
    Worktree,
    /// Something else under `.git` (objects, config, logs, ...).
    GitDir,
    /// What another worktree keeps of its own under `.git/worktrees/<name>`
    /// other than its HEAD (which is a reference here too) and what says
    /// where and whether it is: its index, its logs, its operations. What
    /// is uncommitted there may have changed; nothing of this one has.
    OtherWorktree,
    /// The object database alone: objects written by `git add` anywhere,
    /// packs a partial clone fetched, a repack. No reference moved with
    /// them (that is [`Area::Refs`]), so nothing that is shown changed.
    Objects,
}

/// A debounced batch of changes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ChangeSet {
    /// Areas touched, sorted and unique.
    pub areas: Vec<Area>,
    /// Up to 64 worktree-relative paths, for targeted refreshes.
    pub paths: Vec<PathBuf>,
}

/// Classify a changed path.
pub fn classify(git_dir: &Path, workdir: Option<&Path>, path: &Path) -> Option<Area> {
    if let Ok(rel) = path.strip_prefix(git_dir) {
        let mut parts = rel
            .components()
            .map(|part| part.as_os_str().to_string_lossy());
        let first = parts.next()?;
        return Some(match first.as_ref() {
            "HEAD" | "ORIG_HEAD" | "FETCH_HEAD" | "MERGE_HEAD" | "refs" | "packed-refs"
            | "logs" => Area::Refs,
            "index" => Area::Index,
            "objects" => Area::Objects,
            // Another worktree's own files: git, and the editors and
            // tools working there, write its index and logs all the
            // time, and none of that changes this one's references or
            // history. Its HEAD does (what is checked out where), and so
            // do what says where it is and whether it is locked, and the
            // folder itself coming or going.
            "worktrees" => match (parts.next(), parts.next()) {
                (Some(_), Some(file)) => match file.as_ref() {
                    "HEAD" => Area::Refs,
                    "gitdir" | "commondir" | "locked" => Area::GitDir,
                    _ => Area::OtherWorktree,
                },
                _ => Area::GitDir,
            },
            _ => Area::GitDir,
        });
    }
    if let Some(workdir) = workdir
        && path.strip_prefix(workdir).is_ok()
    {
        return Some(Area::Worktree);
    }
    None
}

/// Whether an event kind means something was written, created or removed.
/// Access events (open, read, close) are reported by inotify and must be
/// ignored, or reading references would look like a change and loop.
pub fn is_mutation(kind: &notify::EventKind) -> bool {
    use notify::EventKind;
    match kind {
        EventKind::Access(_) => false,
        EventKind::Any
        | EventKind::Create(_)
        | EventKind::Modify(_)
        | EventKind::Remove(_)
        | EventKind::Other => true,
    }
}

/// Upper bound on directory watches, so that a hostile or merely enormous
/// tree cannot use up the user's inotify allowance.
#[cfg(target_os = "linux")]
const MAX_DIRECTORY_WATCHES: usize = 65_536;

/// Directories from `from` down (itself included) that are worth a watch:
/// all but the ones git ignores, unless the index has files in them, and
/// never a `.git`. Symbolic links are not followed.
#[cfg(target_os = "linux")]
fn worth_watching(workdir: &Path, from: &Path) -> Vec<PathBuf> {
    // Without a readable repository there are no ignore rules: watch it all.
    let repo = gix::open_opts(workdir, crate::system_config::open_options()).ok();
    let index = repo.as_ref().and_then(|repo| repo.index_or_empty().ok());
    let mut excludes = match (&repo, &index) {
        (Some(repo), Some(index)) => repo.excludes(index, None, Default::default()).ok(),
        _ => None,
    };
    let mut out = Vec::new();
    let mut pending = vec![from.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if out.len() >= MAX_DIRECTORY_WATCHES {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                continue;
            }
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let Ok(relative) = path.strip_prefix(workdir) else {
                continue;
            };
            if let (Some(excludes), Some(index)) = (&mut excludes, &index)
                && ignored_dir(excludes, index, relative)
            {
                continue;
            }
            pending.push(path);
        }
        out.push(dir);
    }
    out
}

/// Whether a directory of the worktree, relative to it, is one git ignores
/// and the index has no files in.
fn ignored_dir(
    excludes: &mut gix::AttributeStack<'_>,
    index: &gix::index::State,
    relative: &Path,
) -> bool {
    use gix::bstr::ByteSlice as _;
    let ignored = excludes
        .at_path(relative, Some(gix::index::entry::Mode::DIR))
        .is_ok_and(|platform| platform.is_excluded());
    ignored
        && !gix::path::into_bstr(relative).is_ok_and(|relative| {
            // The index has `/` between folders, on Windows too.
            let mut prefix = gix::path::to_unix_separators_on_windows(relative).into_owned();
            prefix.push(b'/');
            index
                .prefixed_entries(prefix.as_bstr())
                .is_some_and(|entries| !entries.is_empty())
        })
}

/// The directories git ignores, for the one recursive watch of the systems
/// that have no watch per directory (Windows, macOS): a change in a
/// directory git ignores (build output, installed dependencies) is no
/// change to hear about, unless the index has files there. What each
/// directory is is remembered, and forgotten when an ignore file or the
/// index changes.
#[cfg_attr(target_os = "linux", allow(dead_code))]
struct Ignored {
    workdir: PathBuf,
    known: std::collections::HashMap<PathBuf, bool>,
}

#[cfg_attr(target_os = "linux", allow(dead_code))]
impl Ignored {
    /// The most directories remembered; past that it starts again.
    const MOST: usize = 16_384;

    fn new(workdir: &Path) -> Self {
        Self {
            workdir: workdir.to_path_buf(),
            known: std::collections::HashMap::new(),
        }
    }

    fn forget(&mut self) {
        self.known.clear();
    }

    /// Whether `path` is in a directory git ignores: the first directory
    /// on its way down that git ignores and the index has no files in.
    /// Without a readable repository nothing is ignored.
    fn ignores(&mut self, path: &Path) -> bool {
        let Ok(relative) = path.strip_prefix(&self.workdir) else {
            return false;
        };
        // The path itself too when it is a directory: Windows says that a
        // directory changed whenever what is in it does.
        let mut dirs: Vec<PathBuf> = relative
            .ancestors()
            .skip(usize::from(!path.is_dir()))
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(Path::to_path_buf)
            .collect();
        dirs.reverse();
        if dirs.iter().all(|dir| self.known.contains_key(dir)) {
            return dirs.iter().any(|dir| self.known.get(dir) == Some(&true));
        }
        let Ok(repo) = gix::open_opts(&self.workdir, crate::system_config::open_options()) else {
            return false;
        };
        let Ok(index) = repo.index_or_empty() else {
            return false;
        };
        let Ok(mut excludes) = repo.excludes(&index, None, Default::default()) else {
            return false;
        };
        if self.known.len() >= Self::MOST {
            self.known.clear();
        }
        for dir in dirs {
            let ignored = *self
                .known
                .entry(dir.clone())
                .or_insert_with(|| ignored_dir(&mut excludes, &index, &dir));
            if ignored {
                return true;
            }
        }
        false
    }
}

/// Watch the worktree. One recursive watch where the platform has such a
/// thing; a watch per directory worth watching on Linux.
fn watch_worktree(inner: &mut notify::RecommendedWatcher, workdir: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let mut watching = false;
        for dir in worth_watching(workdir, workdir) {
            watching |= inner.watch(&dir, RecursiveMode::NonRecursive).is_ok();
        }
        watching
    }
    #[cfg(not(target_os = "linux"))]
    {
        inner.watch(workdir, RecursiveMode::Recursive).is_ok()
    }
}

/// Keep the per-directory watches up to date: a directory that appears gets
/// its watches, and a changed `.gitignore` may have brought directories
/// back into view.
#[cfg(target_os = "linux")]
fn follow_new_directories(
    inner: &mut notify::RecommendedWatcher,
    workdir: &Path,
    event: &notify::Event,
) {
    for path in &event.paths {
        if !path.starts_with(workdir) {
            continue;
        }
        let from = if path.file_name().is_some_and(|name| name == ".gitignore") {
            workdir
        } else if matches!(event.kind, notify::EventKind::Create(_))
            && std::fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir())
        {
            path.as_path()
        } else {
            continue;
        };
        for dir in worth_watching(workdir, from) {
            let _ = inner.watch(&dir, RecursiveMode::NonRecursive);
        }
    }
}

/// The path as the system resolves it, or as it is where it cannot be.
#[cfg(target_os = "macos")]
fn resolved(path: PathBuf) -> PathBuf {
    std::fs::canonicalize(&path).unwrap_or(path)
}

/// A running watcher; dropping it stops the thread.
pub struct Watcher {
    stop: mpsc::Sender<()>,
}

impl std::fmt::Debug for Watcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Watcher")
    }
}

impl Watcher {
    /// Watch `git_dir` and `workdir`, delivering debounced change sets to
    /// `on_change` from a background thread.
    ///
    /// Returns at once. The watches are put in place on that thread, since
    /// registering one per directory takes a noticeable time on a big tree
    /// (half a second for twenty thousand directories). Nothing that happens
    /// meanwhile is seen, so the first change set, delivered when the
    /// watches stand, names the references, the index, the worktree and the
    /// git directory: whoever read the repository before that reads it
    /// again.
    pub fn spawn(
        git_dir: &Path,
        workdir: Option<&Path>,
        debounce: Duration,
        on_change: impl Fn(ChangeSet) + Send + 'static,
    ) -> Result<Self> {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        let git_dir = git_dir.to_path_buf();
        let workdir = workdir.map(Path::to_path_buf);
        // FSEvents names a changed file as the system resolves its path:
        // through a link on the way (macOS's `/tmp` and `/var` are links
        // into `/private`) the events name the place the link leads to,
        // and nothing would be classified as this repository's.
        #[cfg(target_os = "macos")]
        let (git_dir, workdir) = (resolved(git_dir), workdir.map(resolved));
        thread::Builder::new()
            .name("bitbraid-watch".into())
            .spawn(move || {
                let (raw_tx, raw_rx) = mpsc::channel::<notify::Result<notify::Event>>();
                let Ok(mut inner) = notify::recommended_watcher(move |event| {
                    let _ = raw_tx.send(event);
                }) else {
                    return;
                };
                let mut watching = inner.watch(&git_dir, RecursiveMode::Recursive).is_ok();
                if let Some(workdir) = &workdir {
                    watching |= watch_worktree(&mut inner, workdir);
                }
                if !watching {
                    return;
                }
                on_change(ChangeSet {
                    areas: vec![Area::Refs, Area::Index, Area::Worktree, Area::GitDir],
                    paths: Vec::new(),
                });

                let mut pending = ChangeSet::default();
                let mut deadline: Option<Instant> = None;
                #[cfg(not(target_os = "linux"))]
                let mut ignored = workdir.as_deref().map(Ignored::new);
                // Everything is read again when changes may have been lost.
                let everything = |pending: &mut ChangeSet| {
                    for area in [Area::Refs, Area::Index, Area::Worktree, Area::GitDir] {
                        if !pending.areas.contains(&area) {
                            pending.areas.push(area);
                        }
                    }
                };
                loop {
                    if stop_rx.try_recv().is_ok() {
                        break;
                    }
                    let wait = deadline
                        .map(|d| d.saturating_duration_since(Instant::now()))
                        .unwrap_or(Duration::from_millis(500));
                    match raw_rx.recv_timeout(wait) {
                        Ok(Ok(event)) => {
                            if !is_mutation(&event.kind) {
                                continue; // reads (access/open/close) are not changes
                            }
                            // The system says it may have missed some.
                            if event.need_rescan() {
                                everything(&mut pending);
                            }
                            #[cfg(target_os = "linux")]
                            if let Some(workdir) = &workdir {
                                follow_new_directories(&mut inner, workdir, &event);
                            }
                            for path in event.paths {
                                if let Some(area) = classify(&git_dir, workdir.as_deref(), &path) {
                                    #[cfg(not(target_os = "linux"))]
                                    if let Some(ignored) = &mut ignored {
                                        let rules = area == Area::Index
                                            || path.file_name().is_some_and(|name| {
                                                name == ".gitignore" || name == "exclude"
                                            });
                                        if rules {
                                            ignored.forget();
                                        }
                                        if area == Area::Worktree && ignored.ignores(&path) {
                                            continue;
                                        }
                                    }
                                    if !pending.areas.contains(&area) {
                                        pending.areas.push(area);
                                    }
                                    if area == Area::Worktree
                                        && pending.paths.len() < 64
                                        && let Some(wd) = &workdir
                                        && let Ok(rel) = path.strip_prefix(wd)
                                    {
                                        pending.paths.push(rel.to_path_buf());
                                    }
                                }
                            }
                            if !pending.areas.is_empty() && deadline.is_none() {
                                deadline = Some(Instant::now() + debounce);
                            }
                        }
                        // A watch that failed may have missed changes.
                        Ok(Err(_)) => {
                            everything(&mut pending);
                            if deadline.is_none() {
                                deadline = Some(Instant::now() + debounce);
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                    if deadline.is_some_and(|d| Instant::now() >= d) {
                        pending.areas.sort();
                        on_change(std::mem::take(&mut pending));
                        deadline = None;
                    }
                }
            })
            .map_err(|err| Error::Git {
                context: say!("watch-spawning-thread"),
                source: Box::new(err),
            })?;
        Ok(Self { stop: stop_tx })
    }
}

impl Drop for Watcher {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_events_are_not_mutations() {
        use notify::event::{AccessKind, CreateKind, ModifyKind};
        assert!(!is_mutation(&notify::EventKind::Access(AccessKind::Read)));
        assert!(!is_mutation(&notify::EventKind::Access(AccessKind::Open(
            notify::event::AccessMode::Any
        ))));
        assert!(is_mutation(&notify::EventKind::Modify(ModifyKind::Any)));
        assert!(is_mutation(&notify::EventKind::Create(CreateKind::File)));
        assert!(is_mutation(&notify::EventKind::Remove(
            notify::event::RemoveKind::Any
        )));
    }

    #[test]
    fn classifies_git_dir_and_worktree_paths() {
        let git_dir = Path::new("/r/.git");
        let wd = Path::new("/r");
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/.git/refs/heads/main")),
            Some(Area::Refs)
        );
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/.git/HEAD")),
            Some(Area::Refs)
        );
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/.git/index")),
            Some(Area::Index)
        );
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/.git/objects/ab/cd")),
            Some(Area::Objects)
        );
        assert_eq!(
            classify(
                git_dir,
                Some(wd),
                Path::new("/r/.git/objects/pack/pack-1.promisor")
            ),
            Some(Area::Objects)
        );
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/.git/config")),
            Some(Area::GitDir)
        );
        assert_eq!(
            classify(git_dir, Some(wd), Path::new("/r/src/main.rs")),
            Some(Area::Worktree)
        );
        assert_eq!(classify(git_dir, Some(wd), Path::new("/elsewhere")), None);
    }

    #[test]
    fn another_worktrees_own_files_are_told_apart() {
        let git_dir = Path::new("/r/.git");
        let wd = Path::new("/r");
        let of = |path: &str| classify(git_dir, Some(wd), Path::new(path));
        // What git writes there all the time: nothing of this worktree.
        for path in [
            "/r/.git/worktrees/wt/index",
            "/r/.git/worktrees/wt/index.lock",
            "/r/.git/worktrees/wt/logs/HEAD",
            "/r/.git/worktrees/wt/ORIG_HEAD",
            "/r/.git/worktrees/wt/rebase-merge/done",
        ] {
            assert_eq!(of(path), Some(Area::OtherWorktree), "{path}");
        }
        // What it has checked out is a reference here too.
        assert_eq!(of("/r/.git/worktrees/wt/HEAD"), Some(Area::Refs));
        // Where it is, whether it is locked, and its coming and going.
        for path in [
            "/r/.git/worktrees/wt/gitdir",
            "/r/.git/worktrees/wt/locked",
            "/r/.git/worktrees/wt",
            "/r/.git/worktrees",
        ] {
            assert_eq!(of(path), Some(Area::GitDir), "{path}");
        }
    }

    #[test]
    fn a_change_where_git_ignores_is_not_heard_unless_the_index_has_files_there() {
        let scratch = bitbraid_testkit::Scratch::of("linear").unwrap();
        let wd = scratch.path().to_path_buf();
        scratch
            .write(
                ".gitignore",
                "target/
build/
",
            )
            .unwrap();
        scratch.write("target/debug/out.o", "o").unwrap();
        scratch.write("build/kept.txt", "kept").unwrap();
        scratch
            .git()
            .run(["add", "-f", "--", "build/kept.txt"])
            .unwrap();
        let mut ignored = Ignored::new(&wd);
        assert!(ignored.ignores(&wd.join("target").join("debug").join("out.o")));
        assert!(ignored.ignores(&wd.join("target").join("new")));
        assert!(!ignored.ignores(&wd.join("a.txt")));
        assert!(!ignored.ignores(&wd.join("src").join("lib.rs")));
        // The index has a file there: what changes there may matter.
        assert!(!ignored.ignores(&wd.join("build").join("new.o")));
        // What is not in the worktree is nothing of it.
        assert!(!ignored.ignores(Path::new("/elsewhere/target/x")));
        // The rules change, and are read again once forgotten.
        scratch
            .write(
                ".gitignore",
                "build/
",
            )
            .unwrap();
        assert!(ignored.ignores(&wd.join("target").join("x")), "remembered");
        ignored.forget();
        assert!(!ignored.ignores(&wd.join("target").join("x")));
    }
}
