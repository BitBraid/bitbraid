//! bitbraid-engine: the command registry, execution context, journal and
//! undo, and the commands.
//!
//! Every user-visible behaviour is a command (see `AGENTS.md`
//! section 2): a handler per command, a context carrying the open
//! repository, JSON parameters in and JSON out, so the CLI and the UI
//! run the same code.
//!
//! Sources: none (original design).
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]

// `say!` and `message!`, in every module: the words come from the catalog.
#[macro_use]
extern crate bitbraid_strings;

pub mod access;
pub mod activity_cmds;
pub mod branch_cmds;
pub mod command;
pub mod command_words;
pub mod commit_cmds;
pub mod completion;
pub mod completion_cmds;
pub mod completion_table;
pub mod create_cmds;
pub mod file_cmds;
pub mod flow_cmds;
pub mod git_cmds;
pub mod glance_cmds;
pub mod housekeeping_cmds;
pub mod ignore_cmds;
pub mod index_cmds;
pub mod interactive_cmds;
pub mod journal;
pub mod journal_cmds;
pub mod lfs_cmds;
pub mod link_cmds;
pub mod maintenance_cmds;
pub mod message_cmds;
pub mod open_cmds;
pub mod params;
pub mod patch_cmds;
pub mod pending;
pub mod profile_cmds;
pub mod remote_cmds;
pub mod repo_cmds;
pub mod rerere_cmds;
pub mod rewrite_cmds;
pub mod search_cmds;
pub mod sequence_cmds;
mod settings;
pub mod sign_cmds;
pub mod signers_cmds;
pub mod sparse_cmds;
pub mod ssh_cmds;
pub mod stage_cmds;
pub mod stash_cmds;
pub mod strings;
pub mod submodule_cmds;
pub mod tag_cmds;
pub mod terminal_cmds;
pub mod tool_cmds;
pub mod windows_cmds;
pub mod workspace;
pub mod workspace_bulk;
pub mod workspace_cmds;
pub mod workspace_file;
pub mod workspace_scan;
pub mod worktree_cmds;

pub use access::{Access, Effect, Needs};
pub use command::{
    CommandContext, CommandError, CommandSpec, Extensions, Handler, Output, Params, ProgressSink,
    Registry, RegistryError,
};
pub use journal::{Change, Entry, Journal};
pub use params::{ParamKind, ParamSpec};

/// The registry with every built-in command.
pub fn builtin_registry() -> Registry {
    let mut registry = Registry::new();
    repo_cmds::register(&mut registry);
    search_cmds::register(&mut registry);
    stage_cmds::register(&mut registry);
    ignore_cmds::register(&mut registry);
    commit_cmds::register(&mut registry);
    message_cmds::register(&mut registry);
    file_cmds::register(&mut registry);
    branch_cmds::register(&mut registry);
    remote_cmds::register(&mut registry);
    rewrite_cmds::register(&mut registry);
    stash_cmds::register(&mut registry);
    submodule_cmds::register(&mut registry);
    tag_cmds::register(&mut registry);
    create_cmds::register(&mut registry);
    sequence_cmds::register(&mut registry);
    interactive_cmds::register(&mut registry);
    tool_cmds::register(&mut registry);
    worktree_cmds::register(&mut registry);
    glance_cmds::register(&mut registry);
    activity_cmds::register(&mut registry);
    journal_cmds::register(&mut registry);
    flow_cmds::register(&mut registry);
    housekeeping_cmds::register(&mut registry);
    sign_cmds::register(&mut registry);
    signers_cmds::register(&mut registry);
    profile_cmds::register(&mut registry);
    link_cmds::register(&mut registry);
    patch_cmds::register(&mut registry);
    index_cmds::register(&mut registry);
    rerere_cmds::register(&mut registry);
    lfs_cmds::register(&mut registry);
    git_cmds::register(&mut registry);
    open_cmds::register(&mut registry);
    terminal_cmds::register(&mut registry);
    completion_cmds::register(&mut registry);
    ssh_cmds::register(&mut registry);
    workspace_cmds::register(&mut registry);
    maintenance_cmds::register(&mut registry);
    sparse_cmds::register(&mut registry);
    windows_cmds::register(&mut registry);
    registry
}
