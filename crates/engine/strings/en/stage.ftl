### Staging.

## The commands: what the palette and the command line call each, and what it does.

stage-paths = Stage files
stage-paths-description = Stage whole paths (adds, changes and deletions); params: paths
stage-unstage-paths = Unstage files
stage-unstage-paths-description = Unstage whole paths; params: paths
stage-hunks = Stage hunks or lines
stage-hunks-description = Stage selected hunks or lines of one path; params: path, selection [{"{"}hunk, lines?{"}"}], context
stage-unstage-hunks = Unstage hunks or lines
stage-unstage-hunks-description = Unstage selected hunks or lines of one path; params: path, selection, context
stage-discard-paths = Discard changes
stage-discard-paths-description = Discard worktree changes of whole paths (untracked files are deleted); undoable; params: paths
stage-discard-hunks = Discard hunks or lines
stage-discard-hunks-description = Discard selected hunks or lines of one path; undoable; params: path, selection, context

## Journal entries: what the undo menu and the activity log call each change.

# The count is how many paths were discarded; the path is the first of them, named when it is the only one.
stage-entry-discard =
    { $count ->
        [one] Discard { $path }
       *[other] Discard { $count } files
    }
stage-entry-discard-lines = Discard lines in { $path }

## What is wrong with a parameter, after its name.

stage-paths-empty = must not be empty
