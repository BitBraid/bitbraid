### Interactive rebase.

## The commands: what the palette and the command line call each, and what it does.

interactive-rebase-plan = Plan an interactive rebase
interactive-rebase-plan-description = The commits from one commit up to HEAD, oldest first, to make a plan for; refused across a merge commit or from the first commit of the history; params: first, or onto (a new base: the plan is then for the commits the branch has and that base lacks)
interactive-rebase-outline = Preview an interactive rebase
interactive-rebase-outline-description = Check a plan and say which commits it leads to, without doing anything; params: first, steps ([{"{"}id, action: pick|reword|squash|fixup|edit|drop, message{"}"}])
interactive-rebase-interactive = Interactive rebase
interactive-rebase-interactive-description = Carry out a plan for the commits from one commit up to HEAD: reorder, reword, squash, fixup, edit, drop; params: first, steps ([{"{"}id, action, message{"}"}]), onto (replay them on this base instead of where they are), no_verify (leave out the pre-rebase hook; named in skipped_hooks when it was there)
interactive-commit-reword = Reword commit
interactive-commit-reword-description = Give a commit in the history of the current branch another message, a merge commit or the first commit too; merge commits after it are kept as they are; params: commit, message
interactive-commit-drop = Drop commit
interactive-commit-drop-description = Remove a commit from the history of the current branch; merge commits after it are merged anew, and it is refused when one of them was resolved or changed by hand; params: commit
interactive-commit-squash = Squash commits
interactive-commit-squash-description = Melt commits that follow one another into the oldest of them, messages together; one commit alone goes into its parent; merge commits after them are kept as they are, and a merge commit is not squashed into its parent; params: commits
interactive-commit-move = Move commits
interactive-commit-move-description = Move a commit, or commits that follow one another, one place in the history of the current branch: up, past the commit after them, or down, past the one before; refused across a merge commit, past the first commit of the history, or when there is nothing to move past; params: commits, up (true: towards HEAD)
interactive-commit-rewrite-way = How a commit can be rewritten
interactive-commit-rewrite-way-description = Whether a plain list of commits leads from a commit up to HEAD (plain: an interactive rebase can be planned from it) or a merge commit or the beginning of the history is in the way, and the commit by itself with its message, to reword it; changes nothing; params: commit

## Journal entries: what the undo menu and the activity log call each change. A commit in them is its short id.

interactive-entry-rebase =
    { $count ->
        [one] Interactive rebase of { $count } commit
       *[other] Interactive rebase of { $count } commits
    }
# Onto is the new base as it was named.
interactive-entry-rebase-onto =
    { $count ->
        [one] Interactive rebase of { $count } commit onto { $onto }
       *[other] Interactive rebase of { $count } commits onto { $onto }
    }
interactive-entry-reword = Reword { $commit }
# When the plan had no commit to name.
interactive-entry-reword-commit = Reword commit
interactive-entry-drop = Drop { $commit }
interactive-entry-drop-commit = Drop commit
interactive-entry-squash = Squash { $count } commits
interactive-entry-move-up = Move { $commit } up
interactive-entry-move-down = Move { $commit } down
interactive-entry-move-many-up = Move { $count } commits up
interactive-entry-move-many-down = Move { $count } commits down

## Refusals.

interactive-no-start = say where the plan begins (first), or the new base (onto)
interactive-squash-how-many = between 1 and 100 commits are squashed at a time
interactive-move-how-many = between 1 and 100 commits are moved at a time
# None of the commits named could be read.
interactive-none-readable = none of them could be read
interactive-squash-not-a-run = they do not follow one another in the history of the current branch, or one is named twice
interactive-squash-not-in-history = they are not all in the history of the current branch
