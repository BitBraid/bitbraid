//! Blame before a commit, against `git blame
//! --porcelain`: the name a line's file has in the commit it came from
//! (`filename`), and where the lines were before that commit
//! (`previous`), across a rename, a rename made with changes, and the
//! commit that added the file.

use std::collections::BTreeMap;

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

const FIRST: &str = "original.txt";
const SECOND: &str = "renamed árvíz.txt";
const LAST: &str = "moved/final name.txt";

/// What git says of one line: its commit, the file's name there, and
/// where the lines were before it.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Said {
    commit: String,
    filename: String,
    previous: Option<(String, String)>,
}

/// `git blame --porcelain` of `path` as of HEAD, a line at a time.
fn git_says(git: &Git, path: &str, extra: &[&str]) -> Vec<Said> {
    let mut args = vec!["-c", "core.quotePath=false", "blame", "--porcelain"];
    args.extend_from_slice(extra);
    args.extend_from_slice(&["HEAD", "--", path]);
    let text = git.run(args).unwrap();
    let mut out = Vec::new();
    let mut known: BTreeMap<String, (String, Option<(String, String)>)> = BTreeMap::new();
    let mut current: Option<String> = None;
    for line in text.lines() {
        if line.starts_with('\t') {
            let commit = current.take().unwrap();
            let (filename, previous) = known[&commit].clone();
            out.push(Said {
                commit,
                filename,
                previous,
            });
        } else if let Some(commit) = &current {
            let entry = known.entry(commit.clone()).or_default();
            if let Some(name) = line.strip_prefix("filename ") {
                entry.0 = name.to_owned();
            } else if let Some(rest) = line.strip_prefix("previous ") {
                let (id, name) = rest.split_once(' ').unwrap();
                entry.1 = Some((id.to_owned(), name.to_owned()));
            }
        } else {
            current = Some(line.split(' ').next().unwrap().to_owned());
        }
    }
    out
}

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    /// A file written, a line of it changed, renamed, reformatted (a
    /// revision to pass over), then renamed into a folder and changed in
    /// the same commit.
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let git = scratch.git();
        for (key, value) in [
            ("blame.markIgnoredLines", "false"),
            ("blame.markUnblamableLines", "false"),
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        let commit = |message: &str| {
            git.run(["add", "-A"]).unwrap();
            git.run(["commit", "-q", "-m", message]).unwrap();
            git.rev_parse("HEAD").unwrap()
        };
        scratch
            .write(FIRST, "alpha\nbeta\ngamma\ndelta\nepsilon\nzeta\n")
            .unwrap();
        commit("write the list");
        scratch
            .write(FIRST, "alpha\nbeta changed\ngamma\ndelta\nepsilon\nzeta\n")
            .unwrap();
        commit("change the second line");
        git.run(["mv", FIRST, SECOND]).unwrap();
        commit("rename the list");
        scratch
            .write(
                SECOND,
                "alpha  \nbeta changed  \ngamma  \ndelta  \nepsilon  \nzeta  \n",
            )
            .unwrap();
        let reformat = commit("only spaces");
        scratch
            .write(".git-blame-ignore-revs", &format!("{reformat}\n"))
            .unwrap();
        std::fs::create_dir_all(scratch.path().join("moved")).unwrap();
        git.run(["mv", SECOND, LAST]).unwrap();
        scratch
            .write(
                LAST,
                "alpha  \nbeta changed  \ngamma  \ndelta, moved  \nepsilon  \nzeta  \n",
            )
            .unwrap();
        git.run(["add", "--", LAST]).unwrap();
        git.run(["commit", "-q", "-m", "move the list and change it"])
            .unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .unwrap();
        Self {
            scratch,
            registry,
            ctx,
        }
    }

    fn run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|err| err.to_string())
    }

    /// Our blame of the last name, and for each line where its lines were
    /// before the commit it came from, as git says both.
    fn ours(&mut self, ignore_revs: bool) -> Vec<Said> {
        let lines = self
            .run(
                "repo.blame",
                json!({ "path": LAST, "ignore_revs": ignore_revs }),
            )
            .unwrap();
        let mut out = Vec::new();
        for line in lines.as_array().unwrap() {
            let commit = line["commit"].as_str().unwrap().to_owned();
            let filename = line
                .get("source_path")
                .and_then(Value::as_str)
                .unwrap_or(LAST)
                .to_owned();
            let previous = self
                .run(
                    "repo.blame_previous",
                    json!({ "path": filename, "rev": commit }),
                )
                .unwrap();
            let previous = (!previous.is_null()).then(|| {
                (
                    previous["rev"].as_str().unwrap().to_owned(),
                    previous["path"].as_str().unwrap().to_owned(),
                )
            });
            out.push(Said {
                commit,
                filename,
                previous,
            });
        }
        out
    }
}

#[test]
fn blame_before_a_commit_follows_the_file_to_its_name_there_as_git_does() {
    let mut h = Harness::new();
    let git = h.scratch.git();
    // Every line as git has it, passing over the reformatting and not.
    for (ignore, extra) in [
        (true, vec!["--ignore-revs-file", ".git-blame-ignore-revs"]),
        (false, vec![]),
    ] {
        let want = git_says(&git, LAST, &extra);
        assert_eq!(want.len(), 6);
        assert_eq!(h.ours(ignore), want, "ignoring revisions: {ignore}");
    }
    // The second line, passing over the reformatting, came from a commit
    // where the file had its first name, and before it the file had that
    // name too.
    let want = git_says(
        &git,
        LAST,
        &["--ignore-revs-file", ".git-blame-ignore-revs"],
    );
    let second = &want[1];
    assert_eq!(second.filename, FIRST);
    let (before, name) = second.previous.clone().unwrap();
    assert_eq!(name, FIRST);
    let blamed = h
        .run("repo.blame", json!({ "path": name, "rev": before }))
        .unwrap();
    assert_eq!(blamed[1]["text"], "beta");
    // The line changed where the file was moved: before it, the file had
    // its second name.
    let moved = h.ours(false)[3].clone();
    assert_eq!(moved.filename, LAST);
    assert_eq!(moved.previous.as_ref().map(|p| p.1.as_str()), Some(SECOND));
    // The first commit added the file: nothing was before it.
    let first = h.ours(true)[0].clone();
    assert_eq!(first.previous, None);
}

#[test]
fn blame_before_refuses_what_is_no_file_of_a_commit() {
    let mut h = Harness::new();
    let tree = h.scratch.git().rev_parse("HEAD^{tree}").unwrap();
    for params in [
        json!({ "path": "../outside.txt", "rev": "HEAD" }),
        json!({ "path": "/etc/passwd", "rev": "HEAD" }),
        json!({ "path": "", "rev": "HEAD" }),
        json!({ "path": LAST, "rev": tree }),
        json!({ "path": LAST, "rev": "no-such-revision" }),
        json!({ "path": LAST }),
        json!({ "rev": "HEAD" }),
    ] {
        assert!(
            h.run("repo.blame_previous", params.clone()).is_err(),
            "{params}"
        );
    }
    // A name the commit does not have was nowhere before it either.
    assert_eq!(
        h.run(
            "repo.blame_previous",
            json!({ "path": "no such file.txt", "rev": "HEAD" })
        )
        .unwrap(),
        Value::Null
    );
}
