//! Rerere commands: whether git records and replays resolutions
//! here, what it has replayed in the conflict at hand, switching it on and
//! off, and forgetting what it recorded for a file.
//!
//! Sources: git-rerere(1), git-config(1) (`rerere.enabled`).

use bitbraid_git::rerere::{self, Scope};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "rerere.status",
            label: say!("rerere-status"),
            description: say!("rerere-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rerere.enable",
            label: say!("rerere-enable"),
            description: say!("rerere-enable-description"),
        },
        enable,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rerere.forget",
            label: say!("rerere-forget"),
            description: say!("rerere-forget-description"),
        },
        forget,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rerere.records",
            label: say!("rerere-records"),
            description: say!("rerere-records-description"),
        },
        records,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rerere.forget_record",
            label: say!("rerere-forget-record"),
            description: say!("rerere-forget-record-description"),
        },
        forget_record,
    );
}

fn records(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", 200)?;
    let repo = ctx.repo()?;
    Ok(json!({
        "enabled": rerere::enabled(repo),
        "records": bitbraid_git::rerere_record::records(repo, limit),
    }))
}

fn forget_record(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id: String = param(params, "id")?;
    let forgotten = bitbraid_git::rerere_record::forget_record(ctx.repo()?, &id)?;
    Ok(json!({ "id": id, "forgotten": forgotten }))
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(rerere::status(ctx.repo()?)?)?)
}

fn enable(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let on: bool = param(params, "on")?;
    let scope: Scope = param_or(params, "scope", Scope::default())?;
    // In this repository's own settings it is journalled; in the user's
    // it is not the repository's to take back.
    let done = crate::settings::journalled(
        ctx,
        |repo| Ok(rerere::enable(repo, on, scope)?),
        |_| {
            if on {
                say!("rerere-entry-on").to_owned()
            } else {
                say!("rerere-entry-off").to_owned()
            }
        },
    )?;
    Ok(serde_json::to_value(done)?)
}

fn forget(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    Ok(serde_json::to_value(rerere::forget(ctx.repo()?, &path)?)?)
}
