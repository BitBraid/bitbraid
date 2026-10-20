//! Sparse checkout against `git sparse-checkout` on twin copies:
//! each step runs as a command on one copy and as git on the other, and
//! the two have to have the same files on disk, the same index (which
//! files are left out), the same file of patterns and the same settings;
//! what the status says is what git lists and counts.
//!
//! The commands run git as whoever runs the tests has it set up, and so
//! does the twin.

use std::{path::Path, process::Command};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::Scratch;
use serde_json::{Value, json};

fn plain_git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .current_dir(dir)
        .args(args)
        .env("LC_ALL", "C")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "git {args:?} in {}: {}",
        dir.display(),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// What the commands do before they change the checkout, as `git status`
/// would: the index's notes of the files' times brought up to date, so
/// that a file that was only copied is not taken for one with changes.
fn refresh(dir: &Path) {
    let _ = Command::new("git")
        .current_dir(dir)
        .args(["update-index", "-q", "--refresh"])
        .env("GIT_CEILING_DIRECTORIES", dir.parent().unwrap())
        .output();
}

struct Engine {
    registry: Registry,
    ctx: CommandContext,
}

impl Engine {
    fn at(path: &Path) -> Self {
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": path }))
            .unwrap();
        Self { registry, ctx }
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
}

/// Every file in the worktree, its git directory left out.
fn on_disk(dir: &Path) -> Vec<String> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            if path.parent() == Some(root) && entry.file_name() == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else {
                // With `/` between folders, as git writes paths.
                let relative = path.strip_prefix(root).unwrap().to_string_lossy();
                out.push(relative.replace(std::path::MAIN_SEPARATOR, "/"));
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

/// Everything a sparse checkout is: the files on disk, which tracked files
/// are left out, the file of patterns, and the settings.
fn state(dir: &Path) -> Vec<String> {
    let mut out = on_disk(dir);
    out.extend(
        plain_git(dir, &["ls-files", "-t"])
            .lines()
            .map(str::to_owned),
    );
    out.push(format!(
        "patterns {:?}",
        std::fs::read_to_string(dir.join(".git/info/sparse-checkout")).ok()
    ));
    out.extend(
        plain_git(dir, &["config", "--list", "--show-scope"])
            .lines()
            .filter(|line| {
                let line = line.to_lowercase();
                line.contains("sparse") || line.contains("worktreeconfig")
            })
            .map(str::to_owned),
    );
    out
}

/// The status as git tells it: on, cone, what it lists, how many tracked
/// and how many left out.
fn status_by_git(dir: &Path) -> (bool, bool, Vec<String>, u64, u64) {
    let flag = |key: &str| {
        Command::new("git")
            .current_dir(dir)
            .args(["config", "--get", "--bool", key])
            .output()
            .is_ok_and(|out| String::from_utf8_lossy(&out.stdout).trim() == "true")
    };
    let enabled = flag("core.sparseCheckout");
    let listed = if enabled {
        plain_git(dir, &["sparse-checkout", "list"])
            .lines()
            .map(str::to_owned)
            .collect()
    } else {
        Vec::new()
    };
    // Counted only where the checkout is sparse.
    let files = if enabled {
        plain_git(dir, &["ls-files", "-t"])
    } else {
        String::new()
    };
    let outside = files.lines().filter(|line| line.starts_with("S ")).count() as u64;
    (
        enabled,
        enabled && flag("core.sparseCheckoutCone"),
        listed,
        files.lines().count() as u64,
        outside,
    )
}

fn status_by_us(status: &Value) -> (bool, bool, Vec<String>, u64, u64) {
    (
        status["enabled"].as_bool().unwrap(),
        status["cone"].as_bool().unwrap(),
        status["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|rule| rule.as_str().unwrap().to_owned())
            .collect(),
        status["tracked"].as_u64().unwrap(),
        status["outside"].as_u64().unwrap(),
    )
}

#[test]
fn each_step_leaves_what_git_sparse_checkout_leaves() {
    let ours = Scratch::of("layout").unwrap();
    let twin = Scratch::of("layout").unwrap();
    let mut engine = Engine::at(ours.path());
    let off = engine.run("sparse.status", json!({}));
    assert_eq!(status_by_us(&off), status_by_git(twin.path()));
    assert_eq!(off["enabled"], false);
    let steps: Vec<(&str, Value, Vec<&str>)> = vec![
        (
            "sparse.set",
            json!({ "directories": ["src/core"] }),
            vec!["sparse-checkout", "set", "--cone", "src/core"],
        ),
        (
            "sparse.add",
            json!({ "directories": ["docs/", "with space"] }),
            vec!["sparse-checkout", "add", "docs", "with space"],
        ),
        (
            "sparse.set",
            json!({ "directories": ["src"] }),
            vec!["sparse-checkout", "set", "--cone", "src"],
        ),
        // Only the files at the top.
        (
            "sparse.set",
            json!({ "directories": [] }),
            vec!["sparse-checkout", "set", "--cone"],
        ),
        (
            "sparse.set",
            json!({ "patterns": ["/*", "!/src/", "/src/ui/"] }),
            vec![
                "sparse-checkout",
                "set",
                "--no-cone",
                "/*",
                "!/src/",
                "/src/ui/",
            ],
        ),
        (
            "sparse.add",
            json!({ "patterns": ["*.rs"] }),
            vec!["sparse-checkout", "add", "*.rs"],
        ),
        // A directory, in a checkout by patterns, is everything below it.
        (
            "sparse.add",
            json!({ "directories": ["docs"] }),
            vec!["sparse-checkout", "add", "/docs/"],
        ),
        (
            "sparse.disable",
            json!({}),
            vec!["sparse-checkout", "disable"],
        ),
        (
            "sparse.set",
            json!({ "directories": ["assets/copy"] }),
            vec!["sparse-checkout", "set", "--cone", "assets/copy"],
        ),
    ];
    for (id, params, by_git) in steps {
        let out = engine.run(id, params.clone());
        refresh(twin.path());
        plain_git(twin.path(), &by_git);
        let what = format!("{id} {params}");
        assert_eq!(state(ours.path()), state(twin.path()), "{what}");
        assert_eq!(
            status_by_us(&out["status"]),
            status_by_git(twin.path()),
            "{what}"
        );
        let status = engine.run("sparse.status", json!({}));
        assert_eq!(status, out["status"], "{what}");
    }
    // The last cone has the files at the top, those of the folders on the
    // way, and the one folder; the files of a fresh copy were not taken
    // for ones with changes.
    let status = engine.run("sparse.status", json!({}));
    assert_eq!(status["rules"], json!(["assets/copy"]));
    assert_eq!(
        on_disk(ours.path()),
        [
            "Cargo.toml",
            "README.md",
            "assets/copy/logo.svg",
            "assets/logo.svg"
        ]
    );
}

#[test]
fn a_changed_file_stays_and_git_says_so() {
    let ours = Scratch::of("layout").unwrap();
    let twin = Scratch::of("layout").unwrap();
    for copy in [&ours, &twin] {
        copy.write("src/ui/view.rs", "pub fn view() { changed(); }")
            .unwrap();
    }
    let mut engine = Engine::at(ours.path());
    let out = engine.run("sparse.set", json!({ "directories": ["docs"] }));
    refresh(twin.path());
    plain_git(twin.path(), &["sparse-checkout", "set", "--cone", "docs"]);
    assert_eq!(state(ours.path()), state(twin.path()));
    assert!(ours.path().join("src/ui/view.rs").exists());
    assert!(
        out["output"].as_str().unwrap().contains("src/ui/view.rs"),
        "{out}"
    );
}

#[test]
fn hostile_rules_are_refused_and_change_nothing() {
    let copy = Scratch::of("layout").unwrap();
    let mut engine = Engine::at(copy.path());
    let before = state(copy.path());
    let long = "d".repeat(5000);
    let many: Vec<String> = (0..1001).map(|n| format!("d{n}")).collect();
    for params in [
        json!({}),
        json!({ "directories": ["src"], "patterns": ["/*"] }),
        json!({ "directories": "src" }),
        json!({ "directories": [5] }),
        json!({ "directories": [null] }),
        json!({ "directories": [""] }),
        json!({ "directories": ["/"] }),
        json!({ "directories": ["/abs"] }),
        json!({ "directories": ["../up"] }),
        json!({ "directories": ["src/../.."] }),
        json!({ "directories": ["./src"] }),
        json!({ "directories": ["src/*"] }),
        json!({ "directories": ["new\nline"] }),
        json!({ "directories": ["nul\u{0}"] }),
        json!({ "directories": [long] }),
        json!({ "directories": many }),
        json!({ "patterns": [""] }),
        json!({ "patterns": ["a\nb"] }),
        json!({ "patterns": [long] }),
        json!({ "patterns": many }),
    ] {
        for id in ["sparse.set", "sparse.add"] {
            assert!(
                engine.try_run(id, params.clone()).is_err(),
                "{id} took {}",
                params.to_string().chars().take(200).collect::<String>()
            );
        }
    }
    // Adding to a checkout that is not sparse.
    let err = engine
        .try_run("sparse.add", json!({ "directories": ["src"] }))
        .unwrap_err();
    assert!(err.contains("not sparse"), "{err}");
    assert_eq!(state(copy.path()), before);
    // Patterns added to a checkout by directory.
    engine.run("sparse.set", json!({ "directories": ["src"] }));
    let cone = state(copy.path());
    let err = engine
        .try_run("sparse.add", json!({ "patterns": ["*.md"] }))
        .unwrap_err();
    assert!(err.contains("by directory"), "{err}");
    assert!(
        engine
            .try_run("sparse.add", json!({ "directories": [] }))
            .is_err()
    );
    assert_eq!(state(copy.path()), cone);
    // Names git could take for options are names.
    engine.run("sparse.set", json!({ "directories": ["--no-cone", "-x"] }));
    assert_eq!(
        engine.run("sparse.status", json!({}))["rules"],
        json!(["--no-cone", "-x"])
    );
}

#[test]
fn a_bare_repository_has_no_sparse_checkout() {
    let bare = Scratch::of("bare").unwrap();
    let mut engine = Engine::at(bare.path());
    let err = engine.try_run("sparse.status", json!({})).unwrap_err();
    assert!(err.contains("bare"), "{err}");
}
