### Single files: open, show in the file manager, restore, read, write.

## The commands: what the palette and the command line call each, and what it does.

file-open = Open file
file-open-description = Open a worktree file with a command or the system's opener, or with rev the file as it is in that commit, from a read-only copy outside the repository; params: path, rev, command (words, with {"{"}path{"}"} and {"{"}line{"}"} filled in), line
file-reveal = Show in file manager
file-reveal-description = Show a worktree file in the system's file manager, or with a command; params: path, command (words, with {"{"}path{"}"} filled in with the folder the file is in)
file-restore = Restore file at revision
file-restore-description = Put a worktree file back to what it was in a commit, leaving the index alone; undoable; params: path, rev
file-bytes = File content
file-bytes-description = The bytes of a file on one side of a diff, base64; params: path, side (worktree, index, HEAD or a revision; worktree when absent), max (refuse a file larger than this many bytes)
file-write = Save edited file
file-write-description = Replace the text of an existing worktree file; undoable; params: path, text, base (blob id the text was edited from: refused when the file changed since)

## Journal entries: what the undo menu and the activity log call each change.

file-entry-write = Edit { $path }
# The commit is its short id.
file-entry-restore = Restore { $path } from { $commit }

## Refusals.

# A program of the user's (an editor, a file manager) did not start; the error is what the system said.
file-cannot-start = cannot start { $program }: { $error }
# The folder copies of files are written to.
file-cannot-make-folder = cannot make { $path }: { $error }
file-not-own-folder = { $path } is not a folder of its own
file-not-a-commit = must name a commit
# The commit is its short id.
file-not-in-commit = { $path } is not a file in { $commit }
file-names-no-file = { $path } names no file
file-cannot-write = cannot write { $path }: { $error }
# The command to open a file with has no program in it.
file-command-empty = is empty
