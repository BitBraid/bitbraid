//! bitbraid-ui: the GPUI desktop application.
//!
//! The shell with tabs, the start page, the references panel, the commit
//! graph, the commit details and the other views. Every repository read
//! runs on a per-repository session thread (`session`) that owns the engine
//! context.
//!
//! Sources: gpui-kit documentation.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::indexing_slicing
    )
)]
// Views hold GPUI entities, handles and subscriptions that do not implement Debug.
#![allow(missing_debug_implementations)]

// `say!` and `message!`, in every module: the words come from the catalog.
#[macro_use]
extern crate bitbraid_strings;

pub mod access;
pub mod app;
pub mod ask;
pub mod askpass;
pub mod avatars;
pub mod brand;
pub mod chrome;
pub mod fuzzy;
pub mod gitexe;
pub mod instance;
pub mod keymap;
pub mod literal;
pub mod memory;
pub mod menus;
pub mod motion;
pub mod notices;
pub mod palette;
pub mod prefs;
pub mod processor_time;
pub mod providers;
pub mod read_only;
pub mod session;
pub mod shell;
pub mod strings;
pub mod theme;
pub mod updates;
pub mod views;
pub mod zoom;

pub use app::run;
