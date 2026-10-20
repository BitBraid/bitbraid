//! Fetch and push through the engine against github.com and gitlab.com,
//! live, as test accounts: over HTTPS with the token typed in the
//! application's dialog, kept by a credential helper in a throwaway
//! keyring, and wrong; over SSH with a key of the test's own that has a
//! passphrase, uploaded with `provider.ssh_key_add` and removed in the
//! same run, and a known-hosts file that starts empty, so that the real
//! question about the host's key comes up, refused and then accepted.
//!
//! Opt-in and never in ci: the tests are ignored, run with `--ignored`
//! when `BITBRAID_LIVE_TRANSPORT` names the hosts (`github`, `gitlab`,
//! both split by a comma, or `all`), and skipped with a message otherwise.
//! They need ssh and ssh-keygen, and the accounts' tokens in the keychain as
//! the integrations crate's live tests keep them (`secret-tool`, service
//! `bitbraid-test`, accounts `github-main` and `gitlab-main`, each with
//! its `login` attribute); the run needs surroundings of its own (below):
//!
//! ```sh
//! env HOME=<scratch> GIT_CONFIG_GLOBAL=<scratch>/gitconfig GIT_CONFIG_NOSYSTEM=1 \
//!     -u SSH_AUTH_SOCK -u GITLAB_TOKEN \
//!     BITBRAID_LIVE_TRANSPORT=github,gitlab BITBRAID_LIVE_GITHUB_ORG=<organisation> \
//!     cargo test -p bitbraid-cli --test suite live_hosts:: -- --ignored
//! ```
//!
//! No account is named in this file. At github.com the repository is
//! `<repo>-writes` of the organisation `BITBRAID_LIVE_GITHUB_ORG` names,
//! pushed to as its machine account; at gitlab.com it is the test
//! account's own `<repo>-writes` (`<repo>` is `bitbraid-live`, or the
//! name `BITBRAID_LIVE_REPO` gives, as for the provider commands' live
//! tests, which make those repositories). The logins are the keychain
//! items' `login` attribute; the tokens are read from the same items when
//! the test runs and are never printed or written: what a helper keeps is
//! compared and never shown, and no message of a failure carries one. The
//! environment's `GITLAB_TOKEN` is never read, and has to be unset.
//!
//! What is pushed: a branch `bitbraid-live-auth-<milliseconds>` per test,
//! of commits by a test identity, deleted through the engine before the
//! test ends, and by plain git when the test failed half-way. Branches of
//! that prefix that a stopped run left behind are deleted when a test
//! starts. Nothing else is pushed to, and no other repository is touched.
//! The key is titled `bitbraid-live-auth test key`; keys of that title are
//! removed when the SSH test starts and when it ends. The provider
//! commands go through a guard that lets `GET /user`, a GitLab token's
//! `GET /personal_access_tokens/self` and a `POST /user/keys` with that
//! title through, and refuses everything else before it is sent.
//!
//! Nothing of the developer's is used. The run needs a home of its own
//! (`HOME` without `.ssh` in it), a global git configuration of its own
//! (`GIT_CONFIG_GLOBAL`, `GIT_CONFIG_NOSYSTEM=1`), no `SSH_AUTH_SOCK` and
//! no askpass program, or the test refuses to start. ssh is run with
//! `-F /dev/null`, the test's key alone (`IdentitiesOnly`), no agent and
//! the scratch known-hosts file: ssh finds the home in the password
//! database rather than in `HOME`, and with these options reads nothing
//! there (its debug output names `/dev/null` and the scratch files
//! alone). Each repository empties its list of credential helpers, so
//! that only the test's own is asked.
//!
//! Sources: docs.github.com "GitHub's SSH key fingerprints", "Managing
//! your personal access tokens" (a token where git asks for a password),
//! REST "Git SSH keys" (list, create, delete for the authenticated user);
//! docs.gitlab.com "GitLab.com settings" (the SSH host keys'
//! fingerprints), "Personal access tokens", REST "User SSH keys"
//! (`/user/keys`); git-push(1), git-ls-remote(1), gitcredentials(7),
//! ssh(1) (`-F`, `-i`, `-o`), ssh_config(5) (`IdentitiesOnly`,
//! `IdentityAgent`, `UserKnownHostsFile`, `GlobalKnownHostsFile`,
//! `StrictHostKeyChecking`), ssh-keygen(1), secret-tool(1).
#![allow(clippy::print_stderr)]

use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, atomic::AtomicBool},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use bitbraid_engine::{CommandContext, Registry};
use bitbraid_git::askpass::Kind;
use bitbraid_integrations::{
    Services,
    http::{Call, Client, Fetch, FetchError, Limits, Method, Response, Url},
    secrets::{MemoryStore, Secret},
};
use bitbraid_testkit::Git;
use serde_json::{Value, json};

use super::{
    Asking, ONE,
    credential_helpers::{Helper, fresh_dir, on_path},
};

/// The repository pushed to, at either host: `<repo>-writes`, `<repo>`
/// being `BITBRAID_LIVE_REPO` or `bitbraid-live`.
fn writes_repo() -> String {
    let repo = std::env::var("BITBRAID_LIVE_REPO")
        .map(|repo| repo.trim().to_owned())
        .unwrap_or_default();
    let repo = if repo.is_empty() {
        "bitbraid-live".to_owned()
    } else {
        repo
    };
    assert!(
        repo.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.')),
        "BITBRAID_LIVE_REPO is not a repository's name"
    );
    format!("{repo}-writes")
}
/// What every branch the tests push begins with.
const PREFIX: &str = "bitbraid-live-auth-";
/// The title of the key the SSH tests upload.
const KEY_TITLE: &str = "bitbraid-live-auth test key";
/// Who the commits are by.
const IDENTITY: (&str, &str) = ("BitBraid Test", "bitbraid-test@example.invalid");
/// The passphrase of the key made for the test.
const PASSPHRASE: &str = "a passphrase made for one live test";
/// A token neither host takes.
const WRONG: &str = "not-a-token-either-host-takes";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Host {
    Github,
    Gitlab,
}

impl Host {
    /// Its name in `BITBRAID_LIVE_TRANSPORT`.
    fn name(self) -> &'static str {
        match self {
            Self::Github => "github",
            Self::Gitlab => "gitlab",
        }
    }

    /// Where git reaches it.
    fn web(self) -> &'static str {
        match self {
            Self::Github => "github.com",
            Self::Gitlab => "gitlab.com",
        }
    }

    /// The keychain item of its test account.
    fn item(self) -> &'static str {
        match self {
            Self::Github => "github-main",
            Self::Gitlab => "gitlab-main",
        }
    }

    /// The host of its REST API, and what every path there begins with.
    fn api(self) -> (&'static str, &'static str) {
        match self {
            Self::Github => ("api.github.com", ""),
            Self::Gitlab => ("gitlab.com", "/api/v4"),
        }
    }

    /// Its SSH host keys' fingerprints, as its documentation gives them.
    fn fingerprints(self) -> &'static [&'static str] {
        match self {
            Self::Github => &[
                "SHA256:+DiY3wvvV6TuJJhbpZisF/zLDA0zPMSvHdkr4UvCOqU",
                "SHA256:p2QAMXNIC1TJYWeIOttrVc98/R1BUFWu3/LiyKgUfQM",
                "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
            ],
            Self::Gitlab => &[
                "SHA256:eUXGGm1YGsMAS7vkcx6JOJdOGHPem5gQp4taiCfCLB8",
                "SHA256:HbW3g8zUjNSksFbqTiUWPWg2Bq1x8xdGUrliXFzSnUw",
                "SHA256:ROQFvPThGrW4RuWLoL9tq9I9zJ42fK4XywyRtbOz/EQ",
            ],
        }
    }
}

/// Whether a name read at run time is a login or a repository's owner.
fn plain_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// The login stored with a keychain item, read from its attributes, which
/// `secret-tool` writes to its standard error; its standard output (the
/// secret) is dropped unread.
fn keychain_login(item: &str) -> Option<String> {
    let out = Command::new("secret-tool")
        .args([
            "search",
            "--all",
            "service",
            "bitbraid-test",
            "account",
            item,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .ok()?;
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|line| line.trim().strip_prefix("attribute.login = "))
        .map(|login| login.trim().to_owned())
}

/// A test account's token, from its keychain item.
fn keychain_token(item: &str) -> Option<Secret> {
    let out = Command::new("secret-tool")
        .args(["lookup", "service", "bitbraid-test", "account", item])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    let token = String::from_utf8(out.stdout).ok()?.trim().to_owned();
    (out.status.success() && !token.is_empty()).then(|| Secret::new(token))
}

/// Refuse to start where something of the developer's could be reached.
fn isolated() {
    for (name, why) in [
        ("GITLAB_TOKEN", "a token of another account"),
        ("SSH_AUTH_SOCK", "an agent with the developer's keys"),
        ("GIT_ASKPASS", "it would be asked instead of the dialog"),
        ("SSH_ASKPASS", "it would be asked instead of the dialog"),
    ] {
        assert!(
            std::env::var_os(name).is_none(),
            "unset {name} to run the live tests: {why}"
        );
    }
    assert!(
        std::env::var_os("GIT_CONFIG_GLOBAL").is_some()
            && std::env::var("GIT_CONFIG_NOSYSTEM").as_deref() == Ok("1"),
        "the live tests need a global git configuration of their own \
         (GIT_CONFIG_GLOBAL) and none of the system's (GIT_CONFIG_NOSYSTEM=1)"
    );
    let home = std::env::var_os("HOME").map(PathBuf::from);
    assert!(
        home.is_some_and(|home| !home.join(".ssh").exists()),
        "the live tests need a HOME of their own, without .ssh in it"
    );
}

fn now_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|since| since.as_millis())
        .unwrap_or_default()
}

/// The provider commands' way to the network: the real client, for the
/// few requests the test needs.
#[derive(Debug)]
struct Guard {
    inner: Client,
    host: Host,
    refused: Mutex<Vec<String>>,
}

impl Fetch for Guard {
    fn send(&self, call: &Call, cancel: &AtomicBool) -> Result<Response, FetchError> {
        let (host, base) = self.host.api();
        let path = call
            .url
            .path()
            .strip_prefix(base)
            .filter(|_| call.url.host() == host && call.url.secure())
            .unwrap_or_default();
        let titled = || {
            call.body
                .as_deref()
                .and_then(|body| serde_json::from_slice::<Value>(body).ok())
                .is_some_and(|body| body["title"] == KEY_TITLE)
        };
        let allowed = match (call.method, path) {
            (Method::Get, "/user") => true,
            (Method::Get, "/personal_access_tokens/self") => self.host == Host::Gitlab,
            (Method::Post, "/user/keys") => titled(),
            _ => false,
        };
        if !allowed {
            if let Ok(mut refused) = self.refused.lock() {
                refused.push(format!("{} {}", call.method.as_str(), call.url.path()));
            }
            return Err(FetchError::Address(
                "one the live test does not let through",
            ));
        }
        self.inner.send(call, cancel)
    }
}

/// One live test: the account, the repository's owner and a scratch
/// directory, and the branch the test may push.
struct Live {
    host: Host,
    test: &'static str,
    login: String,
    token: Secret,
    owner: String,
    repo: String,
    dir: PathBuf,
    branch: String,
    client: Client,
}

impl Live {
    /// The account of `host`, or `None` with a message when the live tests
    /// of that host are not asked for.
    fn start(host: Host, test: &'static str) -> Option<Self> {
        let wanted = std::env::var("BITBRAID_LIVE_TRANSPORT").unwrap_or_default();
        if !wanted
            .split(',')
            .map(str::trim)
            .any(|name| name == host.name() || name == "all")
        {
            eprintln!(
                "{test}: skipped, set BITBRAID_LIVE_TRANSPORT={} with the test account in the keychain",
                host.name()
            );
            return None;
        }
        isolated();
        let login = keychain_login(host.item())
            .unwrap_or_else(|| panic!("no login is stored with the keychain item {}", host.item()));
        assert!(plain_name(&login), "the test account's name is not a login");
        let token = keychain_token(host.item())
            .unwrap_or_else(|| panic!("no token is stored with the keychain item {}", host.item()));
        let owner = match host {
            Host::Github => std::env::var("BITBRAID_LIVE_GITHUB_ORG")
                .ok()
                .map(|org| org.trim().to_owned())
                .filter(|org| plain_name(org))
                .unwrap_or_else(|| panic!("set BITBRAID_LIVE_GITHUB_ORG to the test organisation")),
            Host::Gitlab => login.clone(),
        };
        let dir = fresh_dir(&format!("live-{}", host.name()));
        let branch = format!("{PREFIX}{}", now_millis());
        let git = Git::new(&dir, dir.join("home"));
        git.ensure_home().unwrap();
        Some(Self {
            host,
            test,
            login,
            token,
            owner,
            repo: writes_repo(),
            dir,
            branch,
            client: Client::new(&bitbraid_integrations::services::agent()),
        })
    }

    fn https(&self) -> String {
        format!(
            "https://{}/{}/{}.git",
            self.host.web(),
            self.owner,
            self.repo
        )
    }

    fn ssh(&self) -> String {
        format!("git@{}:{}/{}.git", self.host.web(), self.owner, self.repo)
    }

    /// Plain git in the scratch directory, with a home of its own.
    fn git(&self) -> Git {
        Git::new(&self.dir, self.dir.join("home"))
    }

    /// The branches of the prefix on the remote and their commits, as
    /// `git ls-remote` sees them over HTTPS (the repository is public:
    /// nothing is asked).
    fn remote_branches(&self) -> Result<Vec<(String, String)>, String> {
        let out = self
            .git()
            .run([
                "-c",
                "credential.helper=",
                "ls-remote",
                "--heads",
                &self.https(),
                &format!("refs/heads/{PREFIX}*"),
            ])
            .map_err(|err| err.to_string())?;
        Ok(out
            .lines()
            .filter_map(|line| {
                let (commit, name) = line.split_once('\t')?;
                let name = name.strip_prefix("refs/heads/")?;
                name.starts_with(PREFIX)
                    .then(|| (name.to_owned(), commit.to_owned()))
            })
            .collect())
    }

    /// The commit the test's branch is at on the remote.
    fn remote_tip(&self) -> Option<String> {
        self.remote_branches()
            .unwrap_or_else(|err| panic!("ls-remote: {err}"))
            .into_iter()
            .find(|(name, _)| *name == self.branch)
            .map(|(_, commit)| commit)
    }

    /// Delete a branch of the prefix on the remote with plain git over
    /// HTTPS: what a run stopped half-way left behind. The token is given
    /// through an askpass program that reads it from its environment, and
    /// is in no file. Never panics: it runs while a failed test unwinds.
    fn delete_by_git(&self, branch: &str) {
        if !branch.starts_with(PREFIX) {
            return;
        }
        let script = bitbraid_testkit::script(
            &self.dir,
            "cleanup-askpass",
            "#!/bin/sh\ncase \"$1\" in\n  Username*) printf '%s\\n' \"$BITBRAID_LIVE_USER\" ;;\n  *) printf '%s\\n' \"$BITBRAID_LIVE_SECRET\" ;;\nesac\n",
        );
        let Ok(script) = script else {
            eprintln!(
                "{}: could not write the askpass program to delete {branch}",
                self.test
            );
            return;
        };
        let out = self
            .git()
            .command()
            .args(["-c", "credential.helper=", "push", "-q", &self.https()])
            .arg("--delete")
            .arg(format!("refs/heads/{branch}"))
            .env("GIT_ASKPASS", &script)
            .env("BITBRAID_LIVE_USER", &self.login)
            .env("BITBRAID_LIVE_SECRET", self.token.expose())
            .stdin(Stdio::null())
            .output();
        match out {
            Ok(out) if out.status.success() => {
                eprintln!("{}: deleted {branch}, left behind on the remote", self.test);
            }
            Ok(out) => eprintln!(
                "{}: could not delete {branch}: {}",
                self.test,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
            Err(err) => eprintln!("{}: could not delete {branch}: {err}", self.test),
        }
    }

    /// Delete what runs that were stopped left of the prefix.
    fn sweep(&self) {
        for (name, _) in self.remote_branches().unwrap_or_default() {
            self.delete_by_git(&name);
        }
    }

    /// A request to the host's REST API with the account's token.
    fn api(&self, method: Method, path: &str) -> Result<(u16, Value), String> {
        let (host, base) = self.host.api();
        let url =
            Url::parse(&format!("https://{host}{base}{path}")).map_err(|err| err.to_string())?;
        let mut call = Call::new(
            method,
            url,
            Limits {
                connect: Duration::from_secs(10),
                total: Duration::from_secs(60),
                body: 4 << 20,
            },
        )
        .with_header("Authorization", &format!("Bearer {}", self.token.expose()));
        if self.host == Host::Github {
            call = call
                .with_header("Accept", "application/vnd.github+json")
                .with_header("X-GitHub-Api-Version", "2022-11-28");
        }
        let response = self
            .client
            .send(&call, &AtomicBool::new(false))
            .map_err(|err| format!("{} {path}: {err}", method.as_str()))?;
        let value = serde_json::from_slice(&response.body).unwrap_or(Value::Null);
        Ok((response.status, value))
    }

    /// Remove the account's keys of the test's title; how many there were.
    /// Never panics: it runs while a failed test unwinds.
    fn remove_keys(&self) -> Result<usize, String> {
        let (status, keys) = self.api(Method::Get, "/user/keys?per_page=100")?;
        if status != 200 {
            return Err(format!("listing the keys: {status}"));
        }
        let mut removed = 0;
        for key in keys.as_array().cloned().unwrap_or_default() {
            if key["title"] != KEY_TITLE {
                continue;
            }
            let id = key["id"].as_u64().ok_or("a key without an id")?;
            let (status, _) = self.api(Method::Delete, &format!("/user/keys/{id}"))?;
            if !matches!(status, 204 | 404) {
                return Err(format!("removing key {id}: {status}"));
            }
            removed += 1;
        }
        Ok(removed)
    }

    /// A repository in scratch whose remote `origin` is `url`, open in
    /// the engine with the provider commands' services behind the guard.
    fn repo(&self, url: &str) -> Repo {
        let dir = self.dir.join("repository");
        std::fs::create_dir_all(&dir).unwrap();
        let git = Git::new(&dir, self.dir.join("home"));
        git.run(["init", "-q", "-b", "main"]).unwrap();
        for (key, value) in [
            ("user.name", IDENTITY.0),
            ("user.email", IDENTITY.1),
            // Only the test's own helper, when it adds one.
            ("credential.helper", ""),
            ("ssh.variant", "ssh"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        git.run(["remote", "add", "origin", url]).unwrap();
        let guard = Arc::new(Guard {
            inner: Client::new(&bitbraid_integrations::services::agent()),
            host: self.host,
            refused: Mutex::new(Vec::new()),
        });
        let services = Arc::new(Services::new(
            guard.clone(),
            Arc::new(MemoryStore::new()),
            self.dir.join("config"),
        ));
        let registry = bitbraid_integrations::registry();
        let mut ctx = CommandContext {
            config_dir: Some(self.dir.join("config")),
            ..Default::default()
        };
        ctx.extensions.insert(services);
        registry
            .run("repo.open", &mut ctx, &json!({ "path": dir }))
            .expect("open the scratch repository");
        Repo {
            git,
            registry,
            ctx,
            guard,
        }
    }

    /// ssh for the test: its key alone, no agent, no configuration file,
    /// the known-hosts file in scratch, asking about a host it does not
    /// know.
    fn ssh_command(&self, key: &Path, known_hosts: &Path) -> String {
        let quote = |path: &Path| {
            let text = path.display().to_string();
            assert!(!text.contains('\''), "a quote in {text}");
            format!("'{text}'")
        };
        format!(
            "ssh -F /dev/null -i {} -o IdentitiesOnly=yes -o IdentityAgent=none \
             -o UserKnownHostsFile={} -o GlobalKnownHostsFile=/dev/null \
             -o StrictHostKeyChecking=ask -o HashKnownHosts=no \
             -o PreferredAuthentications=publickey -o PasswordAuthentication=no \
             -o KbdInteractiveAuthentication=no -o ControlMaster=no -o ControlPath=none \
             -o UpdateHostKeys=no -o AddKeysToAgent=no -o ForwardAgent=no",
            quote(key),
            quote(known_hosts)
        )
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // The test's branch, when it is still there because the test
        // failed before deleting it.
        if let Ok(branches) = self.remote_branches()
            && branches.iter().any(|(name, _)| *name == self.branch)
        {
            self.delete_by_git(&self.branch.clone());
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Removes the keys of the test's title, whatever happens.
struct KeysGone<'a>(&'a Live);

impl Drop for KeysGone<'_> {
    fn drop(&mut self) {
        match self.0.remove_keys() {
            Ok(0) => {}
            Ok(removed) => eprintln!("{}: removed {removed} key(s) left behind", self.0.test),
            Err(err) => eprintln!("{}: could not remove the test's key: {err}", self.0.test),
        }
    }
}

/// The scratch repository, open in the engine.
struct Repo {
    git: Git,
    registry: Registry,
    ctx: CommandContext,
    guard: Arc<Guard>,
}

impl Repo {
    fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    /// A commit by the test identity, dated now, on the checked-out branch.
    fn commit(&self, file: &str, branch: &str) -> String {
        std::fs::write(
            self.git.cwd().join(file),
            format!("Written by BitBraid's live test on {branch}.\n"),
        )
        .unwrap();
        let date = format!(
            "{} +0000",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|since| since.as_secs())
                .unwrap_or_default()
        );
        let env = [
            ("GIT_AUTHOR_NAME", IDENTITY.0),
            ("GIT_AUTHOR_EMAIL", IDENTITY.1),
            ("GIT_COMMITTER_NAME", IDENTITY.0),
            ("GIT_COMMITTER_EMAIL", IDENTITY.1),
            ("GIT_AUTHOR_DATE", date.as_str()),
            ("GIT_COMMITTER_DATE", date.as_str()),
        ];
        self.git.run(["add", file]).unwrap();
        self.git
            .run_with(["commit", "-q", "-m", &format!("live test: {file}")], &env)
            .unwrap();
        self.git
            .run(["rev-parse", "HEAD"])
            .unwrap()
            .trim()
            .to_owned()
    }

    /// Check out the test's branch: from the remote's `main` as fetched,
    /// or with no history where there is none.
    fn start_branch(&self, branch: &str) {
        assert!(branch.starts_with(PREFIX));
        let from_main = self
            .git
            .run(["rev-parse", "--verify", "-q", "refs/remotes/origin/main"])
            .is_ok();
        if from_main {
            self.git
                .run(["checkout", "-q", "--no-track", "-b", branch, "origin/main"])
                .unwrap();
        } else {
            self.git
                .run(["checkout", "-q", "--orphan", branch])
                .unwrap();
        }
    }

    fn push(&mut self, branch: &str) -> Result<Value, String> {
        assert!(branch.starts_with(PREFIX));
        self.run(
            "remote.push",
            json!({ "remote": "origin", "branch": branch, "set_upstream": true }),
        )
    }

    fn delete(&mut self, branch: &str) -> Result<Value, String> {
        assert!(branch.starts_with(PREFIX));
        self.run(
            "branch.delete_remote",
            json!({ "remote": "origin", "branch": branch }),
        )
    }
}

/// The dialog, answering the user name and the token.
fn answering(login: &str, token: &str) -> Asking {
    let (login, token) = (login.to_owned(), token.to_owned());
    Asking::with(move |kind, prompt| {
        Some(match kind {
            Kind::Text if prompt.starts_with("Username for") => login.clone(),
            Kind::Secret if prompt.starts_with("Password for") => token.clone(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    })
}

// ------------------------------------------------------------- HTTPS

fn over_https(host: Host, test: &'static str) {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(live) = Live::start(host, test) else {
        return;
    };
    live.sweep();
    let branch = live.branch.clone();
    let mut repo = live.repo(&live.https());
    let token = live.token.expose().to_owned();

    // The repository is public: a fetch asks nothing.
    let asking = answering(&live.login, &token);
    repo.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    assert!(asking.asked().is_empty(), "{:?}", asking.asked());
    repo.start_branch(&branch);
    let first = repo.commit("mx05-first.txt", &branch);

    // A push through the dialog alone: the user name and the token are
    // asked, and nothing keeps them.
    repo.push(&branch)
        .unwrap_or_else(|err| panic!("the first push: {err}"));
    assert_eq!(live.remote_tip().as_deref(), Some(first.as_str()));
    assert_eq!(asking.count("Username for"), 1, "{:?}", asking.asked());
    assert_eq!(asking.count("Password for"), 1, "{:?}", asking.asked());
    drop(asking);

    // A wrong token typed in the dialog: refused and said, nothing moves.
    let mistyped = answering(&live.login, WRONG);
    let second = repo.commit("mx05-second.txt", &branch);
    let err = repo.push(&branch).expect_err("the token is wrong");
    assert!(err.contains("Authentication failed"), "{err}");
    assert!(
        err.contains("The server refused the user name and the password or token"),
        "{err}"
    );
    assert!(!err.contains(&token) && !err.contains(WRONG));
    assert_eq!(mistyped.asked().len(), 2, "{:?}", mistyped.asked());
    assert_eq!(live.remote_tip().as_deref(), Some(first.as_str()));
    drop(mistyped);

    let asking = answering(&live.login, &token);
    match Helper::libsecret_or_skip(test) {
        Some(helper) => {
            // A helper that keeps the token, in a throwaway keyring: the
            // next push asks, and the helper keeps the answer.
            helper.add_to_git(&repo.git);
            let web = host.web();
            assert!(helper.kept_at("https", web).is_none());
            repo.push(&branch)
                .unwrap_or_else(|err| panic!("the push with the helper: {err}"));
            assert_eq!(live.remote_tip().as_deref(), Some(second.as_str()));
            assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());
            assert!(
                helper.kept_at("https", web) == Some((live.login.clone(), token.clone())),
                "the helper does not keep the token typed in the dialog"
            );

            // The push after that asks nothing.
            let third = repo.commit("mx05-third.txt", &branch);
            repo.push(&branch)
                .unwrap_or_else(|err| panic!("the push the helper answered: {err}"));
            assert_eq!(live.remote_tip().as_deref(), Some(third.as_str()));
            assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());

            // The kept token goes wrong: refused without a question, and
            // git has the helper forget it.
            helper.keep_at("https", web, &live.login, WRONG);
            let fourth = repo.commit("mx05-fourth.txt", &branch);
            let err = repo.push(&branch).expect_err("the kept token is wrong");
            assert!(err.contains("Authentication failed"), "{err}");
            assert!(err.contains("the next attempt asks again"), "{err}");
            assert!(!err.contains(&token) && !err.contains(WRONG));
            assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());
            assert_eq!(live.remote_tip().as_deref(), Some(third.as_str()));
            assert!(
                helper.kept_at("https", web).is_none(),
                "git did not have the helper forget the refused token"
            );

            // So the next push asks again, and the answer is kept.
            repo.push(&branch)
                .unwrap_or_else(|err| panic!("the push asked again: {err}"));
            assert_eq!(live.remote_tip().as_deref(), Some(fourth.as_str()));
            assert_eq!(asking.asked().len(), 4, "{:?}", asking.asked());
            assert!(
                helper.kept_at("https", web) == Some((live.login.clone(), token.clone())),
                "the helper does not keep the token typed again"
            );

            // The branch is deleted through the engine, nothing asked.
            repo.delete(&branch)
                .unwrap_or_else(|err| panic!("delete the branch: {err}"));
            assert_eq!(asking.asked().len(), 4, "{:?}", asking.asked());
        }
        None => {
            repo.delete(&branch)
                .unwrap_or_else(|err| panic!("delete the branch: {err}"));
            assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());
        }
    }
    assert_eq!(live.remote_tip(), None, "the branch is gone");
    assert!(repo.guard.refused.lock().unwrap().is_empty());
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_TRANSPORT and the test accounts in the keychain"]
fn github_over_https_the_token_is_asked_kept_and_a_wrong_one_refused() {
    over_https(Host::Github, "github over https");
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_TRANSPORT and the test accounts in the keychain"]
fn gitlab_over_https_the_token_is_asked_kept_and_a_wrong_one_refused() {
    over_https(Host::Gitlab, "gitlab over https");
}

// --------------------------------------------------------------- SSH

fn over_ssh(host: Host, test: &'static str) {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(live) = Live::start(host, test) else {
        return;
    };
    assert!(
        on_path("ssh") && on_path("ssh-keygen"),
        "ssh is not installed"
    );
    live.sweep();
    match live.remove_keys() {
        Ok(0) => {}
        Ok(removed) => eprintln!("{test}: removed {removed} key(s) left behind"),
        Err(err) => panic!("{test}: {err}"),
    }
    let keys_gone = KeysGone(&live);
    let branch = live.branch.clone();

    // A key with a passphrase, made for this run.
    let key = live.dir.join("id_mx05");
    let made = Command::new("ssh-keygen")
        .args([
            "-q",
            "-t",
            "ed25519",
            "-C",
            "bitbraid-live-auth@example.invalid",
            "-N",
        ])
        .arg(PASSPHRASE)
        .arg("-f")
        .arg(&key)
        .stdin(Stdio::null())
        .status()
        .unwrap();
    assert!(made.success(), "ssh-keygen");

    // Uploaded with the provider command, for an account connected with
    // the token.
    let mut repo = live.repo(&live.ssh());
    let account = repo
        .run(
            "account.add_token",
            json!({ "kind": host.name(), "token": live.token.expose() }),
        )
        .unwrap_or_else(|err| panic!("connect the account: {err}"));
    assert!(
        account["login"].as_str() == Some(live.login.as_str()),
        "the token is not the test account's"
    );
    let public = key.with_extension("pub");
    let added = repo
        .run(
            "provider.ssh_key_add",
            json!({ "account": account["id"], "title": KEY_TITLE,
                    "path": public.to_str().unwrap() }),
        )
        .unwrap_or_else(|err| panic!("upload the key: {err}"));
    assert_eq!(added["title"], KEY_TITLE);
    let known_hosts = live.dir.join("known_hosts");
    let ssh = live.ssh_command(&key, &known_hosts);
    repo.git.run(["config", "core.sshCommand", &ssh]).unwrap();
    let published = |prompt: &str| {
        host.fingerprints()
            .iter()
            .any(|fingerprint| prompt.contains(fingerprint))
    };
    let known = || std::fs::read_to_string(&known_hosts).unwrap_or_default();

    // A host never seen before: the question about its key names a key
    // the host publishes. Refused, nothing is sent and nothing written.
    let refusing = Asking::with(|_, prompt| {
        assert!(
            prompt.contains("(yes/no"),
            "only the host is asked: {prompt}"
        );
        Some("no".to_owned())
    });
    let err = repo
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the host was refused");
    assert!(err.contains("Host key verification failed"), "{err}");
    assert!(
        err.contains("The key of the host was not accepted"),
        "{err}"
    );
    let asked = refusing.asked();
    assert_eq!(asked.len(), 1, "{asked:?}");
    assert!(asked[0].1.contains("authenticity of host"), "{asked:?}");
    assert!(
        published(&asked[0].1),
        "not a key {} publishes: {asked:?}",
        host.web()
    );
    assert_eq!(known(), "", "the host was written down");
    drop(refusing);

    // Accepted: then the passphrase is asked, and the fetch goes through.
    let accepting = Asking::with(|kind, prompt| {
        Some(match kind {
            _ if prompt.contains("(yes/no") => "yes".to_owned(),
            Kind::Secret if prompt.starts_with("Enter passphrase") => PASSPHRASE.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    repo.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch over ssh: {err}"));
    let asked = accepting.asked();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert!(published(&asked[0].1), "{asked:?}");
    assert_eq!(asked[1].0, Kind::Secret);
    assert!(
        known().contains(host.web()),
        "the host is known from now on"
    );
    drop(accepting);

    // A wrong passphrase: ssh asks a few times and the server never sees
    // the key.
    let mistyped = Asking::with(|kind, prompt| {
        assert!(!prompt.contains("(yes/no"), "the host is known: {prompt}");
        assert_eq!(kind, Kind::Secret, "{prompt}");
        Some("not the passphrase".to_owned())
    });
    let err = repo
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the passphrase is wrong");
    assert!(err.contains("Permission denied (publickey"), "{err}");
    assert!(err.contains("The server did not accept the key"), "{err}");
    assert!(!err.contains(PASSPHRASE));
    assert!(
        (1..=5).contains(&mistyped.asked().len()),
        "{:?}",
        mistyped.asked()
    );
    drop(mistyped);

    // A branch pushed over ssh and deleted again, the passphrase asked
    // each time and the host no more.
    let asking = Asking::with(|kind, prompt| {
        assert!(!prompt.contains("(yes/no"), "the host is known: {prompt}");
        assert_eq!(kind, Kind::Secret, "{prompt}");
        Some(PASSPHRASE.to_owned())
    });
    repo.start_branch(&branch);
    let first = repo.commit("mx05-ssh.txt", &branch);
    repo.push(&branch)
        .unwrap_or_else(|err| panic!("push over ssh: {err}"));
    assert_eq!(live.remote_tip().as_deref(), Some(first.as_str()));
    repo.delete(&branch)
        .unwrap_or_else(|err| panic!("delete over ssh: {err}"));
    assert_eq!(live.remote_tip(), None, "the branch is gone");
    assert_eq!(asking.count("Enter passphrase"), 2, "{:?}", asking.asked());
    assert!(repo.guard.refused.lock().unwrap().is_empty());

    // The key is removed in the same run.
    assert_eq!(live.remove_keys(), Ok(1), "the test's key was there");
    assert_eq!(live.remove_keys(), Ok(0), "the test's key is gone");
    drop(keys_gone);
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_TRANSPORT and the test accounts in the keychain"]
fn github_over_ssh_the_host_and_the_passphrase_are_asked() {
    over_ssh(Host::Github, "github over ssh");
}

#[test]
#[ignore = "live: needs BITBRAID_LIVE_TRANSPORT and the test accounts in the keychain"]
fn gitlab_over_ssh_the_host_and_the_passphrase_are_asked() {
    over_ssh(Host::Gitlab, "gitlab over ssh");
}
