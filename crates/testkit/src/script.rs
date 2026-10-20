//! Programs written by tests as `sh` scripts, made something that can be
//! run on every system.
//!
//! Tests stand programs in for git, editors, tools and hooks with short
//! scripts. On Unix a script is marked executable and run as it is. On
//! Windows nothing runs a script by its first line, so it is run by the
//! `sh` that Git for Windows brings along, through a batch file of the same
//! name beside it, and the batch file is what is handed out to be run.
//!
//! Sources: Git for Windows' installation layout (`cmd\git.exe`,
//! `usr\bin\sh.exe`), the `cmd /?` help (`%~dp0`, `%*`).

use std::path::{Path, PathBuf};

/// Write the `sh` script `body` as `dir/name` and return the path that runs
/// it: the script itself on Unix, `dir/name.cmd` on Windows. The script is
/// written beside and moved into place, so that a child forked meanwhile by
/// another test's thread never holds it open for writing ("Text file
/// busy").
pub fn script(dir: &Path, name: &str, body: &str) -> std::io::Result<PathBuf> {
    let path = dir.join(name);
    let beside = dir.join(format!("{name}.new"));
    std::fs::write(&beside, body)?;
    runnable(&beside)?;
    std::fs::rename(&beside, &path)?;
    launcher(&path)
}

/// Mark a script that is already written as one that can be run, and
/// return the path that runs it (see [`script`]).
pub fn make_runnable(path: &Path) -> std::io::Result<PathBuf> {
    runnable(path)?;
    launcher(path)
}

#[cfg(unix)]
fn runnable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
}

#[cfg(not(unix))]
fn runnable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(not(windows))]
fn launcher(path: &Path) -> std::io::Result<PathBuf> {
    Ok(path.to_path_buf())
}

/// A batch file beside the script that runs it with Git's `sh`, handing
/// on every argument. The folder of that `sh` goes first on the script's
/// `PATH`, as Git Bash has it: started from anywhere else (PowerShell,
/// Explorer) the programs a script uses (`cat`, `mkdir`) are not found
/// otherwise.
#[cfg(windows)]
fn launcher(path: &Path) -> std::io::Result<PathBuf> {
    let shell = git_sh().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "the sh of Git for Windows was not found",
        )
    })?;
    let bin = shell.parent().unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut batch = path.as_os_str().to_owned();
    batch.push(".cmd");
    let batch = PathBuf::from(batch);
    std::fs::write(
        &batch,
        format!(
            "@setlocal\r\n@set \"PATH={};%PATH%\"\r\n@\"{}\" \"%~dp0{name}\" %*\r\n@exit /b %ERRORLEVEL%\r\n",
            bin.display(),
            shell.display()
        ),
    )?;
    Ok(batch)
}

/// The `sh` of the Git for Windows whose `git` is on `PATH`: `git.exe` is
/// in `cmd` or `bin` of the installation, `sh.exe` in `usr\bin` or `bin`.
#[cfg(windows)]
pub fn git_sh() -> Option<PathBuf> {
    static FOUND: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    FOUND
        .get_or_init(|| {
            let path = std::env::var_os("PATH")?;
            std::env::split_paths(&path)
                .filter(|dir| dir.join("git.exe").is_file())
                .filter_map(|dir| dir.parent().map(Path::to_path_buf))
                .flat_map(|root| [root.join("usr\\bin\\sh.exe"), root.join("bin\\sh.exe")])
                .find(|sh| sh.is_file())
        })
        .clone()
}
