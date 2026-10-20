//! Branch housekeeping: every local branch with what tells whether it is
//! still needed, a description for each, and the merged ones that can go.
//!
//! The overview is one pass over the local branches. Descriptions are not
//! something that pass can give, so they are read from the configuration
//! in one call, and what is merged is asked of git once for all of them.
//!
//! Sources: git-for-each-ref(1) (`committerdate`, `upstream`, `HEAD`,
//! `worktreepath`, `--merged`), git-branch(1) (`--merged`,
//! `--edit-description`, `-d`, `-D`), git-config(1)
//! (`branch.<name>.description`, `--get-regexp`, `-z`, `--unset-all`),
//! git-worktree(1).

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::{
    Error, Oid, Repository, Result,
    branch::{self, DeletedBranch},
    exe, flow,
};

/// After how many days without a commit a branch counts as stale, unless
/// told otherwise.
pub const DEFAULT_STALE_DAYS: u32 = 90;
/// Longest name of a branch that is described or compared with.
const MAX_NAME: usize = 200;
/// Longest description: it is handed to git as one argument.
const MAX_DESCRIPTION: usize = 64 * 1024;
const DAY: i64 = 86_400;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Refused {
        what,
        detail: detail.into(),
    }
}

/// A local branch, with what tells whether it is still needed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BranchInfo {
    /// Short name.
    pub name: String,
    /// The commit it points at.
    pub tip: Option<Oid>,
    /// `branch.<name>.description`, when there is one.
    pub description: Option<String>,
    /// When its last commit was made, in unix seconds.
    pub time: Option<i64>,
    /// That is longer ago than the days asked about.
    pub stale: bool,
    /// Its upstream as configured, `remote/branch`.
    pub upstream: Option<String>,
    /// The upstream it is configured with is not there any more.
    pub gone: bool,
    /// Everything it has is in the branch it is compared with.
    pub merged: bool,
    /// It is the one checked out here.
    pub current: bool,
    /// Another worktree that has it checked out, by its path.
    pub worktree: Option<String>,
}

/// The local branches, looked over.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Overview {
    /// The branch the others are compared with, when there is one.
    pub base: Option<String>,
    /// After how many days a branch counts as stale.
    pub stale_days: u32,
    /// The time that was counted from, in unix seconds.
    pub now: i64,
    /// Every local branch, by name.
    pub branches: Vec<BranchInfo>,
}

/// A local branch as one pass over them gives it.
struct Listed {
    name: String,
    tip: Option<Oid>,
    time: Option<i64>,
    /// The full name of the upstream reference, there or not.
    upstream: Option<String>,
    current: bool,
    worktree: Option<String>,
}

fn listing(repo: &Repository) -> Result<Vec<Listed>> {
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            "--format=%(refname)%00%(objectname)%00%(committerdate:unix)%00%(upstream)%00%(HEAD)%00%(worktreepath)%00",
            "refs/heads",
        ],
        None,
    )?;
    // Every field ends in a NUL, so a path with a line break in it stays
    // one field; the break between records leads the next one's name.
    let fields: Vec<&str> = out.stdout.split('\0').collect();
    Ok(fields
        .as_chunks::<6>()
        .0
        .iter()
        .filter_map(|[name, tip, time, upstream, head, path]| {
            let name = name.trim_start_matches('\n').strip_prefix("refs/heads/")?;
            let current = *head == "*";
            Some(Listed {
                name: name.to_owned(),
                tip: tip.parse().ok(),
                time: time.trim().parse().ok(),
                upstream: (!upstream.is_empty()).then(|| (*upstream).to_owned()),
                current,
                worktree: (!current && !path.is_empty()).then(|| (*path).to_owned()),
            })
        })
        .collect())
}

/// The full names of the upstream references that are not there.
fn gone_upstreams(repo: &Repository, listed: &[Listed]) -> Result<HashSet<String>> {
    let wanted: HashSet<&str> = listed
        .iter()
        .filter_map(|branch| branch.upstream.as_deref())
        .collect();
    if wanted.is_empty() {
        return Ok(HashSet::new());
    }
    // Names only: nothing is read from the object database for this.
    let out = exe::run(repo, ["for-each-ref", "--format=%(refname)"], None)?;
    let there: HashSet<&str> = out.stdout.lines().collect();
    Ok(wanted
        .into_iter()
        .filter(|name| !there.contains(name))
        .map(str::to_owned)
        .collect())
}

/// The local branches that have nothing `base` lacks.
fn merged_into(repo: &Repository, base: Oid) -> Result<HashSet<String>> {
    let option = format!("--merged={}", base.to_hex());
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            option.as_str(),
            "--format=%(refname)",
            "refs/heads",
        ],
        None,
    )?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|name| name.strip_prefix("refs/heads/"))
        .map(str::to_owned)
        .collect())
}

/// Every branch description there is, by branch.
fn descriptions(repo: &Repository) -> Result<HashMap<String, String>> {
    Ok(flow::config_matching(repo, r"^branch\..*\.description$")?
        .into_iter()
        .filter_map(|(key, value)| {
            let name = key.strip_prefix("branch.")?.strip_suffix(".description")?;
            Some((name.to_owned(), value))
        })
        .collect())
}

/// `remote/branch` out of the full name of an upstream reference.
fn short(upstream: &str) -> String {
    upstream
        .strip_prefix("refs/remotes/")
        .or_else(|| upstream.strip_prefix("refs/heads/"))
        .unwrap_or(upstream)
        .to_owned()
}

/// What the branches are compared with.
struct Base {
    name: String,
    tip: Oid,
    /// A local branch, as opposed to one of a remote.
    local: bool,
}

/// The branch to compare with: the one given, a local branch or a remote
/// one as `remote/branch`; else gitflow's main branch, `main`, `master`
/// or the current branch, whichever is there first.
fn base_of(repo: &Repository, given: Option<&str>, listed: &[Listed]) -> Result<Option<Base>> {
    let local = |name: &str| {
        listed
            .iter()
            .find(|branch| branch.name == name)
            .and_then(|branch| branch.tip)
    };
    if let Some(name) = given {
        if name.len() > MAX_NAME {
            return Err(invalid(
                say!("housekeeping-what-base"),
                say!("housekeeping-too-long"),
            ));
        }
        // Either way it is a name under `refs`, and checked as one.
        branch::check_branch_name(repo, name)?;
        if let Some(tip) = local(name) {
            return Ok(Some(Base {
                name: name.to_owned(),
                tip,
                local: true,
            }));
        }
        let reference = format!("refs/remotes/{name}^{{commit}}");
        let remote: Option<Oid> = exe::run(
            repo,
            ["rev-parse", "--verify", "--quiet", reference.as_str()],
            None,
        )
        .ok()
        .and_then(|out| out.stdout.trim().parse().ok());
        return match remote {
            Some(tip) => Ok(Some(Base {
                name: name.to_owned(),
                tip,
                local: false,
            })),
            None => Err(invalid(
                say!("housekeeping-what-base"),
                say!("housekeeping-base-unknown", name = format!("{name:?}")),
            )),
        };
    }
    let (flow_main, _) = flow::configured_branches(repo)?;
    let current = listed
        .iter()
        .find(|branch| branch.current)
        .map(|branch| branch.name.clone());
    Ok([
        flow_main,
        Some("main".to_owned()),
        Some("master".to_owned()),
        current,
    ]
    .into_iter()
    .flatten()
    .find_map(|name| {
        local(&name).map(|tip| Base {
            name,
            tip,
            local: true,
        })
    }))
}

fn clock() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|since| i64::try_from(since.as_secs()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

/// Look over the local branches. `base` is what "merged" is asked about
/// (see [`base_of`] for when it is absent), `stale_days` how long without
/// a commit makes a branch stale, counted back from `now` (the clock when
/// absent).
pub fn overview(
    repo: &Repository,
    base: Option<&str>,
    stale_days: u32,
    now: Option<i64>,
) -> Result<Overview> {
    let listed = listing(repo)?;
    let base = base_of(repo, base, &listed)?;
    let merged = match &base {
        Some(base) => merged_into(repo, base.tip)?,
        None => HashSet::new(),
    };
    let gone = gone_upstreams(repo, &listed)?;
    let mut described = descriptions(repo)?;
    let now = now.unwrap_or_else(clock);
    let stale_before = now.saturating_sub(i64::from(stale_days).saturating_mul(DAY));
    let branches = listed
        .into_iter()
        .map(|branch| BranchInfo {
            description: described
                .remove(&branch.name)
                .filter(|text| !text.trim().is_empty()),
            stale: branch.time.is_some_and(|time| time < stale_before),
            gone: branch
                .upstream
                .as_ref()
                .is_some_and(|upstream| gone.contains(upstream)),
            upstream: branch.upstream.as_deref().map(short),
            merged: merged.contains(&branch.name),
            name: branch.name,
            tip: branch.tip,
            time: branch.time,
            current: branch.current,
            worktree: branch.worktree,
        })
        .collect();
    Ok(Overview {
        base: base.map(|base| base.name),
        stale_days,
        now,
        branches,
    })
}

/// What describing a branch changed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Described {
    /// The description before.
    pub before: Option<String>,
    /// The description now.
    pub after: Option<String>,
}

/// The description of a branch exactly as it is stored.
fn description_of(repo: &Repository, key: &str) -> Result<Option<String>> {
    let (code, out) = exe::run_with_code(repo, ["config", "-z", "--get", key], None)?;
    Ok((code == Some(0)).then(|| {
        out.stdout
            .strip_suffix('\0')
            .unwrap_or(&out.stdout)
            .to_owned()
    }))
}

/// Set the description of a local branch, or remove it with a text that
/// is empty or blank. Any other text is stored as it is given, line
/// breaks included.
pub fn describe(repo: &Repository, name: &str, description: &str) -> Result<Described> {
    let what = say!("housekeeping-what-describe");
    if name.len() > MAX_NAME {
        return Err(invalid(
            say!("housekeeping-what-branch-name"),
            say!("housekeeping-too-long"),
        ));
    }
    branch::check_branch_name(repo, name)?;
    if branch::branch_tip(repo, name).is_none() {
        return Err(refused(what, say!("housekeeping-no-branch", name)));
    }
    if description.len() > MAX_DESCRIPTION {
        return Err(invalid(
            say!("housekeeping-what-description"),
            say!("housekeeping-too-long"),
        ));
    }
    // Text of several lines, and nothing a terminal would act on.
    if description
        .chars()
        .any(|ch| ch.is_control() && !matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(invalid(
            say!("housekeeping-what-description"),
            say!("housekeeping-description-control"),
        ));
    }
    let key = format!("branch.{name}.description");
    let before = description_of(repo, &key)?;
    if description.trim().is_empty() {
        if before.is_some() {
            let (code, out) = exe::run_with_code(
                repo,
                ["config", "--local", "--unset-all", "--", key.as_str()],
                None,
            )?;
            // 5: it is not set in this repository's own configuration.
            if !matches!(code, Some(0 | 5)) {
                return Err(refused(what, out.stderr.trim().to_owned()));
            }
        }
    } else {
        exe::run(
            repo,
            [
                "config",
                "--local",
                "--replace-all",
                "--",
                key.as_str(),
                description,
            ],
            None,
        )?;
    }
    Ok(Described {
        before,
        after: description_of(repo, &key)?,
    })
}

/// Why a branch is not deleted with the merged ones.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Keep {
    /// It is the branch the others are compared with, or the local branch
    /// of the remote one they are compared with.
    Base,
    /// It is checked out here.
    Current,
    /// It is checked out in another worktree.
    Worktree,
    /// It is on the list of protected branches.
    Protected,
    /// It is gitflow's main or develop branch.
    Flow,
    /// It has commits the base lacks.
    Unmerged,
    /// Git would not delete it.
    Failed,
}

/// A branch that stays, and why.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Kept {
    /// Short name.
    pub name: String,
    /// Why it stays.
    pub reason: Keep,
    /// More about the reason: the other worktree's path, what git said.
    pub detail: Option<String>,
    /// The upstream it is configured with is not there any more.
    pub gone: bool,
}

/// A merged branch that can go.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Mergeable {
    /// Short name.
    pub name: String,
    /// The commit it points at.
    pub target: Oid,
    /// Its upstream as configured, `remote/branch`.
    pub upstream: Option<String>,
    /// That upstream is not there any more.
    pub gone: bool,
}

/// Which local branches deleting the merged ones would take, and why each
/// of the others stays.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MergedPlan {
    /// The branch the others are compared with.
    pub base: String,
    /// The ones to delete.
    pub delete: Vec<Mergeable>,
    /// The ones that stay.
    pub kept: Vec<Kept>,
}

/// Sort the local branches into those that are merged into `base` and can
/// go, and those that stay: the base itself, the current branch, one that
/// another worktree has checked out, the protected ones, gitflow's two
/// branches, and whatever is not merged. Nothing is changed.
pub fn merged_plan(
    repo: &Repository,
    base: Option<&str>,
    protected: &[String],
) -> Result<MergedPlan> {
    let listed = listing(repo)?;
    let base = base_of(repo, base, &listed)?.ok_or_else(|| {
        refused(
            say!("housekeeping-what-delete-merged"),
            say!("housekeeping-no-base"),
        )
    })?;
    let merged = merged_into(repo, base.tip)?;
    let gone = gone_upstreams(repo, &listed)?;
    let (flow_main, flow_develop) = flow::configured_branches(repo)?;
    let protected: HashSet<&str> = protected.iter().map(String::as_str).collect();
    // The local side of a remote base: the branch that follows it, and
    // the one that has its name.
    let base_upstream = format!("refs/remotes/{}", base.name);
    let is_base = |branch: &Listed| {
        if base.local {
            branch.name == base.name
        } else {
            branch.upstream.as_deref() == Some(base_upstream.as_str())
                || base
                    .name
                    .strip_suffix(branch.name.as_str())
                    .is_some_and(|remote| remote.ends_with('/'))
        }
    };
    let mut plan = MergedPlan {
        base: base.name.clone(),
        delete: Vec::new(),
        kept: Vec::new(),
    };
    for branch in listed {
        let gone = branch
            .upstream
            .as_ref()
            .is_some_and(|upstream| gone.contains(upstream));
        let of_flow = [&flow_main, &flow_develop]
            .into_iter()
            .any(|name| name.as_deref() == Some(branch.name.as_str()));
        let reason = if is_base(&branch) {
            Some(Keep::Base)
        } else if branch.current {
            Some(Keep::Current)
        } else if branch.worktree.is_some() {
            Some(Keep::Worktree)
        } else if protected.contains(branch.name.as_str()) {
            Some(Keep::Protected)
        } else if of_flow {
            Some(Keep::Flow)
        } else if !merged.contains(&branch.name) {
            Some(Keep::Unmerged)
        } else {
            None
        };
        match (reason, branch.tip) {
            (None, Some(target)) => plan.delete.push(Mergeable {
                name: branch.name,
                target,
                upstream: branch.upstream.as_deref().map(short),
                gone,
            }),
            (reason, _) => plan.kept.push(Kept {
                name: branch.name,
                reason: reason.unwrap_or(Keep::Unmerged),
                detail: branch.worktree.filter(|_| reason == Some(Keep::Worktree)),
                gone,
            }),
        }
    }
    Ok(plan)
}

/// Delete one of the branches [`merged_plan`] found, if it is still where
/// it was found. Git's own check is against the upstream or HEAD, which
/// is another question than the one that was asked, so it is deleted
/// without it.
pub fn delete_merged(repo: &Repository, branch: &Mergeable) -> Result<DeletedBranch> {
    branch::check_branch_name(repo, &branch.name)?;
    if branch::branch_tip(repo, &branch.name) != Some(branch.target) {
        return Err(refused(
            say!("housekeeping-what-delete-merged"),
            say!("housekeeping-moved", name = branch.name.as_str()),
        ));
    }
    branch::branch_delete(repo, &branch.name, true)
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn an_upstream_is_shown_without_its_namespace() {
        assert_eq!(short("refs/remotes/origin/topic"), "origin/topic");
        assert_eq!(short("refs/heads/main"), "main");
        assert_eq!(short("refs/tags/v1"), "refs/tags/v1");
        assert_eq!(short(""), "");
    }
}
