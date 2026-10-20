//! Allowed-signers commands: who may sign with an SSH key here, the
//! file that says so, a signer added and a signer taken out.
//!
//! Git checks an SSH signature against the file `gpg.ssh.allowedSignersFile`
//! names. Choosing that file writes a setting, and is journalled like any
//! change of the repository's settings. A signer put into the file or
//! taken out of it is journalled too, as the lines that came and went and
//! nothing else of the file: it is the user's own list of whom they
//! trust, usually one for all their repositories, and only what was done
//! from here is taken back, and only while the file still holds what that
//! left.
//!
//! Sources: ssh-keygen(1) ("ALLOWED SIGNERS"); git-config(1)
//! (`gpg.ssh.allowedSignersFile`); the XDG base directory specification
//! (`$XDG_CONFIG_HOME`, where git keeps the user's own files besides
//! `~/.gitconfig`).

use std::path::PathBuf;

use bitbraid_git::{
    branch,
    identity::Scope,
    signers::{self, KeyFrom},
};
use serde::Deserialize;
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
            id: "sign.signers",
            label: say!("signers-list"),
            description: say!("signers-list-description"),
        },
        list,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.signers_file",
            label: say!("signers-file"),
            description: say!("signers-file-description"),
        },
        use_file,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.allow",
            label: say!("signers-allow"),
            description: say!("signers-allow-description"),
        },
        allow,
    );
    let _ = registry.register(
        CommandSpec {
            id: "sign.disallow",
            label: say!("signers-disallow"),
            description: say!("signers-disallow-description"),
        },
        disallow,
    );
}

fn bad(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// Whose configuration names the file, as a parameter: `"repo"`,
/// `"user"`, or `{"file": "<path>"}`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Place {
    Repo,
    User,
    File(String),
}

fn list(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(signers::list(ctx.repo()?)?)?)
}

/// Where git keeps the user's own files: `$XDG_CONFIG_HOME/git`, or
/// `.config/git` in the home directory.
fn users_git_dir() -> Option<PathBuf> {
    let set = |name: &str| {
        std::env::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    set("XDG_CONFIG_HOME")
        .or_else(|| {
            bitbraid_core::paths::home()
                .filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })
        .map(|config| config.join("git"))
}

fn use_file(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: Option<String> = param_or(params, "path", None)?;
    let place: Place = param_or(params, "scope", Place::Repo)?;
    let path = path.filter(|path| !path.trim().is_empty());
    let (scope, path) = match (place, path) {
        (Place::Repo, Some(path)) => (Scope::Repo, path),
        (Place::Repo, None) => {
            let usual = signers::file_of_repository(ctx.repo()?);
            let usual = usual
                .to_str()
                .ok_or_else(|| bad("path", say!("signers-path-not-text")))?;
            (Scope::Repo, usual.to_owned())
        }
        (Place::User, Some(path)) => (Scope::User, path),
        (Place::User, None) => {
            let usual = users_git_dir()
                .map(|dir| dir.join("allowed_signers"))
                .and_then(|path| path.to_str().map(str::to_owned))
                .ok_or_else(|| bad("path", say!("signers-no-home")))?;
            (Scope::User, usual)
        }
        (Place::File(config), Some(path)) => (Scope::File(config.into()), path),
        (Place::File(_), None) => {
            return Err(bad("path", say!("signers-say-where")));
        }
    };
    // The repository's own setting is journalled; the user's is not this
    // repository's to take back.
    let chosen = crate::settings::journalled(
        ctx,
        |repo| Ok(signers::use_file(repo, &path, &scope)?),
        |_| say!("signers-entry-file").to_owned(),
    )?;
    Ok(serde_json::to_value(chosen)?)
}

fn allow(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let principals: Vec<String> = param_or(params, "principals", Vec::new())?;
    let key: Option<String> = param_or(params, "key", None)?;
    let key_file: Option<String> = param_or(params, "key_file", None)?;
    let commit: Option<String> = param_or(params, "commit", None)?;
    let repo = ctx.repo()?;
    let from = match (key, key_file, commit) {
        (Some(key), None, None) => KeyFrom::Text(key),
        (None, Some(file), None) => KeyFrom::File(PathBuf::from(file)),
        (None, None, Some(rev)) => KeyFrom::Commit(branch::commit_of(repo, &rev)?),
        (None, None, None) => {
            return Err(bad("key", say!("signers-which-key")));
        }
        _ => {
            return Err(bad("key", say!("signers-one-key")));
        }
    };
    let allowed = signers::allow(repo, &principals, &from)?;
    let out = serde_json::to_value(&allowed)?;
    if let Some(edit) = allowed.edit {
        let label = say!("signers-entry-allow", who = allowed.principals.join(", "));
        ctx.record(label, Change::Signers { edit })?;
    }
    Ok(out)
}

fn disallow(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let key: String = param(params, "key")?;
    let principals: Option<Vec<String>> = param_or(params, "principals", None)?;
    let done = signers::disallow(ctx.repo()?, &key, principals.as_deref())?;
    let mut out = serde_json::to_value(&done)?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("key".to_owned(), json!(key.trim()));
    }
    if let Some(edit) = done.edit {
        // Whose lines went, as the first of them names them.
        let who = edit
            .removed
            .first()
            .and_then(|(_, line)| line.split_whitespace().next())
            .map(|names| names.trim_matches('"').to_owned())
            .unwrap_or_else(|| key.trim().to_owned());
        ctx.record(
            say!("signers-entry-remove", who = who.as_str()),
            Change::Signers { edit },
        )?;
    }
    Ok(out)
}
