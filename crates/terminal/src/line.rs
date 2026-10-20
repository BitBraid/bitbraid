//! The command line being typed at a shell's prompt, read off the screen.
//!
//! A terminal is not told where a shell's prompt ends and what is typed
//! after it begins. The window knows when typing begins: when the first
//! key of a line is typed while the shell itself is in front, what stands
//! before the cursor on its line is the prompt ([`Prompt`], taken by
//! `Session::prompt_here`). From then on, the line being typed is what
//! follows that prompt on the line the cursor is on
//! (`Session::typed_line`); a line longer than the screen is wide goes on
//! in the rows below, which the screen marks as wrapped, and is read as
//! one. When the cursor's line does not begin with the prompt any more
//! (the shell printed another, a program took the screen) there is no
//! line.
//!
//! Whether the shell is in front: on Unix the terminal's foreground
//! process group is the shell's own, not that of a program it started;
//! a program on the alternate screen (an editor, a pager) is never the
//! shell, whatever runs it.
//!
//! Only the screen is read: nothing of the shell's configuration or its
//! own completion runs.
//!
//! Sources: POSIX `tcgetpgrp(3)` and the General Terminal Interface
//! (the foreground process group); `alacritty_terminal` 0.26, `grid`
//! (`Flags::WRAPLINE`, the spacers of wide characters, `input_needs_wrap`);
//! proc(5) (`/proc/<pid>/cwd`).

use crate::ViewPoint;

/// The most rows one line is read across.
pub const MOST_ROWS: usize = 64;

/// The prompt a command line is typed after: the text before the cursor
/// on its line when the first key of the line was typed.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
pub struct Prompt {
    text: String,
}

impl Prompt {
    /// A prompt that reads `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self { text: text.into() }
    }

    /// What it reads, blanks at its end included.
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// A command line being typed after a prompt.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TypedLine {
    /// What is typed before the cursor.
    pub before: String,
    /// What stands after the cursor on the line, blanks at the end left
    /// out.
    pub after: String,
    /// Where the cursor is on the screen.
    pub cursor: ViewPoint,
}

/// One row of the screen as the line is read from it.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenRow {
    /// Its characters with the column each stands in; the second half of
    /// a wide character is not one.
    pub cells: Vec<(u16, char)>,
    /// Whether the row goes on in the next one: a line longer than the
    /// screen is wide.
    pub wraps: bool,
}

/// The line through the cursor, as the text before it and after it.
/// `past_cursor` says the cursor stands past the last column (the next
/// character goes on the next row), so the character under it is typed
/// already. None when the cursor's row is not on the screen.
pub fn line_at(
    rows: &[ScreenRow],
    cursor_row: usize,
    cursor_column: u16,
    past_cursor: bool,
) -> Option<(String, String)> {
    rows.get(cursor_row)?;
    // Up while the row above wraps into this one.
    let mut first = cursor_row;
    while first > 0 && cursor_row.saturating_sub(first) < MOST_ROWS {
        let above = first.saturating_sub(1);
        if rows.get(above).is_some_and(|row| row.wraps) {
            first = above;
        } else {
            break;
        }
    }
    let mut before = String::new();
    let mut after = String::new();
    for row in rows.get(first..cursor_row).unwrap_or_default() {
        before.extend(row.cells.iter().map(|(_, c)| *c));
    }
    let row = rows.get(cursor_row)?;
    for (column, c) in &row.cells {
        if *column < cursor_column || (past_cursor && *column == cursor_column) {
            before.push(*c);
        } else {
            after.push(*c);
        }
    }
    // Down while this row wraps into the next one.
    let mut at = cursor_row;
    while rows.get(at).is_some_and(|row| row.wraps) && at.saturating_sub(cursor_row) < MOST_ROWS {
        at = at.saturating_add(1);
        match rows.get(at) {
            Some(next) => after.extend(next.cells.iter().map(|(_, c)| *c)),
            None => break,
        }
    }
    after.truncate(after.trim_end().len());
    Some((before, after))
}

/// The line typed after `prompt`, from the text of the line through the
/// cursor: none when the line does not begin with it.
pub fn typed_after(
    prompt: &Prompt,
    before: &str,
    after: &str,
    cursor: ViewPoint,
) -> Option<TypedLine> {
    let typed = before.strip_prefix(prompt.text())?;
    Some(TypedLine {
        before: typed.to_owned(),
        after: after.to_owned(),
        cursor,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(text: &str, wraps: bool) -> ScreenRow {
        ScreenRow {
            cells: text
                .chars()
                .enumerate()
                .map(|(column, c)| (u16::try_from(column).unwrap(), c))
                .collect(),
            wraps,
        }
    }

    #[test]
    fn the_line_through_the_cursor_is_split_at_it() {
        let rows = vec![
            row("output of before", false),
            row("$ git checkout ma   ", false),
            row("", false),
        ];
        let (before, after) = line_at(&rows, 1, 17, false).unwrap();
        assert_eq!(before, "$ git checkout ma");
        assert_eq!(after, "");
        let (before, after) = line_at(&rows, 1, 6, false).unwrap();
        assert_eq!(before, "$ git ");
        assert_eq!(after, "checkout ma");
        assert_eq!(line_at(&rows, 9, 0, false), None);
    }

    #[test]
    fn a_line_wider_than_the_screen_is_read_as_one() {
        let rows = vec![
            row("no", false),
            row("$ git add some/", true),
            row("long/path/to/a/", true),
            row("file.txt rest", false),
            row("next", false),
        ];
        let (before, after) = line_at(&rows, 3, 8, false).unwrap();
        assert_eq!(before, "$ git add some/long/path/to/a/file.txt");
        assert_eq!(after, " rest");
        // From the middle row, the rows after it are what follows.
        let (before, after) = line_at(&rows, 2, 4, false).unwrap();
        assert_eq!(before, "$ git add some/long");
        assert_eq!(after, "/path/to/a/file.txt rest");
    }

    #[test]
    fn a_cursor_past_the_last_column_has_its_cell_before_it() {
        let rows = vec![row("$ git sta", false)];
        let (before, _) = line_at(&rows, 0, 8, true).unwrap();
        assert_eq!(before, "$ git sta");
        let (before, after) = line_at(&rows, 0, 8, false).unwrap();
        assert_eq!((before.as_str(), after.as_str()), ("$ git st", "a"));
    }

    #[test]
    fn a_line_only_counts_after_its_prompt() {
        let prompt = Prompt::new("[me@here repo]$ ");
        let at = ViewPoint { row: 3, column: 22 };
        let typed = typed_after(&prompt, "[me@here repo]$ git c", "", at).unwrap();
        assert_eq!(typed.before, "git c");
        assert_eq!(typed.cursor, at);
        assert_eq!(typed_after(&prompt, "> git c", "", at), None);
        assert_eq!(typed_after(&prompt, "[me@here repo]$", "", at), None);
    }

    #[test]
    fn rows_that_all_wrap_are_read_no_further_than_the_most() {
        let rows: Vec<ScreenRow> = (0..300).map(|_| row("ab", true)).collect();
        let (before, after) = line_at(&rows, 150, 1, false).unwrap();
        assert_eq!(before.len(), MOST_ROWS * 2 + 1);
        assert_eq!(after.len(), 1 + MOST_ROWS * 2);
    }
}
