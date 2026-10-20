//! A branch for an issue, live against a GitLab of one's own: a user of
//! this run alone (`bb-a2-<hex>`) makes a project with an issue; a
//! repository on this machine whose `origin` is that project lists the
//! issue, shows it, makes a branch named from its title with
//! `issue.branch` and checks it out; `issue.key_from_branch` reads the
//! number back from the name. The user, and with her the project, is
//! removed when the test ends.
//!
//! Opt-in and never in ci: it runs with `--ignored` when
//! `BITBRAID_GITLAB_CE` names the server (`http://127.0.0.1:8929`) and
//! `BITBRAID_GITLAB_CE_TOKEN_FILE` a file holding an administrator's
//! personal access token (scope `api`), which is read here and sent to
//! that server only:
//!
//! ```sh
//! BITBRAID_GITLAB_CE=http://127.0.0.1:8929 BITBRAID_GITLAB_CE_TOKEN_FILE=<file> \
//!     cargo test -p bitbraid-integrations --test suite live_issue_branch:: -- --ignored
//! ```
//!
//! Sources: docs.gitlab.com/api "Users" (create, personal access tokens,
//! delete), "Projects" (create), "Issues" (create).

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    api::rfc3339,
    http::{Call, Client, Fetch as _, Method, Url},
    secrets::{MemoryStore, Secret},
};
use serde_json::{Value, json};
use sha2::Digest as _;

/// The server and the administrator's token, when the test is asked for:
/// a server on this machine only.
fn server() -> Option<(String, Secret)> {
    let base = std::env::var("BITBRAID_GITLAB_CE").ok()?;
    let file = std::env::var("BITBRAID_GITLAB_CE_TOKEN_FILE").ok()?;
    let token = std::fs::read_to_string(file).ok()?.trim().to_owned();
    let base = base.trim().trim_end_matches('/').to_owned();
    let url = Url::parse(&base).ok()?;
    let local = url.host() == "localhost"
        || url
            .host()
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    local.then(|| (base, Secret::new(token)))
}

/// Something new for this run, as hexadecimal digits.
fn fresh(what: &str) -> String {
    let seed = format!(
        "{what}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    );
    sha2::Sha256::digest(seed.as_bytes())
        .iter()
        .take(12)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The API, as the administrator or as a user.
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
            bitbraid_integrations::http::Limits {
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
}

/// Removes the user of the run, and what she made, when dropped.
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

/// A repository on this machine whose `origin` is `address`.
fn scratch_repository(dir: &Path, address: &str) -> PathBuf {
    let repo = dir.join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let git = bitbraid_testkit::Git::new(&repo, dir.join(".home"));
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git.run(["add", "a.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "A file"]).unwrap();
    git.run(["remote", "add", "origin", address]).unwrap();
    repo
}

#[test]
#[ignore = "live: needs BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE"]
fn a_branch_is_made_for_an_issue_of_a_gitlab_of_ones_own() {
    let Some((base, token)) = server() else {
        panic!("set BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE");
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token,
    };
    // A user of this run alone, with a token of her own.
    let hex = fresh("user");
    let username = format!("bb-a2-{}", hex.get(..10).unwrap());
    let made = admin.ok(
        Method::Post,
        "/users",
        Some(json!({ "username": username, "name": "Dora Tester",
                     "email": format!("{username}@example.invalid"),
                     "password": fresh("password"), "skip_confirmation": true })),
    );
    let id = made["id"].as_u64().unwrap();
    let _gone = Gone { admin: &admin, id };
    let expires = rfc3339(bitbraid_core::clock::unix_seconds() + 86_400);
    let user_token = admin.ok(
        Method::Post,
        &format!("/users/{id}/personal_access_tokens"),
        Some(
            json!({ "name": fresh("token"), "scopes": ["api", "read_user"],
                     "expires_at": expires.get(..10).unwrap() }),
        ),
    )["token"]
        .as_str()
        .unwrap()
        .to_owned();
    let user = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token: Secret::new(user_token.clone()),
    };
    let project = user.ok(
        Method::Post,
        "/projects",
        Some(json!({ "name": "bb-a2-issues", "path": "bb-a2-issues",
                     "initialize_with_readme": true, "visibility": "private" })),
    );
    let project_id = project["id"].as_u64().unwrap();
    let issue = user.ok(
        Method::Post,
        &format!("/projects/{project_id}/issues"),
        Some(json!({ "title": "Quotes are dropped: in the lexer!",
                     "description": "Seen with `\"a b\"`." })),
    )["iid"]
        .as_u64()
        .unwrap();

    // The commands, as the window runs them.
    let dir = std::env::temp_dir().join(format!("bb-a2-live-issue-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let services = Arc::new(Services::new(
        Arc::new(Client::new("BitBraid-Test/live")),
        Arc::new(MemoryStore::new()),
        dir.join("config"),
    ));
    let registry = bitbraid_integrations::registry();
    let context = || {
        let mut ctx = CommandContext {
            config_dir: Some(dir.join("config")),
            ..Default::default()
        };
        ctx.extensions.insert(services.clone());
        ctx
    };
    let account = registry
        .run(
            "account.add_token",
            &mut context(),
            &json!({ "kind": "gitlab", "web": base, "token": user_token }),
        )
        .unwrap();
    assert_eq!(account["login"], username.as_str());
    let repo = scratch_repository(&dir, &format!("{base}/{username}/bb-a2-issues.git"));
    let mut ctx = context();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": repo }))
        .unwrap();
    let links = registry
        .run("provider.links", &mut ctx, &json!({}))
        .unwrap();
    assert_eq!(links[0]["owner"], username.as_str(), "{links}");
    assert_eq!(links[0]["name"], "bb-a2-issues");

    let listed = registry.run("issue.list", &mut ctx, &json!({})).unwrap();
    let numbers: Vec<u64> = listed["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["number"].as_u64().unwrap())
        .collect();
    assert_eq!(numbers, [issue]);
    let shown = registry
        .run("issue.show", &mut ctx, &json!({ "number": issue }))
        .unwrap();
    assert_eq!(shown["issue"]["title"], "Quotes are dropped: in the lexer!");

    let made = registry
        .run(
            "issue.branch",
            &mut ctx,
            &json!({ "number": issue, "checkout": true }),
        )
        .unwrap();
    let name = format!("{issue}-quotes-are-dropped-in-the-lexer");
    assert_eq!(made["created"], name.as_str());
    let git = bitbraid_testkit::Git::new(&repo, dir.join(".home"));
    assert_eq!(
        git.run(["rev-parse", "--abbrev-ref", "HEAD"])
            .unwrap()
            .trim(),
        name
    );
    git.run(["check-ref-format", "--branch", &name]).unwrap();
    let key = registry
        .run(
            "issue.key_from_branch",
            &mut ctx,
            &json!({ "branch": name }),
        )
        .unwrap();
    assert_eq!(key["number"], issue);
    let peek = registry.run("undo.peek", &mut ctx, &json!({})).unwrap();
    assert_eq!(peek["undo"], format!("Create branch {name}").as_str());
    let _ = std::fs::remove_dir_all(&dir);
}
