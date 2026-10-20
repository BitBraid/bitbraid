//! Text that is shown as it was written.
//!
//! Most of what a git client shows was typed somewhere else: the lines of
//! a file, a commit message, a branch name, a command line. A font may
//! draw a run of characters as one sign (a ligature: `--`, `->`, `!=`,
//! `fi`), which is wrong for all of it. The window's text is therefore
//! shaped with the features that join characters switched off
//! ([`features`], set where the shell puts its root element). What the
//! toolkit draws outside that element (dialogs, menus, notifications) is
//! not reached.
//!
//! Two hyphens can read as one long dash in the interface font without
//! any ligature, though: in some fonts (Ubuntu, for one) hyphens nearly
//! touch, less than a pixel apart at the sizes used here. A command line
//! is therefore shown and typed in the monospace font ([`command`]), where
//! every character keeps a cell of its own.
//!
//! What is said in a notification or a dialog is a sentence, and often
//! git's own, with such things inside it: `--force`, a command in
//! backticks, a name between single quotes. Those are drawn outside the
//! window's root element, where neither of the two measures above reaches.
//! [`said`] is for them: the sentence with joining switched off, and its
//! literal parts ([`spans`]) in the monospace font.
//!
//! Sources: the OpenType specification's registered features (`calt`,
//! `liga`, `clig`, `dlig`); the toolkit's `FontFeatures` and `StyledText`
//! (`with_font_family_overrides`).

use std::{ops::Range, sync::Arc};

use gpui_kit::component::ActiveTheme;
use gpui_kit::{
    App, Div, FontFeatures, HighlightStyle, ParentElement, SharedString, Styled, StyledText, div,
};

/// The features by which a font joins characters: contextual alternates
/// (where coding fonts keep `--` and `->`), and the standard, contextual
/// and discretionary ligatures.
pub const JOINING: [&str; 4] = ["calt", "liga", "clig", "dlig"];

/// Font features with everything that joins characters switched off.
pub fn features() -> FontFeatures {
    FontFeatures(Arc::new(
        JOINING.iter().map(|tag| ((*tag).to_owned(), 0)).collect(),
    ))
}

/// A box for a command line: the monospace font at its own size.
pub fn command(cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
}

/// The longest quoted part that is taken for a name or a command; past
/// that it is a sentence somebody quoted.
const LONGEST_QUOTED: usize = 200;

/// The parts of a sentence that are to be read letter by letter, as bytes
/// of the text, in order and apart from each other:
///
/// * an option as it is typed: `--force`, `--force-with-lease=main`;
/// * what stands between backticks;
/// * what stands between single quotes the way git quotes a name or a
///   command (`'origin/main'`, `'git add'`): the quote opens at the start
///   or after a space or a bracket, and closes before a space, the end or
///   a mark of punctuation. An apostrophe inside a word is none of that.
pub fn spans(text: &str) -> Vec<Range<usize>> {
    let bytes = text.as_bytes();
    let mut out: Vec<Range<usize>> = Vec::new();
    let mut at = 0usize;
    while let Some(byte) = bytes.get(at).copied() {
        let found = match byte {
            b'-' => option_at(bytes, at),
            b'`' => quoted_at(text, at, byte),
            b'\'' if opens_a_quote(bytes, at) => quoted_at(text, at, byte),
            _ => None,
        };
        match found {
            Some((part, next)) => {
                out.push(part);
                at = next;
            }
            None => at = at.saturating_add(1),
        }
    }
    out
}

/// An option that begins at `at`: its bytes, and where to read on.
fn option_at(bytes: &[u8], at: usize) -> Option<(Range<usize>, usize)> {
    // Two hyphens and a letter or digit, not inside a word: `a--b` and
    // `---` are no options.
    let begins = bytes.get(at.saturating_add(1)) == Some(&b'-')
        && bytes
            .get(at.saturating_add(2))
            .is_some_and(u8::is_ascii_alphanumeric)
        && at
            .checked_sub(1)
            .and_then(|before| bytes.get(before))
            .is_none_or(|before| !before.is_ascii_alphanumeric() && *before != b'-');
    if !begins {
        return None;
    }
    let mut end = at.saturating_add(2);
    while bytes.get(end).is_some_and(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'=' | b'/' | b'.' | b':')
    }) {
        end = end.saturating_add(1);
    }
    // What ends the sentence, or leads on to what follows, is not part of
    // the option.
    while bytes
        .get(end.saturating_sub(1))
        .is_some_and(|byte| matches!(byte, b'.' | b':' | b'=' | b'/'))
    {
        end = end.saturating_sub(1);
    }
    Some((at..end, end))
}

/// Whether a single quote at `at` opens a quotation: at the start, or
/// after a space, a bracket, a colon or a double quote. After a letter it
/// is an apostrophe.
fn opens_a_quote(bytes: &[u8], at: usize) -> bool {
    at.checked_sub(1)
        .and_then(|before| bytes.get(before))
        .is_none_or(|before| matches!(before, b' ' | b'\n' | b'\t' | b'(' | b'[' | b':' | b'"'))
}

/// What stands between the mark at `at` and the next one like it on the
/// same line: its bytes, and where to read on. None when it never closes,
/// holds nothing, is too long, or (for a single quote) closes in front of
/// a letter.
fn quoted_at(text: &str, at: usize, mark: u8) -> Option<(Range<usize>, usize)> {
    let bytes = text.as_bytes();
    let inside = at.saturating_add(1);
    let offset = bytes
        .get(inside..)?
        .iter()
        .take(LONGEST_QUOTED)
        .position(|found| *found == mark || *found == b'\n')?;
    let end = inside.saturating_add(offset);
    if bytes.get(end) != Some(&mark) || end == inside {
        return None;
    }
    let closes = mark == b'`'
        || bytes
            .get(end.saturating_add(1))
            .is_none_or(|after| !after.is_ascii_alphanumeric() && *after != b'\'');
    (closes && text.is_char_boundary(inside) && text.is_char_boundary(end))
        .then(|| (inside..end, end.saturating_add(1)))
}

/// A sentence as it is shown in a notification or a dialog: nothing
/// joined into one sign, and its literal parts in the monospace font,
/// where two hyphens are two hyphens.
pub fn said(text: impl Into<SharedString>, cx: &App) -> Div {
    let text: SharedString = text.into();
    let mono = cx.theme().mono_font_family.clone();
    let parts: Vec<(Range<usize>, SharedString)> = spans(&text)
        .into_iter()
        .map(|part| (part, mono.clone()))
        .collect();
    // The toolkit gives a part another font only where the part is a run
    // of its own, and runs come from highlights: each part is highlighted
    // with nothing, which makes it one.
    let runs: Vec<(Range<usize>, HighlightStyle)> = parts
        .iter()
        .map(|(part, _)| (part.clone(), HighlightStyle::default()))
        .collect();
    div().font_features(features()).child(
        StyledText::new(text)
            .with_highlights(runs)
            .with_font_family_overrides(parts),
    )
}

#[cfg(test)]
mod tests {
    use super::{JOINING, features, spans};

    fn literal(text: &str) -> Vec<&str> {
        spans(text).into_iter().map(|part| &text[part]).collect()
    }

    #[test]
    fn the_literal_parts_of_a_sentence_are_found() {
        // Options, as git names them in what it says.
        assert_eq!(
            literal("hint: use --force-with-lease, or --force."),
            ["--force-with-lease", "--force"]
        );
        assert_eq!(literal("try --3way"), ["--3way"]);
        assert_eq!(literal("--abort"), ["--abort"]);
        assert_eq!(
            literal("(see --set-upstream-to=origin/main: it is not set)"),
            ["--set-upstream-to=origin/main"]
        );
        // Hyphens that are no option.
        assert!(literal("a--b and --- and -- alone and well-known").is_empty());
        // Between backticks, and between git's single quotes.
        assert_eq!(
            literal("run `git gc --prune=now` and then 'git add'."),
            ["git gc --prune=now", "git add"]
        );
        assert_eq!(
            literal("error: pathspec 'no such file.txt' did not match"),
            ["no such file.txt"]
        );
        assert_eq!(
            literal("fatal: 'origin/main' is not a commit: 'x'"),
            ["origin/main", "x"]
        );
        // An apostrophe is not a quote; a quote that never closes is none.
        assert!(literal("the user's file and the others' files").is_empty());
        assert!(literal("it's 'unfinished").is_empty());
        assert!(literal("don't and won't").is_empty());
        assert!(literal("an empty `` is nothing").is_empty());
        // Our own quotation marks are for labels, not for literal text.
        assert!(literal("\u{201c}New tab\u{201d} has no key now").is_empty());
        // A quoted sentence is a sentence.
        let long = format!("'{}'", "word ".repeat(60));
        assert!(literal(&long).is_empty());
        assert!(literal("").is_empty());
    }

    #[test]
    fn the_parts_fall_on_whole_letters_and_apart() {
        let many = "--a ".repeat(2000);
        for text in [
            "größe 'ß' --força `日本`",
            "'😀' --x😀",
            "--a--b 'c' `d`",
            "\u{0}'\u{0}' --\u{0}",
            "' ' ` ` - -- --- ----",
            many.as_str(),
        ] {
            let mut end = 0usize;
            for part in spans(text) {
                assert!(part.start >= end && part.start < part.end, "{text:?}");
                assert!(
                    text.is_char_boundary(part.start) && text.is_char_boundary(part.end),
                    "{text:?}"
                );
                end = part.end;
            }
            assert!(end <= text.len());
        }
    }

    #[test]
    fn every_joining_feature_is_switched_off_by_a_tag_a_font_knows() {
        let features = features();
        let list = features.tag_value_list();
        assert_eq!(list.len(), JOINING.len());
        for (tag, value) in list {
            // A tag of any other length makes the font fail to load.
            assert_eq!(tag.len(), 4, "{tag}");
            assert!(tag.bytes().all(|byte| byte.is_ascii_lowercase()), "{tag}");
            assert_eq!(*value, 0, "{tag}");
        }
        // The one the toolkit's own switch turns off is among them.
        assert_eq!(features.is_calt_enabled(), Some(false));
    }
}
