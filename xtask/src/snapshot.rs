//! `cargo xtask snapshot [--check] [name...]`: the views of the
//! application, drawn without a display and held against the pictures
//! stored in `crates/ui/tests/snapshots`.
//!
//! Without `--check` every view is drawn and each picture that differs
//! from the stored one is stored anew (and every view is written in full
//! size to `snapshots/` in the target directory, to be looked at). With
//! it the pictures are compared: one that differs fails, named by view
//! and theme, with the picture drawn now and a picture of where the two
//! differ beside it in the target directory. Names narrow it down to the
//! sets of views whose name contains one of them.
//!
//! The work is done by the test program `tests/offscreen.rs` of the
//! interface crate, which is where the application is linked in; this
//! only starts it. Where nothing can draw (no graphics adapter that can
//! be used) it says so and counts as passed: there is nothing to compare.

use std::{process::Command, time::Instant};

use anyhow::{Context, Result, bail};

pub fn run(args: &[String]) -> Result<()> {
    let check = args.iter().any(|arg| arg == "--check");
    if let Some(unknown) = args
        .iter()
        .find(|arg| arg.starts_with('-') && arg.as_str() != "--check")
    {
        bail!("snapshot: {unknown}: the only option is --check");
    }
    let mode = if check {
        "--snapshots=check"
    } else {
        "--snapshots=update"
    };
    let names = args.iter().filter(|arg| !arg.starts_with('-'));
    let started = Instant::now();
    let status = Command::new("cargo")
        .args([
            "test",
            "--quiet",
            "--package",
            "bitbraid-ui",
            "--test",
            "offscreen",
            "--",
            mode,
        ])
        .args(names)
        .current_dir(crate::root())
        .status()
        .context("spawning cargo test for the off-screen views")?;
    println!(
        "snapshot: {:.1} s in all, the build included",
        started.elapsed().as_secs_f64()
    );
    if !status.success() {
        if check {
            bail!(
                "the views are not as their stored pictures have them (see above); \
                 `cargo xtask snapshot` stores the pictures anew"
            );
        }
        bail!("the views could not all be drawn (see above)");
    }
    Ok(())
}
