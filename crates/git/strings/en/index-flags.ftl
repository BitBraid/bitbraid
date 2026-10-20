### Flags of the index.

## Why a flag of the index was not set or cleared.

# Which operation or input was refused.
index-flags-what-flag = index flag
index-flags-what-paths = paths
index-flags-no-paths = no paths were given
# How many paths were given: always more than one change takes.
index-flags-too-many = { $count } paths are more than one change takes
index-flags-intent-cannot-clear = a file stops being marked as intended to be added when it is staged or unstaged; unstage it instead
index-flags-in-conflict = { $path } is in conflict; resolve it first
index-flags-not-tracked = { $path } is not tracked; this is for files git knows
index-flags-tracked = { $path } is tracked already
index-flags-in-git-dir = { $path } is inside git's own directory
index-flags-folder = { $path } is a folder; name the files in it
index-flags-missing = { $path } is not there
index-flags-ignored = { $ignored } is ignored; take it off the ignore list first
# A path git passed over.
index-flags-not-taken = git did not take { $left }
# A step that can fail; it is followed by what failed.
index-flags-reading-index = reading the index
