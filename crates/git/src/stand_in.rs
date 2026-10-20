//! macOS's `/usr/bin/git`, which is no git of its own but stands in for
//! the git of Apple's developer tools. It is always there, so `PATH`
//! always has a git on macOS. With the Command Line Tools or Xcode
//! installed it runs their git; without them it runs nothing and puts up
//! the system's offer to install them, every time it is run. The
//! application runs git in the background, so that would be a dialog out
//! of nowhere, again and again. While the tools are not there the stand-in
//! is never run and counts as no git: a git further along `PATH` runs
//! instead, else the one shipped with the application ([`crate::shipped`]),
//! else the git the stand-in would run, which is not there, so that what
//! fails says what is missing.
//!
//! Whether the tools are there is asked of `xcode-select -p`, which prints
//! the developer folder chosen (`DEVELOPER_DIR` when that is set, as it is
//! not checked) and fails when none is; they are there when that folder
//! has `usr/bin/git`, where Xcode and the Command Line Tools both have it.
//! `xcode-select` is a program of its own, not a stand-in, and shows
//! nothing. What it says is kept for a few seconds: git is run often.
//!
//! Sources: xcode-select(1) (`-p`, `--install`, `DEVELOPER_DIR`), xcrun(1)
//! (how the tools in `/usr/bin` find the developer folder), Apple's
//! Technical Note TN2339, "Building from the Command Line with Xcode FAQ"
//! (the Command Line Tools, `xcode-select --install`).

use std::path::{Path, PathBuf};

/// Where macOS's stand-in is.
pub const STAND_IN: &str = "/usr/bin/git";

/// Where the Command Line Tools are installed.
pub const COMMAND_LINE_TOOLS: &str = "/Library/Developer/CommandLineTools";

/// Whether `candidate` is macOS's stand-in and what it stands in for is
/// not installed: a git that must not be run. Never so off macOS.
pub fn is_idle(candidate: &Path) -> bool {
    is_idle_tool(candidate, "git")
}

/// The same for another of the developer tools' programs that `/usr/bin`
/// stands in for (`opendiff`, whose FileMerge is Xcode's): `candidate` is
/// `/usr/bin/<program>`, and the developer folder has no such program.
pub fn is_idle_tool(candidate: &Path, program: &str) -> bool {
    is_stand_in_for(candidate, program) && developer_program(program).is_none()
}

/// Whether `candidate` is `/usr/bin/<program>`, links and `..` resolved,
/// on macOS. Never so elsewhere, nor for a name with a separator in it.
fn is_stand_in_for(candidate: &Path, program: &str) -> bool {
    cfg!(target_os = "macos")
        && !program.is_empty()
        && !program.contains('/')
        && std::fs::canonicalize(candidate)
            .is_ok_and(|real| real == Path::new("/usr/bin").join(program))
}

/// The git the stand-in would run, where it would be: in the developer
/// folder chosen, else in the Command Line Tools'.
pub fn missing_git() -> PathBuf {
    developer_folder()
        .unwrap_or_else(|| PathBuf::from(COMMAND_LINE_TOOLS))
        .join("usr")
        .join("bin")
        .join("git")
}

/// What is said where the git the stand-in would run cannot be run.
pub fn hint(program: &Path) -> Option<&'static str> {
    (cfg!(target_os = "macos") && program == missing_git() && !program.exists())
        .then(|| say!("repo-stand-in-hint"))
}

/// A program of the developer tools, when they are installed and have it.
fn developer_program(program: &str) -> Option<PathBuf> {
    Some(developer_folder()?.join("usr").join("bin").join(program)).filter(|found| found.is_file())
}

/// The developer folder `xcode-select -p` names, asked at most every few
/// seconds.
fn developer_folder() -> Option<PathBuf> {
    use std::{
        ffi::OsString,
        sync::Mutex,
        time::{Duration, Instant},
    };
    /// What was said, under which `DEVELOPER_DIR`, and when.
    type Said = (Option<OsString>, Instant, Option<PathBuf>);
    static SAID: Mutex<Option<Said>> = Mutex::new(None);
    const KEPT: Duration = Duration::from_secs(5);
    if !cfg!(target_os = "macos") {
        return None;
    }
    let chosen = bitbraid_core::process::var_os("DEVELOPER_DIR");
    let mut said = match SAID.lock() {
        Ok(said) => said,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((of, at, folder)) = said.as_ref()
        && *of == chosen
        && at.elapsed() < KEPT
    {
        return folder.clone();
    }
    let folder = ask_xcode_select();
    *said = Some((chosen, Instant::now(), folder.clone()));
    folder
}

/// `xcode-select -p`, in the environment the programs the application
/// starts have: the folder it prints, or none when it fails.
fn ask_xcode_select() -> Option<PathBuf> {
    use std::process::{Command, Stdio};
    let mut command = Command::new("/usr/bin/xcode-select");
    bitbraid_core::process::with_user_environment(&mut command);
    let out = command
        .arg("-p")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let folder = text.trim();
    (!folder.is_empty()).then(|| PathBuf::from(folder))
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{STAND_IN, hint, is_idle, is_idle_tool, is_stand_in_for, missing_git};

    /// Whether `candidate` is macOS's stand-in for git.
    fn is_stand_in(candidate: &Path) -> bool {
        is_stand_in_for(candidate, "git")
    }

    #[test]
    fn only_the_one_in_usr_bin_is_the_stand_in() {
        assert!(!is_stand_in(Path::new("/opt/homebrew/bin/git")));
        assert!(!is_stand_in(Path::new("/nowhere/git")));
        assert!(!is_stand_in(Path::new("")));
        assert!(!is_idle(Path::new("/nowhere/git")));
        // Only a program's own name in /usr/bin.
        assert!(!is_idle_tool(Path::new("/usr/bin/git"), "../bin/git"));
        assert!(!is_idle_tool(Path::new("/usr/bin/git"), ""));
        if cfg!(target_os = "macos") {
            assert!(is_stand_in(Path::new(STAND_IN)));
            assert!(is_stand_in(Path::new("/usr/bin/../bin/git")));
            assert!(!is_stand_in(Path::new("/usr/bin/opendiff")));
        } else {
            assert!(!is_stand_in(Path::new(STAND_IN)));
            assert!(!is_idle(Path::new(STAND_IN)));
            assert_eq!(hint(&missing_git()), None);
        }
        // A git that is there is never said to be missing.
        assert_eq!(hint(Path::new("/bin/sh")), None);
    }
}
