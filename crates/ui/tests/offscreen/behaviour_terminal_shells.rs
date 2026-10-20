//! The terminal with shells other than bash, as `behaviour.rs` asks the
//! window: each of zsh, fish, nushell, dash and tcsh, chosen in the
//! preferences, starts in the repository, a branch made in it shows in the
//! graph, and `exit` takes its tab and the panel; a git command typed at
//! its own prompt, as it comes without settings (nushell's with a clock on
//! the right), opens the list of completions with the rest of the marked
//! entry after the cursor, and Enter takes it.
//!
//! Opt-in: `BITBRAID_TEST_SHELLS` names a folder holding the shells'
//! programs (any that are missing are left out); without it the tests say
//! they were skipped. The shells run in the scratch surroundings of these
//! tests and read no settings but zsh's module path, which a zsh unpacked
//! from a package rather than installed needs (`../lib/zsh/<version>`
//! beside its program).
//!
//! Sources: the manuals of zsh (zshparam `module_path`, the default
//! `PS1`), fish (`fish_prompt`), nushell (`PROMPT_INDICATOR`,
//! `PROMPT_COMMAND_RIGHT`), dash(1) and tcsh(1) (`prompt`).

use std::path::{Path, PathBuf};

use bitbraid_terminal::ShellSpec;

use crate::{
    behaviour_terminal::run,
    behaviour_terminal_completion::open_with,
    behaviour_terminal_text::wait,
    repos,
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "other_shells_start_in_the_repository_refresh_the_graph_and_end",
        run: other_shells_start_in_the_repository_refresh_the_graph_and_end,
    },
    Test {
        name: "other_shells_complete_git_commands_with_a_hint",
        run: other_shells_complete_git_commands_with_a_hint,
    },
];

const SHELLS: &[&str] = &["zsh", "fish", "nu", "dash", "tcsh"];

/// The shells of the folder; none (and a word said) when the tests are
/// not asked for.
fn shells(test: &str) -> Vec<(&'static str, PathBuf)> {
    let Some(dir) = std::env::var_os("BITBRAID_TEST_SHELLS")
        .map(PathBuf::from)
        .filter(|dir| dir.is_dir())
    else {
        println!("{test}: skipped: BITBRAID_TEST_SHELLS does not name a folder of shells");
        return Vec::new();
    };
    let found: Vec<_> = SHELLS
        .iter()
        .map(|name| (*name, dir.join(name)))
        .filter(|(_, path)| path.is_file())
        .collect();
    assert!(
        !found.is_empty(),
        "no shell known here in {}",
        dir.display()
    );
    found
}

/// Tell a zsh unpacked beside its modules where they are, in `home`.
fn zsh_modules(program: &Path, home: &Path) {
    let modules = program
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("lib").join("zsh"))
        .and_then(|lib| std::fs::read_dir(lib).ok())
        .and_then(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.join("zsh").is_dir())
        });
    if let Some(modules) = modules {
        std::fs::write(
            home.join(".zshenv"),
            format!("module_path=('{}')\n", modules.display()),
        )
        .unwrap();
    }
    std::fs::write(home.join(".zshrc"), "").unwrap();
}

/// Whether the last line with text on show ends the way the shell's own
/// prompt does: `%` (zsh), `>` (fish, nushell, tcsh), `$` (dash); a
/// clock nushell keeps on the right may follow.
fn at_a_prompt(text: &str) -> bool {
    text.lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .is_some_and(|line| {
            let line = line.trim_end();
            let left = line.split("   ").next().unwrap_or(line).trim_end();
            left.ends_with('%') || left.ends_with('>') || left.ends_with('$')
        })
}

/// Each shell from the preferences: in the repository, a branch made in
/// it reaches the references, and `exit` takes the panel with it.
fn other_shells_start_in_the_repository_refresh_the_graph_and_end() {
    for (name, program) in shells("other_shells_start_in_the_repository") {
        repos::fresh();
        let repo = repos::project();
        if name == "zsh" {
            let home = PathBuf::from(std::env::var_os("HOME").unwrap());
            zsh_modules(&program, &home);
        }
        let mut stage = Stage::open(Start::with(&repo).preferences(&format!(
            "[terminal_panel]\nshell = \"{}\"\n",
            program.display()
        )));
        stage.click("terminal");
        stage.until(&format!("{name} at its prompt"), |stage| {
            stage.terminal().screen.as_deref().is_some_and(at_a_prompt)
        });
        let pwd = if name == "nu" { "print (pwd)" } else { "pwd" };
        run(&mut stage, pwd);
        stage.until(&format!("{name} says where it is"), |stage| {
            stage
                .terminal()
                .screen
                .is_some_and(|screen| screen.contains(&*repo.to_string_lossy()))
        });
        let before = stage.workspace().branches;
        run(&mut stage, &format!("git branch typed-in-{name}"));
        stage.until(
            &format!("{name}: the new branch is in the references"),
            |stage| stage.workspace().branches == before + 1,
        );
        run(&mut stage, "exit");
        stage.until(&format!("{name}: the panel goes with the shell"), |stage| {
            !stage.terminal().shown
        });
        assert!(stage.terminal().ids.is_empty(), "{name}");
    }
}

/// Each shell as it comes: `git ch` lists checkout first with its rest
/// after the cursor, and Enter takes it into the line.
fn other_shells_complete_git_commands_with_a_hint() {
    for (name, program) in shells("other_shells_complete_git_commands") {
        let make = {
            let program = program.clone();
            move |repo: &Path| {
                let home = repo.with_extension("home");
                std::fs::create_dir_all(&home).unwrap();
                if name == "zsh" {
                    zsh_modules(&program, &home);
                }
                let at = |sub: &str| home.join(sub).display().to_string();
                ShellSpec {
                    program: program.display().to_string(),
                    args: Vec::new(),
                    cwd: repo.to_path_buf(),
                    env: vec![
                        ("PATH".into(), "/usr/bin:/bin".into()),
                        ("HOME".into(), home.display().to_string()),
                        ("XDG_CONFIG_HOME".into(), at("config")),
                        ("XDG_DATA_HOME".into(), at("data")),
                        ("TERM".into(), "xterm-256color".into()),
                        ("LC_ALL".into(), "C.UTF-8".into()),
                    ],
                }
            }
        };
        let (mut stage, host) = open_with(
            Start::bare(),
            &format!("complete-{name}"),
            make,
            at_a_prompt,
        );
        stage.type_text("git ch");
        let probe = wait(&mut stage, &host, &format!("{name}: the list"), |probe| {
            probe.completions == ["checkout", "cherry-pick", "sparse-checkout"]
        });
        assert_eq!(
            probe.hint.as_deref(),
            Some("eckout"),
            "{name}: on show:\n{}",
            probe.text
        );
        stage.keys("enter");
        wait(
            &mut stage,
            &host,
            &format!("{name}: checkout taken"),
            |probe| probe.completions.is_empty() && probe.text.contains("git checkout"),
        );
    }
}
