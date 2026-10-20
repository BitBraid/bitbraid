//! GitLab, through its REST API (v4), at gitlab.com or a GitLab of one's
//! own (`https://host/api/v4`), and its GraphQL API beside it for what
//! REST would ask once per merge request: the verdicts of a list's
//! reviewers and the state of each one's pipeline. A merge request is a
//! [`crate::model::PullRequest`] here, its `iid` the number; a project is
//! asked for by its path, URL-encoded.
//!
//! Who a token belongs to and what it was granted; the reads of
//! [`Provider`]: projects and where one may be made ([`repos`]), merge
//! requests ([`pulls`]), issues ([`issues`]), the statuses of commits
//! ([`checks`]) and the pictures of addresses; the writes of merge requests
//! ([`pulls::writes`]) and of projects, forks and SSH keys
//! (`repos::writes`); and the inbox's lists across the account ([`inbox`]).
//!
//! Sources: docs.gitlab.com/api "Users" (`GET /user`), "Personal access
//! tokens" (`GET /personal_access_tokens/self`), "Merge requests", "REST
//! API" (namespaced paths as ids, pagination); docs.gitlab.com "Merge
//! requests: checkout locally" (`refs/merge-requests/N/head`);
//! docs.gitlab.com/api/graphql "Reference" (`Project.mergeRequests`,
//! `MergeRequest.headPipeline`, `MergeRequestReviewer`,
//! `MergeRequestReviewState`, `PipelineStatusEnum`).

mod checks;
mod inbox;
mod issues;
mod pulls;
mod repos;

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{
        CiState, Comment, CommitStatus, HostedRepo, Issue, IssueDetails, Label, Listing, Merged,
        Milestone, Namespace, ProviderKind, PullDetails, PullOptions, PullRequest, Review, SshKey,
        Stack, User,
    },
    provider::{
        Identity, IssueQuery, LineComment, MergeRequest, NewPull, NewRepo, Provider, PullChange,
        PullQuery, RepoQuery, ReviewEvent, Session, Target,
    },
};

#[derive(Debug, Clone, Copy)]
pub struct Gitlab;

#[derive(Debug, Deserialize)]
struct WireUser {
    username: String,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    web_url: Option<String>,
}

impl WireUser {
    fn user(&self) -> User {
        api::user(
            &self.username,
            self.name.as_deref(),
            self.avatar_url.as_deref(),
            self.web_url.as_deref(),
        )
    }
}

#[derive(Debug, Deserialize)]
struct WireToken {
    #[serde(default)]
    scopes: Vec<String>,
}

/// A label: an object with its details, or only its name (where a list
/// was not asked for the details).
#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum WireLabel {
    Detailed {
        name: String,
        #[serde(default)]
        color: Option<String>,
        #[serde(default)]
        description: Option<String>,
    },
    Named(String),
}

/// A label: GitLab writes its colour as `#rrggbb`.
fn label(wire: &WireLabel) -> Label {
    let (name, color, description) = match wire {
        WireLabel::Detailed {
            name,
            color,
            description,
        } => (name.as_str(), color.as_deref(), description.as_deref()),
        WireLabel::Named(name) => (name.as_str(), None, None),
    };
    Label {
        name: api::clean_line(name, 256),
        color: color
            .map(|color| color.trim_start_matches('#'))
            .filter(|color| color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase),
        description: description
            .map(|text| api::clean_line(text, 1024))
            .filter(|text| !text.trim().is_empty()),
    }
}

#[derive(Debug, Deserialize)]
struct WireMilestone {
    id: u64,
    title: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    due_date: Option<String>,
}

fn milestone(wire: &WireMilestone) -> Milestone {
    Milestone {
        id: wire.id.to_string(),
        title: api::title(&wire.title),
        due: wire.due_date.as_deref().map(|due| api::clean_line(due, 64)),
        open: wire.state.as_deref() != Some("closed"),
    }
}

/// How a pipeline or a job stands, as REST (lower case) or GraphQL
/// (upper case) names it; a job that may fail and did is neutral.
fn ci_state(status: &str, allowed_to_fail: bool) -> CiState {
    match status.to_ascii_lowercase().as_str() {
        "success" => CiState::Success,
        "failed" if allowed_to_fail => CiState::Neutral,
        "failed" => CiState::Failure,
        "running" | "canceling" => CiState::Running,
        "created"
        | "waiting_for_resource"
        | "preparing"
        | "waiting_for_callback"
        | "pending"
        | "scheduled" => CiState::Pending,
        "canceled" | "cancelled" => CiState::Cancelled,
        "skipped" => CiState::Skipped,
        "manual" => CiState::Neutral,
        _ => CiState::None,
    }
}

/// The path a project is asked for by: `group%2Fsub%2Fname`.
pub fn project(target: &Target) -> String {
    api::encode(&target.full_name())
}

/// `/projects/group%2Fsub%2Fname`.
fn project_path(target: &Target) -> String {
    format!("/projects/{}", project(target))
}

fn fresh(session: &Session<'_>, response: &crate::http::Response) -> crate::model::Freshness {
    api::freshness(response, session.now)
}

/// Ask a few things at once, at most four at a time, each answer in the
/// place of what was asked; the first error, if one comes, is the
/// answer.
fn at_once<T: Sync, R: Send>(
    items: &[T],
    ask: impl Fn(&T) -> Result<R, ProviderError> + Sync,
) -> Result<Vec<R>, ProviderError> {
    const WORKERS: usize = 4;
    let chunk = items.len().div_ceil(WORKERS).max(1);
    let answers: Vec<Result<Vec<R>, ProviderError>> = std::thread::scope(|scope| {
        let handles: Vec<_> = items
            .chunks(chunk)
            .map(|part| scope.spawn(|| part.iter().map(&ask).collect::<Result<Vec<R>, _>>()))
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle.join().unwrap_or_else(|_| {
                    Err(ProviderError::Refused(
                        say!("gitlab-request-failed-on-thread").to_owned(),
                    ))
                })
            })
            .collect()
    });
    let mut out = Vec::with_capacity(items.len());
    for part in answers {
        out.extend(part?);
    }
    Ok(out)
}

impl Provider for Gitlab {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Gitlab
    }

    fn whoami(&self, session: &Session<'_>) -> Result<Identity, ProviderError> {
        let api = session.api()?;
        let (me, _): (WireUser, _) = api.json(Method::Get, "/user", None)?;
        // What a personal, group or project token was granted. A token of
        // the device flow is not one of those and is answered "not found"
        // (GitLab 19 answers 400, "requires token type to be a personal
        // access token"): its scopes are those it was given with.
        let scopes = match api.json::<WireToken>(Method::Get, "/personal_access_tokens/self", None)
        {
            Ok((token, _)) => token
                .scopes
                .iter()
                .map(|scope| api::clean_line(scope, 64))
                .take(64)
                .collect(),
            Err(ProviderError::NotFound | ProviderError::Unauthorized(_)) => Vec::new(),
            Err(ProviderError::Status {
                status: 400 | 403, ..
            }) => Vec::new(),
            Err(err) => return Err(err),
        };
        let user = me.user();
        Ok(Identity {
            login: user.login,
            id: me.id.map(|id| id.to_string()).unwrap_or_default(),
            name: user.name,
            email: me.email.as_deref().map(|email| api::clean_line(email, 256)),
            avatar_url: user.avatar_url,
            scopes,
        })
    }

    fn pull_ref(&self, number: u64) -> String {
        format!("refs/merge-requests/{number}/head")
    }

    fn repos(
        &self,
        session: &Session<'_>,
        query: &RepoQuery,
    ) -> Result<Listing<HostedRepo>, ProviderError> {
        repos::list(session, query)
    }

    fn repo_get(
        &self,
        session: &Session<'_>,
        target: &Target,
    ) -> Result<HostedRepo, ProviderError> {
        repos::get(session, target)
    }

    fn repo_create(
        &self,
        session: &Session<'_>,
        repo: &NewRepo,
    ) -> Result<HostedRepo, ProviderError> {
        repos::writes::create(session, repo)
    }

    fn fork(
        &self,
        session: &Session<'_>,
        target: &Target,
        namespace: Option<&str>,
    ) -> Result<HostedRepo, ProviderError> {
        repos::writes::fork(session, target, namespace)
    }

    fn ssh_key_add(
        &self,
        session: &Session<'_>,
        title: &str,
        key: &str,
    ) -> Result<SshKey, ProviderError> {
        repos::writes::key_add(session, title, key)
    }

    fn namespaces(&self, session: &Session<'_>) -> Result<Vec<Namespace>, ProviderError> {
        repos::namespaces(session)
    }

    fn avatars(
        &self,
        session: &Session<'_>,
        _target: Option<&Target>,
        emails: &[String],
    ) -> Result<BTreeMap<String, String>, ProviderError> {
        repos::avatars(session, emails)
    }

    fn pr_list(
        &self,
        session: &Session<'_>,
        target: &Target,
        query: &PullQuery,
    ) -> Result<Listing<PullRequest>, ProviderError> {
        pulls::list(session, target, query)
    }

    fn pr_show(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<PullDetails, ProviderError> {
        pulls::show(session, target, number)
    }

    fn pr_get(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<PullRequest, ProviderError> {
        pulls::writes::get(session, target, number)
    }

    fn pr_create(
        &self,
        session: &Session<'_>,
        target: &Target,
        pull: &NewPull,
    ) -> Result<PullRequest, ProviderError> {
        pulls::writes::create(session, target, pull)
    }

    fn pr_update(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        change: &PullChange,
    ) -> Result<PullDetails, ProviderError> {
        pulls::writes::update(session, target, number, change)
    }

    fn pr_merge(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        merge: &MergeRequest,
    ) -> Result<Merged, ProviderError> {
        pulls::writes::merge(session, target, number, merge)
    }

    fn pr_review(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        event: ReviewEvent,
        body: Option<&str>,
    ) -> Result<Review, ProviderError> {
        pulls::writes::review(session, target, number, event, body)
    }

    fn pr_comment(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        body: &str,
        reply_to: Option<&str>,
    ) -> Result<Comment, ProviderError> {
        pulls::writes::comment_on(session, target, number, body, reply_to)
    }

    fn pr_line_comment(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
        comment: &LineComment,
    ) -> Result<Comment, ProviderError> {
        pulls::writes::comment_on_line(session, target, number, comment)
    }

    fn pr_options(
        &self,
        session: &Session<'_>,
        target: &Target,
    ) -> Result<PullOptions, ProviderError> {
        pulls::options(session, target)
    }

    fn pr_stack(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<Stack, ProviderError> {
        pulls::stack(session, target, number)
    }

    fn pr_for_branch(
        &self,
        session: &Session<'_>,
        target: &Target,
        branch: &str,
    ) -> Result<Option<PullRequest>, ProviderError> {
        pulls::for_branch(session, target, branch)
    }

    fn issue_list(
        &self,
        session: &Session<'_>,
        target: &Target,
        query: &IssueQuery,
    ) -> Result<Listing<Issue>, ProviderError> {
        issues::list(session, target, query)
    }

    fn issue_show(
        &self,
        session: &Session<'_>,
        target: &Target,
        number: u64,
    ) -> Result<IssueDetails, ProviderError> {
        issues::show(session, target, number)
    }

    fn inbox(
        &self,
        session: &Session<'_>,
        query: &crate::provider::InboxQuery,
    ) -> Result<crate::provider::Found, ProviderError> {
        inbox::inbox(session, query)
    }

    fn ci_status(
        &self,
        session: &Session<'_>,
        target: &Target,
        shas: &[String],
    ) -> Result<Vec<CommitStatus>, ProviderError> {
        checks::status(session, target, shas)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipelines_and_jobs_are_read_into_states() {
        assert_eq!(ci_state("SUCCESS", false), CiState::Success);
        assert_eq!(ci_state("failed", false), CiState::Failure);
        assert_eq!(ci_state("failed", true), CiState::Neutral);
        assert_eq!(ci_state("WAITING_FOR_RESOURCE", false), CiState::Pending);
        assert_eq!(ci_state("canceled", false), CiState::Cancelled);
        assert_eq!(ci_state("manual", false), CiState::Neutral);
        assert_eq!(ci_state("odd", false), CiState::None);
    }

    #[test]
    fn several_things_are_asked_at_once_in_order() {
        let items: Vec<u64> = (0..10).collect();
        let doubled = at_once(&items, |item| Ok(item * 2)).unwrap();
        assert_eq!(doubled, (0..10).map(|item| item * 2).collect::<Vec<_>>());
        assert!(
            at_once(&items, |item| if *item == 7 {
                Err(ProviderError::NotFound)
            } else {
                Ok(*item)
            })
            .is_err()
        );
        let none: Vec<u64> = at_once(&[] as &[u64], |item| Ok(*item)).unwrap();
        assert!(none.is_empty());
    }
}
