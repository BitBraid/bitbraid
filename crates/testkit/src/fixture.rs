//! Fixture repositories built from TOML scripts.
//!
//! A script is a list of steps applied in order with the real git binary. The
//! clock advances one hour per commit-like step and identities alternate, so
//! the resulting object ids are stable. Built fixtures are cached under
//! `<target>/fixtures/<name>` and rebuilt when the script text changes.

use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result, anyhow, bail};
use serde::Deserialize;

use crate::{
    fixtures_out_dir, fixtures_src_dir, fnv1a64,
    oracle::{Git, author_for, clock_at},
};

/// Bump when the builder's behaviour changes in a way that alters output.
const FORMAT_VERSION: u32 = 1;

/// One fixture script (`fixtures/<name>.toml`).
#[derive(Debug, Clone, Deserialize)]
pub struct FixtureScript {
    /// Must equal the file stem.
    pub name: String,
    /// Free-text purpose of the fixture.
    #[serde(default)]
    pub description: String,
    /// Steps applied in order.
    #[serde(default)]
    pub steps: Vec<Step>,
    /// The fixture is a bare repository: the steps build an ordinary one,
    /// and what is kept is a bare clone of it, without a remote.
    #[serde(default)]
    pub bare: bool,
    /// Assertions checked after building.
    #[serde(default)]
    pub expect: Expect,
}

/// Assertions checked against the built repository with the git oracle.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Expect {
    /// Total commits reachable from any ref.
    pub commits: Option<u64>,
    /// Sorted short names of local branches.
    pub branches: Option<Vec<String>>,
    /// Sorted tag names.
    pub tags: Option<Vec<String>>,
}

/// A file written by a commit step.
#[derive(Debug, Clone, Deserialize)]
pub struct FileSpec {
    /// Path relative to the worktree.
    pub path: String,
    /// Content; a trailing newline is added when missing.
    #[serde(default)]
    pub content: String,
}

/// One scripted action.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// Write files, stage everything and commit.
    Commit {
        message: String,
        #[serde(default)]
        file: Option<String>,
        #[serde(default)]
        content: Option<String>,
        #[serde(default)]
        files: Vec<FileSpec>,
    },
    /// Create a branch, optionally at a revision, optionally checking it out.
    Branch {
        name: String,
        #[serde(default)]
        at: Option<String>,
        #[serde(default)]
        checkout: bool,
    },
    /// Check out a branch, tag or commit.
    Checkout { target: String },
    /// Merge one or more branches into the current branch (octopus when several).
    Merge {
        branches: Vec<String>,
        #[serde(default)]
        message: Option<String>,
        /// Allow fast-forward; the default creates a merge commit.
        #[serde(default)]
        ff: bool,
    },
    /// Create a tag; annotated when a message is given.
    Tag {
        name: String,
        #[serde(default)]
        message: Option<String>,
        #[serde(default)]
        at: Option<String>,
    },
    /// Start an orphan branch with an empty index and worktree.
    Orphan { name: String },
    /// `git rm` a path.
    Remove { path: String },
    /// `git mv` a path.
    Move { from: String, to: String },
    /// Escape hatch: run git with these arguments under the fixture clock.
    Run { args: Vec<String> },
}

/// Names of all scripts in `fixtures/`, sorted.
pub fn list_scripts() -> Result<Vec<String>> {
    let dir = fixtures_src_dir();
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "toml")
            && let Some(stem) = path.file_stem().and_then(|stem| stem.to_str())
        {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Load and validate a script by name.
pub fn load_script(name: &str) -> Result<(FixtureScript, String)> {
    let path = fixtures_src_dir().join(format!("{name}.toml"));
    let text = fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let script: FixtureScript =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if script.name != name {
        bail!(
            "fixture {} declares name {:?}, expected {name:?}",
            path.display(),
            script.name
        );
    }
    Ok((script, text))
}

static BUILD_LOCK: Mutex<()> = Mutex::new(());

/// The file whose lock holds apart the test programs that build in one
/// directory at once: `cargo test` runs one program at a time, but a
/// second `cargo test` or the pictures may be at work in the same target
/// directory.
const LOCK_FILE: &str = ".build.lock";

/// Build (or reuse) the fixture `name` and return its worktree path.
pub fn build_fixture(name: &str) -> Result<PathBuf> {
    build_in(name, &fixtures_out_dir())
}

/// Build (or reuse) the fixture `name` under `out_root`. The threads of
/// one program take turns through a mutex, programs through a lock on a
/// file in `out_root`, so that no one finds a fixture half removed or
/// half moved into place.
fn build_in(name: &str, out_root: &Path) -> Result<PathBuf> {
    let _guard = BUILD_LOCK
        .lock()
        .map_err(|_| anyhow!("fixture build lock poisoned"))?;
    let (script, text) = load_script(name)?;
    fs::create_dir_all(out_root).with_context(|| format!("creating {}", out_root.display()))?;
    let lock_path = out_root.join(LOCK_FILE);
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("opening {}", lock_path.display()))?;
    // Released when the file is closed, at the end of this function.
    lock.lock()
        .with_context(|| format!("locking {}", lock_path.display()))?;
    let out = out_root.join(name);
    let stamp = out_root.join(format!("{name}.stamp"));
    let want = format!("{FORMAT_VERSION}:{:016x}", fnv1a64(text.as_bytes()));
    if out.is_dir() && fs::read_to_string(&stamp).ok().as_deref() == Some(want.as_str()) {
        return Ok(out);
    }
    let _ = fs::remove_file(&stamp);
    if out.exists() {
        fs::remove_dir_all(&out).with_context(|| format!("removing stale {}", out.display()))?;
    }
    let tmp = out_root.join(format!(".{name}.tmp-{}", std::process::id()));
    if tmp.exists() {
        fs::remove_dir_all(&tmp)?;
    }
    fs::create_dir_all(&tmp)?;
    let home = out_root.join(".home");
    let git = Git::new(&tmp, &home);
    git.ensure_home()?;
    build_into(&script, &git).with_context(|| format!("building fixture {name}"))?;
    verify(&script, &git).with_context(|| format!("verifying fixture {name}"))?;
    if script.bare {
        let bare = make_bare(&tmp, &git).with_context(|| format!("making {name} bare"))?;
        let bare_git = Git::new(&bare, &home);
        verify(&script, &bare_git).with_context(|| format!("verifying bare fixture {name}"))?;
        fs::remove_dir_all(&tmp)?;
        fs::rename(&bare, &out)
            .with_context(|| format!("moving fixture into {}", out.display()))?;
    } else {
        fs::rename(&tmp, &out).with_context(|| format!("moving fixture into {}", out.display()))?;
    }
    fs::write(&stamp, want)?;
    Ok(out)
}

/// A bare clone of the repository at `from`, beside it: every branch and
/// tag, `HEAD` on the branch that was checked out, and no remote, so that
/// nothing run in it can reach the directory it was made from.
fn make_bare(from: &Path, git: &Git) -> Result<PathBuf> {
    let mut name = from
        .file_name()
        .ok_or_else(|| anyhow!("no directory name in {}", from.display()))?
        .to_os_string();
    name.push(".bare");
    let bare = from.with_file_name(name);
    if bare.exists() {
        fs::remove_dir_all(&bare)?;
    }
    let (Some(source), Some(target)) = (from.to_str(), bare.to_str()) else {
        bail!("fixture paths must be UTF-8: {}", from.display());
    };
    // Copies rather than links: the copy must not share files with a
    // directory that is about to go.
    git.run(["clone", "-q", "--bare", "--no-hardlinks", source, target])?;
    let bare_git = Git::new(&bare, git.home());
    bare_git.run(["remote", "remove", "origin"])?;
    Ok(bare)
}

/// Build every script; returns `(name, path)` pairs.
pub fn build_all() -> Result<Vec<(String, PathBuf)>> {
    let mut built = Vec::new();
    for name in list_scripts()? {
        let path = build_fixture(&name)?;
        built.push((name, path));
    }
    Ok(built)
}

fn build_into(script: &FixtureScript, git: &Git) -> Result<()> {
    git.run(["init", "-q", "-b", "main"])?;
    let mut tick: u64 = 0;
    for (index, step) in script.steps.iter().enumerate() {
        apply(step, git, &mut tick)
            .with_context(|| format!("step {} ({step:?})", index.saturating_add(1)))?;
    }
    Ok(())
}

fn dated_env(tick: u64) -> Vec<(String, String)> {
    let (name, email) = author_for(tick);
    let date = clock_at(tick);
    vec![
        ("GIT_AUTHOR_NAME".into(), name.into()),
        ("GIT_AUTHOR_EMAIL".into(), email.into()),
        ("GIT_COMMITTER_NAME".into(), name.into()),
        ("GIT_COMMITTER_EMAIL".into(), email.into()),
        ("GIT_AUTHOR_DATE".into(), date.clone()),
        ("GIT_COMMITTER_DATE".into(), date),
    ]
}

fn run_dated(git: &Git, args: &[&str], tick: &mut u64) -> Result<String> {
    let env = dated_env(*tick);
    let borrowed: Vec<(&str, &str)> = env.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
    let out = git.run_with(args, &borrowed)?;
    *tick = tick.saturating_add(1);
    Ok(out)
}

fn write_file(root: &Path, rel: &str, content: &str) -> Result<()> {
    if rel.starts_with('/') || rel.contains("..") {
        bail!("fixture file path must be relative and must not contain '..': {rel:?}");
    }
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut body = content.to_owned();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    fs::write(&path, body).with_context(|| format!("writing {}", path.display()))
}

fn apply(step: &Step, git: &Git, tick: &mut u64) -> Result<()> {
    match step {
        Step::Commit {
            message,
            file,
            content,
            files,
        } => {
            if let Some(file) = file {
                write_file(git.cwd(), file, content.as_deref().unwrap_or_default())?;
            }
            for spec in files {
                write_file(git.cwd(), &spec.path, &spec.content)?;
            }
            git.run(["add", "-A"])?;
            run_dated(git, &["commit", "-q", "--allow-empty", "-m", message], tick)?;
        }
        Step::Branch { name, at, checkout } => {
            let mut args = vec!["branch", name.as_str()];
            if let Some(at) = at {
                args.push(at.as_str());
            }
            git.run(&args)?;
            if *checkout {
                git.run(["checkout", "-q", name.as_str()])?;
            }
        }
        Step::Checkout { target } => {
            git.run(["checkout", "-q", target.as_str()])?;
        }
        Step::Merge {
            branches,
            message,
            ff,
        } => {
            if branches.is_empty() {
                bail!("merge step needs at least one branch");
            }
            let mut args = vec!["merge", "-q"];
            args.push(if *ff { "--ff" } else { "--no-ff" });
            if let Some(message) = message {
                args.push("-m");
                args.push(message.as_str());
            }
            args.extend(branches.iter().map(String::as_str));
            run_dated(git, &args, tick)?;
        }
        Step::Tag { name, message, at } => {
            let mut args = vec!["tag"];
            if let Some(message) = message {
                args.extend(["-a", "-m", message.as_str()]);
            }
            args.push(name.as_str());
            if let Some(at) = at {
                args.push(at.as_str());
            }
            run_dated(git, &args, tick)?;
        }
        Step::Orphan { name } => {
            git.run(["checkout", "-q", "--orphan", name.as_str()])?;
            // An orphan keeps the previous index; start empty. Ignore the
            // error git reports when there is nothing to remove.
            let _ = git.run(["rm", "-rfq", "."]);
        }
        Step::Remove { path } => {
            git.run(["rm", "-q", path.as_str()])?;
        }
        Step::Move { from, to } => {
            git.run(["mv", from.as_str(), to.as_str()])?;
        }
        Step::Run { args } => {
            let borrowed: Vec<&str> = args.iter().map(String::as_str).collect();
            run_dated(git, &borrowed, tick)?;
        }
    }
    Ok(())
}

fn verify(script: &FixtureScript, git: &Git) -> Result<()> {
    if let Some(want) = script.expect.commits {
        let got = git.rev_list_count("--all")?;
        if got != want {
            bail!("expected {want} commits, git counts {got}");
        }
    }
    if let Some(want) = &script.expect.branches {
        let got = git.refs("refs/heads")?;
        if &got != want {
            bail!("expected branches {want:?}, git lists {got:?}");
        }
    }
    if let Some(want) = &script.expect.tags {
        let got = git.refs("refs/tags")?;
        if &got != want {
            bail!("expected tags {want:?}, git lists {got:?}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scratch_root;

    #[test]
    fn every_script_builds_and_matches_its_expectations() {
        let built = build_all().expect("fixtures build");
        assert!(!built.is_empty(), "no fixture scripts found");
        for (name, path) in built {
            let (script, _) = load_script(&name).expect("script loads");
            if script.bare {
                assert!(
                    path.join("HEAD").is_file() && !path.join(".git").exists(),
                    "{name}: not a bare repository in {}",
                    path.display()
                );
                let git = Git::new(&path, fixtures_out_dir().join(".home"));
                assert_eq!(
                    git.run(["rev-parse", "--is-bare-repository"])
                        .unwrap()
                        .trim(),
                    "true",
                    "{name}"
                );
                assert!(
                    git.lines(["remote"]).unwrap().is_empty(),
                    "{name}: a remote"
                );
                continue;
            }
            assert!(
                path.join(".git").is_dir(),
                "{name}: no .git in {}",
                path.display()
            );
        }
    }

    /// Built twice from nothing, each time in a directory of its own, and
    /// the same as the one every test uses. Built again in that shared
    /// place, it would be pulled from under whatever else reads or builds
    /// it at the time.
    #[test]
    fn fixtures_are_deterministic_across_rebuilds() {
        let refs = |root: &Path| {
            let path = build_in("linear", root).expect("linear builds");
            let git = Git::new(&path, root.join(".home"));
            git.run(["show-ref", "--head"]).unwrap()
        };
        let built: Vec<String> = (0..2)
            .map(|n| {
                let root = scratch_root().join(format!("rebuilt-{}-{n}", std::process::id()));
                let _ = fs::remove_dir_all(&root);
                let shown = refs(&root);
                assert!(
                    root.join(LOCK_FILE).is_file(),
                    "no lock file in {}",
                    root.display()
                );
                fs::remove_dir_all(&root).unwrap();
                shown
            })
            .collect();
        assert_eq!(built[0], built[1], "the references changed between builds");
        assert_eq!(
            built[0],
            refs(&fixtures_out_dir()),
            "the shared fixture differs from one built anew"
        );
    }

    /// A file's lock holds a second holder back until the first closes
    /// the file, as it holds back a second program building fixtures in
    /// the same directory.
    #[test]
    fn the_build_lock_holds_a_second_holder_back() {
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        let root = scratch_root().join(format!("lock-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let path = root.join(LOCK_FILE);
        let open = |path: &Path| {
            fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .write(true)
                .open(path)
                .unwrap()
        };
        let first = open(&path);
        first.lock().unwrap();
        let (sender, receiver) = mpsc::channel();
        let second = open(&path);
        let waiter = std::thread::spawn(move || {
            second.lock().unwrap();
            sender.send(Instant::now()).unwrap();
        });
        std::thread::sleep(Duration::from_millis(200));
        assert!(receiver.try_recv().is_err(), "the second took the lock");
        // When the second got it, not how long it asked: on a busy machine
        // its thread may begin asking long after it was started.
        let released = Instant::now();
        drop(first);
        let taken = receiver.recv_timeout(Duration::from_secs(10)).unwrap();
        assert!(
            taken >= released,
            "the second took the lock before the first let go"
        );
        waiter.join().unwrap();
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn hostile_paths_are_rejected() {
        let dir = std::env::temp_dir().join(format!("bitbraid-testkit-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        assert!(write_file(&dir, "../escape.txt", "x").is_err());
        assert!(write_file(&dir, "/abs.txt", "x").is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
