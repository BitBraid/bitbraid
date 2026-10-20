//! The Markdown a provider sends (a pull request's description, a
//! comment), drawn as text with its structure by the toolkit's text view,
//! after it has been made safe to draw here:
//!
//! - it is cut at [`MOST_SHOWN`] bytes, with a line that says so;
//! - what would nest without end is held to a few levels (quotes in
//!   quotes, emphasis in emphasis, deep indentation), so that drawing it
//!   cannot run out of stack;
//! - a picture becomes a link that says it is one: nothing a provider
//!   sends is fetched to be drawn (the toolkit is given no client to fetch
//!   pictures with either);
//! - a link opens only when it is clicked, in the system's browser, and
//!   only when it is `https` (or `http` to this machine): see
//!   [`super::facts::may_open`].
//!
//! Sources: CommonMark 0.31 and GitHub Flavored Markdown (code spans,
//! fenced code blocks, images, emphasis); gpui-kit documentation (text
//! view, link clicks).

use gpui_kit::component::text::TextView;
use gpui_kit::{ElementId, SharedString};

use super::facts::may_open;

/// The most of a text that is drawn, in bytes.
pub const MOST_SHOWN: usize = 64 * 1024;
/// How many of a nesting mark stand in a row at most.
const MOST_NESTED: usize = 8;
/// How far a line is indented at most, in columns.
const MOST_INDENT: usize = 24;

/// A text of a provider's, made ready to be drawn as Markdown.
pub fn prepared(text: &str) -> SharedString {
    let cut = text.len() > MOST_SHOWN;
    let text = bitbraid_integrations::model::bounded(text, MOST_SHOWN);
    let mut out = String::with_capacity(text.len().saturating_add(64));
    // The fence a fenced code block was opened with: its character and how
    // many of it.
    let mut fence: Option<(char, usize)> = None;
    for (at, line) in text.split('\n').enumerate() {
        if at > 0 {
            out.push('\n');
        }
        let trimmed = line.trim_start();
        let opener = fence_of(trimmed);
        match (fence, opener) {
            (Some((c, n)), Some((d, m))) if c == d && m >= n && trimmed.trim_end().len() == m => {
                fence = None;
                out.push_str(line);
                continue;
            }
            (Some(_), _) => {
                out.push_str(line);
                continue;
            }
            (None, Some(opened)) => {
                fence = Some(opened);
                out.push_str(line);
                continue;
            }
            (None, None) => {}
        }
        prepare_line(line, &mut out);
    }
    if cut {
        out.push_str(&format!("\n\n*({})*", say!("pulls-markdown-cut")));
    }
    SharedString::from(out)
}

/// The fence a line opens or closes a code block with: three or more
/// backquotes or tildes.
fn fence_of(trimmed: &str) -> Option<(char, usize)> {
    let first = trimmed.chars().next()?;
    if first != '`' && first != '~' {
        return None;
    }
    let count = trimmed.chars().take_while(|c| *c == first).count();
    (count >= 3).then_some((first, count))
}

/// One line outside a code block: its indentation and its quotes held to
/// a few levels, runs of emphasis and brackets too, and its pictures made
/// links, all but inside code spans.
fn prepare_line(line: &str, out: &mut String) {
    let mut chars = line.chars().peekable();
    // The indentation, at most so many columns.
    let mut columns = 0usize;
    while let Some(c) = chars.peek().copied() {
        let width = match c {
            ' ' => 1,
            '\t' => 4,
            _ => break,
        };
        chars.next();
        if columns.saturating_add(width) <= MOST_INDENT {
            columns = columns.saturating_add(width);
            out.push(c);
        }
    }
    // The quotes it begins with, at most so many.
    let mut quotes = 0usize;
    while let Some(c) = chars.peek().copied() {
        match c {
            '>' => {
                chars.next();
                quotes = quotes.saturating_add(1);
                if quotes <= MOST_NESTED {
                    out.push('>');
                }
            }
            ' ' | '\t' if quotes > 0 => {
                chars.next();
                if quotes <= MOST_NESTED {
                    out.push(c);
                }
            }
            _ => break,
        }
    }
    // The rest: a code span is kept as it is; outside one, a picture
    // becomes a link and long runs are cut.
    let mut code: Option<usize> = None;
    let mut run: (Option<char>, usize) = (None, 0);
    while let Some(c) = chars.next() {
        if c == '`' {
            let mut ticks = 1usize;
            while chars.peek() == Some(&'`') {
                chars.next();
                ticks = ticks.saturating_add(1);
            }
            code = match code {
                Some(open) if open == ticks => None,
                Some(open) => Some(open),
                None => Some(ticks),
            };
            out.extend(std::iter::repeat_n('`', ticks));
            run = (None, 0);
            continue;
        }
        if code.is_some() {
            out.push(c);
            continue;
        }
        if c == '!' && chars.peek() == Some(&'[') {
            chars.next();
            out.push_str(&format!("[{}: ", say!("pulls-markdown-image")));
            run = (None, 0);
            continue;
        }
        // Emphasis marks of any kind count as one run; a bracket as its
        // own.
        let kind = match c {
            '*' | '_' | '~' => Some('*'),
            '[' => Some('['),
            _ => None,
        };
        match kind {
            Some(kind) if run.0 == Some(kind) => run.1 = run.1.saturating_add(1),
            Some(kind) => run = (Some(kind), 1),
            None => run = (None, 0),
        }
        if run.1 <= MOST_NESTED {
            out.push(c);
        }
    }
}

/// A text view of prepared Markdown (see [`prepared`]) whose links open
/// in the system's browser when they are clicked, and only when they may.
pub fn view(id: impl Into<ElementId>, text: SharedString) -> TextView {
    TextView::markdown(id, text)
        .selectable(true)
        .on_link_click(|url, _, _, cx| {
            if may_open(url) {
                cx.open_url(url);
            }
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_become_links_outside_code() {
        assert_eq!(
            prepared("See ![the graph](https://x/a.png) and `![no](x)`").as_ref(),
            "See [image: the graph](https://x/a.png) and `![no](x)`"
        );
        let fenced = "```\n![kept](x)\n```\n![made](y)";
        assert_eq!(
            prepared(fenced).as_ref(),
            "```\n![kept](x)\n```\n[image: made](y)"
        );
        // A fence of four is not closed by three.
        let long = "````\n```\n![kept](x)\n````\n![made](y)";
        assert_eq!(
            prepared(long).as_ref(),
            "````\n```\n![kept](x)\n````\n[image: made](y)"
        );
    }

    #[test]
    fn what_would_nest_without_end_is_held_to_a_few_levels() {
        let quotes = format!("{}deep", ">".repeat(10_000));
        assert_eq!(prepared(&quotes).as_ref(), format!("{}deep", ">".repeat(8)));
        let quoted = "> > > > > > > > > > x";
        assert_eq!(prepared(quoted).as_ref(), "> > > > > > > > x");
        let stars = format!("{}a{}", "*_".repeat(5_000), "_*".repeat(5_000));
        let shown = prepared(&stars);
        assert_eq!(
            shown.as_ref(),
            format!("{}a{}", "*_".repeat(4), "_*".repeat(4))
        );
        let brackets = format!("{}x", "[".repeat(1000));
        assert_eq!(prepared(&brackets).as_ref(), format!("{}x", "[".repeat(8)));
        let indented = format!("{}- item", " ".repeat(500));
        assert_eq!(
            prepared(&indented).as_ref(),
            format!("{}- item", " ".repeat(24))
        );
        // A horizontal rule stays one.
        assert_eq!(prepared("***").as_ref(), "***");
    }

    #[test]
    fn a_long_text_is_cut_and_says_so() {
        let long = "é".repeat(MOST_SHOWN);
        let shown = prepared(&long);
        assert!(shown.len() < MOST_SHOWN + 100);
        assert!(shown.ends_with("the rest is at the provider.)*"));
        assert_eq!(prepared("short").as_ref(), "short");
        assert_eq!(prepared("").as_ref(), "");
    }
}
