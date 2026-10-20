//! Merge requests at GitLab: the list, with each one's reviewers'
//! verdicts, pipeline and fork asked of GraphQL a hundred at a time; one
//! merge request with everything its view shows; the choices for making
//! one; the one of a branch; the stack one is in.
//!
//! The list's query takes fewer keys than GitHub's: `author:`,
//! `assignee:`, `review-requested:` (or `reviewer:`), `label:`,
//! `milestone:`, `base:`, `head:`, `draft:`, and words, which GitLab looks
//! for in titles and descriptions. `-` leaves out an author, an assignee,
//! a reviewer, a label or a milestone; values split by commas are
//! alternatives, one search each. Any other key is refused, and the error
//! names it.
//!
//! Sources: docs.gitlab.com/api "Merge requests" (list project merge
//! requests: `state`, `author_username`, `assignee_username`,
//! `reviewer_username`, `labels`, `milestone`, `source_branch`,
//! `target_branch`, `wip`, `search`, `not`; get one; its commits, diffs,
//! approvals and reviewers), "Discussions" (merge request discussions,
//! resolvable notes, `position`), "Projects" (`merge_method`,
//! `squash_option`), "Labels", "Milestones", "Group and project members"
//! (all members); docs.gitlab.com/api/graphql "Reference"
//! (`Project.mergeRequests` by `iids`, `MergeRequest.approvedBy`,
//! `MergeRequestReviewer.mergeRequestInteraction`).

pub(super) mod writes;

use std::collections::{BTreeMap, BTreeSet};

use serde::Deserialize;
use serde_json::json;

use super::{
    WireLabel, WireMilestone, WireUser, checks, ci_state, fresh, label, milestone, project_path,
};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{
        CiState, Comment, Listing, MergeMethod, Mergeable, PullCommit, PullDetails, PullFile,
        PullOptions, PullRequest, PullState, Review, ReviewState, Reviewer, Stack, Thread, User,
    },
    provider::{self, PullFilter, PullQuery, Session, StateFilter, Target},
    search,
};

/// Most commits of a merge request read.
const MOST_COMMITS: usize = 500;
/// Most files of a merge request read.
const MOST_FILES: usize = 300;
/// Most conversations read.
const MOST_DISCUSSIONS: usize = 1000;
/// Most of each choice read: labels, milestones, people.
const MOST_CHOICES: usize = 500;

#[derive(Debug, Deserialize)]
pub(super) struct WireMerge {
    iid: u64,
    title: String,
    state: String,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    work_in_progress: bool,
    author: Option<WireUser>,
    source_branch: String,
    target_branch: String,
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    sha: Option<String>,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    labels: Vec<WireLabel>,
    #[serde(default)]
    reviewers: Vec<WireUser>,
    #[serde(default)]
    assignees: Vec<WireUser>,
    #[serde(default)]
    user_notes_count: Option<u64>,
    web_url: String,
    // What one merge request's answer adds to the list's.
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    milestone: Option<WireMilestone>,
    #[serde(default)]
    detailed_merge_status: Option<String>,
    #[serde(default)]
    has_conflicts: bool,
}

impl WireMerge {
    /// Its number in its project.
    pub(super) fn number(&self) -> u64 {
        self.iid
    }
}

/// Whether it comes from a fork: the fork's project id.
pub(super) fn fork_of(wire: &WireMerge) -> Option<u64> {
    match (wire.source_project_id, wire.target_project_id) {
        (Some(source), Some(target)) if source != target => Some(source),
        _ => None,
    }
}

/// A merge request as REST answers it. A fork is named by its path when
/// `forks` knows it; one the account cannot see is as one that is gone.
pub(super) fn merge_request(
    wire: &WireMerge,
    target: &Target,
    forks: &BTreeMap<u64, String>,
) -> PullRequest {
    let state = match wire.state.as_str() {
        "opened" | "locked" => PullState::Open,
        "merged" => PullState::Merged,
        _ => PullState::Closed,
    };
    let head_repo = fork_of(wire).and_then(|source| forks.get(&source).cloned());
    PullRequest {
        repo: target.full_name(),
        number: wire.iid,
        title: api::title(&wire.title),
        state,
        draft: wire.draft || wire.work_in_progress,
        author: wire.author.as_ref().map(WireUser::user).unwrap_or_default(),
        head_ref: api::clean_line(&wire.source_branch, 512),
        head_repo: head_repo.map(|repo| api::clean_line(&repo, 512)),
        head_sha: wire
            .sha
            .as_deref()
            .map(|sha| api::clean_line(sha, 64))
            .unwrap_or_default(),
        base_ref: api::clean_line(&wire.target_branch, 512),
        created: api::clean_line(&wire.created_at, 64),
        updated: api::clean_line(&wire.updated_at, 64),
        labels: wire.labels.iter().map(label).collect(),
        reviewers: wire
            .reviewers
            .iter()
            .map(|reviewer| Reviewer {
                user: reviewer.user(),
                state: ReviewState::Pending,
            })
            .collect(),
        assignees: wire.assignees.iter().map(WireUser::user).collect(),
        ci: CiState::None,
        comments: wire.user_notes_count,
        web_url: api::clean_line(&wire.web_url, 2048),
    }
}

/// A reviewer's verdict as GraphQL (upper case) or REST (lower case)
/// names it.
fn review_state(state: &str) -> ReviewState {
    match state.to_ascii_lowercase().as_str() {
        "approved" => ReviewState::Approved,
        "requested_changes" => ReviewState::ChangesRequested,
        "reviewed" => ReviewState::Commented,
        _ => ReviewState::Pending,
    }
}

#[derive(Debug, Default, Deserialize)]
struct GraphNodes<T> {
    #[serde(default = "Vec::new")]
    nodes: Vec<Option<T>>,
}

#[derive(Debug, Deserialize)]
struct GraphProjectPath {
    #[serde(default, rename = "fullPath")]
    full_path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphPipeline {
    #[serde(default)]
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphInteraction {
    #[serde(default, rename = "reviewState")]
    review_state: Option<String>,
    #[serde(default)]
    approved: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct GraphPerson {
    #[serde(default)]
    username: Option<String>,
    #[serde(default, rename = "mergeRequestInteraction")]
    interaction: Option<GraphInteraction>,
}

#[derive(Debug, Deserialize)]
pub(super) struct GraphMerge {
    #[serde(default)]
    iid: Option<String>,
    #[serde(default, rename = "sourceProject")]
    source_project: Option<GraphProjectPath>,
    #[serde(default, rename = "headPipeline")]
    head_pipeline: Option<GraphPipeline>,
    #[serde(default)]
    reviewers: Option<GraphNodes<GraphPerson>>,
    #[serde(default, rename = "approvedBy")]
    approved_by: Option<GraphNodes<GraphPerson>>,
    /// Lines added and removed, which only the inbox asks for.
    #[serde(default, rename = "diffStatsSummary")]
    diff_stats: Option<GraphDiffStats>,
}

#[derive(Debug, Deserialize)]
struct GraphDiffStats {
    #[serde(default)]
    additions: Option<u64>,
    #[serde(default)]
    deletions: Option<u64>,
}

impl GraphMerge {
    /// The path of the fork it comes from, when GraphQL said.
    pub(super) fn source_path(&self) -> Option<&str> {
        self.source_project.as_ref()?.full_path.as_deref()
    }

    /// Lines added and removed, when it was asked and said.
    pub(super) fn lines(&self) -> (Option<u64>, Option<u64>) {
        self.diff_stats
            .as_ref()
            .map_or((None, None), |stats| (stats.additions, stats.deletions))
    }
}

/// What GraphQL adds to a page of the list, by number: each one's
/// fork's path, pipeline, and reviewers' verdicts.
fn enrich(
    session: &Session<'_>,
    target: &Target,
    numbers: &[u64],
) -> Result<BTreeMap<u64, GraphMerge>, ProviderError> {
    enrich_with(session, target, numbers, ENRICH)
}

/// The question of [`enrich`].
const ENRICH: &str = "query($path: ID!, $iids: [String!]) { project(fullPath: $path) { \
        mergeRequests(iids: $iids, first: 100) { nodes { iid sourceProject { fullPath } \
        headPipeline { status } \
        reviewers { nodes { username mergeRequestInteraction { reviewState approved } } } \
        approvedBy { nodes { username } } } } } }";

/// [`ENRICH`] with the lines added and removed, which the inbox shows.
pub(super) const ENRICH_WITH_LINES: &str = "query($path: ID!, $iids: [String!]) { \
        project(fullPath: $path) { \
        mergeRequests(iids: $iids, first: 100) { nodes { iid sourceProject { fullPath } \
        headPipeline { status } \
        reviewers { nodes { username mergeRequestInteraction { reviewState approved } } } \
        approvedBy { nodes { username } } diffStatsSummary { additions deletions } } } } }";

/// What GraphQL adds to merge requests, by number, asked with `query`
/// (one of [`ENRICH`] and [`ENRICH_WITH_LINES`]), a hundred at a time.
pub(super) fn enrich_with(
    session: &Session<'_>,
    target: &Target,
    numbers: &[u64],
    query: &str,
) -> Result<BTreeMap<u64, GraphMerge>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct Project {
        #[serde(default, rename = "mergeRequests")]
        merge_requests: Option<GraphNodes<GraphMerge>>,
    }
    #[derive(Debug, Deserialize)]
    struct Data {
        #[serde(default)]
        project: Option<Project>,
    }
    let graphql = session.graphql()?;
    let mut out = BTreeMap::new();
    for chunk in numbers.chunks(100) {
        let iids: Vec<String> = chunk.iter().map(u64::to_string).collect();
        let (data, _): (Data, _) =
            graphql.graphql(query, &json!({ "path": target.full_name(), "iids": iids }))?;
        let merges = data
            .project
            .and_then(|project| project.merge_requests)
            .map(|merges| merges.nodes)
            .unwrap_or_default();
        for merge in merges.into_iter().flatten() {
            if let Some(number) = merge.iid.as_deref().and_then(|iid| iid.parse().ok()) {
                out.insert(number, merge);
            }
        }
    }
    Ok(out)
}

/// The paths of forks by their project's id, asked one by one: when
/// GraphQL could not say.
pub(super) fn fork_paths(
    session: &Session<'_>,
    wires: &[&WireMerge],
) -> Result<BTreeMap<u64, String>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireProject {
        path_with_namespace: String,
    }
    let ids: BTreeSet<u64> = wires.iter().filter_map(|wire| fork_of(wire)).collect();
    let api = session.api()?;
    let mut out = BTreeMap::new();
    // A list of many forks is named by their ids past the first few.
    for id in ids.into_iter().take(20) {
        match api.json::<WireProject>(Method::Get, &format!("/projects/{id}"), None) {
            Ok((project, _)) => {
                out.insert(id, project.path_with_namespace);
            }
            Err(ProviderError::NotFound) => {}
            Err(err) => return Err(err),
        }
    }
    Ok(out)
}

/// A merge request of the list with what GraphQL said of it.
pub(super) fn enriched(mut pull: PullRequest, extra: Option<&GraphMerge>) -> PullRequest {
    let Some(extra) = extra else {
        return pull;
    };
    if let Some(status) = extra
        .head_pipeline
        .as_ref()
        .and_then(|pipeline| pipeline.status.as_deref())
    {
        pull.ci = ci_state(status, false);
    }
    let verdicts: BTreeMap<String, ReviewState> = extra
        .reviewers
        .as_ref()
        .map(|reviewers| reviewers.nodes.iter().flatten().collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|person| {
            let interaction = person.interaction.as_ref()?;
            let state = match (interaction.approved, interaction.review_state.as_deref()) {
                (Some(true), _) => ReviewState::Approved,
                (_, Some(state)) => review_state(state),
                _ => ReviewState::Pending,
            };
            Some((person.username.clone()?, state))
        })
        .collect();
    for reviewer in &mut pull.reviewers {
        if let Some(state) = verdicts.get(&reviewer.user.login) {
            reviewer.state = *state;
        }
    }
    // Who approved without being asked is a reviewer all the same.
    for person in extra
        .approved_by
        .as_ref()
        .map(|approved| approved.nodes.iter().flatten().collect::<Vec<_>>())
        .unwrap_or_default()
    {
        let Some(login) = person.username.as_deref() else {
            continue;
        };
        match pull
            .reviewers
            .iter_mut()
            .find(|reviewer| reviewer.user.login == login)
        {
            Some(reviewer) => reviewer.state = ReviewState::Approved,
            None => pull.reviewers.push(Reviewer {
                user: api::user(login, None, None, None),
                state: ReviewState::Approved,
            }),
        }
    }
    pull
}

/// The list's parameters of one search: the keys of the query as GitLab
/// takes them.
fn query_parameters(
    words: &[search::Single],
    login: &str,
) -> Result<Vec<(String, String)>, ProviderError> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut not_labels: Vec<String> = Vec::new();
    let mut text: Vec<String> = Vec::new();
    let mut ignored: Vec<String> = Vec::new();
    let mut put = |key: &str, value: &str| {
        out.retain(|(known, _)| known != key);
        out.push((key.to_owned(), value.to_owned()));
    };
    for single in words {
        let value = if single.value == "@me" {
            login
        } else {
            single.value.as_str()
        };
        let name = match (single.key.as_deref(), single.negated) {
            (None, false) => {
                text.push(value.to_owned());
                continue;
            }
            (Some("label" | "labels"), false) => {
                labels.push(value.to_owned());
                continue;
            }
            (Some("label" | "labels"), true) => {
                not_labels.push(value.to_owned());
                continue;
            }
            (Some("author"), false) => "author_username",
            (Some("author"), true) => "not[author_username]",
            (Some("assignee"), false) => "assignee_username",
            (Some("assignee"), true) => "not[assignee_username]",
            (Some("review-requested" | "reviewer"), false) => "reviewer_username",
            (Some("review-requested" | "reviewer"), true) => "not[reviewer_username]",
            (Some("milestone"), false) => "milestone",
            (Some("milestone"), true) => "not[milestone]",
            (Some("base"), false) => "target_branch",
            (Some("head"), false) => "source_branch",
            (Some("draft"), negated) => {
                let draft = value.eq_ignore_ascii_case("true") != negated;
                put("wip", if draft { "yes" } else { "no" });
                continue;
            }
            _ => {
                ignored.push(
                    single
                        .github()
                        .split(':')
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                );
                continue;
            }
        };
        put(name, value);
    }
    if !ignored.is_empty() {
        ignored.sort();
        ignored.dedup();
        return Err(ProviderError::Refused(say!(
            "gitlab-pulls-not-searchable",
            keys = ignored.join(", ")
        )));
    }
    if !labels.is_empty() {
        put("labels", &labels.join(","));
    }
    if !not_labels.is_empty() {
        put("not[labels]", &not_labels.join(","));
    }
    if !text.is_empty() {
        put("search", &text.join(" "));
    }
    Ok(out)
}

/// The addresses a list stands for: the state, whose, and the user's
/// query, one address per choice of its alternatives.
fn addresses(
    session: &Session<'_>,
    target: &Target,
    query: &PullQuery,
) -> Result<Vec<String>, ProviderError> {
    let state = match query.state {
        StateFilter::Open => "opened",
        StateFilter::Closed => "closed",
        StateFilter::Merged => "merged",
        StateFilter::All => "all",
    };
    let login = session.account.login.as_str();
    let whose = match query.filter {
        PullFilter::All => None,
        PullFilter::Mine => Some(("author_username", login)),
        PullFilter::Assigned => Some(("assignee_username", login)),
        PullFilter::ReviewRequested => Some(("reviewer_username", login)),
    };
    let terms = search::parse(query.query.as_deref().unwrap_or_default(), false)
        .map_err(ProviderError::Refused)?;
    let mut out = Vec::new();
    for words in search::expand(&terms).map_err(ProviderError::Refused)? {
        let mut parameters = vec![
            ("state".to_owned(), state.to_owned()),
            ("with_labels_details".to_owned(), "true".to_owned()),
        ];
        if let Some((key, value)) = whose {
            parameters.push((key.to_owned(), value.to_owned()));
        }
        for (key, value) in query_parameters(&words, login)? {
            parameters.retain(|(known, _)| *known != key);
            parameters.push((key, value));
        }
        let mut path = format!("{}/merge_requests?", project_path(target));
        for (key, value) in parameters {
            path.push_str(&format!("{key}={}&", api::encode(&value)));
        }
        path.push_str("per_page=100");
        out.push(path);
    }
    Ok(out)
}

/// A project's merge requests.
pub(super) fn list(
    session: &Session<'_>,
    target: &Target,
    query: &PullQuery,
) -> Result<Listing<PullRequest>, ProviderError> {
    let api = session.api()?;
    let addresses = addresses(session, target, query)?;
    let several = addresses.len() > 1;
    let mut wires: Vec<WireMerge> = Vec::new();
    let mut seen = BTreeSet::new();
    let mut more = false;
    let mut freshness = crate::model::Freshness::default();
    for path in addresses {
        let left = query.most.saturating_sub(wires.len());
        if left == 0 {
            more = true;
            break;
        }
        let (found, cut, response) =
            api.pages_where::<WireMerge>(&path, left, |wire| !seen.contains(&wire.iid))?;
        more |= cut;
        freshness = api::latest(freshness, fresh(session, &response));
        for wire in found {
            if seen.insert(wire.iid) {
                wires.push(wire);
            }
        }
    }
    let numbers: Vec<u64> = wires.iter().map(|wire| wire.iid).collect();
    let extra = match enrich(session, target, &numbers) {
        Ok(extra) => extra,
        Err(err @ ProviderError::Http(crate::http::FetchError::Cancelled)) => return Err(err),
        // An older GitLab, or one that refuses the question: the rows
        // without verdicts and pipelines.
        Err(_) => BTreeMap::new(),
    };
    // The forks' paths as GraphQL told them, and those it did not, asked.
    let mut forks: BTreeMap<u64, String> = BTreeMap::new();
    for wire in &wires {
        if let (Some(source), Some(path)) = (
            fork_of(wire),
            extra
                .get(&wire.iid)
                .and_then(|extra| extra.source_project.as_ref())
                .and_then(|project| project.full_path.as_deref()),
        ) {
            forks.insert(source, path.to_owned());
        }
    }
    let missing: Vec<&WireMerge> = wires
        .iter()
        .filter(|wire| fork_of(wire).is_some_and(|source| !forks.contains_key(&source)))
        .collect();
    if !missing.is_empty() {
        forks.extend(fork_paths(session, &missing)?);
    }
    let mut items: Vec<PullRequest> = wires
        .iter()
        .map(|wire| enriched(merge_request(wire, target, &forks), extra.get(&wire.iid)))
        .collect();
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
struct WireProjectSettings {
    #[serde(default)]
    merge_method: Option<String>,
    #[serde(default)]
    squash_option: Option<String>,
}

/// The ways a project allows merging: its merge method, and squashing
/// unless it is never allowed (or only squashing, when it always is).
fn merge_methods(settings: &WireProjectSettings) -> Vec<MergeMethod> {
    if settings.squash_option.as_deref() == Some("always") {
        return vec![MergeMethod::Squash];
    }
    let mut out = vec![match settings.merge_method.as_deref() {
        Some("ff") => MergeMethod::Rebase,
        _ => MergeMethod::Merge,
    }];
    if settings.squash_option.as_deref() != Some("never") {
        out.push(MergeMethod::Squash);
    }
    out
}

/// Whether it can be merged, from `detailed_merge_status`.
fn mergeable(wire: &WireMerge) -> Mergeable {
    if wire.has_conflicts {
        return Mergeable::Conflicts;
    }
    match wire.detailed_merge_status.as_deref() {
        Some("mergeable") => Mergeable::Clean,
        Some("conflict" | "broken_status") => Mergeable::Conflicts,
        Some("checking" | "unchecked" | "preparing" | "approvals_syncing") | None => {
            Mergeable::Unknown
        }
        Some(_) => Mergeable::Blocked,
    }
}

#[derive(Debug, Deserialize)]
struct WireCommit {
    id: String,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    message: Option<String>,
    #[serde(default)]
    author_name: Option<String>,
    #[serde(default)]
    authored_date: Option<String>,
}

fn commit(wire: &WireCommit) -> PullCommit {
    let summary = wire
        .title
        .as_deref()
        .or_else(|| {
            wire.message
                .as_deref()
                .and_then(|message| message.lines().next())
        })
        .unwrap_or_default();
    PullCommit {
        sha: api::clean_line(&wire.id, 64),
        summary: api::title(summary),
        author: None,
        author_name: api::clean_line(wire.author_name.as_deref().unwrap_or_default(), 256),
        date: api::clean_line(wire.authored_date.as_deref().unwrap_or_default(), 64),
    }
}

#[derive(Debug, Deserialize)]
struct WireDiff {
    #[serde(default)]
    old_path: Option<String>,
    new_path: String,
    #[serde(default)]
    new_file: bool,
    #[serde(default)]
    renamed_file: bool,
    #[serde(default)]
    deleted_file: bool,
    #[serde(default)]
    diff: Option<String>,
    #[serde(default)]
    too_large: Option<bool>,
}

fn file(wire: &WireDiff) -> PullFile {
    let status = if wire.new_file {
        "added"
    } else if wire.deleted_file {
        "removed"
    } else if wire.renamed_file {
        "renamed"
    } else {
        "modified"
    };
    let patch = wire
        .diff
        .as_deref()
        .filter(|diff| !diff.is_empty() && wire.too_large != Some(true));
    // GitLab counts no lines: they are counted in the patch.
    let count = |sign: char| {
        patch
            .map(|diff| {
                diff.lines()
                    .filter(|line| line.starts_with(sign))
                    .filter(|line| !line.starts_with("+++") && !line.starts_with("---"))
                    .count()
            })
            .and_then(|count| u64::try_from(count).ok())
            .unwrap_or(0)
    };
    PullFile {
        path: api::clean_line(&wire.new_path, 4096),
        previous_path: wire
            .old_path
            .as_deref()
            .filter(|old| wire.renamed_file && *old != wire.new_path)
            .map(|old| api::clean_line(old, 4096)),
        status: status.to_owned(),
        additions: count('+'),
        deletions: count('-'),
        patch: patch.map(api::body),
    }
}

#[derive(Debug, Deserialize)]
struct WirePosition {
    #[serde(default)]
    new_path: Option<String>,
    #[serde(default)]
    old_path: Option<String>,
    #[serde(default)]
    new_line: Option<u64>,
    #[serde(default)]
    old_line: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireNote {
    id: u64,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    author: Option<WireUser>,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
    #[serde(default)]
    system: bool,
    #[serde(default)]
    resolvable: bool,
    #[serde(default)]
    resolved: bool,
    #[serde(default)]
    position: Option<WirePosition>,
}

impl WireNote {
    /// Whether a person wrote it (GitLab's own notes say what changed).
    pub(super) fn written(&self) -> bool {
        !self.system
    }
}

pub(super) fn note(wire: &WireNote) -> Comment {
    let created = api::clean_line(&wire.created_at, 64);
    Comment {
        id: wire.id.to_string(),
        author: wire.author.as_ref().map(WireUser::user).unwrap_or_default(),
        body: api::body(wire.body.as_deref().unwrap_or_default()),
        updated: wire
            .updated_at
            .as_deref()
            .map(|updated| api::clean_line(updated, 64))
            .filter(|updated| *updated != created),
        created,
    }
}

#[derive(Debug, Deserialize)]
struct WireDiscussion {
    id: String,
    #[serde(default)]
    notes: Vec<WireNote>,
}

/// A discussion as a conversation; one of GitLab's own notes only is
/// left out.
fn thread(wire: &WireDiscussion) -> Option<Thread> {
    let written: Vec<&WireNote> = wire.notes.iter().filter(|note| note.written()).collect();
    let first = *written.first()?;
    let resolvable: Vec<&&WireNote> = written.iter().filter(|note| note.resolvable).collect();
    let position = first.position.as_ref();
    Some(Thread {
        id: api::clean_line(&wire.id, 128),
        path: position
            .and_then(|position| {
                position
                    .new_path
                    .as_deref()
                    .or(position.old_path.as_deref())
            })
            .map(|path| api::clean_line(path, 4096)),
        line: position.and_then(|position| position.new_line.or(position.old_line)),
        diff_hunk: None,
        resolved: !resolvable.is_empty() && resolvable.iter().all(|note| note.resolved),
        comments: written.into_iter().map(note).collect(),
    })
}

#[derive(Debug, Deserialize)]
struct WireApprover {
    user: WireUser,
}

#[derive(Debug, Deserialize)]
struct WireApprovals {
    #[serde(default)]
    approved_by: Vec<WireApprover>,
}

#[derive(Debug, Deserialize)]
struct WireReviewer {
    user: WireUser,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
}

/// One merge request with everything its view shows.
pub(super) fn show(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<PullDetails, ProviderError> {
    let api = session.api()?;
    let base = project_path(target);
    let one = format!("{base}/merge_requests/{number}");
    let (wire, first): (WireMerge, _) = api.json(Method::Get, &one, None)?;
    // How fresh it is, as its answers say: kept ones make all of it kept.
    let mut freshness = fresh(session, &first);
    let mut seen = |response: &crate::http::Response| {
        freshness = api::latest(freshness.clone(), fresh(session, response));
    };
    let (settings, answer): (WireProjectSettings, _) = api.json(Method::Get, &base, None)?;
    seen(&answer);
    let (commits, _, answer) =
        api.pages::<WireCommit>(&format!("{one}/commits?per_page=100"), MOST_COMMITS)?;
    seen(&answer);
    let (diffs, _, answer) =
        api.pages::<WireDiff>(&format!("{one}/diffs?per_page=100"), MOST_FILES)?;
    seen(&answer);
    let (discussions, _, answer) =
        api.pages::<WireDiscussion>(&format!("{one}/discussions?per_page=100"), MOST_DISCUSSIONS)?;
    seen(&answer);
    let (approvals, answer): (WireApprovals, _) =
        api.json(Method::Get, &format!("{one}/approvals"), None)?;
    seen(&answer);
    let (asked, _, answer) =
        api.pages::<WireReviewer>(&format!("{one}/reviewers?per_page=100"), 100)?;
    seen(&answer);
    let head = wire.sha.clone().unwrap_or_default();
    let checks = if head.is_empty() {
        Vec::new()
    } else {
        checks::of_commit(session, target, &head)?
    };
    let forks = match fork_of(&wire) {
        Some(_) => fork_paths(session, &[&wire])?,
        None => BTreeMap::new(),
    };
    let mut pull = merge_request(&wire, target, &forks);
    // Each reviewer's verdict, and who approved without being asked.
    pull.reviewers = asked
        .iter()
        .map(|reviewer| Reviewer {
            user: reviewer.user.user(),
            state: review_state(reviewer.state.as_deref().unwrap_or_default()),
        })
        .collect();
    let mut reviews: Vec<Review> = Vec::new();
    for approver in &approvals.approved_by {
        let user = approver.user.user();
        match pull
            .reviewers
            .iter_mut()
            .find(|reviewer| reviewer.user.login == user.login)
        {
            Some(reviewer) => reviewer.state = ReviewState::Approved,
            None => pull.reviewers.push(Reviewer {
                user: user.clone(),
                state: ReviewState::Approved,
            }),
        }
        reviews.push(Review {
            id: format!("approval-{}", approver.user.id.unwrap_or_default()),
            author: user,
            state: ReviewState::Approved,
            body: String::new(),
            submitted: None,
        });
    }
    for reviewer in &asked {
        if review_state(reviewer.state.as_deref().unwrap_or_default())
            == ReviewState::ChangesRequested
        {
            reviews.push(Review {
                id: format!("changes-{}", reviewer.user.id.unwrap_or_default()),
                author: reviewer.user.user(),
                state: ReviewState::ChangesRequested,
                body: String::new(),
                submitted: reviewer
                    .created_at
                    .as_deref()
                    .map(|at| api::clean_line(at, 64)),
            });
        }
    }
    pull.ci = CiState::sum(checks.iter().map(|check| check.state));
    let mut threads: Vec<Thread> = discussions.iter().filter_map(thread).collect();
    threads.sort_by(|a, b| {
        let start = |thread: &Thread| {
            thread
                .comments
                .first()
                .map(|comment| comment.created.clone())
                .unwrap_or_default()
        };
        start(a).cmp(&start(b))
    });
    Ok(PullDetails {
        body: api::body(wire.description.as_deref().unwrap_or_default()),
        milestone: wire.milestone.as_ref().map(milestone),
        mergeable: mergeable(&wire),
        merge_methods: merge_methods(&settings),
        checks,
        commits: commits.iter().map(commit).collect(),
        files: diffs.iter().map(file).collect(),
        reviews,
        threads,
        pull,
        freshness,
    })
}

/// What may be chosen for a merge request.
pub(super) fn options(
    session: &Session<'_>,
    target: &Target,
) -> Result<PullOptions, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireMember {
        #[serde(flatten)]
        user: WireUser,
        #[serde(default)]
        state: Option<String>,
    }
    let api = session.api()?;
    let base = project_path(target);
    let (labels, _, _) = api.pages::<WireLabel>(
        &format!("{base}/labels?include_ancestor_groups=true&per_page=100"),
        MOST_CHOICES,
    )?;
    let (milestones, _, _) = api.pages::<WireMilestone>(
        &format!("{base}/milestones?state=active&include_ancestors=true&per_page=100"),
        MOST_CHOICES,
    )?;
    let (members, _, _) =
        api.pages::<WireMember>(&format!("{base}/members/all?per_page=100"), MOST_CHOICES)?;
    let (settings, _): (WireProjectSettings, _) = api.json(Method::Get, &base, None)?;
    let mut people: Vec<User> = Vec::new();
    for member in &members {
        if member
            .state
            .as_deref()
            .is_some_and(|state| state != "active")
        {
            continue;
        }
        let user = member.user.user();
        if !people.iter().any(|known| known.login == user.login) {
            people.push(user);
        }
    }
    Ok(PullOptions {
        labels: labels.iter().map(label).collect(),
        milestones: milestones.iter().map(milestone).collect(),
        people,
        merge_methods: merge_methods(&settings),
    })
}

/// The open merge request whose branch is `branch` in the project
/// itself.
pub(super) fn for_branch(
    session: &Session<'_>,
    target: &Target,
    branch: &str,
) -> Result<Option<PullRequest>, ProviderError> {
    let path = format!(
        "{}/merge_requests?state=opened&source_branch={}&with_labels_details=true&per_page=100",
        project_path(target),
        api::encode(branch)
    );
    let (found, _, _) = session.api()?.pages::<WireMerge>(&path, 100)?;
    Ok(found
        .iter()
        .filter(|wire| fork_of(wire).is_none())
        .map(|wire| merge_request(wire, target, &BTreeMap::new()))
        .find(|pull| pull.head_ref == branch))
}

/// The stack a merge request is in, from the project's open merge
/// requests.
pub(super) fn stack(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<Stack, ProviderError> {
    let api = session.api()?;
    let base = project_path(target);
    // Only whether one comes from a fork matters here, not which.
    let stacked = |wire: &WireMerge| {
        let mut pull = merge_request(wire, target, &BTreeMap::new());
        pull.head_repo = fork_of(wire).map(|source| source.to_string());
        pull
    };
    let (open, _, _) = api.pages::<WireMerge>(
        &format!("{base}/merge_requests?state=opened&per_page=100"),
        provider::MOST_STACKED,
    )?;
    let mut pulls: Vec<PullRequest> = open.iter().map(stacked).collect();
    if !pulls.iter().any(|pull| pull.number == number) {
        let (wire, _): (WireMerge, _) = api.json(
            Method::Get,
            &format!("{base}/merge_requests/{number}"),
            None,
        )?;
        pulls.push(stacked(&wire));
    }
    provider::stack_of(&pulls, number).ok_or(ProviderError::NotFound)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn single(negated: bool, key: Option<&str>, value: &str) -> search::Single {
        search::Single {
            negated,
            key: key.map(str::to_owned),
            value: value.to_owned(),
        }
    }

    #[test]
    fn a_query_is_made_into_gitlab_parameters() {
        let parameters = query_parameters(
            &[
                single(false, Some("author"), "@me"),
                single(true, Some("label"), "wip"),
                single(false, Some("label"), "bug"),
                single(false, Some("label"), "ui"),
                single(false, Some("draft"), "false"),
                single(false, None, "fix"),
                single(false, None, "crash"),
                single(false, Some("base"), "main"),
            ],
            "alice",
        )
        .unwrap();
        assert_eq!(
            parameters,
            [
                ("author_username".to_owned(), "alice".to_owned()),
                ("wip".to_owned(), "no".to_owned()),
                ("target_branch".to_owned(), "main".to_owned()),
                ("labels".to_owned(), "bug,ui".to_owned()),
                ("not[labels]".to_owned(), "wip".to_owned()),
                ("search".to_owned(), "fix crash".to_owned()),
            ]
        );
        let err = query_parameters(
            &[
                single(false, Some("involves"), "a"),
                single(false, Some("review"), "approved"),
                single(true, Some("base"), "main"),
            ],
            "alice",
        )
        .unwrap_err()
        .to_string();
        assert!(err.contains("-base, involves, review"), "{err}");
    }

    #[test]
    fn merge_methods_follow_the_project() {
        let settings = |method: &str, squash: &str| WireProjectSettings {
            merge_method: Some(method.to_owned()),
            squash_option: Some(squash.to_owned()),
        };
        assert_eq!(
            merge_methods(&settings("merge", "default_off")),
            [MergeMethod::Merge, MergeMethod::Squash]
        );
        assert_eq!(
            merge_methods(&settings("ff", "never")),
            [MergeMethod::Rebase]
        );
        assert_eq!(
            merge_methods(&settings("rebase_merge", "always")),
            [MergeMethod::Squash]
        );
    }

    #[test]
    fn lines_are_counted_in_the_patch() {
        let diff = WireDiff {
            old_path: Some("a".into()),
            new_path: "b".into(),
            new_file: false,
            renamed_file: true,
            deleted_file: false,
            diff: Some("@@ -1,2 +1,2 @@\n-old\n+new\n+more\n context\n".into()),
            too_large: None,
        };
        let file = file(&diff);
        assert_eq!((file.additions, file.deletions), (2, 1));
        assert_eq!(file.status, "renamed");
        assert_eq!(file.previous_path.as_deref(), Some("a"));
    }
}
