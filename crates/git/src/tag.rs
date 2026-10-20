//! Tags: create (lightweight, annotated, signed), delete, put a deleted
//! one back, and give a lightweight tag a message after the fact. Pushing
//! tags is with the other network operations in [`crate::remote`].
//!
//! Sources: git-tag(1), git-update-ref(1), git-check-ref-format(1).

use crate::{Error, Oid, Repository, Result, exe};

fn invalid(what: &'static str, detail: impl Into<String>) -> Error {
    Error::Invalid {
        what,
        detail: detail.into(),
    }
}

fn refused(detail: impl Into<String>) -> Error {
    Error::Refused {
        what: say!("tag-what-tag"),
        detail: detail.into(),
    }
}

/// Check a tag name literally: nothing git could read as an option or as
/// a revision expression, and a valid reference name under `refs/tags`.
pub fn check_tag_name(repo: &Repository, name: &str) -> Result<()> {
    let bad = || {
        invalid(
            say!("tag-what-name"),
            say!("tag-not-valid-name", name = format!("{name:?}")),
        )
    };
    if name.is_empty()
        || name.starts_with('-')
        || name.chars().any(char::is_control)
        || name == "HEAD"
        || name == "@"
        || name.contains("@{")
    {
        return Err(bad());
    }
    let reference = format!("refs/tags/{name}");
    exe::run(repo, ["check-ref-format", reference.as_str()], None)
        .map(|_| ())
        .map_err(|_| bad())
}

/// What `refs/tags/<name>` holds: the tag object of an annotated tag, the
/// commit of a lightweight one. None when there is no such tag.
pub fn tag_value(repo: &Repository, name: &str) -> Option<Oid> {
    let reference = format!("refs/tags/{name}");
    exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", reference.as_str()],
        None,
    )
    .ok()
    .and_then(|out| out.stdout.trim().parse().ok())
}

/// The commit a revision names.
fn commit_of(repo: &Repository, rev: &str) -> Result<Oid> {
    let bad = || {
        invalid(
            say!("tag-what-target"),
            say!("tag-names-no-commit", rev = format!("{rev:?}")),
        )
    };
    if rev.is_empty() || rev.starts_with('-') || rev.chars().any(char::is_control) {
        return Err(bad());
    }
    let spec = format!("{rev}^{{commit}}");
    exe::run(
        repo,
        ["rev-parse", "--verify", "--quiet", spec.as_str()],
        None,
    )
    .ok()
    .and_then(|out| out.stdout.trim().parse().ok())
    .ok_or_else(bad)
}

/// Create a tag at `target` (HEAD when absent). With a message it is an
/// annotated tag, with `sign` a signed one (which needs a message, and a
/// key git can use). Returns what the tag reference holds.
pub fn tag_create(
    repo: &Repository,
    name: &str,
    target: Option<&str>,
    message: Option<&str>,
    sign: bool,
) -> Result<Oid> {
    check_tag_name(repo, name)?;
    let commit = commit_of(repo, target.unwrap_or("HEAD"))?;
    if tag_value(repo, name).is_some() {
        return Err(refused(say!("tag-exists", name)));
    }
    let message = message.map(str::trim).filter(|text| !text.is_empty());
    if message.is_some_and(|text| text.contains('\0')) {
        return Err(invalid(say!("tag-what-message"), say!("tag-message-nul")));
    }
    let hex = commit.to_hex();
    let mut args = vec!["tag"];
    match (message, sign) {
        (Some(text), true) => args.extend(["--sign", "--message", text]),
        (Some(text), false) => args.extend(["--annotate", "--message", text]),
        (None, true) => return Err(refused(say!("tag-signed-needs-message"))),
        (None, false) => {}
    }
    args.extend(["--", name, hex.as_str()]);
    // A signature that could not be made is explained.
    exe::run(repo, args, None)
        .map_err(|err| crate::signature::explain_failure(repo, err, say!("tag-what-tag")))?;
    tag_value(repo, name).ok_or_else(|| refused(say!("tag-not-created", name)))
}

/// Delete a tag if it still holds `expected` (any value when absent).
/// Returns what it held, with which [`tag_restore`] puts it back.
pub fn tag_delete(repo: &Repository, name: &str, expected: Option<Oid>) -> Result<Oid> {
    check_tag_name(repo, name)?;
    let value = tag_value(repo, name).ok_or_else(|| refused(say!("tag-none", name)))?;
    if expected.is_some_and(|expected| expected != value) {
        return Err(refused(say!("tag-moved", name)));
    }
    let reference = format!("refs/tags/{name}");
    let hex = value.to_hex();
    // Compare-and-swap: only if nobody moved it in the meantime.
    exe::run(
        repo,
        ["update-ref", "-d", reference.as_str(), hex.as_str()],
        None,
    )?;
    Ok(value)
}

/// Put a deleted tag back: the reference is created holding `value` (the
/// tag object or commit it held), and only if no tag of that name exists.
pub fn tag_restore(repo: &Repository, name: &str, value: Oid) -> Result<()> {
    check_tag_name(repo, name)?;
    if tag_value(repo, name).is_some() {
        return Err(refused(say!("tag-exists-again", name)));
    }
    let reference = format!("refs/tags/{name}");
    let hex = value.to_hex();
    let zero = Oid::zero().to_hex();
    exe::run(
        repo,
        [
            "update-ref",
            reference.as_str(),
            hex.as_str(),
            zero.as_str(),
        ],
        None,
    )
    .map_err(|_| refused(say!("tag-target-gone", name)))?;
    Ok(())
}

/// Give the lightweight tag `name` a message: it is made anew as an
/// annotated tag of the same name at the same commit. Returns what the
/// reference held before (the commit) and holds now (the tag object), with
/// which [`tag_swap`] takes it back. Refused for a tag that has a message
/// already, and for a message that is empty or holds a NUL.
pub fn tag_annotate(repo: &Repository, name: &str, message: &str) -> Result<(Oid, Oid)> {
    check_tag_name(repo, name)?;
    let before = tag_value(repo, name).ok_or_else(|| refused(say!("tag-none", name)))?;
    let hex = before.to_hex();
    let kind = exe::run(repo, ["cat-file", "-t", hex.as_str()], None)?;
    if kind.stdout.trim() != "commit" {
        return Err(refused(say!("tag-has-message", name)));
    }
    let message = message.trim();
    if message.is_empty() {
        return Err(invalid(say!("tag-what-message"), say!("tag-message-empty")));
    }
    if message.contains('\0') {
        return Err(invalid(say!("tag-what-message"), say!("tag-message-nul")));
    }
    exe::run(
        repo,
        [
            "tag",
            "--annotate",
            "--force",
            "--message",
            message,
            "--",
            name,
            hex.as_str(),
        ],
        None,
    )
    .map_err(|err| crate::signature::explain_failure(repo, err, say!("tag-what-tag")))?;
    let after = tag_value(repo, name).ok_or_else(|| refused(say!("tag-not-annotated", name)))?;
    Ok((before, after))
}

/// Point the tag `name` at `value` again, if it still holds `expected`:
/// taking an annotation back, or giving it again.
pub fn tag_swap(repo: &Repository, name: &str, expected: Oid, value: Oid) -> Result<()> {
    check_tag_name(repo, name)?;
    if tag_value(repo, name) != Some(expected) {
        return Err(refused(say!("tag-moved", name)));
    }
    let reference = format!("refs/tags/{name}");
    let (to, from) = (value.to_hex(), expected.to_hex());
    exe::run(
        repo,
        ["update-ref", reference.as_str(), to.as_str(), from.as_str()],
        None,
    )
    .map_err(|_| refused(say!("tag-target-gone", name)))?;
    Ok(())
}
