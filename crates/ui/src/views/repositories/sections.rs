//! The sections of the page of repositories, as a list that is put in
//! order and from which some are hidden: the four the page always has
//! and one per workspace. Which order and which are hidden is kept
//! (`prefs::RepositoriesPage`); a section the kept order does not name (a
//! workspace made since) comes after the ones it does, and a workspace
//! that is gone drops out of it.
//!
//! A heading is dragged onto another and its section lands before that
//! one; the end of the page takes one to the bottom. A section is hidden
//! from its heading's menu and shown again from the button that counts
//! the hidden ones, beside the box for the changes.
//!
//! Sources: none (original design).

use super::Part;

/// One section of the page.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Section {
    /// One of the four the page always has.
    Part(Part),
    /// A workspace's, by its id.
    Workspace(String),
}

/// What a workspace's section key begins with.
const WORKSPACE: &str = "workspace:";

impl Section {
    /// The key it is kept under, and that names it in element ids.
    pub fn key(&self) -> String {
        match self {
            Section::Part(part) => part.key().to_owned(),
            Section::Workspace(id) => format!("{WORKSPACE}{id}"),
        }
    }

    /// The section a kept key names, when it names one.
    pub fn parse(key: &str) -> Option<Section> {
        if let Some(id) = key.strip_prefix(WORKSPACE) {
            return (!id.is_empty()).then(|| Section::Workspace(id.to_owned()));
        }
        Part::ALL
            .into_iter()
            .find(|part| part.key() == key)
            .map(Section::Part)
    }
}

/// Every section there is, in the order they are shown: the kept order
/// first, then what it does not name. Without a kept order, the four of
/// the page and then the workspaces in the order they were made, a new
/// one at the end.
pub fn ordered(kept: &[String], workspaces: &[String]) -> Vec<Section> {
    let there: Vec<Section> = Part::ALL
        .into_iter()
        .map(Section::Part)
        .chain(workspaces.iter().cloned().map(Section::Workspace))
        .collect();
    let mut out: Vec<Section> = Vec::with_capacity(there.len());
    for key in kept {
        if let Some(section) = Section::parse(key)
            && there.contains(&section)
            && !out.contains(&section)
        {
            out.push(section);
        }
    }
    for section in there {
        if !out.contains(&section) {
            out.push(section);
        }
    }
    out
}

/// The order after `moved` was dropped before `before` (at the end when
/// there is none), as the keys to keep. None when nothing moved.
pub fn moved(shown: &[Section], moved: &Section, before: Option<&Section>) -> Option<Vec<String>> {
    if before == Some(moved) {
        return None;
    }
    let from = shown.iter().position(|section| section == moved)?;
    let mut order: Vec<Section> = shown.to_vec();
    let taken = order.remove(from);
    let to = match before {
        Some(before) => order.iter().position(|section| section == before)?,
        None => order.len(),
    };
    order.insert(to, taken);
    (order != shown).then(|| order.iter().map(Section::key).collect())
}

/// The hidden keys with one more hidden or shown again, as they are kept;
/// keys of sections that are gone drop out.
pub fn hidden_with(kept: &[String], key: &str, hide: bool, there: &[Section]) -> Vec<String> {
    let mut out: Vec<String> = kept
        .iter()
        .filter(|kept| *kept != key)
        .filter(|kept| Section::parse(kept).is_some_and(|section| there.contains(&section)))
        .cloned()
        .collect();
    if hide {
        out.push(key.to_owned());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{Section, hidden_with, moved, ordered};
    use crate::views::repositories::Part;

    fn keys(sections: &[Section]) -> Vec<String> {
        sections.iter().map(Section::key).collect()
    }

    #[test]
    fn sections_are_in_the_kept_order_and_new_ones_come_last() {
        let workspaces = ["team".to_owned(), "tools".to_owned()];
        assert_eq!(
            keys(&ordered(&[], &workspaces)),
            [
                "open",
                "favourites",
                "recent",
                "all",
                "workspace:team",
                "workspace:tools"
            ]
        );
        let kept = [
            "workspace:tools".to_owned(),
            "all".to_owned(),
            // Gone, unknown, twice: passed over.
            "workspace:gone".to_owned(),
            "nonsense".to_owned(),
            "all".to_owned(),
            "workspace:".to_owned(),
        ];
        assert_eq!(
            keys(&ordered(&kept, &workspaces)),
            [
                "workspace:tools",
                "all",
                "open",
                "favourites",
                "recent",
                "workspace:team"
            ]
        );
    }

    #[test]
    fn a_heading_dropped_on_another_puts_its_section_before_it() {
        let shown = ordered(&[], &["team".to_owned()]);
        let team = Section::Workspace("team".to_owned());
        let all = Section::Part(Part::All);
        let open = Section::Part(Part::Open);
        assert_eq!(
            moved(&shown, &team, Some(&all)).unwrap(),
            ["open", "favourites", "recent", "workspace:team", "all"]
        );
        assert_eq!(
            moved(&shown, &open, None).unwrap(),
            ["favourites", "recent", "all", "workspace:team", "open"]
        );
        // Onto itself, or where it is already: nothing moves.
        assert_eq!(moved(&shown, &team, Some(&team)), None);
        assert_eq!(moved(&shown, &team, None), None);
        let gone = Section::Workspace("gone".to_owned());
        assert_eq!(moved(&shown, &gone, Some(&open)), None);
        assert_eq!(moved(&shown, &open, Some(&gone)), None);
    }

    #[test]
    fn hidden_sections_are_kept_by_key_and_forgotten_when_gone() {
        let there = ordered(&[], &["team".to_owned()]);
        let hidden = hidden_with(&[], "all", true, &there);
        assert_eq!(hidden, ["all"]);
        let hidden = hidden_with(&hidden, "workspace:team", true, &there);
        assert_eq!(hidden, ["all", "workspace:team"]);
        let kept = ["workspace:gone".to_owned(), "all".to_owned()];
        assert_eq!(
            hidden_with(&kept, "all", false, &there),
            Vec::<String>::new()
        );
        assert_eq!(Section::parse("recent"), Some(Section::Part(Part::Recent)));
        assert_eq!(
            Section::parse("workspace:a-b"),
            Some(Section::Workspace("a-b".to_owned()))
        );
    }
}
