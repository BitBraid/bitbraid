//! The inbox at GitLab: the user's open merge requests and issues across
//! every project the account sees. The merge requests are three lists of
//! all projects (`scope=all`): those the user opened, those assigned to
//! them and those they are asked to review; each project's then get
//! their reviewers' verdicts, pipelines and lines added and removed from
//! GraphQL, as the list of merge requests does, for at most
//! [`MOST_ENRICHED`] projects. The issues are two lists, those assigned
//! to the user and those they opened, and the user's pending to-do items
//! that say an issue names them.
//!
//! Whether a merge request merges, and what its approvals add up to, is
//! read from its `detailed_merge_status` and `has_conflicts`.
//!
//! Sources: docs.gitlab.com/api "Merge requests" (list merge requests:
//! `scope`, `state`, `author_username`, `assignee_username`,
//! `reviewer_username`, `with_labels_details`; `references`,
//! `detailed_merge_status` and its values, `has_conflicts`), "Issues"
//! (list issues: `scope`, `assignee_username`, `author_username`;
//! `upvotes`, `downvotes`, `references`), "To-do items" (`type`,
//! `state`, `action_name`, `target`, `project`); docs.gitlab.com/api/graphql
//! "Reference" (`MergeRequest.diffStatsSummary`).

use std::collections::BTreeMap;

use serde::Deserialize;

use super::{
    WireMilestone, fresh,
    issues::{WireIssue, issue},
    milestone,
    pulls::{
        ENRICH_WITH_LINES, WireMerge, enrich_with, enriched, fork_of, fork_paths, merge_request,
    },
};
use crate::{
    api::{self, ProviderError},
    inbox::{Approval, Involvement, PullFacts},
    model::{Issue, IssueState, Mergeable},
    provider::{Found, FoundIssue, FoundPull, InboxQuery, Session, Target},
    remotes,
};

/// Most of each list read: one page.
const MOST_PER_LIST: usize = 100;
/// Most projects whose merge requests are asked of GraphQL.
pub(crate) const MOST_ENRICHED: usize = 30;

#[derive(Debug, Deserialize)]
struct WireReferences {
    #[serde(default)]
    full: Option<String>,
}

/// A merge request of a list of all projects.
#[derive(Debug, Deserialize)]
struct InboxMerge {
    #[serde(flatten)]
    merge: WireMerge,
    #[serde(default)]
    references: Option<WireReferences>,
    #[serde(default)]
    detailed_merge_status: Option<String>,
    #[serde(default)]
    has_conflicts: bool,
    #[serde(default)]
    milestone: Option<WireMilestone>,
}

/// An issue of a list of all projects.
#[derive(Debug, Deserialize)]
struct InboxIssue {
    #[serde(flatten)]
    issue: WireIssue,
    #[serde(default)]
    references: Option<WireReferences>,
    #[serde(default)]
    upvotes: Option<u64>,
    #[serde(default)]
    downvotes: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct WireTodoProject {
    #[serde(default)]
    path_with_namespace: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireTodo {
    #[serde(default)]
    action_name: Option<String>,
    #[serde(default)]
    project: Option<WireTodoProject>,
    #[serde(default)]
    target: Option<InboxIssue>,
}

/// A project's path from its item's full reference
/// (`group/sub/project!12`, `group/project#3`), held to what GitLab
/// allows in a path.
fn project_of(reference: Option<&str>) -> Option<Target> {
    let reference = reference?;
    let path = reference
        .split(['!', '#'])
        .next()
        .filter(|path| !path.is_empty())?;
    let (owner, name) = path.rsplit_once('/')?;
    let plain = owner.split('/').count() <= 20
        && owner.split('/').all(remotes::path_part)
        && remotes::path_part(name);
    plain.then(|| Target {
        owner: owner.to_owned(),
        name: name.to_owned(),
    })
}

/// Whether it merges, and what its approvals add up to, from GitLab's
/// word for it.
fn merge_state(detailed: Option<&str>, conflicts: bool) -> (Mergeable, Approval) {
    if conflicts {
        return (Mergeable::Conflicts, Approval::Unknown);
    }
    match detailed.unwrap_or_default() {
        "mergeable" => (Mergeable::Clean, Approval::Unknown),
        "conflict" => (Mergeable::Conflicts, Approval::Unknown),
        "not_approved" => (Mergeable::Blocked, Approval::Required),
        "requested_changes" => (Mergeable::Blocked, Approval::ChangesRequested),
        "checking" | "unchecked" | "approvals_syncing" | "preparing" | "" => {
            (Mergeable::Unknown, Approval::Unknown)
        }
        // Everything else holds it back: checks that must pass or still
        // run, threads to resolve, a draft, a rebase it needs, rules of
        // the project.
        _ => (Mergeable::Blocked, Approval::Unknown),
    }
}

/// The user's open merge requests and issues.
pub(super) fn inbox(session: &Session<'_>, query: &InboxQuery) -> Result<Found, ProviderError> {
    let api = session.api()?;
    let login = api::encode(&session.account.login);
    let most = query.most.clamp(1, MOST_PER_LIST);
    let mut found = Found::default();
    // The merge requests, each once, with why.
    let mut merges: Vec<(InboxMerge, Target, Vec<Involvement>)> = Vec::new();
    for (key, why) in [
        ("author_username", Involvement::Created),
        ("assignee_username", Involvement::Assigned),
        ("reviewer_username", Involvement::ReviewRequested),
    ] {
        let path = format!(
            "/merge_requests?scope=all&state=opened&{key}={login}&with_labels_details=true&per_page={most}"
        );
        let (page, more, response) = api.pages::<InboxMerge>(&path, most)?;
        found.more |= more;
        found.freshness = api::latest(found.freshness, fresh(session, &response));
        for wire in page {
            let Some(target) = project_of(
                wire.references
                    .as_ref()
                    .and_then(|refs| refs.full.as_deref()),
            ) else {
                continue;
            };
            let number = wire.merge.number();
            match merges.iter_mut().find(|(known, at, _)| {
                at.full_name().eq_ignore_ascii_case(&target.full_name())
                    && known.merge.number() == number
            }) {
                Some((_, _, whys)) => {
                    if !whys.contains(&why) {
                        whys.push(why);
                    }
                }
                None => merges.push((wire, target, vec![why])),
            }
        }
    }
    // What GraphQL adds, a project at a time.
    let mut by_project: BTreeMap<String, (Target, Vec<u64>)> = BTreeMap::new();
    for (wire, target, _) in &merges {
        let number = wire.merge.number();
        by_project
            .entry(target.full_name().to_ascii_lowercase())
            .or_insert_with(|| (target.clone(), Vec::new()))
            .1
            .push(number);
    }
    let mut extra = BTreeMap::new();
    for (project, (target, numbers)) in by_project.into_iter().take(MOST_ENRICHED) {
        match enrich_with(session, &target, &numbers, ENRICH_WITH_LINES) {
            Ok(told) => {
                extra.insert(project, told);
            }
            Err(err @ ProviderError::Http(crate::http::FetchError::Cancelled)) => return Err(err),
            // An older GitLab, or one that refuses the question: those
            // rows without verdicts, pipelines and lines.
            Err(_) => {}
        }
    }
    // The forks' paths, as GraphQL told them or asked one by one.
    let mut forks: BTreeMap<u64, String> = BTreeMap::new();
    for (wire, target, _) in &merges {
        let number = wire.merge.number();
        if let (Some(source), Some(path)) = (
            fork_of(&wire.merge),
            extra
                .get(&target.full_name().to_ascii_lowercase())
                .and_then(|told| told.get(&number))
                .and_then(|merge| merge.source_path()),
        ) {
            forks.insert(source, path.to_owned());
        }
    }
    let missing: Vec<&WireMerge> = merges
        .iter()
        .map(|(wire, _, _)| &wire.merge)
        .filter(|wire| fork_of(wire).is_some_and(|source| !forks.contains_key(&source)))
        .collect();
    if !missing.is_empty() {
        forks.extend(fork_paths(session, &missing)?);
    }
    for (wire, target, involvement) in &merges {
        let row = merge_request(&wire.merge, target, &forks);
        let told = extra
            .get(&target.full_name().to_ascii_lowercase())
            .and_then(|told| told.get(&row.number));
        let (additions, deletions) = told.map_or((None, None), |merge| merge.lines());
        let (mergeable, approval) =
            merge_state(wire.detailed_merge_status.as_deref(), wire.has_conflicts);
        found.pulls.push(FoundPull {
            pull: enriched(row, told),
            facts: PullFacts {
                additions,
                deletions,
                mergeable,
                approval,
                milestone: wire.milestone.as_ref().map(milestone),
            },
            involvement: involvement.clone(),
        });
    }
    // The issues.
    let put = |made: Issue, wire: &InboxIssue, why: Involvement, found: &mut Found| match found
        .issues
        .iter_mut()
        .find(|known| {
            known.issue.number == made.number && known.issue.repo.eq_ignore_ascii_case(&made.repo)
        }) {
        Some(known) => {
            if !known.involvement.contains(&why) {
                known.involvement.push(why);
            }
        }
        None => found.issues.push(FoundIssue {
            issue: made,
            reactions: match (wire.upvotes, wire.downvotes) {
                (None, None) => None,
                (up, down) => Some(up.unwrap_or(0).saturating_add(down.unwrap_or(0))),
            },
            involvement: vec![why],
        }),
    };
    for (key, why) in [
        ("assignee_username", Involvement::Assigned),
        ("author_username", Involvement::Created),
    ] {
        let path = format!(
            "/issues?scope=all&state=opened&{key}={login}&with_labels_details=true&per_page={most}"
        );
        let (page, more, response) = api.pages::<InboxIssue>(&path, most)?;
        found.more |= more;
        found.freshness = api::latest(found.freshness, fresh(session, &response));
        for wire in &page {
            if let Some(target) = project_of(
                wire.references
                    .as_ref()
                    .and_then(|refs| refs.full.as_deref()),
            ) {
                put(issue(&wire.issue, &target), wire, why, &mut found);
            }
        }
    }
    let path = format!("/todos?type=Issue&state=pending&per_page={most}");
    let (todos, more, response) = api.pages::<WireTodo>(&path, most)?;
    found.more |= more;
    found.freshness = api::latest(found.freshness, fresh(session, &response));
    for todo in &todos {
        if !matches!(
            todo.action_name.as_deref(),
            Some("mentioned" | "directly_addressed")
        ) {
            continue;
        }
        let Some(wire) = &todo.target else {
            continue;
        };
        let named = todo
            .project
            .as_ref()
            .and_then(|project| project.path_with_namespace.as_deref());
        let target = project_of(named).or_else(|| {
            project_of(
                wire.references
                    .as_ref()
                    .and_then(|refs| refs.full.as_deref()),
            )
        });
        let Some(target) = target else {
            continue;
        };
        let made = issue(&wire.issue, &target);
        // A to-do item may outlive its issue.
        if made.state == IssueState::Open {
            put(made, wire, Involvement::Mentioned, &mut found);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_is_read_from_a_reference() {
        let of = |text: &str| project_of(Some(text)).map(|target| target.full_name());
        assert_eq!(of("group/project!12"), Some("group/project".to_owned()));
        assert_eq!(of("g/s/project#3"), Some("g/s/project".to_owned()));
        assert_eq!(of("g/s/project"), Some("g/s/project".to_owned()));
        for bad in ["!12", "project!1", "g/../p!1", "g/p q!1", "", "/p!1"] {
            assert_eq!(of(bad), None, "{bad:?}");
        }
        assert_eq!(project_of(None), None);
        let deep = format!("{}p!1", "g/".repeat(21));
        assert_eq!(of(&deep), None);
    }

    #[test]
    fn what_holds_a_merge_request_back_is_read_from_gitlabs_word() {
        assert_eq!(
            merge_state(Some("mergeable"), false),
            (Mergeable::Clean, Approval::Unknown)
        );
        assert_eq!(merge_state(Some("mergeable"), true).0, Mergeable::Conflicts);
        assert_eq!(merge_state(Some("conflict"), false).0, Mergeable::Conflicts);
        assert_eq!(
            merge_state(Some("not_approved"), false),
            (Mergeable::Blocked, Approval::Required)
        );
        assert_eq!(
            merge_state(Some("requested_changes"), false),
            (Mergeable::Blocked, Approval::ChangesRequested)
        );
        for blocked in [
            "ci_must_pass",
            "discussions_not_resolved",
            "draft_status",
            "x",
        ] {
            assert_eq!(merge_state(Some(blocked), false).0, Mergeable::Blocked);
        }
        for unknown in ["checking", "unchecked"] {
            assert_eq!(merge_state(Some(unknown), false).0, Mergeable::Unknown);
        }
        assert_eq!(merge_state(None, false).0, Mergeable::Unknown);
    }
}
