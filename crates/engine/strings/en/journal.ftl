### Undo and redo.

## The commands: what the palette and the command line call each, and what it does.

journal-undo-journal = Undo history
journal-undo-journal-description = Everything that can be undone and redone, newest first: id, label, time and kind of each, and which entry undo and redo would take next
journal-undo-to = Undo back to here
journal-undo-to-description = Undo one entry after another until the one named has been undone; stops at the first that cannot be and says how far it came; params: id
journal-redo-to = Redo up to here
journal-redo-to-description = Redo one entry after another until the one named has been redone; stops at the first that cannot be and says how far it came; params: id

## Taking several steps back or forward.

# The operation a refusal names, before a colon, and the verb in journal-no-entry.
journal-undo = undo
journal-redo = redo
# The id is the number of a journal entry; what is journal-undo or journal-redo.
journal-no-entry = there is no entry { $id } to { $what }; nothing was changed
