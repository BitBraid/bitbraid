//! Index flag commands: which entries are marked intent-to-add,
//! skip-worktree or assume-unchanged, and setting and clearing the marks.
//!
//! Sources: git-update-index(1), git-add(1) (`--intent-to-add`),
//! git-ls-files(1).

use bitbraid_git::index_flags::{self, Flag};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "index.flags",
            label: say!("index-flags"),
            description: say!("index-flags-description"),
        },
        flags,
    );
    let _ = registry.register(
        CommandSpec {
            id: "index.set_flag",
            label: say!("index-set-flag"),
            description: say!("index-set-flag-description"),
        },
        set_flag,
    );
}

fn flags(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(index_flags::list(ctx.repo()?)?)?)
}

fn set_flag(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param(params, "paths")?;
    let flag: Flag = param(params, "flag")?;
    let on: bool = param(params, "on")?;
    index_flags::set(ctx.repo()?, &paths, flag, on)?;
    Ok(json!({ "flag": flag, "on": on, "paths": paths }))
}
