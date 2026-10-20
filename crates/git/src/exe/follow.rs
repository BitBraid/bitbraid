//! Running git for work that other programs do for it, following it while
//! it runs and ending all of it on request.
//!
//! `git lfs fetch` is git starting git-lfs, which starts a transfer program
//! for each connection. Killing git ends none of the others: they go on
//! until they are done, holding the pipes that are read here. So such a
//! command is started as the leader of a process group of its own, and
//! what is stopped is the group: first asked to end, which lets each
//! program put away what it was writing, and killed when it has not ended
//! a moment later.
//!
//! What the command says comes from three places and is passed on as it
//! arrives, on the thread that asked: its standard output, its standard
//! error, and a file it is told to append its report to. Nothing here
//! waits on any one of them, so a command that is silent on all three is
//! still stopped at once.
//!
//! A fetch, a pull and a push are run the same way (`stream` in the parent
//! module), since a push starts whatever the `pre-push` hook starts, an
//! upload by git-lfs for one. A program in a group of its own cannot ask on
//! the terminal, and a pull may have to sign; but nothing the application
//! runs asks there: git itself is told not to (`GIT_TERMINAL_PROMPT=0`),
//! ssh and `ssh-keygen -Y sign` are told to use the askpass program
//! (`SSH_ASKPASS_REQUIRE=force`), and gpg asks through its agent's
//! pinentry, which is the agent's child and in no group of ours. What a
//! group of its own does change is a program that opens the terminal by
//! itself (a hook that reads `/dev/tty`) while the application was started
//! from one: rather than read that terminal, it is stopped by it
//! (`SIGTTIN`) until the command is cancelled, which ends it like
//! everything else. Started from the desktop there is no terminal, and
//! nothing changes.
//!
//! The command also ends when the application does, and not only when it
//! is asked to: a transfer must not go on for an application that was
//! killed or has crashed. Nothing tells a child that its parent died, so
//! the command is given a leash: git is started by a few lines of `sh`
//! that lead the group and wait on a pipe only the application holds the
//! other end of. When that end closes without git having ended, the
//! application is gone, and the group is ended as a request to stop would.
//!
//! Windows has no process groups to signal and no `sh` to lead one: what
//! is stopped there is the tree of processes git started, by taskkill, and
//! at once, since a console program cannot be asked to end. There is no
//! leash there: a command outlives an application that crashed.
//!
//! Sources: setpgid(2) and kill(2) (a process group, and a signal to every
//! process in it); signal(7) (`SIGTERM` can be handled, `SIGKILL` cannot);
//! credentials(7) and termios(3) (a background process group and the
//! terminal, `SIGTTIN`); pipe(7) (the read end of a pipe sees its end when
//! the last write end closes); sh(1p) (`read`, `wait`, `trap`, `kill` with
//! process 0, and that a command started with `&` reads `/dev/null`); the
//! Rust standard library's
//! `std::os::unix::process::CommandExt::process_group` and
//! `std::process::Child::wait` (which closes the child's standard input
//! first); rustix's documentation of `process::kill_process_group`;
//! Microsoft Learn, "taskkill" (`/T`, `/F`); ssh(1)
//! (`SSH_ASKPASS_REQUIRE`), gpg-agent(1) (pinentry).

use std::{
    ffi::OsStr,
    io::{Read, Seek as _, SeekFrom},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

use super::{Output, base_command, cancelled, render, unattended};
use crate::{Error, Repository, Result, activity};

/// How often the report and the request to stop are looked at.
const TICK: Duration = Duration::from_millis(50);
/// How long what was asked to end is given before it is killed.
const GRACE: Duration = Duration::from_secs(2);
/// How long after the kill its pipes are still read: a program that left
/// the group and kept them open is not waited for.
const LINGER: Duration = Duration::from_secs(1);
/// More than this of what was written to one pipe is not kept, and this
/// much of its end is kept then.
const KEPT: usize = 256 * 1024;
const KEPT_OF_MORE: usize = 64 * 1024;
/// A longer line is cut.
const LONGEST_LINE: usize = 8 * 1024;
/// As much of the report is read at a time.
const READ_AT_ONCE: u64 = 256 * 1024;
/// How many pieces of output may wait to be looked at; the pipe fills
/// behind them, and the command waits.
const WAITING: usize = 256;

/// Something a followed command said.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Said<'a> {
    /// A line on standard output; one that it rewrites in place comes
    /// each time it is rewritten.
    Out(&'a str),
    /// A line on standard error.
    Err(&'a str),
    /// A line appended to the file it reports to.
    Reported(&'a str),
}

/// Run git in the repository for work that programs started by git do,
/// with `env` added to its environment.
///
/// What it says goes to `on` as it arrives, also what is appended to the
/// file at `report` while it runs, which is for an environment variable
/// in `env` to name. Setting `cancel` ends git and every program it
/// started; the result is then an error saying it was cancelled.
///
/// In what is returned, a line that was rewritten in place is there as it
/// stood last.
pub fn run_followed<I, S>(
    repo: &Repository,
    args: I,
    env: &[(&str, &OsStr)],
    report: Option<&Path>,
    cancel: &Arc<AtomicBool>,
    on: &mut dyn FnMut(Said<'_>),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    for (key, value) in env {
        cmd.env(key, value);
    }
    super::push_args(&mut cmd, args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unattended(&mut cmd);
    super::carry_environment(&mut cmd);
    let rendered = render(&cmd);
    let failed = |context: &'static str, err: std::io::Error| Error::Git {
        context,
        source: Box::new(err),
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    let record = activity::begin(&cmd, &rendered);
    let (mut child, _leash) = spawn_leashed(cmd).map_err(|err| {
        record.not_started(&err);
        failed(say!("exe-spawning"), err)
    })?;
    let (to, from) = mpsc::sync_channel(WAITING);
    pump(
        "bitbraid-git-stdout",
        child.stdout.take(),
        Pipe::Out,
        to.clone(),
    );
    pump("bitbraid-git-stderr", child.stderr.take(), Pipe::Err, to);

    let mut out = Lines::default();
    let mut err = Lines::default();
    let mut tail = Tail::of(report);
    let mut stop = Stop::Not;
    let mut looked = Instant::now();
    loop {
        match from.recv_timeout(TICK) {
            Ok((Pipe::Out, bytes)) => out.feed(&bytes, &mut |line| on(Said::Out(line))),
            Ok((Pipe::Err, bytes)) => err.feed(&bytes, &mut |line| on(Said::Err(line))),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            // Both pipes are closed: nobody is left who could write.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
        if looked.elapsed() >= TICK {
            looked = Instant::now();
            tail.poll(&mut |line| on(Said::Reported(line)));
            if cancel.load(Ordering::SeqCst) && !stop.step(&mut child) {
                break;
            }
        }
    }
    // Git itself may outlive its pipes.
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {}
            Err(err) => return Err(failed(say!("exe-waiting"), err)),
        }
        if cancel.load(Ordering::SeqCst) {
            stop.step(&mut child);
        }
        std::thread::sleep(TICK);
    };
    tail.poll(&mut |line| on(Said::Reported(line)));
    let stdout = out.finish(&mut |line| on(Said::Out(line)));
    let stderr = err.finish(&mut |line| on(Said::Err(line)));
    if cancel.load(Ordering::SeqCst) {
        record.cancelled(&stdout, &stderr);
        return Err(cancelled());
    }
    record.exited(status, &stdout, &stderr);
    if !status.success() {
        return Err(Error::Command {
            command: rendered,
            status: status.to_string(),
            stderr: stderr.trim().to_owned(),
        });
    }
    Ok(Output { stdout, stderr })
}

/// Have the command lead a process group of its own, which everything it
/// starts is in too unless it leaves.
#[cfg(unix)]
fn own_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    cmd.process_group(0);
}

#[cfg(not(unix))]
fn own_group(_: &mut Command) {}

/// What holds a command to the life of the application: the end of a pipe
/// the command's group waits on. It is closed when this is dropped, and by
/// the system when the application ends in any way at all; either is the
/// end of the group, so it is kept until the command has been waited for.
#[derive(Debug)]
pub(super) struct Leash(#[allow(dead_code)] Option<std::process::ChildStdin>);

/// The lines of `sh` that lead the group. Git is its first job, reading
/// nothing; a second one waits for the end of the leash (descriptor 3,
/// which is what the shell was given to read) and then ends the group the
/// way a request to stop does: asked first, killed two seconds later if it
/// is still there. That one holds none of the pipes that are read here,
/// and is ended with git. What git ended with is what the shell ends with.
#[cfg(unix)]
const LEADER: &str = r#"exec 3<&0
"$@" </dev/null 3<&- &
job=$!
( trap '' TERM; read -r end <&3; kill -TERM 0; sleep 2; kill -KILL 0 ) >/dev/null 2>&1 &
watch=$!
exec 3<&- </dev/null
wait "$job"
code=$?
kill -KILL "$watch" 2>/dev/null
exit "$code"
"#;

/// Whether the program is there to be started: by its full path, or by
/// its name somewhere on the search path. A program that is not is
/// started without the shell, so that what is reported is that it could
/// not be started, and not what a shell says about it.
#[cfg(unix)]
fn can_run(program: &OsStr) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    let runs = |file: &Path| {
        file.metadata()
            .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
    };
    let path = Path::new(program);
    if path.is_absolute() {
        return runs(path);
    }
    // A bare name is looked up; a relative path is started without the
    // shell.
    path.components().count() == 1
        && bitbraid_core::process::var_os("PATH")
            .is_some_and(|dirs| std::env::split_paths(&dirs).any(|dir| runs(&dir.join(path))))
}

/// Where what a command on a leash writes goes.
#[derive(Debug, Clone, Copy)]
pub(super) enum Outputs<'a> {
    /// Its standard output and error are pipes of the child, to be read.
    Piped,
    /// Its standard output goes nowhere and its standard error to this
    /// file: for a command that starts a program which may leave the
    /// group and keep what it was given open for as long as it runs (a
    /// merge tool that hands its window to an instance of its own), which
    /// nothing then waits on.
    Log(&'a std::fs::File),
}

impl Outputs<'_> {
    /// Set them on `cmd`; standard input reads the leash where there is
    /// one, else nothing.
    fn set(self, cmd: &mut Command, leashed: bool) -> std::io::Result<()> {
        cmd.stdin(if leashed {
            Stdio::piped()
        } else {
            Stdio::null()
        });
        match self {
            Outputs::Piped => {
                cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
            }
            Outputs::Log(file) => {
                cmd.stdout(Stdio::null()).stderr(file.try_clone()?);
            }
        }
        Ok(())
    }
}

/// Start `cmd` as the first job of a group of its own, on a leash. The
/// output pipes are the child's as `cmd` has them set.
///
/// Where there is no `sh` to lead the group, the command leads it itself
/// and has no leash: it is ended when asked, not when the application
/// dies. Without process groups (not Unix) it is started as it is.
pub(super) fn spawn_leashed(cmd: Command) -> std::io::Result<(Child, Leash)> {
    spawn_leashed_to(cmd, Outputs::Piped)
}

/// [`spawn_leashed`], with what the command writes going where `outputs`
/// says.
pub(super) fn spawn_leashed_to(
    mut cmd: Command,
    outputs: Outputs<'_>,
) -> std::io::Result<(Child, Leash)> {
    #[cfg(unix)]
    if can_run(cmd.get_program()) {
        let mut led = Command::new("sh");
        led.arg("-c")
            .arg(LEADER)
            .arg("bitbraid-leash")
            .arg(cmd.get_program())
            .args(cmd.get_args());
        if let Some(dir) = cmd.get_current_dir() {
            led.current_dir(dir);
        }
        for (key, value) in cmd.get_envs() {
            match value {
                Some(value) => led.env(key, value),
                None => led.env_remove(key),
            };
        }
        outputs.set(&mut led, true)?;
        own_group(&mut led);
        match led.spawn() {
            Ok(mut child) => {
                let leash = Leash(child.stdin.take());
                return Ok((child, leash));
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
    }
    outputs.set(&mut cmd, false)?;
    own_group(&mut cmd);
    let child = cmd.spawn()?;
    Ok((child, Leash(None)))
}

/// Signal the group the child leads: to end, or with what cannot be
/// refused. The child has not been waited for, so its number still names
/// it, and its group, and nothing else.
#[cfg(unix)]
fn end_group(child: &mut Child, kill: bool) {
    use rustix::process::{Pid, Signal, kill_process_group};
    let signal = if kill { Signal::KILL } else { Signal::TERM };
    let _ = kill_process_group(Pid::from_child(child), signal);
    if kill {
        let _ = child.kill();
    }
}

/// On Windows the tree of processes git started (git-lfs, ssh, a hook's
/// shell) is ended with taskkill, and git with it. A console program there
/// cannot be asked to end, so it is ended outright the first time. Git has
/// not been waited for, and its number names nobody else while it has not
/// (the system gives no number again while a handle to it is open).
#[cfg(windows)]
fn end_group(child: &mut Child, _: bool) {
    let taskkill = std::env::var_os("SystemRoot")
        .map(|root| Path::new(&root).join("System32").join("taskkill.exe"))
        .filter(|path| path.is_file())
        .unwrap_or_else(|| "taskkill.exe".into());
    let mut command = Command::new(taskkill);
    bitbraid_core::process::no_window(&mut command);
    let _ = command
        .args(["/T", "/F", "/PID", &child.id().to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
}

/// Without process groups only git itself is ended.
#[cfg(not(any(unix, windows)))]
fn end_group(child: &mut Child, _: bool) {
    let _ = child.kill();
}

/// How far stopping has come.
#[derive(Debug, Clone, Copy)]
pub(super) enum Stop {
    /// Nothing was done yet.
    Not,
    /// The group was asked to end.
    Asked(Instant),
    /// The group was killed.
    Killed(Instant),
}

impl Stop {
    /// Do what is due now. False when there is nothing left to wait for:
    /// the group was killed, and whoever still holds its pipes is not in
    /// it.
    pub(super) fn step(&mut self, child: &mut Child) -> bool {
        match *self {
            Stop::Not => {
                end_group(child, false);
                *self = Stop::Asked(Instant::now());
                true
            }
            Stop::Asked(since) => {
                if since.elapsed() >= GRACE {
                    end_group(child, true);
                    *self = Stop::Killed(Instant::now());
                }
                true
            }
            Stop::Killed(since) => since.elapsed() < LINGER,
        }
    }
}

/// Which pipe a piece came from.
#[derive(Debug, Clone, Copy)]
enum Pipe {
    Out,
    Err,
}

/// Read a pipe on a thread of its own and hand on what comes, until it is
/// closed or nobody listens.
fn pump<R>(name: &str, pipe: Option<R>, which: Pipe, to: mpsc::SyncSender<(Pipe, Vec<u8>)>)
where
    R: Read + Send + 'static,
{
    let Some(mut pipe) = pipe else {
        return;
    };
    // Without the thread the pipe is closed, and the command is told so
    // when it writes.
    let _ = std::thread::Builder::new()
        .name(name.to_owned())
        .spawn(move || {
            let mut buffer = [0u8; 4096];
            loop {
                let read = match pipe.read(&mut buffer) {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                let bytes: Vec<u8> = buffer.iter().take(read).copied().collect();
                if to.send((which, bytes)).is_err() {
                    break;
                }
            }
        });
}

/// What arrives in pieces, as lines. A carriage return ends a line that
/// the next one replaces, which is how a meter is rewritten in place:
/// each state is passed on, and only the one it stood at last is kept.
#[derive(Debug, Default)]
struct Lines {
    line: Vec<u8>,
    /// The last line that a carriage return ended.
    standing: Option<Vec<u8>>,
    kept: String,
}

impl Lines {
    fn feed(&mut self, bytes: &[u8], on_line: &mut dyn FnMut(&str)) {
        for byte in bytes {
            match *byte {
                b'\r' => {
                    if !self.line.is_empty() {
                        on_line(String::from_utf8_lossy(&self.line).trim_end());
                        self.standing = Some(std::mem::take(&mut self.line));
                    }
                }
                b'\n' => {
                    let line = std::mem::take(&mut self.line);
                    if line.is_empty() {
                        // The end of the line that stood there.
                        if let Some(standing) = self.standing.take() {
                            self.keep(&standing);
                        }
                    } else {
                        on_line(String::from_utf8_lossy(&line).trim_end());
                        self.standing = None;
                        self.keep(&line);
                    }
                }
                byte => {
                    if self.line.len() < LONGEST_LINE {
                        self.line.push(byte);
                    }
                }
            }
        }
    }

    fn keep(&mut self, line: &[u8]) {
        self.kept.push_str(String::from_utf8_lossy(line).trim_end());
        self.kept.push('\n');
        // A stream of lines must not grow without bound.
        if self.kept.len() > KEPT {
            let mut from = self.kept.len().saturating_sub(KEPT_OF_MORE);
            while !self.kept.is_char_boundary(from) {
                from = from.saturating_add(1);
            }
            self.kept.drain(..from);
        }
    }

    /// Everything that was kept, with what had no end yet.
    fn finish(mut self, on_line: &mut dyn FnMut(&str)) -> String {
        if let Some(standing) = self.standing.take() {
            self.keep(&standing);
        }
        if !self.line.is_empty() {
            let line = std::mem::take(&mut self.line);
            on_line(String::from_utf8_lossy(&line).trim_end());
            self.keep(&line);
        }
        self.kept
    }
}

/// The lines appended to a file while someone else writes it.
#[derive(Debug)]
struct Tail<'a> {
    file: Option<&'a Path>,
    /// How much of it was read.
    read: u64,
    /// A line whose end has not come yet.
    line: Vec<u8>,
}

impl<'a> Tail<'a> {
    fn of(file: Option<&'a Path>) -> Self {
        Self {
            file,
            read: 0,
            line: Vec::new(),
        }
    }

    /// Pass on the lines that are new since the last look. A file that is
    /// not there yet, or cannot be read, has none.
    fn poll(&mut self, on_line: &mut dyn FnMut(&str)) {
        let Some(file) = self.file else {
            return;
        };
        // What is at that name is read when it is a file and never
        // through a link.
        if !file.symlink_metadata().is_ok_and(|meta| meta.is_file()) {
            return;
        }
        let Ok(mut open) = std::fs::File::open(file) else {
            return;
        };
        let Ok(length) = open.metadata().map(|meta| meta.len()) else {
            return;
        };
        if length < self.read {
            // Written anew: from its start, then.
            self.read = 0;
            self.line.clear();
        }
        if length == self.read || open.seek(SeekFrom::Start(self.read)).is_err() {
            return;
        }
        let mut bytes = Vec::new();
        let _ = open.take(READ_AT_ONCE).read_to_end(&mut bytes);
        self.read = self
            .read
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        for byte in bytes {
            if byte == b'\n' {
                if !self.line.is_empty() {
                    on_line(String::from_utf8_lossy(&self.line).trim_end());
                    self.line.clear();
                }
            } else if self.line.len() < LONGEST_LINE {
                self.line.push(byte);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use super::*;
    use std::io::Write as _;

    fn fed(pieces: &[&str]) -> (Vec<String>, String) {
        let mut lines = Lines::default();
        let mut seen = Vec::new();
        for piece in pieces {
            lines.feed(piece.as_bytes(), &mut |line| seen.push(line.to_owned()));
        }
        let kept = lines.finish(&mut |line| seen.push(line.to_owned()));
        (seen, kept)
    }

    #[test]
    fn a_line_rewritten_in_place_is_seen_each_time_and_kept_once() {
        let (seen, kept) = fed(&[
            "Uploading LFS objects:   0% (0/2), 0 B | 0 B/s      \rUploading LFS ob",
            "jects:  50% (1/2), 11 B | 0 B/s      \r",
            "Uploading LFS objects: 100% (2/2), 26 B | 0 B/s, done.\nPushed.\n",
        ]);
        assert_eq!(
            seen,
            [
                "Uploading LFS objects:   0% (0/2), 0 B | 0 B/s",
                "Uploading LFS objects:  50% (1/2), 11 B | 0 B/s",
                "Uploading LFS objects: 100% (2/2), 26 B | 0 B/s, done.",
                "Pushed.",
            ]
        );
        assert_eq!(
            kept,
            "Uploading LFS objects: 100% (2/2), 26 B | 0 B/s, done.\nPushed.\n"
        );
    }

    #[test]
    fn what_stood_last_is_kept_when_nothing_replaced_it() {
        // Ended where it stood, with a newline of its own or with none.
        let (seen, kept) = fed(&["one  \rtwo  \r\nthree\r"]);
        assert_eq!(seen, ["one", "two", "three"]);
        assert_eq!(kept, "two\nthree\n");
        // A last line without an end is a line.
        let (seen, kept) = fed(&["first\nsec", "ond"]);
        assert_eq!(seen, ["first", "second"]);
        assert_eq!(kept, "first\nsecond\n");
        // Nothing is nothing, and empty lines are not lines.
        assert_eq!(fed(&[]), (Vec::new(), String::new()));
        assert_eq!(fed(&["\n\r\n\r\r\n"]), (Vec::new(), String::new()));
    }

    #[test]
    fn output_without_end_stays_bounded() {
        let mut lines = Lines::default();
        let mut count = 0usize;
        // One line far longer than is kept of a line, then many lines.
        lines.feed(&vec![b'x'; 10 * LONGEST_LINE], &mut |_| count += 1);
        lines.feed(b"\n", &mut |line| {
            count += 1;
            assert_eq!(line.len(), LONGEST_LINE);
        });
        for _ in 0..40_000 {
            lines.feed("é is two bytes\n".as_bytes(), &mut |_| count += 1);
        }
        assert_eq!(count, 40_001);
        let kept = lines.finish(&mut |_| ());
        assert!(kept.len() <= KEPT, "{}", kept.len());
        assert!(kept.ends_with("é is two bytes\n"));
        // Bytes that are no text are kept as what can be shown of them.
        let mut lines = Lines::default();
        lines.feed(&[0xff, 0xfe, b'\n'], &mut |_| ());
        assert_eq!(lines.finish(&mut |_| ()), "\u{fffd}\u{fffd}\n");
    }

    #[test]
    fn a_file_is_followed_as_it_grows() {
        let dir = bitbraid_testkit::scratch_root().join(format!("follow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("report");
        let mut tail = Tail::of(Some(&file));
        let look = |tail: &mut Tail<'_>| {
            let mut seen = Vec::new();
            tail.poll(&mut |line| seen.push(line.to_owned()));
            seen
        };
        // Not there yet.
        assert_eq!(look(&mut tail), Vec::<String>::new());
        let mut write = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .unwrap();
        write.write_all(b"download 1/2 5/10 a.bin\ndownl").unwrap();
        assert_eq!(look(&mut tail), ["download 1/2 5/10 a.bin"]);
        assert_eq!(look(&mut tail), Vec::<String>::new());
        write
            .write_all(b"oad 1/2 10/10 a.bin\n\n\xff\ndownload 2/2 1/4 b.bin\n")
            .unwrap();
        assert_eq!(
            look(&mut tail),
            [
                "download 1/2 10/10 a.bin",
                "\u{fffd}",
                "download 2/2 1/4 b.bin"
            ]
        );
        // Written anew, it is read from its start.
        drop(write);
        std::fs::write(&file, "upload 1/1 1/2 c\n").unwrap();
        assert_eq!(look(&mut tail), ["upload 1/1 1/2 c"]);
        // A link is not followed, wherever it leads.
        #[cfg(unix)]
        {
            let link = dir.join("link");
            std::os::unix::fs::symlink(&file, &link).unwrap();
            let mut tail = Tail::of(Some(&link));
            assert_eq!(look(&mut tail), Vec::<String>::new());
        }
        // No file to follow.
        assert_eq!(look(&mut Tail::of(None)), Vec::<String>::new());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
