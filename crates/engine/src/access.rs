//! What each command needs of a repository and what it may do to one.
//!
//! Two things hang on this. A bare repository is opened read-only:
//! [`crate::Registry::run`] refuses every command that could change it,
//! and that is the one place where it is refused. And a surface that
//! opens a repository on the caller's behalf (the command line) asks here
//! whether a command needs one at all, so that `repo.init`, `repo.clone`,
//! the links and the profiles run outside one.
//!
//! The answers are one table in this file rather than fields of
//! [`crate::CommandSpec`], so that a new command changes one line here
//! and none of the descriptors. A command that is not in the table
//! is taken for one that needs a repository and changes it, which is the
//! careful answer, and the tests of this module fail until it is entered:
//! every registered command has a line, and every line a command. A
//! command registered from a higher layer brings its own line instead
//! (`Registry::register_described`), and the registry is what
//! is asked: [`crate::Registry::access`].
//!
//! "Changes the repository" is meant widely: its references, objects,
//! index, files, configuration, the journal kept in it, what a tool that
//! is run keeps in the git directory (git-lfs and its copy of the locks
//! it lists), and also what a command does to a remote on its behalf (a
//! push). What a command writes elsewhere on request (a patch file, the
//! profiles of the application) is not a change to the repository.
//!
//! Sources: gitrepository-layout(5), gitglossary(7) ("bare repository").

use serde::Serialize;

use crate::command::Params;

/// Whether a command works on the repository that is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Needs {
    /// It cannot run without an open repository.
    Repository,
    /// It works on the open repository when there is one and without one
    /// otherwise, or its parameters say which.
    RepositoryIfOpen,
    /// It takes no notice of the open repository.
    Nothing,
}

/// What a command may do to the open repository.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// It changes nothing in the repository.
    Reads,
    /// It may change the repository.
    Writes,
    /// It writes configuration where its `scope` parameter says: the
    /// repository's, unless that is `"user"`.
    WritesScope,
}

/// What a command needs and what it may do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Access {
    /// Whether it needs an open repository.
    pub repository: Needs,
    /// What it may do to that repository.
    pub effect: Effect,
}

impl Access {
    /// Needs a repository and changes nothing in it.
    pub const READS: Self = Self {
        repository: Needs::Repository,
        effect: Effect::Reads,
    };
    /// Needs a repository and may change it. What a command that is not
    /// known is taken for.
    pub const WRITES: Self = Self {
        repository: Needs::Repository,
        effect: Effect::Writes,
    };
    /// Needs a repository and writes its configuration, or the user's.
    pub const WRITES_SCOPE: Self = Self {
        repository: Needs::Repository,
        effect: Effect::WritesScope,
    };
    /// Reads the open repository when there is one.
    pub const READS_IF_OPEN: Self = Self {
        repository: Needs::RepositoryIfOpen,
        effect: Effect::Reads,
    };
    /// Writes the configuration its `scope` names, which needs a
    /// repository only when it is the repository's.
    pub const WRITES_SCOPE_IF_OPEN: Self = Self {
        repository: Needs::RepositoryIfOpen,
        effect: Effect::WritesScope,
    };
    /// Has nothing to do with the open repository.
    pub const FREE: Self = Self {
        repository: Needs::Nothing,
        effect: Effect::Reads,
    };

    /// Whether running the command with these parameters could change the
    /// open repository.
    ///
    /// For a command with a scope only the user's configuration counts as
    /// elsewhere. A configuration file that is named could be the
    /// repository's own, so that counts as the repository, and so does a
    /// scope that is absent (the repository is what it then means) or
    /// that cannot be read.
    pub fn changes_repository(self, params: &Params) -> bool {
        match self.effect {
            Effect::Reads => false,
            Effect::Writes => true,
            Effect::WritesScope => {
                params.get("scope").and_then(|scope| scope.as_str()) != Some("user")
            }
        }
    }
}

/// Every command that is not `WRITES`, which is what the rest are.
const TABLE: &[(&str, Access)] = &[
    // The activity log is the application's, not the repository's.
    ("activity.clear", Access::READS),
    ("activity.list", Access::READS),
    ("branch.overview", Access::READS),
    // Writes the tree of the merge it tries, which nothing refers to.
    ("branch.predict", Access::READS),
    ("branch.tracking", Access::READS),
    ("commit.messages", Access::READS),
    ("commit.people", Access::READS),
    ("commit.rewrite_way", Access::READS),
    ("commit.template", Access::READS),
    // Answers with a message; commits nothing.
    ("commit.trailer", Access::READS),
    ("commit.trailer_keys", Access::READS),
    ("conflict.detail", Access::READS),
    ("conflict.list", Access::READS),
    ("conflict.resolved", Access::READS),
    ("file.bytes", Access::READS),
    // Start a program of the user's on a file; the repository stays.
    ("file.open", Access::READS),
    ("file.reveal", Access::READS),
    ("flow.status", Access::READS),
    // Which git every child of this process runs: nothing of a repository.
    ("git.info", Access::FREE),
    ("git.use", Access::FREE),
    ("hook.list", Access::READS),
    ("ignore.tracked", Access::READS),
    ("index.flags", Access::READS),
    ("lfs.files", Access::READS),
    ("lfs.status", Access::READS),
    ("link.make", Access::READS_IF_OPEN),
    ("link.parse", Access::FREE),
    ("link.register", Access::FREE),
    ("op.status", Access::READS),
    ("patch.check", Access::READS),
    // Writes the files it is told to, wherever that is.
    ("patch.export", Access::READS),
    ("profile.current", Access::READS_IF_OPEN),
    ("profile.delete", Access::FREE),
    ("profile.list", Access::FREE),
    ("profile.save", Access::FREE),
    ("profile.use", Access::WRITES_SCOPE_IF_OPEN),
    ("rebase.outline", Access::READS),
    ("rebase.plan", Access::READS),
    ("remote.push_preview", Access::READS),
    ("repo.blame", Access::READS),
    ("repo.blame_ignores", Access::READS),
    ("repo.blame_previous", Access::READS),
    ("repo.clone", Access::FREE),
    ("repo.commits", Access::READS),
    ("repo.diff", Access::READS),
    // git fsck: it reads every object and writes nothing.
    ("repo.fsck", Access::READS),
    // Looks at the folders it is given, from outside them.
    ("repo.glance", Access::FREE),
    ("repo.graph", Access::READS),
    ("repo.history", Access::READS),
    ("repo.ignore_templates", Access::FREE),
    ("repo.init", Access::FREE),
    ("repo.log", Access::READS),
    ("repo.long_paths", Access::READS),
    ("repo.long_paths_enable", Access::WRITES_SCOPE),
    // Changes which repository the context has open, not a repository.
    ("repo.open", Access::FREE),
    // Start a program of the user's in a folder: the repository's, or
    // one that is given by its full path, for which none is needed.
    ("repo.open_editor", Access::READS_IF_OPEN),
    ("repo.open_folder", Access::READS_IF_OPEN),
    ("repo.open_terminal", Access::READS_IF_OPEN),
    ("repo.refs", Access::READS),
    // Counts and lists objects and walks history; writes nothing.
    ("repo.size", Access::READS),
    ("repo.status", Access::READS),
    ("repo.summary", Access::READS),
    ("repo.terminals", Access::FREE),
    ("repo.wsl", Access::READS),
    ("rerere.enable", Access::WRITES_SCOPE),
    ("rerere.records", Access::READS),
    ("rerere.status", Access::READS),
    ("search.commits", Access::READS),
    ("search.content", Access::READS),
    ("search.files", Access::READS),
    ("search.locate", Access::READS),
    ("search.parse", Access::FREE),
    ("sign.configure", Access::WRITES_SCOPE),
    ("sign.keys", Access::READS),
    ("sign.signed", Access::READS),
    ("sign.signers", Access::READS),
    ("sign.signers_file", Access::WRITES_SCOPE),
    ("sign.status", Access::READS),
    ("sign.verify", Access::READS),
    ("sparse.status", Access::READS),
    // A key file of the user's, made or read: nothing of a repository.
    ("ssh.key_fingerprint", Access::FREE),
    ("ssh.key_generate", Access::FREE),
    ("stash.list", Access::READS),
    ("stash.show", Access::READS),
    ("submodule.changes", Access::READS),
    ("submodule.list", Access::READS),
    ("tag.details", Access::READS),
    // What a git command typed in a terminal is completed with.
    ("terminal.complete", Access::READS),
    ("terminal.completion_words", Access::READS),
    // How dropped paths are written for a shell; which shell a terminal
    // starts, and in which worktree.
    ("terminal.paste_paths", Access::FREE),
    ("terminal.shell", Access::READS),
    // Shows a diff in a program of the user's.
    ("tool.diff", Access::READS),
    // A look through PATH: nothing of a repository.
    ("tool.known", Access::FREE),
    ("tool.list", Access::READS),
    ("undo.journal", Access::READS),
    ("undo.peek", Access::READS),
    // The workspaces are the application's; a repository that is added is
    // only looked at.
    ("workspace.add", Access::FREE),
    // Clone, fetch and pull in the workspace's repositories, each opened
    // in a context of its own, whatever is open here: those commands are
    // refused in a bare one there as they would be here.
    ("workspace.clone_missing", Access::FREE),
    ("workspace.create", Access::FREE),
    // Writes a profile to a repository of the workspace, opened in a
    // context of its own.
    ("workspace.defaults", Access::FREE),
    ("workspace.delete", Access::FREE),
    // Writes the file it is told to, wherever that is.
    ("workspace.export", Access::FREE),
    ("workspace.fetch", Access::FREE),
    ("workspace.import", Access::FREE),
    ("workspace.import_code_workspace", Access::FREE),
    ("workspace.list", Access::FREE),
    ("workspace.locate", Access::FREE),
    ("workspace.pull", Access::FREE),
    ("workspace.remove", Access::FREE),
    ("workspace.reorder", Access::FREE),
    // Looks at the folders it is given, from outside them.
    ("workspace.scan", Access::FREE),
    ("workspace.sync_folder", Access::FREE),
    ("workspace.update", Access::FREE),
    ("worktree.changes", Access::READS),
    ("worktree.list", Access::READS),
];

/// The commands that change the repository they run in. They are listed
/// too, so that a command nobody has thought about is noticed: see the
/// tests.
#[cfg(test)]
const WRITERS: &[&str] = &[
    "branch.checkout",
    "branch.create",
    "branch.delete",
    "branch.delete_merged",
    "branch.delete_remote",
    "branch.describe",
    "branch.fast_forward",
    "branch.merge",
    "branch.rebase",
    "branch.rename",
    "branch.reset",
    "branch.set_upstream",
    "commit.amend",
    "commit.cherry_pick",
    "commit.create",
    "commit.drop",
    "commit.move",
    "commit.revert",
    "commit.reword",
    "commit.squash",
    "conflict.reopen",
    "conflict.resolve",
    "conflict.take",
    "conflict.write",
    "discard.hunks",
    "discard.paths",
    "file.restore",
    "file.write",
    "flow.finish",
    "flow.init",
    "flow.start",
    "ignore.add",
    "ignore.untrack",
    "index.set_flag",
    "lfs.checkout",
    "lfs.fetch",
    "lfs.install",
    "lfs.lock",
    // It only lists, and still writes: git-lfs keeps its own copy of the
    // list in the git directory each time it is asked (`lfs/lockcache.db`,
    // `lfs/cache/locks`). So it is refused in a bare repository, is not
    // offered where a repository is read-only, and is not run on every
    // refresh: the watcher would see the git directory change and ask
    // again.
    "lfs.locks",
    "lfs.pull",
    "lfs.push",
    "lfs.track",
    "lfs.unlock",
    "lfs.untrack",
    "op.abort",
    "op.continue",
    "op.skip",
    "patch.apply",
    "patch.mailbox",
    "rebase.interactive",
    "redo.last",
    "redo.to",
    "remote.add",
    "remote.fetch",
    "remote.pull",
    "remote.push",
    "remote.remove",
    "remote.set_url",
    "repo.maintenance",
    "rerere.forget",
    "rerere.forget_record",
    "sign.allow",
    "sign.disallow",
    "sparse.add",
    "sparse.disable",
    "sparse.set",
    "stage.hunks",
    "stage.paths",
    "stash.apply",
    "stash.branch",
    "stash.drop",
    "stash.pop",
    "stash.push",
    "stash.reword",
    "submodule.add",
    "submodule.forget",
    "submodule.remove",
    "submodule.sync",
    "submodule.update",
    "tag.annotate",
    "tag.create",
    "tag.delete",
    "tag.delete_remote",
    "tag.push",
    "tool.merge",
    "undo.last",
    "undo.to",
    "unstage.hunks",
    "unstage.paths",
    "worktree.add",
    "worktree.lock",
    "worktree.move",
    "worktree.prune",
    "worktree.remove",
    "worktree.unlock",
];

/// What the command `id` needs and may do. One that is not known needs a
/// repository and may change it.
pub fn of(id: &str) -> Access {
    TABLE
        .iter()
        .find(|(known, _)| *known == id)
        .map_or(Access::WRITES, |(_, access)| *access)
}

/// Whether `id` has been given an answer of its own, rather than the
/// careful one for what is not known.
#[cfg(test)]
pub fn is_known(id: &str) -> bool {
    WRITERS.contains(&id) || TABLE.iter().any(|(known, _)| *known == id)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn every_command_is_entered_and_every_entry_is_a_command() {
        let registry = crate::builtin_registry();
        let unknown: Vec<&str> = registry
            .iter()
            .map(|spec| spec.id)
            .filter(|id| !is_known(id))
            .collect();
        assert!(
            unknown.is_empty(),
            "say in crates/engine/src/access.rs what these commands need and do: {unknown:?}"
        );
        let stale: Vec<&str> = TABLE
            .iter()
            .map(|(id, _)| *id)
            .chain(WRITERS.iter().copied())
            .filter(|id| registry.get(id).is_none())
            .collect();
        assert!(stale.is_empty(), "no such commands: {stale:?}");
        let twice: Vec<&str> = TABLE
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| WRITERS.contains(id))
            .collect();
        assert!(twice.is_empty(), "entered as both: {twice:?}");
    }

    #[test]
    fn the_lists_are_in_order_and_without_repeats() {
        let ids: Vec<&str> = TABLE.iter().map(|(id, _)| *id).collect();
        assert!(ids.windows(2).all(|pair| pair[0] < pair[1]), "{ids:?}");
        assert!(WRITERS.windows(2).all(|pair| pair[0] < pair[1]));
    }

    #[test]
    fn listing_locks_counts_as_a_change() {
        // Git-lfs writes its cache into the git directory when it lists
        // them: what reads must not write there, so this does not read.
        assert_eq!(of("lfs.locks"), Access::WRITES);
        assert!(of("lfs.locks").changes_repository(&json!({})));
        // What it leaves alone still reads.
        assert_eq!(of("lfs.status"), Access::READS);
        assert_eq!(of("lfs.files"), Access::READS);
    }

    #[test]
    fn what_is_not_known_is_taken_for_a_change() {
        assert_eq!(of("no.such"), Access::WRITES);
        assert!(!is_known("no.such"));
        assert_eq!(of("repo.log"), Access::READS);
        assert_eq!(of("branch.create"), Access::WRITES);
        assert_eq!(of("repo.init").repository, Needs::Nothing);
    }

    #[test]
    fn only_the_users_scope_is_elsewhere() {
        let scoped = Access::WRITES_SCOPE;
        assert!(!scoped.changes_repository(&json!({ "scope": "user" })));
        for params in [
            json!({}),
            json!(null),
            json!({ "scope": null }),
            json!({ "scope": "repo" }),
            json!({ "scope": "USER" }),
            json!({ "scope": { "file": "/somewhere/config" } }),
            json!({ "scope": ["user"] }),
            json!({ "scope": 7 }),
            json!("user"),
        ] {
            assert!(scoped.changes_repository(&params), "{params}");
        }
        assert!(Access::WRITES.changes_repository(&json!({ "scope": "user" })));
        assert!(!Access::READS.changes_repository(&json!({ "scope": "repo" })));
    }
}
