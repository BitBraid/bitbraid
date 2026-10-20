//! The provider commands through the registry, against recorded answers:
//! connecting accounts by token and by code, the file and the keychain
//! they are kept in, the links of a repository's remotes, the pull
//! requests of both providers, and what the providers answer when
//! something is wrong.
//!
//! The answers here are made by hand from the examples of the providers'
//! documentation (docs.github.com/en/rest, docs.gitlab.com/api, RFC 8628);
//! those recorded from a live server are replayed in
//! `tests/suite/recorded.rs`. The keychain is one in memory. Nothing leaves
//! the machine.

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
    replay::Replay,
    secrets::{MemoryStore, Secret, SecretStore},
    services::ClientIds,
};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A held clock: 2026-10-12T10:30:00Z.
fn now() -> i64 {
    1_791_801_000
}

struct Harness {
    dir: PathBuf,
    secrets: Arc<MemoryStore>,
    replay: Arc<Replay>,
    services: Arc<Services>,
    repo: Option<Scratch>,
}

impl Harness {
    fn new(exchanges: Value) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-p-providers-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = Arc::new(
            Replay::from_json(&json!({ "note": "by hand", "exchanges": exchanges }).to_string())
                .unwrap(),
        );
        let secrets = Arc::new(MemoryStore::new());
        let mut services = Services::new(replay.clone(), secrets.clone(), &dir);
        services.client_ids = ClientIds {
            github: "Iv1.0123456789abcdef".to_owned(),
            gitlab: String::new(),
        };
        services.now = now;
        Self {
            dir,
            secrets,
            replay,
            services: Arc::new(services),
            repo: None,
        }
    }

    fn with_repository(mut self, remotes: &[(&str, &str)]) -> Self {
        let repo = Scratch::of("linear").unwrap();
        for (name, url) in remotes {
            repo.git().run(["remote", "add", name, url]).unwrap();
        }
        self.repo = Some(repo);
        self
    }

    fn try_run(&self, id: &str, params: Value) -> Result<Value, CommandError> {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        if let Some(repo) = &self.repo {
            registry
                .run(
                    "repo.open",
                    &mut ctx,
                    &json!({ "path": repo.path().to_str().unwrap() }),
                )
                .unwrap();
        }
        registry.run(id, &mut ctx, &params)
    }

    fn run(&self, id: &str, params: Value) -> Value {
        self.try_run(id, params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn file(&self) -> String {
        std::fs::read_to_string(self.dir.join("integrations.toml")).unwrap_or_default()
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn github_user(login: &str, id: u64) -> Value {
    json!({
        "login": login, "id": id, "node_id": "MDQ6VXNlcjE=",
        "avatar_url": format!("https://avatars.example.invalid/{login}"),
        "html_url": format!("https://github.com/{login}"),
        "type": "User", "site_admin": false,
        "name": "Mona Octocat", "email": null
    })
}

const TOKEN: &str = "ghp_16C7e42F292c6912E7710c838347Ae178B4a";

#[test]
fn a_github_token_is_asked_whose_it_is_and_kept_in_the_keychain_only() {
    let h = Harness::new(json!([
        { "method": "GET", "path_and_query": "/user", "status": 200,
          "headers": { "x-oauth-scopes": "repo, read:org, workflow", "x-ratelimit-remaining": "4999" },
          "body": github_user("octocat", 1) }
    ]));
    let account = h.run(
        "account.add_token",
        json!({ "kind": "github", "token": TOKEN }),
    );
    assert_eq!(account["id"], "github:github.com:octocat");
    assert_eq!(account["api"], "https://api.github.com");
    assert_eq!(account["web"], "https://github.com");
    assert_eq!(account["name"], "Mona Octocat");
    assert_eq!(account["sign_in"], "token");
    assert_eq!(account["scopes"], json!(["repo", "read:org", "workflow"]));
    assert_eq!(account["default"], true);
    assert!(!account.to_string().contains(TOKEN));
    // The keychain has it, labelled; the file has the account without it.
    let kept = h.secrets.get("github:github.com:octocat").unwrap().unwrap();
    assert!(kept.expose().contains(TOKEN));
    assert_eq!(
        h.secrets.label("github:github.com:octocat").as_deref(),
        Some("BitBraid: octocat at github.com")
    );
    let file = h.file();
    assert!(file.contains("login = \"octocat\""), "{file}");
    assert!(!file.contains(TOKEN), "{file}");
    // Listed, with nothing of the token.
    let listed = h.run("account.list", json!({}));
    assert_eq!(listed.as_array().unwrap().len(), 1);
    assert!(!listed.to_string().contains(TOKEN));
}

#[test]
fn a_refused_token_connects_nothing() {
    let h = Harness::new(json!([
        { "method": "GET", "path_and_query": "/user", "status": 401,
          "body": { "message": "Bad credentials", "documentation_url": "https://docs.github.com/rest" } }
    ]));
    let err = h
        .try_run(
            "account.add_token",
            json!({ "kind": "github", "token": TOKEN }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("refused the token"), "{err}");
    assert!(err.contains("Bad credentials"), "{err}");
    assert!(!err.contains(TOKEN), "{err}");
    assert!(h.secrets.accounts().is_empty());
    assert_eq!(h.run("account.list", json!({})), json!([]));
}

fn gitlab_user(username: &str, id: u64) -> Value {
    json!({
        "id": id, "username": username, "name": "Alice Example", "state": "active",
        "avatar_url": "https://gitlab.example.org/uploads/-/system/user/avatar/2/a.png",
        "web_url": format!("https://gitlab.example.org/{username}"),
        "email": "alice@example.org"
    })
}

#[test]
fn a_gitlab_token_says_its_scopes_and_the_second_account_is_not_the_default() {
    let h = Harness::new(json!([
        { "method": "GET", "path_and_query": "/api/v4/user", "status": 200, "body": gitlab_user("alice", 2) },
        { "method": "GET", "path_and_query": "/api/v4/personal_access_tokens/self", "status": 200,
          "body": { "id": 4, "name": "bitbraid", "revoked": false, "scopes": ["api", "read_user"],
                    "user_id": 2, "active": true, "expires_at": null } },
        { "method": "GET", "path_and_query": "/api/v4/user", "status": 200, "body": gitlab_user("bob", 3) },
        { "method": "GET", "path_and_query": "/api/v4/personal_access_tokens/self", "status": 404,
          "body": { "message": "404 Not Found" } }
    ]));
    let web = "https://gitlab.example.org";
    let alice = h.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": web, "token": "glpat-alice-0000000000" }),
    );
    assert_eq!(alice["id"], "gitlab:gitlab.example.org:alice");
    assert_eq!(alice["api"], "https://gitlab.example.org/api/v4");
    assert_eq!(alice["scopes"], json!(["api", "read_user"]));
    assert_eq!(alice["default"], true);
    let bob = h.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": web, "token": "glpat-bob-00000000000" }),
    );
    assert_eq!(bob["default"], false);
    assert_eq!(bob["scopes"], json!([]));

    h.run(
        "account.set_default",
        json!({ "id": "gitlab:gitlab.example.org:bob" }),
    );
    let listed = h.run("account.list", json!({}));
    let defaults: Vec<(&str, bool)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|account| {
            (
                account["login"].as_str().unwrap(),
                account["default"].as_bool().unwrap(),
            )
        })
        .collect();
    assert_eq!(defaults, [("alice", false), ("bob", true)]);

    // Removed: out of the file and out of the keychain; the other one is
    // the default again.
    h.run(
        "account.remove",
        json!({ "id": "gitlab:gitlab.example.org:bob" }),
    );
    assert_eq!(h.secrets.accounts(), ["gitlab:gitlab.example.org:alice"]);
    let listed = h.run("account.list", json!({}));
    assert_eq!(listed[0]["default"], true);
    assert!(
        h.try_run(
            "account.remove",
            json!({ "id": "gitlab:gitlab.example.org:bob" })
        )
        .is_err()
    );
    assert!(
        h.try_run("account.set_default", json!({ "id": "nobody" }))
            .is_err()
    );
}

#[test]
fn signing_in_by_code_at_github_goes_through_every_state() {
    let device = "3584d83530557fdd1f46af8289938c8ef79f9dc5";
    let poll = format!(
        "client_id=Iv1.0123456789abcdef&device_code={device}&grant_type=urn%3Aietf%3Aparams%3Aoauth%3Agrant-type%3Adevice_code"
    );
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/login/device/code",
          "request_body": "client_id=Iv1.0123456789abcdef&scope=repo%20read%3Aorg%20admin%3Apublic_key%20read%3Auser%20user%3Aemail%20workflow",
          "status": 200,
          "body": { "device_code": device, "user_code": "WDJB-MJHT",
                    "verification_uri": "https://github.com/login/device",
                    "expires_in": 900, "interval": 5 } },
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "request_body": poll, "status": 200,
          "body": { "error": "authorization_pending", "error_description": "The authorization request is still pending." } },
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "request_body": poll, "status": 200,
          "body": { "error": "slow_down", "interval": 10 } },
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "request_body": poll, "status": 200,
          "body": { "access_token": "gho_16C7e42F292c6912E7710c838347Ae178B4a", "token_type": "bearer",
                    "scope": "repo,gist" } },
        { "method": "GET", "path_and_query": "/user", "status": 200,
          "headers": { "x-oauth-scopes": "repo, gist" }, "body": github_user("octocat", 1) }
    ]));
    let code = h.run("account.sign_in_start", json!({ "kind": "github" }));
    assert_eq!(code["user_code"], "WDJB-MJHT");
    assert_eq!(code["verification_uri"], "https://github.com/login/device");
    assert_eq!(code["interval"], 5);
    let ask = || {
        h.run(
            "account.sign_in_poll",
            json!({ "kind": "github", "device_code": device, "interval": 5 }),
        )
    };
    assert_eq!(ask(), json!({ "state": "waiting" }));
    assert_eq!(ask(), json!({ "state": "slow_down", "interval": 10 }));
    let done = ask();
    assert_eq!(done["state"], "done");
    assert_eq!(done["account"]["login"], "octocat");
    assert_eq!(done["account"]["sign_in"], "device");
    assert_eq!(done["account"]["scopes"], json!(["repo", "gist"]));
    assert!(!done.to_string().contains("gho_"));
    assert!(
        h.secrets
            .get("github:github.com:octocat")
            .unwrap()
            .unwrap()
            .expose()
            .contains("gho_16C7")
    );
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn a_code_that_is_refused_or_runs_out_says_so() {
    let device = "3584d83530557fdd1f46af8289938c8ef79f9dc5";
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "status": 200,
          "body": { "error": "access_denied" } },
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "status": 200,
          "body": { "error": "expired_token" } },
        { "method": "POST", "path_and_query": "/login/oauth/access_token", "status": 200,
          "body": { "error": "device_flow_disabled", "error_description": "Device flow is not enabled" } }
    ]));
    let ask = || {
        h.try_run(
            "account.sign_in_poll",
            json!({ "kind": "github", "device_code": device }),
        )
    };
    assert_eq!(ask().unwrap(), json!({ "state": "denied" }));
    assert_eq!(ask().unwrap(), json!({ "state": "expired" }));
    let err = ask().unwrap_err().to_string();
    assert!(err.contains("not enabled"), "{err}");
}

#[test]
fn without_an_application_id_signing_in_by_code_is_not_offered() {
    let h = Harness::new(json!([]));
    for params in [
        json!({ "kind": "gitlab" }),
        json!({ "kind": "gitlab", "web": "https://gitlab.example.org" }),
        json!({ "kind": "github", "web": "https://ghe.example.org" }),
    ] {
        let err = h
            .try_run("account.sign_in_start", params.clone())
            .unwrap_err()
            .to_string();
        assert!(err.contains("not available"), "{params}: {err}");
        assert!(err.contains("token"), "{params}: {err}");
    }
    // An id is given for one's own server only.
    assert!(
        h.try_run(
            "account.sign_in_start",
            json!({ "kind": "gitlab", "client_id": "abc" })
        )
        .is_err()
    );
    assert!(h.replay.asked().is_empty());
}

#[test]
fn signing_in_by_code_at_a_gitlab_of_ones_own_keeps_its_application_id() {
    let app = "a4b8c3e2f1d0a4b8c3e2f1d0a4b8c3e2f1d0a4b8c3e2f1d0a4b8c3e2f1d0a4b8";
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/oauth/authorize_device", "status": 200,
          "body": { "device_code": "GmRhmhcxhwAzkoEqiMEg_DnyEysNkuNhszIySk9eS",
                    "user_code": "0A44L90H",
                    "verification_uri": "http://127.0.0.1:8929/oauth/device",
                    "verification_uri_complete": "http://127.0.0.1:8929/oauth/device?user_code=0A44L90H",
                    "expires_in": 300, "interval": 5 } },
        { "method": "POST", "path_and_query": "/oauth/token", "status": 400,
          "body": { "error": "slow_down", "error_description": "slow down" } },
        { "method": "POST", "path_and_query": "/oauth/token", "status": 200,
          "body": { "access_token": "glat-device-token", "token_type": "Bearer", "expires_in": 7200,
                    "refresh_token": "glrt-refresh", "scope": "api read_user", "created_at": 1_791_801_000 } },
        { "method": "GET", "path_and_query": "/api/v4/user", "status": 200, "body": gitlab_user("alice", 2) },
        { "method": "GET", "path_and_query": "/api/v4/personal_access_tokens/self", "status": 401,
          "body": { "message": "401 Unauthorized" } }
    ]));
    let web = "http://127.0.0.1:8929";
    let code = h.run(
        "account.sign_in_start",
        json!({ "kind": "gitlab", "web": web, "client_id": app }),
    );
    assert_eq!(code["user_code"], "0A44L90H");
    assert!(h.file().contains(app), "{}", h.file());
    // Without the id now: it was kept for the site.
    let slow = h.run(
        "account.sign_in_poll",
        json!({ "kind": "gitlab", "web": web, "device_code": code["device_code"], "interval": 7 }),
    );
    assert_eq!(slow, json!({ "state": "slow_down", "interval": 12 }));
    let done = h.run(
        "account.sign_in_poll",
        json!({ "kind": "gitlab", "web": web, "device_code": code["device_code"] }),
    );
    assert_eq!(done["account"]["id"], "gitlab:127.0.0.1:8929:alice");
    assert_eq!(done["account"]["scopes"], json!(["api", "read_user"]));
    let kept: Value = serde_json::from_str(
        h.secrets
            .get("gitlab:127.0.0.1:8929:alice")
            .unwrap()
            .unwrap()
            .expose(),
    )
    .unwrap();
    assert_eq!(kept["refresh_token"], "glrt-refresh");
    assert_eq!(kept["expires_at"], now() + 7200);
}

#[test]
fn a_verification_address_off_the_site_is_refused() {
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/login/device/code", "status": 200,
          "body": { "device_code": "abc", "user_code": "WDJB-MJHT",
                    "verification_uri": "https://github.com.evil.example/login/device",
                    "expires_in": 900, "interval": 5 } }
    ]));
    let err = h
        .try_run("account.sign_in_start", json!({ "kind": "github" }))
        .unwrap_err()
        .to_string();
    assert!(err.contains("not on the site"), "{err}");
}

/// An account at a GitLab of one's own whose token ran out an hour ago.
fn expired_gitlab(h: &Harness) {
    let toml = r#"[[account]]
id = "gitlab:127.0.0.1:8929:alice"
kind = "gitlab"
web = "http://127.0.0.1:8929"
api = "http://127.0.0.1:8929/api/v4"
login = "alice"
sign_in = "device"
scopes = ["api", "read_user"]
default = true

[[host]]
web = "http://127.0.0.1:8929"
client_id = "app-id"
"#;
    std::fs::create_dir_all(&h.dir).unwrap();
    std::fs::write(h.dir.join("integrations.toml"), toml).unwrap();
    h.secrets
        .set(
            "gitlab:127.0.0.1:8929:alice",
            "test",
            &Secret::new(
                json!({ "access_token": "old", "refresh_token": "glrt-1", "expires_at": now() - 3600 })
                    .to_string(),
            ),
        )
        .unwrap();
}

fn merge_request(iid: u64, state: &str, source_project: u64) -> Value {
    json!({
        "id": 1000 + iid, "iid": iid, "project_id": 7, "title": format!("Change {iid}\u{7}\nmore"),
        "description": "", "state": state, "draft": iid == 2, "work_in_progress": iid == 2,
        "author": { "id": 2, "username": "alice", "name": "Alice Example",
                    "avatar_url": null, "web_url": "http://127.0.0.1:8929/alice" },
        "source_branch": format!("topic-{iid}"), "target_branch": "main",
        "source_project_id": source_project, "target_project_id": 7,
        "sha": "4e5d0a7b1c2d3e4f5a6b7c8d9e0f1a2b3c4d5e6f",
        "created_at": "2026-10-01T10:00:00.000Z", "updated_at": "2026-10-02T10:00:00.000Z",
        "labels": [ { "id": 1, "name": "bug", "color": "#D9534F", "description": "Broken" } ],
        "reviewers": [ { "id": 3, "username": "bob", "name": "Bob" } ],
        "assignees": [],
        "user_notes_count": 3,
        "web_url": format!("http://127.0.0.1:8929/grp/sub/proj/-/merge_requests/{iid}")
    })
}

#[test]
fn gitlab_merge_requests_are_listed_after_the_token_is_renewed() {
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/oauth/token",
          "request_body": "grant_type=refresh_token&refresh_token=glrt-1&client_id=app-id",
          "status": 200,
          "body": { "access_token": "new-token", "refresh_token": "glrt-2", "expires_in": 7200 } },
        { "method": "GET",
          "path_and_query": "/api/v4/projects/grp%2Fsub%2Fproj/merge_requests?state=all&with_labels_details=true&per_page=100",
          "status": 200, "headers": { "ratelimit-remaining": "1999", "ratelimit-reset": "1791801060" },
          "body": [ merge_request(1, "merged", 7), merge_request(2, "opened", 9) ] },
        // What GraphQL adds: the fork's path, the pipeline, the verdicts
        // (docs.gitlab.com/api/graphql, `MergeRequest`).
        { "method": "POST", "path_and_query": "/api/graphql",
          "request_body": { "variables": { "path": "grp/sub/proj", "iids": ["1", "2"] } },
          "status": 200,
          "body": { "data": { "project": { "mergeRequests": { "nodes": [
            { "iid": "1", "sourceProject": { "fullPath": "grp/sub/proj" },
              "headPipeline": { "status": "SUCCESS" },
              "reviewers": { "nodes": [ { "username": "bob",
                  "mergeRequestInteraction": { "reviewState": "REVIEWED", "approved": true } } ] },
              "approvedBy": { "nodes": [ { "username": "bob" }, { "username": "carol" } ] } },
            { "iid": "2", "sourceProject": { "fullPath": "alice/proj" },
              "headPipeline": { "status": "RUNNING" },
              "reviewers": { "nodes": [ { "username": "bob",
                  "mergeRequestInteraction": { "reviewState": "REQUESTED_CHANGES", "approved": false } } ] },
              "approvedBy": { "nodes": [] } }
          ] } } } } }
    ]))
    .with_repository(&[("origin", "ssh://git@127.0.0.1:2224/grp/sub/proj.git")]);
    expired_gitlab(&h);
    let links = h.run("provider.links", json!({}));
    assert_eq!(
        links,
        json!([{ "remote": "origin", "account": "gitlab:127.0.0.1:8929:alice", "kind": "gitlab",
                 "owner": "grp/sub", "name": "proj" }])
    );
    let listing = h.run("pr.list", json!({ "state": "all" }));
    let items = listing["items"].as_array().unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[0]["number"], 1);
    assert_eq!(items[0]["state"], "merged");
    assert_eq!(items[0]["title"], "Change 1  more");
    assert_eq!(items[0]["repo"], "grp/sub/proj");
    assert_eq!(items[0]["head_ref"], "topic-1");
    assert_eq!(items[0]["head_repo"], Value::Null);
    assert_eq!(items[0]["labels"][0]["color"], "d9534f");
    assert_eq!(items[0]["reviewers"][0]["user"]["login"], "bob");
    assert_eq!(items[0]["reviewers"][0]["state"], "approved");
    assert_eq!(items[0]["reviewers"][1]["user"]["login"], "carol");
    assert_eq!(items[0]["reviewers"][1]["state"], "approved");
    assert_eq!(items[0]["ci"], "success");
    assert_eq!(items[0]["comments"], 3);
    assert_eq!(items[1]["draft"], true);
    assert_eq!(items[1]["state"], "open");
    assert_eq!(items[1]["head_repo"], "alice/proj");
    assert_eq!(items[1]["ci"], "running");
    assert_eq!(items[1]["reviewers"][0]["state"], "changes_requested");
    assert_eq!(listing["more"], false);
    assert_eq!(listing["freshness"]["remaining"], 1999);
    assert_eq!(listing["freshness"]["reset"], "2026-10-12T10:31:00Z");
    assert_eq!(listing["freshness"]["fetched"], "2026-10-12T10:30:00Z");
    // The new token was kept.
    let kept = h
        .secrets
        .get("gitlab:127.0.0.1:8929:alice")
        .unwrap()
        .unwrap();
    assert!(kept.expose().contains("new-token") && kept.expose().contains("glrt-2"));
}

/// A pull request as GitHub's GraphQL search answers it, after the
/// `PullRequest` object of docs.github.com/en/graphql.
fn github_graph_pull(number: u64, head_repo: &str, state: &str, rollup: Option<&str>) -> Value {
    json!({
        "number": number, "title": "Amazing new feature", "state": state, "isDraft": number == 2,
        "url": format!("https://github.com/octocat/Hello-World/pull/{number}"),
        "createdAt": format!("2011-01-{:02}T19:01:12Z", 10 + number % 20),
        "updatedAt": "2011-01-26T19:01:12Z",
        "author": { "login": "octocat", "avatarUrl": "https://avatars.example.invalid/octocat",
                    "url": "https://github.com/octocat", "name": "Mona Octocat" },
        "headRefName": "new-topic", "headRefOid": "6dcb09b5b57875f334f61aebed695e2e4193db5e",
        "headRepository": { "nameWithOwner": head_repo },
        "baseRefName": "master", "baseRepository": { "nameWithOwner": "octocat/Hello-World" },
        "labels": { "nodes": [ { "name": "bug", "color": "f29513", "description": "Something isn't working" } ] },
        "assignees": { "nodes": [ { "login": "hubot", "name": null, "avatarUrl": null, "url": null } ] },
        "reviewRequests": { "nodes": [
            { "requestedReviewer": { "login": "other_user" } },
            { "requestedReviewer": { "combinedSlug": "octo-org/justice-league", "name": "Justice League" } }
        ] },
        "latestReviews": { "nodes": [
            { "state": "APPROVED", "author": { "login": "hubot" } },
            { "state": "CHANGES_REQUESTED", "author": { "login": "other_user" } },
            { "state": "PENDING", "author": { "login": "octocat" } }
        ] },
        "comments": { "totalCount": 7 },
        "commits": { "nodes": [ { "commit": { "statusCheckRollup": rollup.map(|state| json!({ "state": state })) } } ] }
    })
}

fn github_account(h: &Harness) {
    std::fs::create_dir_all(&h.dir).unwrap();
    std::fs::write(
        h.dir.join("integrations.toml"),
        "[[account]]\nid = \"github:github.com:octocat\"\nkind = \"github\"\nweb = \"https://github.com\"\n\
         api = \"https://api.github.com\"\nlogin = \"octocat\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n",
    )
    .unwrap();
    h.secrets
        .set("github:github.com:octocat", "test", &Secret::new(TOKEN))
        .unwrap();
}

#[test]
fn github_pull_requests_are_searched_page_by_page() {
    let search = "repo:octocat/Hello-World is:pr sort:created-desc";
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/graphql",
          "request_body": { "variables": { "search": search, "first": 100, "after": null } },
          "status": 200,
          "headers": { "x-ratelimit-remaining": "4998", "x-ratelimit-reset": "1791801600" },
          "body": { "data": { "search": {
              "pageInfo": { "hasNextPage": true, "endCursor": "Y3Vyc29yOjI=" },
              "nodes": [ github_graph_pull(1347, "octocat/Hello-World", "OPEN", Some("SUCCESS")),
                         github_graph_pull(1346, "hubot/Hello-World", "MERGED", Some("FAILURE")),
                         {} ] } } } },
        { "method": "POST", "path_and_query": "/graphql",
          "request_body": { "variables": { "search": search, "after": "Y3Vyc29yOjI=" } },
          "status": 200,
          "body": { "data": { "search": {
              "pageInfo": { "hasNextPage": false, "endCursor": null },
              "nodes": [ github_graph_pull(2, "octocat/Hello-World", "CLOSED", None) ] } } } }
    ]));
    github_account(&h);
    let listing = h.run(
        "pr.list",
        json!({ "account": "github:github.com:octocat", "owner": "octocat", "name": "Hello-World",
                "state": "all" }),
    );
    let items = listing["items"].as_array().unwrap();
    let numbers: Vec<u64> = items
        .iter()
        .map(|item| item["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, [1347, 1346, 2]);
    assert_eq!(items[0]["head_repo"], Value::Null);
    assert_eq!(items[0]["ci"], "success");
    assert_eq!(items[0]["comments"], 7);
    assert_eq!(items[0]["author"]["name"], "Mona Octocat");
    assert_eq!(items[0]["assignees"][0]["login"], "hubot");
    assert_eq!(items[0]["labels"][0]["color"], "f29513");
    // Each reviewer's last verdict, asked again is pending; a review not
    // sent yet is nobody's verdict.
    let reviewers: Vec<(String, String)> = items[0]["reviewers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reviewer| {
            (
                reviewer["user"]["login"].as_str().unwrap().to_owned(),
                reviewer["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        reviewers,
        [
            ("hubot".to_owned(), "approved".to_owned()),
            ("other_user".to_owned(), "pending".to_owned()),
            ("octo-org/justice-league".to_owned(), "pending".to_owned()),
        ]
    );
    assert_eq!(items[1]["head_repo"], "hubot/Hello-World");
    assert_eq!(items[1]["state"], "merged");
    assert_eq!(items[1]["ci"], "failure");
    assert_eq!(items[2]["state"], "closed");
    assert_eq!(items[2]["draft"], true);
    assert_eq!(items[2]["ci"], "none");
    assert_eq!(listing["more"], false);
    assert_eq!(listing["freshness"]["remaining"], 4998);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn github_filters_and_queries_are_made_into_searches() {
    let page = |numbers: &[u64]| {
        json!({ "data": { "search": {
            "pageInfo": { "hasNextPage": false, "endCursor": null },
            "nodes": numbers.iter().map(|number| github_graph_pull(*number, "octocat/Hello-World", "OPEN", None)).collect::<Vec<_>>() } } })
    };
    let asked = |search: &str, body: Value| {
        json!({ "method": "POST", "path_and_query": "/graphql",
                "request_body": { "variables": { "search": search } }, "status": 200, "body": body })
    };
    let base = "repo:octocat/Hello-World is:pr";
    let h = Harness::new(json!([
        asked(
            &format!("{base} is:closed is:unmerged author:octocat sort:created-desc"),
            page(&[1])
        ),
        asked(
            &format!("{base} is:merged assignee:octocat sort:created-desc"),
            page(&[2])
        ),
        asked(
            &format!("{base} is:open review-requested:octocat sort:created-desc"),
            page(&[3])
        ),
        // `a,b` is either: one search each, put together by number.
        asked(
            &format!(
                "{base} is:open label:bug -label:wip -label:\"needs review\" draft:false sort:created-desc"
            ),
            page(&[4, 5])
        ),
        asked(
            &format!(
                "{base} is:open label:ui -label:wip -label:\"needs review\" draft:false sort:created-desc"
            ),
            page(&[5, 6])
        ),
        // A sort of one's own is kept.
        asked(
            &format!("{base} is:open \"two words\" sort:updated-asc"),
            page(&[7])
        )
    ]));
    github_account(&h);
    let list = |params: Value| {
        let mut all = json!({ "account": "github:github.com:octocat", "owner": "octocat",
                              "name": "Hello-World" });
        for (key, value) in params.as_object().unwrap() {
            all[key] = value.clone();
        }
        let listing = h.run("pr.list", all);
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|item| item["number"].as_u64().unwrap())
            .collect::<Vec<_>>()
    };
    assert_eq!(list(json!({ "filter": "mine", "state": "closed" })), [1]);
    assert_eq!(
        list(json!({ "filter": "assigned", "state": "merged" })),
        [2]
    );
    assert_eq!(list(json!({ "filter": "review_requested" })), [3]);
    assert_eq!(
        list(json!({ "query": "label:bug,ui -label:wip,\"needs review\" draft:false" })),
        [6, 5, 4]
    );
    assert_eq!(
        list(json!({ "query": "\"two words\" sort:updated-asc" })),
        [7]
    );
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
    // What would leave the repository is refused before anything is asked.
    for query in ["repo:other/x", "org:github", "is:issue", "a:1,2,3 b:1,2,3"] {
        let err = h
            .try_run(
                "pr.list",
                json!({ "account": "github:github.com:octocat", "owner": "octocat",
                        "name": "Hello-World", "query": query }),
            )
            .unwrap_err()
            .to_string();
        assert!(!err.contains("recorded"), "{query}: {err}");
    }
}

#[test]
fn a_repository_is_linked_through_its_remotes() {
    let h = Harness::new(json!([])).with_repository(&[
        ("upstream", "https://github.com/octocat/Hello-World.git"),
        ("origin", "git@github.com:me/Hello-World.git"),
        ("mirror", "/srv/mirror.git"),
    ]);
    // Nothing is linked without accounts.
    assert_eq!(h.run("provider.links", json!({})), json!([]));
    std::fs::create_dir_all(&h.dir).unwrap();
    std::fs::write(
        h.dir.join("integrations.toml"),
        "[[account]]\nid = \"github:github.com:me\"\nkind = \"github\"\nweb = \"https://github.com\"\n\
         api = \"https://api.github.com\"\nlogin = \"me\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n",
    )
    .unwrap();
    let links = h.run("provider.links", json!({}));
    let remotes: Vec<&str> = links
        .as_array()
        .unwrap()
        .iter()
        .map(|link| link["remote"].as_str().unwrap())
        .collect();
    assert_eq!(remotes, ["origin", "upstream"]);
    assert_eq!(links[0]["owner"], "me");
    // Without a repository there is nothing to link.
    let bare = Harness::new(json!([]));
    assert!(bare.try_run("provider.links", json!({})).is_err());
}

#[test]
fn what_cannot_be_done_is_refused_before_anything_is_asked() {
    let h = Harness::new(json!([]));
    std::fs::create_dir_all(&h.dir).unwrap();
    std::fs::write(
        h.dir.join("integrations.toml"),
        "[[account]]\nid = \"github:github.com:me\"\nkind = \"github\"\nweb = \"https://github.com\"\n\
         api = \"https://api.github.com\"\nlogin = \"me\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n",
    )
    .unwrap();
    h.secrets
        .set("github:github.com:me", "test", &Secret::new(TOKEN))
        .unwrap();
    // A push with nowhere to go.
    let err = h
        .try_run(
            "provider.repo_create",
            json!({ "account": "github:github.com:me", "name": "new", "private": true,
                    "push": true }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("name one"), "{err}");
    // A private key, a kind no provider takes, two keys, a key that is
    // not what it says: refused here.
    for (file, text, said) in [
        (
            "id_ed25519.pub",
            "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\n-----END OPENSSH PRIVATE KEY-----\n",
            "private key",
        ),
        (
            "dsa.pub",
            "ssh-dss AAAAB3NzaC1kc3MAAACBAP me@host\n",
            "not a kind",
        ),
        (
            "two.pub",
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5\nssh-ed25519 AAAAC3NzaC1lZDI1NTE5\n",
            "one line",
        ),
        (
            "rsa.pub",
            "ssh-ed25519 AAAAB3NzaC1yc2EAAAADAQAB me@host\n",
            "does not say",
        ),
    ] {
        let path = h.dir.join(file);
        std::fs::write(&path, text).unwrap();
        let err = h
            .try_run(
                "provider.ssh_key_add",
                json!({ "account": "github:github.com:me", "title": "T",
                        "path": path.to_str().unwrap() }),
            )
            .unwrap_err()
            .to_string();
        assert!(err.contains(said), "{file}: {err}");
    }
    assert!(h.replay.asked().is_empty());
}

#[test]
fn templates_are_read_from_the_working_tree_without_asking_anyone() {
    let h = Harness::new(json!([])).with_repository(&[]);
    let root = h.repo.as_ref().unwrap().path().to_path_buf();
    std::fs::create_dir_all(root.join(".github/PULL_REQUEST_TEMPLATE")).unwrap();
    std::fs::write(
        root.join(".github/pull_request_template.md"),
        "## What\n\n## Why\n",
    )
    .unwrap();
    std::fs::write(
        root.join(".github/PULL_REQUEST_TEMPLATE/bug.md"),
        "Fixes #\n",
    )
    .unwrap();
    let found = h.run("pr.templates", json!({}));
    assert_eq!(
        found,
        json!([
            { "path": ".github/pull_request_template.md", "name": "pull_request_template",
              "body": "## What\n\n## Why\n" },
            { "path": ".github/PULL_REQUEST_TEMPLATE/bug.md", "name": "bug", "body": "Fixes #\n" }
        ])
    );
    assert!(h.replay.asked().is_empty());
    // Without a repository there is no working tree to read.
    assert!(
        Harness::new(json!([]))
            .try_run("pr.templates", json!({}))
            .is_err()
    );
}

#[test]
fn every_provider_command_is_on_every_surface() {
    let registry = bitbraid_integrations::registry();
    let ids = [
        "account.list",
        "account.sign_in_start",
        "account.sign_in_poll",
        "account.add_token",
        "account.remove",
        "account.set_default",
        "provider.links",
        "provider.repos",
        "provider.namespaces",
        "provider.repo_create",
        "provider.fork",
        "provider.ssh_key_add",
        "provider.avatars",
        "pr.list",
        "pr.show",
        "pr.options",
        "pr.templates",
        "pr.create",
        "pr.update",
        "pr.checkout",
        "pr.head",
        "pr.merge",
        "pr.review",
        "pr.comment",
        "pr.suggestions",
        "pr.stack",
        "pr.for_branch",
        "issue.list",
        "issue.show",
        "issue.branch",
        "issue.key_from_branch",
        "ci.status",
        "inbox.items",
        "inbox.pin",
        "inbox.unpin",
        "inbox.snooze",
        "inbox.unsnooze",
        // The updater's, registered with them.
        "update.status",
        "update.check",
        "update.install",
    ];
    for id in ids {
        let spec = registry
            .get(id)
            .unwrap_or_else(|| panic!("{id} is not registered"));
        assert!(registry.access(id).is_some(), "{id}");
        let params = registry
            .params(id)
            .unwrap_or_else(|| panic!("{id} has no parameters"));
        let mut names: Vec<&str> = params.iter().map(|param| param.name).collect();
        names.sort_unstable();
        names.dedup();
        assert_eq!(names.len(), params.len(), "{id} names a parameter twice");
        // Every parameter of its table is named in its description
        // (`fresh` aside).
        if let Some((_, listed)) = spec.description.split_once("params: ") {
            for param in params {
                assert!(
                    listed.contains(param.name) || param.name == "fresh",
                    "{id}: {}",
                    param.name
                );
            }
        }
    }
    let provider_areas = [
        "account.",
        "provider.",
        "pr.",
        "issue.",
        "ci.",
        "inbox.",
        "update.",
    ];
    let registered = registry
        .iter()
        .filter(|spec| provider_areas.iter().any(|area| spec.id.starts_with(area)))
        .count();
    assert_eq!(registered, ids.len());
    assert_eq!(
        registry.len(),
        bitbraid_engine::builtin_registry().len() + ids.len()
    );
}
