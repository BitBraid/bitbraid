//! The templates of a pull request's description, read from a working
//! tree without asking anyone: GitHub's `pull_request_template.md` in the
//! root, `docs/` or `.github/`, and the Markdown files of a
//! `PULL_REQUEST_TEMPLATE/` folder in any of those; GitLab's Markdown
//! files in `.gitlab/merge_request_templates/`. Names are matched
//! whatever their case.
//!
//! A working tree is the user's, but what is in it may have come from
//! anyone: a link is not followed, a file is read up to
//! [`crate::model::MAX_BODY`], and at most [`MOST`] templates are read.
//!
//! Sources: docs.github.com "Creating a pull request template for your
//! repository" (where templates may be, the subfolder for several);
//! docs.gitlab.com "Description templates" (merge request templates in
//! `.gitlab/merge_request_templates/`).

use std::{
    fs,
    io::Read as _,
    path::{Path, PathBuf},
};

use crate::model::{MAX_BODY, PullTemplate};

/// Most templates read.
pub const MOST: usize = 50;
/// Most entries of a folder looked at.
const MOST_ENTRIES: usize = 1000;

/// The entries of `dir` that are not links: name and path, sorted by
/// name; nothing when it cannot be read.
fn entries(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<(String, PathBuf)> = read
        .take(MOST_ENTRIES)
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| !kind.is_symlink()))
        .filter_map(|entry| Some((entry.file_name().into_string().ok()?, entry.path())))
        .collect();
    out.sort();
    out
}

/// The entry of `dir` called `name`, whatever its case.
fn named(dir: &Path, name: &str) -> Option<PathBuf> {
    entries(dir)
        .into_iter()
        .find(|(found, _)| found.eq_ignore_ascii_case(name))
        .map(|(_, path)| path)
}

/// Whether a name ends in `.md`, whatever its case.
fn markdown(name: &str) -> bool {
    name.len() > 3
        && name
            .get(name.len().saturating_sub(3)..)
            .is_some_and(|end| end.eq_ignore_ascii_case(".md"))
}

/// A template's file, read up to [`MAX_BODY`] bytes.
fn template(root: &Path, path: &Path) -> Option<PullTemplate> {
    if !fs::symlink_metadata(path).ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .ok()?
        .take(u64::try_from(MAX_BODY).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .ok()?;
    let relative = path.strip_prefix(root).ok()?;
    let shown: Vec<String> = relative
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    let file = shown.last()?.clone();
    let name = file
        .get(..file.len().saturating_sub(3))
        .unwrap_or(&file)
        .to_owned();
    Some(PullTemplate {
        path: crate::api::clean_line(&shown.join("/"), 4096),
        name: crate::api::clean_line(&name, 256),
        body: crate::api::body(&String::from_utf8_lossy(&bytes)),
    })
}

/// The Markdown files of a folder, as templates.
fn folder(root: &Path, dir: &Path) -> Vec<PullTemplate> {
    if !fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) {
        return Vec::new();
    }
    entries(dir)
        .into_iter()
        .filter(|(name, _)| markdown(name))
        .filter_map(|(_, path)| template(root, &path))
        .collect()
}

/// The templates of the working tree at `root`, GitHub's then GitLab's.
pub fn read(root: &Path) -> Vec<PullTemplate> {
    let mut out = Vec::new();
    let places: Vec<PathBuf> = std::iter::once(root.to_path_buf())
        .chain(named(root, "docs"))
        .chain(named(root, ".github"))
        .filter(|place| fs::symlink_metadata(place).is_ok_and(|meta| meta.is_dir()))
        .collect();
    for place in &places {
        if let Some(path) = named(place, "pull_request_template.md") {
            out.extend(template(root, &path));
        }
    }
    for place in &places {
        if let Some(dir) = named(place, "pull_request_template") {
            out.extend(folder(root, &dir));
        }
    }
    if let Some(dir) = named(root, ".gitlab").and_then(|dir| named(&dir, "merge_request_templates"))
    {
        out.extend(folder(root, &dir));
    }
    out.truncate(MOST);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("bitbraid-templates-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(root: &Path, path: &str, text: &str) {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    #[test]
    fn templates_are_found_where_the_providers_look() {
        let root = scratch("found");
        write(&root, "PULL_REQUEST_TEMPLATE.md", "root one");
        write(&root, "docs/pull_request_template.md", "docs one");
        write(&root, ".github/Pull_Request_Template/bug.md", "a bug");
        write(
            &root,
            ".github/Pull_Request_Template/feature.MD",
            "a feature",
        );
        write(&root, ".github/Pull_Request_Template/notes.txt", "not one");
        write(
            &root,
            ".gitlab/merge_request_templates/Default.md",
            "gitlab's",
        );
        write(&root, "src/pull_request_template.md", "not here");
        let found = read(&root);
        let paths: Vec<&str> = found.iter().map(|found| found.path.as_str()).collect();
        assert_eq!(
            paths,
            [
                "PULL_REQUEST_TEMPLATE.md",
                "docs/pull_request_template.md",
                ".github/Pull_Request_Template/bug.md",
                ".github/Pull_Request_Template/feature.MD",
                ".gitlab/merge_request_templates/Default.md",
            ]
        );
        assert_eq!(found[2].name, "bug");
        assert_eq!(found[2].body, "a bug");
        assert_eq!(found[4].name, "Default");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn a_hostile_tree_is_read_within_bounds() {
        let root = scratch("hostile");
        write(
            &root,
            ".github/pull_request_template.md",
            &"x".repeat(MAX_BODY + 10),
        );
        for n in 0..(MOST + 10) {
            write(
                &root,
                &format!(".gitlab/merge_request_templates/t{n:03}.md"),
                "\u{0}control\u{7}",
            );
        }
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc/passwd", root.join("docs-link")).unwrap();
            fs::create_dir_all(root.join("docs")).unwrap();
            std::os::unix::fs::symlink("/etc/passwd", root.join("docs/pull_request_template.md"))
                .unwrap();
        }
        let found = read(&root);
        assert_eq!(found.len(), MOST);
        assert_eq!(found[0].path, ".github/pull_request_template.md");
        assert_eq!(found[0].body.len(), MAX_BODY);
        assert_eq!(found[1].body, "control");
        assert!(found.iter().all(|found| !found.body.contains("root:")));
        let _ = fs::remove_dir_all(&root);
        assert!(read(Path::new("/nonexistent/at/all")).is_empty());
    }
}
