//! The diff viewer: one file as a virtual list. In the diff view it shows
//! the hunks only, each under its heading, or the whole file in one column
//! or in two, with line numbers, word-level emphasis inside edited lines,
//! and stage, unstage or discard per hunk or selected lines. In the file
//! view it shows the file as it is on that side, unmarked.
//!
//! How a diff is shown (one column or two, how whitespace counts, whether
//! long lines wrap, whether changed words are marked) is the preferences'
//! (`Preferences::diff`): the viewer writes what is chosen there and takes
//! over what is written there, so the next diff, the other tabs and the
//! next run show it the same way.
//!
//! Wrapping: a long line is cut where the toolkit's line wrapper would
//! break it at the width the text has, and each piece is a line of its
//! own under the first; a row is as many lines tall as its longer side.
//! The rows are measured again when the width changes.
//!
//! A file with a side larger than `session::DIFF_GUARD_BYTES` arrives
//! measured and not compared; it is compared when that is asked for.
//!
//! The text of the lines is selected with the pointer and copied
//! (`text_select.rs`): a press and a drag over it, two clicks for a word,
//! three for the line, Shift with a click to move the end; Ctrl+C or
//! "Copy" in a line's menu. Only the lines' text is taken, of the column
//! the selection began in. The numbers and signs at the left stay what
//! picks changed lines for staging, and a press on the text picks its
//! line too.
//!
//! Sources: git-diff(1) for the hunk format; gpui-kit documentation
//! (virtual list, styled text).

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Range,
    rc::Rc,
};

use bitbraid_git::{
    ChangeKind, FileDiff, FileLineEndings, Hunk, Line, LineEndings, LineKind, Oid, StatusEntry,
    Whitespace,
};
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, IconName, Sizable, StyledExt, VirtualListScrollHandle,
    button::{Button, ButtonGroup, ButtonVariants},
    h_flex,
    menu::{ContextMenuExt, PopupMenu},
    tooltip::Tooltip,
    v_flex, v_virtual_list,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;
use serde::Deserialize;
use serde_json::json;

use crate::{
    keymap::{CopyText, DIFF_CONTEXT, NextChange, PrevChange, ToggleWrap},
    prefs::{DiffPrefs, Prefs},
    session::{DIFF_GUARD_BYTES, Reply, Request, SessionHandle},
    views::{
        changes::Side,
        chosen,
        diff_doc::{self, Cell, Layout, Mark, Row},
        diff_syntax::{self, Styles},
        image_diff::{self, Mode as ImageMode, Side as ImageSide},
        text_select::{self, Column, LineText, Selecting},
    },
};

/// Height of a line row, and the size of its text.
const LINE_HEIGHT: f32 = 16.0;
const TEXT_SIZE: f32 = 12.0;
/// Height of a hunk's heading, and the space above every heading but the
/// first. The heading's words and the rule under it begin this far in.
const HUNK_HEIGHT: f32 = 32.0;
const HUNK_SPACE: f32 = 24.0;
const HEADING_INSET: f32 = 30.0;
/// What a hunk's buttons have before and after their words, and between
/// one another. They are 20 high (the toolkit's smallest) and the last one
/// ends 20 short of the pane's edge: the heading's 6 and the strip's 14.
const HUNK_BUTTON_PADDING: f32 = 6.0;
const HUNK_BUTTON_GAP: f32 = 10.0;
const HUNK_HEADING_END: f32 = 6.0;
/// Height of the bar that names a version of a picture, the rule under
/// it included, and what a picture has around it under that bar.
const IMAGE_BAR: f32 = 21.0;
const IMAGE_PAD: f32 = 10.0;
/// The buttons of the rows above a file: how high they are, how wide an
/// icon button is, and how far apart two groups are.
pub const CONTROL_HEIGHT: f32 = 23.0;
pub const CONTROL_ICON_WIDTH: f32 = 33.0;
pub const CONTROL_GAP: f32 = 10.0;
/// Width of a line-number column: the old numbers end 30 in, the new
/// ones 60 in.
const NUMBER_WIDTH: f32 = 30.0;
/// Width of the column with the sign of an added or removed line, right
/// after the new number: the text begins 70 in.
const MARKER_WIDTH: f32 = 10.0;
/// The strip between the two halves of a split row, with a rule.
const SPLIT_GAP: f32 = 14.0;
/// The strip at the right edge of a diff: a rule at its left and the
/// mark of the line the cursor is on. Beside the whole file it is followed
/// by the map of where the changes are, a half for each side.
const RULER_WIDTH: f32 = 14.0;
const OVERVIEW_WIDTH: f32 = 30.0;
/// A run of changes on the map is never lower than this, and the cursor's
/// mark is this high.
const MARK_MIN: f32 = 4.0;
const CURSOR_MARK: f32 = 2.0;
/// Beside the file as it is, a miniature of its text stands left of the
/// strip: a line two high, a character one wide, from 12 in.
const MINI_WIDTH: f32 = 120.0;
const MINI_LINE: f32 = 2.0;
const MINI_INSET: f32 = 12.0;
/// How many characters of a line the miniature has room for.
const MINI_COLUMNS: usize = (MINI_WIDTH - MINI_INSET) as usize;
/// A row of the block that says what a submodule is recorded at, and the
/// width of its words.
const SUBMODULE_ROW: f32 = 26.0;
const SUBMODULE_LABEL: f32 = 120.0;
/// Unchanged lines shown around a change unless the whole file is asked for.
const CONTEXT: u32 = 3;

/// What the viewer shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiffSource {
    /// A worktree file on one side of the staging area (actions available).
    Working {
        /// Path.
        path: String,
        /// Unstaged (index to worktree) or staged (HEAD to index).
        side: Side,
    },
    /// A file as changed by a commit (read-only).
    Commit {
        /// Commit id.
        id: bitbraid_git::Oid,
        /// Path on the new side.
        path: String,
        /// Precomputed diff.
        diff: Box<FileDiff>,
    },
    /// A file as it differs between two commits (read-only): what a
    /// selection of several commits shows.
    Range {
        /// The older end; none for the empty tree.
        old: Option<bitbraid_git::Oid>,
        /// The newer end.
        new: bitbraid_git::Oid,
        /// Path on the new side.
        path: String,
        /// Precomputed diff.
        diff: Box<FileDiff>,
    },
    /// A file as one commit changed it, asked for by path (read-only):
    /// what file history opens.
    Revision {
        /// Commit id.
        id: bitbraid_git::Oid,
        /// Path in that commit.
        path: String,
        /// Where the file was before, when this commit moved it.
        old_path: Option<String>,
    },
    /// A file as it differs between a commit and the working tree
    /// (read-only).
    AgainstWorktree {
        /// The commit.
        old: bitbraid_git::Oid,
        /// Path.
        path: String,
        /// Precomputed diff.
        diff: Box<FileDiff>,
    },
}

/// The layout the preferences name.
fn layout_of(prefs: &DiffPrefs) -> Layout {
    match prefs.layout.as_str() {
        "one_column" => Layout::Inline,
        "two_columns" => Layout::Split,
        _ => Layout::Hunks,
    }
}

/// How whitespace counts, as the preferences name it.
fn whitespace_of(prefs: &DiffPrefs) -> Whitespace {
    match prefs.whitespace.as_str() {
        "ignore_trailing" => Whitespace::IgnoreTrailing,
        "ignore_all" => Whitespace::IgnoreAll,
        "ignore_cr_at_eol" => Whitespace::IgnoreCrAtEol,
        _ => Whitespace::Exact,
    }
}

/// The preferences' word for a layout and for a way of counting
/// whitespace.
fn words_for(layout: Layout, whitespace: Whitespace) -> (&'static str, &'static str) {
    (
        match layout {
            Layout::Hunks => "hunks",
            Layout::Inline => "one_column",
            Layout::Split => "two_columns",
        },
        match whitespace {
            Whitespace::Exact => "exact",
            Whitespace::IgnoreTrailing => "ignore_trailing",
            Whitespace::IgnoreAll => "ignore_all",
            Whitespace::IgnoreCrAtEol => "ignore_cr_at_eol",
        },
    )
}

/// What a file that was not compared says about itself: how large its
/// two sides are.
pub fn guard_text(diff: &FileDiff) -> String {
    let size = |bytes: Option<u64>| {
        bytes.map(|bytes| image_diff::human_size(usize::try_from(bytes).unwrap_or(usize::MAX)))
    };
    let sizes = match (size(diff.old_size), size(diff.new_size)) {
        (Some(old), Some(new)) => say!("diff-sizes", old, new),
        (None, Some(new)) => say!("diff-guard-size-new", new),
        (Some(old), None) => say!("diff-guard-size-deleted", old),
        (None, None) => say!("diff-guard-size-unknown").to_owned(),
    };
    say!(
        "diff-guard",
        sizes,
        limit = image_diff::human_size(usize::try_from(DIFF_GUARD_BYTES).unwrap_or(usize::MAX))
    )
}

/// The ids git gives the two versions of a file, short, as "old → new".
pub fn blob_ids(diff: &FileDiff) -> Option<String> {
    match (diff.old_id, diff.new_id) {
        (Some(old), Some(new)) => Some(say!(
            "diff-blob-ids",
            old = old.short(7),
            new = new.short(7)
        )),
        (None, Some(id)) | (Some(id), None) => Some(say!("diff-blob-id", id = id.short(7))),
        (None, None) => None,
    }
}

/// One side of a binary file in words: how large it is and what git
/// calls its content.
pub fn binary_side(size: Option<u64>, id: Option<Oid>) -> Option<(String, String)> {
    let id = id?;
    Some((
        size.map(|bytes| image_diff::human_size(usize::try_from(bytes).unwrap_or(usize::MAX)))
            .unwrap_or_else(|| say!("diff-binary-size-unknown").to_owned()),
        id.to_hex(),
    ))
}

/// One of the buttons a hunk's heading ends in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HunkButton {
    /// What the element is called, with the hunk's index.
    id: &'static str,
    /// The engine command it runs on the hunk or on its selected lines.
    pub command: &'static str,
    /// Two generic words: what is done, and to what.
    pub label: &'static str,
    /// It takes a change back (out of the file, or out of the index).
    pub takes_back: bool,
}

/// The buttons of a hunk on one side of the staging area, in the order they
/// stand: discarding before staging. With lines of the hunk selected they
/// are about those lines.
pub fn hunk_buttons(side: Side, lines: bool) -> Vec<HunkButton> {
    let button = |id, command, hunk, lines_label, takes_back| HunkButton {
        id,
        command,
        label: if lines { lines_label } else { hunk },
        takes_back,
    };
    match side {
        Side::Unstaged => vec![
            button(
                "discard-hunk",
                "discard.hunks",
                say!("diff-discard-hunk"),
                say!("diff-discard-lines"),
                true,
            ),
            button(
                "stage-hunk",
                "stage.hunks",
                say!("diff-stage-hunk"),
                say!("diff-stage-lines"),
                false,
            ),
        ],
        Side::Staged => vec![button(
            "unstage-hunk",
            "unstage.hunks",
            say!("diff-unstage-hunk"),
            say!("diff-unstage-lines"),
            true,
        )],
    }
}

/// The files the index holds under another name than the last commit
/// does, by their new path: where each was before. A file of this kind is
/// compared with what it was under its old name; asked for by its new name
/// alone it is on one side only, and would be shown as new, whole.
pub fn staged_moves(status: &[StatusEntry]) -> BTreeMap<String, String> {
    status
        .iter()
        .filter(|entry| entry.staged == Some(ChangeKind::Renamed))
        .filter_map(|entry| Some((entry.path.clone(), entry.original_path.clone()?)))
        .collect()
}

/// The text of the line of `row` of `rows` in `column`: none for a hunk's
/// heading or the empty side of a row.
fn line_of<'a>(
    diff: Option<&'a FileDiff>,
    rows: &[Row],
    row: usize,
    column: Column,
) -> Option<&'a str> {
    let (hunk, cell) = match (rows.get(row)?, column) {
        (Row::Line { hunk, cell } | Row::Plain { hunk, cell }, Column::Only) => (*hunk, cell),
        (Row::Pair { hunk, left, .. }, Column::Old) => (*hunk, left.as_ref()?),
        (Row::Pair { hunk, right, .. }, Column::New) => (*hunk, right.as_ref()?),
        _ => return None,
    };
    Some(diff?.hunks.get(hunk)?.lines.get(cell.line)?.text.as_str())
}

/// What stands where the lines would, for a file whose text did not change.
pub fn unchanged_text(diff: &FileDiff) -> String {
    match &diff.old_path {
        Some(old_path) => say!("diff-moved-unchanged", old_path),
        None => say!("diff-no-changes").to_owned(),
    }
}

/// How line endings are named in the file's header, short, and in what
/// is said of them at length.
fn ending_name(endings: LineEndings) -> (&'static str, &'static str) {
    match endings {
        LineEndings::Lf => ("LF", "LF"),
        LineEndings::Crlf => ("CRLF", "CRLF"),
        LineEndings::Mixed => ("LF + CRLF", say!("diff-endings-mixed")),
    }
}

/// What the file's header says of its line endings, and what its tooltip
/// says. Nothing where they do not matter: lines that end in LF on both
/// sides and on disk, which is how git stores text.
pub fn line_endings_label(endings: &FileLineEndings) -> Option<(String, String)> {
    let FileLineEndings {
        old, new, worktree, ..
    } = *endings;
    if let (Some(old), Some(new)) = (old, new)
        && old != new
    {
        let ((old_short, old_long), (new_short, new_long)) = (ending_name(old), ending_name(new));
        return Some((
            format!("{old_short} → {new_short}"),
            say!("diff-endings-changed", old_long, new_long),
        ));
    }
    let compared = new.or(old);
    match (compared, worktree) {
        (Some(stored), Some(on_disk)) if on_disk != stored => {
            let ((short, long), (_, stored_long)) = (ending_name(on_disk), ending_name(stored));
            Some((
                short.to_owned(),
                say!("diff-endings-stored", long, stored_long),
            ))
        }
        (Some(LineEndings::Lf), _) | (None, None) => None,
        (Some(endings), _) | (None, Some(endings)) => {
            let (short, long) = ending_name(endings);
            Some((short.to_owned(), say!("diff-endings", long)))
        }
    }
}

/// What is said in place of the lines of a file whose line endings are
/// all that changed.
pub fn endings_only_text(endings: &FileLineEndings) -> String {
    match (endings.old, endings.new) {
        (Some(old), Some(new)) if old != new => say!(
            "diff-endings-only-from-to",
            from = ending_name(old).1,
            to = ending_name(new).1
        ),
        _ => say!("diff-endings-only").to_owned(),
    }
}

/// The mark after a changed line whose line ending is not the other
/// side's: `CR` on a line that ends in CRLF where the other side's lines
/// all end in LF alone, `LF` on one that ends in LF alone where the other
/// side's all end in CRLF. Nothing on unchanged lines, on a last line
/// without a line feed, and where the other side is not there, has no
/// line ends or has both (the header says so).
pub fn ending_mark(line: &Line, endings: &FileLineEndings) -> Option<&'static str> {
    let other = match line.kind {
        LineKind::Context => return None,
        LineKind::Added => endings.old?,
        LineKind::Removed => endings.new?,
    };
    if line.no_newline {
        return None;
    }
    match (line.text.ends_with('\r'), other) {
        (true, LineEndings::Lf) => Some("CR"),
        (false, LineEndings::Crlf) => Some("LF"),
        _ => None,
    }
}

/// What the viewer has on show, as plain facts, for the off-screen tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffProbe {
    /// The answer for what was asked last is not in yet.
    pub loading: bool,
    /// Where the file was before, when it is shown as moved.
    pub moved_from: Option<String>,
    /// How many lines are marked as added, and how many as removed.
    pub added: u32,
    /// See `added`.
    pub removed: u32,
    /// The layout: `hunks`, `one_column` or `two_columns`.
    pub layout: &'static str,
    /// Whether the file is shown as it is (the file view).
    pub file_view: bool,
    /// The row the cursor is on, whose mark is on the strip at the right.
    pub cursor: Option<usize>,
    /// How many lines are selected.
    pub selected: usize,
    /// The pointer is over that strip.
    pub map_hovered: bool,
    /// How far down the rows are scrolled, in whole pixels.
    pub scrolled: i32,
    /// How many hunk headings, and how many rows of lines, there are.
    pub headings: usize,
    /// See `headings`.
    pub lines: usize,
    /// How the two versions of a picture are shown: `apart`, `overlay`,
    /// `swipe` or `difference`. None when what is shown is no picture.
    pub picture: Option<&'static str>,
    /// The text that is selected, as it would be copied.
    pub text_selected: String,
    /// What the header says of the line endings, where it says anything.
    pub line_endings: Option<String>,
    /// The lines are left out for saying that only the line endings
    /// changed.
    pub endings_only: bool,
}

/// Where things go on the map of a diff: the rows are `total` high in
/// all, the map `height`. A file shorter than the map is drawn at its own
/// height, a longer one shrunk to fit.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MapGeometry {
    total: f32,
    height: f32,
    scale: f32,
}

impl MapGeometry {
    /// The map of rows `total` high on a strip `height` high.
    pub fn new(total: f32, height: f32) -> Self {
        let scale = if total > 0.0 && height > 0.0 {
            (height / total).min(1.0)
        } else {
            0.0
        };
        Self {
            total,
            height,
            scale,
        }
    }

    /// Top and height on the map of the rows from `from` to `to`: never
    /// lower than `least`, and inside the map.
    pub fn mark(&self, from: f32, to: f32, least: f32) -> (f32, f32) {
        let tall = ((to - from) * self.scale).max(least).min(self.height);
        let top = (from * self.scale).min(self.height - tall).max(0.0);
        (top, tall)
    }

    /// Top and height of what is in view, the list being scrolled down by
    /// `scrolled`: all of the map when all of the file is in view.
    pub fn view(&self, scrolled: f32) -> (f32, f32) {
        if self.total <= self.height {
            return (0.0, self.height);
        }
        let tall = (self.height * self.scale).max(12.0).min(self.height);
        let top = (scrolled * self.scale).min(self.height - tall).max(0.0);
        (top, tall)
    }
}

/// The runs of a line that are not blank, by column, as the miniature
/// draws them: a tab is four columns, and nothing past what it has room
/// for.
pub fn mini_runs(text: &str) -> Vec<(u16, u16)> {
    let mut out: Vec<(u16, u16)> = Vec::new();
    let mut column = 0usize;
    for ch in text.chars() {
        if column >= MINI_COLUMNS {
            break;
        }
        if ch == '\t' {
            column = column.saturating_add(4);
            continue;
        }
        if !ch.is_whitespace() && !ch.is_control() {
            let at = u16::try_from(column).unwrap_or(u16::MAX);
            match out.last_mut() {
                Some((start, len)) if start.saturating_add(*len) == at => {
                    *len = len.saturating_add(1)
                }
                _ => out.push((at, 1)),
            }
        }
        column = column.saturating_add(1);
    }
    out
}

/// Paint the miniature of a text at the left of `bounds`: the runs of each
/// line (`mini`), two high, moved with the list (rows `total` high in all,
/// scrolled down by `scrolled`, a row's top at `row_top`), what is in view
/// over it when `show_view`, and the rule after it.
pub fn paint_miniature(
    bounds: Bounds<Pixels>,
    mini: &[Vec<(u16, u16)>],
    (total, scrolled): (f32, f32),
    row_top: impl Fn(usize) -> f32,
    (ink, view, rule): (Hsla, Hsla, Hsla),
    show_view: bool,
    window: &mut Window,
) {
    let height = crate::zoom::unpx(bounds.size.height);
    if height <= 0.0 || total <= 0.0 {
        return;
    }
    let quad = |left: f32, top: f32, wide: f32, tall: f32, color: Hsla| {
        fill(
            Bounds::new(
                point(bounds.left() + px(left), bounds.top() + px(top)),
                size(px(wide), px(tall)),
            ),
            color,
        )
    };
    window.paint_quad(quad(MINI_WIDTH, 0.0, 1.0, height, rule));
    // The lines that are drawn: as many as the strip holds, from where
    // the list is.
    let rows = mini.len();
    let moved = MiniGeometry::new(rows, total, height).offset(scrolled);
    let first = (moved / MINI_LINE) as usize;
    let shown = (height / MINI_LINE).ceil() as usize + 1;
    for (ix, runs) in mini.iter().enumerate().skip(first).take(shown) {
        let top = ix as f32 * MINI_LINE - moved;
        for (start, len) in runs {
            window.paint_quad(quad(
                MINI_INSET + f32::from(*start),
                top,
                f32::from(*len),
                MINI_LINE,
                ink,
            ));
        }
    }
    if show_view {
        // What is in view, on the lines that are: the first row whose top
        // is past a height, found by halving.
        let row_of = |y: f32| {
            let (mut low, mut high) = (0usize, rows);
            while low < high {
                let middle = low + (high - low) / 2;
                if row_top(middle) <= y {
                    low = middle + 1;
                } else {
                    high = middle;
                }
            }
            low
        };
        let from = row_of(scrolled).saturating_sub(1) as f32;
        let to = row_of(scrolled + height) as f32;
        let top = from * MINI_LINE - moved;
        let tall = ((to - from) * MINI_LINE).max(MINI_LINE);
        window.paint_quad(quad(0.0, top, MINI_WIDTH, tall, view));
    }
}

/// Where the lines of the miniature are: `rows` of them, two high each,
/// on a strip `height` high beside rows `total` high in all. When they
/// are more than the strip holds, the miniature moves with the list, so
/// that its top is at the file's top and its end at the file's end.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MiniGeometry {
    rows: usize,
    total: f32,
    height: f32,
}

impl MiniGeometry {
    /// The miniature of `rows` rows, `total` high in the list, on a strip
    /// `height` high.
    pub fn new(rows: usize, total: f32, height: f32) -> Self {
        Self {
            rows,
            total,
            height,
        }
    }

    /// How far the miniature is moved up, the list being scrolled down by
    /// `scrolled`.
    pub fn offset(&self, scrolled: f32) -> f32 {
        let drawn = self.rows as f32 * MINI_LINE;
        let over = (drawn - self.height).max(0.0);
        let scrolls = (self.total - self.height).max(0.0);
        if over <= 0.0 || scrolls <= 0.0 {
            return 0.0;
        }
        (scrolled / scrolls).clamp(0.0, 1.0) * over
    }

    /// The row whose line is at `y` on the strip, the list being scrolled
    /// down by `scrolled`.
    pub fn row_at(&self, y: f32, scrolled: f32) -> usize {
        let line = ((y + self.offset(scrolled)) / MINI_LINE).max(0.0) as usize;
        line.min(self.rows.saturating_sub(1))
    }
}

/// The controls of a diff, in two parts for the two rows above it.
pub struct DiffControls {
    /// The groups of the second row, after blame and history.
    pub groups: AnyElement,
    /// The rest, at the right of the first row.
    pub extras: AnyElement,
}

/// What the rows above a file need to know of it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileShape {
    /// What happened to it, once the diff is in.
    pub kind: Option<bitbraid_git::diff::FileChangeKind>,
    /// It has lines: it is not binary, a picture or a submodule.
    pub text: bool,
    /// Where it was before, when it was moved.
    pub moved_from: Option<String>,
}

/// The last row of the run of changes that begins at `start` (the next
/// run beginning at `next`), from the runs of each side.
pub fn change_end(marks: &[(usize, usize, Mark)], start: usize, next: Option<usize>) -> usize {
    marks
        .iter()
        .filter(|(at, _, _)| *at >= start && next.is_none_or(|next| *at < next))
        .map(|(at, len, _)| at.saturating_add(len.saturating_sub(1)))
        .max()
        .unwrap_or(start)
}

/// How high the row of a hunk's heading is: the heading, and the space
/// above it that every heading but the first has.
fn heading_height(hunk: usize) -> f32 {
    if hunk == 0 {
        HUNK_HEIGHT
    } else {
        HUNK_HEIGHT + HUNK_SPACE
    }
}

/// The lines of a hunk that a drag from `from` to `to` (indexes of lines
/// in it) selects: the changed ones between the two, both included.
pub fn dragged_lines(hunk: &Hunk, from: usize, to: usize) -> Vec<usize> {
    let (low, high) = (from.min(to), from.max(to));
    hunk.lines
        .iter()
        .enumerate()
        .skip(low)
        .take(high.saturating_sub(low).saturating_add(1))
        .filter(|(_, line)| line.kind != LineKind::Context)
        .map(|(ix, _)| ix)
        .collect()
}

/// Lines that lean across a place, where one side of a split row has no
/// line.
fn paint_hatch(bounds: Bounds<Pixels>, colour: Hsla, window: &mut Window) {
    let (width, height) = (
        crate::zoom::unpx(bounds.size.width),
        crate::zoom::unpx(bounds.size.height),
    );
    if width <= 0.0 || height <= 0.0 {
        return;
    }
    let mut builder = PathBuilder::stroke(px(1.));
    let mut x = -height;
    while x < width {
        builder.move_to(point(bounds.left() + px(x), bounds.bottom()));
        builder.line_to(point(bounds.left() + px(x + height), bounds.top()));
        x += 6.0;
    }
    if let Ok(path) = builder.build() {
        window.paint_layer(bounds, |window| window.paint_path(path, colour));
    }
}

/// The older end of a range as the session names it.
fn range_old(old: Option<bitbraid_git::Oid>) -> String {
    old.map(|id| id.to_hex())
        .unwrap_or_else(|| "empty".to_owned())
}

/// The two versions of an image file and how they are being looked at.
struct ImageState {
    old: Option<ImageSide>,
    new: Option<ImageSide>,
    /// How many of the two sides have answered.
    answered: u8,
    mode: ImageMode,
    /// Where the divider of a swipe is, or how much of the new version
    /// shows through the old one laid over it: 0 to 1.
    at: f32,
    /// Where the pictures were painted last: how wide their place is, and
    /// where a pointer position is counted from.
    bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// How wide the two versions are where one lies over the other, to
    /// turn a pointer position into `at`.
    span: Rc<std::cell::Cell<f32>>,
    /// The pixel difference, worked out when first asked for.
    difference: Option<Result<(Option<image_diff::Shown>, String), String>>,
}

/// What lies between the two commits a submodule is recorded at, as
/// `submodule.changes` answered.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct SubmoduleState {
    /// The commit recorded on each side, by its whole id.
    old: Option<String>,
    new: Option<String>,
    /// What each of the two says, when the submodule's clone has it.
    old_summary: Option<String>,
    new_summary: Option<String>,
    /// The answer is in.
    answered: bool,
    /// The submodule's own repository had both commits.
    known: bool,
    /// Commits the new side has and the old one has not: id and summary.
    gained: Vec<(String, String)>,
    /// Commits the old side had and the new one has not.
    lost: Vec<(String, String)>,
    more_gained: bool,
    more_lost: bool,
}

impl SubmoduleState {
    /// Read the answer of `submodule.changes`.
    fn from_reply(value: &serde_json::Value) -> Self {
        let text = |key: &str| {
            value
                .get(key)
                .and_then(|text| text.as_str())
                .map(str::to_owned)
        };
        let commits = |key: &str| -> Vec<(String, String)> {
            value
                .get(key)
                .and_then(|list| list.as_array())
                .map(|list| {
                    list.iter()
                        .filter_map(|commit| {
                            // The whole id: a commit of the list can be
                            // opened, and is found by it.
                            Some((
                                commit.get("id")?.as_str()?.to_owned(),
                                commit.get("summary")?.as_str()?.to_owned(),
                            ))
                        })
                        .collect()
                })
                .unwrap_or_default()
        };
        let flag = |key: &str| value.get(key).and_then(|flag| flag.as_bool()) == Some(true);
        Self {
            old: text("old"),
            new: text("new"),
            old_summary: text("old_summary"),
            new_summary: text("new_summary"),
            answered: true,
            known: flag("known"),
            gained: commits("gained"),
            lost: commits("lost"),
            more_gained: flag("more_gained"),
            more_lost: flag("more_lost"),
        }
    }

    /// The rows of the block that says what the submodule is recorded at:
    /// the old commit and what it says, then the new one; the old side's
    /// rows marked as taken out.
    fn rows(&self) -> Vec<(&'static str, String, bool)> {
        let mut rows = Vec::new();
        for (id, summary, old) in [
            (&self.old, &self.old_summary, true),
            (&self.new, &self.new_summary, false),
        ] {
            if let Some(id) = id {
                rows.push((say!("diff-submodule-commit"), id.clone(), old));
                if let Some(summary) = summary {
                    rows.push((say!("diff-submodule-message"), summary.clone(), old));
                }
            }
        }
        // The same commit on both sides is one block.
        if self.old.is_some() && self.old == self.new {
            rows.retain(|(_, _, old)| !*old);
        }
        rows
    }
}

/// Stage, unstage or discard the line a menu was opened on, or the
/// selection it is in.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = diff, no_json)]
pub struct ActOnLines {
    /// The engine command: `stage.hunks`, `unstage.hunks` or
    /// `discard.hunks`.
    pub command: String,
    /// The hunk, and the line in it the menu was opened on.
    pub hunk: usize,
    /// See `hunk`.
    pub line: usize,
}

/// Copy the line a menu was opened on, or the selection it is in.
#[derive(Action, Clone, Debug, PartialEq, Eq, Deserialize)]
#[action(namespace = diff, no_json)]
struct CopyLines {
    hunk: usize,
    line: usize,
}

/// Events from the diff viewer.
#[derive(Debug, Clone)]
pub enum DiffEvent {
    /// The repository changed through the viewer; refresh status.
    Mutated,
    /// A commit of a submodule was picked from the commits between the
    /// two it is recorded at: open the submodule on it.
    OpenSubmoduleCommit {
        /// The submodule's path in this repository.
        path: String,
        /// The commit, by its whole id.
        id: String,
    },
}

pub struct DiffView {
    session: SessionHandle,
    source: Option<DiffSource>,
    diff: Option<Rc<FileDiff>>,
    layout: Layout,
    /// Mark what changed inside edited lines.
    words: bool,
    /// How whitespace counts when lines are compared. Anything but exact
    /// changes the hunks, so staging from the viewer is off meanwhile.
    whitespace: Whitespace,
    /// Show the file as it is on that side, unmarked, instead of how it
    /// changed.
    file_view: bool,
    /// Long lines go on under themselves instead of off to the right.
    wrap: bool,
    /// With wrapping, for each row where its text is cut: the old (or
    /// only) side, and the new side of a split row. Empty without.
    cuts: Rc<Vec<(Vec<usize>, Vec<usize>)>>,
    /// The width the rows were last cut for, and where the list was last
    /// painted.
    cut_width: f32,
    list_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// What of the preferences the view shows.
    applied: DiffPrefs,
    /// The two whole files as last fetched, for the colours: the old and
    /// the new side, each absent where there is none or it is too large.
    /// `None` until both have answered.
    texts: Option<(Option<String>, Option<String>)>,
    texts_pending: (Option<Option<String>>, Option<Option<String>>),
    /// The source whose size guard was waved through.
    load_anyway: Option<DiffSource>,
    /// Which run of working out the colours is the current one: an
    /// answer of an earlier run is for other lines and is dropped.
    colour_run: u64,
    /// Rows where a run of changes begins, and the one last jumped to.
    changes: Rc<Vec<usize>>,
    change_at: Option<usize>,
    /// Syntax colours per line, for the theme they were made with.
    syntax: Rc<Styles>,
    syntax_dark: bool,
    /// The colours of code the syntax was worked out with: a theme of the
    /// user's own can change them without changing the mode.
    syntax_theme: std::sync::Arc<gpui_kit::component::highlighter::HighlightTheme>,
    /// Rows of the list, rebuilt when the diff or how it is shown changes.
    rows: Rc<Vec<Row>>,
    sizes: crate::zoom::Sizes,
    /// For the minimap: the runs of changed rows.
    marks: Rc<Vec<(usize, usize, Mark)>>,
    /// For the miniature of the file view: the runs of characters that
    /// are not blank, per row, as (first column, how many).
    mini: Rc<Vec<Vec<(u16, u16)>>>,
    /// Where each row starts in the list in pixels (one more entry than
    /// rows: the end).
    offsets: Rc<Vec<f32>>,
    /// Where the minimap was painted last, to turn a click into a place.
    minimap_bounds: Rc<std::cell::Cell<Bounds<Pixels>>>,
    /// The pointer is over the strip at the right edge: what is in view is
    /// shown on it then, as a scroll bar would.
    minimap_hovered: bool,
    /// The row the cursor is on: the one last clicked, or the change last
    /// stepped to. Its mark is on the strip at the right edge.
    cursor: Option<usize>,
    /// Longest line in characters, for the horizontal scroll range.
    longest: usize,
    scroll: VirtualListScrollHandle,
    /// Horizontal scroll offset of the text in pixels.
    scroll_x: f32,
    /// Selected change lines as (hunk index, line index).
    selected: BTreeSet<(usize, usize)>,
    /// Where a drag over lines began, while the pointer is held down.
    drag_from: Option<(usize, usize)>,
    /// Text selected with the pointer.
    text: Selecting,
    /// Set when the file shown is a picture.
    image: Option<ImageState>,
    /// Set when what is shown is a submodule.
    submodule: Option<SubmoduleState>,
    /// Files the index holds under a new name, and their old one.
    staged_moves: BTreeMap<String, String>,
    /// The lines of a file whose line endings are all that changed are
    /// shown, which they are not until asked for.
    endings_lines: bool,
    loading: bool,
    error: Option<String>,
    focus_handle: FocusHandle,
}

impl EventEmitter<DiffEvent> for DiffView {}

impl DiffView {
    pub fn new(session: SessionHandle, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let applied = cx.global::<Prefs>().preferences.diff.clone();
        Self {
            session,
            source: None,
            diff: None,
            layout: layout_of(&applied),
            words: applied.words,
            whitespace: whitespace_of(&applied),
            file_view: false,
            wrap: applied.wrap,
            cuts: Rc::new(Vec::new()),
            cut_width: 0.0,
            list_bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            applied,
            texts: None,
            texts_pending: (None, None),
            load_anyway: None,
            colour_run: 0,
            changes: Rc::new(Vec::new()),
            change_at: None,
            syntax: Rc::new(Styles::new()),
            syntax_dark: cx.theme().is_dark(),
            syntax_theme: cx.theme().highlight_theme.clone(),
            rows: Rc::new(Vec::new()),
            sizes: crate::zoom::Sizes::default(),
            marks: Rc::new(Vec::new()),
            mini: Rc::new(Vec::new()),
            offsets: Rc::new(vec![0.0]),
            minimap_bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            minimap_hovered: false,
            cursor: None,
            longest: 0,
            scroll: VirtualListScrollHandle::new(),
            scroll_x: 0.0,
            image: None,
            submodule: None,
            staged_moves: BTreeMap::new(),
            endings_lines: false,
            selected: BTreeSet::new(),
            drag_from: None,
            text: Selecting::default(),
            loading: false,
            error: None,
            focus_handle: cx.focus_handle(),
        }
    }

    fn set_diff(&mut self, diff: Option<FileDiff>, cx: &mut Context<Self>) {
        self.diff = diff.map(Rc::new);
        self.text.clear();
        self.texts = None;
        // The colours there are belong to the lines that were shown; the
        // ones for these lines follow when the two files are in.
        self.syntax = Rc::new(Styles::new());
        self.colour_run = self.colour_run.wrapping_add(1);
        self.rebuild(cx);
        self.load_images(cx);
        self.load_submodule(cx);
        self.load_texts(cx);
    }

    /// Write how the diff is shown into the preferences, when it is not
    /// what they say already.
    fn remember(&mut self, cx: &mut Context<Self>) {
        let (layout, whitespace) = words_for(self.layout, self.whitespace);
        let now = DiffPrefs {
            layout: layout.to_owned(),
            whitespace: whitespace.to_owned(),
            wrap: self.wrap,
            words: self.words,
        };
        if now != self.applied {
            self.applied = now.clone();
            let prefs = cx.global_mut::<Prefs>();
            prefs.preferences.diff = now;
            prefs.save_preferences();
        }
    }

    /// Take over what the preferences say about how a diff is shown,
    /// when that changed elsewhere: in another tab, or in the preferences.
    fn adopt_preferences(&mut self, cx: &mut Context<Self>) {
        let prefs = cx.global::<Prefs>().preferences.diff.clone();
        if prefs == self.applied {
            return;
        }
        let whitespace = whitespace_of(&prefs);
        let context = self.context();
        self.layout = layout_of(&prefs);
        // Another way of counting whitespace, or the whole file instead of
        // its hunks: the diff is asked for again.
        let reload = whitespace != self.whitespace || self.context() != context;
        self.whitespace = whitespace;
        self.wrap = prefs.wrap;
        self.words = prefs.words;
        self.applied = prefs;
        if reload {
            self.reload(cx);
        } else {
            self.lay_out(cx);
        }
    }

    /// Fetch the two whole files, so that the colours of a hunk come
    /// from a parse that saw what stands above it. A side that is not
    /// there, or is too large to parse whole, is not asked for.
    fn load_texts(&mut self, cx: &mut Context<Self>) {
        self.texts_pending = (None, None);
        let Some(diff) = self.diff.clone() else {
            return;
        };
        if diff.binary
            || diff.too_large
            || diff.hunks.is_empty()
            || diff.is_submodule()
            || diff_syntax::language_for(&diff.path).is_none()
        {
            return;
        }
        let Some((old, new)) = self.sides() else {
            return;
        };
        let fits = |size: Option<u64>| {
            size.is_none_or(|size| {
                usize::try_from(size).is_ok_and(|size| size <= diff_syntax::MAX_WHOLE_BYTES)
            })
        };
        use bitbraid_git::diff::FileChangeKind as Kind;
        let wanted = [
            (false, old, diff.kind != Kind::Added && fits(diff.old_size)),
            (true, new, diff.kind != Kind::Deleted && fits(diff.new_size)),
        ];
        let expected = self.source.clone();
        for (is_new, (side, path), wanted) in wanted {
            if !wanted {
                self.text_arrived(is_new, None, cx);
                continue;
            }
            let request = Request::Command {
                id: "file.bytes".to_owned(),
                params: json!({
                    "path": path,
                    "side": side,
                    "max": diff_syntax::MAX_WHOLE_BYTES,
                }),
            };
            let (expected, shown) = (expected.clone(), diff.clone());
            self.session
                .call_asked(request, cx, move |this, reply, cx| {
                    // Another file, or the same one read again, meanwhile.
                    if this.source != expected
                        || !this
                            .diff
                            .as_ref()
                            .is_some_and(|now| Rc::ptr_eq(now, &shown))
                    {
                        return;
                    }
                    use base64::Engine as _;
                    let text = match reply {
                        Reply::Command(value) => value
                            .get("base64")
                            .and_then(|text| text.as_str())
                            .and_then(|text| {
                                base64::engine::general_purpose::STANDARD.decode(text).ok()
                            })
                            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
                        _ => None,
                    };
                    this.text_arrived(is_new, text, cx);
                });
        }
    }

    /// One whole side is in (or is not to be had); with both, colour again.
    fn text_arrived(&mut self, is_new: bool, text: Option<String>, cx: &mut Context<Self>) {
        if is_new {
            self.texts_pending.1 = Some(text);
        } else {
            self.texts_pending.0 = Some(text);
        }
        if let (Some(old), Some(new)) = (&self.texts_pending.0, &self.texts_pending.1) {
            // With neither, the hunks are all there is to parse.
            self.texts = Some((old.clone(), new.clone()));
            self.texts_pending = (None, None);
            self.colour(cx);
        }
    }

    /// When what is shown is a submodule, ask what lies between the two
    /// commits it is recorded at.
    fn load_submodule(&mut self, cx: &mut Context<Self>) {
        let wanted = self.diff.as_ref().is_some_and(|diff| diff.is_submodule());
        let (true, Some(((old, _), (new, path)))) = (wanted, self.sides()) else {
            self.submodule = None;
            return;
        };
        // The two ids are known from the diff; the rest follows.
        let short = |id: Option<bitbraid_git::Oid>| id.map(|id| id.short(7));
        self.submodule = self.diff.as_ref().map(|diff| SubmoduleState {
            old: short(diff.old_id),
            new: short(diff.new_id),
            ..Default::default()
        });
        let expected = self.source.clone();
        let request = Request::Command {
            id: "submodule.changes".to_owned(),
            params: json!({ "path": path, "old": old, "new": new }),
        };
        self.session.call(request, cx, move |this, reply, cx| {
            if this.source != expected {
                return;
            }
            let Some(state) = &mut this.submodule else {
                return;
            };
            match reply {
                Reply::Command(value) => *state = SubmoduleState::from_reply(&value),
                // The ids from the diff stay; there is nothing between
                // them to show.
                _ => state.answered = true,
            }
            cx.notify();
        });
    }

    /// A submodule: where it went from and to, and the commits between.
    fn render_submodule(&self, state: &SubmoduleState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let path = self
            .diff
            .as_ref()
            .map(|diff| diff.path.clone())
            .unwrap_or_default();
        let list = |title: String, commits: &[(String, String)], more: bool, colour: Hsla| {
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_xs()
                        .font_semibold()
                        .text_color(theme.muted_foreground)
                        .child(title),
                )
                .children(commits.iter().map(|(id, summary)| {
                    // A commit of the submodule: a click opens the
                    // submodule in a tab of its own, on that commit.
                    let (open_path, open_id) = (path.clone(), id.clone());
                    h_flex()
                        .id(SharedString::from(format!("submodule-commit-{id}")))
                        .gap_2()
                        .h(px(LINE_HEIGHT))
                        .px_1()
                        .rounded(theme.radius)
                        .cursor_pointer()
                        .hover(|row| row.bg(theme.accent))
                        .on_click(cx.listener(move |_, _, _, cx| {
                            cx.emit(DiffEvent::OpenSubmoduleCommit {
                                path: open_path.clone(),
                                id: open_id.clone(),
                            })
                        }))
                        .child(
                            div()
                                .flex_none()
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .text_color(colour)
                                .child(id.chars().take(7).collect::<String>()),
                        )
                        .child(div().min_w_0().truncate().text_sm().child(summary.clone()))
                }))
                .when(more, |list| {
                    list.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("diff-submodule-and-more")),
                    )
                })
        };
        let count = |n: usize, more: bool| {
            if more {
                say!("diff-submodule-more-than", n)
            } else {
                format!("{n}")
            }
        };
        // What it is recorded at on each side, in a block of rows on the
        // tint of what was taken out and of what was put in.
        let row = |label: &'static str, value: String, old: bool| {
            h_flex()
                .h(px(SUBMODULE_ROW))
                .px(px(10.))
                .gap_2()
                .items_center()
                .bg(if old {
                    theme.red.opacity(0.18)
                } else {
                    theme.green.opacity(0.18)
                })
                .text_sm()
                .child(
                    div()
                        .w(px(SUBMODULE_LABEL))
                        .flex_none()
                        .font_semibold()
                        .child(label),
                )
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .when(label == say!("diff-submodule-commit"), |id| {
                            id.font_family(theme.mono_font_family.clone())
                        })
                        .child(value),
                )
        };
        let rows = state.rows();
        let mut body = v_flex()
            .id("submodule-diff")
            .size_full()
            .p(px(11.))
            .gap_3()
            .overflow_y_scroll()
            .when(!rows.is_empty(), |body| {
                body.child(
                    v_flex().p(px(10.)).bg(theme.muted).children(
                        rows.into_iter()
                            .map(|(label, value, old)| row(label, value, old)),
                    ),
                )
            });
        let moved = state.old.is_some() && state.new.is_some() && state.old != state.new;
        if !state.answered {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(say!("diff-loading")),
            );
        } else if moved && !state.known {
            body = body.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(say!("diff-submodule-not-here")),
            );
        } else {
            if !state.gained.is_empty() {
                body = body.child(list(
                    say!(
                        "diff-submodule-gained",
                        count = count(state.gained.len(), state.more_gained)
                    ),
                    &state.gained,
                    state.more_gained,
                    theme.success,
                ));
            }
            if !state.lost.is_empty() {
                body = body.child(list(
                    say!(
                        "diff-submodule-lost",
                        count = count(state.lost.len(), state.more_lost)
                    ),
                    &state.lost,
                    state.more_lost,
                    theme.danger,
                ));
            }
        }
        body.into_any_element()
    }

    /// The two sides of the source as the engine names them, and the path
    /// on each.
    fn sides(&self) -> Option<((String, String), (String, String))> {
        let old_path = |path: &String| {
            self.diff
                .as_ref()
                .and_then(|diff| diff.old_path.clone())
                .unwrap_or_else(|| path.clone())
        };
        match self.source.as_ref()? {
            DiffSource::Working { path, side } => {
                let (old, new) = match side {
                    Side::Unstaged => ("index", "worktree"),
                    Side::Staged => ("HEAD", "index"),
                };
                Some((
                    (old.to_owned(), old_path(path)),
                    (new.to_owned(), path.clone()),
                ))
            }
            DiffSource::Commit { id, path, .. } | DiffSource::Revision { id, path, .. } => Some((
                (format!("{}^", id.to_hex()), old_path(path)),
                (id.to_hex(), path.clone()),
            )),
            DiffSource::Range { old, new, path, .. } => Some((
                (range_old(*old), old_path(path)),
                (new.to_hex(), path.clone()),
            )),
            DiffSource::AgainstWorktree { old, path, .. } => Some((
                (old.to_hex(), old_path(path)),
                ("worktree".to_owned(), path.clone()),
            )),
        }
    }

    /// When the file is a picture, fetch both versions to show them.
    fn load_images(&mut self, cx: &mut Context<Self>) {
        let wanted = self
            .diff
            .as_ref()
            .filter(|diff| diff.binary)
            .and_then(|diff| image_diff::format_of(&diff.path));
        let (Some(format), Some((old, new))) = (wanted, self.sides()) else {
            self.image = None;
            return;
        };
        // Keep how the picture was being looked at across a reload.
        let (mode, at) = self
            .image
            .as_ref()
            .map_or((ImageMode::Apart, 0.5), |image| (image.mode, image.at));
        self.image = Some(ImageState {
            old: None,
            new: None,
            answered: 0,
            mode,
            at,
            bounds: Rc::new(std::cell::Cell::new(Bounds::default())),
            span: Rc::new(std::cell::Cell::new(0.0)),
            difference: None,
        });
        let expected = self.source.clone();
        for (is_new, (side, path)) in [(false, old), (true, new)] {
            let request = Request::Command {
                id: "file.bytes".to_owned(),
                params: json!({ "path": path, "side": side }),
            };
            let expected = expected.clone();
            self.session
                .call_asked(request, cx, move |this, reply, cx| {
                    if this.source != expected {
                        return;
                    }
                    let Some(image) = &mut this.image else {
                        return;
                    };
                    use base64::Engine as _;
                    // A side that is not there (an added or deleted file, the
                    // parent of a first commit) is absent.
                    let side = match reply {
                        Reply::Command(value) => value
                            .get("base64")
                            .and_then(|text| text.as_str())
                            .and_then(|text| {
                                base64::engine::general_purpose::STANDARD.decode(text).ok()
                            })
                            .map(|bytes| ImageSide {
                                bytes: bytes.len(),
                                image: std::sync::Arc::new(Image::from_bytes(format, bytes)),
                                shown: None,
                            }),
                        _ => None,
                    };
                    let source = side.as_ref().map(|side| side.image.clone());
                    if is_new {
                        image.new = side;
                    } else {
                        image.old = side;
                    }
                    image.answered = image.answered.saturating_add(1);
                    cx.notify();
                    // Decoded for drawing away from the thread that draws.
                    let Some(source) = source else { return };
                    let renderer = cx.svg_renderer();
                    let decoding = source.clone();
                    let shown = cx.background_spawn(async move {
                        let decoded = decoding.to_image_data(renderer).ok()?;
                        image_diff::bordered(&decoded)
                    });
                    cx.spawn(async move |this, cx| {
                        let shown = shown.await;
                        let _ = this.update(cx, |this, cx| {
                            // For the side that still holds this very file.
                            let Some(image) = &mut this.image else { return };
                            let side = if is_new {
                                &mut image.new
                            } else {
                                &mut image.old
                            };
                            if let Some(side) = side
                                && std::sync::Arc::ptr_eq(&side.image, &source)
                            {
                                side.shown = shown;
                                cx.notify();
                            }
                        });
                    })
                    .detach();
                });
        }
    }

    /// Compare the pixels of the two versions, once.
    fn compute_difference(&mut self, cx: &mut Context<Self>) {
        let renderer = cx.svg_renderer();
        let Some(image) = &mut self.image else { return };
        if image.difference.is_some() {
            return;
        }
        let decoded = |side: &Option<ImageSide>| {
            side.as_ref()
                .and_then(|side| side.image.to_image_data(renderer.clone()).ok())
        };
        let result = match (decoded(&image.old), decoded(&image.new)) {
            (Some(old), Some(new)) => {
                fn pixels(data: &RenderImage) -> Option<image_diff::Pixels<'_>> {
                    let size = data.size(0);
                    Some(image_diff::Pixels {
                        data: data.as_bytes(0)?,
                        width: u32::try_from(size.width.0).ok()?,
                        height: u32::try_from(size.height.0).ok()?,
                    })
                }
                match pixels(&old)
                    .zip(pixels(&new))
                    .and_then(|(old, new)| image_diff::compare(old, new))
                {
                    Some(compared) => {
                        let share = compared.differing as f64 * 100.0 / compared.total as f64;
                        Ok((
                            Image::from_bytes(ImageFormat::Bmp, compared.bmp)
                                .to_image_data(renderer.clone())
                                .ok()
                                .and_then(|decoded| image_diff::bordered(&decoded)),
                            say!(
                                "diff-image-pixels-differ",
                                differing = compared.differing,
                                total = compared.total,
                                share = format!("{share:.1}"),
                                width = compared.size.0,
                                height = compared.size.1
                            ),
                        ))
                    }
                    None => Err(say!("diff-image-too-large").to_owned()),
                }
            }
            _ => Err(say!("diff-image-one-version").to_owned()),
        };
        image.difference = Some(result);
    }

    /// The pointer is at window position `x` over the two versions where
    /// one lies over the other: move the divider or the blend there.
    fn image_drag(&mut self, x: Pixels, cx: &mut Context<Self>) {
        if let Some(image) = &mut self.image {
            let left = image.bounds.get().left() + px(IMAGE_PAD);
            let span = image.span.get();
            if span > 0.0 {
                image.at = (crate::zoom::unpx(x - left) / span).clamp(0.0, 1.0);
                cx.notify();
            }
        }
    }

    /// The picture view: the new version above the old one, each under a
    /// bar that names it and at its own size from the left. The switches
    /// beside the upper name lay one version over the other in the upper
    /// place instead: see-through, with a divider, or as what differs.
    fn render_images(&self, image: &ImageState, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let caption = match (&image.old, &image.new) {
            (Some(old), Some(new)) => say!(
                "diff-sizes",
                old = image_diff::human_size(old.bytes),
                new = image_diff::human_size(new.bytes)
            ),
            (None, Some(new)) => say!(
                "diff-image-size-new",
                size = image_diff::human_size(new.bytes)
            ),
            (Some(old), None) => say!(
                "diff-image-size-deleted",
                size = image_diff::human_size(old.bytes)
            ),
            (None, None) if image.answered < 2 => say!("diff-loading").to_owned(),
            (None, None) => say!("diff-image-neither").to_owned(),
        };
        // A picture is a binary file: the ids of its two versions, short.
        let caption = match self.diff.as_deref().and_then(blob_ids) {
            Some(ids) => format!("{caption}  ·  {ids}"),
            None => caption,
        };
        let both = image.old.is_some() && image.new.is_some();
        let mode = if both { image.mode } else { ImageMode::Apart };
        // A picture is laid out in pixels, from the width its place had
        // when it was last painted: nothing until that is known.
        let area = image.bounds.clone();
        let room = (crate::zoom::unpx(area.get().size.width) - 2.0 * IMAGE_PAD).max(0.0);
        let size_of =
            |shown: &image_diff::Shown| image_diff::own_size(shown.width, shown.height, room);
        // A decoded picture at a size. It is laid out one of its own
        // pixels larger on every side, which is its border, and cut to
        // its place (see `image_diff`).
        let drawn = |shown: &image_diff::Shown, (wide, tall, pixel): (f32, f32, f32)| {
            div()
                .relative()
                .flex_none()
                .w(px(wide))
                .h(px(tall))
                .overflow_hidden()
                .child(
                    img(shown.picture.clone())
                        .absolute()
                        .left(px(-pixel))
                        .top(px(-pixel))
                        .w(px(wide + 2.0 * pixel))
                        .h(px(tall + 2.0 * pixel))
                        .object_fit(ObjectFit::Fill),
                )
        };
        // A version as it has been decoded, with the size it is shown at.
        fn sized(
            side: &Option<ImageSide>,
            room: f32,
        ) -> Option<(&image_diff::Shown, (f32, f32, f32))> {
            let shown = side.as_ref()?.shown.as_ref()?;
            Some((
                shown,
                image_diff::own_size(shown.width, shown.height, room)?,
            ))
        }
        let alone = |side: &Option<ImageSide>| {
            sized(side, room).map(|(shown, size)| drawn(shown, size).into_any_element())
        };
        // What stands under the upper bar, and what is said about it.
        let mut said = String::new();
        let upper: Option<AnyElement> = match mode {
            ImageMode::Apart => alone(&image.new),
            ImageMode::Difference => match &image.difference {
                Some(Ok((difference, summary))) => {
                    said.clone_from(summary);
                    difference.as_ref().and_then(|difference| {
                        Some(drawn(difference, size_of(difference)?).into_any_element())
                    })
                }
                Some(Err(why)) => {
                    said.clone_from(why);
                    None
                }
                None => None,
            },
            ImageMode::Onion | ImageMode::Swipe => {
                sized(&image.new, room)
                    .zip(sized(&image.old, room))
                    .map(|(new, old)| {
                        // Both from the same corner, each at its own size.
                        let (wide, tall) = (new.1.0.max(old.1.0), new.1.1.max(old.1.1));
                        image.span.set(wide);
                        let at = image.at;
                        let layer = || div().absolute().left_0().top_0();
                        let over = div()
                            .id("image-over")
                            .relative()
                            .flex_none()
                            .w(px(wide))
                            .h(px(tall))
                            .cursor_pointer()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                                    this.image_drag(event.position.x, cx)
                                }),
                            )
                            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                                if event.pressed_button == Some(MouseButton::Left) {
                                    this.image_drag(event.position.x, cx);
                                }
                            }));
                        let layered = if mode == ImageMode::Onion {
                            said.push_str(say!("diff-image-drag-overlay"));
                            // The old version over the new one, as see-through
                            // as the pointer says: half, to begin with.
                            over.child(layer().child(drawn(new.0, new.1)))
                                .child(layer().opacity(1.0 - at).child(drawn(old.0, old.1)))
                        } else {
                            said.push_str(say!("diff-image-drag-swipe"));
                            // The new version left of the divider: a window
                            // that wide onto a picture laid out full size.
                            let split = wide * at;
                            over.child(layer().child(drawn(old.0, old.1)))
                                .child(
                                    layer()
                                        .w(px(split))
                                        .h(px(tall))
                                        .overflow_hidden()
                                        .child(drawn(new.0, new.1)),
                                )
                                .child(
                                    layer()
                                        .left(px(split - 1.0))
                                        .w(px(2.0))
                                        .h(px(tall))
                                        .bg(theme.primary),
                                )
                        };
                        layered.into_any_element()
                    })
            }
        };
        let caption = if said.is_empty() {
            caption
        } else {
            format!("{said}  ·  {caption}")
        };
        // The bar over a version: its name, and for the upper one the
        // switches and what there is to say.
        let bar = |name: &'static str| {
            h_flex()
                .w_full()
                .h(px(IMAGE_BAR))
                .flex_none()
                .px(px(IMAGE_PAD))
                .gap_2()
                .items_center()
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted)
                .text_xs()
                .child(div().flex_none().font_semibold().child(name))
        };
        let note = |text: String| {
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_right()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        // Where a version stands: a little off the pane's own ground,
        // with a rule under it.
        let place = |picture: Option<AnyElement>| {
            div()
                .w_full()
                .flex_none()
                .p(px(IMAGE_PAD))
                .border_b_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.4))
                .children(picture)
        };
        let switches = both.then(|| {
            h_flex()
                .flex_none()
                .gap_1()
                .children(
                    ImageMode::OVER
                        .iter()
                        .enumerate()
                        .map(|(ix, (which, word))| {
                            let which = *which;
                            chosen(
                                Button::new(("image-mode", ix))
                                    .outline()
                                    .xsmall()
                                    .label(word.text()),
                                mode == which,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, _, cx| {
                                    if let Some(image) = &mut this.image {
                                        image.mode = image.mode.switched(which);
                                    }
                                    if which == ImageMode::Difference {
                                        this.compute_difference(cx);
                                    }
                                    cx.notify();
                                },
                            ))
                        }),
                )
        });
        let mut parts: Vec<AnyElement> = Vec::new();
        let mut caption = Some(caption);
        if self.file_view {
            // The file as it is: the picture of that side alone, or of the
            // other one when it is gone.
            let side = if image.new.is_some() {
                &image.new
            } else {
                &image.old
            };
            if side.is_some() {
                parts.push(place(alone(side)).into_any_element());
                caption = None;
            }
        } else if image.new.is_some() {
            parts.push(
                bar(say!("diff-image-new"))
                    .children(switches)
                    .children(caption.take().map(note))
                    .into_any_element(),
            );
            parts.push(place(upper).into_any_element());
        }
        if image.old.is_some() && !self.file_view {
            parts.push(
                bar(say!("diff-image-old"))
                    .children(caption.take().map(note))
                    .into_any_element(),
            );
            parts.push(place(alone(&image.old)).into_any_element());
        }
        // Neither version: what is the matter, where the first bar would be.
        if let Some(caption) = caption {
            parts.push(
                div()
                    .p_2()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(caption)
                    .into_any_element(),
            );
        }
        div()
            .relative()
            .size_full()
            // How wide the pictures' place is; drawn again once that is
            // known or has changed.
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        if area.get() != bounds {
                            area.set(bounds);
                            window.request_animation_frame();
                        }
                    },
                )
                .absolute()
                .size_full(),
            )
            .child(
                v_flex()
                    .id("image-area")
                    .size_full()
                    .overflow_y_scroll()
                    .children(parts),
            )
            .into_any_element()
    }

    /// Work out the syntax colours of the diff for the current theme,
    /// away from the thread that draws: parsing a file takes a tenth of a
    /// second and more. The lines show plain until the colours are in.
    fn colour(&mut self, cx: &mut Context<Self>) {
        self.syntax_dark = cx.theme().is_dark();
        self.syntax_theme = cx.theme().highlight_theme.clone();
        self.colour_run = self.colour_run.wrapping_add(1);
        let run = self.colour_run;
        let wanted = self.diff.as_deref().filter(|diff| {
            !diff.binary
                && !diff.hunks.is_empty()
                && diff_syntax::language_for(&diff.path).is_some()
        });
        let Some(diff) = wanted.cloned() else {
            self.syntax = Rc::new(Styles::new());
            return;
        };
        let (old, new) = self.texts.clone().unwrap_or_default();
        let theme = cx.theme().highlight_theme.clone();
        let colours = cx.background_spawn(async move {
            diff_syntax::highlight_whole(&diff, old.as_deref(), new.as_deref(), &theme)
        });
        cx.spawn(async move |this, cx| {
            let styles = colours.await;
            let _ = this.update(cx, |this, cx| {
                // Another diff, or another theme, was asked for meanwhile.
                if this.colour_run == run {
                    this.syntax = Rc::new(styles);
                    cx.notify();
                }
            });
        })
        .detach();
    }

    /// The width in pixels the text of a row has, for the layout: what is
    /// left of the list beside the line numbers. Nothing before the list
    /// was painted once.
    fn text_width(&self) -> f32 {
        let list = crate::zoom::unpx(self.list_bounds.get().size.width);
        if list <= 0.0 {
            return 0.0;
        }
        if self.file_view {
            return list - NUMBER_WIDTH - MARKER_WIDTH;
        }
        match self.layout {
            Layout::Hunks | Layout::Inline => list - NUMBER_WIDTH * 2.0 - MARKER_WIDTH,
            Layout::Split => (list - SPLIT_GAP) / 2.0 - NUMBER_WIDTH - MARKER_WIDTH,
        }
    }

    /// Where the lines of the rows are cut to fit the width, when long
    /// lines wrap: for each row its old (or only) side and its new side.
    fn cut_rows(&self, rows: &[Row], width: f32, cx: &App) -> Vec<(Vec<usize>, Vec<usize>)> {
        let Some(diff) = self.diff.as_deref() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let font = font(theme.mono_font_family.clone());
        // Too narrow to cut anything for (or not measured yet).
        if width < 40.0 {
            return Vec::new();
        }
        let mut wrapper = cx.text_system().line_wrapper(font, px(TEXT_SIZE));
        let mut cut = |hunk: usize, cell: Option<&Cell>| -> Vec<usize> {
            let Some(line) = cell.and_then(|cell| diff.hunks.get(hunk)?.lines.get(cell.line))
            else {
                return Vec::new();
            };
            // What is said after the text of a line takes room too.
            let said = if line.no_newline {
                90.0
            } else if ending_mark(line, &diff.line_endings).is_some() {
                30.0
            } else {
                0.0
            };
            let fragments = [LineFragment::Text { text: &line.text }];
            wrapper
                .wrap_line(
                    &fragments,
                    px((width - said).max(40.0)),
                    IndentAdjustment::NoIndent,
                )
                .map(|boundary| boundary.ix)
                .filter(|at| line.text.is_char_boundary(*at))
                .collect()
        };
        rows.iter()
            .map(|row| match row {
                Row::Hunk { .. } => (Vec::new(), Vec::new()),
                Row::Line { hunk, cell } | Row::Plain { hunk, cell } => {
                    (cut(*hunk, Some(cell)), Vec::new())
                }
                Row::Pair { hunk, left, right } => {
                    (cut(*hunk, left.as_ref()), cut(*hunk, right.as_ref()))
                }
            })
            .collect()
    }

    /// Lay the diff out as rows again.
    fn rebuild(&mut self, cx: &mut Context<Self>) {
        self.lay_out(cx);
        cx.notify();
    }

    /// Whether the lines of `diff` are left out for what is said of it: a
    /// change of line endings alone, shown as the diff (not as the file),
    /// whose lines were not asked for.
    fn hides_endings_lines(&self, diff: &FileDiff) -> bool {
        diff.line_endings.only_change && !diff.binary && !self.file_view && !self.endings_lines
    }

    /// Show the lines of a change of line endings alone.
    fn show_endings_lines(&mut self, cx: &mut Context<Self>) {
        self.endings_lines = true;
        self.rebuild(cx);
    }

    /// Lay the diff out as rows: which rows there are, where their lines
    /// wrap, how tall each is.
    fn lay_out(&mut self, cx: &App) {
        let rows = match &self.diff {
            Some(diff) if !diff.binary && self.file_view => diff_doc::file_rows(diff),
            // A change of line endings alone is said, not shown line by
            // line, until that is asked for.
            Some(diff) if self.hides_endings_lines(diff) => Vec::new(),
            Some(diff) if !diff.binary => diff_doc::rows(diff, self.layout, self.words),
            _ => Vec::new(),
        };
        self.cut_width = self.text_width();
        let cuts = if self.wrap {
            self.cut_rows(&rows, self.cut_width, cx)
        } else {
            Vec::new()
        };
        let height = |ix: usize, row: &Row| match row {
            Row::Hunk { hunk } => heading_height(*hunk),
            _ => {
                let lines = cuts
                    .get(ix)
                    .map_or(0, |(left, right)| left.len().max(right.len()))
                    .saturating_add(1);
                LINE_HEIGHT * lines as f32
            }
        };
        self.longest = self
            .diff
            .as_deref()
            .map(diff_doc::longest_line)
            .unwrap_or(0);
        self.changes = Rc::new(match &self.diff {
            Some(diff) => diff_doc::change_starts(diff, &rows),
            None => Vec::new(),
        });
        self.change_at = None;
        self.cursor = None;
        self.marks = Rc::new(match &self.diff {
            Some(diff) => diff_doc::marks(diff, &rows),
            None => Vec::new(),
        });
        let mut offsets = Vec::with_capacity(rows.len().saturating_add(1));
        let mut at = 0.0f32;
        for (ix, row) in rows.iter().enumerate() {
            offsets.push(at);
            at += height(ix, row);
        }
        offsets.push(at);
        self.offsets = Rc::new(offsets);
        self.sizes = crate::zoom::Sizes::new(
            rows.iter()
                .enumerate()
                .map(|(ix, row)| size(px(0.), px(height(ix, row))))
                .collect(),
        );
        self.mini = Rc::new(match &self.diff {
            Some(diff) if self.file_view => rows
                .iter()
                .map(|row| match row {
                    Row::Plain { hunk, cell } => diff
                        .hunks
                        .get(*hunk)
                        .and_then(|hunk| hunk.lines.get(cell.line))
                        .map(|line| mini_runs(&line.text))
                        .unwrap_or_default(),
                    _ => Vec::new(),
                })
                .collect(),
            _ => Vec::new(),
        });
        if *self.rows != rows {
            self.text.clear();
        }
        self.rows = Rc::new(rows);
        self.cuts = Rc::new(cuts);
    }

    /// Show a source, asking the session for its diff unless the one it
    /// came with will do.
    pub fn show(&mut self, source: Option<DiffSource>, cx: &mut Context<Self>) {
        self.selected.clear();
        self.error = None;
        self.scroll_x = 0.0;
        // The lines of a change of line endings, once asked for, stay
        // shown while it is the same file.
        if self.source != source {
            self.endings_lines = false;
        }
        // A guard that was waved through holds for that file alone.
        if self.load_anyway != source {
            self.load_anyway = None;
        }
        let waved = self.load_anyway.is_some();
        match &source {
            None => {
                self.source = None;
                self.scroll_x = 0.0;
                self.set_diff(None, cx);
            }
            Some(
                DiffSource::Commit { diff, .. }
                | DiffSource::Range { diff, .. }
                | DiffSource::AgainstWorktree { diff, .. },
            ) if self.whitespace == Whitespace::Exact
                && self.context() == CONTEXT
                && !(diff.too_large && waved) =>
            {
                // As the commit's file list computed it.
                self.source = source.clone();
                self.set_diff(Some((**diff).clone()), cx);
            }
            Some(_) => {
                // Where the file was before, for one that was moved.
                let moved = |diff: &FileDiff| diff.old_path.clone();
                let (old, new, path, old_path) = match &source {
                    Some(DiffSource::Working { path, side }) => {
                        let (old, new) = match side {
                            Side::Unstaged => ("index", "worktree"),
                            Side::Staged => ("HEAD", "index"),
                        };
                        // Moved in the index: the last commit has it
                        // under its old name.
                        let from = match side {
                            Side::Staged => self.staged_moves.get(path).cloned(),
                            Side::Unstaged => None,
                        };
                        (old.to_owned(), new.to_owned(), path.clone(), from)
                    }
                    Some(DiffSource::Commit { id, path, diff }) => (
                        format!("{}^", id.to_hex()),
                        id.to_hex(),
                        path.clone(),
                        moved(diff),
                    ),
                    Some(DiffSource::Revision { id, path, old_path }) => (
                        format!("{}^", id.to_hex()),
                        id.to_hex(),
                        path.clone(),
                        old_path.clone(),
                    ),
                    Some(DiffSource::Range {
                        old,
                        new,
                        path,
                        diff,
                    }) => (range_old(*old), new.to_hex(), path.clone(), moved(diff)),
                    Some(DiffSource::AgainstWorktree { old, path, diff }) => (
                        old.to_hex(),
                        "worktree".to_owned(),
                        path.clone(),
                        moved(diff),
                    ),
                    None => return,
                };
                self.source = source.clone();
                self.loading = true;
                let request = Request::DiffFile {
                    old,
                    new,
                    path,
                    whitespace: self.whitespace,
                    context: self.context(),
                    old_path,
                    max_bytes: (!waved).then_some(DIFF_GUARD_BYTES),
                };
                let expected = source.clone();
                self.session
                    .call_asked(request, cx, move |this, reply, cx| {
                        if this.source != expected {
                            return;
                        }
                        this.loading = false;
                        match reply {
                            Reply::DiffFile(diff) => this.set_diff(diff, cx),
                            Reply::Error(err) => this.error = Some(err),
                            _ => {}
                        }
                        cx.notify();
                    });
            }
        }
        cx.notify();
    }

    /// Reload the current working source (after a mutation).
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        if let Some(DiffSource::Working { .. }) = &self.source {
            self.reload(cx);
        }
    }

    /// Take over which files the index holds under a new name, as the
    /// status that just arrived says it. The file on show is asked for
    /// again by `refresh`, which follows every status.
    pub fn set_staged_moves(&mut self, moves: BTreeMap<String, String>) {
        self.staged_moves = moves;
    }

    /// What is on show, for the off-screen tests.
    pub fn probe(&self) -> DiffProbe {
        DiffProbe {
            loading: self.loading,
            moved_from: self.diff.as_ref().and_then(|diff| diff.old_path.clone()),
            added: self.diff.as_ref().map_or(0, |diff| diff.additions),
            removed: self.diff.as_ref().map_or(0, |diff| diff.deletions),
            layout: words_for(self.layout, self.whitespace).0,
            file_view: self.file_view,
            cursor: self.cursor,
            selected: self.selected.len(),
            map_hovered: self.minimap_hovered,
            scrolled: (-crate::zoom::unpx(self.scroll.offset().y)).round() as i32,
            headings: self
                .rows
                .iter()
                .filter(|row| matches!(row, Row::Hunk { .. }))
                .count(),
            lines: self
                .rows
                .iter()
                .filter(|row| !matches!(row, Row::Hunk { .. }))
                .count(),
            picture: self.image.as_ref().map(|image| match image.mode {
                ImageMode::Apart => "apart",
                ImageMode::Onion => "overlay",
                ImageMode::Swipe => "swipe",
                ImageMode::Difference => "difference",
            }),
            text_selected: self.selected_text().unwrap_or_default(),
            line_endings: self
                .diff
                .as_deref()
                .filter(|_| self.shape().text)
                .and_then(|diff| line_endings_label(&diff.line_endings))
                .map(|(label, _)| label),
            endings_only: self
                .diff
                .as_deref()
                .is_some_and(|diff| self.hides_endings_lines(diff)),
        }
    }

    /// Ask for the current source again, keeping the place.
    fn reload(&mut self, cx: &mut Context<Self>) {
        let source = self.source.clone();
        let scroll_x = self.scroll_x;
        self.show(source, cx);
        self.scroll_x = scroll_x;
    }

    /// How many unchanged lines are asked for around a change: every one
    /// for the whole file (the file view, and the layouts in one or two
    /// columns), three for the hunks.
    fn context(&self) -> u32 {
        if self.file_view || self.layout.whole() {
            u32::MAX
        } else {
            CONTEXT
        }
    }

    /// Change the layout: the diff is asked for again when that changes
    /// how much of the file it has to hold.
    fn set_layout(&mut self, layout: Layout, cx: &mut Context<Self>) {
        let context = self.context();
        self.layout = layout;
        self.remember(cx);
        if self.context() != context {
            self.reload(cx);
        } else {
            self.rebuild(cx);
        }
    }

    /// The minimap was clicked or dragged at `position`: bring that part
    /// of the file to the middle of the view.
    fn minimap_jump(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let strip = self.minimap_bounds.get();
        let height = crate::zoom::unpx(strip.size.height);
        let total = self.offsets.last().copied().unwrap_or(0.0);
        if height <= 0.0 || total <= height {
            return;
        }
        let y = crate::zoom::unpx(position.y - strip.top());
        let on_mini = self.file_view && crate::zoom::unpx(position.x - strip.left()) < MINI_WIDTH;
        let wanted = if on_mini {
            // The line under the pointer, whatever part of the miniature
            // is drawn.
            let scrolled = -crate::zoom::unpx(self.scroll.offset().y);
            let row = MiniGeometry::new(self.rows.len(), total, height).row_at(y, scrolled);
            self.offsets.get(row).copied().unwrap_or(0.0)
        } else {
            (y / height).clamp(0.0, 1.0) * total
        };
        let top = (wanted - height / 2.0).clamp(0.0, total - height);
        let offset = self.scroll.offset();
        self.scroll.set_offset(point(offset.x, px(-top)));
        cx.notify();
    }

    /// The strip at the right edge, in the place of a scroll bar: a rule,
    /// and the mark of the cursor's line. Beside the whole file a map
    /// follows it, removed lines in its left half and added ones in its
    /// right, with what is in view over it. The hunks have the strip alone,
    /// and show what is in view on it while the pointer is there. A click
    /// or a drag anywhere on it brings that part of the file to the middle.
    fn render_minimap(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let (added, removed) = (theme.green.opacity(0.7), theme.red.opacity(0.7));
        let (view, rule, cursor_colour) = (
            theme.foreground.opacity(0.14),
            theme.border,
            theme.foreground.opacity(0.55),
        );
        let overview = self.layout.whole() && !self.file_view;
        let miniature = self.file_view;
        let show_view = overview || self.minimap_hovered;
        // The strip's own part is right of the miniature, when there is one.
        let ruler = if miniature { MINI_WIDTH } else { 0.0 };
        let (mini, rows) = (self.mini.clone(), self.rows.len());
        let ink = theme.foreground.opacity(0.4);
        let (marks, offsets) = (self.marks.clone(), self.offsets.clone());
        let cursor = self
            .cursor
            .and_then(|row| Some((*offsets.get(row)?, *offsets.get(row.saturating_add(1))?)));
        let painted = self.minimap_bounds.clone();
        let scrolled = -crate::zoom::unpx(self.scroll.offset().y);
        let width = if overview {
            RULER_WIDTH + OVERVIEW_WIDTH
        } else if miniature {
            MINI_WIDTH + RULER_WIDTH
        } else {
            RULER_WIDTH
        };
        div()
            .id("diff-minimap")
            .flex_none()
            .w(px(width))
            .h_full()
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.minimap_hovered = *hovered;
                cx.notify();
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, _, cx| {
                    this.minimap_jump(event.position, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.minimap_jump(event.position, cx);
                }
            }))
            .child(
                canvas(
                    |_, _, _| (),
                    move |bounds, _, window, _| {
                        painted.set(bounds);
                        let height = crate::zoom::unpx(bounds.size.height);
                        let total = offsets.last().copied().unwrap_or(0.0);
                        if height <= 0.0 || total <= 0.0 {
                            return;
                        }
                        let geometry = MapGeometry::new(total, height);
                        let quad = |left: f32, top: f32, wide: f32, tall: f32, color: Hsla| {
                            fill(
                                Bounds::new(
                                    point(bounds.left() + px(left), bounds.top() + px(top)),
                                    size(px(wide), px(tall)),
                                ),
                                color,
                            )
                        };
                        window.paint_quad(quad(ruler, 0.0, 1.0, height, rule));
                        if miniature {
                            let _ = rows;
                            paint_miniature(
                                bounds,
                                &mini,
                                (total, scrolled),
                                |row| offsets.get(row).copied().unwrap_or(total),
                                (ink, view, rule),
                                show_view,
                                window,
                            );
                        }
                        if overview {
                            for (start, len, mark) in marks.iter() {
                                let from = offsets.get(*start).copied().unwrap_or(0.0);
                                let to = offsets
                                    .get(start.saturating_add(*len))
                                    .copied()
                                    .unwrap_or(total);
                                let (top, tall) = geometry.mark(from, to, MARK_MIN);
                                let (left, color) = match mark {
                                    Mark::Removed => (RULER_WIDTH, removed),
                                    Mark::Added => (RULER_WIDTH + OVERVIEW_WIDTH / 2.0, added),
                                };
                                window.paint_quad(quad(
                                    left,
                                    top,
                                    OVERVIEW_WIDTH / 2.0,
                                    tall,
                                    color,
                                ));
                            }
                        }
                        if show_view && !miniature {
                            let (top, tall) = geometry.view(scrolled);
                            let left = if overview { RULER_WIDTH } else { 1.0 };
                            let wide = if overview {
                                OVERVIEW_WIDTH
                            } else {
                                RULER_WIDTH - 1.0
                            };
                            window.paint_quad(quad(left, top, wide, tall, view));
                        }
                        if let Some((from, to)) = cursor {
                            let (top, _) = geometry.mark(from, to, CURSOR_MARK);
                            window.paint_quad(quad(
                                ruler + 1.0,
                                top,
                                RULER_WIDTH - 1.0,
                                CURSOR_MARK,
                                cursor_colour,
                            ));
                        }
                    },
                )
                .size_full(),
            )
            .test_support()
    }

    /// Put the cursor on a row: its mark goes on the strip at the right.
    fn put_cursor(&mut self, row: usize, cx: &mut Context<Self>) {
        if self.cursor != Some(row) {
            self.cursor = Some(row);
            cx.notify();
        }
    }

    /// Jump to the next or previous run of changes, stopping at the ends.
    fn step_change(&mut self, forward: bool, cx: &mut Context<Self>) {
        let Some(last) = self.changes.len().checked_sub(1) else {
            return;
        };
        let next = match self.change_at {
            None if forward => 0,
            None => last,
            Some(at) if forward => at.saturating_add(1).min(last),
            Some(at) => at.saturating_sub(1),
        };
        self.change_at = Some(next);
        if let Some(row) = self.changes.get(next).copied() {
            self.cursor = Some(row);
            // Only as far as needed: going down the change comes in at the
            // bottom, whole when it fits; going up it comes in at the top.
            let target = if forward {
                change_end(&self.marks, row, self.changes.get(next + 1).copied())
            } else {
                row
            };
            self.scroll.scroll_to_item(target, ScrollStrategy::Top);
        }
        cx.notify();
    }

    fn cycle_whitespace(&mut self, cx: &mut Context<Self>) {
        self.whitespace = match self.whitespace {
            Whitespace::Exact => Whitespace::IgnoreTrailing,
            Whitespace::IgnoreTrailing => Whitespace::IgnoreAll,
            // Ignoring a carriage return at the end, which only the
            // preferences choose, goes back to exact as well.
            Whitespace::IgnoreAll | Whitespace::IgnoreCrAtEol => Whitespace::Exact,
        };
        self.remember(cx);
        self.reload(cx);
    }

    /// Compare the file although it is large.
    fn load_large(&mut self, cx: &mut Context<Self>) {
        self.load_anyway = self.source.clone();
        self.reload(cx);
    }

    /// Wrap long lines, or let them run off to the right.
    pub fn toggle_wrap(&mut self, cx: &mut Context<Self>) {
        self.wrap = !self.wrap;
        self.scroll_x = 0.0;
        self.remember(cx);
        self.rebuild(cx);
    }

    fn act(
        &mut self,
        command: &str,
        hunk: usize,
        lines: Option<Vec<usize>>,
        cx: &mut Context<Self>,
    ) {
        let Some(DiffSource::Working { path, .. }) = &self.source else {
            return;
        };
        // The engine cuts the file into hunks again: with the context shown
        // here, so that the hunk and line numbers mean the same there.
        let params = json!({
            "path": path,
            "selection": [{ "hunk": hunk, "lines": lines }],
            "context": self.context(),
        });
        self.session.call(
            Request::Command {
                id: command.to_owned(),
                params,
            },
            cx,
            |this, reply, cx| {
                if let Reply::Error(err) = reply {
                    this.error = Some(err);
                }
                this.selected.clear();
                cx.emit(DiffEvent::Mutated);
                this.refresh(cx);
            },
        );
    }

    /// Show what the view shows in the diff tool: the command from the
    /// preferences, or the tool git is configured with.
    fn open_external(&mut self, cx: &mut Context<Self>) {
        let command = cx.global::<Prefs>().preferences.external_diff.clone();
        let params = match &self.source {
            Some(DiffSource::Working { path, side }) => json!({
                "path": path,
                "staged": *side == Side::Staged,
                "command": command,
            }),
            Some(DiffSource::Commit { id, path, .. } | DiffSource::Revision { id, path, .. }) => {
                json!({ "path": path, "commit": id, "command": command })
            }
            // Two commits that were picked: what differs between them;
            // without an older one, against nothing.
            Some(DiffSource::Range { old, new, path, .. }) => {
                json!({ "path": path, "from": old, "to": new, "command": command })
            }
            // The tool is handed commits, and the working tree is none.
            Some(DiffSource::AgainstWorktree { .. }) => {
                self.error = Some(say!("diff-external-worktree").to_owned());
                cx.notify();
                return;
            }
            None => return,
        };
        self.error = None;
        self.session.call_asked(
            Request::Command {
                id: "tool.diff".to_owned(),
                params,
            },
            cx,
            |this, reply, cx| {
                if let Reply::Error(err) = reply {
                    this.error = Some(err);
                    cx.notify();
                }
            },
        );
        cx.notify();
    }

    fn hunk_actions(&self, hunk_ix: usize, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let Some(DiffSource::Working { side, .. }) = &self.source else {
            return Vec::new();
        };
        if self.whitespace != Whitespace::Exact {
            // These are not the hunks the index would be changed by.
            return Vec::new();
        }
        // A file git does not track yet has nothing to stage by the hunk:
        // the file is staged whole.
        let untracked = *side == Side::Unstaged
            && self
                .diff
                .as_deref()
                .is_some_and(|diff| diff.kind == bitbraid_git::diff::FileChangeKind::Added);
        if untracked {
            return Vec::new();
        }
        // The buttons are about the hunk; lines are acted on from their menu.
        hunk_buttons(*side, false)
            .into_iter()
            .map(|button| {
                let lines: Option<Vec<usize>> = None;
                let command = button.command;
                // Outlined in the colour of what it does: taking something
                // back in the colour of danger, going ahead in the other.
                let outlined = Button::new((button.id, hunk_ix)).outline();
                if button.takes_back {
                    outlined.danger()
                } else {
                    outlined.success()
                }
                .xsmall()
                .px(px(HUNK_BUTTON_PADDING))
                .label(button.label)
                .on_click(
                    cx.listener(move |this, _, _, cx| {
                        this.act(command, hunk_ix, lines.clone(), cx)
                    }),
                )
                .into_any_element()
            })
            .collect()
    }

    /// The two sides of a binary file: how large each is and the id git
    /// gives its content, with a way to take the id along.
    fn render_binary(&self, diff: &FileDiff, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let side = |key: &'static str, label: &'static str, said: Option<(String, String)>| {
            let row = h_flex().gap_3().items_center().text_sm().child(
                div()
                    .w(px(52.))
                    .flex_none()
                    .text_color(theme.muted_foreground)
                    .child(label),
            );
            match said {
                Some((size, id)) => {
                    let copied = id.clone();
                    row.child(div().w(px(86.)).flex_none().child(size))
                        .child(
                            div()
                                .font_family(theme.mono_font_family.clone())
                                .text_xs()
                                .child(id),
                        )
                        .child(
                            Button::new(SharedString::from(format!("copy-blob-{key}")))
                                .accessibility_label(say!("diff-binary-copy-id"))
                                .ghost()
                                .xsmall()
                                .icon(IconName::Copy)
                                .tooltip(say!("diff-binary-copy-id"))
                                .on_click(move |_, _, cx| {
                                    cx.write_to_clipboard(ClipboardItem::new_string(
                                        copied.clone(),
                                    ));
                                }),
                        )
                }
                None => row.child(
                    div()
                        .text_color(theme.muted_foreground)
                        .child(say!("diff-binary-not-there")),
                ),
            }
        };
        v_flex()
            .px_3()
            .pb_2()
            .gap_1()
            .child(side(
                "before",
                say!("diff-binary-before"),
                binary_side(diff.old_size, diff.old_id),
            ))
            .child(side(
                "after",
                say!("diff-binary-after"),
                binary_side(diff.new_size, diff.new_id),
            ))
            .into_any_element()
    }

    /// Horizontal wheel (or shift+wheel) moves the text sideways.
    fn scroll_sideways(&mut self, event: &ScrollWheelEvent, cx: &mut Context<Self>) {
        // Wrapped lines have nothing off to the side.
        if self.wrap {
            return;
        }
        let delta = event.delta.pixel_delta(px(20.));
        let dx = if event.modifiers.shift && delta.x == px(0.) {
            delta.y
        } else {
            delta.x
        };
        if dx == px(0.) {
            return;
        }
        // A monospace character is about six tenths of the font size wide.
        let char_width = TEXT_SIZE * 0.6;
        let max = (self.longest as f32 * char_width - 240.0).max(0.0);
        self.scroll_x = (self.scroll_x - crate::zoom::unpx(dx)).clamp(0.0, max);
        cx.notify();
    }

    /// The text of a line with its syntax colours and its emphasis, shifted
    /// by the sideways scroll, and what of it is selected. `place` is its
    /// row and column.
    fn render_text(
        &self,
        at: (usize, usize),
        place: (usize, Column),
        line: &Line,
        emphasis: &[Range<usize>],
        cuts: &[usize],
        cx: &App,
    ) -> Div {
        let theme = cx.theme();
        let (row, column) = place;
        let selected = self.text.in_row(row, column, line.text.len());
        let colour = theme.selection;
        let mark = match line.kind {
            LineKind::Added => theme.green.opacity(0.35),
            LineKind::Removed => theme.red.opacity(0.35),
            LineKind::Context => theme.transparent,
        };
        let highlights: Vec<(Range<usize>, HighlightStyle)> = emphasis
            .iter()
            .filter(|range| {
                range.start < range.end
                    && line.text.is_char_boundary(range.start)
                    && line.text.is_char_boundary(range.end)
            })
            .map(|range| {
                (
                    range.clone(),
                    HighlightStyle {
                        background_color: Some(mark),
                        ..Default::default()
                    },
                )
            })
            .collect();
        let colours = self.syntax.get(&at).cloned().unwrap_or_default();
        let highlights: Vec<(Range<usize>, HighlightStyle)> =
            combine_highlights(colours, highlights).collect();
        if cuts.is_empty() {
            return div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .cursor_text()
                .child(
                    div()
                        .ml(px(-self.scroll_x))
                        .whitespace_nowrap()
                        .child(LineText::new(
                            StyledText::new(line.text.clone()).with_highlights(highlights),
                            (row, column, 0),
                            selected,
                            colour,
                            self.text.drawn(),
                        )),
                );
        }
        // Wrapped: each piece of the line is a line of its own.
        let pieces = diff_doc::pieces(line.text.len(), cuts);
        let last = pieces.len().saturating_sub(1);
        div()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .cursor_text()
            .children(pieces.into_iter().enumerate().filter_map(|(at, piece)| {
                let text = line.text.get(piece.clone())?.to_owned();
                Some(
                    div()
                        .h(px(LINE_HEIGHT))
                        .whitespace_nowrap()
                        .child(LineText::new(
                            StyledText::new(text)
                                .with_highlights(diff_doc::cut_spans(&highlights, &piece)),
                            (row, column, piece.start),
                            text_select::in_piece(&selected, &piece, at == last),
                            colour,
                            self.text.drawn(),
                        )),
                )
            }))
    }

    fn number(value: Option<u32>, cx: &App) -> Div {
        div()
            .w(px(NUMBER_WIDTH))
            .h(px(LINE_HEIGHT))
            .flex_none()
            .text_right()
            .text_color(cx.theme().muted_foreground)
            .child(value.map(|n| n.to_string()).unwrap_or_default())
    }

    /// Whether the lines of the diff can be staged, unstaged or discarded
    /// from the viewer: a file of the worktree, compared exactly.
    fn acts_on_lines(&self) -> bool {
        self.whitespace == Whitespace::Exact
            && matches!(self.source, Some(DiffSource::Working { .. }))
    }

    /// The pointer went down on a line: the cursor goes there, and a
    /// changed line is selected (with the other key held, added to or
    /// taken out of the selection), and a drag can begin from it.
    fn press_line(
        &mut self,
        row: usize,
        at: (usize, usize),
        changed: bool,
        toggle: bool,
        cx: &mut Context<Self>,
    ) {
        self.put_cursor(row, cx);
        if !changed || !self.acts_on_lines() {
            self.drag_from = None;
            return;
        }
        if toggle {
            if !self.selected.remove(&at) {
                self.selected.insert(at);
            }
        } else {
            self.selected = BTreeSet::from([at]);
        }
        self.drag_from = Some(at);
        cx.notify();
    }

    /// The pointer, held down, is over a line: what lies between it and
    /// where the drag began, in that hunk, is the selection.
    fn drag_over_line(&mut self, at: (usize, usize), cx: &mut Context<Self>) {
        let Some((hunk, from)) = self.drag_from else {
            return;
        };
        if hunk != at.0 {
            return;
        }
        let Some(found) = self.diff.as_deref().and_then(|diff| diff.hunks.get(hunk)) else {
            return;
        };
        let lines: BTreeSet<(usize, usize)> = dragged_lines(found, from, at.1)
            .into_iter()
            .map(|line| (hunk, line))
            .collect();
        if lines != self.selected {
            self.selected = lines;
            cx.notify();
        }
    }

    /// The menu of a line: staging or unstaging and discarding it (or the
    /// selection it is in), copying it, the palette.
    fn line_menu(
        menu: PopupMenu,
        side: Option<Side>,
        lines: usize,
        (changed, text): (bool, bool),
        at: (usize, usize),
    ) -> PopupMenu {
        let act = |command: &'static str| -> Box<dyn Action> {
            Box::new(ActOnLines {
                command: command.to_owned(),
                hunk: at.0,
                line: at.1,
            })
        };
        let mut menu = menu;
        // What is selected of the text, before what is done with lines.
        if text {
            menu = menu.menu(say!("diff-copy"), Box::new(CopyText)).separator();
        }
        if let Some(side) = side.filter(|_| changed) {
            for button in hunk_buttons(side, true) {
                let label = match (button.command, lines > 1) {
                    ("discard.hunks", false) => say!("diff-discard-line"),
                    ("stage.hunks", false) => say!("diff-stage-line"),
                    ("unstage.hunks", false) => say!("diff-unstage-line"),
                    _ => button.label,
                };
                menu = menu.menu(label, act(button.command));
            }
            menu = menu.separator();
        }
        menu.menu(
            if lines > 1 {
                say!("diff-copy-lines")
            } else {
                say!("diff-copy-line")
            },
            Box::new(CopyLines {
                hunk: at.0,
                line: at.1,
            }),
        )
        .separator()
        .menu(say!("diff-palette"), Box::new(crate::keymap::TogglePalette))
    }

    /// The lines a menu of a line is about: the selection when the line is
    /// in it, else the line alone.
    fn menu_lines(&self, at: (usize, usize)) -> Vec<(usize, usize)> {
        if self.selected.contains(&at) {
            self.selected.iter().copied().collect()
        } else {
            vec![at]
        }
    }

    /// Stage, unstage or discard the lines a menu was opened on.
    fn act_on_lines(&mut self, action: &ActOnLines, cx: &mut Context<Self>) {
        let at = (action.hunk, action.line);
        let lines: Vec<usize> = self
            .menu_lines(at)
            .into_iter()
            .filter(|(hunk, _)| *hunk == action.hunk)
            .map(|(_, line)| line)
            .collect();
        let command: &'static str = match action.command.as_str() {
            "stage.hunks" => "stage.hunks",
            "unstage.hunks" => "unstage.hunks",
            "discard.hunks" => "discard.hunks",
            _ => return,
        };
        self.act(command, action.hunk, Some(lines), cx);
    }

    /// Copy the text of the lines a menu was opened on.
    fn copy_lines(&mut self, action: &CopyLines, cx: &mut Context<Self>) {
        let Some(diff) = self.diff.clone() else {
            return;
        };
        let text: String = self
            .menu_lines((action.hunk, action.line))
            .into_iter()
            .filter_map(|(hunk, line)| diff.hunks.get(hunk)?.lines.get(line))
            .map(|line| format!("{}\n", line.text.trim_end_matches('\r')))
            .collect();
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    /// The pointer went down in the list: a selection of text begins on
    /// a line's text, and goes anywhere else (`text_select.rs`). What the
    /// press does to the lines picked for staging is the row's own
    /// (`line_events`).
    fn press_text(&mut self, event: &MouseDownEvent, cx: &mut Context<Self>) {
        let (diff, rows) = (self.diff.as_deref(), self.rows.as_slice());
        if self
            .text
            .press(event, |row, column| line_of(diff, rows, row, column))
        {
            cx.notify();
        }
    }

    /// The pointer moved with its button held, wherever it is: a
    /// selection that grows follows it, and the list scrolls toward it
    /// while it is above or below the list.
    fn drag_text(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        if !self.text.is_growing() {
            return;
        }
        let list = self.list_bounds.get();
        let beyond = if position.y < list.top() {
            crate::zoom::unpx(position.y - list.top())
        } else if position.y > list.bottom() {
            crate::zoom::unpx(position.y - list.bottom())
        } else {
            0.0
        };
        if beyond != 0.0 {
            let offset = self.scroll.offset();
            let total = self.offsets.last().copied().unwrap_or(0.0);
            let room = crate::zoom::unpx(list.size.height);
            let scrolled = -crate::zoom::unpx(offset.y);
            let step = beyond.clamp(-LINE_HEIGHT * 3.0, LINE_HEIGHT * 3.0);
            let wanted = (scrolled + step).clamp(0.0, (total - room).max(0.0));
            if (wanted - scrolled).abs() >= 0.5 {
                self.scroll.set_offset(point(offset.x, px(-wanted)));
                cx.notify();
            }
        }
        let (diff, rows) = (self.diff.as_deref(), self.rows.as_slice());
        if self
            .text
            .drag(position, |row, column| line_of(diff, rows, row, column))
        {
            cx.notify();
        }
    }

    /// The pointer's button went up: the selection stops growing, and is
    /// what a middle click pastes elsewhere on the desktop.
    fn release_text(&mut self, cx: &mut Context<Self>) {
        if self.text.release() {
            text_select::offer_primary(self.selected_text(), cx);
        }
    }

    /// Whether some text is selected.
    fn has_text_selected(&self) -> bool {
        self.text.has_text()
    }

    /// The selected text, as it is copied: none when nothing is selected.
    fn selected_text(&self) -> Option<String> {
        let (diff, rows) = (self.diff.as_deref(), self.rows.as_slice());
        self.text
            .text(|row, column| line_of(diff, rows, row, column))
    }

    /// Copy the selected text (Ctrl+C, the menu's Copy); without any, the
    /// lines picked for staging, as their menu's "Copy Lines" does.
    fn copy_text(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            return;
        }
        if let Some((hunk, line)) = self.selected.first().copied() {
            self.copy_lines(&CopyLines { hunk, line }, cx);
        }
    }

    fn line_background(kind: LineKind, cx: &App) -> Option<Hsla> {
        match kind {
            LineKind::Context => None,
            LineKind::Added => Some(cx.theme().green.opacity(0.18)),
            LineKind::Removed => Some(cx.theme().red.opacity(0.18)),
        }
    }

    fn render_hunk_header(
        &self,
        hunk_ix: usize,
        hunk: &Hunk,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let actions = self.hunk_actions(hunk_ix, cx);
        let theme = cx.theme();
        // Not filled: the range and the buttons over a rule, with space
        // above every hunk but the first.
        div()
            .w_full()
            .h(px(heading_height(hunk_ix)))
            .when(hunk_ix > 0, |row| row.pt(px(HUNK_SPACE)))
            .child(
                h_flex()
                    .h(px(HUNK_HEIGHT))
                    .ml(px(HEADING_INSET))
                    .pr(px(HUNK_HEADING_END))
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(theme.border)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(div().flex_1().child(hunk.header()))
                    // The buttons are words, not code: in the interface's own face.
                    .child(
                        h_flex()
                            .flex_none()
                            .gap(px(HUNK_BUTTON_GAP))
                            .font_family(theme.font_family.clone())
                            .children(actions),
                    ),
            )
            .into_any_element()
    }

    fn render_inline(
        &self,
        ix: usize,
        hunk_ix: usize,
        line: &Line,
        cell: &Cell,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let line_ix = cell.line;
        let marker = match line.kind {
            LineKind::Context => " ",
            LineKind::Added => "+",
            LineKind::Removed => "-",
        };
        let changed = line.kind != LineKind::Context;
        let selected = self.selected.contains(&(hunk_ix, line_ix));
        let theme = cx.theme().clone();
        let cuts = self
            .cuts
            .get(ix)
            .map(|(cuts, _)| cuts.as_slice())
            .unwrap_or(&[]);
        let numbers = |value: Option<u32>| {
            Self::number(value, cx).when(selected, |cell| {
                cell.bg(theme.list_active).text_color(theme.foreground)
            })
        };
        let (old_no, new_no) = (numbers(line.old_no), numbers(line.new_no));
        let (side, lines) = self.menu_args((hunk_ix, line_ix));
        let text = self.has_text_selected();
        let at = (hunk_ix, line_ix);
        self.line_events(
            h_flex()
                .id(("diff-row", ix))
                .w_full()
                .h(px(LINE_HEIGHT * cuts.len().saturating_add(1) as f32))
                .items_start(),
            ix,
            (hunk_ix, line_ix),
            changed,
            cx,
        )
        .test_support()
        .child(old_no)
        .child(new_no)
        .child(
            div()
                .w(px(MARKER_WIDTH))
                .h(px(LINE_HEIGHT))
                .flex_none()
                .text_center()
                .child(marker),
        )
        // The tint begins where the text does; the numbers stay plain.
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_start()
                .when_some(Self::line_background(line.kind, cx), |d, bg| d.bg(bg))
                .child(self.render_text(
                    (hunk_ix, line_ix),
                    (ix, Column::Only),
                    line,
                    &cell.emphasis,
                    cuts,
                    cx,
                ))
                .when_some(self.ending_mark(line), |d, mark| {
                    d.child(Self::render_ending_mark(mark, cx))
                })
                .when(line.no_newline, |d| {
                    d.child(
                        div()
                            .flex_none()
                            .px_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(say!("diff-no-newline")),
                    )
                }),
        )
        .context_menu(move |menu, _, _| Self::line_menu(menu, side, lines, (changed, text), at))
        .into_any_element()
    }

    /// What the menu of a line needs: the side of the index the lines can
    /// be moved from (none when they cannot be acted on), and how many
    /// lines it is about.
    fn menu_args(&self, at: (usize, usize)) -> (Option<Side>, usize) {
        let side = self.side().filter(|_| self.acts_on_lines());
        let lines = if self.selected.contains(&at) {
            self.selected.len()
        } else {
            1
        };
        (side, lines)
    }

    /// What the pointer does on a line: the cursor and the selection on a
    /// press, the selection grown on a drag, and the line's menu.
    fn line_events(
        &self,
        row: Stateful<Div>,
        ix: usize,
        at: (usize, usize),
        changed: bool,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        row.when(changed && self.acts_on_lines(), |row| row.cursor_pointer())
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _, cx| {
                    let toggle = event.modifiers.secondary();
                    this.press_line(ix, at, changed, toggle, cx)
                }),
            )
            .on_mouse_move(cx.listener(move |this, event: &MouseMoveEvent, _, cx| {
                if event.pressed_button == Some(MouseButton::Left) {
                    this.drag_over_line(at, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, _| this.drag_from = None),
            )
    }

    /// A line of the file view: its number on that side and its text, with
    /// nothing that says how it changed.
    fn render_plain(
        &self,
        ix: usize,
        hunk_ix: usize,
        line: &Line,
        cell: &Cell,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let cuts = self
            .cuts
            .get(ix)
            .map(|(cuts, _)| cuts.as_slice())
            .unwrap_or(&[]);
        h_flex()
            .id(("diff-row", ix))
            .test_support()
            .w_full()
            .h(px(LINE_HEIGHT * cuts.len().saturating_add(1) as f32))
            .items_start()
            .child(Self::number(line.new_no.or(line.old_no), cx))
            .child(div().w(px(MARKER_WIDTH)).flex_none())
            .child(self.render_text(
                (hunk_ix, cell.line),
                (ix, Column::Only),
                line,
                &[],
                cuts,
                cx,
            ))
            .into_any_element()
    }

    /// One side of a split row: its number, the sign of a changed line,
    /// and its text, or the hatched place where the other side has a line.
    #[allow(clippy::too_many_arguments)]
    fn render_half(
        &self,
        ix: usize,
        hunk_ix: usize,
        hunk: &Hunk,
        cell: Option<&Cell>,
        old: bool,
        cuts: &[usize],
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme().clone();
        let Some((line, cell)) = cell.and_then(|cell| Some((hunk.lines.get(cell.line)?, cell)))
        else {
            let hatch = theme.muted_foreground.opacity(0.18);
            return div()
                .id(("diff-gap", ix * 2 + usize::from(!old)))
                .flex_1()
                .min_w_0()
                .h_full()
                .child(
                    canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| paint_hatch(bounds, hatch, window),
                    )
                    .size_full(),
                )
                .into_any_element();
        };
        let changed = line.kind != LineKind::Context;
        let selected = self.selected.contains(&(hunk_ix, cell.line));
        let marker = match line.kind {
            LineKind::Context => " ",
            LineKind::Added => "+",
            LineKind::Removed => "-",
        };
        let at = (hunk_ix, cell.line);
        let (side, lines) = self.menu_args(at);
        let text = self.has_text_selected();
        let number = Self::number(if old { line.old_no } else { line.new_no }, cx)
            .when(selected, |n| {
                n.bg(theme.list_active).text_color(theme.foreground)
            });
        self.line_events(
            h_flex()
                .id(("diff-half", ix * 2 + usize::from(!old)))
                .flex_1()
                .min_w_0()
                .h_full()
                .items_start(),
            ix,
            (hunk_ix, cell.line),
            changed,
            cx,
        )
        .child(number)
        .child(
            div()
                .w(px(MARKER_WIDTH))
                .h(px(LINE_HEIGHT))
                .flex_none()
                .text_center()
                .child(marker),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .items_start()
                .when_some(Self::line_background(line.kind, cx), |d, bg| d.bg(bg))
                .child(self.render_text(
                    (hunk_ix, cell.line),
                    (ix, if old { Column::Old } else { Column::New }),
                    line,
                    &cell.emphasis,
                    cuts,
                    cx,
                ))
                .when_some(self.ending_mark(line), |d, mark| {
                    d.child(Self::render_ending_mark(mark, cx))
                }),
        )
        .context_menu(move |menu, _, _| Self::line_menu(menu, side, lines, (changed, text), at))
        .into_any_element()
    }

    /// The mark after a changed line whose line ending is not the other
    /// side's ([`ending_mark`]).
    fn ending_mark(&self, line: &Line) -> Option<&'static str> {
        ending_mark(line, &self.diff.as_deref()?.line_endings)
    }

    fn render_ending_mark(mark: &'static str, cx: &App) -> Div {
        div()
            .flex_none()
            .px_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(mark)
    }

    fn render_row(&self, ix: usize, row: &Row, cx: &mut Context<Self>) -> AnyElement {
        let Some(diff) = self.diff.clone() else {
            return div().into_any_element();
        };
        let empty = |height: f32| div().h(px(height)).into_any_element();
        match row {
            Row::Hunk { hunk } => match diff.hunks.get(*hunk) {
                Some(found) => self.render_hunk_header(*hunk, found, cx),
                None => empty(heading_height(*hunk)),
            },
            Row::Line { hunk, cell } => {
                match diff.hunks.get(*hunk).and_then(|h| h.lines.get(cell.line)) {
                    Some(line) => self.render_inline(ix, *hunk, line, cell, cx),
                    None => empty(LINE_HEIGHT),
                }
            }
            Row::Plain { hunk, cell } => {
                match diff.hunks.get(*hunk).and_then(|h| h.lines.get(cell.line)) {
                    Some(line) => self.render_plain(ix, *hunk, line, cell, cx),
                    None => empty(LINE_HEIGHT),
                }
            }
            Row::Pair { hunk, left, right } => match diff.hunks.get(*hunk) {
                Some(found) => {
                    let none: (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
                    let (cut_left, cut_right) = self.cuts.get(ix).unwrap_or(&none);
                    let lines = cut_left.len().max(cut_right.len()).saturating_add(1);
                    let rule = cx.theme().border;
                    h_flex()
                        .w_full()
                        .h(px(LINE_HEIGHT * lines as f32))
                        .child(self.render_half(
                            ix,
                            *hunk,
                            found,
                            left.as_ref(),
                            true,
                            cut_left,
                            cx,
                        ))
                        .child(
                            div()
                                .w(px(SPLIT_GAP))
                                .h_full()
                                .flex_none()
                                .flex()
                                .justify_center()
                                .child(div().w(px(1.)).h_full().bg(rule)),
                        )
                        .child(self.render_half(
                            ix,
                            *hunk,
                            found,
                            right.as_ref(),
                            false,
                            cut_right,
                            cx,
                        ))
                        .into_any_element()
                }
                None => empty(LINE_HEIGHT),
            },
        }
    }
}

impl DiffView {
    /// What is shown, or being loaded.
    pub fn source(&self) -> Option<&DiffSource> {
        self.source.as_ref()
    }

    /// Which side of the staging area is shown, for a file of the
    /// worktree.
    pub fn side(&self) -> Option<Side> {
        match &self.source {
            Some(DiffSource::Working { side, .. }) => Some(*side),
            _ => None,
        }
    }

    /// Whether the file is shown as it is rather than how it changed.
    pub fn file_view(&self) -> bool {
        self.file_view
    }

    /// Show a source, as the file it is or as how it changed. A file is
    /// opened in the diff view; the history gives back the view it had.
    pub fn show_as(&mut self, source: Option<DiffSource>, file_view: bool, cx: &mut Context<Self>) {
        self.file_view = file_view;
        self.show(source, cx);
    }

    /// Show the file as it is, or how it changed.
    pub fn set_file_view(&mut self, file_view: bool, cx: &mut Context<Self>) {
        if self.file_view != file_view {
            let context = self.context();
            self.file_view = file_view;
            self.selected.clear();
            if self.context() != context {
                self.reload(cx);
            } else {
                self.rebuild(cx);
            }
        }
    }

    /// What the workspace needs to know of the file to lay out the rows
    /// above it: what happened to it, and whether it has lines to show.
    pub fn shape(&self) -> FileShape {
        let diff = self.diff.as_deref();
        FileShape {
            kind: diff.map(|diff| diff.kind),
            text: diff.is_some_and(|diff| !diff.binary)
                && self.image.is_none()
                && self.submodule.is_none(),
            moved_from: diff.and_then(|diff| diff.old_path.clone()),
        }
    }

    /// The controls of the diff itself, for the rows above it that the
    /// workspace lays out. The second row has previous and next, the three
    /// layouts, whitespace and wrapping (for the file as it is, wrapping
    /// alone), there but not to be used for a file without lines. The
    /// first row has how many changes and where, the switch for the words
    /// that changed, the external tool, reading the file again.
    pub fn render_controls(&mut self, cx: &mut Context<Self>) -> DiffControls {
        let theme = cx.theme().clone();
        let layout = self.layout;
        let words = self.words;
        let whitespace = self.whitespace;
        let file_view = self.file_view;
        let text = self.shape().text;
        let stats = self
            .diff
            .as_ref()
            .filter(|_| text)
            .map(|d| format!("+{} −{}", d.additions, d.deletions))
            .unwrap_or_default();
        let endings = self
            .diff
            .as_ref()
            .filter(|_| text)
            .and_then(|d| line_endings_label(&d.line_endings));
        let position = match (self.change_at, self.changes.len()) {
            (_, 0) => String::new(),
            (Some(at), total) => say!("diff-change-position", at = at.saturating_add(1), total),
            (None, total) => say!("diff-changes", total),
        };
        // An icon button of the row.
        let icon_button = |id: &'static str| {
            Button::new(id)
                .xsmall()
                .w(px(CONTROL_ICON_WIDTH))
                .h(px(CONTROL_HEIGHT))
                .disabled(!text)
        };
        // A layout's button: chosen when it is the one, and choosing it
        // asks for the diff again when it needs more of the file.
        let layout_button = |id: &'static str,
                             icon: gpui_kit::assets::IconName,
                             tip: &'static str,
                             which: Layout,
                             cx: &mut Context<Self>|
         -> Button {
            chosen(
                icon_button(id)
                    .icon(Icon::new(icon))
                    .tooltip(tip)
                    .accessibility_label(tip),
                layout == which,
                cx,
            )
            .on_click(cx.listener(move |this, _, _, cx| this.set_layout(which, cx)))
        };
        let wrap = chosen(
            icon_button("diff-wrap")
                .accessibility_label(say!("diff-wrap"))
                .icon(Icon::new(gpui_kit::assets::IconName::TextWrap))
                .tooltip(say!("diff-wrap")),
            self.wrap,
            cx,
        )
        .on_click(cx.listener(|this, _, _, cx| this.toggle_wrap(cx)));
        let groups = h_flex()
            .flex_none()
            .gap(px(CONTROL_GAP))
            .items_center()
            .when(!file_view, |row| {
                row.child(
                    ButtonGroup::new("diff-steps")
                        .outline()
                        .xsmall()
                        .child(
                            icon_button("diff-prev")
                                .accessibility_label(say!("diff-previous-change"))
                                .icon(IconName::ArrowUp)
                                .tooltip(say!("diff-previous-change"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.step_change(false, cx)),
                                ),
                        )
                        .child(
                            icon_button("diff-next")
                                .accessibility_label(say!("diff-next-change"))
                                .icon(IconName::ArrowDown)
                                .tooltip(say!("diff-next-change"))
                                .on_click(cx.listener(|this, _, _, cx| this.step_change(true, cx))),
                        ),
                )
                .child(
                    ButtonGroup::new("diff-layout")
                        .outline()
                        .xsmall()
                        .child(layout_button(
                            "mode-hunks",
                            gpui_kit::assets::IconName::FoldVertical,
                            say!("diff-layout-hunks"),
                            Layout::Hunks,
                            cx,
                        ))
                        .child(layout_button(
                            "mode-inline",
                            gpui_kit::assets::IconName::Rows3,
                            say!("diff-layout-inline"),
                            Layout::Inline,
                            cx,
                        ))
                        .child(layout_button(
                            "mode-split",
                            gpui_kit::assets::IconName::Columns2,
                            say!("diff-layout-split"),
                            Layout::Split,
                            cx,
                        )),
                )
            })
            .child(
                ButtonGroup::new("diff-detail")
                    .outline()
                    .xsmall()
                    .when(!file_view, |group| {
                        group.child(
                            chosen(
                                icon_button("diff-whitespace")
                                    .accessibility_label(say!("diff-whitespace"))
                                    .icon(Icon::new(gpui_kit::assets::IconName::Pilcrow))
                                    .tooltip(match whitespace {
                                        Whitespace::Exact => say!("diff-whitespace-exact"),
                                        Whitespace::IgnoreTrailing => {
                                            say!("diff-whitespace-ignore-trailing")
                                        }
                                        Whitespace::IgnoreAll => say!("diff-whitespace-ignore-all"),
                                        Whitespace::IgnoreCrAtEol => {
                                            say!("diff-whitespace-ignore-cr")
                                        }
                                    }),
                                whitespace != Whitespace::Exact,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _, cx| this.cycle_whitespace(cx))),
                        )
                    })
                    .child(wrap),
            )
            .into_any_element();
        let extras = h_flex()
            .flex_none()
            .gap_2()
            .items_center()
            // How the lines end, where that matters.
            .when_some(endings, |row, (label, said)| {
                row.child(
                    div()
                        .id("diff-line-endings")
                        .text_xs()
                        .font_family(theme.mono_font_family.clone())
                        .text_color(theme.muted_foreground)
                        .child(label)
                        .tooltip(move |window, cx| Tooltip::new(said.clone()).build(window, cx))
                        .test_support(),
                )
            })
            .when(!file_view, |row| {
                row.when(!stats.is_empty() || !position.is_empty(), |row| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{stats}  {position}")),
                    )
                })
                .child(
                    chosen(
                        Button::new("diff-words")
                            .accessibility_label(say!("diff-words"))
                            .ghost()
                            .xsmall()
                            .icon(IconName::CaseSensitive)
                            .tooltip(say!("diff-words"))
                            .disabled(!text),
                        words,
                        cx,
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.words = !this.words;
                        this.remember(cx);
                        this.rebuild(cx);
                    })),
                )
            })
            .child(
                Button::new("diff-external")
                    .accessibility_label(say!("diff-external"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::ExternalLink)
                    .tooltip(say!("diff-external"))
                    .on_click(cx.listener(|this, _, _, cx| this.open_external(cx))),
            )
            .child(
                Button::new("diff-refresh")
                    .accessibility_label(say!("diff-refresh"))
                    .ghost()
                    .xsmall()
                    .icon(IconName::RefreshCw)
                    .tooltip(say!("diff-refresh"))
                    .on_click(cx.listener(|this, _, _, cx| this.refresh(cx))),
            )
            .into_any_element();
        DiffControls { groups, extras }
    }
}

impl Focusable for DiffView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DiffView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.syntax_dark != cx.theme().is_dark()
            || !std::sync::Arc::ptr_eq(&self.syntax_theme, &cx.theme().highlight_theme)
        {
            self.colour(cx);
        }
        self.adopt_preferences(cx);
        // The width changed since the lines were cut: cut them again.
        if self.wrap && (self.text_width() - self.cut_width).abs() > 1.0 {
            self.lay_out(cx);
        }
        let theme = cx.theme().clone();
        let note = |text: &str, color: Hsla| {
            div()
                .p_2()
                .text_sm()
                .text_color(color)
                .child(text.to_owned())
                .into_any_element()
        };
        let mut notes: Vec<AnyElement> = Vec::new();
        if let Some(err) = &self.error {
            notes.push(note(err, theme.danger));
        }
        match &self.diff {
            None if self.loading => notes.push(note(say!("diff-loading"), theme.muted_foreground)),
            None => {}
            Some(diff) if diff.too_large => notes.push(
                v_flex()
                    .p_3()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .text_sm()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(guard_text(diff)),
                    )
                    .child(
                        Button::new("diff-load-anyway")
                            .outline()
                            .small()
                            .label(say!("diff-load-anyway"))
                            .disabled(self.loading)
                            .on_click(cx.listener(|this, _, _, cx| this.load_large(cx))),
                    )
                    .into_any_element(),
            ),
            Some(diff) if diff.binary && self.image.is_none() => {
                notes.push(note(say!("diff-binary"), theme.muted_foreground));
                notes.push(self.render_binary(diff, cx));
            }
            Some(diff) if diff.binary => {}
            // A submodule says what it has to in its own pane.
            Some(_) if self.submodule.is_some() => {}
            Some(diff) if self.hides_endings_lines(diff) => notes.push(
                v_flex()
                    .p_3()
                    .gap_2()
                    .items_start()
                    .child(
                        div()
                            .text_sm()
                            .whitespace_normal()
                            .text_color(theme.muted_foreground)
                            .child(endings_only_text(&diff.line_endings)),
                    )
                    // Whitespace that does not count can leave no lines.
                    .when(!diff.hunks.is_empty(), |notice| {
                        notice.child(
                            Button::new("diff-endings-lines")
                                .outline()
                                .small()
                                .label(say!("diff-endings-show-lines"))
                                .on_click(
                                    cx.listener(|this, _, _, cx| this.show_endings_lines(cx)),
                                ),
                        )
                    })
                    .into_any_element(),
            ),
            Some(diff) if diff.hunks.is_empty() => {
                notes.push(note(&unchanged_text(diff), theme.muted_foreground))
            }
            Some(_) => {}
        }

        let rows = self.rows.clone();
        let painted = self.list_bounds.clone();
        let drawn = self.text.drawn();
        let view = cx.entity().downgrade();
        let list =
            v_flex()
                .id("diff-list")
                .relative()
                .size_full()
                .font_family(theme.mono_font_family.clone())
                .text_size(px(TEXT_SIZE))
                .line_height(px(LINE_HEIGHT))
                // Where the list is, so that wrapped lines are cut for the
                // width it has; a new width asks for another frame.
                .child(
                    canvas(
                        // Laid out before the rows: what they drew in the
                        // last frame is forgotten as they draw again.
                        move |_, _, _| text_select::forget_drawn(&drawn),
                        move |bounds, _, window, _| {
                            if painted.get() != bounds {
                                let wider = painted.get().size.width != bounds.size.width;
                                painted.set(bounds);
                                if wider {
                                    window.refresh();
                                }
                            }
                            text_select::follow_pointer(
                                view.clone(),
                                |this: &mut Self, position, cx| this.drag_text(position, cx),
                                |this: &mut Self, cx| this.release_text(cx),
                                window,
                            );
                        },
                    )
                    .absolute()
                    .size_full(),
                )
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, event: &MouseDownEvent, _, cx| this.press_text(event, cx)),
                )
                .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                    this.scroll_sideways(event, cx)
                }))
                .child(
                    v_virtual_list(
                        cx.entity().clone(),
                        "diff-rows",
                        self.sizes.get(),
                        move |this, range, _window, cx| {
                            range
                                .filter_map(|ix| Some(this.render_row(ix, rows.get(ix)?, cx)))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.scroll),
                )
                .test_support();

        v_flex()
            .id("diff")
            .key_context(DIFF_CONTEXT)
            .track_focus(&self.focus_handle)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| this.focus_handle.focus(window, cx)),
            )
            // A line's menu acts on the viewer: it has the keys first.
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, _, window, cx| this.focus_handle.focus(window, cx)),
            )
            .on_action(cx.listener(|this, _: &NextChange, _, cx| this.step_change(true, cx)))
            .on_action(cx.listener(|this, _: &PrevChange, _, cx| this.step_change(false, cx)))
            .on_action(cx.listener(|this, _: &ToggleWrap, _, cx| this.toggle_wrap(cx)))
            .on_action(
                cx.listener(|this, action: &ActOnLines, _, cx| this.act_on_lines(action, cx)),
            )
            .on_action(cx.listener(|this, action: &CopyLines, _, cx| this.copy_lines(action, cx)))
            .on_action(cx.listener(|this, _: &CopyText, _, cx| this.copy_text(cx)))
            .size_full()
            .bg(theme.background)
            .children(notes)
            .child(match (&self.image, &self.submodule) {
                // A picture is shown as one, not as text.
                (Some(image), _) => div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_images(image, cx))
                    .into_any_element(),
                (None, Some(submodule)) => div()
                    .flex_1()
                    .min_h_0()
                    .child(self.render_submodule(submodule, cx))
                    .into_any_element(),
                (None, None) => h_flex()
                    .flex_1()
                    .min_h_0()
                    .child(div().flex_1().min_w_0().h_full().child(list))
                    .when(!self.rows.is_empty(), |body| {
                        body.child(self.render_minimap(cx))
                    })
                    .into_any_element(),
            })
    }
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use bitbraid_git::{ChangeKind, FileDiff, StatusEntry, Whitespace, diff::FileChangeKind};
    use serde_json::json;

    use super::{
        MapGeometry, MiniGeometry, SubmoduleState, binary_side, change_end, dragged_lines,
        guard_text, hunk_buttons, layout_of, mini_runs, staged_moves, unchanged_text,
        whitespace_of, words_for,
    };
    use crate::{
        prefs::DiffPrefs,
        views::{
            changes::Side,
            diff_doc::{Layout, Mark},
        },
    };

    #[test]
    fn a_drag_selects_the_changed_lines_between_its_ends() {
        let hunk = bitbraid_git::diff_bytes(b"a\nb\nc\nd\ne\n", b"a\nB\nc\nD\ne\n", 3)
            .unwrap()
            .remove(0);
        // a, -b, +B, c, -d, +D, e
        assert_eq!(dragged_lines(&hunk, 1, 5), [1, 2, 4, 5]);
        assert_eq!(dragged_lines(&hunk, 5, 1), [1, 2, 4, 5], "either way");
        assert_eq!(
            dragged_lines(&hunk, 0, 0),
            Vec::<usize>::new(),
            "context alone"
        );
        assert_eq!(dragged_lines(&hunk, 3, 99), [4, 5], "past the end");
    }

    #[test]
    fn going_down_a_change_comes_in_whole() {
        // Two runs: rows 3 to 5 (removed 3, added 4-5), and row 9.
        let marks = [
            (3, 1, Mark::Removed),
            (4, 2, Mark::Added),
            (9, 1, Mark::Added),
        ];
        assert_eq!(change_end(&marks, 3, Some(9)), 5);
        assert_eq!(change_end(&marks, 9, None), 9);
        assert_eq!(change_end(&[], 7, None), 7);
    }

    #[test]
    fn the_miniature_draws_the_characters_that_are_not_blank() {
        assert_eq!(mini_runs("  let a = 1;"), [(2, 3), (6, 1), (8, 1), (10, 2)]);
        assert_eq!(mini_runs("\tx"), [(4, 1)]);
        assert!(mini_runs("").is_empty() && mini_runs("    ").is_empty());
        // Nothing past what it has room for.
        let wide = "x".repeat(500);
        assert_eq!(mini_runs(&wide), [(0, 108)]);
    }

    #[test]
    fn the_miniature_moves_with_the_list_when_it_is_longer_than_the_strip() {
        // 100 rows of 20 drawn two high: 200 of a strip of 400, it stays.
        let short = MiniGeometry::new(100, 2000.0, 400.0);
        assert_eq!(short.offset(800.0), 0.0);
        assert_eq!(short.row_at(51.0, 800.0), 25);
        // 1000 rows: 2000 drawn on 400, moved by as much of the 1600 over
        // as the list is of the way down.
        let long = MiniGeometry::new(1000, 20_000.0, 400.0);
        assert_eq!(long.offset(0.0), 0.0);
        assert_eq!(long.offset(19_600.0), 1600.0);
        assert_eq!(long.offset(9_800.0), 800.0);
        assert_eq!(long.row_at(0.0, 9_800.0), 400);
        // Never a row past the last.
        assert_eq!(long.row_at(10_000.0, 19_600.0), 999);
    }

    #[test]
    fn the_map_draws_a_short_file_at_its_height_and_shrinks_a_long_one() {
        // Shorter than the map: as high as it is, and all of it in view.
        let short = MapGeometry::new(200.0, 800.0);
        assert_eq!(short.mark(40.0, 60.0, 4.0), (40.0, 20.0));
        assert_eq!(short.mark(40.0, 41.0, 4.0), (40.0, 4.0));
        assert_eq!(short.view(0.0), (0.0, 800.0));
        // Four times the map: a quarter as high, and a quarter in view.
        let long = MapGeometry::new(3200.0, 800.0);
        assert_eq!(long.mark(400.0, 800.0, 4.0), (100.0, 100.0));
        assert_eq!(long.view(0.0), (0.0, 200.0));
        assert_eq!(long.view(1600.0), (400.0, 200.0));
        // Never past the end, and never lower than asked.
        assert_eq!(long.view(5000.0), (600.0, 200.0));
        assert_eq!(long.mark(3199.0, 3200.0, 4.0), (796.0, 4.0));
        // Nothing to draw on, and nothing drawn.
        let none = MapGeometry::new(0.0, 0.0);
        assert_eq!(none.mark(0.0, 10.0, 4.0), (0.0, 0.0));
    }

    #[test]
    fn a_hunks_buttons_take_back_first_and_say_it_in_two_words() {
        let said = |side, lines| -> Vec<(&str, &str, bool)> {
            hunk_buttons(side, lines)
                .into_iter()
                .map(|button| (button.command, button.label, button.takes_back))
                .collect()
        };
        assert_eq!(
            said(Side::Unstaged, false),
            [
                ("discard.hunks", "Discard Hunk", true),
                ("stage.hunks", "Stage Hunk", false),
            ]
        );
        assert_eq!(
            said(Side::Unstaged, true),
            [
                ("discard.hunks", "Discard Lines", true),
                ("stage.hunks", "Stage Lines", false),
            ]
        );
        assert_eq!(
            said(Side::Staged, false),
            [("unstage.hunks", "Unstage Hunk", true)]
        );
        assert_eq!(
            said(Side::Staged, true),
            [("unstage.hunks", "Unstage Lines", true)]
        );
        // Each label is two generic words.
        for side in [Side::Unstaged, Side::Staged] {
            for lines in [false, true] {
                for button in hunk_buttons(side, lines) {
                    assert_eq!(button.label.split(' ').count(), 2, "{}", button.label);
                }
            }
        }
    }

    #[test]
    fn a_file_moved_in_the_index_is_known_by_its_old_name() {
        let entry = |path: &str, from: Option<&str>, staged, unstaged| StatusEntry {
            path: path.to_owned(),
            original_path: from.map(str::to_owned),
            staged,
            unstaged,
            conflict: false,
        };
        let moves = staged_moves(&[
            entry(
                "web/add.js",
                Some("web/sum.js"),
                Some(ChangeKind::Renamed),
                None,
            ),
            // Moved and then changed again: the staged side is the move.
            entry(
                "src/new.rs",
                Some("src/old.rs"),
                Some(ChangeKind::Renamed),
                Some(ChangeKind::Modified),
            ),
            entry("src/added.rs", None, Some(ChangeKind::Added), None),
            entry("README.md", None, Some(ChangeKind::Modified), None),
            // A copy has a source too, and is not a move.
            entry("b.txt", Some("a.txt"), Some(ChangeKind::Copied), None),
            // A move without where from says nothing to compare with.
            entry("c.txt", None, Some(ChangeKind::Renamed), None),
        ]);
        assert_eq!(
            moves.into_iter().collect::<Vec<_>>(),
            [
                ("src/new.rs".to_owned(), "src/old.rs".to_owned()),
                ("web/add.js".to_owned(), "web/sum.js".to_owned()),
            ]
        );
    }

    #[test]
    fn a_file_that_only_moved_says_where_from() {
        let mut moved = file(None, None);
        moved.too_large = false;
        assert_eq!(unchanged_text(&moved), "No textual changes.");
        moved.old_path = Some("web/sum.js".to_owned());
        let said = unchanged_text(&moved);
        assert!(said.contains("web/sum.js"), "{said}");
    }

    #[test]
    fn how_a_diff_is_shown_goes_to_the_preferences_and_comes_back() {
        for layout in [Layout::Hunks, Layout::Inline, Layout::Split] {
            for whitespace in [
                Whitespace::Exact,
                Whitespace::IgnoreTrailing,
                Whitespace::IgnoreAll,
                Whitespace::IgnoreCrAtEol,
            ] {
                let (layout_word, whitespace_word) = words_for(layout, whitespace);
                let prefs = DiffPrefs {
                    layout: layout_word.to_owned(),
                    whitespace: whitespace_word.to_owned(),
                    ..Default::default()
                };
                assert_eq!(layout_of(&prefs), layout);
                assert_eq!(whitespace_of(&prefs), whitespace);
                // The words are the ones the preferences know.
                assert!(crate::prefs::DIFF_LAYOUTS.contains(&layout_word));
                assert!(crate::prefs::DIFF_WHITESPACE.contains(&whitespace_word));
            }
        }
        // A word nobody knows is the default, not a failure.
        let odd = DiffPrefs {
            layout: "diagonal".to_owned(),
            whitespace: "sometimes".to_owned(),
            ..Default::default()
        };
        assert_eq!(layout_of(&odd), Layout::Hunks);
        assert_eq!(whitespace_of(&odd), Whitespace::Exact);
    }

    fn file(old_size: Option<u64>, new_size: Option<u64>) -> FileDiff {
        FileDiff {
            kind: FileChangeKind::Modified,
            path: "data/big.csv".to_owned(),
            old_path: None,
            old_id: None,
            new_id: None,
            old_mode: None,
            new_mode: None,
            binary: false,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            old_size,
            new_size,
            too_large: true,
            line_endings: Default::default(),
        }
    }

    #[test]
    fn line_endings_are_named_where_they_matter() {
        use bitbraid_git::{FileLineEndings, LineEndings::*};
        let endings = |old, new, worktree| FileLineEndings {
            old,
            new,
            worktree,
            only_change: false,
        };
        let label = |e: FileLineEndings| super::line_endings_label(&e).map(|(label, _)| label);
        // LF everywhere, or no line ends at all: nothing to say.
        assert_eq!(label(endings(Some(Lf), Some(Lf), None)), None);
        assert_eq!(label(endings(None, Some(Lf), None)), None);
        assert_eq!(label(endings(None, None, None)), None);
        assert_eq!(
            label(endings(Some(Crlf), Some(Crlf), None)).unwrap(),
            "CRLF"
        );
        assert_eq!(
            label(endings(None, Some(Mixed), None)).unwrap(),
            "LF + CRLF"
        );
        assert_eq!(
            label(endings(Some(Lf), Some(Crlf), None)).unwrap(),
            "LF → CRLF"
        );
        assert_eq!(
            label(endings(Some(Crlf), Some(Mixed), None)).unwrap(),
            "CRLF → LF + CRLF"
        );
        // CRLF on disk that git stores as LF: what the file has, and the
        // tooltip says what is compared.
        let converted = endings(Some(Lf), Some(Lf), Some(Crlf));
        let (label, said) = super::line_endings_label(&converted).unwrap();
        assert_eq!(label, "CRLF");
        assert!(said.contains("stores as LF"), "{said}");
        assert_eq!(
            super::endings_only_text(&endings(Some(Lf), Some(Crlf), None)),
            "Only the line endings changed, from LF to CRLF."
        );
        assert_eq!(
            super::endings_only_text(&endings(None, Some(Lf), None)),
            "Only the line endings changed."
        );
    }

    #[test]
    fn a_line_ending_is_marked_where_it_is_not_the_other_sides() {
        use bitbraid_git::{FileLineEndings, Line, LineEndings::*, LineKind};
        let line = |kind, text: &str, no_newline| Line {
            kind,
            text: text.to_owned(),
            old_no: Some(1),
            new_no: Some(1),
            no_newline,
        };
        let endings = |old, new| FileLineEndings {
            old,
            new,
            worktree: None,
            only_change: false,
        };
        let mark = |line: &Line, e| super::ending_mark(line, &e);
        let added_cr = line(LineKind::Added, "x\r", false);
        let added_lf = line(LineKind::Added, "x", false);
        let removed_cr = line(LineKind::Removed, "x\r", false);
        // CRLF where the other side's lines all end in LF.
        assert_eq!(mark(&added_cr, endings(Some(Lf), Some(Mixed))), Some("CR"));
        assert_eq!(
            mark(&removed_cr, endings(Some(Mixed), Some(Lf))),
            Some("CR")
        );
        // LF alone where the other side's lines all end in CRLF.
        assert_eq!(
            mark(&added_lf, endings(Some(Crlf), Some(Mixed))),
            Some("LF")
        );
        // The same as the other side, or a side with both: nothing.
        assert_eq!(mark(&added_cr, endings(Some(Crlf), Some(Crlf))), None);
        assert_eq!(mark(&added_lf, endings(Some(Lf), Some(Lf))), None);
        assert_eq!(mark(&added_cr, endings(Some(Mixed), Some(Mixed))), None);
        assert_eq!(mark(&removed_cr, endings(Some(Crlf), Some(Mixed))), None);
        // No other side, an unchanged line, a last line without an end.
        assert_eq!(mark(&added_cr, endings(None, Some(Crlf))), None);
        assert_eq!(
            mark(
                &line(LineKind::Context, "x\r", false),
                endings(Some(Lf), Some(Crlf))
            ),
            None
        );
        assert_eq!(
            mark(
                &line(LineKind::Added, "x", true),
                endings(Some(Crlf), Some(Crlf))
            ),
            None
        );
    }

    #[test]
    fn a_file_that_was_not_compared_says_how_large_it_is() {
        let both = guard_text(&file(Some(3 * 1024 * 1024), Some(5 * 1024 * 1024)));
        assert!(both.contains("3.0 MiB before, 5.0 MiB after"), "{both}");
        assert!(both.contains("files over 2.0 MiB"), "{both}");
        assert!(guard_text(&file(None, Some(4096))).contains("4.0 KiB, a new file"));
        assert!(guard_text(&file(Some(4096), None)).contains("4.0 KiB, deleted"));
        assert!(guard_text(&file(None, None)).contains("not known"));
        // A size no machine has does not break the sentence.
        assert!(guard_text(&file(Some(u64::MAX), Some(0))).contains("before"));
    }

    #[test]
    fn a_side_of_a_binary_file_is_its_size_and_its_whole_id() {
        let id: bitbraid_git::Oid = "ab".repeat(20).parse().unwrap();
        assert_eq!(
            binary_side(Some(1536), Some(id)),
            Some(("1.5 KiB".to_owned(), "ab".repeat(20)))
        );
        assert_eq!(
            binary_side(None, Some(id)).map(|(size, _)| size),
            Some("size not known".to_owned())
        );
        // A side that is not there has nothing to say.
        assert_eq!(binary_side(Some(10), None), None);
    }

    #[test]
    fn a_submodules_answer_is_read_into_the_rows_of_its_block() {
        let id = |byte: &str| byte.repeat(20);
        let state = SubmoduleState::from_reply(&json!({
            "path": "deps/lib",
            "old": id("aa"),
            "new": id("bb"),
            "old_summary": "lib: three",
            "new_summary": "lib: five",
            "known": true,
            "gained": [{ "id": id("cc"), "summary": "newer" }, { "id": 3 }],
            "lost": [],
            "more_gained": true,
            "more_lost": false,
        }));
        assert!(state.answered && state.known && state.more_gained && !state.more_lost);
        assert_eq!(
            state.rows(),
            [
                ("Commit", id("aa"), true),
                ("Commit Message", "lib: three".to_owned(), true),
                ("Commit", id("bb"), false),
                ("Commit Message", "lib: five".to_owned(), false),
            ]
        );
        // An entry that is not as expected is left out, not shown wrong.
        assert_eq!(state.gained, [(id("cc"), "newer".to_owned())]);
        assert!(state.lost.is_empty());

        // Added: the new side alone; its clone not there: no message.
        let added = SubmoduleState::from_reply(&json!({ "old": null, "new": id("bb") }));
        assert_eq!(added.rows(), [("Commit", id("bb"), false)]);
        assert!(!added.known);
        let same = SubmoduleState::from_reply(&json!({ "old": id("aa"), "new": id("aa") }));
        assert_eq!(same.rows(), [("Commit", id("aa"), false)]);
        // Garbage is an answer with nothing in it.
        let nothing = SubmoduleState::from_reply(&json!("nonsense"));
        assert!(nothing.answered && nothing.gained.is_empty() && nothing.rows().is_empty());
    }
}
