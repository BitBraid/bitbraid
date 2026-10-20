### Blame.

## The steps of blaming a file; each is followed by what failed.

blame-loading-commit = loading a commit
blame-loading-commit-tree = loading a commit tree
blame-looking-up-path = looking up a path
blame-preparing-tree-diff = preparing a tree diff
blame-diffing-trees = diffing two trees
blame-resolving-head = resolving HEAD
blame-blaming-file = blaming a file

## What git's blame said that could not be read.

# Which input could not be read: what git blame wrote.
blame-what = blame
blame-line-before-commit = a line of the file came before its commit
# The name, quoted.
blame-not-file-name = not a file name: { $name }
# The line, quoted.
blame-not-header = not a blame header: { $raw }
# The word, quoted.
blame-not-commit-id = not a commit id: { $id }
# The word, quoted.
blame-not-line-number = not a line number: { $word }
