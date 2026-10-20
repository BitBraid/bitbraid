//! The git executable tier: operations that must behave exactly like the
//! user's git (hooks, signing, templates, credential helpers).
//!
//! Sources: git(1) (also for the environment: `GIT_OPTIONAL_LOCKS`,
//! `GIT_CEILING_DIRECTORIES`, `GIT_NO_LAZY_FETCH`, `GIT_ALLOW_PROTOCOL`),
//! git-commit(1), git-update-ref(1), git-config(1), git's documentation of
//! partial clone (`partial-clone.adoc`: objects fetched on demand).

use std::{
    cell::Cell,
    ffi::OsStr,
    io::Write as _,
    marker::PhantomData,
    path::Path,
    process::{Command, Stdio},
};

use crate::{Error, Repository, Result, activity};

mod follow;
mod wsl;

pub use follow::{Said, run_followed};
use wsl::{carry_environment, push_args};

/// Output of a git invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Output {
    /// Standard output.
    pub stdout: String,
    /// Standard error.
    pub stderr: String,
}

/// The executable the preferences name, when they name one. It is one
/// for the whole process: every repository that is open runs the same git.
static CHOSEN: std::sync::RwLock<Option<std::ffi::OsString>> = std::sync::RwLock::new(None);

/// How many times another executable was chosen. A repository that was
/// opened before the last time has read the configuration of a git that
/// is no longer the one that is run (`Repository::follow_git`).
static CHOICES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Run this executable as git from now on, for every child; `None` (or an
/// empty name) goes back to `$BITBRAID_GIT`, else `git` on `PATH`.
pub fn set_git_binary(program: Option<std::ffi::OsString>) {
    // A poisoned lock still holds a usable value: take it over.
    let mut chosen = match CHOSEN.write() {
        Ok(chosen) => chosen,
        Err(poisoned) => poisoned.into_inner(),
    };
    let program = program.filter(|program| !program.is_empty());
    if *chosen != program {
        *chosen = program;
        CHOICES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }
}

/// A number that is another one whenever another executable was chosen
/// with [`set_git_binary`]: what was read under one number is the
/// configuration of the git that was run then.
pub fn git_choice() -> u64 {
    CHOICES.load(std::sync::atomic::Ordering::SeqCst)
}

/// The executable that was chosen with [`set_git_binary`], if any.
pub fn chosen_git_binary() -> Option<std::ffi::OsString> {
    match CHOSEN.read() {
        Ok(chosen) => chosen.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Which executable to run: the one chosen with [`set_git_binary`], else
/// `$BITBRAID_GIT`, else `git` on `PATH`, else the git shipped with the
/// application ([`crate::shipped`]). With none of them it is `git`, which
/// fails to start and says so. While it is the shipped one, its folder is
/// put first on the `PATH` of every program the application starts
/// ([`crate::shipped::follow`]).
pub fn git_binary() -> std::ffi::OsString {
    let program = chosen_git_binary()
        .or_else(|| std::env::var_os("BITBRAID_GIT").filter(|set| !set.is_empty()))
        .or_else(|| crate::shipped::instead_of_path().map(std::path::PathBuf::into_os_string))
        .unwrap_or_else(|| "git".into());
    crate::shipped::follow(&program);
    program
}

thread_local! {
    /// Whether what this thread runs is work nobody asked for, which may
    /// read only what is on this machine (see [`local_only`]).
    static LOCAL_ONLY: Cell<bool> = const { Cell::new(false) };
}

/// Keeps the git this thread starts on this machine, until it is dropped.
#[derive(Debug)]
#[must_use = "git stays on this machine for as long as this is kept"]
pub struct LocalOnly {
    previous: bool,
    // Dropped on the thread it was made on, or it would let another's go.
    here: PhantomData<*const ()>,
}

/// Say that what this thread runs from now until what is returned is
/// dropped is work nobody asked for (keeping a view current, looking a
/// repository over in the background): it may read only what is on this
/// machine. Git is then run with lazy fetching off (`GIT_NO_LAZY_FETCH`,
/// git 2.44 and later), so that an object a partial clone left on its
/// remote is missing rather than fetched, one fetch and one pack at a
/// time; and with no transport allowed (`GIT_ALLOW_PROTOCOL` naming
/// none), which stops an older git, which fetches all the same, before it
/// reaches the remote. Reads that fall back to git for what a partial
/// clone left out ([`crate::promisor`]) do not fetch either. What is said
/// before is put back when it is dropped.
pub fn local_only() -> LocalOnly {
    let previous = LOCAL_ONLY
        .try_with(|cell| cell.replace(true))
        .unwrap_or(false);
    LocalOnly {
        previous,
        here: PhantomData,
    }
}

impl Drop for LocalOnly {
    fn drop(&mut self) {
        let _ = LOCAL_ONLY.try_with(|cell| cell.set(self.previous));
    }
}

/// Whether what this thread runs may read only what is on this machine
/// (see [`local_only`]).
pub fn is_local_only() -> bool {
    LOCAL_ONLY.try_with(Cell::get).unwrap_or(false)
}

/// What `GIT_ALLOW_PROTOCOL` is set to for work that stays on this
/// machine: a name no transport has, so that every one is refused.
const NO_PROTOCOL: &str = "none";

fn base_command(repo: &Repository) -> Command {
    command_in(repo.workdir().unwrap_or_else(|| repo.git_dir()))
}

/// Git, to be run in `dir`, with the environment every child of ours gets.
fn command_in(dir: &std::path::Path) -> Command {
    command_for(&git_binary(), dir)
}

/// `path` written for the shell git runs editors and `exec` lines with: on
/// Windows a path of a WSL distribution's share as the Linux path it is
/// there (git runs inside the distribution for such a repository, see
/// [`wsl`]), and any other with `/` between its names, as git writes
/// them; elsewhere as it is.
pub(crate) fn path_for_git_shell(path: &str) -> String {
    shell_path_on(path, cfg!(windows))
}

/// [`path_for_git_shell`] as it is on Windows (`windows`) or elsewhere.
fn shell_path_on(path: &str, windows: bool) -> String {
    if !windows {
        return path.to_owned();
    }
    match bitbraid_core::wsl::WslPath::parse(path) {
        Some(inside) => inside.linux,
        None => path.replace('\\', "/"),
    }
}

/// `program` run as git in `dir`, with the environment every child of ours
/// gets. On Windows, a folder inside a WSL distribution has the
/// distribution's git run for it instead (see [`wsl`]).
fn command_for(program: &OsStr, dir: &std::path::Path) -> Command {
    #[cfg(windows)]
    let inside = bitbraid_core::wsl::WslPath::of(dir);
    #[cfg(not(windows))]
    let inside: Option<bitbraid_core::wsl::WslPath> = None;
    let mut cmd = match &inside {
        Some(inside) => wsl::command(inside),
        None => {
            let mut cmd = Command::new(program);
            cmd.current_dir(dir);
            cmd
        }
    };
    bitbraid_core::process::no_window(&mut cmd);
    bitbraid_core::process::with_user_environment(&mut cmd);
    // Tests compare with an oracle that reads no system configuration.
    #[cfg(feature = "hermetic")]
    cmd.env("GIT_CONFIG_NOSYSTEM", "1");
    // Nothing may wait on a terminal that is not there, an editor included.
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "true")
        .env("LC_ALL", "C")
        // What is only asked must not write: `git status` would refresh
        // the index it reads, of this worktree or of another one, and the
        // watcher would take that for a change and ask again, for ever.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE");
    // Where a child that has to ask can ask, while the application runs.
    for (key, value) in crate::askpass::child_env() {
        cmd.env(key, value);
    }
    let ceiling = match &inside {
        Some(inside) => inside.linux_parent().map(std::ffi::OsString::from),
        None => ceiling_for(dir),
    };
    if let Some(ceiling) = ceiling {
        cmd.env("GIT_CEILING_DIRECTORIES", ceiling);
    }
    if is_local_only() {
        cmd.env("GIT_NO_LAZY_FETCH", "1")
            .env("GIT_ALLOW_PROTOCOL", NO_PROTOCOL);
    }
    cmd
}

/// What keeps git inside `dir`: it looks for a repository there and then
/// in every directory above, and the ones above must never be taken
/// instead. A repository whose own directory git no longer reads (a `HEAD`
/// that is garbage) would otherwise hand every command meant for it to
/// whatever repository happens to lie around it. The directory above is
/// put in front of what the user has set.
fn ceiling_for(dir: &std::path::Path) -> Option<std::ffi::OsString> {
    let above = std::path::absolute(dir).ok()?.parent()?.to_path_buf();
    let mut ceiling = above.into_os_string();
    if let Some(set) = std::env::var_os("GIT_CEILING_DIRECTORIES").filter(|set| !set.is_empty()) {
        ceiling.push(if cfg!(windows) { ";" } else { ":" });
        ceiling.push(set);
    }
    Some(ceiling)
}

/// A command line for an error message: `git` and its arguments, and
/// nothing of the environment, which names the askpass socket and its
/// token. Credentials inside a URL are left out, and so is the tail of an
/// argument too long to read.
fn render(cmd: &Command) -> String {
    let mut out = String::from("git");
    // Git inside a WSL distribution: what `wsl.exe` is told before git's
    // own arguments is left out.
    let head = if bitbraid_core::wsl::WslPath::from_invocation(cmd.get_args()).is_some() {
        bitbraid_core::wsl::WslPath::INVOCATION_LENGTH
    } else {
        0
    };
    for arg in cmd.get_args().skip(head) {
        out.push(' ');
        out.push_str(&redact(&arg.to_string_lossy()));
    }
    out
}

/// `scheme://user:secret@host` without the part before the `@`.
fn redact(arg: &str) -> String {
    const LONGEST: usize = 200;
    let mut text = arg.to_owned();
    if let Some(scheme_end) = text.find("://") {
        let rest_start = scheme_end.saturating_add(3);
        let rest = text.get(rest_start..).unwrap_or("");
        // User information ends at the last `@` before the path begins.
        let authority_end = rest.find('/').unwrap_or(rest.len());
        if let Some(at) = rest.get(..authority_end).and_then(|part| part.rfind('@')) {
            let host = rest.get(at.saturating_add(1)..).unwrap_or("").to_owned();
            text.truncate(rest_start);
            text.push_str("***@");
            text.push_str(&host);
        }
    }
    if text.len() > LONGEST {
        let mut cut = LONGEST;
        while !text.is_char_boundary(cut) {
            cut = cut.saturating_sub(1);
        }
        text.truncate(cut);
        text.push('…');
    }
    text
}

/// Run git in the repository with `args`, feeding `stdin` when given.
pub fn run<I, S>(repo: &Repository, args: I, stdin: Option<&[u8]>) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    checked(run_inner(base_command(repo), args, stdin)?)
}

/// Run git in the repository with more in its environment: what a single
/// command needs and no other child should see (the sequence editor of an
/// interactive rebase).
pub fn run_with_env<I, S>(repo: &Repository, args: I, env: &[(&str, &str)]) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    for (key, value) in env {
        cmd.env(key, value);
    }
    checked(run_inner(cmd, args, None)?)
}

/// [`run`] for a command that may run hooks of the repository (a commit,
/// a merge, a rebase, a checkout): when it fails because a hook ended
/// badly, the error is [`Error::Hook`], which says which hook and what it
/// said, and not git's own failure (see [`crate::hookwatch`]).
pub fn run_hooked<I, S>(repo: &Repository, args: I, stdin: Option<&[u8]>) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    run_hooked_with_env(repo, args, &[], stdin)
}

/// [`run_hooked`] with more in the environment, as [`run_with_env`] has.
pub fn run_hooked_with_env<I, S>(
    repo: &Repository,
    args: I,
    env: &[(&str, &str)],
    stdin: Option<&[u8]>,
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let Some(watch) = crate::hookwatch::Watch::begin(repo) else {
        return checked(run_inner(cmd, args, stdin)?);
    };
    let (key, value) = watch.env();
    cmd.env(key, value);
    let done = run_inner(cmd, args, stdin).and_then(checked);
    watch.explain(done)
}

/// [`run_with_code`] for a command that may run hooks: `judge` says what
/// the exit code and the output come to, and an error it gives because a
/// hook ended badly names the hook, as for [`run_hooked`].
pub(crate) fn run_hooked_with_code<I, S, T>(
    repo: &Repository,
    args: I,
    stdin: Option<&[u8]>,
    judge: impl FnOnce(Option<i32>, Output) -> Result<T>,
) -> Result<T>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    let watch = crate::hookwatch::Watch::begin(repo);
    if let Some(watch) = &watch {
        let (key, value) = watch.env();
        cmd.env(key, value);
    }
    let (status, _, out) = run_inner(cmd, args, stdin)?;
    let done = judge(status.code(), out);
    match watch {
        Some(watch) => watch.explain(done),
        None => done,
    }
}

/// Run git in a directory that is not (yet) a repository: `init`, and
/// questions that need none.
pub fn run_in<I, S>(dir: &std::path::Path, args: I) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    checked(run_inner(command_in(dir), args, None)?)
}

/// What `program --version` prints, for a program that may be anything:
/// the one in use when `program` is absent, else one the user is thinking
/// of using. It is given `limit` to answer and killed after that, so that a
/// program that is no git and never ends cannot hold anything up.
pub fn version_text(program: Option<&OsStr>, limit: std::time::Duration) -> Result<String> {
    let program = program.map(OsStr::to_os_string).unwrap_or_else(git_binary);
    asked_within(&program, &["--version"], limit).map(|out| out.stdout.trim().to_owned())
}

/// Ask `program`, run as git, something that needs no repository, and
/// give it `limit` to answer: it is killed after that, so that a program
/// that is no git and never ends cannot hold anything up. It runs in the
/// directory for temporary files.
pub(crate) fn asked_within(
    program: &OsStr,
    args: &[&str],
    limit: std::time::Duration,
) -> Result<Output> {
    use std::sync::{Arc, atomic::AtomicBool, atomic::Ordering};
    let dir = std::env::temp_dir();
    let cancel = Arc::new(AtomicBool::new(false));
    let done = Arc::new(AtomicBool::new(false));
    let timer = {
        let (cancel, done) = (cancel.clone(), done.clone());
        std::thread::Builder::new()
            .name("bitbraid-git-version".to_owned())
            .spawn(move || {
                let started = std::time::Instant::now();
                while !done.load(Ordering::SeqCst) {
                    if started.elapsed() >= limit {
                        cancel.store(true, Ordering::SeqCst);
                        return;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            })
            .ok()
    };
    let out = stream(command_for(program, &dir), args, &cancel, &mut |_| {});
    done.store(true, Ordering::SeqCst);
    if let Some(timer) = timer {
        let _ = timer.join();
    }
    match out {
        Ok(out) => Ok(out),
        Err(_) if cancel.load(Ordering::SeqCst) => Err(Error::Refused {
            what: say!("exe-what-git-executable"),
            detail: say!(
                "exe-no-answer-in-time",
                program = program.to_string_lossy(),
                command = args.first().copied().unwrap_or_default()
            ),
        }),
        Err(err) => Err(err),
    }
}

fn checked((status, rendered, out): (std::process::ExitStatus, String, Output)) -> Result<Output> {
    if !status.success() {
        return Err(Error::Command {
            command: rendered,
            status: status.to_string(),
            stderr: out.stderr.trim().to_owned(),
        });
    }
    Ok(out)
}

/// Run git where the exit code is part of the answer (`merge-tree` says
/// "conflicts" with 1): the code, none when a signal ended the process, and
/// what was written.
pub fn run_with_code<I, S>(
    repo: &Repository,
    args: I,
    stdin: Option<&[u8]>,
) -> Result<(Option<i32>, Output)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let (status, _, out) = run_inner(base_command(repo), args, stdin)?;
    Ok((status.code(), out))
}

/// [`run_with_code`] with more in the environment, as [`run_with_env`]
/// has: values that are paths, which need not be text.
pub fn run_with_code_env<I, S>(
    repo: &Repository,
    args: I,
    env: &[(&str, &OsStr)],
) -> Result<(Option<i32>, Output)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    for (key, value) in env {
        cmd.env(key, value);
    }
    let (status, _, out) = run_inner(cmd, args, None)?;
    Ok((status.code(), out))
}

fn run_inner<I, S>(
    mut cmd: Command,
    args: I,
    stdin: Option<&[u8]>,
) -> Result<(std::process::ExitStatus, String, Output)>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    push_args(&mut cmd, args);
    carry_environment(&mut cmd);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if stdin.is_some() {
        Stdio::piped()
    } else {
        Stdio::null()
    });
    let rendered = render(&cmd);
    // Every process goes into the activity log; one that is left behind on
    // an error below is entered as lost.
    let record = activity::begin(&cmd, &rendered);
    let mut child = cmd.spawn().map_err(|err| {
        record.not_started(&err);
        Error::Git {
            context: say!("exe-spawning"),
            source: Box::new(err),
        }
    })?;
    if let Some(bytes) = stdin
        && let Some(mut pipe) = child.stdin.take()
        && let Err(err) = pipe.write_all(bytes)
    {
        // Git ended before it read this: a hook refused the commit, say.
        // The process is collected first, so that the activity log has
        // what git, or the hook, said. When git ended badly, that is
        // the failure and what it said is the reason: a pipe that broke
        // is only how it came to light.
        drop(pipe);
        if let Ok(out) = child.wait_with_output() {
            let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            record.exited(out.status, &stdout, &stderr);
            if !out.status.success() {
                return Ok((out.status, rendered, Output { stdout, stderr }));
            }
        }
        return Err(Error::Git {
            context: say!("exe-writing-stdin"),
            source: Box::new(err),
        });
    }
    let out = child.wait_with_output().map_err(|err| Error::Git {
        context: say!("exe-waiting"),
        source: Box::new(err),
    })?;
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    record.exited(out.status, &stdout, &stderr);
    Ok((out.status, rendered, Output { stdout, stderr }))
}

/// Run git while it reports: every line it writes to standard error (a
/// carriage return ends a line too, which is how progress is rewritten in
/// place) goes to `on_line` as it arrives. Setting `cancel` kills the
/// process, and the result is then an error saying it was cancelled.
///
/// Nothing may wait on a terminal that is not there. Without an askpass
/// program, the user's or ours (see [`crate::askpass`]), ssh is pointed at
/// one that fails at once rather than at the controlling terminal.
pub fn run_streaming<I, S>(
    repo: &Repository,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    stream(base_command(repo), args, cancel, on_line)
}

/// [`run_streaming`] for a command that changes the repository and may
/// run its hooks on the way (a push its `pre-push`, a pull the hooks of
/// a merge or a rebase): a failure that is a hook's is told apart from
/// git's, as for [`run_hooked`]. Not for what only reads: the watch
/// writes a file into the repository's git directory while it runs.
pub fn run_streaming_hooked<I, S>(
    repo: &Repository,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    let Some(watch) = crate::hookwatch::Watch::begin(repo) else {
        return stream(cmd, args, cancel, on_line);
    };
    let (key, value) = watch.env();
    cmd.env(key, value);
    stream_watched(cmd, args, cancel, on_line, Some(&watch))
}

/// [`run_streaming`] for what runs with nobody looking (a fetch on a
/// timer): no program is named that could ask the user anything. Git is
/// given an askpass program that fails at once in place of the user's or
/// ours, ssh the same, and the askpass server of the application is not
/// named to the child. Credential helpers and the ssh agent answer as
/// they always do; a remote that needs more fails with git's own message
/// (see [`crate::remote::wants_answer`]).
pub fn run_streaming_unasked<I, S>(
    repo: &Repository,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut cmd = base_command(repo);
    unasked(&mut cmd);
    stream(cmd, args, cancel, on_line)
}

/// Take from a command every way of asking the user. `GIT_ASKPASS` comes
/// before `core.askPass` and `SSH_ASKPASS` for git, and ssh is told to use
/// its askpass program whether or not there is a terminal.
fn unasked(cmd: &mut Command) {
    cmd.env("GIT_ASKPASS", "false")
        .env("SSH_ASKPASS", "false")
        .env("SSH_ASKPASS_REQUIRE", "force")
        // The credential manager some systems use opens windows of its own.
        .env("GCM_INTERACTIVE", "never")
        .env_remove(crate::askpass::SOCKET_ENV)
        .env_remove(crate::askpass::TOKEN_ENV);
}

/// [`run_streaming`] in a directory that is not a repository: `clone`.
pub fn run_streaming_in<I, S>(
    dir: &std::path::Path,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    stream(command_in(dir), args, cancel, on_line)
}

/// Keep a command that talks to a remote from waiting on a terminal:
/// without an askpass program, the user's or ours, ssh is pointed at one
/// that fails at once.
fn unattended(cmd: &mut Command) {
    cmd.env("SSH_ASKPASS_REQUIRE", "force");
    if std::env::var_os("SSH_ASKPASS").is_none() && crate::askpass::child_env().is_empty() {
        cmd.env("SSH_ASKPASS", "false");
    }
}

fn stream<I, S>(
    cmd: Command,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    stream_watched(cmd, args, cancel, on_line, None)
}

/// [`stream`], and with a `watch` a failure that is a hook's says so
/// (see [`crate::hookwatch`]), with what the hook wrote on either output:
/// git hands a `pre-push` hook its own standard output, and git-lfs says
/// there which locked files stopped a push.
fn stream_watched<I, S>(
    mut cmd: Command,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    on_line: &mut dyn FnMut(&str),
    watch: Option<&crate::hookwatch::Watch>,
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::{
        io::Read,
        sync::{Arc, Mutex, atomic::Ordering},
    };

    push_args(&mut cmd, args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unattended(&mut cmd);
    carry_environment(&mut cmd);
    let rendered = render(&cmd);
    let failed = |context: &'static str, err: std::io::Error| Error::Git {
        context,
        source: Box::new(err),
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    let record = activity::begin(&cmd, &rendered);
    // In a group of its own and on a leash (see `follow`): what git
    // starts, a hook and what the hook starts, ends with it when the
    // command is stopped and when the application dies.
    let (mut child, leash) = follow::spawn_leashed(cmd).map_err(|err| {
        record.not_started(&err);
        failed(say!("exe-spawning"), err)
    })?;
    let mut stderr = child.stderr.take();
    let mut stdout = child.stdout.take();
    let child = Arc::new(Mutex::new(child));
    let done = Arc::new(std::sync::atomic::AtomicBool::new(false));

    // End the group when asked to: asked first, killed when it is still
    // there a moment later. Look often enough to feel immediate. The
    // child is only ever signalled while it has not been waited for, so
    // that its number names it and nothing else.
    let watcher = {
        let (child, done, cancel) = (child.clone(), done.clone(), cancel.clone());
        std::thread::Builder::new()
            .name("bitbraid-git-cancel".to_owned())
            .spawn(move || {
                let mut stop = follow::Stop::Not;
                while !done.load(Ordering::SeqCst) {
                    if cancel.load(Ordering::SeqCst) {
                        let Ok(mut child) = child.lock() else {
                            break;
                        };
                        if done.load(Ordering::SeqCst) || !stop.step(&mut child) {
                            break;
                        }
                    }
                    std::thread::sleep(std::time::Duration::from_millis(40));
                }
            })
            .ok()
    };
    let out_reader = std::thread::Builder::new()
        .name("bitbraid-git-stdout".to_owned())
        .spawn(move || {
            let mut text = Vec::new();
            if let Some(pipe) = stdout.as_mut() {
                let _ = pipe.read_to_end(&mut text);
            }
            text
        })
        .ok();

    // Lines as they come; keep the tail for an error message.
    let mut all = Vec::new();
    let mut line = Vec::new();
    let mut buffer = [0u8; 4096];
    if let Some(pipe) = stderr.as_mut() {
        loop {
            let read = match pipe.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(read) => read,
            };
            for byte in buffer.iter().take(read) {
                if *byte == b'\n' || *byte == b'\r' {
                    if !line.is_empty() {
                        on_line(String::from_utf8_lossy(&line).trim_end());
                        all.extend_from_slice(&line);
                        all.push(b'\n');
                        line.clear();
                    }
                } else {
                    line.push(*byte);
                }
            }
            // A stream of progress must not grow without bound.
            if all.len() > 256 * 1024 {
                let keep = all.len().saturating_sub(64 * 1024);
                all.drain(..keep);
            }
        }
    }
    if !line.is_empty() {
        on_line(String::from_utf8_lossy(&line).trim_end());
        all.extend_from_slice(&line);
    }
    // Not `wait`: the one who stops it needs the handle in between, and
    // may have to kill what did not end when asked.
    let status = loop {
        let waited = child
            .lock()
            .map_err(|_| Error::Invalid {
                what: say!("exe-what-git-command"),
                detail: say!("exe-handle-lost").to_owned(),
            })
            .and_then(|mut child| {
                let waited = child
                    .try_wait()
                    .map_err(|err| failed(say!("exe-waiting"), err))?;
                if waited.is_some() {
                    done.store(true, Ordering::SeqCst);
                }
                Ok(waited)
            })?;
        match waited {
            Some(status) => break status,
            None => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    };
    drop(leash);
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
    let stdout = out_reader
        .and_then(|reader| reader.join().ok())
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    let stderr = String::from_utf8_lossy(&all).into_owned();
    if cancel.load(Ordering::SeqCst) {
        record.cancelled(&stdout, &stderr);
        return Err(cancelled());
    }
    record.exited(status, &stdout, &stderr);
    if !status.success() {
        let failed = Err(Error::Command {
            command: rendered,
            status: status.to_string(),
            stderr: stderr.trim().to_owned(),
        });
        return match watch {
            Some(watch) => watch.explain_with(failed, &stdout),
            None => failed,
        };
    }
    Ok(Output { stdout, stderr })
}

/// Git for a command that starts a program of the user's own (a diff or
/// merge tool): as for every other child, but in the user's locale, which
/// is the one that program shows itself in.
fn tool_command(repo: &Repository) -> Command {
    let mut cmd = base_command(repo);
    match std::env::var_os("LC_ALL") {
        Some(value) => cmd.env("LC_ALL", value),
        None => cmd.env_remove("LC_ALL"),
    };
    cmd
}

/// How long a tool is watched after it was started, to catch the ones that
/// give up at once. On Windows git reaches the tool through its sh and
/// the difftool script, each of them a program of MSYS2 that is slow to
/// start: a command that fails at once ends there more than half a second
/// after it was started, and 400 ms would take it for one that runs.
const TOOL_GRACE: std::time::Duration = if cfg!(windows) {
    std::time::Duration::from_millis(1500)
} else {
    std::time::Duration::from_millis(400)
};

/// Start git with a tool and leave it running. When it gives up within a
/// moment (there is no such tool, say) that is an error with what it said;
/// otherwise a thread collects it when the tool is closed.
pub fn start_tool<I, S>(repo: &Repository, args: I) -> Result<()>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::io::Read as _;
    let mut cmd = tool_command(repo);
    push_args(&mut cmd, args);
    carry_environment(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let rendered = render(&cmd);
    let failed = |context: &'static str, err: std::io::Error| Error::Git {
        context,
        source: Box::new(err),
    };
    let record = activity::begin(&cmd, &rendered);
    let mut child = cmd.spawn().map_err(|err| {
        record.not_started(&err);
        failed(say!("exe-spawning"), err)
    })?;
    let started = std::time::Instant::now();
    while started.elapsed() < TOOL_GRACE {
        match child.try_wait() {
            Ok(Some(status)) => {
                if status.success() {
                    record.exited(status, "", "");
                    return Ok(());
                }
                let mut said = String::new();
                if let Some(pipe) = child.stderr.as_mut() {
                    // What a process that has ended wrote, cut short.
                    let _ = pipe.take(64 * 1024).read_to_string(&mut said);
                }
                record.exited(status, "", &said);
                return Err(Error::Command {
                    command: rendered,
                    status: status.to_string(),
                    stderr: said.trim().to_owned(),
                });
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(20)),
            Err(err) => return Err(failed(say!("exe-waiting"), err)),
        }
    }
    // Still there: it is the user's now. Read what it writes so that it
    // never waits on a full pipe, and collect it when it ends. Until then
    // the activity log has it as running.
    record.running();
    let _ = std::thread::Builder::new()
        .name("bitbraid-tool".to_owned())
        .spawn(move || {
            let said = child.stderr.take().map(read_tail).unwrap_or_default();
            if let Ok(status) = child.wait() {
                record.exited(status, "", &said);
            }
        });
    Ok(())
}

/// Read a pipe to its end and keep the end of what came, as much as the
/// activity log takes: the process never waits on a full pipe, and one
/// that runs for hours does not fill memory.
fn read_tail(mut pipe: impl std::io::Read) -> String {
    const KEEP: usize = activity::OUTPUT_LIMIT;
    let mut kept = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let read = match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        kept.extend(buffer.iter().take(read));
        if kept.len() > 2 * KEEP {
            let excess = kept.len().saturating_sub(KEEP);
            kept.drain(..excess);
        }
    }
    String::from_utf8_lossy(&kept).into_owned()
}

/// Run git with a tool and wait for it. Setting `cancel` stops waiting
/// and ends git and the tool with it: they are a process group of their
/// own on a leash, as a fetch is (see `follow`), asked to end and killed
/// two seconds later where they have not. A tool that hands its window to
/// an instance of its own outside the group (an editor that was running
/// already) keeps that window; only the waiting ends. The group also
/// ends when the application does. What git writes goes to a file rather
/// than a pipe: the tool inherits it, and a pipe would be waited on for
/// as long as the tool, or an instance it started, stays open.
pub fn run_tool<I, S>(
    repo: &Repository,
    args: I,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Result<Output>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    use std::{io::Read as _, sync::atomic::Ordering};
    let failed = |context: &'static str, err: std::io::Error| Error::Git {
        context,
        source: Box::new(err),
    };
    if cancel.load(Ordering::SeqCst) {
        return Err(cancelled());
    }
    let dir = repo.git_dir().join("bitbraid");
    std::fs::create_dir_all(&dir).map_err(|err| failed(say!("exe-preparing-tool"), err))?;
    let log_path = dir.join(format!("tool-{}.log", std::process::id()));
    let log =
        std::fs::File::create(&log_path).map_err(|err| failed(say!("exe-preparing-tool"), err))?;
    let mut cmd = tool_command(repo);
    push_args(&mut cmd, args);
    carry_environment(&mut cmd);
    let rendered = render(&cmd);
    let record = activity::begin(&cmd, &rendered);
    let spawned = follow::spawn_leashed_to(cmd, follow::Outputs::Log(&log));
    drop(log);
    let (mut child, leash) = match spawned {
        Ok(spawned) => spawned,
        Err(err) => {
            record.not_started(&err);
            let _ = std::fs::remove_file(&log_path);
            return Err(failed(say!("exe-spawning"), err));
        }
    };
    let mut stop = follow::Stop::Not;
    let status = loop {
        if cancel.load(Ordering::SeqCst) && !stop.step(&mut child) {
            // Killed, and still not seen to end: not waited on further.
            break None;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(40)),
            Err(err) => {
                let _ = std::fs::remove_file(&log_path);
                return Err(failed(say!("exe-waiting"), err));
            }
        }
    };
    // Git has ended. What is left of its group (a tool that did not end
    // when asked) is ended by the group's own watch once the leash is let
    // go of.
    drop(leash);
    let mut stderr = String::new();
    if let Ok(file) = std::fs::File::open(&log_path) {
        let mut bytes = Vec::new();
        let _ = file.take(64 * 1024).read_to_end(&mut bytes);
        stderr = String::from_utf8_lossy(&bytes).trim().to_owned();
    }
    let _ = std::fs::remove_file(&log_path);
    let status = match status {
        Some(status) if !cancel.load(Ordering::SeqCst) => status,
        _ => {
            record.cancelled("", &stderr);
            return Err(cancelled());
        }
    };
    record.exited(status, "", &stderr);
    if !status.success() {
        return Err(Error::Command {
            command: rendered,
            status: status.to_string(),
            stderr,
        });
    }
    Ok(Output {
        stdout: String::new(),
        stderr,
    })
}

/// The error of an operation that was stopped on request.
pub(crate) fn cancelled() -> Error {
    Error::Invalid {
        what: say!("exe-what-operation"),
        detail: say!("exe-cancelled").to_owned(),
    }
}

/// Read a configuration value as git resolves it (all scopes).
pub fn config_get(repo: &Repository, key: &str) -> Option<String> {
    run(repo, ["config", "--get", key], None)
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// Resolve a path from configuration, expanding `~`.
pub fn config_path(repo: &Repository, key: &str) -> Option<std::path::PathBuf> {
    let raw = run(repo, ["config", "--get", "--type=path", key], None).ok()?;
    let value = raw.stdout.trim();
    if value.is_empty() {
        return None;
    }
    let path = Path::new(value);
    if path.is_absolute() {
        Some(path.to_path_buf())
    } else {
        repo.workdir().map(|wd| wd.join(path))
    }
}

/// What the activity log says of a value it was not given.
pub static WITHHELD: bitbraid_strings::Message = message!("exe-withheld");

/// A setting of git's own configuration outside any repository (the
/// user's files and the system's, as git reads them), for what the
/// application does without a repository: the proxy of its own
/// connections (`http.proxy`). The command is entered in the activity
/// log, its value is not ([`WITHHELD`]): an address there may hold a
/// password, and one without a scheme (`user:password@host`) is not
/// one the log can take the password out of. Git is given `limit` to
/// answer and stopped after that. None for a key that is not plain, a
/// setting that is not there or empty, and a git that fails.
pub fn user_setting(key: &str, limit: std::time::Duration) -> Option<String> {
    setting_outside(key, limit, &[])
}

/// The value git takes for `key` when it talks to `url` (`git config
/// --get-urlmatch`: `http.<url>.<name>` before `http.<name>`), read in
/// the repository at `dir` (its configuration, the user's and the
/// system's) or, without one, outside any repository; the environment's
/// `GIT_CONFIG_*` counts as it does for git. In the activity log as
/// [`user_setting`] is, its value withheld. None for a key that is not
/// plain, an address that is not an `http://` or `https://` one, a
/// setting that is not there or empty, and a git that fails.
pub fn url_setting(
    dir: Option<&std::path::Path>,
    key: &str,
    url: &str,
    limit: std::time::Duration,
) -> Option<String> {
    let address = url.len() <= 2048
        && (url.starts_with("http://") || url.starts_with("https://"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace());
    if !plain_key(key) || !address {
        return None;
    }
    let temp = std::env::temp_dir();
    read_setting(
        dir.unwrap_or(&temp),
        &["config", "--get-urlmatch", key, url],
        limit,
        &[],
    )
}

/// Whether `key` is a key of git's configuration and nothing else.
fn plain_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 256
        && !key.starts_with(['-', '.'])
        && key
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
}

fn setting_outside(key: &str, limit: std::time::Duration, env: &[(&str, &str)]) -> Option<String> {
    if !plain_key(key) {
        return None;
    }
    read_setting(&std::env::temp_dir(), &["config", "--get", key], limit, env)
}

/// What `git <args>` run in `dir` prints, trimmed, when it succeeds and
/// prints something, within `limit`; in the activity log with the value
/// withheld.
fn read_setting(
    dir: &std::path::Path,
    args: &[&str],
    limit: std::time::Duration,
    env: &[(&str, &str)],
) -> Option<String> {
    use std::io::Read as _;
    let mut cmd = command_in(dir);
    for (name, value) in env {
        cmd.env(name, value);
    }
    push_args(&mut cmd, args);
    carry_environment(&mut cmd);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let rendered = render(&cmd);
    let record = activity::begin(&cmd, &rendered);
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(err) => {
            record.not_started(&err);
            return None;
        }
    };
    let started = std::time::Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() < limit => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                record.cancelled("", "");
                return None;
            }
        }
    };
    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    let value = stdout.trim().to_owned();
    record.exited(
        status,
        if value.is_empty() {
            ""
        } else {
            WITHHELD.text()
        },
        &stderr,
    );
    (status.success() && !value.is_empty()).then_some(value)
}

#[cfg(test)]
mod tests {
    use super::{redact, render, shell_path_on};
    use std::process::Command;

    #[test]
    fn a_path_for_git_shell_is_written_as_the_git_that_reads_it_sees_it() {
        let todo = r"\\wsl$\Ubuntu\home\me\repo\.git\bitbraid\rebase\todo";
        assert_eq!(
            shell_path_on(todo, true),
            "/home/me/repo/.git/bitbraid/rebase/todo"
        );
        assert_eq!(
            shell_path_on(r"\\wsl.localhost\Ubuntu\home\me\x", true),
            "/home/me/x"
        );
        assert_eq!(
            shell_path_on(r"C:\work\repo\.git\todo", true),
            "C:/work/repo/.git/todo"
        );
        assert_eq!(
            shell_path_on("/tmp/it's here/todo", false),
            "/tmp/it's here/todo"
        );
        assert_eq!(shell_path_on(todo, false), todo);
    }

    #[test]
    fn a_rendered_command_has_no_environment_and_no_credentials() {
        let mut cmd = Command::new("git");
        cmd.env("BITBRAID_ASKPASS_TOKEN", "do-not-print")
            .env("SSH_ASKPASS", "/somewhere/askpass")
            .current_dir("/")
            .args([
                "push",
                "--",
                "https://ada:hunter2@example.invalid/repo.git",
                "main",
            ]);
        assert_eq!(
            render(&cmd),
            "git push -- https://***@example.invalid/repo.git main"
        );
    }

    #[test]
    fn redaction_leaves_everything_else_alone() {
        for (given, shown) in [
            (
                "https://example.invalid/a.git",
                "https://example.invalid/a.git",
            ),
            (
                "https://token@example.invalid/a.git",
                "https://***@example.invalid/a.git",
            ),
            (
                "ssh://git@example.invalid:22/a.git",
                "ssh://***@example.invalid:22/a.git",
            ),
            // An `@` in the path is not user information.
            (
                "https://example.invalid/a@b.git",
                "https://example.invalid/a@b.git",
            ),
            // A password with an `@` in it goes as a whole.
            (
                "https://u:p@ss@example.invalid/a",
                "https://***@example.invalid/a",
            ),
            ("git@example.invalid:a/b.git", "git@example.invalid:a/b.git"),
            ("refs/heads/main", "refs/heads/main"),
            ("", ""),
            ("://", "://"),
            ("x://@", "x://***@"),
        ] {
            assert_eq!(redact(given), shown, "{given}");
        }
        // Too long to read: cut at a character, marked as cut.
        let long = "é".repeat(500);
        let shown = redact(&long);
        assert!(
            shown.len() <= 204 && shown.ends_with('…'),
            "{}",
            shown.len()
        );
    }

    #[test]
    fn a_setting_outside_a_repository_is_read_and_its_value_kept_out_of_the_log() {
        use super::{WITHHELD, setting_outside, user_setting};
        use std::time::Duration;
        let limit = Duration::from_secs(20);
        // A password without a scheme, which the log could not find.
        let value = "carol:hunter2@proxy.example.invalid:3128";
        let given = [
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "bitbraid.testproxy"),
            ("GIT_CONFIG_VALUE_0", value),
        ];
        assert_eq!(
            setting_outside("bitbraid.testproxy", limit, &given).as_deref(),
            Some(value)
        );
        let entries = crate::activity::snapshot();
        let entry = entries
            .iter()
            .rev()
            .find(|entry| entry.command.contains("bitbraid.testproxy"))
            .expect("the command is in the log");
        assert_eq!(entry.stdout, WITHHELD.text());
        assert!(
            entries.iter().all(
                |entry| !entry.stdout.contains("hunter2") && !entry.command.contains("hunter2")
            )
        );
        assert!(entry.question);
        // Not there, and not a key at all.
        assert_eq!(
            user_setting("bitbraid.surely-not-set-anywhere", limit),
            None
        );
        for key in ["", "-l", "--list", ".x", "a b", "a\nb", "a=b"] {
            assert_eq!(user_setting(key, limit), None, "{key:?}");
        }
    }

    #[test]
    fn a_setting_for_an_address_is_the_one_git_takes_for_it() {
        use super::url_setting;
        use std::time::Duration;
        let limit = Duration::from_secs(20);
        let dir = std::env::temp_dir().join(format!("bitbraid-url-setting-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let git = |args: &[&str]| {
            let out = std::process::Command::new("git")
                .args(args)
                .current_dir(&dir)
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .output()
                .unwrap();
            assert!(out.status.success(), "{out:?}");
        };
        git(&["init", "-q"]);
        git(&["config", "http.proxyAuthMethod", "basic"]);
        git(&[
            "config",
            "http.https://git.example.test/.proxyAuthMethod",
            "negotiate",
        ]);
        let read = |url: &str| url_setting(Some(&dir), "http.proxyAuthMethod", url, limit);
        assert_eq!(
            read("https://git.example.test/r.git").as_deref(),
            Some("negotiate")
        );
        assert_eq!(
            read("https://other.example.test/r.git").as_deref(),
            Some("basic")
        );
        // Not an address, or not a key: nothing is run.
        for url in [
            "",
            "-l",
            "--list",
            "file:///x",
            "https://a b",
            "https://a\nb",
        ] {
            assert_eq!(read(url), None, "{url:?}");
        }
        assert_eq!(
            url_setting(Some(&dir), "--list", "https://git.example.test/", limit),
            None
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
