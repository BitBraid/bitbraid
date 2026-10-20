//! The terminal's commands: which shell starts where, and paths as a
//! shell reads them.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, CommandError, Registry, builtin_registry};
use bitbraid_testkit::{Scratch, scratch_root};
use serde_json::{Value, json};

struct Harness {
    copy: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let copy = Scratch::of("linear").unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .unwrap();
        Self {
            copy,
            registry,
            ctx,
        }
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }

    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn top(&self) -> PathBuf {
        bitbraid_core::paths::canonical(self.copy.path()).unwrap()
    }
}

fn text(value: &Value) -> &str {
    value.as_str().unwrap()
}

/// A `sh` that is there: the system's, or on Windows the one of Git for
/// Windows.
fn sh() -> String {
    #[cfg(windows)]
    {
        bitbraid_testkit::script::git_sh()
            .unwrap()
            .display()
            .to_string()
    }
    #[cfg(not(windows))]
    {
        "/bin/sh".to_owned()
    }
}

#[test]
fn a_terminal_starts_in_the_worktree_with_an_environment_of_its_own() {
    let mut h = Harness::new();
    let top = h.top();
    let out = h.run("terminal.shell", json!({}));
    assert_eq!(text(&out["cwd"]), top.display().to_string());
    assert_eq!(text(&out["worktree"]), top.display().to_string());
    // The user's shell, or the system's: on Windows PowerShell, or the
    // command prompt where there is none.
    let program = text(&out["program"]);
    match std::env::var("SHELL") {
        Ok(shell) if Path::new(&shell).is_file() => assert_eq!(program, shell),
        _ if cfg!(windows) => assert!(
            ["pwsh.exe", "powershell.exe", "cmd.exe"].contains(&program)
                || std::env::var("COMSPEC").is_ok_and(|comspec| comspec == program),
            "{program}"
        ),
        _ => assert_eq!(program, "/bin/sh"),
    }
    // A login shell on macOS, as its terminals start one.
    if cfg!(target_os = "macos") {
        assert_eq!(out["args"], json!(["-l"]));
    } else {
        assert_eq!(out["args"], json!([]));
    }
    let env = out["env"].as_object().unwrap();
    assert_eq!(env["TERM"], "xterm-256color");
    assert_eq!(env["COLORTERM"], "truecolor");
    assert_eq!(env["TERM_PROGRAM"], "bitbraid");
    assert!(env["TERM_PROGRAM_VERSION"].is_string());
    for name in [
        "GIT_DIR",
        "GIT_WORK_TREE",
        "GIT_INDEX_FILE",
        "TMUX",
        "COLUMNS",
    ] {
        assert!(!env.contains_key(name), "{name}");
    }
    // On Windows the name's case does not count (PowerShell has `Path`).
    if let Ok(path) = std::env::var("PATH") {
        let given = env
            .iter()
            .find(|(name, _)| bitbraid_terminal::shell::same_variable(name, "PATH"))
            .and_then(|(_, value)| value.as_str());
        assert_eq!(given, Some(path.as_str()), "{env:?}");
    }

    // The shell chosen in the preferences, with its arguments; on Windows
    // the sh of Git for Windows. One by its name alone is looked for
    // where `PATH` says: on Windows git, which is there wherever the tests
    // run.
    let sh = sh();
    let out = h.run(
        "terminal.shell",
        json!({ "program": sh, "args": ["-l", "two words"] }),
    );
    assert_eq!(out["program"], sh.as_str());
    assert_eq!(out["args"], json!(["-l", "two words"]));
    assert_eq!(out["kind"], "posix");
    let by_name = if cfg!(windows) { "git" } else { "sh" };
    let out = h.run("terminal.shell", json!({ "program": by_name }));
    assert_eq!(out["program"], by_name);

    // A folder inside it, by a path relative to its top or a full one.
    let deeper = top.join("sub dir").join("deeper");
    std::fs::create_dir_all(&deeper).unwrap();
    let out = h.run("terminal.shell", json!({ "path": "sub dir/deeper" }));
    assert_eq!(text(&out["cwd"]), deeper.display().to_string());
    assert_eq!(text(&out["worktree"]), top.display().to_string());
    let out = h.run(
        "terminal.shell",
        json!({ "path": top.join("sub dir").display().to_string() }),
    );
    assert_eq!(text(&out["cwd"]), top.join("sub dir").display().to_string());
    let out = h.run("terminal.shell", json!({ "path": "sub dir/../sub dir" }));
    assert_eq!(text(&out["cwd"]), top.join("sub dir").display().to_string());

    // Another worktree of the same repository, and a folder in it.
    let linked = scratch_root().join(format!("terminal-linked-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&linked);
    h.copy
        .git()
        .run([
            "worktree",
            "add",
            "-q",
            "-b",
            "terminal-side",
            linked.to_str().unwrap(),
        ])
        .unwrap();
    let linked = bitbraid_core::paths::canonical(linked).unwrap();
    std::fs::create_dir_all(linked.join("inner")).unwrap();
    let out = h.run(
        "terminal.shell",
        json!({ "path": linked.join("inner").display().to_string() }),
    );
    assert_eq!(
        text(&out["cwd"]),
        linked.join("inner").display().to_string()
    );
    assert_eq!(text(&out["worktree"]), linked.display().to_string());
    // Nothing of this changed the repository.
    assert_eq!(h.copy.git().run(["status", "--porcelain"]).unwrap(), "");
    let _ = std::fs::remove_dir_all(&linked);
}

#[cfg(unix)]
#[test]
fn folders_outside_the_worktrees_are_refused() {
    let mut h = Harness::new();
    let top = h.top();
    let outside = scratch_root().join(format!("terminal-outside-{}", std::process::id()));
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(top.join("a-file"), "x").unwrap();
    std::os::unix::fs::symlink(&outside, top.join("leads-out")).unwrap();
    for (path, why) in [
        (json!(outside.display().to_string()), "not in a worktree"),
        (json!("/"), "not in a worktree"),
        (json!("../.."), "not in a worktree"),
        (json!(".."), "not in a worktree"),
        (json!("leads-out"), "not in a worktree"),
        (json!("a-file"), "is not a folder"),
        (json!("no-such-folder"), "is not a folder that is there"),
        (
            json!("/definitely/not/there"),
            "is not a folder that is there",
        ),
        (json!("a\u{0}b"), "is not a path"),
        (json!("a\nb"), "is not a path"),
        (json!("x".repeat(10_000)), "is not a path"),
        (json!(7), "invalid parameter"),
        (json!(["/tmp"]), "invalid parameter"),
        (json!({ "path": "/" }), "invalid parameter"),
    ] {
        let err = h
            .try_run("terminal.shell", json!({ "path": path }))
            .unwrap_err();
        assert!(err.contains(why), "{path}: {err}");
    }
    let _ = std::fs::remove_dir_all(&outside);
}

#[test]
fn hostile_shells_and_arguments_are_refused() {
    let mut h = Harness::new();
    for params in [
        json!({ "program": "/definitely/not/there/sh" }),
        json!({ "program": "no-such-shell-anywhere" }),
        json!({ "program": "sh\u{7}" }),
        json!({ "program": "x".repeat(10_000) }),
        json!({ "program": 7 }),
        json!({ "program": ["sh"] }),
        json!({ "args": "-l" }),
        json!({ "args": [1, 2] }),
        json!({ "args": ["a\u{0}b"] }),
        json!({ "args": ["x".repeat(10_000)] }),
        json!({ "args": vec!["a"; 20_000] }),
    ] {
        assert!(
            h.try_run("terminal.shell", params.clone()).is_err(),
            "{params} should be refused"
        );
    }
    // A blank choice is no choice.
    let out = h.run("terminal.shell", json!({ "program": "  " }));
    assert!(!text(&out["program"]).trim().is_empty());

    // Without a repository there is nowhere to open.
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    assert!(matches!(
        registry.run("terminal.shell", &mut ctx, &json!({})),
        Err(CommandError::NoRepository)
    ));
    assert!(matches!(
        registry.run("terminal.shell", &mut ctx, &json!({ "path": "/tmp" })),
        Err(CommandError::NoRepository)
    ));
}

#[test]
fn paths_are_written_for_the_shell() {
    // No repository is needed.
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let mut run = |params: Value| registry.run("terminal.paste_paths", &mut ctx, &params);
    let out = run(json!({ "paths": ["/work/a b", "/work/it's", "/work/plain.rs"] })).unwrap();
    assert_eq!(out["text"], "'/work/a b' '/work/it'\\''s' /work/plain.rs");
    assert_eq!(out["kind"], "posix");
    let out = run(json!({ "paths": ["/a b"], "shell": "fish" })).unwrap();
    assert_eq!(
        (out["text"].as_str(), out["kind"].as_str()),
        (Some("'/a b'"), Some("fish"))
    );
    let out = run(json!({ "paths": ["/a b"], "shell": "/usr/local/bin/fish" })).unwrap();
    assert_eq!(out["kind"], "fish");
    let out = run(json!({ "paths": ["C:\\a'b"], "shell": "pwsh.exe" })).unwrap();
    assert_eq!(out["text"], "'C:\\a''b'");
    assert_eq!(out["kind"], "powershell");
    let out = run(json!({ "paths": ["C:\\a b"], "shell": "cmd" })).unwrap();
    assert_eq!(out["text"], "\"C:\\a b\"");
    let out = run(json!({ "paths": ["/x"], "shell": "/bin/bash" })).unwrap();
    assert_eq!(out["kind"], "posix");
    // nushell's single quotes cannot hold one; tcsh's take `!` for history.
    let out = run(json!({ "paths": ["/work/it's"], "shell": "/usr/bin/nu" })).unwrap();
    assert_eq!(
        (out["text"].as_str(), out["kind"].as_str()),
        (Some("\"/work/it's\""), Some("nushell"))
    );
    let out = run(json!({ "paths": ["/work/!x"], "shell": "tcsh" })).unwrap();
    assert_eq!(
        (out["text"].as_str(), out["kind"].as_str()),
        (Some("'/work/'\\!'x'"), Some("csh"))
    );
    let out = run(json!({ "paths": ["/a b"], "shell": "nushell" })).unwrap();
    assert_eq!(out["text"], "\"/a b\"");
    let out = run(json!({ "paths": [] })).unwrap();
    assert_eq!(out["text"], "");
    let out = run(json!({ "paths": ["/a\nb"] })).unwrap();
    assert_eq!(out["text"], "'/a'$'\\n''b'");

    for hostile in [
        json!({}),
        json!({ "paths": "/a" }),
        json!({ "paths": [7] }),
        json!({ "paths": [""] }),
        json!({ "paths": ["a\u{0}b"] }),
        json!({ "paths": ["x".repeat(10_000)] }),
        json!({ "paths": vec!["/a"; 20_000] }),
        json!({ "paths": ["/a"], "shell": 7 }),
        json!({ "paths": ["/a"], "shell": "fi\u{7}sh" }),
        json!({ "paths": ["/a"], "shell": "x".repeat(10_000) }),
    ] {
        assert!(run(hostile.clone()).is_err(), "{hostile} should be refused");
    }
}

#[cfg(unix)]
#[test]
fn the_text_pastes_the_same_paths_into_sh() {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let paths = [
        "/tmp/with space",
        "/tmp/$(touch pwned)",
        "/tmp/`id`",
        "/tmp/a\nb",
        "/tmp/quote's \"double\"",
        "/tmp/-dash",
        "/tmp/glob*?[x]",
    ];
    let out = registry
        .run("terminal.paste_paths", &mut ctx, &json!({ "paths": paths }))
        .unwrap();
    let script = format!(
        "for a in {}; do printf '%s\\0' \"$a\"; done",
        out["text"].as_str().unwrap()
    );
    // A line break is written `$'\n'`, which bash, zsh, ksh and
    // POSIX.1-2024 read; the sh of Debian and Ubuntu (dash) does not yet.
    let printed = std::process::Command::new("bash")
        .arg("-c")
        .arg(&script)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    let expected: Vec<u8> = paths
        .iter()
        .flat_map(|path| path.bytes().chain(std::iter::once(0)))
        .collect();
    assert_eq!(printed.stdout, expected, "{script}");
}
