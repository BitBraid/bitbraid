//! Search commands against `git log` and `git rev-list` on the `search`
//! fixture: each word of the grammar alone, words together and negated,
//! the order of the graph, content search, and locate.

use std::{
    collections::BTreeSet,
    io::Write as _,
    process::Stdio,
    sync::{Arc, Mutex, atomic::Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, oracle::CLOCK_EPOCH};
use serde_json::{Value, json};

const HOUR: i64 = 3600;
const DAY: i64 = 24 * HOUR;

/// Where the graph starts from when nothing else is said, as git is told.
const ALL: [&str; 4] = ["HEAD", "--branches", "--remotes", "--tags"];

type Ids = BTreeSet<String>;

struct Harness {
    copy: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        Self::of("search")
    }
    fn of(fixture: &str) -> Self {
        let copy = Scratch::of(fixture).unwrap();
        let registry = builtin_registry();
        let mut ctx = CommandContext::default();
        registry
            .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
            .expect("open scratch");
        Self {
            copy,
            registry,
            ctx,
        }
    }
    fn git(&self) -> Git {
        self.copy.git()
    }
    fn run(&mut self, id: &str, params: Value) -> Value {
        let params = in_utc(id, params);
        self.registry
            .run(id, &mut self.ctx, &params)
            .unwrap_or_else(|err| panic!("{id} {params}: {err}"))
    }
    fn try_run(&mut self, id: &str, params: Value) -> Result<Value, String> {
        let params = in_utc(id, params);
        self.registry
            .run(id, &mut self.ctx, &params)
            .map_err(|e| e.to_string())
    }
    /// The ids the graph has, in its order.
    fn graph(&mut self, tips: Value) -> Vec<String> {
        let out = self.run("repo.graph", json!({ "tips": tips }));
        out["commits"]
            .as_array()
            .unwrap()
            .iter()
            .map(|commit| commit["id"].as_str().unwrap().to_owned())
            .collect()
    }
    /// What `git log` lists with `options`, from `tips`, for `paths`.
    fn log_from(&self, tips: &[&str], options: &[&str], paths: &[&str]) -> Ids {
        let mut args = vec!["log", "--format=%H"];
        args.extend(options);
        args.extend(tips);
        args.push("--");
        args.extend(paths);
        self.git().lines(args).unwrap().into_iter().collect()
    }
    fn log(&self, options: &[&str]) -> Ids {
        self.log_from(&ALL, options, &[])
    }
    /// Message, author and committer as `message:`, `author:` and
    /// `committer:` read them: part of it, letters without case.
    fn grep(&self, option: &str, text: &str) -> Ids {
        self.log(&["-i", "--fixed-strings", &format!("--{option}={text}")])
    }
    fn reachable(&self, rev: &str) -> Ids {
        self.git()
            .lines(["rev-list", rev])
            .unwrap()
            .into_iter()
            .collect()
    }
    fn everything(&self) -> Ids {
        self.log(&[])
    }
    fn beginning_with(&self, digits: &str) -> Ids {
        let digits = digits.to_lowercase();
        self.everything()
            .into_iter()
            .filter(|id| id.starts_with(&digits))
            .collect()
    }
    /// Free text as git has it: the message, the author, or the id.
    fn text(&self, text: &str) -> Ids {
        let mut ids = self.grep("grep", text);
        ids.extend(self.grep("author", text));
        ids.extend(self.beginning_with(text));
        ids
    }
    fn id(&self, rev: &str) -> String {
        self.git().rev_parse(&format!("{rev}^{{commit}}")).unwrap()
    }
    /// A query answers with exactly `want`, in the order the graph has
    /// them.
    fn assert_finds(&mut self, query: &str, want: &Ids) {
        let out = self.run("search.commits", json!({ "query": query }));
        let got = ids_of(&out);
        let in_order: Vec<String> = self
            .graph(json!("all"))
            .into_iter()
            .filter(|id| want.contains(id))
            .collect();
        assert_eq!(in_order.len(), want.len(), "{query}: outside the graph");
        assert_eq!(got, in_order, "{query}");
        assert_eq!(out["truncated"], false, "{query}");
    }
}

/// A search reads its dates in the time zone of the machine, as `git log`
/// does. The git these tests compare with runs in UTC (the testkit sets
/// `TZ`), so the searches are told to as well, unless a test says which
/// zone it means: what a zone does is held to git in a test of its own.
fn in_utc(id: &str, mut params: Value) -> Value {
    if id.starts_with("search.")
        && let Some(fields) = params.as_object_mut()
        && !fields.contains_key("zone")
    {
        fields.insert("zone".to_owned(), json!("UTC"));
    }
    params
}

fn ids_of(out: &Value) -> Vec<String> {
    out["ids"]
        .as_array()
        .unwrap_or_else(|| panic!("no ids in {out}"))
        .iter()
        .map(|id| id.as_str().unwrap().to_owned())
        .collect()
}

fn both(a: &Ids, b: &Ids) -> Ids {
    a.intersection(b).cloned().collect()
}

fn without(a: &Ids, b: &Ids) -> Ids {
    a.difference(b).cloned().collect()
}

#[test]
fn a_query_is_read_into_words_or_into_what_is_wrong() {
    let mut h = Harness::new();
    let out = h.run(
        "search.parse",
        json!({ "query": r#"fix -author:"Ada L" before:3d is:merge"#, "now": CLOCK_EPOCH + 3 * DAY }),
    );
    assert_eq!(
        out,
        json!({ "words": [
            { "key": "text", "value": "fix", "negated": false, "start": 0, "end": 3 },
            { "key": "author", "value": "Ada L", "negated": true, "start": 4, "end": 19 },
            { "key": "before", "value": "3d", "negated": false, "start": 20, "end": 29,
              "seconds": CLOCK_EPOCH },
            { "key": "is", "value": "merge", "negated": false, "start": 30, "end": 38 },
        ] })
    );
    // Every key there is; a date without `now` is the same at any time.
    let out = h.run(
        "search.parse",
        json!({ "query": "message:a committer:b sha:AB12 after:2020-01-01T05:00 ref:main path:src change:x regex:y.z" }),
    );
    let read: Vec<(&str, &str)> = out["words"]
        .as_array()
        .unwrap()
        .iter()
        .map(|word| {
            (
                word["key"].as_str().unwrap(),
                word["value"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        read,
        [
            ("message", "a"),
            ("committer", "b"),
            ("sha", "ab12"),
            ("after", "2020-01-01T05:00"),
            ("ref", "main"),
            ("path", "src"),
            ("change", "x"),
            ("regex", "y.z"),
        ]
    );
    assert_eq!(out["words"][3]["seconds"], CLOCK_EPOCH + 5 * HOUR);
    assert_eq!(
        h.run("search.parse", json!({ "query": " " })),
        json!({ "words": [] })
    );

    // What is wrong is an answer too, with its place.
    assert_eq!(
        h.run("search.parse", json!({ "query": "fix author:" })),
        json!({ "error": { "position": 11, "end": 11, "expected": "a value after `author:`" } })
    );
    for (query, position, expected) in [
        ("-", 0, "a word after `-`"),
        ("fix - it", 4, "a word after `-`"),
        ("--fix", 1, "a word after `-`"),
        (r#"fix "open"#, 4, "a closing quote"),
        (r#"author:"Ada"#, 7, "a closing quote"),
        (r#""""#, 0, "text between the quotes"),
        (
            "foo:bar",
            0,
            "one of the keys message: author: committer: sha: before: after: is: ref: path: change: regex: issue:",
        ),
        ("is:fast", 3, "`merge`"),
        ("sha:xyz", 4, "hexadecimal digits"),
        ("before:soon", 7, "a date such as 2026-10-01"),
        ("after:2020-02-30", 6, "a date such as 2026-10-01"),
        ("ref:-x", 4, "a branch, tag or commit"),
        ("path:-x", 5, "a path"),
    ] {
        let out = h.run("search.parse", json!({ "query": query }));
        assert_eq!(out["error"]["position"], position, "{query}: {out}");
        let said = out["error"]["expected"].as_str().unwrap();
        assert!(said.contains(expected), "{query}: {said}");
        assert!(out.get("words").is_none(), "{query}: {out}");
        // The same query is no search.
        let err = h
            .try_run("search.commits", json!({ "query": query }))
            .unwrap_err();
        assert!(
            err.contains(&format!("at byte {position}: expected")),
            "{err}"
        );
    }
}

#[test]
fn each_word_holds_for_what_git_lists() {
    let mut h = Harness::new();
    let everything = h.everything();
    assert_eq!(everything.len(), 16);

    // The message, the author, the committer: part of it, without case.
    for (key, option, text, count) in [
        ("message", "grep", "lexer", 3),
        ("message", "grep", "FIX", 2),
        ("message", "grep", "tokenizer", 1),
        ("message", "grep", "reads words", 1),
        ("message", "grep", "nowhere", 0),
        ("author", "author", "ada", 6),
        ("author", "author", "BOB@fixture", 9),
        ("author", "author", "Carol Example", 1),
        // Name and address are one text, as git reads the header.
        ("author", "author", "fixture <ada", 6),
        ("author", "author", "example.invalid>", 1),
        ("committer", "committer", "ada", 7),
        ("committer", "committer", "carol", 0),
    ] {
        let want = h.grep(option, text);
        assert_eq!(want.len(), count, "{key}:{text} in the fixture");
        h.assert_finds(&format!("{key}:\"{text}\""), &want);
        h.assert_finds(&format!("-{key}:\"{text}\""), &without(&everything, &want));
    }

    // Free text: the message, the author, or the beginning of the id.
    let wip = h.id("wip");
    for text in [
        "needle",
        "ada",
        "carol",
        "LEXER",
        "fixture.invalid",
        "merge",
        &wip[..7],
    ] {
        let want = h.text(text);
        assert!(!want.is_empty(), "{text} in the fixture");
        h.assert_finds(text, &want);
        h.assert_finds(&format!("-{text}"), &without(&everything, &want));
    }
    // "ada" is an author six times, and in a message of her own.
    assert_eq!(h.text("ada").len(), 6);
    h.assert_finds("\"Ada's unfinished\"", &h.grep("grep", "Ada's unfinished"));
    h.assert_finds(&wip[..7].to_uppercase(), &BTreeSet::from([wip.clone()]));

    // The id.
    for digits in [&wip[..1], &wip[..4], &wip[..12], &wip[..], "0", "f"] {
        let want = h.beginning_with(digits);
        h.assert_finds(&format!("sha:{digits}"), &want);
        h.assert_finds(&format!("sha:{}", digits.to_uppercase()), &want);
        h.assert_finds(&format!("-sha:{digits}"), &without(&everything, &want));
    }
    h.assert_finds(&format!("sha:{wip}0"), &BTreeSet::new());

    // Merges.
    let merges = h.log(&["--merges"]);
    assert_eq!(merges.len(), 3);
    h.assert_finds("is:merge", &merges);
    h.assert_finds("-is:merge", &h.log(&["--no-merges"]));

    // The commit date. `before:` is strictly before and `after:` is at or
    // after, where git's two both take the instant itself in.
    let at = CLOCK_EPOCH + 5 * HOUR;
    let before = h.log(&[&format!("--before=@{}", at - 1)]);
    let after = h.log(&[&format!("--after=@{at}")]);
    assert_eq!((before.len(), after.len()), (4, 12));
    h.assert_finds("before:2020-01-01T05:00", &before);
    h.assert_finds("after:2020-01-01T05:00", &after);
    h.assert_finds("-before:2020-01-01T05:00", &after);
    h.assert_finds("-after:2020-01-01T05:00", &before);
    h.assert_finds("after:2020-01-01", &everything);
    h.assert_finds("before:2020-01-01", &BTreeSet::new());
    h.assert_finds("before:2020-01-02", &everything);

    // What a branch, a tag or a commit reaches.
    let lexer = h.id("feature/lexer");
    for rev in [
        "main",
        "feature/lexer",
        "v0.1",
        "v0.2",
        "pages",
        "wip",
        "HEAD~2",
        "main^2",
        "refs/heads/feature/dates",
        &lexer[..],
        &lexer[..8],
    ] {
        let want = h.reachable(rev);
        h.assert_finds(&format!("ref:{rev}"), &want);
        h.assert_finds(&format!("-ref:{rev}"), &without(&everything, &want));
    }
    assert_eq!(h.reachable("pages").len(), 1);

    // The commits that change a path, a directory, a pattern. A merge is
    // among them when it differs from one of its parents there.
    for (path, count) in [
        ("src/lexer.rs", 6),
        ("src/parser.rs", 4),
        ("grammar.rs", 1),
        ("src", 11),
        ("docs/", 2),
        ("*.md", 3),
        ("src/*.rs", 11),
        ("no/such/path", 0),
    ] {
        let want = h.log_from(&ALL, &["--full-history"], &[path]);
        assert_eq!(want.len(), count, "path:{path} in the fixture");
        h.assert_finds(&format!("path:{path}"), &want);
        h.assert_finds(&format!("-path:\"{path}\""), &without(&everything, &want));
    }

    // What is added or removed, and changed lines that match.
    for (text, count) in [
        ("needle", 5),
        ("quotes", 2),
        ("TODO", 2),
        ("Needle", 0),
        ("fn lex", 1),
    ] {
        let want = h.log(&[&format!("-S{text}")]);
        assert_eq!(want.len(), count, "change:{text} in the fixture");
        h.assert_finds(&format!("change:\"{text}\""), &want);
        h.assert_finds(&format!("-change:\"{text}\""), &without(&everything, &want));
    }
    for (regex, count) in [
        (r"fn [a-z]+\(", 7),
        ("^const", 1),
        ("needle|quotes", 7),
        ("[0-9]{5}", 1),
        ("never there", 0),
    ] {
        let want = h.log(&[&format!("-G{regex}")]);
        assert_eq!(want.len(), count, "regex:{regex} in the fixture");
        h.assert_finds(&format!("regex:\"{regex}\""), &want);
        h.assert_finds(&format!("-regex:\"{regex}\""), &without(&everything, &want));
    }
}

/// The expression `git log -E --grep` finds a key with: `#` and the
/// number, not touching a word after it, and before it the start of a
/// line, white space or a mark a key may follow, with `repo` and its
/// path, any path, or none between.
fn issue_pattern(repo: Option<&str>, number: u64) -> String {
    let before = r#"(^|[[:space:]([{<"'*~,;:|])"#;
    let part = "[A-Za-z0-9_][A-Za-z0-9_.-]*";
    let path = match repo {
        Some(repo) => repo.replace('.', "\\."),
        None => format!("({part}(/{part})+)?"),
    };
    format!("{before}{path}#{number}([^[:alnum:]_]|$)")
}

#[test]
fn issue_keys_are_found_as_git_finds_them() {
    let mut h = Harness::of("issue-keys");
    let everything = h.everything();
    assert_eq!(everything.len(), 15);
    // An expression knows no code: what it finds only in code is not a key.
    let in_code = h.grep("grep", "in code");
    assert_eq!(in_code.len(), 2);
    for (query, repo, number, count) in [
        ("issue:12", None, 12, 5),
        ("issue:#12", None, 12, 5),
        ("issue:\"#12\"", None, 12, 5),
        ("issue:octo/tools#12", Some("octo/tools"), 12, 1),
        ("issue:OCTO/Tools#12", Some("octo/tools"), 12, 1),
        (
            "issue:group/sub/project#12",
            Some("group/sub/project"),
            12,
            1,
        ),
        ("issue:other/repo#12", Some("other/repo"), 12, 1),
        ("issue:sub/project#12", Some("sub/project"), 12, 0),
        ("issue:5", None, 5, 3),
        ("issue:octo/tools#5", Some("octo/tools"), 5, 1),
        ("issue:123", None, 123, 1),
        ("issue:1", None, 1, 1),
        ("issue:7", None, 7, 1),
        ("issue:999", None, 999, 0),
    ] {
        let found = h.log(&[
            "-i",
            "-E",
            &format!("--grep={}", issue_pattern(repo, number)),
        ]);
        let want = without(&found, &in_code);
        assert_eq!(want.len(), count, "{query} in the fixture");
        h.assert_finds(query, &want);
        h.assert_finds(&format!("-{query}"), &without(&everything, &want));
    }
    // With other words, and live from the tips the graph is asked for.
    let bare = without(
        &h.log(&["-E", &format!("--grep={}", issue_pattern(None, 12))]),
        &in_code,
    );
    h.assert_finds("issue:12 lexer", &both(&bare, &h.grep("grep", "lexer")));
    let on_main = h.reachable("main");
    let out = h.run(
        "search.commits",
        json!({ "query": "issue:12", "tips": "head" }),
    );
    let got: Ids = ids_of(&out).into_iter().collect();
    assert_eq!(got, both(&bare, &on_main));
}

#[test]
fn all_words_of_a_query_have_to_hold() {
    let mut h = Harness::new();
    let everything = h.everything();
    let ada = h.grep("author", "ada");
    let bob = h.grep("author", "bob");
    let merges = h.log(&["--merges"]);
    let lexer = h.grep("grep", "lexer");
    let needle = h.log(&["-Sneedle"]);
    let src = h.log_from(&ALL, &["--full-history"], &["src"]);

    // Where git combines the same way itself, it is asked once.
    let want = h.log(&["-i", "--fixed-strings", "--author=ada", "--grep=lexer"]);
    assert_eq!(want, both(&ada, &lexer));
    assert_eq!(want.len(), 2);
    h.assert_finds("author:ada message:lexer", &want);
    h.assert_finds("message:LEXER   author:Ada", &want);

    let want = h.log(&["--author=Bob", "--merges"]);
    assert_eq!(want.len(), 2);
    h.assert_finds("author:bob is:merge", &want);
    h.assert_finds("is:merge -author:ada", &want);

    let (from, to) = (CLOCK_EPOCH + 3 * HOUR, CLOCK_EPOCH + 9 * HOUR);
    let want = h.log(&[
        &format!("--after=@{from}"),
        &format!("--before=@{}", to - 1),
        "--no-merges",
    ]);
    assert_eq!(want.len(), 5);
    h.assert_finds(
        "after:2020-01-01T03:00 before:2020-01-01T09:00 -is:merge",
        &want,
    );

    let want = h.log_from(&["v0.1..main"], &["--full-history"], &["src"]);
    assert_eq!(
        want,
        both(&without(&h.reachable("main"), &h.reachable("v0.1")), &src)
    );
    assert_eq!(want.len(), 9);
    h.assert_finds("ref:main -ref:v0.1 path:src", &want);
    h.assert_finds("path:src -ref:v0.1 ref:main", &want);

    let want = h.log_from(&["main"], &["-Sneedle", "--author=bob"], &[]);
    assert_eq!(want, both(&both(&needle, &bob), &h.reachable("main")));
    assert_eq!(want.len(), 1);
    h.assert_finds("change:needle author:bob ref:main", &want);

    // The rest by sets.
    let text_lexer = h.text("lexer");
    h.assert_finds("lexer -author:bob", &without(&text_lexer, &bob));
    h.assert_finds(
        "-needle -ada",
        &without(&without(&everything, &h.text("needle")), &h.text("ada")),
    );
    h.assert_finds("-author:ada -author:bob", &h.grep("author", "carol"));
    h.assert_finds(
        "\"Merge the\" is:merge",
        &both(&h.grep("grep", "Merge the"), &merges),
    );
    let docs = h.log_from(&ALL, &["--full-history"], &["docs"]);
    let window = both(
        &h.log(&[&format!("--after=@{from}")]),
        &h.log(&[&format!("--before=@{}", CLOCK_EPOCH + 14 * HOUR - 1)]),
    );
    let want = both(&without(&needle, &docs), &window);
    assert_eq!(want.len(), 3);
    h.assert_finds(
        "change:needle -path:docs after:2020-01-01T03:00 before:2020-01-01T14:00",
        &want,
    );
    let quotes = h.log(&["-Gquotes"]);
    let want = without(&both(&quotes, &ada), &h.reachable("feature/lexer"));
    assert_eq!(want.len(), 3);
    h.assert_finds("regex:quotes author:ada -ref:feature/lexer", &want);
    h.assert_finds(
        "regex:quotes change:needle path:src/lexer.rs",
        &both(
            &both(&quotes, &needle),
            &h.log_from(&ALL, &["--full-history"], &["src/lexer.rs"]),
        ),
    );
    // A word and its opposite hold for nothing; twice the same is once.
    h.assert_finds("author:ada -author:ada", &BTreeSet::new());
    h.assert_finds("ref:main -ref:main", &BTreeSet::new());
    h.assert_finds("is:merge is:merge is:merge", &merges);
    // A word that holds for nothing ends it, whatever else is asked.
    h.assert_finds(
        "change:nowhere-to-be-found regex:fn path:src",
        &BTreeSet::new(),
    );
}

#[test]
fn answers_keep_the_order_of_the_graph_within_tips_and_limit() {
    let mut h = Harness::new();
    let all = h.graph(json!("all"));
    assert_eq!(all.len(), 16);

    // No word: everything, as the graph has it.
    let out = h.run("search.commits", json!({ "query": "" }));
    assert_eq!(ids_of(&out), all);
    assert_eq!(out["truncated"], false);

    // The limit cuts, and says so only when something was cut off.
    let plain = h.log(&["--no-merges"]);
    let in_order: Vec<String> = all
        .iter()
        .filter(|id| plain.contains(*id))
        .cloned()
        .collect();
    assert_eq!(in_order.len(), 13);
    for (limit, truncated) in [(0, true), (1, true), (12, true), (13, false), (14, false)] {
        let out = h.run(
            "search.commits",
            json!({ "query": "-is:merge", "limit": limit }),
        );
        let want: Vec<String> = in_order.iter().take(limit).cloned().collect();
        assert_eq!(ids_of(&out), want, "limit {limit}");
        assert_eq!(out["truncated"], truncated, "limit {limit}");
    }
    let out = h.run(
        "search.commits",
        json!({ "query": "-is:merge", "limit": u64::MAX }),
    );
    assert_eq!(ids_of(&out), in_order);
    let out = h.run("search.commits", json!({ "query": "nowhere", "limit": 0 }));
    assert_eq!(out, json!({ "ids": [], "truncated": false }));

    // From HEAD only: what is not reachable from it is not found.
    let head = h.graph(json!("head"));
    assert_eq!(head.len(), 14);
    let in_src = h.log_from(&["HEAD"], &["--full-history"], &["src"]);
    // The branch that tried a cache and dropped it left the file as it
    // was. Git's history of a path leaves such a branch out unless told
    // `--full-history`; `path:` has every commit that touched it.
    let lexer = h.log_from(&["HEAD"], &["--full-history"], &["src/lexer.rs"]);
    assert_eq!(lexer.len(), 5);
    assert_eq!(h.log_from(&["HEAD"], &[], &["src/lexer.rs"]).len(), 2);
    for (query, want) in [
        ("author:bob", h.log_from(&["HEAD"], &["--author=bob"], &[])),
        ("change:needle", h.log_from(&["HEAD"], &["-Sneedle"], &[])),
        (
            "regex:needle path:src",
            both(&h.log_from(&["HEAD"], &["-Gneedle"], &[]), &in_src),
        ),
        ("path:src/lexer.rs", lexer),
    ] {
        let out = h.run("search.commits", json!({ "query": query, "tips": "head" }));
        let in_order: Vec<String> = head
            .iter()
            .filter(|id| want.contains(*id))
            .cloned()
            .collect();
        assert_eq!(ids_of(&out), in_order, "{query} from HEAD");
        assert!(!in_order.is_empty(), "{query} from HEAD");
    }
    // `ref:` is about where a commit sits, whatever the graph starts from.
    let out = h.run(
        "search.commits",
        json!({ "query": "ref:pages", "tips": "head" }),
    );
    assert_eq!(out, json!({ "ids": [], "truncated": false }));

    // From commits that are named.
    let (pages, lexer) = (h.id("pages"), h.id("feature/lexer"));
    let tips = json!({ "ids": [pages, lexer] });
    let some = h.graph(tips.clone());
    assert_eq!(some.len(), 5);
    let want = h.log_from(&[&pages, &lexer], &["-Sneedle"], &[]);
    assert_eq!(want.len(), 3);
    let out = h.run(
        "search.commits",
        json!({ "query": "change:needle", "tips": tips }),
    );
    let in_order: Vec<String> = some
        .iter()
        .filter(|id| want.contains(*id))
        .cloned()
        .collect();
    assert_eq!(ids_of(&out), in_order);
    let out = h.run(
        "search.commits",
        json!({ "query": "-author:nobody", "tips": tips }),
    );
    assert_eq!(ids_of(&out), some);
}

#[test]
fn ages_count_back_from_now() {
    let mut h = Harness::new();
    let everything = h.everything();
    let now = CLOCK_EPOCH + 3 * DAY + 5 * HOUR;
    let at = CLOCK_EPOCH + 5 * HOUR;
    let before = h.log(&[&format!("--before=@{}", at - 1)]);
    let after = h.log(&[&format!("--after=@{at}")]);
    for (query, want) in [
        ("before:3d", &before),
        ("after:3d", &after),
        ("-before:3d", &after),
        ("after:1w", &everything),
        ("before:2d", &everything),
        ("before:1y", &BTreeSet::new()),
        ("after:0d", &BTreeSet::new()),
    ] {
        let out = h.run("search.commits", json!({ "query": query, "now": now }));
        let got: Ids = ids_of(&out).into_iter().collect();
        assert_eq!(&got, want, "{query}");
    }
    // Months are those of the calendar: six back from the 4th of January.
    let out = h.run(
        "search.parse",
        json!({ "query": "after:6m before:1y", "now": now }),
    );
    let july = h.run("search.parse", json!({ "query": "after:2019-07-04T05:00" }));
    assert_eq!(out["words"][0]["seconds"], july["words"][0]["seconds"]);
    assert_eq!(
        out["words"][1]["seconds"],
        CLOCK_EPOCH - 365 * DAY + 3 * DAY + 5 * HOUR
    );
    // Without `now` it is the clock: the fixture is years old by now.
    let out = h.run("search.commits", json!({ "query": "before:1y" }));
    assert_eq!(ids_of(&out).len(), 16);
    assert!(ids_of(&h.run("search.commits", json!({ "query": "after:1y" }))).is_empty());
}

#[test]
fn content_search_matches_the_pickaxe() {
    let mut h = Harness::new();
    let all = h.graph(json!("all"));
    let in_order = |want: &Ids| -> Vec<String> {
        all.iter()
            .filter(|id| want.contains(*id))
            .cloned()
            .collect()
    };
    for (params, options, paths, count) in [
        (json!({ "text": "needle" }), vec!["-Sneedle"], vec![], 5),
        (
            json!({ "text": "needle", "regex": false, "paths": [] }),
            vec!["-Sneedle"],
            vec![],
            5,
        ),
        // As `git log -S` has it: letters with their case.
        (json!({ "text": "NEEDLE" }), vec!["-SNEEDLE"], vec![], 0),
        (
            json!({ "text": "fn lex() { quotes(); }" }),
            vec!["-Sfn lex() { quotes(); }"],
            vec![],
            4,
        ),
        (json!({ "text": "--all" }), vec!["-S--all"], vec![], 0),
        (
            json!({ "text": "needle", "paths": ["src"] }),
            vec!["-Sneedle", "--full-history"],
            vec!["src"],
            3,
        ),
        (
            json!({ "text": "needle", "paths": ["docs", "index.html", "nowhere"] }),
            vec!["-Sneedle", "--full-history"],
            vec!["docs", "index.html", "nowhere"],
            2,
        ),
        (
            json!({ "text": r"fn [a-z]+\(", "regex": true }),
            vec![r"-Gfn [a-z]+\("],
            vec![],
            7,
        ),
        (
            json!({ "text": "needle", "regex": true }),
            vec!["-Gneedle"],
            vec![],
            5,
        ),
        (
            json!({ "text": "n.{4}e|qu?otes", "regex": true }),
            vec!["-Gn.{4}e|qu?otes"],
            vec![],
            7,
        ),
        (
            json!({ "text": "^fn", "regex": true, "paths": ["src/lexer.rs", "src/dates.rs"] }),
            vec!["-G^fn", "--full-history"],
            vec!["src/lexer.rs", "src/dates.rs"],
            6,
        ),
        (
            json!({ "text": "never there", "regex": true }),
            vec!["-Gnever there"],
            vec![],
            0,
        ),
    ] {
        let want = h.log_from(&ALL, &options, &paths);
        assert_eq!(want.len(), count, "{params} in the fixture");
        let out = h.run("search.content", params.clone());
        assert_eq!(ids_of(&out), in_order(&want), "{params}");
        assert_eq!(out["truncated"], false, "{params}");
    }
    // A string is not read as a regular expression, nor the other way.
    let dots = h.run("search.content", json!({ "text": "n....e" }));
    assert!(ids_of(&dots).is_empty());
    let dots = h.run("search.content", json!({ "text": "n....e", "regex": true }));
    assert_eq!(ids_of(&dots).len(), 5);

    // Limit and tips, as for a query.
    let want = in_order(&h.log(&["-Sneedle"]));
    let out = h.run("search.content", json!({ "text": "needle", "limit": 2 }));
    assert_eq!(ids_of(&out), want[..2]);
    assert_eq!(out["truncated"], true);
    let out = h.run("search.content", json!({ "text": "needle", "limit": 5 }));
    assert_eq!(out["truncated"], false);
    let head = h.graph(json!("head"));
    let from_head = h.log_from(&["HEAD"], &["-Sneedle"], &[]);
    let out = h.run(
        "search.content",
        json!({ "text": "needle", "tips": "head" }),
    );
    let want: Vec<String> = head
        .iter()
        .filter(|id| from_head.contains(*id))
        .cloned()
        .collect();
    assert_eq!(want.len(), 3);
    assert_eq!(ids_of(&out), want);
    // Within paths it is every commit that touched them, as `path:` has
    // it: the cache that was tried and dropped again on a branch is found
    // from HEAD, where git's own history of the file leaves that branch out.
    for (text, regex, option) in [
        ("cache", false, "-Scache"),
        ("cache\\(", true, "-Gcache\\("),
    ] {
        let params =
            json!({ "text": text, "regex": regex, "paths": ["src/lexer.rs"], "tips": "head" });
        let want = h.log_from(&["HEAD"], &[option, "--full-history"], &["src/lexer.rs"]);
        assert_eq!(want.len(), 2);
        assert!(
            h.log_from(&["HEAD"], &[option], &["src/lexer.rs"])
                .is_empty()
        );
        let out = h.run("search.content", params);
        let in_order: Vec<String> = head
            .iter()
            .filter(|id| want.contains(*id))
            .cloned()
            .collect();
        assert_eq!(ids_of(&out), in_order, "{text}");
    }
    // The same as the word of the grammar.
    let word = h.run("search.commits", json!({ "query": "change:needle" }));
    assert_eq!(word, h.run("search.content", json!({ "text": "needle" })));
    let word = h.run("search.commits", json!({ "query": "regex:^fn" }));
    assert_eq!(
        word,
        h.run("search.content", json!({ "text": "^fn", "regex": true }))
    );
}

#[test]
fn locate_answers_the_commit_of_a_revision_or_a_date() {
    let mut h = Harness::new();
    let lexer = h.id("feature/lexer");
    for rev in [
        "main",
        "HEAD",
        "v0.1",
        // An annotated tag: the commit it stands for, not the tag.
        "v0.2",
        "feature/lexer",
        "refs/heads/pages",
        "HEAD~3",
        "main^2",
        "v0.2^{commit}",
        &lexer[..],
        &lexer[..7],
    ] {
        let out = h.run("search.locate", json!({ "rev": rev }));
        assert_eq!(out, json!({ "id": h.id(rev) }), "{rev}");
    }

    // A date: the first commit of the graph that is not newer.
    let all = h.graph(json!("all"));
    let newest_by = |h: &Harness, tips: &[&str], at: i64| -> Value {
        let mut args = vec![
            "rev-list".to_owned(),
            "--date-order".to_owned(),
            "-n".to_owned(),
        ];
        args.extend(["1".to_owned(), format!("--before=@{at}")]);
        args.extend(tips.iter().map(|tip| (*tip).to_owned()));
        h.git()
            .lines(args)
            .unwrap()
            .first()
            .map_or(Value::Null, |id| json!(id))
    };
    for hours in [-1, 0, 1, 2, 5, 6, 12, 13, 14, 15, 16, 17, 18, 1000] {
        for at in [CLOCK_EPOCH + hours * HOUR - 1, CLOCK_EPOCH + hours * HOUR] {
            let out = h.run("search.locate", json!({ "date": at }));
            assert_eq!(
                out,
                json!({ "id": newest_by(&h, &ALL, at), "seconds": at }),
                "{at}"
            );
            let out = h.run("search.locate", json!({ "date": at, "tips": "head" }));
            assert_eq!(out["id"], newest_by(&h, &["HEAD"], at), "{at} from HEAD");
        }
    }
    // Before there was anything: nothing to jump to.
    let out = h.run("search.locate", json!({ "date": CLOCK_EPOCH - 1 }));
    assert_eq!(out, json!({ "id": null, "seconds": CLOCK_EPOCH - 1 }));
    // After everything: the top of the graph.
    let out = h.run("search.locate", json!({ "date": "2020-01-02" }));
    assert_eq!(out, json!({ "id": all[0], "seconds": CLOCK_EPOCH + DAY }));
    // Written as in the grammar.
    let fix = h.id("v0.2~5");
    let out = h.run("search.locate", json!({ "date": "2020-01-01T05:00" }));
    assert_eq!(out, json!({ "id": fix, "seconds": CLOCK_EPOCH + 5 * HOUR }));
    let out = h.run(
        "search.locate",
        json!({ "date": "3d", "now": CLOCK_EPOCH + 3 * DAY + 5 * HOUR }),
    );
    assert_eq!(out["id"], json!(fix));
    let out = h.run("search.locate", json!({ "date": "1d" }));
    assert_eq!(out["id"], json!(all[0]));
    // From commits that are named.
    let out = h.run(
        "search.locate",
        json!({ "date": "2020-01-02", "tips": { "ids": [lexer] } }),
    );
    assert_eq!(out["id"], json!(lexer));

    // One of the two, and something that can be read.
    for params in [
        json!({}),
        json!({ "date": null, "rev": null }),
        json!({ "date": CLOCK_EPOCH, "rev": "main" }),
        json!({ "date": "2020-01-01", "rev": "main" }),
    ] {
        let err = h.try_run("search.locate", params).unwrap_err();
        assert!(err.contains("one of the two"), "{err}");
    }
}

#[test]
fn settings_for_reading_a_log_do_not_change_the_answers() {
    let mut h = Harness::new();
    // What the same searches answer without them.
    let asked = |h: &mut Harness| {
        [
            h.run("search.commits", json!({ "query": "path:grammar.rs" })),
            h.run("search.commits", json!({ "query": "path:src" })),
            h.run("search.commits", json!({ "query": "path:*.md" })),
            h.run(
                "search.content",
                json!({ "text": "^fn", "regex": true, "paths": ["grammar.rs"] }),
            ),
            h.run(
                "search.content",
                json!({ "text": "TODO", "paths": ["grammar.rs"] }),
            ),
            h.run("search.commits", json!({ "query": "change:needle" })),
        ]
    };
    let plain = asked(&mut h);
    assert_eq!(
        plain
            .iter()
            .map(|out| ids_of(out).len())
            .collect::<Vec<_>>(),
        [1, 11, 3, 1, 0, 5]
    );

    // With them, `git log` follows the one path through its rename and
    // leaves merges out: other commits, and other numbers of them.
    let git = h.git();
    git.run(["config", "log.follow", "true"]).unwrap();
    git.run(["config", "log.showSignature", "true"]).unwrap();
    let log = |options: &[&str], path: &str| -> usize {
        let mut args = vec!["log", "--all", "--full-history", "--format=%H"];
        args.extend(options);
        args.extend(["--", path]);
        git.lines(args).unwrap().len()
    };
    assert_eq!(log(&[], "grammar.rs"), 3);
    assert_eq!(log(&[], "src"), 9);
    assert_eq!(log(&[], "*.md"), 2);
    assert_eq!(log(&["-STODO"], "grammar.rs"), 2);
    assert_eq!(asked(&mut h), plain);
}

#[test]
fn a_search_reports_and_stops_when_asked() {
    let mut h = Harness::new();
    let stages = Arc::new(Mutex::new(Vec::new()));
    let sink = stages.clone();
    h.ctx.progress = Some(Arc::new(move |progress| {
        sink.lock().unwrap().push(progress.stage);
    }));
    h.run(
        "search.commits",
        json!({ "query": "ref:main path:src change:needle fix" }),
    );
    h.run("search.content", json!({ "text": "needle" }));
    assert_eq!(
        *stages.lock().unwrap(),
        [
            "Listing reachable commits",
            "Searching paths",
            "Searching content",
            "Matching commits",
            "Searching content",
            "Matching commits",
        ]
    );

    h.ctx.cancel.store(true, Ordering::SeqCst);
    for (command, params) in [
        ("search.commits", json!({ "query": "fix" })),
        ("search.commits", json!({ "query": "is:merge" })),
        ("search.commits", json!({ "query": "ref:main" })),
        ("search.commits", json!({ "query": "path:src" })),
        ("search.commits", json!({ "query": "change:needle" })),
        ("search.commits", json!({ "query": "regex:needle" })),
        ("search.content", json!({ "text": "needle" })),
        ("search.content", json!({ "text": "needle", "regex": true })),
    ] {
        let err = h.try_run(command, params.clone()).unwrap_err();
        assert!(err.contains("cancelled"), "{command} {params}: {err}");
    }
    h.ctx.cancel.store(false, Ordering::SeqCst);
    let out = h.run("search.commits", json!({ "query": "change:needle" }));
    assert_eq!(ids_of(&out).len(), 5);
}

#[test]
fn hostile_queries_parameters_and_repositories_do_no_harm() {
    let mut h = Harness::new();
    let git = h.git();
    let refs = git.lines(["for-each-ref"]).unwrap();
    let megabytes = "a".repeat(4 * 1024 * 1024);
    let long = "x".repeat(5000);

    // Queries that cannot be read: an answer from `search.parse`, an error
    // from `search.commits`. The long ones are turned away by their length
    // and the many-worded by their count, with a short answer.
    let dashes = "-".repeat(3 * 1024 * 1024);
    let dashed_words = "-a ".repeat(100_000);
    let words = "word ".repeat(10_000);
    let keys = "author:a ".repeat(33);
    let quotes = "\"".repeat(1_000_001);
    let nested = format!("{}x", "-\"".repeat(2000));
    let long_ref = format!("ref:{}", "r".repeat(1025));
    let wide = "é".repeat(4000);
    for query in [
        megabytes.as_str(),
        dashes.as_str(),
        dashed_words.as_str(),
        words.as_str(),
        keys.as_str(),
        quotes.as_str(),
        nested.as_str(),
        long_ref.as_str(),
        wide.as_str(),
        "-",
        "--",
        "---x",
        "- -",
        "\"",
        "-\"",
        "a:\"",
        "\"\\",
        ":",
        "::",
        "a:b:c",
        "-:",
        "author:",
        "author:\"\"",
        "is:",
        "is:merge:",
        "sha:",
        "sha:g",
        "before:9999-99-99",
        "before:99999999999999999999d",
        "after:-1d",
        "after:1d1d",
        "ref:-n1",
        "ref:--all",
        "ref:--output=pwned",
        "path:--output=pwned",
        "path:-",
        "path::(exclude)src",
        "path::!src",
        "a\0b",
        "ref:a\u{7f}b",
        "ref:\"a\tb\"",
        "path:\"a\tb\"",
        "\"a\nb\"",
        "change:\"a\rb\"",
        "\u{1b}[31m",
        "issue:",
        "issue:#",
        "issue:#0",
        "issue:12345678901",
        "issue:99999999999999999999",
        "issue:!12",
        "issue:tools#12",
        "issue:a/#12",
        "issue:\"a b/c#1\"",
        "issue:\"a/b#1\u{7f}\"",
    ] {
        let label: String = query.chars().take(40).collect();
        let out = h.run("search.parse", json!({ "query": query }));
        let error = &out["error"];
        assert!(error["position"].is_u64(), "{label:?}: {out}");
        assert!(
            error["expected"]
                .as_str()
                .is_some_and(|said| said.len() < 300),
            "{label:?}"
        );
        let err = h
            .try_run("search.commits", json!({ "query": query }))
            .unwrap_err();
        assert!(
            err.len() < 600,
            "{label:?}: an error of {} bytes",
            err.len()
        );
    }

    // Queries that can be read and ask git for what it will not give.
    for query in [
        "ref:no-such-branch",
        "ref:HEAD^{tree}",
        "ref:HEAD:README.md",
        "ref:main..wip",
        "ref:^main",
        "ref:\"$(touch pwned)\"",
        "ref:`touch${IFS}pwned`",
        "ref:main;touch${IFS}pwned",
        "-ref:nowhere",
        "path:../outside",
        "path:/etc/passwd",
        "regex:(",
        "regex:[",
        "regex:\"a\\\\\"",
        "-regex:(",
        "fix regex:( author:ada",
    ] {
        let err = h
            .try_run("search.commits", json!({ "query": query }))
            .unwrap_err();
        assert!(err.len() < 2000, "{query}: an error of {} bytes", err.len());
    }
    // A regular expression git cannot read is said to be one.
    for (command, params) in [
        ("search.commits", json!({ "query": "regex:(" })),
        ("search.content", json!({ "text": "a[", "regex": true })),
    ] {
        let err = h.try_run(command, params).unwrap_err();
        assert!(err.starts_with("invalid regular expression: "), "{err}");
    }
    // And for what it has nothing of: no error, no match.
    for query in [
        "change:--output=pwned",
        "change:--all",
        "regex:--output=pwned",
        "change:-",
        "regex:-",
        "path:\"no such file\"",
        "path:\"$(touch pwned)\"",
        "path:*.none",
        "path:[",
        "change:\"\t\"",
        "\u{202e}abc",
        "\u{feff}",
        "sha:ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        "ffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff",
        wide.get(..4000).unwrap(),
    ] {
        let out = h.run("search.commits", json!({ "query": query }));
        assert_eq!(out, json!({ "ids": [], "truncated": false }), "{query}");
    }

    // Parameters of the wrong shape, and numbers nobody means.
    for (command, params) in [
        ("search.parse", json!({})),
        ("search.parse", json!({ "query": 3 })),
        ("search.parse", json!({ "query": ["a"] })),
        ("search.parse", json!({ "query": "before:3d", "now": "x" })),
        ("search.parse", json!({ "query": "before:3d", "now": 1.5 })),
        ("search.commits", json!({})),
        ("search.commits", json!({ "query": null })),
        ("search.commits", json!({ "query": { "words": [] } })),
        ("search.commits", json!({ "query": "a", "limit": -1 })),
        ("search.commits", json!({ "query": "a", "limit": "many" })),
        ("search.commits", json!({ "query": "a", "limit": 1.5 })),
        ("search.commits", json!({ "query": "a", "limit": 1e30 })),
        ("search.commits", json!({ "query": "a", "tips": 42 })),
        (
            "search.commits",
            json!({ "query": "a", "tips": "everything" }),
        ),
        (
            "search.commits",
            json!({ "query": "a", "tips": { "ids": ["zz"] } }),
        ),
        (
            "search.commits",
            json!({ "query": "a", "tips": { "ids": "main" } }),
        ),
        ("search.commits", json!({ "query": "a", "now": [] })),
        ("search.content", json!({})),
        ("search.content", json!({ "text": "" })),
        ("search.content", json!({ "text": 7 })),
        ("search.content", json!({ "text": "a\nb" })),
        ("search.content", json!({ "text": "a\0b" })),
        ("search.content", json!({ "text": megabytes })),
        ("search.content", json!({ "text": "a", "regex": "yes" })),
        ("search.content", json!({ "text": "(", "regex": true })),
        ("search.content", json!({ "text": "a", "paths": "src" })),
        ("search.content", json!({ "text": "a", "paths": [3] })),
        ("search.content", json!({ "text": "a", "paths": [""] })),
        ("search.content", json!({ "text": "a", "paths": ["-x"] })),
        (
            "search.content",
            json!({ "text": "a", "paths": ["--output=pwned"] }),
        ),
        (
            "search.content",
            json!({ "text": "a", "paths": [":(top)src"] }),
        ),
        ("search.content", json!({ "text": "a", "paths": ["a\nb"] })),
        (
            "search.content",
            json!({ "text": "a", "paths": ["src", "../outside"] }),
        ),
        (
            "search.content",
            json!({ "text": "a", "paths": [megabytes] }),
        ),
        (
            "search.content",
            json!({ "text": "a", "paths": vec!["src"; 65] }),
        ),
        ("search.content", json!({ "text": "a", "limit": -3 })),
        ("search.content", json!({ "text": "a", "tips": [] })),
        ("search.locate", json!({ "rev": "" })),
        ("search.locate", json!({ "rev": "-x" })),
        ("search.locate", json!({ "rev": "--all" })),
        ("search.locate", json!({ "rev": "--output=pwned" })),
        ("search.locate", json!({ "rev": "a\nb" })),
        ("search.locate", json!({ "rev": "main\0" })),
        ("search.locate", json!({ "rev": long })),
        ("search.locate", json!({ "rev": megabytes })),
        ("search.locate", json!({ "rev": "no-such-rev" })),
        ("search.locate", json!({ "rev": "HEAD^{tree}" })),
        ("search.locate", json!({ "rev": "v0.2:README.md" })),
        ("search.locate", json!({ "rev": "main..wip" })),
        ("search.locate", json!({ "rev": "^main" })),
        ("search.locate", json!({ "rev": 12 })),
        ("search.locate", json!({ "date": "tuesday" })),
        ("search.locate", json!({ "date": megabytes })),
        ("search.locate", json!({ "date": true })),
        ("search.locate", json!({ "date": [] })),
        ("search.locate", json!({ "date": 1.5 })),
        ("search.locate", json!({ "date": 1e30 })),
        ("search.locate", json!({ "date": "3d", "now": "then" })),
        ("search.locate", json!({ "date": 0, "tips": "nowhere" })),
    ] {
        let label: String = params.to_string().chars().take(60).collect();
        let err = h.try_run(command, params).expect_err(&label);
        assert!(
            err.len() < 2000,
            "{command} {label}: an error of {} bytes",
            err.len()
        );
    }
    // The far ends of time: an answer, not an overflow.
    for now in [i64::MIN, -1, 0, i64::MAX] {
        for query in [
            "before:4294967295y",
            "after:4294967295m -before:1m",
            "before:0d after:1w",
        ] {
            h.run("search.parse", json!({ "query": query, "now": now }));
            h.run("search.commits", json!({ "query": query, "now": now }));
        }
        h.run(
            "search.locate",
            json!({ "date": "4294967295y", "now": now }),
        );
    }
    let out = h.run("search.locate", json!({ "date": i64::MIN }));
    assert!(out["id"].is_null());
    let out = h.run("search.locate", json!({ "date": i64::MAX }));
    assert_eq!(out["id"], json!(h.id("main")));
    // Starting points that are no commits are left out, as the graph does.
    let (tree, blob) = (
        git.rev_parse("HEAD^{tree}").unwrap(),
        git.rev_parse("HEAD:README.md").unwrap(),
    );
    let gone = "1".repeat(40);
    for tips in [
        json!([tree]),
        json!([blob, h.id("pages")]),
        json!([gone]),
        json!([]),
    ] {
        let tips = json!({ "ids": tips });
        for query in ["needle", "change:needle", "path:index.html ref:pages"] {
            let _ = h.try_run("search.commits", json!({ "query": query, "tips": tips }));
        }
        let _ = h.try_run("search.content", json!({ "text": "needle", "tips": tips }));
        let _ = h.try_run("search.locate", json!({ "date": 0, "tips": tips }));
    }
    let out = h.run(
        "search.commits",
        json!({ "query": "change:needle", "tips": { "ids": [blob, h.id("pages")] } }),
    );
    assert_eq!(ids_of(&out), [h.id("pages")]);
    // Thousands of them: git is told the few that reach all the others.
    let many: Vec<String> = h
        .graph(json!("all"))
        .into_iter()
        .cycle()
        .take(5000)
        .collect();
    for query in ["path:src", "change:needle -ref:main", "tokenizer"] {
        let out = h.run(
            "search.commits",
            json!({ "query": query, "tips": { "ids": many } }),
        );
        assert!(!ids_of(&out).is_empty(), "{query}");
        assert_eq!(out, h.run("search.commits", json!({ "query": query })));
    }
    let out = h.run(
        "search.content",
        json!({ "text": "needle", "tips": { "ids": many } }),
    );
    assert_eq!(out, h.run("search.content", json!({ "text": "needle" })));

    // Nothing was run, written or moved.
    assert_eq!(git.lines(["for-each-ref"]).unwrap(), refs);
    assert_eq!(git.run(["status", "--porcelain"]).unwrap(), "");
    for stray in ["pwned", "src/pwned", "../pwned"] {
        assert!(!h.copy.path().join(stray).exists(), "{stray} was written");
    }
    assert!(h.run("undo.peek", json!({}))["undo"].is_null());

    // Without a repository: a query can still be read, the rest say so.
    let registry = builtin_registry();
    let out = registry
        .run(
            "search.parse",
            &mut CommandContext::default(),
            &json!({ "query": "-is:merge" }),
        )
        .unwrap();
    assert_eq!(out["words"][0]["negated"], true);
    for (command, params) in [
        ("search.commits", json!({ "query": "fix" })),
        ("search.content", json!({ "text": "fix" })),
        ("search.locate", json!({ "rev": "HEAD" })),
        ("search.locate", json!({ "date": 0 })),
    ] {
        let err = registry
            .run(command, &mut CommandContext::default(), &params)
            .unwrap_err();
        assert_eq!(err.to_string(), "no repository is open", "{command}");
    }

    // Hundreds of histories with nothing in common, each of them named:
    // more than git is told on one command line.
    let mut apart = Harness::new();
    let mut stream = String::new();
    for n in 0..600 {
        stream.push_str(&format!(
            "commit refs/heads/apart/{n}\n\
             committer Ada Fixture <ada@fixture.invalid> {} +0000\n\
             data 5\napart\n",
            CLOCK_EPOCH + 100 * HOUR + n
        ));
    }
    let mut import = apart.git().command();
    import
        .args(["fast-import", "--quiet"])
        .stdin(Stdio::piped());
    let mut child = import.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(stream.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    let roots = apart
        .git()
        .lines(["for-each-ref", "--format=%(objectname)", "refs/heads/apart"])
        .unwrap();
    assert_eq!(roots.len(), 600);
    for (command, params) in [
        (
            "search.commits",
            json!({ "query": "path:src", "tips": { "ids": roots } }),
        ),
        (
            "search.commits",
            json!({ "query": "-change:needle", "tips": { "ids": roots } }),
        ),
        (
            "search.content",
            json!({ "text": "needle", "tips": { "ids": roots } }),
        ),
    ] {
        let err = apart.try_run(command, params).unwrap_err();
        assert!(err.contains("512 separate branches at most"), "{err}");
    }
    // Words that do not ask git take any number of them, and git takes
    // them when it is told "all" rather than each one.
    let out = apart.run(
        "search.commits",
        json!({ "query": "message:apart", "tips": { "ids": roots } }),
    );
    assert_eq!(ids_of(&out).len(), 600);
    let out = apart.run("search.commits", json!({ "query": "path:src" }));
    assert_eq!(ids_of(&out).len(), 11);
    let out = apart.run(
        "search.commits",
        json!({ "query": "-path:src", "tips": { "ids": roots[..512] } }),
    );
    assert_eq!(ids_of(&out).len(), 512);

    // Repositories that are not as expected. What comes back may be an
    // answer or an error; it is never a crash.
    let prod = |h: &mut Harness| {
        for query in [
            "",
            "fix",
            "author:ada -is:merge",
            "ref:HEAD",
            "ref:main path:src",
            "change:needle",
            "before:1d",
        ] {
            let _ = h.try_run("search.commits", json!({ "query": query }));
            let _ = h.try_run("search.commits", json!({ "query": query, "tips": "head" }));
        }
        let _ = h.try_run(
            "search.content",
            json!({ "text": "needle", "paths": ["src"] }),
        );
        let _ = h.try_run("search.locate", json!({ "date": CLOCK_EPOCH + 5 * HOUR }));
        let _ = h.try_run("search.locate", json!({ "rev": "HEAD" }));
        let _ = h.try_run("search.locate", json!({ "rev": "main" }));
    };

    // No commits at all: nothing is found, and nothing is wrong.
    let mut empty = Harness::of("linear");
    empty
        .git()
        .run(["update-ref", "-d", "refs/heads/main"])
        .unwrap();
    for query in ["", "fix", "ref:HEAD", "change:needle path:src", "-is:merge"] {
        let out = empty.run("search.commits", json!({ "query": query }));
        assert_eq!(out, json!({ "ids": [], "truncated": false }), "{query}");
    }
    let out = empty.run("search.content", json!({ "text": "a" }));
    assert_eq!(out, json!({ "ids": [], "truncated": false }));
    let out = empty.run("search.locate", json!({ "date": 0 }));
    assert!(out["id"].is_null());
    assert!(
        empty
            .try_run("search.locate", json!({ "rev": "HEAD" }))
            .is_err()
    );
    prod(&mut empty);

    // References that hold garbage, nothing, something that is gone, a
    // tree; a branch named like an option.
    let mut odd = Harness::new();
    let dir = odd.copy.path().join(".git");
    let tree = odd.git().rev_parse("HEAD^{tree}").unwrap();
    std::fs::write(dir.join("refs/heads/garbage"), "not an id\n").unwrap();
    std::fs::write(dir.join("refs/heads/empty"), "").unwrap();
    std::fs::write(dir.join("refs/heads/gone"), format!("{gone}\n")).unwrap();
    std::fs::write(dir.join("refs/heads/tree"), format!("{tree}\n")).unwrap();
    std::fs::write(dir.join("refs/tags/-n1"), format!("{}\n", odd.id("main"))).unwrap();
    prod(&mut odd);
    for rev in ["garbage", "empty", "gone", "tree", "-n1"] {
        assert!(
            odd.try_run("search.locate", json!({ "rev": rev })).is_err(),
            "{rev}"
        );
        assert!(
            odd.try_run("search.commits", json!({ "query": format!("ref:{rev}") }))
                .is_err(),
            "ref:{rev}"
        );
    }
    // What can still be read is still searched.
    let out = odd.run("search.commits", json!({ "query": "is:merge" }));
    assert_eq!(ids_of(&out).len(), 3);

    // A commit in the middle of history that cannot be read.
    let mut torn = Harness::new();
    let middle = torn.id("main~3");
    let object = torn
        .copy
        .path()
        .join(".git/objects")
        .join(&middle[..2])
        .join(&middle[2..]);
    std::fs::remove_file(&object).unwrap();
    std::fs::write(&object, b"this is not a zlib stream").unwrap();
    prod(&mut torn);
    std::fs::remove_file(&object).unwrap();
    prod(&mut torn);

    // HEAD and the packed references as garbage; then no objects.
    let mut lost = Harness::new();
    let dir = lost.copy.path().join(".git");
    std::fs::write(dir.join("HEAD"), b"\0\xff garbage").unwrap();
    std::fs::write(dir.join("packed-refs"), b"\xff\xfe not refs\n^^^\n").unwrap();
    prod(&mut lost);
    std::fs::write(dir.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    std::fs::remove_dir_all(dir.join("objects")).unwrap();
    prod(&mut lost);
    std::fs::remove_dir_all(&dir).unwrap();
    prod(&mut lost);
}

/// What `git log` lists for a date written the way a person writes it,
/// read by a git that runs in the time zone `zone`.
fn git_in_zone(h: &Harness, zone: &str, option: &str, date: &str) -> Ids {
    let mut args = vec!["log".to_owned(), "--format=%H".to_owned()];
    args.push(format!("--{option}={}", date.replace('T', " ")));
    args.extend(ALL.iter().map(|tip| (*tip).to_owned()));
    let zone = git_zone(zone).unwrap();
    h.git()
        .run_with(args, &[("TZ", zone)])
        .unwrap()
        .lines()
        .map(str::to_owned)
        .collect()
}

/// What git is given as `TZ` for the zone `name`. Git for Windows reads
/// `TZ` as the C runtime of Windows does: a name and the hours west of
/// UTC, and no zone of the database, which it takes for UTC. There a
/// zone is given by the offset it has in the days the tests ask about
/// (none of these has summer time then), and one that cannot be given
/// so (Auckland, 13 hours east) has nothing to be compared with.
fn git_zone(name: &str) -> Option<&str> {
    if !cfg!(windows) {
        return Some(name);
    }
    match name {
        "UTC" => Some("UTC0"),
        "Europe/Vienna" => Some("CET-1"),
        "America/New_York" => Some("EST5"),
        "Asia/Kolkata" => Some("IST-5:30"),
        "Pacific/Honolulu" => Some("HST10"),
        _ => None,
    }
}

/// A date in a query is read in a time zone, as `git log
/// --before` and `--after` read theirs; the same words name other
/// commits in Vienna than in Auckland.
#[test]
fn dates_are_read_in_a_time_zone_as_git_log_reads_them() {
    let mut h = Harness::new();
    let everything = h.everything();
    // The fixture's commits are an hour apart from 2020-01-01T00:00Z. A
    // time at seventeen past is never a commit's own, in any zone, so
    // that "before" means the same to git (which takes the instant in)
    // and to a query (which does not).
    let times = [
        "2019-12-31T12:17",
        "2019-12-31T20:17",
        "2020-01-01T00:17",
        "2020-01-01T05:17",
        "2020-01-01T09:17",
        "2020-01-01T13:17",
        "2020-01-01T23:17",
        "2020-01-02T08:17",
    ];
    let mut answers: BTreeSet<Vec<usize>> = BTreeSet::new();
    for zone in [
        "UTC",
        "Europe/Vienna",
        "America/New_York",
        "Asia/Kolkata",
        "Pacific/Auckland",
        "Pacific/Honolulu",
    ] {
        // A system without that zone in its database: git has none to
        // compare with either.
        if h.try_run("search.parse", json!({ "query": "x", "zone": zone }))
            .is_err()
            || git_zone(zone).is_none()
        {
            continue;
        }
        let mut sizes = Vec::new();
        for time in times {
            let before = git_in_zone(&h, zone, "before", time);
            let after = git_in_zone(&h, zone, "after", time);
            assert_eq!(
                before.len() + after.len(),
                everything.len(),
                "{zone} {time}"
            );
            for (word, want) in [("before", &before), ("after", &after)] {
                let query = format!("{word}:{time}");
                let out = h.run("search.commits", json!({ "query": query, "zone": zone }));
                let got: Ids = ids_of(&out).into_iter().collect();
                assert_eq!(&got, want, "{query} in {zone}");
            }
            sizes.push(before.len());
        }
        // A date alone is the beginning of that day in the zone: what
        // git lists up to the last second of the day before.
        let midnight = git_in_zone(&h, zone, "before", "2019-12-31 23:59:59");
        let out = h.run(
            "search.commits",
            json!({ "query": "before:2020-01-01", "zone": zone }),
        );
        assert_eq!(
            ids_of(&out).into_iter().collect::<Ids>(),
            midnight,
            "the day begins in {zone}"
        );
        answers.insert(sizes);
    }
    assert!(
        answers.len() >= 4,
        "the zones tell the commits apart: {answers:?}"
    );

    // Without a zone it is the system's, which is the one git has when
    // it is run as this process is: with its `TZ`, or without one.
    let own = |h: &Harness, option: &str, time: &str| -> Ids {
        let mut command = h.git().command();
        match std::env::var_os("TZ") {
            Some(zone) => command.env("TZ", zone),
            None => command.env_remove("TZ"),
        };
        let out = command
            .args(["log", "--format=%H"])
            .arg(format!("--{option}={}", time.replace('T', " ")))
            .args(ALL)
            .output()
            .unwrap();
        assert!(out.status.success());
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(str::to_owned)
            .collect()
    };
    for time in times {
        for word in ["before", "after"] {
            // Past the harness, which would say UTC.
            let out = h
                .registry
                .run(
                    "search.commits",
                    &mut h.ctx,
                    &json!({ "query": format!("{word}:{time}") }),
                )
                .unwrap();
            assert_eq!(
                ids_of(&out).into_iter().collect::<Ids>(),
                own(&h, word, time),
                "{word}:{time} in the system's zone"
            );
        }
    }

    // Jumping to a date reads it the same way.
    for zone in ["UTC", "Asia/Kolkata"] {
        let Ok(parsed) = h.try_run(
            "search.parse",
            json!({ "query": "before:2020-01-01T05:17", "zone": zone }),
        ) else {
            continue;
        };
        let located = h.run(
            "search.locate",
            json!({ "date": "2020-01-01T05:17", "zone": zone }),
        );
        assert_eq!(located["seconds"], parsed["words"][0]["seconds"], "{zone}");
    }

    // What is no zone is refused, and nothing is searched.
    for zone in [
        json!(""),
        json!("Europe/Nowhere"),
        json!("../../etc/localtime"),
        json!("/etc/passwd"),
        json!("x".repeat(500)),
        json!(7),
        json!(["UTC"]),
    ] {
        for (command, params) in [
            (
                "search.parse",
                json!({ "query": "before:3d", "zone": zone }),
            ),
            (
                "search.commits",
                json!({ "query": "before:3d", "zone": zone }),
            ),
            (
                "search.locate",
                json!({ "date": "2020-01-01", "zone": zone }),
            ),
        ] {
            assert!(
                h.try_run(command, params.clone()).is_err(),
                "{command} {params}"
            );
        }
    }
}

/// People are the ones the mailmap makes of them, as `git log
/// --author` and `--committer` have it.
#[test]
fn people_are_matched_as_the_mailmap_has_them() {
    let mut h = Harness::new();
    let git = h.git();
    let people = [
        ("author", "bob"),
        ("author", "Bob Fixture"),
        ("author", "bob@fixture.invalid"),
        ("author", "robert"),
        ("author", "Robert Builder <rob@"),
        ("author", "rob@example.invalid"),
        ("author", "ada"),
        ("author", "Ada Fixture"),
        ("author", "lovelace"),
        ("author", "countess <ada@fixture"),
        ("author", "carol"),
        ("author", "carol@example.invalid"),
        ("author", "carol@new.invalid"),
        ("author", "fixture.invalid>"),
        ("committer", "bob"),
        ("committer", "robert"),
        ("committer", "lovelace"),
        ("committer", "Ada Fixture"),
    ];
    let check = |h: &mut Harness, what: &str| -> Vec<usize> {
        let mut sizes = Vec::new();
        for (key, text) in people {
            let want = h.grep(key, text);
            let quoted = format!("{key}:\"{text}\"");
            h.assert_finds(&quoted, &want);
            // And the other way round: everybody else.
            let rest: Ids = h.everything().difference(&want).cloned().collect();
            h.assert_finds(&format!("-{quoted}"), &rest);
            sizes.push(want.len());
        }
        // Free text looks at the person too.
        for text in ["robert", "lovelace", "bob"] {
            let want = h.text(text);
            h.assert_finds(text, &want);
        }
        assert!(sizes.iter().any(|size| *size > 0), "{what}");
        sizes
    };
    let unmapped = check(&mut h, "without a mailmap");

    // A mailmap in the worktree, of every form the file has: a name and
    // an address for a name and an address, a name for an address, an
    // address for an address.
    h.copy
        .write(
            ".mailmap",
            "# who is who\n\
             Robert Builder <rob@example.invalid> Bob Fixture <bob@fixture.invalid>\n\
             Ada Lovelace, Countess <ada@fixture.invalid>\n\
             <carol@new.invalid> <carol@example.invalid>\n",
        )
        .unwrap();
    let mapped = check(&mut h, "with a mailmap");
    assert_ne!(mapped, unmapped, "the mailmap changes who is found");
    // Bob's commits are Robert's now, and no longer anybody called Bob's.
    let bobs = h.grep("author", "bob");
    assert!(bobs.is_empty(), "{bobs:?}");
    assert!(!h.grep("author", "robert").is_empty());
    assert!(!h.grep("author", "carol@new.invalid").is_empty());

    // Switched off for the log, it is off here as it is for git.
    git.run(["config", "log.mailmap", "false"]).unwrap();
    assert_eq!(check(&mut h, "log.mailmap = false"), unmapped);
    git.run(["config", "--unset", "log.mailmap"]).unwrap();

    // A file that is named, on top of the one in the worktree: what it
    // says of the same person is what holds, and the rest of the other
    // file with it. (A mailmap is looked up once: what one line makes
    // of a person is not looked up again in another.)
    let named = h.copy.path().with_extension("mailmap");
    std::fs::write(
        &named,
        "Roberta Builder <roberta@example.invalid> Bob Fixture <bob@fixture.invalid>\n\
         Nobody <nobody@example.invalid> <rob@example.invalid>\n",
    )
    .unwrap();
    git.run(["config", "mailmap.file", named.to_str().unwrap()])
        .unwrap();
    let want = h.grep("author", "roberta");
    assert!(!want.is_empty());
    h.assert_finds("author:roberta", &want);
    for (key, text) in [
        ("author", "Robert Builder <rob@"),
        ("author", "nobody"),
        ("author", "lovelace"),
        ("committer", "roberta@example.invalid"),
    ] {
        let want = h.grep(key, text);
        h.assert_finds(&format!("{key}:\"{text}\""), &want);
    }
    git.run(["config", "--unset", "mailmap.file"]).unwrap();
    let _ = std::fs::remove_file(&named);

    // A mailmap that is no mailmap: an answer, as git gives one.
    for garbage in [
        "\0\u{1}\u{2} not a mailmap\n".to_owned(),
        "<<<>>> <<>>\n<\n>\n".to_owned(),
        format!("{} <x@y>\n", "N".repeat(200_000)),
        "Proper Name <proper@example.invalid> <ada@fixture.invalid>".repeat(2000),
    ] {
        h.copy.write(".mailmap", &garbage).unwrap();
        let out = h.try_run("search.commits", json!({ "query": "author:ada" }));
        assert!(out.is_ok(), "{out:?}");
    }
    h.copy.write(".mailmap", "").unwrap();
    assert_eq!(check(&mut h, "an empty mailmap"), unmapped);
}
