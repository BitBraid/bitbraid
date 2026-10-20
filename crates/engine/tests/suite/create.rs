//! Init and clone against `git init` and `git clone`.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::Ordering},
};

use bitbraid_engine::{CommandContext, Registry, builtin_registry};
use bitbraid_testkit::{Git, Scratch, fixtures_out_dir, scratch_root};
use serde_json::{Value, json};

/// A directory of this test's own, empty.
fn playground(name: &str) -> PathBuf {
    let dir = scratch_root().join(format!("create-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    bitbraid_core::paths::canonical(dir).unwrap()
}

fn git_at(dir: &Path) -> Git {
    Git::new(dir, fixtures_out_dir().join(".home"))
}

struct Engine {
    registry: Registry,
    ctx: CommandContext,
}

impl Engine {
    fn new() -> Self {
        Self {
            registry: builtin_registry(),
            ctx: CommandContext::default(),
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
}

/// Everything under `dir`, as relative paths, sorted.
fn listing(dir: &Path) -> Vec<String> {
    fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
        for entry in std::fs::read_dir(dir).unwrap().flatten() {
            let path = entry.path();
            // With `/` between folders, as git writes paths.
            let relative = path.strip_prefix(root).unwrap().to_string_lossy();
            out.push(relative.replace(std::path::MAIN_SEPARATOR, "/"));
            if path.is_dir() && !path.is_symlink() {
                walk(root, &path, out);
            }
        }
    }
    let mut out = Vec::new();
    walk(dir, dir, &mut out);
    out.sort();
    out
}

#[test]
fn init_matches_git_init() {
    let dir = playground("init");
    let mut engine = Engine::new();

    // A new directory several levels down, with a branch name.
    let ours = dir.join("a/b/ours");
    let out = engine.run("repo.init", json!({ "path": ours, "branch": "trunk" }));
    assert_eq!(out["path"], ours.to_str().unwrap());
    let twin = dir.join("twin");
    std::fs::create_dir_all(&twin).unwrap();
    git_at(&twin).run(["init", "-q", "-b", "trunk"]).unwrap();
    for (what, args) in [
        ("HEAD", vec!["symbolic-ref", "HEAD"]),
        ("bare", vec!["rev-parse", "--is-bare-repository"]),
        ("status", vec!["status", "--porcelain=v2", "--branch"]),
    ] {
        assert_eq!(
            git_at(&ours).run(args.clone()).unwrap(),
            git_at(&twin).run(args).unwrap(),
            "{what}"
        );
    }
    // It opens like any repository.
    let summary = engine.run("repo.open", json!({ "path": ours }));
    assert_eq!(summary["head_branch"], "trunk");

    // An existing empty directory, with a first commit.
    let second = dir.join("second");
    std::fs::create_dir(&second).unwrap();
    let git = git_at(&second);
    // The first commit needs to know who makes it; a fresh repository has
    // only the user's own configuration, which a test must not rely on.
    let before_commit =
        engine.try_run("repo.init", json!({ "path": second, "first_commit": true }));
    if before_commit.is_err() {
        // No identity configured here: the repository exists all the same.
        assert!(second.join(".git").is_dir());
        git.run(["config", "user.name", "Ada Fixture"]).unwrap();
        git.run(["config", "user.email", "ada@fixture.invalid"])
            .unwrap();
        git.run(["commit", "-q", "--allow-empty", "-m", "Initial commit"])
            .unwrap();
    }
    assert_eq!(git.rev_list_count("HEAD").unwrap(), 1);
    assert_eq!(
        git.run(["log", "-1", "--format=%s"]).unwrap(),
        "Initial commit\n"
    );
    assert_eq!(
        git.run(["rev-parse", "HEAD^{tree}"]).unwrap().trim(),
        "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
        "the empty tree"
    );
}

#[test]
fn init_refuses_what_it_should_not_touch() {
    let dir = playground("init-refuse");
    let mut engine = Engine::new();

    // A repository already: left exactly as it is.
    let scratch = Scratch::of("linear").unwrap();
    let head = scratch.git().rev_parse("HEAD").unwrap();
    let err = engine
        .try_run(
            "repo.init",
            json!({ "path": scratch.path(), "branch": "other" }),
        )
        .unwrap_err();
    assert!(err.contains("a repository already"), "{err}");
    assert_eq!(scratch.git().rev_parse("HEAD").unwrap(), head);
    assert_eq!(
        scratch
            .git()
            .run(["symbolic-ref", "--short", "HEAD"])
            .unwrap(),
        "main\n"
    );

    // A file in the way, a relative path, and names that are not names.
    std::fs::write(dir.join("file"), "x").unwrap();
    let long = "n".repeat(5000);
    for params in [
        json!({ "path": dir.join("file") }),
        json!({ "path": dir.join("file/below") }),
        json!({ "path": "relative/path" }),
        json!({ "path": "" }),
        json!({}),
        json!({ "path": 7 }),
        json!({ "path": dir.join("x"), "branch": "" }),
        json!({ "path": dir.join("x"), "branch": "-b" }),
        json!({ "path": dir.join("x"), "branch": "a..b" }),
        json!({ "path": dir.join("x"), "branch": "HEAD" }),
        json!({ "path": dir.join("x"), "branch": "a b" }),
        json!({ "path": dir.join("x"), "branch": long }),
        json!({ "path": dir.join("x"), "first_commit": "yes" }),
    ] {
        assert!(
            engine.try_run("repo.init", params.clone()).is_err(),
            "{params}"
        );
    }
    // A refused name creates nothing.
    assert_eq!(listing(&dir), ["file"]);
}

/// `git init` in a bare repository's directory sets that repository up
/// again, and a clone below a `.git` puts a repository among git's own
/// files. Neither is taken, and nothing is written.
#[test]
fn nothing_is_made_inside_a_repositorys_own_directory() {
    let mut engine = Engine::new();
    let bare = Scratch::of("bare").unwrap();
    let plain = Scratch::of("linear").unwrap();
    let source = Scratch::of("linear").unwrap();
    let before = (listing(bare.path()), listing(plain.path()));
    let config = std::fs::read(bare.path().join("config")).unwrap();
    for place in [
        bare.path().to_path_buf(),
        bare.path().join("made"),
        bare.path().join("refs/heads/made"),
        // An empty directory of a bare repository, which a clone would take.
        bare.path().join("branches"),
        plain.path().join(".git"),
        plain.path().join(".git/made"),
        plain.path().join(".git/objects/made/deeper"),
    ] {
        for params in [
            json!({ "path": place }),
            json!({ "path": place, "branch": "other", "first_commit": true }),
        ] {
            let err = engine.try_run("repo.init", params.clone()).unwrap_err();
            assert!(
                err.contains("repository's own directory") || err.contains("a repository already"),
                "{params}: {err}"
            );
        }
        let params = json!({ "url": source.path(), "path": place });
        let err = engine.try_run("repo.clone", params.clone()).unwrap_err();
        assert!(
            err.contains("repository's own directory") || err.contains("not an empty directory"),
            "{params}: {err}"
        );
    }
    assert_eq!((listing(bare.path()), listing(plain.path())), before);
    assert_eq!(std::fs::read(bare.path().join("config")).unwrap(), config);
    // Beside it, and in a directory of a worktree, one is made.
    let beside = bare.path().with_extension("beside");
    let _ = std::fs::remove_dir_all(&beside);
    engine.run("repo.init", json!({ "path": beside }));
    engine.run("repo.init", json!({ "path": plain.path().join("nested") }));
    assert!(plain.path().join("nested/.git").is_dir());
    let _ = std::fs::remove_dir_all(&beside);
}

#[test]
fn clone_matches_git_clone() {
    let dir = playground("clone");
    let source = Scratch::of("branches-merges").unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let mut engine = Engine::new();
    let refs = |path: &Path| {
        git_at(path)
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap()
    };
    let head = |path: &Path| git_at(path).run(["symbolic-ref", "HEAD"]).unwrap();

    // To a path that does not exist, reporting on the way.
    let stages = Arc::new(Mutex::new(Vec::new()));
    let sink = stages.clone();
    engine.ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
        sink.lock().unwrap().push(progress.stage);
    }));
    let ours = dir.join("ours");
    let out = engine.run("repo.clone", json!({ "url": url, "path": ours }));
    assert_eq!(out["path"], ours.to_str().unwrap());
    let twin = dir.join("twin");
    git_at(&dir)
        .run(["clone", "-q", &url, twin.to_str().unwrap()])
        .unwrap();
    assert_eq!(refs(&ours), refs(&twin));
    assert_eq!(head(&ours), head(&twin));
    assert_eq!(git_at(&ours).run(["status", "--porcelain"]).unwrap(), "");
    assert_eq!(
        git_at(&ours)
            .run(["remote", "get-url", "origin"])
            .unwrap()
            .trim(),
        url
    );
    engine.ctx.progress = None;

    // Into an empty directory that is there already, on another branch.
    let branch = git_at(source.path())
        .lines(["for-each-ref", "--format=%(refname:short)", "refs/heads"])
        .unwrap()
        .into_iter()
        .find(|name| name != "main")
        .expect("the fixture has a second branch");
    let second = dir.join("second");
    std::fs::create_dir(&second).unwrap();
    engine.run(
        "repo.clone",
        json!({ "url": url, "path": second, "branch": branch }),
    );
    let second_twin = dir.join("second-twin");
    git_at(&dir)
        .run([
            "clone",
            "-q",
            "--branch",
            &branch,
            &url,
            second_twin.to_str().unwrap(),
        ])
        .unwrap();
    assert_eq!(refs(&second), refs(&second_twin));
    assert_eq!(head(&second), format!("refs/heads/{branch}\n"));

    // Cut short: one commit of history, over a transport that can do that.
    let shallow = dir.join("shallow");
    let file_url = format!("file://{url}");
    engine.run(
        "repo.clone",
        json!({ "url": file_url, "path": shallow, "depth": 1 }),
    );
    assert_eq!(git_at(&shallow).rev_list_count("HEAD").unwrap(), 1);
    assert!(shallow.join(".git/shallow").exists());
}

/// A clone gets the commit-graph file git writes for the same clone with
/// `git commit-graph write --reachable`, byte for byte, and says so on
/// the way; a shallow one gets none, as git writes none for it.
#[test]
fn a_clone_writes_the_commit_graph_git_writes() {
    let dir = playground("clone-graph");
    let source = Scratch::of("branches-merges").unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let mut engine = Engine::new();
    let graph = |path: &Path| std::fs::read(path.join(".git/objects/info/commit-graph"));

    let stages = Arc::new(Mutex::new(Vec::new()));
    let sink = stages.clone();
    engine.ctx.progress = Some(Arc::new(move |progress: bitbraid_git::Progress| {
        sink.lock().unwrap().push(progress.stage);
    }));
    let ours = dir.join("ours");
    let out = engine.run("repo.clone", json!({ "url": url, "path": ours }));
    engine.ctx.progress = None;
    assert_eq!(out["commit_graph"], true, "{out}");
    let stages = stages.lock().unwrap().clone();
    assert!(
        stages
            .iter()
            .any(|stage| stage == bitbraid_git::create::commit_graph_stage()),
        "{stages:?}"
    );

    let twin = dir.join("twin");
    git_at(&dir)
        .run(["clone", "-q", &url, twin.to_str().unwrap()])
        .unwrap();
    assert!(
        graph(&twin).is_err(),
        "git clone wrote a commit-graph itself"
    );
    git_at(&twin)
        .run(["commit-graph", "write", "--reachable"])
        .unwrap();
    let (ours_graph, twin_graph) = (graph(&ours).unwrap(), graph(&twin).unwrap());
    assert!(!ours_graph.is_empty());
    assert_eq!(ours_graph, twin_graph, "the commit-graph files differ");
    git_at(&ours).run(["commit-graph", "verify"]).unwrap();

    // Cut short: git writes none, and none is said to be there.
    let shallow = dir.join("shallow");
    let out = engine.run(
        "repo.clone",
        json!({ "url": format!("file://{url}"), "path": shallow, "depth": 1 }),
    );
    assert_eq!(out["commit_graph"], false, "{out}");
    assert!(graph(&shallow).is_err());
    let shallow_twin = dir.join("shallow-twin");
    git_at(&dir)
        .run([
            "clone",
            "-q",
            "--depth",
            "1",
            &format!("file://{url}"),
            shallow_twin.to_str().unwrap(),
        ])
        .unwrap();
    git_at(&shallow_twin)
        .run(["commit-graph", "write", "--reachable"])
        .unwrap();
    assert!(
        graph(&shallow_twin).is_err(),
        "git wrote one for a shallow clone"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_clone_that_fails_or_is_stopped_leaves_nothing() {
    let dir = playground("clone-fail");
    let source = Scratch::of("linear").unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let mut engine = Engine::new();

    // Nothing to clone from: the destination is not left behind.
    let gone = dir.join("gone");
    let err = engine
        .try_run(
            "repo.clone",
            json!({ "url": dir.join("no-such-repository"), "path": gone }),
        )
        .unwrap_err();
    assert!(err.contains("clone"), "{err}");
    assert!(!gone.exists());

    // The same into an empty directory of the user's: it stays, empty.
    let kept = dir.join("kept");
    std::fs::create_dir(&kept).unwrap();
    assert!(
        engine
            .try_run(
                "repo.clone",
                json!({ "url": dir.join("no-such-repository"), "path": kept }),
            )
            .is_err()
    );
    assert!(kept.is_dir());
    assert!(listing(&kept).is_empty());

    // A branch that is not there: git starts, creates the directory, fails.
    let wrong = dir.join("wrong-branch");
    assert!(
        engine
            .try_run(
                "repo.clone",
                json!({ "url": url, "path": wrong, "branch": "no-such-branch" }),
            )
            .is_err()
    );
    assert!(!wrong.exists());

    // Stopped before it starts.
    engine.ctx.cancel.store(true, Ordering::SeqCst);
    let stopped = dir.join("stopped");
    let err = engine
        .try_run("repo.clone", json!({ "url": url, "path": stopped }))
        .unwrap_err();
    assert!(err.contains("cancelled"), "{err}");
    assert!(!stopped.exists());
    engine.ctx.cancel.store(false, Ordering::SeqCst);

    // Something in the way: refused, and what is there is not touched.
    let busy = dir.join("busy");
    std::fs::create_dir_all(busy.join("sub")).unwrap();
    std::fs::write(busy.join("sub/mine.txt"), "mine").unwrap();
    let err = engine
        .try_run("repo.clone", json!({ "url": url, "path": busy }))
        .unwrap_err();
    assert!(err.contains("not an empty directory"), "{err}");
    assert_eq!(listing(&busy), ["sub", "sub/mine.txt"]);
    std::fs::write(dir.join("a-file"), "x").unwrap();
    assert!(
        engine
            .try_run(
                "repo.clone",
                json!({ "url": url, "path": dir.join("a-file") })
            )
            .is_err()
    );
    assert_eq!(std::fs::read_to_string(dir.join("a-file")).unwrap(), "x");
    // A link to somewhere else is something in the way too, and what it
    // points at is left alone.
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&busy, dir.join("link")).unwrap();
        assert!(
            engine
                .try_run(
                    "repo.clone",
                    json!({ "url": url, "path": dir.join("link") })
                )
                .is_err()
        );
        assert_eq!(listing(&busy), ["sub", "sub/mine.txt"]);
    }
}

#[test]
fn hostile_clone_parameters_are_refused_and_create_nothing() {
    let dir = playground("clone-hostile");
    let source = Scratch::of("linear").unwrap();
    let url = source.path().to_str().unwrap().to_owned();
    let mut engine = Engine::new();
    let long = "u".repeat(5000);
    let target = dir.join("target");

    for bad in [
        "",
        "   ",
        "-u",
        "--upload-pack=touch pwned",
        "--config=core.sshCommand=touch pwned",
        "a\nb",
        // A transport that runs a command: git refuses it by itself.
        "ext::sh -c touch% pwned",
        long.as_str(),
    ] {
        assert!(
            engine
                .try_run("repo.clone", json!({ "url": bad, "path": target }))
                .is_err(),
            "accepted the URL {bad:?}"
        );
        assert!(!target.exists(), "{bad:?} left the destination behind");
    }
    for params in [
        json!({ "url": url }),
        json!({ "path": target }),
        json!({ "url": url, "path": "relative" }),
        json!({ "url": url, "path": "" }),
        json!({ "url": url, "path": target, "branch": "-b" }),
        json!({ "url": url, "path": target, "branch": "a..b" }),
        json!({ "url": url, "path": target, "branch": "" }),
        json!({ "url": url, "path": target, "depth": -1 }),
        json!({ "url": url, "path": target, "depth": "all" }),
        json!({ "url": url, "path": target, "recurse_submodules": 1 }),
        json!({ "url": 5, "path": target }),
    ] {
        assert!(
            engine.try_run("repo.clone", params.clone()).is_err(),
            "{params}"
        );
        assert!(!target.exists(), "{params} left the destination behind");
    }
    assert!(listing(&dir).is_empty(), "{:?}", listing(&dir));
    assert!(!source.path().join("pwned").exists());
    assert!(!Path::new("pwned").exists());
}

#[test]
fn init_with_gitignore_templates_matches_what_git_makes_of_the_same_file() {
    let dir = playground("init-ignore");
    let mut engine = Engine::new();

    // The templates there are, and the file two of them make.
    let listed = engine.run(
        "repo.ignore_templates",
        json!({ "names": ["rust", "python"] }),
    );
    let names: Vec<&str> = listed["templates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|template| template["name"].as_str().unwrap())
        .collect();
    assert!(names.len() >= 8, "{names:?}");
    assert!(names.contains(&"rust") && names.contains(&"node"));
    assert!(
        listed["templates"][0]["title"].is_string() && listed["templates"][0].get("text").is_none()
    );
    let text = listed["text"].as_str().unwrap().to_owned();
    assert!(text.contains("/target/") && text.contains("__pycache__/"));
    assert!(engine.run("repo.ignore_templates", json!({}))["text"].is_null());

    // A repository that starts from them, against git given the same file.
    let ours = dir.join("ours");
    let made = engine.try_run(
        "repo.init",
        json!({ "path": ours, "branch": "trunk", "first_commit": true, "gitignore": ["rust", "python"] }),
    );
    assert_eq!(
        std::fs::read_to_string(ours.join(".gitignore")).unwrap(),
        text
    );
    let twin = dir.join("twin");
    std::fs::create_dir_all(&twin).unwrap();
    let git = git_at(&twin);
    git.run(["init", "-q", "-b", "trunk"]).unwrap();
    std::fs::write(twin.join(".gitignore"), &text).unwrap();
    git.run(["add", "--", ".gitignore"]).unwrap();
    git.run(["commit", "-q", "-m", "Initial commit"]).unwrap();
    match made {
        Ok(_) => {
            // The first commit is the file and nothing else.
            for args in [
                vec!["rev-parse", "HEAD^{tree}"],
                vec!["ls-files", "--stage"],
                vec!["status", "--porcelain=v2"],
                vec!["log", "--format=%s"],
                vec!["symbolic-ref", "HEAD"],
            ] {
                assert_eq!(
                    git_at(&ours).run(args.clone()).unwrap(),
                    git.run(args.clone()).unwrap(),
                    "{args:?}"
                );
            }
        }
        // Nobody to commit as: the repository and its file are there all
        // the same, the file staged for the commit.
        Err(err) => {
            assert!(err.contains("the repository was created"), "{err}");
            assert_eq!(
                git_at(&ours).run(["status", "--porcelain"]).unwrap(),
                "A  .gitignore\n"
            );
        }
    }
    // Git ignores in ours what it ignores in the twin, and what the
    // templates are for.
    for (path, ignored) in [
        ("target/debug/app", true),
        ("src/main.rs", false),
        ("pkg/__pycache__/mod.cpython-312.pyc", true),
        ("pkg/mod.py", false),
        (".venv/bin/python", true),
        ("node_modules/x/index.js", false),
    ] {
        let asked = |dir: &Path| git_at(dir).run(["check-ignore", "-q", path]).is_ok();
        assert_eq!(asked(&ours), ignored, "{path} in ours");
        assert_eq!(asked(&twin), ignored, "{path} in the twin");
    }

    // Without a first commit the file is there to be committed.
    let plain = dir.join("plain");
    engine.run("repo.init", json!({ "path": plain, "gitignore": ["node"] }));
    assert_eq!(
        git_at(&plain).run(["status", "--porcelain"]).unwrap(),
        "?? .gitignore\n"
    );

    // Every template does what it says, by git's own reading of it.
    let all = dir.join("all");
    engine.run("repo.init", json!({ "path": all, "gitignore": names }));
    for path in [
        "target/x",
        "node_modules/a/b.js",
        "a/__pycache__/x.pyc",
        "bin/tool",
        "out/Main.class",
        "build/main.o",
        "obj/project.assets.json",
        ".idea/workspace.xml",
        ".DS_Store",
    ] {
        assert!(
            git_at(&all).run(["check-ignore", "-q", path]).is_ok(),
            "{path} should be ignored"
        );
    }
    for path in [
        "src/lib.rs",
        "README.md",
        "gradle/wrapper/gradle-wrapper.jar",
        "main.c",
    ] {
        assert!(
            git_at(&all).run(["check-ignore", "-q", path]).is_err(),
            "{path} should not be ignored"
        );
    }
}

#[test]
fn hostile_templates_make_nothing_and_overwrite_nothing() {
    let dir = playground("init-ignore-hostile");
    let mut engine = Engine::new();
    let long = "x".repeat(10_000);
    for hostile in [
        json!(["cobol"]),
        json!(["rust", "../rust"]),
        json!(["rust", ""]),
        json!(["RUST"]),
        json!([long]),
        json!(["ru\nst"]),
        json!("rust"),
        json!([7]),
        json!({ "rust": true }),
    ] {
        let target = dir.join("target-of-nothing");
        assert!(
            engine
                .try_run("repo.init", json!({ "path": target, "gitignore": hostile }))
                .is_err(),
            "{hostile} should be refused"
        );
        assert!(!target.exists(), "{hostile} left a directory");
        if !hostile.is_string() && !hostile.is_object() {
            assert!(
                engine
                    .try_run("repo.ignore_templates", json!({ "names": hostile }))
                    .is_err(),
                "{hostile}"
            );
        }
    }
    // A .gitignore that is there is not written over.
    let kept = dir.join("kept");
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join(".gitignore"), "mine\n").unwrap();
    let err = engine
        .try_run("repo.init", json!({ "path": kept, "gitignore": ["rust"] }))
        .unwrap_err();
    assert!(err.contains(".gitignore"), "{err}");
    assert_eq!(
        std::fs::read_to_string(kept.join(".gitignore")).unwrap(),
        "mine\n"
    );
    assert!(
        !kept.join(".git").exists(),
        "refused before anything was made"
    );
    // Without templates the same directory becomes a repository, file kept.
    engine.run("repo.init", json!({ "path": kept }));
    assert_eq!(
        std::fs::read_to_string(kept.join(".gitignore")).unwrap(),
        "mine\n"
    );
}

#[test]
fn a_clone_with_submodules_matches_git_clone_recurse_submodules() {
    let dir = playground("clone-submodules");
    let mut engine = Engine::new();
    // A superproject with a submodule that is a local folder.
    let library = Scratch::of("linear").unwrap();
    let parent = Scratch::of("branches-merges").unwrap();
    let git = parent.git();
    git.run([
        "-c",
        "protocol.file.allow=always",
        "submodule",
        "--quiet",
        "add",
        "--",
        library.path().to_str().unwrap(),
        "deps/library",
    ])
    .unwrap();
    git.run(["commit", "-q", "-m", "add a submodule"]).unwrap();
    let url = parent.path().to_str().unwrap().to_owned();
    let refs = |path: &Path| {
        git_at(path)
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap()
    };
    let head = |path: &Path| git_at(path).run(["symbolic-ref", "HEAD"]).unwrap();

    let ours = dir.join("ours");
    engine.run(
        "repo.clone",
        json!({ "url": url, "path": ours, "recurse_submodules": true, "allow_local": true }),
    );
    let twin = dir.join("twin");
    git_at(&dir)
        .run([
            "-c",
            "protocol.file.allow=always",
            "clone",
            "-q",
            "--recurse-submodules",
            &url,
            twin.to_str().unwrap(),
        ])
        .unwrap();
    assert_eq!(refs(&ours), refs(&twin));
    assert_eq!(head(&ours), head(&twin));
    for args in [vec!["submodule", "status"], vec!["status", "--porcelain"]] {
        assert_eq!(
            git_at(&ours).run(args.clone()).unwrap(),
            git_at(&twin).run(args.clone()).unwrap(),
            "{args:?}"
        );
    }
    // The submodule is checked out, with the library's own files in it.
    let checked_out = |root: &Path| {
        git_at(&root.join("deps/library"))
            .run(["rev-parse", "HEAD"])
            .unwrap()
    };
    assert_eq!(checked_out(&ours), checked_out(&twin));
    assert_eq!(
        checked_out(&ours).trim(),
        git_at(library.path()).rev_parse("HEAD").unwrap()
    );

    // Without submodules the folder is there and empty.
    let bare_of_them = dir.join("without");
    engine.run("repo.clone", json!({ "url": url, "path": bare_of_them }));
    assert!(
        std::fs::read_dir(bare_of_them.join("deps/library"))
            .unwrap()
            .next()
            .is_none()
    );

    // Asked for, but not allowed from a folder: git refuses unless the
    // user's own configuration allows it, and then nothing is left behind.
    let refused = dir.join("refused");
    if engine
        .try_run(
            "repo.clone",
            json!({ "url": url, "path": refused, "recurse_submodules": true }),
        )
        .is_err()
    {
        assert!(!refused.exists());
    }
    assert!(
        engine
            .try_run(
                "repo.clone",
                json!({ "url": url, "path": dir.join("x"), "allow_local": "yes" })
            )
            .is_err()
    );
}
