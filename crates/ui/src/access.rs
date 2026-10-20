//! What the window's own elements are called for assistive technology.
//!
//! The toolkit's controls (buttons, check boxes, fields) report a role of
//! their own and the name they are given; what this crate draws itself (a
//! row of the changes, a tab, the terminal) is given a role and a name
//! where it is built, with the words from here. The toolkit passes them on
//! to AccessKit, and AccessKit to the system's accessibility service,
//! while assistive technology is running. The graph's own words are in
//! `views/graph/access.rs`.
//!
//! What the toolkit does not allow: text drawn as a plain string has no
//! node of its own, so a screen reader finds the names of controls, rows,
//! lists and regions, not the sentences around them (a note under a
//! setting is read only where it is a control's description); the entries
//! of the toolkit's command list (the palette) and its dialogs are
//! reported without a name, which only the toolkit could give them.
//!
//! Sources: the toolkit's accessibility guide (`gpui::_accessibility`);
//! AccessKit's `Role` documentation; WAI-ARIA Authoring Practices
//! ("Listbox", "Tabs", "Disclosure").

use bitbraid_git::ChangeKind;
use bitbraid_strings::Message;

/// The changes panel's name.
pub const CHANGES_NAME: Message = message!("access-changes-name");

/// The tabs' name.
pub const TABS_NAME: Message = message!("access-tabs-name");

/// The name of the pinned tabs before them: the page of repositories and
/// the inbox.
pub const PINNED_TABS_NAME: Message = message!("access-pinned-tabs-name");

/// The terminal's name.
pub const TERMINAL_NAME: Message = message!("access-terminal-name");

/// What happened to a file, in a word or two.
pub fn change_words(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Added => say!("access-change-added"),
        ChangeKind::Modified => say!("access-change-modified"),
        ChangeKind::Deleted => say!("access-change-deleted"),
        ChangeKind::Renamed => say!("access-change-renamed"),
        ChangeKind::Copied => say!("access-change-copied"),
        ChangeKind::TypeChanged => say!("access-change-type-changed"),
        ChangeKind::Untracked => say!("access-change-untracked"),
        ChangeKind::IntentToAdd => say!("access-change-intent-to-add"),
        ChangeKind::Conflicted => say!("access-change-conflicted"),
    }
}

/// A file's row in the changes: its path, what happened to it, and
/// whether that is staged.
pub fn file_row(path: &str, kind: ChangeKind, staged: bool, resolved: bool) -> String {
    let what = if resolved {
        say!("access-change-resolved")
    } else {
        change_words(kind)
    };
    let side = if staged {
        say!("access-staged")
    } else {
        say!("access-not-staged")
    };
    say!("access-file-row", path, what, side)
}

/// The heading of a section of the references: its title as words are
/// written (the heading shows it in capitals, which a screen reader would
/// spell out), what it holds and how many.
pub fn section_heading(title: &str, count: usize) -> String {
    let lower = title.trim().to_lowercase();
    let mut letters = lower.chars();
    let title: String = letters
        .next()
        .map(|first| first.to_uppercase().chain(letters).collect())
        .unwrap_or_default();
    say!("access-section-heading", title, count)
}

/// A folder's row in the changes, as a tree shows them.
pub fn folder_row(path: &str) -> String {
    say!("access-folder-row", path)
}

/// A file's row among those in conflict.
pub fn conflict_row(path: &str) -> String {
    say!("access-conflict-row", path)
}

#[cfg(test)]
mod tests {
    use bitbraid_git::ChangeKind;

    use super::{conflict_row, file_row, folder_row, section_heading};

    #[test]
    fn a_file_row_says_its_path_what_happened_and_where() {
        assert_eq!(
            file_row("src/lib.rs", ChangeKind::Modified, false, false),
            "src/lib.rs, changed, not staged"
        );
        assert_eq!(
            file_row("new.txt", ChangeKind::Untracked, false, false),
            "new.txt, new, not staged"
        );
        assert_eq!(
            file_row("a.txt", ChangeKind::Added, true, true),
            "a.txt, resolved, staged"
        );
        assert_eq!(folder_row("docs"), "folder docs");
        assert_eq!(section_heading("LOCAL BRANCHES", 6), "Local branches, 6");
        assert_eq!(section_heading("", 0), ", 0");
        assert_eq!(conflict_row("docs/guide.md"), "docs/guide.md, in conflict");
    }
}
