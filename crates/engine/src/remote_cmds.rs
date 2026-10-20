//! Remotes: fetch, pull, push and what a push would do, and adding,
//! removing and pointing remotes. The three that talk to a remote report
//! progress to the context and stop when it says so.
//!
//! Sources: git-fetch(1), git-pull(1), git-push(1) (`--no-verify` among
//! it), git-remote(1).

use bitbraid_git::{Force, PullMode, PushOptions, Reporter, ResetMode, remote};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
};

/// Register the remote commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "remote.fetch",
            label: say!("remote-fetch"),
            description: say!("remote-fetch-description"),
        },
        fetch,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.pull",
            label: say!("remote-pull"),
            description: say!("remote-pull-description"),
        },
        pull,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.push",
            label: say!("remote-push"),
            description: say!("remote-push-description"),
        },
        push,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.push_preview",
            label: say!("remote-push-preview"),
            description: say!("remote-push-preview-description"),
        },
        push_preview,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.add",
            label: say!("remote-add"),
            description: say!("remote-add-description"),
        },
        add,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.remove",
            label: say!("remote-remove"),
            description: say!("remote-remove-description"),
        },
        remove,
    );
    let _ = registry.register(
        CommandSpec {
            id: "remote.set_url",
            label: say!("remote-set-url"),
            description: say!("remote-set-url-description"),
        },
        set_url,
    );
}

/// Run something that reports through the context's sink and can be
/// stopped through its flag.
pub(crate) fn reporting<T>(
    ctx: &CommandContext,
    run: impl FnOnce(&bitbraid_git::Repository, &mut Reporter<'_>) -> bitbraid_git::Result<T>,
) -> Result<T, CommandError> {
    let repo = ctx.repo()?;
    let sink = ctx.progress.clone();
    let mut on_progress = |progress| {
        if let Some(sink) = &sink {
            sink(progress);
        }
    };
    let mut reporter = Reporter {
        cancel: ctx.cancel.clone(),
        on_progress: &mut on_progress,
    };
    Ok(run(repo, &mut reporter)?)
}

fn fetch(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote_name: Option<String> = param_or(params, "remote", None)?;
    let reference: Option<String> = param_or(params, "reference", None)?;
    if let Some(reference) = reference {
        // One reference into FETCH_HEAD: a pull request's head.
        let remote_name = remote_name.ok_or_else(|| CommandError::InvalidParam {
            name: "remote".to_owned(),
            detail: say!("remote-reference-from-one").to_owned(),
        })?;
        let (said, commit) = reporting(ctx, |repo, reporter| {
            remote::fetch_reference(repo, &remote_name, &reference, reporter)
        })?;
        return Ok(json!({
            "fetched": remote_name,
            "reference": reference,
            "commit": commit.to_hex(),
            "output": said,
        }));
    }
    let prune: bool = param_or(params, "prune", false)?;
    let tags: bool = param_or(params, "tags", false)?;
    let quiet: bool = param_or(params, "quiet", false)?;
    let said = reporting(ctx, |repo, reporter| {
        remote::fetch_with(repo, remote_name.as_deref(), prune, tags, quiet, reporter)
    })?;
    Ok(json!({ "fetched": remote_name, "output": said }))
}

fn pull(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let mode: PullMode = param_or(params, "mode", PullMode::default())?;
    let before = bitbraid_git::branch::head_state(ctx.repo()?)?;
    let pulled = reporting(ctx, |repo, reporter| remote::pull(repo, mode, reporter))?;
    let after = bitbraid_git::branch::head_state(ctx.repo()?)?;
    if pulled.stopped.is_some() {
        // Its merge or rebase is in progress; journalled when that is done.
        return crate::rewrite_cmds::finish(
            ctx,
            say!("remote-entry-pull").to_owned(),
            bitbraid_git::Moved {
                before,
                after,
                backups: Vec::new(),
                stopped: pulled.stopped,
                files: None,
            },
            ResetMode::Keep,
        );
    }
    let said = pulled.output;
    let out = json!({ "before": before, "after": after, "output": said });
    if before != after {
        // What was fetched stays; undo puts the branch back.
        ctx.record(
            say!("remote-entry-pull").to_owned(),
            Change::HeadMoved {
                before,
                after,
                mode: ResetMode::Keep,
                backups: Vec::new(),
            },
        )?;
    }
    Ok(out)
}

fn push(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let options = PushOptions {
        remote: param_or(params, "remote", None)?,
        branch: param_or(params, "branch", None)?,
        set_upstream: param_or(params, "set_upstream", false)?,
        force: param_or(params, "force", Force::No)?,
        tags: param_or(params, "tags", false)?,
        expect: param_or(params, "expect", None)?,
        to: param_or(params, "to", None)?,
        no_verify: param_or(params, "no_verify", false)?,
    };
    // Read before the push: what is said afterwards is what it went
    // without.
    let skipped =
        crate::rewrite_cmds::hooks_left_out(ctx, params, bitbraid_git::hooks::Unverified::Push)?;
    let said = reporting(ctx, |repo, reporter| remote::push(repo, &options, reporter))?;
    let out = json!({ "pushed": options.branch, "remote": options.remote, "output": said });
    Ok(crate::rewrite_cmds::with_skipped(out, skipped))
}

fn push_preview(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let remote_name: Option<String> = param_or(params, "remote", None)?;
    let branch: Option<String> = param_or(params, "branch", None)?;
    let preview = remote::push_preview(ctx.repo()?, remote_name.as_deref(), branch.as_deref())?;
    let lost: Vec<Output> = preview
        .lost
        .iter()
        .map(|(id, summary)| json!({ "id": id, "summary": summary }))
        .collect();
    Ok(json!({
        "branch": preview.branch,
        "remote": preview.remote,
        "there": preview.there,
        "ahead": preview.ahead,
        "behind": preview.behind,
        "lost": lost,
    }))
}

fn add(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let url: String = param(params, "url")?;
    // A remote that was only added is its settings and nothing else:
    // undo takes them away again.
    crate::settings::journalled(
        ctx,
        |repo| Ok(remote::remote_add(repo, &name, &url)?),
        |()| say!("remote-entry-add", name = name.as_str()),
    )?;
    Ok(json!({ "added": name }))
}

fn remove(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let removed = remote::remote_remove(ctx.repo()?, &name)?;
    let dropped = removed.references.len();
    ctx.record(
        say!("remote-entry-remove", name = name.as_str()),
        Change::RemoteRemove {
            name: removed.name,
            settings: removed.settings,
            references: removed.references,
        },
    )?;
    Ok(json!({ "removed": name, "references": dropped }))
}

fn set_url(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let url: String = param(params, "url")?;
    let push: bool = param_or(params, "push", false)?;
    crate::settings::journalled(
        ctx,
        |repo| Ok(remote::remote_set_url(repo, &name, &url, push)?),
        |()| {
            if push {
                say!("remote-entry-set-push-url", name = name.as_str())
            } else {
                say!("remote-entry-set-url", name = name.as_str())
            }
        },
    )?;
    Ok(json!({ "remote": name, "url": url, "push": push }))
}
