//! Branch housekeeping commands: look the local branches over,
//! describe one, delete the merged ones. Each deletion is journalled like
//! one of `branch.delete`, so undo brings the branches back one by one.
//!
//! Sources: git-branch(1), git-for-each-ref(1), git-config(1).

use bitbraid_git::housekeeping::{self, Keep, Kept};
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
            id: "branch.overview",
            label: say!("housekeeping-branch-overview"),
            description: say!("housekeeping-branch-overview-description"),
        },
        overview,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.describe",
            label: say!("housekeeping-branch-describe"),
            description: say!("housekeeping-branch-describe-description"),
        },
        describe,
    );
    let _ = registry.register(
        CommandSpec {
            id: "branch.delete_merged",
            label: say!("housekeeping-branch-delete-merged"),
            description: say!("housekeeping-branch-delete-merged-description"),
        },
        delete_merged,
    );
}

fn overview(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let base: Option<String> = param_or(params, "base", None)?;
    let stale_days: u32 = param_or(params, "stale_days", housekeeping::DEFAULT_STALE_DAYS)?;
    let now: Option<i64> = param_or(params, "now", None)?;
    let overview = housekeeping::overview(ctx.repo()?, base.as_deref(), stale_days, now)?;
    Ok(serde_json::to_value(overview)?)
}

fn describe(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let description: String = param_or(params, "description", String::new())?;
    // A description is a setting of the repository's, and undone as one.
    let described = crate::settings::journalled(
        ctx,
        |repo| Ok(housekeeping::describe(repo, &name, &description)?),
        |described| match described.after {
            Some(_) => say!("housekeeping-entry-describe", name = name.as_str()),
            None => say!("housekeeping-entry-undescribe", name = name.as_str()),
        },
    )?;
    Ok(json!({
        "branch": name,
        "description": described.after,
        "before": described.before,
    }))
}

fn delete_merged(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let base: Option<String> = param_or(params, "base", None)?;
    let protected: Vec<String> = param_or(params, "protected", Vec::new())?;
    let dry_run: bool = param_or(params, "dry_run", false)?;
    let plan = housekeeping::merged_plan(ctx.repo()?, base.as_deref(), &protected)?;
    let mut kept = plan.kept;
    let mut deleted = Vec::new();
    for branch in plan.delete {
        if dry_run {
            deleted.push(branch);
            continue;
        }
        match housekeeping::delete_merged(ctx.repo()?, &branch) {
            Ok(gone) => {
                ctx.record(
                    say!("housekeeping-entry-delete", name = gone.name.as_str()),
                    Change::BranchDelete {
                        name: gone.name,
                        target: gone.target,
                        upstream: gone.upstream,
                    },
                )?;
                deleted.push(branch);
            }
            // One that git will not let go does not stop the others.
            Err(err) => kept.push(Kept {
                name: branch.name,
                reason: Keep::Failed,
                detail: Some(err.to_string()),
                gone: branch.gone,
            }),
        }
    }
    kept.sort_by(|one, other| one.name.cmp(&other.name));
    Ok(json!({
        "base": plan.base,
        "dry_run": dry_run,
        "deleted": deleted,
        "kept": kept,
    }))
}
