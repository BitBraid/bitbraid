//! The repository layer, tested: one program, so that the code the modules
//! share is compiled and linked once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod bench_corpus;
mod corpus;
mod corpus_findings;
mod diff_guard;
mod head;
mod line_endings;
mod oracle;
mod oracle_worktree;
mod packs_added;
mod remote_urls;
mod settings_followed;
mod shallow;
mod stash_rows;
mod walk_tips;
mod watch;
