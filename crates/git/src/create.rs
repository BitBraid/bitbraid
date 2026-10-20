//! Making repositories: `init` and `clone`.
//!
//! A clone that fails or is stopped leaves nothing behind. Git cleans up
//! after itself when it ends on its own, but not when it is killed, which
//! is how an operation is stopped; so the destination has to be absent or
//! an empty directory before the clone starts, and everything found in it
//! afterwards is the clone's and can go.
//!
//! A partial clone leaves the content of files, or trees too, on the
//! remote until something needs them; git fetches them then, and a fetch
//! keeps the filter the clone was made with.
//!
//! A clone that succeeded gets git's commit-graph file, written by git
//! with its progress: `git clone` leaves none until git's maintenance
//! writes one, and without it every walk of the history reads each commit
//! from the pack, many times slower on a long history.
//!
//! Sources: git-init(1), git-clone(1) (`--filter`, `--no-local`,
//! `--sparse`), git-check-ref-format(1), git-config(1) (`protocol.allow`:
//! git itself refuses the `ext` transport; `remote.<name>.promisor`,
//! `remote.<name>.partialclonefilter`; `core.commitGraph`), gitglossary(7)
//! and the partial clone design notes in git's documentation
//! (`partial-clone.adoc`), git-commit-graph(1) (`write --reachable
//! --progress`), gitformat-commit-graph(5) (not used with a shallow
//! repository).

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Result, exe,
    remote::{Reporter, parse_progress},
};

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

fn io(context: &'static str, err: std::io::Error) -> Error {
    Error::Git {
        context,
        source: Box::new(err),
    }
}

/// The directory git is run in for a path that may not exist yet: the
/// nearest ancestor that does. The path itself has to be absolute.
fn existing_ancestor(what: &'static str, path: &Path) -> Result<PathBuf> {
    if !path.is_absolute() {
        return Err(invalid(what, say!("create-path-not-absolute")));
    }
    path.ancestors()
        .find(|dir| dir.is_dir())
        .map(Path::to_path_buf)
        .ok_or_else(|| invalid(what, say!("create-path-missing")))
}

/// Whether `dir` is laid out as a git directory: the `.git` of a
/// repository or a bare repository itself (gitrepository-layout(5); the
/// three things git looks for before it takes a directory for one).
fn is_git_dir(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

/// Refuse a place that is a repository's own directory, or lies in one:
/// a bare repository, or below a `.git`. `git init` there would quietly
/// set an existing repository up again, and a clone there would put
/// files among git's own.
fn outside_git_dirs(what: &'static str, path: &Path) -> Result<()> {
    if path.ancestors().any(is_git_dir) {
        return Err(refused(what, say!("create-inside-git-dir")));
    }
    Ok(())
}

/// A branch name as git takes it for a new repository.
fn check_branch(dir: &Path, name: &str) -> Result<()> {
    let bad = || {
        invalid(
            say!("create-what-branch-name"),
            say!("create-not-valid-branch", name = format!("{name:?}")),
        )
    };
    if name.is_empty()
        || name.starts_with('-')
        || name.chars().any(char::is_control)
        || name == "HEAD"
        || name == "@"
        || name.contains("@{")
        // `git init` only writes the name into HEAD; a part too long to be
        // a file name would come to light at the first commit.
        || name.split('/').any(|part| part.len() > 200)
        || name.len() > 1000
    {
        return Err(bad());
    }
    let reference = format!("refs/heads/{name}");
    exe::run_in(dir, ["check-ref-format", reference.as_str()])
        .map(|_| ())
        .map_err(|_| bad())
}

/// How to create a repository.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct InitOptions {
    /// Name of the first branch; git's configured default when absent.
    #[serde(default)]
    pub branch: Option<String>,
    /// Make a first commit, so that there is something to branch from:
    /// empty, or with the `.gitignore` when there is one. Needs a
    /// configured user name and email.
    #[serde(default)]
    pub first_commit: bool,
    /// Start the `.gitignore` from these templates
    /// ([`crate::ignore_templates`]), in this order.
    #[serde(default)]
    pub gitignore: Vec<String>,
}

/// Create a repository at `path`, creating the directory when it is not
/// there. A directory that is a repository already is refused, since
/// `git init` there would quietly do something else.
pub fn init(path: &Path, options: &InitOptions) -> Result<PathBuf> {
    let parent = existing_ancestor(say!("create-what-init"), path)?;
    if path.exists() && !path.is_dir() {
        return Err(invalid(
            say!("create-what-init"),
            say!("create-file-at-path"),
        ));
    }
    if path.join(".git").symlink_metadata().is_ok() {
        return Err(refused(
            say!("create-what-init"),
            say!("create-already-repository"),
        ));
    }
    outside_git_dirs(say!("create-what-init"), path)?;
    if let Some(branch) = &options.branch {
        check_branch(&parent, branch)?;
    }
    // Before anything is made: a template that does not exist makes
    // nothing.
    let gitignore = crate::ignore_templates::compose(&options.gitignore)?;
    let ignore_file = path.join(".gitignore");
    if gitignore.is_some() && ignore_file.symlink_metadata().is_ok() {
        return Err(refused(
            say!("create-what-init"),
            say!("create-gitignore-exists"),
        ));
    }
    fs::create_dir_all(path).map_err(|err| io(say!("create-creating-directory"), err))?;
    let mut args = vec!["init", "--quiet"];
    if let Some(branch) = &options.branch {
        args.extend(["--initial-branch", branch.as_str()]);
    }
    // What is run from here on is the new repository's in the activity
    // log, and kept there once the repository is made.
    let about = crate::activity::about(path);
    exe::run_in(path, args)?;
    let written = match &gitignore {
        Some(text) => {
            fs::write(&ignore_file, text).map_err(|err| io(say!("create-writing-gitignore"), err))
        }
        None => Ok(()),
    };
    let committed = if written.is_ok() && options.first_commit {
        let committed = match &gitignore {
            // The file is what the first commit has in it.
            Some(_) => exe::run_in(path, ["add", "--", ".gitignore"]).and_then(|_| {
                exe::run_in(path, ["commit", "--quiet", "--message", "Initial commit"])
            }),
            None => exe::run_in(
                path,
                [
                    "commit",
                    "--quiet",
                    "--allow-empty",
                    "--message",
                    "Initial commit",
                ],
            ),
        };
        committed.map(|_| ()).map_err(|err| {
            refused(
                say!("create-what-init"),
                say!("create-first-commit-failed", err = err.to_string()),
            )
        })
    } else {
        Ok(())
    };
    // The repository is there, whatever became of its first file and
    // its first commit.
    about.came_to_be();
    written?;
    committed?;
    Ok(path.to_path_buf())
}

/// What to clone and where to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CloneOptions {
    /// Where from: a URL or a path.
    pub url: String,
    /// Where to: an absolute path that does not exist, or an empty
    /// directory.
    pub path: PathBuf,
    /// Check this branch out instead of the remote's default.
    #[serde(default)]
    pub branch: Option<String>,
    /// History cut to this many commits.
    #[serde(default)]
    pub depth: Option<u32>,
    /// Clone submodules too.
    #[serde(default)]
    pub recurse_submodules: bool,
    /// Let git clone submodules from local paths, which it refuses by
    /// itself (`protocol.file.allow`): for a superproject whose submodules
    /// are folders on this machine.
    #[serde(default)]
    pub allow_local: bool,
    /// A partial clone: what is left on the remote until it is needed.
    #[serde(default)]
    pub filter: Option<CloneFilter>,
    /// Check out only the files at the top, in cone mode, to choose more
    /// from afterwards ([`crate::sparse`]).
    #[serde(default)]
    pub sparse: bool,
}

/// What a partial clone leaves on the remote, to be fetched by git when
/// something needs it.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum CloneFilter {
    /// The content of files (`blob:none`): history is all there, and the
    /// content of the files checked out.
    #[serde(rename = "blob:none")]
    BlobNone,
    /// Trees too (`tree:0`): only the commits, and what is checked out.
    #[serde(rename = "tree:0")]
    TreeZero,
}

impl CloneFilter {
    /// The filter as git writes it.
    pub fn spec(self) -> &'static str {
        match self {
            CloneFilter::BlobNone => "blob:none",
            CloneFilter::TreeZero => "tree:0",
        }
    }
}

/// Whether `url` is a folder on this machine rather than a URL: a clone
/// from one copies the repository's files, and leaves out a filter
/// (git-clone(1) `--local`).
fn is_local_path(url: &str) -> bool {
    if url.contains("://") {
        return false;
    }
    // `host:path` is ssh, unless a slash comes before the colon
    // (git-clone(1) "GIT URLS"); a drive letter is a path.
    match url.find(':') {
        Some(colon) => {
            let before = url.get(..colon).unwrap_or("");
            before.contains('/') || (colon == 1 && cfg!(windows))
        }
        None => true,
    }
}

/// Whether the repository at `path` is a partial clone: some remote is
/// one git fetches missing objects from.
pub fn is_partial(path: &Path) -> bool {
    exe::run_in(
        path,
        [
            "config",
            "--get-regexp",
            r"^remote\..*\.promisor$",
            "^true$",
        ],
    )
    .is_ok_and(|out| !out.stdout.trim().is_empty())
}

/// The directory name git would pick for a clone of `url`: its last path
/// component without `.git`. None when there is nothing usable.
pub fn clone_dir_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches(['/', '\\']);
    let last = trimmed.rsplit(['/', '\\', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    let usable = !name.is_empty()
        && name != "."
        && name != ".."
        && !name.chars().any(|ch| ch.is_control() || ch == '\0');
    usable.then(|| name.to_owned())
}

/// Is `dir` an existing directory with nothing in it?
fn is_empty_dir(dir: &Path) -> bool {
    fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_none())
}

/// Remove what a clone that did not finish left at `path`. Everything
/// there is the clone's: the path was absent or empty when it started.
fn remove_leftovers(path: &Path, existed: bool) {
    // A link is not followed: only a real directory is emptied.
    if !fs::symlink_metadata(path).is_ok_and(|meta| meta.is_dir()) {
        return;
    }
    if !existed {
        // A killed git's own children (the pack indexer, the transport) may
        // write a moment longer; try again until the directory stays gone.
        for _ in 0..10 {
            if fs::remove_dir_all(path).is_ok() || !path.exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        return;
    }
    let Ok(entries) = fs::read_dir(path) else {
        return;
    };
    for entry in entries.flatten() {
        let inside = entry.path();
        if fs::symlink_metadata(&inside).is_ok_and(|meta| meta.is_dir()) {
            let _ = fs::remove_dir_all(&inside);
        } else {
            let _ = fs::remove_file(&inside);
        }
    }
}

/// What a clone made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cloned {
    /// Where the repository is.
    pub path: PathBuf,
    /// A partial clone was asked for and made. A remote that does not
    /// filter sends everything, and git only warns about it.
    pub partial: bool,
    /// Git wrote its commit-graph file in it.
    pub commit_graph: bool,
}

/// What git says when the remote sent everything in spite of a filter.
const NOT_FILTERED: &str = "filtering not recognized by server";

/// What the progress says while the commit-graph is written, as the
/// maintenance's step says it.
pub fn commit_graph_stage() -> &'static str {
    say!("maintenance-stage-commit-graph")
}

/// Have git write the commit-graph of the repository a clone just made
/// at `path` (never bare, its git directory `.git` in it). Not for a
/// shallow clone, which git writes none for, nor where `core.commitGraph`
/// says git is not to use one. The clone stands whatever becomes of
/// this: whether one is there afterwards is the answer.
fn write_commit_graph(path: &Path, reporter: &mut Reporter<'_>) -> bool {
    let git_dir = path.join(".git");
    if git_dir.join("shallow").exists() {
        return false;
    }
    let off = exe::run_in(path, ["config", "--bool", "--get", "core.commitGraph"])
        .is_ok_and(|out| out.stdout.trim() == "false");
    if off {
        return false;
    }
    (reporter.on_progress)(crate::Progress {
        stage: commit_graph_stage().to_owned(),
        percent: None,
    });
    let cancel = reporter.cancel.clone();
    let written = exe::run_streaming_in(
        path,
        ["commit-graph", "write", "--reachable", "--progress"],
        &cancel,
        &mut |line| {
            if let Some(progress) = parse_progress(line) {
                (reporter.on_progress)(progress);
            }
        },
    );
    let info = git_dir.join("objects").join("info");
    written.is_ok()
        && (info.join("commit-graph").is_file()
            || info
                .join("commit-graphs")
                .join("commit-graph-chain")
                .is_file())
}

/// Clone a repository. On failure, and when stopped, the destination is as
/// it was before.
pub fn clone(options: &CloneOptions, reporter: &mut Reporter<'_>) -> Result<Cloned> {
    let url = options.url.trim();
    if url.is_empty() || url.starts_with('-') || url.chars().any(char::is_control) {
        return Err(invalid(
            say!("create-what-clone-url"),
            say!("create-url-unusable", url = format!("{url:?}")),
        ));
    }
    let path = &options.path;
    let parent = existing_ancestor(say!("create-what-clone"), path)?;
    let existed = path.symlink_metadata().is_ok();
    if existed && !is_empty_dir(path) {
        return Err(refused(
            say!("create-what-clone"),
            say!("create-not-empty", path = path.display().to_string()),
        ));
    }
    outside_git_dirs(say!("create-what-clone"), path)?;
    if let Some(branch) = &options.branch {
        check_branch(&parent, branch)?;
    }
    let destination = path.to_str().ok_or_else(|| {
        invalid(
            say!("create-what-clone"),
            say!("create-destination-not-utf8"),
        )
    })?;
    let depth = options.depth.map(|depth| depth.max(1).to_string());
    let mut args = Vec::new();
    if options.allow_local {
        args.extend(["-c", "protocol.file.allow=always"]);
    }
    args.extend(["clone", "--progress"]);
    if let Some(branch) = &options.branch {
        args.extend(["--branch", branch.as_str()]);
    }
    if let Some(depth) = &depth {
        args.extend(["--depth", depth.as_str()]);
    }
    if options.recurse_submodules {
        args.push("--recurse-submodules");
    }
    let filter = options
        .filter
        .map(|filter| format!("--filter={}", filter.spec()));
    if let Some(filter) = &filter {
        args.push(filter.as_str());
        // From a folder git copies the files and ignores the filter; the
        // transport a URL would use makes a partial clone of it.
        if is_local_path(url) {
            args.push("--no-local");
        }
    }
    if options.sparse {
        args.push("--sparse");
    }
    args.extend(["--", url, destination]);

    let cancel = reporter.cancel.clone();
    // The clone is the new repository's in the activity log, wherever git
    // runs for it, and kept there once there is such a repository.
    let about = crate::activity::about(path);
    let cloned = exe::run_streaming_in(&parent, args, &cancel, &mut |line| {
        if let Some(progress) = parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    });
    match cloned {
        Ok(out) => {
            about.came_to_be();
            let partial =
                options.filter.is_some() && !out.stderr.contains(NOT_FILTERED) && is_partial(path);
            let commit_graph = write_commit_graph(path, reporter);
            Ok(Cloned {
                path: path.clone(),
                partial,
                commit_graph,
            })
        }
        Err(err) => {
            remove_leftovers(path, existed);
            // A remote or a proxy that would not let git in is said in our
            // words, as it is for a fetch.
            Err(crate::remote::explained(err))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{clone_dir_name, is_local_path};

    #[test]
    fn a_folder_is_told_from_a_url() {
        for (url, local) in [
            ("/srv/git/project.git", true),
            ("../sibling", true),
            ("project", true),
            ("./with:colon", true),
            ("file:///srv/git/project.git", false),
            ("https://example.invalid/a/b.git", false),
            ("git@example.invalid:team/project.git", false),
            ("example.invalid:project", false),
        ] {
            assert_eq!(is_local_path(url), local, "{url:?}");
        }
    }

    #[test]
    fn a_clone_is_named_after_the_last_part_of_its_url() {
        for (url, name) in [
            ("https://example.invalid/team/project.git", Some("project")),
            ("https://example.invalid/team/project", Some("project")),
            ("https://example.invalid/team/project/", Some("project")),
            ("git@example.invalid:team/project.git", Some("project")),
            ("git@example.invalid:project.git", Some("project")),
            (
                "ssh://git@example.invalid:22/team/project.git",
                Some("project"),
            ),
            ("/srv/git/project.git", Some("project")),
            ("../sibling", Some("sibling")),
            ("C:\\repos\\project.git", Some("project")),
            ("  https://example.invalid/a/b.git  ", Some("b")),
            ("project.git.git", Some("project.git")),
            ("", None),
            ("/", None),
            ("https://example.invalid/.git", None),
            ("..", None),
            ("https://example.invalid/..", None),
        ] {
            assert_eq!(clone_dir_name(url).as_deref(), name, "{url:?}");
        }
    }
}
