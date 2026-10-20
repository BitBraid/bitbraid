//! The checks of commits at GitLab: the statuses of each commit, which
//! are the jobs of its pipelines and the statuses other services set,
//! latest of each name; a few commits asked at once.
//!
//! The pipelines of a commit are not asked for: its statuses are what
//! they ran, and a commit without them has no checks.
//!
//! Sources: docs.gitlab.com/api "Commits" (list the statuses of a
//! commit: `name`, `status`, `target_url`, `description`, `started_at`,
//! `finished_at`, `allow_failure`), "Pipelines" (the statuses a pipeline
//! and a job may have).

use serde::Deserialize;

use super::{at_once, ci_state, project_path};
use crate::{
    api::{self, ProviderError},
    model::{Check, CiState, CommitStatus},
    provider::{Session, Target},
};

/// Most statuses of one commit read.
const MOST_STATUSES: usize = 300;

#[derive(Debug, Deserialize)]
pub(super) struct WireStatus {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    target_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    started_at: Option<String>,
    #[serde(default)]
    finished_at: Option<String>,
    #[serde(default)]
    created_at: Option<String>,
    #[serde(default)]
    allow_failure: bool,
}

fn some(text: Option<&str>, max: usize) -> Option<String> {
    text.map(|text| api::clean_line(text, max))
        .filter(|text| !text.trim().is_empty())
}

pub(super) fn check(wire: &WireStatus) -> Check {
    Check {
        name: api::clean_line(wire.name.as_deref().unwrap_or_default(), 256),
        state: ci_state(
            wire.status.as_deref().unwrap_or_default(),
            wire.allow_failure,
        ),
        url: some(wire.target_url.as_deref(), 2048),
        description: some(wire.description.as_deref(), 1024),
        started: some(
            wire.started_at.as_deref().or(wire.created_at.as_deref()),
            64,
        ),
        finished: some(wire.finished_at.as_deref(), 64),
    }
}

/// The checks of one commit; one GitLab does not know has none.
pub(super) fn of_commit(
    session: &Session<'_>,
    target: &Target,
    sha: &str,
) -> Result<Vec<Check>, ProviderError> {
    let path = format!(
        "{}/repository/commits/{}/statuses?per_page=100",
        project_path(target),
        api::encode(sha)
    );
    match session.api()?.pages::<WireStatus>(&path, MOST_STATUSES) {
        Ok((statuses, _, _)) => Ok(statuses.iter().map(check).collect()),
        Err(ProviderError::NotFound) => Ok(Vec::new()),
        Err(err) => Err(err),
    }
}

/// The checks of `shas` in `target`, in the order asked.
pub(super) fn status(
    session: &Session<'_>,
    target: &Target,
    shas: &[String],
) -> Result<Vec<CommitStatus>, ProviderError> {
    at_once(shas, |sha| {
        let checks = of_commit(session, target, sha)?;
        Ok(CommitStatus {
            sha: sha.clone(),
            state: CiState::sum(checks.iter().map(|check| check.state)),
            checks,
        })
    })
}
