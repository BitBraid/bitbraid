//! Panic hunt for the workspace commands: every one with hostile
//! parameters, and the readers with hostile files (`workspaces.toml`, a
//! `.code-workspace` file, a workspace file to import). Each has to end
//! in an answer or an error, never a panic, a wait without end or a read
//! without bound; and nothing outside the test's own folder is touched.

use std::{
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, fixtures_out_dir};
use serde_json::{Value, json};

use crate::workspaces::Dir;

struct Hunt {
    dir: Dir,
    registry: Registry,
    repository: String,
    bare: String,
}

impl Hunt {
    fn new() -> Self {
        let dir = Dir::new("workspace-hunt");
        let registry = builtin_registry();
        let repository = dir.text("repo");
        let bare = dir.text("remote.git");
        let git = Git::new(dir.join(""), fixtures_out_dir().join(".home"));
        git.ensure_home().unwrap();
        git.run(["init", "-q", "--bare", bare.as_str()]).unwrap();
        git.run(["init", "-q", "-b", "main", repository.as_str()])
            .unwrap();
        let inner = Git::new(&repository, fixtures_out_dir().join(".home"));
        std::fs::write(Path::new(&repository).join("f.txt"), "f").unwrap();
        inner.run(["add", "f.txt"]).unwrap();
        inner.run(["commit", "-q", "-m", "one"]).unwrap();
        inner
            .run(["remote", "add", "origin", bare.as_str()])
            .unwrap();
        inner.run(["push", "-q", "origin", "main"]).unwrap();
        Self {
            dir,
            registry,
            repository,
            bare,
        }
    }

    fn context(&self) -> CommandContext {
        CommandContext {
            config_dir: Some(self.dir.join("config")),
            ..CommandContext::default()
        }
    }

    /// Run a command; a panic fails the test, anything else is fine.
    fn try_run(&self, id: &str, params: &Value) -> Result<Value, String> {
        let mut ctx = self.context();
        let shown: String = params.to_string().chars().take(300).collect();
        catch_unwind(AssertUnwindSafe(|| self.registry.run(id, &mut ctx, params)))
            .unwrap_or_else(|_| panic!("{id} {shown} panicked"))
            .map_err(|err| err.to_string())
    }

    /// A workspace to work on, made again before each run so that what a
    /// run did is not what the next starts from.
    fn reset(&self) {
        let _ = std::fs::remove_dir_all(self.dir.join("config"));
        let _ = std::fs::remove_dir_all(self.dir.join("out"));
        std::fs::create_dir_all(self.dir.join("out")).unwrap();
        self.try_run(
            "workspace.create",
            &json!({
                "name": "Hunted",
                "paths": [self.repository],
                "repositories": [{ "path": self.dir.text("out/missing"), "remote_url": self.bare }],
            }),
        )
        .unwrap();
    }

    fn plausible(&self, id: &str) -> Vec<Value> {
        let out = self.dir.text("out");
        let exported = self.dir.text("out/shared.toml");
        let file = self.dir.text("shared.toml");
        std::fs::write(
            &file,
            "format = \"bitbraid-workspace\"\nname = \"Shared\"\n\
             [[repositories]]\npath = \"repo\"\n[[repositories]]\nname = \"gone\"\n",
        )
        .unwrap();
        let code = self.dir.text("x.code-workspace");
        std::fs::write(&code, "{ \"folders\": [ { \"path\": \"repo\" }, ] }").unwrap();
        match id {
            "workspace.list" => vec![json!({}), json!({ "id": "hunted" })],
            "workspace.create" => vec![json!({
                "name": "Made", "colour": "#123456", "icon": "star", "description": "d",
                "default_profile": "P", "layout": { "left_width": 10.0 },
                "paths": [self.repository], "repositories": [{ "path": self.dir.text("out/r"), "remote_url": self.bare }],
            })],
            "workspace.update" => vec![json!({
                "id": "hunted", "name": "Renamed", "colour": "", "icon": "x",
                "description": "", "default_profile": "P", "follows_folder": out,
                "layout": { "closed_sections": ["a"] },
            })],
            "workspace.delete" => vec![json!({ "id": "hunted" })],
            "workspace.reorder" => vec![json!({ "ids": ["hunted"] })],
            "workspace.add" => vec![json!({
                "id": "hunted", "paths": [self.repository],
                "repositories": [{ "path": self.dir.text("out/z"), "remote_url": self.bare }],
            })],
            "workspace.remove" => vec![json!({ "id": "hunted", "paths": [self.repository] })],
            "workspace.scan" => vec![json!({ "path": self.dir.text(""), "depth": 2, "limit": 5 })],
            "workspace.sync_folder" => vec![json!({ "id": "hunted" })],
            "workspace.import_code_workspace" => {
                vec![
                    json!({ "file": code, "id": "hunted" }),
                    json!({ "file": code, "name": "Code" }),
                ]
            }
            "workspace.fetch" => vec![json!({
                "id": "hunted", "paths": [self.repository], "parallel": 2,
                "prune": true, "tags": true, "quiet": true,
            })],
            "workspace.pull" => vec![json!({ "id": "hunted", "mode": "ff_only", "parallel": 1 })],
            "workspace.clone_missing" => {
                vec![json!({ "id": "hunted", "parallel": 2, "folder": out })]
            }
            "workspace.locate" => {
                vec![json!({ "id": "hunted", "path": self.repository, "to": self.repository })]
            }
            "workspace.defaults" => {
                vec![json!({ "id": "hunted", "path": self.repository, "apply_profile": false })]
            }
            "workspace.export" => {
                vec![json!({ "id": "hunted", "file": exported, "overwrite": true })]
            }
            "workspace.import" => vec![json!({
                "file": file, "folder": self.dir.text(""), "name": "Imported",
            })],
            other => panic!("no plausible parameters for {other}"),
        }
    }

    /// Values nobody means well with. The paths among them are inside
    /// the test's folder.
    fn hostile(&self) -> Vec<Value> {
        vec![
            Value::Null,
            json!(""),
            json!(" "),
            json!("-"),
            json!("--all"),
            json!("--upload-pack=touch injected"),
            json!(self.dir.text("out/injected")),
            json!(format!("{}/../../../../etc", self.dir.text("out"))),
            json!("../../../../../../etc/passwd"),
            json!("/"),
            json!("a\u{0}b"),
            json!("one\ntwo\r\n"),
            json!("\u{202e}\u{feff}名前 🦑"),
            json!("x".repeat(70_000)),
            json!(format!("/{}", "y/".repeat(5000))),
            json!(-1),
            json!(0),
            json!(u64::MAX),
            json!(1e308),
            json!(true),
            json!([]),
            json!({}),
            json!(["--force", "", null, 7, ["nested"]]),
            json!([{ "path": "relative" }, { "path": "/" }, { "remote_url": "x" }, 5]),
            json!({ "left_width": "wide", "section_sizes": { "a": -1 } }),
        ]
    }
}

/// Whether a value would have a command look at or write to the whole
/// file system's root, which is no place for a test.
fn names_the_root(params: &Value) -> bool {
    match params {
        Value::String(text) => text == "/",
        Value::Array(items) => items.iter().any(names_the_root),
        Value::Object(fields) => fields.values().any(names_the_root),
        _ => false,
    }
}

#[test]
fn every_workspace_command_takes_hostile_parameters() {
    let hunt = Hunt::new();
    let ids: Vec<&str> = hunt
        .registry
        .iter()
        .map(|spec| spec.id)
        .filter(|id| id.starts_with("workspace."))
        .collect();
    assert!(ids.len() >= 16, "{ids:?}");
    let mut ran = 0usize;
    for id in ids {
        let plausible = hunt.plausible(id);
        let mut all = plausible.clone();
        for params in &plausible {
            let Some(object) = params.as_object() else {
                continue;
            };
            for key in object.keys() {
                for value in hunt.hostile() {
                    let mut changed = object.clone();
                    changed.insert(key.clone(), value);
                    all.push(Value::Object(changed));
                }
            }
        }
        all.extend([
            Value::Null,
            json!({}),
            json!([]),
            json!("text"),
            json!(7),
            json!({ "": "", "unknown": { "deep": [1, 2, 3] } }),
        ]);
        for params in all {
            // A scan of the whole machine, or a file or a clone written at
            // its root, is not something to do in a test, whatever it
            // would answer.
            if names_the_root(&params) {
                continue;
            }
            hunt.reset();
            let _ = hunt.try_run(id, &params);
            ran += 1;
        }
    }
    assert!(ran > 400, "{ran} runs are too few to have tried much");
    // Nothing was written where a hostile value pointed, outside its
    // folder: no option was taken for one.
    assert!(!hunt.dir.join("repo/injected").exists());
    assert!(!Path::new("injected").exists());
}

/// Texts of `workspaces.toml` that were not written here.
fn hostile_workspace_files() -> Vec<Vec<u8>> {
    let mut files: Vec<Vec<u8>> = vec![
        b"".to_vec(),
        b"\xff\xfe\x00garbage".to_vec(),
        "\u{feff}version = 1\n".into(),
        "version = 1\r\n[[workspaces]]\r\nid = \"a\"\r\nname = \"A\"\r\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\nid = \"b\"\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\nrepositories = 5\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\nlayout = { left_width = inf }\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\nlayout = { section_sizes = { a = nan } }\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\n[[workspaces.repositories]]\npath = \"/a\\u0000b\"\n".into(),
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\n[[workspaces.repositories]]\npath = \"/a\"\nremote_url = \"-oProxyCommand=x\"\n".into(),
        "version = 18446744073709551616\n".into(),
        "version = 4294967296\n".into(),
        format!("[[workspaces]]\nid = \"a\"\nname = \"{}\"\n", "n".repeat(100_000)).into(),
        format!("x = {}{}\n", "[".repeat(5000), "]".repeat(5000)).into(),
        // Nested as deep as the bound on the size lets it be.
        format!("x = {}{}\n", "[".repeat(1_000_000), "]".repeat(1_000_000)).into(),
        format!("x = {}\n", "{a = ".repeat(700_000)).into(),
        format!("x = {}1{}\n", "{a = ".repeat(5000), "}".repeat(5000)).into(),
        format!("[{}]\n", "a.".repeat(5000) + "b").into(),
    ];
    let many: String = std::iter::once("[[workspaces]]\nid = \"a\"\nname = \"A\"\n".to_owned())
        .chain((0..1001).map(|n| format!("[[workspaces.repositories]]\npath = \"/r{n}\"\n")))
        .collect();
    files.push(many.into());
    let mut large = b"# ".to_vec();
    large.extend(std::iter::repeat_n(b'x', 4 * 1024 * 1024));
    files.push(large);
    files
}

#[test]
fn a_hostile_workspaces_file_is_refused_or_read_never_a_panic() {
    let hunt = Hunt::new();
    let config = hunt.dir.join("config");
    for text in hostile_workspace_files() {
        let _ = std::fs::remove_dir_all(&config);
        std::fs::create_dir_all(&config).unwrap();
        let file = config.join(bitbraid_engine::workspace::FILE);
        std::fs::write(&file, &text).unwrap();
        let before = std::fs::read(&file).unwrap();
        let listed = hunt.try_run("workspace.list", &json!({}));
        let made = hunt.try_run("workspace.create", &json!({ "name": "New" }));
        // A file that cannot be read is not written over.
        if listed.is_err() {
            assert!(made.is_err());
            assert_eq!(std::fs::read(&file).unwrap(), before);
        }
        for id in ["workspace.fetch", "workspace.defaults", "workspace.export"] {
            let _ = hunt.try_run(
                id,
                &json!({ "id": "a", "path": "/a", "file": hunt.dir.text("out.toml") }),
            );
        }
    }
    // Not a file at all: a folder, and a link to a device.
    let _ = std::fs::remove_dir_all(&config);
    std::fs::create_dir_all(config.join(bitbraid_engine::workspace::FILE)).unwrap();
    assert!(hunt.try_run("workspace.list", &json!({})).is_err());
    #[cfg(unix)]
    {
        let _ = std::fs::remove_dir_all(&config);
        std::fs::create_dir_all(&config).unwrap();
        std::os::unix::fs::symlink("/dev/zero", config.join(bitbraid_engine::workspace::FILE))
            .unwrap();
        let said = hunt.try_run("workspace.list", &json!({})).unwrap_err();
        assert!(said.contains("not a file"), "{said}");
    }
}

#[test]
fn a_hostile_code_workspace_or_workspace_file_is_refused_or_read_never_a_panic() {
    let hunt = Hunt::new();
    let out = hunt.dir.join("out");
    std::fs::create_dir_all(&out).unwrap();
    let code_files: Vec<String> = vec![
        String::new(),
        "{".to_owned(),
        "/* never closed".to_owned(),
        "{ \"folders\": [ { \"path\": \"a\\".to_owned(),
        format!("{}{}", "[".repeat(100_000), "]".repeat(100_000)),
        format!("{{\"folders\": [{}]}}", "{\"path\": \"x\"},".repeat(5000)),
        "{\"folders\": [{\"path\": \"\\u0000\"}, {\"path\": \"/dev/null\"}, {\"uri\": \"file:///%2e%2e/%00\"}, {\"uri\": \"file://\"}, {\"path\": null}, {\"path\": [1]}]}".to_owned(),
        format!("{{\"folders\": [{{\"path\": \"{}\"}}]}}", "../".repeat(10_000)),
        format!("{{\"folders\": [{{\"path\": \"{}\"}}]}}", "z".repeat(70_000)),
        "{\"folders\": [], \"folders\": [{\"path\": \"repo\"}]}".to_owned(),
        "\u{feff}\u{feff}{}".to_owned(),
        "{\"folders\": [{\"path\": \"a\" /* c */ , } , ] , }".to_owned(),
    ];
    let code = out.join("hostile.code-workspace");
    for text in &code_files {
        std::fs::write(&code, text).unwrap();
        let _ = hunt.try_run("workspace.import_code_workspace", &json!({ "file": code }));
    }
    let shared_files: Vec<String> = vec![
        String::new(),
        "format = \"bitbraid-workspace\"\nname = \"A\"\nrepositories = [1, 2]\n".to_owned(),
        "format = \"bitbraid-workspace\"\nname = \"A\"\n[[repositories]]\npath = \"\\u0000\"\n"
            .to_owned(),
        format!(
            "format = \"bitbraid-workspace\"\nname = \"A\"\n[[repositories]]\npath = \"{}\"\n",
            "../".repeat(5000)
        ),
        format!(
            "format = \"bitbraid-workspace\"\nname = \"A\"\n[[repositories]]\nname = \"{}\"\n",
            "n".repeat(70_000)
        ),
        format!(
            "format = \"bitbraid-workspace\"\nname = \"A\"\nx = {}{}\n",
            "[".repeat(5000),
            "]".repeat(5000)
        ),
        "format = \"bitbraid-workspace\"\nname = \"A\"\nlayout = { section_order = [\"a\\nb\"] }\n"
            .to_owned(),
        "format = \"bitbraid-workspace\"\nname = \"A\"\ndefault_profile = \"\\u001b[2J\"\n"
            .to_owned(),
    ];
    let shared = out.join("hostile.toml");
    for (n, text) in shared_files.iter().enumerate() {
        std::fs::write(&shared, text).unwrap();
        let _ = hunt.try_run(
            "workspace.import",
            &json!({ "file": shared, "name": format!("Imported {n}") }),
        );
    }
    // A file that is not one: a folder, a device, a pipe that would never
    // end if it were read.
    let pipe = out.join("pipe");
    let made = std::process::Command::new("mkfifo").arg(&pipe).status();
    let mut odd: Vec<PathBuf> = vec![out.clone(), PathBuf::from("/dev/zero")];
    if made.is_ok_and(|status| status.success()) {
        odd.push(pipe);
    }
    for file in odd {
        assert!(
            hunt.try_run("workspace.import_code_workspace", &json!({ "file": file }))
                .is_err()
        );
        assert!(
            hunt.try_run("workspace.import", &json!({ "file": file }))
                .is_err()
        );
    }
}
