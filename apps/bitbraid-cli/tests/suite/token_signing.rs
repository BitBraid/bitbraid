//! SSH signing with a key held by a PKCS#11 token, as far as smart cards
//! go without hardware. SoftHSM2 is the token: a key made with ssh-keygen
//! is imported into it and its file deleted, so that the token alone holds
//! it. A scratch ssh-agent that may load that one module takes the token's
//! key with `ssh-add -s`, which asks for the PIN through the application's
//! askpass program; a commit is signed through the binary with
//! `gpg.format=ssh` and the key in the agent, and judged by the binary and
//! by `git verify-commit` against an allowed-signers file. With the token
//! taken out of the agent, signing is refused with what is wrong.
//!
//! The key is ECDSA (P-256), except where OpenSSH is built on LibreSSL, as
//! Apple's is (`ssh -V`): its `ssh-pkcs11-helper` cannot read the curve of
//! an EC key on a token ("couldn't get curve nid"), and the agent refuses
//! the card with the right PIN; an RSA key it takes, and there the key is
//! RSA (2048).
//!
//! SoftHSM2 is seldom installed, so the test is opt-in:
//! `BITBRAID_SOFTHSM2` names `libsofthsm2.so` (of a package unpacked
//! anywhere; `softhsm2-util` is looked for beside it, in `../../bin`, then
//! on `PATH`). Without it the test says it was skipped. Nothing of the
//! developer's agent, keys or configuration is used: the binary runs with
//! the scratch agent's socket, a home of the test's own and no system
//! configuration.
//!
//! On Windows the programs are Windows' own OpenSSH (`System32\OpenSSH`):
//! Git for Windows' OpenSSH dies loading the native module. Its agent is
//! the system's service, which alone may start the helper that loads a
//! module as the user, and which loads one only from its own list of
//! places (Program Files); its sessions do not see the test's environment,
//! so the service is given `SOFTHSM2_CONF` in its own (the `Environment`
//! value of its registry key) and the test writes that file.
//! `BITBRAID_SOFTHSM2` names `softhsm2-x64.dll` where the agent takes it
//! (`softhsm2-util.exe`, which is 32-bit and loads `softhsm2.dll`, in
//! `..\bin`), and `BITBRAID_SOFTHSM2_CONF` the file the service is given.
//! The test runs only when the agent holds no key, and takes its card out
//! again; git signs with Windows' `ssh-keygen`, whose `ssh-add` the engine
//! asks about the agent's keys.
//!
//! A gpg key on an emulated OpenPGP card is tested in the engine's suite
//! (`crates/engine/tests/suite/signing_card.rs`). Not done, for want of
//! hardware: a certificate on a card.
//!
//! Sources: ssh-agent(1) (`-D`, `-a`, `-P`), ssh-add(1) (`-s`, `-e`, `-L`,
//! `SSH_ASKPASS`, `SSH_ASKPASS_REQUIRE`), ssh-keygen(1) (`-m PKCS8`,
//! `-l`), softhsm2-util(1) (`--init-token`, `--import`), softhsm2.conf(5);
//! git-config(1) (`gpg.format`, `user.signingKey` given as `key::`,
//! `gpg.ssh.allowedSignersFile`), git-verify-commit(1); SoftHSM2 for
//! Windows' README (where it looks for its configuration); Microsoft
//! Learn, "OpenSSH key management" (the agent service and its pipe).
#![cfg(any(unix, windows))]
#![allow(clippy::print_stdout)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};
#[cfg(unix)]
use std::{
    process::Child,
    time::{Duration, Instant},
};

use bitbraid_git::askpass::{Event, Kind, Server, child_env};
use bitbraid_testkit::{Scratch, scratch_root};
use serde_json::{Value, json};

use crate::ASKPASS_SERVER as ONE;

const PIN: &str = "246813";
const SO_PIN: &str = "97531864";

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitbraid-cli"))
}

fn on_path(program: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(program))
            .find(|candidate| candidate.is_file())
    })
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).trim().to_owned()
}

/// An OpenSSH program: on Windows Windows' own, elsewhere the one on
/// `PATH`.
fn program(name: &str) -> PathBuf {
    if cfg!(windows) {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        PathBuf::from(root)
            .join("System32")
            .join("OpenSSH")
            .join(format!("{name}.exe"))
    } else {
        PathBuf::from(name)
    }
}

/// The agent the token's key goes to: on Unix a scratch one, ended with
/// the test; on Windows the system's, which the test's card is taken out
/// of again.
struct Agent {
    #[cfg(unix)]
    child: Child,
    socket: PathBuf,
    /// Where the socket is: a short path, as macOS needs.
    #[cfg(unix)]
    _dir: bitbraid_testkit::SocketDir,
    #[cfg(windows)]
    module: PathBuf,
}

impl Drop for Agent {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = fs::remove_file(&self.socket);
        }
        #[cfg(windows)]
        {
            let _ = Command::new(program("ssh-add"))
                .arg("-e")
                .arg(&self.module)
                .env("SSH_AUTH_SOCK", &self.socket)
                .stdin(Stdio::null())
                .output();
        }
    }
}

/// Everything of the test, in a directory of its own.
struct Token {
    dir: PathBuf,
    module: PathBuf,
    util: PathBuf,
    /// The module `softhsm2-util` loads: on Windows the 32-bit one beside
    /// `module`, as the program is 32-bit.
    util_module: PathBuf,
    conf: PathBuf,
    home: PathBuf,
}

impl Drop for Token {
    fn drop(&mut self) {
        if self.dir.starts_with(scratch_root()) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }
}

impl Token {
    /// A token with a key of `key_kind` on it, made with ssh-keygen and
    /// imported; the key's file is gone afterwards. None, with a line
    /// saying why, where that cannot be done.
    fn new(
        module: PathBuf,
        util: PathBuf,
        util_module: PathBuf,
        conf: Option<PathBuf>,
    ) -> Option<Self> {
        let dir = scratch_root().join(format!("token-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let (tokens, home) = (dir.join("tokens"), dir.join("home"));
        for made in [&tokens, &home] {
            fs::create_dir_all(made).unwrap();
        }
        fs::write(home.join(".gitconfig"), "").unwrap();
        let conf = conf.unwrap_or_else(|| dir.join("softhsm2.conf"));
        fs::write(
            &conf,
            format!(
                "directories.tokendir = {}\nobjectstore.backend = file\nlog.level = ERROR\n",
                tokens.display().to_string().replace('\\', "/")
            ),
        )
        .unwrap();
        let token = Self {
            dir,
            module,
            util,
            util_module,
            conf,
            home,
        };
        let module = token.util_module.to_str().unwrap().to_owned();
        let initialised = token.util(&[
            "--module",
            &module,
            "--init-token",
            "--free",
            "--label",
            "bitbraid",
            "--pin",
            PIN,
            "--so-pin",
            SO_PIN,
        ]);
        if !initialised.status.success() {
            println!(
                "skipped: softhsm2-util could not make a token: {}",
                text(&initialised.stderr)
            );
            return None;
        }
        let key = token.dir.join("key");
        let (kind, bits) = key_kind();
        let made = Command::new(program("ssh-keygen"))
            .args(["-q", "-t", kind, "-b", bits, "-m", "PKCS8", "-N", ""])
            .args(["-C", "on the token", "-f"])
            .arg(&key)
            .stdin(Stdio::null())
            .status()
            .unwrap();
        assert!(made.success());
        let imported = token.util(&[
            "--module",
            &module,
            "--import",
            key.to_str().unwrap(),
            "--token",
            "bitbraid",
            "--label",
            "bitbraid-ssh",
            "--id",
            "01",
            "--pin",
            PIN,
        ]);
        assert!(imported.status.success(), "{}", text(&imported.stderr));
        // Only the token has it now.
        fs::remove_file(&key).unwrap();
        fs::remove_file(key.with_extension("pub")).unwrap();
        Some(token)
    }

    fn util(&self, args: &[&str]) -> Output {
        Command::new(&self.util)
            .args(args)
            .env("SOFTHSM2_CONF", &self.conf)
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    /// Windows' agent, when it runs and holds no key; None, with a line
    /// saying why, when not.
    #[cfg(windows)]
    fn agent(&self) -> Option<Agent> {
        let socket = PathBuf::from(r"\\.\pipe\openssh-ssh-agent");
        let listed = Command::new(program("ssh-add"))
            .arg("-l")
            .env("SSH_AUTH_SOCK", &socket)
            .stdin(Stdio::null())
            .output()
            .unwrap();
        match listed.status.code() {
            // "The agent has no identities."
            Some(1) => Some(Agent {
                socket,
                module: self.module.clone(),
            }),
            Some(0) => {
                println!("skipped: Windows' ssh-agent holds keys, which the test does not touch");
                None
            }
            _ => {
                println!(
                    "skipped: Windows' ssh-agent does not answer: {}",
                    text(&listed.stderr)
                );
                None
            }
        }
    }

    /// An agent that may load this module and no other.
    #[cfg(unix)]
    fn agent(&self) -> Option<Agent> {
        let dir = bitbraid_testkit::SocketDir::new("agent").unwrap();
        let socket = dir.path().join("agent.sock");
        let child = Command::new("ssh-agent")
            .arg("-D")
            .arg("-a")
            .arg(&socket)
            .arg("-P")
            .arg(&self.module)
            .env("SOFTHSM2_CONF", &self.conf)
            .env_remove("SSH_AUTH_SOCK")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !socket.exists() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(socket.exists(), "the agent did not start");
        Some(Agent {
            child,
            socket,
            _dir: dir,
        })
    }

    /// ssh-add asking the scratch agent, with the application's askpass
    /// program to ask on.
    fn ssh_add(&self, agent: &Agent, args: &[&str]) -> Output {
        let mut cmd = Command::new(program("ssh-add"));
        cmd.args(args)
            .env("SSH_AUTH_SOCK", &agent.socket)
            .env("SSH_ASKPASS", helper())
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .stdin(Stdio::null());
        for (key, value) in child_env() {
            if key != "SSH_ASKPASS" && key != "SSH_ASKPASS_REQUIRE" {
                cmd.env(key, value);
            }
        }
        cmd.output().unwrap()
    }

    /// `run <id>` of the binary in `dir`, with the scratch agent and
    /// nothing of the developer's: its output.
    fn cli(&self, agent: &Agent, dir: &Path, id: &str, params: Value) -> Output {
        Command::new(helper())
            .args(["run", id, "--params", &params.to_string()])
            .current_dir(dir)
            .env("SSH_AUTH_SOCK", &agent.socket)
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("BITBRAID_CONFIG_DIR", self.dir.join("bitbraid-config"))
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .stdin(Stdio::null())
            .output()
            .unwrap()
    }

    fn ok(&self, agent: &Agent, dir: &Path, id: &str, params: Value) -> Value {
        let out = self.cli(agent, dir, id, params.clone());
        assert!(out.status.success(), "{id} {params}: {}", text(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    }
}

/// The kind of key made for the token, and its size: RSA where OpenSSH is
/// built on LibreSSL (see the module's documentation), ECDSA elsewhere.
fn key_kind() -> (&'static str, &'static str) {
    let said = Command::new(program("ssh"))
        .arg("-V")
        .stdin(Stdio::null())
        .output()
        .map(|out| format!("{}{}", text(&out.stdout), text(&out.stderr)))
        .unwrap_or_default();
    if said.contains("LibreSSL") {
        ("rsa", "2048")
    } else {
        ("ecdsa", "256")
    }
}

#[test]
fn a_key_on_a_token_signs_through_the_agent_and_its_pin_goes_through_askpass() {
    let Some(module) = std::env::var_os("BITBRAID_SOFTHSM2").map(PathBuf::from) else {
        println!(
            "skipped: set BITBRAID_SOFTHSM2 to SoftHSM2's libsofthsm2.so (on Windows its \
             softhsm2-x64.dll)"
        );
        return;
    };
    for name in ["ssh-agent", "ssh-add", "ssh-keygen"] {
        let found = if cfg!(windows) {
            program(name).is_file()
        } else {
            on_path(name).is_some()
        };
        if !found {
            println!("skipped: {name} is not installed");
            return;
        }
    }
    // The agent compares the module's real path with what it may load
    // (Windows' with its list of places, as it is written).
    let module = if cfg!(windows) {
        module
    } else {
        fs::canonicalize(&module).unwrap()
    };
    let beside = if cfg!(windows) {
        module
            .parent()
            .and_then(Path::parent)
            .map(|root| root.join("bin").join("softhsm2-util.exe"))
    } else {
        module
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent)
            .map(|usr| usr.join("bin").join("softhsm2-util"))
    }
    .filter(|util| util.is_file());
    let Some(util) = beside.or_else(|| on_path("softhsm2-util")) else {
        println!("skipped: softhsm2-util was found neither beside the module nor on PATH");
        return;
    };
    let util_module = Some(module.with_file_name("softhsm2.dll"))
        .filter(|small| cfg!(windows) && small.is_file())
        .unwrap_or_else(|| module.clone());
    // Windows' agent reads the file its service is told of.
    let conf = std::env::var_os("BITBRAID_SOFTHSM2_CONF").map(PathBuf::from);
    if cfg!(windows) && conf.is_none() {
        println!(
            "skipped: set BITBRAID_SOFTHSM2_CONF to the softhsm2.conf that Windows' ssh-agent \
             service is given (SOFTHSM2_CONF in its environment)"
        );
        return;
    }
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(token) = Token::new(module.clone(), util, util_module, conf) else {
        return;
    };
    let Some(agent) = token.agent() else {
        return;
    };

    // The application answers the PIN: wrongly first, then rightly.
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let _server = Server::start(helper(), move |event| {
        if let Event::Asked(question) = event {
            let mut log = log.lock().unwrap();
            log.push((question.kind, question.prompt.clone()));
            let answer = if log.len() == 1 { "000000" } else { PIN };
            question.answer(Some(answer.to_owned()));
        }
    })
    .unwrap();
    let module_arg = module.to_str().unwrap();
    let wrong = token.ssh_add(&agent, &["-s", module_arg]);
    assert!(!wrong.status.success(), "a wrong PIN was taken");
    let listed = token.ssh_add(&agent, &["-L"]);
    assert!(!listed.status.success(), "{}", text(&listed.stdout));
    let added = token.ssh_add(&agent, &["-s", module_arg]);
    assert!(added.status.success(), "{}", text(&added.stderr));
    {
        let asked = asked.lock().unwrap();
        assert_eq!(asked.len(), 2, "{asked:?}");
        for (kind, prompt) in asked.iter() {
            assert_eq!(*kind, Kind::Secret);
            assert!(prompt.contains("PKCS#11"), "{prompt}");
        }
    }
    let listed = token.ssh_add(&agent, &["-L"]);
    assert!(listed.status.success(), "{}", text(&listed.stderr));
    let public = text(&listed.stdout).lines().next().unwrap().to_owned();
    let wanted = match key_kind().0 {
        "rsa" => "ssh-rsa ",
        _ => "ecdsa-sha2-nistp256 ",
    };
    assert!(public.starts_with(wanted), "{public}");
    let public_file = token.dir.join("token-key.pub");
    fs::write(&public_file, format!("{public}\n")).unwrap();
    let fingerprint = Command::new(program("ssh-keygen"))
        .arg("-lf")
        .arg(&public_file)
        .output()
        .unwrap();
    let fingerprint = text(&fingerprint.stdout)
        .split_whitespace()
        .nth(1)
        .unwrap()
        .to_owned();

    // A repository that signs with the key in the agent, given in full.
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    let words: Vec<&str> = public.split_whitespace().collect();
    let key = format!("key::{} {}", words[0], words[1]);
    let allowed = token.dir.join("allowed_signers");
    fs::write(
        &allowed,
        format!("ada@fixture.invalid {} {}\n", words[0], words[1]),
    )
    .unwrap();
    let keygen = program("ssh-keygen");
    for (name, value) in [
        ("user.name", "Ada Fixture"),
        ("user.email", "ada@fixture.invalid"),
        ("gpg.format", "ssh"),
        ("gpg.ssh.program", keygen.to_str().unwrap()),
        ("user.signingkey", key.as_str()),
        ("gpg.ssh.allowedSignersFile", allowed.to_str().unwrap()),
        ("commit.gpgsign", "false"),
        // No gpg of anybody's is asked for its keys: this is SSH's test,
        // and on Windows a daemon gpg starts keeps every handle it had,
        // the binary's output among them, open until it ends.
        ("gpg.program", "no-such-gpg"),
        ("gpg.x509.program", "no-such-gpgsm"),
    ] {
        git.run(["config", "--local", name, value]).unwrap();
    }
    let status = token.ok(&agent, copy.path(), "sign.status", json!({}));
    assert_eq!(status["format"], "ssh", "{status}");
    assert_eq!(status["can_sign"], true, "{status}");
    let no_files = token.dir.join("no-key-files");
    fs::create_dir_all(&no_files).unwrap();
    let keys = token.ok(
        &agent,
        copy.path(),
        "sign.keys",
        json!({ "ssh_dir": no_files, "agent": true }),
    );
    let in_agent = keys
        .as_array()
        .unwrap()
        .iter()
        .find(|listed| listed["fingerprint"] == fingerprint.as_str())
        .unwrap_or_else(|| panic!("the token's key is not listed: {keys}"));
    assert_eq!(in_agent["source"], "agent", "{in_agent}");
    assert_eq!(in_agent["in_agent"], true, "{in_agent}");
    assert_eq!(in_agent["format"], "ssh", "{in_agent}");

    // Signed through the binary, judged by it and by git.
    copy.write("token.txt", "signed with a key on a token\n")
        .unwrap();
    let made = token.ok(
        &agent,
        copy.path(),
        "commit.create",
        json!({ "message": "signed on a token", "stage_all": true, "sign": true }),
    );
    let id = made["id"].as_str().unwrap().to_owned();
    assert_eq!(git.rev_parse("HEAD").unwrap(), id);
    let raw = git.run(["cat-file", "commit", &id]).unwrap();
    assert!(
        raw.contains("gpgsig -----BEGIN SSH SIGNATURE-----"),
        "{raw}"
    );
    let verdicts = token.ok(&agent, copy.path(), "sign.verify", json!({ "ids": [id] }));
    let verdict = &verdicts[0];
    assert_eq!(verdict["state"], "good", "{verdict}");
    assert_eq!(verdict["format"], "ssh");
    assert_eq!(verdict["signer"], "ada@fixture.invalid");
    assert_eq!(verdict["fingerprint"], fingerprint.as_str());
    assert_eq!(
        git.run(["log", "-1", "--format=%G?%x00%GS%x00%GF", &id])
            .unwrap()
            .trim(),
        format!("G\0ada@fixture.invalid\0{fingerprint}")
    );
    assert!(git.run(["verify-commit", &id]).is_ok());

    // The token taken out of the agent: signing cannot work, and says
    // why; nothing is committed.
    let removed = token.ssh_add(&agent, &["-e", module_arg]);
    assert!(removed.status.success(), "{}", text(&removed.stderr));
    let status = token.ok(&agent, copy.path(), "sign.status", json!({}));
    assert_eq!(status["can_sign"], false, "{status}");
    assert!(
        status["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("agent")),
        "{status}"
    );
    copy.write("token.txt", "not to be signed\n").unwrap();
    let refused = token.cli(
        &agent,
        copy.path(),
        "commit.create",
        json!({ "message": "no key", "stage_all": true, "sign": true }),
    );
    assert!(!refused.status.success());
    let said = text(&refused.stderr);
    assert!(
        said.contains("the SSH agent does not have the signing key"),
        "{said}"
    );
    assert_eq!(git.rev_parse("HEAD").unwrap(), id);
}
