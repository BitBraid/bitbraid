//! Diff and merge tools people use, with a command line for each, and
//! which of them are on this machine.
//!
//! The command lines are written here from each tool's documentation of
//! its own command line, not from git's tool definitions, in the form git
//! documents for tool commands: `$LOCAL` and `$REMOTE` are the two
//! versions, and for a merge `$BASE` is what both started from and
//! `$MERGED` the file to write. They are what the preferences' diff and
//! merge commands are filled with when a tool is chosen; a tool is found
//! by looking for its program on `PATH`, and where its installer puts
//! nothing there, where it installs it: on macOS in the application itself
//! (in `/Applications` or `~/Applications`), on Windows in the folder its
//! installer makes under the Program Files folders or the user's
//! `Programs` (for the one that comes as a winget package, among winget's
//! packages). The command lines then name the program by its place.
//! macOS's `/usr/bin/opendiff`, which stands in for Xcode's as
//! `/usr/bin/git` does for git, counts only where Xcode is there
//! ([`crate::stand_in`]). A test holds every command to what a tool
//! command has to do, with a script in the program's place: the right
//! program is started and handed the two versions.
//!
//! Started for real on Linux, through the engine's `tool.diff` and
//! `tool.merge` with git 2.56, in a compositor of their own (an opt-in
//! test, `known_tools_live` in the engine's suite), Meld 3.24, KDiff3 1.12,
//! Visual Studio Code 1.139, VSCodium 1.140, Beyond Compare 5.2, Sublime
//! Merge (build 2132), P4Merge 2026.3, Kompare 26.08, Diffuse 0.11, TkDiff
//! 5.7 and GVim 9.2 show both versions, and merge with the sides, the base
//! where they take one, and the file to write. Meld is given
//! `--auto-merge`: without it the file it writes starts from the base, and
//! a change of one side only that is not picked by hand is lost. TkDiff 6.0
//! takes no ancestor (it hands `-a` on to diff and counts the file as a
//! third one) and refuses the merge; 5.7 takes it. IntelliJ IDEA stops at
//! its user agreement on a profile that has not accepted it; with the
//! agreement accepted in a profile of the test's (2026.2.3), `idea diff`
//! shows the two versions and is left open, `idea merge` shows ours, the
//! result starting from the base and theirs, is waited for and ended by
//! Stop, and its "Apply Changes" writes the result and ends it, which is
//! read back as resolved; it does not save without a person. FileMerge and
//! WinMerge are not on Linux.
//!
//! On macOS (26.7, Apple's git 2.50) FileMerge (Xcode 26), Meld 3.22,
//! KDiff3 1.12.6, Visual Studio Code 1.142, VSCodium 1.140, Beyond Compare
//! 5.2 (`bcomp`), Sublime Merge (build 2132), P4Merge 2026.3, TkDiff 5.7
//! and MacVim 9.2 (`gvim`) do the same, with the same command lines;
//! KDiff3, Beyond Compare and MacVim save without a person. FileMerge is
//! started by `opendiff` in a process group of its own, so Stop ends the
//! wait and leaves its window, as an editor's own instance is left. TkDiff
//! 6.0 shows the two versions there too and refuses the base as on Linux.
//! IntelliJ IDEA is not tried there, for its user agreement; Diffuse and
//! Kompare have no build for macOS.
//!
//! On Windows (10, Git for Windows 2.55) WinMerge 2.16, KDiff3 1.12,
//! Meld 3.24, Visual Studio Code 1.142, VSCodium 1.140, Beyond Compare
//! 5.2 (`BComp.exe`), Sublime Merge (build 2132), P4Merge 2026.3 and
//! TortoiseGitMerge 2.19 are started for real the same way
//! (`known_tools_live`). WinMerge merges with the base in its middle pane
//! and the result written to `$MERGED` (`-o`). TortoiseGitMerge takes
//! each file after its switch as a word of its own (`-base <file>`): the
//! form its manual gives, `/base:<file>`, is a list of paths to the sh of
//! Git for Windows, which hands TortoiseGitMerge `C:\Program
//! Files\Git\base;<file>` for a temporary file of git-mergetool, and
//! `-base:<file>` it does not take. Visual Studio Code's and VSCodium's
//! command is the `.cmd` beside their program, which git's sh runs.
//!
//! Sources: git-difftool(1) and git-mergetool(1) (the variables); the
//! public documentation of each tool's command line: the Meld manual
//! (`--output`, `--auto-merge`), the KDiff3 handbook ("Command-Line
//! Options", `-o`), the Visual Studio Code documentation ("Command Line
//! Interface": `--diff`, `--merge`, `--wait`), the Beyond Compare
//! documentation ("Command Line Reference"; "Using Beyond Compare with
//! Version Control Systems": `BComp.exe` on Windows), the Vim manual
//! (`-d`, `-f`), the Kompare handbook, the Diffuse manual, the P4Merge
//! user guide (launching from the command line), the JetBrains
//! documentation ("Command-line interface": `diff`, `merge`), the Sublime
//! Merge documentation (`smerge mergetool`), the TkDiff manual (`-a`,
//! `-o`), Apple's opendiff(1), the WinMerge manual ("Command line": a
//! middle path, `/o`, `/am`, `/e`, `/u`), the TortoiseGitMerge manual
//! ("Command Line Switches": `base`, `mine`, `theirs`, `merged`), the
//! MSYS2 documentation ("Filesystem Paths": which arguments are
//! converted), the installers' default folders as they install, and
//! winget's documentation of portable packages
//! (`%LOCALAPPDATA%\Microsoft\WinGet\Packages`).

use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
};

use serde::Serialize;

/// A tool that is known by name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct KnownTool {
    /// What it is called.
    pub name: &'static str,
    /// The program that is looked for.
    pub program: &'static str,
    /// The command that shows two versions of a file; none when the tool
    /// only merges.
    pub diff: Option<&'static str>,
    /// The command that resolves a file in conflict; none when the tool
    /// only compares.
    pub merge: Option<&'static str>,
    /// On macOS, where its program is inside its application, from the
    /// folder of applications, for when the program is not on `PATH`
    /// (an application dragged into place puts nothing there).
    #[serde(skip)]
    pub apps: &'static [&'static str],
    /// On Windows, where its installer puts its program, from one of the
    /// folders programs are installed in ([`windows_folders`]), for when
    /// it is not on `PATH`. A part that ends in `*` stands for any folder
    /// whose name begins with what comes before it (a version).
    #[serde(skip)]
    pub windows: &'static [&'static str],
}

/// The tools that are known, in the order they are offered.
pub const KNOWN: &[KnownTool] = &[
    KnownTool {
        name: "Meld",
        program: "meld",
        diff: Some("meld \"$LOCAL\" \"$REMOTE\""),
        merge: Some("meld --auto-merge \"$LOCAL\" \"$BASE\" \"$REMOTE\" --output \"$MERGED\""),
        apps: &["Meld.app/Contents/MacOS/Meld"],
        windows: &["Meld/Meld.exe"],
    },
    KnownTool {
        name: "KDiff3",
        program: "kdiff3",
        diff: Some("kdiff3 \"$LOCAL\" \"$REMOTE\""),
        merge: Some("kdiff3 \"$BASE\" \"$LOCAL\" \"$REMOTE\" -o \"$MERGED\""),
        apps: &["kdiff3.app/Contents/MacOS/kdiff3"],
        windows: &["KDiff3/bin/kdiff3.exe", "KDiff3/kdiff3.exe"],
    },
    KnownTool {
        name: "Visual Studio Code",
        program: "code",
        diff: Some("code --wait --diff \"$LOCAL\" \"$REMOTE\""),
        merge: Some("code --wait --merge \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\""),
        apps: &["Visual Studio Code.app/Contents/Resources/app/bin/code"],
        windows: &["Microsoft VS Code/bin/code.cmd"],
    },
    KnownTool {
        name: "VSCodium",
        program: "codium",
        diff: Some("codium --wait --diff \"$LOCAL\" \"$REMOTE\""),
        merge: Some("codium --wait --merge \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\""),
        apps: &["VSCodium.app/Contents/Resources/app/bin/codium"],
        windows: &["VSCodium/bin/codium.cmd"],
    },
    KnownTool {
        name: "Beyond Compare",
        program: "bcompare",
        diff: Some("bcompare \"$LOCAL\" \"$REMOTE\""),
        merge: Some("bcompare \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\""),
        apps: &["Beyond Compare.app/Contents/MacOS/bcomp"],
        windows: &["Beyond Compare 5/BComp.exe", "Beyond Compare 4/BComp.exe"],
    },
    KnownTool {
        name: "Sublime Merge",
        program: "smerge",
        diff: None,
        merge: Some("smerge mergetool \"$BASE\" \"$LOCAL\" \"$REMOTE\" -o \"$MERGED\""),
        apps: &["Sublime Merge.app/Contents/SharedSupport/bin/smerge"],
        windows: &[
            "Sublime Merge/smerge.exe",
            "SublimeHQ.SublimeMerge_*/smerge.exe",
        ],
    },
    KnownTool {
        name: "P4Merge",
        program: "p4merge",
        diff: Some("p4merge \"$LOCAL\" \"$REMOTE\""),
        merge: Some("p4merge \"$BASE\" \"$LOCAL\" \"$REMOTE\" \"$MERGED\""),
        apps: &["p4merge.app/Contents/MacOS/p4merge"],
        windows: &["Perforce/p4merge.exe"],
    },
    KnownTool {
        name: "IntelliJ IDEA",
        program: "idea",
        diff: Some("idea diff \"$LOCAL\" \"$REMOTE\""),
        merge: Some("idea merge \"$LOCAL\" \"$REMOTE\" \"$BASE\" \"$MERGED\""),
        apps: &[
            "IntelliJ IDEA.app/Contents/MacOS/idea",
            "IntelliJ IDEA CE.app/Contents/MacOS/idea",
            "IntelliJ IDEA Ultimate.app/Contents/MacOS/idea",
        ],
        windows: &["JetBrains/IntelliJ IDEA*/bin/idea64.exe"],
    },
    KnownTool {
        name: "Kompare",
        program: "kompare",
        diff: Some("kompare \"$LOCAL\" \"$REMOTE\""),
        merge: None,
        apps: &[],
        windows: &[],
    },
    KnownTool {
        name: "Diffuse",
        program: "diffuse",
        diff: Some("diffuse \"$LOCAL\" \"$REMOTE\""),
        merge: Some("diffuse \"$LOCAL\" \"$MERGED\" \"$REMOTE\""),
        apps: &[],
        windows: &[],
    },
    KnownTool {
        name: "TkDiff",
        program: "tkdiff",
        diff: Some("tkdiff \"$LOCAL\" \"$REMOTE\""),
        merge: Some("tkdiff -a \"$BASE\" -o \"$MERGED\" \"$LOCAL\" \"$REMOTE\""),
        apps: &[],
        windows: &[],
    },
    KnownTool {
        name: "GVim",
        program: "gvim",
        diff: Some("gvim -f -d \"$LOCAL\" \"$REMOTE\""),
        merge: Some("gvim -f -d \"$LOCAL\" \"$MERGED\" \"$REMOTE\""),
        apps: &["MacVim.app/Contents/bin/gvim"],
        windows: &["Vim/vim*/gvim.exe"],
    },
    KnownTool {
        name: "FileMerge",
        program: "opendiff",
        diff: Some("opendiff \"$LOCAL\" \"$REMOTE\""),
        merge: Some("opendiff \"$LOCAL\" \"$REMOTE\" -ancestor \"$BASE\" -merge \"$MERGED\""),
        apps: &[],
        windows: &[],
    },
    KnownTool {
        name: "WinMerge",
        program: "WinMergeU",
        diff: Some("WinMergeU -e -u \"$LOCAL\" \"$REMOTE\""),
        merge: Some("WinMergeU -e -u -fm -am \"$LOCAL\" \"$BASE\" \"$REMOTE\" -o \"$MERGED\""),
        apps: &[],
        windows: &["WinMerge/WinMergeU.exe"],
    },
    KnownTool {
        name: "TortoiseGitMerge",
        program: "TortoiseGitMerge",
        diff: Some("TortoiseGitMerge -base \"$LOCAL\" -mine \"$REMOTE\""),
        merge: Some(
            "TortoiseGitMerge -base \"$BASE\" -mine \"$LOCAL\" -theirs \"$REMOTE\" -merged \"$MERGED\"",
        ),
        apps: &[],
        windows: &["TortoiseGit/bin/TortoiseGitMerge.exe"],
    },
];

/// A known tool that is on this machine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FoundTool {
    #[serde(skip)]
    pub tool: KnownTool,
    /// What it is called.
    pub name: &'static str,
    /// The program that was looked for.
    pub program: &'static str,
    /// Its command that shows two versions: the tool's, with the program
    /// named by its place where it is not on `PATH`.
    pub diff: Option<String>,
    /// Its command that resolves a file in conflict, the same way.
    pub merge: Option<String>,
    /// Where its program is.
    #[serde(serialize_with = "crate::activity::lossy")]
    pub path: PathBuf,
}

impl FoundTool {
    /// The tool, its program found on `PATH` (run by its name) or in its
    /// application (run by its place).
    fn new(tool: &KnownTool, path: PathBuf, by_place: bool) -> Self {
        let line = |line: &'static str| {
            if by_place {
                at_place(line, tool.program, &path)
            } else {
                line.to_owned()
            }
        };
        Self {
            tool: *tool,
            name: tool.name,
            program: tool.program,
            diff: tool.diff.map(line),
            merge: tool.merge.map(line),
            path: path.clone(),
        }
    }
}

/// A command line with its program, the first word, named by its place,
/// quoted for the shell git runs it with.
fn at_place(line: &str, program: &str, path: &Path) -> String {
    let quoted = format!("'{}'", path.display().to_string().replace('\'', "'\\''"));
    match line.strip_prefix(program) {
        Some(rest) => format!("{quoted}{rest}"),
        None => line.to_owned(),
    }
}

/// Whether a file is a program that can be run.
#[cfg(unix)]
fn runs(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::metadata(path)
        .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn runs(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|meta| meta.is_file())
}

/// Where a program is among the directories of a `PATH`.
pub fn find_in(program: &str, path: &OsStr) -> Option<PathBuf> {
    // A name, not a place: nothing with a separator in it is looked up.
    if program.is_empty() || program.contains(['/', '\\']) {
        return None;
    }
    let names: Vec<String> = if cfg!(windows) {
        vec![format!("{program}.exe"), program.to_owned()]
    } else {
        vec![program.to_owned()]
    };
    std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .flat_map(|dir| {
            names
                .iter()
                .map(move |name| dir.join(name))
                .collect::<Vec<_>>()
        })
        // macOS's stand-in for Xcode's program, with no Xcode behind it,
        // would offer to install the tools: not a program here.
        .find(|candidate| runs(candidate) && !crate::stand_in::is_idle_tool(candidate, program))
}

/// The known tools whose program is in one of the directories of `path`,
/// else in its application in one of the folders `applications` (macOS).
pub fn found_in_places(path: &OsStr, applications: &[PathBuf]) -> Vec<FoundTool> {
    found_in_folders(path, applications, |tool| tool.apps)
}

/// The known tools whose program is in one of the directories of `path`,
/// else where its installer puts it under one of `folders` (Windows).
pub fn found_in_windows_folders(path: &OsStr, folders: &[PathBuf]) -> Vec<FoundTool> {
    found_in_folders(path, folders, |tool| tool.windows)
}

fn found_in_folders(
    path: &OsStr,
    folders: &[PathBuf],
    places: impl Fn(&KnownTool) -> &'static [&'static str],
) -> Vec<FoundTool> {
    KNOWN
        .iter()
        .filter_map(|tool| {
            if let Some(found) = find_in(tool.program, path) {
                return Some(FoundTool::new(tool, found, false));
            }
            let found = folders
                .iter()
                .flat_map(|folder| {
                    places(tool)
                        .iter()
                        .filter_map(move |place| placed(folder, place))
                })
                .find(|candidate| runs(candidate))?;
            Some(FoundTool::new(tool, found, true))
        })
        .collect()
}

/// `place` under `folder`, a part of it that ends in `*` taken for the
/// first folder there (by name, the last) whose name begins with what
/// comes before the `*`. None where there is none.
fn placed(folder: &Path, place: &str) -> Option<PathBuf> {
    let mut at = folder.to_path_buf();
    for part in place.split('/') {
        match part.strip_suffix('*') {
            Some(begins) => {
                let mut names: Vec<String> = std::fs::read_dir(&at)
                    .ok()?
                    .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                    .filter(|name| name.starts_with(begins))
                    .collect();
                names.sort();
                at = at.join(names.pop()?);
            }
            None => at = at.join(part),
        }
    }
    Some(at)
}

/// The folders applications are in: on macOS the system's and the
/// user's, elsewhere none.
fn applications() -> Vec<PathBuf> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    let mut folders = vec![PathBuf::from("/Applications")];
    if let Some(home) = bitbraid_core::process::var_os("HOME").filter(|home| !home.is_empty()) {
        folders.push(Path::new(&home).join("Applications"));
    }
    folders
}

/// The folders programs are installed in on Windows: the Program Files
/// folders, the user's `Programs` and winget's packages; elsewhere none.
pub fn windows_folders() -> Vec<PathBuf> {
    if !cfg!(windows) {
        return Vec::new();
    }
    let var = |name: &str| {
        bitbraid_core::process::var_os(name)
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
    };
    let mut folders: Vec<PathBuf> = ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .into_iter()
        .filter_map(var)
        .collect();
    if let Some(local) = var("LOCALAPPDATA") {
        folders.push(local.join("Programs"));
        folders.push(local.join("Microsoft").join("WinGet").join("Packages"));
    }
    folders.dedup();
    folders
}

/// The known tools that are on this machine.
pub fn found() -> Vec<FoundTool> {
    let path = bitbraid_core::process::var_os("PATH").unwrap_or_default();
    if cfg!(windows) {
        found_in_windows_folders(&path, &windows_folders())
    } else {
        found_in_places(&path, &applications())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The known tools whose program is in one of the directories of `path`.
    fn found_in(path: &OsStr) -> Vec<FoundTool> {
        found_in_places(path, &[])
    }

    #[test]
    fn every_known_tool_has_a_command_that_names_its_files() {
        let mut names = std::collections::BTreeSet::new();
        for tool in KNOWN {
            assert!(names.insert(tool.name), "{} twice", tool.name);
            assert!(tool.diff.is_some() || tool.merge.is_some(), "{}", tool.name);
            for command in [tool.diff, tool.merge].into_iter().flatten() {
                // The command starts the tool's own program, on one line.
                assert_eq!(command.split(' ').next(), Some(tool.program), "{command}");
                assert!(!command.contains('\n'));
            }
            if let Some(diff) = tool.diff {
                assert!(
                    diff.contains("\"$LOCAL\"") && diff.contains("\"$REMOTE\""),
                    "{diff}"
                );
                assert!(!diff.contains("$MERGED"), "{diff}");
            }
            if let Some(merge) = tool.merge {
                for variable in ["\"$LOCAL\"", "\"$REMOTE\"", "\"$MERGED\""] {
                    assert!(merge.contains(variable), "{merge} lacks {variable}");
                }
            }
        }
    }

    /// On Windows a tool its installer put under Program Files or the
    /// user's `Programs` has nothing on `PATH`: it is found there, a
    /// version's folder by its beginning, and run from there by git's sh,
    /// a `.cmd` too.
    #[test]
    fn a_tool_is_found_where_its_windows_installer_puts_it() {
        let dir = std::env::temp_dir().join(format!("bitbraid-win-tools-{}", std::process::id()));
        let (programs, user) = (dir.join("Program Files"), dir.join("it's Programs"));
        let file = |path: &Path| {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
        };
        file(&programs.join("WinMerge/WinMergeU.exe"));
        file(&user.join("Microsoft VS Code/bin/code.cmd"));
        file(&programs.join("JetBrains/IntelliJ IDEA 2025.1/bin/idea64.exe"));
        file(&programs.join("JetBrains/IntelliJ IDEA 2026.2/bin/idea64.exe"));
        // There, and not a program.
        std::fs::create_dir_all(programs.join("Perforce/p4merge.exe")).unwrap();
        let found = found_in_windows_folders(
            OsStr::new(""),
            &[dir.join("none"), programs.clone(), user.clone()],
        );
        let names: Vec<&str> = found.iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["Visual Studio Code", "IntelliJ IDEA", "WinMerge"]);
        assert_eq!(found[0].path, user.join("Microsoft VS Code/bin/code.cmd"));
        // The latest version of those there.
        assert_eq!(
            found[1].path,
            programs.join("JetBrains/IntelliJ IDEA 2026.2/bin/idea64.exe")
        );
        let merge = found[2].merge.clone().unwrap();
        assert!(merge.starts_with('\''), "{merge}");
        assert!(merge.ends_with(" -o \"$MERGED\""), "{merge}");
        assert_eq!(placed(&programs, "Nothing*/x.exe"), None);
        assert_eq!(placed(&dir.join("none"), "a*/b"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn a_tool_is_found_where_its_program_can_be_run() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("bitbraid-tools-{}", std::process::id()));
        let (first, second) = (dir.join("first"), dir.join("second"));
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let program = |at: &Path, name: &str, mode: u32| {
            let file = at.join(name);
            std::fs::write(&file, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(mode)).unwrap();
        };
        program(&second, "meld", 0o755);
        program(&first, "kdiff3", 0o644); // there, and not a program
        program(&second, "kdiff3", 0o755);
        std::fs::create_dir_all(first.join("code")).unwrap(); // a folder of that name
        let path = std::env::join_paths([first.as_path(), second.as_path(), Path::new("relative")])
            .unwrap();
        let found = found_in(&path);
        let names: Vec<&str> = found.iter().map(|tool| tool.tool.name).collect();
        assert_eq!(names, ["Meld", "KDiff3"]);
        assert_eq!(found[0].path, second.join("meld"));
        assert_eq!(found[1].path, second.join("kdiff3"));
        assert_eq!(find_in("../meld", &path), None);
        assert_eq!(find_in("", &path), None);
        assert!(found_in(OsStr::new("")).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On macOS a tool whose application was dragged into place has
    /// nothing on `PATH`: it is found in its application, and its command
    /// lines name the program there, quoted, so that git's shell runs it.
    #[cfg(unix)]
    #[test]
    fn a_tool_is_found_in_its_application_and_run_from_there() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = std::env::temp_dir().join(format!("bitbraid-apps-{}", std::process::id()));
        let (bin, apps, said) = (dir.join("bin"), dir.join("it's apps"), dir.join("said"));
        let program = |file: &Path| {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(
                file,
                format!(
                    "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$@\" > '{}'\n",
                    said.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        };
        program(&bin.join("kdiff3"));
        program(&apps.join("Meld.app/Contents/MacOS/Meld"));
        program(&apps.join("kdiff3.app/Contents/MacOS/kdiff3"));
        program(&apps.join("Beyond Compare.app/Contents/MacOS/bcomp"));
        // There, and not a program.
        std::fs::create_dir_all(apps.join("p4merge.app/Contents/MacOS/p4merge")).unwrap();
        let path = std::env::join_paths([bin.as_path()]).unwrap();
        let found = found_in_places(&path, &[dir.join("none"), apps.clone()]);
        let names: Vec<&str> = found.iter().map(|tool| tool.name).collect();
        assert_eq!(names, ["Meld", "KDiff3", "Beyond Compare"]);
        // On PATH: by name.
        assert_eq!(found[1].path, bin.join("kdiff3"));
        assert_eq!(found[1].diff.as_deref(), KNOWN[1].diff);
        // In an application: by its place, quoted.
        let meld = apps.join("Meld.app/Contents/MacOS/Meld");
        assert_eq!(found[0].path, meld);
        let diff = found[0].diff.clone().unwrap();
        assert!(diff.starts_with("'"), "{diff}");
        assert!(diff.ends_with(" \"$LOCAL\" \"$REMOTE\""), "{diff}");
        // Run as git runs it: the program in the application, with both.
        for (tool, wanted) in [
            (&found[0], &meld),
            (
                &found[2],
                &apps.join("Beyond Compare.app/Contents/MacOS/bcomp"),
            ),
        ] {
            let line = tool.diff.clone().unwrap();
            let ran = std::process::Command::new("sh")
                .args(["-c", &line])
                .env("LOCAL", "one file")
                .env("REMOTE", "the other")
                .status()
                .unwrap();
            assert!(ran.success(), "{line}");
            assert_eq!(
                std::fs::read_to_string(&said).unwrap(),
                format!("{}\none file\nthe other\n", wanted.display())
            );
        }
        // What is said of it is what the preferences read.
        let json = serde_json::to_value(&found[0]).unwrap();
        assert_eq!(json["name"], "Meld");
        assert_eq!(json["diff"], diff.as_str());
        assert!(json.get("apps").is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
