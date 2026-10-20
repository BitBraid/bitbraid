//! The rows of the references panel, as they follow from a snapshot: which
//! sections there are and in which order, the tree of folders in each, the
//! rows a filter keeps and how they are ranked, and where the cursor goes.
//! Nothing here draws; the view in `refs.rs` does.
//!
//! What costs nothing: a section that is closed or hidden gets its count
//! and no rows; a closed folder gets one row and a count, and the names
//! under it are passed over with one comparison each.
//!
//! The filter is fuzzy, by the matcher the palette uses too
//! (`crate::fuzzy`, which says how names are ranked). While a filter is
//! set a section keeps its tree: what matches stays in its folders (all of
//! them open) and under its remote, in its order, and each row knows which
//! of its letters matched; with very many matches the best of them have
//! rows.
//!
//! Sources: none (original design).

use std::{
    collections::{BTreeSet, HashMap},
    ops::Range,
};

use bitbraid_git::{Oid, Snapshot};

pub(super) use crate::fuzzy::Needle;
use crate::fuzzy::{Rank, marks, marks_within, rank};

use crate::views::{
    flow::FlowState,
    worktrees::{self, WorktreeRow},
};

/// The sections there can be, in the order they start out in: what each is
/// known by, and its heading.
pub(super) const SECTIONS: [(&str, bitbraid_strings::Message); 9] = [
    ("gitflow", message!("refs-section-gitflow")),
    ("local", message!("refs-section-local")),
    ("remote", message!("refs-section-remote")),
    ("worktrees", message!("refs-section-worktrees")),
    ("stashes", message!("refs-section-stashes")),
    // Its rows are drawn by `views::pulls`; shown while a remote is linked
    // to an account, or could be.
    ("pulls", message!("refs-section-pulls")),
    // Its rows are drawn by `views::issues`; shown as the pull requests
    // are.
    ("issues", message!("refs-section-issues")),
    ("tags", message!("refs-section-tags")),
    ("submodules", message!("refs-section-submodules")),
];

/// A number for a section, to tell its elements from another's.
pub(super) fn section_number(key: &str) -> usize {
    SECTIONS
        .iter()
        .position(|(known, _)| *known == key)
        .unwrap_or(usize::MAX)
}

/// The key of a section as the one of [`SECTIONS`], when it is one.
pub(super) fn known_section(key: &str) -> Option<&'static str> {
    SECTIONS
        .iter()
        .find(|(known, _)| *known == key)
        .map(|(known, _)| *known)
}

/// What a row is, for the mark in front of its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    Branch,
    Folder,
    Remote,
    Tag,
    Stash,
    Worktree,
    Submodule,
}

/// What a row is besides a reference.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(super) enum Place {
    /// A reference, a folder or a remote.
    #[default]
    None,
    /// A worktree: where it is and what can be done with it.
    Worktree { path: String, row: WorktreeRow },
    /// A submodule, by its path in the repository.
    Submodule { path: String },
}

/// A row that holds others and can be closed: a folder of branches or
/// tags, or the row of a remote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Folder {
    /// What it is known by among the closed ones: `section:path`.
    pub key: String,
    pub open: bool,
    /// How many references a closed one holds.
    pub held: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Item {
    pub label: String,
    /// Full reference name; empty for a folder and for a remote.
    pub full_name: String,
    pub target: Oid,
    pub indent: usize,
    pub is_head: bool,
    pub hint: Option<String>,
    pub mark: Mark,
    /// For the row that stands for a remote: its name.
    pub remote: Option<String>,
    /// For a worktree or a submodule: which one.
    pub place: Place,
    /// For a folder and for a remote: how it is known and whether it is
    /// open.
    pub folder: Option<Folder>,
    /// For a row a filter found: the folders it is in, which the flat list
    /// does not show as rows.
    pub path: Option<String>,
    /// For a row a filter found: the letters of the label that matched,
    /// and those of the folders it is in.
    pub marks: Vec<Range<usize>>,
    pub path_marks: Vec<Range<usize>>,
}

/// What a row is known by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum RowId {
    /// A folder or a remote, by its key.
    Folder(String),
    /// A reference, a worktree, a stash or a submodule, by full name.
    Named(String),
}

impl Item {
    pub(super) fn id(&self) -> RowId {
        match &self.folder {
            Some(folder) => RowId::Folder(folder.key.clone()),
            None => RowId::Named(self.full_name.clone()),
        }
    }
}

/// One section of the panel as it is built from a snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SectionRows {
    /// What the section is known by (see [`SECTIONS`]).
    pub key: &'static str,
    pub title: &'static str,
    /// How many references it has, or how many of them match the filter
    /// (folders and remotes are not counted).
    pub count: usize,
    /// How many it has, whatever the filter.
    pub total: usize,
    /// Its rows; none when they were not asked for. Of what a filter
    /// finds, the best [`MOST_MATCHES`].
    pub items: Vec<Item>,
}

/// As many of the matches of a filter get a row in a section, the best
/// ones: a filter that still matches thousands is not read to its end,
/// and is typed on. The heading says how many there are.
pub(super) const MOST_MATCHES: usize = 500;

fn plain(label: String, full_name: String, target: Oid, mark: Mark) -> Item {
    Item {
        label,
        full_name,
        target,
        indent: 0,
        is_head: false,
        hint: None,
        mark,
        remote: None,
        place: Place::None,
        folder: None,
        path: None,
        marks: Vec::new(),
        path_marks: Vec::new(),
    }
}

/// The key a folder is remembered under.
pub(super) fn folder_key(section: &str, path: &str) -> String {
    format!("{section}:{path}")
}

/// Where a tree of names stands: the section, what stands in front of
/// every path in the folders' keys (the name of the remote, with its
/// slash), where the top level is indented to, and the folders that are
/// closed.
struct Tree<'a> {
    section: &'a str,
    scope: &'a str,
    indent: usize,
    closed: &'a BTreeSet<String>,
}

/// Rows for names that arrive sorted: slashes become folders, each said
/// once, and what is under a closed folder is counted and left out.
fn tree_rows<T>(
    tree: &Tree<'_>,
    leaves: &[T],
    name_of: impl Fn(&T) -> &str,
    item_of: impl Fn(&T) -> Item,
    out: &mut Vec<Item>,
) {
    let Tree {
        section,
        scope,
        indent,
        closed,
    } = *tree;
    // The folders the rows are inside of at the moment.
    let mut inside: Vec<&str> = Vec::new();
    let mut at = 0usize;
    while let Some(leaf) = leaves.get(at) {
        let name = name_of(leaf);
        let parts: Vec<&str> = name.split('/').collect();
        let folders = parts.len().saturating_sub(1);
        let mut common = 0usize;
        while common < folders && inside.get(common) == parts.get(common) {
            common = common.saturating_add(1);
        }
        inside.truncate(common);
        // Where in the name the folder at each depth ends.
        let mut end: usize = parts
            .iter()
            .take(common)
            .map(|part| part.len().saturating_add(1))
            .sum();
        let mut passed = None;
        for (depth, part) in parts.iter().enumerate().take(folders).skip(common) {
            end = end.saturating_add(part.len());
            let path = name.get(..end).unwrap_or(name);
            let key = folder_key(section, &format!("{scope}{path}"));
            let open = !closed.contains(&key);
            let mut held = 0usize;
            if !open {
                // Everything under it stands together, since the names are
                // sorted: count it and go on behind it.
                let under = name.get(..=end).unwrap_or(name);
                let mut next = at;
                while leaves
                    .get(next)
                    .is_some_and(|leaf| name_of(leaf).starts_with(under))
                {
                    held = held.saturating_add(1);
                    next = next.saturating_add(1);
                }
                passed = Some(next.max(at.saturating_add(1)));
            }
            out.push(Item {
                indent: indent.saturating_add(depth),
                folder: Some(Folder { key, open, held }),
                ..plain((*part).to_owned(), String::new(), Oid::zero(), Mark::Folder)
            });
            if !open {
                break;
            }
            inside.push(part);
            end = end.saturating_add(1);
        }
        if let Some(next) = passed {
            at = next;
            continue;
        }
        out.push(Item {
            label: parts.last().copied().unwrap_or(name).to_owned(),
            indent: indent.saturating_add(folders),
            ..item_of(leaf)
        });
        at = at.saturating_add(1);
    }
}

/// The names of a section that match, best first: for each its place in
/// `names`.
fn ranked<'a>(needle: &Needle, names: impl Iterator<Item = &'a str>) -> Vec<usize> {
    let mut buffer = Vec::new();
    let mut matches: Vec<(Rank, &str, usize)> = names
        .enumerate()
        .filter_map(|(at, name)| Some((rank(needle, name, &mut buffer)?, name, at)))
        .collect();
    matches.sort_unstable();
    matches.into_iter().map(|(_, _, at)| at).collect()
}

/// What decides which rows are built.
pub(super) struct Shape<'a> {
    /// What was typed into the filter.
    pub needle: &'a Needle,
    /// Gitflow, once it is known how it is set up.
    pub flow: Option<&'a FlowState>,
    /// The folders that are closed, by key.
    pub closed: &'a BTreeSet<String>,
    /// Whether the rows of a section are wanted: of one that is closed or
    /// hidden only the count is.
    pub wanted: &'a dyn Fn(&str) -> bool,
}

/// One section: counted, and with its rows when they are wanted.
fn section<T>(
    shape: &Shape<'_>,
    key: &'static str,
    leaves: &[T],
    name_of: impl Fn(&T) -> &str,
    rows: impl FnOnce(&mut Vec<Item>),
    out: &mut Vec<SectionRows>,
) {
    let title = SECTIONS
        .iter()
        .find(|(known, _)| *known == key)
        .map_or("", |(_, title)| title.text());
    let wanted = (shape.wanted)(key);
    let mut items = Vec::new();
    let count = if shape.needle.is_empty() {
        if wanted {
            rows(&mut items);
        }
        leaves.len()
    } else {
        // Filtered: the rows that match, in their folders and in their
        // order (the folders open); the best [`MOST_MATCHES`] of them.
        let order = ranked(shape.needle, leaves.iter().map(&name_of));
        if wanted && !order.is_empty() {
            let found: BTreeSet<&str> = order
                .iter()
                .take(MOST_MATCHES)
                .filter_map(|at| leaves.get(*at).map(&name_of))
                .collect();
            rows(&mut items);
            items = found_rows(items, shape.needle, &found);
        }
        order.len()
    };
    out.push(SectionRows {
        key,
        title,
        count,
        total: leaves.len(),
        items,
    });
}

/// What a row is matched by while filtering: a reference by its name
/// without what all of its kind begin with (a remote's branch with its
/// remote's name in front), anything else by what its row says.
fn match_name(item: &Item) -> &str {
    ["refs/heads/", "refs/remotes/", "refs/tags/"]
        .iter()
        .find_map(|space| item.full_name.strip_prefix(space))
        .unwrap_or(&item.label)
}

/// Of the rows of a section, those a filter found (`found`, by the names
/// they are matched by) and the folders and remotes they are in, in their
/// order; the letters that matched stand out in their names.
fn found_rows(rows: Vec<Item>, needle: &Needle, found: &BTreeSet<&str>) -> Vec<Item> {
    let leaf_found = |item: &Item| item.folder.is_none() && found.contains(match_name(item));
    // A folder stays when something under it does: the rows after it that
    // are further in.
    let keep: Vec<bool> = rows
        .iter()
        .enumerate()
        .map(|(at, item)| match &item.folder {
            None => leaf_found(item),
            Some(_) => rows
                .iter()
                .skip(at.saturating_add(1))
                .take_while(|under| under.indent > item.indent)
                .any(&leaf_found),
        })
        .collect();
    rows.into_iter()
        .zip(keep)
        .filter(|(_, keep)| *keep)
        .map(|(item, _)| {
            let marks = if item.folder.is_some() {
                marks(needle, &item.label)
            } else {
                // The letters of the whole name that fall in the row's own
                // part of it.
                let name = match_name(&item);
                let all = marks(needle, name);
                let from = name.len().saturating_sub(item.label.len());
                marks_within(&all, from..name.len())
            };
            Item { marks, ..item }
        })
        .collect()
}

/// The sections of the panel for a snapshot. With a filter, every section
/// says how much it found, and its rows are what was found, in their
/// folders and in their order.
pub(super) fn build_sections(snapshot: &Snapshot, shape: &Shape<'_>) -> Vec<SectionRows> {
    let mut sections: Vec<SectionRows> = Vec::new();
    // While filtering every folder is open, so that what is found shows.
    let none_closed = BTreeSet::new();
    let closed = if shape.needle.is_empty() {
        shape.closed
    } else {
        &none_closed
    };

    let local = |b: &bitbraid_git::snapshot::Branch| Item {
        is_head: b.is_head,
        hint: b.upstream.clone(),
        ..plain(b.name.clone(), b.full_name.clone(), b.target, Mark::Branch)
    };
    section(
        shape,
        "local",
        &snapshot.branches,
        |b| b.name.as_str(),
        |rows| {
            tree_rows(
                &Tree {
                    section: "local",
                    scope: "",
                    indent: 0,
                    closed,
                },
                &snapshot.branches,
                |b| b.name.as_str(),
                local,
                rows,
            )
        },
        &mut sections,
    );

    // Every remote is a group of its own inside the one section; its
    // branches are matched with the remote's name in front.
    let filtering = !shape.needle.is_empty();
    let remote_count: usize = snapshot.remotes.iter().map(|r| r.branches.len()).sum();
    let remote_branch = |b: &bitbraid_git::snapshot::RemoteBranch| {
        plain(b.name.clone(), b.full_name.clone(), b.target, Mark::Branch)
    };
    let shown: Vec<String> = if filtering {
        snapshot
            .remotes
            .iter()
            .flat_map(|remote| {
                remote
                    .branches
                    .iter()
                    .map(move |branch| format!("{}/{}", remote.name, branch.name))
            })
            .collect()
    } else {
        Vec::new()
    };
    let order = if filtering {
        ranked(shape.needle, shown.iter().map(String::as_str))
    } else {
        Vec::new()
    };
    let mut items = Vec::new();
    if (shape.wanted)("remote") && (!filtering || !order.is_empty()) {
        for remote in &snapshot.remotes {
            let key = folder_key("remote", &remote.name);
            let open = !closed.contains(&key);
            items.push(Item {
                remote: Some(remote.name.clone()),
                hint: remote.fetch_url.clone(),
                folder: Some(Folder {
                    key,
                    open,
                    held: remote.branches.len(),
                }),
                ..plain(
                    remote.name.clone(),
                    String::new(),
                    Oid::zero(),
                    Mark::Remote,
                )
            });
            if open {
                tree_rows(
                    &Tree {
                        section: "remote",
                        scope: &format!("{}/", remote.name),
                        indent: 1,
                        closed,
                    },
                    &remote.branches,
                    |b| b.name.as_str(),
                    remote_branch,
                    &mut items,
                );
            }
        }
        if filtering {
            let found: BTreeSet<&str> = order
                .iter()
                .take(MOST_MATCHES)
                .filter_map(|at| shown.get(*at).map(String::as_str))
                .collect();
            items = found_rows(items, shape.needle, &found);
        }
    }
    sections.push(SectionRows {
        key: "remote",
        title: say!("refs-section-remote"),
        count: if filtering { order.len() } else { remote_count },
        total: remote_count,
        items,
    });

    let worktree = |w: &bitbraid_git::snapshot::Worktree| {
        let path = w
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let head = w.head.or(snapshot.head.target).unwrap_or_else(Oid::zero);
        Item {
            // The one this tab shows stands out like the checked-out
            // branch does.
            is_head: w.current,
            hint: Some(worktrees::folder_name(&path)).filter(|name| !name.is_empty()),
            place: Place::Worktree {
                row: WorktreeRow {
                    is_main: w.is_main,
                    current: w.current,
                    locked: w.locked,
                    on_branch: w.branch.is_some(),
                },
                path,
            },
            ..plain(
                worktree_label(w),
                format!("worktree:{}", w.id),
                head,
                Mark::Worktree,
            )
        }
    };
    let worktree_names: Vec<String> = snapshot.worktrees.iter().map(worktree_label).collect();
    flat_section(
        shape,
        "worktrees",
        &snapshot.worktrees,
        &worktree_names,
        worktree,
        &mut sections,
    );

    // A stash by what it says, and the branch it was made on beside it.
    let stash_names: Vec<String> = snapshot
        .stashes
        .iter()
        .map(|s| stash_parts(&s.message).0.to_owned())
        .collect();
    flat_section(
        shape,
        "stashes",
        &snapshot.stashes,
        &stash_names,
        |s| {
            let (said, branch) = stash_parts(&s.message);
            Item {
                hint: branch.map(|branch| say!("refs-stash-on", branch)),
                ..plain(
                    said.to_owned(),
                    format!("stash@{{{}}}", s.index),
                    s.target,
                    Mark::Stash,
                )
            }
        },
        &mut sections,
    );

    let tag = |t: &bitbraid_git::snapshot::Tag| {
        plain(
            t.name.clone(),
            format!("refs/tags/{}", t.name),
            t.target,
            Mark::Tag,
        )
    };
    // Names that are no version first, in order, then the versions from
    // the highest down.
    let tags_in_order = tags_ordered(&snapshot.tags);
    section(
        shape,
        "tags",
        &tags_in_order,
        |t| t.name.as_str(),
        |rows| {
            tree_rows(
                &Tree {
                    section: "tags",
                    scope: "",
                    indent: 0,
                    closed,
                },
                &tags_in_order,
                |t| t.name.as_str(),
                tag,
                rows,
            )
        },
        &mut sections,
    );

    let submodule_names: Vec<String> = snapshot.submodules.iter().map(|s| s.path.clone()).collect();
    flat_section(
        shape,
        "submodules",
        &snapshot.submodules,
        &submodule_names,
        |s| Item {
            hint: s.url.clone(),
            place: Place::Submodule {
                path: s.path.clone(),
            },
            ..plain(
                s.path.clone(),
                format!("submodule:{}", s.name),
                s.head.or(s.index).unwrap_or_else(Oid::zero),
                Mark::Submodule,
            )
        },
        &mut sections,
    );

    // Gitflow's branches, once it is known how it is set up: its two
    // branches and those that begin with one of its prefixes, as branches,
    // in their folders. No section at all where it is not set up.
    if let Some(flow) = shape.flow.filter(|flow| flow.initialized) {
        let prefixes: Vec<&str> = flow
            .prefixes
            .iter()
            .filter(|(kind, prefix)| kind.as_str() != "tag" && !prefix.is_empty())
            .map(|(_, prefix)| prefix.as_str())
            .collect();
        let of_flow: Vec<bitbraid_git::snapshot::Branch> = snapshot
            .branches
            .iter()
            .filter(|b| {
                Some(&b.name) == flow.main.as_ref()
                    || Some(&b.name) == flow.develop.as_ref()
                    || prefixes.iter().any(|prefix| b.name.starts_with(prefix))
            })
            .cloned()
            .collect();
        section(
            shape,
            "gitflow",
            &of_flow,
            |b| b.name.as_str(),
            |rows| {
                tree_rows(
                    &Tree {
                        section: "gitflow",
                        scope: "",
                        indent: 0,
                        closed,
                    },
                    &of_flow,
                    |b| b.name.as_str(),
                    local,
                    rows,
                )
            },
            &mut sections,
        );
    }
    sections
}

/// A branch by its name, a detached worktree by its commit.
fn worktree_label(w: &bitbraid_git::snapshot::Worktree) -> String {
    w.branch
        .clone()
        .or_else(|| w.head.map(|id| id.short(7)))
        .unwrap_or_else(|| w.id.clone())
}

/// A section without folders: its rows in the order they come in, or,
/// filtered, the ones whose name matches.
fn flat_section<T>(
    shape: &Shape<'_>,
    key: &'static str,
    leaves: &[T],
    names: &[String],
    item_of: impl Fn(&T) -> Item,
    out: &mut Vec<SectionRows>,
) {
    let named: Vec<(&str, &T)> = names.iter().map(String::as_str).zip(leaves).collect();
    let before = out.len();
    section(
        shape,
        key,
        &named,
        |(name, _)| name,
        |rows| rows.extend(leaves.iter().map(&item_of)),
        out,
    );
    // What is found here keeps the name it has: a path in the name of a
    // worktree, a stash or a submodule is no folder.
    for section in out.iter_mut().skip(before) {
        for item in &mut section.items {
            if let Some(path) = item.path.take() {
                // The marks of the name behind those of what stands in
                // front of it now.
                let moved = path.len().saturating_add(1);
                let mut marks = std::mem::take(&mut item.path_marks);
                marks.extend(
                    item.marks.iter().map(|mark| {
                        mark.start.saturating_add(moved)..mark.end.saturating_add(moved)
                    }),
                );
                item.marks = marks;
                item.label = format!("{path}/{}", item.label);
            }
        }
    }
}

/// The sections in the order that is remembered: what the list names and
/// is known, in that order, then the rest as they start out. The list
/// comes from a file anyone can edit.
pub(super) fn ordered(remembered: &[String]) -> Vec<&'static str> {
    let mut order: Vec<&'static str> = Vec::new();
    for key in remembered {
        if let Some(known) = known_section(key)
            && !order.contains(&known)
        {
            order.push(known);
        }
    }
    for (key, _) in SECTIONS {
        if !order.contains(&key) {
            order.push(key);
        }
    }
    order
}

/// `order` with one section put in front of another, or at the end.
pub(super) fn moved_before(
    order: &[&'static str],
    moved: &str,
    before: Option<&str>,
) -> Vec<&'static str> {
    let Some(moved) = known_section(moved) else {
        return order.to_vec();
    };
    if before == Some(moved) {
        return order.to_vec();
    }
    let mut out: Vec<&'static str> = order.iter().copied().filter(|key| *key != moved).collect();
    let at = before
        .and_then(|before| out.iter().position(|key| *key == before))
        .unwrap_or(out.len());
    out.insert(at.min(out.len()), moved);
    out
}

/// `order` with one section moved one place up or down among those that
/// are shown; unchanged at the end it is moved towards.
pub(super) fn nudged(
    order: &[&'static str],
    moved: &str,
    down: bool,
    shown: impl Fn(&str) -> bool,
) -> Vec<&'static str> {
    let visible: Vec<&'static str> = order.iter().copied().filter(|key| shown(key)).collect();
    let Some(at) = visible.iter().position(|key| *key == moved) else {
        return order.to_vec();
    };
    if down {
        // In front of what follows the next one, or at the end.
        match visible.get(at.saturating_add(1)) {
            Some(_) => moved_before(order, moved, visible.get(at.saturating_add(2)).copied()),
            None => order.to_vec(),
        }
    } else {
        match at.checked_sub(1).and_then(|above| visible.get(above)) {
            Some(above) => moved_before(order, moved, Some(above)),
            None => order.to_vec(),
        }
    }
}

/// What a whole remote is hidden or shown alone by: the start of the
/// names of its branches.
pub(crate) fn remote_ref(remote: &str) -> String {
    format!("refs/remotes/{remote}")
}

/// Whether a reference is a branch of a remote that is named in `names`
/// as a whole.
/// A name that ends in a slash stands for a folder of branches or tags
/// that is hidden or shown alone as a whole ([`folder_ref`]).
pub(crate) fn under_remote(names: &[String], full_name: &str) -> bool {
    names.iter().any(|name| {
        if name.ends_with('/') {
            name.len() > 1 && full_name.starts_with(name.as_str())
        } else {
            name.starts_with("refs/remotes/")
                && full_name
                    .strip_prefix(name.as_str())
                    .is_some_and(|rest| rest.starts_with('/'))
        }
    })
}

/// What the references in a folder start with, by the folder's key
/// (`section:path`), as hiding a folder as a whole puts it among the
/// hidden ones: `refs/heads/feature/` for the folder `feature` of the
/// branches. None for a folder of a section that is no references'.
pub(super) fn folder_ref(key: &str) -> Option<String> {
    let (section, path) = key.split_once(':')?;
    let space = match section {
        "local" => "refs/heads",
        "remote" => "refs/remotes",
        "tags" => "refs/tags",
        _ => return None,
    };
    (!path.is_empty()).then(|| format!("{space}/{path}/"))
}

/// For the sections of branches, remotes and tags: how many of their
/// references the graph shows, and how many there are, while that is not
/// all of them (something is hidden there, or anything is shown alone). Empty
/// while the graph shows everything.
pub(super) fn sections_seen(
    snapshot: &Snapshot,
    hidden: &[String],
    solo: &[String],
) -> HashMap<&'static str, (usize, usize)> {
    let among = |list: &[String], name: &str| {
        list.iter().any(|known| known == name) || under_remote(list, name)
    };
    let soloing = !solo.is_empty();
    let mut seen = HashMap::new();
    let mut count = |key: &'static str, names: Vec<String>| {
        let all = names.len();
        let shown = names
            .iter()
            .filter(|name| {
                if soloing {
                    among(solo, name)
                } else {
                    !among(hidden, name)
                }
            })
            .count();
        if soloing || shown < all {
            seen.insert(key, (shown, all));
        }
    };
    count(
        "local",
        snapshot
            .branches
            .iter()
            .map(|b| b.full_name.clone())
            .collect(),
    );
    count(
        "remote",
        snapshot
            .remotes
            .iter()
            .flat_map(|remote| remote.branches.iter())
            .map(|b| b.full_name.clone())
            .collect(),
    );
    count(
        "tags",
        snapshot
            .tags
            .iter()
            .map(|tag| format!("refs/tags/{}", tag.name))
            .collect(),
    );
    seen
}

/// What a stash says, and the branch it was made on: git writes "On
/// main: what was said", or "WIP on main: a commit" for one without a
/// message of its own.
pub(super) fn stash_parts(message: &str) -> (&str, Option<&str>) {
    let rest = message
        .strip_prefix("On ")
        .or_else(|| message.strip_prefix("WIP on "));
    match rest.and_then(|rest| rest.split_once(": ")) {
        Some((branch, said)) if !branch.is_empty() && !branch.contains(' ') => (said, Some(branch)),
        _ => (message, None),
    }
}

/// A tag's name as a version: its numbers, when it is one ("v1.2",
/// "2.0.1"); none for any other name.
fn version_of(name: &str) -> Option<Vec<u64>> {
    let digits = name.strip_prefix('v').unwrap_or(name);
    if digits.is_empty() {
        return None;
    }
    digits.split('.').map(|part| part.parse().ok()).collect()
}

/// The tags in the order the panel lists them: names that are no version
/// first, by name (so that those in a folder stand together), then the
/// versions, the highest first.
pub(super) fn tags_ordered(
    tags: &[bitbraid_git::snapshot::Tag],
) -> Vec<bitbraid_git::snapshot::Tag> {
    let (mut versions, mut others): (Vec<_>, Vec<_>) = tags
        .iter()
        .cloned()
        .partition(|tag| version_of(&tag.name).is_some());
    others.sort_by(|a, b| a.name.cmp(&b.name));
    versions.sort_by(|a, b| {
        version_of(&b.name)
            .cmp(&version_of(&a.name))
            .then_with(|| a.name.cmp(&b.name))
    });
    others.extend(versions);
    others
}

/// How many references there are that the graph can show: branches,
/// branches of remotes and tags.
pub(super) fn reference_count(snapshot: &Snapshot) -> usize {
    let remote: usize = snapshot
        .remotes
        .iter()
        .map(|remote| remote.branches.len())
        .sum();
    snapshot
        .branches
        .len()
        .saturating_add(remote)
        .saturating_add(snapshot.tags.len())
}

/// How many references the graph starts from, and whether that is fewer
/// than there are: the ones shown alone when any are, else all but the
/// hidden. A remote that is hidden or shown alone as a whole counts with
/// every branch.
pub(super) fn viewing(snapshot: &Snapshot, hidden: &[String], solo: &[String]) -> (usize, bool) {
    let names = snapshot
        .branches
        .iter()
        .map(|branch| branch.full_name.clone())
        .chain(
            snapshot
                .remotes
                .iter()
                .flat_map(|remote| remote.branches.iter())
                .map(|branch| branch.full_name.clone()),
        )
        .chain(
            snapshot
                .tags
                .iter()
                .map(|tag| format!("refs/tags/{}", tag.name)),
        );
    let among = |list: &[String], name: &String| list.contains(name) || under_remote(list, name);
    let (mut total, mut soloed, mut hid) = (0usize, 0usize, 0usize);
    for name in names {
        total = total.saturating_add(1);
        if among(solo, &name) {
            soloed = soloed.saturating_add(1);
        }
        if among(hidden, &name) {
            hid = hid.saturating_add(1);
        }
    }
    if !solo.is_empty() && soloed > 0 {
        return (soloed, true);
    }
    (total.saturating_sub(hid), hid > 0)
}

/// The base of the checked-out branch: the branch it was made from and goes
/// back to, by full name. Nothing records that in git, so it is found by
/// what is set up:
///
/// 1. with gitflow, a feature's base is the develop branch and a
///    release's or hotfix's the main one;
/// 2. else the default branch of the remote the branch follows, or of the
///    first remote that names one (`refs/remotes/<remote>/HEAD`), as the
///    local branch of that name when there is one;
/// 3. else the first of `main`, `master`, `develop` and `trunk` there is.
///
/// A branch that is its own base by these rules (the default branch) has
/// none, and neither has a detached HEAD.
pub(super) fn base_branch(snapshot: &Snapshot, flow: Option<&FlowState>) -> Option<String> {
    let head = snapshot.branches.iter().find(|branch| branch.is_head)?;
    let local = |name: &str| {
        snapshot
            .branches
            .iter()
            .find(|branch| branch.name == name)
            .map(|branch| branch.full_name.clone())
    };
    let on = |remote: &bitbraid_git::snapshot::Remote, name: &str| {
        remote
            .branches
            .iter()
            .find(|branch| branch.name == name)
            .map(|branch| branch.full_name.clone())
    };
    let not_itself = |found: String| (found != head.full_name).then_some(found);
    if let Some(flow) = flow.filter(|flow| flow.initialized)
        && let Some(own) = flow.branches.iter().find(|own| own.branch == head.name)
    {
        let parent = if matches!(own.kind, "feature" | "bugfix") {
            flow.develop.as_deref()
        } else {
            flow.main.as_deref()
        };
        if let Some(found) = parent.and_then(local) {
            return not_itself(found);
        }
    }
    // The remote it follows first, then the others as they are listed.
    let followed = snapshot
        .remotes
        .iter()
        .filter(|remote| head.remote.as_deref() == Some(remote.name.as_str()));
    let others = snapshot
        .remotes
        .iter()
        .filter(|remote| head.remote.as_deref() != Some(remote.name.as_str()));
    for remote in followed.chain(others) {
        if let Some(default) = &remote.head
            && let Some(found) = local(default).or_else(|| on(remote, default))
        {
            return not_itself(found);
        }
    }
    for usual in ["main", "master", "develop", "trunk"] {
        let found =
            local(usual).or_else(|| snapshot.remotes.iter().find_map(|remote| on(remote, usual)));
        if let Some(found) = found {
            return not_itself(found);
        }
    }
    None
}

/// What "focused visibility" keeps: the checked-out branch, what it follows
/// and its base, by full name. Nothing for a detached HEAD.
pub(super) fn focused(snapshot: &Snapshot, flow: Option<&FlowState>) -> Vec<String> {
    let Some(head) = snapshot.branches.iter().find(|branch| branch.is_head) else {
        return Vec::new();
    };
    let mut kept = vec![head.full_name.clone()];
    let more = head
        .upstream
        .as_ref()
        .map(|upstream| format!("refs/remotes/{upstream}"))
        .into_iter()
        .chain(base_branch(snapshot, flow));
    for name in more {
        if !kept.contains(&name) {
            kept.push(name);
        }
    }
    kept
}

/// Move a pinned reference one lane to the left or to the right among the
/// pinned ones, whose order is the order of their lanes. Whether it moved:
/// one that is not pinned, or stands at that end already, does not.
pub(super) fn pin_moved(pinned: &mut [String], full_name: &str, right: bool) -> bool {
    let Some(at) = pinned.iter().position(|name| name == full_name) else {
        return false;
    };
    let to = if right {
        at.checked_add(1).filter(|to| *to < pinned.len())
    } else {
        at.checked_sub(1)
    };
    match to {
        Some(to) => {
            pinned.swap(at, to);
            true
        }
        None => false,
    }
}

/// Whether a pinned reference has a lane to its left to move to, and one
/// to its right.
pub(super) fn pin_moves(pinned: &[String], full_name: &str) -> (bool, bool) {
    match pinned.iter().position(|name| name == full_name) {
        Some(at) => (at > 0, at.saturating_add(1) < pinned.len()),
        None => (false, false),
    }
}

/// What a click on a reference does to the selection: alone it selects
/// just that one, with the toggle key it adds or removes it, with the range
/// key it selects everything from the last one clicked to this one, in the
/// order the panel shows them.
pub(super) fn clicked(
    selected: &mut BTreeSet<String>,
    anchor: &mut Option<String>,
    order: &[&str],
    name: &str,
    toggle: bool,
    range: bool,
) {
    let position = |wanted: &str| order.iter().position(|shown| *shown == wanted);
    let span = anchor
        .as_deref()
        .and_then(position)
        .zip(position(name))
        .filter(|_| range);
    match span {
        Some((from, to)) => {
            let (low, high) = (from.min(to), from.max(to));
            if !toggle {
                selected.clear();
            }
            selected.extend(
                order
                    .iter()
                    .skip(low)
                    .take(high.saturating_sub(low).saturating_add(1))
                    .map(|shown| (*shown).to_owned()),
            );
        }
        None if toggle => {
            if !selected.remove(name) {
                selected.insert(name.to_owned());
            }
            *anchor = Some(name.to_owned());
        }
        None => {
            selected.clear();
            selected.insert(name.to_owned());
            *anchor = Some(name.to_owned());
        }
    }
}

/// "2↑ 1↓" for a branch that is two ahead of its upstream and one behind;
/// nothing when it is neither.
pub(super) fn track_text(ahead: u32, behind: u32) -> Option<String> {
    match (ahead, behind) {
        (0, 0) => None,
        (ahead, 0) => Some(format!("{ahead}↑")),
        (0, behind) => Some(format!("{behind}↓")),
        (ahead, behind) => Some(format!("{ahead}↑ {behind}↓")),
    }
}

/// How long ago, in as few letters as say it: "now", "5m", "3h", "12d",
/// "7mo", "2y". Nothing for a time that is not known or lies ahead by more
/// than a clock can be off.
pub(super) fn age_text(then: i64, now: i64) -> Option<String> {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    let ago = now.checked_sub(then)?;
    if ago < -DAY {
        return None;
    }
    let ago = ago.max(0);
    Some(match ago {
        ago if ago < MINUTE => say!("refs-age-now").to_owned(),
        ago if ago < HOUR => say!("refs-age-minutes", count = ago / MINUTE),
        ago if ago < DAY => say!("refs-age-hours", count = ago / HOUR),
        ago if ago < 60 * DAY => say!("refs-age-days", count = ago / DAY),
        ago if ago < 730 * DAY => say!("refs-age-months", count = ago / (30 * DAY)),
        ago => say!("refs-age-years", count = ago / (365 * DAY)),
    })
}

/// What stands at the right of a local branch's row: how far it is ahead
/// of what it follows and behind it ("2↑ 1↓"), then "gone" when what it
/// followed is no more and "stale" when its last commit is older than the
/// preferences allow. Nothing when there is none of that.
pub(super) fn branch_aside(track: Option<(u32, u32)>, gone: bool, stale: bool) -> Option<String> {
    let parts: Vec<String> = [
        track.and_then(|(ahead, behind)| track_text(ahead, behind)),
        gone.then(|| say!("refs-branch-gone").to_owned()),
        stale.then(|| say!("refs-branch-stale").to_owned()),
    ]
    .into_iter()
    .flatten()
    .collect();
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// What the tooltip of a branch's row says, a line each: what the user
/// wrote about it; that merging it into the checked-out branch would
/// conflict; what it follows, and that this is gone; how old its last
/// commit is, and that this makes it stale. The row itself has room for
/// the name, for how far the branch is ahead and behind, and for the
/// short marks of [`branch_aside`]. None when there is nothing to say.
pub(super) fn branch_tip(
    description: Option<&str>,
    conflicts: Option<&str>,
    upstream: Option<&str>,
    gone: bool,
    age: Option<&str>,
    stale: bool,
) -> Option<String> {
    let follows = match (upstream, gone) {
        (Some(upstream), true) => Some(say!("refs-tip-follows-gone", upstream)),
        (Some(upstream), false) => Some(say!("refs-tip-follows", upstream)),
        (None, true) => Some(say!("refs-tip-followed-gone").to_owned()),
        (None, false) => None,
    };
    // What `age_text` says of a commit made less than a minute ago.
    let now = say!("refs-age-now");
    let last = match (age, stale) {
        (Some(age), false) if age == now => Some(say!("refs-tip-last-now").to_owned()),
        (Some(age), false) => Some(say!("refs-tip-last", age)),
        (Some(age), true) if age == now => Some(say!("refs-tip-stale-now").to_owned()),
        (Some(age), true) => Some(say!("refs-tip-stale", age)),
        (None, true) => Some(say!("refs-tip-stale-old").to_owned()),
        (None, false) => None,
    };
    let lines: Vec<String> = [
        description
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned),
        conflicts.map(|mark| say!("refs-tip-conflicts", mark)),
        follows,
        last,
    ]
    .into_iter()
    .flatten()
    .collect();
    (!lines.is_empty()).then(|| lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use bitbraid_git::{Oid, Snapshot};

    use super::{
        Item, Mark, Needle, Place, RowId, SectionRows, Shape, WorktreeRow, age_text, branch_aside,
        branch_tip, build_sections, clicked, folder_key, folder_ref, moved_before, nudged, ordered,
        rank, sections_seen, stash_parts, tags_ordered, track_text, under_remote,
    };

    fn oid(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    fn snapshot() -> Snapshot {
        let mut snapshot = Snapshot::default();
        for (name, is_head) in [
            ("feature/a", false),
            ("feature/b", false),
            ("feature/x/deep", false),
            ("fix/crash", false),
            ("main", true),
        ] {
            snapshot.branches.push(bitbraid_git::snapshot::Branch {
                name: name.to_owned(),
                full_name: format!("refs/heads/{name}"),
                target: oid(1),
                upstream: None,
                remote: None,
                is_head,
            });
        }
        snapshot.remotes.push(bitbraid_git::snapshot::Remote {
            name: "origin".to_owned(),
            fetch_url: None,
            push_url: None,
            head: None,
            branches: (0..3000)
                .map(|n| bitbraid_git::snapshot::RemoteBranch {
                    name: format!("team-{:02}/topic-{n:04}", n % 30),
                    full_name: format!("refs/remotes/origin/team-{:02}/topic-{n:04}", n % 30),
                    target: oid(2),
                })
                .collect(),
        });
        // Sorted, as a snapshot has them.
        for remote in &mut snapshot.remotes {
            remote.branches.sort_by(|a, b| a.name.cmp(&b.name));
        }
        snapshot
    }

    /// A stash is listed by what it says, with the branch it was made on;
    /// tags that are no version first, then the versions from the highest
    /// down.
    #[test]
    fn stashes_say_their_message_and_tags_go_versions_down() {
        assert_eq!(
            stash_parts("On main: readme tweak"),
            ("readme tweak", Some("main"))
        );
        assert_eq!(
            stash_parts("WIP on feature/x: 0123abc a commit"),
            ("0123abc a commit", Some("feature/x"))
        );
        assert_eq!(stash_parts("just words"), ("just words", None));
        assert_eq!(stash_parts("On : odd"), ("On : odd", None));
        assert_eq!(stash_parts("On two words: x"), ("On two words: x", None));

        let tag = |name: &str| bitbraid_git::snapshot::Tag {
            name: name.to_owned(),
            target: oid(1),
            tag_object: None,
            annotated: false,
            message: None,
        };
        let tags: Vec<_> = [
            "nightly",
            "v0.1",
            "v1.0",
            "v1.1",
            "release/x",
            "v10.0",
            "2.0.1",
            "v",
        ]
        .iter()
        .map(|name| tag(name))
        .collect();
        let order: Vec<String> = tags_ordered(&tags).into_iter().map(|t| t.name).collect();
        assert_eq!(
            order,
            [
                "nightly",
                "release/x",
                "v",
                "v10.0",
                "2.0.1",
                "v1.1",
                "v1.0",
                "v0.1"
            ]
        );
        assert!(tags_ordered(&[]).is_empty());
    }

    /// A folder is hidden or shown alone as a whole by what the names in it
    /// begin with, and the sections say how many of theirs are shown.
    #[test]
    fn a_folder_is_hidden_as_a_whole_and_sections_count_what_shows() {
        assert_eq!(
            folder_ref("local:feature").as_deref(),
            Some("refs/heads/feature/")
        );
        assert_eq!(
            folder_ref("remote:origin/team-01").as_deref(),
            Some("refs/remotes/origin/team-01/")
        );
        assert_eq!(
            folder_ref("tags:release").as_deref(),
            Some("refs/tags/release/")
        );
        assert_eq!(folder_ref("worktrees:x"), None);
        assert_eq!(folder_ref("local:"), None);
        assert_eq!(folder_ref("nonsense"), None);

        let folder = vec!["refs/heads/feature/".to_owned()];
        assert!(under_remote(&folder, "refs/heads/feature/a"));
        assert!(under_remote(&folder, "refs/heads/feature/x/deep"));
        assert!(!under_remote(&folder, "refs/heads/feature"));
        assert!(!under_remote(&folder, "refs/heads/features/a"));
        assert!(
            !under_remote(&["/".to_owned()], "refs/heads/main"),
            "no slash alone"
        );

        let snapshot = snapshot();
        assert!(sections_seen(&snapshot, &[], &[]).is_empty(), "all shown");
        let seen = sections_seen(&snapshot, &folder, &[]);
        assert_eq!(
            seen.get("local"),
            Some(&(2, 5)),
            "three of five in the folder"
        );
        assert_eq!(seen.get("remote"), None, "nothing hidden there");
        // While anything is shown alone, every section says how many are.
        let seen = sections_seen(&snapshot, &[], &["refs/heads/main".to_owned()]);
        assert_eq!(seen.get("local"), Some(&(1, 5)));
        assert_eq!(seen.get("remote"), Some(&(0, 3000)));
    }

    fn build(snapshot: &Snapshot, filter: &str, closed: &[&str]) -> Vec<SectionRows> {
        build_but(snapshot, filter, closed, &[])
    }

    fn build_but(
        snapshot: &Snapshot,
        filter: &str,
        closed: &[&str],
        unwanted: &[&str],
    ) -> Vec<SectionRows> {
        let closed: BTreeSet<String> = closed.iter().map(|key| (*key).to_owned()).collect();
        let needle = Needle::new(filter);
        let wanted = |key: &str| !unwanted.contains(&key);
        build_sections(
            snapshot,
            &Shape {
                needle: &needle,
                flow: None,
                closed: &closed,
                wanted: &wanted,
            },
        )
    }

    /// Gitflow's section: its two branches and those that begin with one
    /// of its prefixes, as branches in their folders; none where gitflow
    /// is not set up.
    #[test]
    fn gitflows_section_lists_its_branches_as_a_tree() {
        let snapshot = snapshot();
        let mut flow = crate::views::flow::FlowState {
            initialized: true,
            main: Some("main".to_owned()),
            develop: Some("develop".to_owned()),
            ..Default::default()
        };
        for (kind, prefix) in [("feature", "feature/"), ("hotfix", "hotfix/"), ("tag", "v")] {
            flow.prefixes.insert(kind.to_owned(), prefix.to_owned());
        }
        let needle = Needle::new("");
        let closed = BTreeSet::new();
        let wanted = |_: &str| true;
        let build = |flow: &crate::views::flow::FlowState| {
            build_sections(
                &snapshot,
                &Shape {
                    needle: &needle,
                    flow: Some(flow),
                    closed: &closed,
                    wanted: &wanted,
                },
            )
        };
        let sections = build(&flow);
        let gitflow = sections.iter().find(|s| s.key == "gitflow").unwrap();
        assert_eq!(
            gitflow.count, 4,
            "feature/a, feature/b, feature/x/deep, main"
        );
        assert_eq!(
            labels(&gitflow.items),
            [
                ("feature", 0, true),
                ("a", 1, false),
                ("b", 1, false),
                ("x", 1, true),
                ("deep", 2, false),
                ("main", 0, false),
            ]
        );
        // Its rows are the branches themselves, the checked-out one too.
        let main = gitflow.items.last().unwrap();
        assert_eq!(main.full_name, "refs/heads/main");
        assert!(main.is_head && main.place == Place::None);
        // Not set up: no section.
        flow.initialized = false;
        assert!(!build(&flow).iter().any(|s| s.key == "gitflow"));
    }

    fn rows<'a>(sections: &'a [SectionRows], key: &str) -> &'a [Item] {
        sections
            .iter()
            .find(|section| section.key == key)
            .map(|section| section.items.as_slice())
            .unwrap_or_default()
    }

    fn labels(items: &[Item]) -> Vec<(&str, usize, bool)> {
        items
            .iter()
            .map(|i| (i.label.as_str(), i.indent, i.folder.is_some()))
            .collect()
    }

    fn headings(sections: &[SectionRows]) -> Vec<(&'static str, usize, usize)> {
        sections
            .iter()
            .map(|section| (section.title, section.count, section.items.len()))
            .collect()
    }

    #[test]
    fn slashes_become_folders_said_once() {
        let sections = build(&snapshot(), "", &[]);
        assert_eq!(
            labels(rows(&sections, "local")),
            [
                ("feature", 0, true),
                ("a", 1, false),
                ("b", 1, false),
                ("x", 1, true),
                ("deep", 2, false),
                ("fix", 0, true),
                ("crash", 1, false),
                ("main", 0, false),
            ]
        );
        // The rows know what they are found again by.
        let local = rows(&sections, "local");
        assert_eq!(local[0].id(), RowId::Folder("local:feature".to_owned()));
        assert_eq!(local[3].id(), RowId::Folder("local:feature/x".to_owned()));
        assert_eq!(
            local[4].id(),
            RowId::Named("refs/heads/feature/x/deep".to_owned())
        );
    }

    #[test]
    fn a_closed_folder_is_one_row_that_says_how_much_it_holds() {
        let sections = build(&snapshot(), "", &["local:feature"]);
        let local = rows(&sections, "local");
        assert_eq!(
            labels(local),
            [
                ("feature", 0, true),
                ("fix", 0, true),
                ("crash", 1, false),
                ("main", 0, false),
            ]
        );
        let folder = local[0].folder.as_ref().unwrap();
        assert!(!folder.open);
        assert_eq!(folder.held, 3, "what is under folders inside it too");
        // The section still counts every branch.
        assert_eq!(sections[0].count, 5);

        // A folder inside an open one; a key that names nothing is nothing.
        let sections = build(&snapshot(), "", &["local:feature/x", "local:nothing", "x"]);
        assert_eq!(
            labels(rows(&sections, "local")),
            [
                ("feature", 0, true),
                ("a", 1, false),
                ("b", 1, false),
                ("x", 1, true),
                ("fix", 0, true),
                ("crash", 1, false),
                ("main", 0, false),
            ]
        );
    }

    #[test]
    fn every_remote_is_a_folder_of_its_own_with_folders_inside() {
        let mut snapshot = snapshot();
        snapshot.remotes.push(bitbraid_git::snapshot::Remote {
            name: "upstream".to_owned(),
            fetch_url: None,
            push_url: None,
            head: None,
            branches: vec![bitbraid_git::snapshot::RemoteBranch {
                name: "team/main".to_owned(),
                full_name: "refs/remotes/upstream/team/main".to_owned(),
                target: oid(3),
            }],
        });
        let sections = build(&snapshot, "", &[]);
        let remote = sections.iter().find(|s| s.key == "remote").unwrap();
        // 3000 branches in 30 folders under one remote, one under another.
        assert_eq!(remote.count, 3001);
        assert_eq!(remote.items.len(), 1 + 30 + 3000 + 1 + 1 + 1);
        assert_eq!(remote.items[0].remote.as_deref(), Some("origin"));
        assert_eq!(remote.items[1].label, "team-00");
        assert_eq!(remote.items[1].indent, 1);
        assert_eq!(remote.items[2].indent, 2);
        assert_eq!(
            remote.items[2].full_name,
            "refs/remotes/origin/team-00/topic-0000"
        );
        let tail: Vec<(&str, usize, Mark)> = remote
            .items
            .iter()
            .skip(3031)
            .map(|item| (item.label.as_str(), item.indent, item.mark))
            .collect();
        assert_eq!(
            tail,
            [
                ("upstream", 0, Mark::Remote),
                ("team", 1, Mark::Folder),
                ("main", 2, Mark::Branch),
            ]
        );

        // A closed remote is its row; a closed folder inside one is one
        // row; the same folder name under another remote stays open.
        let sections = build(&snapshot, "", &["remote:upstream", "remote:origin/team-07"]);
        let remote = rows(&sections, "remote");
        assert_eq!(remote.len(), 1 + 30 + 2900 + 1);
        let closed: Vec<(&str, usize)> = remote
            .iter()
            .filter_map(|item| {
                let folder = item.folder.as_ref()?;
                (!folder.open).then_some((item.label.as_str(), folder.held))
            })
            .collect();
        assert_eq!(closed, [("team-07", 100), ("upstream", 1)]);
    }

    #[test]
    fn rows_that_are_not_wanted_are_not_built_and_still_counted() {
        let sections = build_but(&snapshot(), "", &[], &["remote", "local"]);
        assert_eq!(
            headings(&sections),
            [
                ("BRANCHES", 5, 0),
                ("REMOTES", 3000, 0),
                ("WORKTREES", 0, 0),
                ("STASHES", 0, 0),
                ("TAGS", 0, 0),
                ("SUBMODULES", 0, 0),
            ]
        );
        // Filtered too: counted by what matches, every section there.
        let sections = build_but(&snapshot(), "topic-0042", &[], &["remote"]);
        assert_eq!(
            headings(&sections),
            [
                ("BRANCHES", 0, 0),
                ("REMOTES", 1, 0),
                ("WORKTREES", 0, 0),
                ("STASHES", 0, 0),
                ("TAGS", 0, 0),
                ("SUBMODULES", 0, 0),
            ]
        );
    }

    #[test]
    fn worktrees_and_submodules_are_rows_that_know_what_they_are() {
        let mut snapshot = snapshot();
        let worktree = |id: &str, branch: Option<&str>, main: bool, current: bool, locked: bool| {
            bitbraid_git::snapshot::Worktree {
                id: id.to_owned(),
                path: Some(format!("/work/{id}").into()),
                head: Some(oid(3)),
                branch: branch.map(str::to_owned),
                locked,
                lock_reason: None,
                is_main: main,
                current,
            }
        };
        snapshot.worktrees = vec![
            worktree("main", Some("main"), true, false, false),
            worktree("topic", Some("feature/a"), false, true, false),
            worktree("kept", None, false, false, true),
        ];
        snapshot.submodules.push(bitbraid_git::snapshot::Submodule {
            name: "lib".to_owned(),
            path: "deps/lib".to_owned(),
            url: Some("https://example.invalid/lib.git".to_owned()),
            head: None,
            index: Some(oid(4)),
            active: true,
        });
        let sections = build(&snapshot, "", &[]);
        let worktrees = rows(&sections, "worktrees");
        // A branch by its name, a detached one by its commit; the folder
        // beside it; the one that is open marked like a checked-out branch.
        let seen: Vec<(&str, Option<&str>, bool)> = worktrees
            .iter()
            .map(|item| (item.label.as_str(), item.hint.as_deref(), item.is_head))
            .collect();
        assert_eq!(
            seen,
            [
                ("main", Some("main"), false),
                ("feature/a", Some("topic"), true),
                ("0303030", Some("kept"), false),
            ]
        );
        assert_eq!(
            worktrees[2].place,
            Place::Worktree {
                path: "/work/kept".to_owned(),
                row: WorktreeRow {
                    is_main: false,
                    current: false,
                    locked: true,
                    on_branch: false,
                },
            }
        );
        let submodules = rows(&sections, "submodules");
        assert_eq!(submodules.len(), 1);
        assert_eq!(submodules[0].label, "deps/lib");
        assert_eq!(
            submodules[0].place,
            Place::Submodule {
                path: "deps/lib".to_owned()
            }
        );
        // A branch is neither, and a worktree's name is not made a folder.
        assert_eq!(rows(&sections, "local")[1].place, Place::None);
        assert!(worktrees.iter().all(|item| item.folder.is_none()));
        // Filtered, a worktree is found by what its row says.
        snapshot.remotes.clear();
        let sections = build(&snapshot, "0303", &[]);
        let found: Vec<_> = headings(&sections)
            .into_iter()
            .filter(|(_, count, _)| *count > 0)
            .collect();
        assert_eq!(found, [("WORKTREES", 1, 1)]);
        // A slash in such a name makes no folder, filtered or not.
        let sections = build(&snapshot, "feature/a", &[]);
        let found = &rows(&sections, "worktrees")[0];
        assert_eq!(
            (found.label.as_str(), found.path.as_deref()),
            ("feature/a", None)
        );
    }

    #[test]
    fn a_filter_keeps_the_tree_and_every_section() {
        // What is found stays in its folders and under its remote, and
        // the letters that matched stand out.
        let sections = build(&snapshot(), "  Topic-0042 ", &[]);
        let found: Vec<_> = headings(&sections)
            .into_iter()
            .filter(|(_, count, _)| *count > 0)
            .collect();
        assert_eq!(found, [("REMOTES", 1, 3)]);
        let remote = rows(&sections, "remote");
        assert_eq!(
            labels(remote),
            [
                ("origin", 0, true),
                ("team-12", 1, true),
                ("topic-0042", 2, false)
            ]
        );
        assert_eq!(
            remote[2].full_name,
            "refs/remotes/origin/team-12/topic-0042"
        );
        assert_eq!(remote[2].marks, vec![0..10]);
        // Nothing found: every section is there, with nothing in it.
        let none = build(&snapshot(), "nothing like it", &[]);
        assert!(!none.is_empty());
        assert!(
            none.iter()
                .all(|section| section.count == 0 && section.items.is_empty())
        );
        // A closed folder hides nothing from the filter: it is open while
        // filtering, and the folders it holds without a match are left out.
        let sections = build(&snapshot(), "deep", &["local:feature"]);
        let local = rows(&sections, "local");
        assert_eq!(
            labels(local),
            [("feature", 0, true), ("x", 1, true), ("deep", 2, false)]
        );
        // What matches thousands gets rows for the best of them, and the
        // count says how many there are.
        let sections = build(&snapshot(), "topic", &[]);
        let remote = sections.iter().find(|s| s.key == "remote").unwrap();
        let leaves = remote
            .items
            .iter()
            .filter(|item| item.folder.is_none())
            .count();
        assert_eq!((remote.count, leaves), (3000, super::MOST_MATCHES));
        assert_eq!(remote.total, 3000);
    }

    fn order_of(names: &[&str], typed: &str) -> Vec<String> {
        let needle = Needle::new(typed);
        let mut buffer = Vec::new();
        let mut found: Vec<_> = names
            .iter()
            .filter_map(|name| Some((rank(&needle, name, &mut buffer)?, *name)))
            .collect();
        found.sort();
        found.into_iter().map(|(_, name)| name.to_owned()).collect()
    }

    #[test]
    fn matches_are_ranked_exact_word_start_in_a_row_then_letters_in_order() {
        let names = [
            "fix/login-page",
            "login",
            "feature/relogin",
            "feature/login",
            "long-input-name",
            "main",
        ];
        assert_eq!(
            order_of(&names, "login"),
            [
                // Exactly it; at the start of a word, the shorter name
                // first; in a row elsewhere; the letters in order.
                "login",
                "feature/login",
                "fix/login-page",
                "feature/relogin",
                "long-input-name",
            ]
        );
        // Upper and lower case are the same, both ways.
        assert_eq!(order_of(&["Fix/Login"], "fix/login"), ["Fix/Login"]);
        assert_eq!(order_of(&["fix/login"], "FIX"), ["fix/login"]);
        // Fewer letters in between first; then the shorter name.
        assert_eq!(
            order_of(&["a-x-b-x-c", "abxc", "zzabxc", "a-b-c"], "abc"),
            ["abxc", "zzabxc", "a-b-c", "a-x-b-x-c"]
        );
        // Every word has to be there, in any order.
        assert_eq!(
            order_of(
                &["team-01/topic-7", "team-02/topic-7", "team-01/other"],
                "topic team-01"
            ),
            ["team-01/topic-7"]
        );
        // Letters out of order, or more of them than the name has, match
        // nothing; nor does nothing typed.
        assert!(order_of(&["abc"], "cba").is_empty());
        assert!(order_of(&["ab"], "abc").is_empty());
        assert!(rank(&Needle::new("  "), "abc", &mut Vec::new()).is_none());
        // Names in other scripts, and ones that grow in lower case.
        assert_eq!(order_of(&["ÄNDERUNG", "İstanbul"], "änder"), ["ÄNDERUNG"]);
    }

    #[test]
    fn four_thousand_names_are_filtered_in_no_time() {
        let mut snapshot = snapshot();
        for n in 0..1500 {
            snapshot.tags.push(bitbraid_git::snapshot::Tag {
                name: format!("release/v{}.{}.{}", n / 100, n / 10 % 10, n % 10),
                target: oid(5),
                tag_object: None,
                annotated: false,
                message: None,
            });
        }
        let started = std::time::Instant::now();
        let mut found = 0;
        for typed in ["t", "to", "top", "topic-00", "tm07", "v1.2", "zzzz"] {
            found += build(&snapshot, typed, &[])
                .iter()
                .map(|section| section.count)
                .sum::<usize>();
        }
        assert!(found > 3000, "{found}");
        // Seven keystrokes over 4,500 names take a few tens of milliseconds
        // in a debug build; the bound catches a filter slow by a power, not
        // a factor.
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
    }

    #[test]
    fn sections_keep_the_order_that_is_remembered() {
        // Gitflow first (it is shown only where gitflow is set up).
        let all = ordered(&[]);
        assert_eq!(
            all,
            [
                "gitflow",
                "local",
                "remote",
                "worktrees",
                "stashes",
                "pulls",
                "issues",
                "tags",
                "submodules"
            ]
        );
        // What a file names first comes first; what it does not name, or
        // names twice, or makes up, changes nothing else.
        let remembered: Vec<String> = ["tags", "nonsense", "local", "tags", ""]
            .map(str::to_owned)
            .to_vec();
        assert_eq!(
            ordered(&remembered),
            [
                "tags",
                "local",
                "gitflow",
                "remote",
                "worktrees",
                "stashes",
                "pulls",
                "issues",
                "submodules"
            ]
        );
        // Dropped in front of another, or at the end.
        let moved = moved_before(&all, "tags", Some("local"));
        assert_eq!(moved[..4], ["gitflow", "tags", "local", "remote"]);
        let moved = moved_before(&all, "local", None);
        assert_eq!(moved.last(), Some(&"local"));
        assert_eq!(moved_before(&all, "local", Some("local")), all);
        assert_eq!(moved_before(&all, "nonsense", Some("local")), all);
        assert_eq!(
            moved_before(&all, "tags", Some("nonsense")).last(),
            Some(&"tags")
        );
        // One place up or down, among the sections that are shown.
        let shown = |key: &str| key != "remote";
        let down = nudged(&all, "local", true, shown);
        assert_eq!(down[..4], ["gitflow", "remote", "worktrees", "local"]);
        assert_eq!(
            nudged(&down, "local", false, shown)[..4],
            ["gitflow", "remote", "local", "worktrees"]
        );
        assert_eq!(
            nudged(&all, "gitflow", false, shown),
            all,
            "it is the first"
        );
        assert_eq!(
            nudged(&all, "submodules", true, shown),
            all,
            "it is the last"
        );
        assert_eq!(nudged(&all, "remote", true, shown), all, "it is not shown");
    }

    #[test]
    fn how_far_a_branch_is_from_its_upstream_is_said_in_arrows() {
        assert_eq!(track_text(0, 0), None);
        assert_eq!(track_text(3, 0).as_deref(), Some("3↑"));
        assert_eq!(track_text(0, 1).as_deref(), Some("1↓"));
        assert_eq!(track_text(1, 12).as_deref(), Some("1↑ 12↓"));
    }

    /// A stale branch, and one whose upstream is gone, says so on its row
    /// beside how far it is ahead and behind.
    #[test]
    fn a_stale_or_orphaned_branch_says_so_on_its_row() {
        assert_eq!(branch_aside(None, false, false), None);
        assert_eq!(branch_aside(Some((0, 0)), false, false), None);
        assert_eq!(
            branch_aside(Some((2, 1)), false, false).as_deref(),
            Some("2↑ 1↓")
        );
        assert_eq!(branch_aside(None, false, true).as_deref(), Some("stale"));
        assert_eq!(
            branch_aside(Some((0, 0)), true, false).as_deref(),
            Some("gone")
        );
        assert_eq!(
            branch_aside(Some((3, 0)), true, true).as_deref(),
            Some("3↑ · gone · stale")
        );
    }

    #[test]
    fn a_branch_row_says_how_old_and_what_it_follows() {
        const DAY: i64 = 86_400;
        let now = 1_800_000_000;
        let ages: Vec<Option<String>> = [
            10,
            5 * 60,
            3 * 3600,
            12 * DAY,
            59 * DAY,
            70 * DAY,
            400 * DAY,
            800 * DAY,
            -3600,
            -3 * DAY,
        ]
        .into_iter()
        .map(|ago| age_text(now - ago, now))
        .collect();
        let texts: Vec<&str> = ages
            .iter()
            .map(|age| age.as_deref().unwrap_or("-"))
            .collect();
        assert_eq!(
            texts,
            [
                "now", "5m", "3h", "12d", "59d", "2mo", "13mo", "2y", "now", "-"
            ]
        );
        assert_eq!(age_text(i64::MIN, i64::MAX), None, "no overflow");

        // What the row has no room for is said in its tooltip, a line each.
        assert_eq!(
            branch_tip(
                Some(" Quotes and escapes \n"),
                Some("conflicts in 2 files"),
                Some("origin/main"),
                false,
                Some("3d"),
                false
            )
            .as_deref(),
            Some(
                "Quotes and escapes\n\
                 A merge into the checked-out branch: conflicts in 2 files\n\
                 Follows origin/main\n\
                 Last commit 3d ago"
            )
        );
        assert_eq!(
            branch_tip(None, None, Some("origin/gone"), true, None, true).as_deref(),
            Some("Follows origin/gone, which is gone\nStale: its last commit is old")
        );
        // A stale branch's tooltip says how old it is too, when that is
        // known.
        assert_eq!(
            branch_tip(None, None, None, false, Some("17mo"), true).as_deref(),
            Some("Stale: last commit 17mo ago")
        );
        assert_eq!(
            branch_tip(None, None, None, false, Some("now"), false).as_deref(),
            Some("Last commit just now")
        );
        assert_eq!(branch_tip(Some("  "), None, None, false, None, false), None);
    }

    fn branch(name: &str, upstream: Option<&str>, is_head: bool) -> bitbraid_git::snapshot::Branch {
        bitbraid_git::snapshot::Branch {
            name: name.to_owned(),
            full_name: format!("refs/heads/{name}"),
            target: oid(1),
            upstream: upstream.map(str::to_owned),
            remote: upstream
                .and_then(|upstream| upstream.split_once('/'))
                .map(|(remote, _)| remote.to_owned()),
            is_head,
        }
    }

    fn remote(name: &str, head: Option<&str>, branches: &[&str]) -> bitbraid_git::snapshot::Remote {
        bitbraid_git::snapshot::Remote {
            name: name.to_owned(),
            fetch_url: None,
            push_url: None,
            head: head.map(str::to_owned),
            branches: branches
                .iter()
                .map(|branch| bitbraid_git::snapshot::RemoteBranch {
                    name: (*branch).to_owned(),
                    full_name: format!("refs/remotes/{name}/{branch}"),
                    target: oid(2),
                })
                .collect(),
        }
    }

    #[test]
    fn the_base_of_a_branch_is_found_by_what_is_set_up() {
        use super::{base_branch, focused};
        use crate::views::flow::{FlowBranch, FlowState};
        let mut snapshot = Snapshot {
            branches: vec![
                branch("develop", None, false),
                branch("feature/x", Some("origin/feature/x"), true),
                branch("trunk", None, false),
            ],
            remotes: vec![
                remote("fork", Some("next"), &["next"]),
                remote("origin", Some("trunk"), &["feature/x", "trunk"]),
            ],
            ..Snapshot::default()
        };
        // The default branch of the remote it follows, as the local branch
        // of that name; the other remote is not asked.
        assert_eq!(
            base_branch(&snapshot, None).as_deref(),
            Some("refs/heads/trunk")
        );
        assert_eq!(
            focused(&snapshot, None),
            [
                "refs/heads/feature/x",
                "refs/remotes/origin/feature/x",
                "refs/heads/trunk"
            ]
        );
        // Without a local branch of that name, the remote's.
        snapshot.branches.retain(|branch| branch.name != "trunk");
        assert_eq!(
            base_branch(&snapshot, None).as_deref(),
            Some("refs/remotes/origin/trunk")
        );
        // Gitflow says it for its own branches, whatever the remote does.
        let flow = FlowState {
            initialized: true,
            main: Some("trunk".to_owned()),
            develop: Some("develop".to_owned()),
            branches: vec![FlowBranch {
                kind: "feature",
                name: "x".to_owned(),
                branch: "feature/x".to_owned(),
                tip: None,
                current: true,
            }],
            ..FlowState::default()
        };
        assert_eq!(
            base_branch(&snapshot, Some(&flow)).as_deref(),
            Some("refs/heads/develop")
        );
        // A remote that names no default: another remote's, then the
        // usual names.
        snapshot.remotes[1].head = None;
        assert_eq!(
            base_branch(&snapshot, None).as_deref(),
            Some("refs/remotes/fork/next")
        );
        snapshot.remotes[0].head = None;
        assert_eq!(
            base_branch(&snapshot, None).as_deref(),
            Some("refs/heads/develop")
        );
        snapshot.branches.push(branch("main", None, false));
        assert_eq!(
            base_branch(&snapshot, None).as_deref(),
            Some("refs/heads/main")
        );

        // The default branch has no base, and it is not looked for further.
        let mut snapshot = Snapshot {
            branches: vec![
                branch("main", Some("origin/main"), true),
                branch("master", None, false),
            ],
            remotes: vec![remote("origin", Some("main"), &["main"])],
            ..Snapshot::default()
        };
        assert_eq!(base_branch(&snapshot, None), None);
        assert_eq!(
            focused(&snapshot, None),
            ["refs/heads/main", "refs/remotes/origin/main"]
        );
        // Nothing checked out on a branch, nothing to focus on.
        snapshot.branches[0].is_head = false;
        assert_eq!(base_branch(&snapshot, None), None);
        assert!(focused(&snapshot, None).is_empty());
        assert!(focused(&Snapshot::default(), None).is_empty());
    }

    #[test]
    fn a_whole_remote_is_hidden_or_shown_alone_with_every_branch() {
        use super::{remote_ref, under_remote, viewing};
        let mut snapshot = Snapshot {
            branches: vec![branch("main", None, true), branch("topic", None, false)],
            remotes: vec![
                remote("origin", None, &["main", "team/a", "team/b"]),
                remote("origin-fork", None, &["main"]),
            ],
            ..Snapshot::default()
        };
        snapshot.tags.push(bitbraid_git::snapshot::Tag {
            name: "v1".to_owned(),
            target: oid(5),
            tag_object: None,
            annotated: false,
            message: None,
        });
        let names = |list: &[&str]| {
            list.iter()
                .map(|name| (*name).to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(remote_ref("origin"), "refs/remotes/origin");
        // Seven references, all in the graph.
        assert_eq!(viewing(&snapshot, &[], &[]), (7, false));
        // A remote hidden as a whole takes its three branches, and not
        // those of a remote whose name only starts the same.
        let hidden = names(&["refs/remotes/origin", "refs/heads/topic"]);
        assert_eq!(viewing(&snapshot, &hidden, &[]), (3, true));
        assert!(under_remote(&hidden, "refs/remotes/origin/team/a"));
        assert!(!under_remote(&hidden, "refs/remotes/origin-fork/main"));
        assert!(!under_remote(&hidden, "refs/heads/topic/x"));
        // Named twice, once with its remote: counted once.
        let hidden = names(&["refs/remotes/origin", "refs/remotes/origin/main"]);
        assert_eq!(viewing(&snapshot, &hidden, &[]), (4, true));
        // Shown alone: a remote with all its branches, and a tag beside it.
        let solo = names(&["refs/remotes/origin", "refs/tags/v1"]);
        assert_eq!(viewing(&snapshot, &hidden, &solo), (4, true));
        // Showing only what names nothing that exists shows everything.
        let solo = names(&["refs/heads/gone"]);
        assert_eq!(viewing(&snapshot, &[], &solo), (7, false));
    }

    #[test]
    fn folder_keys_tell_sections_and_remotes_apart() {
        assert_eq!(folder_key("local", "feature/x"), "local:feature/x");
        assert_ne!(
            folder_key("remote", "origin/team"),
            folder_key("remote", "upstream/team")
        );
    }

    #[test]
    fn a_pinned_branch_moves_a_lane_at_a_time_and_stops_at_the_ends() {
        use super::{pin_moved, pin_moves};
        let mut pinned: Vec<String> = ["a", "b", "c"].iter().map(|n| (*n).to_owned()).collect();
        assert_eq!(pin_moves(&pinned, "a"), (false, true));
        assert_eq!(pin_moves(&pinned, "b"), (true, true));
        assert_eq!(pin_moves(&pinned, "c"), (true, false));
        assert_eq!(pin_moves(&pinned, "not pinned"), (false, false));
        assert!(pin_moved(&mut pinned, "a", true));
        assert_eq!(pinned, ["b", "a", "c"]);
        assert!(pin_moved(&mut pinned, "a", true));
        assert_eq!(pinned, ["b", "c", "a"]);
        // At the end it stays, and so does what is not pinned.
        assert!(!pin_moved(&mut pinned, "a", true));
        assert!(!pin_moved(&mut pinned, "b", false));
        assert!(!pin_moved(&mut pinned, "not pinned", false));
        assert_eq!(pinned, ["b", "c", "a"]);
        assert!(pin_moved(&mut pinned, "a", false));
        assert_eq!(pinned, ["b", "a", "c"]);
        // One alone has nowhere to go; none at all is nothing.
        let mut one = vec!["only".to_owned()];
        assert_eq!(pin_moves(&one, "only"), (false, false));
        assert!(!pin_moved(&mut one, "only", true) && !pin_moved(&mut one, "only", false));
        assert!(!pin_moved(&mut [], "x", true));
    }

    #[test]
    fn clicks_select_one_toggle_or_take_a_range() {
        let order = ["a", "b", "c", "d", "e"];
        let mut selected = BTreeSet::new();
        let mut anchor = None;
        let names = |selected: &BTreeSet<String>| selected.iter().cloned().collect::<Vec<_>>();

        clicked(&mut selected, &mut anchor, &order, "b", false, false);
        assert_eq!(names(&selected), ["b"]);
        // The range key: from the last one clicked to this one, either way.
        clicked(&mut selected, &mut anchor, &order, "d", false, true);
        assert_eq!(names(&selected), ["b", "c", "d"]);
        clicked(&mut selected, &mut anchor, &order, "a", false, true);
        assert_eq!(names(&selected), ["a", "b"], "the range still starts at b");
        // The toggle key adds and removes single ones and moves the start.
        clicked(&mut selected, &mut anchor, &order, "e", true, false);
        assert_eq!(names(&selected), ["a", "b", "e"]);
        clicked(&mut selected, &mut anchor, &order, "a", true, false);
        assert_eq!(names(&selected), ["b", "e"]);
        // Both keys: the range is added to what is selected.
        clicked(&mut selected, &mut anchor, &order, "c", true, true);
        assert_eq!(names(&selected), ["a", "b", "c", "e"]);
        // A plain click starts over.
        clicked(&mut selected, &mut anchor, &order, "c", false, false);
        assert_eq!(names(&selected), ["c"]);

        // A range with nothing to start from, or from something that is no
        // longer shown, is a plain click; a name that is not shown at all
        // does not bring anything down.
        let mut selected = BTreeSet::new();
        let mut anchor = None;
        clicked(&mut selected, &mut anchor, &order, "d", false, true);
        assert_eq!(names(&selected), ["d"]);
        anchor = Some("gone".to_owned());
        clicked(&mut selected, &mut anchor, &order, "b", false, true);
        assert_eq!(names(&selected), ["b"]);
        clicked(&mut selected, &mut anchor, &[], "zzz", false, true);
        assert_eq!(names(&selected), ["zzz"]);
    }
}
