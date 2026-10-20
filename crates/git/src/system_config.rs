//! Where the git that is run keeps its system-wide configuration, so that
//! gitoxide reads the same file as the git children do.
//!
//! gitoxide does not know which git is run. Left to itself it takes
//! `/etc/gitconfig` for the system configuration on Unix, and on Windows it
//! asks the `git` on `PATH`. Either is another git's file when the
//! executable in use is installed elsewhere: `/opt/git/bin/git` reads
//! `/opt/git/etc/gitconfig`, a git from a package manager of the user's
//! reads one under that manager's prefix. What gitoxide read (for the
//! graph, the status, the settings shown) and what the git children did
//! (commit, merge, fetch) could then disagree.
//!
//! So the git in use is asked which file its system scope is: the one the
//! preferences name, else `$BITBRAID_GIT`, else `git` on `PATH`, else the
//! one shipped with the application ([`crate::shipped`]), which, built to
//! be moved, reads `etc/gitconfig` of its own installation and says so
//! like any other. It is asked once for each executable, when the first
//! repository is opened with it; the answer is handed to gitoxide with
//! every repository that is opened after that:
//!
//! * a file: gitoxide reads that file as the system configuration;
//! * two files (a git that ships a configuration of its own in front of
//!   the system's): the first as the installation's, the second as the
//!   system's;
//! * none (git says its system file is not there): gitoxide reads no
//!   system configuration, rather than the one of another git;
//! * no answer (no git, or something that does not answer as git does):
//!   gitoxide's own idea holds.
//!
//! `GIT_CONFIG_SYSTEM` and `GIT_CONFIG_NOSYSTEM` in the environment reach
//! the git that is asked like every other child, so what they say holds for
//! both. A repository that is open when another executable is chosen is
//! opened again the next time anything is asked of it
//! (`Repository::follow_git`, which the engine calls before every command
//! and the interface's sessions before every request), so it has the new
//! answer from then on.
//!
//! Not covered: the system-wide attributes file (`$(prefix)/etc/gitattributes`),
//! for which gitoxide has no way to be told a place.
//!
//! Every repository is opened with these options, so the room its object
//! database has for the packs git adds while it is open is set here too
//! ([`OBJECT_STORE_SLOTS`]).
//!
//! Sources: git-config(1) (`--system`, `--show-origin`, `--no-includes`,
//! `-z`, FILES, `GIT_CONFIG_SYSTEM`, `GIT_CONFIG_NOSYSTEM`); gitoxide's
//! `open::Options::system_config_path`, `git_installation_config_path` and
//! `object_store_slots`; gix-odb's `store::init::Slots`.

use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
    sync::Mutex,
    time::Duration,
};

use gix::sec::trust::DefaultForLevel as _;

use crate::{Error, exe};

/// What the git that is run says about its system configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SystemConfig {
    /// These files, in the order git reads them: one, or the installation's
    /// own and then the system's.
    Files(Vec<PathBuf>),
    /// It has none: the file is not there.
    None,
    /// It could not be asked, or did not answer as git does.
    Unknown,
}

/// How long an executable gets to say where its system configuration is.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// The last executable that was asked, and what it said.
static ASKED: Mutex<Option<(OsString, SystemConfig)>> = Mutex::new(None);

/// The files of the system scope git names as the origin of what it
/// lists, in the order they come and each once. The list is what `git
/// config --list --show-scope --show-origin --name-only -z` prints: a
/// scope, an origin and a name in turn, each ended by a NUL. Origins that
/// are no files (a blob, the command line) are left out.
pub fn origins(listed: &str) -> Vec<PathBuf> {
    let fields: Vec<&str> = listed.split('\0').collect();
    let mut files: Vec<PathBuf> = Vec::new();
    for entry in fields.chunks(3) {
        let [scope, origin, _name] = entry else {
            continue;
        };
        if *scope != "system" {
            continue;
        }
        let Some(file) = origin.strip_prefix("file:") else {
            continue;
        };
        let file = PathBuf::from(file);
        if !file.as_os_str().is_empty() && !files.contains(&file) {
            files.push(file);
        }
    }
    files
}

/// Ask `program`, as git, where its system configuration is.
pub fn system_config_of(program: &OsStr) -> SystemConfig {
    let answer = exe::asked_within(
        program,
        // Not `--system`, which reads the file when it is asked for even
        // where git leaves it out otherwise (`GIT_CONFIG_NOSYSTEM`): what
        // git reads, with the scope of each, and the system's taken.
        &[
            "config",
            "--no-includes",
            "--list",
            "--show-scope",
            "--show-origin",
            "--name-only",
            "-z",
        ],
        ANSWER_WITHIN,
    );
    match answer {
        Ok(out) => {
            let files = origins(&out.stdout);
            // A file git names and that is not there for us (a name that
            // is no text, a file gone since) is not one to hand on.
            if files.iter().any(|file| !file.is_file()) {
                SystemConfig::Unknown
            } else if files.is_empty() {
                SystemConfig::None
            } else {
                SystemConfig::Files(files)
            }
        }
        // Git ran and said its system file is not there.
        Err(Error::Command { stderr, .. }) if stderr.contains("unable to read config file") => {
            SystemConfig::None
        }
        Err(_) => SystemConfig::Unknown,
    }
}

/// Where the git in use keeps its system configuration. It is asked once
/// for each executable; choosing another asks again.
pub fn system_config() -> SystemConfig {
    let program = exe::git_binary();
    let mut asked = match ASKED.lock() {
        Ok(asked) => asked,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some((of, said)) = asked.as_ref()
        && *of == program
    {
        return said.clone();
    }
    let said = system_config_of(&program);
    *asked = Some((program, said.clone()));
    said
}

/// Options for opening a repository with the system configuration of the
/// git in use put in place of gitoxide's own idea of it, and room for the
/// packs git adds while it is open ([`OBJECT_STORE_SLOTS`]).
fn with_system(
    options: gix::open::Options,
    mut permissions: gix::open::Permissions,
    said: &SystemConfig,
) -> gix::open::Options {
    let options = options.object_store_slots(OBJECT_STORE_SLOTS);
    match said {
        SystemConfig::Files(files) => match files.as_slice() {
            [] => options,
            // Nothing else: gitoxide would ask a git of its own choosing
            // for an installation's file, and on Windows be told the
            // system's file of Git for Windows, whichever git is run.
            [system] => {
                permissions.config.git_binary = false;
                options.permissions(permissions).system_config_path(system)
            }
            [installation, system, ..] => {
                permissions.config.git_binary = true;
                options
                    .permissions(permissions)
                    .git_installation_config_path(installation)
                    .system_config_path(system)
            }
        },
        SystemConfig::None => {
            permissions.config.system = false;
            permissions.config.git_binary = false;
            options.permissions(permissions)
        }
        SystemConfig::Unknown => options,
    }
}

/// How much room the object database of an open repository has for packs:
/// twice as many as are on disk when it is opened, and never less than
/// 1024. gitoxide fixes the room when the repository is opened and fails
/// a read once more packs are on disk than it holds; a repository stays
/// open for as long as its tab, while git (a fetch, a partial clone's
/// fetches of what it left on its remote, another program working in the
/// repository) adds packs. Its own default is a tenth more than are on
/// disk, at least 32, and 32 for a repository that is not trusted.
///
/// A place costs about sixteen bytes until a pack is in it (gix-odb's
/// `MutableIndexAndPack`: a swappable pointer, a lock, a generation
/// number), so 1024 of them are some sixteen kilobytes for each handle.
/// A read that finds the room full all the same opens the repository
/// again and is tried once more ([`crate::repo::is_out_of_room`]).
pub const OBJECT_STORE_SLOTS: gix::odb::store::init::Slots =
    gix::odb::store::init::Slots::AsNeededByDiskState {
        multiplier: 2.0,
        minimum: 1024,
    };

/// What a repository is opened with when its place is known.
pub(crate) fn open_options() -> gix::open::Options {
    with_system(
        gix::open::Options::default(),
        gix::open::Permissions::default(),
        &system_config(),
    )
}

/// What a repository is opened with when it is looked for from a path:
/// the options for one that is trusted and for one that is not.
pub(crate) fn trust_map() -> gix::sec::trust::Mapping<gix::open::Options> {
    let said = system_config();
    let of = |level: gix::sec::Trust| {
        with_system(
            gix::open::Options::default_for_level(level),
            gix::open::Permissions::default_for_level(level),
            &said,
        )
    };
    gix::sec::trust::Mapping {
        full: of(gix::sec::Trust::Full),
        reduced: of(gix::sec::Trust::Reduced),
    }
}

/// What gitoxide is told of a repository inside a WSL distribution, whose
/// git is the distribution's (`exe`, `bitbraid_core::wsl`):
/// the system configuration that git reads, the distribution's
/// `/etc/gitconfig` through the share (or none, where it has none), in
/// place of the Windows git's; and `core.filemode` off, since Windows
/// cannot see the executable bit through the share and every script
/// would read as changed.
pub fn wsl_options(
    inside: &bitbraid_core::wsl::WslPath,
    level: gix::sec::Trust,
) -> gix::open::Options {
    let system = PathBuf::from(format!(r"\\wsl$\{}\etc\gitconfig", inside.distribution));
    let said = if system.is_file() {
        SystemConfig::Files(vec![system])
    } else {
        SystemConfig::None
    };
    with_system(
        gix::open::Options::default_for_level(level),
        gix::open::Permissions::default_for_level(level),
        &said,
    )
    .config_overrides(["core.filemode=false"])
}

/// [`trust_map`] for a repository inside a WSL distribution
/// ([`wsl_options`]).
pub fn wsl_trust_map(
    inside: &bitbraid_core::wsl::WslPath,
) -> gix::sec::trust::Mapping<gix::open::Options> {
    gix::sec::trust::Mapping {
        full: wsl_options(inside, gix::sec::Trust::Full),
        reduced: wsl_options(inside, gix::sec::Trust::Reduced),
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::origins;

    /// What a repository inside WSL is opened with, tried on one here:
    /// its own `core.filemode` is passed over, and no system file is read
    /// where the distribution has none (here, none of that name exists).
    #[test]
    fn a_repository_inside_wsl_is_read_without_the_executable_bit() {
        let scratch = bitbraid_testkit::Scratch::of("linear").unwrap();
        scratch
            .git()
            .run(["config", "core.filemode", "true"])
            .unwrap();
        let inside =
            bitbraid_core::wsl::WslPath::parse(r"\\wsl$\NoSuchDistribution\home\me\repo").unwrap();
        let options = super::wsl_options(&inside, gix::sec::Trust::Full);
        let repo = gix::open_opts(scratch.path(), options).unwrap();
        let config = repo.config_snapshot();
        assert_eq!(config.boolean("core.filemode"), Some(false));
        let plain = gix::open_opts(scratch.path(), super::open_options()).unwrap();
        assert_eq!(plain.config_snapshot().boolean("core.filemode"), Some(true));
    }

    #[test]
    fn the_files_git_names_are_read_from_its_list() {
        let files = |listed: &str| origins(listed);
        assert_eq!(
            files(
                "system\0file:/etc/gitconfig\0core.autocrlf\0system\0file:/etc/gitconfig\0include.path\0"
            ),
            [PathBuf::from("/etc/gitconfig")]
        );
        // A git with a configuration of its own in front of the system's.
        assert_eq!(
            files("system\0file:/opt/git/share/gitconfig\0a.b\0system\0file:/etc/gitconfig\0c.d\0"),
            [
                PathBuf::from("/opt/git/share/gitconfig"),
                PathBuf::from("/etc/gitconfig")
            ]
        );
        // The other scopes are not the system's.
        assert_eq!(
            files(
                "global\0file:/home/u/.gitconfig\0user.name\0system\0file:/etc/gitconfig\0a.b\0\
                 local\0file:.git/config\0core.bare\0"
            ),
            [PathBuf::from("/etc/gitconfig")]
        );
        assert!(files("global\0file:/home/u/.gitconfig\0user.name\0").is_empty());
        // A path with a space and a colon in it, as Windows has them.
        assert_eq!(
            files("system\0file:C:/Program Files/Git/etc/gitconfig\0core.symlinks\0"),
            [PathBuf::from("C:/Program Files/Git/etc/gitconfig")]
        );
        // Nothing listed, and things that are no files.
        assert!(files("").is_empty());
        assert!(files("\0\0\0").is_empty());
        assert!(
            files("command\0command line:\0a.b\0system\0blob:1234\0c.d\0system\0standard input:\0e.f\0")
                .is_empty()
        );
        assert!(files("system\0file:\0a.b\0").is_empty());
        // A name that looks like an origin is a name: only the second of
        // each three fields is one.
        assert_eq!(
            files("system\0file:/a\0file:/not-an-origin\0system\0file:/a\0x.y\0"),
            [PathBuf::from("/a")]
        );
        // Anything at all.
        for odd in [
            "file:",
            "\u{0}",
            "garbage without an end",
            "system\0file:/x",
            "a\0b\0c",
        ] {
            let _ = files(odd);
        }
    }
}
