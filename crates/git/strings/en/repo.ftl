### Repositories.

## Opening a repository (repo.rs).

# Looking for a repository found none where it was asked for, nor above.
repo-no-repository = there is no git repository there or in any folder above it
# A folder too long for git on Windows: how many characters it has, and
# the most git works from.
repo-folder-too-long-for-windows = it is { $length } characters long, and on Windows git works in no repository or worktree whose folder is longer than { $most } characters, even with long paths turned on; move it to a shorter folder
# Which operation was refused: the first words of the error.
repo-what-reopen = opening the repository again
# The folder it was opened from, and the git directory found around it instead.
repo-no-longer-readable = { $path } is no longer a repository git can read; { $found } was found around it
# A step, followed by what failed.
repo-reading-head = reading HEAD

## macOS's stand-in for git, with no git behind it (stand_in.rs).

# Said where the git it would run cannot be run.
repo-stand-in-hint = macOS has no git without Apple's Command Line Tools: `xcode-select --install` installs them, or name a git in the preferences

## The paths the index tracks (tracked.rs).

# A step, followed by what failed.
repo-loading-index = loading the index
