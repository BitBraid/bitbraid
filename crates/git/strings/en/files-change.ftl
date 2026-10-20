### Changes to files.

## Why the files an operation changed were not put back (undo) or
## changed again (redo).

# Which operation was refused.
files-change-what-undo = undo
files-change-what-redo = redo
# The operation that is not finished ("a rebase", "a merge", …).
files-change-in-progress = { $other } is in progress; finish or abort it first
# A path, or "the index" (below); and what happened since ("it was done").
files-change-changed-since = { $path } was changed since { $since }; nothing was changed
# In place of a path, in the message above.
files-change-the-index = the index
files-change-unsound = what the journal says of the files does not hold together; nothing was changed
# A step that can fail; it is followed by what failed.
files-change-writing-prepared = writing a prepared commit message
