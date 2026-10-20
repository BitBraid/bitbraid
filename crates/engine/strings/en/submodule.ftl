### Submodules.

## The commands: what the palette and the command line call each, and what it does.

submodule-list = List submodules
submodule-list-description = Every submodule with its URL and branch, the commit recorded and the one checked out, where it stands (not_initialised, in_step, moved, conflicted) and whether its own files are changed
submodule-add = Add submodule
submodule-add-description = Clone a repository into this one as a submodule and stage it; params: url, path, branch, allow_local (bool: let git clone from a local path)
submodule-update = Update submodules
submodule-update-description = Check out the commits the superproject records; params: paths (all when absent), init (bool: clone those that are missing), recursive (bool), allow_local (bool)
submodule-sync = Sync submodule URLs
submodule-sync-description = Take the URLs in .gitmodules over into the clones; params: paths (all when absent), recursive (bool)
submodule-remove = Remove submodule
submodule-remove-description = Remove a submodule and stage that; params: path, force (bool: with its own changes), purge (bool: the clone git keeps of it too)
submodule-forget = Forget a removed submodule
submodule-forget-description = Delete the clone git still keeps, under its own directory, of a submodule that was removed, with every commit that was only there; params: name (the name the submodule had, usually its path)
submodule-changes = Submodule changes
submodule-changes-description = The commits between what two sides of a diff record for a submodule; params: path, old and new (a revision, index, worktree or empty; HEAD and worktree when absent)
