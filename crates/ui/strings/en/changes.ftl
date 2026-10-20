### The changes panel: what is staged and what is not.

## The top of the panel.

changes-discard-all = Discard what is not staged
changes-discard-selected-tooltip = Discard what is not staged of the selected files
changes-discard-all-tooltip = Discard every change that is not staged
# Followed by the name of the branch, in a frame of its own.
changes-files-on =
    { $count ->
        [one] 1 changed file on
       *[other] { $count } changed files on
    }
changes-layout-path = Path
changes-layout-tree = Tree
changes-order = Order of the files
changes-order-forward = Files A to Z; Z to A
changes-order-reversed = Files Z to A; A to Z
changes-working = Working…

## While an operation is stopped part way: which side comes into which. The line reads as the first word, the incoming side, the second word, the side it comes into ("Rebasing feature onto main").

changes-sides-rebasing = Rebasing
changes-sides-onto = onto
changes-sides-picking = Picking
changes-sides-into = into
changes-sides-reverting = Reverting
changes-sides-in = in
changes-sides-applying = Applying
changes-sides-to = to
changes-sides-merging = Merging

## The headings of the lists, and their buttons.

changes-unstaged-heading = Unstaged Files ({ $count })
changes-staged-heading = Staged Files ({ $count })
changes-stage-all = Stage all
changes-unstage-all = Unstage all
# What of the list is selected.
changes-stage-files =
    { $count ->
        [one] Stage 1 file
       *[other] Stage { $count } files
    }
changes-unstage-files =
    { $count ->
        [one] Unstage 1 file
       *[other] Unstage { $count } files
    }
# The first row of a tree.
changes-collapse-all = Collapse All
changes-expand-all = Expand All
changes-stage-file = Stage File
changes-unstage-file = Unstage File

## The files in conflict.

changes-conflicts-heading = Conflicted Files ({ $all })
changes-conflicts-heading-all-reused = Conflicted Files ({ $all }) · all resolved as last time
# $reused is how many of the $all files hold a resolution rerere remembered.
changes-conflicts-heading-reused = Conflicted Files ({ $all }) · { $reused } resolved as last time
changes-resolve-all = Resolve all
changes-resolve-all-tooltip = Mark every file in conflict resolved as it is now
# What kind of conflict a file is in.
changes-conflict-reused = resolved as last time
changes-conflict-submodule = submodule, at different commits
changes-conflict-both-modified = changed on both sides
changes-conflict-both-added = added on both sides
changes-conflict-deleted-by-them = deleted on their side
changes-conflict-deleted-by-us = deleted on our side
changes-conflict-added-by-us = only on our side
changes-conflict-added-by-them = only on their side
changes-conflict-both-deleted = deleted on both sides
changes-conflict-reused-tooltip = The file holds the resolution this conflict got the last time (rerere). Accept it as it is, or open the file and change it first.
changes-accept = Accept
changes-accept-tooltip = Take the file as it is now for the resolution, and stage it
changes-ours = Ours
changes-theirs = Theirs

## The menu of a file in conflict. $ours and $theirs say what each side is.

changes-open-merge-tool = Open in merge tool
changes-take-ours = Take ours: { $ours }
changes-take-theirs = Take theirs: { $theirs }
changes-mark-resolved = Mark resolved as the file is now
changes-mark-resolved-markers = Mark resolved, conflict markers and all
changes-forget-resolution = Forget the recorded resolution

## The menu of a file, of a folder, of several files.

changes-reopen-conflict = Put back in conflict
changes-stage = Stage
changes-unstage = Unstage
changes-discard = Discard changes
changes-ignore = Ignore
changes-delete-file = Delete file
changes-skip-worktree = Keep my copy out of git's sight (skip worktree)
changes-assume-unchanged = Assume it unchanged
changes-stage-folder = Stage folder
changes-unstage-folder = Unstage folder
changes-open-terminal = Open a terminal here
changes-open-folder = Open folder
# Several files are selected: $count is two or more.
changes-selection-stage = Stage { $count } files
changes-selection-unstage = Unstage { $count } files
changes-selection-discard = Discard { $count } files
changes-selection-ignore = Ignore { $count } files

## What to ignore: the submenu "Ignore".

changes-ignore-this-folder = This folder
changes-ignore-this-file = This file
# $extension is the pattern of the file's extension ("*.log").
changes-ignore-extension = Every { $extension } file
# A folder the file is in.
changes-ignore-folder-above = The folder { $above }/
changes-ignore-pattern = A pattern…
changes-ignore-pattern-local = A pattern for this clone only…
changes-ignore-pattern-global = A pattern for all my repositories…
changes-untrack-folder = Stop tracking what is in it
changes-untrack-file = Stop tracking it (the file stays)
