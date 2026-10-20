//! The provider commands with their answers kept: a list read twice is
//! asked the second time whether it changed, `fresh` asks
//! without offering what was kept, and when the provider cannot be
//! reached or its allowance is spent the kept list is answered, said to
//! be offline, with when it was read and when the allowance comes back.
//!
//! The network is a stand-in that answers what the test lines up, from
//! the examples of docs.gitlab.com/api "Issues" and of GitHub's GraphQL
//! `search`, and notes the header lines it was asked with. Nothing leaves
//! the machine.

use std::{
    cell::Cell,
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    http::{Call, Fetch, FetchError, Response},
    secrets::{MemoryStore, Secret, SecretStore},
};
use serde_json::{Value, json};

thread_local! {
    /// The time the stand-in clock says, set by each test on its thread.
    static NOW: Cell<i64> = const { Cell::new(1_791_801_000) };
}

fn now() -> i64 {
    NOW.with(Cell::get)
}

fn later(seconds: i64) {
    NOW.with(|now| now.set(now.get() + seconds));
}

#[derive(Debug, Default)]
struct Network {
    answers: Mutex<Vec<Result<Response, FetchError>>>,
    asked: Mutex<Vec<Call>>,
}

impl Network {
    fn then(&self, answer: Result<Response, FetchError>) {
        self.answers.lock().unwrap().push(answer);
    }

    fn last_asked(&self) -> Call {
        self.asked.lock().unwrap().last().unwrap().clone()
    }

    fn count(&self) -> usize {
        self.asked.lock().unwrap().len()
    }
}

impl Fetch for Network {
    fn send(&self, call: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
        self.asked.lock().unwrap().push(call.clone());
        let mut answers = self.answers.lock().unwrap();
        if answers.is_empty() {
            return Err(FetchError::Unrecorded(call.url.path().to_owned()));
        }
        answers.remove(0)
    }
}

fn answer(status: u16, headers: &[(&str, &str)], body: &Value) -> Result<Response, FetchError> {
    Ok(Response {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: if body.is_null() {
            Vec::new()
        } else {
            body.to_string().into_bytes()
        },
        ..Default::default()
    })
}

struct Harness {
    dir: PathBuf,
    network: Arc<Network>,
    services: Arc<Services>,
}

impl Harness {
    fn new(name: &str) -> Self {
        let dir = std::env::temp_dir().join(format!("bb-p-cache-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("integrations.toml"),
            "[[account]]\nid = \"gitlab:gitlab.example.org:alice\"\nkind = \"gitlab\"\n\
             web = \"https://gitlab.example.org\"\napi = \"https://gitlab.example.org/api/v4\"\n\
             login = \"alice\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n\n\
             [[account]]\nid = \"github:github.com:octocat\"\nkind = \"github\"\n\
             web = \"https://github.com\"\napi = \"https://api.github.com\"\nlogin = \"octocat\"\n\
             sign_in = \"token\"\nscopes = []\ndefault = true\n",
        )
        .unwrap();
        let secrets = Arc::new(MemoryStore::new());
        for id in [
            "gitlab:gitlab.example.org:alice",
            "github:github.com:octocat",
        ] {
            secrets.set(id, "test", &Secret::new("token")).unwrap();
        }
        let network = Arc::new(Network::default());
        let mut services = Services::new(network.clone(), secrets, &dir);
        services.cache_dir = Some(dir.join("cache"));
        services.now = now;
        Self {
            dir,
            network,
            services: Arc::new(services),
        }
    }

    fn run(&self, id: &str, params: Value) -> Result<Value, String> {
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        bitbraid_integrations::registry()
            .run(id, &mut ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn issues(&self, fresh: bool) -> Result<Value, String> {
        self.run(
            "issue.list",
            json!({ "account": "gitlab:gitlab.example.org:alice", "owner": "grp",
                    "name": "proj", "fresh": fresh }),
        )
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn issue(iid: u64, title: &str) -> Value {
    json!({
        "id": 76 + iid, "iid": iid, "project_id": 1, "title": title,
        "description": "Ratione dolores corrupti mollitia soluta quia.", "state": "opened",
        "created_at": "2016-01-04T15:31:39.996Z", "updated_at": "2016-01-04T15:31:39.996Z",
        "labels": [], "milestone": null, "assignees": [],
        "author": { "id": 18, "username": "eileen.lowe", "name": "Alexandra Bashirian",
                    "state": "active", "avatar_url": null,
                    "web_url": "https://gitlab.example.org/eileen.lowe" },
        "user_notes_count": 1,
        "web_url": format!("https://gitlab.example.org/grp/proj/-/issues/{iid}")
    })
}

#[test]
fn a_list_read_again_is_asked_whether_it_changed() {
    let h = Harness::new("again");
    h.network.then(answer(
        200,
        &[("etag", "W/\"81b26f\""), ("ratelimit-remaining", "1999")],
        &json!([issue(1, "Consequatur vero maxime")]),
    ));
    h.network.then(answer(
        304,
        &[("ratelimit-remaining", "1998")],
        &Value::Null,
    ));
    let first = h.issues(false).unwrap();
    assert_eq!(first["items"][0]["title"], "Consequatur vero maxime");
    assert_eq!(first["freshness"]["offline"], false);
    later(60);
    let again = h.issues(false).unwrap();
    assert_eq!(again["items"], first["items"]);
    assert_eq!(again["freshness"]["offline"], false);
    assert_eq!(again["freshness"]["remaining"], 1998);
    assert_eq!(
        h.network.last_asked().header("If-None-Match"),
        Some("W/\"81b26f\"")
    );
    // Fresh: asked without the tag, and what comes is kept.
    h.network.then(answer(
        200,
        &[("etag", "W/\"92c37a\"")],
        &json!([
            issue(1, "Consequatur vero maxime"),
            issue(2, "Ut commodi ullam")
        ]),
    ));
    let fresh = h.issues(true).unwrap();
    assert_eq!(fresh["items"].as_array().unwrap().len(), 2);
    assert_eq!(h.network.last_asked().header("If-None-Match"), None);
    h.network.then(answer(304, &[], &Value::Null));
    let kept = h.issues(false).unwrap();
    assert_eq!(kept["items"].as_array().unwrap().len(), 2);
    assert_eq!(
        h.network.last_asked().header("If-None-Match"),
        Some("W/\"92c37a\"")
    );
}

#[test]
fn without_the_provider_the_kept_list_is_answered_offline() {
    let h = Harness::new("offline");
    let read_at = now();
    h.network.then(answer(
        200,
        &[],
        &json!([issue(1, "Consequatur vero maxime")]),
    ));
    h.issues(false).unwrap();
    later(3600);
    h.network
        .then(Err(FetchError::Connect("network is unreachable".into())));
    let offline = h.issues(false).unwrap();
    assert_eq!(offline["items"][0]["title"], "Consequatur vero maxime");
    assert_eq!(offline["freshness"]["offline"], true);
    assert_eq!(
        offline["freshness"]["fetched"],
        bitbraid_integrations::api::rfc3339(read_at)
    );
    // Asked fresh, it is still offline: there is nothing else.
    h.network.then(Err(FetchError::Timeout));
    assert_eq!(h.issues(true).unwrap()["freshness"]["offline"], true);
    // What was never read cannot be answered without the provider.
    h.network
        .then(Err(FetchError::Connect("network is unreachable".into())));
    let never = h.run(
        "issue.list",
        json!({ "account": "gitlab:gitlab.example.org:alice", "owner": "grp",
                "name": "other" }),
    );
    assert!(never.unwrap_err().contains("no connection"));
}

#[test]
fn a_spent_allowance_is_waited_out_on_the_kept_list() {
    let h = Harness::new("spent");
    h.network.then(answer(
        200,
        &[],
        &json!([issue(1, "Consequatur vero maxime")]),
    ));
    h.issues(false).unwrap();
    let reset = now() + 900;
    h.network.then(answer(
        429,
        &[
            ("ratelimit-remaining", "0"),
            ("ratelimit-reset", &reset.to_string()),
        ],
        &json!({ "message": "Retry later" }),
    ));
    let limited = h.issues(false).unwrap();
    assert_eq!(limited["freshness"]["offline"], true);
    assert_eq!(limited["freshness"]["remaining"], 0);
    assert_eq!(
        limited["freshness"]["reset"],
        bitbraid_integrations::api::rfc3339(reset)
    );
    // Not asked again until then.
    let asked = h.network.count();
    assert_eq!(h.issues(false).unwrap()["freshness"]["offline"], true);
    assert_eq!(h.network.count(), asked);
    // After it, asked again.
    later(reset + 1 - now());
    h.network.then(answer(304, &[], &Value::Null));
    assert_eq!(h.issues(false).unwrap()["freshness"]["offline"], false);
    assert_eq!(h.network.count(), asked + 1);
}

#[test]
fn a_graphql_question_is_kept_and_answered_offline() {
    let h = Harness::new("graphql");
    let page = json!({ "data": { "search": {
        "pageInfo": { "hasNextPage": false, "endCursor": null },
        "nodes": [ { "number": 1347, "title": "Amazing new feature", "state": "OPEN",
                     "headRefName": "new-topic", "baseRefName": "master",
                     "createdAt": "2011-01-26T19:01:12Z", "updatedAt": "2011-01-26T19:01:12Z",
                     "url": "https://github.com/octocat/Hello-World/pull/1347" } ] } } });
    h.network
        .then(answer(200, &[("x-ratelimit-remaining", "4999")], &page));
    let list = || {
        h.run(
            "pr.list",
            json!({ "account": "github:github.com:octocat", "owner": "octocat",
                    "name": "Hello-World" }),
        )
    };
    assert_eq!(list().unwrap()["items"][0]["number"], 1347);
    h.network
        .then(answer(502, &[], &json!({ "message": "Server Error" })));
    let offline = list().unwrap();
    assert_eq!(offline["items"][0]["number"], 1347);
    assert_eq!(offline["freshness"]["offline"], true);
    // Another question is another answer: nothing kept for it.
    h.network
        .then(answer(502, &[], &json!({ "message": "Server Error" })));
    let mine = h.run(
        "pr.list",
        json!({ "account": "github:github.com:octocat", "owner": "octocat",
                "name": "Hello-World", "filter": "mine" }),
    );
    assert!(mine.unwrap_err().contains("502"));
}

/// A recorded provider that can be cut off, as when its server stops.
#[derive(Debug)]
struct Unplugged {
    replay: bitbraid_integrations::replay::Replay,
    off: AtomicBool,
}

impl Fetch for Unplugged {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if self.off.load(std::sync::atomic::Ordering::SeqCst) {
            return Err(FetchError::Connect("network is unreachable".into()));
        }
        self.replay.send(call, cancel)
    }
}

/// A pull request or an issue read again while its provider cannot be
/// reached is the one kept from before, said to be offline with when it
/// was read, at GitHub and at GitLab. Without the mark, the details' view
/// could not say that the refresh had not reached the provider.
#[test]
fn a_pull_request_read_without_the_provider_is_the_kept_one_said_to_be_offline() {
    let github = json!({ "kind": "github", "token": "ghx_replayed" });
    let gitlab = json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929",
                         "token": "glpat-replayed" });
    let cases = [
        (
            "github/pr_show.json",
            github.clone(),
            "pr.show",
            json!({ "account": "github:github.com:user-1", "owner": "user-1",
                    "name": "bitbraid-test-core", "number": 1 }),
        ),
        (
            "gitlab/pr_show.json",
            gitlab.clone(),
            "pr.show",
            json!({ "account": "gitlab:127.0.0.1:8929:user-1", "owner": "bb-p-group",
                    "name": "bb-p-project", "number": 2 }),
        ),
        (
            "github/issues.json",
            github,
            "issue.show",
            json!({ "account": "github:github.com:user-1", "owner": "user-1",
                    "name": "bitbraid-test-core", "number": 7 }),
        ),
        (
            "gitlab/issues.json",
            gitlab,
            "issue.show",
            json!({ "account": "gitlab:127.0.0.1:8929:user-1", "owner": "bb-p-group",
                    "name": "bb-p-project", "number": 1 }),
        ),
    ];
    for (at, (recording, token, command, shown)) in cases.into_iter().enumerate() {
        let dir = std::env::temp_dir().join(format!("bb-p-cache-show-{at}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/recordings")
            .join(recording);
        let fetch = Arc::new(Unplugged {
            replay: bitbraid_integrations::replay::Replay::load(&path).unwrap(),
            off: AtomicBool::new(false),
        });
        let mut services = Services::new(fetch.clone(), Arc::new(MemoryStore::new()), &dir);
        services.cache_dir = Some(dir.join("cache"));
        services.now = now;
        let services = Arc::new(services);
        let run = |id: &str, params: Value| {
            let mut ctx = CommandContext {
                config_dir: Some(dir.clone()),
                ..Default::default()
            };
            ctx.extensions.insert(services.clone());
            bitbraid_integrations::registry()
                .run(id, &mut ctx, &params)
                .unwrap_or_else(|err| panic!("{recording}: {id}: {err}"))
        };
        run("account.add_token", token);
        let read_at = now();
        let online = run(command, shown.clone());
        assert_eq!(online["freshness"]["offline"], false, "{recording}");
        later(3600);
        fetch.off.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut fresh = shown;
        fresh["fresh"] = json!(true);
        let offline = run(command, fresh);
        let mut kept = offline.clone();
        kept["freshness"] = online["freshness"].clone();
        assert_eq!(kept, online, "{recording}: what was kept is answered");
        assert_eq!(offline["freshness"]["offline"], true, "{recording}");
        assert_eq!(
            offline["freshness"]["fetched"],
            bitbraid_integrations::api::rfc3339(read_at),
            "{recording}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
