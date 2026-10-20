//! Holding on to objects that undo needs: references of our own that keep
//! them from being pruned.
//!
//! What undo puts back is often something no reference names any more: the
//! content of a file that was discarded (a blob made for the purpose), a
//! stash that was dropped, the tip of a branch or the object of a tag that
//! was deleted, what a removed remote's branches pointed at. Git keeps
//! such objects only until it next prunes, and `git gc --prune=now` or two
//! weeks of ordinary use are enough to lose them. A reference is what
//! keeps an object, so each one that the journal still has a use for gets
//! one, and loses it when the journal forgets the entry it belongs to.
//!
//! The references live under `refs/bitbraid/undo/`, apart from branches,
//! remotes and tags: nothing here lists them, no walk starts from them,
//! and a fetch or a clone does not take them. `git for-each-ref` and `git
//! log --all` do show them; that is the price of something git itself
//! respects.
//!
//! A linked worktree has a journal of its own, so it has a part of the
//! namespace of its own: `main/` for the main worktree, `worktrees/<id>/`
//! for a linked one. References that are per worktree (`refs/worktree/`)
//! would have been the obvious place, and are no use: `git gc` run in one
//! worktree prunes what only such a reference of another worktree holds
//! (tried with git 2.56). The part of a worktree that is gone is cleared
//! away by whichever worktree next writes its journal.
//!
//! The name of a reference is `<entry>/<object id>` under that part, and
//! its value is the object. Making what is wanted and removing what is
//! not is therefore a comparison of two sets of names, and doing it again
//! changes nothing.
//!
//! Sources: git-gc(1) and git-prune(1) (what is kept: everything a
//! reference reaches), git-update-ref(1), gitrepository-layout(5)
//! (`worktrees/<id>`), git-worktree(1) ("Refs": which references are per
//! worktree), git-check-ref-format(1).

use std::collections::{BTreeMap, BTreeSet};

use gix::refs::{
    FullName, Target,
    transaction::{Change, LogChange, PreviousValue, RefEdit, RefLog},
};

use crate::{Oid, Repository, Result, git_err};

/// Where the references are kept.
pub const NAMESPACE: &str = "refs/bitbraid/undo/";

/// The part of the namespace that is this worktree's.
fn own_part(repo: &Repository) -> String {
    let r = repo.gix();
    let linked = r.git_dir() != r.common_dir();
    let id = r
        .git_dir()
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|_| linked);
    match id {
        Some(id) => format!("{NAMESPACE}worktrees/{id}/"),
        None => format!("{NAMESPACE}main/"),
    }
}

/// The full names of the references under `prefix`.
fn names_under(repo: &Repository, prefix: &str) -> Result<BTreeSet<String>> {
    let platform = repo
        .gix()
        .references()
        .map_err(git_err(say!("hold-listing-references")))?;
    let found = platform
        .prefixed(prefix)
        .map_err(git_err(say!("hold-listing-held")))?
        .filter_map(|reference| reference.ok())
        .map(|reference| reference.name().as_bstr().to_string())
        .collect();
    Ok(found)
}

/// The entry and the object a reference of ours is for, from what stands
/// after the worktree's part of its name.
fn parse(rest: &str) -> Option<(u64, Oid)> {
    let (entry, object) = rest.split_once('/')?;
    Some((entry.parse().ok()?, object.parse().ok()?))
}

/// What is held for this worktree's journal: entry and object, in order.
pub fn held(repo: &Repository) -> Result<Vec<(u64, Oid)>> {
    let part = own_part(repo);
    let mut found: Vec<(u64, Oid)> = names_under(repo, &part)?
        .iter()
        .filter_map(|name| parse(name.strip_prefix(part.as_str())?))
        .collect();
    found.sort();
    Ok(found)
}

/// The parts of linked worktrees that are no longer there.
fn parts_of_the_gone(repo: &Repository) -> Result<BTreeSet<String>> {
    let prefix = format!("{NAMESPACE}worktrees/");
    let worktrees = repo.gix().common_dir().join("worktrees");
    Ok(names_under(repo, &prefix)?
        .into_iter()
        .filter(|name| {
            name.strip_prefix(prefix.as_str())
                .and_then(|rest| rest.split('/').next())
                .is_some_and(|id| !worktrees.join(id).is_dir())
        })
        .collect())
}

/// Hold exactly `wanted` for this worktree's journal: each object of each
/// entry gets a reference, and every reference of ours that is for
/// something else goes. An object that is not in the repository (pruned
/// before it was held) is passed over. Returns how many references were
/// made and how many removed.
pub fn reconcile(repo: &Repository, wanted: &[(u64, Oid)]) -> Result<(usize, usize)> {
    let r = repo.gix();
    let part = own_part(repo);
    let want: BTreeMap<String, Oid> = wanted
        .iter()
        .map(|(entry, object)| (format!("{part}{entry}/{}", object.to_hex()), *object))
        .collect();
    let have = names_under(repo, &part)?;
    let mut gone = parts_of_the_gone(repo)?;
    gone.extend(
        have.iter()
            .filter(|name| !want.contains_key(*name))
            .cloned(),
    );
    let make: Vec<(&String, &Oid)> = want
        .iter()
        .filter(|(name, object)| !have.contains(*name) && r.has_object(*object.as_gix()))
        .collect();
    if make.is_empty() && gone.is_empty() {
        return Ok((0, 0));
    }
    let mut edits = Vec::with_capacity(make.len().saturating_add(gone.len()));
    for (name, object) in &make {
        let name =
            FullName::try_from(name.as_str()).map_err(git_err(say!("hold-naming-reference")))?;
        edits.push(RefEdit {
            change: Change::Update {
                log: LogChange::default(),
                expected: PreviousValue::Any,
                new: Target::Object(*object.as_gix()),
            },
            name,
            deref: false,
        });
    }
    for name in &gone {
        // A name of ours that is not a valid one was not made by us, and
        // is left where it is.
        let Ok(name) = FullName::try_from(name.as_str()) else {
            continue;
        };
        edits.push(RefEdit {
            change: Change::Delete {
                expected: PreviousValue::Any,
                log: RefLog::AndReference,
            },
            name,
            deref: false,
        });
    }
    r.edit_references(edits)
        .map_err(git_err(say!("hold-updating-references")))?;
    Ok((make.len(), gone.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_says_which_entry_and_which_object() {
        let hex = "1234567890abcdef1234567890abcdef12345678";
        let object: Oid = hex.parse().unwrap();
        assert_eq!(parse(&format!("42/{hex}")), Some((42, object)));
        for rest in [
            "",
            "42",
            "42/",
            "/1234",
            "x/1234567890abcdef1234567890abcdef12345678",
            "42/not-an-object",
            "-1/1234567890abcdef1234567890abcdef12345678",
            "99999999999999999999999/1234567890abcdef1234567890abcdef12345678",
        ] {
            assert_eq!(parse(rest), None, "{rest:?}");
        }
    }
}
