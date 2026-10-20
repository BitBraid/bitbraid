//! What the tests of sessions share: `/bin/sh` with an environment of the
//! test's own (a fixed prompt, a home of its own, `ENV` unset, nothing of
//! the user's shell or its configuration), and waiting for the screen.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    dead_code
)]

use std::{
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

use bitbraid_terminal::{
    Frame, GridSize, Input, Key, Modifiers, Palette, Session, SessionEvent, ShellSpec,
};

pub const WAIT: Duration = Duration::from_secs(10);

/// A folder of the test's own, removed at the end.
pub struct Home(pub PathBuf);

impl Home {
    pub fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("bitbraid-terminal-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        // Without `\\?\` on Windows: the command prompt takes no such folder.
        Self(bitbraid_core::paths::canonical(&dir).unwrap())
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// `/bin/sh` with these arguments, in the home, with the test's
/// environment.
pub fn spec(home: &Home, args: &[&str]) -> ShellSpec {
    ShellSpec {
        program: "/bin/sh".into(),
        args: args.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: home.0.clone(),
        env: vec![
            ("PATH".into(), "/usr/bin:/bin".into()),
            ("PS1".into(), "$ ".into()),
            ("HOME".into(), home.0.display().to_string()),
            ("TERM".into(), "xterm-256color".into()),
            ("LC_ALL".into(), "C.UTF-8".into()),
            // No history file: bash writes one into the home when it is
            // hung up on, after the test, while the home is emptied.
            ("HISTFILE".into(), String::new()),
        ],
    }
}

/// A session and what it said.
pub struct Running {
    pub session: Session,
    pub events: Arc<Mutex<Vec<SessionEvent>>>,
    wake: mpsc::Receiver<()>,
}

pub fn start(spec: &ShellSpec, size: GridSize) -> Running {
    let events = Arc::new(Mutex::new(Vec::new()));
    let (tx, wake) = mpsc::channel();
    let kept = events.clone();
    let tx = Mutex::new(tx);
    let session = Session::spawn(spec, size, move |event| {
        // Output comes in streams; what else was said is kept.
        if event != SessionEvent::Output {
            kept.lock().unwrap().push(event);
        }
        let _ = tx.lock().unwrap().send(());
    })
    .unwrap();
    Running {
        session,
        events,
        wake,
    }
}

impl Running {
    /// An interactive `/bin/sh` at its prompt, 80 by 24.
    pub fn shell(home: &Home) -> Self {
        Self::shell_sized(home, GridSize::new(80, 24, 8, 16))
    }

    pub fn shell_sized(home: &Home, size: GridSize) -> Self {
        let running = start(&spec(home, &[]), size);
        running.until("the prompt", prompt);
        running
    }

    /// An interactive bash at its prompt, 80 by 24, for what needs a line
    /// editor or a shell that hangs up on its jobs: `/bin/sh` is dash on
    /// Debian and Ubuntu, which has neither. Its own and the system's
    /// startup files are not read, so the prompt is the test's.
    pub fn bash(home: &Home) -> Self {
        let mut bash = spec(home, &["--norc"]);
        bash.program = "/bin/bash".into();
        let running = start(&bash, GridSize::new(80, 24, 8, 16));
        running.until("the prompt", prompt);
        running
    }

    pub fn frame(&self) -> Frame {
        self.session.frame(&Palette::default())
    }

    /// Wait until what is on show satisfies `test`.
    pub fn until(&self, what: &str, test: impl Fn(&Frame) -> bool) -> Frame {
        let started = Instant::now();
        loop {
            let frame = self.frame();
            if test(&frame) {
                return frame;
            }
            if started.elapsed() > WAIT {
                panic!(
                    "never saw {what}; modes {:?}, cursor {:?}; on show:\n{}",
                    frame.mode,
                    frame.cursor,
                    frame.text()
                );
            }
            let _ = self.wake.recv_timeout(Duration::from_millis(50));
        }
    }

    /// Type a line and press Return.
    pub fn type_line(&self, line: &str) {
        self.session.input(Input::Text(line.to_owned()));
        self.session.input(Input::Key(Key::Enter, Modifiers::NONE));
    }

    /// Type a line, press Return and wait for the prompt after `marker`
    /// was printed on a line of its own.
    pub fn run(&self, line: &str, marker: &str) -> Frame {
        self.type_line(&format!("{line}; echo {marker}"));
        self.until(marker, |frame| {
            frame.rows.iter().any(|row| row.text() == marker) && prompt(frame)
        })
    }

    /// How the shell ended, once `Exited` was said.
    pub fn exited(&self) -> Option<Option<i32>> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .find_map(|event| match event {
                SessionEvent::Exited(code) => Some(*code),
                _ => None,
            })
    }

    pub fn wait_exit(&self) -> Option<i32> {
        let started = Instant::now();
        loop {
            if let Some(code) = self.exited() {
                return code;
            }
            if started.elapsed() > WAIT {
                panic!("the shell never ended");
            }
            let _ = self.wake.recv_timeout(Duration::from_millis(50));
        }
    }
}

/// Whether the last line with text on show is a prompt and nothing else.
pub fn prompt(frame: &Frame) -> bool {
    frame
        .rows
        .iter()
        .rev()
        .map(|row| row.text())
        .find(|text| !text.is_empty())
        .is_some_and(|text| text == "$")
}

/// The row whose text starts with `start`.
pub fn row_starting<'a>(
    frame: &'a Frame,
    start: &str,
) -> Option<(usize, &'a bitbraid_terminal::Row)> {
    frame
        .rows
        .iter()
        .enumerate()
        .find(|(_, row)| row.text().starts_with(start))
}
