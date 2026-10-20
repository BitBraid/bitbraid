//! Git LFS commands: how it stands, setting it up, tracking
//! patterns, the LFS files of a commit, their content to and from a
//! remote and into the worktree, and locks.
//!
//! What moves content reports how far it is through the context's sink
//! and is stopped through its flag, as a fetch and a push are.
//!
//! Sources: git-lfs(1) and the pages of its commands.

use bitbraid_git::lfs;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "lfs.status",
            label: say!("lfs-status"),
            description: say!("lfs-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.install",
            label: say!("lfs-install"),
            description: say!("lfs-install-description"),
        },
        install,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.track",
            label: say!("lfs-track"),
            description: say!("lfs-track-description"),
        },
        track,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.untrack",
            label: say!("lfs-untrack"),
            description: say!("lfs-untrack-description"),
        },
        untrack,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.files",
            label: say!("lfs-files"),
            description: say!("lfs-files-description"),
        },
        files,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.fetch",
            label: say!("lfs-fetch"),
            description: say!("lfs-fetch-description"),
        },
        fetch,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.pull",
            label: say!("lfs-pull"),
            description: say!("lfs-pull-description"),
        },
        pull,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.push",
            label: say!("lfs-push"),
            description: say!("lfs-push-description"),
        },
        push,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.checkout",
            label: say!("lfs-checkout"),
            description: say!("lfs-checkout-description"),
        },
        checkout,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.locks",
            label: say!("lfs-locks"),
            description: say!("lfs-locks-description"),
        },
        locks,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.lock",
            label: say!("lfs-lock"),
            description: say!("lfs-lock-description"),
        },
        lock,
    );
    let _ = registry.register(
        CommandSpec {
            id: "lfs.unlock",
            label: say!("lfs-unlock"),
            description: say!("lfs-unlock-description"),
        },
        unlock,
    );
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(lfs::status(ctx.repo()?))?)
}

fn install(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(lfs::install(ctx.repo()?)?)?)
}

fn track(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let pattern: String = param(params, "pattern")?;
    let lockable: Option<bool> = param_or(params, "lockable", None)?;
    Ok(serde_json::to_value(lfs::track(
        ctx.repo()?,
        &pattern,
        lockable,
    )?)?)
}

fn untrack(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let pattern: String = param(params, "pattern")?;
    Ok(serde_json::to_value(lfs::untrack(ctx.repo()?, &pattern)?)?)
}

fn files(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let rev: Option<String> = param_or(params, "rev", None)?;
    Ok(serde_json::to_value(lfs::files(
        ctx.repo()?,
        rev.as_deref(),
    )?)?)
}

fn fetch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote: Option<String> = param_or(params, "remote", None)?;
    let all: bool = param_or(params, "all", false)?;
    let done = reporting(ctx, |repo, reporter| {
        lfs::fetch(repo, remote.as_deref(), all, reporter)
    })?;
    Ok(serde_json::to_value(done)?)
}

fn pull(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote: Option<String> = param_or(params, "remote", None)?;
    let done = reporting(ctx, |repo, reporter| {
        lfs::pull(repo, remote.as_deref(), reporter)
    })?;
    Ok(serde_json::to_value(done)?)
}

fn push(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote: Option<String> = param_or(params, "remote", None)?;
    let all: bool = param_or(params, "all", false)?;
    let done = reporting(ctx, |repo, reporter| {
        lfs::push(repo, remote.as_deref(), all, reporter)
    })?;
    Ok(serde_json::to_value(done)?)
}

fn checkout(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let done = reporting(ctx, lfs::checkout)?;
    Ok(serde_json::to_value(done)?)
}

fn locks(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let found = reporting(ctx, lfs::locks)?;
    Ok(serde_json::to_value(found)?)
}

fn lock(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let made = reporting(ctx, |repo, reporter| lfs::lock(repo, &path, reporter))?;
    Ok(serde_json::to_value(made)?)
}

fn unlock(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let force: bool = param_or(params, "force", false)?;
    let done = reporting(ctx, |repo, reporter| {
        lfs::unlock(repo, &path, force, reporter)
    })?;
    Ok(serde_json::to_value(done)?)
}
