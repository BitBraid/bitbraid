//! Suggestions from local edits: what the live tests set up at both
//! providers and here, and the replays of what they recorded. A file of
//! the writes' repository is changed by a pull request; a repository
//! here has that pull request's head checked out and the file edited in
//! its working tree in three places: a line both sides of the pull
//! request's diff have, a line it changed, and a line added after its
//! last one. `pr.suggestions` makes a suggestion of each, and each is sent
//! with `pr.comment` on its lines.
//!
//! The live tests (`live_github`, `live_gitlab_com`) record what they ask
//! into `tests/recordings/*/pr_suggestions.json`; the tests here ask the
//! same of the recordings.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::CommandContext;
use bitbraid_integrations::{Services, replay::Replay, secrets::MemoryStore};
use bitbraid_testkit::Git;
use serde_json::{Value, json};

/// The file, in the writes' repository.
pub const FILE: &str = "bb-w/suggest.txt";
/// The pull request's branch.
pub const BRANCH: &str = "bb-w-suggest";
/// The file as the base branch has it.
pub const BASE: &str = "one\ntwo\nthree\nfour\nfive\nsix\nseven\neight\n";
/// As the pull request's head has it: two lines changed, one added.
pub const HEAD: &str = "one\ntwo\nTHREE\nFOUR\nfive\nsix\nseven\neight\nnine\n";
/// As the working tree here has it.
pub const EDITED: &str = "ONE\ntwo\nThree\nFOUR\nfive\nsix\nseven\neight\nnine\nten\n";

/// A repository in `dir` with the head checked out and the file edited.
pub fn local_repo(dir: &Path, home: &Path) -> PathBuf {
    std::fs::create_dir_all(dir.join("bb-w")).unwrap();
    let git = Git::new(dir, home);
    git.ensure_home().unwrap();
    git.run(["init", "-q", "-b", BRANCH]).unwrap();
    std::fs::write(dir.join(FILE), HEAD).unwrap();
    git.run(["add", FILE]).unwrap();
    git.run(["commit", "-q", "-m", "The head of the pull request"])
        .unwrap();
    std::fs::write(dir.join(FILE), EDITED).unwrap();
    dir.to_path_buf()
}

/// The lines each suggestion is on and what it offers, with whether it
/// can be sent.
pub fn shape(answer: &Value) -> Vec<(u64, u64, Vec<String>, bool)> {
    answer["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suggestion| {
            (
                suggestion["start_line"].as_u64().unwrap(),
                suggestion["line"].as_u64().unwrap(),
                suggestion["suggested"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|line| line.as_str().unwrap().to_owned())
                    .collect(),
                suggestion["why_not"].is_null(),
            )
        })
        .collect()
}

/// What the three suggestions are, at either provider.
pub fn expected() -> Vec<(u64, u64, Vec<String>, bool)> {
    let lines = |lines: &[&str]| lines.iter().map(|line| (*line).to_owned()).collect();
    vec![
        (1, 1, lines(&["ONE"]), true),
        (3, 3, lines(&["Three"]), true),
        (9, 9, lines(&["nine", "ten"]), true),
    ]
}

/// The parameters of `pr.comment` that send a suggestion.
pub fn sent(suggestion: &Value) -> Value {
    let mut params = json!({
        "path": suggestion["path"],
        "line": suggestion["line"],
        "body": suggestion["body"],
    });
    if suggestion["start_line"] != suggestion["line"] {
        params["start_line"] = suggestion["start_line"].clone();
    }
    params
}

// The replays: the same commands, in the order the live tests asked them,
// answered from what they recorded.

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A recording of a provider, with its account connected as the live
/// test did, and a repository here as the live test had it.
struct Replayed {
    dir: PathBuf,
    replay: Arc<Replay>,
    services: Arc<Services>,
    account: String,
    local: PathBuf,
}

impl Replayed {
    fn new(site: &str, kind: &str, token: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-t-replayed-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let recording = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/recordings")
            .join(site)
            .join("pr_suggestions.json");
        let replay = Arc::new(Replay::load(&recording).unwrap());
        let services = Arc::new(Services::new(
            replay.clone(),
            Arc::new(MemoryStore::new()),
            &dir,
        ));
        let local = local_repo(&dir.join("work"), &dir.join("home"));
        let mut replayed = Self {
            dir,
            replay,
            services,
            account: String::new(),
            local,
        };
        let account = replayed.run(
            None,
            "account.add_token",
            json!({ "kind": kind, "token": token }),
        );
        replayed.account = account["id"].as_str().unwrap().to_owned();
        replayed
    }

    /// A command, with the repository here open when `here`.
    fn run(&self, here: Option<&Path>, id: &str, params: Value) -> Value {
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(self.services.clone());
        if let Some(repo) = here {
            registry
                .run("repo.open", &mut ctx, &json!({ "path": repo }))
                .unwrap();
        }
        registry
            .run(id, &mut ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    /// The live test's commands again: the pull request of the branch,
    /// the suggestions made here, each sent, the pull request read again.
    fn again(&self, owner: &str) -> (Value, Value) {
        let on = |extra: Value| {
            let mut params =
                json!({ "account": self.account, "owner": owner, "name": "bitbraid-live-writes" });
            for (key, value) in extra.as_object().unwrap() {
                params[key] = value.clone();
            }
            params
        };
        let found = self.run(None, "pr.for_branch", on(json!({ "branch": BRANCH })));
        let number = found["number"].as_u64().unwrap();
        let offered = self.run(
            Some(&self.local),
            "pr.suggestions",
            on(json!({ "number": number })),
        );
        assert_eq!(shape(&offered), expected());
        for suggestion in offered["suggestions"].as_array().unwrap() {
            let mut params = sent(suggestion);
            params["number"] = json!(number);
            let comment = self.run(None, "pr.comment", on(params));
            assert_eq!(comment["body"], suggestion["body"]);
        }
        let shown = self.run(None, "pr.show", on(json!({ "number": number })));
        assert!(
            self.replay.unused().is_empty(),
            "{:?}",
            self.replay.unused()
        );
        (offered, shown)
    }
}

impl Drop for Replayed {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The lines of the file the threads of the pull request are on.
fn thread_lines(shown: &Value) -> Vec<u64> {
    let mut lines: Vec<u64> = shown["threads"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|thread| thread["path"] == FILE)
        .map(|thread| thread["line"].as_u64().unwrap())
        .collect();
    lines.sort_unstable();
    lines
}

#[test]
fn github_takes_suggestions_from_local_edits_as_blocks_on_their_lines() {
    let replayed = Replayed::new("github-org", "github", "ghx_replayed_0");
    let (offered, shown) = replayed.again("org-1");
    let bodies: Vec<&str> = offered["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suggestion| suggestion["body"].as_str().unwrap())
        .collect();
    assert_eq!(
        bodies,
        [
            "```suggestion\nONE\n```",
            "```suggestion\nThree\n```",
            "```suggestion\nnine\nten\n```"
        ]
    );
    assert_eq!(thread_lines(&shown), [1, 3, 9]);
}

#[test]
fn gitlab_takes_suggestions_from_local_edits_as_threads_on_the_diff() {
    let replayed = Replayed::new("gitlab-com", "gitlab", "glpat-replayed");
    let (offered, shown) = replayed.again("user-1");
    assert_eq!(
        offered["suggestions"][2]["body"],
        "```suggestion:-0+0\nnine\nten\n```"
    );
    assert_eq!(thread_lines(&shown), [1, 3, 9]);
    // The thread on the line both sides have is placed on both.
    let threads = shown["threads"].as_array().unwrap();
    assert!(
        threads
            .iter()
            .any(|thread| thread["comments"][0]["body"] == "```suggestion:-0+0\nONE\n```"),
        "{shown}"
    );
}

/// What the working tree differs in from the head, where the head checked
/// out is not the pull request's: listed, with the reason, not sent.
#[test]
fn a_head_that_is_not_the_pull_requests_is_said_so() {
    let replayed = Replayed::new("github-org", "github", "ghx_replayed_0");
    // The commit here reads otherwise on line 3.
    let git = Git::new(&replayed.local, replayed.dir.join("home"));
    std::fs::write(
        replayed.local.join(FILE),
        HEAD.replace("THREE", "three, older"),
    )
    .unwrap();
    git.run(["commit", "-q", "-am", "Older"]).unwrap();
    std::fs::write(replayed.local.join(FILE), EDITED).unwrap();
    let on =
        json!({ "account": replayed.account, "owner": "org-1", "name": "bitbraid-live-writes" });
    let found = replayed.run(
        None,
        "pr.for_branch",
        json!({ "account": on["account"], "owner": "org-1", "name": "bitbraid-live-writes",
                "branch": BRANCH }),
    );
    let offered = replayed.run(
        Some(&replayed.local),
        "pr.suggestions",
        json!({ "account": on["account"], "owner": "org-1", "name": "bitbraid-live-writes",
                "number": found["number"] }),
    );
    let why: Vec<bool> = offered["suggestions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|suggestion| suggestion["why_not"].is_null())
        .collect();
    assert_eq!(why, [true, false, true], "{offered}");
    assert!(
        offered["suggestions"][1]["why_not"]
            .as_str()
            .unwrap()
            .contains("up to date")
    );
}
