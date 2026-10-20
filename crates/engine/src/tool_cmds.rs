//! External diff and merge tools: the tool git is configured with, or
//! a command handed in with the request (the interface keeps one in its
//! preferences).
//!
//! `tool.diff` starts the tool and returns; `tool.merge` waits for the
//! tool and says whether the file is resolved, so it belongs on the
//! operations thread, where it can be stopped.
//!
//! Sources: git-difftool(1), git-mergetool(1).

use bitbraid_git::{Oid, known_tools, tools};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "tool.list",
            label: say!("tool-list"),
            description: say!("tool-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tool.diff",
            label: say!("tool-diff"),
            description: say!("tool-diff-description"),
        },
        diff,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tool.merge",
            label: say!("tool-merge"),
            description: say!("tool-merge-description"),
        },
        merge,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tool.known",
            label: say!("tool-known"),
            description: say!("tool-known-description"),
        },
        known,
    );
}

fn known(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let all: bool = param_or(params, "all", false)?;
    let found = known_tools::found();
    Ok(if all {
        json!({ "found": found, "known": known_tools::KNOWN })
    } else {
        json!({ "found": found })
    })
}

fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(json!(tools::configured(ctx.repo()?)))
}

fn diff(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let staged: bool = param_or(params, "staged", false)?;
    let commit: Option<Oid> = param_or(params, "commit", None)?;
    let from: Option<Oid> = param_or(params, "from", None)?;
    let to: Option<Oid> = param_or(params, "to", None)?;
    let command: Option<String> = param_or(params, "command", None)?;
    let what = match (to, commit, staged) {
        // Two commits that were picked.
        (Some(to), None, false) => tools::What::Range(from, to),
        (Some(_), _, _) => {
            return Err(CommandError::InvalidParam {
                name: "to".to_owned(),
                detail: say!("tool-range-alone").to_owned(),
            });
        }
        (None, _, _) if from.is_some() => {
            return Err(CommandError::InvalidParam {
                name: "from".to_owned(),
                detail: say!("tool-from-without-to").to_owned(),
            });
        }
        (None, Some(id), _) => tools::What::Commit(id),
        (None, None, true) => tools::What::Staged,
        (None, None, false) => tools::What::Unstaged,
    };
    let tool = tools::diff(ctx.repo()?, what, &path, command.as_deref())?;
    Ok(json!({ "started": tool }))
}

fn merge(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let command: Option<String> = param_or(params, "command", None)?;
    let cancel = ctx.cancel.clone();
    let resolved = tools::merge(ctx.repo()?, &path, command.as_deref(), &cancel)?;
    Ok(json!({ "resolved": resolved }))
}
