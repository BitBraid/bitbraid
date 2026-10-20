//! The provider commands against answers recorded from live servers
//! (`tests/recordings/`, made by `tests/suite/live_gitlab_ce.rs` and
//! `tests/suite/live_github.rs` with `BITBRAID_RECORD`, scrubbed of tokens
//! and people). What the commands make of them is held here, in ci,
//! without a network.

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
    replay::{Recording, Replay},
    secrets::MemoryStore,
};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

fn recording(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/recordings")
        .join(name)
}

struct Harness {
    dir: PathBuf,
    replay: Arc<Replay>,
    services: Arc<Services>,
}

impl Harness {
    fn new(name: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-p-recorded-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = Arc::new(Replay::load(&recording(name)).unwrap());
        let services = Services::new(replay.clone(), Arc::new(MemoryStore::new()), &dir);
        Self {
            dir,
            replay,
            services: Arc::new(services),
        }
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
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The value of a form's field in the first request to `path`.
fn field_of(name: &str, path: &str, field: &str) -> String {
    let text = std::fs::read_to_string(recording(name)).unwrap();
    let recorded: Recording = serde_json::from_str(&text).unwrap();
    let exchange = recorded
        .exchanges
        .iter()
        .find(|exchange| exchange.path_and_query == path)
        .unwrap();
    exchange
        .request_body
        .as_str()
        .unwrap()
        .split('&')
        .find_map(|pair| pair.strip_prefix(&format!("{field}=")))
        .unwrap()
        .to_owned()
}

/// The words that may be in no recording: the real accounts of whoever
/// recorded them (logins, the organisation, the parts of a name an
/// account carries), from `BITBRAID_RECORD_FORBID` (split by commas, as
/// the live tests read it when they record). They are named where the
/// recordings are checked and in no file; without the variable they are
/// not looked for, and the checks that need no names still run.
fn forbidden_words() -> Vec<String> {
    std::env::var("BITBRAID_RECORD_FORBID")
        .unwrap_or_default()
        .split(',')
        .map(|word| word.trim().to_lowercase())
        .filter(|word| !word.is_empty())
        .collect()
}

/// Which of `words` (in lower case) a text names, by its place among
/// them: one of the text's words (a run of letters, digits, `-` and `_`,
/// in lower case) or a part of one between `-` and `_`; a word with other
/// characters in it (a full name) anywhere.
fn named_word(text: &str, words: &[String]) -> Option<usize> {
    let lower = text.to_lowercase();
    let candidates: std::collections::BTreeSet<&str> = lower
        .split(|c: char| !(c.is_alphanumeric() || c == '-' || c == '_'))
        .filter(|word| !word.is_empty())
        .flat_map(|word| std::iter::once(word).chain(word.split(['-', '_'])))
        .collect();
    words.iter().position(|word| {
        if word
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
        {
            candidates.contains(word.as_str())
        } else {
            lower.contains(word.as_str())
        }
    })
}

/// The e-mail addresses in a text.
fn addresses(text: &str) -> Vec<&str> {
    let local = |c: char| c.is_ascii_alphanumeric() || "._%+-".contains(c);
    let domain = |c: char| c.is_ascii_alphanumeric() || ".-".contains(c);
    text.match_indices('@')
        .filter_map(|(at, _)| {
            let start = text[..at]
                .char_indices()
                .rev()
                .take_while(|(_, c)| local(*c))
                .last()
                .map(|(start, _)| start)?;
            let end = text[at + 1..]
                .char_indices()
                .take_while(|(_, c)| domain(*c))
                .last()
                .map(|(end, c)| at + 1 + end + c.len_utf8())?;
            let address = &text[start..end];
            address.contains('.').then_some(address)
        })
        .collect()
}

/// Whether a stand-in's login: `user-1`, `org-2`.
fn stand_in_login(text: &str) -> bool {
    ["user-", "org-"].iter().any(|kind| {
        text.strip_prefix(kind)
            .is_some_and(|number| !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()))
    })
}

/// Whether an address in a recording of a public provider leads to no
/// one: at `example.invalid`, the provider's own, GitHub's address for a
/// stand-in (`100001+user-1@users.noreply.github.com`), or a GitLab
/// project's service desk of a stand-in.
fn leads_to_no_one(address: &str) -> bool {
    let Some((local, domain)) = address.rsplit_once('@') else {
        return true;
    };
    match domain.to_ascii_lowercase().as_str() {
        "example.invalid" => true,
        "github.com" => ["git", "noreply", "support"].contains(&local),
        "gitlab.com" => local == "git",
        "users.noreply.github.com" => local
            .split_once('+')
            .is_some_and(|(id, login)| stand_in(id.parse().unwrap_or(0)) && stand_in_login(login)),
        "incoming.gitlab.com" => local.strip_prefix("contact-project+").is_some_and(|rest| {
            let mut parts = rest.splitn(3, '-');
            let login = format!(
                "{}-{}",
                parts.next().unwrap_or_default(),
                parts.next().unwrap_or_default()
            );
            stand_in_login(&login)
        }),
        _ => false,
    }
}

/// Whether an id is a stand-in: a person's (`100001`…), an
/// organisation's (`200001`…) or a thing's (`300001`…).
fn stand_in(id: u64) -> bool {
    (100_001..400_000).contains(&id)
}

/// Whether a key names an id, as the scrubber reads it.
fn names_an_id(key: &str) -> bool {
    !key.ends_with("iid")
        && (key == "id" || key.ends_with("_id") || key.ends_with("_ids") || key.ends_with("Id"))
}

/// Whether a text is a GitHub node id as GitHub makes them (`PR_kwDO…`),
/// or as it made them before (base64 of `012:IssueComment1`).
fn a_node_id(text: &str) -> bool {
    use base64::Engine as _;
    let today = text.split_once('_').is_some_and(|(kind, rest)| {
        !kind.is_empty()
            && kind.bytes().all(|b| b.is_ascii_uppercase())
            && rest.len() >= 8
            && rest.starts_with('k')
            && rest
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    });
    let before = text.starts_with("MD")
        && base64::engine::general_purpose::STANDARD
            .decode(text)
            .ok()
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .is_some_and(|decoded| {
                decoded
                    .split_once(':')
                    .is_some_and(|(length, _)| length.starts_with('0'))
            });
    today || before
}

/// What in a recorded answer of a public provider could lead to an
/// account: an id of five digits or more that is not a stand-in, and a
/// node id. Each is said by where it is.
fn leads(value: &Value, key: Option<&str>, at: &str, found: &mut Vec<String>) {
    match value {
        Value::Object(object) => {
            for (inner_key, inner) in object {
                leads(inner, Some(inner_key), &format!("{at}/{inner_key}"), found);
            }
        }
        Value::Array(items) => {
            for (n, item) in items.iter().enumerate() {
                leads(item, key, &format!("{at}/{n}"), found);
            }
        }
        Value::Number(number)
            if key.is_some_and(names_an_id)
                && number
                    .as_u64()
                    .is_some_and(|id| id >= 10_000 && !stand_in(id)) =>
        {
            found.push(format!("an id at {at}"));
        }
        Value::String(text) if a_node_id(text) => {
            found.push(format!("a node id at {at}"));
        }
        _ => {}
    }
}

#[test]
fn words_addresses_and_ids_that_lead_to_someone_are_found() {
    let words = [
        "alpha".to_owned(),
        "beta-2".to_owned(),
        "first last".to_owned(),
    ];
    for text in [
        "by Alpha",
        "\"login\": \"ALPHA\"",
        "https://github.com/alpha/bitbraid-live",
        "123+alpha@users.noreply.github.com",
        "x-alpha_y",
        "a beta-2 b",
        "\"name\": \"First Last\"",
    ] {
        assert!(named_word(text, &words).is_some(), "{text}");
    }
    for text in ["alphabet", "beta", "user-1 org-1/bitbraid-live", "first"] {
        assert_eq!(named_word(text, &words), None, "{text}");
    }
    assert_eq!(
        addresses("by a.b+c@example.invalid, and @x, git@github.com:org-1/r.git"),
        ["a.b+c@example.invalid", "git@github.com"]
    );
    for address in [
        "user-1@example.invalid",
        "git@github.com",
        "100001+user-1@users.noreply.github.com",
        "contact-project+user-1-bitbraid-live-300001-issue-@incoming.gitlab.com",
    ] {
        assert!(leads_to_no_one(address), "{address}");
    }
    for address in [
        "someone@gmail.com",
        "12345+someone@users.noreply.github.com",
        "contact-project+someone-project-1-issue-@incoming.gitlab.com",
    ] {
        assert!(!leads_to_no_one(address), "{address}");
    }
    let mut found = Vec::new();
    leads(
        &json!({ "id": 1_417_000_001_u64, "iid": 123_456, "number": 10_001,
                 "owner": { "id": 100_001 }, "project_id": 300_002,
                 "node_id": "R_kgDOAbCdEf", "other": "MDQ6VXNlcjE=", "kept": "PR_node-1" }),
        None,
        "",
        &mut found,
    );
    assert_eq!(
        found,
        [
            "an id at /id",
            "a node id at /node_id",
            "a node id at /other"
        ]
    );
}

#[test]
fn every_recording_is_scrubbed_of_the_live_tests_people() {
    let forbidden = forbidden_words();
    for folder in ["gitlab", "github", "github-org", "gitlab-com"] {
        let Ok(entries) = std::fs::read_dir(recording(folder)) else {
            continue;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let text = std::fs::read_to_string(&path).unwrap();
            let recorded: Recording = serde_json::from_str(&text).unwrap();
            for real in ["bb-p-alice", "bb-p-bob", "Alice Tester", "Bob Tester"] {
                assert!(!text.contains(real), "{} holds {real}", path.display());
            }
            if let Some(which) = named_word(&text, &forbidden) {
                panic!(
                    "{} names word {} of BITBRAID_RECORD_FORBID",
                    path.display(),
                    which + 1
                );
            }
            let lower = text.to_lowercase();
            for secret in [
                "glpat-",
                "glrt-",
                "gho_",
                "ghp_",
                "ghu_",
                "github_pat_",
                "gr1348941",
                "\"authorization\"",
                "private-token",
                "set-cookie",
                "cookie",
            ] {
                assert!(!lower.contains(secret), "{} holds {secret}", path.display());
            }
            // What was recorded at github.com and gitlab.com leads to no
            // one: no address but a stand-in's, no picture of an account,
            // no id but a stand-in, no node id.
            if folder == "gitlab" || recorded.hand_made.is_some() {
                continue;
            }
            for address in addresses(&text) {
                assert!(
                    leads_to_no_one(address),
                    "{} holds the address {address}",
                    path.display()
                );
            }
            for picture in ["avatars.githubusercontent.com", "/uploads/-/system/user/"] {
                assert!(
                    !text.contains(picture),
                    "{} holds a picture of {picture}",
                    path.display()
                );
            }
            let mut found = Vec::new();
            for (n, exchange) in recorded.exchanges.iter().enumerate() {
                leads(&exchange.body, None, &format!("{n}"), &mut found);
                leads(&exchange.request_body, None, &format!("{n}"), &mut found);
            }
            assert!(found.is_empty(), "{}: {found:?}", path.display());
        }
    }
}

#[test]
fn gitlab_a_token_connects_and_the_merge_requests_are_listed() {
    let h = Harness::new("gitlab/add_token_and_pr_list.json");
    let account = h.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "token": "glpat-replayed" }),
    );
    assert_eq!(account["id"], "gitlab:127.0.0.1:8929:user-1");
    assert_eq!(account["name"], "User 1");
    assert_eq!(account["scopes"], json!(["api", "read_user"]));
    assert_eq!(account["api"], "http://127.0.0.1:8929/api/v4");
    let listing = h.run(
        "pr.list",
        json!({ "account": "gitlab:127.0.0.1:8929:user-1", "owner": "bb-p-group",
                "name": "bb-p-project", "state": "all" }),
    );
    let items = listing["items"].as_array().unwrap();
    let pull = items
        .iter()
        .find(|pull| pull["head_ref"] == "bb-p-topic")
        .unwrap_or_else(|| panic!("{listing}"));
    assert_eq!(pull["author"]["login"], "user-1");
    assert_eq!(pull["base_ref"], "main");
    assert_eq!(pull["state"], "open");
    assert_eq!(pull["repo"], "bb-p-group/bb-p-project");
    assert!(pull["web_url"].as_str().unwrap().ends_with(&format!(
        "/bb-p-group/bb-p-project/-/merge_requests/{}",
        pull["number"]
    )));
    assert!(listing["freshness"]["fetched"].is_string());
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_signs_in_by_code_once_its_application_may() {
    let name = "gitlab/device_flow.json";
    let h = Harness::new(name);
    // The first application was made through the API, which leaves its
    // device grant off; the second through the site's form, with it on.
    let text = std::fs::read_to_string(recording(name)).unwrap();
    let recorded: Recording = serde_json::from_str(&text).unwrap();
    let ids: Vec<String> = recorded
        .exchanges
        .iter()
        .filter(|exchange| exchange.path_and_query == "/oauth/authorize_device")
        .map(|exchange| {
            exchange
                .request_body
                .as_str()
                .unwrap()
                .split('&')
                .find_map(|pair| pair.strip_prefix("client_id="))
                .unwrap()
                .to_owned()
        })
        .collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(
        field_of(name, "/oauth/authorize_device", "client_id"),
        ids[0]
    );
    let start = |id: &str| {
        h.try_run(
            "account.sign_in_start",
            json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "client_id": id }),
        )
    };
    let err = start(&ids[0]).unwrap_err().to_string();
    assert!(err.contains("denied the request"), "{err}");
    assert!(
        err.contains("turn on its \"Device authorization grant\""),
        "{err}"
    );
    let code = start(&ids[1]).unwrap();
    assert_eq!(
        code["verification_uri"],
        "http://127.0.0.1:8929/oauth/device"
    );
    assert_eq!(code["interval"], 5);
    let poll = json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929",
                       "device_code": code["device_code"] });
    assert_eq!(
        h.run("account.sign_in_poll", poll.clone()),
        json!({ "state": "waiting" })
    );
    let done = h.run("account.sign_in_poll", poll);
    assert_eq!(done["state"], "done");
    assert_eq!(done["account"]["login"], "user-2");
    assert_eq!(done["account"]["sign_in"], "device");
    assert_eq!(done["account"]["scopes"], json!(["api", "read_user"]));
    // Its token, from the keychain, reads.
    let namespaces = h.run(
        "provider.namespaces",
        json!({ "account": done["account"]["id"] }),
    );
    assert_eq!(namespaces[0]["path"], "user-2");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

/// The account of the live test at a GitLab of one's own, as its
/// recordings name it: alice is `user-1`, bob `user-2`.
const GITLAB: &str = "gitlab:127.0.0.1:8929:user-1";

impl Harness {
    /// A recording of a GitLab of one's own, alice connected first, as
    /// the live test did.
    fn gitlab(name: &str) -> Self {
        let h = Self::new(name);
        h.run(
            "account.add_token",
            json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "token": "glpat-replayed" }),
        );
        h
    }

    /// A command about the live test's project.
    fn on_project(&self, id: &str, extra: Value) -> Value {
        let mut params =
            json!({ "account": GITLAB, "owner": "bb-p-group", "name": "bb-p-project" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.run(id, params)
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

#[test]
fn gitlab_merge_requests_are_listed_by_every_filter_and_query() {
    let h = Harness::gitlab("gitlab/pr_list.json");
    let all = h.on_project("pr.list", json!({ "state": "all" }));
    assert_eq!(numbers(&all), [8, 7, 6, 5, 4, 3, 2]);
    let row = |number: u64| {
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["number"] == number)
            .unwrap()
            .clone()
    };
    let topic = row(2);
    assert_eq!(topic["labels"][0]["name"], "bug");
    assert_eq!(topic["labels"][0]["color"], "d9534f");
    assert_eq!(topic["assignees"][0]["login"], "user-1");
    assert_eq!(topic["reviewers"][0]["user"]["login"], "user-2");
    assert_eq!(topic["reviewers"][0]["state"], "approved");
    assert_eq!(topic["ci"], "failure");
    assert_eq!(topic["comments"], 5);
    assert_eq!(topic["head_repo"], Value::Null);
    assert_eq!(row(3)["draft"], true);
    assert_eq!(row(3)["ci"], "pending");
    assert_eq!(row(3)["reviewers"][0]["state"], "pending");
    assert_eq!(row(4)["ci"], "running");
    assert_eq!(row(6)["state"], "merged");
    assert_eq!(row(7)["state"], "closed");
    // From bob's fork, which alice may see.
    assert_eq!(row(8)["head_repo"], "user-2/bb-p-project");
    assert_eq!(row(8)["author"]["login"], "user-2");
    assert_eq!(all["more"], false);
    assert!(all["freshness"]["fetched"].is_string());

    assert_eq!(
        numbers(&h.on_project("pr.list", json!({}))),
        [8, 5, 4, 3, 2]
    );
    assert_eq!(
        numbers(&h.on_project("pr.list", json!({ "filter": "mine" }))),
        [5, 4, 3, 2]
    );
    assert_eq!(
        numbers(&h.on_project("pr.list", json!({ "filter": "assigned" }))),
        [2]
    );
    assert_eq!(
        numbers(&h.on_project("pr.list", json!({ "filter": "review_requested" }))),
        [3]
    );
    let query = |query: &str| numbers(&h.on_project("pr.list", json!({ "query": query })));
    assert_eq!(query("label:bug"), [2]);
    // Either label: one search each, newest first.
    assert_eq!(query("label:bug,ui"), [3, 2]);
    assert_eq!(query("draft:true"), [3]);
    assert_eq!(query("author:user-2"), [8]);
    assert_eq!(query("-label:bug"), [8, 5, 4, 3]);
    assert_eq!(query("base:bb-p-stack-1"), [5]);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
    // What GitLab cannot search by is named, and nothing is asked.
    let asked = h.replay.asked().len();
    let err = h
        .try_run(
            "pr.list",
            json!({ "account": GITLAB, "owner": "bb-p-group", "name": "bb-p-project",
                    "query": "involves:user-1 review:approved" }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("involves, review"), "{err}");
    assert_eq!(h.replay.asked().len(), asked);
}

#[test]
fn gitlab_a_merge_request_is_shown_with_everything_its_view_shows() {
    let h = Harness::gitlab("gitlab/pr_show.json");
    let topic = h.on_project("pr.show", json!({ "number": 2 }));
    assert_eq!(topic["pull"]["number"], 2);
    assert_eq!(topic["pull"]["title"], "Add a file to review");
    assert_eq!(topic["body"], "Made by the live test.");
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["milestone"]["due"], "2026-12-31");
    assert_eq!(topic["milestone"]["open"], true);
    assert_eq!(topic["mergeable"], "clean");
    assert_eq!(topic["merge_methods"], json!(["merge", "squash"]));
    assert_eq!(topic["pull"]["reviewers"][0]["user"]["login"], "user-2");
    assert_eq!(topic["pull"]["reviewers"][0]["state"], "approved");
    assert_eq!(topic["reviews"][0]["state"], "approved");
    assert_eq!(topic["reviews"][0]["author"]["login"], "user-2");
    assert_eq!(topic["pull"]["ci"], "failure");
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
    assert!(checks.contains(&("bb-p-lint", "success")), "{checks:?}");
    assert!(checks.contains(&("bb-p-test", "failure")), "{checks:?}");
    assert_eq!(topic["commits"][0]["summary"], "Add a file to review");
    let file = &topic["files"][0];
    assert_eq!(file["path"], "bb-p.txt");
    assert_eq!(file["status"], "added");
    assert_eq!(
        (file["additions"].as_u64(), file["deletions"].as_u64()),
        (Some(1), Some(0))
    );
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
    assert_eq!(on_lines.len(), 2);
    assert_eq!(on_lines[0]["line"], 1);
    assert_eq!(on_lines[0]["resolved"], true);
    assert_eq!(on_lines[1]["resolved"], false);
    assert_eq!(on_lines[0]["comments"][0]["author"]["login"], "user-2");
    assert_eq!(on_lines[0]["comments"][1]["body"], "Done.");
    assert!(threads.iter().any(|thread| {
        thread["path"].is_null() && thread["comments"][0]["body"] == "Looks good to me."
    }));
    // No note GitLab wrote itself is a comment.
    assert!(
        threads
            .iter()
            .flat_map(|thread| thread["comments"].as_array().unwrap())
            .all(|comment| !comment["body"].as_str().unwrap().starts_with("approved"))
    );
    let fork = h.on_project("pr.show", json!({ "number": 8 }));
    assert_eq!(fork["pull"]["head_repo"], "user-2/bb-p-project");
    let draft = h.on_project("pr.show", json!({ "number": 3 }));
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["mergeable"], "blocked");
    assert_eq!(draft["pull"]["ci"], "pending");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_choices_stacks_and_the_merge_request_of_a_branch() {
    let h = Harness::gitlab("gitlab/pr_options_stack.json");
    let options = h.on_project("pr.options", json!({}));
    let labels: Vec<&str> = options["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|label| label["name"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["bug", "ui"]);
    assert_eq!(options["milestones"][0]["title"], "v1");
    let people: Vec<&str> = options["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["login"].as_str().unwrap())
        .collect();
    assert!(
        people.contains(&"user-1") && people.contains(&"user-2"),
        "{people:?}"
    );
    assert_eq!(options["merge_methods"], json!(["merge", "squash"]));
    let stack = h.on_project("pr.stack", json!({ "number": 5 }));
    let stacked: Vec<(u64, &str, &str)> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["number"].as_u64().unwrap(),
                entry["head_ref"].as_str().unwrap(),
                entry["base_ref"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        stacked,
        [
            (4, "bb-p-stack-1", "main"),
            (5, "bb-p-stack-2", "bb-p-stack-1")
        ]
    );
    assert_eq!(stack["position"], 1);
    assert_eq!(
        h.on_project("pr.stack", json!({ "number": 4 }))["position"],
        0
    );
    let branch = h.on_project("pr.for_branch", json!({ "branch": "bb-p-stack-1" }));
    assert_eq!(branch["number"], 4);
    assert_eq!(
        h.on_project("pr.for_branch", json!({ "branch": "bb-p-no-such-branch" })),
        Value::Null
    );
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_issues_are_listed_by_every_filter_and_shown() {
    let h = Harness::gitlab("gitlab/issues.json");
    let list = |extra: Value| numbers(&h.on_project("issue.list", extra));
    assert_eq!(list(json!({})), [1]);
    assert_eq!(list(json!({})), [1]);
    assert_eq!(list(json!({ "state": "closed" })), [2]);
    assert_eq!(list(json!({ "state": "all" })), [2, 1]);
    assert_eq!(list(json!({ "filter": "assigned" })), Vec::<u64>::new());
    assert_eq!(list(json!({ "filter": "created" })), [1]);
    // Named by bob in a comment: a to-do item of alice's.
    assert_eq!(list(json!({ "filter": "mentioned" })), [1]);
    assert_eq!(list(json!({ "query": "#bug" })), [1]);
    assert_eq!(list(json!({ "query": "@user-2" })), [1]);
    assert_eq!(list(json!({ "query": "$v1 button" })), [1]);
    assert_eq!(list(json!({ "query": "-label:bug", "state": "all" })), [2]);
    let shown = h.on_project("issue.show", json!({ "number": 1 }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["issue"]["state"], "open");
    assert_eq!(shown["issue"]["author"]["login"], "user-1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], "user-2");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["comments"], 2);
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    let comments = shown["comments"].as_array().unwrap();
    assert_eq!(comments.len(), 2);
    assert_eq!(comments[0]["author"]["login"], "user-2");
    assert_eq!(comments[0]["body"], "@user-1 which button do you mean?");
    assert_eq!(comments[1]["body"], "The one at the top.");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_the_checks_of_commits_are_read() {
    let h = Harness::gitlab("gitlab/ci_status.json");
    let shas = [
        "91342a24f52d21ef96b01fff4ab4015c95acc7af",
        "72272d5f10c55f9e3da533a926531ac0f2b59a22",
        "87ed86802975245acad6a9d3e9c45c243e3b2a28",
        "0123456789abcdef0123456789abcdef01234567",
    ];
    let statuses = h.on_project("ci.status", json!({ "shas": shas }));
    let summed: Vec<(&str, &str)> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| {
            (
                status["sha"].as_str().unwrap(),
                status["state"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        summed,
        [
            (shas[0], "failure"),
            (shas[1], "pending"),
            (shas[2], "running"),
            (shas[3], "none")
        ]
    );
    let lint = statuses[0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|check| check["name"] == "bb-p-lint")
        .unwrap();
    assert_eq!(lint["state"], "success");
    assert_eq!(lint["description"], "bb-p-lint says success");
    assert_eq!(
        lint["url"],
        "http://127.0.0.1:8929/bb-p-group/bb-p-project/-/pipelines"
    );
    assert!(lint["started"].is_string());
    assert!(statuses[3]["checks"].as_array().unwrap().is_empty());
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_projects_namespaces_and_pictures_are_read() {
    let h = Harness::gitlab("gitlab/repos.json");
    let repos = |filter: &str, query: Option<&str>| {
        h.run(
            "provider.repos",
            json!({ "account": GITLAB, "filter": filter, "query": query }),
        )
    };
    let names = |listing: &Value| {
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let own = repos("own", None);
    assert_eq!(names(&own), ["user-1/bb-p-own"]);
    let repo = &own["items"][0];
    assert_eq!(repo["owner"], "user-1");
    assert_eq!(repo["name"], "bb-p-own");
    assert_eq!(repo["private"], true);
    assert_eq!(repo["default_branch"], "main");
    assert_eq!(repo["description"], "Alice's own project");
    assert_eq!(
        repo["clone_https"],
        "http://127.0.0.1:8929/user-1/bb-p-own.git"
    );
    assert_eq!(
        repo["clone_ssh"],
        "ssh://git@127.0.0.1:2224/user-1/bb-p-own.git"
    );
    assert_eq!(own["more"], false);
    let organisation = names(&repos("organisation", None));
    assert!(organisation.contains(&"bb-p-group/bb-p-project".to_owned()));
    assert!(!organisation.contains(&"user-1/bb-p-own".to_owned()));
    let _ = repos("organisation", None);
    assert_eq!(names(&repos("starred", None)), ["bb-p-group/bb-p-project"]);
    let all = names(&repos("all", None));
    assert!(all.contains(&"user-1/bb-p-own".to_owned()) && all.len() >= 2);
    assert_eq!(names(&repos("all", Some("bb-p-own"))), ["user-1/bb-p-own"]);
    let namespaces = h.run("provider.namespaces", json!({ "account": GITLAB }));
    let kinds: Vec<(&str, &str)> = namespaces
        .as_array()
        .unwrap()
        .iter()
        .map(|namespace| {
            (
                namespace["path"].as_str().unwrap(),
                namespace["kind"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(kinds.contains(&("user-1", "user")), "{kinds:?}");
    assert!(kinds.contains(&("bb-p-group", "group")), "{kinds:?}");
    let pictures = h.run(
        "provider.avatars",
        json!({ "account": GITLAB,
                "emails": ["user-1@example.invalid", "user-2@example.invalid"] }),
    );
    assert!(
        pictures["user-1@example.invalid"]
            .as_str()
            .unwrap()
            .starts_with("https://")
    );
    assert_eq!(pictures.as_object().unwrap().len(), 2);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

/// The test repository at github.com, as its recordings name it.
const GITHUB: &str = "github:github.com:user-1";

impl Harness {
    /// A recording of github.com, the account connected first, as the
    /// live test did.
    fn github(name: &str) -> Self {
        let h = Self::new(name);
        h.run(
            "account.add_token",
            json!({ "kind": "github", "token": "ghx_replayed" }),
        );
        h
    }

    /// A command about the test repository.
    fn on_repo(&self, id: &str, extra: Value) -> Value {
        let mut params =
            json!({ "account": GITHUB, "owner": "user-1", "name": "bitbraid-test-core" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.run(id, params)
    }
}

#[test]
fn github_pull_requests_are_listed_by_every_filter_and_query() {
    let h = Harness::github("github/pr_list.json");
    let all = h.on_repo("pr.list", json!({ "state": "all" }));
    assert_eq!(numbers(&all), [6, 5, 4, 3, 2, 1]);
    let row = |number: u64| {
        all["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["number"] == number)
            .unwrap()
            .clone()
    };
    let topic = row(1);
    assert_eq!(topic["title"], "Add a file to review");
    assert_eq!(topic["repo"], "user-1/bitbraid-test-core");
    assert_eq!(topic["head_ref"], "bb-p-topic");
    assert_eq!(topic["base_ref"], "main");
    assert_eq!(
        topic["head_sha"],
        "7802fee5a71c66a697cad3315808700a2e4e87f8"
    );
    assert_eq!(topic["labels"][0]["name"], "bug");
    assert_eq!(topic["labels"][0]["color"], "d73a4a");
    assert_eq!(topic["assignees"][0]["login"], "user-1");
    assert_eq!(topic["author"]["login"], "user-1");
    assert_eq!(topic["ci"], "failure");
    assert_eq!(topic["comments"], 1);
    assert!(topic["web_url"].as_str().unwrap().ends_with("/pull/1"));
    assert_eq!(row(2)["draft"], true);
    assert_eq!(row(2)["ci"], "pending");
    assert_eq!(row(3)["ci"], "failure");
    assert_eq!(row(4)["base_ref"], "bb-p-stack-1");
    assert_eq!(row(5)["state"], "merged");
    assert_eq!(row(6)["state"], "closed");
    assert!(all["freshness"]["remaining"].is_u64());
    let list = |extra: Value| numbers(&h.on_repo("pr.list", extra));
    assert_eq!(list(json!({})), [4, 3, 2, 1]);
    assert_eq!(list(json!({ "filter": "mine" })), [4, 3, 2, 1]);
    assert_eq!(list(json!({ "filter": "assigned" })), [1]);
    assert_eq!(
        list(json!({ "filter": "review_requested" })),
        Vec::<u64>::new()
    );
    assert_eq!(list(json!({ "state": "merged" })), [5]);
    assert_eq!(list(json!({ "state": "closed" })), [6]);
    assert_eq!(list(json!({ "query": "label:bug" })), [1]);
    assert_eq!(list(json!({ "query": "label:bug,ui" })), [2, 1]);
    assert_eq!(list(json!({ "query": "draft:true" })), [2]);
    assert_eq!(list(json!({ "query": "-label:bug" })), [4, 3, 2]);
    assert_eq!(list(json!({ "query": "base:bb-p-stack-1" })), [4]);
    assert_eq!(list(json!({ "query": "status:failure label:bug" })), [1]);
    assert_eq!(list(json!({ "query": "milestone:v1" })), [1]);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn github_a_pull_request_is_shown_with_everything_its_view_shows() {
    let h = Harness::github("github/pr_show.json");
    let topic = h.on_repo("pr.show", json!({ "number": 1 }));
    assert_eq!(topic["pull"]["number"], 1);
    assert_eq!(topic["body"], "Made by the live test.");
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["milestone"]["id"], "1");
    assert_eq!(topic["milestone"]["due"], "2026-12-30T00:00:00Z");
    assert_eq!(topic["merge_methods"], json!(["merge", "squash", "rebase"]));
    assert_ne!(topic["mergeable"], "conflicts");
    assert_eq!(topic["pull"]["ci"], "failure");
    let checks: Vec<(&str, &str, &str)> = topic["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| {
            (
                check["name"].as_str().unwrap(),
                check["state"].as_str().unwrap(),
                check["url"].as_str().unwrap(),
            )
        })
        .collect();
    assert!(checks.contains(&("bb-p-lint", "success", "https://example.invalid/bb-p-lint")));
    assert!(checks.contains(&("bb-p-test", "failure", "https://example.invalid/bb-p-test")));
    assert_eq!(topic["commits"][0]["summary"], "Add a file to review");
    assert_eq!(topic["commits"][0]["author"]["login"], "user-1");
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
    assert_eq!(on_lines.len(), 2);
    assert_eq!(on_lines[0]["line"], 1);
    assert_eq!(on_lines[0]["resolved"], true);
    assert!(on_lines[0]["diff_hunk"].as_str().unwrap().starts_with("@@"));
    let said: Vec<&str> = on_lines[0]["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| comment["body"].as_str().unwrap())
        .collect();
    assert_eq!(said, ["This line could say more.", "Done."]);
    assert_eq!(on_lines[1]["resolved"], false);
    // The conversation on the whole of it, without a path.
    let whole = threads
        .iter()
        .find(|thread| thread["path"].is_null())
        .unwrap();
    assert_eq!(whole["comments"][0]["body"], "Looks good to me.");
    let reviews = topic["reviews"].as_array().unwrap();
    assert!(!reviews.is_empty());
    assert!(reviews.iter().all(|review| review["state"] == "commented"));
    assert_eq!(reviews[0]["body"], "A few things on the lines.");
    // Only a comment of their own: the reviewer is the author, commenting.
    assert_eq!(topic["pull"]["reviewers"][0]["state"], "commented");
    let draft = h.on_repo("pr.show", json!({ "number": 2 }));
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["mergeable"], "blocked");
    assert!(draft["threads"].as_array().unwrap().is_empty());
    let second = h.on_repo("pr.show", json!({ "number": 4 }));
    assert_eq!(second["pull"]["base_ref"], "bb-p-stack-1");
    assert_eq!(second["files"][0]["path"], "bb-p-stack-2.txt");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn github_issues_are_listed_by_every_filter_and_shown() {
    let h = Harness::github("github/issues.json");
    let list = |extra: Value| numbers(&h.on_repo("issue.list", extra));
    // The pull requests among GitHub's issues are left out.
    assert_eq!(list(json!({})), [7]);
    assert_eq!(list(json!({ "state": "closed" })), [8]);
    assert_eq!(list(json!({ "state": "all" })), [8, 7]);
    assert_eq!(list(json!({ "filter": "assigned" })), [7]);
    assert_eq!(list(json!({ "filter": "created" })), [7]);
    assert_eq!(list(json!({ "query": "labels:bug" })), [7]);
    // A milestone by its title is asked by its number.
    assert_eq!(list(json!({ "query": "milestone:v1 button" })), [7]);
    assert_eq!(
        list(json!({ "query": "nothing-like-this" })),
        Vec::<u64>::new()
    );
    let shown = h.on_repo("issue.show", json!({ "number": 7 }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["issue"]["state"], "open");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], "user-1");
    assert_eq!(shown["issue"]["comments"], 2);
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    let comments: Vec<&str> = shown["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| comment["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        comments,
        ["Which button do you mean?", "The one at the top."]
    );
    let err = h
        .try_run(
            "issue.show",
            json!({ "account": GITHUB, "owner": "user-1", "name": "bitbraid-test-core",
                    "number": 1 }),
        )
        .unwrap_err()
        .to_string();
    assert!(err.contains("pull request"), "{err}");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn hand_made_recordings_say_what_they_were_made_from() {
    for name in ["github/hand_made_repos.json", "github/hand_made_pull.json"] {
        let text = std::fs::read_to_string(recording(name)).unwrap();
        let made: Recording = serde_json::from_str(&text).unwrap();
        let from = made.hand_made.unwrap_or_default();
        assert!(from.contains("rest-api-description"), "{name}: {from}");
    }
    // Those recorded live say nothing of the kind.
    let text = std::fs::read_to_string(recording("github/pr_list.json")).unwrap();
    let live: Recording = serde_json::from_str(&text).unwrap();
    assert_eq!(live.hand_made, None);
}

/// The account of GitHub's examples.
const OCTOCAT: &str = "github:github.com:octocat";

#[test]
fn github_repositories_and_where_one_may_be_made_by_hand() {
    let h = Harness::new("github/hand_made_repos.json");
    h.run(
        "account.add_token",
        json!({ "kind": "github", "token": "ghx_replayed" }),
    );
    let repos = |extra: Value| {
        let mut params = json!({ "account": OCTOCAT });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        h.run("provider.repos", params)
    };
    let names = |listing: &Value| {
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let first = repos(json!({ "filter": "own" }));
    assert_eq!(names(&first), ["octocat/Hello-World"]);
    assert_eq!(first["more"], true);
    let repo = &first["items"][0];
    assert_eq!(repo["id"], "1296269");
    assert_eq!(repo["owner"], "octocat");
    assert_eq!(repo["name"], "Hello-World");
    assert_eq!(repo["description"], "This your first repo!");
    assert_eq!(repo["private"], false);
    assert_eq!(repo["default_branch"], "master");
    assert_eq!(
        repo["clone_https"],
        "https://github.com/octocat/Hello-World.git"
    );
    assert_eq!(repo["clone_ssh"], "git@github.com:octocat/Hello-World.git");
    assert_eq!(repo["web_url"], "https://github.com/octocat/Hello-World");
    assert_eq!(repo["updated"], "2011-01-26T19:06:43Z");
    assert_eq!(repo["stars"], 80);
    assert_eq!(first["freshness"]["remaining"], 4999);
    let second = repos(json!({ "filter": "own", "page": 2 }));
    assert_eq!(names(&second), ["octocat/Spoon-Knife"]);
    assert_eq!(second["items"][0]["archived"], true);
    assert_eq!(second["more"], false);
    let organisation = repos(json!({ "filter": "organisation" }));
    assert_eq!(names(&organisation), ["github/Hello-World"]);
    assert_eq!(organisation["items"][0]["private"], true);
    assert_eq!(
        names(&repos(json!({ "filter": "starred" }))),
        ["octocat/Hello-World"]
    );
    assert_eq!(names(&repos(json!({}))).len(), 3);
    // GitHub's list has no search: the words are looked for in what was
    // read.
    assert_eq!(
        names(&repos(json!({ "query": "spoon demonstration" }))),
        ["octocat/Spoon-Knife"]
    );
    let namespaces = h.run("provider.namespaces", json!({ "account": OCTOCAT }));
    assert_eq!(
        namespaces,
        json!([
            { "path": "octocat", "name": "monalisa octocat", "kind": "user",
              "avatar_url": "https://github.com/images/error/octocat_happy.gif" },
            { "path": "github", "name": "github", "kind": "organisation",
              "avatar_url": "https://github.com/images/error/octocat_happy.gif" }
        ])
    );
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn github_a_pull_request_from_a_fork_with_verdicts_and_check_runs_by_hand() {
    let h = Harness::new("github/hand_made_pull.json");
    h.run(
        "account.add_token",
        json!({ "kind": "github", "token": "ghx_replayed" }),
    );
    let on = |extra: Value| {
        let mut params = json!({ "account": OCTOCAT, "owner": "octocat", "name": "Hello-World" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        params
    };
    let shown = h.run("pr.show", on(json!({ "number": 1347 })));
    let pull = &shown["pull"];
    assert_eq!(pull["title"], "Amazing new feature");
    assert_eq!(pull["head_repo"], "hubot/Hello-World");
    assert_eq!(pull["comments"], 10);
    assert_eq!(shown["mergeable"], "clean");
    assert_eq!(shown["milestone"]["title"], "v1.0");
    assert_eq!(shown["merge_methods"], json!(["merge", "squash", "rebase"]));
    // Each reviewer's verdict; those asked and silent are pending, a team
    // among them.
    let reviewers: Vec<(&str, &str)> = pull["reviewers"]
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
    assert_eq!(
        reviewers,
        [
            ("octocat", "approved"),
            ("hubot", "changes_requested"),
            ("other_user", "pending"),
            ("justice-league", "pending")
        ]
    );
    let reviews: Vec<(&str, &str)> = shown["reviews"]
        .as_array()
        .unwrap()
        .iter()
        .map(|review| {
            (
                review["state"].as_str().unwrap(),
                review["body"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        reviews,
        [
            ("approved", "Here is the body for the review."),
            ("changes_requested", "Please add a test.")
        ]
    );
    // A check run of an app and a status, summed up.
    let checks = shown["checks"].as_array().unwrap();
    assert_eq!(checks[0]["name"], "mighty_readme");
    assert_eq!(checks[0]["state"], "neutral");
    assert_eq!(checks[0]["description"], "Mighty Readme report");
    assert_eq!(checks[0]["url"], "https://example.com");
    assert_eq!(checks[0]["finished"], "2018-05-04T01:14:52Z");
    assert_eq!(checks[1]["name"], "continuous-integration/jenkins");
    assert_eq!(checks[1]["state"], "success");
    assert_eq!(pull["ci"], "success");
    // A review comment and its answer, resolved; the conversation on the
    // whole of it.
    let threads = shown["threads"].as_array().unwrap();
    let on_line = threads
        .iter()
        .find(|thread| thread["path"] == "file1.txt")
        .unwrap();
    assert_eq!(on_line["id"], "8");
    assert_eq!(on_line["line"], 2);
    assert_eq!(on_line["resolved"], true);
    let said: Vec<&str> = on_line["comments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|comment| comment["body"].as_str().unwrap())
        .collect();
    assert_eq!(said, ["Is this right?", "Great stuff!"]);
    let whole = threads
        .iter()
        .find(|thread| thread["path"].is_null())
        .unwrap();
    assert_eq!(whole["comments"][0]["body"], "Me too");
    assert_eq!(shown["files"][0]["additions"], 103);
    assert_eq!(shown["commits"][0]["author_name"], "Monalisa Octocat");
    assert_eq!(shown["commits"][0]["author"]["login"], "octocat");
    let statuses = h.run(
        "ci.status",
        on(json!({ "shas": ["6dcb09b5b57875f334f61aebed695e2e4193db5e",
                            "ce587453ced02b1526dfb4cb910479d431683101"] })),
    );
    assert_eq!(statuses[0]["state"], "success");
    assert_eq!(statuses[0]["checks"].as_array().unwrap().len(), 2);
    assert_eq!(statuses[1]["state"], "none");
    // The pictures of commits' authors, by the address in the commit; an
    // address GitHub made for an account says whose it is.
    let pictures = h.run(
        "provider.avatars",
        on(json!({ "emails": ["support@github.com",
                              "583231+octocat@users.noreply.github.com",
                              "nobody@example.org"] })),
    );
    assert_eq!(
        pictures,
        json!({
            "support@github.com": "https://github.com/images/error/octocat_happy.gif",
            "583231+octocat@users.noreply.github.com":
                "https://avatars.githubusercontent.com/u/583231?v=4"
        })
    );
    // Learnt: asked again, the commits are not read again.
    let asked = h.replay.asked().len();
    let again = h.run(
        "provider.avatars",
        on(json!({ "emails": ["Support@GitHub.com"] })),
    );
    assert_eq!(
        again["Support@GitHub.com"],
        "https://github.com/images/error/octocat_happy.gif"
    );
    assert_eq!(h.replay.asked().len(), asked);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}
