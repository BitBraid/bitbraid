//! Repository read commands: open, references, log, graph, status, diff,
//! blame, file history.
//!
//! Sources: none (original design).

use bitbraid_git::{DiffOptions, DiffSide, HistoryOptions, LogOptions, Order, Tips};
use serde_json::json;

use crate::command::{
    CommandContext, CommandError, CommandSpec, Output, Params, Registry, param, param_or,
};

/// Register every repository read command.
pub fn register(registry: &mut Registry) {
    // Registration cannot fail for the built-in set; ids are unique by construction.
    let _ = registry.register(
        CommandSpec {
            id: "repo.open",
            label: say!("repo-open"),
            description: say!("repo-open-description"),
        },
        open,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.summary",
            label: say!("repo-summary"),
            description: say!("repo-summary-description"),
        },
        summary,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.refs",
            label: say!("repo-refs"),
            description: say!("repo-refs-description"),
        },
        refs,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.log",
            label: say!("repo-log"),
            description: say!("repo-log-description"),
        },
        log,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.graph",
            label: say!("repo-graph"),
            description: say!("repo-graph-description"),
        },
        graph,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.commits",
            label: say!("repo-commits"),
            description: say!("repo-commits-description"),
        },
        commits,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.status",
            label: say!("repo-status"),
            description: say!("repo-status-description"),
        },
        status,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.diff",
            label: say!("repo-diff"),
            description: say!("repo-diff-description"),
        },
        diff,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.blame",
            label: say!("repo-blame"),
            description: say!("repo-blame-description"),
        },
        blame,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.blame_previous",
            label: say!("repo-blame-previous"),
            description: say!("repo-blame-previous-description"),
        },
        blame_previous,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.blame_ignores",
            label: say!("repo-blame-ignores"),
            description: say!("repo-blame-ignores-description"),
        },
        blame_ignores,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.history",
            label: say!("repo-history"),
            description: say!("repo-history-description"),
        },
        history,
    );
}

fn log_options(params: &Params) -> Result<LogOptions, CommandError> {
    Ok(LogOptions {
        tips: param_or(params, "tips", Tips::All)?,
        limit: param_or(params, "limit", None)?,
        first_parent: param_or(params, "first_parent", false)?,
        order: param_or(params, "order", Order::Date)?,
    })
}

fn open(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param_or(params, "path", ".".to_owned())?;
    let summary = ctx.open(&path)?;
    Ok(serde_json::to_value(summary)?)
}

fn summary(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(ctx.repo()?.summary()?)?)
}

fn refs(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(bitbraid_git::snapshot(ctx.repo()?)?)?)
}

fn log(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let options = log_options(params)?;
    Ok(serde_json::to_value(bitbraid_git::log(
        ctx.repo()?,
        &options,
    )?)?)
}

fn graph(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let options = log_options(params)?;
    let ascii: bool = param_or(params, "ascii", false)?;
    let with_details: bool = param_or(params, "details", false)?;
    // Branches pinned to the left: each keeps a lane from its
    // tip down its first parents. One that is not in the walk keeps none.
    let pinned: Vec<String> = param_or(params, "pinned", Vec::new())?;
    if pinned.len() > bitbraid_git::MAX_PINNED {
        return Err(CommandError::InvalidParam {
            name: "pinned".to_owned(),
            detail: say!("repo-too-many-pinned", max = bitbraid_git::MAX_PINNED),
        });
    }
    let pins = pinned
        .iter()
        .map(|rev| bitbraid_git::branch::commit_of(repo, rev))
        .collect::<Result<Vec<_>, _>>()?;
    let commits = bitbraid_git::walk(repo, &options)?;
    // A row per stash, hung on the commit it was made on; one
    // whose commit is not in the walk is left out.
    let commits = if param_or(params, "stashes", false)? {
        let stashes: Vec<bitbraid_git::Oid> = bitbraid_git::snapshot(repo)?
            .stashes
            .iter()
            .map(|stash| stash.target)
            .collect();
        bitbraid_git::with_stashes_in(repo, commits, &stashes, options.order)
    } else {
        commits
    };
    let rows = bitbraid_git::layout_pinned(&commits, &pins);
    let mut out = json!({ "commits": commits, "rows": rows });
    if (ascii || with_details)
        && let Some(object) = out.as_object_mut()
    {
        let ids: Vec<bitbraid_git::Oid> = commits.iter().map(|c| c.id).collect();
        let details = bitbraid_git::details(repo, &ids)?;
        if ascii {
            let art = bitbraid_git::ascii(&details, &rows, |c| {
                format!("{} {}", c.id.short(7), c.summary)
            });
            object.insert("ascii".to_owned(), json!(art));
        }
        if with_details {
            object.insert("details".to_owned(), json!(details));
        }
    }
    Ok(out)
}

fn commits(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let ids: Vec<bitbraid_git::Oid> = param(params, "ids")?;
    Ok(serde_json::to_value(bitbraid_git::details(
        ctx.repo()?,
        &ids,
    )?)?)
}

fn status(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let limit: Option<usize> = param_or(params, "limit", None)?;
    match limit {
        None => Ok(serde_json::to_value(bitbraid_git::status(ctx.repo()?)?)?),
        // Bounded: the scan stops there, and says so.
        Some(limit) => {
            let (entries, more) = bitbraid_git::status_bounded(ctx.repo()?, limit)?;
            Ok(json!({ "entries": entries, "more": more }))
        }
    }
}

fn diff(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let old_text: String = param_or(params, "old", "HEAD".to_owned())?;
    let new_text: String = param_or(params, "new", "worktree".to_owned())?;
    let old = DiffSide::parse(repo, &old_text)?;
    let new = DiffSide::parse(repo, &new_text)?;
    let options = DiffOptions {
        context: param_or(params, "context", 3)?,
        paths: param_or(params, "paths", Vec::new())?,
        detect_renames: param_or(params, "detect_renames", true)?,
        whitespace: param_or(params, "whitespace", Default::default())?,
        max_bytes: param_or(params, "max_bytes", None)?,
    };
    let tree_sides = |side: &DiffSide| !matches!(side, DiffSide::Index | DiffSide::Worktree);
    if tree_sides(&old) && tree_sides(&new) {
        return Ok(serde_json::to_value(bitbraid_git::diff_trees(
            repo, &old, &new, &options,
        )?)?);
    }
    // The index or the worktree on a side: every path that differs
    // between the two, which for a commit other than HEAD is more than
    // what git's status lists.
    Ok(serde_json::to_value(bitbraid_git::compare::diff_mixed(
        repo, &old, &new, &options,
    )?)?)
}

fn blame(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let path: String = param(params, "path")?;
    let rev: Option<String> = param_or(params, "rev", None)?;
    let rev = match rev {
        Some(text) => match DiffSide::parse(repo, &text)? {
            DiffSide::Commit(id) => Some(id),
            _ => {
                return Err(CommandError::InvalidParam {
                    name: "rev".into(),
                    detail: say!("repo-not-a-commit").into(),
                });
            }
        },
        None => None,
    };
    // Revisions to pass over: when asked for, or, when nothing is said,
    // where git's configuration names a file of them, as `git blame` does
    // by itself.
    let ignore: Option<bool> = param_or(params, "ignore_revs", None)?;
    let listed = bitbraid_git::blame_revs::ignore_revs(repo);
    let pass_over = match (ignore, listed) {
        (Some(false), _) => None,
        (Some(true), None) => {
            return Err(CommandError::InvalidParam {
                name: "ignore_revs".into(),
                detail: say!(
                    "repo-no-ignore-revs",
                    file = bitbraid_git::blame_revs::CONVENTIONAL
                ),
            });
        }
        (Some(true), Some(listed)) => Some(listed.file),
        (None, Some(listed)) if listed.configured && listed.exists => Some(listed.file),
        (None, _) => None,
    };
    let lines = match pass_over {
        Some(file) => bitbraid_git::blame_revs::blame_ignoring(repo, &path, rev, &file)?,
        None => bitbraid_git::blame(repo, &path, rev)?,
    };
    Ok(serde_json::to_value(lines)?)
}

fn blame_previous(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let path: String = param(params, "path")?;
    let rev: String = param(params, "rev")?;
    let DiffSide::Commit(commit) = DiffSide::parse(repo, &rev)? else {
        return Err(CommandError::InvalidParam {
            name: "rev".into(),
            detail: say!("repo-not-a-commit").into(),
        });
    };
    Ok(match bitbraid_git::blame::previous(repo, commit, &path)? {
        Some((rev, path)) => json!({ "rev": rev, "path": path }),
        None => serde_json::Value::Null,
    })
}

fn blame_ignores(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    Ok(serde_json::to_value(
        bitbraid_git::blame_revs::ignore_revs(ctx.repo()?),
    )?)
}

fn history(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let repo = ctx.repo()?;
    let path: String = param(params, "path")?;
    let options = HistoryOptions {
        log: LogOptions {
            tips: param_or(params, "tips", Tips::Head)?,
            limit: param_or(params, "limit", None)?,
            first_parent: param_or(params, "first_parent", false)?,
            order: Order::Date,
        },
        follow: param_or(params, "follow", true)?,
        max_entries: param_or(params, "max_entries", None)?,
    };
    Ok(serde_json::to_value(bitbraid_git::file_history(
        repo, &path, &options,
    )?)?)
}
