//! What the commit box asks about messages: the messages of the commits
//! that were made here lately, to take one back from; trailers added to a
//! message by git's own rules; and the trailer keys the repository uses,
//! to offer them.
//!
//! None of them changes the repository: `commit.trailer` answers with the
//! message as it would read, and the caller commits it or not.
//!
//! Sources: git-interpret-trailers(1), git-reflog(1).

use bitbraid_git::message::{self, Trailer};
use serde_json::json;

use crate::command::{
    CommandContext, CommandError, CommandSpec, Output, Params, Registry, param, param_or,
};

/// Register the message commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "commit.messages",
            label: say!("message-commit-messages"),
            description: say!("message-commit-messages-description"),
        },
        messages,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.trailer",
            label: say!("message-commit-trailer"),
            description: say!("message-commit-trailer-description"),
        },
        trailer,
    );
    let _ = registry.register(
        CommandSpec {
            id: "commit.trailer_keys",
            label: say!("message-commit-trailer-keys"),
            description: say!("message-commit-trailer-keys-description"),
        },
        trailer_keys,
    );
}

fn messages(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", 30)?;
    Ok(serde_json::to_value(message::recent_messages(
        ctx.repo()?,
        limit.min(500),
    )?)?)
}

fn trailer(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let text: String = param(params, "message")?;
    let mut trailers: Vec<Trailer> = param_or(params, "trailers", Vec::new())?;
    let sign_off: bool = param_or(params, "sign_off", false)?;
    let repo = ctx.repo()?;
    if sign_off {
        trailers.push(message::sign_off(repo)?);
    }
    if trailers.is_empty() {
        return Err(CommandError::InvalidParam {
            name: "trailers".into(),
            detail: say!("message-trailers-missing").into(),
        });
    }
    Ok(json!({ "message": message::add_trailers(repo, &text, &trailers)? }))
}

fn trailer_keys(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: usize = param_or(params, "limit", 300)?;
    Ok(serde_json::to_value(message::trailer_keys(
        ctx.repo()?,
        limit,
    )?)?)
}
