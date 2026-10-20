### Staging.

## Which input or operation was refused: the first words of the error.

stage-what-path = path
stage-what-repository = repository
stage-what-file = file
stage-what-selection = selection
stage-what-undo = undo
stage-what-discard = discard
stage-what-edit = edit

## Why it was refused. A path quoted in code is quoted in the argument.

stage-path-outside = { $path } must be relative and inside the worktree
stage-bare = bare repositories have no worktree
stage-not-text = hunk operations need UTF-8 text; use whole-path staging for binary files
stage-binary-stage = binary files can only be staged whole
stage-binary-unstage = binary files can only be unstaged whole
stage-binary-discard = binary files can only be discarded whole
stage-hunks-stale-file = the hunks no longer match the file; refresh and retry
stage-hunks-stale-index = the hunks no longer match the index; refresh and retry
stage-path-not-plain = { $path } must name a file plainly, as git lists it
# The path, and the part of it that is a link or a file.
stage-behind-link = { $path } is behind { $behind }, which is a link or a file and not a folder of the worktree
# The path, and the submodule it is inside.
stage-inside-submodule = { $path } is inside the submodule { $above }; open the submodule and discard it there
stage-folder = { $path } is a folder; discard the files in it
stage-in-conflict = { $path } is in conflict; resolve it, or take one side of it
stage-submodule = { $path } is a submodule; update it to bring it back to the commit that is recorded
stage-symbolic-link = { $path } is a symbolic link, which is discarded whole
stage-too-large = { $path } would be too large to edit here
stage-edit-missing = { $path } is not there; only an existing file is edited
stage-edit-not-plain = { $path } is not a plain file inside the worktree
stage-edit-changed = { $path } changed on disk since it was opened; nothing was written
stage-changed-since = { $path } changed since; nothing was changed

## The steps of staging, unstaging, discarding and editing; each is
## followed by what failed.

stage-opening-libgit2 = opening the repository with libgit2
stage-opening-the-index = opening the index
stage-staging-a-path = staging a path
stage-staging-a-deletion = staging a deletion
stage-writing-the-index = writing the index
stage-loading-head = loading HEAD
stage-unstaging-paths = unstaging paths
stage-unstaging-a-path = unstaging a path
stage-loading-the-index-blob = loading the index blob
stage-reading-a-worktree-file = reading a worktree file
stage-converting-an-id = converting an id
stage-loading-a-commit = loading a commit
stage-loading-a-tree = loading a tree
stage-loading-a-blob = loading a blob
stage-reading-file-metadata = reading file metadata
stage-writing-a-staged-blob = writing a staged blob
stage-reading-a-link-for-backup = reading a link for backup
stage-reading-a-file-for-backup = reading a file for backup
stage-writing-a-backup-blob = writing a backup blob
stage-removing-a-file = removing a file
stage-loading-a-backup-blob = loading a backup blob
stage-creating-directories = creating directories
stage-removing-in-place = removing what is in place of a restored file
stage-restoring-a-link = restoring a link
stage-writing-a-restored-file = writing a restored file
stage-removing-an-untracked-file = removing an untracked file
stage-writing-a-file = writing a file
stage-writing-the-edited-file = writing the edited file
stage-replacing-the-edited-file = replacing the edited file
