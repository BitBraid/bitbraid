//! The provider commands against gitlab.com, live, with a test account:
//! one account made for the tests, its token of the `api` and `read_user`
//! scopes. Every read, and every write one account can make, is made here
//! on public projects in the account's own namespace: `<repo>` for the
//! reads, with merge requests in every state, conversations on lines (one
//! resolved), labels, a milestone, issues with comments and the statuses
//! of commits; `<repo>-fork`, a fork of it in the same namespace under
//! another name, for a merge request from a fork; `<repo>-writes` and
//! `<repo>-ff` (which merges only by fast-forward) for the writes.
//! `<repo>` is `bitbraid-live`, or the name `BITBRAID_LIVE_REPO` gives.
//! Projects `provider.repo_create` makes (`<repo>-made-user`) and the key
//! `provider.ssh_key_add` uploads (`<repo> test key`) are removed again in
//! the same run.
//!
//! What one account cannot show is not here: a verdict given by someone
//! else than the author is tested with two users on a GitLab of one's own
//! (`live_gitlab_ce.rs`); gitlab.com runs a new account's jobs only once
//! it has given a credit card, so there are no pipelines, and
//! the checks are statuses set through the API, as another service would
//! set them; gitlab.com makes a top-level group only in its web pages
//! (its API answers 403), so there is no group, and `provider.fork` is
//! seen refused (a fork of one's own project into one's own namespace
//! takes its path, which is taken).
//!
//! Opt-in, ignored by default: it runs with `--ignored` when
//! `BITBRAID_LIVE_GITLAB_COM` is set, and is skipped with a message
//! otherwise. It needs, on Linux with a Secret Service and `secret-tool`,
//! the account's token in the keychain under the service `bitbraid-test`
//! with the login as an attribute:
//!
//! ```sh
//! secret-tool store --label='BitBraid test: gitlab.com' \
//!     service bitbraid-test account gitlab-main login <login>
//! BITBRAID_LIVE_GITLAB_COM=1 \
//!     cargo test -p bitbraid-integrations --test suite live_gitlab_com:: -- --ignored
//! ```
//!
//! No account is named in this file: the login is the keychain item's
//! `login` attribute, and the token is read from the same item at run
//! time and never printed or written. The environment's `GITLAB_TOKEN` is
//! never read. The token's first request has to be `GET /api/v4/user`
//! answering with the item's login, or nothing else is let through for
//! it.
//!
//! A guard in front of the network holds every request to what the test
//! may touch, and refuses the rest before it is sent: at
//! `https://gitlab.com/api/v4`, the account itself (`/user`, its keys,
//! its token's own facts), the lists of its projects (`/projects?owned`,
//! `starred`, `membership`) and namespaces, a project of the test's name
//! made in its namespace (`<repo>-made-*`), pictures (`/avatar`), a
//! user asked for by the account's own login, its to-do items of a test
//! project, and anything under `/projects/<login>%2F<repo>*` (or such a
//! project's id, once an answer named it), where removing is let through
//! only for `<repo>-made-*`; GraphQL questions whose one top field is
//! `project(fullPath: $path)` with a test project in its variables;
//! without a token, `gitlab.com/oauth/authorize_device` and
//! `/oauth/token` with the built-in application id; and a token the guard
//! does not know (the device flow's) may ask who it is and nothing else.
//! The account's own things that were there before the tests (a project,
//! a group) are taken out of the lists the guard hands on, so that no
//! command sees them and no recording names them; they are never asked
//! about by name, changed or removed.
//!
//! What the reads need is made when missing (the setup can run any number
//! of times); the writes make their branches anew in each run. The
//! content is synthetic, and commits are written with a test identity
//! (gitlab.com still names the account as their committer). With
//! `BITBRAID_RECORD=<dir>` the answers of the commands (not of the setup)
//! are recorded there, scrubbed, the account as `user-1` and `<repo>` as
//! `bitbraid-live`: `tests/recordings/gitlab-com/` keeps them, and
//! `tests/suite/recorded_gitlab_com.rs` replays them by default.
//! `BITBRAID_RECORD_FORBID` names more words, split by commas, that may
//! not be in a recording (a name the account carries); it is read when
//! the test runs and is in no file.
//!
//! Signing in by a code is tested up to the user's part (a code is asked
//! for, and asking for the token says "waiting"); the whole way, with
//! someone approving the code at gitlab.com as the test account, is a test
//! of its own that runs only when `BITBRAID_LIVE_GITLAB_DEVICE` names the
//! file the code is written to.
//!
//! Sources: docs.gitlab.com/api "Users" (get the current user), "Personal
//! access tokens" (`/personal_access_tokens/self`), "Projects" (create,
//! get, edit, delete with `permanently_remove` and `full_path`, fork with
//! `name`, `path` and `namespace_path`, star, `import_status`),
//! "Namespaces", "Groups" (create a group: not for a top-level group at
//! GitLab.com), "Commits" (create with `actions`, `author_name` and
//! `author_email`; list the statuses of a commit; set the status of a
//! commit: `state`, `name`, `ref`, `target_url`, `description`),
//! "Branches", "Merge requests" (create, with `target_project_id` from a
//! fork; update with `state_event`; merge; `diff_refs`), "Merge request
//! approvals" (approve), "Draft notes" (publish with `reviewer_state`),
//! "Discussions" (create a thread on a line with `position`, add a note,
//! resolve), "Notes", "Issues", "Labels", "Milestones", "To-do items",
//! "User SSH keys" (list, delete), "Avatar", "REST API" (namespaced paths
//! as ids); docs.gitlab.com/api/graphql "Reference" (`Query.project`);
//! docs.gitlab.com "OAuth 2.0 identity provider API" (the device
//! authorization grant); secret-tool(1).
#![allow(clippy::print_stderr)]

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, atomic::AtomicBool},
    time::{Duration, Instant},
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    api::encode,
    http::{Call, Client, Fetch, FetchError, Limits, Method, Response, Url},
    oauth::GITLAB_CLIENT_ID,
    replay::Recorder,
    secrets::{MemoryStore, Secret},
};
use serde_json::{Value, json};

use super::live_github::{DEFAULT_REPO, LiveName, live_repo, top_fields};

/// The site.
const SITE: &str = "https://gitlab.com";
/// The project of the reads, public (`bitbraid-live`, or the name
/// `BITBRAID_LIVE_REPO` gives), and its fork in the same namespace.
static REPO: LiveName = LiveName::new("");
static FORK: LiveName = LiveName::new("-fork");
/// The projects of the writes, public: one that merges in every way, one
/// that merges only by fast-forward.
static WRITES: LiveName = LiveName::new("-writes");
static FAST_FORWARD: LiveName = LiveName::new("-ff");
/// The project `provider.repo_create` makes, removed in the same run.
static MADE: LiveName = LiveName::new("-made-user");
/// The title of the key the test uploads.
static KEY_TITLE: LiveName = LiveName::new(" test key");
/// Who the commits the test makes are by.
const IDENTITY: (&str, &str) = ("BitBraid Test", "bitbraid-test@example.invalid");
/// The keychain item of the test account.
const ITEM: &str = "gitlab-main";

/// The test account's login, taken at run time from the keychain item;
/// none, with a message, when the live tests are not asked for.
fn login_of_this_run() -> Option<String> {
    if std::env::var_os("BITBRAID_LIVE_GITLAB_COM").is_none() {
        eprintln!("skipped: set BITBRAID_LIVE_GITLAB_COM, with the test account in the keychain");
        return None;
    }
    let login = keychain_login(ITEM)
        .unwrap_or_else(|| panic!("no login is stored with the keychain item {ITEM}"));
    assert!(
        !login.is_empty()
            && login
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "the test account's name is not a login"
    );
    Some(login)
}

/// The login of this run, once [`world`] read it.
static LOGIN: OnceLock<String> = OnceLock::new();

/// The test account's login.
fn login() -> &'static str {
    LOGIN.get().expect("the test account is read first")
}

/// The login stored with a keychain item, read from its attributes, which
/// `secret-tool` writes to its standard error; its standard output (the
/// secret) is dropped unread.
fn keychain_login(item: &str) -> Option<String> {
    let out = std::process::Command::new("secret-tool")
        .args([
            "search",
            "--all",
            "service",
            "bitbraid-test",
            "account",
            item,
        ])
        .stdout(std::process::Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|line| line.trim().strip_prefix("attribute.login = "))
        .map(|login| login.trim().to_owned())
}

/// The test account's token, from its keychain item.
fn token() -> Option<Secret> {
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "service", "bitbraid-test", "account", ITEM])
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !token.is_empty()).then(|| Secret::new(token))
}

/// Whether a project's path is one the tests may touch: `<repo>` or
/// `<repo>-*`, in the account's namespace.
fn ours(login: &str, repo: &str, full_path: &str) -> bool {
    let Some((owner, name)) = full_path.rsplit_once('/') else {
        return false;
    };
    !login.is_empty()
        && !repo.is_empty()
        && owner.eq_ignore_ascii_case(login)
        && name
            .strip_prefix(repo)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Whether a project of the tests may be removed: one made by
/// `provider.repo_create` (named so, or so before it was marked for
/// removal).
fn removable(repo: &str, full_path: &str) -> bool {
    full_path.rsplit_once('/').is_some_and(|(_, name)| {
        name.strip_prefix(repo)
            .is_some_and(|rest| rest.starts_with("-made-"))
    })
}

/// The value of a field of an address's query.
fn query_field<'a>(query: &'a str, field: &str) -> Option<&'a str> {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix(field)?.strip_prefix('='))
}

/// Whether a GraphQL question may be asked: one top field,
/// `project(fullPath: $path)`, a test project in its variables, nothing
/// named in its text, and none of the fields that reach the account's
/// other things.
fn graphql_is_ours(login: &str, repo: &str, body: &Value) -> bool {
    let Some(query) = body["query"].as_str() else {
        return false;
    };
    if query.contains('"') || !query.trim_start().starts_with("query") {
        return false;
    }
    let forbidden = [
        "currentUser",
        "user",
        "users",
        "group",
        "groups",
        "namespace",
        "projects",
        "todos",
        "snippets",
        "mutation",
    ];
    if query
        .split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .any(|word| forbidden.contains(&word))
    {
        return false;
    }
    let Some(fields) = top_fields(query) else {
        return false;
    };
    fields.len() == 1
        && fields[0].0 == "project"
        && fields[0].1 == "fullPath:$path"
        && body["variables"]["path"]
            .as_str()
            .is_some_and(|path| ours(login, repo, path))
}

/// Who sends a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// The test account, by its token.
    Account,
    /// A token the guard does not know: the device flow's.
    Stranger,
    /// No token.
    Nobody,
}

/// The network, held to what this test may ask.
struct Guard {
    inner: Client,
    /// Whose things it lets through.
    login: String,
    /// The name of the test projects (see [`live_repo`]).
    repo: String,
    /// The test account's token, when it is used.
    token: Option<Secret>,
    /// Whether the token's `GET /user` answered with the login.
    confirmed: Mutex<bool>,
    /// The test projects' ids, as answers named them, and their paths.
    projects: Mutex<BTreeMap<u64, String>>,
    /// The ids of the keys of the test's title, as answers named them.
    keys: Mutex<BTreeSet<u64>>,
    /// What was taken out of lists: names that may not be recorded.
    hidden: Mutex<BTreeSet<String>>,
}

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field("token", &self.token.is_some())
            .finish()
    }
}

impl Guard {
    fn new(login: &str, token: Option<Secret>) -> Self {
        Self::with_repo(login, token, live_repo())
    }

    /// The title of the key the test uploads.
    fn key_title(&self) -> String {
        format!("{} test key", self.repo)
    }

    /// The guard of the test projects named `repo`.
    fn with_repo(login: &str, token: Option<Secret>, repo: &str) -> Self {
        Self {
            inner: Client::new("BitBraid-Test/live"),
            login: login.to_owned(),
            repo: repo.to_owned(),
            token,
            confirmed: Mutex::default(),
            projects: Mutex::default(),
            keys: Mutex::default(),
            hidden: Mutex::default(),
        }
    }

    fn caller(&self, call: &Call) -> Caller {
        let Some(value) = call
            .header("authorization")
            .or_else(|| call.header("private-token"))
        else {
            return Caller::Nobody;
        };
        let sent = value.strip_prefix("Bearer ").unwrap_or(value).trim();
        match &self.token {
            Some(token) if token.expose() == sent => Caller::Account,
            _ => Caller::Stranger,
        }
    }

    /// The test project a project's part of an address stands for: its id
    /// once an answer named it, or its path with `%2F` for each `/`.
    fn project_of(&self, segment: &str) -> Option<String> {
        if !segment.is_empty() && segment.bytes().all(|b| b.is_ascii_digit()) {
            let id: u64 = segment.parse().ok()?;
            return self.projects.lock().unwrap().get(&id).cloned();
        }
        let path = segment.replace("%2F", "/").replace("%2f", "/");
        (!path.contains('%') && ours(&self.login, &self.repo, &path)).then_some(path)
    }

    fn body(call: &Call) -> Value {
        call.body
            .as_deref()
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .unwrap_or(Value::Null)
    }

    fn allows(&self, call: &Call) -> bool {
        if call.url.host() != "gitlab.com" || !call.url.secure() || call.url.port() != 443 {
            return false;
        }
        let (path, query) = call
            .url
            .path()
            .split_once('?')
            .unwrap_or((call.url.path(), ""));
        let lower = call.url.path().to_ascii_lowercase();
        if lower.contains("..") || lower.contains("%2e") || lower.contains("%25") {
            return false;
        }
        let caller = self.caller(call);
        if let Some(rest) = path.strip_prefix("/oauth/") {
            // The device flow, with the built-in application.
            let form = call
                .body
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            return caller == Caller::Nobody
                && call.method == Method::Post
                && matches!(rest, "authorize_device" | "token")
                && form
                    .split('&')
                    .any(|pair| pair == format!("client_id={GITLAB_CLIENT_ID}"));
        }
        let confirmed = *self.confirmed.lock().unwrap();
        if path == "/api/graphql" {
            return caller == Caller::Account
                && confirmed
                && call.method == Method::Post
                && graphql_is_ours(&self.login, &self.repo, &Self::body(call));
        }
        let Some(rest) = path.strip_prefix("/api/v4/") else {
            return false;
        };
        let segments: Vec<&str> = rest.split('/').collect();
        match caller {
            Caller::Nobody => false,
            Caller::Stranger => {
                call.method == Method::Get
                    && matches!(
                        segments.as_slice(),
                        ["user"] | ["personal_access_tokens", "self"]
                    )
            }
            Caller::Account => {
                if call.method == Method::Get && segments == ["user"] {
                    true
                } else if !confirmed {
                    false
                } else {
                    self.account_allows(call, &segments, query)
                }
            }
        }
    }

    fn account_allows(&self, call: &Call, segments: &[&str], query: &str) -> bool {
        let body = || Self::body(call);
        let named = |key: &str, test: &dyn Fn(&str) -> bool| body()[key].as_str().is_some_and(test);
        match (call.method, segments) {
            (Method::Get, ["personal_access_tokens", "self"])
            | (Method::Get, ["user", "keys"])
            | (Method::Get, ["projects"])
            | (Method::Get, ["namespaces"])
            | (Method::Get, ["avatar"]) => true,
            (Method::Post, ["user", "keys"]) => named("title", &|title| title == self.key_title()),
            (Method::Delete, ["user", "keys", id]) => id
                .parse::<u64>()
                .is_ok_and(|id| self.keys.lock().unwrap().contains(&id)),
            // A project of the test's name in the account's namespace.
            (Method::Post, ["projects"]) => {
                body()["namespace_id"].is_null()
                    && named("name", &|name| name.starts_with(self.repo.as_str()))
                    && named("path", &|path| path.starts_with(self.repo.as_str()))
            }
            (Method::Get, ["namespaces", namespace]) => namespace.eq_ignore_ascii_case(&self.login),
            // A user, by the account's own login or one nobody has.
            (Method::Get, ["users"]) => query_field(query, "username").is_some_and(|name| {
                name.eq_ignore_ascii_case(&self.login) || name.starts_with("bitbraid-live-nobody")
            }),
            // To-do items of a test project; or the account's pending
            // ones about issues, for the inbox, of which only the test
            // projects' are handed on.
            (Method::Get, ["todos"]) => match query_field(query, "project_id") {
                Some(id) => id
                    .parse::<u64>()
                    .is_ok_and(|id| self.projects.lock().unwrap().contains_key(&id)),
                None => {
                    query_field(query, "type") == Some("Issue")
                        && query_field(query, "state") == Some("pending")
                }
            },
            // The inbox's lists of all projects: the account's own open
            // merge requests and issues, of which only the test projects'
            // are handed on.
            (Method::Get, ["merge_requests"] | ["issues"]) => {
                query_field(query, "scope") == Some("all")
                    && query_field(query, "state") == Some("opened")
                    && ["author_username", "assignee_username", "reviewer_username"]
                        .iter()
                        .any(|key| {
                            query_field(query, key)
                                .is_some_and(|name| name.eq_ignore_ascii_case(&self.login))
                        })
            }
            (method, ["projects", project, rest @ ..]) => {
                let Some(path) = self.project_of(project) else {
                    return false;
                };
                match (method, rest) {
                    (Method::Delete, []) => removable(&self.repo, &path),
                    // A fork in the account's namespace under a test name.
                    (Method::Post, ["fork"]) => {
                        let body = body();
                        body["namespace_id"].is_null()
                            && body["namespace_path"]
                                .as_str()
                                .is_none_or(|namespace| namespace.eq_ignore_ascii_case(&self.login))
                            && body["path"]
                                .as_str()
                                .is_none_or(|name| name.starts_with(self.repo.as_str()))
                    }
                    _ => true,
                }
            }
            _ => false,
        }
    }

    /// Learn the ids of test projects and keys an answer names.
    fn learn(&self, value: &Value) {
        match value {
            Value::Object(object) => {
                if let (Some(id), Some(path)) = (
                    object.get("id").and_then(Value::as_u64),
                    object.get("path_with_namespace").and_then(Value::as_str),
                ) && ours(&self.login, &self.repo, path)
                {
                    self.projects.lock().unwrap().insert(id, path.to_owned());
                }
                if let (Some(id), Some(title)) = (
                    object.get("id").and_then(Value::as_u64),
                    object.get("title").and_then(Value::as_str),
                ) && title == self.key_title()
                    && object.contains_key("key")
                {
                    self.keys.lock().unwrap().insert(id);
                }
                object.values().for_each(|inner| self.learn(inner));
            }
            Value::Array(items) => items.iter().for_each(|item| self.learn(item)),
            _ => {}
        }
    }

    /// A list of all projects' merge requests, issues or to-do items with
    /// only the test projects' in it; what was taken out is noted by the
    /// names of its project.
    fn only_ours_found(&self, answer: &mut Value) -> bool {
        let Value::Array(items) = answer else {
            return false;
        };
        let before = items.len();
        let mut hidden = self.hidden.lock().unwrap();
        items.retain(|item| {
            let path = item["project"]["path_with_namespace"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| {
                    item["references"]["full"]
                        .as_str()
                        .and_then(|full| full.split(['!', '#']).next())
                        .map(str::to_owned)
                });
            let keep = path
                .as_deref()
                .is_some_and(|path| ours(&self.login, &self.repo, path));
            if !keep && let Some(path) = path {
                for part in path.split('/') {
                    if !part.eq_ignore_ascii_case(&self.login) && part.len() >= 3 {
                        hidden.insert(part.to_owned());
                    }
                }
                hidden.insert(path);
            }
            keep
        });
        items.len() != before
    }

    /// A list of projects or namespaces with only the test's in it; what
    /// was taken out is noted by its names.
    fn only_ours(&self, segments: &[&str], answer: &mut Value) -> bool {
        let Value::Array(items) = answer else {
            return false;
        };
        let before = items.len();
        let mut hidden = self.hidden.lock().unwrap();
        items.retain(|item| {
            let keep = match segments {
                ["projects"] => item["path_with_namespace"]
                    .as_str()
                    .is_some_and(|path| ours(&self.login, &self.repo, path)),
                _ => item["full_path"]
                    .as_str()
                    .is_some_and(|path| path.eq_ignore_ascii_case(&self.login)),
            };
            if !keep {
                for key in ["name", "path", "full_path", "path_with_namespace"] {
                    if let Some(text) = item[key].as_str() {
                        hidden.insert(text.to_owned());
                    }
                }
                for key in ["name", "path", "full_path"] {
                    if let Some(text) = item["namespace"][key].as_str()
                        && !text.eq_ignore_ascii_case(&self.login)
                    {
                        hidden.insert(text.to_owned());
                    }
                }
            }
            keep
        });
        items.len() != before
    }
}

impl Fetch for Guard {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if !self.allows(call) {
            return Err(FetchError::Address("refused by the live test's guard"));
        }
        let mut response = self.inner.send(call, cancel)?;
        let path = call.url.path().split('?').next().unwrap_or_default();
        let mut answer = serde_json::from_slice::<Value>(&response.body).unwrap_or(Value::Null);
        let caller = self.caller(call);
        // Whose the token is, before anything else.
        if caller == Caller::Account
            && call.method == Method::Get
            && path == "/api/v4/user"
            && response.status == 200
        {
            let me = answer["username"].as_str() == Some(self.login.as_str());
            *self.confirmed.lock().unwrap() = me;
        }
        if caller == Caller::Account {
            let segments: Vec<&str> = path
                .strip_prefix("/api/v4/")
                .unwrap_or_default()
                .split('/')
                .collect();
            if call.method == Method::Get
                && matches!(segments.as_slice(), ["projects"] | ["namespaces"])
                && self.only_ours(&segments, &mut answer)
            {
                response.body = serde_json::to_vec(&answer).unwrap();
            }
            let all_projects = match segments.as_slice() {
                ["merge_requests"] | ["issues"] => true,
                ["todos"] => !call.url.path().contains("project_id="),
                _ => false,
            };
            if call.method == Method::Get && all_projects && self.only_ours_found(&mut answer) {
                response.body = serde_json::to_vec(&answer).unwrap();
            }
            self.learn(&answer);
        }
        Ok(response)
    }
}

/// The guarded API as the test account, for what the test makes.
#[derive(Clone)]
struct Api {
    guard: Arc<Guard>,
    token: Secret,
}

impl Api {
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = Url::parse(&format!("{SITE}/api/v4{path}")).unwrap();
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
            .guard
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
            value.get("message").cloned().unwrap_or(value.clone())
        );
        value
    }
}

#[test]
fn the_guard_refuses_what_is_not_the_tests() {
    let guard = Guard::with_repo("test-main", Some(Secret::new("main-token")), DEFAULT_REPO);
    let call = |token: Option<&str>, method: Method, address: &str, body: Option<Value>| {
        let mut call = Call::new(
            method,
            Url::parse(address).unwrap(),
            Limits {
                connect: Duration::from_secs(1),
                total: Duration::from_secs(1),
                body: 1,
            },
        );
        if let Some(token) = token {
            call = call.with_header("Authorization", &format!("Bearer {token}"));
        }
        if let Some(body) = body {
            call = call.with_body("application/json", body.to_string().into_bytes());
        }
        call
    };
    let api = |method: Method, path: &str, body: Option<Value>| {
        guard.allows(&call(
            Some("main-token"),
            method,
            &format!("https://gitlab.com/api/v4{path}"),
            body,
        ))
    };
    // Before the token's `GET /user` said whose it is, nothing else.
    assert!(api(Method::Get, "/user", None));
    assert!(!api(
        Method::Get,
        "/projects/test-main%2Fbitbraid-live",
        None
    ));
    *guard.confirmed.lock().unwrap() = true;
    guard
        .projects
        .lock()
        .unwrap()
        .insert(41, "test-main/bitbraid-live".to_owned());
    guard
        .projects
        .lock()
        .unwrap()
        .insert(42, "test-main/bitbraid-live-made-user".to_owned());
    for (method, path, body) in [
        (Method::Get, "/personal_access_tokens/self", None),
        (Method::Get, "/user/keys", None),
        (
            Method::Post,
            "/user/keys",
            Some(json!({ "title": "bitbraid-live test key", "key": "ssh-ed25519 AAAA" })),
        ),
        (Method::Get, "/projects?owned=true&per_page=30", None),
        (Method::Get, "/projects?starred=true", None),
        (Method::Get, "/namespaces?per_page=100", None),
        (Method::Get, "/namespaces/test-main", None),
        (
            Method::Get,
            "/avatar?email=a%40example.invalid&size=64",
            None,
        ),
        (Method::Get, "/users?username=test-main", None),
        (
            Method::Get,
            "/users?username=bitbraid-live-nobody-at-all",
            None,
        ),
        (Method::Get, "/todos?project_id=41&type=Issue", None),
        (
            Method::Post,
            "/projects",
            Some(json!({ "name": "bitbraid-live-made-user", "path": "bitbraid-live-made-user" })),
        ),
        (Method::Get, "/projects/test-main%2Fbitbraid-live", None),
        (
            Method::Get,
            "/projects/test-main%2Fbitbraid-live/merge_requests?state=all",
            None,
        ),
        (
            Method::Put,
            "/projects/test-main%2Fbitbraid-live-writes/merge_requests/3/merge",
            None,
        ),
        (Method::Post, "/projects/41/statuses/abc", None),
        (Method::Get, "/projects/41", None),
        (
            Method::Post,
            "/projects/test-main%2Fbitbraid-live/fork",
            Some(
                json!({ "name": "bitbraid-live-fork", "path": "bitbraid-live-fork",
                         "namespace_path": "test-main" }),
            ),
        ),
        (
            Method::Post,
            "/projects/test-main%2Fbitbraid-live/fork",
            Some(json!({})),
        ),
        (Method::Delete, "/projects/42", None),
        (
            Method::Delete,
            "/projects/test-main%2Fbitbraid-live-made-user",
            None,
        ),
    ] {
        assert!(api(method, path, body), "{path}");
    }
    for (method, path, body) in [
        (Method::Put, "/user", None),
        (Method::Get, "/user/emails", None),
        (Method::Get, "/user/keys/1", None),
        (Method::Delete, "/user/keys/1", None),
        (
            Method::Post,
            "/user/keys",
            Some(json!({ "title": "another key", "key": "ssh-ed25519 AAAA" })),
        ),
        (Method::Get, "/groups", None),
        (Method::Get, "/groups/someone-else", None),
        (
            Method::Post,
            "/groups",
            Some(json!({ "path": "bitbraid-live" })),
        ),
        (Method::Get, "/namespaces/someone-else", None),
        (Method::Get, "/users?username=someone-else", None),
        (Method::Get, "/users/1", None),
        (Method::Get, "/todos", None),
        (Method::Get, "/todos?project_id=7", None),
        (Method::Get, "/events", None),
        (
            Method::Post,
            "/projects",
            Some(json!({ "name": "dotfiles" })),
        ),
        (
            Method::Post,
            "/projects",
            Some(
                json!({ "name": "bitbraid-live-made-user", "path": "bitbraid-live-made-user",
                         "namespace_id": 7 }),
            ),
        ),
        (Method::Get, "/projects/someone%2Fbitbraid-live", None),
        (Method::Get, "/projects/test-main%2Fother", None),
        (
            Method::Get,
            "/projects/group%2Ftest-main%2Fbitbraid-live",
            None,
        ),
        (Method::Get, "/projects/7", None),
        (
            Method::Get,
            "/projects/test-main%2Fbitbraid-live%2F..%2Fx",
            None,
        ),
        (
            Method::Get,
            "/projects/test-main%2Fbitbraid-live/../../user/emails",
            None,
        ),
        (
            Method::Get,
            "/projects/test-main%2Fbitbraid-live/%2E%2E/x",
            None,
        ),
        (Method::Delete, "/projects/test-main%2Fbitbraid-live", None),
        (Method::Delete, "/projects/41", None),
        (
            Method::Delete,
            "/projects/test-main%2Fbitbraid-live-writes",
            None,
        ),
        (
            Method::Post,
            "/projects/test-main%2Fbitbraid-live/fork",
            Some(json!({ "namespace_path": "someone-else" })),
        ),
        (
            Method::Post,
            "/projects/test-main%2Fbitbraid-live/fork",
            Some(json!({ "namespace_id": 7 })),
        ),
        (
            Method::Post,
            "/projects/test-main%2Fbitbraid-live/fork",
            Some(json!({ "path": "dotfiles" })),
        ),
    ] {
        assert!(!api(method, path, body), "{path}");
    }
    // A key of the test's title, once an answer named it, may go.
    guard.learn(
        &json!([{ "id": 77, "title": "bitbraid-live test key", "key": "ssh-ed25519 AAAA" }]),
    );
    assert!(api(Method::Delete, "/user/keys/77", None));
    // A project an answer named is known by its id; not another's.
    guard.learn(&json!({ "id": 43, "path_with_namespace": "test-main/bitbraid-live-fork" }));
    guard.learn(&json!({ "id": 44, "path_with_namespace": "someone/bitbraid-live" }));
    assert!(api(Method::Get, "/projects/43/merge_requests", None));
    assert!(!api(Method::Get, "/projects/44", None));
    // Elsewhere: another host, plain HTTP, another port.
    for address in [
        "https://example.org/api/v4/user",
        "http://gitlab.com/api/v4/user",
        "https://gitlab.com:8443/api/v4/user",
        "https://gitlab.com/api/v3/user",
        "https://gitlab.com/someone/bitbraid-live",
    ] {
        assert!(
            !guard.allows(&call(Some("main-token"), Method::Get, address, None)),
            "{address}"
        );
    }
    // GraphQL: one question about a test project.
    let graphql = |query: &str, variables: Value| {
        guard.allows(&call(
            Some("main-token"),
            Method::Post,
            "https://gitlab.com/api/graphql",
            Some(json!({ "query": query, "variables": variables })),
        ))
    };
    let merges = "query($path: ID!, $iids: [String!]) { project(fullPath: $path) { \
        mergeRequests(iids: $iids, first: 100) { nodes { iid sourceProject { fullPath } \
        reviewers { nodes { username } } } } } }";
    assert!(graphql(
        merges,
        json!({ "path": "test-main/bitbraid-live", "iids": ["1"] })
    ));
    for (query, variables) in [
        (merges, json!({ "path": "someone/bitbraid-live" })),
        (merges, json!({ "path": "test-main/other" })),
        (
            "query { project(fullPath: \"someone/x\") { id } }",
            json!({}),
        ),
        (
            "query { currentUser { emails { nodes { email } } } }",
            json!({}),
        ),
        (
            "query($path: ID!) { project(fullPath: $path) { id } user(username: $path) { id } }",
            json!({ "path": "test-main/bitbraid-live" }),
        ),
        (
            "query($path: ID!) { project(fullPath: $path) { group { projects { nodes { id } } } } }",
            json!({ "path": "test-main/bitbraid-live" }),
        ),
        (
            "mutation($path: ID!) { projectDelete(input: {fullPath: $path}) { errors } }",
            json!({ "path": "test-main/bitbraid-live" }),
        ),
    ] {
        assert!(!graphql(query, variables.clone()), "{query} {variables}");
    }
    // The device flow: without a token, with the built-in application.
    let site = |path: &str, token: Option<&str>, form: &str| {
        let mut call = Call::new(
            Method::Post,
            Url::parse(&format!("https://gitlab.com{path}")).unwrap(),
            Limits {
                connect: Duration::from_secs(1),
                total: Duration::from_secs(1),
                body: 1,
            },
        )
        .with_body(
            "application/x-www-form-urlencoded",
            form.as_bytes().to_vec(),
        );
        if let Some(token) = token {
            call = call.with_header("Authorization", &format!("Bearer {token}"));
        }
        guard.allows(&call)
    };
    let form = format!("client_id={GITLAB_CLIENT_ID}&scope=api%20read_user");
    assert!(site("/oauth/authorize_device", None, &form));
    assert!(site("/oauth/token", None, &form));
    assert!(!site("/oauth/authorize_device", None, "client_id=another"));
    assert!(!site("/oauth/token", Some("main-token"), &form));
    assert!(!site("/oauth/applications", None, &form));
    assert!(!site("/users/sign_in", None, &form));
    // A token the guard does not know: who it is, and nothing else; no
    // token: nothing at the API.
    let stranger = |path: &str| {
        guard.allows(&call(
            Some("stranger"),
            Method::Get,
            &format!("https://gitlab.com/api/v4{path}"),
            None,
        ))
    };
    assert!(stranger("/user"));
    assert!(stranger("/personal_access_tokens/self"));
    assert!(!stranger("/projects?owned=true"));
    assert!(!stranger("/projects/test-main%2Fbitbraid-live"));
    assert!(!guard.allows(&call(
        None,
        Method::Get,
        "https://gitlab.com/api/v4/user",
        None
    )));
}

#[test]
fn the_guard_hands_on_lists_with_only_the_tests_things() {
    let guard = Guard::with_repo("test-main", None, DEFAULT_REPO);
    let mut projects = json!([
        { "id": 1, "name": "bitbraid-live", "path": "bitbraid-live",
          "path_with_namespace": "test-main/bitbraid-live",
          "namespace": { "name": "Test Main", "path": "test-main", "full_path": "test-main" } },
        { "id": 2, "name": "Private-thing", "path": "private-thing",
          "path_with_namespace": "a-group/private-thing",
          "namespace": { "name": "A-group", "path": "a-group", "full_path": "a-group" } },
    ]);
    assert!(guard.only_ours(&["projects"], &mut projects));
    assert_eq!(projects.as_array().unwrap().len(), 1);
    assert_eq!(projects[0]["path"], "bitbraid-live");
    let mut namespaces = json!([
        { "id": 3, "name": "Test Main", "path": "test-main", "full_path": "test-main", "kind": "user" },
        { "id": 4, "name": "A-group", "path": "a-group", "full_path": "a-group", "kind": "group" },
    ]);
    assert!(guard.only_ours(&["namespaces"], &mut namespaces));
    assert_eq!(namespaces.as_array().unwrap().len(), 1);
    let hidden = guard.hidden.lock().unwrap().clone();
    for name in [
        "Private-thing",
        "private-thing",
        "a-group/private-thing",
        "A-group",
        "a-group",
    ] {
        assert!(hidden.contains(name), "{name} in {hidden:?}");
    }
    assert!(!hidden.contains("bitbraid-live") && !hidden.contains("test-main"));
    // Nothing taken out: the answer is handed on as it came.
    let mut only = json!([{ "path_with_namespace": "test-main/bitbraid-live-writes" }]);
    assert!(!guard.only_ours(&["projects"], &mut only));
}

#[test]
fn the_guard_lets_the_inboxs_lists_through_with_only_the_tests_things() {
    let guard = Guard::with_repo("test-main", Some(Secret::new("main-token")), DEFAULT_REPO);
    *guard.confirmed.lock().unwrap() = true;
    let get = |path: &str| {
        let call = Call::new(
            Method::Get,
            Url::parse(&format!("https://gitlab.com/api/v4/{path}")).unwrap(),
            Limits {
                connect: Duration::from_secs(1),
                total: Duration::from_secs(1),
                body: 1,
            },
        )
        .with_header("Authorization", "Bearer main-token");
        guard.allows(&call)
    };
    for path in [
        "merge_requests?scope=all&state=opened&author_username=test-main&per_page=50",
        "merge_requests?scope=all&state=opened&reviewer_username=test-main&per_page=50",
        "issues?scope=all&state=opened&assignee_username=test-main&per_page=50",
        "todos?type=Issue&state=pending&per_page=50",
    ] {
        assert!(get(path), "{path}");
    }
    for path in [
        "merge_requests?scope=all&state=opened&author_username=someone&per_page=50",
        "merge_requests?scope=all&state=closed&author_username=test-main",
        "merge_requests?state=opened&author_username=test-main",
        "issues?scope=all&state=opened",
        "todos?type=MergeRequest&state=pending",
        "todos?type=Issue&state=done",
    ] {
        assert!(!get(path), "{path}");
    }
    let mut found = json!([
        { "iid": 1, "references": { "full": "test-main/bitbraid-live!1" } },
        { "iid": 2, "references": { "full": "a-group/private-thing!2" } },
        { "iid": 3 },
        { "action_name": "mentioned", "project": { "path_with_namespace": "test-main/bitbraid-live-writes" } },
    ]);
    assert!(guard.only_ours_found(&mut found));
    assert_eq!(found.as_array().unwrap().len(), 2);
    let hidden = guard.hidden.lock().unwrap().clone();
    assert!(hidden.contains("a-group/private-thing") && hidden.contains("private-thing"));
    assert!(!hidden.contains("test-main"));
}

/// What every live test starts from: the guard, the account's API, and
/// what may not be in a recording. Made once per run.
struct World {
    guard: Arc<Guard>,
    api: Api,
    /// The login, id, name, addresses and other facts of the account.
    real: Vec<String>,
}

fn world() -> Option<&'static World> {
    static WORLD: OnceLock<Option<World>> = OnceLock::new();
    WORLD
        .get_or_init(|| {
            let login = login_of_this_run()?;
            let _ = LOGIN.set(login.clone());
            let token = token().expect("the test account's token");
            let guard = Arc::new(Guard::new(&login, Some(token.clone())));
            let api = Api {
                guard: guard.clone(),
                token,
            };
            // Whose the token is, before anything else.
            let me = api.ok(Method::Get, "/user", None);
            assert_eq!(
                me["username"], login,
                "the token is not its keychain item's login's: stopping"
            );
            let mut real = vec![login.clone()];
            if let Some(id) = me["id"].as_u64() {
                real.push(id.to_string());
            }
            for key in ["name", "email", "public_email", "commit_email"] {
                if let Some(value) = me[key].as_str()
                    && value.trim().len() >= 3
                {
                    real.push(value.trim().to_owned());
                }
            }
            // Each part of the name, and the picture's digest of the
            // address.
            if let Some(name) = me["name"].as_str() {
                real.extend(
                    name.split_whitespace()
                        .filter(|part| part.chars().count() >= 3)
                        .map(str::to_owned),
                );
            }
            if let Some(avatar) = me["avatar_url"].as_str()
                && let Some(digest) = avatar
                    .split(['/', '?'])
                    .find(|part| part.len() >= 32 && part.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                real.push(digest.to_owned());
            }
            // Where the token was used from.
            let mine = api.ok(Method::Get, "/personal_access_tokens/self", None);
            for address in mine["last_used_ips"]
                .as_array()
                .cloned()
                .unwrap_or_default()
            {
                if let Some(address) = address.as_str() {
                    real.push(address.to_owned());
                }
            }
            Some(World { guard, api, real })
        })
        .as_ref()
}

/// The world of a test that asked [`world`] first.
fn the_world() -> &'static World {
    world().expect("the live tests are asked for")
}

/// One live test at a time: they share the projects.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// `/projects/<login>%2F<name>`.
fn at(name: &str) -> String {
    format!("/projects/{}", encode(&format!("{}/{name}", login())))
}

/// A public project of the account, made when missing: its id.
fn project(api: &Api, name: &str) -> u64 {
    let (status, found) = api.call(Method::Get, &at(name), None);
    if status == 200 {
        return found["id"].as_u64().unwrap();
    }
    api.ok(
        Method::Post,
        "/projects",
        Some(json!({ "name": name, "path": name, "visibility": "public",
                     "initialize_with_readme": true, "default_branch": "main",
                     "description": "Synthetic test data of BitBraid's live tests" })),
    )["id"]
        .as_u64()
        .unwrap()
}

/// A branch with one commit of its own, by the test's identity, made when
/// missing.
fn branch_with_commit(api: &Api, project: u64, branch: &str, from: &str, file: &str, text: &str) {
    let (status, _) = api.call(
        Method::Get,
        &format!("/projects/{project}/repository/branches/{}", encode(branch)),
        None,
    );
    if status == 200 {
        return;
    }
    commit(api, project, branch, Some(from), file, text);
}

/// A commit of one new file on `branch` (made from `from` when given), by
/// the test's identity.
fn commit(api: &Api, project: u64, branch: &str, from: Option<&str>, file: &str, text: &str) {
    let mut body = json!({ "branch": branch, "commit_message": format!("Write {file}"),
                           "author_name": IDENTITY.0, "author_email": IDENTITY.1,
                           "actions": [ { "action": "create", "file_path": file,
                                          "content": text } ] });
    if let Some(from) = from {
        body["start_branch"] = json!(from);
    }
    let (status, said) = api.call(
        Method::Post,
        &format!("/projects/{project}/repository/commits"),
        Some(body),
    );
    // A branch merged and removed before: its file is in `from` already.
    assert!(
        (200..300).contains(&status) || said.to_string().contains("already exists"),
        "{branch}: {status} {said}"
    );
}

/// The head commit of a branch.
fn head(api: &Api, project: u64, branch: &str) -> String {
    api.ok(
        Method::Get,
        &format!("/projects/{project}/repository/branches/{}", encode(branch)),
        None,
    )["commit"]["id"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The merge request from `source` (of `from_project`) into `target` of
/// `project`, made when missing: its iid.
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
        &format!(
            "/projects/{project}/merge_requests?state=all&source_branch={}",
            encode(source)
        ),
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

/// The issue called `title`, made when missing: its iid.
fn issue(api: &Api, project: u64, title: &str, extra: Value) -> u64 {
    let (_, found) = api.call(
        Method::Get,
        &format!("/projects/{project}/issues?state=all&per_page=100"),
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

/// The labels `bug` and `ui` and the milestone `v1` of a project, made
/// when missing: the milestone's id.
fn labels_and_milestone(api: &Api, project: u64) -> u64 {
    for (name, color, description) in [
        ("bug", "#d9534f", "Something is broken"),
        ("ui", "#428bca", "How it looks"),
    ] {
        // There already: a conflict, which is fine.
        let (status, said) = api.call(
            Method::Post,
            &format!("/projects/{project}/labels"),
            Some(json!({ "name": name, "color": color, "description": description })),
        );
        assert!(matches!(status, 201 | 409), "a label: {status} {said}");
    }
    let (_, milestones) = api.call(
        Method::Get,
        &format!("/projects/{project}/milestones?title=v1"),
        None,
    );
    match milestones.as_array().and_then(|all| all.first()) {
        Some(found) => found["id"].as_u64().unwrap(),
        None => api.ok(
            Method::Post,
            &format!("/projects/{project}/milestones"),
            Some(json!({ "title": "v1", "due_date": "2026-12-31" })),
        )["id"]
            .as_u64()
            .unwrap(),
    }
}

/// A commit status, set when the commit has none of that name.
fn status_once(api: &Api, project: u64, sha: &str, branch: &str, name: &str, state: &str) {
    let (_, statuses) = api.call(
        Method::Get,
        &format!("/projects/{project}/repository/commits/{sha}/statuses?all=true&per_page=100"),
        None,
    );
    if statuses
        .as_array()
        .is_some_and(|all| all.iter().any(|status| status["name"] == name))
    {
        return;
    }
    api.ok(
        Method::Post,
        &format!("/projects/{project}/statuses/{sha}"),
        Some(json!({ "state": state, "name": name, "ref": branch,
                     "description": format!("{name} says {state}"),
                     "target_url": format!("https://example.invalid/{name}") })),
    );
}

/// Wait until GitLab has worked out a merge request's diff: its
/// `diff_refs`.
fn diff_refs(api: &Api, project: u64, iid: u64) -> Value {
    let deadline = Instant::now() + Duration::from_secs(90);
    loop {
        let merge = api.ok(
            Method::Get,
            &format!("/projects/{project}/merge_requests/{iid}"),
            None,
        );
        if merge["diff_refs"]["head_sha"].is_string() {
            return merge["diff_refs"].clone();
        }
        assert!(Instant::now() < deadline, "!{iid} has no diff");
        std::thread::sleep(Duration::from_secs(2));
    }
}

/// Wait until a fork is copied.
fn copied(api: &Api, project: u64) {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let state = api.ok(Method::Get, &format!("/projects/{project}"), None)["import_status"]
            .as_str()
            .unwrap_or("finished")
            .to_owned();
        if matches!(state.as_str(), "finished" | "none") {
            return;
        }
        assert!(state != "failed", "the fork was not copied");
        assert!(Instant::now() < deadline, "the fork was not copied in time");
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// What the reads are tested on, in the account's `<repo>`.
#[derive(Debug, Clone)]
struct Reads {
    project: u64,
    /// Open, labelled, assigned, in the milestone, the account asked to
    /// review it, with conversations on its lines (one resolved) and on
    /// the whole, its checks failing.
    topic: u64,
    /// Open, the account (asked to review) asking for changes.
    changes: u64,
    /// A draft, the account asked to review and silent.
    draft: u64,
    /// Two stacked.
    stack: [u64; 2],
    merged: u64,
    closed: u64,
    /// From the fork.
    fork: u64,
    /// The heads of `topic`, `draft` and the first of the stack.
    shas: [String; 3],
    /// Open, labelled, assigned, in the milestone, with comments, one
    /// naming the account.
    issue: u64,
    closed_issue: u64,
    /// Open, nothing on it.
    plain_issue: u64,
    /// What GitLab answered when the author approved their own merge
    /// request: `None` when it took it.
    self_approval: Option<String>,
}

/// Make what the reads are tested on, when missing; once per run.
fn reads() -> Reads {
    static READS: Mutex<Option<Reads>> = Mutex::new(None);
    let mut made = READS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(reads) = made.as_ref() {
        return reads.clone();
    }
    let reads = setup_reads(the_world());
    *made = Some(reads.clone());
    reads
}

fn setup_reads(world: &World) -> Reads {
    let api = &world.api;
    let me = api.ok(Method::Get, "/user", None)["id"].as_u64().unwrap();
    let project = project(api, &REPO);
    let milestone = labels_and_milestone(api, project);
    // Starred already: 304, which is fine.
    let (status, _) = api.call(Method::Post, &format!("/projects/{project}/star"), None);
    assert!(matches!(status, 201 | 304), "starring: {status}");
    // The fork, in the same namespace under another name, with a branch
    // of its own.
    let (status, found) = api.call(Method::Get, &at(&FORK), None);
    let fork = if status == 200 {
        found["id"].as_u64().unwrap()
    } else {
        api.ok(
            Method::Post,
            &format!("/projects/{project}/fork"),
            Some(json!({ "name": FORK, "path": FORK, "namespace_path": login() })),
        )["id"]
            .as_u64()
            .unwrap()
    };
    copied(api, fork);
    // The merge requests, in an order that gives them the same numbers
    // in the first run of a new project.
    let one = |branch: &str, file: &str, title: &str| {
        branch_with_commit(
            api,
            project,
            branch,
            "main",
            file,
            &format!("a line of {file}\nanother line\n"),
        );
        merge_request(api, project, project, branch, "main", title)
    };
    let topic = one("bb-topic", "bb-topic.txt", "Add a file to review");
    let mr = |iid: u64| format!("/projects/{project}/merge_requests/{iid}");
    api.ok(
        Method::Put,
        &mr(topic),
        Some(json!({ "labels": "bug", "milestone_id": milestone,
                     "assignee_ids": [me], "reviewer_ids": [me] })),
    );
    let (_, discussions) = api.call(
        Method::Get,
        &format!("{}/discussions?per_page=100", mr(topic)),
        None,
    );
    let on_lines = discussions
        .as_array()
        .map(|all| {
            all.iter()
                .filter(|discussion| discussion["notes"][0]["position"].is_object())
                .count()
        })
        .unwrap_or(0);
    if on_lines < 2 {
        let refs = diff_refs(api, project, topic);
        for (line, body, resolve) in [
            (1, "This line could say more.", true),
            (2, "Is this the right file for it?", false),
        ] {
            let made = api.ok(
                Method::Post,
                &format!("{}/discussions", mr(topic)),
                Some(json!({ "body": body, "position": {
                    "position_type": "text", "base_sha": refs["base_sha"],
                    "start_sha": refs["start_sha"], "head_sha": refs["head_sha"],
                    "new_path": "bb-topic.txt", "new_line": line } })),
            );
            let id = made["id"].as_str().unwrap().to_owned();
            if resolve {
                api.ok(
                    Method::Post,
                    &format!("{}/discussions/{id}/notes", mr(topic)),
                    Some(json!({ "body": "Done." })),
                );
                api.ok(
                    Method::Put,
                    &format!("{}/discussions/{id}?resolved=true", mr(topic)),
                    None,
                );
            }
        }
    }
    comment_once(
        api,
        project,
        &format!("merge_requests/{topic}"),
        "Looks good to me.",
    );
    // Approved by its author, when GitLab lets them.
    let approvals = api.ok(Method::Get, &format!("{}/approvals", mr(topic)), None);
    let approved = approvals["approved_by"]
        .as_array()
        .is_some_and(|all| all.iter().any(|by| by["user"]["id"] == me));
    let self_approval = if approved {
        None
    } else {
        let (status, said) = api.call(Method::Post, &format!("{}/approve", mr(topic)), None);
        (!(200..300).contains(&status)).then(|| format!("{status} {said}"))
    };
    // One the account, asked to review, wants changed.
    let changes = one("bb-changes", "bb-changes.txt", "Change the wording");
    let asked = api.ok(Method::Get, &format!("{}/reviewers", mr(changes)), None);
    if !asked.as_array().is_some_and(|all| {
        all.iter()
            .any(|reviewer| reviewer["state"] == "requested_changes")
    }) {
        api.ok(
            Method::Put,
            &mr(changes),
            Some(json!({ "reviewer_ids": [me] })),
        );
        api.ok(
            Method::Post,
            &format!("{}/draft_notes/bulk_publish", mr(changes)),
            Some(json!({ "reviewer_state": "requested_changes",
                         "note": "Please add a test." })),
        );
    }
    // A draft, the account asked to review.
    let draft = one("bb-draft", "bb-draft.txt", "Draft: Try another way");
    api.ok(
        Method::Put,
        &mr(draft),
        Some(json!({ "labels": "ui", "reviewer_ids": [me] })),
    );
    // Two stacked.
    let first = one("bb-stack-1", "bb-stack-1.txt", "First of a stack");
    branch_with_commit(
        api,
        project,
        "bb-stack-2",
        "bb-stack-1",
        "bb-stack-2.txt",
        "the second of a stack\n",
    );
    let second = merge_request(
        api,
        project,
        project,
        "bb-stack-2",
        "bb-stack-1",
        "Second of a stack",
    );
    // One merged, one closed.
    let merged = one("bb-merged", "bb-merged.txt", "Merged already");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let state = api.ok(Method::Get, &mr(merged), None)["state"]
            .as_str()
            .unwrap()
            .to_owned();
        if state == "merged" {
            break;
        }
        assert!(Instant::now() < deadline, "!{merged} was not merged");
        let _ = api.call(Method::Put, &format!("{}/merge", mr(merged)), None);
        std::thread::sleep(Duration::from_secs(3));
    }
    let closed = one("bb-closed", "bb-closed.txt", "Closed without merging");
    api.ok(
        Method::Put,
        &mr(closed),
        Some(json!({ "state_event": "close" })),
    );
    // From the fork.
    branch_with_commit(
        api,
        fork,
        "bb-fork-topic",
        "main",
        "bb-fork.txt",
        "a line from a fork\n",
    );
    let from_fork = merge_request(
        api,
        project,
        fork,
        "bb-fork-topic",
        "main",
        "A change from a fork",
    );
    // Statuses set on commits, as a service would: success and failure on
    // the first, one waiting on the draft, one running on the first of
    // the stack.
    let shas = [
        head(api, project, "bb-topic"),
        head(api, project, "bb-draft"),
        head(api, project, "bb-stack-1"),
    ];
    for (sha, branch, name, state) in [
        (&shas[0], "bb-topic", "bb-lint", "success"),
        (&shas[0], "bb-topic", "bb-test", "failed"),
        (&shas[1], "bb-draft", "bb-test", "pending"),
        (&shas[2], "bb-stack-1", "bb-test", "running"),
    ] {
        status_once(api, project, sha, branch, name, state);
    }
    // Issues.
    let issue_one = issue(
        api,
        project,
        "The button does nothing",
        json!({ "description": "Clicking it does nothing at all.", "labels": "bug",
                "milestone_id": milestone, "assignee_ids": [me] }),
    );
    comment_once(
        api,
        project,
        &format!("issues/{issue_one}"),
        "Which button do you mean?",
    );
    comment_once(
        api,
        project,
        &format!("issues/{issue_one}"),
        &format!("@{} the one at the top.", login()),
    );
    let closed_issue = issue(
        api,
        project,
        "Colours are off",
        json!({ "description": "Too bright.", "labels": "ui" }),
    );
    let _ = api.call(
        Method::Put,
        &format!("/projects/{project}/issues/{closed_issue}"),
        Some(json!({ "state_event": "close" })),
    );
    let plain_issue = issue(
        api,
        project,
        "Say what the menu is for",
        json!({ "description": "A line under it would do." }),
    );
    Reads {
        project,
        topic,
        changes,
        draft,
        stack: [first, second],
        merged,
        closed,
        fork: from_fork,
        shas,
        issue: issue_one,
        closed_issue,
        plain_issue,
        self_approval,
    }
}

/// The commands' services, through the guard and recorded when asked.
struct Live {
    dir: PathBuf,
    services: Arc<Services>,
    recorder: Option<Arc<Recorder>>,
    /// The account's id here, once connected.
    account: String,
}

impl Live {
    /// The services, the account connected.
    fn new(name: &str) -> Self {
        let world = the_world();
        let mut live = Self::bare(name, world.guard.clone(), true);
        let account = live.run(
            "account.add_token",
            json!({ "kind": "gitlab", "token": world.api.token.expose() }),
        );
        assert_eq!(account["login"], login());
        live.account = account["id"].as_str().unwrap().to_owned();
        live
    }

    /// The services through `guard`, no account connected, recorded when
    /// `record` and asked.
    fn bare(name: &str, guard: Arc<Guard>, record: bool) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bb-l-live-gitlab-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let guard: Arc<dyn Fetch> = guard;
        let recorder = record
            .then(|| {
                Recorder::from_environment(
                    guard.clone(),
                    name,
                    "Recorded from gitlab.com by tests/suite/live_gitlab_com.rs: the test \
                     account's public projects",
                )
            })
            .flatten()
            .map(Arc::new);
        let fetch: Arc<dyn Fetch> = match &recorder {
            Some(recorder) => recorder.clone(),
            None => guard,
        };
        let services = Arc::new(Services::new(fetch, Arc::new(MemoryStore::new()), &dir));
        Self {
            dir,
            services,
            recorder,
            account: String::new(),
        }
    }

    fn try_run(&self, id: &str, params: Value) -> Result<Value, String> {
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        bitbraid_integrations::registry()
            .run(id, &mut ctx, &params)
            .map_err(|err| err.to_string())
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
        let mut params = json!({ "account": self.account, "owner": login(), "name": name });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.try_run(id, params)
    }

    /// A command about the reads' project.
    fn reads(&self, id: &str, extra: Value) -> Value {
        self.on(&REPO, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn finish(&self) {
        if let Some(recorder) = &self.recorder {
            let world = the_world();
            let mut real = world.real.clone();
            // What the guard took out of lists.
            real.extend(world.guard.hidden.lock().unwrap().iter().cloned());
            // More that may not be in a recording, given when the test
            // runs and kept in no file.
            if let Ok(more) = std::env::var("BITBRAID_RECORD_FORBID") {
                real.extend(
                    more.split(',')
                        .map(|word| word.trim().to_owned())
                        .filter(|word| word.len() >= 3),
                );
            }
            // A name of the projects of one's own is recorded as the one
            // the recordings use, and has to be gone.
            let mut named = vec![(login(), "user-1")];
            if live_repo() != DEFAULT_REPO {
                named.push((live_repo(), DEFAULT_REPO));
                real.push(live_repo().to_owned());
            }
            let real: Vec<&str> = real.iter().map(String::as_str).collect();
            let path = recorder.finish_named(&named, &real).unwrap();
            eprintln!("recorded {}", path.display());
        }
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

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_token_connects_and_the_merge_requests_are_listed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("add_token_and_pr_list");
    let accounts = live.run("account.list", json!({}));
    assert_eq!(accounts[0]["web"], SITE);
    assert_eq!(accounts[0]["api"], "https://gitlab.com/api/v4");
    assert_eq!(accounts[0]["scopes"], json!(["api", "read_user"]));
    assert_eq!(accounts[0]["sign_in"], "token");
    let listing = live.reads("pr.list", json!({ "state": "all" }));
    let topic = listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pull| pull["number"] == reads.topic)
        .unwrap();
    assert_eq!(topic["head_ref"], "bb-topic");
    assert_eq!(topic["author"]["login"], login());
    assert_eq!(topic["repo"], format!("{}/{REPO}", login()));
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn merge_requests_are_listed_by_every_filter_and_query() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_list");
    let all = live.reads("pr.list", json!({ "state": "all" }));
    let open = sorted(vec![
        reads.topic,
        reads.changes,
        reads.draft,
        reads.stack[0],
        reads.stack[1],
        reads.fork,
    ]);
    let found = numbers(&all);
    for number in open.iter().chain([&reads.merged, &reads.closed]) {
        assert!(found.contains(number), "!{number} in {found:?}");
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
    assert_eq!(topic["labels"][0]["color"], "d9534f");
    assert_eq!(topic["assignees"][0]["login"], login());
    assert_eq!(topic["reviewers"][0]["user"]["login"], login());
    assert_eq!(topic["reviewers"][0]["state"], "approved");
    assert_eq!(topic["ci"], "failure");
    assert!(topic["comments"].as_u64().unwrap() >= 4, "{topic}");
    let changes = row(reads.changes);
    assert_eq!(changes["reviewers"][0]["state"], "changes_requested");
    assert_eq!(changes["ci"], "none");
    let draft = row(reads.draft);
    assert_eq!(draft["draft"], true);
    assert_eq!(draft["ci"], "pending");
    assert_eq!(draft["reviewers"][0]["state"], "pending");
    assert_eq!(row(reads.stack[0])["ci"], "running");
    assert_eq!(row(reads.stack[1])["base_ref"], "bb-stack-1");
    assert_eq!(row(reads.merged)["state"], "merged");
    assert_eq!(row(reads.closed)["state"], "closed");
    let fork = row(reads.fork);
    assert_eq!(fork["head_repo"], format!("{}/{FORK}", login()));
    assert_eq!(fork["head_ref"], "bb-fork-topic");
    let list = |extra: Value| sorted(numbers(&live.reads("pr.list", extra)));
    assert_eq!(list(json!({})), open);
    assert_eq!(list(json!({ "filter": "mine" })), open);
    assert_eq!(list(json!({ "filter": "assigned" })), [reads.topic]);
    assert_eq!(
        list(json!({ "filter": "review_requested" })),
        sorted(vec![reads.topic, reads.changes, reads.draft])
    );
    assert_eq!(list(json!({ "state": "merged" })), [reads.merged]);
    assert_eq!(list(json!({ "state": "closed" })), [reads.closed]);
    assert_eq!(list(json!({ "query": "label:bug" })), [reads.topic]);
    assert_eq!(
        list(json!({ "query": "label:bug,ui" })),
        sorted(vec![reads.topic, reads.draft])
    );
    assert_eq!(list(json!({ "query": "draft:true" })), [reads.draft]);
    assert!(!list(json!({ "query": "-label:bug" })).contains(&reads.topic));
    assert_eq!(
        list(json!({ "query": "base:bb-stack-1" })),
        [reads.stack[1]]
    );
    assert_eq!(list(json!({ "query": "milestone:v1" })), [reads.topic]);
    assert_eq!(list(json!({ "query": "head:bb-fork-topic" })), [reads.fork]);
    assert_eq!(
        list(json!({ "query": format!("author:{}", login()) })),
        open
    );
    assert_eq!(
        list(json!({ "query": format!("review-requested:{} label:ui", login()) })),
        [reads.draft]
    );
    assert_eq!(list(json!({ "query": "stack" })), reads.stack.to_vec());
    let err = live
        .on(&REPO, "pr.list", json!({ "query": "involves:x" }))
        .unwrap_err();
    assert!(err.contains("involves"), "{err}");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_merge_request_is_shown_with_everything_its_view_shows() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_show");
    let show = |number: u64| live.reads("pr.show", json!({ "number": number }));
    let topic = show(reads.topic);
    assert_eq!(topic["pull"]["number"], reads.topic);
    assert_eq!(
        topic["body"],
        "Made by the live test: Add a file to review."
    );
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["pull"]["ci"], "failure");
    // The author approved it, and was asked to review.
    assert_eq!(reads.self_approval, None);
    assert_eq!(topic["pull"]["reviewers"][0]["state"], "approved");
    assert!(
        topic["reviews"]
            .as_array()
            .unwrap()
            .iter()
            .any(|review| review["state"] == "approved" && review["author"]["login"] == login()),
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
    assert_eq!(commits[0]["author_name"], IDENTITY.0);
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
        threads.iter().any(|thread| thread["path"].is_null()
            && thread["comments"][0]["body"] == "Looks good to me."),
        "{threads:?}"
    );
    assert_eq!(topic["merge_methods"], json!(["merge", "squash"]));
    assert_ne!(topic["mergeable"], "conflicts");
    let changes = show(reads.changes);
    assert_eq!(
        changes["pull"]["reviewers"][0]["state"],
        "changes_requested"
    );
    let draft = show(reads.draft);
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["pull"]["reviewers"][0]["state"], "pending");
    assert_eq!(draft["mergeable"], "blocked");
    let second = show(reads.stack[1]);
    assert_eq!(second["pull"]["base_ref"], "bb-stack-1");
    let fork = show(reads.fork);
    assert_eq!(fork["pull"]["head_repo"], format!("{}/{FORK}", login()));
    assert_eq!(fork["files"][0]["path"], "bb-fork.txt");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn choices_stacks_and_the_merge_request_of_a_branch() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_options_stack");
    let options = live.reads("pr.options", json!({}));
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
    let people: Vec<&str> = options["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["login"].as_str().unwrap())
        .collect();
    assert_eq!(people, [login()]);
    assert_eq!(options["merge_methods"], json!(["merge", "squash"]));
    let stack = live.reads("pr.stack", json!({ "number": reads.stack[1] }));
    let stacked: Vec<u64> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["number"].as_u64().unwrap())
        .collect();
    assert_eq!(stacked, reads.stack);
    assert_eq!(stack["position"], 1);
    let first = live.reads("pr.stack", json!({ "number": reads.stack[0] }));
    assert_eq!(first["position"], 0);
    let branch = live.reads("pr.for_branch", json!({ "branch": "bb-stack-1" }));
    assert_eq!(branch["number"], reads.stack[0]);
    // A fork's branch is not one of the project's own.
    for other in ["bb-fork-topic", "bb-no-such-branch"] {
        assert_eq!(
            live.reads("pr.for_branch", json!({ "branch": other })),
            Value::Null
        );
    }
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn the_inbox_gathers_the_accounts_merge_requests_and_issues() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("inbox");
    let found = live.run("inbox.items", json!({ "wip": false }));
    assert_eq!(found["accounts"].as_array().unwrap().len(), 1, "{found:#}");
    assert!(found["accounts"][0]["error"].is_null(), "{found:#}");
    let pulls = found["pulls"].as_array().unwrap();
    let pull = |number: u64| {
        let key = format!("{}/pull/{}/{REPO}#{number}", live.account, login());
        pulls
            .iter()
            .find(|pull| pull["key"] == key)
            .unwrap_or_else(|| panic!("no {key} in {found:#}"))
    };
    let groups = |number: u64| -> Vec<String> {
        pull(number)["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|group| group.as_str().unwrap().to_owned())
            .collect()
    };
    assert!(pulls.iter().all(|pull| {
        let repo = pull["pull"]["repo"].as_str().unwrap();
        repo.starts_with(&format!("{}/{REPO}", login()))
    }));
    // The account opened them and is asked to review them.
    let topic = pull(reads.topic);
    let why = topic["involvement"].as_array().unwrap();
    assert!(why.contains(&json!("created")) && why.contains(&json!("review_requested")));
    assert!(topic["additions"].as_u64().is_some(), "{topic:#}");
    assert!(groups(reads.changes).contains(&"changes_requested".to_owned()));
    assert!(groups(reads.draft).contains(&"draft".to_owned()));
    for number in [reads.merged, reads.closed] {
        let key = format!("{}/pull/{}/{REPO}#{number}", live.account, login());
        assert!(
            !pulls.iter().any(|pull| pull["key"] == key),
            "{key} is not open"
        );
    }
    let issues = found["issues"].as_array().unwrap();
    let issue = format!("{}/issue/{}/{REPO}#{}", live.account, login(), reads.issue);
    let row = issues
        .iter()
        .find(|row| row["key"] == issue)
        .unwrap_or_else(|| panic!("no {issue} in {found:#}"));
    assert!(
        row["involvement"]
            .as_array()
            .unwrap()
            .contains(&json!("assigned"))
    );
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn issues_are_listed_by_every_filter_and_shown() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("issues");
    let list = |extra: Value| sorted(numbers(&live.reads("issue.list", extra)));
    let open = sorted(vec![reads.issue, reads.plain_issue]);
    assert_eq!(list(json!({})), open);
    assert_eq!(list(json!({ "state": "closed" })), [reads.closed_issue]);
    assert_eq!(
        list(json!({ "state": "all" })),
        sorted(vec![reads.issue, reads.plain_issue, reads.closed_issue])
    );
    assert_eq!(list(json!({ "filter": "assigned" })), [reads.issue]);
    assert_eq!(list(json!({ "filter": "created" })), open);
    // gitlab.com makes a to-do item of a comment naming oneself.
    assert_eq!(list(json!({ "filter": "mentioned" })), [reads.issue]);
    assert_eq!(list(json!({ "query": "#bug" })), [reads.issue]);
    assert_eq!(list(json!({ "query": "label:bug" })), [reads.issue]);
    assert_eq!(list(json!({ "query": "$v1 button" })), [reads.issue]);
    assert_eq!(
        list(json!({ "query": format!("@{}", login()) })),
        [reads.issue]
    );
    assert_eq!(
        list(json!({ "query": "-label:bug", "state": "all" })),
        sorted(vec![reads.closed_issue, reads.plain_issue])
    );
    assert_eq!(
        list(json!({ "query": "nothing-like-this" })),
        Vec::<u64>::new()
    );
    let shown = live.reads("issue.show", json!({ "number": reads.issue }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
    assert_eq!(shown["body"], "Clicking it does nothing at all.");
    assert_eq!(shown["issue"]["labels"][0]["name"], "bug");
    assert_eq!(shown["issue"]["milestone"]["title"], "v1");
    assert_eq!(shown["issue"]["assignees"][0]["login"], login());
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
    let named = format!("@{} the one at the top.", login());
    assert_eq!(
        comments,
        [
            (login(), "Which button do you mean?"),
            (login(), named.as_str())
        ]
    );
    let closed = live.reads("issue.show", json!({ "number": reads.closed_issue }));
    assert_eq!(closed["issue"]["state"], "closed");
    assert_eq!(closed["comments"], json!([]));
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn the_statuses_of_commits_and_pictures_are_read() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("ci_status_avatars");
    let unknown = "0123456789abcdef0123456789abcdef01234567".to_owned();
    let mut shas = reads.shas.to_vec();
    shas.push(unknown.clone());
    let statuses = live.reads("ci.status", json!({ "shas": shas }));
    let states: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["failure", "pending", "running", "none"]);
    assert_eq!(statuses[0]["sha"], reads.shas[0]);
    let checks = statuses[0]["checks"].as_array().unwrap();
    assert_eq!(checks.len(), 2, "{checks:?}");
    let test = checks
        .iter()
        .find(|check| check["name"] == "bb-test")
        .unwrap();
    assert_eq!(test["state"], "failure");
    assert_eq!(test["url"], "https://example.invalid/bb-test");
    assert_eq!(test["description"], "bb-test says failed");
    assert_eq!(statuses[3]["sha"], unknown);
    assert_eq!(statuses[3]["checks"], json!([]));
    // An address nobody at gitlab.com has: the picture of an outside
    // service, as gitlab.com shows it.
    let pictures = live.reads("provider.avatars", json!({ "emails": [IDENTITY.1] }));
    assert!(
        pictures[IDENTITY.1]
            .as_str()
            .is_some_and(|url| url.starts_with("https://")),
        "{pictures}"
    );
    let _ = reads.project;
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn projects_and_where_one_may_be_made_are_listed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    reads();
    let live = Live::new("repos");
    let names = |filter: &str, query: Option<&str>| {
        let listing = live.run(
            "provider.repos",
            json!({ "account": live.account, "filter": filter, "query": query }),
        );
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let reads_project = format!("{}/{REPO}", login());
    let fork = format!("{}/{FORK}", login());
    let own = names("own", None);
    assert!(
        own.contains(&reads_project) && own.contains(&fork),
        "{own:?}"
    );
    assert!(
        own.iter()
            .all(|name| name.starts_with(&format!("{}/", login())))
    );
    assert_eq!(names("organisation", None), Vec::<String>::new());
    assert_eq!(names("starred", None), std::slice::from_ref(&reads_project));
    let all = names("all", None);
    assert!(
        all.contains(&reads_project) && all.contains(&fork),
        "{all:?}"
    );
    assert_eq!(names("all", Some(&FORK)), std::slice::from_ref(&fork));
    let listing = live.run(
        "provider.repos",
        json!({ "account": live.account, "filter": "own" }),
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
    let project = item(&reads_project);
    assert_eq!(project["private"], false);
    assert_eq!(project["default_branch"], "main");
    assert_eq!(
        project["clone_https"],
        format!("https://gitlab.com/{reads_project}.git")
    );
    assert_eq!(
        project["clone_ssh"],
        format!("git@gitlab.com:{reads_project}.git")
    );
    assert_eq!(
        project["web_url"],
        format!("https://gitlab.com/{reads_project}")
    );
    assert_eq!(item(&fork)["parent"], reads_project);
    let namespaces = live.run("provider.namespaces", json!({ "account": live.account }));
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
    assert_eq!(places, [(login(), "user")]);
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM"]
fn signing_in_by_code_starts_and_waits_for_the_user() {
    if std::env::var_os("BITBRAID_LIVE_GITLAB_COM").is_none() {
        eprintln!("skipped: set BITBRAID_LIVE_GITLAB_COM to run the live tests");
        return;
    }
    let _serial = serial();
    // No test account: the guard knows no token.
    let live = Live::bare("device_flow", Arc::new(Guard::new("", None)), true);
    let code = live.run("account.sign_in_start", json!({ "kind": "gitlab" }));
    assert_eq!(code["verification_uri"], "https://gitlab.com/oauth/device");
    let interval = code["interval"].as_u64().unwrap();
    assert!((1..=120).contains(&interval));
    assert!(code["expires_in"].as_u64().unwrap() > 60);
    std::thread::sleep(Duration::from_secs(interval));
    let polled = live.run(
        "account.sign_in_poll",
        json!({ "kind": "gitlab", "device_code": code["device_code"] }),
    );
    assert_eq!(polled, json!({ "state": "waiting" }));
    if let Some(recorder) = &live.recorder {
        let path = recorder.finish(&[]).unwrap();
        eprintln!("recorded {}", path.display());
    }
    let _ = std::fs::remove_dir_all(&live.dir);
}

/// The whole way: a code is asked for and written, with where to type it,
/// to the file `BITBRAID_LIVE_GITLAB_DEVICE` names (`<user_code>
/// <verification_uri>`); someone signed in at gitlab.com as the test
/// account approves it there; the test asks at the interval given, for up
/// to twelve minutes, and then writes `done`, `denied` or `expired` to
/// that file's name with `.result` after it. The token is held in memory
/// only, used for `GET /api/v4/user` (which has to answer the test
/// account) and the token's own facts, and dropped with the account at the
/// end.
#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_DEVICE=<file> and someone to approve the code"]
fn signing_in_by_code_ends_with_the_test_account() {
    let Some(file) = std::env::var_os("BITBRAID_LIVE_GITLAB_DEVICE").map(PathBuf::from) else {
        panic!("set BITBRAID_LIVE_GITLAB_DEVICE to the file the code is written to");
    };
    let login = keychain_login(ITEM)
        .unwrap_or_else(|| panic!("no login is stored with the keychain item {ITEM}"));
    let mut result = file.clone().into_os_string();
    result.push(".result");
    let result = PathBuf::from(result);
    let _ = std::fs::remove_file(&result);
    let _serial = serial();
    // Never recorded: the answer that ends it holds the token.
    let live = Live::bare(
        "device_flow_approved",
        Arc::new(Guard::new(&login, None)),
        false,
    );
    let code = live.run("account.sign_in_start", json!({ "kind": "gitlab" }));
    let user_code = code["user_code"].as_str().unwrap();
    let uri = code["verification_uri"].as_str().unwrap();
    std::fs::write(&file, format!("{user_code} {uri}\n")).unwrap();
    eprintln!("the code is in {}", file.display());
    let mut interval = code["interval"].as_u64().unwrap().clamp(1, 120);
    let expires = code["expires_in"].as_u64().unwrap().min(12 * 60);
    let deadline = Instant::now() + Duration::from_secs(expires);
    let outcome = loop {
        std::thread::sleep(Duration::from_secs(interval));
        if Instant::now() >= deadline {
            break "expired";
        }
        let polled = live.run(
            "account.sign_in_poll",
            json!({ "kind": "gitlab", "device_code": code["device_code"] }),
        );
        match polled["state"].as_str() {
            Some("waiting") => {}
            Some("slow_down") => interval = polled["interval"].as_u64().unwrap_or(interval + 5),
            Some("denied") => break "denied",
            Some("expired") => break "expired",
            Some("done") => {
                // Its `GET /user` answered: it has to be the test account.
                let account = &polled["account"];
                let signed = account["login"].as_str().unwrap_or_default().to_owned();
                assert_eq!(account["sign_in"], "device");
                live.run("account.remove", json!({ "id": account["id"] }));
                if signed != login {
                    std::fs::write(&result, "wrong account\n").unwrap();
                    panic!("the code was approved by another account than the test account");
                }
                break "done";
            }
            other => panic!("account.sign_in_poll said {other:?}"),
        }
    };
    std::fs::write(&result, format!("{outcome}\n")).unwrap();
    // The token goes with the services' keychain, which is in memory.
    let _ = std::fs::remove_dir_all(&live.dir);
    drop(live);
    assert_eq!(outcome, "done");
}

// The writes: merge requests made, changed, reviewed, commented on and
// merged in `<repo>-writes` and `<repo>-ff`, so that the reads keep what
// they are tested on.

/// Something new for this run, for the names of files.
fn run_tag() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos ^ u128::from(std::process::id()))
}

/// A branch made anew from `from` with one new file: the merge requests
/// open from it before are closed and it is removed first.
fn fresh_branch(api: &Api, project: u64, branch: &str, from: &str, run: &str) {
    let (_, open) = api.call(
        Method::Get,
        &format!(
            "/projects/{project}/merge_requests?state=opened&source_branch={}",
            encode(branch)
        ),
        None,
    );
    for merge in open.as_array().cloned().unwrap_or_default() {
        api.ok(
            Method::Put,
            &format!("/projects/{project}/merge_requests/{}", merge["iid"]),
            Some(json!({ "state_event": "close" })),
        );
    }
    let _ = api.call(
        Method::Delete,
        &format!("/projects/{project}/repository/branches/{}", encode(branch)),
        None,
    );
    commit(
        api,
        project,
        branch,
        Some(from),
        &format!("bb-w/{branch}-{run}.txt"),
        &format!("{branch}\n"),
    );
}

/// What the writes start from: the two projects made when missing, their
/// branches made anew; the milestone's id.
fn start_writes() -> u64 {
    let api = &the_world().api;
    let run = run_tag();
    let writes = project(api, &WRITES);
    let milestone = labels_and_milestone(api, writes);
    for branch in ["bb-w-one", "bb-w-alt", "bb-w-stack-1"] {
        fresh_branch(api, writes, branch, "main", &run);
    }
    fresh_branch(api, writes, "bb-w-stack-2", "bb-w-stack-1", &run);
    // A project that merges only what is on top of its target, and a
    // branch that is behind it.
    let ff = project(api, &FAST_FORWARD);
    api.ok(
        Method::Put,
        &format!("/projects/{ff}"),
        Some(json!({ "merge_method": "ff" })),
    );
    fresh_branch(api, ff, "bb-w-behind", "main", &run);
    commit(
        api,
        ff,
        "main",
        None,
        &format!("bb-w/main-{run}.txt"),
        "main\n",
    );
    milestone
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn merge_requests_are_made_changed_reviewed_commented_on_and_merged() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let milestone = start_writes();
    let live = Live::new("pr_writes");
    let write = |id: &str, extra: Value| {
        live.on(&WRITES, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };
    let made = write(
        "pr.create",
        json!({ "head": "bb-w-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true,
                "reviewers": [login()], "assignees": [login()],
                "labels": ["bug"], "milestone": milestone.to_string() }),
    );
    let number = made["number"].as_u64().unwrap();
    assert_eq!(made["title"], "Draft: Change one");
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["reviewers"][0]["user"]["login"], login());
    assert_eq!(made["assignees"][0]["login"], login());
    // A login nobody has stops it before anything is made.
    let err = live
        .on(
            &WRITES,
            "pr.create",
            json!({ "head": "bb-w-alt", "base": "main", "title": "Nobody",
                    "reviewers": ["bitbraid-live-nobody-at-all"] }),
        )
        .unwrap_err();
    assert!(err.contains("no user"), "{err}");
    // The author's own verdicts, asked to review: gitlab.com takes them.
    let approved = write("pr.review", json!({ "number": number, "event": "approve" }));
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], login());
    let changes = write(
        "pr.review",
        json!({ "number": number, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(changes["state"], "changes_requested");
    let shown = write("pr.show", json!({ "number": number }));
    assert_eq!(
        shown["pull"]["reviewers"][0]["state"], "changes_requested",
        "{shown}"
    );
    let commented = write(
        "pr.review",
        json!({ "number": number, "event": "comment", "body": "Better now." }),
    );
    assert_eq!(commented["state"], "commented");
    // Comments, and an answer in the thread of one.
    let comment = write(
        "pr.comment",
        json!({ "number": number, "body": "A comment from the live test." }),
    );
    assert_eq!(comment["body"], "A comment from the live test.");
    assert_eq!(comment["author"]["login"], login());
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
    assert_eq!(
        answered["comments"].as_array().unwrap().len(),
        2,
        "{answered}"
    );
    // Changed: a new title, ready, other labels, nobody asked or
    // assigned, no milestone, another target; then a draft again on its
    // first target; then ready.
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
    // Merged, squashed, its branch removed; merged again: refused.
    let merged = write(
        "pr.merge",
        json!({ "number": number, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert!(merged["sha"].is_string(), "{merged}");
    assert_eq!(merged["branch_deleted"], true);
    let err = live
        .on(
            &WRITES,
            "pr.merge",
            json!({ "number": number, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("merged already"), "{err}");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_stack_moves_down_when_its_bottom_is_merged_and_a_rebase_comes_first() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    start_writes();
    let live = Live::new("pr_merge_stack");
    let write = |project: &str, id: &str, extra: Value| {
        live.on(project, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };
    let first = write(
        &WRITES,
        "pr.create",
        json!({ "head": "bb-w-stack-1", "base": "main", "title": "First of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = write(
        &WRITES,
        "pr.create",
        json!({ "head": "bb-w-stack-2", "base": "bb-w-stack-1", "title": "Second of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let stack = write(&WRITES, "pr.stack", json!({ "number": second }));
    assert_eq!(stack["position"], 1, "{stack}");
    let merged = write(
        &WRITES,
        "pr.merge",
        json!({ "number": first, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "message": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["retargeted"], json!([second]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = write(
        &WRITES,
        "pr.for_branch",
        json!({ "branch": "bb-w-stack-2" }),
    );
    assert_eq!(moved["base_ref"], "main");
    assert_eq!(moved["state"], "open");
    // The fast-forward project: a merge commit is not its way; behind its
    // target, the branch is rebased first.
    let behind = write(
        &FAST_FORWARD,
        "pr.create",
        json!({ "head": "bb-w-behind", "base": "main", "title": "Behind main" }),
    )["number"]
        .as_u64()
        .unwrap();
    let err = live
        .on(
            &FAST_FORWARD,
            "pr.merge",
            json!({ "number": behind, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("allows rebase"), "{err}");
    let rebased = write(
        &FAST_FORWARD,
        "pr.merge",
        json!({ "number": behind, "method": "rebase", "delete_branch": true }),
    );
    assert_eq!(rebased["merged"], true, "{rebased}");
    assert_eq!(rebased["branch_deleted"], true);
    live.finish();
}

/// What a repository has that checking out could change: its branches
/// and remote branches, where they go, what HEAD is, which branch follows
/// what, its remotes, and its index.
fn state(git: &bitbraid_testkit::Git) -> Vec<String> {
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
    out.extend(
        git.lines(["config", "--get-regexp", "^(remote|branch)\\."])
            .unwrap_or_default(),
    );
    out.sort();
    out
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_merge_request_from_a_fork_is_checked_out_as_git_would() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_checkout");
    let home = live.dir.join("home");
    let git = |dir: &Path| bitbraid_testkit::Git::new(dir, &home);
    git(&live.dir).ensure_home().unwrap();
    let origin = format!("{SITE}/{}/{REPO}.git", login());
    // Two clones of the public project: ours, and git's twin.
    for clone in ["ours", "twin"] {
        git(&live.dir)
            .run(["clone", "-q", origin.as_str(), clone])
            .unwrap();
    }
    let ours = live.dir.join("ours");
    let twin = git(&live.dir.join("twin"));
    let checkout = || {
        live.run_in(
            &ours,
            "pr.checkout",
            json!({ "account": live.account, "owner": login(), "name": REPO,
                    "number": reads.fork, "remote": "origin" }),
        )
        .unwrap()
    };
    let out = checkout();
    assert_eq!(out["branch"], "bb-fork-topic");
    assert_eq!(out["remote_added"], login());
    assert_eq!(out["upstream"], format!("{}/bb-fork-topic", login()));
    assert_eq!(out["created"], true);
    let fork = format!("{SITE}/{}/{FORK}.git", login());
    let reference = format!("refs/merge-requests/{}/head", reads.fork);
    for args in [
        vec!["remote", "add", login(), fork.as_str()],
        vec!["fetch", "-q", login()],
        vec!["fetch", "-q", "origin", reference.as_str()],
        vec!["branch", "--no-track", "bb-fork-topic", "FETCH_HEAD"],
        vec!["switch", "-q", "bb-fork-topic"],
    ] {
        twin.run(args).unwrap();
    }
    let upstream = format!("--set-upstream-to={}/bb-fork-topic", login());
    twin.run(["branch", "-q", upstream.as_str(), "bb-fork-topic"])
        .unwrap();
    assert_eq!(state(&git(&ours)), state(&twin));
    assert_eq!(
        out["commit"],
        twin.rev_parse("bb-fork-topic").unwrap().as_str()
    );
    // Again: nothing more is added, and the branch is the same.
    let again = checkout();
    assert_eq!(again["created"], false);
    assert_eq!(again["remote_added"], Value::Null);
    assert_eq!(state(&git(&ours)), state(&twin));
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_branch_is_made_for_an_issue_as_git_would() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("issue_branch");
    let home = live.dir.join("home");
    let git = |dir: &Path| bitbraid_testkit::Git::new(dir, &home);
    git(&live.dir).ensure_home().unwrap();
    let origin = format!("{SITE}/{}/{REPO}.git", login());
    for clone in ["ours", "twin"] {
        git(&live.dir)
            .run(["clone", "-q", origin.as_str(), clone])
            .unwrap();
    }
    let ours = live.dir.join("ours");
    let twin = git(&live.dir.join("twin"));
    // The open repository's link names the project: no account, owner or
    // name is given.
    let made = live
        .run_in(
            &ours,
            "issue.branch",
            json!({ "number": reads.plain_issue, "checkout": true }),
        )
        .unwrap();
    let name = format!("{}-say-what-the-menu-is-for", reads.plain_issue);
    assert_eq!(made["created"], name.as_str());
    twin.run(["switch", "-q", "-c", name.as_str()]).unwrap();
    assert_eq!(state(&git(&ours)), state(&twin));
    let key = live
        .run_in(&ours, "issue.key_from_branch", json!({ "branch": name }))
        .unwrap();
    assert_eq!(key["number"], reads.plain_issue);
    live.finish();
}

/// The key of the tests, made by `ssh-keygen -t ed25519` (its private
/// half was thrown away), and its fingerprint as `ssh-keygen -l` says it.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
const FINGERPRINT: &str = "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0";

/// Remove a project the test made, for good: gitlab.com may first mark a
/// project for removal (and rename it), and removes it at once when asked
/// again with its new path.
fn remove_project(api: &Api, id: u64) {
    let (status, _) = api.call(Method::Delete, &format!("/projects/{id}"), None);
    assert!(matches!(status, 202 | 204 | 404), "DELETE !{id}: {status}");
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        let (status, project) = api.call(Method::Get, &format!("/projects/{id}"), None);
        if status == 404 {
            return;
        }
        let marked = project["marked_for_deletion_on"].is_string()
            || project["marked_for_deletion_at"].is_string();
        if marked && let Some(path) = project["path_with_namespace"].as_str() {
            let (status, said) = api.call(
                Method::Delete,
                &format!(
                    "/projects/{id}?permanently_remove=true&full_path={}",
                    encode(path)
                ),
                None,
            );
            assert!(
                matches!(status, 202 | 204 | 404),
                "DELETE !{id} for good: {status} {said}"
            );
        }
        assert!(Instant::now() < deadline, "!{id} was not removed");
        std::thread::sleep(Duration::from_secs(3));
    }
}

/// Removes what the test of making things made, whatever happens: the
/// projects of its name and the keys of its title.
struct Gone;

impl Gone {
    fn now() {
        let api = &the_world().api;
        let (_, made) = api.call(
            Method::Get,
            &format!("/projects?owned=true&search={REPO}-made&per_page=100"),
            None,
        );
        for project in made.as_array().cloned().unwrap_or_default() {
            if let (Some(id), Some(path)) = (
                project["id"].as_u64(),
                project["path_with_namespace"].as_str(),
            ) && removable(live_repo(), path)
            {
                remove_project(api, id);
            }
        }
        let keys = api.ok(Method::Get, "/user/keys?per_page=100", None);
        for key in keys.as_array().cloned().unwrap_or_default() {
            if key["title"] == KEY_TITLE {
                let (status, _) =
                    api.call(Method::Delete, &format!("/user/keys/{}", key["id"]), None);
                assert!(matches!(status, 204 | 404), "DELETE a key: {status}");
            }
        }
    }
}

impl Drop for Gone {
    fn drop(&mut self) {
        Self::now();
    }
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn a_project_is_made_a_fork_refused_and_a_key_uploaded_and_all_removed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    reads();
    let api = &the_world().api;
    project(api, &WRITES);
    // What a run before left, if it stopped half-way.
    Gone::now();
    let _gone = Gone;
    let live = Live::new("repos_forks_keys");
    let home = live.dir.join("home");
    let dir = live.dir.join("repository");
    std::fs::create_dir_all(&dir).unwrap();
    let git = bitbraid_testkit::Git::new(&dir, &home);
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    let origin = format!("{SITE}/{}/{WRITES}.git", login());
    git.run(["remote", "add", "origin", origin.as_str()])
        .unwrap();
    let made = live
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": live.account, "name": MADE, "private": true,
                    "description": "Made by BitBraid's live test", "remote": "made" }),
        )
        .unwrap();
    assert_eq!(made["repo"]["full_name"], format!("{}/{MADE}", login()));
    assert_eq!(made["repo"]["private"], true);
    assert_eq!(made["remote"], "made");
    let id: u64 = made["repo"]["id"].as_str().unwrap().parse().unwrap();
    // A fork of one's own project into one's own namespace would take its
    // path, which is taken: gitlab.com says so, and no remote is added.
    let err = live
        .run_in(&dir, "provider.fork", json!({ "remote": "origin" }))
        .unwrap_err();
    assert!(err.contains("taken"), "{err}");
    let remotes = git.run(["remote", "-v"]).unwrap();
    assert!(
        remotes.contains(&format!("made\t{SITE}/{}/{MADE}.git (fetch)", login())),
        "{remotes}"
    );
    assert_eq!(remotes.lines().count(), 4, "{remotes}");
    let key = live.dir.join("bitbraid-live.pub");
    std::fs::write(&key, KEY).unwrap();
    let added = live.run(
        "provider.ssh_key_add",
        json!({ "account": live.account, "title": KEY_TITLE, "path": key.to_str().unwrap() }),
    );
    assert_eq!(added["title"], KEY_TITLE);
    assert_eq!(added["fingerprint"], FINGERPRINT);
    let key_id = added["id"].as_str().unwrap().to_owned();
    live.finish();
    // Removed in the same run.
    let (status, _) = api.call(Method::Delete, &format!("/user/keys/{key_id}"), None);
    assert_eq!(status, 204);
    remove_project(api, id);
}

/// A file of `branch` written as `text` by the test's identity: made, or
/// changed when it is there (`delete` when `text` is none).
fn write_file(api: &Api, project: u64, branch: &str, file: &str, text: Option<&str>) {
    let (status, _) = api.call(
        Method::Get,
        &format!(
            "/projects/{project}/repository/files/{}?ref={}",
            encode(file),
            encode(branch)
        ),
        None,
    );
    let action = match (status == 200, text) {
        (true, Some(_)) => "update",
        (false, Some(_)) => "create",
        (true, None) => "delete",
        (false, None) => return,
    };
    let mut change = json!({ "action": action, "file_path": file });
    if let Some(text) = text {
        change["content"] = json!(text);
    }
    api.ok(
        Method::Post,
        &format!("/projects/{project}/repository/commits"),
        Some(
            json!({ "branch": branch, "commit_message": format!("Write {file}"),
                     "author_name": IDENTITY.0, "author_email": IDENTITY.1,
                     "actions": [change] }),
        ),
    );
}

/// Suggestions from local edits: a merge request changes a
/// file; a repository here has its head checked out and the file edited;
/// what `pr.suggestions` makes of the edits is sent with `pr.comment` on
/// the lines (threads on the diff), and the merge request shows a
/// suggestion on each. The merge request is closed and its branch and the
/// file removed at the end.
#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITLAB_COM and the test account in the keychain"]
fn local_edits_are_offered_and_sent_as_suggestions() {
    use crate::suggestions;
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let api = &the_world().api;
    let writes = project(api, &WRITES);
    write_file(
        api,
        writes,
        "main",
        suggestions::FILE,
        Some(suggestions::BASE),
    );
    let (_, open) = api.call(
        Method::Get,
        &format!(
            "/projects/{writes}/merge_requests?state=opened&source_branch={}",
            encode(suggestions::BRANCH)
        ),
        None,
    );
    for merge in open.as_array().cloned().unwrap_or_default() {
        api.ok(
            Method::Put,
            &format!("/projects/{writes}/merge_requests/{}", merge["iid"]),
            Some(json!({ "state_event": "close" })),
        );
    }
    let _ = api.call(
        Method::Delete,
        &format!(
            "/projects/{writes}/repository/branches/{}",
            encode(suggestions::BRANCH)
        ),
        None,
    );
    api.ok(
        Method::Post,
        &format!(
            "/projects/{writes}/repository/branches?branch={}&ref=main",
            encode(suggestions::BRANCH)
        ),
        None,
    );
    write_file(
        api,
        writes,
        suggestions::BRANCH,
        suggestions::FILE,
        Some(suggestions::HEAD),
    );
    let number = api.ok(
        Method::Post,
        &format!("/projects/{writes}/merge_requests"),
        Some(
            json!({ "source_branch": suggestions::BRANCH, "target_branch": "main",
                     "title": "Suggestions from local edits",
                     "description": "Made by the live test." }),
        ),
    )["iid"]
        .as_u64()
        .unwrap();
    // GitLab works its diff out after it answers.
    let started = Instant::now();
    loop {
        let merge = api.ok(
            Method::Get,
            &format!("/projects/{writes}/merge_requests/{number}"),
            None,
        );
        let (_, diffs) = api.call(
            Method::Get,
            &format!("/projects/{writes}/merge_requests/{number}/diffs"),
            None,
        );
        let ready = !merge["diff_refs"]["head_sha"].is_null()
            && diffs.as_array().is_some_and(|diffs| !diffs.is_empty());
        if ready {
            break;
        }
        assert!(started.elapsed() < Duration::from_secs(60), "no diff came");
        std::thread::sleep(Duration::from_secs(2));
    }
    let here = std::env::temp_dir().join(format!("bb-t-suggest-lab-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&here);
    let local = suggestions::local_repo(&here.join("work"), &here.join("home"));
    let live = Live::new("pr_suggestions");
    let found = live
        .on(
            &WRITES,
            "pr.for_branch",
            json!({ "branch": suggestions::BRANCH }),
        )
        .unwrap();
    assert_eq!(found["number"].as_u64(), Some(number));
    let offered = live
        .run_in(
            &local,
            "pr.suggestions",
            json!({ "account": live.account, "owner": login(), "name": WRITES,
                    "number": number }),
        )
        .unwrap();
    assert_eq!(suggestions::shape(&offered), suggestions::expected());
    assert_eq!(
        offered["suggestions"][2]["body"],
        "```suggestion:-0+0\nnine\nten\n```"
    );
    for suggestion in offered["suggestions"].as_array().unwrap() {
        let mut params = suggestions::sent(suggestion);
        params["number"] = json!(number);
        let comment = live.on(&WRITES, "pr.comment", params).unwrap();
        assert_eq!(comment["body"], suggestion["body"]);
    }
    let shown = live
        .on(&WRITES, "pr.show", json!({ "number": number }))
        .unwrap();
    let mut lines: Vec<u64> = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|thread| thread["path"] == suggestions::FILE)
        .map(|thread| thread["line"].as_u64().unwrap())
        .collect();
    lines.sort_unstable();
    assert_eq!(lines, [1, 3, 9]);
    live.finish();
    // What was made goes.
    api.ok(
        Method::Put,
        &format!("/projects/{writes}/merge_requests/{number}"),
        Some(json!({ "state_event": "close" })),
    );
    let _ = api.call(
        Method::Delete,
        &format!(
            "/projects/{writes}/repository/branches/{}",
            encode(suggestions::BRANCH)
        ),
        None,
    );
    write_file(api, writes, "main", suggestions::FILE, None);
    let _ = std::fs::remove_dir_all(&here);
}
