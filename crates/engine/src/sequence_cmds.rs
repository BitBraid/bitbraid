//! Commands for an operation that is in the middle: what it is, go
//! on, skip, give up; and the conflicted files one at a time.
//!
//! Sources: git-merge(1), git-rebase(1), git-cherry-pick(1),
//! git-revert(1), git-am(1).

use bitbraid_git::{
    ResetMode,
    sequence::{self, Outcome, Side},
};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
    pending,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "op.status",
            label: say!("sequence-status"),
            description: say!("sequence-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.list",
            label: say!("sequence-conflicts"),
            description: say!("sequence-conflicts-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "op.continue",
            label: say!("sequence-continue"),
            description: say!("sequence-continue-description"),
        },
        go_on,
    );
    let _ = registry.register(
        CommandSpec {
            id: "op.skip",
            label: say!("sequence-skip"),
            description: say!("sequence-skip-description"),
        },
        skip,
    );
    let _ = registry.register(
        CommandSpec {
            id: "op.abort",
            label: say!("sequence-abort"),
            description: say!("sequence-abort-description"),
        },
        abort,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.take",
            label: say!("sequence-take"),
            description: say!("sequence-take-description"),
        },
        take,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.resolve",
            label: say!("sequence-resolve"),
            description: say!("sequence-resolve-description"),
        },
        resolve,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.detail",
            label: say!("sequence-detail"),
            description: say!("sequence-detail-description"),
        },
        detail,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.write",
            label: say!("sequence-write"),
            description: say!("sequence-write-description"),
        },
        write,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.reopen",
            label: say!("sequence-reopen"),
            description: say!("sequence-reopen-description"),
        },
        reopen,
    );
    let _ = registry.register(
        CommandSpec {
            id: "conflict.resolved",
            label: say!("sequence-resolved"),
            description: say!("sequence-resolved-description"),
        },
        resolved,
    );
}

fn resolved(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(json!({ "paths": sequence::resolved(ctx.repo()?)? }))
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(json!({ "operation": sequence::in_progress(ctx.repo()?)? }))
}

/// The files in conflict, whether or not an operation is in progress: a
/// patch applied with a merge leaves conflicts and no operation.
fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(json!({ "conflicts": sequence::conflicts(ctx.repo()?)? }))
}

/// Run one step of the operation and settle what comes of it: when it is
/// over and was begun from here, it becomes an entry of the journal.
fn step(
    ctx: &mut CommandContext,
    run: impl FnOnce(&bitbraid_git::Repository) -> bitbraid_git::Result<Outcome>,
) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let note = sequence::in_progress(repo)?
        .as_ref()
        .and_then(|operation| pending::load_for(repo, operation));
    let outcome = run(repo)?;
    let operation = sequence::in_progress(repo)?;
    if outcome.finished {
        pending::clear(repo);
        if let Some(note) = note {
            let after = bitbraid_git::branch::head_state(repo)?;
            if after != note.before {
                ctx.record(
                    note.label,
                    Change::HeadMoved {
                        before: note.before,
                        after,
                        mode: ResetMode::Keep,
                        backups: Vec::new(),
                    },
                )?;
            }
        }
    }
    Ok(json!({
        "finished": outcome.finished,
        "message": outcome.message,
        "operation": operation,
    }))
}

fn go_on(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    step(ctx, sequence::continue_operation)
}

fn skip(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    step(ctx, sequence::skip_operation)
}

fn abort(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    sequence::abort_operation(repo)?;
    pending::clear(repo);
    Ok(json!({ "aborted": true }))
}

fn take(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let side: Side = param(params, "side")?;
    sequence::take_side(ctx.repo()?, &path, side)?;
    Ok(json!({ "resolved": path }))
}

fn resolve(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let allow_markers: bool = param_or(params, "allow_markers", false)?;
    sequence::mark_resolved(ctx.repo()?, &path, allow_markers)?;
    Ok(json!({ "resolved": path }))
}

fn detail(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    Ok(serde_json::to_value(sequence::detail(ctx.repo()?, &path)?)?)
}

fn write(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let text: String = param(params, "text")?;
    let allow_markers: bool = param_or(params, "allow_markers", false)?;
    sequence::write_resolution(ctx.repo()?, &path, &text, allow_markers)?;
    Ok(json!({ "resolved": path }))
}

fn reopen(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    sequence::reopen(ctx.repo()?, &path)?;
    Ok(json!({ "reopened": path }))
}
