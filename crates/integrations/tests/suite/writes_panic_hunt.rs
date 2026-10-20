//! Panic hunt for the provider commands that change things: making,
//! changing, merging, reviewing and commenting on pull requests, checking
//! one out, making and forking repositories and uploading keys. Each with
//! hostile parameters, which are refused before anything is asked; and
//! against a provider whose every answer is hostile: wrong shapes, errors
//! with odd bodies, a merge answered 405, addresses git would hand to a
//! helper. Each has to end in an answer or an error, never a panic or a
//! wait without end, and nothing a provider sends becomes a remote unless
//! it is a repository at the account's site.

use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{
    Services,
    http::{Call, Fetch, FetchError, Response},
    replay::Replay,
    secrets::{MemoryStore, Secret, SecretStore},
};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

const GITHUB: &str = "github:github.com:me";
const GITLAB: &str = "gitlab:gitlab.example.org:me";

const ACCOUNTS: &str = "[[account]]\nid = \"github:github.com:me\"\nkind = \"github\"\n\
web = \"https://github.com\"\napi = \"https://api.github.com\"\nlogin = \"me\"\n\
sign_in = \"token\"\nscopes = []\ndefault = true\n\n\
[[account]]\nid = \"gitlab:gitlab.example.org:me\"\nkind = \"gitlab\"\n\
web = \"https://gitlab.example.org\"\napi = \"https://gitlab.example.org/api/v4\"\n\
login = \"me\"\nsign_in = \"token\"\nscopes = []\ndefault = true\n";

/// The longest body a command sends, as the commands hold it.
const MAX_SENT_BODY: usize = 256 * 1024;

/// A configuration directory with both accounts, and a repository whose
/// remotes are at their sites, and one on this machine that is not there.
struct World {
    dir: PathBuf,
    secrets: Arc<MemoryStore>,
    repo: Scratch,
}

impl World {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-p3-hunt-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("integrations.toml"), ACCOUNTS).unwrap();
        let secrets = Arc::new(MemoryStore::new());
        for id in [GITHUB, GITLAB] {
            secrets.set(id, "test", &Secret::new("token")).unwrap();
        }
        let repo = Scratch::of("linear").unwrap();
        for (name, address) in [
            ("origin", "git@github.com:me/proj.git"),
            ("lab", "git@gitlab.example.org:me/proj.git"),
            ("local", "/nonexistent/bb-p3/proj.git"),
        ] {
            repo.git().run(["remote", "add", name, address]).unwrap();
        }
        Self { dir, secrets, repo }
    }

    fn remotes(&self) -> String {
        self.repo.git().run(["remote", "-v"]).unwrap()
    }

    fn branches(&self) -> String {
        self.repo
            .git()
            .run([
                "for-each-ref",
                "--format=%(refname) %(objectname)",
                "refs/heads",
            ])
            .unwrap()
    }

    /// Run `id` with the repository open, the network `fetch`, and the
    /// wish to stop set after `stop_after`: the answer or error, and how
    /// long it took.
    fn run(
        &self,
        fetch: Arc<dyn Fetch>,
        id: &str,
        params: &Value,
        stop_after: Duration,
    ) -> (Result<Value, CommandError>, Duration) {
        let services = Arc::new(Services::new(fetch, self.secrets.clone(), &self.dir));
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(services);
        registry
            .run(
                "repo.open",
                &mut ctx,
                &json!({ "path": self.repo.path().to_str().unwrap() }),
            )
            .unwrap();
        // The wish to stop comes after a while, unless the command is done
        // before (its end drops `done`).
        let cancel = ctx.cancel.clone();
        let (done, waiting) = std::sync::mpsc::channel::<()>();
        let stopper = std::thread::spawn(move || {
            if let Err(std::sync::mpsc::RecvTimeoutError::Timeout) =
                waiting.recv_timeout(stop_after)
            {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        let started = Instant::now();
        let out = registry.run(id, &mut ctx, params);
        let took = started.elapsed();
        drop(done);
        let _ = stopper.join();
        (out, took)
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The parameters of a command about `me/proj` at `account`, with `extra`.
fn on(account: &str, extra: Value) -> Value {
    let mut params = json!({ "account": account, "owner": "me", "name": "proj" });
    for (key, value) in extra.as_object().unwrap() {
        params[key] = value.clone();
    }
    params
}

#[test]
fn hostile_parameters_of_every_writer_are_refused_before_anything_is_asked() {
    let world = World::new();
    let key = world.dir.join("key.pub");
    std::fs::write(&key, "-----BEGIN OPENSSH PRIVATE KEY-----\nabc\n").unwrap();
    let not_a_file = world.dir.join("dir.pub");
    std::fs::create_dir_all(&not_a_file).unwrap();
    let big_key = world.dir.join("big.pub");
    std::fs::write(&big_key, format!("ssh-rsa {}", "A".repeat(20 * 1024))).unwrap();
    let just_over = |limit: usize| "x".repeat(limit + 1);
    let many: Vec<String> = (0..101).map(|n| format!("u{n}")).collect();
    let pull = |extra: Value| {
        let mut params = json!({ "head": "topic", "base": "main", "title": "T" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        params
    };
    let mut cases: Vec<(&str, Value)> = Vec::new();
    for account in [GITHUB, GITLAB] {
        for extra in [
            json!({ "title": just_over(4096) }),
            json!({ "title": "a\u{0}b" }),
            json!({ "title": "line\nbreak" }),
            json!({ "title": "   " }),
            json!({ "body": just_over(MAX_SENT_BODY) }),
            json!({ "body": "bell\u{7}" }),
            json!({ "head": "--upload-pack=touch /tmp/x" }),
            json!({ "head": "-f" }),
            json!({ "base": "a..b" }),
            json!({ "base": "main~1" }),
            json!({ "head": just_over(255) }),
            json!({ "reviewers": many }),
            json!({ "reviewers": ["ok", "bad\u{1b}[31m"] }),
            json!({ "labels": [""] }),
            json!({ "assignees": "not a list" }),
            json!({ "milestone": just_over(64) }),
            json!({ "draft": "yes" }),
        ] {
            cases.push(("pr.create", on(account, pull(extra))));
        }
        for extra in [
            json!({ "number": 0, "title": "T" }),
            json!({ "number": -1, "title": "T" }),
            json!({ "number": u64::from(u32::MAX) + 1, "title": "T" }),
            json!({ "number": 1e300, "title": "T" }),
            json!({ "number": "7", "title": "T" }),
            json!({ "number": 1 }),
            json!({ "number": 1, "base": "--orphan" }),
            json!({ "number": 1, "milestone": just_over(64) }),
            json!({ "number": 1, "body": just_over(MAX_SENT_BODY) }),
        ] {
            cases.push(("pr.update", on(account, extra)));
        }
        for extra in [
            json!({ "number": 1 }),
            json!({ "number": 1, "method": "octopus" }),
            json!({ "number": 1, "method": "merge", "delete_branch": "true" }),
            json!({ "number": 1, "method": "merge", "retarget_stack": 1 }),
            json!({ "number": 1, "method": "merge", "title": just_over(1024) }),
            json!({ "number": 1, "method": "merge", "message": just_over(MAX_SENT_BODY) }),
            json!({ "number": 0, "method": "merge" }),
        ] {
            cases.push(("pr.merge", on(account, extra)));
        }
        for extra in [
            json!({ "number": 1 }),
            json!({ "number": 1, "event": "reject" }),
            json!({ "number": 1, "event": "comment", "body": just_over(MAX_SENT_BODY) }),
        ] {
            cases.push(("pr.review", on(account, extra)));
        }
        for extra in [
            json!({ "number": 1 }),
            json!({ "number": 1, "body": "  \n " }),
            json!({ "number": 1, "body": "hi", "reply_to": "../../users" }),
            json!({ "number": 1, "body": "hi", "reply_to": "1 OR 1" }),
            json!({ "number": 1, "body": "hi", "reply_to": just_over(128) }),
            // On lines of a file.
            json!({ "number": 1, "body": "hi", "path": "a.txt", "reply_to": "12" }),
            json!({ "number": 1, "body": "hi", "path": "a.txt" }),
            json!({ "number": 1, "body": "hi", "path": "a.txt", "line": 0 }),
            json!({ "number": 1, "body": "hi", "path": "a.txt", "line": 5, "start_line": 9 }),
            json!({ "number": 1, "body": "hi", "path": "a.txt", "line": u64::MAX }),
            json!({ "number": 1, "body": "hi", "line": 5 }),
            json!({ "number": 1, "body": "hi", "path": "/etc/passwd", "line": 1 }),
            json!({ "number": 1, "body": "hi", "path": just_over(4096), "line": 1 }),
        ] {
            cases.push(("pr.comment", on(account, extra)));
        }
        for extra in [
            json!({ "number": 0 }),
            json!({ "number": 1, "path": "../outside" }),
            json!({ "number": 1, "path": "/etc/passwd" }),
            json!({ "number": 1, "note": just_over(4096) }),
            json!({ "number": 1, "note": 7 }),
        ] {
            cases.push(("pr.suggestions", on(account, extra)));
        }
        for extra in [
            json!({ "number": 0 }),
            json!({ "number": 1, "dirty": "maybe" }),
            json!({ "number": 1, "remote": "nowhere" }),
            json!({ "number": 1, "remote": just_over(100) }),
        ] {
            cases.push(("pr.checkout", on(account, extra)));
        }
        for extra in [
            json!({ "number": 0 }),
            json!({ "number": 1, "remote": "nowhere" }),
            json!({ "number": 1, "remote": just_over(100) }),
        ] {
            cases.push(("pr.head", on(account, extra)));
        }
        for params in [
            json!({ "account": account, "name": "x.git", "private": true }),
            json!({ "account": account, "name": "..", "private": true }),
            json!({ "account": account, "name": "a/b", "private": true }),
            json!({ "account": account, "name": "x", "private": "no" }),
            json!({ "account": account, "name": "x" }),
            json!({ "account": account, "name": "x", "private": true, "push": true }),
            json!({ "account": account, "name": "x", "private": true, "remote": "-x" }),
            json!({ "account": account, "name": "x", "private": true, "protocol": "ftp" }),
            json!({ "account": account, "name": "x", "private": true,
                    "description": just_over(2048) }),
        ] {
            cases.push(("provider.repo_create", params));
        }
        for path in [
            key.to_str().unwrap().to_owned(),
            not_a_file.to_str().unwrap().to_owned(),
            big_key.to_str().unwrap().to_owned(),
            "relative/key.pub".to_owned(),
            "/nonexistent/key.pub".to_owned(),
            "/etc/passwd".to_owned(),
        ] {
            cases.push((
                "provider.ssh_key_add",
                json!({ "account": account, "title": "T", "path": path }),
            ));
        }
        cases.push((
            "provider.ssh_key_add",
            json!({ "account": account, "title": just_over(255), "path": "/x.pub" }),
        ));
    }
    cases.push(("provider.fork", json!({ "remote": "local" })));
    cases.push(("provider.fork", json!({ "remote": "nowhere" })));
    let remotes = world.remotes();
    let branches = world.branches();
    for (id, params) in cases {
        let replay = Arc::new(Replay::from_json(&json!({ "exchanges": [] }).to_string()).unwrap());
        let (out, _) = world.run(replay.clone(), id, &params, Duration::from_secs(60));
        assert!(out.is_err(), "{id} {params}");
        assert!(
            replay.asked().is_empty(),
            "{id} {params} asked {:?}",
            replay.asked()
        );
    }
    assert_eq!(world.remotes(), remotes);
    assert_eq!(world.branches(), branches);
}

/// A provider that answers the same to everything.
#[derive(Debug)]
struct Same {
    status: u16,
    body: Vec<u8>,
    asked: AtomicUsize,
}

impl Fetch for Same {
    fn send(&self, _: &Call, _: &AtomicBool) -> Result<Response, FetchError> {
        self.asked.fetch_add(1, Ordering::Relaxed);
        Ok(Response {
            status: self.status,
            body: self.body.clone(),
            ..Default::default()
        })
    }
}

/// Every writer of both providers against a provider that answers `body`
/// with `status` to everything: an answer or an error in bounded time and
/// requests, and no remote added unless it is at the account's site.
fn every_writer_against(world: &World, status: u16, body: &Value) {
    let key = world.dir.join("bb-p3.pub");
    std::fs::write(
        &key,
        "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n",
    )
    .unwrap();
    for account in [GITHUB, GITLAB] {
        let remote = if account == GITHUB { "origin" } else { "lab" };
        for (id, params) in [
            (
                "pr.create",
                on(
                    account,
                    json!({ "head": "topic", "base": "main", "title": "T", "body": "B",
                            "draft": true, "reviewers": ["a", "org/team"], "assignees": ["a"],
                            "labels": ["bug"], "milestone": "1" }),
                ),
            ),
            (
                "pr.create",
                on(
                    account,
                    json!({ "head": "fork:topic", "base": "main", "title": "T" }),
                ),
            ),
            (
                "pr.update",
                on(
                    account,
                    json!({ "number": 1, "title": "T", "body": "B", "reviewers": ["b"],
                            "assignees": [], "labels": [], "milestone": "", "draft": true,
                            "base": "main" }),
                ),
            ),
            (
                "pr.update",
                on(account, json!({ "number": 1, "draft": false })),
            ),
            (
                "pr.merge",
                on(
                    account,
                    json!({ "number": 1, "method": "merge", "delete_branch": true,
                            "retarget_stack": true, "title": "T", "message": "M" }),
                ),
            ),
            (
                "pr.merge",
                on(account, json!({ "number": 1, "method": "squash" })),
            ),
            (
                "pr.merge",
                on(
                    account,
                    json!({ "number": 1, "method": "rebase", "delete_branch": true }),
                ),
            ),
            (
                "pr.review",
                on(
                    account,
                    json!({ "number": 1, "event": "approve", "body": "ok" }),
                ),
            ),
            (
                "pr.review",
                on(
                    account,
                    json!({ "number": 1, "event": "request_changes", "body": "no" }),
                ),
            ),
            (
                "pr.review",
                on(
                    account,
                    json!({ "number": 1, "event": "comment", "body": "hm" }),
                ),
            ),
            (
                "pr.comment",
                on(account, json!({ "number": 1, "body": "hi" })),
            ),
            (
                "pr.comment",
                on(
                    account,
                    json!({ "number": 1, "body": "hi", "reply_to": "12345" }),
                ),
            ),
            (
                "pr.comment",
                on(
                    account,
                    json!({ "number": 1, "body": "hi", "reply_to": "abcdef0123" }),
                ),
            ),
            (
                "pr.comment",
                on(
                    account,
                    json!({ "number": 1, "body": "hi", "path": "a.txt", "line": 3,
                            "start_line": 2 }),
                ),
            ),
            (
                "pr.suggestions",
                on(account, json!({ "number": 1, "note": "n" })),
            ),
            (
                "pr.checkout",
                on(account, json!({ "number": 1, "remote": "local" })),
            ),
            (
                "pr.head",
                on(account, json!({ "number": 1, "remote": "local" })),
            ),
            (
                "provider.repo_create",
                json!({ "account": account, "name": "new", "private": true,
                        "namespace": "group/sub", "remote": "made", "protocol": "ssh" }),
            ),
            (
                "provider.fork",
                json!({ "remote": remote, "namespace": "group" }),
            ),
            (
                "provider.ssh_key_add",
                json!({ "account": account, "title": "T", "path": key.to_str().unwrap() }),
            ),
        ] {
            let same = Arc::new(Same {
                status,
                body: body.to_string().into_bytes(),
                asked: AtomicUsize::new(0),
            });
            let (_, took) = world.run(same.clone(), id, &params, Duration::from_millis(300));
            assert!(took < Duration::from_secs(20), "{id} took {took:?}");
            let asked = same.asked.load(Ordering::Relaxed);
            assert!(asked <= 2000, "{id} asked {asked} times");
        }
    }
    // What was added is at the sites only.
    for line in world.remotes().lines() {
        let address = line.split_whitespace().nth(1).unwrap_or_default();
        assert!(
            address.starts_with("git@github.com:")
                || address.starts_with("git@gitlab.example.org:")
                || address.starts_with("https://github.com/")
                || address.starts_with("https://gitlab.example.org/")
                || address == "/nonexistent/bb-p3/proj.git",
            "{line}"
        );
    }
}

#[test]
fn every_writer_survives_hostile_answers() {
    let world = World::new();
    // Sizes just past the bounds, not volume.
    let long = "\u{1b}[31m\u{0}x\u{202e}".repeat(300);
    let user = json!({ "login": long, "username": long, "name": long, "id": u64::MAX });
    // An answer that reads as a pull request, a merge request, a merge, a
    // review, a comment, a repository and a key all at once, and so drives
    // every writer past its first request; its addresses are ones git
    // would hand to a helper or take for an option.
    let everything = json!({
        "number": 1, "iid": 1, "id": 1, "node_id": "PR_x", "title": "Draft: t",
        "state": "open", "draft": true, "work_in_progress": true, "merged": true,
        "sha": "0123456789abcdef0123456789abcdef01234567", "message": long,
        "user": user, "author": user,
        "head": { "ref": "topic", "sha": "0123456789abcdef0123456789abcdef01234567",
                  "repo": { "full_name": "evil/proj" } },
        "base": { "ref": "main", "sha": "", "repo": { "full_name": "me/proj" } },
        "source_branch": "topic", "target_branch": "main",
        "source_project_id": 2, "target_project_id": 3,
        "detailed_merge_status": "checking", "rebase_in_progress": true,
        "created_at": "", "updated_at": "", "html_url": "", "web_url": "",
        "body": long, "path_with_namespace": "evil/proj", "path": "proj", "name": "proj",
        "full_name": "evil/proj", "owner": { "login": "evil" },
        "namespace": { "full_path": "evil", "kind": "user" },
        "clone_url": "ext::sh -c touch% /tmp/bb-p3-owned",
        "ssh_url": "-oProxyCommand=touch /tmp/bb-p3-owned",
        "http_url_to_repo": "ext::sh -c touch% /tmp/bb-p3-owned",
        "ssh_url_to_repo": "--upload-pack=touch /tmp/bb-p3-owned",
        "import_status": "started", "issue_url": "https://api.github.com/repos/me/proj/issues/1",
    });
    for (status, body) in [
        (200, json!(null)),
        (200, json!([])),
        (200, json!({})),
        (200, json!("text")),
        (200, json!([null, 1, "two"])),
        (
            200,
            json!({ "number": null, "iid": null, "state": [], "head": 7 }),
        ),
        (200, everything.clone()),
        (201, everything.clone()),
        (202, everything),
        (
            200,
            json!({ "data": null, "errors": [{ "message": long }] }),
        ),
        (405, json!({ "message": "Pull Request is not mergeable" })),
        (405, json!({ "message": "405 Method Not Allowed" })),
        (406, json!({ "message": "Branch cannot be merged" })),
        (409, json!({ "message": { "base": ["is", { "odd": [] }] } })),
        (409, json!({ "message": long })),
        (
            422,
            json!({ "message": "Validation Failed",
                      "errors": [{ "resource": "PullRequest", "code": "custom",
                                   "message": long }] }),
        ),
        (422, json!({ "error": [1, 2, 3] })),
        (422, json!(["not", "an", "object"])),
        (401, json!({ "message": "Bad credentials" })),
        (403, json!({ "message": long })),
        (404, json!({ "message": "Not Found" })),
        (500, json!(long)),
    ] {
        every_writer_against(&world, status, &body);
    }
    assert!(!std::path::Path::new("/tmp/bb-p3-owned").exists());
}
