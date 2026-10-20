### Worktrees.

## The commands: what the palette and the command line call each, and what it does.

worktree-list = List worktrees
worktree-list-description = Every worktree with what it has checked out, whether it is locked or prunable, and how many files are changed in it; params: changes (bool, true when absent: count them)
worktree-changes = Uncommitted work in the other worktrees
worktree-changes-description = For each worktree other than the open one: what it has checked out, a stamp (its HEAD and the size and time of its index) and its changed tracked files counted by kind (untracked files are not looked for); params: known ({"{"}path: stamp{"}"}: a worktree that still has that stamp is listed without being looked at), skip (paths not to be looked at now: one whose stamp changed is listed as waiting), at_most (how many to look at, the rest that changed waiting)
worktree-add = New worktree
worktree-add-description = Add a worktree; params: path, and one of branch (an existing local branch), new_branch (with start, HEAD when absent) or detach (bool, with start)
worktree-remove = Remove worktree
worktree-remove-description = Remove a linked worktree; params: path, force (bool: with its changes), delete_branch (bool: its branch too)
worktree-lock = Lock worktree
worktree-lock-description = Keep a linked worktree from being removed or pruned; params: path, reason
worktree-move = Move worktree
worktree-move-description = Move a linked worktree to another folder, which must not exist yet; params: path, to
worktree-unlock = Unlock worktree
worktree-unlock-description = Let a locked worktree be removed again; params: path
worktree-prune = Prune worktrees
worktree-prune-description = Forget the worktrees whose directory is gone

## Journal entries: what the undo menu and the activity log call each change.

# Removing a worktree with its branch: the branch comes back with undo.
worktree-entry-delete-branch = Delete branch { $name }

## What is wrong with the parameters.

worktree-too-many = more worktrees than a repository is looked at for
worktree-branch-with-start = an existing branch is checked out where it is; start goes with new_branch or detach
worktree-start-missing = say what to check out: branch, new_branch or detach
worktree-start-several = give one of branch, new_branch and detach
