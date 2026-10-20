//! The activity log and the hooks: the git processes that were run
//! for the open repository, with what they wrote, and the hooks git will
//! run in it.
//!
//! The record is kept by `bitbraid_git::activity` for the whole process;
//! these commands show and forget the part that belongs to the repository
//! of the context, so that one tab clearing its log leaves the others
//! theirs. What a hook prints is in the entry of the command that ran it.
//!
//! Between runs it is kept in the application's configuration directory,
//! in `activity/`, a file per repository: never in the repository, which
//! is not ours to write a log into and may be read-only. The registry
//! tells the record for which repository and into which directory a
//! command's git processes go ([`during`]), so that every surface that
//! sets a configuration directory keeps a log, and one that sets none (a
//! test) keeps none.
//!
//! Sources: githooks(5).

use std::path::PathBuf;

use bitbraid_git::{activity, hooks};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param_or},
};

/// How many entries a list has when it is not said.
const DEFAULT_LIMIT: usize = 200;

/// Longest text looked for.
const MAX_TEXT: usize = 1000;

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "activity.list",
            label: say!("activity-list"),
            description: say!("activity-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "activity.clear",
            label: say!("activity-clear"),
            description: say!("activity-clear-description"),
        },
        clear,
    );
    let _ = registry.register(
        CommandSpec {
            id: "hook.list",
            label: say!("activity-hook-list"),
            description: say!("activity-hook-list-description"),
        },
        hook_list,
    );
}

/// Where the log is kept between runs, when the application has said
/// where its own files go.
fn kept_in(ctx: &CommandContext) -> Option<PathBuf> {
    ctx.config_dir.as_ref().map(|dir| dir.join("activity"))
}

/// Name the command a thread runs, the repository it runs it for and
/// where its git processes are kept, for as long as what is returned
/// lives. [`crate::Registry::run`] holds it around every handler.
pub(crate) fn during(command: &'static str, ctx: &CommandContext) -> activity::During {
    activity::during(command, ctx.repo.as_ref(), kept_in(ctx).as_deref())
}

fn list(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", DEFAULT_LIMIT)?;
    let questions: bool = param_or(params, "questions", false)?;
    let text: Option<String> = param_or(params, "text", None)?;
    if text.as_ref().is_some_and(|text| text.len() > MAX_TEXT) {
        return Err(CommandError::InvalidParam {
            name: "text".to_owned(),
            detail: say!("activity-text-too-long", max = MAX_TEXT),
        });
    }
    let needle = text
        .map(|text| text.to_lowercase())
        .filter(|text| !text.is_empty());
    let entries = activity::history(ctx.repo()?, kept_in(ctx).as_deref());
    let matching: Vec<&activity::Entry> = entries
        .iter()
        .rev()
        .map(AsRef::as_ref)
        .filter(|entry| questions || !entry.question)
        .filter(|entry| {
            needle.as_deref().is_none_or(|needle| {
                [&entry.command, &entry.stdout, &entry.stderr]
                    .iter()
                    .any(|text| text.to_lowercase().contains(needle))
            })
        })
        .collect();
    let matched = matching.len();
    let shown: Vec<&activity::Entry> = matching
        .into_iter()
        .take(limit.min(activity::CAPACITY))
        .collect();
    Ok(json!({
        "entries": serde_json::to_value(shown)?,
        "matched": matched,
    }))
}

fn clear(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let cleared = activity::forget(ctx.repo()?, kept_in(ctx).as_deref());
    Ok(json!({ "cleared": cleared }))
}

fn hook_list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(hooks::hooks(ctx.repo()?)?)?)
}
