//! What the window does, as `behaviour.rs` asks it, when a git command is
//! typed in a terminal: the list of completions opens at the cursor, keys
//! move through it and take from it, Escape puts it away, and the
//! preferences turn it off or give Tab to it. The terminal runs `/bin/sh`
//! in a copy of a fixture repository, with the test's environment, as
//! `behaviour_terminal_text.rs` sets it up; the names it completes with are
//! the engine's (`terminal.completion_words`).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use bitbraid_engine::{CommandContext, builtin_registry, completion::Words};
use bitbraid_terminal::{GridSize, Session, SessionEvent, ShellSpec};
use bitbraid_ui::prefs::Prefs;
use bitbraid_ui::views::terminal::{
    completion,
    text::{TerminalText, TerminalTextProbe},
};
use gpui_kit::{AppContext as _, Entity};
use serde_json::json;

use crate::{
    behaviour_terminal_text::{Host, focus_text, home, shown, spec, wait},
    runner::Test,
    stage::{Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "terminal_completion_offers_git_commands_and_takes_one_with_enter",
        run: terminal_completion_offers_git_commands_and_takes_one_with_enter,
    },
    Test {
        name: "terminal_completion_moves_through_branches_and_is_put_away",
        run: terminal_completion_moves_through_branches_and_is_put_away,
    },
    Test {
        name: "terminal_completion_follows_the_preferences",
        run: terminal_completion_follows_the_preferences,
    },
    Test {
        name: "terminal_completion_is_only_for_the_shell",
        run: terminal_completion_is_only_for_the_shell,
    },
    Test {
        name: "terminal_completion_reads_the_repositorys_names_and_follows_them",
        run: terminal_completion_reads_the_repositorys_names_and_follows_them,
    },
    Test {
        name: "terminal_completion_hints_short_of_a_prompt_on_the_right",
        run: terminal_completion_hints_short_of_a_prompt_on_the_right,
    },
    Test {
        name: "terminal_completion_follows_an_echo_that_came_unannounced",
        run: terminal_completion_follows_an_echo_that_came_unannounced,
    },
];

/// The shell of [`spec`] with a prompt that also writes a clock at
/// `column` of its row and comes back, as nushell's (and zsh's, fish's
/// and tcsh's, where it is set) prompt on the right does.
fn right_prompt_at(column: u16) -> impl FnOnce(&Path) -> ShellSpec {
    move |repo| {
        let mut shell = spec(repo);
        for (name, value) in &mut shell.env {
            if name == "PS1" {
                *value = format!("$ \u{1b}7\u{1b}[{column}G12:00\u{1b}8");
            }
        }
        shell
    }
}

/// The rest of the marked entry is shown after the cursor where the cells
/// it takes are blank, short of a prompt the shell keeps on the right
/// with a cell to spare; where the prompt stands closer, it is not shown
/// and the list is. Shown only when nothing at all followed the cursor, it
/// would never be shown at nushell's prompt.
fn terminal_completion_hints_short_of_a_prompt_on_the_right() {
    for (column, hinted) in [(60, Some("eckout")), (12, None)] {
        let (mut stage, host) = open_with(
            Start::bare(),
            &format!("complete-right-{column}"),
            right_prompt_at(column),
            |text| {
                text.lines()
                    .any(|line| line.starts_with("$ ") && line.ends_with("12:00"))
            },
        );
        stage.type_text("git ch");
        let probe = wait(&mut stage, &host, "the list", |probe| {
            probe.completions == ["checkout", "cherry-pick", "sparse-checkout"]
        });
        assert_eq!(probe.hint.as_deref(), hinted, "a prompt at column {column}");
        assert!(
            probe
                .text
                .lines()
                .any(|line| line.starts_with("$ git ch") && line.ends_with("12:00")),
            "{}",
            probe.text
        );
    }
}

/// The list follows the line on show even when the shell's echo of the
/// last key comes after the list was worked out and no word of it comes
/// after: a session says that output came once until a frame is taken,
/// so an echo that lands between the two is not announced. The frame
/// works the list out again; otherwise the list would stay a key behind.
fn terminal_completion_follows_an_echo_that_came_unannounced() {
    let repo = home("complete-unannounced");
    bitbraid_testkit::copy_fixture("branches-merges", &repo).expect("the fixture is copied");
    let words = Arc::new(words_of(&repo));
    let shell = spec(&repo);
    // What the session says reaches the text only when this test hands it
    // on, as the text's own listener would.
    let (said, heard) = std::sync::mpsc::channel::<SessionEvent>();
    let (mut stage, host) = Stage::open_root(Start::bare(), move |window, cx| {
        let session = Session::spawn(&shell, GridSize::default(), move |event| {
            let _ = said.send(event);
        })
        .expect("the shell starts on a terminal");
        let text = cx.new(|cx| TerminalText::new(session, window, cx));
        Host::showing(text, cx)
    });
    let text = stage.app(|cx| host.read(cx).text.clone());
    let hand_on = |stage: &mut Stage| {
        let events: Vec<SessionEvent> = heard.try_iter().collect();
        let text = text.clone();
        stage.app(move |cx| {
            text.update(cx, |text, cx| {
                for event in events {
                    text.on_session_event(event, cx);
                }
            });
        });
    };
    let until_said = |stage: &mut Stage, what: &str, met: &dyn Fn(&TerminalTextProbe) -> bool| {
        let started = std::time::Instant::now();
        loop {
            hand_on(stage);
            stage.frame();
            let probe = shown(stage, &host);
            if met(&probe) {
                return probe;
            }
            assert!(
                started.elapsed() < Duration::from_secs(60),
                "waited for this and it did not come: {what}; on show:\n{}",
                probe.text
            );
            std::thread::park_timeout(Duration::from_millis(20));
        }
    };
    until_said(&mut stage, "the first prompt", &|probe| {
        probe.text.lines().any(|line| line.starts_with('$'))
    });
    stage.app(|cx| text.update(cx, |text, _| text.set_completion_words(words)));
    focus_text(&mut stage, &host);
    stage.type_text("git che");
    until_said(&mut stage, "the list for che", &|probe| {
        last_line(probe) == "$ git che"
            && probe.completions == ["checkout", "cherry-pick", "sparse-checkout"]
    });
    // One more key, whose echo is never announced.
    stage.type_text("c");
    let started = std::time::Instant::now();
    let probe = loop {
        stage.frame();
        let probe = shown(&mut stage, &host);
        if last_line(&probe) == "$ git chec" {
            break probe;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the echo did not come: {}",
            probe.text
        );
        std::thread::park_timeout(Duration::from_millis(20));
    };
    let _unannounced = heard.try_iter().count();
    stage.frame();
    let probe_after = shown(&mut stage, &host);
    assert_eq!(
        probe_after.completions,
        ["checkout", "sparse-checkout"],
        "{}",
        probe.text
    );
    stage.keys("ctrl-u");
}

/// The names of the repository at `path`, as the workspace reads them.
pub fn words_of(path: &std::path::Path) -> Words {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": path }))
        .expect("the fixture opens");
    let value = registry
        .run("terminal.completion_words", &mut ctx, &json!({}))
        .expect("its names are read");
    serde_json::from_value(value).expect("the names come as they are kept")
}

/// A terminal in a copy of the fixture with two feature branches and a
/// tag, with its names given to it and the first prompt on show.
pub fn open(start: Start, name: &str) -> (Stage, Entity<Host>) {
    open_with(start, name, spec, |text| {
        text.lines().any(|line| line.starts_with('$'))
    })
}

/// [`open`] with the shell `make` makes for the repository's folder, at
/// the first prompt as `prompted` knows it.
pub fn open_with(
    start: Start,
    name: &str,
    make: impl FnOnce(&Path) -> ShellSpec,
    prompted: impl Fn(&str) -> bool,
) -> (Stage, Entity<Host>) {
    let repo = home(name);
    bitbraid_testkit::copy_fixture("branches-merges", &repo).expect("the fixture is copied");
    let words = Arc::new(words_of(&repo));
    let shell = make(&repo);
    let (mut stage, host) =
        Stage::open_root(start, move |window, cx| Host::with(&shell, window, cx));
    wait(&mut stage, &host, "the first prompt", |probe| {
        prompted(&probe.text)
    });
    let given = host.clone();
    stage.app(move |cx| {
        let text = given.read(cx).text.clone();
        text.update(cx, |text, _| text.set_completion_words(words));
    });
    focus_text(&mut stage, &host);
    (stage, host)
}

/// The last row with text on show.
fn last_line(probe: &TerminalTextProbe) -> String {
    probe
        .text
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default()
        .to_owned()
}

/// Wait until the line typed reads `line` and the list says what `listed`
/// accepts.
fn until(
    stage: &mut Stage,
    host: &Entity<Host>,
    line: &str,
    listed: impl Fn(&[String]) -> bool,
) -> TerminalTextProbe {
    wait(stage, host, &format!("{line:?} and its list"), |probe| {
        last_line(probe) == line && listed(&probe.completions)
    })
}

/// Typing `git ` lists git's commands, `ch` narrows them to those that
/// begin with it, checkout marked and its rest shown after the cursor;
/// Enter takes it into the shell, and a word that is whole closes the
/// list, so that Enter runs it.
fn terminal_completion_offers_git_commands_and_takes_one_with_enter() {
    let (mut stage, host) = open(Start::bare(), "complete-command");
    stage.type_text("git ");
    let probe = until(&mut stage, &host, "$ git", |listed| {
        listed.first().map(String::as_str) == Some("status")
    });
    assert_eq!(probe.hint, None, "nothing typed, nothing to show after it");
    // 8 left of the cursor; eight rows do not fit under the first row of
    // a terminal this high, so above it, 6 clear.
    let (x, y) = probe.origin.unwrap();
    let (width, height) = probe.cell.unwrap();
    let (row, column) = probe.cursor.unwrap();
    let (list_x, list_y) = probe.list_at.unwrap();
    assert!((list_x - (x + width * f32::from(column) - completion::LEFT_OF_CURSOR)).abs() < 0.5);
    assert!(probe.completions.len() >= completion::ROWS_SHOWN);
    let high = completion::ROW_HEIGHT * completion::ROWS_SHOWN as f32;
    let above = y + height * f32::from(row) - completion::GAP_ABOVE - high;
    assert!((list_y - above).abs() < 0.5, "{list_y} {above}");
    // Fewer fit under the row.
    stage.type_text("ch");
    let probe = until(&mut stage, &host, "$ git ch", |listed| {
        listed == ["checkout", "cherry-pick", "sparse-checkout"]
    });
    assert_eq!(probe.marked, Some(0));
    assert_eq!(probe.hint.as_deref(), Some("eckout"));
    let (list_x, list_y) = probe.list_at.unwrap();
    let column = column + 2;
    assert!((list_x - (x + width * f32::from(column) - completion::LEFT_OF_CURSOR)).abs() < 0.5);
    assert!((list_y - (y + height * (f32::from(row) + 1.0))).abs() < 0.5);
    stage.keys("enter");
    let probe = until(&mut stage, &host, "$ git checkout", <[String]>::is_empty);
    assert_eq!(probe.hint, None);
    // The word is whole: Enter goes to the shell and runs the command.
    stage.keys("enter");
    wait(&mut stage, &host, "git to answer", |probe| {
        probe
            .text
            .lines()
            .filter(|line| line.starts_with('$'))
            .count()
            >= 2
    });
}

/// Branch names are offered after `git checkout`; Down marks the next,
/// Enter takes it; Escape puts the list away for the word, Ctrl+Space
/// brings it back.
fn terminal_completion_moves_through_branches_and_is_put_away() {
    let (mut stage, host) = open(Start::bare(), "complete-branch");
    stage.type_text("git checkout feat");
    until(&mut stage, &host, "$ git checkout feat", |listed| {
        listed == ["feature/a", "feature/b"]
    });
    stage.keys("down");
    let probe = until(&mut stage, &host, "$ git checkout feat", |listed| {
        !listed.is_empty()
    });
    assert_eq!(probe.marked, Some(1));
    assert_eq!(probe.hint.as_deref(), Some("ure/b"));
    stage.keys("enter");
    until(
        &mut stage,
        &host,
        "$ git checkout feature/b",
        <[String]>::is_empty,
    );
    // A tag, put away and brought back.
    stage.type_text(" v");
    until(&mut stage, &host, "$ git checkout feature/b v", |listed| {
        listed.first().map(String::as_str) == Some("v1.0")
    });
    stage.keys("escape");
    until(
        &mut stage,
        &host,
        "$ git checkout feature/b v",
        <[String]>::is_empty,
    );
    stage.type_text("1");
    stage.pass(Duration::from_millis(100));
    let probe = until(&mut stage, &host, "$ git checkout feature/b v1", |_| true);
    assert!(probe.completions.is_empty(), "put away for this word");
    stage.keys("ctrl-space");
    until(&mut stage, &host, "$ git checkout feature/b v1", |listed| {
        listed == ["v1.0"]
    });
    // A click takes the one clicked.
    let (x, y) = shown_list_row(&mut stage, &host, 0);
    stage.click_at((x, y));
    until(
        &mut stage,
        &host,
        "$ git checkout feature/b v1.0",
        <[String]>::is_empty,
    );
    stage.keys("ctrl-u");
}

/// Where the middle of a row of the list is.
fn shown_list_row(stage: &mut Stage, host: &Entity<Host>, row: usize) -> (f32, f32) {
    let probe = wait(stage, host, "the list", |probe| probe.list_at.is_some());
    let (x, y) = probe.list_at.unwrap();
    (
        x + completion::LIST_WIDTH / 2.0,
        y + completion::ROW_HEIGHT * (row as f32 + 0.5),
    )
}

/// Tab goes to the shell at first and takes the one marked when the
/// preferences say so; with completion off nothing is offered.
fn terminal_completion_follows_the_preferences() {
    let (mut stage, host) = open(Start::bare(), "complete-settings");
    stage.app(|cx| {
        cx.global_mut::<Prefs>()
            .preferences
            .terminal_panel
            .completion_tab = "accept".to_owned();
    });
    stage.type_text("git sta");
    until(&mut stage, &host, "$ git sta", |listed| {
        listed.first().map(String::as_str) == Some("status")
    });
    stage.keys("tab");
    until(&mut stage, &host, "$ git status", <[String]>::is_empty);
    stage.keys("ctrl-u");
    stage.app(|cx| {
        cx.global_mut::<Prefs>()
            .preferences
            .terminal_panel
            .completion = false;
    });
    stage.type_text("git sta");
    stage.pass(Duration::from_millis(200));
    let probe = until(&mut stage, &host, "$ git sta", |_| true);
    assert!(probe.completions.is_empty(), "off: {:?}", probe.completions);
    stage.keys("ctrl-u");
}

/// A program the shell started has the terminal: what is typed to it is
/// not completed; once the shell is back, it is.
fn terminal_completion_is_only_for_the_shell() {
    let (mut stage, host) = open(Start::bare(), "complete-front");
    stage.type_text("echo READY; cat");
    stage.keys("enter");
    wait(&mut stage, &host, "cat to start", |probe| {
        probe.text.lines().any(|line| line == "READY")
    });
    stage.type_text("git ch");
    let probe = until(&mut stage, &host, "git ch", |_| true);
    assert!(probe.completions.is_empty(), "{:?}", probe.completions);
    stage.keys("enter ctrl-d");
    wait(&mut stage, &host, "the prompt again", |probe| {
        last_line(probe) == "$"
    });
    stage.type_text("git ch");
    until(&mut stage, &host, "$ git ch", |listed| !listed.is_empty());
    stage.keys("ctrl-u");
}

/// The window with the project open and its terminal at the prompt, the
/// names of the repository handed to it by the tab.
fn window_with_terminal(pictures: bool) -> Stage {
    crate::repos::fresh();
    let repo = crate::repos::project();
    let start = Start::with(&repo).preferences(crate::behaviour_terminal::SHELL);
    let mut stage = Stage::open(if pictures { start.pictures() } else { start });
    stage.click("terminal");
    crate::behaviour_terminal::at_prompt(&mut stage);
    stage.until("the names to complete with", |stage| {
        stage.terminal().completion_branches.is_some()
    });
    stage
}

/// Type into the terminal on show and wait until its list says what
/// `listed` accepts.
fn type_and_list(stage: &mut Stage, text: &str, listed: impl Fn(&[String]) -> bool) {
    stage.type_text(text);
    stage.until(&format!("the list after {text:?}"), |stage| {
        listed(&stage.terminal().completions)
    });
}

/// In the window, the tab reads the names its terminals complete with
/// when one starts, and again when a command typed in one is done: a
/// branch made in the terminal is offered next.
fn terminal_completion_reads_the_repositorys_names_and_follows_them() {
    let mut stage = window_with_terminal(false);
    let branches = stage.terminal().completion_branches.unwrap();
    assert!(branches >= 1);
    type_and_list(&mut stage, "git checkout ", |listed| {
        listed.iter().any(|label| label == "main")
    });
    stage.keys("ctrl-u");
    crate::behaviour_terminal::run(&mut stage, "git branch made-in-the-terminal");
    stage.until("the new branch among the names", |stage| {
        stage.terminal().completion_branches == Some(branches + 1)
    });
    crate::behaviour_terminal::at_prompt(&mut stage);
    type_and_list(&mut stage, "git checkout made", |listed| {
        listed == ["made-in-the-terminal"]
    });
    stage.keys("ctrl-u");
}

/// The list at the prompt of the terminal under the graph: above the
/// cursor's row over the graph, where eight rows do not fit below; under
/// it where they do.
pub fn completion_pictures(shots: &mut crate::runner::Shots) {
    let mut stage = window_with_terminal(true);
    type_and_list(&mut stage, "git checkout ", |listed| {
        listed.len() >= completion::ROWS_SHOWN && listed.iter().any(|label| label == "main")
    });
    shots.take(
        &mut stage,
        "terminal_completion",
        crate::runner::Themes::All,
    );
    stage.keys("ctrl-u");
    type_and_list(&mut stage, "git sta", |listed| {
        listed.first().map(String::as_str) == Some("status") && listed.len() < 4
    });
    shots.take(
        &mut stage,
        "terminal_completion_below",
        crate::runner::Themes::Two,
    );
    stage.keys("ctrl-u");
}
