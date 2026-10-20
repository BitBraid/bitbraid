//! Commands that move the current branch: merge, rebase, reset,
//! cherry-pick, revert. Each is journalled as "HEAD went from here to
//! there", which is what undo and redo need. One that stops on a conflict
//! is not journalled yet: a note is left for when it is finished.
//!
//! `branch.predict` changes nothing: it says which files merging `target`
//! into the current commit, or into `ours`, would leave in conflict, to
//! warn before one is begun.
//!
//! Merge and rebase take `no_verify`, as git does: the hooks
//! that can refuse them are left out, and the answer names the ones that
//! were there to be left out (`skipped_hooks`).
//!
//! Sources: git-merge(1), git-rebase(1) (`--no-verify` of each),
//! git-reset(1), git-cherry-pick(1), git-revert(1), git-merge-tree(1),
//! githooks(5).

use bitbraid_git::{MergeMode, Moved, RebaseOptions, ResetMode, hooks::Unverified, rewrite};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "branch.merge",
            label: say!("rewrite-merge"),
            description: say!("rewrite-merge-description"),
        },
        merge,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.rebase",
            label: say!("rewrite-rebase"),
            description: say!("rewrite-rebase-description"),
        },
        rebase,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.predict",
            label: say!("rewrite-predict"),
            description: say!("rewrite-predict-description"),
        },
        predict,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.reset",
            label: say!("rewrite-reset"),
            description: say!("rewrite-reset-description"),
        },
        reset,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.cherry_pick",
            label: say!("rewrite-cherry-pick"),
            description: say!("rewrite-cherry-pick-description"),
        },
        cherry_pick,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.revert",
            label: say!("rewrite-revert"),
            description: say!("rewrite-revert-description"),
        },
        revert,
    );
}

/// With `no_verify` asked for: the hooks `command` leaves out that are
/// there, read before the command runs, to be said afterwards. None when
/// it was not asked for.
pub(crate) fn hooks_left_out(
    ctx: &CommandContext,
    params: &Params,
    command: Unverified,
) -> Result<Option<Vec<String>>, CommandError> {
    let no_verify: bool = param_or(params, "no_verify", false)?;
    if !no_verify {
        return Ok(None);
    }
    Ok(Some(bitbraid_git::hooks::left_out(ctx.repo()?, command)))
}

/// Put the hooks that were left out into a command's answer.
pub(crate) fn with_skipped(mut out: Output, skipped: Option<Vec<String>>) -> Output {
    if let (Some(skipped), Some(fields)) = (skipped, out.as_object_mut()) {
        fields.insert("skipped_hooks".to_owned(), json!(skipped));
    }
    out
}

/// Journal a move of HEAD and describe it. Where HEAD stayed (a squash, a
/// pick without commit) what was merged into the files and the index is
/// journalled instead, when it was recorded; nothing for an already merged
/// branch, and nothing yet when the operation stopped half-way.
pub(crate) fn finish(
    ctx: &mut CommandContext,
    label: String,
    moved: Moved,
    mode: ResetMode,
) -> Result<Output, CommandError> {
    if let Some(message) = &moved.stopped {
        let repo = ctx.repo()?;
        crate::pending::save(
            repo,
            &crate::pending::Pending {
                label,
                before: moved.before.clone(),
            },
        );
        let conflicts: Vec<String> = bitbraid_git::sequence::conflicts(repo)?
            .into_iter()
            .map(|conflict| conflict.path)
            .collect();
        return Ok(json!({
            "before": moved.before,
            "after": moved.after,
            "stopped": message,
            "conflicts": conflicts,
        }));
    }
    let out = json!({ "before": moved.before, "after": moved.after });
    if moved.before != moved.after {
        ctx.record(
            label,
            Change::HeadMoved {
                before: moved.before,
                after: moved.after,
                mode,
                backups: moved.backups,
            },
        )?;
    } else if let Some(files) = moved.files {
        // HEAD stayed, and what was merged is taken back by undo.
        ctx.record(label, Change::FilesMerged { files })?;
    }
    Ok(out)
}

fn merge(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let target: String = param(params, "target")?;
    let mode: MergeMode = param_or(params, "mode", MergeMode::default())?;
    let skipped = hooks_left_out(ctx, params, Unverified::Merge)?;
    let moved = rewrite::merge_with(ctx.repo()?, &target, mode, skipped.is_some())?;
    let label = match mode {
        MergeMode::Squash => say!("rewrite-entry-squash-merge", target = target.as_str()),
        _ => say!("rewrite-entry-merge", target = target.as_str()),
    };
    let out = finish(ctx, label, moved, ResetMode::Keep)?;
    Ok(with_skipped(out, skipped))
}

fn rebase(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let onto: String = param(params, "onto")?;
    let options = RebaseOptions {
        after: param_or(params, "after", None)?,
        autostash: param_or(params, "autostash", false)?,
        no_verify: param_or(params, "no_verify", false)?,
    };
    let skipped = hooks_left_out(ctx, params, Unverified::Rebase)?;
    let rebased = rewrite::rebase_with(ctx.repo()?, &onto, &options)?;
    let mut out = finish(
        ctx,
        say!("rewrite-entry-rebase", onto = onto.as_str()),
        rebased.moved,
        ResetMode::Keep,
    )?;
    if rebased.stash_kept
        && let Some(out) = out.as_object_mut()
    {
        // The rebase is done; the changes that were set aside did not
        // fit back in and are in the stash.
        out.insert("stash_kept".to_owned(), json!(true));
    }
    Ok(with_skipped(out, skipped))
}

fn predict(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let target: String = param(params, "target")?;
    let aside: bool = param_or(params, "aside", false)?;
    let ours: Option<String> = param_or(params, "ours", None)?;
    let prediction = match ours {
        Some(ours) => rewrite::predict_between(ctx.repo()?, &ours, &target, aside)?,
        None if aside => rewrite::predict_aside(ctx.repo()?, &target)?,
        None => rewrite::predict(ctx.repo()?, &target)?,
    };
    Ok(json!(prediction))
}

fn reset(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let target: String = param(params, "target")?;
    let mode: ResetMode = param_or(params, "mode", ResetMode::default())?;
    let moved = rewrite::reset(ctx.repo()?, &target, mode)?;
    let how = match mode {
        ResetMode::Soft => "soft",
        ResetMode::Mixed => "mixed",
        ResetMode::Hard => "hard",
        ResetMode::Keep => "keep",
    };
    let to = moved
        .after
        .commit
        .map(|id| id.short(7))
        .unwrap_or_else(|| target.clone());
    finish(
        ctx,
        say!("rewrite-entry-reset", commit = to, mode = how),
        moved,
        mode,
    )
}

fn cherry_pick(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commits: Vec<String> = param(params, "commits")?;
    let no_commit: bool = param_or(params, "no_commit", false)?;
    let mainline: Option<u32> = param_or(params, "mainline", None)?;
    let moved = rewrite::cherry_pick(ctx.repo()?, &commits, mainline, no_commit)?;
    finish(
        ctx,
        say!("rewrite-entry-cherry-pick", count = commits.len()),
        moved,
        ResetMode::Keep,
    )
}

fn revert(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commits: Vec<String> = param(params, "commits")?;
    let mainline: Option<u32> = param_or(params, "mainline", None)?;
    let moved = rewrite::revert(ctx.repo()?, &commits, mainline)?;
    finish(
        ctx,
        say!("rewrite-entry-revert", count = commits.len()),
        moved,
        ResetMode::Keep,
    )
}
