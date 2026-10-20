//! Quoted paths read back by real shells: what `quote_path` writes, the
//! shell hands to `printf` as one argument, byte for byte. `/bin/sh`
//! always; bash, zsh, dash, mksh, ksh, fish, nushell and tcsh when they are
//! installed, or in the folder `BITBRAID_TEST_SHELLS` names.
#![cfg(unix)]

use std::{
    ffi::OsStr,
    os::unix::ffi::OsStrExt as _,
    path::{Path, PathBuf},
    process::Command,
};

use bitbraid_terminal::{ShellKind, quote_path, quote_paths};

fn hostile() -> Vec<Vec<u8>> {
    let mut names: Vec<Vec<u8>> = [
        "plain.txt",
        "with space",
        "  two  spaces  ",
        "it's",
        "''",
        "say \"hi\"",
        "$HOME",
        "${x:-y}",
        "`id`",
        "$(touch pwned)",
        "a\nb",
        "\n",
        "tab\there",
        "-n",
        "--help",
        "-",
        "*",
        "?[a-z]",
        "~",
        "~root",
        "=ls",
        "a\\b",
        "\\",
        "!event",
        "#comment",
        "a;b&c|d",
        "<in >out",
        "(sub)",
        "{a,b}",
        "%1",
        "ctrl\u{1}\u{7}\u{8}\u{1b}[31m\u{7f}",
        "c1\u{85}\u{9b}",
        "grüße 世界 👋",
        "/abs/path/to dir/file",
        "",
    ]
    .iter()
    .map(|name| name.as_bytes().to_vec())
    .collect();
    names.push(b"bad\xffbyte".to_vec());
    names.push(b"\x80\xfe".to_vec());
    names.push(b"mixed \xc3 'quote'\n".to_vec());
    names
}

/// What the shell `program` prints for `printf '%s\n' <quoted>`.
fn printed(program: &str, quoted: &str) -> Vec<u8> {
    let script = format!("printf '%s\\n' {quoted}");
    let out = Command::new(program)
        .arg("-c")
        .arg(&script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", "/nonexistent")
        .env("LC_ALL", "C.UTF-8")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{program} -c {script:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// A shell of the system's, or of the folder `BITBRAID_TEST_SHELLS`
/// names (the opt-in tests of `shells.rs`).
fn installed(program: &str) -> Option<PathBuf> {
    let asked = std::env::var_os("BITBRAID_TEST_SHELLS").map(PathBuf::from);
    asked
        .into_iter()
        .chain(["/usr/bin", "/bin", "/usr/local/bin"].map(PathBuf::from))
        .map(|dir| dir.join(program))
        .find(|path| path.is_file())
}

/// Whether a POSIX shell knows `$'...'` (POSIX.1-2024): a shell that does
/// not is skipped for names that need it.
fn knows_dollar_quotes(program: &str) -> bool {
    printed(program, "$'a\\101'") == b"aA\n"
}

fn round_trip(program: &str, kind: ShellKind) {
    let dollar = kind != ShellKind::Posix || knows_dollar_quotes(program);
    let mut checked = 0;
    for name in hostile() {
        let path = Path::new(OsStr::from_bytes(&name));
        let quoted = quote_path(path, kind);
        if !dollar && quoted.contains("$'") {
            continue;
        }
        let mut expected = name.clone();
        expected.push(b'\n');
        assert_eq!(
            printed(program, &quoted),
            expected,
            "{program}: {:?} quoted as {quoted:?}",
            String::from_utf8_lossy(&name)
        );
        checked += 1;
    }
    assert!(checked >= 30, "{program}: only {checked} names were tried");
}

#[test]
fn posix_shells_read_their_paths_back() {
    round_trip("/bin/sh", ShellKind::Posix);
    for shell in ["bash", "zsh", "dash", "mksh", "ksh"] {
        if let Some(path) = installed(shell) {
            round_trip(&path.display().to_string(), ShellKind::Posix);
        }
    }
}

#[test]
fn fish_reads_its_paths_back() {
    if let Some(fish) = installed("fish") {
        round_trip(&fish.display().to_string(), ShellKind::Fish);
    }
}

/// What a shell that cannot hold every name prints for it: nushell's
/// strings are UTF-8 (what is not becomes the replacement character), and
/// a C shell has no way to write a control character or such a byte at
/// its prompt (they are left out).
fn as_written(name: &[u8], kind: ShellKind) -> Vec<u8> {
    match kind {
        ShellKind::Nushell => String::from_utf8_lossy(name).into_owned().into_bytes(),
        ShellKind::Csh => String::from_utf8_lossy(name)
            .chars()
            .filter(|c| !c.is_control() && *c != char::REPLACEMENT_CHARACTER)
            .collect::<String>()
            .into_bytes(),
        _ => name.to_vec(),
    }
}

/// `print` in nushell, `printf` in tcsh: each name comes back as it can
/// be written there.
#[test]
fn nushell_and_tcsh_read_their_paths_back() {
    for (program, kind) in [("nu", ShellKind::Nushell), ("tcsh", ShellKind::Csh)] {
        let Some(path) = installed(program) else {
            continue;
        };
        for name in hostile() {
            // tcsh keeps characters in sixteen bits: one beyond U+FFFF
            // (an emoji) comes out mangled, however it is quoted.
            if kind == ShellKind::Csh
                && String::from_utf8_lossy(&name)
                    .chars()
                    .any(|c| u32::from(c) > 0xffff)
            {
                continue;
            }
            let quoted = quote_path(Path::new(OsStr::from_bytes(&name)), kind);
            let (args, script) = match kind {
                ShellKind::Nushell => (vec!["--no-config-file", "-c"], format!("print {quoted}")),
                _ => (vec!["-f", "-c"], format!("printf '%s\\n' {quoted}")),
            };
            let out = Command::new(&path)
                .args(&args)
                .arg(&script)
                .env_clear()
                .env("PATH", "/usr/bin:/bin")
                .env("HOME", "/nonexistent")
                .env("LC_ALL", "C.UTF-8")
                .current_dir(std::env::temp_dir())
                .output()
                .unwrap();
            let mut expected = as_written(&name, kind);
            expected.push(b'\n');
            assert_eq!(
                out.stdout,
                expected,
                "{program}: {:?} quoted as {quoted:?}: {}",
                String::from_utf8_lossy(&name),
                String::from_utf8_lossy(&out.stderr)
            );
        }
    }
}

#[test]
fn several_paths_are_several_arguments() {
    let paths: Vec<PathBuf> = hostile()
        .into_iter()
        .map(|name| PathBuf::from(OsStr::from_bytes(&name)))
        .collect();
    let line = quote_paths(&paths, ShellKind::Posix);
    let script = format!("for a in {line}; do printf '%s\\0' \"$a\"; done");
    let out = Command::new("/bin/bash")
        .arg("-c")
        .arg(&script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .output();
    let Ok(out) = out else {
        return;
    };
    let mut expected = Vec::new();
    for name in hostile() {
        expected.extend_from_slice(&name);
        expected.push(0);
    }
    assert_eq!(out.stdout, expected);
}
