//! What undo needs survives a prune. Each test does something
//! whose undo depends on an object nothing of the user's refers to any
//! more, runs `git gc --prune=now`, which removes every object that is
//! not reachable, and undoes: the content, the stash, the branch, the tag
//! is back as it was. And the other way round: what the journal no longer
//! has a use for is let go, and git prunes it.

use std::path::PathBuf;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_git::{Oid, hold};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        let git = scratch.git();
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        Self::open(scratch)
    }
    fn open(scratch: Scratch) -> Self {
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
    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.scratch.path().join(rel)).unwrap()
    }
    /// Remove every object that is not reachable, at once.
    fn prune(&self) {
        self.git().run(["gc", "-q", "--prune=now"]).unwrap();
    }
    fn exists(&self, object: &str) -> bool {
        self.git().run(["cat-file", "-e", object]).is_ok()
    }
    /// What is held for the journal: entry and object.
    fn held(&self) -> Vec<(u64, String)> {
        hold::held(self.ctx.repo.as_ref().unwrap())
            .unwrap()
            .into_iter()
            .map(|(entry, object)| (entry, object.to_hex()))
            .collect()
    }
    /// The references that hold, as git lists them.
    fn holding(&self) -> Vec<String> {
        self.git()
            .lines(["for-each-ref", "--format=%(refname)", "refs/bitbraid"])
            .unwrap()
    }
    /// The blob git would make of a text, without making it.
    fn blob_of(&self, text: &str) -> String {
        let id: Oid = bitbraid_git::stage::blob_id(text.as_bytes()).unwrap();
        id.to_hex()
    }
}

#[test]
fn discarded_content_comes_back_after_a_prune() {
    let mut h = Harness::new("branches-merges");
    let tracked = "a1\nchanged in the worktree and never committed\n";
    let untracked = "a new file nobody has ever staged\n";
    h.scratch.write("a.txt", tracked).unwrap();
    h.scratch.write("new.txt", untracked).unwrap();
    let (kept_a, kept_new) = (h.blob_of(tracked), h.blob_of(untracked));
    assert!(!h.exists(&kept_a), "nothing has made the blob yet");

    h.run("discard.paths", json!({ "paths": ["a.txt", "new.txt"] }));
    assert_ne!(h.read("a.txt"), tracked);
    assert!(!h.scratch.path().join("new.txt").exists());
    // The backups are held, each under the entry that needs it.
    let held = h.held();
    assert_eq!(held.len(), 2, "{held:?}");
    assert!(held.iter().all(|(entry, _)| *entry == held[0].0));
    let objects: Vec<&str> = held.iter().map(|(_, object)| object.as_str()).collect();
    assert!(objects.contains(&kept_a.as_str()) && objects.contains(&kept_new.as_str()));

    // No branch, tag or commit has them; the references of ours do.
    h.prune();
    assert!(h.exists(&kept_a) && h.exists(&kept_new), "pruned");
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), tracked);
    assert_eq!(h.read("new.txt"), untracked);

    // Redone, the backups are new ones and held in their turn.
    h.run("redo.last", json!({}));
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), tracked);
    assert_eq!(h.read("new.txt"), untracked);
}

#[test]
fn every_kind_of_backup_is_held() {
    let mut h = Harness::new("branches-merges");

    // Lines of a hunk.
    let hunk = "a1\na line to throw away\n";
    h.scratch.write("a.txt", hunk).unwrap();
    h.run(
        "discard.hunks",
        json!({ "path": "a.txt", "selection": [{ "hunk": 0 }], "context": 3 }),
    );
    assert_ne!(h.read("a.txt"), hunk);
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), hunk);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));

    // A file put back to what it was in a commit.
    let local = "what the file held before it was restored\n";
    h.scratch.write("a.txt", local).unwrap();
    h.run("file.restore", json!({ "path": "a.txt", "rev": "HEAD~1" }));
    assert_ne!(h.read("a.txt"), local);
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), local);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));

    // A file edited in the application: before and after.
    let before = h.read("a.txt");
    let edited = "edited in the application\n";
    h.run("file.write", json!({ "path": "a.txt", "text": edited }));
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), before);
    h.prune();
    h.run("redo.last", json!({}));
    assert_eq!(h.read("a.txt"), edited);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));

    // A hard reset over local changes.
    let lost = "local work a hard reset writes over\n";
    h.scratch.write("a.txt", lost).unwrap();
    h.run(
        "branch.reset",
        json!({ "target": "HEAD~1", "mode": "hard" }),
    );
    assert_ne!(h.read("a.txt"), lost);
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), lost);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));

    // A checkout that throws local changes away.
    let thrown = "local work a checkout throws away\n";
    h.scratch.write("a.txt", thrown).unwrap();
    h.run(
        "branch.checkout",
        json!({ "target": "feature/a", "dirty": "discard" }),
    );
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), thrown);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));

    // A patch that was applied: the patch itself is what undo applies
    // the other way round.
    let patch: PathBuf = h.scratch.path().with_extension("patch");
    h.scratch.write("a.txt", "a1\nfrom a patch\n").unwrap();
    let diff = h.git().run(["diff"]).unwrap();
    std::fs::write(&patch, &diff).unwrap();
    h.git().run(["checkout", "-q", "--", "a.txt"]).unwrap();
    let clean = h.read("a.txt");
    h.run("patch.apply", json!({ "file": patch }));
    assert_eq!(h.read("a.txt"), "a1\nfrom a patch\n");
    std::fs::remove_file(&patch).unwrap();
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), clean);
    h.prune();
    h.run("redo.last", json!({}));
    assert_eq!(h.read("a.txt"), "a1\nfrom a patch\n");
}

#[test]
fn a_dropped_stash_a_deleted_branch_and_a_deleted_tag_come_back_after_a_prune() {
    let mut h = Harness::new("branches-merges");
    let git = h.git();

    // A stash that is dropped leaves the stash list, which was the one
    // thing that held it.
    h.scratch.write("a.txt", "a1\nstashed work\n").unwrap();
    h.scratch.write("untracked.txt", "stashed too\n").unwrap();
    let out = h.run(
        "stash.push",
        json!({ "message": "set aside", "include_untracked": true }),
    );
    let stash = out["stash"].as_str().unwrap().to_owned();
    h.run("stash.drop", json!({ "stash": stash }));
    assert!(git.rev_parse("refs/stash").is_err(), "the list is empty");
    h.prune();
    assert!(h.exists(&stash), "the dropped stash was pruned");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("refs/stash").unwrap(), stash);
    h.run("stash.pop", json!({}));
    assert_eq!(h.read("a.txt"), "a1\nstashed work\n");
    assert_eq!(h.read("untracked.txt"), "stashed too\n");
    git.run(["checkout", "-q", "--", "a.txt"]).unwrap();
    std::fs::remove_file(h.scratch.path().join("untracked.txt")).unwrap();

    // A branch that was never checked out here and is merged nowhere:
    // no reflog of HEAD has its commit.
    let tree = git.rev_parse("main^{tree}").unwrap();
    let lonely = git
        .run([
            "commit-tree",
            "-p",
            "main",
            "-m",
            "on a branch of its own",
            &tree,
        ])
        .unwrap()
        .trim()
        .to_owned();
    git.run(["update-ref", "refs/heads/lonely", &lonely])
        .unwrap();
    h.run("branch.delete", json!({ "name": "lonely", "force": true }));
    h.prune();
    assert!(h.exists(&lonely), "the deleted branch's commit was pruned");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("refs/heads/lonely").unwrap(), lonely);

    // An annotated tag is an object of its own, and this one is the only
    // thing that holds its commit as well.
    let tagged = git
        .run([
            "commit-tree",
            "-p",
            "main",
            "-m",
            "held by a tag only",
            &tree,
        ])
        .unwrap()
        .trim()
        .to_owned();
    git.run(["tag", "-a", "-m", "notes", "noted", &tagged])
        .unwrap();
    let object = git.rev_parse("refs/tags/noted").unwrap();
    h.run("tag.delete", json!({ "name": "noted" }));
    h.prune();
    assert!(h.exists(&object) && h.exists(&tagged), "the tag was pruned");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("refs/tags/noted").unwrap(), object);
    // Created here, undone, pruned, redone: the same tag object again.
    let made = h.run(
        "tag.create",
        json!({ "name": "made", "message": "made here" }),
    );
    let value = made["value"].as_str().unwrap().to_owned();
    h.run("undo.last", json!({}));
    h.prune();
    h.run("redo.last", json!({}));
    assert_eq!(git.rev_parse("refs/tags/made").unwrap(), value);
}

/// What HEAD moved away from is in its reflog, which git prunes by age
/// and not by reachability: an amended commit, a commit that was undone
/// and what a reset left behind need no reference of ours, and get none.
#[test]
fn a_popped_stash_and_what_it_overwrote_are_held_against_a_prune() {
    let mut h = Harness::new("branches-merges");
    let git = h.git();
    // A stash that is popped leaves the list, which held it.
    h.scratch.write("a.txt", "a2\nstashed work\n").unwrap();
    h.scratch.write("untracked.txt", "stashed too\n").unwrap();
    let out = h.run(
        "stash.push",
        json!({ "message": "set aside", "include_untracked": true }),
    );
    let stash = out["stash"].as_str().unwrap().to_owned();
    h.run("stash.pop", json!({}));
    assert!(git.rev_parse("refs/stash").is_err(), "the list is empty");
    h.prune();
    assert!(h.exists(&stash), "the popped stash was pruned");
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("refs/stash").unwrap(), stash);
    assert!(!h.scratch.path().join("untracked.txt").exists());
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "");
    h.run("stash.drop", json!({}));

    // Popped over a staged change to another line of the same file, the
    // two are merged: what the file held in between is only in a backup
    // once the index has moved on.
    let lines = |first: &str, last: &str| format!("{first}\n2\n3\n4\n5\n6\n7\n8\n{last}\n");
    h.scratch.write("many.txt", &lines("1", "9")).unwrap();
    git.run(["add", "many.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "many lines"]).unwrap();
    h.scratch.write("many.txt", &lines("1", "stashed")).unwrap();
    h.run("stash.push", json!({}));
    h.scratch.write("many.txt", &lines("local", "9")).unwrap();
    git.run(["add", "many.txt"]).unwrap();
    let between = h.blob_of(&lines("local", "9"));
    h.run("stash.pop", json!({}));
    assert_eq!(h.read("many.txt"), lines("local", "stashed"));
    git.run(["reset", "-q"]).unwrap();
    h.prune();
    assert!(h.exists(&between), "what the file held before was pruned");
    assert!(h.holding().iter().any(|name| name.contains("undo")));
}

#[test]
fn what_head_left_behind_is_held_by_the_reflog() {
    let mut h = Harness::new("branches-merges");
    let git = h.git();
    let original = git.rev_parse("HEAD").unwrap();

    h.scratch.write("a.txt", "a1\namended in\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["a.txt"] }));
    let amended = h.run("commit.amend", json!({ "message": "amended" }));
    let amended = amended["id"].as_str().unwrap().to_owned();
    assert_ne!(amended, original);
    assert!(h.held().is_empty(), "{:?}", h.held());
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), original);
    h.prune();
    h.run("redo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), amended);

    h.run(
        "branch.reset",
        json!({ "target": "HEAD~1", "mode": "keep" }),
    );
    h.prune();
    h.run("undo.last", json!({}));
    assert_eq!(git.rev_parse("HEAD").unwrap(), amended);
    assert!(h.holding().is_empty(), "{:?}", h.holding());
}

#[test]
fn what_the_journal_forgets_is_let_go_and_pruned() {
    let mut h = Harness::new("branches-merges");
    let text = "discarded, undone, and then something else is done\n";
    h.scratch.write("a.txt", text).unwrap();
    let kept = h.blob_of(text);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));
    assert_eq!(h.held().len(), 1);
    h.prune();
    assert!(h.exists(&kept));

    // Undone, the file has its content again, and a redo would make a
    // backup of its own: the entry has no use for this one any more.
    h.run("undo.last", json!({}));
    assert!(h.held().is_empty(), "{:?}", h.held());
    // The content is thrown away by hand, behind the journal's back:
    // now nothing at all has it, and a prune takes it.
    h.git().run(["checkout", "-q", "--", "a.txt"]).unwrap();
    h.prune();
    assert!(!h.exists(&kept), "nothing holds it, and it is gone");

    // A tag that was made holds nothing while it is there, its object
    // once the making is undone (for the redo), and nothing again when a
    // new action ends what could be redone.
    let made = h.run("tag.create", json!({ "name": "noted", "message": "notes" }));
    let object = made["value"].as_str().unwrap().to_owned();
    assert!(h.held().is_empty());
    h.run("undo.last", json!({}));
    assert_eq!(h.held().len(), 1);
    assert_eq!(h.held()[0].1, object);
    h.prune();
    assert!(h.exists(&object));
    h.run("tag.create", json!({ "name": "something-else" }));
    assert!(h.held().is_empty(), "{:?}", h.held());
    h.prune();
    assert!(!h.exists(&object), "the tag object nobody can redo is gone");

    // The journal keeps a hundred entries: the hundred-and-first pushes
    // the oldest out, and what it held with it.
    let first = "the first of many discards\n";
    h.scratch.write("a.txt", first).unwrap();
    let first_blob = h.blob_of(first);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));
    let entry = h.held().iter().map(|(entry, _)| *entry).max().unwrap();
    for n in 0..bitbraid_engine::journal::CAPACITY - 1 {
        h.run("tag.create", json!({ "name": format!("filler-{n}") }));
    }
    assert!(
        h.held().iter().any(|(held, _)| *held == entry),
        "still in the journal, still held"
    );
    h.run("tag.create", json!({ "name": "one-too-many" }));
    assert!(h.held().iter().all(|(held, _)| *held != entry));
    h.prune();
    assert!(!h.exists(&first_blob));
    // Every reference of ours is for an entry that is in the journal.
    let journal = h.run("undo.journal", json!({}));
    let ids: Vec<u64> = ["undo", "redo"]
        .iter()
        .flat_map(|stack| journal[*stack].as_array().unwrap())
        .map(|entry| entry["id"].as_u64().unwrap())
        .collect();
    assert_eq!(ids.len(), bitbraid_engine::journal::CAPACITY);
    assert!(h.held().iter().all(|(entry, _)| ids.contains(entry)));
}

#[test]
fn the_references_that_hold_are_nobodys_branches() {
    let mut h = Harness::new("branches-merges");
    let git = h.git();
    let tree = git.rev_parse("main^{tree}").unwrap();
    let lonely = git
        .run(["commit-tree", "-p", "main", "-m", "lonely", &tree])
        .unwrap()
        .trim()
        .to_owned();
    git.run(["update-ref", "refs/heads/lonely", &lonely])
        .unwrap();
    h.scratch.write("a.txt", "thrown away\n").unwrap();
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));
    h.run("branch.delete", json!({ "name": "lonely", "force": true }));

    // Where they are: under a name of our own, by entry and object.
    let holding = h.holding();
    assert_eq!(holding.len(), 2, "{holding:?}");
    for (reference, (entry, object)) in holding.iter().zip(h.held()) {
        assert_eq!(
            reference,
            &format!("refs/bitbraid/undo/main/{entry}/{object}")
        );
        assert_eq!(git.rev_parse(reference).unwrap(), object);
    }
    // Nothing of the application lists them or walks from them.
    let refs = h.run("repo.refs", json!({}));
    let text = refs.to_string();
    assert!(!text.contains("refs/bitbraid"), "{text}");
    assert!(!text.contains(&lonely), "{text}");
    let log = h.run("repo.log", json!({}));
    assert!(
        log.as_array()
            .unwrap()
            .iter()
            .all(|commit| commit["id"] != lonely.as_str()),
        "a held commit is no row of the graph"
    );
    // Git's own walks take them in their stride: one of them is a blob.
    assert!(git.run(["rev-list", "--all"]).is_ok());
    assert!(git.run(["log", "--all", "--oneline"]).is_ok());
    assert!(git.run(["fsck", "--no-dangling"]).is_ok());
    // Packed away by a gc, they are found and removed all the same.
    h.prune();
    assert!(
        !h.scratch
            .path()
            .join(".git/refs/bitbraid")
            .join("undo/main")
            .exists()
            || h.holding().len() == 2
    );
    assert_eq!(h.holding().len(), 2);
    h.run("undo.last", json!({}));
    h.run("undo.last", json!({}));
    h.run("tag.create", json!({ "name": "moving-on" }));
    assert!(h.holding().is_empty(), "{:?}", h.holding());
    assert!(
        std::fs::read_to_string(h.scratch.path().join(".git/packed-refs"))
            .unwrap_or_default()
            .lines()
            .all(|line| !line.contains("refs/bitbraid")),
    );
}

#[test]
fn a_linked_worktree_holds_its_own_and_loses_it_when_it_is_gone() {
    let mut h = Harness::new("branches-merges");
    let git = h.git();
    let tree = h.scratch.path().with_extension("tree");
    let _ = std::fs::remove_dir_all(&tree);
    git.run([
        "worktree",
        "add",
        "-q",
        tree.to_str().unwrap(),
        "-b",
        "tree",
    ])
    .unwrap();
    let tree_git = Git::new(&tree, fixtures_out_dir().join(".home"));

    // A discard in each: the journals are two, and so are the parts of
    // the namespace.
    let (in_main, in_tree) = (
        "discarded in the main worktree\n",
        "discarded in the linked one\n",
    );
    h.scratch.write("a.txt", in_main).unwrap();
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));
    std::fs::write(tree.join("a.txt"), in_tree).unwrap();
    let mut linked = CommandContext::default();
    h.registry
        .run("repo.open", &mut linked, &json!({ "path": tree }))
        .unwrap();
    h.registry
        .run("discard.paths", &mut linked, &json!({ "paths": ["a.txt"] }))
        .unwrap();
    let holding = h.holding();
    assert_eq!(holding.len(), 2, "{holding:?}");
    assert!(holding[0].starts_with("refs/bitbraid/undo/main/1/"));
    assert!(holding[1].starts_with("refs/bitbraid/undo/worktrees/"));
    assert_eq!(h.held().len(), 1, "each sees its own");

    // A prune from either worktree keeps both (which a reference under
    // refs/worktree/ would not do for the other one).
    h.prune();
    tree_git.run(["gc", "-q", "--prune=now"]).unwrap();
    h.registry
        .run("undo.last", &mut linked, &json!({}))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(tree.join("a.txt")).unwrap(),
        in_tree
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.read("a.txt"), in_main);

    // The worktree is removed: what was held for its journal is cleared
    // away the next time a journal is written.
    assert!(h.holding().is_empty(), "undone, neither holds anything");
    h.registry
        .run("redo.last", &mut linked, &json!({}))
        .unwrap();
    let holding = h.holding();
    assert_eq!(holding.len(), 1, "{holding:?}");
    assert!(holding[0].starts_with("refs/bitbraid/undo/worktrees/"));
    drop(linked);
    git.run(["worktree", "remove", "--force", tree.to_str().unwrap()])
        .unwrap();
    h.run("redo.last", json!({}));
    let holding = h.holding();
    assert_eq!(holding.len(), 1, "{holding:?}");
    assert!(holding[0].starts_with("refs/bitbraid/undo/main/"));
    let _ = std::fs::remove_dir_all(&tree);
}

/// When somebody takes our references away, a prune takes the backup:
/// the undo is refused and says why, and nothing is half done.
#[test]
fn without_the_reference_a_prune_takes_the_backup_and_undo_says_so() {
    let mut h = Harness::new("branches-merges");
    let text = "discarded, and then let go by hand\n";
    h.scratch.write("a.txt", text).unwrap();
    h.scratch.write("b.txt", "kept through it all\n").unwrap();
    let kept = h.blob_of(text);
    h.run("discard.paths", json!({ "paths": ["a.txt"] }));
    for reference in h.holding() {
        h.git().run(["update-ref", "-d", &reference]).unwrap();
    }
    h.prune();
    assert!(
        !h.exists(&kept),
        "this is the weakness the references close"
    );
    let after = h.read("a.txt");
    let err = h.try_run("undo.last", json!({})).unwrap_err();
    assert!(!err.is_empty());
    assert_eq!(h.read("a.txt"), after);
    assert_eq!(h.read("b.txt"), "kept through it all\n");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Discard a.txt");
}
