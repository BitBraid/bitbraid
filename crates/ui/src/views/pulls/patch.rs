//! The changes a pull request makes to one file, from the unified diff the
//! provider sent with it (`model::PullFile::patch`), drawn the way the
//! window's diff view draws a file: the numbers of the old and the new
//! line, a mark, and the line tinted green where it was added and red
//! where it was removed, each hunk under its range. Also the lines of a
//! conversation's place in a file (`model::Thread::diff_hunk`).
//!
//! What a provider sends is held to [`MOST_LINES`] lines, and numbers in
//! a hunk's range that are no numbers are taken as unknown.
//!
//! Sources: the unified diff format as GNU diffutils and git document it
//! (`diff -u`, git-diff(1) "combined diff format" left out);
//! docs.github.com/en/rest/pulls (the `patch` of a file).

use std::rc::Rc;

use gpui_kit::component::{ActiveTheme, h_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

use crate::zoom::px;

/// The most lines of a patch that are drawn.
pub const MOST_LINES: usize = 20_000;
/// How high a line is.
pub const LINE_HEIGHT: f32 = 20.0;
/// How wide a column of line numbers is.
const NUMBER_WIDTH: f32 = 44.0;
/// How wide the column of marks is.
const MARK_WIDTH: f32 = 16.0;
/// The most characters of a line that are drawn.
const MOST_CHARACTERS: usize = 1000;

/// What a line of a patch is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The range of a hunk: `@@ -1,4 +1,5 @@`.
    Hunk,
    /// A line both sides have.
    Context,
    /// A line only the new side has.
    Added,
    /// A line only the old side has.
    Removed,
    /// What the diff says about itself (`\ No newline at end of file`),
    /// or that the rest was left out.
    Note,
}

/// One line of a patch, with its numbers on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchLine {
    pub kind: Kind,
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub text: String,
}

/// Where a hunk's range says it begins: `-12,3` is 12, on each side.
fn range_start(header: &str) -> (Option<u64>, Option<u64>) {
    let mut old = None;
    let mut new = None;
    for part in header.split_whitespace().skip(1).take(2) {
        let number = |text: &str| text.split(',').next()?.parse::<u64>().ok();
        if let Some(rest) = part.strip_prefix('-') {
            old = number(rest);
        } else if let Some(rest) = part.strip_prefix('+') {
            new = number(rest);
        }
    }
    (old, new)
}

/// The lines of a unified diff, numbered. Lines before the first hunk
/// (file headers) are left out.
pub fn parse(patch: &str) -> Vec<PatchLine> {
    let mut lines: Vec<PatchLine> = Vec::new();
    let (mut old, mut new): (Option<u64>, Option<u64>) = (None, None);
    let mut in_hunk = false;
    for line in patch.lines() {
        if lines.len() >= MOST_LINES {
            lines.push(PatchLine {
                kind: Kind::Note,
                old: None,
                new: None,
                text: say!("pulls-patch-cut", most = MOST_LINES),
            });
            break;
        }
        let text: String = line.chars().take(MOST_CHARACTERS).collect();
        if line.starts_with("@@") {
            (old, new) = range_start(line);
            in_hunk = true;
            lines.push(PatchLine {
                kind: Kind::Hunk,
                old: None,
                new: None,
                text,
            });
            continue;
        }
        if !in_hunk {
            continue;
        }
        let next = |at: &mut Option<u64>| {
            let here = *at;
            *at = at.and_then(|n| n.checked_add(1));
            here
        };
        let (kind, old_no, new_no, body) = match line.chars().next() {
            Some('+') => (Kind::Added, None, next(&mut new), text.get(1..)),
            Some('-') => (Kind::Removed, next(&mut old), None, text.get(1..)),
            Some('\\') => (Kind::Note, None, None, Some(text.as_str())),
            Some(' ') => (Kind::Context, next(&mut old), next(&mut new), text.get(1..)),
            // An empty line in a hunk is a context line whose space was
            // lost on the way.
            None => (Kind::Context, next(&mut old), next(&mut new), Some("")),
            Some(_) => (Kind::Note, None, None, Some(text.as_str())),
        };
        lines.push(PatchLine {
            kind,
            old: old_no,
            new: new_no,
            text: body.unwrap_or_default().to_owned(),
        });
    }
    lines
}

/// The tint of a line, as the diff view has it.
fn tint(kind: Kind, cx: &App) -> Option<Hsla> {
    match kind {
        Kind::Added => Some(cx.theme().green.opacity(0.18)),
        Kind::Removed => Some(cx.theme().red.opacity(0.18)),
        _ => None,
    }
}

/// One line, drawn.
pub fn render_line(line: &PatchLine, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let number = |value: Option<u64>| {
        div()
            .w(px(NUMBER_WIDTH))
            .flex_none()
            .pr_1()
            .text_right()
            .text_color(theme.muted_foreground)
            .child(value.map(|n| n.to_string()).unwrap_or_default())
    };
    let row = h_flex()
        .w_full()
        .h(px(LINE_HEIGHT))
        .flex_none()
        .items_center()
        .font_family(theme.mono_font_family.clone())
        .text_size(theme.mono_font_size)
        .whitespace_nowrap();
    match line.kind {
        Kind::Hunk => row
            .text_color(theme.muted_foreground)
            .border_b_1()
            .border_color(theme.border)
            .pl(px(NUMBER_WIDTH * 2.0 + MARK_WIDTH))
            .child(div().min_w_0().truncate().child(line.text.clone()))
            .into_any_element(),
        Kind::Note => row
            .text_color(theme.muted_foreground)
            .pl(px(NUMBER_WIDTH * 2.0 + MARK_WIDTH))
            .child(div().min_w_0().truncate().child(line.text.clone()))
            .into_any_element(),
        kind => row
            .child(number(line.old))
            .child(number(line.new))
            .child(
                div()
                    .w(px(MARK_WIDTH))
                    .flex_none()
                    .text_center()
                    .child(match kind {
                        Kind::Added => "+",
                        Kind::Removed => "-",
                        _ => " ",
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .flex()
                    .items_center()
                    .when_some(tint(kind, cx), |text, tint| text.bg(tint))
                    .child(div().min_w_0().truncate().child(if line.text.is_empty() {
                        " ".to_owned()
                    } else {
                        line.text.clone()
                    })),
            )
            .into_any_element(),
    }
}

/// A patch as a list that draws only the lines in view.
pub fn render_list(id: impl Into<ElementId>, lines: Rc<Vec<PatchLine>>) -> UniformList {
    let count = lines.len();
    uniform_list(id, count, move |range, _, cx| {
        range
            .filter_map(|at| lines.get(at).map(|line| render_line(line, cx)))
            .collect::<Vec<_>>()
    })
}

#[cfg(test)]
mod tests {
    // No glob import: `gpui_kit::*` carries its own `test` attribute.
    use super::{Kind, MOST_CHARACTERS, MOST_LINES, parse};

    #[test]
    fn a_patch_is_numbered_on_each_side() {
        let lines =
            parse("@@ -1,3 +1,4 @@ fn main\n a\n-b\n+c\n+d\n e\n\\ No newline at end of file");
        let seen: Vec<(Kind, Option<u64>, Option<u64>, &str)> = lines
            .iter()
            .map(|line| (line.kind, line.old, line.new, line.text.as_str()))
            .collect();
        assert_eq!(
            seen,
            vec![
                (Kind::Hunk, None, None, "@@ -1,3 +1,4 @@ fn main"),
                (Kind::Context, Some(1), Some(1), "a"),
                (Kind::Removed, Some(2), None, "b"),
                (Kind::Added, None, Some(2), "c"),
                (Kind::Added, None, Some(3), "d"),
                (Kind::Context, Some(3), Some(4), "e"),
                (Kind::Note, None, None, "\\ No newline at end of file"),
            ]
        );
    }

    #[test]
    fn a_hostile_patch_is_read_without_harm() {
        // Headers before the first hunk are left out; a range that is no
        // number leaves the numbers unknown; one at the end does not wrap.
        let lines = parse(&format!(
            "diff --git a/x b/x\n--- a/x\n+++ b/x\n@@ -x +{} @@\n a\n+b\n",
            u64::MAX
        ));
        assert_eq!(lines.len(), 3);
        assert_eq!(lines[1].old, None);
        assert_eq!(lines[1].new, Some(u64::MAX));
        assert_eq!(lines[2].new, None);
        // Very many lines are held to the most, with a note.
        let many = format!("@@ -1 +1 @@\n{}", "+x\n".repeat(MOST_LINES + 10));
        let lines = parse(&many);
        assert_eq!(lines.len(), MOST_LINES + 1);
        assert_eq!(lines.last().map(|line| line.kind), Some(Kind::Note));
        // A long line is cut, at a character's edge.
        let wide = format!("@@ -1 +1 @@\n+{}", "é".repeat(5000));
        assert_eq!(parse(&wide)[1].text.chars().count(), MOST_CHARACTERS - 1);
        assert!(parse("").is_empty());
    }
}
