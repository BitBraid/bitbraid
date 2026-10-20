//! Which git is run, what version it is, and whether that is new enough
//! for what the rest of this crate asks of it.
//!
//! Sources: git-version(1); the release notes of git 2.23 (`switch`,
//! `restore`), 2.28 (`init --initial-branch`), 2.36 (`worktree list
//! --porcelain -z`), 2.38 (`merge-tree --write-tree`) and 2.45
//! (`merge-tree --merge-base` given trees).

use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Serialize;

use crate::{Error, Result, exe};

/// A git version as `git --version` prints it: `2.56.0`, and whatever a
/// packager added after it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Version {
    /// First number.
    pub major: u32,
    /// Second number.
    pub minor: u32,
    /// Third number; 0 when there is none.
    pub patch: u32,
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// The oldest git everything here works with. Below it nothing can be
/// merged in memory, and below 2.36 the worktree list cannot be read.
pub const MINIMUM: Version = Version {
    major: 2,
    minor: 38,
    patch: 0,
};

/// What a git older than some version does without, newest need first.
const NEEDS: &[(Version, bitbraid_strings::Message)] = &[
    (
        Version {
            major: 2,
            minor: 45,
            patch: 0,
        },
        message!("version-lacks-stash-foretelling"),
    ),
    (
        Version {
            major: 2,
            minor: 38,
            patch: 0,
        },
        message!("version-lacks-conflict-foretelling"),
    ),
    (
        Version {
            major: 2,
            minor: 36,
            patch: 0,
        },
        message!("version-lacks-worktree-list"),
    ),
    (
        Version {
            major: 2,
            minor: 28,
            patch: 0,
        },
        message!("version-lacks-initial-branch"),
    ),
    (
        Version {
            major: 2,
            minor: 23,
            patch: 0,
        },
        message!("version-lacks-switch-restore"),
    ),
];

/// The version in what `git --version` printed: `git version 2.56.0`,
/// `git version 2.39.5 (Apple Git-154)`, `git version 2.47.1.windows.2`.
/// None for anything that is not git's answer.
pub fn parse_version(text: &str) -> Option<Version> {
    let rest = text.trim().strip_prefix("git version ")?;
    let numbers = rest.split_whitespace().next()?;
    let mut parts = numbers.split('.');
    let mut number = || -> Option<u32> {
        let part = parts.next()?;
        // `2.56.0-rc1`: the digits a part starts with.
        let digits: String = part.chars().take_while(char::is_ascii_digit).collect();
        digits.parse().ok()
    };
    let major = number()?;
    let minor = number()?;
    let patch = number().unwrap_or(0);
    Some(Version {
        major,
        minor,
        patch,
    })
}

/// What a git of this version does without.
pub fn lacking(version: Version) -> Vec<&'static str> {
    NEEDS
        .iter()
        .filter(|(since, _)| version < *since)
        .map(|(_, what)| what.text())
        .collect()
}

/// What is known about a git executable.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitInfo {
    /// What is run: a name looked up on `PATH`, or a path.
    pub program: String,
    /// Where that is, when it was found.
    pub path: Option<String>,
    /// Whether the preferences chose it, rather than it being the default.
    pub chosen: bool,
    /// Whether it is the one shipped with the application
    /// ([`crate::shipped`]): there is no other on `PATH`, or it was chosen.
    pub shipped: bool,
    /// What it answered to `--version`.
    pub version_text: String,
    /// The version in that answer.
    pub version: Version,
    /// Whether it is at least [`MINIMUM`].
    pub supported: bool,
    /// The oldest version everything works with.
    pub minimum: Version,
    /// What this version does without, in words.
    pub lacks: Vec<&'static str>,
}

/// How long an executable gets to say what version it is.
const ANSWER_WITHIN: Duration = Duration::from_secs(5);

/// Ask an executable what git it is: the one in use when `program` is
/// absent. Something that does not answer as git does is refused.
pub fn probe(program: Option<&OsStr>) -> Result<GitInfo> {
    probe_within(program, ANSWER_WITHIN)
}

/// [`probe`], with the time the executable gets to answer.
pub fn probe_within(program: Option<&OsStr>, limit: Duration) -> Result<GitInfo> {
    let asked = program.map(OsStr::to_os_string);
    if let Some(program) = &asked {
        check_name(program)?;
        // Never run, not even to ask: it would offer to install the tools.
        if locate(program).is_some_and(|found| crate::stand_in::is_idle(&found)) {
            return Err(Error::Refused {
                what: say!("version-what-git-executable"),
                detail: say!("version-stand-in", program = program.to_string_lossy()),
            });
        }
    }
    let run = asked.clone().unwrap_or_else(exe::git_binary);
    let chosen = match &asked {
        Some(program) => exe::chosen_git_binary().as_ref() == Some(program),
        None => exe::chosen_git_binary().is_some(),
    };
    let text = exe::version_text(Some(&run), limit).map_err(|err| match err {
        // The usual case, said plainly: there is no such program.
        Error::Git { context, source } if context == say!("exe-spawning") => Error::Refused {
            what: say!("version-what-git-executable"),
            detail: match crate::stand_in::hint(Path::new(&run)) {
                Some(hint) => say!(
                    "version-cannot-run-hint",
                    program = run.to_string_lossy(),
                    source = source.to_string(),
                    hint
                ),
                None => say!(
                    "version-cannot-run",
                    program = run.to_string_lossy(),
                    source = source.to_string()
                ),
            },
        },
        other => other,
    })?;
    let version = parse_version(&text).ok_or_else(|| Error::Refused {
        what: say!("version-what-git-executable"),
        detail: say!(
            "version-not-git",
            program = run.to_string_lossy(),
            answer = format!("{:?}", shortened(&text))
        ),
    })?;
    Ok(GitInfo {
        program: run.to_string_lossy().into_owned(),
        path: locate(&run).map(|path| path.display().to_string()),
        chosen,
        shipped: crate::shipped::is_shipped(&run),
        version_text: text,
        version,
        supported: version >= MINIMUM,
        minimum: MINIMUM,
        lacks: lacking(version),
    })
}

/// A name that can be run at all.
fn check_name(program: &OsStr) -> Result<()> {
    let text = program.to_string_lossy();
    let bad = |detail: &str| Error::Invalid {
        what: say!("version-what-git-executable"),
        detail: detail.to_owned(),
    };
    if text.trim().is_empty() {
        return Err(bad(say!("version-no-name")));
    }
    if text.contains('\0') || text.chars().any(char::is_control) {
        return Err(bad(say!("version-name-control")));
    }
    if text.len() > 4096 {
        return Err(bad(say!("version-name-too-long")));
    }
    Ok(())
}

/// The start of an answer, for an error message.
fn shortened(text: &str) -> String {
    let first = text.lines().next().unwrap_or("");
    let mut cut: String = first.chars().take(80).collect();
    if cut.len() < first.len() {
        cut.push('…');
    }
    cut
}

/// Where a program is: itself when it is a path, else the first directory
/// of `PATH` that has it.
pub fn locate(program: &OsStr) -> Option<PathBuf> {
    locate_in(program, bitbraid_core::process::var_os("PATH").as_deref())
}

fn locate_in(program: &OsStr, path: Option<&OsStr>) -> Option<PathBuf> {
    let as_path = Path::new(program);
    if as_path.components().count() > 1 || as_path.is_absolute() {
        return as_path.is_file().then(|| as_path.to_path_buf());
    }
    on_path(program, path).next()
}

/// The git to run by name on `PATH`: the first one there that may be run,
/// macOS's stand-in passed over while it has no git behind it
/// ([`crate::stand_in`]); and whether it was.
pub(crate) fn git_on_path() -> (Option<PathBuf>, bool) {
    git_on_path_in(bitbraid_core::process::var_os("PATH").as_deref())
}

fn git_on_path_in(path: Option<&OsStr>) -> (Option<PathBuf>, bool) {
    let mut passed_over = false;
    for candidate in on_path(OsStr::new("git"), path) {
        if crate::stand_in::is_idle(&candidate) {
            passed_over = true;
        } else {
            return (Some(candidate), passed_over);
        }
    }
    (None, passed_over)
}

/// Every file named `program` in the directories of `path`, in order.
fn on_path(program: &OsStr, path: Option<&OsStr>) -> impl Iterator<Item = PathBuf> {
    let names: Vec<OsString> = if cfg!(windows) {
        ["", ".exe", ".cmd", ".bat"]
            .iter()
            .map(|ending| {
                let mut name = program.to_os_string();
                name.push(ending);
                name
            })
            .collect()
    } else {
        vec![program.to_os_string()]
    };
    path.map(std::env::split_paths)
        .into_iter()
        .flatten()
        .filter(|dir| !dir.as_os_str().is_empty())
        .flat_map(move |dir| names.clone().into_iter().map(move |name| dir.join(name)))
        .filter(|candidate| candidate.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_read_from_what_git_prints() {
        let version = |major, minor, patch| Version {
            major,
            minor,
            patch,
        };
        for (text, expected) in [
            ("git version 2.56.0", Some(version(2, 56, 0))),
            ("git version 2.56.0\n", Some(version(2, 56, 0))),
            (
                "git version 2.39.5 (Apple Git-154)",
                Some(version(2, 39, 5)),
            ),
            ("git version 2.47.1.windows.2", Some(version(2, 47, 1))),
            ("git version 2.45.0-rc1", Some(version(2, 45, 0))),
            ("git version 2.9", Some(version(2, 9, 0))),
            ("git version 3", None),
            ("git version", None),
            ("git version x.y.z", None),
            ("hg version 6.1", None),
            ("--version", None),
            ("", None),
            ("git version 99999999999999999999.1.1", None),
        ] {
            assert_eq!(parse_version(text), expected, "{text:?}");
        }
        assert_eq!(version(2, 56, 0).to_string(), "2.56.0");
    }

    #[test]
    fn an_older_git_is_told_what_it_does_without() {
        let version = |major, minor| Version {
            major,
            minor,
            patch: 0,
        };
        assert!(lacking(version(2, 56)).is_empty());
        assert!(lacking(version(3, 0)).is_empty());
        assert_eq!(
            lacking(version(2, 40)),
            ["foretelling whether a stash applies cleanly"]
        );
        assert_eq!(lacking(version(2, 37)).len(), 2);
        assert_eq!(lacking(version(1, 9)).len(), NEEDS.len());
        assert!(version(2, 37) < MINIMUM && version(2, 38) >= MINIMUM);
        // Ordered by version, not by text: 2.9 is older than 2.38.
        assert!(version(2, 9) < version(2, 38));
    }

    #[test]
    fn a_program_is_looked_up_on_the_path_it_is_given() {
        let root = std::env::temp_dir().join(format!("bitbraid-locate-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let (first, second) = (root.join("first"), root.join("second"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(second.join("tool"), "").unwrap();
        std::fs::create_dir(first.join("folder")).unwrap();
        let path = std::env::join_paths([&first, &second]).unwrap();
        assert_eq!(
            locate_in(OsStr::new("tool"), Some(&path)),
            Some(second.join("tool"))
        );
        // A directory of that name is not a program; nothing is not found.
        assert_eq!(locate_in(OsStr::new("folder"), Some(&path)), None);
        assert_eq!(locate_in(OsStr::new("absent"), Some(&path)), None);
        assert_eq!(locate_in(OsStr::new("tool"), None), None);
        // A path is taken as it is.
        let direct = second.join("tool");
        assert_eq!(
            locate_in(direct.as_os_str(), Some(&path)),
            Some(direct.clone())
        );
        assert_eq!(locate_in(root.join("nope").as_os_str(), Some(&path)), None);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn the_git_in_use_answers_and_other_things_are_refused() {
        let info = probe(None).unwrap();
        assert!(info.version_text.starts_with("git version "));
        assert_eq!(parse_version(&info.version_text), Some(info.version));
        assert!(info.path.is_some(), "{info:?}");

        for hostile in [
            "",
            "   ",
            "a\0b",
            "line\nbreak",
            "/",
            "/definitely/not/there",
        ] {
            assert!(
                probe(Some(OsStr::new(hostile))).is_err(),
                "{hostile:?} should be refused"
            );
        }
        assert!(probe(Some(&OsString::from("x".repeat(5000)))).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn a_program_that_is_not_git_or_never_ends_is_refused_in_time() {
        use std::os::unix::fs::PermissionsExt as _;
        let root = std::env::temp_dir().join(format!("bitbraid-probe-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let script = |name: &str, body: &str| {
            let path = root.join(name);
            std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            path
        };
        // A script that was just written is "busy" for as long as a child
        // another test's thread forked meanwhile holds it open for writing.
        // Run something that ends at once until it starts.
        let settled = |path: &std::path::Path| {
            for _ in 0..200 {
                match std::process::Command::new(path).arg("--settled").output() {
                    Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    _ => break,
                }
            }
        };
        let other = script("other", "echo 'other 1.0'");
        settled(&other);
        let err = probe(Some(other.as_os_str())).unwrap_err().to_string();
        assert!(err.contains("is not git"), "{err}");

        // It ends at once when asked to settle, and never otherwise.
        let stuck = script("stuck", "[ \"$1\" = --settled ] && exit 0\nexec sleep 30");
        settled(&stuck);
        let started = std::time::Instant::now();
        let err = probe_within(Some(stuck.as_os_str()), Duration::from_millis(300))
            .unwrap_err()
            .to_string();
        assert!(err.contains("did not answer"), "{err}");
        assert!(started.elapsed() < Duration::from_secs(5));

        let old = script("old", "echo 'git version 2.30.1'");
        settled(&old);
        let info = probe(Some(old.as_os_str())).unwrap();
        assert!(!info.supported);
        assert!(!info.lacks.is_empty());
        assert_eq!(info.path.as_deref(), Some(old.to_str().unwrap()));
        assert!(!info.chosen);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
