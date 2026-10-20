### Merge, rebase, cherry-pick and revert.

## The word for an operation that is in the middle, in a sentence ("the
## rebase is finished"). Also the first words of an error about it.

sequence-kind-merge = merge
sequence-kind-rebase = rebase
sequence-kind-cherry-pick = cherry-pick
sequence-kind-revert = revert
# Patches from a mailbox, applied by git am.
sequence-kind-mailbox = patch series

## What each side of a conflict is, as the conflict view names it.

# A commit whose id is not known.
sequence-unknown-commit = a commit
# A branch, and the first seven digits of the commit it is at.
sequence-branch-at = { $branch } at { $commit }
sequence-current-branch = the current branch
# The subject line of the patch being applied.
sequence-the-patch = the patch "{ $subject }"
sequence-patch-being-applied = the patch being applied
# The commit a revert takes back (seven digits of its id, or "a commit").
sequence-undone = { $commit } undone

## Which operation was refused: the first words of the error.

sequence-what-continue = continue
sequence-what-skip = skip
sequence-what-abort = abort
sequence-what-resolve = resolve
sequence-what-reopen = reopen

## Why going on, skipping, aborting or resolving was refused, or what
## stopped an operation.

sequence-nothing-in-progress = nothing is in progress
# The operation (see above), and why its commit could not be made.
sequence-commit-not-made = the { $what } could not make its commit: { $detail }. It is left where it stopped and nothing of it is lost: put that right and continue, or abort
# What the hook's failure says (a whole sentence), and the operation.
sequence-hook-refused = { $err } The { $what } is left where it stopped: continue once the hook has nothing against it, or abort.
sequence-still-in-conflict =
    { $count ->
        [one] { $count } file is still in conflict
       *[other] { $count } files are still in conflict
    }
sequence-merge-nothing-to-skip = a merge has nothing to skip
sequence-not-in-conflict = { $path } is not in conflict
sequence-markers-left = { $path } still has conflict markers in it
sequence-not-in-conflict-here = { $path } was not in conflict in this operation
# The file, and what git merge-file wrote to standard error.
sequence-merge-failed = git could not merge { $path }: { $stderr }
sequence-result-has-markers = the result for { $path } still has conflict markers in it

## How git am ended, when it failed.

# The number git am ended with.
sequence-exit-status = exit status: { $code }
sequence-ended-by-signal = ended by a signal

## Why a conflict can only be resolved by taking one side's file whole.

sequence-whole-submodule = This is a submodule: keep the commit one side points it at. Its own files are not changed by that.
sequence-whole-deleted = One side deleted this file: keep one side's version or remove it.
sequence-whole-not-text = This is not a text file, or it is too large: keep one side's version.

## The steps of resolving a conflict; each is followed by what failed.

sequence-reading-resolved-file = reading a resolved file
sequence-making-room = making room for the merge tool
sequence-writing-version = writing a version for the merge tool
