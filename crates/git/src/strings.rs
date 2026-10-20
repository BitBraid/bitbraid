//! The words the repository layer says to a person: what it refused and
//! why, what git's answers mean. Each is a message in
//! `strings/en/<area>.ftl`, one file for each area of the crate, looked up
//! with `say!` (and named in a table with `message!`). See
//! `bitbraid_strings` for how.
//!
//! The test below holds the crate to it: every lookup is in the catalog
//! with the arguments its message takes, and every message is looked up.
//!
//! Sources: Project Fluent's syntax guide
//! (<https://projectfluent.org/fluent/guide/>).

bitbraid_strings::catalog![
    "askpass",
    "blame",
    "branch",
    "commit",
    "create",
    "diff",
    "errors",
    "exe",
    "files-change",
    "find",
    "flow",
    "glance",
    "history",
    "hold",
    "hookwatch",
    "housekeeping",
    "identity",
    "ignore",
    "index-flags",
    "interactive",
    "issue-keys",
    "lfs",
    "log",
    "long-paths",
    "maintenance",
    "message",
    "oid",
    "patch",
    "promisor",
    "remote",
    "repo",
    "rerere",
    "rewrite",
    "search",
    "sequence",
    "settings",
    "signature",
    "signers",
    "snapshot",
    "sparse",
    "stage",
    "stash",
    "status",
    "submodule",
    "tag",
    "tools",
    "version",
    "watch",
    "worktree",
];

#[cfg(test)]
mod tests {
    #[test]
    fn every_lookup_is_in_the_catalog_and_every_message_is_looked_up() {
        let problems =
            bitbraid_strings::check::catalog(&super::CATALOG, env!("CARGO_MANIFEST_DIR"));
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
