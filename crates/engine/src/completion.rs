//! Completing a git command typed in the terminal: from the line typed so
//! far, the word being typed and what may take its place, best first.
//!
//! The line is split into words the way a POSIX shell splits it (blanks,
//! quotes, a backslash), and only its last command counts (after `;`,
//! `&&`, `|`). It is completed when that command is `git`: git's
//! commands and their options from [`crate::completion_table`], and the
//! repository's names from [`Words`], which the engine reads
//! (`terminal.completion_words`): branches, remote branches, tags,
//! remotes, stashes, and paths, the changed ones first. Nothing is run
//! to complete a line: not the shell's own completion, nothing of the
//! repository's.
//!
//! What is typed is matched against each name: names that begin with it
//! come first, then those with a part (after `/`, `-`, `_` or `.`) that
//! begins with it, then (paths and the repository's names) those that
//! hold it, then (the repository's names) those that hold its letters in
//! order; within each, the order the names are offered in. Git's own
//! commands and options only match from where they or a part begin.
//!
//! Sources: POSIX Shell Command Language, 2.2 Quoting and 2.3 Token
//! Recognition; git(1) and the manual pages of the commands in the table.

use std::collections::BTreeSet;
use std::path::Path;

use bitbraid_terminal::{ShellKind, quote_path};
use serde::{Deserialize, Serialize};

use crate::completion_table::{
    self as table, Args, COMMANDS, Command, ELSEWHERE, GLOBAL_OPTIONS, Opt, PathSet, Value,
};

/// The most candidates one answer holds.
pub const MOST_CANDIDATES: usize = 200;
/// The longest line completed, in characters.
pub const LONGEST_LINE: usize = 4096;

/// How a path differs from the last commit or the index.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PathChange {
    /// A new file.
    Added,
    /// Its content changed.
    Modified,
    /// It is gone.
    Deleted,
    /// It was moved.
    Renamed,
    /// It was copied.
    Copied,
    /// It became another kind of thing (a link, a submodule).
    TypeChanged,
    /// Git does not track it.
    Untracked,
    /// It is noted to be added.
    IntentToAdd,
    /// It is in conflict.
    Conflicted,
}

impl PathChange {
    /// The words the list says it in.
    pub fn words(self) -> &'static str {
        match self {
            Self::Added => say!("completion-change-added"),
            Self::Modified => say!("completion-change-modified"),
            Self::Deleted => say!("completion-change-deleted"),
            Self::Renamed => say!("completion-change-renamed"),
            Self::Copied => say!("completion-change-copied"),
            Self::TypeChanged => say!("completion-change-type-changed"),
            Self::Untracked => say!("completion-change-untracked"),
            Self::IntentToAdd => say!("completion-change-intent-to-add"),
            Self::Conflicted => say!("completion-change-conflicted"),
        }
    }
}

/// A path that differs, and how.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedPath {
    /// The path, from the top of the worktree.
    pub path: String,
    /// How it differs from the index: what can be staged.
    pub unstaged: Option<PathChange>,
    /// How the index differs from the last commit: what is staged.
    pub staged: Option<PathChange>,
}

/// A remote and its branches.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct RemoteWords {
    pub name: String,
    /// Where it is fetched from.
    pub url: Option<String>,
    /// Its branches, without its name in front.
    pub branches: Vec<String>,
}

/// A stash.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashWord {
    /// Its place in the list, newest first.
    pub index: usize,
    /// What it says it holds.
    pub message: String,
}

/// The repository's names that a command line is completed with.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Words {
    /// The top of the worktree the paths are from.
    pub top: String,
    /// The branch checked out.
    pub head: Option<String>,
    /// Local branches.
    pub branches: Vec<String>,
    /// Remotes with their branches.
    pub remotes: Vec<RemoteWords>,
    pub tags: Vec<String>,
    /// Stashes, newest first.
    pub stashes: Vec<StashWord>,
    /// Paths that differ from the last commit or the index.
    pub changes: Vec<ChangedPath>,
    /// Paths the index tracks.
    pub tracked: Vec<String>,
    /// Whether it tracks more than are listed.
    pub more_tracked: bool,
}

/// Where a line is completed.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Place {
    /// The folder the shell is in, from the top of the worktree (`""` at
    /// the top, `/` between names); none when it is outside the
    /// worktree, and then only git's own words are offered.
    pub folder: Option<String>,
    /// How the shell reads quoted text.
    pub shell: ShellKind,
}

/// What a candidate is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateKind {
    /// A command of git's.
    Command,
    /// A word of a command's own (`stash pop`).
    Word,
    Option,
    /// A local branch.
    Branch,
    /// A branch of a remote.
    RemoteBranch,
    Tag,
    Remote,
    Stash,
    File,
    Folder,
}

/// What may take the place of the word being typed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Candidate {
    /// What the list shows.
    pub label: String,
    /// What takes the place of the word typed, written as the shell
    /// reads it.
    pub insert: String,
    pub kind: CandidateKind,
    /// What the list says of it.
    pub detail: String,
}

/// The word being typed and what may take its place.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Completion {
    /// Where the word begins in the line, in characters.
    pub start: usize,
    /// The word as typed, quotes and all.
    pub word: String,
    /// What may take its place, best first.
    pub candidates: Vec<Candidate>,
}

/// A word of a command line.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct Word {
    /// Where it begins, in characters.
    start: usize,
    /// As typed.
    raw: String,
    /// With the quoting taken off.
    value: String,
}

/// The words of the last command of a line, the last one being the word
/// the cursor is in (empty, after the line's end, when the line ends in a
/// blank; begun with a quote that is still open, it is completed all the
/// same). None when the line cannot be completed: `$` or a backquote in
/// the word being typed, a redirection.
fn words_of(line: &str) -> Option<Vec<Word>> {
    let mut words: Vec<Word> = Vec::new();
    let mut current: Option<Word> = None;
    let mut quote: Option<char> = None;
    let mut escaped = false;
    let mut redirected = false;
    let mut count = 0usize;
    for (at, c) in line.chars().enumerate() {
        count = at.saturating_add(1);
        if escaped {
            escaped = false;
            let word = current.get_or_insert_with(|| Word {
                start: at.saturating_sub(1),
                ..Word::default()
            });
            word.raw.push(c);
            word.value.push(c);
            continue;
        }
        if let Some(open) = quote {
            let word = current.get_or_insert_with(Word::default);
            word.raw.push(c);
            if c == open {
                quote = None;
            } else if open == '"' && c == '\\' {
                escaped = true;
            } else {
                word.value.push(c);
            }
            continue;
        }
        match c {
            '\\' => {
                let word = current.get_or_insert_with(|| Word {
                    start: at,
                    ..Word::default()
                });
                word.raw.push(c);
                escaped = true;
            }
            '\'' | '"' => {
                let word = current.get_or_insert_with(|| Word {
                    start: at,
                    ..Word::default()
                });
                word.raw.push(c);
                quote = Some(c);
            }
            ';' | '|' | '&' | '(' | ')' | '\n' => {
                current = None;
                words.clear();
                redirected = false;
            }
            '<' | '>' => {
                current = None;
                redirected = true;
            }
            c if c.is_whitespace() => {
                if let Some(word) = current.take() {
                    words.push(word);
                }
            }
            c => {
                let word = current.get_or_insert_with(|| Word {
                    start: at,
                    ..Word::default()
                });
                word.raw.push(c);
                word.value.push(c);
            }
        }
    }
    if redirected {
        return None;
    }
    let last = current.unwrap_or(Word {
        start: count,
        ..Word::default()
    });
    // A variable or a command's output: what it stands for is not known.
    if last.raw.contains(['$', '`']) {
        return None;
    }
    words.push(last);
    Some(words)
}

/// How far a name may be from what is typed and still match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Loose {
    /// It or a part of it begins with it: git's own words.
    Begins,
    /// It holds it: paths.
    Holds,
    /// It holds its letters in order: names of the repository.
    Letters,
}

impl Loose {
    fn of(kind: CandidateKind) -> Self {
        match kind {
            CandidateKind::Command | CandidateKind::Word | CandidateKind::Option => Self::Begins,
            CandidateKind::File | CandidateKind::Folder => Self::Holds,
            CandidateKind::Branch
            | CandidateKind::RemoteBranch
            | CandidateKind::Tag
            | CandidateKind::Remote
            | CandidateKind::Stash => Self::Letters,
        }
    }
}

/// How well a name matches what is typed: 0 is best; none when it does
/// not, or only more loosely than `loose` allows. Upper and lower case
/// count alike unless an upper case letter is typed.
fn rank(name: &str, typed: &str, loose: Loose) -> Option<u8> {
    if typed.is_empty() || name.starts_with(typed) {
        return Some(0);
    }
    let alike = !typed.chars().any(char::is_uppercase);
    let fold = |text: &str| {
        if alike {
            text.to_lowercase()
        } else {
            text.to_owned()
        }
    };
    let name = fold(name);
    let typed = fold(typed);
    if alike && name.starts_with(&typed) {
        return Some(1);
    }
    let parts = name
        .char_indices()
        .filter(|(at, c)| *at > 0 && matches!(c, '/' | '-' | '_' | '.'))
        .filter_map(|(at, c)| name.get(at.saturating_add(c.len_utf8())..));
    for part in parts {
        if part.starts_with(&typed) {
            return Some(2);
        }
    }
    if loose >= Loose::Holds && name.contains(&typed) {
        return Some(3);
    }
    if loose >= Loose::Letters {
        let mut letters = name.chars();
        if typed.chars().all(|wanted| letters.any(|c| c == wanted)) {
            return Some(4);
        }
    }
    None
}

/// Candidates as they are gathered: each with how well it matched.
#[derive(Default)]
struct Gathered {
    found: Vec<(u8, Candidate)>,
    labels: BTreeSet<(CandidateKind, String)>,
}

impl Gathered {
    fn offer(
        &mut self,
        typed: &str,
        label: &str,
        insert: String,
        kind: CandidateKind,
        detail: &str,
    ) {
        let Some(tier) = rank(label, typed, Loose::of(kind)) else {
            return;
        };
        if !self.labels.insert((kind, label.to_owned())) {
            return;
        }
        self.found.push((
            tier,
            Candidate {
                label: label.to_owned(),
                insert,
                kind,
                detail: detail.to_owned(),
            },
        ));
    }

    fn done(mut self) -> Vec<Candidate> {
        // Stable: within a tier the order they were offered in stays.
        self.found.sort_by_key(|(tier, _)| *tier);
        self.found
            .into_iter()
            .take(MOST_CANDIDATES)
            .map(|(_, candidate)| candidate)
            .collect()
    }
}

/// A name written as the shell reads it.
fn quoted(name: &str, shell: ShellKind) -> String {
    quote_path(Path::new(name), shell)
}

/// What the line around the word being typed says.
struct Line<'a> {
    words: &'a [Word],
    place: &'a Place,
    repo: Option<&'a Words>,
}

/// Complete `line`, the text typed before the cursor: none when it is not
/// a git command, or the word being typed is not one that is completed.
pub fn complete(line: &str, words: &Words, place: &Place) -> Option<Completion> {
    if line.chars().count() > LONGEST_LINE {
        return None;
    }
    let all = words_of(line)?;
    let (typed, before) = all.split_last()?;
    // Settings of the shell's own before the command (`A=1 git ...`).
    let mut rest = before
        .iter()
        .skip_while(|word| is_assignment(&word.value))
        .peekable();
    let program = rest.next()?;
    if program.value != "git" && !program.value.ends_with("/git") {
        return None;
    }
    let after: Vec<Word> = rest.cloned().collect();
    let elsewhere = after.iter().any(|word| {
        ELSEWHERE
            .iter()
            .any(|option| word.value == *option || word.value.starts_with(&format!("{option}=")))
    });
    let line_words = Line {
        words: &after,
        place,
        repo: (!elsewhere && place.folder.is_some()).then_some(words),
    };
    let candidates = line_words.candidates(typed);
    Some(Completion {
        start: typed.start,
        word: typed.raw.clone(),
        candidates,
    })
}

fn is_assignment(word: &str) -> bool {
    word.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty()
            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            && !name.starts_with(|c: char| c.is_ascii_digit())
    })
}

impl Line<'_> {
    /// The candidates for the word being typed, after git and the words
    /// before it.
    fn candidates(&self, typed: &Word) -> Vec<Candidate> {
        let mut gathered = Gathered::default();
        // Git's own options first, until its command.
        let mut at = 0usize;
        while let Some(word) = self.words.get(at) {
            if !word.value.starts_with('-') {
                break;
            }
            let takes_value = table::option_named(GLOBAL_OPTIONS, &word.value)
                .is_some_and(|option| option.value != Value::None);
            at = at.saturating_add(if takes_value { 2 } else { 1 });
        }
        let Some(name) = self.words.get(at) else {
            // The command is being typed, or a value of git's own option.
            if let Some(option) = self
                .words
                .last()
                .and_then(|last| table::option_named(GLOBAL_OPTIONS, &last.value))
                .filter(|option| option.value != Value::None)
            {
                self.values(option.value, typed, &mut gathered);
            } else if typed.value.starts_with('-') {
                options(GLOBAL_OPTIONS, typed, &mut gathered);
            } else {
                for command in COMMANDS {
                    gathered.offer(
                        &typed.value,
                        command.name,
                        command.name.to_owned(),
                        CandidateKind::Command,
                        command.detail.text(),
                    );
                }
            }
            return gathered.done();
        };
        let Some(command) = table::command_named(&name.value) else {
            return Vec::new();
        };
        let after = self.words.get(at.saturating_add(1)..).unwrap_or_default();
        self.after_command(command, after, typed, &mut gathered);
        gathered.done()
    }

    /// The candidates after a command (or a word of one), with the words
    /// typed after it.
    fn after_command(
        &self,
        command: &'static Command,
        after: &[Word],
        typed: &Word,
        gathered: &mut Gathered,
    ) {
        // Its words of its own come first: `stash pop`, `remote add`.
        if let Args::Words(own) = command.args {
            let first = after.iter().position(|word| !word.value.starts_with('-'));
            match first.and_then(|at| after.get(at).map(|word| (at, word))) {
                Some((at, word)) => {
                    if let Some(sub) = own.iter().find(|sub| sub.name == word.value) {
                        let rest = after.get(at.saturating_add(1)..).unwrap_or_default();
                        self.after_command(sub, rest, typed, gathered);
                    }
                }
                None => {
                    if typed.value.starts_with('-') {
                        options(command.options, typed, gathered);
                    } else {
                        for sub in own {
                            gathered.offer(
                                &typed.value,
                                sub.name,
                                sub.name.to_owned(),
                                CandidateKind::Word,
                                sub.detail.text(),
                            );
                        }
                    }
                }
            }
            return;
        }
        // What the words typed so far are: options, their values, the
        // rest in order; and whether `--` came.
        let mut positional = 0usize;
        let mut paths_only = false;
        let mut value_of: Option<&'static Opt> = None;
        let mut flags: Vec<&str> = Vec::new();
        for word in after {
            if value_of.take().is_some() {
                continue;
            }
            if paths_only {
                positional = positional.saturating_add(1);
                continue;
            }
            if word.value == "--" {
                paths_only = true;
                continue;
            }
            if word.value.starts_with('-') && word.value.len() > 1 {
                flags.push(&word.value);
                if !word.value.contains('=')
                    && let Some(option) = table::option_named(command.options, &word.value)
                    && option.value != Value::None
                {
                    value_of = Some(option);
                }
                continue;
            }
            positional = positional.saturating_add(1);
        }
        // The value of the option before it.
        if let Some(option) = value_of {
            self.values(option.value, typed, gathered);
            return;
        }
        // `--option=value`.
        if !paths_only
            && let Some((name, value)) = typed.value.split_once('=')
            && let Some(option) = table::option_named(command.options, name)
        {
            let given = Word {
                start: typed.start,
                raw: value.to_owned(),
                value: value.to_owned(),
            };
            let mut inner = Gathered::default();
            self.values(option.value, &given, &mut inner);
            for candidate in inner.done() {
                gathered.offer(
                    value,
                    &candidate.label,
                    format!("{name}={}", candidate.insert),
                    candidate.kind,
                    &candidate.detail,
                );
            }
            return;
        }
        if !paths_only && typed.value.starts_with('-') {
            options(command.options, typed, gathered);
            return;
        }
        let args = if paths_only {
            match command.args {
                Args::Paths(set) => Args::Paths(set),
                _ => Args::Paths(PathSet::Tracked),
            }
        } else {
            override_by_flags(command.name, command.args, &flags)
        };
        // Before anything is typed, the options come first.
        if typed.raw.is_empty() && !paths_only {
            options(command.options, typed, gathered);
        }
        self.arguments(args, positional, typed, gathered);
    }

    /// The candidates for the `positional`-th word after a command.
    fn arguments(&self, args: Args, positional: usize, typed: &Word, gathered: &mut Gathered) {
        match args {
            Args::Nothing | Args::Words(_) => {}
            Args::Refs => self.refs(typed, gathered),
            // Paths only after `--`, which is handled before.
            Args::RefsThenPaths => self.refs(typed, gathered),
            Args::Branches => self.branches(typed, gathered),
            Args::Switch => self.switch(typed, gathered),
            Args::NewThenRefs => {
                if positional >= 1 {
                    self.refs(typed, gathered);
                }
            }
            Args::Tags => self.tags(typed, gathered),
            Args::Remotes => self.remotes(typed, gathered),
            Args::RemoteThenBranches => {
                if positional == 0 {
                    self.remotes(typed, gathered);
                } else {
                    self.branches(typed, gathered);
                }
            }
            Args::RemoteThenTheirs => {
                if positional == 0 {
                    self.remotes(typed, gathered);
                } else {
                    self.theirs(typed, gathered);
                }
            }
            Args::Stashes => self.stashes(typed, gathered),
            Args::Paths(set) => self.paths(set, typed, gathered),
            Args::Commands => {
                for command in COMMANDS {
                    gathered.offer(
                        &typed.value,
                        command.name,
                        command.name.to_owned(),
                        CandidateKind::Command,
                        command.detail.text(),
                    );
                }
            }
        }
    }

    /// What an option's value is completed with.
    fn values(&self, value: Value, typed: &Word, gathered: &mut Gathered) {
        match value {
            Value::None | Value::Text => {}
            Value::Ref => self.refs(typed, gathered),
            Value::Path => {
                if self.repo.is_some() {
                    self.paths(PathSet::Tracked, typed, gathered);
                }
            }
            Value::Remote => self.remotes(typed, gathered),
        }
    }

    fn shell(&self) -> ShellKind {
        self.place.shell
    }

    fn branches(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        for branch in &repo.branches {
            let detail = if repo.head.as_deref() == Some(branch.as_str()) {
                say!("completion-detail-checked-out")
            } else {
                say!("completion-detail-local")
            };
            gathered.offer(
                &typed.value,
                branch,
                quoted(branch, self.shell()),
                CandidateKind::Branch,
                detail,
            );
        }
    }

    fn remote_branches(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        for remote in &repo.remotes {
            for branch in remote.branches.iter().filter(|name| *name != "HEAD") {
                let name = format!("{}/{branch}", remote.name);
                gathered.offer(
                    &typed.value,
                    &name,
                    quoted(&name, self.shell()),
                    CandidateKind::RemoteBranch,
                    &remote.name,
                );
            }
        }
    }

    fn tags(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        for tag in &repo.tags {
            gathered.offer(
                &typed.value,
                tag,
                quoted(tag, self.shell()),
                CandidateKind::Tag,
                say!("completion-detail-tag"),
            );
        }
    }

    fn refs(&self, typed: &Word, gathered: &mut Gathered) {
        self.branches(typed, gathered);
        self.remote_branches(typed, gathered);
        self.tags(typed, gathered);
    }

    /// Local branches, then the remote ones by the name a local branch
    /// made from them gets.
    fn switch(&self, typed: &Word, gathered: &mut Gathered) {
        self.branches(typed, gathered);
        let Some(repo) = self.repo else {
            return;
        };
        for remote in &repo.remotes {
            for branch in remote.branches.iter().filter(|name| *name != "HEAD") {
                if repo.branches.contains(branch) {
                    continue;
                }
                gathered.offer(
                    &typed.value,
                    branch,
                    quoted(branch, self.shell()),
                    CandidateKind::RemoteBranch,
                    &remote.name,
                );
            }
        }
    }

    fn remotes(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        for remote in &repo.remotes {
            gathered.offer(
                &typed.value,
                &remote.name,
                quoted(&remote.name, self.shell()),
                CandidateKind::Remote,
                remote
                    .url
                    .as_deref()
                    .unwrap_or_else(|| say!("completion-detail-remote")),
            );
        }
    }

    /// The branches of the remote named before the word being typed.
    fn theirs(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        let named = self
            .words
            .iter()
            .rev()
            .find_map(|word| repo.remotes.iter().find(|remote| remote.name == word.value));
        let Some(remote) = named else {
            return;
        };
        for branch in remote.branches.iter().filter(|name| *name != "HEAD") {
            gathered.offer(
                &typed.value,
                branch,
                quoted(branch, self.shell()),
                CandidateKind::RemoteBranch,
                &remote.name,
            );
        }
    }

    fn stashes(&self, typed: &Word, gathered: &mut Gathered) {
        let Some(repo) = self.repo else {
            return;
        };
        for stash in &repo.stashes {
            let name = format!("stash@{{{}}}", stash.index);
            gathered.offer(
                &typed.value,
                &name,
                quoted(&name, self.shell()),
                CandidateKind::Stash,
                &stash.message,
            );
        }
    }

    /// Paths below the shell's folder, from it: the changed ones of the
    /// set first, then their folders, then (for the tracked set) every
    /// tracked one.
    fn paths(&self, set: PathSet, typed: &Word, gathered: &mut Gathered) {
        let (Some(repo), Some(folder)) = (self.repo, self.place.folder.as_deref()) else {
            return;
        };
        let base = if folder.is_empty() {
            String::new()
        } else {
            format!("{}/", folder.trim_end_matches('/'))
        };
        let (dot, typed_value) = match typed.value.strip_prefix("./") {
            Some(rest) => ("./", rest),
            None => ("", typed.value.as_str()),
        };
        let shell = self.shell();
        let offer = |gathered: &mut Gathered, path: &str, kind: CandidateKind, detail: &str| {
            let Some(relative) = path.strip_prefix(base.as_str()) else {
                return;
            };
            if relative.is_empty() {
                return;
            }
            let mut insert = format!("{dot}{}", quoted(relative.trim_end_matches('/'), shell));
            if kind == CandidateKind::Folder {
                insert.push('/');
            }
            gathered.offer(typed_value, relative, insert, kind, detail);
        };
        let changed: Vec<(&str, String)> = repo
            .changes
            .iter()
            .filter_map(|change| {
                let detail = match set {
                    PathSet::Unstaged => change.unstaged?.words().to_owned(),
                    PathSet::Staged => {
                        say!("completion-change-staged", change = change.staged?.words())
                    }
                    PathSet::Untracked => {
                        (change.unstaged? == PathChange::Untracked).then_some(())?;
                        PathChange::Untracked.words().to_owned()
                    }
                    PathSet::Tracked => match (change.unstaged, change.staged) {
                        (Some(kind), _) => kind.words().to_owned(),
                        (None, Some(kind)) => {
                            say!("completion-change-staged", change = kind.words())
                        }
                        (None, None) => return None,
                    },
                    PathSet::Folders => return None,
                };
                Some((change.path.as_str(), detail))
            })
            .collect();
        for (path, detail) in &changed {
            offer(gathered, path, CandidateKind::File, detail);
        }
        // The folders the paths offered are in, below the shell's.
        let mut folders: BTreeSet<String> = BTreeSet::new();
        let sources: Box<dyn Iterator<Item = &str>> = match set {
            PathSet::Tracked | PathSet::Folders => {
                Box::new(repo.tracked.iter().map(String::as_str))
            }
            _ => Box::new(changed.iter().map(|(path, _)| *path)),
        };
        for path in sources {
            let mut end = 0usize;
            while let Some(slash) = path.get(end..).and_then(|rest| rest.find('/')) {
                end = end.saturating_add(slash);
                if let Some(folder) = path.get(..end) {
                    folders.insert(format!("{folder}/"));
                }
                end = end.saturating_add(1);
            }
        }
        for folder in &folders {
            offer(
                gathered,
                folder,
                CandidateKind::Folder,
                say!("completion-detail-folder"),
            );
        }
        if set == PathSet::Tracked {
            for path in &repo.tracked {
                offer(
                    gathered,
                    path,
                    CandidateKind::File,
                    say!("completion-detail-tracked"),
                );
            }
        }
    }
}

/// A command's options that match what is typed.
fn options(options: &'static [Opt], typed: &Word, gathered: &mut Gathered) {
    for option in options {
        gathered.offer(
            &typed.value,
            option.name,
            option.name.to_owned(),
            CandidateKind::Option,
            option.detail.text(),
        );
    }
}

/// What a command's words are once these flags were given: `branch -d`
/// deletes branches, `tag -d` tags, `restore --staged` unstages.
fn override_by_flags(command: &str, args: Args, flags: &[&str]) -> Args {
    let given = |names: &[&str]| flags.iter().any(|flag| names.contains(flag));
    match command {
        "branch"
            if given(&[
                "-d", "-D", "--delete", "-m", "-M", "--move", "-c", "-C", "--copy",
            ]) =>
        {
            Args::Branches
        }
        "tag" if given(&["-d", "--delete", "-v", "--verify"]) => Args::Tags,
        "restore" if given(&["--staged", "-S"]) => Args::Paths(PathSet::Staged),
        _ => args,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn words() -> Words {
        Words {
            top: "/repo".into(),
            head: Some("main".into()),
            branches: vec![
                "develop".into(),
                "feature/login".into(),
                "fix/crash".into(),
                "main".into(),
            ],
            remotes: vec![RemoteWords {
                name: "origin".into(),
                url: Some("https://example.invalid/align.git".into()),
                branches: vec!["HEAD".into(), "main".into(), "remote-only".into()],
            }],
            tags: vec!["v0.1".into(), "v1.0".into()],
            stashes: vec![StashWord {
                index: 0,
                message: "WIP on main: notes".into(),
            }],
            changes: vec![
                ChangedPath {
                    path: "README.md".into(),
                    unstaged: Some(PathChange::Modified),
                    staged: None,
                },
                ChangedPath {
                    path: "src/new.rs".into(),
                    unstaged: Some(PathChange::Untracked),
                    staged: None,
                },
                ChangedPath {
                    path: "docs/guide.md".into(),
                    unstaged: None,
                    staged: Some(PathChange::Added),
                },
            ],
            tracked: vec![
                "README.md".into(),
                "docs/intro.md".into(),
                "src/main.rs".into(),
                "src/ui/view.rs".into(),
            ],
            more_tracked: false,
        }
    }

    fn top() -> Place {
        Place {
            folder: Some(String::new()),
            shell: ShellKind::Posix,
        }
    }

    fn labels(line: &str) -> Vec<String> {
        labels_in(line, &top())
    }

    fn labels_in(line: &str, place: &Place) -> Vec<String> {
        complete(line, &words(), place)
            .unwrap_or_else(|| panic!("{line:?} is completed"))
            .candidates
            .into_iter()
            .map(|candidate| candidate.label)
            .collect()
    }

    #[test]
    fn lines_are_split_as_a_shell_splits_them() {
        let words = words_of("echo 'a b' \"c\\\"d\" e\\ f g").unwrap();
        let values: Vec<&str> = words.iter().map(|word| word.value.as_str()).collect();
        assert_eq!(values, ["echo", "a b", "c\"d", "e f", "g"]);
        assert_eq!(words.last().unwrap().start, 23);
        // The last command only.
        let words = words_of("make && git ch").unwrap();
        assert_eq!(words.len(), 2);
        assert_eq!(words[1].raw, "ch");
        assert_eq!(words[1].start, 12);
        // A blank at the end: a new word.
        let words = words_of("git ").unwrap();
        assert_eq!(
            words.last().unwrap(),
            &Word {
                start: 4,
                ..Word::default()
            }
        );
        assert_eq!(words_of("git log > out"), None);
        assert_eq!(words_of("git checkout $BR"), None);
    }

    #[test]
    fn only_git_is_completed() {
        assert_eq!(complete("ls -l", &words(), &top()), None);
        assert_eq!(complete("gi", &words(), &top()), None);
        assert_eq!(complete("", &words(), &top()), None);
        assert!(complete("GIT_PAGER=cat git lo", &words(), &top()).is_some());
        assert!(complete("/usr/bin/git lo", &words(), &top()).is_some());
    }

    #[test]
    fn commands_are_offered_by_what_is_typed() {
        let all = labels("git ");
        assert_eq!(all.first().map(String::as_str), Some("status"));
        assert_eq!(all.len(), COMMANDS.len());
        assert_eq!(
            labels("git ch").first().map(String::as_str),
            Some("checkout")
        );
        assert_eq!(
            labels("git ch"),
            ["checkout", "cherry-pick", "sparse-checkout"]
        );
        // Git's own words match where they or a part begin; names of the
        // repository also by what they hold and by letters in order.
        assert_eq!(labels("git pick"), ["cherry-pick"]);
        assert_eq!(labels("git cp"), Vec::<String>::new());
        assert_eq!(labels("git checkout fcr"), ["fix/crash"]);
        let completion = complete("git ch", &words(), &top()).unwrap();
        assert_eq!((completion.start, completion.word.as_str()), (4, "ch"));
        assert_eq!(
            complete("git frobnicate x", &words(), &top())
                .unwrap()
                .candidates,
            []
        );
    }

    #[test]
    fn branches_remote_branches_and_tags_are_offered_for_a_commit() {
        // Before anything is typed, the options and then the names.
        let all = labels("git checkout ");
        assert_eq!(all.first().map(String::as_str), Some("-b"));
        let names: Vec<&str> = all
            .iter()
            .map(String::as_str)
            .filter(|label| !label.starts_with('-'))
            .collect();
        assert_eq!(
            names,
            [
                "develop",
                "feature/login",
                "fix/crash",
                "main",
                "origin/main",
                "origin/remote-only",
                "v0.1",
                "v1.0"
            ]
        );
        // What begins with it first, then a part that does, then holds it.
        assert_eq!(
            labels("git checkout m")
                .into_iter()
                .take(3)
                .collect::<Vec<_>>(),
            ["main", "origin/main", "origin/remote-only"]
        );
        assert_eq!(labels("git merge v")[..2], ["v0.1", "v1.0"]);
        // Options when one is begun.
        assert!(labels("git checkout -").contains(&"-b".to_owned()));
        // The value of -b is a new name: nothing.
        assert_eq!(labels("git checkout -b fe"), Vec::<String>::new());
        // After --, paths.
        assert_eq!(labels("git checkout -- READ"), ["README.md"]);
        let completion = complete("git switch rem", &words(), &top()).unwrap();
        let first = completion.candidates.first().unwrap();
        assert_eq!(
            (first.label.as_str(), first.detail.as_str()),
            ("remote-only", "origin")
        );
        let branch = complete("git checkout mai", &words(), &top()).unwrap();
        assert_eq!(branch.candidates.first().unwrap().detail, "Checked out");
    }

    #[test]
    fn remotes_then_their_branches() {
        assert_eq!(labels("git push o")[..1], ["origin"]);
        assert_eq!(labels("git pull origin re"), ["remote-only"]);
        assert_eq!(labels("git push origin fe"), ["feature/login"]);
        assert_eq!(labels("git remote rem"), ["remove"]);
        assert_eq!(labels("git remote remove "), ["origin"]);
        assert_eq!(labels("git rebase --onto ma")[..1], ["main"]);
        assert_eq!(labels("git rebase --onto=ma")[..1], ["main"]);
        let insert = complete("git rebase --onto=ma", &words(), &top()).unwrap();
        assert_eq!(insert.candidates[0].insert, "--onto=main");
    }

    #[test]
    fn stashes_and_flags_that_change_what_follows() {
        assert_eq!(labels("git stash p")[..2], ["push", "pop"]);
        assert_eq!(labels("git stash pop "), ["--index", "stash@{0}"]);
        assert_eq!(labels("git branch -d f"), ["feature/login", "fix/crash"]);
        assert_eq!(labels("git branch new-one ma")[..1], ["main"]);
        assert_eq!(labels("git branch ne"), Vec::<String>::new());
        assert_eq!(labels("git tag -d v"), ["v0.1", "v1.0"]);
    }

    #[test]
    fn paths_changed_first_from_the_shells_folder() {
        // An upper case letter typed: case counts.
        assert_eq!(labels("git add R"), ["README.md"]);
        assert_eq!(labels("git add r")[..1], ["README.md"]);
        // Untracked and modified files and their folders; not what is
        // only staged.
        assert_eq!(labels("git add s"), ["src/new.rs", "src/"]);
        assert_eq!(labels("git restore --staged d"), ["docs/guide.md", "docs/"]);
        assert_eq!(
            labels("git clean "),
            labels("git clean -")
                .into_iter()
                .chain(["src/new.rs", "src/"].map(str::to_owned))
                .collect::<Vec<_>>()
        );
        // From a folder below the top, what is in it, from it.
        let inside = Place {
            folder: Some("src".into()),
            shell: ShellKind::Posix,
        };
        assert_eq!(labels_in("git rm v", &inside), ["ui/view.rs"]);
        assert_eq!(labels_in("git rm u", &inside), ["ui/", "ui/view.rs"]);
        assert_eq!(labels_in("git blame ./m", &inside), ["main.rs"]);
        let completion = complete("git blame ./m", &words(), &inside).unwrap();
        assert_eq!(completion.candidates[0].insert, "./main.rs");
        // Outside the worktree: git's own words only.
        let outside = Place {
            folder: None,
            shell: ShellKind::Posix,
        };
        assert_eq!(labels_in("git add R", &outside), Vec::<String>::new());
        assert_eq!(labels_in("git checkout m", &outside), Vec::<String>::new());
        assert_eq!(labels_in("git ch", &outside)[..1], ["checkout"]);
        // Another repository named: nothing of this one.
        assert_eq!(labels("git -C ../other checkout m"), Vec::<String>::new());
    }

    #[test]
    fn names_are_written_as_the_shell_reads_them() {
        let mut odd = words();
        odd.tracked.push("my notes.txt".into());
        let completion = complete("git rm my", &odd, &top()).unwrap();
        assert_eq!(completion.candidates[0].insert, "'my notes.txt'");
        // A quote begun: what is typed is matched without it, and the
        // whole word is written again.
        let completion = complete("git rm 'my", &odd, &top()).unwrap();
        assert_eq!(completion.word, "'my");
        assert_eq!(completion.candidates[0].insert, "'my notes.txt'");
    }

    #[test]
    fn hostile_lines_are_answered_without_trouble() {
        let long = format!("git checkout {}", "a".repeat(LONGEST_LINE));
        assert_eq!(complete(&long, &words(), &top()), None);
        for line in [
            "git \u{0}",
            "git \\",
            "git '",
            "git \"\\",
            "git -c",
            "git -C",
            "git --",
            "git stash -- ",
            "git checkout -- -- ",
            "git é漢 ",
            "git checkout ü/../",
            "git remote add origin",
        ] {
            let _ = complete(line, &words(), &top());
        }
        let many = Words {
            tracked: (0..5_000)
                .map(|n| format!("dir{}/file{n}.txt", n % 50))
                .collect(),
            ..words()
        };
        let completion = complete("git rm ", &many, &top()).unwrap();
        assert_eq!(completion.candidates.len(), MOST_CANDIDATES);
    }
}
