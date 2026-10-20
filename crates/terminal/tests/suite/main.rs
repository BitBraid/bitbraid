//! The terminal sessions, tested: one program, so the code the modules
//! share is compiled and linked once.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing
)]

// Shared with `tests/ending.rs`, a program of its own because it counts
// the threads of the whole process.
#[path = "../common/mod.rs"]
mod common;
mod quoting;
mod session;
mod shells;
mod throughput;
mod typed_line;
mod windows;
