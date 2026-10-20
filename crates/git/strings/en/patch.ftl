### Patches.

## Which operation or input was refused: the first words of the error.

patch-what-patch-file = patch file
patch-what-patch = patch
patch-what-apply = apply
patch-what-apply-mailbox = apply mailbox
patch-what-export = export
patch-what-commits = commits
patch-what-commit = commit
patch-what-from = from
patch-what-to = to
patch-what-file = file
patch-what-directory = directory
patch-what-destination = destination

## Why a path or a patch file was refused.

patch-empty = must not be empty
patch-too-long = it is too long
# The text, quoted.
patch-control = { $text } must not hold control characters
patch-unreadable = { $path } is not there, or cannot be read
patch-not-a-file = { $path } is not a file
# The file, and the most mebibytes that are taken.
patch-larger-than-mib = { $path } is larger than { $mib } MiB, which is more than is taken
patch-larger-than-taken = { $path } is larger than is taken

## Applying a patch.

# In place of what git said, when it said nothing.
patch-no-reason = git gave no reason
# Why it does not apply, as git said it.
patch-does-not-apply = the patch does not apply ({ $reason }); nothing was changed
# What git said.
patch-not-applied = git did not apply the patch ({ $said })
patch-gone = the patch is no longer in the repository; nothing was changed
patch-not-a-patch = what the journal names is not a patch; nothing was changed
patch-no-longer-applies = the patch does not apply that way any more ({ $reason }); nothing was changed

## Applying a mailbox.

patch-staged-changes = there are staged changes; commit or unstage them first
# What stopped, in a sentence ("the mailbox could not make its commit").
patch-noun-mailbox = mailbox
# In place of why git stopped, when it said nothing.
patch-no-reason-given = it gave no reason
patch-not-taken = git does not take it: { $words }

## Exporting patches.

# How many commits were asked for, and the most one export takes.
patch-too-many = that is { $count } commits; one export takes at most { $most }
patch-no-commits = no commits were given
patch-no-commit-between = there is no commit with a patch between the two
patch-unresolved = there are unresolved conflicts; resolve them first
patch-nothing-staged = nothing is staged
patch-nothing-changed = no tracked file is changed and unstaged; a new file is part of a patch once it is staged, or marked as intended to be added
patch-exists = { $path } exists already; nothing was written
patch-in-git-dir = that is inside git's own directory; choose a place outside it
# The file or directory to write.
patch-no-directory = the directory { $path } is to be in does not exist
patch-is-directory = { $path } is a directory
patch-no-patch = none of these commits has a patch: a merge commit is left out
# "file" and "dir" are the names of the two kinds of destination.
patch-one-file = uncommitted changes are written to one file; give file, not dir
patch-not-directory = { $path } is not a directory

## The steps of reading and writing patches; each is followed by what
## failed.

patch-opening = opening a patch file
patch-reading = reading a patch file
patch-looking-for-kept = looking for a patch that was kept
patch-reading-kept = reading a patch that was kept
patch-making-room = making room for the export
patch-listing = listing the patches
patch-writing-mailbox = writing the mailbox
patch-reading-patch = reading a patch
patch-placing = putting a patch in place
patch-making-directory = making the directory for the patches
