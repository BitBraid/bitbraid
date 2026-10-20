### Branches.

## The commands: what the palette and the command line call each, and what it does.

branch-create = Create branch
branch-create-description = Create a branch at a commit or reference; params: name, start (default HEAD), checkout (bool), dirty (refuse|stash|discard)
branch-checkout = Check out
branch-checkout-description = Check out a local branch, a remote branch (tracking it) or any commit or tag (detached); params: target, dirty (refuse|stash|discard)
branch-rename = Rename branch
branch-rename-description = Rename a local branch; params: name, new_name
branch-delete = Delete branch
branch-delete-description = Delete a local branch; params: name, force (also when unmerged), protected (names that are refused)
branch-delete-remote = Delete branch on remote
branch-delete-remote-description = Delete a branch on a remote, if it is there where it was last fetched; undo pushes it back; params: remote, branch, protected (names that are refused)
branch-set-upstream = Set upstream
branch-set-upstream-description = Set the upstream of a local branch to remote/branch, or clear it; params: name, upstream (null clears)
branch-fast-forward = Fast-forward
branch-fast-forward-description = Move a local branch that is not checked out forward to its upstream, or to a commit that is named; params: name, to
branch-tracking = Ahead and behind
branch-tracking-description = For every local branch that is not level with its upstream: how many commits it is ahead and behind

## Journal entries: what the undo menu and the activity log call each change.

branch-entry-create = Create branch { $name }
branch-entry-checkout = Check out { $name }
# The commit is its short id, or what was asked for when it has none.
branch-entry-checkout-detached = Check out { $commit } (detached)
branch-entry-rename = Rename branch { $name } to { $new_name }
branch-entry-delete = Delete branch { $name }
branch-entry-delete-remote = Delete { $remote }/{ $branch } on { $remote }
branch-entry-set-upstream = Set upstream of { $name } to { $upstream }
branch-entry-clear-upstream = Clear upstream of { $name }
branch-entry-fast-forward = Fast-forward { $name }

## Refusals. A name in them is in quotes.

branch-protected = { $name } is a protected branch
branch-head-moved = HEAD is not where this action left it; nothing was changed
# A branch made along with settings has moved since; undo leaves both.
branch-made-moved = branch { $name } is not where this action left it; nothing was changed
branch-moved-since = branch { $name } is not where it was; nothing was changed
branch-exists-again = branch { $name } exists again; nothing was changed
branch-remote-moved-since = { $remote }/{ $branch } is not where it was; nothing was changed

## Undoing and redoing files a change wrote.

# The operation a refusal names, before a colon.
branch-undo = undo
branch-redo = redo
# What follows "<file> was changed since".
branch-since-done = it was done
branch-since-undone = it was undone
