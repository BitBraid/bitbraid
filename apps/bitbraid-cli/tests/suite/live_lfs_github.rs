//! Git LFS at github.com, live: transfers and locks through the engine's
//! own commands, beside git-lfs by itself, in a private repository made
//! for the run and removed at its end, which is the one way GitHub gives
//! the LFS storage of a repository back.
//!
//! Opt-in and never in ci: it runs with `--ignored` when
//! `BITBRAID_LIVE_GITHUB_ORG` names the test organisation, and is skipped
//! with a message otherwise. It needs git-lfs, and the test accounts'
//! tokens in the keychain as the integrations crate's `live_github` keeps
//! them (`secret-tool`, service `bitbraid-test`, accounts `github-main`
//! and `github-reviewer`, each with its `login` attribute):
//!
//! ```sh
//! BITBRAID_LIVE_GITHUB_ORG=<organisation> [BITBRAID_LIVE_GITHUB_SECOND=1] \
//!     cargo test -p bitbraid-cli --test suite live_lfs_github:: -- --ignored
//! ```
//!
//! No account is named in this file: the organisation comes from that
//! variable, each login and token from its keychain item at run time, and
//! a token is never printed or written. A token has to answer `GET /user`
//! with its item's login before anything else is sent with it.
//!
//! The first test uses the organisation's machine account alone (item
//! `github-main`). The second takes a second person (item
//! `github-reviewer`), and only when `BITBRAID_LIVE_GITHUB_SECOND` is `1`
//! as well: what github.com does with a push that changes a file somebody
//! else has locked. The machine account gives the second person push
//! access to the repository of the run, and to nothing else; each locks a
//! file, and each pushes a change to the file the other holds, with
//! git-lfs's own check of locks at its default, on and off; however it
//! comes out, what the application says of it is checked. A token can
//! have less than its account: where the second person's may only read
//! (a fine-grained token without the right to write the contents), their
//! push and lock are refused in words of ours, they are shown the list
//! without whose each lock is, and the test then fails, saying what the
//! token lacks.
//!
//! A guard in front of the API lets through only: `GET /user`; making a
//! private repository `bitbraid-live-lfs-<seconds>` in the organisation; reading
//! and removing that one repository; and, for the second test, giving the
//! second person push access to it (the second person's token may only
//! ask `GET /user` and read that repository). Git and git-lfs are pointed
//! at that repository alone. The content is a few small files (about
//! 2 MiB in all, a few KiB in the second test), made for the run.
//!
//! Sources: docs.github.com/en/rest "Users" (get the authenticated user),
//! "Repositories" (create an organization repository, get, delete),
//! "Collaborators" (add a repository collaborator);
//! docs.github.com "About Git Large File Storage", "About storage and
//! bandwidth usage" (deleting a repository is what frees its storage);
//! git-lfs's `docs/api/locking.md`; git-lfs-config(5)
//! (`lfs.<url>.locksverify`), git-lfs-pre-push(1); secret-tool(1).
#![allow(clippy::print_stderr)]

use std::{
    collections::BTreeMap,
    path::Path,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

use bitbraid_integrations::{
    http::{Call, Client, Fetch, Limits, Method, Url},
    secrets::Secret,
};
use serde_json::{Value, json};

use crate::ASKPASS_SERVER as ONE;
use crate::live_lfs::{
    Copy, Run, Twin, answers, asked_for, content, figures, is_pointer, oid, same_files, started,
    write,
};

/// The engine's words for an account, or a token, that may not write.
const NOT_ALLOWED: &str = "The server did not let the account that signed in do this";
const PREFIX: &str = "bitbraid-live-lfs-";
const ITEM: &str = "github-main";
const SECOND_ITEM: &str = "github-reviewer";

/// The login stored with a keychain item, from its attributes, which
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
        .filter(|login| {
            !login.is_empty()
                && login
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// A token, from its keychain item.
fn keychain_token(item: &str) -> Option<Secret> {
    let out = std::process::Command::new("secret-tool")
        .args(["lookup", "service", "bitbraid-test", "account", item])
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !token.is_empty()).then(|| Secret::new(token))
}

/// The test organisation, from `BITBRAID_LIVE_GITHUB_ORG`.
fn org_from_env() -> Option<String> {
    std::env::var("BITBRAID_LIVE_GITHUB_ORG")
        .ok()
        .map(|org| org.trim().to_owned())
        .filter(|org| {
            !org.is_empty() && org.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
}

/// The second person: their login, which the machine account may give
/// push access to the repository of the run, and their token.
struct Second {
    login: String,
    token: Secret,
}

/// The API, held to what this test may touch.
struct GitHub {
    client: Client,
    token: Secret,
    org: String,
    repo: String,
    second: Option<Second>,
}

impl GitHub {
    /// Whether a request is one the test may send, with the machine
    /// account's token or (`as_second`) the second person's.
    fn allowed(&self, as_second: bool, method: Method, path: &str, body: Option<&Value>) -> bool {
        let own = format!("/repos/{}/{}", self.org, self.repo);
        if as_second {
            return self.second.is_some()
                && method == Method::Get
                && (path == "/user" || path == own);
        }
        match method {
            Method::Get => path == "/user" || path == own,
            Method::Post => {
                path == format!("/orgs/{}/repos", self.org)
                    && body.is_some_and(|body| body["name"] == self.repo.as_str())
                    && body.is_some_and(|body| body["private"] == true)
            }
            Method::Put => self.second.as_ref().is_some_and(|second| {
                path == format!("{own}/collaborators/{}", second.login)
                    && body.is_some_and(|body| body == &json!({ "permission": "push" }))
            }),
            Method::Delete => path == own,
            _ => false,
        }
    }

    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        self.send(false, method, path, body)
    }

    /// A request with the second person's token.
    fn call_as_second(&self, method: Method, path: &str) -> (u16, Value) {
        self.send(true, method, path, None)
    }

    fn send(
        &self,
        as_second: bool,
        method: Method,
        path: &str,
        body: Option<Value>,
    ) -> (u16, Value) {
        assert!(
            self.allowed(as_second, method, path, body.as_ref()),
            "refused by the live test's guard: {} {path}",
            method.as_str()
        );
        let token = match (&self.second, as_second) {
            (Some(second), true) => &second.token,
            _ => &self.token,
        };
        let url = Url::parse(&format!("https://api.github.com{path}")).unwrap();
        let mut call = Call::new(
            method,
            url,
            Limits {
                connect: Duration::from_secs(15),
                total: Duration::from_secs(60),
                body: 4 << 20,
            },
        )
        .with_header("Accept", "application/vnd.github+json")
        .with_header("X-GitHub-Api-Version", "2022-11-28")
        .with_header("Authorization", &format!("Bearer {}", token.expose()));
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
}

/// Whether a token answers `GET /user` with the login it is kept with.
fn is_of(user: &(u16, Value), login: &str) -> bool {
    user.0 == 200
        && user.1["login"]
            .as_str()
            .is_some_and(|said| said.eq_ignore_ascii_case(login))
}

impl GitHub {
    /// The machine account's API, its token checked, for a repository of
    /// the run that is not made yet; and the machine account's login.
    fn of_this_run(org: &str, second: Option<Second>) -> (Self, String) {
        let login = keychain_login(ITEM).expect("the machine account's keychain item");
        let github = Self {
            client: Client::new("BitBraid-Test/live-lfs"),
            token: keychain_token(ITEM).expect("the machine account's token"),
            org: org.to_owned(),
            repo: format!("{PREFIX}{}", bitbraid_core::clock::unix_seconds()),
            second,
        };
        assert!(
            is_of(&github.call(Method::Get, "/user", None), &login),
            "the token is not its keychain item's login's: stopping"
        );
        if let Some(second) = &github.second {
            assert!(
                is_of(&github.call_as_second(Method::Get, "/user"), &second.login),
                "the second token is not its keychain item's login's: stopping"
            );
        }
        (github, login)
    }

    /// The private repository of the run, made now and removed when what
    /// this returns is dropped.
    fn make(&self) -> Made<'_> {
        let (status, made) = self.call(
            Method::Post,
            &format!("/orgs/{}/repos", self.org),
            Some(
                json!({ "name": self.repo, "private": true, "auto_init": false,
                         "has_issues": false, "has_projects": false, "has_wiki": false,
                         "description": "A throwaway repository of BitBraid's live LFS test" }),
            ),
        );
        assert_eq!(
            status, 201,
            "the repository was not made: {}",
            made["message"]
        );
        eprintln!("made {}/{} (private)", self.org, self.repo);
        Made(self)
    }

    /// The address of the repository of the run for `login`.
    fn url(&self, login: &str) -> String {
        format!("https://{login}@github.com/{}/{}.git", self.org, self.repo)
    }
}

/// The repository of the run, removed when this is dropped, also when the
/// test fails.
struct Made<'a>(&'a GitHub);

impl Drop for Made<'_> {
    fn drop(&mut self) {
        let github = self.0;
        let path = format!("/repos/{}/{}", github.org, github.repo);
        let (status, _) = github.call(Method::Delete, &path, None);
        let (after, _) = github.call(Method::Get, &path, None);
        eprintln!(
            "removed {}: DELETE answered {status}, then GET {after}",
            github.repo
        );
        // Never a second panic while a failure unwinds: that would end
        // the process before it said anything.
        if after != 404 && !std::thread::panicking() {
            panic!("the repository of the run is still there: remove it by hand");
        }
    }
}

/// What `lfs.locks` lists, as (path, ours) by path.
fn listed(locks: &Value) -> Vec<(String, Option<bool>)> {
    let mut out: Vec<(String, Option<bool>)> = locks
        .as_array()
        .unwrap()
        .iter()
        .map(|lock| {
            (
                lock["path"].as_str().unwrap().to_owned(),
                lock["ours"].as_bool(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
#[ignore = "live: github.com with the test organisation (BITBRAID_LIVE_GITHUB_ORG)"]
fn github_moves_lfs_content_and_keeps_locks_in_a_repository_of_the_run() {
    let Some(org) = org_from_env() else {
        eprintln!(
            "skipped: set BITBRAID_LIVE_GITHUB_ORG to the test organisation, with its machine \
             account in the keychain"
        );
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let (github, login) = GitHub::of_this_run(&org, None);
    let _made = github.make();
    let url = github.url(&login);
    let asking = answers(BTreeMap::from([(login.clone(), github.token.clone())]));
    let run = Run::new("github");

    // Transfers: pushed, fetched, pulled, against git-lfs's twin.
    let a = content(51, 300 * 1024);
    let b = content(52, 700 * 1024);
    let c = content(53, 1 << 20);
    let files: [(&str, &[u8]); 3] = [("a.bin", &a), ("b.bin", &b), ("sub/c.bin", &c)];
    let names = ["a.bin", "b.bin", "sub/c.bin"];
    let ours = run.path("ours");
    let mut copy = started(&run, &ours, true, &files);
    run.git(&ours)
        .run(["remote", "add", "origin", &url])
        .unwrap();
    let seen = copy.watch(None);
    let began = std::time::Instant::now();
    copy.ok("lfs.push", json!({}));
    eprintln!("lfs.push of 2 MiB took {:?}", began.elapsed());
    assert_eq!(
        figures(&seen, "Uploading LFS objects").last(),
        Some(&100),
        "{:?}",
        seen.lock().unwrap()
    );
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    let theirs = run.path("clone");
    let mut clone = Copy::cloned(&run, &url, &theirs);
    assert!(is_pointer(&theirs, "b.bin"));
    clone.ok("lfs.install", json!({}));
    let twin = Twin::clone(&run, &url, &run.path("twin"), &[]);
    let seen = clone.watch(None);
    clone.ok("lfs.fetch", json!({}));
    twin.ok(&["lfs", "fetch", "origin"]);
    assert_eq!(figures(&seen, "Downloading LFS objects").last(), Some(&100));
    assert_eq!(clone.store(), twin.store());
    assert_eq!(clone.store().len(), 3);
    clone.ok("lfs.pull", json!({}));
    twin.ok(&["lfs", "pull", "origin"]);
    same_files(&theirs, twin.git.cwd(), &names);
    same_files(&theirs, &ours, &names);
    let fresh_file = content(54, 200 * 1024);
    write(twin.git.cwd(), "new.bin", &fresh_file);
    twin.ok(&["add", "new.bin"]);
    twin.ok(&["commit", "-q", "-m", "a new LFS file"]);
    twin.ok(&["push", "-q", "origin", "main"]);
    clone.ok("remote.pull", json!({ "mode": "ff_only" }));
    same_files(&theirs, twin.git.cwd(), &["new.bin"]);
    assert!(clone.store().contains(&oid(&fresh_file)));

    // Locks.
    assert_eq!(listed(&copy.ok("lfs.locks", json!({}))), vec![]);
    let made = copy.ok("lfs.lock", json!({ "path": "a.bin" }));
    // GitHub answers a new lock without its owner; it is ours all the
    // same.
    eprintln!("the new lock, as answered: {made}");
    assert!(
        made["owner"].is_null() || made["owner"] == login.as_str(),
        "another owner"
    );
    assert_eq!(made["ours"], true);
    let seen = clone.ok("lfs.locks", json!({}));
    assert_eq!(listed(&seen), vec![("a.bin".to_owned(), Some(true))]);
    assert_eq!(seen[0]["owner"], login.as_str(), "the owner in the list");
    let again = clone.refused("lfs.lock", json!({ "path": "a.bin" }));
    assert!(
        again.contains("a.bin is already locked by"),
        "locked again: {again}"
    );

    // Per branch? A lock taken on a branch of its own, seen from main.
    let ours_git = run.git(&ours);
    ours_git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "topic", "set_upstream": true }),
    );
    copy.ok("lfs.lock", json!({ "path": "b.bin" }));
    let from_main = listed(&clone.ok("lfs.locks", json!({})));
    assert_eq!(
        from_main,
        vec![
            ("a.bin".to_owned(), Some(true)),
            ("b.bin".to_owned(), Some(true))
        ],
        "github.com keeps locks for the whole repository"
    );
    let err = clone.refused("lfs.lock", json!({ "path": "b.bin" }));
    assert!(err.contains("b.bin is already locked by"), "{err}");
    ours_git.run(["switch", "-q", "main"]).unwrap();
    copy.ok("lfs.unlock", json!({ "path": "b.bin" }));

    // A lockable file is left read-only where it is not locked (git-lfs's
    // `lfs.setlockablereadonly`); where it is, a push that changes it goes
    // through.
    assert!(
        std::fs::metadata(theirs.join("a.bin"))
            .unwrap()
            .permissions()
            .readonly(),
        "a lockable file that is not locked here is writable"
    );
    copy.ok("remote.pull", json!({ "mode": "ff_only" }));
    std::fs::write(ours.join("a.bin"), content(55, 4096)).unwrap();
    ours_git
        .run(["commit", "-q", "-am", "change a file of one's own lock"])
        .unwrap();
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    );

    // One's own, given up; and by force.
    copy.ok("lfs.unlock", json!({ "path": "a.bin" }));
    let err = copy.refused("lfs.unlock", json!({ "path": "a.bin" }));
    assert!(err.contains("a.bin is not locked"), "{err}");
    copy.ok("lfs.lock", json!({ "path": "sub/c.bin" }));
    copy.ok("lfs.unlock", json!({ "path": "sub/c.bin", "force": true }));
    assert_eq!(listed(&copy.ok("lfs.locks", json!({}))), vec![]);

    let asked = asking.take();
    for (_, prompt) in &asked {
        assert!(
            asked_for(prompt) == Some(login.as_str()),
            "a question for somebody else"
        );
    }
    eprintln!(
        "{} questions, all for the machine account's token",
        asked.len()
    );
}

/// Put `bytes` in place of a file, also where git-lfs left it read-only
/// (a lockable file that is not locked here).
fn overwrite(path: &Path, bytes: &[u8]) {
    let _ = std::fs::remove_file(path);
    std::fs::write(path, bytes).unwrap();
}

/// What became of a push: what the command returned, or what it said.
type Pushed = Result<Value, String>;

/// One person's change to `file`, which the other holds, committed and
/// pushed to a new branch three times: with git-lfs's check of locks at
/// its default (unset), on and off (`lfs.<url>.locksverify`). What each
/// push came to, by the setting.
fn push_the_others(
    run: &Run,
    copy: &mut Copy,
    url: &str,
    file: &str,
    seed: u64,
    tag: &str,
) -> Vec<(&'static str, Pushed)> {
    let git = run.git(&copy.dir);
    overwrite(&copy.dir.join(file), &content(seed, 4096));
    git.run([
        "commit",
        "-q",
        "-am",
        &format!("change {file}, which the other holds"),
    ])
    .unwrap();
    let key = format!("lfs.{url}/info/lfs.locksverify");
    let mut out = Vec::new();
    for setting in ["unset", "true", "false"] {
        let _ = git.run(["config", "--unset", &key]);
        if setting != "unset" {
            git.run(["config", &key, setting]).unwrap();
        }
        let to = format!("{tag}-{setting}");
        let came = copy.run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main", "to": to }),
        );
        match &came {
            Ok(value) => eprintln!("{tag}, locksverify {setting}: pushed: {value}"),
            Err(err) => eprintln!("{tag}, locksverify {setting}: refused:\n{err}"),
        }
        out.push((setting, came));
    }
    let _ = git.run(["config", "--unset", &key]);
    out
}

/// What a push that changes `file`, which `holder` has locked, came to,
/// held to how it is said: with git-lfs's check on it never goes through,
/// and is stopped by the pre-push hook, which names the file and its
/// holder; a refusal by the server is said in words of ours first.
fn said_well(setting: &str, came: &Pushed, file: &str, holder: &str) {
    match came {
        Ok(_) => assert_ne!(
            setting, "true",
            "git-lfs's check of locks, on, let a push of {file} through"
        ),
        Err(err) if err.contains("the pre-push hook") => {
            assert!(err.contains(&format!("{file} - {holder}")), "{err}");
        }
        Err(err) => {
            assert_ne!(setting, "true", "not stopped by git-lfs's check: {err}");
            assert!(
                err.contains("it changes a file that somebody else has locked"),
                "the server's refusal is not said in words of ours: {err}"
            );
        }
    }
}

#[test]
#[ignore = "live: github.com with the test organisation and a second person \
            (BITBRAID_LIVE_GITHUB_ORG, BITBRAID_LIVE_GITHUB_SECOND)"]
fn github_and_git_lfs_meet_a_push_of_a_file_somebody_else_has_locked() {
    let Some(org) = org_from_env() else {
        eprintln!(
            "skipped: set BITBRAID_LIVE_GITHUB_ORG to the test organisation, with its machine \
             account in the keychain"
        );
        return;
    };
    if std::env::var("BITBRAID_LIVE_GITHUB_SECOND").as_deref() != Ok("1") {
        eprintln!(
            "skipped: set BITBRAID_LIVE_GITHUB_SECOND=1 as well, with the second person in the \
             keychain ({SECOND_ITEM})"
        );
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let second = Second {
        login: keychain_login(SECOND_ITEM).expect("the second person's keychain item"),
        token: keychain_token(SECOND_ITEM).expect("the second person's token"),
    };
    let (second_login, second_token) = (second.login.clone(), second.token.clone());
    let (github, first_login) = GitHub::of_this_run(&org, Some(second));
    assert!(
        !first_login.eq_ignore_ascii_case(&second_login),
        "one person twice"
    );
    let _made = github.make();

    // Push access for the second person, to this repository alone. A
    // member of the organisation has it at once (204); anybody else
    // would be sent an invitation (201), which this test does not accept.
    let own = format!("/repos/{org}/{}", github.repo);
    let (status, said) = github.call(
        Method::Put,
        &format!("{own}/collaborators/{second_login}"),
        Some(json!({ "permission": "push" })),
    );
    assert_eq!(
        status, 204,
        "push access for the second person: {status} {said}"
    );
    let until = Instant::now() + Duration::from_secs(60);
    loop {
        let (status, repo) = github.call_as_second(Method::Get, &own);
        if status == 200 && repo["permissions"]["push"] == true {
            break;
        }
        assert!(
            Instant::now() < until,
            "the second person cannot push after a minute: {status}"
        );
        std::thread::sleep(Duration::from_secs(2));
    }
    let asking = answers(BTreeMap::from([
        (first_login.clone(), github.token.clone()),
        (second_login.clone(), second_token),
    ]));
    let run = Run::new("github-two");

    // The machine account starts the repository with two lockable files;
    // the second person clones it.
    let files: [(&str, &[u8]); 2] = [("a.bin", &content(61, 4096)), ("b.bin", &content(62, 4096))];
    let first_dir = run.path("first");
    let first_url = github.url(&first_login);
    let mut first = started(&run, &first_dir, true, &files);
    run.git(&first_dir)
        .run(["remote", "add", "origin", &first_url])
        .unwrap();
    first.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    let second_dir = run.path("second");
    let second_url = github.url(&second_login);
    let mut second = Copy::cloned(&run, &second_url, &second_dir);
    second.ok("lfs.install", json!({}));
    second.ok("lfs.pull", json!({}));

    // Each locks a file.
    first.ok("lfs.lock", json!({ "path": "a.bin" }));
    if let Err(refused) = second.run("lfs.lock", json!({ "path": "b.bin" })) {
        // The role may push, but a token can have less than its account
        // (a fine-grained one that may read the contents and not write
        // them). Then the second person is a reader: what they are told,
        // and what they are shown, before the test says it could not go
        // on.
        eprintln!("the second person's lock:\n{refused}");
        assert!(
            refused.starts_with(&format!("lfs: b.bin was not locked: {NOT_ALLOWED}")),
            "{refused}"
        );
        let pushed = second
            .run(
                "remote.push",
                json!({ "remote": "origin", "branch": "main", "to": "second-may-push" }),
            )
            .expect_err("a push by one who may not lock");
        eprintln!("the second person's push:\n{pushed}");
        assert!(pushed.contains(NOT_ALLOWED), "{pushed}");
        let seen = second.ok("lfs.locks", json!({}));
        assert_eq!(listed(&seen), vec![("a.bin".to_owned(), None)], "{seen}");
        assert_eq!(seen[0]["owner"], first_login.as_str());
        let by_itself = Twin {
            git: run.git(&second_dir),
        }
        .run(&["lfs", "locks", "--verify"]);
        eprintln!("git-lfs by itself, verifying as the second person: {by_itself:?}");
        let forced = second.refused("lfs.unlock", json!({ "path": "a.bin", "force": true }));
        eprintln!("the second person forcing the other's lock:\n{forced}");
        assert!(forced.contains("a.bin was not unlocked"), "{forced}");
        first.ok("lfs.unlock", json!({ "path": "a.bin" }));
        panic!(
            "the second person's token may not write to the repository of the run, though \
             their role may, so they cannot hold a lock and what github.com does with a \
             push of somebody else's locked file cannot be seen: give that token Contents \
             (read and write) for the test organisation's repositories"
        );
    }
    let seen = first.ok("lfs.locks", json!({}));
    assert_eq!(
        listed(&seen),
        vec![
            ("a.bin".to_owned(), Some(true)),
            ("b.bin".to_owned(), Some(false))
        ]
    );
    let seen = second.ok("lfs.locks", json!({}));
    assert_eq!(
        listed(&seen),
        vec![
            ("a.bin".to_owned(), Some(false)),
            ("b.bin".to_owned(), Some(true))
        ]
    );

    // Each pushes a change to the file the other holds.
    let by_second = push_the_others(&run, &mut second, &second_url, "a.bin", 63, "second");
    let by_first = push_the_others(&run, &mut first, &first_url, "b.bin", 64, "first");
    // And to the default branch, with git-lfs's check off.
    let key = format!("lfs.{second_url}/info/lfs.locksverify");
    let second_git = run.git(&second_dir);
    second_git.run(["config", &key, "false"]).unwrap();
    let to_main = second.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    );
    match &to_main {
        Ok(value) => eprintln!("second, to main, locksverify false: pushed: {value}"),
        Err(err) => eprintln!("second, to main, locksverify false: refused:\n{err}"),
    }
    second_git.run(["config", "--unset", &key]).unwrap();
    eprintln!(
        "outcomes (pushed or not): second {:?}; first {:?}; second to main {}",
        by_second
            .iter()
            .map(|(setting, came)| (*setting, came.is_ok()))
            .collect::<Vec<_>>(),
        by_first
            .iter()
            .map(|(setting, came)| (*setting, came.is_ok()))
            .collect::<Vec<_>>(),
        to_main.is_ok()
    );
    // Whoever stopped a push, it is said well.
    for (setting, came) in &by_second {
        said_well(setting, came, "a.bin", &first_login);
    }
    for (setting, came) in &by_first {
        said_well(setting, came, "b.bin", &second_login);
    }
    said_well("false", &to_main, "a.bin", &first_login);

    first.ok("lfs.unlock", json!({ "path": "a.bin" }));
    second.ok("lfs.unlock", json!({ "path": "b.bin" }));
    assert_eq!(listed(&first.ok("lfs.locks", json!({}))), vec![]);
    let asked = asking.take();
    for (_, prompt) in &asked {
        let who = asked_for(prompt);
        assert!(
            who == Some(first_login.as_str()) || who == Some(second_login.as_str()),
            "a question for somebody else"
        );
    }
    eprintln!("{} questions, all for one of the two", asked.len());
}
