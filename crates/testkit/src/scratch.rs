//! Mutable copies of fixtures for tests that change the working tree or index.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use anyhow::{Context, Result};

use crate::{Git, build_fixture, fixtures_out_dir, scratch_root};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A private copy of a fixture, removed on drop.
#[derive(Debug)]
pub struct Scratch {
    path: PathBuf,
}

impl Scratch {
    /// Copy the fixture `name` into a fresh directory.
    pub fn of(name: &str) -> Result<Self> {
        let source = build_fixture(name)?;
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = scratch_root().join(format!("{name}-{}-{n}", std::process::id()));
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        copy_dir(&source, &path).with_context(|| format!("copying fixture {name}"))?;
        Ok(Self { path })
    }

    /// Working directory of the copy.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// A hermetic git runner for the copy.
    pub fn git(&self) -> Git {
        let git = Git::new(&self.path, fixtures_out_dir().join(".home"));
        let _ = git.ensure_home();
        git
    }

    /// Write a file relative to the worktree, creating parents.
    pub fn write(&self, rel: &str, content: &str) -> Result<()> {
        let path = self.path.join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&path, content).with_context(|| format!("writing {}", path.display()))
    }

    /// Remove a file relative to the worktree.
    pub fn remove(&self, rel: &str) -> Result<()> {
        let path = self.path.join(rel);
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))
    }
}

/// A directory of its own for Unix sockets, for this user alone, removed
/// on drop. A socket's path is at most 104 bytes on macOS (108 on Linux),
/// and macOS's temporary folder, where the scratch root is, leaves too
/// little of that for a test's folder and a socket's name: there it is in
/// `/tmp`, elsewhere in the scratch root.
#[derive(Debug)]
pub struct SocketDir {
    path: PathBuf,
}

impl SocketDir {
    /// A fresh one, named after `what`.
    pub fn new(what: &str) -> Result<Self> {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let name = format!("bb-{what}-{}-{n}", std::process::id());
        let path = if cfg!(target_os = "macos") {
            PathBuf::from("/tmp").join(name)
        } else {
            scratch_root().join(name)
        };
        if path.exists() {
            fs::remove_dir_all(&path)?;
        }
        fs::create_dir_all(&path).with_context(|| format!("making {}", path.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
        }
        Ok(Self { path })
    }

    /// Where it is.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for SocketDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

/// Copy the fixture `name` to `to`, for a test that needs its copy at a
/// place of its own choosing (the off-screen tests draw the path). What
/// is at `to` is replaced; the caller removes the copy.
pub fn copy_fixture(name: &str, to: &Path) -> Result<()> {
    let source = build_fixture(name)?;
    if to.exists() {
        fs::remove_dir_all(to)?;
    }
    copy_dir(&source, to).with_context(|| format!("copying fixture {name}"))
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    fs::create_dir_all(to)?;
    for entry in fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            fs::copy(entry.path(), &target)?;
        }
        // Symlinks are not part of any fixture; skip them deliberately.
    }
    Ok(())
}
