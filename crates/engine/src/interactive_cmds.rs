//! Interactive rebase: the commits a plan can be made for, what a
//! plan leads to, and carrying it out through git's sequencer. Rewording
//! or dropping one commit and squashing commits together are plans with
//! one change and go the same way; where a plain list of commits cannot
//! say the way up to HEAD (a merge commit on it, or nothing before the
//! commit), git writes the list and the one change is made in it
//! (`bitbraid_git::rewrite_one`), merges kept. Moving commits one place
//! up or down is a plan that swaps them with their neighbour; it has no
//! such second way and is refused across a merge commit.
//!
//! A plan that runs to its end is one step for undo, like any other move
//! of HEAD. One that stops (on a conflict, or at a commit that is to be
//! edited) leaves a note and becomes that step when it is finished.
//!
//! Sources: git-rebase(1).

use bitbraid_git::{
    Oid, ResetMode,
    interactive::{self, Action, Range, Step},
    rewrite_one::{self, Change},
};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    rewrite_cmds::finish,
};

/// Most commits squashed together in one go by naming them.
const MAX_SQUASHED: usize = 100;

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "rebase.plan",
            label: say!("interactive-rebase-plan"),
            description: say!("interactive-rebase-plan-description"),
        },
        plan,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rebase.outline",
            label: say!("interactive-rebase-outline"),
            description: say!("interactive-rebase-outline-description"),
        },
        outline,
    );
    let _ = registry.register(
        CommandSpec {
            id: "rebase.interactive",
            label: say!("interactive-rebase-interactive"),
            description: say!("interactive-rebase-interactive-description"),
        },
        run,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.reword",
            label: say!("interactive-commit-reword"),
            description: say!("interactive-commit-reword-description"),
        },
        reword,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.drop",
            label: say!("interactive-commit-drop"),
            description: say!("interactive-commit-drop-description"),
        },
        drop_commit,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.squash",
            label: say!("interactive-commit-squash"),
            description: say!("interactive-commit-squash-description"),
        },
        squash,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.move",
            label: say!("interactive-commit-move"),
            description: say!("interactive-commit-move-description"),
        },
        move_commits,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.rewrite_way",
            label: say!("interactive-commit-rewrite-way"),
            description: say!("interactive-commit-rewrite-way-description"),
        },
        rewrite_way,
    );
}

fn plan(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let first: Option<String> = param_or(params, "first", None)?;
    let onto: Option<String> = param_or(params, "onto", None)?;
    let repo = ctx.repo()?;
    // Where the plan begins: where it is said to, or at the first commit
    // the new base does not have.
    let first = match (first, &onto) {
        (Some(first), _) => first,
        (None, Some(onto)) => interactive::first_after(repo, onto)?.to_hex(),
        (None, None) => {
            return Err(CommandError::InvalidParam {
                name: "first".to_owned(),
                detail: say!("interactive-no-start").to_owned(),
            });
        }
    };
    let range = interactive::range(repo, &first)?;
    let mut out = json!(range);
    if let Some(out) = out.as_object_mut() {
        // The commit itself, however it was named.
        out.insert("first".to_owned(), json!(first_of(&range)));
        if let Some(onto) = &onto {
            let base = bitbraid_git::branch::commit_of(repo, onto)?;
            out.insert("onto".to_owned(), json!(base));
        }
    }
    Ok(out)
}

fn outline(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let first: String = param(params, "first")?;
    let steps: Vec<Step> = param(params, "steps")?;
    let range = interactive::range(ctx.repo()?, &first)?;
    let result = interactive::outline(&range, &steps)?;
    Ok(json!({ "result": result }))
}

fn carry_out(
    ctx: &mut CommandContext,
    first: &str,
    steps: &[Step],
    label: String,
) -> Result<Output, CommandError> {
    let moved = interactive::run(ctx.repo()?, first, steps)?;
    finish(ctx, label, moved, ResetMode::Keep)
}

fn run(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let first: String = param(params, "first")?;
    let steps: Vec<Step> = param(params, "steps")?;
    let onto: Option<String> = param_or(params, "onto", None)?;
    let count = steps.len();
    let label = match &onto {
        Some(onto) => say!("interactive-entry-rebase-onto", count, onto = onto.as_str()),
        None => say!("interactive-entry-rebase", count),
    };
    let skipped =
        crate::rewrite_cmds::hooks_left_out(ctx, params, bitbraid_git::hooks::Unverified::Rebase)?;
    let moved = interactive::run_onto_with(
        ctx.repo()?,
        &first,
        &steps,
        onto.as_deref(),
        skipped.is_some(),
    )?;
    let out = finish(ctx, label, moved, ResetMode::Keep)?;
    Ok(crate::rewrite_cmds::with_skipped(out, skipped))
}

/// The plan that changes nothing for the commits from `first`, to change
/// one thing in.
fn untouched(ctx: &mut CommandContext, first: &str) -> Result<(Range, Vec<Step>), CommandError> {
    let range = interactive::range(ctx.repo()?, first)?;
    let steps = interactive::untouched(&range);
    Ok((range, steps))
}

/// The commit a revision names, for a change to it.
fn named(ctx: &mut CommandContext, rev: &str) -> Result<Oid, CommandError> {
    Ok(bitbraid_git::branch::commit_of(ctx.repo()?, rev)?)
}

/// One change where a plain list does not do: git's own list, the change
/// made in it, merges kept.
fn across(
    ctx: &mut CommandContext,
    change: &Change,
    label: String,
) -> Result<Output, CommandError> {
    let moved = rewrite_one::run(ctx.repo()?, change)?;
    finish(ctx, label, moved, ResetMode::Keep)
}

fn rewrite_way(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commit: String = param(params, "commit")?;
    let id = named(ctx, &commit)?;
    let repo = ctx.repo()?;
    let parents = rewrite_one::parents(repo, id)?;
    let plain = !rewrite_one::needed(repo, id)?;
    // The commit by itself, as a plan of one has it: what is shown of it
    // where only its message can change.
    let alone = rewrite_one::alone(repo, id)?;
    Ok(json!({
        "commit": id,
        "plain": plain,
        "merge": parents.len() > 1,
        "first": parents.is_empty(),
        "alone": alone,
    }))
}

fn reword(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commit: String = param(params, "commit")?;
    let message: String = param(params, "message")?;
    let id = named(ctx, &commit)?;
    if rewrite_one::needed(ctx.repo()?, id)? {
        let label = say!("interactive-entry-reword", commit = id.short(7));
        return across(
            ctx,
            &Change::Reword {
                commit: id,
                message,
            },
            label,
        );
    }
    let (range, mut steps) = untouched(ctx, &commit)?;
    let mut label = say!("interactive-entry-reword-commit").to_owned();
    if let Some(step) = steps.first_mut() {
        label = say!("interactive-entry-reword", commit = step.id.short(7));
        step.action = Action::Reword;
        step.message = Some(message);
    }
    carry_out(ctx, &first_of(&range), &steps, label)
}

fn drop_commit(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commit: String = param(params, "commit")?;
    let id = named(ctx, &commit)?;
    if rewrite_one::needed(ctx.repo()?, id)? {
        let label = say!("interactive-entry-drop", commit = id.short(7));
        return across(ctx, &Change::Drop { commit: id }, label);
    }
    let (range, mut steps) = untouched(ctx, &commit)?;
    let mut label = say!("interactive-entry-drop-commit").to_owned();
    if let Some(step) = steps.first_mut() {
        label = say!("interactive-entry-drop", commit = step.id.short(7));
        step.action = Action::Drop;
    }
    carry_out(ctx, &first_of(&range), &steps, label)
}

fn squash(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commits: Vec<String> = param(params, "commits")?;
    let bad = |detail: &str| CommandError::InvalidParam {
        name: "commits".to_owned(),
        detail: detail.to_owned(),
    };
    if commits.is_empty() || commits.len() > MAX_SQUASHED {
        return Err(bad(say!("interactive-squash-how-many")));
    }
    // Where a merge commit is on the way up to HEAD, or the oldest of them
    // is the first commit of all: in git's own list.
    let mut ids = Vec::new();
    for commit in &commits {
        ids.push(named(ctx, commit)?);
    }
    let change = rewrite_one::squash_of(ctx.repo()?, &ids)?;
    if rewrite_one::needed(ctx.repo()?, change.oldest())? {
        let count = ids.len().max(2);
        return across(ctx, &change, say!("interactive-entry-squash", count));
    }
    // The range of each, to find the oldest: the one with the longest.
    let mut ranges = Vec::new();
    for commit in &commits {
        ranges.push(interactive::range(ctx.repo()?, commit)?);
    }
    let named: Vec<Oid> = ranges
        .iter()
        .filter_map(|range| range.commits.first().map(|entry| entry.id))
        .collect();
    let Some(mut range) = ranges.into_iter().max_by_key(|range| range.commits.len()) else {
        return Err(bad(say!("interactive-none-readable")));
    };
    if named.len() == 1 {
        // One commit goes into the one before it.
        range = interactive::range(ctx.repo()?, &range.base.to_hex())?;
    }
    let count = if named.len() == 1 { 2 } else { named.len() };
    let mut steps = interactive::untouched(&range);
    for (index, step) in steps.iter_mut().enumerate() {
        let in_run = index < count;
        if named.len() > 1 && in_run != named.contains(&step.id) {
            return Err(bad(say!("interactive-squash-not-a-run")));
        }
        if in_run && index > 0 {
            step.action = Action::Squash;
        }
    }
    if steps.len() < count {
        return Err(bad(say!("interactive-squash-not-in-history")));
    }
    carry_out(
        ctx,
        &first_of(&range),
        &steps,
        say!("interactive-entry-squash", count),
    )
}

fn move_commits(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commits: Vec<String> = param(params, "commits")?;
    let up: bool = param(params, "up")?;
    if commits.is_empty() || commits.len() > MAX_SQUASHED {
        return Err(CommandError::InvalidParam {
            name: "commits".to_owned(),
            detail: say!("interactive-move-how-many").to_owned(),
        });
    }
    let mut ids = Vec::new();
    for commit in &commits {
        ids.push(named(ctx, commit)?);
    }
    // The oldest of them is the one with the longest way up to HEAD; to
    // move down, the plan begins at the commit before it.
    let mut oldest: Option<Range> = None;
    for id in &ids {
        let range = interactive::range(ctx.repo()?, &id.to_hex())?;
        if oldest
            .as_ref()
            .is_none_or(|longest| range.commits.len() > longest.commits.len())
        {
            oldest = Some(range);
        }
    }
    let Some(oldest) = oldest else {
        return Err(CommandError::InvalidParam {
            name: "commits".to_owned(),
            detail: say!("interactive-none-readable").to_owned(),
        });
    };
    let range = if up {
        oldest
    } else {
        interactive::range(ctx.repo()?, &oldest.base.to_hex())?
    };
    let steps = interactive::moved(&range, &ids, up)?;
    let label = match (ids.as_slice(), up) {
        ([one], true) => say!("interactive-entry-move-up", commit = one.short(7)),
        ([one], false) => say!("interactive-entry-move-down", commit = one.short(7)),
        (many, true) => say!("interactive-entry-move-many-up", count = many.len()),
        (many, false) => say!("interactive-entry-move-many-down", count = many.len()),
    };
    carry_out(ctx, &first_of(&range), &steps, label)
}

/// The oldest commit of a range by its id, which is how a plan names where
/// it begins.
fn first_of(range: &Range) -> String {
    range
        .commits
        .first()
        .map(|entry| entry.id.to_hex())
        .unwrap_or_default()
}
