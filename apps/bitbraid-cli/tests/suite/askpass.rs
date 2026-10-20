//! The askpass chain from end to end: git runs the binary as its askpass
//! program, the binary asks the server, git gets the answer. `git
//! credential fill` asks the same way a fetch over HTTPS does, and
//! `ssh-keygen` asks for a key's passphrase the way ssh does when it
//! connects, with no network involved. On Unix the binary reaches the
//! server through its socket, on Windows through its port on the loopback
//! interface.

use std::{
    ffi::OsString,
    io::Write,
    path::PathBuf,
    process::{Command, Output, Stdio},
    sync::{Arc, Mutex},
};

use bitbraid_git::askpass::{Event, Kind, Server, child_env};
use bitbraid_testkit::Scratch;

use crate::ASKPASS_SERVER as ONE;

const PASSPHRASE: &str = "correct horse battery staple";

fn helper() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_bitbraid-cli"))
}

/// `git credential fill` for a made-up host, with nothing to answer it but
/// the askpass program: no helper, no terminal, no askpass of the user's.
fn credential_fill(scratch: &Scratch) -> (bool, String) {
    let mut cmd = Command::new("git");
    cmd.current_dir(scratch.path())
        .args(["-c", "credential.helper=", "credential", "fill"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env_remove("GIT_ASKPASS")
        .env_remove("SSH_ASKPASS")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in child_env() {
        cmd.env(key, value);
    }
    // The test process may have an askpass program of the user's in its
    // environment, in which case ours was not offered; offer it here.
    cmd.env("SSH_ASKPASS", helper())
        .env("SSH_ASKPASS_REQUIRE", "force");
    let mut child = cmd.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"protocol=https\nhost=example.invalid\n\n")
        .unwrap();
    let out = child.wait_with_output().unwrap();
    (
        out.status.success(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
    )
}

#[test]
fn git_gets_its_answers_through_the_binary() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = Scratch::of("linear").unwrap();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let _server = Server::start(helper(), move |event| {
        if let Event::Asked(question) = event {
            log.lock()
                .unwrap()
                .push((question.kind, question.prompt.clone()));
            let answer = match question.kind {
                Kind::Text => "ada",
                _ => PASSPHRASE,
            };
            question.answer(Some(answer.to_owned()));
        }
    })
    .unwrap();

    let (ok, out) = credential_fill(&scratch);
    assert!(ok, "{out}");
    assert!(out.contains("username=ada\n"), "{out}");
    assert!(out.contains(&format!("password={PASSPHRASE}\n")), "{out}");
    let asked = asked.lock().unwrap().clone();
    assert_eq!(asked.len(), 2, "{asked:?}");
    assert_eq!(asked[0].0, Kind::Text);
    assert!(
        asked[0]
            .1
            .starts_with("Username for 'https://example.invalid'")
    );
    assert_eq!(asked[1].0, Kind::Secret);
    assert!(
        asked[1]
            .1
            .starts_with("Password for 'https://ada@example.invalid'")
    );
}

#[test]
fn a_declined_question_makes_git_give_up() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let scratch = Scratch::of("linear").unwrap();
    let _server = Server::start(helper(), |event| {
        if let Event::Asked(question) = event {
            question.answer(None);
        }
    })
    .unwrap();
    let (ok, out) = credential_fill(&scratch);
    assert!(!ok, "{out}");
    assert!(!out.contains("password="), "{out}");
}

/// The binary run the way git and ssh run it, with `env` added: the
/// question as its one argument.
fn ask_binary(prompt: &str, env: &[(&str, OsString)]) -> Output {
    let mut cmd = Command::new(helper());
    cmd.arg(prompt)
        .env_remove("SSH_ASKPASS_PROMPT")
        .stdin(Stdio::null());
    for (key, value) in env {
        cmd.env(key, value);
    }
    cmd.output().unwrap()
}

#[test]
fn the_binary_asks_the_server_and_prints_the_answer() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let _server = Server::start(helper(), move |event| {
        if let Event::Asked(question) = event {
            log.lock()
                .unwrap()
                .push((question.kind, question.prompt.clone()));
            let answer = (!question.prompt.contains("nope")).then(|| PASSPHRASE.to_owned());
            question.answer(answer);
        }
    })
    .unwrap();
    let env = child_env();

    let out = ask_binary("Enter passphrase for key 'id': ", &env);
    assert_eq!(out.status.code(), Some(0), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        format!("{PASSPHRASE}\n")
    );
    assert!(out.stderr.is_empty(), "{out:?}");

    // What ssh says about the question comes along.
    let mut confirm = env.clone();
    confirm.push(("SSH_ASKPASS_PROMPT", OsString::from("confirm")));
    let out = ask_binary("Allow use of key id?", &confirm);
    assert_eq!(out.status.code(), Some(0), "{out:?}");

    // Declined: a failure, with nothing printed.
    let out = ask_binary("Password for nope: ", &env);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());

    // A wrong token: refused without anybody being asked.
    let wrong: Vec<_> = env
        .iter()
        .map(|(key, value)| match *key {
            bitbraid_git::askpass::TOKEN_ENV => (*key, OsString::from("0".repeat(32))),
            _ => (*key, value.clone()),
        })
        .collect();
    let out = ask_binary("Password: ", &wrong);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(out.stdout.is_empty());

    assert_eq!(
        asked.lock().unwrap().clone(),
        [
            (Kind::Secret, "Enter passphrase for key 'id': ".to_owned()),
            (Kind::Confirm, "Allow use of key id?".to_owned()),
            (Kind::Secret, "Password for nope: ".to_owned()),
        ]
    );
}

/// `ssh-keygen` where git's ssh is found: on the path, and on Windows
/// beside the `sh` of Git for Windows.
fn ssh_keygen() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "ssh-keygen.exe"
    } else {
        "ssh-keygen"
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    #[cfg(windows)]
    let beside_git =
        bitbraid_testkit::script::git_sh().and_then(|sh| sh.parent().map(|dir| dir.to_path_buf()));
    #[cfg(not(windows))]
    let beside_git = None;
    std::env::split_paths(&path)
        .chain(beside_git)
        .map(|dir| dir.join(name))
        .find(|file| file.is_file())
}

#[test]
#[allow(clippy::print_stderr)] // saying why it did not run
fn ssh_gets_a_passphrase_through_the_binary() {
    let _one = ONE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let Some(keygen) = ssh_keygen() else {
        eprintln!("ssh asking through the binary: skipped, ssh-keygen is not installed");
        return;
    };
    let scratch = Scratch::of("linear").unwrap();
    let key = scratch.path().join("test_key");
    let made = Command::new(&keygen)
        .args([
            "-q",
            "-t",
            "ed25519",
            "-C",
            "bitbraid test",
            "-N",
            PASSPHRASE,
        ])
        .arg("-f")
        .arg(&key)
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(made.status.success(), "{made:?}");
    let public = std::fs::read_to_string(key.with_extension("pub")).unwrap();

    let answer = Arc::new(Mutex::new(PASSPHRASE.to_owned()));
    let asked = Arc::new(Mutex::new(Vec::new()));
    let (log, given) = (asked.clone(), answer.clone());
    let _server = Server::start(helper(), move |event| {
        if let Event::Asked(question) = event {
            log.lock()
                .unwrap()
                .push((question.kind, question.prompt.clone()));
            question.answer(Some(given.lock().unwrap().clone()));
        }
    })
    .unwrap();
    // Reading the private key takes its passphrase, which ssh-keygen asks
    // for as ssh does: through the askpass program, as it has no terminal.
    let read_key = || {
        let mut cmd = Command::new(&keygen);
        cmd.args(["-y", "-f"])
            .arg(&key)
            .env_remove("SSH_ASKPASS_PROMPT")
            .stdin(Stdio::null());
        for (name, value) in child_env() {
            cmd.env(name, value);
        }
        cmd.env("SSH_ASKPASS", helper())
            .env("SSH_ASKPASS_REQUIRE", "force");
        cmd.output().unwrap()
    };

    let out = read_key();
    assert!(out.status.success(), "{out:?}");
    let first_two = |text: &str| {
        text.split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(
        first_two(&String::from_utf8_lossy(&out.stdout)),
        first_two(&public)
    );
    let asked_once = asked.lock().unwrap().clone();
    assert_eq!(asked_once.len(), 1, "{asked_once:?}");
    assert_eq!(asked_once[0].0, Kind::Secret);
    assert!(
        asked_once[0].1.to_lowercase().contains("passphrase"),
        "{asked_once:?}"
    );

    // A wrong passphrase: the key is not read.
    *answer.lock().unwrap() = "not the passphrase".to_owned();
    let out = read_key();
    assert!(!out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty(), "{out:?}");
}

#[test]
fn the_binary_is_an_ordinary_program_without_a_server() {
    // No socket named: the arguments are the command line's, and a question
    // is not a subcommand.
    let out = Command::new(helper())
        .arg("Password: ")
        .env_remove(bitbraid_git::askpass::SOCKET_ENV)
        .env_remove(bitbraid_git::askpass::TOKEN_ENV)
        .output()
        .unwrap();
    assert!(!out.status.success());
    assert!(out.stdout.is_empty());

    // A socket that is not there: declined, with nothing printed.
    let out = Command::new(helper())
        .arg("Password: ")
        .env(bitbraid_git::askpass::SOCKET_ENV, "/nonexistent/askpass")
        .env(bitbraid_git::askpass::TOKEN_ENV, "0000")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
}
