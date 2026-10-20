//! Shells other than bash on a real PTY: zsh, fish,
//! nushell, dash and tcsh start in the folder they are given, the folder
//! they move to is followed, they start as login shells with `-l` (as the
//! engine starts them on macOS), the keys a line editor acts on reach
//! zle, fish's editor, reedline and tcsh's editor as they understand
//! them, a paste arrives as typed text, a new size is seen, and `exit`
//! ends them with its code. Paths dropped on them are read back whole by
//! each (the quoting of `quote_path` for their kind).
//!
//! Opt-in: `BITBRAID_TEST_SHELLS` names a folder holding the shells'
//! programs (`zsh`, `fish`, `nu`, `dash`, `tcsh`; any that are missing are
//! left out); without it the tests say they were skipped. Each shell runs
//! with a home and configuration of the test's own, a prompt of `$ ` and
//! nothing of the user's; zsh is told where its modules are when they lie
//! beside its program (`../lib/zsh/<version>`), as for a shell unpacked
//! from a package rather than installed.
//!
//! Sources: the manuals of zsh (zshparam `module_path`, zshoptions
//! `LOGIN`, `PROMPT_SP`, zshzle and the terminfo keys `khome`, `kend`,
//! `kdch1`, `smkx`), fish (`status is-login`, `fish_prompt`), nushell
//! (`$nu.is-login`, `config.nu`, `PROMPT_INDICATOR`), dash(1) (`-l`,
//! `.profile`) and tcsh(1) (`loginsh`, `.tcshrc`, `prompt`); stty(1)
//! (`size`).
#![cfg(unix)]
#![allow(clippy::print_stdout)]

use std::path::{Path, PathBuf};

use crate::common::{Home, Running, prompt, row_starting, start};
use bitbraid_terminal::{GridSize, Input, Key, Modifiers, ShellKind, ShellSpec, quote_path};

/// The folder of the shells, when the tests are asked for.
fn shells_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BITBRAID_TEST_SHELLS")?);
    dir.is_dir().then_some(dir)
}

/// The shells known here: their program, and whether a line editor
/// answers the keys that move the cursor.
const SHELLS: &[(&str, bool)] = &[
    ("zsh", true),
    ("fish", true),
    ("nu", true),
    ("dash", false),
    ("tcsh", true),
];

/// The shells of the folder, by program and path; none (and a word said)
/// when the tests are not asked for.
fn shells(test: &str) -> Vec<(&'static str, bool, PathBuf)> {
    let Some(dir) = shells_dir() else {
        println!("{test}: skipped: BITBRAID_TEST_SHELLS does not name a folder of shells");
        return Vec::new();
    };
    let found: Vec<_> = SHELLS
        .iter()
        .filter_map(|(name, edits)| {
            let path = dir.join(name);
            path.is_file().then_some((*name, *edits, path))
        })
        .collect();
    assert!(
        !found.is_empty(),
        "no shell known here in {}",
        dir.display()
    );
    found
}

/// Write `text` to `path`, making the folders it is in.
fn put(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// Where zsh's modules are when they lie beside its program.
fn zsh_modules(program: &Path) -> Option<PathBuf> {
    let lib = program.parent()?.parent()?.join("lib").join("zsh");
    std::fs::read_dir(lib)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.join("zsh").is_dir())
}

/// The spec that starts `name` from `program` in `home`, with its startup
/// files written there: a prompt of `$ ` and nothing else of note.
fn spec_for(name: &str, program: &Path, home: &Home, args: &[&str]) -> ShellSpec {
    let at = &home.0;
    let config = at.join("config");
    let mut env = vec![
        ("PATH".to_owned(), "/usr/bin:/bin".to_owned()),
        ("HOME".to_owned(), at.display().to_string()),
        ("TERM".to_owned(), "xterm-256color".to_owned()),
        ("LC_ALL".to_owned(), "C.UTF-8".to_owned()),
        ("XDG_CONFIG_HOME".to_owned(), config.display().to_string()),
        (
            "XDG_DATA_HOME".to_owned(),
            at.join("data").display().to_string(),
        ),
        (
            "XDG_CACHE_HOME".to_owned(),
            at.join("cache").display().to_string(),
        ),
    ];
    match name {
        "zsh" => {
            let dot = at.join("zdot");
            let modules = zsh_modules(program)
                .map(|dir| format!("module_path=('{}')\n", dir.display()))
                .unwrap_or_default();
            put(&dot.join(".zshenv"), &modules);
            // A prompt of the test's, and the keys most setups bind: Home,
            // End and Delete by what the terminal says they send, which
            // they send once the keypad is put in its application mode
            // while a line is edited.
            put(
                &dot.join(".zshrc"),
                "PS1='$ '\nRPS1=''\nunsetopt PROMPT_SP\n\
                 bindkey -e\n\
                 bindkey \"${terminfo[khome]}\" beginning-of-line\n\
                 bindkey \"${terminfo[kend]}\" end-of-line\n\
                 bindkey \"${terminfo[kdch1]}\" delete-char\n\
                 zle-line-init() { echoti smkx }\nzle-line-finish() { echoti rmkx }\n\
                 zle -N zle-line-init\nzle -N zle-line-finish\n",
            );
            env.push(("ZDOTDIR".to_owned(), dot.display().to_string()));
        }
        "fish" => put(
            &config.join("fish").join("config.fish"),
            "set -g fish_greeting ''\nfunction fish_prompt; printf '$ '; end\n\
             function fish_right_prompt; end\nset -g fish_autosuggestion_enabled 0\n",
        ),
        "nu" => {
            put(&config.join("nushell").join("env.nu"), "");
            put(
                &config.join("nushell").join("config.nu"),
                "$env.config.show_banner = false\n\
                 $env.PROMPT_COMMAND = {|| '' }\n\
                 $env.PROMPT_COMMAND_RIGHT = {|| '' }\n\
                 $env.PROMPT_INDICATOR = {|| '$ ' }\n\
                 $env.config.hinter = {}\n",
            );
            put(&config.join("nushell").join("login.nu"), "");
        }
        "dash" => {
            env.push(("PS1".to_owned(), "$ ".to_owned()));
            // Read by a login shell only.
            put(&at.join(".profile"), "PROFILE_READ=1\n");
        }
        "tcsh" => {
            // tcsh binds the arrows, Home and End itself; Delete is bound
            // by the setups that ship with it, as here.
            put(
                &at.join(".tcshrc"),
                "set prompt='$ '\nbindkey '^[[3~' delete-char\n",
            );
            put(&at.join(".login"), "");
        }
        _ => {}
    }
    ShellSpec {
        program: program.display().to_string(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: at.clone(),
        env,
    }
}

/// The shell at its first prompt.
fn started(name: &str, program: &Path, home: &Home, args: &[&str]) -> Running {
    let running = start(
        &spec_for(name, program, home, args),
        GridSize::new(80, 24, 8, 16),
    );
    running.until(&format!("{name}'s prompt"), prompt);
    running
}

/// A line that prints `what` (a word) in this shell.
fn say(name: &str, what: &str) -> String {
    match name {
        "nu" => format!("print {what}"),
        _ => format!("echo {what}"),
    }
}

/// A line that prints the folder the shell is in.
fn print_folder(name: &str) -> &'static str {
    match name {
        "nu" => "print (pwd)",
        _ => "pwd",
    }
}

/// A line that prints `LOGIN` when the shell is a login shell.
fn says_if_login(name: &str) -> &'static str {
    match name {
        "zsh" => "[[ -o login ]] && echo LOGIN",
        "fish" => "status is-login; and echo LOGIN",
        "nu" => "if $nu.is-login { print LOGIN }",
        // dash has no way to ask: `-l` reads `.profile`, which the test
        // has write a mark.
        "dash" => "[ -n \"$PROFILE_READ\" ] && echo LOGIN",
        "tcsh" => "if ($?loginsh) echo LOGIN",
        _ => "false",
    }
}

/// Wait until a row reads exactly `text`.
fn shows(running: &Running, what: &str, text: &str) {
    running.until(what, |frame| {
        frame.rows.iter().any(|row| row.text() == text)
    });
}

/// Type a line, press Return, and wait for `marker` on a row of its own
/// and the prompt after it. The line and the marker are separate
/// commands in every shell here.
fn run(running: &Running, name: &str, line: &str, marker: &str) {
    running.type_line(&format!("{line}; {}", say(name, marker)));
    running.until(marker, |frame| {
        frame.rows.iter().any(|row| row.text() == marker) && prompt(frame)
    });
}

fn key(running: &Running, key: Key) {
    running.session.input(Input::Key(key, Modifiers::NONE));
}

#[test]
fn other_shells_start_where_they_are_asked_and_are_followed() {
    for (name, _, program) in shells("other_shells_start_where_they_are_asked") {
        let home = Home::new(&format!("shell-folder-{name}"));
        std::fs::create_dir_all(home.0.join("deeper")).unwrap();
        let running = started(name, &program, &home, &[]);
        assert_eq!(running.session.current_folder(), home.0, "{name}");
        run(&running, name, print_folder(name), "HERE");
        let frame = running.frame();
        assert!(
            frame
                .text()
                .replace('\n', "")
                .contains(&home.0.display().to_string()),
            "{name}: {}",
            frame.text()
        );
        run(&running, name, "cd deeper", "MOVED");
        let started_at = std::time::Instant::now();
        while running.session.current_folder() != home.0.join("deeper") {
            assert!(
                started_at.elapsed() < crate::common::WAIT,
                "{name}: the new folder was not followed"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // The shell itself is in front, and a job it runs is not
        // (nushell's own commands are no jobs: the program is asked for).
        // The job says so once it runs: a Ctrl+C that comes while the
        // shell is still starting it can be lost, and `sleep` goes on.
        assert!(running.session.shell_in_front(), "{name}");
        running.type_line(if name == "nu" {
            "^sh -c 'echo STARTED; exec sleep 30'"
        } else {
            "sh -c 'echo STARTED; exec sleep 30'"
        });
        running.until(&format!("{name}'s job to start"), |frame| {
            frame.rows.iter().any(|row| row.text() == "STARTED")
        });
        let started_at = std::time::Instant::now();
        while running.session.shell_in_front() {
            assert!(
                started_at.elapsed() < crate::common::WAIT,
                "{name}: the job was never in front"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        running
            .session
            .input(Input::Key(Key::Character('c'), Modifiers::CONTROL));
        running.until(&format!("{name}'s prompt again"), prompt);
    }
}

#[test]
fn other_shells_start_as_login_shells_with_l() {
    for (name, _, program) in shells("other_shells_start_as_login_shells") {
        let home = Home::new(&format!("shell-login-{name}"));
        let running = started(name, &program, &home, &["-l"]);
        run(&running, name, says_if_login(name), "ASKED");
        assert!(
            row_starting(&running.frame(), "LOGIN").is_some(),
            "{name}: {}",
            running.frame().text()
        );
        // And not one without it.
        let plain = started(name, &program, &home, &[]);
        run(&plain, name, says_if_login(name), "ASKED");
        assert!(
            row_starting(&plain.frame(), "LOGIN").is_none(),
            "{name}: {}",
            plain.frame().text()
        );
    }
}

#[test]
fn the_keys_of_a_line_editor_reach_other_shells() {
    for (name, edits, program) in shells("the_keys_of_a_line_editor_reach_other_shells") {
        let home = Home::new(&format!("shell-keys-{name}"));
        let running = started(name, &program, &home, &[]);
        let echo = say(name, "");
        let echo = echo.trim_end();
        if edits {
            // "ech wrld": back three, `o`; Home, right three, `o`; End,
            // `s`, Backspace: the line reads "echo world" ("prin wrld"
            // becomes "print world" the same way).
            let (verb, last) = echo.split_at(echo.len().saturating_sub(1));
            running.session.input(Input::Text(format!("{verb} wrld")));
            for _ in 0..3 {
                key(&running, Key::Left);
            }
            running.session.input(Input::Text("o".into()));
            key(&running, Key::Home);
            for _ in 0..verb.len() {
                key(&running, Key::Right);
            }
            running.session.input(Input::Text(last.into()));
            key(&running, Key::End);
            running.session.input(Input::Text("s".into()));
            key(&running, Key::Backspace);
            key(&running, Key::Enter);
            shows(
                &running,
                &format!("{name}: the edited line's output"),
                "world",
            );
            running.until(&format!("{name}'s prompt"), prompt);
            // Up brings the line back from the history.
            key(&running, Key::Up);
            key(&running, Key::Enter);
            running.until(&format!("{name}: the line again"), |frame| {
                frame
                    .rows
                    .iter()
                    .filter(|row| row.text() == "world")
                    .count()
                    == 2
                    && prompt(frame)
            });
            // Delete takes the character under the cursor; Ctrl+A and
            // Ctrl+E go to either end; Ctrl+U takes the whole line.
            running.session.input(Input::Text(format!("{echo} zaap")));
            key(&running, Key::Left);
            key(&running, Key::Left);
            key(&running, Key::Delete);
            running
                .session
                .input(Input::Key(Key::Character('a'), Modifiers::CONTROL));
            running
                .session
                .input(Input::Key(Key::Character('e'), Modifiers::CONTROL));
            key(&running, Key::Enter);
            shows(&running, &format!("{name}: Delete"), "zap");
            running.until(&format!("{name}'s prompt"), prompt);
            running.session.input(Input::Text(format!("{echo} never")));
            running
                .session
                .input(Input::Key(Key::Character('u'), Modifiers::CONTROL));
            run(&running, name, &say(name, "after"), "AFTER");
            assert!(
                !running.frame().text().contains("never\n"),
                "{name}: {}",
                running.frame().text()
            );
        } else {
            run(&running, name, &say(name, "plain"), "PLAIN");
            shows(&running, name, "plain");
        }
        // A paste arrives as typed text, bracketed or not as the shell
        // asked.
        running.session.paste(&say(name, "pasted"));
        key(&running, Key::Enter);
        shows(&running, &format!("{name}: the paste"), "pasted");
        running.until(&format!("{name}'s prompt"), prompt);
    }
}

#[test]
fn other_shells_see_a_new_size_and_end_with_their_code() {
    for (name, _, program) in shells("other_shells_see_a_new_size") {
        let home = Home::new(&format!("shell-size-{name}"));
        let running = started(name, &program, &home, &[]);
        run(&running, name, "stty size", "SIZED");
        assert!(row_starting(&running.frame(), "24 80").is_some(), "{name}");
        running.session.resize(GridSize::new(100, 30, 8, 16));
        run(&running, name, "stty size", "RESIZED");
        assert!(
            row_starting(&running.frame(), "30 100").is_some(),
            "{name}: {}",
            running.frame().text()
        );
        running.type_line("exit 3");
        assert_eq!(running.wait_exit(), Some(3), "{name}");
    }
}

/// Paths dropped on the terminal are pasted at the prompt, quoted for
/// the shell's kind, and the shell hands them on whole: typed after a
/// command that prints its argument, each comes back on a row of its own.
#[test]
fn other_shells_read_dropped_paths_back_whole() {
    let paths: &[&str] = &[
        "/a b/it's",
        "/$HOME/`id`/$(id)",
        "/*/?/[a]/{x,y}/~",
        "/!x/!!/^a^b",
        "/#c/a;b/a&b/a|b/<i>",
        "/\"q\"/a\\b/(p)",
        "/grüße 世界",
        "/%1/@x/+y/,z/:w",
    ];
    for (name, _, program) in shells("other_shells_read_dropped_paths_back_whole") {
        let kind = ShellKind::of_program(&program.display().to_string());
        let home = Home::new(&format!("shell-paths-{name}"));
        let running = started(name, &program, &home, &[]);
        let print = match name {
            "nu" => "print ",
            _ => "printf '%s\\n' ",
        };
        for path in paths {
            let quoted = quote_path(Path::new(path), kind);
            running.session.input(Input::Text(print.to_owned()));
            running.session.paste(&quoted);
            key(&running, Key::Enter);
            running.until(&format!("{name}: {path:?} as {quoted:?}"), |frame| {
                frame.rows.iter().any(|row| row.text() == *path) && prompt(frame)
            });
            running.type_line("clear");
            running.until(&format!("{name}: a clear screen"), |frame| {
                frame.text().trim() == "$"
            });
        }
    }
}
