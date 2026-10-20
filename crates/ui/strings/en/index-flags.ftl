### Files git is told to assume unchanged or skip.

index-flags-heading = Files git was told about
index-flags-close = Close
# Over the list. "Look again" is the name of the button of a row.
index-flags-explained = Files marked "assume unchanged" or "skip worktree" are not looked at, so changes to them never show in the status. A file can be marked from its menu among the changes. Here a mark is taken off with "Look again", or with ↓ ↑ and Enter.
index-flags-none = No file is marked.
index-flags-truncated = There are more than are listed.
# Takes the marks off every file listed; $hidden is how many are marked.
index-flags-look-again-all = Look at all { $hidden } again

## A row: what the file is marked as, and the button that takes the mark off.

index-flags-assumed-and-skipped = assumed unchanged, and skipped
index-flags-assumed = assumed unchanged
index-flags-skipped = skipped
index-flags-intent-to-add = to be added; unstage it to take that back
index-flags-look-again = Look again
index-flags-look-again-tooltip = Take the mark off: git looks at the file again

## Said when the marks are off.

index-flags-cleared = Git looks at { $path } again
index-flags-cleared-all =
    { $files ->
        [one] Git looks at the file again
       *[other] Git looks at all { $files } files again
    }
