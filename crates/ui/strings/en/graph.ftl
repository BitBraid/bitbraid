### The commit graph and its columns.

graph-loading = Loading commits…
# While a large history is still being walked. $rows and $walked are counts with their thousands apart.
graph-walking = Reading the history: { $rows } rows of { $walked } commits read so far
# Under the pointer on an author's circle, when someone else committed. $person is a name and an address.
graph-committed-by = Committed by { $person }

## The headings of the columns.

graph-heading-refs = BRANCH / TAG
graph-heading-graph = GRAPH
graph-heading-summary = COMMIT MESSAGE
graph-heading-author = AUTHOR
graph-heading-date = DATE
graph-heading-id = SHA

## The menu of the columns.

graph-columns = Columns
graph-columns-refs = Branch / tag
graph-columns-graph = Graph
graph-columns-summary = Message
graph-columns-absolute-dates = Absolute dates
graph-columns-date-dividers = Date dividers
graph-columns-commit-dates = Commit dates instead of author dates
graph-columns-topological = Topological order
graph-columns-author-order = Order by author date
graph-columns-reset = Reset columns

## The dividers between the dates of the rows.

graph-date-hours-ago =
    { $hours ->
        [0] less than an hour ago
        [one] { $hours } hour ago
       *[other] { $hours } hours ago
    }
graph-date-days-ago =
    { $days ->
        [one] { $days } day ago
       *[other] { $days } days ago
    }

## The search bar.

# The words before the colons are what a search can be narrowed by; they are typed as they are.
graph-search-placeholder = Search, or author: path: before: …
graph-search-pending = searching…
graph-search-stopped = stopped
# Which of what was found is selected ($at), of how many ($found).
graph-search-match-of = { $at } of { $found }
graph-search-results =
    { $found ->
        [one] { $found } result
       *[other] { $found } results
    }
graph-search-stop = Stop searching
graph-search-only = Only the matches
graph-search-only-off = Show every commit again
graph-search-only-on = Show only what was found
graph-search-previous = Previous match
graph-search-next = Next match
graph-search-close = Close the search

## The row of uncommitted work, and those of other worktrees.

graph-wip-stage-all = Stage all changes
graph-wip-unstage-all = Unstage everything
graph-wip-discard-all = Discard all changes…
graph-wip-commit = Commit what is staged
graph-wip-commit-all = Stage everything and commit
# The field for the summary of the next commit, while it is empty.
graph-wip-summary = Summary
# $name is the worktree's folder, $branch what it has checked out.
graph-worktree-changes-on = Changes in { $name } (on { $branch })
graph-worktree-changes = Changes in { $name }
# The same row while the worktree's folder is being deleted.
graph-worktree-removing = Removing { $name }…

## The field a new branch or tag is named in, on a commit's row.

graph-name-branch = name of the branch
graph-name-tag = name of the tag

## What is done when a label or a commit is dropped on another.

graph-drop-nothing = Nothing to do with these two
# $choice is what is done; $first the branch that is checked out before it.
graph-drop-checks-out-first = { $choice } (checks out { $first } first)

## The menu of a label and of a commit.

graph-pull = Pull
graph-push = Push
graph-set-upstream = Set upstream…
graph-fast-forward = Fast-forward to { $upstream }
graph-fast-forward-elsewhere = Fast-forward to { $upstream } (another worktree has it)
graph-checkout = Check out
graph-checkout-detached = Check out this commit (detached)
# In the submenu of checking out: the label's branch, which another worktree has checked out.
graph-checkout-elsewhere = { $branch } (another worktree has it)
graph-checkout-this-commit = This commit (detached)
# $target is a branch on the commit's row.
graph-checkout-target = Check out { $target }
graph-worktree-from = New worktree from
graph-worktree-from-commit = This commit…
graph-new-branch = New branch here…
# A merge commit is asked about first (against which parent): its entries end in an ellipsis.
graph-cherry-pick = Cherry-pick this commit
graph-cherry-pick-staged = Cherry-pick this commit without committing
graph-cherry-pick-merge = Cherry-pick this merge commit…
graph-cherry-pick-merge-staged = Cherry-pick this merge commit without committing…
graph-cherry-pick-at-head = Cherry-pick this commit (HEAD is already there)
graph-cherry-pick-staged-at-head = Cherry-pick this commit without committing (HEAD is already there)
graph-cherry-pick-merge-at-head = Cherry-pick this merge commit (HEAD is already there)
graph-cherry-pick-merge-staged-at-head = Cherry-pick this merge commit without committing (HEAD is already there)
graph-reword = Reword…
graph-revert = Revert this commit
graph-revert-merge = Revert this merge commit…
graph-drop = Drop…
graph-move-down = Move down
graph-apply-patch = Apply a patch…
graph-rename = Rename…
graph-delete = Delete…
# $why is one of the reasons below.
graph-delete-not = Delete ({ $why })
graph-why-checked-out = it is checked out
graph-why-elsewhere = another worktree has it
graph-why-protected = it is protected
graph-delete-on-remote = Delete { $name } on the remote…
graph-copy-name = Copy the name
graph-copy-id = Copy commit id
graph-copy-ids = Copy { $count } commit ids
graph-copy-link = Copy a link to this commit
graph-save-patch = Save as a patch…
graph-save-patches = Save { $count } commits as patches…
graph-compare = Compare with working tree
graph-select-only-this = Select only this one
# How the entries of a commit no branch is on call it: "Rebase onto this commit".
graph-this-commit = this commit

## What a screen reader is told about the graph.

graph-access-list = Commits
# A label of a row: what it stands for. $name is the name of the stash, tag, branch or worktree.
graph-access-stash = stash { $name }
graph-access-tag = tag { $name }
graph-access-detached = HEAD, detached
graph-access-branch = branch { $name }
graph-access-remote-branch = remote branch { $name }
graph-access-worktree = worktree { $name }
graph-access-checked-out = checked out
# $remotes are the remote branches at the same commit, joined by graph-access-and.
graph-access-also-on = also on { $remotes }
graph-access-and = {" "}and{" "}
graph-access-upstream = upstream
graph-access-in-another-worktree = in another worktree
# What a row is called while its commit has no summary.
graph-access-commit = commit
graph-access-merge = Merge: { $summary }
graph-access-stash-row = Stash: { $summary }
graph-access-by = by { $author }
graph-access-wip = Uncommitted changes
graph-access-wip-none = Uncommitted changes: none
# $kinds are how many files of each kind, joined by commas.
graph-access-wip-counted = Uncommitted changes: { $kinds }
graph-access-changed = { $count } changed
graph-access-added = { $count } added
graph-access-deleted = { $count } deleted
graph-access-renamed = { $count } renamed
