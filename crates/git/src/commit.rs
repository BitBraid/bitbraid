//! Creating commits through the git executable, so hooks, signing, templates
//! and identity configuration behave exactly as on the command line.
//!
//! Sources: git-commit(1), git-interpret-trailers(1), git-config(1)
//! (`commit.template`, `user.name`, `user.email`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{Oid, Repository, Result, exe};

/// What to commit and how.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CommitRequest {
    /// Full message (summary line, blank line, body). Trailers included.
    pub message: String,
    /// Replace the tip commit instead of adding one.
    #[serde(default)]
    pub amend: bool,
    /// Keep the previous author when amending.
    #[serde(default)]
    pub keep_author: bool,
    /// Skip pre-commit and commit-msg hooks.
    #[serde(default)]
    pub no_verify: bool,
    /// Sign or do not sign, overriding configuration; `None` follows config.
    #[serde(default)]
    pub sign: Option<bool>,
    /// Allow a commit with no changes.
    #[serde(default)]
    pub allow_empty: bool,
}

/// What a commit produced.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CommitOutcome {
    /// The new tip.
    pub id: Oid,
    /// The previous tip, absent on an unborn branch.
    pub previous: Option<Oid>,
    /// Branch that moved, absent when detached.
    pub branch: Option<String>,
    /// The hooks that are there and did not run because the request said
    /// `no_verify`; empty when it did not, or when there are none of them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped_hooks: Vec<String>,
}

/// Create (or amend) a commit from the index.
pub fn commit(repo: &Repository, request: &CommitRequest) -> Result<CommitOutcome> {
    if request.message.trim().is_empty() {
        return Err(crate::Error::Invalid {
            what: say!("commit-what-message"),
            detail: say!("commit-message-required").to_owned(),
        });
    }
    let summary = repo.summary()?;
    let previous = summary.head;
    // Asked before the commit: what is said afterwards is what this
    // commit was made without.
    let skipped_hooks = if request.no_verify {
        crate::hooks::left_out_by_no_verify(repo)
    } else {
        Vec::new()
    };
    let mut args: Vec<String> = vec![
        "commit".into(),
        "-q".into(),
        "-F".into(),
        "-".into(),
        "--cleanup=strip".into(),
    ];
    if request.amend {
        args.push("--amend".into());
        if !request.keep_author {
            args.push("--reset-author".into());
        }
    }
    if request.no_verify {
        args.push("--no-verify".into());
    }
    match request.sign {
        Some(true) => args.push("--gpg-sign".into()),
        Some(false) => args.push("--no-gpg-sign".into()),
        None => {}
    }
    if request.allow_empty {
        args.push("--allow-empty".into());
    }
    // A signature that could not be made is explained, not just reported,
    // and a hook that refused the commit is told apart from git.
    exe::run_hooked(repo, &args, Some(request.message.as_bytes()))
        .map_err(|err| crate::signature::explain_failure(repo, err, say!("commit-what-commit")))?;
    let after = repo.summary()?;
    let id = after.head.ok_or(crate::Error::Invalid {
        what: say!("commit-what-commit"),
        detail: say!("commit-head-unborn").to_owned(),
    })?;
    Ok(CommitOutcome {
        id,
        previous,
        branch: after.head_branch,
        skipped_hooks,
    })
}

/// The configured commit template text, if any.
pub fn template(repo: &Repository) -> Option<String> {
    let path = exe::config_path(repo, "commit.template")?;
    std::fs::read_to_string(path).ok()
}

/// A person seen in recent history, for the co-author picker.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    /// How many recent commits mention them.
    pub count: usize,
}

/// Authors and `Co-authored-by` trailers from the last `limit` commits on HEAD,
/// most frequent first.
pub fn recent_people(repo: &Repository, limit: usize) -> Result<Vec<Person>> {
    let commits = crate::log(
        repo,
        &crate::LogOptions {
            tips: crate::Tips::Head,
            limit: Some(limit),
            ..Default::default()
        },
    )?;
    let mut seen: BTreeMap<String, (String, usize)> = BTreeMap::new();
    for commit in &commits {
        let entry = seen
            .entry(commit.author.email.clone())
            .or_insert((commit.author.name.clone(), 0));
        entry.1 = entry.1.saturating_add(1);
        let message = crate::message(repo, commit.id).unwrap_or_default();
        for line in message.lines() {
            if let Some(rest) = line.strip_prefix("Co-authored-by:")
                && let Some((name, email)) = rest.trim().rsplit_once(" <")
            {
                let email = email.trim_end_matches('>').to_owned();
                let entry = seen.entry(email).or_insert((name.trim().to_owned(), 0));
                entry.1 = entry.1.saturating_add(1);
            }
        }
    }
    let mut people: Vec<Person> = seen
        .into_iter()
        .map(|(email, (name, count))| Person { name, email, count })
        .collect();
    people.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
    Ok(people)
}

/// Move a branch (or detached HEAD) to `new` only if it still points at
/// `expected`, like `git update-ref ref new expected`. Used by undo and redo.
pub fn move_head(
    repo: &Repository,
    branch: Option<&str>,
    new: Oid,
    expected: Option<Oid>,
) -> Result<()> {
    // To git an id of zeros means "delete" as the new value and "must not
    // exist" as the expected one. A journal that says so is not followed.
    if new == Oid::zero() || expected == Some(Oid::zero()) {
        return Err(crate::Error::Refused {
            what: say!("commit-what-undo"),
            detail: say!("commit-journal-no-commit").to_owned(),
        });
    }
    let reference = match branch {
        Some(name) => format!("refs/heads/{name}"),
        None => "HEAD".to_owned(),
    };
    let mut args = vec![
        "update-ref".to_owned(),
        "-m".to_owned(),
        "bitbraid: undo/redo".to_owned(),
        reference,
        new.to_hex(),
    ];
    if let Some(expected) = expected {
        args.push(expected.to_hex());
    }
    exe::run(repo, &args, None)?;
    Ok(())
}
