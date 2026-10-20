### Large files (Git LFS).

## Which operation or input was refused: the first words of the error.

lfs-what = lfs
lfs-what-pattern = pattern
lfs-what-revision = revision
lfs-what-remote = remote
lfs-what-path = path
lfs-what-answer = answer of git-lfs

## Why git-lfs was not asked.

lfs-bare-no-files = a bare repository has no files to do this with
lfs-not-installed = git-lfs is not installed: install it (the git-lfs package of your system, or from https://git-lfs.com) and try again
lfs-bare-untouched = git-lfs has not been used in this bare repository, and asking it now would write its settings into the repository, which is opened read-only

## Setting LFS up.

# A hook's name, and where its file is.
lfs-hook-in-way = the hook { $name } is in the way and git-lfs would not replace it: add the line `git lfs { $name } "$@"` to { $path } or remove it, then set LFS up again
# Where the hooks are kept.
lfs-hooks-elsewhere = hooks are kept in { $path }, outside this repository, and nothing is written there: add git-lfs to them yourself (`git lfs update --manual` says how)

## What is wrong with a pattern to track. The pattern is quoted.

lfs-pattern-empty = { $pattern } is empty
lfs-too-long = it is too long
lfs-pattern-control = { $pattern } must not hold control characters
lfs-pattern-dash = { $pattern } must not start with a dash
lfs-pattern-negated = { $pattern } starts with `!`, and attributes have no patterns that take away; write `\!` for a name that starts with one
lfs-pattern-folder = { $pattern } names a folder, which attributes do not go into; write `folder/**` for everything in it
lfs-pattern-outside = { $pattern } leads out of the repository

## The attribute file (.gitattributes, the file's name).

# The file, and why.
lfs-attributes-unreadable = { $file } cannot be read: { $err }
lfs-attributes-link = { $file } is a link, which git does not read attributes through; put a file in its place
lfs-attributes-not-file = { $file } is not a file; move what is there out of the way
lfs-attributes-too-large = { $file } is too large to be changed from here
# The file, and what git-lfs said.
lfs-not-written = git-lfs did not write { $file }: { $said }

## The files of a commit, and content to and from a remote.

# Why what git-lfs printed could not be read.
lfs-not-file-list = not a list of files: { $err }
# The revision, quoted.
lfs-not-commit = { $named } does not name a commit
lfs-too-old = this git-lfs is too old to list files in a form that can be read; update it
# The remote, quoted.
lfs-no-such-remote = there is no remote called { $name }
lfs-no-remote = this repository has no remote to exchange LFS files with; add one first
lfs-not-set-up-pull = LFS is not set up for this repository, so the files would be downloaded and not put in place: set it up first, or fetch only
lfs-not-set-up-checkout = LFS is not set up for this repository, so the files would not be put in place: set it up first

## Locks.

# Why what git-lfs printed could not be read.
lfs-not-lock-list = not a list of locks: { $err }
lfs-no-remote-for-locks = this repository has no remote, and locks are kept by the LFS server of one; add a remote first
lfs-bare-locks = git-lfs keeps a copy of the locks it lists inside the repository, and this is a bare repository, which is opened read-only; list them from a clone
lfs-locks-not-listed = the locks could not be listed
# What did not happen (one of the messages here), what a refusal that is
# known means (a sentence), and what git-lfs said.
lfs-with-meaning = { $what }: { $meaning } Git-lfs said: { $said }
# The path, quoted.
lfs-not-worktree-file = { $path } is not the path of a file in the worktree
# A file locked by someone: who holds the lock and since when, as far as
# the server says.
lfs-already-locked = { $path } is already locked
lfs-already-locked-by = { $path } is already locked by { $owner }
lfs-already-locked-since = { $path } is already locked since { $since }
lfs-already-locked-by-since = { $path } is already locked by { $owner } since { $since }
lfs-not-a-file-here = { $path } is not a file here; only a file that is there can be locked
lfs-was-not-locked = { $path } was not locked
lfs-nothing-locked = it did not say what it locked
lfs-not-said-unlocked = git-lfs did not say that it is unlocked
lfs-not-locked = { $path } is not locked
lfs-was-not-unlocked = { $path } was not unlocked
