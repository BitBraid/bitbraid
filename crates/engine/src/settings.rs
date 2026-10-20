//! Changes of settings in the journal: a command that writes to
//! the repository's own configuration is run between two readings of it,
//! and what differs is what undo puts back. The commands themselves stay
//! as they are, and none of them has to say which names it writes.
//!
//! What a command writes to the user's configuration (a scope of `user`)
//! changes nothing in the repository's, so nothing is recorded for it:
//! the journal is the repository's, and taking back from one repository
//! what holds for all of them would be a surprise.
//!
//! Sources: `bitbraid_git::settings`; git-config(1) (`--local`).

use bitbraid_git::{Repository, settings, settings::Setting};

use crate::{
    command::{CommandContext, CommandError},
    journal::Change,
};

/// Run something on the open repository, and say with its answer which
/// of the repository's own settings it changed.
pub(crate) fn watching<T>(
    ctx: &CommandContext,
    run: impl FnOnce(&Repository) -> Result<T, CommandError>,
) -> Result<(T, Vec<Setting>), CommandError> {
    let repo = ctx.repo()?;
    let before = settings::local(repo)?;
    let done = run(repo)?;
    let after = settings::local(repo)?;
    Ok((done, settings::changed(&before, &after)))
}

/// Record a change of settings, when there was one.
pub(crate) fn record(
    ctx: &mut CommandContext,
    label: String,
    settings: Vec<Setting>,
) -> Result<(), CommandError> {
    if settings.is_empty() {
        return Ok(());
    }
    ctx.record(
        label,
        Change::Config {
            settings,
            branch: None,
        },
    )
}

/// Run something that writes settings and record what it changed in the
/// repository's own configuration, under the label made from its answer.
pub(crate) fn journalled<T>(
    ctx: &mut CommandContext,
    run: impl FnOnce(&Repository) -> Result<T, CommandError>,
    label: impl FnOnce(&T) -> String,
) -> Result<T, CommandError> {
    let (done, changed) = watching(ctx, run)?;
    record(ctx, label(&done), changed)?;
    Ok(done)
}
