//! Branches and checkout: create, check out, rename, delete (also at a
//! remote), upstream, fast-forward and tracking. Each change is journalled
//! with what it takes to reverse it, and undoing and redoing a journal
//! entry of any kind but a commit, a discard or a restore is here too.
//!
//! Sources: git-branch(1), git-switch(1), git-update-ref(1).

use bitbraid_git::{Dirty, HeadState, Repository, branch};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
};

/// Register the branch commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "branch.create",
            label: say!("branch-create"),
            description: say!("branch-create-description"),
        },
        create,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.checkout",
            label: say!("branch-checkout"),
            description: say!("branch-checkout-description"),
        },
        checkout,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.rename",
            label: say!("branch-rename"),
            description: say!("branch-rename-description"),
        },
        rename,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.delete",
            label: say!("branch-delete"),
            description: say!("branch-delete-description"),
        },
        delete,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.delete_remote",
            label: say!("branch-delete-remote"),
            description: say!("branch-delete-remote-description"),
        },
        delete_remote,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.set_upstream",
            label: say!("branch-set-upstream"),
            description: say!("branch-set-upstream-description"),
        },
        set_upstream,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.fast_forward",
            label: say!("branch-fast-forward"),
            description: say!("branch-fast-forward-description"),
        },
        fast_forward,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.tracking",
            label: say!("branch-tracking"),
            description: say!("branch-tracking-description"),
        },
        tracking,
    );
}

fn refused(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

fn create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let start: Option<String> = param_or(params, "start", None)?;
    let switch: bool = param_or(params, "checkout", false)?;
    let dirty: Dirty = param_or(params, "dirty", Dirty::Refuse)?;
    let repo = ctx.repo()?;
    let target = branch::branch_create(repo, &name, start.as_deref())?;
    let mut switched_from = None;
    let mut stash_kept = false;
    let mut hook = None;
    if switch {
        match branch::checkout(repo, &name, dirty) {
            Ok(outcome) => {
                stash_kept = outcome.stash_kept;
                hook = outcome.hook;
                switched_from = Some(outcome.before);
            }
            Err(err) => {
                // Half done is worse than not done: take the branch back.
                let _ = branch::branch_move(repo, &name, None, Some(target));
                return Err(err.into());
            }
        }
    }
    ctx.record(
        say!("branch-entry-create", name = name.as_str()),
        Change::BranchCreate {
            name: name.clone(),
            target,
            switched_from,
        },
    )?;
    Ok(json!({
        "created": name,
        "target": target.to_hex(),
        "checked_out": switch,
        "stash_kept": stash_kept,
        "hook": hook,
    }))
}

fn checkout(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let target: String = param(params, "target")?;
    let dirty: Dirty = param_or(params, "dirty", Dirty::Refuse)?;
    let outcome = branch::checkout(ctx.repo()?, &target, dirty)?;
    let label = match &outcome.after.branch {
        Some(name) => say!("branch-entry-checkout", name = name.as_str()),
        None => say!(
            "branch-entry-checkout-detached",
            commit = outcome
                .after
                .commit
                .map(|id| id.short(7))
                .unwrap_or_else(|| target.clone())
        ),
    };
    let out = json!({
        "before": outcome.before,
        "after": outcome.after,
        "created": outcome.created,
        "stash_kept": outcome.stash_kept,
        // What a hook had against the checkout after it had happened.
        "hook": outcome.hook,
    });
    if outcome.before != outcome.after {
        ctx.record(
            label,
            Change::Checkout {
                target,
                dirty,
                before: outcome.before,
                after: outcome.after,
                created: outcome.created,
                backups: outcome.backups,
            },
        )?;
    }
    Ok(out)
}

fn rename(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let new_name: String = param(params, "new_name")?;
    branch::branch_rename(ctx.repo()?, &name, &new_name)?;
    ctx.record(
        say!(
            "branch-entry-rename",
            name = name.as_str(),
            new_name = new_name.as_str()
        ),
        Change::BranchRename {
            name: name.clone(),
            new_name: new_name.clone(),
        },
    )?;
    Ok(json!({ "renamed": name, "to": new_name }))
}

fn delete(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let force: bool = param_or(params, "force", false)?;
    let protected: Vec<String> = param_or(params, "protected", Vec::new())?;
    if protected.contains(&name) {
        return Err(refused(
            "name",
            say!("branch-protected", name = format!("{name:?}")),
        ));
    }
    let deleted = branch::branch_delete(ctx.repo()?, &name, force)?;
    ctx.record(
        say!("branch-entry-delete", name = name.as_str()),
        Change::BranchDelete {
            name: deleted.name,
            target: deleted.target,
            upstream: deleted.upstream,
        },
    )?;
    Ok(json!({ "deleted": name, "target": deleted.target.to_hex() }))
}

fn delete_remote(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote: String = param(params, "remote")?;
    let branch: String = param(params, "branch")?;
    let protected: Vec<String> = param_or(params, "protected", Vec::new())?;
    if protected.contains(&branch) {
        return Err(refused(
            "branch",
            say!("branch-protected", name = format!("{branch:?}")),
        ));
    }
    let deleted = crate::remote_cmds::reporting(ctx, |repo, reporter| {
        bitbraid_git::remote::delete_remote_branch(repo, &remote, &branch, reporter)
    })?;
    ctx.record(
        say!(
            "branch-entry-delete-remote",
            remote = remote.as_str(),
            branch = branch.as_str()
        ),
        Change::RemoteBranchDelete {
            remote: deleted.remote,
            branch: deleted.branch,
            target: deleted.target,
        },
    )?;
    Ok(json!({
        "deleted": format!("{remote}/{branch}"),
        "target": deleted.target.to_hex(),
    }))
}

fn set_upstream(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let upstream: Option<String> = param_or(params, "upstream", None)?;
    let before = branch::branch_set_upstream(ctx.repo()?, &name, upstream.as_deref())?;
    if before != upstream {
        ctx.record(
            match &upstream {
                Some(upstream) => say!(
                    "branch-entry-set-upstream",
                    name = name.as_str(),
                    upstream = upstream.as_str()
                ),
                None => say!("branch-entry-clear-upstream", name = name.as_str()),
            },
            Change::Upstream {
                name: name.clone(),
                before: before.clone(),
                after: upstream.clone(),
            },
        )?;
    }
    Ok(json!({ "branch": name, "upstream": upstream, "before": before }))
}

fn tracking(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let branches = branch::tracking(ctx.repo()?)?;
    Ok(json!({ "branches": branches }))
}

fn fast_forward(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let to: Option<String> = param_or(params, "to", None)?;
    let (before, after) = branch::fast_forward_to(ctx.repo()?, &name, to.as_deref())?;
    if before != after {
        ctx.record(
            say!("branch-entry-fast-forward", name = name.as_str()),
            Change::BranchMove {
                name: name.clone(),
                before,
                after,
            },
        )?;
    }
    Ok(json!({ "branch": name, "before": before, "after": after }))
}

/// HEAD has to be where the entry left it, or undoing would move the user
/// away from somewhere they went on their own.
fn expect_head(repo: &Repository, expected: &HeadState) -> Result<(), CommandError> {
    let now = branch::head_state(repo)?;
    if now == *expected {
        Ok(())
    } else {
        Err(refused("undo", say!("branch-head-moved")))
    }
}

/// Reverse a change other than a commit, a discard or a restore. Whether
/// local changes a checkout carried back in a stash did not fit there and
/// stay in the stash.
pub(crate) fn undo_change(repo: &Repository, change: &Change) -> Result<bool, CommandError> {
    let mut stash_kept = false;
    match change {
        Change::Checkout {
            before,
            after,
            created,
            backups,
            dirty,
            ..
        } => {
            expect_head(repo, after)?;
            // The local changes go back the way they came.
            stash_kept = branch::switch_back(repo, before, *dirty)?;
            for backup in backups {
                bitbraid_git::restore_backup(repo, backup)?;
            }
            if let (Some(name), Some(commit)) = (created, after.commit) {
                // Only if nothing was committed on it meanwhile.
                let _ = branch::branch_delete_if_at(repo, name, commit);
            }
        }
        Change::BranchCreate {
            name,
            target,
            switched_from,
        } => {
            if let Some(from) = switched_from {
                expect_head(
                    repo,
                    &HeadState {
                        branch: Some(name.clone()),
                        commit: Some(*target),
                    },
                )?;
                branch::switch_to(repo, from)?;
            }
            branch::branch_delete_if_at(repo, name, *target)?;
        }
        Change::BranchDelete {
            name,
            target,
            upstream,
        } => {
            branch::branch_move(repo, name, Some(*target), None)?;
            if let Some(upstream) = upstream {
                // The remote branch may be gone by now; the branch is back
                // either way.
                let _ = branch::branch_set_upstream(repo, name, Some(upstream));
            }
        }
        Change::BranchRename { name, new_name } => branch::branch_rename(repo, new_name, name)?,
        Change::Upstream { name, before, .. } => {
            branch::branch_set_upstream(repo, name, before.as_deref())?;
        }
        Change::HeadMoved {
            before,
            after,
            mode,
            backups,
        } => {
            bitbraid_git::rewrite::move_back(repo, after, before, *mode)?;
            for backup in backups {
                bitbraid_git::restore_backup(repo, backup)?;
            }
        }
        Change::StashPush { .. }
        | Change::StashDrop { .. }
        | Change::StashApply { .. }
        | Change::StashReword { .. } => {
            crate::stash_cmds::undo_change(repo, change)?;
        }
        Change::TagAnnotate {
            name,
            before,
            after,
        } => {
            bitbraid_git::tag::tag_swap(repo, name, *after, *before)?;
        }
        Change::TagCreate { name, value } => {
            bitbraid_git::tag::tag_delete(repo, name, Some(*value))?;
        }
        Change::TagDelete { name, value } => {
            bitbraid_git::tag::tag_restore(repo, name, *value)?;
        }
        Change::BranchMove {
            name,
            before,
            after,
        } => {
            branch::branch_move(repo, name, Some(*before), Some(*after))?;
        }
        Change::FileWrite { before, after } => {
            bitbraid_git::stage::swap_backup(repo, after, before)?;
        }
        Change::Signers { edit } => bitbraid_git::signers::take_back(edit)?,
        Change::PatchApply {
            patch,
            index,
            reverse,
        } => {
            // The same patch the other way round, when it still applies.
            bitbraid_git::patch::apply_recorded(
                repo,
                *patch,
                *index,
                !*reverse,
                say!("branch-undo"),
            )?;
        }
        Change::FilesMerged { files } => {
            bitbraid_git::files_change::take_back(repo, files, say!("branch-since-done"))?;
        }
        Change::Config {
            settings,
            branch: made,
        } => {
            // Looked at before anything is written: a branch that was
            // made with the settings goes with them only where it was put.
            if let Some((name, target)) = made
                && branch::branch_tip(repo, name).is_some_and(|tip| tip != *target)
            {
                return Err(refused(
                    "undo",
                    say!("branch-made-moved", name = format!("{name:?}")),
                ));
            }
            bitbraid_git::settings::put(repo, settings, true)?;
            if let Some((name, target)) = made {
                // Gone already is as good as taken back.
                let _ = branch::branch_delete_if_at(repo, name, *target);
            }
        }
        Change::RemoteRemove {
            name,
            settings,
            references,
        } => {
            bitbraid_git::remote::remote_restore(
                repo,
                &bitbraid_git::remote::RemovedRemote {
                    name: name.clone(),
                    settings: settings.clone(),
                    references: references.clone(),
                },
            )?;
        }
        Change::RemoteBranchDelete {
            remote,
            branch,
            target,
        } => {
            // Pushed back, if the remote has no branch of that name by now.
            bitbraid_git::remote::restore_remote_branch(
                repo,
                &bitbraid_git::remote::DeletedRemoteBranch {
                    remote: remote.clone(),
                    branch: branch.clone(),
                    target: *target,
                },
                &mut quiet(&mut |_| {}),
            )?;
        }
        Change::Commit { .. } | Change::Discard { .. } | Change::Restore { .. } => {}
    }
    Ok(stash_kept)
}

/// A reporter for a push that undo or redo makes: nobody watches it, and
/// it is not stopped half-way.
fn quiet<'a>(on_progress: &'a mut dyn FnMut(bitbraid_git::Progress)) -> bitbraid_git::Reporter<'a> {
    bitbraid_git::Reporter {
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        on_progress,
    }
}

/// Do such a change again. Returns the change to remember instead when
/// doing it again produced new backups.
pub(crate) fn redo_change(
    repo: &Repository,
    change: &Change,
) -> Result<Option<Change>, CommandError> {
    match change {
        Change::Checkout {
            target,
            dirty,
            before,
            ..
        } => {
            expect_head(repo, before)?;
            let outcome = branch::checkout(repo, target, *dirty)?;
            return Ok(Some(Change::Checkout {
                target: target.clone(),
                dirty: *dirty,
                before: outcome.before,
                after: outcome.after,
                created: outcome.created,
                backups: outcome.backups,
            }));
        }
        Change::BranchCreate {
            name,
            target,
            switched_from,
        } => {
            branch::branch_move(repo, name, Some(*target), None)?;
            if switched_from.is_some() {
                branch::switch_to(
                    repo,
                    &HeadState {
                        branch: Some(name.clone()),
                        commit: Some(*target),
                    },
                )?;
            }
        }
        Change::BranchDelete { name, target, .. } => {
            // Through `git branch` so that its configuration goes as well,
            // but only if it is still the branch that was deleted before.
            if branch::branch_tip(repo, name) != Some(*target) {
                return Err(refused(
                    "redo",
                    say!("branch-moved-since", name = format!("{name:?}")),
                ));
            }
            branch::branch_delete(repo, name, true)?;
        }
        Change::BranchRename { name, new_name } => branch::branch_rename(repo, name, new_name)?,
        Change::Upstream { name, after, .. } => {
            branch::branch_set_upstream(repo, name, after.as_deref())?;
        }
        Change::HeadMoved {
            before,
            after,
            mode,
            ..
        } => {
            // A hard reset done again overwrites what undo put back, and
            // backs it up again; the others move HEAD and keep local work.
            let backups = match (mode, after.commit) {
                (bitbraid_git::ResetMode::Hard, Some(commit)) => {
                    expect_head(repo, before)?;
                    bitbraid_git::rewrite::reset(repo, &commit.to_hex(), *mode)?.backups
                }
                _ => {
                    bitbraid_git::rewrite::move_back(repo, before, after, *mode)?;
                    Vec::new()
                }
            };
            return Ok(Some(Change::HeadMoved {
                before: before.clone(),
                after: after.clone(),
                mode: *mode,
                backups,
            }));
        }
        Change::StashPush { .. }
        | Change::StashDrop { .. }
        | Change::StashApply { .. }
        | Change::StashReword { .. } => {
            return crate::stash_cmds::redo_change(repo, change);
        }
        Change::TagAnnotate {
            name,
            before,
            after,
        } => {
            bitbraid_git::tag::tag_swap(repo, name, *before, *after)?;
        }
        Change::TagCreate { name, value } => {
            bitbraid_git::tag::tag_restore(repo, name, *value)?;
        }
        Change::TagDelete { name, value } => {
            bitbraid_git::tag::tag_delete(repo, name, Some(*value))?;
        }
        Change::BranchMove {
            name,
            before,
            after,
        } => {
            branch::branch_move(repo, name, Some(*after), Some(*before))?;
        }
        Change::FileWrite { before, after } => {
            bitbraid_git::stage::swap_backup(repo, before, after)?;
        }
        Change::Signers { edit } => bitbraid_git::signers::make_again(edit)?,
        Change::PatchApply {
            patch,
            index,
            reverse,
        } => {
            bitbraid_git::patch::apply_recorded(
                repo,
                *patch,
                *index,
                *reverse,
                say!("branch-redo"),
            )?;
        }
        Change::FilesMerged { files } => {
            bitbraid_git::files_change::do_again(repo, files, say!("branch-since-undone"))?;
        }
        Change::Config {
            settings,
            branch: made,
        } => {
            if let Some((name, _)) = made
                && branch::branch_tip(repo, name).is_some()
            {
                return Err(refused(
                    "redo",
                    say!("branch-exists-again", name = format!("{name:?}")),
                ));
            }
            bitbraid_git::settings::put(repo, settings, false)?;
            if let Some((name, target)) = made {
                branch::branch_move(repo, name, Some(*target), None)?;
            }
        }
        Change::RemoteRemove { name, .. } => {
            // Removed again, it takes what is there now: that is what a
            // later undo has to put back.
            let removed = bitbraid_git::remote::remote_remove(repo, name)?;
            return Ok(Some(Change::RemoteRemove {
                name: removed.name,
                settings: removed.settings,
                references: removed.references,
            }));
        }
        Change::RemoteBranchDelete {
            remote,
            branch,
            target,
        } => {
            // Deleted again only if it is the branch undo pushed back.
            let tracking = format!("refs/remotes/{remote}/{branch}");
            if bitbraid_git::branch::commit_of(repo, &tracking).ok() != Some(*target) {
                return Err(refused(
                    "redo",
                    say!(
                        "branch-remote-moved-since",
                        remote = remote.as_str(),
                        branch = branch.as_str()
                    ),
                ));
            }
            bitbraid_git::remote::delete_remote_branch(
                repo,
                remote,
                branch,
                &mut quiet(&mut |_| {}),
            )?;
        }
        Change::Commit { .. } | Change::Discard { .. } | Change::Restore { .. } => {}
    }
    Ok(None)
}
