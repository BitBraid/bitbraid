//! What the commit box asks about messages, against git: trailers added
//! as `git interpret-trailers` adds them, the trailer keys in use, and
//! the messages of the commits made here, as the reflog has them.

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new(fixture: &str) -> Self {
        let scratch = Scratch::of(fixture).unwrap();
        // The engine's git reads the developer's own configuration: what
        // the answers depend on is said in the copy's.
        let git = scratch.git();
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("trailer.ifexists", "addIfDifferentNeighbor"),
            ("trailer.ifmissing", "add"),
            ("trailer.where", "end"),
            ("trailer.separators", ":"),
            ("core.logAllRefUpdates", "true"),
            ("commit.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
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
}

/// `git interpret-trailers` with the message on standard input.
fn oracle(git: &Git, message: &str, trailers: &[&str]) -> String {
    use std::io::Write as _;
    let mut cmd = git.command();
    cmd.args(["interpret-trailers", "--no-divider"]);
    for trailer in trailers {
        cmd.args(["--trailer", trailer]);
    }
    let mut child = cmd
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(message.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

const MESSAGES: &[&str] = &[
    "add a parser",
    "add a parser\n",
    "add a parser\n\nIt reads the file line by line.\n",
    "add a parser\n\nIt reads the file line by line.\n\nReviewed-by: Bob <bob@x.invalid>\n",
    "add a parser\n\nReviewed-by: Bob <bob@x.invalid>\nCo-authored-by: Cy <cy@x.invalid>\n",
    // A paragraph that looks like trailers and is not the last one.
    "add a parser\n\nNote: this is prose\n\nAnd the body goes on.\n",
    // A line of dashes is text in a commit message, not the start of a patch.
    "add a parser\n\nBefore\n---\nAfter\n",
    "add a parser\n\nSee: the notes\nmore prose in the same paragraph\n",
    "add a parser\n\n\n\nBody after blank lines.\n\n\n",
    "ünïcödé summary\n\nBödy with 🦑.\n",
    "one line with a colon: in it",
    "",
];

#[test]
fn trailers_are_added_as_git_adds_them() {
    let mut h = Harness::new("linear");
    let cases: &[&[(&str, &str)]] = &[
        &[("Co-authored-by", "Cy <cy@x.invalid>")],
        &[("Reviewed-by", "Bob <bob@x.invalid>")],
        &[
            ("Fixes", "#12"),
            ("Refs", "https://example.invalid/a?b=c:d"),
        ],
        &[
            ("Reviewed-by", "Bob <bob@x.invalid>"),
            ("Reviewed-by", "Bob <bob@x.invalid>"),
        ],
        &[("Change-Id", "I0123456789abcdef")],
    ];
    for message in MESSAGES {
        for trailers in cases {
            let asked: Vec<Value> = trailers
                .iter()
                .map(|(key, value)| json!({ "key": key, "value": value }))
                .collect();
            let ours = h.run(
                "commit.trailer",
                json!({ "message": message, "trailers": asked }),
            );
            let arguments: Vec<String> = trailers
                .iter()
                .map(|(key, value)| format!("{key}: {value}"))
                .collect();
            let arguments: Vec<&str> = arguments.iter().map(String::as_str).collect();
            assert_eq!(
                ours["message"].as_str().unwrap(),
                oracle(&h.git(), message, &arguments),
                "{message:?} with {trailers:?}"
            );
        }
    }
}

#[test]
fn the_repositorys_trailer_settings_are_honoured() {
    let mut h = Harness::new("linear");
    let message = "summary\n\nBody.\n\nAcked-by: Bob <bob@x.invalid>\n";
    let acked = json!([{ "key": "Acked-by", "value": "Cy <cy@x.invalid>" }]);
    let plain = h.run(
        "commit.trailer",
        json!({ "message": message, "trailers": acked }),
    );
    assert!(
        plain["message"]
            .as_str()
            .unwrap()
            .ends_with("Acked-by: Bob <bob@x.invalid>\nAcked-by: Cy <cy@x.invalid>\n")
    );
    // Told to replace a key that is there, git replaces it; so does this.
    h.git()
        .run(["config", "trailer.Acked-by.ifexists", "replace"])
        .unwrap();
    h.git().run(["config", "trailer.where", "start"]).unwrap();
    for (key, value) in [("Acked-by", "Cy <cy@x.invalid>"), ("Fixes", "#3")] {
        let ours = h.run(
            "commit.trailer",
            json!({ "message": message, "trailers": [{ "key": key, "value": value }] }),
        );
        assert_eq!(
            ours["message"].as_str().unwrap(),
            oracle(&h.git(), message, &[&format!("{key}: {value}")]),
            "{key}"
        );
    }
    let replaced = h.run(
        "commit.trailer",
        json!({ "message": message, "trailers": acked }),
    );
    assert!(
        !replaced["message"].as_str().unwrap().contains("Bob"),
        "{replaced}"
    );
}

#[test]
fn signing_off_names_who_git_commits_as_and_the_message_commits_with_its_trailers() {
    let mut h = Harness::new("linear");
    let out = h.run(
        "commit.trailer",
        json!({
            "message": "add a line\n\nWhy it was needed.",
            "sign_off": true,
            "trailers": [{ "key": "Co-authored-by", "value": "Cy <cy@x.invalid>" }],
        }),
    );
    let message = out["message"].as_str().unwrap();
    assert_eq!(
        message,
        oracle(
            &h.git(),
            "add a line\n\nWhy it was needed.",
            &[
                "Co-authored-by: Cy <cy@x.invalid>",
                "Signed-off-by: Ada Fixture <ada@fixture.invalid>"
            ]
        )
    );
    // Committed, git reads the same trailers back.
    h.scratch.write("a.txt", "2\nmore\n").unwrap();
    h.run(
        "commit.create",
        json!({ "message": message, "stage_all": true }),
    );
    assert_eq!(
        h.git()
            .lines(["log", "-1", "--format=%(trailers:only=true,unfold=true)"])
            .unwrap()
            .into_iter()
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>(),
        [
            "Co-authored-by: Cy <cy@x.invalid>",
            "Signed-off-by: Ada Fixture <ada@fixture.invalid>"
        ]
    );
    // What `git commit --signoff` would have written for the same commit.
    let twin = Scratch::of("linear").unwrap();
    twin.write("a.txt", "2\nmore\n").unwrap();
    twin.git().run(["add", "-A"]).unwrap();
    twin.git()
        .run([
            "commit",
            "-q",
            "--signoff",
            "-m",
            "add a line\n\nWhy it was needed.\n\nCo-authored-by: Cy <cy@x.invalid>",
        ])
        .unwrap();
    let body = |git: &Git| git.run(["log", "-1", "--format=%B%n%T"]).unwrap();
    assert_eq!(body(&h.git()), body(&twin.git()));
}

#[test]
fn the_trailer_keys_in_use_are_the_ones_git_reads_from_the_commits() {
    let mut h = Harness::new("linear");
    let commits = [
        "one\n\nReviewed-by: Bob <bob@x.invalid>\nFixes: #1\n",
        "two\n\nBody.\n\nReviewed-by: Cy <cy@x.invalid>\n",
        "three\n\nNote: prose\n\nmore prose\n",
        "four\n\nreviewed-by: lower <l@x.invalid>\nChange-Id: I1\n",
        "five",
    ];
    for (n, message) in commits.iter().enumerate() {
        h.scratch.write("a.txt", &format!("2\n{n}\n")).unwrap();
        h.git().run(["add", "-A"]).unwrap();
        h.git().run(["commit", "-q", "-m", message]).unwrap();
    }
    // Counted by hand from what git parses out of each message.
    let mut counted: std::collections::BTreeMap<String, usize> = Default::default();
    for id in h.git().lines(["rev-list", "HEAD"]).unwrap() {
        let message = h.git().run(["log", "-1", "--format=%B", &id]).unwrap();
        let mut cmd = h.git().command();
        cmd.args(["interpret-trailers", "--no-divider", "--parse"]);
        use std::io::Write as _;
        let mut child = cmd
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(message.as_bytes())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        for line in String::from_utf8(out.stdout).unwrap().lines() {
            if let Some((key, _)) = line.split_once(':') {
                *counted.entry(key.to_ascii_lowercase()).or_default() += 1;
            }
        }
    }
    let keys = h.run("commit.trailer_keys", json!({}));
    // Whoever runs the tests may have trailer keys configured for
    // themselves: those are listed too, and counted as nothing here.
    let ours: std::collections::BTreeMap<String, usize> = keys
        .as_array()
        .unwrap()
        .iter()
        .filter(|key| key["count"] != 0)
        .map(|key| {
            (
                key["key"].as_str().unwrap().to_ascii_lowercase(),
                key["count"].as_u64().unwrap() as usize,
            )
        })
        .collect();
    assert_eq!(ours, counted);
    assert_eq!(counted["reviewed-by"], 3);
    // The most used first, each as it was first written.
    let used: Vec<&Value> = keys
        .as_array()
        .unwrap()
        .iter()
        .filter(|key| key["configured"] == false)
        .collect();
    assert_eq!(used[0]["key"], "reviewed-by");
    assert_eq!(used[0]["count"], 3);

    // A key the configuration names comes first, used or not; only so
    // many commits are looked at.
    h.git()
        .run(["config", "trailer.ticket.key", "Ticket"])
        .unwrap();
    let keys = h.run("commit.trailer_keys", json!({ "limit": 1 }));
    let keys = keys.as_array().unwrap();
    assert!(
        keys.contains(&json!({ "key": "Ticket", "count": 0, "configured": true })),
        "{keys:?}"
    );
    assert!(keys.iter().all(|key| key["configured"] == true), "{keys:?}");
}

#[test]
fn the_messages_made_here_are_the_commits_of_the_reflog_newest_first() {
    let mut h = Harness::new("linear");
    // The fixture was copied, not made here: nothing yet.
    let none = h.run("commit.messages", json!({}));
    let reflog = |git: &Git| {
        git.lines(["reflog", "show", "--format=%H%x09%gs", "HEAD"])
            .unwrap_or_default()
    };
    assert_eq!(
        none.as_array().unwrap().len(),
        reflog(&h.git())
            .iter()
            .filter(|line| line.contains("\tcommit"))
            .count()
    );

    let commit = |h: &mut Harness, n: usize, message: &str| {
        h.scratch.write("a.txt", &format!("2\n{n}\n")).unwrap();
        h.run(
            "commit.create",
            json!({ "message": message, "stage_all": true }),
        )
    };
    commit(&mut h, 1, "first here\n\nWith a body.\n\nRefs: #1");
    commit(&mut h, 2, "second here");
    h.run("commit.amend", json!({ "message": "second here, amended" }));
    // Undone: gone from the branch, and its message is still to be had.
    commit(
        &mut h,
        3,
        "third, to be undone\n\nA long explanation\nover two lines.",
    );
    h.run("undo.last", json!({}));
    // Not commits made here: a reset (which drops what the undone commit
    // left staged) and two checkouts.
    h.git().run(["reset", "-q", "--hard", "HEAD"]).unwrap();
    h.git()
        .run(["checkout", "-q", "--detach", "HEAD~1"])
        .unwrap();
    h.git().run(["checkout", "-q", "main"]).unwrap();
    // The same message twice is listed once.
    commit(&mut h, 4, "second here");

    let listed = h.run("commit.messages", json!({}));
    let ours: Vec<&str> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["message"].as_str().unwrap())
        .collect();
    // What was made in this test comes before what the fixture was built
    // with, whose reflog was copied along.
    assert_eq!(
        ours[..4],
        [
            "second here",
            "third, to be undone\n\nA long explanation\nover two lines.",
            "second here, amended",
            "first here\n\nWith a body.\n\nRefs: #1",
        ]
    );
    // Against git: the reflog entries that are commits, each message once.
    let mut expected: Vec<(String, String)> = Vec::new();
    for line in reflog(&h.git()) {
        let (id, action) = line.split_once('\t').unwrap();
        if !action.starts_with("commit") {
            continue;
        }
        let message = h.git().run(["log", "-1", "--format=%B", id]).unwrap();
        let message = message.trim_end().to_owned();
        if !expected.iter().any(|(_, known)| *known == message) {
            expected.push((id.to_owned(), message));
        }
    }
    let got: Vec<(String, String)> = listed
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| {
            (
                entry["id"].as_str().unwrap().to_owned(),
                entry["message"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(got, expected);
    assert_eq!(listed[1]["summary"], "third, to be undone");
    assert!(listed[0]["time"].as_i64().unwrap() > 1_600_000_000);

    assert_eq!(
        h.run("commit.messages", json!({ "limit": 2 }))
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert_eq!(h.run("commit.messages", json!({ "limit": 0 })), json!([]));
    // Reading changed nothing.
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");
}

#[test]
fn hostile_parameters_are_refused_and_nothing_is_committed_or_started() {
    let mut h = Harness::new("linear");
    let head = h.git().rev_parse("HEAD").unwrap();
    let hostile = [
        Value::Null,
        json!(""),
        json!("-"),
        json!("--all"),
        json!("--in-place"),
        json!("--output=/tmp/bb-injected-trailer"),
        json!("a\u{0}b"),
        json!("one\ntwo\r\n"),
        json!("\u{202e}\u{feff}名前 🦑"),
        json!("x".repeat(70_000)),
        json!(-1),
        json!(u64::MAX),
        json!(1e308),
        json!(true),
        json!([]),
        json!({}),
        json!(["--force", "", null, 7, ["nested"]]),
        json!([{ "key": "--trailer", "value": "x" }]),
        json!([{ "key": "Key", "value": "a\nb" }]),
        json!([{ "key": "Key" }]),
    ];
    for value in &hostile {
        for params in [
            json!({ "message": value, "trailers": [{ "key": "Fixes", "value": "#1" }] }),
            json!({ "message": "m", "trailers": value }),
            json!({ "message": "m", "trailers": [{ "key": value, "value": "v" }] }),
            json!({ "message": "m", "trailers": [{ "key": "Fixes", "value": value }] }),
            json!({ "message": "m", "sign_off": value }),
        ] {
            if let Ok(out) = h.try_run("commit.trailer", params.clone()) {
                // What was taken was taken as text for the message.
                let message = out["message"].as_str().unwrap();
                assert!(message.contains(':'), "{params} gave {message:?}");
            }
        }
        let _ = h.try_run("commit.messages", json!({ "limit": value }));
        let _ = h.try_run("commit.trailer_keys", json!({ "limit": value }));
    }
    for shape in [Value::Null, json!([]), json!("text"), json!(7)] {
        assert!(h.try_run("commit.trailer", shape.clone()).is_err());
        let _ = h.try_run("commit.messages", shape.clone());
        let _ = h.try_run("commit.trailer_keys", shape);
    }
    // A key that is none, a value that is none, and nothing to add.
    for (params, word) in [
        (
            json!({ "message": "m", "trailers": [{ "key": "two words", "value": "v" }] }),
            "key",
        ),
        (
            json!({ "message": "m", "trailers": [{ "key": "Fixes", "value": " " }] }),
            "value",
        ),
        (json!({ "message": "m" }), "trailers"),
        (
            json!({ "trailers": [{ "key": "Fixes", "value": "#1" }] }),
            "message",
        ),
    ] {
        let err = h.try_run("commit.trailer", params).unwrap_err();
        assert!(err.contains(word), "{err}");
    }
    assert_eq!(h.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(h.git().run(["status", "--porcelain"]).unwrap(), "");
    assert!(!std::path::Path::new("/tmp/bb-injected-trailer").exists());
}
