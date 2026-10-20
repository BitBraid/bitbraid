//! Profile commands against the file they keep and against `git config`:
//! saving, renaming and deleting are read back from `profiles.toml` on
//! disk; using a profile runs on one scratch copy while the same settings
//! are written with `git config` on a twin, and the two configurations
//! have to come out the same.
//!
//! Nothing here touches the configuration of whoever runs the tests: an
//! identity goes to a scratch repository or to a configuration file that
//! is named, never to the user's own.

use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

/// A directory of the test's own among the scratch copies, removed with
/// it. It lies inside this checkout, so git is only ever run in it on a
/// file that is named.
struct Dir(PathBuf);

impl Dir {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = fixtures_out_dir()
            .join("scratch")
            .join(format!("profiles-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
    fn text(&self, name: &str) -> String {
        self.join(name).to_str().unwrap().to_owned()
    }
    /// Git for `config --file` and nothing else.
    fn git(&self) -> Git {
        let git = Git::new(&self.0, fixtures_out_dir().join(".home"));
        git.ensure_home().unwrap();
        git
    }
    /// An empty file to stand for a private key, at `keys/<name>`.
    fn key(&self, name: &str) -> String {
        let path = self.join("keys").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "").unwrap();
        path.to_str().unwrap().to_owned()
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Harness {
    dir: Dir,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    /// No repository open; the profiles go to `config` in a directory of
    /// the test's own, which is not there yet.
    fn new() -> Self {
        let dir = Dir::new();
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
    /// A scratch copy open, and its twin.
    fn with_repository() -> (Self, Scratch, Scratch) {
        let mut h = Self::new();
        let ours = Scratch::of("branches-merges").unwrap();
        let twin = Scratch::of("branches-merges").unwrap();
        h.run("repo.open", json!({ "path": ours.path() }));
        (h, ours, twin)
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    fn refused(&mut self, id: &str, params: Value, because: &str) {
        let err = self
            .try_run(id, params.clone())
            .expect_err(&format!("{id} {params} should be refused"));
        assert!(err.contains(because), "{id} {params}: {err}");
    }
    fn file(&self) -> PathBuf {
        self.dir.join("config").join("profiles.toml")
    }
    /// The file as it is on disk, read by the test itself.
    fn on_disk(&self) -> Value {
        let text = std::fs::read_to_string(self.file()).unwrap();
        toml::from_str(&text).unwrap_or_else(|err| panic!("{err}\n{text}"))
    }
    fn save(&mut self, profile: Value) -> Value {
        self.run("profile.save", json!({ "profile": profile }))
    }
    fn names(&mut self) -> Vec<String> {
        self.run("profile.list", json!({}))["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .map(|profile| profile["name"].as_str().unwrap().to_owned())
            .collect()
    }
}

/// A profile as a command answers with it, from what was saved: every
/// field, the ones that were not given as null.
fn answered(profile: &Value) -> Value {
    let mut all = json!({
        "name": null, "user_name": null, "user_email": null,
        "signing_key": null, "signing_format": null, "ssh_key": null,
    });
    for (key, value) in profile.as_object().unwrap() {
        all[key] = value.clone();
    }
    all
}

/// Everything using a profile in a repository may change, and what it
/// may not: the repository's own settings, and the references.
fn state(copy: &Scratch) -> Vec<String> {
    let git = copy.git();
    let mut out = git
        .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
        .unwrap();
    out.push(format!("HEAD {}", git.rev_parse("HEAD").unwrap()));
    out.extend(settings(&git, &["--local"]));
    out
}

/// The settings of one scope or file, in an order of their own: what is
/// set counts, not where in the file it stands.
fn settings(git: &Git, scope: &[&str]) -> Vec<String> {
    let mut args = vec!["config"];
    args.extend(scope);
    args.extend(["--list", "-z"]);
    let mut all: Vec<String> = git
        .run(args)
        .unwrap_or_default()
        .split('\0')
        .filter(|entry| !entry.is_empty())
        .map(|entry| format!("config {entry}"))
        .collect();
    all.sort();
    all
}

/// One setting written with git itself, or taken away.
fn by_hand(git: &Git, scope: &[&str], key: &str, value: Option<&str>) {
    let mut args = vec!["config"];
    args.extend(scope);
    match value {
        Some(value) => {
            args.extend([key, value]);
            git.run(args).unwrap();
        }
        // Git ends with 5 when there is nothing to take away.
        None => {
            args.extend(["--unset-all", key]);
            let _ = git.run(args);
        }
    }
}

/// The SSH command for a key file, as the test writes it. Whether the
/// quotes hold is for the shell to say, further down.
fn ssh_command(key: &str) -> String {
    format!(
        "ssh -i '{}' -o IdentitiesOnly=yes",
        key.replace('\'', "'\\''")
    )
}

#[test]
fn profiles_are_saved_listed_renamed_and_deleted_in_the_file() {
    let mut h = Harness::new();
    // Nothing yet, and asking makes nothing.
    assert_eq!(h.run("profile.list", json!({})), json!({ "profiles": [] }));
    assert!(!h.dir.join("config").exists());

    let work =
        json!({ "name": "Work", "user_name": "Ada Lovelace", "user_email": "ada@work.invalid" });
    let out = h.save(work.clone());
    assert_eq!(
        out,
        json!({ "profile": answered(&work), "created": true, "renamed_from": null })
    );
    assert_eq!(h.on_disk(), json!({ "profiles": [work] }));

    let key = h.dir.text("keys/id_home");
    let home = json!({
        "name": "Home",
        "user_name": "Ada \"the Countess\" Ł. O'Neil",
        "user_email": "ada@home.invalid",
        "signing_key": "0xABCD1234",
        "signing_format": "openpgp",
        "ssh_key": key,
    });
    assert_eq!(h.save(home.clone())["created"], true);
    assert_eq!(h.on_disk(), json!({ "profiles": [work, home] }));
    assert_eq!(
        h.run("profile.list", json!({})),
        json!({ "profiles": [answered(&work), answered(&home)] })
    );
    // Written beside the file and moved into its place: nothing is left.
    let left: Vec<String> = std::fs::read_dir(h.dir.join("config"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(left, ["profiles.toml"]);

    // The same name again, in whatever case, is the same profile: it is
    // replaced where it stands and takes the name as it is written now.
    let work = json!({
        "name": "WORK",
        "user_name": "A. Lovelace",
        "user_email": "a.lovelace@work.invalid",
        "signing_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIexample ada@work",
        "signing_format": "ssh",
    });
    let out = h.save(work.clone());
    assert_eq!(out["created"], false);
    assert_eq!(out["profile"], answered(&work));
    assert_eq!(h.on_disk(), json!({ "profiles": [work, home] }));

    // What is typed with blanks around it is kept without them; a key
    // without a format is a GPG key; blank fields are fields left empty.
    let out = h.save(json!({
        "name": "  Third ", "user_name": " Third Person ", "user_email": " third@example.invalid ",
        "signing_key": " FEED ", "signing_format": null, "ssh_key": "   ",
    }));
    let third = json!({
        "name": "Third", "user_name": "Third Person", "user_email": "third@example.invalid",
        "signing_key": "FEED", "signing_format": "openpgp",
    });
    assert_eq!(out["profile"], answered(&third));
    assert_eq!(h.on_disk(), json!({ "profiles": [work, home, third] }));
    let out = h.save(json!({
        "name": "third", "user_name": "Third Person", "user_email": "third@example.invalid",
        "signing_key": "", "signing_format": "x509",
    }));
    let third = json!({ "name": "third", "user_name": "Third Person", "user_email": "third@example.invalid" });
    assert_eq!(out["profile"], answered(&third));
    assert_eq!(h.on_disk(), json!({ "profiles": [work, home, third] }));

    // Renamed, it stays where it is and is found by either case of its
    // old name.
    let office = json!({ "name": "Office", "user_name": "A. Lovelace", "user_email": "a.lovelace@work.invalid" });
    let out = h.run(
        "profile.save",
        json!({ "profile": office, "rename_from": "work" }),
    );
    assert_eq!(
        out,
        json!({ "profile": answered(&office), "created": false, "renamed_from": "WORK" })
    );
    assert_eq!(h.names(), ["Office", "Home", "third"]);
    assert_eq!(h.on_disk(), json!({ "profiles": [office, home, third] }));
    // Its own name in another case is a new name for it.
    let shouted = json!({ "name": "OFFICE", "user_name": "A. Lovelace", "user_email": "a.lovelace@work.invalid" });
    h.run(
        "profile.save",
        json!({ "profile": shouted, "rename_from": "Office" }),
    );
    assert_eq!(h.names(), ["OFFICE", "Home", "third"]);

    // Not onto the name of another one, and not from a name nobody has.
    let before = std::fs::read(h.file()).unwrap();
    h.refused(
        "profile.save",
        json!({ "profile": { "name": "hOmE", "user_name": "x", "user_email": "x@x" }, "rename_from": "office" }),
        "there is a profile \"hOmE\" already",
    );
    h.refused(
        "profile.save",
        json!({ "profile": { "name": "New", "user_name": "x", "user_email": "x@x" }, "rename_from": "Work" }),
        "there is no profile \"Work\" to rename",
    );
    h.refused(
        "profile.delete",
        json!({ "name": "Work" }),
        "there is no profile \"Work\"",
    );
    assert_eq!(std::fs::read(h.file()).unwrap(), before);

    // Deleted by its name in any case; the answer is what is gone.
    let out = h.run("profile.delete", json!({ "name": " HOME " }));
    assert_eq!(out, json!({ "deleted": answered(&home) }));
    assert_eq!(h.on_disk(), json!({ "profiles": [shouted, third] }));
    h.refused(
        "profile.delete",
        json!({ "name": "Home" }),
        "there is no profile",
    );
    h.run("profile.delete", json!({ "name": "office" }));
    h.run("profile.delete", json!({ "name": "third" }));
    assert_eq!(h.on_disk(), json!({ "profiles": [] }));
    assert_eq!(h.run("profile.list", json!({})), json!({ "profiles": [] }));
}

#[test]
fn text_of_every_kind_comes_back_from_the_file_as_it_went_in() {
    let mut h = Harness::new();
    let profiles = [
        json!({ "name": "quotes \" ' ` and \\ back", "user_name": "O'Neil \"Q\" \\n", "user_email": "o'neil+tag@example.invalid" }),
        json!({ "name": "ünï 人 🦑", "user_name": "Ådä Łövelace 人", "user_email": "ådä@exämple.invalid" }),
        json!({ "name": "# not a comment = [table]", "user_name": "[[profiles]]", "user_email": "name=\"x\"@example.invalid",
                "signing_key": "key = \"\"\" ''' #", "signing_format": "x509" }),
        json!({ "name": "x".repeat(64), "user_name": "n".repeat(256), "user_email": "e".repeat(320) }),
    ];
    for profile in &profiles {
        assert_eq!(h.save(profile.clone())["profile"], answered(profile));
    }
    assert_eq!(h.on_disk(), json!({ "profiles": profiles }));
    let listed = h.run("profile.list", json!({}));
    let expected: Vec<Value> = profiles.iter().map(answered).collect();
    assert_eq!(listed["profiles"], json!(expected));
}

#[test]
fn using_a_profile_in_a_repository_matches_git_config_on_a_twin() {
    let (mut h, ours, twin) = Harness::with_repository();
    let local = ["--local"];
    // An identity nobody has a profile for, to begin with.
    for copy in [&ours, &twin] {
        by_hand(&copy.git(), &local, "user.name", Some("Nobody Known"));
        by_hand(
            &copy.git(),
            &local,
            "user.email",
            Some("nobody@example.invalid"),
        );
    }
    let key = h.dir.key("id_work");
    let work = json!({
        "name": "Work", "user_name": "Ada Lovelace", "user_email": "ada@work.invalid",
        "signing_key": "0xABCD1234", "signing_format": "openpgp", "ssh_key": key,
    });
    let home = json!({ "name": "Home", "user_name": "Ada L.", "user_email": "ada@home.invalid" });
    let signer = json!({
        "name": "Signer", "user_name": "Ada Lovelace", "user_email": "ada@sign.invalid",
        "signing_key": "/keys/id_sign.pub", "signing_format": "ssh",
    });
    for profile in [&work, &home, &signer] {
        h.save(profile.clone());
    }
    assert_eq!(state(&ours), state(&twin), "before");
    // A copy that no profile is ever used in: what undo has to come to.
    let untouched = state(&ours);
    let current = h.run("profile.current", json!({}));
    assert_eq!(current["scope"], "effective");
    assert_eq!(current["identity"]["user_name"], "Nobody Known");
    assert_eq!(current["identity"]["user_email"], "nobody@example.invalid");
    assert!(current["profile"].is_null() && current["problem"].is_null());

    // A profile with both keys.
    let out = h.run("profile.use", json!({ "name": "work", "scope": "repo" }));
    assert_eq!(out["profile"], "Work");
    assert_eq!(out["scope"], "repo");
    assert_eq!(
        out["before"],
        json!({
            "user_name": "Nobody Known", "user_email": "nobody@example.invalid",
            "signing_key": null, "signing_format": null, "ssh_command": null, "ssh_key": null,
        })
    );
    let as_work = json!({
        "user_name": "Ada Lovelace", "user_email": "ada@work.invalid",
        "signing_key": "0xABCD1234", "signing_format": "openpgp",
        "ssh_command": ssh_command(&key), "ssh_key": key,
    });
    assert_eq!(out["after"], as_work);
    let git = twin.git();
    by_hand(&git, &local, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &local, "user.email", Some("ada@work.invalid"));
    by_hand(&git, &local, "user.signingkey", Some("0xABCD1234"));
    by_hand(&git, &local, "gpg.format", Some("openpgp"));
    by_hand(&git, &local, "core.sshCommand", Some(&ssh_command(&key)));
    assert_eq!(state(&ours), state(&twin), "a profile with keys");
    // What the repository says is what is in effect in it.
    let current = h.run("profile.current", json!({}));
    assert_eq!(current["profile"], "Work");
    assert_eq!(current["identity"], as_work);
    assert_eq!(
        h.run("profile.current", json!({ "scope": "repo" }))["identity"],
        as_work
    );

    // To a profile without keys: no key is left behind.
    let out = h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
    assert_eq!(out["before"], as_work);
    let as_home = json!({
        "user_name": "Ada L.", "user_email": "ada@home.invalid",
        "signing_key": null, "signing_format": null, "ssh_command": null, "ssh_key": null,
    });
    assert_eq!(out["after"], as_home);
    by_hand(&git, &local, "user.name", Some("Ada L."));
    by_hand(&git, &local, "user.email", Some("ada@home.invalid"));
    by_hand(&git, &local, "user.signingkey", None);
    by_hand(&git, &local, "gpg.format", None);
    by_hand(&git, &local, "core.sshCommand", None);
    assert_eq!(state(&ours), state(&twin), "a profile without keys");
    let own = h.run("profile.current", json!({ "scope": "repo" }));
    assert_eq!(own["scope"], "repo");
    assert_eq!(own["identity"], as_home);
    assert_eq!(own["profile"], "Home");
    assert_eq!(h.run("profile.current", json!({}))["profile"], "Home");
    assert!(
        settings(&ours.git(), &local)
            .iter()
            .all(|entry| !entry.contains("signingkey")
                && !entry.contains("gpg.")
                && !entry.contains("sshcommand")),
    );

    // A signing key that is an SSH key, and no key file to connect with.
    let out = h.run("profile.use", json!({ "name": "Signer", "scope": "repo" }));
    assert_eq!(out["after"]["signing_format"], "ssh");
    by_hand(&git, &local, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &local, "user.email", Some("ada@sign.invalid"));
    by_hand(&git, &local, "user.signingkey", Some("/keys/id_sign.pub"));
    by_hand(&git, &local, "gpg.format", Some("ssh"));
    assert_eq!(state(&ours), state(&twin), "an SSH signing key");
    assert_eq!(h.run("profile.current", json!({}))["profile"], "Signer");

    // An SSH command the user wrote stays when the profile has no key
    // file, however much it looks like one of ours.
    for theirs in [
        "ssh -i /somewhere/else -v".to_owned(),
        format!("{} -v", ssh_command(&key)),
        format!("ssh -i {key} -o IdentitiesOnly=yes"),
        format!("ssh -i \"{key}\" -o IdentitiesOnly=yes"),
        format!("/usr/bin/{}", ssh_command(&key)),
    ] {
        for copy in [&ours, &twin] {
            by_hand(&copy.git(), &local, "core.sshCommand", Some(&theirs));
        }
        let out = h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
        assert_eq!(out["after"]["ssh_command"], theirs.as_str());
        assert!(out["after"]["ssh_key"].is_null());
        by_hand(&git, &local, "user.name", Some("Ada L."));
        by_hand(&git, &local, "user.email", Some("ada@home.invalid"));
        by_hand(&git, &local, "user.signingkey", None);
        by_hand(&git, &local, "gpg.format", None);
        assert_eq!(state(&ours), state(&twin), "{theirs} is the user's own");
    }
    // A profile with a key file takes its place, and that one is ours
    // to take away again.
    h.run("profile.use", json!({ "name": "Work", "scope": "repo" }));
    by_hand(&git, &local, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &local, "user.email", Some("ada@work.invalid"));
    by_hand(&git, &local, "user.signingkey", Some("0xABCD1234"));
    by_hand(&git, &local, "gpg.format", Some("openpgp"));
    by_hand(&git, &local, "core.sshCommand", Some(&ssh_command(&key)));
    assert_eq!(state(&ours), state(&twin), "over the user's command");
    h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
    by_hand(&git, &local, "user.name", Some("Ada L."));
    by_hand(&git, &local, "user.email", Some("ada@home.invalid"));
    by_hand(&git, &local, "user.signingkey", None);
    by_hand(&git, &local, "gpg.format", None);
    by_hand(&git, &local, "core.sshCommand", None);
    assert_eq!(state(&ours), state(&twin), "and away again");

    // The same profile twice changes nothing the second time.
    let again = h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
    assert_eq!(again["before"], again["after"]);
    assert_eq!(state(&ours), state(&twin), "twice");

    // Using a profile in a repository is an action like any other:
    // the one that changed nothing left no entry, the last
    // that did is undone to what the twin had before it, and redone to
    // what it has.
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Use profile Home");
    let as_home = state(&ours);
    h.run("undo.last", json!({}));
    by_hand(&git, &local, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &local, "user.email", Some("ada@work.invalid"));
    by_hand(&git, &local, "user.signingkey", Some("0xABCD1234"));
    by_hand(&git, &local, "gpg.format", Some("openpgp"));
    by_hand(&git, &local, "core.sshCommand", Some(&ssh_command(&key)));
    assert_eq!(state(&ours), state(&twin), "undone to Work, keys and all");
    assert_eq!(h.run("profile.current", json!({}))["profile"], "Work");
    h.run("redo.last", json!({}));
    assert_eq!(state(&ours), as_home, "redone");
    // Further back, the uses that had an SSH command written by hand
    // between them: undo goes as far as the settings are still what a
    // use left them as, and where the user wrote over one it stops,
    // says so and changes nothing.
    let mut refusal = None;
    for _ in 0..50 {
        if h.run("undo.peek", json!({}))["undo"].is_null() {
            break;
        }
        let before = state(&ours);
        if let Err(err) = h.try_run("undo.last", json!({})) {
            assert_eq!(state(&ours), before, "a refused undo changes nothing");
            refusal = Some(err);
            break;
        }
    }
    let refusal = refusal.expect("undo went past a setting the user wrote");
    assert!(
        refusal.contains("core.sshcommand is not what this action left it as"),
        "{refusal}"
    );
    assert_ne!(state(&ours), untouched);
}

/// The repository after a profile was used and the use undone is the one
/// no profile was ever used in, and a file that is named is not the
/// repository's to take back.
#[test]
fn using_a_profile_is_undone_to_the_repository_it_was_never_used_in() {
    let (mut h, ours, twin) = Harness::with_repository();
    let local = ["--local"];
    for copy in [&ours, &twin] {
        by_hand(&copy.git(), &local, "user.name", Some("Nobody Known"));
        by_hand(&copy.git(), &local, "user.signingkey", Some("OLDKEY"));
        by_hand(&copy.git(), &local, "core.sshCommand", Some("ssh -v"));
    }
    let key = h.dir.key("id_work");
    h.save(json!({
        "name": "Work", "user_name": "Ada Lovelace", "user_email": "ada@work.invalid",
        "signing_key": "0xABCD1234", "signing_format": "openpgp", "ssh_key": key,
    }));
    h.save(json!({ "name": "Home", "user_name": "Ada L.", "user_email": "ada@home.invalid" }));
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    h.run("profile.use", json!({ "name": "Work", "scope": "repo" }));
    h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
    assert_ne!(state(&ours), state(&twin));
    let used = state(&ours);
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Use profile Home");
    h.run("undo.last", json!({}));
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Use profile Work");
    h.run("undo.last", json!({}));
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
    assert_eq!(state(&ours), state(&twin), "the twin never used a profile");
    h.run("redo.last", json!({}));
    h.run("redo.last", json!({}));
    assert_eq!(state(&ours), used);

    // Written to a file that is named: nothing of the repository's
    // changes, and nothing is journalled for it.
    let peek = h.run("undo.peek", json!({}));
    let file = h.dir.join("a.gitconfig");
    h.run(
        "profile.use",
        json!({ "name": "Work", "scope": { "file": file } }),
    );
    assert!(file.is_file());
    assert_eq!(state(&ours), used);
    assert_eq!(h.run("undo.peek", json!({})), peek);
}

#[test]
fn in_a_linked_worktree_the_repositorys_configuration_is_the_shared_one() {
    let (mut h, ours, twin) = Harness::with_repository();
    h.save(json!({
        "name": "Work", "user_name": "Ada Lovelace", "user_email": "ada@work.invalid",
        "signing_key": "ABCD",
    }));
    let (linked, twin_linked) = (h.dir.text("linked"), h.dir.text("twin-linked"));
    for (copy, path) in [(&ours, &linked), (&twin, &twin_linked)] {
        copy.git()
            .run(["worktree", "add", "-q", "--detach", path.as_str()])
            .unwrap();
    }
    h.run("repo.open", json!({ "path": linked }));
    h.run("profile.use", json!({ "name": "Work", "scope": "repo" }));
    let git = Git::new(&twin_linked, fixtures_out_dir().join(".home"));
    let local = ["--local"];
    by_hand(&git, &local, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &local, "user.email", Some("ada@work.invalid"));
    by_hand(&git, &local, "user.signingkey", Some("ABCD"));
    by_hand(&git, &local, "gpg.format", Some("openpgp"));
    // What the main worktree says is what was written from the linked one.
    assert_eq!(state(&ours), state(&twin));
    assert!(
        settings(&ours.git(), &local).contains(&"config user.email\nada@work.invalid".to_owned())
    );
    let current = h.run("profile.current", json!({}));
    assert_eq!(current["profile"], "Work");
    assert_eq!(current["identity"]["signing_format"], "openpgp");
}

#[test]
fn settings_that_are_not_as_expected_are_read_and_put_right() {
    let (mut h, ours, twin) = Harness::with_repository();
    h.save(json!({ "name": "Home", "user_name": "Ada L.", "user_email": "ada@home.invalid" }));
    // Two names, an address and a key that are set to nothing, a format
    // git does not know, a command of several lines that is nearly ours,
    // a value too long to be anything, an include that leads nowhere.
    let command = "ssh -i '/a'\n\t-o IdentitiesOnly=yes";
    let long = "x".repeat(3000);
    for copy in [&ours, &twin] {
        let git = copy.git();
        for (key, value) in [
            ("user.name", "One"),
            ("user.name", long.as_str()),
            ("user.name", "Two"),
            ("user.email", ""),
            ("user.signingkey", ""),
            ("gpg.format", "nonsense"),
            ("core.sshCommand", command),
            ("include.path", "/nowhere/at/all.gitconfig"),
        ] {
            git.run(["config", "--local", "--add", key, value]).unwrap();
        }
    }
    let own = h.run("profile.current", json!({ "scope": "repo" }));
    assert_eq!(
        own["identity"],
        json!({
            "user_name": "Two", "user_email": "", "signing_key": "",
            "signing_format": "nonsense", "ssh_command": command, "ssh_key": null,
        })
    );
    assert!(own["profile"].is_null());
    let current = h.run("profile.current", json!({}));
    assert_eq!(current["identity"]["user_name"], "Two");
    assert!(current["profile"].is_null());

    // One name is left of the three, the empty key and the unknown
    // format are gone, and the command, which is not ours, stays.
    let out = h.run("profile.use", json!({ "name": "Home", "scope": "repo" }));
    assert_eq!(out["before"], own["identity"]);
    assert_eq!(
        out["after"],
        json!({
            "user_name": "Ada L.", "user_email": "ada@home.invalid", "signing_key": null,
            "signing_format": null, "ssh_command": command, "ssh_key": null,
        })
    );
    let git = twin.git();
    for args in [
        vec!["--replace-all", "user.name", "Ada L."],
        vec!["--replace-all", "user.email", "ada@home.invalid"],
        vec!["--unset-all", "user.signingkey"],
        vec!["--unset-all", "gpg.format"],
    ] {
        let mut all = vec!["config", "--local"];
        all.extend(args);
        git.run(all).unwrap();
    }
    assert_eq!(state(&ours), state(&twin));
    assert_eq!(h.run("profile.current", json!({}))["profile"], "Home");
}

#[test]
fn a_named_file_stands_in_for_the_users_configuration() {
    let mut h = Harness::new();
    let key = h.dir.key("id_work");
    h.save(json!({
        "name": "Work", "user_name": "Ada Lovelace", "user_email": "ada@work.invalid",
        "signing_key": "/keys/id_sign.pub", "signing_format": "ssh", "ssh_key": key,
    }));
    h.save(json!({ "name": "Home", "user_name": "Ada L.", "user_email": "ada@home.invalid" }));
    let (ours, twin) = (h.dir.text("ours.gitconfig"), h.dir.text("twin.gitconfig"));
    let git = h.dir.git();
    let scope = json!({ "file": ours });
    let (in_ours, in_twin) = (["--file", ours.as_str()], ["--file", twin.as_str()]);

    // A file that is not there has no identity, and asking makes none.
    let current = h.run("profile.current", json!({ "scope": scope }));
    assert_eq!(current["scope"], scope);
    assert_eq!(
        current["identity"],
        json!({
            "user_name": null, "user_email": null, "signing_key": null,
            "signing_format": null, "ssh_command": null, "ssh_key": null,
        })
    );
    assert!(current["profile"].is_null());
    assert!(!Path::new(&ours).exists());

    // What else is in the file stays as it is.
    let theirs =
        "# mine\n[alias]\n\tco = checkout\n[user]\n\tname = Somebody Else\n[core]\n\teditor = vi\n";
    std::fs::write(&ours, theirs).unwrap();
    std::fs::write(&twin, theirs).unwrap();

    let out = h.run("profile.use", json!({ "name": "Work", "scope": scope }));
    assert_eq!(out["scope"], scope);
    assert_eq!(out["before"]["user_name"], "Somebody Else");
    assert!(out["before"]["user_email"].is_null());
    by_hand(&git, &in_twin, "user.name", Some("Ada Lovelace"));
    by_hand(&git, &in_twin, "user.email", Some("ada@work.invalid"));
    by_hand(&git, &in_twin, "user.signingkey", Some("/keys/id_sign.pub"));
    by_hand(&git, &in_twin, "gpg.format", Some("ssh"));
    by_hand(&git, &in_twin, "core.sshCommand", Some(&ssh_command(&key)));
    assert_eq!(settings(&git, &in_ours), settings(&git, &in_twin));
    assert_eq!(settings(&git, &in_ours).len(), 7);
    assert!(
        std::fs::read_to_string(&ours)
            .unwrap()
            .starts_with("# mine\n[alias]")
    );
    let current = h.run("profile.current", json!({ "scope": scope }));
    assert_eq!(current["profile"], "Work");
    assert_eq!(current["identity"], out["after"]);
    assert_eq!(current["identity"]["ssh_key"], key.as_str());

    // To the profile without keys, and none is left in the file.
    let out = h.run("profile.use", json!({ "name": "Home", "scope": scope }));
    assert!(out["after"]["signing_key"].is_null() && out["after"]["ssh_command"].is_null());
    by_hand(&git, &in_twin, "user.name", Some("Ada L."));
    by_hand(&git, &in_twin, "user.email", Some("ada@home.invalid"));
    by_hand(&git, &in_twin, "user.signingkey", None);
    by_hand(&git, &in_twin, "gpg.format", None);
    by_hand(&git, &in_twin, "core.sshCommand", None);
    assert_eq!(settings(&git, &in_ours), settings(&git, &in_twin));
    assert_eq!(
        settings(&git, &in_ours),
        [
            "config alias.co\ncheckout",
            "config core.editor\nvi",
            "config user.email\nada@home.invalid",
            "config user.name\nAda L.",
        ]
    );
    assert_eq!(
        h.run("profile.current", json!({ "scope": scope }))["profile"],
        "Home"
    );

    // The user's own SSH command stays.
    by_hand(
        &git,
        &in_ours,
        "core.sshCommand",
        Some("ssh -o ControlMaster=auto"),
    );
    let out = h.run("profile.use", json!({ "name": "Home", "scope": scope }));
    assert_eq!(out["after"]["ssh_command"], "ssh -o ControlMaster=auto");
    assert_eq!(out["before"], out["after"]);

    // What the file includes counts when the identity is asked for, and
    // is not what a profile is written to.
    let included = h.dir.text("included.gitconfig");
    by_hand(
        &git,
        &["--file", included.as_str()],
        "user.signingkey",
        Some("FROM-ELSEWHERE"),
    );
    by_hand(&git, &in_ours, "include.path", Some(&included));
    let current = h.run("profile.current", json!({ "scope": scope }));
    assert_eq!(current["identity"]["signing_key"], "FROM-ELSEWHERE");
    let before = std::fs::read(&included).unwrap();
    let out = h.run("profile.use", json!({ "name": "Home", "scope": scope }));
    assert!(
        out["after"]["signing_key"].is_null(),
        "the file itself has none"
    );
    assert_eq!(std::fs::read(&included).unwrap(), before);

    // A file that does not exist yet is made, in a directory that is there.
    let fresh = h.dir.text("fresh.gitconfig");
    h.run(
        "profile.use",
        json!({ "name": "Work", "scope": { "file": fresh } }),
    );
    assert_eq!(settings(&git, &["--file", fresh.as_str()]).len(), 5);

    // The repository's configuration needs a repository.
    h.refused(
        "profile.use",
        json!({ "name": "Home", "scope": "repo" }),
        "no repository is open",
    );
    h.refused(
        "profile.current",
        json!({ "scope": "repo" }),
        "no repository is open",
    );
}

/// A program called `ssh` that writes down what it was started with, one
/// argument a line, and gives up.
#[cfg(unix)]
fn fake_ssh(dir: &Dir) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let bin = dir.join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let script = bin.join("ssh");
    std::fs::write(
        &script,
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$FAKE_SSH_SAID\"\nexit 1\n",
    )
    .unwrap();
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    bin
}

#[cfg(unix)]
fn path_with(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(unix)]
#[test]
fn a_key_path_of_any_kind_reaches_ssh_as_one_argument() {
    let (mut h, ours, _twin) = Harness::with_repository();
    let bin = fake_ssh(&h.dir);
    let said = h.dir.join("said");
    // Where a command that got loose would leave its mark.
    let loose = h.dir.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    let config = h.dir.text("ssh.gitconfig");

    for name in [
        "id_plain",
        "id work",
        "  blanks around  ",
        "it's",
        "a'b''c'",
        "'",
        "dou\"ble",
        "dollar$HOME",
        "$(touch pwned)",
        "`touch pwned`",
        "semi;touch pwned",
        "and&&touch pwned",
        "pipe|touch pwned",
        "redirect>pwned",
        "back\\slash\\",
        "star*?[a-z]",
        "#hash",
        "~tilde",
        "-oProxyCommand=touch pwned",
        "ünï ключ 鍵",
        "{brace}$",
    ] {
        let key = h.dir.key(name);
        h.save(json!({ "name": "Key", "user_name": "Ada", "user_email": "ada@example.invalid", "ssh_key": key }));
        h.run(
            "profile.use",
            json!({ "name": "Key", "scope": { "file": config } }),
        );
        let current = h.run("profile.current", json!({ "scope": { "file": config } }));
        assert_eq!(current["identity"]["ssh_key"], key.as_str(), "known again");

        // The configured command as the shell reads it, the way git
        // hands it over: with the host and what to run there after it.
        let command = h
            .dir
            .git()
            .run([
                "config",
                "--file",
                config.as_str(),
                "--get",
                "core.sshCommand",
            ])
            .unwrap();
        let _ = std::fs::remove_file(&said);
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} example.invalid git-upload-pack /x.git",
                command.trim_end_matches('\n')
            ))
            .current_dir(&loose)
            .env("PATH", path_with(&bin))
            .env("FAKE_SSH_SAID", &said)
            .status()
            .unwrap();
        assert_eq!(status.code(), Some(1), "{name}: the stand-in for ssh ran");
        let arguments = std::fs::read_to_string(&said).unwrap();
        assert_eq!(
            arguments.lines().collect::<Vec<_>>(),
            [
                "-i",
                key.as_str(),
                "-o",
                "IdentitiesOnly=yes",
                "example.invalid",
                "git-upload-pack",
                "/x.git"
            ],
            "{name}"
        );
        assert_eq!(std::fs::read_dir(&loose).unwrap().count(), 0, "{name}");
    }

    // And as git itself starts it, from the repository's configuration.
    for name in ["it's a $(touch pwned) `key`", "id_plain"] {
        let key = h.dir.key(name);
        h.save(json!({ "name": "Key", "user_name": "Ada", "user_email": "ada@example.invalid", "ssh_key": key }));
        h.run("profile.use", json!({ "name": "Key", "scope": "repo" }));
        let _ = std::fs::remove_file(&said);
        let mut git = ours.git().command();
        git.args(["ls-remote", "ssh://example.invalid/x.git"])
            .env("PATH", path_with(&bin))
            .env("FAKE_SSH_SAID", &said)
            .env_remove("GIT_SSH")
            .env_remove("GIT_SSH_COMMAND")
            .env_remove("GIT_SSH_VARIANT");
        assert!(!git.output().unwrap().status.success(), "nothing answers");
        let arguments = std::fs::read_to_string(&said).unwrap();
        let arguments: Vec<&str> = arguments.lines().collect();
        assert_eq!(
            arguments[..4],
            ["-i", key.as_str(), "-o", "IdentitiesOnly=yes"],
            "{name}"
        );
        assert!(arguments.contains(&"example.invalid"), "{arguments:?}");
        assert!(!ours.path().join("pwned").exists());
    }
}

#[test]
fn a_name_that_is_kept_is_a_name_git_writes_as_it_is() {
    let (mut h, ours, _twin) = Harness::with_repository();
    let git = ours.git();
    // Who git says the author is, for a name and an address: the line
    // it would write into a commit, or its refusal.
    let written = |name: &str, email: &str| {
        git.run_with(
            ["var", "GIT_AUTHOR_IDENT"],
            &[("GIT_AUTHOR_NAME", name), ("GIT_AUTHOR_EMAIL", email)],
        )
    };
    let profile = |name: &str, email: &str| json!({ "profile": { "name": "Checked", "user_name": name, "user_email": email } });
    for name in [
        "Ada Lovelace",
        "A",
        "Ådä Łövelace",
        "人",
        "O'Neil-Smith, Jr",
        "Ada \"the Countess\" Lovelace",
        "x.y:z;w",
        "a\\b",
    ] {
        for email in [
            "ada@example.invalid",
            "local",
            "a+b@x",
            "ådä@exämple.invalid",
        ] {
            h.run("profile.save", profile(name, email));
            let line = written(name, email).unwrap();
            assert!(
                line.starts_with(&format!("{name} <{email}> ")),
                "{name:?} {email:?} came out as {line:?}"
            );
        }
    }
    // What git refuses, or writes as something else, is not kept.
    for name in ["", " ", ".,:;\"'\\", "<>", "<Ada>", "Ada <ada@x>", "A > B"] {
        assert!(
            h.try_run("profile.save", profile(name, "ada@example.invalid"))
                .is_err(),
            "kept the name {name:?}"
        );
        if let Ok(line) = written(name, "ada@example.invalid") {
            assert!(
                !line.starts_with(&format!("{name} <ada@example.invalid> ")),
                "git writes {name:?} as it is: {line:?}"
            );
        }
    }
    // Nor is a name of periods alone: older gits take those off the ends
    // of a name, and nothing is left of such a one.
    assert!(
        h.try_run("profile.save", profile("...", "ada@example.invalid"))
            .is_err()
    );
    for email in ["<ada@x>", "ada@x>", "a<d>a@x"] {
        assert!(
            h.try_run("profile.save", profile("Ada", email)).is_err(),
            "kept the address {email:?}"
        );
        let line = written("Ada", email).unwrap();
        assert!(!line.starts_with(&format!("Ada <{email}> ")), "{line:?}");
    }
}

#[test]
fn a_profile_that_cannot_work_is_not_used_and_nothing_is_written() {
    let (mut h, ours, _twin) = Harness::with_repository();
    let key = h.dir.key("id_there");
    h.save(json!({ "name": "Keyed", "user_name": "Ada", "user_email": "ada@example.invalid", "ssh_key": key }));
    h.save(json!({ "name": "Plain", "user_name": "Ada", "user_email": "ada@example.invalid" }));
    let config = h.dir.text("a.gitconfig");
    std::fs::write(&config, "[core]\n\teditor = vi\n").unwrap();
    let before = (state(&ours), std::fs::read(&config).unwrap());

    // The key file is gone: ssh would pass over it and use other keys.
    std::fs::remove_file(&key).unwrap();
    for scope in [json!("repo"), json!({ "file": config })] {
        h.refused(
            "profile.use",
            json!({ "name": "Keyed", "scope": scope }),
            "there is no key file at",
        );
    }
    // A directory is no key file either.
    std::fs::create_dir_all(&key).unwrap();
    h.refused(
        "profile.use",
        json!({ "name": "Keyed", "scope": "repo" }),
        "there is no key file at",
    );
    assert_eq!((state(&ours), std::fs::read(&config).unwrap()), before);

    // Files that cannot be written as a configuration.
    let directory = h.dir.text("keys");
    let nowhere = h.dir.text("no/such/dir/x.gitconfig");
    let garbage = h.dir.text("garbage.gitconfig");
    std::fs::write(&garbage, b"\x00\x01[[[ not a configuration\n").unwrap();
    for (file, because) in [
        (directory.as_str(), "is not a file"),
        (nowhere.as_str(), "is not there"),
        (garbage.as_str(), "cannot be"),
        ("relative.gitconfig", "full path"),
        ("./relative.gitconfig", "full path"),
        ("", "must not be empty"),
        ("/line\nbreak", "control characters"),
        ("/nul\0", "control characters"),
    ] {
        h.refused(
            "profile.use",
            json!({ "name": "Plain", "scope": { "file": file } }),
            because,
        );
    }
    assert_eq!(
        std::fs::read(&garbage).unwrap(),
        b"\x00\x01[[[ not a configuration\n"
    );
    assert!(!Path::new(&nowhere).exists() && !Path::new("relative.gitconfig").exists());
    assert_eq!((state(&ours), std::fs::read(&config).unwrap()), before);
}

#[test]
fn without_a_configuration_directory_every_profile_command_refuses() {
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    let profile =
        json!({ "name": "Work", "user_name": "Ada", "user_email": "ada@example.invalid" });
    for (id, params) in [
        ("profile.list", json!({})),
        ("profile.save", json!({ "profile": profile })),
        ("profile.delete", json!({ "name": "Work" })),
        (
            "profile.current",
            json!({ "scope": { "file": "/nowhere/at/all" } }),
        ),
        (
            "profile.use",
            json!({ "name": "Work", "scope": { "file": "/nowhere/at/all" } }),
        ),
    ] {
        let err = registry.run(id, &mut ctx, &params).unwrap_err().to_string();
        assert!(err.contains("no configuration directory"), "{id}: {err}");
    }
}

#[test]
fn hostile_profile_parameters_are_refused_and_change_nothing() {
    let (mut h, ours, _twin) = Harness::with_repository();
    let key = h.dir.key("id_kept");
    h.save(json!({
        "name": "Kept", "user_name": "Ada", "user_email": "ada@example.invalid",
        "signing_key": "ABCD", "ssh_key": key,
    }));
    let config = h.dir.text("a.gitconfig");
    std::fs::write(&config, "[user]\n\tname = Kept As It Is\n").unwrap();
    let unchanged = |h: &Harness| {
        (
            std::fs::read(h.file()).unwrap(),
            state(&ours),
            std::fs::read(&config).unwrap(),
        )
    };
    let before = unchanged(&h);
    let long = "x".repeat(5000);
    let fine = json!({ "name": "Fine", "user_name": "Ada", "user_email": "ada@example.invalid" });
    let with = |field: &str, value: Value| {
        let mut profile = fine.clone();
        profile[field] = value;
        json!({ "profile": profile })
    };

    // What a profile is called.
    let long_name = "n".repeat(65);
    for name in [
        "",
        "   ",
        "\t\n",
        "two\nlines",
        "nul\0",
        "\u{1b}[2J",
        "\u{7}",
        long_name.as_str(),
        long.as_str(),
    ] {
        assert!(
            h.try_run("profile.save", with("name", json!(name)))
                .is_err(),
            "saved under the name {name:?}"
        );
        for (id, params) in [
            (
                "profile.save",
                json!({ "profile": fine, "rename_from": name }),
            ),
            ("profile.delete", json!({ "name": name })),
            ("profile.use", json!({ "name": name, "scope": "repo" })),
            (
                "profile.use",
                json!({ "name": name, "scope": { "file": config } }),
            ),
        ] {
            assert!(h.try_run(id, params.clone()).is_err(), "{id} {params}");
        }
    }
    // Names nobody has.
    for name in [
        "Kep",
        "Kept2",
        "-Kept",
        "--help",
        "../Kept",
        "Kept\u{200b}",
        "KEPT.",
    ] {
        for (id, params) in [
            ("profile.delete", json!({ "name": name })),
            ("profile.use", json!({ "name": name, "scope": "repo" })),
            (
                "profile.save",
                json!({ "profile": fine, "rename_from": name }),
            ),
        ] {
            let err = h.try_run(id, params.clone()).expect_err(name);
            assert!(err.contains("there is no profile"), "{id} {params}: {err}");
        }
    }
    // Who the work is by: what git would refuse, drop or change.
    for name in [
        "",
        " ",
        "...",
        "<>",
        "Ada <ada@example.invalid>",
        "A > B",
        "two\nlines",
        "tab\tin it",
        "nul\0",
        long.as_str(),
    ] {
        assert!(
            h.try_run("profile.save", with("user_name", json!(name)))
                .is_err(),
            "saved the user name {name:?}"
        );
    }
    for email in [
        "",
        "  ",
        "ada@x\nCommitter: Eve <eve@example.invalid>",
        "ada@\0x",
        "<ada@example.invalid>",
        "ada@example.invalid>",
        "ada @example.invalid",
        "ada@example.invalid extra",
        long.as_str(),
    ] {
        assert!(
            h.try_run("profile.save", with("user_email", json!(email)))
                .is_err(),
            "saved the address {email:?}"
        );
    }
    // Keys.
    for key in [
        "-u",
        "--default-key=x",
        "two\nlines",
        "nul\0",
        long.as_str(),
    ] {
        assert!(
            h.try_run("profile.save", with("signing_key", json!(key)))
                .is_err(),
            "saved the signing key {key:?}"
        );
    }
    for format in [
        json!("pgp"),
        json!("SSH"),
        json!("gpg"),
        json!(""),
        json!(3),
        json!(["ssh"]),
        json!({ "ssh": true }),
    ] {
        let mut params = with("signing_format", format.clone());
        params["profile"]["signing_key"] = json!("ABCD");
        assert!(
            h.try_run("profile.save", params).is_err(),
            "saved the format {format}"
        );
    }
    let long_path = format!("/{long}");
    for key in [
        "relative/key",
        "./key",
        "~/.ssh/id_ed25519",
        "-oProxyCommand=touch pwned",
        "/line\nbreak",
        "/carriage\rreturn",
        "/nul\0",
        "/escape\u{1b}[31m",
        "/ssh/fills/%d/in",
        "/ssh/fills/%%/in",
        "/ssh/fills/${HOME}/in",
        long_path.as_str(),
    ] {
        assert!(
            h.try_run("profile.save", with("ssh_key", json!(key)))
                .is_err(),
            "saved the key file {key:?}"
        );
    }
    // Parameters of the wrong shape.
    for (id, params) in [
        ("profile.save", json!({})),
        ("profile.save", json!({ "profile": null })),
        ("profile.save", json!({ "profile": "Fine" })),
        ("profile.save", json!({ "profile": [fine] })),
        ("profile.save", json!({ "profile": {} })),
        ("profile.save", json!({ "profile": { "name": "Fine" } })),
        ("profile.save", with("name", json!(3))),
        ("profile.save", with("user_name", json!(null))),
        ("profile.save", with("user_email", json!(["a@x"]))),
        ("profile.save", with("ssh_key", json!(false))),
        ("profile.save", with("colour", json!("red"))),
        ("profile.save", with("email", json!("typo@example.invalid"))),
        ("profile.save", json!({ "profile": fine, "rename_from": 3 })),
        (
            "profile.save",
            json!({ "profile": fine, "rename_from": ["Kept"] }),
        ),
        ("profile.delete", json!({})),
        ("profile.delete", json!({ "name": null })),
        ("profile.delete", json!({ "name": ["Kept"] })),
        ("profile.use", json!({})),
        ("profile.use", json!({ "name": "Kept" })),
        ("profile.use", json!({ "scope": "repo" })),
        ("profile.use", json!({ "name": 3, "scope": "repo" })),
        ("profile.use", json!({ "name": "Kept", "scope": null })),
        ("profile.use", json!({ "name": "Kept", "scope": "global" })),
        ("profile.use", json!({ "name": "Kept", "scope": "system" })),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": "--global" }),
        ),
        ("profile.use", json!({ "name": "Kept", "scope": "" })),
        ("profile.use", json!({ "name": "Kept", "scope": 3 })),
        ("profile.use", json!({ "name": "Kept", "scope": ["repo"] })),
        ("profile.use", json!({ "name": "Kept", "scope": {} })),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": { "file": 3 } }),
        ),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": { "file": null } }),
        ),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": { "path": config } }),
        ),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": { "file": config, "repo": true } }),
        ),
        (
            "profile.use",
            json!({ "name": "Kept", "scope": { "file": long_path } }),
        ),
        ("profile.current", json!({ "scope": "global" })),
        ("profile.current", json!({ "scope": 3 })),
        (
            "profile.current",
            json!({ "scope": { "file": "relative" } }),
        ),
        (
            "profile.current",
            json!({ "scope": { "file": "/line\nbreak" } }),
        ),
        (
            "profile.current",
            json!({ "scope": { "file": h.dir.text("keys") } }),
        ),
    ] {
        assert!(h.try_run(id, params.clone()).is_err(), "{id} {params}");
    }
    assert_eq!(unchanged(&h), before);
    assert_eq!(h.names(), ["Kept"]);
    assert!(!ours.path().join("pwned").exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

/// A file of `count` profiles, as this application would write it.
fn many(count: usize) -> String {
    (0..count)
        .map(|n| format!("[[profiles]]\nname = \"p{n}\"\nuser_name = \"Person {n}\"\nuser_email = \"p{n}@example.invalid\"\n\n"))
        .collect()
}

#[test]
fn a_profiles_file_that_is_not_as_expected_is_never_written_over() {
    let (mut h, ours, _twin) = Harness::with_repository();
    let config = h.dir.join("config");
    std::fs::create_dir_all(&config).unwrap();
    let file = h.file();
    let fine = json!({ "name": "Fine", "user_name": "Ada", "user_email": "ada@example.invalid" });
    let entry =
        "[[profiles]]\nname = \"a\"\nuser_name = \"A\"\nuser_email = \"a@example.invalid\"\n";
    let repository = state(&ours);

    let huge = format!("# {}\n{entry}", "padding ".repeat(200_000));
    let damaged: Vec<(&str, Vec<u8>, &str)> = vec![
        (
            "binary",
            vec![0, 159, 146, 150, 255, 254, 0, 1, 2],
            "is not text",
        ),
        (
            "binary that is text",
            b"\x00\x01\x02\x7f[[[\n".to_vec(),
            "cannot be understood",
        ),
        ("huge", huge.into_bytes(), "is larger than 1024 KiB"),
        (
            "not TOML",
            b"profiles: [name: a]\n".to_vec(),
            "cannot be understood (line 1",
        ),
        (
            "cut short",
            b"[[profiles]]\nname = \"a\"\nuser_name = \"unterminated\n".to_vec(),
            "cannot be understood (line 3",
        ),
        (
            "a string for the list",
            b"profiles = \"none\"\n".to_vec(),
            "cannot be understood",
        ),
        (
            "a table for the list",
            b"[profiles]\nname = \"a\"\n".to_vec(),
            "cannot be understood",
        ),
        (
            "a list of numbers",
            b"profiles = [1, 2, 3]\n".to_vec(),
            "cannot be understood",
        ),
        (
            "a number for a name",
            b"[[profiles]]\nname = 3\nuser_name = \"A\"\nuser_email = \"a@x\"\n".to_vec(),
            "cannot be understood (line 2",
        ),
        (
            "no address",
            b"[[profiles]]\nname = \"a\"\nuser_name = \"A\"\n".to_vec(),
            "cannot be understood",
        ),
        (
            "a field of a later version",
            format!("{entry}accounts = [\"x\"]\n").into_bytes(),
            "cannot be understood",
        ),
        (
            "a setting of a later version",
            format!("version = 2\n{entry}").into_bytes(),
            "cannot be understood",
        ),
        (
            "a format nobody knows",
            format!("{entry}signing_key = \"k\"\nsigning_format = \"pgp\"\n").into_bytes(),
            "cannot be understood",
        ),
        (
            "one name twice",
            format!("{entry}{}", entry.replace("\"a\"", "\"A\"")).into_bytes(),
            "has two profiles of one name (the second is number 2)",
        ),
        (
            "a name of control characters",
            entry
                .replace("\"a\"", "\"a\\u0000\\u001b[2J\"")
                .into_bytes(),
            "has a profile that cannot be used (number 1",
        ),
        (
            "an empty name",
            entry.replace("\"a\"", "\"  \"").into_bytes(),
            "has a profile that cannot be used (number 1",
        ),
        (
            "an address of two lines",
            entry.replace("a@example.invalid", "a@x\\nb").into_bytes(),
            "has a profile that cannot be used (number 1",
        ),
        (
            "a key file ssh would fill in",
            format!("{entry}ssh_key = \"/keys/%d/id\"\n").into_bytes(),
            "has a profile that cannot be used (number 1",
        ),
        (
            "ten thousand profiles",
            many(10_000).into_bytes(),
            "has 10000 profiles, and no more than 256 are kept",
        ),
        (
            "one profile too many",
            many(257).into_bytes(),
            "has 257 profiles",
        ),
    ];
    for (what, bytes, because) in &damaged {
        std::fs::write(&file, bytes).unwrap();
        for (id, params) in [
            ("profile.list", json!({})),
            ("profile.save", json!({ "profile": fine })),
            (
                "profile.save",
                json!({ "profile": fine, "rename_from": "a" }),
            ),
            ("profile.delete", json!({ "name": "a" })),
            ("profile.use", json!({ "name": "a", "scope": "repo" })),
        ] {
            let err = h
                .try_run(id, params)
                .expect_err(&format!("{id} took a file that is {what}"));
            assert!(err.contains(because), "{what}, {id}: {err}");
            assert!(
                err.contains("profiles.toml") && err.len() < 1000,
                "{what}: {err}"
            );
        }
        // The identity is still told, with what is wrong beside it.
        let current = h.run("profile.current", json!({ "scope": "repo" }));
        assert!(current["profile"].is_null(), "{what}");
        assert!(
            current["problem"]
                .as_str()
                .is_some_and(|said| said.contains(because)),
            "{what}: {current}"
        );
        assert_eq!(
            &std::fs::read(&file).unwrap(),
            bytes,
            "{what} was written over"
        );
        assert_eq!(state(&ours), repository, "{what}");
    }

    // Something that is no file at all: a directory, a pipe, a link that
    // leads to a directory.
    std::fs::remove_file(&file).unwrap();
    std::fs::create_dir_all(file.join("inside")).unwrap();
    let mut others = vec!["a directory"];
    #[cfg(unix)]
    {
        let made = |program: &str, arguments: &[&std::ffi::OsStr]| {
            std::process::Command::new(program)
                .args(arguments)
                .status()
                .is_ok_and(|status| status.success())
        };
        let pipe = config.join("pipe");
        if made("mkfifo", &[pipe.as_os_str()]) {
            others.push("a pipe");
        }
    }
    others.push("a link to a directory");
    for what in others {
        match what {
            "a pipe" => {
                std::fs::remove_dir_all(&file).unwrap();
                std::fs::rename(config.join("pipe"), &file).unwrap();
            }
            "a link to a directory" => {
                let _ = std::fs::remove_dir_all(&file);
                let _ = std::fs::remove_file(&file);
                std::fs::create_dir_all(h.dir.join("keys")).unwrap();
                bitbraid_testkit::symlink_dir(h.dir.join("keys"), &file).unwrap();
            }
            _ => {}
        }
        for (id, params) in [
            ("profile.list", json!({})),
            ("profile.save", json!({ "profile": fine })),
            ("profile.delete", json!({ "name": "a" })),
            ("profile.use", json!({ "name": "a", "scope": "repo" })),
        ] {
            let err = h
                .try_run(id, params)
                .expect_err(&format!("{id} took {what} for the file"));
            assert!(err.contains("is not a file"), "{what}, {id}: {err}");
        }
        assert!(
            h.run("profile.current", json!({ "scope": "repo" }))["problem"]
                .as_str()
                .is_some_and(|said| said.contains("is not a file"))
        );
        assert!(
            std::fs::symlink_metadata(&file).is_ok_and(|meta| !meta.is_file()),
            "{what}"
        );
        assert_eq!(state(&ours), repository);
    }
    let _ = std::fs::remove_dir_all(&file);
    let _ = std::fs::remove_file(&file);

    // What is merely unusual is read: nothing in the file, comments
    // only, an empty list, the list written on one line.
    for (text, names) in [
        ("", vec![]),
        ("\n\n# nothing yet\n", vec![]),
        ("profiles = []\n", vec![]),
        (
            "profiles = [{ name = \"inline\", user_name = \"A\", user_email = \"a@x\" }]\n",
            vec!["inline"],
        ),
        // Written by hand, a little untidily: it is tidied when read.
        (
            "[[profiles]]\nuser_email = \" a@x \"\nname = \" by hand \"\nuser_name = \"A\"\nsigning_key = \"K\"\nssh_key = \"\"\n",
            vec!["by hand"],
        ),
    ] {
        std::fs::write(&file, text).unwrap();
        assert_eq!(h.names(), names, "{text:?}");
    }
    let listed = h.run("profile.list", json!({}));
    assert_eq!(
        listed["profiles"][0],
        answered(&json!({
            "name": "by hand", "user_name": "A", "user_email": "a@x",
            "signing_key": "K", "signing_format": "openpgp",
        }))
    );

    // As many as are kept: read, changed, and not added to.
    std::fs::write(&file, many(256)).unwrap();
    assert_eq!(h.names().len(), 256);
    h.refused(
        "profile.save",
        json!({ "profile": fine }),
        "there are 256 profiles already",
    );
    let replaced =
        json!({ "name": "P255", "user_name": "Ada", "user_email": "ada@example.invalid" });
    assert_eq!(h.save(replaced)["created"], false);
    h.run("profile.delete", json!({ "name": "p0" }));
    assert_eq!(h.save(fine.clone())["created"], true);
    let names = h.names();
    assert_eq!(names.len(), 256);
    assert_eq!(
        (names[0].as_str(), names[254].as_str(), names[255].as_str()),
        ("p1", "P255", "Fine")
    );

    // A link that leads nowhere is no file yet; saving puts a file there.
    std::fs::remove_file(&file).unwrap();
    bitbraid_testkit::symlink_file(h.dir.join("nowhere"), &file).unwrap();
    assert!(h.names().is_empty());
    h.save(fine.clone());
    assert_eq!(h.names(), ["Fine"]);
    assert!(std::fs::symlink_metadata(&file).unwrap().is_file());
    assert!(!h.dir.join("nowhere").exists());
    assert_eq!(state(&ours), repository);
}

#[test]
fn a_configuration_directory_that_cannot_be_one_is_a_refusal() {
    let dir = Dir::new();
    // A file where the directory should be.
    std::fs::write(dir.join("config"), "in the way\n").unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext {
        config_dir: Some(dir.join("config")),
        ..CommandContext::default()
    };
    let profile =
        json!({ "name": "Work", "user_name": "Ada", "user_email": "ada@example.invalid" });
    for (id, params) in [
        ("profile.list", json!({})),
        ("profile.save", json!({ "profile": profile })),
        ("profile.delete", json!({ "name": "Work" })),
    ] {
        assert!(registry.run(id, &mut ctx, &params).is_err(), "{id}");
    }
    assert_eq!(
        std::fs::read_to_string(dir.join("config")).unwrap(),
        "in the way\n"
    );

    // A directory where the file is written before it is moved into
    // place: nothing can be saved, and what is kept is still read.
    let kept = dir.join("kept");
    std::fs::create_dir_all(kept.join("profiles.toml.tmp")).unwrap();
    let text = "[[profiles]]\nname = \"Kept\"\nuser_name = \"Ada\"\nuser_email = \"ada@example.invalid\"\n";
    std::fs::write(kept.join("profiles.toml"), text).unwrap();
    ctx.config_dir = Some(kept.clone());
    let err = registry
        .run("profile.save", &mut ctx, &json!({ "profile": profile }))
        .unwrap_err()
        .to_string();
    assert!(err.contains("cannot be written"), "{err}");
    assert!(
        registry
            .run("profile.delete", &mut ctx, &json!({ "name": "Kept" }))
            .is_err()
    );
    let listed = registry.run("profile.list", &mut ctx, &json!({})).unwrap();
    assert_eq!(listed["profiles"][0]["name"], "Kept");
    assert_eq!(
        std::fs::read_to_string(kept.join("profiles.toml")).unwrap(),
        text
    );
}

/// A workspace names its default profile by name. A profile that
/// is renamed is still the one its workspaces name, in whatever case they
/// wrote it; one that is not renamed stays as it is named.
#[test]
fn a_renamed_profile_is_still_the_one_its_workspaces_name() {
    let mut h = Harness::new();
    for name in ["Work", "Home"] {
        h.save(json!({ "name": name, "user_name": "Ada", "user_email": "ada@x.invalid" }));
    }
    for (workspace, profile) in [("Services", "Work"), ("Tools", "work"), ("Garden", "Home")] {
        h.run(
            "workspace.create",
            json!({ "name": workspace, "default_profile": profile }),
        );
    }
    let named = |h: &mut Harness| -> Vec<(String, Value)> {
        h.run("workspace.list", json!({}))["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                (
                    w["name"].as_str().unwrap().to_owned(),
                    w["default_profile"].clone(),
                )
            })
            .collect()
    };

    let out = h.run(
        "profile.save",
        json!({
            "profile": { "name": "Office", "user_name": "Ada", "user_email": "ada@x.invalid" },
            "rename_from": "Work",
        }),
    );
    assert_eq!(out["renamed_from"], "Work");
    assert_eq!(out["workspaces"], 2);
    assert_eq!(
        named(&mut h),
        [
            ("Services".to_owned(), json!("Office")),
            ("Tools".to_owned(), json!("Office")),
            ("Garden".to_owned(), json!("Home")),
        ]
    );
    // Saved again under its own name, nothing else moves.
    let out = h.run(
        "profile.save",
        json!({
            "profile": { "name": "Home", "user_name": "Ada B", "user_email": "ada@x.invalid" },
            "rename_from": "Home",
        }),
    );
    assert!(out.get("workspaces").is_none(), "{out}");
    assert_eq!(named(&mut h)[2], ("Garden".to_owned(), json!("Home")));

    // A workspaces file that is not to be written over keeps the rename
    // from reaching it, and says so; the profile is renamed all the same.
    let file = h.dir.join("config").join("workspaces.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let newer = text.replacen("version = 1", "version = 99", 1);
    assert_ne!(newer, text, "the file has a version to raise");
    std::fs::write(&file, &newer).unwrap();
    let out = h.run(
        "profile.save",
        json!({
            "profile": { "name": "Shed", "user_name": "Ada", "user_email": "ada@x.invalid" },
            "rename_from": "Home",
        }),
    );
    assert_eq!(out["renamed_from"], "Home");
    assert!(out["workspaces_error"].is_string(), "{out}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), newer);
    assert_eq!(h.names(), ["Office", "Shed"]);
}

/// A deleted profile is named by no workspace afterwards, whatever the case
/// it was named in; the others keep theirs. A workspaces file that is not
/// to be written over is left alone and said, and the profile goes anyway.
#[test]
fn a_deleted_profile_is_named_by_no_workspace() {
    let mut h = Harness::new();
    for name in ["Work", "Home", "Spare"] {
        h.save(json!({ "name": name, "user_name": "Ada", "user_email": "ada@x.invalid" }));
    }
    for (workspace, profile) in [("Services", "Work"), ("Tools", "work"), ("Garden", "Home")] {
        h.run(
            "workspace.create",
            json!({ "name": workspace, "default_profile": profile }),
        );
    }
    let named = |h: &mut Harness| -> Vec<(String, Value)> {
        h.run("workspace.list", json!({}))["workspaces"]
            .as_array()
            .unwrap()
            .iter()
            .map(|w| {
                (
                    w["name"].as_str().unwrap().to_owned(),
                    w["default_profile"].clone(),
                )
            })
            .collect()
    };

    let out = h.run("profile.delete", json!({ "name": "WORK" }));
    assert_eq!(out["deleted"]["name"], "Work");
    assert_eq!(out["workspaces"], 2);
    assert_eq!(
        named(&mut h),
        [
            ("Services".to_owned(), Value::Null),
            ("Tools".to_owned(), Value::Null),
            ("Garden".to_owned(), json!("Home")),
        ]
    );
    // One that no workspace names moves nothing.
    let out = h.run("profile.delete", json!({ "name": "Spare" }));
    assert!(out.get("workspaces").is_none(), "{out}");

    let file = h.dir.join("config").join("workspaces.toml");
    let text = std::fs::read_to_string(&file).unwrap();
    let newer = text.replacen("version = 1", "version = 99", 1);
    assert_ne!(newer, text, "the file has a version to raise");
    std::fs::write(&file, &newer).unwrap();
    let out = h.run("profile.delete", json!({ "name": "Home" }));
    assert_eq!(out["deleted"]["name"], "Home");
    assert!(out["workspaces_error"].is_string(), "{out}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), newer);
    assert!(h.names().is_empty());
}
