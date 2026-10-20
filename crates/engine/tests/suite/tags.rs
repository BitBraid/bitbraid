//! Tag commands against `git tag` and `git push` on twin scratch copies,
//! with bare repositories on the local file system as remotes.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Side {
    copy: Scratch,
    bare: PathBuf,
}

impl Side {
    fn new() -> Self {
        Self::of("branches-merges")
    }
    fn of(fixture: &str) -> Self {
        let copy = Scratch::of(fixture).unwrap();
        let bare = copy.path().with_extension("origin.git");
        let _ = std::fs::remove_dir_all(&bare);
        let git = copy.git();
        git.run(["clone", "-q", "--bare", ".", bare.to_str().unwrap()])
            .unwrap();
        git.run(["remote", "add", "origin", bare.to_str().unwrap()])
            .unwrap();
        git.run(["fetch", "-q", "origin"]).unwrap();
        // Commands run git with the real environment; annotated tags need a
        // name.
        git.run(["config", "user.name", "Ada Fixture"]).unwrap();
        git.run(["config", "user.email", "ada@fixture.invalid"])
            .unwrap();
        Self { copy, bare }
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    /// Tags as (name, kind, commit, subject). An annotated tag's own id
    /// carries the date it was made, so it is left out.
    fn tags_of(git: &Git) -> Vec<String> {
        git.lines([
            "for-each-ref",
            "--format=%(refname)|%(objecttype)|%(objectname)|%(*objectname)|%(contents:subject)",
            "refs/tags",
        ])
        .unwrap()
        .into_iter()
        .map(|line| {
            let parts: Vec<&str> = line.split('|').collect();
            let commit = if parts[1] == "tag" {
                parts[3]
            } else {
                parts[2]
            };
            format!("{} {} {commit} {}", parts[0], parts[1], parts[4])
        })
        .collect()
    }
    fn tags(&self) -> Vec<String> {
        Self::tags_of(&self.git())
    }
    fn remote_tags(&self) -> Vec<String> {
        Self::tags_of(&Git::new(&self.bare, fixtures_out_dir().join(".home")))
    }
    fn value(&self, tag: &str) -> String {
        self.git().rev_parse(&format!("refs/tags/{tag}")).unwrap()
    }
    /// Every reference with what it holds, here and on the remote: for
    /// tags that exist on both sides from the start, object for object.
    fn every_ref(&self) -> (Vec<String>, Vec<String>) {
        let format = "%(refname) %(objectname)";
        let remote = Git::new(&self.bare, fixtures_out_dir().join(".home"));
        (
            self.git().user_refs(format).unwrap(),
            remote.user_refs(format).unwrap(),
        )
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
        Self::with(Side::new(), Side::new())
    }
    fn of(fixture: &str) -> Self {
        Self::with(Side::of(fixture), Side::of(fixture))
    }
    fn with(ours: Side, twin: Side) -> Self {
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
    fn both(&self, f: impl Fn(&Side)) {
        f(&self.ours);
        f(&self.twin);
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
    fn assert_same(&self, what: &str) {
        assert_eq!(self.ours.tags(), self.twin.tags(), "{what}: local tags");
        assert_eq!(
            self.ours.remote_tags(),
            self.twin.remote_tags(),
            "{what}: remote tags"
        );
    }
}

#[test]
fn create_matches_git_tag_and_undoes() {
    let mut h = Harness::new();
    let start = h.ours.tags();

    h.run("tag.create", json!({ "name": "light" }));
    h.twin.git().run(["tag", "light"]).unwrap();
    h.run("tag.create", json!({ "name": "older", "target": "HEAD~1" }));
    h.twin.git().run(["tag", "older", "HEAD~1"]).unwrap();
    let out = h.run(
        "tag.create",
        json!({ "name": "release/1.0", "message": "first release\n\nwith notes" }),
    );
    h.twin
        .git()
        .run([
            "tag",
            "-a",
            "-m",
            "first release\n\nwith notes",
            "release/1.0",
        ])
        .unwrap();
    // An empty message is no message: a lightweight tag.
    h.run("tag.create", json!({ "name": "plain", "message": "  " }));
    h.twin.git().run(["tag", "plain"]).unwrap();
    h.assert_same("create");
    let annotated = h.ours.value("release/1.0");
    assert_eq!(out["value"], annotated);
    assert_eq!(
        h.ours
            .git()
            .run(["cat-file", "-t", "refs/tags/release/1.0"])
            .unwrap(),
        "tag\n"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Tag plain");

    // Taken: refused, and the one that is there stays.
    for name in ["light", "release/1.0"] {
        let err = h
            .try_run("tag.create", json!({ "name": name, "target": "HEAD~2" }))
            .unwrap_err();
        assert!(err.contains("exists already"), "{err}");
    }
    h.assert_same("refused create");

    for _ in 0..4 {
        h.run("undo.last", json!({}));
    }
    assert_eq!(h.ours.tags(), start);
    for _ in 0..4 {
        h.run("redo.last", json!({}));
    }
    h.assert_same("redo of create");
    // The annotated tag is the same object again, not a new one.
    assert_eq!(h.ours.value("release/1.0"), annotated);
}

#[test]
fn delete_matches_git_tag_delete_and_undo_brings_the_same_tag_back() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        git.run(["tag", "light"]).unwrap();
        git.run(["tag", "-a", "-m", "notes", "noted", "HEAD~1"])
            .unwrap();
    });
    let (light, noted) = (h.ours.value("light"), h.ours.value("noted"));

    let out = h.run("tag.delete", json!({ "name": "noted" }));
    assert_eq!(out["value"], noted);
    h.run("tag.delete", json!({ "name": "light" }));
    h.twin.git().run(["tag", "-d", "noted", "light"]).unwrap();
    h.assert_same("delete");
    assert!(h.try_run("tag.delete", json!({ "name": "light" })).is_err());

    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.value("light"), light);
    assert_eq!(h.ours.value("noted"), noted, "the tag object, not a copy");
    h.run("redo.last", json!({}));
    h.run("redo.last", json!({}));
    h.assert_same("redo of delete");
}

#[test]
fn undo_leaves_a_tag_alone_that_was_moved_since() {
    let mut h = Harness::new();
    let git = h.ours.git();

    // Created here, moved by hand: undo must not delete it.
    h.run("tag.create", json!({ "name": "moving" }));
    git.run(["tag", "-f", "moving", "HEAD~2"]).unwrap();
    let moved = h.ours.value("moving");
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("not where this action left it"), "{err}");
    assert_eq!(h.ours.value("moving"), moved);

    // Deleted here, made again by hand elsewhere: undo must not overwrite.
    h.run("tag.delete", json!({ "name": "moving" }));
    git.run(["tag", "moving", "HEAD~1"]).unwrap();
    let remade = h.ours.value("moving");
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(err.contains("exists again"), "{err}");
    assert_eq!(h.ours.value("moving"), remade);
}

#[test]
fn push_and_remote_delete_match_git_push() {
    let mut h = Harness::new();
    h.both(|side| {
        let git = side.git();
        git.run(["tag", "one"]).unwrap();
        git.run(["tag", "two", "HEAD~1"]).unwrap();
        git.run(["tag", "-a", "-m", "notes", "three", "HEAD~2"])
            .unwrap();
    });

    // One tag, to the only remote there is.
    let out = h.run("tag.push", json!({ "name": "one" }));
    assert_eq!(out["remote"], "origin");
    h.twin
        .git()
        .run(["push", "-q", "origin", "refs/tags/one"])
        .unwrap();
    h.assert_same("push of one tag");

    // All of them.
    h.run("tag.push", json!({ "remote": "origin" }));
    h.twin
        .git()
        .run(["push", "-q", "--tags", "origin"])
        .unwrap();
    h.assert_same("push of all tags");
    assert!(
        h.ours
            .remote_tags()
            .iter()
            .any(|tag| tag.contains("refs/tags/three tag"))
    );

    // Gone on the remote, still here.
    h.run("tag.delete_remote", json!({ "name": "two" }));
    h.twin
        .git()
        .run(["push", "-q", "--delete", "origin", "refs/tags/two"])
        .unwrap();
    h.assert_same("delete on the remote");
    assert!(
        h.ours
            .tags()
            .iter()
            .any(|tag| tag.starts_with("refs/tags/two "))
    );
    // Once more: git only warns that it is not there, and so do we.
    h.run("tag.delete_remote", json!({ "name": "two" }));
    h.assert_same("delete on the remote, again");
    // A branch of the same name is not what "delete the tag" means.
    h.both(|side| {
        side.git().run(["branch", "one"]).unwrap();
        side.git()
            .run(["push", "-q", "origin", "refs/heads/one"])
            .unwrap();
    });
    h.run("tag.delete_remote", json!({ "name": "one" }));
    h.twin
        .git()
        .run(["push", "-q", "--delete", "origin", "refs/tags/one"])
        .unwrap();
    h.assert_same("delete of a tag that shares its name with a branch");
    let heads = Git::new(&h.ours.bare, fixtures_out_dir().join(".home"))
        .lines(["for-each-ref", "--format=%(refname)", "refs/heads/one"])
        .unwrap();
    assert_eq!(heads, ["refs/heads/one"]);

    // Several remotes, none called origin: it has to be said which.
    let git = h.ours.git();
    git.run(["remote", "rename", "origin", "first"]).unwrap();
    git.run(["remote", "add", "second", h.ours.bare.to_str().unwrap()])
        .unwrap();
    let err = h
        .try_run("tag.push", json!({ "name": "three" }))
        .unwrap_err();
    assert!(err.contains("say which one"), "{err}");
    h.run("tag.push", json!({ "name": "three", "remote": "second" }));
}

#[test]
fn hostile_tag_parameters_are_refused_and_change_nothing() {
    let mut h = Harness::new();
    let git = h.ours.git();
    git.run(["tag", "kept"]).unwrap();
    let local = h.ours.tags();
    let remote = h.ours.remote_tags();
    let refs = git.lines(["for-each-ref"]).unwrap();
    let long = "t".repeat(5000);

    for name in [
        "",
        "-d",
        "--force",
        "a..b",
        "a b",
        "a\nb",
        "HEAD",
        "@",
        "@{-1}",
        "x.lock",
        "/lead",
        "trail/",
        "a~1",
        "a^",
        "a:b",
        long.as_str(),
    ] {
        for (command, params) in [
            ("tag.create", json!({ "name": name })),
            ("tag.create", json!({ "name": name, "message": "m" })),
            ("tag.delete", json!({ "name": name })),
            ("tag.push", json!({ "name": name })),
            ("tag.delete_remote", json!({ "name": name })),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the name {name:?}"
            );
        }
    }
    for target in [
        "",
        "-x",
        "no-such-rev",
        "HEAD^{tree}",
        "a\nb",
        long.as_str(),
    ] {
        assert!(
            h.try_run("tag.create", json!({ "name": "fine", "target": target }))
                .is_err(),
            "tag.create accepted the target {target:?}"
        );
    }
    for remote_name in [
        "",
        "-x",
        "--exec=touch pwned",
        "nowhere",
        "a/b",
        long.as_str(),
    ] {
        for (command, params) in [
            ("tag.push", json!({ "name": "kept", "remote": remote_name })),
            ("tag.push", json!({ "remote": remote_name })),
            (
                "tag.delete_remote",
                json!({ "name": "kept", "remote": remote_name }),
            ),
        ] {
            assert!(
                h.try_run(command, params).is_err(),
                "{command} accepted the remote {remote_name:?}"
            );
        }
    }
    for params in [
        json!({}),
        json!({ "name": 3 }),
        json!({ "name": "fine", "sign": "yes" }),
        json!({ "name": "fine", "message": "nul\u{0}" }),
        // Signed without a message: refused before any key is asked for.
        json!({ "name": "fine", "sign": true }),
    ] {
        assert!(h.try_run("tag.create", params.clone()).is_err(), "{params}");
    }
    assert_eq!(h.ours.tags(), local);
    assert_eq!(h.ours.remote_tags(), remote);
    assert_eq!(git.lines(["for-each-ref"]).unwrap(), refs);
    assert!(!h.ours.copy.path().join("pwned").exists());
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());
}

const NESTED: [&str; 6] = [
    "alias",
    "inner",
    "kept-twice",
    "outer",
    "outermost",
    "plain",
];

#[test]
fn a_tag_of_a_tag_is_told_in_full_as_git_peels_it() {
    let mut h = Harness::of("nested-tags");
    let git = h.ours.git();
    for name in NESTED {
        let reference = format!("refs/tags/{name}");
        let details = h.run("tag.details", json!({ "name": name }));
        let held = git.rev_parse(&reference).unwrap();
        let annotated = git.run(["cat-file", "-t", &held]).unwrap() == "tag\n";
        assert_eq!(details["value"], held, "{name}");
        assert_eq!(details["annotated"], annotated, "{name}");
        // Through every tag on the way, to the commit.
        assert_eq!(
            details["target"],
            git.rev_parse(&format!("{reference}^{{}}")).unwrap(),
            "{name}"
        );
        assert_eq!(
            details["target"],
            git.rev_parse(&format!("{reference}^{{commit}}")).unwrap(),
            "{name}"
        );
        assert_eq!(details["target_kind"], "commit", "{name}");
        // Message and tagger are the outermost tag object's own.
        let format = |field: &str| {
            git.run(["for-each-ref", &format!("--format={field}"), &reference])
                .unwrap()
        };
        if annotated {
            assert_eq!(
                details["message"].as_str().unwrap(),
                format("%(contents)").trim_end(),
                "{name}"
            );
            assert_eq!(
                details["tagger"]["email"].as_str().unwrap(),
                format("%(taggeremail:trim)").trim(),
                "{name}"
            );
        } else {
            assert_eq!(details["message"], "", "{name}");
            assert!(details["tagger"].is_null(), "{name}");
        }
        assert_eq!(details["signature"]["state"], "none", "{name}");
    }
    // The reference that holds another tag's object tells that tag.
    let alias = h.run("tag.details", json!({ "name": "alias" }));
    let inner = h.run("tag.details", json!({ "name": "inner" }));
    assert_eq!(alias["value"], inner["value"]);
    assert_eq!(alias["message"], "the commit");
    assert_eq!(
        h.run("tag.details", json!({ "name": "outermost" }))["message"],
        "the tag of the tag"
    );
}

#[test]
fn tags_are_made_deleted_and_restored_on_tags_of_tags() {
    let mut h = Harness::of("nested-tags");
    let start = h.ours.every_ref();
    assert_eq!(start, h.twin.every_ref());
    let commit =
        |side: &Side, rev: &str| side.git().rev_parse(&format!("{rev}^{{commit}}")).unwrap();

    // A new tag at a tag of a tag is a tag of the commit that comes to.
    let out = h.run(
        "tag.create",
        json!({ "name": "light-on-nested", "target": "outermost" }),
    );
    assert_eq!(out["value"], commit(&h.ours, "outermost"));
    h.twin
        .git()
        .run(["tag", "light-on-nested", "outermost^{commit}"])
        .unwrap();
    h.run(
        "tag.create",
        json!({ "name": "noted-on-nested", "target": "kept-twice", "message": "on g1" }),
    );
    h.twin
        .git()
        .run([
            "tag",
            "-a",
            "-m",
            "on g1",
            "noted-on-nested",
            "kept-twice^{commit}",
        ])
        .unwrap();
    h.assert_same("create at a nested tag");
    let made = h
        .ours
        .git()
        .run(["cat-file", "-p", "refs/tags/noted-on-nested"])
        .unwrap();
    assert!(made.contains("type commit"), "{made}");
    assert!(
        made.starts_with(&format!("object {}\n", commit(&h.ours, "kept-twice"))),
        "{made}"
    );
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.every_ref(), start);
    h.twin
        .git()
        .run(["tag", "-d", "light-on-nested", "noted-on-nested"])
        .unwrap();

    // Deleting takes the reference and nothing else; undo puts back the
    // very tag object it held, the tag of a tag.
    for name in ["outer", "kept-twice", "alias"] {
        let held = h.ours.value(name);
        let peeled = commit(&h.ours, name);
        let out = h.run("tag.delete", json!({ "name": name }));
        assert_eq!(out["value"], held, "{name}");
        h.twin.git().run(["tag", "-d", name]).unwrap();
        assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "delete {name}");
        h.run("undo.last", json!({}));
        assert_eq!(h.ours.value(name), held, "{name}: the same object again");
        assert_eq!(commit(&h.ours, name), peeled, "{name}");
        assert_eq!(h.ours.every_ref(), start, "undo of delete {name}");
        h.run("redo.last", json!({}));
        assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "redo {name}");
        h.run("undo.last", json!({}));
        h.twin.git().run(["tag", name, &held]).unwrap();
        assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "{name} is back");
    }
    // The tags that pointed through the deleted one never noticed.
    assert_eq!(h.ours.every_ref(), start);

    // A branch and a checkout at a nested tag go to its commit.
    h.run(
        "branch.create",
        json!({ "name": "from-nested", "start": "outermost" }),
    );
    h.twin
        .git()
        .run(["branch", "from-nested", "outermost"])
        .unwrap();
    h.run("branch.checkout", json!({ "target": "kept-twice" }));
    h.twin.git().run(["checkout", "-q", "kept-twice"]).unwrap();
    assert_eq!(h.ours.every_ref(), h.twin.every_ref());
    h.both(|side| {
        let git = side.git();
        assert_eq!(git.rev_parse("HEAD").unwrap(), commit(side, "kept-twice"));
        assert!(git.run(["symbolic-ref", "-q", "HEAD"]).is_err(), "detached");
        assert_eq!(git.rev_parse("from-nested").unwrap(), commit(side, "inner"));
    });
}

#[test]
fn tags_of_tags_are_pushed_and_deleted_on_a_remote_with_what_they_hold() {
    let mut h = Harness::of("nested-tags");
    // The remotes were cloned with every tag: take them out there first.
    h.both(|side| {
        let remote = Git::new(&side.bare, fixtures_out_dir().join(".home"));
        for name in NESTED {
            remote
                .run(["update-ref", "-d", &format!("refs/tags/{name}")])
                .unwrap();
        }
        // What only the tags held is gone from the remote with them.
        remote.run(["gc", "-q", "--prune=now"]).unwrap();
        assert!(remote.refs("refs/tags").unwrap().is_empty());
    });
    let inner_object = h.ours.value("inner");
    let in_remote = |side: &Side, object: &str| {
        Git::new(&side.bare, fixtures_out_dir().join(".home"))
            .run(["cat-file", "-t", object])
            .map(|kind| kind.trim().to_owned())
            .ok()
    };
    assert_eq!(in_remote(&h.ours, &inner_object), None);

    // One tag of a tag of a tag: the whole chain goes with it.
    h.run("tag.push", json!({ "name": "outermost" }));
    h.twin
        .git()
        .run(["push", "-q", "origin", "refs/tags/outermost"])
        .unwrap();
    assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "push of one");
    assert_eq!(
        h.ours
            .every_ref()
            .1
            .iter()
            .filter(|line| line.contains("refs/tags/"))
            .count(),
        1
    );
    assert_eq!(in_remote(&h.ours, &inner_object).as_deref(), Some("tag"));
    let remote = Git::new(&h.ours.bare, fixtures_out_dir().join(".home"));
    assert_eq!(
        remote.rev_parse("refs/tags/outermost^{commit}").unwrap(),
        h.ours.git().rev_parse("refs/tags/inner^{commit}").unwrap()
    );

    // All of them, the one that alone holds a commit included.
    h.run("tag.push", json!({ "remote": "origin" }));
    h.twin
        .git()
        .run(["push", "-q", "--tags", "origin"])
        .unwrap();
    assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "push of all");
    assert_eq!(
        remote.rev_parse("refs/tags/kept-twice^{commit}").unwrap(),
        h.ours
            .git()
            .rev_parse("refs/tags/kept-twice^{commit}")
            .unwrap()
    );

    // Deleted there, kept here.
    h.run("tag.delete_remote", json!({ "name": "outer" }));
    h.twin
        .git()
        .run(["push", "-q", "origin", ":refs/tags/outer"])
        .unwrap();
    assert_eq!(h.ours.every_ref(), h.twin.every_ref(), "remote delete");
    assert!(h.ours.git().rev_parse("refs/tags/outer").is_ok());
    assert!(remote.rev_parse("refs/tags/outer").is_err());
    // What pointed through it there still does.
    assert_eq!(
        in_remote(&h.ours, &h.ours.value("outer")).as_deref(),
        Some("tag")
    );
}

#[test]
fn a_lightweight_tag_is_annotated_like_git_tag_force_and_undoes() {
    let mut h = Harness::new();
    h.both(|side| {
        side.git().run(["tag", "light", "HEAD~1"]).unwrap();
        side.git()
            .run(["tag", "-a", "-m", "has one", "told"])
            .unwrap();
    });
    let light = h.ours.value("light");
    let out = h.run(
        "tag.annotate",
        json!({ "name": "light", "message": "said afterwards\n\nwith notes" }),
    );
    h.twin
        .git()
        .run([
            "tag",
            "-a",
            "-f",
            "-m",
            "said afterwards\n\nwith notes",
            "light",
            "HEAD~1",
        ])
        .unwrap();
    h.assert_same("annotate");
    let annotated = h.ours.value("light");
    assert_eq!(out["value"], annotated);
    assert_eq!(
        h.ours
            .git()
            .run(["cat-file", "-t", "refs/tags/light"])
            .unwrap(),
        "tag\n"
    );
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Annotate tag light");

    // Undone: the lightweight tag at the same commit again; done again:
    // the same tag object.
    h.run("undo.last", json!({}));
    assert_eq!(h.ours.value("light"), light);
    assert_eq!(
        h.ours
            .git()
            .run(["cat-file", "-t", "refs/tags/light"])
            .unwrap(),
        "commit\n"
    );
    h.run("redo.last", json!({}));
    assert_eq!(h.ours.value("light"), annotated);

    // Moved since: the undo leaves it alone.
    h.ours.git().run(["tag", "-f", "light", "HEAD"]).unwrap();
    let moved = h.ours.value("light");
    assert!(h.try_run("undo.last", json!({})).is_err());
    assert_eq!(h.ours.value("light"), moved);

    // A tag with a message already, one that is not there, and hostile
    // names and messages: refused, nothing changes.
    let before = h.ours.every_ref();
    let long = "x".repeat(300);
    for params in [
        json!({ "name": "told", "message": "again" }),
        json!({ "name": "nowhere", "message": "m" }),
        json!({ "name": "-d", "message": "m" }),
        json!({ "name": "a..b", "message": "m" }),
        json!({ "name": "HEAD", "message": "m" }),
        json!({ "name": "", "message": "m" }),
        json!({ "name": long, "message": "m" }),
        json!({ "name": "told", "message": "" }),
        json!({ "name": 3, "message": "m" }),
        json!({ "name": "told" }),
        json!({ "message": "m" }),
    ] {
        assert!(
            h.try_run("tag.annotate", params.clone()).is_err(),
            "{params}"
        );
    }
    h.ours.git().run(["tag", "bare-light"]).unwrap();
    for message in ["", "   ", "nul\u{0}here"] {
        assert!(
            h.try_run(
                "tag.annotate",
                json!({ "name": "bare-light", "message": message })
            )
            .is_err(),
            "{message:?}"
        );
    }
    h.ours.git().run(["tag", "-d", "bare-light"]).unwrap();
    assert_eq!(h.ours.every_ref(), before);
}
