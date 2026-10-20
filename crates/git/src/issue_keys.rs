//! Issue keys: the `#123`, `owner/name#123` and `group/sub/project#123`
//! that name an issue of GitHub or GitLab in a commit's message; the
//! number a branch's name holds (`123-fix`, `fix-123`, `issue-123`); and
//! the name of a branch made for an issue.
//!
//! A key in a text is `#` and a number, with the path of a repository
//! before the `#` or not (`#12`, `octo/tools#12`, `group/sub/tools#12`).
//! The number has one to ten digits and does not begin with 0. A key is
//! never part of a word: a letter, a digit or `_` may not follow the
//! number, and before the key (before its path, when it has one) is the
//! start of a line, white space, or one of `( [ { < " ' * ~ , ; : |`.
//! So `abc#12`, `&#12;`, `\#12` and `#12abc` are no keys. A path has two
//! parts or more, each beginning with a letter, a digit or `_` and made of
//! those, `.` and `-`. Nothing in a code span (between backticks, on one
//! line) or a fenced block of code (between lines of three backticks or
//! tildes) is a key, and nor is anything in an address: a run without
//! white space that holds `://` or begins with `www.`. GitLab's `!12`
//! names a merge request and is no key.
//!
//! A branch's number is read from the last part of its name (after the
//! last `/`), split into words at `-` and `_`, and is the first of:
//! 1. a word `#123`;
//! 2. the first word, when it is a number and the second is not one
//!    (`123-fix`, `123`; not `2024-10-notes`);
//! 3. a number right after the word `issue`, `issues`, `gh`, `bug` or `fix`,
//!    in any case (`fix-123-login`, `issue-123`, `GH-123`);
//! 4. the last word, when it is a number and the one before it is neither
//!    a number nor written in two to ten capitals other than `GH`, which
//!    is how another tracker's key looks (`login-123`; not `PROJ-123`).
//!
//! A number is one to ten digits not beginning with 0, as in a text.
//!
//! A branch for an issue is named by a template with `{number}` and
//! `{title}` in it, `{number}-{title}` unless another is given. The title
//! is made a slug: lowercase ASCII letters, digits and single hyphens,
//! cut so that the whole name is at most 60 characters. What the template
//! says besides is kept as written, and the name has to be one git takes
//! (git-check-ref-format(1)).
//!
//! Sources: docs.github.com "Autolinked references and URLs" (issues by
//! `#26`, `GH-26` and `owner/repo#26`); docs.gitlab.com "GitLab Flavored
//! Markdown" (references: `#123`, `group/project#123`, `!123` for merge
//! requests); the CommonMark specification 0.31 (code spans, fenced code
//! blocks); git-check-ref-format(1).

use std::fmt;

use serde::{Deserialize, Serialize};

/// The most digits an issue's number has.
pub const MAX_DIGITS: usize = 10;
/// The longest path of a repository in a key, in bytes.
pub const MAX_PATH_BYTES: usize = 255;
/// The most parts a path has (`group/sub/…/project`).
pub const MAX_PATH_PARTS: usize = 20;
/// The most keys read from one text.
pub const MAX_KEYS: usize = 256;
/// How a branch for an issue is named when no template is given.
pub const DEFAULT_BRANCH_TEMPLATE: &str = "{number}-{title}";
/// The longest name a template makes from an issue's title, in characters.
pub const MAX_BRANCH_CHARS: usize = 60;
/// The longest template, in bytes.
pub const MAX_TEMPLATE_BYTES: usize = 255;

/// What may stand right before a key, besides white space and the start.
const BEFORE: &[char] = &['(', '[', '{', '<', '"', '\'', '*', '~', ',', ';', ':', '|'];

/// An issue named in a text.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IssueKey {
    /// The repository it names: `owner/name`, `group/sub/project`. None
    /// for `#123`, an issue of the repository the text belongs to.
    pub repo: Option<String>,
    /// The issue's number.
    pub number: u64,
}

impl IssueKey {
    /// Read a key written alone: `123`, `#123`, `owner/name#123`. White
    /// space around it is let go.
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let (repo, digits) = match text.rsplit_once('#') {
            Some(("", digits)) => (None, digits),
            Some((path, digits)) => (Some(path), digits),
            None => (None, text),
        };
        let number = number(digits)?;
        let repo = match repo {
            Some(path) if valid_path(path) => Some(path.to_owned()),
            Some(_) => return None,
            None => None,
        };
        Some(Self { repo, number })
    }

    /// Whether a key found in a text is this one: the same number, and
    /// the same repository (letters in any case) when this one names one.
    /// `#12` asked for is found as `#12` and as any `owner/name#12`.
    pub fn finds(&self, found: &IssueKey) -> bool {
        self.number == found.number
            && match (&self.repo, &found.repo) {
                (None, _) => true,
                (Some(wanted), Some(found)) => wanted.eq_ignore_ascii_case(found),
                (Some(_), None) => false,
            }
    }
}

impl fmt::Display for IssueKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repo {
            Some(repo) => write!(f, "{repo}#{}", self.number),
            None => write!(f, "#{}", self.number),
        }
    }
}

/// A key and where it is in the text it was read from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyAt {
    pub key: IssueKey,
    /// Where it begins, in bytes, its path included.
    pub start: usize,
    /// Where it ends, exclusive.
    pub end: usize,
}

/// A number as a key has it: one to [`MAX_DIGITS`] ASCII digits, not
/// beginning with 0.
fn number(digits: &str) -> Option<u64> {
    if digits.is_empty()
        || digits.len() > MAX_DIGITS
        || digits.starts_with('0')
        || !digits.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    digits.parse().ok()
}

/// Whether a character may be in a path: letters and digits of ASCII,
/// `_`, `.`, `-` and the `/` between parts.
fn in_path(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-' | '/')
}

/// Whether a path names a repository: two parts or more, each beginning
/// with a letter, a digit or `_`.
fn valid_path(path: &str) -> bool {
    if path.is_empty() || path.len() > MAX_PATH_BYTES || !path.chars().all(in_path) {
        return false;
    }
    let mut parts = 0usize;
    for part in path.split('/') {
        parts = parts.saturating_add(1);
        let first_fits = part
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        if !first_fits || parts > MAX_PATH_PARTS {
            return false;
        }
    }
    parts >= 2
}

/// Whether a character is part of a word: a key may not touch one.
fn wordy(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Whether a key may begin after this character (none: the start).
fn may_follow(before: Option<char>) -> bool {
    match before {
        None => true,
        Some(c) => c.is_whitespace() || BEFORE.contains(&c),
    }
}

/// The keys in a text, in the order they are written; at most
/// [`MAX_KEYS`].
pub fn keys_in(text: &str) -> Vec<KeyAt> {
    let mut found = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    let mut line_start = 0usize;
    for line in text.split_inclusive('\n') {
        let at = line_start;
        line_start = line_start.saturating_add(line.len());
        if let Some(marker) = fence_marker(line) {
            match fence {
                None => fence = Some(marker),
                Some((open, length)) if marker.0 == open && marker.1 >= length => {
                    // A closing fence has nothing after its marks.
                    let rest = line.trim().trim_start_matches(open);
                    if rest.is_empty() {
                        fence = None;
                    }
                }
                Some(_) => {}
            }
            continue;
        }
        if fence.is_some() {
            continue;
        }
        keys_in_line(line, at, &mut found);
        if found.len() >= MAX_KEYS {
            found.truncate(MAX_KEYS);
            break;
        }
    }
    found
}

/// The character and the count of the marks a line of a fence begins
/// with: three backticks or tildes or more, indented three spaces at most.
fn fence_marker(line: &str) -> Option<(char, usize)> {
    let indent = line
        .len()
        .saturating_sub(line.trim_start_matches(' ').len());
    if indent > 3 {
        return None;
    }
    let rest = line.get(indent..)?;
    let mark = rest.chars().next().filter(|c| matches!(c, '`' | '~'))?;
    let count = rest.chars().take_while(|c| *c == mark).count();
    (count >= 3).then_some((mark, count))
}

/// The keys of one line that is not in a fence, `offset` being where the
/// line begins in the text.
fn keys_in_line(line: &str, offset: usize, found: &mut Vec<KeyAt>) {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    let mut ix = 0usize;
    while let Some(&(at, c)) = chars.get(ix) {
        let before = ix
            .checked_sub(1)
            .and_then(|prev| chars.get(prev))
            .map(|p| p.1);
        if c == '`' {
            ix = after_code_span(&chars, ix);
            continue;
        }
        // At the start of a run without white space: an address is passed
        // over whole.
        if !c.is_whitespace() && before.is_none_or(char::is_whitespace) {
            let end = chars
                .iter()
                .skip(ix)
                .position(|(_, c)| c.is_whitespace())
                .map_or(chars.len(), |len| ix.saturating_add(len));
            let run_end = chars.get(end).map_or(line.len(), |(at, _)| *at);
            let run = line.get(at..run_end).unwrap_or_default();
            if run.contains("://") || run.get(..4).is_some_and(|w| w.eq_ignore_ascii_case("www.")) {
                ix = end;
                continue;
            }
        }
        if c == '#'
            && let Some((key, start, end_ix)) = key_at(line, &chars, ix)
        {
            let end = chars.get(end_ix).map_or(line.len(), |(at, _)| *at);
            found.push(KeyAt {
                key,
                start: offset.saturating_add(start),
                end: offset.saturating_add(end),
            });
            if found.len() >= MAX_KEYS {
                return;
            }
            ix = end_ix;
            continue;
        }
        ix = ix.saturating_add(1);
    }
}

/// Past a code span whose backticks begin at `ix`: after its closing run
/// of as many backticks, or after the opening run when the line has none.
fn after_code_span(chars: &[(usize, char)], ix: usize) -> usize {
    let run = |from: usize| {
        chars
            .iter()
            .skip(from)
            .take_while(|(_, c)| *c == '`')
            .count()
    };
    let opening = run(ix);
    let mut look = ix.saturating_add(opening);
    while let Some(&(_, c)) = chars.get(look) {
        if c == '`' {
            let length = run(look);
            if length == opening {
                return look.saturating_add(length);
            }
            look = look.saturating_add(length);
        } else {
            look = look.saturating_add(1);
        }
    }
    ix.saturating_add(opening)
}

/// The key whose `#` is at `hash` (an index into `chars`): the key, the
/// byte where it begins in the line, and the index past its number.
fn key_at(line: &str, chars: &[(usize, char)], hash: usize) -> Option<(IssueKey, usize, usize)> {
    let digits_from = hash.saturating_add(1);
    let digit_count = chars
        .iter()
        .skip(digits_from)
        .take_while(|(_, c)| c.is_ascii_digit())
        .count();
    let digits_end = digits_from.saturating_add(digit_count);
    if chars.get(digits_end).is_some_and(|(_, c)| wordy(*c)) {
        return None;
    }
    let byte = |ix: usize| chars.get(ix).map_or(line.len(), |(at, _)| *at);
    let number = number(line.get(byte(digits_from)..byte(digits_end))?)?;
    // The path before the `#`, if there is one.
    let path_count = chars
        .iter()
        .take(hash)
        .rev()
        .take_while(|(_, c)| in_path(*c))
        .count();
    let path_from = hash.saturating_sub(path_count);
    let before = path_from
        .checked_sub(1)
        .and_then(|ix| chars.get(ix))
        .map(|(_, c)| *c);
    if !may_follow(before) {
        return None;
    }
    let repo = if path_count == 0 {
        None
    } else {
        let path = line.get(byte(path_from)..byte(hash))?;
        if !valid_path(path) {
            return None;
        }
        Some(path.to_owned())
    };
    Some((IssueKey { repo, number }, byte(path_from), digits_end))
}

/// The number of the issue a branch's name holds, by the rule in this
/// module's description; none when it holds none.
pub fn branch_number(name: &str) -> Option<u64> {
    branch_key(name).map(|(number, _)| number)
}

/// The number of the issue a branch's name holds, by the rule in this
/// module's description, and where in the name it stands: the bytes of
/// the word that holds it (`#123`, or `123`), for the name to show it as a
/// link. None when it holds none.
pub fn branch_key(name: &str) -> Option<(u64, std::ops::Range<usize>)> {
    let last = name.rsplit('/').next().unwrap_or(name);
    if last.len() > 255 {
        return None;
    }
    // Each word with where it begins in the whole name; the words are
    // apart by one byte, `-` or `_`.
    let base = name.len().saturating_sub(last.len());
    let mut words: Vec<(usize, &str)> = Vec::new();
    let mut at = base;
    for word in last.split(['-', '_']) {
        words.push((at, word));
        at = at.saturating_add(word.len()).saturating_add(1);
    }
    let span = |start: usize, word: &str| start..start.saturating_add(word.len());
    let is_number = |word: &str| number(word).is_some();
    // 1. `#123`.
    if let Some(found) = words.iter().find_map(|&(start, word)| {
        word.strip_prefix('#')
            .and_then(number)
            .map(|found| (found, span(start, word)))
    }) {
        return Some(found);
    }
    // 2. The first word.
    if let Some(&(start, word)) = words.first()
        && let Some(first) = number(word)
        && !words.get(1).is_some_and(|(_, next)| is_number(next))
    {
        return Some((first, span(start, word)));
    }
    // 3. After a word that says what follows.
    let marker = |word: &str| {
        ["issue", "issues", "gh", "bug", "fix"]
            .iter()
            .any(|said| word.eq_ignore_ascii_case(said))
    };
    if let Some(found) = words.windows(2).find_map(|pair| match pair {
        [(_, said), (start, value)] if marker(said) => {
            number(value).map(|found| (found, span(*start, value)))
        }
        _ => None,
    }) {
        return Some(found);
    }
    // 4. The last word.
    let count = words.len();
    if count >= 2
        && let Some(&(start, word)) = words.last()
        && let Some(last) = number(word)
    {
        let previous = words
            .get(count.saturating_sub(2))
            .map(|(_, previous)| *previous)
            .unwrap_or("");
        let tracker = (2..=10).contains(&previous.len())
            && previous.chars().all(|c| c.is_ascii_uppercase())
            && previous != "GH";
        if number(previous).is_none() && !tracker {
            return Some((last, span(start, word)));
        }
    }
    None
}

/// An issue's title as a branch's name has it: lowercase ASCII letters and
/// digits, with single hyphens between runs of them.
pub fn slug(title: &str) -> String {
    let mut slug = String::with_capacity(title.len().min(MAX_BRANCH_CHARS * 2));
    for c in title.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c.to_ascii_lowercase());
        } else if !slug.is_empty() && !slug.ends_with('-') {
            slug.push('-');
        }
        if slug.len() > MAX_TEMPLATE_BYTES {
            break;
        }
    }
    slug.trim_end_matches('-').to_owned()
}

/// Cut a slug to at most `room` characters, at a hyphen when there is one
/// past the middle of that room, without a hyphen at its end.
fn cut_slug(slug: &str, room: usize) -> &str {
    if slug.len() <= room {
        return slug;
    }
    let cut = slug.get(..room).unwrap_or_default();
    let at_hyphen = cut
        .rfind('-')
        .filter(|at| *at >= room / 2)
        .and_then(|at| cut.get(..at));
    at_hyphen.unwrap_or(cut).trim_end_matches('-')
}

/// The name of a branch for an issue, from a template (the default one
/// when none is given), the issue's number and its title; or what is wrong
/// with the name, in words for the user.
pub fn branch_for_issue(
    template: Option<&str>,
    number: u64,
    title: &str,
) -> std::result::Result<String, String> {
    let template = template
        .map(str::trim)
        .filter(|template| !template.is_empty())
        .unwrap_or(DEFAULT_BRANCH_TEMPLATE);
    if template.len() > MAX_TEMPLATE_BYTES {
        return Err(say!(
            "issue-keys-template-too-long",
            most = MAX_TEMPLATE_BYTES
        ));
    }
    if template.chars().any(char::is_control) {
        return Err(say!("issue-keys-template-control").to_owned());
    }
    let with_number = template.replace("{number}", &number.to_string());
    let titles = with_number.matches("{title}").count();
    let fixed = with_number.replace("{title}", "").chars().count();
    let name = if let Some(room) = MAX_BRANCH_CHARS.saturating_sub(fixed).checked_div(titles) {
        let slug = slug(title);
        let cut = cut_slug(&slug, room);
        let made = with_number.replace("{title}", cut);
        if cut.is_empty() {
            // No title to put there: the hyphen or slash meant to lead to
            // it goes too.
            made.trim_end_matches(['-', '_', '/', '.']).to_owned()
        } else {
            made
        }
    } else {
        // No `{title}`: the name is what the template says.
        with_number
    };
    if name.is_empty() {
        return Err(say!("issue-keys-no-name").to_owned());
    }
    match crate::branch::name_problem(&name) {
        Some(problem) => Err(problem),
        None => Ok(name),
    }
}

#[cfg(test)]
mod tests {
    use super::{IssueKey, branch_for_issue, branch_key, branch_number, keys_in, slug};

    fn keys(text: &str) -> Vec<String> {
        keys_in(text)
            .into_iter()
            .map(|found| {
                assert_eq!(
                    text.get(found.start..found.end).map(str::to_owned),
                    Some(found.key.to_string()),
                    "{text:?}"
                );
                found.key.to_string()
            })
            .collect()
    }

    #[test]
    fn keys_are_read_where_they_stand_alone() {
        assert_eq!(keys("Fix the lexer (#12)"), ["#12"]);
        assert_eq!(keys("#7 first"), ["#7"]);
        assert_eq!(keys("fixes #12, #13; and #14."), ["#12", "#13", "#14"]);
        assert_eq!(keys("see octo/tools#5"), ["octo/tools#5"]);
        assert_eq!(keys("group/sub/project#9 too"), ["group/sub/project#9"]);
        assert_eq!(keys("Refs: a-b/c.d_e#1"), ["a-b/c.d_e#1"]);
        assert_eq!(keys("line\n#3\r\n\"#4\""), ["#3", "#4"]);
        assert_eq!(keys("~~#5~~ *#6* [#7](x)"), ["#5", "#6", "#7"]);
        assert_eq!(keys("#12-fix #12."), ["#12", "#12"]);
    }

    #[test]
    fn keys_are_not_read_inside_words_addresses_or_code() {
        for text in [
            "abc#12",
            "C#1 and F#2",
            "&#123;",
            "\\#12",
            "##12",
            "#12abc",
            "#12_",
            "#0",
            "#012",
            "#",
            "# 12",
            "#12345678901",
            "!12 is a merge request",
            "see https://github.com/o/r/issues/1#issuecomment-2",
            "http://example.org/page#12",
            "(https://example.org/#12)",
            "www.example.org/#12",
            "`#12` in code",
            "``a ` #12``",
            "a/#12",
            "/a/b#12",
            "a//b#12",
            ".a/b#12",
            "me@a/b#12",
            "x=#12",
            "#12é",
        ] {
            assert_eq!(keys(text), Vec::<String>::new(), "{text:?}");
        }
        assert_eq!(keys("`code` #4 `more`"), ["#4"]);
        assert_eq!(keys("an ` alone #4"), ["#4"]);
        assert_eq!(
            keys("before #1\n```\n#2\n  ~~~\n#3\n```\nafter #4\n~~~~\n#5\n~~~\n#6"),
            ["#1", "#4"]
        );
        assert_eq!(keys("    #8 indented is not a fence"), ["#8"]);
    }

    #[test]
    fn many_keys_are_bounded() {
        let text = "#1 ".repeat(1000);
        assert_eq!(keys_in(&text).len(), super::MAX_KEYS);
        let long = format!("{}#1", "a/".repeat(500));
        assert!(keys_in(&long).is_empty());
    }

    #[test]
    fn a_key_alone_is_read_and_found() {
        let parsed = |text: &str| IssueKey::parse(text).map(|key| key.to_string());
        assert_eq!(parsed("123").as_deref(), Some("#123"));
        assert_eq!(parsed(" #123 ").as_deref(), Some("#123"));
        assert_eq!(parsed("Octo/Tools#5").as_deref(), Some("Octo/Tools#5"));
        assert_eq!(parsed("g/s/p#5").as_deref(), Some("g/s/p#5"));
        for bad in [
            "", "#", "abc", "#0", "12a", "tools#5", "a/#5", "a/b#", "a b#1", "1#2",
        ] {
            assert_eq!(parsed(bad), None, "{bad:?}");
        }
        let wanted = IssueKey::parse("octo/tools#5").unwrap();
        assert!(wanted.finds(&IssueKey::parse("OCTO/tools#5").unwrap()));
        assert!(!wanted.finds(&IssueKey::parse("#5").unwrap()));
        assert!(!wanted.finds(&IssueKey::parse("octo/tools#6").unwrap()));
        let bare = IssueKey::parse("#5").unwrap();
        assert!(bare.finds(&IssueKey::parse("other/thing#5").unwrap()));
        assert!(bare.finds(&IssueKey::parse("#5").unwrap()));
    }

    #[test]
    fn a_branch_names_its_issue_by_the_rule() {
        for (name, number) in [
            ("123-fix", Some(123)),
            ("123", Some(123)),
            ("fix-123", Some(123)),
            ("feature/123-x", Some(123)),
            ("issue-123", Some(123)),
            ("gh-123", Some(123)),
            ("GH-123", Some(123)),
            ("#123", Some(123)),
            ("feature/#123-login", Some(123)),
            ("fix-123-login", Some(123)),
            ("bug_77_crash", Some(77)),
            ("login-page-42", Some(42)),
            ("users/ada/9-docs", Some(9)),
            ("123/topic", None),
            ("main", None),
            ("release-1.2", None),
            ("2024-10-notes", None),
            ("PROJ-123", None),
            ("feature-PROJ-123", None),
            ("v2-hotfix", None),
            ("0-zero", None),
            ("fix-0123", None),
            ("12345678901-long", None),
            ("", None),
        ] {
            assert_eq!(branch_number(name), number, "{name:?}");
        }
    }

    /// Where the key stands in the name, so that the name can show it as
    /// a link: the word that holds the number, `#` and all.
    #[test]
    fn a_branch_names_where_its_issue_stands() {
        for (name, key) in [
            ("qa/1-the-button-does-nothing", Some((1, "1"))),
            ("qa/#1-button", Some((1, "#1"))),
            ("fix-123-login", Some((123, "123"))),
            ("users/ada/login-page-42", Some((42, "42"))),
            ("bug_77_crash", Some((77, "77"))),
            ("123", Some((123, "123"))),
            ("PROJ-123", None),
            ("main", None),
            ("", None),
        ] {
            let found = branch_key(name).map(|(number, at)| (number, name.get(at).unwrap()));
            assert_eq!(found, key, "{name:?}");
        }
    }

    #[test]
    fn a_branch_for_an_issue_is_named_from_its_title() {
        assert_eq!(
            slug("Fix the Lexer: quotes & \"spaces\"!"),
            "fix-the-lexer-quotes-spaces"
        );
        assert_eq!(slug("Café — déjà vu"), "caf-d-j-vu");
        assert_eq!(slug("   "), "");
        assert_eq!(
            branch_for_issue(None, 12, "Fix the lexer").unwrap(),
            "12-fix-the-lexer"
        );
        assert_eq!(
            branch_for_issue(Some("feature/{number}-{title}"), 12, "Add login").unwrap(),
            "feature/12-add-login"
        );
        assert_eq!(
            branch_for_issue(Some("Ada/fix"), 12, "x").unwrap(),
            "Ada/fix"
        );
        assert_eq!(branch_for_issue(None, 12, "!!!").unwrap(), "12");
        let long = branch_for_issue(
            None,
            1234,
            "When the window is resized while a rebase is planned the list jumps to the top",
        )
        .unwrap();
        assert!(long.chars().count() <= 60, "{long}");
        assert_eq!(
            long,
            "1234-when-the-window-is-resized-while-a-rebase-is-planned"
        );
        let one_word = branch_for_issue(None, 1, &"a".repeat(300)).unwrap();
        assert_eq!(one_word.len(), 60);
        assert!(branch_for_issue(Some("a..b-{title}"), 1, "x").is_err());
        assert!(branch_for_issue(Some("x\n{title}"), 1, "x").is_err());
        assert!(branch_for_issue(Some(&"x".repeat(300)), 1, "x").is_err());
        assert!(branch_for_issue(Some("-{number}"), 1, "x").is_err());
    }
}
