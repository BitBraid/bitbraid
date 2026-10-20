//! Worktree commands: list, what is uncommitted in each, add, remove,
//! move, lock, unlock, prune.
//!
//! Sources: git-worktree(1).

use bitbraid_git::worktree::{self, Start};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "worktree.list",
            label: say!("worktree-list"),
            description: say!("worktree-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.changes",
            label: say!("worktree-changes"),
            description: say!("worktree-changes-description"),
        },
        changes,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.add",
            label: say!("worktree-add"),
            description: say!("worktree-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.remove",
            label: say!("worktree-remove"),
            description: say!("worktree-remove-description"),
        },
        remove,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.lock",
            label: say!("worktree-lock"),
            description: say!("worktree-lock-description"),
        },
        lock,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.move",
            label: say!("worktree-move"),
            description: say!("worktree-move-description"),
        },
        move_to,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.unlock",
            label: say!("worktree-unlock"),
            description: say!("worktree-unlock-description"),
        },
        unlock,
    );
    let _ = registry.register(
        CommandSpec {
            id: "worktree.prune",
            label: say!("worktree-prune"),
            description: say!("worktree-prune-description"),
        },
        prune,
    );
}

fn list(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let changes: bool = param_or(params, "changes", true)?;
    Ok(serde_json::to_value(worktree::list(ctx.repo()?, changes)?)?)
}

/// What is uncommitted in the other worktrees, for their rows in the graph.
/// One `git status` for each worktree that is not known by its stamp and
/// that may be looked at now, one after the other, stopped when the
/// command is.
fn changes(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let known: std::collections::HashMap<String, String> =
        param_or(params, "known", Default::default())?;
    let skip: std::collections::HashSet<String> = param_or(params, "skip", Default::default())?;
    let at_most: Option<usize> = param_or(params, "at_most", None)?;
    for (name, count) in [("known", known.len()), ("skip", skip.len())] {
        if count > 4096 {
            return Err(CommandError::InvalidParam {
                name: name.to_owned(),
                detail: say!("worktree-too-many").to_owned(),
            });
        }
    }
    let scope = bitbraid_git::wip::Scope { skip, at_most };
    let cancel = ctx.cancel.clone();
    Ok(serde_json::to_value(bitbraid_git::wip::scan(
        ctx.repo()?,
        &known,
        &scope,
        &cancel,
    )?)?)
}

/// What the new worktree checks out: exactly one of the three ways.
fn start(params: &Params) -> Result<Start, CommandError> {
    let branch: Option<String> = param_or(params, "branch", None)?;
    let new_branch: Option<String> = param_or(params, "new_branch", None)?;
    let detach: bool = param_or(params, "detach", false)?;
    let start: Option<String> = param_or(params, "start", None)?;
    let one_of = |detail: &str| CommandError::InvalidParam {
        name: "branch".to_owned(),
        detail: detail.to_owned(),
    };
    match (branch, new_branch, detach) {
        (Some(name), None, false) if start.is_none() => Ok(Start::Branch(name)),
        (Some(_), None, false) => Err(one_of(say!("worktree-branch-with-start"))),
        (None, Some(name), false) => Ok(Start::NewBranch { name, start }),
        (None, None, true) => Ok(Start::Detached(start.unwrap_or_else(|| "HEAD".to_owned()))),
        (None, None, false) => Err(one_of(say!("worktree-start-missing"))),
        _ => Err(one_of(say!("worktree-start-several"))),
    }
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let start = start(params)?;
    let made = reporting(ctx, |repo, reporter| {
        worktree::add(repo, &path, &start, reporter)
    })?;
    Ok(serde_json::to_value(made)?)
}

fn remove(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let force: bool = param_or(params, "force", false)?;
    let delete_branch: bool = param_or(params, "delete_branch", false)?;
    let removed = worktree::remove(ctx.repo()?, &path, force, delete_branch)?;
    if let Some(deleted) = &removed.deleted {
        // The worktree is gone for good; the branch comes back with undo.
        ctx.record(
            say!("worktree-entry-delete-branch", name = deleted.name.as_str()),
            Change::BranchDelete {
                name: deleted.name.clone(),
                target: deleted.target,
                upstream: deleted.upstream.clone(),
            },
        )?;
    }
    Ok(serde_json::to_value(removed)?)
}

fn lock(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let reason: Option<String> = param_or(params, "reason", None)?;
    Ok(serde_json::to_value(worktree::lock(
        ctx.repo()?,
        &path,
        reason.as_deref(),
    )?)?)
}

fn move_to(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let to: String = param(params, "to")?;
    Ok(serde_json::to_value(worktree::move_to(
        ctx.repo()?,
        &path,
        &to,
    )?)?)
}

fn unlock(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    Ok(serde_json::to_value(worktree::unlock(ctx.repo()?, &path)?)?)
}

fn prune(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    // As text: a path that is not UTF-8 cannot be written as it is.
    let pruned: Vec<String> = worktree::prune(ctx.repo()?)?
        .iter()
        .map(|path| path.display().to_string())
        .collect();
    Ok(json!({ "pruned": pruned }))
}
