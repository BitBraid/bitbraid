//! A file that one side of a stopped merge, rebase or pick changed and
//! the other deleted: there are no lines to choose between, only whether
//! the file stays. The window asks that in the bar where the toolbar is
//! and stays on what it showed: keep the changed file, delete it, or
//! resolve it in the external merge tool; giving up leaves it in
//! conflict.
//!
//! Sources: git-merge(1) (how a modify/delete conflict is resolved);
//! `crates/ui/src/ask.rs`.

use bitbraid_git::sequence::ConflictKind;
use gpui_kit::*;
use serde_json::json;

use super::Workspace;

/// The side that kept the file and the side that deleted it, as
/// `conflict.take` names them, for a conflict of this kind.
pub(crate) fn kept_and_deleted(kind: ConflictKind) -> Option<(&'static str, &'static str)> {
    match kind {
        ConflictKind::DeletedByThem => Some(("ours", "theirs")),
        ConflictKind::DeletedByUs => Some(("theirs", "ours")),
        _ => None,
    }
}

impl Workspace {
    /// Ask about a file in conflict that one side deleted. False when the
    /// file is not one of those: it is opened in the merge tool then.
    pub(super) fn ask_about_deleted(
        &mut self,
        path: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let sides = self.in_progress.as_ref().and_then(|operation| {
            operation
                .conflicts
                .iter()
                .find(|conflict| conflict.path == path)
                .and_then(|conflict| kept_and_deleted(conflict.kind))
        });
        let Some((kept, deleted)) = sides else {
            return false;
        };
        let name = path.rsplit('/').next().unwrap_or(path).to_owned();
        let take = |side: &'static str, done: String| {
            let workspace = cx.entity().downgrade();
            let path = path.to_owned();
            move |_: Vec<String>, window: &mut Window, cx: &mut App| {
                let (path, done) = (path.clone(), done.clone());
                let _ = workspace.update(cx, |this, cx| {
                    this.run_mutation(
                        "conflict.take",
                        json!({ "path": path, "side": side }),
                        done,
                        |_, _, _, _| false,
                        window,
                        cx,
                    )
                });
            }
        };
        let external = {
            let workspace = cx.entity().downgrade();
            let path = path.to_owned();
            move |_: Vec<String>, window: &mut Window, cx: &mut App| {
                let path = path.clone();
                let _ = workspace.update(cx, |this, cx| this.external_merge(path, window, cx));
            }
        };
        crate::ask::show(
            crate::ask::Ask::new(say!("workspace-deleted-question", name = name.as_str()))
                .go(
                    say!("workspace-deleted-keep"),
                    take(kept, say!("workspace-deleted-kept", name = name.as_str())),
                )
                .danger(
                    say!("workspace-deleted-delete"),
                    take(
                        deleted,
                        say!("workspace-deleted-deleted", name = name.as_str()),
                    ),
                )
                .other(say!("workspace-deleted-external"), external),
            window,
            cx,
        );
        true
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::sequence::ConflictKind;

    use super::kept_and_deleted;

    #[test]
    fn the_side_that_kept_the_file_is_the_one_taken_to_keep_it() {
        assert_eq!(
            kept_and_deleted(ConflictKind::DeletedByThem),
            Some(("ours", "theirs"))
        );
        assert_eq!(
            kept_and_deleted(ConflictKind::DeletedByUs),
            Some(("theirs", "ours"))
        );
        for kind in [
            ConflictKind::BothModified,
            ConflictKind::BothAdded,
            ConflictKind::AddedByUs,
            ConflictKind::AddedByThem,
            ConflictKind::BothDeleted,
        ] {
            assert_eq!(kept_and_deleted(kind), None, "{kind:?}");
        }
    }
}
