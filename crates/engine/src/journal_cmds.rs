//! The undo journal as a whole: everything that can be undone and
//! redone, and going back or forward several entries at once.
//!
//! Going to an entry is `undo.last` or `redo.last` taken as often as it
//! needs, so each step does exactly what a single one does, with the same
//! checks. A step that cannot be taken ends it: what was undone until then
//! stays undone, and the answer names both that and the error, as a result
//! rather than as the command's error, so that neither is lost.
//!
//! Sources: none (original design).

use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Handler, Output, Params, param},
    commit_cmds,
    journal::{Change, Entry, Journal},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "undo.journal",
            label: say!("journal-undo-journal"),
            description: say!("journal-undo-journal-description"),
        },
        journal,
    );
    let _ = registry.register(
        CommandSpec {
            id: "undo.to",
            label: say!("journal-undo-to"),
            description: say!("journal-undo-to-description"),
        },
        undo_to,
    );
    let _ = registry.register(
        CommandSpec {
            id: "redo.to",
            label: say!("journal-redo-to"),
            description: say!("journal-redo-to-description"),
        },
        redo_to,
    );
}

/// The kind of a change: the tag it is stored under.
fn kind(change: &Change) -> Option<String> {
    let value = serde_json::to_value(change).ok()?;
    value.get("kind")?.as_str().map(str::to_owned)
}

fn row(entry: &Entry) -> Output {
    json!({
        "id": entry.id,
        "label": entry.label,
        "time": entry.time,
        "kind": kind(&entry.change),
    })
}

fn journal(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let journal = ctx.journal.as_ref().ok_or(CommandError::NoRepository)?;
    // Read as one list, redo above undo, it is every entry from the newest
    // to the oldest, with the place the repository is at in between: undo
    // takes the first below it, redo the last above it.
    let undo: Vec<Output> = journal.done.iter().rev().map(row).collect();
    let redo: Vec<Output> = journal.undone.iter().map(row).collect();
    Ok(json!({
        "undo": undo,
        "redo": redo,
        "next_undo": journal.peek_undo().map(|entry| entry.id),
        "next_redo": journal.peek_redo().map(|entry| entry.id),
    }))
}

fn not_there(what: &'static str, id: u64) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what,
        detail: say!("journal-no-entry", id, what),
    })
}

/// How many steps it takes until the entry `id` has been taken from the
/// top of `stack`, when it is on it.
fn steps_to(stack: &[Entry], id: u64) -> Option<usize> {
    let at = stack.iter().rposition(|entry| entry.id == id)?;
    Some(stack.len().saturating_sub(at))
}

fn undo_to(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id: u64 = param(params, "id")?;
    ctx.repo()?;
    let journal = ctx.journal.as_ref().ok_or(CommandError::NoRepository)?;
    let steps = steps_to(&journal.done, id).ok_or_else(|| not_there(say!("journal-undo"), id))?;
    walk(ctx, steps, "undone", commit_cmds::undo, Journal::peek_undo)
}

fn redo_to(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let id: u64 = param(params, "id")?;
    ctx.repo()?;
    let journal = ctx.journal.as_ref().ok_or(CommandError::NoRepository)?;
    let steps = steps_to(&journal.undone, id).ok_or_else(|| not_there(say!("journal-redo"), id))?;
    walk(ctx, steps, "redone", commit_cmds::redo, Journal::peek_redo)
}

/// Take `steps` single steps and stop at the first that fails. The labels
/// of the entries taken are answered under `key`, in the order they were
/// taken.
fn walk(
    ctx: &mut CommandContext,
    steps: usize,
    key: &'static str,
    step: Handler,
    next: fn(&Journal) -> Option<&Entry>,
) -> Result<Output, CommandError> {
    let mut taken = Vec::new();
    let mut error = None;
    let mut failed = Output::Null;
    for _ in 0..steps {
        let upcoming = ctx
            .journal
            .as_ref()
            .and_then(next)
            .map(|entry| (entry.id, entry.label.clone()));
        let Some((id, label)) = upcoming else { break };
        match step(ctx, &Params::Null) {
            Ok(_) => taken.push(label),
            Err(err) => {
                error = Some(err.to_string());
                failed = json!({ "id": id, "label": label });
                break;
            }
        }
    }
    Ok(json!({
        key: taken,
        "complete": taken.len() == steps,
        "error": error,
        "failed": failed,
    }))
}
