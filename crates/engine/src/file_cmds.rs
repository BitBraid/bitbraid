//! Commands on single files: open one in an editor, as it is in the
//! worktree or as it was in a commit, show it in the file manager, put it
//! back to what it was in a commit, read its bytes on any side of a diff
//! (for images), and write a quick fix made in the application.
//!
//! A file as it was in a commit is opened from a copy: the content is
//! written to a folder of the user's own under the system's temporary
//! directory, read-only, under the file's own name (so that an editor
//! knows what kind of file it is), in a folder named by the commit.
//!
//! Sources: git-restore(1); git-hash-object(1); the freedesktop `xdg-open`
//! manual, macOS open(1) and Windows `explorer` documentation for the
//! system openers; RFC 4648 (base64).

use std::{
    ffi::{OsStr, OsString},
    path::Path,
    process::{Command, Stdio},
};

use bitbraid_git::DiffSide;
use serde_json::json;

use crate::{
    Registry,
    command::{CommandContext, CommandError, CommandSpec, Output, Params, param, param_or},
    journal::Change,
};

/// Register the file commands.
pub fn register(registry: &mut Registry) {
    let _ = registry.register(
        CommandSpec {
            id: "file.open",
            label: say!("file-open"),
            description: say!("file-open-description"),
        },
        open,
    );
    let _ = registry.register(
        CommandSpec {
            id: "file.reveal",
            label: say!("file-reveal"),
            description: say!("file-reveal-description"),
        },
        reveal,
    );
    let _ = registry.register(
        CommandSpec {
            id: "file.restore",
            label: say!("file-restore"),
            description: say!("file-restore-description"),
        },
        restore,
    );
    let _ = registry.register(
        CommandSpec {
            id: "file.bytes",
            label: say!("file-bytes"),
            description: say!("file-bytes-description"),
        },
        bytes,
    );
    let _ = registry.register(
        CommandSpec {
            id: "file.write",
            label: say!("file-write"),
            description: say!("file-write-description"),
        },
        write,
    );
}

/// Largest file `file.bytes` hands out.
const MAX_BYTES: usize = 32 * 1024 * 1024;

fn bytes(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    use base64::Engine as _;
    let path: String = param(params, "path")?;
    let side_text: String = param_or(params, "side", "worktree".to_owned())?;
    // What the caller wants at most, when that is less than what is
    // handed out at all.
    let max: usize = param_or(params, "max", MAX_BYTES)?;
    let repo = ctx.repo()?;
    let side = DiffSide::parse(repo, &side_text)?;
    let found = bitbraid_git::file_bytes(repo, &side, &path, max.min(MAX_BYTES))?;
    Ok(match found {
        Some(content) => json!({
            "exists": true,
            "size": content.len(),
            "id": bitbraid_git::stage::blob_id(&content),
            "base64": base64::engine::general_purpose::STANDARD.encode(&content),
        }),
        None => json!({ "exists": false }),
    })
}

fn write(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let text: String = param(params, "text")?;
    let base: Option<bitbraid_git::Oid> = param_or(params, "base", None)?;
    let (before, after) = bitbraid_git::stage::write_text(ctx.repo()?, &path, &text, base)?;
    let id = after.blob;
    if before.blob != after.blob {
        ctx.record(
            say!("file-entry-write", path = path.as_str()),
            Change::FileWrite { before, after },
        )?;
    }
    Ok(json!({ "written": path, "id": id }))
}

/// Start a program and leave it running; a thread collects it when it ends.
pub(crate) fn launch(program: &OsString, args: &[OsString]) -> Result<(), CommandError> {
    let mut child = command_for(program, args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| CommandError::InvalidParam {
            name: "command".to_owned(),
            detail: say!(
                "file-cannot-start",
                program = program.to_string_lossy(),
                error = err.to_string()
            ),
        })?;
    // Without a thread to wait for it, the child is left to the system.
    let _ = std::thread::Builder::new()
        .name("bitbraid-child".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
    Ok(())
}

/// What starts a program of the user's (an editor, a file manager, a
/// terminal) with its arguments, before where it runs and what it reads
/// are said.
pub(crate) fn command_for(program: &OsStr, args: &[OsString]) -> Command {
    let program = resolved(program);
    // Which git is in use is settled first: while it is the one shipped
    // with the application, the program finds it first on its `PATH`.
    let _ = bitbraid_git::exe::git_binary();
    let mut command = Command::new(&program);
    bitbraid_core::process::with_user_environment(&mut command);
    add_args(&mut command, &program, args);
    // The command prompt that runs a `start` of another program needs no
    // window of its own: `start` gives the program one. Without a console
    // to read it would end at once all the same.
    let prompt = Path::new(&program)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("cmd"));
    if cfg!(windows) && prompt {
        bitbraid_core::process::no_window(&mut command);
    }
    command
}

/// The program to start for a name a person wrote. On Windows it is looked
/// for with the endings of `PATHEXT` (`code` is code.cmd), which starting
/// a program by its name does not do; elsewhere, and when nothing is found,
/// it is the name as it is.
fn resolved(program: &OsStr) -> OsString {
    #[cfg(windows)]
    {
        let path = std::env::var("PATH").ok();
        if let Some(found) = program
            .to_str()
            .and_then(|name| bitbraid_terminal::shell::find_program(name, path.as_deref()))
        {
            return found.into_os_string();
        }
    }
    program.to_owned()
}

#[cfg(not(windows))]
fn add_args(command: &mut Command, _program: &OsStr, args: &[OsString]) {
    command.args(args);
}

/// explorer.exe reads its command line itself: a path is one word only in
/// quotes (a comma parts it otherwise), and `/select,` goes in front of
/// the quotes. Its words are handed over as they are, quoted here; a path
/// on Windows cannot hold a quote. Every other program's are quoted the
/// usual way.
#[cfg(windows)]
fn add_args(command: &mut Command, program: &OsStr, args: &[OsString]) {
    use std::os::windows::process::CommandExt as _;
    let explorer = Path::new(program)
        .file_stem()
        .is_some_and(|stem| stem.eq_ignore_ascii_case("explorer"));
    if !explorer {
        command.args(args);
        return;
    }
    for arg in args {
        command.raw_arg(explorer_word(&arg.to_string_lossy()));
    }
}

/// A word of explorer.exe's command line: `/select,` kept in front, the
/// path quoted, with `\` between its parts and no quote in it.
#[cfg_attr(not(windows), allow(dead_code))]
fn explorer_word(arg: &str) -> String {
    let (switch, path) = match arg.strip_prefix("/select,") {
        Some(path) => ("/select,", path),
        None => ("", arg),
    };
    format!("{switch}\"{}\"", path.replace('/', "\\").replace('"', ""))
}

/// The words of an editor command with `{path}` and `{line}` filled in,
/// split and unquoted as [`crate::command_words`] says. The path is
/// appended when the command does not place it. No shell is involved, so
/// nothing in a path can turn into syntax.
fn editor_words(command: &str, path: &Path, line: u32) -> Result<Vec<OsString>, CommandError> {
    use crate::command_words::{LINE, PATH, fill};
    let line = OsString::from(line.to_string());
    let mut filled = fill(command, &[(PATH, path.as_os_str()), (LINE, &line)])?;
    if !filled.placed.contains(PATH) {
        filled.words.push(path.as_os_str().to_owned());
    }
    Ok(filled.words)
}

/// The system's way to open a file or folder with its default application.
pub(crate) fn system_opener(target: &Path) -> (OsString, Vec<OsString>) {
    if cfg!(target_os = "macos") {
        ("open".into(), vec![target.as_os_str().to_owned()])
    } else if cfg!(target_os = "windows") {
        // Not `cmd /C start`: the command prompt reads `&`, `^` and `%`
        // in a file's name as its own syntax.
        ("explorer.exe".into(), vec![target.as_os_str().to_owned()])
    } else {
        ("xdg-open".into(), vec![target.as_os_str().to_owned()])
    }
}

/// Where copies of files as they were in a commit are kept: a folder of
/// the user's own under the temporary directory.
fn copies_dir() -> std::path::PathBuf {
    let user = ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .map(|user| {
            user.chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
                .take(40)
                .collect::<String>()
        })
        .filter(|user| !user.is_empty() && user != "." && user != "..")
        .unwrap_or_else(|| "user".to_owned());
    std::env::temp_dir()
        .join(format!("bitbraid-{user}"))
        .join("at")
}

/// Make a folder that only its owner reads, and refuse one that is a
/// link to somewhere else: the temporary directory is everybody's.
fn private_dir(dir: &Path) -> Result<(), CommandError> {
    let failed = |err: std::io::Error| CommandError::InvalidParam {
        name: "rev".to_owned(),
        detail: say!(
            "file-cannot-make-folder",
            path = dir.display().to_string(),
            error = err.to_string()
        ),
    };
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder.create(dir).map_err(failed)?;
    let mut at = Some(dir);
    // The folder itself and the two above it, which are ours as well.
    for _ in 0..3 {
        let Some(part) = at else { break };
        let meta = part.symlink_metadata().map_err(failed)?;
        // A link, or a folder others may write into, is somebody else's.
        #[cfg(unix)]
        let shared = {
            use std::os::unix::fs::PermissionsExt as _;
            meta.permissions().mode() & 0o022 != 0
        };
        #[cfg(not(unix))]
        let shared = false;
        if !meta.is_dir() || shared {
            return Err(CommandError::InvalidParam {
                name: "rev".to_owned(),
                detail: say!("file-not-own-folder", path = part.display().to_string()),
            });
        }
        at = part.parent();
    }
    Ok(())
}

/// Write the file as it is in a commit to a read-only copy and say where
/// that is.
fn copy_at(
    repo: &bitbraid_git::Repository,
    path: &str,
    rev: &str,
) -> Result<(std::path::PathBuf, bitbraid_git::Oid), CommandError> {
    let invalid = |detail: String| CommandError::InvalidParam {
        name: "rev".to_owned(),
        detail,
    };
    let DiffSide::Commit(id) = DiffSide::parse(repo, rev)? else {
        return Err(invalid(say!("file-not-a-commit").to_owned()));
    };
    let content = bitbraid_git::file_bytes(repo, &DiffSide::Commit(id), path, MAX_BYTES)?
        .ok_or_else(|| invalid(say!("file-not-in-commit", path, commit = id.short(7))))?;
    let name = Path::new(path)
        .file_name()
        .ok_or_else(|| invalid(say!("file-names-no-file", path)))?;
    let dir = copies_dir().join(id.short(12));
    private_dir(&dir)?;
    let file = dir.join(name);
    let failed = |err: std::io::Error| {
        invalid(say!(
            "file-cannot-write",
            path = file.display().to_string(),
            error = err.to_string()
        ))
    };
    // The copy from last time is read-only: it is replaced, not written to.
    match std::fs::remove_file(&file) {
        Ok(()) => {}
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
        Err(err) => return Err(failed(err)),
    }
    std::fs::write(&file, &content).map_err(failed)?;
    let mut permissions = std::fs::metadata(&file).map_err(failed)?.permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&file, permissions).map_err(failed)?;
    Ok((file, id))
}

fn open(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let command: Option<String> = param_or(params, "command", None)?;
    let line: u32 = param_or(params, "line", 1)?;
    let rev: Option<String> = param_or(params, "rev", None)?;
    let (file, at) = match rev.as_deref().map(str::trim).filter(|rev| !rev.is_empty()) {
        Some(rev) => {
            let (file, id) = copy_at(ctx.repo()?, &path, rev)?;
            (file, Some(id))
        }
        None => (bitbraid_git::worktree_path(ctx.repo()?, &path)?, None),
    };
    let (program, args) = match command.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(command) => {
            let mut words = editor_words(command, &file, line)?.into_iter();
            let program = words.next().ok_or(CommandError::InvalidParam {
                name: "command".to_owned(),
                detail: say!("file-command-empty").to_owned(),
            })?;
            (program, words.collect())
        }
        None => system_opener(&file),
    };
    launch(&program, &args)?;
    Ok(match at {
        // As text: `json!` cannot write a path that is not UTF-8.
        Some(id) => json!({ "opened": path, "rev": id, "copy": file.display().to_string() }),
        None => json!({ "opened": path }),
    })
}

fn reveal(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let command: Option<String> = param_or(params, "command", None)?;
    let file = bitbraid_git::worktree_path(ctx.repo()?, &path)?;
    // A command of the user's is written for a folder, as the one that
    // shows the repository's is.
    if let Some(command) = command.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        let folder = file.parent().unwrap_or(&file);
        let mut words = editor_words(command, folder, 1)?.into_iter();
        let program = words.next().ok_or(CommandError::InvalidParam {
            name: "command".to_owned(),
            detail: say!("file-command-empty").to_owned(),
        })?;
        launch(&program, &words.collect::<Vec<_>>())?;
        return Ok(json!({ "revealed": path }));
    }
    let (program, args): (OsString, Vec<OsString>) = if cfg!(target_os = "macos") {
        (
            "open".into(),
            vec!["-R".into(), file.as_os_str().to_owned()],
        )
    } else if cfg!(target_os = "windows") {
        let mut select = OsString::from("/select,");
        select.push(file.as_os_str());
        ("explorer.exe".into(), vec![select])
    } else {
        // No portable way to select a file: open the folder it is in.
        system_opener(file.parent().unwrap_or(&file))
    };
    launch(&program, &args)?;
    Ok(json!({ "revealed": path }))
}

fn restore(ctx: &mut CommandContext, params: &Params) -> Result<Output, CommandError> {
    let path: String = param(params, "path")?;
    let rev_text: String = param(params, "rev")?;
    let repo = ctx.repo()?;
    let DiffSide::Commit(rev) = DiffSide::parse(repo, &rev_text)? else {
        return Err(CommandError::InvalidParam {
            name: "rev".to_owned(),
            detail: say!("file-not-a-commit").to_owned(),
        });
    };
    let backup = bitbraid_git::restore_path_at(repo, &path, rev)?;
    ctx.record(
        say!(
            "file-entry-restore",
            path = path.as_str(),
            commit = rev.short(7)
        ),
        Change::Restore { backup, rev },
    )?;
    Ok(json!({ "restored": path, "rev": rev.to_hex() }))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{editor_words, explorer_word};

    #[test]
    fn explorer_is_given_a_path_as_one_quoted_word() {
        assert_eq!(explorer_word(r"C:\r\a,b&c.txt"), r#""C:\r\a,b&c.txt""#);
        assert_eq!(explorer_word(r"C:\r/sub/a b.txt"), r#""C:\r\sub\a b.txt""#);
        assert_eq!(
            explorer_word(r"/select,C:\r\a b.txt"),
            r#"/select,"C:\r\a b.txt""#
        );
        assert_eq!(explorer_word(r#"C:\r\"x".txt"#), r#""C:\r\x.txt""#);
    }

    fn words(command: &str, path: &str, line: u32) -> Vec<String> {
        editor_words(command, Path::new(path), line)
            .unwrap_or_else(|err| panic!("{command:?}: {err}"))
            .into_iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn the_path_and_line_are_filled_in_as_single_words() {
        assert_eq!(
            words("code --goto {path}:{line}", "/r/a b.rs", 12),
            ["code", "--goto", "/r/a b.rs:12"]
        );
        assert_eq!(words("  vim  ", "/r/x", 1), ["vim", "/r/x"]);
        assert_eq!(
            words("edit +{line} {path}", "/r/x", 7),
            ["edit", "+7", "/r/x"]
        );
    }

    /// Quoted the way a shell command is, the placeholders are the path
    /// and the line, without the quotes.
    #[test]
    fn quoted_placeholders_are_filled_in_without_the_quotes() {
        let want = ["/t/capture-tool", "editor", "/r/a b.rs", "12"];
        assert_eq!(
            words(
                "/t/capture-tool editor \"{path}\" \"{line}\"",
                "/r/a b.rs",
                12
            ),
            want
        );
        assert_eq!(
            words("/t/capture-tool editor '{path}' '{line}'", "/r/a b.rs", 12),
            want
        );
        assert_eq!(
            words(
                "\"/opt/my editor/bin/ed\" --goto \"{path}:{line}\"",
                "/r/x",
                3
            ),
            ["/opt/my editor/bin/ed", "--goto", "/r/x:3"]
        );
        assert!(editor_words("ed \"{path}", Path::new("/r/x"), 1).is_err());
    }

    #[test]
    fn a_hostile_path_stays_one_argument() {
        let path = "/r/$(rm -rf ~); `x` | y && z {path} {line}";
        assert_eq!(words("editor {path}", path, 3), ["editor", path]);
        assert_eq!(words("editor", path, 3), ["editor", path]);
        // An empty command has no program; the caller uses the system's
        // opener instead.
        assert_eq!(words("", "/r/x", 1), ["/r/x"]);
    }
}
