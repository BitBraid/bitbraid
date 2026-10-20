//! Credential helpers that keep what they are told, with the engine's
//! fetch and push against the smart-HTTP server of the module above: git's
//! `store` (a file), its `cache` (a daemon behind a socket), its
//! `libsecret` helper (the desktop's Secret Service, here one of the
//! test's own), its `osxkeychain` helper (macOS's keychain, here one of
//! the test's own), and its `wincred` helper and Git Credential Manager
//! (the user's own Credential Manager, under names of the test's own).
//! Git asks a helper before anyone else, tells it what the server took,
//! and tells it to forget what the server refused; the application's
//! dialog comes last, and what is typed there reaches the helper through
//! git.
//!
//! The cases, for every helper:
//! - nothing kept: a fetch with nobody to ask (a timer's) is refused at
//!   once, and asks nobody;
//! - the first push asks in the dialog, and the helper keeps the answer;
//! - the next push, a fetch and a fetch with nobody to ask ask nothing;
//! - a kept token that has gone wrong (revoked at the server): the push
//!   is refused without a question, git takes the token out of the helper,
//!   and the push after that asks again and has the new answer kept;
//! - a wrong answer typed in the dialog is refused and never kept.
//!
//! Which comes first, with the `store` helper alone: a helper that has an
//! answer before any question, the first of several helpers that has one
//! before the others, every helper told what was taken and told to forget
//! what was refused (which a recent git has it do only where the token
//! matches); an askpass program of the user's own (`core.askPass`) before
//! the application's, with its answer kept; over ssh, an agent that holds
//! the key before the question for its passphrase.
//!
//! Everything a helper keeps is in a scratch directory. The throwaway
//! Secret Service is `gnome-keyring-daemon` on a session bus of its own,
//! started by `dbus-run-session` with a configuration that listens on a
//! socket in the scratch directory and starts no services: its keyrings,
//! home and runtime directory are there, its environment has nothing of
//! the developer's session (no bus address, no display), its collection is
//! made and unlocked with a password on its standard input, and the helper
//! is named that bus's address explicitly, so that a bus that is gone
//! fails rather than falls back to the developer's. Where `dbus-run-session`,
//! `gnome-keyring-daemon`, `dbus-send` or git's libsecret helper is not
//! installed (macOS, Windows, a hosted runner), that test says so and
//! passes without having run; so does the `cache` one where git has no
//! `credential-cache` (Git for Windows).
//!
//! The throwaway keychain of macOS is a file in a scratch home, made with
//! `security create-keychain` and a password of the test's, and left as it
//! is made, unlocked, with no locking after a while: the keychains of the
//! system's framework are found from `HOME`, and in a home with no list of
//! its own the default one is `Library/Keychains/login.keychain-db` there,
//! which is where it is made. The helper is given that home in its own
//! command, as the libsecret one is given its bus, and the test makes sure
//! the scratch keychain is the default one before anything is kept, and
//! that the user's own keychains never had the item. Off macOS, or where
//! git has no osxkeychain helper, that test says so and passes without
//! having run; so do the Credential Manager ones where git has no such
//! helper.
//!
//! Sources: gitcredentials(7) (the helpers, the order they are asked in,
//! an empty value emptying the list), git-credential(1) (the format;
//! `get`, `store`, `erase`), git-credential-store(1) (`--file`),
//! git-credential-cache(1) (`--socket`, `exit`), git-config(1)
//! (`credential.helper`, `core.askPass`, `core.sshCommand`), the
//! freedesktop.org Secret Service API (collections, the `default` alias,
//! `org.freedesktop.secrets`), gnome-keyring-daemon(1) (`--foreground`,
//! `--components`, `--unlock`, `--control-directory`), dbus-run-session(1)
//! (`--config-file`), dbus-daemon(1) (the configuration file: `listen`,
//! `policy`, no `servicedir`), dbus-send(1), the D-Bus specification
//! (`NameHasOwner`), ssh-agent(1) (`-a`, `-D`), ssh-add(1) (`SSH_ASKPASS`,
//! `SSH_ASKPASS_REQUIRE`), ssh_config(5) (`IdentityAgent`), security(1)
//! (`create-keychain`, `set-keychain-settings`, `default-keychain`,
//! `delete-keychain`, `find-internet-password`), Apple's Keychain Services
//! documentation (keychain search lists, the default keychain).
#![allow(clippy::print_stderr)]

use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::Ordering,
};

use bitbraid_git::askpass::Kind;
use bitbraid_testkit::{Git, fixtures_out_dir, scratch_root};
use serde_json::json;

use super::{COUNTER, ONE, Site, TOKEN, USER, http_or_skip};

/// A token the server once took and takes no more.
const REVOKED: &str = "ghp_a-token-that-was-revoked";

/// Where git keeps its own programs (`git --exec-path`).
fn exec_path() -> Option<PathBuf> {
    let out = Command::new("git").arg("--exec-path").output().ok()?;
    Some(PathBuf::from(String::from_utf8(out.stdout).ok()?.trim()))
}

/// Whether `program` is on `PATH`.
pub(super) fn on_path(program: &str) -> bool {
    std::env::var_os("PATH")
        .map(|path| std::env::split_paths(&path).collect::<Vec<_>>())
        .unwrap_or_default()
        .iter()
        .any(|dir| dir.join(program).is_file())
}

/// A scratch directory of its own.
pub(super) fn fresh_dir(what: &str) -> PathBuf {
    let dir = scratch_root().join(format!(
        "{what}-{}-{}",
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Quote for `sh`, as git hands a helper's command to it.
fn quoted(path: &Path) -> String {
    let text = path.display().to_string();
    assert!(!text.contains('\''), "a scratch path with a quote: {text}");
    format!("'{text}'")
}

// ------------------------------------------------------------ helpers

/// Where one helper keeps what it is told.
enum Keeper {
    /// `git credential-store --file <file>`.
    Store(PathBuf),
    /// `git credential-cache --socket <socket>`: a daemon of git's.
    Cache(PathBuf),
    /// git's libsecret helper on the bus of a throwaway keyring.
    Libsecret {
        program: PathBuf,
        keyring: Box<keyring::Keyring>,
    },
    /// git's osxkeychain helper with a throwaway keychain for its default.
    Osxkeychain {
        program: PathBuf,
        keychain: Box<keychain::Keychain>,
    },
    /// git's wincred helper: Credential Manager, which is the user's own
    /// on Windows; what it keeps is under a path of the test's own.
    Wincred(PathBuf),
    /// Git Credential Manager, keeping in Credential Manager the same way,
    /// and never asking anyone itself ([`MANAGER_ALONE`]).
    Manager(PathBuf),
}

/// What Git Credential Manager is told so that it never asks anyone (no
/// window and no terminal: the dialog is the application's), takes the
/// test's server for a plain one without asking it how it signs in, lets
/// it be plain HTTP, and keeps what it is told in Credential Manager.
const MANAGER_ALONE: [(&str, &str); 6] = [
    ("GCM_INTERACTIVE", "never"),
    ("GCM_GUI_PROMPT", "false"),
    ("GCM_PROVIDER", "generic"),
    ("GCM_ALLOW_WINDOWSAUTH", "false"),
    ("GCM_ALLOW_UNSAFE_REMOTES", "true"),
    ("GCM_CREDENTIAL_STORE", "wincredman"),
];

/// A credential helper that keeps what git tells it, in scratch.
pub(super) struct Helper {
    /// The value of `credential.helper` that names it.
    config: String,
    keeper: Keeper,
    dir: PathBuf,
    /// For a helper that keeps in the user's own Credential Manager: the
    /// path of the repository it keeps for (git is told to hand it over,
    /// `credential.useHttpPath`), which no credential of the user's has,
    /// and the hosts it was asked about, whose credentials of that path
    /// are erased when it is done.
    path: std::cell::RefCell<Option<String>>,
    hosts: std::cell::RefCell<Vec<String>>,
}

impl Helper {
    fn store() -> Self {
        let dir = fresh_dir("credential-store");
        let file = dir.join("credentials");
        Self {
            config: format!("store --file {}", quoted(&file)),
            keeper: Keeper::Store(file),
            dir,
            path: Default::default(),
            hosts: Default::default(),
        }
    }

    /// `None`, with a message, where git has no `credential-cache`.
    fn cache_or_skip(test: &str) -> Option<Self> {
        let have = exec_path().is_some_and(|dir| dir.join("git-credential-cache").is_file());
        if !have {
            eprintln!("{test}: skipped, git has no credential-cache here");
            return None;
        }
        let dir = fresh_dir("credential-cache");
        // In a folder that is not there yet: the daemon makes it, for
        // this user alone, and refuses one that others may enter.
        let socket = dir.join("cache").join("socket");
        Some(Self {
            config: format!("cache --timeout=300 --socket {}", quoted(&socket)),
            keeper: Keeper::Cache(socket),
            dir,
            path: Default::default(),
            hosts: Default::default(),
        })
    }

    /// `None`, with a message, where there is no throwaway keyring to be
    /// had or git has no libsecret helper.
    pub(super) fn libsecret_or_skip(test: &str) -> Option<Self> {
        let program = exec_path()
            .map(|dir| dir.join("git-credential-libsecret"))
            .filter(|program| program.is_file());
        let Some(program) = program else {
            eprintln!("{test}: skipped, git has no libsecret helper here");
            return None;
        };
        let keyring = keyring::Keyring::start(test)?;
        // The bus is named in the helper's own command: whatever the
        // environment says, the developer's keyring is never asked.
        let config = format!(
            "!DBUS_SESSION_BUS_ADDRESS={} {}",
            quoted_text(&keyring.address()),
            quoted(&program)
        );
        let dir = fresh_dir("credential-libsecret");
        Some(Self {
            config,
            keeper: Keeper::Libsecret {
                program,
                keyring: Box::new(keyring),
            },
            dir,
            path: Default::default(),
            hosts: Default::default(),
        })
    }

    /// `None`, with a message, off macOS or where git has no osxkeychain
    /// helper.
    fn osxkeychain_or_skip(test: &str) -> Option<Self> {
        let program = exec_path()
            .map(|dir| dir.join("git-credential-osxkeychain"))
            .filter(|program| program.is_file());
        let Some(program) = program else {
            eprintln!("{test}: skipped, git has no osxkeychain helper here");
            return None;
        };
        let keychain = keychain::Keychain::start(test)?;
        // The home is named in the helper's own command: whatever git's
        // environment says, the user's keychains are never asked.
        let config = format!("!HOME={} {}", quoted(&keychain.home()), quoted(&program));
        let dir = fresh_dir("credential-osxkeychain");
        Some(Self {
            config,
            keeper: Keeper::Osxkeychain {
                program,
                keychain: Box::new(keychain),
            },
            dir,
            path: Default::default(),
            hosts: Default::default(),
        })
    }

    /// `None`, with a message, where git has no wincred helper (off
    /// Windows).
    fn wincred_or_skip(test: &str) -> Option<Self> {
        let program = exec_path()
            .map(|dir| dir.join("git-credential-wincred.exe"))
            .filter(|program| program.is_file());
        let Some(program) = program else {
            eprintln!("{test}: skipped, git has no wincred helper here");
            return None;
        };
        Some(Self {
            config: format!("!{}", quoted(&program)),
            keeper: Keeper::Wincred(program),
            dir: fresh_dir("credential-wincred"),
            path: Default::default(),
            hosts: Default::default(),
        })
    }

    /// `None`, with a message, where there is no Git Credential Manager
    /// beside git: Git for Windows has it in `mingw64/bin`.
    fn manager_or_skip(test: &str) -> Option<Self> {
        let name = if cfg!(windows) {
            "git-credential-manager.exe"
        } else {
            "git-credential-manager"
        };
        let program = exec_path()
            .into_iter()
            .flat_map(|dir| {
                let beside = dir
                    .parent()
                    .and_then(Path::parent)
                    .map(|prefix| prefix.join("bin").join(name));
                [Some(dir.join(name)), beside]
            })
            .flatten()
            .find(|program| program.is_file());
        let Some(program) = program else {
            eprintln!("{test}: skipped, there is no Git Credential Manager beside git here");
            return None;
        };
        let settings: Vec<String> = MANAGER_ALONE
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect();
        Some(Self {
            config: format!("!{} {}", settings.join(" "), quoted(&program)),
            keeper: Keeper::Manager(program),
            dir: fresh_dir("credential-manager"),
            path: Default::default(),
            hosts: Default::default(),
        })
    }

    /// Whether it keeps in the user's own Credential Manager.
    fn in_credential_manager(&self) -> bool {
        matches!(self.keeper, Keeper::Wincred(_) | Keeper::Manager(_))
    }

    /// Make it the repository's one helper, after the list was emptied
    /// (`Site::new`).
    fn add_to(&self, site: &Site) {
        if self.in_credential_manager() {
            site.git()
                .run(["config", "credential.useHttpPath", "true"])
                .unwrap();
            let name = site.bare.file_name().unwrap().to_str().unwrap();
            *self.path.borrow_mut() = Some(name.to_owned());
        }
        self.add_to_git(&site.git());
    }

    /// The same for the repository `git` runs in, whose list of helpers
    /// was emptied.
    pub(super) fn add_to_git(&self, git: &Git) {
        git.run(["config", "--add", "credential.helper", &self.config])
            .unwrap();
    }

    /// Run one of its actions, as git would, with `input` on its standard
    /// input; what it printed.
    fn action(&self, op: &str, input: &str) -> String {
        let (done, out, err) = self.try_action(op, input);
        // Git Credential Manager fails where it has nothing and may not
        // ask; git takes that for no answer, as it is here.
        let nothing = matches!(self.keeper, Keeper::Manager(_)) && op == "get";
        assert!(done || nothing, "the helper's {op}: {err}");
        if done { out } else { String::new() }
    }

    /// Run one of its actions; whether it succeeded, what it printed and
    /// what it said on its error output.
    fn try_action(&self, op: &str, input: &str) -> (bool, String, String) {
        let mut cmd = match &self.keeper {
            Keeper::Store(file) => {
                let mut cmd = Git::new(&self.dir, fixtures_out_dir().join(".home")).command();
                cmd.arg("credential-store").arg("--file").arg(file);
                cmd
            }
            Keeper::Cache(socket) => {
                let mut cmd = Git::new(&self.dir, fixtures_out_dir().join(".home")).command();
                cmd.arg("credential-cache").arg("--socket").arg(socket);
                cmd
            }
            Keeper::Libsecret { program, keyring } => {
                let mut cmd = Command::new(program);
                cmd.env("DBUS_SESSION_BUS_ADDRESS", keyring.address());
                cmd
            }
            Keeper::Osxkeychain { program, keychain } => {
                let mut cmd = Command::new(program);
                cmd.env("HOME", keychain.home());
                cmd
            }
            Keeper::Wincred(program) => Command::new(program),
            Keeper::Manager(program) => {
                let mut cmd = Command::new(program);
                cmd.envs(MANAGER_ALONE);
                cmd
            }
        };
        cmd.arg(op)
            .current_dir(&self.dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        (
            out.status.success(),
            String::from_utf8(out.stdout).unwrap(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    /// The lines that say which credential is meant: the protocol, the
    /// host, and the repository's path where the helper is told it.
    fn which(&self, protocol: &str, host: &str) -> String {
        if self.in_credential_manager() {
            let mut hosts = self.hosts.borrow_mut();
            if !hosts.iter().any(|known| known == host) {
                hosts.push(host.to_owned());
            }
        }
        match self.path.borrow().as_deref() {
            Some(path) => format!("protocol={protocol}\nhost={host}\npath={path}\n"),
            None => format!("protocol={protocol}\nhost={host}\n"),
        }
    }

    /// Whether Windows' own `cmdkey` lists a credential of the path it
    /// keeps for at `host`, under either name git's helpers give one
    /// (with the user name before the host, or without): only names of
    /// the test's own are asked for.
    #[cfg(windows)]
    fn listed(&self, host: &str) -> bool {
        let Some(path) = self.path.borrow().clone() else {
            return false;
        };
        [
            format!("git:http://{host}/{path}"),
            format!("git:http://{USER}@{host}/{path}"),
        ]
        .iter()
        .any(|target| {
            let out = Command::new("cmdkey")
                .arg(format!("/list:{target}"))
                .output()
                .unwrap();
            assert!(out.status.success(), "cmdkey /list:{target}");
            String::from_utf8_lossy(&out.stdout)
                .lines()
                .any(|line| line.trim_end().ends_with(&format!(" {target}")))
        })
    }

    /// The user name and the password it keeps for `host` over plain
    /// HTTP, asked as git asks it.
    fn kept(&self, host: &str) -> Option<(String, String)> {
        self.kept_at("http", host)
    }

    /// The user name and the password it keeps for `host` over
    /// `protocol`, asked as git asks it.
    pub(super) fn kept_at(&self, protocol: &str, host: &str) -> Option<(String, String)> {
        let said = self.action("get", &format!("{}\n", self.which(protocol, host)));
        let field = |name: &str| {
            said.lines()
                .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
                .map(str::to_owned)
        };
        Some((field("username")?, field("password")?))
    }

    /// Have it keep `password` for `user` at `host` over plain HTTP, as
    /// git has it keep what a server took.
    fn keep(&self, host: &str, user: &str, password: &str) {
        self.keep_at("http", host, user, password);
    }

    /// The same over `protocol`. What it keeps is compared, never shown:
    /// the live tests keep real tokens.
    pub(super) fn keep_at(&self, protocol: &str, host: &str, user: &str, password: &str) {
        self.action(
            "store",
            &format!(
                "{}username={user}\npassword={password}\n\n",
                self.which(protocol, host)
            ),
        );
        assert!(
            self.kept_at(protocol, host) == Some((user.to_owned(), password.to_owned())),
            "the helper does not keep what it was given"
        );
    }

    /// Have it forget what it keeps for `user` at `host` over `protocol`.
    pub(super) fn forget_at(&self, protocol: &str, host: &str, user: &str) {
        self.action(
            "erase",
            &format!("{}username={user}\n\n", self.which(protocol, host)),
        );
    }
}

/// Quote any text for `sh`.
fn quoted_text(text: &str) -> String {
    assert!(!text.contains('\''), "a quote in {text}");
    format!("'{text}'")
}

impl Drop for Helper {
    fn drop(&mut self) {
        // What the user's Credential Manager holds of the test's, gone
        // whether the test passed or not.
        if self.in_credential_manager() {
            let hosts = self.hosts.borrow().clone();
            for host in hosts {
                let which = self.which("http", &host);
                let _ = self.try_action("erase", &format!("{which}\n"));
            }
        }
        if let Keeper::Cache(socket) = &self.keeper {
            // The daemon would wait out its timeout otherwise.
            let _ = Git::new(&self.dir, fixtures_out_dir().join(".home"))
                .command()
                .arg("credential-cache")
                .arg("--socket")
                .arg(socket)
                .arg("exit")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// ----------------------------------------------------- the throwaway keyring

#[cfg(unix)]
mod keyring {
    use std::{
        io::Write,
        os::unix::{fs::PermissionsExt, process::CommandExt},
        path::PathBuf,
        process::{Child, Command, Stdio},
        time::{Duration, Instant},
    };

    use super::{fresh_dir, on_path};

    /// A Secret Service of the test's own: `gnome-keyring-daemon` on a
    /// session bus of its own, all of it in a scratch directory, and all of
    /// it gone when this is.
    pub struct Keyring {
        dir: PathBuf,
        /// `dbus-run-session`, the leader of a process group of its own
        /// that the bus and the keyring daemon are in.
        child: Child,
    }

    impl Keyring {
        /// Start one, or say why not: `None` where a program is missing.
        pub fn start(test: &str) -> Option<Self> {
            for program in [
                "dbus-run-session",
                "gnome-keyring-daemon",
                "dbus-send",
                "kill",
            ] {
                if !on_path(program) {
                    eprintln!("{test}: skipped, {program} is not installed");
                    return None;
                }
            }
            let dir = fresh_dir("keyring");
            for sub in ["home", "data", "config", "cache", "run"] {
                std::fs::create_dir_all(dir.join(sub)).unwrap();
            }
            std::fs::set_permissions(dir.join("run"), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            // A session bus on a socket in the scratch directory, which
            // starts no service of the system's on demand.
            std::fs::write(
                dir.join("session.conf"),
                format!(
                    "<!DOCTYPE busconfig PUBLIC \"-//freedesktop//DTD D-Bus Bus Configuration 1.0//EN\"\n \
                     \"http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd\">\n\
                     <busconfig>\n  <type>session</type>\n  <listen>unix:path={}/bus</listen>\n  \
                     <auth>EXTERNAL</auth>\n  <policy context=\"default\">\n    \
                     <allow send_destination=\"*\" eavesdrop=\"true\"/>\n    \
                     <allow eavesdrop=\"true\"/>\n    <allow own=\"*\"/>\n  </policy>\n</busconfig>\n",
                    dir.display()
                ),
            )
            .unwrap();
            let log = std::fs::File::create(dir.join("daemon.log")).unwrap();
            let mut child = Command::new("dbus-run-session")
                .arg(format!(
                    "--config-file={}",
                    dir.join("session.conf").display()
                ))
                .args([
                    "--",
                    "gnome-keyring-daemon",
                    "--foreground",
                    "--components=secrets",
                    "--unlock",
                ])
                .arg(format!(
                    "--control-directory={}",
                    dir.join("run").join("keyring").display()
                ))
                // Nothing of the developer's session: no bus address, no
                // display, no runtime directory, no home.
                .env_clear()
                .env("PATH", std::env::var_os("PATH").unwrap_or_default())
                .env("HOME", dir.join("home"))
                .env("XDG_DATA_HOME", dir.join("data"))
                .env("XDG_CONFIG_HOME", dir.join("config"))
                .env("XDG_CACHE_HOME", dir.join("cache"))
                .env("XDG_RUNTIME_DIR", dir.join("run"))
                .process_group(0)
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .unwrap();
            // The collection is made and unlocked with this password, read
            // up to the end of the input.
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(b"a throwaway password").unwrap();
            }
            let keyring = Self { dir, child };
            let started = Instant::now();
            while !keyring.ready() {
                assert!(
                    started.elapsed() < Duration::from_secs(15),
                    "the throwaway keyring did not come up:\n{}",
                    std::fs::read_to_string(keyring.dir.join("daemon.log")).unwrap_or_default()
                );
                std::thread::sleep(Duration::from_millis(50));
            }
            Some(keyring)
        }

        /// The address of its bus.
        pub fn address(&self) -> String {
            format!("unix:path={}/bus", self.dir.display())
        }

        /// The keyrings it writes, all in the scratch directory.
        pub fn files(&self) -> PathBuf {
            self.dir.join("data").join("keyrings")
        }

        /// Whether the collection is made and the service answers on the bus.
        fn ready(&self) -> bool {
            if !self.files().join("login.keyring").is_file() {
                return false;
            }
            Command::new("dbus-send")
                .arg(format!("--bus={}", self.address()))
                .args([
                    "--print-reply",
                    "--dest=org.freedesktop.DBus",
                    "/",
                    "org.freedesktop.DBus.NameHasOwner",
                    "string:org.freedesktop.secrets",
                ])
                .env_remove("DBUS_SESSION_BUS_ADDRESS")
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .output()
                .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).contains("boolean true"))
        }
    }

    impl Drop for Keyring {
        fn drop(&mut self) {
            // The whole group: dbus-run-session leaves the bus and the
            // daemon behind when it is the only one stopped.
            let _ = Command::new("kill")
                .args(["-TERM", "--", &format!("-{}", self.child.id())])
                .stdin(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }
}

// ---------------------------------------------------- the throwaway keychain

#[cfg(target_os = "macos")]
mod keychain {
    use std::{
        path::{Path, PathBuf},
        process::{Command, Stdio},
    };

    use super::{fresh_dir, on_path};

    /// The password the throwaway keychain is made with.
    const PASSWORD: &str = "a throwaway password";

    /// A keychain of the test's own: the default keychain of a scratch
    /// home, and gone when this is.
    pub struct Keychain {
        dir: PathBuf,
    }

    impl Keychain {
        /// Make one and unlock it, or say why not: `None` where there is
        /// no `security`.
        pub fn start(test: &str) -> Option<Self> {
            if !on_path("security") {
                eprintln!("{test}: skipped, security is not installed");
                return None;
            }
            let keychain = Self {
                dir: fresh_dir("keychain"),
            };
            let folder = keychain.home().join("Library").join("Keychains");
            std::fs::create_dir_all(&folder).unwrap();
            // Made as `login.keychain`, it is the file the framework takes
            // for the default one of a home that names none.
            keychain.security(&[
                "create-keychain",
                "-p",
                PASSWORD,
                folder.join("login.keychain").to_str().unwrap(),
            ]);
            let file = keychain.file();
            assert!(file.is_file(), "no keychain at {}", file.display());
            let file = file.to_str().unwrap();
            // It is made unlocked; and is not to lock after a while or on
            // sleep. (Unlocked again, by its password, it is not: the
            // system takes a keychain of that name for the user's own.)
            keychain.security(&["set-keychain-settings", file]);
            // It is the default one there, and nothing else is: a system
            // that ignored the home would have the user's own instead.
            let default = keychain.security(&["default-keychain"]);
            let named = default.trim().trim_matches('"');
            assert!(
                Path::new(named).ends_with("Library/Keychains/login.keychain-db")
                    && std::fs::canonicalize(named).ok() == std::fs::canonicalize(file).ok(),
                "the default keychain of the scratch home is not the scratch one: {default}"
            );
            Some(keychain)
        }

        /// The home whose default keychain it is.
        pub fn home(&self) -> PathBuf {
            self.dir.join("home")
        }

        /// The keychain's file.
        pub fn file(&self) -> PathBuf {
            self.home()
                .join("Library")
                .join("Keychains")
                .join("login.keychain-db")
        }

        /// Run `security` in the scratch home; what it printed.
        fn security(&self, args: &[&str]) -> String {
            let out = Command::new("security")
                .args(args)
                .env("HOME", self.home())
                .stdin(Stdio::null())
                .output()
                .unwrap();
            assert!(
                out.status.success(),
                "security {}: {}",
                args.first().copied().unwrap_or_default(),
                String::from_utf8_lossy(&out.stderr)
            );
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    }

    impl Drop for Keychain {
        fn drop(&mut self) {
            let _ = Command::new("security")
                .arg("delete-keychain")
                .arg(self.file())
                .env("HOME", self.home())
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    /// Whether any of the user's own keychains (their real home's search
    /// list) has an internet password for `server`. Only what is said of
    /// an item is asked, never its secret, so nothing is shown and no
    /// dialog comes up.
    pub fn users_keychains_have(server: &str, port: u16) -> bool {
        Command::new("security")
            .args(["find-internet-password", "-s", server, "-P"])
            .arg(port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    }
}

#[cfg(not(target_os = "macos"))]
mod keychain {
    use std::path::PathBuf;

    /// No keychain of macOS elsewhere.
    pub struct Keychain;

    impl Keychain {
        pub fn start(test: &str) -> Option<Self> {
            eprintln!("{test}: skipped, the keychain is macOS's");
            None
        }

        pub fn home(&self) -> PathBuf {
            PathBuf::new()
        }

        pub fn file(&self) -> PathBuf {
            PathBuf::new()
        }
    }

    pub fn users_keychains_have(_server: &str, _port: u16) -> bool {
        false
    }
}

#[cfg(not(unix))]
mod keyring {
    use std::path::PathBuf;

    /// No throwaway Secret Service off Unix.
    pub struct Keyring;

    impl Keyring {
        pub fn start(test: &str) -> Option<Self> {
            eprintln!("{test}: skipped, there is no Secret Service on this system");
            None
        }

        pub fn address(&self) -> String {
            String::new()
        }

        pub fn files(&self) -> PathBuf {
            PathBuf::new()
        }
    }
}

// --------------------------------------------------------------- cases

/// The whole life of a kept token, through one helper: see the module's
/// documentation. The port of the server it ran against, unless it was
/// skipped.
fn asked_once_kept_refused_forgotten_and_asked_again(test: &str, helper: &Helper) -> Option<u16> {
    let mut site = Site::new();
    let http = http_or_skip(test, &site)?;
    helper.add_to(&site);
    let host = format!("127.0.0.1:{}", http.port);
    let asking = super::Asking::with(|kind, prompt| {
        Some(match kind {
            Kind::Text if prompt.starts_with("Username for") => USER.to_owned(),
            Kind::Secret if prompt.starts_with("Password for") => TOKEN.to_owned(),
            _ => panic!("an unexpected question: {prompt}"),
        })
    });
    let served = |site: &Site| site.served().rev_parse("refs/heads/main").unwrap();
    let push = |site: &mut Site| {
        site.run(
            "remote.push",
            json!({ "remote": "origin", "branch": "main" }),
        )
    };

    // Nothing kept yet: a fetch with nobody to ask is refused at once.
    assert_eq!(helper.kept(&host), None);
    let err = site
        .run("remote.fetch", json!({ "remote": "origin", "quiet": true }))
        .expect_err("nothing is kept and nobody may be asked");
    assert!(err.contains("wants an answer"), "{err}");
    assert!(asking.asked().is_empty(), "{:?}", asking.asked());

    // The first push asks in the dialog, and the helper keeps the answer.
    let local = site.local_commit("first.txt");
    push(&mut site).unwrap_or_else(|err| panic!("the first push: {err}"));
    assert_eq!(served(&site), local);
    assert_eq!(asking.count("Username for"), 1, "{:?}", asking.asked());
    assert_eq!(asking.count("Password for"), 1, "{:?}", asking.asked());
    assert_eq!(
        helper.kept(&host),
        Some((USER.to_owned(), TOKEN.to_owned()))
    );

    // The next push asks nothing: the helper answers. Nor does a fetch,
    // nor one with nobody to ask.
    let local = site.local_commit("second.txt");
    push(&mut site).unwrap_or_else(|err| panic!("the second push: {err}"));
    assert_eq!(served(&site), local);
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("a fetch: {err}"));
    site.run("remote.fetch", json!({ "remote": "origin", "quiet": true }))
        .unwrap_or_else(|err| panic!("a fetch with nobody to ask: {err}"));
    assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());

    // The token goes wrong at the server (it was revoked), and the helper
    // still has it: the push is refused without a question, and git takes
    // the token out of the helper.
    helper.keep(&host, USER, REVOKED);
    let before = served(&site);
    let local = site.local_commit("third.txt");
    let err = push(&mut site).expect_err("the kept token was revoked");
    assert!(err.contains("Authentication failed"), "{err}");
    assert!(
        err.contains("The server refused the user name and the password or token"),
        "{err}"
    );
    // Nobody was asked, so what happens next is said too.
    assert!(err.contains("the next attempt asks again"), "{err}");
    assert!(!err.contains(REVOKED) && !err.contains(TOKEN), "{err}");
    assert_eq!(asking.asked().len(), 2, "{:?}", asking.asked());
    assert_eq!(served(&site), before);
    assert_eq!(helper.kept(&host), None, "git took it out of the helper");
    let wanted = format!(
        "Basic {}",
        super::base64(format!("{USER}:{REVOKED}").as_bytes())
    );
    assert!(
        http.seen().iter().any(
            |request| request.authorization.as_deref() == Some(wanted.as_str()) && !request.let_in
        ),
        "the server saw the revoked token and turned it away"
    );

    // So the push after that asks again, and the new answer is kept.
    push(&mut site).unwrap_or_else(|err| panic!("the push asked again: {err}"));
    assert_eq!(served(&site), local);
    assert_eq!(asking.count("Username for"), 2, "{:?}", asking.asked());
    assert_eq!(asking.count("Password for"), 2, "{:?}", asking.asked());
    assert_eq!(
        helper.kept(&host),
        Some((USER.to_owned(), TOKEN.to_owned()))
    );
    drop(asking);

    // A wrong answer typed in the dialog is refused and never kept.
    helper.forget_at("http", &host, USER);
    assert_eq!(helper.kept(&host), None);
    let mistyped = super::Asking::with(|kind, _| {
        Some(match kind {
            Kind::Text => USER.to_owned(),
            _ => "not-the-token".to_owned(),
        })
    });
    let before = served(&site);
    site.local_commit("fourth.txt");
    let err = push(&mut site).expect_err("the token typed was wrong");
    assert!(err.contains("Authentication failed"), "{err}");
    assert_eq!(mistyped.asked().len(), 2, "{:?}", mistyped.asked());
    assert_eq!(served(&site), before);
    assert_eq!(helper.kept(&host), None, "a refused answer is not kept");

    // The repository's configuration and the remote's address hold no
    // token: the helper is the one place it was kept.
    let config = std::fs::read_to_string(site.copy.path().join(".git/config")).unwrap();
    assert!(
        !config.contains(TOKEN) && !config.contains(REVOKED),
        "{config}"
    );
    assert!(!site.copy.path().join(".git/credentials").exists());
    Some(http.port)
}

#[test]
fn store_keeps_the_answer_of_the_dialog_and_forgets_a_refused_token() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let helper = Helper::store();
    asked_once_kept_refused_forgotten_and_asked_again("the store helper", &helper);
    // What it keeps is a file of its own, with the token in it as git
    // writes it there.
    let Keeper::Store(file) = &helper.keeper else {
        panic!("a store helper keeps a file")
    };
    if file.exists() {
        let kept = std::fs::read_to_string(file).unwrap();
        assert!(kept.is_empty() || kept.starts_with("http://ada:"), "{kept}");
    }
}

#[test]
fn cache_keeps_the_answer_of_the_dialog_and_forgets_a_refused_token() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(helper) = Helper::cache_or_skip("the cache helper") else {
        return;
    };
    asked_once_kept_refused_forgotten_and_asked_again("the cache helper", &helper);
}

#[test]
fn libsecret_keeps_the_answer_of_the_dialog_in_a_throwaway_keyring() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(helper) = Helper::libsecret_or_skip("the libsecret helper") else {
        return;
    };
    let Keeper::Libsecret { keyring, .. } = &helper.keeper else {
        panic!("a libsecret helper has its keyring")
    };
    let files = keyring.files();
    asked_once_kept_refused_forgotten_and_asked_again("the libsecret helper", &helper);
    // The keyring that was written is the throwaway one, in scratch.
    assert!(files.join("login.keyring").is_file(), "{}", files.display());
    assert!(files.starts_with(scratch_root()));
}

#[test]
fn osxkeychain_keeps_the_answer_of_the_dialog_in_a_throwaway_keychain() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(helper) = Helper::osxkeychain_or_skip("the osxkeychain helper") else {
        return;
    };
    let Keeper::Osxkeychain { keychain, .. } = &helper.keeper else {
        panic!("an osxkeychain helper has its keychain")
    };
    let file = keychain.file();
    let port = asked_once_kept_refused_forgotten_and_asked_again("the osxkeychain helper", &helper);
    // The keychain that was written is the throwaway one, in scratch, and
    // the user's own never had the item.
    assert!(file.is_file(), "{}", file.display());
    assert!(file.starts_with(scratch_root()));
    if let Some(port) = port {
        assert!(!keychain::users_keychains_have("127.0.0.1", port));
    }
}

#[test]
fn wincred_keeps_the_answer_of_the_dialog_in_credential_manager() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(helper) = Helper::wincred_or_skip("the wincred helper") else {
        return;
    };
    in_credential_manager("the wincred helper", &helper);
}

#[test]
fn git_credential_manager_keeps_the_answer_of_the_dialog_without_a_window() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(helper) = Helper::manager_or_skip("Git Credential Manager") else {
        return;
    };
    in_credential_manager("Git Credential Manager", &helper);
}

/// The whole life of a kept token through a helper that keeps it in the
/// user's own Credential Manager, under a name of the test's own: kept,
/// answered, erased when refused; and nothing of it left there after.
fn in_credential_manager(test: &str, helper: &Helper) {
    let Some(port) = asked_once_kept_refused_forgotten_and_asked_again(test, helper) else {
        return;
    };
    let host = format!("127.0.0.1:{port}");
    assert!(helper.path.borrow().is_some(), "{test}: kept by the path");
    #[cfg(windows)]
    {
        // Kept under a name of the test's own, as Credential Manager lists
        // it, and gone once the helper is told to forget it.
        helper.keep(&host, USER, TOKEN);
        assert!(helper.listed(&host), "{test}: not in Credential Manager");
        helper.forget_at("http", &host, USER);
        assert!(!helper.listed(&host), "{test}: left in Credential Manager");
    }
    assert_eq!(helper.kept(&host), None);
}

#[test]
fn the_first_helper_with_an_answer_is_taken_and_what_was_refused_is_forgotten() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(http) = http_or_skip("two helpers", &site) else {
        return;
    };
    let (first, second) = (Helper::store(), Helper::store());
    first.add_to(&site);
    second.add_to(&site);
    let host = format!("127.0.0.1:{}", http.port);
    let asking = super::Asking::with(|kind, _| {
        Some(match kind {
            Kind::Text => USER.to_owned(),
            _ => TOKEN.to_owned(),
        })
    });

    // The first has nothing, the second has the token: nobody is asked,
    // and git tells both what the server took.
    second.keep(&host, USER, TOKEN);
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    assert!(asking.asked().is_empty(), "{:?}", asking.asked());
    let good = Some((USER.to_owned(), TOKEN.to_owned()));
    assert_eq!(first.kept(&host), good);
    assert_eq!(second.kept(&host), good);

    // The first has a token that went wrong: its answer is the one taken,
    // the second is not asked, and git tells both to forget what was
    // refused. A git that hands the refused password along (2.56 does)
    // has the second keep its good token, which does not match it; one
    // that does not has the second forget its token too.
    first.keep(&host, USER, REVOKED);
    let err = site
        .run("remote.fetch", json!({ "remote": "origin" }))
        .expect_err("the first helper's token was revoked");
    assert!(err.contains("Authentication failed"), "{err}");
    assert!(asking.asked().is_empty(), "{:?}", asking.asked());
    assert_eq!(first.kept(&host), None);
    let second_kept = second.kept(&host);
    assert!(
        second_kept.is_none() || second_kept == good,
        "{second_kept:?}"
    );

    // So the next fetch takes the second's token, or asks in the dialog
    // when no helper has one left; either way both keep the good one.
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    let questions = if second_kept.is_some() { 0 } else { 2 };
    assert_eq!(asking.asked().len(), questions, "{:?}", asking.asked());
    assert_eq!(first.kept(&host), good);
    assert_eq!(second.kept(&host), good);
}

#[test]
fn an_askpass_program_of_the_users_comes_before_the_dialog_and_its_answer_is_kept() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let mut site = Site::new();
    let Some(http) = http_or_skip("the user's askpass", &site) else {
        return;
    };
    let helper = Helper::store();
    helper.add_to(&site);
    let host = format!("127.0.0.1:{}", http.port);
    // The user's own askpass program, which writes down what it is asked.
    let asked_file = helper.dir.join("asked");
    let program = bitbraid_testkit::script(
        &helper.dir,
        "users-askpass",
        &format!(
            "#!/bin/sh\nprintf '%s\\n' \"$1\" >> {}\ncase \"$1\" in\n  Username*) printf '%s\\n' '{USER}' ;;\n  *) printf '%s\\n' '{TOKEN}' ;;\nesac\n",
            quoted(&asked_file)
        ),
    )
    .unwrap();
    site.git()
        .run(["config", "core.askPass", program.to_str().unwrap()])
        .unwrap();
    let users = || {
        std::fs::read_to_string(&asked_file)
            .unwrap_or_default()
            .lines()
            .count()
    };
    // The application's dialog is there, and is not asked.
    let asking = super::Asking::with(|_, prompt| panic!("the dialog was asked: {prompt}"));

    let local = site.local_commit("local.txt");
    site.run(
        "remote.push",
        json!({ "remote": "origin", "branch": "main" }),
    )
    .unwrap_or_else(|err| panic!("push: {err}"));
    assert_eq!(site.served().rev_parse("refs/heads/main").unwrap(), local);
    assert!(asking.asked().is_empty(), "{:?}", asking.asked());
    assert_eq!(users(), 2, "the user's program was asked for both");
    assert_eq!(
        helper.kept(&host),
        Some((USER.to_owned(), TOKEN.to_owned()))
    );

    // And once the helper has it, neither is asked.
    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}"));
    assert_eq!(users(), 2);
    assert!(asking.asked().is_empty());
}

#[cfg(unix)]
#[test]
fn over_ssh_an_agent_that_holds_the_key_comes_before_its_passphrase() {
    use std::time::{Duration, Instant};
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if !on_path("ssh-agent") || !on_path("ssh-add") {
        eprintln!("an agent over ssh: skipped, ssh-agent or ssh-add is not installed");
        return;
    }
    let Some(sshd) = super::sshd_or_skip("an agent over ssh") else {
        return;
    };
    let mut site = Site::new();
    sshd.serve(&site);
    let upstream = site.upstream_commit("upstream.txt");

    // An agent of the test's own, with the key in it: its passphrase is
    // given to ssh-add by a script, never by anyone's askpass program. Its
    // socket is where the path is short enough for one (macOS).
    let socket_dir = bitbraid_testkit::SocketDir::new("agent").unwrap();
    let socket = socket_dir.path().join("agent.sock");
    let mut agent = Command::new("ssh-agent")
        .arg("-D")
        .arg("-a")
        .arg(&socket)
        .env_remove("SSH_AUTH_SOCK")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while !socket.exists() {
        assert!(started.elapsed() < Duration::from_secs(10), "no agent");
        std::thread::sleep(Duration::from_millis(25));
    }
    let give = bitbraid_testkit::script(
        &sshd.dir,
        "give-passphrase",
        &format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", super::PASSPHRASE),
    )
    .unwrap();
    let added = Command::new("ssh-add")
        .arg(sshd.dir.join("user_key"))
        .env("SSH_AUTH_SOCK", &socket)
        .env("SSH_ASKPASS", &give)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert!(added.success(), "ssh-add");

    // ssh is pointed at that agent rather than at none.
    let at = sshd.dir.display();
    let mut ssh = format!(
        "ssh -F {at}/ssh_config -o IdentityAgent={} -o IdentitiesOnly=yes \
         -o UserKnownHostsFile={at}/known_hosts -o GlobalKnownHostsFile=/dev/null",
        socket.display()
    );
    if std::env::var_os("SSH_ASKPASS").is_some() {
        ssh = format!(
            "SSH_ASKPASS={} SSH_ASKPASS_REQUIRE=force {ssh}",
            super::helper().display()
        );
    }
    site.git().run(["config", "core.sshCommand", &ssh]).unwrap();
    let asking = super::Asking::with(|_, prompt| {
        assert!(
            prompt.contains("(yes/no"),
            "only the host is asked: {prompt}"
        );
        Some("yes".to_owned())
    });

    site.run("remote.fetch", json!({ "remote": "origin" }))
        .unwrap_or_else(|err| panic!("fetch: {err}\n{}", sshd.log()));
    assert_eq!(site.tracking().as_deref(), Some(upstream.as_str()));
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
    // The host once, the passphrase never.
    assert_eq!(asking.asked().len(), 1, "{:?}", asking.asked());
    assert_eq!(asking.count("passphrase"), 0);
    assert_eq!(sshd.log().matches("Accepted publickey").count(), 2);
    let _ = agent.kill();
    let _ = agent.wait();
}
