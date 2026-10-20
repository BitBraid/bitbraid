//! Changing pull requests at GitHub: opening one with its reviewers,
//! assignees, labels and milestone; changing any of those, its base and
//! whether it is a draft; merging it in a way the repository allows,
//! moving the pull requests stacked on it to its base and removing its
//! branch; reviewing it; commenting on it, on lines of one of its files
//! (what a suggestion from local edits is sent as), or answering a
//! comment.
//!
//! Opening one takes up to three requests: the pull request, who is asked
//! to review it, and its assignees, labels and milestone (which are the
//! issue's it also is). When one after the first fails, the pull request
//! is there all the same, and the error says its number.
//!
//! Stacks: when a merge is asked to take the stack along, the open pull
//! requests whose base is the merged one's branch are moved to its base
//! before that branch is removed (GitHub closes a pull request whose
//! base is removed). A branch that still has one going into it is not
//! removed.
//!
//! Sources: docs.github.com/en/rest "Pulls" (get, create: `draft`, update:
//! `base`, merge: `merge_method`, `commit_title`, `commit_message`, `sha`,
//! the answers 405 and 409), "Review requests" (request and remove
//! reviewers, `team_reviewers`), "Issues" (update: `assignees`, `labels`,
//! `milestone`), "Pull request reviews" (create: `event`, a body needed for
//! `REQUEST_CHANGES` and `COMMENT`), "Pull request review comments" (create
//! a reply; create one on lines: `commit_id`, `line`, `side`,
//! `start_line`, `start_side`), "Issue comments" (create, get), "Git
//! references" (delete a
//! reference); docs.github.com/en/graphql "Mutations"
//! (`convertPullRequestToDraft`, `markPullRequestReadyForReview`);
//! docs.github.com "About pull request merges", "Changing the base branch
//! of a pull request".

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{WireComment, WirePull, WireReview, comment, pull, review_state, show};
use crate::{
    api::{self, Api, ProviderError},
    github::repo_path,
    http::Method,
    model::{
        Comment, MergeMethod, Merged, PullDetails, PullRequest, PullState, Review, ReviewState,
    },
    provider::{
        self, LineComment, MergeRequest, NewPull, PullChange, ReviewEvent, Session, Target,
    },
};

/// Most lines of a comment quoted in an answer to it.
const QUOTED_LINES: usize = 20;

/// `/repos/owner/name/pulls/7`.
fn one(target: &Target, number: u64) -> String {
    format!("{}/pulls/{number}", repo_path(target))
}

/// A pull request as GitHub answers it.
fn wire(api: &Api<'_>, target: &Target, number: u64) -> Result<WirePull, ProviderError> {
    let (wire, _) = api.json(Method::Get, &one(target, number), None)?;
    Ok(wire)
}

/// An object's fields, to add more to.
fn fields(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(fields) => fields,
        _ => Map::new(),
    }
}

/// Ask, and keep nothing of the answer but that it is a success.
fn send(api: &Api<'_>, method: Method, path: &str, body: &Value) -> Result<(), ProviderError> {
    api.send(method, api.url(path)?, Some(body))?;
    Ok(())
}

/// One pull request as a row of the list shows it.
pub(in crate::github) fn get(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<PullRequest, ProviderError> {
    Ok(pull(&wire(&session.api()?, target, number)?, target))
}

/// A milestone's number, which is its id at GitHub.
fn milestone_number(id: &str) -> Result<u64, ProviderError> {
    id.trim()
        .parse::<u64>()
        .ok()
        .filter(|number| (1..=u64::from(u32::MAX)).contains(number))
        .ok_or_else(|| {
            ProviderError::Refused(say!(
                "github-not-a-milestone-number",
                id = format!("{:?}", api::clean_line(id, 64))
            ))
        })
}

/// Who is asked to review: people by their login, and teams as
/// `organisation/team`, which GitHub names by the team's part.
fn people_and_teams(names: &[String]) -> (Vec<&str>, Vec<&str>) {
    let mut people = Vec::new();
    let mut teams = Vec::new();
    for name in names {
        match name.split_once('/') {
            Some((_, team)) => teams.push(team),
            None => people.push(name.as_str()),
        }
    }
    (people, teams)
}

/// Ask these to review, or (with `remove`) no longer ask them.
fn reviewers(
    api: &Api<'_>,
    target: &Target,
    number: u64,
    names: &[String],
    remove: bool,
) -> Result<(), ProviderError> {
    if names.is_empty() {
        return Ok(());
    }
    let (people, teams) = people_and_teams(names);
    let method = if remove { Method::Delete } else { Method::Post };
    send(
        api,
        method,
        &format!("{}/requested_reviewers", one(target, number)),
        &json!({ "reviewers": people, "team_reviewers": teams }),
    )
}

/// Open a pull request with its reviewers, assignees, labels and
/// milestone.
pub(in crate::github) fn create(
    session: &Session<'_>,
    target: &Target,
    new: &NewPull,
) -> Result<PullRequest, ProviderError> {
    let milestone = new.milestone.as_deref().map(milestone_number).transpose()?;
    let api = session.api()?;
    let base = repo_path(target);
    let (made, _): (WirePull, _) = api.json(
        Method::Post,
        &format!("{base}/pulls"),
        Some(&json!({
            "title": new.title, "head": new.head, "base": new.base,
            "body": new.body, "draft": new.draft,
        })),
    )?;
    let number = made.number;
    let then = |what: &str, err: ProviderError| {
        ProviderError::Refused(say!(
            "github-opened-but",
            number,
            what,
            error = err.to_string()
        ))
    };
    reviewers(&api, target, number, &new.reviewers, false)
        .map_err(|err| then(say!("github-reviewers-not-asked"), err))?;
    let mut issue = Map::new();
    if !new.assignees.is_empty() {
        issue.insert("assignees".to_owned(), json!(new.assignees));
    }
    if !new.labels.is_empty() {
        issue.insert("labels".to_owned(), json!(new.labels));
    }
    if let Some(milestone) = milestone {
        issue.insert("milestone".to_owned(), json!(milestone));
    }
    if !issue.is_empty() {
        send(
            &api,
            Method::Patch,
            &format!("{base}/issues/{number}"),
            &Value::Object(issue),
        )
        .map_err(|err| then(say!("github-fields-not-set"), err))?;
    }
    if new.reviewers.is_empty()
        && new.assignees.is_empty()
        && new.labels.is_empty()
        && milestone.is_none()
    {
        return Ok(pull(&made, target));
    }
    get(session, target, number).map_err(|err| then(say!("github-not-read-again"), err))
}

/// Change a pull request: what is given, the rest as it is. Answers it
/// as its view shows it.
pub(in crate::github) fn update(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    change: &PullChange,
) -> Result<PullDetails, ProviderError> {
    let milestone = match &change.milestone {
        Some(Some(id)) => Some(json!(milestone_number(id)?)),
        Some(None) => Some(Value::Null),
        None => None,
    };
    let api = session.api()?;
    let current = wire(&api, target, number)?;
    let mut fields = Map::new();
    for (key, value) in [
        ("title", &change.title),
        ("body", &change.body),
        ("base", &change.base),
    ] {
        if let Some(value) = value {
            fields.insert(key.to_owned(), json!(value));
        }
    }
    if !fields.is_empty() {
        send(
            &api,
            Method::Patch,
            &one(target, number),
            &Value::Object(fields),
        )?;
    }
    if let Some(wanted) = &change.reviewers {
        let asked: Vec<String> = current
            .requested_reviewers
            .iter()
            .map(|user| user.login.clone())
            .chain(
                current
                    .requested_teams
                    .iter()
                    .filter_map(|team| team.slug.clone())
                    .map(|slug| format!("team/{slug}")),
            )
            .collect();
        // A team is known by its own part of `organisation/team`.
        let key = |name: &str| {
            name.split_once('/').map_or_else(
                || name.to_ascii_lowercase(),
                |(_, team)| format!("/{}", team.to_ascii_lowercase()),
            )
        };
        let had: Vec<String> = asked.iter().map(|name| key(name)).collect();
        let want: Vec<String> = wanted.iter().map(|name| key(name)).collect();
        let add: Vec<String> = wanted
            .iter()
            .filter(|name| !had.contains(&key(name)))
            .cloned()
            .collect();
        let remove: Vec<String> = asked
            .iter()
            .filter(|name| !want.contains(&key(name)))
            .cloned()
            .collect();
        reviewers(&api, target, number, &remove, true)?;
        reviewers(&api, target, number, &add, false)?;
    }
    let mut issue = Map::new();
    if let Some(assignees) = &change.assignees {
        issue.insert("assignees".to_owned(), json!(assignees));
    }
    if let Some(labels) = &change.labels {
        issue.insert("labels".to_owned(), json!(labels));
    }
    if let Some(milestone) = milestone {
        issue.insert("milestone".to_owned(), milestone);
    }
    if !issue.is_empty() {
        send(
            &api,
            Method::Patch,
            &format!("{}/issues/{number}", repo_path(target)),
            &Value::Object(issue),
        )?;
    }
    if let Some(draft) = change.draft
        && draft != current.draft
    {
        let id = current
            .node_id
            .as_deref()
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .ok_or_else(|| ProviderError::Malformed(say!("github-no-node-id").to_owned()))?;
        // REST cannot change it: GraphQL can, by the pull request's id.
        let question = if draft {
            "mutation($id: ID!) { convertPullRequestToDraft(input: {pullRequestId: $id}) { \
             pullRequest { isDraft } } }"
        } else {
            "mutation($id: ID!) { markPullRequestReadyForReview(input: {pullRequestId: $id}) { \
             pullRequest { isDraft } } }"
        };
        session
            .graphql()?
            .graphql::<Value>(question, &json!({ "id": id }))?;
    }
    show(session, target, number)
}

/// What a merge answers.
#[derive(Debug, Deserialize)]
struct WireMerged {
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    merged: bool,
    #[serde(default)]
    message: Option<String>,
}

/// A branch's path in an address: each of its parts encoded, the `/`
/// between them kept.
pub(in crate::github) fn branch_path(branch: &str) -> String {
    branch
        .split('/')
        .map(api::encode)
        .collect::<Vec<_>>()
        .join("/")
}

/// Move the open pull requests whose base is `from` to `to`: the numbers
/// moved, and what stopped one, if one was.
fn retarget(api: &Api<'_>, target: &Target, from: &str, to: &str) -> (Vec<u64>, Option<String>) {
    let path = format!(
        "{}/pulls?state=open&base={}&per_page=100",
        repo_path(target),
        api::encode(from)
    );
    let above = match api.pages::<WirePull>(&path, provider::MOST_STACKED) {
        Ok((above, _, _)) => above,
        Err(err) => {
            return (
                Vec::new(),
                Some(say!(
                    "github-stacked-not-read",
                    from,
                    error = err.to_string()
                )),
            );
        }
    };
    let mut moved = Vec::new();
    for wire in above.iter().filter(|wire| wire.base.name == from) {
        match send(
            api,
            Method::Patch,
            &one(target, wire.number),
            &json!({ "base": to }),
        ) {
            Ok(()) => moved.push(wire.number),
            Err(err) => {
                return (
                    moved,
                    Some(say!(
                        "github-stacked-still-goes",
                        number = wire.number,
                        from,
                        error = err.to_string()
                    )),
                );
            }
        }
    }
    (moved, None)
}

/// The error for a pull request that is not open any more.
pub(in crate::github) fn not_open(state: PullState, number: u64) -> Result<(), ProviderError> {
    match state {
        PullState::Open => Ok(()),
        PullState::Merged => Err(ProviderError::Refused(say!(
            "github-merged-already",
            number
        ))),
        PullState::Closed => Err(ProviderError::Refused(say!("github-closed", number))),
    }
}

/// Merge a pull request, and after it, as asked, move the pull requests
/// stacked on it to its base and remove its branch.
pub(in crate::github) fn merge(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    request: &MergeRequest,
) -> Result<Merged, ProviderError> {
    let api = session.api()?;
    let current = wire(&api, target, number)?;
    let summary = pull(&current, target);
    // GitHub answers a merged one as merged again.
    not_open(summary.state, number)?;
    let method = match request.method {
        MergeMethod::Merge => "merge",
        MergeMethod::Squash => "squash",
        MergeMethod::Rebase => "rebase",
    };
    // The head as it was read: a push since then makes the merge fail
    // rather than take what nobody looked at.
    let mut body = fields(json!({ "merge_method": method, "sha": current.head.sha }));
    if let Some(title) = &request.title {
        body.insert("commit_title".to_owned(), json!(title));
    }
    if let Some(message) = &request.message {
        body.insert("commit_message".to_owned(), json!(message));
    }
    let (merged, _): (WireMerged, _) = api.json(
        Method::Put,
        &format!("{}/merge", one(target, number)),
        Some(&Value::Object(body)),
    )?;
    let mut out = Merged {
        merged: merged.merged,
        sha: merged
            .sha
            .as_deref()
            .map(|sha| api::clean_line(sha, 64))
            .filter(|sha| !sha.is_empty()),
        message: merged
            .message
            .as_deref()
            .map(|text| api::clean_line(text, 300)),
        retargeted: Vec::new(),
        branch_deleted: false,
    };
    if !out.merged {
        return Ok(out);
    }
    // Its branch is the repository's own (not a fork's, nor one gone).
    let own = current.head.repo.is_some() && summary.head_repo.is_none();
    let mut notes: Vec<String> = Vec::new();
    let mut held = false;
    if request.retarget_stack && own {
        let (moved, stopped) = retarget(&api, target, &current.head.name, &current.base.name);
        out.retargeted = moved;
        if let Some(stopped) = stopped {
            notes.push(stopped);
            held = true;
        }
    }
    if request.delete_branch {
        if !own {
            notes.push(say!("github-branch-not-here").to_owned());
        } else if held {
            notes.push(say!("github-branch-kept").to_owned());
        } else {
            let path = format!(
                "{}/git/refs/heads/{}",
                repo_path(target),
                branch_path(&current.head.name)
            );
            match api.send(Method::Delete, api.url(&path)?, None) {
                Ok(_) => out.branch_deleted = true,
                // Gone already: the repository removes merged branches.
                Err(ProviderError::NotFound | ProviderError::Status { status: 422, .. }) => {
                    out.branch_deleted = true;
                }
                Err(err) => notes.push(say!("github-branch-not-removed", error = err.to_string())),
            }
        }
    }
    if !notes.is_empty() {
        let said = out
            .message
            .take()
            .into_iter()
            .chain(notes)
            .collect::<Vec<_>>();
        out.message = Some(said.join("; "));
    }
    Ok(out)
}

/// Review a pull request. GitHub does not take a review of one's own
/// pull request that approves or asks for changes, and takes one that
/// asks for changes or only comments only with a body.
pub(in crate::github) fn review(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    event: ReviewEvent,
    body: Option<&str>,
) -> Result<Review, ProviderError> {
    let body = body.unwrap_or_default();
    let (name, needs_body) = match event {
        ReviewEvent::Approve => ("APPROVE", false),
        ReviewEvent::RequestChanges => ("REQUEST_CHANGES", true),
        ReviewEvent::Comment => ("COMMENT", true),
    };
    if needs_body && body.trim().is_empty() {
        return Err(ProviderError::Refused(
            say!("github-review-needs-text").to_owned(),
        ));
    }
    let mut sent = fields(json!({ "event": name }));
    if !body.is_empty() {
        sent.insert("body".to_owned(), json!(body));
    }
    let (wire, _): (WireReview, _) = session.api()?.json(
        Method::Post,
        &format!("{}/reviews", one(target, number)),
        Some(&Value::Object(sent)),
    )?;
    Ok(Review {
        id: wire.id.to_string(),
        author: wire
            .user
            .as_ref()
            .map(crate::github::WireUser::user)
            .unwrap_or_default(),
        state: review_state(&wire.state).unwrap_or(ReviewState::Commented),
        body: api::body(wire.body.as_deref().unwrap_or_default()),
        submitted: wire
            .submitted_at
            .as_deref()
            .map(|at| api::clean_line(at, 64)),
    })
}

/// `body` after the first lines of `quoted`, each as a quote.
fn quoting(quoted: &WireComment, body: &str) -> String {
    let mut out = String::new();
    for line in quoted
        .body
        .as_deref()
        .unwrap_or_default()
        .lines()
        .take(QUOTED_LINES)
    {
        out.push_str("> ");
        out.push_str(&api::clean_line(line, 1000));
        out.push('\n');
    }
    out.push('\n');
    out.push_str(body);
    out
}

/// Comment on a pull request's conversation, or answer a comment: a
/// comment on a line is answered in its thread; a comment on the
/// conversation, which has no threads at GitHub, is answered by a comment
/// that quotes it.
pub(in crate::github) fn comment_on(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    body: &str,
    reply_to: Option<&str>,
) -> Result<Comment, ProviderError> {
    let api = session.api()?;
    let conversation = format!("{}/issues/{number}/comments", repo_path(target));
    let Some(reply_to) = reply_to else {
        let (made, _): (WireComment, _) =
            api.json(Method::Post, &conversation, Some(&json!({ "body": body })))?;
        return Ok(comment(&made));
    };
    let id = reply_to
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            ProviderError::Refused(say!(
                "github-not-a-comment-id",
                id = format!("{:?}", api::clean_line(reply_to, 64))
            ))
        })?;
    match api.json::<WireComment>(
        Method::Post,
        &format!("{}/comments/{id}/replies", one(target, number)),
        Some(&json!({ "body": body })),
    ) {
        Ok((made, _)) => return Ok(comment(&made)),
        Err(ProviderError::NotFound) => {}
        Err(err) => return Err(err),
    }
    let (quoted, _): (WireComment, _) = api.json(
        Method::Get,
        &format!("{}/issues/comments/{id}", repo_path(target)),
        None,
    )?;
    // Only a comment of this pull request's conversation.
    let here = quoted
        .issue_url
        .as_deref()
        .is_some_and(|url| url.ends_with(&format!("/issues/{number}")));
    if !here {
        return Err(ProviderError::NotFound);
    }
    let (made, _): (WireComment, _) = api.json(
        Method::Post,
        &conversation,
        Some(&json!({ "body": quoting(&quoted, body) })),
    )?;
    Ok(comment(&made))
}

/// Comment on lines of a file of a pull request, as its head has them:
/// on the new side, against its head commit.
pub(in crate::github) fn comment_on_line(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    wanted: &LineComment,
) -> Result<Comment, ProviderError> {
    let api = session.api()?;
    let head = wire(&api, target, number)?.head.sha;
    if head.is_empty() {
        return Err(ProviderError::Refused(
            say!("github-no-head-commit").to_owned(),
        ));
    }
    let mut fields = fields(json!({
        "body": wanted.body,
        "commit_id": head,
        "path": wanted.path,
        "line": wanted.line,
        "side": "RIGHT",
    }));
    if let Some(start) = wanted.start_line.filter(|start| *start < wanted.line) {
        fields.insert("start_line".to_owned(), json!(start));
        fields.insert("start_side".to_owned(), json!("RIGHT"));
    }
    let (made, _): (WireComment, _) = api.json(
        Method::Post,
        &format!("{}/comments", one(target, number)),
        Some(&Value::Object(fields)),
    )?;
    Ok(comment(&made))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn people_and_teams_are_told_apart() {
        let names = ["alice".to_owned(), "org/reviewers".to_owned()];
        assert_eq!(people_and_teams(&names), (vec!["alice"], vec!["reviewers"]));
        assert!(milestone_number("3").is_ok());
        assert!(milestone_number("0").is_err());
        assert!(milestone_number("x").is_err());
        assert_eq!(branch_path("feature/a b"), "feature/a%20b");
    }
}
