//! Changed files as a list of paths or a tree of folders, the same for the
//! lists of uncommitted work and for the files of a commit: in which order
//! they come, which folders a tree has, and what a folder that is closed
//! says of what is in it. Nothing here draws.
//!
//! Sources: none (original design).

use std::{cmp::Ordering, collections::BTreeSet};

/// What happened to a file, as a folder counts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Changed in place.
    Modified,
    /// New.
    Added,
    /// Gone.
    Deleted,
    /// Moved, or copied.
    Renamed,
}

impl Kind {
    /// Every kind, in the order a folder lists its counts.
    pub const ALL: [Kind; 4] = [Kind::Modified, Kind::Added, Kind::Deleted, Kind::Renamed];

    fn slot(self) -> usize {
        match self {
            Kind::Modified => 0,
            Kind::Added => 1,
            Kind::Deleted => 2,
            Kind::Renamed => 3,
        }
    }
}

/// How many files of each kind, in the order of [`Kind::ALL`].
pub type Counts = [usize; 4];

/// The number of one kind in the counts.
pub fn count_of(counts: &Counts, kind: Kind) -> usize {
    counts.get(kind.slot()).copied().unwrap_or(0)
}

/// One row of a tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Node {
    /// A folder: its whole path, its own name, how deep it is, whether it
    /// is open, and how many files of each kind are in it at any depth.
    Folder {
        path: String,
        name: String,
        depth: usize,
        open: bool,
        counts: Counts,
    },
    /// A file, by its place among the items given, and how deep it is.
    File { item: usize, depth: usize },
}

/// Two paths in the order of the lists: by path, without regard to case
/// (a folder in small letters before a file in capitals that follows it
/// in the alphabet), and by their bytes where only case tells them apart.
pub fn compare(a: &str, b: &str) -> Ordering {
    a.to_lowercase()
        .cmp(&b.to_lowercase())
        .then_with(|| a.cmp(b))
}

/// The places of `paths` in the order they are listed: A to Z, or Z to A.
pub fn order(paths: &[&str], reversed: bool) -> Vec<usize> {
    let mut places: Vec<usize> = (0..paths.len()).collect();
    places.sort_by(|a, b| {
        let by_path = compare(
            paths.get(*a).copied().unwrap_or_default(),
            paths.get(*b).copied().unwrap_or_default(),
        );
        if reversed { by_path.reverse() } else { by_path }
    });
    places
}

/// Every folder that holds one of the paths, at any depth.
pub fn folders<'a>(paths: impl IntoIterator<Item = &'a str>) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for path in paths {
        let mut end = 0;
        while let Some(found) = path.get(end..).and_then(|rest| rest.find('/')) {
            end = end.saturating_add(found);
            if let Some(folder) = path.get(..end) {
                out.insert(folder.to_owned());
            }
            end = end.saturating_add(1);
        }
    }
    out
}

/// The folder and the ones under it that open with it: while a folder
/// holds nothing but one folder, that one too.
pub fn opens_with<'a>(
    folder: &str,
    paths: impl IntoIterator<Item = &'a str> + Clone,
) -> Vec<String> {
    let mut out = vec![folder.to_owned()];
    let mut at = folder.to_owned();
    loop {
        let prefix = format!("{at}/");
        let mut inside = paths
            .clone()
            .into_iter()
            .filter_map(|path| path.strip_prefix(prefix.as_str()));
        let Some(first) = inside.next() else { break };
        // What is right under it: a file, or the first part of a folder.
        let next = |rest: &str| rest.split_once('/').map(|(name, _)| name.to_owned());
        let Some(only) = next(first) else { break };
        if !inside.all(|rest| next(rest).as_deref() == Some(only.as_str())) {
            break;
        }
        at = format!("{at}/{only}");
        out.push(at.clone());
    }
    out
}

/// The tree of `items` (each its place, its path and its kind), which are
/// in the order they are to be listed: at each depth the folders first,
/// each once, in the order their first file comes, then the files. What is
/// in a folder is under it while it is open.
pub fn tree(items: &[(usize, &str, Kind)], open: &dyn Fn(&str) -> bool) -> Vec<Node> {
    let parts: Vec<(usize, Vec<&str>, Kind)> = items
        .iter()
        .map(|(item, path, kind)| (*item, path.split('/').collect(), *kind))
        .collect();
    let mut out = Vec::new();
    level(&parts, "", 0, open, &mut out);
    out
}

fn level(
    items: &[(usize, Vec<&str>, Kind)],
    prefix: &str,
    depth: usize,
    open: &dyn Fn(&str) -> bool,
    out: &mut Vec<Node>,
) {
    let mut names: Vec<&str> = Vec::new();
    for (_, parts, _) in items {
        if parts.len() > 1
            && let Some(first) = parts.first()
            && !names.contains(first)
        {
            names.push(first);
        }
    }
    for name in names {
        let inside: Vec<(usize, Vec<&str>, Kind)> = items
            .iter()
            .filter(|(_, parts, _)| parts.len() > 1 && parts.first() == Some(&name))
            .map(|(item, parts, kind)| (*item, parts.iter().skip(1).copied().collect(), *kind))
            .collect();
        let path = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let mut counts: Counts = [0; 4];
        for (_, _, kind) in &inside {
            if let Some(slot) = counts.get_mut(kind.slot()) {
                *slot = slot.saturating_add(1);
            }
        }
        let is_open = open(&path);
        out.push(Node::Folder {
            path: path.clone(),
            name: name.to_owned(),
            depth,
            open: is_open,
            counts,
        });
        if is_open {
            level(&inside, &path, depth.saturating_add(1), open, out);
        }
    }
    for (item, parts, _) in items {
        if parts.len() == 1 {
            out.push(Node::File { item: *item, depth });
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{Kind, Node, compare, folders, opens_with, order, tree};

    #[test]
    fn paths_are_listed_without_regard_to_case_either_way() {
        let paths = [
            "src/graph.rs",
            "README.md",
            "docs/logo.png",
            "untracked.txt",
            "readme.md",
        ];
        let listed = |reversed| -> Vec<&str> {
            order(&paths, reversed)
                .into_iter()
                .filter_map(|ix| paths.get(ix).copied())
                .collect()
        };
        assert_eq!(
            listed(false),
            [
                "docs/logo.png",
                "README.md",
                "readme.md",
                "src/graph.rs",
                "untracked.txt"
            ]
        );
        assert_eq!(
            listed(true),
            [
                "untracked.txt",
                "src/graph.rs",
                "readme.md",
                "README.md",
                "docs/logo.png"
            ]
        );
        assert_eq!(compare("a", "a"), std::cmp::Ordering::Equal);
        assert!(order(&[], false).is_empty());
    }

    #[test]
    fn a_tree_lists_its_folders_first_and_counts_what_is_in_a_closed_one() {
        let items = [
            (0, "docs/logo.png", Kind::Modified),
            (1, "README.md", Kind::Modified),
            (2, "src/a/one.rs", Kind::Added),
            (3, "src/a/two.rs", Kind::Deleted),
            (4, "src/b.rs", Kind::Renamed),
        ];
        let open: BTreeSet<String> = ["src".to_owned()].into();
        let rows = tree(&items, &|path| open.contains(path));
        let folder = |path: &str, depth, open, counts| Node::Folder {
            path: path.to_owned(),
            name: path.rsplit('/').next().unwrap_or(path).to_owned(),
            depth,
            open,
            counts,
        };
        assert_eq!(
            rows,
            [
                folder("docs", 0, false, [1, 0, 0, 0]),
                folder("src", 0, true, [0, 1, 1, 1]),
                folder("src/a", 1, false, [0, 1, 1, 0]),
                Node::File { item: 4, depth: 1 },
                Node::File { item: 1, depth: 0 },
            ]
        );
        // Everything open: every file has its row.
        let all = folders(items.iter().map(|(_, path, _)| *path));
        assert_eq!(
            all,
            ["docs", "src", "src/a"]
                .map(str::to_owned)
                .into_iter()
                .collect()
        );
        let rows = tree(&items, &|path| all.contains(path));
        let files = rows
            .iter()
            .filter(|row| matches!(row, Node::File { .. }))
            .count();
        assert_eq!(files, 5);
    }

    #[test]
    fn a_folder_that_holds_only_a_folder_opens_with_it() {
        let paths = ["a/b/c/deep.md", "a/b/c/other.md", "x/y.rs", "x/z/w.rs"];
        assert_eq!(opens_with("a", paths), ["a", "a/b", "a/b/c"]);
        assert_eq!(opens_with("a/b", paths), ["a/b", "a/b/c"]);
        // A folder with a file and a folder in it opens by itself.
        assert_eq!(opens_with("x", paths), ["x"]);
        assert_eq!(opens_with("nowhere", paths), ["nowhere"]);
        // Odd paths do not loop or break.
        assert_eq!(opens_with("a", ["a//b", "a/"]), ["a"]);
        assert_eq!(
            folders(["", "/", "a//b", "a/"]),
            ["", "a", "a/"].map(str::to_owned).into_iter().collect()
        );
    }
}
