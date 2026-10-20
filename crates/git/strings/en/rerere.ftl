### Recorded resolutions (rerere).

## Which input or operation was refused: the first words of the error.

rerere-what-rerere = rerere
# A record of how a conflict was resolved, named by its id.
rerere-what-record = record

## Why forgetting a resolution was refused.

rerere-not-on = rerere is not on in this repository; nothing is recorded that could be forgotten
rerere-not-in-conflict = { $path } is not in conflict; there is no resolution of it to forget
# The name given, quoted.
rerere-not-a-record = { $id } is not the name of a recorded conflict
rerere-being-resolved = this is the conflict in { $path } that is being resolved now; there is no resolution of it yet to forget
# Why the system could not remove the record's directory.
rerere-not-removed = the record could not be removed: { $err }
