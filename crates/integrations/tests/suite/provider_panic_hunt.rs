//! Panic hunt for the provider commands: every one with hostile
//! parameters, and the readers with hostile answers. Each has to end in an
//! answer or an error, never a panic, a wait without end or a buffer
//! without bound.

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
};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const ACCOUNTS: &str = "[[account]]\nid = \"github:github.com:me\"\nkind = \"github\"\n\
web = \"https://github.com\"\napi = \"https://api.github.com\"\nlogin = \"me\"\n\
sign_in = \"token\"\nscopes = []\ndefault = true\n\n\
[[account]]\nid = \"gitlab:gitlab.example.org:me\"\nkind = \"gitlab\"\n\
web = \"https://gitlab.example.org\"\napi = \"https://gitlab.example.org/api/v4\"\n\
login = \"me\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n";

struct Harness {
    dir: PathBuf,
    services: Arc<Services>,
    repo: Scratch,
}

impl Harness {
    fn new(exchanges: Value) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-p-hunt-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("integrations.toml"), ACCOUNTS).unwrap();
        let replay =
            Arc::new(Replay::from_json(&json!({ "exchanges": exchanges }).to_string()).unwrap());
        let secrets = Arc::new(MemoryStore::new());
        for id in ["github:github.com:me", "gitlab:gitlab.example.org:me"] {
            secrets.set(id, "test", &Secret::new("token")).unwrap();
        }
        let repo = Scratch::of("linear").unwrap();
        repo.git()
            .run(["remote", "add", "origin", "git@github.com:me/proj.git"])
            .unwrap();
        Self {
            services: Arc::new(Services::new(replay, secrets, &dir)),
            dir,
            repo,
        }
    }

    fn try_run(&self, id: &str, params: &Value, open: bool) -> Result<Value, CommandError> {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        if open {
            registry
                .run(
                    "repo.open",
                    &mut ctx,
                    &json!({ "path": self.repo.path().to_str().unwrap() }),
                )
                .unwrap();
        }
        registry.run(id, &mut ctx, params)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const COMMANDS: &[&str] = &[
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
];

#[test]
fn every_provider_command_survives_hostile_parameters() {
    let h = Harness::new(json!([]));
    let registry = bitbraid_integrations::registry();
    let huge = "x".repeat(1 << 20);
    let hostile_values = [
        Value::Null,
        json!(true),
        json!(-1),
        json!(u64::MAX),
        json!(1e300),
        json!(""),
        json!(huge),
        json!("a\u{0}b\r\nInjected: yes"),
        json!("../../../../etc/passwd"),
        json!("/etc/passwd"),
        json!("\u{202e}evil"),
        json!(["a", 1, null, { "x": [] }]),
        json!(vec!["a@b.c"; 1000]),
        json!({ "nested": { "deeper": [1, 2, 3] } }),
    ];
    for id in COMMANDS {
        let names: Vec<&str> = registry
            .params(id)
            .unwrap()
            .iter()
            .map(|param| param.name)
            .collect();
        // Not an object at all.
        for params in [Value::Null, json!("text"), json!([1, 2]), json!(42)] {
            for open in [false, true] {
                let _ = h.try_run(id, &params, open);
            }
        }
        // Each parameter hostile in turn, with the others sane.
        for name in &names {
            for value in &hostile_values {
                let mut params = json!({
                    "kind": "github", "account": "github:github.com:me", "owner": "me",
                    "name": "proj", "number": 1, "id": "github:github.com:me",
                    "token": "t", "device_code": "d", "emails": ["a@b.c"], "shas": ["abc123"],
                    "branch": "topic", "head": "topic", "base": "main", "title": "T",
                    "body": "B", "method": "merge", "event": "comment", "private": true,
                    "path": "/nonexistent/key.pub",
                });
                params[*name] = value.clone();
                for open in [false, true] {
                    let _ = h.try_run(id, &params, open);
                }
            }
        }
    }
    // Nothing hostile was taken for an account.
    let listed = h.try_run("account.list", &json!({}), false).unwrap();
    assert_eq!(listed.as_array().unwrap().len(), 2);
}

#[test]
fn hostile_texts_are_refused_before_anything_is_asked() {
    let h = Harness::new(json!([]));
    for (id, params) in [
        (
            "account.add_token",
            json!({ "kind": "github", "token": "a\r\nX-Injected: yes" }),
        ),
        (
            "account.add_token",
            json!({ "kind": "github", "token": "two words" }),
        ),
        ("account.add_token", json!({ "kind": "nope", "token": "t" })),
        (
            "account.add_token",
            json!({ "kind": "gitlab", "web": "http://gitlab.example.org", "token": "t" }),
        ),
        (
            "account.sign_in_poll",
            json!({ "kind": "github", "device_code": "x".repeat(10_000) }),
        ),
        (
            "pr.list",
            json!({ "account": "github:github.com:me", "owner": "../..", "name": "x" }),
        ),
        (
            "pr.list",
            json!({ "account": "github:github.com:me", "owner": "a/b", "name": "x" }),
        ),
        (
            "pr.list",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x%2F..%2Fy" }),
        ),
        (
            "pr.show",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x", "number": 0 }),
        ),
        (
            "pr.show",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x",
                    "number": 99_999_999_999u64 }),
        ),
        (
            "ci.status",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x",
                    "shas": vec!["abc"; 101] }),
        ),
        (
            "ci.status",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x",
                    "shas": ["not-hex"] }),
        ),
        (
            "provider.avatars",
            json!({ "account": "github:github.com:me", "emails": ["no-at-sign"] }),
        ),
        (
            "provider.ssh_key_add",
            json!({ "account": "github:github.com:me", "path": "../id.pub", "title": "t" }),
        ),
        (
            "provider.ssh_key_add",
            json!({ "account": "github:github.com:me", "path": "/etc/passwd", "title": "t" }),
        ),
        (
            "pr.create",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x",
                    "head": "--upload-pack=x", "base": "main", "title": "t" }),
        ),
        (
            "pr.comment",
            json!({ "account": "github:github.com:me", "owner": "me", "name": "x",
                    "number": 1, "body": "hi", "reply_to": "1; DROP" }),
        ),
    ] {
        assert!(h.try_run(id, &params, false).is_err(), "{id} took {params}");
    }
}

fn list_harness(answers: Value) -> Harness {
    Harness::new(answers)
}

/// A list read page by page through `Link` lines: GitHub's issues.
fn list(h: &Harness) -> Result<Value, CommandError> {
    h.try_run(
        "issue.list",
        &json!({ "account": "github:github.com:me", "owner": "me", "name": "proj", "state": "all" }),
        false,
    )
}

const ISSUES: &str = "/repos/me/proj/issues?per_page=100&state=all";

#[test]
fn hostile_answers_end_in_an_answer_or_an_error() {
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    for body in [
        json!({ "message": "an object, not a list" }),
        json!(["strings", "not", "pulls"]),
        json!([{ "number": "seven" }]),
        json!([{ "number": 1e30, "title": "x" }]),
        json!(null),
        json!(deep),
        json!("<html>a captive portal</html>"),
    ] {
        let h = list_harness(json!([
            { "method": "GET", "path_and_query": ISSUES, "status": 200, "body": body }
        ]));
        assert!(list(&h).is_err());
    }
    // Statuses of every kind.
    for status in [
        204, 301, 304, 400, 401, 403, 404, 409, 410, 422, 429, 500, 502, 503,
    ] {
        let h = list_harness(json!([
            { "method": "GET", "path_and_query": ISSUES, "status": status,
              "headers": { "retry-after": "999999999999", "x-ratelimit-reset": "-5",
                           "x-ratelimit-remaining": "0" },
              "body": { "message": "\u{0}\u{7}".repeat(1000) } }
        ]));
        let _ = list(&h);
    }
}

fn pull(number: u64, title: &str) -> Value {
    json!({
        "number": number, "title": title, "state": "open", "user": null,
        "head": { "ref": "x", "sha": "", "repo": null }, "base": { "ref": "main", "sha": "" },
        "created_at": "", "updated_at": "", "html_url": "",
        "labels": [ { "name": "\u{0}bad", "color": "zzzzzz" } ]
    })
}

#[test]
fn texts_from_a_provider_are_cut_and_cleaned() {
    let long = "t\u{1b}[31m".repeat(100_000);
    let h = list_harness(json!([
        { "method": "GET", "path_and_query": ISSUES, "status": 200,
          "body": [ pull(1, &long) ] }
    ]));
    let listing = list(&h).unwrap();
    let title = listing["items"][0]["title"].as_str().unwrap();
    assert!(title.chars().count() <= bitbraid_integrations::model::MAX_TITLE);
    assert!(!title.chars().any(char::is_control));
    assert_eq!(listing["items"][0]["labels"][0]["color"], Value::Null);
    assert!(
        !listing["items"][0]["labels"][0]["name"]
            .as_str()
            .unwrap()
            .chars()
            .any(char::is_control)
    );
}

#[test]
fn a_link_that_loops_or_leads_away_is_not_followed() {
    // To itself.
    let h = list_harness(json!([
        { "method": "GET", "path_and_query": ISSUES, "status": 200,
          "headers": { "link": format!("<https://api.github.com{ISSUES}>; rel=\"next\"") },
          "body": [ pull(1, "a") ] }
    ]));
    let listing = list(&h).unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 1);
    assert_eq!(listing["more"], true);
    // Two pages that name each other.
    let second = "/repos/me/proj/pulls?state=all&per_page=100&page=2";
    let h = list_harness(json!([
        { "method": "GET", "path_and_query": ISSUES, "status": 200,
          "headers": { "link": format!("<https://api.github.com{second}>; rel=\"next\"") },
          "body": [ pull(1, "a") ] },
        { "method": "GET", "path_and_query": second, "status": 200,
          "headers": { "link": format!("<https://api.github.com{ISSUES}>; rel=\"next\"") },
          "body": [ pull(2, "b") ] }
    ]));
    let listing = list(&h).unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 2);
    // Away from the API: to another host, another scheme, another path.
    for away in [
        "https://evil.example/repos/me/proj/pulls?page=2",
        "http://api.github.com/repos/me/proj/pulls?page=2",
        "https://api.github.com.evil.example/x",
        "javascript:alert(1)",
    ] {
        let h = list_harness(json!([
            { "method": "GET", "path_and_query": ISSUES, "status": 200,
              "headers": { "link": format!("<{away}>; rel=\"next\"") },
              "body": [ pull(1, "a") ] }
        ]));
        let listing = list(&h).unwrap();
        assert_eq!(listing["items"].as_array().unwrap().len(), 1, "{away}");
    }
    // Pages without end: at most fifty are read, and the rest is said to
    // be there.
    let mut pages = Vec::new();
    for page in 1..=60u64 {
        let path = if page == 1 {
            ISSUES.to_owned()
        } else {
            format!("{ISSUES}&page={page}")
        };
        pages.push(json!({
            "method": "GET", "path_and_query": path, "status": 200,
            "headers": { "link": format!("<https://api.github.com{ISSUES}&page={}>; rel=\"next\"", page + 1) },
            "body": [ pull(page, "p") ]
        }));
    }
    let h = list_harness(Value::Array(pages));
    let listing = list(&h).unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 50);
    assert_eq!(listing["more"], true);
}

#[test]
fn a_huge_list_is_cut_at_what_is_kept() {
    let many: Vec<Value> = (1..=1500).map(|number| pull(number, "p")).collect();
    let h = list_harness(json!([
        { "method": "GET", "path_and_query": ISSUES, "status": 200, "body": many }
    ]));
    let listing = list(&h).unwrap();
    assert_eq!(listing["items"].as_array().unwrap().len(), 1000);
    assert_eq!(listing["more"], true);
}

#[test]
fn a_hostile_device_flow_answer_is_refused() {
    for body in [
        json!({ "device_code": "d", "user_code": "\u{0}CODE", "verification_uri": "https://github.com/login/device" }),
        json!({ "device_code": "", "user_code": "C", "verification_uri": "https://github.com/login/device" }),
        json!({ "device_code": "d", "user_code": "C", "verification_uri": "javascript:alert(1)" }),
        json!({ "device_code": "d", "user_code": "C", "verification_uri": "http://github.com/login/device" }),
        json!({ "device_code": "d", "user_code": "C" }),
        json!({ "device_code": "d", "user_code": "x".repeat(100), "verification_uri": "https://github.com/login/device" }),
        json!([]),
    ] {
        let h = Harness::new(json!([
            { "method": "POST", "path_and_query": "/login/device/code", "status": 200, "body": body }
        ]));
        // The client id comes from the environment or the build: give one.
        let mut services = Services::new(
            Arc::new(
                Replay::from_json(
                    &json!({ "exchanges": [
                        { "method": "POST", "path_and_query": "/login/device/code", "status": 200, "body": body }
                    ] })
                    .to_string(),
                )
                .unwrap(),
            ),
            Arc::new(MemoryStore::new()),
            &h.dir,
        );
        services.client_ids.github = "Iv1.test".to_owned();
        let mut ctx = CommandContext {
            config_dir: Some(h.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(Arc::new(services));
        let result = bitbraid_integrations::registry().run(
            "account.sign_in_start",
            &mut ctx,
            &json!({ "kind": "github" }),
        );
        assert!(result.is_err(), "{body}: {result:?}");
    }
    // Numbers beyond sense are kept within it.
    let h = Harness::new(json!([]));
    let mut services = Services::new(
        Arc::new(
            Replay::from_json(
                &json!({ "exchanges": [
                    { "method": "POST", "path_and_query": "/login/device/code", "status": 200,
                      "body": { "device_code": "d", "user_code": "C",
                                "verification_uri": "https://github.com/login/device",
                                "expires_in": u64::MAX, "interval": 0 } }
                ] })
                .to_string(),
            )
            .unwrap(),
        ),
        Arc::new(MemoryStore::new()),
        &h.dir,
    );
    services.client_ids.github = "Iv1.test".to_owned();
    let mut ctx = CommandContext {
        config_dir: Some(h.dir.clone()),
        ..Default::default()
    };
    ctx.extensions.insert(Arc::new(services));
    let code = bitbraid_integrations::registry()
        .run(
            "account.sign_in_start",
            &mut ctx,
            &json!({ "kind": "github" }),
        )
        .unwrap();
    assert_eq!(code["expires_in"], 3600);
    assert_eq!(code["interval"], 1);
}

#[test]
fn a_hostile_accounts_file_is_an_error_and_is_left_as_it_is() {
    let h = Harness::new(json!([]));
    for text in [
        "not toml at all [[[",
        "[[account]]\nid = 1\n",
        "[[account]]\nid = \"x\"\nkind = \"bitbucket\"\n",
        "\u{0}\u{1}\u{2}",
    ] {
        std::fs::write(h.dir.join("integrations.toml"), text).unwrap();
        assert!(
            h.try_run("account.list", &json!({}), false).is_err(),
            "{text:?}"
        );
        assert!(
            h.try_run(
                "account.add_token",
                &json!({ "kind": "github", "token": "t" }),
                false
            )
            .is_err()
        );
        assert_eq!(
            std::fs::read_to_string(h.dir.join("integrations.toml")).unwrap(),
            text
        );
    }
}

/// A provider that answers every request the same, whatever it was.
#[derive(Debug)]
struct Same {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    asked: std::sync::atomic::AtomicUsize,
}

impl bitbraid_integrations::http::Fetch for Same {
    fn send(
        &self,
        _: &bitbraid_integrations::http::Call,
        _: &std::sync::atomic::AtomicBool,
    ) -> Result<bitbraid_integrations::http::Response, bitbraid_integrations::http::FetchError>
    {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Ok(bitbraid_integrations::http::Response {
            status: self.status,
            headers: self.headers.clone(),
            body: self.body.clone(),
            ..Default::default()
        })
    }
}

/// Run every reader of both providers against a provider that answers
/// `body` to everything: an answer or an error, never a panic, and a
/// bounded number of requests.
fn every_reader_against(status: u16, headers: &[(&str, &str)], body: &[u8]) {
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("bb-p-hunt-same-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("integrations.toml"), ACCOUNTS).unwrap();
    let secrets = Arc::new(MemoryStore::new());
    for id in ["github:github.com:me", "gitlab:gitlab.example.org:me"] {
        secrets.set(id, "test", &Secret::new("token")).unwrap();
    }
    let same = Arc::new(Same {
        status,
        headers: headers
            .iter()
            .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
            .collect(),
        body: body.to_vec(),
        asked: std::sync::atomic::AtomicUsize::new(0),
    });
    let services = Arc::new(Services::new(same.clone(), secrets, &dir));
    let registry = bitbraid_integrations::registry();
    for account in ["github:github.com:me", "gitlab:gitlab.example.org:me"] {
        let on = |extra: Value| {
            let mut params = json!({ "account": account, "owner": "me", "name": "proj" });
            for (key, value) in extra.as_object().unwrap() {
                params[key] = value.clone();
            }
            params
        };
        for (id, params) in [
            ("pr.list", on(json!({ "state": "all" }))),
            ("pr.list", on(json!({ "query": "label:a,b author:@me" }))),
            ("pr.show", on(json!({ "number": 1 }))),
            ("pr.options", on(json!({}))),
            ("pr.stack", on(json!({ "number": 1 }))),
            ("pr.for_branch", on(json!({ "branch": "topic" }))),
            ("issue.list", on(json!({ "state": "all" }))),
            ("issue.list", on(json!({ "filter": "mentioned" }))),
            ("issue.list", on(json!({ "query": "milestone:v1" }))),
            ("issue.show", on(json!({ "number": 1 }))),
            (
                "ci.status",
                on(json!({ "shas": ["abc123", "0123456789abcdef0123456789abcdef01234567"] })),
            ),
            (
                "provider.avatars",
                on(json!({ "emails": ["a@example.org", "1+me@users.noreply.github.com"] })),
            ),
            ("provider.repos", json!({ "account": account })),
            (
                "provider.repos",
                json!({ "account": account, "query": "x", "page": 3 }),
            ),
            ("provider.namespaces", json!({ "account": account })),
            ("inbox.items", json!({ "account": account, "wip": false })),
        ] {
            let before = same.asked.load(Ordering::Relaxed);
            let mut ctx = CommandContext {
                config_dir: Some(dir.clone()),
                ..Default::default()
            };
            ctx.extensions.insert(services.clone());
            let _ = registry.run(id, &mut ctx, &params);
            let asked = same.asked.load(Ordering::Relaxed) - before;
            assert!(asked <= 2000, "{id} asked {asked} times");
        }
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn every_reader_survives_hostile_answers() {
    // Sizes just past the bounds, not volume: a text longer than any
    // kept (2,400 characters, past MAX_TITLE and the 2,048 of an
    // address), a list longer than any shown at once.
    let long = "\u{1b}[31m\u{0}x\u{202e}".repeat(300);
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let user = json!({ "login": long, "username": long, "name": long, "avatar_url": long,
                       "id": u64::MAX });
    let wide: Vec<Value> = (0..120)
        .map(|n| json!({ "login": format!("u{n}"), "username": format!("u{n}"), "id": n }))
        .collect();
    for body in [
        json!(null),
        json!([]),
        json!({}),
        json!("text"),
        json!(1e300),
        json!(true),
        json!(deep),
        json!([null, 1, "two", [], {}]),
        // Nulls where objects were, numbers where texts were.
        json!({ "number": null, "iid": null, "title": null, "state": null, "user": null,
                "head": null, "base": null, "author": null, "labels": null, "commit": null }),
        json!([{ "number": 1, "iid": 1, "title": 7, "state": [], "head": { "ref": {} } }]),
        json!([{ "number": u64::MAX, "iid": -1, "title": long, "state": "open",
                 "created_at": long, "updated_at": long, "html_url": long, "web_url": long,
                 "source_branch": long, "target_branch": long, "user": user, "author": user,
                 "head": { "ref": long, "sha": long, "repo": { "full_name": long } },
                 "base": { "ref": long, "sha": long },
                 "labels": [ { "name": long, "color": long } ], "assignees": wide }]),
        json!(wide),
        // GraphQL: errors, partial data, nulls, a cursor that loops.
        json!({ "errors": [ { "message": long, "type": "NOT_FOUND" } ] }),
        json!({ "errors": [ { "message": long, "type": "RATE_LIMITED" } ], "data": null }),
        json!({ "errors": [ { "message": "\u{0}" } ], "data": null }),
        json!({ "errors": "not a list", "data": 5 }),
        json!({ "data": { "search": null, "repository": null, "project": null } }),
        json!({ "data": { "search": { "pageInfo": { "hasNextPage": true, "endCursor": "same" },
                                      "nodes": [ { "number": 1, "title": long }, null, 5 ] } } }),
        json!({ "data": { "repository": { "pullRequest": { "reviewThreads": { "nodes": [ null,
                  { "isResolved": "yes", "comments": { "nodes": [ { "databaseId": -1 } ] } } ] },
                  "commits": { "nodes": [ { "commit": { "statusCheckRollup": { "contexts": {
                  "nodes": [ { "__typename": "CheckRun", "name": long, "status": long },
                             { "__typename": "Odd" }, null ] } } } } ] } },
                  "c0": { "statusCheckRollup": null }, "c1": 7 } } }),
        json!({ "data": { "project": { "mergeRequests": { "nodes": [ { "iid": "x" }, null,
                  { "iid": "1", "reviewers": { "nodes": [ { "username": null } ] } } ] } } } }),
        json!({ "avatar_url": long }),
        json!({ "avatar_url": "javascript:alert(1)" }),
    ] {
        every_reader_against(200, &[], body.to_string().as_bytes());
    }
    // A link to the same page every time, and one that runs on.
    let page = json!([{ "number": 1, "iid": 1, "title": "t", "state": "open",
                        "created_at": "", "updated_at": "", "html_url": "", "web_url": "",
                        "source_branch": "a", "target_branch": "b",
                        "head": { "ref": "a", "sha": "" }, "base": { "ref": "b", "sha": "" } }]);
    for link in [
        "<https://api.github.com/repos/me/proj/pulls?page=2>; rel=\"next\"",
        "<https://gitlab.example.org/api/v4/projects/me%2Fproj/issues?page=2>; rel=\"next\"",
        "<https://evil.example/x>; rel=\"next\"",
        "garbage",
    ] {
        every_reader_against(200, &[("link", link)], page.to_string().as_bytes());
    }
    // Not JSON at all, and statuses of every kind.
    every_reader_against(200, &[], b"<html>a captive portal</html>");
    every_reader_against(200, &[], &[0xff, 0xfe, 0x00]);
    for status in [304, 401, 403, 404, 422, 429, 500, 502] {
        every_reader_against(
            status,
            &[("x-ratelimit-remaining", "0"), ("retry-after", "-1")],
            b"{\"message\":\"no\"}",
        );
    }
}

#[test]
fn graphql_errors_are_the_errors_they_name() {
    let h = Harness::new(json!([
        { "method": "POST", "path_and_query": "/graphql", "status": 200,
          "headers": { "x-ratelimit-reset": "1791801600" },
          "body": { "data": null, "errors": [ { "type": "RATE_LIMITED",
                    "message": "API rate limit exceeded" } ] } },
        { "method": "POST", "path_and_query": "/graphql", "status": 200,
          "body": { "data": { "repository": null }, "errors": [ { "type": "NOT_FOUND",
                    "message": "Could not resolve to a Repository" } ] } },
        { "method": "POST", "path_and_query": "/graphql", "status": 200,
          "body": { "errors": [ { "message": "Field 'x' doesn't exist on type 'Query'" } ] } },
        // Data with errors beside it: the data is taken, missing parts and
        // all.
        { "method": "POST", "path_and_query": "/graphql", "status": 200,
          "body": { "data": { "repository": { "c0": null } },
                    "errors": [ { "type": "FORBIDDEN", "message": "Resource not accessible" } ] } }
    ]));
    let ci = || {
        h.try_run(
            "ci.status",
            &json!({ "account": "github:github.com:me", "owner": "me", "name": "proj",
                     "shas": ["abc123"] }),
            false,
        )
    };
    let limited = ci().unwrap_err().to_string();
    assert!(
        limited.contains("allowance") && limited.contains("2026-10-12T"),
        "{limited}"
    );
    assert!(ci().unwrap_err().to_string().contains("not found"));
    assert!(ci().unwrap_err().to_string().contains("doesn't exist"));
    let partial = ci().unwrap();
    assert_eq!(partial[0]["state"], "none");
}
