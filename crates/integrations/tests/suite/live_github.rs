//! The provider commands against github.com, live, with test accounts: a
//! test organisation, its machine account (an administrator of it) and a
//! reviewer, whose token reaches the organisation's repositories only.
//! Every read and every write of the commands is made here on public
//! repositories of the organisation: `<repo>` for the reads, with real
//! check runs of GitHub Actions, reviews by a second person, a fork and
//! its pull request; `<repo>-writes` for the writes. `<repo>` is
//! `bitbraid-live`, or the name `BITBRAID_LIVE_REPO` gives. Repositories
//! made by `provider.repo_create` (`<repo>-made-org`, `<repo>-made-user`)
//! and the key `provider.ssh_key_add` uploads (`<repo> test key`) are
//! removed again in the same run.
//!
//! Opt-in, ignored by default: it runs with `--ignored` when
//! `BITBRAID_LIVE_GITHUB_ORG` names the test organisation, and is
//! skipped with a message otherwise. It needs, on Linux with a Secret
//! Service and `secret-tool`, the two accounts' tokens in the keychain,
//! each under the service `bitbraid-test` with the login as an attribute:
//!
//! ```sh
//! secret-tool store --label='BitBraid test: machine account' \
//!     service bitbraid-test account github-main login <machine-login>
//! secret-tool store --label='BitBraid test: reviewer' \
//!     service bitbraid-test account github-reviewer login <reviewer-login>
//! BITBRAID_LIVE_GITHUB_ORG=<organisation> \
//!     cargo test -p bitbraid-integrations --test suite live_github:: -- --ignored
//! ```
//!
//! The machine account's token has the scopes `repo`, `workflow`,
//! `read:org`, `read:user`, `user:email`, `delete_repo` and
//! `admin:public_key`; the reviewer's has to reach the organisation's
//! public repositories, to review and comment there. No account is
//! named in this file: the organisation comes from the variable, and the
//! two logins from the keychain items' `login` attribute. The tokens are
//! read from the same items at run time and are never printed or written;
//! a token's first request has to be `GET /user` answering with its
//! item's login, or nothing else is let through for it.
//!
//! A guard in front of the network holds every request to what the test
//! may touch, and refuses the rest before it is sent:
//! - the machine account: `GET /user`, `/rate_limit`, its own lists
//!   (`/user/repos`, `/user/starred`, `/user/orgs`, `/user/keys`); making
//!   a repository `<repo>-made-*` and removing it; starring one of the
//!   test repositories; uploading the test's key and removing a key of
//!   that title; reading the organisation (`/orgs/<organisation>…`) and
//!   making its test repositories; anything under
//!   `/repos/<organisation>/<repo>*` and `/repos/<machine account>/<repo>*`;
//!   GraphQL questions that name only those repositories, and the three
//!   changes the commands make there;
//! - the reviewer: `GET /user`; reviews, review comments and issue
//!   comments of `/repos/<organisation>/<repo>*`; GraphQL reads of those;
//! - a token the guard does not know (the device flow's): `GET /user`;
//! - without a token: `github.com/login/device/code` and
//!   `/login/oauth/access_token` with the built-in application id.
//!
//! A GraphQL question passes when it names nothing in its text (every
//! value comes in its variables), names none of the fields that reach an
//! account's other things (`viewer`, `user`, `organization`,
//! `repositoryOwner`, `node`, `repositories`, …), and its top fields are
//! only `repository(owner: $owner, name: $name)` with a test repository
//! in its variables, `search(query: $search, …)` whose every `repo:` is a
//! test repository and which names no `org:`, `user:` or `owner:`, and
//! `rateLimit`. The changes it lets through (for the machine account
//! only) are marking a review conversation resolved and a pull request a
//! draft or ready, of one the guard saw in an answer about a test
//! repository.
//!
//! What the reads need is made when missing (the setup can run any number
//! of times); the writes make their branches anew in each run. The
//! content is synthetic, and commits are made with a test identity. With
//! `BITBRAID_RECORD=<dir>` the answers of the commands (not of the setup)
//! are recorded there, scrubbed, with the machine account as `user-1`,
//! the reviewer as `user-2`, the organisation as `org-1` and `<repo>` as
//! `bitbraid-live`: `tests/recordings/github-org/` keeps them, and
//! `tests/suite/recorded_github.rs` and `recorded_github_writes.rs` replay
//! them by default. `BITBRAID_RECORD_FORBID` names more words, split by
//! commas, that may not be in a recording (a name an account carries);
//! it is read when the test runs and is in no file, and
//! `recorded::every_recording_is_scrubbed_of_the_live_tests_people`
//! looks for the same words when it is set.
//!
//! Signing in by a code is tested up to the user's part (a code is asked
//! for, and asking for the token says "waiting"); the whole way, with
//! someone approving the code at github.com as the machine account, is a
//! test of its own that runs only when `BITBRAID_LIVE_GITHUB_DEVICE`
//! names the file the code is written to.
//!
//! Sources: docs.github.com/en/rest "Users" (get the authenticated user),
//! "Repositories" (create an organization repository, create a repository
//! for the authenticated user, get, delete), "Forks" (create a fork),
//! "Branches", "Git references", "Repository contents" (create or update
//! file contents, with `author`, `committer` and `sha`), "Pulls" (create,
//! list with `head`, update, merge: the 405 and 409 answers), "Review
//! requests" (request reviewers), "Pull request reviews" (create with
//! `event` and `comments`), "Pull request review comments" (reply),
//! "Issues", "Issue comments", "Labels", "Milestones", "Commit statuses"
//! (create, get the combined status), "Check runs" (list check runs for a
//! Git reference), "Starring" (star a repository for the authenticated
//! user), "Git SSH keys" (list, delete), "Organizations" (get);
//! docs.github.com/en/graphql "Mutations" (`resolveReviewThread`),
//! "Forming calls with GraphQL"; docs.github.com/en/actions "Workflow
//! syntax for GitHub Actions" (`on`, `jobs.<job_id>.if`, `permissions`),
//! "Events that trigger workflows" (`pull_request`, `push`);
//! docs.github.com "Authorizing OAuth apps" (the device flow);
//! secret-tool(1).
#![allow(clippy::print_stderr)]

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard, OnceLock, atomic::AtomicBool},
    time::{Duration, Instant},
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{
    Services,
    http::{Call, Client, Fetch, FetchError, Limits, Method, Response, Url},
    oauth::GITHUB_CLIENT_ID,
    replay::Recorder,
    secrets::{MemoryStore, Secret},
};
use serde_json::{Value, json};

/// The name of the live tests' repository of the reads when
/// `BITBRAID_LIVE_REPO` gives none; the recordings call it so whatever its
/// name was.
pub(crate) const DEFAULT_REPO: &str = "bitbraid-live";

/// The name of the live tests' repository of the reads, at github.com and
/// at gitlab.com: `BITBRAID_LIVE_REPO`, or [`DEFAULT_REPO`]. The others are
/// named after it.
pub(crate) fn live_repo() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let name = std::env::var("BITBRAID_LIVE_REPO")
            .map(|name| name.trim().to_owned())
            .unwrap_or_default();
        if name.is_empty() {
            return DEFAULT_REPO.to_owned();
        }
        assert!(
            name.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
            "BITBRAID_LIVE_REPO is not a repository's name"
        );
        name
    })
}

/// A name made from [`live_repo`]: it followed by `suffix`.
#[derive(Debug)]
pub(crate) struct LiveName {
    suffix: &'static str,
    name: OnceLock<String>,
}

impl LiveName {
    pub(crate) const fn new(suffix: &'static str) -> Self {
        Self {
            suffix,
            name: OnceLock::new(),
        }
    }
}

impl std::ops::Deref for LiveName {
    type Target = str;

    fn deref(&self) -> &str {
        self.name
            .get_or_init(|| format!("{}{}", live_repo(), self.suffix))
    }
}

impl std::fmt::Display for LiveName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self)
    }
}

impl serde::Serialize for LiveName {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self)
    }
}

impl PartialEq<LiveName> for Value {
    fn eq(&self, other: &LiveName) -> bool {
        self.as_str() == Some(&**other)
    }
}

/// The repository of the reads, public, and its fork's name.
static REPO: LiveName = LiveName::new("");
/// The repository of the writes, public.
static WRITES: LiveName = LiveName::new("-writes");
/// The repositories `provider.repo_create` makes, in the organisation and
/// in the machine account's own namespace, removed in the same run.
static MADE_IN_ORG: LiveName = LiveName::new("-made-org");
static MADE_OWN: LiveName = LiveName::new("-made-user");
/// The title of the key the test uploads.
static KEY_TITLE: LiveName = LiveName::new(" test key");
/// Who the commits the test makes are by.
const IDENTITY: (&str, &str) = ("BitBraid Test", "bitbraid-test@example.invalid");

/// The workflow of the reads' repository: one job passes, one fails on
/// the branch `bb-topic` only, one is always skipped.
const WORKFLOW: &str = r#"# The checks of BitBraid's live tests: one job passes, one fails on
# the branch bb-topic, and one is always skipped.
name: checks
on:
  push:
    branches: [main]
  pull_request:
permissions:
  contents: read
jobs:
  passes:
    runs-on: ubuntu-latest
    steps:
      - run: echo "This check passes."
  fails:
    if: github.head_ref == 'bb-topic'
    runs-on: ubuntu-latest
    steps:
      - run: |
          echo "This check fails on purpose."
          exit 1
  skipped:
    if: false
    runs-on: ubuntu-latest
    steps:
      - run: echo "This check never runs."
"#;

/// The test accounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Role {
    Main,
    Reviewer,
}

impl Role {
    fn item(self) -> &'static str {
        match self {
            Self::Main => "github-main",
            Self::Reviewer => "github-reviewer",
        }
    }
}

/// The test accounts' names, taken at run time: the organisation from
/// `BITBRAID_LIVE_GITHUB_ORG`, the two logins from the keychain items, and
/// the name of the repository of the reads (see [`live_repo`]).
#[derive(Debug, Clone, Default)]
struct Names {
    org: String,
    main: String,
    reviewer: String,
    repo: String,
}

impl Names {
    fn login(&self, role: Role) -> &str {
        match role {
            Role::Main => &self.main,
            Role::Reviewer => &self.reviewer,
        }
    }

    /// Whether a repository's name is one the test makes and removes
    /// again (`<repo>-made-org`, `<repo>-made-user`).
    fn made(&self, name: &str) -> bool {
        name.strip_prefix(self.repo.as_str())
            .is_some_and(|rest| rest.starts_with("-made-"))
    }

    /// The title of the key the test uploads.
    fn key_title(&self) -> String {
        format!("{} test key", self.repo)
    }

    /// The names of this run; none, with a message, when the live tests
    /// are not asked for.
    fn of_this_run() -> Option<Self> {
        let Some(org) = std::env::var("BITBRAID_LIVE_GITHUB_ORG")
            .ok()
            .map(|org| org.trim().to_owned())
            .filter(|org| !org.is_empty())
        else {
            eprintln!(
                "skipped: set BITBRAID_LIVE_GITHUB_ORG to the test organisation, \
                 with the test accounts in the keychain"
            );
            return None;
        };
        let login = |role: Role| {
            keychain_login(role.item()).unwrap_or_else(|| {
                panic!("no login is stored with the keychain item {}", role.item())
            })
        };
        let names = Self {
            org,
            main: login(Role::Main),
            reviewer: login(Role::Reviewer),
            repo: live_repo().to_owned(),
        };
        for name in [&names.org, &names.main, &names.reviewer] {
            assert!(
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "a test account's name is not a login"
            );
        }
        Some(names)
    }

    /// The stand-ins of the accounts in every recording, and of the
    /// repositories' name when it is not the one the recordings use.
    fn stand_ins(&self) -> Vec<(&str, &'static str)> {
        let mut stand_ins = vec![
            (self.main.as_str(), "user-1"),
            (self.reviewer.as_str(), "user-2"),
            (self.org.as_str(), "org-1"),
        ];
        if self.repo != DEFAULT_REPO {
            stand_ins.push((self.repo.as_str(), DEFAULT_REPO));
        }
        stand_ins
    }
}

/// The names of this run, once [`world`] read them.
static NAMES: OnceLock<Names> = OnceLock::new();

fn names() -> &'static Names {
    NAMES.get().expect("the test accounts are read first")
}

/// The test organisation.
fn org() -> &'static str {
    &names().org
}

/// The machine account's login.
fn main_login() -> &'static str {
    &names().main
}

/// The reviewer's login.
fn reviewer_login() -> &'static str {
    &names().reviewer
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

/// A test account's token, from its keychain item.
fn token(role: Role) -> Option<Secret> {
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "service", "bitbraid-test", "account", role.item()])
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !token.is_empty()).then(|| Secret::new(token))
}

/// Whether a repository is one the tests may touch: `<repo>` or
/// `<repo>-*`, of the organisation or of the machine account.
fn ours(names: &Names, owner: &str, name: &str) -> bool {
    !owner.is_empty()
        && (owner.eq_ignore_ascii_case(&names.org) || owner.eq_ignore_ascii_case(&names.main))
        && name
            .strip_prefix(names.repo.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Whether `role` may ask about a repository: the reviewer only about the
/// organisation's.
fn ours_for(names: &Names, role: Role, owner: &str, name: &str) -> bool {
    ours(names, owner, name) && (role == Role::Main || owner.eq_ignore_ascii_case(&names.org))
}

/// The top fields of a GraphQL question: each one's name (an alias left
/// out) and its arguments without spaces.
pub(crate) fn top_fields(query: &str) -> Option<Vec<(String, String)>> {
    let start = query.find('{')?;
    let chars: Vec<char> = query.get(start + 1..)?.chars().collect();
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut at = 0;
    while at < chars.len() {
        let c = chars[at];
        if c == '{' {
            depth += 1;
        } else if c == '}' {
            if depth == 0 {
                break;
            }
            depth -= 1;
        } else if depth == 0 && (c.is_alphabetic() || c == '_') {
            let mut name = String::new();
            while at < chars.len() && (chars[at].is_alphanumeric() || chars[at] == '_') {
                name.push(chars[at]);
                at += 1;
            }
            while at < chars.len() && chars[at].is_whitespace() {
                at += 1;
            }
            if chars.get(at) == Some(&':') {
                // An alias: the field's name comes next.
                at += 1;
                continue;
            }
            let mut arguments = String::new();
            if chars.get(at) == Some(&'(') {
                let mut open = 0usize;
                while at < chars.len() {
                    let c = chars[at];
                    at += 1;
                    match c {
                        '(' => {
                            open += 1;
                            if open == 1 {
                                continue;
                            }
                        }
                        ')' => {
                            open -= 1;
                            if open == 0 {
                                break;
                            }
                        }
                        _ => {}
                    }
                    if !c.is_whitespace() {
                        arguments.push(c);
                    }
                }
            }
            out.push((name, arguments));
            continue;
        }
        at += 1;
    }
    Some(out)
}

/// Whether a search names only test repositories `role` may ask about.
fn search_is_ours(names: &Names, search: &str, role: Role) -> bool {
    let words: Vec<&str> = search.split_whitespace().collect();
    let repos: Vec<&str> = words
        .iter()
        .filter_map(|word| word.trim_start_matches('-').strip_prefix("repo:"))
        .collect();
    !repos.is_empty()
        && repos.iter().all(|repo| {
            repo.split_once('/')
                .is_some_and(|(owner, name)| ours_for(names, role, owner, name))
        })
        && !words.iter().any(|word| {
            let word = word.trim_start_matches('-').to_ascii_lowercase();
            word.starts_with("org:") || word.starts_with("user:") || word.starts_with("owner:")
        })
}

/// Whether a GraphQL question may be asked (the rules at the top).
fn graphql_is_ours(names: &Names, body: &Value, seen: &BTreeSet<String>, role: Role) -> bool {
    let Some(query) = body["query"].as_str() else {
        return false;
    };
    let variables = &body["variables"];
    if query.contains('"') {
        return false;
    }
    let lower = query.to_ascii_lowercase();
    // Fields, by their whole names (a type's name starts in capitals).
    let forbidden = [
        "viewer",
        "user",
        "organization",
        "organizations",
        "repositoryOwner",
        "node",
        "repositories",
        "starredRepositories",
        "enterprise",
        "resource",
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
    if fields.is_empty() {
        return false;
    }
    if lower.trim_start().starts_with("mutation") {
        let allowed = match fields[0].0.as_str() {
            "resolveReviewThread" => fields[0].1 == "input:{threadId:$id}",
            "convertPullRequestToDraft" | "markPullRequestReadyForReview" => {
                fields[0].1 == "input:{pullRequestId:$id}"
            }
            _ => false,
        };
        return role == Role::Main
            && fields.len() == 1
            && allowed
            && variables["id"].as_str().is_some_and(|id| seen.contains(id));
    }
    if !lower.trim_start().starts_with("query") {
        return false;
    }
    fields.iter().all(|(name, arguments)| match name.as_str() {
        "repository" => {
            arguments == "owner:$owner,name:$name"
                && ours_for(
                    names,
                    role,
                    variables["owner"].as_str().unwrap_or_default(),
                    variables["name"].as_str().unwrap_or_default(),
                )
        }
        "search" => {
            arguments.starts_with("query:$search,")
                && search_is_ours(
                    names,
                    variables["search"].as_str().unwrap_or_default(),
                    role,
                )
        }
        "rateLimit" => true,
        _ => false,
    })
}

/// The searches of the inbox's question (`inbox.items`), by the
/// names of their variables.
const INBOX_SEARCHES: [&str; 5] = ["pulls", "review", "assigned", "created", "mentioned"];

/// Whether a search of the inbox may be asked: the machine account's own
/// open pull requests and issues, and nothing else named.
fn inbox_search_is_ours(names: &Names, search: &str) -> bool {
    search.split_whitespace().all(|word| {
        matches!(
            word,
            "is:pr" | "is:issue" | "is:open" | "archived:false" | "sort:updated-desc"
        ) || word.split_once(':').is_some_and(|(key, value)| match key {
            "involves" | "review-requested" | "assignee" | "author" | "mentions" => {
                value.eq_ignore_ascii_case(&names.main)
            }
            "repo" => value
                .split_once('/')
                .is_some_and(|(owner, name)| ours(names, owner, name)),
            _ => false,
        })
    })
}

/// Whether a GraphQL question is the inbox's, as the machine account:
/// only searches, each of the shape the command asks, every one of the
/// machine account's own; what they answer of other repositories is taken
/// out before it is handed on ([`only_ours_found`]).
fn inbox_is_ours(names: &Names, body: &Value, role: Role) -> bool {
    let Some(query) = body["query"].as_str() else {
        return false;
    };
    if role != Role::Main || query.contains('"') || !query.trim_start().starts_with("query") {
        return false;
    }
    let forbidden = [
        "viewer",
        "user",
        "organization",
        "organizations",
        "repositoryOwner",
        "node",
        "repositories",
        "starredRepositories",
        "enterprise",
        "resource",
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
    fields.len() == INBOX_SEARCHES.len()
        && fields.iter().all(|(name, arguments)| {
            name == "search"
                && INBOX_SEARCHES.iter().any(|var| {
                    *arguments == format!("query:${var},type:ISSUE,first:$first")
                        && body["variables"][*var]
                            .as_str()
                            .is_some_and(|search| inbox_search_is_ours(names, search))
                })
        })
}

/// The inbox's answer with only the test repositories' pull requests and
/// issues in it: whether anything was taken out.
fn only_ours_found(names: &Names, answer: &mut Value) -> bool {
    let mut taken = false;
    let Some(data) = answer.get_mut("data").and_then(Value::as_object_mut) else {
        return false;
    };
    for searched in data.values_mut() {
        let Some(nodes) = searched.get_mut("nodes").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = nodes.len();
        nodes.retain(|node| {
            let repo = node["baseRepository"]["nameWithOwner"]
                .as_str()
                .or_else(|| node["repository"]["nameWithOwner"].as_str());
            repo.and_then(|repo| repo.split_once('/'))
                .is_some_and(|(owner, name)| ours(names, owner, name))
        });
        taken |= nodes.len() != before;
    }
    taken
}

/// Every `id` (GraphQL's) and `node_id` (REST's) an answer names.
fn ids(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                if (key == "id" || key == "node_id")
                    && let Some(id) = inner.as_str()
                {
                    out.insert(id.to_owned());
                }
                ids(inner, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| ids(item, out)),
        _ => {}
    }
}

/// Who sends a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Caller {
    /// A test account, by its token.
    Account(Role),
    /// A token the guard does not know: the device flow's.
    Stranger,
    /// No token.
    Nobody,
}

/// Something done just before a request is sent.
type Hook = Box<dyn FnOnce() + Send>;

/// The network, held to what this test may ask.
struct Guard {
    inner: Client,
    /// Whose things it lets through.
    names: Names,
    /// The test accounts' tokens.
    tokens: Vec<(Secret, Role)>,
    /// The accounts whose `GET /user` answered with their login.
    confirmed: Mutex<BTreeSet<Role>>,
    /// The ids named in answers about the test repositories.
    seen: Mutex<BTreeSet<String>>,
    /// The ids of the keys of the test's title, as answers named them.
    keys: Mutex<BTreeSet<u64>>,
    /// What to do before a request (its method and path) is sent.
    before: Mutex<Vec<(Method, String, Hook)>>,
}

impl std::fmt::Debug for Guard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Guard")
            .field("accounts", &self.tokens.len())
            .finish()
    }
}

impl Guard {
    fn new(names: Names, tokens: Vec<(Secret, Role)>) -> Self {
        Self {
            inner: Client::new("BitBraid-Test/live"),
            names,
            tokens,
            confirmed: Mutex::default(),
            seen: Mutex::default(),
            keys: Mutex::default(),
            before: Mutex::default(),
        }
    }

    /// Do `hook` just before `method path` is next sent.
    fn before(&self, method: Method, path: &str, hook: Hook) {
        self.before
            .lock()
            .unwrap()
            .push((method, path.to_owned(), hook));
    }

    fn caller(&self, call: &Call) -> Caller {
        let Some(value) = call.header("authorization") else {
            return Caller::Nobody;
        };
        let sent = value
            .strip_prefix("Bearer ")
            .or_else(|| value.strip_prefix("token "))
            .unwrap_or(value)
            .trim();
        self.tokens
            .iter()
            .find(|(token, _)| token.expose() == sent)
            .map_or(Caller::Stranger, |(_, role)| Caller::Account(*role))
    }

    fn allows(&self, call: &Call) -> bool {
        let path = call.url.path().split('?').next().unwrap_or_default();
        let lower = path.to_ascii_lowercase();
        if lower.contains("..") || lower.contains("%2e") || lower.contains("%2f") {
            return false;
        }
        let caller = self.caller(call);
        if call.url.host() == "github.com" && call.url.secure() {
            // The device flow, with the built-in application.
            let form = call
                .body
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default();
            return caller == Caller::Nobody
                && call.method == Method::Post
                && matches!(path, "/login/device/code" | "/login/oauth/access_token")
                && form
                    .split('&')
                    .any(|pair| pair == format!("client_id={GITHUB_CLIENT_ID}"));
        }
        if call.url.host() != "api.github.com" || !call.url.secure() {
            return false;
        }
        let segments: Vec<&str> = path.split('/').skip(1).collect();
        match caller {
            Caller::Nobody => false,
            Caller::Stranger => call.method == Method::Get && path == "/user",
            Caller::Account(role) => {
                let confirmed = self.confirmed.lock().unwrap().contains(&role);
                if call.method == Method::Get && path == "/user" {
                    true
                } else if !confirmed {
                    false
                } else if role == Role::Main {
                    self.main_allows(call, &segments)
                } else {
                    self.reviewer_allows(call, &segments)
                }
            }
        }
    }

    fn body(call: &Call) -> Value {
        call.body
            .as_deref()
            .and_then(|body| serde_json::from_slice::<Value>(body).ok())
            .unwrap_or(Value::Null)
    }

    fn main_allows(&self, call: &Call, segments: &[&str]) -> bool {
        let body = || Self::body(call);
        let named = |key: &str, test: &dyn Fn(&str) -> bool| body()[key].as_str().is_some_and(test);
        match (call.method, segments) {
            (Method::Get, ["rate_limit"])
            | (Method::Get, ["user", "repos" | "starred" | "orgs" | "keys"]) => true,
            (Method::Post, ["user", "repos"]) => named("name", &|name| self.names.made(name)),
            (Method::Put | Method::Delete, ["user", "starred", owner, name]) => {
                ours(&self.names, owner, name)
            }
            (Method::Post, ["user", "keys"]) => {
                named("title", &|title| title == self.names.key_title())
            }
            (Method::Delete, ["user", "keys", id]) => id
                .parse::<u64>()
                .is_ok_and(|id| self.keys.lock().unwrap().contains(&id)),
            (Method::Get, ["orgs", org, ..]) => org.eq_ignore_ascii_case(&self.names.org),
            (Method::Post, ["orgs", org, "repos"]) => {
                org.eq_ignore_ascii_case(&self.names.org)
                    && named("name", &|name| {
                        name == self.names.repo
                            || name == format!("{}-writes", self.names.repo)
                            || self.names.made(name)
                    })
            }
            (Method::Delete, ["repos", owner, name]) => {
                ours(&self.names, owner, name) && self.names.made(name)
            }
            // A fork into the machine account's own namespace only.
            (Method::Post, ["repos", owner, name, "forks"]) => {
                ours(&self.names, owner, name) && body()["organization"].is_null()
            }
            (_, ["repos", owner, name, ..]) => ours(&self.names, owner, name),
            (Method::Post, ["graphql"]) => {
                graphql_is_ours(&self.names, &body(), &self.seen.lock().unwrap(), Role::Main)
                    || inbox_is_ours(&self.names, &body(), Role::Main)
            }
            _ => false,
        }
    }

    fn reviewer_allows(&self, call: &Call, segments: &[&str]) -> bool {
        let number = |text: &str| !text.is_empty() && text.bytes().all(|b| b.is_ascii_digit());
        match (call.method, segments) {
            (Method::Post, ["graphql"]) => graphql_is_ours(
                &self.names,
                &Self::body(call),
                &self.seen.lock().unwrap(),
                Role::Reviewer,
            ),
            (method, ["repos", owner, name, rest @ ..])
                if ours_for(&self.names, Role::Reviewer, owner, name) =>
            {
                match (method, rest) {
                    // Reviews.
                    (Method::Get | Method::Post, ["pulls", n, "reviews"]) => number(n),
                    (Method::Get, ["pulls", n, "reviews", id, ..]) => number(n) && number(id),
                    // Review comments.
                    (Method::Get | Method::Post, ["pulls", n, "comments"]) => number(n),
                    (Method::Post, ["pulls", n, "comments", id, "replies"]) => {
                        number(n) && number(id)
                    }
                    (Method::Get, ["pulls", "comments", id]) => number(id),
                    // Issue comments.
                    (Method::Get | Method::Post, ["issues", n, "comments"]) => number(n),
                    (Method::Get, ["issues", "comments", id]) => number(id),
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

impl Fetch for Guard {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        if !self.allows(call) {
            return Err(FetchError::Address("refused by the live test's guard"));
        }
        let path = call.url.path().split('?').next().unwrap_or_default();
        let hook = {
            let mut hooks = self.before.lock().unwrap();
            hooks
                .iter()
                .position(|(method, at, _)| *method == call.method && at == path)
                .map(|at| hooks.remove(at).2)
        };
        if let Some(hook) = hook {
            hook();
        }
        let mut response = self.inner.send(call, cancel)?;
        let mut answer = serde_json::from_slice::<Value>(&response.body).unwrap_or(Value::Null);
        // The inbox's searches reach every repository: only the tests'
        // are handed on.
        if path == "/graphql"
            && self.caller(call) == Caller::Account(Role::Main)
            && inbox_is_ours(&self.names, &Self::body(call), Role::Main)
            && only_ours_found(&self.names, &mut answer)
        {
            response.body = serde_json::to_vec(&answer).unwrap();
        }
        // Whose the token is, before anything else.
        if let Caller::Account(role) = self.caller(call)
            && call.method == Method::Get
            && path == "/user"
            && response.status == 200
            && answer["login"].as_str() == Some(self.names.login(role))
        {
            self.confirmed.lock().unwrap().insert(role);
        }
        let segments: Vec<&str> = path.split('/').skip(1).collect();
        let about_ours = match segments.as_slice() {
            ["graphql"] => true,
            ["repos", owner, name, ..] => ours(&self.names, owner, name),
            _ => false,
        };
        if about_ours {
            ids(&answer, &mut self.seen.lock().unwrap());
        }
        if path == "/user/keys" {
            let listed = match &answer {
                Value::Array(items) => items.clone(),
                other => vec![other.clone()],
            };
            let mut keys = self.keys.lock().unwrap();
            for key in listed {
                if key["title"] == self.names.key_title().as_str()
                    && let Some(id) = key["id"].as_u64()
                {
                    keys.insert(id);
                }
            }
        }
        Ok(response)
    }
}

/// The guarded API as one test account, for what the test makes.
#[derive(Clone)]
struct Api {
    guard: Arc<Guard>,
    token: Secret,
}

impl Api {
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = Url::parse(&format!("https://api.github.com{path}")).unwrap();
        let mut call = Call::new(
            method,
            url,
            Limits {
                connect: Duration::from_secs(10),
                total: Duration::from_secs(60),
                body: 16 << 20,
            },
        )
        .with_header("Accept", "application/vnd.github+json")
        .with_header("X-GitHub-Api-Version", "2022-11-28")
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
            value.get("message").cloned().unwrap_or_default()
        );
        value
    }

    fn graphql(&self, query: &str, variables: Value) -> Value {
        let answer = self.ok(
            Method::Post,
            "/graphql",
            Some(json!({ "query": query, "variables": variables })),
        );
        assert!(answer["errors"].is_null(), "{answer}");
        answer["data"].clone()
    }
}

#[test]
fn the_guard_refuses_what_is_not_the_tests() {
    let guard = Guard::new(
        Names {
            org: "test-org".to_owned(),
            main: "test-main".to_owned(),
            reviewer: "test-reviewer".to_owned(),
            repo: DEFAULT_REPO.to_owned(),
        },
        vec![
            (Secret::new("main-token"), Role::Main),
            (Secret::new("reviewer-token"), Role::Reviewer),
        ],
    );
    let call = |token: Option<&str>, method: Method, path: &str, body: Option<Value>| {
        let mut call = Call::new(
            method,
            Url::parse(&format!("https://api.github.com{path}")).unwrap(),
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
    let main = |method: Method, path: &str, body: Option<Value>| {
        guard.allows(&call(Some("main-token"), method, path, body))
    };
    let reviewer = |method: Method, path: &str, body: Option<Value>| {
        guard.allows(&call(Some("reviewer-token"), method, path, body))
    };
    // Before a token's `GET /user` said whose it is, nothing else.
    assert!(main(Method::Get, "/user", None));
    assert!(!main(Method::Get, "/repos/test-org/bitbraid-live", None));
    assert!(!reviewer(
        Method::Get,
        "/repos/test-org/bitbraid-live/pulls/1/reviews",
        None
    ));
    guard.confirmed.lock().unwrap().insert(Role::Main);
    guard.confirmed.lock().unwrap().insert(Role::Reviewer);
    for (method, path, body) in [
        (Method::Get, "/rate_limit", None),
        (Method::Get, "/user/repos?affiliation=owner", None),
        (Method::Get, "/user/starred", None),
        (Method::Get, "/user/orgs", None),
        (Method::Get, "/user/keys", None),
        (
            Method::Post,
            "/user/repos",
            Some(json!({ "name": "bitbraid-live-made-user" })),
        ),
        (
            Method::Post,
            "/orgs/test-org/repos",
            Some(json!({ "name": "bitbraid-live" })),
        ),
        (Method::Get, "/orgs/test-org", None),
        (Method::Put, "/user/starred/test-org/bitbraid-live", None),
        (
            Method::Get,
            "/repos/test-org/bitbraid-live/pulls?state=all",
            None,
        ),
        (
            Method::Put,
            "/repos/test-org/bitbraid-live/contents/a.txt",
            None,
        ),
        (Method::Get, "/repos/test-main/bitbraid-live/branches", None),
        (
            Method::Delete,
            "/repos/test-org/bitbraid-live-made-org",
            None,
        ),
        (
            Method::Delete,
            "/repos/test-main/bitbraid-live-made-user",
            None,
        ),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live/forks",
            Some(json!({})),
        ),
        (
            Method::Post,
            "/user/keys",
            Some(json!({ "title": "bitbraid-live test key", "key": "ssh-ed25519 AAAA" })),
        ),
    ] {
        assert!(main(method, path, body), "{path}");
    }
    for (method, path, body) in [
        (Method::Get, "/user/emails", None),
        (Method::Get, "/user/followers", None),
        (Method::Patch, "/user", None),
        (
            Method::Post,
            "/user/repos",
            Some(json!({ "name": "dotfiles" })),
        ),
        (Method::Post, "/user/repos", None),
        (
            Method::Post,
            "/user/keys",
            Some(json!({ "title": "another key", "key": "ssh-ed25519 AAAA" })),
        ),
        (Method::Delete, "/user/keys/1", None),
        (Method::Put, "/user/starred/someone/bitbraid-live", None),
        (Method::Get, "/orgs/other", None),
        (Method::Patch, "/orgs/test-org", None),
        (
            Method::Post,
            "/orgs/test-org/repos",
            Some(json!({ "name": "anything" })),
        ),
        (Method::Get, "/repos/test-org/other-repository", None),
        (Method::Get, "/repos/test-reviewer/bitbraid-live", None),
        (Method::Get, "/repos/someone/bitbraid-live", None),
        (
            Method::Get,
            "/repos/test-org/bitbraid-live/../other-repository",
            None,
        ),
        (Method::Get, "/repos/test-org/bitbraid-live/%2E%2E/x", None),
        (Method::Delete, "/repos/test-org/bitbraid-live", None),
        (Method::Delete, "/repos/test-org/bitbraid-live-writes", None),
        (Method::Delete, "/repos/test-main/bitbraid-live", None),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live/forks",
            Some(json!({ "organization": "other" })),
        ),
        (Method::Post, "/graphql", None),
    ] {
        assert!(!main(method, path, body), "{path}");
    }
    // A key of the test's title, once an answer named it, may go.
    guard.keys.lock().unwrap().insert(77);
    assert!(main(Method::Delete, "/user/keys/77", None));
    // The reviewer: reviews and comments of the organisation's test
    // repositories, and nothing else.
    for (method, path) in [
        (Method::Get, "/user"),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live/pulls/3/reviews",
        ),
        (Method::Get, "/repos/test-org/bitbraid-live/pulls/3/reviews"),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live/pulls/3/comments",
        ),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live-writes/pulls/3/comments/9/replies",
        ),
        (
            Method::Get,
            "/repos/test-org/bitbraid-live/pulls/comments/9",
        ),
        (
            Method::Post,
            "/repos/test-org/bitbraid-live/issues/4/comments",
        ),
        (
            Method::Get,
            "/repos/test-org/bitbraid-live/issues/comments/9",
        ),
    ] {
        assert!(reviewer(method, path, None), "{path}");
    }
    for (method, path) in [
        (Method::Get, "/rate_limit"),
        (Method::Get, "/user/repos"),
        (Method::Get, "/repos/test-org/bitbraid-live"),
        (Method::Get, "/repos/test-org/bitbraid-live/pulls/3"),
        (Method::Put, "/repos/test-org/bitbraid-live/pulls/3/merge"),
        (Method::Post, "/repos/test-org/bitbraid-live/pulls"),
        (Method::Post, "/repos/test-org/bitbraid-live/issues"),
        (
            Method::Post,
            "/repos/test-main/bitbraid-live/pulls/3/reviews",
        ),
        (
            Method::Post,
            "/repos/test-org/other-repository/pulls/3/reviews",
        ),
        (
            Method::Post,
            "/repos/test-reviewer/bitbraid-live/issues/4/comments",
        ),
        (Method::Delete, "/repos/test-org/bitbraid-live-made-org"),
    ] {
        assert!(!reviewer(method, path, None), "{path}");
    }
    // A token the guard does not know: who it is, and nothing else; no
    // token: nothing at the API.
    assert!(guard.allows(&call(Some("stranger"), Method::Get, "/user", None)));
    assert!(!guard.allows(&call(Some("stranger"), Method::Get, "/user/repos", None)));
    assert!(!guard.allows(&call(None, Method::Get, "/user", None)));
    // GraphQL.
    let graphql = |token: &str, query: &str, variables: Value| {
        guard.allows(&call(
            Some(token),
            Method::Post,
            "/graphql",
            Some(json!({ "query": query, "variables": variables })),
        ))
    };
    let repository = "query($owner: String!, $name: String!, $number: Int!) { \
        repository(owner: $owner, name: $name) { pullRequest(number: $number) { title } } }";
    let checks = "query($owner: String!, $name: String!, $c0: String!) { \
        repository(owner: $owner, name: $name) { c0: object(expression: $c0) { ...Checks } } } \
        fragment Checks on Commit { statusCheckRollup { state } }";
    let search = "query($search: String!, $first: Int!, $after: String) { \
        search(query: $search, type: ISSUE, first: $first, after: $after) { \
        nodes { ... on PullRequest { number reviewRequests(first: 1) { nodes { \
        requestedReviewer { ... on User { login } ... on Team { name } } } } } } } }";
    let resolve = "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { \
        thread { isResolved } } }";
    for (token, query, variables) in [
        (
            "main-token",
            repository,
            json!({ "owner": "test-org", "name": "bitbraid-live", "number": 1 }),
        ),
        (
            "main-token",
            checks,
            json!({ "owner": "test-main", "name": "bitbraid-live", "c0": "abc123" }),
        ),
        (
            "main-token",
            search,
            json!({ "search": "repo:test-org/bitbraid-live is:pr author:test-main", "first": 100 }),
        ),
        (
            "reviewer-token",
            search,
            json!({ "search": "repo:test-org/bitbraid-live is:pr review-requested:test-reviewer" }),
        ),
        (
            "reviewer-token",
            repository,
            json!({ "owner": "test-org", "name": "bitbraid-live-writes", "number": 1 }),
        ),
    ] {
        assert!(graphql(token, query, variables.clone()), "{variables}");
    }
    for (token, query, variables) in [
        // Another repository, or the accounts' other things.
        (
            "main-token",
            repository,
            json!({ "owner": "test-org", "name": "other-repository", "number": 1 }),
        ),
        (
            "main-token",
            repository,
            json!({ "owner": "test-reviewer", "name": "bitbraid-live", "number": 1 }),
        ),
        (
            "main-token",
            "query { repository(owner: \"test-org\", name: \"other-repository\") { id } }",
            json!({}),
        ),
        ("main-token", "query { viewer { login } }", json!({})),
        (
            "main-token",
            "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { \
             owner { ... on User { repositories(first: 100) { nodes { name } } } } } }",
            json!({ "owner": "test-org", "name": "bitbraid-live" }),
        ),
        (
            "main-token",
            "query($login: String!) { user(login: $login) { name } }",
            json!({ "login": "test-reviewer" }),
        ),
        (
            "main-token",
            "query($owner: String!, $name: String!) { repository(owner: $owner, name: $name) { id } \
             other: repository(owner: $other, name: $name) { id } }",
            json!({ "owner": "test-org", "name": "bitbraid-live" }),
        ),
        // A search that is not held to a test repository.
        (
            "main-token",
            search,
            json!({ "search": "is:pr author:test-main" }),
        ),
        (
            "main-token",
            search,
            json!({ "search": "repo:test-org/bitbraid-live repo:test-org/other-repository" }),
        ),
        (
            "main-token",
            search,
            json!({ "search": "repo:test-org/bitbraid-live org:someone" }),
        ),
        (
            "main-token",
            search,
            json!({ "search": "repo:test-org/bitbraid-live user:test-reviewer" }),
        ),
        // The reviewer: not the machine account's fork, and no changes.
        (
            "reviewer-token",
            repository,
            json!({ "owner": "test-main", "name": "bitbraid-live", "number": 1 }),
        ),
        (
            "reviewer-token",
            search,
            json!({ "search": "repo:test-main/bitbraid-live is:pr" }),
        ),
        // A change to something the guard did not see.
        ("main-token", resolve, json!({ "id": "PRRT_unseen" })),
        (
            "main-token",
            "mutation($id: ID!) { deleteRepository(input: {repositoryId: $id}) { clientMutationId } }",
            json!({ "id": "x" }),
        ),
    ] {
        assert!(
            !graphql(token, query, variables.clone()),
            "{query} {variables}"
        );
    }
    // Seen in an answer about a test repository: resolving it passes, for
    // the machine account only.
    guard.seen.lock().unwrap().insert("PRRT_seen".to_owned());
    assert!(graphql("main-token", resolve, json!({ "id": "PRRT_seen" })));
    assert!(!graphql(
        "reviewer-token",
        resolve,
        json!({ "id": "PRRT_seen" })
    ));
    for mutation in ["convertPullRequestToDraft", "markPullRequestReadyForReview"] {
        let question = format!(
            "mutation($id: ID!) {{ {mutation}(input: {{pullRequestId: $id}}) {{ \
             pullRequest {{ isDraft }} }} }}"
        );
        assert!(!graphql(
            "main-token",
            &question,
            json!({ "id": "PR_unseen" })
        ));
        guard.seen.lock().unwrap().insert("PR_seen".to_owned());
        assert!(graphql("main-token", &question, json!({ "id": "PR_seen" })));
    }
    // The device flow at github.com: without a token, with the built-in
    // application, and nothing else there.
    let site = |path: &str, token: Option<&str>, form: &str| {
        let mut call = Call::new(
            Method::Post,
            Url::parse(&format!("https://github.com{path}")).unwrap(),
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
    let ours = format!("client_id={GITHUB_CLIENT_ID}&scope=repo");
    assert!(site("/login/device/code", None, &ours));
    assert!(site("/login/oauth/access_token", None, &ours));
    assert!(!site(
        "/login/device/code",
        None,
        "client_id=another&scope=repo"
    ));
    assert!(!site("/login/device/code", Some("main-token"), &ours));
    assert!(!site("/settings/keys", None, &ours));
    let elsewhere = Call::new(
        Method::Get,
        Url::parse("https://example.org/user").unwrap(),
        Limits {
            connect: Duration::from_secs(1),
            total: Duration::from_secs(1),
            body: 1,
        },
    )
    .with_header("Authorization", "Bearer main-token");
    assert!(!guard.allows(&elsewhere));
}

#[test]
fn the_guard_lets_the_inboxs_searches_through_with_only_the_tests_answers() {
    let names = Names {
        org: "test-org".to_owned(),
        main: "test-main".to_owned(),
        reviewer: "test-reviewer".to_owned(),
        repo: DEFAULT_REPO.to_owned(),
    };
    let query = "query($pulls: String!, $review: String!, $assigned: String!, $created: String!, \
         $mentioned: String!, $first: Int!) { \
         pulls: search(query: $pulls, type: ISSUE, first: $first) { nodes { ... on PullRequest { number } } } \
         review: search(query: $review, type: ISSUE, first: $first) { nodes { ... on PullRequest { number } } } \
         assigned: search(query: $assigned, type: ISSUE, first: $first) { nodes { ... on Issue { number } } } \
         created: search(query: $created, type: ISSUE, first: $first) { nodes { ... on Issue { number } } } \
         mentioned: search(query: $mentioned, type: ISSUE, first: $first) { nodes { ... on Issue { number } } } }";
    let body = |who: &str, more: &str| {
        let search = |kind: &str, key: &str| {
            format!("is:{kind} is:open archived:false {key}:{who}{more} sort:updated-desc")
        };
        json!({ "query": query, "variables": {
            "pulls": search("pr", "involves"), "review": search("pr", "review-requested"),
            "assigned": search("issue", "assignee"), "created": search("issue", "author"),
            "mentioned": search("issue", "mentions"), "first": 50,
        }})
    };
    assert!(inbox_is_ours(&names, &body("test-main", ""), Role::Main));
    assert!(inbox_is_ours(
        &names,
        &body("test-main", " repo:test-org/bitbraid-live"),
        Role::Main
    ));
    assert!(!inbox_is_ours(
        &names,
        &body("test-main", ""),
        Role::Reviewer
    ));
    for (who, more) in [
        ("someone-else", ""),
        ("test-main", " org:test-org"),
        ("test-main", " repo:test-org/other"),
        ("test-main", " user:test-main"),
        ("test-main", " label:x"),
    ] {
        assert!(
            !inbox_is_ours(&names, &body(who, more), Role::Main),
            "{who}{more}"
        );
    }
    let mut asked = body("test-main", "");
    asked["query"] = json!(query.replace("pulls: search", "pulls: viewer { login } x: search"));
    assert!(!inbox_is_ours(&names, &asked, Role::Main));
    // What other repositories answered is taken out.
    let mut answer = json!({ "data": {
        "pulls": { "nodes": [
            { "number": 1, "baseRepository": { "nameWithOwner": "test-org/bitbraid-live" } },
            { "number": 2, "baseRepository": { "nameWithOwner": "someone/private" } },
        ] },
        "assigned": { "nodes": [
            { "number": 3, "repository": { "nameWithOwner": "test-main/bitbraid-live-writes" } },
            { "number": 4, "repository": { "nameWithOwner": "test-org/elsewhere" } },
            {},
        ] },
    }});
    assert!(only_ours_found(&names, &mut answer));
    assert_eq!(
        answer["data"]["pulls"]["nodes"].as_array().unwrap().len(),
        1
    );
    assert_eq!(answer["data"]["assigned"]["nodes"][0]["number"], 3);
    assert_eq!(
        answer["data"]["assigned"]["nodes"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(!only_ours_found(&names, &mut answer));
}

/// What every live test starts from: the guard, the two accounts' APIs,
/// and what may not be in a recording. Made once per run.
struct World {
    guard: Arc<Guard>,
    main: Api,
    reviewer: Api,
    /// The logins, ids, names and addresses of the accounts.
    real: Vec<String>,
}

fn world() -> Option<&'static World> {
    static WORLD: OnceLock<Option<World>> = OnceLock::new();
    WORLD
        .get_or_init(|| {
            let names = Names::of_this_run()?;
            let _ = NAMES.set(names.clone());
            let main_token = token(Role::Main).expect("the machine account's token");
            let reviewer_token = token(Role::Reviewer).expect("the reviewer's token");
            let guard = Arc::new(Guard::new(
                names,
                vec![
                    (main_token.clone(), Role::Main),
                    (reviewer_token.clone(), Role::Reviewer),
                ],
            ));
            let main = Api {
                guard: guard.clone(),
                token: main_token,
            };
            let reviewer = Api {
                guard: guard.clone(),
                token: reviewer_token,
            };
            // Whose the tokens are, before anything else.
            let me = main.ok(Method::Get, "/user", None);
            assert_eq!(
                me["login"],
                main_login(),
                "the machine account's token is not its keychain item's login's: stopping"
            );
            let them = reviewer.ok(Method::Get, "/user", None);
            assert_eq!(
                them["login"],
                reviewer_login(),
                "the reviewer's token is not its keychain item's login's: stopping"
            );
            let organisation = main.ok(Method::Get, &format!("/orgs/{}", org()), None);
            let mut real = vec![
                main_login().to_owned(),
                reviewer_login().to_owned(),
                org().to_owned(),
            ];
            for person in [&me, &them, &organisation] {
                if let Some(id) = person["id"].as_u64() {
                    real.push(id.to_string());
                }
                for key in ["name", "email"] {
                    if let Some(value) = person[key].as_str()
                        && value.trim().len() >= 3
                    {
                        real.push(value.trim().to_owned());
                    }
                }
            }
            Some(World {
                guard,
                main,
                reviewer,
                real,
            })
        })
        .as_ref()
}

/// The world of a test that asked [`world`] first.
fn the_world() -> &'static World {
    world().expect("the live tests are asked for")
}

/// One live test at a time: they share the repositories.
fn serial() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Base64, as the contents API takes a file.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            chunk.get(1).copied().unwrap_or(0),
            chunk.get(2).copied().unwrap_or(0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(char::from(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// `/repos/owner/name`.
fn at(owner: &str, name: &str) -> String {
    format!("/repos/{owner}/{name}")
}

/// A repository of the organisation, public, made when missing: its
/// default branch.
fn org_repository(api: &Api, name: &str) -> String {
    let repo = at(org(), name);
    let (status, _) = api.call(Method::Get, &repo, None);
    if status == 404 {
        api.ok(
            Method::Post,
            &format!("/orgs/{}/repos", org()),
            Some(json!({ "name": name, "private": false, "auto_init": true,
                         "has_wiki": false, "has_projects": false,
                         "description": "Synthetic test data of BitBraid's live tests" })),
        );
    }
    // GitHub makes the first commit after it answers.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let (status, _) = api.call(Method::Get, &format!("{repo}/git/ref/heads/main"), None);
        if status == 200 {
            break;
        }
        assert!(Instant::now() < deadline, "{repo} has no main branch");
        std::thread::sleep(Duration::from_secs(2));
    }
    api.ok(Method::Get, &repo, None)["default_branch"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// A file on a branch, committed by the test's identity: made when
/// missing, or replaced when `replace` and it says something else.
fn put_file(api: &Api, repo: &str, branch: &str, path: &str, text: &str, replace: bool) {
    let (status, found) = api.call(
        Method::Get,
        &format!("{repo}/contents/{path}?ref={branch}"),
        None,
    );
    let mut body = json!({ "message": format!("Write {path}"), "content": base64(text.as_bytes()),
                           "branch": branch,
                           "author": { "name": IDENTITY.0, "email": IDENTITY.1 },
                           "committer": { "name": IDENTITY.0, "email": IDENTITY.1 } });
    if status == 200 {
        let same = found["content"]
            .as_str()
            .map(|content| content.replace('\n', ""))
            .is_some_and(|content| content == base64(text.as_bytes()));
        if same || !replace {
            return;
        }
        body["sha"] = found["sha"].clone();
    }
    api.ok(Method::Put, &format!("{repo}/contents/{path}"), Some(body));
}

/// A branch from `from`, made when missing.
fn branch(api: &Api, repo: &str, name: &str, from: &str) {
    let (status, _) = api.call(Method::Get, &format!("{repo}/git/ref/heads/{name}"), None);
    if status == 200 {
        return;
    }
    let sha = head(api, repo, from);
    api.ok(
        Method::Post,
        &format!("{repo}/git/refs"),
        Some(json!({ "ref": format!("refs/heads/{name}"), "sha": sha })),
    );
}

/// The head commit of a branch.
fn head(api: &Api, repo: &str, branch: &str) -> String {
    api.ok(Method::Get, &format!("{repo}/git/ref/heads/{branch}"), None)["object"]["sha"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// The pull request from `head` (`owner:branch` for a fork's) into
/// `base` of the organisation's `name`, made when missing: its number.
fn pull(api: &Api, name: &str, head: &str, base: &str, title: &str, draft: bool) -> u64 {
    let repo = at(org(), name);
    let qualified = if head.contains(':') {
        head.to_owned()
    } else {
        format!("{}:{head}", org())
    };
    let (_, found) = api.call(
        Method::Get,
        &format!("{repo}/pulls?state=all&head={qualified}"),
        None,
    );
    if let Some(number) = found
        .as_array()
        .and_then(|all| all.first())
        .and_then(|pull| pull["number"].as_u64())
    {
        return number;
    }
    api.ok(
        Method::Post,
        &format!("{repo}/pulls"),
        Some(
            json!({ "title": title, "head": head, "base": base, "draft": draft,
                     "body": format!("Made by the live test: {title}.") }),
        ),
    )["number"]
        .as_u64()
        .unwrap()
}

/// The issue called `title`, made when missing: its number.
fn issue(api: &Api, repo: &str, title: &str, extra: Value) -> u64 {
    let (_, found) = api.call(
        Method::Get,
        &format!("{repo}/issues?state=all&per_page=100"),
        None,
    );
    if let Some(number) = found
        .as_array()
        .and_then(|all| {
            all.iter()
                .find(|issue| issue["title"] == title && issue["pull_request"].is_null())
        })
        .and_then(|issue| issue["number"].as_u64())
    {
        return number;
    }
    let mut body = json!({ "title": title });
    for (key, value) in extra.as_object().unwrap() {
        body[key] = value.clone();
    }
    api.ok(Method::Post, &format!("{repo}/issues"), Some(body))["number"]
        .as_u64()
        .unwrap()
}

/// A comment on an issue or a pull request by `api`'s account, written
/// when no comment says it.
fn comment_once(api: &Api, repo: &str, number: u64, body: &str) {
    let path = format!("{repo}/issues/{number}/comments");
    let (_, comments) = api.call(Method::Get, &format!("{path}?per_page=100"), None);
    if comments
        .as_array()
        .is_some_and(|all| all.iter().any(|comment| comment["body"] == body))
    {
        return;
    }
    api.ok(Method::Post, &path, Some(json!({ "body": body })));
}

/// The labels `bug` and `ui` and the milestone `v1`, made when missing:
/// the milestone's number.
fn labels_and_milestone(api: &Api, repo: &str) -> u64 {
    for (name, color, description) in [
        ("bug", "d73a4a", "Something is broken"),
        ("ui", "0075ca", "How it looks"),
    ] {
        let (status, _) = api.call(Method::Get, &format!("{repo}/labels/{name}"), None);
        if status == 404 {
            // A new repository gets GitHub's own labels a little after it
            // is made: one of them there by now is taken as it is.
            let (status, said) = api.call(
                Method::Post,
                &format!("{repo}/labels"),
                Some(json!({ "name": name, "color": color, "description": description })),
            );
            assert!(matches!(status, 201 | 422), "a label: {status} {said}");
        }
    }
    let (_, milestones) = api.call(Method::Get, &format!("{repo}/milestones?state=all"), None);
    match milestones
        .as_array()
        .and_then(|all| all.iter().find(|milestone| milestone["title"] == "v1"))
    {
        Some(found) => found["number"].as_u64().unwrap(),
        None => api.ok(
            Method::Post,
            &format!("{repo}/milestones"),
            Some(json!({ "title": "v1", "due_on": "2026-12-31T00:00:00Z" })),
        )["number"]
            .as_u64()
            .unwrap(),
    }
}

/// The reviews of a pull request by an account: their states.
fn reviews_by(api: &Api, repo: &str, number: u64, login: &str) -> Vec<String> {
    let (_, reviews) = api.call(
        Method::Get,
        &format!("{repo}/pulls/{number}/reviews?per_page=100"),
        None,
    );
    reviews
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .filter(|review| review["user"]["login"] == login)
        .filter_map(|review| review["state"].as_str().map(str::to_owned))
        .collect()
}

/// Ask the reviewer to review, unless they did or were asked already.
fn ask_reviewer(api: &Api, repo: &str, number: u64) {
    if !reviews_by(api, repo, number, reviewer_login()).is_empty() {
        return;
    }
    let pull = api.ok(Method::Get, &format!("{repo}/pulls/{number}"), None);
    let asked = pull["requested_reviewers"]
        .as_array()
        .is_some_and(|all| all.iter().any(|user| user["login"] == reviewer_login()));
    if !asked {
        api.ok(
            Method::Post,
            &format!("{repo}/pulls/{number}/requested_reviewers"),
            Some(json!({ "reviewers": [reviewer_login()] })),
        );
    }
}

/// The reviewer's verdict, given once.
fn verdict(world: &World, repo: &str, number: u64, event: &str, body: &str) {
    let state = match event {
        "APPROVE" => "APPROVED",
        _ => "CHANGES_REQUESTED",
    };
    if reviews_by(&world.main, repo, number, reviewer_login())
        .iter()
        .any(|said| said == state)
    {
        return;
    }
    world.reviewer.ok(
        Method::Post,
        &format!("{repo}/pulls/{number}/reviews"),
        Some(json!({ "event": event, "body": body })),
    );
}

/// A commit status, set when the commit has none of that context.
fn status_once(api: &Api, repo: &str, sha: &str, context: &str, state: &str) {
    let combined = api.ok(Method::Get, &format!("{repo}/commits/{sha}/status"), None);
    if combined["statuses"]
        .as_array()
        .is_some_and(|all| all.iter().any(|status| status["context"] == context))
    {
        return;
    }
    api.ok(
        Method::Post,
        &format!("{repo}/statuses/{sha}"),
        Some(json!({ "state": state, "context": context,
                     "description": format!("{context} says {state}"),
                     "target_url": format!("https://example.invalid/{context}") })),
    );
}

/// Put an empty commit on a branch: the workflow of its pull request runs
/// again for it (GitHub's `synchronize`), and its files stay as they are.
fn nudge(api: &Api, repo: &str, branch: &str) {
    let sha = head(api, repo, branch);
    let commit = api.ok(Method::Get, &format!("{repo}/git/commits/{sha}"), None);
    let identity = json!({ "name": IDENTITY.0, "email": IDENTITY.1 });
    let made = api.ok(
        Method::Post,
        &format!("{repo}/git/commits"),
        Some(
            json!({ "message": "Run the checks again", "tree": commit["tree"]["sha"],
                     "parents": [sha], "author": identity, "committer": identity }),
        ),
    );
    api.ok(
        Method::Patch,
        &format!("{repo}/git/refs/heads/{branch}"),
        Some(json!({ "sha": made["sha"] })),
    );
}

/// Wait until the check runs of GitHub Actions on the heads of these
/// branches (the repository each is in, and its name) are done in `repo`:
/// three on each. A head that has none (made before its workflow was
/// right, or whose run GitHub did not start) gets an empty commit, which
/// runs the workflow, at most three times.
fn wait_for_checks(api: &Api, repo: &str, branches: &[(&str, &str)]) {
    let runs = |(at, branch): &(&str, &str)| {
        let sha = head(api, at, branch);
        let (_, runs) = api.call(
            Method::Get,
            &format!("{repo}/commits/{sha}/check-runs?per_page=100"),
            None,
        );
        runs["check_runs"].as_array().cloned().unwrap_or_default()
    };
    let deadline = Instant::now() + Duration::from_secs(900);
    for attempt in 0..3 {
        for branch in branches {
            if runs(branch).is_empty() {
                nudge(api, branch.0, branch.1);
            }
        }
        // A while for the runs to start and finish.
        let round = Instant::now() + Duration::from_secs(180);
        let done = loop {
            let all: Vec<Vec<Value>> = branches.iter().map(runs).collect();
            if all
                .iter()
                .all(|runs| runs.len() >= 3 && runs.iter().all(|run| run["status"] == "completed"))
            {
                break true;
            }
            if Instant::now() > round && all.iter().any(Vec::is_empty) {
                break false;
            }
            assert!(
                Instant::now() < deadline,
                "the checks did not finish in fifteen minutes"
            );
            std::thread::sleep(Duration::from_secs(10));
        };
        if done {
            return;
        }
        eprintln!("some checks did not start (round {attempt}): asking again");
    }
    panic!("GitHub did not run the checks of every pull request");
}

/// What the reads are tested on, in the organisation's `<repo>`.
#[derive(Debug, Clone)]
struct Reads {
    /// Open, labelled, assigned, in the milestone, with conversations on
    /// its lines (one resolved) and on the whole, approved by the
    /// reviewer, its checks failing.
    topic: u64,
    /// Open, the reviewer asking for changes.
    changes: u64,
    /// A draft, the reviewer asked and silent.
    draft: u64,
    /// Two stacked.
    stack: [u64; 2],
    merged: u64,
    closed: u64,
    /// From the machine account's fork.
    fork: u64,
    /// The heads of `topic`, `draft`, the first of the stack, `changes`.
    shas: [String; 4],
    /// Open, labelled, assigned, in the milestone, with comments by both.
    issue: u64,
    closed_issue: u64,
    /// Open, nothing on it.
    plain_issue: u64,
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
    let api = &world.main;
    let repo = at(org(), &REPO);
    let base = org_repository(api, &REPO);
    assert_eq!(base, "main");
    // The workflow first, so that every branch has it.
    put_file(
        api,
        &repo,
        &base,
        ".github/workflows/checks.yml",
        WORKFLOW,
        true,
    );
    let milestone = labels_and_milestone(api, &repo);
    api.ok(
        Method::Put,
        &format!("/user/starred/{}/{REPO}", org()),
        None,
    );
    // The machine account's fork, with a branch of its own.
    let fork = at(main_login(), &REPO);
    let (status, _) = api.call(Method::Get, &fork, None);
    if status == 404 {
        api.ok(Method::Post, &format!("{repo}/forks"), Some(json!({})));
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            let (status, _) = api.call(Method::Get, &format!("{fork}/git/ref/heads/main"), None);
            if status == 200 {
                break;
            }
            assert!(Instant::now() < deadline, "the fork was not made");
            std::thread::sleep(Duration::from_secs(3));
        }
    }
    branch(api, &fork, "bb-fork-topic", "main");
    put_file(
        api,
        &fork,
        "bb-fork-topic",
        "bb-fork.txt",
        "a line from a fork\n",
        false,
    );
    // The first pull request, filled in.
    branch(api, &repo, "bb-topic", &base);
    put_file(
        api,
        &repo,
        "bb-topic",
        "bb-topic.txt",
        "a line to review\nanother line\n",
        false,
    );
    let topic = pull(api, &REPO, "bb-topic", &base, "Add a file to review", false);
    api.ok(
        Method::Patch,
        &format!("{repo}/issues/{topic}"),
        Some(json!({ "labels": ["bug"], "milestone": milestone, "assignees": [main_login()] })),
    );
    let (_, comments) = api.call(
        Method::Get,
        &format!("{repo}/pulls/{topic}/comments?per_page=100"),
        None,
    );
    if comments.as_array().is_none_or(|all| all.len() < 3) {
        let review = api.ok(
            Method::Post,
            &format!("{repo}/pulls/{topic}/reviews"),
            Some(
                json!({ "event": "COMMENT", "body": "A few things on the lines.",
                         "comments": [
                             { "path": "bb-topic.txt", "line": 1, "body": "This line could say more." }
                         ] }),
            ),
        );
        let review_id = review["id"].as_u64().unwrap();
        let (_, made) = api.call(
            Method::Get,
            &format!("{repo}/pulls/{topic}/reviews/{review_id}/comments"),
            None,
        );
        let first = made[0]["id"].as_u64().unwrap();
        api.ok(
            Method::Post,
            &format!("{repo}/pulls/{topic}/comments/{first}/replies"),
            Some(json!({ "body": "Done." })),
        );
        api.ok(
            Method::Post,
            &format!("{repo}/pulls/{topic}/reviews"),
            Some(json!({ "event": "COMMENT", "body": "",
                         "comments": [
                             { "path": "bb-topic.txt", "line": 2, "body": "Is this the right file for it?" }
                         ] })),
        );
    }
    // The first conversation resolved.
    let threads = api.graphql(
        "query($owner: String!, $name: String!, $number: Int!) { \
         repository(owner: $owner, name: $name) { pullRequest(number: $number) { \
         reviewThreads(first: 100) { nodes { id isResolved \
         comments(first: 1) { nodes { body } } } } } } }",
        json!({ "owner": org(), "name": REPO, "number": topic }),
    );
    for thread in threads["repository"]["pullRequest"]["reviewThreads"]["nodes"]
        .as_array()
        .unwrap()
    {
        if thread["comments"]["nodes"][0]["body"] == "This line could say more."
            && thread["isResolved"] == false
        {
            api.graphql(
                "mutation($id: ID!) { resolveReviewThread(input: {threadId: $id}) { \
                 thread { isResolved } } }",
                json!({ "id": thread["id"] }),
            );
        }
    }
    comment_once(api, &repo, topic, "Looks good to me.");
    comment_once(&world.reviewer, &repo, topic, "A second look: fine.");
    ask_reviewer(api, &repo, topic);
    verdict(world, &repo, topic, "APPROVE", "Fine by me.");
    // One the reviewer wants changed.
    branch(api, &repo, "bb-changes", &base);
    put_file(
        api,
        &repo,
        "bb-changes",
        "bb-changes.txt",
        "a line to change\n",
        false,
    );
    let changes = pull(api, &REPO, "bb-changes", &base, "Change the wording", false);
    ask_reviewer(api, &repo, changes);
    verdict(
        world,
        &repo,
        changes,
        "REQUEST_CHANGES",
        "Please add a test.",
    );
    // A draft, the reviewer asked.
    branch(api, &repo, "bb-draft", &base);
    put_file(
        api,
        &repo,
        "bb-draft",
        "bb-draft.txt",
        "a line of the draft\n",
        false,
    );
    let draft = pull(api, &REPO, "bb-draft", &base, "Try another way", true);
    api.ok(
        Method::Patch,
        &format!("{repo}/issues/{draft}"),
        Some(json!({ "labels": ["ui"] })),
    );
    ask_reviewer(api, &repo, draft);
    // Two stacked.
    branch(api, &repo, "bb-stack-1", &base);
    put_file(
        api,
        &repo,
        "bb-stack-1",
        "bb-stack-1.txt",
        "the first of a stack\n",
        false,
    );
    branch(api, &repo, "bb-stack-2", "bb-stack-1");
    put_file(
        api,
        &repo,
        "bb-stack-2",
        "bb-stack-2.txt",
        "the second of a stack\n",
        false,
    );
    let first = pull(api, &REPO, "bb-stack-1", &base, "First of a stack", false);
    let second = pull(
        api,
        &REPO,
        "bb-stack-2",
        "bb-stack-1",
        "Second of a stack",
        false,
    );
    // One merged, one closed.
    let (status, _) = api.call(
        Method::Get,
        &format!("{repo}/contents/bb-merged.txt?ref={base}"),
        None,
    );
    let merged = if status == 200 {
        pull(api, &REPO, "bb-merged", &base, "Merged already", false)
    } else {
        branch(api, &repo, "bb-merged", &base);
        put_file(api, &repo, "bb-merged", "bb-merged.txt", "merged\n", false);
        let number = pull(api, &REPO, "bb-merged", &base, "Merged already", false);
        api.ok(
            Method::Put,
            &format!("{repo}/pulls/{number}/merge"),
            Some(json!({ "merge_method": "squash" })),
        );
        number
    };
    branch(api, &repo, "bb-closed", &base);
    put_file(api, &repo, "bb-closed", "bb-closed.txt", "closed\n", false);
    let closed = pull(
        api,
        &REPO,
        "bb-closed",
        &base,
        "Closed without merging",
        false,
    );
    api.ok(
        Method::Patch,
        &format!("{repo}/pulls/{closed}"),
        Some(json!({ "state": "closed" })),
    );
    // From the fork.
    let from_fork = pull(
        api,
        &REPO,
        &format!("{}:bb-fork-topic", main_login()),
        &base,
        "From a fork",
        false,
    );
    // Issues.
    let issue_one = issue(
        api,
        &repo,
        "The button does nothing",
        json!({ "body": "Clicking it does nothing at all.", "labels": ["bug"],
                "milestone": milestone, "assignees": [main_login()] }),
    );
    comment_once(api, &repo, issue_one, "Which button do you mean?");
    comment_once(&world.reviewer, &repo, issue_one, "The one at the top.");
    let closed_issue = issue(
        api,
        &repo,
        "Colours are off",
        json!({ "body": "Too bright.", "labels": ["ui"] }),
    );
    api.ok(
        Method::Patch,
        &format!("{repo}/issues/{closed_issue}"),
        Some(json!({ "state": "closed" })),
    );
    let plain_issue = issue(
        api,
        &repo,
        "Say what the menu is for",
        json!({ "body": "A line under it would do." }),
    );
    wait_for_checks(
        api,
        &repo,
        &[
            (repo.as_str(), "bb-topic"),
            (repo.as_str(), "bb-draft"),
            (repo.as_str(), "bb-stack-1"),
            (repo.as_str(), "bb-changes"),
            (fork.as_str(), "bb-fork-topic"),
        ],
    );
    // Statuses set on commits, as a service would, beside the checks.
    let shas = [
        head(api, &repo, "bb-topic"),
        head(api, &repo, "bb-draft"),
        head(api, &repo, "bb-stack-1"),
        head(api, &repo, "bb-changes"),
    ];
    for (sha, context, state) in [
        (&shas[0], "bb-lint", "success"),
        (&shas[0], "bb-test", "failure"),
        (&shas[1], "bb-test", "pending"),
        (&shas[2], "bb-test", "error"),
    ] {
        status_once(api, &repo, sha, context, state);
    }
    Reads {
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
    }
}

/// The commands' services, through the guard and recorded when asked.
struct Live {
    dir: PathBuf,
    services: Arc<Services>,
    recorder: Option<Arc<Recorder>>,
    real: Vec<String>,
    /// The machine account's id, and the reviewer's when connected.
    main: String,
    reviewer: String,
}

impl Live {
    /// The services, the machine account connected, and the reviewer too
    /// when `reviewer`.
    fn new(name: &str, reviewer: bool) -> Self {
        let world = the_world();
        let mut live = Self::bare(name, world.guard.clone(), world.real.clone(), true);
        live.main = live.connect(&world.main, main_login());
        if reviewer {
            live.reviewer = live.connect(&world.reviewer, reviewer_login());
        }
        live
    }

    /// The services through `guard`, no account connected, recorded when
    /// `record` and asked.
    fn bare(name: &str, guard: Arc<Guard>, real: Vec<String>, record: bool) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bb-g-live-github-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let guard: Arc<dyn Fetch> = guard;
        let recorder = record
            .then(|| {
                Recorder::from_environment(
                    guard.clone(),
                    name,
                    "Recorded from github.com by tests/suite/live_github.rs: the test \
                     organisation's public repositories, its machine account and a reviewer",
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
            real,
            main: String::new(),
            reviewer: String::new(),
        }
    }

    fn connect(&self, api: &Api, login: &str) -> String {
        let account = self.run(
            "account.add_token",
            json!({ "kind": "github", "token": api.token.expose() }),
        );
        assert_eq!(account["login"], login);
        account["id"].as_str().unwrap().to_owned()
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

    /// A command about one of the organisation's repositories, as an
    /// account.
    fn on(&self, account: &str, name: &str, id: &str, extra: Value) -> Result<Value, String> {
        let mut params = json!({ "account": account, "owner": org(), "name": name });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.try_run(id, params)
    }

    /// A command about the reads' repository, as the machine account.
    fn reads(&self, id: &str, extra: Value) -> Value {
        self.on(&self.main, &REPO, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    /// A command about the reads' repository, as the reviewer.
    fn reviewer_reads(&self, id: &str, extra: Value) -> Value {
        self.on(&self.reviewer, &REPO, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    /// A command about the writes' repository, as the machine account.
    fn writes(&self, id: &str, extra: Value) -> Result<Value, String> {
        self.on(&self.main, &WRITES, id, extra)
    }

    fn write(&self, id: &str, extra: Value) -> Value {
        self.writes(id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    /// A command about the writes' repository, as the reviewer.
    fn review(&self, id: &str, extra: Value) -> Value {
        self.on(&self.reviewer, &WRITES, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn finish(&self) {
        if let Some(recorder) = &self.recorder {
            let mut real = self.real.clone();
            // More that may not be in a recording, given when the test
            // runs and kept in no file.
            if let Ok(more) = std::env::var("BITBRAID_RECORD_FORBID") {
                real.extend(
                    more.split(',')
                        .map(|word| word.trim().to_owned())
                        .filter(|word| word.len() >= 3),
                );
            }
            // A name of the repositories of one's own is replaced, and has
            // to be gone.
            if live_repo() != DEFAULT_REPO {
                real.push(live_repo().to_owned());
            }
            let real: Vec<&str> = real.iter().map(String::as_str).collect();
            let named = NAMES.get().map(Names::stand_ins).unwrap_or_default();
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
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn a_token_connects_and_the_pull_requests_are_listed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("add_token_and_pr_list", false);
    let accounts = live.run("account.list", json!({}));
    let scopes = accounts[0]["scopes"].as_array().unwrap();
    for scope in ["repo", "workflow", "read:org", "admin:public_key"] {
        assert!(scopes.iter().any(|known| known == scope), "{scope}");
    }
    let listing = live.reads("pr.list", json!({ "state": "all" }));
    let found = numbers(&listing);
    assert!(found.contains(&reads.topic), "{found:?}");
    let topic = listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|pull| pull["number"] == reads.topic)
        .unwrap();
    assert_eq!(topic["head_ref"], "bb-topic");
    assert_eq!(topic["repo"], format!("{}/{REPO}", org()));
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn pull_requests_are_listed_by_every_filter_and_query_from_both_sides() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_list", true);
    let all = live.reads("pr.list", json!({ "state": "all" }));
    let found = numbers(&all);
    let open = [
        reads.topic,
        reads.changes,
        reads.draft,
        reads.stack[0],
        reads.stack[1],
        reads.fork,
    ];
    for number in open.iter().chain([&reads.merged, &reads.closed]) {
        assert!(found.contains(number), "#{number} in {found:?}");
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
    assert_eq!(topic["assignees"][0]["login"], main_login());
    assert_eq!(topic["ci"], "failure");
    assert!(topic["comments"].as_u64().unwrap() >= 2);
    assert_eq!(topic["reviewers"][0]["user"]["login"], reviewer_login());
    assert_eq!(topic["reviewers"][0]["state"], "approved");
    assert_eq!(
        row(reads.changes)["reviewers"][0]["state"],
        "changes_requested"
    );
    assert_eq!(row(reads.changes)["ci"], "success");
    let draft = row(reads.draft);
    assert_eq!(draft["draft"], true);
    assert_eq!(draft["ci"], "pending");
    assert_eq!(draft["reviewers"][0]["user"]["login"], reviewer_login());
    assert_eq!(draft["reviewers"][0]["state"], "pending");
    assert_eq!(row(reads.stack[0])["ci"], "failure");
    assert_eq!(row(reads.stack[1])["base_ref"], "bb-stack-1");
    assert_eq!(row(reads.merged)["state"], "merged");
    assert_eq!(row(reads.closed)["state"], "closed");
    let fork = row(reads.fork);
    assert_eq!(fork["head_repo"], format!("{}/{REPO}", main_login()));
    assert_eq!(fork["head_ref"], "bb-fork-topic");
    let list = |extra: Value| sorted(numbers(&live.reads("pr.list", extra)));
    let by_reviewer = |extra: Value| sorted(numbers(&live.reviewer_reads("pr.list", extra)));
    assert_eq!(list(json!({})), sorted(open.to_vec()));
    assert_eq!(list(json!({ "filter": "mine" })), sorted(open.to_vec()));
    assert_eq!(list(json!({ "filter": "assigned" })), [reads.topic]);
    assert_eq!(
        list(json!({ "filter": "review_requested" })),
        Vec::<u64>::new()
    );
    // The reviewer's side: asked on the draft, which they have not
    // answered; nothing of their own.
    assert_eq!(
        by_reviewer(json!({ "filter": "review_requested" })),
        [reads.draft]
    );
    assert_eq!(by_reviewer(json!({ "filter": "mine" })), Vec::<u64>::new());
    assert_eq!(
        by_reviewer(json!({ "filter": "all" })),
        sorted(open.to_vec())
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
    assert_eq!(
        list(json!({ "query": "status:failure label:bug" })),
        [reads.topic]
    );
    assert_eq!(list(json!({ "query": "milestone:v1" })), [reads.topic]);
    assert_eq!(list(json!({ "query": "review:approved" })), [reads.topic]);
    assert_eq!(
        list(json!({ "query": "review:changes_requested" })),
        [reads.changes]
    );
    assert_eq!(
        list(json!({ "query": format!("review-requested:{}", reviewer_login()) })),
        [reads.draft]
    );
    assert_eq!(list(json!({ "query": "head:bb-fork-topic" })), [reads.fork]);
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn a_pull_request_is_shown_with_its_check_runs_verdicts_and_conversations() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_show", false);
    let topic = live.reads("pr.show", json!({ "number": reads.topic }));
    assert_eq!(topic["pull"]["number"], reads.topic);
    assert_eq!(
        topic["body"],
        "Made by the live test: Add a file to review."
    );
    assert_eq!(topic["milestone"]["title"], "v1");
    assert_eq!(topic["pull"]["ci"], "failure");
    let checks: Vec<(String, String)> = topic["checks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|check| {
            (
                check["name"].as_str().unwrap().to_owned(),
                check["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (name, state) in [
        ("passes", "success"),
        ("fails", "failure"),
        ("skipped", "skipped"),
        ("bb-lint", "success"),
        ("bb-test", "failure"),
    ] {
        assert!(
            checks.contains(&(name.to_owned(), state.to_owned())),
            "{name} {state} in {checks:?}"
        );
    }
    assert_eq!(topic["files"][0]["path"], "bb-topic.txt");
    assert!(!topic["commits"].as_array().unwrap().is_empty());
    let threads = topic["threads"].as_array().unwrap();
    let on_lines: Vec<&Value> = threads
        .iter()
        .filter(|thread| thread["path"] == "bb-topic.txt")
        .collect();
    assert_eq!(on_lines.len(), 2, "{threads:?}");
    assert_eq!(on_lines[0]["resolved"], true);
    assert_eq!(on_lines[0]["comments"].as_array().unwrap().len(), 2);
    assert_eq!(on_lines[1]["resolved"], false);
    // The conversation on the whole of it, by both.
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
            (main_login(), "Looks good to me."),
            (reviewer_login(), "A second look: fine.")
        ]
    );
    let reviews = topic["reviews"].as_array().unwrap();
    assert!(reviews.iter().any(
        |review| review["state"] == "approved" && review["author"]["login"] == reviewer_login()
    ));
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
    assert!(
        verdicts.contains(&(reviewer_login(), "approved")),
        "{verdicts:?}"
    );
    let changes = live.reads("pr.show", json!({ "number": reads.changes }));
    assert_eq!(
        changes["pull"]["reviewers"][0]["state"],
        "changes_requested"
    );
    assert_eq!(changes["reviews"][0]["body"], "Please add a test.");
    assert_eq!(changes["pull"]["ci"], "success");
    let draft = live.reads("pr.show", json!({ "number": reads.draft }));
    assert_eq!(draft["pull"]["draft"], true);
    assert_eq!(draft["pull"]["reviewers"][0]["state"], "pending");
    let second = live.reads("pr.show", json!({ "number": reads.stack[1] }));
    assert_eq!(second["pull"]["base_ref"], "bb-stack-1");
    let fork = live.reads("pr.show", json!({ "number": reads.fork }));
    assert_eq!(
        fork["pull"]["head_repo"],
        format!("{}/{REPO}", main_login())
    );
    assert_eq!(fork["files"][0]["path"], "bb-fork.txt");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn choices_stacks_and_the_pull_request_of_a_branch() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_options_stack", false);
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
    let people: Vec<&str> = options["people"]
        .as_array()
        .unwrap()
        .iter()
        .map(|person| person["login"].as_str().unwrap())
        .collect();
    assert!(
        people.contains(&main_login()) && people.contains(&reviewer_login()),
        "{people:?}"
    );
    assert_eq!(
        options["merge_methods"],
        json!(["merge", "squash", "rebase"])
    );
    let stack = live.reads("pr.stack", json!({ "number": reads.stack[1] }));
    let stacked: Vec<u64> = stack["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["number"].as_u64().unwrap())
        .collect();
    assert_eq!(stacked, reads.stack);
    assert_eq!(stack["position"], 1);
    let branch = live.reads("pr.for_branch", json!({ "branch": "bb-stack-1" }));
    assert_eq!(branch["number"], reads.stack[0]);
    // A fork's branch is not one of the repository's own.
    for other in ["bb-fork-topic", "bb-no-such-branch"] {
        assert_eq!(
            live.reads("pr.for_branch", json!({ "branch": other })),
            Value::Null
        );
    }
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn the_inbox_gathers_the_machine_accounts_pull_requests_and_issues() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("inbox", false);
    let found = live.run("inbox.items", json!({ "wip": false }));
    assert_eq!(found["accounts"].as_array().unwrap().len(), 1, "{found:#}");
    assert!(found["accounts"][0]["error"].is_null(), "{found:#}");
    let pulls = found["pulls"].as_array().unwrap();
    let pull = |number: u64| {
        let key = format!("{}/pull/{}/{REPO}#{number}", live.main, org());
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
    // Every row is a test repository's: the guard took out the rest.
    assert!(pulls.iter().all(|pull| {
        let repo = pull["pull"]["repo"].as_str().unwrap();
        repo.starts_with(&format!("{}/{REPO}", org()))
    }));
    assert!(groups(reads.topic).contains(&"failing_checks".to_owned()));
    assert!(groups(reads.changes).contains(&"changes_requested".to_owned()));
    assert!(groups(reads.draft).contains(&"draft".to_owned()));
    assert!(pull(reads.fork)["pull"]["head_repo"].is_string());
    assert!(pull(reads.topic)["additions"].as_u64().is_some());
    assert_eq!(pull(reads.topic)["involvement"][0], "created");
    for number in [reads.merged, reads.closed] {
        let key = format!("{}/pull/{}/{REPO}#{number}", live.main, org());
        assert!(
            !pulls.iter().any(|pull| pull["key"] == key),
            "{key} is not open"
        );
    }
    let issues = found["issues"].as_array().unwrap();
    let issue = format!("{}/issue/{}/{REPO}#{}", live.main, org(), reads.issue);
    assert!(issues.iter().any(|row| row["key"] == issue), "{found:#}");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn issues_are_listed_by_every_filter_and_shown() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("issues", false);
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
    assert_eq!(list(json!({ "filter": "mentioned" })), Vec::<u64>::new());
    assert_eq!(list(json!({ "query": "labels:bug" })), [reads.issue]);
    assert_eq!(
        list(json!({ "query": "milestone:v1 button" })),
        [reads.issue]
    );
    assert_eq!(
        list(json!({ "query": "nothing-like-this" })),
        Vec::<u64>::new()
    );
    let shown = live.reads("issue.show", json!({ "number": reads.issue }));
    assert_eq!(shown["issue"]["title"], "The button does nothing");
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
            (main_login(), "Which button do you mean?"),
            (reviewer_login(), "The one at the top.")
        ]
    );
    let err = live
        .on(
            &live.main,
            &REPO,
            "issue.show",
            json!({ "number": reads.topic }),
        )
        .unwrap_err();
    assert!(err.contains("pull request"), "{err}");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn the_check_runs_statuses_and_authors_pictures_of_commits_are_read() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let world = the_world();
    // The account's own commits (the repository's first), and the address
    // they are by.
    let commits = world.main.ok(
        Method::Get,
        &format!("{}/commits?per_page=100", at(org(), &REPO)),
        None,
    );
    let own = commits
        .as_array()
        .unwrap()
        .iter()
        .find(|commit| commit["author"]["login"] == main_login())
        .expect("a commit of the machine account");
    let email = own["commit"]["author"]["email"]
        .as_str()
        .unwrap()
        .to_owned();
    let live = Live::new("ci_status_avatars", false);
    let unknown = "0123456789abcdef0123456789abcdef01234567".to_owned();
    let mut shas = reads.shas[..3].to_vec();
    shas.push(unknown);
    let statuses = live.reads("ci.status", json!({ "shas": shas }));
    let states: Vec<&str> = statuses
        .as_array()
        .unwrap()
        .iter()
        .map(|status| status["state"].as_str().unwrap())
        .collect();
    assert_eq!(states, ["failure", "pending", "failure", "none"]);
    // Three check runs of GitHub Actions and two statuses.
    let topic = statuses[0]["checks"].as_array().unwrap();
    assert_eq!(topic.len(), 5, "{topic:?}");
    let run = topic.iter().find(|check| check["name"] == "fails").unwrap();
    assert_eq!(run["state"], "failure");
    assert!(
        run["url"]
            .as_str()
            .unwrap()
            .contains(&format!("/{}/{REPO}/actions/runs/", org())),
        "{run}"
    );
    assert!(run["finished"].is_string());
    // The test's identity has no account: no picture; the account's
    // address has one.
    let pictures = live.reads("provider.avatars", json!({ "emails": [IDENTITY.1, email] }));
    let pictures = pictures.as_object().unwrap();
    assert_eq!(pictures.len(), 1);
    assert!(
        pictures
            .get(&email)
            .and_then(Value::as_str)
            .is_some_and(|url| url.starts_with("https://avatars.githubusercontent.com/")),
        "no picture for the account's address"
    );
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn repositories_and_where_one_may_be_made_are_listed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    reads();
    org_repository(&the_world().main, &WRITES);
    let live = Live::new("repos", false);
    let names = |extra: Value| {
        let mut params = json!({ "account": live.main });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        let listing = live.run("provider.repos", params);
        listing["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|repo| repo["full_name"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    let own = names(json!({ "filter": "own" }));
    assert!(own.contains(&format!("{}/{REPO}", main_login())), "{own:?}");
    assert!(
        own.iter()
            .all(|name| name.starts_with(&format!("{}/", main_login())))
    );
    let organisation = names(json!({ "filter": "organisation" }));
    for name in [&REPO, &WRITES] {
        assert!(
            organisation.contains(&format!("{}/{name}", org())),
            "{organisation:?}"
        );
    }
    assert_eq!(
        names(json!({ "filter": "starred" })),
        [format!("{}/{REPO}", org())]
    );
    let everything = names(json!({ "filter": "all" }));
    assert!(everything.contains(&format!("{}/{REPO}", main_login())));
    assert!(everything.contains(&format!("{}/{WRITES}", org())));
    // The list has no search: the words are looked for in what was read.
    let found = names(json!({ "filter": "all", "query": "writes" }));
    assert_eq!(found, [format!("{}/{WRITES}", org())]);
    let listing = live.run(
        "provider.repos",
        json!({ "account": live.main, "filter": "own" }),
    );
    let fork = listing["items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|repo| repo["name"] == REPO)
        .unwrap();
    assert_eq!(fork["private"], false);
    assert_eq!(
        fork["clone_https"],
        format!("https://github.com/{}/{REPO}.git", main_login())
    );
    assert_eq!(
        fork["clone_ssh"],
        format!("git@github.com:{}/{REPO}.git", main_login())
    );
    let namespaces = live.run("provider.namespaces", json!({ "account": live.main }));
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
    assert_eq!(places, [(main_login(), "user"), (org(), "organisation")]);
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn signing_in_by_code_starts_and_waits_for_the_user() {
    if std::env::var_os("BITBRAID_LIVE_GITHUB_ORG").is_none() {
        eprintln!("skipped: set BITBRAID_LIVE_GITHUB_ORG to run the live tests");
        return;
    }
    let _serial = serial();
    // No test account: the guard knows no token.
    let live = Live::bare(
        "device_flow",
        Arc::new(Guard::new(Names::default(), Vec::new())),
        Vec::new(),
        true,
    );
    let code = live.run("account.sign_in_start", json!({ "kind": "github" }));
    assert_eq!(code["verification_uri"], "https://github.com/login/device");
    let interval = code["interval"].as_u64().unwrap();
    assert!((1..=120).contains(&interval));
    assert!(code["expires_in"].as_u64().unwrap() > 60);
    std::thread::sleep(Duration::from_secs(interval));
    let polled = live.run(
        "account.sign_in_poll",
        json!({ "kind": "github", "device_code": code["device_code"], "interval": interval }),
    );
    assert_eq!(polled, json!({ "state": "waiting" }));
    live.finish();
}

/// The whole way: a code is asked for and written, with where to type it,
/// to the file `BITBRAID_LIVE_GITHUB_DEVICE` names (`<user_code>
/// <verification_uri>`); someone signed in at github.com as the machine
/// account approves it there; the test asks at the interval given, for up
/// to twelve minutes, and then writes `done`, `denied` or `expired` to
/// that file's name with `.result` after it. The token is held in memory
/// only, used for `GET /user` alone (which has to answer the machine
/// account), and dropped with the account at the end; it is not revoked
/// (that takes the application's client secret, which the test does not hold).
#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_DEVICE=<file> and someone to approve the code"]
fn signing_in_by_code_ends_with_the_machine_account() {
    let Some(file) = std::env::var_os("BITBRAID_LIVE_GITHUB_DEVICE").map(PathBuf::from) else {
        panic!("set BITBRAID_LIVE_GITHUB_DEVICE to the file the code is written to");
    };
    let machine = keychain_login(Role::Main.item())
        .expect("no login is stored with the keychain item of the machine account");
    let mut result = file.clone().into_os_string();
    result.push(".result");
    let result = PathBuf::from(result);
    let _ = std::fs::remove_file(&result);
    let _serial = serial();
    // Never recorded: the answer that ends it holds the token.
    let live = Live::bare(
        "device_flow_approved",
        Arc::new(Guard::new(Names::default(), Vec::new())),
        Vec::new(),
        false,
    );
    let code = live.run("account.sign_in_start", json!({ "kind": "github" }));
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
            json!({ "kind": "github", "device_code": code["device_code"], "interval": interval }),
        );
        match polled["state"].as_str() {
            Some("waiting") => {}
            Some("slow_down") => interval = polled["interval"].as_u64().unwrap_or(interval + 5),
            Some("denied") => break "denied",
            Some("expired") => break "expired",
            Some("done") => {
                // Its `GET /user` answered: it has to be the machine account.
                let account = &polled["account"];
                let login = account["login"].as_str().unwrap_or_default().to_owned();
                live.run("account.remove", json!({ "id": account["id"] }));
                if login != machine {
                    std::fs::write(&result, "wrong account\n").unwrap();
                    panic!("the code was approved by another account than the machine account");
                }
                break "done";
            }
            other => panic!("account.sign_in_poll said {other:?}"),
        }
    };
    std::fs::write(&result, format!("{outcome}\n")).unwrap();
    // The token goes with the services' keychain, which is in memory.
    live.finish();
    drop(live);
    assert_eq!(outcome, "done");
}

// The writes: pull requests made, changed, reviewed, commented on and
// merged in the organisation's `<repo>-writes`, so that the reads keep what they
// are tested on.

/// Something new for this run, for the names of files.
fn run_tag() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    format!("{:x}", nanos ^ u128::from(std::process::id()))
}

/// A branch of the writes' repository made anew from `from`, with `path`
/// written: the pull requests open from it before are closed and it is
/// removed first.
fn fresh_branch(api: &Api, branch: &str, from: &str, path: &str, text: &str) {
    let repo = at(org(), &WRITES);
    let (_, open) = api.call(
        Method::Get,
        &format!("{repo}/pulls?state=open&head={}:{branch}", org()),
        None,
    );
    for pull in open.as_array().cloned().unwrap_or_default() {
        api.ok(
            Method::Patch,
            &format!("{repo}/pulls/{}", pull["number"]),
            Some(json!({ "state": "closed" })),
        );
    }
    let _ = api.call(
        Method::Delete,
        &format!("{repo}/git/refs/heads/{branch}"),
        None,
    );
    let sha = head(api, &repo, from);
    api.ok(
        Method::Post,
        &format!("{repo}/git/refs"),
        Some(json!({ "ref": format!("refs/heads/{branch}"), "sha": sha })),
    );
    put_file(api, &repo, branch, path, text, true);
}

/// What the writes start from: the writes' repository made when missing,
/// its branches made anew; the milestone's number and the file of
/// `bb-w-one`.
fn start_writes() -> (u64, String) {
    let api = &the_world().main;
    let repo = at(org(), &WRITES);
    let base = org_repository(api, &WRITES);
    assert_eq!(base, "main");
    let milestone = labels_and_milestone(api, &repo);
    let run = run_tag();
    for branch in [
        "bb-w-one",
        "bb-w-alt",
        "bb-w-merge",
        "bb-w-moved",
        "bb-w-stack-1",
        "bb-w-rebase",
    ] {
        fresh_branch(
            api,
            branch,
            &base,
            &format!("bb-w/{branch}-{run}.txt"),
            &format!("{branch}\n"),
        );
    }
    fresh_branch(
        api,
        "bb-w-stack-2",
        "bb-w-stack-1",
        &format!("bb-w/bb-w-stack-2-{run}.txt"),
        "bb-w-stack-2\n",
    );
    // Two that change the same line.
    for (branch, text) in [
        ("bb-w-conflict-a", "one way"),
        ("bb-w-conflict-b", "another way"),
    ] {
        fresh_branch(
            api,
            branch,
            &base,
            "bb-w/conflict.txt",
            &format!("{text} {run}\n"),
        );
    }
    (milestone, format!("bb-w/bb-w-one-{run}.txt"))
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn pull_requests_are_made_changed_reviewed_commented_on_and_merged() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let world = the_world();
    let (milestone, file) = start_writes();
    let api = &world.main;
    let repo = at(org(), &WRITES);
    let live = Live::new("pr_writes", true);
    let made = live.write(
        "pr.create",
        json!({ "head": "bb-w-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true, "reviewers": [reviewer_login()],
                "assignees": [main_login()], "labels": ["bug"], "milestone": milestone.to_string() }),
    );
    let number = made["number"].as_u64().unwrap();
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["assignees"][0]["login"], main_login());
    assert_eq!(made["reviewers"][0]["user"]["login"], reviewer_login());
    assert_eq!(made["reviewers"][0]["state"], "pending");
    // One's own login is not one GitHub asks to review: the pull request
    // is there, and the error says so.
    let err = live
        .writes(
            "pr.create",
            json!({ "head": "bb-w-alt", "base": "main", "title": "Asked of myself",
                    "reviewers": [main_login()] }),
        )
        .unwrap_err();
    assert!(err.contains("was opened, but its reviewers"), "{err}");
    let alt = live.write("pr.for_branch", json!({ "branch": "bb-w-alt" }))["number"]
        .as_u64()
        .unwrap();
    // The author's own reviews: a comment is taken; a verdict is not.
    let commented = live.write(
        "pr.review",
        json!({ "number": number, "event": "comment", "body": "A look of my own." }),
    );
    assert_eq!(commented["state"], "commented");
    let err = live
        .writes("pr.review", json!({ "number": number, "event": "approve" }))
        .unwrap_err();
    assert!(err.contains("422"), "{err}");
    let err = live
        .writes(
            "pr.review",
            json!({ "number": number, "event": "request_changes" }),
        )
        .unwrap_err();
    assert!(err.contains("something written"), "{err}");
    // Comments: on the conversation, an answer to one (quoting it), and an
    // answer in the thread of a comment on a line, by the reviewer.
    let comment = live.write(
        "pr.comment",
        json!({ "number": number, "body": "A comment from the live test." }),
    );
    let quoted = live.write(
        "pr.comment",
        json!({ "number": number, "body": "An answer to it.", "reply_to": comment["id"] }),
    );
    assert_eq!(
        quoted["body"],
        "> A comment from the live test.\n\nAn answer to it."
    );
    api.ok(
        Method::Post,
        &format!("{repo}/pulls/{number}/reviews"),
        Some(json!({ "event": "COMMENT", "body": "",
                     "comments": [ { "path": file, "line": 1, "body": "On this line." } ] })),
    );
    let shown = live.write("pr.show", json!({ "number": number }));
    let thread = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["path"] == file.as_str())
        .unwrap()["id"]
        .clone();
    let answer = live.review(
        "pr.comment",
        json!({ "number": number, "body": "An answer on the line.", "reply_to": thread }),
    );
    assert_eq!(answer["body"], "An answer on the line.");
    assert_eq!(answer["author"]["login"], reviewer_login());
    // Changed: a new title, ready, other labels, nobody assigned, no
    // milestone, another base; then a draft again on its first base; then
    // ready.
    let changed = live.write(
        "pr.update",
        json!({ "number": number, "title": "Change one, better", "draft": false,
                "labels": ["ui"], "assignees": [], "milestone": "", "base": "bb-w-alt" }),
    );
    assert_eq!(changed["pull"]["title"], "Change one, better");
    assert_eq!(changed["pull"]["draft"], false);
    assert_eq!(changed["pull"]["labels"][0]["name"], "ui");
    assert_eq!(changed["pull"]["assignees"], json!([]));
    assert_eq!(changed["pull"]["base_ref"], "bb-w-alt");
    assert_eq!(changed["milestone"], Value::Null);
    let drafted = live.write(
        "pr.update",
        json!({ "number": number, "draft": true, "base": "main" }),
    );
    assert_eq!(drafted["pull"]["draft"], true);
    assert_eq!(drafted["pull"]["base_ref"], "main");
    live.write("pr.update", json!({ "number": number, "draft": false }));
    // The reviewer's verdicts: approved, and changes asked on the other.
    let approved = live.review(
        "pr.review",
        json!({ "number": number, "event": "approve", "body": "Fine by me." }),
    );
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], reviewer_login());
    let asked = live.review(
        "pr.review",
        json!({ "number": alt, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(asked["state"], "changes_requested");
    let shown = live.write("pr.show", json!({ "number": number }));
    let verdicts: Vec<(&str, &str)> = shown["pull"]["reviewers"]
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
    assert!(
        verdicts.contains(&(reviewer_login(), "approved")),
        "{verdicts:?}"
    );
    // Merged, squashed, its branch removed; merged again: refused.
    let merged = live.write(
        "pr.merge",
        json!({ "number": number, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["branch_deleted"], true);
    let err = live
        .writes("pr.merge", json!({ "number": number, "method": "merge" }))
        .unwrap_err();
    assert!(err.contains("merged already"), "{err}");
    // A merge commit.
    let with_commit = live.write(
        "pr.create",
        json!({ "head": "bb-w-merge", "base": "main", "title": "Merged with a commit" }),
    )["number"]
        .as_u64()
        .unwrap();
    let merged = live.write(
        "pr.merge",
        json!({ "number": with_commit, "method": "merge", "title": "Merge it" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["sha"].as_str().unwrap().len(), 40);
    // Two that change the same line: the second cannot be merged once the
    // first is.
    let first = live.write(
        "pr.create",
        json!({ "head": "bb-w-conflict-a", "base": "main", "title": "One way" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = live.write(
        "pr.create",
        json!({ "head": "bb-w-conflict-b", "base": "main", "title": "Another way" }),
    )["number"]
        .as_u64()
        .unwrap();
    let merged = live.write("pr.merge", json!({ "number": first, "method": "merge" }));
    assert_eq!(merged["merged"], true, "{merged}");
    let err = live
        .writes("pr.merge", json!({ "number": second, "method": "merge" }))
        .unwrap_err();
    assert!(err.contains("405"), "{err}");
    // A head that moves between the read and the merge: refused.
    let moved = live.write(
        "pr.create",
        json!({ "head": "bb-w-moved", "base": "main", "title": "Moved under it" }),
    )["number"]
        .as_u64()
        .unwrap();
    let pusher = api.clone();
    let pushed = repo.clone();
    world.guard.before(
        Method::Put,
        &format!("{repo}/pulls/{moved}/merge"),
        Box::new(move || {
            put_file(
                &pusher,
                &pushed,
                "bb-w-moved",
                "bb-w/moved.txt",
                &format!("moved {}\n", run_tag()),
                true,
            );
            // Until GitHub knows the new head can be merged, it answers
            // that it cannot be, whatever the head asked for.
            let new = head(&pusher, &pushed, "bb-w-moved");
            let deadline = Instant::now() + Duration::from_secs(90);
            loop {
                let pull = pusher.ok(Method::Get, &format!("{pushed}/pulls/{moved}"), None);
                if pull["head"]["sha"] == new.as_str() && pull["mergeable"] == true {
                    break;
                }
                assert!(Instant::now() < deadline, "#{moved} stayed unmergeable");
                std::thread::sleep(Duration::from_secs(3));
            }
        }),
    );
    let err = live
        .writes("pr.merge", json!({ "number": moved, "method": "squash" }))
        .unwrap_err();
    assert!(err.contains("409"), "{err}");
    live.finish();
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn a_stack_moves_down_when_its_bottom_is_merged() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    start_writes();
    let live = Live::new("pr_merge_stack", false);
    let first = live.write(
        "pr.create",
        json!({ "head": "bb-w-stack-1", "base": "main", "title": "First of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = live.write(
        "pr.create",
        json!({ "head": "bb-w-stack-2", "base": "bb-w-stack-1", "title": "Second of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let stack = live.write("pr.stack", json!({ "number": second }));
    assert_eq!(stack["position"], 1, "{stack}");
    let merged = live.write(
        "pr.merge",
        json!({ "number": first, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "title": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["retargeted"], json!([second]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = live.write("pr.for_branch", json!({ "branch": "bb-w-stack-2" }));
    assert_eq!(moved["base_ref"], "main");
    assert_eq!(moved["state"], "open");
    let rebased = live.write(
        "pr.create",
        json!({ "head": "bb-w-rebase", "base": "main", "title": "Replayed onto main" }),
    )["number"]
        .as_u64()
        .unwrap();
    let merged = live.write("pr.merge", json!({ "number": rebased, "method": "rebase" }));
    assert_eq!(merged["merged"], true, "{merged}");
    assert_eq!(merged["branch_deleted"], false);
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
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn a_pull_request_from_a_fork_is_checked_out_as_git_would() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let reads = reads();
    let live = Live::new("pr_checkout", false);
    let home = live.dir.join("home");
    let git = |dir: &Path| bitbraid_testkit::Git::new(dir, &home);
    git(&live.dir).ensure_home().unwrap();
    let origin = format!("https://github.com/{}/{REPO}.git", org());
    // Two clones of the public repository: ours, and git's twin.
    for clone in ["ours", "twin"] {
        git(&live.dir)
            .run(["clone", "-q", origin.as_str(), clone])
            .unwrap();
    }
    let ours = live.dir.join("ours");
    let twin = git(&live.dir.join("twin"));
    let out = live
        .run_in(
            &ours,
            "pr.checkout",
            json!({ "account": live.main, "owner": org(), "name": REPO,
                    "number": reads.fork, "remote": "origin" }),
        )
        .unwrap();
    assert_eq!(out["branch"], "bb-fork-topic");
    assert_eq!(out["remote_added"], main_login());
    assert_eq!(out["upstream"], format!("{}/bb-fork-topic", main_login()));
    assert_eq!(out["created"], true);
    let fork = format!("https://github.com/{}/{REPO}.git", main_login());
    let reference = format!("refs/pull/{}/head", reads.fork);
    for args in [
        vec!["remote", "add", main_login(), fork.as_str()],
        vec!["fetch", "-q", main_login()],
        vec!["fetch", "-q", "origin", reference.as_str()],
        vec!["branch", "--no-track", "bb-fork-topic", "FETCH_HEAD"],
        vec!["switch", "-q", "bb-fork-topic"],
    ] {
        twin.run(args).unwrap();
    }
    let upstream = format!("--set-upstream-to={}/bb-fork-topic", main_login());
    twin.run(["branch", "-q", upstream.as_str(), "bb-fork-topic"])
        .unwrap();
    assert_eq!(state(&git(&ours)), state(&twin));
    assert_eq!(
        out["commit"],
        twin.rev_parse("bb-fork-topic").unwrap().as_str()
    );
    // Again: nothing more is added, and the branch is the same.
    let again = live
        .run_in(
            &ours,
            "pr.checkout",
            json!({ "account": live.main, "owner": org(), "name": REPO,
                    "number": reads.fork, "remote": "origin" }),
        )
        .unwrap();
    assert_eq!(again["created"], false);
    assert_eq!(again["remote_added"], Value::Null);
    assert_eq!(state(&git(&ours)), state(&twin));
    live.finish();
}

/// The key of the tests, made by `ssh-keygen -t ed25519` (its private
/// half was thrown away), and its fingerprint as `ssh-keygen -l` says it.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
const FINGERPRINT: &str = "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0";

/// Removes what the test of making things made, whatever happens: the
/// two repositories and the keys of the test's title.
struct Gone;

impl Gone {
    fn now() {
        let api = &the_world().main;
        for repo in [at(org(), &MADE_IN_ORG), at(main_login(), &MADE_OWN)] {
            let (status, _) = api.call(Method::Delete, &repo, None);
            assert!(matches!(status, 204 | 404), "DELETE {repo}: {status}");
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
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn repositories_are_made_and_forked_and_a_key_uploaded_and_all_removed() {
    if world().is_none() {
        return;
    }
    let _serial = serial();
    reads();
    // What a run before left, if it stopped half-way.
    Gone::now();
    let _gone = Gone;
    let live = Live::new("repos_forks_keys", false);
    let home = live.dir.join("home");
    let dir = live.dir.join("repository");
    std::fs::create_dir_all(&dir).unwrap();
    let git = bitbraid_testkit::Git::new(&dir, &home);
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    let origin = format!("https://github.com/{}/{REPO}.git", org());
    git.run(["remote", "add", "origin", origin.as_str()])
        .unwrap();
    let made = live
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": live.main, "namespace": org(), "name": MADE_IN_ORG,
                    "private": true, "description": "Made by BitBraid's live test",
                    "remote": "made" }),
        )
        .unwrap();
    assert_eq!(
        made["repo"]["full_name"],
        format!("{}/{MADE_IN_ORG}", org())
    );
    assert_eq!(made["repo"]["private"], true);
    assert_eq!(made["remote"], "made");
    let mine = live
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": live.main, "name": MADE_OWN, "private": false,
                    "remote": "mine" }),
        )
        .unwrap();
    assert_eq!(
        mine["repo"]["full_name"],
        format!("{}/{MADE_OWN}", main_login())
    );
    assert_eq!(mine["repo"]["private"], false);
    // The fork is there already: GitHub answers it again.
    let forked = live
        .run_in(&dir, "provider.fork", json!({ "remote": "origin" }))
        .unwrap();
    assert_eq!(
        forked["repo"]["full_name"],
        format!("{}/{REPO}", main_login())
    );
    assert_eq!(forked["repo"]["parent"], format!("{}/{REPO}", org()));
    assert_eq!(forked["remote"], main_login());
    let remotes = git.run(["remote", "-v"]).unwrap();
    for (name, full) in [
        ("made", format!("{}/{MADE_IN_ORG}", org())),
        ("mine", format!("{}/{MADE_OWN}", main_login())),
        (main_login(), format!("{}/{REPO}", main_login())),
    ] {
        assert!(
            remotes.contains(&format!("{name}\thttps://github.com/{full}.git (fetch)")),
            "{remotes}"
        );
    }
    let key = live.dir.join("bitbraid-live.pub");
    std::fs::write(&key, KEY).unwrap();
    let added = live.run(
        "provider.ssh_key_add",
        json!({ "account": live.main, "title": KEY_TITLE, "path": key.to_str().unwrap() }),
    );
    assert_eq!(added["title"], KEY_TITLE);
    assert_eq!(added["fingerprint"], FINGERPRINT);
    let id = added["id"].as_str().unwrap().to_owned();
    live.finish();
    // Removed in the same run.
    let api = &the_world().main;
    let (status, _) = api.call(Method::Delete, &format!("/user/keys/{id}"), None);
    assert_eq!(status, 204);
    for repo in [at(org(), &MADE_IN_ORG), at(main_login(), &MADE_OWN)] {
        let (status, _) = api.call(Method::Delete, &repo, None);
        assert_eq!(status, 204, "DELETE {repo}");
    }
}

/// Suggestions from local edits: a pull request changes a file;
/// a repository here has its head checked out and the file edited; what
/// `pr.suggestions` makes of the edits is sent with `pr.comment` on the
/// lines, and the pull request shows a suggestion block on each. The pull
/// request is closed and its branch and the file removed at the end.
#[test]
#[ignore = "live: needs BITBRAID_LIVE_GITHUB_ORG and the test accounts in the keychain"]
fn local_edits_are_offered_and_sent_as_suggestions() {
    use crate::suggestions;
    if world().is_none() {
        return;
    }
    let _serial = serial();
    let api = &the_world().main;
    let repo = at(org(), &WRITES);
    let base = org_repository(api, &WRITES);
    put_file(
        api,
        &repo,
        &base,
        suggestions::FILE,
        suggestions::BASE,
        true,
    );
    fresh_branch(
        api,
        suggestions::BRANCH,
        &base,
        suggestions::FILE,
        suggestions::HEAD,
    );
    let made = api.ok(
        Method::Post,
        &format!("{repo}/pulls"),
        Some(
            json!({ "title": "Suggestions from local edits", "head": suggestions::BRANCH,
                     "base": base, "body": "Made by the live test." }),
        ),
    );
    let number = made["number"].as_u64().unwrap();
    let here = std::env::temp_dir().join(format!("bb-t-suggest-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&here);
    let local = suggestions::local_repo(&here.join("work"), &here.join("home"));
    let live = Live::new("pr_suggestions", false);
    let on = |extra: Value| {
        let mut params = json!({ "account": live.main, "owner": org(), "name": WRITES });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        params
    };
    let found = live.write("pr.for_branch", json!({ "branch": suggestions::BRANCH }));
    assert_eq!(found["number"].as_u64(), Some(number));
    let offered = live
        .run_in(&local, "pr.suggestions", on(json!({ "number": number })))
        .unwrap();
    assert_eq!(suggestions::shape(&offered), suggestions::expected());
    assert_eq!(offered["suggestions"][0]["body"], "```suggestion\nONE\n```");
    for suggestion in offered["suggestions"].as_array().unwrap() {
        let mut params = suggestions::sent(suggestion);
        params["number"] = json!(number);
        let comment = live.write("pr.comment", params);
        assert_eq!(comment["body"], suggestion["body"]);
    }
    let shown = live.write("pr.show", json!({ "number": number }));
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
        Method::Patch,
        &format!("{repo}/pulls/{number}"),
        Some(json!({ "state": "closed" })),
    );
    let _ = api.call(
        Method::Delete,
        &format!("{repo}/git/refs/heads/{}", suggestions::BRANCH),
        None,
    );
    let (_, file) = api.call(
        Method::Get,
        &format!("{repo}/contents/{}?ref={base}", suggestions::FILE),
        None,
    );
    if let Some(sha) = file["sha"].as_str() {
        api.ok(
            Method::Delete,
            &format!("{repo}/contents/{}", suggestions::FILE),
            Some(
                json!({ "message": format!("Remove {}", suggestions::FILE), "sha": sha,
                         "branch": base }),
            ),
        );
    }
    let _ = std::fs::remove_dir_all(&here);
}
