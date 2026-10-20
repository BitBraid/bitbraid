//! Pull requests at GitHub: the list, through GraphQL's search, which
//! answers a row's checks, reviewers and their verdicts, labels,
//! assignees and comment count in one question; one pull request with
//! everything its view shows; the choices for making one; the one of a
//! branch; the stack one is in.
//!
//! The list's query is GitHub's own search grammar, held to the
//! repository and to pull requests: `repo:`, `org:`, `user:`, `owner:`
//! and `is:issue` are refused, and values split by commas are
//! alternatives, one search each ([`crate::search`]).
//!
//! Sources: docs.github.com/en/graphql "Queries" (`search` with
//! `type: ISSUE`), "Objects" (`PullRequest`, `ReviewRequest`,
//! `PullRequestReview`, `PullRequestReviewThread`), "Enums"
//! (`PullRequestState`, `PullRequestReviewState`, `StatusState`);
//! docs.github.com "Searching issues and pull requests" (qualifiers);
//! docs.github.com/en/rest "Pull requests" (get, list commits, list
//! files, `head` to filter by branch, `mergeable` and `mergeable_state`),
//! "Pull request reviews" (list), "Pull request review comments" (list,
//! `in_reply_to_id`), "Issue comments" (list), "Repositories" (get:
//! `allow_merge_commit`, `allow_squash_merge`, `allow_rebase_merge`),
//! "Labels", "Milestones", "Assignees" (list).

pub(super) mod writes;

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::json;

use super::{
    GraphUser, WireLabel, WireMilestone, WireUser,
    checks::{self, WireNodes, WireObject},
    fresh, label, milestone, repo_path,
};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{
        CiState, Comment, Listing, MergeMethod, Mergeable, PullCommit, PullDetails, PullFile,
        PullOptions, PullRequest, PullState, Review, ReviewState, Reviewer, Stack, Thread,
    },
    provider::{self, PullFilter, PullQuery, Session, StateFilter, Target},
    search,
};

/// Most commits of a pull request read (GitHub lists 250 at most).
const MOST_COMMITS: usize = 250;
/// Most files of a pull request read.
const MOST_FILES: usize = 300;
/// Most comments, reviews or conversations read.
const MOST_COMMENTS: usize = 1000;
/// Most of each choice read: labels, milestones, people.
const MOST_CHOICES: usize = 500;

/// What a row of the list asks of each pull request.
pub(super) const ROW: &str = "number title state isDraft url createdAt updatedAt \
author { login avatarUrl url ... on User { name } } \
headRefName headRefOid headRepository { nameWithOwner } \
baseRefName baseRepository { nameWithOwner } \
labels(first: 20) { nodes { name color description } } \
assignees(first: 20) { nodes { login name avatarUrl url } } \
reviewRequests(first: 20) { nodes { requestedReviewer { \
... on User { login name avatarUrl url } ... on Team { combinedSlug name avatarUrl url } \
... on Bot { login avatarUrl url } ... on Mannequin { login avatarUrl url } } } } \
latestReviews(first: 20) { nodes { state author { login avatarUrl url ... on User { name } } } } \
comments { totalCount } \
commits(last: 1) { nodes { commit { statusCheckRollup { state } } } }";

#[derive(Debug, Deserialize)]
struct GraphRepository {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

#[derive(Debug, Deserialize)]
struct GraphReviewRequest {
    #[serde(default, rename = "requestedReviewer")]
    requested: Option<GraphUser>,
}

#[derive(Debug, Deserialize)]
struct GraphReview {
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    author: Option<GraphUser>,
}

#[derive(Debug, Deserialize)]
struct GraphCount {
    #[serde(default, rename = "totalCount")]
    total: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GraphStateRollup {
    #[serde(default)]
    state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphCommit {
    #[serde(default, rename = "statusCheckRollup")]
    rollup: Option<GraphStateRollup>,
}

#[derive(Debug, Deserialize)]
struct GraphCommitNode {
    #[serde(default)]
    commit: Option<GraphCommit>,
}

/// A pull request as the list's search answers it; another kind of
/// result (an issue) is all `None` and left out.
#[derive(Debug, Deserialize)]
pub(super) struct GraphPull {
    #[serde(default)]
    number: Option<u64>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default, rename = "isDraft")]
    draft: Option<bool>,
    #[serde(default)]
    url: Option<String>,
    #[serde(default, rename = "createdAt")]
    created: Option<String>,
    #[serde(default, rename = "updatedAt")]
    updated: Option<String>,
    #[serde(default)]
    author: Option<GraphUser>,
    #[serde(default, rename = "headRefName")]
    head_ref: Option<String>,
    #[serde(default, rename = "headRefOid")]
    head_sha: Option<String>,
    #[serde(default, rename = "headRepository")]
    head_repo: Option<GraphRepository>,
    #[serde(default, rename = "baseRefName")]
    base_ref: Option<String>,
    #[serde(default, rename = "baseRepository")]
    base_repo: Option<GraphRepository>,
    #[serde(default)]
    labels: Option<WireNodes<WireLabel>>,
    #[serde(default)]
    assignees: Option<WireNodes<GraphUser>>,
    #[serde(default, rename = "reviewRequests")]
    review_requests: Option<WireNodes<GraphReviewRequest>>,
    #[serde(default, rename = "latestReviews")]
    latest_reviews: Option<WireNodes<GraphReview>>,
    #[serde(default)]
    comments: Option<GraphCount>,
    #[serde(default)]
    commits: Option<WireNodes<GraphCommitNode>>,
}

/// A verdict as GitHub names it.
fn review_state(state: &str) -> Option<ReviewState> {
    match state {
        "APPROVED" => Some(ReviewState::Approved),
        "CHANGES_REQUESTED" => Some(ReviewState::ChangesRequested),
        "COMMENTED" => Some(ReviewState::Commented),
        "DISMISSED" => Some(ReviewState::Dismissed),
        "PENDING" => Some(ReviewState::Pending),
        _ => None,
    }
}

/// The reviewers: each one's last verdict, and those asked who have
/// not answered since as pending.
fn reviewers(
    verdicts: impl IntoIterator<Item = Reviewer>,
    asked: impl IntoIterator<Item = crate::model::User>,
) -> Vec<Reviewer> {
    let mut out: Vec<Reviewer> = Vec::new();
    for verdict in verdicts {
        if verdict.user.login.is_empty() {
            continue;
        }
        match out
            .iter_mut()
            .find(|known| known.user.login == verdict.user.login)
        {
            Some(known) => *known = verdict,
            None => out.push(verdict),
        }
    }
    for user in asked {
        if user.login.is_empty() {
            continue;
        }
        match out.iter_mut().find(|known| known.user.login == user.login) {
            Some(known) => known.state = ReviewState::Pending,
            None => out.push(Reviewer {
                user,
                state: ReviewState::Pending,
            }),
        }
    }
    out
}

/// The sum GitHub gives of a commit's checks.
fn rollup_state(state: Option<&str>) -> CiState {
    match state {
        Some("SUCCESS") => CiState::Success,
        Some("FAILURE" | "ERROR") => CiState::Failure,
        Some("PENDING" | "EXPECTED") => CiState::Pending,
        _ => CiState::None,
    }
}

/// The items of a list GraphQL answered, the missing ones left out.
pub(super) fn items<T>(list: &Option<WireNodes<T>>) -> impl Iterator<Item = &T> {
    list.iter().flat_map(|list| list.nodes.iter().flatten())
}

pub(super) fn graph_pull(wire: &GraphPull, target: &Target) -> Option<PullRequest> {
    let number = wire.number?;
    let state = match wire.state.as_deref() {
        Some("OPEN") => PullState::Open,
        Some("MERGED") => PullState::Merged,
        _ => PullState::Closed,
    };
    let base_repo = wire
        .base_repo
        .as_ref()
        .map_or_else(|| target.full_name(), |repo| repo.name_with_owner.clone());
    let head_repo = wire
        .head_repo
        .as_ref()
        .map(|repo| repo.name_with_owner.clone())
        .filter(|head| !head.eq_ignore_ascii_case(&base_repo));
    let verdicts = items(&wire.latest_reviews).filter_map(|review| {
        let state = review_state(review.state.as_deref()?)?;
        // A review not sent yet is its author's own.
        (state != ReviewState::Pending).then(|| Reviewer {
            user: review
                .author
                .as_ref()
                .map(GraphUser::user)
                .unwrap_or_default(),
            state,
        })
    });
    let asked = items(&wire.review_requests)
        .filter_map(|request| request.requested.as_ref())
        .map(GraphUser::user);
    let ci = items(&wire.commits)
        .last()
        .and_then(|node| node.commit.as_ref())
        .and_then(|commit| commit.rollup.as_ref())
        .map_or(CiState::None, |rollup| {
            rollup_state(rollup.state.as_deref())
        });
    let text = |text: &Option<String>, max: usize| {
        api::clean_line(text.as_deref().unwrap_or_default(), max)
    };
    Some(PullRequest {
        repo: api::clean_line(&base_repo, 512),
        number,
        title: api::title(wire.title.as_deref().unwrap_or_default()),
        state,
        draft: wire.draft.unwrap_or(false),
        author: wire
            .author
            .as_ref()
            .map(GraphUser::user)
            .unwrap_or_default(),
        head_ref: text(&wire.head_ref, 512),
        head_repo: head_repo.map(|repo| api::clean_line(&repo, 512)),
        head_sha: text(&wire.head_sha, 64),
        base_ref: text(&wire.base_ref, 512),
        created: text(&wire.created, 64),
        updated: text(&wire.updated, 64),
        labels: items(&wire.labels).map(label).collect(),
        reviewers: reviewers(verdicts, asked),
        assignees: items(&wire.assignees).map(GraphUser::user).collect(),
        ci,
        comments: wire.comments.as_ref().and_then(|count| count.total),
        web_url: text(&wire.url, 2048),
    })
}

/// Keys of GitHub's search that would leave the repository, or the pull
/// requests.
fn refused(single: &search::Single) -> bool {
    let key = single.key.as_deref().unwrap_or_default();
    let value = single.value.to_ascii_lowercase();
    matches!(key, "repo" | "org" | "user" | "owner")
        || (matches!(key, "is" | "type") && value == "issue")
}

/// The searches a list stands for: the repository, the state, whose, and
/// the user's query, one search per choice of its alternatives.
fn searches(
    session: &Session<'_>,
    target: &Target,
    query: &PullQuery,
) -> Result<Vec<String>, ProviderError> {
    let mut base = vec![format!("repo:{}", target.full_name()), "is:pr".to_owned()];
    match query.state {
        StateFilter::Open => base.push("is:open".to_owned()),
        StateFilter::Closed => base.extend(["is:closed".to_owned(), "is:unmerged".to_owned()]),
        StateFilter::Merged => base.push("is:merged".to_owned()),
        StateFilter::All => {}
    }
    let login = &session.account.login;
    match query.filter {
        PullFilter::All => {}
        PullFilter::Mine => base.push(format!("author:{login}")),
        PullFilter::Assigned => base.push(format!("assignee:{login}")),
        PullFilter::ReviewRequested => base.push(format!("review-requested:{login}")),
    }
    let terms = search::parse(query.query.as_deref().unwrap_or_default(), false)
        .map_err(ProviderError::Refused)?;
    let expanded = search::expand(&terms).map_err(ProviderError::Refused)?;
    let mut out = Vec::new();
    for words in expanded {
        if let Some(odd) = words.iter().find(|single| refused(single)) {
            return Err(ProviderError::Refused(say!(
                "github-pulls-query-not-taken",
                term = odd.github()
            )));
        }
        let sorted = words
            .iter()
            .any(|single| single.key.as_deref() == Some("sort"));
        let mut all = base.clone();
        all.extend(words.iter().map(search::Single::github));
        if !sorted {
            all.push("sort:created-desc".to_owned());
        }
        out.push(all.join(" "));
    }
    Ok(out)
}

/// A repository's pull requests.
pub(super) fn list(
    session: &Session<'_>,
    target: &Target,
    query: &PullQuery,
) -> Result<Listing<PullRequest>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct PageInfo {
        #[serde(default, rename = "hasNextPage")]
        next: bool,
        #[serde(default, rename = "endCursor")]
        cursor: Option<String>,
    }
    #[derive(Debug, Deserialize)]
    struct Found {
        #[serde(rename = "pageInfo")]
        page: PageInfo,
        #[serde(default = "Vec::new")]
        nodes: Vec<Option<GraphPull>>,
    }
    #[derive(Debug, Deserialize)]
    struct Data {
        search: Found,
    }
    let text = format!(
        "query($search: String!, $first: Int!, $after: String) {{ \
         search(query: $search, type: ISSUE, first: $first, after: $after) {{ \
         pageInfo {{ hasNextPage endCursor }} nodes {{ ... on PullRequest {{ {ROW} }} }} }} }}"
    );
    let graphql = session.graphql()?;
    let mut items: Vec<PullRequest> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut more = false;
    let mut freshness = crate::model::Freshness::default();
    let searches = searches(session, target, query)?;
    let several = searches.len() > 1;
    'searches: for words in searches {
        let mut after: Option<String> = None;
        for _ in 0..api::MAX_PAGES {
            let (data, response): (Data, _) = graphql.graphql(
                &text,
                &json!({ "search": words, "first": 100, "after": after }),
            )?;
            freshness = api::latest(freshness, fresh(session, &response));
            for pull in data.search.nodes.iter().flatten() {
                let Some(pull) = graph_pull(pull, target) else {
                    continue;
                };
                if seen.insert(pull.number) {
                    items.push(pull);
                }
                if items.len() >= query.most {
                    more = true;
                    break 'searches;
                }
            }
            match data.search.page.cursor {
                Some(cursor) if data.search.page.next && after.as_ref() != Some(&cursor) => {
                    after = Some(cursor);
                }
                _ => continue 'searches,
            }
        }
        // At most so many pages: the rest is said to be there.
        more = true;
    }
    if several {
        items.sort_by(|a, b| b.created.cmp(&a.created));
    }
    Ok(Listing {
        items,
        more,
        freshness,
    })
}

#[derive(Debug, Deserialize)]
struct WireRepoName {
    full_name: String,
}

#[derive(Debug, Deserialize)]
struct WireBranch {
    #[serde(rename = "ref")]
    name: String,
    #[serde(default)]
    sha: String,
    #[serde(default)]
    repo: Option<WireRepoName>,
}

/// A pull request as REST answers it.
#[derive(Debug, Deserialize)]
struct WirePull {
    number: u64,
    /// GraphQL's id for it, which marking it a draft or ready takes.
    #[serde(default)]
    node_id: Option<String>,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    merged_at: Option<String>,
    #[serde(default)]
    merged: Option<bool>,
    user: Option<WireUser>,
    head: WireBranch,
    base: WireBranch,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    labels: Vec<WireLabel>,
    #[serde(default)]
    requested_reviewers: Vec<WireUser>,
    #[serde(default)]
    requested_teams: Vec<WireTeam>,
    #[serde(default)]
    assignees: Vec<WireUser>,
    #[serde(default)]
    comments: Option<u64>,
    html_url: String,
    // What one pull request's answer adds to the list's.
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    milestone: Option<WireMilestone>,
    #[serde(default)]
    mergeable: Option<bool>,
    #[serde(default)]
    mergeable_state: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireTeam {
    #[serde(default)]
    slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

fn pull(wire: &WirePull, target: &Target) -> PullRequest {
    let state = match (
        wire.state.as_str(),
        wire.merged_at.is_some() || wire.merged == Some(true),
    ) {
        ("open", _) => PullState::Open,
        (_, true) => PullState::Merged,
        _ => PullState::Closed,
    };
    let base_repo = wire
        .base
        .repo
        .as_ref()
        .map_or_else(|| target.full_name(), |repo| repo.full_name.clone());
    let head_repo = wire
        .head
        .repo
        .as_ref()
        .map(|repo| repo.full_name.clone())
        .filter(|head| !head.eq_ignore_ascii_case(&base_repo));
    let teams = wire.requested_teams.iter().map(|team| {
        api::user(
            team.slug.as_deref().unwrap_or_default(),
            team.name.as_deref(),
            None,
            team.html_url.as_deref(),
        )
    });
    PullRequest {
        repo: api::clean_line(&base_repo, 512),
        number: wire.number,
        title: api::title(&wire.title),
        state,
        draft: wire.draft,
        author: wire.user.as_ref().map(WireUser::user).unwrap_or_default(),
        head_ref: api::clean_line(&wire.head.name, 512),
        head_repo: head_repo.map(|repo| api::clean_line(&repo, 512)),
        head_sha: api::clean_line(&wire.head.sha, 64),
        base_ref: api::clean_line(&wire.base.name, 512),
        created: api::clean_line(&wire.created_at, 64),
        updated: api::clean_line(&wire.updated_at, 64),
        labels: wire.labels.iter().map(label).collect(),
        reviewers: reviewers(
            std::iter::empty(),
            wire.requested_reviewers
                .iter()
                .map(WireUser::user)
                .chain(teams),
        ),
        assignees: wire.assignees.iter().map(WireUser::user).collect(),
        ci: CiState::None,
        comments: wire.comments,
        web_url: api::clean_line(&wire.html_url, 2048),
    }
}

/// What a repository allows when merging, as its answer says.
#[derive(Debug, Deserialize)]
struct WireRepoSettings {
    #[serde(default)]
    allow_merge_commit: Option<bool>,
    #[serde(default)]
    allow_squash_merge: Option<bool>,
    #[serde(default)]
    allow_rebase_merge: Option<bool>,
}

/// The ways a repository allows merging; a way the answer does not
/// name (it names them to those who may push) is taken as allowed.
fn merge_methods(settings: &WireRepoSettings) -> Vec<MergeMethod> {
    [
        (settings.allow_merge_commit, MergeMethod::Merge),
        (settings.allow_squash_merge, MergeMethod::Squash),
        (settings.allow_rebase_merge, MergeMethod::Rebase),
    ]
    .into_iter()
    .filter(|(allowed, _)| allowed.unwrap_or(true))
    .map(|(_, method)| method)
    .collect()
}

/// Whether it can be merged, from `mergeable` and `mergeable_state`; a
/// draft is not merged until it is marked ready, as at GitLab.
fn mergeable(wire: &WirePull) -> Mergeable {
    match (wire.mergeable, wire.mergeable_state.as_deref()) {
        (_, Some("dirty")) | (Some(false), _) => Mergeable::Conflicts,
        _ if wire.draft => Mergeable::Blocked,
        (_, Some("blocked" | "behind" | "draft")) => Mergeable::Blocked,
        (Some(true), _) => Mergeable::Clean,
        _ => Mergeable::Unknown,
    }
}

#[derive(Debug, Deserialize)]
struct WireGitPerson {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireGitCommit {
    #[serde(default)]
    message: String,
    #[serde(default)]
    author: Option<WireGitPerson>,
}

#[derive(Debug, Deserialize)]
struct WireCommit {
    sha: String,
    commit: WireGitCommit,
    #[serde(default)]
    author: Option<WireUser>,
}

fn commit(wire: &WireCommit) -> PullCommit {
    let person = wire.commit.author.as_ref();
    PullCommit {
        sha: api::clean_line(&wire.sha, 64),
        summary: api::title(wire.commit.message.lines().next().unwrap_or_default()),
        author: wire.author.as_ref().map(WireUser::user),
        author_name: api::clean_line(
            person
                .and_then(|person| person.name.as_deref())
                .unwrap_or_default(),
            256,
        ),
        date: api::clean_line(
            person
                .and_then(|person| person.date.as_deref())
                .unwrap_or_default(),
            64,
        ),
    }
}

#[derive(Debug, Deserialize)]
struct WireFile {
    filename: String,
    #[serde(default)]
    previous_filename: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    additions: u64,
    #[serde(default)]
    deletions: u64,
    #[serde(default)]
    patch: Option<String>,
}

fn file(wire: &WireFile) -> PullFile {
    let status = wire
        .status
        .as_deref()
        .filter(|status| {
            matches!(
                *status,
                "added" | "removed" | "modified" | "renamed" | "copied"
            )
        })
        .unwrap_or("changed");
    PullFile {
        path: api::clean_line(&wire.filename, 4096),
        previous_path: wire
            .previous_filename
            .as_deref()
            .map(|path| api::clean_line(path, 4096)),
        status: status.to_owned(),
        additions: wire.additions,
        deletions: wire.deletions,
        patch: wire.patch.as_deref().map(api::body),
    }
}

#[derive(Debug, Deserialize)]
struct WireReview {
    id: u64,
    #[serde(default)]
    user: Option<WireUser>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    submitted_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct WireComment {
    id: u64,
    #[serde(default)]
    user: Option<WireUser>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
    // A review comment's place.
    #[serde(default)]
    in_reply_to_id: Option<u64>,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    line: Option<u64>,
    #[serde(default)]
    original_line: Option<u64>,
    #[serde(default)]
    diff_hunk: Option<String>,
    // An issue comment's pull request or issue.
    #[serde(default)]
    issue_url: Option<String>,
}

fn comment(wire: &WireComment) -> Comment {
    let created = api::clean_line(&wire.created_at, 64);
    Comment {
        id: wire.id.to_string(),
        author: wire.user.as_ref().map(WireUser::user).unwrap_or_default(),
        body: api::body(wire.body.as_deref().unwrap_or_default()),
        updated: wire
            .updated_at
            .as_deref()
            .map(|updated| api::clean_line(updated, 64))
            .filter(|updated| *updated != created),
        created,
    }
}

/// Review comments grouped into conversations by the comment each
/// answers, with whether each was marked resolved (by its first
/// comment's id); the conversation on the whole pull request first when
/// it began first.
fn threads(
    review_comments: &[WireComment],
    issue_comments: &[WireComment],
    resolved: &BTreeMap<u64, bool>,
) -> Vec<Thread> {
    let by_id: BTreeMap<u64, &WireComment> = review_comments
        .iter()
        .map(|comment| (comment.id, comment))
        .collect();
    // The first comment of each one's conversation: follow the answers
    // up, at most so far, in case they loop.
    let root = |comment: &WireComment| {
        let mut at = comment;
        for _ in 0..64 {
            match at.in_reply_to_id.and_then(|id| by_id.get(&id)) {
                Some(parent) if parent.id != at.id => at = parent,
                _ => break,
            }
        }
        at.id
    };
    let mut grouped: BTreeMap<u64, Vec<&WireComment>> = BTreeMap::new();
    for comment in review_comments {
        grouped.entry(root(comment)).or_default().push(comment);
    }
    let mut out: Vec<Thread> = grouped
        .into_iter()
        .filter_map(|(id, mut comments)| {
            comments.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
            let first = by_id.get(&id).copied().or(comments.first().copied())?;
            Some(Thread {
                id: id.to_string(),
                path: first
                    .path
                    .as_deref()
                    .map(|path| api::clean_line(path, 4096)),
                line: first.line.or(first.original_line),
                diff_hunk: first.diff_hunk.as_deref().map(api::body),
                resolved: resolved.get(&id).copied().unwrap_or(false),
                comments: comments.into_iter().map(comment).collect(),
            })
        })
        .collect();
    if let Some(first) = issue_comments.first() {
        out.push(Thread {
            id: first.id.to_string(),
            path: None,
            line: None,
            diff_hunk: None,
            resolved: false,
            comments: issue_comments.iter().map(comment).collect(),
        });
    }
    out.sort_by(|a, b| {
        let start = |thread: &Thread| {
            thread
                .comments
                .first()
                .map(|comment| comment.created.clone())
                .unwrap_or_default()
        };
        start(a).cmp(&start(b))
    });
    out
}

/// What GraphQL adds to a pull request's view: whether each review
/// conversation was resolved, and the checks of its head commit.
fn conversations_and_checks(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<(BTreeMap<u64, bool>, Vec<crate::model::Check>), ProviderError> {
    #[derive(Debug, Deserialize)]
    struct FirstComment {
        #[serde(default, rename = "databaseId")]
        id: Option<u64>,
    }
    #[derive(Debug, Deserialize)]
    struct WireThread {
        #[serde(default, rename = "isResolved")]
        resolved: bool,
        #[serde(default)]
        comments: Option<WireNodes<FirstComment>>,
    }
    #[derive(Debug, Deserialize)]
    struct HeadCommit {
        #[serde(default)]
        commit: Option<WireObject>,
    }
    #[derive(Debug, Deserialize)]
    struct Pull {
        #[serde(default, rename = "reviewThreads")]
        threads: Option<WireNodes<WireThread>>,
        #[serde(default)]
        commits: Option<WireNodes<HeadCommit>>,
    }
    #[derive(Debug, Deserialize)]
    struct Repository {
        #[serde(default, rename = "pullRequest")]
        pull: Option<Pull>,
    }
    #[derive(Debug, Deserialize)]
    struct Data {
        #[serde(default)]
        repository: Option<Repository>,
    }
    let query = format!(
        "query($owner: String!, $name: String!, $number: Int!) {{ \
         repository(owner: $owner, name: $name) {{ pullRequest(number: $number) {{ \
         reviewThreads(first: 100) {{ nodes {{ isResolved comments(first: 1) {{ nodes {{ databaseId }} }} }} }} \
         commits(last: 1) {{ nodes {{ commit {{ {} }} }} }} }} }} }}",
        checks::CONTEXTS
    );
    let (data, _): (Data, _) = session.graphql()?.graphql(
        &query,
        &json!({ "owner": target.owner, "name": target.name, "number": number }),
    )?;
    let pull = data
        .repository
        .and_then(|repository| repository.pull)
        .ok_or(ProviderError::NotFound)?;
    let resolved = pull
        .threads
        .map(|threads| threads.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .filter_map(|thread| {
            let first = thread
                .comments
                .as_ref()?
                .nodes
                .iter()
                .flatten()
                .next()?
                .id?;
            Some((first, thread.resolved))
        })
        .collect();
    let head = pull
        .commits
        .map(|commits| commits.nodes)
        .unwrap_or_default()
        .into_iter()
        .flatten()
        .last()
        .and_then(|node| node.commit);
    let found = checks::checks(head.as_ref().and_then(|commit| commit.rollup.as_ref()));
    Ok((resolved, found))
}

/// One pull request with everything its view shows.
pub(super) fn show(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<PullDetails, ProviderError> {
    let api = session.api()?;
    let base = repo_path(target);
    let (wire, first): (WirePull, _) =
        api.json(Method::Get, &format!("{base}/pulls/{number}"), None)?;
    // How fresh it is, as its answers say: kept ones make all of it kept.
    let mut freshness = fresh(session, &first);
    let mut seen = |response: &crate::http::Response| {
        freshness = api::latest(freshness.clone(), fresh(session, response));
    };
    let (settings, answer): (WireRepoSettings, _) = api.json(Method::Get, &base, None)?;
    seen(&answer);
    let (commits, _, answer) = api.pages::<WireCommit>(
        &format!("{base}/pulls/{number}/commits?per_page=100"),
        MOST_COMMITS,
    )?;
    seen(&answer);
    let (files, _, answer) = api.pages::<WireFile>(
        &format!("{base}/pulls/{number}/files?per_page=100"),
        MOST_FILES,
    )?;
    seen(&answer);
    let (reviews, _, answer) = api.pages::<WireReview>(
        &format!("{base}/pulls/{number}/reviews?per_page=100"),
        MOST_COMMENTS,
    )?;
    seen(&answer);
    let (review_comments, _, answer) = api.pages::<WireComment>(
        &format!("{base}/pulls/{number}/comments?per_page=100"),
        MOST_COMMENTS,
    )?;
    seen(&answer);
    let (issue_comments, _, answer) = api.pages::<WireComment>(
        &format!("{base}/issues/{number}/comments?per_page=100"),
        MOST_COMMENTS,
    )?;
    seen(&answer);
    let (resolved, checks) = conversations_and_checks(session, target, number)?;
    let reviews: Vec<Review> = reviews
        .iter()
        .filter_map(|review| {
            let state = review_state(&review.state)?;
            (state != ReviewState::Pending).then(|| Review {
                id: review.id.to_string(),
                author: review.user.as_ref().map(WireUser::user).unwrap_or_default(),
                state,
                body: api::body(review.body.as_deref().unwrap_or_default()),
                submitted: review
                    .submitted_at
                    .as_deref()
                    .map(|at| api::clean_line(at, 64)),
            })
        })
        .collect();
    let mut summary = pull(&wire, target);
    // The last verdict of each reviewer that said more than a comment,
    // or their comment when that is all they said.
    let mut verdicts: Vec<Reviewer> = Vec::new();
    for review in &reviews {
        let said = Reviewer {
            user: review.author.clone(),
            state: review.state,
        };
        match verdicts
            .iter_mut()
            .find(|known| known.user.login == said.user.login)
        {
            Some(known) if said.state != ReviewState::Commented => *known = said,
            Some(_) => {}
            None => verdicts.push(said),
        }
    }
    let asked: Vec<crate::model::User> = summary
        .reviewers
        .iter()
        .map(|reviewer| reviewer.user.clone())
        .collect();
    summary.reviewers = reviewers(verdicts, asked);
    summary.ci = CiState::sum(checks.iter().map(|check| check.state));
    Ok(PullDetails {
        body: api::body(wire.body.as_deref().unwrap_or_default()),
        milestone: wire.milestone.as_ref().map(milestone),
        mergeable: mergeable(&wire),
        merge_methods: merge_methods(&settings),
        checks,
        commits: commits.iter().map(commit).collect(),
        files: files.iter().map(file).collect(),
        reviews,
        threads: threads(&review_comments, &issue_comments, &resolved),
        pull: summary,
        freshness,
    })
}

/// What may be chosen for a pull request.
pub(super) fn options(
    session: &Session<'_>,
    target: &Target,
) -> Result<PullOptions, ProviderError> {
    let api = session.api()?;
    let base = repo_path(target);
    let (labels, _, _) =
        api.pages::<WireLabel>(&format!("{base}/labels?per_page=100"), MOST_CHOICES)?;
    let (milestones, _, _) = api.pages::<WireMilestone>(
        &format!("{base}/milestones?state=open&per_page=100"),
        MOST_CHOICES,
    )?;
    let (people, _, _) =
        api.pages::<WireUser>(&format!("{base}/assignees?per_page=100"), MOST_CHOICES)?;
    let (settings, _): (WireRepoSettings, _) = api.json(Method::Get, &base, None)?;
    Ok(PullOptions {
        labels: labels.iter().map(label).collect(),
        milestones: milestones.iter().map(milestone).collect(),
        people: people.iter().map(WireUser::user).collect(),
        merge_methods: merge_methods(&settings),
    })
}

/// The open pull request whose branch is `branch` in the repository
/// itself.
pub(super) fn for_branch(
    session: &Session<'_>,
    target: &Target,
    branch: &str,
) -> Result<Option<PullRequest>, ProviderError> {
    let path = format!(
        "{}/pulls?state=open&head={}&per_page=100",
        repo_path(target),
        api::encode(&format!("{}:{branch}", target.owner))
    );
    let (found, _, _) = session.api()?.pages::<WirePull>(&path, 100)?;
    Ok(found
        .iter()
        .map(|wire| pull(wire, target))
        .find(|pull| pull.head_ref == branch && pull.head_repo.is_none()))
}

/// The stack a pull request is in, from the repository's open pull
/// requests.
pub(super) fn stack(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<Stack, ProviderError> {
    let api = session.api()?;
    let base = repo_path(target);
    let (open, _, _) = api.pages::<WirePull>(
        &format!("{base}/pulls?state=open&per_page=100"),
        provider::MOST_STACKED,
    )?;
    let mut pulls: Vec<PullRequest> = open.iter().map(|wire| pull(wire, target)).collect();
    if !pulls.iter().any(|pull| pull.number == number) {
        let (wire, _): (WirePull, _) =
            api.json(Method::Get, &format!("{base}/pulls/{number}"), None)?;
        pulls.push(pull(&wire, target));
    }
    provider::stack_of(&pulls, number).ok_or(ProviderError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_search_written_by_hand_is_held_to_the_repository() {
        let single = |key: &str, value: &str| search::Single {
            negated: false,
            key: Some(key.to_owned()),
            value: value.to_owned(),
        };
        assert!(refused(&single("repo", "a/b")));
        assert!(refused(&single("org", "a")));
        assert!(refused(&single("is", "Issue")));
        assert!(!refused(&single("is", "draft")));
        assert!(!refused(&single("label", "bug")));
    }

    #[test]
    fn reviewers_keep_their_last_verdict_unless_asked_again() {
        let user = |login: &str| crate::model::User {
            login: login.to_owned(),
            ..Default::default()
        };
        let said = |login: &str, state| Reviewer {
            user: user(login),
            state,
        };
        let out = reviewers(
            [
                said("a", ReviewState::Approved),
                said("b", ReviewState::ChangesRequested),
                said("", ReviewState::Approved),
            ],
            [user("b"), user("c")],
        );
        assert_eq!(
            out,
            [
                said("a", ReviewState::Approved),
                said("b", ReviewState::Pending),
                said("c", ReviewState::Pending)
            ]
        );
    }
}
