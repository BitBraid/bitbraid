//! Commit and amend, the message template and recent people, undo and
//! redo.
//!
//! Sources: none (original design).

use bitbraid_git::CommitRequest;
use serde_json::json;

use crate::{
    command::{
        CommandContext, CommandError, CommandSpec, Output, Params, Registry, param, param_or,
    },
    journal::Change,
};

/// Register commit and undo commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "commit.create",
            label: say!("commit-create"),
            description: say!("commit-create-description"),
        },
        create,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.amend",
            label: say!("commit-amend"),
            description: say!("commit-amend-description"),
        },
        amend,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.template",
            label: say!("commit-template"),
            description: say!("commit-template-description"),
        },
        template,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.people",
            label: say!("commit-people"),
            description: say!("commit-people-description"),
        },
        people,
    );
    let _ = registry.register(
        CommandSpec {
            id: "undo.last",
            label: say!("commit-undo-last"),
            description: say!("commit-undo-last-description"),
        },
        undo,
    );
    let _ = registry.register(
        CommandSpec {
            id: "redo.last",
            label: say!("commit-redo-last"),
            description: say!("commit-redo-last-description"),
        },
        redo,
    );
    let _ = registry.register(
        CommandSpec {
            id: "undo.peek",
            label: say!("commit-undo-peek"),
            description: say!("commit-undo-peek-description"),
        },
        peek,
    );
}

fn request(params: &Params, amend: bool) -> Result<CommitRequest, CommandError> {
    Ok(CommitRequest {
        message: param(params, "message")?,
        amend,
        keep_author: param_or(params, "keep_author", true)?,
        no_verify: param_or(params, "no_verify", false)?,
        sign: param_or(params, "sign", None)?,
        allow_empty: param_or(params, "allow_empty", false)?,
    })
}

/// With `stage_all`, everything that is not ignored is staged first, as
/// `git add --all` does.
fn stage_first(ctx: &CommandContext, params: &Params) -> Result<(), CommandError> {
    if param_or(params, "stage_all", false)? {
        bitbraid_git::stage_all(ctx.repo()?)?;
    }
    Ok(())
}

fn create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let request = request(params, false)?;
    stage_first(ctx, params)?;
    let outcome = bitbraid_git::commit(ctx.repo()?, &request)?;
    let summary = request
        .message
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    ctx.record(
        say!(
            "commit-entry-create",
            commit = outcome.id.short(7),
            summary = summary.as_str()
        ),
        Change::Commit {
            branch: outcome.branch.clone(),
            previous: outcome.previous,
            id: outcome.id,
            amend: false,
        },
    )?;
    Ok(serde_json::to_value(outcome)?)
}

fn amend(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let request = request(params, true)?;
    stage_first(ctx, params)?;
    let outcome = bitbraid_git::commit(ctx.repo()?, &request)?;
    ctx.record(
        say!("commit-entry-amend", commit = outcome.id.short(7)),
        Change::Commit {
            branch: outcome.branch.clone(),
            previous: outcome.previous,
            id: outcome.id,
            amend: true,
        },
    )?;
    Ok(serde_json::to_value(outcome)?)
}

fn template(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(json!({ "template": bitbraid_git::template(ctx.repo()?) }))
}

fn people(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", 200)?;
    Ok(serde_json::to_value(bitbraid_git::recent_people(
        ctx.repo()?,
        limit,
    )?)?)
}

pub(crate) fn undo(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let CommandContext { repo, journal, .. } = ctx;
    let repo = repo.as_ref().ok_or(CommandError::NoRepository)?;
    let journal = journal.as_mut().ok_or(CommandError::NoRepository)?;
    let Some(entry) = journal.peek_undo().cloned() else {
        return Ok(json!({ "undone": null }));
    };
    let mut stash_kept = false;
    match &entry.change {
        Change::Commit {
            branch,
            previous,
            id,
            ..
        } => {
            let Some(previous) = previous else {
                return Err(CommandError::InvalidParam {
                    name: "undo".into(),
                    detail: say!("commit-unborn-undo").into(),
                });
            };
            bitbraid_git::move_head(repo, branch.as_deref(), *previous, Some(*id))?;
        }
        Change::Discard { backups } => {
            for backup in backups {
                bitbraid_git::restore_backup(repo, backup)?;
            }
        }
        Change::Restore { backup, .. } => bitbraid_git::restore_backup(repo, backup)?,
        other => stash_kept = crate::branch_cmds::undo_change(repo, other)?,
    }
    journal.pop_undo();
    journal.save(repo);
    // Undone, and local changes carried back in a stash stayed there.
    if stash_kept {
        return Ok(json!({ "undone": entry.label, "stash_kept": true }));
    }
    Ok(json!({ "undone": entry.label }))
}

pub(crate) fn redo(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let CommandContext { repo, journal, .. } = ctx;
    let repo = repo.as_ref().ok_or(CommandError::NoRepository)?;
    let journal = journal.as_mut().ok_or(CommandError::NoRepository)?;
    let Some(entry) = journal.peek_redo().cloned() else {
        return Ok(json!({ "redone": null }));
    };
    match &entry.change {
        Change::Commit {
            branch,
            previous,
            id,
            ..
        } => {
            bitbraid_git::move_head(repo, branch.as_deref(), *id, *previous)?;
        }
        Change::Discard { backups } => {
            let paths: Vec<String> = backups.iter().map(|b| b.path.clone()).collect();
            // Re-discarding takes fresh backups, which replace the old ones.
            let fresh = bitbraid_git::discard_paths(repo, &paths)?;
            if let Some(last) = journal.undone.last_mut() {
                last.change = Change::Discard { backups: fresh };
            }
        }
        Change::Restore { backup, rev } => {
            let fresh = bitbraid_git::restore_path_at(repo, &backup.path, *rev)?;
            if let Some(last) = journal.undone.last_mut() {
                last.change = Change::Restore {
                    backup: fresh,
                    rev: *rev,
                };
            }
        }
        other => {
            if let Some(fresh) = crate::branch_cmds::redo_change(repo, other)?
                && let Some(last) = journal.undone.last_mut()
            {
                last.change = fresh;
            }
        }
    }
    journal.pop_redo();
    journal.save(repo);
    Ok(json!({ "redone": entry.label }))
}

fn peek(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let journal = ctx.journal.as_ref().ok_or(CommandError::NoRepository)?;
    Ok(json!({
        "undo": journal.peek_undo().map(|e| e.label.clone()),
        "redo": journal.peek_redo().map(|e| e.label.clone()),
        // Which entries those are: a caller that saw the id change knows
        // that what it ran was journalled.
        "undo_id": journal.peek_undo().map(|e| e.id),
        "redo_id": journal.peek_redo().map(|e| e.id),
        // Whether undoing or redoing would talk to a remote: it then
        // takes as long as a push, and is run where it can.
        "undo_remote": journal.peek_undo().is_some_and(|e| e.change.talks_to_remote()),
        "redo_remote": journal.peek_redo().is_some_and(|e| e.change.talks_to_remote()),
    }))
}
