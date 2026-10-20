//! The known diff and merge tools started for real, through the engine's
//! `tool.diff` and `tool.merge` with the commands `tool.known` offers, on
//! files of a repository: a diff tool is handed the two versions and left
//! open; a merge tool is handed the sides, the base where it takes one and
//! the file to write, and is waited for; Stop ends it; when it ends,
//! whether it wrote the file is what is read back. A tool that can save
//! without a person does (GVim with `-c` commands, KDiff3 with `--auto`,
//! Beyond Compare with `-automerge`); for the others the file is written
//! while the tool is open, as saving in it would, and the tool is then
//! ended.
//!
//! Opt-in, and never in ci: `BITBRAID_TEST_TOOLS` names a folder that
//! holds the tools' programs under the names `known_tools` looks for
//! (links or small scripts that start them); without it the test says it
//! was skipped. On Linux the windows open wherever the environment's
//! `DISPLAY` and `WAYLAND_DISPLAY` say: run it inside a compositor of its
//! own (`cage` with `WLR_BACKENDS=headless`), never on a desktop. On macOS
//! they open on the display, which is fine on a Mac nobody works at while
//! it runs. Every tool gets a home and configuration folders of its own,
//! set in front of its command (`HOME=… XDG_CONFIG_HOME=…`; on macOS also
//! `CFFIXED_USER_HOME`, which is where Apple's frameworks take the home
//! from, `HOME` being the shell's; and `-Duser.home` for a tool on Java,
//! which takes the home from the system's list of users), so that nothing
//! it writes reaches the user's, and an editor that is running on the
//! desktop is not handed the files; the processes of a tool are told apart
//! by that home and ended after each step. On Linux they are found through
//! `/proc`. On macOS through `ps`: those whose environment has the home
//! (`ps -E`, which shows it for every program but the system's own, such
//! as `opendiff`, whose FileMerge it does show), and everything they
//! started. Not those whose arguments name the home: git's do, as the
//! command is handed to it, and git is not to be ended. What macOS keeps
//! in the user's preferences (`cfprefsd` goes by the user, not by a home)
//! still goes there: on macOS the test says at the end which of the user's
//! preference files are new since it started.
//!
//! On Windows the windows open on the display as on macOS, and nothing
//! tells a process's environment from outside without code the workspace
//! forbids: a tool's processes are those the test started (through git
//! and its sh, which are not ended) and everything they started, found in
//! the system's list of processes (`Win32_Process`, through the system's
//! PowerShell), and those whose command line names the tool's home.
//! Every tool gets `USERPROFILE`, `APPDATA` and `LOCALAPPDATA` in its
//! home as well, which is where Windows finds a user's folders; what a
//! tool keeps in the user's registry still goes there, and the test says
//! at the end which keys under `HKCU\Software` are new since it started.
//! The folder of tools holds a small sh script under each program's name
//! that starts the tool where it is installed.
//!
//! IntelliJ IDEA asks for its user agreement on a profile that has not
//! accepted it, and keeps the answer in Java's preferences: its program
//! in the folder is to be a script that gives it a profile of its own
//! (`IDEA_PROPERTIES` naming its config, system, log and plugins folders,
//! `IDEA_VM_OPTIONS` a small heap, `-Duser.home` and
//! `-Djava.util.prefs.userRoot`, and the XDG folders) in which the
//! agreement was accepted once, and that leaves `HOME` as it is given.
//! It takes some 15 seconds to show its window, and is given half a
//! minute before it is looked at (`settles_in`).
//!
//! Sources: git-difftool(1), git-mergetool(1) (the temporary files and how
//! an untrusted tool is judged by its output file); proc(5) (`cmdline`,
//! `environ`, `stat`); ps(1) on macOS (`-A`, `-E`, `-ww`, `-o`); Microsoft
//! Learn, "Win32_Process class" (`ProcessId`, `ParentProcessId`,
//! `CommandLine`, `ExecutablePath`), "Get-CimInstance", "taskkill" (`/T`,
//! `/F`), "Parsing C++ command-line arguments" and "reg query"; Apple's
//! "File System Programming Guide" (the user's Library) and the
//! documentation of `NSHomeDirectory` and `CFCopyHomeDirectoryURL`; the
//! Java documentation of the system property `user.home` and of
//! `JAVA_TOOL_OPTIONS`; the KDiff3 handbook (`--auto`), the Vim manual
//! (`-c`, `:diffget`, `:wqa`), the Beyond Compare documentation, "Command
//! Line Reference" (`-automerge`, `-force`).
#![cfg(any(target_os = "linux", target_os = "macos", windows))]
#![allow(clippy::print_stdout)]

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_git::known_tools::{KNOWN, KnownTool};
use bitbraid_testkit::Scratch;
use serde_json::json;

/// The file the tools are shown: a name of its own, so that what is
/// handed to a tool is found among its arguments.
const FILE: &str = "live_tool.txt";
const BASE: &str = "one\ntwo\nthree\nfour\n";
/// Ours changes the second line and theirs the third: lines next to each
/// other, which git takes for a conflict and a merge tool may solve.
const OURS: &str = "one\nTWO ours\nthree\nfour\n";
const THEIRS: &str = "one\ntwo\nTHREE theirs\nfour\n";
const BOTH: &str = "one\nTWO ours\nTHREE theirs\nfour\n";
const WORKTREE: &str = "one\ntwo\nthree\nfour\nchanged in the worktree\n";

/// How long a tool is given to show up and to end.
const PATIENCE: Duration = Duration::from_secs(60);

/// The folder of the tools, when the test is asked for.
fn tools_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BITBRAID_TEST_TOOLS")?);
    dir.is_dir().then_some(dir)
}

/// What a file a tool wrote by itself is to hold.
#[derive(Debug, Clone, Copy)]
enum Holds {
    /// Exactly this.
    Exactly(&'static str),
    /// Each of these, among what else the tool wrote.
    Each(&'static [&'static str]),
}

/// How long a tool is given to be on the screen once its program is
/// seen with the files, before it is looked at, stopped or written
/// behind: a few seconds, and longer for an IDE that starts a JVM and its
/// plugins first.
fn settles_in(tool: &KnownTool) -> Duration {
    match tool.program {
        // IntelliJ IDEA 2026.2.3 shows its window about 15 s after it is
        // started, in a headless compositor with a small heap.
        "idea" => Duration::from_secs(30),
        _ => Duration::from_secs(3),
    }
}

/// What a tool's command is followed by so that it saves without a
/// person, and what the file then holds.
fn saves_alone(tool: &KnownTool) -> Option<(&'static str, Holds)> {
    match tool.program {
        // The result's window is the second one; everything is taken
        // from the first (ours), and every file is written on the way out.
        "gvim" => Some((
            " -c 'wincmd l' -c '%diffget 1' -c 'wqa'",
            Holds::Exactly(OURS),
        )),
        // Lines next to each other are no conflict to KDiff3: no window,
        // both changes written.
        "kdiff3" => Some((" --auto", Holds::Exactly(BOTH))),
        // To Beyond Compare they are one: written with its own markers
        // around both sides, as asked.
        "bcompare" => Some((
            " -automerge -force",
            Holds::Each(&["TWO ours", "THREE theirs"]),
        )),
        _ => None,
    }
}

/// A process: its number, its arguments.
#[derive(Debug, Clone)]
struct Process {
    pid: u32,
    args: Vec<String>,
}

/// The processes whose environment has `HOME` set to `home`: the tool's,
/// and whatever it started.
#[cfg(target_os = "linux")]
fn processes_of(home: &Path) -> Vec<Process> {
    let wanted = format!("HOME={}", home.display());
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };
    entries
        .filter_map(|entry| {
            let pid: u32 = entry.ok()?.file_name().to_str()?.parse().ok()?;
            let environ = std::fs::read(format!("/proc/{pid}/environ")).ok()?;
            if !environ
                .split(|byte| *byte == 0)
                .any(|var| var == wanted.as_bytes())
            {
                return None;
            }
            // A process that has ended and waits to be collected is gone.
            let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
            let state = stat.rsplit_once(')')?.1.split_whitespace().next()?;
            if state == "Z" || state == "X" {
                return None;
            }
            let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
            let args = cmdline
                .split(|byte| *byte == 0)
                .filter(|arg| !arg.is_empty())
                .map(|arg| String::from_utf8_lossy(arg).into_owned())
                .collect();
            Some(Process { pid, args })
        })
        .collect()
}

/// The processes of the tool whose home is `home`, on macOS: those whose
/// environment has `HOME` set to it, and what they started.
#[cfg(target_os = "macos")]
fn processes_of(home: &Path) -> Vec<Process> {
    use std::collections::{HashMap, HashSet};
    let ps = |columns: &str, environment: bool| -> HashMap<u32, String> {
        let mut command = std::process::Command::new("ps");
        command.args(["-A", "-ww", "-o", columns]);
        if environment {
            command.arg("-E");
        }
        let Ok(out) = command.stderr(std::process::Stdio::null()).output() else {
            return HashMap::new();
        };
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter_map(|line| {
                let (pid, rest) = line.trim_start().split_once(' ')?;
                Some((pid.parse().ok()?, rest.to_owned()))
            })
            .collect()
    };
    // The parent and the state; the program; what it was started with;
    // and that with its environment after it.
    let states = ps("pid=,ppid=,stat=", false);
    let programs = ps("pid=,comm=", false);
    let commands = ps("pid=,command=", false);
    let environments = ps("pid=,command=", true);
    let wanted = format!("HOME={}", home.display());
    let parents: HashMap<u32, u32> = states
        .iter()
        .filter_map(|(pid, rest)| {
            let mut fields = rest.split_whitespace();
            let ppid = fields.next()?.parse().ok()?;
            let state = fields.next()?;
            // A process that has ended and waits to be collected is gone.
            (!state.starts_with('Z')).then_some((*pid, ppid))
        })
        .collect();
    let own = |pid: &u32| {
        let command = commands.get(pid).map(String::as_str).unwrap_or_default();
        let environment = environments
            .get(pid)
            .and_then(|all| all.strip_prefix(command))
            .unwrap_or_default();
        environment.split(' ').any(|var| var == wanted)
    };
    let mut ours: HashSet<u32> = parents.keys().copied().filter(|pid| own(pid)).collect();
    // And everything they started, down to the last.
    loop {
        let more: Vec<u32> = parents
            .iter()
            .filter(|(pid, ppid)| !ours.contains(pid) && ours.contains(ppid))
            .map(|(pid, _)| *pid)
            .collect();
        if more.is_empty() {
            break;
        }
        ours.extend(more);
    }
    ours.remove(&std::process::id());
    ours.into_iter()
        .filter_map(|pid| {
            let program = programs.get(&pid)?.trim().to_owned();
            let command = commands.get(&pid)?;
            // The program at its whole path, then the arguments: a path
            // with spaces in it is the program's alone.
            let rest = command.strip_prefix(&program).unwrap_or(command);
            let mut args = vec![program];
            args.extend(
                rest.split_whitespace()
                    .skip(usize::from(rest == command))
                    .map(str::to_owned),
            );
            Some(Process { pid, args })
        })
        .collect()
}

/// A command line as Windows programs take it apart: words between
/// blanks, a double quote beginning and ending a part with blanks in it,
/// backslashes before a quote halved (an odd one makes the quote a
/// letter), other backslashes kept.
#[cfg(windows)]
fn split_command_line(line: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut started = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                let mut slashes = 1;
                while chars.peek() == Some(&'\\') {
                    chars.next();
                    slashes += 1;
                }
                if chars.peek() == Some(&'"') {
                    current.push_str(&"\\".repeat(slashes / 2));
                    if slashes % 2 == 1 {
                        chars.next();
                        current.push('"');
                    }
                } else {
                    current.push_str(&"\\".repeat(slashes));
                }
                started = true;
            }
            '"' => {
                quoted = !quoted;
                started = true;
            }
            ' ' | '\t' if !quoted => {
                if started {
                    args.push(std::mem::take(&mut current));
                    started = false;
                }
            }
            other => {
                current.push(other);
                started = true;
            }
        }
    }
    if started {
        args.push(current);
    }
    args
}

/// Where Git for Windows is installed: its programs (git, its sh) are the
/// test's means, and are never taken for a tool's or ended.
#[cfg(windows)]
fn git_root() -> Option<PathBuf> {
    let out = std::process::Command::new("git")
        .arg("--exec-path")
        .output()
        .ok()?;
    let exec = PathBuf::from(String::from_utf8(out.stdout).ok()?.trim());
    // `<root>/mingw64/libexec/git-core`
    Some(exec.parent()?.parent()?.parent()?.to_path_buf())
}

/// The processes of the tool whose home is `home`, on Windows: those this
/// test started and what they started, git's own programs aside, and those
/// whose command line names the home.
#[cfg(windows)]
fn processes_of(home: &Path) -> Vec<Process> {
    use std::collections::{HashMap, HashSet};
    let Some(root) = std::env::var_os("SystemRoot") else {
        return Vec::new();
    };
    let powershell = Path::new(&root).join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let Ok(out) = std::process::Command::new(powershell)
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,CommandLine,ExecutablePath | ConvertTo-Json -Compress",
        ])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
    else {
        return Vec::new();
    };
    let Ok(rows) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else {
        return Vec::new();
    };
    let lower = |text: &str| text.to_lowercase().replace('/', "\\");
    let git = git_root().map(|root| lower(&root.display().to_string()));
    let home_named = lower(&home.display().to_string());
    struct Row {
        parent: u32,
        command: String,
        exe: String,
    }
    let rows: HashMap<u32, Row> = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| {
            let pid = u32::try_from(row["ProcessId"].as_u64()?).ok()?;
            let parent = u32::try_from(row["ParentProcessId"].as_u64().unwrap_or(0)).ok()?;
            Some((
                pid,
                Row {
                    parent,
                    command: row["CommandLine"].as_str().unwrap_or_default().to_owned(),
                    exe: row["ExecutablePath"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                },
            ))
        })
        .collect();
    let me = std::process::id();
    // MSYS2 starts the sh that runs a script of the folder of tools from
    // a process of its own that is gone by then: the chain of parents from
    // this test breaks there, and that sh is where it goes on.
    let scripts = tools_dir().map(|dir| lower(&dir.display().to_string()));
    let mut ours: HashSet<u32> = rows
        .iter()
        .filter(|(_, row)| {
            let command = lower(&row.command);
            command.contains(&home_named)
                || scripts
                    .as_ref()
                    .is_some_and(|scripts| command.contains(scripts.as_str()))
        })
        .map(|(pid, _)| *pid)
        .collect();
    ours.insert(me);
    loop {
        let more: Vec<u32> = rows
            .iter()
            .filter(|(pid, row)| {
                !ours.contains(pid) && ours.contains(&row.parent) && **pid != row.parent
            })
            .map(|(pid, _)| *pid)
            .collect();
        if more.is_empty() {
            break;
        }
        ours.extend(more);
    }
    ours.remove(&me);
    ours.into_iter()
        .filter_map(|pid| {
            let row = rows.get(&pid)?;
            let exe = lower(&row.exe);
            let gits = git
                .as_ref()
                .is_some_and(|git| exe.starts_with(git.as_str()));
            let console = exe.ends_with("\\conhost.exe");
            // What the test starts itself (git through the engine, the
            // list of processes, taskkill) is never a tool's.
            let own = row.parent == me;
            if gits || console || own || exe.is_empty() {
                return None;
            }
            Some(Process {
                pid,
                args: split_command_line(&row.command),
            })
        })
        .collect()
}

/// The keys under `HKCU\Software`, by name: what a tool keeps in the
/// user's registry, whatever its home. Their names only are read.
#[cfg(windows)]
fn users_registry_keys() -> std::collections::BTreeSet<String> {
    let Ok(out) = std::process::Command::new("reg")
        .args(["query", "HKCU\\Software"])
        .output()
    else {
        return Default::default();
    };
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("HKEY_CURRENT_USER\\"))
        .map(str::to_owned)
        .collect()
}

/// The user's preference files, by name: what macOS keeps for each
/// program by the user, whatever its home.
#[cfg(target_os = "macos")]
fn users_preferences() -> std::collections::BTreeSet<String> {
    let Some(home) = std::env::var_os("HOME") else {
        return Default::default();
    };
    std::fs::read_dir(Path::new(&home).join("Library").join("Preferences"))
        .map(|entries| {
            entries
                .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Wait until a process of the tool has an argument that `wanted` takes;
/// all of its arguments then.
fn wait_for_args(home: &Path, wanted: impl Fn(&str) -> bool) -> Option<Vec<String>> {
    let started = Instant::now();
    while started.elapsed() < PATIENCE {
        if let Some(found) = processes_of(home)
            .into_iter()
            .find(|process| process.args.iter().any(|arg| wanted(arg)))
        {
            return Some(found.args);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    None
}

fn signal(pid: u32, name: &str) {
    #[cfg(windows)]
    {
        // Asked: the window is told to close; killed: ended at once.
        let mut taskkill = std::process::Command::new("taskkill");
        taskkill.args(["/PID", &pid.to_string(), "/T"]);
        if name == "KILL" {
            taskkill.arg("/F");
        }
        let _ = taskkill
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(windows))]
    let _ = std::process::Command::new("kill")
        .args([&format!("-{name}"), &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status();
}

/// End every process of the tool: asked first, killed when it does not
/// end. What was left after it was asked is said.
fn end_all(home: &Path) -> Vec<String> {
    let left = processes_of(home);
    for process in &left {
        signal(process.pid, "TERM");
    }
    let started = Instant::now();
    while !processes_of(home).is_empty() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(100));
    }
    for process in processes_of(home) {
        signal(process.pid, "KILL");
    }
    left.iter()
        .map(|process| process.args.first().cloned().unwrap_or_default())
        .collect()
}

/// The command for a tool, with its program at its full path in `dir`, a
/// home and configuration of its own in front of it, and `more` after it.
fn command_for(line: &str, tool: &KnownTool, dir: &Path, home: &Path, more: &str) -> String {
    let home = home.display();
    let program = dir.join(tool.program);
    // Apple's frameworks take the home from CFFIXED_USER_HOME, and Java
    // from the system's list of users, not from HOME.
    let apple = if cfg!(target_os = "macos") {
        format!("CFFIXED_USER_HOME='{home}' ")
    } else {
        String::new()
    };
    // Windows finds a user's folders from these, not from HOME.
    let windows = if cfg!(windows) {
        format!(
            "USERPROFILE='{home}' APPDATA='{home}\\AppData\\Roaming' \
             LOCALAPPDATA='{home}\\AppData\\Local' "
        )
    } else {
        String::new()
    };
    let start = format!(
        "HOME='{home}' {apple}{windows}XDG_CONFIG_HOME='{home}/config' XDG_DATA_HOME='{home}/data' \
         XDG_CACHE_HOME='{home}/cache' XDG_STATE_HOME='{home}/state' \
         JAVA_TOOL_OPTIONS='-Duser.home={home}' '{}'",
        program.display()
    );
    format!("{}{more}", line.replacen(tool.program, &start, 1))
}

/// A repository with the file changed in the worktree.
fn changed() -> Scratch {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    git.run(["config", "commit.gpgsign", "false"]).unwrap();
    copy.write(FILE, BASE).unwrap();
    git.run(["add", FILE]).unwrap();
    git.run(["commit", "-q", "-m", "base"]).unwrap();
    copy.write(FILE, WORKTREE).unwrap();
    copy
}

/// A repository with the file in conflict, in a merge begun.
fn in_conflict() -> Scratch {
    let copy = changed();
    let git = copy.git();
    git.run(["checkout", "-q", "--", FILE]).unwrap();
    for (key, value) in [
        ("merge.conflictStyle", "merge"),
        ("rerere.enabled", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write(FILE, THEIRS).unwrap();
    git.run(["commit", "-q", "-am", "theirs"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write(FILE, OURS).unwrap();
    git.run(["commit", "-q", "-am", "ours"]).unwrap();
    assert!(git.run(["merge", "-q", "topic"]).is_err(), "no conflict");
    copy
}

fn read(path: &str, repo: &Path) -> Option<String> {
    // A file handed by a switch of Windows' kind (`/base:C:\…`) is what
    // follows its colon.
    let path = path
        .split_once(':')
        .filter(|(switch, _)| {
            switch.len() > 2
                && switch
                    .strip_prefix('/')
                    .is_some_and(|word| word.chars().all(|c| c.is_ascii_alphabetic()))
        })
        .map_or(path, |(_, file)| file);
    let path = Path::new(path);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        repo.join(path)
    };
    std::fs::read_to_string(path).ok()
}

/// What happened to one tool, said at the end.
#[derive(Default)]
struct Report {
    lines: Vec<String>,
}

impl Report {
    fn say(&mut self, tool: &KnownTool, what: impl Into<String>) {
        let line = format!("{}: {}", tool.name, what.into());
        println!("{line}");
        self.lines.push(line);
    }
}

/// The diff: the two versions handed over, and left open.
fn diff(tool: &KnownTool, line: &str, dir: &Path, home: &Path, report: &mut Report) {
    let copy = changed();
    let index = BASE;
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    let command = command_for(line, tool, dir, home, "");
    let answer = registry
        .run(
            "tool.diff",
            &mut ctx,
            &json!({ "path": FILE, "command": command }),
        )
        .unwrap_or_else(|err| panic!("{}: {err}", tool.name));
    assert!(answer["started"].is_string(), "{}: {answer}", tool.name);
    let args = wait_for_args(home, |arg| arg.ends_with(FILE))
        .unwrap_or_else(|| panic!("{}: never seen with the file", tool.name));
    let files: Vec<String> = args
        .iter()
        .filter(|arg| arg.ends_with(FILE))
        .filter_map(|arg| read(arg, copy.path()))
        .collect();
    assert_eq!(
        files,
        [index.to_owned(), WORKTREE.to_owned()],
        "{} was handed {args:?}",
        tool.name
    );
    // Left open: still there a while later, and nobody waits for it.
    std::thread::sleep(settles_in(tool));
    assert!(
        !processes_of(home).is_empty(),
        "{}: the diff tool did not stay open",
        tool.name
    );
    end_all(home);
    report.say(tool, "diff: handed both versions, left open");
}

/// Run `tool.merge` while `watch` looks on from another thread; what it
/// answered (or the error), and what `watch` returned.
fn merge_watched<T: Send + 'static>(
    copy: &Scratch,
    command: &str,
    watch: impl FnOnce(Arc<AtomicBool>, Arc<AtomicBool>) -> T + Send + 'static,
) -> (Result<serde_json::Value, String>, T) {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    let returned = Arc::new(AtomicBool::new(false));
    let watcher = {
        let (cancel, returned) = (ctx.cancel.clone(), returned.clone());
        std::thread::spawn(move || watch(cancel, returned))
    };
    let answer = registry
        .run(
            "tool.merge",
            &mut ctx,
            &json!({ "path": FILE, "command": command }),
        )
        .map_err(|err| err.to_string());
    returned.store(true, Ordering::SeqCst);
    (answer, watcher.join().unwrap())
}

/// Whether the arguments of a merge tool are the sides, the result, and
/// the base where it takes one.
fn handed_for_a_merge(tool: &KnownTool, line: &str, args: &[String], repo: &Path) {
    let files: Vec<String> = args
        .iter()
        .filter(|arg| arg.contains("live_tool"))
        .filter_map(|arg| read(arg, repo))
        .collect();
    assert!(files.contains(&OURS.to_owned()), "{}: {args:?}", tool.name);
    assert!(
        files.contains(&THEIRS.to_owned()),
        "{}: {args:?}",
        tool.name
    );
    assert!(
        files.iter().any(|file| file.contains("<<<<<<<")),
        "{} was not handed the file to write: {args:?}",
        tool.name
    );
    assert_eq!(
        files.contains(&BASE.to_owned()),
        line.contains("$BASE"),
        "{}: {args:?}",
        tool.name
    );
}

/// The merge, stopped while it is waited for.
fn merge_stopped(tool: &KnownTool, line: &str, dir: &Path, home: &Path, report: &mut Report) {
    let copy = in_conflict();
    let command = command_for(line, tool, dir, home, "");
    let watched_home = home.to_path_buf();
    let settle = settles_in(tool);
    let (answer, (args, waited, left)) = merge_watched(&copy, &command, move |cancel, returned| {
        let args = wait_for_args(&watched_home, |arg| arg.contains("live_tool_LOCAL"));
        std::thread::sleep(settle);
        let waited = !returned.load(Ordering::SeqCst);
        cancel.store(true, Ordering::SeqCst);
        // What of the tool is left once the waiting was stopped.
        let started = Instant::now();
        while !returned.load(Ordering::SeqCst) && started.elapsed() < PATIENCE {
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_secs(4));
        let left: Vec<Process> = processes_of(&watched_home);
        (args, waited, left)
    });
    let args = args.unwrap_or_else(|| panic!("{}: never seen with the files", tool.name));
    handed_for_a_merge(tool, line, &args, copy.path());
    assert!(waited, "{}: the merge tool was not waited for", tool.name);
    let err = answer.expect_err("stopped");
    assert!(err.contains("cancelled"), "{}: {err}", tool.name);
    let status = copy.git().run(["status", "--porcelain", FILE]).unwrap();
    assert!(status.starts_with("UU"), "{}: {status}", tool.name);
    // What still has the files open is a window of the tool's own
    // instance outside the group, which only the user closes.
    let (with_files, helpers): (Vec<&Process>, Vec<&Process>) = left
        .iter()
        .partition(|process| process.args.iter().any(|arg| arg.contains("live_tool")));
    let names = |processes: Vec<&Process>| -> Vec<String> {
        processes
            .iter()
            .map(|process| process.args.first().cloned().unwrap_or_default())
            .collect()
    };
    let (with_files, helpers) = (names(with_files), names(helpers));
    end_all(home);
    // A helper the tool started into a session of its own (IntelliJ
    // IDEA's `jetbrainsd`) outlives Stop as well, without the files.
    let helpers = if helpers.is_empty() {
        String::new()
    } else {
        format!("; a helper of its own without the files stayed: {helpers:?}")
    };
    report.say(
        tool,
        if with_files.is_empty() {
            format!("merge: handed the sides, waited for, ended by Stop{helpers}")
        } else {
            format!(
                "merge: handed the sides, waited for; Stop ended the wait, and left \
                 {with_files:?}{helpers}"
            )
        },
    );
}

/// The merge, finished: by the tool alone where it can, else by writing
/// the file while it is open and ending it; then ended without writing.
fn merge_finished(tool: &KnownTool, line: &str, dir: &Path, home: &Path, report: &mut Report) {
    let copy = in_conflict();
    if let Some((more, expected)) = saves_alone(tool) {
        let command = command_for(line, tool, dir, home, more);
        let (answer, ()) = merge_watched(&copy, &command, |_, _| ());
        let answer = answer.unwrap_or_else(|err| panic!("{}: {err}", tool.name));
        assert_eq!(answer["resolved"], true, "{}", tool.name);
        let written = std::fs::read_to_string(copy.path().join(FILE)).unwrap();
        match expected {
            Holds::Exactly(text) => assert_eq!(written, text, "{}", tool.name),
            Holds::Each(parts) => assert!(
                parts.iter().all(|part| written.contains(part)),
                "{}: {written:?}",
                tool.name
            ),
        }
        end_all(home);
        report.say(tool, format!("merge: saved by itself ({})", more.trim()));
        return;
    }
    // Written while it is open, as saving would, then closed.
    let command = command_for(line, tool, dir, home, "");
    let worktree_file = copy.path().join(FILE);
    let watched_home = home.to_path_buf();
    let settle = settles_in(tool);
    let (answer, ()) = merge_watched(&copy, &command, move |_, _| {
        wait_for_args(&watched_home, |arg| arg.contains("live_tool_LOCAL"));
        std::thread::sleep(settle);
        std::fs::write(&worktree_file, BOTH).unwrap();
        end_all(&watched_home);
    });
    let answer = answer.unwrap_or_else(|err| panic!("{}: {err}", tool.name));
    assert_eq!(answer["resolved"], true, "{}", tool.name);
    assert_eq!(
        std::fs::read_to_string(copy.path().join(FILE)).unwrap(),
        BOTH
    );
    // Closed without the file written: still in conflict.
    let copy = in_conflict();
    let command = command_for(line, tool, dir, home, "");
    let watched_home = home.to_path_buf();
    let (answer, ()) = merge_watched(&copy, &command, move |_, _| {
        wait_for_args(&watched_home, |arg| arg.contains("live_tool_LOCAL"));
        std::thread::sleep(settle);
        end_all(&watched_home);
    });
    let answer = answer.unwrap_or_else(|err| panic!("{}: {err}", tool.name));
    assert_eq!(answer["resolved"], false, "{}", tool.name);
    report.say(
        tool,
        "merge: what was written while it was open was read back when it ended; ended unwritten, still in conflict",
    );
}

#[test]
fn the_known_tools_run_for_real() {
    let Some(dir) = tools_dir() else {
        println!(
            "known_tools_live: skipped: BITBRAID_TEST_TOOLS does not name a folder of tools \
             (run inside a compositor of its own)"
        );
        return;
    };
    let homes = std::env::temp_dir().join(format!("bitbraid-live-tools-{}", std::process::id()));
    #[cfg(target_os = "macos")]
    let preferences_before = users_preferences();
    #[cfg(windows)]
    let registry_before = users_registry_keys();
    let mut report = Report::default();
    let mut tried = 0;
    for tool in KNOWN {
        if !dir.join(tool.program).is_file() {
            println!("{}: not in {}", tool.name, dir.display());
            continue;
        }
        tried += 1;
        let home = homes.join(tool.program);
        std::fs::create_dir_all(&home).unwrap();
        // On macOS a home has these, and Tk stops (a trap) where it finds
        // no `Library/Caches` in it.
        if cfg!(target_os = "macos") {
            for folder in ["Caches", "Preferences", "Application Support"] {
                std::fs::create_dir_all(home.join("Library").join(folder)).unwrap();
            }
        }
        // And on Windows the folders of a user's own.
        if cfg!(windows) {
            for folder in ["Roaming", "Local", "LocalLow"] {
                std::fs::create_dir_all(home.join("AppData").join(folder)).unwrap();
            }
        }
        if let Some(line) = tool.diff {
            diff(tool, line, &dir, &home, &mut report);
        }
        if let Some(line) = tool.merge {
            merge_stopped(tool, line, &dir, &home, &mut report);
            merge_finished(tool, line, &dir, &home, &mut report);
        }
        end_all(&home);
    }
    assert!(tried > 0, "no known tool in {}", dir.display());
    println!("\n{}", report.lines.join("\n"));
    #[cfg(target_os = "macos")]
    {
        let new: Vec<String> = users_preferences()
            .difference(&preferences_before)
            .cloned()
            .collect();
        println!(
            "\nNew in the user's ~/Library/Preferences since the start (macOS keeps \
             these by the user, not by the home): {new:?}"
        );
    }
    #[cfg(windows)]
    {
        let new: Vec<String> = users_registry_keys()
            .difference(&registry_before)
            .cloned()
            .collect();
        println!(
            "\nNew under the user's HKCU\\Software since the start (Windows keeps \
             these by the user, not by the home): {new:?}"
        );
    }
}

#[cfg(windows)]
#[test]
fn a_command_line_is_taken_apart_as_windows_programs_do() {
    assert_eq!(
        split_command_line(
            r#""C:\Program Files\Tool\tool.exe" -e  a\b.txt "two words" "q\"uote" x\\"#
        ),
        [
            r"C:\Program Files\Tool\tool.exe",
            "-e",
            r"a\b.txt",
            "two words",
            "q\"uote",
            r"x\\",
        ]
    );
    assert!(split_command_line("").is_empty());
    assert_eq!(split_command_line(r#""""#), [""]);
}
