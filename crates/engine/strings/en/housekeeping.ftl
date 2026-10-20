### Housekeeping.

## The commands: what the palette and the command line call each, and what it does.

housekeeping-branch-overview = Branch overview
housekeeping-branch-overview-description = Every local branch with its description, the time of its last commit, and whether it is stale, merged, left without its upstream or checked out elsewhere; params: base (what merged is asked about), stale_days (90 when absent), now (unix seconds, the clock when absent)
housekeeping-branch-describe = Describe branch
housekeeping-branch-describe-description = Set the description of a local branch; params: name, description (empty removes it)
housekeeping-branch-delete-merged = Delete merged branches
housekeeping-branch-delete-merged-description = Delete every local branch that is merged into a base, except the base, the current branch, protected ones and those checked out in a worktree; params: base, protected (names that stay), dry_run (bool: only say what would go)

## Journal entries: what the undo menu and the activity log call each change.

housekeeping-entry-describe = Describe branch { $name }
housekeeping-entry-undescribe = Remove the description of branch { $name }
# A merged branch deleted in a clean-up.
housekeeping-entry-delete = Delete branch { $name }
