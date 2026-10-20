//! The command line, tested, as one program: the code the modules share is
//! compiled and linked once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

mod app_proxy;
mod askpass;
mod lfs_tls;
mod live_lfs;
mod live_lfs_github;
mod orphans;
mod real_proxy;
mod run;
mod shipped_git;
mod shipped_mingit;
mod tls_front;
mod token_signing;
mod transport_auth;
mod transport_proxy;

/// One askpass server at a time, for every module: where git children
/// look is global to the process, and the modules' tests run side by side
/// in it.
static ASKPASS_SERVER: std::sync::Mutex<()> = std::sync::Mutex::new(());
