//! The `.gitignore` templates a new repository can start from.
//!
//! The templates are files of our own under `assets/gitignore/`, built
//! into the binary. A new repository's `.gitignore` is the chosen ones
//! one after the other, each under a line that names it.
//!
//! Sources: gitignore(5) for the pattern syntax; the patterns themselves
//! are our own.

use serde::Serialize;

use crate::{Error, Result};

/// One template.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Template {
    /// What it is asked for by.
    pub name: &'static str,
    /// What it is for, as people say it.
    pub title: bitbraid_strings::Message,
    /// Its lines.
    #[serde(skip)]
    pub text: &'static str,
}

macro_rules! template {
    ($name:literal, $title:expr) => {
        Template {
            name: $name,
            title: $title,
            text: include_str!(concat!("../../../assets/gitignore/", $name, ".gitignore")),
        }
    };
}

/// Every template there is, in the order they are offered.
pub static TEMPLATES: &[Template] = &[
    template!("rust", message!("ignore-template-rust")),
    template!("node", message!("ignore-template-node")),
    template!("python", message!("ignore-template-python")),
    template!("go", message!("ignore-template-go")),
    template!("java", message!("ignore-template-java")),
    template!("c-cpp", message!("ignore-template-c-cpp")),
    template!("dotnet", message!("ignore-template-dotnet")),
    template!(
        "editors-and-systems",
        message!("ignore-template-editors-and-systems")
    ),
];

/// The template of a name.
pub fn template(name: &str) -> Option<&'static Template> {
    TEMPLATES.iter().find(|template| template.name == name)
}

/// The `.gitignore` made of the templates named, in the order named, each
/// once. A name there is no template for is refused; no names give no
/// file (none).
pub fn compose(names: &[String]) -> Result<Option<String>> {
    let mut chosen: Vec<&Template> = Vec::new();
    for name in names {
        let found = template(name.trim()).ok_or_else(|| Error::Invalid {
            what: say!("ignore-what-template"),
            detail: say!(
                "ignore-no-template",
                name = format!("{:?}", name.chars().take(40).collect::<String>()),
                names = TEMPLATES
                    .iter()
                    .map(|template| template.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        })?;
        if !chosen.iter().any(|template| template.name == found.name) {
            chosen.push(found);
        }
    }
    if chosen.is_empty() {
        return Ok(None);
    }
    let mut out = String::new();
    for (at, template) in chosen.iter().enumerate() {
        if at > 0 {
            out.push('\n');
        }
        out.push_str(template.text.trim_end());
        out.push('\n');
    }
    Ok(Some(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_template_is_patterns_under_a_line_that_says_what_for() {
        let mut names = std::collections::BTreeSet::new();
        for template in TEMPLATES {
            assert!(names.insert(template.name), "{} twice", template.name);
            assert!(
                template
                    .name
                    .chars()
                    .all(|c| c.is_ascii_lowercase() || c == '-'),
                "{}",
                template.name
            );
            assert!(template.text.starts_with("# "), "{}", template.name);
            let patterns = template
                .text
                .lines()
                .filter(|line| !line.trim().is_empty() && !line.starts_with('#'))
                .count();
            assert!(patterns >= 3, "{} has {patterns} patterns", template.name);
            // No line that git would read as something else by accident.
            for line in template.text.lines() {
                assert_eq!(line, line.trim_end(), "{}: {line:?}", template.name);
                assert!(!line.contains('\r') && !line.contains('\t'), "{line:?}");
            }
        }
        assert!(template("rust").is_some() && template("cobol").is_none());
    }

    #[test]
    fn a_file_is_composed_of_the_templates_named() {
        let names =
            |names: &[&str]| -> Vec<String> { names.iter().map(|n| (*n).to_owned()).collect() };
        assert_eq!(compose(&[]).unwrap(), None);
        let rust = compose(&names(&["rust"])).unwrap().unwrap();
        assert_eq!(rust, template("rust").unwrap().text);
        let both = compose(&names(&["python", " rust ", "python"]))
            .unwrap()
            .unwrap();
        assert!(both.starts_with("# Python"));
        assert!(both.contains("\n\n# Rust"), "{both}");
        assert_eq!(
            both.matches("# Python").count(),
            1,
            "named twice, there once"
        );
        assert!(both.ends_with("*.pdb\n"));
        for hostile in ["cobol", "", "../rust", "ru\nst", "RUST", &"x".repeat(5000)] {
            let err = compose(&names(&["rust", hostile])).unwrap_err().to_string();
            assert!(err.contains("there is no template"), "{hostile:?}: {err}");
            assert!(err.len() < 400, "{}", err.len());
        }
    }
}
