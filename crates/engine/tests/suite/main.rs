//! The engine's commands, tested: one program, so that the code the modules
//! share is compiled and linked once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod activity;
mod activity_kept;
mod bare;
mod blame_previous;
mod blame_revs;
mod branches;
mod commit_box;
mod conflicts;
mod create;
mod diff_viewer;
mod discard;
mod enclosing;
mod explained;
mod file_actions;
mod find_file;
mod flow;
mod glance;
mod graph_order;
mod graph_stashes;
mod held;
mod housekeeping;
mod ignore;
mod index_flags;
mod interactive;
mod journal;
mod known_tools;
mod known_tools_live;
mod lfs;
mod lfs_locks;
mod links;
mod long_paths;
mod maintenance;
mod messages;
mod mutations;
mod odd_paths;
mod open;
mod packs_added;
mod params;
mod partial_clone;
mod patches;
mod pinned;
mod profiles;
mod remote_branches;
mod remotes;
mod rerere;
mod rerere_records;
mod rewrite_across;
mod rewrites;
mod search;
mod settings_undo;
mod signers;
mod signing;
mod signing_card;
mod sparse;
mod ssh_keys;
mod stashes;
mod stopping;
mod submodules;
mod tags;
mod terminal;
mod terminal_completion;
mod tools;
mod undo_files;
mod workspace_bulk;
mod workspace_files;
mod workspace_panic_hunt;
mod workspace_sources;
mod workspaces;
mod worktrees;
mod x509_chains;
