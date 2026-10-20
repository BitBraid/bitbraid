//! Opening repositories.

use std::{
    fmt,
    path::{Path, PathBuf},
};

use serde::Serialize;

use crate::{Error, Oid, Result};

/// An open repository. Wraps a gitoxide repository; not `Send` (one per thread).
pub struct Repository {
    inner: gix::Repository,
    /// Which choice of git executable it was opened under
    /// (`exe::git_choice`): its configuration was read with that git's
    /// system configuration.
    choice: u64,
    /// When the repository's own configuration files were last written,
    /// as they were when it was opened: gitoxide reads them once, and what
    /// git writes into them later (a remote added, an address changed) is
    /// only seen by a handle opened again.
    settings_stamp: Vec<Option<(std::time::SystemTime, u64)>>,
}

impl fmt::Debug for Repository {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Repository")
            .field("git_dir", &self.inner.git_dir())
            .finish_non_exhaustive()
    }
}

/// What the UI shows in a tab header: where the repository is and what HEAD is.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct RepoSummary {
    /// The `.git` directory (or the bare repository directory).
    #[serde(serialize_with = "crate::path_text")]
    pub git_dir: PathBuf,
    /// The working directory, absent for bare repositories.
    #[serde(serialize_with = "crate::opt_path_text")]
    pub workdir: Option<PathBuf>,
    /// Whether the repository is bare.
    pub bare: bool,
    /// Current branch short name, absent when HEAD is detached or unborn.
    pub head_branch: Option<String>,
    /// Commit HEAD points at, absent when unborn.
    pub head: Option<Oid>,
    /// Whether HEAD is detached.
    pub detached: bool,
}

impl Repository {
    /// Open the repository containing `path`, walking up like `git` does.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        // Read before the git is asked: a choice made while this opens is
        // seen the next time somebody looks.
        let choice = crate::exe::git_choice();
        // With the system configuration of the git that is run, not of
        // whatever git gitoxide would think of (`crate::system_config`):
        // inside a WSL distribution, the distribution's.
        #[cfg(windows)]
        let options = match bitbraid_core::wsl::WslPath::of(path) {
            Some(inside) => crate::system_config::wsl_trust_map(&inside),
            None => crate::system_config::trust_map(),
        };
        #[cfg(not(windows))]
        let options = crate::system_config::trust_map();
        let inner = gix::ThreadSafeRepository::discover_opts(path, Default::default(), options)
            .map_err(|err| Error::Open {
                path: path.to_path_buf(),
                source: if found_none(&err) {
                    Box::new(NoRepository)
                } else if let Some(length) = too_long_for_git(path, cfg!(windows)) {
                    Box::new(FolderTooLong { length })
                } else {
                    Box::new(err)
                },
            })?
            .to_thread_local();
        let settings_stamp = settings_stamp(&inner);
        Ok(Self {
            inner,
            choice,
            settings_stamp,
        })
    }

    /// Whether the configuration this handle holds was read under the git
    /// that is run now. False once another executable was chosen.
    pub fn follows_git(&self) -> bool {
        self.choice == crate::exe::git_choice()
    }

    /// Follow the git that is run: when another executable was chosen
    /// since this handle was opened, open the repository again where it
    /// is, so that what gitoxide reads (the system configuration of that
    /// git among it) is what the git children read. Says whether it was
    /// opened again. Costs a comparison when nothing changed.
    ///
    /// The same when the repository's own configuration was written since
    /// (by git, which the commands run for what they set): the handle
    /// would go on reading what it said before.
    ///
    /// When it cannot be opened again the handle stays as it was, and it
    /// is not tried again until the next choice.
    pub fn follow_git(&mut self) -> Result<bool> {
        let now = crate::exe::git_choice();
        if self.choice == now && self.settings_stamp == settings_stamp(&self.inner) {
            return Ok(false);
        }
        match self.reopen() {
            Ok(()) => Ok(true),
            Err(err) => {
                self.choice = now;
                self.settings_stamp = settings_stamp(&self.inner);
                Err(err)
            }
        }
    }

    /// Open the repository again where it is, in place of this handle:
    /// with the configuration as it is now, and with room for the packs
    /// that are on disk now ([`is_out_of_room`]). When it cannot be opened
    /// again the handle stays as it was.
    pub fn reopen(&mut self) -> Result<()> {
        let at = self
            .workdir()
            .unwrap_or_else(|| self.git_dir())
            .to_path_buf();
        // Opening walks up as git does, so a repository whose own `.git`
        // is gone or broken would be found as the one around it: only the
        // same repository may take this handle's place.
        let opened = Self::open(&at)?;
        if !same_git_dir(opened.git_dir(), self.git_dir()) {
            return Err(Error::Refused {
                what: say!("repo-what-reopen"),
                detail: say!(
                    "repo-no-longer-readable",
                    path = at.display().to_string(),
                    found = opened.git_dir().display().to_string()
                ),
            });
        }
        *self = opened;
        Ok(())
    }

    /// The gitoxide handle.
    pub fn gix(&self) -> &gix::Repository {
        &self.inner
    }

    /// The `.git` directory.
    pub fn git_dir(&self) -> &Path {
        self.inner.git_dir()
    }

    /// The working directory, absent for bare repositories.
    pub fn workdir(&self) -> Option<&Path> {
        self.inner.workdir()
    }

    /// Whether the repository is bare: it has no working directory.
    ///
    /// A linked worktree of a bare repository is not bare. It shares the
    /// configuration that says `core.bare`, which is all gitoxide asks,
    /// but it has files, and git itself answers `false` for it
    /// (git-rev-parse(1) `--is-bare-repository`, git-worktree(1)).
    pub fn is_bare(&self) -> bool {
        self.inner.is_bare() && self.inner.workdir().is_none()
    }

    /// Summary for tab headers and the CLI.
    pub fn summary(&self) -> Result<RepoSummary> {
        let head = self
            .inner
            .head()
            .map_err(crate::git_err(say!("repo-reading-head")))?;
        Ok(RepoSummary {
            git_dir: self.git_dir().to_path_buf(),
            workdir: self.workdir().map(Path::to_path_buf),
            bare: self.is_bare(),
            head_branch: head.referent_name().map(|name| name.shorten().to_string()),
            head: head.id().map(|id| Oid::from(id.detach())),
            detached: head.is_detached(),
        })
    }
}

/// What gitoxide says when the object database of an open repository has
/// no room left for the packs on disk (gix-odb's `load_index`, a message
/// and not an error of a type of its own).
const OUT_OF_ROOM: &str = "too few index slots";

/// Whether an error, as it is said, is the object database of an open
/// repository running out of room for the packs on disk: more packs were
/// added while it was open than it was given room for
/// ([`crate::system_config::OBJECT_STORE_SLOTS`]). Nothing is wrong with
/// the repository: opened again ([`Repository::reopen`]) it has room for
/// what is there, and the read can be tried once more.
pub fn is_out_of_room(said: &str) -> bool {
    said.contains(OUT_OF_ROOM)
}

/// There is no repository where one was looked for, nor above it.
#[derive(Debug, thiserror::Error)]
#[error("{}", say!("repo-no-repository"))]
struct NoRepository;

/// The longest folder a repository or worktree works from on Windows, in
/// UTF-16 units, long paths on or not: its `.git`, five more, has to be
/// within the 259 Windows takes, for git and for gitoxide alike (a folder
/// grown longer after its worktree was made is read by neither).
const WINDOWS_REPOSITORY_FOLDER_LIMIT: usize = 254;

/// What was asked to be opened is in a folder longer than git works in on
/// Windows.
#[derive(Debug, thiserror::Error)]
#[error("{}", say!("repo-folder-too-long-for-windows", length = *.length, most = WINDOWS_REPOSITORY_FOLDER_LIMIT))]
struct FolderTooLong {
    length: usize,
}

/// How long `path` is when that is longer than git works in on Windows
/// (`windows`); None otherwise, and elsewhere.
fn too_long_for_git(path: &Path, windows: bool) -> Option<usize> {
    if !windows {
        return None;
    }
    let text = path.to_string_lossy();
    let plain = text.strip_prefix(r"\\?\").unwrap_or(&text);
    let length = bitbraid_core::paths::windows_length(plain);
    (length > WINDOWS_REPOSITORY_FOLDER_LIMIT).then_some(length)
}

/// Whether looking for a repository found none. Gitoxide says so in
/// three ways, by where it stopped looking (the root, a ceiling, the edge
/// of a file system), which depend on how the machine's disks are mounted
/// and are of no use to whoever asked.
fn found_none(err: &gix::Error) -> bool {
    use gix::discover::upwards::Error as Upwards;
    matches!(
        err.downcast_any_ref::<Upwards>(),
        Some(
            Upwards::NoGitRepository { .. }
                | Upwards::NoGitRepositoryWithinCeiling { .. }
                | Upwards::NoGitRepositoryWithinFs { .. }
        )
    )
}

/// [`is_out_of_room`] for an error or any error it came from.
pub fn came_from_out_of_room(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(err);
    while let Some(err) = next {
        if is_out_of_room(&err.to_string()) {
            return true;
        }
        next = err.source();
    }
    false
}

/// Whether two `.git` directories are the same one, however they were
/// written down.
fn same_git_dir(a: &Path, b: &Path) -> bool {
    match (
        bitbraid_core::paths::canonical(a),
        bitbraid_core::paths::canonical(b),
    ) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// When the configuration files of a repository were last written, and
/// how long they are: `config` in its common directory, and a worktree's
/// own `config.worktree`. A file that is not there is `None`.
fn settings_stamp(repo: &gix::Repository) -> Vec<Option<(std::time::SystemTime, u64)>> {
    [
        repo.common_dir().join("config"),
        repo.git_dir().join("config.worktree"),
    ]
    .iter()
    .map(|file| {
        let meta = std::fs::metadata(file).ok()?;
        Some((meta.modified().ok()?, meta.len()))
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folder_longer_than_git_works_in_on_windows_is_told_there_alone() {
        let folder = |length: usize| {
            let base = r"D:\work\";
            PathBuf::from(format!("{base}{}", "w".repeat(length - base.len())))
        };
        assert_eq!(too_long_for_git(&folder(254), true), None);
        assert_eq!(too_long_for_git(&folder(255), true), Some(255));
        assert_eq!(too_long_for_git(&folder(400), false), None);
        let verbatim = PathBuf::from(format!(r"\\?\{}", folder(254).display()));
        assert_eq!(too_long_for_git(&verbatim, true), None);
        let said = FolderTooLong { length: 270 }.to_string();
        assert!(said.contains("270 characters"), "{said}");
        assert!(said.contains("254"), "{said}");
    }
}
