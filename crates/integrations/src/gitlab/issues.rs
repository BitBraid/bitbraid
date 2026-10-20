//! Issues at GitLab: a project's list, and one issue with its comments.
//!
//! The list's query: `@alice` or `assignee:alice`, `author:alice`,
//! `#bug`, `~bug` or `label:bug` (several labels are all required, as
//! GitLab reads them), `$v1`, `%v1` or `milestone:v1`, `state:`, values
//! in quotes when they have spaces, and words, which GitLab looks for in
//! titles and descriptions; `-` leaves out an assignee, an author, a
//! label or a milestone; `@me` is the account's login. GitLab lists no
//! issues by who is named in them: those that name the user are found
//! through the user's to-do items, which GitLab makes when they are.
//!
//! Sources: docs.gitlab.com/api "Issues" (list project issues: `state`,
//! `assignee_username`, `author_username`, `labels`, `milestone`,
//! `search`, `not`, `iids[]`, `with_labels_details`; get one), "Notes"
//! (list issue notes, `system`), "To-do items" (`action`, `type`,
//! `project_id`, `state`; the actions `mentioned` and
//! `directly_addressed`); docs.gitlab.com "Searching and filtering" (the
//! `@`, `~` and `%` marks).

use std::collections::BTreeSet;

use serde::Deserialize;

use super::pulls::{WireNote, note};
use super::{WireLabel, WireMilestone, WireUser, fresh, label, milestone, project_path};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{Issue, IssueDetails, IssueState, Listing},
    provider::{IssueFilter, IssueQuery, Session, StateFilter, Target},
    search,
};

/// Most comments of an issue read.
const MOST_NOTES: usize = 1000;
/// Most issues that name the user looked for.
const MOST_MENTIONED: usize = 100;

#[derive(Debug, Deserialize)]
pub(super) struct WireIssue {
    iid: u64,
    title: String,
    state: String,
    #[serde(default)]
    author: Option<WireUser>,
    #[serde(default)]
    assignees: Vec<WireUser>,
    #[serde(default)]
    labels: Vec<WireLabel>,
    #[serde(default)]
    milestone: Option<WireMilestone>,
    #[serde(default)]
    user_notes_count: u64,
    created_at: String,
    updated_at: String,
    web_url: String,
    #[serde(default)]
    description: Option<String>,
}

pub(super) fn issue(wire: &WireIssue, target: &Target) -> Issue {
    Issue {
        repo: target.full_name(),
        number: wire.iid,
        title: api::title(&wire.title),
        state: if wire.state == "opened" {
            IssueState::Open
        } else {
            IssueState::Closed
        },
        author: wire.author.as_ref().map(WireUser::user).unwrap_or_default(),
        assignees: wire.assignees.iter().map(WireUser::user).collect(),
        labels: wire.labels.iter().map(label).collect(),
        milestone: wire.milestone.as_ref().map(milestone),
        created: api::clean_line(&wire.created_at, 64),
        updated: api::clean_line(&wire.updated_at, 64),
        comments: wire.user_notes_count,
        web_url: api::clean_line(&wire.web_url, 2048),
    }
}

/// The list's parameters from the query.
fn query_parameters(query: &str, login: &str) -> Result<Vec<(String, String)>, ProviderError> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut put = |key: &str, value: String| {
        out.retain(|(known, _)| known != key);
        out.push((key.to_owned(), value));
    };
    let mut labels: Vec<String> = Vec::new();
    let mut not_labels: Vec<String> = Vec::new();
    let mut words: Vec<String> = Vec::new();
    for term in search::parse(query, true).map_err(ProviderError::Refused)? {
        let values: Vec<String> = term
            .values
            .iter()
            .map(|value| {
                if value == "@me" {
                    login.to_owned()
                } else {
                    value.clone()
                }
            })
            .collect();
        let first = values.first().cloned().unwrap_or_default();
        match (term.key.as_deref(), term.negated) {
            (None, false) => words.extend(values),
            (Some("label" | "labels"), false) => labels.extend(values),
            (Some("label" | "labels"), true) => not_labels.extend(values),
            (Some("assignee"), false) => put("assignee_username", first),
            (Some("assignee"), true) => put("not[assignee_username]", first),
            (Some("author"), false) => put("author_username", first),
            (Some("author"), true) => put("not[author_username]", first),
            (Some("milestone"), false) => put("milestone", first),
            (Some("milestone"), true) => put("not[milestone]", first),
            (Some("state"), false) => put(
                "state",
                match first.as_str() {
                    "open" | "opened" => "opened",
                    "closed" => "closed",
                    _ => "all",
                }
                .to_owned(),
            ),
            (key, negated) => {
                return Err(ProviderError::Refused(say!(
                    "gitlab-issues-not-searchable",
                    negation = if negated { "-" } else { "" },
                    key = key
                        .map(|key| format!("{key}:"))
                        .unwrap_or_else(|| say!("gitlab-issues-words").to_owned())
                )));
            }
        }
    }
    if !labels.is_empty() {
        put("labels", labels.join(","));
    }
    if !not_labels.is_empty() {
        put("not[labels]", not_labels.join(","));
    }
    if !words.is_empty() {
        put("search", words.join(" "));
    }
    Ok(out)
}

/// The numbers of the issues that name the user: their to-do items for
/// being named, pending or done.
fn mentioned(session: &Session<'_>, target: &Target) -> Result<Vec<u64>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireProject {
        id: u64,
    }
    #[derive(Debug, Deserialize)]
    struct WireTarget {
        iid: u64,
    }
    #[derive(Debug, Deserialize)]
    struct WireTodo {
        #[serde(default)]
        action_name: Option<String>,
        #[serde(default)]
        target: Option<WireTarget>,
    }
    let api = session.api()?;
    let (project, _): (WireProject, _) = api.json(Method::Get, &project_path(target), None)?;
    let mut numbers = BTreeSet::new();
    for state in ["pending", "done"] {
        let path = format!(
            "/todos?project_id={}&type=Issue&state={state}&per_page=100",
            project.id
        );
        let (todos, _, _) = api.pages::<WireTodo>(&path, MOST_MENTIONED)?;
        numbers.extend(
            todos
                .iter()
                .filter(|todo| {
                    matches!(
                        todo.action_name.as_deref(),
                        Some("mentioned" | "directly_addressed")
                    )
                })
                .filter_map(|todo| todo.target.as_ref().map(|target| target.iid)),
        );
    }
    Ok(numbers.into_iter().rev().take(MOST_MENTIONED).collect())
}

/// A project's issues.
pub(super) fn list(
    session: &Session<'_>,
    target: &Target,
    query: &IssueQuery,
) -> Result<Listing<Issue>, ProviderError> {
    let login = session.account.login.as_str();
    let mut parameters: Vec<(String, String)> = vec![
        (
            "state".to_owned(),
            match query.state {
                StateFilter::Closed => "closed",
                StateFilter::All => "all",
                StateFilter::Open | StateFilter::Merged => "opened",
            }
            .to_owned(),
        ),
        ("with_labels_details".to_owned(), "true".to_owned()),
    ];
    match query.filter {
        IssueFilter::All | IssueFilter::Mentioned => {}
        IssueFilter::Assigned => {
            parameters.push(("assignee_username".to_owned(), login.to_owned()))
        }
        IssueFilter::Created => parameters.push(("author_username".to_owned(), login.to_owned())),
    }
    for (key, value) in query_parameters(query.query.as_deref().unwrap_or_default(), login)? {
        parameters.retain(|(known, _)| *known != key);
        parameters.push((key, value));
    }
    if query.filter == IssueFilter::Mentioned {
        let numbers = mentioned(session, target)?;
        if numbers.is_empty() {
            return Ok(Listing {
                items: Vec::new(),
                more: false,
                freshness: crate::model::Freshness {
                    fetched: Some(api::rfc3339(session.now)),
                    ..Default::default()
                },
            });
        }
        parameters.extend(
            numbers
                .iter()
                .map(|number| ("iids[]".to_owned(), number.to_string())),
        );
    }
    let mut path = format!("{}/issues?", project_path(target));
    for (key, value) in &parameters {
        path.push_str(&format!("{key}={}&", api::encode(value)));
    }
    path.push_str("per_page=100");
    let (found, more, last) = session.api()?.pages::<WireIssue>(&path, query.most)?;
    Ok(Listing {
        items: found.iter().map(|wire| issue(wire, target)).collect(),
        more,
        freshness: fresh(session, &last),
    })
}

/// One issue with its comments.
pub(super) fn show(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<IssueDetails, ProviderError> {
    let api = session.api()?;
    let one = format!("{}/issues/{number}", project_path(target));
    let (wire, first): (WireIssue, _) = api.json(Method::Get, &one, None)?;
    let (notes, _, last) = api.pages::<WireNote>(
        &format!("{one}/notes?sort=asc&order_by=created_at&per_page=100"),
        MOST_NOTES,
    )?;
    Ok(IssueDetails {
        issue: issue(&wire, target),
        body: api::body(wire.description.as_deref().unwrap_or_default()),
        comments: notes
            .iter()
            .filter(|note| note.written())
            .map(note)
            .collect(),
        freshness: api::latest(fresh(session, &first), fresh(session, &last)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn marks_and_keys_are_made_into_gitlab_parameters() {
        let parameters = query_parameters(
            r#"@me #bug ~"to do" -label:wip $v1 state:closed crash fix"#,
            "alice",
        )
        .unwrap();
        assert_eq!(
            parameters,
            [
                ("assignee_username".to_owned(), "alice".to_owned()),
                ("milestone".to_owned(), "v1".to_owned()),
                ("state".to_owned(), "closed".to_owned()),
                ("labels".to_owned(), "bug,to do".to_owned()),
                ("not[labels]".to_owned(), "wip".to_owned()),
                ("search".to_owned(), "crash fix".to_owned()),
            ]
        );
        assert!(query_parameters("sort:updated", "a").is_err());
        assert!(query_parameters("-words", "a").is_err());
    }
}
