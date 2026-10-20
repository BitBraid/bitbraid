//! A shell on a pseudo-terminal, its screen, and what is sent to it.
//!
//! [`Session::spawn`] starts the program of a [`ShellSpec`] on a new PTY
//! with exactly the spec's environment (on Unix through `env -i`, because
//! the PTY code adds to the environment of this process and cannot take
//! anything out of it; `alacritty_terminal::tty::setup_env` is never
//! called, since it changes this process's own environment). An I/O thread
//! of `alacritty_terminal` reads what the shell writes and parses it into
//! the screen; a second, small thread waits for that one to end, makes
//! sure the shell is gone and reaped, and says so.
//!
//! The screen is behind a lock that the I/O thread holds while it parses;
//! [`Session::frame`] holds it only to copy what is on show.
//! [`SessionEvent::Output`] is sent at most once until `frame` is asked
//! for again, so a stream of output wakes the window once per frame.
//!
//! What the shell asks of the terminal is answered here: where the
//! cursor is and which modes are on (by the parser), the size of the text
//! area, the colours of the palette. A request to read the clipboard is
//! refused; a request to set it is passed on, bounded. A title is cut to
//! 200 characters with control and bidirectional formatting characters
//! taken out.
//!
//! [`Session::kill`] hangs up on the shell's process group (SIGHUP, as a
//! terminal that is closed does), and kills the group a second later if
//! the shell is still there, hanging up first on the jobs of its session
//! (on Linux and macOS), which a shell that did not end did not hang up
//! on. Dropping the last handle does the same.
//!
//! Sources: `alacritty_terminal` 0.26 (`tty`, `event_loop`, `term`,
//! `selection`); XTerm Control Sequences, "Bracketed Paste Mode" and
//! "Operating System Commands" (OSC 0, 2, 4, 10-12, 52); `proc(5)` (the
//! session in `/proc/<pid>/stat`); POSIX `kill(2)`,
//! `waitid(2)` and the General Terminal Interface (hang-up of a
//! controlling terminal); `env(1)`; macOS ps(1) (`-t`, the processes of a
//! terminal, and `tty`).

use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::Duration,
};

#[cfg(unix)]
use std::time::Instant;

use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg, State},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{self, ClipboardType, Term},
    tty,
    vte::ansi::{Color, CursorShape as ProgramShape, CursorStyle},
};

use crate::{
    GridSize, Input, Mouse, ShellKind, ShellSpec,
    frame::{self, Frame, Modes, Palette},
    line::{self, Prompt, TypedLine},
    shell::find_program,
};

/// What a session tells the window. Sent from the session's own thread,
/// sometimes while its screen is locked: hand it over to your own thread
/// and return; never call back into the [`Session`] from inside the
/// callback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    /// The screen changed; sent once until [`Session::frame`] is asked
    /// for again.
    Output,
    /// The program set the title (empty when it reset it).
    Title(String),
    /// The program rang the bell.
    Bell,
    /// The program asked for text to be put on the clipboard (OSC 52).
    Clipboard(String),
    /// The shell ended, with its exit code when it exited rather than
    /// being killed. Sent once, after the last output.
    Exited(Option<i32>),
}

/// How the shell ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ExitStatus {
    /// The exit code, when it exited.
    pub code: Option<i32>,
    /// The signal that ended it, when one did.
    pub signal: Option<i32>,
}

impl ExitStatus {
    /// Whether it exited with 0.
    pub fn success(self) -> bool {
        self.code == Some(0)
    }
}

/// Which side of a cell a point is on: where a selection starts or ends
/// within it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Side {
    /// The left half.
    #[default]
    Left,
    /// The right half.
    Right,
}

/// A cell of what is on show: row 0 is the top row of the view, whether
/// or not it is scrolled back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct ViewPoint {
    /// The row from the top of the view.
    pub row: u16,
    pub column: u16,
}

/// What a selection does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectOp {
    /// Start a selection of characters here (a press).
    Simple(ViewPoint, Side),
    /// Select the word here and go on by words (a double click).
    Word(ViewPoint, Side),
    /// Select the line here and go on by lines (a triple click).
    Line(ViewPoint, Side),
    /// Move the end of the selection here (a drag, or a click with Shift);
    /// starts one of characters when there is none.
    Extend(ViewPoint, Side),
    /// Select everything, history included.
    All,
    /// Select nothing.
    Clear,
}

/// Why a session could not be started.
#[derive(Debug)]
pub enum Error {
    /// The spec cannot be right: the program, a variable or the folder.
    Spec(String),
    /// The system refused: no PTY, no process, no thread.
    Spawn(std::io::Error),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Spec(detail) => {
                f.write_str(&say!("session-spec-error", detail = detail.as_str()))
            }
            Self::Spawn(err) => f.write_str(&say!("session-spawn-error", error = err.to_string())),
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spec(_) => None,
            Self::Spawn(err) => Some(err),
        }
    }
}

/// The longest title kept, in characters.
pub const MOST_TITLE: usize = 200;
/// The most text a program may put on the clipboard at once, in bytes.
pub const MOST_CLIPBOARD: usize = 1 << 20;
/// Lines of history kept.
pub const HISTORY: usize = 10_000;
/// The most lines one scroll moves.
const MOST_SCROLL: i32 = 1_000_000;
/// How long a shell has to end after it was hung up on before it is
/// killed.
const GRACE: Duration = Duration::from_millis(1000);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// What the session's threads and its handles share.
struct Shared {
    on_event: Box<dyn Fn(SessionEvent) + Send + Sync>,
    /// An `Output` was sent and no frame was asked for since.
    output_pending: AtomicBool,
    title: Mutex<Option<String>>,
    exit: Mutex<Option<ExitStatus>>,
    /// The I/O thread saw the shell end and reaped it.
    reaped: AtomicBool,
    /// `Exited` was sent.
    exited_sent: AtomicBool,
    /// The I/O thread and the waiter are both done.
    ended: AtomicBool,
    size: Mutex<GridSize>,
    palette: Mutex<Palette>,
    sender: OnceLock<EventLoopSender>,
}

impl Shared {
    fn send(&self, event: SessionEvent) {
        (self.on_event)(event);
    }

    fn write(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if let Some(sender) = self.sender.get() {
            let _ = sender.send(Msg::Input(Cow::Owned(bytes)));
        }
    }

    fn window_size(&self) -> WindowSize {
        let size = *lock(&self.size);
        WindowSize {
            num_lines: size.rows,
            num_cols: size.columns,
            cell_width: size.cell_width_px,
            cell_height: size.cell_height_px,
        }
    }
}

/// The listener `alacritty_terminal` reports to.
#[derive(Clone)]
struct Listener(Arc<Shared>);

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        let shared = &self.0;
        match event {
            Event::Wakeup => {
                if !shared.output_pending.swap(true, Ordering::AcqRel) {
                    shared.send(SessionEvent::Output);
                }
            }
            Event::Title(title) => {
                let title = clean_title(&title);
                *lock(&shared.title) = Some(title.clone());
                shared.send(SessionEvent::Title(title));
            }
            Event::ResetTitle => {
                *lock(&shared.title) = None;
                shared.send(SessionEvent::Title(String::new()));
            }
            Event::Bell => shared.send(SessionEvent::Bell),
            Event::ClipboardStore(ClipboardType::Clipboard, text) => {
                if text.len() <= MOST_CLIPBOARD {
                    shared.send(SessionEvent::Clipboard(text));
                }
            }
            // The primary selection is the user's, and reading the
            // clipboard is never allowed.
            Event::ClipboardStore(ClipboardType::Selection, _) | Event::ClipboardLoad(..) => {}
            Event::ColorRequest(index, format) => {
                // Answered from the palette in use. The screen is locked
                // while this runs, so a colour the program changed itself
                // is not looked up.
                let palette = lock(&shared.palette).clone();
                let color = palette_color(&palette, index);
                let rgb = alacritty_terminal::vte::ansi::Rgb {
                    r: color.r,
                    g: color.g,
                    b: color.b,
                };
                shared.write(format(rgb).into_bytes());
            }
            Event::PtyWrite(text) => shared.write(text.into_bytes()),
            Event::TextAreaSizeRequest(format) => {
                let size = shared.window_size();
                shared.write(format(size).into_bytes());
            }
            Event::ChildExit(status) => {
                *lock(&shared.exit) = Some(exit_status(status));
                shared.reaped.store(true, Ordering::Release);
            }
            Event::Exit | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
        }
    }
}

/// A colour of the palette by the index a program asks for it by.
fn palette_color(palette: &Palette, index: usize) -> crate::Rgb {
    match index {
        0..=255 => palette.indexed(u8::try_from(index).unwrap_or(0)),
        256 => palette.foreground,
        257 => palette.background,
        258 => palette.cursor,
        _ => palette.foreground,
    }
}

#[cfg(unix)]
fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    use std::os::unix::process::ExitStatusExt as _;
    ExitStatus {
        code: status.code(),
        signal: status.signal(),
    }
}

#[cfg(not(unix))]
fn exit_status(status: std::process::ExitStatus) -> ExitStatus {
    ExitStatus {
        code: status.code(),
        signal: None,
    }
}

/// A title as it is kept: no control or bidirectional formatting
/// characters, at most [`MOST_TITLE`] characters.
pub fn clean_title(title: &str) -> String {
    title
        .chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(
                    c,
                    '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
                )
        })
        .take(MOST_TITLE)
        .collect()
}

/// The size of a screen, as the grid takes it.
struct Dims {
    columns: usize,
    lines: usize,
}

impl Dimensions for Dims {
    fn total_lines(&self) -> usize {
        self.lines
    }

    fn screen_lines(&self) -> usize {
        self.lines
    }

    fn columns(&self) -> usize {
        self.columns
    }
}

impl From<GridSize> for Dims {
    fn from(size: GridSize) -> Self {
        Self {
            columns: usize::from(size.columns),
            lines: usize::from(size.rows),
        }
    }
}

type Loop = EventLoop<tty::Pty, Listener>;

/// A running shell and its screen. Cheap to clone: every clone is the same
/// session. It ends when [`Session::kill`] is called or the last clone is
/// dropped.
#[derive(Clone)]
pub struct Session {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("kind", &self.inner.kind)
            .field("pid", &self.inner.pid)
            .field("exited", &self.exited())
            .finish()
    }
}

struct Inner {
    term: Arc<FairMutex<Term<Listener>>>,
    shared: Arc<Shared>,
    kind: ShellKind,
    pid: Option<u32>,
    cwd: PathBuf,
    /// The terminal's side of the PTY, kept to ask which process group is
    /// in front.
    #[cfg(unix)]
    master: Option<std::fs::File>,
    /// The folder the shell was last found in on macOS and Windows, and
    /// when: asking takes a program of the system's (macOS) or a look at
    /// the system's list of processes (Windows), so it is asked at most
    /// every [`FOLDER_KEPT`] and anew after each Enter.
    #[cfg(any(target_os = "macos", windows))]
    folder: std::sync::Mutex<Option<(std::time::Instant, PathBuf)>>,
}

/// How long the folder the shell was found in counts on macOS and
/// Windows.
#[cfg(any(target_os = "macos", windows))]
const FOLDER_KEPT: Duration = Duration::from_secs(2);

/// The folder a process is in on Windows: its working folder, which the
/// command prompt and Git's bash keep where they are, and PowerShell where
/// the terminal starts it so ([`crate::shell::powershell_following`]).
/// Read by `sysinfo`, which asks the system for it with code this
/// workspace forbids itself.
#[cfg(windows)]
fn folder_of(pid: u32) -> Option<PathBuf> {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    // Names and parents only: what the system's list of processes has.
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    // A program that starts the shell of its name and waits for it (Git's
    // `bin\bash.exe` starts `usr\bin\bash.exe`) is not where the shell is:
    // the one it started is. A shell run inside it is followed the same way.
    let mut at = Pid::from_u32(pid);
    for _ in 0..4 {
        let name = system.process(at)?.name().to_owned();
        let mut started = system
            .processes()
            .values()
            .filter(|process| process.parent() == Some(at) && process.name() == name);
        match (started.next(), started.next()) {
            (Some(shell), None) => at = shell.pid(),
            _ => break,
        }
    }
    system.refresh_processes_specifics(
        ProcessesToUpdate::Some(&[at]),
        true,
        ProcessRefreshKind::nothing().with_cwd(UpdateKind::Always),
    );
    system
        .process(at)?
        .cwd()
        .filter(|folder| folder.is_absolute())
        .map(|folder| {
            // Without the separator Windows leaves at the end of a folder's
            // name, as a folder is named everywhere else.
            let text = folder.to_string_lossy();
            let trimmed = text.trim_end_matches('\\');
            if trimmed.ends_with(':') {
                folder.to_path_buf()
            } else {
                PathBuf::from(trimmed)
            }
        })
}

/// The folder a process is in on macOS, which has no `/proc`: as `lsof`
/// lists it (`-d cwd`, one field a line: `n` and the path).
#[cfg(target_os = "macos")]
fn folder_of(pid: u32) -> Option<PathBuf> {
    let out = std::process::Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    text.lines()
        .find_map(|line| line.strip_prefix('n'))
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

impl Drop for Inner {
    fn drop(&mut self) {
        hang_up(&self.shared, self.pid);
    }
}

/// Hang up on the shell and stop reading it; the waiter does the rest.
fn hang_up(shared: &Shared, pid: Option<u32>) {
    if !shared.ended.load(Ordering::Acquire) && !shared.reaped.load(Ordering::Acquire) {
        signal_group(pid, Signal::HangUp);
    }
    if let Some(sender) = shared.sender.get() {
        let _ = sender.send(Msg::Shutdown);
    }
}

#[derive(Debug, Clone, Copy)]
enum Signal {
    HangUp,
    #[cfg(unix)]
    Kill,
}

#[cfg(unix)]
fn signal_group(pid: Option<u32>, signal: Signal) {
    use rustix::process::{Pid, Signal as Sig, kill_process, kill_process_group};
    // Never 0 or 1: those mean this process's group and every process.
    let Some(pid) = pid
        .and_then(|pid| i32::try_from(pid).ok())
        .filter(|pid| *pid > 1)
        .and_then(Pid::from_raw)
    else {
        return;
    };
    let signal = match signal {
        Signal::HangUp => Sig::HUP,
        Signal::Kill => Sig::KILL,
    };
    // The shell leads its own session and group; programs it runs as
    // jobs have groups of their own, which a shell hung up on hangs up on.
    let _ = kill_process_group(pid, signal);
    let _ = kill_process(pid, signal);
}

#[cfg(not(unix))]
fn signal_group(_pid: Option<u32>, _signal: Signal) {}

/// The most processes of a shell's session that are hung up on.
#[cfg(any(target_os = "linux", target_os = "macos"))]
const MOST_JOBS: usize = 4096;

/// Hang up on what the shell started in groups of their own (its jobs),
/// as a shell that is hung up on does itself: every process of the session
/// it leads, but the shell, read from `/proc`. Asked while the shell is
/// still there, so its id still names its session.
#[cfg(target_os = "linux")]
fn hang_up_jobs(shell: u32) {
    use rustix::process::{Pid, Signal as Sig, kill_process};
    let Ok(shell) = i32::try_from(shell) else {
        return;
    };
    if shell <= 1 {
        return;
    }
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return;
    };
    let members = entries
        .filter_map(|entry| {
            let pid: i32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            (pid > 1 && pid != shell && session_of(pid)? == shell).then_some(pid)
        })
        .take(MOST_JOBS);
    for pid in members {
        if let Some(pid) = Pid::from_raw(pid) {
            let _ = kill_process(pid, Sig::HUP);
        }
    }
}

/// The session a process is in, from `/proc/<pid>/stat`: after the name
/// in parentheses (which may hold anything, so the last `)` counts) come
/// the state, the parent, the group and the session.
#[cfg(target_os = "linux")]
fn session_of(pid: i32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let rest = stat.get(stat.rfind(')')?.saturating_add(1)..)?;
    rest.split_whitespace().nth(3)?.parse().ok()
}

/// The same on macOS, which has no `/proc`: the processes whose terminal
/// is the shell's, as `ps` lists them, but the shell. A job the shell
/// started has the terminal of its session.
#[cfg(target_os = "macos")]
fn hang_up_jobs(shell: u32) {
    use rustix::process::{Pid, Signal as Sig, kill_process};
    let Ok(shell_pid) = i32::try_from(shell) else {
        return;
    };
    if shell_pid <= 1 {
        return;
    }
    let ps = |args: &[&str]| -> Option<String> {
        let out = std::process::Command::new("/bin/ps")
            .args(args)
            .stdin(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .output()
            .ok()?;
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let Some(tty) = ps(&["-o", "tty=", "-p", &shell.to_string()]) else {
        return;
    };
    let tty = tty.trim();
    // `??` is no terminal; anything but letters and digits is no name of one.
    if tty.is_empty() || !tty.chars().all(|c| c.is_ascii_alphanumeric()) {
        return;
    }
    let Some(listed) = ps(&["-t", tty, "-o", "pid="]) else {
        return;
    };
    let members = listed
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .filter(|pid| *pid > 1 && *pid != shell_pid)
        .take(MOST_JOBS);
    for pid in members {
        if let Some(pid) = Pid::from_raw(pid) {
            let _ = kill_process(pid, Sig::HUP);
        }
    }
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn hang_up_jobs(_shell: u32) {}

/// Whether the process group in front of the terminal is the shell's:
/// the shell leads a group of its own, and a job it starts gets another.
#[cfg(unix)]
fn foreground_is(master: Option<&std::fs::File>, pid: Option<u32>) -> bool {
    let (Some(master), Some(pid)) = (master, pid) else {
        return false;
    };
    match rustix::termios::tcgetpgrp(master) {
        Ok(group) => i64::from(group.as_raw_nonzero().get()) == i64::from(pid),
        Err(_) => false,
    }
}

/// Whether the shell has ended, without reaping it (that is the PTY's to
/// do), and how. None while it runs, or when that cannot be known.
#[cfg(unix)]
fn peek_exit(pid: u32) -> Option<Option<ExitStatus>> {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let pid = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .and_then(Pid::from_raw)?;
    match waitid(
        WaitId::Pid(pid),
        WaitIdOptions::EXITED | WaitIdOptions::NOHANG | WaitIdOptions::NOWAIT,
    ) {
        Ok(Some(status)) => Some(Some(ExitStatus {
            code: status.exit_status(),
            signal: status.terminating_signal(),
        })),
        Ok(None) => None,
        // Not a child of this process any more: gone.
        Err(_) => Some(None),
    }
}

/// What the waiter thread does: wait for the I/O thread, end the shell if
/// it is still there, reap it, and tell.
fn wait_for(
    handle: JoinHandle<(Loop, State)>,
    shared: Arc<Shared>,
    #[allow(unused_variables)] pid: Option<u32>,
    #[allow(unused_variables)] grace: Duration,
) {
    let finished = handle.join();
    let mut status = *lock(&shared.exit);
    #[cfg(unix)]
    if finished.is_ok()
        && !shared.reaped.load(Ordering::Acquire)
        && let Some(pid) = pid
    {
        // The reading stopped first (the session was ended): the shell is
        // still this process's child, so its id is still its own.
        let deadline = Instant::now() + grace;
        let mut killed = false;
        loop {
            if let Some(seen) = peek_exit(pid) {
                if seen.is_some() {
                    status = seen;
                }
                break;
            }
            if Instant::now() >= deadline {
                if killed {
                    break;
                }
                // The shell did not end, so it did not hang up on its jobs
                // either: that is done for it before it is killed.
                hang_up_jobs(pid);
                signal_group(Some(pid), Signal::Kill);
                killed = true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
    // Dropping the loop drops the PTY, which reaps the shell.
    drop(finished);
    if status.is_none() {
        status = Some(ExitStatus::default());
    }
    *lock(&shared.exit) = status;
    shared.ended.store(true, Ordering::Release);
    if !shared.exited_sent.swap(true, Ordering::AcqRel) {
        shared.send(SessionEvent::Exited(status.and_then(|status| status.code)));
    }
}

/// The program and arguments the PTY starts for a spec: on Unix `env -i`
/// with the spec's environment in front of the shell.
fn command(spec: &ShellSpec) -> Result<(String, Vec<String>), Error> {
    let bad = |detail: String| Err(Error::Spec(detail));
    if spec.program.trim().is_empty() {
        return bad(say!("session-no-program").into());
    }
    if spec.program.contains('\0') || spec.args.iter().any(|arg| arg.contains('\0')) {
        return bad(say!("session-nul").into());
    }
    for (name, value) in &spec.env {
        if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
            return bad(say!("session-bad-variable", name = format!("{name:?}")));
        }
    }
    if !spec.cwd.is_dir() {
        return bad(say!(
            "session-no-folder",
            path = spec.cwd.display().to_string()
        ));
    }
    let program = find_program(&spec.program, spec.var("PATH"))
        .ok_or_else(|| Error::Spec(say!("session-not-found", program = spec.program.as_str())))?;
    let program = program.to_string_lossy().into_owned();
    if cfg!(not(unix)) {
        // The PTY puts the program at the head of the command line as it
        // is: quoted, a path with spaces (`C:\Program Files\...`) is one
        // word. A path on Windows cannot hold a quote.
        if program.contains('"') {
            return bad(say!("session-quote-in-path", program = program.as_str()));
        }
        return Ok((format!("\"{program}\""), spec.args.clone()));
    }
    // `env` takes a word with `=` in it for a variable: such a program
    // would not be started.
    if program.contains('=') {
        return bad(say!("session-equals-in-path", program = program.as_str()));
    }
    let env = ["/usr/bin/env", "/bin/env"]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.is_file())
        .or_else(|| find_program("env", std::env::var("PATH").ok().as_deref()))
        .ok_or_else(|| Error::Spec(say!("session-no-env").into()))?;
    let mut args = vec!["-i".to_owned(), "--".to_owned()];
    // The last value of a name counts; each name is given once.
    let mut seen: Vec<&str> = Vec::new();
    for (name, value) in spec.env.iter().rev() {
        if seen.contains(&name.as_str()) {
            continue;
        }
        seen.push(name);
        args.push(format!("{name}={value}"));
    }
    args.push(program);
    args.extend(spec.args.iter().cloned());
    Ok((env.to_string_lossy().into_owned(), args))
}

impl Session {
    /// Start the spec's shell on a screen of `size`. `on_event` hears what
    /// the session has to say, from the session's own thread (see
    /// [`SessionEvent`]).
    pub fn spawn(
        spec: &ShellSpec,
        size: GridSize,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
    ) -> Result<Session, Error> {
        Self::spawn_with_grace(spec, size, on_event, GRACE)
    }

    fn spawn_with_grace(
        spec: &ShellSpec,
        size: GridSize,
        on_event: impl Fn(SessionEvent) + Send + Sync + 'static,
        grace: Duration,
    ) -> Result<Session, Error> {
        let size = size.bounded();
        let (program, args) = command(spec)?;
        #[allow(unused_mut)]
        let mut options = tty::Options {
            shell: Some(tty::Shell::new(program, args)),
            working_directory: Some(spec.cwd.clone()),
            drain_on_exit: true,
            ..tty::Options::default()
        };
        #[cfg(not(unix))]
        {
            // Added to this process's environment, not instead of it: what
            // `left_out` keeps from the shell on Unix reaches it here.
            options.env = spec.env.iter().cloned().collect();
            // Each argument quoted the way programs on Windows split their
            // command line.
            options.escape_args = true;
        }
        let window = WindowSize {
            num_lines: size.rows,
            num_cols: size.columns,
            cell_width: size.cell_width_px,
            cell_height: size.cell_height_px,
        };
        // The PTY code panics on a few things a system may hand it (a
        // user name that is not UTF-8); that is an error here.
        let pty = std::panic::catch_unwind(|| tty::new(&options, window, 0))
            .map_err(|_| Error::Spawn(std::io::Error::other(say!("session-no-pty"))))?
            .map_err(Error::Spawn)?;
        #[cfg(unix)]
        let pid = Some(pty.child().id());
        #[cfg(windows)]
        let pid = pty.child_watcher().pid().map(std::num::NonZeroU32::get);
        #[cfg(not(any(unix, windows)))]
        let pid = None;
        // Without it the shell is never known to be in front, and nothing
        // is completed: no reason not to start.
        #[cfg(unix)]
        let master = pty.file().try_clone().ok();

        let shared = Arc::new(Shared {
            on_event: Box::new(on_event),
            output_pending: AtomicBool::new(false),
            title: Mutex::new(None),
            exit: Mutex::new(None),
            reaped: AtomicBool::new(false),
            exited_sent: AtomicBool::new(false),
            ended: AtomicBool::new(false),
            size: Mutex::new(size),
            palette: Mutex::new(Palette::default()),
            sender: OnceLock::new(),
        });
        let listener = Listener(shared.clone());
        let config = term::Config {
            scrolling_history: HISTORY,
            // A shape no program can ask for: it stands for "not asked",
            // and the window draws the user's own.
            default_cursor_style: CursorStyle {
                shape: ProgramShape::HollowBlock,
                blinking: false,
            },
            kitty_keyboard: false,
            osc52: term::Osc52::OnlyCopy,
            ..term::Config::default()
        };
        let term = Arc::new(FairMutex::new(Term::new(
            config,
            &Dims::from(size),
            listener.clone(),
        )));
        let event_loop = match EventLoop::new(term.clone(), listener, pty, true, false) {
            Ok(event_loop) => event_loop,
            Err(err) => return Err(Error::Spawn(err)),
        };
        let _ = shared.sender.set(event_loop.channel());
        let handle = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| event_loop.spawn()))
            .map_err(|_| {
                hang_up(&shared, pid);
                Error::Spawn(std::io::Error::other(say!("session-no-thread")))
            })?;
        let waiter_shared = shared.clone();
        if let Err(err) = std::thread::Builder::new()
            .name("bitbraid-terminal-wait".into())
            .spawn(move || wait_for(handle, waiter_shared, pid, grace))
        {
            hang_up(&shared, pid);
            return Err(Error::Spawn(err));
        }
        Ok(Session {
            inner: Arc::new(Inner {
                term,
                shared,
                kind: ShellKind::of_program(&spec.program),
                pid,
                cwd: spec.cwd.clone(),
                #[cfg(unix)]
                master,
                #[cfg(any(target_os = "macos", windows))]
                folder: std::sync::Mutex::new(None),
            }),
        })
    }

    /// The kind of shell this is, for quoting paths.
    pub fn kind(&self) -> ShellKind {
        self.inner.kind
    }

    /// The folder it was started in.
    pub fn cwd(&self) -> &std::path::Path {
        &self.inner.cwd
    }

    /// The shell's process id.
    pub fn pid(&self) -> Option<u32> {
        self.inner.pid
    }

    /// The folder the shell is in now, where the system says it (Linux,
    /// macOS, Windows); the one it was started in elsewhere.
    pub fn current_folder(&self) -> PathBuf {
        #[cfg(target_os = "linux")]
        if let Some(pid) = self.inner.pid
            && let Ok(folder) = std::fs::read_link(format!("/proc/{pid}/cwd"))
        {
            return folder;
        }
        #[cfg(any(target_os = "macos", windows))]
        if let Some(pid) = self.inner.pid {
            let mut kept = self
                .inner
                .folder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if let Some((when, folder)) = kept.as_ref()
                && when.elapsed() < FOLDER_KEPT
            {
                return folder.clone();
            }
            if let Some(folder) = folder_of(pid) {
                *kept = Some((std::time::Instant::now(), folder.clone()));
                return folder;
            }
        }
        self.inner.cwd.clone()
    }

    /// Whether the shell itself is in front, waiting at its prompt or
    /// reading a line, rather than a program it started: on Unix the
    /// terminal's foreground process group is the shell's own. Never
    /// while the alternate screen is on show or once the shell ended.
    pub fn shell_in_front(&self) -> bool {
        if self.inner.shared.ended.load(Ordering::Acquire) || self.modes().alternate_screen {
            return false;
        }
        #[cfg(unix)]
        {
            foreground_is(self.inner.master.as_ref(), self.inner.pid)
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    /// The prompt at the cursor: what stands before it on its line. Taken
    /// when the first key of a line is typed; none when the shell is not
    /// in front or the view is scrolled back.
    pub fn prompt_here(&self) -> Option<Prompt> {
        if !self.shell_in_front() {
            return None;
        }
        let (before, _, _) = self.line_through_cursor()?;
        Some(Prompt::new(before))
    }

    /// The line typed after `prompt` where the cursor is: none when the
    /// shell is not in front, the view is scrolled back, or the cursor's
    /// line does not begin with the prompt.
    pub fn typed_line(&self, prompt: &Prompt) -> Option<TypedLine> {
        if !self.shell_in_front() {
            return None;
        }
        let (before, after, cursor) = self.line_through_cursor()?;
        line::typed_after(prompt, &before, &after, cursor)
    }

    /// The text of the line through the cursor, before and after it, and
    /// where the cursor is. The screen is locked only to copy its rows.
    fn line_through_cursor(&self) -> Option<(String, String, ViewPoint)> {
        use alacritty_terminal::term::cell::Flags;
        let (rows, cursor_row, cursor_column, past) = {
            let term = self.inner.term.lock();
            if term.grid().display_offset() != 0 || term.mode().contains(term::TermMode::ALT_SCREEN)
            {
                return None;
            }
            let columns = term.columns();
            let mut rows: Vec<line::ScreenRow> = (0..term.screen_lines())
                .map(|_| line::ScreenRow::default())
                .collect();
            for indexed in term.grid().display_iter() {
                let Some(row) = usize::try_from(indexed.point.line.0)
                    .ok()
                    .and_then(|row| rows.get_mut(row))
                else {
                    continue;
                };
                let flags = indexed.cell.flags;
                if indexed.point.column.0.saturating_add(1) == columns
                    && flags.contains(Flags::WRAPLINE)
                {
                    row.wraps = true;
                }
                if flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                let c = match indexed.cell.c {
                    c if c.is_control() => ' ',
                    c => c,
                };
                let column = u16::try_from(indexed.point.column.0).unwrap_or(u16::MAX);
                row.cells.push((column, c));
            }
            let cursor = &term.grid().cursor;
            (
                rows,
                usize::try_from(cursor.point.line.0).ok()?,
                u16::try_from(cursor.point.column.0).unwrap_or(u16::MAX),
                cursor.input_needs_wrap,
            )
        };
        let (before, after) = line::line_at(&rows, cursor_row, cursor_column, past)?;
        let point = ViewPoint {
            row: u16::try_from(cursor_row).ok()?,
            column: cursor_column,
        };
        Some((before, after, point))
    }

    /// The size of the screen.
    pub fn size(&self) -> GridSize {
        *lock(&self.inner.shared.size)
    }

    /// The modes the program set.
    pub fn modes(&self) -> Modes {
        Modes::of(*self.inner.term.lock().mode())
    }

    /// Send a key or text to the program, as the modes it set say, and
    /// bring the view back to the bottom.
    pub fn input(&self, input: Input) {
        // A command that is run may change the folder: Enter, or a line
        // that is pasted or typed whole.
        #[cfg(any(target_os = "macos", windows))]
        if match &input {
            Input::Key(crate::input::Key::Enter, _) => true,
            Input::Text(text) => text.contains(['\r', '\n']),
            _ => false,
        } {
            *self
                .inner
                .folder
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
        }
        let bytes = {
            let mut term = self.inner.term.lock();
            let modes = Modes::of(*term.mode());
            let bytes = crate::input::encode(&input, &modes);
            if bytes.is_some() && term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
            }
            bytes
        };
        if let Some(bytes) = bytes {
            self.inner.shared.write(bytes);
        }
    }

    /// Paste text: bracketed when the program asked for that, with line
    /// breaks sent as Return and the escape character, NUL and the C1
    /// controls left out, so pasted text can neither end the bracket nor
    /// send an escape sequence of its own.
    pub fn paste(&self, text: &str) {
        let bracketed = {
            let mut term = self.inner.term.lock();
            if term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
            }
            term.mode().contains(term::TermMode::BRACKETED_PASTE)
        };
        let bytes = paste_bytes(text, bracketed);
        self.inner.shared.write(bytes);
    }

    /// Send bytes as they are.
    pub fn write(&self, bytes: Vec<u8>) {
        self.inner.shared.write(bytes);
    }

    /// Change the size of the screen.
    pub fn resize(&self, size: GridSize) {
        let size = size.bounded();
        {
            let mut current = lock(&self.inner.shared.size);
            if *current == size {
                return;
            }
            *current = size;
        }
        self.inner.term.lock().resize(Dims::from(size));
        if let Some(sender) = self.inner.shared.sender.get() {
            let _ = sender.send(Msg::Resize(self.inner.shared.window_size()));
        }
    }

    /// Scroll the view by lines: back into history when positive, towards
    /// the bottom when negative.
    pub fn scroll(&self, lines: i32) {
        if lines == 0 {
            return;
        }
        // The grid adds this to where it is as an i32: keep it far from
        // the ends (more than the history never scrolls further anyway).
        let lines = lines.clamp(-MOST_SCROLL, MOST_SCROLL);
        self.inner.term.lock().scroll_display(Scroll::Delta(lines));
    }

    /// Scroll the view to the bottom.
    pub fn scroll_to_bottom(&self) {
        self.inner.term.lock().scroll_display(Scroll::Bottom);
    }

    /// Report a mouse event to the program when it asked for the mouse
    /// (or the wheel in the alternate screen, as arrow keys). False when
    /// the event is the window's to handle: selecting, scrolling history.
    pub fn mouse(&self, event: Mouse) -> bool {
        let modes = self.modes();
        match crate::mouse::encode(&event, &modes) {
            Some(bytes) => {
                self.inner.shared.write(bytes);
                true
            }
            None => false,
        }
    }

    /// Tell the program the window took or lost the keys, when it asked
    /// to be told (1004).
    pub fn focus(&self, focused: bool) {
        let report = {
            let mut term = self.inner.term.lock();
            term.is_focused = focused;
            term.mode().contains(term::TermMode::FOCUS_IN_OUT)
        };
        if report {
            let bytes = if focused { b"\x1b[I" } else { b"\x1b[O" };
            self.inner.shared.write(bytes.to_vec());
        }
    }

    /// Change the selection.
    pub fn select(&self, op: SelectOp) {
        let mut term = self.inner.term.lock();
        let at = |term: &Term<Listener>, point: ViewPoint| -> Point {
            let lines = term.screen_lines();
            let columns = term.columns();
            let row = usize::from(point.row).min(lines.saturating_sub(1));
            let column = usize::from(point.column).min(columns.saturating_sub(1));
            let offset = term.grid().display_offset();
            let line = i32::try_from(row).unwrap_or(0) - i32::try_from(offset).unwrap_or(0);
            Point::new(Line(line), Column(column))
        };
        let side = |side: Side| match side {
            Side::Left => alacritty_terminal::index::Side::Left,
            Side::Right => alacritty_terminal::index::Side::Right,
        };
        match op {
            SelectOp::Simple(point, s) => {
                let point = at(&term, point);
                term.selection = Some(Selection::new(SelectionType::Simple, point, side(s)));
            }
            SelectOp::Word(point, s) => {
                let point = at(&term, point);
                term.selection = Some(Selection::new(SelectionType::Semantic, point, side(s)));
            }
            SelectOp::Line(point, s) => {
                let point = at(&term, point);
                term.selection = Some(Selection::new(SelectionType::Lines, point, side(s)));
            }
            SelectOp::Extend(point, s) => {
                let point = at(&term, point);
                match term.selection.as_mut() {
                    Some(selection) => selection.update(point, side(s)),
                    None => {
                        term.selection =
                            Some(Selection::new(SelectionType::Simple, point, side(s)));
                    }
                }
            }
            SelectOp::All => {
                let top = Point::new(term.topmost_line(), Column(0));
                let bottom = Point::new(term.bottommost_line(), term.last_column());
                let mut selection = Selection::new(
                    SelectionType::Simple,
                    top,
                    alacritty_terminal::index::Side::Left,
                );
                selection.update(bottom, alacritty_terminal::index::Side::Right);
                term.selection = Some(selection);
            }
            SelectOp::Clear => term.selection = None,
        }
    }

    /// Whether something is selected.
    pub fn has_selection(&self) -> bool {
        let term = self.inner.term.lock();
        term.selection
            .as_ref()
            .is_some_and(|selection| !selection.is_empty())
    }

    /// The selected text, when something is selected.
    pub fn selected_text(&self) -> Option<String> {
        let term = self.inner.term.lock();
        if term.selection.as_ref().is_none_or(Selection::is_empty) {
            return None;
        }
        term.selection_to_string().filter(|text| !text.is_empty())
    }

    /// Clear the screen and the history: the line the cursor is on becomes
    /// the top line and everything else goes. The program is not told; a
    /// shell's prompt stays where it was typed in. In the alternate screen
    /// (a full-screen program) nothing is done.
    pub fn clear(&self) {
        let mut term = self.inner.term.lock();
        if term.mode().contains(term::TermMode::ALT_SCREEN) {
            return;
        }
        let lines = term.screen_lines();
        let cursor_line = term.grid().cursor.point.line.0;
        let above = usize::try_from(cursor_line)
            .unwrap_or(0)
            .min(lines.saturating_sub(1));
        let region = Line(0)..Line(i32::try_from(lines).unwrap_or(i32::MAX));
        term.selection = None;
        let grid = term.grid_mut();
        if above > 0 {
            grid.scroll_up::<Color>(&region, above);
        }
        grid.clear_history();
        grid.cursor.point.line = Line(0);
    }

    /// What is on show, with the palette's colours. The screen is locked
    /// only while it is copied; the next output is announced again.
    pub fn frame(&self, palette: &Palette) -> Frame {
        self.inner
            .shared
            .output_pending
            .store(false, Ordering::Release);
        {
            let mut kept = lock(&self.inner.shared.palette);
            if *kept != *palette {
                *kept = palette.clone();
            }
        }
        let term = self.inner.term.lock();
        frame::of(&term, palette)
    }

    /// The title the program set last, if any.
    pub fn title(&self) -> Option<String> {
        lock(&self.inner.shared.title).clone()
    }

    /// How the shell ended, once it has (and the session has stopped
    /// reading it).
    pub fn exited(&self) -> Option<ExitStatus> {
        if self.inner.shared.ended.load(Ordering::Acquire) {
            *lock(&self.inner.shared.exit)
        } else {
            None
        }
    }

    /// End the shell: hang up on it and its process group, and kill them
    /// a second later if they are still there. [`SessionEvent::Exited`]
    /// follows.
    pub fn kill(&self) {
        hang_up(&self.inner.shared, self.inner.pid);
    }
}

/// The bytes pasting `text` sends.
pub(crate) fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    let mut cleaned = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                cleaned.push('\r');
            }
            '\n' => cleaned.push('\r'),
            '\u{1b}' | '\u{0}' | '\u{80}'..='\u{9f}' => {}
            c => cleaned.push(c),
        }
    }
    let mut out = Vec::with_capacity(cleaned.len().saturating_add(12));
    if bracketed {
        out.extend_from_slice(b"\x1b[200~");
    }
    out.extend_from_slice(cleaned.as_bytes());
    if bracketed {
        out.extend_from_slice(b"\x1b[201~");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_are_cut_and_cleaned() {
        assert_eq!(clean_title("a\u{7}b\u{1b}[c\u{202e}d\n"), "ab[cd");
        assert_eq!(clean_title(&"é".repeat(500)).chars().count(), MOST_TITLE);
        assert_eq!(clean_title(""), "");
    }

    #[test]
    fn pasted_text_is_cleaned_and_bracketed() {
        assert_eq!(paste_bytes("a\r\nb\nc\rd", false), b"a\rb\rc\rd");
        assert_eq!(
            paste_bytes("x\x1b[201~rm -rf ~\n", true),
            b"\x1b[200~x[201~rm -rf ~\r\x1b[201~"
        );
        assert_eq!(paste_bytes("\u{9b}31m\u{0}", false), b"31m");
        assert_eq!(paste_bytes("", true), b"\x1b[200~\x1b[201~");
    }

    #[test]
    fn a_spec_that_cannot_be_right_is_refused() {
        let dir = std::env::temp_dir();
        let good = ShellSpec {
            program: "/bin/sh".into(),
            args: Vec::new(),
            cwd: dir.clone(),
            env: vec![("PATH".into(), "/usr/bin:/bin".into())],
        };
        if cfg!(unix) {
            let (program, args) = command(&good).unwrap();
            assert!(program.ends_with("/env"), "{program}");
            assert_eq!(args, ["-i", "--", "PATH=/usr/bin:/bin", "/bin/sh"]);
        }
        for bad in [
            ShellSpec {
                program: String::new(),
                ..good.clone()
            },
            ShellSpec {
                program: "/definitely/not/there".into(),
                ..good.clone()
            },
            ShellSpec {
                program: "no-such-shell-anywhere".into(),
                ..good.clone()
            },
            ShellSpec {
                cwd: dir.join("definitely-not-there-either"),
                ..good.clone()
            },
            ShellSpec {
                args: vec!["a\0b".into()],
                ..good.clone()
            },
            ShellSpec {
                env: vec![("A=B".into(), "1".into())],
                ..good.clone()
            },
            ShellSpec {
                env: vec![(String::new(), "1".into())],
                ..good.clone()
            },
            ShellSpec {
                env: vec![("A".into(), "1\u{0}2".into())],
                ..good.clone()
            },
        ] {
            assert!(command(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_name_given_twice_is_given_once_with_its_last_value() {
        if cfg!(not(unix)) {
            return;
        }
        let spec = ShellSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "true".into()],
            cwd: std::env::temp_dir(),
            env: vec![
                ("A".into(), "1".into()),
                ("B".into(), "x y".into()),
                ("A".into(), "2".into()),
            ],
        };
        let (_, args) = command(&spec).unwrap();
        assert_eq!(args, ["-i", "--", "A=2", "B=x y", "/bin/sh", "-c", "true"]);
    }
}
