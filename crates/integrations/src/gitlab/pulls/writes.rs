//! Changing merge requests at GitLab: opening one with its reviewers,
//! assignees, labels and milestone; changing any of those, its target
//! branch and whether it is a draft; merging it in a way the project
//! allows (rebasing first when the project merges only what is on top of
//! its target), moving the merge requests stacked on it to its target and
//! removing its branch; reviewing it; commenting on it, on a line of one
//! of its files (what a suggestion from local edits is sent as), or
//! answering in a thread.
//!
//! A comment on a line is a thread on the merge request's diff: placed by
//! the diff's three commits and the line's number on the new side, and on
//! the old side too where both sides have it, as the diff shows them.
//!
//! GitLab names people by their ids where a merge request is made or
//! changed: the logins given are looked up first, and one that is not
//! there stops the change before anything is made. A draft is a merge
//! request whose title starts with `Draft:` (or `[Draft]`, `(Draft)`):
//! marking one a draft or ready changes its title. Its source branch is
//! never removed unless asked.
//!
//! A review: approving is GitLab's approval; asking for changes and
//! commenting set the reviewer's state (`requested_changes`, `reviewed`)
//! the way GitLab's own review does, by publishing the user's pending
//! comments with that state and the body as a summary; asking for changes
//! takes back an approval the user gave before. GitLab keeps a reviewer's
//! state only for someone asked to review: for anyone else the body is
//! posted and the error says the state was not kept.
//!
//! Sources: docs.gitlab.com/api "Merge requests" (create: `source_branch`,
//! `target_branch`, `target_project_id`, `assignee_ids`, `reviewer_ids`,
//! `labels`, `milestone_id`, `remove_source_branch`; update; merge:
//! `squash`, `should_remove_source_branch`, `merge_commit_message`,
//! `squash_commit_message`, `sha`; rebase, `rebase_in_progress`,
//! `merge_error`; `detailed_merge_status`), "Merge request approvals"
//! (approve, unapprove), "Draft notes" (publish all pending draft notes:
//! `reviewer_state`, `note`), "Notes" (create a merge request note),
//! "Discussions" (add a note to a merge request thread; create a thread
//! in the diff: `position`, `base_sha`, `start_sha`, `head_sha`,
//! `new_line`, `old_line`), "Merge requests" (`diff_refs`, list diffs),
//! "Users" (list by
//! `username`), "Branches" (delete), "Projects" (`merge_method`,
//! `squash_option`); docs.gitlab.com "Draft merge requests", "Merge
//! methods".

use std::{collections::BTreeMap, time::Duration};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{
    MOST_FILES, WireDiff, WireDiscussion, WireMerge, WireNote, WireProjectSettings, fork_of,
    fork_paths, merge_methods, merge_request, note, show,
};
use crate::{
    api::{self, Api, ProviderError},
    gitlab::{WireUser, project_path},
    http::Method,
    model::{Comment, MergeMethod, Merged, PullDetails, PullRequest, Review, ReviewState},
    provider::{
        self, LineComment, MergeRequest, NewPull, PullChange, ReviewEvent, Session, Target,
    },
};

/// How many times a merge is asked again while GitLab is still working
/// out whether it can be merged, or a rebase is running.
const MOST_WAITS: usize = 30;
/// The wait between two such asks.
const WAIT: Duration = Duration::from_secs(1);

/// `/projects/group%2Fname/merge_requests/7`.
fn one(target: &Target, number: u64) -> String {
    format!("{}/merge_requests/{number}", project_path(target))
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

/// One merge request as a row of the list shows it, its fork named when
/// the account may see it.
pub(in crate::gitlab) fn get(
    session: &Session<'_>,
    target: &Target,
    number: u64,
) -> Result<PullRequest, ProviderError> {
    let (wire, _): (WireMerge, _) = session
        .api()?
        .json(Method::Get, &one(target, number), None)?;
    let forks = match fork_of(&wire) {
        Some(_) => fork_paths(session, &[&wire])?,
        None => BTreeMap::new(),
    };
    Ok(merge_request(&wire, target, &forks))
}

/// The ids of people by their logins; one that is not there is an error
/// that names it.
fn user_ids(api: &Api<'_>, logins: &[String]) -> Result<Vec<u64>, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireId {
        id: u64,
        username: String,
    }
    logins
        .iter()
        .map(|login| {
            let (found, _): (Vec<WireId>, _) = api.json(
                Method::Get,
                &format!("/users?username={}", api::encode(login)),
                None,
            )?;
            found
                .iter()
                .find(|user| user.username.eq_ignore_ascii_case(login))
                .map(|user| user.id)
                .ok_or_else(|| {
                    ProviderError::Refused(say!(
                        "gitlab-no-such-user",
                        login = format!("{:?}", api::clean_line(login, 256))
                    ))
                })
        })
        .collect()
}

/// A milestone's id at GitLab.
fn milestone_id(id: &str) -> Result<u64, ProviderError> {
    id.trim()
        .parse::<u64>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| {
            ProviderError::Refused(say!(
                "gitlab-not-a-milestone-id",
                id = format!("{:?}", api::clean_line(id, 64))
            ))
        })
}

/// A title without the marks GitLab reads as a draft at its start.
fn undrafted(title: &str) -> &str {
    let mut rest = title.trim_start();
    for _ in 0..8 {
        let lower = rest.to_ascii_lowercase();
        let Some(mark) = ["draft:", "[draft]", "(draft)"]
            .iter()
            .find(|mark| lower.starts_with(**mark))
        else {
            break;
        };
        rest = rest.get(mark.len()..).unwrap_or_default().trim_start();
    }
    rest
}

/// A title that makes a draft, or one that does not.
fn titled(title: &str, draft: bool) -> String {
    let bare = undrafted(title);
    if draft {
        format!("Draft: {bare}")
    } else {
        bare.to_owned()
    }
}

/// Open a merge request. `head` as `owner:branch` is the branch of the
/// fork `owner/<name>`, opened from there into `target`.
pub(in crate::gitlab) fn create(
    session: &Session<'_>,
    target: &Target,
    new: &NewPull,
) -> Result<PullRequest, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireProjectId {
        id: u64,
    }
    let milestone = new.milestone.as_deref().map(milestone_id).transpose()?;
    let api = session.api()?;
    let reviewer_ids = user_ids(&api, &new.reviewers)?;
    let assignee_ids = user_ids(&api, &new.assignees)?;
    let (fork, branch) = match new.head.split_once(':') {
        Some((owner, branch)) => (
            Some(Target {
                owner: owner.to_owned(),
                name: target.name.clone(),
            }),
            branch,
        ),
        None => (None, new.head.as_str()),
    };
    let mut body = fields(json!({
        "source_branch": branch, "target_branch": new.base,
        "title": if new.draft { titled(&new.title, true) } else { new.title.clone() },
        "description": new.body, "remove_source_branch": false,
    }));
    if !reviewer_ids.is_empty() {
        body.insert("reviewer_ids".to_owned(), json!(reviewer_ids));
    }
    if !assignee_ids.is_empty() {
        body.insert("assignee_ids".to_owned(), json!(assignee_ids));
    }
    if !new.labels.is_empty() {
        body.insert("labels".to_owned(), json!(new.labels.join(",")));
    }
    if let Some(milestone) = milestone {
        body.insert("milestone_id".to_owned(), json!(milestone));
    }
    let path = match &fork {
        Some(fork) => {
            let (project, _): (WireProjectId, _) =
                api.json(Method::Get, &project_path(target), None)?;
            body.insert("target_project_id".to_owned(), json!(project.id));
            format!("{}/merge_requests", project_path(fork))
        }
        None => format!("{}/merge_requests", project_path(target)),
    };
    let (made, _): (WireMerge, _) = api.json(Method::Post, &path, Some(&Value::Object(body)))?;
    let forks: BTreeMap<u64, String> = match (&fork, fork_of(&made)) {
        (Some(fork), Some(id)) => BTreeMap::from([(id, fork.full_name())]),
        _ => BTreeMap::new(),
    };
    Ok(merge_request(&made, target, &forks))
}

/// Change a merge request: what is given, the rest as it is. Answers it
/// as its view shows it.
pub(in crate::gitlab) fn update(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    change: &PullChange,
) -> Result<PullDetails, ProviderError> {
    let milestone = match &change.milestone {
        Some(Some(id)) => Some(milestone_id(id)?),
        // GitLab takes 0 for none.
        Some(None) => Some(0),
        None => None,
    };
    let api = session.api()?;
    let mut body = Map::new();
    if change.title.is_some() || change.draft.is_some() {
        // A new title keeps the draft mark unless the change says
        // otherwise; marking a draft keeps the title.
        let (wire, _): (WireMerge, _) = api.json(Method::Get, &one(target, number), None)?;
        let draft = change.draft.unwrap_or(wire.draft || wire.work_in_progress);
        let title = change.title.as_deref().unwrap_or(&wire.title);
        body.insert("title".to_owned(), json!(titled(title, draft)));
    }
    if let Some(text) = &change.body {
        body.insert("description".to_owned(), json!(text));
    }
    if let Some(base) = &change.base {
        body.insert("target_branch".to_owned(), json!(base));
    }
    if let Some(reviewers) = &change.reviewers {
        body.insert("reviewer_ids".to_owned(), json!(user_ids(&api, reviewers)?));
    }
    if let Some(assignees) = &change.assignees {
        body.insert("assignee_ids".to_owned(), json!(user_ids(&api, assignees)?));
    }
    if let Some(labels) = &change.labels {
        body.insert("labels".to_owned(), json!(labels.join(",")));
    }
    if let Some(milestone) = milestone {
        body.insert("milestone_id".to_owned(), json!(milestone));
    }
    if !body.is_empty() {
        send(
            &api,
            Method::Put,
            &one(target, number),
            &Value::Object(body),
        )?;
    }
    show(session, target, number)
}

/// What the merge of a merge request has come to.
#[derive(Debug, Deserialize)]
struct WireMergeState {
    state: String,
    source_branch: String,
    target_branch: String,
    #[serde(default)]
    source_project_id: Option<u64>,
    #[serde(default)]
    target_project_id: Option<u64>,
    #[serde(default)]
    sha: Option<String>,
    #[serde(default)]
    merge_commit_sha: Option<String>,
    #[serde(default)]
    squash_commit_sha: Option<String>,
    #[serde(default)]
    detailed_merge_status: Option<String>,
    #[serde(default)]
    rebase_in_progress: Option<bool>,
    #[serde(default)]
    merge_error: Option<String>,
}

impl WireMergeState {
    /// Whether GitLab is still working out whether it can be merged.
    fn checking(&self) -> bool {
        matches!(
            self.detailed_merge_status.as_deref(),
            Some("checking" | "unchecked" | "preparing" | "approvals_syncing")
        )
    }
}

fn state(api: &Api<'_>, target: &Target, number: u64) -> Result<WireMergeState, ProviderError> {
    let (wire, _) = api.json(
        Method::Get,
        &format!("{}?include_rebase_in_progress=true", one(target, number)),
        None,
    )?;
    Ok(wire)
}

/// Wait, bounded, while `waiting` says so of the merge request.
fn wait_while(
    session: &Session<'_>,
    api: &Api<'_>,
    target: &Target,
    number: u64,
    waiting: impl Fn(&WireMergeState) -> bool,
) -> Result<WireMergeState, ProviderError> {
    let mut now = state(api, target, number)?;
    for _ in 0..MOST_WAITS {
        if !waiting(&now) {
            break;
        }
        if session.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(ProviderError::Http(crate::http::FetchError::Cancelled));
        }
        std::thread::sleep(WAIT);
        now = state(api, target, number)?;
    }
    Ok(now)
}

/// The ways named as a sentence: `merge, squash`.
fn ways(methods: &[MergeMethod]) -> String {
    methods
        .iter()
        .map(|method| match method {
            MergeMethod::Merge => say!("gitlab-way-merge"),
            MergeMethod::Squash => say!("gitlab-way-squash"),
            MergeMethod::Rebase => say!("gitlab-way-rebase"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Move the open merge requests whose target is `from` to `to`: the
/// numbers moved, and what stopped one, if one was.
fn retarget(api: &Api<'_>, target: &Target, from: &str, to: &str) -> (Vec<u64>, Option<String>) {
    #[derive(Debug, Deserialize)]
    struct WireAbove {
        iid: u64,
        target_branch: String,
    }
    let path = format!(
        "{}/merge_requests?state=opened&target_branch={}&per_page=100",
        project_path(target),
        api::encode(from)
    );
    let above = match api.pages::<WireAbove>(&path, provider::MOST_STACKED) {
        Ok((above, _, _)) => above,
        Err(err) => {
            return (
                Vec::new(),
                Some(say!(
                    "gitlab-stacked-not-read",
                    from,
                    error = err.to_string()
                )),
            );
        }
    };
    let mut moved = Vec::new();
    for wire in above.iter().filter(|wire| wire.target_branch == from) {
        match send(
            api,
            Method::Put,
            &one(target, wire.iid),
            &json!({ "target_branch": to }),
        ) {
            Ok(()) => moved.push(wire.iid),
            Err(err) => {
                return (
                    moved,
                    Some(say!(
                        "gitlab-stacked-still-goes",
                        number = wire.iid,
                        from,
                        error = err.to_string()
                    )),
                );
            }
        }
    }
    (moved, None)
}

/// Merge a merge request in a way the project allows, rebasing it first
/// when the project needs that and it is behind; and after it, as asked,
/// move the merge requests stacked on it to its target and remove its
/// branch.
pub(in crate::gitlab) fn merge(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    request: &MergeRequest,
) -> Result<Merged, ProviderError> {
    let api = session.api()?;
    let (settings, _): (WireProjectSettings, _) =
        api.json(Method::Get, &project_path(target), None)?;
    let allowed = merge_methods(&settings);
    if !allowed.contains(&request.method) {
        return Err(ProviderError::Refused(say!(
            "gitlab-way-not-allowed",
            ways = ways(&allowed)
        )));
    }
    let mut before = wait_while(session, &api, target, number, WireMergeState::checking)?;
    match before.state.as_str() {
        "opened" | "locked" => {}
        "merged" => {
            return Err(ProviderError::Refused(say!(
                "gitlab-merged-already",
                number
            )));
        }
        _ => return Err(ProviderError::Refused(say!("gitlab-closed", number))),
    }
    let rebases = matches!(
        settings.merge_method.as_deref(),
        Some("ff" | "rebase_merge")
    );
    if rebases && before.detailed_merge_status.as_deref() == Some("need_rebase") {
        send(
            &api,
            Method::Put,
            &format!("{}/rebase", one(target, number)),
            &json!({}),
        )?;
        // Until the rebase is done and the merge request names the
        // rebased head.
        let old = before.sha.clone();
        before = wait_while(session, &api, target, number, |now| {
            now.rebase_in_progress == Some(true) || now.checking() || now.sha == old
        })?;
        if before.sha == old {
            // Still the head of before: the merge is not held to it.
            before.sha = None;
        }
        if let Some(error) = before
            .merge_error
            .as_deref()
            .filter(|error| !error.is_empty())
        {
            return Err(ProviderError::Refused(say!(
                "gitlab-not-rebased",
                error = api::clean_line(error, 300)
            )));
        }
    }
    let own =
        before.source_project_id.is_some() && before.source_project_id == before.target_project_id;
    // GitLab removes the branch itself unless the stack has to move first.
    let removed_by_gitlab = request.delete_branch && !request.retarget_stack;
    let squash = request.method == MergeMethod::Squash;
    let mut body =
        fields(json!({ "squash": squash, "should_remove_source_branch": removed_by_gitlab }));
    if let Some(sha) = before.sha.as_deref().filter(|sha| !sha.is_empty()) {
        body.insert("sha".to_owned(), json!(sha));
    }
    let text = match (&request.title, &request.message) {
        (Some(title), Some(message)) => Some(format!("{title}\n\n{message}")),
        (Some(text), None) | (None, Some(text)) => Some(text.clone()),
        (None, None) => None,
    };
    if let Some(text) = text {
        let key = if squash {
            "squash_commit_message"
        } else {
            "merge_commit_message"
        };
        body.insert(key.to_owned(), json!(text));
    }
    let (mut after, _): (WireMergeState, _) = api.json(
        Method::Put,
        &format!("{}/merge", one(target, number)),
        Some(&Value::Object(body)),
    )?;
    if after.state != "merged" {
        after = wait_while(session, &api, target, number, |now| {
            now.state == "opened" && now.merge_error.is_none()
        })?;
    }
    let mut out = Merged {
        merged: after.state == "merged",
        sha: after
            .merge_commit_sha
            .as_deref()
            .or(after.squash_commit_sha.as_deref())
            .or(after.sha.as_deref())
            .map(|sha| api::clean_line(sha, 64))
            .filter(|sha| !sha.is_empty()),
        message: after
            .merge_error
            .as_deref()
            .map(|error| api::clean_line(error, 300))
            .filter(|error| !error.is_empty()),
        retargeted: Vec::new(),
        branch_deleted: false,
    };
    if !out.merged {
        return Ok(out);
    }
    let mut notes: Vec<String> = Vec::new();
    let mut held = false;
    if request.retarget_stack {
        let (moved, stopped) = retarget(&api, target, &before.source_branch, &before.target_branch);
        out.retargeted = moved;
        if let Some(stopped) = stopped {
            notes.push(stopped);
            held = true;
        }
    }
    if removed_by_gitlab {
        out.branch_deleted = true;
    } else if request.delete_branch {
        if !own {
            notes.push(say!("gitlab-branch-not-here").to_owned());
        } else if held {
            notes.push(say!("gitlab-branch-kept").to_owned());
        } else {
            let path = format!(
                "{}/repository/branches/{}",
                project_path(target),
                api::encode(&before.source_branch)
            );
            match api.send(Method::Delete, api.url(&path)?, None) {
                Ok(_) | Err(ProviderError::NotFound) => out.branch_deleted = true,
                Err(err) => notes.push(say!("gitlab-branch-not-removed", error = err.to_string())),
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

/// Review a merge request.
pub(in crate::gitlab) fn review(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    event: ReviewEvent,
    body: Option<&str>,
) -> Result<Review, ProviderError> {
    #[derive(Debug, Deserialize)]
    struct WireReviewer {
        user: WireUser,
        #[serde(default)]
        state: Option<String>,
    }
    let api = session.api()?;
    let (me, _): (WireUser, _) = api.json(Method::Get, "/user", None)?;
    let body = body.unwrap_or_default();
    let base = one(target, number);
    let note = |text: &str| {
        send(
            &api,
            Method::Post,
            &format!("{base}/notes"),
            &json!({ "body": text }),
        )
    };
    let (state, kind) = match event {
        ReviewEvent::Approve => {
            send(&api, Method::Post, &format!("{base}/approve"), &json!({}))?;
            if !body.trim().is_empty() {
                note(body)?;
            }
            (ReviewState::Approved, "approval")
        }
        ReviewEvent::RequestChanges | ReviewEvent::Comment => {
            let (state, word, kind) = if event == ReviewEvent::RequestChanges {
                // An approval given before is taken back: GitLab answers
                // "not found" when there was none.
                match api.send(
                    Method::Post,
                    api.url(&format!("{base}/unapprove"))?,
                    Some(&json!({})),
                ) {
                    Ok(_) | Err(ProviderError::NotFound) => {}
                    Err(err) => return Err(err),
                }
                (
                    ReviewState::ChangesRequested,
                    "requested_changes",
                    "changes",
                )
            } else {
                (ReviewState::Commented, "reviewed", "reviewed")
            };
            let mut sent = fields(json!({ "reviewer_state": word }));
            if !body.trim().is_empty() {
                sent.insert("note".to_owned(), json!(body));
            }
            send(
                &api,
                Method::Post,
                &format!("{base}/draft_notes/bulk_publish"),
                &Value::Object(sent),
            )?;
            // GitLab keeps the state only for someone asked to review.
            let (asked, _, _) =
                api.pages::<WireReviewer>(&format!("{base}/reviewers?per_page=100"), 100)?;
            let kept = asked.iter().any(|reviewer| {
                reviewer.user.username == me.username && reviewer.state.as_deref() == Some(word)
            });
            if !kept {
                return Err(ProviderError::Refused(if body.trim().is_empty() {
                    say!("gitlab-verdict-not-kept", number)
                } else {
                    say!("gitlab-verdict-not-kept-posted", number)
                }));
            }
            (state, kind)
        }
    };
    Ok(Review {
        id: format!("{kind}-{}", me.id.unwrap_or_default()),
        author: me.user(),
        state,
        body: api::body(body),
        submitted: Some(api::rfc3339(session.now)),
    })
}

/// Comment on a merge request, or answer in one of its threads, which
/// `reply_to` names by its id.
pub(in crate::gitlab) fn comment_on(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    body: &str,
    reply_to: Option<&str>,
) -> Result<Comment, ProviderError> {
    let path = match reply_to {
        Some(thread) => {
            if thread.is_empty()
                || thread.len() > 64
                || !thread.bytes().all(|b| b.is_ascii_hexdigit())
            {
                return Err(ProviderError::Refused(say!(
                    "gitlab-not-a-thread-id",
                    id = format!("{:?}", api::clean_line(thread, 64))
                )));
            }
            format!("{}/discussions/{thread}/notes", one(target, number))
        }
        None => format!("{}/notes", one(target, number)),
    };
    let (made, _): (WireNote, _) =
        session
            .api()?
            .json(Method::Post, &path, Some(&json!({ "body": body })))?;
    Ok(note(&made))
}

/// The three commits a merge request's diff is made of.
#[derive(Debug, Deserialize)]
struct WireRefs {
    #[serde(default)]
    diff_refs: Option<WireDiffRefs>,
}

#[derive(Debug, Deserialize)]
struct WireDiffRefs {
    #[serde(default)]
    base_sha: Option<String>,
    #[serde(default)]
    head_sha: Option<String>,
    #[serde(default)]
    start_sha: Option<String>,
}

/// Comment on a line of a file of a merge request, as its head has it: a
/// thread on its diff, on the comment's last line.
pub(in crate::gitlab) fn comment_on_line(
    session: &Session<'_>,
    target: &Target,
    number: u64,
    comment: &LineComment,
) -> Result<Comment, ProviderError> {
    let api = session.api()?;
    let (refs, _): (WireRefs, _) = api.json(Method::Get, &one(target, number), None)?;
    let refused = |what: String| ProviderError::Refused(what);
    let (base, head, start) = match refs.diff_refs {
        Some(WireDiffRefs {
            base_sha: Some(base),
            head_sha: Some(head),
            start_sha: Some(start),
        }) => (base, head, start),
        _ => {
            return Err(refused(say!("gitlab-no-diff").to_owned()));
        }
    };
    let (diffs, _, _) = api.pages::<WireDiff>(
        &format!("{}/diffs?per_page=100", one(target, number)),
        MOST_FILES,
    )?;
    let file = diffs
        .iter()
        .find(|diff| diff.new_path == comment.path)
        .ok_or_else(|| {
            refused(say!(
                "gitlab-file-not-changed",
                path = api::clean_line(&comment.path, 200)
            ))
        })?;
    let lines = crate::suggest::hunks(file.diff.as_deref().unwrap_or_default());
    let (_, old_line) = crate::suggest::place_of(&lines, comment.line).ok_or_else(|| {
        refused(say!(
            "gitlab-line-not-changed",
            line = comment.line,
            path = api::clean_line(&comment.path, 200)
        ))
    })?;
    let mut position = fields(json!({
        "position_type": "text",
        "base_sha": base,
        "start_sha": start,
        "head_sha": head,
        "new_path": file.new_path,
        "old_path": file.old_path.as_deref().unwrap_or(&file.new_path),
        "new_line": comment.line,
    }));
    if let Some(old) = old_line {
        position.insert("old_line".to_owned(), json!(old));
    }
    let (made, _): (WireDiscussion, _) = api.json(
        Method::Post,
        &format!("{}/discussions", one(target, number)),
        Some(&json!({ "body": comment.body, "position": Value::Object(position) })),
    )?;
    let first = made
        .notes
        .first()
        .ok_or_else(|| refused(say!("gitlab-no-comment").to_owned()))?;
    Ok(note(first))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_draft_is_marked_in_its_title() {
        assert_eq!(titled("Fix it", true), "Draft: Fix it");
        assert_eq!(titled("Draft: Fix it", true), "Draft: Fix it");
        assert_eq!(titled("[Draft] (draft) DRAFT: Fix it", false), "Fix it");
        assert_eq!(titled("Drafting rules", false), "Drafting rules");
        assert_eq!(undrafted("draft:"), "");
        assert!(milestone_id("12").is_ok());
        assert!(milestone_id("-1").is_err());
    }
}
