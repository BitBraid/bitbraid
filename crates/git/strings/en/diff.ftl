### Diffs.

## Which input was refused: the first words of the error.

diff-what-revision = revision
diff-what-side = diff side
diff-what-file = file

## Why.

# The revision, quoted, and the kind of object it names (blob, …).
diff-revision-is-kind = { $rev } is a { $other }
diff-side-not-tree = tree diffs need commits or trees; use `diff_file` for index and worktree
diff-too-large = { $path } is too large to show

## The steps of a diff; each is followed by what failed.

diff-loading-revision = loading a revision
diff-peeling-tag = peeling a tag
diff-loading-commit = loading a commit
diff-loading-commit-tree = loading a commit tree
diff-loading-tree = loading a tree
diff-loading-blob = loading a blob
diff-preparing-tree-diff = preparing a tree diff
diff-diffing-trees = diffing trees
diff-looking-up-path = looking up a path
diff-loading-index = loading the index
diff-reading-link = reading a symbolic link
diff-reading-worktree-file = reading a worktree file
