//! Git LFS against a real LFS server, live: a GitLab of one's own,
//! through the engine's own commands, beside git-lfs by itself on a twin.
//!
//! Opt-in and never in ci: it runs with `--ignored` when
//! `BITBRAID_GITLAB_CE` names the server (`http://127.0.0.1:8929`, on this
//! machine only) and `BITBRAID_GITLAB_CE_TOKEN_FILE` a file holding an
//! administrator's personal access token (scope `api`), which is read
//! here and sent to that server only; otherwise each test says that it
//! was skipped. It needs git-lfs, ssh and ssh-keygen, and the server's
//! SSH on port `BITBRAID_GITLAB_CE_SSH_PORT` (2224 without it): a GitLab
//! CE in Docker will do, its web port published on 127.0.0.1:8929 and its
//! SSH port on 127.0.0.1:2224, with gitlab-shell's
//! `lfs_pure_ssh_protocol` on for the test of the pure SSH transfer:
//!
//! ```sh
//! BITBRAID_GITLAB_CE=http://127.0.0.1:8929 BITBRAID_GITLAB_CE_TOKEN_FILE=<file> \
//!     cargo test -p bitbraid-cli --test suite live_lfs:: -- --ignored
//! ```
//!
//! `BITBRAID_LIVE_SCRATCH` names where the scratch repositories go (the
//! tests' scratch folder without it), and `BITBRAID_LIVE_KEEP` keeps them
//! and what was made on the server, to be looked at. Through the
//! administrator's API it makes what it needs: the users `lfs-alice`
//! and `lfs-bob` when they are missing, a token of each for this run
//! (revoked at its end), the group `lfs-group`, and a private project
//! of the test's own (`lfs-<what>-<hex>`), removed at its end with
//! what it holds. The users' passwords are made once and kept nowhere.
//!
//! Run it with a home of its own (`HOME`, `XDG_CONFIG_HOME`,
//! `GIT_CONFIG_NOSYSTEM=1`, no `SSH_AUTH_SOCK`): the engine runs git as the
//! test process has it set up. What could reach further is pinned in each
//! scratch repository: no credential helper, ssh with a configuration, a
//! key and a known-hosts file of the test's own, no agent.
//!
//! Every question git, git-lfs and ssh ask goes to the askpass server of
//! the application, through this binary run as their askpass program, as
//! it does in the application: who is asked for which password, which
//! passphrase and which host key is recorded and held to what was meant.
//! The twin, git-lfs's own commands on a clone of its own, is answered by
//! the same server, so that no token is ever written anywhere.
//!
//! What is tested:
//! - over HTTP: several LFS files pushed, fetched and pulled, one of
//!   24 MiB, saying how far they are; a push and a fetch stopped half way
//!   (through a front of the test's that holds the bytes to a rate, the
//!   loopback interface being too fast to stop anything on); what the
//!   server and the local stores end with, against the twin;
//! - over SSH: the passphrase of a key and the host's key asked through
//!   the askpass program; git-lfs finding the HTTP API with
//!   `git-lfs-authenticate`, and git-lfs's pure SSH transfer
//!   (`git-lfs-transfer`), which GitLab offers when gitlab-shell has
//!   `lfs_pure_ssh_protocol` on (it is off unless set); locks over both;
//! - locks with two users: a lock of the other's shown with its owner and
//!   time, one's own given up, a developer refused the other's even by
//!   force, a maintainer forcing it, a reporter refused a lock and still
//!   shown the list, whether locks are kept per branch, and what a push
//!   of a file locked by the other does;
//! - TLS: a front of the test's with a certificate of an authority made
//!   for the run, trusted through `http.sslCAInfo` in the scratch
//!   repository; refused, in the application's words, where it is not.
//!
//! Sources: docs.gitlab.com "Git Large File Storage (LFS)" (the
//! administration page: `lfs_pure_ssh_protocol`), "File Locking" (exclusive
//! locks with Git LFS), docs.gitlab.com/api "Users" (create, personal
//! access tokens of a user, SSH keys of a user), "Personal access tokens"
//! (revoke), "Groups", "Projects" (create, delete, `permanently_remove`),
//! "Project members"; git-lfs's `docs/api/batch.md`, `docs/api/locking.md`,
//! `docs/api/authentication.md` (`git-lfs-authenticate`) and
//! `docs/proposals/ssh_adapter.md` (`git-lfs-transfer`); git-lfs-config(5)
//! (`lfs.<url>.sshtransfer`, `lfs.<url>.locksverify`), git-lfs-push(1),
//! git-lfs-fetch(1), git-lfs-pull(1), git-lfs-lock(1), git-lfs-locks(1),
//! git-lfs-unlock(1); gitcredentials(7); git-config(1) (`http.sslCAInfo`,
//! `core.sshCommand`); ssh(1), ssh-keygen(1).
#![allow(clippy::print_stderr)]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_git::askpass::{Event, Kind, Server};
use bitbraid_integrations::{
    api::{encode, rfc3339},
    http::{Call, Client, Fetch, Limits, Method, Url},
    secrets::Secret,
};
use bitbraid_testkit::{Git, scratch_root};
use serde_json::{Value, json};
use sha2::Digest as _;

use crate::ASKPASS_SERVER as ONE;
use crate::tls_front::{Authority, Front, Passing, base64};

const GROUP: &str = "lfs-group";
const MAINTAINER: &str = "lfs-alice";
const DEVELOPER: &str = "lfs-bob";
const PASSPHRASE: &str = "a passphrase for the live LFS test";
const UP: &str = "Uploading LFS objects";
const DOWN: &str = "Downloading LFS objects";

pub(crate) fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitbraid-cli"))
}

/// Something new for this run.
pub(crate) fn fresh(what: &str) -> String {
    let seed = format!(
        "{what}-{}-{:?}",
        std::process::id(),
        std::time::SystemTime::now()
    );
    let hash = sha2::Sha256::digest(seed.as_bytes());
    hash.iter()
        .take(6)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Bytes that do not compress, the same for the same seed.
pub(crate) fn content(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::with_capacity(len + 8);
    while out.len() < len {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        out.extend_from_slice(&state.to_le_bytes());
    }
    out.truncate(len);
    out
}

/// The LFS object id of some content.
pub(crate) fn oid(bytes: &[u8]) -> String {
    sha2::Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

// ------------------------------------------------------------ the askpass

/// The askpass server of a running application, answering by `reply`, and
/// what it was asked.
pub(crate) struct Asking {
    _server: Server,
    asked: Arc<Mutex<Vec<(Kind, String)>>>,
}

impl Asking {
    pub(crate) fn with(
        reply: impl Fn(Kind, &str) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let server = Server::start(helper(), move |event| {
            if let Event::Asked(question) = event {
                log.lock()
                    .unwrap()
                    .push((question.kind, question.prompt.clone()));
                let answer = reply(question.kind, &question.prompt);
                question.answer(answer);
            }
        })
        .unwrap();
        Self {
            _server: server,
            asked,
        }
    }

    pub(crate) fn take(&self) -> Vec<(Kind, String)> {
        std::mem::take(&mut *self.asked.lock().unwrap())
    }
}

/// The user a password question is for: `Password for 'http://who@host'`.
pub(crate) fn asked_for(prompt: &str) -> Option<&str> {
    let after = prompt.split("://").nth(1)?;
    let (user, _) = after.split_once('@')?;
    Some(user)
}

/// Answers by who is asked for: a token of each user, the passphrase of
/// the test's key, yes to a host key. Anything else is declined, and
/// recorded like everything else.
pub(crate) fn answers(tokens: BTreeMap<String, Secret>) -> Asking {
    Asking::with(move |kind, prompt| {
        if prompt.contains("(yes/no") {
            return Some("yes".to_owned());
        }
        if kind == Kind::Secret && prompt.contains("passphrase") {
            return Some(PASSPHRASE.to_owned());
        }
        if prompt.starts_with("Password") {
            let user = asked_for(prompt)?;
            return tokens.get(user).map(|token| token.expose().to_owned());
        }
        None
    })
}

/// The environment that has git and git-lfs of the twin ask the same
/// server: the program, and where the server is.
fn askpass_env(cmd: &mut Command) {
    for (key, value) in bitbraid_git::askpass::child_env() {
        cmd.env(key, value);
    }
    cmd.env("GIT_ASKPASS", helper());
}

// ------------------------------------------------------------ copies

/// A scratch directory of one test, removed at its end unless
/// `BITBRAID_LIVE_KEEP` is set.
pub(crate) struct Run {
    pub(crate) dir: PathBuf,
    pub(crate) home: PathBuf,
}

impl Run {
    pub(crate) fn new(what: &str) -> Self {
        let base = std::env::var_os("BITBRAID_LIVE_SCRATCH")
            .map(PathBuf::from)
            .unwrap_or_else(scratch_root);
        let dir = base.join(format!("live-lfs-{what}-{}", fresh(what)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let home = dir.join("home");
        let run = Self { dir, home };
        Git::new(&run.dir, &run.home).ensure_home().unwrap();
        run
    }

    pub(crate) fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    pub(crate) fn git(&self, dir: &Path) -> Git {
        Git::new(dir, &self.home)
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        if std::env::var_os("BITBRAID_LIVE_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

/// What a context has reported since it was watched.
type Seen = Mutex<Vec<(String, Option<u8>)>>;

/// A repository opened in the engine.
pub(crate) struct Copy {
    pub(crate) dir: PathBuf,
    registry: Registry,
    pub(crate) ctx: CommandContext,
}

impl Copy {
    pub(crate) fn open(dir: &Path) -> Self {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": dir }))
            .unwrap_or_else(|err| panic!("open {}: {err}", dir.display()));
        Self {
            dir: dir.to_owned(),
            registry,
            ctx,
        }
    }

    /// A clone of `url` at `dir` made by the engine, pinned and opened.
    /// Without the LFS filter in the home of the test it has pointers.
    pub(crate) fn cloned(run: &Run, url: &str, dir: &Path) -> Self {
        Self::try_cloned(run, url, dir).unwrap_or_else(|err| panic!("repo.clone {url}: {err}"))
    }

    /// [`Self::cloned`], or why the clone failed.
    pub(crate) fn try_cloned(run: &Run, url: &str, dir: &Path) -> Result<Self, String> {
        let mut outside = Self {
            dir: run.dir.clone(),
            registry: builtin_registry(),
            ctx: CommandContext::default(),
        };
        outside.run("repo.clone", json!({ "url": url, "path": dir }))?;
        pin(&run.git(dir), None);
        Ok(Self::open(dir))
    }

    /// A clone of `url` at `dir` for what has to be set before git first
    /// talks to the server (ssh, an authority): a new repository with
    /// `config`, fetched by the engine, its `main` checked out (pointers,
    /// with no filter there yet).
    pub(crate) fn fetched(
        run: &Run,
        url: &str,
        dir: &Path,
        ssh: Option<&str>,
        config: &[(&str, &str)],
    ) -> Self {
        std::fs::create_dir_all(dir).unwrap();
        let git = run.git(dir);
        git.run(["init", "-q", "-b", "main"]).unwrap();
        pin(&git, ssh);
        for (key, value) in config {
            git.run(["config", key, value]).unwrap();
        }
        git.run(["remote", "add", "origin", url]).unwrap();
        let mut copy = Self::open(dir);
        copy.ok("remote.fetch", json!({ "remote": "origin" }));
        git.run(["checkout", "-q", "-B", "main", "--track", "origin/main"])
            .unwrap();
        copy
    }

    pub(crate) fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    pub(crate) fn ok(&mut self, id: &str, params: Value) -> Value {
        let shown = params.to_string();
        self.run(id, params)
            .unwrap_or_else(|err| panic!("{id} {shown} in {}: {err}", self.dir.display()))
    }

    pub(crate) fn refused(&mut self, id: &str, params: Value) -> String {
        let shown = params.to_string();
        match self.run(id, params) {
            Ok(value) => panic!("{id} {shown} was meant to fail and gave {value}"),
            Err(err) => err,
        }
    }

    /// What the commands report from now on. With `stop_in`, the command
    /// is asked to stop as soon as that stage is at a figure between
    /// nothing and everything.
    pub(crate) fn watch(&mut self, stop_in: Option<&'static str>) -> Arc<Seen> {
        let seen: Arc<Seen> = Arc::default();
        let (sink, cancel) = (seen.clone(), self.ctx.cancel.clone());
        cancel.store(false, Ordering::SeqCst);
        self.ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
            if stop_in == Some(progress.stage.as_str())
                && progress.percent.is_some_and(|at| at > 0 && at < 100)
            {
                cancel.store(true, Ordering::SeqCst);
            }
            sink.lock()
                .unwrap()
                .push((progress.stage, progress.percent));
        }));
        seen
    }

    pub(crate) fn go_on(&mut self) {
        self.ctx.cancel.store(false, Ordering::SeqCst);
    }

    /// The LFS objects in the local store.
    pub(crate) fn store(&self) -> Vec<String> {
        store(&self.dir.join(".git"))
    }
}

/// The figures a stage was at, in the order they were reported.
pub(crate) fn figures(seen: &Seen, stage: &str) -> Vec<u8> {
    seen.lock()
        .unwrap()
        .iter()
        .filter(|(reported, _)| reported == stage)
        .filter_map(|(_, percent)| *percent)
        .collect()
}

/// The object ids in an LFS store.
pub(crate) fn store(git_dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if let Some(name) = path.file_name().and_then(|name| name.to_str())
                && name.len() == 64
            {
                out.push(name.to_owned());
            }
        }
    }
    let mut out = Vec::new();
    walk(&git_dir.join("lfs/objects"), &mut out);
    out.sort();
    out
}

/// Pin what a scratch repository could reach beyond the test: no
/// credential helper, hooks inside it, and ssh with the test's own files.
pub(crate) fn pin(git: &Git, ssh: Option<&str>) {
    for (key, value) in [
        ("user.name", "BitBraid Test"),
        ("user.email", "bitbraid-test@example.invalid"),
        ("credential.helper", ""),
        ("core.hooksPath", ".git/hooks"),
        ("commit.gpgsign", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    // An askpass program of the developer's own is never started by a
    // test: git is pointed at this binary instead, which is all that can
    // be done about it here.
    if std::env::var_os("SSH_ASKPASS").is_some() {
        git.run(["config", "core.askPass", helper().to_str().unwrap()])
            .unwrap();
    }
    if let Some(ssh) = ssh {
        git.run(["config", "core.sshCommand", ssh]).unwrap();
    }
}

/// git-lfs by itself, on a clone of its own, answered by the askpass
/// server of the test. Its errors name the command and what it wrote,
/// never its environment.
pub(crate) struct Twin {
    pub(crate) git: Git,
}

impl Twin {
    pub(crate) fn run(&self, args: &[&str]) -> Result<String, String> {
        let mut cmd = self.git.command();
        askpass_env(&mut cmd);
        cmd.env("GIT_TERMINAL_PROMPT", "0").args(args);
        let out = cmd.output().map_err(|err| format!("git {args:?}: {err}"))?;
        let said = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(format!(
                "git {args:?} in {}: {said}",
                self.git.cwd().display()
            ))
        }
    }

    pub(crate) fn ok(&self, args: &[&str]) -> String {
        self.run(args).unwrap_or_else(|err| panic!("{err}"))
    }

    pub(crate) fn store(&self) -> Vec<String> {
        store(&self.git.cwd().join(".git"))
    }

    /// Clone `url` into `dir` with git-lfs set up there, its content
    /// left as pointers until asked for.
    pub(crate) fn clone(run: &Run, url: &str, dir: &Path, extra: &[(&str, &str)]) -> Self {
        let mut args: Vec<String> = vec!["clone".into(), "-q".into()];
        for (key, value) in [("credential.helper", ""), ("core.hooksPath", ".git/hooks")]
            .iter()
            .chain(extra)
        {
            args.push("-c".into());
            args.push(format!("{key}={value}"));
        }
        args.push(url.into());
        args.push(dir.display().to_string());
        let outside = Self {
            git: run.git(&run.dir),
        };
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        // Without the filter in the twin's home, the clone has pointers.
        outside.ok(&refs);
        let twin = Self { git: run.git(dir) };
        twin.ok(&["lfs", "install", "--local"]);
        twin
    }
}

/// Write `bytes` at `rel` under `dir`.
pub(crate) fn write(dir: &Path, rel: &str, bytes: &[u8]) {
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

/// The files of a worktree that are compared, as they are there.
pub(crate) fn same_files(left: &Path, right: &Path, files: &[&str]) {
    for file in files {
        let read = |dir: &Path| std::fs::read(dir.join(file)).ok();
        let (l, r) = (read(left), read(right));
        assert!(
            l == r,
            "{file} differs: {} bytes in {} and {} in {}",
            l.as_ref().map_or(0, Vec::len),
            left.display(),
            r.as_ref().map_or(0, Vec::len),
            right.display()
        );
    }
}

pub(crate) fn is_pointer(dir: &Path, file: &str) -> bool {
    std::fs::read(dir.join(file))
        .is_ok_and(|bytes| bytes.starts_with(b"version https://git-lfs.github.com/spec/v1"))
}

// ------------------------------------------------------------ the server

/// The administrator's API of the server, or a user's.
pub(crate) struct Api {
    client: Client,
    base: String,
    token: Secret,
}

fn limits() -> Limits {
    Limits {
        connect: Duration::from_secs(10),
        total: Duration::from_secs(120),
        body: 16 << 20,
    }
}

impl Api {
    fn call(&self, method: Method, path: &str, body: Option<Value>) -> (u16, Value) {
        let url = Url::parse(&format!("{}/api/v4{path}", self.base)).unwrap();
        let mut call = Call::new(method, url, limits())
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

/// A user of the test, with a token of this run.
pub(crate) struct Person {
    id: u64,
    pub(crate) username: String,
    pub(crate) token: Secret,
    token_id: u64,
}

/// The server, the administrator, and what one test made there.
pub(crate) struct Site {
    admin: Api,
    base: String,
    group: u64,
    people: Vec<Person>,
    projects: Vec<(u64, String)>,
    keys: Vec<(u64, u64)>,
}

impl Site {
    /// The site, when the live tests are asked for; `None` with a
    /// message otherwise.
    fn of_this_run(test: &str) -> Option<Self> {
        let asked = || -> Option<(String, Secret)> {
            let base = std::env::var("BITBRAID_GITLAB_CE").ok()?;
            let file = std::env::var("BITBRAID_GITLAB_CE_TOKEN_FILE").ok()?;
            let token = std::fs::read_to_string(file).ok()?.trim().to_owned();
            let base = base.trim().trim_end_matches('/').to_owned();
            let url = Url::parse(&base).ok()?;
            // Only a server on this machine.
            let local = url.host() == "localhost"
                || url
                    .host()
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback());
            local.then(|| (base, Secret::new(token)))
        };
        let Some((base, token)) = asked() else {
            eprintln!(
                "skipped {test}: set BITBRAID_GITLAB_CE to a GitLab on this machine and \
                 BITBRAID_GITLAB_CE_TOKEN_FILE to a file with an administrator's token"
            );
            return None;
        };
        if Command::new("git")
            .args(["lfs", "version"])
            .output()
            .map_or(true, |out| !out.status.success())
        {
            eprintln!("skipped {test}: git-lfs is not installed");
            return None;
        }
        let admin = Api {
            client: Client::new("BitBraid-Test/live-lfs"),
            base: base.clone(),
            token,
        };
        let (status, group) = admin.call(Method::Get, &format!("/groups/{GROUP}"), None);
        let group = if status == 200 {
            group["id"].as_u64().unwrap()
        } else {
            admin.ok(
                Method::Post,
                "/groups",
                Some(json!({ "name": GROUP, "path": GROUP, "visibility": "private" })),
            )["id"]
                .as_u64()
                .unwrap()
        };
        Some(Self {
            admin,
            base,
            group,
            people: Vec::new(),
            projects: Vec::new(),
            keys: Vec::new(),
        })
    }

    /// A user of the test, made when missing, with a new token.
    fn person(&mut self, username: &str) -> &Person {
        let (_, found) = self
            .admin
            .call(Method::Get, &format!("/users?username={username}"), None);
        let id = match found.as_array().and_then(|users| users.first()) {
            Some(user) => user["id"].as_u64().unwrap(),
            None => self.admin.ok(
                Method::Post,
                "/users",
                Some(json!({ "username": username, "name": username,
                             "email": format!("{username}@example.invalid"),
                             "password": format!("Bb-{}-{}-p9", fresh(username), fresh("pw")),
                             "skip_confirmation": true })),
            )["id"]
                .as_u64()
                .unwrap(),
        };
        let expires = rfc3339(bitbraid_core::clock::unix_seconds() + 2 * 86_400);
        let made = self.admin.ok(
            Method::Post,
            &format!("/users/{id}/personal_access_tokens"),
            Some(json!({ "name": format!("lfs-{}", fresh("token")),
                         "scopes": ["api", "read_repository", "write_repository"],
                         "expires_at": &expires[..10] })),
        );
        self.people.push(Person {
            id,
            username: username.to_owned(),
            token: Secret::new(made["token"].as_str().unwrap()),
            token_id: made["id"].as_u64().unwrap(),
        });
        self.people.last().unwrap()
    }

    fn who(&self, username: &str) -> &Person {
        self.people
            .iter()
            .find(|person| person.username == username)
            .unwrap()
    }

    fn tokens(&self) -> BTreeMap<String, Secret> {
        self.people
            .iter()
            .map(|person| (person.username.clone(), person.token.clone()))
            .collect()
    }

    /// A private project of this test's, with LFS on, and its members.
    fn project(&mut self, what: &str, members: &[(&str, u32)]) -> Project {
        let path = format!("lfs-{what}-{}", fresh(what));
        let made = self.admin.ok(
            Method::Post,
            "/projects",
            Some(
                json!({ "name": path, "path": path, "namespace_id": self.group,
                         "visibility": "private", "lfs_enabled": true,
                         "initialize_with_readme": false }),
            ),
        );
        let id = made["id"].as_u64().unwrap();
        self.projects.push((id, path.clone()));
        for (user, level) in members {
            let user = self.who(user).id;
            self.admin.ok(
                Method::Post,
                &format!("/projects/{id}/members"),
                Some(json!({ "user_id": user, "access_level": level })),
            );
        }
        Project {
            id,
            path: format!("{GROUP}/{path}"),
        }
    }

    /// Give a member another role, and wait until the server holds to it.
    fn set_role(&self, project: &Project, user: &str, level: u32, applied: impl Fn() -> bool) {
        let person = self.who(user);
        self.admin.ok(
            Method::Put,
            &format!("/projects/{}/members/{}", project.id, person.id),
            Some(json!({ "access_level": level })),
        );
        // What a member may do is worked out again after the change, by a
        // job of the server's: until then the old role holds.
        let started = Instant::now();
        while !applied() {
            assert!(
                started.elapsed() < Duration::from_secs(90),
                "the new role of {user} was not applied"
            );
            std::thread::sleep(Duration::from_secs(1));
        }
    }

    /// Upload a public key for a user; removed at the end.
    fn add_key(&mut self, user: &str, key: &str) {
        let id = self.who(user).id;
        let made = self.admin.ok(
            Method::Post,
            &format!("/users/{id}/keys"),
            Some(json!({ "title": format!("lfs-{}", fresh("key")), "key": key.trim() })),
        );
        self.keys.push((id, made["id"].as_u64().unwrap()));
    }

    /// `http://user@host:port/group/project.git`.
    fn url(&self, project: &Project, user: &str) -> String {
        let base = self.base.replacen("://", &format!("://{user}@"), 1);
        format!("{base}/{}.git", project.path)
    }

    fn upstream(&self) -> std::net::SocketAddr {
        let url = Url::parse(&self.base).unwrap();
        format!("{}:{}", url.host(), url.port()).parse().unwrap()
    }

    /// Whether the server has an object, asked through the batch API as
    /// `user` for a download.
    fn has(&self, project: &Project, user: &str, bytes: &[u8]) -> bool {
        let person = self.who(user);
        let url = Url::parse(&format!(
            "{}/{}.git/info/lfs/objects/batch",
            self.base, project.path
        ))
        .unwrap();
        let basic = base64(format!("{user}:{}", person.token.expose()).as_bytes());
        let body = json!({ "operation": "download", "transfers": ["basic"],
                           "objects": [{ "oid": oid(bytes), "size": bytes.len() }] });
        let call = Call::new(Method::Post, url, limits())
            .with_header("Accept", "application/vnd.git-lfs+json")
            .with_header("Authorization", &format!("Basic {basic}"))
            .with_body(
                "application/vnd.git-lfs+json",
                body.to_string().into_bytes(),
            );
        let response = self
            .admin
            .client
            .send(&call, &AtomicBool::new(false))
            .unwrap();
        assert_eq!(response.status, 200, "batch: {}", response.status);
        let value: Value = serde_json::from_slice(&response.body).unwrap();
        value["objects"][0]["actions"]["download"].is_object()
    }

    /// Whether `user` may verify locks, which takes the right to push.
    fn may_push(&self, project: &Project, user: &str) -> bool {
        let person = self.who(user);
        let url = Url::parse(&format!(
            "{}/{}.git/info/lfs/locks/verify",
            self.base, project.path
        ))
        .unwrap();
        let basic = base64(format!("{user}:{}", person.token.expose()).as_bytes());
        let call = Call::new(Method::Post, url, limits())
            .with_header("Accept", "application/vnd.git-lfs+json")
            .with_header("Authorization", &format!("Basic {basic}"))
            .with_body("application/vnd.git-lfs+json", b"{}".to_vec());
        let response = self
            .admin
            .client
            .send(&call, &AtomicBool::new(false))
            .unwrap();
        response.status == 200
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        if std::env::var_os("BITBRAID_LIVE_KEEP").is_some() {
            return;
        }
        for (user, key) in &self.keys {
            let _ = self
                .admin
                .call(Method::Delete, &format!("/users/{user}/keys/{key}"), None);
        }
        for (id, _) in &self.projects {
            // Marked for removal first; then removed for good, under the
            // name the server gave it when it was marked.
            let _ = self
                .admin
                .call(Method::Delete, &format!("/projects/{id}"), None);
            let (status, project) = self
                .admin
                .call(Method::Get, &format!("/projects/{id}"), None);
            if status == 200
                && let Some(full) = project["path_with_namespace"].as_str()
            {
                let (status, said) = self.admin.call(
                    Method::Delete,
                    &format!(
                        "/projects/{id}?permanently_remove=true&full_path={}",
                        encode(full)
                    ),
                    None,
                );
                if !(200..300).contains(&status) {
                    eprintln!("project {id} left marked for removal: {status} {said}");
                }
            }
        }
        for person in &self.people {
            let _ = self.admin.call(
                Method::Delete,
                &format!("/personal_access_tokens/{}", person.token_id),
                None,
            );
        }
    }
}

/// A project of the test's.
pub(crate) struct Project {
    id: u64,
    path: String,
}

/// A new repository at `dir` with LFS set up by the engine, tracking
/// `*.bin` (lockable when asked), and these files committed.
pub(crate) fn started(run: &Run, dir: &Path, lockable: bool, files: &[(&str, &[u8])]) -> Copy {
    let git = run.git(dir);
    std::fs::create_dir_all(dir).unwrap();
    git.run(["init", "-q", "-b", "main"]).unwrap();
    pin(&git, None);
    let mut copy = Copy::open(dir);
    copy.ok("lfs.install", json!({}));
    copy.ok(
        "lfs.track",
        json!({ "pattern": "*.bin", "lockable": lockable }),
    );
    std::fs::write(
        dir.join("readme.txt"),
        "a repository of the live LFS test\n",
    )
    .unwrap();
    for (file, bytes) in files {
        write(dir, file, bytes);
    }
    git.run(["add", "-A"]).unwrap();
    git.run(["commit", "-q", "-m", "LFS files"]).unwrap();
    copy
}

/// The questions that were asked, as lines, for a message.
fn shown(asked: &[(Kind, String)]) -> String {
    asked
        .iter()
        .map(|(kind, prompt)| format!("{kind:?}: {prompt}"))
        .collect::<Vec<_>>()
        .join("\n")
}

// ------------------------------------------------------------ the tests

#[test]
#[ignore = "live: a GitLab on this machine (BITBRAID_GITLAB_CE)"]
fn gitlab_transfers_over_http_say_how_far_they_are_and_stop_when_asked() {
    let test = "gitlab_transfers_over_http_say_how_far_they_are_and_stop_when_asked";
    let Some(mut site) = Site::of_this_run(test) else {
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    site.person(MAINTAINER);
    let project = site.project("http", &[(MAINTAINER, 40)]);
    let run = Run::new("http");
    let asking = answers(site.tokens());

    let small = content(1, 1024);
    let two = content(2, 300 * 1024);
    let mid = content(3, 3 << 20);
    let big = content(4, 24 << 20);
    let files: [(&str, &[u8]); 4] = [
        ("small.bin", &small),
        ("sub/two.bin", &two),
        ("mid.bin", &mid),
        ("big.bin", &big),
    ];
    let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    let ours = run.path("ours");
    let mut copy = started(&run, &ours, false, &files);
    let direct = site.url(&project, MAINTAINER);

    // Stopped half way: through a front that holds the bytes to 2 MiB a
    // second, so that the 24 MiB take twelve.
    let slow = Front::start(
        None,
        Passing {
            upstream: site.upstream(),
            rewrite: Some(site.base.clone()),
            rate: Some(2 << 20),
        },
    );
    let through = direct.replacen(&site.base[7..], &slow.origin()[7..], 1);
    run.git(&ours)
        .run(["remote", "add", "origin", &through])
        .unwrap();
    let seen = copy.watch(Some(UP));
    let began = Instant::now();
    let err = copy.refused("lfs.push", json!({}));
    let took = began.elapsed();
    assert!(err.contains("cancelled"), "{err}");
    assert!(took < Duration::from_secs(20), "{took:?}");
    let at = figures(&seen, UP);
    assert!(
        at.iter().any(|at| *at > 0 && *at < 100),
        "no figure in between: {:?}",
        seen.lock().unwrap()
    );
    assert!(at.is_sorted(), "{at:?}");
    assert!(!site.has(&project, MAINTAINER, &big), "big.bin arrived");
    assert!(
        slow.heard()
            .iter()
            .any(|line| line.contains("/gitlab-lfs/objects/")),
        "the upload did not go through the front: {:?}",
        slow.heard()
    );
    eprintln!("push stopped after {took:?} at {at:?}");

    // To the end, at full speed, straight to the server.
    run.git(&ours)
        .run(["remote", "set-url", "origin", &direct])
        .unwrap();
    copy.go_on();
    let seen = copy.watch(None);
    let began = Instant::now();
    let pushed = copy.ok("lfs.push", json!({}));
    eprintln!("push of 27 MiB took {:?}", began.elapsed());
    let at = figures(&seen, UP);
    assert!(at.is_sorted(), "{at:?}");
    assert_eq!(at.last(), Some(&100), "{:?} {pushed}", seen.lock().unwrap());
    for (name, bytes) in &files {
        assert!(site.has(&project, MAINTAINER, bytes), "{name} is not there");
    }
    // The commits, with nothing left for the hook to upload.
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    // Every question was for the password of the user in the address,
    // and went through the askpass program; nobody was asked who they
    // are.
    let asked = asking.take();
    assert!(!asked.is_empty(), "nothing was asked");
    for (kind, prompt) in &asked {
        assert_eq!(*kind, Kind::Secret, "{}", shown(&asked));
        assert_eq!(asked_for(prompt), Some(MAINTAINER), "{}", shown(&asked));
    }

    // A clone of the engine's, and git-lfs's twin of it.
    let theirs = run.path("clone");
    let mut clone = Copy::cloned(&run, &direct, &theirs);
    assert!(is_pointer(&theirs, "big.bin"));
    clone.ok("lfs.install", json!({}));
    let twin = Twin::clone(&run, &direct, &run.path("twin"), &[]);

    // A fetch stopped half way, through the slow front.
    run.git(&theirs)
        .run(["remote", "set-url", "origin", &through])
        .unwrap();
    let seen = clone.watch(Some(DOWN));
    let err = clone.refused("lfs.fetch", json!({}));
    assert!(err.contains("cancelled"), "{err}");
    let at = figures(&seen, DOWN);
    assert!(
        at.iter().any(|at| *at > 0 && *at < 100),
        "no figure in between: {:?}",
        seen.lock().unwrap()
    );
    assert!(!clone.store().contains(&oid(&big)), "big.bin arrived");
    run.git(&theirs)
        .run(["remote", "set-url", "origin", &direct])
        .unwrap();

    // To the end: what git-lfs by itself fetches.
    clone.go_on();
    let seen = clone.watch(None);
    clone.ok("lfs.fetch", json!({}));
    twin.ok(&["lfs", "fetch", "origin"]);
    assert_eq!(figures(&seen, DOWN).last(), Some(&100));
    assert_eq!(clone.store(), twin.store());
    assert_eq!(clone.store().len(), 4);
    assert!(is_pointer(&theirs, "big.bin"));
    // Pulled: in place of the pointers, as git-lfs puts them.
    clone.ok("lfs.pull", json!({}));
    twin.ok(&["lfs", "pull", "origin"]);
    same_files(&theirs, twin.git.cwd(), &names);
    same_files(&theirs, &ours, &names);
    assert_eq!(
        run.git(&theirs).lines(["status", "--porcelain"]).unwrap(),
        Vec::<String>::new()
    );

    // Something new from the twin comes in with a pull of commits, the
    // filter downloading it as the files are checked out.
    let fresh_file = content(5, 500 * 1024);
    write(twin.git.cwd(), "new.bin", &fresh_file);
    twin.ok(&["add", "new.bin"]);
    twin.ok(&["commit", "-q", "-m", "a new LFS file"]);
    twin.ok(&["push", "-q", "origin", "main"]);
    clone.ok("remote.pull", json!({ "mode": "ff_only" }));
    same_files(&theirs, twin.git.cwd(), &["new.bin"]);
    assert!(clone.store().contains(&oid(&fresh_file)));
    let listed = clone.ok("lfs.files", json!({}));
    assert!(
        listed["files"]
            .as_array()
            .unwrap()
            .iter()
            .all(|file| file["present"] == true && file["checked_out"] == true),
        "{listed}"
    );
    // Everything of every reference.
    clone.ok("lfs.fetch", json!({ "all": true }));
    let asked = asking.take();
    eprintln!("asked:\n{}", shown(&asked));
    for (kind, prompt) in &asked {
        assert_eq!(*kind, Kind::Secret, "{}", shown(&asked));
        assert_eq!(asked_for(prompt), Some(MAINTAINER), "{}", shown(&asked));
    }

    // The question declined, and a wrong token given: nothing comes, and
    // the error says which.
    drop(asking);
    std::fs::remove_dir_all(theirs.join(".git/lfs/objects")).unwrap();
    let declining = Asking::with(|_, _| None);
    for (id, params) in [
        ("lfs.fetch", json!({})),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "small.bin" })),
    ] {
        let err = clone.refused(id, params);
        eprintln!("{id}, the question declined: {err}");
        assert!(err.contains("was not answered"), "{id}: {err}");
    }
    assert!(!declining.take().is_empty());
    drop(declining);
    let wrong = Asking::with(|kind, _| (kind == Kind::Secret).then(|| "not-the-token".to_owned()));
    for (id, params) in [
        ("lfs.fetch", json!({})),
        ("lfs.locks", json!({})),
        ("lfs.lock", json!({ "path": "small.bin" })),
    ] {
        let err = clone.refused(id, params);
        eprintln!("{id}, a wrong token: {err}");
        assert!(
            err.contains("refused the user name and the password"),
            "{id}: {err}"
        );
    }
    assert!(!wrong.take().is_empty());
    assert!(clone.store().is_empty());
}

/// A key with a passphrase, made for the run, and the ssh that uses it
/// and nothing else of the developer's.
fn ssh_key(run: &Run) -> (String, String) {
    let key = run.path("key");
    let made = Command::new("ssh-keygen")
        .args([
            "-q", "-t", "ed25519", "-N", PASSPHRASE, "-C", "lfs-live", "-f",
        ])
        .arg(&key)
        .output()
        .unwrap();
    assert!(made.status.success(), "ssh-keygen: {made:?}");
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();
    let ssh = format!(
        "ssh -F /dev/null -o IdentityAgent=none -o IdentitiesOnly=yes -i {} \
         -o UserKnownHostsFile={} -o GlobalKnownHostsFile=/dev/null -o BatchMode=no",
        key.display(),
        run.path("known_hosts").display()
    );
    (public, ssh)
}

#[test]
#[ignore = "live: a GitLab on this machine (BITBRAID_GITLAB_CE)"]
fn gitlab_over_ssh_asks_through_the_askpass_and_moves_content_both_ways() {
    let test = "gitlab_over_ssh_asks_through_the_askpass_and_moves_content_both_ways";
    let Some(mut site) = Site::of_this_run(test) else {
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    site.person(MAINTAINER);
    let project = site.project("ssh", &[(MAINTAINER, 40)]);
    let run = Run::new("ssh");
    let (public, ssh) = ssh_key(&run);
    site.add_key(MAINTAINER, &public);
    let asking = answers(site.tokens());
    let ssh_port = std::env::var("BITBRAID_GITLAB_CE_SSH_PORT").unwrap_or_else(|_| "2224".into());
    let url = format!(
        "ssh://git@{}:{ssh_port}/{}.git",
        site.upstream().ip(),
        project.path
    );
    let one = content(31, 200 * 1024);
    let two = content(32, 2 << 20);
    let ours = run.path("ours");
    let mut copy = started(&run, &ours, true, &[("one.bin", &one), ("two.bin", &two)]);
    let git = run.git(&ours);
    git.run(["config", "core.sshCommand", &ssh]).unwrap();
    git.run(["remote", "add", "origin", &url]).unwrap();

    // By git-lfs-authenticate: ssh says who, and git-lfs gets an address
    // and a header for the HTTP API from the server.
    git.run(["config", "lfs.sshtransfer", "never"]).unwrap();
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    assert!(site.has(&project, MAINTAINER, &one));
    assert!(site.has(&project, MAINTAINER, &two));
    let asked = asking.take();
    eprintln!("asked over ssh:\n{}", shown(&asked));
    assert!(
        asked
            .iter()
            .any(|(kind, prompt)| *kind == Kind::Text && prompt.contains("(yes/no")),
        "the host's key was not asked about: {}",
        shown(&asked)
    );
    assert!(
        asked
            .iter()
            .any(|(kind, prompt)| *kind == Kind::Secret && prompt.contains("passphrase")),
        "{}",
        shown(&asked)
    );
    assert!(
        asked
            .iter()
            .all(|(_, prompt)| !prompt.starts_with("Password")),
        "a password was asked over ssh: {}",
        shown(&asked)
    );
    let made = copy.ok("lfs.lock", json!({ "path": "one.bin" }));
    assert_eq!(
        (made["owner"].as_str(), made["ours"].as_bool()),
        (Some(MAINTAINER), Some(true))
    );
    assert_eq!(listed(&copy.ok("lfs.locks", json!({}))).len(), 1);
    copy.ok("lfs.unlock", json!({ "path": "one.bin" }));

    let theirs = run.path("clone");
    let mut clone = Copy::fetched(
        &run,
        &url,
        &theirs,
        Some(&ssh),
        &[("lfs.sshtransfer", "never")],
    );
    clone.ok("lfs.install", json!({}));
    let seen = clone.watch(None);
    clone.ok("lfs.fetch", json!({}));
    assert_eq!(
        figures(&seen, DOWN).last(),
        Some(&100),
        "{:?}",
        seen.lock().unwrap()
    );
    clone.ok("lfs.pull", json!({}));
    same_files(&theirs, &ours, &["one.bin", "two.bin"]);

    // git-lfs's own transfer over ssh, which only this server's setting
    // turns on: with `always` git-lfs takes nothing else, so that it
    // working is the proof.
    git.run(["config", "lfs.sshtransfer", "always"]).unwrap();
    let three = content(33, 3 << 20);
    write(&ours, "three.bin", &three);
    git.run(["add", "three.bin"]).unwrap();
    git.run(["commit", "-q", "-m", "a third"]).unwrap();
    let seen = copy.watch(None);
    match copy.run("lfs.push", json!({})) {
        Ok(_) => {
            assert_eq!(
                figures(&seen, UP).last(),
                Some(&100),
                "{:?}",
                seen.lock().unwrap()
            );
            assert!(site.has(&project, MAINTAINER, &three));
            copy.ok(
                "remote.push",
                json!({ "remote": "origin", "branch": "main" }),
            );
            let made = copy.ok("lfs.lock", json!({ "path": "two.bin" }));
            assert_eq!(made["ours"], true);
            assert_eq!(
                listed(&copy.ok("lfs.locks", json!({}))),
                vec![("two.bin".to_owned(), MAINTAINER.to_owned(), Some(true))]
            );
            copy.ok("lfs.unlock", json!({ "path": "two.bin" }));
            run.git(&theirs)
                .run(["config", "lfs.sshtransfer", "always"])
                .unwrap();
            clone.ok("remote.pull", json!({ "mode": "ff_only" }));
            same_files(&theirs, &ours, &["three.bin"]);
            eprintln!("pure SSH transfer: offered and used");
        }
        Err(err) => {
            eprintln!("pure SSH transfer: not offered by this server: {err}");
            assert!(
                err.contains("git-lfs-transfer") || err.contains("Disallowed"),
                "{err}"
            );
        }
    }
    let asked = asking.take();
    eprintln!("asked later:\n{}", shown(&asked));
    assert!(
        asked
            .iter()
            .all(|(_, prompt)| !prompt.starts_with("Password")),
        "{}",
        shown(&asked)
    );
}

#[test]
#[ignore = "live: a GitLab on this machine (BITBRAID_GITLAB_CE)"]
fn gitlab_over_tls_is_trusted_through_the_repository_and_refused_otherwise() {
    let test = "gitlab_over_tls_is_trusted_through_the_repository_and_refused_otherwise";
    let Some(mut site) = Site::of_this_run(test) else {
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    site.person(MAINTAINER);
    let project = site.project("tls", &[(MAINTAINER, 40)]);
    let run = Run::new("tls");
    let asking = answers(site.tokens());
    let authority = Authority::new("BitBraid live LFS test authority");
    let ca = run.path("authority.pem");
    std::fs::write(&ca, authority.pem()).unwrap();
    let front = Front::start(
        Some(authority.server()),
        Passing {
            upstream: site.upstream(),
            rewrite: Some(site.base.clone()),
            rate: None,
        },
    );
    let url = format!(
        "https://{MAINTAINER}@{}/{}.git",
        front.address(),
        project.path
    );
    let one = content(41, 100 * 1024);
    let two = content(42, 1 << 20);
    let ours = run.path("ours");
    let mut copy = started(&run, &ours, true, &[("one.bin", &one), ("two.bin", &two)]);
    let git = run.git(&ours);
    git.run(["config", "http.sslCAInfo", ca.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", &url]).unwrap();

    // Trusted: the commits, the content and the locks, all through the
    // front.
    copy.ok("lfs.push", json!({}));
    copy.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    assert!(site.has(&project, MAINTAINER, &one));
    assert!(site.has(&project, MAINTAINER, &two));
    let made = copy.ok("lfs.lock", json!({ "path": "one.bin" }));
    assert_eq!(made["ours"], true);
    assert_eq!(listed(&copy.ok("lfs.locks", json!({}))).len(), 1);
    copy.ok("lfs.unlock", json!({ "path": "one.bin" }));
    let theirs = run.path("clone");
    let mut clone = Copy::fetched(
        &run,
        &url,
        &theirs,
        None,
        &[("http.sslCAInfo", ca.to_str().unwrap())],
    );
    clone.ok("lfs.install", json!({}));
    clone.ok("lfs.fetch", json!({}));
    clone.ok("lfs.pull", json!({}));
    same_files(&theirs, &ours, &["one.bin", "two.bin"]);
    let heard = front.heard();
    for wanted in [
        "/info/lfs/objects/batch",
        "/gitlab-lfs/objects/",
        "/info/lfs/locks",
        "git-receive-pack",
        "git-upload-pack",
    ] {
        assert!(
            heard.iter().any(|line| line.contains(wanted)),
            "{wanted} did not go through the front: {heard:?}"
        );
    }
    assert_eq!(front.handshakes_failed(), 0);
    eprintln!("asked over tls:\n{}", shown(&asking.take()));

    // Not trusted: the system's authorities alone, and then another
    // authority than the one that signed. Refused, and nothing is sent.
    let other = Authority::new("Another authority");
    let other_ca = run.path("other.pem");
    std::fs::write(&other_ca, other.pem()).unwrap();
    // What would be fetched has to be fetched.
    std::fs::remove_dir_all(theirs.join(".git/lfs/objects")).unwrap();
    for setting in [None, Some(other_ca.to_str().unwrap())] {
        let cgit = run.git(&theirs);
        match setting {
            None => cgit.run(["config", "--unset", "http.sslCAInfo"]).unwrap(),
            Some(file) => cgit.run(["config", "http.sslCAInfo", file]).unwrap(),
        };
        let asked_before = front.heard().len();
        for (id, params) in [
            ("lfs.fetch", json!({ "all": true })),
            ("lfs.push", json!({ "all": true })),
            ("lfs.locks", json!({})),
            ("lfs.lock", json!({ "path": "two.bin" })),
            ("lfs.unlock", json!({ "path": "two.bin" })),
            ("remote.fetch", json!({ "remote": "origin" })),
        ] {
            let err = clone.refused(id, params);
            eprintln!("{id} untrusted ({setting:?}): {err}");
            assert!(
                err.contains(
                    "The server's certificate was not accepted, so nothing was sent to it"
                ),
                "{id}: {err}"
            );
            assert!(err.contains("http.sslCAInfo"), "{id}: {err}");
        }
        assert_eq!(front.heard().len(), asked_before, "something was asked");
        assert!(front.handshakes_failed() > 0);
    }
    // git-lfs asks for the password before it connects where it has
    // noted that the server wants one (`lfs.<url>.access`); the handshake
    // then fails before anything is sent. Git asks nothing.
    let asked = asking.take();
    eprintln!("asked of the untrusted server:\n{}", shown(&asked));
    assert!(
        asked
            .iter()
            .all(|(_, prompt)| prompt.starts_with("Password for \"")),
        "{}",
        shown(&asked)
    );
}

/// What `lfs.locks` lists, as (path, owner, ours) by path.
fn listed(locks: &Value) -> Vec<(String, String, Option<bool>)> {
    let mut out: Vec<(String, String, Option<bool>)> = locks
        .as_array()
        .unwrap()
        .iter()
        .map(|lock| {
            (
                lock["path"].as_str().unwrap().to_owned(),
                lock["owner"].as_str().unwrap_or_default().to_owned(),
                lock["ours"].as_bool(),
            )
        })
        .collect();
    out.sort();
    out
}

#[test]
#[ignore = "live: a GitLab on this machine (BITBRAID_GITLAB_CE)"]
fn gitlab_locks_between_two_users_hold_to_their_roles() {
    let test = "gitlab_locks_between_two_users_hold_to_their_roles";
    let Some(mut site) = Site::of_this_run(test) else {
        return;
    };
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    site.person(MAINTAINER);
    site.person(DEVELOPER);
    let project = site.project("locks", &[(MAINTAINER, 40), (DEVELOPER, 30)]);
    let run = Run::new("locks");
    let asking = answers(site.tokens());
    let files: [(&str, &[u8]); 4] = [
        ("a.bin", &content(11, 4096)),
        ("b.bin", &content(12, 4096)),
        ("c.bin", &content(13, 4096)),
        ("d.bin", &content(14, 4096)),
    ];
    let alice_dir = run.path("alice");
    let mut alice = started(&run, &alice_dir, true, &files);
    run.git(&alice_dir)
        .run(["remote", "add", "origin", &site.url(&project, MAINTAINER)])
        .unwrap();
    alice.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "set_upstream": true }),
    );
    let bob_dir = run.path("bob");
    let mut bob = Copy::cloned(&run, &site.url(&project, DEVELOPER), &bob_dir);
    bob.ok("lfs.install", json!({}));
    bob.ok("lfs.pull", json!({}));

    // Nothing is locked yet.
    assert_eq!(listed(&alice.ok("lfs.locks", json!({}))), vec![]);

    // Alice locks a.bin: it is hers, with a time.
    let made = alice.ok("lfs.lock", json!({ "path": "a.bin" }));
    assert_eq!(made["path"], "a.bin");
    assert_eq!(made["owner"], MAINTAINER);
    assert_eq!(made["ours"], true);
    let since = made["locked_at"].as_str().unwrap().to_owned();
    assert!(since.ends_with('Z') && since.len() >= 20, "{made}");

    // Bob sees it as hers, since then.
    let seen = bob.ok("lfs.locks", json!({}));
    assert_eq!(
        listed(&seen),
        vec![("a.bin".to_owned(), MAINTAINER.to_owned(), Some(false))]
    );
    assert_eq!(seen[0]["locked_at"], since.as_str());
    // He cannot lock it again: who holds it and since when.
    let err = bob.refused("lfs.lock", json!({ "path": "a.bin" }));
    assert!(
        err.contains(&format!(
            "a.bin is already locked by {MAINTAINER} since {since}"
        )),
        "{err}"
    );
    // Nor give it up, without force or with it: a developer may not
    // force.
    let err = bob.refused("lfs.unlock", json!({ "path": "a.bin" }));
    assert!(err.contains("a.bin was not unlocked"), "{err}");
    assert!(err.contains(MAINTAINER), "{err}");
    let err = bob.refused("lfs.unlock", json!({ "path": "a.bin", "force": true }));
    assert!(err.contains("a.bin was not unlocked"), "{err}");
    assert!(err.contains("maintainer"), "{err}");
    eprintln!("a developer forcing: {err}");

    // His own lock he gives up.
    let his = bob.ok("lfs.lock", json!({ "path": "b.bin" }));
    assert_eq!(
        (his["owner"].as_str(), his["ours"].as_bool()),
        (Some(DEVELOPER), Some(true))
    );
    let gone = bob.ok("lfs.unlock", json!({ "path": "b.bin" }));
    assert_eq!(gone["unlocked"], true);
    let err = bob.refused("lfs.unlock", json!({ "path": "b.bin" }));
    assert!(err.contains("b.bin is not locked"), "{err}");

    // Alice, a maintainer, may give up his by force, and only by force.
    bob.ok("lfs.lock", json!({ "path": "c.bin" }));
    let err = alice.refused("lfs.unlock", json!({ "path": "c.bin" }));
    assert!(err.contains("c.bin was not unlocked"), "{err}");
    assert!(err.contains(DEVELOPER), "{err}");
    let forced = alice.ok("lfs.unlock", json!({ "path": "c.bin", "force": true }));
    assert_eq!(forced["unlocked"], true);
    assert_eq!(
        listed(&bob.ok("lfs.locks", json!({}))),
        vec![("a.bin".to_owned(), MAINTAINER.to_owned(), Some(false))]
    );

    // Per branch? git-lfs sends the branch it is on with a lock and with a
    // listing. A lock taken on another branch is seen from main and keeps
    // the file from being locked there: GitLab keeps locks for the whole
    // project.
    let alice_git = run.git(&alice_dir);
    alice_git.run(["switch", "-q", "-c", "topic"]).unwrap();
    alice.ok(
        "remote.push",
        json!({ "remote": "origin", "branch": "topic", "set_upstream": true }),
    );
    let on_topic = alice.ok("lfs.lock", json!({ "path": "d.bin" }));
    assert_eq!(on_topic["ours"], true);
    let from_main = listed(&bob.ok("lfs.locks", json!({})));
    assert_eq!(
        from_main,
        vec![
            ("a.bin".to_owned(), MAINTAINER.to_owned(), Some(false)),
            ("d.bin".to_owned(), MAINTAINER.to_owned(), Some(false)),
        ]
    );
    let err = bob.refused("lfs.lock", json!({ "path": "d.bin" }));
    assert!(err.contains("d.bin is already locked by"), "{err}");
    alice_git.run(["switch", "-q", "main"]).unwrap();
    // And from main, the lock taken on topic is hers to give up.
    alice.ok("lfs.unlock", json!({ "path": "d.bin" }));

    // A push that changes a file somebody else holds. git-lfs warns and
    // pushes; GitLab refuses it, on any branch.
    let bob_git = run.git(&bob_dir);
    std::fs::write(bob_dir.join("a.bin"), content(21, 4096)).unwrap();
    bob_git
        .run(["commit", "-q", "-am", "change a.bin"])
        .unwrap();
    let err = bob.refused(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "to": "bobs-change" }),
    );
    eprintln!("a push of a file somebody else holds:\n{err}");
    assert!(
        err.contains(
            "The server refused the push: it changes a file that somebody else has locked"
        ),
        "{err}"
    );
    assert!(
        err.contains(&format!("'a.bin' is locked in Git LFS by @{MAINTAINER}")),
        "{err}"
    );
    assert!(!err.contains("Counting objects"), "{err}");
    // With git-lfs told to hold to locks itself, it refuses before
    // anything is sent.
    let verify_key = format!("lfs.{}/info/lfs.locksverify", site.url(&project, DEVELOPER));
    bob_git.run(["config", &verify_key, "true"]).unwrap();
    let err = bob.refused(
        "remote.push",
        json!({ "remote": "origin", "branch": "main", "to": "bobs-change" }),
    );
    eprintln!("the same with locksverify:\n{err}");
    assert!(err.contains("the pre-push hook"), "{err}");
    assert!(
        err.contains(&format!("* a.bin - {MAINTAINER} (refs: bobs-change)")),
        "{err}"
    );
    bob_git.run(["config", "--unset", &verify_key]).unwrap();
    bob_git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();

    // A reporter may read: the list, with nothing said to be his, and the
    // content; and may not lock.
    site.set_role(&project, DEVELOPER, 20, || {
        !site.may_push(&project, DEVELOPER)
    });
    let err = bob.refused("lfs.lock", json!({ "path": "b.bin" }));
    eprintln!("a reporter locking: {err}");
    assert!(err.contains("b.bin was not locked"), "{err}");
    let seen = bob.ok("lfs.locks", json!({}));
    assert_eq!(
        listed(&seen),
        vec![("a.bin".to_owned(), MAINTAINER.to_owned(), None)]
    );
    let err = bob.refused("lfs.unlock", json!({ "path": "a.bin", "force": true }));
    eprintln!("a reporter forcing: {err}");
    bob.ok("lfs.fetch", json!({}));

    alice.ok("lfs.unlock", json!({ "path": "a.bin" }));
    assert_eq!(listed(&alice.ok("lfs.locks", json!({}))), vec![]);
    let asked = asking.take();
    eprintln!("asked:\n{}", shown(&asked));
    for (kind, prompt) in &asked {
        assert_eq!(*kind, Kind::Secret, "{}", shown(&asked));
        assert!(
            matches!(asked_for(prompt), Some(MAINTAINER | DEVELOPER)),
            "{}",
            shown(&asked)
        );
    }
}
