### Stashes.

## Which input or operation was refused: the first words of the error.

stash-what-stash = stash
stash-what-undo = undo

## Why it was refused. A stash is named by the start of its commit id.

stash-not-in-list = { $stash } is not in the stash list
stash-none = there is no stash
stash-message-control = the message has control characters in it
stash-staged-and-untracked = only what is staged, or untracked files too: not both at once
# The files in the way, joined with commas.
stash-untracked-in-the-way = { $files } would be overwritten by the stash's untracked files; nothing was changed
# The files that would be in conflict, joined with commas.
stash-would-conflict = it would conflict in { $files }; nothing was changed
stash-gone-nothing-changed = { $stash } is gone or is not a stash; nothing was changed
stash-gone = { $stash } is gone or is not a stash
stash-path-in-conflict = { $path } is in conflict; resolve that first
# A path, or the index (stash-the-index).
stash-changed-since-applied = { $path } was changed since the stash was applied; nothing was changed
# What was changed since a stash was applied, when it is no one path.
stash-the-index = the index
stash-message-empty = the message is empty
stash-message-too-long = the message is too long, or has a line break or another control character in it
stash-unreadable = { $stash } could not be read
stash-no-new-commit = git made no commit for the new message
