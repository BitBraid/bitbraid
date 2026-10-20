### Sparse checkout.

## Which input or operation was refused: the first words of the error.

sparse-what-sparse-checkout = sparse checkout
sparse-what-directory = directory
sparse-what-pattern = pattern

## Why a directory or a pattern was refused. The text is quoted.

sparse-too-long = { $text } is too long
sparse-control-characters = { $text } has control characters
sparse-names-no-directory = { $text } names no directory
sparse-not-relative = { $text } has to be relative to the top of the worktree
sparse-outside-worktree = { $text } has to name a directory inside the worktree
sparse-is-pattern = { $text } is a pattern; cone mode takes directories
sparse-empty = { $text } is empty

## Why the checkout was not changed.

sparse-bare = a bare repository has no worktree
# The most directories or patterns taken at once.
sparse-too-many = at most { $most } directories or patterns are taken at once
sparse-cone-takes-directories = the checkout is chosen by directory; give directories, or set patterns instead
sparse-not-sparse = the checkout is not sparse; choose what to check out first
sparse-nothing-to-add = nothing to add
