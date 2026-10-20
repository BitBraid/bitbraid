//! The git CLI as an oracle: a hermetic runner around the `git` binary.
//!
//! All configuration comes from a private home directory so the user's own
//! `~/.gitconfig`, hooks templates, signing settings and locale cannot leak
//! into tests. Dates and identities are fixed by the caller through
//! [`Git::run_with`].

use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result, bail};

/// First fixture identity (even-numbered commits).
pub const AUTHOR_A: (&str, &str) = ("Ada Fixture", "ada@fixture.invalid");
/// Second fixture identity (odd-numbered commits).
pub const AUTHOR_B: (&str, &str) = ("Bob Fixture", "bob@fixture.invalid");

/// Epoch of the fixture clock, 2020-01-01T00:00:00Z, as unix seconds.
pub const CLOCK_EPOCH: i64 = 1_577_836_800;

/// Identity to use for the `n`-th commit of a fixture.
pub fn author_for(n: u64) -> (&'static str, &'static str) {
    if n.is_multiple_of(2) {
        AUTHOR_A
    } else {
        AUTHOR_B
    }
}

/// Git date string (raw format) for tick `n` of the fixture clock: one hour per tick.
pub fn clock_at(n: u64) -> String {
    let secs =
        CLOCK_EPOCH.saturating_add(i64::try_from(n).unwrap_or(i64::MAX).saturating_mul(3600));
    format!("{secs} +0000")
}

/// The git executable to use: `$BITBRAID_GIT` or `git` on `PATH`.
pub fn git_binary() -> OsString {
    std::env::var_os("BITBRAID_GIT").unwrap_or_else(|| OsString::from("git"))
}

/// A hermetic git runner bound to one working directory.
#[derive(Debug, Clone)]
pub struct Git {
    cwd: PathBuf,
    home: PathBuf,
}

impl Git {
    /// A runner for `cwd` whose global and system configuration are replaced
    /// by `<home>/.gitconfig`. Call [`Git::ensure_home`] once before use.
    pub fn new(cwd: impl Into<PathBuf>, home: impl Into<PathBuf>) -> Self {
        Self {
            cwd: cwd.into(),
            home: home.into(),
        }
    }

    /// Working directory commands run in.
    pub fn cwd(&self) -> &Path {
        &self.cwd
    }

    /// The private home directory.
    pub fn home(&self) -> &Path {
        &self.home
    }

    /// Create the private home with a minimal global configuration. Idempotent.
    pub fn ensure_home(&self) -> Result<()> {
        fs::create_dir_all(&self.home)
            .with_context(|| format!("creating fixture home {}", self.home.display()))?;
        let config = self.home.join(".gitconfig");
        if !config.exists() {
            fs::write(
                &config,
                concat!(
                    "[init]\n\tdefaultBranch = main\n",
                    "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n",
                    "[commit]\n\tgpgsign = false\n",
                    "[tag]\n\tgpgsign = false\n",
                    "[core]\n\tautocrlf = false\n\tpager = cat\n",
                    "[gc]\n\tauto = 0\n",
                    "[advice]\n\tdetachedHead = false\n",
                    "[merge]\n\tverbosity = 1\n",
                ),
            )
            .with_context(|| format!("writing {}", config.display()))?;
        }
        Ok(())
    }

    /// A pre-configured `Command` for git in this working directory.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(git_binary());
        cmd.current_dir(&self.cwd)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_PAGER", "cat")
            .env("LC_ALL", "C")
            .env("TZ", "UTC")
            .env("GIT_AUTHOR_NAME", AUTHOR_A.0)
            .env("GIT_AUTHOR_EMAIL", AUTHOR_A.1)
            .env("GIT_COMMITTER_NAME", AUTHOR_A.0)
            .env("GIT_COMMITTER_EMAIL", AUTHOR_A.1)
            .env("GIT_AUTHOR_DATE", clock_at(0))
            .env("GIT_COMMITTER_DATE", clock_at(0))
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .env_remove("GIT_NAMESPACE");
        // Never a repository above the one that is meant.
        if let Some(above) = self.cwd.parent() {
            cmd.env("GIT_CEILING_DIRECTORIES", above);
        }
        cmd
    }

    /// Run git with `args`; returns stdout. Non-zero exit is an error carrying
    /// both output streams.
    pub fn run<I, S>(&self, args: I) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.run_with(args, &[])
    }

    /// Like [`Git::run`] with extra environment variables (dates, identity).
    pub fn run_with<I, S>(&self, args: I, env: &[(&str, &str)]) -> Result<String>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let mut cmd = self.command();
        cmd.args(args);
        for (key, value) in env {
            cmd.env(key, value);
        }
        let rendered = format!("{cmd:?}");
        let out = cmd
            .output()
            .with_context(|| format!("spawning {rendered}"))?;
        if !out.status.success() {
            bail!(
                "git exited with {} in {}\ncommand: {rendered}\nstdout:\n{}\nstderr:\n{}",
                out.status,
                self.cwd.display(),
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// Run git and split stdout into trimmed, non-empty lines.
    pub fn lines<I, S>(&self, args: I) -> Result<Vec<String>>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        Ok(self
            .run(args)?
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// `git rev-list --count <rev>`.
    pub fn rev_list_count(&self, rev: &str) -> Result<u64> {
        let out = self.run(["rev-list", "--count", rev])?;
        out.trim()
            .parse()
            .with_context(|| format!("parsing rev-list count {out:?}"))
    }

    /// Short names of references under `prefix` (for example `refs/heads`), sorted.
    pub fn refs(&self, prefix: &str) -> Result<Vec<String>> {
        let mut refs = self.lines(["for-each-ref", "--format=%(refname:short)", prefix])?;
        refs.sort();
        Ok(refs)
    }

    /// `git rev-parse <rev>`.
    pub fn rev_parse(&self, rev: &str) -> Result<String> {
        Ok(self.run(["rev-parse", rev])?.trim().to_owned())
    }

    /// `git for-each-ref --format=<format>` over the user's references:
    /// every reference but the ones the application keeps for itself under
    /// `refs/bitbraid/`, which hold what undo needs and are no part of
    /// what a command did to the repository. A twin that only git worked
    /// on has none, so this is what the two are compared by.
    pub fn user_refs(&self, format: &str) -> Result<Vec<String>> {
        let with_name = format!("--format=%(refname)%00{format}");
        Ok(self
            .lines(["for-each-ref", with_name.as_str()])?
            .into_iter()
            .filter_map(|line| {
                let (name, rest) = line.split_once('\0')?;
                (!name.starts_with("refs/bitbraid/")).then(|| rest.to_owned())
            })
            .collect())
    }
}

/// Version string of the git binary the oracle uses.
pub fn git_version() -> Result<String> {
    let out = Command::new(git_binary())
        .arg("--version")
        .output()
        .context("running git --version; is git installed or BITBRAID_GIT set?")?;
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}
