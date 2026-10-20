### An operation in progress (merge, rebase, cherry-pick, revert) and its conflicts.

## The commands: what the palette and the command line call each, and what it does.

sequence-status = Operation in progress
sequence-status-description = The merge, rebase, cherry-pick, revert or mailbox that is unfinished, with its conflicted files
sequence-conflicts = Conflicted files
sequence-conflicts-description = The files in conflict, each with its kind and whether rerere has put a recorded resolution into it (reused)
sequence-continue = Continue
sequence-continue-description = Go on with the unfinished operation once its conflicts are resolved
sequence-skip = Skip
sequence-skip-description = Leave out the commit the operation stopped on and go on
sequence-abort = Abort
sequence-abort-description = Give the unfinished operation up and go back to where it began
sequence-take = Take one side
sequence-take-description = Resolve a conflicted file with one side's version; params: path, side (ours|theirs)
sequence-resolve = Mark resolved
sequence-resolve-description = Resolve a conflicted file with what the worktree holds; params: path, allow_markers (bool: also when conflict markers are left in it)
sequence-detail = Conflict in detail
sequence-detail-description = A conflicted file cut into the stretches that merge and the ones that do not, with each side's lines; params: path
sequence-write = Save resolution
sequence-write-description = Resolve a conflicted file with the given text; params: path, text, allow_markers (bool)
sequence-reopen = Reopen conflict
sequence-reopen-description = Put a resolved file back in conflict; params: path
sequence-resolved = Resolved conflicts
sequence-resolved-description = The files that were in conflict in the operation under way and are resolved now: the ones conflict.reopen can put back in conflict
