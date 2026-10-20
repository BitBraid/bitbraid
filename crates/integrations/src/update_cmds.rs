//! The updater's commands: `update.status`, which says how this copy was
//! installed and what the updater can do with it, asking nobody;
//! `update.check`, which reads the feed of releases and says whether a
//! newer version is out and what this copy can do about it; and
//! `update.install`, which downloads that version's package, checks it
//! and installs it the system's way ([`crate::update`]).
//!
//! The last two send nothing unless they are run: the window runs them
//! only while the user has updates on (`update.check` at start, once a
//! day and when asked; `update.install` when asked), the command line when
//! it is told to. A build without the update key sends nothing at all and
//! says so.
//!
//! Sources: none beyond those of [`crate::update`].

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};

use crate::{
    cmds::{FREE, invalid, opt},
    update::{self, UpdateError, Updater},
};

const NONE: &[ParamSpec] = &[];
const INSTALL: &[ParamSpec] = &[opt(
    "version",
    ParamKind::Text,
    message!("update-param-version"),
)];

/// Register the updater's commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "update.status",
            label: say!("update-status"),
            description: say!("update-status-description"),
        },
        status,
        FREE,
        NONE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "update.check",
            label: say!("update-check"),
            description: say!("update-check-description"),
        },
        check,
        FREE,
        NONE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "update.install",
            label: say!("update-install"),
            description: say!("update-install-description"),
        },
        install,
        FREE,
        INSTALL,
    );
}

/// The engine's error for the updater's.
fn failed(err: UpdateError) -> CommandError {
    match err {
        UpdateError::Cancelled => CommandError::Git(bitbraid_git::Error::Invalid {
            what: say!("commands-what-operation"),
            detail: say!("commands-cancelled").to_owned(),
        }),
        other => CommandError::Git(bitbraid_git::Error::Refused {
            what: say!("update-what"),
            detail: other.to_string(),
        }),
    }
}

fn status(ctx: &mut CommandContext, _params: &Params) -> Result<Output, CommandError> {
    let updater = Updater::of(ctx);
    Ok(serde_json::to_value(update::status(&updater))?)
}

fn check(ctx: &mut CommandContext, _params: &Params) -> Result<Output, CommandError> {
    let updater = Updater::of(ctx);
    let found = update::check(&updater, &ctx.cancel).map_err(failed)?;
    Ok(serde_json::to_value(found)?)
}

fn install(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let offered = match params.get("version").filter(|value| !value.is_null()) {
        None => None,
        Some(value) => {
            let text = value
                .as_str()
                .ok_or_else(|| invalid("version", say!("update-param-not-text")))?;
            Some(
                update::feed::version(text)
                    .map_err(|_| invalid("version", say!("update-param-not-version")))?,
            )
        }
    };
    let updater = Updater::of(ctx);
    let progress = ctx.progress.clone();
    let stage = match &offered {
        Some(version) => say!("update-downloading-version", version = version.to_string()),
        None => say!("update-downloading").to_owned(),
    };
    let mut report = |percent: u8| {
        if let Some(progress) = &progress {
            progress(bitbraid_git::Progress {
                stage: stage.clone(),
                percent: Some(percent),
            });
        }
    };
    let installed =
        update::install(&updater, offered.as_ref(), &ctx.cancel, &mut report).map_err(failed)?;
    Ok(serde_json::to_value(installed)?)
}
