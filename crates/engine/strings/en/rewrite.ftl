### Rewriting commits.

## The commands: what the palette and the command line call each, and what it does.

rewrite-merge = Merge
rewrite-merge-description = Merge a branch or commit into the current branch; params: target, mode (ff_only|ff|no_ff|squash|no_commit: stop before the merge commit, to be continued or aborted), no_verify (leave out the pre-merge-commit and commit-msg hooks; the ones that were there are named in skipped_hooks)
rewrite-rebase = Rebase
rewrite-rebase-description = Rebase the current branch onto a branch or commit; params: onto, after (only the commits after this one: a range onto another base), autostash (bool: set local changes aside and bring them back), no_verify (leave out the pre-rebase hook; named in skipped_hooks when it was there)
rewrite-predict = Predict conflicts
rewrite-predict-description = The files that merging a branch or commit into the current commit would leave in conflict, without touching anything; params: target, ours (what it is merged into when not the current commit: a pull request's base), aside (bool: write nothing into the repository either, not even the trial merge's own objects)
rewrite-reset = Reset
rewrite-reset-description = Move the current branch to a commit; params: target, mode (soft|mixed|hard|keep)
rewrite-cherry-pick = Cherry-pick
rewrite-cherry-pick-description = Apply commits onto the current branch in the order given; params: commits, no_commit (bool), mainline (parent number, for a merge commit)
rewrite-revert = Revert
rewrite-revert-description = Undo commits with new commits; params: commits, mainline (parent number, for a merge commit)

## Journal entries: what the undo menu and the activity log call each change.

# The target is the branch or commit as it was asked for.
rewrite-entry-merge = Merge { $target }
rewrite-entry-squash-merge = Squash-merge { $target }
rewrite-entry-rebase = Rebase onto { $onto }
# The commit is its short id, or what was asked for when it has none; the mode is git's name for the kind of reset (soft, mixed, hard or keep).
rewrite-entry-reset = Reset to { $commit } ({ $mode })
rewrite-entry-cherry-pick =
    { $count ->
        [one] Cherry-pick { $count } commit
       *[other] Cherry-pick { $count } commits
    }
rewrite-entry-revert =
    { $count ->
        [one] Revert { $count } commit
       *[other] Revert { $count } commits
    }
