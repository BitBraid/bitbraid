//! Link commands through the registry: what a link is read as and what
//! is made, with and without a repository open, and the desktop entry
//! (the key of the registry, on Windows) that has the system hand links
//! to the application.
//!
//! The entry is written to a scratch directory or not at all, and
//! `xdg-mime` is never run: the desktop of whoever runs the tests stays
//! as it is. On Windows the command only ever says what it would write;
//! the key itself is written under a name of the test's own by the unit
//! tests of `link_cmds`. Every call of `link.register` goes through
//! [`Harness::register`], which sees to that.

use std::{
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

use bitbraid_engine::{
    CommandContext, Registry, builtin_registry,
    link_cmds::{LinkError, LinkRepository, LinkTarget, parse_link},
};
use bitbraid_testkit::{Scratch, fixtures_out_dir};
use serde_json::{Value, json};

static COUNTER: AtomicU64 = AtomicU64::new(0);

struct Harness {
    /// A directory of the test's own among the scratch copies.
    dir: PathBuf,
    registry: Registry,
    ctx: CommandContext,
}

impl Harness {
    fn new() -> Self {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = fixtures_out_dir()
            .join("scratch")
            .join(format!("links-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Self {
            dir,
            registry: builtin_registry(),
            ctx: CommandContext::default(),
        }
    }
    fn text(&self, name: &str) -> String {
        self.dir.join(name).to_str().unwrap().to_owned()
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
    /// `link.register`, only ever in a way that leaves the desktop alone:
    /// as a dry run, or into this test's directory without `xdg-mime`.
    #[cfg(target_os = "linux")]
    fn register(&mut self, params: Value) -> Result<Value, String> {
        let dry = params["dry_run"] == true;
        let scratch = params["no_mime"] == true
            && params["applications_dir"]
                .as_str()
                .is_some_and(|dir| dir.starts_with(self.dir.to_str().unwrap()));
        assert!(dry || scratch, "{params} could reach the desktop");
        self.try_run("link.register", params)
    }
    /// `link.register`, only ever as a dry run: the user's own key is
    /// never written, and on macOS no bundle is made known to the system.
    #[cfg(any(windows, target_os = "macos"))]
    fn register(&mut self, params: Value) -> Result<Value, String> {
        assert!(
            params["dry_run"] == true,
            "{params} could write the user's own key"
        );
        self.try_run("link.register", params)
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn links_are_read_and_made_without_a_repository() {
    let mut h = Harness::new();
    assert_eq!(
        h.run(
            "link.parse",
            json!({ "url": "bitbraid://open?path=/home/ada/my%20work&branch=feature%2Fa" })
        ),
        json!({
            "repository": { "path": "/home/ada/my work" },
            "target": { "kind": "branch", "value": "feature/a" },
        })
    );
    assert_eq!(
        h.run(
            "link.parse",
            json!({ "url": "bitbraid://open?url=git%40example.invalid%3Aa%2Fb.git&pr=128" })
        ),
        json!({
            "repository": { "url": "git@example.invalid:a/b.git" },
            "target": { "kind": "pr", "value": "128" },
        })
    );
    assert_eq!(
        h.run("link.parse", json!({ "url": "bitbraid://open?path=/a" }))["target"],
        json!({ "kind": "none", "value": null })
    );
    // As a launcher hands it over that has taken it apart on the way.
    assert_eq!(
        h.run(
            "link.parse",
            json!({ "url": "bitbraid://open/?path=/a%20b&branch=x%26y" })
        ),
        json!({
            "repository": { "path": "/a b" },
            "target": { "kind": "branch", "value": "x&y" },
        })
    );

    assert_eq!(
        h.run(
            "link.make",
            json!({ "path": "/home/ada/my work", "branch": "feature/a&b" })
        ),
        json!({ "link": "bitbraid://open?path=/home/ada/my%20work&branch=feature/a%26b" })
    );
    assert_eq!(
        h.run(
            "link.make",
            json!({ "url": "https://example.invalid/a/b.git", "tag": "v1.0" })
        ),
        json!({ "link": "bitbraid://open?url=https%3A//example.invalid/a/b.git&tag=v1.0" })
    );
    assert_eq!(
        h.run("link.make", json!({ "path": "/a", "commit": "ABCDEF0123" })),
        json!({ "link": "bitbraid://open?path=/a&commit=abcdef0123" })
    );
    assert_eq!(
        h.run("link.make", json!({ "path": "/a" })),
        json!({ "link": "bitbraid://open?path=/a" })
    );

    // What is made reads back as what went in, whatever is in it.
    let repositories = [
        ("path", "/plain"),
        ("path", "/with space/a&b/c=d/e#f/100%/g?h/i+j"),
        ("path", "/quotes \"double\" 'single' `back`/$HOME/\\"),
        ("path", "/ünï/ключ/鍵/🦑"),
        ("path", r"C:\Users\Ada Lovelace\my repo"),
        ("path", "D:/work/a&b"),
        ("path", "/%20/%2F/%zz/%"),
        ("url", "https://example.invalid/a/b.git"),
        (
            "url",
            "https://example.invalid/path with space/b.git?x=1&y=2",
        ),
        ("url", "ssh://git@example.invalid:2222/~ada/b.git"),
        ("url", "git@example.invalid:ünï/b c.git"),
    ];
    let targets = [
        None,
        Some(("branch", "main")),
        Some(("branch", "feature/a&b=c#d%e+f g")),
        Some(("branch", "ünï/ветка")),
        Some(("tag", "v1.0")),
        Some(("tag", "release candidate #2 (100%)")),
        Some(("commit", "abcdef0")),
        Some(("commit", "0123456789abcdef0123456789abcdef01234567")),
    ];
    for (place, repository) in repositories {
        for target in targets {
            let mut params = json!({ place: repository });
            if let Some((kind, value)) = target {
                params[kind] = json!(value);
            }
            let made = h.run("link.make", params.clone());
            let link = made["link"].as_str().unwrap();
            assert!(
                link.is_ascii() && !link.contains([' ', '#', '"', '\'', '\\', '+']),
                "{link}"
            );
            let (kind, value) =
                target.map_or(("none", Value::Null), |(kind, value)| (kind, json!(value)));
            assert_eq!(
                h.run("link.parse", json!({ "url": link })),
                json!({ "repository": { place: repository }, "target": { "kind": kind, "value": value } }),
                "{params} as {link}"
            );
        }
    }

    // With no repository open there is none to stand in.
    h.refused("link.make", json!({}), "say which repository");
    h.refused(
        "link.make",
        json!({ "branch": "main" }),
        "say which repository",
    );
}

#[test]
fn a_link_to_the_open_repository_names_what_is_in_it() {
    let mut h = Harness::new();
    let copy = Scratch::of("branches-merges").unwrap();
    let git = copy.git();
    h.run("repo.open", json!({ "path": copy.path() }));
    let here = copy.path().to_str().unwrap();
    let head = git.rev_parse("HEAD").unwrap();

    let read = |h: &mut Harness, params: Value| {
        let made = h.run("link.make", params);
        h.run("link.parse", json!({ "url": made["link"] }))
    };
    let plain = read(&mut h, json!({}));
    assert_eq!(
        plain,
        json!({ "repository": { "path": here }, "target": { "kind": "none", "value": null } })
    );
    // A branch and a tag of the fixture, as git knows them.
    let branch = read(&mut h, json!({ "branch": "feature/a" }));
    assert_eq!(branch["repository"]["path"], here);
    assert_eq!(
        branch["target"],
        json!({ "kind": "branch", "value": "feature/a" })
    );
    let named = branch["target"]["value"].as_str().unwrap();
    assert!(
        git.refs("refs/heads")
            .unwrap()
            .iter()
            .any(|name| name == named)
    );
    let tag = read(&mut h, json!({ "tag": "v1.0" }));
    assert_eq!(tag["target"], json!({ "kind": "tag", "value": "v1.0" }));
    assert!(
        git.refs("refs/tags")
            .unwrap()
            .iter()
            .any(|name| name == "v1.0")
    );
    // A commit by its id, whole, cut short, or shouted: git finds the
    // same commit by what the link says.
    for id in [head.clone(), head[..7].to_owned(), head.to_uppercase()] {
        let commit = read(&mut h, json!({ "commit": id }));
        assert_eq!(commit["target"]["kind"], "commit");
        let value = commit["target"]["value"].as_str().unwrap();
        assert_eq!(value, id.to_lowercase());
        assert_eq!(git.rev_parse(&format!("{value}^{{commit}}")).unwrap(), head);
    }
    // Another repository when one is said, by path or by address.
    let other = read(
        &mut h,
        json!({ "path": "/somewhere/else", "branch": "main" }),
    );
    assert_eq!(other["repository"], json!({ "path": "/somewhere/else" }));
    let remote = read(&mut h, json!({ "url": "git@example.invalid:a/b.git" }));
    assert_eq!(
        remote["repository"],
        json!({ "url": "git@example.invalid:a/b.git" })
    );
}

#[test]
fn what_is_not_a_link_of_ours_is_refused_by_the_command() {
    let mut h = Harness::new();
    let long = format!("bitbraid://open?path=/{}", "a".repeat(5000));
    for (link, because) in [
        ("", "does not begin with bitbraid://"),
        (
            "https://example.invalid/",
            "does not begin with bitbraid://",
        ),
        (
            "bitbraid://clone?url=https://example.invalid/a",
            "something other than open",
        ),
        ("bitbraid://open", "names no repository"),
        ("bitbraid://open?branch=main", "names no repository"),
        (
            "bitbraid://open?path=/a&url=https://example.invalid/a",
            "names two repositories",
        ),
        (
            "bitbraid://open?path=/a&branch=b&tag=t",
            "more than one of branch, tag, commit and pr",
        ),
        (
            "bitbraid://open?path=/a&commit=abc",
            "not 7 to 64 hex digits",
        ),
        (
            "bitbraid://open?path=/a&commit=zzzzzzz",
            "not 7 to 64 hex digits",
        ),
        (
            "bitbraid://open?path=/a%0Ab",
            "path has a control character",
        ),
        (
            "bitbraid://open?path=/a&branch=b%00",
            "branch has a control character",
        ),
        ("bitbraid://open?path=/a&path=/a", "path is given twice"),
        ("bitbraid://open?path=/a%2", "percent sign"),
        ("bitbraid://open?path=/a%FF%FE", "not UTF-8"),
        ("bitbraid://open?path=relative", "not a full path"),
        ("bitbraid://open?path=//server/share", "another machine"),
        (
            "bitbraid://open?url=ext::sh%20-c%20id",
            "not an address a link may name",
        ),
        (
            "bitbraid://open?url=--upload-pack%3Did",
            "not an address a link may name",
        ),
        (
            "bitbraid://open?url=https://ada:secret@example.invalid/a",
            "password",
        ),
        (
            "bitbraid://open?path=/a&branch=--force",
            "branch begins with a dash",
        ),
        (
            "bitbraid://open?path=/a&pr=one",
            "not the number of a pull request",
        ),
        (
            "bitbraid://open?path=/a&line=3",
            "\"line\" is not a parameter",
        ),
        ("bitbraid://open?path=/a#b", "fragment"),
        ("bitbraid://open?path=/a b", "blank or a control character"),
        (
            "bitbraid://open?path=/a\n&branch=b",
            "blank or a control character",
        ),
        (long.as_str(), "longer than 4096 bytes"),
    ] {
        h.refused("link.parse", json!({ "url": link }), because);
        // The command says what the function says.
        let why = parse_link(link).unwrap_err();
        h.refused("link.parse", json!({ "url": link }), &why.to_string());
    }
    for params in [
        json!({}),
        json!({ "url": null }),
        json!({ "url": 3 }),
        json!({ "url": ["bitbraid://open?path=/a"] }),
        json!({ "url": { "path": "/a" } }),
        json!({ "link": "bitbraid://open?path=/a" }),
    ] {
        assert!(h.try_run("link.parse", params.clone()).is_err(), "{params}");
    }
    // The function is there for the application, before anything is open.
    assert_eq!(parse_link("bitbraid://clone"), Err(LinkError::Action));
    let link = parse_link("bitbraid://open?path=/a&tag=v1").unwrap();
    assert_eq!(link.repository, LinkRepository::Path("/a".to_owned()));
    assert_eq!(link.target, LinkTarget::Tag("v1".to_owned()));
}

#[test]
fn a_link_that_could_not_be_read_is_not_made() {
    let mut h = Harness::new();
    let long = "x".repeat(5000);
    for (params, because) in [
        (
            json!({ "path": "/a", "url": "https://example.invalid/a" }),
            "path or url, not both",
        ),
        (
            json!({ "path": "/a", "branch": "b", "tag": "t" }),
            "at most one of",
        ),
        (
            json!({ "path": "/a", "branch": "b", "commit": "abcdef0" }),
            "at most one of",
        ),
        (
            json!({ "path": "/a", "tag": "t", "commit": "abcdef0" }),
            "at most one of",
        ),
        (json!({ "path": "relative" }), "not a full path"),
        (json!({ "path": "." }), "not a full path"),
        (json!({ "path": "~/work" }), "not a full path"),
        (json!({ "path": "" }), "path has no value"),
        (json!({ "path": "/two\nlines" }), "control character"),
        (json!({ "path": "/nul\u{0}" }), "control character"),
        (
            json!({ "path": "/turned\u{202e}round" }),
            "control character",
        ),
        (
            json!({ "path": "\\\\server\\share\\repo" }),
            "another machine",
        ),
        (
            json!({ "path": format!("/{long}") }),
            "longer than 4096 bytes",
        ),
        (
            json!({ "url": "/srv/git/a.git" }),
            "not an address a link may name",
        ),
        (
            json!({ "url": "file:///srv/git/a.git" }),
            "not an address a link may name",
        ),
        (
            json!({ "url": "ext::sh -c 'touch pwned'" }),
            "not an address a link may name",
        ),
        (
            json!({ "url": "-oProxyCommand=touch pwned" }),
            "not an address a link may name",
        ),
        (
            json!({ "url": "https://ada:hunter2@example.invalid/a.git" }),
            "password",
        ),
        (json!({ "path": "/a", "branch": "" }), "branch has no value"),
        (
            json!({ "path": "/a", "branch": "-D" }),
            "begins with a dash",
        ),
        (
            json!({ "path": "/a", "tag": "--delete" }),
            "begins with a dash",
        ),
        (
            json!({ "path": "/a", "tag": "two\nlines" }),
            "control character",
        ),
        (
            json!({ "path": "/a", "commit": "HEAD" }),
            "not 7 to 64 hex digits",
        ),
        (
            json!({ "path": "/a", "commit": "abc" }),
            "not 7 to 64 hex digits",
        ),
        (
            json!({ "path": "/a", "commit": "abcdef0~1" }),
            "not 7 to 64 hex digits",
        ),
        (
            json!({ "path": "/a", "commit": "a".repeat(65) }),
            "not 7 to 64 hex digits",
        ),
        (
            json!({ "path": "/a", "branch": long }),
            "longer than 4096 bytes",
        ),
    ] {
        h.refused("link.make", params, because);
    }
    for params in [
        json!({ "path": 3 }),
        json!({ "path": ["/a"] }),
        json!({ "url": { "a": 1 } }),
        json!({ "path": "/a", "branch": 3 }),
        json!({ "path": "/a", "tag": false }),
        json!({ "path": "/a", "commit": 1234567 }),
    ] {
        assert!(h.try_run("link.make", params.clone()).is_err(), "{params}");
    }
    assert!(!std::path::Path::new("pwned").exists());
}

/// The program of an `Exec` line as a shell reads it. What an entry puts
/// on top of the shell's quoting is taken off first: a backslash is
/// written twice in any value.
#[cfg(target_os = "linux")]
fn program_of(entry: &str) -> String {
    let exec = entry
        .lines()
        .find_map(|line| line.strip_prefix("Exec="))
        .unwrap();
    let program = exec.strip_suffix(" %u").unwrap();
    assert!(!program.contains('%'), "{program} has a field code in it");
    let program = program.replace("\\\\", "\\");
    let out = std::process::Command::new("sh")
        .arg("-c")
        .arg(format!("printf '%s' {program}"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{program}");
    String::from_utf8(out.stdout).unwrap()
}

#[cfg(target_os = "linux")]
#[test]
fn registering_writes_a_desktop_entry_and_leaves_the_desktop_alone() {
    let mut h = Harness::new();
    let applications = h.text("share/applications");
    let file = format!("{applications}/bitbraid-url.desktop");
    // Where the entry would go for real, and what is there now.
    let real = h.register(json!({ "dry_run": true })).ok().map(|out| {
        let file = out["file"].as_str().unwrap().to_owned();
        assert!(
            file.ends_with("/applications/bitbraid-url.desktop"),
            "{file}"
        );
        assert!(!file.starts_with(h.dir.to_str().unwrap()));
        // The running program, when none is said.
        let running = std::env::current_exe().unwrap();
        assert_eq!(
            program_of(out["entry"].as_str().unwrap()),
            running.to_str().unwrap()
        );
        let stamp = std::fs::metadata(&file)
            .and_then(|meta| meta.modified())
            .ok();
        (file, stamp)
    });

    std::fs::create_dir_all(h.dir.join("bin")).unwrap();
    for name in [
        "bitbraid",
        "bit braid",
        "it's \"quoted\" `here`",
        "$HOME $(touch pwned) ${x}",
        "back\\slash\\",
        "a;b&c|d>e<f*g?h#i~j(k)",
        "ünï ключ 鍵",
    ] {
        let exe = h.text(&format!("bin/{name}"));
        std::fs::write(&exe, "").unwrap();

        // A dry run says what would be written, and writes nothing.
        let dry = h
            .register(json!({ "exe": exe, "applications_dir": applications, "dry_run": true }))
            .unwrap();
        assert_eq!(dry["file"], file.as_str());
        assert_eq!(dry["dry_run"], true);
        assert_eq!(dry["written"], false);
        assert_eq!(dry["default_set"], false);
        assert!(dry["problem"].is_null());
        assert!(!h.dir.join("share").exists(), "a dry run made something");
        let entry = dry["entry"].as_str().unwrap();
        assert_eq!(
            entry.lines().collect::<Vec<_>>()[..3],
            ["[Desktop Entry]", "Type=Application", "Name=BitBraid"]
        );
        assert!(
            entry.contains("\nMimeType=x-scheme-handler/bitbraid;\n"),
            "{entry}"
        );
        assert_eq!(entry.lines().count(), 7, "{entry}");
        assert_eq!(program_of(entry), exe, "{entry}");

        // For real, into the test's own directory and without xdg-mime.
        let out = h
            .register(json!({ "exe": exe, "applications_dir": applications, "no_mime": true }))
            .unwrap();
        assert_eq!(
            out,
            json!({
                "file": file, "entry": entry, "dry_run": false,
                "written": true, "default_set": false, "problem": null,
            })
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), entry);
        let left: Vec<String> = std::fs::read_dir(&applications)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, ["bitbraid-url.desktop"]);
        // The checker of the specification's authors, where it is installed.
        if let Ok(checked) = std::process::Command::new("desktop-file-validate")
            .arg(&file)
            .output()
        {
            assert!(
                checked.status.success(),
                "{entry}\n{}",
                String::from_utf8_lossy(&checked.stdout)
            );
        }
        std::fs::remove_dir_all(h.dir.join("share")).unwrap();
    }
    assert!(!h.dir.join("pwned").exists() && !std::path::Path::new("pwned").exists());

    // An entry that is there is replaced as a whole.
    let exe = h.text("bin/bitbraid");
    std::fs::create_dir_all(&applications).unwrap();
    std::fs::write(
        &file,
        "[Desktop Entry]\nName=Something Older\nExec=/bin/false %u\n",
    )
    .unwrap();
    let out = h
        .register(json!({ "exe": exe, "applications_dir": applications, "no_mime": true }))
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        out["entry"].as_str().unwrap()
    );

    // What cannot be registered is refused, and the entry stays.
    let entry = std::fs::read(&file).unwrap();
    let long = format!("/{}", "x".repeat(5000));
    let in_the_way = h.text("bin/bitbraid/applications");
    let a_directory = h.text("bin");
    for (exe, because) in [
        ("", "must not be empty"),
        ("bitbraid", "full path"),
        ("./bitbraid", "full path"),
        ("bin/bitbraid", "full path"),
        ("/line\nbreak/bitbraid", "control characters"),
        (
            "/usr/bin/bitbraid\nExec=/bin/sh -c id",
            "control characters",
        ),
        ("/nul\0", "control characters"),
        ("/with=equals/bitbraid", "with = in its path"),
        ("/100%/bitbraid", "with % in its path"),
        ("/field %u code/bitbraid", "with % in its path"),
        ("/no/such/program/anywhere", "there is no program at"),
        (a_directory.as_str(), "there is no program at"),
        (long.as_str(), "too long"),
    ] {
        for guard in ["dry_run", "no_mime"] {
            let mut params = json!({ "exe": exe, "applications_dir": applications });
            params[guard] = json!(true);
            let err = h.register(params.clone()).expect_err(exe);
            assert!(err.contains(because), "{params}: {err}");
        }
    }
    let exe = h.text("bin/bitbraid");
    for (directory, because) in [
        ("", "must not be empty"),
        ("relative", "full path"),
        ("./applications", "full path"),
        ("~/.local/share/applications", "full path"),
        ("/line\nbreak", "control characters"),
        (long.as_str(), "too long"),
    ] {
        let params = json!({ "exe": exe, "applications_dir": directory, "dry_run": true });
        let err = h.register(params.clone()).expect_err(directory);
        assert!(err.contains(because), "{params}: {err}");
    }
    // A file where the directory would have to be made.
    let err = h
        .register(json!({ "exe": exe, "applications_dir": in_the_way, "no_mime": true }))
        .unwrap_err();
    assert!(err.contains("cannot be made"), "{err}");
    for params in [
        json!({ "exe": 3, "dry_run": true }),
        json!({ "exe": ["/bin/true"], "dry_run": true }),
        json!({ "applications_dir": 3, "dry_run": true }),
        json!({ "applications_dir": applications, "no_mime": "yes", "dry_run": true }),
        json!({ "applications_dir": applications, "no_mime": true, "dry_run": "yes" }),
        json!({ "applications_dir": applications, "no_mime": true, "dry_run": 1 }),
    ] {
        assert!(h.register(params.clone()).is_err(), "{params}");
    }
    assert_eq!(std::fs::read(&file).unwrap(), entry);
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "");

    // Nothing was written where the desktop looks.
    if let Some((file, stamp)) = real {
        assert_eq!(
            std::fs::metadata(&file)
                .and_then(|meta| meta.modified())
                .ok(),
            stamp,
            "{file} was touched"
        );
    }
}

/// The user's own key for the scheme, read and never written: whether it
/// is there, and its command.
#[cfg(windows)]
fn users_own_key() -> (bool, Option<String>) {
    use windows_registry::CURRENT_USER;
    let there = CURRENT_USER.open(r"Software\Classes\bitbraid").is_ok();
    let command = CURRENT_USER
        .open(r"Software\Classes\bitbraid\shell\open\command")
        .and_then(|key| key.get_string(""))
        .ok();
    (there, command)
}

#[cfg(windows)]
#[test]
fn registering_on_windows_says_what_it_would_write() {
    let mut h = Harness::new();
    let before = users_own_key();

    // The running program, when none is said.
    let running = std::env::current_exe().unwrap();
    let running = running.to_str().unwrap();
    let command = format!("\"{running}\" \"%1\"");
    assert_eq!(
        h.register(json!({ "dry_run": true })).unwrap(),
        json!({
            "key": r"HKEY_CURRENT_USER\Software\Classes\bitbraid",
            "command": command,
            "values": [
                { "key": "", "name": "", "data": "URL:BitBraid link" },
                { "key": "DefaultIcon", "name": "", "data": format!("{running},0") },
                { "key": r"shell\open\command", "name": "", "data": command },
                { "key": "", "name": "URL Protocol", "data": "" },
            ],
            "dry_run": true,
            "written": false,
        })
    );

    // A program with blanks and the like in its path.
    let bin = h.dir.join("Bit Braid (x86)");
    std::fs::create_dir_all(&bin).unwrap();
    for name in [
        "bitbraid.exe",
        "bit braid.exe",
        "it's & that; (here).exe",
        "ünï 鍵.exe",
    ] {
        let exe = bin.join(name);
        std::fs::write(&exe, "").unwrap();
        let exe = exe.to_str().unwrap();
        let out = h.register(json!({ "exe": exe, "dry_run": true })).unwrap();
        assert_eq!(out["command"], format!("\"{exe}\" \"%1\""));
        assert_eq!(out["values"][1]["data"], format!("{exe},0"));
        assert_eq!(out["written"], false);
    }

    // What cannot be registered is refused.
    let long = format!(r"C:\{}", "x".repeat(5000));
    let a_directory = bin.to_str().unwrap().to_owned();
    for (exe, because) in [
        ("", "must not be empty"),
        ("bitbraid.exe", "full path"),
        (r".\bitbraid.exe", "full path"),
        (r"bin\bitbraid.exe", "full path"),
        ("C:bitbraid.exe", "full path"),
        (r"\Program Files\bitbraid.exe", "full path"),
        ("/usr/bin/bitbraid", "full path"),
        ("C:\\line\nbreak\\bitbraid.exe", "control characters"),
        ("C:\\nul\0.exe", "control characters"),
        (r#"C:\with"quote\bitbraid.exe"#, "double quote"),
        (r"C:\100%\bitbraid.exe", "with % in its path"),
        (r"C:\no\such\program\anywhere.exe", "there is no program at"),
        (a_directory.as_str(), "there is no program at"),
        (long.as_str(), "too long"),
    ] {
        let err = h
            .register(json!({ "exe": exe, "dry_run": true }))
            .expect_err(exe);
        assert!(err.contains(because), "{exe:?}: {err}");
    }
    // What is for Linux alone is refused here.
    let applications = h.text("applications");
    for params in [
        json!({ "applications_dir": applications, "dry_run": true }),
        json!({ "no_mime": true, "dry_run": true }),
    ] {
        let err = h.register(params.clone()).unwrap_err();
        assert!(err.contains("Linux"), "{params}: {err}");
    }
    for params in [
        json!({ "exe": 3, "dry_run": true }),
        json!({ "exe": [r"C:\bitbraid.exe"], "dry_run": true }),
        json!({ "no_mime": "yes", "dry_run": true }),
    ] {
        assert!(h.register(params.clone()).is_err(), "{params}");
    }
    assert!(!h.dir.join("applications").exists());

    // The user's own key is as it was.
    assert_eq!(users_own_key(), before);
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
#[test]
fn registering_is_the_installers_work_elsewhere() {
    let mut h = Harness::new();
    let dir = h.text("applications");
    for params in [
        json!({ "dry_run": true }),
        json!({ "applications_dir": dir, "no_mime": true }),
    ] {
        h.refused("link.register", params, "installer");
    }
    assert!(!h.dir.join("applications").exists());
}

/// A bundle of the test's own, as `cargo xtask bundle` lays one out, with
/// a program that is no BitBraid.
#[cfg(target_os = "macos")]
fn bundle_in(dir: &std::path::Path, name: &str) -> std::path::PathBuf {
    let bundle = dir.join(name);
    let macos = bundle.join("Contents/MacOS");
    std::fs::create_dir_all(&macos).unwrap();
    std::fs::write(bundle.join("Contents/Info.plist"), "<plist/>").unwrap();
    let exe = macos.join("bitbraid");
    std::fs::write(&exe, "").unwrap();
    exe
}

#[cfg(target_os = "macos")]
#[test]
fn registering_on_macos_names_the_bundle_the_program_is_in() {
    let mut h = Harness::new();

    // The test program is in no bundle: the system has nothing to take
    // the scheme from.
    let err = h.register(json!({ "dry_run": true })).unwrap_err();
    assert!(err.contains("no application bundle"), "{err}");

    // A program in a bundle: the bundle is what is registered, wherever
    // it is and whatever its path holds.
    for name in ["BitBraid.app", "Bit Braid (copy).app", "it's ünï 鍵.APP"] {
        let exe = bundle_in(&h.dir, name);
        let bundle = h.dir.join(name);
        let out = h
            .register(json!({ "exe": exe.to_str().unwrap(), "dry_run": true }))
            .unwrap();
        assert_eq!(out["bundle"], bundle.to_str().unwrap());
        assert_eq!(out["command"][1], "-f");
        assert_eq!(out["command"][2], bundle.to_str().unwrap());
        assert!(out["command"][0].as_str().unwrap().ends_with("/lsregister"));
        assert_eq!(out["written"], false);
    }

    // What cannot be registered is refused.
    let loose = h.dir.join("loose");
    std::fs::create_dir_all(&loose).unwrap();
    std::fs::write(loose.join("bitbraid"), "").unwrap();
    let not_there = h.dir.join("Gone.app/Contents/MacOS/bitbraid");
    let no_plist = h.dir.join("Bare.app/Contents/MacOS/bitbraid");
    std::fs::create_dir_all(no_plist.parent().unwrap()).unwrap();
    std::fs::write(&no_plist, "").unwrap();
    let long = format!("/{}.app/Contents/MacOS/bitbraid", "x".repeat(5000));
    for (exe, because) in [
        (
            loose.join("bitbraid").to_str().unwrap().to_owned(),
            "no application bundle",
        ),
        (
            "/Applications/BitBraid.app/Contents/bitbraid".to_owned(),
            "no application bundle",
        ),
        (
            not_there.to_str().unwrap().to_owned(),
            "there is no application",
        ),
        (
            no_plist.to_str().unwrap().to_owned(),
            "there is no application",
        ),
        (
            "/A.app/Contents/MacOS/line\nbreak".to_owned(),
            "no path of a program",
        ),
        (long, "no path of a program"),
    ] {
        let err = h
            .register(json!({ "exe": exe, "dry_run": true }))
            .expect_err(&exe);
        assert!(err.contains(because), "{exe:?}: {err}");
    }
    // What is for Linux alone is refused here.
    let applications = h.text("applications");
    for params in [
        json!({ "applications_dir": applications, "dry_run": true }),
        json!({ "no_mime": true, "dry_run": true }),
    ] {
        let err = h.register(params.clone()).unwrap_err();
        assert!(err.contains("Linux"), "{params}: {err}");
    }
    assert!(!h.dir.join("applications").exists());
}
