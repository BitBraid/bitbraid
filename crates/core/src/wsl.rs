//! Repositories inside a WSL distribution, as Windows sees them.
//!
//! Windows reaches the files of a WSL distribution through a share named
//! after it: `\\wsl.localhost\Ubuntu\home\me\repo`, or in the older form
//! `\\wsl$\Ubuntu\home\me\repo`, which is `/home/me/repo` inside. A
//! repository there is the distribution's: its hooks, filters, credential
//! helpers and configuration are programs and files of that Linux, its
//! files have the Linux executable bit Windows cannot see, and Git for
//! Windows working through the share is slow and takes the files for
//! somebody else's (`safe.directory`). So git is run inside the
//! distribution for it: `wsl.exe --distribution Ubuntu --cd /home/me/repo
//! --exec git …`, with no shell in between.
//!
//! What git is handed then has to be what the distribution sees: a path of
//! the Windows side (`C:\Users\me\AppData\Local\Temp\msg`) is
//! `/mnt/c/Users/me/AppData/Local/Temp/msg` there, a path of the same
//! share is the Linux path it stands for, and a variable of the
//! environment only crosses into the distribution when `WSLENV` names it,
//! with `/p` for one that holds a path and `/l` for a list of paths.
//!
//! Everything here is text, the same on every system, so that it is tested
//! anywhere; only Windows ever starts what it makes.
//!
//! Sources: Microsoft Learn, "Working across Windows and Linux file
//! systems" (`\\wsl$`, `\\wsl.localhost`, `/mnt/<drive>`), "Basic commands
//! for WSL" (`--distribution`, `--cd`, `--exec`), "Share environment
//! variables between WSL and Windows" (`WSLENV`, its `/p`, `/l`, `/u` and
//! `/w` flags), "Advanced settings configuration in WSL" (`[automount]`
//! `root`, `/mnt/` by default); git-config(1) (`safe.directory`).

use std::ffi::{OsStr, OsString};

/// The names Windows gives the share of the distributions, either case.
const HOSTS: [&str; 2] = ["wsl$", "wsl.localhost"];

/// Where the drives of Windows are inside a distribution, unless its
/// `wsl.conf` says otherwise.
pub const DRIVES_ROOT: &str = "/mnt/";

/// The program that starts a command inside a distribution.
pub const LAUNCHER: &str = "wsl.exe";

/// A place inside a WSL distribution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WslPath {
    /// The distribution, as `wsl.exe --list` names it.
    pub distribution: String,
    /// The place inside it, a Linux path from `/`.
    pub linux: String,
}

/// Whether `name` can be a distribution's: letters, digits, `.`, `_` and
/// `-`, as WSL takes them, and not `.` or `..`.
fn distribution_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '-'))
}

impl WslPath {
    /// How many arguments [`WslPath::git_invocation`] gives.
    pub const INVOCATION_LENGTH: usize = 6;

    /// The place inside a distribution that `text`, a Windows path, names:
    /// `\\wsl.localhost\<distribution>\…` or `\\wsl$\<distribution>\…`,
    /// in either case, with `\` or `/` between the names, and in the
    /// verbatim form (`\\?\UNC\wsl$\…`). None for any other path, and for
    /// one with a step up or a name that is not one (`..`, `.`, an empty
    /// one between two separators).
    pub fn parse(text: &str) -> Option<Self> {
        let unc = text
            .strip_prefix(r"\\?\UNC\")
            .or_else(|| text.strip_prefix(r"\\"))
            .or_else(|| text.strip_prefix("//"))?;
        let mut parts = unc.split(['\\', '/']);
        let host = parts.next()?;
        if !HOSTS.iter().any(|known| host.eq_ignore_ascii_case(known)) {
            return None;
        }
        let distribution = parts.next()?;
        if !distribution_name(distribution) {
            return None;
        }
        let names: Vec<&str> = parts.collect();
        // A separator at the end leaves an empty name: the folder itself.
        let names = match names.split_last() {
            Some((&"", before)) => before,
            _ => &names[..],
        };
        if names
            .iter()
            .any(|name| name.is_empty() || *name == "." || *name == "..")
        {
            return None;
        }
        Some(Self {
            distribution: distribution.to_owned(),
            linux: format!("/{}", names.join("/")),
        })
    }

    /// [`WslPath::parse`] for a path of the system, which may not be text.
    pub fn of(path: &std::path::Path) -> Option<Self> {
        Self::parse(path.to_str()?)
    }

    /// The folder this place is in, inside the distribution; none for `/`.
    pub fn linux_parent(&self) -> Option<String> {
        if self.linux == "/" {
            return None;
        }
        let at = self.linux.rfind('/')?;
        Some(if at == 0 {
            "/".to_owned()
        } else {
            self.linux.get(..at)?.to_owned()
        })
    }

    /// What `wsl.exe` is given to run git here, before git's own
    /// arguments: the distribution, the folder, and git started without a
    /// shell, so that nothing in an argument is read as the shell's.
    pub fn git_invocation(&self) -> Vec<String> {
        vec![
            "--distribution".to_owned(),
            self.distribution.clone(),
            "--cd".to_owned(),
            self.linux.clone(),
            "--exec".to_owned(),
            "git".to_owned(),
        ]
    }

    /// The distribution and the folder of an invocation made by
    /// [`WslPath::git_invocation`], read back from the arguments of a
    /// command: none for any other.
    pub fn from_invocation<'a>(args: impl IntoIterator<Item = &'a OsStr>) -> Option<Self> {
        let head: Vec<&OsStr> = args.into_iter().take(Self::INVOCATION_LENGTH).collect();
        match head.as_slice() {
            [flag, distribution, cd, linux, exec, git]
                if *flag == "--distribution"
                    && *cd == "--cd"
                    && *exec == "--exec"
                    && *git == "git" =>
            {
                let distribution = distribution.to_str()?;
                let linux = linux.to_str()?;
                (distribution_name(distribution) && linux.starts_with('/')).then(|| Self {
                    distribution: distribution.to_owned(),
                    linux: linux.to_owned(),
                })
            }
            _ => None,
        }
    }
}

/// `text`, an absolute path of the Windows side, as the distribution
/// `distribution` sees it: a drive's path under [`DRIVES_ROOT`] with the
/// drive's letter in lower case, a path of that distribution's share as
/// the Linux path it is. None for anything else: a relative path, a
/// share of another machine or of another distribution (which the
/// distribution does not see), text that is no path.
pub fn linux_view(text: &str, distribution: &str) -> Option<String> {
    if let Some(inside) = WslPath::parse(text) {
        return (inside.distribution.eq_ignore_ascii_case(distribution)).then_some(inside.linux);
    }
    let text = text.strip_prefix(r"\\?\").unwrap_or(text);
    let mut chars = text.chars();
    let letter = chars.next()?;
    if !letter.is_ascii_alphabetic() || chars.next()? != ':' {
        return None;
    }
    let rest = chars.as_str();
    // `C:x` is relative to the drive's own current folder.
    if !rest.is_empty() && !rest.starts_with(['\\', '/']) {
        return None;
    }
    let names: Vec<&str> = rest.split(['\\', '/']).filter(|n| !n.is_empty()).collect();
    // A name of a Windows path holds none of these: what does is no path
    // (two paths in a list, say).
    const FORBIDDEN: &[char] = &['<', '>', ':', '"', '|', '?', '*'];
    if names.iter().any(|name| {
        *name == "."
            || *name == ".."
            || name
                .chars()
                .any(|ch| u32::from(ch) < 32 || FORBIDDEN.contains(&ch))
    }) {
        return None;
    }
    let mut out = format!("{DRIVES_ROOT}{}", letter.to_ascii_lowercase());
    for name in names {
        out.push('/');
        out.push_str(name);
    }
    Some(out)
}

/// One argument of git's for a git that runs inside `distribution`: an
/// absolute path of the Windows side as the distribution sees it, and the
/// value of an option or a setting written `name=value` likewise; any
/// other argument as it is.
pub fn argument(arg: &OsStr, distribution: &str) -> OsString {
    let Some(text) = arg.to_str() else {
        return arg.to_os_string();
    };
    if let Some(linux) = linux_view(text, distribution) {
        return linux.into();
    }
    if let Some((name, value)) = text.split_once('=')
        && !name.is_empty()
        && !name.contains(['\\', '/'])
        && let Some(linux) = linux_view(value, distribution)
    {
        return format!("{name}={linux}").into();
    }
    arg.to_os_string()
}

/// How a variable crosses into the distribution (a flag of `WSLENV`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Crossing {
    /// As it is.
    Plain,
    /// A path, written as the distribution sees it (`/p`).
    Path,
    /// A list of paths, separated as each side does (`/l`).
    PathList,
}

impl Crossing {
    /// How a variable with this value crosses: a path of the Windows side
    /// as a path, anything else as it is.
    pub fn of(value: &str, distribution: &str) -> Self {
        if linux_view(value, distribution).is_some() {
            Self::Path
        } else if value.contains(';')
            && value
                .split(';')
                .filter(|part| !part.is_empty())
                .all(|part| linux_view(part, distribution).is_some())
        {
            Self::PathList
        } else {
            Self::Plain
        }
    }
}

/// What `WSLENV` is to be: what it says already (`existing`), then each
/// of `names` that it does not name yet, with the flag of its crossing.
/// A name that could not be one (empty, or holding `:`, `/` or `=`) is
/// left out.
pub fn wslenv(existing: Option<&str>, names: &[(&str, Crossing)]) -> String {
    let mut out: Vec<String> = existing
        .unwrap_or("")
        .split(':')
        .filter(|entry| !entry.is_empty())
        .map(str::to_owned)
        .collect();
    let named = |out: &[String], name: &str| {
        out.iter()
            .any(|entry| entry.split('/').next() == Some(name))
    };
    for (name, crossing) in names {
        if name.is_empty() || name.contains([':', '/', '=']) || named(&out, name) {
            continue;
        }
        out.push(match crossing {
            Crossing::Plain => (*name).to_owned(),
            Crossing::Path => format!("{name}/p"),
            Crossing::PathList => format!("{name}/l"),
        });
    }
    out.join(":")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(distribution: &str, linux: &str) -> Option<WslPath> {
        Some(WslPath {
            distribution: distribution.to_owned(),
            linux: linux.to_owned(),
        })
    }

    #[test]
    fn a_path_of_the_share_is_a_place_inside_a_distribution() {
        let parse = WslPath::parse;
        assert_eq!(
            parse(r"\\wsl$\Ubuntu\home\me\repo"),
            at("Ubuntu", "/home/me/repo")
        );
        assert_eq!(
            parse(r"\\wsl.localhost\Ubuntu-22.04\home\me"),
            at("Ubuntu-22.04", "/home/me")
        );
        assert_eq!(
            parse(r"\\WSL.LOCALHOST\Debian\srv\x"),
            at("Debian", "/srv/x")
        );
        assert_eq!(
            parse(r"\\?\UNC\wsl$\Ubuntu\home\me\repo"),
            at("Ubuntu", "/home/me/repo")
        );
        assert_eq!(parse(r"\\?\UNC\wsl.localhost\Arch\x"), at("Arch", "/x"));
        assert_eq!(parse("//wsl$/Ubuntu/home/me/"), at("Ubuntu", "/home/me"));
        assert_eq!(parse(r"\\wsl$\Ubuntu"), at("Ubuntu", "/"));
        assert_eq!(parse(r"\\wsl$\Ubuntu\"), at("Ubuntu", "/"));
        assert_eq!(
            parse(r"\\wsl$\Ubuntu\home\with space\naïve"),
            at("Ubuntu", "/home/with space/naïve")
        );
        for other in [
            r"C:\work\repo",
            r"\\server\share\repo",
            r"\\wsl\Ubuntu\home",
            r"\\wsl$",
            r"\\wsl$\",
            r"\\wsl$\..\home",
            r"\\wsl$\Ubu ntu\home",
            r"\\wsl$\Ubuntu\home\..\etc",
            r"\\wsl$\Ubuntu\home\.\me",
            r"\\wsl$\Ubuntu\home\\me",
            r"\\?\C:\work",
            "/home/me/repo",
            "wsl$/Ubuntu/home",
            "",
        ] {
            assert_eq!(parse(other), None, "{other}");
        }
    }

    #[test]
    fn the_folder_above_a_place_inside() {
        let place = WslPath::parse(r"\\wsl$\Ubuntu\home\me").unwrap();
        assert_eq!(place.linux_parent().as_deref(), Some("/home"));
        let top = WslPath::parse(r"\\wsl$\Ubuntu\home").unwrap();
        assert_eq!(top.linux_parent().as_deref(), Some("/"));
        let root = WslPath::parse(r"\\wsl$\Ubuntu").unwrap();
        assert_eq!(root.linux_parent(), None);
    }

    #[test]
    fn git_is_run_inside_without_a_shell_and_read_back() {
        let place = WslPath::parse(r"\\wsl$\Ubuntu\home\me\repo").unwrap();
        let invocation = place.git_invocation();
        assert_eq!(
            invocation,
            [
                "--distribution",
                "Ubuntu",
                "--cd",
                "/home/me/repo",
                "--exec",
                "git"
            ]
        );
        let mut args: Vec<OsString> = invocation.iter().map(OsString::from).collect();
        args.extend(["status".into(), "--porcelain".into()]);
        assert_eq!(
            WslPath::from_invocation(args.iter().map(OsString::as_os_str)),
            Some(place)
        );
        let plain: Vec<OsString> = ["status", "--porcelain"].map(OsString::from).to_vec();
        assert_eq!(
            WslPath::from_invocation(plain.iter().map(OsString::as_os_str)),
            None
        );
        let hostile: Vec<OsString> = ["--distribution", "a b", "--cd", "/x", "--exec", "git"]
            .map(OsString::from)
            .to_vec();
        assert_eq!(
            WslPath::from_invocation(hostile.iter().map(OsString::as_os_str)),
            None
        );
    }

    #[test]
    fn a_windows_path_is_seen_inside_under_the_drives_root() {
        let view = |text: &str| linux_view(text, "Ubuntu");
        assert_eq!(
            view(r"C:\Users\me\AppData\Local\Temp\msg").as_deref(),
            Some("/mnt/c/Users/me/AppData/Local/Temp/msg")
        );
        assert_eq!(view("D:/work/x").as_deref(), Some("/mnt/d/work/x"));
        assert_eq!(
            view(r"\\?\E:\long\path").as_deref(),
            Some("/mnt/e/long/path")
        );
        assert_eq!(view(r"C:\").as_deref(), Some("/mnt/c"));
        assert_eq!(view("C:").as_deref(), Some("/mnt/c"));
        assert_eq!(
            view(r"\\wsl$\ubuntu\home\me").as_deref(),
            Some("/home/me"),
            "the same distribution, whatever the case"
        );
        for not in [
            r"\\wsl$\Debian\home",
            r"\\server\share\x",
            r"relative\path",
            "C:relative",
            r"C:\a\..\b",
            r"C:\a;D:\b",
            r"C:\what?",
            "/home/me",
            "-m",
            "HEAD~1",
            "",
        ] {
            assert_eq!(view(not), None, "{not}");
        }
    }

    #[test]
    fn arguments_that_are_paths_are_written_as_the_distribution_sees_them() {
        let arg = |text: &str| argument(OsStr::new(text), "Ubuntu");
        assert_eq!(arg(r"C:\Temp\message"), "/mnt/c/Temp/message");
        assert_eq!(arg(r"--git-dir=C:\x\.git"), "--git-dir=/mnt/c/x/.git");
        assert_eq!(
            arg(r"core.hooksPath=\\wsl$\Ubuntu\home\me\hooks"),
            "core.hooksPath=/home/me/hooks"
        );
        for kept in [
            "status",
            "--porcelain=v2",
            "HEAD",
            "a=b",
            "src/main.rs",
            "-z",
        ] {
            assert_eq!(arg(kept), kept);
        }
    }

    #[test]
    fn the_environment_crosses_with_the_flags_of_its_values() {
        assert_eq!(
            Crossing::of(r"C:\Program Files\BitBraid\bitbraid.exe", "U"),
            Crossing::Path
        );
        assert_eq!(Crossing::of(r"C:\a;D:\b", "U"), Crossing::PathList);
        assert_eq!(Crossing::of("0", "U"), Crossing::Plain);
        assert_eq!(Crossing::of("true", "U"), Crossing::Plain);
        assert_eq!(Crossing::of("a;C:\\b", "U"), Crossing::Plain);
        assert_eq!(
            wslenv(
                Some("USERPROFILE/p:"),
                &[
                    ("GIT_ASKPASS", Crossing::Path),
                    ("LC_ALL", Crossing::Plain),
                    ("USERPROFILE", Crossing::Plain),
                    ("GIT_CEILING_DIRECTORIES", Crossing::PathList),
                    ("BAD:NAME", Crossing::Plain),
                    ("", Crossing::Plain),
                ]
            ),
            "USERPROFILE/p:GIT_ASKPASS/p:LC_ALL:GIT_CEILING_DIRECTORIES/l"
        );
        assert_eq!(wslenv(None, &[]), "");
    }
}
