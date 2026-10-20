//! The interface without a display.
//!
//! The real `Shell` is opened on fixture repositories on the toolkit's
//! test platform, driven with keys, clicks and actions, waited on until it
//! is at rest, and either asked what it shows or drawn into pictures that
//! are compared with the stored ones (through `cargo xtask snapshot`).
//!
//! Plain `cargo test` runs the tests of behaviour, which need no graphics
//! adapter. `cargo xtask snapshot --check` compares the pictures and
//! `cargo xtask snapshot` stores them anew; both skip, saying so, where
//! nothing can draw. See `AGENTS.md` section 3 for what is
//! stored and why.
//!
//! This is a program of its own rather than a set of `#[test]` functions:
//! it starts itself again inside scratch surroundings (`hermetic.rs`),
//! which a test cannot arrange for itself, and it keeps the application on
//! the main thread.
//!
//! Sources: the toolkit's own sources for its test support
//! (`gpui::HeadlessAppContext`, `gpui_wgpu::WgpuHeadlessRenderer` and
//! `CosmicTextSystem::new_without_system_fonts`, `gpui_kit::test`).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::print_stdout,
    missing_debug_implementations
)]

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour.rs"]
mod behaviour;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_access.rs"]
mod behaviour_access;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_busy.rs"]
mod behaviour_busy;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_commit.rs"]
mod behaviour_commit;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_drag.rs"]
mod behaviour_drag;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_files.rs"]
mod behaviour_files;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_fit.rs"]
mod behaviour_fit;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_graph.rs"]
mod behaviour_graph;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_providers.rs"]
mod behaviour_providers;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_refs.rs"]
mod behaviour_refs;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_settings.rs"]
mod behaviour_settings;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_shell.rs"]
mod behaviour_shell;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_terminal.rs"]
mod behaviour_terminal;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_terminal_completion.rs"]
mod behaviour_terminal_completion;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_terminal_shells.rs"]
mod behaviour_terminal_shells;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_terminal_text.rs"]
mod behaviour_terminal_text;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_undo.rs"]
mod behaviour_undo;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/behaviour_upkeep.rs"]
mod behaviour_upkeep;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/compare.rs"]
mod compare;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/hermetic.rs"]
mod hermetic;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/inbox.rs"]
mod inbox;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/issues.rs"]
mod issues;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/live_gitlab.rs"]
mod live_gitlab;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/pictures.rs"]
mod pictures;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/pull_suggestions.rs"]
mod pull_suggestions;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/pull_writes.rs"]
mod pull_writes;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/pulls.rs"]
mod pulls;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/repos.rs"]
mod repos;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/runner.rs"]
mod runner;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/signed.rs"]
mod signed;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/stage.rs"]
mod stage;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/updates.rs"]
mod updates;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/views.rs"]
mod views;
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
#[path = "offscreen/workspaces.rs"]
mod workspaces;

#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn main() -> std::process::ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    // On macOS the pictures are drawn by Metal and the text by Core Text,
    // not as stored (on Linux, by wgpu and cosmic-text): asked for, they
    // are drawn and compared there too, to see how far off they are.
    #[cfg(target_os = "macos")]
    if hermetic::inside().is_none() && std::env::var_os(stage::ON_MACOS).is_none() {
        println!(
            "offscreen: skipped: the stored pictures and these tests are Linux's; set {} to \
             run them on macOS (Metal, Core Text)",
            stage::ON_MACOS
        );
        return std::process::ExitCode::SUCCESS;
    }
    // On Windows the application draws with DirectX and DirectWrite, which
    // the toolkit cannot do off screen; asked for, the views are drawn by
    // its Linux renderer (wgpu, on DirectX 12 or Vulkan here) and text
    // system there, to be looked at.
    #[cfg(windows)]
    if hermetic::inside().is_none() && std::env::var_os(stage::ON_WINDOWS).is_none() {
        println!(
            "offscreen: skipped: the stored pictures and these tests are Linux's; set {} to \
             run them on Windows (wgpu, cosmic-text)",
            stage::ON_WINDOWS
        );
        return std::process::ExitCode::SUCCESS;
    }
    match hermetic::inside() {
        // Started by cargo: the same again, where nothing of the
        // developer's can be reached.
        None => hermetic::run_again(&arguments),
        Some(_) => {
            // One list per area of the window, so that work on two areas
            // at once adds its tests in two files.
            let tests: Vec<runner::Test> = [
                behaviour::TESTS,
                behaviour_shell::TESTS,
                behaviour_settings::TESTS,
                behaviour_graph::TESTS,
                behaviour_refs::TESTS,
                behaviour_commit::TESTS,
                behaviour_files::TESTS,
                behaviour_fit::TESTS,
                behaviour_terminal::TESTS,
                behaviour_terminal_text::TESTS,
                behaviour_terminal_completion::TESTS,
                behaviour_terminal_shells::TESTS,
                behaviour_drag::TESTS,
                behaviour_providers::TESTS,
                behaviour_upkeep::TESTS,
                behaviour_undo::TESTS,
                pulls::TESTS,
                pull_writes::TESTS,
                pull_suggestions::TESTS,
                live_gitlab::TESTS,
                issues::TESTS,
                workspaces::TESTS,
                inbox::TESTS,
                behaviour_access::TESTS,
                behaviour_busy::TESTS,
                updates::TESTS,
            ]
            .concat();
            runner::run(&arguments, &tests, views::VIEWS)
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn main() {
    println!(
        "offscreen: skipped: the renderer and the text system these tests draw with are the \
         toolkit's Linux ones"
    );
}
