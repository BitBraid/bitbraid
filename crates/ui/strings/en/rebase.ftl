### The interactive rebase editor.

## The title row.

rebase-title = Interactive Rebase
# The title when only one commit's message can change.
rebase-title-reword = Reword
# Before the name of the branch whose commits are planned for.
rebase-rebasing = Rebasing
# Before the name of the branch, when only one commit's message can change.
rebase-on = On
# Before the name of the base the commits are moved to.
rebase-onto = onto
rebase-commits =
    { $count ->
        [one] { $count } commit
       *[other] { $count } commits
    }
rebase-cancel = Cancel
rebase-cancel-rebase = Cancel rebase
rebase-loading = Loading…
# The plan the engine sent cannot be read.
rebase-no-beginning = the plan does not say where it begins

## Why only the message of one commit can change, and no plan can be made.

rebase-alone-merge = This is a merge commit: only its message can change. It stays a merge, and the merge commits after it stay as they are.
rebase-alone-first = This is the first commit of the history: nothing is before it to replay the others on, so only its message can change. Merge commits after it stay as they are.
rebase-alone-behind-merge = A merge commit comes after this one, so no plan for the commits in between can be made: only its message can change. The merge commits after it stay as they are.

## What can happen to a commit: the choice at the left of its row. The list of keys under the plan shows each name with its first letter stressed, that letter being the key that chooses it (p, r, s, f, e, d).

rebase-action-pick = Pick
rebase-action-pick-note = keep the commit as it is
rebase-action-reword = Reword
rebase-action-reword-note = keep it under another message
rebase-action-squash = Squash
rebase-action-squash-note = melt it into the commit below, messages together
rebase-action-fixup = Fixup
rebase-action-fixup-note = melt it into the commit below and forget its message
rebase-action-edit = Edit
rebase-action-edit-note = stop after it, to amend it
rebase-action-drop = Drop
rebase-action-drop-note = leave it out

## The rows of the plan.

# Beside a commit the upstream has already.
rebase-pushed = pushed
rebase-published-warning =
    { $count ->
        [one] { $count } of these commits is on { $upstream } already and would be made anew: afterwards { $branch } has to be pushed by force.
       *[other] { $count } of these commits are on { $upstream } already and would be made anew: afterwards { $branch } has to be pushed by force.
    }

## The form under a row that is reworded.

rebase-new-message = New message
rebase-message-placeholder = The new message
rebase-form-cancel = Cancel
rebase-update-message = Update message

## Under the plan: the keys, and the buttons.

rebase-shortcuts = shortcuts:
# After the keys that move a commit up or down the list.
rebase-move-up = Move up
rebase-move-down = Move down
rebase-reset = Reset
rebase-reset-tooltip = Every commit as it is again
rebase-start = Start rebase
rebase-start-without-hooks = Start rebase without hooks
# Carries out the new message of a commit that is by itself.
rebase-reword = Reword

## What there will be: the start button's tooltip, a commit a line; `summary` is the first line of a commit's message.

rebase-preview = What there will be:
rebase-preview-nothing = No commit is kept: the branch goes back to where the plan begins.
rebase-preview-stops = { $summary } (stops here to be amended)
# A commit made of several; `count` is how many.
rebase-preview-from = { $summary } (from { $count } commits)
rebase-preview-from-stops = { $summary } (from { $count } commits; stops here to be amended)
rebase-preview-more = and { $more } more
