//! The inbox's commands against stand-in answers of both providers,
//! made by hand in the shapes their API documents give: a
//! GitHub account and a GitLab one connected, two repositories on this
//! machine linked to them, one with uncommitted work. What is held: the
//! rows each account answers, the groups and why each is the user's, the
//! repository here of each, work in progress, pins and snoozes kept and
//! read back, a workspace that holds the inbox to its repositories, and
//! an account that cannot be asked while the other is answered.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{
    Services,
    http::Method,
    replay::{Exchange, Recording, Replay},
    secrets::MemoryStore,
};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The moment the tests are at: 2026-10-14T21:30:00Z.
const NOW: i64 = 1_792_013_400;

const GITHUB: &str = "github:github.com:me";
const GITLAB: &str = "gitlab:gitlab.example.org:me";

fn exchange(method: Method, path: &str, status: u16, body: Value) -> Exchange {
    Exchange {
        method,
        path_and_query: path.to_owned(),
        request_body: Value::Null,
        status,
        headers: Default::default(),
        body,
    }
}

fn user(login: &str) -> Value {
    json!({ "login": login, "avatarUrl": format!("https://avatars.example.invalid/{login}"),
            "url": format!("https://github.com/{login}") })
}

/// A pull request as GitHub's search answers the inbox.
fn github_pull(number: u64, repo: &str, author: &str, extra: Value) -> Value {
    let mut pull = json!({
        "number": number,
        "title": format!("Pull request {number}"),
        "state": "OPEN",
        "isDraft": false,
        "url": format!("https://github.com/{repo}/pull/{number}"),
        "createdAt": "2026-10-10T10:00:00Z",
        "updatedAt": format!("2026-10-1{}T10:00:00Z", number % 10),
        "author": user(author),
        "headRefName": format!("topic-{number}"),
        "headRefOid": "0123456789abcdef0123456789abcdef01234567",
        "headRepository": { "nameWithOwner": repo },
        "baseRefName": "main",
        "baseRepository": { "nameWithOwner": repo },
        "labels": { "nodes": [{ "name": "bug", "color": "d73a4a", "description": null }] },
        "assignees": { "nodes": [] },
        "reviewRequests": { "nodes": [] },
        "latestReviews": { "nodes": [] },
        "comments": { "totalCount": 2 },
        "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "SUCCESS" } } }] },
        "additions": 10,
        "deletions": 2,
        "mergeable": "MERGEABLE",
        "reviewDecision": null,
        "milestone": { "number": 3, "title": "v1", "state": "OPEN", "dueOn": null }
    });
    for (key, value) in extra.as_object().cloned().unwrap_or_default() {
        pull[key] = value;
    }
    pull
}

fn github_issue(number: u64, repo: &str) -> Value {
    json!({
        "number": number,
        "title": format!("Issue {number}"),
        "state": "OPEN",
        "url": format!("https://github.com/{repo}/issues/{number}"),
        "createdAt": "2026-10-10T10:00:00Z",
        "updatedAt": format!("2026-10-1{}T11:00:00Z", number % 10),
        "author": user("ann"),
        "repository": { "nameWithOwner": repo },
        "assignees": { "nodes": [user("me")] },
        "labels": { "nodes": [] },
        "milestone": null,
        "comments": { "totalCount": 1 },
        "reactions": { "totalCount": 4 }
    })
}

fn searched(nodes: Vec<Value>) -> Value {
    json!({ "pageInfo": { "hasNextPage": false }, "nodes": nodes })
}

/// GitHub's answer to the inbox's question: #1 mine, clean, its checks
/// failing; #2 someone else's that asks for my review; #3 one asked of
/// my team; an issue assigned to me and opened by me, and one naming me
/// elsewhere.
fn github_answer() -> Value {
    let mine = github_pull(
        1,
        "o/r",
        "me",
        json!({ "commits": { "nodes": [{ "commit": { "statusCheckRollup": { "state": "FAILURE" } } }] } }),
    );
    let theirs = github_pull(
        2,
        "o/r",
        "ann",
        json!({ "reviewRequests": { "nodes": [{ "requestedReviewer": user("me") }] },
                "reviewDecision": "REVIEW_REQUIRED" }),
    );
    let team = github_pull(
        3,
        "o/other",
        "bob",
        json!({ "reviewRequests": { "nodes": [{ "requestedReviewer": { "combinedSlug": "o/team" } }] },
                "isDraft": true, "mergeable": "CONFLICTING" }),
    );
    json!({ "data": {
        "pulls": searched(vec![mine, theirs.clone(), json!({})]),
        "review": searched(vec![theirs, team]),
        "assigned": searched(vec![github_issue(7, "o/r")]),
        "created": searched(vec![github_issue(7, "o/r")]),
        "mentioned": searched(vec![github_issue(8, "o/elsewhere")]),
    }})
}

fn gitlab_user(login: &str) -> Value {
    json!({ "username": login, "name": login, "avatar_url": null, "web_url": null })
}

fn gitlab_merge(iid: u64, author: &str, status: &str) -> Value {
    json!({
        "iid": iid,
        "project_id": 9,
        "title": format!("Merge request {iid}"),
        "state": "opened",
        "draft": false,
        "author": gitlab_user(author),
        "source_branch": format!("feature-{iid}"),
        "target_branch": "main",
        "source_project_id": 9,
        "target_project_id": 9,
        "sha": "89abcdef0123456789abcdef0123456789abcdef",
        "created_at": "2026-10-11T08:00:00.000Z",
        "updated_at": format!("2026-10-12T0{iid}:00:00.000Z"),
        "labels": [],
        "reviewers": [gitlab_user("me")],
        "assignees": [],
        "user_notes_count": 0,
        "web_url": format!("https://gitlab.example.org/g/sub/p/-/merge_requests/{iid}"),
        "references": { "full": format!("g/sub/p!{iid}") },
        "detailed_merge_status": status,
        "has_conflicts": false,
        "milestone": null
    })
}

fn gitlab_issue(iid: u64) -> Value {
    json!({
        "iid": iid,
        "title": format!("GitLab issue {iid}"),
        "state": "opened",
        "author": gitlab_user("ann"),
        "assignees": [gitlab_user("me")],
        "labels": [],
        "user_notes_count": 3,
        "created_at": "2026-10-11T08:00:00.000Z",
        "updated_at": "2026-10-11T09:00:00.000Z",
        "web_url": format!("https://gitlab.example.org/g/sub/p/-/issues/{iid}"),
        "references": { "full": format!("g/sub/p#{iid}") },
        "upvotes": 2,
        "downvotes": 1
    })
}

fn gitlab_lists(most: usize) -> Vec<Exchange> {
    let list = |what: &str, key: &str| {
        format!(
            "/api/v4/{what}?scope=all&state=opened&{key}=me&with_labels_details=true&per_page={most}"
        )
    };
    vec![
        exchange(
            Method::Get,
            &list("merge_requests", "author_username"),
            200,
            json!([gitlab_merge(1, "me", "mergeable")]),
        ),
        exchange(
            Method::Get,
            &list("merge_requests", "assignee_username"),
            200,
            json!([]),
        ),
        // Mine again among those I review, and someone else's.
        exchange(
            Method::Get,
            &list("merge_requests", "reviewer_username"),
            200,
            json!([
                gitlab_merge(1, "me", "mergeable"),
                gitlab_merge(2, "ann", "not_approved")
            ]),
        ),
        exchange(
            Method::Post,
            "/api/graphql",
            200,
            json!({ "data": { "project": { "mergeRequests": { "nodes": [
                { "iid": "1", "headPipeline": { "status": "SUCCESS" },
                  "reviewers": { "nodes": [{ "username": "me",
                      "mergeRequestInteraction": { "reviewState": "REVIEWED", "approved": false } }] },
                  "approvedBy": { "nodes": [] },
                  "diffStatsSummary": { "additions": 3, "deletions": 1 } },
                { "iid": "2", "headPipeline": { "status": "FAILED" },
                  "reviewers": { "nodes": [{ "username": "me",
                      "mergeRequestInteraction": { "reviewState": "UNREVIEWED", "approved": false } }] },
                  "approvedBy": { "nodes": [] },
                  "diffStatsSummary": { "additions": 40, "deletions": 0 } }
            ] } } } }),
        ),
        exchange(
            Method::Get,
            &list("issues", "assignee_username"),
            200,
            json!([gitlab_issue(5)]),
        ),
        exchange(
            Method::Get,
            &list("issues", "author_username"),
            200,
            json!([]),
        ),
        exchange(
            Method::Get,
            &format!("/api/v4/todos?type=Issue&state=pending&per_page={most}"),
            200,
            json!([
                { "action_name": "mentioned", "project": { "path_with_namespace": "g/sub/p" },
                  "target": gitlab_issue(6) },
                { "action_name": "assigned", "project": { "path_with_namespace": "g/sub/p" },
                  "target": gitlab_issue(9) }
            ]),
        ),
    ]
}

fn sign_ins() -> Vec<Exchange> {
    vec![
        exchange(Method::Get, "/user", 200, json!({ "login": "me", "id": 1 })),
        exchange(
            Method::Get,
            "/api/v4/user",
            200,
            json!({ "username": "me", "id": 2, "name": "Me" }),
        ),
        exchange(
            Method::Get,
            "/api/v4/personal_access_tokens/self",
            200,
            json!({ "scopes": ["api"] }),
        ),
    ]
}

struct Harness {
    dir: PathBuf,
    services: Arc<Services>,
    github_repo: Scratch,
    gitlab_repo: Scratch,
}

impl Harness {
    /// Both accounts connected, answered by `github` and the GitLab
    /// lists; two repositories here, the GitHub one with work in
    /// progress.
    fn new(github: Exchange) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-l-inbox-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut exchanges = sign_ins();
        exchanges.push(github);
        exchanges.extend(gitlab_lists(50));
        let replay = Replay::new(Recording {
            note: "stand-ins for the inbox's tests".to_owned(),
            hand_made: Some(
                "made by hand in the shapes of GitHub's GraphQL reference and GitLab's REST \
                 and GraphQL references: one account of each cannot show every state at once"
                    .to_owned(),
            ),
            exchanges,
        });
        let mut services = Services::new(Arc::new(replay), Arc::new(MemoryStore::new()), &dir);
        services.now = || NOW;
        let github_repo = Scratch::of("linear").unwrap();
        github_repo
            .git()
            .run(["remote", "add", "origin", "https://github.com/o/r.git"])
            .unwrap();
        github_repo.write("work.txt", "not committed\n").unwrap();
        let gitlab_repo = Scratch::of("linear").unwrap();
        gitlab_repo
            .git()
            .run([
                "remote",
                "add",
                "origin",
                "git@gitlab.example.org:g/sub/p.git",
            ])
            .unwrap();
        let h = Self {
            dir,
            services: Arc::new(services),
            github_repo,
            gitlab_repo,
        };
        let account = h.run(
            "account.add_token",
            json!({ "kind": "github", "token": "ghx_1" }),
        );
        assert_eq!(account["id"], GITHUB);
        let account = h.run(
            "account.add_token",
            json!({ "kind": "gitlab", "web": "https://gitlab.example.org", "token": "glx_1" }),
        );
        assert_eq!(account["id"], GITLAB);
        h
    }

    fn try_run(&self, id: &str, params: Value) -> Result<Value, CommandError> {
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        bitbraid_integrations::registry().run(id, &mut ctx, &params)
    }

    fn run(&self, id: &str, params: Value) -> Value {
        self.try_run(id, params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn paths(&self) -> Value {
        json!([
            self.github_repo.path().to_string_lossy(),
            self.gitlab_repo.path().to_string_lossy()
        ])
    }

    fn items(&self, extra: Value) -> Value {
        let mut params = json!({ "paths": self.paths() });
        for (key, value) in extra.as_object().cloned().unwrap_or_default() {
            params[key] = value;
        }
        self.run("inbox.items", params)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn github_graphql(status: u16, body: Value) -> Exchange {
    exchange(Method::Post, "/graphql", status, body)
}

fn row<'a>(rows: &'a Value, key: &str) -> &'a Value {
    rows.as_array()
        .unwrap()
        .iter()
        .find(|row| row["key"] == key)
        .unwrap_or_else(|| panic!("no row {key} in {rows:#}"))
}

fn keys(rows: &Value) -> Vec<String> {
    rows.as_array()
        .unwrap()
        .iter()
        .map(|row| row["key"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn every_account_answers_its_rows_with_their_groups_and_where_they_are() {
    let h = Harness::new(github_graphql(200, github_answer()));
    let found = h.items(json!({}));
    let states = found["accounts"].as_array().unwrap();
    assert_eq!(states.len(), 2, "{found:#}");
    assert!(
        states.iter().all(|state| state["error"].is_null()),
        "{found:#}"
    );
    let pulls = &found["pulls"];
    let here = h.github_repo.path().to_string_lossy().into_owned();
    let there = h.gitlab_repo.path().to_string_lossy().into_owned();

    let mine = row(pulls, "github:github.com:me/pull/o/r#1");
    assert_eq!(mine["groups"], json!(["no_reviewers", "failing_checks"]));
    assert_eq!(mine["involvement"], json!(["created"]));
    assert_eq!(mine["path"], here);
    assert_eq!(mine["remote"], "origin");
    assert_eq!(
        (mine["additions"].as_u64(), mine["deletions"].as_u64()),
        (Some(10), Some(2))
    );
    assert_eq!(mine["milestone"]["title"], "v1");
    assert_eq!(mine["pinned"], false);
    assert!(mine["snoozed"].is_null());

    let theirs = row(pulls, "github:github.com:me/pull/o/r#2");
    assert_eq!(theirs["groups"], json!(["review_mine"]));
    assert_eq!(theirs["involvement"], json!(["review_requested"]));
    assert_eq!(theirs["approval"], "required");

    let team = row(pulls, "github:github.com:me/pull/o/other#3");
    assert_eq!(
        team["groups"],
        json!(["conflicts", "waiting_for_review", "draft"])
    );
    assert_eq!(team["involvement"], json!(["review_requested"]));
    assert!(team["path"].is_null());

    let merge = row(pulls, "gitlab:gitlab.example.org:me/pull/g/sub/p#1");
    assert_eq!(merge["involvement"], json!(["created", "review_requested"]));
    // Its reviewer commented: not ready until the review is settled.
    assert_eq!(merge["groups"], json!(["commented"]));
    assert_eq!(merge["additions"], 3);
    assert_eq!(merge["pull"]["ci"], "success");
    assert_eq!(merge["path"], there);
    let other = row(pulls, "gitlab:gitlab.example.org:me/pull/g/sub/p#2");
    assert_eq!(other["groups"], json!(["review_mine", "failing_checks"]));
    // Five, the most recently changed first.
    assert_eq!(pulls.as_array().unwrap().len(), 5);
    let updated: Vec<&str> = pulls
        .as_array()
        .unwrap()
        .iter()
        .map(|pull| pull["pull"]["updated"].as_str().unwrap())
        .collect();
    let mut sorted = updated.clone();
    sorted.sort_by(|a, b| b.cmp(a));
    assert_eq!(updated, sorted);

    let issues = &found["issues"];
    let both = row(issues, "github:github.com:me/issue/o/r#7");
    assert_eq!(both["involvement"], json!(["assigned", "created"]));
    assert_eq!(both["reactions"], 4);
    assert_eq!(both["path"], here);
    let named = row(issues, "github:github.com:me/issue/o/elsewhere#8");
    assert_eq!(named["involvement"], json!(["mentioned"]));
    assert!(named["path"].is_null());
    let assigned = row(issues, "gitlab:gitlab.example.org:me/issue/g/sub/p#5");
    assert_eq!(assigned["reactions"], 3);
    let mentioned = row(issues, "gitlab:gitlab.example.org:me/issue/g/sub/p#6");
    assert_eq!(mentioned["involvement"], json!(["mentioned"]));
    // A to-do item that is not about naming the user is not one.
    assert_eq!(issues.as_array().unwrap().len(), 4, "{issues:#}");

    // The repository with a file not committed is work in progress.
    let work = found["work"].as_array().unwrap();
    assert_eq!(work.len(), 1, "{found:#}");
    assert_eq!(work[0]["path"], here);
    assert_eq!(work[0]["counts"]["added"], 1);

    // One account alone.
    let alone = h.items(json!({ "account": GITLAB, "wip": false }));
    assert_eq!(alone["accounts"].as_array().unwrap().len(), 1);
    assert!(
        keys(&alone["pulls"])
            .iter()
            .all(|key| key.starts_with("gitlab:"))
    );
    assert!(alone["work"].as_array().unwrap().is_empty());
    assert!(
        h.try_run(
            "inbox.items",
            json!({ "account": "github:github.com:nobody" })
        )
        .is_err()
    );
}

#[test]
fn pins_and_snoozes_are_kept_and_shown_on_the_rows() {
    let h = Harness::new(github_graphql(200, github_answer()));
    let key = "gitlab:gitlab.example.org:me/pull/g/sub/p#2";
    let marks = h.run("inbox.pin", json!({ "key": key }));
    assert_eq!(marks["pin"][0]["key"], key);
    assert_eq!(marks["pin"][0]["since"], NOW);
    // Pinning twice keeps one pin.
    let marks = h.run("inbox.pin", json!({ "key": key }));
    assert_eq!(marks["pin"].as_array().unwrap().len(), 1);
    let found = h.items(json!({ "wip": false }));
    assert_eq!(keys(&found["pulls"])[0], key);
    assert_eq!(row(&found["pulls"], key)["pinned"], true);

    let snoozed = "github:github.com:me/issue/o/r#7";
    h.run("inbox.snooze", json!({ "key": snoozed, "for": "hour" }));
    let found = h.items(json!({ "wip": false }));
    assert_eq!(
        row(&found["issues"], snoozed)["snoozed"],
        json!({ "since": NOW, "until": NOW + 3600 })
    );
    // Snoozed again, it lasts until it is brought back.
    h.run("inbox.snooze", json!({ "key": snoozed }));
    let found = h.items(json!({ "wip": false }));
    assert_eq!(
        row(&found["issues"], snoozed)["snoozed"],
        json!({ "since": NOW, "until": null })
    );
    let until = h.run("inbox.snooze", json!({ "key": snoozed, "until": NOW + 60 }));
    assert_eq!(until["snooze"][0]["until"], NOW + 60);
    let tomorrow = h.run("inbox.snooze", json!({ "key": snoozed, "for": "tomorrow" }));
    let ends = tomorrow["snooze"][0]["until"].as_i64().unwrap();
    assert!(ends > NOW && ends <= NOW + 2 * 86_400, "{ends}");

    h.run("inbox.unsnooze", json!({ "key": snoozed }));
    h.run("inbox.unpin", json!({ "key": key }));
    let found = h.items(json!({ "wip": false }));
    assert!(row(&found["issues"], snoozed)["snoozed"].is_null());
    assert_eq!(row(&found["pulls"], key)["pinned"], false);
    let text = std::fs::read_to_string(h.dir.join("inbox.toml")).unwrap();
    assert!(
        !text.contains("[[pin]]") && !text.contains("[[snooze]]"),
        "{text}"
    );
}

#[test]
fn a_workspace_holds_inbox_to_its_repositories() {
    let h = Harness::new(github_graphql(200, github_answer()));
    let made = h.run(
        "workspace.create",
        json!({ "name": "Platform", "paths": [h.gitlab_repo.path().to_string_lossy()] }),
    );
    let id = made["workspace"]["id"].as_str().unwrap().to_owned();
    let found = h.items(json!({ "workspace": id }));
    // GitHub has none of its repositories: it is not asked.
    let states = found["accounts"].as_array().unwrap();
    assert_eq!(states.len(), 1, "{found:#}");
    assert_eq!(states[0]["account"], GITLAB);
    assert!(
        keys(&found["pulls"])
            .iter()
            .all(|key| key.starts_with("gitlab:"))
    );
    assert_eq!(found["pulls"].as_array().unwrap().len(), 2);
    // Work in progress only of its repositories.
    assert!(found["work"].as_array().unwrap().is_empty());
    assert!(
        h.try_run(
            "inbox.items",
            json!({ "workspace": "nothing-of-that-name" })
        )
        .is_err()
    );
}

#[test]
fn an_account_that_cannot_be_asked_says_why_and_the_other_answers() {
    let h = Harness::new(github_graphql(502, json!({ "message": "bad gateway" })));
    let found = h.items(json!({ "wip": false }));
    let states = found["accounts"].as_array().unwrap();
    let github = states
        .iter()
        .find(|state| state["account"] == GITHUB)
        .unwrap();
    assert!(github["error"].is_string(), "{found:#}");
    let gitlab = states
        .iter()
        .find(|state| state["account"] == GITLAB)
        .unwrap();
    assert!(gitlab["error"].is_null());
    assert_eq!(found["pulls"].as_array().unwrap().len(), 2);
}

#[test]
fn hostile_answers_and_parameters_are_refused_or_passed_over() {
    // Wrong shapes, missing numbers, huge numbers, control characters.
    let hostile = json!({ "data": {
        "pulls": { "nodes": [
            { "number": u64::MAX, "title": "a\u{0}b\nc", "baseRepository": { "nameWithOwner": "o/r" },
              "additions": u64::MAX, "mergeable": 7, "reviewDecision": ["x"] },
            { "number": 4, "baseRepository": { "nameWithOwner": "../../etc" } },
            { "number": 5 },
            null, 3, "text"
        ] },
        "review": null,
        "assigned": { "nodes": [{ "number": 1, "repository": { "nameWithOwner": "no-slash" } }] },
        "created": "nothing",
        "mentioned": { "pageInfo": { "hasNextPage": "yes" }, "nodes": [] }
    }});
    let h = Harness::new(github_graphql(200, hostile));
    let found = h.try_run("inbox.items", json!({ "paths": h.paths(), "wip": false }));
    // Either the answer is refused as a whole, or what is left is clean.
    if let Ok(found) = found {
        for pull in found["pulls"].as_array().unwrap() {
            let title = pull["pull"]["title"].as_str().unwrap();
            assert!(!title.chars().any(char::is_control), "{title:?}");
        }
    }
    for (id, params) in [
        ("inbox.pin", json!({})),
        ("inbox.pin", json!({ "key": "" })),
        ("inbox.pin", json!({ "key": "a b" })),
        ("inbox.pin", json!({ "key": "x".repeat(601) })),
        ("inbox.pin", json!({ "key": 5 })),
        ("inbox.unpin", json!({ "key": "a\u{7}" })),
        ("inbox.snooze", json!({ "key": "k", "for": "week" })),
        ("inbox.snooze", json!({ "key": "k", "until": NOW - 1 })),
        ("inbox.snooze", json!({ "key": "k", "until": "soon" })),
        ("inbox.snooze", json!({ "key": "k", "until": i64::MIN })),
        (
            "inbox.snooze",
            json!({ "key": "k", "for": "hour", "until": NOW + 5 }),
        ),
        ("inbox.items", json!({ "paths": "/a" })),
        ("inbox.items", json!({ "paths": ["relative/path"] })),
        ("inbox.items", json!({ "workspace": 3 })),
        ("inbox.items", json!({ "workspace": "x".repeat(65) })),
        ("inbox.items", json!({ "wip": "yes" })),
    ] {
        assert!(h.try_run(id, params.clone()).is_err(), "{id} {params}");
    }
    // A snooze far in the future is taken as it is.
    let far = h.run("inbox.snooze", json!({ "key": "k", "until": i64::MAX }));
    assert_eq!(far["snooze"][0]["until"], i64::MAX);
    // A file of marks that cannot be read: the rows come without marks
    // and say why; a change is refused and the file is left alone.
    std::fs::write(h.dir.join("inbox.toml"), "[[pin]\n").unwrap();
    let found = h.items(json!({ "wip": false }));
    assert!(found["marks_problem"].is_string());
    assert!(h.try_run("inbox.pin", json!({ "key": "k" })).is_err());
    assert_eq!(
        std::fs::read_to_string(h.dir.join("inbox.toml")).unwrap(),
        "[[pin]\n"
    );
}
