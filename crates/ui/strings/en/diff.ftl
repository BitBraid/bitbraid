### The diff: text, images, a file's header.

diff-loading = Loading…
diff-no-changes = No textual changes.
# A file that was moved and not changed; $old_path is where it was.
diff-moved-unchanged = Moved here from { $old_path }; what is in it did not change.
# At the end of a line that has no line break after it.
diff-no-newline = ⏎ missing
# The tool cannot be run on a comparison with the worktree.
diff-external-worktree = The external tool compares commits; the working tree is not one.

# The sizes of the two versions of a file.
diff-sizes = { $old } before, { $new } after

## A file too large to be compared at once. $old and $new are the sizes of its two sides.

diff-guard-size-new = { $new }, a new file
diff-guard-size-deleted = { $old }, deleted
diff-guard-size-unknown = of a size that is not known
# $sizes is diff-sizes or one of the three above; $limit the size up to which files are compared.
diff-guard = This file is large ({ $sizes }), so it was not compared: files over { $limit } are compared when asked.
diff-load-anyway = Load anyway

## A binary file: the two versions' sizes and the ids git gives their content.

diff-binary = Binary file; no text diff.
diff-binary-before = before
diff-binary-after = after
diff-binary-size-unknown = size not known
diff-binary-copy-id = Copy the id
# The version is not there (the file is new, or deleted).
diff-binary-not-there = not there
# The short ids of the two versions.
diff-blob-ids = blob { $old } → { $new }
diff-blob-id = blob { $id }

## The buttons of a hunk, and the menu of a line.

diff-discard-hunk = Discard Hunk
diff-discard-lines = Discard Lines
diff-discard-line = Discard Line
diff-stage-hunk = Stage Hunk
diff-stage-lines = Stage Lines
diff-stage-line = Stage Line
diff-unstage-hunk = Unstage Hunk
diff-unstage-lines = Unstage Lines
diff-unstage-line = Unstage Line
diff-copy = Copy
diff-copy-lines = Copy Lines
diff-copy-line = Copy Line
diff-palette = Command Palette

## The controls over the diff.

# Which change the cursor is at, of how many.
diff-change-position = { $at } of { $total }
diff-changes =
    { $total ->
        [one] 1 change
       *[other] { $total } changes
    }
diff-wrap = Wrap long lines
diff-previous-change = Previous change
diff-next-change = Next change
diff-layout-hunks = Only the parts that changed
diff-layout-inline = The whole file in one column
diff-layout-split = The whole file in two columns, before and after
diff-whitespace = Whitespace
diff-whitespace-exact = Whitespace counts
diff-whitespace-ignore-trailing = Whitespace at line ends is ignored
diff-whitespace-ignore-all = Whitespace is ignored
diff-whitespace-ignore-cr = Carriage returns at line ends are ignored
diff-words = Mark the words that changed
diff-external = Open in the external diff tool
diff-refresh = Read the file again

## A submodule: the commit it is recorded at on each side, and the commits between.

diff-submodule-commit = Commit
diff-submodule-message = Commit Message
diff-submodule-not-here = The commits between them are not here: the submodule is not cloned, or has not fetched them yet.
# $count is how many commits, or "more than" so many.
diff-submodule-gained = Gained ({ $count })
diff-submodule-lost = No longer in it ({ $count })
diff-submodule-more-than = more than { $n }
diff-submodule-and-more = and more

## An image: the two versions, one over the other or apart.

diff-image-new = New image
diff-image-old = Old image
diff-image-size-new = New file, { $size }
diff-image-size-deleted = Deleted file, { $size }
diff-image-neither = Neither version can be shown.
# The switches that lay one version over the other.
diff-image-overlay = Overlay
diff-image-swipe = Swipe
diff-image-difference = Difference
diff-image-drag-overlay = Drag right to see more of the new one, left for the old.
diff-image-drag-swipe = The new one is left of the line; drag it.
# $share is the part that differs, in per cent with one decimal; $width and $height in pixels.
diff-image-pixels-differ = { $differing } of { $total } pixels differ ({ $share } %), { $width } × { $height }
diff-image-too-large = These two are too large to compare pixel by pixel.
diff-image-one-version = There is only one version, or one cannot be read as a picture.

## Line endings: the file header's tooltip, and a change of them alone.

# Lines that end in LF and lines that end in CRLF, in one file.
diff-endings-mixed = LF and CRLF mixed
diff-endings-changed = Line endings: { $old_long } before, { $new_long } after
diff-endings-stored = Line endings: { $long } in the worktree, which git stores as { $stored_long } (core.autocrlf or the eol attribute), so that is what is compared
diff-endings = Line endings: { $long }
diff-endings-only-from-to = Only the line endings changed, from { $from } to { $to }.
diff-endings-only = Only the line endings changed.
diff-endings-show-lines = Show the lines
