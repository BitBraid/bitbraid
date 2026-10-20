### Stashes.

## The commands: what the palette and the command line call each, and what it does.

stash-push = Stash
stash-push-description = Set local changes aside; params: message, include_untracked (bool), keep_index (bool), staged_only (bool)
stash-apply = Apply stash
stash-apply-description = Bring a stash's changes back and keep it; undoable; params: stash (commit id, newest when absent), index (bool: restore what was staged as staged)
stash-pop = Pop stash
stash-pop-description = Bring a stash's changes back and drop it; undoable; params: stash (commit id, newest when absent), index (bool)
stash-drop = Drop stash
stash-drop-description = Remove a stash from the list; params: stash (commit id, newest when absent)
stash-branch = Stash to a new branch
stash-branch-description = Create a branch where a stash was made, check it out and pop the stash there, what was staged staged again; two steps of the journal, undoable; params: name, stash (commit id, newest when absent)
stash-reword = Edit stash message
stash-reword-description = Give a stash another message, keeping its place in the list (its commit is made anew with the same changes, author and dates); undoable; params: stash (commit id, newest when absent), message (one line)
stash-show = What a stash holds
stash-show-description = The files a stash brings back, with their changes: tracked files against the commit it was made on, and the untracked files it carries as added; params: stash (commit id; the newest when absent), context
stash-list = List stashes
stash-list-description = The stash list, newest first

## Journal entries: what the undo menu and the activity log call each change.

stash-entry-push = Stash
stash-entry-apply = Apply stash
stash-entry-pop = Pop stash
stash-entry-drop = Drop stash
stash-entry-reword = Edit stash message
# Stashing to a new branch is two entries: the branch made, then the pop onto it.
stash-entry-branch-create = Stash on a new branch: create { $name }
stash-entry-branch-pop = Stash on a new branch: pop onto { $name }

## Refusals.

stash-not-an-id = expected the commit id of a stash
# The operation a refusal names, before a colon.
stash-refused = stash
# The error is why the pop failed.
stash-branch-not-popped = the branch { $name } was made and checked out, and the stash is still in the list: it could not be brought back there ({ $error })
stash-not-newest = the stash is no longer the newest one; nothing was changed
stash-nothing-any-more = there is nothing to stash any more
