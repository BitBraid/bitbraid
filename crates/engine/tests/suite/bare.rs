//! A bare repository is opened read-only: every registered
//! command is run against one, with parameters that make sense and with
//! hostile ones, and afterwards the repository is what it was: the same
//! files with the same bytes, the same references and the same objects.
//! What reads is compared with git; what would change the repository is
//! refused before anything runs, and says that it is read-only.
//!
//! The commands run git with the developer's own environment, so whatever
//! could reach outside is pinned: gpg is a script that knows no key, the
//! SSH agent is not asked, the desktop is not written to, no tool of the
//! developer's is started, and nothing names the user's configuration.
#![cfg(unix)]

use std::{
    collections::BTreeMap,
    fs,
    os::unix::fs::PermissionsExt as _,
    panic::{AssertUnwindSafe, catch_unwind},
    path::{Path, PathBuf},
};

use bitbraid_engine::{
    Access, CommandContext, CommandError, Effect, Needs, Registry, builtin_registry,
};
use bitbraid_testkit::{Git, Scratch, fnv1a64};
use serde_json::{Value, json};

/// The bare copy, a directory beside it for what commands write on
/// request, and what the test needs to know about the history.
struct Bare {
    copy: Scratch,
    side: PathBuf,
    registry: Registry,
    /// Commits, newest first.
    commits: Vec<String>,
}

impl Bare {
    fn new() -> Self {
        let copy = Scratch::of("bare").unwrap();
        let side = copy.path().with_extension("side");
        let _ = fs::remove_dir_all(&side);
        fs::create_dir_all(side.join("ssh")).unwrap();
        fs::create_dir_all(side.join("config")).unwrap();
        // Whatever verifies or lists keys asks this, and it knows nothing.
        let nobody = side.join("no-gpg.sh");
        fs::write(&nobody, "#!/bin/sh\nexit 2\n").unwrap();
        fs::set_permissions(&nobody, fs::Permissions::from_mode(0o755)).unwrap();
        let git = copy.git();
        for (key, value) in [
            ("gpg.program", nobody.to_str().unwrap()),
            ("gpg.x509.program", nobody.to_str().unwrap()),
            ("gpg.ssh.program", nobody.to_str().unwrap()),
            ("gpg.format", "openpgp"),
            // Whatever the developer's own configuration names, the
            // signers that are listed here are nobody's.
            (
                "gpg.ssh.allowedSignersFile",
                side.join("allowed-signers").to_str().unwrap(),
            ),
            ("commit.gpgsign", "false"),
            ("tag.gpgsign", "false"),
            ("user.name", "Ada Fixture"),
            ("user.email", "ada@fixture.invalid"),
        ] {
            git.run(["config", "--local", key, value]).unwrap();
        }
        let commits = git.lines(["rev-list", "--all", "--date-order"]).unwrap();
        // A patch for the commands that read one.
        let patch = git.run(["format-patch", "--stdout", "-1", "v1"]).unwrap();
        fs::write(side.join("one.patch"), patch).unwrap();
        Self {
            copy,
            side,
            registry: builtin_registry(),
            commits,
        }
    }

    fn path(&self) -> &Path {
        self.copy.path()
    }

    fn git(&self) -> Git {
        self.copy.git()
    }

    fn inside(&self, rel: &str) -> String {
        self.path().join(rel).to_str().unwrap().to_owned()
    }

    fn beside(&self, rel: &str) -> String {
        self.side.join(rel).to_str().unwrap().to_owned()
    }

    /// A context with the bare repository open, fresh for every run: a
    /// command that opened something else must not hand that to the next.
    fn context(&self) -> CommandContext {
        let mut ctx = CommandContext {
            config_dir: Some(self.side.join("config")),
            ..Default::default()
        };
        self.registry
            .run("repo.open", &mut ctx, &json!({ "path": self.path() }))
            .expect("the bare repository opens");
        ctx
    }

    /// Every file of the repository with a digest of its bytes.
    fn files(&self) -> BTreeMap<String, (u64, u64)> {
        fn walk(dir: &Path, root: &Path, into: &mut BTreeMap<String, (u64, u64)>) {
            for entry in fs::read_dir(dir).unwrap() {
                let entry = entry.unwrap();
                let path = entry.path();
                let name = path.strip_prefix(root).unwrap().display().to_string();
                let kind = entry.file_type().unwrap();
                if kind.is_dir() {
                    into.insert(format!("{name}/"), (0, 0));
                    walk(&path, root, into);
                } else {
                    let bytes = fs::read(&path).unwrap_or_default();
                    into.insert(name, (bytes.len() as u64, fnv1a64(&bytes)));
                }
            }
        }
        let mut found = BTreeMap::new();
        walk(self.path(), self.path(), &mut found);
        found
    }

    /// What git says the repository holds: where HEAD is, every reference
    /// with its value, and every object there is, reachable or not.
    fn held(&self) -> (String, Vec<String>, Vec<String>) {
        let git = self.git();
        let head = git.run(["symbolic-ref", "HEAD"]).unwrap();
        let refs = git
            .lines(["for-each-ref", "--format=%(refname) %(objectname)"])
            .unwrap();
        let mut objects = git
            .lines(["cat-file", "--batch-all-objects", "--batch-check"])
            .unwrap();
        objects.sort();
        (head, refs, objects)
    }

    /// Parameters a command could well be given here. None names the
    /// user's configuration, the desktop or a program of the developer's.
    fn plausible(&self, id: &str) -> Vec<Value> {
        let head = self.commits[0].as_str();
        let first = self.commits.last().unwrap().as_str();
        let ids = json!([head, first]);
        match id {
            "activity.list" => vec![json!({ "limit": 5, "questions": true, "text": "log" })],
            "branch.overview" => {
                vec![json!({ "base": "main", "stale_days": 30, "now": 1_600_000_000 })]
            }
            "branch.predict" => vec![json!({ "target": "topic" }), json!({ "target": "v1" })],
            "commit.people" => vec![json!({ "limit": 10 })],
            "conflict.detail" => vec![json!({ "path": "a.txt" })],
            "file.bytes" => vec![
                json!({ "path": "a.txt", "side": "HEAD" }),
                json!({ "path": "a.txt", "side": "HEAD", "max": 1 }),
                json!({ "path": "a.txt" }),
            ],
            "file.open" => vec![
                json!({ "path": "a.txt", "command": "true {path}", "line": 3 }),
                // As it is in a commit: from a copy outside the repository.
                json!({ "path": "a.txt", "rev": "v1", "command": "true {path}" }),
            ],
            "file.reveal" => vec![json!({ "path": "a.txt", "command": "true" })],
            "lfs.files" => vec![json!({ "rev": "HEAD" })],
            "link.make" => vec![
                json!({ "branch": "main" }),
                json!({ "path": "/somewhere/else", "tag": "v1" }),
                json!({ "url": "https://example.invalid/a.git", "commit": head }),
            ],
            "link.parse" => vec![json!({ "url": "bitbraid://open?path=/somewhere&branch=main" })],
            "link.register" => vec![json!({
                "dry_run": true,
                "no_mime": true,
                "applications_dir": self.beside("applications"),
                "exe": "/usr/bin/true",
            })],
            "patch.check" => vec![json!({ "file": self.beside("one.patch") })],
            "patch.export" => vec![
                json!({ "commits": [head], "dir": self.beside("out"), "overwrite": true }),
                json!({ "from": "v1", "to": "main", "file": self.beside("out.mbox"), "overwrite": true }),
                // Into the repository itself: refused.
                json!({ "commits": [head], "file": self.inside("HEAD"), "overwrite": true }),
                json!({ "commits": [head], "dir": self.inside("refs/heads"), "overwrite": true }),
            ],
            "profile.current" => vec![
                json!({}),
                json!({ "scope": "repo" }),
                json!({ "scope": { "file": self.inside("config") } }),
            ],
            "profile.delete" => vec![json!({ "name": "Work" })],
            "profile.save" => vec![json!({
                "profile": { "name": "Work", "user_name": "Ada", "user_email": "ada@fixture.invalid" },
            })],
            "profile.use" => vec![
                json!({ "name": "Work", "scope": "repo" }),
                json!({ "name": "Work", "scope": { "file": self.inside("config") } }),
                json!({ "name": "Work", "scope": { "file": self.beside("a.gitconfig") } }),
            ],
            "rebase.outline" => vec![json!({
                "first": head,
                "steps": [{ "id": head, "action": "reword", "message": "again" }],
            })],
            "rebase.plan" => vec![json!({ "first": head }), json!({ "first": first })],
            "repo.blame" => vec![
                json!({ "path": "a.txt", "rev": "HEAD" }),
                json!({ "path": "a.txt", "rev": "HEAD", "ignore_revs": false }),
            ],
            "repo.blame_previous" => vec![json!({ "path": "a.txt", "rev": "HEAD" })],
            "repo.clone" => vec![
                json!({ "url": self.path(), "path": self.beside("cloned") }),
                json!({ "url": self.path(), "path": self.path() }),
                json!({ "url": self.path(), "path": self.inside("HEAD") }),
            ],
            "repo.commits" | "sign.signed" | "sign.verify" => vec![json!({ "ids": ids })],
            "repo.diff" => vec![
                json!({ "old": "v1", "new": "main", "context": 1 }),
                json!({ "old": "v1", "new": "main", "max_bytes": 3 }),
                json!({ "old": "HEAD", "new": "worktree" }),
                json!({ "old": "index", "new": "HEAD", "paths": ["a.txt"] }),
            ],
            "repo.graph" => vec![json!({ "limit": 10, "details": true, "ascii": true })],
            "repo.history" => vec![json!({ "path": "a.txt", "follow": true })],
            "repo.init" => vec![
                json!({ "path": self.beside("made"), "branch": "main" }),
                // The bare repository itself, and inside it.
                json!({ "path": self.path(), "branch": "other" }),
                json!({ "path": self.path(), "first_commit": true }),
                json!({ "path": self.inside("HEAD") }),
            ],
            "repo.log" => vec![json!({ "limit": 3, "tips": "head", "first_parent": true })],
            "repo.maintenance" => vec![json!({ "task": "gc" }), json!({ "task": "prune" })],
            "repo.size" => vec![json!({ "blobs": 3, "commits": 10 })],
            "sparse.set" | "sparse.add" => vec![
                json!({ "directories": ["sub"] }),
                json!({ "patterns": ["/*"] }),
            ],
            "repo.open" => vec![json!({ "path": self.path() })],
            // With a command of their own, so that no file manager, editor
            // or terminal of whoever runs the tests is started.
            "repo.open_editor" | "repo.open_folder" | "repo.open_terminal" => vec![
                json!({ "command": "true" }),
                json!({ "command": "true", "path": self.path() }),
            ],
            "rerere.enable" => vec![
                json!({ "on": true }),
                json!({ "on": false, "scope": "repo" }),
            ],
            "search.commits" => vec![json!({ "query": "author:ada b1", "limit": 5 })],
            "search.content" => vec![json!({ "text": "topic", "paths": ["b.txt"] })],
            "search.locate" => vec![json!({ "rev": "v1" }), json!({ "date": 1_577_840_000 })],
            "search.parse" => vec![json!({ "query": "is:merge -author:bob \"quoted text\"" })],
            "sign.configure" => vec![
                json!({ "format": "ssh", "key": "key", "commits": true, "tags": true }),
                json!({ "commits": false, "scope": "repo" }),
            ],
            "sign.keys" => vec![json!({ "ssh_dir": self.beside("ssh"), "agent": false })],
            "submodule.changes" => vec![json!({ "path": "sub", "old": "v1", "new": "HEAD" })],
            "tag.details" => vec![json!({ "name": "v2" }), json!({ "name": "v1" })],
            // Answers only: no shell is started.
            "terminal.shell" => vec![
                json!({ "program": "/bin/sh", "args": ["-i"] }),
                json!({ "path": self.path(), "program": "sh" }),
                json!({ "path": "refs" }),
                json!({ "path": self.beside("") }),
            ],
            "terminal.paste_paths" => vec![
                json!({ "paths": [self.inside("HEAD"), "/a b"], "shell": "posix" }),
                json!({ "paths": ["/x"], "shell": "pwsh" }),
            ],
            "tool.diff" => vec![json!({ "path": "a.txt", "command": "true", "commit": head })],
            "worktree.list" => vec![json!({ "changes": true }), json!({ "changes": false })],

            // What would change the repository.
            "branch.checkout" => vec![json!({ "target": "topic", "dirty": "discard" })],
            "branch.create" => vec![
                json!({ "name": "made" }),
                json!({ "name": "made", "start": "v1", "checkout": true }),
            ],
            "branch.delete" => vec![json!({ "name": "topic", "force": true })],
            "branch.delete_remote" => vec![json!({ "remote": "origin", "branch": "topic" })],
            "branch.delete_merged" => vec![
                json!({ "base": "main" }),
                json!({ "base": "main", "dry_run": true }),
            ],
            "branch.describe" => vec![json!({ "name": "main", "description": "the trunk" })],
            "branch.fast_forward" => vec![json!({ "name": "next" })],
            "branch.merge" => vec![json!({ "target": "topic", "mode": "no_ff" })],
            "branch.rebase" => vec![json!({ "onto": "topic" })],
            "branch.rename" => vec![json!({ "name": "topic", "new_name": "renamed" })],
            "branch.reset" => vec![json!({ "target": "v1", "mode": "hard" })],
            "branch.set_upstream" => vec![json!({ "name": "main", "upstream": null })],
            "commit.amend" => vec![json!({ "message": "amended" })],
            "commit.messages" => vec![json!({ "limit": 5 })],
            "commit.trailer" => vec![json!({
                "message": "a summary\n\nA body.",
                "trailers": [{ "key": "Reviewed-by", "value": "Bob <bob@fixture.invalid>" }],
                "sign_off": true,
            })],
            "commit.trailer_keys" => vec![json!({ "limit": 20 })],
            "commit.cherry_pick" | "commit.revert" => vec![json!({ "commits": [first] })],
            "commit.create" => vec![json!({ "message": "made", "allow_empty": true })],
            "commit.drop" => vec![json!({ "commit": head })],
            "commit.reword" => vec![json!({ "commit": head, "message": "again" })],
            "commit.squash" => vec![json!({ "commits": [head] })],
            "conflict.reopen" | "conflict.resolve" => vec![json!({ "path": "a.txt" })],
            "conflict.take" => vec![json!({ "path": "a.txt", "side": "ours" })],
            "conflict.write" => vec![json!({ "path": "a.txt", "text": "resolved\n" })],
            "discard.hunks" | "stage.hunks" | "unstage.hunks" => {
                vec![json!({ "path": "a.txt", "selection": [{ "hunk": 0 }], "context": 3 })]
            }
            "discard.paths" | "stage.paths" | "unstage.paths" | "ignore.untrack" => {
                vec![json!({ "paths": ["a.txt"] })]
            }
            "file.restore" => vec![json!({ "path": "a.txt", "rev": "v1" })],
            "file.write" => vec![json!({ "path": "a.txt", "text": "written\n" })],
            "flow.finish" | "flow.start" => vec![json!({ "kind": "feature", "name": "x" })],
            "flow.init" => vec![json!({ "main": "main", "develop": "next" })],
            "ignore.add" => vec![
                json!({ "pattern": "*.log", "target": "local" }),
                json!({ "pattern": "*.log", "target": "repository" }),
                json!({ "path": "a.txt", "what": "extension", "target": "local" }),
            ],
            "ignore.tracked" => vec![json!({ "limit": 3 })],
            "index.set_flag" => {
                vec![json!({ "paths": ["a.txt"], "flag": "skip_worktree", "on": true })]
            }
            "lfs.fetch" | "lfs.push" => vec![json!({ "all": true })],
            "lfs.lock" | "rerere.forget" => vec![json!({ "path": "a.txt" })],
            "rerere.forget_record" => vec![json!({ "id": "ab".repeat(20) })],
            "rerere.records" => vec![json!({ "limit": 5 })],
            "lfs.track" | "lfs.untrack" => vec![json!({ "pattern": "*.bin" })],
            "lfs.unlock" => vec![json!({ "path": "a.txt", "force": true })],
            "patch.apply" => vec![json!({ "file": self.beside("one.patch"), "index": true })],
            "patch.mailbox" => vec![json!({ "file": self.beside("one.patch") })],
            "rebase.interactive" => vec![json!({
                "first": head,
                "steps": [{ "id": head, "action": "drop" }],
            })],
            "redo.to" | "undo.to" => vec![json!({ "id": 1 })],
            "remote.add" => vec![json!({ "name": "origin", "url": self.beside("remote.git") })],
            "remote.fetch" => vec![json!({ "prune": true, "tags": true })],
            "remote.pull" => vec![json!({ "mode": "rebase" })],
            "remote.push" => vec![json!({ "branch": "main", "force": "force", "tags": true })],
            "remote.remove" => vec![json!({ "name": "origin" })],
            "remote.set_url" => vec![json!({ "name": "origin", "url": self.beside("other.git") })],
            "stash.apply" | "stash.drop" | "stash.pop" => vec![json!({ "stash": head })],
            "stash.branch" => vec![json!({ "name": "from-stash" })],
            "stash.push" => vec![json!({ "message": "aside", "include_untracked": true })],
            "submodule.add" => vec![json!({
                "url": self.path(), "path": "sub", "allow_local": true,
            })],
            "submodule.remove" => vec![json!({ "path": "sub", "force": true, "purge": true })],
            "submodule.forget" => vec![json!({ "name": "sub" })],
            "sign.signers_file" => vec![
                json!({ "path": self.beside("allowed-signers"), "scope": "repo" }),
                json!({ "path": self.beside("other-signers") }),
            ],
            "sign.allow" => vec![json!({
                "principals": ["ada@fixture.invalid"],
                "key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINBvEivhFTA8QiNTh5Dc8w5ak5Ep6BH0UpKZPXwB9XXH",
            })],
            "sign.disallow" => vec![json!({ "key": "SHA256:nobody" })],
            "submodule.sync" | "submodule.update" => vec![json!({ "paths": ["sub"] })],
            "tag.create" => vec![
                json!({ "name": "v3" }),
                json!({ "name": "v3", "target": "v1", "message": "third" }),
            ],
            "tag.delete" => vec![json!({ "name": "v1" })],
            "tag.delete_remote" | "tag.push" => vec![json!({ "name": "v1", "remote": "origin" })],
            "tool.merge" => vec![json!({ "path": "a.txt", "command": "true" })],
            "worktree.add" => vec![json!({ "path": self.beside("tree"), "new_branch": "tree" })],
            "worktree.lock" => vec![json!({ "path": self.beside("tree"), "reason": "kept" })],
            "worktree.remove" => vec![json!({
                "path": self.beside("tree"), "force": true, "delete_branch": true,
            })],
            "worktree.unlock" => vec![json!({ "path": self.beside("tree") })],
            _ => vec![json!({})],
        }
    }

    /// Values nobody means well with, to be put in the place of each
    /// parameter in turn.
    fn hostile(&self) -> Vec<Value> {
        vec![
            Value::Null,
            json!(""),
            json!("-"),
            json!("--all"),
            // Would write a file into the repository, were it taken as an
            // option by the git that is run.
            json!(format!("--output={}", self.inside("injected"))),
            json!(self.inside("injected")),
            json!("../../../../../../etc/passwd"),
            json!("a\u{0}b"),
            json!("one\ntwo\r\n"),
            json!("refs/heads/main^{/b1}@{-1}..HEAD:"),
            json!("\u{202e}\u{feff}名前 🦑"),
            json!("x".repeat(70_000)),
            json!(-1),
            json!(0),
            json!(u64::MAX),
            json!(1e308),
            json!(true),
            json!([]),
            json!({}),
            json!(["--force", "", null, 7, ["nested"]]),
            json!({ "file": self.inside("config") }),
        ]
    }

    /// The parameters a command is tried with: the plausible ones, each of
    /// them with every parameter replaced by every hostile value, and
    /// shapes that are no parameters at all.
    ///
    /// The parameters that keep a command away from the developer's own
    /// desktop, keys and tools stay as they are, and for such a command
    /// the shapes without them are left out.
    fn variants(&self, id: &str) -> Vec<Value> {
        let pinned: &[&str] = match id {
            "sign.keys" => &["ssh_dir", "agent"],
            "link.register" => &["dry_run", "no_mime", "applications_dir"],
            "file.open" | "file.reveal" | "tool.diff" | "tool.merge" => &["command"],
            "repo.open_editor" | "repo.open_folder" | "repo.open_terminal" => &["command"],
            _ => &[],
        };
        let plausible = self.plausible(id);
        let mut all = plausible.clone();
        for params in &plausible {
            let Some(object) = params.as_object() else {
                continue;
            };
            for key in object.keys().filter(|key| !pinned.contains(&key.as_str())) {
                for value in self.hostile() {
                    let mut changed = object.clone();
                    changed.insert(key.clone(), value);
                    all.push(Value::Object(changed));
                }
            }
        }
        if pinned.is_empty() {
            all.extend([
                Value::Null,
                json!({}),
                json!([]),
                json!("text"),
                json!(7),
                json!({ "": "", "unknown": { "deep": [1, 2, 3] } }),
            ]);
        }
        for params in &all {
            assert!(
                !names_the_user(params),
                "{id} {params} would reach the user's own configuration"
            );
        }
        all
    }
}

impl Drop for Bare {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.side);
    }
}

/// Whether the text `user` stands anywhere in the parameters: as a scope
/// it means the configuration of whoever runs the tests.
fn names_the_user(params: &Value) -> bool {
    match params {
        Value::String(text) => text == "user",
        Value::Array(items) => items.iter().any(names_the_user),
        Value::Object(fields) => fields.values().any(names_the_user),
        _ => false,
    }
}

/// Run every command whose id `pick` takes, with all its variants, and
/// hold the repository to what it was after each.
fn run_all(pick: impl Fn(usize) -> bool) {
    let bare = Bare::new();
    let files = bare.files();
    let held = bare.held();
    let ids: Vec<&str> = bare.registry.iter().map(|spec| spec.id).collect();
    let mut ran = 0usize;
    for (position, id) in ids.iter().enumerate() {
        if !pick(position) {
            continue;
        }
        let access: Access = bare.registry.access(id).unwrap();
        for params in bare.variants(id) {
            let mut ctx = bare.context();
            let shown: String = params.to_string().chars().take(300).collect();
            let result = catch_unwind(AssertUnwindSafe(|| {
                bare.registry.run(id, &mut ctx, &params)
            }))
            .unwrap_or_else(|_| panic!("{id} {shown} panicked"));
            if access.changes_repository(&params) {
                assert!(
                    matches!(result, Err(CommandError::ReadOnly(_))),
                    "{id} {shown} was not refused as a change to a bare repository: {result:?}"
                );
            } else {
                assert!(
                    !matches!(result, Err(CommandError::ReadOnly(_))),
                    "{id} {shown} changes nothing and was refused"
                );
            }
            ran += 1;
            let now = bare.files();
            if now != files {
                let changed: Vec<&String> = now
                    .iter()
                    .filter(|(name, digest)| files.get(*name) != Some(digest))
                    .map(|(name, _)| name)
                    .chain(files.keys().filter(|name| !now.contains_key(*name)))
                    .collect();
                panic!("{id} {shown} changed the bare repository: {changed:?}");
            }
        }
    }
    assert!(ran > 500, "{ran} runs are too few to have tried much");
    assert_eq!(bare.held(), held, "references and objects");
}

// Four tests, so that the commands are tried side by side.
#[test]
fn no_command_changes_a_bare_repository_first_quarter() {
    run_all(|position| position % 4 == 0);
}

#[test]
fn no_command_changes_a_bare_repository_second_quarter() {
    run_all(|position| position % 4 == 1);
}

#[test]
fn no_command_changes_a_bare_repository_third_quarter() {
    run_all(|position| position % 4 == 2);
}

#[test]
fn no_command_changes_a_bare_repository_last_quarter() {
    run_all(|position| position % 4 == 3);
}

#[test]
fn what_would_change_a_bare_repository_says_that_it_is_read_only() {
    let bare = Bare::new();
    let held = bare.held();
    let mut ctx = bare.context();
    // A branch to be created: refused, and the error names the command.
    let err = bare
        .registry
        .run("branch.create", &mut ctx, &json!({ "name": "made" }))
        .unwrap_err();
    assert!(matches!(err, CommandError::ReadOnly(_)), "{err:?}");
    let said = err.to_string();
    assert!(
        said.contains("bare repository") && said.contains("read-only"),
        "{said}"
    );
    assert!(
        said.contains("Create branch") && said.contains("branch.create"),
        "{said}"
    );
    assert!(bare.git().refs("refs/heads").unwrap() == ["main", "next", "topic"]);

    // Every command that writes, whatever it is given.
    let writers: Vec<&str> = bare
        .registry
        .iter()
        .map(|spec| spec.id)
        .filter(|id| bare.registry.access(id).unwrap().effect == Effect::Writes)
        .collect();
    assert!(writers.len() > 60, "{}", writers.len());
    for id in writers {
        for params in [json!({}), Value::Null] {
            let err = bare.registry.run(id, &mut ctx, &params).unwrap_err();
            assert!(matches!(err, CommandError::ReadOnly(_)), "{id}: {err:?}");
        }
    }
    assert_eq!(bare.held(), held);

    // The same commands are not refused for being what they are: in a
    // repository with files they get as far as their parameters.
    let copy = Scratch::of("linear").unwrap();
    let mut ctx = CommandContext::default();
    bare.registry
        .run("repo.open", &mut ctx, &json!({ "path": copy.path() }))
        .unwrap();
    let err = bare
        .registry
        .run("branch.create", &mut ctx, &json!({}))
        .unwrap_err();
    assert!(matches!(err, CommandError::InvalidParam { .. }), "{err:?}");
    bare.registry
        .run("branch.create", &mut ctx, &json!({ "name": "made" }))
        .unwrap();
}

#[test]
fn a_bare_repository_is_read_like_any_other() {
    let bare = Bare::new();
    let git = bare.git();
    let mut ctx = bare.context();
    let mut run = |id: &str, params: Value| {
        bare.registry
            .run(id, &mut ctx, &params)
            .unwrap_or_else(|err| panic!("{id}: {err}"))
    };

    let summary = run("repo.summary", json!({}));
    assert_eq!(summary["bare"], true);
    assert!(summary["workdir"].is_null());
    assert_eq!(summary["head_branch"], "main");
    assert_eq!(summary["head"], git.rev_parse("HEAD").unwrap());

    let refs = run("repo.refs", json!({}));
    let branches: Vec<String> = refs["branches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|branch| {
            format!(
                "{} {}",
                branch["full_name"].as_str().unwrap(),
                branch["target"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        branches,
        git.lines([
            "for-each-ref",
            "--format=%(refname) %(objectname)",
            "refs/heads"
        ])
        .unwrap()
    );
    let tags: Vec<String> = refs["tags"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tag| {
            format!(
                "{} {}",
                tag["name"].as_str().unwrap(),
                tag["target"].as_str().unwrap()
            )
        })
        .collect();
    let want: Vec<String> = ["v1", "v2"]
        .iter()
        .map(|name| {
            format!(
                "{name} {}",
                git.rev_parse(&format!("{name}^{{commit}}")).unwrap()
            )
        })
        .collect();
    assert_eq!(tags, want);
    assert!(refs["worktrees"].as_array().unwrap().is_empty());

    let log = run("repo.log", json!({}));
    let ours: Vec<&str> = log
        .as_array()
        .unwrap()
        .iter()
        .map(|commit| commit["id"].as_str().unwrap())
        .collect();
    assert_eq!(ours, bare.commits);

    // No worktree: no status, no stash, and a file is what a commit has.
    assert_eq!(run("repo.status", json!({})), json!([]));
    assert_eq!(run("stash.list", json!({}))["stashes"], json!([]));
    let bytes = run("file.bytes", json!({ "path": "a.txt", "side": "HEAD" }));
    assert_eq!(bytes["exists"], true);
    assert_eq!(bytes["id"], git.rev_parse("HEAD:a.txt").unwrap());
    assert!(
        bare.registry
            .run(
                "file.bytes",
                &mut bare.context(),
                &json!({ "path": "a.txt" })
            )
            .is_err(),
        "there is no worktree to read a file from"
    );
    let plan = run("rebase.plan", json!({ "first": bare.commits[0] }));
    assert!(plan.to_string().contains(&bare.commits[0]), "{plan}");
    let diff = run("repo.diff", json!({ "old": "v1", "new": "main" }));
    let paths: Vec<&str> = diff
        .as_array()
        .unwrap()
        .iter()
        .map(|file| file["path"].as_str().unwrap())
        .collect();
    assert_eq!(
        paths,
        git.lines(["diff", "--name-only", "v1", "main"]).unwrap()
    );
    let blame = run("repo.blame", json!({ "path": "a.txt" }));
    assert_eq!(blame.as_array().unwrap().len(), 1);
    let history = run("repo.history", json!({ "path": "a.txt" }));
    assert_eq!(
        history.as_array().unwrap().len(),
        git.lines(["log", "--format=%H", "--", "a.txt"])
            .unwrap()
            .len()
    );
    let found = run("search.commits", json!({ "query": "is:merge" }));
    assert_eq!(
        found["ids"],
        json!(git.lines(["rev-list", "--all", "--merges"]).unwrap()),
        "{found}"
    );

    // What needs no repository works beside one that is bare, and a clone
    // of it is a way to work with it.
    let cloned = bare.beside("clone-to-work-in");
    run("repo.clone", json!({ "url": bare.path(), "path": cloned }));
    assert!(Path::new(&cloned).join(".git").is_dir());
    assert_eq!(
        run("link.make", json!({ "branch": "main" }))["link"]
            .as_str()
            .map(|link| link.starts_with("bitbraid://")),
        Some(true)
    );
}

#[test]
fn the_registry_says_what_each_command_needs() {
    let registry = builtin_registry();
    assert_eq!(registry.access("no.such"), None);
    let free: Vec<&str> = registry
        .iter()
        .map(|spec| spec.id)
        .filter(|id| registry.access(id).unwrap().repository == Needs::Nothing)
        .collect();
    assert_eq!(
        free,
        [
            "git.info",
            "git.use",
            "link.parse",
            "link.register",
            "profile.delete",
            "profile.list",
            "profile.save",
            "repo.clone",
            "repo.glance",
            "repo.ignore_templates",
            "repo.init",
            "repo.open",
            "repo.terminals",
            "search.parse",
            "ssh.key_fingerprint",
            "ssh.key_generate",
            "terminal.paste_paths",
            "tool.known",
            "workspace.add",
            "workspace.clone_missing",
            "workspace.create",
            "workspace.defaults",
            "workspace.delete",
            "workspace.export",
            "workspace.fetch",
            "workspace.import",
            "workspace.import_code_workspace",
            "workspace.list",
            "workspace.locate",
            "workspace.pull",
            "workspace.remove",
            "workspace.reorder",
            "workspace.scan",
            "workspace.sync_folder",
            "workspace.update",
        ]
    );
    let either: Vec<&str> = registry
        .iter()
        .map(|spec| spec.id)
        .filter(|id| registry.access(id).unwrap().repository == Needs::RepositoryIfOpen)
        .collect();
    assert_eq!(
        either,
        [
            "link.make",
            "profile.current",
            "profile.use",
            "repo.open_editor",
            "repo.open_folder",
            "repo.open_terminal",
        ]
    );
    // What is free of a repository runs without one; the rest says so.
    for spec in registry.iter() {
        let access = registry.access(spec.id).unwrap();
        // The one command that writes to the desktop is never run with
        // nothing to hold it back, whatever the table comes to say of it.
        if access.repository != Needs::Repository || spec.id == "link.register" {
            continue;
        }
        let mut ctx = CommandContext::default();
        let result = registry.run(spec.id, &mut ctx, &json!({}));
        // A parameter that is missing may be noticed first.
        assert!(
            matches!(
                result,
                Err(CommandError::NoRepository | CommandError::InvalidParam { .. })
            ),
            "{}: {result:?}",
            spec.id
        );
    }
}
