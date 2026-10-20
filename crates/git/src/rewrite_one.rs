//! One change to a commit of the current branch where a plain list of
//! commits cannot describe the way back up to HEAD: a merge commit lies in
//! between, the commit is a merge commit itself, or it is the first commit
//! of the history and there is nothing before it to replay the others on.
//!
//! Git writes the list for such a history itself (`git rebase
//! --interactive --rebase-merges`, with `--root` from the first commit):
//! besides the commits it has labels, resets to a label and merges of
//! labels. That list is asked for first (with an editor that keeps a copy
//! and fails, so that nothing is begun), the one change is made in it, and
//! git is started again with the changed list, as [`crate::interactive`]
//! starts it with a list of its own.
//!
//! What "the merges are kept" means differs between the three changes:
//!
//! - **Reword** and **squash** leave every tree after them as it is. Each
//!   merge commit that comes after is therefore made again with exactly
//!   the tree, the message and the author it has, on its new parents: a
//!   merge that was resolved by hand, or changed afterwards, is kept to the
//!   letter. Git's own `merge` line would merge anew and ask again.
//! - **Drop** takes changes out, so what a later merge has to be is not
//!   known beforehand: git merges anew. That is exact when merging the two
//!   parents by itself gives the merge commit that is there. A later merge
//!   that was resolved by hand or holds changes of its own would be made
//!   without them, so the drop is refused and says which merge is in the
//!   way.
//! - A merge commit cannot be squashed into its parent (that would be the
//!   merge undone and its changes kept, which no line of the list says),
//!   and commits are squashed only where no other line of the history
//!   starts between them.
//!
//! Sources: git-rebase(1) (`--rebase-merges`, `--root`, "Rebasing merges":
//! `label`, `reset`, `merge -C`, `refs/rewritten/`; `exec`, `drop`,
//! `fixup`), git-merge(1) (`-s ours`, `--no-commit`), git-read-tree(1),
//! git-commit(1) (`-C`, `--cleanup`), git-merge-tree(1), git-rev-list(1)
//! (`--ancestry-path`, `--parents`).

use std::collections::{HashMap, HashSet};

use crate::{
    Error, Oid, Repository, Result, branch, exe,
    interactive::{self, MAX_COMMITS},
    rewrite::{self, Moved},
};

/// The one change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    /// Another message for a commit.
    Reword {
        /// The commit.
        commit: Oid,
        /// Its new message.
        message: String,
    },
    /// A commit taken out.
    Drop {
        /// The commit.
        commit: Oid,
    },
    /// Commits melted into the one before them, messages together.
    Squash {
        /// The commit that stays.
        into: Oid,
        /// The ones that go into it, oldest first: each the only child of
        /// the one before.
        melted: Vec<Oid>,
    },
}

impl Change {
    /// The oldest commit the change touches: the rewrite begins there.
    pub fn oldest(&self) -> Oid {
        match self {
            Change::Reword { commit, .. } | Change::Drop { commit } => *commit,
            Change::Squash { into, .. } => *into,
        }
    }
}

fn what() -> &'static str {
    say!("rewrite-what-rewrite")
}

/// Largest message a change can carry.
const MAX_MESSAGE: usize = 1 << 20;

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: what(),
        detail: detail.into(),
    }
}

fn invalid(detail: impl Into<String>) -> Error {
    Error::Invalid {
        what: what(),
        detail: detail.into(),
    }
}

/// The parents of a commit.
pub fn parents(repo: &Repository, commit: Oid) -> Result<Vec<Oid>> {
    let hex = commit.to_hex();
    let line = exe::run(
        repo,
        ["rev-list", "--parents", "-n", "1", hex.as_str()],
        None,
    )?;
    Ok(line
        .stdout
        .split_whitespace()
        .skip(1)
        .filter_map(|id| id.parse().ok())
        .collect())
}

/// Whether a change that begins at `oldest` has to go this way: it is the
/// first commit of the history, a merge commit, or a merge commit lies
/// between it and HEAD. Otherwise a plain list of commits says it all and
/// [`crate::interactive`] carries it out.
pub fn needed(repo: &Repository, oldest: Oid) -> Result<bool> {
    let parents = parents(repo, oldest)?;
    let [parent] = parents.as_slice() else {
        return Ok(true);
    };
    let span = format!("{}..HEAD", parent.to_hex());
    let merges = exe::run(
        repo,
        ["rev-list", "--merges", "--max-count=1", span.as_str()],
        None,
    )?;
    Ok(!merges.stdout.trim().is_empty())
}

/// A commit by itself as an [`interactive::Range`] of one: what there is
/// to show of it where it is reworded and no plan for more can be made.
/// The base is the commit before it, or the commit itself when there is
/// none.
pub fn alone(repo: &Repository, commit: Oid) -> Result<interactive::Range> {
    let hex = commit.to_hex();
    let branch = branch::head_state(repo)?.branch.unwrap_or_default();
    let upstream = exe::run(
        repo,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
        None,
    )
    .ok()
    .map(|out| out.stdout.trim().to_owned())
    .filter(|name| !name.is_empty());
    let published = upstream.as_ref().is_some_and(|upstream| {
        exe::run_with_code(
            repo,
            [
                "merge-base",
                "--is-ancestor",
                hex.as_str(),
                upstream.as_str(),
            ],
            None,
        )
        .is_ok_and(|(code, _)| code == Some(0))
    });
    let out = exe::run(
        repo,
        ["log", "-1", "--format=%an%n%ae%n%at%n%B", hex.as_str()],
        None,
    )?;
    let mut parts = out.stdout.splitn(4, '\n');
    let author = parts.next().unwrap_or("").to_owned();
    let email = parts.next().unwrap_or("").to_owned();
    let time = parts
        .next()
        .and_then(|time| time.trim().parse().ok())
        .unwrap_or(0);
    let message = tidy(parts.next().unwrap_or("")).to_owned();
    Ok(interactive::Range {
        base: parents(repo, commit)?.first().copied().unwrap_or(commit),
        branch,
        upstream,
        commits: vec![interactive::Entry {
            id: commit,
            summary: message.lines().next().unwrap_or("").to_owned(),
            message,
            author,
            email,
            time,
            published,
        }],
    })
}

/// The commits to squash, put in order: the one that stays and the ones
/// that go into it, oldest first. One commit alone goes into its parent.
/// Refused when they do not follow one another, or when one that would go
/// into another is a merge commit.
pub fn squash_of(repo: &Repository, commits: &[Oid]) -> Result<Change> {
    let merge = |id: Oid| refused(say!("rewrite-merge-not-squashable", commit = id.short(7)));
    if let [only] = commits {
        return match parents(repo, *only)?.as_slice() {
            [] => Err(refused(say!(
                "rewrite-first-not-squashable",
                commit = only.short(7)
            ))),
            [parent] => Ok(Change::Squash {
                into: *parent,
                melted: vec![*only],
            }),
            _ => Err(merge(*only)),
        };
    }
    let apart = || invalid(say!("rewrite-commits-apart"));
    let named: HashSet<Oid> = commits.iter().copied().collect();
    if named.len() != commits.len() || commits.is_empty() {
        return Err(apart());
    }
    // Each with the commits before it; the one that stays is the only one
    // none of whose parents is among them.
    let mut before: HashMap<Oid, Vec<Oid>> = HashMap::new();
    for id in commits {
        before.insert(*id, parents(repo, *id)?);
    }
    let mut oldest = before
        .iter()
        .filter(|(_, parents)| !parents.iter().any(|parent| named.contains(parent)))
        .map(|(id, _)| *id);
    let (Some(into), None) = (oldest.next(), oldest.next()) else {
        return Err(apart());
    };
    let mut melted = Vec::new();
    let mut last = into;
    while melted.len().saturating_add(1) < commits.len() {
        // The one whose first parent is the last one taken.
        let Some((next, its)) = before
            .iter()
            .find(|(_, parents)| parents.first() == Some(&last))
        else {
            return Err(apart());
        };
        if its.len() != 1 {
            return Err(merge(*next));
        }
        last = *next;
        melted.push(last);
    }
    Ok(Change::Squash { into, melted })
}

/// A message without the blank lines around it.
fn tidy(message: &str) -> &str {
    message.trim_matches(['\n', '\r', ' ', '\t'])
}

/// The whole message of a commit, without the blank lines around it.
pub fn message_of(repo: &Repository, commit: Oid) -> Result<String> {
    let hex = commit.to_hex();
    let out = exe::run(repo, ["log", "-1", "--format=%B", hex.as_str()], None)?;
    Ok(tidy(&out.stdout).to_owned())
}

/// A word in single quotes for the shell git runs `exec` lines with.
fn quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn quoted_path(path: &std::path::Path) -> Result<String> {
    path.to_str()
        .filter(|text| !text.contains(['\n', '\r', '\0']))
        // Git's shell reads it: inside a WSL distribution for a repository
        // there, elsewhere on Windows with `/` in the path, as git.
        .map(|text| quoted(&exe::path_for_git_shell(text)))
        .ok_or_else(|| refused(say!("rewrite-path-not-for-sequencer")))
}

/// Git's list without its comments, as `grep -v '^#'` prints it, which is
/// how it is compared when it is asked for the second time. The comments
/// name the commit the list is replayed onto, and with `--root` that is a
/// commit git makes each time, with the time it is made: the lists of one
/// history differ there when they are asked for a second apart.
fn uncommented(list: &str) -> String {
    let mut out = String::with_capacity(list.len());
    for line in list
        .split_inclusive('\n')
        .filter(|line| !line.starts_with('#'))
    {
        out.push_str(line);
        if !line.ends_with('\n') {
            out.push('\n');
        }
    }
    out
}

/// One line of git's list, as far as it matters here.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Line {
    /// `pick <commit>`.
    Pick(Oid),
    /// `merge -C <commit> <label>…`: the merge commit made again from the
    /// labels (or commits) named.
    Merge(Oid, Vec<String>),
    /// Anything else, kept as it is: labels, resets, references to update.
    Other(String),
}

/// The commit of the range that a (possibly shortened) id names.
fn full(short: &str, range: &[Oid]) -> Result<Oid> {
    let mut found = range.iter().filter(|id| id.to_hex().starts_with(short));
    match (found.next(), found.next()) {
        (Some(id), None) if !short.is_empty() => Ok(*id),
        _ => Err(invalid(say!(
            "rewrite-list-names-stranger",
            short = format!("{short:?}")
        ))),
    }
}

/// Read git's list. Comments and blank lines are left out.
fn parse(list: &str, range: &[Oid]) -> Result<Vec<Line>> {
    let mut lines = Vec::new();
    for raw in list.lines() {
        let text = raw.trim();
        if text.is_empty() || text.starts_with('#') {
            continue;
        }
        let mut words = text.split_whitespace();
        match words.next() {
            Some("pick") => lines.push(Line::Pick(full(words.next().unwrap_or(""), range)?)),
            Some("merge") => {
                // `-C <commit>`; a merge that is new (no commit to take the
                // message from) is not something git writes for a history
                // that exists.
                if words.next() != Some("-C") {
                    return Err(invalid(say!("rewrite-list-merge-without-commit")));
                }
                let id = full(words.next().unwrap_or(""), range)?;
                let labels: Vec<String> = words
                    .take_while(|word| !word.starts_with('#'))
                    .map(str::to_owned)
                    .collect();
                if labels.is_empty() {
                    return Err(invalid(say!("rewrite-list-merge-of-nothing")));
                }
                lines.push(Line::Merge(id, labels));
            }
            Some(_) => lines.push(Line::Other(text.to_owned())),
            None => {}
        }
    }
    Ok(lines)
}

/// The line that makes the merge commit `merge` again as it is, on the
/// commit the list has arrived at and the labels named: its tree, its
/// message and its author, whatever merging them anew would give. A label
/// is a reference under `refs/rewritten/` while the rebase runs; what is
/// not one names a commit that stays as it is.
fn remade(merge: Oid, labels: &[String]) -> String {
    let hex = merge.to_hex();
    let mut line = String::from("exec ");
    let mut heads = String::new();
    for (index, label) in labels.iter().enumerate() {
        let rewritten = quoted(&format!("refs/rewritten/{label}^{{commit}}"));
        let plain = quoted(&format!("{label}^{{commit}}"));
        line.push_str(&format!(
            "h{index}=$(git rev-parse --verify --quiet {rewritten} || git rev-parse --verify \
             {plain}) && "
        ));
        heads.push_str(&format!(" \"$h{index}\""));
    }
    // A merge that takes nothing from the other side, so that it cannot
    // conflict; then the tree of the merge that was; then its commit, with
    // no hook asked: nothing in it is new.
    line.push_str(&format!(
        "git merge --quiet -s ours --no-ff --no-commit{heads} && git read-tree -u --reset {hex} \
         && git commit --quiet --no-verify --cleanup=verbatim -C {hex}"
    ));
    line
}

/// The list with the change made in it, and the message files it refers to
/// (name and content). `after` holds the commits that come after the
/// change and are made again; `dir` is where the files will be.
fn changed(
    lines: &[Line],
    change: &Change,
    after: &HashSet<Oid>,
    messages: &HashMap<Oid, String>,
    dir: &std::path::Path,
) -> Result<(String, Vec<(String, String)>)> {
    let position = |id: Oid| {
        lines
            .iter()
            .position(|line| matches!(line, Line::Pick(at) | Line::Merge(at, _) if *at == id))
            .ok_or_else(|| invalid(say!("rewrite-list-lacks-commit", commit = id.short(7))))
    };
    let mut files: Vec<(String, String)> = Vec::new();
    let mut amend = |message: &str, out: &mut Vec<String>| -> Result<()> {
        let name = format!("message-{}", files.len());
        out.push(format!(
            "exec git commit --amend --quiet --cleanup=verbatim --file={}",
            quoted_path(&dir.join(&name))?
        ));
        files.push((name, format!("{message}\n")));
        Ok(())
    };
    // Trees stay as they are after a reword and after a squash, so the
    // merges that follow are made again as they are; a drop lets git merge.
    let keep_merges = !matches!(change, Change::Drop { .. });
    let at = position(change.oldest())?;
    let mut out: Vec<String> = Vec::new();
    let mut skip = 0usize;
    for (index, line) in lines.iter().enumerate() {
        if skip > 0 {
            skip = skip.saturating_sub(1);
            continue;
        }
        let plain = |line: &Line| match line {
            Line::Pick(id) => format!("pick {}", id.to_hex()),
            Line::Merge(id, labels) if keep_merges && after.contains(id) => remade(*id, labels),
            Line::Merge(id, labels) => format!("merge -C {} {}", id.to_hex(), labels.join(" ")),
            Line::Other(text) => text.clone(),
        };
        if index != at {
            out.push(plain(line));
            continue;
        }
        match change {
            Change::Reword { message, .. } => {
                out.push(match line {
                    // Made again whatever its parents are, so that a merge
                    // git would redo is not redone.
                    Line::Merge(id, labels) => remade(*id, labels),
                    other => plain(other),
                });
                amend(tidy(message), &mut out)?;
            }
            // A merge is left out of the list rather than dropped: git
            // 2.50 (Apple's, macOS's own) stops at a `drop` of a merge
            // commit with "BUG: sequencer.c: unexpected todo_command" and
            // leaves the rebase begun. Left out, it is dropped by every
            // version (`rebase.missingCommitsCheck=ignore`).
            Change::Drop { .. } if matches!(line, Line::Merge(..)) => {}
            Change::Drop { commit } => out.push(format!("drop {}", commit.to_hex())),
            Change::Squash { into, melted } => {
                out.push(match line {
                    Line::Merge(id, labels) => remade(*id, labels),
                    other => plain(other),
                });
                let mut parts = vec![messages.get(into).cloned().unwrap_or_default()];
                for (offset, id) in melted.iter().enumerate() {
                    let next = index.saturating_add(offset).saturating_add(1);
                    if lines.get(next) != Some(&Line::Pick(*id)) {
                        return Err(refused(say!(
                            "rewrite-line-starts-between",
                            into = into.short(7),
                            commit = id.short(7)
                        )));
                    }
                    out.push(format!("fixup {}", id.to_hex()));
                    let part = messages.get(id).cloned().unwrap_or_default();
                    if !part.is_empty() {
                        parts.push(part);
                    }
                }
                skip = melted.len();
                parts.retain(|part| !part.is_empty());
                amend(&parts.join("\n\n"), &mut out)?;
            }
        }
    }
    let mut list = out.join("\n");
    list.push('\n');
    Ok((list, files))
}

/// Refuse a drop that a later merge could not follow: git makes the merge
/// anew, and that is the merge there is only when merging its two parents
/// by itself gives the tree it has.
fn merges_can_be_redone(repo: &Repository, redone: &[(Oid, Vec<Oid>)]) -> Result<()> {
    for (merge, parents) in redone {
        let [ours, theirs] = parents.as_slice() else {
            return Err(refused(say!(
                "rewrite-merge-of-more-than-two",
                merge = merge.short(7)
            )));
        };
        let (ours, theirs) = (ours.to_hex(), theirs.to_hex());
        let (code, out) = exe::run_with_code(
            repo,
            [
                "merge-tree",
                "--write-tree",
                "--no-messages",
                ours.as_str(),
                theirs.as_str(),
            ],
            None,
        )?;
        let tree = format!("{}^{{tree}}", merge.to_hex());
        let has = exe::run(repo, ["rev-parse", "--verify", tree.as_str()], None)?;
        let by_itself = out.stdout.lines().next().unwrap_or("").trim();
        match code {
            Some(0) if by_itself == has.stdout.trim() => {}
            Some(0) => {
                return Err(refused(say!(
                    "rewrite-merge-has-own-changes",
                    merge = merge.short(7)
                )));
            }
            Some(1) => {
                return Err(refused(say!(
                    "rewrite-merge-resolved-by-hand",
                    merge = merge.short(7)
                )));
            }
            _ => {
                return Err(refused(say!(
                    "rewrite-merge-cannot-be-told",
                    merge = merge.short(7)
                )));
            }
        }
    }
    Ok(())
}

/// Make the change. A conflict (a drop can meet one) leaves the rebase in
/// progress, and the result says so.
pub fn run(repo: &Repository, change: &Change) -> Result<Moved> {
    let oldest = change.oldest();
    let before = branch::head_state(repo)?;
    let Some(branch) = before.branch.clone() else {
        return Err(refused(say!("rewrite-head-not-on-branch")));
    };
    let oldest_hex = oldest.to_hex();
    let (code, _) = exe::run_with_code(
        repo,
        ["merge-base", "--is-ancestor", oldest_hex.as_str(), "HEAD"],
        None,
    )?;
    if code != Some(0) {
        return Err(refused(say!(
            "rewrite-not-in-branch-history",
            commit = oldest.short(7),
            branch = branch.as_str()
        )));
    }
    if let Change::Reword { message, .. } = change
        && (tidy(message).is_empty() || message.len() > MAX_MESSAGE || message.contains('\0'))
    {
        return Err(invalid(say!("rewrite-message-unusable")));
    }
    // Where the replay begins: on the first parent, or from nothing.
    let base = parents(repo, oldest)?.first().copied();
    let base_hex = base.map(|base| base.to_hex());
    let not_base = base_hex.as_ref().map(|base| format!("^{base}"));
    let limit = format!("--max-count={}", MAX_COMMITS.saturating_add(1));
    let mut listing = vec!["rev-list", "--parents", limit.as_str(), "HEAD"];
    if let Some(not_base) = &not_base {
        listing.push(not_base.as_str());
    }
    let listed = exe::run(repo, listing, None)?;
    let mut range: Vec<Oid> = Vec::new();
    let mut parents_of: HashMap<Oid, Vec<Oid>> = HashMap::new();
    for line in listed.stdout.lines() {
        let mut ids = line
            .split_whitespace()
            .filter_map(|id| id.parse::<Oid>().ok());
        let Some(id) = ids.next() else {
            return Err(invalid(say!("rewrite-range-unreadable")));
        };
        range.push(id);
        parents_of.insert(id, ids.collect());
    }
    if range.len() > MAX_COMMITS {
        return Err(refused(say!(
            "rewrite-too-many-commits",
            most = MAX_COMMITS,
            commit = oldest.short(7),
            branch = branch.as_str()
        )));
    }
    if matches!(change, Change::Drop { .. }) && range.len() < 2 && base.is_none() {
        return Err(refused(say!(
            "rewrite-only-commit",
            commit = oldest.short(7),
            branch = branch.as_str()
        )));
    }
    // What comes after the last commit the change touches: made again.
    let last = match change {
        Change::Squash { into, melted } => melted.last().copied().unwrap_or(*into),
        _ => oldest,
    };
    let path = format!("{}..HEAD", last.to_hex());
    let later = exe::run(repo, ["rev-list", "--ancestry-path", path.as_str()], None)?;
    let after: HashSet<Oid> = later
        .stdout
        .lines()
        .filter_map(|id| id.trim().parse().ok())
        .collect();
    let mut messages = HashMap::new();
    if let Change::Squash { into, melted } = change {
        for id in std::iter::once(into).chain(melted) {
            if !range.contains(id) {
                return Err(refused(say!(
                    "rewrite-not-in-branch-history",
                    commit = id.short(7),
                    branch = branch.as_str()
                )));
            }
            messages.insert(*id, message_of(repo, *id)?);
        }
    }
    rewrite::ensure_idle(repo, what())?;
    if matches!(change, Change::Drop { .. }) {
        let redone: Vec<(Oid, Vec<Oid>)> = range
            .iter()
            .filter(|id| after.contains(id))
            .filter_map(|id| {
                let parents = parents_of.get(id)?;
                (parents.len() > 1).then(|| (*id, parents.clone()))
            })
            .collect();
        merges_can_be_redone(repo, &redone)?;
    }

    let dir = interactive::plan_dir(repo);
    let io = |err: std::io::Error| refused(say!("rewrite-plan-not-written", err = err.to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(io)?;
    let theirs = dir.join("theirs");
    let ours = dir.join("todo");
    let mut args = vec![
        // The list is read here by its words, and no commit may be missed
        // for having been left out of it on purpose.
        "-c",
        "rebase.abbreviateCommands=false",
        "-c",
        "rebase.missingCommitsCheck=ignore",
        "rebase",
        "--interactive",
        "--rebase-merges",
        "--empty=drop",
        "--no-autosquash",
        "--reschedule-failed-exec",
    ];
    match &base_hex {
        Some(base) => args.push(base.as_str()),
        None => args.push("--root"),
    }
    // First for the list alone: the editor keeps a copy and fails, and git
    // gives up before anything is begun. No hook is asked; the second time
    // is the one that counts.
    let keep = format!("f() {{ cp \"$1\" {}; exit 1; }}; f", quoted_path(&theirs)?);
    let mut asking = args.clone();
    asking.push("--no-verify");
    let asked = exe::run_with_env(repo, asking, &[("GIT_SEQUENCE_EDITOR", keep.as_str())]);
    let list = match (std::fs::read_to_string(&theirs), asked) {
        (Ok(list), _) => list,
        // Git refused before it had a list (local changes in the way, say).
        (Err(_), Err(err)) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(err);
        }
        (Err(_), Ok(_)) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(refused(say!("rewrite-nothing-to-replay")));
        }
    };
    let plain = dir.join("theirs-uncommented");
    std::fs::write(&plain, uncommented(&list)).map_err(io)?;
    let made =
        parse(&list, &range).and_then(|lines| changed(&lines, change, &after, &messages, &dir));
    let (list, files) = match made {
        Ok(made) => made,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&dir);
            return Err(err);
        }
    };
    for (name, message) in &files {
        std::fs::write(dir.join(name), message).map_err(io)?;
    }
    std::fs::write(&ours, list).map_err(io)?;
    // The changed list goes in only where git wrote the list it was made
    // from: a history that moved in between is not rewritten by a plan for
    // the one before. Its comments are left out of that (see
    // `uncommented`).
    let put = format!(
        "f() {{ grep -v '^#' \"$1\" | cmp -s - {} && cp {} \"$1\"; }}; f",
        quoted_path(&plain)?,
        quoted_path(&ours)?
    );
    interactive::carry_out(repo, &args, &put, before, what())
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::{Change, Line, changed, parse, remade, uncommented};
    use crate::Oid;

    fn id(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    /// Git's list for: 1, 2, a side line 3 from 2, 4 on the main line, the
    /// merge 5 of the side line, 6.
    fn list() -> (String, Vec<Oid>) {
        let range: Vec<Oid> = (1..=6).map(id).collect();
        let short = |n: u8| id(n).to_hex()[..9].to_owned();
        let text = format!(
            "label onto\n\n# Branch side\nreset onto\npick {} # one\npick {} # two\nlabel \
             branch-point\npick {} # three\nlabel side\n\nreset branch-point # two\npick {} # \
             four\nmerge -C {} side # Merge branch 'side'\npick {} # six\n",
            short(1),
            short(2),
            short(3),
            short(4),
            short(5),
            short(6)
        );
        (text, range)
    }

    #[test]
    fn the_list_is_compared_without_its_comments() {
        let asked = |onto: &str| {
            format!(
                "label onto\n\nreset [new root]\npick 1234567 # one\n\n# Rebase 89abcde onto {onto} (3 commands)\n#\n# Commands:\n"
            )
        };
        assert_eq!(
            uncommented(&asked("1c2609e")),
            uncommented(&asked("8e2b43b"))
        );
        assert_eq!(
            uncommented(&asked("1c2609e")),
            "label onto\n\nreset [new root]\npick 1234567 # one\n\n"
        );
        // As grep prints it: every line ends in one.
        assert_eq!(uncommented("pick 1\n# x\npick 2"), "pick 1\npick 2\n");
        assert_eq!(uncommented(""), "");
        assert_eq!(uncommented(" # not a comment\n"), " # not a comment\n");
    }

    #[test]
    fn gits_list_is_read_by_its_words() {
        let (text, range) = list();
        let lines = parse(&text, &range).unwrap();
        assert_eq!(lines.len(), 11);
        assert_eq!(lines[2], Line::Pick(id(1)));
        assert_eq!(lines[9], Line::Merge(id(5), vec!["side".to_owned()]));
        assert_eq!(lines[7], Line::Other("reset branch-point # two".to_owned()));
        // An id that is none of the range, or more than one of it.
        assert!(parse("pick 99\n", &range).is_err());
        assert!(parse("pick \n", &range).is_err());
        assert!(parse("pick\n", &range).is_err());
        let twins: Vec<Oid> = vec![
            "aa".repeat(20).parse().unwrap(),
            format!("aa{}", "bb".repeat(19)).parse().unwrap(),
        ];
        assert!(parse("pick aa\n", &twins).is_err());
        assert!(parse("pick aab\n", &twins).is_ok());
        // A merge with nothing to take the message from, or of nothing.
        assert!(parse("merge side\n", &range).is_err());
        assert!(parse(&format!("merge -C {}\n", id(5).to_hex()), &range).is_err());
        assert!(parse("", &range).unwrap().is_empty());
    }

    #[test]
    fn one_change_is_made_in_gits_list() {
        let (text, range) = list();
        let lines = parse(&text, &range).unwrap();
        let dir = std::path::Path::new("/r/.git/bitbraid/rebase");
        let after = |from: u8| -> HashSet<Oid> { (from..=6).map(id).collect() };
        let none = HashMap::new();

        // A reword: the commit, then its message; the merge after it made
        // again as it is, not merged anew.
        let reword = Change::Reword {
            commit: id(1),
            message: "\nONE\n".to_owned(),
        };
        let (list, files) = changed(&lines, &reword, &after(2), &none, dir).unwrap();
        let rows: Vec<&str> = list.lines().collect();
        assert_eq!(rows[2], format!("pick {}", id(1).to_hex()));
        assert_eq!(
            rows[3],
            "exec git commit --amend --quiet --cleanup=verbatim \
             --file='/r/.git/bitbraid/rebase/message-0'"
        );
        assert_eq!(files, [("message-0".to_owned(), "ONE\n".to_owned())]);
        assert_eq!(rows[10], remade(id(5), &["side".to_owned()]));
        assert!(rows[10].contains("refs/rewritten/side^{commit}"));
        assert!(rows[10].contains(&format!("-C {}", id(5).to_hex())));
        assert!(!list.contains("merge -C"));

        // The merge itself reworded: made again, then its message.
        let reword = Change::Reword {
            commit: id(5),
            message: "merged".to_owned(),
        };
        let (list, _) = changed(&lines, &reword, &after(6), &none, dir).unwrap();
        let rows: Vec<&str> = list.lines().collect();
        assert!(rows[9].starts_with("exec h0="));
        assert!(rows[10].starts_with("exec git commit --amend"));

        // A drop leaves the merge to git; a dropped merge is left out,
        // which git 2.50 takes and a `drop` line of it it does not.
        let (list, files) = changed(
            &lines,
            &Change::Drop { commit: id(2) },
            &after(3),
            &none,
            dir,
        )
        .unwrap();
        assert!(files.is_empty());
        assert!(list.contains(&format!("drop {}\n", id(2).to_hex())));
        assert!(list.contains(&format!("merge -C {} side\n", id(5).to_hex())));
        let (list, _) = changed(
            &lines,
            &Change::Drop { commit: id(5) },
            &after(6),
            &none,
            dir,
        )
        .unwrap();
        assert!(!list.contains(&id(5).to_hex()));
        assert!(!list.contains("merge"));

        // A squash where the two lines follow one another.
        let messages: HashMap<Oid, String> = [(id(1), "one"), (id(2), "two"), (id(3), "three")]
            .into_iter()
            .map(|(id, text)| (id, text.to_owned()))
            .collect();
        let squash = Change::Squash {
            into: id(1),
            melted: vec![id(2)],
        };
        let (list, files) = changed(&lines, &squash, &after(3), &messages, dir).unwrap();
        let rows: Vec<&str> = list.lines().collect();
        assert_eq!(rows[3], format!("fixup {}", id(2).to_hex()));
        assert!(rows[4].starts_with("exec git commit --amend"));
        assert_eq!(rows[5], "label branch-point");
        assert_eq!(files, [("message-0".to_owned(), "one\n\ntwo\n".to_owned())]);
        // And where another line starts between them: refused.
        let squash = Change::Squash {
            into: id(2),
            melted: vec![id(3)],
        };
        let err = changed(&lines, &squash, &after(4), &messages, dir)
            .unwrap_err()
            .to_string();
        assert!(err.contains("another line of the history starts"), "{err}");
        // A commit git's list does not have.
        assert!(
            changed(
                &lines,
                &Change::Drop { commit: id(9) },
                &after(6),
                &none,
                dir
            )
            .is_err()
        );
    }

    #[test]
    fn a_label_is_quoted_for_the_shell() {
        let line = remade(id(5), &["it's".to_owned(), "two".to_owned()]);
        assert!(
            line.contains("'refs/rewritten/it'\\''s^{commit}'"),
            "{line}"
        );
        assert!(line.contains("\"$h0\" \"$h1\""), "{line}");
    }
}
