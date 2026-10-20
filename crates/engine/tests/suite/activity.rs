//! The activity log and the hook list, on scratch copies. The record of
//! git processes is one for the whole test binary, and the tests run side
//! by side: each looks only at the entries of its own scratch directory,
//! which is what the commands list.

use std::path::{Path, PathBuf};

use bitbraid_engine::{
    CommandContext, CommandError, CommandSpec, Output, Params, Registry, builtin_registry,
};
use bitbraid_git::exe;
use bitbraid_testkit::{Git, Scratch};
use serde_json::{Value, json};

struct Harness {
    scratch: Scratch,
    registry: Registry,
    ctx: CommandContext,
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

fn make_executable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // Elsewhere files carry no such mark: git runs a hook that is a file.
    #[cfg(not(unix))]
    let _ = path;
}

impl Harness {
    fn new() -> Self {
        Self::with(builtin_registry())
    }
    fn with(registry: Registry) -> Self {
        let scratch = Scratch::of("linear").unwrap();
        let git = scratch.git();
        // Commands run git with the real environment: what a commit needs
        // is set here, and the developer's own signing is kept out.
        for (key, value) in [
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
        ] {
            git.run(["config", key, value]).unwrap();
        }
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
    fn repo(&self) -> &bitbraid_git::Repository {
        self.ctx.repo.as_ref().unwrap()
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
    /// The entries `activity.list` gives.
    fn log(&mut self, params: Value) -> Vec<Value> {
        self.run("activity.list", params)["entries"]
            .as_array()
            .unwrap()
            .clone()
    }
    /// Everything recorded for this repository, questions included.
    fn all(&mut self) -> Vec<Value> {
        self.log(json!({ "questions": true, "limit": 2000 }))
    }
    /// The newest entry whose command line begins with `start`.
    fn entry(&mut self, start: &str) -> Value {
        self.all()
            .into_iter()
            .find(|entry| entry["command"].as_str().unwrap().starts_with(start))
            .unwrap_or_else(|| panic!("no entry for {start:?}"))
    }
    /// A hooks directory of this test's own, with `core.hooksPath`
    /// pointing at it: whatever the developer has configured for
    /// themselves, these are the hooks that run.
    fn hooks_dir(&self) -> PathBuf {
        let dir = self.scratch.path().join(".git").join("test-hooks");
        std::fs::create_dir_all(&dir).unwrap();
        self.git()
            .run(["config", "core.hooksPath", dir.to_str().unwrap()])
            .unwrap();
        dir
    }
    fn hook(&self, dir: &Path, name: &str, script: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{script}\n")).unwrap();
        make_executable(&path);
        path
    }
}

fn text<'a>(entry: &'a Value, field: &str) -> &'a str {
    entry[field]
        .as_str()
        .unwrap_or_else(|| panic!("{field} of {entry}"))
}

#[test]
fn a_mutation_shows_with_its_command_line_exit_and_duration() {
    let mut h = Harness::new();
    let head = h.git().rev_parse("HEAD").unwrap();

    let before = now_ms();
    h.run("tag.create", json!({ "name": "v-one" }));
    let after = now_ms();

    let entries = h.log(json!({}));
    let entry = entries
        .iter()
        .find(|entry| text(entry, "command").starts_with("git tag "))
        .unwrap_or_else(|| panic!("{entries:?}"));
    assert_eq!(entry["command"], format!("git tag -- v-one {head}"));
    assert_eq!(entry["end"], "exited");
    assert_eq!(entry["code"], 0);
    assert_eq!(entry["origin"], "tag.create");
    assert_eq!(entry["question"], false);
    assert_eq!(entry["stdout"], "");
    assert_eq!(entry["stderr"], "");
    assert_eq!(entry["stdout_cut"], false);
    assert_eq!(entry["stderr_cut"], false);
    let started = entry["started"].as_u64().unwrap();
    let took = entry["duration_ms"].as_u64().unwrap();
    assert!(before <= started && started <= after, "{entry}");
    assert!(started + took <= after + 1, "{entry}");
    assert!(entry["id"].as_u64().unwrap() > 0);
    assert_eq!(
        Path::new(text(entry, "directory")).canonicalize().unwrap(),
        h.scratch.path().canonicalize().unwrap()
    );

    // Newest first, and nothing that only asks.
    let ids: Vec<u64> = entries.iter().map(|e| e["id"].as_u64().unwrap()).collect();
    assert!(ids.windows(2).all(|pair| pair[0] > pair[1]), "{ids:?}");
    assert!(entries.iter().all(|entry| entry["question"] == false));

    // The questions are there when asked for: the checks made on the way,
    // under the same command.
    let all = h.all();
    assert!(all.len() > entries.len());
    for asked in [
        "git check-ref-format refs/tags/v-one",
        "git rev-parse --verify --quiet refs/tags/v-one",
    ] {
        let entry = all
            .iter()
            .find(|entry| entry["command"] == asked)
            .unwrap_or_else(|| panic!("no entry for {asked:?}"));
        assert_eq!(entry["question"], true, "{entry}");
        assert_eq!(entry["origin"], "tag.create", "{entry}");
    }
    // A question's answer: the tag did not exist before, and does now.
    let answers: Vec<(i64, &str)> = all
        .iter()
        .filter(|entry| entry["command"] == "git rev-parse --verify --quiet refs/tags/v-one")
        .map(|entry| (entry["code"].as_i64().unwrap(), text(entry, "stdout")))
        .collect();
    assert_eq!(answers, [(0, head.as_str()), (1, "")]);

    // A limit keeps the newest, and says how many there would be.
    let out = h.run("activity.list", json!({ "limit": 1, "questions": true }));
    assert_eq!(out["entries"].as_array().unwrap().len(), 1);
    assert_eq!(out["entries"][0]["id"], all[0]["id"]);
    assert_eq!(out["matched"], all.len());
    let none = h.run("activity.list", json!({ "limit": 0 }));
    assert_eq!(none["entries"], json!([]));
    assert_eq!(none["matched"], entries.len());

    // Text is looked for in the command line and the output, whatever the
    // case of its letters.
    let found = h.log(json!({ "text": "TAG -- V-ONE" }));
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0]["id"], entry["id"]);
    let found = h.log(json!({ "text": &head[..12], "questions": true }));
    assert!(found.len() >= 2, "in a command line and in an answer");
    assert!(h.log(json!({ "text": "no such text anywhere" })).is_empty());
}

#[test]
fn a_failing_call_shows_its_error_and_a_hook_its_message() {
    let mut h = Harness::new();
    let hooks = h.hooks_dir();
    let hook = h.hook(
        &hooks,
        "pre-commit",
        "echo 'lint: a.txt has a line that is too long' >&2\nexit 1",
    );

    h.scratch.write("a.txt", "2\nand a long line\n").unwrap();
    h.run("stage.paths", json!({ "paths": ["a.txt"] }));
    // The error says that it was the hook, which one and what it said,
    // however quick the hook was: the message goes to git on its standard
    // input, and a hook that is quick enough has ended git before it was
    // written. The input that could not be written is not the error.
    let err = h
        .try_run("commit.create", json!({ "message": "blocked" }))
        .unwrap_err();
    assert!(err.contains("the pre-commit hook"), "{err}");
    assert!(err.contains("a.txt has a line that is too long"), "{err}");
    assert!(!err.contains("stdin"), "{err}");

    // What the hook printed is with the commit that ran it, either way.
    let entry = h.entry("git commit ");
    assert_eq!(entry["end"], "exited");
    assert_eq!(entry["code"], 1);
    assert_eq!(entry["origin"], "commit.create");
    assert_eq!(entry["question"], false);
    assert_eq!(
        text(&entry, "stderr"),
        "lint: a.txt has a line that is too long"
    );
    // And it is found by what it said, in the list as it is by default.
    let found = h.log(json!({ "text": "too long" }));
    assert_eq!(found.len(), 1);
    assert_eq!(found[0]["id"], entry["id"]);

    // The hook is one of those that will run.
    let listed = h.run("hook.list", json!({}));
    assert_eq!(listed["configured"], true);
    assert_eq!(
        listed["hooks"],
        json!([{ "name": "pre-commit", "path": hook }])
    );

    // Git failing by itself, outside any command.
    assert!(
        exe::run(
            h.repo(),
            ["rev-parse", "--verify", "no-such-revision"],
            None
        )
        .is_err()
    );
    let entry = h.entry("git rev-parse --verify no-such-revision");
    assert_eq!(entry["code"], 128);
    assert_eq!(entry["origin"], Value::Null);
    assert_eq!(entry["question"], true);
    assert!(text(&entry, "stderr").starts_with("fatal: "), "{entry}");

    // Git gone before it was given its input, for certain this time: more
    // input than a pipe holds, for a command that reads none. The writing
    // fails, and what the call fails with is how git ended and what it
    // said, which is the reason; the entry has the same.
    let input = vec![b'x'; 1024 * 1024];
    let err = exe::run(h.repo(), ["no-such-command"], Some(&input)).unwrap_err();
    assert!(err.to_string().contains("is not a git command"), "{err}");
    assert!(!err.to_string().contains("stdin"), "{err}");
    let entry = h.entry("git no-such-command");
    assert_eq!(entry["end"], "exited");
    assert_eq!(entry["code"], 1);
    assert!(
        text(&entry, "stderr").contains("is not a git command"),
        "{entry}"
    );

    // Skipping the hook: no message, and the commit is made.
    h.run(
        "commit.create",
        json!({ "message": "allowed", "no_verify": true }),
    );
    let entry = h.entry("git commit ");
    assert_eq!(entry["code"], 0);
    assert!(text(&entry, "command").contains("--no-verify"));
    assert_eq!(entry["stderr"], "");
}

#[test]
fn credentials_in_urls_never_show() {
    let mut h = Harness::new();
    let url = "https://ada:hunter2@example.invalid/repo.git";
    let shown = "https://***@example.invalid/repo.git";

    // In the arguments.
    h.run("remote.add", json!({ "name": "cred", "url": url }));
    let entry = h.entry("git remote add ");
    assert_eq!(entry["command"], format!("git remote add -- cred {shown}"));

    // In what git answers.
    let out = exe::run(h.repo(), ["config", "--get", "remote.cred.url"], None).unwrap();
    assert_eq!(
        out.stdout.trim(),
        url,
        "the caller still gets what git said"
    );
    let entry = h.entry("git config --get remote.cred.url");
    assert_eq!(entry["stdout"], shown);

    // In what git complains about.
    assert!(exe::run(h.repo(), ["ls-files", "--error-unmatch", "--", url], None).is_err());
    let entry = h.entry("git ls-files --error-unmatch ");
    assert_eq!(
        entry["command"],
        format!("git ls-files --error-unmatch -- {shown}")
    );
    assert!(text(&entry, "stderr").contains(shown), "{entry}");

    // In what a hook prints.
    let hooks = h.hooks_dir();
    h.hook(
        &hooks,
        "pre-commit",
        &format!("echo 'pushing a copy to {url} first' >&2\nexit 1"),
    );
    assert!(
        h.try_run(
            "commit.create",
            json!({ "message": "blocked", "allow_empty": true })
        )
        .is_err()
    );
    let entry = h.entry("git commit ");
    assert_eq!(
        text(&entry, "stderr"),
        format!("pushing a copy to {shown} first")
    );

    // Nowhere in anything recorded for this repository, and not found
    // when looked for either.
    let all = h.all();
    assert!(all.len() >= 4);
    let dump = serde_json::to_string(&all).unwrap();
    assert!(!dump.contains("hunter2"), "{dump}");
    assert!(dump.matches("***@example.invalid").count() >= 5, "{dump}");
    assert!(
        h.log(json!({ "text": "hunter2", "questions": true }))
            .is_empty()
    );
    assert!(
        h.log(json!({ "text": "EXAMPLE.invalid", "questions": true }))
            .len()
            >= 4
    );
}

#[test]
fn output_larger_than_the_bound_is_cut_and_marked() {
    let mut h = Harness::new();
    let bound = bitbraid_git::activity::OUTPUT_LIMIT;

    // Standard output: a file of two-byte characters, more than twice the
    // bound, with an odd byte in front so that the cut falls inside one.
    let content = format!("a{}\nTHE END.\n", "é".repeat(20_000));
    h.scratch.write("big.txt", &content).unwrap();
    h.run("stage.paths", json!({ "paths": ["big.txt"] }));
    let out = exe::run(h.repo(), ["cat-file", "-p", ":big.txt"], None).unwrap();
    assert_eq!(out.stdout, content, "the caller gets all of it");
    let entry = h.entry("git cat-file -p :big.txt");
    let kept = text(&entry, "stdout");
    assert_eq!(entry["stdout_cut"], true);
    assert_eq!(entry["stderr_cut"], false);
    assert!(
        kept.len() <= bound && kept.len() > bound - 4,
        "{}",
        kept.len()
    );
    assert!(kept.starts_with('é') && kept.ends_with("é\nTHE END."));

    // Standard error: a hook with a lot to say.
    let hooks = h.hooks_dir();
    h.hook(
        &hooks,
        "pre-commit",
        "i=0\nwhile [ $i -lt 1500 ]; do echo \"report line $i: something is wrong here\" >&2; i=$((i+1)); done\necho 'THE LAST LINE' >&2\nexit 1",
    );
    assert!(
        h.try_run("commit.create", json!({ "message": "blocked" }))
            .is_err()
    );
    let entry = h.entry("git commit ");
    let kept = text(&entry, "stderr");
    assert_eq!(entry["stderr_cut"], true);
    assert!(
        kept.len() <= bound && kept.len() > bound - 4,
        "{}",
        kept.len()
    );
    assert!(kept.ends_with("THE LAST LINE"), "the end is what is kept");
    assert!(!kept.contains("report line 0:"));

    // What fits is left whole.
    assert_eq!(entry["stdout_cut"], false);
}

/// A command that runs another one in between two git calls of its own.
fn outer(ctx: &mut CommandContext, _: &Params) -> Result<Output, CommandError> {
    exe::run(ctx.repo()?, ["tag", "before-inner"], None)?;
    builtin_registry().run("tag.create", ctx, &json!({ "name": "inner" }))?;
    exe::run(ctx.repo()?, ["tag", "after-inner"], None)?;
    Ok(Value::Null)
}

#[test]
fn entries_carry_the_engine_command_also_when_one_runs_another() {
    let mut registry = builtin_registry();
    registry
        .register(
            CommandSpec {
                id: "test.outer",
                label: "Outer",
                description: "Runs tag.create between two tags of its own",
            },
            outer,
        )
        .unwrap();
    let mut h = Harness::with(registry);

    exe::run(h.repo(), ["tag", "before-any"], None).unwrap();
    h.run("test.outer", json!({}));
    exe::run(h.repo(), ["tag", "after-all"], None).unwrap();
    h.run("tag.delete", json!({ "name": "inner" }));

    let origin = |h: &mut Harness, start: &str| h.entry(start)["origin"].clone();
    assert_eq!(origin(&mut h, "git tag before-any"), Value::Null);
    assert_eq!(origin(&mut h, "git tag before-inner"), "test.outer");
    assert_eq!(origin(&mut h, "git tag -- inner "), "tag.create");
    assert_eq!(origin(&mut h, "git tag after-inner"), "test.outer");
    assert_eq!(origin(&mut h, "git tag after-all"), Value::Null);
    assert_eq!(
        origin(&mut h, "git update-ref -d refs/tags/inner"),
        "tag.delete"
    );
    // Every entry made inside the inner command is the inner command's.
    let all = h.all();
    let of = |name: &str| all.iter().filter(|entry| entry["origin"] == name).count();
    assert_eq!(of("test.outer"), 2);
    assert!(of("tag.create") >= 3 && of("tag.delete") >= 2, "{all:?}");

    // A command that fails gives its name back as well.
    assert!(h.try_run("tag.create", json!({ "name": "inner" })).is_ok());
    assert!(h.try_run("tag.create", json!({ "name": "inner" })).is_err());
    exe::run(h.repo(), ["tag", "after-failure"], None).unwrap();
    assert_eq!(origin(&mut h, "git tag after-failure"), Value::Null);
}

#[test]
fn clear_forgets_this_repository_only() {
    let mut ours = Harness::new();
    let mut other = Harness::new();
    ours.run("tag.create", json!({ "name": "ours" }));
    other.run("tag.create", json!({ "name": "theirs" }));
    let before = ours.all();
    let theirs = other.all();
    assert!(!before.is_empty() && !theirs.is_empty());
    // Each sees its own only.
    assert!(
        before
            .iter()
            .all(|e| !text(e, "command").contains("theirs"))
    );
    assert!(theirs.iter().all(|e| !text(e, "command").contains("ours")));

    let out = ours.run("activity.clear", json!({}));
    assert_eq!(out["cleared"], before.len());
    assert!(ours.all().is_empty());
    assert_eq!(other.all(), theirs, "another repository keeps its entries");
    assert_eq!(ours.run("activity.clear", json!({}))["cleared"], 0);

    // What comes afterwards is recorded as before, under later numbers.
    ours.run("tag.delete", json!({ "name": "ours" }));
    let after = ours.all();
    assert!(!after.is_empty());
    let newest_before = before[0]["id"].as_u64().unwrap();
    assert!(
        after
            .iter()
            .all(|entry| entry["id"].as_u64().unwrap() > newest_before)
    );
}

#[test]
fn a_tool_is_one_entry_running_until_it_is_closed() {
    let mut h = Harness::new();
    let git = h.git();
    for key in ["diff.tool", "diff.guitool", "merge.tool", "merge.guitool"] {
        git.run(["config", key, ""]).unwrap();
    }
    h.scratch.write("a.txt", "2\nchanged\n").unwrap();
    let gate = h.scratch.path().join(".git").join("tool-may-close");
    let tool = format!(
        "echo 'the tool has a remark' >&2; while [ ! -e '{}' ]; do sleep 0.05; done",
        gate.display()
    );

    // Started and left open: there, and marked as running.
    h.run("tool.diff", json!({ "path": "a.txt", "command": tool }));
    let entry = h.entry("git --literal-pathspecs -c difftool.");
    assert_eq!(entry["end"], "running");
    assert_eq!(entry["code"], Value::Null);
    assert_eq!(entry["origin"], "tool.diff");
    assert_eq!(entry["question"], false);
    let id = entry["id"].clone();

    // Closed: the same entry, completed.
    std::fs::write(&gate, "").unwrap();
    let started = std::time::Instant::now();
    let entry = loop {
        let entry = h.entry("git --literal-pathspecs -c difftool.");
        if entry["end"] != "running" {
            break entry;
        }
        assert!(
            started.elapsed() < std::time::Duration::from_secs(20),
            "the tool was never collected"
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    assert_eq!(entry["id"], id);
    assert_eq!(entry["end"], "exited");
    assert_eq!(entry["code"], 0);
    assert_eq!(entry["stderr"], "the tool has a remark");
    assert!(entry["duration_ms"].as_u64().unwrap() >= 400, "{entry}");
    let tools = h
        .all()
        .iter()
        .filter(|entry| text(entry, "command").contains("difftool"))
        .count();
    assert_eq!(tools, 1);
}

/// Whether the developer running the tests has a hooks directory
/// configured for all their repositories. Commands run git with their
/// configuration, so the repository's own directory is then not the one
/// that counts.
fn hooks_configured_outside(h: &Harness) -> bool {
    let (code, _) =
        exe::run_with_code(h.repo(), ["config", "--get", "core.hooksPath"], None).unwrap();
    code == Some(0)
}

fn names(listed: &Value) -> Vec<&str> {
    listed["hooks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|hook| text(hook, "name"))
        .collect()
}

fn same_place(listed: &Value, expected: &Path) {
    let directory = Path::new(text(listed, "directory"));
    assert_eq!(
        directory.canonicalize().unwrap(),
        expected.canonicalize().unwrap()
    );
}

#[test]
fn hooks_are_listed_where_git_looks_for_them() {
    let mut h = Harness::new();
    let git = h.git();
    let root = h.scratch.path().to_path_buf();
    let own = root.join(".git").join("hooks");
    std::fs::create_dir_all(&own).unwrap();

    // The repository's own directory: what git would run, and nothing
    // else that lies there.
    let pre_commit = h.hook(&own, "pre-commit", "exit 0");
    h.hook(&own, "commit-msg.sample", "exit 0");
    h.hook(&own, "not-a-hook", "exit 0");
    std::fs::write(own.join("pre-push"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::create_dir_all(own.join("post-merge")).unwrap();
    make_executable(&own.join("post-merge"));
    let outside = hooks_configured_outside(&h);
    let listed = h.run("hook.list", json!({}));
    assert_eq!(listed["configured"], outside);
    if !outside {
        same_place(&listed, &own);
        // Where files carry no executable mark (Windows), git runs every
        // hook that is a file, the one that is not marked too.
        if cfg!(unix) {
            assert_eq!(names(&listed), ["pre-commit"]);
        } else {
            assert_eq!(names(&listed), ["pre-commit", "pre-push"]);
        }
        assert_eq!(
            Path::new(text(&listed["hooks"][0], "path"))
                .canonicalize()
                .unwrap(),
            pre_commit.canonicalize().unwrap()
        );
        // Git agrees: that hook runs, and the others do not.
        git.run(["hook", "run", "pre-commit"]).unwrap();
        assert_eq!(git.run(["hook", "run", "pre-push"]).is_ok(), !cfg!(unix));
    }

    // `core.hooksPath`, relative: taken from the top of the worktree. The
    // repository's own directory no longer counts.
    git.run(["config", "core.hooksPath", ".githooks"]).unwrap();
    let configured = root.join(".githooks");
    std::fs::create_dir_all(&configured).unwrap();
    h.hook(&configured, "pre-push", "exit 0");
    h.hook(&configured, "commit-msg", "exit 0");
    h.hook(&configured, "post-checkout.sample", "exit 0");
    let listed = h.run("hook.list", json!({}));
    assert_eq!(listed["configured"], true);
    same_place(&listed, &configured);
    assert_eq!(names(&listed), ["commit-msg", "pre-push"], "in git's order");

    // A linked worktree: a relative path is taken from its own top.
    let linked = root.with_extension("linked");
    let _ = std::fs::remove_dir_all(&linked);
    git.run([
        "worktree",
        "add",
        "-q",
        "-b",
        "elsewhere",
        linked.to_str().unwrap(),
    ])
    .unwrap();
    let mut there = CommandContext::default();
    h.registry
        .run("repo.open", &mut there, &json!({ "path": linked }))
        .unwrap();
    let listed = h.registry.run("hook.list", &mut there, &json!({})).unwrap();
    assert_eq!(listed["configured"], true);
    assert!(names(&listed).is_empty(), "{listed}");
    std::fs::create_dir_all(linked.join(".githooks")).unwrap();
    h.hook(&linked.join(".githooks"), "post-commit", "exit 0");
    let listed = h.registry.run("hook.list", &mut there, &json!({})).unwrap();
    same_place(&listed, &linked.join(".githooks"));
    assert_eq!(names(&listed), ["post-commit"]);

    // Without the setting it shares the hooks of the repository it
    // belongs to.
    git.run(["config", "--unset", "core.hooksPath"]).unwrap();
    let listed = h.registry.run("hook.list", &mut there, &json!({})).unwrap();
    assert_eq!(listed["configured"], outside);
    if !outside {
        same_place(&listed, &own);
        // (On Windows the hook that is not marked runs as well.)
        if cfg!(unix) {
            assert_eq!(names(&listed), ["pre-commit"]);
        } else {
            assert_eq!(names(&listed), ["pre-commit", "pre-push"]);
        }
    }

    // An absolute path, and one that is no directory at all.
    git.run(["config", "core.hooksPath", configured.to_str().unwrap()])
        .unwrap();
    let listed = h.registry.run("hook.list", &mut there, &json!({})).unwrap();
    same_place(&listed, &configured);
    assert_eq!(names(&listed), ["commit-msg", "pre-push"]);
    git.run(["config", "core.hooksPath", "/dev/null"]).unwrap();
    let listed = h.run("hook.list", json!({}));
    assert_eq!(listed["configured"], true);
    // On Windows a full path without a drive is one on the drive git runs
    // in.
    let null = if cfg!(windows) {
        format!("{}\\dev\\null", &root.to_str().unwrap()[..2])
    } else {
        "/dev/null".to_owned()
    };
    assert_eq!(listed["directory"], null.as_str());
    assert!(names(&listed).is_empty());

    // The linked worktree's git processes are its own in the log.
    let here = h.all();
    let there_log = h
        .registry
        .run("activity.list", &mut there, &json!({ "questions": true }))
        .unwrap();
    let there_log = there_log["entries"].as_array().unwrap();
    assert!(!there_log.is_empty());
    for entry in there_log {
        assert_eq!(entry["origin"], "hook.list");
        assert!(!here.contains(entry), "{entry}");
    }

    drop(there);
    let _ = std::fs::remove_dir_all(&linked);
}

/// A repository in a directory whose name is not UTF-8. A path like that
/// cannot be written as JSON text as it is; it is shown with the bytes
/// that are no text replaced, and nothing fails over it.
#[cfg(unix)]
#[test]
fn a_directory_whose_name_is_not_text_is_listed_all_the_same() {
    use std::os::unix::ffi::OsStrExt;

    let scratch = Scratch::of("linear").unwrap();
    let name = format!("not-text-{}-", std::process::id());
    let mut bytes = name.clone().into_bytes();
    bytes.extend_from_slice(b"\xff\xfe");
    let odd = scratch
        .path()
        .with_file_name(std::ffi::OsStr::from_bytes(&bytes));
    let _ = std::fs::remove_dir_all(&odd);
    if std::fs::rename(scratch.path(), &odd).is_err() {
        // A file system that takes no such name has no such repository.
        return;
    }
    let git = Git::new(&odd, bitbraid_testkit::fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git.run(["config", "core.hooksPath", ".githooks"]).unwrap();
    std::fs::create_dir_all(odd.join(".githooks")).unwrap();
    let hook = odd.join(".githooks").join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    make_executable(&hook);

    // Opened by its path as it is: a command's parameters are text and
    // could not name it.
    let registry = builtin_registry();
    let mut ctx = CommandContext {
        repo: Some(bitbraid_git::Repository::open(&odd).unwrap()),
        ..CommandContext::default()
    };
    let shown = format!("{name}\u{fffd}\u{fffd}");

    // Git ran there, the hook is found there, and both say where.
    let listed = registry.run("hook.list", &mut ctx, &json!({})).unwrap();
    assert_eq!(listed["configured"], true);
    assert_eq!(names(&listed), ["pre-commit"]);
    assert!(text(&listed, "directory").contains(&shown), "{listed}");
    assert!(text(&listed["hooks"][0], "path").ends_with("/.githooks/pre-commit"));
    let out = registry
        .run("activity.list", &mut ctx, &json!({ "questions": true }))
        .unwrap();
    let entries = out["entries"].as_array().unwrap();
    assert_eq!(entries.len(), 2, "{out}");
    for entry in entries {
        assert!(text(entry, "directory").ends_with(&shown), "{entry}");
        assert_eq!(entry["origin"], "hook.list");
    }
    let found = registry
        .run("activity.list", &mut ctx, &json!({ "text": "HOOKS" }))
        .unwrap();
    assert!(found["entries"].as_array().unwrap().is_empty(), "{found}");
    let found = registry.run(
        "activity.list",
        &mut ctx,
        &json!({ "text": "HOOKS", "questions": true }),
    );
    assert_eq!(found.unwrap()["matched"], 2);
    let cleared = registry.run("activity.clear", &mut ctx, &json!({}));
    assert_eq!(cleared.unwrap()["cleared"], 2);

    drop(ctx);
    std::fs::remove_dir_all(&odd).unwrap();
}

#[test]
fn hostile_parameters_and_a_broken_repository_are_survived() {
    let mut h = Harness::new();
    let git = h.git();
    h.run("tag.create", json!({ "name": "kept" }));
    let refs = git.lines(["for-each-ref"]).unwrap();
    let long = "x".repeat(5000);

    // Parameters of the wrong shape are refused; the rest is taken as it
    // is, however large.
    for params in [
        json!({ "limit": -1 }),
        json!({ "limit": "many" }),
        json!({ "limit": 1.5 }),
        json!({ "limit": 1e300 }),
        json!({ "limit": [1] }),
        json!({ "questions": "yes" }),
        json!({ "questions": 1 }),
        json!({ "text": 5 }),
        json!({ "text": ["a"] }),
        json!({ "text": long.clone() }),
    ] {
        assert!(
            h.try_run("activity.list", params.clone()).is_err(),
            "activity.list accepted {params}"
        );
    }
    let everything = h.all().len();
    for params in [
        json!(null),
        json!([]),
        json!("junk"),
        json!({ "limit": u64::MAX }),
        json!({ "limit": 0 }),
        json!({ "limit": null, "questions": null, "text": null }),
        json!({ "text": "" }),
        json!({ "text": "\u{0}\n\t" }),
        json!({ "text": "İ\u{130}ß.*[(" }),
        json!({ "text": "x".repeat(1000) }),
        json!({ "unknown": true }),
    ] {
        let out = h
            .try_run("activity.list", params.clone())
            .unwrap_or_else(|err| panic!("activity.list refused {params}: {err}"));
        assert!(out["entries"].as_array().unwrap().len() <= everything);
    }

    // A hooks directory that is not what it should be.
    let outside = h.scratch.path().join(".git").join("elsewhere");
    std::fs::write(&outside, "a file, not a directory").unwrap();
    for path in [
        "",
        "no/such/directory",
        "/dev/null",
        "/",
        ".",
        "..",
        "-x",
        "--help",
        "a\nb",
        outside.to_str().unwrap(),
        long.as_str(),
    ] {
        git.run(["config", "core.hooksPath", path]).unwrap();
        let listed = h.run("hook.list", json!({ "junk": [1, 2] }));
        assert_eq!(listed["configured"], true, "{path:?}");
        assert!(names(&listed).is_empty(), "{path:?}");
    }
    // A home directory that cannot be found: git refuses to do anything
    // at all in the repository, and that is an error here, not a crash.
    let config = h.scratch.path().join(".git").join("config");
    let sound = std::fs::read_to_string(&config).unwrap();
    git.run(["config", "core.hooksPath", "~no-such-user-here/hooks"])
        .unwrap();
    let err = h.try_run("hook.list", json!({})).unwrap_err();
    assert!(err.contains("failed to expand user dir"), "{err}");
    let entry = h.entry("git rev-parse --git-path hooks");
    assert_eq!(entry["code"], 128);
    assert_eq!(entry["origin"], "hook.list");
    std::fs::write(&config, sound).unwrap();

    // Hooks that are not programs: a directory, a link to nothing, a link
    // to itself, a file nobody may run. Where files carry no executable
    // mark (Windows), git runs that file, and it is listed.
    let hooks = h.hooks_dir();
    std::fs::create_dir_all(hooks.join("pre-commit")).unwrap();
    std::fs::write(hooks.join("pre-push"), "#!/bin/sh\n").unwrap();
    bitbraid_testkit::symlink_file(hooks.join("nowhere"), hooks.join("commit-msg")).unwrap();
    bitbraid_testkit::symlink_file(hooks.join("post-commit"), hooks.join("post-commit")).unwrap();
    let listed = h.run("hook.list", json!({}));
    if cfg!(unix) {
        assert!(names(&listed).is_empty(), "{listed}");
    } else {
        assert_eq!(names(&listed), ["pre-push"], "{listed}");
    }
    // The directory itself gone, or a file.
    std::fs::remove_dir_all(&hooks).unwrap();
    assert!(names(&h.run("hook.list", json!({}))).is_empty());
    std::fs::write(&hooks, "").unwrap();
    assert!(names(&h.run("hook.list", json!({}))).is_empty());

    // Nothing of this touched the repository.
    assert_eq!(git.lines(["for-each-ref"]).unwrap(), refs);

    // No repository open.
    let mut nothing = CommandContext::default();
    for id in ["activity.list", "activity.clear", "hook.list"] {
        assert!(
            h.registry.run(id, &mut nothing, &json!({})).is_err(),
            "{id}"
        );
    }

    // A repository git no longer takes for one. What git answers then
    // depends on what lies around the directory, so only this much is
    // asked for: it is survived, and it is in the log.
    let head = h.scratch.path().join(".git").join("HEAD");
    for garbage in ["garbage\n", "", "ref: \0\n"] {
        std::fs::write(&head, garbage).unwrap();
        let _ = h.try_run("hook.list", json!({}));
        let entry = h.entry("git rev-parse --git-path hooks");
        assert_eq!(entry["origin"], "hook.list");
    }
    std::fs::remove_file(&head).unwrap();
    let _ = h.try_run("hook.list", json!({}));

    // The directory gone altogether: git cannot even be started there,
    // and the entry says so.
    let before = h.all().len();
    std::fs::remove_dir_all(h.scratch.path()).unwrap();
    assert!(h.try_run("hook.list", json!({})).is_err());
    let all = h.all();
    assert_eq!(all.len(), before + 1);
    assert_eq!(all[0]["end"], "not_started");
    assert_eq!(all[0]["code"], Value::Null);
    assert_eq!(all[0]["command"], "git rev-parse --git-path hooks");
    assert!(!text(&all[0], "stderr").is_empty());
    assert_eq!(h.run("activity.clear", json!([1, 2]))["cleared"], all.len());
    assert!(h.all().is_empty());
}
