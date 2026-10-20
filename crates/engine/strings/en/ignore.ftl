### Ignore rules.

## The commands: what the palette and the command line call each, and what it does.

ignore-add = Ignore
ignore-add-description = Add an ignore pattern: either pattern (as typed), or path with what (file, folder or extension), which is written so that it means that path alone; target: repository (.gitignore), local (.git/info/exclude) or global (the user's ignore file). Answers with the pattern, the file, whether it was added, and the tracked files it covers, which stay tracked
ignore-tracked = Tracked files that are ignored
ignore-tracked-description = The files git tracks although an ignore rule covers them; params: limit
ignore-untrack = Stop tracking
ignore-untrack-description = Take files out of the index and leave them on disk (a folder stands for what is tracked in it); staged as deletions, so unstaging takes it back; params: paths

## Refusals. A path in them is in quotes.

ignore-no-extension = { $path } has no extension to ignore
ignore-not-a-worktree-path = { $path } must be a path inside the worktree, on one line, to make a pattern from
ignore-pattern-and-path = give either pattern, or path with what, not both
ignore-pattern-missing = missing: give pattern, or path with what
# The list of paths to stop tracking is empty.
ignore-no-paths = must not be empty
