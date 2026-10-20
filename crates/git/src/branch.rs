//! Branches and checkout, through the git executable: reflogs, `branch.*`
//! configuration, worktree checks, filters and hooks all behave as in the
//! user's shell.
//!
//! Nothing a caller passes sits where git reads options: names starting
//! with a dash are refused, and every name is checked the way git checks
//! it before a command runs.
//!
//! Sources: git-branch(1), git-switch(1), git-check-ref-format(1),
//! git-stash(1), git-rev-parse(1), git-symbolic-ref(1),
//! git-for-each-ref(1) (`upstream:track`).

use serde::{Deserialize, Serialize};

use crate::{Backup, Error, Oid, Repository, Result, exe};

/// Where HEAD is: on a branch, or detached at a commit, or on a branch
/// without commits yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct HeadState {
    /// Short branch name when HEAD is on a branch.
    pub branch: Option<String>,
    /// The commit, absent on an unborn branch.
    pub commit: Option<Oid>,
}

/// What to do with local changes that a checkout would overwrite.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum Dirty {
    /// Let git decide: it carries changes along when it can and refuses
    /// when it would have to overwrite them.
    #[default]
    Refuse,
    /// Stash everything, switch, and bring the stash back.
    Stash,
    /// Throw the changes away (after backing them up).
    Discard,
}

/// What a checkout did.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CheckoutOutcome {
    /// HEAD before.
    pub before: HeadState,
    /// HEAD after.
    pub after: HeadState,
    /// A local branch that was created to track a remote one.
    pub created: Option<String>,
    /// Changes were stashed and could not be brought back cleanly; they are
    /// still in the stash.
    pub stash_kept: bool,
    /// Backups of files whose changes were thrown away.
    pub backups: Vec<Backup>,
    /// The `post-checkout` hook ended badly, after the checkout had
    /// happened: what there is to say about that.
    #[serde(default)]
    pub hook: Option<String>,
}

/// A deleted branch, with what is needed to bring it back.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DeletedBranch {
    /// Short name.
    pub name: String,
    /// The commit it pointed at.
    pub target: Oid,
    /// Its upstream as `remote/branch`, when it had one.
    pub upstream: Option<String>,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

/// Refuse anything git could read as an option or as more than one line.
fn check_plain(what: &'static str, text: &str) -> Result<()> {
    if text.is_empty() {
        return Err(invalid(what, say!("branch-empty")));
    }
    if text.starts_with('-') {
        return Err(invalid(
            what,
            say!("branch-starts-with-dash", text = format!("{text:?}")),
        ));
    }
    if text.chars().any(char::is_control) {
        return Err(invalid(
            what,
            say!("branch-control-characters", text = format!("{text:?}")),
        ));
    }
    Ok(())
}

/// Check a branch name the way git does, as a literal name. `@{-1}` and its
/// kind are refused rather than expanded: `check-ref-format --branch` would
/// turn them into whatever branch was checked out before, and a rename or
/// delete would then hit that branch.
pub fn check_branch_name(repo: &Repository, name: &str) -> Result<()> {
    check_plain(say!("branch-what-branch-name"), name)?;
    let bad = || {
        invalid(
            say!("branch-what-branch-name"),
            say!("branch-not-valid-name", name = format!("{name:?}")),
        )
    };
    if name == "HEAD" || name == "@" || name.contains("@{") {
        return Err(bad());
    }
    let reference = format!("refs/heads/{name}");
    exe::run(repo, ["check-ref-format", reference.as_str()], None)
        .map(|_| ())
        .map_err(|_| bad())
}

/// The commit a revision names, or nothing.
pub(crate) fn resolve(repo: &Repository, rev: &str) -> Option<Oid> {
    check_plain(say!("branch-what-revision"), rev).ok()?;
    let spec = format!("{rev}^{{commit}}");
    let out = exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", spec.as_str()],
        None,
    )
    .ok()?;
    out.stdout.trim().parse().ok()
}

/// The commit a revision names (a branch, a tag, an id, `HEAD~2`), or an
/// error that says it names none.
pub fn commit_of(repo: &Repository, rev: &str) -> Result<Oid> {
    check_plain(say!("branch-what-revision"), rev)?;
    resolve(repo, rev).ok_or_else(|| {
        invalid(
            say!("branch-what-revision"),
            say!("branch-names-no-commit", rev = format!("{rev:?}")),
        )
    })
}

/// The commit a local branch points at, or nothing when there is no such
/// branch.
pub fn branch_tip(repo: &Repository, name: &str) -> Option<Oid> {
    resolve(repo, &format!("refs/heads/{name}"))
}

/// Where HEAD is now.
pub fn head_state(repo: &Repository) -> Result<HeadState> {
    let branch = exe::run(repo, ["symbolic-ref", "--quiet", "--short", "HEAD"], None)
        .ok()
        .map(|out| out.stdout.trim().to_owned())
        .filter(|name| !name.is_empty());
    Ok(HeadState {
        branch,
        commit: resolve(repo, "HEAD"),
    })
}

/// The upstream of a local branch as `remote/branch`.
pub fn upstream_of(repo: &Repository, name: &str) -> Option<String> {
    check_plain(say!("branch-what-branch-name"), name).ok()?;
    let spec = format!("{name}@{{upstream}}");
    exe::run(
        repo,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            spec.as_str(),
        ],
        None,
    )
    .ok()
    .map(|out| out.stdout.trim().to_owned())
    .filter(|upstream| !upstream.is_empty())
}

/// Create a branch at `start` (HEAD when absent). Returns its commit.
pub fn branch_create(repo: &Repository, name: &str, start: Option<&str>) -> Result<Oid> {
    check_branch_name(repo, name)?;
    let start = start.unwrap_or("HEAD");
    let target = resolve(repo, start).ok_or_else(|| {
        invalid(
            say!("branch-what-start-point"),
            say!("branch-start-not-commit", start = format!("{start:?}")),
        )
    })?;
    // No tracking unless asked for: a branch made from a remote branch by
    // its commit is a plain local branch.
    exe::run(
        repo,
        ["branch", "--no-track", "--", name, target.to_hex().as_str()],
        None,
    )?;
    Ok(target)
}

/// Rename a local branch.
pub fn branch_rename(repo: &Repository, name: &str, new_name: &str) -> Result<()> {
    check_branch_name(repo, name)?;
    check_branch_name(repo, new_name)?;
    exe::run(repo, ["branch", "-m", "--", name, new_name], None)?;
    Ok(())
}

/// Delete a local branch. Without `force` git refuses a branch that is not
/// merged into its upstream or HEAD; the checked-out branch is always
/// refused.
pub fn branch_delete(repo: &Repository, name: &str, force: bool) -> Result<DeletedBranch> {
    check_branch_name(repo, name)?;
    let target = branch_tip(repo, name).ok_or_else(|| {
        invalid(
            say!("branch-what-branch"),
            say!("branch-does-not-exist", name = format!("{name:?}")),
        )
    })?;
    let upstream = upstream_of(repo, name);
    let flag = if force { "-D" } else { "-d" };
    exe::run(repo, ["branch", flag, "--", name], None)?;
    Ok(DeletedBranch {
        name: name.to_owned(),
        target,
        upstream,
    })
}

/// Set the upstream of a local branch to `remote/branch`, or clear it.
/// Returns what it was before.
pub fn branch_set_upstream(
    repo: &Repository,
    name: &str,
    upstream: Option<&str>,
) -> Result<Option<String>> {
    check_branch_name(repo, name)?;
    let before = upstream_of(repo, name);
    match upstream {
        Some(upstream) => {
            check_plain(say!("branch-what-upstream"), upstream)?;
            let option = format!("--set-upstream-to={upstream}");
            exe::run(repo, ["branch", option.as_str(), "--", name], None)?;
        }
        None if before.is_some() => {
            exe::run(repo, ["branch", "--unset-upstream", "--", name], None)?;
        }
        None => {}
    }
    Ok(before)
}

/// How far a local branch is from its upstream.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Tracking {
    /// The branch, by its short name.
    pub name: String,
    /// Commits it has that its upstream lacks.
    pub ahead: u32,
    /// Commits its upstream has that it lacks.
    pub behind: u32,
    /// The upstream it is configured with is not there any more.
    pub gone: bool,
}

/// One number out of "ahead 3, behind 1".
fn tracked(text: &str, word: &str) -> u32 {
    text.split(", ")
        .find_map(|part| part.strip_prefix(word))
        .and_then(|count| count.trim().parse().ok())
        .unwrap_or(0)
}

/// Every local branch that has an upstream and is not level with it: how
/// many commits it is ahead and behind, as git counts them.
pub fn tracking(repo: &Repository) -> Result<Vec<Tracking>> {
    let out = exe::run(
        repo,
        [
            "for-each-ref",
            "--format=%(refname)%00%(upstream:track,nobracket)",
            "refs/heads",
        ],
        None,
    )?;
    Ok(out
        .stdout
        .lines()
        .filter_map(|line| {
            let (name, track) = line.split_once('\0')?;
            let name = name.strip_prefix("refs/heads/")?;
            let track = track.trim();
            if track.is_empty() {
                return None;
            }
            Some(Tracking {
                name: name.to_owned(),
                ahead: tracked(track, "ahead "),
                behind: tracked(track, "behind "),
                gone: track == "gone",
            })
        })
        .collect())
}

/// Move a local branch that is not checked out forward to its upstream,
/// or to the commit `to` names: the branch moves there if it is behind it
/// and has nothing of its own. Returns where it was and where it is now;
/// the two are the same when it was up to date. Refused when the branch is
/// checked out somewhere (pull there instead), has no upstream, or has
/// commits its upstream lacks.
pub fn fast_forward_to(repo: &Repository, name: &str, to: Option<&str>) -> Result<(Oid, Oid)> {
    check_branch_name(repo, name)?;
    let refuse = |detail: String| Error::Refused {
        what: say!("branch-what-fast-forward"),
        detail,
    };
    let before = branch_tip(repo, name).ok_or_else(|| refuse(say!("branch-no-branch", name)))?;
    let (upstream, after) = match to {
        Some(to) => (to.to_owned(), commit_of(repo, to)?),
        None => {
            let upstream =
                upstream_of(repo, name).ok_or_else(|| refuse(say!("branch-no-upstream", name)))?;
            // By the branch's own configuration, not by the short name,
            // which a local branch or a tag could share.
            let after = resolve(repo, &format!("{name}@{{upstream}}")).ok_or_else(|| {
                refuse(say!(
                    "branch-upstream-missing",
                    upstream = upstream.as_str()
                ))
            })?;
            (upstream, after)
        }
    };
    // Moving a branch under a worktree that has it checked out would leave
    // that worktree looking as if everything had been reverted.
    let reference = format!("refs/heads/{name}");
    let held = exe::run(
        repo,
        [
            "for-each-ref",
            "--format=%(worktreepath)",
            reference.as_str(),
        ],
        None,
    )?;
    let held = held.stdout.trim();
    if !held.is_empty() {
        return Err(refuse(say!("branch-checked-out-elsewhere", name, held)));
    }
    if before == after {
        return Ok((before, after));
    }
    let (old, new) = (before.to_hex(), after.to_hex());
    if exe::run(
        repo,
        ["merge-base", "--is-ancestor", old.as_str(), new.as_str()],
        None,
    )
    .is_err()
    {
        return Err(refuse(say!("branch-has-own-commits", name, upstream)));
    }
    branch_move(repo, name, Some(after), Some(before))?;
    Ok((before, after))
}

/// Move a local branch to `new` only if it still points at `expected`
/// (absent: only if it does not exist). Used by undo and redo.
pub fn branch_move(
    repo: &Repository,
    name: &str,
    new: Option<Oid>,
    expected: Option<Oid>,
) -> Result<()> {
    check_branch_name(repo, name)?;
    let reference = format!("refs/heads/{name}");
    let zero = "0".repeat(40);
    let expected = expected.map(|id| id.to_hex()).unwrap_or(zero);
    match new {
        Some(new) => exe::run(
            repo,
            [
                "update-ref",
                "-m",
                "bitbraid: undo/redo",
                reference.as_str(),
                new.to_hex().as_str(),
                expected.as_str(),
            ],
            None,
        )?,
        None => exe::run(
            repo,
            ["update-ref", "-d", reference.as_str(), expected.as_str()],
            None,
        )?,
    };
    Ok(())
}

/// Delete a local branch with its configuration, but only if it still
/// points at `expected`. Used by undo, which must not take away a branch
/// that got commits since.
pub fn branch_delete_if_at(repo: &Repository, name: &str, expected: Oid) -> Result<()> {
    check_branch_name(repo, name)?;
    if branch_tip(repo, name) != Some(expected) {
        return Err(invalid(
            say!("branch-what-branch"),
            say!("branch-moved", name = format!("{name:?}")),
        ));
    }
    exe::run(repo, ["branch", "-D", "--", name], None)?;
    Ok(())
}

/// After a `stash pop` that stopped on a conflict: the stash is still
/// there with everything in it, so put the worktree back to a clean
/// checkout instead of leaving conflict markers and half of the changes.
/// Untracked files the pop brought back are removed again, those and no
/// others.
fn clean_up_failed_pop(repo: &Repository) {
    let restored = exe::run(
        repo,
        ["show", "--name-only", "--format=", "-z", "stash@{0}^3"],
        None,
    )
    .map(|out| out.stdout)
    .unwrap_or_default();
    let _ = exe::run(repo, ["reset", "--hard", "--quiet"], None);
    if let Some(workdir) = repo.workdir() {
        for path in restored.split('\0').filter(|path| !path.is_empty()) {
            if crate::worktree_path(repo, path).is_ok() {
                let _ = std::fs::remove_file(workdir.join(path));
            }
        }
    }
}

/// Run `git switch`, and take a switch that happened for one that
/// happened. The `post-checkout` hook runs after HEAD and the files have
/// moved, and its exit status becomes git's (githooks(5)): a hook that
/// ends badly there (git-lfs's, where git-lfs is no longer installed)
/// would otherwise turn every checkout into a failure that has moved
/// HEAD and left no entry to undo it by. What the hook said is in the
/// activity log, under the command that ran it.
fn switch(repo: &Repository, args: &[&str]) -> Result<Option<String>> {
    match exe::run_hooked(repo, args, None) {
        Ok(_) => Ok(None),
        // The checkout happened; what the hook had against it afterwards
        // is for whoever asked to be told.
        Err(err @ Error::Hook { after: true, .. }) => Ok(Some(err.to_string())),
        Err(err) => Err(err),
    }
}

/// Put the local changes in a stash to carry them over a switch, untracked
/// files too. Whether a stash was made: with nothing to stash there is
/// none.
fn stash_to_carry(repo: &Repository) -> Result<bool> {
    let stashes = |repo: &Repository| {
        exe::run(repo, ["stash", "list"], None)
            .map(|out| out.stdout.lines().count())
            .unwrap_or(0)
    };
    let had = stashes(repo);
    exe::run(
        repo,
        [
            "stash",
            "push",
            "--include-untracked",
            "-m",
            "bitbraid: changes carried over a checkout",
        ],
        None,
    )?;
    Ok(stashes(repo) > had)
}

/// Bring the carried changes back after the switch (also when it did not
/// happen). Whether they did not fit and stay in the stash, the worktree
/// a clean checkout.
fn bring_back(repo: &Repository) -> bool {
    let kept = exe::run(repo, ["stash", "pop"], None).is_err();
    if kept {
        clean_up_failed_pop(repo);
    }
    kept
}

/// Go back to where a checkout started from, which is how it is undone:
/// with the local changes as the checkout treated them. When it carried
/// them in a stash ([`Dirty::Stash`]) they are carried back the same way,
/// which git's plain switch refuses where the two branches differ in a
/// file that has them; otherwise as [`switch_to`] goes. Whether they were
/// carried and did not fit, so that they stay in the stash.
pub fn switch_back(repo: &Repository, state: &HeadState, dirty: Dirty) -> Result<bool> {
    let stashed = dirty == Dirty::Stash && stash_to_carry(repo)?;
    let switched = switch_to(repo, state);
    let kept = stashed && bring_back(repo);
    switched?;
    Ok(kept)
}

/// Switch HEAD to a state recorded earlier: a branch, or a commit detached.
pub fn switch_to(repo: &Repository, state: &HeadState) -> Result<()> {
    match (&state.branch, state.commit) {
        (Some(branch), _) => {
            check_branch_name(repo, branch)?;
            switch(repo, &["switch", "--", branch.as_str()])?;
        }
        (None, Some(commit)) => {
            switch(repo, &["switch", "--detach", commit.to_hex().as_str()])?;
        }
        (None, None) => {
            return Err(invalid(
                say!("branch-what-checkout"),
                say!("branch-nowhere-back"),
            ));
        }
    }
    Ok(())
}

/// What a name to check out turns out to be.
enum Target {
    /// An existing local branch.
    Local(String),
    /// A remote branch without a local one of its name: create and track.
    Track { local: String, remote: String },
    /// Anything else that names a commit: detach.
    Detach(Oid),
}

fn classify(repo: &Repository, target: &str) -> Result<Target> {
    check_plain(say!("branch-what-checkout-target"), target)?;
    if branch_tip(repo, target).is_some() {
        return Ok(Target::Local(target.to_owned()));
    }
    let remote_ref = format!("refs/remotes/{target}");
    if resolve(repo, &remote_ref).is_some()
        && let Some((_, local)) = target.split_once('/')
    {
        return Ok(match branch_tip(repo, local) {
            Some(_) => Target::Local(local.to_owned()),
            None => Target::Track {
                local: local.to_owned(),
                remote: target.to_owned(),
            },
        });
    }
    resolve(repo, target).map(Target::Detach).ok_or_else(|| {
        invalid(
            say!("branch-what-checkout-target"),
            say!(
                "branch-nothing-to-check-out",
                target = format!("{target:?}")
            ),
        )
    })
}

/// Check out `target`: a local branch, a remote branch (creating the local
/// branch that tracks it, or switching to the one that exists), or any
/// commit or tag, detached.
pub fn checkout(repo: &Repository, target: &str, dirty: Dirty) -> Result<CheckoutOutcome> {
    let before = head_state(repo)?;
    let target = classify(repo, target)?;

    let mut backups = Vec::new();
    let mut stashed = false;
    match dirty {
        Dirty::Refuse => {}
        Dirty::Stash => stashed = stash_to_carry(repo)?,
        Dirty::Discard => {
            let changed: Vec<String> = crate::status(repo)?
                .into_iter()
                .filter(|entry| entry.staged.is_some() || entry.unstaged.is_some())
                .filter(|entry| entry.unstaged != Some(crate::ChangeKind::Untracked))
                .map(|entry| entry.path)
                .collect();
            backups = crate::stage::backup_paths(repo, &changed)?;
        }
    }

    let mut created = None;
    let discard = (dirty == Dirty::Discard).then_some("--discard-changes");
    let switched = match &target {
        Target::Local(name) => {
            let mut args = vec!["switch"];
            args.extend(discard);
            args.extend(["--", name.as_str()]);
            switch(repo, &args)
        }
        Target::Track { local, remote } => {
            check_branch_name(repo, local)?;
            created = Some(local.clone());
            let mut args = vec!["switch"];
            args.extend(discard);
            args.extend(["--track", "-c", local.as_str(), "--", remote.as_str()]);
            switch(repo, &args)
        }
        Target::Detach(commit) => {
            let hex = commit.to_hex();
            let mut args = vec!["switch"];
            args.extend(discard);
            args.extend(["--detach", hex.as_str()]);
            switch(repo, &args)
        }
    };

    // Bring the changes back, also when the switch did not happen.
    let stash_kept = stashed && bring_back(repo);
    let hook = switched?;
    Ok(CheckoutOutcome {
        before,
        after: head_state(repo)?,
        created,
        stash_kept,
        backups,
        hook,
    })
}

/// What is wrong with `name` as the name of a branch, by git's rules for
/// reference names (git-check-ref-format(1)), in words for whoever is
/// typing it; nothing when git would take it, and nothing for an empty
/// name, which is not a name yet. It asks no git: [`check_branch_name`]
/// is what a command goes by, and a test holds the two together.
pub fn name_problem(name: &str) -> Option<String> {
    if name.is_empty() {
        return None;
    }
    let problem = |text: &str| Some(text.to_owned());
    if name.starts_with('-') {
        return problem(say!("branch-name-begins-with-dash"));
    }
    if name == "HEAD" || name == "@" {
        return Some(say!("branch-name-reserved", name));
    }
    if name.contains("@{") {
        return problem(say!("branch-name-at-brace"));
    }
    if name.contains("..") {
        return problem(say!("branch-name-two-dots"));
    }
    if let Some(bad) = name
        .chars()
        .find(|ch| ch.is_control() || " ~^:?*[\\".contains(*ch))
    {
        return Some(match bad {
            ' ' => say!("branch-name-space").to_owned(),
            bad if bad.is_control() => say!("branch-name-control").to_owned(),
            bad => say!("branch-name-character", bad = bad.to_string()),
        });
    }
    if name.starts_with('/') || name.ends_with('/') || name.contains("//") {
        return problem(say!("branch-name-slash"));
    }
    if name.ends_with('.') {
        return problem(say!("branch-name-ends-with-dot"));
    }
    if name.split('/').any(|part| part.starts_with('.')) {
        return problem(say!("branch-name-part-dot"));
    }
    if name.split('/').any(|part| part.ends_with(".lock")) {
        return problem(say!("branch-name-part-lock"));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::name_problem;

    #[test]
    fn a_name_is_judged_as_git_judges_it() {
        let long = "n".repeat(200);
        let names = [
            "main",
            "feature/x",
            "a.b",
            "a..b",
            ".hidden",
            "a/.b",
            "x.lock",
            "a.lock/b",
            "a.lockb",
            "lock",
            "a/",
            "/a",
            "a//b",
            "a b",
            "a~b",
            "a^b",
            "a:b",
            "a?b",
            "a*b",
            "a[b",
            "a]b",
            "a\\b",
            "a.",
            ".",
            "..",
            "a/./b",
            "@x",
            "x@y",
            "a@{b",
            "HEAD",
            "head",
            "-x",
            "x-",
            "a\tb",
            "a\u{7f}b",
            "a\u{1}b",
            "ü",
            "héllo/wörld",
            "refs/heads/x",
            "1",
            "a,b",
            "a'b\"c",
            "a{b}",
            "a@",
            long.as_str(),
        ];
        let dir = std::env::temp_dir();
        for name in names {
            let git_takes_it =
                crate::exe::run_in(&dir, ["check-ref-format", "--branch", name]).is_ok();
            assert_eq!(
                name_problem(name).is_none(),
                git_takes_it,
                "{name:?}: {:?}",
                name_problem(name)
            );
        }
        // `@` alone git's check lets through as a branch name, and then
        // reads it as HEAD wherever it is used: refused here, as
        // `check_branch_name` refuses it.
        assert!(name_problem("@").is_some());
        // Not a name yet: nothing to say.
        assert_eq!(name_problem(""), None);
        // What is said names what is wrong.
        assert_eq!(
            name_problem("a b").as_deref(),
            Some("The name cannot contain a space.")
        );
        assert_eq!(
            name_problem("a~b").as_deref(),
            Some("The name cannot contain ~")
        );
        assert_eq!(
            name_problem("HEAD").as_deref(),
            Some("HEAD is not a name it can have.")
        );
    }
}
