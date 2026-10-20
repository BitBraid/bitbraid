//! Submodule commands: list, add, update, sync, remove, forget the
//! clone that is kept of a removed one, and the commits between two
//! commits recorded for one.
//!
//! Sources: git-submodule(1).

use bitbraid_git::submodule;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "submodule.list",
            label: say!("submodule-list"),
            description: say!("submodule-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.add",
            label: say!("submodule-add"),
            description: say!("submodule-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.update",
            label: say!("submodule-update"),
            description: say!("submodule-update-description"),
        },
        update,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.sync",
            label: say!("submodule-sync"),
            description: say!("submodule-sync-description"),
        },
        sync,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.remove",
            label: say!("submodule-remove"),
            description: say!("submodule-remove-description"),
        },
        remove,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.forget",
            label: say!("submodule-forget"),
            description: say!("submodule-forget-description"),
        },
        forget,
    );
    let _ = registry.register(
        CommandSpec {
            id: "submodule.changes",
            label: say!("submodule-changes"),
            description: say!("submodule-changes-description"),
        },
        changes,
    );
}

fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(submodule::list(ctx.repo()?)?)?)
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let url: String = param(params, "url")?;
    let path: String = param(params, "path")?;
    let branch: Option<String> = param_or(params, "branch", None)?;
    let allow_local: bool = param_or(params, "allow_local", false)?;
    let added = reporting(ctx, |repo, reporter| {
        submodule::add(repo, &url, &path, branch.as_deref(), allow_local, reporter)
    })?;
    Ok(serde_json::to_value(added)?)
}

fn update(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param_or(params, "paths", Vec::new())?;
    let init: bool = param_or(params, "init", false)?;
    let recursive: bool = param_or(params, "recursive", false)?;
    let allow_local: bool = param_or(params, "allow_local", false)?;
    let after = reporting(ctx, |repo, reporter| {
        submodule::update(repo, &paths, init, recursive, allow_local, reporter)
    })?;
    Ok(serde_json::to_value(after)?)
}

fn sync(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param_or(params, "paths", Vec::new())?;
    let recursive: bool = param_or(params, "recursive", false)?;
    Ok(serde_json::to_value(submodule::sync(
        ctx.repo()?,
        &paths,
        recursive,
    )?)?)
}

fn remove(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let force: bool = param_or(params, "force", false)?;
    let purge: bool = param_or(params, "purge", false)?;
    Ok(serde_json::to_value(submodule::remove(
        ctx.repo()?,
        &path,
        force,
        purge,
    )?)?)
}

fn forget(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    Ok(serde_json::to_value(submodule::forget(
        ctx.repo()?,
        &name,
    )?)?)
}

fn changes(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let old: String = param_or(params, "old", "HEAD".to_owned())?;
    let new: String = param_or(params, "new", "worktree".to_owned())?;
    Ok(serde_json::to_value(submodule::changes(
        ctx.repo()?,
        &path,
        &old,
        &new,
    )?)?)
}
