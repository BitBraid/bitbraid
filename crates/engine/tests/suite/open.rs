//! Opening the repository's folder in other programs, with programs of the
//! test's own in place of the file manager, the editor and the terminal:
//! each writes down where it was started and with what. (On Windows such
//! a program is an sh script started through a batch file, by the `sh` of
//! Git for Windows.)

use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Scratch, scratch_root};
use serde_json::{Value, json};

struct Harness {
    copy: Scratch,
    dir: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(name: &str) -> Self {
        let copy = Scratch::of("linear").unwrap();
        let dir = scratch_root().join(format!("open-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .unwrap();
        Self {
            copy,
            dir: bitbraid_core::paths::canonical(dir).unwrap(),
            registry,
            ctx,
        }
    }

    /// A program that writes the directory it was started in and its
    /// arguments, one a line, to a file of its own; where that file is.
    fn recorder(&self, name: &str) -> (PathBuf, PathBuf) {
        let (program, record) = (self.dir.join(name), self.dir.join(format!("{name}.record")));
        // Where it was started, as the system writes it: the sh of Git for
        // Windows says that with `-W`, with `/` between folders.
        let here = if cfg!(windows) {
            "pwd -W | tr / '\\\\'"
        } else {
            "pwd -P"
        };
        let body = format!(
            "#!/bin/sh\n[ \"$1\" = --settle ] && exit 0\n{{ {here}; for a in \"$@\"; do printf '%s\\n' \"$a\"; done; }} > '{}.tmp'\nmv '{}.tmp' '{}'\n",
            record.display(),
            record.display(),
            record.display()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::write(&program, body).unwrap();
            std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
            // A script that was just written can be "busy" for a moment.
            for _ in 0..200 {
                match std::process::Command::new(&program)
                    .arg("--settle")
                    .output()
                {
                    Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                        std::thread::sleep(Duration::from_millis(10))
                    }
                    _ => break,
                }
            }
            (program, record)
        }
        // Elsewhere the batch file that runs it is the program.
        #[cfg(not(unix))]
        {
            let _ = program;
            (
                bitbraid_testkit::script(&self.dir, name, &body).unwrap(),
                record,
            )
        }
    }

    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }

    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    fn top(&self) -> String {
        bitbraid_core::paths::canonical(self.copy.path())
            .unwrap()
            .display()
            .to_string()
    }
}

/// What a recorder wrote, once it has: the program is left running, so
/// its record arrives a moment after the command returns.
fn recorded(record: &Path) -> Vec<String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(10) {
        if let Ok(text) = std::fs::read_to_string(record) {
            let _ = std::fs::remove_file(record);
            return text.lines().map(str::to_owned).collect();
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    panic!("nothing was started: {} never appeared", record.display());
}

#[test]
fn the_folder_is_opened_where_it_is_with_the_program_that_is_named() {
    let mut h = Harness::new("programs");
    let top = h.top();

    // A terminal: started in the folder, told the folder where it asks.
    let (terminal, record) = h.recorder("terminal");
    let command = format!(
        "{} --working-directory={{path}} --title here",
        terminal.display()
    );
    let out = h.run("repo.open_terminal", json!({ "command": command }));
    assert_eq!(out["opened"], top);
    assert_eq!(out["with"], terminal.display().to_string());
    assert_eq!(
        recorded(&record),
        [
            top.clone(),
            format!("--working-directory={top}"),
            "--title".into(),
            "here".into()
        ]
    );
    // One that is told nothing starts there all the same.
    h.run("repo.open_terminal", json!({ "command": terminal }));
    assert_eq!(recorded(&record), std::slice::from_ref(&top));

    // A folder inside the worktree, and a folder elsewhere by its full path.
    std::fs::create_dir_all(h.copy.path().join("sub dir/deeper")).unwrap();
    h.run(
        "repo.open_terminal",
        json!({ "command": terminal, "path": "sub dir/deeper" }),
    );
    let deeper = Path::new(&top).join("sub dir").join("deeper");
    assert_eq!(recorded(&record), [deeper.display().to_string()]);
    let elsewhere = h.dir.display().to_string();
    h.run(
        "repo.open_terminal",
        json!({ "command": format!("{} {{path}}", terminal.display()), "path": elsewhere }),
    );
    assert_eq!(recorded(&record), [elsewhere.clone(), elsewhere.clone()]);

    // The file manager: a command gets the folder, placed or appended.
    let (manager, record) = h.recorder("manager");
    h.run("repo.open_folder", json!({ "command": manager }));
    assert_eq!(recorded(&record), [top.clone(), top.clone()]);
    h.run(
        "repo.open_folder",
        json!({ "command": format!("{} --select {{path}} --new", manager.display()) }),
    );
    assert_eq!(
        recorded(&record),
        [top.clone(), "--select".into(), top.clone(), "--new".into()]
    );

    // The editor: its command is written for a file; a folder gets its
    // program and the folder, and none of the rest.
    let (editor, record) = h.recorder("editor");
    h.run(
        "repo.open_editor",
        json!({ "command": format!("{} --goto {{path}}:{{line}}", editor.display()) }),
    );
    assert_eq!(recorded(&record), [top.clone(), top.clone()]);

    // Nothing of this touched the repository (git does not list the empty
    // folders the test made).
    assert_eq!(h.copy.git().run(["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn the_terminals_found_are_listed_with_their_commands() {
    let mut h = Harness::new("terminals");
    let out = h.run("repo.terminals", json!({}));
    let found = out["terminals"].as_array().unwrap();
    // Whatever this machine has, the first is the default and each has a
    // command that starts with its program.
    match found.first() {
        Some(first) => assert_eq!(&out["default"], first),
        None => assert!(out["default"].is_null()),
    }
    // On Windows the terminals have names of their own, and the command
    // prompt, which is always there, comes last.
    if cfg!(windows) {
        assert_eq!(
            found.last().unwrap(),
            &json!({ "name": "Command Prompt", "command": "cmd /C start cmd" })
        );
        return;
    }
    // On macOS they are applications, opened by `open`; the system's own
    // comes first.
    if cfg!(target_os = "macos") {
        assert_eq!(
            found.first().unwrap(),
            &json!({ "name": "Terminal", "command": "open -a Terminal {path}" })
        );
        for terminal in found {
            let (name, command) = (
                terminal["name"].as_str().unwrap(),
                terminal["command"].as_str().unwrap(),
            );
            assert!(
                command.starts_with(&format!("open -a {name} "))
                    || command.starts_with(&format!("open -na {name} --args ")),
                "{terminal}"
            );
            assert!(command.contains("{path}"), "{terminal}");
        }
        return;
    }
    for terminal in found {
        let (name, command) = (
            terminal["name"].as_str().unwrap(),
            terminal["command"].as_str().unwrap(),
        );
        let program = command.split_whitespace().next().unwrap();
        assert!(
            Path::new(program).file_name().unwrap().to_str().unwrap() == name,
            "{terminal}"
        );
    }
}

#[test]
fn hostile_parameters_start_nothing() {
    let mut h = Harness::new("hostile");
    let (program, record) = h.recorder("program");
    let top = h.top();
    let file = h.copy.path().join("a-file-for-the-test");
    std::fs::write(&file, "x").unwrap();
    for id in ["repo.open_terminal", "repo.open_folder", "repo.open_editor"] {
        for hostile in [
            json!({ "command": program, "path": "/definitely/not/there" }),
            json!({ "command": program, "path": "no-such-folder" }),
            json!({ "command": program, "path": "../.." }),
            json!({ "command": program, "path": "a-file-for-the-test" }),
            json!({ "command": program, "path": file }),
            json!({ "command": program, "path": "a\u{0}b" }),
            json!({ "command": program, "path": "x".repeat(10_000) }),
            json!({ "command": program, "path": 7 }),
            json!({ "command": program, "path": ["/tmp"] }),
            json!({ "command": "/definitely/not/there/program" }),
            json!({ "command": "prog\u{7}ram" }),
            json!({ "command": "x".repeat(10_000) }),
            json!({ "command": 7 }),
            json!({ "command": { "program": "sh" } }),
        ] {
            assert!(
                h.try_run(id, hostile.clone()).is_err(),
                "{id} {hostile} should be refused"
            );
        }
    }
    // Without an editor there is nothing to open a folder with.
    let err = h.try_run("repo.open_editor", json!({})).unwrap_err();
    assert!(err.contains("no editor is named"), "{err}");
    assert!(
        h.try_run("repo.open_editor", json!({ "command": "   " }))
            .is_err()
    );
    // A path that looks like options or like a command is a path.
    let odd = h.copy.path().join("--help; $(touch pwned) `id`");
    std::fs::create_dir_all(&odd).unwrap();
    h.run(
        "repo.open_terminal",
        json!({ "command": format!("{} {{path}}", program.display()), "path": odd }),
    );
    let seen = recorded(&record);
    assert_eq!(seen.len(), 2, "{seen:?}");
    let odd_there = Path::new(&top).join("--help; $(touch pwned) `id`");
    assert_eq!(seen[1], odd_there.display().to_string());
    assert!(!h.copy.path().join("pwned").exists());
    assert!(!odd.join("pwned").exists());
    // Nothing else was started along the way.
    std::thread::sleep(Duration::from_millis(200));
    assert!(!record.exists());

    // Without a repository, a full path is needed and is enough.
    let mut ctx = CommandContext::default();
    let registry = builtin_registry();
    assert!(
        registry
            .run(
                "repo.open_terminal",
                &mut ctx,
                &json!({ "command": program })
            )
            .is_err()
    );
    assert!(
        registry
            .run(
                "repo.open_terminal",
                &mut ctx,
                &json!({ "command": program, "path": "relative" })
            )
            .is_err()
    );
    registry
        .run(
            "repo.open_terminal",
            &mut ctx,
            &json!({ "command": program, "path": h.dir }),
        )
        .unwrap();
    assert_eq!(recorded(&record), [h.dir.display().to_string()]);
}
