//! Files and folders picked up from a list to be dropped somewhere else by
//! their full paths: the rows of the changes lists and of a commit's files
//! are dragged as this, and the terminal pastes what is dropped on it,
//! quoted for its shell.
//!
//! A row that is one of several selected rows drags all of them; any
//! other row drags itself; a folder row drags its folder. What follows the
//! pointer is a chip in the style of the other drags (the file's name, or
//! how many there are), which names every path to assistive technology.
//!
//! Sources: the toolkit's sources for dragging
//! (`InteractiveElement::on_drag`, `on_drop`, `ExternalPaths`).

use std::path::{Path, PathBuf};

use gpui_kit::component::ActiveTheme as _;
use gpui_kit::*;

/// Files and folders being dragged, by their full paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraggedPaths {
    /// The paths, in the order of the list they came from, each once.
    pub paths: Vec<PathBuf>,
    /// What the chip that follows the pointer says.
    pub label: SharedString,
}

impl DraggedPaths {
    /// These paths, each once, with the label they call for: the name of
    /// the one, or how many there are.
    pub fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut unique: Vec<PathBuf> = Vec::new();
        for path in paths {
            if !unique.contains(&path) {
                unique.push(path);
            }
        }
        let label = match unique.as_slice() {
            [one] => one
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| one.display().to_string()),
            more => say!("views-dragged-files", count = more.len()),
        };
        Self {
            paths: unique,
            label: label.into(),
        }
    }

    /// Paths relative to a worktree (as git lists them, with `/`), made
    /// full. None when there is no worktree to make them full with, or no
    /// path.
    pub fn in_worktree<'a>(
        worktree: Option<&Path>,
        relative: impl IntoIterator<Item = &'a str>,
    ) -> Option<Self> {
        let worktree = worktree?;
        let paths: Vec<PathBuf> = relative
            .into_iter()
            .filter(|path| !path.is_empty())
            .map(|path| worktree.join(path))
            .collect();
        (!paths.is_empty()).then(|| Self::new(paths))
    }
}

/// The chip that follows the pointer while paths are dragged: the
/// constructor `on_drag` takes.
pub fn chip(
    dragged: &DraggedPaths,
    _: Point<Pixels>,
    _: &mut Window,
    cx: &mut App,
) -> Entity<PathsChip> {
    let dragged = dragged.clone();
    cx.new(|_| PathsChip(dragged))
}

/// What [`chip`] draws.
#[derive(Debug)]
pub struct PathsChip(DraggedPaths);

impl Render for PathsChip {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let every: String = self
            .0
            .paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join("\n");
        div()
            .id("dragged-paths")
            .px_2()
            .py_0p5()
            .rounded(theme.radius)
            .bg(theme.primary)
            .text_color(theme.primary_foreground)
            .text_xs()
            .aria_label(self.0.label.clone())
            .aria_value(every)
            .child(self.0.label.clone())
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use std::path::{Path, PathBuf};

    use super::DraggedPaths;

    #[test]
    fn labels_and_full_paths() {
        let one = DraggedPaths::in_worktree(Some(Path::new("/work/repo")), ["src/main.rs"]);
        let one = one.unwrap();
        assert_eq!(one.paths, [PathBuf::from("/work/repo/src/main.rs")]);
        assert_eq!(one.label, "main.rs");
        let folder = DraggedPaths::in_worktree(Some(Path::new("/work/repo")), ["src"]).unwrap();
        assert_eq!(folder.label, "src");
        let many = DraggedPaths::in_worktree(Some(Path::new("/w")), ["a", "b/c", "a", ""]).unwrap();
        assert_eq!(many.paths, [PathBuf::from("/w/a"), PathBuf::from("/w/b/c")]);
        assert_eq!(many.label, "2 files");
        assert_eq!(DraggedPaths::in_worktree(None, ["a"]), None);
        assert_eq!(DraggedPaths::in_worktree(Some(Path::new("/w")), [""]), None);
        assert_eq!(DraggedPaths::new([PathBuf::from("/")]).label, "/");
    }
}
