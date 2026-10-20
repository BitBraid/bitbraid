//! Issues at GitHub: a repository's list, and one issue with its
//! comments. GitHub's list of issues holds its pull requests too; they
//! are left out.
//!
//! The list's query is the list's own parameters written as `key:value`:
//! `assignee:`, `creator:` (or `author:`), `mentioned:` (or `mentions:`),
//! `state:`, `milestone:` (a number, a title, `*` or `none`), `labels:` (or
//! `label:`; several, split by commas, are all required, as GitHub reads
//! them), `since:`, `sort:`, `direction:`; `@me` is the account's login.
//! Words without a key are looked for in the titles. Nothing can be left
//! out with `-` here: GitHub's list has no way to.
//!
//! Sources: docs.github.com/en/rest "Issues" (list repository issues:
//! `state`, `assignee`, `creator`, `mentioned`, `milestone`, `labels`,
//! `since`, `sort`, `direction`; get an issue; "pull requests are
//! issues", the `pull_request` key), "Issue comments" (list), "Milestones"
//! (list).

use serde::Deserialize;

use super::{WireLabel, WireMilestone, WireUser, fresh, label, milestone, repo_path};
use crate::{
    api::{self, ProviderError},
    http::Method,
    model::{Comment, Issue, IssueDetails, IssueState, Listing},
    provider::{IssueFilter, IssueQuery, Session, StateFilter, Target},
    search,
};

/// Most comments of an issue read.
const MOST_COMMENTS: usize = 1000;

#[derive(Debug, Deserialize)]
struct WireIssue {
    number: u64,
    title: String,
    state: String,
    #[serde(default)]
    user: Option<WireUser>,
    #[serde(default)]
    assignees: Vec<WireUser>,
    #[serde(default)]
    labels: Vec<WireLabel>,
    #[serde(default)]
    milestone: Option<WireMilestone>,
    #[serde(default)]
    comments: u64,
    created_at: String,
    updated_at: String,
    html_url: String,
    #[serde(default)]
    body: Option<String>,
    /// Present when it is a pull request.
    #[serde(default)]
    pull_request: Option<serde_json::Value>,
}

fn issue(wire: &WireIssue, target: &Target) -> Issue {
    Issue {
        repo: target.full_name(),
        number: wire.number,
        title: api::title(&wire.title),
        state: if wire.state == "open" {
            IssueState::Open
        } else {
            IssueState::Closed
        },
        author: wire.user.as_ref().map(WireUser::user).unwrap_or_default(),
        assignees: wire.assignees.iter().map(WireUser::user).collect(),
        labels: wire.labels.iter().map(label).collect(),
        milestone: wire.milestone.as_ref().map(milestone),
        created: api::clean_line(&wire.created_at, 64),
        updated: api::clean_line(&wire.updated_at, 64),
        comments: wire.comments,
        web_url: api::clean_line(&wire.html_url, 2048),
    }
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

/// The parameters of a list: each key with its value.
type Parameters = Vec<(&'static str, String)>;

/// The list's parameters from what is asked: the filter first, the
/// query's keys in its place; and the words looked for in titles.
fn parameters(
    session: &Session<'_>,
    target: &Target,
    query: &IssueQuery,
) -> Result<(Parameters, Vec<String>), ProviderError> {
    let login = session.account.login.clone();
    let mut set: Vec<(&'static str, String)> = Vec::new();
    let mut put = |key: &'static str, value: String| {
        set.retain(|(known, _)| *known != key);
        set.push((key, value));
    };
    put(
        "state",
        match query.state {
            StateFilter::Closed => "closed",
            StateFilter::All => "all",
            StateFilter::Open | StateFilter::Merged => "open",
        }
        .to_owned(),
    );
    match query.filter {
        IssueFilter::All => {}
        IssueFilter::Assigned => put("assignee", login.clone()),
        IssueFilter::Created => put("creator", login.clone()),
        IssueFilter::Mentioned => put("mentioned", login.clone()),
    }
    let mut words = Vec::new();
    let terms = search::parse(query.query.as_deref().unwrap_or_default(), false)
        .map_err(ProviderError::Refused)?;
    for term in terms {
        if term.negated {
            return Err(ProviderError::Refused(
                say!("github-issues-no-negation").to_owned(),
            ));
        }
        let Some(key) = term.key.as_deref() else {
            words.extend(term.values.iter().map(|word| word.to_lowercase()));
            continue;
        };
        let value = term
            .values
            .iter()
            .map(|value| {
                if value == "@me" {
                    login.as_str()
                } else {
                    value
                }
            })
            .collect::<Vec<_>>()
            .join(",");
        let key = match key {
            "assignee" => "assignee",
            "creator" | "author" => "creator",
            "mentioned" | "mentions" => "mentioned",
            "state" => "state",
            "milestone" => "milestone",
            "labels" | "label" => "labels",
            "since" => "since",
            "sort" => "sort",
            "direction" => "direction",
            other => {
                return Err(ProviderError::Refused(say!(
                    "github-issues-key-not-taken",
                    key = other
                )));
            }
        };
        let value = if key == "milestone" {
            milestone_number(session, target, &value)?
        } else {
            value
        };
        put(key, value);
    }
    Ok((set, words))
}

/// A milestone as the list takes it: its number, `*` or `none`; a title
/// is looked up among the repository's milestones.
fn milestone_number(
    session: &Session<'_>,
    target: &Target,
    value: &str,
) -> Result<String, ProviderError> {
    if value == "*" || value.eq_ignore_ascii_case("none") || value.parse::<u64>().is_ok() {
        return Ok(value.to_owned());
    }
    let (all, _, _) = session.api()?.pages::<WireMilestone>(
        &format!("{}/milestones?state=all&per_page=100", repo_path(target)),
        500,
    )?;
    all.iter()
        .find(|known| known.title.eq_ignore_ascii_case(value))
        .map(|known| known.number.to_string())
        .ok_or_else(|| {
            ProviderError::Refused(say!(
                "github-no-milestone-called",
                title = format!("{value:?}")
            ))
        })
}

/// A repository's issues.
pub(super) fn list(
    session: &Session<'_>,
    target: &Target,
    query: &IssueQuery,
) -> Result<Listing<Issue>, ProviderError> {
    let (set, words) = parameters(session, target, query)?;
    let mut path = format!("{}/issues?per_page=100", repo_path(target));
    for (key, value) in &set {
        path.push_str(&format!("&{key}={}", api::encode(value)));
    }
    let (found, more, last) =
        session
            .api()?
            .pages_where::<WireIssue>(&path, query.most, |wire| {
                wire.pull_request.is_none() && {
                    let title = wire.title.to_lowercase();
                    words.iter().all(|word| title.contains(word.as_str()))
                }
            })?;
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
    let base = repo_path(target);
    let (wire, first): (WireIssue, _) =
        api.json(Method::Get, &format!("{base}/issues/{number}"), None)?;
    if wire.pull_request.is_some() {
        return Err(ProviderError::Refused(say!(
            "github-issue-is-pull-request",
            number
        )));
    }
    let (comments, _, last) = api.pages::<WireComment>(
        &format!("{base}/issues/{number}/comments?per_page=100"),
        MOST_COMMENTS,
    )?;
    Ok(IssueDetails {
        issue: issue(&wire, target),
        body: api::body(wire.body.as_deref().unwrap_or_default()),
        comments: comments.iter().map(comment).collect(),
        freshness: api::latest(fresh(session, &first), fresh(session, &last)),
    })
}
