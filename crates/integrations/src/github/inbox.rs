//! The inbox at GitHub: the user's open pull requests and issues across
//! every repository the account sees, in one GraphQL question of five
//! searches: the pull requests that involve the user (opened, assigned,
//! naming or commented on by them), those that ask for their review, and
//! the issues assigned to them, opened by them and naming them. A pull
//! request's row is the one of the list of pull requests with its lines
//! added and removed, whether it merges, what its reviews add up to and
//! its milestone.
//!
//! When the inbox is about a workspace, the searches are held to its
//! repositories with `repo:` (at most [`MOST_SCOPED`] of them; past that,
//! or when none is a path GitHub has, every repository is searched and
//! the command leaves the others out).
//!
//! Sources: docs.github.com/en/graphql "Queries" (`search` with
//! `type: ISSUE`), "Objects" (`PullRequest`: `additions`, `deletions`,
//! `mergeable`, `reviewDecision`, `milestone`; `Issue`; `Milestone`;
//! `ReactionConnection`), "Enums" (`MergeableState`,
//! `PullRequestReviewDecision`, `MilestoneState`, `IssueState`);
//! docs.github.com "Searching issues and pull requests" (`involves:`,
//! `review-requested:`, `assignee:`, `author:`, `mentions:`,
//! `archived:`, `repo:`, `sort:`), "Understanding the search syntax" (a
//! query's length).

use serde::Deserialize;
use serde_json::json;

use super::{
    GraphUser, WireLabel,
    checks::WireNodes,
    fresh, label,
    pulls::{GraphPull, ROW, graph_pull, items},
};
use crate::{
    api::{self, ProviderError},
    inbox::{Approval, Involvement, PullFacts},
    model::{Issue, IssueState, Mergeable, Milestone},
    provider::{Found, FoundIssue, FoundPull, InboxQuery, Session, Target},
    remotes,
};

/// Most repositories a search is held to with `repo:`.
pub(crate) const MOST_SCOPED: usize = 30;
/// Most of each search read: one page.
const MOST_PER_SEARCH: usize = 50;

/// What an issue's row asks.
const ISSUE_ROW: &str = "number title state url createdAt updatedAt \
author { login avatarUrl url ... on User { name } } \
repository { nameWithOwner } \
assignees(first: 20) { nodes { login name avatarUrl url } } \
labels(first: 20) { nodes { name color description } } \
milestone { number title state dueOn } \
comments { totalCount } reactions { totalCount }";

#[derive(Debug, Deserialize)]
struct GraphMilestone {
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default, rename = "dueOn")]
    due: Option<String>,
}

fn milestone(wire: &GraphMilestone) -> Option<Milestone> {
    Some(Milestone {
        id: wire.number?.to_string(),
        title: api::title(wire.title.as_deref()?),
        due: wire.due.as_deref().map(|due| api::clean_line(due, 64)),
        open: wire.state.as_deref() != Some("CLOSED"),
    })
}

#[derive(Debug, Deserialize)]
struct GraphCount {
    #[serde(default, rename = "totalCount")]
    total: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GraphName {
    #[serde(default, rename = "nameWithOwner")]
    name_with_owner: Option<String>,
}

/// A pull request as the inbox's search answers it.
#[derive(Debug, Deserialize)]
struct InboxPull {
    #[serde(flatten)]
    row: GraphPull,
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
    #[serde(default)]
    mergeable: Option<String>,
    #[serde(default, rename = "reviewDecision")]
    review_decision: Option<String>,
    #[serde(default)]
    milestone: Option<GraphMilestone>,
}

/// An issue as the inbox's search answers it; a pull request found by
/// an issue search is all `None` and left out.
#[derive(Debug, Deserialize)]
struct InboxIssue {
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "createdAt")]
    created: Option<String>,
    #[serde(default, rename = "updatedAt")]
    updated: Option<String>,
    #[serde(default)]
    author: Option<GraphUser>,
    #[serde(default)]
    repository: Option<GraphName>,
    #[serde(default)]
    assignees: Option<WireNodes<GraphUser>>,
    #[serde(default)]
    labels: Option<WireNodes<WireLabel>>,
    #[serde(default)]
    milestone: Option<GraphMilestone>,
    #[serde(default)]
    comments: Option<GraphCount>,
    #[serde(default)]
    reactions: Option<GraphCount>,
}

#[derive(Debug, Default, Deserialize)]
struct PageInfo {
    #[serde(default, rename = "hasNextPage")]
    next: bool,
}

#[derive(Debug, Deserialize)]
struct Searched<T> {
    #[serde(default, rename = "pageInfo")]
    page: PageInfo,
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

impl<T> Default for Searched<T> {
    fn default() -> Self {
        Self {
            page: PageInfo::default(),
            nodes: Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct Data {
    #[serde(default)]
    pulls: Option<Searched<InboxPull>>,
    #[serde(default)]
    review: Option<Searched<InboxPull>>,
    #[serde(default)]
    assigned: Option<Searched<InboxIssue>>,
    #[serde(default)]
    created: Option<Searched<InboxIssue>>,
    #[serde(default)]
    mentioned: Option<Searched<InboxIssue>>,
}

/// Whether a login can be put into a search as it is: GitHub's logins
/// are letters, digits and hyphens (an Enterprise Server's may have
/// underscores).
fn plain_login(login: &str) -> bool {
    !login.is_empty()
        && login.len() <= 100
        && login
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

/// The `repo:` words of a workspace's repositories, when there are few
/// enough to name and every one is a path GitHub has.
fn scoped(query: &InboxQuery) -> String {
    let Some(repos) = &query.repos else {
        return String::new();
    };
    if repos.len() > MOST_SCOPED {
        return String::new();
    }
    let plain = |repo: &&String| {
        repo.split_once('/').is_some_and(|(owner, name)| {
            remotes::path_part(owner) && remotes::path_part(name) && !name.contains('/')
        })
    };
    let words: Vec<String> = repos
        .iter()
        .filter(plain)
        .map(|repo| format!(" repo:{repo}"))
        .collect();
    // None of them a path GitHub has: every repository is searched,
    // and the command keeps the workspace's.
    words.concat()
}

fn approval(decision: Option<&str>) -> Approval {
    match decision {
        Some("APPROVED") => Approval::Approved,
        Some("CHANGES_REQUESTED") => Approval::ChangesRequested,
        Some("REVIEW_REQUIRED") => Approval::Required,
        _ => Approval::Unknown,
    }
}

fn mergeable(state: Option<&str>) -> Mergeable {
    match state {
        Some("MERGEABLE") => Mergeable::Clean,
        Some("CONFLICTING") => Mergeable::Conflicts,
        _ => Mergeable::Unknown,
    }
}

/// A repository's `owner/name` as it may be used: two plain parts.
fn repository(name: &str) -> Option<String> {
    let (owner, rest) = name.split_once('/')?;
    (remotes::path_part(owner) && remotes::path_part(rest)).then(|| api::clean_line(name, 512))
}

fn pull(wire: &InboxPull, me: &str, review_asked: bool) -> Option<FoundPull> {
    let pull = graph_pull(
        &wire.row,
        &Target {
            owner: String::new(),
            name: String::new(),
        },
    )?;
    repository(&pull.repo)?;
    let facts = PullFacts {
        additions: wire.additions,
        deletions: wire.deletions,
        mergeable: mergeable(wire.mergeable.as_deref()),
        approval: approval(wire.review_decision.as_deref()),
        milestone: wire.milestone.as_ref().and_then(milestone),
    };
    let mut involvement = Vec::new();
    if pull.author.login.eq_ignore_ascii_case(me) {
        involvement.push(Involvement::Created);
    }
    if pull
        .assignees
        .iter()
        .any(|user| user.login.eq_ignore_ascii_case(me))
    {
        involvement.push(Involvement::Assigned);
    }
    if review_asked
        || pull.reviewers.iter().any(|reviewer| {
            reviewer.user.login.eq_ignore_ascii_case(me)
                && reviewer.state == crate::model::ReviewState::Pending
        })
    {
        involvement.push(Involvement::ReviewRequested);
    }
    if involvement.is_empty() {
        involvement.push(Involvement::Mentioned);
    }
    Some(FoundPull {
        pull,
        facts,
        involvement,
    })
}

fn issue(wire: &InboxIssue) -> Option<Issue> {
    let number = wire.number?;
    let repo = repository(wire.repository.as_ref()?.name_with_owner.as_deref()?)?;
    let text = |text: &Option<String>, max: usize| {
        api::clean_line(text.as_deref().unwrap_or_default(), max)
    };
    Some(Issue {
        repo,
        number,
        title: api::title(wire.title.as_deref().unwrap_or_default()),
        state: if wire.state.as_deref() == Some("CLOSED") {
            IssueState::Closed
        } else {
            IssueState::Open
        },
        author: wire
            .author
            .as_ref()
            .map(GraphUser::user)
            .unwrap_or_default(),
        assignees: items(&wire.assignees).map(GraphUser::user).collect(),
        labels: items(&wire.labels).map(label).collect(),
        milestone: wire.milestone.as_ref().and_then(milestone),
        created: text(&wire.created, 64),
        updated: text(&wire.updated, 64),
        comments: wire
            .comments
            .as_ref()
            .and_then(|count| count.total)
            .unwrap_or(0),
        web_url: text(&wire.url, 2048),
    })
}

/// The user's open pull requests and issues.
pub(super) fn inbox(session: &Session<'_>, query: &InboxQuery) -> Result<Found, ProviderError> {
    let login = session.account.login.as_str();
    if !plain_login(login) {
        return Err(ProviderError::Refused(say!(
            "github-login-not-searchable",
            login = format!("{login:?}")
        )));
    }
    let first = query.most.clamp(1, MOST_PER_SEARCH);
    let scope = scoped(query);
    let pulls = |whose: &str| {
        format!("is:pr is:open archived:false {whose}:{login}{scope} sort:updated-desc")
    };
    let issues = |whose: &str| {
        format!("is:issue is:open archived:false {whose}:{login}{scope} sort:updated-desc")
    };
    let pull_fields = format!(
        "pageInfo {{ hasNextPage }} nodes {{ ... on PullRequest {{ {ROW} \
         additions deletions mergeable reviewDecision \
         milestone {{ number title state dueOn }} }} }}"
    );
    let issue_fields =
        format!("pageInfo {{ hasNextPage }} nodes {{ ... on Issue {{ {ISSUE_ROW} }} }}");
    let text = format!(
        "query($pulls: String!, $review: String!, $assigned: String!, $created: String!, \
         $mentioned: String!, $first: Int!) {{ \
         pulls: search(query: $pulls, type: ISSUE, first: $first) {{ {pull_fields} }} \
         review: search(query: $review, type: ISSUE, first: $first) {{ {pull_fields} }} \
         assigned: search(query: $assigned, type: ISSUE, first: $first) {{ {issue_fields} }} \
         created: search(query: $created, type: ISSUE, first: $first) {{ {issue_fields} }} \
         mentioned: search(query: $mentioned, type: ISSUE, first: $first) {{ {issue_fields} }} }}"
    );
    let variables = json!({
        "pulls": pulls("involves"),
        "review": pulls("review-requested"),
        "assigned": issues("assignee"),
        "created": issues("author"),
        "mentioned": issues("mentions"),
        "first": first,
    });
    let (data, response): (Data, _) = session.graphql()?.graphql(&text, &variables)?;
    let mut found = Found {
        freshness: fresh(session, &response),
        ..Found::default()
    };
    let pulls_of = data.pulls.unwrap_or_default();
    let review_of = data.review.unwrap_or_default();
    found.more |= pulls_of.page.next || review_of.page.next;
    let asked: Vec<(String, u64)> = review_of
        .nodes
        .iter()
        .flatten()
        .filter_map(|wire| pull(wire, login, true))
        .map(|found| (found.pull.repo.to_ascii_lowercase(), found.pull.number))
        .collect();
    for (wire, review) in pulls_of
        .nodes
        .iter()
        .flatten()
        .map(|wire| (wire, false))
        .chain(review_of.nodes.iter().flatten().map(|wire| (wire, true)))
    {
        let Some(mut made) = pull(wire, login, review) else {
            continue;
        };
        let id = (made.pull.repo.to_ascii_lowercase(), made.pull.number);
        if found
            .pulls
            .iter()
            .any(|known| (known.pull.repo.to_ascii_lowercase(), known.pull.number) == id)
        {
            continue;
        }
        if asked.contains(&id) && !made.involvement.contains(&Involvement::ReviewRequested) {
            made.involvement
                .retain(|why| *why != Involvement::Mentioned);
            made.involvement.push(Involvement::ReviewRequested);
        }
        found.pulls.push(made);
    }
    for (searched, why) in [
        (data.assigned, Involvement::Assigned),
        (data.created, Involvement::Created),
        (data.mentioned, Involvement::Mentioned),
    ] {
        let searched = searched.unwrap_or_default();
        found.more |= searched.page.next;
        for wire in searched.nodes.iter().flatten() {
            let Some(made) = issue(wire) else {
                continue;
            };
            let reactions = wire.reactions.as_ref().and_then(|count| count.total);
            match found.issues.iter_mut().find(|known| {
                known.issue.number == made.number
                    && known.issue.repo.eq_ignore_ascii_case(&made.repo)
            }) {
                Some(known) => {
                    if !known.involvement.contains(&why) {
                        known.involvement.push(why);
                    }
                }
                None => found.issues.push(FoundIssue {
                    issue: made,
                    reactions,
                    involvement: vec![why],
                }),
            }
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_is_searched_for_only_when_it_is_plain() {
        assert!(plain_login("octo-cat"));
        assert!(plain_login("ghe_user"));
        for bad in ["", "a b", "a:b", "x\"y", "repo:o/r", &"a".repeat(101)] {
            assert!(!plain_login(bad), "{bad:?}");
        }
    }

    #[test]
    fn a_workspace_holds_the_searches_to_its_repositories() {
        let of = |repos: Option<Vec<&str>>| {
            scoped(&InboxQuery {
                repos: repos.map(|repos| repos.into_iter().map(str::to_owned).collect()),
                most: 10,
            })
        };
        assert_eq!(of(None), "");
        assert_eq!(of(Some(vec!["o/a", "o/b"])), " repo:o/a repo:o/b");
        // A GitLab path or a hostile one is not GitHub's.
        assert_eq!(of(Some(vec!["g/s/p", "o/a b", "o/a"])), " repo:o/a");
        assert_eq!(of(Some(vec!["g/s/p"])), "");
        let many: Vec<String> = (0..=MOST_SCOPED).map(|at| format!("o/r{at}")).collect();
        assert_eq!(
            scoped(&InboxQuery {
                repos: Some(many),
                most: 10
            }),
            ""
        );
    }

    #[test]
    fn states_are_read_as_github_names_them() {
        assert_eq!(mergeable(Some("MERGEABLE")), Mergeable::Clean);
        assert_eq!(mergeable(Some("CONFLICTING")), Mergeable::Conflicts);
        assert_eq!(mergeable(Some("UNKNOWN")), Mergeable::Unknown);
        assert_eq!(mergeable(None), Mergeable::Unknown);
        assert_eq!(approval(Some("APPROVED")), Approval::Approved);
        assert_eq!(
            approval(Some("CHANGES_REQUESTED")),
            Approval::ChangesRequested
        );
        assert_eq!(approval(Some("REVIEW_REQUIRED")), Approval::Required);
        assert_eq!(approval(Some("SOMETHING")), Approval::Unknown);
    }
}
