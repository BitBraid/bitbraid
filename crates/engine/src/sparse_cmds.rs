//! Sparse checkout: how it stands, choosing the part of the
//! files to check out by directory or by patterns, adding to it, and
//! turning it off. Changing it updates the worktree, which reports how
//! far it is and is stopped through the context's flag.
//!
//! Sources: git-sparse-checkout(1), git-ls-files(1).

use bitbraid_git::sparse::{self, Rules};

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param_or},
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "sparse.status",
            label: say!("sparse-status"),
            description: say!("sparse-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sparse.set",
            label: say!("sparse-set"),
            description: say!("sparse-set-description"),
        },
        set,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sparse.add",
            label: say!("sparse-add"),
            description: say!("sparse-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sparse.disable",
            label: say!("sparse-disable"),
            description: say!("sparse-disable-description"),
        },
        disable,
    );
}

/// The directories or the patterns the parameters give: one of the two.
fn rules(params: &Params, empty_allowed: bool) -> Result<Rules, CommandError> {
    let directories: Option<Vec<String>> = param_or(params, "directories", None)?;
    let patterns: Option<Vec<String>> = param_or(params, "patterns", None)?;
    let rules = match (directories, patterns) {
        (Some(directories), None) => Rules::Directories(directories),
        (None, Some(patterns)) => Rules::Patterns(patterns),
        (Some(_), Some(_)) => {
            return Err(CommandError::InvalidParam {
                name: "patterns".to_owned(),
                detail: say!("sparse-not-both").to_owned(),
            });
        }
        (None, None) => {
            return Err(CommandError::InvalidParam {
                name: "directories".to_owned(),
                detail: say!("sparse-neither").to_owned(),
            });
        }
    };
    let empty = match &rules {
        Rules::Directories(list) | Rules::Patterns(list) => list.is_empty(),
    };
    if empty && !empty_allowed {
        return Err(CommandError::InvalidParam {
            name: "directories".to_owned(),
            detail: say!("sparse-nothing-to-add").to_owned(),
        });
    }
    Ok(rules)
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(sparse::status(ctx.repo()?)?)?)
}

fn set(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    // No directories at all is the files at the top only.
    let rules = rules(params, true)?;
    let changed = reporting(ctx, |repo, reporter| sparse::set(repo, &rules, reporter))?;
    Ok(serde_json::to_value(changed)?)
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let rules = rules(params, false)?;
    let changed = reporting(ctx, |repo, reporter| sparse::add(repo, &rules, reporter))?;
    Ok(serde_json::to_value(changed)?)
}

fn disable(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let changed = reporting(ctx, sparse::disable)?;
    Ok(serde_json::to_value(changed)?)
}
