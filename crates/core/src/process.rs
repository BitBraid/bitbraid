//! Programs the application starts.
//!
//! The application has no console on Windows. A console program it starts
//! (git, ssh-keygen, gpg) would get a console window of its own, which
//! flashes up and goes for every git that runs, unless it is asked not to
//! have one. Programs that show a window of their own (an editor, a file
//! manager, a terminal) are started as they are.
//!
//! The environment they are started with is the application's, except
//! where the application was started by the system rather than from a
//! shell: on macOS, from the Finder, the Dock or a link, it has the
//! system's few variables, and a `PATH` with none of what the user
//! installed (Homebrew's git, git-lfs, gpg, an editor's command). The
//! user's login shell is then asked once for its environment
//! ([`login_environment`]), which every program started from then on gets
//! ([`adopt_environment`], [`with_user_environment`]), and in which
//! programs are looked for ([`var_os`]).
//!
//! While the git in use is the one shipped with the application, its
//! folder stands first on the `PATH` of every program started
//! ([`set_path_first`]): a hook, an editor, a tool or the shell of the
//! terminal that runs `git` runs that one, on a system that has no other.
//! Programs are still looked for in the `PATH` without it ([`var_os`]),
//! which is where the user's own are.
//!
//! Sources: Microsoft Learn, "Process Creation Flags" (`CREATE_NO_WINDOW`);
//! Apple's "Daemons and Services Programming Guide" (what launchd gives an
//! application it starts) and zsh(1), bash(1), fish(1) (`-l`, `-i`, `-c`;
//! which files a login and an interactive shell read); env(1) (`-0`);
//! environ(7) (`PATH`); Rust's `std::env::join_paths`.

use std::{
    ffi::{OsStr, OsString},
    io::Read as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{OnceLock, RwLock},
    time::{Duration, Instant},
};

/// `CREATE_NO_WINDOW`: a console program runs without a console window.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Start `command` without a window of its own: on Windows a console
/// program then runs without a console window; elsewhere nothing changes.
pub fn no_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        command.creation_flags(CREATE_NO_WINDOW)
    }
    #[cfg(not(windows))]
    {
        command
    }
}

/// The environment the user's login shell has, once it was adopted.
static ADOPTED: OnceLock<Vec<(OsString, OsString)>> = OnceLock::new();

/// Variables of a shell's own that say nothing about the user's
/// surroundings: where that shell was, how deep, what it ran last.
const SHELLS_OWN: &[&str] = &["PWD", "OLDPWD", "SHLVL", "_"];

/// Start every program from now on with `vars` for its environment, over
/// the application's own. Only the first call counts; says whether this
/// one did.
pub fn adopt_environment(vars: Vec<(OsString, OsString)>) -> bool {
    let vars = vars
        .into_iter()
        .filter(|(name, _)| !SHELLS_OWN.iter().any(|own| name == own))
        .collect();
    ADOPTED.set(vars).is_ok()
}

/// Whether the environment of the user's login shell was adopted.
pub fn adopted() -> bool {
    ADOPTED.get().is_some()
}

/// The folder put first on the `PATH` of the programs started, if any.
static PATH_FIRST: RwLock<Option<PathBuf>> = RwLock::new(None);

/// Put `folder` first on the `PATH` of every program started from now on,
/// or nothing (`None`). Says whether that changed anything.
pub fn set_path_first(folder: Option<PathBuf>) -> bool {
    let unchanged = match PATH_FIRST.read() {
        Ok(first) => *first == folder,
        Err(poisoned) => *poisoned.into_inner() == folder,
    };
    if unchanged {
        return false;
    }
    match PATH_FIRST.write() {
        Ok(mut first) => *first = folder,
        Err(poisoned) => *poisoned.into_inner() = folder,
    }
    true
}

/// The folder put first on the `PATH` of the programs started, if any.
pub fn path_first() -> Option<PathBuf> {
    match PATH_FIRST.read() {
        Ok(first) => first.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// `path` with `first` before what it has, unless it begins with it
/// already. None where the folder cannot stand in a `PATH` (on Unix it
/// has the separator in it; on Windows, where one with the separator is
/// put in quotes, a quote), and then the `PATH` is left as it is.
pub fn path_with_first(first: &Path, path: Option<&OsStr>) -> Option<OsString> {
    let rest: Vec<PathBuf> = path
        .map(std::env::split_paths)
        .into_iter()
        .flatten()
        .collect();
    if rest.first().is_some_and(|already| already == first) {
        return None;
    }
    std::env::join_paths(std::iter::once(first.to_path_buf()).chain(rest)).ok()
}

/// The `PATH` of the programs started, when it is another than the one
/// they would have without the folder put first.
fn child_path() -> Option<OsString> {
    let first = path_first()?;
    path_with_first(&first, var_os("PATH").as_deref())
}

/// A variable as the programs the application starts see it, but `PATH`
/// without the folder [`set_path_first`] puts first: it is what programs
/// are looked for in.
pub fn var_os(name: &str) -> Option<OsString> {
    match ADOPTED.get() {
        Some(vars) => vars
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.clone())
            .or_else(|| std::env::var_os(name)),
        None => std::env::var_os(name),
    }
}

/// Every variable, as the programs the application starts see them: the
/// application's own, with the adopted ones over them, and `PATH` with
/// the folder [`set_path_first`] put first.
pub fn vars_os() -> Vec<(OsString, OsString)> {
    let mut vars: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    let mut set = |name: &OsStr, value: OsString| match vars.iter_mut().find(|(key, _)| key == name)
    {
        Some(slot) => slot.1 = value,
        None => vars.push((name.to_os_string(), value)),
    };
    if let Some(adopted) = ADOPTED.get() {
        for (name, value) in adopted {
            set(name, value.clone());
        }
    }
    if let Some(path) = child_path() {
        set(OsStr::new("PATH"), path);
    }
    vars
}

/// Give `command` the adopted environment, when there is one, and the
/// `PATH` with the folder [`set_path_first`] put first. Called right after
/// it is made: what it sets or removes later is its own and stays.
pub fn with_user_environment(command: &mut Command) -> &mut Command {
    if let Some(adopted) = ADOPTED.get() {
        for (name, value) in adopted {
            if std::env::var_os(name).as_deref() != Some(value.as_os_str()) {
                command.env(name, value);
            }
        }
    }
    if let Some(path) = child_path() {
        command.env("PATH", path);
    }
    command
}

/// What marks the start and the end of the environment in what a shell
/// writes, so that what its files print is left out.
const MARK: &str = "__BITBRAID_ENVIRONMENT__";

/// The shells whose environment is read, by their name: those that take
/// `-l -i -c` and run `env` from there.
const KNOWN_SHELLS: &[&str] = &["zsh", "bash", "sh", "ksh", "dash", "fish"];

/// The environment of `shell` started as the user's login shell is, and
/// as an interactive one (many put their `PATH` in `.zshrc`): read from
/// what it writes, between two marks. None for a shell that is not known
/// to take this, one that fails, and one that takes longer than `wait`
/// (it is ended then).
pub fn login_environment(shell: &Path, wait: Duration) -> Option<Vec<(OsString, OsString)>> {
    let name = shell.file_name()?.to_str()?;
    if !shell.is_absolute() || !KNOWN_SHELLS.contains(&name) {
        return None;
    }
    let script = format!("echo {MARK}; /usr/bin/env -0; echo {MARK}");
    let mut child = Command::new(shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        // At most a few megabytes: an environment is far less.
        let _ = (&mut stdout).take(8 << 20).read_to_end(&mut out);
        out
    });
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break,
            Ok(Some(_)) | Err(_) => return None,
            Ok(None) if started.elapsed() >= wait => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(5)),
        }
    }
    let out = reader.join().ok()?;
    parse_environment(&out)
}

/// The variables between the two marks of what a shell wrote (`env -0`:
/// `NAME=value`, each ended by a NUL). None without both marks.
fn parse_environment(out: &[u8]) -> Option<Vec<(OsString, OsString)>> {
    let mark = format!("{MARK}\n");
    let mark = mark.as_bytes();
    let start = find(out, mark)?.checked_add(mark.len())?;
    let rest = out.get(start..)?;
    let end = find(rest, mark)?;
    let body = rest.get(..end)?;
    let mut vars = Vec::new();
    for entry in body.split(|byte| *byte == 0) {
        let Some(equals) = entry.iter().position(|byte| *byte == b'=') else {
            continue;
        };
        let (name, value) = (entry.get(..equals)?, entry.get(equals.checked_add(1)?..)?);
        if name.is_empty() {
            continue;
        }
        vars.push((os_string(name)?, os_string(value)?));
    }
    (!vars.is_empty()).then_some(vars)
}

/// Where `needle` first stands in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// Bytes as a string of the system: any bytes on Unix; UTF-8 elsewhere.
fn os_string(bytes: &[u8]) -> Option<OsString> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        Some(std::ffi::OsStr::from_bytes(bytes).to_owned())
    }
    #[cfg(not(unix))]
    {
        std::str::from_utf8(bytes).ok().map(OsString::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_environment_is_read_between_the_marks() {
        let out = format!(
            "Welcome!\n{MARK}\nPATH=/opt/homebrew/bin:/usr/bin\0EMPTY=\0WITH_EQUALS=a=b\0\
             MULTI=one\ntwo\0=no name\0no equals\0{MARK}\ngoodbye\n"
        );
        let vars = parse_environment(out.as_bytes()).unwrap();
        let pairs: Vec<(String, String)> = vars
            .into_iter()
            .map(|(name, value)| {
                (
                    name.to_string_lossy().into_owned(),
                    value.to_string_lossy().into_owned(),
                )
            })
            .collect();
        assert_eq!(
            pairs,
            [
                ("PATH", "/opt/homebrew/bin:/usr/bin"),
                ("EMPTY", ""),
                ("WITH_EQUALS", "a=b"),
                ("MULTI", "one\ntwo"),
            ]
            .map(|(name, value)| (name.to_owned(), value.to_owned()))
        );
    }

    #[test]
    fn without_both_marks_nothing_is_read() {
        for out in [
            String::new(),
            "PATH=/usr/bin\0".to_owned(),
            format!("{MARK}\nPATH=/usr/bin\0"),
            format!("{MARK}\n{MARK}\n"),
            format!("{MARK}PATH=/usr/bin\0{MARK}"),
        ] {
            assert_eq!(parse_environment(out.as_bytes()), None, "{out:?}");
        }
    }

    #[test]
    fn a_folder_is_put_first_on_a_path_once() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let first = Path::new("/opt/bitbraid/libexec/bitbraid/git/bin");
        let path = OsString::from(format!("/usr/local/bin{sep}/usr/bin"));
        assert_eq!(
            path_with_first(first, Some(path.as_os_str())),
            Some(OsString::from(format!(
                "/opt/bitbraid/libexec/bitbraid/git/bin{sep}/usr/local/bin{sep}/usr/bin"
            )))
        );
        // Already first: nothing to change.
        let already = path_with_first(first, Some(path.as_os_str())).unwrap();
        assert_eq!(path_with_first(first, Some(already.as_os_str())), None);
        // No PATH at all: the folder alone.
        assert_eq!(
            path_with_first(first, None),
            Some(first.as_os_str().to_os_string())
        );
        // A folder that cannot stand in a PATH leaves it as it is: on
        // Windows one with a quote, on Unix one with the separator.
        let broken = PathBuf::from(if cfg!(windows) { "/a\"b" } else { "/a:b" });
        assert_eq!(path_with_first(&broken, Some(path.as_os_str())), None);
        // On Windows a folder with the separator is put in quotes.
        if cfg!(windows) {
            assert_eq!(
                path_with_first(Path::new("/a;b"), Some(path.as_os_str())),
                Some(OsString::from("\"/a;b\";/usr/local/bin;/usr/bin"))
            );
        }
    }

    /// The folder goes first on what every program started gets, and
    /// comes off again; what programs are looked for in stays without it.
    /// The only test in this crate that sets it: the crate's tests run
    /// side by side in one process.
    #[test]
    fn the_folder_put_first_reaches_every_program_started() {
        let first = std::env::temp_dir().join("bitbraid-path-first");
        let path_of = |command: &Command| {
            command
                .get_envs()
                .find(|(name, _)| *name == OsStr::new("PATH"))
                .and_then(|(_, value)| value.map(OsStr::to_os_string))
        };
        assert!(set_path_first(Some(first.clone())));
        assert!(!set_path_first(Some(first.clone())), "set twice");
        let mut command = Command::new("true");
        with_user_environment(&mut command);
        let given = path_of(&command).unwrap();
        assert_eq!(
            std::env::split_paths(&given).next().as_deref(),
            Some(first.as_path())
        );
        let listed = vars_os()
            .into_iter()
            .find(|(name, _)| name == "PATH")
            .map(|(_, value)| value)
            .unwrap();
        assert_eq!(listed, given);
        assert_eq!(var_os("PATH"), std::env::var_os("PATH"));

        assert!(set_path_first(None));
        let mut command = Command::new("true");
        with_user_environment(&mut command);
        assert_eq!(path_of(&command), None);
        assert_eq!(path_first(), None);
    }

    #[test]
    fn only_a_known_shell_by_its_full_path_is_asked() {
        let wait = Duration::from_secs(5);
        assert_eq!(login_environment(Path::new("zsh"), wait), None);
        assert_eq!(login_environment(Path::new("/bin/csh"), wait), None);
        assert_eq!(login_environment(Path::new("/no/such/zsh"), wait), None);
        assert_eq!(login_environment(Path::new(""), wait), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_shell_tells_its_environment() {
        let wait = Duration::from_secs(10);
        let vars = login_environment(Path::new("/bin/sh"), wait).unwrap();
        assert!(vars.iter().any(|(name, _)| name == "PATH"), "{vars:?}");
    }
}
