//! The command line being typed at the prompt of a real shell, read off
//! its screen: where typing began, what is typed, whether the shell is in
//! front, and the folder it is in.
#![cfg(unix)]

use std::time::{Duration, Instant};

use crate::common::{Home, Running, WAIT, prompt};
use bitbraid_terminal::{GridSize, Input, Key, Modifiers, Prompt, Session, TypedLine, ViewPoint};

/// Wait until `test` holds of the session.
fn until<T>(what: &str, session: &Session, test: impl Fn(&Session) -> Option<T>) -> T {
    let started = Instant::now();
    loop {
        if let Some(found) = test(session) {
            return found;
        }
        assert!(started.elapsed() < WAIT, "never saw {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Wait until the line typed after `prompt` reads `before` up to the
/// cursor.
fn typed(running: &Running, prompt: &Prompt, before: &str) -> TypedLine {
    until(&format!("{before:?} typed"), &running.session, |session| {
        session
            .typed_line(prompt)
            .filter(|line| line.before == before)
    })
}

#[test]
fn the_line_typed_after_the_prompt_is_read_off_the_screen() {
    let home = Home::new("typed-line");
    // The cursor is moved back below: a line editor's work.
    let running = Running::bash(&home);
    assert!(running.session.shell_in_front());
    let here = running.session.prompt_here().expect("a prompt");
    assert_eq!(here.text(), "$ ");

    running.session.input(Input::Text("git chec".into()));
    let line = typed(&running, &here, "git chec");
    assert_eq!(line.after, "");
    let row = line.cursor.row;
    assert_eq!(line.cursor, ViewPoint { row, column: 10 });

    // The cursor moved back: what follows it is told apart.
    running
        .session
        .input(Input::Key(Key::Left, Modifiers::NONE));
    let line = typed(&running, &here, "git che");
    assert_eq!(line.after, "c");

    // Another prompt: the line is not read after one it does not begin
    // with.
    let other = Prompt::new("> ");
    assert_eq!(running.session.typed_line(&other), None);
    running
        .session
        .input(Input::Key(Key::Character('u'), Modifiers::CONTROL));
    typed(&running, &here, "");
}

#[test]
fn a_line_wider_than_the_screen_is_read_whole() {
    let home = Home::new("typed-wide");
    let running = Running::shell_sized(&home, GridSize::new(20, 10, 8, 16));
    let here = running.session.prompt_here().unwrap();
    let long = "git add some/long/path/to/a/file.txt";
    running.session.input(Input::Text(long.into()));
    let line = typed(&running, &here, long);
    assert!(line.cursor.row >= 1, "{line:?}");
}

#[test]
fn a_program_in_front_or_on_the_alternate_screen_is_not_the_shell() {
    let home = Home::new("typed-front");
    let running = Running::shell(&home);
    assert!(running.session.shell_in_front());
    // A job in the foreground has the terminal. It says so once it runs:
    // the shell hands the terminal to the job's group before the job's
    // own program has started, and a Ctrl+C in between is taken by the
    // shell's child as the shell takes it, so `sleep` would go on.
    running.type_line("sh -c 'echo STARTED; exec sleep 100'");
    running.until("the job started", |frame| {
        frame.rows.iter().any(|row| row.text() == "STARTED")
    });
    until("the job in front", &running.session, |session| {
        (!session.shell_in_front()).then_some(())
    });
    assert_eq!(running.session.prompt_here(), None);
    running
        .session
        .input(Input::Key(Key::Character('c'), Modifiers::CONTROL));
    running.until("the prompt again", prompt);
    until("the shell in front", &running.session, |session| {
        session.shell_in_front().then_some(())
    });
    // A program that takes the whole screen: not the shell, even though
    // the shell itself switched to it here.
    running.run("printf '\\033[?1049h'", "IN");
    assert!(!running.session.shell_in_front());
    assert_eq!(running.session.prompt_here(), None);
    running.run("printf '\\033[?1049l'", "OUT");
    assert!(running.session.shell_in_front());
}

#[test]
fn the_folder_the_shell_is_in_is_followed() {
    let home = Home::new("typed-folder");
    std::fs::create_dir_all(home.0.join("deeper")).unwrap();
    let running = Running::shell(&home);
    assert_eq!(running.session.current_folder(), home.0);
    running.run("cd deeper", "MOVED");
    if cfg!(any(target_os = "linux", target_os = "macos")) {
        until("the new folder", &running.session, |session| {
            (session.current_folder() == home.0.join("deeper")).then_some(())
        });
    }
}
