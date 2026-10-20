//! What the graph says to assistive technology.
//!
//! The graph is a list box of commits: the list has a name, each row is
//! an option of it with a name of its own (what the commit says, who
//! wrote it, when, and the branches and tags on it), selected or not, and
//! the row the keys are on is the one a screen reader follows while the
//! keys stay with the list. Each label of a row is a button named by what
//! it stands for, since two clicks on it check that out.
//!
//! The toolkit hands these to AccessKit only while assistive technology
//! is running; the names are made for every row drawn all the same, so
//! that the tests can read them.
//!
//! Sources: the toolkit's accessibility guide (`gpui::_accessibility`:
//! roles, labels, `aria_active_descendant`); AccessKit's `Role`
//! documentation; WAI-ARIA Authoring Practices, "Listbox".

use super::{RefLabel, WipCounts};

/// The graph's name.
pub(super) fn list_name() -> &'static str {
    say!("graph-access-list")
}

/// What kind of row a commit is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RowKind {
    Commit,
    Merge,
    Stash,
}

/// What a label stands for, in words: "branch main, checked out", "tag
/// v1.0", "remote branch origin/main".
pub(super) fn label_name(label: &RefLabel) -> String {
    let mut parts: Vec<String> = Vec::new();
    if label.stash {
        parts.push(say!("graph-access-stash", name = label.name.as_str()));
    } else if label.tag {
        parts.push(say!("graph-access-tag", name = label.name.as_str()));
    } else if label.head && !label.local && label.name == "HEAD" {
        parts.push(say!("graph-access-detached").to_owned());
    } else if label.local || label.head {
        parts.push(say!("graph-access-branch", name = label.name.as_str()));
    } else if let Some(first) = label.remotes.first() {
        parts.push(say!("graph-access-remote-branch", name = first.as_str()));
    } else if label.worktree.is_some() {
        parts.push(say!("graph-access-worktree", name = label.name.as_str()));
    } else {
        parts.push(label.name.clone());
    }
    if label.head && label.name != "HEAD" {
        parts.push(say!("graph-access-checked-out").to_owned());
    }
    if (label.local || label.head) && !label.remotes.is_empty() {
        let remotes = label.remotes.join(say!("graph-access-and"));
        parts.push(say!("graph-access-also-on", remotes));
    }
    if label.upstream {
        parts.push(say!("graph-access-upstream").to_owned());
    }
    if label.worktree.is_some() && (label.local || label.head) {
        parts.push(say!("graph-access-in-another-worktree").to_owned());
    }
    parts.join(", ")
}

/// A row's name: what the commit says, who wrote it and when, then what
/// is on it.
pub(super) fn row_name(
    kind: RowKind,
    summary: &str,
    author: Option<&str>,
    when: &str,
    labels: &[RefLabel],
) -> String {
    let summary = summary.trim();
    let summary = if summary.is_empty() {
        say!("graph-access-commit")
    } else {
        summary
    };
    let mut out = match kind {
        RowKind::Commit => summary.to_owned(),
        RowKind::Merge => say!("graph-access-merge", summary),
        RowKind::Stash => say!("graph-access-stash-row", summary),
    };
    if let Some(author) = author.map(str::trim).filter(|author| !author.is_empty()) {
        out.push_str(", ");
        out.push_str(&say!("graph-access-by", author));
    }
    let when = when.trim();
    if !when.is_empty() {
        out.push_str(", ");
        out.push_str(when);
    }
    let on: Vec<String> = labels
        .iter()
        .filter(|label| !label.stash)
        .map(label_name)
        .collect();
    if !on.is_empty() {
        out.push_str("; ");
        out.push_str(&on.join("; "));
    }
    out
}

/// The name of the row of uncommitted work, with how many files of each
/// kind it has when that is known.
pub(super) fn wip_name(counts: Option<WipCounts>) -> String {
    let Some(counts) = counts else {
        return say!("graph-access-wip").to_owned();
    };
    let mut kinds: Vec<String> = Vec::new();
    if counts.modified > 0 {
        kinds.push(say!("graph-access-changed", count = counts.modified));
    }
    if counts.added > 0 {
        kinds.push(say!("graph-access-added", count = counts.added));
    }
    if counts.deleted > 0 {
        kinds.push(say!("graph-access-deleted", count = counts.deleted));
    }
    if counts.renamed > 0 {
        kinds.push(say!("graph-access-renamed", count = counts.renamed));
    }
    if kinds.is_empty() {
        say!("graph-access-wip-none").to_owned()
    } else {
        say!("graph-access-wip-counted", kinds = kinds.join(", "))
    }
}

#[cfg(test)]
mod tests {
    use super::{RefLabel, RowKind, WipCounts, label_name, row_name, wip_name};

    fn label(name: &str) -> RefLabel {
        RefLabel {
            name: name.to_owned(),
            head: false,
            local: false,
            remotes: Vec::new(),
            tag: false,
            stash: false,
            worktree: None,
            upstream: false,
        }
    }

    #[test]
    fn a_label_says_what_it_stands_for() {
        let main = RefLabel {
            head: true,
            local: true,
            remotes: vec!["origin/main".to_owned()],
            ..label("main")
        };
        assert_eq!(
            label_name(&main),
            "branch main, checked out, also on origin/main"
        );
        let tag = RefLabel {
            tag: true,
            ..label("v1.0")
        };
        assert_eq!(label_name(&tag), "tag v1.0");
        let remote = RefLabel {
            remotes: vec!["upstream/dev".to_owned()],
            upstream: true,
            ..label("dev")
        };
        assert_eq!(label_name(&remote), "remote branch upstream/dev, upstream");
        let detached = RefLabel {
            head: true,
            ..label("HEAD")
        };
        assert_eq!(label_name(&detached), "HEAD, detached");
        let elsewhere = RefLabel {
            local: true,
            worktree: Some("/work/other".to_owned()),
            ..label("feature")
        };
        assert_eq!(
            label_name(&elsewhere),
            "branch feature, in another worktree"
        );
    }

    #[test]
    fn a_row_says_what_who_when_and_what_is_on_it() {
        let main = RefLabel {
            head: true,
            local: true,
            ..label("main")
        };
        assert_eq!(
            row_name(
                RowKind::Commit,
                "Fix the lexer",
                Some("Ann Example"),
                "3 d ago",
                &[main]
            ),
            "Fix the lexer, by Ann Example, 3 d ago; branch main, checked out"
        );
        assert_eq!(
            row_name(RowKind::Merge, "Merge dev", None, "", &[]),
            "Merge: Merge dev"
        );
        // Not read yet: a row is a commit all the same.
        assert_eq!(
            row_name(RowKind::Commit, "  ", Some(" "), " ", &[]),
            "commit"
        );
        let stash = RefLabel {
            stash: true,
            ..label("stash@{0}")
        };
        assert_eq!(
            row_name(RowKind::Stash, "WIP on main", None, "now", &[stash]),
            "Stash: WIP on main, now"
        );
    }

    #[test]
    fn the_row_of_uncommitted_work_counts_its_files() {
        assert_eq!(wip_name(None), "Uncommitted changes");
        assert_eq!(
            wip_name(Some(WipCounts::default())),
            "Uncommitted changes: none"
        );
        let counts = WipCounts {
            modified: 2,
            deleted: 1,
            ..WipCounts::default()
        };
        assert_eq!(
            wip_name(Some(counts)),
            "Uncommitted changes: 2 changed, 1 deleted"
        );
    }
}
