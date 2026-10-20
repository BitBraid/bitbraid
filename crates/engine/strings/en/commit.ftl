### Commits.

## The commands: what the palette and the command line call each, and what it does.

commit-create = Commit
commit-create-description = Commit the index; params: message, stage_all (bool: stage everything first), no_verify (leave out the pre-commit and commit-msg hooks; the ones that were there are named in skipped_hooks), sign, allow_empty
commit-amend = Amend commit
commit-amend-description = Replace the tip commit with the index and message; params: message, keep_author, no_verify (as for commit.create), sign
commit-template = Commit template
commit-template-description = The configured commit.template text, if any
commit-people = Recent people
commit-people-description = Authors and co-authors from recent history for the co-author picker; params: limit
commit-undo-last = Undo
commit-undo-last-description = Reverse the last journaled action
commit-redo-last = Redo
commit-redo-last-description = Replay the last undone action
commit-undo-peek = Undo status
commit-undo-peek-description = What undo and redo would do next

## Journal entries: what the undo menu and the activity log call each change.

# The commit is its short id; the summary is the first line of its message.
commit-entry-create = Commit { $commit }: { $summary }
commit-entry-amend = Amend { $commit }

## Refusals.

commit-unborn-undo = the first commit on an unborn branch cannot be undone yet
