//! The inbox's commands: `inbox.items`, the user's open pull requests and
//! issues of every connected account, or of a workspace's repositories,
//! with the repositories on this machine that have uncommitted work; and
//! `inbox.pin`, `.unpin`, `.snooze` and `.unsnooze`, which keep a row at
//! the top or out of sight ([`crate::inbox::store`]).
//!
//! Each account is asked once, with its kept answers in front of it as
//! the lists of pull requests and issues are; an account that cannot be
//! asked says why in its own entry and the others are answered all the
//! same. A row is found on this machine when one of the repositories
//! looked at has a remote that is its repository at that account (or the
//! fork it comes from). The repositories looked at are a workspace's
//! (`workspace`, its id in `workspaces.toml`), and those named (`paths`:
//! the window names the open and recent ones). With a workspace, only
//! the rows of its repositories are answered.
//!
//! Sources: the documents named in [`crate::github`], [`crate::gitlab`]
//! and [`crate::inbox`]; git-status(1) (through `repo.glance`'s look).

use std::{collections::BTreeSet, path::PathBuf};

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, ParamKind, ParamSpec, Params, Registry,
};
use bitbraid_git::glance;

use crate::{
    accounts,
    cmds::{self, FREE, FRESH, failed, invalid, opt, req},
    inbox::{
        self, AccountState, Inbox, InboxIssue, InboxPull, MAX_PATHS, SnoozeFor, WorkInProgress,
        store::{self, Marks, Pin, Snooze},
    },
    model::{Account, RepoLink},
    provider::{self, InboxQuery},
    remotes,
    services::Services,
};

/// Most of each kind asked of an account.
const MOST: usize = 50;
/// Longest path taken, in bytes.
const MAX_PATH: usize = 4096;

const ITEMS: &[ParamSpec] = &[
    opt(
        "workspace",
        ParamKind::Text,
        message!("inbox-param-items-workspace"),
    ),
    opt(
        "paths",
        ParamKind::List,
        message!("inbox-param-items-paths"),
    ),
    opt(
        "account",
        ParamKind::Text,
        message!("inbox-param-items-account"),
    ),
    opt("wip", ParamKind::Flag, message!("inbox-param-items-wip")),
    FRESH,
];
const KEY: ParamSpec = req("key", ParamKind::Text, message!("inbox-param-key"));
const ONE: &[ParamSpec] = &[KEY];
const SNOOZE: &[ParamSpec] = &[
    KEY,
    opt(
        "for",
        ParamKind::Choice(&["hour", "four_hours", "tomorrow", "unsnoozed"]),
        message!("inbox-param-snooze-for"),
    ),
    opt(
        "until",
        ParamKind::Text,
        message!("inbox-param-snooze-until"),
    ),
];

/// Register the inbox's commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register_described(
        CommandSpec {
            id: "inbox.items",
            label: say!("inbox-items"),
            description: say!("inbox-items-description"),
        },
        items,
        FREE,
        ITEMS,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "inbox.pin",
            label: say!("inbox-pin"),
            description: say!("inbox-pin-description"),
        },
        pin,
        FREE,
        ONE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "inbox.unpin",
            label: say!("inbox-unpin"),
            description: say!("inbox-unpin-description"),
        },
        unpin,
        FREE,
        ONE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "inbox.snooze",
            label: say!("inbox-snooze"),
            description: say!("inbox-snooze-description"),
        },
        snooze,
        FREE,
        SNOOZE,
    );
    let _ = registry.register_described(
        CommandSpec {
            id: "inbox.unsnooze",
            label: say!("inbox-unsnooze"),
            description: say!("inbox-unsnooze-description"),
        },
        unsnooze,
        FREE,
        ONE,
    );
}

/// A row's key, as it may be kept.
fn row_key(params: &Params) -> Result<String, CommandError> {
    let key = cmds::text_with(params, "key", inbox::MAX_KEY, false)?
        .ok_or_else(|| invalid("key", say!("inbox-missing")))?;
    if !inbox::usable_key(&key) {
        return Err(invalid("key", say!("inbox-not-a-key")));
    }
    Ok(key)
}

/// The full paths named by `paths`.
fn paths(params: &Params) -> Result<Vec<String>, CommandError> {
    let Some(value) = params.get("paths").filter(|value| !value.is_null()) else {
        return Ok(Vec::new());
    };
    let items = value
        .as_array()
        .ok_or_else(|| invalid("paths", say!("inbox-paths-not-a-list")))?;
    if items.len() > MAX_PATHS {
        return Err(invalid(
            "paths",
            say!("inbox-paths-too-many", most = MAX_PATHS),
        ));
    }
    let mut out: Vec<String> = Vec::new();
    for item in items {
        let text = item
            .as_str()
            .ok_or_else(|| invalid("paths", say!("inbox-paths-not-texts")))?;
        if text.is_empty()
            || text.len() > MAX_PATH
            || text.chars().any(char::is_control)
            || text.starts_with('-')
            || !std::path::Path::new(text).is_absolute()
        {
            return Err(invalid("paths", say!("inbox-paths-not-full")));
        }
        if !out.iter().any(|known| known == text) {
            out.push(text.to_owned());
        }
    }
    Ok(out)
}

/// A repository looked at: its folder, whether it is there, and the
/// addresses of its remotes.
struct Local {
    path: String,
    present: bool,
    remotes: Vec<(String, String)>,
}

/// The remotes of the repository in a folder; none when it cannot be
/// opened.
fn remotes_of(path: &str) -> Vec<(String, String)> {
    let Ok(repo) = bitbraid_git::Repository::open(path) else {
        return Vec::new();
    };
    let Ok(snapshot) = bitbraid_git::snapshot(&repo) else {
        return Vec::new();
    };
    snapshot
        .remotes
        .iter()
        .filter_map(|remote| Some((remote.name.clone(), remote.fetch_url.clone()?)))
        .collect()
}

/// The repositories to look at: the workspace's (whether it was found),
/// then those named.
fn locals(
    ctx: &CommandContext,
    services: &Services,
    workspace: Option<&str>,
    named: &[String],
) -> Result<(Vec<Local>, BTreeSet<String>), CommandError> {
    let mut out: Vec<Local> = Vec::new();
    let mut of_workspace = BTreeSet::new();
    if let Some(id) = workspace {
        let loaded = bitbraid_engine::workspace::load(&services.config_dir)?;
        let found = loaded
            .workspaces
            .iter()
            .find(|known| known.id == id)
            .ok_or_else(|| {
                invalid(
                    "workspace",
                    say!("inbox-no-workspace", id = format!("{id:?}")),
                )
            })?;
        for repository in found.repositories.iter().take(MAX_PATHS) {
            if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            let present = repository.is_present();
            let mut remotes = if present {
                remotes_of(&repository.path)
            } else {
                Vec::new()
            };
            // One that is not here yet, or whose remotes cannot be read:
            // the address it was added with.
            if remotes.is_empty()
                && let Some(url) = &repository.remote_url
            {
                remotes.push(("origin".to_owned(), url.clone()));
            }
            of_workspace.insert(repository.path.clone());
            out.push(Local {
                path: repository.path.clone(),
                present,
                remotes,
            });
        }
    }
    for path in named {
        if out.iter().any(|known| &known.path == path) {
            continue;
        }
        if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let present = std::path::Path::new(path).is_dir();
        out.push(Local {
            path: path.clone(),
            present,
            remotes: if present {
                remotes_of(path)
            } else {
                Vec::new()
            },
        });
    }
    Ok((out, of_workspace))
}

/// The repository here of a row and its remote: the first looked at
/// with a remote that is `repo` (or `fork`) at the account.
fn here(
    links: &[(String, Vec<RepoLink>)],
    account: &str,
    repo: &str,
    fork: Option<&str>,
) -> (Option<String>, Option<String>) {
    let same = |link: &RepoLink, wanted: &str| {
        link.account == account
            && format!("{}/{}", link.owner, link.name).eq_ignore_ascii_case(wanted)
    };
    let find = |wanted: &str| {
        links.iter().find_map(|(path, found)| {
            found
                .iter()
                .find(|link| same(link, wanted))
                .map(|link| (Some(path.clone()), Some(link.remote.clone())))
        })
    };
    find(repo).or_else(|| find(fork?)).unwrap_or((None, None))
}

fn items(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let workspace = cmds::text_with(params, "workspace", 64, false)?;
    let named = paths(params)?;
    let only = cmds::text(params, "account")?;
    let wip = cmds::flag(params, "wip")?.unwrap_or(true);
    cmds::flag(params, "fresh")?;
    let services = cmds::services(ctx)?;
    let now = (services.now)();
    let known = accounts::load(&services.config_dir).map_err(failed)?;
    if let Some(id) = &only
        && known.find(id).is_none()
    {
        return Err(invalid(
            "account",
            say!("inbox-no-account", id = format!("{id:?}")),
        ));
    }
    let (locals, of_workspace) = locals(ctx, &services, workspace.as_deref(), &named)?;
    let links: Vec<(String, Vec<RepoLink>)> = locals
        .iter()
        .map(|local| {
            let found = remotes::links(&local.remotes, &known.accounts, |account| {
                known.ssh_hosts(account)
            });
            (local.path.clone(), found)
        })
        .collect();
    // With a workspace, the repositories of each account it has.
    let scope_of = |account: &Account| -> Option<Vec<String>> {
        workspace.as_ref()?;
        let mut repos: Vec<String> = Vec::new();
        for (path, found) in &links {
            if !of_workspace.contains(path) {
                continue;
            }
            for link in found.iter().filter(|link| link.account == account.id) {
                let name = format!("{}/{}", link.owner, link.name);
                if !repos.iter().any(|known| known.eq_ignore_ascii_case(&name)) {
                    repos.push(name);
                }
            }
        }
        Some(repos)
    };
    let loaded = store::load(&services.config_dir);
    let marks = loaded.marks.clone();
    let mut answer = Inbox {
        marks_problem: loaded.read_only,
        ..Inbox::default()
    };
    for account in known
        .accounts
        .iter()
        .filter(|account| only.as_ref().is_none_or(|id| *id == account.id))
    {
        let scope = scope_of(account);
        if scope.as_ref().is_some_and(Vec::is_empty) {
            // The workspace has nothing at this account.
            continue;
        }
        let mut state = AccountState {
            account: account.id.clone(),
            kind: account.kind,
            web: account.web.clone(),
            login: account.login.clone(),
            freshness: Default::default(),
            more: false,
            error: None,
        };
        let asked = cmds::signed_as(ctx, params, &services, account.clone()).and_then(|signed| {
            provider::of(account.kind)
                .inbox(
                    &signed.session(ctx),
                    &InboxQuery {
                        repos: scope.clone(),
                        most: MOST,
                    },
                )
                .map_err(failed)
        });
        let found = match asked {
            Ok(found) => found,
            Err(err) => {
                if ctx.cancel.load(std::sync::atomic::Ordering::Relaxed) {
                    return Err(err);
                }
                state.error = Some(err.to_string());
                answer.accounts.push(state);
                continue;
            }
        };
        state.freshness = found.freshness;
        state.more = found.more;
        answer.accounts.push(state);
        let in_scope = |repo: &str, fork: Option<&str>| {
            scope.as_ref().is_none_or(|repos| {
                repos.iter().any(|known| {
                    known.eq_ignore_ascii_case(repo)
                        || fork.is_some_and(|fork| known.eq_ignore_ascii_case(fork))
                })
            })
        };
        for found in found.pulls {
            let pull = found.pull;
            if !in_scope(&pull.repo, pull.head_repo.as_deref()) {
                continue;
            }
            let key = inbox::key(&account.id, false, &pull.repo, pull.number);
            let (path, remote) = here(&links, &account.id, &pull.repo, pull.head_repo.as_deref());
            answer.pulls.push(InboxPull {
                groups: inbox::groups(&pull, &found.facts, &account.login),
                path,
                remote,
                pinned: marks.pinned(&key),
                snoozed: marks.snoozed(&key, now),
                key,
                account: account.id.clone(),
                kind: account.kind,
                additions: found.facts.additions,
                deletions: found.facts.deletions,
                mergeable: found.facts.mergeable,
                approval: found.facts.approval,
                milestone: found.facts.milestone,
                involvement: found.involvement,
                pull,
            });
        }
        for found in found.issues {
            let issue = found.issue;
            if !in_scope(&issue.repo, None) {
                continue;
            }
            let key = inbox::key(&account.id, true, &issue.repo, issue.number);
            let (path, remote) = here(&links, &account.id, &issue.repo, None);
            answer.issues.push(InboxIssue {
                path,
                remote,
                pinned: marks.pinned(&key),
                snoozed: marks.snoozed(&key, now),
                key,
                account: account.id.clone(),
                kind: account.kind,
                reactions: found.reactions,
                involvement: found.involvement,
                issue,
            });
        }
    }
    answer.pulls.sort_by(|a, b| {
        (b.pinned, &b.pull.updated)
            .cmp(&(a.pinned, &a.pull.updated))
            .then_with(|| a.key.cmp(&b.key))
    });
    answer.issues.sort_by(|a, b| {
        (b.pinned, &b.issue.updated)
            .cmp(&(a.pinned, &a.issue.updated))
            .then_with(|| a.key.cmp(&b.key))
    });
    if wip {
        let looked: Vec<PathBuf> = locals
            .iter()
            .filter(|local| local.present)
            .filter(|local| workspace.is_none() || of_workspace.contains(&local.path))
            .map(|local| PathBuf::from(&local.path))
            .collect();
        for chunk in looked.chunks(glance::MAX_GLANCED) {
            for seen in glance::glance_all(chunk, &ctx.cancel) {
                if seen.problem.is_some() || seen.counts.total() == 0 {
                    continue;
                }
                let path = seen.path.to_string_lossy().into_owned();
                answer.work.push(WorkInProgress {
                    name: seen
                        .path
                        .file_name()
                        .map_or_else(|| path.clone(), |name| name.to_string_lossy().into_owned()),
                    path,
                    branch: seen.branch,
                    ahead: seen.ahead,
                    behind: seen.behind,
                    counts: seen.counts,
                });
            }
        }
    }
    cmds::answer(&answer)
}

/// The configuration directory of a command, and its clock.
fn where_and_when(ctx: &CommandContext) -> Result<(PathBuf, i64), CommandError> {
    let services = cmds::services(ctx)?;
    Ok((services.config_dir.clone(), (services.now)()))
}

fn refused(why: String) -> CommandError {
    CommandError::Git(bitbraid_git::Error::Refused {
        what: say!("inbox-refused"),
        detail: why,
    })
}

fn changed(
    ctx: &CommandContext,
    change: impl FnOnce(&mut Marks, i64),
) -> Result<Output, CommandError> {
    let (dir, now) = where_and_when(ctx)?;
    let marks = store::change(&dir, now, |marks| change(marks, now)).map_err(refused)?;
    cmds::answer(&marks)
}

fn pin(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let key = row_key(params)?;
    changed(ctx, |marks, now| {
        if !marks.pinned(&key) {
            marks.pins.push(Pin { key, since: now });
        }
    })
}

fn unpin(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let key = row_key(params)?;
    changed(ctx, |marks, _| marks.pins.retain(|pin| pin.key != key))
}

/// A time `until` names: seconds, or a local date and time.
fn until(params: &Params, now: i64) -> Result<Option<i64>, CommandError> {
    let Some(value) = params.get("until").filter(|value| !value.is_null()) else {
        return Ok(None);
    };
    let at = match value {
        serde_json::Value::Number(number) => number.as_i64(),
        serde_json::Value::String(text) => {
            let text = text.trim();
            if text.len() > 64 || text.chars().any(char::is_control) {
                None
            } else if let Ok(seconds) = text.parse::<i64>() {
                Some(seconds)
            } else if text.as_bytes().get(4) == Some(&b'-') {
                bitbraid_git::search::instant(text, now)
            } else {
                None
            }
        }
        _ => None,
    }
    .ok_or_else(|| invalid("until", say!("inbox-until-unreadable")))?;
    if at <= now {
        return Err(invalid("until", say!("inbox-until-past")));
    }
    Ok(Some(at))
}

fn snooze(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let key = row_key(params)?;
    let (_, now) = where_and_when(ctx)?;
    let length = match (
        cmds::choice(
            params,
            "for",
            &["hour", "four_hours", "tomorrow", "unsnoozed"],
            "",
        )?,
        until(params, now)?,
    ) {
        ("", Some(at)) => SnoozeFor::Until(at),
        ("", None) | ("unsnoozed", None) => SnoozeFor::Unsnoozed,
        ("hour", None) => SnoozeFor::Hour,
        ("four_hours", None) => SnoozeFor::FourHours,
        ("tomorrow", None) => SnoozeFor::Tomorrow,
        _ => return Err(invalid("until", say!("inbox-until-with-for"))),
    };
    let zone = bitbraid_git::search::Zone::system();
    changed(ctx, |marks, now| {
        let ends = inbox::snooze_end(length, now, |at| zone.offset_at(at));
        marks.snoozes.retain(|snooze| snooze.key != key);
        marks.snoozes.push(Snooze {
            key,
            since: now,
            until: ends,
        });
    })
}

fn unsnooze(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let key = row_key(params)?;
    changed(ctx, |marks, _| {
        marks.snoozes.retain(|snooze| snooze.key != key)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ProviderKind;

    fn link(account: &str, owner: &str, name: &str) -> RepoLink {
        RepoLink {
            remote: "origin".into(),
            account: account.into(),
            kind: ProviderKind::Github,
            owner: owner.into(),
            name: name.into(),
        }
    }

    #[test]
    fn a_row_is_found_here_by_its_repository_or_its_fork() {
        let links = vec![
            ("/a".to_owned(), vec![link("one", "o", "r")]),
            (
                "/b".to_owned(),
                vec![link("two", "o", "r"), link("one", "me", "fork")],
            ),
        ];
        let at = |path: &str| (Some(path.to_owned()), Some("origin".to_owned()));
        assert_eq!(here(&links, "one", "O/R", None), at("/a"));
        assert_eq!(here(&links, "two", "o/r", None), at("/b"));
        assert_eq!(here(&links, "one", "x/y", Some("me/fork")), at("/b"));
        assert_eq!(here(&links, "three", "o/r", None), (None, None));
    }

    #[test]
    fn paths_are_full_paths_and_bounded() {
        let read = |value: serde_json::Value| paths(&serde_json::json!({ "paths": value }));
        // A full path begins with a drive on Windows.
        let root = if cfg!(windows) { "C:" } else { "" };
        let (a, b) = (format!("{root}/a"), format!("{root}/b"));
        assert_eq!(
            read(serde_json::json!([a, a, b])).unwrap(),
            [a.clone(), b.clone()]
        );
        for bad in [
            serde_json::json!(a),
            serde_json::json!([1]),
            serde_json::json!(["relative"]),
            serde_json::json!([format!("{root}/a\nb")]),
            serde_json::json!([""]),
            serde_json::json!([format!("{root}/{}", "a".repeat(MAX_PATH))]),
        ] {
            assert!(read(bad.clone()).is_err(), "{bad}");
        }
        let many: Vec<String> = (0..=MAX_PATHS).map(|at| format!("{root}/r{at}")).collect();
        assert!(read(serde_json::json!(many)).is_err());
        assert!(paths(&serde_json::json!({})).unwrap().is_empty());
    }

    #[test]
    fn until_is_seconds_or_a_local_time_in_the_future() {
        let at = |value: serde_json::Value| until(&serde_json::json!({ "until": value }), 1000);
        assert_eq!(at(serde_json::json!(2000)).unwrap(), Some(2000));
        assert_eq!(at(serde_json::json!("2000")).unwrap(), Some(2000));
        assert!(at(serde_json::json!("2099-01-01T09:00")).unwrap().is_some());
        for bad in [
            serde_json::json!(999),
            serde_json::json!(1000),
            serde_json::json!("soon"),
            serde_json::json!("3d"),
            serde_json::json!(true),
            serde_json::json!("2026-13-01"),
        ] {
            assert!(at(bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(until(&serde_json::json!({}), 0).unwrap(), None);
    }
}
