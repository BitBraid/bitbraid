//! gitoxide reads the system configuration of the git that is chosen, not
//! of the `git` on `PATH` or of whatever it would think of itself: what
//! the application reads and what its git children do come from the same
//! files.
//!
//! The executable is chosen for the whole process, so everything is in one
//! test, and this file is a test binary of its own.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

use std::{
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use bitbraid_git::{
    Repository, exe,
    system_config::{SystemConfig, system_config, system_config_of},
};
use bitbraid_testkit::Scratch;

/// An executable that is a git installed elsewhere: it runs the real git
/// with a system configuration of its own.
fn installed_git(dir: &Path, name: &str, system: &Path) -> OsString {
    // The tests may keep git children from the machine's system
    // configuration (`GIT_CONFIG_NOSYSTEM`); this git reads its own.
    let body = format!(
        "#!/bin/sh\nunset GIT_CONFIG_NOSYSTEM\nGIT_CONFIG_SYSTEM='{}' exec git \"$@\"\n",
        system.display()
    );
    bitbraid_testkit::script(dir, name, &body)
        .unwrap()
        .into_os_string()
}

/// What gitoxide has for a key in a repository that is opened now.
fn read(repo: &Path, key: &str) -> Option<String> {
    let repo = Repository::open(repo).unwrap();
    repo.gix()
        .config_snapshot()
        .string(key)
        .map(|value| value.to_string())
}

/// What the git that is chosen has for it, asked as a child.
fn asked(repo: &Path, key: &str) -> Option<String> {
    let repo = Repository::open(repo).unwrap();
    exe::config_get(&repo, key)
}

#[test]
fn gitoxide_reads_the_system_configuration_of_the_git_that_is_chosen() {
    let scratch = Scratch::of("linear").unwrap();
    let repo = scratch.path().to_path_buf();
    let tools: PathBuf = repo.join(".git").join("bitbraid-test-tools");
    fs::create_dir_all(&tools).unwrap();

    // Two gits, each with a system configuration of its own, and one
    // whose system configuration is not there.
    let (first_file, second_file) = (
        tools.join("first.gitconfig"),
        tools.join("second.gitconfig"),
    );
    fs::write(
        &first_file,
        "[bitbraid]\n\tprobe = from the first git\n[include]\n\tpath = included.gitconfig\n",
    )
    .unwrap();
    fs::write(
        tools.join("included.gitconfig"),
        "[bitbraid]\n\tincluded = yes\n",
    )
    .unwrap();
    fs::write(&second_file, "[bitbraid]\n\tprobe = from the second git\n").unwrap();
    let first = installed_git(&tools, "first-git", &first_file);
    let second = installed_git(&tools, "second-git", &second_file);
    let bare = installed_git(&tools, "bare-git", &tools.join("no-such.gitconfig"));

    // Each says where its system configuration is; what it includes is
    // not a place of its own.
    assert_eq!(
        system_config_of(&first),
        SystemConfig::Files(vec![first_file.clone()])
    );
    assert_eq!(
        system_config_of(&second),
        SystemConfig::Files(vec![second_file.clone()])
    );
    assert_eq!(system_config_of(&bare), SystemConfig::None);
    // What is no git, or is not there, has no answer; nothing fails.
    // `/bin` on Linux, `/usr/bin` alone on macOS.
    let unix_program = |name: &str| {
        ["/bin", "/usr/bin"]
            .iter()
            .map(|dir| format!("{dir}/{name}"))
            .find(|path| std::path::Path::new(path).is_file())
            .unwrap()
    };
    let truthful = if cfg!(unix) {
        unix_program("true")
    } else {
        String::new()
    };
    let programs: Vec<String> = if cfg!(unix) {
        vec![
            unix_program("false"),
            truthful.clone(),
            "/no/such/program".to_owned(),
            String::new(),
        ]
    } else {
        vec!["/no/such/program".to_owned(), String::new()]
    };
    for program in programs {
        assert_eq!(
            system_config_of(&OsString::from(&program)),
            if cfg!(unix) && program == truthful {
                // It "answers", with nothing: no file.
                SystemConfig::None
            } else {
                SystemConfig::Unknown
            },
            "{program:?}"
        );
    }

    // Before anything is chosen the probe is nowhere: it is in no file the
    // default git reads.
    assert_eq!(read(&repo, "bitbraid.probe"), None);

    // The first git is chosen: gitoxide reads its file, with what the file
    // includes, and so does the git child. The two agree.
    exe::set_git_binary(Some(first.clone()));
    assert_eq!(system_config(), SystemConfig::Files(vec![first_file]));
    assert_eq!(
        read(&repo, "bitbraid.probe").as_deref(),
        Some("from the first git")
    );
    assert_eq!(read(&repo, "bitbraid.included").as_deref(), Some("yes"));
    assert_eq!(
        asked(&repo, "bitbraid.probe"),
        read(&repo, "bitbraid.probe")
    );
    // What the repository itself says still wins over the system's.
    scratch
        .git()
        .run(["config", "bitbraid.probe", "from the repository"])
        .unwrap();
    assert_eq!(
        read(&repo, "bitbraid.probe").as_deref(),
        Some("from the repository")
    );
    assert_eq!(
        asked(&repo, "bitbraid.probe"),
        read(&repo, "bitbraid.probe")
    );
    scratch
        .git()
        .run(["config", "--unset", "bitbraid.probe"])
        .unwrap();

    // A repository that is open now, under the first git.
    let held = |open: &Repository, key: &str| {
        open.gix()
            .config_snapshot()
            .string(key)
            .map(|value| value.to_string())
    };
    let mut open = Repository::open(&repo).unwrap();
    assert!(open.follows_git());
    assert!(!open.follow_git().unwrap(), "nothing to follow yet");
    assert_eq!(
        held(&open, "bitbraid.probe").as_deref(),
        Some("from the first git")
    );
    // One in a folder that will be gone by the time it is to follow.
    let doomed = Scratch::of("linear").unwrap();
    let mut lost = Repository::open(doomed.path()).unwrap();
    let doomed_at = doomed.path().to_path_buf();
    drop(doomed);
    let _ = fs::remove_dir_all(&doomed_at);

    // Another git is chosen: the next repository that is opened has that
    // one's file, and nothing of the first.
    exe::set_git_binary(Some(second.clone()));
    assert_eq!(system_config(), SystemConfig::Files(vec![second_file]));

    // The one that was open has not looked yet: it holds what it read.
    assert!(!open.follows_git());
    assert_eq!(
        held(&open, "bitbraid.probe").as_deref(),
        Some("from the first git")
    );
    // Following, it is opened again and reads what the git that is run
    // reads; a second time there is nothing left to follow.
    assert!(open.follow_git().unwrap());
    assert!(open.follows_git());
    assert_eq!(
        held(&open, "bitbraid.probe").as_deref(),
        Some("from the second git")
    );
    assert_eq!(held(&open, "bitbraid.included"), None);
    assert_eq!(
        asked(&repo, "bitbraid.probe"),
        held(&open, "bitbraid.probe")
    );
    assert!(!open.follow_git().unwrap());
    // It is the same repository still.
    assert_eq!(
        open.git_dir().canonicalize().unwrap(),
        repo.join(".git").canonicalize().unwrap()
    );
    // Choosing the same executable again is no other choice.
    exe::set_git_binary(Some(second));
    assert!(open.follows_git());
    // The one whose folder is gone cannot be opened again: that is said
    // once, the handle is kept, and it is not tried at every turn.
    assert!(!lost.follows_git());
    assert!(lost.follow_git().is_err());
    assert!(lost.follows_git());
    assert!(!lost.follow_git().unwrap());
    assert_eq!(
        read(&repo, "bitbraid.probe").as_deref(),
        Some("from the second git")
    );
    assert_eq!(read(&repo, "bitbraid.included"), None);
    assert_eq!(
        asked(&repo, "bitbraid.probe"),
        read(&repo, "bitbraid.probe")
    );

    // A git without a system configuration: gitoxide reads none either,
    // and the repository opens as ever.
    exe::set_git_binary(Some(bare));
    assert_eq!(system_config(), SystemConfig::None);
    assert_eq!(read(&repo, "bitbraid.probe"), None);
    assert_eq!(asked(&repo, "bitbraid.probe"), None);
    assert!(Repository::open(&repo).unwrap().summary().is_ok());

    // Something that is no git at all: no answer, and a repository still
    // opens and is read (gitoxide needs no git for that).
    exe::set_git_binary(Some("/bin/false".into()));
    assert_eq!(system_config(), SystemConfig::Unknown);
    assert!(Repository::open(&repo).unwrap().summary().is_ok());

    // Back to the default.
    exe::set_git_binary(None);
    assert_eq!(read(&repo, "bitbraid.probe"), None);
}
