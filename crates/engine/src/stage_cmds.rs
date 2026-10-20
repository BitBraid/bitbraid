//! Staging and discarding. Discards are journalled. Ignoring is in
//! `ignore_cmds.rs`.
//!
//! Sources: none (original design).

use bitbraid_git::HunkSelection;
use serde_json::json;

use crate::{
    command::{
        CommandContext, CommandError, CommandSpec, Output, Params, Registry, param, param_or,
    },
    journal::Change,
};

/// Register the staging commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "stage.paths",
            label: say!("stage-paths"),
            description: say!("stage-paths-description"),
        },
        stage_paths,
    );
    let _ = registry.register(
        CommandSpec {
            id: "unstage.paths",
            label: say!("stage-unstage-paths"),
            description: say!("stage-unstage-paths-description"),
        },
        unstage_paths,
    );
    let _ = registry.register(
        CommandSpec {
            id: "stage.hunks",
            label: say!("stage-hunks"),
            description: say!("stage-hunks-description"),
        },
        stage_hunks,
    );
    let _ = registry.register(
        CommandSpec {
            id: "unstage.hunks",
            label: say!("stage-unstage-hunks"),
            description: say!("stage-unstage-hunks-description"),
        },
        unstage_hunks,
    );
    let _ = registry.register(
        CommandSpec {
            id: "discard.paths",
            label: say!("stage-discard-paths"),
            description: say!("stage-discard-paths-description"),
        },
        discard_paths,
    );
    let _ = registry.register(
        CommandSpec {
            id: "discard.hunks",
            label: say!("stage-discard-hunks"),
            description: say!("stage-discard-hunks-description"),
        },
        discard_hunks,
    );
}

fn paths(params: &Params) -> Result<Vec<String>, CommandError> {
    let paths: Vec<String> = param(params, "paths")?;
    if paths.is_empty() {
        return Err(CommandError::InvalidParam {
            name: "paths".into(),
            detail: say!("stage-paths-empty").into(),
        });
    }
    Ok(paths)
}

fn selection(params: &Params) -> Result<Vec<HunkSelection>, CommandError> {
    param(params, "selection")
}

fn stage_paths(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths = paths(params)?;
    bitbraid_git::stage_paths(ctx.repo()?, &paths)?;
    Ok(json!({ "staged": paths }))
}

fn unstage_paths(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths = paths(params)?;
    bitbraid_git::unstage_paths(ctx.repo()?, &paths)?;
    Ok(json!({ "unstaged": paths }))
}

fn stage_hunks(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let context: u32 = param_or(params, "context", 3)?;
    bitbraid_git::stage_hunks(ctx.repo()?, &path, &selection(params)?, context)?;
    Ok(json!({ "staged": path }))
}

fn unstage_hunks(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let context: u32 = param_or(params, "context", 3)?;
    bitbraid_git::unstage_hunks(ctx.repo()?, &path, &selection(params)?, context)?;
    Ok(json!({ "unstaged": path }))
}

fn discard_paths(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths = paths(params)?;
    let backups = bitbraid_git::discard_paths(ctx.repo()?, &paths)?;
    let label = say!(
        "stage-entry-discard",
        count = paths.len(),
        path = paths.first().map(String::as_str).unwrap_or_default()
    );
    ctx.record(label, Change::Discard { backups })?;
    Ok(json!({ "discarded": paths }))
}

fn discard_hunks(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let context: u32 = param_or(params, "context", 3)?;
    let before = bitbraid_git::discard_hunks(ctx.repo()?, &path, &selection(params)?, context)?;
    // Journalled as the file before and after, like an edit: redo puts
    // back the file without those lines. As a discard of the path it
    // would discard every change the file has.
    let after = bitbraid_git::stage::backup_paths(ctx.repo()?, std::slice::from_ref(&path))?
        .into_iter()
        .next();
    let change = match after {
        Some(after) => Change::FileWrite { before, after },
        None => Change::Discard {
            backups: vec![before],
        },
    };
    ctx.record(
        say!("stage-entry-discard-lines", path = path.as_str()),
        change,
    )?;
    Ok(json!({ "discarded": path }))
}
