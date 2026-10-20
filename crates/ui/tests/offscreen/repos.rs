//! The repositories the window is opened on: fixtures from
//! `fixtures/*.toml`, built by the testkit and copied to the scratch
//! directory of this run, and what a view needs beyond them (local
//! changes, an image, a submodule, a merge that stopped), made with git
//! under the fixtures' clock. All of it is at fixed paths, since paths
//! are on show.

use std::{
    fs,
    path::{Path, PathBuf},
};

use bitbraid_testkit::{Git, copy_fixture, fixtures_out_dir, oracle::clock_at};

/// The directory the repositories of a test are in. Emptied by
/// [`fresh`], which every test starts with.
pub fn work() -> PathBuf {
    crate::hermetic::inside()
        .expect("the tests run in scratch surroundings")
        .join("work")
}

/// Empty the directory of repositories: a test starts from nothing.
pub fn fresh() {
    let work = work();
    if work.exists() {
        fs::remove_dir_all(&work).unwrap();
    }
    fs::create_dir_all(&work).unwrap();
}

fn git_in(path: &Path) -> Git {
    let git = Git::new(path, fixtures_out_dir().join(".home"));
    git.ensure_home().unwrap();
    git
}

/// Run git at hour `tick` of the fixtures' clock.
fn at(git: &Git, tick: u64, args: &[&str]) -> String {
    let date = clock_at(tick);
    git.run_with(
        args,
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    )
    .unwrap()
}

fn write(repo: &Path, rel: &str, content: impl AsRef<[u8]>) {
    let path = repo.join(rel);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, content).unwrap();
}

/// A copy of the fixture `name` under the directory of repositories.
pub fn fixture(name: &str, folder: &str) -> PathBuf {
    let path = work().join(folder);
    copy_fixture(name, &path).unwrap();
    path
}

/// The repository most views are shown on: the `search` fixture (sixteen
/// commits, merges, six branches, two tags, three authors, a rename).
pub fn project() -> PathBuf {
    fixture("search", "project")
}

/// A small picture: a frame around a field with a square in it.
fn picture(field: [u8; 3], square: [u8; 3], size: u32) -> Vec<u8> {
    let mut image = image::RgbImage::from_pixel(size, size, image::Rgb(field));
    for (x, y, pixel) in image.enumerate_pixels_mut() {
        let edge = x < 4 || y < 4 || x >= size - 4 || y >= size - 4;
        let inside = (size / 4..size * 3 / 4).contains(&x) && (size / 4..size * 3 / 4).contains(&y);
        if edge {
            *pixel = image::Rgb([40, 44, 52]);
        } else if inside {
            *pixel = image::Rgb(square);
        }
    }
    let mut bytes = Vec::new();
    image
        .write_to(
            &mut std::io::Cursor::new(&mut bytes),
            image::ImageFormat::Png,
        )
        .unwrap();
    bytes
}

const RENDER_FIRST: &str = "\
//! Turns a parsed line into text.

pub struct Renderer {
    width: usize,
    indent: usize,
}

impl Renderer {
    pub fn new(width: usize) -> Self {
        Self { width, indent: 0 }
    }

    pub fn line(&self, words: &[&str]) -> String {
        let mut out = String::new();
        for word in words {
            if out.len() + word.len() > self.width {
                break;
            }
            out.push_str(word);
            out.push(' ');
        }
        out
    }

    pub fn indent(&mut self, by: usize) {
        self.indent += by;
    }
}
";

/// The project with more to look at in a file: a source file with three
/// commits to its name, a picture and a submodule, all committed.
pub fn project_with_files() -> PathBuf {
    let repo = project();
    add_files(&repo);
    repo
}

fn add_files(repo: &Path) {
    let repo = repo.to_path_buf();
    let git = git_in(&repo);

    write(&repo, "src/render.rs", RENDER_FIRST);
    git.run(["add", "-A"]).unwrap();
    at(&git, 30, &["commit", "-q", "-m", "render: lines of words"]);

    let second = RENDER_FIRST.replace(
        "            out.push(' ');\n",
        "            out.push(' ');\n            // one space between words\n",
    );
    write(&repo, "src/render.rs", &second);
    write(
        &repo,
        "assets/logo.png",
        picture([222, 226, 232], [60, 120, 200], 96),
    );
    git.run(["add", "-A"]).unwrap();
    at(
        &git,
        31,
        &[
            "commit",
            "-q",
            "--author=Bob Fixture <bob@fixture.invalid>",
            "-m",
            "render: say why the space; add the logo",
        ],
    );

    // A library of its own, taken in as a submodule.
    let library = work().join("library");
    fs::create_dir_all(&library).unwrap();
    let lib = git_in(&library);
    lib.run(["init", "-q", "-b", "main"]).unwrap();
    write(&library, "lib.txt", "one\n");
    lib.run(["add", "-A"]).unwrap();
    at(&lib, 20, &["commit", "-q", "-m", "library: one"]);
    at(
        &git,
        32,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "--quiet",
            "add",
            library.to_str().unwrap(),
            "vendor/library",
        ],
    );
    at(&git, 32, &["commit", "-q", "-m", "take the library in"]);

    let third = second.replace(
        "        Self { width, indent: 0 }",
        "        Self {\n            width: width.max(20),\n            indent: 0,\n        }",
    );
    write(&repo, "src/render.rs", third);
    git.run(["add", "-A"]).unwrap();
    at(
        &git,
        33,
        &["commit", "-q", "-m", "render: never narrower than twenty"],
    );
}

/// The project with something in every section of the references panel:
/// a remote it is four commits ahead of, a second worktree, a stash, and
/// the submodule and tags it has anyway.
pub fn project_with_everything() -> PathBuf {
    let repo = project();
    let git = git_in(&repo);
    let origin = work().join("origin.git");
    git.run([
        "clone",
        "-q",
        "--bare",
        "--no-hardlinks",
        repo.to_str().unwrap(),
        origin.to_str().unwrap(),
    ])
    .unwrap();
    git.run(["remote", "add", "origin", origin.to_str().unwrap()])
        .unwrap();
    git.run(["-c", "protocol.file.allow=always", "fetch", "-q", "origin"])
        .unwrap();
    // Fetched at the fixtures' clock, so that what says when reads the
    // same on every run.
    fetched_at(&repo, crate::hermetic::NOW - 20 * 3600);
    git.run(["branch", "-q", "--set-upstream-to=origin/main", "main"])
        .unwrap();
    add_files(&repo);
    git.run([
        "worktree",
        "add",
        "-q",
        "-b",
        "hotfix",
        "../project-hotfix",
        "v0.2",
    ])
    .unwrap();
    write(&repo, "notes.txt", "to do: wrap at the width\n");
    git.run(["add", "notes.txt"]).unwrap();
    at(&git, 36, &["stash", "push", "-q", "-m", "notes for later"]);
    repo
}

/// The project with uncommitted work of every kind: a text file changed
/// in two places, one change staged, a file nobody tracks, a file gone, a
/// picture replaced and a submodule that moved on.
pub fn project_with_changes() -> PathBuf {
    let repo = project_with_files();
    let git = git_in(&repo);

    let text = fs::read_to_string(repo.join("src/render.rs")).unwrap();
    let text = text
        .replace("Turns a parsed line into text.", "Turns a parsed line into wrapped text.")
        .replace(
            "    pub fn indent(&mut self, by: usize) {\n        self.indent += by;\n    }\n",
            "    pub fn indent(&mut self, by: usize) {\n        self.indent = self.indent.saturating_add(by);\n    }\n\n    pub fn outdent(&mut self, by: usize) {\n        self.indent = self.indent.saturating_sub(by);\n    }\n",
        );
    write(&repo, "src/render.rs", text);
    write(
        &repo,
        "docs/guide.md",
        "Search for a needle.\nUse quotes for several words.\n",
    );
    git.run(["add", "docs/guide.md"]).unwrap();
    write(&repo, "notes.txt", "to do: wrap at the width\n");
    fs::remove_file(repo.join("grammar.rs")).unwrap();
    write(
        &repo,
        "assets/logo.png",
        picture([232, 226, 214], [200, 90, 60], 128),
    );

    let module = repo.join("vendor/library");
    let lib = git_in(&module);
    write(&module, "lib.txt", "one\ntwo\n");
    lib.run(["add", "-A"]).unwrap();
    at(&lib, 40, &["commit", "-q", "-m", "library: two"]);
    repo
}

/// The project with files whose line endings are what is to be looked
/// at: notes stored with LF and now with CRLF, nothing else
/// changed; a script stored with CRLF with a line changed and one added
/// that ends in LF alone.
pub fn project_with_line_endings() -> PathBuf {
    let repo = project();
    let git = git_in(&repo);
    write(
        &repo,
        "scripts/build.cmd",
        "@echo off\r\nset TARGET=release\r\ncargo build --%TARGET%\r\n",
    );
    write(
        &repo,
        "docs/notes.txt",
        "Line one.\nLine two.\nLine three.\n",
    );
    git.run(["add", "-A"]).unwrap();
    at(&git, 30, &["commit", "-q", "-m", "a script and notes"]);
    write(
        &repo,
        "docs/notes.txt",
        "Line one.\r\nLine two.\r\nLine three.\r\n",
    );
    write(
        &repo,
        "scripts/build.cmd",
        "@echo off\r\nset TARGET=debug\r\ncargo build --%TARGET%\r\necho done\n",
    );
    repo
}

/// The project with a merge that stopped: one file in conflict.
pub fn project_in_conflict() -> PathBuf {
    let repo = project();
    let git = git_in(&repo);
    git.run(["checkout", "-q", "-b", "feature/guide"]).unwrap();
    write(
        &repo,
        "docs/guide.md",
        "Search for a needle.\nUse double quotes around several words.\nSee the README.\n",
    );
    git.run(["add", "-A"]).unwrap();
    at(&git, 30, &["commit", "-q", "-m", "guide: double quotes"]);
    git.run(["checkout", "-q", "main"]).unwrap();
    write(
        &repo,
        "docs/guide.md",
        "Search for a needle.\nUse single quotes around several words.\n",
    );
    git.run(["add", "-A"]).unwrap();
    at(&git, 31, &["commit", "-q", "-m", "guide: single quotes"]);
    // It stops, which git says by failing.
    let date = clock_at(32);
    let merged = git.run_with(
        ["merge", "--no-ff", "-m", "Merge the guide", "feature/guide"],
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    );
    assert!(merged.is_err(), "the merge was to stop on a conflict");
    repo
}

/// The project with a rebase of `main` onto `feature/guide` that stopped
/// on its first commit, in conflict in `docs/guide.md`: the form under the
/// changes has the buttons to go on, to give up and to skip the commit.
pub fn project_in_rebase_conflict() -> PathBuf {
    let repo = project_in_conflict();
    let git = git_in(&repo);
    git.run(["merge", "--abort"]).unwrap();
    let date = clock_at(33);
    let rebased = git.run_with(
        ["rebase", "feature/guide"],
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    );
    assert!(rebased.is_err(), "the rebase was to stop on a conflict");
    repo
}

/// The project with a merge that stopped because one side changed a file
/// (`docs/guide.md`) and the other deleted it.
pub fn project_with_a_deleted_conflict() -> PathBuf {
    let repo = project();
    let git = git_in(&repo);
    git.run(["checkout", "-q", "-b", "feature/no-guide"])
        .unwrap();
    git.run(["rm", "-q", "docs/guide.md"]).unwrap();
    at(&git, 30, &["commit", "-q", "-m", "guide: gone"]);
    git.run(["checkout", "-q", "main"]).unwrap();
    write(&repo, "docs/guide.md", "Search for a needle, once more.\n");
    git.run(["add", "-A"]).unwrap();
    at(&git, 31, &["commit", "-q", "-m", "guide: once more"]);
    let date = clock_at(32);
    let merged = git.run_with(
        [
            "merge",
            "--no-ff",
            "-m",
            "Merge without the guide",
            "feature/no-guide",
        ],
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    );
    assert!(merged.is_err(), "the merge was to stop on a conflict");
    repo
}

/// The project with a merge that stopped on a conflict it has met before:
/// the first time `docs/guide.md` was resolved to `resolution` and the
/// merge was made with rerere switched on; then it was taken back and
/// begun again, and git put the remembered resolution into the file. The
/// path is still unmerged.
pub fn project_resolved_as_last_time(resolution: &str) -> PathBuf {
    let repo = project_in_conflict();
    let git = git_in(&repo);
    for (key, value) in [("rerere.enabled", "true"), ("rerere.autoUpdate", "false")] {
        git.run(["config", key, value]).unwrap();
    }
    // The conflict as it stands is what the resolution will be for.
    git.run(["rerere"]).unwrap();
    write(&repo, "docs/guide.md", resolution);
    git.run(["add", "docs/guide.md"]).unwrap();
    at(&git, 32, &["commit", "-q", "--no-edit"]);
    git.run(["reset", "-q", "--hard", "HEAD~1"]).unwrap();
    let date = clock_at(33);
    let merged = git.run_with(
        ["merge", "--no-ff", "-m", "Merge the guide", "feature/guide"],
        &[("GIT_AUTHOR_DATE", &date), ("GIT_COMMITTER_DATE", &date)],
    );
    assert!(merged.is_err(), "the merge was to stop on a conflict again");
    assert_eq!(
        fs::read_to_string(repo.join("docs/guide.md")).unwrap(),
        resolution,
        "git put the remembered resolution into the file"
    );
    assert_eq!(
        git.run(["ls-files", "--unmerged", "--format=%(path)"])
            .unwrap()
            .lines()
            .next(),
        Some("docs/guide.md"),
        "and left it unmerged"
    );
    repo
}

/// A library of its own to take in as a submodule: two commits on
/// `main`, and a branch `stable` that stayed at the first.
pub fn library() -> PathBuf {
    let library = work().join("library");
    fs::create_dir_all(&library).unwrap();
    let lib = git_in(&library);
    lib.run(["init", "-q", "-b", "main"]).unwrap();
    write(&library, "lib.txt", "one\n");
    lib.run(["add", "-A"]).unwrap();
    at(&lib, 20, &["commit", "-q", "-m", "library: one"]);
    lib.run(["branch", "stable"]).unwrap();
    write(&library, "lib.txt", "one\ntwo\n");
    lib.run(["add", "-A"]).unwrap();
    at(&lib, 21, &["commit", "-q", "-m", "library: two"]);
    library
}

/// Store a commit as it is given, signature and all, and say its id.
pub fn store_commit(repo: &Path, object: &str) -> String {
    use std::io::Write as _;
    let mut child = git_in(repo)
        .command()
        .args(["hash-object", "-t", "commit", "-w", "--stdin"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(object.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "git stored the commit");
    String::from_utf8_lossy(&out.stdout).trim().to_owned()
}

/// Whether a program is installed. Looked up, never run; on Windows with
/// `.exe` at its end as well.
pub fn on_path(program: &str) -> bool {
    let exe = format!("{program}.exe");
    std::env::var_os("PATH").is_some_and(|paths| {
        std::env::split_paths(&paths)
            .any(|dir| dir.join(program).is_file() || (cfg!(windows) && dir.join(&exe).is_file()))
    })
}

/// An SSH key made for one test, without a passphrase, in the directory
/// of repositories; the public half is `<path>.pub`. Nothing of anybody's
/// home is read or written.
pub fn ssh_key(name: &str) -> PathBuf {
    let dir = work().join("keys");
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join(name);
    let status = std::process::Command::new("ssh-keygen")
        .args(["-q", "-t", "ed25519", "-N", "", "-C", name, "-f"])
        .arg(&path)
        .stdin(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "ssh-keygen made a key");
    path
}

/// Put a hook into a repository's own hooks directory: it says `says`
/// and ends with `status`. Written by a child process: a file this
/// process has open for writing while another thread starts a program is
/// "busy" to whoever then runs it.
pub fn hook(repo: &Path, name: &str, says: &str, status: i32) {
    use std::io::Write as _;
    let path = repo.join(".git/hooks").join(name);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut child = std::process::Command::new("sh")
        .arg("-c")
        .arg("cat > \"$0\"")
        .arg(&path)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(format!("#!/bin/sh\necho '{says}' >&2\nexit {status}\n").as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    runnable(&path);
}

/// Mark a script as one that can be run. Git for Windows runs one that
/// begins with `#!` whatever its mode.
fn runnable(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// The project with more files, hooks that refuse a merge, a rebase, a
/// push and a patch of a mailbox, and an empty remote to push to.
pub fn project_with_refusing_hooks() -> PathBuf {
    let repo = project_with_files();
    let origin = work().join("origin.git");
    let git = git_in(&repo);
    git.run(["init", "-q", "--bare", origin.to_str().unwrap()])
        .unwrap();
    git.run(["remote", "add", "origin", origin.to_str().unwrap()])
        .unwrap();
    for (name, says) in [
        ("pre-merge-commit", "the tests have not run"),
        ("pre-rebase", "not on a Friday"),
        ("pre-push", "protected: ask first"),
        ("applypatch-msg", "no patches by mail"),
    ] {
        hook(&repo, name, says, 1);
    }
    repo
}

/// A bare repository, which is opened read-only.
pub fn archive() -> PathBuf {
    fixture("bare", "archive.git")
}

/// A clone of the `search` fixture that left the trees of history on its
/// remote (`--filter=tree:0`), with a local branch for each of the
/// remote's, as a treeless clone that has been worked in has; and the file
/// in which its remote writes a line each time it is asked for something
/// (its `upload-pack` is a script that does that first: git-config(1)
/// `remote.<name>.uploadpack`).
pub fn treeless_clone() -> (PathBuf, PathBuf) {
    let source = fixture("search", "treeless-source");
    git(&source, &["config", "uploadpack.allowFilter", "true"]);
    let clone = work().join("treeless");
    git(
        &work(),
        &[
            "clone",
            "-q",
            "--no-local",
            "--filter=tree:0",
            source.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    let branches = git(
        &clone,
        &[
            "for-each-ref",
            "--format=%(refname:lstrip=3)",
            "refs/remotes/origin",
        ],
    );
    let head = git(&clone, &["branch", "--show-current"]);
    for branch in branches.lines() {
        if branch != "HEAD" && branch != head.trim() {
            git(
                &clone,
                &["branch", "-q", branch, &format!("origin/{branch}")],
            );
        }
    }
    // On a commit whose files need its parent's tree, which is not here:
    // the merge at the top of `main` changed nothing, and its parent's
    // tree is its own.
    git(&clone, &["checkout", "-q", "feature/dates"]);
    let log = work().join("asked.log");
    let script = work().join("upload-pack.sh");
    // Git runs it through its shell, which takes a `\` for an escape:
    // written with `/` (which Windows takes as well).
    let slashed = |path: &Path| path.to_str().unwrap().replace('\\', "/");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\necho asked >> '{}'\nexec git upload-pack \"$@\"\n",
            slashed(&log)
        ),
    )
    .unwrap();
    runnable(&script);
    git(
        &clone,
        &["config", "remote.origin.uploadpack", &slashed(&script)],
    );
    (clone, log)
}

/// How many times the remote of [`treeless_clone`] was asked for
/// something.
pub fn remote_asked(log: &Path) -> usize {
    fs::read_to_string(log)
        .map(|text| text.lines().count())
        .unwrap_or(0)
}

/// `count` packs more in a repository: copies of a pack that is there,
/// each under a name of its own, as many small fetches leave them.
pub fn more_packs(repo: &Path, count: usize) {
    let dir = repo.join(".git/objects/pack");
    let pack = fs::read_dir(&dir)
        .unwrap()
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| path.extension().is_some_and(|ext| ext == "pack"))
        .expect("a pack to copy");
    for n in 0..count {
        let name = format!("pack-{:040x}", 0x00fe_ed00_0000_usize.saturating_add(n));
        fs::copy(&pack, dir.join(format!("{name}.pack"))).unwrap();
        fs::copy(pack.with_extension("idx"), dir.join(format!("{name}.idx"))).unwrap();
    }
}

/// How many packs a repository's objects are in.
pub fn packs(repo: &Path) -> usize {
    fs::read_dir(repo.join(".git/objects/pack"))
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "pack"))
                .count()
        })
        .unwrap_or(0)
}

/// Say that a repository was last fetched at `seconds`: when its
/// `FETCH_HEAD` was written, which is what the page of repositories reads.
pub fn fetched_at(repo: &Path, seconds: i64) {
    let at = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds.unsigned_abs());
    fs::File::options()
        .write(true)
        .open(repo.join(".git/FETCH_HEAD"))
        .unwrap()
        .set_modified(at)
        .unwrap();
}

/// The id of a revision, in full.
pub fn rev(repo: &Path, name: &str) -> String {
    git_in(repo).rev_parse(name).unwrap()
}

/// Run git in a repository, for what a test sets up or looks at itself.
pub fn git(repo: &Path, args: &[&str]) -> String {
    git_in(repo).run(args).unwrap()
}

/// The same at hour `tick` of the fixtures' clock: a commit made at a
/// late hour is the newest, at the graph's top.
pub fn git_at(repo: &Path, tick: u64, args: &[&str]) -> String {
    at(&git_in(repo), tick, args)
}
