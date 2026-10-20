//! Syntax colours for the lines of a diff.
//!
//! The colours of a line depend on what stands above it: a hunk that
//! starts inside a comment or a string is only read right by a parse that
//! saw where that began. So the two whole files are parsed, when the
//! viewer has them ([`highlight_whole`]): the old one colours the removed
//! lines, the new one the rest, each line found by its number. A file of
//! more than [`MAX_WHOLE_BYTES`] is not parsed whole.
//!
//! Without a file (it may be too large, or not be had) that side is
//! parsed from the hunks alone: all of them one after the other as one
//! text, the old side from the context and removed lines, the new side
//! from the context and added lines. Tree-sitter recovers from text that
//! starts or stops mid-construct, so this is right for the most part and
//! never worse than no colour.
//!
//! Either way a side is one parse. Making a highlighter costs tens of
//! milliseconds before any text is read, so one for every hunk would make
//! a diff of seven hundred hunks take over a minute; and the viewer has
//! the colours worked out off the thread that draws.
//!
//! Sources: gpui-kit documentation (`highlighter`); tree-sitter
//! documentation (error recovery).

use std::{collections::HashMap, ops::Range};

use bitbraid_git::{FileDiff, LineKind};
use gpui_kit::HighlightStyle;
use gpui_kit::component::{
    Rope,
    highlighter::{HighlightTheme, SyntaxHighlighter},
};

/// Diffs beyond this many lines stay plain: colour is not worth the wait.
const MAX_LINES: usize = 20_000;

/// A side of a file larger than this is not parsed whole; it is then
/// coloured from its hunks alone. Half a megabyte of Rust parses in a
/// fifth to a quarter of a second, which is waited for off the thread
/// that draws.
pub const MAX_WHOLE_BYTES: usize = 512 * 1024;

/// More hunks than this are coloured as one stretch of the text from the
/// first to the last, not one by one.
const MANY_HUNKS: usize = 8;

/// Colours of one line: byte ranges of its text, in order.
pub type LineStyles = Vec<(Range<usize>, HighlightStyle)>;

/// Colours by (hunk index, line index).
pub type Styles = HashMap<(usize, usize), LineStyles>;

/// The grammar for a path, by its extension or name.
pub fn language_for(path: &str) -> Option<&'static str> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let by_name = match name {
        // not shown: names of files
        "Cargo.lock" | "Pipfile" => Some("toml"),
        ".bashrc" | ".zshrc" | ".profile" | ".envrc" => Some("bash"),
        _ => None,
    };
    if by_name.is_some() {
        return by_name;
    }
    let (_, extension) = name.rsplit_once('.')?;
    Some(match extension.to_ascii_lowercase().as_str() {
        "rs" => "rust",
        "go" => "go",
        "js" | "mjs" | "cjs" | "jsx" => "javascript",
        "ts" | "mts" | "cts" => "typescript",
        "tsx" => "tsx",
        "py" | "pyi" => "python",
        "json" | "jsonc" => "json",
        "yaml" | "yml" => "yaml",
        "toml" => "toml",
        "md" | "markdown" => "markdown",
        "html" | "htm" => "html",
        "css" | "scss" => "css",
        "sh" | "bash" | "zsh" => "bash",
        "c" | "h" => "c",
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" => "cpp",
        "java" => "java",
        _ => return None,
    })
}

/// Where the lines of a diff are in a text that is parsed for them: for
/// each hunk, the lines of it that this text colours, each with its index
/// in the hunk and its bytes in the text. In the order of the text.
type Placed = Vec<Vec<(usize, Range<usize>)>>;

/// Which lines of a hunk one side of the file colours: the old side the
/// removed lines, the new side everything else.
fn of_side(kind: LineKind, old: bool) -> bool {
    (kind == LineKind::Removed) == old
}

/// The lines of a diff found in one whole side of the file, each by its
/// number. `None` when a line is not in the file as the diff has it: the
/// text is then not the one the diff was made from.
fn in_whole(diff: &FileDiff, text: &str, old: bool) -> Option<Placed> {
    // Where each line starts, and one past the end.
    let mut starts = vec![0];
    starts.extend(
        text.bytes()
            .enumerate()
            .filter(|(_, byte)| *byte == b'\n')
            .map(|(at, _)| at.saturating_add(1)),
    );
    if starts.last() != Some(&text.len()) {
        starts.push(text.len());
    }
    let line = |number: u32| -> Option<Range<usize>> {
        let index = usize::try_from(number.checked_sub(1)?).ok()?;
        let start = *starts.get(index)?;
        let next = *starts.get(index.saturating_add(1))?;
        let end = if text.as_bytes().get(next.checked_sub(1)?) == Some(&b'\n') {
            next.saturating_sub(1)
        } else {
            next
        };
        (start <= end).then_some(start..end)
    };
    let mut placed = Vec::with_capacity(diff.hunks.len());
    for hunk in &diff.hunks {
        let mut lines = Vec::new();
        for (line_ix, hunk_line) in hunk.lines.iter().enumerate() {
            if !of_side(hunk_line.kind, old) {
                continue;
            }
            let number = if old {
                hunk_line.old_no
            } else {
                hunk_line.new_no
            };
            let range = line(number?)?;
            if text.get(range.clone()) != Some(hunk_line.text.as_str()) {
                return None;
            }
            lines.push((line_ix, range));
        }
        placed.push(lines);
    }
    Some(placed)
}

/// One side of a diff as one text, when the whole file is not to be had:
/// the hunks one after the other, each as that side has it (context and
/// removed lines for the old side, context and added lines for the new).
/// A construct that is opened above a hunk is not seen this way.
fn joined(diff: &FileDiff, old: bool) -> (String, Placed) {
    let mut text = String::new();
    let mut placed = Vec::with_capacity(diff.hunks.len());
    for hunk in &diff.hunks {
        let mut lines = Vec::new();
        for (line_ix, line) in hunk.lines.iter().enumerate() {
            let in_text = match line.kind {
                LineKind::Context => true,
                LineKind::Removed => old,
                LineKind::Added => !old,
            };
            if !in_text {
                continue;
            }
            if of_side(line.kind, old) {
                lines.push((
                    line_ix,
                    text.len()..text.len().saturating_add(line.text.len()),
                ));
            }
            text.push_str(&line.text);
            text.push('\n');
        }
        placed.push(lines);
    }
    (text, placed)
}

/// Parse a text once and cut its colours into the lines placed in it.
fn colour(text: &str, placed: &Placed, language: &str, theme: &HighlightTheme, out: &mut Styles) {
    // Every line with its hunk, in the order of the text.
    let lines: Vec<(usize, usize, Range<usize>)> = placed
        .iter()
        .enumerate()
        .flat_map(|(hunk_ix, lines)| {
            lines
                .iter()
                .map(move |(line_ix, range)| (hunk_ix, *line_ix, range.clone()))
        })
        .collect();
    if text.is_empty() || lines.is_empty() {
        return;
    }
    // Making a highlighter costs tens of milliseconds whatever the text:
    // one for a side, not one for a hunk.
    let mut highlighter = SyntaxHighlighter::new(language);
    highlighter.update(None, &Rope::from_str(text), None);
    // Asking for the colours of a stretch costs about a millisecond
    // whatever its length: a few hunks are asked for one by one, so that a
    // large file is not coloured whole for them; many are asked for as one
    // stretch from the first to the last.
    let hunks = placed.iter().filter(|lines| !lines.is_empty()).count();
    let mut stretches: Vec<&[(usize, usize, Range<usize>)]> = Vec::new();
    if hunks > MANY_HUNKS {
        stretches.push(&lines);
    } else {
        let mut rest = lines.as_slice();
        while let Some((hunk_ix, ..)) = rest.first() {
            let len = rest
                .iter()
                .take_while(|(other, ..)| other == hunk_ix)
                .count();
            let (stretch, after) = rest.split_at(len);
            stretches.push(stretch);
            rest = after;
        }
    }
    for stretch in stretches {
        let (Some((.., first)), Some((.., last))) = (stretch.first(), stretch.last()) else {
            continue;
        };
        let span = first.start..last.end;
        if span.start >= span.end {
            continue;
        }
        let mut next = 0;
        for (range, style) in highlighter.styles(&span, theme) {
            if style == HighlightStyle::default() || range.start >= range.end {
                continue;
            }
            // Styles come in order, and so do lines: skip those behind.
            while stretch
                .get(next)
                .is_some_and(|(.., line)| line.end <= range.start)
            {
                next += 1;
            }
            for (hunk_ix, line_ix, line) in stretch.iter().skip(next) {
                if line.start >= range.end {
                    break;
                }
                let from = range.start.max(line.start).saturating_sub(line.start);
                let to = range.end.min(line.end).saturating_sub(line.start);
                let Some(line_text) = text.get(line.clone()) else {
                    continue;
                };
                if from < to && line_text.is_char_boundary(from) && line_text.is_char_boundary(to) {
                    out.entry((*hunk_ix, *line_ix))
                        .or_default()
                        .push((from..to, style));
                }
            }
        }
    }
}

/// Colours for the lines of one text, from one parse of all of them: for
/// each line its spans. Nothing for a language without a grammar, and
/// nothing for a text of more than [`MAX_WHOLE_BYTES`] or more lines than
/// a diff is coloured for.
pub fn highlight_lines(
    lines: &[String],
    language: &str,
    theme: &HighlightTheme,
) -> Vec<LineStyles> {
    let mut out = vec![Vec::new(); lines.len()];
    let total: usize = lines.iter().map(|line| line.len().saturating_add(1)).sum();
    if lines.is_empty() || lines.len() > MAX_LINES || total > MAX_WHOLE_BYTES {
        return out;
    }
    let mut text = String::with_capacity(total);
    let mut placed = Vec::with_capacity(lines.len());
    for (line_ix, line) in lines.iter().enumerate() {
        placed.push((line_ix, text.len()..text.len().saturating_add(line.len())));
        text.push_str(line);
        text.push('\n');
    }
    let mut styles = Styles::new();
    colour(&text, &vec![placed], language, theme, &mut styles);
    for ((_, line_ix), spans) in styles {
        if let Some(slot) = out.get_mut(line_ix) {
            *slot = spans;
        }
    }
    out
}

/// Colours for every line of a diff. `old` is the whole file before the
/// change and `new` after it; with them the colours come from a parse of
/// the whole file, so that a hunk inside a comment or a string reads as
/// one. A side that is absent, larger than [`MAX_WHOLE_BYTES`], or not
/// the text the diff was made from is coloured from the hunks alone.
pub fn highlight_whole(
    diff: &FileDiff,
    old: Option<&str>,
    new: Option<&str>,
    theme: &HighlightTheme,
) -> Styles {
    let mut out = Styles::new();
    let Some(language) = language_for(&diff.path) else {
        return out;
    };
    let lines: usize = diff.hunks.iter().map(|hunk| hunk.lines.len()).sum();
    if diff.binary || lines == 0 || lines > MAX_LINES {
        return out;
    }
    for (is_old, whole) in [(true, old), (false, new)] {
        let whole = whole
            .filter(|text| !text.is_empty() && text.len() <= MAX_WHOLE_BYTES)
            .and_then(|text| Some((text, in_whole(diff, text, is_old)?)));
        match whole {
            Some((text, placed)) => colour(text, &placed, language, theme, &mut out),
            None => {
                let (text, placed) = joined(diff, is_old);
                colour(&text, &placed, language, theme, &mut out);
            }
        }
    }
    out
}
#[cfg(test)]
mod tests {
    use bitbraid_git::{FileDiff, LineKind, diff::FileChangeKind};
    use gpui_kit::component::highlighter::HighlightTheme;

    use super::{MAX_WHOLE_BYTES, Styles, highlight_lines, highlight_whole, language_for};

    /// Colours for every line of a diff from its hunks alone, or nothing
    /// when the path has no grammar, the file is binary or the diff is too
    /// long.
    fn highlight(diff: &FileDiff, theme: &HighlightTheme) -> Styles {
        highlight_whole(diff, None, None, theme)
    }

    #[test]
    fn the_lines_of_a_text_are_coloured_from_one_parse_of_it() {
        let theme = HighlightTheme::default_dark();
        let lines: Vec<String> = [
            "fn main() {",
            "    /* a comment that goes on",
            "       let x = 1; inside it",
            "    */",
            "    let größe = \"neu 🦑\";",
            "",
            "}",
        ]
        .iter()
        .map(|line| (*line).to_owned())
        .collect();
        let styles = highlight_lines(&lines, "rust", &theme);
        assert_eq!(styles.len(), lines.len());
        // The line inside the comment is one span, in the comment's
        // colour: `let` is no keyword there.
        assert_eq!(styles[2].len(), 1, "{:?}", styles[2]);
        assert_eq!(styles[2][0].1, styles[1][0].1);
        assert!(styles[4].len() > 1, "{:?}", styles[4]);
        assert!(styles[5].is_empty());
        for (line, spans) in lines.iter().zip(&styles) {
            let mut end = 0;
            for (range, _) in spans {
                assert!(range.start >= end && range.end <= line.len(), "{line:?}");
                assert!(line.is_char_boundary(range.start) && line.is_char_boundary(range.end));
                end = range.end;
            }
        }
        // No lines, too much text: nothing, and nothing breaks.
        assert!(highlight_lines(&[], "rust", &theme).is_empty());
        let huge = vec!["x".repeat(MAX_WHOLE_BYTES)];
        assert_eq!(highlight_lines(&huge, "rust", &theme), vec![Vec::new()]);
    }

    /// The colour of the first span of a line, as something to compare.
    fn first_colour(styles: &super::Styles, at: (usize, usize)) -> Option<gpui_kit::Hsla> {
        styles.get(&at)?.first()?.1.color
    }

    #[test]
    fn a_hunk_that_starts_inside_a_comment_is_coloured_by_the_whole_file() {
        let theme = HighlightTheme::default_dark();
        // A block comment opens far above the change and closes below it.
        let mut old = String::from("fn before() {}\n/* a long comment\n");
        for n in 0..30 {
            old.push_str(&format!("line {n} of the comment, with let x = 1;\n"));
        }
        old.push_str("the end of it */\nfn after() { let y = 2; }\n");
        let new = old.replace("line 20 of the comment", "line twenty of the comment");
        let diff = diff("src/lib.rs", &old, &new);
        assert_eq!(diff.hunks.len(), 1);
        let hunk = &diff.hunks[0];
        let by_hunk = highlight(&diff, &theme);
        let whole = highlight_whole(&diff, Some(&old), Some(&new), &theme);

        // In the comment, every line is one span in the comment's colour,
        // on both sides; `let` is no keyword there.
        let comment = whole
            .get(&(0, 0))
            .and_then(|spans| spans.first())
            .map(|(_, style)| *style)
            .expect("the first context line has a colour");
        for (line_ix, line) in hunk.lines.iter().enumerate() {
            let spans = whole
                .get(&(0, line_ix))
                .unwrap_or_else(|| panic!("no colours for {:?}", line.text));
            assert_eq!(spans.len(), 1, "{:?}: {spans:?}", line.text);
            assert_eq!(spans[0].0, 0..line.text.len(), "{:?}", line.text);
            assert_eq!(spans[0].1, comment, "{:?}", line.text);
        }
        assert!(hunk.lines.iter().any(|line| line.kind == LineKind::Removed));
        assert!(hunk.lines.iter().any(|line| line.kind == LineKind::Added));
        // Parsed by itself, the hunk reads as code, wrongly: `let` is a
        // keyword and the line has several spans.
        let alone = by_hunk.get(&(0, 0)).map_or(0, Vec::len);
        assert!(alone > 1, "the hunk alone reads as code: {alone} spans");
        assert_ne!(first_colour(&by_hunk, (0, 0)), comment.color);
    }

    #[test]
    fn whole_files_that_are_absent_too_large_or_stale_fall_back_to_the_hunk() {
        let theme = HighlightTheme::default_dark();
        let old = "fn main() {\n    let a = 1;\n}\n";
        let new = "fn main() {\n    let a = 2;\n    run();\n}\n";
        let diff = diff("src/main.rs", old, new);
        let by_hunk = highlight(&diff, &theme);
        // Nothing to parse: exactly the colours of the hunk.
        assert_eq!(highlight_whole(&diff, None, None, &theme), by_hunk);
        // The same text whole gives the same colours here, where nothing
        // above the hunk changes how it reads.
        assert_eq!(
            highlight_whole(&diff, Some(old), Some(new), &theme),
            by_hunk
        );
        // A file that is not the one the diff was made from colours no
        // line it does not have as the hunk has it.
        let stale = "// something else entirely\nconst X: u8 = 1;\n";
        assert_eq!(
            highlight_whole(&diff, Some(stale), Some(stale), &theme),
            by_hunk
        );
        // Too large to parse whole.
        let huge = format!("{new}{}", "// filler\n".repeat(MAX_WHOLE_BYTES / 10 + 1));
        assert!(huge.len() > MAX_WHOLE_BYTES);
        assert_eq!(highlight_whole(&diff, Some(old), Some(&huge), &theme), {
            // The old side is still parsed whole; it says the same.
            by_hunk.clone()
        });
        // An added file has no old side, a deleted one no new side.
        let added = diff_of("src/new.rs", "", new);
        assert!(!highlight_whole(&added, None, Some(new), &theme).is_empty());
        let deleted = diff_of("src/old.rs", old, "");
        assert!(!highlight_whole(&deleted, Some(old), None, &theme).is_empty());
        // No grammar, no colours.
        let plain = diff_of("notes.unknown", old, new);
        assert!(highlight_whole(&plain, Some(old), Some(new), &theme).is_empty());
    }

    #[test]
    fn odd_text_does_not_break_the_mapping_of_lines() {
        let theme = HighlightTheme::default_dark();
        // No line end at the end, DOS line ends, wide characters, a line
        // number past the end.
        let old = "let größe = \"alt\";\r\nlet b = 1;";
        let new = "let größe = \"neu 🦑\";\r\nlet b = 1;\r\nlet c = 3;";
        let mut diff = diff_of("src/x.rs", old, new);
        let styles = highlight_whole(&diff, Some(old), Some(new), &theme);
        for ((hunk_ix, line_ix), spans) in &styles {
            let line = &diff.hunks[*hunk_ix].lines[*line_ix];
            let mut end = 0;
            for (range, _) in spans {
                assert!(range.start >= end && range.end <= line.text.len());
                assert!(line.text.is_char_boundary(range.start));
                assert!(line.text.is_char_boundary(range.end));
                end = range.end;
            }
        }
        assert!(!styles.is_empty());
        for line in diff.hunks.iter_mut().flat_map(|hunk| &mut hunk.lines) {
            line.old_no = line.old_no.map(|n| n.saturating_add(1_000_000));
            line.new_no = line.new_no.map(|_| u32::MAX);
        }
        // Lines that are nowhere in the files keep the hunk's colours.
        assert_eq!(
            highlight_whole(&diff, Some(old), Some(new), &theme),
            highlight(&diff, &theme)
        );
    }

    fn diff_of(path: &str, old: &str, new: &str) -> FileDiff {
        diff(path, old, new)
    }

    /// Rust source of about `bytes` bytes: functions with strings,
    /// comments and numbers in them.
    fn source(bytes: usize) -> String {
        let mut text = String::with_capacity(bytes + 200);
        let mut n = 0;
        while text.len() < bytes {
            text.push_str(&format!(
                "/// Adds {n} to what it is given.\nfn add_{n}(value: u64) -> u64 {{\n    let label = \"number {n}\"; // a note\n    if value > {n} {{ value + {n} }} else {{ label.len() as u64 }}\n}}\n\n"
            ));
            n += 1;
        }
        text
    }

    /// What the bounds were chosen from:
    /// `cargo test -p bitbraid-ui --lib measure_ -- --ignored --nocapture`
    #[test]
    #[ignore = "a measurement, not a check"]
    #[allow(clippy::print_stdout)] // its numbers are what it is for
    fn measure_parsing_whole_files_and_laying_out_large_diffs() {
        use std::time::Instant;

        use gpui_kit::component::{Rope, highlighter::SyntaxHighlighter};

        use crate::views::diff_doc::{Layout, rows};

        let theme = HighlightTheme::default_dark();
        println!("whole-file parse and the colours of 60 lines, by size of the file:");
        for kib in [64, 128, 256, 512, 1024, 2048] {
            let text = source(kib * 1024);
            let started = Instant::now();
            let mut highlighter = SyntaxHighlighter::new("rust");
            highlighter.update(None, &Rope::from_str(&text), None);
            let parsed = started.elapsed();
            let middle = text.len() / 2;
            let from = text[..middle].rfind('\n').unwrap_or(0);
            let to = (from + 3000).min(text.len());
            let to = text[..to].rfind('\n').unwrap_or(to);
            let started = Instant::now();
            let spans = highlighter.styles(&(from..to), &*theme).len();
            println!(
                "  {kib:>5} KiB: parse {:>7.1} ms, colours of a hunk {:>6.2} ms ({spans} spans)",
                parsed.as_secs_f64() * 1e3,
                started.elapsed().as_secs_f64() * 1e3
            );
        }
        println!("compare, colour by hunk and lay out, by size of the file:");
        for (kib, every) in [
            (256, 10),
            (1024, 10),
            (2048, 10),
            (4096, 10),
            (8192, 10),
            (256, 1),
            (1024, 1),
            (2048, 1),
            (4096, 1),
        ] {
            let old = source(kib * 1024);
            let new: String = old
                .lines()
                .enumerate()
                .map(|(n, line)| {
                    if n % every == 0 {
                        format!("{line} // changed\n")
                    } else {
                        format!("{line}\n")
                    }
                })
                .collect();
            let started = Instant::now();
            let hunks = bitbraid_git::diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
            let compared = started.elapsed();
            let mut diff = diff_of("src/big.rs", "", "");
            diff.hunks = hunks;
            let lines: usize = diff.hunks.iter().map(|hunk| hunk.lines.len()).sum();
            let started = Instant::now();
            let coloured = highlight(&diff, &theme).len();
            let colouring = started.elapsed();
            let started = Instant::now();
            let laid = rows(&diff, Layout::Split, true).len();
            let laying = started.elapsed();
            println!(
                "  {kib:>5} KiB, every {every:>2}th line changed: compare {:>7.1} ms, colours {:>7.1} ms ({coloured} lines), rows {:>7.1} ms ({laid} rows of {lines} lines)",
                compared.as_secs_f64() * 1e3,
                colouring.as_secs_f64() * 1e3,
                laying.as_secs_f64() * 1e3
            );
        }
    }

    #[test]
    fn languages_are_found_by_extension_and_name() {
        assert_eq!(language_for("crates/ui/src/views/diff.rs"), Some("rust"));
        assert_eq!(language_for("web/App.TSX"), Some("tsx"));
        assert_eq!(language_for("Cargo.lock"), Some("toml"));
        assert_eq!(language_for("deep/dir/.envrc"), Some("bash"));
        assert_eq!(language_for("Makefile"), None);
        assert_eq!(language_for("archive.tar.gz"), None);
        assert_eq!(language_for(""), None);
        assert_eq!(language_for("trailing."), None);
    }

    fn diff(path: &str, old: &str, new: &str) -> FileDiff {
        FileDiff {
            kind: FileChangeKind::Modified,
            path: path.to_owned(),
            old_path: None,
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            binary: false,
            hunks: bitbraid_git::diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap(),
            additions: 0,
            deletions: 0,
            old_size: None,
            new_size: None,
            too_large: false,
            line_endings: Default::default(),
        }
    }

    #[test]
    fn every_kind_of_line_gets_colours_inside_its_own_text() {
        let theme = HighlightTheme::default_dark();
        let diff = diff(
            "src/main.rs",
            "fn main() {\n    let größe = \"alt\"; // old\n}\n",
            "fn main() {\n    let größe = \"neu\"; // new\n    run();\n}\n",
        );
        let styles = highlight(&diff, &theme);
        let hunk = &diff.hunks[0];
        // Context, removed and added lines with code all have colours.
        for (line_ix, line) in hunk.lines.iter().enumerate() {
            if line.text.trim() == "}" {
                continue;
            }
            let spans = styles
                .get(&(0, line_ix))
                .unwrap_or_else(|| panic!("no colours for {:?}", line.text));
            assert!(!spans.is_empty());
            let mut end = 0;
            for (range, _) in spans {
                assert!(range.start >= end, "spans overlap in {:?}", line.text);
                assert!(range.end <= line.text.len());
                assert!(line.text.is_char_boundary(range.start));
                assert!(line.text.is_char_boundary(range.end));
                end = range.end;
            }
        }
        // The keyword at the start of the first line is coloured as one span.
        let first = &styles[&(0, 0)];
        assert_eq!(first[0].0, 0..2, "{first:?}");
    }

    #[test]
    fn unknown_binary_and_empty_diffs_stay_plain() {
        let theme = HighlightTheme::default_dark();
        assert!(highlight(&diff("notes.unknown", "a\n", "b\n"), &theme).is_empty());
        assert!(highlight(&diff("same.rs", "fn a() {}\n", "fn a() {}\n"), &theme).is_empty());
        let mut binary = diff("image.rs", "a\n", "b\n");
        binary.binary = true;
        assert!(highlight(&binary, &theme).is_empty());
    }
}
