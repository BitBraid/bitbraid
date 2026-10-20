### Worktrees.

## Which input or operation was refused: the first words of the error.

worktree-what-worktree = worktree
worktree-what-path = worktree path
worktree-what-start-point = start point
worktree-what-lock-reason = lock reason

## Why it was refused. A typed path or revision is quoted in the
## argument; a worktree's path is written out as it is.

worktree-path-empty = must not be empty
# A path or a lock reason that is longer than is taken.
worktree-too-long = it is too long
worktree-path-control = { $text } must not hold control characters
worktree-not-listed = { $text } is not a worktree of this repository
worktree-not-empty = { $path } exists and is not an empty directory
worktree-already = { $path } is a worktree already
worktree-no-branch = there is no branch { $name }
# The branch, and the worktree it is checked out in.
worktree-checked-out-in = { $name } is checked out in { $path }
worktree-branch-exists = { $name } exists already
worktree-names-no-commit = { $rev } does not name a commit
worktree-not-made = { $path } was not made
worktree-main-not-removed = the main worktree cannot be removed
worktree-open-not-removed = { $path } is the worktree that is open; remove it from another one
# The reason is the one given when it was locked.
worktree-locked-because = { $path } is locked ({ $reason }); unlock it first
worktree-locked = { $path } is locked; unlock it first
worktree-gone = the directory { $path } is gone; pruning removes what is left of it
# How many paths are changed or untracked in it (never 0).
worktree-has-changes =
    { $count ->
        [one] { $path } has { $count } changed file; they would be lost
       *[other] { $path } has { $count } changed files; they would be lost
    }
worktree-main-not-locked = the main worktree cannot be locked
worktree-locked-already = { $path } is locked already
worktree-reason-nul = it has a NUL character in it
worktree-main-not-moved = the main worktree cannot be moved
worktree-open-not-moved = this is the worktree that is open; move it from another one
worktree-something-there = there is something at { $path } already
# A folder longer than Git for Windows makes a worktree in, long paths on
# or not: the folder, its length and the longest that is taken.
worktree-folder-too-long-for-windows = { $path } is { $length } characters long, and on Windows git makes no worktree in a folder longer than { $most } characters, even with long paths turned on; choose a shorter folder
worktree-not-where-moved = it is not where it was moved to
worktree-not-locked = { $path } is not locked
