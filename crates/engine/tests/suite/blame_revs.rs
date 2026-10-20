//! Blame that passes over revisions, against `git blame
//! --ignore-revs-file`: each line is given to the commit git gives it to,
//! with the file named by the configuration, with the conventional file,
//! and with the switch off.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
    /// The commit that only reformatted.
    reformat: String,
}

const PATH: &str = "src/list.txt";

impl Harness {
    /// A file written in one commit, changed in a second, reformatted
    /// whole in a third and changed again in a fourth.
    fn new() -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let git = scratch.git();
        // Whatever whoever runs the tests has configured for blame is not
        // left to decide.
        for (key, value) in [
            ("blame.markIgnoredLines", "false"),
            ("blame.markUnblamableLines", "false"),
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
        let commit = |text: &str, message: &str| -> String {
            scratch.write(PATH, text).unwrap();
            git.run(["add", "-A"]).unwrap();
            git.run(["commit", "-q", "-m", message]).unwrap();
            git.rev_parse("HEAD").unwrap()
        };
        commit(
            "alpha one\nbeta two\ngamma three\ndelta four\nepsilon five\nzeta six\n",
            "write the list",
        );
        commit(
            "alpha one\nbeta two, changed\ngamma three\ndelta four\nepsilon five\nzeta six\n",
            "change the second line",
        );
        let reformat = commit(
            "    alpha one\n    beta two, changed\n    gamma three\n    delta four\n    epsilon five\n    zeta six\n",
            "indent everything",
        );
        commit(
            "    alpha one\n    beta two, changed\n    gamma three\n    delta four\n    epsilon five, changed\n    zeta six\n    eta seven\n",
            "change the fifth line and add one",
        );
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": scratch.path() }))
            .expect("open scratch");
        Self {
            scratch,
            registry,
            ctx,
            reformat,
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
    /// The commit of each line, as the command answers.
    fn ours(&mut self, params: Value) -> Vec<(String, String)> {
        self.run("repo.blame", params)
            .as_array()
            .unwrap()
            .iter()
            .map(|line| {
                (
                    line["commit"].as_str().unwrap().to_owned(),
                    line["text"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }
    /// The commit of each line, as git prints a blame for people.
    fn git_says(&self, extra: &[&str], rev: &str) -> Vec<(String, String)> {
        let mut args = vec!["blame", "-l", "-s", "--root"];
        args.extend_from_slice(extra);
        args.extend_from_slice(&[rev, "--", PATH]);
        self.git()
            .lines(args)
            .unwrap()
            .into_iter()
            .map(|line| {
                let (id, rest) = line.split_once(' ').unwrap();
                let (_, text) = rest.split_once(") ").unwrap_or((rest, ""));
                (id.to_owned(), text.to_owned())
            })
            .collect()
    }
}

#[test]
fn revisions_listed_in_the_conventional_file_are_passed_over_when_asked() {
    let mut h = Harness::new();
    // Without a file there is nothing to pass over, and asking is refused.
    assert_eq!(h.run("repo.blame_ignores", json!({})), Value::Null);
    let err = h
        .try_run("repo.blame", json!({ "path": PATH, "ignore_revs": true }))
        .unwrap_err();
    assert!(err.contains(".git-blame-ignore-revs"), "{err}");

    let reformat = h.reformat.clone();
    h.scratch
        .write(
            ".git-blame-ignore-revs",
            &format!("# only whitespace\n{reformat}\n"),
        )
        .unwrap();
    let found = h.run("repo.blame_ignores", json!({}));
    assert_eq!(found["configured"], false);
    assert_eq!(found["exists"], true);
    assert_eq!(found["revisions"], 1);
    assert!(
        found["file"]
            .as_str()
            .unwrap()
            .ends_with(".git-blame-ignore-revs")
    );

    // Git does not read the conventional file by itself, and neither is
    // it read here unless asked for.
    let plain = h.git_says(&[], "HEAD");
    assert_eq!(h.ours(json!({ "path": PATH })), plain);
    assert_eq!(h.ours(json!({ "path": PATH, "ignore_revs": false })), plain);
    assert!(
        plain.iter().filter(|(id, _)| *id == reformat).count() >= 4,
        "the reformat owns most lines: {plain:?}"
    );

    let passed = h.git_says(&["--ignore-revs-file", ".git-blame-ignore-revs"], "HEAD");
    assert_eq!(h.ours(json!({ "path": PATH, "ignore_revs": true })), passed);
    assert!(
        passed.iter().all(|(id, _)| *id != reformat),
        "nothing is the reformat's any more: {passed:?}"
    );
    assert_eq!(passed.len(), 7);

    // At an earlier commit as well.
    let earlier = h.git_says(&["--ignore-revs-file", ".git-blame-ignore-revs"], "HEAD~1");
    assert_eq!(
        h.ours(json!({ "path": PATH, "rev": "HEAD~1", "ignore_revs": true })),
        earlier
    );
    assert_eq!(earlier.len(), 6);
}

#[test]
fn the_file_the_configuration_names_is_honoured_as_git_honours_it() {
    let mut h = Harness::new();
    let reformat = h.reformat.clone();
    h.scratch
        .write("meta/skip these", &format!("{reformat} # the indent\n"))
        .unwrap();
    h.git()
        .run(["config", "blame.ignoreRevsFile", "meta/skip these"])
        .unwrap();
    let found = h.run("repo.blame_ignores", json!({}));
    assert_eq!(found["configured"], true);
    assert_eq!(found["revisions"], 1);

    // Git passes them over without being told, and so does the command.
    let by_config = h.git_says(&[], "HEAD");
    assert!(by_config.iter().all(|(id, _)| *id != reformat));
    assert_eq!(h.ours(json!({ "path": PATH })), by_config);
    assert_eq!(
        h.ours(json!({ "path": PATH, "ignore_revs": true })),
        by_config
    );
    // Switched off, it is the blame git gives with the list cleared.
    let cleared = h.git_says(&["--ignore-revs-file="], "HEAD");
    assert!(cleared.iter().any(|(id, _)| *id == reformat));
    assert_eq!(
        h.ours(json!({ "path": PATH, "ignore_revs": false })),
        cleared
    );

    // The file that is named is gone: git fails, and so does asking for
    // it; a plain blame still works.
    std::fs::remove_file(h.scratch.path().join("meta/skip these")).unwrap();
    let found = h.run("repo.blame_ignores", json!({}));
    assert_eq!(
        (&found["configured"], &found["exists"]),
        (&json!(true), &json!(false))
    );
    assert!(
        h.try_run("repo.blame", json!({ "path": PATH, "ignore_revs": true }))
            .is_err()
    );
    assert_eq!(h.ours(json!({ "path": PATH })), cleared);
}

#[test]
fn hostile_files_and_parameters_fail_without_harm() {
    let mut h = Harness::new();
    // A file of revisions that names none: git says so.
    h.scratch
        .write(
            ".git-blame-ignore-revs",
            "not a commit\n--output=/tmp/bb-injected-blame\n",
        )
        .unwrap();
    assert!(
        h.try_run("repo.blame", json!({ "path": PATH, "ignore_revs": true }))
            .is_err()
    );
    let reformat = h.reformat.clone();
    h.scratch
        .write(".git-blame-ignore-revs", &format!("{reformat}\n"))
        .unwrap();
    for hostile in [
        json!("yes"),
        json!(1),
        json!([true]),
        json!({ "ignore": true }),
        json!("--ignore-revs-file=/etc/passwd"),
    ] {
        let err = h
            .try_run(
                "repo.blame",
                json!({ "path": PATH, "ignore_revs": hostile }),
            )
            .unwrap_err();
        assert!(err.contains("ignore_revs"), "{err}");
    }
    for path in [
        json!("--all"),
        json!("--output=/tmp/bb-injected-blame"),
        json!("../outside"),
        json!("/etc/passwd"),
        json!(""),
        json!("a\u{0}b"),
        json!("one\ntwo"),
        json!("nowhere.txt"),
        json!("x".repeat(70_000)),
    ] {
        assert!(
            h.try_run("repo.blame", json!({ "path": path, "ignore_revs": true }))
                .is_err(),
            "{path}"
        );
    }
    for rev in [json!("--all"), json!("no-such"), json!("HEAD:src")] {
        assert!(
            h.try_run(
                "repo.blame",
                json!({ "path": PATH, "rev": rev, "ignore_revs": true })
            )
            .is_err(),
            "{rev}"
        );
    }
    let _ = h.try_run("repo.blame_ignores", json!(["x"]));
    assert!(!std::path::Path::new("/tmp/bb-injected-blame").exists());
    assert_eq!(
        h.git().run(["status", "--porcelain"]).unwrap().trim(),
        "?? .git-blame-ignore-revs"
    );
}
