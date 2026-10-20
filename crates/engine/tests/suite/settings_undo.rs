//! Undo and redo of what changes the repository's own settings: a removed
//! remote with everything removal takes along, a remote that was added or
//! pointed elsewhere, the signing settings. After undo the repository is
//! held to a twin copy that never ran the command; after redo, to a twin
//! that git did the same to.
//!
//! The other settings commands are tested where their commands are: a
//! profile's identity in `profiles.rs`, gitflow's names in `flow.rs`, a
//! branch's description in `housekeeping.rs`, rerere in `rerere.rs`.

use std::{collections::BTreeMap, path::PathBuf};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Side {
    copy: Scratch,
    bare: PathBuf,
}

impl Side {
    /// A copy with a remote `origin` that has everything a remote can
    /// have: two fetch specifications, a push URL, settings of its own, a
    /// symbolic `HEAD`, two branches that follow it, and a branch that is
    /// only there, so that its commit is held here by nothing but the
    /// remote-tracking reference.
    fn new() -> Self {
        let copy = Scratch::of("branches-merges").unwrap();
        let bare = copy.path().with_extension("origin.git");
        let _ = std::fs::remove_dir_all(&bare);
        let git = copy.git();
        let url = bare.to_str().unwrap();
        git.run(["clone", "-q", "--bare", ".", url]).unwrap();
        // Someone else's branch, with a commit of its own.
        let remote = Git::new(&bare, fixtures_out_dir().join(".home"));
        let tree = remote.rev_parse("main^{tree}").unwrap();
        let only_there = remote
            .run([
                "commit-tree",
                "-p",
                "main",
                "-m",
                "only on the remote",
                &tree,
            ])
            .unwrap();
        remote
            .run(["update-ref", "refs/heads/only-there", only_there.trim()])
            .unwrap();
        remote.run(["tag", "remote-tag", "main~1"]).unwrap();

        git.run(["remote", "add", "origin", url]).unwrap();
        git.run([
            "config",
            "--add",
            "remote.origin.fetch",
            "+refs/tags/*:refs/remotes/origin/tags/*",
        ])
        .unwrap();
        git.run([
            "remote",
            "set-url",
            "--push",
            "origin",
            &format!("{url}/push"),
        ])
        .unwrap();
        git.run(["config", "remote.origin.tagOpt", "--no-tags"])
            .unwrap();
        git.run(["config", "remote.origin.prune", "true"]).unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        git.run(["remote", "set-head", "origin", "main"]).unwrap();
        git.run(["branch", "-q", "--set-upstream-to=origin/main", "main"])
            .unwrap();
        git.run([
            "branch",
            "-q",
            "--set-upstream-to=origin/feature/a",
            "feature/a",
        ])
        .unwrap();
        git.run(["config", "branch.main.pushRemote", "origin"])
            .unwrap();
        git.run(["config", "remote.pushDefault", "origin"]).unwrap();
        // A second remote, which stays through all of it.
        git.run(["remote", "add", "other", &format!("{url}/elsewhere")])
            .unwrap();
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("commit.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        Self { copy, bare }
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    /// Everything removing a remote can touch, without what differs
    /// between two copies for being in two places: the references of the
    /// user's with what they hold and where a symbolic one points, HEAD,
    /// and the repository's own settings, each name with its values in
    /// their order.
    fn state(&self) -> (Vec<String>, BTreeMap<String, Vec<String>>) {
        let git = self.git();
        let mut refs = git.user_refs("%(refname) %(objectname) %(symref)").unwrap();
        refs.push(format!(
            "HEAD {}",
            git.run(["symbolic-ref", "HEAD"]).unwrap().trim()
        ));
        let here = self.bare.to_str().unwrap();
        let mut settings: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for record in git
            .run(["config", "--local", "--list", "-z"])
            .unwrap()
            .split('\0')
            .filter(|record| !record.is_empty())
        {
            let (key, value) = record.split_once('\n').unwrap_or((record, "true"));
            settings
                .entry(key.to_owned())
                .or_default()
                .push(value.replace(here, "<remote>"));
        }
        (refs, settings)
    }
}

struct Harness {
    ours: Side,
    twin: Side,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let ours = Side::new();
        let twin = Side::new();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": ours.copy.path() }))
            .expect("open scratch");
        Self {
            ours,
            twin,
            registry,
            ctx,
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
            .map_err(|e| e.to_string())
    }
    fn peek(&mut self) -> Value {
        self.run("undo.peek", json!({}))["undo"].clone()
    }
    fn assert_same(&self, what: &str) {
        let (ours, twin) = (self.ours.state(), self.twin.state());
        assert_eq!(ours.0, twin.0, "{what}: references");
        assert_eq!(ours.1, twin.1, "{what}: settings");
    }
}

#[test]
fn a_removed_remote_comes_back_with_everything_removal_took() {
    let mut h = Harness::new();
    h.assert_same("to begin with");
    let before = h.ours.state();
    // What there is to lose.
    for expected in [
        "refs/remotes/origin/HEAD",
        "refs/remotes/origin/main",
        "refs/remotes/origin/only-there",
        "refs/remotes/origin/tags/remote-tag",
    ] {
        assert!(
            before.0.iter().any(|line| line.starts_with(expected)),
            "{expected} is in {:?}",
            before.0
        );
    }
    assert_eq!(before.1["remote.origin.fetch"].len(), 2);
    assert_eq!(before.1["remote.origin.pushurl"], ["<remote>/push"]);
    assert_eq!(before.1["branch.main.remote"], ["origin"]);
    assert_eq!(before.1["branch.feature/a.merge"], ["refs/heads/feature/a"]);
    let only_there = h
        .ours
        .git()
        .rev_parse("refs/remotes/origin/only-there")
        .unwrap();

    let out = h.run("remote.remove", json!({ "name": "origin" }));
    assert_eq!(out["removed"], "origin");
    assert!(out["references"].as_u64().unwrap() >= 5, "{out}");
    let removed = h.ours.state();
    assert!(
        removed
            .0
            .iter()
            .all(|line| !line.contains("refs/remotes/origin/"))
    );
    assert!(
        removed
            .1
            .keys()
            .all(|key| !key.starts_with("remote.origin."))
    );
    assert!(!removed.1.contains_key("branch.main.remote"));
    assert!(!removed.1.contains_key("branch.feature/a.merge"));
    // The other remote was nobody's business.
    assert_eq!(removed.1["remote.other.url"], before.1["remote.other.url"]);
    assert_eq!(h.peek(), "Remove remote origin");

    // What only the remote's references held is not lost to a prune in
    // between.
    h.ours.git().run(["gc", "-q", "--prune=now"]).unwrap();
    assert_eq!(
        h.ours.git().run(["cat-file", "-t", &only_there]).unwrap(),
        "commit\n"
    );

    // Undone: the copy is the twin, which never removed anything.
    h.run("undo.last", json!({}));
    h.assert_same("undo of the removal");
    assert_eq!(h.ours.state(), before);
    assert_eq!(
        h.ours
            .git()
            .run(["rev-parse", "--abbrev-ref", "main@{upstream}"])
            .unwrap(),
        "origin/main\n"
    );
    assert_eq!(
        h.ours
            .git()
            .run(["symbolic-ref", "refs/remotes/origin/HEAD"])
            .unwrap(),
        "refs/remotes/origin/main\n"
    );
    // It works as a remote again.
    h.run("remote.fetch", json!({ "remote": "origin" }));
    h.twin.git().run(["fetch", "-q", "origin"]).unwrap();
    h.assert_same("a fetch from the remote that is back");

    // Redone: the twin removes it with git.
    h.run("redo.last", json!({}));
    h.twin.git().run(["remote", "remove", "origin"]).unwrap();
    h.assert_same("redo of the removal");
    assert_eq!(h.ours.state(), removed);
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.state(), before, "and back once more");
}

#[test]
fn a_removed_remote_is_not_put_back_over_what_came_since() {
    let mut h = Harness::new();
    h.run("remote.remove", json!({ "name": "origin" }));
    let removed = h.ours.state();
    let git = h.ours.git();

    // A remote of that name again, added by hand.
    git.run(["remote", "add", "origin", "/somewhere/else"])
        .unwrap();
    let by_hand = h.ours.state();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("not what this action left it as"), "{err}");
    assert_eq!(h.ours.state(), by_hand, "nothing was changed");
    git.run(["remote", "remove", "origin"]).unwrap();
    assert_eq!(h.ours.state(), removed);

    // A reference where one of its own would go.
    git.run(["update-ref", "refs/remotes/origin/main", "main"])
        .unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("exists again"), "{err}");
    assert!(
        h.ours
            .state()
            .1
            .keys()
            .all(|key| !key.starts_with("remote.origin."))
    );
    git.run(["update-ref", "-d", "refs/remotes/origin/main"])
        .unwrap();

    // A branch that followed it follows something else now.
    git.run(["config", "branch.main.remote", "other"]).unwrap();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("branch.main.remote"), "{err}");
    git.run(["config", "--unset", "branch.main.remote"])
        .unwrap();
    assert_eq!(h.ours.state(), removed);

    // With all of that out of the way it comes back.
    h.run("undo.last", json!({}));
    h.assert_same("undo after all");

    // What its references held was pruned behind our back (the
    // references that held it were taken away by hand): refused, with
    // nothing changed, rather than references to nothing.
    h.run("redo.last", json!({}));
    let held = git
        .lines(["for-each-ref", "--format=%(refname)", "refs/bitbraid"])
        .unwrap();
    assert!(!held.is_empty());
    for reference in held {
        git.run(["update-ref", "-d", &reference]).unwrap();
    }
    git.run(["gc", "-q", "--prune=now"]).unwrap();
    let removed = h.ours.state();
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("is gone"), "{err}");
    assert_eq!(h.ours.state(), removed);
}

#[test]
fn adding_a_remote_and_pointing_it_elsewhere_are_undone() {
    let mut h = Harness::new();
    let before = h.ours.state();

    h.run(
        "remote.add",
        json!({ "name": "second", "url": "https://example.invalid/second.git" }),
    );
    assert_eq!(h.peek(), "Add remote second");
    h.run("undo.last", json!({}));
    h.assert_same("undo of add");
    h.run("redo.last", json!({}));
    h.twin
        .git()
        .run([
            "remote",
            "add",
            "second",
            "https://example.invalid/second.git",
        ])
        .unwrap();
    h.assert_same("redo of add");

    h.run(
        "remote.set_url",
        json!({ "name": "second", "url": "https://example.invalid/moved.git" }),
    );
    assert_eq!(h.peek(), "Set URL of second");
    h.run(
        "remote.set_url",
        json!({ "name": "second", "url": "ssh://example.invalid/push.git", "push": true }),
    );
    assert_eq!(h.peek(), "Set push URL of second");
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    h.assert_same("undo of both URLs");
    h.run("redo.last", json!({}));
    h.run("redo.last", json!({}));
    let twin = h.twin.git();
    twin.run([
        "remote",
        "set-url",
        "second",
        "https://example.invalid/moved.git",
    ])
    .unwrap();
    twin.run([
        "remote",
        "set-url",
        "--push",
        "second",
        "ssh://example.invalid/push.git",
    ])
    .unwrap();
    h.assert_same("redo of both URLs");

    // The same URL again changes nothing and leaves no entry.
    h.run(
        "remote.set_url",
        json!({ "name": "second", "url": "https://example.invalid/moved.git" }),
    );
    assert_eq!(h.peek(), "Set push URL of second");

    // All of it back, in order.
    for _ in 0..3 {
        h.run("undo.last", json!({}));
    }
    assert!(h.peek().is_null());
    assert_eq!(h.ours.state(), before);
}

#[test]
fn signing_settings_are_undone_and_redone() {
    let mut h = Harness::new();
    let before = h.ours.state();
    let out = h.run(
        "sign.configure",
        json!({ "format": "ssh", "key": "/keys/id_sign.pub", "commits": true, "tags": true }),
    );
    assert_eq!(out["format"], "ssh");
    assert_eq!(h.peek(), "Change signing settings");
    let twin = h.twin.git();
    for (key, value) in [
        ("gpg.format", "ssh"),
        ("user.signingkey", "/keys/id_sign.pub"),
        ("commit.gpgsign", "true"),
        ("tag.gpgsign", "true"),
    ] {
        twin.run(["config", key, value]).unwrap();
    }
    h.assert_same("configured");
    let configured = h.ours.state();

    h.run("undo.last", json!({}));
    assert_eq!(h.ours.state(), before, "undone: as it never was");
    // The copy had commit.gpgsign set to false to begin with: undone, it
    // is that again, not gone.
    assert_eq!(h.ours.state().1["commit.gpgsign"], ["false"]);
    h.run("redo.last", json!({}));
    assert_eq!(h.ours.state(), configured);

    // One switch alone, then the key taken away: two entries.
    h.run("sign.configure", json!({ "tags": false }));
    h.run("sign.configure", json!({ "key": "" }));
    assert!(!h.ours.state().1.contains_key("user.signingkey"));
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.state().1["user.signingkey"], ["/keys/id_sign.pub"]);
    assert_eq!(h.ours.state().1["tag.gpgsign"], ["false"]);
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.state(), configured);
    // What is as it is already leaves no entry.
    h.run("sign.configure", json!({ "tags": true }));
    assert_eq!(h.peek(), "Change signing settings");
    h.run("undo.last", json!({}));
    assert!(h.peek().is_null());
    assert_eq!(h.ours.state(), before);
}

/// A journal that somebody else wrote: what it names as settings is
/// refused when it is nothing git could have listed, and nothing is
/// written.
#[test]
fn a_journal_that_names_no_settings_is_refused_without_a_crash() {
    let h = Harness::new();
    let before = h.ours.state();
    let long = "k".repeat(100_000);
    let zero = "0".repeat(40);
    for (n, change) in [
        json!({ "kind": "config", "settings": [{ "key": "-c", "before": ["x"], "after": [] }] }),
        json!({ "kind": "config", "settings": [{ "key": "nodot", "before": ["x"], "after": [] }] }),
        json!({ "kind": "config", "settings": [{ "key": "a.b\nc.d", "before": ["x"], "after": [] }] }),
        json!({ "kind": "config", "settings": [{ "key": "", "before": ["x"], "after": [] }] }),
        json!({ "kind": "config", "settings": [{ "key": long, "before": ["x"], "after": [] }] }),
        json!({ "kind": "config", "settings": [{ "key": "core.x", "before": ["a\u{0}b"], "after": [] }] }),
        json!({ "kind": "config", "settings": [
            { "key": "user.name", "before": ["Somebody Else"], "after": ["Not What Is There"] },
        ] }),
        json!({ "kind": "config", "settings": [], "branch": ["main", zero] }),
        json!({ "kind": "remote_remove", "name": "-x", "settings": [], "references": [] }),
        json!({ "kind": "remote_remove", "name": "gone", "settings": [], "references": [
            { "name": "HEAD", "object": zero, "symbolic": null },
        ] }),
        json!({ "kind": "remote_remove", "name": "gone", "settings": [], "references": [
            { "name": "refs/heads/main", "object": zero, "symbolic": null },
        ] }),
        json!({ "kind": "remote_remove", "name": "gone", "settings": [], "references": [
            { "name": "refs/remotes/gone/x", "object": zero, "symbolic": "../../../etc/passwd" },
        ] }),
        json!({ "kind": "remote_remove", "name": "gone", "settings": [], "references": [
            { "name": "refs/remotes/gone/x y", "object": zero, "symbolic": null },
        ] }),
        json!({ "kind": "remote_remove", "name": "gone", "settings": [], "references": [
            { "name": "refs/remotes/gone/nothing", "object": zero, "symbolic": null },
        ] }),
    ]
    .into_iter()
    .enumerate()
    {
        let journal = json!({
            "next_id": 1,
            "done": [{ "id": 1, "label": "written by somebody else", "time": 0, "change": change }],
            "undone": [],
        });
        let dir = h.ours.copy.path().join(".git/bitbraid");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("journal.json"), journal.to_string()).unwrap();
        let mut ctx = CommandContext::default();
        h.registry
            .run("repo.open", &mut ctx, &json!({ "path": h.ours.copy.path() }))
            .unwrap();
        let peek = h.registry.run("undo.peek", &mut ctx, &json!({})).unwrap();
        assert_eq!(peek["undo"], "written by somebody else", "{n}: it is read");
        let undone = h.registry.run("undo.last", &mut ctx, &json!({}));
        assert!(undone.is_err(), "{n}: {undone:?}");
        assert_eq!(h.ours.state(), before, "{n}: nothing was changed");
    }
    // An entry about a branch that is not there has nothing to take
    // back, and takes nothing.
    let journal = json!({
        "next_id": 1,
        "done": [{ "id": 1, "label": "nothing to it", "time": 0, "change": {
            "kind": "config", "settings": [], "branch": ["-x", zero],
        } }],
        "undone": [],
    });
    std::fs::write(
        h.ours.copy.path().join(".git/bitbraid/journal.json"),
        journal.to_string(),
    )
    .unwrap();
    let mut ctx = CommandContext::default();
    h.registry
        .run(
            "repo.open",
            &mut ctx,
            &json!({ "path": h.ours.copy.path() }),
        )
        .unwrap();
    let _ = h.registry.run("undo.last", &mut ctx, &json!({}));
    assert_eq!(h.ours.state(), before);
}
