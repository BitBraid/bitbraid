//! The embedded terminal's two questions: which shell a terminal starts,
//! where and with which environment, and how dropped paths are written
//! for that shell. The session itself is the window's (`AGENTS.md`
//! section 2); these only answer.
//!
//! A terminal opens in the open repository's worktree, in another
//! worktree of the same repository, or in a folder inside one of them,
//! and nowhere else. Its shell is the one the user chose, else `$SHELL`,
//! else `/bin/sh` (on Windows PowerShell, else the command prompt). On
//! macOS it is started as a login shell unless it is given arguments, as
//! terminals there start it: the system's `path_helper` and the user's
//! `.zprofile` (where Homebrew is put on the `PATH`) are read. Its
//! environment is this process's, without what would tie the git it runs
//! to a repository (`GIT_DIR`, `GIT_WORK_TREE` and the like, which a git
//! that started this process may have set), without what tells programs
//! about the terminal this process was started from (`TMUX`, `WINDOWID`,
//! `COLUMNS`), without what the application keeps for itself (the askpass
//! socket, startup tokens, its test switches), and with `TERM`,
//! `COLORTERM`, `TERM_PROGRAM` and `TERM_PROGRAM_VERSION` of its own;
//! while the git in use is the one shipped with the application, its
//! folder stands first on `PATH`.
//!
//! Sources: git(1), "ENVIRONMENT VARIABLES" (the repository locations,
//! `GIT_CONFIG_PARAMETERS` and `GIT_CONFIG_COUNT`); git-worktree(1);
//! environ(7) (`SHELL`, `TERM`, `COLUMNS`, `LINES`); the freedesktop
//! startup-notification and xdg-activation specifications
//! (`DESKTOP_STARTUP_ID`, `XDG_ACTIVATION_TOKEN`); the `TERM_PROGRAM`
//! convention of terminal emulators; `bitbraid_terminal::quote` for the
//! quoting.

use std::path::{Path, PathBuf};

use bitbraid_terminal::{
    ShellKind, quote_paths,
    shell::{find_program, same_variable},
};
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "terminal.shell",
            label: say!("terminal-shell"),
            description: say!("terminal-shell-description"),
        },
        shell,
    );
    let _ = registry.register(
        CommandSpec {
            id: "terminal.paste_paths",
            label: say!("terminal-paste-paths"),
            description: say!("terminal-paste-paths-description"),
        },
        paste_paths,
    );
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// The longest path, program or argument taken.
const LONGEST: usize = 4096;
/// The most arguments a shell is given, and the most paths pasted at once.
const MOST: usize = 10_000;

/// Variables that tie git to a repository, or carry configuration a git
/// that started this process passed to its children. A shell opened in a
/// worktree finds its repository by its folder, as one started anywhere
/// else does.
const GIT_BINDING: &[&str] = &[
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_PREFIX",
    "GIT_NAMESPACE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_SHALLOW_FILE",
    "GIT_REPLACE_REF_BASE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_QUARANTINE_PATH",
    "GIT_INTERNAL_SUPER_PREFIX",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_REFLOG_ACTION",
];

/// Prefixes of git's numbered configuration pairs, which go with
/// `GIT_CONFIG_COUNT`.
const GIT_BINDING_PREFIXES: &[&str] = &["GIT_CONFIG_KEY_", "GIT_CONFIG_VALUE_"];

/// Variables that describe the terminal this process was started from,
/// or this process's own startup: wrong in a terminal of its own.
const NOT_THIS_TERMINAL: &[&str] = &[
    "COLUMNS",
    "LINES",
    "WINDOWID",
    "TERM_SESSION_ID",
    "VTE_VERSION",
    "TMUX",
    "TMUX_PANE",
    "STY",
    "WINDOW",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "DESKTOP_STARTUP_ID",
    "XDG_ACTIVATION_TOKEN",
];

/// Prefixes of what terminal emulators and multiplexers tell their
/// children about themselves.
const NOT_THIS_TERMINAL_PREFIXES: &[&str] = &[
    "ALACRITTY_",
    "GHOSTTY_",
    "ITERM_",
    "KITTY_",
    "KONSOLE_",
    "WEZTERM_",
    "ZELLIJ",
];

/// What the application keeps for itself: the askpass helper's socket
/// and token, and `BITBRAID_NEW_INSTANCE`.
const APPLICATIONS_OWN: &[&str] = &[
    bitbraid_git::askpass::SOCKET_ENV,
    bitbraid_git::askpass::TOKEN_ENV,
    "BITBRAID_NEW_INSTANCE",
];

/// Prefixes of the application's test and benchmark switches.
const APPLICATIONS_OWN_PREFIXES: &[&str] = &["BITBRAID_BENCH", "BITBRAID_TEST_"];

/// Whether a variable of this process is left out of a terminal's. On
/// Windows the case of a name does not count.
pub fn left_out(name: &str) -> bool {
    let upper;
    let name = if cfg!(windows) {
        upper = name.to_ascii_uppercase();
        upper.as_str()
    } else {
        name
    };
    GIT_BINDING.contains(&name)
        || NOT_THIS_TERMINAL.contains(&name)
        || APPLICATIONS_OWN.contains(&name)
        || GIT_BINDING_PREFIXES
            .iter()
            .chain(NOT_THIS_TERMINAL_PREFIXES)
            .chain(APPLICATIONS_OWN_PREFIXES)
            .any(|prefix| name.starts_with(prefix))
        || name.starts_with("TERM_PROGRAM")
}

/// The environment of a terminal, from the variables of this process: the
/// ones that are text, without what is left out, with the terminal's own.
pub fn environment(
    ours: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
) -> Vec<(String, String)> {
    let mut env: std::collections::BTreeMap<String, String> = ours
        .into_iter()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, value)| {
            !name.is_empty()
                && !name.contains(['=', '\0'])
                && !value.contains('\0')
                && !left_out(name)
        })
        .collect();
    for (name, value) in [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "bitbraid"),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
    ] {
        env.insert(name.to_owned(), value.to_owned());
    }
    env.into_iter().collect()
}

/// The shell to start: the one chosen, else `$SHELL`, else the system's.
fn program_of(params: &Params, env: &[(String, String)]) -> Result<String, CommandError> {
    let path = env
        .iter()
        .find(|(name, _)| same_variable(name, "PATH"))
        .map(|(_, value)| value.as_str());
    let chosen: Option<String> = param_or(params, "program", None)?;
    if let Some(chosen) = chosen
        .map(|chosen| chosen.trim().to_owned())
        .filter(|chosen| !chosen.is_empty())
    {
        if chosen.len() > LONGEST || chosen.chars().any(char::is_control) {
            return Err(invalid("program", say!("terminal-not-a-program")));
        }
        if find_program(&chosen, path).is_none() {
            return Err(invalid(
                "program",
                say!("terminal-program-not-found", program = chosen.as_str()),
            ));
        }
        return Ok(chosen);
    }
    let from_environment = env
        .iter()
        .find(|(name, _)| same_variable(name, "SHELL"))
        .map(|(_, value)| value.trim())
        .filter(|shell| {
            !shell.is_empty()
                && shell.len() <= LONGEST
                && !shell.chars().any(char::is_control)
                && find_program(shell, path).is_some()
        });
    if let Some(shell) = from_environment {
        return Ok(shell.to_owned());
    }
    if cfg!(windows) {
        for candidate in ["pwsh.exe", "powershell.exe"] {
            if find_program(candidate, path).is_some() {
                return Ok(candidate.to_owned());
            }
        }
        // The command prompt, where the system says it is.
        let comspec = env
            .iter()
            .find(|(name, _)| same_variable(name, "COMSPEC"))
            .map(|(_, value)| value.trim())
            .filter(|comspec| find_program(comspec, path).is_some());
        return Ok(comspec.unwrap_or("cmd.exe").to_owned());
    }
    Ok("/bin/sh".to_owned())
}

fn args_of(params: &Params) -> Result<Vec<String>, CommandError> {
    let args: Vec<String> = param_or(params, "args", Vec::new())?;
    if args.len() > MOST {
        return Err(invalid("args", say!("terminal-too-many-arguments")));
    }
    if args
        .iter()
        .any(|arg| arg.len() > LONGEST || arg.contains('\0'))
    {
        return Err(invalid("args", say!("terminal-argument-not-passed")));
    }
    Ok(args)
}

/// The folders a terminal of the open repository may open in: the tops of
/// its worktrees that are there (the repository itself when it is bare),
/// each as the system names it.
fn roots(ctx: &CommandContext) -> Result<Vec<PathBuf>, CommandError> {
    let repo = ctx.repo()?;
    let mut roots: Vec<PathBuf> = Vec::new();
    let own = repo.workdir().unwrap_or_else(|| repo.git_dir());
    roots.extend(bitbraid_core::paths::canonical(own).ok());
    // Another worktree is as good as this one; a listing that fails leaves
    // this one.
    if let Ok(listed) = bitbraid_git::worktree::list(repo, false) {
        for info in listed {
            if info.prunable {
                continue;
            }
            if let Ok(path) = bitbraid_core::paths::canonical(&info.path)
                && path.is_dir()
                && !roots.contains(&path)
            {
                roots.push(path);
            }
        }
    }
    Ok(roots)
}

/// The folder a terminal opens in, and the worktree it is in.
fn folder(ctx: &CommandContext, params: &Params) -> Result<(PathBuf, PathBuf), CommandError> {
    let repo = ctx.repo()?;
    let top = repo.workdir().unwrap_or_else(|| repo.git_dir());
    let given: Option<String> = param_or(params, "path", None)?;
    let wanted = match given
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        None => top.to_path_buf(),
        Some(path) if path.len() > LONGEST || path.chars().any(char::is_control) => {
            return Err(invalid("path", say!("terminal-not-a-path")));
        }
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => top.join(path),
    };
    let real = bitbraid_core::paths::canonical(&wanted).map_err(|_| {
        invalid(
            "path",
            say!(
                "terminal-folder-missing",
                path = wanted.display().to_string()
            ),
        )
    })?;
    if !real.is_dir() {
        return Err(invalid(
            "path",
            say!("terminal-not-a-folder", path = wanted.display().to_string()),
        ));
    }
    // The innermost worktree it is in: one may be inside another.
    let root = roots(ctx)?
        .into_iter()
        .filter(|root| real.starts_with(root))
        .max_by_key(|root| root.components().count())
        .ok_or_else(|| {
            invalid(
                "path",
                say!(
                    "terminal-not-in-a-worktree",
                    path = wanted.display().to_string()
                ),
            )
        })?;
    Ok((real, root))
}

fn shell(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let (cwd, worktree) = folder(ctx, params)?;
    // Which git is in use is settled first: while it is the one shipped
    // with the application, the shell finds it first on its `PATH`.
    let _ = bitbraid_git::exe::git_binary();
    let env = environment(bitbraid_core::process::vars_os());
    let program = program_of(params, &env)?;
    let kind = ShellKind::of_program(&program);
    let args = match args_of(params)? {
        args if args.is_empty() && cfg!(target_os = "macos") && kind.takes_login() => {
            vec!["-l".to_owned()]
        }
        // On Windows PowerShell keeps its working folder where it is, so
        // that the terminal can follow it (`Session::current_folder`).
        args if args.is_empty() && cfg!(windows) && kind == ShellKind::PowerShell => {
            bitbraid_terminal::shell::powershell_following()
        }
        args => args,
    };
    let env: serde_json::Map<String, serde_json::Value> = env
        .into_iter()
        .map(|(name, value)| (name, serde_json::Value::String(value)))
        .collect();
    Ok(json!({
        "program": program,
        "args": args,
        "cwd": cwd.to_string_lossy(),
        "worktree": worktree.to_string_lossy(),
        "env": env,
        "kind": kind.name(),
    }))
}

fn paste_paths(_ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let paths: Vec<String> = param(params, "paths")?;
    if paths.len() > MOST {
        return Err(invalid("paths", say!("terminal-too-many-paths")));
    }
    if paths
        .iter()
        .any(|path| path.is_empty() || path.len() > LONGEST || path.contains('\0'))
    {
        return Err(invalid("paths", say!("terminal-one-not-a-path")));
    }
    let shell: Option<String> = param_or(params, "shell", None)?;
    let kind = match shell.as_deref().map(str::trim) {
        None | Some("") => ShellKind::Posix,
        Some(shell) if shell.len() > LONGEST || shell.chars().any(char::is_control) => {
            return Err(invalid("shell", say!("terminal-not-a-shell")));
        }
        Some(shell) => ShellKind::from_name(shell).unwrap_or_else(|| ShellKind::of_program(shell)),
    };
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    Ok(json!({
        "text": quote_paths(&paths, kind),
        "kind": kind.name(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn what_is_left_out_of_the_environment() {
        let ours = [
            ("PATH", "/usr/bin"),
            ("HOME", "/home/me"),
            ("SHELL", "/bin/zsh"),
            ("GIT_DIR", "/elsewhere/.git"),
            ("GIT_WORK_TREE", "/elsewhere"),
            ("GIT_CONFIG_COUNT", "1"),
            ("GIT_CONFIG_KEY_0", "a.b"),
            ("GIT_AUTHOR_NAME", "Me"),
            ("GIT_SSH_COMMAND", "ssh -i key"),
            ("TMUX", "/tmp/tmux"),
            ("COLUMNS", "80"),
            ("KITTY_WINDOW_ID", "1"),
            ("TERM", "screen"),
            ("TERM_PROGRAM", "tmux"),
            ("TERM_PROGRAM_VERSION", "3.4"),
            ("BITBRAID_ASKPASS_SOCKET", "/run/x"),
            ("BITBRAID_BENCH_REPO", "/x"),
            ("BITBRAID_CONFIG_DIR", "/cfg"),
            ("XDG_ACTIVATION_TOKEN", "t"),
        ]
        .map(|(name, value)| (name.into(), value.into()));
        let env = environment(ours);
        let names: Vec<&str> = env.iter().map(|(name, _)| name.as_str()).collect();
        assert_eq!(
            names,
            [
                "BITBRAID_CONFIG_DIR",
                "COLORTERM",
                "GIT_AUTHOR_NAME",
                "GIT_SSH_COMMAND",
                "HOME",
                "PATH",
                "SHELL",
                "TERM",
                "TERM_PROGRAM",
                "TERM_PROGRAM_VERSION"
            ]
        );
        let value = |name: &str| {
            env.iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        };
        assert_eq!(value("TERM"), Some("xterm-256color"));
        assert_eq!(value("COLORTERM"), Some("truecolor"));
        assert_eq!(value("TERM_PROGRAM"), Some("bitbraid"));
        assert_eq!(
            value("TERM_PROGRAM_VERSION"),
            Some(env!("CARGO_PKG_VERSION"))
        );
    }

    #[cfg(unix)]
    #[test]
    fn variables_that_are_not_text_are_left_out() {
        use std::os::unix::ffi::OsStringExt as _;
        let ours = [
            (
                std::ffi::OsString::from_vec(b"BAD\xff".to_vec()),
                "x".into(),
            ),
            (
                "ALSO_BAD".into(),
                std::ffi::OsString::from_vec(b"\xfe".to_vec()),
            ),
            ("GOOD".into(), "y".into()),
        ];
        let env = environment(ours);
        assert!(env.iter().any(|(name, _)| name == "GOOD"));
        assert!(!env.iter().any(|(name, _)| name.contains("BAD")));
    }
}
