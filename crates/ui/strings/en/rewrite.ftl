### Rewriting commits: reword, squash, drop, reorder.

## The ways to merge, as a dialog offers them, each with what it does in fainter print.

rewrite-mode-ff = Fast-forward if possible
rewrite-mode-ff-note = else a merge commit
rewrite-mode-ff-only = Fast-forward only
rewrite-mode-ff-only-note = refused when the branch cannot simply move forward
rewrite-mode-no-ff = Always a merge commit
rewrite-mode-no-ff-note = also where the branch could move forward
rewrite-mode-squash = Squash
rewrite-mode-squash-note = all its changes as staged work, no merge commit
rewrite-mode-no-commit = Without committing
rewrite-mode-no-commit-note = merged and left to look at: continue commits it

## What the status bar says while an operation runs, and what is said when it worked. A commit is named by the start of its id, a branch or a tag by its name.

rewrite-fast-forwarding = Fast-forwarding
rewrite-fast-forwarded-branch = Fast-forwarded { $branch } to { $to }
rewrite-fast-forwarded = Fast-forwarded to { $target }
rewrite-merging = Merging
rewrite-merged = Merged { $target }
rewrite-squashed-staged = Squashed { $target } into the staged changes
rewrite-rebasing = Rebasing
rewrite-rebased = Rebased onto { $onto }
rewrite-reverting = Reverting
rewrite-reverted = Reverted { $commit }
rewrite-reverted-many = Reverted { $count } commits
rewrite-cherry-picking = Cherry-picking
rewrite-cherry-picked = Cherry-picked { $commit }
rewrite-picked-staged = Picked { $commit } into the staged changes; nothing is committed
rewrite-cherry-picked-many = Cherry-picked { $count } commits
rewrite-squashing = Squashing
rewrite-squashed-many = Squashed { $count } commits
rewrite-squashed-into-parent = Squashed { $commit } into its parent
rewrite-dropping = Dropping
rewrite-dropped = Dropped { $commit }
rewrite-moving = Moving
rewrite-moved-up = Moved { $commit } up
rewrite-moved-down = Moved { $commit } down
rewrite-moved-many-up = Moved { $count } commits up
rewrite-moved-many-down = Moved { $count } commits down
rewrite-resetting = Resetting
rewrite-reset-to = Reset to { $target }

## The warning before something that would meet conflicts is begun.

rewrite-merging-target = Merging { $target }
rewrite-merge-anyway = Merge anyway
rewrite-rebasing-onto = Rebasing onto { $onto }
rewrite-rebase-anyway = Rebase anyway
rewrite-rebase-caveat = This compares the two tips. A rebase replays its commits one at a time and may stop on other files as well.
# The heading; `what` is one of the two above, such as "Merging topic".
rewrite-would-conflict =
    { $count ->
        [one] { $what } would conflict in { $count } file
       *[other] { $what } would conflict in { $count } files
    }
# `files` is the files named, with commas between.
rewrite-in-conflict = In conflict: { $files }. It stops there and waits for the files to be resolved, or to be aborted.
# After the files named, how many more there are.
rewrite-more-files = { $more } more

## Menu entries. `name` is how the menu calls a branch, a tag or a commit; `here` is the checked-out branch, or HEAD when none is.

rewrite-merge-into = Merge { $name } into { $into }
rewrite-merge-into-with-options = Merge { $name } into { $into } with options…
rewrite-merge-already-there = Merge { $name } (HEAD is already there)
rewrite-rebase-onto = Rebase { $branch } onto { $onto }
rewrite-rebase-onto-with-options = Rebase { $branch } onto { $onto } with options…
rewrite-rebase-onto-interactively = Rebase { $branch } onto { $onto } interactively…
rewrite-rebase-onto-already-there = Rebase onto { $name } (HEAD is already there)
rewrite-rebase-onto-detached = Rebase onto { $name } (HEAD is not on a branch)
rewrite-reset-here = Reset { $here } to this commit
rewrite-reset-here-already-there = Reset { $here } to this commit (HEAD is already there)
rewrite-reset-soft = Soft: keep every change, staged
rewrite-reset-mixed = Mixed: keep every change, unstaged
rewrite-reset-hard = Hard: drop local changes…

## What dropping something dragged on a row of the graph can mean.

rewrite-fast-forward-to = Fast-forward { $branch } to { $target }
rewrite-push-to = Push { $branch } to { $target }
rewrite-cherry-pick-onto = Cherry-pick { $name } onto { $into }
rewrite-start-pull = Start a pull request from { $head } into { $base }…
rewrite-reset-to-soft = Reset { $branch } to { $target } (soft: keep every change, staged)
rewrite-reset-to-mixed = Reset { $branch } to { $target } (mixed: keep every change, unstaged)
rewrite-reset-to-hard = Reset { $branch } to { $target } (hard: drop local changes…)

## The menu of a commit's row. `kind` says whether it is a merge commit, which is asked about first (against which parent).

rewrite-cherry-pick-this =
    { $kind ->
        [merge] Cherry-pick this merge commit…
       *[commit] Cherry-pick this commit
    }
rewrite-cherry-pick-this-already-there =
    { $kind ->
        [merge] Cherry-pick this merge commit (HEAD is already there)
       *[commit] Cherry-pick this commit (HEAD is already there)
    }
rewrite-revert-this =
    { $kind ->
        [merge] Revert this merge commit…
       *[commit] Revert this commit
    }
rewrite-cherry-pick-this-staged =
    { $kind ->
        [merge] Cherry-pick this merge commit without committing…
       *[commit] Cherry-pick this commit without committing
    }
rewrite-cherry-pick-this-staged-already-there =
    { $kind ->
        [merge] Cherry-pick this merge commit without committing (HEAD is already there)
       *[commit] Cherry-pick this commit without committing (HEAD is already there)
    }
rewrite-rebase-interactively = Rebase interactively from here…
rewrite-reword = Reword…
rewrite-drop = Drop…
rewrite-move-up = Move up
rewrite-move-down = Move down
rewrite-rebase-elsewhere = Rebase from here onto another base…
rewrite-squash-into-parent = Squash into its parent
rewrite-squash-into-parent-not-merge = Squash into its parent (not for a merge commit)

## The menu of a row of several selected commits. `among` says whether a merge commit is among them.

rewrite-cherry-pick-many =
    { $among ->
        [merge] Cherry-pick { $count } commits (a merge commit is among them)
       *[none] Cherry-pick { $count } commits
    }
rewrite-revert-many =
    { $among ->
        [merge] Revert { $count } commits (a merge commit is among them)
       *[none] Revert { $count } commits
    }
rewrite-move-many-up =
    { $among ->
        [merge] Move up { $count } commits (a merge commit is among them)
       *[none] Move up { $count } commits
    }
rewrite-move-many-down =
    { $among ->
        [merge] Move down { $count } commits (a merge commit is among them)
       *[none] Move down { $count } commits
    }
rewrite-squash-many =
    { $among ->
        [merge] Squash { $count } commits (a merge commit is among them)
       *[none] Squash { $count } commits
    }
rewrite-rebase-onto-this = Rebase { $branch } onto this commit
rewrite-rebase-onto-this-already-there = Rebase onto this commit (HEAD is already there)
rewrite-rebase-onto-this-detached = Rebase onto this commit (HEAD is not on a branch)
