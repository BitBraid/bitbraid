//! Patch commands: write commits or uncommitted work out as patches,
//! say whether a patch file would apply, apply one, and apply a mailbox of
//! them as commits.
//!
//! An applied patch is journalled with the patch itself, kept as a blob, so
//! that undo applies it the other way round. A mailbox makes commits: that
//! is journalled as a move of the branch, like a cherry-pick, and one that
//! stops half-way leaves the same note for when it is finished. An export
//! can take a while and stops on the context's flag; it belongs on the
//! operations thread.
//!
//! Sources: git-format-patch(1), git-diff(1), git-apply(1), git-am(1).

use std::path::Path;

use bitbraid_git::{
    ResetMode,
    patch::{self, Destination, Options, Source},
};
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
            id: "patch.export",
            label: say!("patch-export"),
            description: say!("patch-export-description"),
        },
        export,
    );
    let _ = registry.register(
        CommandSpec {
            id: "patch.check",
            label: say!("patch-check"),
            description: say!("patch-check-description"),
        },
        check,
    );
    let _ = registry.register(
        CommandSpec {
            id: "patch.apply",
            label: say!("patch-apply"),
            description: say!("patch-apply-description"),
        },
        apply,
    );
    let _ = registry.register(
        CommandSpec {
            id: "patch.mailbox",
            label: say!("patch-mailbox"),
            description: say!("patch-mailbox-description"),
        },
        mailbox,
    );
}

fn one_of(name: &str, detail: &str) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.to_owned(),
    }
}

fn export(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let commits: Option<Vec<String>> = param_or(params, "commits", None)?;
    let from: Option<String> = param_or(params, "from", None)?;
    let to: Option<String> = param_or(params, "to", None)?;
    let staged: bool = param_or(params, "staged", false)?;
    let worktree: bool = param_or(params, "worktree", false)?;
    let dir: Option<String> = param_or(params, "dir", None)?;
    let file: Option<String> = param_or(params, "file", None)?;
    let overwrite: bool = param_or(params, "overwrite", false)?;
    // Exactly one source and one destination; `to` goes with `from` only.
    let source = match (commits, from, staged, worktree) {
        (Some(commits), None, false, false) if to.is_none() => Source::Commits(commits),
        (None, Some(from), false, false) => Source::Range {
            from,
            to: to.unwrap_or_else(|| "HEAD".to_owned()),
        },
        (None, None, true, false) if to.is_none() => Source::Staged,
        (None, None, false, true) if to.is_none() => Source::Worktree,
        (None, None, false, false) if to.is_none() => {
            return Err(one_of("commits", say!("patch-what-to-export")));
        }
        _ => {
            return Err(one_of("commits", say!("patch-one-source")));
        }
    };
    let destination = match (dir, file) {
        (Some(dir), None) => Destination::Dir(dir),
        (None, Some(file)) => Destination::File(file),
        (None, None) => return Err(one_of("file", say!("patch-where-to-write"))),
        _ => return Err(one_of("file", say!("patch-one-destination"))),
    };
    // Many commits take a while to write: it stops on the context's flag.
    let cancel = ctx.cancel.clone();
    let exported = patch::export(ctx.repo()?, &source, &destination, overwrite, &cancel)?;
    Ok(serde_json::to_value(exported)?)
}

fn options(params: &Params) -> Result<Options, CommandError> {
    Ok(Options {
        index: param_or(params, "index", false)?,
        three_way: param_or(params, "three_way", false)?,
        reverse: param_or(params, "reverse", false)?,
    })
}

fn check(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let file: String = param(params, "file")?;
    let options = options(params)?;
    Ok(serde_json::to_value(patch::check(
        ctx.repo()?,
        &file,
        options,
    )?)?)
}

/// The name of a file for a label: its last part.
fn name_of(file: &str) -> String {
    Path::new(file)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| file.to_owned())
}

fn apply(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let file: String = param(params, "file")?;
    let options = options(params)?;
    let mut applied = patch::apply(ctx.repo()?, &file, options)?;
    let name = name_of(&file);
    let label = if applied.reverse {
        say!("patch-entry-apply-reverse", name = name.as_str())
    } else {
        say!("patch-entry-apply", name = name.as_str())
    };
    // What a merge left in conflict is not what the patch says any more:
    // the patch the other way round would not take it back, but what the
    // merge did to the files and the index is put back as it was.
    let files_change = applied.files_change.take();
    let undoable = if applied.conflicts.is_empty() {
        ctx.record(
            label,
            Change::PatchApply {
                patch: applied.patch,
                index: applied.index,
                reverse: applied.reverse,
            },
        )?;
        true
    } else if let Some(files) = files_change {
        ctx.record(label, Change::FilesMerged { files })?;
        true
    } else {
        false
    };
    let mut out = serde_json::to_value(applied)?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("undoable".to_owned(), json!(undoable));
    }
    Ok(out)
}

fn mailbox(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let file: String = param(params, "file")?;
    let three_way: Option<bool> = param_or(params, "three_way", None)?;
    let skipped =
        crate::rewrite_cmds::hooks_left_out(ctx, params, bitbraid_git::hooks::Unverified::Mailbox)?;
    let mailed = patch::mailbox_with(ctx.repo()?, &file, three_way, skipped.is_some())?;
    let label = say!("patch-entry-mailbox", name = name_of(&file));
    let mut out = crate::rewrite_cmds::finish(ctx, label, mailed.moved, ResetMode::Keep)?;
    if let Some(fields) = out.as_object_mut() {
        fields.insert("commits".to_owned(), json!(mailed.commits));
    }
    Ok(crate::rewrite_cmds::with_skipped(out, skipped))
}
