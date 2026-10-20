//! Commit messages as text: trailers added by git's own rules, the
//! trailer keys a repository uses, and the messages of the commits that
//! were made here lately.
//!
//! Trailers are added by `git interpret-trailers`, so that where a trailer
//! goes, what happens when it is there already, and what the repository's
//! `trailer.*` configuration says are git's doing and not a copy of it.
//! The message is treated whole (`--no-divider`): a line of dashes in a
//! commit message is text, not the start of a patch.
//!
//! The messages made here come from the reflog of `HEAD`: every commit,
//! amend, merge and cherry-pick that was made in this worktree, also the
//! ones that were undone or amended away since, which is what a message is
//! wanted back from. Checkouts, resets and pulls are not commits made here
//! and are left out.
//!
//! Sources: git-interpret-trailers(1), git-log(1) (`-g`, `%gs`,
//! `%(trailers)`), git-reflog(1), git-config(1) (`trailer.<token>.key`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, exe};

/// One trailer to add: `Key: value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Trailer {
    /// The key, for example `Reviewed-by`.
    pub key: String,
    /// What follows the colon.
    pub value: String,
}

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

/// Whether git reads the text as a trailer key: letters, digits and
/// hyphens, starting with a letter or a digit.
pub fn is_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 100
        && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !key.starts_with('-')
}

impl Trailer {
    /// The trailer as one argument for git, or why it is none.
    fn argument(&self) -> Result<String> {
        let key = self.key.trim().trim_end_matches(':');
        if !is_key(key) {
            return Err(invalid(
                say!("message-what-trailer"),
                say!("message-not-trailer-key", key = format!("{:?}", self.key)),
            ));
        }
        let value = self.value.trim();
        if value.is_empty() {
            return Err(invalid(
                say!("message-what-trailer"),
                say!("message-needs-value", key),
            ));
        }
        if value.contains(['\n', '\r', '\0']) {
            return Err(invalid(
                say!("message-what-trailer"),
                say!("message-value-one-line", key),
            ));
        }
        Ok(format!("{key}: {value}"))
    }
}

/// The message with the trailers added, as `git interpret-trailers` adds
/// them: into the block of trailers at the end when there is one, as a
/// new block otherwise, and by the repository's `trailer.*` settings.
pub fn add_trailers(repo: &Repository, message: &str, trailers: &[Trailer]) -> Result<String> {
    if message.contains('\0') {
        return Err(invalid(
            say!("message-what-message"),
            say!("message-no-nul"),
        ));
    }
    let mut args: Vec<String> = vec!["interpret-trailers".into(), "--no-divider".into()];
    for trailer in trailers {
        args.push("--trailer".into());
        args.push(trailer.argument()?);
    }
    Ok(exe::run(repo, &args, Some(message.as_bytes()))?.stdout)
}

/// The trailer that signs a message off as the identity git would commit
/// with here, as `git commit --signoff` writes it.
pub fn sign_off(repo: &Repository) -> Result<Trailer> {
    let name = exe::config_get(repo, "user.name");
    let email = exe::config_get(repo, "user.email");
    match (name, email) {
        (Some(name), Some(email)) => Ok(Trailer {
            key: "Signed-off-by".to_owned(),
            value: format!("{name} <{email}>"),
        }),
        _ => Err(Error::Refused {
            what: say!("message-what-sign-off"),
            detail: say!("message-no-identity").to_owned(),
        }),
    }
}

/// A trailer key a repository uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrailerKey {
    /// The key as it is written.
    pub key: String,
    /// How many of the commits looked at carry it.
    pub count: usize,
    /// Whether the configuration names it (`trailer.<token>.key`).
    pub configured: bool,
}

/// The trailer keys of the last `limit` commits on `HEAD` and of the
/// configuration: the configured ones first, then the most used.
pub fn trailer_keys(repo: &Repository, limit: usize) -> Result<Vec<TrailerKey>> {
    let mut seen: BTreeMap<String, (String, usize, bool)> = BTreeMap::new();
    // An unborn branch has no commits to look at, which is no failure.
    let depth = format!("--max-count={}", limit.min(10_000));
    let (code, out) = exe::run_with_code(
        repo,
        [
            "log",
            depth.as_str(),
            "--format=%(trailers:only=true,keyonly=true)",
            "HEAD",
            "--",
        ],
        None,
    )?;
    if code == Some(0) {
        for key in out.stdout.lines().map(str::trim).filter(|key| is_key(key)) {
            let entry = seen
                .entry(key.to_ascii_lowercase())
                .or_insert_with(|| (key.to_owned(), 0, false));
            entry.1 = entry.1.saturating_add(1);
        }
    }
    let (code, out) = exe::run_with_code(
        repo,
        ["config", "--get-regexp", r"^trailer\..*\.key$"],
        None,
    )?;
    if code == Some(0) {
        for line in out.stdout.lines() {
            let Some((_, key)) = line.split_once(' ') else {
                continue;
            };
            let key = key.trim().trim_end_matches(':').trim();
            if !is_key(key) {
                continue;
            }
            let entry = seen
                .entry(key.to_ascii_lowercase())
                .or_insert_with(|| (key.to_owned(), 0, true));
            entry.2 = true;
        }
    }
    let mut keys: Vec<TrailerKey> = seen
        .into_values()
        .map(|(key, count, configured)| TrailerKey {
            key,
            count,
            configured,
        })
        .collect();
    keys.sort_by(|a, b| {
        b.configured
            .cmp(&a.configured)
            .then_with(|| b.count.cmp(&a.count))
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(keys)
}

/// The message of a commit that was made in this worktree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentMessage {
    /// The commit it was the message of; it may be gone from every branch.
    pub id: Oid,
    /// The first line.
    pub summary: String,
    /// The whole message.
    pub message: String,
    /// When the commit was made, in unix seconds.
    pub time: i64,
}

/// How many reflog entries are looked through at most.
const SCANNED: usize = 2000;

/// The messages of the last commits made in this worktree, newest first,
/// each message once: what the reflog of `HEAD` records as a commit, an
/// amend, a merge or a cherry-pick.
pub fn recent_messages(repo: &Repository, limit: usize) -> Result<Vec<RecentMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let depth = format!("--max-count={SCANNED}");
    // Without a reflog, or on an unborn branch, there is nothing to list.
    let (code, out) = exe::run_with_code(
        repo,
        [
            "log",
            "--walk-reflogs",
            depth.as_str(),
            "--format=%H%x00%gs%x00%ct%x00%B%x00",
            "HEAD",
            "--",
        ],
        None,
    )?;
    if code != Some(0) {
        return Ok(Vec::new());
    }
    let mut messages: Vec<RecentMessage> = Vec::new();
    let mut fields = out.stdout.split('\0');
    while let (Some(id), Some(action), Some(time), Some(message)) =
        (fields.next(), fields.next(), fields.next(), fields.next())
    {
        if messages.len() >= limit {
            break;
        }
        // Each record follows the line end of the one before it.
        let Ok(id) = id.trim().parse::<Oid>() else {
            continue;
        };
        if !made_here(action) {
            continue;
        }
        let message = message.trim_end().to_owned();
        if message.is_empty() || messages.iter().any(|known| known.message == message) {
            continue;
        }
        messages.push(RecentMessage {
            id,
            summary: message.lines().next().unwrap_or_default().to_owned(),
            message,
            time: time.trim().parse().unwrap_or(0),
        });
    }
    Ok(messages)
}

/// Whether a reflog entry is a commit that was made here: `commit:`,
/// `commit (amend):`, `commit (merge):`, `commit (initial):`,
/// `commit (cherry-pick):`, and the commits a cherry-pick, a revert or a
/// rebase made while rewording.
pub fn made_here(action: &str) -> bool {
    let action = action.trim_start();
    action.starts_with("commit:")
        || action.starts_with("commit (")
        || action.starts_with("cherry-pick:")
        || action.starts_with("revert:")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_trailer_key_is_letters_digits_and_hyphens() {
        for key in [
            "Reviewed-by",
            "Fixes",
            "Change-Id",
            "X",
            "BREAKING-CHANGE",
            "2nd",
        ] {
            assert!(is_key(key), "{key}");
        }
        for key in ["", "-x", "two words", "key:", "a=b", "naïve", "a\nb", "a_b"] {
            assert!(!is_key(key), "{key:?}");
        }
        assert!(!is_key(&"k".repeat(101)));
    }

    #[test]
    fn a_trailer_becomes_one_argument_or_is_refused() {
        let trailer = |key: &str, value: &str| Trailer {
            key: key.to_owned(),
            value: value.to_owned(),
        };
        assert_eq!(
            trailer(" Reviewed-by: ", "  Ada <ada@x.invalid> ")
                .argument()
                .unwrap(),
            "Reviewed-by: Ada <ada@x.invalid>"
        );
        for (key, value) in [
            ("", "v"),
            ("two words", "v"),
            ("--trailer", "v"),
            ("Key", ""),
            ("Key", "   "),
            ("Key", "two\nlines"),
            ("Key", "nul\0"),
            ("a=b", "v"),
        ] {
            assert!(trailer(key, value).argument().is_err(), "{key:?} {value:?}");
        }
    }

    #[test]
    fn only_commits_made_here_are_messages_made_here() {
        for action in [
            "commit: add a",
            "commit (amend): add a",
            "commit (initial): start",
            "commit (merge): Merge branch 'x'",
            "commit (cherry-pick): pick",
            "cherry-pick: pick",
            "revert: Revert \"x\"",
        ] {
            assert!(made_here(action), "{action}");
        }
        for action in [
            "checkout: moving from main to topic",
            "reset: moving to HEAD~1",
            "pull: Fast-forward",
            "merge topic: Fast-forward",
            "rebase (finish): returning to refs/heads/main",
            "clone: from /somewhere",
            "committed: no",
            "",
        ] {
            assert!(!made_here(action), "{action}");
        }
    }
}
