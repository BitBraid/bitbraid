//! Interactive rebase: a plan for the commits from one commit up to HEAD,
//! checked here and carried out by git's own sequencer.
//!
//! A plan gives every commit of the range one action: keep it, keep it
//! with another message, melt it into the commit before it (with or
//! without its message), stop at it, or leave it out. The order of the
//! plan is the order the commits are applied in.
//!
//! Git gets the plan as its todo list: `GIT_SEQUENCE_EDITOR` is a command
//! that copies the list prepared here over the one git wrote. No message
//! goes through an editor. A new message is a `pick` followed by `exec git
//! commit --amend --file=…`, and a squash is a `fixup` followed by the
//! same for the messages put together here, so that a line beginning with
//! `#` stays in a message instead of being taken for a comment.
//!
//! A range with a merge commit in it, or one that reaches the first commit
//! of the history, is refused: git can rebase both, but not in a way a
//! plain list of commits describes. One change to one commit of such a
//! history goes through [`crate::rewrite_one`] instead.
//!
//! Sources: git-rebase(1) (interactive mode, the todo commands, `--empty`,
//! `--reschedule-failed-exec`), git-commit(1) (`--amend`, `--file`,
//! cleanup modes), git(1) (`GIT_SEQUENCE_EDITOR`), git-rev-list(1).

use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{
    Error, Oid, Repository, Result, branch, exe,
    rewrite::{self, Moved, OnStop},
};

/// What happens to one commit of the plan.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Keep it as it is.
    #[default]
    Pick,
    /// Keep its changes under another message.
    Reword,
    /// Melt it into the commit before it, and add its message to that one.
    Squash,
    /// Melt it into the commit before it, and forget its message.
    Fixup,
    /// Stop after it, so that it can be amended.
    Edit,
    /// Leave it out.
    Drop,
}

/// One line of a plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    /// The commit.
    pub id: Oid,
    /// What to do with it.
    #[serde(default)]
    pub action: Action,
    /// For a reword: the new message. For a squash: what it adds to the
    /// message of the commit it melts into, when that is not its own
    /// message.
    #[serde(default)]
    pub message: Option<String>,
}

/// A commit that a plan can be made for.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Entry {
    /// The commit.
    pub id: Oid,
    /// The first line of its message.
    pub summary: String,
    /// Its whole message.
    pub message: String,
    /// Who wrote it.
    pub author: String,
    /// Their address, for their picture.
    #[serde(default)]
    pub email: String,
    /// When, in seconds since the epoch.
    pub time: i64,
    /// It is on the branch's upstream already: rewriting it means the
    /// branch has to be pushed by force afterwards.
    pub published: bool,
}

/// The commits a plan is about.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Range {
    /// The commit the plan is replayed on: the parent of the oldest one.
    pub base: Oid,
    /// The branch that is rewritten.
    pub branch: String,
    /// Its upstream, when it has one.
    pub upstream: Option<String>,
    /// The commits, oldest first.
    pub commits: Vec<Entry>,
}

/// A commit as it will be once the plan is carried out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Resulting {
    /// The commits that go into it, in the order they are applied.
    pub from: Vec<Oid>,
    /// Its message.
    pub message: String,
    /// The rebase stops after it, for it to be amended.
    pub stops: bool,
}

/// Most commits a plan can hold: a list longer than this is not edited by
/// hand.
pub const MAX_COMMITS: usize = 1000;

/// Largest message a plan can carry.
const MAX_MESSAGE: usize = 1 << 20;

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("interactive-what"),
        detail: detail.into(),
    }
}

/// A message without the blank lines around it.
fn tidy(message: &str) -> &str {
    message.trim_matches(['\n', '\r', ' ', '\t'])
}

/// The commits from `first` up to HEAD, to make a plan for.
pub fn range(repo: &Repository, first: &str) -> Result<Range> {
    let first_id = rewrite::commit_of(repo, say!("interactive-what-first-commit"), first)?;
    let head = branch::head_state(repo)?;
    let Some(branch) = head.branch else {
        return Err(refused(say!("interactive-head-detached")));
    };
    let first_hex = first_id.to_hex();
    if exe::run(
        repo,
        ["merge-base", "--is-ancestor", first_hex.as_str(), "HEAD"],
        None,
    )
    .is_err()
    {
        return Err(refused(say!(
            "interactive-not-in-history",
            commit = first_id.short(7),
            branch = branch.as_str()
        )));
    }
    // The commit itself with its parents.
    let line = exe::run(
        repo,
        ["rev-list", "--parents", "-n", "1", first_hex.as_str()],
        None,
    )?;
    let mut ids = line.stdout.split_whitespace().skip(1);
    let base: Oid = match (ids.next(), ids.next()) {
        (None, _) => {
            return Err(refused(say!(
                "interactive-first-of-history",
                commit = first_id.short(7)
            )));
        }
        (Some(_), Some(_)) => return Err(merge_in_the_way(first_id)),
        (Some(parent), None) => parent.parse().map_err(|_| {
            invalid(
                say!("interactive-what-first-commit"),
                say!("interactive-parent-unreadable"),
            )
        })?,
    };
    let span = format!("{}..HEAD", base.to_hex());
    let limit = format!("--max-count={}", MAX_COMMITS.saturating_add(1));
    let listed = exe::run(
        repo,
        ["rev-list", "--parents", limit.as_str(), span.as_str()],
        None,
    )?;
    let mut count = 0usize;
    for line in listed.stdout.lines() {
        count = count.saturating_add(1);
        let mut ids = line.split_whitespace();
        let id = ids.next().and_then(|id| id.parse::<Oid>().ok());
        if ids.count() != 1 {
            return Err(match id {
                Some(id) => merge_in_the_way(id),
                None => invalid(
                    say!("interactive-what-range"),
                    say!("interactive-commit-unreadable"),
                ),
            });
        }
    }
    if count > MAX_COMMITS {
        return Err(refused(say!(
            "interactive-too-many",
            most = MAX_COMMITS,
            commit = first_id.short(7),
            branch = branch.as_str()
        )));
    }
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
    // What the upstream does not have yet.
    let unpublished: Option<HashSet<Oid>> = upstream.as_ref().and_then(|upstream| {
        let not = format!("^{upstream}");
        exe::run(repo, ["rev-list", span.as_str(), not.as_str()], None)
            .ok()
            .map(|out| {
                out.stdout
                    .lines()
                    .filter_map(|id| id.trim().parse().ok())
                    .collect()
            })
    });
    // One record per commit, ended by a NUL, which no message contains:
    // the id, the author, their address and the time on a line each, then
    // the message.
    let log = exe::run(
        repo,
        [
            "log",
            "-z",
            "--reverse",
            "--format=%H%n%an%n%ae%n%at%n%B",
            span.as_str(),
        ],
        None,
    )?;
    let mut commits = Vec::new();
    for record in log.stdout.split('\0') {
        if record.trim().is_empty() {
            continue;
        }
        let mut parts = record.splitn(5, '\n');
        let id: Oid = parts
            .next()
            .and_then(|id| id.trim().parse().ok())
            .ok_or_else(|| {
                invalid(
                    say!("interactive-what-range"),
                    say!("interactive-commit-unreadable"),
                )
            })?;
        let author = parts.next().unwrap_or("").to_owned();
        let email = parts.next().unwrap_or("").to_owned();
        let time = parts
            .next()
            .and_then(|time| time.trim().parse().ok())
            .unwrap_or(0);
        let message = tidy(parts.next().unwrap_or("")).to_owned();
        commits.push(Entry {
            id,
            summary: message.lines().next().unwrap_or("").to_owned(),
            message,
            author,
            email,
            time,
            published: unpublished
                .as_ref()
                .is_some_and(|unpublished| !unpublished.contains(&id)),
        });
    }
    if commits.len() != count {
        return Err(invalid(
            say!("interactive-what-range"),
            say!("interactive-commits-unreadable"),
        ));
    }
    Ok(Range {
        base,
        branch,
        upstream,
        commits,
    })
}

fn merge_in_the_way(id: Oid) -> Error {
    refused(say!("interactive-merge-in-way", commit = id.short(7)))
}

/// The plan that changes nothing: every commit kept, in order.
pub fn untouched(range: &Range) -> Vec<Step> {
    range
        .commits
        .iter()
        .map(|entry| Step {
            id: entry.id,
            action: Action::Pick,
            message: None,
        })
        .collect()
}

/// The plan that moves `run`, commits of `range` that follow one another
/// (named in any order), one place: towards HEAD (`up`) past the commit
/// after them, or away from it past the commit before them. Everything
/// else is kept as it is. Refused when they do not follow one another,
/// are not all in the range, or have nothing there to move past.
pub fn moved(range: &Range, run: &[Oid], up: bool) -> Result<Vec<Step>> {
    let mut steps = untouched(range);
    let mut at: Vec<usize> = Vec::new();
    for id in run {
        let Some(index) = steps.iter().position(|step| step.id == *id) else {
            return Err(invalid(
                say!("interactive-what-commits"),
                say!("interactive-not-in-range", commit = id.short(7)),
            ));
        };
        if at.contains(&index) {
            return Err(invalid(
                say!("interactive-what-commits"),
                say!("interactive-named-twice", commit = id.short(7)),
            ));
        }
        at.push(index);
    }
    at.sort_unstable();
    let (Some(&first), Some(&last)) = (at.first(), at.last()) else {
        return Err(invalid(
            say!("interactive-what-commits"),
            say!("interactive-none-named"),
        ));
    };
    if last.saturating_sub(first).saturating_add(1) != at.len() {
        return Err(invalid(
            say!("interactive-what-commits"),
            say!("interactive-not-consecutive"),
        ));
    }
    if up {
        // The commit after them comes before them.
        let Some(after) = last.checked_add(1).filter(|after| *after < steps.len()) else {
            return Err(refused(say!("interactive-nothing-after")));
        };
        let step = steps.remove(after);
        steps.insert(first, step);
    } else {
        // The commit before them comes after them.
        let Some(before) = first.checked_sub(1) else {
            return Err(refused(say!("interactive-nothing-before")));
        };
        let step = steps.remove(before);
        steps.insert(last, step);
    }
    Ok(steps)
}

/// A group of the plan while it is put together: a commit that is kept and
/// what is melted into it.
struct Group {
    from: Vec<Oid>,
    /// The messages that make up the group's message, in order.
    parts: Vec<String>,
    /// The message is not the first commit's own any more.
    rewritten: bool,
    stops: bool,
}

/// Check a plan against its range and say what it leads to: the commits
/// there will be, oldest first. Each has to be for exactly the commits of
/// the range, each once, in any order; the first commit that is kept
/// cannot be melted into anything; a reword needs a message.
pub fn outline(range: &Range, steps: &[Step]) -> Result<Vec<Resulting>> {
    Ok(groups(range, steps)?
        .into_iter()
        .map(|group| Resulting {
            from: group.from,
            message: group.parts.join("\n\n"),
            stops: group.stops,
        })
        .collect())
}

fn groups(range: &Range, steps: &[Step]) -> Result<Vec<Group>> {
    if steps.len() != range.commits.len() {
        return Err(invalid(
            say!("interactive-what-plan"),
            say!(
                "interactive-step-count",
                steps = steps.len(),
                commits = range.commits.len()
            ),
        ));
    }
    let mut left: HashSet<Oid> = range.commits.iter().map(|entry| entry.id).collect();
    let mut groups: Vec<Group> = Vec::new();
    for step in steps {
        if !left.remove(&step.id) {
            return Err(invalid(
                say!("interactive-what-plan"),
                say!("interactive-step-twice", commit = step.id.short(7)),
            ));
        }
        let own = range
            .commits
            .iter()
            .find(|entry| entry.id == step.id)
            .map(|entry| entry.message.as_str())
            .unwrap_or("");
        let given = match &step.message {
            Some(message) => {
                if message.len() > MAX_MESSAGE || message.contains('\0') {
                    return Err(invalid(
                        say!("interactive-what-plan"),
                        say!("interactive-message-unusable", commit = step.id.short(7)),
                    ));
                }
                Some(tidy(message))
            }
            None => None,
        };
        match step.action {
            Action::Drop => {}
            Action::Pick | Action::Edit => groups.push(Group {
                from: vec![step.id],
                parts: vec![own.to_owned()],
                rewritten: false,
                stops: step.action == Action::Edit,
            }),
            Action::Reword => {
                let message = given.unwrap_or("");
                if message.is_empty() {
                    return Err(invalid(
                        say!("interactive-what-plan"),
                        say!("interactive-reword-no-message", commit = step.id.short(7)),
                    ));
                }
                groups.push(Group {
                    from: vec![step.id],
                    parts: vec![message.to_owned()],
                    rewritten: message != own,
                    stops: false,
                });
            }
            Action::Squash | Action::Fixup => {
                let Some(group) = groups.last_mut() else {
                    return Err(invalid(
                        say!("interactive-what-plan"),
                        say!("interactive-squash-first", commit = step.id.short(7)),
                    ));
                };
                group.from.push(step.id);
                if step.action == Action::Squash {
                    let part = given.unwrap_or(own);
                    if !part.is_empty() {
                        group.parts.push(part.to_owned());
                        group.rewritten = true;
                    }
                }
            }
        }
    }
    Ok(groups)
}

/// A path in single quotes for the shell git runs editors and `exec` lines
/// with.
fn quoted(path: &std::path::Path) -> Result<String> {
    let text = path
        .to_str()
        .filter(|text| !text.contains(['\n', '\r', '\0']))
        .ok_or_else(|| refused(say!("interactive-path-unusable")))?;
    // Git's shell reads the line: inside a WSL distribution for a
    // repository there, elsewhere on Windows with `/` in a path, as git.
    let text = exe::path_for_git_shell(text);
    Ok(format!("'{}'", text.replace('\'', "'\\''")))
}

/// The todo list for a plan, and the message files it refers to (file
/// name and content). `dir` is where those files will be.
fn todo(
    range: &Range,
    steps: &[Step],
    dir: &std::path::Path,
) -> Result<(String, Vec<(String, String)>)> {
    let groups = groups(range, steps)?;
    // Where each group ends: after its last commit in the plan.
    let mut ends: Vec<(Oid, &Group)> = Vec::new();
    for group in &groups {
        if let Some(last) = group.from.last() {
            ends.push((*last, group));
        }
    }
    let mut out = String::new();
    let mut files = Vec::new();
    for step in steps {
        let hex = step.id.to_hex();
        let command = match step.action {
            Action::Pick | Action::Reword => "pick",
            Action::Edit => "edit",
            Action::Squash | Action::Fixup => "fixup",
            Action::Drop => "drop",
        };
        out.push_str(command);
        out.push(' ');
        out.push_str(&hex);
        out.push('\n');
        let ended = ends
            .iter()
            .find(|(last, _)| *last == step.id)
            .map(|(_, group)| *group);
        if let Some(group) = ended.filter(|group| group.rewritten) {
            let name = format!("message-{}", files.len());
            let path = quoted(&dir.join(&name))?;
            out.push_str("exec git commit --amend --quiet --file=");
            out.push_str(&path);
            out.push('\n');
            let mut message = group.parts.join("\n\n");
            message.push('\n');
            files.push((name, message));
        }
    }
    Ok((out, files))
}

/// Where the todo list and the messages of a plan are kept while git works
/// through it.
pub(crate) fn plan_dir(repo: &Repository) -> std::path::PathBuf {
    repo.git_dir().join("bitbraid").join("rebase")
}

/// Carry out a plan for the commits from `first` up to HEAD. A conflict,
/// or a commit that is to be edited, leaves the rebase in progress, and the
/// result says so.
pub fn run(repo: &Repository, first: &str, steps: &[Step]) -> Result<Moved> {
    run_onto(repo, first, steps, None)
}

/// The oldest commit the current branch has that `onto` does not: where a
/// plan begins that moves the branch's own commits onto `onto`.
pub fn first_after(repo: &Repository, onto: &str) -> Result<Oid> {
    let base = rewrite::commit_of(repo, say!("interactive-what-new-base"), onto)?;
    let span = format!("{}..HEAD", base.to_hex());
    // Counted first: the list is read whole, and a branch that is far
    // from the base is refused by [`range`] anyway.
    let counted = exe::run(repo, ["rev-list", "--count", span.as_str()], None)?;
    let count: usize = counted.stdout.trim().parse().unwrap_or(0);
    if count == 0 {
        return Err(refused(say!("interactive-nothing-to-rebase", onto)));
    }
    if count > MAX_COMMITS.saturating_mul(4) {
        return Err(refused(say!("interactive-too-far", count, onto)));
    }
    let listed = exe::run(
        repo,
        ["rev-list", "--first-parent", "--reverse", span.as_str()],
        None,
    )?;
    listed
        .stdout
        .lines()
        .next()
        .and_then(|id| id.trim().parse().ok())
        .ok_or_else(|| {
            invalid(
                say!("interactive-what-range"),
                say!("interactive-first-unreadable"),
            )
        })
}

/// [`run`], with the commits replayed on `onto` instead of where they
/// are: an interactive rebase onto another base.
pub fn run_onto(
    repo: &Repository,
    first: &str,
    steps: &[Step],
    onto: Option<&str>,
) -> Result<Moved> {
    run_onto_with(repo, first, steps, onto, false)
}

/// [`run_onto`], and with `no_verify` without the `pre-rebase` hook
/// (`git rebase --no-verify`). The hooks of the commits it makes run all
/// the same.
pub fn run_onto_with(
    repo: &Repository,
    first: &str,
    steps: &[Step],
    onto: Option<&str>,
    no_verify: bool,
) -> Result<Moved> {
    let range = range(repo, first)?;
    let onto = match onto {
        Some(onto) => Some(rewrite::commit_of(
            repo,
            say!("interactive-what-new-base"),
            onto,
        )?),
        None => None,
    }
    // Onto where they are is no other base.
    .filter(|onto| *onto != range.base);
    let dir = plan_dir(repo);
    let (list, files) = todo(&range, steps, &dir)?;
    rewrite::ensure_idle(repo, say!("interactive-what"))?;
    let before = branch::head_state(repo)?;
    if onto.is_none() && steps == untouched(&range).as_slice() {
        // Nothing to do, and git would say as much.
        return rewrite::moved(repo, before, Vec::new(), None);
    }
    let io =
        |err: std::io::Error| refused(say!("interactive-plan-unwritable", err = err.to_string()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(io)?;
    for (name, message) in &files {
        std::fs::write(dir.join(name), message).map_err(io)?;
    }
    let list_path = dir.join("todo");
    std::fs::write(&list_path, list).map_err(io)?;
    let editor = format!("cp {}", quoted(&list_path)?);
    let base = range.base.to_hex();
    let new_base = onto.map(|onto| onto.to_hex());
    let mut args = vec![
        "rebase",
        "--interactive",
        "--empty=drop",
        "--no-autosquash",
        "--reschedule-failed-exec",
    ];
    if no_verify {
        args.push("--no-verify");
    }
    if let Some(new_base) = &new_base {
        args.extend(["--onto", new_base.as_str()]);
    }
    args.push(base.as_str());
    carry_out(repo, &args, &editor, before, say!("interactive-what"))
}

/// Start git's interactive rebase with `args`, its list put in place by
/// `editor`, and say where that left HEAD: at the end, or stopped on a
/// conflict or at a commit that is to be edited. The files of the plan are
/// removed when it ran to its end.
pub(crate) fn carry_out(
    repo: &Repository,
    args: &[&str],
    editor: &str,
    before: branch::HeadState,
    what: &'static str,
) -> Result<Moved> {
    let dir = plan_dir(repo);
    // With a watch on the hooks: a rebase runs `pre-rebase`, and a reword
    // the hooks of a commit.
    let result = exe::run_hooked_with_env(repo, args, &[("GIT_SEQUENCE_EDITOR", editor)], None);
    let stopped = match result {
        // Git is content and the rebase is still there: it stopped at a
        // commit that is to be edited.
        Ok(_) => crate::sequence::in_progress(repo)?.map(|operation| {
            let at = operation
                .current
                .map(|id| id.short(7))
                .unwrap_or_else(|| say!("interactive-a-commit").to_owned());
            say!("interactive-stopped-to-edit", at)
        }),
        Err(err) => match rewrite::stopped_or_failed(repo, OnStop::Leave, what, err) {
            Ok(stopped) => stopped,
            Err(err) => {
                // Nothing was begun: nothing of the plan is left behind.
                let _ = std::fs::remove_dir_all(&dir);
                return Err(err);
            }
        },
    };
    if stopped.is_none() {
        // The message files are needed for as long as the rebase lasts.
        let _ = std::fs::remove_dir_all(&dir);
    }
    rewrite::moved(repo, before, Vec::new(), stopped)
}

#[cfg(test)]
mod tests {
    use super::{Action, Entry, Range, Step, moved, outline, quoted, todo, untouched};
    use crate::Oid;

    fn id(byte: u8) -> Oid {
        format!("{byte:02x}").repeat(20).parse().unwrap()
    }

    fn range(messages: &[&str]) -> Range {
        Range {
            base: id(0),
            branch: "main".to_owned(),
            upstream: None,
            commits: messages
                .iter()
                .zip(1u8..)
                .map(|(message, n)| Entry {
                    id: id(n),
                    summary: message.lines().next().unwrap_or("").to_owned(),
                    message: (*message).to_owned(),
                    author: "Ada".to_owned(),
                    email: "ada@example.invalid".to_owned(),
                    time: 0,
                    published: false,
                })
                .collect(),
        }
    }

    fn step(n: u8, action: Action, message: Option<&str>) -> Step {
        Step {
            id: id(n),
            action,
            message: message.map(str::to_owned),
        }
    }

    #[test]
    fn a_plan_is_checked_against_its_commits() {
        let range = range(&["one", "two", "three"]);
        let fine = untouched(&range);
        assert_eq!(outline(&range, &fine).unwrap().len(), 3);
        // Reordered is fine.
        let mut turned = fine.clone();
        turned.reverse();
        let result = outline(&range, &turned).unwrap();
        assert_eq!(result[0].message, "three");
        // Too few, too many, one twice, one from elsewhere.
        assert!(outline(&range, &fine[..2]).is_err());
        let mut twice = fine.clone();
        twice[2] = twice[0].clone();
        assert!(outline(&range, &twice).is_err());
        let mut foreign = fine.clone();
        foreign[1].id = id(9);
        assert!(outline(&range, &foreign).is_err());
        assert!(outline(&range, &[]).is_err());
        // Nothing before the first kept commit to melt it into.
        for action in [Action::Squash, Action::Fixup] {
            let mut plan = fine.clone();
            plan[0].action = action;
            assert!(outline(&range, &plan).is_err(), "{action:?} first");
            // Not even behind a dropped one.
            plan[0].action = Action::Drop;
            plan[1].action = action;
            let err = outline(&range, &plan).unwrap_err().to_string();
            assert!(err.contains("nothing before it"), "{err}");
        }
        // A reword says what to.
        for message in [None, Some(""), Some(" \n\t\n")] {
            let mut plan = fine.clone();
            plan[1] = step(2, Action::Reword, message);
            assert!(outline(&range, &plan).is_err(), "{message:?}");
        }
        let mut plan = fine.clone();
        plan[1] = step(2, Action::Reword, Some("with a \0 in it"));
        assert!(outline(&range, &plan).is_err());
        plan[1] = step(2, Action::Reword, Some(&"x".repeat((1 << 20) + 1)));
        assert!(outline(&range, &plan).is_err());
        // Everything dropped leaves nothing, which is a plan too.
        let none: Vec<Step> = (1..=3).map(|n| step(n, Action::Drop, None)).collect();
        assert!(outline(&range, &none).unwrap().is_empty());
    }

    #[test]
    fn the_outline_says_what_there_will_be() {
        let range = range(&["one\n\nbody of one", "two", "three", "#4 four", "five"]);
        let plan = [
            step(1, Action::Reword, Some("\nONE\n\n")),
            step(2, Action::Squash, None),
            step(3, Action::Drop, None),
            step(4, Action::Fixup, None),
            step(5, Action::Edit, None),
        ];
        let result = outline(&range, &plan).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].from, [id(1), id(2), id(4)]);
        assert_eq!(result[0].message, "ONE\n\ntwo");
        assert!(!result[0].stops);
        assert_eq!(result[1].from, [id(5)]);
        assert_eq!(result[1].message, "five");
        assert!(result[1].stops);

        let dir = std::path::Path::new("/r/.git/bitbraid/rebase");
        let (list, files) = todo(&range, &plan, dir).unwrap();
        assert_eq!(
            list,
            format!(
                "pick {}\nfixup {}\ndrop {}\nfixup {}\nexec git commit --amend --quiet \
                 --file='/r/.git/bitbraid/rebase/message-0'\nedit {}\n",
                id(1).to_hex(),
                id(2).to_hex(),
                id(3).to_hex(),
                id(4).to_hex(),
                id(5).to_hex()
            )
        );
        assert_eq!(files, [("message-0".to_owned(), "ONE\n\ntwo\n".to_owned())]);

        // A squash with words of its own, and one that adds nothing.
        let plan = [
            step(1, Action::Pick, None),
            step(2, Action::Squash, Some("and the second")),
            step(3, Action::Squash, Some("  \n")),
            step(4, Action::Pick, None),
            step(5, Action::Reword, Some("five")),
        ];
        let result = outline(&range, &plan).unwrap();
        assert_eq!(result[0].message, "one\n\nbody of one\n\nand the second");
        // A message that begins like a comment is a message.
        assert_eq!(result[1].message, "#4 four");
        let (list, files) = todo(&range, &plan, dir).unwrap();
        // The reword to the same words is a pick: one file, for the squash.
        assert_eq!(files.len(), 1);
        assert_eq!(list.matches("exec ").count(), 1);
        assert!(list.ends_with(&format!("pick {}\n", id(5).to_hex())));

        // Fixups alone leave the message be.
        let plan = [
            step(1, Action::Pick, None),
            step(2, Action::Fixup, None),
            step(3, Action::Fixup, None),
            step(4, Action::Pick, None),
            step(5, Action::Pick, None),
        ];
        let (list, files) = todo(&range, &plan, dir).unwrap();
        assert!(files.is_empty());
        assert!(!list.contains("exec"));
    }

    #[test]
    fn paths_are_quoted_for_the_shell() {
        let path = std::path::Path::new("/tmp/it's here/todo");
        assert_eq!(quoted(path).unwrap(), "'/tmp/it'\\''s here/todo'");
        assert!(quoted(std::path::Path::new("/tmp/two\nlines")).is_err());
    }

    #[test]
    fn a_run_of_commits_moves_one_place_up_or_down() {
        let range = range(&["one", "two", "three", "four", "five"]);
        let order = |steps: Vec<Step>| -> Vec<u8> {
            steps
                .iter()
                .map(|step| {
                    assert_eq!(step.action, Action::Pick);
                    assert_eq!(step.message, None);
                    (1u8..=5).find(|n| id(*n) == step.id).unwrap()
                })
                .collect()
        };
        // One commit, up past the next and down past the one before.
        assert_eq!(
            order(moved(&range, &[id(2)], true).unwrap()),
            [1, 3, 2, 4, 5]
        );
        assert_eq!(
            order(moved(&range, &[id(2)], false).unwrap()),
            [2, 1, 3, 4, 5]
        );
        // A run, named in any order.
        assert_eq!(
            order(moved(&range, &[id(4), id(2), id(3)], true).unwrap()),
            [1, 5, 2, 3, 4]
        );
        assert_eq!(
            order(moved(&range, &[id(3), id(2)], false).unwrap()),
            [2, 3, 1, 4, 5]
        );
        // Nothing to move past, not one after another, twice, not there,
        // or none at all: refused.
        assert!(moved(&range, &[id(5)], true).is_err());
        assert!(moved(&range, &[id(4), id(5)], true).is_err());
        assert!(moved(&range, &[id(1)], false).is_err());
        assert!(moved(&range, &[id(2), id(4)], true).is_err());
        assert!(moved(&range, &[id(2), id(2)], true).is_err());
        assert!(moved(&range, &[id(9)], false).is_err());
        assert!(moved(&range, &[], true).is_err());
        assert!(
            moved(
                &super::Range {
                    commits: Vec::new(),
                    ..range.clone()
                },
                &[id(1)],
                true
            )
            .is_err()
        );
    }
}
