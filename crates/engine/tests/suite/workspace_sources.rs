//! Where a workspace's repositories come from: a folder scanned
//! a few levels down, and a VS Code `.code-workspace` file. What is found
//! is held to what git says is a repository's own folder
//! (`git rev-parse --show-toplevel`, `--is-bare-repository`); what is
//! passed over is said, links are not followed, and nothing is changed.

use std::path::Path;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, fixtures_out_dir};
use serde_json::{Value, json};

use crate::workspaces::{Dir, state_of};

struct Tree {
    dir: Dir,
    registry: Registry,
    ctx: CommandContext,
}

impl Tree {
    fn new() -> Self {
        let dir = Dir::new("workspace-sources");
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
    fn git(&self, rel: &str) -> Git {
        let git = Git::new(self.dir.join(rel), fixtures_out_dir().join(".home"));
        git.ensure_home().unwrap();
        git
    }
    /// A repository at `rel` with one commit and an `origin`.
    fn repository(&self, rel: &str) -> String {
        std::fs::create_dir_all(self.dir.join(rel)).unwrap();
        let git = self.git(rel);
        git.run(["init", "-q", "-b", "main"]).unwrap();
        std::fs::write(self.dir.join(rel).join("f.txt"), rel).unwrap();
        git.run(["add", "f.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "one"]).unwrap();
        git.run([
            "remote",
            "add",
            "origin",
            &format!("https://example.invalid/{rel}.git"),
        ])
        .unwrap();
        self.dir.text(rel)
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
}

fn paths(found: &Value) -> Vec<String> {
    found
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap().to_owned())
        .collect()
}

/// What git says of a folder: whether it is a repository's own folder
/// (its top, or a bare repository itself).
fn git_says_repository(path: &Path) -> bool {
    let git = Git::new(path, fixtures_out_dir().join(".home"));
    if git
        .run(["rev-parse", "--is-bare-repository"])
        .is_ok_and(|said| said.trim() == "true")
    {
        return git
            .run(["rev-parse", "--absolute-git-dir"])
            .is_ok_and(|said| Path::new(said.trim()) == path);
    }
    git.run(["rev-parse", "--show-toplevel"])
        .is_ok_and(|said| Path::new(said.trim()) == path)
}

#[test]
fn a_scan_finds_what_git_calls_repositories_and_says_what_it_passed_over() {
    let mut t = Tree::new();
    let root = t.dir.join("root");
    let a = t.repository("root/a");
    // Inside a repository: its own business, not looked for.
    t.repository("root/a/vendored");
    let b = t.repository("root/group/b");
    std::fs::create_dir_all(t.dir.join("root/group")).unwrap();
    t.git("root/group")
        .run(["init", "-q", "--bare", "c.git"])
        .unwrap();
    let c = t.dir.text("root/group/c.git");
    let d = t.repository("root/one/two/d");
    t.repository("root/one/two/three/too-deep");
    std::fs::create_dir_all(t.dir.join("root/plain/empty")).unwrap();
    // A linked worktree has a .git file.
    t.git("root/a")
        .run([
            "worktree",
            "add",
            "-q",
            "-b",
            "side",
            t.dir.text("root/wt").as_str(),
        ])
        .unwrap();
    let wt = t.dir.text("root/wt");
    // A link to a repository elsewhere is not followed.
    let outside = t.repository("outside/e");
    bitbraid_testkit::symlink_dir(&outside, t.dir.join("root/link")).unwrap();
    let before = state_of(&t.git("root/a"));

    let scanned = t.run("workspace.scan", json!({ "path": root }));
    let mut found = paths(&scanned["found"]);
    found.sort();
    let mut expected = vec![a.clone(), b.clone(), c.clone(), d.clone(), wt.clone()];
    expected.sort();
    assert_eq!(found, expected, "{scanned}");
    for path in &found {
        assert!(git_says_repository(Path::new(path)), "{path}");
    }
    assert!(!git_says_repository(&t.dir.join("root/plain")));
    assert_eq!(scanned["complete"], true);
    assert_eq!(scanned["folder"], root.to_str().unwrap());
    let found_c = scanned["found"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == c.as_str())
        .unwrap();
    assert_eq!(found_c["bare"], true);
    let found_a = scanned["found"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["path"] == a.as_str())
        .unwrap();
    assert_eq!(found_a["remote_url"], "https://example.invalid/root/a.git");
    assert_eq!(found_a["bare"], false);
    let skipped: Vec<(String, String)> = scanned["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["path"].as_str().unwrap().to_owned(),
                entry["why"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert!(
        skipped.contains(&(
            t.dir.text("root/link"),
            "a symbolic link, which is not followed".to_owned()
        )),
        "{skipped:?}"
    );
    assert!(
        skipped
            .iter()
            .any(|(path, why)| *path == t.dir.text("root/one/two/three") && why.contains("deeper")),
        "{skipped:?}"
    );

    // Fewer levels, fewer repositories; a bound on the count says so.
    let shallow = t.run("workspace.scan", json!({ "path": root, "depth": 1 }));
    let mut found = paths(&shallow["found"]);
    found.sort();
    assert_eq!(found, [a.clone(), wt.clone()]);
    let two = t.run("workspace.scan", json!({ "path": root, "limit": 2 }));
    assert_eq!(two["found"].as_array().unwrap().len(), 2);
    assert_eq!(two["complete"], false);
    // A repository given is the one found.
    let itself = t.run("workspace.scan", json!({ "path": a }));
    assert_eq!(paths(&itself["found"]), std::slice::from_ref(&a));

    for (params, expected) in [
        (json!({ "path": "relative" }), "full path"),
        (json!({ "path": t.dir.text("nowhere") }), "not a folder"),
        (json!({ "path": root, "depth": 4 }), "at most 3"),
        (json!({ "path": root, "limit": 0 }), "from 1"),
        (json!({ "path": root, "limit": 501 }), "from 1"),
        (json!({ "path": root, "depth": -1 }), "depth"),
    ] {
        let said = t.refused("workspace.scan", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
    assert_eq!(state_of(&t.git("root/a")), before);
}

#[test]
fn a_workspace_that_follows_a_folder_takes_what_comes_into_it() {
    let mut t = Tree::new();
    let a = t.repository("src/a");
    let made = t.run(
        "workspace.create",
        json!({ "name": "Source", "follows_folder": t.dir.text("src") }),
    );
    assert_eq!(
        paths(&made["workspace"]["repositories"]),
        std::slice::from_ref(&a)
    );
    assert_eq!(
        made["workspace"]["repositories"][0]["remote_url"],
        "https://example.invalid/src/a.git"
    );
    let b = t.repository("src/b");
    let synced = t.run("workspace.sync_folder", json!({ "id": "source" }));
    assert_eq!(synced["added"], json!([b.clone()]));
    assert_eq!(paths(&synced["workspace"]["repositories"]), [a.clone(), b]);
    let again = t.run("workspace.sync_folder", json!({ "id": "source" }));
    assert_eq!(again["added"], json!([]));

    t.run("workspace.create", json!({ "name": "Free" }));
    let said = t.refused("workspace.sync_folder", json!({ "id": "free" }));
    assert!(said.contains("follows no folder"), "{said}");
    // Told to follow one later, it takes what is there at once.
    let followed = t.run(
        "workspace.update",
        json!({ "id": "free", "follows_folder": t.dir.text("src") }),
    );
    assert_eq!(followed["added"], json!([a.clone(), t.dir.text("src/b")]));
    let unfollowed = t.run(
        "workspace.update",
        json!({ "id": "free", "follows_folder": "" }),
    );
    assert!(unfollowed["workspace"].get("follows_folder").is_none());
    assert_eq!(unfollowed["added"], json!([]));
    assert_eq!(
        unfollowed["workspace"]["repositories"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let said = t.refused(
        "workspace.create",
        json!({ "name": "Gone", "follows_folder": t.dir.text("nowhere") }),
    );
    assert!(said.contains("not a folder"), "{said}");
}

/// A full path as the address of a file, as VS Code writes one: on
/// Windows with the drive after a slash and `/` between folders.
fn file_uri(path: &str) -> String {
    if cfg!(windows) {
        format!("file:///{}", path.replace('\\', "/"))
    } else {
        format!("file://{path}")
    }
}

#[test]
fn a_code_workspace_file_gives_its_folders_that_are_repositories() {
    let mut t = Tree::new();
    let a = t.repository("code/a");
    let b = t.repository("other/b");
    let c = t.repository("far/c d");
    std::fs::create_dir_all(t.dir.join("code/plain")).unwrap();
    let file = t.dir.join("code/project.code-workspace");
    std::fs::write(
        &file,
        format!(
            "\u{feff}{{\n  // what the editor opens\n  \"folders\": [\n    {{ \"path\": \"a\" }},\n    \
             {{ \"name\": \"B\", \"path\": \"../other/b/\" }}, /* elsewhere */\n    \
             {{ \"uri\": \"{}\" }},\n    {{ \"path\": \"plain\" }},\n    \
             {{ \"path\": \"missing\" }},\n    {{ \"uri\": \"vscode-remote://ssh-remote+box/x\" }},\n    \
             {{ \"path\": \"a\" }},\n  ],\n  \"settings\": {{ \"x\": 1, }},\n}}\n",
            file_uri(&c).replace(' ', "%20")
        ),
    )
    .unwrap();
    let file = file.to_str().unwrap().to_owned();
    let read = t.run("workspace.import_code_workspace", json!({ "file": file }));
    assert_eq!(paths(&read["found"]), [a.clone(), b.clone(), c.clone()]);
    let skipped: Vec<&str> = read["skipped"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["why"].as_str().unwrap())
        .collect();
    assert_eq!(
        skipped,
        [
            "not the folder of a repository",
            "not there",
            "an address that is not a folder on this machine"
        ]
    );
    assert!(read["workspace"].is_null());
    assert!(
        !t.dir.join("config").exists(),
        "only reading writes nothing"
    );

    let made = t.run(
        "workspace.import_code_workspace",
        json!({ "file": file, "name": "Project" }),
    );
    assert_eq!(made["workspace"]["id"], "project");
    assert_eq!(
        paths(&made["workspace"]["repositories"]),
        [a.clone(), b.clone(), c.clone()]
    );
    t.run(
        "workspace.create",
        json!({ "name": "Later", "paths": [b.clone()] }),
    );
    let added = t.run(
        "workspace.import_code_workspace",
        json!({ "file": file, "id": "later" }),
    );
    assert_eq!(added["added"], json!([a, c]));

    std::fs::write(
        t.dir.join("bad.code-workspace"),
        "{ \"folders\": [ { \"path\": ",
    )
    .unwrap();
    for (params, expected) in [
        (
            json!({ "file": t.dir.text("bad.code-workspace") }),
            "cannot be understood",
        ),
        (
            json!({ "file": t.dir.text("none.code-workspace") }),
            "cannot be read",
        ),
        (json!({ "file": t.dir.text("code") }), "not a file"),
        (json!({ "file": "project.code-workspace" }), "full path"),
        (
            json!({ "file": file, "id": "later", "name": "X" }),
            "not both",
        ),
        (json!({ "file": file, "id": "nobody" }), "no workspace"),
        (json!({ "file": file, "name": "Project" }), "already"),
    ] {
        let said = t.refused("workspace.import_code_workspace", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
}
