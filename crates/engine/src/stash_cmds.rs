//! Stash commands: set local changes aside, bring them back, drop
//! them. A stash is named by its commit id; without one, the newest is
//! meant.
//!
//! Sources: git-stash(1).

use bitbraid_git::{Oid, Repository, StashOptions, stash};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "stash.push",
            label: say!("stash-push"),
            description: say!("stash-push-description"),
        },
        push,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.apply",
            label: say!("stash-apply"),
            description: say!("stash-apply-description"),
        },
        apply,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.pop",
            label: say!("stash-pop"),
            description: say!("stash-pop-description"),
        },
        pop,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.drop",
            label: say!("stash-drop"),
            description: say!("stash-drop-description"),
        },
        drop_stash,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.branch",
            label: say!("stash-branch"),
            description: say!("stash-branch-description"),
        },
        branch,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.reword",
            label: say!("stash-reword"),
            description: say!("stash-reword-description"),
        },
        reword,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.show",
            label: say!("stash-show"),
            description: say!("stash-show-description"),
        },
        show,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stash.list",
            label: say!("stash-list"),
            description: say!("stash-list-description"),
        },
        list,
    );
}

/// The stash a command is about: the `stash` parameter as a commit id, or
/// the newest one.
fn which(repo: &Repository, params: &Params) -> Result<Oid, CommandError> {
    let given: Option<String> = param_or(params, "stash", None)?;
    let id = match given {
        Some(text) => Some(
            text.parse::<Oid>()
                .map_err(|_| CommandError::InvalidParam {
                    name: "stash".to_owned(),
                    detail: say!("stash-not-an-id").to_owned(),
                })?,
        ),
        None => None,
    };
    Ok(stash::resolve(repo, id)?)
}

fn push(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let options = StashOptions {
        message: param_or(params, "message", None)?,
        include_untracked: param_or(params, "include_untracked", false)?,
        keep_index: param_or(params, "keep_index", false)?,
        staged_only: param_or(params, "staged_only", false)?,
    };
    let made = stash::stash_push(ctx.repo()?, &options)?;
    if let Some(id) = made {
        ctx.record(
            say!("stash-entry-push").to_owned(),
            Change::StashPush { stash: id, options },
        )?;
    }
    Ok(json!({ "stash": made }))
}

fn apply(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    let index: bool = param_or(params, "index", false)?;
    let applied = stash::stash_apply_recorded(repo, id, false, index)?;
    let undoable = applied.is_some();
    if let Some(applied) = applied {
        ctx.record(
            say!("stash-entry-apply").to_owned(),
            Change::StashApply { applied },
        )?;
    }
    Ok(json!({ "applied": id, "undoable": undoable }))
}

fn pop(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    let index: bool = param_or(params, "index", false)?;
    let applied = stash::stash_apply_recorded(repo, id, true, index)?;
    let undoable = applied.is_some();
    if let Some(applied) = applied {
        ctx.record(
            say!("stash-entry-pop").to_owned(),
            Change::StashApply { applied },
        )?;
    }
    Ok(json!({ "popped": id, "undoable": undoable }))
}

fn show(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    let files = stash::stash_files(
        repo,
        id,
        &bitbraid_git::DiffOptions {
            context: param_or(params, "context", 3)?,
            paths: Vec::new(),
            detect_renames: true,
            whitespace: bitbraid_git::Whitespace::Exact,
            max_bytes: None,
        },
    )?;
    Ok(json!({ "stash": id, "files": files }))
}

fn drop_stash(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    let message = stash::stash_drop(repo, id)?;
    ctx.record(
        say!("stash-entry-drop").to_owned(),
        Change::StashDrop { stash: id, message },
    )?;
    Ok(json!({ "dropped": id }))
}

fn reword(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let message: String = param(params, "message")?;
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    let (after, before_message) = stash::stash_reword(repo, id, &message)?;
    let after_message = stash::stash_list(ctx.repo()?)?
        .into_iter()
        .find(|entry| entry.id == after)
        .map(|entry| entry.message)
        .unwrap_or_default();
    ctx.record(
        say!("stash-entry-reword").to_owned(),
        Change::StashReword {
            before: id,
            before_message,
            after,
            after_message,
        },
    )?;
    Ok(json!({ "stash": after, "was": id }))
}

fn branch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let repo = ctx.repo()?;
    let id = which(repo, params)?;
    // Two steps, each journalled as what it is, so that undoing them one
    // after the other takes the whole of it back: the branch where the
    // stash was made, checked out; then the pop there, with what was
    // staged staged again, which is what `git stash branch` does.
    let (target, before) = stash::stash_branch_start(repo, id, &name)?;
    ctx.record(
        say!("stash-entry-branch-create", name = name.as_str()),
        Change::BranchCreate {
            name: name.clone(),
            target,
            switched_from: Some(before),
        },
    )?;
    let applied = stash::stash_apply_recorded(ctx.repo()?, id, true, true).map_err(|err| {
        // The branch is there and says so in the journal; the stash is
        // still in the list.
        CommandError::Git(bitbraid_git::Error::Refused {
            what: say!("stash-refused"),
            detail: say!(
                "stash-branch-not-popped",
                name = name.as_str(),
                error = err.to_string()
            ),
        })
    })?;
    if let Some(applied) = applied {
        ctx.record(
            say!("stash-entry-branch-pop", name = name.as_str()),
            Change::StashApply { applied },
        )?;
    }
    Ok(json!({ "branch": name, "popped": id }))
}

fn list(ctx: &mut CommandContext, _params: &Params) -> Result<Output, CommandError> {
    let entries = stash::stash_list(ctx.repo()?)?;
    Ok(json!({ "stashes": entries }))
}

fn refused(detail: &str) -> CommandError {
    CommandError::InvalidParam {
        name: "undo".to_owned(),
        detail: detail.to_owned(),
    }
}

/// Reverse a stash change.
pub(crate) fn undo_change(repo: &Repository, change: &Change) -> Result<(), CommandError> {
    match change {
        Change::StashPush { stash: id, options } => {
            // Only the newest stash goes back the way it came: an older one
            // would land on top of whatever was done since.
            if stash::stash_list(repo)?.first().map(|entry| entry.id) != Some(*id) {
                return Err(refused(say!("stash-not-newest")));
            }
            // What stayed staged is still staged; restoring the index on
            // top of itself would only conflict.
            stash::stash_apply(repo, *id, true, !options.keep_index)?;
        }
        Change::StashDrop { stash: id, message } => {
            stash::stash_store(repo, *id, message)?;
        }
        Change::StashApply { applied } => stash::stash_unapply(repo, applied)?,
        Change::StashReword {
            before,
            before_message,
            after,
            ..
        } => stash::stash_replace(repo, *after, *before, before_message)?,
        _ => {}
    }
    Ok(())
}

/// Do a stash change again. A stash made again is a new commit, so the
/// change to remember is a new one.
pub(crate) fn redo_change(
    repo: &Repository,
    change: &Change,
) -> Result<Option<Change>, CommandError> {
    match change {
        Change::StashPush { options, .. } => {
            let made = stash::stash_push(repo, options)?
                .ok_or_else(|| refused(say!("stash-nothing-any-more")))?;
            Ok(Some(Change::StashPush {
                stash: made,
                options: options.clone(),
            }))
        }
        Change::StashDrop { stash: id, .. } => {
            stash::stash_drop(repo, *id)?;
            Ok(None)
        }
        Change::StashReword {
            before,
            after,
            after_message,
            ..
        } => {
            stash::stash_replace(repo, *before, *after, after_message)?;
            Ok(None)
        }
        // Applied afresh: what the files hold before it may differ.
        Change::StashApply { applied } => {
            let again =
                stash::stash_apply_recorded(repo, applied.stash, applied.popped, applied.index)?;
            Ok(again.map(|applied| Change::StashApply { applied }))
        }
        _ => Ok(None),
    }
}
