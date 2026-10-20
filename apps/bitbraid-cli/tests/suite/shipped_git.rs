//! A git shipped beside the application, run where `PATH` has none. The
//! binary is laid out as the application would be shipped on this system
//! (`bitbraid_git::shipped::layout`: `bin/` beside
//! `libexec/bitbraid/git/` on Linux, `Contents/MacOS` beside
//! `Contents/Resources/git` in a bundle on macOS), with a `PATH` that holds
//! no git and nothing of the developer's configuration, and asked which
//! git it runs, to commit and to read; what it did is held against the git
//! of the tests.
//!
//! The git shipped is a stand-in at first: a script that runs the git of
//! the tests with the system configuration of the shipped installation
//! (`etc/gitconfig` beside its `bin`), and writes down that it was run.
//! Where `BITBRAID_RELOCATABLE_GIT` names an installation of git built
//! from git's release with `RUNTIME_PREFIX=YesPlease` (`make
//! prefix=<dir> RUNTIME_PREFIX=YesPlease install`), that installation is
//! laid out instead and the same is asked of it, and also that it finds
//! its programs, templates and system configuration from where it was
//! put, not where it was built. Without the variable that part says it
//! was skipped.
//!
//! On macOS `PATH` always has a git: `/usr/bin/git`, which stands in for
//! the git of Apple's developer tools (`bitbraid_git::stand_in`). With
//! `DEVELOPER_DIR` naming a folder that is not there, as `xcode-select -p`
//! then says, the tools are not there, and it is held that the stand-in
//! counts as no git and is never run: the shipped git runs, or one further
//! along `PATH`, and where there is neither what fails says what is
//! missing; named as the git to run, it is refused without being asked.
//! (Run all the same, it would say the folder is missing and offer
//! nothing.)
//!
//! Sources: git(1) (`--exec-path`), git-init(1) (TEMPLATE DIRECTORY),
//! git-config(1) (FILES, `GIT_CONFIG_SYSTEM`), git's `INSTALL` on
//! `RUNTIME_PREFIX`; `bitbraid_git::shipped` for the layouts;
//! xcode-select(1) (`-p`, `DEVELOPER_DIR`).
#![cfg(unix)]
#![allow(clippy::print_stdout)]

use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

use bitbraid_git::shipped::{System, layout};
use bitbraid_testkit::{Git, Scratch, scratch_root};
use serde_json::{Value, json};

/// The git the tests run, as a full path: the stand-in runs it from a
/// `PATH` that has no git.
fn real_git() -> PathBuf {
    let name = bitbraid_testkit::oracle::git_binary();
    let as_path = PathBuf::from(&name);
    if as_path.is_absolute() {
        return as_path;
    }
    std::env::var_os("PATH")
        .and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|dir| dir.join(&name))
                .find(|candidate| candidate.is_file())
        })
        .expect("the tests' git is on PATH")
}

/// A file in place: hard-linked where both are on one file system, else
/// copied, with its permissions.
fn place(from: &Path, to: &Path) {
    if fs::hard_link(from, to).is_err() {
        fs::copy(from, to).unwrap();
    }
}

/// Runs a program this test wrote (a copy of the binary, where the
/// scratch directory is on another file system than the target directory
/// and cannot take a link; the stand-in git) and waits for it. A child
/// that a test beside this one forked while the file was still open for
/// writing here holds it open until it starts its own program, and
/// starting this one meanwhile fails with "text file busy" (ETXTBSY): it
/// is started again then, for up to five seconds.
fn output_of(cmd: &mut Command) -> Output {
    let mut tries = 0;
    loop {
        match cmd.output() {
            Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy && tries < 250 => {
                tries += 1;
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            result => return result.unwrap(),
        }
    }
}

/// An installation copied as it is: its links stay links.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        let kind = entry.file_type().unwrap();
        if kind.is_symlink() {
            std::os::unix::fs::symlink(fs::read_link(&source).unwrap(), &target).unwrap();
        } else if kind.is_dir() {
            copy_tree(&source, &target);
        } else {
            place(&source, &target);
        }
    }
}

/// The application as it would be shipped, in a directory of its own.
struct Shipped {
    root: PathBuf,
    /// The binary, where the application's executable would be.
    exe: PathBuf,
    /// The shipped git's executable, where the layout puts it.
    git: PathBuf,
    /// The top of the shipped git's installation (above its `bin`).
    prefix: PathBuf,
    /// A directory for `PATH` that has no git.
    no_git: PathBuf,
    /// The home and configuration of the runs.
    home: PathBuf,
}

impl Shipped {
    fn new(name: &str) -> Self {
        let root = scratch_root().join(format!("shipped-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let exe_dir = match System::this() {
            System::MacOs => root.join("BitBraid.app").join("Contents").join("MacOS"),
            _ => root.join("bin"),
        };
        fs::create_dir_all(&exe_dir).unwrap();
        let exe = exe_dir.join("bitbraid-cli");
        place(Path::new(env!("CARGO_BIN_EXE_bitbraid-cli")), &exe);
        let git = layout(System::this(), &exe).unwrap();
        let prefix = git.parent().unwrap().parent().unwrap().to_path_buf();
        let (no_git, home) = (root.join("no-git"), root.join("home"));
        for dir in [&no_git, &home] {
            fs::create_dir_all(dir).unwrap();
        }
        fs::write(
            home.join(".gitconfig"),
            "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n[init]\n\tdefaultBranch = main\n",
        )
        .unwrap();
        Self {
            root,
            exe,
            git,
            prefix,
            no_git,
            home,
        }
    }

    /// The stand-in: the tests' git, with the shipped installation's
    /// system configuration, writing down each time it is run.
    fn stand_in(&self) -> PathBuf {
        let bin = self.git.parent().unwrap();
        fs::create_dir_all(bin).unwrap();
        fs::create_dir_all(self.prefix.join("etc")).unwrap();
        let log = self.root.join("shipped-runs.log");
        // A shipped git needs no developer folder; the tests' git may be
        // macOS's stand-in, which does.
        let body = format!(
            "#!/bin/sh\necho \"$*\" >> '{}'\nunset GIT_CONFIG_NOSYSTEM DEVELOPER_DIR\nGIT_CONFIG_SYSTEM='{}'\nexport GIT_CONFIG_SYSTEM\nexec '{}' \"$@\"\n",
            log.display(),
            self.prefix.join("etc").join("gitconfig").display(),
            real_git().display()
        );
        fs::write(&self.git, body).unwrap();
        fs::set_permissions(&self.git, fs::Permissions::from_mode(0o755)).unwrap();
        log
    }

    /// The binary run in `dir` with `path` for `PATH`: no git of the
    /// developer's, no configuration of theirs, no system configuration
    /// but the shipped git's own.
    fn cli(&self, dir: &Path, path: &Path) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.current_dir(dir)
            .env("PATH", path)
            .env("HOME", &self.home)
            .env("XDG_CONFIG_HOME", self.home.join(".config"))
            .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
            .env("BITBRAID_CONFIG_DIR", self.root.join("bitbraid-config"))
            .env_remove("BITBRAID_GIT")
            .env_remove("GIT_CONFIG_NOSYSTEM")
            .env_remove("GIT_CONFIG_SYSTEM")
            .env_remove("GIT_EXEC_PATH")
            .env_remove("GIT_TEMPLATE_DIR")
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("SSH_AUTH_SOCK")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    /// `run <id>` with `PATH` holding no git: its JSON.
    fn run(&self, dir: &Path, id: &str, params: Value) -> Value {
        self.run_with_path(dir, &self.no_git, id, params)
    }

    fn run_with_path(&self, dir: &Path, path: &Path, id: &str, params: Value) -> Value {
        let out: Output =
            output_of(
                self.cli(dir, path)
                    .args(["run", id, "--params", &params.to_string()]),
            );
        assert!(
            out.status.success(),
            "{id} {params}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    /// The shipped git asked directly, with nothing of the developer's.
    fn ask_shipped(&self, args: &[&str]) -> String {
        let out = output_of(
            Command::new(&self.git)
                .args(args)
                .current_dir(&self.root)
                .env("PATH", &self.no_git)
                .env("HOME", &self.home)
                .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
                .env_remove("GIT_CONFIG_NOSYSTEM")
                .env_remove("GIT_EXEC_PATH")
                .env_remove("GIT_TEMPLATE_DIR")
                .stdin(Stdio::null()),
        );
        assert!(
            out.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    /// Which git the binary runs, from where it is laid out, and that
    /// every git child of a commit and of the reads is that one: the
    /// commit is there for the tests' git, and a remote that only the
    /// shipped installation's system configuration names is seen both by
    /// what gitoxide reads and by git.
    fn check(&self, version_text: &str) {
        let shipped = fs::canonicalize(&self.git).unwrap();
        let info = self.run(&self.root, "git.info", json!({}));
        assert_eq!(info["shipped"], true, "{info}");
        assert_eq!(info["chosen"], false, "{info}");
        assert_eq!(
            fs::canonicalize(info["path"].as_str().unwrap()).unwrap(),
            shipped,
            "{info}"
        );
        assert_eq!(info["version_text"], version_text, "{info}");

        // The installation's system configuration names a remote.
        fs::create_dir_all(self.prefix.join("etc")).unwrap();
        fs::write(
            self.prefix.join("etc").join("gitconfig"),
            "[remote \"from-the-shipped-system\"]\n\turl = https://example.invalid/shipped.git\n",
        )
        .unwrap();

        // A commit in a copy of a fixture, made through the shipped git.
        let copy = Scratch::of("linear").unwrap();
        let before = copy.git().rev_parse("HEAD").unwrap();
        copy.write("shipped.txt", "committed by the shipped git\n")
            .unwrap();
        let made = self.run(
            copy.path(),
            "commit.create",
            json!({ "message": "made with the shipped git", "stage_all": true }),
        );
        let oracle = copy.git();
        let head = oracle.rev_parse("HEAD").unwrap();
        assert_eq!(made["id"], head.as_str());
        assert_eq!(
            oracle.rev_parse("HEAD~1").unwrap(),
            before,
            "one commit on top"
        );
        assert_eq!(
            oracle
                .run(["log", "-1", "--format=%s%x00%an"])
                .unwrap()
                .trim(),
            "made with the shipped git\0Ada Fixture"
        );

        // What gitoxide reads (the remotes of `repo.refs`) has the remote
        // of the shipped system configuration; so has the shipped git.
        let refs = self.run(copy.path(), "repo.refs", json!({}));
        let names: Vec<String> = refs["remotes"]
            .as_array()
            .unwrap_or_else(|| panic!("no remotes in {refs}"))
            .iter()
            .filter_map(|remote| remote["name"].as_str().map(str::to_owned))
            .collect();
        assert!(
            names.iter().any(|name| name == "from-the-shipped-system"),
            "{refs}"
        );
        let asked = output_of(
            Command::new(&self.git)
                .args(["remote"])
                .current_dir(copy.path())
                .env("PATH", &self.no_git)
                .env("HOME", &self.home)
                .env("GIT_CONFIG_GLOBAL", self.home.join(".gitconfig"))
                .env_remove("GIT_CONFIG_NOSYSTEM"),
        );
        assert!(String::from_utf8_lossy(&asked.stdout).contains("from-the-shipped-system"));
        // The tests' git, without that file, knows no such remote.
        assert!(
            !oracle
                .run(["remote"])
                .unwrap()
                .contains("from-the-shipped-system")
        );

        // With a git on PATH that one runs, and the shipped one is not
        // said to be in use.
        let other_bin = self.root.join("other-bin");
        fs::create_dir_all(&other_bin).unwrap();
        let other = other_bin.join("git");
        fs::write(
            &other,
            format!("#!/bin/sh\nexec '{}' \"$@\"\n", real_git().display()),
        )
        .unwrap();
        fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();
        let info = self.run_with_path(&self.root, &other_bin, "git.info", json!({}));
        assert_eq!(info["shipped"], false, "{info}");
        assert_eq!(
            fs::canonicalize(info["path"].as_str().unwrap()).unwrap(),
            fs::canonicalize(&other).unwrap(),
            "{info}"
        );
    }
}

impl Drop for Shipped {
    fn drop(&mut self) {
        if self.root.starts_with(scratch_root()) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }
}

#[test]
fn a_git_shipped_beside_the_application_runs_where_path_has_none() {
    let shipped = Shipped::new("stand-in");
    let log = shipped.stand_in();
    let version_text = bitbraid_testkit::oracle::git_version().unwrap();
    shipped.check(&version_text);
    // Every git child went through it.
    let runs = fs::read_to_string(&log).unwrap_or_default();
    for command in ["--version", "commit", "config"] {
        assert!(
            runs.lines()
                .any(|line| line.split(' ').any(|word| word == command)),
            "{command} did not go through the shipped git: {runs}"
        );
    }

    // Nothing shipped and nothing on PATH: no git, and it says so.
    fs::remove_file(&shipped.git).unwrap();
    let out = output_of(
        shipped
            .cli(&shipped.root, &shipped.no_git)
            .args(["run", "git.info"]),
    );
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("cannot run git"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The folders of a `PATH`.
fn folders(path: &str) -> Vec<PathBuf> {
    std::env::split_paths(path).collect()
}

/// While the shipped git is the one in use its folder stands first on the
/// `PATH` of what the application starts: a hook git runs, a program the
/// user's command names, the terminal's shell; with a git on `PATH` it
/// does not.
#[test]
fn the_shipped_gits_folder_is_first_on_the_path_of_what_is_started() {
    let shipped = Shipped::new("on-path");
    shipped.stand_in();
    let bin = shipped.git.parent().unwrap().to_path_buf();
    let copy = Scratch::of("linear").unwrap();

    // A hook, run by git: after git's own programs, which git puts first,
    // the shipped folder, then the PATH the application had.
    let hook_said = shipped.root.join("hook-path.txt");
    let hook = copy.path().join(".git/hooks/post-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\nprintf '%s' \"$PATH\" > '{}'\n",
            hook_said.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o755)).unwrap();
    copy.write("hooked.txt", "a hook runs\n").unwrap();
    shipped.run(
        copy.path(),
        "commit.create",
        json!({ "message": "with a hook", "stage_all": true }),
    );
    let hook_path = folders(&fs::read_to_string(&hook_said).unwrap());
    let at = |folder: &Path| hook_path.iter().position(|seen| seen == folder);
    let (shipped_at, rest_at) = (at(&bin), at(&shipped.no_git));
    assert!(
        shipped_at.is_some() && shipped_at < rest_at,
        "{hook_path:?}"
    );

    // The terminal's shell.
    let terminal = shipped.run(copy.path(), "terminal.shell", json!({}));
    let path = terminal["env"]["PATH"].as_str().unwrap_or_default();
    assert_eq!(
        folders(path),
        [bin.clone(), shipped.no_git.clone()],
        "{terminal}"
    );

    // A program a command of the user's names, started and left running.
    // Its PATH has nothing but the two folders: `mv` by its full path.
    let said = shipped.root.join("program-path.txt");
    let program = shipped.root.join("record-path");
    fs::write(
        &program,
        format!(
            "#!/bin/sh\nprintf '%s' \"$PATH\" > '{}.part'\n/bin/mv '{0}.part' '{0}'\n",
            said.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    shipped.run(
        copy.path(),
        "repo.open_folder",
        json!({ "command": format!("{} {{path}}", program.display()) }),
    );
    let started = std::time::Instant::now();
    while !said.exists() && started.elapsed() < std::time::Duration::from_secs(20) {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_eq!(
        folders(&fs::read_to_string(&said).unwrap()),
        [bin.clone(), shipped.no_git.clone()]
    );

    // A git on PATH is the one in use: nothing is put before it.
    let other_bin = shipped.root.join("other-bin");
    fs::create_dir_all(&other_bin).unwrap();
    let other = other_bin.join("git");
    fs::write(
        &other,
        format!("#!/bin/sh\nexec '{}' \"$@\"\n", real_git().display()),
    )
    .unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();
    let terminal = shipped.run_with_path(copy.path(), &other_bin, "terminal.shell", json!({}));
    let path = terminal["env"]["PATH"].as_str().unwrap_or_default();
    assert_eq!(folders(path), [other_bin], "{terminal}");
}

#[test]
fn a_relocatable_git_from_the_release_runs_from_where_it_is_laid_out() {
    let Some(built) = std::env::var_os("BITBRAID_RELOCATABLE_GIT").map(PathBuf::from) else {
        println!(
            "skipped: set BITBRAID_RELOCATABLE_GIT to a git installed with \
             `make prefix=<dir> RUNTIME_PREFIX=YesPlease install`"
        );
        return;
    };
    assert!(built.join("bin").join("git").is_file(), "{built:?}");
    let shipped = Shipped::new("relocatable");
    copy_tree(&built, &shipped.prefix);
    // Asked from where it was put: its programs, and its version.
    let exec_path = shipped.ask_shipped(&["--exec-path"]);
    assert!(
        Path::new(&exec_path).starts_with(fs::canonicalize(&shipped.prefix).unwrap())
            || Path::new(&exec_path).starts_with(&shipped.prefix),
        "{exec_path}"
    );
    assert!(!Path::new(&exec_path).starts_with(&built), "{exec_path}");
    let version_text = shipped.ask_shipped(&["--version"]);
    shipped.check(&version_text);

    // A repository made through it starts from its own templates.
    let made = shipped.root.join("made");
    shipped.run(
        &shipped.root,
        "repo.init",
        json!({ "path": made.to_str().unwrap() }),
    );
    let hooks = made.join(".git").join("hooks");
    assert!(
        hooks.join("pre-commit.sample").is_file(),
        "no templates in {hooks:?}"
    );
    let git = Git::new(made.clone(), shipped.home.clone());
    assert_eq!(git.run(["rev-parse", "--git-dir"]).unwrap().trim(), ".git");
}

/// macOS's `/usr/bin/git` with no developer tools behind it: no git, and
/// never run (see the module's documentation).
#[cfg(target_os = "macos")]
#[test]
fn macos_stand_in_without_the_tools_is_no_git_and_is_never_run() {
    use bitbraid_git::stand_in::STAND_IN;

    let shipped = Shipped::new("stand-in-idle");
    let log = shipped.stand_in();
    let no_tools = shipped.root.join("no-developer-tools");
    let path = |dirs: &[&Path]| std::env::join_paths(dirs).unwrap();
    let run = |dir_path: &std::ffi::OsStr, id: &str, params: Value| -> Output {
        output_of(
            shipped
                .cli(&shipped.root, Path::new(dir_path))
                .env("DEVELOPER_DIR", &no_tools)
                .args(["run", id, "--params", &params.to_string()]),
        )
    };
    let json_of = |out: &Output| -> Value {
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };
    let usr_bin = Path::new("/usr/bin");

    // Only the stand-in on PATH: the shipped git runs.
    let only_stand_in = path(&[&shipped.no_git, usr_bin]);
    let info = json_of(&run(&only_stand_in, "git.info", json!({})));
    assert_eq!(info["shipped"], true, "{info}");
    assert_eq!(
        fs::canonicalize(info["path"].as_str().unwrap()).unwrap(),
        fs::canonicalize(&shipped.git).unwrap(),
        "{info}"
    );
    let copy = Scratch::of("linear").unwrap();
    copy.write("idle.txt", "committed past the stand-in\n")
        .unwrap();
    let made = output_of(
        shipped
            .cli(copy.path(), Path::new(&only_stand_in))
            .env("DEVELOPER_DIR", &no_tools)
            .args([
                "run",
                "commit.create",
                "--params",
                &json!({ "message": "past the stand-in", "stage_all": true }).to_string(),
            ]),
    );
    let made = json_of(&made);
    assert_eq!(made["id"], copy.git().rev_parse("HEAD").unwrap().as_str());
    let runs = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        runs.lines().any(|line| line.starts_with("commit")),
        "{runs}"
    );

    // A git further along PATH than the stand-in runs, by its place.
    let other_bin = shipped.root.join("other-bin");
    fs::create_dir_all(&other_bin).unwrap();
    let other = other_bin.join("git");
    fs::write(
        &other,
        format!(
            "#!/bin/sh\nunset DEVELOPER_DIR\nexec '{}' \"$@\"\n",
            real_git().display()
        ),
    )
    .unwrap();
    fs::set_permissions(&other, fs::Permissions::from_mode(0o755)).unwrap();
    let info = json_of(&run(&path(&[usr_bin, &other_bin]), "git.info", json!({})));
    assert_eq!(info["shipped"], false, "{info}");
    assert_eq!(info["program"], other.to_str().unwrap(), "{info}");

    // Named as the git to run, it is refused without being run.
    let out = run(&only_stand_in, "git.info", json!({ "program": STAND_IN }));
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    assert!(said.contains("Command Line Tools"), "{said}");
    assert!(!said.contains("DEVELOPER_DIR"), "it was run: {said}");
    let out = run(&only_stand_in, "git.use", json!({ "program": "git" }));
    assert!(!out.status.success());
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("xcode-select --install"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Nothing shipped, and only the stand-in: the git it would run is
    // named, with what to do, and the stand-in is not run.
    fs::remove_file(&shipped.git).unwrap();
    let out = run(&only_stand_in, "git.info", json!({}));
    assert!(!out.status.success());
    let said = String::from_utf8_lossy(&out.stderr);
    let missing = no_tools.join("usr").join("bin").join("git");
    assert!(
        said.contains(&format!("cannot run {}", missing.display())),
        "{said}"
    );
    assert!(said.contains("xcode-select --install"), "{said}");
    assert!(!said.contains("xcrun"), "it was run: {said}");

    // With the tools there (as `xcode-select -p` says without
    // DEVELOPER_DIR), the stand-in is git on PATH like any other.
    let tools = Command::new("/usr/bin/xcode-select")
        .arg("-p")
        .env_remove("DEVELOPER_DIR")
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
        .filter(|folder| folder.join("usr/bin/git").is_file());
    if tools.is_none() {
        println!("the tools are not installed here: the stand-in with them is not tried");
        return;
    }
    let out = output_of(
        shipped
            .cli(&shipped.root, Path::new(&only_stand_in))
            .env_remove("DEVELOPER_DIR")
            .args(["run", "git.info"]),
    );
    let info = json_of(&out);
    assert_eq!(info["shipped"], false, "{info}");
    assert_eq!(info["path"], STAND_IN, "{info}");
}
