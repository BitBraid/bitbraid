//! The provider commands against answers recorded from gitlab.com with
//! the test account (`tests/recordings/gitlab-com/`, made by
//! `tests/suite/live_gitlab_com.rs` with `BITBRAID_RECORD`, scrubbed):
//! the account is `user-1`; the reads' project is `user-1/bitbraid-live`,
//! public, with merge requests in every state, conversations on lines,
//! the account's own verdicts, statuses of commits set through the API, a
//! merge request from `user-1/bitbraid-live-fork` (a fork in the same
//! namespace), and issues; the writes are made in `user-1/bitbraid-live-writes`
//! and `user-1/bitbraid-live-ff`, which merges only by fast-forward. What the
//! commands make of the answers, and every request they make (with the
//! body sent then), is held here, in ci, without a network.

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
    oauth::GITLAB_CLIENT_ID,
    replay::{Recording, Replay},
    secrets::MemoryStore,
};
use bitbraid_testkit::Git;
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The test account, as the recordings name it.
const ACCOUNT: &str = "gitlab:gitlab.com:user-1";

fn recording(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recordings/gitlab-com")
        .join(name)
}

struct Harness {
    dir: PathBuf,
    replay: Arc<Replay>,
    services: Arc<Services>,
}

impl Harness {
    /// A recording, with the account connected as the live test did,
    /// unless it was a sign-in by code.
    fn new(name: &str, connect: bool) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-l-recorded-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = Arc::new(Replay::load(&recording(name)).unwrap());
        let services = Services::new(replay.clone(), Arc::new(MemoryStore::new()), &dir);
        let h = Self {
            dir,
            replay,
            services: Arc::new(services),
        };
        if connect {
            let account = h.run(
                "account.add_token",
                json!({ "kind": "gitlab", "token": "glpat-replayed" }),
            );
            assert_eq!(account["id"], ACCOUNT);
        }
        h
    }

    fn git(&self, dir: &Path) -> Git {
        let git = Git::new(dir, self.dir.join("home"));
        git.ensure_home().unwrap();
        git
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

    /// A command with the repository at `repo` open.
    fn run_in(&self, repo: &Path, id: &str, params: Value) -> Result<Value, String> {
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
            .unwrap();
        registry
            .run(id, &mut ctx, &params)
            .map_err(|err| err.to_string())
    }

    /// A command about one of the account's projects.
    fn on(&self, name: &str, id: &str, extra: Value) -> Result<Value, String> {
        let mut params = json!({ "account": ACCOUNT, "owner": "user-1", "name": name });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.try_run(id, params).map_err(|err| err.to_string())
    }

    /// A command about the reads' project.
    fn reads(&self, id: &str, extra: Value) -> Value {
        self.on("bitbraid-live", id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
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

/// The merge requests of the reads' project: `bb-topic`, `bb-changes`,
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
const ISSUE: u64 = 1;
const CLOSED_ISSUE: u64 = 2;
const PLAIN_ISSUE: u64 = 3;
/// The heads of `bb-topic`, the draft and the first of the stack.
const SHAS: [&str; 3] = [
    "c12f078ee5a3857a636a969a66eef60436473833",
    "5f780a5f633dabf5d2f076e99d56b8ae136f593a",
    "91a5bf1e167d364597fcd73ed7b4c783f0cb13af",
];

#[test]
fn every_recording_of_the_test_account_was_recorded_live() {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(recording("")).unwrap() {
        let path = entry.unwrap().path();
        let text = std::fs::read_to_string(&path).unwrap();
        let recorded: Recording = serde_json::from_str(&text).unwrap();
        assert!(recorded.hand_made.is_none(), "{}", path.display());
        assert!(
            recorded.note.starts_with("Recorded from gitlab.com"),
            "{}",
            path.display()
        );
        names.push(path.file_stem().unwrap().to_string_lossy().into_owned());
    }
    names.sort();
    assert_eq!(
        names,
        [
            "add_token_and_pr_list",
            "ci_status_avatars",
            "device_flow",
            "inbox",
            "issue_branch",
            "issues",
            "pr_checkout",
            "pr_list",
            "pr_merge_stack",
            "pr_options_stack",
            "pr_show",
            "pr_suggestions",
            "pr_writes",
            "repos",
            "repos_forks_keys",
        ]
    );
}

#[test]
fn gitlab_com_a_token_connects_and_the_merge_requests_are_listed() {
    let h = Harness::new("add_token_and_pr_list.json", true);
    let accounts = h.run("account.list", json!({}));
    assert_eq!(accounts[0]["web"], "https://gitlab.com");
    assert_eq!(accounts[0]["api"], "https://gitlab.com/api/v4");
    assert_eq!(accounts[0]["login"], "user-1");
    assert_eq!(accounts[0]["name"], "User 1");
    assert_eq!(accounts[0]["scopes"], json!(["api", "read_user"]));
    assert_eq!(accounts[0]["sign_in"], "token");
    assert_eq!(
        accounts[0]["avatar_url"],
        "https://avatars.example.invalid/user-1"
    );
    let listing = h.reads("pr.list", json!({ "state": "all" }));
    assert_eq!(
        sorted(numbers(&listing)),
        sorted([&OPEN[..], &[MERGED, CLOSED]].concat())
    );
    let topic = listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pull| pull["number"] == TOPIC)
        .unwrap();
    assert_eq!(topic["head_ref"], "bb-topic");
    assert_eq!(topic["author"]["login"], "user-1");
    assert_eq!(topic["repo"], "user-1/bitbraid-live");
    assert_eq!(
        topic["web_url"],
        "https://gitlab.com/user-1/bitbraid-live/-/merge_requests/1"
    );
    h.all_used();
}

#[test]
fn gitlab_com_merge_requests_are_listed_by_every_filter_and_query() {
    let h = Harness::new("pr_list.json", true);
    let all = h.reads("pr.list", json!({ "state": "all" }));
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
    assert_eq!(topic["labels"][0]["name"], "bug");
    assert_eq!(topic["labels"][0]["color"], "d9534f");
    assert_eq!(topic["assignees"][0]["login"], "user-1");
    assert_eq!(topic["reviewers"][0]["user"]["login"], "user-1");
    assert_eq!(topic["reviewers"][0]["state"], "approved");
    assert_eq!(topic["ci"], "failure");
    assert!(topic["comments"].as_u64().unwrap() >= 4, "{topic}");
    assert_eq!(row(CHANGES)["reviewers"][0]["state"], "changes_requested");
    assert_eq!(row(CHANGES)["ci"], "none");
    let draft = row(DRAFT);
    assert_eq!(draft["draft"], true);
    assert_eq!(draft["ci"], "pending");
    assert_eq!(draft["reviewers"][0]["state"], "pending");
    assert_eq!(row(STACK[0])["ci"], "running");
    assert_eq!(row(STACK[1])["base_ref"], "bb-stack-1");
    assert_eq!(row(MERGED)["state"], "merged");
    assert_eq!(row(CLOSED)["state"], "closed");
    let fork = row(FORK);
    assert_eq!(fork["head_repo"], "user-1/bitbraid-live-fork");
    assert_eq!(fork["head_ref"], "bb-fork-topic");
    let list = |extra: Value| sorted(numbers(&h.reads("pr.list", extra)));
    let open = sorted(OPEN.to_vec());
    assert_eq!(list(json!({})), open);
    assert_eq!(list(json!({ "filter": "mine" })), open);
    assert_eq!(list(json!({ "filter": "assigned" })), [TOPIC]);
    assert_eq!(
        list(json!({ "filter": "review_requested" })),
        [TOPIC, CHANGES, DRAFT]
    );
    assert_eq!(list(json!({ "state": "merged" })), [MERGED]);
    assert_eq!(list(json!({ "state": "closed" })), [CLOSED]);
    assert_eq!(list(json!({ "query": "label:bug" })), [TOPIC]);
    assert_eq!(list(json!({ "query": "label:bug,ui" })), [TOPIC, DRAFT]);
    assert_eq!(list(json!({ "query": "draft:true" })), [DRAFT]);
    assert!(!list(json!({ "query": "-label:bug" })).contains(&TOPIC));
    assert_eq!(list(json!({ "query": "base:bb-stack-1" })), [STACK[1]]);
    assert_eq!(list(json!({ "query": "milestone:v1" })), [TOPIC]);
    assert_eq!(list(json!({ "query": "head:bb-fork-topic" })), [FORK]);
    assert_eq!(list(json!({ "query": "author:user-1" })), open);
    assert_eq!(
        list(json!({ "query": "review-requested:user-1 label:ui" })),
        [DRAFT]
    );
    assert_eq!(list(json!({ "query": "stack" })), STACK.to_vec());
    // Refused before anything is asked.
    let err = h
        .on("bitbraid-live", "pr.list", json!({ "query": "involves:x" }))
        .unwrap_err();
    assert!(err.contains("involves"), "{err}");
    h.all_used();
}

#[test]
fn gitlab_com_a_merge_request_is_shown_with_everything_its_view_shows() {
    let h = Harness::new("pr_show.json", true);
    let show = |number: u64| h.reads("pr.show", json!({ "number": number }));
    let topic = show(TOPIC);
    assert_eq!(topic["pull"]["number"], TOPIC);
    assert_eq!(
        topic["body"],
        "Made by the live test: Add a file to review."
    );
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["pull"]["ci"], "failure");
    // The author approved it, asked to review it themselves.
    assert_eq!(topic["pull"]["reviewers"][0]["user"]["login"], "user-1");
    assert_eq!(topic["pull"]["reviewers"][0]["state"], "approved");
    assert!(
        topic["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .any(|review| review["state"] == "approved" && review["author"]["login"] == "user-1"),
        "{}",
        topic["reviews"]
    );
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
    for check in [("bb-lint", "success"), ("bb-test", "failure")] {
        assert!(checks.contains(&check), "{check:?} in {checks:?}");
    }
    let commits = topic["commits"].as_array().unwrap();
    assert_eq!(commits.len(), 1, "{commits:?}");
    assert_eq!(commits[0]["summary"], "Write bb-topic.txt");
    // The test's identity, scrubbed like anyone who writes a commit.
    assert_eq!(commits[0]["author_name"], "User 2");
    let file = &topic["files"][0];
    assert_eq!(file["path"], "bb-topic.txt");
    assert_eq!(file["status"], "added");
    assert_eq!(file["additions"], 2);
    assert!(
        file["patch"]
            .as_str()
            .unwrap()
            .contains("+a line of bb-topic.txt")
    );
    let threads = topic["threads"].as_array().unwrap();
    let on_lines: Vec<&Value> = threads
        .iter()
        .filter(|thread| thread["path"] == "bb-topic.txt")
        .collect();
    assert_eq!(on_lines.len(), 2, "{threads:?}");
    assert_eq!(on_lines[0]["line"], 1);
    assert_eq!(on_lines[0]["resolved"], true);
    assert_eq!(on_lines[0]["comments"].as_array().unwrap().len(), 2);
    assert_eq!(on_lines[0]["comments"][1]["body"], "Done.");
    assert_eq!(on_lines[1]["line"], 2);
    assert_eq!(on_lines[1]["resolved"], false);
    assert!(
        on_lines[1]["diff_hunk"]
            .as_str()
            .is_none_or(|hunk| !hunk.is_empty())
    );
    assert!(
        threads.iter().any(|thread| thread["path"].is_null()
            && thread["comments"][0]["body"] == "Looks good to me."),
        "{threads:?}"
    );
    assert_eq!(topic["merge_methods"], json!(["merge", "squash"]));
    assert_ne!(topic["mergeable"], "conflicts");
    let changes = show(CHANGES);
    assert_eq!(
        changes["pull"]["reviewers"][0]["state"],
        "changes_requested"
    );
    let draft = show(DRAFT);
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["pull"]["reviewers"][0]["state"], "pending");
    assert_eq!(draft["mergeable"], "blocked");
    assert_eq!(show(STACK[1])["pull"]["base_ref"], "bb-stack-1");
    let fork = show(FORK);
    assert_eq!(fork["pull"]["head_repo"], "user-1/bitbraid-live-fork");
    assert_eq!(fork["files"][0]["path"], "bb-fork.txt");
    h.all_used();
}

#[test]
fn gitlab_com_choices_stacks_and_the_merge_request_of_a_branch() {
    let h = Harness::new("pr_options_stack.json", true);
    let options = h.reads("pr.options", json!({}));
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
    assert_eq!(options["milestones"][0]["due"], "2026-12-31");
    assert_eq!(options["people"][0]["login"], "user-1");
    assert_eq!(options["people"].as_array().unwrap().len(), 1);
    assert_eq!(options["merge_methods"], json!(["merge", "squash"]));
    let stack = h.reads("pr.stack", json!({ "number": STACK[1] }));
    let stacked: Vec<u64> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["number"].as_u64().unwrap())
        .collect();
    assert_eq!(stacked, STACK);
    assert_eq!(stack["position"], 1);
    assert_eq!(
        h.reads("pr.stack", json!({ "number": STACK[0] }))["position"],
        0
    );
    let branch = h.reads("pr.for_branch", json!({ "branch": "bb-stack-1" }));
    assert_eq!(branch["number"], STACK[0]);
    for other in ["bb-fork-topic", "bb-no-such-branch"] {
        assert_eq!(
            h.reads("pr.for_branch", json!({ "branch": other })),
            Value::Null
        );
    }
    h.all_used();
}

#[test]
fn gitlab_com_issues_are_listed_by_every_filter_and_shown() {
    let h = Harness::new("issues.json", true);
    let list = |extra: Value| sorted(numbers(&h.reads("issue.list", extra)));
    assert_eq!(list(json!({})), [ISSUE, PLAIN_ISSUE]);
    assert_eq!(list(json!({ "state": "closed" })), [CLOSED_ISSUE]);
    assert_eq!(
        list(json!({ "state": "all" })),
        [ISSUE, CLOSED_ISSUE, PLAIN_ISSUE]
    );
    assert_eq!(list(json!({ "filter": "assigned" })), [ISSUE]);
    assert_eq!(list(json!({ "filter": "created" })), [ISSUE, PLAIN_ISSUE]);
    // A comment naming the account made it a to-do item.
    assert_eq!(list(json!({ "filter": "mentioned" })), [ISSUE]);
    assert_eq!(list(json!({ "query": "#bug" })), [ISSUE]);
    assert_eq!(list(json!({ "query": "label:bug" })), [ISSUE]);
    assert_eq!(list(json!({ "query": "$v1 button" })), [ISSUE]);
    assert_eq!(list(json!({ "query": "@user-1" })), [ISSUE]);
    assert_eq!(
        list(json!({ "query": "-label:bug", "state": "all" })),
        [CLOSED_ISSUE, PLAIN_ISSUE]
    );
    assert_eq!(
        list(json!({ "query": "nothing-like-this" })),
        Vec::<u64>::new()
    );
    let shown = h.reads("issue.show", json!({ "number": ISSUE }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], "user-1");
    assert_eq!(shown["issue"]["comments"], 2);
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
            ("user-1", "@user-1 the one at the top.")
        ]
    );
    let closed = h.reads("issue.show", json!({ "number": CLOSED_ISSUE }));
    assert_eq!(closed["issue"]["state"], "closed");
    assert_eq!(closed["comments"], json!([]));
    h.all_used();
}

#[test]
fn gitlab_com_the_statuses_of_commits_and_pictures_are_read() {
    let h = Harness::new("ci_status_avatars.json", true);
    let unknown = "0123456789abcdef0123456789abcdef01234567";
    let mut shas = SHAS.to_vec();
    shas.push(unknown);
    let statuses = h.reads("ci.status", json!({ "shas": shas }));
    let states: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["failure", "pending", "running", "none"]);
    assert_eq!(statuses[0]["sha"], SHAS[0]);
    let checks = statuses[0]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2, "{checks:?}");
    let test = checks
        .iter()
        .find(|check| check["name"] == "bb-test")
        .unwrap();
    assert_eq!(test["state"], "failure");
    assert_eq!(test["url"], "https://example.invalid/bb-test");
    assert_eq!(test["description"], "bb-test says failed");
    assert!(test["started"].is_string() || test["finished"].is_string());
    assert_eq!(statuses[3]["sha"], unknown);
    assert_eq!(statuses[3]["checks"], json!([]));
    let pictures = h.reads(
        "provider.avatars",
        json!({ "emails": ["bitbraid-test@example.invalid"] }),
    );
    assert!(
        pictures["bitbraid-test@example.invalid"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://")),
        "{pictures}"
    );
    h.all_used();
}

#[test]
fn gitlab_com_projects_and_where_one_may_be_made_are_listed() {
    let h = Harness::new("repos.json", true);
    let names = |filter: &str, query: Option<&str>| {
        let listing = h.run(
            "provider.repos",
            json!({ "account": ACCOUNT, "filter": filter, "query": query }),
        );
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let own = names("own", None);
    for name in ["user-1/bitbraid-live", "user-1/bitbraid-live-fork"] {
        assert!(own.iter().any(|known| known == name), "{own:?}");
    }
    assert!(own.iter().all(|name| name.starts_with("user-1/")));
    assert_eq!(names("organisation", None), Vec::<String>::new());
    assert_eq!(names("starred", None), ["user-1/bitbraid-live"]);
    let all = names("all", None);
    assert!(all.iter().any(|known| known == "user-1/bitbraid-live-fork"));
    assert_eq!(
        names("all", Some("bitbraid-live-fork")),
        ["user-1/bitbraid-live-fork"]
    );
    let listing = h.run(
        "provider.repos",
        json!({ "account": ACCOUNT, "filter": "own" }),
    );
    let item = |name: &str| {
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|repo| repo["full_name"] == name)
            .unwrap()
            .clone()
    };
    let project = item("user-1/bitbraid-live");
    assert_eq!(project["private"], false);
    assert_eq!(project["default_branch"], "main");
    assert_eq!(
        project["clone_https"],
        "https://gitlab.com/user-1/bitbraid-live.git"
    );
    assert_eq!(
        project["clone_ssh"],
        "git@gitlab.com:user-1/bitbraid-live.git"
    );
    assert_eq!(
        item("user-1/bitbraid-live-fork")["parent"],
        "user-1/bitbraid-live"
    );
    let namespaces = h.run("provider.namespaces", json!({ "account": ACCOUNT }));
    assert_eq!(
        namespaces,
        json!([{ "path": "user-1", "name": "User 1", "kind": "user",
                 "avatar_url": "https://avatars.example.invalid/user-1" }])
    );
    h.all_used();
}

#[test]
fn gitlab_com_signing_in_by_code_asks_for_the_api_with_the_built_in_application() {
    let h = Harness::new("device_flow.json", false);
    let code = h.run("account.sign_in_start", json!({ "kind": "gitlab" }));
    assert_eq!(code["user_code"], "user-code-1");
    assert_eq!(code["device_code"], "device-code-1");
    assert_eq!(code["verification_uri"], "https://gitlab.com/oauth/device");
    let polled = h.run(
        "account.sign_in_poll",
        json!({ "kind": "gitlab", "device_code": code["device_code"] }),
    );
    assert_eq!(polled, json!({ "state": "waiting" }));
    // What was sent: the built-in application, and the scope it was
    // registered for.
    let text = std::fs::read_to_string(recording("device_flow.json")).unwrap();
    let recorded: Recording = serde_json::from_str(&text).unwrap();
    assert_eq!(
        recorded.exchanges[0].request_body,
        format!("client_id={GITLAB_CLIENT_ID}&scope=api")
    );
    h.all_used();
}

#[test]
fn gitlab_com_merge_requests_are_made_changed_reviewed_commented_on_and_merged() {
    let h = Harness::new("pr_writes.json", true);
    let write = |id: &str, extra: Value| {
        h.on("bitbraid-live-writes", id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };
    let made = write(
        "pr.create",
        json!({ "head": "bb-w-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true,
                "reviewers": ["user-1"], "assignees": ["user-1"],
                "labels": ["bug"], "milestone": "300176" }),
    );
    let number = made["number"].as_u64().unwrap();
    assert_eq!(made["title"], "Draft: Change one");
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["reviewers"][0]["user"]["login"], "user-1");
    assert_eq!(made["assignees"][0]["login"], "user-1");
    let err = h
        .on(
            "bitbraid-live-writes",
            "pr.create",
            json!({ "head": "bb-w-alt", "base": "main", "title": "Nobody",
                    "reviewers": ["bitbraid-live-nobody-at-all"] }),
        )
        .unwrap_err();
    assert!(err.contains("no user"), "{err}");
    // The author's own verdicts, asked to review: gitlab.com takes them.
    let approved = write("pr.review", json!({ "number": number, "event": "approve" }));
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], "user-1");
    let changes = write(
        "pr.review",
        json!({ "number": number, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(changes["state"], "changes_requested");
    let shown = write("pr.show", json!({ "number": number }));
    assert_eq!(shown["pull"]["reviewers"][0]["state"], "changes_requested");
    let commented = write(
        "pr.review",
        json!({ "number": number, "event": "comment", "body": "Better now." }),
    );
    assert_eq!(commented["state"], "commented");
    let comment = write(
        "pr.comment",
        json!({ "number": number, "body": "A comment from the live test." }),
    );
    assert_eq!(comment["author"]["login"], "user-1");
    let shown = write("pr.show", json!({ "number": number }));
    let thread = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["comments"][0]["body"] == "A comment from the live test.")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let answer = write(
        "pr.comment",
        json!({ "number": number, "body": "An answer.", "reply_to": thread }),
    );
    assert_eq!(answer["body"], "An answer.");
    let shown = write("pr.show", json!({ "number": number }));
    let answered = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|found| found["id"] == thread.as_str())
        .unwrap()
        .clone();
    assert_eq!(answered["comments"].as_array().unwrap().len(), 2);
    let changed = write(
        "pr.update",
        json!({ "number": number, "title": "Change one, better", "draft": false,
                "labels": ["ui"], "reviewers": [], "assignees": [], "milestone": "",
                "base": "bb-w-alt" }),
    );
    assert_eq!(changed["pull"]["title"], "Change one, better");
    assert_eq!(changed["pull"]["draft"], false);
    assert_eq!(changed["pull"]["labels"][0]["name"], "ui");
    assert_eq!(changed["pull"]["base_ref"], "bb-w-alt");
    assert_eq!(changed["pull"]["assignees"], json!([]));
    assert_eq!(changed["pull"]["reviewers"], json!([]));
    assert_eq!(changed["milestone"], Value::Null);
    let drafted = write(
        "pr.update",
        json!({ "number": number, "draft": true, "base": "main" }),
    );
    assert_eq!(drafted["pull"]["title"], "Draft: Change one, better");
    assert_eq!(drafted["pull"]["base_ref"], "main");
    let ready = write("pr.update", json!({ "number": number, "draft": false }));
    assert_eq!(ready["pull"]["draft"], false);
    let merged = write(
        "pr.merge",
        json!({ "number": number, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert!(merged["sha"].is_string(), "{merged}");
    assert_eq!(merged["branch_deleted"], true);
    let err = h
        .on(
            "bitbraid-live-writes",
            "pr.merge",
            json!({ "number": number, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("merged already"), "{err}");
    h.all_used();
}

#[test]
fn gitlab_com_a_stack_moves_down_when_its_bottom_is_merged_and_a_rebase_comes_first() {
    let h = Harness::new("pr_merge_stack.json", true);
    let write = |project: &str, id: &str, extra: Value| {
        h.on(project, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };
    let first = write(
        "bitbraid-live-writes",
        "pr.create",
        json!({ "head": "bb-w-stack-1", "base": "main", "title": "First of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = write(
        "bitbraid-live-writes",
        "pr.create",
        json!({ "head": "bb-w-stack-2", "base": "bb-w-stack-1", "title": "Second of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let stack = write(
        "bitbraid-live-writes",
        "pr.stack",
        json!({ "number": second }),
    );
    assert_eq!(stack["position"], 1, "{stack}");
    let merged = write(
        "bitbraid-live-writes",
        "pr.merge",
        json!({ "number": first, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "message": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["retargeted"], json!([second]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = write(
        "bitbraid-live-writes",
        "pr.for_branch",
        json!({ "branch": "bb-w-stack-2" }),
    );
    assert_eq!(moved["base_ref"], "main");
    assert_eq!(moved["state"], "open");
    let behind = write(
        "bitbraid-live-ff",
        "pr.create",
        json!({ "head": "bb-w-behind", "base": "main", "title": "Behind main" }),
    )["number"]
        .as_u64()
        .unwrap();
    let err = h
        .on(
            "bitbraid-live-ff",
            "pr.merge",
            json!({ "number": behind, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("allows rebase"), "{err}");
    let rebased = write(
        "bitbraid-live-ff",
        "pr.merge",
        json!({ "number": behind, "method": "rebase", "delete_branch": true }),
    );
    assert_eq!(rebased["merged"], true, "{rebased}");
    assert_eq!(rebased["branch_deleted"], true);
    h.all_used();
}

/// What a repository has that checking out could change: its branches
/// and remote branches, where they go, what HEAD is, which branch follows
/// what, its remotes, and its index.
fn state(git: &Git, root: &Path) -> Vec<String> {
    let mut out = git
        .lines([
            "for-each-ref",
            "--format=%(refname) %(objectname) %(upstream)",
            "refs/heads",
            "refs/remotes",
        ])
        .unwrap();
    out.push(format!(
        "HEAD {}",
        git.run(["symbolic-ref", "-q", "HEAD"])
            .unwrap_or_default()
            .trim()
    ));
    out.push(format!("tree {}", git.run(["write-tree"]).unwrap().trim()));
    for line in git
        .lines(["config", "--get-regexp", "^(remote|branch)\\."])
        .unwrap_or_default()
    {
        out.push(line.replace(root.to_str().unwrap(), "<root>"));
    }
    out.sort();
    out
}

#[test]
fn gitlab_com_a_merge_request_from_a_fork_is_checked_out_as_git_would() {
    let h = Harness::new("pr_checkout.json", true);
    // The project and its fork as bare repositories here, at the
    // addresses gitlab.com gave: `refs/merge-requests/8/head` is the
    // fork's branch.
    let root = h.dir.join("world");
    std::fs::create_dir_all(root.join("work")).unwrap();
    let work = h.git(&root.join("work"));
    work.run(["init", "-q", "-b", "main"]).unwrap();
    std::fs::write(root.join("work/README.md"), "bitbraid-live\n").unwrap();
    work.run(["add", "README.md"]).unwrap();
    work.run(["commit", "-q", "-m", "Initial commit"]).unwrap();
    let here = h.git(&root);
    here.run(["clone", "-q", "--bare", "work", "origin.git"])
        .unwrap();
    here.run(["clone", "-q", "--bare", "origin.git", "fork.git"])
        .unwrap();
    work.run(["switch", "-q", "-c", "bb-fork-topic"]).unwrap();
    std::fs::write(root.join("work/bb-fork.txt"), "a line from a fork\n").unwrap();
    work.run(["add", "bb-fork.txt"]).unwrap();
    work.run(["commit", "-q", "-m", "Write bb-fork.txt"])
        .unwrap();
    let fork_path = root.join("fork.git");
    let origin_path = root.join("origin.git");
    work.run(["push", "-q", fork_path.to_str().unwrap(), "bb-fork-topic"])
        .unwrap();
    work.run([
        "push",
        "-q",
        origin_path.to_str().unwrap(),
        "bb-fork-topic:refs/merge-requests/8/head",
    ])
    .unwrap();
    let head = work.rev_parse("bb-fork-topic").unwrap();
    for clone in ["ours", "twin"] {
        here.run(["clone", "-q", "origin.git", clone]).unwrap();
        let git = h.git(&root.join(clone));
        git.run([
            "remote",
            "set-url",
            "origin",
            "https://gitlab.com/user-1/bitbraid-live.git",
        ])
        .unwrap();
        for (path, address) in [
            (&origin_path, "https://gitlab.com/user-1/bitbraid-live.git"),
            (
                &fork_path,
                "https://gitlab.com/user-1/bitbraid-live-fork.git",
            ),
        ] {
            git.run([
                "config",
                &format!("url.{}.insteadOf", path.to_str().unwrap()),
                address,
            ])
            .unwrap();
        }
    }
    let ours = root.join("ours");
    let twin = h.git(&root.join("twin"));
    let checkout = || {
        h.run_in(
            &ours,
            "pr.checkout",
            json!({ "account": ACCOUNT, "owner": "user-1", "name": "bitbraid-live", "number": FORK,
                    "remote": "origin" }),
        )
        .unwrap()
    };
    let out = checkout();
    assert_eq!(out["branch"], "bb-fork-topic");
    assert_eq!(out["commit"], head.as_str());
    assert_eq!(out["created"], true);
    // The fork's owner is the project's: its remote is named after them.
    assert_eq!(out["remote_added"], "user-1");
    assert_eq!(out["upstream"], "user-1/bb-fork-topic");
    for args in [
        &[
            "remote",
            "add",
            "user-1",
            "https://gitlab.com/user-1/bitbraid-live-fork.git",
        ][..],
        &["fetch", "-q", "user-1"],
        &["fetch", "-q", "origin", "refs/merge-requests/8/head"],
        &["branch", "--no-track", "bb-fork-topic", "FETCH_HEAD"],
        &["switch", "-q", "bb-fork-topic"],
        &[
            "branch",
            "-q",
            "--set-upstream-to=user-1/bb-fork-topic",
            "bb-fork-topic",
        ],
    ] {
        twin.run(args).unwrap();
    }
    let ours_git = h.git(&ours);
    assert_eq!(state(&ours_git, &root), state(&twin, &root));
    // Again: the remote is not added twice, and nothing moves.
    let again = checkout();
    assert_eq!(again["created"], false);
    assert_eq!(again["remote_added"], Value::Null);
    assert_eq!(state(&ours_git, &root), state(&twin, &root));
    h.all_used();
}

#[test]
fn gitlab_com_a_branch_is_made_for_an_issue_as_git_would() {
    let h = Harness::new("issue_branch.json", true);
    let root = h.dir.join("world");
    std::fs::create_dir_all(root.join("ours")).unwrap();
    let ours = h.git(&root.join("ours"));
    ours.run(["init", "-q", "-b", "main"]).unwrap();
    std::fs::write(root.join("ours/README.md"), "bitbraid-live\n").unwrap();
    ours.run(["add", "README.md"]).unwrap();
    ours.run(["commit", "-q", "-m", "Initial commit"]).unwrap();
    ours.run([
        "remote",
        "add",
        "origin",
        "https://gitlab.com/user-1/bitbraid-live.git",
    ])
    .unwrap();
    h.git(&root).run(["clone", "-q", "ours", "twin"]).unwrap();
    let twin = h.git(&root.join("twin"));
    twin.run([
        "remote",
        "set-url",
        "origin",
        "https://gitlab.com/user-1/bitbraid-live.git",
    ])
    .unwrap();
    // The open repository's link names the project.
    let made = h
        .run_in(
            &root.join("ours"),
            "issue.branch",
            json!({ "number": PLAIN_ISSUE, "checkout": true }),
        )
        .unwrap();
    assert_eq!(made["created"], "3-say-what-the-menu-is-for");
    twin.run(["switch", "-q", "-c", "3-say-what-the-menu-is-for"])
        .unwrap();
    let heads = |git: &Git| {
        (
            git.run(["symbolic-ref", "HEAD"]).unwrap(),
            git.rev_parse("HEAD").unwrap(),
            git.run(["write-tree"]).unwrap(),
        )
    };
    assert_eq!(heads(&ours), heads(&twin));
    let key = h
        .run_in(
            &root.join("ours"),
            "issue.key_from_branch",
            json!({ "branch": "3-say-what-the-menu-is-for" }),
        )
        .unwrap();
    assert_eq!(key["number"], PLAIN_ISSUE);
    h.all_used();
}

/// The key of the tests and its fingerprint as `ssh-keygen -l` says it.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
const FINGERPRINT: &str = "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0";

#[test]
fn gitlab_com_a_project_is_made_a_fork_refused_and_a_key_uploaded() {
    let h = Harness::new("repos_forks_keys.json", true);
    let dir = h.dir.join("repository");
    std::fs::create_dir_all(&dir).unwrap();
    let git = h.git(&dir);
    git.run(["init", "-q", "-b", "main"]).unwrap();
    git.run([
        "remote",
        "add",
        "origin",
        "https://gitlab.com/user-1/bitbraid-live-writes.git",
    ])
    .unwrap();
    let made = h
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": ACCOUNT, "name": "bitbraid-live-made-user", "private": true,
                    "description": "Made by BitBraid's live test", "remote": "made" }),
        )
        .unwrap();
    assert_eq!(made["repo"]["full_name"], "user-1/bitbraid-live-made-user");
    assert_eq!(made["repo"]["private"], true);
    assert_eq!(made["remote"], "made");
    // One's own project forked into one's own namespace would take its
    // path: gitlab.com answers 409, and no remote is added.
    let err = h
        .run_in(&dir, "provider.fork", json!({ "remote": "origin" }))
        .unwrap_err();
    assert!(err.contains("409") && err.contains("taken"), "{err}");
    let remotes = git.run(["remote", "-v"]).unwrap();
    assert_eq!(
        remotes,
        "made\thttps://gitlab.com/user-1/bitbraid-live-made-user.git (fetch)\n\
         made\thttps://gitlab.com/user-1/bitbraid-live-made-user.git (push)\n\
         origin\thttps://gitlab.com/user-1/bitbraid-live-writes.git (fetch)\n\
         origin\thttps://gitlab.com/user-1/bitbraid-live-writes.git (push)\n"
    );
    let key = h.dir.join("bitbraid-live.pub");
    std::fs::write(&key, KEY).unwrap();
    let added = h.run(
        "provider.ssh_key_add",
        json!({ "account": ACCOUNT, "title": "bitbraid-live test key", "path": key.to_str().unwrap() }),
    );
    assert!(
        added["id"]
            .as_str()
            .is_some_and(|id| id.bytes().all(|b| b.is_ascii_digit())),
        "{added}"
    );
    assert_eq!(added["title"], "bitbraid-live test key");
    assert_eq!(added["fingerprint"], FINGERPRINT);
    h.all_used();
}

#[test]
fn gitlab_com_the_inbox_gathers_the_accounts_merge_requests_and_issues() {
    let h = Harness::new("inbox.json", true);
    let found = h.run("inbox.items", json!({ "wip": false }));
    let pull = |number: u64, groups: Value, why: Value| {
        (
            format!("{ACCOUNT}/pull/user-1/bitbraid-live#{number}"),
            groups,
            why,
        )
    };
    let issue = |number: u64, why: Value| {
        (
            format!("{ACCOUNT}/issue/user-1/bitbraid-live#{number}"),
            Value::Null,
            why,
        )
    };
    assert_eq!(
        summary(&found),
        [
            pull(
                8,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            pull(
                5,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            pull(
                4,
                json!(["ready_to_merge", "no_reviewers"]),
                json!(["created"])
            ),
            pull(
                3,
                json!(["waiting_for_review", "draft"]),
                json!(["created", "review_requested"])
            ),
            pull(
                2,
                json!(["changes_requested"]),
                json!(["created", "review_requested"])
            ),
            pull(
                1,
                json!(["failing_checks"]),
                json!(["created", "assigned", "review_requested"])
            ),
            issue(3, json!(["created"])),
            issue(1, json!(["assigned", "created", "mentioned"])),
        ]
    );
    // GraphQL's lines, GitLab's word on mergeability.
    let topic = &found["pulls"][5];
    assert!(topic["additions"].as_u64().is_some());
    assert_eq!(topic["mergeable"], "clean");
    assert_eq!(found["pulls"][3]["mergeable"], "blocked");
    h.all_used();
}

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
