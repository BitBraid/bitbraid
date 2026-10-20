//! What a worktree file is as git stores it, and the other way round.
//!
//! A file in the worktree is not always what git stores. Line endings are
//! changed on the way in and out as `core.autocrlf` (on by default in Git
//! for Windows: CRLF in the worktree, LF in the repository), `core.eol` and
//! the `text` and `eol` attributes say, and `ident` and
//! `working-tree-encoding` change a file too. What the worktree side of a
//! diff shows is what git would store, so that a file checked out with
//! CRLF does not read as every line changed, as `git diff` has it; and
//! what is written back into the worktree is what git would check out.
//!
//! A filter driver (git-lfs) is a program of the repository's: it is not
//! run to show a diff or to stage or discard part of a file. A file it
//! stands for is compared as it is in the worktree, so that a changed
//! picture under git-lfs is a changed picture (staged whole, as any other
//! binary file), not two lines of a pointer to be staged one at a time.
//!
//! When the conversion cannot be made the bytes are taken as they are: a
//! diff that shows too much is better than none.
//!
//! Sources: gitattributes(5) (`text`, `eol`, `filter`), git-config(1)
//! (`core.autocrlf`, `core.eol`); gitoxide's `filter::Pipeline`
//! (`convert_to_git`, `convert_to_worktree`).

use std::{io::Read as _, path::Path};

use gix::{
    bstr::ByteSlice as _,
    filter::plumbing::{
        Pipeline,
        driver::apply::MaybeDelayed,
        pipeline::convert::{ToGitOutcome, ToWorktreeOutcome},
    },
    objs::Find as _,
    worktree::{IndexPersistedOrInMemory, Stack},
};

use crate::Repository;

/// gitoxide's pipeline as the repository's configuration makes it, with
/// no filter driver in it; the attributes of the paths it converts; and
/// the index it was made with.
fn pipeline(repo: &Repository) -> Option<(Pipeline, Stack, IndexPersistedOrInMemory)> {
    let (pipeline, index) = repo.gix().filter_pipeline(None).ok()?;
    let (mut pipeline, attributes) = pipeline.into_parts();
    pipeline.options_mut().drivers.clear();
    Some((pipeline, attributes, index))
}

/// The worktree's `bytes` of `path` as git would store them.
pub(crate) fn to_git(repo: &Repository, path: &str, bytes: Vec<u8>) -> Vec<u8> {
    converted_to_git(repo, path, &bytes).unwrap_or(bytes)
}

fn converted_to_git(repo: &Repository, path: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let objects = &repo.gix().objects;
    let (mut pipeline, mut attributes, index) = pipeline(repo)?;
    let at = attributes.at_path(Path::new(path), None, objects).ok()?;
    let outcome = pipeline
        .convert_to_git(
            bytes,
            Path::new(path),
            &mut |_, out| {
                at.matching_attributes(out);
            },
            // What the index has there, which `core.autocrlf` looks into:
            // a file stored with CRLF keeps them.
            &mut |buf| {
                let found = index
                    .entry_by_path(path.as_bytes().as_bstr())
                    .and_then(|entry| objects.try_find(&entry.id, buf).ok().flatten());
                Ok(found
                    .filter(|object| object.kind == gix::objs::Kind::Blob)
                    .map(|_| ()))
            },
        )
        .ok()?;
    match outcome {
        ToGitOutcome::Unchanged(_) => None,
        ToGitOutcome::Buffer(converted) => Some(converted.to_vec()),
        ToGitOutcome::Process(mut stream) => {
            let mut converted = Vec::new();
            stream.read_to_end(&mut converted).ok()?;
            Some(converted)
        }
    }
}

/// `bytes` of `path` as git stores them, as git would check them out.
pub(crate) fn to_worktree(repo: &Repository, path: &str, bytes: Vec<u8>) -> Vec<u8> {
    converted_to_worktree(repo, path, &bytes).unwrap_or(bytes)
}

fn converted_to_worktree(repo: &Repository, path: &str, bytes: &[u8]) -> Option<Vec<u8>> {
    let objects = &repo.gix().objects;
    let (mut pipeline, mut attributes, _) = pipeline(repo)?;
    let at = attributes
        .at_entry(path.as_bytes().as_bstr(), None, objects)
        .ok()?;
    let outcome = pipeline
        .convert_to_worktree(
            bytes,
            path.as_bytes().as_bstr(),
            &mut |_, out| {
                at.matching_attributes(out);
            },
            Default::default(),
        )
        .ok()?;
    match outcome {
        ToWorktreeOutcome::Unchanged(_) => None,
        ToWorktreeOutcome::Buffer(converted) => Some(converted.to_vec()),
        ToWorktreeOutcome::Process(MaybeDelayed::Immediate(mut stream)) => {
            let mut converted = Vec::new();
            stream.read_to_end(&mut converted).ok()?;
            Some(converted)
        }
        ToWorktreeOutcome::Process(MaybeDelayed::Delayed(_)) => None,
    }
}
