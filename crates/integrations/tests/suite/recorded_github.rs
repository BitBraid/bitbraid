//! The provider commands' reads against answers recorded from github.com
//! with the test accounts (`tests/recordings/github-org/`, made by
//! `tests/suite/live_github.rs` with `BITBRAID_RECORD`, scrubbed): the
//! organisation is `org-1`, its machine account `user-1` and the reviewer
//! `user-2`; the reads' repository is `org-1/bitbraid-live`, public, with check
//! runs of GitHub Actions, reviews by the reviewer and a pull request from
//! `user-1`'s fork. What the commands make of the answers is held here, in
//! ci, without a network.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{
    Services,
    oauth::GITHUB_CLIENT_ID,
    replay::{Recording, Replay},
    secrets::MemoryStore,
};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The machine account and the reviewer, as the recordings name them.
const MAIN: &str = "github:github.com:user-1";
const REVIEWER: &str = "github:github.com:user-2";

fn recording(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recordings/github-org")
        .join(name)
}

struct Harness {
    dir: PathBuf,
    replay: Arc<Replay>,
    services: Arc<Services>,
}

impl Harness {
    /// A recording, and as many accounts connected as the live test did:
    /// the machine account first, then the reviewer.
    fn new(name: &str, accounts: usize) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-g-recorded-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = Arc::new(Replay::load(&recording(name)).unwrap());
        let services = Services::new(replay.clone(), Arc::new(MemoryStore::new()), &dir);
        let h = Self {
            dir,
            replay,
            services: Arc::new(services),
        };
        for (at, id) in [MAIN, REVIEWER].into_iter().take(accounts).enumerate() {
            let account = h.run(
                "account.add_token",
                json!({ "kind": "github", "token": format!("ghx_replayed_{at}") }),
            );
            assert_eq!(account["id"], id);
        }
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

    /// A command about the reads' repository, as `account`.
    fn on(&self, account: &str, id: &str, extra: Value) -> Value {
        let mut params = json!({ "account": account, "owner": "org-1", "name": "bitbraid-live" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.run(id, params)
    }

    fn all_used(&self) {
        assert!(
            self.replay.unused().is_empty(),
            "{:?}",
            self.replay.unused()
        );
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn numbers(listing: &Value) -> Vec<u64> {
    listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|item| item["number"].as_u64().unwrap())
        .collect()
}

fn sorted(mut numbers: Vec<u64>) -> Vec<u64> {
    numbers.sort_unstable();
    numbers
}

/// The pull requests of the reads' repository: `bb-topic`, `bb-changes`,
/// the draft, the two stacked, the merged and the closed one, and the one
/// from the fork; then the issues.
const TOPIC: u64 = 1;
const CHANGES: u64 = 2;
const DRAFT: u64 = 3;
const STACK: [u64; 2] = [4, 5];
const MERGED: u64 = 6;
const CLOSED: u64 = 7;
const FORK: u64 = 8;
const OPEN: [u64; 6] = [TOPIC, CHANGES, DRAFT, STACK[0], STACK[1], FORK];
const ISSUE: u64 = 9;
const CLOSED_ISSUE: u64 = 10;
const PLAIN_ISSUE: u64 = 11;

#[test]
fn every_recording_of_the_test_accounts_was_recorded_live() {
    for entry in std::fs::read_dir(recording("")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let recorded: Recording = serde_json::from_str(&text).unwrap();
        assert_eq!(recorded.hand_made, None, "{}", path.display());
        assert!(
            recorded.note.contains("live_github.rs"),
            "{}",
            path.display()
        );
    }
}

#[test]
fn github_org_a_token_connects_and_the_pull_requests_are_listed() {
    let h = Harness::new("add_token_and_pr_list.json", 1);
    let accounts = h.run("account.list", json!({}));
    assert_eq!(accounts[0]["login"], "user-1");
    assert_eq!(accounts[0]["api"], "https://api.github.com");
    let scopes = accounts[0]["scopes"].as_array().unwrap();
    for scope in ["repo", "workflow", "read:org", "admin:public_key"] {
        assert!(scopes.iter().any(|known| known == scope), "{scope}");
    }
    let listing = h.on(MAIN, "pr.list", json!({ "state": "all" }));
    assert_eq!(numbers(&listing), [8, 7, 6, 5, 4, 3, 2, 1]);
    let topic = &listing["items"][7];
    assert_eq!(topic["head_ref"], "bb-topic");
    assert_eq!(topic["repo"], "org-1/bitbraid-live");
    assert_eq!(topic["author"]["login"], "user-1");
    assert!(listing["freshness"]["remaining"].is_u64());
    h.all_used();
}

#[test]
fn github_org_pull_requests_are_listed_by_every_filter_and_query_from_both_sides() {
    let h = Harness::new("pr_list.json", 2);
    let all = h.on(MAIN, "pr.list", json!({ "state": "all" }));
    assert_eq!(numbers(&all), [8, 7, 6, 5, 4, 3, 2, 1]);
    let row = |number: u64| {
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["number"] == number)
            .unwrap()
            .clone()
    };
    let topic = row(TOPIC);
    assert_eq!(topic["title"], "Add a file to review");
    assert_eq!(topic["labels"][0]["name"], "bug");
    assert_eq!(topic["labels"][0]["color"], "d73a4a");
    assert_eq!(topic["assignees"][0]["login"], "user-1");
    assert_eq!(topic["ci"], "failure");
    assert_eq!(topic["comments"], 2);
    assert_eq!(
        topic["reviewers"],
        json!([{ "user": topic["reviewers"][0]["user"], "state": "approved" }])
    );
    assert_eq!(topic["reviewers"][0]["user"]["login"], "user-2");
    assert_eq!(
        topic["web_url"],
        "https://github.com/org-1/bitbraid-live/pull/1"
    );
    assert_eq!(row(CHANGES)["reviewers"][0]["state"], "changes_requested");
    assert_eq!(row(CHANGES)["ci"], "success");
    let draft = row(DRAFT);
    assert_eq!(draft["draft"], true);
    assert_eq!(draft["ci"], "pending");
    assert_eq!(draft["reviewers"][0]["user"]["login"], "user-2");
    assert_eq!(draft["reviewers"][0]["state"], "pending");
    assert_eq!(row(STACK[0])["ci"], "failure");
    assert_eq!(row(STACK[1])["base_ref"], "bb-stack-1");
    assert_eq!(row(MERGED)["state"], "merged");
    assert_eq!(row(CLOSED)["state"], "closed");
    let fork = row(FORK);
    assert_eq!(fork["head_repo"], "user-1/bitbraid-live");
    assert_eq!(fork["head_ref"], "bb-fork-topic");
    assert_eq!(fork["ci"], "success");
    let list = |extra: Value| sorted(numbers(&h.on(MAIN, "pr.list", extra)));
    let by_reviewer = |extra: Value| sorted(numbers(&h.on(REVIEWER, "pr.list", extra)));
    assert_eq!(list(json!({})), OPEN);
    assert_eq!(list(json!({ "filter": "mine" })), OPEN);
    assert_eq!(list(json!({ "filter": "assigned" })), [TOPIC]);
    assert_eq!(
        list(json!({ "filter": "review_requested" })),
        Vec::<u64>::new()
    );
    assert_eq!(
        by_reviewer(json!({ "filter": "review_requested" })),
        [DRAFT]
    );
    assert_eq!(by_reviewer(json!({ "filter": "mine" })), Vec::<u64>::new());
    assert_eq!(by_reviewer(json!({ "filter": "all" })), OPEN);
    assert_eq!(list(json!({ "state": "merged" })), [MERGED]);
    assert_eq!(list(json!({ "state": "closed" })), [CLOSED]);
    assert_eq!(list(json!({ "query": "label:bug" })), [TOPIC]);
    assert_eq!(list(json!({ "query": "label:bug,ui" })), [TOPIC, DRAFT]);
    assert_eq!(list(json!({ "query": "draft:true" })), [DRAFT]);
    assert!(!list(json!({ "query": "-label:bug" })).contains(&TOPIC));
    assert_eq!(list(json!({ "query": "base:bb-stack-1" })), [STACK[1]]);
    assert_eq!(
        list(json!({ "query": "status:failure label:bug" })),
        [TOPIC]
    );
    assert_eq!(list(json!({ "query": "milestone:v1" })), [TOPIC]);
    assert_eq!(list(json!({ "query": "review:approved" })), [TOPIC]);
    assert_eq!(
        list(json!({ "query": "review:changes_requested" })),
        [CHANGES]
    );
    assert_eq!(list(json!({ "query": "review-requested:user-2" })), [DRAFT]);
    assert_eq!(list(json!({ "query": "head:bb-fork-topic" })), [FORK]);
    h.all_used();
}

#[test]
fn github_org_a_pull_request_is_shown_with_its_check_runs_verdicts_and_conversations() {
    let h = Harness::new("pr_show.json", 1);
    let topic = h.on(MAIN, "pr.show", json!({ "number": TOPIC }));
    assert_eq!(topic["pull"]["number"], TOPIC);
    assert_eq!(
        topic["body"],
        "Made by the live test: Add a file to review."
    );
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["milestone"]["due"], "2026-12-30T00:00:00Z");
    assert_eq!(topic["merge_methods"], json!(["merge", "squash", "rebase"]));
    assert_eq!(topic["pull"]["ci"], "failure");
    // The check runs of GitHub Actions, and the statuses beside them.
    let checks: Vec<(&str, &str)> = topic["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| {
            (
                check["name"].as_str().unwrap(),
                check["state"].as_str().unwrap(),
            )
        })
        .collect();
    for check in [
        ("passes", "success"),
        ("fails", "failure"),
        ("skipped", "skipped"),
        ("bb-lint", "success"),
        ("bb-test", "failure"),
    ] {
        assert!(checks.contains(&check), "{check:?} in {checks:?}");
    }
    let run = topic["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "fails")
        .unwrap();
    assert!(
        run["url"]
            .as_str()
            .unwrap()
            .starts_with("https://github.com/org-1/bitbraid-live/actions/runs/"),
        "{run}"
    );
    assert!(run["started"].is_string() && run["finished"].is_string());
    let file = &topic["files"][0];
    assert_eq!(file["path"], "bb-topic.txt");
    assert_eq!(file["status"], "added");
    assert_eq!(file["additions"], 2);
    let threads = topic["threads"].as_array().unwrap();
    let on_lines: Vec<&Value> = threads
        .iter()
        .filter(|thread| thread["path"] == "bb-topic.txt")
        .collect();
    assert_eq!(on_lines.len(), 2);
    assert_eq!(on_lines[0]["line"], 1);
    assert_eq!(on_lines[0]["resolved"], true);
    let said: Vec<&str> = on_lines[0]["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| comment["body"].as_str().unwrap())
        .collect();
    assert_eq!(said, ["This line could say more.", "Done."]);
    assert_eq!(on_lines[1]["line"], 2);
    assert_eq!(on_lines[1]["resolved"], false);
    let whole = threads
        .iter()
        .find(|thread| thread["path"].is_null())
        .unwrap();
    let said: Vec<(&str, &str)> = whole["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| {
            (
                comment["author"]["login"].as_str().unwrap(),
                comment["body"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        said,
        [
            ("user-1", "Looks good to me."),
            ("user-2", "A second look: fine.")
        ]
    );
    // The reviewer's verdict beside the author's own comments.
    let reviews: Vec<(&str, &str)> = topic["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|review| {
            (
                review["author"]["login"].as_str().unwrap(),
                review["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(reviews.contains(&("user-2", "approved")), "{reviews:?}");
    assert!(reviews.contains(&("user-1", "commented")), "{reviews:?}");
    let verdicts: Vec<(&str, &str)> = topic["pull"]["reviewers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reviewer| {
            (
                reviewer["user"]["login"].as_str().unwrap(),
                reviewer["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(verdicts.contains(&("user-2", "approved")), "{verdicts:?}");
    let changes = h.on(MAIN, "pr.show", json!({ "number": CHANGES }));
    assert_eq!(
        changes["pull"]["reviewers"],
        json!([{ "user": changes["pull"]["reviewers"][0]["user"], "state": "changes_requested" }])
    );
    assert_eq!(changes["reviews"][0]["body"], "Please add a test.");
    assert_eq!(changes["pull"]["ci"], "success");
    let draft = h.on(MAIN, "pr.show", json!({ "number": DRAFT }));
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["pull"]["reviewers"][0]["state"], "pending");
    assert!(draft["threads"].as_array().unwrap().is_empty());
    let second = h.on(MAIN, "pr.show", json!({ "number": STACK[1] }));
    assert_eq!(second["pull"]["base_ref"], "bb-stack-1");
    assert_eq!(second["files"][0]["path"], "bb-stack-2.txt");
    let fork = h.on(MAIN, "pr.show", json!({ "number": FORK }));
    assert_eq!(fork["pull"]["head_repo"], "user-1/bitbraid-live");
    assert_eq!(fork["files"][0]["path"], "bb-fork.txt");
    assert_eq!(fork["pull"]["ci"], "success");
    h.all_used();
}

#[test]
fn github_org_choices_stacks_and_the_pull_request_of_a_branch() {
    let h = Harness::new("pr_options_stack.json", 1);
    let options = h.on(MAIN, "pr.options", json!({}));
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
    assert_eq!(sorted_text(people), ["user-1", "user-2"]);
    assert_eq!(
        options["merge_methods"],
        json!(["merge", "squash", "rebase"])
    );
    let stack = h.on(MAIN, "pr.stack", json!({ "number": STACK[1] }));
    let stacked: Vec<(u64, &str)> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["number"].as_u64().unwrap(),
                entry["base_ref"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(stacked, [(STACK[0], "main"), (STACK[1], "bb-stack-1")]);
    assert_eq!(stack["position"], 1);
    let branch = h.on(MAIN, "pr.for_branch", json!({ "branch": "bb-stack-1" }));
    assert_eq!(branch["number"], STACK[0]);
    for other in ["bb-fork-topic", "bb-no-such-branch"] {
        assert_eq!(
            h.on(MAIN, "pr.for_branch", json!({ "branch": other })),
            Value::Null
        );
    }
    h.all_used();
}

fn sorted_text(mut texts: Vec<&str>) -> Vec<&str> {
    texts.sort_unstable();
    texts
}

#[test]
fn github_org_issues_are_listed_by_every_filter_and_shown() {
    let h = Harness::new("issues.json", 1);
    let list = |extra: Value| sorted(numbers(&h.on(MAIN, "issue.list", extra)));
    assert_eq!(list(json!({})), [ISSUE, PLAIN_ISSUE]);
    assert_eq!(list(json!({ "state": "closed" })), [CLOSED_ISSUE]);
    assert_eq!(
        list(json!({ "state": "all" })),
        [ISSUE, CLOSED_ISSUE, PLAIN_ISSUE]
    );
    assert_eq!(list(json!({ "filter": "assigned" })), [ISSUE]);
    assert_eq!(list(json!({ "filter": "created" })), [ISSUE, PLAIN_ISSUE]);
    assert_eq!(list(json!({ "filter": "mentioned" })), Vec::<u64>::new());
    assert_eq!(list(json!({ "query": "labels:bug" })), [ISSUE]);
    assert_eq!(list(json!({ "query": "milestone:v1 button" })), [ISSUE]);
    assert_eq!(
        list(json!({ "query": "nothing-like-this" })),
        Vec::<u64>::new()
    );
    let shown = h.on(MAIN, "issue.show", json!({ "number": ISSUE }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["issue"]["state"], "open");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], "user-1");
    assert_eq!(shown["issue"]["comments"], 2);
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    let comments: Vec<(&str, &str)> = shown["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| {
            (
                comment["author"]["login"].as_str().unwrap(),
                comment["body"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        comments,
        [
            ("user-1", "Which button do you mean?"),
            ("user-2", "The one at the top.")
        ]
    );
    let err = h
        .try_run(
            "issue.show",
            json!({ "account": MAIN, "owner": "org-1", "name": "bitbraid-live", "number": TOPIC }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("pull request"), "{err}");
    h.all_used();
}

#[test]
fn github_org_the_check_runs_statuses_and_authors_pictures_of_commits_are_read() {
    let h = Harness::new("ci_status_avatars.json", 1);
    // The heads of bb-topic, bb-draft and bb-stack-1, as recorded, and a
    // commit GitHub does not know.
    let text = std::fs::read_to_string(recording("ci_status_avatars.json")).unwrap();
    let recorded: Recording = serde_json::from_str(&text).unwrap();
    let asked = recorded
        .exchanges
        .iter()
        .find(|exchange| exchange.path_and_query == "/graphql")
        .unwrap();
    let shas: Vec<String> = (0..4)
        .map(|at| {
            asked.request_body["variables"][format!("c{at}")]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(shas[3], "0123456789abcdef0123456789abcdef01234567");
    let statuses = h.on(MAIN, "ci.status", json!({ "shas": shas }));
    let states: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["failure", "pending", "failure", "none"]);
    let topic = statuses[0]["checks"].as_array().unwrap();
    assert_eq!(topic.len(), 5);
    let lint = topic
        .iter()
        .find(|check| check["name"] == "bb-lint")
        .unwrap();
    assert_eq!(lint["state"], "success");
    assert_eq!(lint["description"], "bb-lint says success");
    assert_eq!(lint["url"], "https://example.invalid/bb-lint");
    let skipped = topic
        .iter()
        .find(|check| check["name"] == "skipped")
        .unwrap();
    assert_eq!(skipped["state"], "skipped");
    let draft = statuses[1]["checks"].as_array().unwrap();
    assert!(
        draft
            .iter()
            .any(|check| check["name"] == "bb-test" && check["state"] == "pending")
    );
    assert!(
        draft
            .iter()
            .any(|check| check["name"] == "passes" && check["state"] == "success")
    );
    // The test's identity (user-3 here) has no account; the address of
    // the machine account's own commits is learnt from the commits, with
    // its picture.
    let pictures = h.on(
        MAIN,
        "provider.avatars",
        json!({ "emails": ["user-3@example.invalid", "user-4@example.invalid"] }),
    );
    assert_eq!(
        pictures,
        json!({ "user-4@example.invalid": "https://avatars.example.invalid/user-1" })
    );
    // An address GitHub made for an account needs no question.
    let own = "100001+user-1@users.noreply.github.com";
    let asked = h.replay.asked().len();
    assert_eq!(
        h.on(MAIN, "provider.avatars", json!({ "emails": [own] })),
        json!({ own: "https://avatars.githubusercontent.com/u/100001?v=4" })
    );
    assert_eq!(h.replay.asked().len(), asked);
    h.all_used();
}

#[test]
fn github_org_repositories_and_where_one_may_be_made_are_listed() {
    let h = Harness::new("repos.json", 1);
    let names = |extra: Value| {
        let mut params = json!({ "account": MAIN });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let listing = h.run("provider.repos", params);
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(json!({ "filter": "own" })), ["user-1/bitbraid-live"]);
    let organisation = names(json!({ "filter": "organisation" }));
    for name in ["org-1/bitbraid-live", "org-1/bitbraid-live-writes"] {
        assert!(organisation.contains(&name.to_owned()), "{organisation:?}");
    }
    assert_eq!(
        names(json!({ "filter": "starred" })),
        ["org-1/bitbraid-live"]
    );
    let everything = names(json!({ "filter": "all" }));
    assert!(everything.contains(&"user-1/bitbraid-live".to_owned()));
    assert_eq!(
        names(json!({ "filter": "all", "query": "writes" })),
        ["org-1/bitbraid-live-writes"]
    );
    let listing = h.run(
        "provider.repos",
        json!({ "account": MAIN, "filter": "own" }),
    );
    let fork = &listing["items"][0];
    assert_eq!(fork["owner"], "user-1");
    assert_eq!(fork["private"], false);
    assert_eq!(
        fork["clone_https"],
        "https://github.com/user-1/bitbraid-live.git"
    );
    assert_eq!(fork["clone_ssh"], "git@github.com:user-1/bitbraid-live.git");
    assert_eq!(fork["web_url"], "https://github.com/user-1/bitbraid-live");
    assert_eq!(listing["more"], false);
    let namespaces = h.run("provider.namespaces", json!({ "account": MAIN }));
    let places: Vec<(&str, &str)> = namespaces
        .as_array()
        .unwrap()
        .iter()
        .map(|place| {
            (
                place["path"].as_str().unwrap(),
                place["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(places, [("user-1", "user"), ("org-1", "organisation")]);
    assert_eq!(
        namespaces[1]["avatar_url"],
        "https://avatars.example.invalid/org-1"
    );
    h.all_used();
}

#[test]
fn github_org_signing_in_by_code_asks_github_com_and_waits() {
    let name = "device_flow.json";
    let h = Harness::new(name, 0);
    let code = h.run("account.sign_in_start", json!({ "kind": "github" }));
    // The codes were scrubbed when recorded.
    assert_eq!(code["user_code"], "user-code-1");
    assert_eq!(code["device_code"], "device-code-1");
    assert_eq!(code["verification_uri"], "https://github.com/login/device");
    assert!(code["interval"].as_u64().unwrap() >= 5);
    // The application built in, asking for what the commands need.
    let text = std::fs::read_to_string(recording(name)).unwrap();
    let recorded: Recording = serde_json::from_str(&text).unwrap();
    let start = &recorded.exchanges[0];
    assert_eq!(start.path_and_query, "/login/device/code");
    let form = start.request_body.as_str().unwrap();
    assert!(
        form.split('&')
            .any(|pair| pair == format!("client_id={GITHUB_CLIENT_ID}")),
        "{form}"
    );
    assert!(form.contains("scope=repo"), "{form}");
    let polled = h.run(
        "account.sign_in_poll",
        json!({ "kind": "github", "device_code": code["device_code"],
                "interval": code["interval"] }),
    );
    assert_eq!(polled, json!({ "state": "waiting" }));
    h.all_used();
}

#[test]
fn github_org_the_inbox_gathers_the_machine_accounts_pull_requests_and_issues() {
    let h = Harness::new("inbox.json", 1);
    let found = h.run("inbox.items", json!({ "wip": false }));
    // The groups of these pull requests: failing checks keep one out of
    // those that can be merged, a fork's that wait for leave to run do
    // not, a draft waits for review all the same.
    let pull = |repo: &str, number: u64, groups: Value, why: Value| {
        (
            format!("{INBOX_ACCOUNT}/pull/org-1/{repo}#{number}"),
            groups,
            why,
        )
    };
    let issue = |number: u64, why: Value| {
        (
            format!("{INBOX_ACCOUNT}/issue/org-1/bitbraid-live#{number}"),
            Value::Null,
            why,
        )
    };
    assert_eq!(
        summary(&found),
        [
            pull(
                "bitbraid-live",
                3,
                json!(["waiting_for_review", "draft"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live",
                1,
                json!(["failing_checks"]),
                json!(["created", "assigned"])
            ),
            pull(
                "bitbraid-live-writes",
                30,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live-writes",
                29,
                json!(["no_reviewers", "conflicts"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live-writes",
                26,
                json!(["changes_requested"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live",
                4,
                json!(["no_reviewers", "failing_checks"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live",
                8,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live",
                2,
                json!(["changes_requested"]),
                json!(["created"])
            ),
            pull(
                "bitbraid-live",
                5,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            issue(11, json!(["created"])),
            issue(9, json!(["assigned", "created"])),
        ]
    );
    let fork = &found["pulls"][6];
    assert_eq!(fork["pull"]["head_repo"], "user-1/bitbraid-live");
    assert!(
        found["pulls"][1]["additions"]
            .as_u64()
            .is_some_and(|lines| lines > 0)
    );
    assert_eq!(found["accounts"][0]["error"], Value::Null);
    h.all_used();
}

/// The machine account, as the inbox's keys name it.
const INBOX_ACCOUNT: &str = MAIN;

/// What a test of the inbox looks at: each row's key, groups and why.
fn summary(found: &Value) -> Vec<(String, Value, Value)> {
    found["pulls"]
        .as_array()
        .unwrap()
        .iter()
        .chain(found["issues"].as_array().unwrap())
        .map(|row| {
            (
                row["key"].as_str().unwrap().to_owned(),
                row["groups"].clone(),
                row["involvement"].clone(),
            )
        })
        .collect()
}
