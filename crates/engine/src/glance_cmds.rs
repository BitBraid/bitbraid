//! A look at repositories from outside them: for the page of
//! repositories, the branch each is on, how far it is ahead and behind
//! what it follows, and its uncommitted files by kind. No repository needs
//! to be open, and none that is looked at is changed.
//!
//! Sources: git-status(1) (the short format with `--branch`).

use std::path::PathBuf;

use bitbraid_git::glance;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param},
};

/// The longest path taken, in bytes.
const MAX_PATH: usize = 4096;

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "repo.glance",
            label: say!("glance-repo-glance"),
            description: say!("glance-repo-glance-description"),
        },
        look,
    );
}

fn look(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param(params, "paths")?;
    if paths.len() > glance::MAX_GLANCED {
        return Err(CommandError::InvalidParam {
            name: "paths".to_owned(),
            detail: say!("glance-too-many", max = glance::MAX_GLANCED),
        });
    }
    let mut folders = Vec::new();
    for text in paths {
        let path = PathBuf::from(&text);
        if text.len() > MAX_PATH
            || text.chars().any(char::is_control)
            || text.starts_with('-')
            || !path.is_absolute()
        {
            return Err(CommandError::InvalidParam {
                name: "paths".to_owned(),
                detail: say!("glance-not-a-full-path").to_owned(),
            });
        }
        folders.push(path);
    }
    let cancel = ctx.cancel.clone();
    Ok(serde_json::to_value(glance::glance_all(&folders, &cancel))?)
}
