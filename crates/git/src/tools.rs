//! External diff and merge tools, through `git difftool` and `git
//! mergetool`: the tool the user has configured for git, or a command of
//! their own.
//!
//! A command of the user's is handed to git as a tool definition for this
//! one run (`-c difftool.bitbraid.cmd=…`), so it is written the way git
//! documents tool commands: with `$LOCAL` and `$REMOTE`, and for a merge
//! also `$BASE` and `$MERGED`. Git runs it with its shell.
//!
//! A diff tool is started and left running; one that gives up at once
//! (git is told to pass its exit code on) is an error with what it said.
//! A merge tool is waited for, and whether the file is still in conflict
//! is read afterwards. Without a configured tool and without a command
//! nothing is started: git would pick whatever it finds, and that may be
//! an editor waiting on a terminal that is not there.
//!
//! What a merge tool did is told by the file, not by how the tool ended:
//! a command of the user's is run so that it ends with whether the file
//! it was to write holds something else than before (its id as git would
//! store it, `git hash-object --no-filters`, taken before and after; git
//! is the one program its shell surely has), and git is told to trust
//! that: the file counts as resolved when the tool changed it, and as
//! still in conflict when it did not. Several tools end well when they are
//! closed without saving (Meld, Vim's `:qa`), which would otherwise mark
//! the file resolved with its conflict markers in it. Git's own way of
//! telling, for a tool whose exit code it does not trust, is by the time
//! of the file against that of a copy it touches just before the tool
//! starts, which the shells it runs on macOS and the BSDs and dash compare
//! to the second only: a tool that writes the file within that second
//! (KDiff3 with `--auto`, a script) is taken for one that wrote nothing,
//! and git puts the file with the markers back over what it wrote. The
//! copy with the markers that git keeps beside the file (`.orig`) is not
//! kept unless the user's configuration asks for it, as the versions the
//! tool is shown are kept out of the worktree.
//!
//! The tool, and with it the command it stands for, comes from git's
//! configuration, the repository's own included. That is the trust git
//! itself places in a repository's configuration.
//!
//! Sources: git-difftool(1) (`--trust-exit-code`), git-mergetool(1)
//! (`mergetool.keepBackup`, `mergetool.writeToTemp`, and how an untrusted
//! tool is judged by its output file, read in git's `git-mergetool--lib`
//! to understand it: the copy touched before the tool and `test -nt`;
//! not copied), git-hash-object(1) (`--no-filters`), sh(1p) (`eval`,
//! `test -nt`), git-config(1) (`diff.tool`,
//! `diff.guitool`, `merge.tool`, `merge.guitool`, `difftool.<tool>.cmd`,
//! `mergetool.<tool>.cmd`, `mergetool.<tool>.trustExitCode`).

use std::sync::{Arc, atomic::AtomicBool};

use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, exe, sequence, stage};

/// The tools git is configured with, by name.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Configured {
    /// The diff tool: what `git difftool --gui` would start.
    pub diff: Option<String>,
    /// The merge tool: what `git mergetool --gui` would start.
    pub merge: Option<String>,
}

/// Which two versions of a file a diff tool is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum What {
    /// The index and the file in the worktree.
    Unstaged,
    /// The last commit and the index.
    Staged,
    /// A commit and its first parent.
    Commit(Oid),
    /// Two commits, the older first; without an older one, nothing at all
    /// (the newer one reaches the first commit).
    Range(Option<Oid>, Oid),
}

/// The name of the tool definition a command of the user's is given.
const CUSTOM: &str = "bitbraid";

/// Longest command taken.
const MAX_COMMAND: usize = 4096;

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

/// The first of these configuration keys that has a value.
fn first_set(repo: &Repository, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| exe::config_get(repo, key))
}

/// The tools git would use, in the order `--gui` looks for them.
pub fn configured(repo: &Repository) -> Configured {
    Configured {
        diff: first_set(
            repo,
            &["diff.guitool", "merge.guitool", "diff.tool", "merge.tool"],
        ),
        merge: first_set(repo, &["merge.guitool", "merge.tool"]),
    }
}

/// A command of the user's, when there is one worth the name.
fn custom(command: Option<&str>) -> Result<Option<&str>> {
    let Some(command) = command.map(str::trim).filter(|command| !command.is_empty()) else {
        return Ok(None);
    };
    if command.len() > MAX_COMMAND || command.contains(['\0', '\n', '\r']) {
        return Err(Error::Invalid {
            what: say!("tools-what-command"),
            detail: say!("tools-command-unusable").to_owned(),
        });
    }
    Ok(Some(command))
}

/// A merge command of the user's as git is to run it: it ends well when
/// the file it was to write (`$MERGED`) holds something else afterwards
/// than before, whatever the tool ended with. The command is run by an
/// `eval` of its own in a shell of its own, so that whatever is in it (a
/// `#`, an `exit`) stays within it. Git splits a tool's command into words
/// and joins them with a space before it runs it (`eval` of the value
/// unquoted), which this is written to come through.
fn judged_by_the_file(command: &str) -> String {
    let quoted = command.replace('\'', "'\\''");
    format!(
        "bitbraid_before=$(git hash-object --no-filters -- \"$MERGED\" 2>/dev/null); \
         (eval '{quoted}'); \
         test \"$(git hash-object --no-filters -- \"$MERGED\" 2>/dev/null)\" != \"$bitbraid_before\""
    )
}

/// The id of the tree with nothing in it, which a first commit is compared
/// with.
fn empty_tree(repo: &Repository) -> Result<String> {
    let out = exe::run(
        repo,
        ["hash-object", "-t", "tree", "--stdin"],
        Some(b"".as_slice()),
    )?;
    Ok(out.stdout.trim().to_owned())
}

/// Start a diff tool on one file and leave it running: `command` when
/// there is one, else the tool git is configured with. Says which.
pub fn diff(repo: &Repository, what: What, path: &str, command: Option<&str>) -> Result<String> {
    stage::worktree_path(repo, path)?;
    let command = custom(command)?;
    let mut args: Vec<String> = vec!["--literal-pathspecs".to_owned()];
    let name = match command {
        Some(command) => {
            args.push("-c".to_owned());
            args.push(format!("difftool.{CUSTOM}.cmd={command}"));
            args.push("difftool".to_owned());
            args.push(format!("--tool={CUSTOM}"));
            say!("tools-from-preferences").to_owned()
        }
        None => {
            let Some(name) = configured(repo).diff else {
                return Err(refused(
                    say!("tools-what-external-diff"),
                    say!("tools-no-diff-tool"),
                ));
            };
            args.push("difftool".to_owned());
            args.push("--gui".to_owned());
            name
        }
    };
    // A tool that gives up at once (a library missing, an option it does
    // not take) is told by its exit code, which git otherwise ignores.
    args.push("--no-prompt".to_owned());
    args.push("--trust-exit-code".to_owned());
    match what {
        What::Unstaged => {}
        What::Staged => args.push("--cached".to_owned()),
        What::Commit(id) => {
            let hex = id.to_hex();
            let parent = format!("{hex}^");
            let before = exe::run(
                repo,
                ["rev-parse", "--verify", "--quiet", parent.as_str()],
                None,
            )
            .map(|out| out.stdout.trim().to_owned())
            .ok()
            .filter(|id| !id.is_empty());
            args.push(match before {
                Some(parent) => parent,
                None => empty_tree(repo)?,
            });
            args.push(hex);
        }
        What::Range(old, new) => {
            args.push(match old {
                Some(old) => old.to_hex(),
                None => empty_tree(repo)?,
            });
            args.push(new.to_hex());
        }
    }
    args.push("--".to_owned());
    args.push(path.to_owned());
    exe::start_tool(repo, &args)?;
    Ok(name)
}

/// Run a merge tool on one conflicted file and wait for it: `command`
/// when there is one, else the tool git is configured with. True when the
/// file is no longer in conflict afterwards.
pub fn merge(
    repo: &Repository,
    path: &str,
    command: Option<&str>,
    cancel: &Arc<AtomicBool>,
) -> Result<bool> {
    stage::worktree_path(repo, path)?;
    let in_conflict = |repo: &Repository| -> Result<bool> {
        Ok(sequence::conflicts(repo)?
            .iter()
            .any(|conflict| conflict.path == path))
    };
    if !in_conflict(repo)? {
        return Err(refused(
            say!("tools-what-external-merge"),
            say!("tools-not-in-conflict", path),
        ));
    }
    let command = custom(command)?;
    let mut args: Vec<String> = vec![
        "--literal-pathspecs".to_owned(),
        // The versions the tool is given are kept out of the worktree,
        // where they would show up as new files for as long as it is open.
        "-c".to_owned(),
        "mergetool.writeToTemp=true".to_owned(),
    ];
    // Nor is the copy with the markers kept beside the file once it is
    // resolved, unless the user asked git for it.
    if exe::config_get(repo, "mergetool.keepBackup").is_none() {
        args.push("-c".to_owned());
        args.push("mergetool.keepBackup=false".to_owned());
    }
    match command {
        Some(command) => {
            args.push("-c".to_owned());
            args.push(format!(
                "mergetool.{CUSTOM}.cmd={}",
                judged_by_the_file(command)
            ));
            // Whether the tool changed the file says whether it resolved
            // it: several end well when they are closed without saving.
            // That is what the command above ends with.
            args.push("-c".to_owned());
            args.push(format!("mergetool.{CUSTOM}.trustExitCode=true"));
            args.push("mergetool".to_owned());
            args.push(format!("--tool={CUSTOM}"));
        }
        None => {
            if configured(repo).merge.is_none() {
                return Err(refused(
                    say!("tools-what-external-merge"),
                    say!("tools-no-merge-tool"),
                ));
            }
            args.push("mergetool".to_owned());
            args.push("--gui".to_owned());
        }
    }
    args.push("--no-prompt".to_owned());
    args.push("--".to_owned());
    args.push(path.to_owned());
    match exe::run_tool(repo, &args, cancel) {
        // A tool that ends badly has resolved nothing, which the index
        // says better than its exit code.
        Ok(_) | Err(Error::Command { .. }) => {}
        Err(other) => return Err(other),
    }
    Ok(!in_conflict(repo)?)
}

#[cfg(test)]
mod tests {
    use super::custom;

    #[test]
    fn a_command_is_one_line_of_reasonable_length() {
        assert_eq!(custom(None).unwrap(), None);
        assert_eq!(custom(Some("  \t ")).unwrap(), None);
        assert_eq!(
            custom(Some(" meld \"$LOCAL\" \"$REMOTE\" ")).unwrap(),
            Some("meld \"$LOCAL\" \"$REMOTE\"")
        );
        assert!(custom(Some("one\ntwo")).is_err());
        assert!(custom(Some("nul\0l")).is_err());
        assert!(custom(Some(&"x".repeat(5000))).is_err());
    }

    /// What git is handed, run by a shell as git runs it (split into words
    /// and given to `eval`), with `$MERGED` changed or not: how it ends.
    #[cfg(unix)]
    #[test]
    fn a_merge_command_ends_with_whether_it_changed_the_file() {
        use std::process::Command;
        let dir = std::env::temp_dir().join(format!("bitbraid-judged-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let merged = dir.join("merged");
        let run = |command: &str| {
            std::fs::write(&merged, "<<<<<<< ours\n").unwrap();
            let judged = super::judged_by_the_file(command);
            Command::new("sh")
                .args(["-c", "( eval $cmd )"])
                .env("cmd", &judged)
                .env("MERGED", &merged)
                .status()
                .unwrap()
                .success()
        };
        // Written at once, in the same second: resolved.
        assert!(run("printf 'both\\n' > \"$MERGED\""));
        // Ended well without writing, or written with what was there.
        assert!(!run("true"));
        assert!(!run(
            "cat \"$MERGED\" > \"$MERGED.x\" && cat \"$MERGED.x\" > \"$MERGED\""
        ));
        // Ended badly after writing: what it wrote is what counts.
        assert!(run("echo it > \"$MERGED\"; exit 3"));
        // Quotes, a comment and an exit stay within the command.
        assert!(run(
            "printf '%s\\n' \"it's\" > \"$MERGED\" # a comment; true"
        ));
        assert!(!run("exit 0"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
