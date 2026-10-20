//! What the window does, as `behaviour.rs` asks it, for the text of a
//! terminal: a `TerminalText` at the bottom of a window of its own, among
//! the application's keys, running `/bin/sh` with an environment of the
//! test's (a fixed prompt, a home of its own, `ENV` unset, the terminal
//! type from us), never the user's shell.
//!
//! What the shell writes reaches the window from a thread of its own and
//! is not counted by `Stage::settle`: every wait here is `Stage::until`.

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use bitbraid_terminal::{Palette, ShellSpec};
use bitbraid_ui::{
    keymap,
    shell::Shell,
    views::terminal::text::{TerminalLook, TerminalText, TerminalTextEvent, TerminalTextProbe},
    zoom,
};
use gpui_kit::{
    AnyView, AppContext as _, Context, Entity, EntityInputHandler as _, ExternalPaths,
    FileDropEvent, FocusHandle, Focusable as _, InputEvent as _, InteractiveElement as _,
    IntoElement, Modifiers, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement as _, Pixels,
    Render, ScrollDelta, ScrollWheelEvent, Styled as _, Subscription, TouchPhase, Window,
    component::ActiveTheme as _, div, point, px, test::TestWindowExt as _,
};

use crate::{
    repos,
    runner::Test,
    stage::{HEIGHT, Stage, Start},
};

/// The tests, in the order they are run.
pub const TESTS: &[Test] = &[
    Test {
        name: "terminal_text_shows_what_is_typed_and_what_the_shell_answers",
        run: terminal_text_shows_what_is_typed_and_what_the_shell_answers,
    },
    Test {
        name: "terminal_text_takes_the_keys_the_window_has_bindings_for",
        run: terminal_text_takes_the_keys_the_window_has_bindings_for,
    },
    Test {
        name: "terminal_text_takes_what_an_input_method_commits",
        run: terminal_text_takes_what_an_input_method_commits,
    },
    Test {
        name: "terminal_text_selects_by_dragging_and_copies_what_is_selected",
        run: terminal_text_selects_by_dragging_and_copies_what_is_selected,
    },
    Test {
        name: "terminal_text_scrolls_its_history_with_the_wheel",
        run: terminal_text_scrolls_its_history_with_the_wheel,
    },
    Test {
        name: "terminal_text_reports_the_mouse_to_a_program_that_asks",
        run: terminal_text_reports_the_mouse_to_a_program_that_asks,
    },
    // The primary selection is a thing of X11 and Wayland.
    #[cfg(target_os = "linux")]
    Test {
        name: "terminal_text_pastes_the_primary_selection_with_the_middle_button",
        run: terminal_text_pastes_the_primary_selection_with_the_middle_button,
    },
    Test {
        name: "terminal_text_types_a_dropped_path_as_the_shell_reads_it",
        run: terminal_text_types_a_dropped_path_as_the_shell_reads_it,
    },
    Test {
        name: "terminal_text_types_a_changed_file_dragged_from_the_list",
        run: terminal_text_types_a_changed_file_dragged_from_the_list,
    },
    Test {
        name: "terminal_text_tells_the_shell_its_new_size",
        run: terminal_text_tells_the_shell_its_new_size,
    },
    Test {
        name: "terminal_text_grows_with_the_zoom",
        run: terminal_text_grows_with_the_zoom,
    },
    Test {
        name: "terminal_text_asks_for_its_menu_and_says_when_the_shell_ends",
        run: terminal_text_asks_for_its_menu_and_says_when_the_shell_ends,
    },
    Test {
        name: "terminal_text_draws_whatever_a_program_writes",
        run: terminal_text_draws_whatever_a_program_writes,
    },
    Test {
        name: "terminal_text_frame_cost",
        run: terminal_text_frame_cost,
    },
];

/// The height the text has at first: the panel's first height less its
/// header.
const TEXT_HEIGHT: f32 = 245.0;

/// A window with nothing but a terminal's text at its bottom, and a place
/// above it that takes the keys when the text does not.
pub struct Host {
    pub text: Entity<TerminalText>,
    /// The application's own window above the text, when a test wants it.
    above: Option<AnyView>,
    focus: FocusHandle,
    height: Pixels,
    /// How often the palette was asked for by its key.
    pub palette_asked: usize,
    /// What the text said, oldest first.
    pub events: Vec<TerminalTextEvent>,
    _events: Subscription,
}

impl Host {
    pub fn new(home: &Path, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::with(&spec(home), window, cx)
    }

    /// The window with a terminal that runs `spec`.
    pub fn with(spec: &ShellSpec, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = cx.new(|cx| {
            TerminalText::spawn(spec, window, cx).expect("the shell starts on a terminal")
        });
        Self::showing(text, cx)
    }

    /// The window with a text the test made, from a session whose words
    /// the test hands on itself.
    pub fn showing(text: Entity<TerminalText>, cx: &mut Context<Self>) -> Self {
        let events = cx.subscribe(&text, |host, _, event: &TerminalTextEvent, cx| {
            host.events.push(event.clone());
            cx.notify();
        });
        Self {
            text,
            above: None,
            focus: cx.focus_handle(),
            height: px(TEXT_HEIGHT),
            palette_asked: 0,
            events: Vec::new(),
            _events: events,
        }
    }
}

impl Render for Host {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The terminal's colours follow the theme's lightness.
        let palette = if cx.theme().is_dark() {
            Palette::dark()
        } else {
            Palette::light()
        };
        if self.text.read(cx).look().palette != palette {
            let look = TerminalLook {
                palette,
                ..self.text.read(cx).look().clone()
            };
            self.text.update(cx, |text, cx| text.set_look(look, cx));
        }
        div()
            .id("host")
            .bg(cx.theme().background)
            .track_focus(&self.focus)
            .size_full()
            .flex()
            .flex_col()
            .justify_end()
            .on_action(cx.listener(|host, _: &keymap::TogglePalette, _, cx| {
                host.palette_asked += 1;
                cx.notify();
            }))
            .children(
                self.above
                    .clone()
                    .map(|above| div().w_full().flex_1().min_h_0().child(above)),
            )
            .child(div().w_full().h(self.height).child(self.text.clone()))
    }
}

/// `/bin/sh` in `home` with the test's environment; on Windows the `sh` of
/// Git for Windows, with its own folder for `PATH`.
pub fn spec(home: &Path) -> ShellSpec {
    let (program, path) = if cfg!(windows) {
        let sh = std::env::var_os("PATH")
            .and_then(|paths| {
                std::env::split_paths(&paths)
                    .map(|dir| dir.join("sh.exe"))
                    .find(|candidate| candidate.is_file())
            })
            .expect("the sh of Git for Windows is on PATH");
        let dir = sh.parent().unwrap().display().to_string();
        (sh.display().to_string(), dir)
    } else {
        ("/bin/sh".to_owned(), "/usr/bin:/bin".to_owned())
    };
    ShellSpec {
        program,
        args: Vec::new(),
        cwd: home.to_path_buf(),
        env: vec![
            ("PATH".into(), path),
            ("PS1".into(), "$ ".into()),
            ("HOME".into(), home.display().to_string()),
            ("TERM".into(), "xterm-256color".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            // No history file: bash writes one into the home when it is
            // hung up on, after the test, while the home is emptied.
            ("HISTFILE".into(), String::new()),
        ],
    }
}

/// A folder of the test's own inside the scratch surroundings.
pub fn home(name: &str) -> PathBuf {
    let root = crate::hermetic::inside().expect("the tests run in scratch surroundings");
    let home = root.join("work").join(format!("terminal-{name}"));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    home.canonicalize().unwrap()
}

/// The window, with the shell's first prompt on show.
pub fn open(start: Start, name: &str) -> (Stage, Entity<Host>) {
    let home = home(name);
    let (mut stage, host) = Stage::open_root(start, move |window, cx| Host::new(&home, window, cx));
    wait(&mut stage, &host, "the first prompt", |probe| {
        probe.text.lines().any(|line| line.starts_with('$'))
    });
    (stage, host)
}

pub fn shown(stage: &mut Stage, host: &Entity<Host>) -> TerminalTextProbe {
    let host = host.clone();
    stage.window(|window, cx| {
        let text = host.read(cx).text.clone();
        text.read(cx).probe(window)
    })
}

pub fn wait(
    stage: &mut Stage,
    host: &Entity<Host>,
    what: &str,
    met: impl Fn(&TerminalTextProbe) -> bool,
) -> TerminalTextProbe {
    let started = Instant::now();
    loop {
        stage.frame();
        let probe = shown(stage, host);
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
}

/// Give the text the keys, as a click on it does.
pub fn focus_text(stage: &mut Stage, host: &Entity<Host>) {
    let host = host.clone();
    stage.window(|window, cx| {
        let text = host.read(cx).text.clone();
        text.read(cx).focus_handle(cx).focus(window, cx);
    });
    stage.frame();
}

/// Type a line and Enter.
pub fn run(stage: &mut Stage, host: &Entity<Host>, line: &str) {
    focus_text(stage, host);
    stage.type_text(line);
    stage.keys("enter");
}

/// Where a cell of the text is in the window: its middle, or a little
/// inside its left or right edge.
fn spot(probe: &TerminalTextProbe, row: usize, column: usize, part: f32) -> (f32, f32) {
    let (x, y) = probe.origin.expect("the text was drawn");
    let (width, height) = probe.cell.expect("the text was drawn");
    (
        x + width * (column as f32 + part),
        y + height * (row as f32 + 0.5),
    )
}

/// The row of the screen that starts with `start`.
fn row_of(probe: &TerminalTextProbe, start: &str) -> usize {
    probe
        .text
        .lines()
        .position(|line| line.starts_with(start))
        .unwrap_or_else(|| panic!("no row starts with {start:?}:\n{}", probe.text))
}

/// Typing a line shows it, Enter runs it, and the answer shows under it;
/// when the output has been quiet for a while the text says the command
/// is done.
fn terminal_text_shows_what_is_typed_and_what_the_shell_answers() {
    let (mut stage, host) = open(Start::bare(), "echo");
    run(&mut stage, &host, "echo hel''lo");
    let probe = wait(&mut stage, &host, "the answer", |probe| {
        probe.text.lines().any(|line| line == "hello")
    });
    assert!(
        probe.text.contains("$ echo hel''lo"),
        "the line typed is on show: {}",
        probe.text
    );
    assert!(probe.focused);
    // Quiet for long enough on the scheduler's clock.
    stage.pass(Duration::from_millis(500));
    let events = stage.app(|cx| host.read(cx).events.clone());
    assert!(
        events.contains(&TerminalTextEvent::CommandFinished),
        "Enter, then quiet: {events:?}"
    );
}

/// While the text has the keys, keys the window has bindings for go to
/// the shell: Ctrl+P and Ctrl+Shift+P reach `cat -v` and not the palette,
/// while the terminal's own Ctrl+` stays the window's. Without the keys,
/// Ctrl+P opens the palette.
fn terminal_text_takes_the_keys_the_window_has_bindings_for() {
    let (mut stage, host) = open(Start::bare(), "keys");
    // Keys typed while the shell's line editor still has the terminal are
    // its: they are pressed once the shell has given it to cat.
    run(&mut stage, &host, "echo READY; cat -v");
    wait(&mut stage, &host, "cat to start", |probe| {
        probe.text.lines().any(|line| line == "READY")
    });
    // Ctrl+` is the window's (it shows and hides the terminal): cat would
    // write it as ^@.
    stage.keys("ctrl-p ctrl-` ctrl-shift-p");
    stage.type_text("x");
    stage.keys("enter");
    wait(
        &mut stage,
        &host,
        "cat writing back the control keys",
        |probe| probe.text.lines().filter(|line| *line == "^P^Px").count() == 2,
    );
    assert_eq!(
        stage.app(|cx| host.read(cx).palette_asked),
        0,
        "the window's binding did not run"
    );
    stage.keys("ctrl-d");
    // Somewhere else has the keys: the window's binding runs.
    let focus = stage.app(|cx| host.read(cx).focus.clone());
    stage.window(|window, cx| focus.focus(window, cx));
    stage.keys("ctrl-p");
    assert_eq!(stage.app(|cx| host.read(cx).palette_asked), 1);
}

/// What an input method composes is drawn at the cursor and its window
/// is placed there; what it commits goes to the shell as it is. The
/// toolkit's tests cannot drive an input method: the handler is called as
/// the platform calls it.
fn terminal_text_takes_what_an_input_method_commits() {
    let (mut stage, host) = open(Start::bare(), "ime");
    focus_text(&mut stage, &host);
    let text = stage.app(|cx| host.read(cx).text.clone());
    let composing = text.clone();
    stage.window(|window, cx| {
        composing.update(cx, |text, cx| {
            text.replace_and_mark_text_in_range(None, "かん", None, window, cx)
        })
    });
    stage.frame();
    let asked = text.clone();
    let (marked, place) = stage.window(|window, cx| {
        asked.update(cx, |text, cx| {
            (
                text.marked_text_range(window, cx),
                text.bounds_for_range(0..0, Default::default(), window, cx),
            )
        })
    });
    assert_eq!(marked, Some(0..2));
    let probe = shown(&mut stage, &host);
    let (row, column) = probe.cursor.expect("the cursor shows");
    let at = spot(&probe, usize::from(row), usize::from(column), 0.0);
    let place = place.expect("the input method's window is placed");
    assert!(
        (f32::from(place.origin.x) - at.0).abs() < 0.5,
        "{place:?} {at:?}"
    );
    let committing = text.clone();
    stage.window(|window, cx| {
        committing.update(cx, |text, cx| {
            text.replace_text_in_range(None, "漢字", window, cx)
        })
    });
    stage.frame();
    wait(
        &mut stage,
        &host,
        "the committed text at the prompt",
        |probe| probe.text.lines().any(|line| line == "$ 漢字"),
    );
    let left =
        stage.window(|window, cx| text.update(cx, |text, cx| text.marked_text_range(window, cx)));
    assert_eq!(left, None, "nothing is composed any more");
}

/// A drag over text selects it, Ctrl+Shift+C copies it, and so does Ctrl+C
/// while there is a selection (which it lets go); without one Ctrl+C goes
/// to the shell. A double click selects a word and a triple click the
/// line.
fn terminal_text_selects_by_dragging_and_copies_what_is_selected() {
    let (mut stage, host) = open(Start::bare(), "select");
    run(&mut stage, &host, "printf 'alpha beta gamma\\n'");
    let probe = wait(&mut stage, &host, "the words", |probe| {
        probe.text.lines().any(|line| line == "alpha beta gamma")
    });
    let row = row_of(&probe, "alpha beta gamma");
    // From the left half of the b to the right half of the a.
    stage.drag_at(spot(&probe, row, 6, 0.2), spot(&probe, row, 9, 0.8));
    assert_eq!(probe_selected(&mut stage, &host), Some("beta".to_owned()));
    stage.keys("ctrl-shift-c");
    let copied = stage.app(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("beta"));
    stage.drag_at(spot(&probe, row, 0, 0.2), spot(&probe, row, 4, 0.8));
    assert_eq!(probe_selected(&mut stage, &host), Some("alpha".to_owned()));
    stage.keys("ctrl-c");
    let copied = stage.app(|cx| cx.read_from_clipboard().and_then(|item| item.text()));
    assert_eq!(copied.as_deref(), Some("alpha"));
    assert_eq!(
        probe_selected(&mut stage, &host),
        None,
        "Ctrl+C let the selection go"
    );

    click_times(&mut stage, spot(&probe, row, 12, 0.5), 2);
    assert_eq!(probe_selected(&mut stage, &host), Some("gamma".to_owned()));
    click_times(&mut stage, spot(&probe, row, 2, 0.5), 3);
    assert_eq!(
        probe_selected(&mut stage, &host),
        Some("alpha beta gamma\n".to_owned()),
        "a line is selected with its end"
    );
    // Typing takes the selection away.
    focus_text(&mut stage, &host);
    stage.type_text("y");
    assert_eq!(probe_selected(&mut stage, &host), None);
    wait(&mut stage, &host, "the typed y", |probe| {
        probe.text.lines().any(|line| line.starts_with("$ y"))
    });
    // Without a selection Ctrl+C is the shell's: what was typed is gone,
    // and the shell gives a new prompt. Typing on before that prompt comes
    // races the shell's handling of the interrupt: on a loaded machine the
    // next line is echoed after the y, before the shell's new line.
    stage.keys("ctrl-c");
    wait(&mut stage, &host, "a new prompt after Ctrl+C", |probe| {
        let lines: Vec<&str> = probe
            .text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .collect();
        lines.last().is_some_and(|line| line.trim_end() == "$")
    });
    run(&mut stage, &host, "echo after");
    let probe = wait(&mut stage, &host, "the line after Ctrl+C", |probe| {
        probe.text.lines().any(|line| line == "after")
    });
    assert!(
        probe.text.lines().any(|line| line == "$ echo after"),
        "the typed y was dropped by the shell:\n{}",
        probe.text
    );
}

fn probe_selected(stage: &mut Stage, host: &Entity<Host>) -> Option<String> {
    shown(stage, host).selected
}

/// Press and let go the left button `count` times at a place: a double or
/// a triple click.
fn click_times(stage: &mut Stage, (x, y): (f32, f32), count: usize) {
    let position = point(px(x), px(y));
    stage.window(|window, cx| {
        for click_count in 1..=count {
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers: Modifiers::default(),
                    click_count,
                }
                .to_platform_input(),
                cx,
            );
        }
    });
    stage.frame();
}

/// A program that asks for the mouse hears a click as a report where it
/// was (here in SGR form, read back by `cat -v`), and the click selects
/// nothing.
fn terminal_text_reports_the_mouse_to_a_program_that_asks() {
    let (mut stage, host) = open(Start::bare(), "mouse");
    run(
        &mut stage,
        &host,
        r"printf '\033[?1000h\033[?1006h'; echo READY; cat -v",
    );
    let probe = wait(&mut stage, &host, "cat to start", |probe| {
        probe.text.lines().any(|line| line == "READY")
    });
    // Row 3, column 5 of the view: 4 and 6 counted from one.
    stage.click_at(spot(&probe, 3, 5, 0.5));
    focus_text(&mut stage, &host);
    stage.keys("enter");
    wait(&mut stage, &host, "the report", |probe| {
        probe
            .text
            .lines()
            .filter(|line| *line == "^[[<0;6;4M^[[<0;6;4m")
            .count()
            == 2
    });
    assert_eq!(shown(&mut stage, &host).selected, None);
    stage.keys("ctrl-c");
    run(&mut stage, &host, r"printf '\033[?1000l\033[?1006l'");
}

/// The middle button pastes what was selected last, here or elsewhere.
#[cfg(target_os = "linux")]
fn terminal_text_pastes_the_primary_selection_with_the_middle_button() {
    let (mut stage, host) = open(Start::bare(), "middle");
    stage.app(|cx| {
        let item = gpui_kit::ClipboardItem::new_string("echo from-elsewhere".to_owned());
        cx.write_to_primary(item.clone());
        cx.write_to_clipboard(item);
    });
    let probe = shown(&mut stage, &host);
    let place = spot(&probe, 2, 10, 0.5);
    let position = point(px(place.0), px(place.1));
    stage.window(|window, cx| {
        for event in [
            MouseDownEvent {
                button: MouseButton::Middle,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
                first_mouse: false,
            }
            .to_platform_input(),
            MouseUpEvent {
                button: MouseButton::Middle,
                position,
                modifiers: Modifiers::default(),
                click_count: 1,
            }
            .to_platform_input(),
        ] {
            window.dispatch_event(event, cx);
        }
    });
    stage.frame();
    wait(
        &mut stage,
        &host,
        "the pasted text at the prompt",
        |probe| {
            probe
                .text
                .lines()
                .any(|line| line == "$ echo from-elsewhere")
        },
    );
}

/// The wheel scrolls back into what went by and a key brings the view
/// back to the bottom.
fn terminal_text_scrolls_its_history_with_the_wheel() {
    let (mut stage, host) = open(Start::bare(), "wheel");
    run(&mut stage, &host, "seq 1 300");
    let probe = wait(&mut stage, &host, "the last number", |probe| {
        probe.text.lines().any(|line| line == "300") && probe.history > 0
    });
    assert_eq!(probe.display_offset, 0);
    let place = spot(&probe, 2, 5, 0.5);
    stage.window(|window, cx| {
        window.dispatch_event(
            ScrollWheelEvent {
                position: point(px(place.0), px(place.1)),
                delta: ScrollDelta::Lines(point(0.0, 3.0)),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            }
            .to_platform_input(),
            cx,
        );
    });
    stage.frame();
    let scrolled = shown(&mut stage, &host);
    assert_eq!(scrolled.display_offset, 3, "{}", scrolled.text);
    assert!(
        !scrolled.text.lines().any(|line| line == "300"),
        "the last lines went down out of view: {}",
        scrolled.text
    );
    focus_text(&mut stage, &host);
    stage.type_text("z");
    assert_eq!(shown(&mut stage, &host).display_offset, 0);
}

/// Files dropped from the desktop are typed at the prompt, each quoted as
/// the shell reads it, with a space after them.
fn terminal_text_types_a_dropped_path_as_the_shell_reads_it() {
    let (mut stage, host) = open(Start::bare(), "drop");
    let probe = shown(&mut stage, &host);
    let place = spot(&probe, 3, 10, 0.5);
    let position = point(px(place.0), px(place.1));
    let paths =
        ExternalPaths(vec![PathBuf::from("/tmp/two words"), PathBuf::from("/tmp/it's")].into());
    stage.window(|window, cx| {
        window.dispatch_event(
            gpui_kit::PlatformInput::FileDrop(FileDropEvent::Entered { position, paths }),
            cx,
        );
        window.dispatch_event(
            gpui_kit::PlatformInput::FileDrop(FileDropEvent::Submit { position }),
            cx,
        );
    });
    stage.frame();
    stage.type_text("|");
    let probe = wait(
        &mut stage,
        &host,
        "the quoted paths at the prompt",
        |probe| probe.text.contains("|"),
    );
    assert!(
        probe
            .text
            .lines()
            .any(|line| line == r#"$ '/tmp/two words' '/tmp/it'\''s' |"#),
        "{}",
        probe.text
    );
    assert!(probe.focused, "the text has the keys after a drop");
}

/// A row of the changes list dragged onto the text is typed at the
/// prompt by its full path, quoted, with a space after it; the window
/// does not take it for a folder to open.
fn terminal_text_types_a_changed_file_dragged_from_the_list() {
    repos::fresh();
    let repo = repos::project_with_changes();
    let top = repo.canonicalize().unwrap();
    let home = home("dragged");
    let (mut stage, host) = Stage::open_root(Start::bare(), move |window, cx| {
        let mut host = Host::new(&home, window, cx);
        let shell = cx.new(|cx| Shell::new(Some(repo), None, window, cx));
        host.above = Some(shell.into());
        host
    });
    wait(&mut stage, &host, "the first prompt", |probe| {
        probe.text.lines().any(|line| line.starts_with('$'))
    });
    stage.until("the changes are listed", |stage| {
        stage.shows("unstaged-rows")
    });
    let (x, y, _, _) = stage.place("unstaged-rows");
    let row = (x + 80., y + 16.);
    let probe = shown(&mut stage, &host);
    let target = spot(&probe, 3, 20, 0.5);
    stage.drag_at(row, target);
    stage.type_text("|");
    let probe = wait(
        &mut stage,
        &host,
        "the dragged path at the prompt",
        |probe| probe.text.contains('|'),
    );
    let typed = probe
        .text
        .lines()
        .find(|line| line.ends_with('|'))
        .expect("the prompt line")
        .to_owned();
    let path = typed
        .strip_prefix("$ ")
        .and_then(|rest| rest.strip_suffix(" |"))
        .unwrap_or_else(|| panic!("a path and a space at the prompt: {typed:?}"));
    let path = path.trim_matches('\'');
    assert!(
        Path::new(path).starts_with(&top) && Path::new(path).is_file(),
        "a changed file of the repository by its full path: {typed:?}"
    );
    let tabs = stage.app(|cx| {
        let above = host.read(cx).above.clone().expect("the window is above");
        let shell = above.downcast::<Shell>().expect("it is the window's shell");
        shell.read(cx).probe(cx).tabs.len()
    });
    assert_eq!(tabs, 1, "the window opened nothing");
    assert!(probe.focused, "the text has the keys after a drop");
}

/// The shell hears the size the text has, and again when it changes.
fn terminal_text_tells_the_shell_its_new_size() {
    let (mut stage, host) = open(Start::bare(), "size");
    let first = shown(&mut stage, &host);
    run(&mut stage, &host, "stty size");
    let said = format!("{} {}", first.rows, first.columns);
    wait(&mut stage, &host, "the first size", |probe| {
        probe.text.lines().any(|line| line == said)
    });
    assert!(first.rows >= 10 && first.columns >= 100, "{first:?}");
    host.update_in_stage(&mut stage, |host| host.height = px(HEIGHT - 100.0));
    let second = shown(&mut stage, &host);
    assert!(second.rows > first.rows, "{second:?}");
    run(&mut stage, &host, "clear; stty size");
    let said = format!("{} {}", second.rows, second.columns);
    wait(&mut stage, &host, "the second size", |probe| {
        probe.text.lines().any(|line| line == said)
    });
}

/// The window's zoom scales the text, its cells and its margins, as it
/// scales everything else; the shell is told the size that leaves.
fn terminal_text_grows_with_the_zoom() {
    let (mut stage, host) = open(Start::bare(), "zoom");
    let before = shown(&mut stage, &host);
    let (width, height) = before.cell.expect("drawn");
    let (left, top) = before.origin.expect("drawn");
    // Back to 100 % at the end, also when the test fails.
    struct Zoomed;
    impl Drop for Zoomed {
        fn drop(&mut self) {
            zoom::set(1.0);
        }
    }
    let _zoomed = Zoomed;
    zoom::set(1.5);
    host.update_in_stage(&mut stage, |_| {});
    let after = shown(&mut stage, &host);
    let (zoomed_width, zoomed_height) = after.cell.expect("drawn");
    let (zoomed_left, zoomed_top) = after.origin.expect("drawn");
    assert!(
        (zoomed_width / width - 1.5).abs() < 0.05,
        "{width} {zoomed_width}"
    );
    // Rows are whole device pixels high: 14 at 12 points, 21 at 18.
    assert!(
        (zoomed_height - height * 1.5).abs() <= 0.5,
        "{height} {zoomed_height}"
    );
    assert!(
        (zoomed_left - left - 6.0).abs() < 0.01,
        "the margin grows by half"
    );
    assert!((zoomed_top - top - 6.0).abs() < 0.01);
    assert!(after.columns < before.columns && after.rows < before.rows);
    run(&mut stage, &host, "stty size");
    let said = format!("{} {}", after.rows, after.columns);
    wait(&mut stage, &host, "the zoomed size", |probe| {
        probe.text.lines().any(|line| line == said)
    });
}

/// A right click asks for the menu where it was; `exit` ends the shell and
/// the text says so.
fn terminal_text_asks_for_its_menu_and_says_when_the_shell_ends() {
    let (mut stage, host) = open(Start::bare(), "exit");
    let probe = shown(&mut stage, &host);
    let place = spot(&probe, 1, 4, 0.5);
    stage.right_click_at(place);
    let events = stage.app(|cx| host.read(cx).events.clone());
    assert!(
        events.contains(&TerminalTextEvent::ContextMenu(point(
            px(place.0),
            px(place.1)
        ))),
        "{events:?}"
    );
    run(&mut stage, &host, "exit 3");
    stage.until("the shell to end", |stage| {
        stage
            .app(|cx| host.read(cx).events.clone())
            .contains(&TerminalTextEvent::Exited(Some(3)))
    });
}

/// Change the host and draw it again.
trait UpdateInStage {
    fn update_in_stage(&self, stage: &mut Stage, change: impl FnOnce(&mut Host));
}

impl UpdateInStage for Entity<Host> {
    fn update_in_stage(&self, stage: &mut Stage, change: impl FnOnce(&mut Host)) {
        let host = self.clone();
        stage.app(|cx| {
            host.update(cx, |host, cx| {
                change(host);
                cx.notify();
            })
        });
        stage.frame();
    }
}

/// Bytes no program should write, and some that programs do: noise, sizes
/// and places far out of range, a title of 10000 characters, the mouse
/// asked for, marks that combine, wide characters, a right-to-left
/// script, a very long line. The text draws all of it, takes the mouse
/// and the wheel over it, and the shell goes on answering.
fn terminal_text_draws_whatever_a_program_writes() {
    let (mut stage, host) = open(Start::bare(), "hostile");
    let home = home_of(&mut stage, &host);
    let mut bytes = Vec::new();
    // Noise from a fixed seed, the same on every run.
    let mut seed: u64 = 0x5eed_7e57;
    for _ in 0..20_000 {
        seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
        bytes.push((seed >> 33) as u8);
    }
    bytes.extend_from_slice(b"\x1bc");
    for nasty in [
        "\x1b[99999999;99999999H",
        "\x1b[8;9999;9999t",
        "\x1b[4;99999;99999t",
        "\x1b[?1000h\x1b[?1006h",
        "\x1b[38;2;999;999;999mcolour\x1b[0m",
        "\x1b[38;5;9999mindex\x1b[0m",
        "e\u{301}\u{302}\u{303}\u{304} a\u{20dd} \u{200d}\u{200b}",
        "漢字 😀👍🏽 \u{1f468}\u{200d}\u{1f469}\u{200d}\u{1f467}",
        "\u{5e9}\u{5dc}\u{5d5}\u{5dd} مرحبا",
        "\x1b[2J\x1b[H",
    ] {
        bytes.extend_from_slice(nasty.as_bytes());
        bytes.push(b'\n');
    }
    bytes.extend_from_slice(b"\x1b]0;");
    bytes.extend(std::iter::repeat_n(b'T', 10_000));
    bytes.extend_from_slice(b"\x07");
    bytes.extend(std::iter::repeat_n(b'w', 5_000));
    bytes.extend_from_slice(b"\n\x1b[?1000l\x1b[?1006l\x1b[0m");
    std::fs::write(home.join("hostile"), &bytes).unwrap();
    run(&mut stage, &host, "cat hostile; echo; echo DONE");
    let probe = wait(&mut stage, &host, "the end of it", |probe| {
        probe.text.lines().any(|line| line == "DONE")
    });
    let place = spot(&probe, 2, 3, 0.5);
    stage.drag_at(place, spot(&probe, 5, 40, 0.5));
    click_times(&mut stage, place, 3);
    stage.window(|window, cx| {
        window.dispatch_event(
            ScrollWheelEvent {
                position: point(px(place.0), px(place.1)),
                delta: ScrollDelta::Pixels(point(px(0.0), px(1.0e30))),
                modifiers: Modifiers::default(),
                touch_phase: TouchPhase::Moved,
            }
            .to_platform_input(),
            cx,
        );
    });
    stage.frame();
    run(&mut stage, &host, "echo alive");
    wait(&mut stage, &host, "the shell answering", |probe| {
        probe.text.lines().any(|line| line == "alive")
    });
    let title = stage
        .app(|cx| host.read(cx).events.clone())
        .into_iter()
        .find_map(|event| match event {
            TerminalTextEvent::Title(title) => Some(title),
            _ => None,
        });
    assert!(
        title.is_some_and(|title| title.chars().count() <= 200),
        "a title is cut"
    );
}

/// The folder the host's shell started in.
fn home_of(stage: &mut Stage, host: &Entity<Host>) -> PathBuf {
    let host = host.clone();
    stage.app(|cx| host.read(cx).text.read(cx).session().cwd().to_path_buf())
}

/// Lines in every colour and style a program can ask for, as the pictures
/// show them.
const COLOURS: &str = r#"for c in 0 1 2 3 4 5 6 7; do printf '\033[3%sm colour %s \033[0m' $c $c; done; echo
for c in 0 1 2 3 4 5 6 7; do printf '\033[9%sm bright %s \033[0m' $c $c; done; echo
for c in 0 1 2 3 4 5 6 7; do printf '\033[4%sm back %s \033[0m' $c $c; done; echo
for c in 0 1 2 3 4 5 6 7; do printf '\033[30;10%sm back %s \033[0m' $c $c; done; echo
printf '\033[1mbold\033[0m \033[3mitalic\033[0m \033[4munderlined\033[0m \033[4:3mcurly\033[0m \033[9mstruck\033[0m \033[2mdim\033[0m \033[7minverse\033[0m \033[38;2;255;136;0mtrue colour\033[0m \033[38;5;45mindexed\033[0m\n'
printf 'plain text in the default colours, then a selection: pick this word\n'
"#;

/// The text of a terminal in both themes: the 16 colours as text and as
/// backgrounds, bold, italic, underlined, struck, dim and inverse text, a
/// selected word, and the cursor; then without the keys, dimmed with a
/// hollow cursor.
pub fn terminal_text_pictures(shots: &mut crate::runner::Shots) {
    let (mut stage, host) = open(Start::bare().pictures(), "pictures");
    let home = home_of(&mut stage, &host);
    std::fs::write(home.join("colours.sh"), COLOURS).unwrap();
    run(&mut stage, &host, "clear; sh colours.sh");
    let probe = wait(&mut stage, &host, "the colours", |probe| {
        probe
            .text
            .lines()
            .any(|line| line.ends_with("pick this word"))
            && probe.text.trim_end().lines().last() == Some("$")
    });
    let row = row_of(&probe, "plain text");
    let column = "plain text in the default colours, then a selection: pick this ".len();
    click_times(&mut stage, spot(&probe, row, column + 1, 0.5), 2);
    assert_eq!(shown(&mut stage, &host).selected.as_deref(), Some("word"));
    focus_text(&mut stage, &host);
    assert!(shown(&mut stage, &host).focused);
    shots.take(&mut stage, "terminal_text", crate::runner::Themes::Two);
    let focus = stage.app(|cx| host.read(cx).focus.clone());
    stage.window(|window, cx| focus.focus(window, cx));
    stage.frame();
    assert!(!shown(&mut stage, &host).focused);
    shots.take(
        &mut stage,
        "terminal_text_unfocused",
        crate::runner::Themes::Two,
    );
}

/// What a frame of a full screen costs to build and paint: 200 by 60
/// cells and more of text in many colours, first standing still (the
/// toolkit's shaped lines are reused), then with every row new in every
/// frame (a program printing as fast as it can). Said, not judged: a
/// machine busy with other things takes longer; a frame that takes
/// longer than a second fails.
fn terminal_text_frame_cost() {
    let (mut stage, host) = open(Start::bare(), "cost");
    let text = stage.app(|cx| host.read(cx).text.clone());
    host.update_in_stage(&mut stage, |host| host.height = px(HEIGHT));
    stage.app(|cx| {
        text.update(cx, |text, cx| {
            let look = TerminalLook {
                font_size: 10.0,
                line_height: 1.15,
                ..text.look().clone()
            };
            text.set_look(look, cx);
        })
    });
    stage.frame();
    let size = shown(&mut stage, &host);
    assert!(
        size.columns >= 200 && size.rows >= 60,
        "a screen of 200 by 60 at least: {size:?}"
    );
    let home = home_of(&mut stage, &host);
    // Every line a different colour every four cells, and a number of its
    // own so that no two lines are the same.
    std::fs::write(
        home.join("screen.sh"),
        format!(
            r#"i=0
line() {{ c=0; while [ $((c + 4)) -le {cells} ]; do printf '\033[3%d;4%dm%s' $((c % 8)) $(((c / 4 + $1) % 8)) "$2"; c=$((c + 4)); done; printf '\033[0m\n'; }}
while [ $i -lt {rows} ]; do i=$((i + 1)); line $i "$(printf %04d $i)"; done
"#,
            cells = size.columns,
            rows = size.rows,
        ),
    )
    .unwrap();
    run(
        &mut stage,
        &host,
        "clear; sh screen.sh > screen; clear; cat screen",
    );
    let rows = usize::from(size.rows);
    wait(&mut stage, &host, "a full screen", |probe| {
        probe.text.lines().filter(|line| line.len() > 190).count() >= rows - 1
    });
    let still = time_frames(&mut stage, 60);

    // Unique rows, faster than frames are drawn: a counter in every line.
    // While it runs the window is never at rest, so nothing here waits for
    // rest: keys are pressed and frames drawn directly.
    std::fs::write(
        home.join("stream.sh"),
        r#"awk 'BEGIN { for (i = 0; i < 3000000; i++) printf "\033[3%dm%08d \033[1;4%dm%0180d\033[0m %d\n", i % 8, i, i % 7, i, i }'"#,
    )
    .unwrap();
    focus_text(&mut stage, &host);
    stage.window(|window, cx| {
        window.input("sh stream.sh", cx);
        window.press("enter", cx);
    });
    let started = Instant::now();
    loop {
        let streaming = stage.window(|window, cx| {
            window.render_frame(cx);
            let text = host.read(cx).text.read(cx).probe(window).text;
            text.lines().filter(|line| line.len() > 180).count() > rows / 2
        });
        if streaming {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(60),
            "the stream did not come"
        );
        std::thread::park_timeout(Duration::from_millis(10));
    }
    let streaming = time_frames(&mut stage, 60);
    stage.window(|window, cx| window.press("ctrl-c", cx));
    wait(&mut stage, &host, "the prompt after the stream", |probe| {
        probe.text.trim_end().lines().last() == Some("$")
    });
    println!(
        "terminal frame cost, {}x{} cells: standing still {}; every row new {}",
        size.columns, size.rows, still, streaming
    );
}

/// Draw frames, each from scratch, and say how long they took.
fn time_frames(stage: &mut Stage, count: usize) -> String {
    let mut times: Vec<Duration> = Vec::with_capacity(count);
    for _ in 0..count {
        let started = Instant::now();
        stage.window(|window, cx| window.render_frame(cx));
        times.push(started.elapsed());
        // A moment for the session's own thread to read what came.
        std::thread::park_timeout(Duration::from_millis(5));
    }
    times.sort();
    let mean = times.iter().sum::<Duration>() / u32::try_from(count).unwrap();
    let median = times[count / 2];
    let worst = times[count - 1];
    assert!(worst < Duration::from_secs(1), "a frame took {worst:?}");
    format!("mean {mean:.2?}, median {median:.2?}, worst {worst:.2?}")
}
