//! Diffs: file lists between two trees with rename detection, and unified
//! hunks per file. Also blob-level diffs for a path between a commit, the
//! index and the worktree (what the staging views need).
//!
//! A file's line endings are told on each side (LF, CRLF or both), and a
//! change that is nothing but line endings is said to be one, as
//! `git diff --ignore-cr-at-eol` finds nothing in it.
//!
//! Sources: git-diff(1) (including `--ignore-space-at-eol`,
//! `--ignore-cr-at-eol` and `--ignore-all-space`), gitattributes(5)
//! (`text`, `eol`), git-config(1) (`core.autocrlf`), gix tree diff
//! documentation, imara-diff.

use gix::bstr::ByteSlice;
use gix::diff::blob::{
    Algorithm, InternedInput, UnifiedDiff, diff_with_slider_heuristics,
    unified_diff::{ConsumeHunk, ContextSize, DiffLineKind, HunkHeader},
};
use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, git_err};

/// What a file did between the two sides.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    /// Present only on the new side.
    Added,
    /// Present only on the old side.
    Deleted,
    /// Content or mode changed.
    Modified,
    /// Detected rename.
    Renamed,
    /// Detected copy.
    Copied,
    /// File, symlink or submodule type changed.
    TypeChanged,
}

/// Kind of a diff line.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineKind {
    /// Unchanged context.
    Context,
    /// Added in the new side.
    Added,
    /// Removed from the old side.
    Removed,
}

/// One line of a hunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Text without the trailing newline.
    pub text: String,
    /// 1-based line number on the old side, absent for added lines.
    pub old_no: Option<u32>,
    /// 1-based line number on the new side, absent for removed lines.
    pub new_no: Option<u32>,
    /// The line had no trailing newline (last line of a file).
    pub no_newline: bool,
}

/// A unified-diff hunk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Hunk {
    /// 1-based first old line (0 when the old side is empty).
    pub old_start: u32,
    /// Old line count covered.
    pub old_lines: u32,
    /// 1-based first new line (0 when the new side is empty).
    pub new_start: u32,
    /// New line count covered.
    pub new_lines: u32,
    /// Lines in order.
    pub lines: Vec<Line>,
}

impl Hunk {
    /// The `@@ -a,b +c,d @@` header as git prints it.
    pub fn header(&self) -> String {
        fn side(start: u32, lines: u32) -> String {
            if lines == 1 {
                format!("{start}")
            } else {
                format!("{start},{lines}")
            }
        }
        format!(
            "@@ -{} +{} @@",
            side(self.old_start, self.old_lines),
            side(self.new_start, self.new_lines)
        )
    }
}

/// The diff of one file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileDiff {
    pub kind: FileChangeKind,
    /// Path on the new side (old side for deletions).
    pub path: String,
    /// Path on the old side for renames and copies.
    pub old_path: Option<String>,
    /// Blob id on the old side.
    pub old_id: Option<Oid>,
    /// Blob id on the new side.
    pub new_id: Option<Oid>,
    /// Mode on the old side as git prints it, for example `100644`.
    pub old_mode: Option<String>,
    /// Mode on the new side as git prints it.
    pub new_mode: Option<String>,
    /// Either side looks binary; no hunks.
    pub binary: bool,
    /// Hunks (empty for binary files, pure mode changes and submodules).
    pub hunks: Vec<Hunk>,
    /// Added line count.
    pub additions: u32,
    /// Removed line count.
    pub deletions: u32,
    /// Size of the old side in bytes, where there is one. Not known for a
    /// submodule.
    #[serde(default)]
    pub old_size: Option<u64>,
    /// Size of the new side in bytes.
    #[serde(default)]
    pub new_size: Option<u64>,
    /// A side is larger than the caller wanted compared (`max_bytes`):
    /// the file was not read, there are no hunks, and whether it is
    /// binary is not known. Asked for again without the bound, it is
    /// compared like any other.
    #[serde(default)]
    pub too_large: bool,
    /// How the lines end on the two sides, and whether that is all that
    /// changed. Not told for a binary file, a submodule or a file that was
    /// not compared.
    #[serde(default)]
    pub line_endings: FileLineEndings,
}

/// How the lines of a text end.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LineEndings {
    /// Every line ends in a line feed alone.
    Lf,
    /// Every line ends in a carriage return and a line feed.
    Crlf,
    /// Some lines end one way and some the other.
    Mixed,
}

impl LineEndings {
    /// How the lines of `bytes` end; none when no line ends in it (one
    /// line without an end, or nothing). A carriage return that no line
    /// feed follows ends no line, as git has it.
    pub fn of(bytes: &[u8]) -> Option<Self> {
        let (mut lf, mut crlf) = (false, false);
        let mut previous = 0u8;
        for &byte in bytes {
            if byte == b'\n' {
                if previous == b'\r' {
                    crlf = true;
                } else {
                    lf = true;
                }
                if lf && crlf {
                    return Some(Self::Mixed);
                }
            }
            previous = byte;
        }
        match (lf, crlf) {
            (true, false) => Some(Self::Lf),
            (false, true) => Some(Self::Crlf),
            (true, true) => Some(Self::Mixed),
            (false, false) => None,
        }
    }
}

/// The line endings of a file on the two sides of a diff.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileLineEndings {
    /// The old side, as it was compared: none where it is not there or
    /// no line ends in it.
    pub old: Option<LineEndings>,
    /// The new side, as it was compared.
    pub new: Option<LineEndings>,
    /// The worktree's file as it is on disk, when the worktree is a side
    /// and git changes its line endings on the way into the repository
    /// (`core.autocrlf`, the `eol` and `text` attributes). The sides above
    /// are then what git would store, which is what is compared.
    pub worktree: Option<LineEndings>,
    /// The two sides differ, and in nothing but how their lines end:
    /// `git diff --ignore-cr-at-eol` shows nothing of the file.
    pub only_change: bool,
}

impl FileLineEndings {
    /// What the line endings of two texts are, the worktree's on disk
    /// being `worktree` where git converts them.
    pub fn between(old: &[u8], new: &[u8], worktree: Option<LineEndings>) -> Self {
        Self {
            old: LineEndings::of(old),
            new: LineEndings::of(new),
            worktree,
            only_change: only_line_endings_differ(old, new),
        }
    }
}

/// A line without its line feed, and without a carriage return before
/// it: what is left of a line when a CR at its end does not count.
fn without_line_end(line: &[u8]) -> &[u8] {
    line.strip_suffix(b"\r\n")
        .or_else(|| line.strip_suffix(b"\n"))
        .unwrap_or(line)
}

/// Whether two texts differ and nothing but their line endings does: the
/// same lines, once a CR before a line feed and the line feed itself are
/// taken off each, as `git diff --ignore-cr-at-eol` compares them (which
/// also passes over a line feed missing at the end of the file). A CR at
/// the end of a last line without a line feed still counts.
pub fn only_line_endings_differ(old: &[u8], new: &[u8]) -> bool {
    old != new
        && !looks_binary(old)
        && !looks_binary(new)
        && lines_of(old)
            .map(without_line_end)
            .eq(lines_of(new).map(without_line_end))
}

impl FileDiff {
    /// Whether this is a submodule on either side: its ids are then commits
    /// of the submodule, and it has no hunks.
    pub fn is_submodule(&self) -> bool {
        const GITLINK: &str = "160000";
        self.old_mode.as_deref() == Some(GITLINK) || self.new_mode.as_deref() == Some(GITLINK)
    }
}

/// How whitespace counts when two lines are compared.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Whitespace {
    /// Every byte counts.
    #[default]
    Exact,
    /// Whitespace at the end of a line does not (`--ignore-space-at-eol`).
    IgnoreTrailing,
    /// No whitespace does (`--ignore-all-space`).
    IgnoreAll,
    /// A carriage return at the end of a line does not
    /// (`--ignore-cr-at-eol`); nor, as in git, a line feed missing at the
    /// end of the file.
    IgnoreCrAtEol,
}

/// A line that compares by what is left of it once the ignored whitespace
/// is gone, and prints as it is.
#[derive(Debug, Clone, Copy)]
struct LooseLine<'a> {
    raw: &'a [u8],
    whitespace: Whitespace,
}

impl LooseLine<'_> {
    fn significant(&self) -> impl Iterator<Item = u8> + '_ {
        let (kept, drop_all) = match self.whitespace {
            Whitespace::Exact => (self.raw, false),
            Whitespace::IgnoreTrailing => (self.raw.trim_ascii_end(), false),
            Whitespace::IgnoreAll => (self.raw, true),
            Whitespace::IgnoreCrAtEol => (without_line_end(self.raw), false),
        };
        kept.iter()
            .copied()
            .filter(move |byte| !(drop_all && byte.is_ascii_whitespace()))
    }
}

impl PartialEq for LooseLine<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.significant().eq(other.significant())
    }
}

impl Eq for LooseLine<'_> {}

impl std::hash::Hash for LooseLine<'_> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        for byte in self.significant() {
            state.write_u8(byte);
        }
        state.write_u8(0xff);
    }
}

impl AsRef<[u8]> for LooseLine<'_> {
    fn as_ref(&self) -> &[u8] {
        self.raw
    }
}

/// The lines of a buffer, each with its line ending.
fn lines_of(bytes: &[u8]) -> impl Iterator<Item = &[u8]> {
    bytes.split_inclusive(|byte| *byte == b'\n')
}

/// A buffer as lines compared loosely.
#[derive(Debug, Clone, Copy)]
struct LooseLines<'a> {
    bytes: &'a [u8],
    whitespace: Whitespace,
}

impl<'a> gix::diff::blob::TokenSource for LooseLines<'a> {
    type Token = LooseLine<'a>;
    type Tokenizer = Box<dyn Iterator<Item = LooseLine<'a>> + 'a>;

    fn tokenize(&self) -> Self::Tokenizer {
        let whitespace = self.whitespace;
        Box::new(lines_of(self.bytes).map(move |raw| LooseLine { raw, whitespace }))
    }

    fn estimate_tokens(&self) -> u32 {
        u32::try_from(self.bytes.len() / 40).unwrap_or(u32::MAX)
    }
}

/// Options for tree diffs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiffOptions {
    /// Context lines per hunk side.
    #[serde(default = "default_context")]
    pub context: u32,
    /// Restrict to these paths (exact or directory prefix). Empty means all.
    #[serde(default)]
    pub paths: Vec<String>,
    /// Detect renames and copies.
    #[serde(default = "default_true")]
    pub detect_renames: bool,
    /// How whitespace counts when lines are compared.
    #[serde(default)]
    pub whitespace: Whitespace,
    /// Files with a side larger than this many bytes are listed and not
    /// compared (`FileDiff::too_large`); every file is compared when
    /// absent.
    #[serde(default)]
    pub max_bytes: Option<u64>,
}

fn default_context() -> u32 {
    3
}
fn default_true() -> bool {
    true
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            context: 3,
            paths: Vec::new(),
            detect_renames: true,
            whitespace: Whitespace::Exact,
            max_bytes: None,
        }
    }
}

/// One side of a diff.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DiffSide {
    /// A commit's tree.
    Commit(Oid),
    /// A tree object.
    Tree(Oid),
    /// The empty tree (for root commits).
    Empty,
    /// The index.
    Index,
    /// The working tree.
    Worktree,
}

impl DiffSide {
    /// Parse `HEAD`, a revision, `index`, `worktree` or `empty`.
    pub fn parse(repo: &Repository, text: &str) -> Result<Self> {
        match text.trim() {
            "index" | "INDEX" => Ok(Self::Index),
            "worktree" | "WORKTREE" | "working" => Ok(Self::Worktree),
            "empty" | "EMPTY" => Ok(Self::Empty),
            rev => {
                let id = repo
                    .gix()
                    .rev_parse_single(rev)
                    .map_err(|err| Error::Invalid {
                        what: say!("diff-what-revision"),
                        detail: format!("{rev:?}: {}", crate::with_causes(&err)),
                    })?;
                let object = id
                    .object()
                    .map_err(git_err(say!("diff-loading-revision")))?;
                match object.kind {
                    gix::object::Kind::Commit => Ok(Self::Commit(Oid::from(id.detach()))),
                    gix::object::Kind::Tree => Ok(Self::Tree(Oid::from(id.detach()))),
                    gix::object::Kind::Tag => {
                        let commit = object
                            .peel_to_kind(gix::object::Kind::Commit)
                            .map_err(git_err(say!("diff-peeling-tag")))?;
                        Ok(Self::Commit(Oid::from(commit.id)))
                    }
                    other => Err(Error::Invalid {
                        what: say!("diff-what-revision"),
                        detail: say!(
                            "diff-revision-is-kind",
                            rev = format!("{rev:?}"),
                            other = other.to_string()
                        ),
                    }),
                }
            }
        }
    }
}

fn mode_class(mode: gix::object::tree::EntryMode) -> u8 {
    if mode.is_commit() {
        2
    } else if mode.is_link() {
        1
    } else if mode.is_tree() {
        3
    } else {
        0
    }
}

fn octal(mode: gix::object::tree::EntryMode) -> String {
    format!("{:06o}", mode.value())
}

/// Collects unified-diff hunks from gix's sink.
#[derive(Default)]
struct Collector {
    hunks: Vec<Hunk>,
}

impl ConsumeHunk for Collector {
    type Out = Vec<Hunk>;

    fn consume_hunk(
        &mut self,
        header: HunkHeader,
        lines: &[(DiffLineKind, &[u8])],
    ) -> std::io::Result<()> {
        let mut old_no = header.before_hunk_start;
        let mut new_no = header.after_hunk_start;
        let mut out = Vec::with_capacity(lines.len());
        for (kind, raw) in lines {
            let no_newline = !raw.ends_with(b"\n");
            let text = raw
                .strip_suffix(b"\n")
                .unwrap_or(raw)
                .to_str_lossy()
                .into_owned();
            let line = match kind {
                DiffLineKind::Context => {
                    let l = Line {
                        kind: LineKind::Context,
                        text,
                        old_no: Some(old_no),
                        new_no: Some(new_no),
                        no_newline,
                    };
                    old_no = old_no.saturating_add(1);
                    new_no = new_no.saturating_add(1);
                    l
                }
                DiffLineKind::Add => {
                    let l = Line {
                        kind: LineKind::Added,
                        text,
                        old_no: None,
                        new_no: Some(new_no),
                        no_newline,
                    };
                    new_no = new_no.saturating_add(1);
                    l
                }
                DiffLineKind::Remove => {
                    let l = Line {
                        kind: LineKind::Removed,
                        text,
                        old_no: Some(old_no),
                        new_no: None,
                        no_newline,
                    };
                    old_no = old_no.saturating_add(1);
                    l
                }
            };
            out.push(line);
        }
        // gix numbers starts 1-based even for empty sides; git prints the
        // 0-based position in that case (`@@ -0,0 +1 @@` for an addition).
        let git_start = |start: u32, len: u32| {
            if len == 0 {
                start.saturating_sub(1)
            } else {
                start
            }
        };
        self.hunks.push(Hunk {
            old_start: git_start(header.before_hunk_start, header.before_hunk_len),
            old_lines: header.before_hunk_len,
            new_start: git_start(header.after_hunk_start, header.after_hunk_len),
            new_lines: header.after_hunk_len,
            lines: out,
        });
        Ok(())
    }

    fn finish(self) -> Self::Out {
        self.hunks
    }
}

/// git's heuristic: a NUL in the first 8000 bytes means binary.
pub fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|b| *b == 0)
}

/// A context size the hunk builder can do arithmetic with. It adds the
/// context to line numbers and doubles it, so a caller's `u32::MAX` ("the
/// whole file") overflows there. More context than the longer side has
/// lines changes nothing, so that is where it is capped.
fn bounded_context(context: u32, old: &[u8], new: &[u8]) -> ContextSize {
    let lines = |bytes: &[u8]| bytes.iter().filter(|byte| **byte == b'\n').count();
    let longest = lines(old).max(lines(new)).saturating_add(1);
    let cap = u32::try_from(longest).unwrap_or(u32::MAX).min(u32::MAX / 4);
    ContextSize::symmetrical(context.min(cap))
}

/// Unified hunks between two byte strings with whitespace counted as
/// `whitespace` says. Returns `None` when either side looks binary.
///
/// Lines that differ only in ignored whitespace are context, shown as they
/// are on the new side, which is what git does.
pub fn diff_bytes_with(
    old: &[u8],
    new: &[u8],
    context: u32,
    whitespace: Whitespace,
) -> Option<Vec<Hunk>> {
    if whitespace == Whitespace::Exact {
        return diff_bytes(old, new, context);
    }
    if looks_binary(old) || looks_binary(new) {
        return None;
    }
    let input = InternedInput::new(
        LooseLines {
            bytes: old,
            whitespace,
        },
        LooseLines {
            bytes: new,
            whitespace,
        },
    );
    let diff = diff_with_slider_heuristics(Algorithm::Myers, &input);
    let unified = UnifiedDiff::new(
        &diff,
        &input,
        Collector::default(),
        bounded_context(context, old, new),
    );
    let mut hunks = unified.consume().unwrap_or_default();
    // Equal lines share one interned text, the first one seen; put back what
    // each line really says.
    let old_lines: Vec<&[u8]> = lines_of(old).collect();
    let new_lines: Vec<&[u8]> = lines_of(new).collect();
    let at = |lines: &[&[u8]], number: Option<u32>| -> Option<(String, bool)> {
        let index = usize::try_from(number?.checked_sub(1)?).ok()?;
        let raw = lines.get(index).copied()?;
        let text = raw.strip_suffix(b"\n").unwrap_or(raw);
        Some((text.to_str_lossy().into_owned(), !raw.ends_with(b"\n")))
    };
    for line in hunks.iter_mut().flat_map(|hunk| &mut hunk.lines) {
        let actual = match line.kind {
            LineKind::Added | LineKind::Context => at(&new_lines, line.new_no),
            LineKind::Removed => at(&old_lines, line.old_no),
        };
        if let Some((text, no_newline)) = actual {
            line.text = text;
            line.no_newline = no_newline;
        }
    }
    Some(hunks)
}

/// Unified hunks between two byte strings. Returns `None` when either side
/// looks binary.
pub fn diff_bytes(old: &[u8], new: &[u8], context: u32) -> Option<Vec<Hunk>> {
    if looks_binary(old) || looks_binary(new) {
        return None;
    }
    let input = InternedInput::new(old, new);
    let diff = diff_with_slider_heuristics(Algorithm::Myers, &input);
    let unified = UnifiedDiff::new(
        &diff,
        &input,
        Collector::default(),
        bounded_context(context, old, new),
    );
    Some(unified.consume().unwrap_or_default())
}

fn count(hunks: &[Hunk]) -> (u32, u32) {
    let mut add = 0u32;
    let mut del = 0u32;
    for hunk in hunks {
        for line in &hunk.lines {
            match line.kind {
                LineKind::Added => add = add.saturating_add(1),
                LineKind::Removed => del = del.saturating_add(1),
                LineKind::Context => {}
            }
        }
    }
    (add, del)
}

/// How many pairs of a removed and an added file are compared for
/// likeness at most, when renames are looked for among files that are not
/// the same byte for byte: 200 removed against 200 added.
///
/// gitoxide's own default of 1000 is this number of pairs (about 31 files
/// against 31), not git's `diff.renameLimit` of 1000 files a side: with
/// it, a directory of forty edited files moved in one commit comes out as
/// forty deletions and forty additions. Git's full million pairs is not
/// asked for either, because a pair costs a line diff here where git
/// compares fingerprints: above this bound renames are found among
/// identical files only, as git does above its own.
const RENAME_PAIRS: usize = 200 * 200;

/// How renames are looked for, everywhere a rename is looked for: files at
/// least half alike, as git has it.
pub(crate) fn rewrites() -> gix::diff::Rewrites {
    gix::diff::Rewrites {
        limit: RENAME_PAIRS,
        ..Default::default()
    }
}

/// A tree entry with content to compare line by line: a file, or a
/// symbolic link, whose content is where it points.
fn has_content(mode: gix::object::tree::EntryMode) -> bool {
    mode.is_blob() || mode.is_link()
}

fn tree_of<'r>(r: &'r gix::Repository, side: &DiffSide) -> Result<gix::Tree<'r>> {
    match side {
        DiffSide::Commit(id) => r
            .find_commit(*id.as_gix())
            .map_err(git_err(say!("diff-loading-commit")))?
            .tree()
            .map_err(git_err(say!("diff-loading-commit-tree"))),
        DiffSide::Tree(id) => r
            .find_tree(*id.as_gix())
            .map_err(git_err(say!("diff-loading-tree"))),
        DiffSide::Empty => Ok(r.empty_tree()),
        DiffSide::Index | DiffSide::Worktree => Err(Error::Invalid {
            what: say!("diff-what-side"),
            detail: say!("diff-side-not-tree").to_owned(),
        }),
    }
}

fn path_selected(paths: &[String], location: &str) -> bool {
    paths.is_empty()
        || paths.iter().any(|p| {
            let p = p.trim_end_matches('/');
            location == p
                || location
                    .strip_prefix(p)
                    .is_some_and(|rest| rest.starts_with('/'))
        })
}

/// How many bytes a blob has, without reading it.
fn blob_size(r: &gix::Repository, id: gix::ObjectId) -> Option<u64> {
    r.find_header(id).ok().map(|header| header.size())
}

fn blob_bytes(r: &gix::Repository, id: gix::ObjectId) -> Result<Vec<u8>> {
    Ok(r.find_blob(id)
        .map_err(git_err(say!("diff-loading-blob")))?
        .data
        .clone())
}

/// Diff two trees (commits, trees or the empty tree). In a partial clone
/// what the diff needs and was left on the remote is fetched
/// ([`crate::promisor`]).
pub fn diff_trees(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    options: &DiffOptions,
) -> Result<Vec<FileDiff>> {
    crate::promisor::fetching(
        repo,
        || diff_trees_here(repo, old, new, options),
        || {
            let paths: Vec<&str> = options.paths.iter().map(String::as_str).collect();
            crate::promisor::fetch_for_diff(repo, old, new, &paths)
        },
    )
}

/// What a commit changed, as `git show` lists it: against its first
/// parent, or against the empty tree for a root and for a commit at the
/// edge of a shallow clone, whose parent is not here and which git takes
/// for a root ([`crate::parents`]). A stash made with untracked files has
/// them in a third parent, and they are listed with what it changes.
pub fn commit_files(repo: &Repository, id: Oid, options: &DiffOptions) -> Result<Vec<FileDiff>> {
    let parents = crate::parents(repo, id);
    // Asked only of a commit with three parents, which few have.
    if parents.len() == 3 && crate::stash::untracked_part(repo, id).is_some() {
        return crate::stash::stash_files(repo, id, options);
    }
    let old = parents
        .first()
        .copied()
        .map(DiffSide::Commit)
        .unwrap_or(DiffSide::Empty);
    diff_trees(repo, &old, &DiffSide::Commit(id), options)
}

/// [`diff_trees`] with what is here.
fn diff_trees_here(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    options: &DiffOptions,
) -> Result<Vec<FileDiff>> {
    let r = repo.gix();
    let old_tree = tree_of(r, old)?;
    let new_tree = tree_of(r, new)?;
    let mut platform = old_tree
        .changes()
        .map_err(git_err(say!("diff-preparing-tree-diff")))?;
    platform.options(|o| {
        o.track_path();
        o.track_rewrites(if options.detect_renames {
            Some(rewrites())
        } else {
            None
        });
    });

    let mut files: Vec<FileDiff> = Vec::new();
    let mut failure: Option<Error> = None;
    platform
        .for_each_to_obtain_tree(&new_tree, |change| {
            use gix::object::tree::diff::Change;
            let location = change.location().to_str_lossy().into_owned();
            if !path_selected(&options.paths, &location) {
                return Ok(std::ops::ControlFlow::Continue(()));
            }
            let (kind, old_path, old_id, new_id, old_mode, new_mode) = match &change {
                Change::Addition { entry_mode, id, .. } => (
                    FileChangeKind::Added,
                    None,
                    None,
                    Some(id.detach()),
                    None,
                    Some(*entry_mode),
                ),
                Change::Deletion { entry_mode, id, .. } => (
                    FileChangeKind::Deleted,
                    None,
                    Some(id.detach()),
                    None,
                    Some(*entry_mode),
                    None,
                ),
                Change::Modification {
                    previous_entry_mode,
                    previous_id,
                    entry_mode,
                    id,
                    ..
                } => {
                    let kind = if mode_class(*previous_entry_mode) != mode_class(*entry_mode) {
                        FileChangeKind::TypeChanged
                    } else {
                        FileChangeKind::Modified
                    };
                    (
                        kind,
                        None,
                        Some(previous_id.detach()),
                        Some(id.detach()),
                        Some(*previous_entry_mode),
                        Some(*entry_mode),
                    )
                }
                Change::Rewrite {
                    source_location,
                    source_id,
                    source_entry_mode,
                    id,
                    entry_mode,
                    copy,
                    ..
                } => (
                    if *copy {
                        FileChangeKind::Copied
                    } else {
                        FileChangeKind::Renamed
                    },
                    Some(source_location.to_str_lossy().into_owned()),
                    Some(source_id.detach()),
                    Some(id.detach()),
                    Some(*source_entry_mode),
                    Some(*entry_mode),
                ),
            };
            if old_mode.is_some_and(|m| m.is_tree()) || new_mode.is_some_and(|m| m.is_tree()) {
                return Ok(std::ops::ControlFlow::Continue(())); // directories are not files
            }
            let mut file = FileDiff {
                kind,
                path: location,
                old_path,
                old_id: old_id.map(Oid::from),
                new_id: new_id.map(Oid::from),
                old_mode: old_mode.map(octal),
                new_mode: new_mode.map(octal),
                binary: false,
                hunks: Vec::new(),
                additions: 0,
                deletions: 0,
                old_size: None,
                new_size: None,
                too_large: false,
                line_endings: FileLineEndings::default(),
            };
            let old_is_blob = old_mode.is_some_and(has_content);
            let new_is_blob = new_mode.is_some_and(has_content);
            // How large the two blobs are, from their headers: a file
            // that is too large to compare is not read at all.
            if old_is_blob {
                file.old_size = old_id.and_then(|id| blob_size(r, id));
            }
            if new_is_blob {
                file.new_size = new_id.and_then(|id| blob_size(r, id));
            }
            let over = |size: Option<u64>| {
                options
                    .max_bytes
                    .is_some_and(|max| size.is_some_and(|size| size > max))
            };
            if (old_is_blob || old_mode.is_none())
                && (new_is_blob || new_mode.is_none())
                && old_id != new_id
                && (over(file.old_size) || over(file.new_size))
            {
                file.too_large = true;
            } else if (old_is_blob || old_mode.is_none())
                && (new_is_blob || new_mode.is_none())
                && old_id != new_id
            {
                let old_bytes = match old_id {
                    Some(id) => blob_bytes(r, id),
                    None => Ok(Vec::new()),
                };
                let new_bytes = match new_id {
                    Some(id) => blob_bytes(r, id),
                    None => Ok(Vec::new()),
                };
                match (old_bytes, new_bytes) {
                    (Ok(old_bytes), Ok(new_bytes)) => {
                        match diff_bytes_with(
                            &old_bytes,
                            &new_bytes,
                            options.context,
                            options.whitespace,
                        ) {
                            Some(hunks) => {
                                let (add, del) = count(&hunks);
                                file.additions = add;
                                file.deletions = del;
                                file.hunks = hunks;
                                file.line_endings =
                                    FileLineEndings::between(&old_bytes, &new_bytes, None);
                            }
                            None => file.binary = true,
                        }
                    }
                    (Err(err), _) | (_, Err(err)) => {
                        failure = Some(err);
                        return Ok(std::ops::ControlFlow::Break(()));
                    }
                }
            }
            files.push(file);
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .map_err(git_err(say!("diff-diffing-trees")))?;
    if let Some(err) = failure {
        return Err(err);
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

/// One side of a file as it is compared: its bytes, its blob id where the
/// side has one, and for a worktree file whose line endings git changes
/// on the way in, how its lines end on disk.
type Compared = (Vec<u8>, Option<Oid>, Option<LineEndings>);

/// [`bytes_at`], to be compared: a worktree file as git would store it
/// (`filters`), so that one checked out with CRLF compares as the LF it is
/// in the repository, as `git diff` compares it. A link is what it says.
fn compared_at(repo: &Repository, side: &DiffSide, path: &str) -> Result<Option<Compared>> {
    let found = bytes_at(repo, side, path)?;
    let link = || {
        repo.workdir()
            .and_then(|workdir| workdir.join(path).symlink_metadata().ok())
            .is_some_and(|meta| meta.is_symlink())
    };
    Ok(match side {
        DiffSide::Worktree if !link() => found.map(|(bytes, id)| {
            let on_disk = LineEndings::of(&bytes);
            let stored = crate::filters::to_git(repo, path, bytes);
            let converted = if LineEndings::of(&stored) == on_disk {
                None
            } else {
                on_disk
            };
            (stored, id, converted)
        }),
        _ => found.map(|(bytes, id)| (bytes, id, None)),
    })
}

fn bytes_at(
    repo: &Repository,
    side: &DiffSide,
    path: &str,
) -> Result<Option<(Vec<u8>, Option<Oid>)>> {
    let r = repo.gix();
    match side {
        DiffSide::Commit(_) | DiffSide::Tree(_) | DiffSide::Empty => {
            let tree = tree_of(r, side)?;
            let Some(entry) = tree
                .lookup_entry_by_path(path)
                .map_err(git_err(say!("diff-looking-up-path")))?
            else {
                return Ok(None);
            };
            if !has_content(entry.mode()) {
                return Ok(None);
            }
            let id = entry.object_id();
            Ok(Some((blob_bytes(r, id)?, Some(Oid::from(id)))))
        }
        DiffSide::Index => {
            let index = r
                .index_or_empty()
                .map_err(git_err(say!("diff-loading-index")))?;
            let Some(entry) = index.entry_by_path(path.as_bytes().as_bstr()) else {
                return Ok(None);
            };
            if entry.mode.contains(gix::index::entry::Mode::COMMIT) {
                return Ok(None);
            }
            let id = entry.id;
            Ok(Some((blob_bytes(r, id)?, Some(Oid::from(id)))))
        }
        DiffSide::Worktree => {
            let Some(workdir) = r.workdir() else {
                return Ok(None);
            };
            let file = workdir.join(path);
            // A symbolic link is what it says, not what it leads to: git
            // keeps where it points as its content, and so it is compared.
            if file.symlink_metadata().is_ok_and(|meta| meta.is_symlink()) {
                return match std::fs::read_link(&file) {
                    Ok(target) => Ok(Some((target.into_os_string().into_encoded_bytes(), None))),
                    Err(err) => Err(Error::Git {
                        context: say!("diff-reading-link"),
                        source: Box::new(err),
                    }),
                };
            }
            // A directory has no content of its own: a submodule is told
            // by the commit it is at (see `gitlink_at`).
            if file.is_dir() {
                return Ok(None);
            }
            match std::fs::read(&file) {
                Ok(bytes) => Ok(Some((bytes, None))),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(err) => Err(Error::Git {
                    context: say!("diff-reading-worktree-file"),
                    source: Box::new(err),
                }),
            }
        }
    }
}

/// The commit a side records for a submodule at `path`; nothing when the
/// path is not a submodule there. In the worktree that is the commit the
/// submodule has checked out, and for one that is not checked out the one
/// the index records, so that it does not read as a change.
fn gitlink_at(repo: &Repository, side: &DiffSide, path: &str) -> Result<Option<Oid>> {
    let r = repo.gix();
    let in_index = || -> Result<Option<Oid>> {
        let index = r
            .index_or_empty()
            .map_err(git_err(say!("diff-loading-index")))?;
        Ok(index
            .entry_by_path(path.as_bytes().as_bstr())
            .filter(|entry| entry.mode.contains(gix::index::entry::Mode::COMMIT))
            .map(|entry| Oid::from(entry.id)))
    };
    match side {
        DiffSide::Commit(_) | DiffSide::Tree(_) | DiffSide::Empty => {
            let tree = tree_of(r, side)?;
            Ok(tree
                .lookup_entry_by_path(path)
                .map_err(git_err(say!("diff-looking-up-path")))?
                .filter(|entry| entry.mode().is_commit())
                .map(|entry| Oid::from(entry.object_id())))
        }
        DiffSide::Index => in_index(),
        DiffSide::Worktree => {
            let Some(recorded) = in_index()? else {
                return Ok(None);
            };
            let dir = crate::stage::worktree_path(repo, path)?;
            if !dir.join(".git").exists() {
                return Ok(dir.is_dir().then_some(recorded));
            }
            let head = gix::open_opts(&dir, crate::system_config::open_options())
                .ok()
                .and_then(|sub| sub.head_id().ok().map(|id| Oid::from(id.detach())));
            Ok(Some(head.unwrap_or(recorded)))
        }
    }
}

/// The content of one path on one side, for showing it as it is (an image,
/// say). `None` when the path is not there or is not a file. Refused when
/// it is larger than `max` bytes, and when the path leaves the worktree.
pub fn file_bytes(
    repo: &Repository,
    side: &DiffSide,
    path: &str,
    max: usize,
) -> Result<Option<Vec<u8>>> {
    let too_large = || Error::Refused {
        what: say!("diff-what-file"),
        detail: say!("diff-too-large", path),
    };
    if matches!(side, DiffSide::Worktree) {
        // Before reading: a worktree file can be of any size.
        let file = crate::stage::worktree_path(repo, path)?;
        match file.symlink_metadata() {
            Ok(meta) if !meta.is_file() => return Ok(None),
            Ok(meta) if meta.len() > u64::try_from(max).unwrap_or(u64::MAX) => {
                return Err(too_large());
            }
            Ok(_) => {}
            Err(_) => return Ok(None),
        }
    } else {
        // What a commit or the index holds is there without a worktree
        // too: a bare repository shows its files like any other.
        crate::stage::check_rel(path)?;
    }
    // In a partial clone the content may be on the remote still: what
    // reads the file whole against nothing fetches it
    // ([`crate::promisor`]).
    let found = crate::promisor::fetching(
        repo,
        || bytes_at(repo, side, path),
        || crate::promisor::fetch_for_diff(repo, &DiffSide::Empty, side, &[path]),
    )?;
    match found {
        Some((bytes, _)) if bytes.len() > max => Err(too_large()),
        Some((bytes, _)) => Ok(Some(bytes)),
        None => Ok(None),
    }
}

/// Diff one path between any two sides, including the index and worktree.
/// Returns `None` when the path exists on neither side.
pub fn diff_file(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    path: &str,
    context: u32,
) -> Result<Option<FileDiff>> {
    diff_file_with(repo, old, new, path, context, Whitespace::Exact)
}

/// [`diff_file`] with whitespace counted as `whitespace` says.
pub fn diff_file_with(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    path: &str,
    context: u32,
    whitespace: Whitespace,
) -> Result<Option<FileDiff>> {
    diff_file_guarded(repo, old, new, None, path, context, whitespace, None)
}

/// How large one path is on one side, and its blob id where the side has
/// one, without reading the file. `None` when the path is no file there.
fn size_at(repo: &Repository, side: &DiffSide, path: &str) -> Result<Option<(u64, Option<Oid>)>> {
    let r = repo.gix();
    match side {
        DiffSide::Commit(_) | DiffSide::Tree(_) | DiffSide::Empty => {
            let tree = tree_of(r, side)?;
            Ok(tree
                .lookup_entry_by_path(path)
                .map_err(git_err(say!("diff-looking-up-path")))?
                .filter(|entry| entry.mode().is_blob())
                .and_then(|entry| {
                    let id = entry.object_id();
                    Some((blob_size(r, id)?, Some(Oid::from(id))))
                }))
        }
        DiffSide::Index => {
            let index = r
                .index_or_empty()
                .map_err(git_err(say!("diff-loading-index")))?;
            Ok(index
                .entry_by_path(path.as_bytes().as_bstr())
                .filter(|entry| !entry.mode.contains(gix::index::entry::Mode::COMMIT))
                .and_then(|entry| Some((blob_size(r, entry.id)?, Some(Oid::from(entry.id))))))
        }
        DiffSide::Worktree => {
            let Some(workdir) = r.workdir() else {
                return Ok(None);
            };
            Ok(workdir
                .join(path)
                .metadata()
                .ok()
                .filter(|meta| meta.is_file())
                .map(|meta| (meta.len(), None)))
        }
    }
}

/// Diff one path between two sides, as [`diff_file_with`] does, with two
/// things more. `old_path` is where the file is on the old side when
/// that is another place (a file that was moved). And with `max_bytes`,
/// a file larger than that on either side is not read: what comes back
/// says how large it is and that it was left alone
/// ([`FileDiff::too_large`]). In a partial clone what the diff needs and
/// was left on the remote is fetched ([`crate::promisor`]).
#[allow(clippy::too_many_arguments)]
pub fn diff_file_guarded(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    old_path: Option<&str>,
    path: &str,
    context: u32,
    whitespace: Whitespace,
    max_bytes: Option<u64>,
) -> Result<Option<FileDiff>> {
    crate::promisor::fetching(
        repo,
        || {
            diff_file_guarded_here(
                repo, old, new, old_path, path, context, whitespace, max_bytes,
            )
        },
        || {
            let mut paths = vec![path];
            paths.extend(old_path.filter(|old_path| *old_path != path));
            crate::promisor::fetch_for_diff(repo, old, new, &paths)
        },
    )
}

/// [`diff_file_guarded`] with what is here.
#[allow(clippy::too_many_arguments)]
fn diff_file_guarded_here(
    repo: &Repository,
    old: &DiffSide,
    new: &DiffSide,
    old_path: Option<&str>,
    path: &str,
    context: u32,
    whitespace: Whitespace,
    max_bytes: Option<u64>,
) -> Result<Option<FileDiff>> {
    // A path is a place in the worktree: one that leads out of it would
    // have a file from anywhere on the machine read as the worktree's.
    crate::stage::check_rel(path)?;
    let moved = old_path.filter(|old_path| *old_path != path);
    let old_path = moved.unwrap_or(path);
    if moved.is_some() {
        crate::stage::check_rel(old_path)?;
    }
    // A submodule on either side: the two commits are all there is to
    // compare here; what lies between them is the submodule's to tell.
    let (old_link, new_link) = (
        gitlink_at(repo, old, old_path)?,
        gitlink_at(repo, new, path)?,
    );
    if old_link.is_some() || new_link.is_some() {
        let mode = |link: Option<Oid>| link.map(|_| "160000".to_owned());
        return Ok(Some(FileDiff {
            kind: match (old_link, new_link) {
                (None, _) => FileChangeKind::Added,
                (_, None) => FileChangeKind::Deleted,
                _ => FileChangeKind::Modified,
            },
            path: path.to_owned(),
            old_path: moved.map(str::to_owned),
            old_id: old_link,
            new_id: new_link,
            old_mode: mode(old_link),
            new_mode: mode(new_link),
            binary: false,
            hunks: Vec::new(),
            additions: 0,
            deletions: 0,
            old_size: None,
            new_size: None,
            too_large: false,
            line_endings: FileLineEndings::default(),
        }));
    }
    let kind_of = |old: bool, new: bool| match (old, new, moved) {
        (true, true, Some(_)) => FileChangeKind::Renamed,
        (true, true, None) => FileChangeKind::Modified,
        (false, _, _) => FileChangeKind::Added,
        (true, false, _) => FileChangeKind::Deleted,
    };
    // Before anything is read: a file that is too large is only measured.
    if let Some(max) = max_bytes {
        let old_size = size_at(repo, old, old_path)?;
        let new_size = size_at(repo, new, path)?;
        let over = |side: &Option<(u64, Option<Oid>)>| side.is_some_and(|(size, _)| size > max);
        if over(&old_size) || over(&new_size) {
            return Ok(Some(FileDiff {
                kind: kind_of(old_size.is_some(), new_size.is_some()),
                path: path.to_owned(),
                old_path: moved.map(str::to_owned),
                old_id: old_size.and_then(|(_, id)| id),
                new_id: new_size.and_then(|(_, id)| id),
                old_mode: None,
                new_mode: None,
                binary: false,
                hunks: Vec::new(),
                additions: 0,
                deletions: 0,
                old_size: old_size.map(|(size, _)| size),
                new_size: new_size.map(|(size, _)| size),
                too_large: true,
                line_endings: FileLineEndings::default(),
            }));
        }
    }
    let old_side = compared_at(repo, old, old_path)?;
    let new_side = compared_at(repo, new, path)?;
    if old_side.is_none() && new_side.is_none() {
        return Ok(None);
    }
    let kind = kind_of(old_side.is_some(), new_side.is_some());
    let (old_there, new_there) = (old_side.is_some(), new_side.is_some());
    let (old_bytes, old_id, old_disk) = old_side.unwrap_or_default();
    let (new_bytes, new_id, new_disk) = new_side.unwrap_or_default();
    let size =
        |there: bool, bytes: &[u8]| there.then(|| u64::try_from(bytes.len()).unwrap_or(u64::MAX));
    let mut file = FileDiff {
        kind,
        path: path.to_owned(),
        old_path: moved.map(str::to_owned),
        old_id,
        new_id,
        old_mode: None,
        new_mode: None,
        binary: false,
        hunks: Vec::new(),
        additions: 0,
        deletions: 0,
        old_size: size(old_there, &old_bytes),
        new_size: size(new_there, &new_bytes),
        too_large: false,
        line_endings: FileLineEndings::default(),
    };
    match diff_bytes_with(&old_bytes, &new_bytes, context, whitespace) {
        Some(hunks) => {
            let (add, del) = count(&hunks);
            file.additions = add;
            file.deletions = del;
            file.hunks = hunks;
            file.line_endings =
                FileLineEndings::between(&old_bytes, &new_bytes, old_disk.or(new_disk));
        }
        None => {
            file.binary = true;
            // A binary file is told by what it is, not by its lines: the
            // id of a file of the worktree is the one git would give its
            // bytes as they are.
            if old_there && file.old_id.is_none() {
                file.old_id = crate::stage::blob_id(&old_bytes);
            }
            if new_there && file.new_id.is_none() {
                file.new_id = crate::stage::blob_id(&new_bytes);
            }
        }
    }
    Ok(Some(file))
}

/// Apply hunks to the old text to reproduce the new text: how the tests
/// prove hunks are complete.
pub fn apply_hunks(old: &str, hunks: &[Hunk]) -> Option<String> {
    let old_lines: Vec<&str> = old.split_inclusive('\n').collect();
    let mut out = String::new();
    let mut cursor = 0usize; // 0-based index into old_lines
    for hunk in hunks {
        let start = usize::try_from(hunk.old_start).ok()?.saturating_sub(1);
        let start = if hunk.old_lines == 0 {
            usize::try_from(hunk.old_start).ok()?
        } else {
            start
        };
        if start < cursor {
            return None;
        }
        for line in old_lines.get(cursor..start)? {
            out.push_str(line);
        }
        cursor = start;
        for line in &hunk.lines {
            match line.kind {
                LineKind::Context | LineKind::Removed => {
                    let actual = old_lines.get(cursor)?;
                    let text = actual.strip_suffix('\n').unwrap_or(actual);
                    if text != line.text {
                        return None;
                    }
                    if line.kind == LineKind::Context {
                        out.push_str(actual);
                    }
                    cursor = cursor.saturating_add(1);
                }
                LineKind::Added => {
                    out.push_str(&line.text);
                    if !line.no_newline {
                        out.push('\n');
                    }
                }
            }
        }
    }
    for line in old_lines.get(cursor..)? {
        out.push_str(line);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hunks_reproduce_the_new_text() {
        let old = "a\nb\nc\nd\ne\nf\ng\n";
        let new = "a\nB\nc\nd\ne\nf\ng\nh\n";
        let hunks = diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
        assert_eq!(apply_hunks(old, &hunks).as_deref(), Some(new));
        let (add, del) = count(&hunks);
        assert_eq!((add, del), (2, 1));
    }

    #[test]
    fn no_trailing_newline_is_preserved() {
        let old = "one\ntwo";
        let new = "one\ntwo\nthree";
        let hunks = diff_bytes(old.as_bytes(), new.as_bytes(), 3).unwrap();
        assert_eq!(apply_hunks(old, &hunks).as_deref(), Some(new));
    }

    #[test]
    fn binary_is_detected() {
        assert!(diff_bytes(b"\0\x01", b"text", 3).is_none());
        assert!(looks_binary(b"abc\0def"));
        assert!(!looks_binary(b"plain text\n"));
    }

    #[test]
    fn identical_input_has_no_hunks() {
        assert!(diff_bytes(b"same\n", b"same\n", 3).unwrap().is_empty());
    }

    #[test]
    fn line_endings_are_told_by_what_ends_the_lines() {
        assert_eq!(LineEndings::of(b"a\nb\n"), Some(LineEndings::Lf));
        assert_eq!(LineEndings::of(b"a\r\nb"), Some(LineEndings::Crlf));
        assert_eq!(LineEndings::of(b"a\r\nb\n"), Some(LineEndings::Mixed));
        assert_eq!(LineEndings::of(b"a\nb\r\n"), Some(LineEndings::Mixed));
        // No line ends: one line, nothing, a CR alone.
        assert_eq!(LineEndings::of(b"one line"), None);
        assert_eq!(LineEndings::of(b""), None);
        assert_eq!(LineEndings::of(b"a\rb\r"), None);
        assert_eq!(LineEndings::of(b"\r\n"), Some(LineEndings::Crlf));
        assert_eq!(LineEndings::of(b"\n\r"), Some(LineEndings::Lf));
    }

    #[test]
    fn a_change_of_line_endings_alone_is_told_apart() {
        let only = only_line_endings_differ;
        assert!(only(b"a\nb\n", b"a\r\nb\r\n"));
        assert!(only(b"a\r\nb\r\n", b"a\nb\n"));
        assert!(only(b"a\nb\n", b"a\r\nb\n"));
        // A line feed added or taken away at the end, as git passes over.
        assert!(only(b"a\nb", b"a\nb\n"));
        assert!(only(b"a\nb\r\n", b"a\nb"));
        // The same, a real change, a line more, binary.
        assert!(!only(b"a\r\n", b"a\r\n"));
        assert!(!only(b"a\nb\n", b"a\r\nc\r\n"));
        assert!(!only(b"a\n", b"a\n\n"));
        assert!(!only(b"a\0\n", b"a\0\r\n"));
        // A CR at the end of a last line without a line feed counts.
        assert!(!only(b"a\nb\r", b"a\nb"));
        // What the whitespace option says agrees: no hunks.
        for (old, new) in [(&b"a\nb\n"[..], &b"a\r\nb\r\n"[..]), (b"a\nb", b"a\nb\n")] {
            let hunks = diff_bytes_with(old, new, 3, Whitespace::IgnoreCrAtEol).unwrap();
            assert!(hunks.is_empty(), "{hunks:?}");
            assert!(!diff_bytes(old, new, 3).unwrap().is_empty());
        }
        let ending = FileLineEndings::between(b"a\nb\n", b"a\r\nb\r\n", None);
        assert_eq!(
            ending,
            FileLineEndings {
                old: Some(LineEndings::Lf),
                new: Some(LineEndings::Crlf),
                worktree: None,
                only_change: true,
            }
        );
    }

    #[test]
    fn lines_that_differ_in_a_cr_alone_are_context_when_it_does_not_count() {
        let old = b"one\ntwo\nthree\n";
        let new = b"one\r\nTWO\r\nthree\r\n";
        let hunks = diff_bytes_with(old, new, 3, Whitespace::IgnoreCrAtEol).unwrap();
        assert_eq!(count(&hunks), (1, 1));
        let lines = &hunks[0].lines;
        // Shown as the new side has them, CR and all.
        assert_eq!(lines[0].kind, LineKind::Context);
        assert_eq!(lines[0].text, "one\r");
        assert!(
            lines
                .iter()
                .any(|l| l.kind == LineKind::Added && l.text == "TWO\r")
        );
        assert!(
            lines
                .iter()
                .any(|l| l.kind == LineKind::Removed && l.text == "two")
        );
    }

    #[test]
    fn path_filter_matches_files_and_directories() {
        let paths = vec!["src".to_owned(), "README.md".to_owned()];
        assert!(path_selected(&paths, "src/main.rs"));
        assert!(path_selected(&paths, "README.md"));
        assert!(!path_selected(&paths, "srcs/x"));
        assert!(path_selected(&[], "anything"));
    }
}
