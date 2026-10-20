//! Sessions on Windows: ConPTY starts the system's shells in the folder
//! asked for, with the arguments they are given, what is typed reaches
//! them and what they write is on show.
#![cfg(windows)]

use std::path::PathBuf;

use crate::common::{Home, Running, row_starting, start};
use bitbraid_terminal::{GridSize, ShellSpec};

/// `program` with `args` in the home, with the variables programs on
/// Windows need from this process (ConPTY adds the rest of it).
fn spec(home: &Home, program: &str, args: &[&str]) -> ShellSpec {
    let wanted = [
        "PATH",
        "PATHEXT",
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "TEMP",
        "TMP",
    ];
    let mut env: Vec<(String, String)> = std::env::vars()
        .filter(|(name, _)| wanted.contains(&name.to_ascii_uppercase().as_str()))
        .collect();
    env.push(("TERM".into(), "xterm-256color".into()));
    ShellSpec {
        program: program.into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: home.0.clone(),
        env,
    }
}

fn size() -> GridSize {
    GridSize::new(100, 30, 8, 16)
}

/// Whether a row of what is on show is `text` and nothing else.
fn shows(running: &Running, what: &str, text: &str) {
    running.until(what, |frame| {
        frame.rows.iter().any(|row| row.text() == text)
    });
}

#[test]
fn the_command_prompt_runs_a_command_and_ends_with_its_code() {
    let home = Home::new("windows-cmd");
    // By its name, found with the endings of PATHEXT; an argument with
    // blanks and the prompt's own syntax in it is one argument. What a
    // program writes as it ends can be lost (ConPTY may close before it
    // has passed the last of it on), so the line looked for is not the
    // last one written.
    let running = start(
        &spec(
            &home,
            "cmd",
            &["/d", "/c", "echo hel^lo & echo more & exit 4"],
        ),
        size(),
    );
    shows(&running, "the echo", "hello");
    assert_eq!(running.wait_exit(), Some(4));
}

#[test]
fn powershell_is_given_its_command_whole() {
    let home = Home::new("windows-powershell");
    let running = start(
        &spec(
            &home,
            "powershell",
            &[
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "Write-Output ('a' + 'b'); (Get-Location).Path; exit 3",
            ],
        ),
        size(),
    );
    shows(&running, "the sum", "ab");
    shows(&running, "the folder", &home.0.display().to_string());
    assert_eq!(running.wait_exit(), Some(3));
}

#[test]
fn a_program_in_a_folder_with_blanks_is_started() {
    // The sh of Git for Windows, under `C:\Program Files\Git`.
    // The tests need git on PATH everywhere; on Windows that is Git for
    // Windows, which brings it.
    let sh: PathBuf = std::env::var_os("PATH")
        .and_then(|path| {
            std::env::split_paths(&path)
                .filter(|dir| dir.join("git.exe").is_file())
                .filter_map(|dir| dir.parent().map(|root| root.join("usr").join("bin")))
                .map(|bin| bin.join("sh.exe"))
                .find(|sh| sh.is_file())
        })
        .expect("the sh of Git for Windows");
    let home = Home::new("windows-sh");
    let running = start(
        &spec(&home, &sh.display().to_string(), &["-c", "echo $((6 * 7))"]),
        size(),
    );
    shows(&running, "the answer", "42");
    assert_eq!(running.wait_exit(), Some(0));
}

#[test]
fn what_is_typed_reaches_the_shell() {
    let home = Home::new("windows-typed");
    let running = start(
        &spec(&home, "cmd", &["/d", "/q", "/k", "prompt $g"]),
        size(),
    );
    running.until("the prompt", |frame| row_starting(frame, ">").is_some());
    running.type_line("echo TYPED");
    shows(&running, "the answer", "TYPED");
    // It started in the folder it was given.
    running.type_line("cd");
    shows(&running, "the folder", &home.0.display().to_string());
    running.type_line("exit 5");
    assert_eq!(running.wait_exit(), Some(5));
}

/// Wait until the shell is found in `folder`: its working folder, as the
/// system tells it.
fn followed_to(running: &Running, name: &str, folder: &std::path::Path) {
    let started = std::time::Instant::now();
    while running.session.current_folder() != folder {
        assert!(
            started.elapsed() < crate::common::WAIT,
            "{name}: the new folder was not followed: {}",
            running.session.current_folder().display()
        );
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// The command prompt, PowerShell as the terminal starts it and Git's
/// bash each start in the home and are followed into a folder below it.
/// Git's bash is the program people name, `bin\bash.exe`, which starts
/// the bash of MSYS2 and waits for it.
#[test]
fn the_shells_folder_is_followed() {
    let bash: Option<PathBuf> = std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .filter(|dir| dir.join("git.exe").is_file())
            .filter_map(|dir| dir.parent().map(|root| root.join("bin").join("bash.exe")))
            .find(|bash| bash.is_file())
    });
    let following = bitbraid_terminal::shell::powershell_following();
    let following: Vec<&str> = following.iter().map(String::as_str).collect();
    let mut shells: Vec<(&str, String, Vec<&str>)> = vec![
        ("cmd", "cmd".to_owned(), vec!["/d", "/q", "/k", "prompt $g"]),
        ("powershell", "powershell".to_owned(), following),
    ];
    if let Some(bash) = &bash {
        shells.push((
            "bash",
            bash.display().to_string(),
            vec!["--norc", "--noprofile", "-i"],
        ));
    }
    for (name, program, args) in shells {
        let home = Home::new(&format!("windows-folder-{name}"));
        std::fs::create_dir_all(home.0.join("deeper")).unwrap();
        let mut spec = spec(&home, &program, &args);
        // PowerShell reads no profile of the user's: the scratch home
        // is where it would look.
        spec.env
            .push(("USERPROFILE".into(), home.0.display().to_string()));
        spec.env.push(("HOME".into(), home.0.display().to_string()));
        let running = start(&spec, size());
        followed_to(&running, name, &home.0);
        running.type_line("cd deeper");
        followed_to(&running, name, &home.0.join("deeper"));
        running.type_line("exit");
        let _ = running.wait_exit();
    }
}
