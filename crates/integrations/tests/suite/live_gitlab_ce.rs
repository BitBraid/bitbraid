//! The provider commands against a GitLab of one's own, live: connecting
//! an account by token, its merge requests, and signing in by a code.
//!
//! Opt-in and never in ci: it runs with `--ignored` when
//! `BITBRAID_GITLAB_CE` names the server (`http://127.0.0.1:8929`) and
//! `BITBRAID_GITLAB_CE_TOKEN_FILE` a file holding an administrator's
//! personal access token (scope `api`), which is read here and sent to
//! that server only:
//!
//! ```sh
//! BITBRAID_GITLAB_CE=http://127.0.0.1:8929 BITBRAID_GITLAB_CE_TOKEN_FILE=<file> \
//!     cargo test -p bitbraid-integrations --test suite live_gitlab_ce:: -- --ignored
//! ```
//!
//! The recordings were made against a GitLab CE in Docker with its web
//! port published on 127.0.0.1:8929, the address their replays expect;
//! the test itself takes any server on this machine. It first makes
//! what it needs through the administrator's API, as many times as it is
//! run without making it twice: the users `bb-p-alice` and `bb-p-bob`
//! with tokens of their own, the group `bb-p-group`, its project
//! `bb-p-project` with a merge request from `bb-p-alice`, and an
//! application for the device flow that is not confidential. Passwords
//! and tokens are made for the run and kept nowhere.
//!
//! GitLab makes an application with its device grant off, and only its
//! web form turns it on: the test of signing in by code makes a user of
//! its own run, signs in at the site as them (a browser of the test's own,
//! its cookies in memory), makes the application with the grant on
//! through the form, and approves the code there. Users of one run
//! (`bb-p3-<what>-<hex>`) are removed at its end with what they made.
//!
//! With `BITBRAID_RECORD=<dir>` the answers of the commands (not of the
//! setup) are recorded there, scrubbed (`bitbraid_integrations::replay`):
//! `crates/integrations/tests/recordings/gitlab/` keeps them, and
//! `tests/suite/recorded.rs` replays them in ci.
//!
//! Sources: docs.gitlab.com/api "Users" (create, personal access tokens of
//! a user), "Groups", "Group and project members", "Projects", "Commits"
//! (create with actions), "Branches", "Merge requests", "Applications".
#![allow(clippy::print_stderr)]

use std::{
    path::PathBuf,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    api::{encode, rfc3339},
    http::{Call, Client, Fetch, Limits, Method, Url},
    replay::Recorder,
    secrets::{MemoryStore, Secret},
};
use serde_json::{Value, json};
use sha2::Digest as _;

const GROUP: &str = "bb-p-group";
const PROJECT: &str = "bb-p-project";
const BRANCH: &str = "bb-p-topic";
const APPLICATION: &str = "bb-p-device";

/// The server and the administrator's token, when the test is asked for.
fn server() -> Option<(String, Secret)> {
    let base = std::env::var("BITBRAID_GITLAB_CE").ok()?;
    let file = std::env::var("BITBRAID_GITLAB_CE_TOKEN_FILE").ok()?;
    let token = std::fs::read_to_string(file).ok()?.trim().to_owned();
    let base = base.trim().trim_end_matches('/').to_owned();
    // Only a server on this machine.
    let url = Url::parse(&base).ok()?;
    let local = url.host() == "localhost"
        || url
            .host()
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    local.then(|| (base, Secret::new(token)))
}

/// Something new for this run: a password or a token's name.
fn fresh(what: &str) -> String {
    let seed = format!(
        "{what}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    );
    let hash = sha2::Sha256::digest(seed.as_bytes());
    let hex: String = hash
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect();
    format!("Bb-{hex}-p9")
}

/// The administrator's API, or a user's.
struct Api {
    client: Client,
    base: String,
    token: Secret,
}

impl Api {
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = Url::parse(&format!("{}/api/v4{path}", self.base)).unwrap();
        let mut call = Call::new(
            method,
            url,
            Limits {
                connect: Duration::from_secs(10),
                total: Duration::from_secs(120),
                body: 16 << 20,
            },
        )
        .with_header("Accept", "application/json")
        .with_header("Authorization", &format!("Bearer {}", self.token.expose()));
        if let Some(body) = body {
            call = call.with_body("application/json", body.to_string().into_bytes());
        }
        let response = self
            .client
            .send(&call, &AtomicBool::new(false))
            .unwrap_or_else(|err| panic!("{} {path}: {err}", method.as_str()));
        let value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        (response.status, value)
    }

    fn ok(&self, method: Method, path: &str, body: Option<Value>) -> Value {
        let (status, value) = self.call(method, path, body);
        assert!(
            (200..300).contains(&status),
            "{} {path}: {status} {}",
            method.as_str(),
            value.get("message").cloned().unwrap_or_default()
        );
        value
    }

    fn as_user(&self, token: &Secret) -> Self {
        Self {
            client: Client::new("BitBraid-Test/live"),
            base: self.base.clone(),
            token: token.clone(),
        }
    }
}

/// A user of the test: made when missing, a new password and a new token
/// for this run.
struct Person {
    id: u64,
    username: String,
    name: String,
    email: String,
    password: String,
    token: Secret,
}

fn person(admin: &Api, username: &str, name: &str) -> Person {
    let (_, found) = admin.call(Method::Get, &format!("/users?username={username}"), None);
    let email = format!("{username}@example.invalid");
    // A password is set when the user is made, and never again: one an
    // administrator sets has run out at once, and with it the user's
    // tokens stop working. It is kept nowhere; nothing here signs in
    // with it.
    let password = fresh(username);
    let id = match found.as_array().and_then(|users| users.first()) {
        Some(user) => user["id"].as_u64().unwrap(),
        None => admin.ok(
            Method::Post,
            "/users",
            Some(json!({ "username": username, "name": name, "email": email,
                         "password": password, "skip_confirmation": true })),
        )["id"]
            .as_u64()
            .unwrap(),
    };
    let expires = rfc3339(bitbraid_core::clock::unix_seconds() + 7 * 86_400);
    let made = admin.ok(
        Method::Post,
        &format!("/users/{id}/personal_access_tokens"),
        Some(
            json!({ "name": fresh("token"), "scopes": ["api", "read_user"],
                     "expires_at": &expires[..10] }),
        ),
    );
    Person {
        id,
        username: username.to_owned(),
        name: name.to_owned(),
        email,
        password,
        token: Secret::new(made["token"].as_str().unwrap()),
    }
}

/// What the test works with, made when missing.
struct Setup {
    alice: Person,
    bob: Person,
    application: String,
}

fn setup(admin: &Api) -> Setup {
    let alice = person(admin, "bb-p-alice", "Alice Tester");
    let bob = person(admin, "bb-p-bob", "Bob Tester");
    let (status, group) = admin.call(Method::Get, &format!("/groups/{GROUP}"), None);
    let group_id = if status == 200 {
        group["id"].as_u64().unwrap()
    } else {
        admin.ok(
            Method::Post,
            "/groups",
            Some(json!({ "name": GROUP, "path": GROUP, "visibility": "private" })),
        )["id"]
            .as_u64()
            .unwrap()
    };
    for (user, level) in [(alice.id, 40), (bob.id, 30)] {
        // Already a member: a conflict, which is fine.
        let _ = admin.call(
            Method::Post,
            &format!("/groups/{group_id}/members"),
            Some(json!({ "user_id": user, "access_level": level })),
        );
    }
    let path = encode(&format!("{GROUP}/{PROJECT}"));
    let (status, project) = admin.call(Method::Get, &format!("/projects/{path}"), None);
    let project_id = if status == 200 {
        project["id"].as_u64().unwrap()
    } else {
        admin.ok(
            Method::Post,
            "/projects",
            Some(
                json!({ "name": PROJECT, "path": PROJECT, "namespace_id": group_id,
                         "initialize_with_readme": true, "default_branch": "main",
                         "visibility": "private" }),
            ),
        )["id"]
            .as_u64()
            .unwrap()
    };
    let (status, _) = admin.call(
        Method::Get,
        &format!("/projects/{project_id}/repository/branches/{BRANCH}"),
        None,
    );
    if status != 200 {
        admin.ok(
            Method::Post,
            &format!("/projects/{project_id}/repository/commits"),
            Some(json!({ "branch": BRANCH, "start_branch": "main",
                         "commit_message": "Add a file to review",
                         "actions": [ { "action": "create", "file_path": "bb-p.txt",
                                        "content": "a line to review\n" } ] })),
        );
    }
    let (_, open) = admin.call(
        Method::Get,
        &format!("/projects/{project_id}/merge_requests?state=opened&source_branch={BRANCH}"),
        None,
    );
    if open.as_array().is_none_or(Vec::is_empty) {
        admin.as_user(&alice.token).ok(
            Method::Post,
            &format!("/projects/{project_id}/merge_requests"),
            Some(json!({ "source_branch": BRANCH, "target_branch": "main",
                         "title": "Add a file to review", "description": "Made by the live test." })),
        );
    }
    let (_, applications) = admin.call(Method::Get, "/applications", None);
    let application = applications
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|application| application["application_name"] == APPLICATION)
        })
        .and_then(|application| application["application_id"].as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| {
            admin.ok(
                Method::Post,
                "/applications",
                Some(
                    json!({ "name": APPLICATION, "redirect_uri": "http://127.0.0.1/unused",
                             "scopes": "api read_user", "confidential": false }),
                ),
            )["application_id"]
                .as_str()
                .unwrap()
                .to_owned()
        });
    Setup {
        alice,
        bob,
        application,
    }
}

/// What the reads are tested on, made when missing.
struct Reads {
    project_id: u64,
    /// The merge request from `bb-p-topic`: labelled, with a milestone,
    /// an assignee, a reviewer who approved, conversations on its lines
    /// (one resolved) and a comment.
    topic: u64,
    /// A draft.
    draft: u64,
    /// Two stacked: the second's base is the first's branch.
    stack: [u64; 2],
    /// One merged, one closed.
    merged: u64,
    closed: u64,
    /// One from `bb-p-bob`'s fork.
    fork: u64,
    /// The head commits of `topic`, `draft` and the first of the stack.
    shas: [String; 3],
    /// An issue with comments, labelled, with a milestone, assigned to
    /// bob, naming alice; and a closed one.
    issue: u64,
    closed_issue: u64,
}

/// A branch with one commit of its own, made when missing.
fn branch_with_commit(api: &Api, project: u64, branch: &str, from: &str, file: &str) {
    let (status, _) = api.call(
        Method::Get,
        &format!("/projects/{project}/repository/branches/{branch}"),
        None,
    );
    if status == 200 {
        return;
    }
    // A branch merged and removed before: its file is in `from` already,
    // and its merge request is found without it.
    let (status, said) = api.call(
        Method::Post,
        &format!("/projects/{project}/repository/commits"),
        Some(json!({ "branch": branch, "start_branch": from,
                     "commit_message": format!("Change {file}"),
                     "actions": [ { "action": "create", "file_path": file,
                                    "content": format!("a line of {file}\nand another\n") } ] })),
    );
    assert!(
        (200..300).contains(&status) || said.to_string().contains("already exists"),
        "{branch}: {status} {said}"
    );
}

/// The merge request from `source` (of `from_project`) into `target` of
/// `project`, made by `api`'s user when missing: its iid.
fn merge_request(
    api: &Api,
    project: u64,
    from_project: u64,
    source: &str,
    target: &str,
    title: &str,
) -> u64 {
    let (_, found) = api.call(
        Method::Get,
        &format!("/projects/{project}/merge_requests?state=all&source_branch={source}"),
        None,
    );
    if let Some(iid) = found
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|merge| merge["source_project_id"] == from_project)
        })
        .and_then(|merge| merge["iid"].as_u64())
    {
        return iid;
    }
    api.ok(
        Method::Post,
        &format!("/projects/{from_project}/merge_requests"),
        Some(
            json!({ "source_branch": source, "target_branch": target, "title": title,
                     "target_project_id": project,
                     "description": format!("Made by the live test: {title}.") }),
        ),
    )["iid"]
        .as_u64()
        .unwrap()
}

/// The issue called `title`, made by `api`'s user when missing: its iid.
fn issue(api: &Api, project: u64, title: &str, extra: Value) -> u64 {
    let (_, found) = api.call(
        Method::Get,
        &format!(
            "/projects/{project}/issues?state=all&search={}",
            encode(title)
        ),
        None,
    );
    if let Some(iid) = found
        .as_array()
        .and_then(|all| all.iter().find(|issue| issue["title"] == title))
        .and_then(|issue| issue["iid"].as_u64())
    {
        return iid;
    }
    let mut body = json!({ "title": title });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    api.ok(
        Method::Post,
        &format!("/projects/{project}/issues"),
        Some(body),
    )["iid"]
        .as_u64()
        .unwrap()
}

/// A comment on a merge request or an issue (`what` is
/// `merge_requests/2` or `issues/1`), written when no comment says it.
fn comment_once(api: &Api, project: u64, what: &str, body: &str) {
    let (_, notes) = api.call(
        Method::Get,
        &format!("/projects/{project}/{what}/notes?per_page=100"),
        None,
    );
    if notes
        .as_array()
        .is_some_and(|notes| notes.iter().any(|note| note["body"] == body))
    {
        return;
    }
    api.ok(
        Method::Post,
        &format!("/projects/{project}/{what}/notes"),
        Some(json!({ "body": body })),
    );
}

/// The head commit of a branch.
fn head(api: &Api, project: u64, branch: &str) -> String {
    api.ok(
        Method::Get,
        &format!("/projects/{project}/repository/branches/{branch}"),
        None,
    )["commit"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

fn setup_reads(admin: &Api, made: &Setup) -> Reads {
    let alice = admin.as_user(&made.alice.token);
    let bob = admin.as_user(&made.bob.token);
    let path = encode(&format!("{GROUP}/{PROJECT}"));
    let project = admin.ok(Method::Get, &format!("/projects/{path}"), None)["id"]
        .as_u64()
        .unwrap();
    // Labels and a milestone.
    for (name, color, description) in [
        ("bug", "#d9534f", "Something is broken"),
        ("ui", "#428bca", "How it looks"),
    ] {
        let _ = admin.call(
            Method::Post,
            &format!("/projects/{project}/labels"),
            Some(json!({ "name": name, "color": color, "description": description })),
        );
    }
    let (_, milestones) = admin.call(
        Method::Get,
        &format!("/projects/{project}/milestones?title=v1"),
        None,
    );
    let milestone = match milestones.as_array().and_then(|all| all.first()) {
        Some(found) => found["id"].as_u64().unwrap(),
        None => admin.ok(
            Method::Post,
            &format!("/projects/{project}/milestones"),
            Some(json!({ "title": "v1", "due_date": "2026-12-31" })),
        )["id"]
            .as_u64()
            .unwrap(),
    };
    // The first merge request, filled in.
    let topic = merge_request(
        &alice,
        project,
        project,
        BRANCH,
        "main",
        "Add a file to review",
    );
    alice.ok(
        Method::Put,
        &format!("/projects/{project}/merge_requests/{topic}"),
        Some(json!({ "labels": "bug", "milestone_id": milestone,
                     "assignee_ids": [made.alice.id], "reviewer_ids": [made.bob.id] })),
    );
    // Approved already: refused, which is fine.
    let _ = bob.call(
        Method::Post,
        &format!("/projects/{project}/merge_requests/{topic}/approve"),
        None,
    );
    let one = format!("/projects/{project}/merge_requests/{topic}");
    let (_, discussions) = admin.call(Method::Get, &format!("{one}/discussions"), None);
    let on_lines = discussions
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|discussion| discussion["notes"][0]["position"].is_object())
                .count()
        })
        .unwrap_or(0);
    if on_lines < 2 {
        let refs = admin.ok(Method::Get, &one, None)["diff_refs"].clone();
        for (body, resolve) in [
            ("This line could say more.", true),
            ("Is this the right file for it?", false),
        ] {
            let made_one = bob.ok(
                Method::Post,
                &format!("{one}/discussions"),
                Some(json!({ "body": body, "position": {
                    "position_type": "text", "base_sha": refs["base_sha"],
                    "start_sha": refs["start_sha"], "head_sha": refs["head_sha"],
                    "new_path": "bb-p.txt", "new_line": 1 } })),
            );
            let id = made_one["id"].as_str().unwrap().to_owned();
            alice.ok(
                Method::Post,
                &format!("{one}/discussions/{id}/notes"),
                Some(json!({ "body": if resolve { "Done." } else { "I think so." } })),
            );
            if resolve {
                alice.ok(
                    Method::Put,
                    &format!("{one}/discussions/{id}?resolved=true"),
                    None,
                );
            }
        }
    }
    comment_once(
        &bob,
        project,
        &format!("merge_requests/{topic}"),
        "Looks good to me.",
    );
    // A draft.
    branch_with_commit(&alice, project, "bb-p-draft", "main", "bb-p-draft.txt");
    let draft = merge_request(
        &alice,
        project,
        project,
        "bb-p-draft",
        "main",
        "Draft: Try another way",
    );
    alice.ok(
        Method::Put,
        &format!("/projects/{project}/merge_requests/{draft}"),
        Some(json!({ "labels": "ui", "reviewer_ids": [made.alice.id] })),
    );
    // Two stacked.
    branch_with_commit(&alice, project, "bb-p-stack-1", "main", "bb-p-stack-1.txt");
    branch_with_commit(
        &alice,
        project,
        "bb-p-stack-2",
        "bb-p-stack-1",
        "bb-p-stack-2.txt",
    );
    let first = merge_request(
        &alice,
        project,
        project,
        "bb-p-stack-1",
        "main",
        "First of a stack",
    );
    let second = merge_request(
        &alice,
        project,
        project,
        "bb-p-stack-2",
        "bb-p-stack-1",
        "Second of a stack",
    );
    // One merged, one closed.
    branch_with_commit(&alice, project, "bb-p-merged", "main", "bb-p-merged.txt");
    let merged = merge_request(
        &alice,
        project,
        project,
        "bb-p-merged",
        "main",
        "Merged already",
    );
    for _ in 0..30 {
        let state = admin.ok(
            Method::Get,
            &format!("/projects/{project}/merge_requests/{merged}"),
            None,
        )["state"]
            .as_str()
            .unwrap()
            .to_owned();
        if state == "merged" {
            break;
        }
        let _ = alice.call(
            Method::Put,
            &format!("/projects/{project}/merge_requests/{merged}/merge"),
            None,
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    branch_with_commit(&alice, project, "bb-p-closed", "main", "bb-p-closed.txt");
    let closed = merge_request(
        &alice,
        project,
        project,
        "bb-p-closed",
        "main",
        "Closed without merging",
    );
    alice.ok(
        Method::Put,
        &format!("/projects/{project}/merge_requests/{closed}"),
        Some(json!({ "state_event": "close" })),
    );
    // A fork of bob's, and a merge request from it.
    let fork_path = encode(&format!("{}/{PROJECT}", made.bob.username));
    let (status, fork) = admin.call(Method::Get, &format!("/projects/{fork_path}"), None);
    let fork_id = if status == 200 {
        fork["id"].as_u64().unwrap()
    } else {
        bob.ok(
            Method::Post,
            &format!("/projects/{project}/fork"),
            Some(json!({ "namespace_path": made.bob.username })),
        )["id"]
            .as_u64()
            .unwrap()
    };
    for _ in 0..60 {
        let state = admin.ok(Method::Get, &format!("/projects/{fork_id}"), None)["import_status"]
            .as_str()
            .unwrap_or("finished")
            .to_owned();
        if state == "finished" || state == "none" {
            break;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
    // Alice may see the fork, so that its path can be told to her. A
    // member already: a conflict, which is fine.
    let _ = admin.call(
        Method::Post,
        &format!("/projects/{fork_id}/members"),
        Some(json!({ "user_id": made.alice.id, "access_level": 20 })),
    );
    branch_with_commit(&bob, fork_id, "bb-p-fork-topic", "main", "bb-p-fork.txt");
    let from_fork = merge_request(
        &bob,
        project,
        fork_id,
        "bb-p-fork-topic",
        "main",
        "A change from a fork",
    );
    // Statuses set on commits, as a service would: success and failure
    // on the first, one running on the first of the stack, one waiting on
    // the draft. Set twice: the server refuses, which is fine.
    let shas = [
        head(admin, project, BRANCH),
        head(admin, project, "bb-p-draft"),
        head(admin, project, "bb-p-stack-1"),
    ];
    for (sha, branch, name, state) in [
        (&shas[0], BRANCH, "bb-p-lint", "success"),
        (&shas[0], BRANCH, "bb-p-test", "failed"),
        (&shas[1], "bb-p-draft", "bb-p-test", "pending"),
        (&shas[2], "bb-p-stack-1", "bb-p-test", "running"),
    ] {
        let _ = alice.call(
            Method::Post,
            &format!("/projects/{project}/statuses/{sha}"),
            Some(json!({ "state": state, "name": name, "ref": branch,
                         "description": format!("{name} says {state}"),
                         "target_url": format!("http://127.0.0.1:8929/{GROUP}/{PROJECT}/-/pipelines") })),
        );
    }
    // Issues.
    let issue_one = issue(
        &alice,
        project,
        "The button does nothing",
        json!({ "description": "Clicking it does nothing at all.", "labels": "bug",
                 "milestone_id": milestone, "assignee_ids": [made.bob.id] }),
    );
    comment_once(
        &bob,
        project,
        &format!("issues/{issue_one}"),
        &format!("@{} which button do you mean?", made.alice.username),
    );
    comment_once(
        &alice,
        project,
        &format!("issues/{issue_one}"),
        "The one at the top.",
    );
    let closed_issue = issue(
        &bob,
        project,
        "Colours are off",
        json!({ "description": "Too bright.", "labels": "ui" }),
    );
    let _ = bob.call(
        Method::Put,
        &format!("/projects/{project}/issues/{closed_issue}"),
        Some(json!({ "state_event": "close" })),
    );
    // Alice's own project, and the group's starred.
    let own = encode(&format!("{}/bb-p-own", made.alice.username));
    let (status, _) = admin.call(Method::Get, &format!("/projects/{own}"), None);
    if status != 200 {
        alice.ok(
            Method::Post,
            "/projects",
            Some(
                json!({ "name": "bb-p-own", "path": "bb-p-own", "visibility": "private",
                         "initialize_with_readme": true,
                         "description": "Alice's own project" }),
            ),
        );
    }
    let _ = alice.call(Method::Post, &format!("/projects/{project}/star"), None);
    Reads {
        project_id: project,
        topic,
        draft,
        stack: [first, second],
        merged,
        closed,
        fork: from_fork,
        shas,
        issue: issue_one,
        closed_issue,
    }
}

/// The commands' services: the real client, recorded when asked, a
/// keychain in memory, a scratch configuration directory.
struct Live {
    dir: PathBuf,
    services: Arc<Services>,
    recorder: Option<Arc<Recorder>>,
}

impl Live {
    fn new(name: &str, note: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bb-p-live-gitlab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let client: Arc<dyn Fetch> = Arc::new(Client::new("BitBraid-Test/live"));
        let recorder = Recorder::from_environment(client.clone(), name, note).map(Arc::new);
        let fetch: Arc<dyn Fetch> = match &recorder {
            Some(recorder) => recorder.clone(),
            None => client,
        };
        let services = Services::new(fetch, Arc::new(MemoryStore::new()), &dir);
        Self {
            dir,
            services: Arc::new(services),
            recorder,
        }
    }

    fn try_run(&self, id: &str, params: Value) -> Result<Value, String> {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        registry
            .run(id, &mut ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn run(&self, id: &str, params: Value) -> Value {
        self.try_run(id, params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn finish(&self, people: &[&Person]) {
        if let Some(recorder) = &self.recorder {
            let mut real: Vec<String> = Vec::new();
            for person in people {
                real.extend([
                    person.username.clone(),
                    person.id.to_string(),
                    person.name.clone(),
                    person.email.clone(),
                ]);
            }
            let real: Vec<&str> = real.iter().map(String::as_str).collect();
            let path = recorder.finish(&real).unwrap();
            eprintln!("recorded {}", path.display());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn a_token_connects_and_the_merge_requests_are_listed() {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    let made = setup(&admin);
    let live = Live::new(
        "add_token_and_pr_list",
        "Recorded from GitLab CE 19.4 on this machine by tests/suite/live_gitlab_ce.rs",
    );
    let account = live.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": base, "token": made.alice.token.expose() }),
    );
    assert_eq!(account["login"], made.alice.username);
    assert_eq!(account["scopes"], json!(["api", "read_user"]));
    let listing = live.run(
        "pr.list",
        json!({ "account": account["id"], "owner": GROUP, "name": PROJECT, "state": "all" }),
    );
    let items = listing["items"].as_array().unwrap();
    assert!(
        items.iter().any(
            |pull| pull["head_ref"] == BRANCH && pull["author"]["login"] == made.alice.username
        ),
        "{listing}"
    );
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn signing_in_by_code_goes_through_the_site() {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    let made = setup(&admin);
    let live = Live::new(
        "device_flow",
        "Recorded from GitLab CE 19.4 on this machine by tests/suite/live_gitlab_ce.rs",
    );
    // An application made through the API cannot sign in by code: GitLab
    // (18.10 and later) makes every application with its device grant
    // turned off (`device_code_enabled`), which only its web form turns
    // on, and answers `access_denied` until then.
    let refused = live
        .try_run(
            "account.sign_in_start",
            json!({ "kind": "gitlab", "web": base, "client_id": made.application }),
        )
        .unwrap_err();
    assert!(refused.contains("denied the request"), "{refused}");
    // So a user of this run signs in at the site, makes an application
    // with the grant turned on through the form, and approves the code
    // there, as a person would.
    let dana = passing_person(&admin, "dana");
    let _gone = Gone {
        admin: &admin,
        id: dana.id,
    };
    let mut browser = Browser::new(&base);
    browser.sign_in(&dana.username, &dana.password);
    let client_id = browser.application("bb-p3-device", "api read_user");
    let code = live.run(
        "account.sign_in_start",
        json!({ "kind": "gitlab", "web": base, "client_id": client_id }),
    );
    assert!(
        code["verification_uri"]
            .as_str()
            .unwrap()
            .starts_with(&base),
        "{code}"
    );
    let poll = json!({ "kind": "gitlab", "web": base, "device_code": code["device_code"] });
    let waiting = live.run("account.sign_in_poll", poll.clone());
    assert_eq!(waiting, json!({ "state": "waiting" }));
    browser.approve(code["user_code"].as_str().unwrap());
    // Asked again after the interval GitLab gave.
    std::thread::sleep(Duration::from_secs(code["interval"].as_u64().unwrap_or(5)));
    let done = live.run("account.sign_in_poll", poll);
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["account"]["login"], dana.username);
    assert_eq!(done["account"]["sign_in"], "device");
    // The token works.
    let namespaces = live.run(
        "provider.namespaces",
        json!({ "account": done["account"]["id"] }),
    );
    assert_eq!(namespaces[0]["path"], dana.username);
    live.finish(&[&made.alice, &made.bob, &dana]);
}

/// The site as a person's browser sees it, on this machine only: its
/// cookies kept in memory for this test and nowhere else, each request
/// a connection of its own.
struct Browser {
    address: String,
    cookies: std::collections::BTreeMap<String, String>,
}

/// An answer of the site: its status, header lines and body.
struct Page {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Page {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(known, _)| known.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The form's token against forged requests.
    fn csrf(&self) -> String {
        let at = self.body.find("name=\"csrf-token\" content=\"").unwrap() + 27;
        let rest = &self.body[at..];
        rest[..rest.find('"').unwrap()]
            .replace("&#x2F;", "/")
            .replace("&#43;", "+")
            .replace("&#x3D;", "=")
    }
}

/// `application/x-www-form-urlencoded`.
fn form_of(pairs: &[(&str, &str)]) -> String {
    String::from_utf8(bitbraid_integrations::api::form(pairs)).unwrap()
}

impl Browser {
    fn new(base: &str) -> Self {
        let url = Url::parse(base).unwrap();
        Self {
            address: format!("{}:{}", url.host(), url.port()),
            cookies: std::collections::BTreeMap::new(),
        }
    }

    fn ask(&mut self, method: &str, path: &str, form: Option<&str>) -> Page {
        use std::io::{Read as _, Write as _};
        let mut stream = std::net::TcpStream::connect(&self.address).unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(60)))
            .unwrap();
        let cookie = self
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ");
        let mut head = format!(
            "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\
             Accept: text/html\r\nUser-Agent: BitBraid-Test/live\r\n",
            self.address
        );
        if !cookie.is_empty() {
            head.push_str(&format!("Cookie: {cookie}\r\n"));
        }
        let body = form.unwrap_or_default();
        if form.is_some() {
            head.push_str(&format!(
                "Content-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n",
                body.len()
            ));
        }
        head.push_str("\r\n");
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(body.as_bytes()).unwrap();
        let mut bytes = Vec::new();
        stream.read_to_end(&mut bytes).unwrap();
        let split = bytes.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let text = String::from_utf8_lossy(&bytes[..split]).into_owned();
        let mut lines = text.split("\r\n");
        let status = lines
            .next()
            .unwrap()
            .split(' ')
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let headers: Vec<(String, String)> = lines
            .filter_map(|line| line.split_once(':'))
            .map(|(name, value)| (name.trim().to_owned(), value.trim().to_owned()))
            .collect();
        for (name, value) in &headers {
            if name.eq_ignore_ascii_case("set-cookie")
                && let Some((pair, _)) = value.split_once(';').or(Some((value.as_str(), "")))
                && let Some((name, value)) = pair.split_once('=')
            {
                self.cookies.insert(name.to_owned(), value.to_owned());
            }
        }
        let mut body = bytes[split + 4..].to_vec();
        let chunked = headers.iter().any(|(name, value)| {
            name.eq_ignore_ascii_case("transfer-encoding") && value.contains("chunked")
        });
        if chunked {
            let mut out = Vec::new();
            let mut rest = &body[..];
            while let Some(end) = rest.windows(2).position(|w| w == b"\r\n") {
                let size = usize::from_str_radix(
                    String::from_utf8_lossy(&rest[..end])
                        .split(';')
                        .next()
                        .unwrap()
                        .trim(),
                    16,
                )
                .unwrap();
                if size == 0 {
                    break;
                }
                out.extend_from_slice(&rest[end + 2..end + 2 + size]);
                rest = &rest[end + 2 + size + 2..];
            }
            body = out;
        }
        Page {
            status,
            headers,
            body: String::from_utf8_lossy(&body).into_owned(),
        }
    }

    /// Sign in with the form, as the user. A password an administrator
    /// set is changed through the form GitLab sends one to.
    fn sign_in(&mut self, login: &str, password: &str) {
        let page = self.ask("GET", "/users/sign_in", None);
        let token = page.csrf();
        let signed = self.ask(
            "POST",
            "/users/sign_in",
            Some(&form_of(&[
                ("authenticity_token", &token),
                ("user[login]", login),
                ("user[password]", password),
                ("user[remember_me]", "0"),
            ])),
        );
        assert_eq!(signed.status, 302, "signing in: {}", signed.status);
        let home = self.ask("GET", "/", None);
        assert!(
            !home
                .header("location")
                .is_some_and(|to| to.contains("/users/sign_in")),
            "not signed in"
        );
    }

    /// Make an application of the user's that may sign in by code: its id.
    fn application(&mut self, name: &str, scopes: &str) -> String {
        let page = self.ask("GET", "/-/user_settings/applications", None);
        let token = page.csrf();
        let mut pairs = vec![
            ("authenticity_token", token.as_str()),
            ("authn_oauth_application[name]", name),
            (
                "authn_oauth_application[redirect_uri]",
                "http://127.0.0.1/unused",
            ),
            ("authn_oauth_application[confidential]", "0"),
            ("authn_oauth_application[device_code_enabled]", "1"),
        ];
        for scope in scopes.split(' ') {
            pairs.push(("authn_oauth_application[scopes][]", scope));
        }
        let made = self.ask("POST", "/oauth/applications", Some(&form_of(&pairs)));
        // Shown at once (its secret is shown only then), or after a
        // redirect.
        let page = if made.status == 302 {
            let shown = made.header("location").unwrap().to_owned();
            let path = Url::parse(&shown)
                .map(|url| url.path().to_owned())
                .unwrap_or(shown);
            self.ask("GET", &path, None)
        } else {
            made
        };
        assert!(
            page.body.contains("Application ID"),
            "the application was not made"
        );
        // The application's id is shown after its label: 64 hexadecimal
        // digits.
        let at = page.body.find("Application ID").unwrap();
        let rest = &page.body[at..];
        let start = rest
            .char_indices()
            .find(|(i, _)| {
                rest[*i..].len() >= 64 && rest[*i..*i + 64].bytes().all(|b| b.is_ascii_hexdigit())
            })
            .unwrap()
            .0;
        rest[start..start + 64].to_owned()
    }

    /// Type the code at the site and confirm it.
    fn approve(&mut self, user_code: &str) {
        let page = self.ask("GET", "/oauth/device", None);
        let token = page.csrf();
        let confirm = self.ask(
            "POST",
            "/oauth/device/confirm",
            Some(&form_of(&[
                ("authenticity_token", &token),
                ("user_code", user_code),
            ])),
        );
        assert_eq!(confirm.status, 200, "confirming: {}", confirm.status);
        let token = confirm.csrf();
        let approved = self.ask(
            "POST",
            "/oauth/device",
            Some(&form_of(&[
                ("authenticity_token", &token),
                ("user_code", user_code),
            ])),
        );
        assert_eq!(approved.status, 302, "approving: {}", approved.status);
    }
}

/// What a test of the reads starts from: the server set up, a recording
/// named `name`, and alice's account connected in it.
fn reads(name: &str) -> (Setup, Reads, Live, String) {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    let made = setup(&admin);
    let reads = setup_reads(&admin, &made);
    let live = Live::new(
        name,
        "Recorded from GitLab CE 19.4 on this machine by tests/suite/live_gitlab_ce.rs",
    );
    let account = live.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": base, "token": made.alice.token.expose() }),
    );
    let id = account["id"].as_str().unwrap().to_owned();
    (made, reads, live, id)
}

fn numbers(listing: &Value) -> Vec<u64> {
    listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["number"].as_u64().unwrap())
        .collect()
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn merge_requests_are_listed_by_every_filter_and_query() {
    let (made, reads, live, account) = reads("pr_list");
    let list = |params: Value| {
        let mut all = json!({ "account": account, "owner": GROUP, "name": PROJECT });
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        live.run("pr.list", all)
    };
    let all = list(json!({ "state": "all" }));
    let found = numbers(&all);
    for number in [
        reads.topic,
        reads.draft,
        reads.stack[0],
        reads.stack[1],
        reads.merged,
        reads.closed,
        reads.fork,
    ] {
        assert!(found.contains(&number), "!{number} in {all}");
    }
    let row = |number: u64| {
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["number"] == number)
            .unwrap()
            .clone()
    };
    let topic = row(reads.topic);
    assert_eq!(topic["labels"][0]["name"], "bug");
    assert_eq!(topic["assignees"][0]["login"], made.alice.username);
    assert_eq!(topic["reviewers"][0]["user"]["login"], made.bob.username);
    assert_eq!(topic["reviewers"][0]["state"], "approved");
    assert_eq!(topic["ci"], "failure");
    assert!(topic["comments"].as_u64().unwrap() >= 5, "{topic}");
    assert_eq!(row(reads.draft)["draft"], true);
    assert_eq!(row(reads.draft)["ci"], "pending");
    assert_eq!(row(reads.stack[0])["ci"], "running");
    assert_eq!(row(reads.merged)["state"], "merged");
    assert_eq!(row(reads.closed)["state"], "closed");
    assert_eq!(
        row(reads.fork)["head_repo"],
        format!("{}/{PROJECT}", made.bob.username)
    );
    // Open ones by default; whose; and queries.
    let open = numbers(&list(json!({})));
    assert!(!open.contains(&reads.merged) && open.contains(&reads.topic));
    let mine = numbers(&list(json!({ "filter": "mine" })));
    assert!(mine.contains(&reads.topic) && !mine.contains(&reads.fork));
    assert_eq!(
        numbers(&list(json!({ "filter": "assigned" }))),
        [reads.topic]
    );
    assert_eq!(
        numbers(&list(json!({ "filter": "review_requested" }))),
        [reads.draft]
    );
    assert_eq!(
        numbers(&list(json!({ "query": "label:bug" }))),
        [reads.topic]
    );
    let either = numbers(&list(json!({ "query": "label:bug,ui" })));
    assert_eq!(either.len(), 2, "{either:?}");
    assert_eq!(
        numbers(&list(json!({ "query": "draft:true" }))),
        [reads.draft]
    );
    assert_eq!(
        numbers(&list(
            json!({ "query": format!("author:{}", made.bob.username) })
        )),
        [reads.fork]
    );
    assert!(!numbers(&list(json!({ "query": "-label:bug" }))).contains(&reads.topic));
    assert_eq!(
        numbers(&list(json!({ "query": "base:bb-p-stack-1" }))),
        [reads.stack[1]]
    );
    let err = live
        .try_run(
            "pr.list",
            json!({ "account": account, "owner": GROUP, "name": PROJECT, "query": "involves:x" }),
        )
        .unwrap_err();
    assert!(err.contains("involves"), "{err}");
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn a_merge_request_is_shown_with_everything_its_view_shows() {
    let (made, reads, live, account) = reads("pr_show");
    let show = |number: u64| {
        live.run(
            "pr.show",
            json!({ "account": account, "owner": GROUP, "name": PROJECT, "number": number }),
        )
    };
    let topic = show(reads.topic);
    assert_eq!(topic["pull"]["number"], reads.topic);
    assert_eq!(topic["body"], "Made by the live test.");
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["pull"]["reviewers"][0]["state"], "approved");
    assert_eq!(topic["reviews"][0]["state"], "approved");
    assert_eq!(topic["reviews"][0]["author"]["login"], made.bob.username);
    assert_eq!(topic["pull"]["ci"], "failure");
    let checks: Vec<&str> = topic["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| check["name"].as_str().unwrap())
        .collect();
    assert!(
        checks.contains(&"bb-p-lint") && checks.contains(&"bb-p-test"),
        "{checks:?}"
    );
    assert!(!topic["commits"].as_array().unwrap().is_empty());
    let file = &topic["files"][0];
    assert_eq!(file["path"], "bb-p.txt");
    assert_eq!(file["status"], "added");
    assert_eq!(file["additions"], 1);
    assert!(
        file["patch"]
            .as_str()
            .unwrap()
            .contains("+a line to review")
    );
    let threads = topic["threads"].as_array().unwrap();
    let on_lines: Vec<&Value> = threads
        .iter()
        .filter(|thread| thread["path"] == "bb-p.txt")
        .collect();
    assert_eq!(on_lines.len(), 2, "{threads:?}");
    assert_eq!(on_lines[0]["line"], 1);
    assert_eq!(on_lines[0]["resolved"], true);
    assert_eq!(on_lines[1]["resolved"], false);
    assert_eq!(on_lines[0]["comments"].as_array().unwrap().len(), 2);
    assert!(
        threads.iter().any(|thread| thread["path"].is_null()
            && thread["comments"][0]["body"] == "Looks good to me.")
    );
    assert!(!topic["merge_methods"].as_array().unwrap().is_empty());
    assert_ne!(topic["mergeable"], "conflicts");
    let fork = show(reads.fork);
    assert_eq!(
        fork["pull"]["head_repo"],
        format!("{}/{PROJECT}", made.bob.username)
    );
    let draft = show(reads.draft);
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["mergeable"], "blocked");
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn choices_stacks_and_the_merge_request_of_a_branch() {
    let (made, reads, live, account) = reads("pr_options_stack");
    let on = |extra: Value| {
        let mut all = json!({ "account": account, "owner": GROUP, "name": PROJECT });
        for (key, value) in extra.as_object().unwrap() {
            all[key] = value.clone();
        }
        all
    };
    let options = live.run("pr.options", on(json!({})));
    let labels: Vec<&str> = options["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label["name"].as_str().unwrap())
        .collect();
    assert!(
        labels.contains(&"bug") && labels.contains(&"ui"),
        "{labels:?}"
    );
    assert_eq!(options["milestones"][0]["title"], "v1");
    let people: Vec<&str> = options["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["login"].as_str().unwrap())
        .collect();
    assert!(
        people.contains(&made.alice.username.as_str())
            && people.contains(&made.bob.username.as_str()),
        "{people:?}"
    );
    assert!(!options["merge_methods"].as_array().unwrap().is_empty());
    let stack = live.run("pr.stack", on(json!({ "number": reads.stack[1] })));
    let stacked: Vec<u64> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["number"].as_u64().unwrap())
        .collect();
    assert_eq!(stacked, reads.stack);
    assert_eq!(stack["position"], 1);
    let first = live.run("pr.stack", on(json!({ "number": reads.stack[0] })));
    assert_eq!(first["position"], 0);
    let branch = live.run("pr.for_branch", on(json!({ "branch": "bb-p-stack-1" })));
    assert_eq!(branch["number"], reads.stack[0]);
    let none = live.run(
        "pr.for_branch",
        on(json!({ "branch": "bb-p-no-such-branch" })),
    );
    assert_eq!(none, Value::Null);
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn issues_are_listed_by_every_filter_and_shown() {
    let (made, reads, live, account) = reads("issues");
    let list = |params: Value| {
        let mut all = json!({ "account": account, "owner": GROUP, "name": PROJECT });
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        numbers(&live.run("issue.list", all))
    };
    assert!(list(json!({})).contains(&reads.issue));
    assert!(!list(json!({})).contains(&reads.closed_issue));
    assert!(list(json!({ "state": "closed" })).contains(&reads.closed_issue));
    assert!(list(json!({ "state": "all" })).contains(&reads.closed_issue));
    assert_eq!(list(json!({ "filter": "assigned" })), Vec::<u64>::new());
    assert_eq!(list(json!({ "filter": "created" })), [reads.issue]);
    assert_eq!(list(json!({ "filter": "mentioned" })), [reads.issue]);
    assert_eq!(list(json!({ "query": "#bug" })), [reads.issue]);
    assert_eq!(
        list(json!({ "query": format!("@{}", made.bob.username) })),
        [reads.issue]
    );
    assert_eq!(list(json!({ "query": "$v1 button" })), [reads.issue]);
    assert_eq!(
        list(json!({ "query": "-label:bug", "state": "all" })),
        [reads.closed_issue]
    );
    let shown = live.run(
        "issue.show",
        json!({ "account": account, "owner": GROUP, "name": PROJECT, "number": reads.issue }),
    );
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], made.bob.username);
    let comments = shown["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2, "{comments:?}");
    assert_eq!(comments[1]["body"], "The one at the top.");
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn the_checks_of_commits_are_read() {
    let (made, reads, live, account) = reads("ci_status");
    let unknown = "0123456789abcdef0123456789abcdef01234567".to_owned();
    let mut shas = reads.shas.to_vec();
    shas.push(unknown.clone());
    let statuses = live.run(
        "ci.status",
        json!({ "account": account, "owner": GROUP, "name": PROJECT, "shas": shas }),
    );
    let states: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["failure", "pending", "running", "none"]);
    assert_eq!(statuses[0]["sha"], reads.shas[0]);
    assert_eq!(statuses[0]["checks"].as_array().unwrap().len(), 2);
    assert!(
        statuses[0]["checks"][0]["url"]
            .as_str()
            .unwrap()
            .ends_with("/-/pipelines")
    );
    assert_eq!(statuses[3]["sha"], unknown);
    let _ = reads.project_id;
    live.finish(&[&made.alice, &made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn projects_namespaces_and_pictures_are_read() {
    let (made, _, live, account) = reads("repos");
    let repos = |filter: &str, query: Option<&str>| {
        let listing = live.run(
            "provider.repos",
            json!({ "account": account, "filter": filter, "query": query }),
        );
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let own = format!("{}/bb-p-own", made.alice.username);
    let group = format!("{GROUP}/{PROJECT}");
    assert_eq!(repos("own", None), std::slice::from_ref(&own));
    assert!(repos("organisation", None).contains(&group));
    assert!(!repos("organisation", None).contains(&own));
    assert!(repos("starred", None).contains(&group));
    let all = repos("all", None);
    assert!(all.contains(&own) && all.contains(&group), "{all:?}");
    assert_eq!(repos("all", Some("bb-p-own")), [own]);
    let namespaces = live.run("provider.namespaces", json!({ "account": account }));
    let kinds: Vec<(String, String)> = namespaces
        .as_array()
        .unwrap()
        .iter()
        .map(|namespace| {
            (
                namespace["path"].as_str().unwrap().to_owned(),
                namespace["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(kinds.contains(&(made.alice.username.clone(), "user".to_owned())));
    assert!(kinds.contains(&(GROUP.to_owned(), "group".to_owned())));
    let pictures = live.run(
        "provider.avatars",
        json!({ "account": account, "emails": [made.alice.email, made.bob.email] }),
    );
    assert!(pictures[&made.alice.email].is_string(), "{pictures}");
    live.finish(&[&made.alice, &made.bob]);
}

// The writes: merge requests made, changed, reviewed, commented on and
// merged, in projects of their own so that the reads above keep what
// they are tested on.

const WRITES: &str = "bb-p3-writes";
const FAST_FORWARD: &str = "bb-p3-ff";

/// A project of the group, made when missing: its id.
fn group_project(admin: &Api, name: &str) -> u64 {
    let path = encode(&format!("{GROUP}/{name}"));
    let (status, project) = admin.call(Method::Get, &format!("/projects/{path}"), None);
    if status == 200 {
        return project["id"].as_u64().unwrap();
    }
    let group = admin.ok(Method::Get, &format!("/groups/{GROUP}"), None)["id"]
        .as_u64()
        .unwrap();
    admin.ok(
        Method::Post,
        "/projects",
        Some(json!({ "name": name, "path": name, "namespace_id": group,
                     "initialize_with_readme": true, "default_branch": "main",
                     "visibility": "private" })),
    )["id"]
        .as_u64()
        .unwrap()
}

/// A branch made anew from `from` with one new file: the merge requests
/// open from it before are closed and it is removed first.
fn fresh_branch(admin: &Api, project: u64, branch: &str, from: &str, run: &str) {
    let (_, open) = admin.call(
        Method::Get,
        &format!("/projects/{project}/merge_requests?state=opened&source_branch={branch}"),
        None,
    );
    for merge in open.as_array().cloned().unwrap_or_default() {
        let _ = admin.call(
            Method::Put,
            &format!("/projects/{project}/merge_requests/{}", merge["iid"]),
            Some(json!({ "state_event": "close" })),
        );
    }
    let _ = admin.call(
        Method::Delete,
        &format!("/projects/{project}/repository/branches/{}", encode(branch)),
        None,
    );
    admin.ok(
        Method::Post,
        &format!("/projects/{project}/repository/commits"),
        Some(json!({ "branch": branch, "start_branch": from,
                     "commit_message": format!("Change {branch}"),
                     "actions": [ { "action": "create",
                                    "file_path": format!("bb-p3/{branch}-{run}.txt"),
                                    "content": format!("{branch}\n") } ] })),
    );
}

/// What the writes are tested on: two projects with their branches made
/// anew, the milestone's id, and alice's and bob's accounts connected.
struct Writes {
    made: Setup,
    live: Live,
    alice: String,
    bob: String,
    milestone: u64,
}

fn writes(name: &str) -> Writes {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    let made = setup(&admin);
    let run = fresh("run");
    let project = group_project(&admin, WRITES);
    for (label, color) in [("bug", "#d9534f"), ("ui", "#428bca")] {
        let _ = admin.call(
            Method::Post,
            &format!("/projects/{project}/labels"),
            Some(json!({ "name": label, "color": color })),
        );
    }
    let (_, milestones) = admin.call(
        Method::Get,
        &format!("/projects/{project}/milestones?title=v1"),
        None,
    );
    let milestone = match milestones.as_array().and_then(|all| all.first()) {
        Some(found) => found["id"].as_u64().unwrap(),
        None => admin.ok(
            Method::Post,
            &format!("/projects/{project}/milestones"),
            Some(json!({ "title": "v1" })),
        )["id"]
            .as_u64()
            .unwrap(),
    };
    for branch in ["bb-p3-one", "bb-p3-alt", "bb-p3-stack-1"] {
        fresh_branch(&admin, project, branch, "main", &run);
    }
    fresh_branch(&admin, project, "bb-p3-stack-2", "bb-p3-stack-1", &run);
    // A project that merges only what is on top of its target, and a
    // branch that is behind it.
    let ff = group_project(&admin, FAST_FORWARD);
    admin.ok(
        Method::Put,
        &format!("/projects/{ff}"),
        Some(json!({ "merge_method": "ff" })),
    );
    fresh_branch(&admin, ff, "bb-p3-behind", "main", &run);
    admin.ok(
        Method::Post,
        &format!("/projects/{ff}/repository/commits"),
        Some(json!({ "branch": "main", "commit_message": "Move main on",
                     "actions": [ { "action": "create",
                                    "file_path": format!("bb-p3/main-{run}.txt"),
                                    "content": "main\n" } ] })),
    );
    let live = Live::new(
        name,
        "Recorded from GitLab CE 19.4 on this machine by tests/suite/live_gitlab_ce.rs",
    );
    let alice = live.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": base, "token": made.alice.token.expose() }),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let bob = live.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": base, "token": made.bob.token.expose() }),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    Writes {
        made,
        live,
        alice,
        bob,
        milestone,
    }
}

impl Writes {
    /// A command about a project of the writes, as `account`.
    fn on(&self, account: &str, project: &str, id: &str, extra: Value) -> Result<Value, String> {
        let mut params = json!({ "account": account, "owner": GROUP, "name": project });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.live.try_run(id, params)
    }

    fn ok(&self, account: &str, project: &str, id: &str, extra: Value) -> Value {
        self.on(account, project, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn merge_requests_are_made_changed_reviewed_commented_on_and_merged() {
    let w = writes("pr_writes");
    let (alice, bob) = (w.alice.clone(), w.bob.clone());
    let made = w.ok(
        &alice,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true,
                "reviewers": [w.made.bob.username], "assignees": [w.made.alice.username],
                "labels": ["bug"], "milestone": w.milestone.to_string() }),
    );
    let number = made["number"].as_u64().unwrap();
    assert_eq!(made["title"], "Draft: Change one");
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["reviewers"][0]["user"]["login"], w.made.bob.username);
    assert_eq!(made["assignees"][0]["login"], w.made.alice.username);
    // A login that is not there stops it before anything is made.
    let err = w
        .on(
            &alice,
            WRITES,
            "pr.create",
            json!({ "head": "bb-p3-alt", "base": "main", "title": "Nobody",
                    "reviewers": ["bb-p3-nobody-at-all"] }),
        )
        .unwrap_err();
    assert!(err.contains("no user"), "{err}");
    // Reviews by bob, who is asked to review.
    let approved = w.ok(
        &bob,
        WRITES,
        "pr.review",
        json!({ "number": number, "event": "approve" }),
    );
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], w.made.bob.username);
    let changes = w.ok(
        &bob,
        WRITES,
        "pr.review",
        json!({ "number": number, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(changes["state"], "changes_requested");
    let shown = w.ok(&alice, WRITES, "pr.show", json!({ "number": number }));
    assert_eq!(
        shown["pull"]["reviewers"][0]["state"], "changes_requested",
        "{shown}"
    );
    let commented = w.ok(
        &bob,
        WRITES,
        "pr.review",
        json!({ "number": number, "event": "comment", "body": "Better now." }),
    );
    assert_eq!(commented["state"], "commented");
    // Alice is not asked to review her own.
    let err = w
        .on(
            &alice,
            WRITES,
            "pr.review",
            json!({ "number": number, "event": "comment", "body": "Me too." }),
        )
        .unwrap_err();
    assert!(err.contains("asked to review"), "{err}");
    // Comments, and an answer in the thread of one.
    let comment = w.ok(
        &alice,
        WRITES,
        "pr.comment",
        json!({ "number": number, "body": "A comment from the live test." }),
    );
    assert_eq!(comment["body"], "A comment from the live test.");
    let shown = w.ok(&alice, WRITES, "pr.show", json!({ "number": number }));
    let thread = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["comments"][0]["body"] == "A comment from the live test.")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let answer = w.ok(
        &alice,
        WRITES,
        "pr.comment",
        json!({ "number": number, "body": "An answer.", "reply_to": thread }),
    );
    assert_eq!(answer["body"], "An answer.");
    let shown = w.ok(&alice, WRITES, "pr.show", json!({ "number": number }));
    let answered = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|found| found["id"] == thread.as_str())
        .unwrap()
        .clone();
    assert_eq!(
        answered["comments"].as_array().unwrap().len(),
        2,
        "{answered}"
    );
    // Changed: a new title, ready, other labels, nobody asked, no
    // milestone, another target and back.
    let changed = w.ok(
        &alice,
        WRITES,
        "pr.update",
        json!({ "number": number, "title": "Change one, better", "draft": false,
                "labels": ["ui"], "reviewers": [], "assignees": [], "milestone": "",
                "base": "bb-p3-alt" }),
    );
    assert_eq!(changed["pull"]["title"], "Change one, better");
    assert_eq!(changed["pull"]["draft"], false);
    assert_eq!(changed["pull"]["labels"][0]["name"], "ui");
    assert_eq!(changed["pull"]["base_ref"], "bb-p3-alt");
    assert_eq!(changed["pull"]["assignees"], json!([]));
    assert_eq!(changed["milestone"], Value::Null);
    let drafted = w.ok(
        &alice,
        WRITES,
        "pr.update",
        json!({ "number": number, "draft": true, "base": "main" }),
    );
    assert_eq!(drafted["pull"]["title"], "Draft: Change one, better");
    assert_eq!(drafted["pull"]["base_ref"], "main");
    let ready = w.ok(
        &alice,
        WRITES,
        "pr.update",
        json!({ "number": number, "draft": false }),
    );
    assert_eq!(ready["pull"]["draft"], false);
    // Merged, squashed, its branch removed.
    let merged = w.ok(
        &alice,
        WRITES,
        "pr.merge",
        json!({ "number": number, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert!(merged["sha"].is_string());
    assert_eq!(merged["branch_deleted"], true);
    // Merged again: GitLab says no.
    let err = w
        .on(
            &alice,
            WRITES,
            "pr.merge",
            json!({ "number": number, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("merged already"), "{err}");
    w.live.finish(&[&w.made.alice, &w.made.bob]);
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn a_stack_moves_down_when_its_bottom_is_merged_and_a_rebase_comes_first() {
    let w = writes("pr_merge_stack");
    let alice = w.alice.clone();
    let first = w.ok(
        &alice,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-stack-1", "base": "main", "title": "First of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = w.ok(
        &alice,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-stack-2", "base": "bb-p3-stack-1", "title": "Second of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let stack = w.ok(&alice, WRITES, "pr.stack", json!({ "number": second }));
    assert_eq!(stack["position"], 1, "{stack}");
    let merged = w.ok(
        &alice,
        WRITES,
        "pr.merge",
        json!({ "number": first, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "message": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["retargeted"], json!([second]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = w.ok(
        &alice,
        WRITES,
        "pr.for_branch",
        json!({ "branch": "bb-p3-stack-2" }),
    );
    assert_eq!(moved["base_ref"], "main");
    // The fast-forward project: a merge commit is not its way; behind
    // its target, the branch is rebased first.
    let behind = w.ok(
        &alice,
        FAST_FORWARD,
        "pr.create",
        json!({ "head": "bb-p3-behind", "base": "main", "title": "Behind main" }),
    )["number"]
        .as_u64()
        .unwrap();
    let err = w
        .on(
            &alice,
            FAST_FORWARD,
            "pr.merge",
            json!({ "number": behind, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("allows rebase"), "{err}");
    let rebased = w.ok(
        &alice,
        FAST_FORWARD,
        "pr.merge",
        json!({ "number": behind, "method": "rebase", "delete_branch": true }),
    );
    assert_eq!(rebased["merged"], true, "{rebased}");
    w.live.finish(&[&w.made.alice, &w.made.bob]);
}

/// A user of this run alone, `bb-p3-<what>-<hex>`, with a token: removed
/// by [`Gone`] when the test ends, with what they made.
fn passing_person(admin: &Api, what: &str) -> Person {
    let hex = fresh(what);
    let username = format!("bb-p3-{what}-{}", hex.get(3..11).unwrap());
    let mut name: Vec<char> = what.chars().collect();
    if let Some(first) = name.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    person(
        admin,
        &username,
        &format!("{} Tester", name.iter().collect::<String>()),
    )
}

/// Removes a user of one run when dropped.
struct Gone<'a> {
    admin: &'a Api,
    id: u64,
}

impl Drop for Gone<'_> {
    fn drop(&mut self) {
        let _ = self.admin.call(
            Method::Delete,
            &format!("/users/{}?hard_delete=true", self.id),
            None,
        );
    }
}

/// A repository on this machine whose `origin` is `address`, for the
/// commands that need one open.
fn scratch_repository(name: &str, address: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("bb-p3-live-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = bitbraid_testkit::Git::new(&dir, dir.join(".home"));
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    git.run(["add", "a.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "A file"]).unwrap();
    git.run(["remote", "add", "origin", address]).unwrap();
    dir
}

impl Live {
    /// A command with the repository at `repo` open.
    fn run_in(&self, repo: &std::path::Path, id: &str, params: Value) -> Result<Value, String> {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        registry
            .run(
                "repo.open",
                &mut ctx,
                &json!({ "path": repo.to_str().unwrap() }),
            )
            .map_err(|err| err.to_string())?;
        registry
            .run(id, &mut ctx, &params)
            .map_err(|err| err.to_string())
    }
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn projects_are_made_and_forked_and_keys_uploaded() {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    let made = setup(&admin);
    group_project(&admin, WRITES);
    let carol = passing_person(&admin, "carol");
    let _gone = Gone {
        admin: &admin,
        id: carol.id,
    };
    // Carol may see the group's projects, and fork them.
    let group = admin.ok(Method::Get, &format!("/groups/{GROUP}"), None)["id"]
        .as_u64()
        .unwrap();
    admin.ok(
        Method::Post,
        &format!("/groups/{group}/members"),
        Some(json!({ "user_id": carol.id, "access_level": 30 })),
    );
    let live = Live::new(
        "repos_forks_keys",
        "Recorded from GitLab CE 19.4 on this machine by tests/suite/live_gitlab_ce.rs",
    );
    let account = live.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": base, "token": carol.token.expose() }),
    )["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let repo = scratch_repository("carol", &format!("{base}/{GROUP}/{WRITES}.git"));
    // A project of her own, added as a remote.
    let created = live
        .run_in(
            &repo,
            "provider.repo_create",
            json!({ "account": account, "name": "bb-p3-made", "private": true,
                    "description": "Made by the live test", "remote": "made" }),
        )
        .unwrap();
    assert_eq!(
        created["repo"]["full_name"],
        format!("{}/bb-p3-made", carol.username)
    );
    assert_eq!(created["repo"]["private"], true);
    assert_eq!(created["remote"], "made");
    // The group's project forked into her namespace, and the fork added
    // as a remote named after her.
    let forked = live
        .run_in(&repo, "provider.fork", json!({ "remote": "origin" }))
        .unwrap();
    assert_eq!(
        forked["repo"]["full_name"],
        format!("{}/{WRITES}", carol.username)
    );
    assert_eq!(forked["repo"]["parent"], format!("{GROUP}/{WRITES}"));
    assert_eq!(forked["remote"], carol.username);
    let remotes = bitbraid_testkit::Git::new(&repo, repo.join(".home"))
        .run(["remote", "-v"])
        .unwrap();
    assert!(
        remotes.contains(&format!("{base}/{}/{WRITES}.git", carol.username)),
        "{remotes}"
    );
    assert!(
        remotes.contains(&format!("{base}/{}/bb-p3-made.git", carol.username)),
        "{remotes}"
    );
    // A key: uploaded, its fingerprint as ssh-keygen says it, then taken
    // away again in the same run.
    let key = repo.join("bb-p3.pub");
    std::fs::write(
        &key,
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n",
    )
    .unwrap();
    let added = live.run(
        "provider.ssh_key_add",
        json!({ "account": account, "title": "bb-p3 test key", "path": key.to_str().unwrap() }),
    );
    assert_eq!(added["title"], "bb-p3 test key");
    assert_eq!(
        added["fingerprint"],
        "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0"
    );
    let id = added["id"].as_str().unwrap().to_owned();
    admin
        .as_user(&carol.token)
        .ok(Method::Delete, &format!("/user/keys/{id}"), None);
    // A private key is not sent.
    std::fs::write(
        repo.join("id_ed25519.pub"),
        "-----BEGIN OPENSSH PRIVATE KEY-----\nx\n",
    )
    .unwrap();
    let err = live
        .try_run(
            "provider.ssh_key_add",
            json!({ "account": account, "title": "x",
                    "path": repo.join("id_ed25519.pub").to_str().unwrap() }),
        )
        .unwrap_err();
    assert!(err.contains("private key"), "{err}");
    let _ = std::fs::remove_dir_all(&repo);
    live.finish(&[&made.alice, &made.bob, &carol]);
}
