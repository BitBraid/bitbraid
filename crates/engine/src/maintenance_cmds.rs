//! Repository maintenance: git's own maintenance tasks run through git, a
//! check of the objects that only reads, and how large the repository is
//! and what takes the room.
//!
//! What runs long reports how far it is through the context's sink and
//! is stopped through its flag, as a fetch is.
//!
//! Sources: git-gc(1), git-prune(1), git-repack(1), git-commit-graph(1),
//! git-fsck(1), git-count-objects(1), git-cat-file(1), git-log(1).

use bitbraid_git::maintenance::{self, SizeOptions, Task};

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "repo.maintenance",
            label: say!("maintenance-repo-maintenance"),
            description: say!("maintenance-repo-maintenance-description"),
        },
        run,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.fsck",
            label: say!("maintenance-repo-fsck"),
            description: say!("maintenance-repo-fsck-description"),
        },
        fsck,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.size",
            label: say!("maintenance-repo-size"),
            description: say!("maintenance-repo-size-description"),
        },
        size,
    );
}

fn run(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let task: Task = param(params, "task")?;
    let done = reporting(ctx, |repo, reporter| maintenance::run(repo, task, reporter))?;
    Ok(serde_json::to_value(done)?)
}

fn fsck(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let check = reporting(ctx, maintenance::fsck)?;
    Ok(serde_json::to_value(check)?)
}

fn size(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let blobs: usize = param_or(params, "blobs", maintenance::DEFAULT_BLOBS)?;
    let commits: u64 = param_or(params, "commits", maintenance::DEFAULT_COMMITS)?;
    if blobs > maintenance::MOST_BLOBS {
        return Err(CommandError::InvalidParam {
            name: "blobs".to_owned(),
            detail: say!("maintenance-too-many-blobs", max = maintenance::MOST_BLOBS),
        });
    }
    if commits > maintenance::MOST_COMMITS {
        return Err(CommandError::InvalidParam {
            name: "commits".to_owned(),
            detail: say!(
                "maintenance-too-many-commits",
                max = maintenance::MOST_COMMITS
            ),
        });
    }
    let report = reporting(ctx, |repo, reporter| {
        maintenance::size(repo, SizeOptions { blobs, commits }, reporter)
    })?;
    Ok(serde_json::to_value(report)?)
}
