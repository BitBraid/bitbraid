//! The words the engine says to a person: command labels and
//! descriptions, what parameters are, what completion offers, journal
//! entries and errors. Each is a message in `strings/en/<area>.ftl`, one
//! file for each area of the crate, looked up with `say!` (and named in a
//! table with `message!`). See `bitbraid_strings` for how.
//!
//! The test below holds the crate to it: every lookup is in the catalog
//! with the arguments its message takes, and every message is looked up.
//!
//! Sources: Project Fluent's syntax guide
//! (<https://projectfluent.org/fluent/guide/>).

bitbraid_strings::catalog![
    "activity",
    "branch",
    "command",
    "command-words",
    "commit",
    "completion",
    "create",
    "file",
    "flow",
    "git",
    "glance",
    "housekeeping",
    "ignore",
    "index",
    "interactive",
    "journal",
    "lfs",
    "link",
    "maintenance",
    "message",
    "open",
    "params",
    "patch",
    "profile",
    "remote",
    "repo",
    "rerere",
    "rewrite",
    "search",
    "sequence",
    "sign",
    "signers",
    "sparse",
    "ssh",
    "stage",
    "stash",
    "submodule",
    "tag",
    "terminal",
    "tool",
    "windows",
    "workspace",
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
