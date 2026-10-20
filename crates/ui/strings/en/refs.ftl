### The references panel: branches, remotes, tags, stashes.

## The headings of the sections.

refs-section-gitflow = GITFLOW
refs-section-local = BRANCHES
refs-section-remote = REMOTES
refs-section-worktrees = WORKTREES
refs-section-stashes = STASHES
refs-section-pulls = PULL REQUESTS
refs-section-issues = ISSUES
refs-section-tags = TAGS
refs-section-submodules = SUBMODULES
# The issues section, named after the provider whose issues it shows.
refs-section-github-issues = GITHUB ISSUES
refs-section-gitlab-issues = GITLAB ISSUES

## The buttons of a heading, under the pointer.

refs-head-add-remote = Add a remote
refs-head-refresh-pulls = Read the pull requests again
refs-head-create-pull = Create a pull request of the checked-out branch
refs-head-refresh-issues = Read the issues again
refs-head-create-issue = Create an issue (issues are made at the provider for now)
refs-head-count-worktrees = Look again at what is changed in each
refs-head-add-worktree = Add a worktree
refs-head-add-submodule = Add a submodule

## The menu of a heading.

refs-section-hide-all = Hide all of them from the graph
refs-section-show-all = Show all of them in the graph
refs-section-maximise = Maximise this section
# Puts the sections back in the order they start out in.
refs-section-reset = As they start out
refs-move-up = Move up
refs-move-down = Move down
refs-folders-close = Close all folders
refs-folders-open = Open all folders
refs-add-remote = Add a remote…

## The top of the panel.

refs-fold = Fold this panel to a strip
refs-unfold = Open the references panel
# The panel folded because the window is narrow: its button is off.
refs-unfold-narrow = The window is too narrow for the references panel beside the graph and the details: a mark below opens its section
# The filter's placeholder. $key is the key that puts the cursor in it.
refs-filter-key = Filter ({ $key })
refs-filter = Filter references
refs-loading = Loading…
# Before how many references a filter found of all.
refs-top-filtering = Filtering
refs-top-clear-filter = Clear filter
# Before how many references are shown alone (of all).
refs-top-solo = Showing only
# Before how many references the graph shows (of all).
refs-top-viewing = Viewing
refs-top-show-all = Show All

## A row.

# What a screen reader is told of the checked-out branch's row. $name is its name.
refs-row-checked-out = { $name }, checked out
# The button that opens a row's menu, for a screen reader. $name is the row's name.
refs-row-menu = Menu of { $name }
# Beside a stash: the branch it was made on.
refs-stash-on = on: { $branch }
# Beside a worktree that is locked.
refs-worktree-locked = locked
# Beside a branch whose upstream is gone.
refs-branch-gone = gone
# Beside a branch whose last commit is older than the preferences allow.
refs-branch-stale = stale

## How long ago a branch's last commit was, in as few letters as say it.

refs-age-now = now
refs-age-minutes = { $count }m
refs-age-hours = { $count }h
refs-age-days = { $count }d
refs-age-months = { $count }mo
refs-age-years = { $count }y

## The tooltip of a branch's row, a line each.

# Merging the branch into the checked-out one would conflict in so many files.
refs-conflict-mark =
    { $files ->
        [one] conflicts in { $files } file
       *[other] conflicts in { $files } files
    }
# $mark is refs-conflict-mark.
refs-tip-conflicts = A merge into the checked-out branch: { $mark }
refs-tip-follows-gone = Follows { $upstream }, which is gone
refs-tip-follows = Follows { $upstream }
refs-tip-followed-gone = What it followed is gone
refs-tip-last-now = Last commit just now
# $age is one of the refs-age messages.
refs-tip-last = Last commit { $age } ago
refs-tip-stale-now = Stale: last commit just now
# $age is one of the refs-age messages.
refs-tip-stale = Stale: last commit { $age } ago
refs-tip-stale-old = Stale: its last commit is old

## The menu of a branch.

refs-pull-nothing = Pull (it follows nothing)
refs-pull-from = Pull from { $upstream }
# For a branch that is not checked out.
refs-pull-fast-forward = Pull (fetch, then fast-forward to { $upstream })
refs-push-to = Push to { $upstream }
refs-push-publish = Push (publishes it, and it follows what was pushed)
refs-set-upstream = Set upstream…
refs-clear-upstream = Clear upstream (stop following { $upstream })
refs-clear-upstream-nothing = Clear upstream (it follows nothing)
refs-push-lease = Push with lease…
refs-push-force = Force push…
refs-fast-forward = Fast-forward to { $upstream }
refs-fast-forward-checked-out = Fast-forward to { $upstream } (it is checked out: pull)
refs-checkout = Check out
# For a tag.
refs-checkout-detached = Check out (detached)
refs-checkout-checked-out = Check out (it is checked out)
refs-new-branch = New branch here…
# A merge commit is asked about first (against which parent): its entries end in an ellipsis.
refs-cherry-pick = Cherry-pick this commit
refs-cherry-pick-staged = Cherry-pick this commit without committing
refs-cherry-pick-merge = Cherry-pick this merge commit…
refs-cherry-pick-merge-staged = Cherry-pick this merge commit without committing…
refs-cherry-pick-at-head = Cherry-pick this commit (HEAD is already there)
refs-cherry-pick-staged-at-head = Cherry-pick this commit without committing (HEAD is already there)
refs-cherry-pick-merge-at-head = Cherry-pick this merge commit (HEAD is already there)
refs-cherry-pick-merge-staged-at-head = Cherry-pick this merge commit without committing (HEAD is already there)
refs-reword = Reword…
refs-revert = Revert this commit
refs-revert-merge = Revert this merge commit…
refs-drop = Drop…
refs-apply-patch = Apply a patch…
refs-rename = Rename…
refs-delete = Delete…
refs-delete-checked-out = Delete (it is checked out)
refs-delete-protected = Delete (it is protected)
refs-delete-on-remote = Delete on the remote…
refs-copy-name = Copy the name
refs-copy-id = Copy commit id
refs-show-all = Show all references
refs-compare = Compare with working tree
# $current is the checked-out branch, $name the one the menu is of.
refs-rebase-onto = Rebase { $current } onto { $name } interactively…
refs-rebase-onto-detached = Rebase onto it interactively (HEAD is not on a branch)

## What the graph shows of one reference.

refs-show-in-graph = Show in graph
refs-hide-from-graph = Hide from graph
refs-unpin = Unpin
refs-pin = Pin to the left
refs-pin-not-in-graph = Pin to the left (it is not in the graph as shown)
refs-pin-full = Pin to the left (unpin another first)
refs-lane-left = Move its lane to the left
refs-lane-right = Move its lane to the right
refs-solo-stop = Stop showing only this
refs-solo = Show only this in graph

## The entry of focused visibility: which branches it keeps.

refs-focus-detached = Only the checked-out branch (HEAD is on none)
# $branch is the checked-out branch.
refs-focus-alone = Only { $branch } (it follows nothing and has no base)
# $branches are the branches kept, joined by commas.
refs-focus = Only { $branches }

## The menu of a remote.

refs-remote-fetch = Fetch { $name }
refs-remote-edit-urls = Edit URLs…
refs-remote-remove = Remove { $name }…
refs-remote-show = Show the branches of { $name } in graph
refs-remote-hide = Hide the branches of { $name } from graph
refs-remote-solo-stop = Stop showing only { $name }
refs-remote-solo = Show only the branches of { $name } in graph

## The menu of several selected references.

refs-batch-hide = Hide all { $count } from the graph
refs-batch-show = Show all { $count } in the graph
refs-batch-solo = Show only these { $count } in the graph
refs-batch-select-none = Select none
refs-batch-delete-none = Delete (no local branch that may go is selected)
refs-batch-delete =
    { $count ->
        [one] Delete { $count } branch…
       *[other] Delete { $count } branches…
    }
refs-batch-push-none = Push (no local branch is selected)
refs-batch-push =
    { $count ->
        [one] Push { $count } branch
       *[other] Push { $count } branches
    }

## The menu of a folder.

# The local branches in the folder that may be deleted.
refs-folder-delete =
    { $count ->
        [0] Delete (no branch here may go)
        [one] Delete the { $count } branch in it…
       *[other] Delete the { $count } branches in it…
    }
refs-folder-show = Show it all in the graph
refs-folder-hide = Hide it all from the graph
refs-folder-solo = Show only this in the graph
