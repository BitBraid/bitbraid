//! The window's pull requests against a GitLab of one's own, live: a
//! merge request made from the panel, changed, commented on, checked out
//! and merged from its view, with the real provider commands and the real
//! network, and looked at afterwards through GitLab's own API.
//!
//! Opt-in: it does anything only when `BITBRAID_GITLAB_CE` names a
//! server on this machine (`http://127.0.0.1:8929`) and
//! `BITBRAID_GITLAB_CE_TOKEN_FILE` a file holding an administrator's
//! personal access token (scope `api`), which is read here and sent to
//! that server only; otherwise it says it was skipped. It is no
//! `#[ignore]` test: the variables alone ask for it, and it needs no
//! renderer (Linux only, as the program of these tests):
//!
//! ```sh
//! BITBRAID_GITLAB_CE=http://127.0.0.1:8929 BITBRAID_GITLAB_CE_TOKEN_FILE=<file> \
//!     cargo test -p bitbraid-ui --test offscreen live_gitlab
//! ```
//! It makes what it needs through the
//! administrator's API, without making it twice: the user `bb-b2-amy`
//! with a token for the run (her password is made for the run and kept
//! nowhere), her public project `bb-b2-project`, and, each run, a branch
//! `bb-b2-<run>` with one commit, whose merge request the test merges and
//! whose branch it removes. The account's token is kept in a keychain in
//! memory; nothing reaches the machine's.
//!
//! Sources: docs.gitlab.com/api "Users" (create, personal access tokens
//! of a user), "Projects" (create for a user), "Commits" (create with
//! actions), "Merge requests".
#![allow(clippy::print_stdout)]

use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitbraid_integrations::{
    api::{encode, rfc3339},
    http::{Call, Client, Fetch as _, Limits, Method, Url},
    secrets::{MemoryStore, Secret},
    services::Services,
};
use bitbraid_ui::views::{pulls::create::NewPullProbe, rewrite::Rewrite};
use gpui_kit::{point, px, test::TestWindowExt as _};
use serde_json::{Value, json};

use crate::{
    pulls, repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[Test {
    name: "live_gitlab_a_merge_request_is_made_changed_checked_out_and_merged",
    run: live_gitlab_a_merge_request_is_made_changed_checked_out_and_merged,
}];

const USER: &str = "bb-b2-amy";
const PROJECT: &str = "bb-b2-project";

/// The server and the administrator's token, when the test is asked for.
fn server() -> Option<(String, Secret)> {
    let base = std::env::var("BITBRAID_GITLAB_CE").ok()?;
    let file = std::env::var("BITBRAID_GITLAB_CE_TOKEN_FILE").ok()?;
    let token = std::fs::read_to_string(file).ok()?.trim().to_owned();
    let base = base.trim().trim_end_matches('/').to_owned();
    // Only a server on this machine.
    let url = Url::parse(&base).ok()?;
    let local = url.host() == "localhost"
        || url
            .host()
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback());
    local.then(|| (base, Secret::new(token)))
}

/// Seconds since the epoch by the real clock: the window's is held still.
fn real_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// Something new for this run.
fn run_mark() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.subsec_nanos());
    format!(
        "{:x}{:x}",
        real_now() % 0x10_0000,
        nanos ^ std::process::id()
    )
}

/// GitLab's API, as the administrator or as a user.
struct Api {
    client: Client,
    base: String,
    token: Secret,
}

impl Api {
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = Url::parse(&format!("{}/api/v4{path}", self.base)).unwrap();
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
            .client
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
}

/// The user, her project and a branch of this run with one commit: the
/// user's token for the run, the project's id, the branch.
fn setup(admin: &Api, mark: &str) -> (Secret, u64, String) {
    let (_, found) = admin.call(Method::Get, &format!("/users?username={USER}"), None);
    let user = match found.as_array().and_then(|users| users.first()) {
        Some(user) => user["id"].as_u64().unwrap(),
        None => admin.ok(
            Method::Post,
            "/users",
            Some(json!({
                "username": USER,
                "name": "Amy Window",
                "email": format!("{USER}@example.invalid"),
                // Made for this run and kept nowhere: nothing signs in with it.
                "password": format!("Bb-{mark}-{mark}-b2"),
                "skip_confirmation": true,
            })),
        )["id"]
            .as_u64()
            .unwrap(),
    };
    let expires = rfc3339(real_now().saturating_add(2 * 86_400) as i64);
    let made = admin.ok(
        Method::Post,
        &format!("/users/{user}/personal_access_tokens"),
        Some(json!({
            "name": format!("bb-b2-{mark}"),
            "scopes": ["api", "read_user"],
            "expires_at": expires.get(..10).unwrap_or_default(),
        })),
    );
    let token = Secret::new(made["token"].as_str().unwrap());
    let path = encode(&format!("{USER}/{PROJECT}"));
    let (status, project) = admin.call(Method::Get, &format!("/projects/{path}"), None);
    let project = if status == 200 {
        project["id"].as_u64().unwrap()
    } else {
        admin.ok(
            Method::Post,
            &format!("/projects/user/{user}"),
            Some(json!({
                "name": PROJECT,
                "path": PROJECT,
                "initialize_with_readme": true,
                "default_branch": "main",
                // Public, so that git fetches from it without a password.
                "visibility": "public",
            })),
        )["id"]
            .as_u64()
            .unwrap()
    };
    let branch = format!("bb-b2-{mark}");
    let commit = admin.ok(
        Method::Post,
        &format!("/projects/{project}/repository/commits"),
        Some(json!({
            "branch": branch,
            "start_branch": "main",
            "commit_message": "Notes from the window's live test",
            "author_name": "Amy Window",
            "author_email": format!("{USER}@example.invalid"),
            "actions": [{
                "action": "create",
                "file_path": format!("{branch}.txt"),
                "content": "a line made by the window's live test\n",
            }],
        })),
    );
    // A check on its commit, as another service would set one.
    let sha = commit["id"].as_str().unwrap().to_owned();
    admin.ok(
        Method::Post,
        &format!("/projects/{project}/statuses/{sha}"),
        Some(json!({
            "state": "success",
            "name": "bb-b2-check",
            "description": "set by the window's live test",
        })),
    );
    (token, project, branch)
}

fn panel(stage: &mut Stage) -> NewPullProbe {
    stage.with_workspace(|workspace, _, cx| {
        workspace
            .read(cx)
            .new_pull()
            .expect("the panel is open")
            .read(cx)
            .probe()
    })
}

/// Wait for something that takes a server's answer, at most a minute.
fn waiting(stage: &mut Stage, what: &str, mut met: impl FnMut(&mut Stage) -> bool) {
    let started = std::time::Instant::now();
    loop {
        stage.settle();
        if met(stage) {
            return;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "waited a minute for {what}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The whole way: an account connected by the user's token, the branch
/// of this run made a merge request from the panel (its title from its
/// one commit), its title changed and a comment written in its view,
/// checked out by two clicks on its branch, and merged with its branch
/// removed; GitLab says so afterwards.
fn live_gitlab_a_merge_request_is_made_changed_checked_out_and_merged() {
    let Some((base, admin_token)) = server() else {
        println!("    skipped: BITBRAID_GITLAB_CE and BITBRAID_GITLAB_CE_TOKEN_FILE are not set");
        return;
    };
    let admin = Api {
        client: Client::new("BitBraid-Test/live"),
        base: base.clone(),
        token: admin_token,
    };
    let mark = run_mark();
    let (token, project, branch) = setup(&admin, &mark);

    repos::fresh();
    let work = repos::work();
    repos::git(
        &work,
        &[
            "clone",
            "-q",
            &format!("{base}/{USER}/{PROJECT}.git"),
            "live",
        ],
    );
    let repo = work.join("live");
    let config = crate::hermetic::inside().unwrap().join("config/bitbraid");
    Services::install(
        &config,
        Arc::new(Services::new(
            Arc::new(Client::new("BitBraid-Test/live")),
            Arc::new(MemoryStore::new()),
            &config,
        )),
    );
    let mut stage = Stage::open(Start::with(&repo));
    bitbraid_integrations::registry()
        .run(
            "account.add_token",
            &mut bitbraid_engine::CommandContext {
                config_dir: Some(config.clone()),
                ..Default::default()
            },
            &json!({ "kind": "gitlab", "web": base, "token": token.expose() }),
        )
        .unwrap();
    stage.with_workspace(|workspace, _, cx| {
        let pulls = workspace.read(cx).pulls();
        pulls.update(cx, |view, cx| view.refresh(cx));
    });
    waiting(&mut stage, "the remote to be linked", |stage| {
        pulls::probe(stage).groups.len() == 4
    });
    // The graph asks ci.status for the commits of its rows in view: the
    // branch's commit has its check marked.
    let graph = |stage: &mut Stage| {
        stage.with_workspace(|workspace, _, cx| workspace.read(cx).graph().read(cx).checks_probe())
    };
    waiting(&mut stage, "the graph's check marks", |stage| {
        let (asking, marked, _) = graph(stage);
        asking && marked >= 1
    });

    // The panel, from the branch of this run as fetched.
    let head = format!("origin/{branch}");
    stage.with_workspace(|workspace, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.rewrite(
                Rewrite::StartPull {
                    head: head.clone(),
                    base: None,
                },
                window,
                cx,
            )
        })
    });
    waiting(&mut stage, "the panel's reads", |stage| {
        let seen = panel(stage);
        seen.conflicts.is_some() && seen.title == "Notes from the window's live test"
    });
    let seen = panel(&mut stage);
    assert_eq!(seen.into, "origin main");
    assert!(!seen.push_first, "a branch of the remote is not pushed");
    stage.click("new-pull-create");
    waiting(
        &mut stage,
        "the merge request to be made and shown",
        |stage| {
            stage.workspace().sheet == Some("pull")
                && pulls::view_probe(stage)
                    .is_some_and(|seen| !seen.loading && seen.title.is_some())
        },
    );
    let number = pulls::view_probe(&mut stage).unwrap().number;
    let at_gitlab = |admin: &Api| {
        admin.ok(
            Method::Get,
            &format!("/projects/{project}/merge_requests/{number}"),
            None,
        )
    };
    let made = at_gitlab(&admin);
    // The lists are read again: the branch has an open merge request.
    waiting(&mut stage, "the branch's mark", |stage| graph(stage).2 >= 1);
    assert_eq!(made["source_branch"], json!(branch));
    assert_eq!(made["target_branch"], json!("main"));
    assert_eq!(made["state"], json!("opened"));

    // Its title, changed in place.
    stage.click("pull-title-edit");
    stage.keys("ctrl-a backspace");
    stage.type_text("Notes from the window, changed in place");
    stage.keys("enter");
    waiting(&mut stage, "the title to be changed", |stage| {
        pulls::view_probe(stage).is_some_and(|seen| {
            seen.title.as_deref() == Some("Notes from the window, changed in place")
        })
    });
    assert_eq!(
        at_gitlab(&admin)["title"],
        json!("Notes from the window, changed in place")
    );

    // A comment, from the field at the bottom.
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    stage.frame();
    stage.click("pull-comment-field");
    stage.type_text("Written in the window's live test.");
    stage.click("pull-comment");
    waiting(
        &mut stage,
        "the comment to be sent and read again",
        |stage| {
            pulls::view_probe(stage).is_some_and(|seen| {
                seen.composer.is_empty() && seen.sending.is_none() && seen.threads > 0
            })
        },
    );
    let notes = admin.ok(
        Method::Get,
        &format!("/projects/{project}/merge_requests/{number}/notes"),
        None,
    );
    assert!(
        notes.as_array().is_some_and(|notes| notes
            .iter()
            .any(|note| note["body"] == json!("Written in the window's live test."))),
        "the comment is at GitLab"
    );

    // Checked out by two clicks on its branch.
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(4000.))),
            cx,
        )
    });
    stage.frame();
    crate::pull_writes::double_click(&mut stage, "pull-head-branch");
    waiting(&mut stage, "the branch to be checked out", |_| {
        repos::git(&repo, &["rev-parse", "--abbrev-ref", "HEAD"]).trim() == branch
    });

    // Merged, its branch removed.
    stage.window(|window, cx| {
        window.scroll(
            "pull-page",
            gpui_kit::ScrollDelta::Pixels(point(px(0.), px(-4000.))),
            cx,
        )
    });
    stage.frame();
    // GitLab works out whether it merges a moment after it is made.
    let started = std::time::Instant::now();
    while at_gitlab(&admin)["detailed_merge_status"] != json!("mergeable") {
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "GitLab never said it merges"
        );
        std::thread::sleep(Duration::from_millis(500));
    }
    stage.click("pull-merge-delete");
    stage.click("pull-merge");
    stage.click(("ask-choice", 0usize));
    waiting(&mut stage, "the merge", |stage| {
        pulls::view_probe(stage).is_some_and(|seen| seen.sending.is_none())
            && at_gitlab(&admin)["state"] == json!("merged")
    });
    assert!(
        stage
            .notices()
            .iter()
            .any(|notice| notice.contains(&format!("Merged !{number}"))),
        "{:?}",
        stage.notices()
    );
    let (status, _) = admin.call(
        Method::Get,
        &format!(
            "/projects/{project}/repository/branches/{}",
            encode(&branch)
        ),
        None,
    );
    assert_eq!(status, 404, "its branch was removed");
}
