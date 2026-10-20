//! bitbraid-integrations: what the application asks of other machines.
//! It is layer L3 (`AGENTS.md` section 1).
//!
//! [`http`] is the one way bytes are fetched over HTTPS, and [`avatars`]
//! the pictures of authors that are asked of Gravatar, or at the addresses
//! a provider gives, when the user chose so. The hosting providers
//! (`AGENTS.md` section 2): [`secrets`] keeps their tokens in the system's
//! keychain, [`accounts`] the accounts, [`oauth`] signs in by a code,
//! [`remotes`] links remotes to accounts, [`provider`] is the one face of
//! [`github`] and [`gitlab`], [`model`] the shapes they answer with,
//! [`search`] the words their lists are narrowed by, [`templates`] the
//! descriptions a pull request may start from, [`cache`] keeps their
//! answers for when they cannot be asked, [`replay`] their recorded answers
//! for the tests, and [`inbox`] what waits for the user across all of them.
//! [`update`] is the updater, which the user turns on. The commands of the
//! `account`, `provider`, `pr`, `issue`, `ci`, `inbox` and `update` areas
//! are registered by [`register`]; [`registry`] is the engine's commands
//! and these, which every surface builds.
//!
//! Sources: see the modules.
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

mod account_cmds;
pub mod accounts;
pub mod api;
pub mod avatars;
pub mod cache;
mod cmds;
pub mod github;
pub mod gitlab;
pub mod http;
pub mod inbox;
mod inbox_cmds;
mod issue_cmds;
pub mod keys;
pub mod model;
pub mod oauth;
mod pr_checkout;
mod pr_cmds;
pub mod provider;
mod provider_cmds;
pub mod remotes;
pub mod replay;
pub mod search;
pub mod secrets;
pub mod services;
pub mod strings;
pub mod suggest;
mod suggest_cmds;
pub mod templates;
pub mod update;
mod update_cmds;

pub use services::Services;

use bitbraid_engine::Registry;

/// Register the provider commands into `registry`.
pub fn register(registry: &mut Registry) {
    account_cmds::register(registry);
    provider_cmds::register(registry);
    pr_cmds::register(registry);
    suggest_cmds::register(registry);
    issue_cmds::register(registry);
    inbox_cmds::register(registry);
    update_cmds::register(registry);
}

/// The engine's commands and the provider commands: the registry every
/// surface builds.
pub fn registry() -> Registry {
    let mut registry = bitbraid_engine::builtin_registry();
    register(&mut registry);
    registry
}

/// One [`registry`] for the whole process, for what only looks commands
/// up: what one needs and may do, what it takes.
pub fn shared() -> &'static Registry {
    static SHARED: std::sync::OnceLock<Registry> = std::sync::OnceLock::new();
    SHARED.get_or_init(registry)
}
