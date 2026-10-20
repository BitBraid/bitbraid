//! Workspace commands against the file they keep: making,
//! changing and deleting workspaces is read back from `workspaces.toml`
//! on disk; adding repositories looks at them as git does (whether a
//! folder is a repository, where it is cloned from) and changes none of
//! them.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry, workspace};
use bitbraid_testkit::{Git, fixtures_out_dir};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory of the test's own among the scratch copies, removed with
/// it.
pub struct Dir(PathBuf);

impl Dir {
    pub fn new(what: &str) -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = fixtures_out_dir()
            .join("scratch")
            .join(format!("{what}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    /// `name`, with `/` between its folders, under the directory: with
    /// the system's separator, as the commands answer with paths.
    pub fn join(&self, name: &str) -> PathBuf {
        bitbraid_core::paths::join(&self.0, name)
    }
    pub fn text(&self, name: &str) -> String {
        self.join(name).to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A full path of this system for one written the Unix way: on Windows
/// on drive C, with its separators.
pub fn full(path: &str) -> String {
    if cfg!(windows) {
        format!("C:{}", path.replace('/', "\\"))
    } else {
        path.to_owned()
    }
}

/// What git says a repository is: every reference, the status and the
/// configuration, to hold it to what it was.
pub fn state_of(git: &Git) -> (Vec<String>, String, String) {
    (
        git.lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap(),
        git.run(["status", "--porcelain", "--ignored"]).unwrap(),
        git.run(["config", "--local", "--list"]).unwrap(),
    )
}

struct Harness {
    dir: Dir,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let dir = Dir::new("workspaces");
        let ctx = CommandContext {
            config_dir: Some(dir.join("config")),
            ..CommandContext::default()
        };
        Self {
            dir,
            registry: builtin_registry(),
            ctx,
        }
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn refused(&mut self, id: &str, params: Value) -> String {
        match self.registry.run(id, &mut self.ctx, &params) {
            Ok(out) => panic!("{id} {params} was taken: {out}"),
            Err(err) => err.to_string(),
        }
    }
    fn file(&self) -> PathBuf {
        self.dir.join("config").join(workspace::FILE)
    }
    /// The workspaces as the file on disk has them.
    fn on_disk(&self) -> Vec<workspace::Workspace> {
        workspace::load(&self.dir.join("config"))
            .unwrap()
            .workspaces
    }
    /// A repository of the test's own with an `origin`.
    fn repository(&self, name: &str) -> (String, Git) {
        let path = self.dir.join(name);
        std::fs::create_dir_all(&path).unwrap();
        let git = Git::new(&path, fixtures_out_dir().join(".home"));
        git.ensure_home().unwrap();
        git.run(["init", "-q", "-b", "main"]).unwrap();
        git.run([
            "remote",
            "add",
            "origin",
            &format!("https://example.invalid/{name}.git"),
        ])
        .unwrap();
        (path.to_str().unwrap().to_owned(), git)
    }
}

#[test]
fn a_workspace_is_made_changed_and_deleted_in_its_file() {
    let mut h = Harness::new();
    assert_eq!(h.run("workspace.list", json!({}))["workspaces"], json!([]));
    assert!(!h.file().exists(), "listing writes nothing");

    let made = h.run(
        "workspace.create",
        json!({
            "name": "  Web Services ",
            "colour": "#A1B2C3",
            "icon": "globe",
            "description": "the services\nof the web",
            "default_profile": "Work",
            "layout": { "left_width": 280.0, "closed_sections": ["tags"] },
        }),
    );
    assert_eq!(made["created"], true);
    let workspace = &made["workspace"];
    assert_eq!(workspace["id"], "web-services");
    assert_eq!(workspace["name"], "Web Services");
    assert_eq!(workspace["colour"], "#a1b2c3");
    assert_eq!(workspace["layout"]["left_width"], 280.0);
    assert_eq!(workspace["repositories"], json!([]));
    assert_eq!(workspace["missing"], json!([]));
    let disk = h.on_disk();
    assert_eq!(disk.len(), 1);
    assert_eq!(disk[0].default_profile.as_deref(), Some("Work"));
    assert_eq!(
        disk[0].layout.as_ref().unwrap().closed_sections,
        Some(vec!["tags".to_owned()])
    );

    // One name is one workspace, in whatever case; another gets an id of
    // its own.
    let said = h.refused("workspace.create", json!({ "name": "web SERVICES" }));
    assert!(said.contains("already"), "{said}");
    let other = h.run("workspace.create", json!({ "name": "Web-Services!" }));
    assert_eq!(other["workspace"]["id"], "web-services-2");

    // Renamed, it keeps its id; what is not given stays; empty takes away.
    let changed = h.run(
        "workspace.update",
        json!({ "id": "web-services", "name": "Front", "colour": "", "layout": {} }),
    );
    assert_eq!(changed["renamed_from"], "Web Services");
    let workspace = &changed["workspace"];
    assert_eq!(workspace["id"], "web-services");
    assert_eq!(workspace["name"], "Front");
    assert!(workspace.get("colour").is_none(), "{workspace}");
    assert!(workspace.get("layout").is_none(), "{workspace}");
    assert_eq!(workspace["icon"], "globe");
    assert_eq!(workspace["description"], "the services\nof the web");
    let said = h.refused(
        "workspace.update",
        json!({ "id": "web-services", "name": "WEB-SERVICES!" }),
    );
    assert!(said.contains("already"), "{said}");
    // Its own name in another case is still its own.
    h.run(
        "workspace.update",
        json!({ "id": "web-services", "name": "FRONT" }),
    );

    let one = h.run("workspace.list", json!({ "id": "web-services-2" }));
    assert_eq!(one["workspaces"].as_array().unwrap().len(), 1);
    assert_eq!(one["workspaces"][0]["name"], "Web-Services!");

    // Their order is the file's, and is changed by naming them.
    h.run("workspace.create", json!({ "name": "Third" }));
    let order = h.run(
        "workspace.reorder",
        json!({ "ids": ["third", "web-services-2"] }),
    );
    assert_eq!(
        order["order"],
        json!(["third", "web-services-2", "web-services"])
    );
    let ids: Vec<String> = h.on_disk().into_iter().map(|w| w.id).collect();
    assert_eq!(ids, ["third", "web-services-2", "web-services"]);
    for (ids, expected) in [
        (json!(["third", "third"]), "twice"),
        (json!(["nobody"]), "no workspace"),
        (json!(["Bad Id"]), "lowercase"),
    ] {
        let said = h.refused("workspace.reorder", json!({ "ids": ids }));
        assert!(said.contains(expected), "{ids}: {said}");
    }
    h.run("workspace.delete", json!({ "id": "third" }));

    let gone = h.run("workspace.delete", json!({ "id": "web-services" }));
    assert_eq!(gone["deleted"]["name"], "FRONT");
    let left: Vec<String> = h.on_disk().into_iter().map(|w| w.id).collect();
    assert_eq!(left, ["web-services-2"]);
    for id in ["workspace.delete", "workspace.update", "workspace.list"] {
        let said = h.refused(id, json!({ "id": "web-services" }));
        assert!(said.contains("no workspace"), "{id}: {said}");
    }
}

#[test]
fn repositories_are_added_as_git_sees_them_and_left_as_they_are() {
    let mut h = Harness::new();
    let (api, api_git) = h.repository("api");
    let (web, web_git) = h.repository("web");
    let before = (state_of(&api_git), state_of(&web_git));
    h.run("workspace.create", json!({ "name": "All", "paths": [api] }));
    let added = h.run(
        "workspace.add",
        json!({
            "id": "all",
            "paths": [format!("{web}/"), api],
            "repositories": [{ "path": h.dir.text("later"), "remote_url": "https://example.invalid/later.git" }],
        }),
    );
    assert_eq!(added["added"], json!([web, h.dir.text("later")]));
    assert_eq!(added["already"], json!([api]));
    let workspace = &added["workspace"];
    assert_eq!(
        workspace["repositories"],
        json!([
            { "path": api, "remote_url": "https://example.invalid/api.git" },
            { "path": web, "remote_url": "https://example.invalid/web.git" },
            { "path": h.dir.text("later"), "remote_url": "https://example.invalid/later.git" },
        ])
    );
    assert_eq!(workspace["missing"], json!([h.dir.text("later")]));

    // What is not a repository, or not there without an address, is not
    // taken, and nothing of the call is.
    std::fs::create_dir_all(h.dir.join("plain")).unwrap();
    std::fs::create_dir_all(h.dir.join("api/inside")).unwrap();
    std::fs::write(h.dir.join("file"), "x").unwrap();
    let file_before = std::fs::read(h.file()).unwrap();
    for (paths, expected) in [
        (
            json!([h.dir.text("plain")]),
            "not the folder of a repository",
        ),
        (
            json!([h.dir.text("api/inside")]),
            "not the folder of a repository",
        ),
        (json!([h.dir.text("file")]), "not a folder"),
        (json!([h.dir.text("nowhere")]), "no address"),
        (json!(["relative/path"]), "full path"),
        (json!([format!("{api}/../web")]), ".."),
        (json!([web, h.dir.text("plain")]), "not the folder"),
    ] {
        let said = h.refused("workspace.add", json!({ "id": "all", "paths": paths }));
        assert!(said.contains(expected), "{paths}: {said}");
    }
    let said = h.refused("workspace.add", json!({ "id": "all" }));
    assert!(said.contains("at least one"), "{said}");
    assert_eq!(std::fs::read(h.file()).unwrap(), file_before);

    let removed = h.run(
        "workspace.remove",
        json!({ "id": "all", "paths": [format!("{web}/"), h.dir.text("plain")] }),
    );
    assert_eq!(removed["removed"], json!([web]));
    assert_eq!(removed["not_there"], json!([h.dir.text("plain")]));
    let kept: Vec<String> = h.on_disk()[0]
        .repositories
        .iter()
        .map(|entry| entry.path.clone())
        .collect();
    assert_eq!(kept, [api.clone(), h.dir.text("later")]);

    h.run("workspace.delete", json!({ "id": "all" }));
    assert_eq!((state_of(&api_git), state_of(&web_git)), before);
    assert!(Path::new(&web).join(".git").is_dir());
}

#[test]
fn a_file_that_cannot_be_read_is_said_and_left_alone() {
    let mut h = Harness::new();
    std::fs::create_dir_all(h.dir.join("config")).unwrap();
    for text in [
        "[[workspaces]]\nid = \"a\"\n",
        "not toml at all [",
        "[[workspaces]]\nid = \"a\"\nname = \"A\"\n[[workspaces.repositories]]\npath = \"../x\"\n",
    ] {
        std::fs::write(h.file(), text).unwrap();
        let said = h.refused("workspace.list", json!({}));
        assert!(said.contains("left as it is"), "{said}");
        let said = h.refused("workspace.create", json!({ "name": "B" }));
        assert!(said.contains("left as it is"), "{said}");
        assert_eq!(std::fs::read_to_string(h.file()).unwrap(), text);
    }
    // A newer version's file is read, and not written over.
    let newer =
        "version = 2\nfuture = true\n[[workspaces]]\nid = \"a\"\nname = \"A\"\nshape = \"round\"\n";
    std::fs::write(h.file(), newer).unwrap();
    let listed = h.run("workspace.list", json!({}));
    assert_eq!(listed["workspaces"][0]["name"], "A");
    assert!(listed["read_only"].as_str().unwrap().contains("newer"));
    let said = h.refused("workspace.update", json!({ "id": "a", "name": "B" }));
    assert!(said.contains("newer"), "{said}");
    assert_eq!(std::fs::read_to_string(h.file()).unwrap(), newer);

    // Without a configuration directory there is nowhere to keep them.
    let mut ctx = CommandContext::default();
    let said = h
        .registry
        .run("workspace.list", &mut ctx, &json!({}))
        .unwrap_err()
        .to_string();
    assert!(said.contains("configuration directory"), "{said}");
}

#[test]
fn what_a_workspace_is_given_is_checked() {
    let mut h = Harness::new();
    let long = "n".repeat(65);
    for (params, expected) in [
        (json!({}), "name"),
        (json!({ "name": "  " }), "empty"),
        (json!({ "name": long }), "longer"),
        (json!({ "name": "a\nb" }), "control"),
        (json!({ "name": "A", "colour": "red" }), "#rrggbb"),
        (json!({ "name": "A", "icon": "Big Star" }), "lowercase"),
        (json!({ "name": "A", "follows_folder": "src" }), "full path"),
        (
            json!({ "name": "A", "layout": { "left_width": -5 } }),
            "pixels",
        ),
        (json!({ "name": "A", "layout": "wide" }), "layout"),
        (json!({ "name": "A", "paths": "/one" }), "paths"),
        (json!({ "name": 7 }), "name"),
    ] {
        let said = h.refused("workspace.create", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
    assert!(!h.file().exists());
    for (params, expected) in [
        (json!({ "id": "" }), "id"),
        (json!({ "id": "../x" }), "id"),
        (json!({ "id": "A" }), "id"),
    ] {
        let said = h.refused("workspace.update", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
    // What would make the file larger than is read is not written: just
    // over the bound, with the longest paths a thousand times.
    let url = format!("https://example.invalid/{}", "u".repeat(200));
    let entries: Vec<Value> = (0..workspace::MAX_REPOSITORIES)
        .map(|n| json!({ "path": full(&format!("/{}/{n:04}", "p".repeat(4080))), "remote_url": url }))
        .collect();
    let said = h.refused(
        "workspace.create",
        json!({ "name": "Huge", "repositories": entries }),
    );
    assert!(said.contains("more than"), "{said}");
    assert!(!h.file().exists());
    let more: Vec<Value> = (0..=workspace::MAX_REPOSITORIES)
        .map(
            |n| json!({ "path": full(&format!("/r/{n}")), "remote_url": "https://example.invalid/r.git" }),
        )
        .collect();
    let said = h.refused(
        "workspace.create",
        json!({ "name": "Many", "repositories": more }),
    );
    assert!(said.contains("at most"), "{said}");
}
