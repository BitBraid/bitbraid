//! The provider commands that change things at GitHub, against answers
//! recorded from github.com with the test accounts
//! (`tests/recordings/github-org/`, made by `tests/suite/live_github.rs`
//! with `BITBRAID_RECORD`, scrubbed): the machine account `user-1` makes,
//! changes, comments on and merges pull requests of `org-1/bitbraid-live-writes`
//! in every way the repository allows, and the reviewer `user-2` approves,
//! asks for changes and answers on a line; GitHub refuses a merge with
//! conflicts (405) and one whose head moved (409); a stack moves down;
//! repositories are made in the organisation and in the account's own
//! name, the reads' repository is forked, a key uploaded; a pull request
//! from the fork is checked out, its repositories here bare ones on this
//! machine that git reaches in place of github.com. Every request a
//! command makes has to be one that was recorded, with the body that was
//! sent then.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, CommandError};
use bitbraid_integrations::{Services, replay::Replay, secrets::MemoryStore};
use bitbraid_testkit::Git;
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
        let dir = std::env::temp_dir().join(format!("bb-g-recorded-w-{}-{n}", std::process::id()));
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

    /// A command about the writes' repository, as `account`.
    fn writes(&self, account: &str, id: &str, extra: Value) -> Result<Value, String> {
        let mut params =
            json!({ "account": account, "owner": "org-1", "name": "bitbraid-live-writes" });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        self.try_run(id, params).map_err(|err| err.to_string())
    }

    fn ok(&self, account: &str, id: &str, extra: Value) -> Value {
        self.writes(account, id, extra)
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

fn verdicts(pull: &Value) -> Vec<(String, String)> {
    pull["reviewers"]
        .as_array()
        .unwrap()
        .iter()
        .map(|reviewer| {
            (
                reviewer["user"]["login"].as_str().unwrap().to_owned(),
                reviewer["state"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn github_org_pull_requests_are_made_changed_reviewed_commented_on_and_merged() {
    let h = Harness::new("pr_writes.json", 2);
    let made = h.ok(
        MAIN,
        "pr.create",
        json!({ "head": "bb-w-one", "base": "main", "title": "Change one",
                "body": "Made by the live test.", "draft": true, "reviewers": ["user-2"],
                "assignees": ["user-1"], "labels": ["bug"], "milestone": "1" }),
    );
    assert_eq!(made["number"], 25);
    assert_eq!(made["draft"], true);
    assert_eq!(made["labels"][0]["name"], "bug");
    assert_eq!(made["assignees"][0]["login"], "user-1");
    assert_eq!(
        verdicts(&made),
        [("user-2".to_owned(), "pending".to_owned())]
    );
    // The author cannot be asked: the pull request is there all the same.
    let err = h
        .writes(
            MAIN,
            "pr.create",
            json!({ "head": "bb-w-alt", "base": "main", "title": "Asked of myself",
                    "reviewers": ["user-1"] }),
        )
        .unwrap_err();
    assert!(
        err.contains("pull request #26 was opened, but its reviewers could not be asked"),
        "{err}"
    );
    assert!(err.contains("Review cannot be requested from pull request author"));
    assert_eq!(
        h.ok(MAIN, "pr.for_branch", json!({ "branch": "bb-w-alt" }))["number"],
        26
    );
    // The author's own reviews: a comment is taken, a verdict is not.
    let commented = h.ok(
        MAIN,
        "pr.review",
        json!({ "number": 25, "event": "comment", "body": "A look of my own." }),
    );
    assert_eq!(commented["state"], "commented");
    assert_eq!(commented["author"]["login"], "user-1");
    let err = h
        .writes(
            MAIN,
            "pr.review",
            json!({ "number": 25, "event": "approve" }),
        )
        .unwrap_err();
    assert!(err.contains("422"), "{err}");
    let err = h
        .writes(
            MAIN,
            "pr.review",
            json!({ "number": 25, "event": "request_changes" }),
        )
        .unwrap_err();
    assert!(err.contains("something written"), "{err}");
    // Comments, an answer quoting one, and the reviewer's answer on a line.
    let comment = h.ok(
        MAIN,
        "pr.comment",
        json!({ "number": 25, "body": "A comment from the live test." }),
    );
    let quoted = h.ok(
        MAIN,
        "pr.comment",
        json!({ "number": 25, "body": "An answer to it.", "reply_to": comment["id"] }),
    );
    assert_eq!(
        quoted["body"],
        "> A comment from the live test.\n\nAn answer to it."
    );
    let shown = h.ok(MAIN, "pr.show", json!({ "number": 25 }));
    let thread = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .find(|thread| thread["path"].is_string())
        .unwrap();
    assert!(
        thread["path"]
            .as_str()
            .unwrap()
            .starts_with("bb-w/bb-w-one-")
    );
    assert_eq!(thread["comments"][0]["body"], "On this line.");
    let answer = h.ok(
        REVIEWER,
        "pr.comment",
        json!({ "number": 25, "body": "An answer on the line.", "reply_to": thread["id"] }),
    );
    assert_eq!(answer["body"], "An answer on the line.");
    assert_eq!(answer["author"]["login"], "user-2");
    // Changed, made a draft again on its first base, then ready.
    let changed = h.ok(
        MAIN,
        "pr.update",
        json!({ "number": 25, "title": "Change one, better", "draft": false,
                "labels": ["ui"], "assignees": [], "milestone": "", "base": "bb-w-alt" }),
    );
    assert_eq!(changed["pull"]["title"], "Change one, better");
    assert_eq!(changed["pull"]["draft"], false);
    assert_eq!(changed["pull"]["labels"][0]["name"], "ui");
    assert_eq!(changed["pull"]["assignees"], json!([]));
    assert_eq!(changed["pull"]["base_ref"], "bb-w-alt");
    assert_eq!(changed["milestone"], Value::Null);
    let drafted = h.ok(
        MAIN,
        "pr.update",
        json!({ "number": 25, "draft": true, "base": "main" }),
    );
    assert_eq!(drafted["pull"]["draft"], true);
    assert_eq!(drafted["pull"]["base_ref"], "main");
    h.ok(MAIN, "pr.update", json!({ "number": 25, "draft": false }));
    // The reviewer's verdicts.
    let approved = h.ok(
        REVIEWER,
        "pr.review",
        json!({ "number": 25, "event": "approve", "body": "Fine by me." }),
    );
    assert_eq!(approved["state"], "approved");
    assert_eq!(approved["author"]["login"], "user-2");
    let asked = h.ok(
        REVIEWER,
        "pr.review",
        json!({ "number": 26, "event": "request_changes", "body": "Please say more." }),
    );
    assert_eq!(asked["state"], "changes_requested");
    let shown = h.ok(MAIN, "pr.show", json!({ "number": 25 }));
    assert!(
        verdicts(&shown["pull"]).contains(&("user-2".to_owned(), "approved".to_owned())),
        "{:?}",
        verdicts(&shown["pull"])
    );
    // Squashed with a title, its branch removed; merged again: refused
    // before anything is sent.
    let merged = h.ok(
        MAIN,
        "pr.merge",
        json!({ "number": 25, "method": "squash", "delete_branch": true,
                "title": "Change one, squashed" }),
    );
    assert_eq!(merged["merged"], true);
    assert_eq!(merged["branch_deleted"], true);
    assert_eq!(merged["message"], "Pull Request successfully merged");
    assert_eq!(merged["sha"], "ec1393ec82eb483db11857a627c3343b286d0923");
    let err = h
        .writes(MAIN, "pr.merge", json!({ "number": 25, "method": "merge" }))
        .unwrap_err();
    assert!(err.contains("#25 is merged already"), "{err}");
    // With a merge commit.
    assert_eq!(
        h.ok(
            MAIN,
            "pr.create",
            json!({ "head": "bb-w-merge", "base": "main", "title": "Merged with a commit" })
        )["number"],
        27
    );
    let merged = h.ok(
        MAIN,
        "pr.merge",
        json!({ "number": 27, "method": "merge", "title": "Merge it" }),
    );
    assert_eq!(merged["merged"], true);
    assert_eq!(merged["sha"], "0b1a37d70bd0b30c1fedcb0e7109d87d96531b30");
    assert_eq!(merged["branch_deleted"], false);
    // Two that change the same line: once the first is in, GitHub refuses
    // the second.
    for (head, title) in [
        ("bb-w-conflict-a", "One way"),
        ("bb-w-conflict-b", "Another way"),
    ] {
        h.ok(
            MAIN,
            "pr.create",
            json!({ "head": head, "base": "main", "title": title }),
        );
    }
    assert_eq!(
        h.ok(MAIN, "pr.merge", json!({ "number": 28, "method": "merge" }))["merged"],
        true
    );
    let err = h
        .writes(MAIN, "pr.merge", json!({ "number": 29, "method": "merge" }))
        .unwrap_err();
    assert!(
        err.contains("405: Pull Request has merge conflicts"),
        "{err}"
    );
    // A head that moved after it was read: refused, nothing merged.
    assert_eq!(
        h.ok(
            MAIN,
            "pr.create",
            json!({ "head": "bb-w-moved", "base": "main", "title": "Moved under it" })
        )["number"],
        30
    );
    let err = h
        .writes(
            MAIN,
            "pr.merge",
            json!({ "number": 30, "method": "squash" }),
        )
        .unwrap_err();
    assert!(err.contains("409: Head branch was modified"), "{err}");
    h.all_used();
}

#[test]
fn github_org_a_stack_moves_down_when_its_bottom_is_merged() {
    let h = Harness::new("pr_merge_stack.json", 1);
    let first = h.ok(
        MAIN,
        "pr.create",
        json!({ "head": "bb-w-stack-1", "base": "main", "title": "First of a stack" }),
    );
    let second = h.ok(
        MAIN,
        "pr.create",
        json!({ "head": "bb-w-stack-2", "base": "bb-w-stack-1", "title": "Second of a stack" }),
    );
    assert_eq!(
        (first["number"].clone(), second["number"].clone()),
        (json!(22), json!(23))
    );
    let stack = h.ok(MAIN, "pr.stack", json!({ "number": 23 }));
    assert_eq!(stack["position"], 1);
    let merged = h.ok(
        MAIN,
        "pr.merge",
        json!({ "number": 22, "method": "merge", "delete_branch": true,
                "retarget_stack": true, "title": "Merge the first of a stack" }),
    );
    assert_eq!(merged["merged"], true);
    assert_eq!(merged["retargeted"], json!([23]));
    assert_eq!(merged["branch_deleted"], true);
    let moved = h.ok(MAIN, "pr.for_branch", json!({ "branch": "bb-w-stack-2" }));
    assert_eq!(moved["base_ref"], "main");
    assert_eq!(moved["state"], "open");
    h.ok(
        MAIN,
        "pr.create",
        json!({ "head": "bb-w-rebase", "base": "main", "title": "Replayed onto main" }),
    );
    let rebased = h.ok(
        MAIN,
        "pr.merge",
        json!({ "number": 24, "method": "rebase" }),
    );
    assert_eq!(rebased["merged"], true);
    assert_eq!(rebased["branch_deleted"], false);
    h.all_used();
}

/// The key of the tests, made by `ssh-keygen -t ed25519` (its private
/// half was thrown away), and its fingerprint as `ssh-keygen -l` says it.
const KEY: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIAWUi35BfUEiiOjbuDIeFeVHDul6RR5Nf+5RPMLvtWfd bb-p3@example.invalid\n";
const FINGERPRINT: &str = "SHA256:IeI4dtwxvncxctQMgpbLrR9zUYXixuEIOupl+ATPL/0";

#[test]
fn github_org_repositories_are_made_and_forked_and_a_key_uploaded() {
    let h = Harness::new("repos_forks_keys.json", 1);
    let dir = h.dir.join("repository");
    std::fs::create_dir_all(&dir).unwrap();
    let git = h.git(&dir);
    git.run(["init", "-q", "-b", "main"]).unwrap();
    git.run([
        "remote",
        "add",
        "origin",
        "https://github.com/org-1/bitbraid-live.git",
    ])
    .unwrap();
    let made = h
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": MAIN, "namespace": "org-1", "name": "bitbraid-live-made-org",
                    "private": true, "description": "Made by BitBraid's live test",
                    "remote": "made" }),
        )
        .unwrap();
    assert_eq!(made["repo"]["full_name"], "org-1/bitbraid-live-made-org");
    assert_eq!(made["repo"]["private"], true);
    assert_eq!(made["remote"], "made");
    let mine = h
        .run_in(
            &dir,
            "provider.repo_create",
            json!({ "account": MAIN, "name": "bitbraid-live-made-user", "private": false,
                    "remote": "mine" }),
        )
        .unwrap();
    assert_eq!(mine["repo"]["full_name"], "user-1/bitbraid-live-made-user");
    assert_eq!(mine["repo"]["private"], false);
    // The fork was there: GitHub answers it again, and it is waited for
    // until it has branches.
    let forked = h
        .run_in(&dir, "provider.fork", json!({ "remote": "origin" }))
        .unwrap();
    assert_eq!(forked["repo"]["full_name"], "user-1/bitbraid-live");
    assert_eq!(forked["repo"]["parent"], "org-1/bitbraid-live");
    assert_eq!(forked["remote"], "user-1");
    let remotes = git.run(["remote", "-v"]).unwrap();
    for line in [
        "made\thttps://github.com/org-1/bitbraid-live-made-org.git (fetch)",
        "mine\thttps://github.com/user-1/bitbraid-live-made-user.git (fetch)",
        "user-1\thttps://github.com/user-1/bitbraid-live.git (fetch)",
    ] {
        assert!(remotes.contains(line), "{remotes}");
    }
    let key = h.dir.join("bitbraid-live.pub");
    std::fs::write(&key, KEY).unwrap();
    let added = h.run(
        "provider.ssh_key_add",
        json!({ "account": MAIN, "title": "bitbraid-live test key", "path": key.to_str().unwrap() }),
    );
    assert_eq!(added["id"], "300097");
    assert_eq!(added["title"], "bitbraid-live test key");
    assert_eq!(added["fingerprint"], FINGERPRINT);
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
fn github_org_a_pull_request_from_a_fork_is_checked_out_as_git_would() {
    let h = Harness::new("pr_checkout.json", 1);
    // The repository and its fork as bare repositories here, at the
    // addresses GitHub gave: `refs/pull/8/head` is the fork's branch.
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
        "bb-fork-topic:refs/pull/8/head",
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
            "https://github.com/org-1/bitbraid-live.git",
        ])
        .unwrap();
        for (path, address) in [
            (&origin_path, "https://github.com/org-1/bitbraid-live.git"),
            (&fork_path, "https://github.com/user-1/bitbraid-live.git"),
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
            json!({ "account": MAIN, "owner": "org-1", "name": "bitbraid-live", "number": 8,
                    "remote": "origin" }),
        )
        .unwrap()
    };
    let out = checkout();
    assert_eq!(out["branch"], "bb-fork-topic");
    assert_eq!(out["commit"], head.as_str());
    assert_eq!(out["created"], true);
    assert_eq!(out["remote_added"], "user-1");
    assert_eq!(out["upstream"], "user-1/bb-fork-topic");
    for args in [
        &[
            "remote",
            "add",
            "user-1",
            "https://github.com/user-1/bitbraid-live.git",
        ][..],
        &["fetch", "-q", "user-1"],
        &["fetch", "-q", "origin", "refs/pull/8/head"],
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
