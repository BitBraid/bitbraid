//! GitHub, through its REST API (v3), at github.com (`api.github.com`) or
//! at a GitHub Enterprise Server (`https://host/api/v3`), and its GraphQL
//! API (v4) beside it where REST would ask once per item: a list of pull
//! requests with their checks and reviews, the resolved state of review
//! conversations, the checks of many commits at once.
//!
//! Who a token belongs to and what it was granted; the reads of
//! [`Provider`]: repositories and where one may be made ([`repos`]), pull
//! requests ([`pulls`]), issues ([`issues`]), checks ([`checks`]) and the
//! pictures of commit authors; the writes of pull requests
//! ([`pulls::writes`]) and of repositories, forks and SSH keys
//! (`repos::writes`); and the inbox's searches across the account
//! ([`inbox`]).
//!
//! Sources: docs.github.com/en/rest "Users" (`GET /user`), "Scopes for
//! OAuth apps" (the `X-OAuth-Scopes` line), "Pull requests", "Using
//! pagination in the REST API"; docs.github.com "Checking out pull
//! requests locally" (`refs/pull/N/head`); docs.github.com/en/graphql
//! "Forming calls with GraphQL", "Queries" (`search`, `repository`,
//! `object`), "Objects" (`PullRequest`, `StatusCheckRollup`, `CheckRun`,
//! `StatusContext`, `PullRequestReviewThread`), "Enums" (`StatusState`,
//! `CheckStatusState`, `CheckConclusionState`, `PullRequestReviewState`).

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
        Comment, CommitStatus, HostedRepo, Issue, IssueDetails, Label, Listing, Merged, Milestone,
        Namespace, ProviderKind, PullDetails, PullOptions, PullRequest, Review, SshKey, Stack,
        User,
    },
    provider::{
        Identity, IssueQuery, LineComment, MergeRequest, NewPull, NewRepo, Provider, PullChange,
        PullQuery, RepoQuery, ReviewEvent, Session, Target,
    },
};

#[derive(Debug, Clone, Copy)]
pub struct Github;

#[derive(Debug, Deserialize)]
struct WireUser {
    login: String,
    #[serde(default)]
    id: Option<u64>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    avatar_url: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
}

impl WireUser {
    fn user(&self) -> User {
        api::user(
            &self.login,
            self.name.as_deref(),
            self.avatar_url.as_deref(),
            self.html_url.as_deref(),
        )
    }
}

/// A person as GraphQL names them.
#[derive(Debug, Deserialize)]
struct GraphUser {
    #[serde(default)]
    login: Option<String>,
    /// A team's `org/team`.
    #[serde(default, rename = "combinedSlug")]
    combined_slug: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default, rename = "avatarUrl")]
    avatar_url: Option<String>,
    #[serde(default)]
    url: Option<String>,
}

impl GraphUser {
    fn user(&self) -> User {
        let login = self
            .login
            .as_deref()
            .or(self.combined_slug.as_deref())
            .unwrap_or_default();
        api::user(
            login,
            self.name.as_deref(),
            self.avatar_url.as_deref(),
            self.url.as_deref(),
        )
    }
}

#[derive(Debug, Deserialize)]
struct WireLabel {
    name: String,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

/// A label as the model has it: the colour only when it is six
/// hexadecimal digits.
fn label(wire: &WireLabel) -> Label {
    Label {
        name: api::clean_line(&wire.name, 256),
        color: wire
            .color
            .as_deref()
            .map(|color| color.trim_start_matches('#'))
            .filter(|color| color.len() == 6 && color.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_ascii_lowercase),
        description: wire
            .description
            .as_deref()
            .map(|text| api::clean_line(text, 1024))
            .filter(|text| !text.trim().is_empty()),
    }
}

#[derive(Debug, Deserialize)]
struct WireMilestone {
    number: u64,
    title: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    due_on: Option<String>,
}

fn milestone(wire: &WireMilestone) -> Milestone {
    Milestone {
        id: wire.number.to_string(),
        title: api::title(&wire.title),
        due: wire.due_on.as_deref().map(|due| api::clean_line(due, 64)),
        open: wire.state.as_deref() != Some("closed"),
    }
}

/// `/repos/owner/name`.
fn repo_path(target: &Target) -> String {
    format!(
        "/repos/{}/{}",
        api::encode(&target.owner),
        api::encode(&target.name)
    )
}

/// What every answer's header lines say about the allowance, with the
/// clock of the session.
fn fresh(session: &Session<'_>, response: &crate::http::Response) -> crate::model::Freshness {
    api::freshness(response, session.now)
}

impl Provider for Github {
    fn kind(&self) -> ProviderKind {
        ProviderKind::Github
    }

    fn whoami(&self, session: &Session<'_>) -> Result<Identity, ProviderError> {
        let (me, response): (WireUser, _) = session.api()?.json(Method::Get, "/user", None)?;
        let scopes = response
            .header("x-oauth-scopes")
            .map(|scopes| {
                scopes
                    .split(',')
                    .map(|scope| api::clean_line(scope.trim(), 64))
                    .filter(|scope| !scope.is_empty())
                    .take(64)
                    .collect()
            })
            .unwrap_or_default();
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
        format!("refs/pull/{number}/head")
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
        target: Option<&Target>,
        emails: &[String],
    ) -> Result<BTreeMap<String, String>, ProviderError> {
        repos::avatars(session, target, emails)
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
