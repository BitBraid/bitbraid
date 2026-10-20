//! A branch for an issue and the issue a branch's name holds, held to git:
//! `issue.branch` runs on one scratch copy and git does the same on a
//! twin, and references, HEAD and branch configuration have to come out
//! the same; undo takes the branch back. The issue's title comes from a
//! recording of a GitLab of one's own, so nothing here needs a network.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use bitbraid_engine::{CommandContext, Registry};
use bitbraid_git::issue_keys::branch_for_issue;
use bitbraid_integrations::{Services, replay::Replay, secrets::MemoryStore};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// The account of the recordings' GitLab, as they name it.
const GITLAB: &str = "gitlab:127.0.0.1:8929:user-1";

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    dir: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

/// Everything a branch command can change, as text.
fn state(git: &Git) -> Vec<String> {
    let mut out = git.user_refs("%(refname) %(objectname)").unwrap();
    let head = git
        .run(["symbolic-ref", "--quiet", "HEAD"])
        .or_else(|_| git.run(["rev-parse", "HEAD"]))
        .unwrap();
    out.push(format!("HEAD {}", head.trim()));
    let config = git
        .run(["config", "--get-regexp", r"^branch\."])
        .unwrap_or_default();
    out.extend(config.lines().map(|line| format!("config {line}")));
    out
}

impl Harness {
    /// The `issue-keys` fixture open, and a provider that answers from
    /// `recording` (one that answers nothing when none is named).
    fn new(recording: Option<&str>) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("bb-a2-issue-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let replay = match recording {
            Some(name) => Replay::load(
                &Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/recordings")
                    .join(name),
            )
            .unwrap(),
            None => Replay::from_json(r#"{ "note": "nothing", "exchanges": [] }"#).unwrap(),
        };
        let services = Services::new(Arc::new(replay), Arc::new(MemoryStore::new()), &dir);
        let scratch = Scratch::of("issue-keys").unwrap();
        let twin = Scratch::of("issue-keys").unwrap();
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(dir.clone()),
            ..Default::default()
        };
        ctx.extensions.insert(Arc::new(services));
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            twin,
            dir,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.scratch.git()
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.try_run(id, params.clone())
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn assert_same(&self, what: &str) {
        assert_eq!(state(&self.git()), state(&self.twin.git()), "{what}");
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn the_issue_a_branch_names_is_read_by_the_rule() {
    let mut h = Harness::new(None);
    for (branch, number) in [
        ("123-fix", Some(123)),
        ("fix-123", Some(123)),
        ("feature/123-x", Some(123)),
        ("issue-123", Some(123)),
        ("gh-123", Some(123)),
        ("#123", Some(123)),
        ("12-handle-quotes", Some(12)),
        ("main", None),
        ("release-1.2", None),
        ("PROJ-123", None),
        ("2024-10-notes", None),
    ] {
        let out = h.run("issue.key_from_branch", json!({ "branch": branch }));
        assert_eq!(out["number"].as_u64(), number, "{branch}");
        assert_eq!(
            out["key"].as_str().map(str::to_owned),
            number.map(|number| format!("#{number}")),
            "{branch}"
        );
    }
    // Hostile names are refused or read as holding nothing, briefly.
    let long = format!("{}-1", "a".repeat(5000));
    let digits = format!("fix-{}", "9".repeat(400));
    for (params, refused) in [
        (json!({}), true),
        (json!({ "branch": "" }), true),
        (json!({ "branch": "  " }), true),
        (json!({ "branch": 12 }), true),
        (json!({ "branch": "a\u{0}1" }), true),
        (json!({ "branch": "12\nfix" }), true),
        (json!({ "branch": long }), true),
        (json!({ "branch": digits }), false),
        (json!({ "branch": "/////" }), false),
        (json!({ "branch": "-" }), false),
        (json!({ "branch": "#" }), false),
    ] {
        match h.try_run("issue.key_from_branch", params.clone()) {
            Ok(out) => {
                assert!(!refused, "{params}: {out}");
                assert_eq!(out["number"], Value::Null, "{params}");
            }
            Err(err) => {
                assert!(refused, "{params}: {err}");
                assert!(err.len() < 300, "{params}: {err}");
            }
        }
    }
}

#[test]
fn a_branch_for_an_issue_is_made_as_git_makes_it_and_undone() {
    let mut h = Harness::new(None);
    let before = state(&h.git());

    // A name the user settled on: no provider is asked.
    let out = h.run(
        "issue.branch",
        json!({ "number": 12, "branch": "12-quotes-again" }),
    );
    assert_eq!(out["created"], "12-quotes-again");
    assert_eq!(out["issue"], 12);
    assert_eq!(out["checked_out"], false);
    h.twin
        .git()
        .run(["branch", "--no-track", "12-quotes-again"])
        .unwrap();
    h.assert_same("a branch for an issue");
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        "Create branch 12-quotes-again"
    );
    h.run("undo.last", json!({}));
    assert_eq!(state(&h.git()), before);
    h.run("redo.last", json!({}));
    h.assert_same("redo");

    // Checked out, from a start of its own.
    h.run(
        "issue.branch",
        json!({ "number": 5, "branch": "feature/5-x", "start": "HEAD~2", "checkout": true }),
    );
    h.twin
        .git()
        .run(["switch", "-q", "--no-track", "-c", "feature/5-x", "HEAD~2"])
        .unwrap();
    h.assert_same("checked out");

    // What git would not take, or a branch there already is, changes
    // nothing.
    let before = state(&h.git());
    for (params, said) in [
        (json!({ "number": 1, "branch": "a..b" }), "two dots"),
        (json!({ "number": 1, "branch": "-x" }), "dash"),
        (json!({ "number": 1, "branch": "x.lock" }), ".lock"),
        (json!({ "number": 1, "branch": "a b" }), "space"),
        (
            json!({ "number": 1, "branch": "12-handle-quotes" }),
            "exists",
        ),
        (
            json!({ "number": 1, "branch": "y", "start": "nowhere" }),
            "",
        ),
        (json!({ "number": 0, "branch": "y" }), "number"),
        (
            json!({ "number": 5_000_000_000u64, "branch": "y" }),
            "number",
        ),
        (json!({ "number": "7", "branch": "y" }), "number"),
        (json!({ "branch": "y" }), "number"),
        (json!({ "number": 1, "branch": "y\u{1b}" }), "control"),
        (
            json!({ "number": 1, "branch": "y", "checkout": "yes" }),
            "checkout",
        ),
        (
            json!({ "number": 1, "template": "x".repeat(300) }),
            "template",
        ),
    ] {
        let err = h.try_run("issue.branch", params.clone()).unwrap_err();
        assert!(err.contains(said), "{params}: {err}");
        assert!(err.len() < 600, "{params}: {err}");
        assert_eq!(state(&h.git()), before, "{params}");
    }
    // Without a name, the title is the provider's, and there is none here.
    let err = h
        .try_run("issue.branch", json!({ "number": 1 }))
        .unwrap_err();
    assert!(err.contains("account"), "{err}");
    assert_eq!(state(&h.git()), before);
}

#[test]
fn a_branch_is_named_from_the_issue_the_provider_has() {
    let mut h = Harness::new(Some("gitlab/issues.json"));
    h.run(
        "account.add_token",
        json!({ "kind": "gitlab", "web": "http://127.0.0.1:8929", "token": "glpat-replayed" }),
    );
    let on_project = |extra: Value| {
        let mut params = json!({ "account": GITLAB, "owner": "bb-p-group", "name": "bb-p-project", "number": 1 });
        for (key, value) in extra.as_object().unwrap() {
            params[key] = value.clone();
        }
        params
    };
    // "The button does nothing".
    let out = h.run("issue.branch", on_project(json!({})));
    assert_eq!(out["created"], "1-the-button-does-nothing");
    h.twin
        .git()
        .run(["branch", "--no-track", "1-the-button-does-nothing"])
        .unwrap();
    h.assert_same("named by the default template");

    let out = h.run(
        "issue.branch",
        on_project(json!({ "template": "fix/{number}/{title}", "checkout": true })),
    );
    assert_eq!(out["created"], "fix/1/the-button-does-nothing");
    h.twin
        .git()
        .run(["switch", "-q", "-c", "fix/1/the-button-does-nothing"])
        .unwrap();
    h.assert_same("named by a template and checked out");

    // A template that makes a name git would not take changes nothing.
    let before = state(&h.git());
    let err = h
        .try_run(
            "issue.branch",
            on_project(json!({ "template": "{title}.." })),
        )
        .unwrap_err();
    assert!(err.contains("template"), "{err}");
    assert_eq!(state(&h.git()), before);
}

#[test]
fn names_made_for_issues_are_names_git_takes() {
    let h = Harness::new(None);
    let git = h.git();
    let takes = |name: &str| git.run(["check-ref-format", "--branch", name]).is_ok();
    let titles = [
        "Fix the lexer",
        "The button does nothing",
        "Ünïcödé only — ☃",
        "   ",
        "..",
        "a.lock",
        "@{u}",
        "HEAD",
        "Fix: `quotes` [and] ~tilde^ ?*: \\back\\slash",
        &"long words ".repeat(40),
        &"x".repeat(500),
    ];
    let templates = [
        None,
        Some("{number}-{title}"),
        Some("feature/{number}-{title}"),
        Some("users/ada/{title}"),
        Some("{title}"),
        Some("issue-{number}"),
        Some("{number}/{title}/{title}"),
        Some("{title}.lock"),
        Some(".{title}"),
        Some("-{number}"),
        Some("a//{number}"),
        Some("{number}@{title}"),
        Some("x/{number}."),
    ];
    for template in templates {
        for title in titles {
            for number in [1, 12, 4_294_967_295] {
                match branch_for_issue(template, number, title) {
                    Ok(name) => {
                        assert!(takes(&name), "{template:?} {title:?}: {name:?}");
                        if template.is_none_or(|template| template.contains("{title}")) {
                            assert!(
                                name.chars().count() <= 60,
                                "{template:?} {title:?}: {name:?}"
                            );
                        }
                    }
                    Err(problem) => {
                        // Refused: the template, filled, is no name git
                        // takes, or makes none.
                        let filled = template
                            .unwrap_or("{number}-{title}")
                            .replace("{number}", &number.to_string())
                            .replace("{title}", "t");
                        assert!(
                            !takes(&filled) || problem.contains("template"),
                            "{template:?} {title:?}: {problem}"
                        );
                    }
                }
            }
        }
    }
}
