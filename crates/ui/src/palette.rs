//! What the command palette lists for what was typed, and in which order.
//! Nothing here draws; the window does.
//!
//! An entry has a label, sometimes a second name beside it (the id of a
//! command) and words it is also found by. Every word that was typed has
//! to be in one of them, by the matcher the filter of the references panel
//! uses (`crate::fuzzy`). How a word is found decides how far up the entry
//! stands:
//!
//! 1. in the label, as it was typed;
//! 2. in the second name or in one of the other words, as it was typed;
//! 3. in the label, as letters in order with others in between.
//!
//! Letters in order are not looked for outside the label: the other words
//! are whole descriptions, and nearly anything is in those. The worst word
//! decides; among equals the matcher's own order holds (start of a word
//! before the middle of one, the shorter name first), then the order the
//! entries come in. The sections are listed with the one that has the best
//! entry first, by the kind of match only: where two sections match
//! equally well they keep their order, so that the window's own way to do
//! something (which asks for what it needs in words) stays in front of the
//! engine's command of nearly the same name. The letters that matched are
//! marked in the label and in the second name.
//!
//! Sources: none (original design).

use std::ops::Range;

use crate::fuzzy::{Needle, Rank, marks, rank};

/// What an entry is found by.
#[derive(Debug, Clone, Copy)]
pub struct Names<'a> {
    /// What it says.
    pub label: &'a str,
    /// What stands small beside it.
    pub detail: Option<&'a str>,
    /// Further words.
    pub keywords: &'a [&'a str],
}

/// Which letters of an entry matched.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Found {
    /// In the label.
    pub label: Vec<Range<usize>>,
    /// In what stands beside it.
    pub detail: Vec<Range<usize>>,
}

/// How far up an entry stands; less is better.
pub type Order = (u8, Rank);

/// How one entry matches the words that were typed (each a needle of its
/// own, see [`Needle::each`]), and what to mark in it. None when a word is
/// in none of its names.
pub fn find(words: &[Needle], names: &Names<'_>, buffer: &mut Vec<char>) -> Option<(Order, Found)> {
    let mut worst: Option<Order> = None;
    let mut found = Found::default();
    for word in words {
        let in_label = rank(word, names.label, buffer);
        let order = if let Some(rank) = in_label.filter(Rank::in_a_row) {
            found.label.extend(marks(word, names.label));
            (0, rank)
        } else {
            let in_detail = names
                .detail
                .and_then(|detail| rank(word, detail, buffer))
                .filter(Rank::in_a_row);
            let in_keywords = names
                .keywords
                .iter()
                .filter_map(|keyword| rank(word, keyword, buffer))
                .filter(Rank::in_a_row)
                .min();
            match (in_detail, in_keywords, in_label) {
                (Some(rank), keyword, _) => {
                    if let Some(detail) = names.detail {
                        found.detail.extend(marks(word, detail));
                    }
                    (1, keyword.map_or(rank, |keyword| keyword.min(rank)))
                }
                (None, Some(rank), _) => (1, rank),
                (None, None, Some(rank)) => {
                    found.label.extend(marks(word, names.label));
                    (2, rank)
                }
                (None, None, None) => return None,
            }
        };
        worst = Some(worst.map_or(order, |so_far| so_far.max(order)));
    }
    found.label = joined(found.label);
    found.detail = joined(found.detail);
    Some((worst?, found))
}

/// Marks in order, with those that touch or overlap made one.
fn joined(mut marks: Vec<Range<usize>>) -> Vec<Range<usize>> {
    marks.sort_by_key(|mark| (mark.start, mark.end));
    let mut out: Vec<Range<usize>> = Vec::new();
    for mark in marks {
        match out.last_mut() {
            Some(before) if before.end >= mark.start => before.end = before.end.max(mark.end),
            _ => out.push(mark),
        }
    }
    out
}

/// One section as it is listed: which of the sections it is, and its
/// entries by their place in it, best first, each with what to mark.
pub type Listed = (usize, Vec<(usize, Found)>);

/// What the palette lists for what was typed. With nothing typed, every
/// section that has entries, as it comes. Otherwise the entries that
/// match, best first, in the sections that have any, the section with the
/// best entry first.
pub fn listing(sections: &[Vec<Names<'_>>], typed: &str) -> Vec<Listed> {
    let needle = Needle::new(typed);
    if needle.is_empty() {
        return sections
            .iter()
            .enumerate()
            .filter(|(_, entries)| !entries.is_empty())
            .map(|(at, entries)| {
                (
                    at,
                    (0..entries.len())
                        .map(|entry| (entry, Found::default()))
                        .collect(),
                )
            })
            .collect();
    }
    let words = needle.each();
    let mut buffer = Vec::new();
    let mut listed: Vec<(Order, Listed)> = Vec::new();
    for (at, entries) in sections.iter().enumerate() {
        let mut matches: Vec<(Order, usize, Found)> = entries
            .iter()
            .enumerate()
            .filter_map(|(entry, names)| {
                let (order, found) = find(&words, names, &mut buffer)?;
                Some((order, entry, found))
            })
            .collect();
        matches.sort_by_key(|(order, entry, _)| (*order, *entry));
        let Some(best) = matches.first().map(|(order, _, _)| *order) else {
            continue;
        };
        listed.push((
            best,
            (
                at,
                matches
                    .into_iter()
                    .map(|(_, entry, found)| (entry, found))
                    .collect(),
            ),
        ));
    }
    listed.sort_by_key(|(best, (at, _))| (best.0, best.1.kind(), *at));
    listed.into_iter().map(|(_, section)| section).collect()
}

#[cfg(test)]
mod tests {
    use super::{Found, Names, find, listing};
    use crate::fuzzy::Needle;

    /// Marks as where they begin and end.
    fn spans(marks: &[std::ops::Range<usize>]) -> Vec<(usize, usize)> {
        marks.iter().map(|mark| (mark.start, mark.end)).collect()
    }

    fn entry<'a>(label: &'a str, detail: Option<&'a str>, keywords: &'a [&'a str]) -> Names<'a> {
        Names {
            label,
            detail,
            keywords,
        }
    }

    /// The labels of what is listed, section by section.
    fn labels<'a>(sections: &[Vec<Names<'a>>], typed: &str) -> Vec<(usize, Vec<&'a str>)> {
        listing(sections, typed)
            .into_iter()
            .map(|(at, entries)| {
                (
                    at,
                    entries
                        .into_iter()
                        .map(|(entry, _)| sections[at][entry].label)
                        .collect(),
                )
            })
            .collect()
    }

    fn sections() -> Vec<Vec<Names<'static>>> {
        vec![
            vec![
                entry("Command palette", None, &["commands", "run", "find"]),
                entry(
                    "Pull the current branch",
                    None,
                    &["pull", "download", "merge", "rebase", "remote.pull"],
                ),
                entry(
                    "Push the current branch",
                    None,
                    &["push", "upload", "publish", "remote.push"],
                ),
                entry(
                    "Pop the newest stash",
                    None,
                    &["stash", "apply", "stash.pop"],
                ),
                entry("Preferences", None, &["settings", "options", "configure"]),
            ],
            Vec::new(),
            vec![
                entry(
                    "Pull",
                    Some("remote.pull"),
                    &["Fetch", "and", "integrate", "the", "upstream"],
                ),
                entry(
                    "Delete a branch",
                    Some("branch.delete"),
                    &["Delete", "a", "local", "branch", "that", "was", "pulled"],
                ),
                entry(
                    "Create a branch",
                    Some("branch.create"),
                    &["Create", "a", "branch", "at", "a", "revision"],
                ),
                entry("Push", Some("remote.push"), &["Upload", "the", "branch"]),
            ],
        ]
    }

    #[test]
    fn with_nothing_typed_everything_is_listed_as_it_comes() {
        let sections = sections();
        let all = listing(&sections, "  ");
        // A section without entries has no heading.
        assert_eq!(all.iter().map(|(at, _)| *at).collect::<Vec<_>>(), [0, 2]);
        assert_eq!(all[0].1.len(), 5);
        assert_eq!(all[1].1.len(), 4);
        assert!(
            all.iter()
                .flat_map(|(_, entries)| entries)
                .all(|(_, found)| *found == Found::default())
        );
        assert_eq!(
            all[0].1.iter().map(|(entry, _)| *entry).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4]
        );
        assert!(listing(&[], "x").is_empty());
        assert!(listing(&[Vec::new()], "").is_empty());
    }

    #[test]
    fn what_is_typed_ranks_the_entries_and_the_sections() {
        let sections = sections();
        // The label that is the word comes before one that starts with it;
        // the section that has it comes first. A description that has the
        // word ("pulled") comes after every label that has it.
        assert_eq!(
            labels(&sections, "pull"),
            [
                (2, vec!["Pull", "Delete a branch"]),
                (0, vec!["Pull the current branch"]),
            ]
        );
        // Letters with others in between find a label, after everything
        // that has the text as it was typed.
        assert_eq!(
            labels(&sections, "prf"),
            [(0, vec!["Preferences"])],
            "p-r-f"
        );
        assert_eq!(
            labels(&sections, "nwst"),
            [(0, vec!["Pop the newest stash"])]
        );
        // Found by the id beside the label and by another word.
        assert_eq!(
            labels(&sections, "branch.del"),
            [(2, vec!["Delete a branch"])]
        );
        assert_eq!(labels(&sections, "settings"), [(0, vec!["Preferences"])]);
        // Letters in order are not looked for in the other words: "stg"
        // is in "settings" that way, and in no label.
        assert!(labels(&sections, "stg").is_empty());
        // Every word has to be found, each where it is.
        assert_eq!(
            labels(&sections, "branch upload"),
            [(0, vec!["Push the current branch"]), (2, vec!["Push"])]
        );
        assert!(labels(&sections, "branch zzz").is_empty());
        // Upper and lower case are one.
        assert_eq!(labels(&sections, "PREF"), [(0, vec!["Preferences"])]);
        // Two sections that match equally well keep their order: the
        // shorter label of the later one does not bring it forward.
        assert_eq!(
            labels(&sections, "pu"),
            [
                (
                    0,
                    vec!["Pull the current branch", "Push the current branch"]
                ),
                (2, vec!["Pull", "Push", "Delete a branch"]),
            ]
        );
    }

    #[test]
    fn the_letters_that_matched_are_marked_where_they_are_shown() {
        let words = Needle::new("cur br").each();
        let names = entry("Push the current branch", None, &[]);
        let (_, found) = find(&words, &names, &mut Vec::new()).unwrap();
        assert_eq!(spans(&found.label), [(9, 12), (17, 19)]);
        assert!(found.detail.is_empty());
        // In the id beside the label, when that is where it is.
        let words = Needle::new("remote.pu").each();
        let names = entry("Push", Some("remote.push"), &[]);
        let (order, found) = find(&words, &names, &mut Vec::new()).unwrap();
        assert_eq!(order.0, 1);
        assert_eq!(spans(&found.detail), [(0, 9)]);
        assert!(found.label.is_empty());
        // Found by a word that is not shown: nothing to mark.
        let words = Needle::new("upload").each();
        let names = entry("Push", Some("remote.push"), &["upload"]);
        let (_, found) = find(&words, &names, &mut Vec::new()).unwrap();
        assert_eq!(found, Found::default());
        // Letters in order, in the label.
        let words = Needle::new("prf").each();
        let names = entry("Preferences", None, &[]);
        let (order, found) = find(&words, &names, &mut Vec::new()).unwrap();
        assert_eq!(order.0, 2);
        assert_eq!(spans(&found.label), [(0, 2), (3, 4)]);
        // Two words on the same letters are one mark.
        let words = Needle::new("pre pref").each();
        let (_, found) = find(&words, &names, &mut Vec::new()).unwrap();
        assert_eq!(spans(&found.label), [(0, 4)]);
        // No words, no match: the caller lists everything instead.
        assert!(find(&[], &names, &mut Vec::new()).is_none());
    }

    #[test]
    fn hostile_text_is_only_text() {
        let long = "x".repeat(100_000);
        let odd = ["\u{0}", "İ", "😀", "\u{202e}", &long];
        let keywords = odd;
        let sections = vec![vec![
            entry("", None, &[]),
            entry(&long, Some(&long), &keywords),
            entry("İstanbul 😀", Some("\u{0}"), &keywords),
        ]];
        for typed in [
            "\u{0}",
            "İ",
            "i",
            "😀",
            &long,
            "x x x x x x x x x x x x",
            "\u{202e}",
        ] {
            let _ = listing(&sections, typed);
        }
        let found = listing(&sections, "ist");
        assert_eq!(found.len(), 1);
        for (entry, marks) in &found[0].1 {
            let label = sections[0][*entry].label;
            for mark in &marks.label {
                assert!(label.is_char_boundary(mark.start) && label.is_char_boundary(mark.end));
            }
        }
    }
}
