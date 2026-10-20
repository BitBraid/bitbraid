//! Layer check: every workspace crate declares `[package.metadata.bitbraid] layer = N`
//! (or `"tool"`), and may only depend on workspace crates with a strictly lower
//! layer. Dev-dependencies are exempt, so any crate may use the testkit.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind, MetadataCommand, Package};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layer {
    Level(u64),
    Tool,
}

impl std::fmt::Display for Layer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Layer::Level(n) => write!(f, "L{n}"),
            Layer::Tool => write!(f, "tool"),
        }
    }
}

fn layer_of(pkg: &Package) -> Result<Layer> {
    let value = pkg.metadata.get("bitbraid").and_then(|m| m.get("layer"));
    match value {
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(Layer::Level)
            .with_context(|| format!("{}: layer must be a non-negative integer", pkg.name)),
        Some(serde_json::Value::String(s)) if s == "tool" => Ok(Layer::Tool),
        Some(other) => bail!("{}: unsupported layer value {other}", pkg.name),
        None => bail!(
            "{}: missing `[package.metadata.bitbraid] layer = <n>` in {}",
            pkg.name,
            pkg.manifest_path
        ),
    }
}

pub fn run() -> Result<()> {
    let metadata = MetadataCommand::new()
        .no_deps()
        .exec()
        .context("running cargo metadata")?;
    let members: BTreeMap<String, &Package> = metadata
        .workspace_packages()
        .into_iter()
        .map(|pkg| (pkg.name.to_string(), pkg))
        .collect();

    let mut layers: BTreeMap<String, Layer> = BTreeMap::new();
    for (name, pkg) in &members {
        layers.insert(name.clone(), layer_of(pkg)?);
    }

    let mut violations = Vec::new();
    for (name, pkg) in &members {
        let Some(Layer::Level(own)) = layers.get(name).copied() else {
            continue; // tools may depend on anything
        };
        for dep in &pkg.dependencies {
            if dep.kind == DependencyKind::Development || dep.path.is_none() {
                continue;
            }
            let Some(target) = layers.get(dep.name.as_str()) else {
                continue;
            };
            match target {
                Layer::Tool => violations.push(format!(
                    "{name} (L{own}) depends on tool crate {}",
                    dep.name
                )),
                Layer::Level(theirs) if *theirs >= own => violations.push(format!(
                    "{name} (L{own}) depends on {} (L{theirs}); dependencies must be strictly lower",
                    dep.name
                )),
                Layer::Level(_) => {}
            }
        }
    }

    let mut by_layer: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    for (name, layer) in &layers {
        let key = match layer {
            Layer::Level(n) => format!("L{n}"),
            Layer::Tool => "tool".to_owned(),
        };
        by_layer.entry(key).or_default().push(name);
    }
    for (layer, names) in &by_layer {
        println!("{layer:>5}  {}", names.join(", "));
    }

    if violations.is_empty() {
        println!("layers: ok ({} crates)", layers.len());
        Ok(())
    } else {
        for v in &violations {
            eprintln!("layer violation: {v}");
        }
        bail!("{} layer violation(s)", violations.len())
    }
}
