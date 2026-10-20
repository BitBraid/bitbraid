//! MinGit shipped beside the application on Windows, run where `PATH` has
//! no git: the binary laid out as the Windows packages lay it out
//! (`git\cmd\git.exe` beside the executable, `bitbraid_git::shipped::layout`),
//! with a `PATH` that holds no git and nothing of the developer's
//! configuration, asked which git it runs, to commit with a hook and to
//! start the terminal's shell. The hook, run by MinGit's own sh, finds
//! MinGit's git first on its `PATH`, and so does the terminal's shell.
//!
//! Opt-in: `BITBRAID_MINGIT` names MinGit unpacked (the release workflow's
//! `MinGit-<version>-64-bit.zip`); without it the test says it was
//! skipped. The stand-in of `shipped_git.rs` holds the same on Linux and
//! macOS without a real git.
//!
//! Sources: Git for Windows' documentation of MinGit (`cmd\git.exe`, its
//! `usr\bin\sh.exe`), git(1) (`--exec-path`), githooks(5)
//! (`post-commit`); `bitbraid_git::shipped` for the layout.
#![cfg(windows)]
#![allow(clippy::print_stdout)]

use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use bitbraid_testkit::{Scratch, scratch_root};
use serde_json::{Value, json};

/// MinGit unpacked, when the test is asked for.
fn mingit() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("BITBRAID_MINGIT")?);
    dir.join("cmd").join("git.exe").is_file().then_some(dir)
}

/// A folder copied as it is.
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let (source, target) = (entry.path(), to.join(entry.file_name()));
        if entry.file_type().unwrap().is_dir() {
            copy_tree(&source, &target);
        } else {
            fs::copy(&source, &target).unwrap();
        }
    }
}

/// The folders of a `PATH` written by MSYS2's sh (`/d/x:/tmp/y`), with
/// `/` between their parts: MSYS2 writes the temporary folder as `/tmp`,
/// so a folder is told by how it ends.
fn msys_folders(path: &str) -> Vec<String> {
    path.split(':')
        .filter(|part| !part.is_empty())
        .map(str::to_owned)
        .collect()
}

#[test]
fn mingit_shipped_beside_the_application_runs_where_path_has_none() {
    let Some(mingit) = mingit() else {
        println!("skipped: BITBRAID_MINGIT names no MinGit unpacked");
        return;
    };
    let root = scratch_root().join(format!("shipped-mingit-{}", std::process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    let exe = root.join("bitbraid-cli.exe");
    fs::copy(env!("CARGO_BIN_EXE_bitbraid-cli"), &exe).unwrap();
    copy_tree(&mingit, &root.join("git"));
    let (no_git, home) = (root.join("no-git"), root.join("home"));
    for dir in [&no_git, &home] {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(
        home.join(".gitconfig"),
        "[user]\n\tname = Ada Fixture\n\temail = ada@fixture.invalid\n",
    )
    .unwrap();
    // No git anywhere on it; the system's own folder, which every program
    // on Windows has.
    let system = PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32");
    let path = std::env::join_paths([&no_git, &system]).unwrap();
    let run = |dir: &Path, id: &str, params: Value| -> Value {
        let out = Command::new(&exe)
            .args(["run", id, "--params", &params.to_string()])
            .current_dir(dir)
            .env("PATH", &path)
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("GIT_CONFIG_GLOBAL", home.join(".gitconfig"))
            .env("BITBRAID_CONFIG_DIR", root.join("bitbraid-config"))
            .env_remove("BITBRAID_GIT")
            .env_remove("GIT_CONFIG_NOSYSTEM")
            .env_remove("GIT_CONFIG_SYSTEM")
            .env_remove("GIT_EXEC_PATH")
            .env_remove("GIT_ASKPASS")
            .env_remove("SSH_ASKPASS")
            .env_remove("SSH_AUTH_SOCK")
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{id}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    };

    // The shipped one runs, and says what it is.
    let shipped_git = root.join("git").join("cmd").join("git.exe");
    let info = run(&root, "git.info", json!({}));
    assert_eq!(info["shipped"], true, "{info}");
    assert_eq!(
        fs::canonicalize(info["path"].as_str().unwrap()).unwrap(),
        fs::canonicalize(&shipped_git).unwrap(),
        "{info}"
    );
    let version = info["version_text"].as_str().unwrap_or_default();
    assert!(version.contains(".windows."), "{info}");

    // A commit with a hook, which MinGit's sh runs: MinGit's git is the
    // one it finds, its folder before the application's PATH.
    let copy = Scratch::of("linear").unwrap();
    let said = root.join("hook-said.txt");
    let hook = copy.path().join(".git").join("hooks").join("post-commit");
    fs::write(
        &hook,
        format!(
            "#!/bin/sh\n{{ printf '%s\\n' \"$PATH\"; git --version; }} > '{}'\n",
            said.display().to_string().replace('\\', "/")
        ),
    )
    .unwrap();
    copy.write("shipped.txt", "committed by MinGit\n").unwrap();
    let made = run(
        copy.path(),
        "commit.create",
        json!({ "message": "made with MinGit", "stage_all": true }),
    );
    assert_eq!(made["id"], copy.git().rev_parse("HEAD").unwrap().as_str());
    let text = fs::read_to_string(&said).unwrap();
    let mut lines = text.lines();
    let hook_path = msys_folders(lines.next().unwrap_or_default());
    assert_eq!(lines.next().map(str::trim), Some(version.trim()), "{text}");
    let cmd = shipped_git.parent().unwrap().to_path_buf();
    let run_name = root.file_name().unwrap().to_string_lossy().into_owned();
    let at = |ending: &str| {
        hook_path
            .iter()
            .position(|seen| seen.ends_with(&format!("{run_name}/{ending}")))
    };
    let (shipped_at, rest_at) = (at("git/cmd"), at("no-git"));
    assert!(
        shipped_at.is_some() && shipped_at < rest_at,
        "{hook_path:?}"
    );

    // The terminal's shell has MinGit's folder first.
    let terminal = run(copy.path(), "terminal.shell", json!({}));
    // Named as the environment writes it, which on Windows may be `Path`.
    let shell_path = terminal["env"]
        .as_object()
        .and_then(|env| {
            env.iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("PATH"))
        })
        .and_then(|(_, value)| value.as_str())
        .unwrap_or_default();
    // In the usual form: the command prompt and PowerShell do not look in
    // a folder written `\\?\C:\…`.
    let first = std::env::split_paths(shell_path).next();
    assert_eq!(first.as_deref(), Some(cmd.as_path()), "{shell_path}");

    let _ = fs::remove_dir_all(&root);
}
