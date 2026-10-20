//! Sessions on a real PTY: what is typed reaches `/bin/sh` and what it
//! writes is on show, in its colours, at the size the window gives.
#![cfg(unix)]

use crate::common::{Home, Running, prompt, row_starting, spec, start};
use bitbraid_terminal::{
    CellWidth, CursorShape, GridSize, Input, Key, Modifiers, Mouse, MouseButton, MouseKind,
    Palette, Rgb, SelectOp, SessionEvent, Side, Underline, ViewPoint, contrast,
};

#[test]
fn what_is_typed_is_echoed_and_answered() {
    let home = Home::new("echo");
    let running = Running::shell(&home);
    let frame = running.run("echo hel''lo; pwd", "DONE");
    assert!(row_starting(&frame, "hello").is_some(), "{}", frame.text());
    // A long folder (macOS's temporary ones are) runs on into the next row.
    assert!(
        frame
            .text()
            .replace('\n', "")
            .contains(&home.0.display().to_string()),
        "{}",
        frame.text()
    );
    // Nothing of this process's environment reached the shell.
    let frame = running.run(
        "clear; env | cut -d= -f1 | sort | tr '\\n' ' '; echo",
        "END",
    );
    // What a shell sets itself aside: PWD, and bash's SHLVL and `_`.
    let text = frame.rows[0].text();
    let names: Vec<&str> = text
        .split_whitespace()
        .filter(|name| !matches!(*name, "PWD" | "SHLVL" | "_"))
        .collect();
    assert_eq!(
        names,
        ["HISTFILE", "HOME", "LC_ALL", "PATH", "PS1", "TERM"],
        "{}",
        frame.text()
    );
    // Keys: Ctrl+U deletes what was typed, Ctrl+C ends what runs.
    running.session.input(Input::Text("echo never".into()));
    running
        .session
        .input(Input::Key(Key::Character('u'), Modifiers::CONTROL));
    let frame = running.run("echo after", "AFTER");
    assert!(!frame.text().contains("never"), "{}", frame.text());
    // It says it waits once it is the job in the foreground.
    running.type_line("sh -c 'echo WAITING; exec sleep 100'");
    running.until("the sleep", |frame| {
        frame.rows.iter().any(|row| row.text() == "WAITING")
    });
    running
        .session
        .input(Input::Key(Key::Character('c'), Modifiers::CONTROL));
    running.until("the prompt again", prompt);
}

#[test]
fn colours_and_styles_are_worked_out() {
    let home = Home::new("colours");
    let running = Running::shell(&home);
    let palette = Palette::default();
    running.run(
        "printf '\\033[31mred\\033[0m \\033[1;38;5;196mX\\033[0m \\033[48;2;1;2;3mT\\033[0m \\033[4:3mC\\033[0m \\033[9mS\\033[0m \\033[7mI\\033[0m \\033[2mD\\033[0m \\033[8mH\\033[0m \\033[4:2mW\\033[0m 中a\\n'",
        "DONE",
    );
    let frame = running.frame();
    let (_, row) = row_starting(&frame, "red X T C S I D").unwrap();
    let at = |column: usize| &row.cells[column];
    assert_eq!(at(0).foreground, palette.ansi[1]);
    assert_eq!(at(4).foreground, Rgb::new(255, 0, 0));
    assert!(at(4).style.bold);
    assert_eq!(at(6).background, Rgb::new(1, 2, 3));
    assert_eq!(at(6).foreground, palette.foreground);
    assert_eq!(at(8).style.underline, Underline::Curly);
    assert!(at(10).style.strikeout);
    // Inverse swaps the colours.
    assert_eq!(at(12).foreground, palette.background);
    assert_eq!(at(12).background, palette.foreground);
    // Dim is mixed into the background; hidden is drawn in it.
    assert_eq!(
        at(14).foreground,
        palette.foreground.mix(palette.background, 1.0 / 3.0)
    );
    assert_eq!(at(16).foreground, at(16).background);
    assert_eq!(at(18).style.underline, Underline::Double);
    // A wide character and its spacer.
    assert_eq!(at(20).character, '中');
    assert_eq!(at(20).width, CellWidth::Wide);
    assert_eq!(at(21).width, CellWidth::Spacer);
    assert_eq!(at(22).character, 'a');
    let runs = row.runs();
    assert!(
        runs.iter()
            .any(|run| run.text == "中" && run.wide && run.columns == 2)
    );
    assert!(runs.iter().any(|run| run.text == "red"));
    let spans = row.backgrounds(frame.background);
    assert!(
        spans
            .iter()
            .any(|span| span.column == 6 && span.color == Rgb::new(1, 2, 3))
    );

    // What the program changes in the palette counts before ours.
    running.run(
        "printf '\\033]4;1;rgb:12/34/56\\033\\\\\\033[31mP\\033[0m\\n'",
        "DONE2",
    );
    let frame = running.frame();
    let (_, row) = row_starting(&frame, "P").unwrap();
    // Dark blue on the dark background is lifted until it reads.
    assert_eq!(
        row.cells[0].foreground,
        contrast::readable(
            Rgb::new(0x12, 0x34, 0x56),
            frame.background,
            contrast::MINIMUM
        )
    );
    assert_ne!(row.cells[0].foreground, Rgb::new(0x12, 0x34, 0x56));
    // And a palette the window changes is used at once.
    let light = Palette::light();
    let frame = running.session.frame(&light);
    assert_eq!(frame.background, light.background);
}

#[test]
fn text_reads_on_any_background_a_program_asks_for() {
    let home = Home::new("contrast");
    let running = Running::shell(&home);
    // Black and white behind text that has no colour of its own, and
    // text in the colour of its background without being hidden.
    running.run(
        "printf '\\033[40mK\\033[0m \\033[47mW\\033[0m \\033[34;44mB\\033[0m\\n'",
        "DONE",
    );
    for palette in [Palette::dark(), Palette::light()] {
        let frame = running.session.frame(&palette);
        let (_, row) = row_starting(&frame, "K W B").unwrap();
        for column in [0, 2, 4] {
            let cell = &row.cells[column];
            assert!(
                contrast::ratio(cell.foreground, cell.background) >= contrast::MINIMUM - 0.01,
                "column {column}: {:?} on {:?}",
                cell.foreground,
                cell.background
            );
        }
    }
}

#[test]
fn the_size_is_the_one_the_window_gives() {
    let home = Home::new("size");
    let running = Running::shell(&home);
    let frame = running.run("stty size", "DONE");
    assert!(row_starting(&frame, "24 80").is_some(), "{}", frame.text());
    running.session.resize(GridSize::new(40, 10, 9, 18));
    let frame = running.run("stty size", "AGAIN");
    assert!(row_starting(&frame, "10 40").is_some(), "{}", frame.text());
    assert_eq!(frame.rows.len(), 10);
    assert_eq!(frame.columns, 40);
    assert!(frame.rows.iter().all(|row| row.cells.len() == 40));
    // Hostile sizes are bounded.
    running.session.resize(GridSize::new(0, 0, 0, 0));
    assert_eq!(running.session.size(), GridSize::new(2, 1, 1, 1));
    running
        .session
        .resize(GridSize::new(u16::MAX, u16::MAX, 1, 1));
    let frame = running.frame();
    assert_eq!((frame.columns, frame.rows.len()), (2000, 1000));
    running.session.resize(GridSize::new(80, 24, 8, 16));
    running.run("echo fine", "FINE");
}

#[test]
fn the_alternate_screen_comes_and_goes() {
    let home = Home::new("alternate");
    let running = Running::shell(&home);
    running.run("echo before", "MARK");
    running.type_line("printf '\\033[?1049hALT\\033[?1h'; read x; printf '\\033[?1049l\\033[?1l'");
    let frame = running.until("the alternate screen", |frame| {
        frame.mode.alternate_screen && frame.text().trim_start().starts_with("ALT")
    });
    assert!(!frame.text().contains("MARK"));
    assert!(frame.mode.application_cursor);
    assert_eq!(frame.history, 0);
    // The wheel sends arrow keys there, which `read` takes as text.
    let wheel = Mouse {
        kind: MouseKind::Wheel { lines: 1 },
        point: ViewPoint::default(),
        modifiers: Modifiers::NONE,
    };
    assert!(running.session.mouse(wheel));
    running.type_line("");
    let frame = running.until("the screen before", |frame| {
        !frame.mode.alternate_screen && frame.text().contains("MARK") && prompt(frame)
    });
    assert!(!frame.mode.application_cursor);
    // Outside it, the wheel is the window's.
    assert!(!running.session.mouse(wheel));
}

#[test]
fn pasting_is_bracketed_when_asked_and_cleaned() {
    let home = Home::new("paste");
    let running = Running::shell(&home);
    // The shell's line editor may ask for bracketed paste itself: wait
    // until `od` reads.
    running.type_line("stty -echo; printf '\\033[?2004hREADY\\n'; od -An -c; stty echo");
    running.until("od waiting", |frame| {
        frame.mode.bracketed_paste && frame.rows.iter().any(|row| row.text() == "READY")
    });
    running.session.paste("a\r\nb\x1b[201~c");
    running
        .session
        .input(Input::Key(Key::Enter, Modifiers::NONE));
    running
        .session
        .input(Input::Key(Key::Character('d'), Modifiers::CONTROL));
    let frame = running.until("what od read", prompt);
    let text: String = frame
        .text()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    // ESC [ 2 0 0 ~ a \n b [ 2 0 1 ~ c ESC [ 2 0 1 ~ \n: the tty turns the
    // Return into a line feed; the escape that was pasted is gone.
    assert!(
        text.contains("033 [ 2 0 0 ~ a \\n b [ 2 0 1 ~ c 033 [ 2 0 1 ~ \\n"),
        "{text}"
    );
}

#[test]
fn a_selection_gives_its_text() {
    let home = Home::new("select");
    let running = Running::shell(&home);
    let frame = running.run("clear; echo alpha beta gamma", "DONE");
    let (row, _) = row_starting(&frame, "alpha beta").unwrap();
    let row = u16::try_from(row).unwrap();
    assert!(!running.session.has_selection());
    assert_eq!(running.session.selected_text(), None);
    let at = |column| ViewPoint { row, column };
    running.session.select(SelectOp::Word(at(7), Side::Left));
    assert_eq!(running.session.selected_text().as_deref(), Some("beta"));
    running.session.select(SelectOp::Simple(at(0), Side::Left));
    running.session.select(SelectOp::Extend(at(4), Side::Right));
    assert_eq!(running.session.selected_text().as_deref(), Some("alpha"));
    let frame = running.frame();
    let cells = &frame.rows[usize::from(row)].cells;
    assert!(cells[..5].iter().all(|cell| cell.selected));
    assert!(!cells[5].selected);
    assert_eq!(cells[0].background, Palette::default().selection_background);
    running.session.select(SelectOp::Line(at(3), Side::Left));
    assert_eq!(
        running.session.selected_text().as_deref(),
        Some("alpha beta gamma\n")
    );
    running.session.select(SelectOp::All);
    let all = running.session.selected_text().unwrap();
    assert!(all.contains("alpha beta gamma\nDONE"), "{all:?}");
    running.session.select(SelectOp::Clear);
    assert_eq!(running.session.selected_text(), None);
    // Points off the screen are brought onto it.
    running.session.select(SelectOp::Simple(
        ViewPoint {
            row: u16::MAX,
            column: u16::MAX,
        },
        Side::Right,
    ));
    running.session.select(SelectOp::Extend(
        ViewPoint { row: 0, column: 0 },
        Side::Left,
    ));
    assert!(running.session.selected_text().is_some());
}

#[test]
fn history_scrolls_and_clears() {
    let home = Home::new("history");
    let running = Running::shell(&home);
    let frame = running.run("seq 1 100", "DONE");
    assert!(frame.history > 50, "{}", frame.history);
    running.session.scroll(10);
    let frame = running.frame();
    assert_eq!(frame.display_offset, 10);
    // Scrolled back past it, the cursor is not on show.
    assert!(frame.cursor.is_none());
    running.session.scroll(i32::MAX);
    assert_eq!(running.frame().display_offset, running.frame().history);
    running.session.scroll(i32::MIN);
    assert_eq!(running.frame().display_offset, 0);
    // Typing brings the view back down.
    running.session.scroll(5);
    running.session.input(Input::Text(" ".into()));
    assert_eq!(running.frame().display_offset, 0);
    running
        .session
        .input(Input::Key(Key::Backspace, Modifiers::NONE));
    // Clear: the prompt line goes to the top, everything else goes.
    running.until("the prompt", prompt);
    running.session.clear();
    let frame = running.frame();
    assert_eq!(frame.history, 0);
    assert_eq!(frame.rows[0].text(), "$");
    assert!(frame.rows[1..].iter().all(|row| row.text().is_empty()));
    assert_eq!(frame.cursor.map(|cursor| cursor.row), Some(0));
    let frame = running.run("echo still", "HERE");
    assert!(row_starting(&frame, "still").is_some(), "{}", frame.text());
}

#[test]
fn the_cursor_has_the_shape_the_program_asks_for() {
    let home = Home::new("cursor");
    let running = Running::shell(&home);
    let frame = running.frame();
    let cursor = frame.cursor.unwrap();
    assert_eq!(cursor.shape, None, "no shape was asked for");
    assert_eq!(cursor.column, 2);
    running.run("printf '\\033[6 q'", "BAR");
    assert_eq!(
        running.frame().cursor.unwrap().shape,
        Some(CursorShape::Bar)
    );
    running.run("printf '\\033[4 q'", "UNDER");
    assert_eq!(
        running.frame().cursor.unwrap().shape,
        Some(CursorShape::Underline)
    );
    running.run("printf '\\033[0 q'", "RESET");
    assert_eq!(running.frame().cursor.unwrap().shape, None);
    running.run("printf '\\033[?25l'", "HIDDEN");
    assert!(running.frame().cursor.is_none());
    running.run("printf '\\033[?25h'", "SHOWN");
    assert!(running.frame().cursor.is_some());
}

#[test]
fn titles_are_cleaned_and_cut() {
    let home = Home::new("title");
    let running = Running::shell(&home);
    assert_eq!(running.session.title(), None);
    running.run("printf '\\033]2;hello\\tworld\\033\\\\'", "ONE");
    assert_eq!(running.session.title().as_deref(), Some("helloworld"));
    running.run("printf '\\033]0;%s\\007' $(printf '%0300d' 0)", "TWO");
    assert_eq!(running.session.title().map(|t| t.len()), Some(200));
    let titles: Vec<String> = running
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|event| match event {
            SessionEvent::Title(title) => Some(title.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(titles.first().map(String::as_str), Some("helloworld"));
}

#[test]
fn the_program_is_answered_and_the_clipboard_never_read() {
    let home = Home::new("answers");
    let running = Running::shell(&home);
    // The text area in pixels, the cursor position, the background
    // colour; then a clipboard read that must get no answer, then one
    // more question so that the end can be seen.
    running.type_line(
        "stty -echo raw min 0 time 10; printf '\\033[14t\\033[6n\\033]11;?\\033\\\\\\033]52;c;?\\033\\\\\\033[5n'; dd bs=1 count=200 2>/dev/null | od -An -c > answers; stty sane; echo DONE",
    );
    running.until("the answers", |frame| {
        frame.rows.iter().any(|row| row.text() == "DONE")
    });
    let answers = std::fs::read_to_string(home.0.join("answers")).unwrap();
    let answers: String = answers.split_whitespace().collect();
    // 24 rows of 16 pixels by 80 columns of 8.
    assert!(answers.contains("033[4;384;640t"), "{answers}");
    assert!(answers.contains("033]11;rgb:"), "{answers}");
    assert!(answers.contains("033[0n"), "{answers}");
    assert!(!answers.contains("]52"), "{answers}");
    // OSC 52 with text: the clipboard is offered it.
    running.run("printf '\\033]52;c;aGVsbG8=\\007'", "SET");
    assert!(
        running
            .events
            .lock()
            .unwrap()
            .contains(&SessionEvent::Clipboard("hello".into()))
    );
}

#[test]
fn the_mouse_is_reported_when_the_program_asks() {
    let home = Home::new("mouse");
    let running = Running::shell(&home);
    let press = Mouse {
        kind: MouseKind::Press(MouseButton::Left),
        point: ViewPoint { row: 3, column: 2 },
        modifiers: Modifiers::NONE,
    };
    assert!(!running.session.mouse(press), "nobody asked");
    running.type_line("printf '\\033[?1000h\\033[?1006h'; od -An -c");
    running.until("the mode", |frame| {
        frame.mode.mouse_reporting && frame.mode.sgr_mouse
    });
    assert!(running.session.mouse(press));
    let mut shifted = press;
    shifted.modifiers = Modifiers::SHIFT;
    assert!(
        !running.session.mouse(shifted),
        "Shift keeps it for the window"
    );
    running
        .session
        .input(Input::Key(Key::Enter, Modifiers::NONE));
    running
        .session
        .input(Input::Key(Key::Character('d'), Modifiers::CONTROL));
    let frame = running.until("what od read", |frame| {
        frame.text().contains("<   0   ;   3   ;   4   M")
    });
    drop(frame);
    running.type_line("printf '\\033[?1000l\\033[?1006l'");
    running.until("the mode off", |frame| !frame.mode.mouse_reporting);
}

#[test]
fn a_shell_that_ends_says_how() {
    let home = Home::new("exit");
    let running = Running::shell(&home);
    running.type_line("exit 3");
    assert_eq!(running.wait_exit(), Some(3));
    let status = running.session.exited().unwrap();
    assert_eq!(status.code, Some(3));
    assert!(!status.success());
    // Exited is said once; typing into an ended session does nothing.
    running.type_line("echo nobody");
    running.session.resize(GridSize::new(50, 10, 8, 16));
    running.session.kill();
    let exits = running
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|event| matches!(event, SessionEvent::Exited(_)))
        .count();
    assert_eq!(exits, 1);

    // A command that ends by itself: its last output is on show.
    let quick = start(
        &spec(&home, &["-c", "echo last words; exit 0"]),
        GridSize::default(),
    );
    assert_eq!(quick.wait_exit(), Some(0));
    assert!(quick.frame().text().contains("last words"));
    assert!(quick.session.exited().unwrap().success());
}

#[test]
fn programs_that_cannot_be_started_are_errors() {
    let home = Home::new("refused");
    let mut bad = spec(&home, &[]);
    bad.program = "no-such-shell-at-all".into();
    assert!(bitbraid_terminal::Session::spawn(&bad, GridSize::default(), |_| {}).is_err());
    let mut bad = spec(&home, &[]);
    bad.cwd = home.0.join("not-there");
    assert!(bitbraid_terminal::Session::spawn(&bad, GridSize::default(), |_| {}).is_err());
}

/// A shell that is hung up on and does not end does not hang up on its
/// jobs either; it is killed a second later, and its jobs are hung up on
/// first, so none is left running. (On a loaded machine an interactive
/// shell killed before it has passed the hang-up on would otherwise leave
/// its `sleep` running.)
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[test]
fn a_shell_that_does_not_end_leaves_no_job_behind() {
    use std::time::{Duration, Instant};
    let home = Home::new("jobs");
    let stubborn = start(
        &spec(
            &home,
            &[
                "-c",
                "set -m; trap ':' HUP; sleep 1000 & echo JOB $!; while :; do sleep 0.05; done",
            ],
        ),
        GridSize::default(),
    );
    let frame = stubborn.until("the job", |frame| {
        frame.rows.iter().any(|row| row.text().starts_with("JOB "))
    });
    let job: u32 = frame
        .rows
        .iter()
        .find_map(|row| row.text().strip_prefix("JOB ")?.trim().parse().ok())
        .unwrap();
    stubborn.session.kill();
    stubborn.wait_exit();
    let started = Instant::now();
    // There while a signal of nought reaches it (macOS has no `/proc`).
    let there = |pid: u32| {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|status| status.success())
    };
    while there(job) {
        if started.elapsed() > Duration::from_secs(10) {
            // Nothing is left behind by a failure either.
            let _ = std::process::Command::new("kill")
                .arg(job.to_string())
                .status();
            panic!("the job {job} was left running");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
