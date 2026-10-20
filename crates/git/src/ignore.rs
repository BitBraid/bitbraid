//! Ignoring: a pattern for a file, a folder or an extension, written to
//! the repository's `.gitignore`, to this clone's `info/exclude` or to the
//! user's ignore file for every repository; the tracked files a pattern
//! covers (which git goes on tracking); and taking a file out of the index
//! while it stays on disk.
//!
//! A pattern made from a path is written so that it means that path and
//! nothing else: anchored at the top with a leading slash, and with every
//! character that a pattern reads as something else (`*`, `?`, `[`, `\`, a
//! blank at the end) escaped with a backslash. Behind the slash, a `#` or
//! `!` the path begins with means itself.
//!
//! Where the user's ignore file is: `core.excludesFile` when it is set,
//! else `git/ignore` under `$XDG_CONFIG_HOME`, or under `$HOME/.config`
//! when that is not set or empty, as gitignore(5) has it.
//!
//! Sources: gitignore(5), git-config(1) (`core.excludesFile`),
//! git-ls-files(1) (`--cached --ignored --exclude-standard`), git-rm(1)
//! (`--cached`, `--pathspec-from-file`), git-check-ignore(1).

use std::{ffi::OsStr, fs, io::Write as _, path::PathBuf};

use serde::{Deserialize, Serialize};

use crate::{Error, Repository, Result, exe, stage::check_rel};

/// Where an ignore pattern goes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreTarget {
    /// `.gitignore` at the worktree root (shared through git).
    Repository,
    /// `.git/info/exclude` (this clone only).
    Local,
    /// The user's ignore file, which holds for every repository.
    Global,
}

/// What of a path a pattern is made for.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IgnoreWhat {
    /// The file itself, where it is.
    File,
    /// The folder at the path, with everything in it.
    Folder,
    /// Every file with the path's extension, anywhere.
    Extension,
}

fn invalid(detail: impl Into<String>) -> Error {
    Error::Invalid {
        what: say!("ignore-what-pattern"),
        detail: detail.into(),
    }
}

/// Escape a name for use inside a pattern: what a pattern would read as a
/// wildcard, a class or an escape is taken literally.
fn escaped(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        if matches!(c, '*' | '?' | '[' | '\\') {
            out.push('\\');
        }
        out.push(c);
    }
    // Blanks at the end of a pattern are dropped unless each is escaped.
    let kept = out.trim_end_matches(' ');
    let blanks = out.len().saturating_sub(kept.len());
    let mut out = kept.to_owned();
    for _ in 0..blanks {
        out.push_str("\\ ");
    }
    out
}

/// The pattern that ignores `path` (relative to the worktree, with `/`
/// between its parts) as a file, as a folder, or by its extension. None
/// where there is no such pattern: an empty path, one that leaves the
/// worktree, one with a line break in it (a pattern is one line), a
/// folder or an extension where the path has none.
pub fn pattern_for(path: &str, what: IgnoreWhat) -> Option<String> {
    let path = path.trim_end_matches('/');
    if check_rel(path).is_err() || path.contains(['\n', '\r', '\0']) {
        return None;
    }
    match what {
        // The slash in front anchors it: without one, a name alone would
        // match in every folder.
        IgnoreWhat::File => Some(format!("/{}", escaped(path))),
        IgnoreWhat::Folder => Some(format!("/{}/", escaped(path))),
        IgnoreWhat::Extension => {
            let name = path.rsplit('/').next()?;
            let (stem, extension) = name.rsplit_once('.')?;
            // ".gitignore" has no extension, and neither has "name.".
            if stem.is_empty() || extension.is_empty() {
                return None;
            }
            Some(format!("*.{}", escaped(extension)))
        }
    }
}

/// A pattern as it is written to a file: one line, without the blanks
/// around it that mean nothing (a blank at the end that is escaped stays).
fn cleaned(pattern: &str) -> Result<String> {
    if pattern.contains(['\n', '\r', '\0']) {
        return Err(invalid(say!("ignore-one-line")));
    }
    let start = pattern.trim_start();
    let mut kept = start.trim_end().to_owned();
    // "name\ " ends in an escaped blank, which trimming just took.
    let taken = start.get(kept.len()..).unwrap_or_default();
    if taken.starts_with(' ') && ends_in_lone_backslash(&kept) {
        kept.push(' ');
    }
    if kept.is_empty() {
        return Err(invalid(say!("ignore-empty")));
    }
    if kept.starts_with('#') {
        return Err(invalid(say!("ignore-comment")));
    }
    if kept == "!" || kept == "/" {
        return Err(invalid(say!(
            "ignore-matches-nothing",
            kept = format!("{kept:?}")
        )));
    }
    Ok(kept)
}

/// Whether the text ends in a backslash that is not itself escaped.
fn ends_in_lone_backslash(text: &str) -> bool {
    let trailing = text.chars().rev().take_while(|c| *c == '\\').count();
    trailing % 2 == 1
}

/// Where git looks for the user's own ignore file when `core.excludesFile`
/// is not set, from the two variables it goes by.
pub fn default_global_file(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Option<PathBuf> {
    let base = match xdg_config_home.filter(|value| !value.is_empty()) {
        Some(xdg) => PathBuf::from(xdg),
        None => PathBuf::from(home.filter(|value| !value.is_empty())?).join(".config"),
    };
    base.is_absolute().then(|| base.join("git").join("ignore"))
}

/// A path without the steps up that undo a step down before them: a
/// linked worktree names what it shares as `worktrees/<id>/../..`.
fn tidy(path: &std::path::Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir
                if matches!(out.components().next_back(), Some(Component::Normal(_))) =>
            {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// The file a target stands for in this repository.
pub fn ignore_file(repo: &Repository, target: IgnoreTarget) -> Result<PathBuf> {
    match target {
        IgnoreTarget::Repository => repo
            .workdir()
            .map(|workdir| workdir.join(".gitignore"))
            .ok_or_else(|| Error::Invalid {
                what: say!("ignore-what-repository"),
                detail: say!("ignore-bare").to_owned(),
            }),
        // Shared by the worktrees of a repository, as git reads it.
        IgnoreTarget::Local => Ok(tidy(&repo.gix().common_dir().join("info").join("exclude"))),
        IgnoreTarget::Global => exe::config_path(repo, "core.excludesFile")
            .or_else(|| {
                default_global_file(
                    std::env::var_os("XDG_CONFIG_HOME").as_deref(),
                    bitbraid_core::paths::home()
                        .as_deref()
                        .map(std::path::Path::as_os_str),
                )
            })
            .ok_or_else(|| Error::Refused {
                what: say!("ignore-what-ignore"),
                detail: say!("ignore-no-place").to_owned(),
            }),
    }
}

/// What adding a pattern did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ignored {
    /// The pattern as it stands in the file.
    pub pattern: String,
    /// The file it is in.
    pub file: PathBuf,
    /// False when the file had the pattern already and was left alone.
    pub added: bool,
}

/// Append a pattern to an ignore file, creating the file when it is not
/// there. What the file holds is never rewritten: the line is added at its
/// end, with the line ending the file uses.
pub fn ignore_add(repo: &Repository, pattern: &str, target: IgnoreTarget) -> Result<Ignored> {
    let pattern = cleaned(pattern)?;
    let file = ignore_file(repo, target)?;
    let io = |context: &'static str| {
        move |err: std::io::Error| Error::Git {
            context,
            source: Box::new(err),
        }
    };
    let existing = match fs::read(&file) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(err) => return Err(io(say!("ignore-reading"))(err)),
    };
    let already = existing
        .split(|byte| *byte == b'\n')
        .any(|line| line.strip_suffix(b"\r").unwrap_or(line) == pattern.as_bytes());
    if already {
        return Ok(Ignored {
            pattern,
            file,
            added: false,
        });
    }
    if let Some(parent) = file.parent() {
        fs::create_dir_all(parent).map_err(io(say!("ignore-creating-folder")))?;
    }
    let ending: &[u8] = if existing.windows(2).any(|pair| pair == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let mut line = Vec::new();
    if !existing.is_empty() && !existing.ends_with(b"\n") {
        line.extend_from_slice(ending);
    }
    line.extend_from_slice(pattern.as_bytes());
    line.extend_from_slice(ending);
    fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&file)
        .and_then(|mut out| out.write_all(&line))
        .map_err(io(say!("ignore-writing")))?;
    Ok(Ignored {
        pattern,
        file,
        added: true,
    })
}

/// The files git tracks although an ignore rule covers them: adding a
/// pattern does not stop a file from being tracked. As `git ls-files
/// --cached --ignored --exclude-standard` lists them.
pub fn tracked_ignored(repo: &Repository) -> Result<Vec<String>> {
    let out = exe::run(
        repo,
        [
            "ls-files",
            "-z",
            "--cached",
            "--ignored",
            "--exclude-standard",
        ],
        None,
    )?;
    Ok(out
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect())
}

/// Stop tracking paths: they leave the index and stay on disk, as `git rm
/// --cached -r` does it. A folder stands for the tracked files in it. The
/// change is staged like any deletion, and unstaging it takes it back.
pub fn untrack(repo: &Repository, paths: &[String]) -> Result<Vec<String>> {
    if paths.is_empty() {
        return Err(Error::Invalid {
            what: say!("ignore-what-paths"),
            detail: say!("ignore-empty").to_owned(),
        });
    }
    let mut stdin = Vec::new();
    for path in paths {
        check_rel(path)?;
        if path.contains('\0') {
            return Err(Error::Invalid {
                what: say!("ignore-what-path"),
                detail: say!("ignore-no-nul").to_owned(),
            });
        }
        stdin.extend_from_slice(path.as_bytes());
        stdin.push(0);
    }
    if repo.workdir().is_none() {
        return Err(Error::Invalid {
            what: say!("ignore-what-repository"),
            detail: say!("ignore-bare").to_owned(),
        });
    }
    let before = tracked(repo)?;
    exe::run(
        repo,
        [
            "--literal-pathspecs",
            "rm",
            "--cached",
            "-r",
            "-q",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ],
        Some(&stdin),
    )?;
    // What left the index: a folder stood for the files in it.
    let after: std::collections::HashSet<String> = tracked(repo)?.into_iter().collect();
    Ok(before
        .into_iter()
        .filter(|path| !after.contains(path))
        .collect())
}

/// Every tracked file, in the order of the index.
fn tracked(repo: &Repository) -> Result<Vec<String>> {
    let out = exe::run(repo, ["ls-files", "-z", "--cached"], None)?;
    let mut paths: Vec<String> = out
        .stdout
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect();
    // A path in conflict is listed once per stage.
    paths.dedup();
    Ok(paths)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_pattern_for_a_path_is_anchored_and_escaped() {
        let file = |path| pattern_for(path, IgnoreWhat::File);
        let folder = |path| pattern_for(path, IgnoreWhat::Folder);
        let extension = |path| pattern_for(path, IgnoreWhat::Extension);
        assert_eq!(file("notes.txt").as_deref(), Some("/notes.txt"));
        assert_eq!(file("a/b/c.rs").as_deref(), Some("/a/b/c.rs"));
        assert_eq!(file("#hash").as_deref(), Some("/#hash"));
        assert_eq!(file("a*b?[c]\\d").as_deref(), Some("/a\\*b\\?\\[c]\\\\d"));
        assert_eq!(
            file("ends in blanks  ").as_deref(),
            Some("/ends in blanks\\ \\ ")
        );
        assert_eq!(folder("target").as_deref(), Some("/target/"));
        assert_eq!(folder("a/b/").as_deref(), Some("/a/b/"));
        assert_eq!(extension("a/b/c.log").as_deref(), Some("*.log"));
        assert_eq!(extension("archive.tar.gz").as_deref(), Some("*.gz"));
        assert_eq!(extension("odd.a*b").as_deref(), Some("*.a\\*b"));
        assert_eq!(extension(".gitignore"), None);
        assert_eq!(extension("Makefile"), None);
        assert_eq!(extension("dir.d/Makefile"), None);
        assert_eq!(extension("name."), None);
        for hostile in [
            "",
            "/abs",
            "../up",
            "a/../../b",
            "two\nlines",
            "cr\rhere",
            "nul\0",
        ] {
            assert_eq!(file(hostile), None, "{hostile:?}");
            assert_eq!(folder(hostile), None, "{hostile:?}");
            assert_eq!(extension(hostile), None, "{hostile:?}");
        }
    }

    #[test]
    fn a_typed_pattern_is_one_line_that_ignores_something() {
        assert_eq!(cleaned("  *.log  ").unwrap(), "*.log");
        assert_eq!(cleaned("!keep.log").unwrap(), "!keep.log");
        assert_eq!(cleaned("\\#name").unwrap(), "\\#name");
        // An escaped blank at the end is part of the pattern.
        assert_eq!(cleaned("name\\ ").unwrap(), "name\\ ");
        assert_eq!(cleaned("name\\   ").unwrap(), "name\\ ");
        // A backslash that is itself escaped escapes no blank.
        assert_eq!(cleaned("name\\\\ ").unwrap(), "name\\\\");
        for refused in [
            "",
            "   ",
            "\t",
            "#comment",
            "  # comment",
            "a\nb",
            "a\r",
            "a\0",
            "!",
            "/",
        ] {
            assert!(cleaned(refused).is_err(), "{refused:?}");
        }
    }

    #[test]
    fn steps_up_that_undo_steps_down_are_taken_out_of_a_path() {
        let tidied = |path: &str| tidy(std::path::Path::new(path));
        assert_eq!(
            tidied("/r/.git/worktrees/w/../../info/exclude"),
            PathBuf::from("/r/.git/info/exclude")
        );
        assert_eq!(tidied("/r/./.git/info"), PathBuf::from("/r/.git/info"));
        // What cannot be undone stays as it is.
        assert_eq!(tidied("../x"), PathBuf::from("../x"));
        assert_eq!(tidied("/../x"), PathBuf::from("/../x"));
    }

    #[test]
    fn the_users_ignore_file_is_where_git_looks_for_it() {
        let os = |text: &'static str| Some(OsStr::new(text));
        // A full path begins with a drive on Windows.
        let (config, home) = if cfg!(windows) {
            (r"C:\x\config", r"C:\home\u")
        } else {
            ("/x/config", "/home/u")
        };
        let ignore = |base: &str| PathBuf::from(base).join("git").join("ignore");
        assert_eq!(
            default_global_file(os(config), os(home)),
            Some(ignore(config))
        );
        let in_home = Some(
            PathBuf::from(home)
                .join(".config")
                .join("git")
                .join("ignore"),
        );
        assert_eq!(default_global_file(None, os(home)), in_home);
        assert_eq!(default_global_file(os(""), os(home)), in_home);
        assert_eq!(default_global_file(None, None), None);
        assert_eq!(default_global_file(os(""), os("")), None);
        // A relative place is no place.
        assert_eq!(default_global_file(os("relative"), os(home)), None);
    }
}
