//! A workspace's file to share, its defaults and a repository that moved.
//! The file is written and read back in another folder, as by someone who
//! cloned the repository it is kept in; a hostile one is refused where it
//! would lead out of its folder. A workspace's profile is written to a
//! repository's configuration as `git config` writes the same values on a
//! twin.

use std::path::Path;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, fixtures_out_dir};
use serde_json::{Value, json};

use crate::workspaces::Dir;

struct Harness {
    dir: Dir,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let dir = Dir::new("workspace-files");
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
        let name = Path::new(rel).file_name().unwrap().to_str().unwrap();
        git.run([
            "remote",
            "add",
            "origin",
            &format!("https://example.invalid/{name}.git"),
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

fn paths(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["path"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_moved_repository_is_located() {
    let mut h = Harness::new();
    let a = h.repository("a");
    let other = h.repository("other");
    h.run(
        "workspace.create",
        json!({ "name": "Moving", "paths": [a] }),
    );
    std::fs::rename(&a, h.dir.join("moved")).unwrap();
    let listed = h.run("workspace.list", json!({ "id": "moving" }));
    assert_eq!(listed["workspaces"][0]["missing"], json!([a]));

    let moved = h.dir.text("moved");
    let out = h.run(
        "workspace.locate",
        json!({ "id": "moving", "path": a, "to": format!("{moved}/") }),
    );
    assert_eq!(out["same_remote"], true);
    assert_eq!(out["to"], moved.as_str());
    assert_eq!(
        paths(&out["workspace"]["repositories"]),
        std::slice::from_ref(&moved)
    );
    assert_eq!(out["workspace"]["missing"], json!([]));

    // Another repository is taken, and said to be another.
    let out = h.run(
        "workspace.locate",
        json!({ "id": "moving", "path": moved, "to": other }),
    );
    assert_eq!(out["same_remote"], false);
    // Its address stays the one the entry had.
    assert_eq!(
        out["workspace"]["repositories"][0]["remote_url"],
        "https://example.invalid/a.git"
    );

    std::fs::create_dir_all(h.dir.join("plain")).unwrap();
    for (params, expected) in [
        (
            json!({ "id": "moving", "path": other, "to": h.dir.text("plain") }),
            "not the folder of a repository",
        ),
        (
            json!({ "id": "moving", "path": other, "to": h.dir.text("none") }),
            "not the folder",
        ),
        (
            json!({ "id": "moving", "path": h.dir.text("x"), "to": other }),
            "no repository at",
        ),
        (
            json!({ "id": "moving", "path": other, "to": "rel" }),
            "full path",
        ),
        (json!({ "id": "moving", "path": other }), "to"),
    ] {
        let said = h.refused("workspace.locate", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
}

#[test]
fn a_workspace_opens_its_repositories_with_its_profile_and_layout() {
    let mut h = Harness::new();
    let a = h.repository("a");
    let twin = h.repository("twin");
    let key_file = h.dir.join("keys/id_work");
    std::fs::create_dir_all(key_file.parent().unwrap()).unwrap();
    std::fs::write(&key_file, "").unwrap();
    let key_file = key_file.to_str().unwrap().to_owned();
    // Every part of the identity is the repository's own, so that what
    // a global configuration says cannot make it in effect.
    h.run(
        "profile.save",
        json!({ "profile": {
            "name": "Work", "user_name": "Ada Work", "user_email": "ada@work.invalid",
            "signing_key": "ABCD1234", "signing_format": "openpgp", "ssh_key": key_file,
        } }),
    );
    for git in [h.git("a"), h.git("twin")] {
        git.run(["config", "user.name", "Somebody Else"]).unwrap();
    }
    let layout = json!({ "left_width": 300.0, "closed_sections": ["tags", "stashes"], "right_panel": false });
    h.run(
        "workspace.create",
        json!({ "name": "Work", "paths": [a, twin], "default_profile": "Work", "layout": layout }),
    );

    let out = h.run("workspace.defaults", json!({ "id": "work", "path": a }));
    assert_eq!(out["profile"]["applied"], true, "{out}");
    assert!(out["profile"]["error"].is_null(), "{out}");
    assert_eq!(out["layout"], layout);
    // As git config writes the same identity.
    let twin_git = h.git("twin");
    for (key, value) in [
        ("user.name", "Ada Work"),
        ("user.email", "ada@work.invalid"),
        ("user.signingkey", "ABCD1234"),
        ("gpg.format", "openpgp"),
    ] {
        twin_git.run(["config", key, value]).unwrap();
    }
    let read = |git: &Git, key: &str| {
        git.run(["config", "--local", "--get", key])
            .unwrap_or_default()
    };
    let ours = h.git("a");
    for key in ["user.name", "user.email", "user.signingkey", "gpg.format"] {
        assert_eq!(read(&ours, key), read(&twin_git, key), "{key}");
    }
    assert!(read(&ours, "core.sshCommand").contains("id_work"));
    // Once in effect, it is not written again; and it is undone there.
    let config_before = ours.run(["config", "--local", "--list"]).unwrap();
    let again = h.run("workspace.defaults", json!({ "id": "work", "path": a }));
    assert_eq!(
        (
            again["profile"]["applied"].clone(),
            again["profile"]["already"].clone()
        ),
        (json!(false), json!(true)),
        "{again}"
    );
    assert_eq!(
        ours.run(["config", "--local", "--list"]).unwrap(),
        config_before
    );
    let mut there = CommandContext::default();
    h.registry
        .run("repo.open", &mut there, &json!({ "path": a }))
        .unwrap();
    let peek = h.registry.run("undo.peek", &mut there, &json!({})).unwrap();
    assert_eq!(peek["undo"], "Use profile Work", "{peek}");

    // Asked not to, it only says which.
    let only = h.run(
        "workspace.defaults",
        json!({ "id": "work", "path": twin, "apply_profile": false }),
    );
    assert_eq!(only["profile"]["applied"], false);
    assert_eq!(read(&h.git("twin"), "user.name"), "Ada Work\n");

    // A profile that is gone (deleting one takes it out of the workspaces;
    // a workspace can still name one that is not there) is said, and the
    // layout still comes.
    h.run(
        "workspace.update",
        json!({ "id": "work", "default_profile": "Gone" }),
    );
    let gone = h.run("workspace.defaults", json!({ "id": "work", "path": twin }));
    assert!(
        gone["profile"]["error"]
            .as_str()
            .unwrap()
            .contains("no profile"),
        "{gone}"
    );
    assert_eq!(gone["layout"], layout);
    // Without a default profile there is none to write.
    h.run(
        "workspace.update",
        json!({ "id": "work", "default_profile": "" }),
    );
    let none = h.run("workspace.defaults", json!({ "id": "work", "path": twin }));
    assert!(none["profile"].is_null());
    let said = h.refused(
        "workspace.defaults",
        json!({ "id": "work", "path": h.dir.text("elsewhere") }),
    );
    assert!(said.contains("not one of"), "{said}");
}

#[test]
fn a_workspace_file_is_read_back_in_another_folder() {
    let mut h = Harness::new();
    let a = h.repository("team/a");
    let b = h.repository("team/sub/b");
    let c = h.repository("elsewhere/c");
    h.run(
        "workspace.create",
        json!({
            "name": "Team",
            "colour": "#336699",
            "icon": "users",
            "description": "the team's",
            "layout": { "left_width": 250.0 },
            "paths": [a, b, c],
            "repositories": [{ "path": h.dir.text("team/d"), "remote_url": "https://example.invalid/d.git" }],
        }),
    );
    let file = h.dir.text("team/workspace.toml");
    let out = h.run("workspace.export", json!({ "id": "team", "file": file }));
    assert_eq!(out["repositories"], 4);
    assert_eq!(out["by_name"], json!([c]));
    let text = std::fs::read_to_string(&file).unwrap();
    for line in [
        "format = \"bitbraid-workspace\"",
        "path = \"a\"",
        "path = \"sub/b\"",
        "name = \"c\"",
        "path = \"d\"",
        "remote_url = \"https://example.invalid/d.git\"",
    ] {
        assert!(text.contains(line), "{line}\n{text}");
    }
    assert!(
        !text.contains(h.dir.text("").as_str()),
        "no full path is written:\n{text}"
    );
    let said = h.refused("workspace.export", json!({ "id": "team", "file": file }));
    assert!(said.contains("overwrite"), "{said}");
    h.run(
        "workspace.export",
        json!({ "id": "team", "file": file, "overwrite": true }),
    );

    // Someone else has the file in a folder of their own, with a and b
    // cloned there, and neither c nor d.
    let mate = h.dir.join("mate");
    std::fs::create_dir_all(mate.join("sub")).unwrap();
    for (from, to) in [(&a, "mate/a"), (&b, "mate/sub/b")] {
        h.git("mate")
            .run(["clone", "-q", from.as_str(), h.dir.text(to).as_str()])
            .unwrap();
    }
    let theirs = h.dir.text("mate/workspace.toml");
    std::fs::copy(&file, &theirs).unwrap();
    let said = h.refused("workspace.import", json!({ "file": theirs }));
    assert!(said.contains("already"), "{said}");
    let out = h.run(
        "workspace.import",
        json!({ "file": theirs, "name": "Team mate" }),
    );
    let workspace = &out["workspace"];
    assert_eq!(workspace["id"], "team-mate");
    assert_eq!(workspace["colour"], "#336699");
    assert_eq!(workspace["icon"], "users");
    assert_eq!(workspace["layout"]["left_width"], 250.0);
    assert_eq!(
        paths(&workspace["repositories"]),
        [
            h.dir.text("mate/a"),
            h.dir.text("mate/sub/b"),
            h.dir.text("mate/c"),
            h.dir.text("mate/d")
        ]
    );
    assert_eq!(
        workspace["missing"],
        json!([h.dir.text("mate/c"), h.dir.text("mate/d")])
    );
    // The addresses are the file's, whatever the clones here follow.
    assert_eq!(
        workspace["repositories"][0]["remote_url"],
        "https://example.invalid/a.git"
    );
    assert_eq!(
        workspace["repositories"][3]["remote_url"],
        "https://example.invalid/d.git"
    );
    assert_eq!(out["refused"], json!([]));

    // Into a folder that is named.
    let other = h.dir.join("other");
    std::fs::create_dir_all(&other).unwrap();
    let out = h.run(
        "workspace.import",
        json!({ "file": file, "folder": other, "name": "Third" }),
    );
    assert_eq!(
        out["workspace"]["missing"].as_array().unwrap().len(),
        4,
        "{out}"
    );
}

#[test]
fn a_hostile_workspace_file_never_leads_out_of_its_folder() {
    let mut h = Harness::new();
    let inside = h.repository("box/inside");
    let outside = h.repository("outside");
    std::fs::create_dir_all(h.dir.join("box/plain")).unwrap();
    bitbraid_testkit::symlink_dir(h.dir.join("outside"), h.dir.join("box/link")).unwrap();
    let file = h.dir.text("box/hostile.toml");
    // In TOML a `\` (Windows' separator) is written twice.
    let outside_in_toml = outside.replace('\\', "\\\\");
    std::fs::write(
        &file,
        format!(
            "format = \"bitbraid-workspace\"\nversion = 1\nname = \"Hostile\"\nextra = \"ignored\"\n\
             [[repositories]]\npath = \"inside\"\n\
             [[repositories]]\npath = \"{outside_in_toml}\"\n\
             [[repositories]]\npath = \"../outside\"\n\
             [[repositories]]\npath = \"inside/../../outside\"\n\
             [[repositories]]\npath = \"link\"\n\
             [[repositories]]\npath = \"link/deeper\"\nremote_url = \"https://example.invalid/x.git\"\n\
             [[repositories]]\nname = \"..\"\n\
             [[repositories]]\nname = \"a/b\"\n\
             [[repositories]]\npath = \"plain\"\n\
             [[repositories]]\npath = \"gone\"\n\
             [[repositories]]\npath = \"gone2\"\nremote_url = \"--upload-pack=touch /tmp/x\"\n\
             [[repositories]]\nremote_url = \"https://example.invalid/y.git\"\n\
             [[repositories]]\npath = \"later\"\nremote_url = \"https://example.invalid/later.git\"\n"
        ),
    )
    .unwrap();
    let out = h.run("workspace.import", json!({ "file": file }));
    assert_eq!(
        paths(&out["workspace"]["repositories"]),
        [inside.clone(), h.dir.text("box/later")]
    );
    let refused: Vec<(String, String)> = out["refused"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["repository"].as_str().unwrap().to_owned(),
                entry["why"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let why = |what: &str| {
        refused
            .iter()
            .find(|(named, _)| named == what)
            .map(|(_, why)| why.clone())
            .unwrap_or_else(|| panic!("{what} was not refused: {refused:?}"))
    };
    assert!(why(&outside).contains("full path"));
    assert!(why("../outside").contains("leads out"));
    assert!(why("inside/../../outside").contains("leads out"));
    assert!(why("link").contains("symbolic link"));
    assert!(why("link/deeper").contains("symbolic link"));
    assert!(why("..").contains("name"));
    assert!(why("a/b").contains("name"));
    assert!(why("plain").contains("not the folder of a repository"));
    assert!(why("gone").contains("no address"));
    assert!(why("gone2").contains("remote_url"));
    assert!(why("number 12").contains("neither"));
    assert_eq!(refused.len(), 11, "{refused:?}");

    // Files that are no workspace files, and fields that cannot be.
    for (text, expected) in [
        ("name = \"A\"\n", "format"),
        ("format = \"other\"\nname = \"A\"\n", "format"),
        (
            "format = \"bitbraid-workspace\"\nversion = 9\nname = \"A\"\n",
            "newer",
        ),
        ("format = \"bitbraid-workspace\"\n", "cannot be understood"),
        (
            "format = \"bitbraid-workspace\"\nname = \"A\"\ncolour = \"red\"\n",
            "colour",
        ),
        ("format = \"bitbraid-workspace\"\nname = \"\"\n", "name"),
        (
            "format = \"bitbraid-workspace\"\nname = \"A\"\nlayout = { left_width = nan }\n",
            "layout",
        ),
        ("not = [toml", "cannot be understood"),
    ] {
        std::fs::write(&file, text).unwrap();
        let said = h.refused("workspace.import", json!({ "file": file }));
        assert!(said.contains(expected), "{text}: {said}");
    }
    let mut large = String::from("format = \"bitbraid-workspace\"\nname = \"A\"\n# ");
    large.push_str(&"x".repeat(1024 * 1024));
    std::fs::write(&file, large).unwrap();
    let said = h.refused("workspace.import", json!({ "file": file }));
    assert!(said.contains("larger"), "{said}");
    let many: String =
        std::iter::once("format = \"bitbraid-workspace\"\nname = \"A\"\n".to_owned())
            .chain((0..1001).map(|n| format!("[[repositories]]\nname = \"r{n}\"\n")))
            .collect();
    std::fs::write(&file, many).unwrap();
    let said = h.refused("workspace.import", json!({ "file": file }));
    assert!(said.contains("more than"), "{said}");
    for (params, expected) in [
        (json!({ "file": "rel.toml" }), "full path"),
        (json!({ "file": h.dir.text("none.toml") }), "cannot be read"),
        (
            json!({ "file": file, "folder": h.dir.text("nowhere") }),
            "not a folder",
        ),
        (json!({ "file": file, "name": "a\nb" }), "control"),
    ] {
        let said = h.refused("workspace.import", params.clone());
        assert!(said.contains(expected), "{params}: {said}");
    }
}
