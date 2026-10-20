//! The providers, the keychain and what talks to them, tested: one
//! program, so the code the modules share is compiled and linked once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod avatars;
mod common;
mod http_local;
mod http_proxy;
mod http_tls;
mod inbox;
mod issue_cmds;
mod keychain;
mod live_github;
mod live_gitlab_ce;
mod live_gitlab_com;
mod live_issue_branch;
mod pr_checkout;
mod provider_cache;
mod provider_panic_hunt;
mod providers;
mod real_proxy;
mod recorded;
mod recorded_github;
mod recorded_github_writes;
mod recorded_gitlab_com;
mod recorded_writes;
mod suggestions;
mod update;
mod writes_panic_hunt;
