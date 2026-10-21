//! The same test program again, in surroundings of its own.
//!
//! A test cannot set an environment variable, and the application reads
//! several: where its configuration is, the home directory, the keys of
//! gpg and of the SSH agent, the user's git configuration, the time zone.
//! So the program that cargo starts does nothing but start itself once
//! more with an environment that names only scratch directories, and that
//! second process runs the tests. Nothing in it can reach the developer's
//! configuration, keys or desktop, and what it draws does not depend on
//! them.
//!
//! The scratch directories are at one fixed place, because paths are on
//! show (the status bar) and the pictures are compared: one run at a time
//! holds a lock on it.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
};

/// Set in the second process: the root of its scratch directories.
const INSIDE: &str = "BITBRAID_OFFSCREEN_ROOT";

/// Where the files are that hold git up (`hold_git`), under the root.
const HOLDS: &str = "holds";

/// How many times a git that is held up looks again, a twentieth of a
/// second apart: a minute, so that a test that fails with a hold in place
/// leaves no git waiting for longer.
const HELD_LOOKS: u32 = 1200;

/// Where everything of a run lives. Fixed, so that a path that is drawn
/// is the same on every machine and in every checkout. On macOS `/tmp` is
/// a link to `/private/tmp`, and the application takes paths as the
/// system resolves them: there it is the resolved place.
#[cfg(not(any(target_os = "macos", windows)))]
const ROOT: &str = "/tmp/bitbraid-offscreen";
#[cfg(target_os = "macos")]
const ROOT: &str = "/private/tmp/bitbraid-offscreen";
/// On Windows, which has no `/tmp`, in the folder every user shares;
/// paths on show differ there, and the pictures are not compared. Written
/// as Windows writes it: the application keys a repository's state by its
/// path so written, and the tests write that state for it.
#[cfg(windows)]
const ROOT: &str = r"C:\Users\Public\bitbraid-offscreen";

/// The moment the interface's clock is held at (`BITBRAID_NOW`): two and
/// a half days after the fixtures' first commit, 2020-01-03 12:00 UTC.
pub const NOW: i64 = 1_577_836_800 + 60 * 3600;

/// What the git that is run says it is. The real one runs; the version it
/// reports is on show in the status bar and would differ by machine.
pub const GIT_VERSION: &str = "2.99.0";

/// The root of the scratch directories, when this is the second process.
pub fn inside() -> Option<PathBuf> {
    std::env::var_os(INSIDE).map(PathBuf::from)
}

/// git held up: while it is there, every git that is run with `word`
/// among its arguments waits before it begins (at most a minute). Gone
/// when dropped. For what the window shows while something it asked for
/// takes a while (`Stage::hold_git`).
pub struct Held(PathBuf);

/// Whether git can be held up here: on Windows git runs without the
/// wrapper (`git_wrapper`).
pub fn holds_git() -> bool {
    !cfg!(windows)
}

/// Hold up every git run with `word` among its arguments (`Held`).
pub fn hold_git(word: &str) -> Held {
    let root = inside().expect("a hold is for the second process");
    let file = root.join(HOLDS).join(format!("hold-{word}"));
    fs::write(&file, "").expect("the hold is written");
    Held(file)
}

impl Drop for Held {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}

/// Variables the second process keeps: what finds programs and libraries,
/// what cargo and the testkit go by, and what chooses a graphics driver.
fn kept(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "PATH",
        "LD_LIBRARY_PATH",
        "TMPDIR",
        "CARGO_TARGET_DIR",
        "CARGO_MANIFEST_DIR",
        "RUST_BACKTRACE",
        "CI",
        "BITBRAID_DEBUG",
        // The opt-in live test of the pull requests against a GitLab of
        // one's own (`live_gitlab.rs`).
        "BITBRAID_GITLAB_CE",
        "BITBRAID_GITLAB_CE_TOKEN_FILE",
        // The opt-in tests of shells other than bash
        // (`behaviour_terminal_shells.rs`).
        "BITBRAID_TEST_SHELLS",
        // Asked for on macOS (`stage::ON_MACOS`) and Windows.
        "BITBRAID_OFFSCREEN_ON_MACOS",
        "BITBRAID_OFFSCREEN_ON_WINDOWS",
        // What a program on Windows needs of its surroundings: the
        // system's folder, the endings of programs, the shell.
        "SYSTEMROOT",
        "WINDIR",
        "COMSPEC",
        "PATHEXT",
        "TEMP",
        "TMP",
        "NUMBER_OF_PROCESSORS",
        "PROCESSOR_ARCHITECTURE",
        "ZED_DEVICE_ID",
        "DRI_PRIME",
    ];
    const PREFIXES: &[&str] = &[
        "VK_", "WGPU_", "MESA_", "LIBGL_", "GALLIUM_", "LP_", "__NV", "__GLX", "__EGL", "NVIDIA_",
    ];
    // On Windows a variable's name has no case (`Path`, `SystemRoot`).
    let name = if cfg!(windows) {
        name.to_ascii_uppercase()
    } else {
        name.to_owned()
    };
    EXACT.contains(&name.as_str()) || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
}

fn make_dir(path: &Path, private: bool) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    if private {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(())
}

/// Lay out the scratch directories anew.
fn prepare(root: &Path, real_git: &str) -> std::io::Result<()> {
    if root.exists() {
        fs::remove_dir_all(root)?;
    }
    make_dir(root, true)?;
    make_dir(&root.join("home"), false)?;
    make_dir(&root.join("config"), false)?;
    make_dir(&root.join("gnupg"), true)?;
    make_dir(&root.join("run"), true)?;
    make_dir(&root.join("work"), false)?;
    make_dir(&root.join("bin"), false)?;
    make_dir(&root.join(HOLDS), false)?;
    fs::write(
        root.join("gitconfig"),
        concat!(
            "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n",
            "[init]\n\tdefaultBranch = main\n",
            "[commit]\n\tgpgsign = false\n",
            "[tag]\n\tgpgsign = false\n",
            "[core]\n\tautocrlf = false\n\tpager = cat\n",
            "[gc]\n\tauto = 0\n",
            "[protocol \"file\"]\n\tallow = always\n",
            "[credential]\n\thelper =\n",
        ),
    )?;
    // The git that is run: the real one, under a version that is the same
    // everywhere, which waits while a test holds it up (`hold_git`).
    // Written and closed before anything is started, so that no child
    // holds it open while it is run.
    let wrapper = root.join("bin/git");
    let holds = root.join(HOLDS).to_string_lossy().replace('\'', "'\\''");
    let mut file = fs::File::create(&wrapper)?;
    writeln!(file, "#!/bin/sh")?;
    writeln!(
        file,
        "if [ \"$1\" = \"--version\" ]; then echo \"git version {GIT_VERSION}\"; exit 0; fi"
    )?;
    writeln!(
        file,
        "for word; do n=0; while [ -f '{holds}/hold-'\"$word\" ] && [ $n -lt {HELD_LOOKS} ]; do sleep 0.05; n=$((n + 1)); done; done"
    )?;
    writeln!(file, "exec '{}' \"$@\"", real_git.replace('\'', "'\\''"))?;
    file.sync_all()?;
    drop(file);
    bitbraid_testkit::script::make_runnable(&wrapper).map(|_| ())
}

/// The git the tests and the application run: the wrapper of `root`, and
/// on Windows git itself (`real_git`, found on `PATH` when it is a name).
/// A batch file is all that runs a script there, and it cannot hand on a
/// message of several lines or a name with `&` or `%`; the pictures drawn
/// on Windows are not compared, so the version git says can be its own.
fn git_wrapper(root: &Path, real_git: &str) -> PathBuf {
    if !cfg!(windows) {
        return root.join("bin/git");
    }
    let given = PathBuf::from(real_git);
    if given.components().count() > 1 {
        return given;
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(format!("{real_git}.exe")))
                .find(|candidate| candidate.is_file())
        })
        .unwrap_or(given)
}

/// Start this program again inside scratch surroundings and wait for it.
pub fn run_again(arguments: &[String]) -> ExitCode {
    let root = PathBuf::from(ROOT);
    // One run at a time: the directories are at a fixed place.
    let lock_path = PathBuf::from(format!("{ROOT}.lock"));
    let lock = match fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
    {
        Ok(lock) => lock,
        Err(err) => {
            println!(
                "offscreen: skipped: cannot open {}: {err}",
                lock_path.display()
            );
            return ExitCode::SUCCESS;
        }
    };
    if lock.try_lock().is_err() {
        println!("offscreen: another run holds {ROOT}; waiting for it");
        if let Err(err) = lock.lock() {
            println!(
                "offscreen: failed: cannot lock {}: {err}",
                lock_path.display()
            );
            return ExitCode::FAILURE;
        }
    }
    let real_git = std::env::var("BITBRAID_GIT")
        .ok()
        .filter(|set| !set.is_empty())
        .unwrap_or_else(|| "git".to_owned());
    if let Err(err) = prepare(&root, &real_git) {
        println!("offscreen: failed: cannot prepare {ROOT}: {err}");
        return ExitCode::FAILURE;
    }
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(err) => {
            println!("offscreen: failed: no path to this program: {err}");
            return ExitCode::FAILURE;
        }
    };
    let mut again = Command::new(program);
    again.args(arguments).env_clear();
    for (name, value) in std::env::vars_os() {
        if name.to_str().is_some_and(kept) {
            again.env(name, value);
        }
    }
    again
        .env(INSIDE, &root)
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("home/.local/share"))
        .env("XDG_CACHE_HOME", root.join("home/.cache"))
        .env("XDG_RUNTIME_DIR", root.join("run"))
        .env("GNUPGHOME", root.join("gnupg"))
        .env("GIT_CONFIG_GLOBAL", root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("BITBRAID_GIT", git_wrapper(&root, &real_git))
        .env("BITBRAID_NOW", NOW.to_string())
        .env("USERPROFILE", root.join("home"))
        .env("APPDATA", root.join("home/AppData/Roaming"))
        .env("LOCALAPPDATA", root.join("home/AppData/Local"))
        .env("TZ", "UTC")
        .env("LC_ALL", "C")
        .env("LANG", "C");
    let code = match again.status() {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => {
            if status.code().is_none() {
                println!("offscreen: the test process was ended: {status}");
            }
            ExitCode::FAILURE
        }
        Err(err) => {
            println!("offscreen: failed: cannot start the test process: {err}");
            ExitCode::FAILURE
        }
    };
    // Anything gpg started for the scratch home goes with it.
    let _ = Command::new("gpgconf")
        .args(["--kill", "all"])
        .env("GNUPGHOME", root.join("gnupg"))
        .output();
    if std::env::var_os("BITBRAID_OFFSCREEN_KEEP").is_none() {
        let _ = fs::remove_dir_all(&root);
    }
    drop(lock);
    code
}
