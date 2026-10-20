//! Fuzzy matching of what is typed against names, with a rank, and the
//! letters that matched: one matcher for the filter of the references
//! panel and for the command palette.
//!
//! A name matches when it has every word that was typed, each either in a
//! row or as its letters in order with others in between; upper and lower
//! case are the same. Best first:
//!
//! 1. the name is exactly what was typed;
//! 2. the text stands at the start of a word of the name (the start of the
//!    name, or after anything that is not a letter or a digit);
//! 3. the text stands somewhere else in the name;
//! 4. the letters are there in order with others in between, the fewer in
//!    between the better.
//!
//! Among equals the shorter name wins, then the earlier place in the name.
//! With several words the worst one decides.
//!
//! [`marks`] says where in the name the match is, so that it can be shown:
//! the same places the rank was given for.
//!
//! Sources: none (original design).

use std::ops::Range;

use gpui_kit::{FontWeight, HighlightStyle, Hsla, SharedString, StyledText};

/// What was typed, made ready to compare names with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Needle {
    /// The words, in lower case.
    words: Vec<Vec<char>>,
}

impl Needle {
    /// The words of what was typed; no more than eight of them count.
    pub fn new(typed: &str) -> Self {
        Self {
            words: typed
                .split_whitespace()
                .take(8)
                .map(|word| word.chars().flat_map(char::to_lowercase).collect())
                .collect(),
        }
    }

    /// Nothing was typed: everything is kept as it is.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Every word as a needle of its own, for a thing that has several
    /// names a word may be in.
    pub fn each(&self) -> Vec<Needle> {
        self.words
            .iter()
            .map(|word| Needle {
                words: vec![word.clone()],
            })
            .collect()
    }
}

/// How well a name matches; less is better (see the module's text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Rank {
    /// 0 exact, 1 at the start of a word, 2 in a row elsewhere, 3 the
    /// letters in order.
    class: u8,
    /// Letters in between, for the last class.
    gaps: u16,
    /// How long the name is.
    length: u16,
    /// Where in the name the match begins.
    place: u16,
}

impl Rank {
    /// Whether what was typed stands in the name as it was typed, not as
    /// letters with others in between.
    pub fn in_a_row(&self) -> bool {
        self.class < 3
    }

    /// The kind of match alone, without where it is and how long the name
    /// is: 0 the name itself, 1 the start of a word, 2 inside a word, 3
    /// letters in order.
    pub fn kind(&self) -> u8 {
        self.class
    }
}

fn small(n: usize) -> u16 {
    u16::try_from(n).unwrap_or(u16::MAX)
}

/// A character that words of a name are parted by: anything that is not a
/// letter or a digit. A branch has its `/`, `-`, `_` and `.`; a label of
/// the palette has brackets and colons as well, and the word in
/// "resolutions (rerere): list" starts a word like any other.
fn parts_words(c: char) -> bool {
    !c.is_alphanumeric()
}

/// Where a word was found in a name.
#[derive(Debug, Clone, Copy)]
enum Found {
    /// In a row, from this place on.
    Row(usize),
    /// Letter by letter, the last of them at this place.
    Spread(usize),
}

/// How one word matches a name that is in lower case already, and where.
fn locate(word: &[char], name: &[char]) -> Option<(Rank, Found)> {
    let length = small(name.len());
    if word.is_empty() {
        return None;
    }
    if word.len() > name.len() {
        return None;
    }
    // In a row: the best place it stands at.
    let mut in_a_row: Option<(u8, usize)> = None;
    for (at, window) in name.windows(word.len()).enumerate() {
        if window != word {
            continue;
        }
        let starts_word = at
            .checked_sub(1)
            .and_then(|before| name.get(before))
            .is_none_or(|before| parts_words(*before));
        if starts_word {
            in_a_row = Some((1, at));
            break;
        }
        in_a_row.get_or_insert((2, at));
    }
    if let Some((class, at)) = in_a_row {
        return Some((
            Rank {
                class: if word.len() == name.len() { 0 } else { class },
                gaps: 0,
                place: small(at),
                length,
            },
            Found::Row(at),
        ));
    }
    // The letters in order: find where the last one falls going forward,
    // then go back from there for the shortest stretch that holds them.
    let mut wanted = word.iter();
    let mut next = wanted.next();
    let mut last = None;
    for (at, c) in name.iter().enumerate() {
        if Some(c) == next {
            next = wanted.next();
            if next.is_none() {
                last = Some(at);
                break;
            }
        }
    }
    let last = last?;
    let mut first = last;
    spread_back(word, name, last, |at| first = at);
    let stretch = last.saturating_sub(first).saturating_add(1);
    Some((
        Rank {
            class: 3,
            gaps: small(stretch.saturating_sub(word.len())),
            place: small(first),
            length,
        },
        Found::Spread(last),
    ))
}

/// Walk back from the place the last letter of a word fell on and say each
/// place a letter of it is taken from, the last letter first.
fn spread_back(word: &[char], name: &[char], last: usize, mut taken: impl FnMut(usize)) {
    let mut wanted = word.iter().rev();
    let mut next = wanted.next();
    for (at, c) in name.iter().enumerate().take(last.saturating_add(1)).rev() {
        if Some(c) == next {
            next = wanted.next();
            taken(at);
            if next.is_none() {
                break;
            }
        }
    }
}

/// Put a name in lower case, letter by letter.
fn lowered(name: &str, buffer: &mut Vec<char>) {
    buffer.clear();
    if name.is_ascii() {
        // Nearly every name: no tables to look letters up in.
        buffer.extend(
            name.bytes()
                .map(|byte| char::from(byte.to_ascii_lowercase())),
        );
    } else {
        buffer.extend(name.chars().flat_map(char::to_lowercase));
    }
}

/// How a name matches what was typed: every word has to match, and the
/// worst word decides. `buffer` is room to put the name in lower case.
pub fn rank(needle: &Needle, name: &str, buffer: &mut Vec<char>) -> Option<Rank> {
    lowered(name, buffer);
    let mut worst: Option<Rank> = None;
    for word in &needle.words {
        let (rank, _) = locate(word, buffer)?;
        worst = Some(worst.map_or(rank, |so_far| so_far.max(rank)));
    }
    worst
}

/// Where in a name what was typed matches: the bytes of the letters the
/// rank was given for, in order, a stretch where they stand in a row. A
/// word that is not in the name marks nothing; the others still do.
pub fn marks(needle: &Needle, name: &str) -> Vec<Range<usize>> {
    if needle.is_empty() || name.is_empty() {
        return Vec::new();
    }
    // The name in lower case, and for each of those letters the bytes of
    // the letter of the name it came from (one letter can become several).
    let mut letters: Vec<char> = Vec::new();
    let mut origin: Vec<Range<usize>> = Vec::new();
    for (at, c) in name.char_indices() {
        let bytes = at..at.saturating_add(c.len_utf8());
        for low in c.to_lowercase() {
            letters.push(low);
            origin.push(bytes.clone());
        }
    }
    let mut taken = vec![false; letters.len()];
    let mut take = |at: usize| {
        if let Some(slot) = taken.get_mut(at) {
            *slot = true;
        }
    };
    for word in &needle.words {
        match locate(word, &letters) {
            Some((_, Found::Row(at))) => (at..at.saturating_add(word.len())).for_each(&mut take),
            Some((_, Found::Spread(last))) => spread_back(word, &letters, last, &mut take),
            None => {}
        }
    }
    let mut out: Vec<Range<usize>> = Vec::new();
    for (bytes, _) in origin.iter().zip(&taken).filter(|(_, taken)| **taken) {
        match out.last_mut() {
            Some(before) if before.end >= bytes.start => before.end = before.end.max(bytes.end),
            _ => out.push(bytes.clone()),
        }
    }
    out
}

/// The marks of a name that fall into one part of it, counted from where
/// that part begins: for a name that is shown in pieces.
pub fn marks_within(marks: &[Range<usize>], part: Range<usize>) -> Vec<Range<usize>> {
    marks
        .iter()
        .filter_map(|mark| {
            let start = mark.start.max(part.start);
            let end = mark.end.min(part.end);
            (start < end).then(|| start.saturating_sub(part.start)..end.saturating_sub(part.start))
        })
        .collect()
}

/// A text with the letters that matched standing out. Marks that do not
/// fit the text (beyond its end, inside a letter, out of order) are left
/// out: the text is shown either way.
pub fn marked(text: impl Into<SharedString>, marks: &[Range<usize>], colour: Hsla) -> StyledText {
    let text: SharedString = text.into();
    let mut end = 0usize;
    let mut fitting = Vec::new();
    for mark in marks {
        if mark.start >= end
            && mark.start < mark.end
            && mark.end <= text.len()
            && text.is_char_boundary(mark.start)
            && text.is_char_boundary(mark.end)
        {
            end = mark.end;
            fitting.push((
                mark.clone(),
                HighlightStyle {
                    color: Some(colour),
                    font_weight: Some(FontWeight::BOLD),
                    ..HighlightStyle::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(fitting)
}

#[cfg(test)]
mod tests {
    use super::{Needle, marks, marks_within, rank};

    /// Marks as where they begin and end.
    fn spans(marks: &[std::ops::Range<usize>]) -> Vec<(usize, usize)> {
        marks.iter().map(|mark| (mark.start, mark.end)).collect()
    }

    fn marked_of(typed: &str, name: &str) -> Vec<String> {
        marks(&Needle::new(typed), name)
            .into_iter()
            .map(|range| name[range].to_owned())
            .collect()
    }

    #[test]
    fn the_marks_are_the_letters_the_rank_was_given_for() {
        // In a row: one stretch, at the start of a word when there is one.
        assert_eq!(marked_of("log", "feature/login-dialog"), ["log"]);
        assert_eq!(marked_of("LOG", "feature/Login"), ["Log"]);
        assert_eq!(marked_of("main", "main"), ["main"]);
        // The start of a word is taken before a place inside one.
        assert_eq!(spans(&marks(&Needle::new("in"), "main/inbox")), [(5, 7)]);
        // Letters in order: the shortest stretch that holds them.
        assert_eq!(marked_of("flg", "feature/login"), ["f", "l", "g"]);
        assert_eq!(
            spans(&marks(&Needle::new("ab"), "a-xx-a-b")),
            [(5, 6), (7, 8)]
        );
        // Several words: each marks its own, and stretches that touch are
        // one.
        assert_eq!(marked_of("fix log", "fix/login"), ["fix", "log"]);
        assert_eq!(marked_of("fea ture", "feature"), ["feature"]);
        // A word that is not there marks nothing; the other still does.
        assert_eq!(marked_of("log zzz", "login"), ["log"]);
        // Nothing typed, nothing to look in.
        assert!(marks(&Needle::new("  "), "login").is_empty());
        assert!(marks(&Needle::new("a"), "").is_empty());
    }

    #[test]
    fn a_word_starts_after_anything_that_is_not_a_letter_or_a_digit() {
        let kind = |typed: &str, name: &str| {
            rank(&Needle::new(typed), name, &mut Vec::new()).map(|rank| rank.kind())
        };
        // The marks of a branch's name.
        for name in [
            "fix/login",
            "fix-login",
            "fix_login",
            "fix.login",
            "fix login",
        ] {
            assert_eq!(kind("login", name), Some(1), "{name}");
        }
        // What a label is written with: a word in brackets or after a
        // colon is as good as one after a space.
        assert_eq!(
            kind("rerere", "Recorded resolutions (rerere): list"),
            Some(1)
        );
        assert_eq!(kind("rerere", "What rerere has recorded"), Some(1));
        assert_eq!(kind("list", "resolutions:list"), Some(1));
        assert_eq!(kind("42", "issue#42"), Some(1));
        // Inside a word it stays what it was, digits and letters of any
        // script being letters.
        assert_eq!(kind("ogin", "fix/login"), Some(2));
        assert_eq!(kind("2", "v12"), Some(2));
        assert_eq!(kind("本語", "日本語"), Some(2));
    }

    #[test]
    fn marks_fall_on_whole_letters_of_any_script() {
        // Letters of more than one byte, and one that becomes two in lower
        // case: the marks are bytes of the name as it is written.
        for (typed, name) in [
            ("é", "café/été"),
            ("fé", "CAFÉ"),
            ("i", "İstanbul"),
            ("ist", "İstanbul"),
            ("ß", "straße"),
            ("日本", "ブランチ/日本語"),
            ("x", "😀x😀"),
        ] {
            let found = marks(&Needle::new(typed), name);
            assert!(!found.is_empty(), "{typed} in {name}");
            let mut end = 0;
            for range in &found {
                assert!(range.start >= end && range.start < range.end, "{found:?}");
                assert!(
                    name.is_char_boundary(range.start) && name.is_char_boundary(range.end),
                    "{typed} in {name}: {found:?}"
                );
                end = range.end;
            }
            assert!(end <= name.len());
            // Whatever marks, ranks.
            assert!(rank(&Needle::new(typed), name, &mut Vec::new()).is_some());
        }
    }

    #[test]
    fn marks_are_shared_out_among_the_parts_of_a_name() {
        // "feature/login" shown as the folder and the name.
        let name = "feature/login";
        let found = marks(&Needle::new("ure/log"), name);
        assert_eq!(spans(&found), [(4, 11)]);
        assert_eq!(spans(&marks_within(&found, 0..7)), [(4, 7)]);
        assert_eq!(spans(&marks_within(&found, 8..13)), [(0, 3)]);
        assert!(marks_within(&found, 11..13).is_empty());
        assert!(marks_within(&found, 20..30).is_empty());
        assert!(marks_within(&[], 0..7).is_empty());
    }

    #[test]
    fn every_word_is_a_needle_of_its_own() {
        let needle = Needle::new("Fix  LOG");
        let each = needle.each();
        assert_eq!(each.len(), 2);
        assert!(rank(&each[0], "fix", &mut Vec::new()).is_some());
        assert!(rank(&each[0], "log", &mut Vec::new()).is_none());
        assert!(rank(&each[1], "login", &mut Vec::new()).is_some_and(|rank| rank.in_a_row()));
        assert!(rank(&each[1], "l-o-g", &mut Vec::new()).is_some_and(|rank| !rank.in_a_row()));
        assert!(Needle::new("").each().is_empty());
    }
}
