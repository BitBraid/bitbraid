//! The note about an operation that stopped half-way: what it is called
//! and where HEAD was when it began. When the operation is finished from
//! here the note becomes its journal entry; when it is aborted, or turns
//! out to be about another operation, it is dropped.
//!
//! Stored as `.git/bitbraid/operation.json`, next to the journal.
//!
//! Sources: none (original design).

use std::{fs, path::PathBuf};

use bitbraid_git::{HeadState, Repository, sequence::InProgress};
use serde::{Deserialize, Serialize};

/// The note.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pending {
    /// What the journal entry will be called, for example `Merge topic`.
    pub label: String,
    /// HEAD when the operation began.
    pub before: HeadState,
}

fn path(repo: &Repository) -> PathBuf {
    repo.git_dir().join("bitbraid").join("operation.json")
}

/// Remember what the operation that just stopped began from. Like the
/// journal, a note that cannot be written does not fail the operation.
pub fn save(repo: &Repository, pending: &Pending) {
    let path = path(repo);
    let Some(parent) = path.parent() else { return };
    if fs::create_dir_all(parent).is_err() {
        return;
    }
    let Ok(text) = serde_json::to_string_pretty(pending) else {
        return;
    };
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &path);
    }
}

/// Forget the note.
pub fn clear(repo: &Repository) {
    let _ = fs::remove_file(path(repo));
}

/// The note, when there is one and it is about `operation`: the operation
/// began where the note says HEAD was. A note left by an operation that
/// was ended somewhere else does not pass for a later one.
pub fn load_for(repo: &Repository, operation: &InProgress) -> Option<Pending> {
    let text = fs::read_to_string(path(repo)).ok()?;
    let pending: Pending = serde_json::from_str(&text).ok()?;
    let began = operation.original_head.or_else(|| {
        bitbraid_git::branch::head_state(repo)
            .ok()
            .and_then(|head| head.commit)
    });
    (began.is_some() && began == pending.before.commit).then_some(pending)
}
