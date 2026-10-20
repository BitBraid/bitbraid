//! The git executable itself: which one is run, what version it is,
//! whether that is new enough, and running another one instead.
//!
//! The choice is one for the whole process: every repository that is open
//! runs the same git. It is kept by whoever starts the engine (the desktop
//! application keeps it in its preferences) and handed in with `git.use`.
//!
//! Sources: git-version(1); `bitbraid_git::version` for what each
//! version is needed for.

use std::ffi::OsString;

use bitbraid_git::{exe, version};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "git.info",
            label: say!("git-info"),
            description: say!("git-info-description"),
        },
        info,
    );
    let _ = registry.register(
        CommandSpec {
            id: "git.use",
            label: say!("git-use"),
            description: say!("git-use-description"),
        },
        use_program,
    );
}

fn info(_ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let program: Option<String> = param_or(params, "program", None)?;
    let program = program.map(OsString::from);
    Ok(serde_json::to_value(version::probe(program.as_deref())?)?)
}

fn use_program(_ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let program: Option<String> = param_or(params, "program", None)?;
    let verify: bool = param_or(params, "verify", true)?;
    let program = program
        .map(|program| program.trim().to_owned())
        .filter(|program| !program.is_empty())
        .map(OsString::from);
    let Some(program) = program else {
        // Back to the default, which is asked as it is: there may be none.
        exe::set_git_binary(None);
        let info = version::probe(None).ok();
        return Ok(json!({ "program": null, "info": info }));
    };
    let info = if verify {
        Some(version::probe(Some(&program))?)
    } else {
        None
    };
    exe::set_git_binary(Some(program.clone()));
    Ok(json!({ "program": program.to_string_lossy(), "info": info }))
}
