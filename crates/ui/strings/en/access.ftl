### What the window says to a screen reader.

## The names of regions of the window.

access-changes-name = Changed files
access-tabs-name = Tabs
# The tabs pinned before the others: the page of repositories and the inbox.
access-pinned-tabs-name = Pinned tabs
access-terminal-name = Terminal

## What happened to a file, in a word or two.

access-change-added = added
access-change-modified = changed
access-change-deleted = deleted
access-change-renamed = renamed
access-change-copied = copied
access-change-type-changed = changed type
# A file git does not track yet.
access-change-untracked = new
# A file marked to be added (intent to add).
access-change-intent-to-add = to be added
access-change-conflicted = in conflict
# A file that was in conflict and is resolved.
access-change-resolved = resolved

## The rows of the changes and the headings of the references.

# A file's row. $path is the file, $what what happened to it (one of the words above), $side whether that is staged.
access-file-row = { $path }, { $what }, { $side }
access-staged = staged
access-not-staged = not staged
# A heading of the references: $title is the section's name, $count how many it holds.
access-section-heading = { $title }, { $count }
# A folder's row, as a tree of the changes shows it.
access-folder-row = folder { $path }
# A file's row among those in conflict.
access-conflict-row = { $path }, in conflict
