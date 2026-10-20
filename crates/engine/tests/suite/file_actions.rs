//! Opening a file as it was in a commit, and showing a file's folder with
//! a command of the user's: the copy an editor is handed is
//! what git has for that path in that commit, and the programs are started
//! with whole paths as single arguments. Only `cp`, `touch` and `true` are
//! ever started here: on Windows, where they are not on every `PATH`,
//! through sh scripts of the test's own that run them.

use std::path::{Path, PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.scratch.git()
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    /// A place beside the copy for what the started programs leave.
    fn beside(&self, name: &str) -> PathBuf {
        let dir = self.scratch.path().with_extension("out");
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.scratch.path().with_extension("out"));
    }
}

/// The program called `name` (`cp`, `touch`, `true`) as a command names
/// it: by its name on Unix, and on Windows the batch file of an sh script
/// that runs it with the `sh` of Git for Windows.
fn program(h: &Harness, name: &str) -> String {
    #[cfg(unix)]
    {
        let _ = h;
        name.to_owned()
    }
    #[cfg(not(unix))]
    {
        let dir = h.beside("programs");
        std::fs::create_dir_all(&dir).unwrap();
        let body = format!("#!/bin/sh\n{name} \"$@\"\n");
        bitbraid_testkit::script(&dir, name, &body)
            .unwrap()
            .display()
            .to_string()
    }
}

/// What a started program was to leave, once it has.
fn awaited(file: &Path) -> Vec<u8> {
    for _ in 0..250 {
        if let Ok(bytes) = std::fs::read(file)
            && !bytes.is_empty()
        {
            return bytes;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    panic!("{} never appeared", file.display());
}

fn remove_copy(out: &Value) {
    let copy = PathBuf::from(out["copy"].as_str().unwrap());
    let mut permissions = std::fs::metadata(&copy).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(false);
    let _ = std::fs::set_permissions(&copy, permissions);
    let _ = std::fs::remove_file(&copy);
    if let Some(dir) = copy.parent() {
        let _ = std::fs::remove_dir(dir);
    }
}

#[test]
fn a_file_is_opened_as_git_has_it_in_the_commit() {
    let mut h = Harness::new("linear");
    // A name with a blank, changed over three commits, then deleted.
    for (n, text) in ["first\n", "second\n", "third, with ünïcödé\n"]
        .iter()
        .enumerate()
    {
        h.scratch.write("dir/with space.txt", text).unwrap();
        h.git().run(["add", "-A"]).unwrap();
        h.git()
            .run(["commit", "-q", "-m", &format!("change {n}")])
            .unwrap();
    }
    h.git().run(["rm", "-q", "dir/with space.txt"]).unwrap();
    h.git().run(["commit", "-q", "-m", "delete it"]).unwrap();
    h.git().run(["tag", "before-delete", "HEAD~1"]).unwrap();

    for (rev, n) in [
        ("HEAD~3", 0),
        ("HEAD~2", 1),
        ("HEAD~1", 2),
        ("before-delete", 2),
    ] {
        let taken = h.beside(&format!("taken-{rev}-{n}"));
        let out = h.run(
            "file.open",
            json!({
                "path": "dir/with space.txt",
                "rev": rev,
                "command": format!("{} {{path}} {}", program(&h, "cp"), taken.display()),
            }),
        );
        let expected = h
            .git()
            .run(["show", &format!("{rev}:dir/with space.txt")])
            .unwrap();
        assert_eq!(awaited(&taken), expected.as_bytes(), "{rev}");
        assert_eq!(
            out["rev"],
            h.git().rev_parse(&format!("{rev}^{{commit}}")).unwrap()
        );
        // The copy has the file's own name, is outside the repository,
        // and cannot be written to by mistake.
        let copy = PathBuf::from(out["copy"].as_str().unwrap());
        assert_eq!(copy.file_name().unwrap(), "with space.txt");
        assert!(!copy.starts_with(h.scratch.path()));
        assert!(std::fs::metadata(&copy).unwrap().permissions().readonly());
        assert_eq!(std::fs::read(&copy).unwrap(), expected.as_bytes());
        // Asked again, the copy is made again.
        let again = h.run(
            "file.open",
            json!({ "path": "dir/with space.txt", "rev": rev, "command": program(&h, "true") }),
        );
        assert_eq!(again["copy"], out["copy"]);
        remove_copy(&out);
    }
    // The worktree was not touched: the file is still deleted there.
    assert!(!h.scratch.path().join("dir/with space.txt").exists());
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn what_is_no_file_in_the_commit_is_not_opened() {
    let mut h = Harness::new("linear");
    let marker = h.beside("started");
    let command = format!("{} {}", program(&h, "touch"), marker.display());
    for (path, rev, word) in [
        ("b.txt", "HEAD~4", "not a file"),
        ("nowhere.txt", "HEAD", "not a file"),
        ("a.txt", "no-such-rev", "revision"),
        ("a.txt", "HEAD:a.txt", "revision"),
        ("../outside", "HEAD", "path"),
        ("/etc/passwd", "HEAD", "path"),
        ("", "HEAD", "path"),
    ] {
        let err = h
            .try_run(
                "file.open",
                json!({ "path": path, "rev": rev, "command": command }),
            )
            .unwrap_err();
        assert!(err.contains(word), "{path} at {rev}: {err}");
    }
    for hostile in [
        json!("--all"),
        json!("--output=/tmp/bb-injected-open"),
        json!("a\u{0}b"),
        json!("one\ntwo"),
        json!("x".repeat(70_000)),
        json!(7),
        json!(["HEAD"]),
        json!({ "rev": "HEAD" }),
    ] {
        let result = h.try_run(
            "file.open",
            json!({ "path": "a.txt", "rev": hostile, "command": command }),
        );
        assert!(result.is_err(), "{hostile}: {result:?}");
    }
    std::thread::sleep(std::time::Duration::from_millis(200));
    assert!(!marker.exists(), "nothing was started for any of them");
    assert!(!Path::new("/tmp/bb-injected-open").exists());
}

#[test]
fn a_command_shows_the_folder_a_file_is_in() {
    let mut h = Harness::new("linear");
    h.scratch.write("deep/er dir/file.txt", "x\n").unwrap();
    let out = h.run(
        "file.reveal",
        json!({ "path": "deep/er dir/file.txt", "command": format!("{} {{path}}/shown.marker", program(&h, "touch")) }),
    );
    assert_eq!(out["revealed"], "deep/er dir/file.txt");
    let marker = h.scratch.path().join("deep/er dir/shown.marker");
    for _ in 0..250 {
        if marker.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(marker.exists(), "the folder arrived as one argument");
    for hostile in [json!(7), json!(["touch"]), json!("a\u{0}b")] {
        let result = h.try_run(
            "file.reveal",
            json!({ "path": "a.txt", "command": hostile }),
        );
        assert!(result.is_err(), "{hostile}: {result:?}");
    }
    assert!(
        h.try_run(
            "file.reveal",
            json!({ "path": "../outside", "command": "true" })
        )
        .is_err()
    );
}
