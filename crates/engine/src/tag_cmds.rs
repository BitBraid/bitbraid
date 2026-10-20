//! Tag commands: create, delete, push, delete on a remote, and a message
//! given to a lightweight tag afterwards.
//!
//! Sources: git-tag(1), git-push(1).

use bitbraid_git::{remote, tag};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "tag.create",
            label: say!("tag-create"),
            description: say!("tag-create-description"),
        },
        create,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tag.annotate",
            label: say!("tag-annotate"),
            description: say!("tag-annotate-description"),
        },
        annotate,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tag.delete",
            label: say!("tag-delete"),
            description: say!("tag-delete-description"),
        },
        delete,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tag.push",
            label: say!("tag-push"),
            description: say!("tag-push-description"),
        },
        push,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tag.delete_remote",
            label: say!("tag-delete-remote"),
            description: say!("tag-delete-remote-description"),
        },
        delete_remote,
    );
}

fn create(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let target: Option<String> = param_or(params, "target", None)?;
    let message: Option<String> = param_or(params, "message", None)?;
    let sign: bool = param_or(params, "sign", false)?;
    let value = tag::tag_create(
        ctx.repo()?,
        &name,
        target.as_deref(),
        message.as_deref(),
        sign,
    )?;
    ctx.record(
        say!("tag-entry-create", name = name.as_str()),
        Change::TagCreate {
            name: name.clone(),
            value,
        },
    )?;
    Ok(json!({ "tag": name, "value": value }))
}

fn annotate(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let message: String = param(params, "message")?;
    let (before, after) = tag::tag_annotate(ctx.repo()?, &name, &message)?;
    ctx.record(
        say!("tag-entry-annotate", name = name.as_str()),
        Change::TagAnnotate {
            name: name.clone(),
            before,
            after,
        },
    )?;
    Ok(json!({ "tag": name, "value": after }))
}

fn delete(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let value = tag::tag_delete(ctx.repo()?, &name, None)?;
    ctx.record(
        say!("tag-entry-delete", name = name.as_str()),
        Change::TagDelete {
            name: name.clone(),
            value,
        },
    )?;
    Ok(json!({ "deleted": name, "value": value }))
}

/// The remote a command is about: the one named, or the default.
fn which_remote(ctx: &CommandContext, params: &Params) -> Result<String, CommandError> {
    let given: Option<String> = param_or(params, "remote", None)?;
    match given {
        Some(name) => Ok(name),
        None => Ok(remote::default_remote(ctx.repo()?)?),
    }
}

fn push(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: Option<String> = param_or(params, "name", None)?;
    let to = which_remote(ctx, params)?;
    let said = reporting(ctx, |repo, reporter| {
        remote::push_tag(repo, &to, name.as_deref(), reporter)
    })?;
    Ok(json!({ "pushed": name, "remote": to, "output": said }))
}

fn delete_remote(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    let from = which_remote(ctx, params)?;
    let said = reporting(ctx, |repo, reporter| {
        remote::delete_remote_tag(repo, &from, &name, reporter)
    })?;
    Ok(json!({ "deleted": name, "remote": from, "output": said }))
}
