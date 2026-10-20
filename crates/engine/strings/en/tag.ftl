### Tags.

## The commands: what the palette and the command line call each, and what it does.

tag-create = New tag
tag-create-description = Create a tag; params: name, target (HEAD when absent), message (makes it annotated), sign (bool, needs a message)
tag-annotate = Annotate tag
tag-annotate-description = Give a lightweight tag a message: it is made anew as an annotated tag at the same commit; refused for a tag that has a message; undoable; params: name, message
tag-delete = Delete tag
tag-delete-description = Delete a local tag; params: name
tag-push = Push tag
tag-push-description = Push one tag or all of them; params: name (all when absent), remote (origin, or the only one, when absent)
tag-delete-remote = Delete tag on the remote
tag-delete-remote-description = Delete a tag on a remote and keep the local one; params: name, remote (origin, or the only one, when absent)

## Journal entries: what the undo menu and the activity log call each change.

tag-entry-create = Tag { $name }
tag-entry-annotate = Annotate tag { $name }
tag-entry-delete = Delete tag { $name }
