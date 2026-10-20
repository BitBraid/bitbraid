//! A git shipped with the application: where it lies beside the
//! executable on each system, and when it is the one that runs.
//!
//! The order in which a git is chosen is: the one the preferences name,
//! else `$BITBRAID_GIT`, else `git` on `PATH`, and only where there is none
//! on `PATH` the one shipped with the application. A git the user installed
//! comes first, with its configuration, its helpers and its version; the
//! shipped one is for a system that has none.
//!
//! Where it lies, from the application's own executable (its real place,
//! links followed):
//!
//! * Linux and the other Unix systems: `<exe dir>/../libexec/bitbraid/git/bin/git`.
//!   A git installation of its own, under `libexec`, which is for programs
//!   that other programs run and not users, so it never stands on `PATH`
//!   or hides a git there. It is the same for an installation under `/usr`,
//!   `/usr/local` or `/opt/bitbraid`, for a tarball, for an AppImage's `usr`
//!   and for a Flatpak's `/app`: `bin/bitbraid` beside `libexec/bitbraid/`.
//! * macOS: `<bundle>/Contents/Resources/git/bin/git`, from the executable
//!   in `Contents/MacOS`. A bundle's places for code (`Contents/Helpers`,
//!   `Contents/MacOS`) hold code alone: every file there is taken for a
//!   program and must be signed, and a script's signature is kept in
//!   extended attributes that archives and many transfers lose. git's
//!   installation is scripts and data as much as programs (its shell
//!   commands, the templates, the licences), so it lies whole among the
//!   resources, sealed with the bundle, each of its Mach-O programs signed
//!   on its own as well (`cargo xtask bundle`).
//! * Windows: `<exe dir>\git\cmd\git.exe`. MinGit, the build of Git for
//!   Windows that is made for programs to carry, is unpacked whole into
//!   `git`; `cmd\git.exe` is its entry for them, which finds the rest of
//!   the installation (its shell, its ssh, its libraries) from where it is.
//!
//! What a git needs to be moved like that is a build that finds everything
//! from where its own executable is (`RUNTIME_PREFIX`): the programs of
//! `git --exec-path` (`libexec/git-core`), the templates a new repository
//! starts from (`share/git-core/templates`), the messages, and the system
//! configuration (`etc/gitconfig`), each relative to its installation. Git
//! for Windows and MinGit are built so; on Linux and macOS it is a build of
//! git's release with `RUNTIME_PREFIX=YesPlease` (macOS reads the place of
//! the executable from the system as well). Nothing is set in the
//! environment of its children for it: no `GIT_EXEC_PATH`, no
//! `GIT_TEMPLATE_DIR`, no `GIT_CONFIG_SYSTEM`. Its system configuration is
//! found by [`crate::system_config`] as any git's is, by asking it.
//!
//! While it is the git in use, the folder it is in stands first on the
//! `PATH` of every program the application starts ([`follow`],
//! `bitbraid_core::process::set_path_first`): git itself, and through it
//! the hooks, the editor and the tools it runs; the editors and tools the
//! application starts; the shell of the terminal in the window. Each of
//! them that runs `git` by its name runs the same git as the application,
//! on a system that has no other. When another git is chosen it comes
//! off again.
//!
//! What goes into the package and how it is built is the release's
//! packaging, not this module's.
//!
//! Sources: git's `INSTALL` and the documentation of `RUNTIME_PREFIX` in
//! its Makefile (read, not copied); git(1) (`--exec-path`,
//! `GIT_EXEC_PATH`), git-init(1) (TEMPLATE DIRECTORY), git-config(1)
//! (FILES: `$(prefix)/etc/gitconfig`); the Filesystem Hierarchy Standard
//! 3.0 (`/usr/libexec`); Apple's "Placing content in a bundle" and
//! Technical Note TN2206, "macOS Code Signing In Depth" (the places for
//! nested code, and scripts in `Contents/Resources`); Git for
//! Windows' documentation of MinGit; Rust's `std::env::current_exe`.

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock},
};

/// The systems with a layout of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum System {
    /// Linux and the other Unix systems but macOS.
    Unix,
    /// macOS, where the application is a bundle.
    MacOs,
    Windows,
}

impl System {
    /// The system this was built for.
    pub fn this() -> Self {
        if cfg!(windows) {
            System::Windows
        } else if cfg!(target_os = "macos") {
            System::MacOs
        } else {
            System::Unix
        }
    }
}

/// Where a git shipped with the application lies on `system`, for the
/// application's executable at `exe`; whether it is there is not looked
/// at. None where `exe` has no directories enough above it.
pub fn layout(system: System, exe: &Path) -> Option<PathBuf> {
    let dir = exe.parent().filter(|dir| !dir.as_os_str().is_empty())?;
    Some(match system {
        System::Unix => dir
            .parent()?
            .join("libexec")
            .join("bitbraid")
            .join("git")
            .join("bin")
            .join("git"),
        System::MacOs => dir
            .parent()?
            .join("Resources")
            .join("git")
            .join("bin")
            .join("git"),
        System::Windows => dir.join("git").join("cmd").join("git.exe"),
    })
}

/// The git shipped with the application, when there is one where this
/// system's layout puts it. Looked for once: the application does not move
/// while it runs.
pub fn shipped_git() -> Option<PathBuf> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            let exe = std::env::current_exe().ok()?;
            // Started through a link (`/usr/bin/bitbraid` to the real one),
            // the layout is beside what the link points at. In the usual
            // form on Windows, not `\\?\`: its folder goes first on the
            // `PATH` of the terminal's shell, and the command prompt and
            // PowerShell do not look in a verbatim one.
            let exe = bitbraid_core::paths::canonical(&exe).unwrap_or(exe);
            layout(System::this(), &exe).filter(|git| git.is_file())
        })
        .clone()
}

/// The git to run in place of `git` by name, when that is not the one:
/// where `PATH` has none, the shipped git; where the first on `PATH` is
/// macOS's stand-in with no git behind it ([`crate::stand_in`]), the next
/// one on `PATH`, else the shipped git, else the git the stand-in would
/// run, which is not there (so that the stand-in is never run). `PATH` is
/// looked through again only when it is another than the last time (the
/// login environment may be taken over after start), or after the
/// stand-in was passed over (the tools may be installed meanwhile).
pub fn instead_of_path() -> Option<PathBuf> {
    type Looked = (Option<OsString>, Option<PathBuf>);
    static LOOKED: Mutex<Option<Looked>> = Mutex::new(None);
    let path = bitbraid_core::process::var_os("PATH");
    let mut looked = match LOOKED.lock() {
        Ok(looked) => looked,
        Err(poisoned) => poisoned.into_inner(),
    };
    let (found, passed_over) = match looked.as_ref() {
        Some((of, found)) if *of == path => (found.clone(), false),
        _ => {
            let (found, passed_over) = crate::version::git_on_path();
            *looked = (!passed_over).then(|| (path, found.clone()));
            (found, passed_over)
        }
    };
    drop(looked);
    match found {
        Some(git) => passed_over.then_some(git),
        None => shipped_git().or_else(|| passed_over.then(crate::stand_in::missing_git)),
    }
}

/// Whether `program` is the git shipped with the application.
pub fn is_shipped(program: &OsStr) -> bool {
    shipped_git().is_some_and(|shipped| Path::new(program) == shipped)
}

/// The folder to put first on the `PATH` of the programs started while
/// `program` is the git in use: the shipped git's own folder when it is
/// that one, else none.
pub fn path_first_for(program: &OsStr, shipped: Option<&Path>) -> Option<PathBuf> {
    let shipped = shipped.filter(|shipped| Path::new(program) == *shipped)?;
    shipped
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

/// Follow the choice of git: `program` is the one in use from now on. Its
/// folder is put first on the `PATH` of every program the application
/// starts while it is the shipped git, and taken off when it is not.
pub fn follow(program: &OsStr) {
    bitbraid_core::process::set_path_first(path_first_for(program, shipped_git().as_deref()));
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{System, layout, path_first_for};

    #[test]
    fn only_the_shipped_git_puts_its_folder_first() {
        let shipped = Path::new("/opt/bitbraid/libexec/bitbraid/git/bin/git");
        assert_eq!(
            path_first_for(shipped.as_os_str(), Some(shipped)),
            Some(PathBuf::from("/opt/bitbraid/libexec/bitbraid/git/bin"))
        );
        // Another git in use, or none shipped: nothing.
        assert_eq!(path_first_for("/usr/bin/git".as_ref(), Some(shipped)), None);
        assert_eq!(path_first_for("git".as_ref(), Some(shipped)), None);
        assert_eq!(path_first_for(shipped.as_os_str(), None), None);
        // A shipped git with no folder above it is nowhere to put first.
        assert_eq!(path_first_for("git".as_ref(), Some(Path::new("git"))), None);
    }

    #[test]
    fn each_system_has_its_place_beside_the_executable() {
        assert_eq!(
            layout(System::Unix, Path::new("/usr/bin/bitbraid")),
            Some(PathBuf::from("/usr/libexec/bitbraid/git/bin/git"))
        );
        assert_eq!(
            layout(System::Unix, Path::new("/opt/bitbraid/bin/bitbraid")),
            Some(PathBuf::from("/opt/bitbraid/libexec/bitbraid/git/bin/git"))
        );
        assert_eq!(
            layout(
                System::MacOs,
                Path::new("/Applications/BitBraid.app/Contents/MacOS/bitbraid")
            ),
            Some(PathBuf::from(
                "/Applications/BitBraid.app/Contents/Resources/git/bin/git"
            ))
        );
        let windows = layout(
            System::Windows,
            Path::new("C:/Program Files/BitBraid/bitbraid.exe"),
        )
        .unwrap();
        assert_eq!(
            windows
                .components()
                .rev()
                .take(4)
                .map(|part| part.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>(),
            ["git.exe", "cmd", "git", "BitBraid"]
        );
        // Too few directories above the executable: nowhere.
        assert_eq!(layout(System::Unix, Path::new("bitbraid")), None);
        assert_eq!(layout(System::Unix, Path::new("/bitbraid")), None);
        assert_eq!(layout(System::MacOs, Path::new("")), None);
        assert_eq!(layout(System::Windows, Path::new("bitbraid.exe")), None);
        assert!(layout(System::Windows, Path::new("/bitbraid.exe")).is_some());
    }
}
