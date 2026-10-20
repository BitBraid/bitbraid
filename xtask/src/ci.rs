//! `cargo xtask ci`: the full local gate, identical to what CI runs.

use std::process::Command;

use anyhow::{Context, Result, bail};

fn cargo(args: &[&str]) -> Result<()> {
    let rendered = format!("cargo {}", args.join(" "));
    println!("\n==> {rendered}");
    let status = Command::new("cargo")
        .args(args)
        .current_dir(crate::root())
        .status()
        .with_context(|| format!("spawning {rendered}"))?;
    if !status.success() {
        bail!("{rendered} failed with {status}");
    }
    Ok(())
}

fn have_cargo_deny() -> bool {
    Command::new("cargo")
        .args(["deny", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

pub fn run(args: &[String]) -> Result<()> {
    if let Some(unknown) = args.first() {
        bail!("ci: {unknown}: it takes no options");
    }
    cargo(&["fmt", "--all", "--check"])?;
    cargo(&["clippy", "--all-targets", "--", "-D", "warnings"])?;
    // Every test program, so that one run says all that fails.
    cargo(&["test", "--no-fail-fast"])?;

    // The views against their stored pictures. The test program is built
    // by the step before; where nothing can draw it says so and passes.
    println!("\n==> xtask snapshot --check");
    super::snapshot::run(&["--check".to_owned()])?;

    println!("\n==> xtask layers");
    super::layers::run()?;
    println!("\n==> xtask fixtures");
    super::fixtures::run(&[])?;

    if have_cargo_deny() {
        cargo(&["deny", "check"])?;
    } else if std::env::var_os("CI").is_some() {
        bail!("cargo-deny is required in CI; install it with `cargo install cargo-deny`");
    } else {
        println!("\n==> cargo deny: skipped (not installed; `cargo install cargo-deny`)");
    }

    println!("\nci: all checks passed");
    Ok(())
}
