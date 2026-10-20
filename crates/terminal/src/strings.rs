//! The words the terminal says to a person: each is a message in `strings/en/<area>.ftl`,
//! one file for each area of the crate, looked up with `say!` (and named
//! in a table with `message!`). See `bitbraid_strings` for how.
//!
//! The test below holds the crate to it: every lookup is in the catalog
//! with the arguments its message takes, and every message is looked up.
//!
//! Sources: Project Fluent's syntax guide
//! (<https://projectfluent.org/fluent/guide/>).

bitbraid_strings::catalog!["session",];

#[cfg(test)]
mod tests {
    #[test]
    fn every_lookup_is_in_the_catalog_and_every_message_is_looked_up() {
        let problems =
            bitbraid_strings::check::catalog(&super::CATALOG, env!("CARGO_MANIFEST_DIR"));
        assert!(problems.is_empty(), "{}", problems.join("\n"));
    }
}
