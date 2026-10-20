//! Paths as the system and the people using it write them.
//!
//! On Windows `std::fs::canonicalize` answers in the verbatim form
//! (`\\?\C:\work\repo`): a form where `/` is not a separator, that
//! `cmd.exe` refuses as a working folder, that git's own programs do not
//! all take, and that nobody wants to read in a title or a status bar.
//! [`canonical`] gives the usual form back whenever it names the same
//! place, a long one included: the standard library makes a path of 260
//! characters or more verbatim again on its own where it hands it to the
//! system, so the length is no reason to keep the prefix.
//!
//! What is too long for Windows ([`beyond_windows_limit`]) is what git for
//! Windows refuses without `core.longpaths`: a path of 260 UTF-16 units or
//! more, the NUL that ends it counted (`MAX_PATH`), or a folder to be made
//! of 248 or more.
//!
//! Sources: Microsoft Learn, "Naming Files, Paths, and Namespaces"
//! (the `\\?\` prefix, reserved names, characters a name cannot hold, a
//! name not ending in a dot or a space), "Maximum Path Length Limitation"
//! (`MAX_PATH`, and 248 for a directory); the `dunce` crate's
//! documentation (which keeps the prefix of a long path); Git for Windows'
//! documentation of `core.longpaths`; the Rust standard library's
//! documentation of paths on Windows.

use std::path::{Path, PathBuf};

/// The real path of `path`, with every link followed and every `.` and
/// `..` taken out, in the form people write: never `\\?\` on Windows when
/// the usual form names the same place.
pub fn canonical(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    #[cfg(windows)]
    {
        let real = std::fs::canonicalize(path)?;
        Ok(simplified(&real).to_path_buf())
    }
    #[cfg(not(windows))]
    {
        std::fs::canonicalize(path)
    }
}

/// `path` in the usual form when it is a verbatim one that can be written
/// so (see [`canonical`]); on other systems, as it is.
pub fn simplified(path: &Path) -> &Path {
    #[cfg(windows)]
    {
        match path.to_str().and_then(usual_form) {
            Some(usual) => Path::new(usual),
            None => path,
        }
    }
    #[cfg(not(windows))]
    {
        path
    }
}

/// The names Windows keeps for devices: a file of one of these names,
/// with any extension, is the device in the usual form of a path.
const RESERVED_NAMES: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Whether `name`, one name of a path, means the same in the usual form of
/// a Windows path as in the verbatim one: none of the characters a name
/// cannot hold there, not `.` or `..`, not ending in a dot or a space
/// (which the usual form takes off), not a device's name, and not longer
/// than a name can be.
fn plain_windows_name(name: &str) -> bool {
    const FORBIDDEN: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];
    let stem = name.split('.').next().unwrap_or(name);
    let stem = stem.trim_end_matches([' ', '.']);
    !name.is_empty()
        && name != "."
        && name != ".."
        && !name.ends_with(['.', ' '])
        && !name
            .chars()
            .any(|ch| u32::from(ch) < 32 || FORBIDDEN.contains(&ch))
        && !RESERVED_NAMES
            .iter()
            .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        && name.encode_utf16().count() <= 255
}

/// The usual form of `text`, a verbatim path of a drive
/// (`\\?\C:\work\repo` is `C:\work\repo`), when it names the same place
/// in that form, whatever its length; none for any other path, and for a
/// verbatim one with a name the usual form would read otherwise (a
/// device's name, a dot or a space at the end, a `/`). A share
/// (`\\?\UNC\server\share`) is left as it is. Pure text: the same on
/// every system.
pub fn usual_form(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(r"\\?\")?;
    let mut parts = rest.split('\\');
    let drive = parts.next()?;
    let letter = drive.strip_suffix(':')?;
    if letter.len() != 1 || !letter.chars().all(|ch| ch.is_ascii_alphabetic()) {
        return None;
    }
    let names: Vec<&str> = parts.collect();
    // `\\?\C:` alone, or with the root after it.
    let names = match names.as_slice() {
        [] | [""] => &[][..],
        names => names,
    };
    names
        .iter()
        .all(|name| plain_windows_name(name))
        .then_some(rest)
}

/// How long a path is as Windows counts: in UTF-16 units.
pub fn windows_length(path: &str) -> usize {
    path.encode_utf16().count()
}

/// The longest path git for Windows takes without `core.longpaths`, in
/// UTF-16 units: `MAX_PATH` less the NUL that ends a path.
pub const WINDOWS_PATH_LIMIT: usize = 259;

/// The longest folder it makes without `core.longpaths`: a directory
/// leaves room for a short name of 8 and 3 in it (248 less the NUL).
pub const WINDOWS_FOLDER_LIMIT: usize = 247;

/// Whether git for Windows, without `core.longpaths`, refuses to check out
/// a file at `file`, a full path as the system writes it: the path is
/// longer than [`WINDOWS_PATH_LIMIT`], or the folder it is in longer than
/// [`WINDOWS_FOLDER_LIMIT`]. Pure text: the same on every system, so that
/// what would not check out on Windows can be told anywhere.
pub fn beyond_windows_limit(file: &str) -> bool {
    let folder = file
        .rfind(['\\', '/'])
        .and_then(|at| file.get(..at))
        .unwrap_or("");
    windows_length(file) > WINDOWS_PATH_LIMIT || windows_length(folder) > WINDOWS_FOLDER_LIMIT
}

/// Whether `path`, a path as git writes the ones of a repository (`/`
/// between folders), stays inside the folder it is joined to on this
/// system: not absolute, and no step up. On Windows `\` is a separator as
/// well and a drive (`C:`, `C:x`) or a share (`\\host\share`) leads
/// elsewhere, so `..\x` and `C:\x` are refused there as `../x` and `/x`
/// are everywhere. An empty path stays where it is.
pub fn stays_inside(path: &str) -> bool {
    use std::path::Component;
    !path.starts_with('/')
        && !path.split('/').any(|part| part == "..")
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

/// `base` with `path`, a path as git writes the ones of a repository,
/// joined to it part by part: on Windows with `\` between the parts, as
/// programs started with it and people reading it expect.
pub fn join(base: &Path, path: &str) -> PathBuf {
    let mut joined = base.to_path_buf();
    joined.extend(path.split('/').filter(|part| !part.is_empty()));
    joined
}

/// `path`, a full path as git prints it, in the form of the system: on
/// Windows git writes `C:/work/repo`, and the rest of the application and
/// the people using it write `C:\work\repo`. A `/` there is always a
/// separator (no name holds one), so it becomes `\` and the path still
/// names the same place, except in a verbatim path (`\\?\`), where `/` is
/// no separator and which is left as it is. On other systems `path` is
/// left as it is.
pub fn native(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::{OsStrExt, OsStringExt};
        const SLASH: u16 = b'/' as u16;
        const BACKSLASH: u16 = b'\\' as u16;
        let wide: Vec<u16> = path.as_os_str().encode_wide().collect();
        let verbatim = wide.starts_with(&[BACKSLASH, BACKSLASH, u16::from(b'?'), BACKSLASH]);
        if verbatim || !wide.contains(&SLASH) {
            return path;
        }
        let wide: Vec<u16> = wide
            .into_iter()
            .map(|unit| if unit == SLASH { BACKSLASH } else { unit })
            .collect();
        PathBuf::from(std::ffi::OsString::from_wide(&wide))
    }
    #[cfg(not(windows))]
    {
        path
    }
}

/// The user's home folder: `HOME`, and where that is not set (Windows,
/// outside a shell that sets it) `USERPROFILE`. Empty values are no
/// answer.
pub fn home() -> Option<PathBuf> {
    ["HOME", "USERPROFILE"]
        .into_iter()
        .filter_map(std::env::var_os)
        .find(|value| !value.is_empty())
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_canonical_path_is_absolute_and_never_verbatim() {
        let here = canonical(".").unwrap();
        assert!(here.is_absolute(), "{}", here.display());
        assert!(
            !here.to_string_lossy().starts_with(r"\\?\"),
            "{}",
            here.display()
        );
        assert_eq!(canonical(&here).unwrap(), here);
    }

    #[cfg(windows)]
    #[test]
    fn a_verbatim_path_is_simplified_only_when_it_names_the_same_place() {
        assert_eq!(
            simplified(Path::new(r"\\?\C:\work\repo")),
            Path::new(r"C:\work\repo")
        );
        // A share is left as it is: its usual form is no better a working
        // folder (the command prompt takes neither).
        assert_eq!(
            simplified(Path::new(r"\\?\UNC\server\share\repo")),
            Path::new(r"\\?\UNC\server\share\repo")
        );
        // A reserved name is a device in the usual form: it stays verbatim.
        assert_eq!(
            simplified(Path::new(r"\\?\C:\work\nul")),
            Path::new(r"\\?\C:\work\nul")
        );
        assert_eq!(simplified(Path::new(r"C:\work")), Path::new(r"C:\work"));
        // A long path loses the prefix too: the standard library puts it
        // back where it hands the path to the system.
        let deep = format!(r"C:\{}", vec!["a".repeat(60); 6].join(r"\"));
        assert_eq!(
            simplified(Path::new(&format!(r"\\?\{deep}"))),
            Path::new(&deep)
        );
    }

    #[test]
    fn a_verbatim_path_has_its_usual_form_whatever_its_length() {
        assert_eq!(usual_form(r"\\?\C:\work\repo"), Some(r"C:\work\repo"));
        assert_eq!(usual_form(r"\\?\d:\"), Some(r"d:\"));
        assert_eq!(usual_form(r"\\?\C:"), Some("C:"));
        // Long: the standard library makes it verbatim again by itself.
        let deep = format!(r"\\?\C:\{}", vec!["a".repeat(60); 6].join(r"\"));
        assert!(windows_length(&deep) > 300);
        assert_eq!(usual_form(&deep), deep.strip_prefix(r"\\?\"));
        // Where the usual form would read otherwise, or name a share, or
        // is no verbatim path: none.
        for kept in [
            r"\\?\C:\work\nul",
            r"\\?\C:\work\NUL.tar.gz",
            r"\\?\C:\work\lpt0",
            r"\\?\C:\work\COM².log",
            r"\\?\C:\work\com1 .txt",
            r"\\?\C:\work\dot.",
            r"\\?\C:\work\space ",
            r"\\?\C:\work\a/b",
            r"\\?\C:\work\..\x",
            r"\\?\C:\work\.\x",
            r"\\?\C:\work\\x",
            r"\\?\C:\work\a:b",
            r"\\?\C:\work\a?b",
            r"\\?\UNC\server\share\repo",
            r"\\?\Volume{0}\x",
            r"\\?\CC:\x",
            r"C:\work",
            r"\\server\share",
            "/home/me",
            "",
        ] {
            assert_eq!(usual_form(kept), None, "{kept}");
        }
        let long_name = format!(r"\\?\C:\{}", "n".repeat(256));
        assert_eq!(usual_form(&long_name), None);
        // Names that only look like a device's are names.
        for name in ["console", "nul_", "com10", "lpt", "auxiliary.txt", ".con"] {
            let path = format!(r"\\?\C:\work\{name}");
            assert!(usual_form(&path).is_some(), "{path}");
        }
    }

    #[test]
    fn what_is_beyond_the_limit_of_windows_is_told_by_text() {
        let root = r"C:\work\repo";
        let at = |rel: &str| format!(r"{root}\{rel}");
        assert!(!beyond_windows_limit(&at("a.txt")));
        // 259 units in all is the most a file can have.
        let file = at(&"f".repeat(WINDOWS_PATH_LIMIT - root.len() - 1));
        assert_eq!(windows_length(&file), WINDOWS_PATH_LIMIT);
        assert!(!beyond_windows_limit(&file));
        assert!(beyond_windows_limit(&format!("{file}x")));
        // The folder a file is in: 247 at most.
        let folder = at(&"d".repeat(WINDOWS_FOLDER_LIMIT - root.len() - 1));
        assert!(!beyond_windows_limit(&format!(r"{folder}\x")));
        assert!(beyond_windows_limit(&format!(r"{folder}d\x")));
        // Counted as Windows counts: a character beyond the first plane is
        // two units, and `/` divides as `\` does.
        assert_eq!(windows_length("🦑"), 2);
        let wide = format!("{root}/{}", "🦑".repeat(124));
        assert_eq!(windows_length(&wide), root.len() + 1 + 248);
        assert!(beyond_windows_limit(&wide));
        assert!(!beyond_windows_limit(""));
    }

    #[test]
    fn a_path_that_stays_inside_has_no_root_and_no_step_up() {
        for inside in [
            "", "a", "a/b", "./a", "a/./b", "a..b/c", "...", "a/b..", ".hidden",
        ] {
            assert!(stays_inside(inside), "{inside:?}");
        }
        for outside in ["/", "/a", "..", "../a", "a/..", "a/../b", "a/../../b"] {
            assert!(!stays_inside(outside), "{outside:?}");
        }
        // A separator, a drive and a share of Windows lead out there; on
        // other systems they are characters of a name.
        for windows in [
            r"..\a",
            r"a\..\..\b",
            r"\a",
            r"C:\a",
            "C:a",
            "C:",
            r"\\host\share\a",
        ] {
            assert_eq!(stays_inside(windows), !cfg!(windows), "{windows:?}");
        }
    }

    #[test]
    fn a_repository_path_is_joined_with_the_systems_separator() {
        let base = Path::new("top");
        assert_eq!(
            join(base, "a/b/c.txt"),
            base.join("a").join("b").join("c.txt")
        );
        assert_eq!(join(base, "a.txt"), base.join("a.txt"));
        assert_eq!(join(base, ""), base);
        assert_eq!(join(base, "a//b/"), base.join("a").join("b"));
    }

    #[test]
    fn a_path_as_git_prints_it_is_written_as_the_system_writes_it() {
        let native = |text: &str| native(PathBuf::from(text));
        if cfg!(windows) {
            assert_eq!(native("C:/work/repo"), Path::new(r"C:\work\repo"));
            assert_eq!(native("C:/work\\a b/c"), Path::new(r"C:\work\a b\c"));
            assert_eq!(native("//host/share/x"), Path::new(r"\\host\share\x"));
            // In a verbatim path `/` is no separator: it is left alone.
            assert_eq!(native(r"\\?\C:\a/b"), Path::new(r"\\?\C:\a/b"));
        } else {
            for text in ["/work/repo", "/work/a\\b", "/work//x/"] {
                assert_eq!(native(text).as_os_str(), text);
            }
        }
        assert_eq!(native(r"C:\work\repo"), Path::new(r"C:\work\repo"));
    }

    #[test]
    fn what_is_not_there_has_no_canonical_path() {
        assert!(canonical("no/such/place/at/all").is_err());
    }
}
