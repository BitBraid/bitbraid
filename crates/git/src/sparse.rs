//! Sparse checkout: only part of the tracked files in the worktree,
//! chosen by directory (cone mode) or by patterns, through
//! `git sparse-checkout`.
//!
//! Git keeps the choice in the git directory of the worktree
//! (`info/sparse-checkout`) and turns it on in the worktree's own
//! configuration; files outside it are marked as skipped in the index
//! and taken out of the worktree. A file with changes is left where it
//! is, and git says so; that is passed on.
//!
//! Sources: git-sparse-checkout(1) (`set`, `add`, `list`, `disable`,
//! `--cone`, `--no-cone`, what cone mode accepts), git-ls-files(1) (`-t`:
//! `S` for a file outside the checkout), git-config(1)
//! (`core.sparseCheckout`, `core.sparseCheckoutCone`, `index.sparse`),
//! gitglossary(7) (`--end-of-options`).

use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe, remote::Reporter};

/// The most directories or patterns taken at once.
pub const MOST_RULES: usize = 1000;
/// The longest directory or pattern taken.
pub const LONGEST_RULE: usize = 4096;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("sparse-what-sparse-checkout"),
        detail: detail.into(),
    }
}

/// How the checkout stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Default)]
pub struct SparseStatus {
    /// Only part of the files are checked out.
    pub enabled: bool,
    /// The part is chosen by directory: every file at the top, and
    /// everything below the directories listed.
    pub cone: bool,
    /// The index itself keeps the directories outside the checkout as one
    /// entry each (`index.sparse`).
    pub sparse_index: bool,
    /// The directories of a cone, or the patterns, as git lists them.
    pub rules: Vec<String>,
    /// Tracked files; counted only when the checkout is sparse, and zero
    /// otherwise, so that asking costs next to nothing where it is not.
    pub tracked: u64,
    /// Tracked files that are not checked out.
    pub outside: u64,
}

/// What part of the files to check out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rules {
    /// Directories, relative to the top of the worktree: the files at the
    /// top and everything below these (cone mode).
    Directories(Vec<String>),
    /// Patterns as in `.gitignore`, which say which files are in.
    Patterns(Vec<String>),
}

/// What a change did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Changed {
    /// How the checkout stands now.
    pub status: SparseStatus,
    /// What git said: files with changes that were left in place, among
    /// other things.
    pub output: String,
}

/// A worktree is needed: a bare repository has no files to leave out.
fn worktree(repo: &Repository) -> Result<()> {
    if repo.workdir().is_none() {
        return Err(refused(say!("sparse-bare")));
    }
    Ok(())
}

fn flag(repo: &Repository, key: &str) -> bool {
    exe::run(repo, ["config", "--get", "--bool", key], None)
        .is_ok_and(|out| out.stdout.trim() == "true")
}

/// A directory of a cone, as git takes one: relative, inside the
/// worktree, without what makes a pattern of it. A slash at the end is
/// taken off.
fn check_directory(text: &str) -> Result<String> {
    let bad = |why: String| invalid(say!("sparse-what-directory"), why);
    let quoted = || format!("{text:?}");
    if text.len() > LONGEST_RULE {
        return Err(bad(say!("sparse-too-long", text = quoted())));
    }
    if text.chars().any(char::is_control) {
        return Err(bad(say!("sparse-control-characters", text = quoted())));
    }
    let trimmed = text.trim_end_matches('/');
    if trimmed.is_empty() {
        return Err(bad(say!("sparse-names-no-directory", text = quoted())));
    }
    if trimmed.starts_with('/') || !bitbraid_core::paths::stays_inside(trimmed) {
        return Err(bad(say!("sparse-not-relative", text = quoted())));
    }
    if trimmed
        .split('/')
        .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(bad(say!("sparse-outside-worktree", text = quoted())));
    }
    if trimmed.contains(['*', '?', '[', ']', '\\']) {
        return Err(bad(say!("sparse-is-pattern", text = quoted())));
    }
    Ok(trimmed.to_owned())
}

/// A pattern as a line of the file git keeps: anything but an empty line
/// or one with control characters, which would split it.
fn check_pattern(text: &str) -> Result<String> {
    let bad = |why: String| invalid(say!("sparse-what-pattern"), why);
    let quoted = || format!("{text:?}");
    if text.len() > LONGEST_RULE {
        return Err(bad(say!("sparse-too-long", text = quoted())));
    }
    if text.chars().any(char::is_control) {
        return Err(bad(say!("sparse-control-characters", text = quoted())));
    }
    if text.trim().is_empty() {
        return Err(bad(say!("sparse-empty", text = quoted())));
    }
    Ok(text.to_owned())
}

fn check_count(count: usize) -> Result<()> {
    if count > MOST_RULES {
        return Err(invalid(
            say!("sparse-what-sparse-checkout"),
            say!("sparse-too-many", most = MOST_RULES),
        ));
    }
    Ok(())
}

/// How the checkout stands: whether it is sparse, how, with what
/// directories or patterns, and how many tracked files are left out.
pub fn status(repo: &Repository) -> Result<SparseStatus> {
    worktree(repo)?;
    // The three settings in one question, each as git reads a yes or no.
    let set = exe::run(
        repo,
        [
            "config",
            "--type=bool",
            "--get-regexp",
            r"^(core\.sparsecheckout|core\.sparsecheckoutcone|index\.sparse)$",
        ],
        None,
    )
    .map(|out| out.stdout)
    .unwrap_or_default();
    // The last value of a key is the one that counts.
    let on = |key: &str| {
        set.lines()
            .rev()
            .filter_map(|line| line.split_once(' '))
            .find(|(name, _)| name.eq_ignore_ascii_case(key))
            .is_some_and(|(_, value)| value.trim() == "true")
    };
    let enabled = on("core.sparsecheckout");
    let mut status = SparseStatus {
        enabled,
        cone: enabled && on("core.sparsecheckoutcone"),
        sparse_index: enabled && on("index.sparse"),
        ..SparseStatus::default()
    };
    if !enabled {
        return Ok(status);
    }
    {
        let listed = exe::run(repo, ["sparse-checkout", "list"], None)?;
        status.rules = listed
            .stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| match line.strip_prefix('"') {
                Some(quoted) => crate::lfs::unquote(quoted)
                    .map(|(text, _)| text)
                    .unwrap_or_else(|| line.to_owned()),
                None => line.to_owned(),
            })
            .collect();
    }
    let (mut tracked, mut outside) = (0u64, 0u64);
    let never = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    exe::run_followed(repo, ["ls-files", "-t"], &[], None, &never, &mut |said| {
        if let exe::Said::Out(line) = said {
            tracked = tracked.saturating_add(1);
            if line.starts_with("S ") {
                outside = outside.saturating_add(1);
            }
        }
    })?;
    status.tracked = tracked;
    status.outside = outside;
    Ok(status)
}

/// The arguments for `rules`, checked, as a checkout in cone mode
/// (`cone_mode`) or by patterns takes them.
fn rule_args(rules: &Rules, cone_mode: bool) -> Result<Vec<String>> {
    match rules {
        Rules::Directories(directories) => {
            check_count(directories.len())?;
            let checked = directories
                .iter()
                .map(|one| check_directory(one))
                .collect::<Result<Vec<_>>>()?;
            if cone_mode {
                Ok(checked)
            } else {
                // A checkout by patterns takes a directory as the pattern
                // of everything below it.
                Ok(checked.into_iter().map(|one| format!("/{one}/")).collect())
            }
        }
        Rules::Patterns(patterns) => {
            if cone_mode {
                return Err(refused(say!("sparse-cone-takes-directories")));
            }
            check_count(patterns.len())?;
            patterns.iter().map(|one| check_pattern(one)).collect()
        }
    }
}

/// Run `git sparse-checkout` with `args`, saying how far it is.
fn run_sparse(repo: &Repository, args: &[String], reporter: &mut Reporter<'_>) -> Result<Changed> {
    // Git takes a file whose times and size are not what the index last
    // noted for one with changes, and leaves it in place. Nothing here
    // writes those notes (the status is read without taking the index's
    // lock), so they are brought up to date first, as `git status` would:
    // a file that was copied or touched is not one with changes. What
    // fails here only leaves more files where they are, and git says so.
    let _ = exe::run(repo, ["update-index", "-q", "--refresh"], None);
    let cancel = reporter.cancel.clone();
    let mut on_line = |line: &str| {
        if let Some(progress) = crate::remote::parse_progress(line) {
            (reporter.on_progress)(progress);
        }
    };
    let out = exe::run_streaming(repo, args, &cancel, &mut on_line)?;
    let mut said = String::new();
    for line in out.stdout.lines().chain(out.stderr.lines()) {
        if line.trim().is_empty() || crate::remote::parse_progress(line).is_some() {
            continue;
        }
        said.push_str(line.trim_end());
        said.push('\n');
    }
    Ok(Changed {
        status: status(repo)?,
        output: said.trim_end().to_owned(),
    })
}

/// Check out only what `rules` say: by directory (cone mode) or by
/// patterns. Turns sparse checkout on when it is off.
pub fn set(repo: &Repository, rules: &Rules, reporter: &mut Reporter<'_>) -> Result<Changed> {
    worktree(repo)?;
    let cone = matches!(rules, Rules::Directories(_));
    let mut args = vec![
        "sparse-checkout".to_owned(),
        "set".to_owned(),
        if cone { "--cone" } else { "--no-cone" }.to_owned(),
        "--end-of-options".to_owned(),
    ];
    args.extend(rule_args(rules, cone)?);
    run_sparse(repo, &args, reporter)
}

/// Check out more: these directories, or the files these patterns take,
/// beside what is checked out already. In a checkout by patterns a
/// directory stands for everything below it.
pub fn add(repo: &Repository, rules: &Rules, reporter: &mut Reporter<'_>) -> Result<Changed> {
    worktree(repo)?;
    if !flag(repo, "core.sparseCheckout") {
        return Err(refused(say!("sparse-not-sparse")));
    }
    let cone = flag(repo, "core.sparseCheckoutCone");
    let more = rule_args(rules, cone)?;
    if more.is_empty() {
        return Err(invalid(
            say!("sparse-what-sparse-checkout"),
            say!("sparse-nothing-to-add"),
        ));
    }
    let mut args = vec![
        "sparse-checkout".to_owned(),
        "add".to_owned(),
        "--end-of-options".to_owned(),
    ];
    args.extend(more);
    run_sparse(repo, &args, reporter)
}

/// Check out every file again. What was chosen is kept by git, for the
/// next time.
pub fn disable(repo: &Repository, reporter: &mut Reporter<'_>) -> Result<Changed> {
    worktree(repo)?;
    run_sparse(
        repo,
        &["sparse-checkout".to_owned(), "disable".to_owned()],
        reporter,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directories_are_checked_as_git_takes_them() {
        assert_eq!(check_directory("a/b/").unwrap(), "a/b");
        assert_eq!(check_directory("-dash").unwrap(), "-dash");
        assert_eq!(check_directory("with space").unwrap(), "with space");
        for bad in [
            "",
            "/",
            "/abs",
            "../up",
            "a/../b",
            "a//b",
            "./a",
            "a/*",
            "x?",
            "[a]",
            "a\\b",
            "new\nline",
            "tab\there",
        ] {
            assert!(check_directory(bad).is_err(), "{bad:?}");
        }
        assert!(check_directory(&"a".repeat(LONGEST_RULE + 1)).is_err());
    }

    #[test]
    fn patterns_are_lines() {
        assert_eq!(check_pattern("/*").unwrap(), "/*");
        assert_eq!(check_pattern("!/a/").unwrap(), "!/a/");
        assert_eq!(check_pattern("-x").unwrap(), "-x");
        for bad in ["", "  ", "a\nb", "a\rb", "\u{0}"] {
            assert!(check_pattern(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn rules_become_arguments() {
        let directories = Rules::Directories(vec!["a/".into(), "b".into()]);
        assert_eq!(rule_args(&directories, true).unwrap(), ["a", "b"]);
        assert_eq!(rule_args(&directories, false).unwrap(), ["/a/", "/b/"]);
        let patterns = Rules::Patterns(vec!["/*".into()]);
        assert!(rule_args(&patterns, true).is_err());
        assert_eq!(rule_args(&patterns, false).unwrap(), ["/*"]);
        let many = Rules::Directories(vec!["a".into(); MOST_RULES + 1]);
        assert!(rule_args(&many, true).is_err());
    }
}
