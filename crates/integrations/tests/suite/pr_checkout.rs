//! Checking a pull request out, held to git doing the same on a twin
//! clone. The remote is a bare repository on this machine with
//! `refs/pull/N/head` written into it, as GitHub keeps them, and a fork
//! of it is another; what the provider says of the pull requests is made
//! by hand from GitHub's published examples, its addresses those of the
//! two repositories. Nothing leaves the machine.

use std::{
    path::PathBuf,
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

/// The addresses of the remote and of its fork, as GitHub gives them.
const ORIGIN: &str = "https://github.com/octocat/hello.git";
const FORK: &str = "https://github.com/hubot/hello.git";

/// The repositories of a test: the remote, a fork of it, our clone and
/// git's twin of it.
struct World {
    root: PathBuf,
    home: PathBuf,
}

impl World {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!("bb-p3-checkout-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let world = Self {
            home: root.join("home"),
            root,
        };
        world.git(".").ensure_home().unwrap();
        world
    }

    fn path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    fn git(&self, name: &str) -> Git {
        Git::new(self.root.join(name), &self.home)
    }

    /// A commit of a new file on the current branch of `name`.
    fn commit(&self, name: &str, file: &str) -> String {
        std::fs::write(self.path(name).join(file), format!("{file}\n")).unwrap();
        let git = self.git(name);
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "-m", &format!("Add {file}")])
            .unwrap();
        git.rev_parse("HEAD").unwrap()
    }

    /// The remote with `main` and `feature`, its pull requests' heads
    /// written: 7 from `feature`, 8 from the fork's `topic`, 9 from a
    /// fork's `main` that is gone. Then the fork, our clone and the twin.
    fn build() -> (Self, [String; 3]) {
        let world = Self::new();
        let work = world.git("work");
        std::fs::create_dir_all(world.path("work")).unwrap();
        work.run(["init", "-q", "-b", "main"]).unwrap();
        world.commit("work", "a.txt");
        world
            .git(".")
            .run(["clone", "-q", "--bare", "work", "origin.git"])
            .unwrap();
        work.run([
            "remote",
            "add",
            "origin",
            world.path("origin.git").to_str().unwrap(),
        ])
        .unwrap();
        work.run(["switch", "-q", "-c", "feature"]).unwrap();
        let feature = world.commit("work", "feature.txt");
        work.run(["push", "-q", "origin", "feature"]).unwrap();
        work.run(["push", "-q", "origin", "feature:refs/pull/7/head"])
            .unwrap();
        // The fork, with a branch of its own; its head is the remote's
        // too, as GitHub keeps it.
        world
            .git(".")
            .run(["clone", "-q", "--bare", "origin.git", "fork.git"])
            .unwrap();
        work.run(["switch", "-q", "-c", "topic", "main"]).unwrap();
        let topic = world.commit("work", "topic.txt");
        work.run([
            "push",
            "-q",
            world.path("fork.git").to_str().unwrap(),
            "topic",
        ])
        .unwrap();
        work.run(["push", "-q", "origin", "topic:refs/pull/8/head"])
            .unwrap();
        // A fork's `main` that is gone: only its head is left.
        work.run(["switch", "-q", "-c", "gone", "main"]).unwrap();
        let gone = world.commit("work", "gone.txt");
        work.run(["push", "-q", "origin", "gone:refs/pull/9/head"])
            .unwrap();
        // The clones know the remote and the fork by their addresses at
        // GitHub, which git turns into the repositories here.
        for clone in ["ours", "twin"] {
            world
                .git(".")
                .run(["clone", "-q", "origin.git", clone])
                .unwrap();
            let git = world.git(clone);
            git.run(["remote", "set-url", "origin", ORIGIN]).unwrap();
            for (here, there) in [("origin.git", ORIGIN), ("fork.git", FORK)] {
                git.run([
                    "config",
                    &format!("url.{}.insteadOf", world.path(here).to_str().unwrap()),
                    there,
                ])
                .unwrap();
            }
        }
        (world, [feature, topic, gone])
    }

    /// What a repository has that checking out could change: its branches
    /// and remote branches, where they go, what HEAD is, which branch
    /// follows what, its remotes, and its index.
    fn state(&self, name: &str) -> Vec<String> {
        let git = self.git(name);
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
            // Our clone's remote addresses are the same paths as the twin's.
            out.push(line.replace(self.root.to_str().unwrap(), "<root>"));
        }
        out.sort();
        out
    }
}

impl Drop for World {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

/// A pull request as GitHub answers it (its example, made ours).
fn pull(number: u64, head: &str, sha: &str, head_repo: Value) -> Value {
    let owner = json!({ "login": "octocat", "id": 1 });
    json!({
        "number": number, "title": format!("Pull request {number}"), "state": "open",
        "draft": false, "user": owner,
        "head": { "ref": head, "sha": sha, "repo": head_repo },
        "base": { "ref": "main", "sha": "0000000000000000000000000000000000000000",
                  "repo": { "full_name": "octocat/hello" } },
        "created_at": "2026-10-13T08:00:00Z", "updated_at": "2026-10-13T08:00:00Z",
        "html_url": format!("https://github.com/octocat/hello/pull/{number}"),
    })
}

/// The answers of the provider for the pull requests of `world`.
fn answers(world: &World, shas: &[String; 3]) -> Vec<Value> {
    let get = |path: &str, body: Value| {
        json!({ "method": "GET", "path_and_query": path, "request_body": null,
                "status": 200, "headers": { "content-type": "application/json" }, "body": body })
    };
    let _ = world;
    vec![
        get("/user", json!({ "login": "octocat", "id": 1 })),
        get(
            "/repos/octocat/hello/pulls/7",
            pull(
                7,
                "feature",
                &shas[0],
                json!({ "full_name": "octocat/hello" }),
            ),
        ),
        get(
            "/repos/octocat/hello/pulls/8",
            pull(8, "topic", &shas[1], json!({ "full_name": "hubot/hello" })),
        ),
        get(
            "/repos/hubot/hello",
            json!({ "id": 2, "name": "hello", "full_name": "hubot/hello",
                    "owner": { "login": "hubot" }, "private": false,
                    "clone_url": FORK, "ssh_url": "git@github.com:hubot/hello.git",
                    "html_url": "https://github.com/hubot/hello" }),
        ),
        get(
            "/repos/octocat/hello/pulls/9",
            pull(9, "main", &shas[2], Value::Null),
        ),
    ]
}

struct Harness {
    world: World,
    shas: [String; 3],
    dir: PathBuf,
    services: Arc<Services>,
}

impl Harness {
    fn new() -> Self {
        let (world, shas) = World::build();
        let replay = Arc::new(
            Replay::from_json(
                &json!({ "note": "by hand", "exchanges": answers(&world, &shas) }).to_string(),
            )
            .unwrap(),
        );
        let dir = world.path("config");
        let services = Arc::new(Services::new(replay, Arc::new(MemoryStore::new()), &dir));
        let h = Self {
            world,
            shas,
            dir,
            services,
        };
        h.run(
            "account.add_token",
            json!({ "kind": "github", "token": "ghx_by_hand" }),
        )
        .unwrap();
        h
    }

    fn run(&self, id: &str, params: Value) -> Result<Value, CommandError> {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        if id.starts_with("pr.") {
            registry
                .run(
                    "repo.open",
                    &mut ctx,
                    &json!({ "path": self.world.path("ours").to_str().unwrap() }),
                )
                .unwrap();
        }
        registry.run(id, &mut ctx, &params)
    }

    fn checkout(&self, number: u64) -> Value {
        self.run(
            "pr.checkout",
            json!({ "account": "github:github.com:octocat", "owner": "octocat",
                    "name": "hello", "number": number, "remote": "origin" }),
        )
        .unwrap_or_else(|err| panic!("pr.checkout {number}: {err}"))
    }

    fn twin(&self, args: &[&str]) {
        self.world.git("twin").run(args).unwrap();
    }

    fn same(&self) {
        assert_eq!(self.world.state("ours"), self.world.state("twin"));
    }
}

#[test]
fn a_pull_request_of_the_same_repository_is_checked_out_as_git_would() {
    let h = Harness::new();
    let out = h.checkout(7);
    assert_eq!(out["branch"], "feature");
    assert_eq!(out["commit"], h.shas[0]);
    assert_eq!(out["created"], true);
    assert_eq!(out["upstream"], "origin/feature");
    assert_eq!(out["remote_added"], Value::Null);
    h.twin(&["fetch", "-q", "origin"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/7/head"]);
    h.twin(&["branch", "--no-track", "feature", "FETCH_HEAD"]);
    h.twin(&["switch", "-q", "feature"]);
    h.twin(&[
        "branch",
        "-q",
        "--set-upstream-to=origin/feature",
        "feature",
    ]);
    h.same();
    // The journal has each step: undone, the branch is gone and main is
    // checked out again.
    let journal = h.run_on_repo("undo.journal").to_string();
    assert!(journal.contains("Create branch feature"), "{journal}");
    h.run_on_repo("undo.last");
    h.run_on_repo("undo.last");
    let ours = h.world.git("ours");
    assert_eq!(
        ours.run(["symbolic-ref", "--short", "HEAD"])
            .unwrap()
            .trim(),
        "main"
    );
    assert!(
        ours.run(["rev-parse", "--verify", "-q", "refs/heads/feature"])
            .is_err()
    );
}

#[test]
fn a_pull_request_from_a_fork_adds_the_fork_as_a_remote() {
    let h = Harness::new();
    let out = h.checkout(8);
    assert_eq!(out["branch"], "topic");
    assert_eq!(out["commit"], h.shas[1]);
    assert_eq!(out["upstream"], "hubot/topic");
    assert_eq!(out["remote_added"], "hubot");
    h.twin(&["remote", "add", "hubot", FORK]);
    h.twin(&["fetch", "-q", "hubot"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/8/head"]);
    h.twin(&["branch", "--no-track", "topic", "FETCH_HEAD"]);
    h.twin(&["switch", "-q", "topic"]);
    h.twin(&["branch", "-q", "--set-upstream-to=hubot/topic", "topic"]);
    h.same();
    // Again: the remote is not added twice, and the branch is the same.
    let again = h.checkout(8);
    assert_eq!(again["created"], false);
    assert_eq!(again["remote_added"], Value::Null);
    h.same();
}

#[test]
fn a_pull_request_whose_fork_is_gone_comes_by_its_head_under_a_free_name() {
    let h = Harness::new();
    let out = h.checkout(9);
    // Its branch was a fork's `main`, which is ours here already.
    assert_eq!(out["branch"], "pull-9");
    assert_eq!(out["upstream"], Value::Null);
    h.twin(&["fetch", "-q", "origin"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/9/head"]);
    h.twin(&["branch", "--no-track", "pull-9", "FETCH_HEAD"]);
    h.twin(&["switch", "-q", "pull-9"]);
    h.same();
}

#[test]
fn a_branch_that_is_there_is_moved_forward_to_the_head() {
    let h = Harness::new();
    h.checkout(7);
    h.twin(&["fetch", "-q", "origin"]);
    h.twin(&["switch", "-q", "--track", "origin/feature"]);
    // The pull request gains a commit.
    let work = h.world.git("work");
    work.run(["switch", "-q", "feature"]).unwrap();
    let newer = h.world.commit("work", "more.txt");
    work.run(["push", "-q", "origin", "feature"]).unwrap();
    work.run(["push", "-q", "-f", "origin", "feature:refs/pull/7/head"])
        .unwrap();
    // Checked out: pulled forward.
    let out = h.checkout(7);
    assert_eq!(out["created"], false);
    assert_eq!(out["updated"], true, "{out}");
    assert_eq!(out["commit"], newer);
    h.twin(&["pull", "-q", "--ff-only"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/7/head"]);
    h.same();
    // Elsewhere, and one more commit: moved forward without checking out
    // in between, then checked out.
    h.run_on_ours(&["switch", "-q", "main"]);
    h.twin(&["switch", "-q", "main"]);
    let newest = h.world.commit("work", "most.txt");
    work.run(["push", "-q", "origin", "feature"]).unwrap();
    work.run(["push", "-q", "-f", "origin", "feature:refs/pull/7/head"])
        .unwrap();
    let out = h.checkout(7);
    assert_eq!(out["updated"], true, "{out}");
    assert_eq!(out["commit"], newest);
    h.twin(&["fetch", "-q", "origin"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/7/head"]);
    h.twin(&["branch", "-f", "feature", "FETCH_HEAD"]);
    h.twin(&["switch", "-q", "feature"]);
    h.same();
}

#[test]
fn a_branch_that_went_its_own_way_is_left_where_it_is() {
    let h = Harness::new();
    h.checkout(7);
    h.run_on_ours(&["switch", "-q", "main"]);
    // Our feature gains a commit of its own; the pull request another.
    h.run_on_ours(&["switch", "-q", "feature"]);
    std::fs::write(h.world.path("ours").join("mine.txt"), "mine\n").unwrap();
    h.run_on_ours(&["add", "mine.txt"]);
    h.run_on_ours(&["commit", "-q", "-m", "Mine"]);
    h.run_on_ours(&["switch", "-q", "main"]);
    let work = h.world.git("work");
    work.run(["switch", "-q", "feature"]).unwrap();
    h.world.commit("work", "theirs.txt");
    work.run(["push", "-q", "origin", "feature"]).unwrap();
    work.run(["push", "-q", "-f", "origin", "feature:refs/pull/7/head"])
        .unwrap();
    let out = h.checkout(7);
    assert_eq!(out["updated"], false);
    assert!(
        out["note"].as_str().unwrap().contains("left where it was"),
        "{out}"
    );
    let head = h
        .world
        .git("ours")
        .run(["symbolic-ref", "--short", "HEAD"])
        .unwrap();
    assert_eq!(head.trim(), "feature");
}

impl Harness {
    fn run_on_ours(&self, args: &[&str]) {
        self.world.git("ours").run(args).unwrap();
    }

    /// An engine command on our clone, without parameters.
    fn run_on_repo(&self, id: &str) -> Value {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext::default();
        registry
            .run(
                "repo.open",
                &mut ctx,
                &json!({ "path": self.world.path("ours").to_str().unwrap() }),
            )
            .unwrap();
        registry
            .run(id, &mut ctx, &json!({}))
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
}

#[test]
fn hostile_parameters_and_a_missing_remote_are_refused() {
    let h = Harness::new();
    for params in [
        json!({ "account": "github:github.com:octocat", "owner": "octocat", "name": "hello",
                "number": 7, "remote": "nowhere" }),
        json!({ "account": "github:github.com:octocat", "owner": "someone", "name": "else",
                "number": 7 }),
        json!({ "account": "github:github.com:octocat", "owner": "octocat", "name": "hello",
                "number": 7, "remote": "--upload-pack=x" }),
        json!({ "account": "github:github.com:octocat", "owner": "octocat", "name": "hello",
                "number": 7, "remote": "origin", "dirty": "maybe" }),
        json!({ "account": "github:github.com:octocat", "owner": "octocat", "name": "hello",
                "number": 0, "remote": "origin" }),
    ] {
        assert!(h.run("pr.checkout", params.clone()).is_err(), "{params}");
        // The same for the head alone, which takes no `dirty`.
        if params.get("dirty").is_none() {
            assert!(h.run("pr.head", params.clone()).is_err(), "{params}");
        }
    }
    // Nothing moved.
    h.same();
}

/// A pull request's head fetched for a worktree of its own (`pr.head`):
/// of the same repository it starts at the remote's branch; from a fork
/// that is no remote here at the head's commit, nothing added; once a
/// checkout added the fork, at the fork's branch. Each fetch as git's on
/// the twin, and a worktree made from what it says is at the head. A
/// worktree started at `origin/topic`, which only the fork has, would fail.
#[test]
fn a_pull_requests_head_is_fetched_for_a_worktree_where_it_comes_from() {
    let h = Harness::new();
    let head = |number: u64| {
        h.run(
            "pr.head",
            json!({ "account": "github:github.com:octocat", "owner": "octocat",
                    "name": "hello", "number": number, "remote": "origin" }),
        )
        .unwrap_or_else(|err| panic!("pr.head {number}: {err}"))
    };
    let same = head(7);
    assert_eq!(same["commit"], h.shas[0]);
    assert_eq!(same["branch"], "feature");
    assert_eq!(same["remote"], "origin");
    assert_eq!(same["start"], "origin/feature");
    h.twin(&["fetch", "-q", "origin"]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/7/head"]);
    h.same();

    let fork = head(8);
    assert_eq!(fork["commit"], h.shas[1]);
    assert_eq!(fork["branch"], "topic");
    assert_eq!(fork["remote"], Value::Null, "no remote is added for it");
    assert_eq!(fork["start"], h.shas[1]);
    h.twin(&["fetch", "-q", "origin", "refs/pull/8/head"]);
    h.same();
    // A worktree starts there, at the head.
    let folder = h.world.path("topic-worktree");
    let registry = bitbraid_integrations::registry();
    let mut ctx = CommandContext::default();
    registry
        .run(
            "repo.open",
            &mut ctx,
            &json!({ "path": h.world.path("ours").to_str().unwrap() }),
        )
        .unwrap();
    registry
        .run(
            "worktree.add",
            &mut ctx,
            &json!({ "path": folder.to_str().unwrap(), "detach": true, "start": fork["start"] }),
        )
        .unwrap();
    let at = Git::new(&folder, &h.world.home).rev_parse("HEAD").unwrap();
    assert_eq!(at, h.shas[1]);

    // Checked out once, the fork is a remote here: its branch is the start.
    h.checkout(8);
    let again = head(8);
    assert_eq!(again["remote"], "hubot");
    assert_eq!(again["start"], "hubot/topic");
}
