//! Reword, drop and squash where a plain list of commits does not lead up
//! to HEAD: across merge commits, of a merge commit, from the first commit
//! of the history. Each against the same rewrite made by hand with git on
//! a twin copy: with `commit-tree` where the trees stay as they are (every
//! commit made again on its new parents), with `cherry-pick` and `merge`
//! where they do not. Never with a rebase.

use std::collections::HashMap;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fnv1a64};
use serde_json::{Value, json};

/// How the side line is merged.
#[derive(Clone, Copy, PartialEq)]
enum Merged {
    /// Git merges it by itself.
    Clean,
    /// Both lines changed `both.txt`, and the merge says what it is to be.
    ByHand,
    /// Merged by itself, and then a file added to the merge commit.
    Amended,
}

/// On top of the fixture: `one`, `two`; a side line `s1`, `s2` from there;
/// `three` on the main line; the merge of the side line; `four`, `five`.
fn work(copy: &Scratch, merged: Merged) {
    let git = copy.git();
    git.run(["config", "user.name", "Ada Fixture"]).unwrap();
    git.run(["config", "user.email", "ada@fixture.invalid"])
        .unwrap();
    let commit = |file: &str, content: &str, message: &str| {
        copy.write(file, content).unwrap();
        git.run(["add", file]).unwrap();
        git.run(["commit", "-q", "--cleanup=verbatim", "-m", message])
            .unwrap();
    };
    commit("one.txt", "1\n", "one");
    commit("two.txt", "2\n", "two\n\nbody of two");
    git.run(["switch", "-q", "-c", "side"]).unwrap();
    commit("s.txt", "s1\n", "s1");
    if merged == Merged::ByHand {
        commit("both.txt", "from the side\n", "s2");
    } else {
        commit("s.txt", "s1\ns2\n", "s2");
    }
    git.run(["switch", "-q", "main"]).unwrap();
    if merged == Merged::ByHand {
        commit("both.txt", "from the main line\n", "three");
        assert!(git.run(["merge", "-q", "--no-ff", "side"]).is_err());
        copy.write("both.txt", "neither: said by hand\n").unwrap();
        git.run(["add", "both.txt"]).unwrap();
        git.run(["commit", "-q", "-m", "Merge branch 'side'"])
            .unwrap();
    } else {
        commit("three.txt", "3\n", "three");
        git.run([
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'side'",
            "side",
        ])
        .unwrap();
    }
    if merged == Merged::Amended {
        copy.write("extra.txt", "only in the merge\n").unwrap();
        git.run(["add", "extra.txt"]).unwrap();
        git.run(["commit", "-q", "--amend", "--no-edit"]).unwrap();
    }
    commit("four.txt", "4\n", "four");
    commit("five.txt", "5\n", "five");
}

/// The commit with this subject in the history of HEAD.
fn id_of(git: &Git, subject: &str) -> String {
    git.lines(["log", "--format=%H %s", "HEAD"])
        .unwrap()
        .into_iter()
        .find_map(|line| {
            let (id, s) = line.split_once(' ')?;
            (s == subject).then(|| id.to_owned())
        })
        .unwrap_or_else(|| panic!("no commit called {subject:?}"))
}

struct Commit {
    tree: String,
    parents: Vec<String>,
    /// The author's name, address and date, as git takes them back.
    who: [String; 3],
    message: String,
}

/// Every commit in the history of `tip`.
fn commits(git: &Git, tip: &str) -> HashMap<String, Commit> {
    git.run([
        "log",
        "--date=raw",
        "--format=%H%x01%T%x01%P%x01%an%x01%ae%x01%ad%x01%B%x00",
        tip,
    ])
    .unwrap()
    .split('\0')
    .filter_map(|record| {
        let mut parts = record.trim_start_matches('\n').splitn(7, '\u{1}');
        let id = parts.next().filter(|id| !id.is_empty())?.to_owned();
        Some((
            id,
            Commit {
                tree: parts.next()?.to_owned(),
                parents: parts
                    .next()?
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect(),
                who: [
                    parts.next()?.to_owned(),
                    parts.next()?.to_owned(),
                    parts.next()?.to_owned(),
                ],
                message: parts.next()?.to_owned(),
            },
        ))
    })
    .collect()
}

/// Write a commit with `commit-tree`: this tree, these parents, this
/// message, and the author another commit has.
fn commit_tree(git: &Git, tree: &str, parents: &[String], message: &str, by: &Commit) -> String {
    let mut args = vec!["commit-tree".to_owned(), tree.to_owned()];
    for parent in parents {
        args.extend(["-p".to_owned(), parent.clone()]);
    }
    args.extend(["-m".to_owned(), message.trim_end().to_owned()]);
    git.run_with(
        args,
        &[
            ("GIT_AUTHOR_NAME", by.who[0].as_str()),
            ("GIT_AUTHOR_EMAIL", by.who[1].as_str()),
            ("GIT_AUTHOR_DATE", by.who[2].as_str()),
        ],
    )
    .unwrap()
    .trim()
    .to_owned()
}

/// Everything about a history that does not depend on when it was made
/// again: for each commit its tree, its author with the author date, its
/// whole message and the same of its parents, in their order.
fn shape(git: &Git) -> String {
    fn of(id: &str, all: &HashMap<String, Commit>, known: &mut HashMap<String, u64>) -> u64 {
        if let Some(found) = known.get(id) {
            return *found;
        }
        let commit = &all[id];
        let mut text = format!("{}\n{:?}\n{}", commit.tree, commit.who, commit.message);
        for parent in &commit.parents {
            text.push_str(&format!("\n{:x}", of(parent, all, known)));
        }
        let hash = fnv1a64(text.as_bytes());
        known.insert(id.to_owned(), hash);
        hash
    }
    let all = commits(git, "HEAD");
    let head = git.rev_parse("HEAD").unwrap();
    // The hash says whether two are the same; the subjects, first parents
    // first, say where they are not.
    let subjects = git
        .lines(["log", "--first-parent", "--format=%p|%s", "HEAD"])
        .unwrap()
        .into_iter()
        .map(|line| {
            let (parents, subject) = line.split_once('|').unwrap();
            format!("{subject}/{}", parents.split_whitespace().count())
        })
        .collect::<Vec<_>>()
        .join(", ");
    format!("{:x} [{subjects}]", of(&head, &all, &mut HashMap::new()))
}

/// What is done to a commit when the history is made again by hand.
enum Hand {
    /// Another message.
    Message(&'static str),
    /// It goes into the commit before it, which gets its tree.
    Into,
}

/// Make the history of the twin again with `commit-tree`: every commit
/// whose parents or message changed is written anew with the tree, the
/// author and the message it has, on the commits its parents became. A
/// commit that goes into the one before it gives that one its tree and
/// then stands for it. The branch is moved to the result.
fn by_hand(twin: &Scratch, edits: &[(&str, Hand)], messages: &[(&str, &str)]) {
    let git = twin.git();
    let all = commits(&git, "HEAD");
    let order = git
        .lines(["rev-list", "--topo-order", "--reverse", "HEAD"])
        .unwrap();
    let subject = |id: &str| all[id].message.lines().next().unwrap_or("").to_owned();
    let mut became: HashMap<String, String> = HashMap::new();
    // A commit that went into another: the one that stays for it.
    let mut stays: HashMap<String, String> = HashMap::new();
    for id in &order {
        let commit = &all[id];
        let edit = edits
            .iter()
            .find(|(s, _)| *s == subject(id))
            .map(|(_, e)| e);
        if matches!(edit, Some(Hand::Into)) {
            // Made as the commit before it (or the one that one went
            // into), with this one's tree.
            let before = &commit.parents[0];
            let kept_id = stays.get(before).unwrap_or(before).clone();
            let kept = &all[&kept_id];
            let message = messages
                .iter()
                .find(|(s, _)| *s == subject(&kept_id))
                .map(|(_, m)| (*m).to_owned())
                .unwrap_or_else(|| kept.message.clone());
            let parents: Vec<String> = kept.parents.iter().map(|p| became[p].clone()).collect();
            let made = commit_tree(&git, &commit.tree, &parents, &message, kept);
            became.insert(kept_id.clone(), made.clone());
            became.insert(id.clone(), made);
            stays.insert(id.clone(), kept_id);
            continue;
        }
        let parents: Vec<String> = commit.parents.iter().map(|p| became[p].clone()).collect();
        let message = match edit {
            Some(Hand::Message(message)) => Some((*message).to_owned()),
            _ => None,
        };
        if parents == commit.parents && message.is_none() {
            became.insert(id.clone(), id.clone());
            continue;
        }
        let message = message.unwrap_or_else(|| commit.message.clone());
        became.insert(
            id.clone(),
            commit_tree(&git, &commit.tree, &parents, &message, commit),
        );
    }
    let head = git.rev_parse("HEAD").unwrap();
    git.run(["reset", "-q", "--hard", became[&head].as_str()])
        .unwrap();
}

struct Harness {
    scratch: Scratch,
    twin: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(merged: Merged) -> Self {
        Self::with(|copy| work(copy, merged))
    }
    fn with(work: impl Fn(&Scratch)) -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let twin = Scratch::of("linear").unwrap();
        work(&scratch);
        work(&twin);
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            twin,
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
    fn id(&self, subject: &str) -> String {
        id_of(&self.git(), subject)
    }
    fn assert_same(&self, what: &str) {
        let (ours, theirs) = (self.git(), self.twin.git());
        assert_eq!(shape(&ours), shape(&theirs), "{what}: history");
        assert_eq!(
            ours.run(["status", "--porcelain"]).unwrap(),
            theirs.run(["status", "--porcelain"]).unwrap(),
            "{what}: status"
        );
        assert_eq!(
            ours.run(["ls-files", "--stage"]).unwrap(),
            theirs.run(["ls-files", "--stage"]).unwrap(),
            "{what}: index"
        );
        assert_eq!(
            ours.run(["symbolic-ref", "HEAD"]).unwrap(),
            "refs/heads/main\n",
            "{what}: still on the branch"
        );
        assert!(
            ours.refs("refs/rewritten/").unwrap().is_empty(),
            "{what}: no label is left"
        );
        assert!(
            !self.scratch.path().join(".git/bitbraid/rebase").exists(),
            "{what}: nothing of the plan is left"
        );
    }
    /// Nothing moved and nothing is in progress.
    fn assert_untouched(&mut self, start: &str, what: &str) {
        assert_eq!(self.git().rev_parse("HEAD").unwrap(), start, "{what}");
        assert_eq!(
            self.git().run(["status", "--porcelain"]).unwrap(),
            "",
            "{what}"
        );
        assert!(
            self.run("op.status", json!({}))["operation"].is_null(),
            "{what}"
        );
        assert!(
            !self.scratch.path().join(".git/bitbraid/rebase").exists(),
            "{what}: nothing of the plan is left"
        );
    }
    /// The last step is one step for undo, and comes back with redo.
    fn undo_and_redo(&mut self, start: &str, label: &str) {
        assert_eq!(self.run("undo.peek", json!({}))["undo"], label);
        let after = self.git().rev_parse("HEAD").unwrap();
        self.run("undo.last", json!({}));
        assert_eq!(self.git().rev_parse("HEAD").unwrap(), start, "undone");
        assert_eq!(self.git().run(["status", "--porcelain"]).unwrap(), "");
        self.run("redo.last", json!({}));
        assert_eq!(self.git().rev_parse("HEAD").unwrap(), after, "redone");
    }
}

#[test]
fn the_way_a_commit_can_be_rewritten_is_told() {
    let mut h = Harness::new(Merged::Clean);
    let git = h.git();
    let root = git
        .run(["rev-list", "--max-parents=0", "HEAD"])
        .unwrap()
        .trim()
        .to_owned();
    let merge = h.id("Merge branch 'side'");
    for (commit, plain, is_merge, first) in [
        (h.id("five"), true, false, false),
        (h.id("four"), true, false, false),
        (merge.clone(), false, true, false),
        (h.id("three"), false, false, false),
        (h.id("s1"), false, false, false),
        (h.id("two"), false, false, false),
        (root.clone(), false, false, true),
    ] {
        let way = h.run("commit.rewrite_way", json!({ "commit": commit }));
        assert_eq!(way["plain"], plain, "{way}");
        assert_eq!(way["merge"], is_merge, "{way}");
        assert_eq!(way["first"], first, "{way}");
        assert_eq!(way["commit"], commit);
        // What says "plain" is what an interactive rebase can be planned
        // from, and the other way round.
        assert_eq!(
            h.try_run("rebase.plan", json!({ "first": commit })).is_ok(),
            plain,
            "{commit}"
        );
    }
    let two = h.id("two");
    let way = h.run("commit.rewrite_way", json!({ "commit": two }));
    let alone = &way["alone"];
    assert_eq!(alone["branch"], "main");
    assert!(alone["upstream"].is_null());
    assert_eq!(alone["base"], h.id("one"));
    assert_eq!(alone["commits"].as_array().unwrap().len(), 1);
    let entry = &alone["commits"][0];
    assert_eq!(entry["id"], two);
    assert_eq!(entry["summary"], "two");
    assert_eq!(entry["message"], "two\n\nbody of two");
    assert_eq!(entry["author"], "Ada Fixture");
    assert_eq!(entry["published"], false);
    // What the upstream has already is marked.
    git.run(["branch", "up", two.as_str()]).unwrap();
    git.run(["branch", "-q", "--set-upstream-to=up"]).unwrap();
    for (subject, published) in [("two", true), ("one", true), ("three", false)] {
        let way = h.run("commit.rewrite_way", json!({ "commit": h.id(subject) }));
        assert_eq!(way["alone"]["upstream"], "up");
        assert_eq!(way["alone"]["commits"][0]["published"], published);
    }
    // Nothing is written by asking.
    assert!(git.refs("refs/rewritten/").unwrap().is_empty());
    for bad in [
        json!({}),
        json!({ "commit": "" }),
        json!({ "commit": "--root" }),
        json!({ "commit": "no-such" }),
        json!({ "commit": 7 }),
        json!({ "commit": "a\nb" }),
        json!({ "commit": "\0" }),
    ] {
        assert!(
            h.try_run("commit.rewrite_way", bad.clone()).is_err(),
            "{bad}"
        );
    }
}

#[test]
fn a_reword_before_a_merge_keeps_the_merge_to_the_letter() {
    for merged in [Merged::Clean, Merged::ByHand, Merged::Amended] {
        let mut h = Harness::new(merged);
        let start = h.git().rev_parse("HEAD").unwrap();
        let merge_tree = h
            .git()
            .rev_parse(&format!("{}^{{tree}}", h.id("Merge branch 'side'")))
            .unwrap();
        let one = h.id("one");
        let message = "ONE\n\nwith a body\n# and a line like a comment";
        let out = h.run(
            "commit.reword",
            json!({ "commit": one, "message": message }),
        );
        assert!(out.get("stopped").is_none(), "{out}");
        by_hand(&h.twin, &[("one", Hand::Message(message))], &[]);
        h.assert_same("reword before the merge");
        // The merge is the merge that was: what was said by hand, and what
        // was added to it afterwards, are in it.
        let merge = h.id("Merge branch 'side'");
        assert_eq!(
            h.git().rev_parse(&format!("{merge}^{{tree}}")).unwrap(),
            merge_tree
        );
        assert_eq!(
            h.git()
                .run(["log", "-1", "--format=%B", h.id("ONE").as_str()])
                .unwrap(),
            format!("{message}\n\n")
        );
        h.undo_and_redo(&start, &format!("Reword {}", &one[..7]));
        h.assert_same("redo");
    }
}

#[test]
fn a_reword_on_the_side_line_leaves_the_main_line_before_the_merge_alone() {
    let mut h = Harness::new(Merged::ByHand);
    let start = h.git().rev_parse("HEAD").unwrap();
    let (s1, three) = (h.id("s1"), h.id("three"));
    h.run(
        "commit.reword",
        json!({ "commit": s1, "message": "s1, said again" }),
    );
    by_hand(&h.twin, &[("s1", Hand::Message("s1, said again"))], &[]);
    h.assert_same("reword on the side line");
    assert_eq!(h.id("three"), three, "not made again");
    h.undo_and_redo(&start, &format!("Reword {}", &s1[..7]));
}

#[test]
fn a_merge_commit_is_reworded() {
    for merged in [Merged::Clean, Merged::ByHand, Merged::Amended] {
        let mut h = Harness::new(merged);
        let start = h.git().rev_parse("HEAD").unwrap();
        let merge = h.id("Merge branch 'side'");
        h.run(
            "commit.reword",
            json!({ "commit": merge, "message": "The side line comes in\n\nAs agreed." }),
        );
        // By hand: amend it where it is, then the two after it.
        let twin = h.twin.git();
        let (m, four, five) = (
            id_of(&twin, "Merge branch 'side'"),
            id_of(&twin, "four"),
            id_of(&twin, "five"),
        );
        twin.run(["reset", "-q", "--hard", m.as_str()]).unwrap();
        twin.run([
            "commit",
            "-q",
            "--amend",
            "-m",
            "The side line comes in\n\nAs agreed.",
        ])
        .unwrap();
        twin.run(["cherry-pick", four.as_str(), five.as_str()])
            .unwrap();
        h.assert_same("reword of the merge");
        assert_eq!(
            h.git()
                .lines(["log", "-1", "--format=%p", "HEAD~2"])
                .unwrap()[0]
                .split_whitespace()
                .count(),
            2,
            "still a merge"
        );
        h.undo_and_redo(&start, &format!("Reword {}", &merge[..7]));
    }
}

#[test]
fn the_first_commit_of_the_history_is_reworded() {
    let mut h = Harness::new(Merged::ByHand);
    let start = h.git().rev_parse("HEAD").unwrap();
    let root = h.id("add readme");
    h.run(
        "commit.reword",
        json!({ "commit": root, "message": "The beginning" }),
    );
    by_hand(
        &h.twin,
        &[("add readme", Hand::Message("The beginning"))],
        &[],
    );
    h.assert_same("reword of the first commit");
    let roots = h
        .git()
        .lines(["rev-list", "--max-parents=0", "HEAD"])
        .unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(h.id("The beginning"), roots[0]);
    h.undo_and_redo(&start, &format!("Reword {}", &root[..7]));

    // And without a merge anywhere: the fixture as it is.
    let mut h = Harness::with(|_| {});
    let root = h.id("add readme");
    h.run(
        "commit.reword",
        json!({ "commit": root, "message": "The beginning" }),
    );
    by_hand(
        &h.twin,
        &[("add readme", Hand::Message("The beginning"))],
        &[],
    );
    h.assert_same("reword of the first commit of a plain history");
}

#[test]
fn commits_are_squashed_with_merges_after_them() {
    // One into its parent, before the merge.
    for merged in [Merged::Clean, Merged::ByHand, Merged::Amended] {
        let mut h = Harness::new(merged);
        let start = h.git().rev_parse("HEAD").unwrap();
        let two = h.id("two");
        h.run("commit.squash", json!({ "commits": [two] }));
        by_hand(
            &h.twin,
            &[("two", Hand::Into)],
            &[("one", "one\n\ntwo\n\nbody of two")],
        );
        h.assert_same("squash of one before the merge");
        h.undo_and_redo(&start, "Squash 2 commits");
    }

    // Several, named in any order; into the first commit of all; and into
    // a merge commit, which stays a merge.
    let mut h = Harness::new(Merged::ByHand);
    let (one, two, a) = (h.id("one"), h.id("two"), h.id("update b"));
    h.run("commit.squash", json!({ "commits": [two, a, one] }));
    by_hand(
        &h.twin,
        &[("one", Hand::Into), ("two", Hand::Into)],
        &[("update b", "update b\n\none\n\ntwo\n\nbody of two")],
    );
    h.assert_same("squash of three");
    assert_eq!(h.run("undo.peek", json!({}))["undo"], "Squash 3 commits");

    let second = h.id("add a");
    h.run("commit.squash", json!({ "commits": [second] }));
    by_hand(
        &h.twin,
        &[("add a", Hand::Into)],
        &[("add readme", "add readme\n\nadd a")],
    );
    h.assert_same("squash into the first commit");

    let four = h.id("four");
    h.run("commit.squash", json!({ "commits": [four] }));
    by_hand(
        &h.twin,
        &[("four", Hand::Into)],
        &[("Merge branch 'side'", "Merge branch 'side'\n\nfour")],
    );
    h.assert_same("squash into the merge");
    let merge = h.id("Merge branch 'side'");
    assert_eq!(
        h.git()
            .run([
                "show",
                "--format=",
                "--name-only",
                "-m",
                "--first-parent",
                merge.as_str()
            ])
            .unwrap()
            .lines()
            .filter(|line| *line == "four.txt")
            .count(),
        1,
        "the merge has what was squashed into it"
    );
}

#[test]
fn a_squash_that_cannot_be_exact_is_refused() {
    let mut h = Harness::new(Merged::Clean);
    let start = h.git().rev_parse("HEAD").unwrap();
    // The merge itself into its parent.
    let merge = h.id("Merge branch 'side'");
    let err = h
        .try_run("commit.squash", json!({ "commits": [merge] }))
        .unwrap_err();
    assert!(
        err.contains("is a merge commit and cannot be squashed"),
        "{err}"
    );
    h.assert_untouched(&start, "squash of the merge");
    // A merge among several.
    let err = h
        .try_run(
            "commit.squash",
            json!({ "commits": [h.id("three"), merge, h.id("four")] }),
        )
        .unwrap_err();
    assert!(
        err.contains("is a merge commit and cannot be squashed"),
        "{err}"
    );
    // Into a commit that another line starts from.
    for commits in [json!([h.id("three")]), json!([h.id("s1")])] {
        let err = h
            .try_run("commit.squash", json!({ "commits": commits }))
            .unwrap_err();
        assert!(err.contains("another line of the history starts"), "{err}");
        h.assert_untouched(&start, "squash into a branch point");
    }
    // The first commit of all has nothing before it.
    let root = h.id("add readme");
    let err = h
        .try_run("commit.squash", json!({ "commits": [root] }))
        .unwrap_err();
    assert!(err.contains("first commit of the history"), "{err}");
    // Not following one another, across the merge.
    for commits in [
        json!([h.id("three"), h.id("four")]),
        json!([h.id("two"), h.id("four")]),
        json!([h.id("one"), h.id("one")]),
        json!([h.id("one"), "no-such"]),
        json!([h.id("one"), "--root"]),
    ] {
        assert!(
            h.try_run("commit.squash", json!({ "commits": commits }))
                .is_err(),
            "{commits}"
        );
        h.assert_untouched(&start, "hostile squash");
    }
}

#[test]
fn a_drop_before_a_merge_merges_anew() {
    let mut h = Harness::new(Merged::Clean);
    let start = h.git().rev_parse("HEAD").unwrap();
    let one = h.id("one");
    let out = h.run("commit.drop", json!({ "commit": one }));
    assert!(out.get("stopped").is_none(), "{out}");
    // By hand: the commits picked one by one, and the side line merged.
    let twin = h.twin.git();
    let t: HashMap<&str, String> = ["one", "two", "s1", "s2", "three", "four", "five"]
        .into_iter()
        .map(|subject| (subject, id_of(&twin, subject)))
        .collect();
    twin.run(["reset", "-q", "--hard", &format!("{}^", t["one"])])
        .unwrap();
    twin.run(["cherry-pick", t["two"].as_str()]).unwrap();
    twin.run(["switch", "-q", "-c", "side-again"]).unwrap();
    twin.run(["cherry-pick", t["s1"].as_str(), t["s2"].as_str()])
        .unwrap();
    twin.run(["switch", "-q", "main"]).unwrap();
    twin.run(["cherry-pick", t["three"].as_str()]).unwrap();
    twin.run([
        "merge",
        "-q",
        "--no-ff",
        "-m",
        "Merge branch 'side'",
        "side-again",
    ])
    .unwrap();
    twin.run(["cherry-pick", t["four"].as_str(), t["five"].as_str()])
        .unwrap();
    h.assert_same("drop before the merge");
    assert!(!h.scratch.path().join("one.txt").exists());
    h.undo_and_redo(&start, &format!("Drop {}", &one[..7]));
    assert!(!h.scratch.path().join("one.txt").exists());
}

#[test]
fn a_merge_commit_is_dropped() {
    let mut h = Harness::new(Merged::Clean);
    let start = h.git().rev_parse("HEAD").unwrap();
    let merge = h.id("Merge branch 'side'");
    h.run("commit.drop", json!({ "commit": merge }));
    let twin = h.twin.git();
    let (three, four, five) = (
        id_of(&twin, "three"),
        id_of(&twin, "four"),
        id_of(&twin, "five"),
    );
    twin.run(["reset", "-q", "--hard", three.as_str()]).unwrap();
    twin.run(["cherry-pick", four.as_str(), five.as_str()])
        .unwrap();
    h.assert_same("drop of the merge");
    assert!(!h.scratch.path().join("s.txt").exists());
    h.undo_and_redo(&start, &format!("Drop {}", &merge[..7]));
}

#[test]
fn a_drop_that_a_later_merge_could_not_follow_is_refused() {
    for (merged, why) in [
        (Merged::ByHand, "was resolved by hand"),
        (Merged::Amended, "holds changes of its own"),
    ] {
        let mut h = Harness::new(merged);
        let start = h.git().rev_parse("HEAD").unwrap();
        let merge = h.id("Merge branch 'side'");
        for subject in ["one", "s1", "add readme"] {
            let err = h
                .try_run("commit.drop", json!({ "commit": h.id(subject) }))
                .unwrap_err();
            assert!(err.contains(why), "{subject}: {err}");
            assert!(err.contains(&merge[..7]), "{subject}: {err}");
            h.assert_untouched(&start, subject);
        }
        assert!(h.run("undo.peek", json!({}))["undo"].is_null());
        // After the merge nothing is in the way, and the merge itself goes
        // like any commit.
        h.run("commit.drop", json!({ "commit": h.id("four") }));
        h.run(
            "commit.drop",
            json!({ "commit": h.id("Merge branch 'side'") }),
        );
        assert_eq!(
            h.git().lines(["log", "--format=%s", "-2"]).unwrap(),
            ["five", "three"]
        );
    }
}

#[test]
fn the_first_commit_of_the_history_is_dropped() {
    let mut h = Harness::with(|_| {});
    let start = h.git().rev_parse("HEAD").unwrap();
    let root = h.id("add readme");
    h.run("commit.drop", json!({ "commit": root }));
    // By hand: a branch with nothing on it, and the others picked onto it.
    let twin = h.twin.git();
    let rest: Vec<String> = ["add a", "update a", "add b", "update b"]
        .iter()
        .map(|subject| id_of(&twin, subject))
        .collect();
    twin.run(["switch", "-q", "--orphan", "anew"]).unwrap();
    for id in &rest {
        twin.run(["cherry-pick", id.as_str()]).unwrap();
    }
    twin.run(["branch", "-q", "-M", "main"]).unwrap();
    h.assert_same("drop of the first commit");
    assert!(!h.scratch.path().join("README.md").exists());
    assert_eq!(h.git().rev_list_count("HEAD").unwrap(), 4);
    h.undo_and_redo(&start, &format!("Drop {}", &root[..7]));

    // The only commit there is cannot go.
    let h2 = Scratch::of("linear").unwrap();
    let git = h2.git();
    git.run(["switch", "-q", "--orphan", "alone"]).unwrap();
    h2.write("only.txt", "x\n").unwrap();
    git.run(["add", "only.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "only"]).unwrap();
    let registry = builtin_registry();
    let mut ctx = CommandContext::default();
    registry
        .run("repo.open", &mut ctx, &json!({ "path": h2.path() }))
        .unwrap();
    let only = git.rev_parse("HEAD").unwrap();
    let err = registry
        .run("commit.drop", &mut ctx, &json!({ "commit": only }))
        .unwrap_err()
        .to_string();
    assert!(err.contains("the only commit"), "{err}");
    assert_eq!(git.rev_parse("HEAD").unwrap(), only);
}

#[test]
fn a_drop_that_conflicts_stops_and_is_aborted_or_finished() {
    // `late` changes the file `one` added: without `one` it has nothing to
    // change.
    let late = |copy: &Scratch| {
        work(copy, Merged::Clean);
        copy.write("one.txt", "1\nlate\n").unwrap();
        copy.git().run(["add", "one.txt"]).unwrap();
        copy.git().run(["commit", "-q", "-m", "late"]).unwrap();
    };
    let mut h = Harness::with(late);
    let start = h.git().rev_parse("HEAD").unwrap();
    let one = h.id("one");
    let out = h.run("commit.drop", json!({ "commit": one }));
    assert!(out["stopped"].is_string(), "{out}");
    assert_eq!(out["conflicts"], json!(["one.txt"]));
    let status = h.run("op.status", json!({}));
    assert_eq!(status["operation"]["kind"], "rebase", "{status}");
    h.run("op.abort", json!({}));
    h.assert_untouched(&start, "aborted");
    assert!(h.git().refs("refs/rewritten/").unwrap().is_empty());

    // Again, and this time the file stays out and the rebase goes on.
    let out = h.run("commit.drop", json!({ "commit": one }));
    assert!(out["stopped"].is_string(), "{out}");
    h.git().run(["rm", "-q", "--", "one.txt"]).unwrap();
    let done = h.run("op.continue", json!({}));
    assert_eq!(done["finished"], true, "{done}");
    assert!(!h.scratch.path().join("one.txt").exists());
    assert!(h.git().refs("refs/rewritten/").unwrap().is_empty());
    assert_eq!(
        h.git()
            .lines(["log", "--first-parent", "--format=%s", "-4"])
            .unwrap(),
        ["five", "four", "Merge branch 'side'", "three"]
    );
    // One step for undo, as the whole of it.
    assert_eq!(
        h.run("undo.peek", json!({}))["undo"],
        format!("Drop {}", &one[..7])
    );
    h.run("undo.last", json!({}));
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), start);
}

#[test]
fn hostile_changes_across_a_merge_change_nothing() {
    let mut h = Harness::new(Merged::Clean);
    let git = h.git();
    let start = git.rev_parse("HEAD").unwrap();
    let one = h.id("one");
    for message in [
        json!(""),
        json!(" \n\t\n"),
        json!("with a \0 in it"),
        json!(7),
    ] {
        assert!(
            h.try_run(
                "commit.reword",
                json!({ "commit": one, "message": message })
            )
            .is_err(),
            "{message}"
        );
        h.assert_untouched(&start, "hostile message");
    }
    assert!(
        h.try_run(
            "commit.reword",
            json!({ "commit": one, "message": "x".repeat((1 << 20) + 1) })
        )
        .is_err()
    );
    // Local changes in the way: git says so, and nothing is begun.
    h.scratch.write("one.txt", "changed\n").unwrap();
    let err = h
        .try_run("commit.drop", json!({ "commit": one }))
        .unwrap_err();
    assert!(err.contains("unstaged changes"), "{err}");
    assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), " M one.txt\n");
    assert!(!h.scratch.path().join(".git/bitbraid/rebase").exists());
    git.run(["checkout", "-q", "--", "one.txt"]).unwrap();

    // A commit that is not in the history of the branch.
    let s2 = h.id("s2");
    git.run(["switch", "-q", "-c", "other", s2.as_str()])
        .unwrap();
    h.scratch.write("other.txt", "o\n").unwrap();
    git.run(["add", "other.txt"]).unwrap();
    git.run(["commit", "-q", "-m", "Merge elsewhere"]).unwrap();
    let elsewhere = git.rev_parse("HEAD").unwrap();
    git.run(["switch", "-q", "main"]).unwrap();
    for (id, params) in [
        ("commit.drop", json!({ "commit": elsewhere })),
        (
            "commit.reword",
            json!({ "commit": elsewhere, "message": "m" }),
        ),
        ("commit.squash", json!({ "commits": [elsewhere] })),
    ] {
        assert!(h.try_run(id, params).is_err(), "{id}");
        h.assert_untouched(&start, id);
    }

    // Not on a branch.
    git.run(["switch", "-q", "--detach"]).unwrap();
    for (id, params) in [
        ("commit.drop", json!({ "commit": one })),
        ("commit.reword", json!({ "commit": one, "message": "m" })),
        ("commit.squash", json!({ "commits": [h.id("two")] })),
    ] {
        let err = h.try_run(id, params).unwrap_err();
        assert!(err.contains("not on a branch"), "{id}: {err}");
        assert_eq!(git.rev_parse("HEAD").unwrap(), start);
    }
}
