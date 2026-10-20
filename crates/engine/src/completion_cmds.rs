//! Completion of git commands typed in the terminal: the repository's
//! names it completes with, read once and kept by the window
//! (`terminal.completion_words`), and a line completed with them
//! (`terminal.complete`), which is what the window does with each key and
//! what the command line can ask for.
//!
//! The names are read as the references panel and the changes list read
//! them (`repo.refs`, `repo.status`) and from the index; nothing is run,
//! neither git's nor the shell's own completion.
//!
//! Sources: git-for-each-ref(1), git-status(1) and git-ls-files(1) for
//! what is read; `crate::completion` for how a line is completed.

use std::path::Path;

use bitbraid_git::{ChangeKind, StatusEntry};
use bitbraid_terminal::ShellKind;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    completion::{
        self, ChangedPath, LONGEST_LINE, PathChange, Place, RemoteWords, StashWord, Words,
    },
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "terminal.completion_words",
            label: say!("completion-terminal-completion-words"),
            description: say!("completion-terminal-completion-words-description"),
        },
        completion_words,
    );
    let _ = registry.register(
        CommandSpec {
            id: "terminal.complete",
            label: say!("completion-terminal-complete"),
            description: say!("completion-terminal-complete-description"),
        },
        complete,
    );
}

/// How many tracked paths are listed when nothing is said.
pub const MOST_PATHS: usize = 50_000;
/// How many tracked paths may be asked for.
pub const MOST_PATHS_ASKED: usize = 500_000;

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

fn change_of(kind: ChangeKind) -> PathChange {
    match kind {
        ChangeKind::Added => PathChange::Added,
        ChangeKind::Modified => PathChange::Modified,
        ChangeKind::Deleted => PathChange::Deleted,
        ChangeKind::Renamed => PathChange::Renamed,
        ChangeKind::Copied => PathChange::Copied,
        ChangeKind::TypeChanged => PathChange::TypeChanged,
        ChangeKind::Untracked => PathChange::Untracked,
        ChangeKind::IntentToAdd => PathChange::IntentToAdd,
        ChangeKind::Conflicted => PathChange::Conflicted,
    }
}

/// A changed path as completion offers it: a conflict is what is not
/// staged, whatever the sides say.
pub fn changed_path(entry: &StatusEntry) -> ChangedPath {
    ChangedPath {
        path: entry.path.clone(),
        unstaged: if entry.conflict {
            Some(PathChange::Conflicted)
        } else {
            entry.unstaged.map(change_of)
        },
        staged: if entry.conflict {
            None
        } else {
            entry.staged.map(change_of)
        },
    }
}

/// Put the names of a snapshot of the references into `words`: the branch
/// checked out, branches, remotes and their branches, tags, stashes.
pub fn set_names(words: &mut Words, snapshot: &bitbraid_git::Snapshot) {
    words.head = if snapshot.head.detached {
        None
    } else {
        snapshot.head.branch.clone()
    };
    words.branches = snapshot.branches.iter().map(|b| b.name.clone()).collect();
    words.remotes = snapshot
        .remotes
        .iter()
        .map(|remote| RemoteWords {
            name: remote.name.clone(),
            url: remote.fetch_url.clone(),
            branches: remote.branches.iter().map(|b| b.name.clone()).collect(),
        })
        .collect();
    words.tags = snapshot.tags.iter().map(|tag| tag.name.clone()).collect();
    words.stashes = snapshot
        .stashes
        .iter()
        .map(|stash| StashWord {
            index: stash.index,
            message: stash.message.clone(),
        })
        .collect();
}

/// The names of the open repository; changed paths only when `changes`.
pub fn words_of(
    repo: &bitbraid_git::Repository,
    changes: bool,
    most_paths: usize,
) -> Result<Words, CommandError> {
    let snapshot = bitbraid_git::snapshot(repo)?;
    let top = repo.workdir().unwrap_or_else(|| repo.git_dir());
    let top = bitbraid_core::paths::canonical(top).unwrap_or_else(|_| top.to_path_buf());
    let changes = if changes && repo.workdir().is_some() {
        bitbraid_git::status(repo)?
            .iter()
            .map(changed_path)
            .collect()
    } else {
        Vec::new()
    };
    let (tracked, more_tracked) = bitbraid_git::tracked::tracked_paths(repo, most_paths)?;
    let mut words = Words {
        top: top.to_string_lossy().into_owned(),
        changes,
        tracked,
        more_tracked,
        ..Words::default()
    };
    set_names(&mut words, &snapshot);
    Ok(words)
}

fn completion_words(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let changes: bool = param_or(params, "changes", true)?;
    let most: usize = param_or(params, "most_paths", MOST_PATHS)?;
    if most > MOST_PATHS_ASKED {
        return Err(invalid(
            "most_paths",
            say!("completion-most-paths-too-many", most = MOST_PATHS_ASKED),
        ));
    }
    let words = words_of(ctx.repo()?, changes, most)?;
    Ok(serde_json::to_value(words)?)
}

/// The folder a shell in `folder` is in, from the worktree's `top`: none
/// when it is outside it.
pub fn folder_in(top: &Path, folder: &Path) -> Option<String> {
    let inside = folder.strip_prefix(top).ok()?;
    let parts: Option<Vec<&str>> = inside
        .components()
        .map(|part| match part {
            std::path::Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    Some(parts?.join("/"))
}

fn complete(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let line: String = param(params, "line")?;
    if line.chars().count() > LONGEST_LINE {
        return Err(invalid(
            "line",
            say!("completion-line-too-long", longest = LONGEST_LINE),
        ));
    }
    let folder: Option<String> = param_or(params, "folder", None)?;
    let folder = folder
        .map(|folder| folder.trim().trim_matches('/').to_owned())
        .unwrap_or_default();
    if folder.chars().any(char::is_control)
        || folder.split('/').any(|part| part == ".." || part == ".")
        || !bitbraid_core::paths::stays_inside(&folder)
    {
        return Err(invalid("folder", say!("completion-folder-outside")));
    }
    let shell: Option<String> = param_or(params, "shell", None)?;
    let shell = match shell.as_deref() {
        None => ShellKind::Posix,
        Some(name) => ShellKind::from_name(name)
            .ok_or_else(|| invalid("shell", say!("completion-shell-unknown")))?,
    };
    let words = words_of(ctx.repo()?, true, MOST_PATHS)?;
    let place = Place {
        folder: Some(folder),
        shell,
    };
    let answer = completion::complete(&line, &words, &place);
    Ok(serde_json::to_value(answer)?)
}

#[cfg(test)]
mod tests {
    use super::folder_in;
    use std::path::Path;

    #[test]
    fn a_folder_is_named_from_the_top_or_not_at_all() {
        let top = Path::new("/work/repo");
        assert_eq!(folder_in(top, Path::new("/work/repo")).as_deref(), Some(""));
        assert_eq!(
            folder_in(top, Path::new("/work/repo/src/ui")).as_deref(),
            Some("src/ui")
        );
        assert_eq!(folder_in(top, Path::new("/work/other")), None);
        assert_eq!(folder_in(top, Path::new("/work/repository")), None);
    }
}
