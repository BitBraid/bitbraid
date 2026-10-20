//! Fetch and push through the engine against servers that ask who is
//! there: over SSH with a key that has a passphrase and a host nobody has
//! seen before, and over HTTP with a user name and a token.
//! The questions git and ssh ask have to arrive at the application's
//! askpass server, through this binary run as their askpass program, and
//! the answers have to get back; a wrong answer or a refusal has to end
//! the command with what went wrong, and nothing may be kept.
//!
//! Both servers are started here, on a free port of the loopback
//! interface, and are gone when the test is:
//!
//! - `sshd`, unprivileged, with a host key and a user key made for the
//!   test in a scratch directory. The client is steered through the
//!   scratch repository's `core.sshCommand` alone: a configuration file
//!   of its own (`-F`), its own known-hosts file, the one key, no agent.
//!   Nothing under `~/.ssh` is read or written by either side, and the
//!   server's sessions run with a scratch home.
//! - A smart-HTTP server of the test's own that asks for Basic
//!   credentials and hands authorized requests to `git http-backend`.
//!   Plain HTTP: on the loopback interface there is nobody to listen in,
//!   and what is tested is the asking, which is the same over TLS.
//!
//! Where `sshd` or `git http-backend` is not installed the tests say so
//! and pass without having run. Git for Windows brings no `sshd`, so there
//! the ssh tests say so; `askpass::ssh_gets_a_passphrase_through_the_binary`
//! has ssh's own way of asking go through the binary on every system.
//!
//! Sources: sshd(8), sshd_config(5) (`SetEnv`, `AuthorizedKeysFile`,
//! `StrictModes`, `PermitUserRC`), ssh_config(5) (`IdentityAgent`,
//! `IdentitiesOnly`, `UserKnownHostsFile`, `StrictHostKeyChecking`),
//! ssh-keygen(1), git-http-backend(1), gitcredentials(7), git-config(1)
//! (`core.sshCommand`, `credential.helper`), RFC 7617 (Basic), RFC 3875
//! (CGI), RFC 9112 (chunked transfer coding).
#![allow(clippy::print_stderr)]

use std::{
    io::{BufRead, BufReader, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_git::askpass::{Event, Kind, Server};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir, scratch_root};
use serde_json::{Value, json};

use crate::ASKPASS_SERVER as ONE;
static COUNTER: AtomicU64 = AtomicU64::new(0);

mod credential_helpers;
mod live_hosts;
mod wsl;

pub(crate) fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitbraid-cli"))
}

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

    pub(crate) fn asked(&self) -> Vec<(Kind, String)> {
        self.asked.lock().unwrap().clone()
    }

    pub(crate) fn count(&self, what: &str) -> usize {
        self.asked()
            .iter()
            .filter(|(_, prompt)| prompt.contains(what))
            .count()
    }
}

/// A scratch repository opened in the engine, and the bare repository a
/// server hands out for it.
pub(crate) struct Site {
    pub(crate) copy: Scratch,
    pub(crate) bare: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Site {
    pub(crate) fn new() -> Self {
        let copy = Scratch::of("linear").unwrap();
        let bare = copy.path().with_extension("served.git");
        let _ = std::fs::remove_dir_all(&bare);
        let git = copy.git();
        git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
            .unwrap();
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            // Nothing of the developer's is asked and nothing is kept: an
            // empty value empties the list of helpers.
            ("credential.helper", ""),
            ("http.proxy", ""),
            ("ssh.variant", "ssh"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        // The developer has an askpass program of their own: the engine
        // leaves that alone, and it would be asked on their screen. Have
        // git ask this binary instead, which is all that can be done here.
        if std::env::var_os("SSH_ASKPASS").is_some() {
            git.run(["config", "core.askPass", helper().to_str().unwrap()])
                .unwrap();
        }
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .expect("open scratch");
        Self {
            copy,
            bare,
            registry,
            ctx,
        }
    }

    pub(crate) fn git(&self) -> Git {
        self.copy.git()
    }

    pub(crate) fn served(&self) -> Git {
        Git::new(&self.bare, fixtures_out_dir().join(".home"))
    }

    pub(crate) fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    /// Someone else pushes a commit, over the file system.
    pub(crate) fn upstream_commit(&self, file: &str) -> String {
        let other = self.copy.path().with_extension("other");
        let _ = std::fs::remove_dir_all(&other);
        self.git()
            .run([
                "clone",
                "-q",
                self.bare.to_str().unwrap(),
                other.to_str().unwrap(),
            ])
            .unwrap();
        let git = Git::new(&other, fixtures_out_dir().join(".home"));
        std::fs::write(other.join(file), "from upstream\n").unwrap();
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "-m", &format!("upstream: {file}")])
            .unwrap();
        git.run(["push", "-q", "origin", "main"]).unwrap();
        let _ = std::fs::remove_dir_all(&other);
        self.served().rev_parse("refs/heads/main").unwrap()
    }

    pub(crate) fn local_commit(&self, file: &str) -> String {
        self.copy.write(file, "from here\n").unwrap();
        self.git().run(["add", file]).unwrap();
        self.git()
            .run(["commit", "-q", "-m", &format!("local: {file}")])
            .unwrap();
        self.git().rev_parse("HEAD").unwrap()
    }

    pub(crate) fn tracking(&self) -> Option<String> {
        self.git().rev_parse("refs/remotes/origin/main").ok()
    }
}

impl Drop for Site {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.bare);
    }
}

/// Wait until something listens on the port.
pub(crate) fn listening(port: u16, within: Duration) -> bool {
    listening_at(LOOPBACK, port, within)
}

/// Wait until something listens on the port at `host`.
pub(crate) fn listening_at(host: std::net::IpAddr, port: u16, within: Duration) -> bool {
    let started = Instant::now();
    while started.elapsed() < within {
        if TcpStream::connect((host, port)).is_ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// The loopback address the servers here listen on unless told otherwise.
const LOOPBACK: std::net::IpAddr = std::net::IpAddr::V4(std::net::Ipv4Addr::LOCALHOST);

pub(crate) fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// ---------------------------------------------------------------- SSH

pub(crate) const PASSPHRASE: &str = "correct horse battery staple";

/// An `sshd` of the test's own.
pub(crate) struct Sshd {
    pub(crate) dir: PathBuf,
    pub(crate) port: u16,
    pub(crate) user: String,
    child: Child,
}

/// Where sshd is: on `PATH` or in the system's folders, and on Windows the
/// system's own OpenSSH server, which a user may run without being an
/// administrator.
fn sshd_program() -> Option<PathBuf> {
    let name = if cfg!(windows) { "sshd.exe" } else { "sshd" };
    let system = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("OpenSSH"));
    // sshd insists on being started by its full path.
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .into_iter()
        .chain(["/usr/sbin", "/usr/local/sbin", "/sbin"].map(PathBuf::from))
        .chain(system)
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

/// A path as the configuration files of ssh and sshd and git's addresses
/// take it: with forward slashes on Windows (`D:/scratch/…`), as it is
/// elsewhere.
pub(crate) fn slashed(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text
    }
}

impl Sshd {
    /// Start one, or say why not: `None` when the programs are not there.
    ///
    /// On Windows it is the system's OpenSSH server run as the user, who
    /// is the only one it can let in that way; its keys are made by the
    /// `ssh-keygen` beside it, which keeps a private key for the user
    /// alone as it insists. Its sessions run their commands with `cmd`,
    /// which takes the quotes git puts around a repository's path for part
    /// of the path, so each command is handed on to the sh of Git for
    /// Windows (`ForceCommand`), as a server set up for git there has its
    /// shell set to it.
    fn start() -> Option<Sshd> {
        let sshd = sshd_program()?;
        let have = |program: &Path| {
            Command::new(program)
                .arg("-V")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok()
        };
        let keygen = if cfg!(windows) {
            sshd.with_file_name("ssh-keygen.exe")
        } else {
            PathBuf::from("ssh-keygen")
        };
        if !have(Path::new("ssh")) || !have(&keygen) {
            return None;
        }
        #[cfg(windows)]
        let sh = bitbraid_testkit::script::git_sh()?;
        let user = if cfg!(windows) {
            std::env::var("USERNAME").ok()?
        } else {
            String::from_utf8(Command::new("id").arg("-un").output().ok()?.stdout)
                .ok()?
                .trim()
                .to_owned()
        };

        let dir = scratch_root().join(format!(
            "sshd-{}-{}",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("home")).unwrap();
        let keygen = |file: &str, passphrase: &str| {
            let done = Command::new(&keygen)
                .args(["-q", "-t", "ed25519", "-C", "bitbraid test", "-N"])
                .arg(passphrase)
                .arg("-f")
                .arg(dir.join(file))
                .stdin(Stdio::null())
                .status()
                .unwrap();
            assert!(done.success(), "ssh-keygen {file}");
        };
        keygen("host_key", "");
        keygen("user_key", PASSPHRASE);
        std::fs::copy(dir.join("user_key.pub"), dir.join("authorized_keys")).unwrap();

        let port = free_port();
        let at = slashed(&dir);
        // What the server's own build has no use for, and on Windows the
        // command handed on to git's sh. The user is the server's own
        // there, and no other can come in.
        #[cfg(not(windows))]
        let system = format!("UsePAM no\nPermitUserRC no\nPrintLastLog no\nAllowUsers {user}\n");
        #[cfg(windows)]
        let system = {
            std::fs::write(
                dir.join("serve.cmd"),
                format!("@\"{}\" -c \"%SSH_ORIGINAL_COMMAND%\"\r\n", sh.display()),
            )
            .unwrap();
            format!("ForceCommand {at}/serve.cmd\n")
        };
        // Sessions get a home of their own: the shell that runs
        // git-upload-pack reads no file of the developer's, and git there
        // no configuration of theirs.
        std::fs::write(
            dir.join("sshd_config"),
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {at}/host_key\n\
                 PidFile {at}/sshd.pid\nAuthorizedKeysFile {at}/authorized_keys\n\
                 StrictModes no\nPasswordAuthentication no\n\
                 KbdInteractiveAuthentication no\nPubkeyAuthentication yes\n\
                 PermitUserEnvironment no\nPrintMotd no\n{system}\
                 SetEnv HOME={at}/home GIT_CONFIG_NOSYSTEM=1 GIT_CONFIG_GLOBAL=/dev/null\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join("ssh_config"),
            format!(
                "Host *\n  IdentityFile {at}/user_key\n  IdentitiesOnly yes\n  IdentityAgent none\n\
                 \x20 UserKnownHostsFile {at}/known_hosts\n  GlobalKnownHostsFile /dev/null\n\
                 \x20 StrictHostKeyChecking ask\n  PreferredAuthentications publickey\n\
                 \x20 PasswordAuthentication no\n  KbdInteractiveAuthentication no\n\
                 \x20 ControlMaster no\n  ControlPath none\n  AddKeysToAgent no\n\
                 \x20 UpdateHostKeys no\n  ForwardAgent no\n  LogLevel ERROR\n"
            ),
        )
        .unwrap();
        let log = std::fs::File::create(dir.join("sshd.log")).unwrap();
        let child = Command::new(&sshd)
            .args(["-D", "-e", "-f"])
            .arg(dir.join("sshd_config"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .unwrap();
        let server = Sshd {
            dir,
            port,
            user,
            child,
        };
        assert!(
            listening(port, Duration::from_secs(10)),
            "sshd did not come up:\n{}",
            server.log()
        );
        Some(server)
    }

    pub(crate) fn log(&self) -> String {
        std::fs::read_to_string(self.dir.join("sshd.log")).unwrap_or_default()
    }

    pub(crate) fn known_hosts(&self) -> String {
        std::fs::read_to_string(self.dir.join("known_hosts")).unwrap_or_default()
    }

    /// Point the scratch repository at this server, and its ssh at the
    /// scratch files and at nothing else.
    pub(crate) fn serve(&self, site: &Site) {
        let at = slashed(&self.dir);
        let mut ssh = format!(
            "ssh -F {at}/ssh_config -o IdentityAgent=none -o IdentitiesOnly=yes \
             -o UserKnownHostsFile={at}/known_hosts -o GlobalKnownHostsFile=/dev/null"
        );
        // See `Site::new`: an askpass program of the developer's own is
        // never started by a test.
        if std::env::var_os("SSH_ASKPASS").is_some() {
            ssh = format!(
                "SSH_ASKPASS={} SSH_ASKPASS_REQUIRE=force {ssh}",
                helper().display()
            );
        }
        let git = site.git();
        git.run(["config", "core.sshCommand", &ssh]).unwrap();
        let url = format!(
            "ssh://{}@127.0.0.1:{}{}",
            self.user,
            self.port,
            served_path(&site.bare)
        );
        git.run(["remote", "add", "origin", &url]).unwrap();
    }

    /// The user's key as ssh names it when it asks for its passphrase:
    /// as the configuration file has it.
    pub(crate) fn key(&self) -> String {
        format!("{}/user_key", slashed(&self.dir))
    }
}

/// The path of a repository in an address of ssh: from the root, and on
/// Windows its drive after a slash (`/D:/scratch/…`).
pub(crate) fn served_path(bare: &Path) -> String {
    let path = slashed(bare);
    if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn sshd_or_skip(test: &str) -> Option<Sshd> {
    let server = Sshd::start();
    if server.is_none() {
        eprintln!("{test}: skipped, sshd, ssh or ssh-keygen is not installed");
    }
    server
}

#[test]
fn over_ssh_an_unknown_host_and_a_passphrase_are_asked_and_answered() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sshd) = sshd_or_skip("fetch and push over ssh") else {
        return;
    };
    let mut site = Site::new();
    sshd.serve(&site);
    let upstream = site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Secret if prompt.contains("passphrase") => PASSPHRASE.to_owned(),
            _ if prompt.contains("(yes/no") => "yes".to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });

    // Fetch: who is this host, then the passphrase of the key.
    assert_eq!(site.tracking(), None);
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}\n{}", sshd.log()));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    let asked = asking.asked();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0].0, Kind::Text);
    assert!(
        asked[0].1.contains("authenticity of host") && asked[0].1.contains("(yes/no"),
        "{asked:?}"
    );
    assert!(asked[0].1.contains("fingerprint"), "{asked:?}");
    assert_eq!(asked[1].0, Kind::Secret);
    // ssh names at most the first hundred bytes of the key's path
    // (`'%.100s'`), and macOS's temporary folders are that long.
    let key = sshd.key();
    let named = key.get(..key.len().min(100)).unwrap();
    assert!(
        asked[1].1.starts_with("Enter passphrase for key") && asked[1].1.contains(named),
        "{asked:?}"
    );
    // The host is known from now on, in the scratch file.
    assert!(sshd.known_hosts().contains("ssh-ed25519"));

    // Push: the passphrase again (nobody kept it), the host no more.
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local.txt");
    site.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    )
    .unwrap_or_else(|err| panic!("push: {err}\n{}", sshd.log()));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    assert_eq!(asking.count("(yes/no"), 1);
    assert_eq!(asking.count("Enter passphrase"), 2);
    // What the server saw: the key, both times.
    let log = sshd.log();
    assert_eq!(log.matches("Accepted publickey").count(), 2, "{log}");
}

#[test]
fn over_ssh_no_to_an_unknown_host_stops_before_anything_is_sent() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sshd) = sshd_or_skip("refusing a host over ssh") else {
        return;
    };
    let mut site = Site::new();
    sshd.serve(&site);
    site.upstream_commit("upstream.txt");
    let asking = Asking::with(|_, prompt| {
        assert!(
            prompt.contains("(yes/no"),
            "only the host is asked about: {prompt}"
        );
        Some("no".to_owned())
    });
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the fetch is refused");
    assert!(err.contains("Host key verification failed"), "{err}");
    // In words of the application's own too, with what can be done.
    assert!(
        err.contains("The key of the host was not accepted"),
        "{err}"
    );
    assert_eq!(asking.asked().len(), 1);
    assert_eq!(site.tracking(), None);
    assert_eq!(sshd.known_hosts(), "", "the host was not written down");
    assert!(!sshd.log().contains("Accepted publickey"));

    // And a question nobody answers is a no.
    drop(asking);
    let declined = Asking::with(|_, _| None);
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the fetch is refused");
    assert!(err.contains("Host key verification failed"), "{err}");
    assert_eq!(declined.asked().len(), 1);
    assert_eq!(sshd.known_hosts(), "");
}

#[test]
fn over_ssh_a_wrong_passphrase_is_refused_and_said() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(sshd) = sshd_or_skip("a wrong passphrase over ssh") else {
        return;
    };
    let mut site = Site::new();
    sshd.serve(&site);
    site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Secret => "not the passphrase".to_owned(),
            _ if prompt.contains("(yes/no") => "yes".to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    let started = Instant::now();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the key cannot be used");
    assert!(err.contains("Permission denied (publickey)"), "{err}");
    // ssh never says that a passphrase was wrong; the application says
    // that this is one of the two things it can have been.
    assert!(
        err.contains("The server did not accept the key") && err.contains("passphrase"),
        "{err}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "it did not hang"
    );
    // ssh asks a few times before it gives up; every time it got the same
    // wrong answer, and the server never saw the key.
    let passphrases = asking.count("Enter passphrase");
    assert!((1..=5).contains(&passphrases), "{:?}", asking.asked());
    assert_eq!(site.tracking(), None);
    assert!(!sshd.log().contains("Accepted publickey"));
    // Neither the passphrase nor the wrong one is in what is reported.
    assert!(!err.contains(PASSPHRASE) && !err.contains("not the passphrase"));

    // A push fails the same way and moves nothing.
    let before = site.served().rev_parse("refs/heads/main").unwrap();
    site.local_commit("local.txt");
    let err = site
        .run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main" }),
        )
        .expect_err("the key cannot be used");
    assert!(err.contains("Permission denied (publickey)"), "{err}");
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), before);
}

// --------------------------------------------------------------- HTTP

const USER: &str = "ada";
const TOKEN: &str = "ghp_not-a-real-token/with+signs=";

/// Base64 as RFC 4648 has it, for the one header that needs it.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, byte)| n | (u32::from(*byte) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// What one request was, for the test to look at.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Seen {
    method: String,
    target: String,
    /// The credentials it came with, as sent, when it came with any.
    authorization: Option<String>,
    let_in: bool,
}

/// A smart-HTTP server for the repositories under `root`, for one user
/// with one token.
struct Http {
    host: std::net::IpAddr,
    port: u16,
    stop: Arc<AtomicBool>,
    seen: Arc<Mutex<Vec<Seen>>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Http {
    /// `None` when git has no `http-backend` here.
    fn start(root: &Path) -> Option<Http> {
        Self::start_on(root, LOOPBACK)
    }

    /// [`Http::start`], listening on `host`.
    fn start_on(root: &Path, host: std::net::IpAddr) -> Option<Http> {
        let exec_path = Command::new("git").arg("--exec-path").output().ok()?;
        let exec_path = PathBuf::from(String::from_utf8(exec_path.stdout).ok()?.trim());
        let backend = if cfg!(windows) {
            "git-http-backend.exe"
        } else {
            "git-http-backend"
        };
        if !exec_path.join(backend).is_file() {
            return None;
        }
        let listener = TcpListener::bind((host, 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let stop = Arc::new(AtomicBool::new(false));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let thread = {
            let (stop, seen, root) = (stop.clone(), seen.clone(), root.to_path_buf());
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                    if let Ok(stream) = stream {
                        // A request that cannot be read is the client's
                        // to notice; the next one is served all the same.
                        let _ = answer(stream, &root, &seen);
                    }
                }
            })
        };
        Some(Http {
            host,
            port,
            stop,
            seen,
            thread: Some(thread),
        })
    }

    fn url(&self, bare: &Path) -> String {
        format!(
            "http://{}:{}/{}",
            self.host,
            self.port,
            bare.file_name().unwrap().to_str().unwrap()
        )
    }

    fn seen(&self) -> Vec<Seen> {
        self.seen.lock().unwrap().clone()
    }
}

impl Drop for Http {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        // Wake the thread out of its wait for a connection.
        let _ = TcpStream::connect((self.host, self.port));
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Read one request, answer it, close.
fn answer(stream: TcpStream, root: &Path, seen: &Mutex<Vec<Seen>>) -> std::io::Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut stream = stream;
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let (Some(method), Some(target)) = (parts.next(), parts.next()) else {
        return Ok(()); // the wake-up call, or nothing a request starts with
    };
    let (method, target) = (method.to_owned(), target.to_owned());
    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line)?;
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            headers.push((name.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    let header = |name: &str| {
        headers
            .iter()
            .find(|(known, _)| known == name)
            .map(|(_, value)| value.as_str())
    };
    if header("expect").is_some_and(|value| value.eq_ignore_ascii_case("100-continue")) {
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n")?;
    }
    let mut body = Vec::new();
    if header("transfer-encoding").is_some_and(|value| value.eq_ignore_ascii_case("chunked")) {
        loop {
            let mut size = String::new();
            reader.read_line(&mut size)?;
            let size = usize::from_str_radix(size.trim().split(';').next().unwrap_or("0"), 16)
                .map_err(std::io::Error::other)?;
            let mut chunk = vec![0u8; size];
            reader.read_exact(&mut chunk)?;
            body.extend_from_slice(&chunk);
            let mut end = String::new();
            reader.read_line(&mut end)?;
            if size == 0 {
                break;
            }
        }
    } else if let Some(length) = header("content-length").and_then(|value| value.parse().ok()) {
        body = vec![0u8; length];
        reader.read_exact(&mut body)?;
    }

    let wanted = format!("Basic {}", base64(format!("{USER}:{TOKEN}").as_bytes()));
    let authorization = header("authorization").map(str::to_owned);
    let let_in = authorization.as_deref() == Some(wanted.as_str());
    seen.lock().unwrap().push(Seen {
        method: method.clone(),
        target: target.clone(),
        authorization,
        let_in,
    });
    if !let_in {
        return stream.write_all(
            b"HTTP/1.1 401 Unauthorized\r\nWWW-Authenticate: Basic realm=\"bitbraid test\"\r\n\
              Content-Length: 0\r\nConnection: close\r\n\r\n",
        );
    }

    // Let in: git's own program does the rest, as a CGI program does.
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    let mut cgi = Git::new(root, fixtures_out_dir().join(".home")).command();
    cgi.arg("http-backend")
        .env("GIT_PROJECT_ROOT", root)
        .env("GIT_HTTP_EXPORT_ALL", "1")
        .env("REQUEST_METHOD", &method)
        .env("PATH_INFO", path)
        .env("QUERY_STRING", query)
        .env("REMOTE_USER", USER)
        .env("REMOTE_ADDR", "127.0.0.1")
        .env("CONTENT_LENGTH", body.len().to_string())
        .env("CONTENT_TYPE", header("content-type").unwrap_or_default())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(encoding) = header("content-encoding") {
        cgi.env("HTTP_CONTENT_ENCODING", encoding);
    }
    if let Some(protocol) = header("git-protocol") {
        cgi.env("GIT_PROTOCOL", protocol);
    }
    let mut child = cgi.spawn()?;
    let mut stdin = child.stdin.take().expect("piped");
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&body);
    });
    let out = child.wait_with_output()?;
    let _ = writer.join();
    let split = out
        .stdout
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|at| (at, at + 4))
        .or_else(|| {
            out.stdout
                .windows(2)
                .position(|window| window == b"\n\n")
                .map(|at| (at, at + 2))
        });
    let Some((head_end, body_start)) = split else {
        return stream.write_all(
            b"HTTP/1.1 500 No Answer\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        );
    };
    let head = String::from_utf8_lossy(&out.stdout[..head_end]).into_owned();
    let payload = &out.stdout[body_start..];
    let mut status = "200 OK".to_owned();
    let mut response = String::new();
    for line in head.lines() {
        match line.split_once(':') {
            Some((name, value)) if name.eq_ignore_ascii_case("status") => {
                status = value.trim().to_owned();
            }
            _ => {
                response.push_str(line.trim_end());
                response.push_str("\r\n");
            }
        }
    }
    stream.write_all(
        format!(
            "HTTP/1.1 {status}\r\n{response}Content-Length: {}\r\nConnection: close\r\n\r\n",
            payload.len()
        )
        .as_bytes(),
    )?;
    stream.write_all(payload)
}

/// The server, with the scratch repository pointed at it.
fn http_or_skip(test: &str, site: &Site) -> Option<Http> {
    // An askpass program for git alone in the developer's environment
    // comes before everything a test can set, and would be asked on their
    // screen.
    if std::env::var_os("GIT_ASKPASS").is_some() {
        eprintln!("{test}: skipped, GIT_ASKPASS is set and would be asked instead");
        return None;
    }
    let Some(http) = Http::start(site.bare.parent().unwrap()) else {
        eprintln!("{test}: skipped, git has no http-backend here");
        return None;
    };
    assert!(listening(http.port, Duration::from_secs(5)));
    site.git()
        .run(["remote", "add", "origin", &http.url(&site.bare)])
        .unwrap();
    Some(http)
}

#[test]
fn over_http_the_user_and_the_token_are_asked_and_answered_and_not_kept() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(http) = http_or_skip("fetch and push over http", &site) else {
        return;
    };
    let upstream = site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Text if prompt.starts_with("Username for") => USER.to_owned(),
            Kind::Secret if prompt.starts_with("Password for") => TOKEN.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    let host = format!("http://127.0.0.1:{}", http.port);

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
    let asked = asking.asked();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0].0, Kind::Text);
    assert!(
        asked[0].1.starts_with(&format!("Username for '{host}'")),
        "{asked:?}"
    );
    assert_eq!(asked[1].0, Kind::Secret);
    assert!(
        asked[1].1.starts_with(&format!(
            "Password for 'http://{USER}@127.0.0.1:{}'",
            http.port
        )),
        "{asked:?}"
    );
    // The server turned the first request away and let the rest in.
    let seen = http.seen();
    assert!(seen.len() >= 3, "{seen:?}");
    assert_eq!(
        (seen[0].authorization.as_deref(), seen[0].let_in),
        (None, false)
    );
    assert!(seen[1..].iter().all(|request| request.let_in), "{seen:?}");
    assert!(
        seen.iter()
            .any(|request| request.target.contains("git-upload-pack"))
    );

    // Push: asked again, because nothing was kept anywhere.
    site.git()
        .run(["merge", "-q", "--ff-only", "origin/main"])
        .unwrap();
    let local = site.local_commit("local.txt");
    site.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    )
    .unwrap_or_else(|err| panic!("push: {err}"));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    assert_eq!(asking.count("Username for"), 2);
    assert_eq!(asking.count("Password for"), 2);
    assert!(
        http.seen()
            .iter()
            .any(|request| request.method == "POST" && request.target.contains("git-receive-pack")),
    );
    // Not in the repository's configuration, and not in the remote's URL.
    let config = std::fs::read_to_string(site.copy.path().join(".git/config")).unwrap();
    assert!(!config.contains(TOKEN), "{config}");
    assert_eq!(
        site.git()
            .run(["remote", "get-url", "origin"])
            .unwrap()
            .trim(),
        http.url(&site.bare)
    );
    assert!(!site.copy.path().join(".git/credentials").exists());
    // And a third time for a fetch that has nothing to bring.
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap();
    assert_eq!(asking.count("Password for"), 3);
}

#[test]
fn over_http_a_wrong_token_is_refused_and_said() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(http) = http_or_skip("a wrong token over http", &site) else {
        return;
    };
    site.upstream_commit("upstream.txt");
    let asking = Asking::with(|kind, _| {
        Some(match kind {
            Kind::Text => USER.to_owned(),
            _ => "a-token-that-was-revoked".to_owned(),
        })
    });
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the token is wrong");
    assert!(err.contains("Authentication failed"), "{err}");
    assert!(err.contains(&format!("127.0.0.1:{}", http.port)), "{err}");
    assert!(
        err.contains("The server refused the user name and the password or token"),
        "{err}"
    );
    // What was typed is not repeated in what is reported.
    assert!(!err.contains("a-token-that-was-revoked"), "{err}");
    assert_eq!(asking.asked().len(), 2, "asked once, not again and again");
    assert_eq!(site.tracking(), None);
    let seen = http.seen();
    assert!(seen.iter().all(|request| !request.let_in), "{seen:?}");
    assert!(seen.iter().any(|request| request.authorization.is_some()));

    // A push with it moves nothing on the server.
    let before = site.served().rev_parse("refs/heads/main").unwrap();
    site.local_commit("local.txt");
    let err = site
        .run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main" }),
        )
        .expect_err("the token is wrong");
    assert!(err.contains("Authentication failed"), "{err}");
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), before);
}

#[test]
fn over_http_a_question_nobody_answers_ends_the_command() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(http) = http_or_skip("declining over http", &site) else {
        return;
    };
    site.upstream_commit("upstream.txt");
    // The dialog is closed: no user name.
    let asking = Asking::with(|_, _| None);
    let started = Instant::now();
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("nobody said who");
    assert!(
        started.elapsed() < Duration::from_secs(60),
        "it did not hang"
    );
    assert!(err.contains("could not read Username"), "{err}");
    assert!(
        err.contains("The question for the user name was not answered"),
        "{err}"
    );
    assert_eq!(asking.asked().len(), 1, "{:?}", asking.asked());
    assert_eq!(site.tracking(), None);
    assert!(http.seen().iter().all(|request| !request.let_in));
    drop(asking);

    // Without an application to ask (no askpass server at all) git is
    // told there is no terminal either, and says so at once.
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("nobody to ask");
    assert!(
        err.contains("terminal prompts disabled") || err.contains("could not read Username"),
        "{err}"
    );
}
