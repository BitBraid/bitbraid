//! The checks of commits at GitHub: one GraphQL question for up to a
//! hundred commits, each commit an aliased `object` asked for its
//! `statusCheckRollup`, whose contexts are check runs (GitHub Actions and
//! other apps) and statuses that services set.
//!
//! Sources: docs.github.com/en/graphql "Objects" (`Repository.object`,
//! `Commit.statusCheckRollup`, `StatusCheckRollup`, `CheckRun`,
//! `StatusContext`), "Enums" (`CheckStatusState`, `CheckConclusionState`,
//! `StatusState`), "Forming calls with GraphQL" (aliases, fragments,
//! variables); docs.github.com/en/rest "Commit statuses".

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{Map, Value};

use crate::{
    api::{self, ProviderError},
    model::{Check, CiState, CommitStatus},
    provider::{Session, Target},
};

/// What GraphQL is asked of each commit: its checks, at most a hundred.
pub(super) const CONTEXTS: &str = "statusCheckRollup { contexts(first: 100) { nodes { \
__typename \
... on CheckRun { name status conclusion detailsUrl startedAt completedAt title } \
... on StatusContext { context state targetUrl description createdAt } } } }";

/// A check run or a status, as GraphQL answers.
#[derive(Debug, Deserialize)]
pub(super) struct WireContext {
    #[serde(default, rename = "__typename")]
    typename: Option<String>,
    // A check run.
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default, rename = "detailsUrl")]
    details_url: Option<String>,
    #[serde(default, rename = "startedAt")]
    started_at: Option<String>,
    #[serde(default, rename = "completedAt")]
    completed_at: Option<String>,
    #[serde(default)]
    title: Option<String>,
    // A status.
    #[serde(default)]
    context: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default, rename = "targetUrl")]
    target_url: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "createdAt")]
    created_at: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub(super) struct WireNodes<T> {
    #[serde(default = "Vec::new")]
    pub(super) nodes: Vec<Option<T>>,
}

#[derive(Debug, Deserialize)]
pub(super) struct WireRollup {
    #[serde(default)]
    contexts: Option<WireNodes<WireContext>>,
}

/// A commit, or another object, as `object(expression:)` answers.
#[derive(Debug, Deserialize)]
pub(super) struct WireObject {
    #[serde(default, rename = "statusCheckRollup")]
    pub(super) rollup: Option<WireRollup>,
}

fn some(text: Option<&str>, max: usize) -> Option<String> {
    text.map(|text| api::clean_line(text, max))
        .filter(|text| !text.trim().is_empty())
}

/// How a check run stands.
fn run_state(status: Option<&str>, conclusion: Option<&str>) -> CiState {
    match status {
        Some("COMPLETED") | None => match conclusion {
            Some("SUCCESS") => CiState::Success,
            Some("FAILURE" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED") => {
                CiState::Failure
            }
            Some("CANCELLED") => CiState::Cancelled,
            Some("SKIPPED") => CiState::Skipped,
            Some("NEUTRAL" | "STALE") => CiState::Neutral,
            _ => CiState::None,
        },
        Some("IN_PROGRESS") => CiState::Running,
        Some(_) => CiState::Pending,
    }
}

/// How a status stands.
fn status_state(state: Option<&str>) -> CiState {
    match state {
        Some("SUCCESS") => CiState::Success,
        Some("FAILURE" | "ERROR") => CiState::Failure,
        Some("PENDING" | "EXPECTED") => CiState::Pending,
        _ => CiState::None,
    }
}

/// A context as the model has it; one of an unknown kind is left out.
fn check(wire: &WireContext) -> Option<Check> {
    let status = wire.typename.as_deref() == Some("StatusContext")
        || (wire.typename.is_none() && wire.context.is_some());
    if status {
        return Some(Check {
            name: api::clean_line(wire.context.as_deref().unwrap_or_default(), 256),
            state: status_state(wire.state.as_deref()),
            url: some(wire.target_url.as_deref(), 2048),
            description: some(wire.description.as_deref(), 1024),
            started: some(wire.created_at.as_deref(), 64),
            finished: None,
        });
    }
    if wire
        .typename
        .as_deref()
        .is_some_and(|kind| kind != "CheckRun")
    {
        return None;
    }
    Some(Check {
        name: api::clean_line(wire.name.as_deref().unwrap_or_default(), 256),
        state: run_state(wire.status.as_deref(), wire.conclusion.as_deref()),
        url: some(wire.details_url.as_deref(), 2048),
        description: some(wire.title.as_deref(), 1024),
        started: some(wire.started_at.as_deref(), 64),
        finished: some(wire.completed_at.as_deref(), 64),
    })
}

/// The checks of a commit's rollup.
pub(super) fn checks(rollup: Option<&WireRollup>) -> Vec<Check> {
    rollup
        .and_then(|rollup| rollup.contexts.as_ref())
        .map(|contexts| contexts.nodes.iter().flatten().filter_map(check).collect())
        .unwrap_or_default()
}

/// The checks of `shas` in `target`, in the order asked; a commit the
/// provider does not know has none.
pub(super) fn status(
    session: &Session<'_>,
    target: &Target,
    shas: &[String],
) -> Result<Vec<CommitStatus>, ProviderError> {
    if shas.is_empty() {
        return Ok(Vec::new());
    }
    let mut declared = vec!["$owner: String!".to_owned(), "$name: String!".to_owned()];
    let mut asked = Vec::new();
    let mut variables = Map::new();
    variables.insert("owner".to_owned(), Value::String(target.owner.clone()));
    variables.insert("name".to_owned(), Value::String(target.name.clone()));
    for (at, sha) in shas.iter().enumerate() {
        declared.push(format!("$c{at}: String!"));
        asked.push(format!("c{at}: object(expression: $c{at}) {{ ...Checks }}"));
        variables.insert(format!("c{at}"), Value::String(sha.clone()));
    }
    let query = format!(
        "query({}) {{ repository(owner: $owner, name: $name) {{ {} }} }} \
         fragment Checks on Commit {{ {CONTEXTS} }}",
        declared.join(", "),
        asked.join(" ")
    );
    #[derive(Debug, Deserialize)]
    struct Data {
        repository: Option<BTreeMap<String, Option<WireObject>>>,
    }
    let (data, _): (Data, _) = session
        .graphql()?
        .graphql(&query, &Value::Object(variables))?;
    let objects = data.repository.ok_or(ProviderError::NotFound)?;
    Ok(shas
        .iter()
        .enumerate()
        .map(|(at, sha)| {
            let checks = checks(
                objects
                    .get(&format!("c{at}"))
                    .and_then(Option::as_ref)
                    .and_then(|object| object.rollup.as_ref()),
            );
            CommitStatus {
                sha: sha.clone(),
                state: CiState::sum(checks.iter().map(|check| check.state)),
                checks,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_runs_and_statuses_are_read_into_states() {
        assert_eq!(run_state(Some("IN_PROGRESS"), None), CiState::Running);
        assert_eq!(run_state(Some("QUEUED"), None), CiState::Pending);
        assert_eq!(run_state(Some("WAITING"), None), CiState::Pending);
        assert_eq!(
            run_state(Some("COMPLETED"), Some("TIMED_OUT")),
            CiState::Failure
        );
        assert_eq!(
            run_state(Some("COMPLETED"), Some("SKIPPED")),
            CiState::Skipped
        );
        assert_eq!(run_state(Some("COMPLETED"), Some("ODD")), CiState::None);
        assert_eq!(status_state(Some("EXPECTED")), CiState::Pending);
        assert_eq!(status_state(Some("ERROR")), CiState::Failure);
        assert_eq!(status_state(None), CiState::None);
    }
}
