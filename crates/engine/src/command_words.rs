//! The words of a command written for a program of the user's own: an
//! editor, a file manager, a terminal.
//!
//! No shell runs such a command, so nothing in a path can turn into
//! syntax. Its words are split the way a shell splits them all the same,
//! so that a command means what it would mean written for a shell, and
//! for git's diff tool, whose command a shell runs: blanks part words,
//! and quotes and backslashes keep blanks and quotes in a word and are
//! taken off. `'…'` keeps everything in it as it is; `"…"` everything but
//! a backslash before `"`, `\`, `$` or a backquote, which stands for that
//! character; a backslash outside quotes keeps the character after it.
//! On Windows, where `\` parts the folders of a path (`C:\tools\vim.exe`),
//! a backslash outside quotes is a character like the others unless a
//! blank, a quote or another backslash comes after it.
//! Nothing is expanded: `$`, `*`, `~`, `;` and `|` are characters like
//! the others. A quote that is not closed is refused, not guessed at.
//!
//! The placeholders (`{path}`, `{line}`) are filled in afterwards, in a
//! word quoted or not: what a placeholder stands for is one word, or one
//! part of a word, whatever it holds, blanks and quotes included, and it
//! is not looked into again (a path with `{line}` in its name keeps it).
//!
//! Sources: POSIX.1-2017 Shell Command Language, 2.2 "Quoting" (escape
//! character, single quotes, double quotes) and 2.3 "Token Recognition";
//! git-difftool(1) (`difftool.<tool>.cmd`, run by the shell).

use std::{
    collections::BTreeSet,
    ffi::{OsStr, OsString},
};

use crate::command::CommandError;

/// Where the file (or folder) goes.
pub const PATH: &str = "{path}";
/// Where the line goes.
pub const LINE: &str = "{line}";

fn refused(detail: &str) -> CommandError {
    CommandError::InvalidParam {
        name: "command".to_owned(),
        detail: detail.to_owned(),
    }
}

/// The words of a command, quotes and backslashes taken off.
pub fn split(command: &str) -> Result<Vec<String>, CommandError> {
    let mut words = Vec::new();
    // Nothing between words; a word begun by quotes may be empty.
    let mut word: Option<String> = None;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err(refused(say!("command-words-single-quote-open"))),
                    }
                }
            }
            '"' => {
                let word = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err(refused(say!("command-words-double-quote-open"))),
                        },
                        Some(c) => word.push(c),
                        None => return Err(refused(say!("command-words-double-quote-open"))),
                    }
                }
            }
            '\\' => match chars.next() {
                Some(c)
                    if cfg!(windows) && !(c.is_whitespace() || matches!(c, '\'' | '"' | '\\')) =>
                {
                    let word = word.get_or_insert_with(String::new);
                    word.push('\\');
                    word.push(c);
                }
                Some(c) => word.get_or_insert_with(String::new).push(c),
                None => return Err(refused(say!("command-words-ends-in-backslash"))),
            },
            c if c.is_whitespace() => {
                if let Some(word) = word.take() {
                    words.push(word);
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    words.extend(word);
    Ok(words)
}

/// The words of a command with its placeholders filled in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Filled {
    /// The program, then its arguments.
    pub words: Vec<OsString>,
    /// The placeholders that stood in the command.
    pub placed: BTreeSet<&'static str>,
}

/// Split a command ([`split`]) and fill in each placeholder of `values`
/// wherever it stands in a word.
pub fn fill(command: &str, values: &[(&'static str, &OsStr)]) -> Result<Filled, CommandError> {
    let mut placed = BTreeSet::new();
    let words = split(command)?
        .iter()
        .map(|word| {
            let mut out = OsString::new();
            let mut rest = word.as_str();
            loop {
                // The placeholder that comes first in what is left.
                let next = values
                    .iter()
                    .filter_map(|(name, value)| rest.find(name).map(|at| (at, *name, *value)))
                    .min_by_key(|(at, _, _)| *at);
                let Some((at, name, value)) = next else {
                    out.push(rest);
                    break;
                };
                out.push(rest.get(..at).unwrap_or_default());
                out.push(value);
                placed.insert(name);
                rest = rest
                    .get(at.saturating_add(name.len())..)
                    .unwrap_or_default();
            }
            out
        })
        .collect();
    Ok(Filled { words, placed })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn said(command: &str) -> Vec<String> {
        split(command).unwrap_or_else(|err| panic!("{command:?}: {err}"))
    }

    fn filled(command: &str, path: &str, line: &str) -> Vec<String> {
        fill(command, &[(PATH, path.as_ref()), (LINE, line.as_ref())])
            .unwrap_or_else(|err| panic!("{command:?}: {err}"))
            .words
            .iter()
            .map(|word| word.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn words_are_split_as_a_shell_splits_them() {
        assert_eq!(said("  vim  -p\tx "), ["vim", "-p", "x"]);
        assert_eq!(said("'a b' \"c d\" e\\ f"), ["a b", "c d", "e f"]);
        assert_eq!(said("x'y'\"z\"w"), ["xyzw"]);
        assert_eq!(said("'it''s' \"say \\\"hi\\\"\""), ["its", "say \"hi\""]);
        assert_eq!(said(r#""\$HOME \` \\ \n" '\n'"#), ["$HOME ` \\ \\n", "\\n"]);
        // Quotes with nothing in them are an empty word.
        assert_eq!(said("ed '' \"\""), ["ed", "", ""]);
        // Nothing is expanded or taken for syntax.
        assert_eq!(
            said("a $HOME ~ * ; | &&"),
            ["a", "$HOME", "~", "*", ";", "|", "&&"]
        );
        assert!(said("").is_empty() && said("   ").is_empty());
    }

    #[test]
    fn a_backslash_in_a_path_of_windows_is_kept_there() {
        let said_for_tools = said(r"C:\tools\vim.exe +{line} {path}");
        if cfg!(windows) {
            assert_eq!(said_for_tools, [r"C:\tools\vim.exe", "+{line}", "{path}"]);
        } else {
            assert_eq!(said_for_tools, ["C:toolsvim.exe", "+{line}", "{path}"]);
        }
        // Before a blank, a quote or a backslash it keeps that everywhere;
        // before anything else only on Windows is it kept itself.
        assert_eq!(said(r"a\ b e\'f g\\h"), ["a b", "e'f", r"g\h"]);
        let kept = if cfg!(windows) { r"c\d" } else { "cd" };
        assert_eq!(said(r"c\d"), [kept]);
        assert_eq!(
            said(r#""C:\Program Files\Tool\tool.exe" {path}"#),
            [r"C:\Program Files\Tool\tool.exe", "{path}"]
        );
    }

    #[test]
    fn what_is_not_closed_is_refused() {
        for bad in ["ed 'x", "ed \"x", "ed \"x\\\"", "ed x\\", "'"] {
            assert!(split(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_placeholder_is_one_word_quoted_or_not() {
        let path = "/r/a b's \"c\".rs";
        let want = ["tool", "editor", path, "7"];
        assert_eq!(filled("tool editor {path} {line}", path, "7"), want);
        assert_eq!(filled("tool editor \"{path}\" \"{line}\"", path, "7"), want);
        assert_eq!(filled("tool editor '{path}' '{line}'", path, "7"), want);
        assert_eq!(
            filled("code --goto \"{path}:{line}\"", path, "7"),
            ["code", "--goto", "/r/a b's \"c\".rs:7"]
        );
        assert_eq!(
            filled("sh -c 'exec ed \"$0\"' {path}", path, "7"),
            ["sh", "-c", "exec ed \"$0\"", path]
        );
        // What a value holds is not looked into again.
        assert_eq!(
            filled("ed {path} {line}", "/r/{line}{path}", "{path}"),
            ["ed", "/r/{line}{path}", "{path}"]
        );
        // Twice in a word, and which ones stood there.
        let done = fill(
            "x {path}{path}",
            &[(PATH, "p".as_ref()), (LINE, "1".as_ref())],
        )
        .unwrap_or_else(|err| panic!("{err}"));
        assert_eq!(done.words, ["x", "pp"]);
        assert_eq!(done.placed.into_iter().collect::<Vec<_>>(), [PATH]);
    }
}
