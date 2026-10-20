### Resolving a conflict.

merge-tool-choose-a-file = Choose a file that is in conflict to resolve it.
merge-tool-loading = Loading…

## A file that can only be taken whole from one side. $side says what that side is.

merge-tool-take-ours = Take ours: { $side }
merge-tool-take-theirs = Take theirs: { $side }
merge-tool-external = External tool

## The line at the top: the file, how far it is, and what is done to it as a whole.

merge-tool-none-left = (no conflicts left in it)
merge-tool-decided = ({ $done } of { $total } conflicts decided)
merge-tool-search = Search both sides
# What the search found: nothing, which line of how many, or how many lines.
merge-tool-not-found = not found
merge-tool-found-at = { $at } of { $count }
merge-tool-found =
    { $count ->
        [one] 1 line
       *[other] { $count } lines
    }
merge-tool-find-previous = Previous line that has it
merge-tool-find-next = Next line that has it
merge-tool-base = Base
merge-tool-base-tooltip = Show what both sides started from
merge-tool-by-hand = Edit by hand
merge-tool-by-hand-tooltip = The file as it was left, markers and all, in the editor
merge-tool-external-tooltip = Resolve this file in the external merge tool
merge-tool-save = Save
merge-tool-save-tooltip = Write the result and mark the file resolved
merge-tool-close = Close; what was chosen is kept for this file
# Saving was refused because conflict markers are left in the result.
merge-tool-save-anyway = Save with the markers

## The two sides and the result.

merge-tool-ours = Ours
merge-tool-theirs = Theirs
# Before a line of what both sides started from.
merge-tool-base-row = base
merge-tool-result = Result
# Which conflict the steps count from, of how many.
merge-tool-no-conflicts = no conflicts
merge-tool-conflict-at = conflict { $at } of { $total }
merge-tool-conflicts =
    { $total ->
        [one] 1 conflict
       *[other] { $total } conflicts
    }
merge-tool-previous-conflict = Previous conflict
merge-tool-next-conflict = Next conflict
merge-tool-type-result = Type into the result
merge-tool-reset = Reset
merge-tool-reset-remembered = Leave what was remembered and start from the two sides
merge-tool-reset-choices = Take every choice back
