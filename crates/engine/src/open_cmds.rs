//! The repository's folder in other programs: the file manager, an editor,
//! a terminal.
//!
//! Nothing here goes through a shell: a command is cut into words at its
//! blanks, with quotes as a shell has them ([`crate::command_words`]),
//! `{path}` in a word is filled in with the folder, and the folder stays
//! one argument whatever characters it has. The program is started in the
//! folder and left running.
//!
//! Which terminal there is, is found by looking for the programs people
//! use on `PATH`: `$TERMINAL` first when it names one, then
//! `xdg-terminal-exec`, then a list of emulators, each with the option
//! that tells it where to start (most start where they are started, a few
//! hand the window to a server that does not). On macOS terminals are
//! applications, found by their bundles in the folders applications are
//! kept in, and started with `open`: by the folder, which those that open
//! one in a new window take, or with the option that tells them where.
//!
//! Sources: the freedesktop `xdg-open` manual and the xdg-terminal-exec
//! proposal; macOS open(1) (`-a`, `-n`, `--args`) and the folders of
//! applications (`/Applications`, `~/Applications`, the system's
//! Utilities); Windows Terminal's command-line documentation
//! (`wt -d`); the manuals of kitty, alacritty, foot, wezterm, ghostty,
//! gnome-terminal, kgx, ptyxis, konsole, xfce4-terminal, tilix and
//! terminator for their working-directory option.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param_or},
    file_cmds::{launch, system_opener},
};

/// Register the commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "repo.open_folder",
            label: say!("open-folder"),
            description: say!("open-folder-description"),
        },
        open_folder,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.open_editor",
            label: say!("open-editor"),
            description: say!("open-editor-description"),
        },
        open_editor,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.open_terminal",
            label: say!("open-terminal"),
            description: say!("open-terminal-description"),
        },
        open_terminal,
    );
    let _ = registry.register(
        CommandSpec {
            id: "repo.terminals",
            label: say!("open-terminals"),
            description: say!("open-terminals-description"),
        },
        terminals,
    );
}

fn invalid(name: &str, detail: impl Into<String>) -> CommandError {
    CommandError::InvalidParam {
        name: name.to_owned(),
        detail: detail.into(),
    }
}

/// The folder a command is about: the worktree's top, a folder inside it
/// given relative to it, or any folder given by its full path. It has to
/// be there.
fn folder(ctx: &CommandContext, params: &Params) -> Result<PathBuf, CommandError> {
    let given: Option<String> = param_or(params, "path", None)?;
    let folder = match given
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())
    {
        None => {
            let repo = ctx.repo()?;
            repo.workdir()
                .unwrap_or_else(|| repo.git_dir())
                .to_path_buf()
        }
        Some(path) if path.chars().any(char::is_control) || path.len() > 4096 => {
            return Err(invalid("path", say!("open-not-a-path")));
        }
        Some(path) if Path::new(path).is_absolute() => PathBuf::from(path),
        Some(path) => bitbraid_git::worktree_path(ctx.repo()?, path)?,
    };
    if !folder.is_dir() {
        return Err(invalid(
            "path",
            say!("open-not-a-folder", path = folder.display().to_string()),
        ));
    }
    Ok(folder)
}

/// The words of a command with `{path}` filled in with the folder, split
/// and unquoted as [`crate::command_words`] says. No shell is involved, so
/// nothing in a path can turn into syntax. The second value says whether
/// the command placed the folder itself.
fn words_with(command: &str, folder: &Path) -> Result<(Vec<OsString>, bool), CommandError> {
    use crate::command_words::{PATH, fill};
    let filled = fill(command, &[(PATH, folder.as_os_str())])?;
    let placed = filled.placed.contains(PATH);
    Ok((filled.words, placed))
}

/// A command the caller handed in, when it handed one in.
fn command_of(params: &Params) -> Result<Option<String>, CommandError> {
    let command: Option<String> = param_or(params, "command", None)?;
    let command = command
        .map(|command| command.trim().to_owned())
        .filter(|command| !command.is_empty());
    if let Some(command) = &command
        && (command.chars().any(char::is_control) || command.len() > 4096)
    {
        return Err(invalid("command", say!("open-not-a-command")));
    }
    Ok(command)
}

/// Start a program in a folder and leave it running.
fn start_in(folder: &Path, words: Vec<OsString>) -> Result<String, CommandError> {
    let mut words = words.into_iter();
    let program = words
        .next()
        .ok_or_else(|| invalid("command", say!("open-empty")))?;
    let args: Vec<OsString> = words.collect();
    // Where it runs is the folder: most programs start where they are
    // started. The directory of this process is not touched.
    launch_in(folder, &program, &args)?;
    Ok(program.to_string_lossy().into_owned())
}

/// [`launch`], in a directory.
fn launch_in(folder: &Path, program: &OsString, args: &[OsString]) -> Result<(), CommandError> {
    use std::process::Stdio;
    let mut child = crate::file_cmds::command_for(program, args)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            invalid(
                "command",
                say!(
                    "open-cannot-start",
                    program = program.to_string_lossy(),
                    error = err.to_string()
                ),
            )
        })?;
    let _ = std::thread::Builder::new()
        .name("bitbraid-child".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

fn open_folder(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let folder = folder(ctx, params)?;
    let started = match command_of(params)? {
        Some(command) => {
            let (mut words, placed) = words_with(&command, &folder)?;
            if !placed {
                words.push(folder.as_os_str().to_owned());
            }
            start_in(&folder, words)?
        }
        None => {
            let (program, args) = system_opener(&folder);
            launch(&program, &args)?;
            program.to_string_lossy().into_owned()
        }
    };
    Ok(json!({ "opened": folder.display().to_string(), "with": started }))
}

fn open_editor(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let folder = folder(ctx, params)?;
    let command = command_of(params)?.ok_or_else(|| invalid("command", say!("open-no-editor")))?;
    // An editor command is written for a file (`code --goto
    // {path}:{line}`); for a folder it is its program and the folder.
    let program = crate::command_words::split(&command)?
        .into_iter()
        .next()
        .map(OsString::from)
        .ok_or_else(|| invalid("command", say!("open-empty")))?;
    let started = start_in(&folder, vec![program, folder.as_os_str().to_owned()])?;
    Ok(json!({ "opened": folder.display().to_string(), "with": started }))
}

/// A terminal program and how it is told where to start.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Terminal {
    /// The program's name.
    pub name: String,
    /// The command it is started by, with `{path}` where the folder goes
    /// when it has to be told.
    pub command: String,
}

/// The emulators that are looked for, in this order, each with the words
/// that tell it where to start. One without words starts where it is
/// started.
const KNOWN: &[(&str, &str)] = &[
    ("xdg-terminal-exec", ""),
    ("kitty", "--directory {path}"),
    ("alacritty", "--working-directory {path}"),
    ("foot", "--working-directory={path}"),
    ("wezterm", "start --cwd {path}"),
    ("ghostty", "--working-directory={path}"),
    ("ptyxis", "--new-window -d {path}"),
    ("kgx", "--working-directory={path}"),
    ("gnome-terminal", "--working-directory={path}"),
    ("konsole", "--workdir {path}"),
    ("xfce4-terminal", "--working-directory={path}"),
    ("tilix", "--working-directory={path}"),
    ("terminator", "--working-directory={path}"),
    ("mate-terminal", "--working-directory={path}"),
    ("lxterminal", "--working-directory={path}"),
    ("x-terminal-emulator", ""),
    ("urxvt", ""),
    ("st", ""),
    ("xterm", ""),
];

/// The terminals of macOS that are looked for, in this order: the name of
/// the application's bundle, and how it is started in a folder. The
/// system's own comes first.
const KNOWN_ON_MAC: &[(&str, &str)] = &[
    ("Terminal", "open -a Terminal {path}"),
    ("iTerm", "open -a iTerm {path}"),
    ("Ghostty", "open -a Ghostty {path}"),
    ("WezTerm", "open -na WezTerm --args start --cwd {path}"),
    ("kitty", "open -na kitty --args --directory {path}"),
    (
        "Alacritty",
        "open -na Alacritty --args --working-directory {path}",
    ),
    ("Warp", "open -a Warp {path}"),
];

/// The folders macOS keeps applications in: the system's, everyone's and
/// the user's own.
fn mac_application_folders() -> Vec<PathBuf> {
    let mut folders = vec![
        PathBuf::from("/System/Applications/Utilities"),
        PathBuf::from("/Applications"),
        PathBuf::from("/Applications/Utilities"),
    ];
    if let Some(home) = bitbraid_core::process::var_os("HOME").filter(|home| !home.is_empty()) {
        folders.push(PathBuf::from(home).join("Applications"));
    }
    folders
}

/// The terminals of macOS there are in these folders of applications.
pub fn mac_terminals_in(folders: &[PathBuf]) -> Vec<Terminal> {
    KNOWN_ON_MAC
        .iter()
        .filter(|(name, _)| {
            folders
                .iter()
                .any(|folder| folder.join(format!("{name}.app")).is_dir())
        })
        .map(|(name, command)| Terminal {
            name: (*name).to_owned(),
            command: (*command).to_owned(),
        })
        .collect()
}

fn command_for(name: &str, words: &str) -> String {
    if words.is_empty() {
        name.to_owned()
    } else {
        format!("{name} {words}")
    }
}

/// The terminals there are among the directories of `path`, the one named
/// by `preferred` (what `$TERMINAL` says) first when it is there.
pub fn terminals_in(path: Option<&OsStr>, preferred: Option<&str>) -> Vec<Terminal> {
    let found = |name: &str| -> bool {
        let Some(path) = path else {
            return false;
        };
        std::env::split_paths(path)
            .filter(|dir| !dir.as_os_str().is_empty())
            .any(|dir| dir.join(name).is_file())
    };
    let mut out: Vec<Terminal> = Vec::new();
    // What the user's environment names is one word, a program; anything
    // else in it is not for here to interpret.
    if let Some(name) = preferred.map(str::trim).filter(|name| {
        !name.is_empty()
            && name.len() <= 200
            && !name.contains(char::is_whitespace)
            && !name.chars().any(char::is_control)
    }) {
        let as_path = Path::new(name);
        let there = if as_path.is_absolute() {
            as_path.is_file()
        } else {
            !name.contains('/') && found(name)
        };
        if there {
            let base = as_path
                .file_name()
                .map(|base| base.to_string_lossy().into_owned())
                .unwrap_or_default();
            let words = KNOWN
                .iter()
                .find(|(known, _)| *known == base)
                .map(|(_, words)| *words)
                .unwrap_or("");
            out.push(Terminal {
                name: base,
                command: command_for(name, words),
            });
        }
    }
    for (name, words) in KNOWN {
        if found(name) && !out.iter().any(|terminal| terminal.name == *name) {
            out.push(Terminal {
                name: (*name).to_owned(),
                command: command_for(name, words),
            });
        }
    }
    out
}

/// The terminals of Windows that are there, on `path`: Windows Terminal,
/// the Git Bash of the Git for Windows whose git is on it, PowerShell and
/// the command prompt, which is always there. The consoles are started
/// through `start`, which gives each a window of its own in the folder.
fn windows_terminals(path: Option<&str>) -> Vec<Terminal> {
    use bitbraid_terminal::shell::find_program;
    let mut out = Vec::new();
    if find_program("wt", path).is_some() {
        out.push(Terminal {
            name: "Windows Terminal".to_owned(),
            command: "wt -d {path}".to_owned(),
        });
    }
    let git_bash = find_program("git", path)
        .and_then(|git| Some(git.parent()?.parent()?.join("git-bash.exe")))
        .filter(|bash| bash.is_file());
    if let Some(bash) = git_bash {
        out.push(Terminal {
            name: "Git Bash".to_owned(),
            command: format!("\"{}\" --cd={{path}}", bash.display()),
        });
    }
    if let Some(shell) = ["pwsh", "powershell"]
        .into_iter()
        .find(|shell| find_program(shell, path).is_some())
    {
        out.push(Terminal {
            name: "PowerShell".to_owned(),
            command: format!("cmd /C start {shell} -NoLogo"),
        });
    }
    out.push(Terminal {
        name: "Command Prompt".to_owned(),
        command: "cmd /C start cmd".to_owned(),
    });
    out
}

/// The terminals found on this system.
fn found_terminals() -> Vec<Terminal> {
    if cfg!(target_os = "macos") {
        let found = mac_terminals_in(&mac_application_folders());
        if !found.is_empty() {
            return found;
        }
        // The system's own is there even where it was not found.
        return KNOWN_ON_MAC
            .iter()
            .take(1)
            .map(|(name, command)| Terminal {
                name: (*name).to_owned(),
                command: (*command).to_owned(),
            })
            .collect();
    }
    if cfg!(target_os = "windows") {
        return windows_terminals(std::env::var("PATH").ok().as_deref());
    }
    let preferred = bitbraid_core::process::var_os("TERMINAL").and_then(|t| t.into_string().ok());
    terminals_in(
        bitbraid_core::process::var_os("PATH").as_deref(),
        preferred.as_deref(),
    )
}

fn terminals(_ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    let found = found_terminals();
    Ok(json!({
        "default": found.first(),
        "terminals": found,
    }))
}

fn open_terminal(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let folder = folder(ctx, params)?;
    let command = match command_of(params)? {
        Some(command) => command,
        None => found_terminals()
            .into_iter()
            .next()
            .map(|terminal| terminal.command)
            .ok_or_else(|| invalid("command", say!("open-no-terminal")))?,
    };
    let (words, _) = words_with(&command, &folder)?;
    let started = start_in(&folder, words)?;
    Ok(json!({ "opened": folder.display().to_string(), "with": started }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_terminals_of_macos_are_the_applications_there_are() {
        let root = std::env::temp_dir().join(format!("bb-mac-terminals-{}", std::process::id()));
        let (system, everyone, own) = (root.join("system"), root.join("all"), root.join("own"));
        for (folder, app) in [
            (&system, "Terminal.app"),
            (&everyone, "kitty.app"),
            (&everyone, "Unknown.app"),
            (&own, "iTerm.app"),
        ] {
            std::fs::create_dir_all(folder.join(app)).unwrap();
        }
        // A file of that name is no application.
        std::fs::write(own.join("Ghostty.app"), "").unwrap();
        let found = mac_terminals_in(&[system.clone(), everyone.clone(), own.clone()]);
        let names: Vec<&str> = found.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["Terminal", "iTerm", "kitty"]);
        assert_eq!(found[2].command, "open -na kitty --args --directory {path}");
        assert!(mac_terminals_in(&[]).is_empty());
        let _ = std::fs::remove_dir_all(&root);
    }

    fn shown(words: &[OsString]) -> Vec<String> {
        words
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    fn split(command: &str, folder: &Path) -> (Vec<OsString>, bool) {
        words_with(command, folder).unwrap_or_else(|err| panic!("{command:?}: {err}"))
    }

    #[test]
    fn the_folder_is_filled_in_as_one_word() {
        let folder = Path::new("/work/my project");
        let (words, placed) = split("kitty --directory {path}", folder);
        assert_eq!(shown(&words), ["kitty", "--directory", "/work/my project"]);
        assert!(placed);
        let (words, placed) = split("foot --working-directory={path}", folder);
        assert_eq!(
            shown(&words),
            ["foot", "--working-directory=/work/my project"]
        );
        assert!(placed);
        let (words, placed) = split("  xterm  ", folder);
        assert_eq!(shown(&words), ["xterm"]);
        assert!(!placed);
        // Nothing in a path turns into syntax.
        let hostile = Path::new("/work/$(rm -rf ~); `x` | y && z {path}");
        let (words, _) = split("term --cwd {path}", hostile);
        assert_eq!(words.len(), 3);
        assert_eq!(words[2], hostile.as_os_str());
        assert!(split("", folder).0.is_empty());
        // Quoted as for a shell, without the quotes.
        let (words, placed) = split("\"/opt/my term\" --cwd \"{path}\" -e 'a b'", folder);
        assert_eq!(
            shown(&words),
            ["/opt/my term", "--cwd", "/work/my project", "-e", "a b"]
        );
        assert!(placed);
        assert!(words_with("term --cwd '{path}", folder).is_err());
    }

    #[test]
    fn terminals_are_found_in_order_and_the_environments_choice_comes_first() {
        let root = std::env::temp_dir().join(format!("bitbraid-terminals-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (one, two) = (root.join("one"), root.join("two"));
        std::fs::create_dir_all(&one).unwrap();
        std::fs::create_dir_all(&two).unwrap();
        for (dir, name) in [
            (&two, "xterm"),
            (&one, "konsole"),
            (&two, "kitty"),
            (&two, "myterm"),
        ] {
            std::fs::write(dir.join(name), "").unwrap();
        }
        std::fs::create_dir(one.join("alacritty")).unwrap();
        let path = std::env::join_paths([&one, &two]).unwrap();

        let found = terminals_in(Some(&path), None);
        let names: Vec<&str> = found
            .iter()
            .map(|terminal| terminal.name.as_str())
            .collect();
        // In the order of the list, not of PATH; a folder of that name is
        // no program.
        assert_eq!(names, ["kitty", "konsole", "xterm"]);
        assert_eq!(found[0].command, "kitty --directory {path}");
        assert_eq!(found[2].command, "xterm");

        // What $TERMINAL names goes first, with its option when it is known.
        let found = terminals_in(Some(&path), Some("konsole"));
        assert_eq!(found[0].command, "konsole --workdir {path}");
        assert_eq!(found.len(), 3, "and is not listed twice");
        let found = terminals_in(Some(&path), Some("myterm"));
        assert_eq!(
            (found[0].name.as_str(), found[0].command.as_str()),
            ("myterm", "myterm")
        );
        let full = two.join("kitty").display().to_string();
        let found = terminals_in(Some(&path), Some(&full));
        assert_eq!(found[0].command, format!("{full} --directory {{path}}"));

        // Nonsense in $TERMINAL is not run.
        for odd in [
            "",
            "  ",
            "absent",
            "kitty --hold",
            "sub/kitty",
            "a\u{7}b",
            "/not/there",
        ] {
            let found = terminals_in(Some(&path), Some(odd));
            assert_eq!(found[0].name, "kitty", "{odd:?}");
            assert_eq!(found.len(), 3, "{odd:?}");
        }
        assert!(terminals_in(None, Some("kitty")).is_empty());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
