//! Signing commands: how the repository signs, the keys there are to
//! sign with, the settings, what git makes of the signatures on commits,
//! and a tag in full with its own signature.
//!
//! Signing itself is the `sign` parameter of `commit.create` and
//! `tag.create`. Only `sign.configure` writes: settings, journalled as
//! any change of the repository's own settings.
//!
//! Sources: git-config(1), git-log(1) (`%G?`), git-verify-tag(1).

use std::path::PathBuf;

use bitbraid_git::{
    Oid,
    signature::{self, Format, Scope, Settings},
};

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Longest directory name taken, in bytes.
const MAX_PATH: usize = 4096;

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "sign.status",
            label: say!("sign-status"),
            description: say!("sign-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.keys",
            label: say!("sign-keys"),
            description: say!("sign-keys-description"),
        },
        keys,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.configure",
            label: say!("sign-configure"),
            description: say!("sign-configure-description"),
        },
        configure,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.verify",
            label: say!("sign-verify"),
            description: say!("sign-verify-description"),
        },
        verify,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.signed",
            label: say!("sign-signed"),
            description: say!("sign-signed-description"),
        },
        signed,
    );
    let _ = registry.register(
        CommandSpec {
            id: "tag.details",
            label: say!("sign-tag-details"),
            description: say!("sign-tag-details-description"),
        },
        tag_details,
    );
}

fn bad(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

fn status(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(signature::status(ctx.repo()?))?)
}

/// The user's home, where ssh keeps its keys.
fn home() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|home| !home.is_empty())
        .map(PathBuf::from)
}

fn keys(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let given: Option<String> = param_or(params, "ssh_dir", None)?;
    let agent: bool = param_or(params, "agent", true)?;
    let dir = match given {
        Some(text) => {
            let path = PathBuf::from(&text);
            if text.len() > MAX_PATH
                || text.starts_with('-')
                || text.chars().any(char::is_control)
                || !path.is_absolute()
            {
                return Err(bad("ssh_dir", say!("sign-ssh-dir-not-full")));
            }
            Some(path)
        }
        None => home().map(|home| home.join(".ssh")),
    };
    Ok(serde_json::to_value(signature::keys(
        ctx.repo()?,
        dir.as_deref(),
        agent,
    ))?)
}

fn configure(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let format: Option<String> = param_or(params, "format", None)?;
    let key: Option<String> = param_or(params, "key", None)?;
    let commits: Option<bool> = param_or(params, "commits", None)?;
    let tags: Option<bool> = param_or(params, "tags", None)?;
    let scope: String = param_or(params, "scope", "repo".to_owned())?;
    let format = match format {
        Some(text) => {
            Some(Format::parse(&text).ok_or_else(|| bad("format", say!("sign-format-values")))?)
        }
        None => None,
    };
    let scope = Scope::parse(&scope).ok_or_else(|| bad("scope", say!("sign-scope-values")))?;
    let settings = Settings {
        format,
        key,
        commits,
        tags,
    };
    // What it changes in this repository's own settings is journalled;
    // the user's settings are not this repository's to take back.
    crate::settings::journalled(
        ctx,
        |repo| Ok(signature::configure(repo, &settings, scope)?),
        |()| say!("sign-entry-configure").to_owned(),
    )?;
    Ok(serde_json::to_value(signature::status(ctx.repo()?))?)
}

fn verify(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let ids: Vec<Oid> = param(params, "ids")?;
    Ok(serde_json::to_value(signature::verify(ctx.repo()?, &ids)?)?)
}

fn signed(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let ids: Vec<Oid> = param(params, "ids")?;
    Ok(serde_json::to_value(signature::signed(ctx.repo()?, &ids)?)?)
}

fn tag_details(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let name: String = param(params, "name")?;
    Ok(serde_json::to_value(signature::tag_details(
        ctx.repo()?,
        &name,
    )?)?)
}
