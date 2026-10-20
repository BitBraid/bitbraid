//! Checking a pull request out in the open repository, and fetching its
//! head for a worktree of its own (`pr.head`).
//!
//! Its head is fetched from the remote its repository is
//! (`refs/pull/7/head` at GitHub, `refs/merge-requests/7/head` at GitLab),
//! so that it comes even when its branch is gone or its fork cannot be
//! reached. The branch it comes from is fetched too when it can be: from
//! the same remote, or from its fork's, which is added (named after the
//! fork's owner) when no remote is that fork yet. The local branch is
//! made at the head and follows that branch, or is moved forward to the
//! head when it is there already; then it is checked out.
//!
//! An address a provider gives becomes a remote only when it is a
//! repository at the account's site (or an SSH host name listed for it),
//! written plainly: never one git would hand to a helper (`ext::`) or
//! take for an option. A fork whose address is not is left out, and its
//! head comes by the pull request's reference alone.
//!
//! The local branch is named as the branch it comes from. A fork's branch
//! whose name a branch here has already (its `main`, often) is named
//! `<owner>/<branch>` instead; and when that is taken too, or nothing it
//! comes from can be reached and its name is taken, `pull-<number>`. A
//! branch that is there is taken as this pull request's only when it
//! follows the same branch, or (following nothing) is at its head. One
//! that has moved away from the head is checked out where it is, and the
//! answer says so.
//!
//! Every step is the engine's own command (`remote.add`, `remote.fetch`,
//! `branch.create`, `branch.set_upstream`, `branch.fast_forward`,
//! `remote.pull`, `branch.checkout`), so the journal has each and undo
//! takes each back.
//!
//! Sources: docs.github.com "Checking out pull requests locally"
//! (`refs/pull/N/head`); docs.gitlab.com "Merge requests: check out
//! locally" (`refs/merge-requests/N/head`); git-fetch(1), git-branch(1),
//! git-switch(1), git-remote(1).

use bitbraid_engine::{CommandContext, CommandError, Output, Params};
use bitbraid_git::branch;
use serde_json::json;

use crate::{
    accounts,
    api::ProviderError,
    cmds::{self, failed, invalid},
    model::{Account, HostedRepo, PullCheckout, PullHead, PullRequest},
    remotes,
};

/// What the provider said of the pull request, before anything here moves.
struct Asked {
    pull: PullRequest,
    /// Its fork, when it comes from one the account can see.
    fork: Option<HostedRepo>,
    /// The reference its head is fetched from.
    reference: String,
    /// The remote its repository is.
    remote: String,
    /// The account, and the SSH host names listed for its site.
    site: Site,
}

/// An account and the SSH host names its site is reached by besides its
/// own: where the addresses a provider gives have to be.
pub(crate) struct Site {
    account: Account,
    aliases: Vec<String>,
}

impl Site {
    /// The site of a signed account.
    pub(crate) fn of(signed: &cmds::Signed) -> Result<Self, CommandError> {
        let known = accounts::load(&signed.services.config_dir).map_err(failed)?;
        Ok(Self {
            aliases: known.ssh_hosts(&signed.account),
            account: signed.account.clone(),
        })
    }

    /// Whether an address a provider gave may become a remote and be
    /// fetched: a repository at this site, written plainly, nothing git
    /// would hand to a helper or take for an option.
    pub(crate) fn holds(&self, address: &str) -> bool {
        let plain = !address.starts_with('-')
            && !address.contains("::")
            && !address.chars().any(|c| c.is_control() || c.is_whitespace());
        plain
            && remotes::parse(address).is_some_and(|parsed| {
                remotes::same_site(&self.account, &parsed.host, &self.aliases)
            })
    }
}

/// Run an engine command on the open repository.
fn run(
    ctx: &mut CommandContext,
    id: &str,
    params: serde_json::Value,
) -> Result<Output, CommandError> {
    crate::shared().run(id, ctx, &params)
}

/// The remotes of the open repository: name and fetch address, as git
/// reaches it and as it is written in the repository's configuration
/// (they differ where `url.<base>.insteadOf` rewrites it).
fn remotes_here(ctx: &CommandContext) -> Result<Vec<(String, String)>, CommandError> {
    let repo = ctx.repo()?;
    let mut out: Vec<(String, String)> = bitbraid_git::snapshot(repo)?
        .remotes
        .into_iter()
        .filter_map(|remote| Some((remote.name, remote.fetch_url?)))
        .collect();
    let written = bitbraid_git::settings::local(repo).unwrap_or_default();
    for (key, value) in written {
        if let Some(name) = key
            .strip_prefix("remote.")
            .and_then(|rest| rest.strip_suffix(".url"))
            && !out
                .iter()
                .any(|(known, address)| known == name && *address == value)
        {
            out.push((name.to_owned(), value));
        }
    }
    Ok(out)
}

/// Whether two addresses name the same repository: the same host and
/// path, however written, or the same text.
fn same_address(one: &str, other: &str) -> bool {
    let trimmed = |text: &str| {
        text.trim()
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .to_owned()
    };
    match (remotes::parse(one), remotes::parse(other)) {
        (Some(one), Some(other)) => {
            one.host.eq_ignore_ascii_case(&other.host)
                && one.full_name().eq_ignore_ascii_case(&other.full_name())
        }
        _ => trimmed(one) == trimmed(other),
    }
}

/// Whether an address is reached over SSH.
fn over_ssh(address: &str) -> bool {
    let address = address.trim();
    address.starts_with("ssh://")
        || (!address.contains("://") && address.contains('@') && address.contains(':'))
}

/// A name for a fork's remote from its owner: what a remote's name may
/// hold, `-` for the rest.
fn remote_name(owner: &str) -> String {
    let name: String = owner
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(60)
        .collect();
    let name = name.trim_matches(|c| c == '.' || c == '-').to_owned();
    if name.is_empty() {
        "fork".to_owned()
    } else {
        name
    }
}

/// Ask the provider about the pull request and its fork.
fn ask(ctx: &CommandContext, params: &Params, number: u64) -> Result<Asked, CommandError> {
    let named = cmds::text_with(params, "remote", 100, false)?;
    let (signed, target) = cmds::on_repository(ctx, params)?;
    let here = remotes_here(ctx)?;
    let remote = match named {
        Some(name) => {
            if !here.iter().any(|(known, _)| *known == name) {
                return Err(invalid(
                    "remote",
                    say!("pr-no-remote", name = format!("{name:?}")),
                ));
            }
            name
        }
        None => {
            let known = accounts::load(&signed.services.config_dir).map_err(failed)?;
            cmds::links(ctx, &known)?
                .into_iter()
                .find(|link| {
                    link.account == signed.account.id
                        && link.owner.eq_ignore_ascii_case(&target.owner)
                        && link.name.eq_ignore_ascii_case(&target.name)
                })
                .map(|link| link.remote)
                .ok_or_else(|| {
                    invalid(
                        "remote",
                        say!("pr-no-linked-remote", repository = target.full_name()),
                    )
                })?
        }
    };
    let provider = signed.provider();
    let session = signed.session(ctx);
    let pull = provider.pr_get(&session, &target, number).map_err(failed)?;
    let fork = match pull
        .head_repo
        .as_deref()
        .and_then(|full| full.rsplit_once('/'))
    {
        Some((owner, name)) => {
            let at = crate::provider::Target {
                owner: owner.to_owned(),
                name: name.to_owned(),
            };
            match provider.repo_get(&session, &at) {
                Ok(fork) => Some(fork),
                // Gone, or not to be seen: its head comes all the same.
                Err(ProviderError::NotFound) => None,
                Err(err) => return Err(failed(err)),
            }
        }
        None => None,
    };
    Ok(Asked {
        reference: provider.pull_ref(number),
        pull,
        fork,
        remote,
        site: Site::of(&signed)?,
    })
}

/// The remote of a fork: one that is there already, or one added now.
/// Answers its name, and whether it was added; nothing when the fork's
/// addresses are not at the account's site.
pub(crate) fn fork_remote(
    ctx: &mut CommandContext,
    fork: &HostedRepo,
    base_address: &str,
    site: &Site,
) -> Result<Option<(String, bool)>, CommandError> {
    let here = remotes_here(ctx)?;
    let (first, second) = if over_ssh(base_address) {
        (&fork.clone_ssh, &fork.clone_https)
    } else {
        (&fork.clone_https, &fork.clone_ssh)
    };
    let Some(address) = [first, second]
        .into_iter()
        .find(|address| site.holds(address))
    else {
        return Ok(None);
    };
    if let Some((name, _)) = here
        .iter()
        .find(|(_, known)| same_address(known, first) || same_address(known, second))
    {
        return Ok(Some((name.clone(), false)));
    }
    let base = remote_name(&fork.owner);
    let name = std::iter::once(base.clone())
        .chain((1..10).map(|n| {
            if n == 1 {
                format!("{base}-fork")
            } else {
                format!("{base}-fork-{n}")
            }
        }))
        .find(|name| !here.iter().any(|(known, _)| known == name))
        .ok_or_else(|| invalid("remote", say!("pr-fork-names-taken", owner = base.as_str())))?;
    run(ctx, "remote.add", json!({ "name": name, "url": address }))?;
    Ok(Some((name, true)))
}

/// The commit a reference of the open repository names, if it names one.
fn tip(ctx: &CommandContext, reference: &str) -> Result<Option<String>, CommandError> {
    Ok(branch::commit_of(ctx.repo()?, reference)
        .ok()
        .map(|commit| commit.to_hex()))
}

/// Fetch a pull request's head without checking anything out: where a
/// worktree for it starts. The head comes by its reference from the pull
/// request's remote, as for a checkout; the branch it comes from is
/// fetched when a remote here is that branch's repository (the pull
/// request's own, or its fork's that is a remote already: none is added
/// for this), and is where the worktree starts when it is at the head;
/// else the commit is. For a fork's pull request that is never the branch
/// of the same name at the pull request's remote: the branch is the
/// fork's alone.
pub(crate) fn head(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    ctx.repo()?;
    let asked = ask(ctx, params, number)?;
    let branch = asked.pull.head_ref.clone();
    let here = remotes_here(ctx)?;
    let from = match (&asked.fork, &asked.pull.head_repo) {
        (Some(fork), _) => here
            .iter()
            .find(|(_, address)| {
                same_address(address, &fork.clone_https) || same_address(address, &fork.clone_ssh)
            })
            .map(|(name, _)| name.clone()),
        (None, None) => Some(asked.remote.clone()),
        (None, Some(_)) => None,
    };
    // The branch is a help, not a need: the head comes all the same when
    // its repository cannot be reached.
    let fetched_branch = match &from {
        Some(remote) => run(ctx, "remote.fetch", json!({ "remote": remote })).is_ok(),
        None => false,
    };
    let fetched = run(
        ctx,
        "remote.fetch",
        json!({ "remote": asked.remote, "reference": asked.reference }),
    )?;
    let commit = fetched
        .get("commit")
        .and_then(|commit| commit.as_str())
        .map(str::to_owned)
        .ok_or_else(|| invalid("number", say!("pr-head-missing")))?;
    let mut start = commit.clone();
    if let Some(remote) = from.as_ref().filter(|_| fetched_branch)
        && !branch.is_empty()
        && branch::name_problem(&branch).is_none()
    {
        let candidate = format!("{remote}/{branch}");
        if tip(ctx, &format!("refs/remotes/{candidate}"))?.as_deref() == Some(commit.as_str()) {
            start = candidate;
        }
    }
    cmds::answer(&PullHead {
        commit,
        branch,
        remote: from,
        start,
    })
}

/// Check a pull request out.
pub(crate) fn checkout(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let number = cmds::item_number(params)?;
    let dirty = cmds::choice(params, "dirty", &["refuse", "stash", "discard"], "refuse")?;
    ctx.repo()?;
    let asked = ask(ctx, params, number)?;
    let head = asked.pull.head_ref.clone();
    if head.is_empty() || branch::name_problem(&head).is_some() {
        return Err(failed(ProviderError::Malformed(say!(
            "pr-head-unusable",
            number,
            branch = format!("{head:?}")
        ))));
    }
    let base_address = remotes_here(ctx)?
        .into_iter()
        .find(|(name, _)| *name == asked.remote)
        .map(|(_, address)| address)
        .unwrap_or_default();
    // Where the branch it comes from is fetched from, when it can be.
    let mut remote_added = None;
    let mut note = None;
    let from = match (&asked.fork, &asked.pull.head_repo) {
        (Some(fork), _) => match fork_remote(ctx, fork, &base_address, &asked.site)? {
            Some((name, added)) => {
                if added {
                    remote_added = Some(name.clone());
                }
                Some(name)
            }
            None => {
                note = Some(say!("pr-fork-elsewhere", fork = fork.full_name.as_str()));
                None
            }
        },
        (None, None) => Some(asked.remote.clone()),
        (None, Some(_)) => None,
    };
    if let Some(remote) = &from {
        run(ctx, "remote.fetch", json!({ "remote": remote }))?;
    }
    let fetched = run(
        ctx,
        "remote.fetch",
        json!({ "remote": asked.remote, "reference": asked.reference }),
    )?;
    let commit = fetched
        .get("commit")
        .and_then(|commit| commit.as_str())
        .map(str::to_owned)
        .ok_or_else(|| invalid("number", say!("pr-head-missing")))?;
    // The branch it follows: the one it comes from, when that is there.
    let mut upstream = None;
    if let Some(remote) = &from {
        let candidate = format!("{remote}/{head}");
        let at = tip(ctx, &format!("refs/remotes/{candidate}"))?;
        // From the same repository, only when that branch is its head (a
        // branch of the same name may be another's once a fork is gone).
        let same = asked.fork.is_some() || at.as_deref() == Some(commit.as_str());
        if at.is_some() && same {
            upstream = Some(candidate);
        }
    }
    let owner = asked
        .pull
        .head_repo
        .as_deref()
        .and_then(|full| full.rsplit_once('/'))
        .map(|(owner, _)| remote_name(owner));
    let mut names = vec![head.clone()];
    if let Some(owner) = &owner {
        names.push(format!("{owner}/{head}"));
    }
    names.push(format!("pull-{number}"));
    let repo = ctx.repo()?;
    let mut chosen = None;
    for name in names {
        if branch::name_problem(&name).is_some() {
            continue;
        }
        match branch::branch_tip(repo, &name) {
            None => {
                chosen = Some((name, None));
                break;
            }
            Some(at) => {
                let follows = branch::upstream_of(repo, &name);
                let ours = match (&follows, &upstream) {
                    (Some(follows), Some(wanted)) => follows == wanted,
                    (None, _) => at.to_hex() == commit,
                    (Some(_), None) => false,
                };
                if ours {
                    chosen = Some((name, Some(at.to_hex())));
                    break;
                }
            }
        }
    }
    let (name, there) = chosen.ok_or_else(|| {
        invalid(
            "number",
            say!("pr-names-taken", head = head.as_str(), number),
        )
    })?;
    let current = branch::head_state(repo)?.branch;
    let mut out = PullCheckout {
        branch: name.clone(),
        commit: commit.clone(),
        created: false,
        updated: false,
        note,
        upstream: upstream.clone(),
        remote_added,
    };
    match there {
        None => {
            run(
                ctx,
                "branch.create",
                json!({ "name": name, "start": commit, "checkout": true, "dirty": dirty }),
            )?;
            out.created = true;
        }
        Some(at) => {
            let here = current.as_deref() == Some(name.as_str());
            if at != commit {
                let moved = if here {
                    match &upstream {
                        Some(_) if branch::upstream_of(ctx.repo()?, &name) == upstream => {
                            run(ctx, "remote.pull", json!({ "mode": "ff_only" }))
                        }
                        _ => Err(invalid("branch", say!("pr-nothing-to-pull"))),
                    }
                } else {
                    run(
                        ctx,
                        "branch.fast_forward",
                        json!({ "name": name, "to": commit }),
                    )
                };
                match moved {
                    Ok(_) => out.updated = true,
                    Err(err) => {
                        out.note = Some(say!(
                            "pr-left-behind",
                            name = name.as_str(),
                            error = err.to_string()
                        ));
                    }
                }
            }
            if !here {
                run(
                    ctx,
                    "branch.checkout",
                    json!({ "target": name, "dirty": dirty }),
                )?;
            }
        }
    }
    if let Some(wanted) = &upstream
        && branch::upstream_of(ctx.repo()?, &name).as_ref() != Some(wanted)
    {
        run(
            ctx,
            "branch.set_upstream",
            json!({ "name": name, "upstream": wanted }),
        )?;
    }
    cmds::answer(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_and_names_of_forks() {
        assert!(same_address(
            "git@github.com:octo/repo.git",
            "https://github.com/Octo/repo"
        ));
        assert!(!same_address(
            "git@github.com:octo/repo.git",
            "https://github.com/octo/other"
        ));
        assert!(same_address("/tmp/fork.git/", "/tmp/fork"));
        assert!(over_ssh("git@github.com:octo/repo.git"));
        assert!(over_ssh("ssh://git@host:2224/a/b.git"));
        assert!(!over_ssh("https://user@host/a/b.git"));
        assert_eq!(remote_name("group/sub"), "group-sub");
        assert_eq!(remote_name("../"), "fork");
        assert_eq!(remote_name("hubot"), "hubot");
    }
}
