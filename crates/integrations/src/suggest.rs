//! Suggestions made from local edits: with the branch of a pull request
//! checked out, what was changed in a file of it in the working tree is
//! offered as suggestions on the pull request's lines, the way a
//! reviewer would write them by hand: GitHub's ```suggestion block in a
//! comment on the lines it replaces, GitLab's suggestion on the last of
//! them, saying how many lines above it it takes.
//!
//! What was changed is what differs between the commit checked out and
//! the working tree, with no lines around it ([`bitbraid_git::diff::Hunk`]s
//! with no context). Each change becomes one suggestion on the lines of
//! the commit's version it replaces; a change that only adds lines takes
//! the line before them (the first line, for lines added at the top) and
//! offers it again with them. A suggestion is offered only on lines the
//! pull request's diff shows on its new side, all in one of its hunks
//! (where the providers take comments on lines), and only where the
//! commit checked out reads as the pull request's head does on those
//! lines; otherwise it is listed with the reason it cannot be sent.
//!
//! Sources: docs.github.com "Incorporating feedback in your pull request"
//! (suggested changes) and docs.github.com/en/rest/pulls/comments
//! ("Create a review comment for a pull request": `line`, `side`,
//! `start_line`); docs.gitlab.com "Suggest changes" (`suggestion:-N+M`)
//! and docs.gitlab.com/api/discussions ("Create a new thread in the merge
//! request diff"); the unified diff format as git-diff(1) prints it.

use serde::{Deserialize, Serialize};

use crate::model::ProviderKind;

/// The most suggestions made from one file.
pub const MOST_SUGGESTIONS: usize = 100;
/// The most lines one suggestion replaces or offers.
pub const MOST_LINES: usize = 200;

/// What a line of a pull request's diff is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    /// On both sides.
    Context,
    /// On the new side only.
    Added,
    /// On the old side only.
    Removed,
}

/// A line of a pull request's diff, with its numbers on each side.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffLine {
    pub kind: LineKind,
    pub old: Option<u64>,
    pub new: Option<u64>,
    pub text: String,
}

/// The two starts of a hunk's range, `@@ -12,3 +14,5 @@`; none when it is
/// not one.
fn starts(header: &str) -> Option<(u64, u64)> {
    let inner = header.strip_prefix("@@ ")?;
    let mut parts = inner.split(' ');
    let old = parts.next()?.strip_prefix('-')?;
    let new = parts.next()?.strip_prefix('+')?;
    let first = |range: &str| range.split(',').next()?.parse::<u64>().ok();
    Some((first(old)?, first(new)?))
}

/// The hunks of a unified diff as a provider sends it for one file, each
/// its lines with their numbers. Lines before the first range, and the
/// diff's notes about itself, are left out; a range that is no range ends
/// the reading.
pub fn hunks(patch: &str) -> Vec<Vec<DiffLine>> {
    let mut out: Vec<Vec<DiffLine>> = Vec::new();
    let (mut old, mut new) = (0u64, 0u64);
    for line in patch.lines() {
        if line.starts_with("@@") {
            let Some((old_start, new_start)) = starts(line) else {
                break;
            };
            (old, new) = (old_start, new_start);
            out.push(Vec::new());
            continue;
        }
        let Some(hunk) = out.last_mut() else {
            continue;
        };
        let (kind, text) = match line.chars().next() {
            Some('+') => (LineKind::Added, line.get(1..).unwrap_or_default()),
            Some('-') => (LineKind::Removed, line.get(1..).unwrap_or_default()),
            Some(' ') => (LineKind::Context, line.get(1..).unwrap_or_default()),
            // An empty context line, as some tools write it.
            None => (LineKind::Context, ""),
            Some(_) => continue,
        };
        let (at_old, at_new) = match kind {
            LineKind::Context => (Some(old), Some(new)),
            LineKind::Added => (None, Some(new)),
            LineKind::Removed => (Some(old), None),
        };
        if at_old.is_some() {
            old = old.saturating_add(1);
        }
        if at_new.is_some() {
            new = new.saturating_add(1);
        }
        hunk.push(DiffLine {
            kind,
            old: at_old,
            new: at_new,
            text: text.to_owned(),
        });
    }
    out
}

/// Where a line of the new side is in a pull request's diff: the hunk it
/// is in, and its number on the old side when both sides have it. None
/// when the diff does not show it.
pub fn place_of(hunks: &[Vec<DiffLine>], line: u64) -> Option<(usize, Option<u64>)> {
    hunks.iter().enumerate().find_map(|(at, hunk)| {
        hunk.iter()
            .find(|diff| diff.new == Some(line))
            .map(|diff| (at, diff.old))
    })
}

/// A change in the working tree: lines of the commit's version, from
/// `old_start` on, that became `new` (none removed when `old` is empty:
/// `new` comes after line `old_start`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalChange {
    pub old_start: u64,
    pub old: Vec<String>,
    pub new: Vec<String>,
}

impl LocalChange {
    /// The change one of git's hunks with no context says.
    pub fn of(hunk: &bitbraid_git::diff::Hunk) -> Self {
        use bitbraid_git::diff::LineKind as Kind;
        let text = |kind: Kind| {
            hunk.lines
                .iter()
                .filter(|line| line.kind == kind)
                .map(|line| line.text.clone())
                .collect()
        };
        Self {
            old_start: u64::from(hunk.old_start),
            old: text(Kind::Removed),
            new: text(Kind::Added),
        }
    }
}

/// A suggestion made from a change in the working tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Suggestion {
    /// The file.
    pub path: String,
    /// The first line of the pull request's head it replaces.
    pub start_line: u64,
    /// The last.
    pub line: u64,
    /// Those lines as the head has them.
    pub replaced: Vec<String>,
    /// What is offered in their place.
    pub suggested: Vec<String>,
    /// The comment that is sent, in Markdown.
    pub body: String,
    /// Why it cannot be sent, when it cannot.
    pub why_not: Option<String>,
}

/// What `pr.suggestions` answers.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Suggestions {
    /// The pull request's head.
    pub head_sha: String,
    /// The commit checked out.
    pub checked_out: Option<String>,
    /// The suggestions, file by file, in the order of the lines.
    pub suggestions: Vec<Suggestion>,
}

/// A fence of backquotes longer than any run of them in `lines`, three at
/// the least, so that the block cannot end early.
fn fence(lines: &[String]) -> String {
    let longest = lines
        .iter()
        .map(|line| {
            let mut most = 0usize;
            let mut run = 0usize;
            for c in line.chars() {
                run = if c == '`' { run.saturating_add(1) } else { 0 };
                most = most.max(run);
            }
            most
        })
        .max()
        .unwrap_or(0);
    "`".repeat(longest.saturating_add(1).max(3))
}

/// The comment a suggestion is sent as: a note first when one is given,
/// then the block. GitLab's block says how many lines above the one the
/// comment is on it replaces.
pub fn body(kind: ProviderKind, note: Option<&str>, lines: u64, suggested: &[String]) -> String {
    let fence = fence(suggested);
    let opening = match kind {
        ProviderKind::Github => format!("{fence}suggestion"),
        ProviderKind::Gitlab => format!("{fence}suggestion:-{}+0", lines.saturating_sub(1)),
    };
    let mut out = String::new();
    if let Some(note) = note.map(str::trim).filter(|note| !note.is_empty()) {
        out.push_str(note);
        out.push_str("\n\n");
    }
    out.push_str(&opening);
    out.push('\n');
    for line in suggested {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&fence);
    out
}

/// The suggestions for one file: its changes in the working tree against
/// `head` (the commit checked out, a line each), offered where the pull
/// request's `patch` shows its new side.
pub fn suggestions(
    path: &str,
    patch: &str,
    head: &[String],
    changes: &[LocalChange],
    kind: ProviderKind,
    note: Option<&str>,
) -> Vec<Suggestion> {
    let hunks = hunks(patch);
    let line_of = |number: u64| -> Option<&String> {
        usize::try_from(number)
            .ok()
            .and_then(|number| number.checked_sub(1))
            .and_then(|at| head.get(at))
    };
    let mut out = Vec::new();
    for change in changes.iter().take(MOST_SUGGESTIONS) {
        // The lines replaced, and what takes their place.
        let (start, replaced, suggested) = if change.old.is_empty() {
            if change.old_start == 0 {
                let first = line_of(1).cloned().unwrap_or_default();
                let mut offered = change.new.clone();
                offered.push(first.clone());
                (1, vec![first], offered)
            } else {
                let before = line_of(change.old_start).cloned().unwrap_or_default();
                let mut offered = vec![before.clone()];
                offered.extend(change.new.iter().cloned());
                (change.old_start, vec![before], offered)
            }
        } else {
            (change.old_start, change.old.clone(), change.new.clone())
        };
        let count = u64::try_from(replaced.len()).unwrap_or(u64::MAX).max(1);
        let end = start.saturating_add(count).saturating_sub(1);
        let why_not = if replaced.len() > MOST_LINES || suggested.len() > MOST_LINES {
            Some(say!("suggest-too-many-lines", most = MOST_LINES))
        } else if start == 0 || line_of(end).is_none() {
            Some(say!("suggest-not-in-commit").to_owned())
        } else {
            let places: Vec<Option<(usize, Option<u64>)>> =
                (start..=end).map(|line| place_of(&hunks, line)).collect();
            let hunk = places.first().copied().flatten().map(|(hunk, _)| hunk);
            let in_one =
                hunk.is_some() && places.iter().all(|place| place.map(|(at, _)| at) == hunk);
            if !in_one {
                Some(say!("suggest-not-in-one-part").to_owned())
            } else {
                // The head reads there as the commit checked out does.
                let differs = (start..=end).any(|line| {
                    let theirs = hunks
                        .iter()
                        .flatten()
                        .find(|diff| diff.new == Some(line))
                        .map(|diff| diff.text.as_str());
                    theirs != line_of(line).map(String::as_str)
                });
                differs.then(|| say!("suggest-head-differs").to_owned())
            }
        };
        out.push(Suggestion {
            path: path.to_owned(),
            start_line: start,
            line: end,
            body: body(kind, note, count, &suggested),
            replaced,
            suggested,
            why_not,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(text: &str) -> Vec<String> {
        text.lines().map(str::to_owned).collect()
    }

    fn change(old_start: u64, old: &[&str], new: &[&str]) -> LocalChange {
        LocalChange {
            old_start,
            old: old.iter().map(|line| (*line).to_owned()).collect(),
            new: new.iter().map(|line| (*line).to_owned()).collect(),
        }
    }

    /// A pull request that changes lines 3 and 4 of a file of eight, with
    /// a line of context on each side, and adds line 8.
    const PATCH: &str = "@@ -2,4 +2,4 @@ fn top()\n two\n-three\n-four\n+THREE\n+FOUR\n five\n@@ -7 +7,2 @@\n seven\n+eight";

    fn head() -> Vec<String> {
        lines("one\ntwo\nTHREE\nFOUR\nfive\nsix\nseven\neight")
    }

    #[test]
    fn a_patch_is_read_with_the_numbers_of_its_lines() {
        let read = hunks(PATCH);
        assert_eq!(read.len(), 2);
        assert_eq!(
            read[0][1],
            DiffLine {
                kind: LineKind::Removed,
                old: Some(3),
                new: None,
                text: "three".into()
            }
        );
        assert_eq!(read[0][3].new, Some(3));
        assert_eq!(place_of(&read, 3), Some((0, None)));
        assert_eq!(place_of(&read, 5), Some((0, Some(5))));
        assert_eq!(place_of(&read, 8), Some((1, None)));
        assert_eq!(place_of(&read, 6), None);
        assert!(hunks("no range here\n+x").is_empty());
        assert_eq!(hunks("@@ -x +y @@\n+a").len(), 0);
    }

    #[test]
    fn changes_become_suggestions_on_the_lines_they_replace() {
        let made = suggestions(
            "src/a.rs",
            PATCH,
            &head(),
            &[
                change(3, &["THREE"], &["Three"]),
                change(4, &[], &["four and a half"]),
                change(7, &["seven", "eight"], &["seven and eight"]),
                change(6, &["six"], &["SIX"]),
            ],
            ProviderKind::Github,
            None,
        );
        assert_eq!(made.len(), 4);
        assert_eq!((made[0].start_line, made[0].line), (3, 3));
        assert_eq!(made[0].body, "```suggestion\nThree\n```");
        assert_eq!(made[0].why_not, None);
        // Lines added after line 4: line 4 offered again with them.
        assert_eq!((made[1].start_line, made[1].line), (4, 4));
        assert_eq!(made[1].suggested, ["FOUR", "four and a half"]);
        // Two lines in another hunk.
        assert_eq!((made[2].start_line, made[2].line), (7, 8));
        assert_eq!(made[2].why_not, None);
        // Line 6 is not in the pull request's diff.
        assert!(made[3].why_not.is_some());
    }

    #[test]
    fn gitlab_says_how_many_lines_above() {
        let made = suggestions(
            "a",
            PATCH,
            &head(),
            &[change(3, &["THREE", "FOUR"], &["34"])],
            ProviderKind::Gitlab,
            Some("Shorter?"),
        );
        assert_eq!(made[0].body, "Shorter?\n\n```suggestion:-1+0\n34\n```");
        assert_eq!(made[0].line, 4);
    }

    #[test]
    fn a_head_that_reads_otherwise_is_not_suggested_on() {
        let mut stale = head();
        stale[2] = "three, older".into();
        let made = suggestions(
            "a",
            PATCH,
            &stale,
            &[change(3, &["three, older"], &["x"])],
            ProviderKind::Github,
            None,
        );
        assert!(made[0].why_not.as_deref().unwrap().contains("up to date"));
    }

    #[test]
    fn backquotes_in_a_suggestion_lengthen_its_fence() {
        let text = body(
            ProviderKind::Github,
            None,
            1,
            &["```rust".to_owned(), "x".to_owned()],
        );
        assert_eq!(text, "````suggestion\n```rust\nx\n````");
        // Lines added at the top go before the first.
        let made = suggestions(
            "a",
            "@@ -0,0 +1,2 @@\n+one\n+two",
            &lines("one\ntwo"),
            &[change(0, &[], &["zero"])],
            ProviderKind::Github,
            None,
        );
        assert_eq!(made[0].suggested, ["zero", "one"]);
        assert_eq!((made[0].start_line, made[0].line), (1, 1));
        assert_eq!(made[0].why_not, None);
    }
}
