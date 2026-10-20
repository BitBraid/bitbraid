//! Git for a repository inside a WSL distribution: run there, by
//! `wsl.exe`, rather than the Windows git through the share
//! (`bitbraid_core::wsl` says why, and how what git is handed is written
//! for the distribution).
//!
//! A command for such a repository is made by [`command`] in place of the
//! usual one, and is told by its arguments from then on: [`push_args`]
//! writes git's arguments as the distribution sees them, and
//! [`carry_environment`], called last before it is started, names in
//! `WSLENV` every variable set for it, so that they cross into the
//! distribution. Only Windows makes such a command
//! (`exe::command_for`); the rest is the same everywhere, which is how it
//! is tested.
//!
//! Sources: Microsoft Learn, "Basic commands for WSL" (`--distribution`,
//! `--cd`, `--exec`) and "Share environment variables between WSL and
//! Windows" (`WSLENV`).

use std::{ffi::OsStr, process::Command};

use bitbraid_core::wsl::{Crossing, LAUNCHER, WslPath, argument, wslenv};

/// A command that runs git inside the distribution, in the folder
/// `inside`: started from the folder for temporary files, which Windows
/// can be in (a share is no working folder for every program), and
/// moved to the repository's folder by `wsl.exe` itself.
pub(super) fn command(inside: &WslPath) -> Command {
    let mut cmd = Command::new(LAUNCHER);
    cmd.current_dir(std::env::temp_dir());
    cmd.args(inside.git_invocation());
    cmd
}

/// The distribution `cmd` runs git in, when it is one [`command`] made.
fn distribution(cmd: &Command) -> Option<String> {
    if cmd.get_program() != LAUNCHER {
        return None;
    }
    WslPath::from_invocation(cmd.get_args()).map(|inside| inside.distribution)
}

/// Hand git's `args` to `cmd`: as they are, or for git inside a
/// distribution with each path of the Windows side written as the
/// distribution sees it. A message (what follows `-m` or `--message`) is
/// never taken for a path.
pub(super) fn push_args<I, S>(cmd: &mut Command, args: I)
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let Some(distribution) = distribution(cmd) else {
        cmd.args(args);
        return;
    };
    let mut message_next = false;
    for arg in args {
        let arg = arg.as_ref();
        let message = message_next
            || arg
                .to_str()
                .is_some_and(|text| text.starts_with("--message="));
        message_next = arg == "-m" || arg == "--message";
        if message {
            cmd.arg(arg);
        } else {
            cmd.arg(argument(arg, &distribution));
        }
    }
}

/// Name in `WSLENV` every variable set for `cmd`, when it runs git inside
/// a distribution: a path with `/p`, a list of paths with `/l`, anything
/// else as it is, after what `WSLENV` names already. Nothing for any other
/// command.
pub(super) fn carry_environment(cmd: &mut Command) {
    let Some(distribution) = distribution(cmd) else {
        return;
    };
    const NAME: &str = "WSLENV";
    let mut existing = std::env::var(NAME).ok();
    let mut names = Vec::new();
    for (name, value) in cmd.get_envs() {
        let (Some(name), Some(value)) = (name.to_str(), value) else {
            continue;
        };
        if name == NAME {
            existing = value.to_str().map(str::to_owned);
            continue;
        }
        let value = value.to_str().unwrap_or_default();
        names.push((name.to_owned(), Crossing::of(value, &distribution)));
    }
    let names: Vec<(&str, Crossing)> = names
        .iter()
        .map(|(name, crossing)| (name.as_str(), *crossing))
        .collect();
    cmd.env(NAME, wslenv(existing.as_deref(), &names));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    fn inside() -> WslPath {
        WslPath::parse(r"\\wsl.localhost\Ubuntu\home\me\repo").unwrap()
    }

    fn args(cmd: &Command) -> Vec<String> {
        cmd.get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn git_inside_is_handed_paths_as_the_distribution_sees_them() {
        let mut cmd = command(&inside());
        assert_eq!(cmd.get_program(), LAUNCHER);
        push_args(
            &mut cmd,
            [
                "commit",
                "-F",
                r"C:\Users\me\AppData\Local\Temp\message",
                "-m",
                r"C:\not\a\path\but\a\message",
                r"--message=D:\also\a\message",
                r"--git-dir=\\wsl$\Ubuntu\home\me\repo\.git",
                "--",
                "src/main.rs",
            ],
        );
        assert_eq!(
            args(&cmd),
            [
                "--distribution",
                "Ubuntu",
                "--cd",
                "/home/me/repo",
                "--exec",
                "git",
                "commit",
                "-F",
                "/mnt/c/Users/me/AppData/Local/Temp/message",
                "-m",
                r"C:\not\a\path\but\a\message",
                r"--message=D:\also\a\message",
                "--git-dir=/home/me/repo/.git",
                "--",
                "src/main.rs",
            ]
        );
        // What is written of it in the activity log and in errors is git
        // and its arguments, not what `wsl.exe` is told first.
        let rendered = super::super::render(&cmd);
        assert!(
            rendered.starts_with("git commit -F /mnt/c/Users/"),
            "{rendered}"
        );
        assert!(!rendered.contains("--distribution"), "{rendered}");
    }

    #[test]
    fn any_other_git_is_handed_its_arguments_as_they_are() {
        let mut cmd = Command::new("git");
        push_args(&mut cmd, ["-F", r"C:\Temp\message"]);
        assert_eq!(args(&cmd), ["-F", r"C:\Temp\message"]);
        cmd.env("GIT_ASKPASS", r"C:\x.exe");
        carry_environment(&mut cmd);
        assert!(cmd.get_envs().all(|(name, _)| name != "WSLENV"));
        // A program of that name with other arguments is not ours.
        let mut other = Command::new(LAUNCHER);
        other.args(["--list"]);
        push_args(&mut other, [r"C:\x"]);
        assert_eq!(args(&other), ["--list", r"C:\x"]);
    }

    #[test]
    fn the_environment_set_for_git_inside_crosses_into_the_distribution() {
        let mut cmd = command(&inside());
        cmd.env("GIT_TERMINAL_PROMPT", "0")
            .env("SSH_ASKPASS", r"C:\Program Files\BitBraid\bitbraid.exe")
            .env("GIT_CEILING_DIRECTORIES", "/home/me")
            .env("WSLENV", "USERPROFILE/p")
            .env_remove("GIT_DIR");
        carry_environment(&mut cmd);
        let wslenv: Option<OsString> = cmd
            .get_envs()
            .find(|(name, _)| *name == "WSLENV")
            .and_then(|(_, value)| value.map(OsStr::to_os_string));
        let wslenv = wslenv.unwrap().into_string().unwrap();
        let mut entries: Vec<&str> = wslenv.split(':').collect();
        assert_eq!(entries.remove(0), "USERPROFILE/p", "what was there first");
        entries.sort_unstable();
        assert_eq!(
            entries,
            [
                "GIT_CEILING_DIRECTORIES",
                "GIT_TERMINAL_PROMPT",
                "SSH_ASKPASS/p"
            ]
        );
    }
}
