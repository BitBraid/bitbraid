//! The provider commands that change things, against answers recorded
//! from a GitLab of one's own (`tests/recordings/gitlab/`, made by
//! `tests/suite/live_gitlab_ce.rs` with `BITBRAID_RECORD`, scrubbed of
//! tokens and people) and against answers made by hand. GitHub's writes,
//! recorded with the test accounts, are replayed in
//! `tests/suite/recorded_github_writes.rs`. Every request a command makes
//! has to be one that was recorded, with the body that was sent then:
//! what the commands send is held here too.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{Services, replay::Replay, secrets::MemoryStore};
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
        Self::with(Replay::load(&recording(name)).unwrap())
    }

    fn with(replay: Replay) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-p3-recorded-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = Arc::new(replay);
        let services = Services::new(replay.clone(), Arc::new(MemoryStore::new()), &dir);
        Self {
            dir,
            replay,
            services: Arc::new(services),
        }
    }

    /// A repository of the test whose `origin` is `address`, with one
    /// commit on `main`.
    fn repository(&self, address: &str) -> PathBuf {
        let dir = self.dir.join("repository");
        std::fs::create_dir_all(&dir).unwrap();
        let git = self.git(&dir);
        git.run(["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(dir.join("a.txt"), "a\n").unwrap();
        git.run(["add", "a.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "A file"]).unwrap();
        git.run(["remote", "add", "origin", address]).unwrap();
        dir
    }

    fn git(&self, dir: &Path) -> bitbraid_testkit::Git {
        let git = bitbraid_testkit::Git::new(dir, self.dir.join("home"));
        git.ensure_home().unwrap();
        git
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

    /// A recording of a GitLab of one's own: alice (`user-1`) connected
    /// first, then bob (`user-2`), as the live test did.
    fn gitlab(name: &str) -> Self {
        let h = Self::new(name);
        for token in ["glpat-replayed-1", "glpat-replayed-2"] {
            h.run(
                "account.add_token",
                json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "token": token }),
            );
        }
        h
    }

    /// A command about a project of the live test's group, as `account`.
    fn on(&self, account: &str, project: &str, id: &str, extra: Value) -> Result<Value, String> {
        let mut params = json!({ "account": account, "owner": "bb-p-group", "name": project });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.try_run(id, params).map_err(|err| err.to_string())
    }

    fn ok(&self, account: &str, project: &str, id: &str, extra: Value) -> Value {
        self.on(account, project, id, extra)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const ALICE: &str = "gitlab:127.0.0.1:8929:user-1";
const BOB: &str = "gitlab:127.0.0.1:8929:user-2";
const WRITES: &str = "bb-p3-writes";
const FAST_FORWARD: &str = "bb-p3-ff";

#[test]
fn gitlab_a_merge_request_is_made_changed_reviewed_commented_on_and_merged() {
    let h = Harness::gitlab("gitlab/pr_writes.json");
    let made = h.ok(
        ALICE,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true,
                "reviewers": ["user-2"], "assignees": ["user-1"],
                "labels": ["bug"], "milestone": "2" }),
    );
    let number = made["number"].as_u64().unwrap();
    assert_eq!(number, 27);
    assert_eq!(made["title"], "Draft: Change one");
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["reviewers"][0]["user"]["login"], "user-2");
    assert_eq!(made["reviewers"][0]["state"], "pending");
    assert_eq!(made["assignees"][0]["login"], "user-1");
    let err = h
        .on(
            ALICE,
            WRITES,
            "pr.create",
            json!({ "head": "bb-p3-alt", "base": "main", "title": "Nobody",
                    "reviewers": ["bb-p3-nobody-at-all"] }),
        )
        .unwrap_err();
    assert!(err.contains("no user \"bb-p3-nobody-at-all\""), "{err}");

    let approved = h.ok(
        BOB,
        WRITES,
        "pr.review",
        json!({ "number": 27, "event": "approve" }),
    );
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], "user-2");
    assert_eq!(approved["id"], "approval-100002");
    let changes = h.ok(
        BOB,
        WRITES,
        "pr.review",
        json!({ "number": 27, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(changes["state"], "changes_requested");
    assert_eq!(changes["body"], "Please say more.");
    let shown = h.ok(ALICE, WRITES, "pr.show", json!({ "number": 27 }));
    assert_eq!(shown["pull"]["reviewers"][0]["state"], "changes_requested");
    let commented = h.ok(
        BOB,
        WRITES,
        "pr.review",
        json!({ "number": 27, "event": "comment", "body": "Better now." }),
    );
    assert_eq!(commented["state"], "commented");
    let err = h
        .on(
            ALICE,
            WRITES,
            "pr.review",
            json!({ "number": 27, "event": "comment", "body": "Me too." }),
        )
        .unwrap_err();
    assert!(
        err.contains("only for someone asked to review !27"),
        "{err}"
    );
    assert!(err.contains("what was written was posted"), "{err}");

    let comment = h.ok(
        ALICE,
        WRITES,
        "pr.comment",
        json!({ "number": 27, "body": "A comment from the live test." }),
    );
    assert_eq!(comment["author"]["login"], "user-1");
    let shown = h.ok(ALICE, WRITES, "pr.show", json!({ "number": 27 }));
    let thread = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["comments"][0]["body"] == "A comment from the live test.")
        .unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let answer = h.ok(
        ALICE,
        WRITES,
        "pr.comment",
        json!({ "number": 27, "body": "An answer.", "reply_to": thread }),
    );
    assert_eq!(answer["body"], "An answer.");
    let shown = h.ok(ALICE, WRITES, "pr.show", json!({ "number": 27 }));
    let answered = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|found| found["id"] == thread.as_str())
        .unwrap()
        .clone();
    assert_eq!(answered["comments"][1]["body"], "An answer.");

    let changed = h.ok(
        ALICE,
        WRITES,
        "pr.update",
        json!({ "number": 27, "title": "Change one, better", "draft": false,
                "labels": ["ui"], "reviewers": [], "assignees": [], "milestone": "",
                "base": "bb-p3-alt" }),
    );
    assert_eq!(changed["pull"]["title"], "Change one, better");
    assert_eq!(changed["pull"]["draft"], false);
    assert_eq!(changed["pull"]["labels"][0]["name"], "ui");
    assert_eq!(changed["pull"]["base_ref"], "bb-p3-alt");
    assert_eq!(changed["milestone"], Value::Null);
    let drafted = h.ok(
        ALICE,
        WRITES,
        "pr.update",
        json!({ "number": 27, "draft": true, "base": "main" }),
    );
    assert_eq!(drafted["pull"]["title"], "Draft: Change one, better");
    let ready = h.ok(
        ALICE,
        WRITES,
        "pr.update",
        json!({ "number": 27, "draft": false }),
    );
    assert_eq!(ready["pull"]["draft"], false);

    let merged = h.ok(
        ALICE,
        WRITES,
        "pr.merge",
        json!({ "number": 27, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true);
    assert_eq!(merged["branch_deleted"], true);
    assert_eq!(merged["retargeted"], json!([]));
    assert_eq!(merged["sha"].as_str().unwrap().len(), 40);
    let err = h
        .on(
            ALICE,
            WRITES,
            "pr.merge",
            json!({ "number": 27, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("!27 is merged already"), "{err}");
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn gitlab_a_stack_moves_down_and_a_rebase_comes_before_a_merge() {
    let h = Harness::gitlab("gitlab/pr_merge_stack.json");
    let first = h.ok(
        ALICE,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-stack-1", "base": "main", "title": "First of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    let second = h.ok(
        ALICE,
        WRITES,
        "pr.create",
        json!({ "head": "bb-p3-stack-2", "base": "bb-p3-stack-1", "title": "Second of a stack" }),
    )["number"]
        .as_u64()
        .unwrap();
    assert_eq!((first, second), (25, 26));
    let stack = h.ok(ALICE, WRITES, "pr.stack", json!({ "number": second }));
    assert_eq!(stack["position"], 1);
    let merged = h.ok(
        ALICE,
        WRITES,
        "pr.merge",
        json!({ "number": first, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "message": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true);
    assert_eq!(merged["retargeted"], json!([second]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = h.ok(
        ALICE,
        WRITES,
        "pr.for_branch",
        json!({ "branch": "bb-p3-stack-2" }),
    );
    assert_eq!(moved["base_ref"], "main");
    let behind = h.ok(
        ALICE,
        FAST_FORWARD,
        "pr.create",
        json!({ "head": "bb-p3-behind", "base": "main", "title": "Behind main" }),
    )["number"]
        .as_u64()
        .unwrap();
    let err = h
        .on(
            ALICE,
            FAST_FORWARD,
            "pr.merge",
            json!({ "number": behind, "method": "merge" }),
        )
        .unwrap_err();
    assert!(err.contains("it allows rebase"), "{err}");
    let rebased = h.ok(
        ALICE,
        FAST_FORWARD,
        "pr.merge",
        json!({ "number": behind, "method": "rebase", "delete_branch": true }),
    );
    assert_eq!(rebased["merged"], true);
    assert_eq!(rebased["branch_deleted"], true);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

/// The account of GitHub's examples.
const OCTOCAT: &str = "github:github.com:octocat";

/// The key of the tests, made by `ssh-keygen -t ed25519` (its private
/// half was thrown away), and its fingerprint as `ssh-keygen -l` says it.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
const FINGERPRINT: &str = "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0";

/// The user of one run of the live test, as its recording names her.
const CAROL: &str = "gitlab:127.0.0.1:8929:user-2";

#[test]
fn gitlab_a_project_is_made_and_forked_and_a_key_uploaded() {
    let h = Harness::new("gitlab/repos_forks_keys.json");
    h.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "token": "glpat-replayed" }),
    );
    let repo = h.repository("http://127.0.0.1:8929/bb-p-group/bb-p3-writes.git");
    let created = h
        .run_in(
            &repo,
            "provider.repo_create",
            json!({ "account": CAROL, "name": "bb-p3-made", "private": true,
                    "description": "Made by the live test", "remote": "made" }),
        )
        .unwrap();
    assert_eq!(created["repo"]["full_name"], "user-2/bb-p3-made");
    assert_eq!(created["remote"], "made");
    assert_eq!(created["pushed"], Value::Null);
    let forked = h
        .run_in(&repo, "provider.fork", json!({ "remote": "origin" }))
        .unwrap();
    assert_eq!(forked["repo"]["full_name"], "user-2/bb-p3-writes");
    assert_eq!(forked["repo"]["parent"], "bb-p-group/bb-p3-writes");
    assert_eq!(forked["remote"], "user-2");
    let remotes = h.git(&repo).run(["remote", "-v"]).unwrap();
    assert!(
        remotes.contains("user-2\thttp://127.0.0.1:8929/user-2/bb-p3-writes.git (fetch)"),
        "{remotes}"
    );
    assert!(
        remotes.contains("made\thttp://127.0.0.1:8929/user-2/bb-p3-made.git (fetch)"),
        "{remotes}"
    );
    let key = h.dir.join("bb-p3.pub");
    std::fs::write(&key, KEY).unwrap();
    let added = h.run(
        "provider.ssh_key_add",
        json!({ "account": CAROL, "title": "bb-p3 test key", "path": key.to_str().unwrap() }),
    );
    assert_eq!(added["title"], "bb-p3 test key");
    assert_eq!(added["fingerprint"], FINGERPRINT);
    assert!(h.replay.unused().is_empty(), "{:?}", h.replay.unused());
}

#[test]
fn a_new_repository_is_added_as_a_remote_and_the_branch_pushed_to_it() {
    // The provider's answer, from GitHub's example of a new repository,
    // with the address of a bare repository on this machine.
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    let bare = std::env::temp_dir().join(format!("bb-p3-bare-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&bare);
    std::fs::create_dir_all(&bare).unwrap();
    let address = bare.to_str().unwrap().to_owned();
    let replay = Replay::from_json(
        &json!({ "note": "by hand", "exchanges": [
            { "method": "GET", "path_and_query": "/user", "request_body": null, "status": 200,
              "headers": {}, "body": { "login": "octocat", "id": 1 } },
            { "method": "POST", "path_and_query": "/user/repos",
              "request_body": { "name": "Hello-World", "private": false },
              "status": 201, "headers": {},
              "body": { "id": 1296269, "name": "Hello-World", "full_name": "octocat/Hello-World",
                        "owner": { "login": "octocat" }, "private": false,
                        "clone_url": "https://github.com/octocat/Hello-World.git",
                        "ssh_url": "git@github.com:octocat/Hello-World.git",
                        "html_url": "https://github.com/octocat/Hello-World" } }
        ] })
        .to_string(),
    )
    .unwrap();
    let h = Harness::with(replay);
    h.git(&bare).run(["init", "-q", "--bare"]).unwrap();
    h.run(
        "account.add_token",
        json!({ "kind": "github", "token": "ghx_by_hand" }),
    );
    let repo = h.repository("https://github.com/octocat/other.git");
    // git turns the new repository's address into the bare one here.
    h.git(&repo)
        .run([
            "config",
            &format!("url.{address}.insteadOf"),
            "https://github.com/octocat/Hello-World.git",
        ])
        .unwrap();
    let made = h
        .run_in(
            &repo,
            "provider.repo_create",
            json!({ "account": OCTOCAT, "name": "Hello-World", "private": false,
                    "remote": "hello", "push": true }),
        )
        .unwrap();
    assert_eq!(made["remote"], "hello");
    assert_eq!(made["pushed"], "main");
    let git = h.git(&repo);
    assert_eq!(
        git.rev_parse("refs/remotes/hello/main").unwrap(),
        git.rev_parse("main").unwrap()
    );
    assert_eq!(
        git.run(["rev-parse", "--abbrev-ref", "main@{upstream}"])
            .unwrap()
            .trim(),
        "hello/main"
    );
    assert_eq!(
        h.git(&bare).rev_parse("main").unwrap(),
        git.rev_parse("main").unwrap()
    );
    let _ = std::fs::remove_dir_all(&bare);
}
