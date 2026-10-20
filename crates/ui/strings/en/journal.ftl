### Undo and redo.

## The undo history.

journal-heading = Undo history
journal-close = Close
journal-undo-to-here = Undo back to here
journal-redo-to-here = Redo up to here
# The line between what can be redone, above, and what can be undone, below.
journal-here = the repository is here
journal-nothing = Nothing has been done here that could be undone.

## What is said after undoing or redoing back to an entry.

journal-undid =
    { $count ->
        [one] Undid { $count } action
       *[other] Undid { $count } actions
    }
journal-redid =
    { $count ->
        [one] Redid { $count } action
       *[other] Redid { $count } actions
    }
# It stopped half-way: `done` is one of the two above, `failed` the entry it stopped at, `why` what went wrong.
journal-stopped = { $done }, then stopped at "{ $failed }": { $why }
# Stands for the entry it stopped at when the answer did not name it.
journal-next-one = the next one
# Stands for what went wrong when the answer did not say.
journal-could-not = it could not be done
