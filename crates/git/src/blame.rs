//! Blame: which commit introduced each line of a file at a revision.
//!
//! Sources: git-blame(1), gix blame documentation.

use gix::bstr::ByteSlice;
use serde::{Deserialize, Serialize};

use crate::{Error, Oid, Repository, Result, git_err};

/// One blamed line.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlameLine {
    /// 1-based line number in the blamed file.
    pub line: u32,
    /// Commit that introduced the line.
    pub commit: Oid,
    /// 1-based line number in that commit's version of the file.
    pub source_line: u32,
    /// Text without the trailing newline.
    pub text: String,
    /// The file's name in `commit`, where that is not the name of the
    /// file that was blamed: it was renamed since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// Where the lines of `path` as `commit` has it were before it, as git's
/// blame says it (`previous` in its porcelain): the commit's first parent
/// and the file's name there. That is the same name, or the name the file
/// had before the commit renamed it. Nothing for a commit that added the
/// file, and for one without a parent (a root, or the edge of a shallow
/// clone).
pub fn previous(repo: &Repository, commit: Oid, path: &str) -> Result<Option<(Oid, String)>> {
    crate::stage::check_rel(path)?;
    let Some(parent) = crate::parents(repo, commit).first().copied() else {
        return Ok(None);
    };
    let r = repo.gix();
    let tree_of = |id: Oid| {
        r.find_commit(*id.as_gix())
            .map_err(git_err(say!("blame-loading-commit")))?
            .tree()
            .map_err(git_err(say!("blame-loading-commit-tree")))
    };
    let (old_tree, new_tree) = (tree_of(parent)?, tree_of(commit)?);
    let is_file = |tree: &gix::Tree<'_>, path: &str| -> Result<bool> {
        Ok(tree
            .lookup_entry_by_path(path)
            .map_err(git_err(say!("blame-looking-up-path")))?
            .is_some_and(|entry| entry.mode().is_blob() || entry.mode().is_link()))
    };
    if is_file(&old_tree, path)? {
        return Ok(Some((parent, path.to_owned())));
    }
    // Not there by that name: the file the commit renamed to it, as a
    // diff that looks for renames pairs them.
    let mut platform = old_tree
        .changes()
        .map_err(git_err(say!("blame-preparing-tree-diff")))?;
    platform.options(|o| {
        o.track_path();
        o.track_rewrites(Some(crate::diff::rewrites()));
    });
    let mut found: Option<String> = None;
    platform
        .for_each_to_obtain_tree(&new_tree, |change| {
            use gix::object::tree::diff::Change;
            if let Change::Rewrite {
                source_location,
                location,
                copy: false,
                ..
            } = &change
                && location.as_bytes() == path.as_bytes()
            {
                found = Some(source_location.to_str_lossy().into_owned());
                return Ok(std::ops::ControlFlow::Break(()));
            }
            Ok(std::ops::ControlFlow::Continue(()))
        })
        .map_err(git_err(say!("blame-diffing-trees")))?;
    Ok(found.map(|old| (parent, old)))
}

/// Blame `path` as of `rev` (HEAD when absent). In a partial clone what
/// it needs and was left on the remote is fetched ([`crate::promisor`]).
pub fn blame(repo: &Repository, path: &str, rev: Option<Oid>) -> Result<Vec<BlameLine>> {
    crate::promisor::fetching(
        repo,
        || blame_here(repo, path, rev),
        || {
            let rev = rev.map(|id| id.to_hex());
            crate::promisor::fetch_for_blame(repo, path, rev.as_deref())
        },
    )
}

/// [`blame`] with what is here.
fn blame_here(repo: &Repository, path: &str, rev: Option<Oid>) -> Result<Vec<BlameLine>> {
    let r = repo.gix();
    let suspect = match rev {
        Some(id) => *id.as_gix(),
        None => r
            .head_id()
            .map_err(git_err(say!("blame-resolving-head")))?
            .detach(),
    };
    let options = gix::repository::blame_file::Options {
        rewrites: Some(crate::diff::rewrites()),
        ..Default::default()
    };
    let outcome = r
        .blame_file(path.as_bytes().as_bstr(), suspect, options)
        .map_err(|err| Error::Git {
            context: say!("blame-blaming-file"),
            source: Box::new(err),
        })?;
    let lines: Vec<&[u8]> = outcome.blob.split_inclusive(|b| *b == b'\n').collect();
    let mut out = Vec::with_capacity(lines.len());
    for entry in &outcome.entries {
        for offset in 0..entry.len.get() {
            let line_ix = entry.start_in_blamed_file.saturating_add(offset);
            let text = lines
                .get(usize::try_from(line_ix).unwrap_or(usize::MAX))
                .map(|raw| {
                    raw.strip_suffix(b"\n")
                        .unwrap_or(raw)
                        .to_str_lossy()
                        .into_owned()
                })
                .unwrap_or_default();
            out.push(BlameLine {
                line: line_ix.saturating_add(1),
                commit: Oid::from(entry.commit_id),
                source_line: entry
                    .start_in_source_file
                    .saturating_add(offset)
                    .saturating_add(1),
                text,
                source_path: entry
                    .source_file_name
                    .as_ref()
                    .map(|name| name.to_str_lossy().into_owned())
                    .filter(|name| name != path),
            });
        }
    }
    out.sort_by_key(|l| l.line);
    Ok(out)
}
