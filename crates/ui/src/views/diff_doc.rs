//! The rows a diff is shown as, and the word-level comparison of the lines
//! a change replaces.
//!
//! A diff arrives as hunks of context, removed and added lines. The viewer
//! shows them as rows of a virtual list in one of three layouts: only the
//! hunks, each under its heading; the whole file in one column; or the
//! whole file in two, a row per pair of old and new lines. (For the whole
//! file the diff is asked for with every line as context: it is then one
//! hunk, shown without its heading.) The file view is the lines of one
//! side alone, without marks. Inside a run of removed lines followed by
//! added lines, the n-th removed line is compared with the n-th added one
//! token by token, and what differs is marked for emphasis, unless the two
//! have too little in common to be the same line edited.
//!
//! Sources: the longest common subsequence as in any algorithms text
//! (original implementation).

use std::ops::Range;

use bitbraid_git::{FileDiff, Hunk, Line, LineKind};

/// Two lines are compared word by word only below this many token pairs;
/// longer ones (minified code, data) are shown without emphasis.
const MAX_TOKEN_PAIRS: usize = 60_000;

/// Least share, in percent, of a line's visible text that has to survive
/// the edit for the two lines to count as one line edited.
const MIN_COMMON_PERCENT: usize = 30;

/// Byte ranges of a line's text that changed.
pub type Emphasis = Vec<Range<usize>>;

/// Layout of a diff.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    /// Only the changed parts, each under the heading of its hunk, in one
    /// column.
    Hunks,
    /// The whole file in one column, each change's removed lines above its
    /// added ones.
    Inline,
    /// The whole file, old and new side by side.
    Split,
}

impl Layout {
    /// Whether the layout shows the whole file rather than its hunks.
    pub fn whole(self) -> bool {
        self != Layout::Hunks
    }
}

/// One line on one side of a split row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    /// Index of the line in its hunk.
    pub line: usize,
    /// Byte ranges of the text that changed against the paired line.
    pub emphasis: Vec<Range<usize>>,
}

/// One row of the viewer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    /// The header of hunk `hunk`.
    Hunk {
        /// Hunk index.
        hunk: usize,
    },
    /// One line of the inline layout.
    Line {
        /// Hunk index.
        hunk: usize,
        /// The line and its emphasis.
        cell: Cell,
    },
    /// Old and new line of the split layout; a side is absent where the
    /// other side has more lines.
    Pair {
        /// Hunk index.
        hunk: usize,
        /// Old side.
        left: Option<Cell>,
        /// New side.
        right: Option<Cell>,
    },
    /// A line of the file view: the file as it is on one side, unmarked.
    Plain {
        /// Hunk index.
        hunk: usize,
        /// The line (its emphasis is none).
        cell: Cell,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Class {
    Word,
    Space,
    Other,
}

fn class_of(ch: char) -> Class {
    if ch.is_alphanumeric() || ch == '_' {
        Class::Word
    } else if ch.is_whitespace() {
        Class::Space
    } else {
        Class::Other
    }
}

/// Split a line into tokens: runs of word characters, runs of whitespace,
/// and every other character on its own.
fn tokens(text: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut chars = text.char_indices().peekable();
    while let Some((start, ch)) = chars.next() {
        let class = class_of(ch);
        let mut end = start + ch.len_utf8();
        if class != Class::Other {
            while let Some((at, next)) = chars.peek().copied() {
                if class_of(next) != class {
                    break;
                }
                end = at + next.len_utf8();
                chars.next();
            }
        }
        out.push(start..end);
    }
    out
}

/// Byte ranges of `tokens` not marked as kept, with neighbours joined.
fn changed_ranges(tokens: &[Range<usize>], kept: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (token, kept) in tokens.iter().zip(kept) {
        if *kept {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == token.start => last.end = token.end,
            _ => out.push(token.clone()),
        }
    }
    out
}

/// Compare two lines token by token. Returns the byte ranges that changed
/// in the old and in the new line, or `None` when the lines are too long to
/// compare or have too little in common to be one line edited.
pub fn word_diff(old: &str, new: &str) -> Option<(Emphasis, Emphasis)> {
    let old_tokens = tokens(old);
    let new_tokens = tokens(new);
    let (n, m) = (old_tokens.len(), new_tokens.len());
    if n.saturating_mul(m) > MAX_TOKEN_PAIRS {
        return None;
    }
    let text = |source: &str, range: &Range<usize>| source.get(range.clone()).map(str::to_owned);
    let old_text: Vec<Option<String>> = old_tokens.iter().map(|r| text(old, r)).collect();
    let new_text: Vec<Option<String>> = new_tokens.iter().map(|r| text(new, r)).collect();

    // Longest common subsequence, by the table of suffix lengths.
    let width = m + 1;
    let mut table = vec![0u32; (n + 1) * width];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            let value = if old_text.get(i) == new_text.get(j) {
                table.get((i + 1) * width + j + 1).copied().unwrap_or(0) + 1
            } else {
                let down = table.get((i + 1) * width + j).copied().unwrap_or(0);
                let right = table.get(i * width + j + 1).copied().unwrap_or(0);
                down.max(right)
            };
            if let Some(slot) = table.get_mut(i * width + j) {
                *slot = value;
            }
        }
    }
    let mut old_kept = vec![false; n];
    let mut new_kept = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if old_text.get(i) == new_text.get(j) {
            if let (Some(a), Some(b)) = (old_kept.get_mut(i), new_kept.get_mut(j)) {
                *a = true;
                *b = true;
            }
            i += 1;
            j += 1;
        } else {
            let down = table.get((i + 1) * width + j).copied().unwrap_or(0);
            let right = table.get(i * width + j + 1).copied().unwrap_or(0);
            if down >= right {
                i += 1;
            } else {
                j += 1;
            }
        }
    }

    // Whitespace that survived says nothing about the lines being related.
    let visible = |source: &str, tokens: &[Range<usize>], kept: Option<&[bool]>| -> usize {
        tokens
            .iter()
            .enumerate()
            .filter(|(ix, _)| kept.is_none_or(|kept| kept.get(*ix).copied().unwrap_or(false)))
            .filter_map(|(_, range)| source.get(range.clone()))
            .filter(|token| !token.chars().all(char::is_whitespace))
            .map(str::len)
            .sum()
    };
    let total = visible(old, &old_tokens, None).max(visible(new, &new_tokens, None));
    let common = visible(old, &old_tokens, Some(&old_kept));
    if total > 0 && common * 100 < total * MIN_COMMON_PERCENT {
        return None;
    }
    Some((
        changed_ranges(&old_tokens, &old_kept),
        changed_ranges(&new_tokens, &new_kept),
    ))
}

/// Emphasis for a run of removed lines followed by added lines: the n-th
/// removed line against the n-th added one. Indexed like the inputs.
fn pair_emphasis(
    removed: &[&Line],
    added: &[&Line],
    words: bool,
) -> (Vec<Emphasis>, Vec<Emphasis>) {
    let mut left = vec![Vec::new(); removed.len()];
    let mut right = vec![Vec::new(); added.len()];
    if words {
        for (ix, (old, new)) in removed.iter().zip(added).enumerate() {
            if let Some((old_ranges, new_ranges)) = word_diff(&old.text, &new.text) {
                if let Some(slot) = left.get_mut(ix) {
                    *slot = old_ranges;
                }
                if let Some(slot) = right.get_mut(ix) {
                    *slot = new_ranges;
                }
            }
        }
    }
    (left, right)
}

fn hunk_rows(hunk_ix: usize, hunk: &Hunk, layout: Layout, words: bool, rows: &mut Vec<Row>) {
    // The whole file has no headings: it is one hunk, from the first line.
    if layout == Layout::Hunks {
        rows.push(Row::Hunk { hunk: hunk_ix });
    }
    let lines = &hunk.lines;
    let mut at = 0;
    while let Some(line) = lines.get(at) {
        if line.kind == LineKind::Context {
            let cell = Cell {
                line: at,
                emphasis: Vec::new(),
            };
            rows.push(match layout {
                Layout::Hunks | Layout::Inline => Row::Line {
                    hunk: hunk_ix,
                    cell,
                },
                Layout::Split => Row::Pair {
                    hunk: hunk_ix,
                    left: Some(cell.clone()),
                    right: Some(cell),
                },
            });
            at += 1;
            continue;
        }
        // A run of changes: removed and added lines up to the next context.
        let mut removed: Vec<usize> = Vec::new();
        let mut added: Vec<usize> = Vec::new();
        while let Some(line) = lines.get(at) {
            match line.kind {
                LineKind::Removed => removed.push(at),
                LineKind::Added => added.push(at),
                LineKind::Context => break,
            }
            at += 1;
        }
        let pick = |indexes: &[usize]| -> Vec<&Line> {
            indexes.iter().filter_map(|ix| lines.get(*ix)).collect()
        };
        let (left, right) = pair_emphasis(&pick(&removed), &pick(&added), words);
        let cell = |indexes: &[usize], emphasis: &[Vec<Range<usize>>], n: usize| {
            Some(Cell {
                line: *indexes.get(n)?,
                emphasis: emphasis.get(n).cloned().unwrap_or_default(),
            })
        };
        match layout {
            Layout::Hunks | Layout::Inline => {
                // In the order of the hunk, which may interleave the two.
                let mut all: Vec<(usize, Cell)> = (0..removed.len())
                    .filter_map(|n| cell(&removed, &left, n))
                    .chain((0..added.len()).filter_map(|n| cell(&added, &right, n)))
                    .map(|cell| (cell.line, cell))
                    .collect();
                all.sort_by_key(|(line, _)| *line);
                rows.extend(all.into_iter().map(|(_, cell)| Row::Line {
                    hunk: hunk_ix,
                    cell,
                }));
            }
            Layout::Split => {
                for n in 0..removed.len().max(added.len()) {
                    rows.push(Row::Pair {
                        hunk: hunk_ix,
                        left: cell(&removed, &left, n),
                        right: cell(&added, &right, n),
                    });
                }
            }
        }
    }
}

/// The rows of a diff in a layout, with or without word-level emphasis.
pub fn rows(diff: &FileDiff, layout: Layout, words: bool) -> Vec<Row> {
    let mut rows = Vec::new();
    for (hunk_ix, hunk) in diff.hunks.iter().enumerate() {
        hunk_rows(hunk_ix, hunk, layout, words, &mut rows);
    }
    rows
}

/// The rows of the file view: the lines of the new side as they are, or
/// of the old side for a file that is not there any more.
pub fn file_rows(diff: &FileDiff) -> Vec<Row> {
    let side = |kept: LineKind| -> Vec<Row> {
        diff.hunks
            .iter()
            .enumerate()
            .flat_map(|(hunk_ix, hunk)| {
                hunk.lines
                    .iter()
                    .enumerate()
                    .filter(move |(_, line)| line.kind != kept)
                    .map(move |(line, _)| Row::Plain {
                        hunk: hunk_ix,
                        cell: Cell {
                            line,
                            emphasis: Vec::new(),
                        },
                    })
            })
            .collect()
    };
    let new_side = side(LineKind::Removed);
    if new_side.is_empty() {
        side(LineKind::Added)
    } else {
        new_side
    }
}

/// Indexes of the rows where a run of changed lines begins: the places
/// "next change" and "previous change" stop at.
pub fn change_starts(diff: &FileDiff, rows: &[Row]) -> Vec<usize> {
    let changed = |hunk: usize, cell: &Option<Cell>| match cell {
        // A gap on one side is part of a change.
        None => true,
        Some(cell) => diff
            .hunks
            .get(hunk)
            .and_then(|hunk| hunk.lines.get(cell.line))
            .is_some_and(|line| line.kind != LineKind::Context),
    };
    let mut out = Vec::new();
    let mut inside = false;
    for (ix, row) in rows.iter().enumerate() {
        let is_change = match row {
            Row::Hunk { .. } | Row::Plain { .. } => false,
            Row::Line { hunk, cell } => changed(*hunk, &Some(cell.clone())),
            Row::Pair { hunk, left, right } => changed(*hunk, left) || changed(*hunk, right),
        };
        if is_change && !inside {
            out.push(ix);
        }
        inside = is_change;
    }
    out
}

/// What a run of rows is to the minimap: the side its lines are on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// Lines taken out: the map's left half.
    Removed,
    /// Lines put in: the map's right half.
    Added,
}

/// The runs of changed rows, as (first row, number of rows, which side),
/// by the row they start at and removed before added. A row of the split
/// layout with a change on both sides is in a run of each. This is all
/// the minimap draws, so a diff of a hundred thousand lines with three
/// changes costs a handful of rectangles.
pub fn marks(diff: &FileDiff, rows: &[Row]) -> Vec<(usize, usize, Mark)> {
    let kind = |hunk: usize, cell: &Option<Cell>| {
        cell.as_ref()
            .and_then(|cell| diff.hunks.get(hunk)?.lines.get(cell.line))
            .map(|line| line.kind)
            .filter(|kind| *kind != LineKind::Context)
    };
    let mark_of = |kind: Option<LineKind>| match kind {
        Some(LineKind::Added) => Some(Mark::Added),
        Some(LineKind::Removed) => Some(Mark::Removed),
        _ => None,
    };
    let mut out: Vec<(usize, usize, Mark)> = Vec::new();
    // The run of each side that is still open: its index in `out`.
    let mut open: [Option<usize>; 2] = [None, None];
    for (ix, row) in rows.iter().enumerate() {
        let on: [Option<Mark>; 2] = match row {
            Row::Hunk { .. } | Row::Plain { .. } => [None, None],
            Row::Line { hunk, cell } => {
                let mark = mark_of(kind(*hunk, &Some(cell.clone())));
                [
                    mark.filter(|mark| *mark == Mark::Removed),
                    mark.filter(|mark| *mark == Mark::Added),
                ]
            }
            Row::Pair { hunk, left, right } => [
                mark_of(kind(*hunk, left)).filter(|mark| *mark == Mark::Removed),
                mark_of(kind(*hunk, right)).filter(|mark| *mark == Mark::Added),
            ],
        };
        for (side, mark) in on.into_iter().enumerate() {
            let Some(slot) = open.get_mut(side) else {
                continue;
            };
            match (mark, *slot) {
                (Some(_), Some(at)) => {
                    if let Some(run) = out.get_mut(at) {
                        run.1 = run.1.saturating_add(1);
                    }
                }
                (Some(mark), None) => {
                    *slot = Some(out.len());
                    out.push((ix, 1, mark));
                }
                (None, _) => *slot = None,
            }
        }
    }
    out.sort();
    out
}

/// A line of `len` bytes cut where it wraps: the byte ranges of the
/// pieces it is shown as, in order and covering all of it. A place to cut
/// that is not inside the line, or not after the one before it, is passed
/// over.
pub fn pieces(len: usize, cuts: &[usize]) -> Vec<Range<usize>> {
    let mut out = Vec::with_capacity(cuts.len().saturating_add(1));
    let mut start = 0;
    for cut in cuts {
        if *cut > start && *cut < len {
            out.push(start..*cut);
            start = *cut;
        }
    }
    out.push(start..len);
    out
}

/// The part of some spans of a line (its colours, its emphasis) that
/// falls into one piece of it, counted from where the piece starts.
pub fn cut_spans<T: Clone>(
    spans: &[(Range<usize>, T)],
    piece: &Range<usize>,
) -> Vec<(Range<usize>, T)> {
    spans
        .iter()
        .filter(|(range, _)| range.start < piece.end && range.end > piece.start)
        .map(|(range, what)| {
            let from = range.start.max(piece.start).saturating_sub(piece.start);
            let to = range.end.min(piece.end).saturating_sub(piece.start);
            (from..to, what.clone())
        })
        .filter(|(range, _)| range.start < range.end)
        .collect()
}

/// Length in characters of the longest line, for the horizontal extent.
pub fn longest_line(diff: &FileDiff) -> usize {
    diff.hunks
        .iter()
        .flat_map(|hunk| &hunk.lines)
        .map(|line| line.text.chars().count())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use bitbraid_git::{FileDiff, LineKind, diff::FileChangeKind};

    use super::{
        Layout, Mark, Row, change_starts, cut_spans, file_rows, longest_line, marks, pieces, rows,
        tokens, word_diff,
    };

    #[test]
    fn a_wrapped_line_is_cut_into_pieces_that_cover_it() {
        assert_eq!(pieces(10, &[]), vec![0..10; 1]);
        assert_eq!(pieces(10, &[4, 8]), [0..4, 4..8, 8..10]);
        // Cuts that are nowhere in the line, or out of order, are not cuts.
        assert_eq!(pieces(10, &[0, 4, 4, 3, 10, 99]), [0..4, 4..10]);
        assert_eq!(pieces(0, &[3]), vec![0..0; 1]);
        for cuts in [vec![], vec![1], vec![2, 5, 7], vec![9, 1]] {
            let cut = pieces(9, &cuts);
            assert_eq!(cut.first().map(|piece| piece.start), Some(0));
            assert_eq!(cut.last().map(|piece| piece.end), Some(9));
            assert!(cut.windows(2).all(|pair| pair[0].end == pair[1].start));
        }
    }

    #[test]
    fn spans_are_cut_to_a_piece_and_counted_from_its_start() {
        let spans = vec![(0..3, 'a'), (3..9, 'b'), (12..14, 'c')];
        assert_eq!(cut_spans(&spans, &(0..5)), [(0..3, 'a'), (3..5, 'b')]);
        assert_eq!(cut_spans(&spans, &(5..10)), [(0..4, 'b')]);
        assert_eq!(cut_spans(&spans, &(10..20)), [(2..4, 'c')]);
        assert!(cut_spans(&spans, &(9..12)).is_empty());
        assert!(cut_spans(&spans, &(5..5)).is_empty());
        // Every byte of a span lands in exactly one piece.
        let cut = pieces(14, &[5, 10]);
        let total: usize = cut
            .iter()
            .flat_map(|piece| cut_spans(&spans, piece))
            .map(|(range, _)| range.len())
            .sum();
        assert_eq!(total, 3 + 6 + 2);
    }

    fn marked(text: &str, ranges: &[std::ops::Range<usize>]) -> String {
        let mut out = String::new();
        for (ix, ch) in text.char_indices() {
            if ranges.iter().any(|r| r.start == ix) {
                out.push('[');
            }
            out.push(ch);
            if ranges.iter().any(|r| r.end == ix + ch.len_utf8()) {
                out.push(']');
            }
        }
        out
    }

    fn words(old: &str, new: &str) -> Option<(String, String)> {
        word_diff(old, new).map(|(a, b)| (marked(old, &a), marked(new, &b)))
    }

    #[test]
    fn tokens_are_words_spaces_and_single_marks() {
        let text = "let x_1 = foo(a,  b);";
        let parts: Vec<&str> = tokens(text).into_iter().map(|r| &text[r]).collect();
        assert_eq!(
            parts,
            [
                "let", " ", "x_1", " ", "=", " ", "foo", "(", "a", ",", "  ", "b", ")", ";"
            ]
        );
        assert!(tokens("").is_empty());
    }

    #[test]
    fn an_edited_word_is_the_only_thing_marked() {
        assert_eq!(
            words(
                "    println!(\"hello\");",
                "    println!(\"hello, world\");"
            ),
            Some((
                "    println!(\"hello\");".to_owned(),
                "    println!(\"hello[, world]\");".to_owned()
            ))
        );
        assert_eq!(
            words(
                "input.split_whitespace().collect()",
                "input.split_ascii_whitespace().collect()"
            ),
            Some((
                "input.[split_whitespace]().collect()".to_owned(),
                "input.[split_ascii_whitespace]().collect()".to_owned()
            ))
        );
        assert_eq!(
            words("same", "same"),
            Some(("same".to_owned(), "same".to_owned()))
        );
    }

    #[test]
    fn unrelated_lines_get_no_emphasis() {
        assert_eq!(words("fn tokens(input: &str) {", "return 42;"), None);
        assert_eq!(words("use std::fmt;", "}"), None);
        // Half of the visible text surviving is enough to call it an edit.
        assert_eq!(
            words("    a = b;", "    x = y;"),
            Some(("    [a] = [b];".to_owned(), "    [x] = [y];".to_owned()))
        );
    }

    #[test]
    fn multibyte_text_keeps_ranges_on_character_boundaries() {
        let (old, new) = ("größe = 1 // ünïcödé", "größe = 2 // ünïcödé");
        let (a, b) = word_diff(old, new).unwrap();
        assert_eq!(marked(old, &a), "größe = [1] // ünïcödé");
        assert_eq!(marked(new, &b), "größe = [2] // ünïcödé");
        for range in a.iter().chain(&b) {
            assert!(old.is_char_boundary(range.start) && old.is_char_boundary(range.end));
        }
    }

    #[test]
    fn very_long_lines_are_not_compared() {
        let old = "a ".repeat(400);
        let new = "b ".repeat(400);
        assert_eq!(word_diff(&old, &new), None);
        // Whitespace-only lines have nothing visible to lose.
        assert!(word_diff("    ", "\t").is_some());
    }

    fn diff(lines: &[(char, &str)]) -> FileDiff {
        let hunks = bitbraid_git::diff_bytes(
            lines
                .iter()
                .filter(|(kind, _)| *kind != '+')
                .map(|(_, text)| format!("{text}\n"))
                .collect::<String>()
                .as_bytes(),
            lines
                .iter()
                .filter(|(kind, _)| *kind != '-')
                .map(|(_, text)| format!("{text}\n"))
                .collect::<String>()
                .as_bytes(),
            3,
        )
        .unwrap();
        FileDiff {
            kind: FileChangeKind::Modified,
            path: "file".to_owned(),
            old_path: None,
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            binary: false,
            hunks,
            additions: 0,
            deletions: 0,
            old_size: None,
            new_size: None,
            too_large: false,
            line_endings: Default::default(),
        }
    }

    /// Rows as text: `-`/`+`/space per side with the emphasis in brackets.
    fn sketch(diff: &FileDiff, layout: Layout, words: bool) -> Vec<String> {
        let side = |hunk: usize, cell: &Option<super::Cell>| match cell {
            Some(cell) => {
                let line = &diff.hunks[hunk].lines[cell.line];
                let mark = match line.kind {
                    LineKind::Context => ' ',
                    LineKind::Added => '+',
                    LineKind::Removed => '-',
                };
                format!("{mark}{}", marked(&line.text, &cell.emphasis))
            }
            None => "~".to_owned(),
        };
        rows(diff, layout, words)
            .iter()
            .map(|row| match row {
                Row::Hunk { hunk } => diff.hunks[*hunk].header(),
                Row::Line { hunk, cell } => side(*hunk, &Some(cell.clone())),
                Row::Pair { hunk, left, right } => {
                    format!("{} | {}", side(*hunk, left), side(*hunk, right))
                }
                Row::Plain { hunk, cell } => {
                    format!("={}", diff.hunks[*hunk].lines[cell.line].text)
                }
            })
            .collect()
    }

    #[test]
    fn the_minimap_gets_one_run_per_stretch_of_changes() {
        let small = sample();
        // Hunks: the removed line, then the two added ones (rows 2, 3-4),
        // after the heading and a line of context.
        let inline = rows(&small, Layout::Hunks, true);
        assert_eq!(
            marks(&small, &inline),
            [(2, 1, Mark::Removed), (3, 2, Mark::Added)]
        );
        // The whole file in one column: the same, without the heading.
        assert_eq!(
            marks(&small, &rows(&small, Layout::Inline, true)),
            [(1, 1, Mark::Removed), (2, 2, Mark::Added)]
        );
        // Split: one row changed on both sides, then one only on the new:
        // a run in the left half, and one of two rows in the right.
        let split = rows(&small, Layout::Split, true);
        assert_eq!(
            marks(&small, &split),
            [(1, 1, Mark::Removed), (1, 2, Mark::Added)]
        );
        // Two changes far apart are two runs, in both layouts.
        let mut lines: Vec<(char, String)> = (0..40).map(|n| (' ', format!("line {n}"))).collect();
        lines[5] = ('+', "new near the top".to_owned());
        lines[30] = ('-', "old near the end".to_owned());
        let borrowed: Vec<(char, &str)> = lines.iter().map(|(k, t)| (*k, t.as_str())).collect();
        let far = diff(&borrowed);
        for layout in [Layout::Hunks, Layout::Inline, Layout::Split] {
            let rows = rows(&far, layout, false);
            let found = marks(&far, &rows);
            assert_eq!(found.len(), 2, "{layout:?}");
            assert_eq!(found[0].2, Mark::Added);
            assert_eq!(found[1].2, Mark::Removed);
            assert!(found[0].0 < found[1].0);
        }
        // Nothing changed, nothing to draw; rows that are not this diff's
        // do not bring it down.
        assert!(marks(&small, &[]).is_empty());
        let empty = super::FileDiff {
            hunks: Vec::new(),
            ..sample()
        };
        assert!(marks(&empty, &inline).is_empty());
    }

    fn sample() -> FileDiff {
        diff(&[
            (' ', "fn main() {"),
            ('-', "    println!(\"hello\");"),
            ('+', "    println!(\"hello, world\");"),
            ('+', "    run();"),
            (' ', "}"),
        ])
    }

    #[test]
    fn inline_rows_follow_the_hunk_with_emphasis_on_paired_lines() {
        assert_eq!(
            sketch(&sample(), Layout::Hunks, true),
            [
                "@@ -1,3 +1,4 @@",
                " fn main() {",
                "-    println!(\"hello\");",
                "+    println!(\"hello[, world]\");",
                "+    run();",
                " }",
            ]
        );
        assert_eq!(
            sketch(&sample(), Layout::Hunks, false)[3],
            "+    println!(\"hello, world\");"
        );
        // The whole file in one column is the same rows without headings.
        assert_eq!(
            sketch(&sample(), Layout::Inline, true),
            sketch(&sample(), Layout::Hunks, true)[1..]
        );
    }

    #[test]
    fn the_file_view_is_one_side_as_it_is() {
        // The new side: context and added lines, in order, unmarked.
        let rows = file_rows(&sample());
        assert!(rows.iter().all(|row| matches!(row, Row::Plain { .. })));
        let text = |diff: &FileDiff, rows: &[Row]| -> Vec<String> {
            rows.iter()
                .map(|row| match row {
                    Row::Plain { hunk, cell } => diff.hunks[*hunk].lines[cell.line].text.clone(),
                    _ => String::new(),
                })
                .collect()
        };
        assert_eq!(
            text(&sample(), &rows),
            [
                "fn main() {",
                "    println!(\"hello, world\");",
                "    run();",
                "}"
            ]
        );
        // A file that is gone is shown as it was.
        let gone = diff(&[('-', "first"), ('-', "second")]);
        assert_eq!(text(&gone, &file_rows(&gone)), ["first", "second"]);
        // Nothing marks a change there.
        assert!(change_starts(&sample(), &rows).is_empty());
        assert!(marks(&sample(), &rows).is_empty());
    }

    #[test]
    fn split_rows_pair_the_sides_and_leave_gaps() {
        assert_eq!(
            sketch(&sample(), Layout::Split, true),
            [
                " fn main() { |  fn main() {",
                "-    println!(\"hello\"); | +    println!(\"hello[, world]\");",
                "~ | +    run();",
                " } |  }",
            ]
        );
    }

    #[test]
    fn changes_start_where_a_run_of_changed_rows_begins() {
        let diff = diff(&[
            ('-', "first"),
            (' ', "same 1"),
            (' ', "same 2"),
            ('-', "old"),
            ('+', "new"),
            ('+', "newer"),
            (' ', "same 3"),
            ('+', "last"),
        ]);
        for layout in [Layout::Hunks, Layout::Inline, Layout::Split] {
            let rows = rows(&diff, layout, true);
            let starts = change_starts(&diff, &rows);
            assert_eq!(starts.len(), 3, "{layout:?}");
            // Of the hunks row 0 is the heading; the first change follows.
            assert_eq!(starts[0], usize::from(layout == Layout::Hunks));
            assert!(starts.windows(2).all(|w| w[0] < w[1]));
        }
        assert!(change_starts(&diff, &[]).is_empty());
    }

    #[test]
    fn an_empty_diff_has_no_rows() {
        let none = diff(&[(' ', "same")]);
        assert!(rows(&none, Layout::Inline, true).is_empty());
        assert_eq!(longest_line(&none), 0);
        assert_eq!(longest_line(&sample()), 29);
    }
}
