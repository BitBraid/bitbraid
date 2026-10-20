//! Windows specifics: paths too long for Windows and git's
//! long paths, and repositories inside a WSL distribution, for which the
//! distribution's git is run. Line endings are told by `repo.diff`.
//!
//! Sources: Git for Windows' documentation of `core.longpaths`;
//! git-config(1); Microsoft Learn, "Maximum Path Length Limitation",
//! "Working across Windows and Linux file systems" and "Basic commands
//! for WSL".

use bitbraid_core::wsl::{LAUNCHER, WslPath};
use bitbraid_git::{long_paths, rerere::Scope};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "repo.long_paths",
            label: say!("windows-long-paths"),
            description: say!("windows-long-paths-description"),
        },
        report,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.long_paths_enable",
            label: say!("windows-long-paths-enable"),
            description: say!("windows-long-paths-enable-description"),
        },
        enable,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.wsl",
            label: say!("windows-wsl"),
            description: say!("windows-wsl-description"),
        },
        wsl,
    );
}

fn report(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let found = long_paths::report(ctx.repo()?)?;
    let blocks = found.blocks_git();
    let mut out = serde_json::to_value(found)?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("blocks_git".to_owned(), json!(blocks));
    }
    Ok(out)
}

fn enable(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let scope: Scope = param_or(params, "scope", Scope::default())?;
    // In this repository's own settings it is journalled; in the user's it
    // is not the repository's to take back.
    let found = crate::settings::journalled(
        ctx,
        |repo| Ok(long_paths::enable(repo, scope)?),
        |_| say!("windows-long-paths-enable").to_owned(),
    )?;
    Ok(serde_json::to_value(found)?)
}

fn wsl(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let place = repo.workdir().unwrap_or_else(|| repo.git_dir());
    // Only Windows reaches a distribution's share, and runs its git.
    let inside = WslPath::of(place).filter(|_| cfg!(windows));
    Ok(match inside {
        Some(inside) => {
            let mut git = vec![LAUNCHER.to_owned()];
            git.extend(inside.git_invocation());
            json!({
                "inside": true,
                "distribution": inside.distribution,
                "linux_path": inside.linux,
                "git": git,
            })
        }
        None => json!({ "inside": false }),
    })
}
