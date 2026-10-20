//! The diff and merge tools found on this machine: what
//! `tool.known` answers is what is on `PATH` (on macOS, or in a tool's
//! application, and on Windows, or where its installer puts it, its
//! command lines naming the program there), and a command of the list
//! works as a tool command the way git runs one. No tool of whoever runs
//! the tests is started: the commands are tried with a script that stands
//! in for the tool's program. (On Windows git's shell is the `sh` of Git
//! for Windows, which runs the script by its first line.)

use std::path::Path;

use bitbraid_engine::{CommandContext, builtin_registry};
use bitbraid_git::known_tools::KNOWN;
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

/// Whether `path` ends in a place a Windows installer puts a tool's
/// program (`KnownTool::windows`), a part ending in `*` standing for any
/// name that begins so; letters of any case.
fn installed_there(path: &Path, place: &str) -> bool {
    let theirs: Vec<String> = path
        .components()
        .rev()
        .map(|part| part.as_os_str().to_string_lossy().to_lowercase())
        .collect();
    let ours: Vec<String> = place.split('/').rev().map(str::to_lowercase).collect();
    ours.len() <= theirs.len()
        && ours
            .iter()
            .zip(&theirs)
            .all(|(ours, theirs)| match ours.strip_suffix('*') {
                Some(begins) => theirs.starts_with(begins),
                None => ours == theirs,
            })
}

/// Whether `program` is in a folder of PATH, with `.exe` at its end on
/// Windows or as it is named.
fn on_path(program: &str) -> bool {
    let exe = format!("{program}.exe");
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join(program).is_file() || (cfg!(windows) && dir.join(&exe).is_file()))
    })
}

#[test]
fn what_is_found_is_on_the_path_and_known() {
    let registry = builtin_registry();
    // Without a repository: it is a look through PATH.
    let mut ctx = CommandContext::default();
    let answer = registry.run("tool.known", &mut ctx, &json!({})).unwrap();
    let found = answer["found"].as_array().unwrap();
    assert!(answer.get("known").is_none());
    for tool in found {
        let known = KNOWN
            .iter()
            .find(|known| tool["name"] == known.name)
            .unwrap_or_else(|| panic!("{tool} is not a tool that is known"));
        assert_eq!(tool["program"], known.program);
        let path = Path::new(tool["path"].as_str().unwrap());
        assert!(path.is_absolute(), "{tool}");
        // Found in its application (macOS) or where its installer put it
        // (Windows), and not on PATH: the program is named by its place in
        // each command line, and is where the tool keeps it. (On PATH it is
        // named by its name, even where PATH leads to it through a link,
        // as winget's portable packages do.)
        let placed = known
            .apps
            .iter()
            .find(|app| path.ends_with(app))
            .filter(|_| cfg!(target_os = "macos"))
            .or_else(|| {
                known
                    .windows
                    .iter()
                    .find(|place| installed_there(path, place))
                    .filter(|_| cfg!(windows))
            })
            .filter(|_| !on_path(known.program));
        if let Some(app) = placed {
            let place = format!("'{}'", path.display());
            for (line, own) in [(&tool["diff"], known.diff), (&tool["merge"], known.merge)] {
                let wanted = own.map(|own| own.replacen(known.program, &place, 1));
                assert_eq!(line, &json!(wanted), "{tool} ({app})");
            }
            assert!(runs(path), "{tool} cannot be run");
            continue;
        }
        assert_eq!(tool["diff"], json!(known.diff));
        assert_eq!(tool["merge"], json!(known.merge));
        let name = path.file_name().unwrap().to_str().unwrap();
        // On Windows a program is found with `.exe` at its end, or as it
        // is named.
        if cfg!(windows) {
            assert!(
                name == known.program || name == format!("{}.exe", known.program),
                "{tool}"
            );
        } else {
            assert_eq!(name, known.program);
        }
        assert!(runs(path), "{tool} cannot be run");
    }
    let all = registry
        .run("tool.known", &mut ctx, &json!({ "all": true }))
        .unwrap();
    assert_eq!(all["found"], answer["found"]);
    assert_eq!(all["known"].as_array().unwrap().len(), KNOWN.len());
    for hostile in [json!("yes"), json!(3), json!(["x"]), json!({ "all": true })] {
        assert!(
            registry
                .run("tool.known", &mut ctx, &json!({ "all": hostile }))
                .is_err()
        );
    }
    for shape in [Value::Null, json!([]), json!("text")] {
        let _ = registry.run("tool.known", &mut ctx, &shape);
    }
}

/// Wait for a file a started program was to leave.
fn awaited(file: &Path) -> String {
    for _ in 0..250 {
        if let Ok(text) = std::fs::read_to_string(file)
            && text.ends_with("end\n")
        {
            return text;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{} never appeared", file.display());
}

/// Whether a program that was found can be run: marked so for someone, or
/// on Windows, where files carry no such mark, a file.
fn runs(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::metadata(path).unwrap().permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        std::fs::metadata(path).unwrap().is_file()
    }
}

/// Write the stand-in `body` at `path`, which leaves `record` when it
/// runs, and give the path to put in a tool command: on Windows with `/`
/// between folders, which git's shell takes as it is.
fn stand_in(path: &Path, record: &Path, body: &str) -> String {
    std::fs::write(path, body).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // A script that was just written can be "busy" for a moment, while
        // a child another test's thread forked still holds it open: run
        // it once by hand until it starts, and take away what that left.
        for _ in 0..300 {
            match std::process::Command::new(path).output() {
                Err(err) if err.kind() == std::io::ErrorKind::ExecutableFileBusy => {
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
                other => {
                    other.unwrap();
                    break;
                }
            }
        }
        awaited(record);
        std::fs::remove_file(record).unwrap();
        path.to_str().unwrap().to_owned()
    }
    #[cfg(not(unix))]
    {
        let _ = record;
        path.to_str().unwrap().replace('\\', "/")
    }
}

/// Every diff command of the list, run as git runs a tool command, hands
/// its program two files that are there: the two versions.
#[test]
fn a_known_diff_command_hands_the_two_versions_to_its_program() {
    let copy = Scratch::of("linear").unwrap();
    copy.git()
        .run(["config", "user.name", "Ada Fixture"])
        .unwrap();
    copy.write("a.txt", "2\nchanged in the worktree\n").unwrap();
    let side = copy.path().with_extension("tools");
    std::fs::create_dir_all(&side).unwrap();
    // What the index has, as the stand-in will write it down.
    let before = copy
        .git()
        .run(["show", ":a.txt"])
        .unwrap()
        .replace('\n', "|");
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    for (n, tool) in KNOWN.iter().enumerate() {
        let Some(diff) = tool.diff else { continue };
        // A stand-in for the program: it writes down what it was handed
        // that is a file, with what the file holds.
        let record = side.join(format!("record-{n}.txt"));
        let program = stand_in(
            &side.join(format!("tool-{n}.sh")),
            &record,
            &format!(
                "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ -f \"$arg\" ]; then printf 'file:%s\\n' \"$(cat \"$arg\" | tr '\\n' '|')\"; else printf 'word:%s\\n' \"$arg\"; fi\ndone > '{record}'\necho end >> '{record}'\n",
                record = record.display()
            ),
        );
        // The command as it is listed, with the program's name replaced.
        let command = diff.replacen(tool.program, &program, 1);
        registry
            .run(
                "tool.diff",
                &mut ctx,
                &json!({ "path": "a.txt", "command": command }),
            )
            .unwrap_or_else(|err| panic!("{}: {err}", tool.name));
        let said = awaited(&record);
        let files: Vec<&str> = said
            .lines()
            .filter_map(|line| line.strip_prefix("file:"))
            .collect();
        assert_eq!(
            files,
            [before.as_str(), "2|changed in the worktree|"],
            "{} was handed {said:?}",
            tool.name
        );
    }
    let _ = std::fs::remove_dir_all(&side);
}

/// Every merge command of the list, run as git runs one on a file in
/// conflict, hands its program our side, their side and the file to
/// write; a tool that takes it also what both started from.
#[test]
fn a_known_merge_command_hands_the_sides_and_the_result_to_its_program() {
    let copy = Scratch::of("linear").unwrap();
    let git = copy.git();
    for (key, value) in [
        ("user.name", "Ada Fixture"),
        ("user.email", "ada@fixture.invalid"),
        ("commit.gpgsign", "false"),
        ("merge.conflictStyle", "merge"),
        ("rerere.enabled", "false"),
    ] {
        git.run(["config", key, value]).unwrap();
    }
    git.run(["switch", "-q", "-c", "topic"]).unwrap();
    copy.write("a.txt", "theirs\n").unwrap();
    git.run(["commit", "-q", "-am", "topic"]).unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    copy.write("a.txt", "ours\n").unwrap();
    git.run(["commit", "-q", "-am", "main"]).unwrap();
    assert!(git.run(["merge", "-q", "topic"]).is_err());
    let base = git.run(["show", ":1:a.txt"]).unwrap().replace('\n', "|");
    let side = copy.path().with_extension("tools");
    std::fs::create_dir_all(&side).unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    for (n, tool) in KNOWN.iter().enumerate() {
        let Some(merge) = tool.merge else { continue };
        // The stand-in writes down the files it was handed and ends badly,
        // so that the file stays in conflict for the next tool.
        let record = side.join(format!("merged-{n}.txt"));
        let program = stand_in(
            &side.join(format!("merge-{n}.sh")),
            &record,
            &format!(
                "#!/bin/sh\nfor arg in \"$@\"; do\n  if [ -f \"$arg\" ]; then printf 'file:%s\\n' \"$(cat \"$arg\" | tr '\\n' '|')\"; fi\ndone > '{record}'\necho end >> '{record}'\nexit 1\n",
                record = record.display()
            ),
        );
        let command = merge.replacen(tool.program, &program, 1);
        let answer = registry
            .run(
                "tool.merge",
                &mut ctx,
                &json!({ "path": "a.txt", "command": command }),
            )
            .unwrap_or_else(|err| panic!("{}: {err}", tool.name));
        assert_eq!(answer["resolved"], false, "{}", tool.name);
        let said = awaited(&record);
        let files: Vec<&str> = said
            .lines()
            .filter_map(|line| line.strip_prefix("file:"))
            .collect();
        assert!(files.contains(&"ours|"), "{}: {said:?}", tool.name);
        assert!(files.contains(&"theirs|"), "{}: {said:?}", tool.name);
        assert!(
            files.iter().any(|file| file.contains("<<<<<<<")),
            "{} was not handed the file to write: {said:?}",
            tool.name
        );
        // Three files, or four with what both started from.
        match files.len() {
            3 => {}
            4 => assert!(files.contains(&base.as_str()), "{}: {said:?}", tool.name),
            other => panic!("{} was handed {other} files: {said:?}", tool.name),
        }
        assert_eq!(merge.contains("$BASE"), files.len() == 4, "{}", tool.name);
    }
    let _ = std::fs::remove_dir_all(&side);
}
