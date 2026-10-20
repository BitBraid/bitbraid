//! What an operation that stopped part way says about itself: one sentence
//! with what it is, how far it is and what it waits for. The row of
//! uncommitted work in the graph says it, the panel of changes says it
//! over its lists, and the banner over the middle says it when neither is
//! on show.
//!
//! Sources: git-merge(1), git-rebase(1), git-am(1) (what each waits for
//! when it stops).

use bitbraid_git::sequence::{InProgress, Kind};

/// Whether the operation cannot go on as things are: files are in
/// conflict, or the step makes a commit of what is staged (a patch of a
/// mailbox, a pick, a revert) and nothing is. What git itself found is in
/// [`InProgress::ready`].
pub fn blocked(operation: &InProgress) -> bool {
    !operation.ready
}

/// Nothing is in conflict and the operation still cannot go on: its step
/// commits what is staged, and nothing is staged. A patch that did not
/// apply at all leaves it so, and so does a conflict resolved to what was
/// there before.
fn stuck(operation: &InProgress) -> bool {
    operation.conflicts.is_empty() && !operation.ready
}

/// The sentence: what is going on, how far it is, and what it waits for.
/// `part_of` is what a merge belongs to when it is one step of something
/// larger ("Finishing feature search", a gitflow finish that stopped in it).
pub fn sentence(operation: &InProgress, part_of: Option<&str>) -> String {
    let conflicts = operation.conflicts.len();
    let what = match (operation.kind, &operation.branch) {
        (Kind::Merge, _) => match part_of {
            Some(part_of) => say!("stopped-merging-part-of", part_of),
            None => say!("stopped-merging").to_owned(),
        },
        (Kind::Rebase, Some(branch)) => say!("stopped-rebasing-branch", branch),
        (Kind::Rebase, None) => say!("stopped-rebasing").to_owned(),
        (Kind::CherryPick, _) => say!("stopped-cherry-picking").to_owned(),
        (Kind::Revert, _) => say!("stopped-reverting").to_owned(),
        (Kind::Mailbox, _) => say!("stopped-applying-patches").to_owned(),
    };
    let mailbox = operation.kind == Kind::Mailbox;
    let progress = match (operation.step, operation.remaining) {
        (Some((step, total)), _) if mailbox => say!("stopped-patch-step", step, total),
        (Some((step, total)), _) => say!("stopped-commit-step", step, total),
        (None, Some(left)) if left > 1 => say!("stopped-commits-to-go", left),
        _ => String::new(),
    };
    let state = match conflicts {
        0 if operation.editing => match operation.current {
            Some(id) => say!("stopped-amending-at", commit = id.short(7)),
            None => say!("stopped-amending").to_owned(),
        },
        // Said so that it is true of a patch that did not apply and of one
        // whose conflicts were resolved to nothing alike.
        0 if stuck(operation) && mailbox => say!("stopped-patch-stuck").to_owned(),
        0 if stuck(operation) => say!("stopped-nothing-staged").to_owned(),
        // A patch whose changes are staged: it goes on with them.
        0 if mailbox => say!("stopped-patch-staged").to_owned(),
        0 => say!("stopped-no-conflicts").to_owned(),
        n => conflicts_said(
            n,
            operation
                .conflicts
                .iter()
                .filter(|conflict| conflict.reused)
                .count(),
        ),
    };
    say!("stopped-sentence", what, progress, state)
}

/// What is said of the files in conflict: how many, and how many of them
/// hold a resolution remembered from last time (rerere) and only wait to
/// be accepted.
fn conflicts_said(all: usize, reused: usize) -> String {
    let files = say!("stopped-files-in-conflict", count = all);
    match (all, reused) {
        (_, 0) => files,
        (1, _) => say!("stopped-one-resolved-as-last-time", files),
        (all, reused) if reused >= all => say!("stopped-all-resolved-as-last-time", files),
        (_, reused) => say!("stopped-some-resolved-as-last-time", files, reused),
    }
}

#[cfg(test)]
mod tests {
    use bitbraid_git::sequence::{Conflict, ConflictKind, InProgress, Kind};

    use super::{blocked, conflicts_said, sentence};

    fn operation(kind: Kind, conflicts: usize) -> InProgress {
        InProgress {
            kind,
            current: None,
            step: None,
            remaining: None,
            branch: None,
            onto: None,
            original_head: None,
            ours: String::new(),
            theirs: String::new(),
            conflicts: (0..conflicts)
                .map(|n| Conflict {
                    path: format!("file-{n}"),
                    kind: ConflictKind::BothModified,
                    base: None,
                    ours: None,
                    theirs: None,
                    submodule: false,
                    reused: false,
                })
                .collect(),
            can_skip: kind != Kind::Merge,
            editing: false,
            // As git would have it for a merge and a rebase; the tests of
            // the others say what is staged.
            ready: conflicts == 0 && matches!(kind, Kind::Merge | Kind::Rebase),
        }
    }

    #[test]
    fn a_merge_says_how_many_files_are_in_conflict() {
        let merge = operation(Kind::Merge, 2);
        assert_eq!(sentence(&merge, None), "Merging: 2 files in conflict");
        assert!(blocked(&merge));
        let one = operation(Kind::Merge, 1);
        assert_eq!(sentence(&one, None), "Merging: 1 file in conflict");
        let none = operation(Kind::Merge, 0);
        assert_eq!(sentence(&none, None), "Merging: nothing left in conflict");
        assert!(!blocked(&none));
    }

    #[test]
    fn a_rebase_says_its_branch_and_the_step_it_is_on() {
        let mut rebase = operation(Kind::Rebase, 1);
        rebase.branch = Some("topic".to_owned());
        rebase.step = Some((2, 5));
        assert_eq!(
            sentence(&rebase, None),
            "Rebasing topic, commit 2 of 5: 1 file in conflict"
        );
        // Stopped at a commit to amend it: nothing is wrong.
        let mut editing = operation(Kind::Rebase, 0);
        editing.editing = true;
        assert_eq!(
            sentence(&editing, None),
            "Rebasing: stopped for a commit to be amended"
        );
        assert!(!blocked(&editing));
    }

    #[test]
    fn picks_and_patches_say_what_is_left() {
        // A pick makes a commit of what is staged: with something staged
        // it is ready, with nothing it waits although no file is in
        // conflict.
        let mut pick = operation(Kind::CherryPick, 0);
        pick.remaining = Some(3);
        pick.ready = true;
        assert_eq!(
            sentence(&pick, None),
            "Cherry-picking, 3 commits to go: nothing left in conflict"
        );
        assert!(!blocked(&pick));
        pick.ready = false;
        assert_eq!(
            sentence(&pick, None),
            "Cherry-picking, 3 commits to go: nothing is staged for this commit; stage what it \
             should change and continue, or skip it"
        );
        assert!(blocked(&pick));
        let mut revert = operation(Kind::Revert, 0);
        assert!(blocked(&revert));
        assert!(sentence(&revert, None).starts_with("Reverting: nothing is staged"));
        revert.ready = true;
        assert!(!blocked(&revert));
        // A patch that does not apply waits although no file is in conflict.
        let mut patches = operation(Kind::Mailbox, 0);
        patches.step = Some((1, 4));
        assert_eq!(
            sentence(&patches, None),
            "Applying patches, patch 1 of 4: stopped at this patch; stage what it should \
             change and continue, or skip it"
        );
        assert!(blocked(&patches));
        // Once what it should change is staged, it is ready and says so.
        patches.ready = true;
        assert_eq!(
            sentence(&patches, None),
            "Applying patches, patch 1 of 4: what this patch changes is staged; ready to \
             continue"
        );
        assert!(!blocked(&patches));
        // In conflict is never ready, whatever is staged beside it.
        let conflicted = operation(Kind::Mailbox, 1);
        assert!(blocked(&conflicted));
    }

    #[test]
    fn what_was_resolved_as_last_time_is_counted() {
        assert_eq!(conflicts_said(1, 0), "1 file in conflict");
        assert_eq!(conflicts_said(3, 0), "3 files in conflict");
        assert_eq!(
            conflicts_said(1, 1),
            "1 file in conflict, resolved as last time: accept it or change it"
        );
        assert_eq!(
            conflicts_said(3, 1),
            "3 files in conflict, 1 of them resolved as last time"
        );
        assert_eq!(
            conflicts_said(2, 2),
            "2 files in conflict, all resolved as last time: accept them or change them"
        );
        // More than there are is all of them.
        assert!(conflicts_said(2, 9).contains("all resolved"));
        // The sentence counts them itself.
        let mut merge = operation(Kind::Merge, 2);
        if let Some(first) = merge.conflicts.first_mut() {
            first.reused = true;
        }
        assert_eq!(
            sentence(&merge, None),
            "Merging: 2 files in conflict, 1 of them resolved as last time"
        );
        assert_eq!(
            crate::views::changes::conflicts_heading(2, 0),
            "Conflicted Files (2)"
        );
        assert_eq!(
            crate::views::changes::conflicts_heading(2, 1),
            "Conflicted Files (2) · 1 resolved as last time"
        );
        assert_eq!(
            crate::views::changes::conflicts_heading(2, 2),
            "Conflicted Files (2) · all resolved as last time"
        );
    }

    #[test]
    fn a_merge_that_is_a_step_of_a_finish_says_what_it_is_part_of() {
        let merge = operation(Kind::Merge, 1);
        assert_eq!(
            sentence(&merge, Some("Finishing feature search")),
            "Finishing feature search, merging: 1 file in conflict"
        );
        // Nothing else is a step of one.
        let rebase = operation(Kind::Rebase, 1);
        assert_eq!(
            sentence(&rebase, Some("Finishing feature search")),
            "Rebasing: 1 file in conflict"
        );
    }
}
