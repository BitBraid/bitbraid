//! Ignoring: a pattern, typed or made from a path, written to
//! the repository's `.gitignore`, this clone's `info/exclude` or the
//! user's own ignore file; the tracked files that ignore rules cover; and
//! taking files out of the index while they stay on disk.
//!
//! A pattern does not stop git from tracking a file it already tracks.
//! `ignore.add` therefore answers with the tracked files that the rules
//! cover since the pattern was added, which `ignore.untrack` takes out of
//! the index. That is staged like any deletion, so it is taken back by
//! unstaging it and is not journalled; a pattern in `.gitignore` is a
//! change to a file of the worktree like any other.
//!
//! Sources: gitignore(5), git-rm(1), git-ls-files(1).

use bitbraid_git::{IgnoreTarget, IgnoreWhat, ignore};
use serde_json::json;

use crate::command::{
    CommandContext, CommandError, CommandSpec, Output, Params, Registry, param, param_or,
};

/// How many paths an answer lists at most; the count is always whole.
const LISTED: usize = 5000;

/// Register the ignore commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "ignore.add",
            label: say!("ignore-add"),
            description: say!("ignore-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "ignore.tracked",
            label: say!("ignore-tracked"),
            description: say!("ignore-tracked-description"),
        },
        tracked,
    );
    let _ = registry.register(
        CommandSpec {
            id: "ignore.untrack",
            label: say!("ignore-untrack"),
            description: say!("ignore-untrack-description"),
        },
        untrack,
    );
}

/// The first `LISTED` of some paths, and how many there are.
fn listed(paths: &[String], limit: usize) -> (Vec<&String>, usize) {
    (paths.iter().take(limit).collect(), paths.len())
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let typed: Option<String> = param_or(params, "pattern", None)?;
    let path: Option<String> = param_or(params, "path", None)?;
    let what: Option<IgnoreWhat> = param_or(params, "what", None)?;
    let target: IgnoreTarget = param_or(params, "target", IgnoreTarget::Repository)?;
    let pattern = match (typed, path) {
        (Some(pattern), None) if what.is_none() => pattern,
        (None, Some(path)) => {
            let what = what.unwrap_or(IgnoreWhat::File);
            ignore::pattern_for(&path, what).ok_or_else(|| CommandError::InvalidParam {
                name: "path".into(),
                detail: match what {
                    IgnoreWhat::Extension => {
                        say!("ignore-no-extension", path = format!("{path:?}"))
                    }
                    _ => say!("ignore-not-a-worktree-path", path = format!("{path:?}")),
                },
            })?
        }
        (Some(_), _) => {
            return Err(CommandError::InvalidParam {
                name: "pattern".into(),
                detail: say!("ignore-pattern-and-path").into(),
            });
        }
        (None, None) => {
            return Err(CommandError::InvalidParam {
                name: "pattern".into(),
                detail: say!("ignore-pattern-missing").into(),
            });
        }
    };
    let repo = ctx.repo()?;
    // What the rules covered before, so that the answer names what this
    // pattern brought. Asking is not worth failing over.
    let before = ignore::tracked_ignored(repo).unwrap_or_default();
    let done = ignore::ignore_add(repo, &pattern, target)?;
    let covered: Vec<String> = ignore::tracked_ignored(repo)
        .unwrap_or_default()
        .into_iter()
        .filter(|path| !before.contains(path))
        .collect();
    let (tracked, tracked_count) = listed(&covered, LISTED);
    Ok(json!({
        "pattern": done.pattern,
        // As text: `json!` cannot write a path that is not UTF-8.
        "file": done.file.display().to_string(),
        "added": done.added,
        "tracked": tracked,
        "tracked_count": tracked_count,
    }))
}

fn tracked(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", LISTED)?;
    let paths = ignore::tracked_ignored(ctx.repo()?)?;
    let (shown, count) = listed(&paths, limit);
    Ok(json!({ "paths": shown, "count": count }))
}

fn untrack(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param(params, "paths")?;
    if paths.is_empty() {
        return Err(CommandError::InvalidParam {
            name: "paths".into(),
            detail: say!("ignore-no-paths").into(),
        });
    }
    let gone = ignore::untrack(ctx.repo()?, &paths)?;
    let (untracked, count) = listed(&gone, LISTED);
    Ok(json!({ "untracked": untracked, "count": count }))
}
