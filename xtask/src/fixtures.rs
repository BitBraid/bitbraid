//! Build and verify fixture repositories.

use std::fs;

use anyhow::{Context, Result};

pub fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--clean") {
        let dir = bitbraid_testkit::fixtures_out_dir();
        if dir.exists() {
            fs::remove_dir_all(&dir).with_context(|| format!("removing {}", dir.display()))?;
            println!("fixtures: removed {}", dir.display());
        }
    }
    let names: Vec<String> = args
        .iter()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .collect();
    let built = if names.is_empty() {
        bitbraid_testkit::build_all()?
    } else {
        let mut built = Vec::new();
        for name in names {
            let path = bitbraid_testkit::build_fixture(&name)?;
            built.push((name, path));
        }
        built
    };
    println!(
        "fixtures: git = {}",
        bitbraid_testkit::oracle::git_version()?
    );
    for (name, path) in &built {
        println!("  {name:<20} {}", path.display());
    }
    println!("fixtures: ok ({})", built.len());
    Ok(())
}
