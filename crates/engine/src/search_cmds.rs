//! Search commands: the query grammar, the commits a query holds for,
//! content search, files by path, and the commit to jump to. The `issue:`
//! keyword finds the commits whose message names an issue.
//!
//! Sources: git-log(1), git-rev-list(1), git-rev-parse(1); docs.github.com
//! "Autolinked references and URLs" and docs.gitlab.com "GitLab Flavored
//! Markdown" for how issues are named.

use bitbraid_git::{
    Tips,
    search::{self, ContentSearch, SearchOptions, Zone},
};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    remote_cmds::reporting,
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "search.parse",
            label: say!("search-parse"),
            description: say!("search-parse-description"),
        },
        parse,
    );
    let _ = registry.register(
        CommandSpec {
            id: "search.commits",
            label: say!("search-commits"),
            description: say!("search-commits-description"),
        },
        commits,
    );
    let _ = registry.register(
        CommandSpec {
            id: "search.content",
            label: say!("search-content"),
            description: say!("search-content-description"),
        },
        content,
    );
    let _ = registry.register(
        CommandSpec {
            id: "search.files",
            label: say!("search-files"),
            description: say!("search-files-description"),
        },
        files,
    );
    let _ = registry.register(
        CommandSpec {
            id: "search.locate",
            label: say!("search-locate"),
            description: say!("search-locate-description"),
        },
        locate,
    );
}

/// What ages count back from: the `now` parameter, the clock without it.
fn now_of(params: &Params) -> Result<i64, CommandError> {
    let given: Option<i64> = param_or(params, "now", None)?;
    Ok(given.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .and_then(|since| i64::try_from(since.as_secs()).ok())
            .unwrap_or_default()
    }))
}

/// The time zone dates are read in: the `zone` parameter, a name from
/// the time zone database, or the system's without it, which is how `git
/// log` reads a date.
fn zone_of(params: &Params) -> Result<Zone, CommandError> {
    let named: Option<String> = param_or(params, "zone", None)?;
    match named {
        None => Ok(Zone::system()),
        Some(name) => Zone::named(&name).ok_or_else(|| CommandError::InvalidParam {
            name: "zone".to_owned(),
            detail: say!("search-bad-zone").to_owned(),
        }),
    }
}

fn options_of(params: &Params) -> Result<SearchOptions, CommandError> {
    Ok(SearchOptions {
        tips: param_or(params, "tips", Tips::All)?,
        limit: param_or(params, "limit", search::DEFAULT_LIMIT)?,
    })
}

fn parse(_: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let query: String = param(params, "query")?;
    let now = now_of(params)?;
    let zone = zone_of(params)?;
    // A query that cannot be read is an answer here, not a failure: the
    // interface asks while it is being typed.
    Ok(match search::parse_in(&query, now, &zone) {
        Ok(query) => serde_json::to_value(query)?,
        Err(error) => json!({ "error": error }),
    })
}

fn commits(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let text: String = param(params, "query")?;
    let now = now_of(params)?;
    let options = options_of(params)?;
    let zone = zone_of(params)?;
    let query = search::parse_in(&text, now, &zone).map_err(|err| CommandError::InvalidParam {
        name: "query".to_owned(),
        detail: err.to_string(),
    })?;
    let found = reporting(ctx, |repo, reporter| {
        search::commits(repo, &query, &options, reporter)
    })?;
    Ok(serde_json::to_value(found)?)
}

fn content(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let what = ContentSearch {
        text: param(params, "text")?,
        regex: param_or(params, "regex", false)?,
        paths: param_or(params, "paths", Vec::new())?,
    };
    let options = options_of(params)?;
    let found = reporting(ctx, |repo, reporter| {
        search::content(repo, &what, &options, reporter)
    })?;
    Ok(serde_json::to_value(found)?)
}

/// Find a file by its path, to open its history.
fn files(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let query: String = param(params, "query")?;
    let rev: Option<String> = param_or(params, "rev", None)?;
    let limit: usize = param_or(params, "limit", 50)?;
    let cancel = ctx.cancel.clone();
    Ok(serde_json::to_value(bitbraid_git::find::files(
        ctx.repo()?,
        rev.as_deref(),
        &query,
        limit,
        &cancel,
    )?)?)
}

fn locate(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let bad_date = || CommandError::InvalidParam {
        name: "date".to_owned(),
        detail: say!("search-bad-date", forms = search::DATE_FORMS.to_string()),
    };
    let rev: Option<String> = param_or(params, "rev", None)?;
    let date = params.get("date").filter(|value| !value.is_null());
    match (date, rev) {
        (Some(date), None) => {
            let seconds = match date {
                serde_json::Value::Number(number) => number.as_i64().ok_or_else(bad_date)?,
                serde_json::Value::String(text) => {
                    search::instant_in(text, now_of(params)?, &zone_of(params)?)
                        .ok_or_else(bad_date)?
                }
                _ => return Err(bad_date()),
            };
            let tips: Tips = param_or(params, "tips", Tips::All)?;
            let id = search::locate_date(ctx.repo()?, seconds, &tips)?;
            Ok(json!({ "id": id, "seconds": seconds }))
        }
        (None, Some(rev)) => {
            let id = search::locate_rev(ctx.repo()?, &rev)?;
            Ok(json!({ "id": id }))
        }
        _ => Err(CommandError::InvalidParam {
            name: "date".to_owned(),
            detail: say!("search-date-or-rev").to_owned(),
        }),
    }
}
