### A repository's workspace: the views around the graph.

## Buttons used in several places.

workspace-close = Close
workspace-cancel = Cancel
# Saves: what a prompt asks where to write, the file being edited.
workspace-save = Save
workspace-discard = Discard
workspace-delete = Delete
# The button of a question whose answer takes something away.
workspace-remove = Remove
workspace-go-on = Go on

## Moving the content of large files: what the status bar says while it
## runs, and what is said when it is done.

workspace-lfs-fetching = Fetching large files
workspace-lfs-fetched = Fetched the large files
workspace-lfs-pulling = Pulling large files
workspace-lfs-pulled = Pulled the large files
workspace-lfs-pushing = Pushing large files
workspace-lfs-pushed = Pushed the large files
workspace-lfs-checking-out = Checking out large files
workspace-lfs-checked-out = Put the large files that are here in place

## The prompt's two fields until a prompt names what goes in them.

workspace-prompt-name = name
workspace-prompt-message-optional = message (optional)

## Branches.

workspace-upstream-cleared = { $name } follows nothing now
workspace-fast-forwarded = Fast-forwarded { $name }

## Rewriting commits.

workspace-rebasing = Rebasing
workspace-rebased = Rebased
workspace-rewording = Rewording
# The commit's id, cut short.
workspace-reworded = Reworded { $short }

## Locks on large files.

workspace-lfs-locking = Locking
workspace-lfs-locked = Locked { $path }

## Committing.

workspace-committed = Committed
# The hooks that did not run, named in a list.
workspace-committed-without = Committed without running { $hooks }
workspace-stashed-staged = Stashed what was staged; the rest is as it was

## The external merge tool.

workspace-merge-tool-waiting = Waiting for the merge tool
workspace-merge-tool-ended = The merge tool has ended

## What is said when a command is done.

# A checkout happened and the changes that were stashed for it did not
# fit back; done is what was said of the checkout ("Checked out main").
workspace-done-stash-kept = { $done }. Your changes did not fit there; they are in the stash.
# A notice's button that shows the git processes that were run.
workspace-show-log = Show log

# A command that would change a bare repository, opened read-only, is not
# sent; in_short is the line that says the repository is read-only.
workspace-read-only-refused = { $in_short }: nothing here changes it. Clone it to work in the clone.
# A notice's button that undoes what it is about.
workspace-undo = Undo
workspace-undo-not-last = That is no longer the last thing that was done. The undo history goes back as far as it.
workspace-no-editor = No editor is named in the preferences (External tools), so there is nothing to open the folder with.

## An operation, which shows in the status bar while it runs; label is
## what the status bar said ("Fetching").

workspace-working = Working
workspace-operation-stopped = { $label }: stopped
workspace-operation-not-answered = { $label }: git asked and got no answer, so it gave up
workspace-operation-stopped-half-way = { $label }: stopped half-way; see the banner
workspace-operation-stopped-on-conflicts =
    { $count ->
        [one] { $label }: stopped on a conflict in { $count } file
       *[other] { $label }: stopped on conflicts in { $count } files
    }
# The same, when it is a gitflow finish that stopped: the rest of it
# follows once the conflict is resolved.
workspace-finish-stopped-on-conflicts =
    { $count ->
        [one] { $label }: stopped on a conflict in { $count } file. Resolve it and press Continue: the rest of the finish follows by itself
       *[other] { $label }: stopped on conflicts in { $count } files. Resolve it and press Continue: the rest of the finish follows by itself
    }
# Done, and the changes set aside for it did not fit back in (a rebase
# with autostash); done is what was said of it.
workspace-done-autostash-kept = { $done }. Your local changes did not fit back in: they are in the stash, and the files they conflict in are marked.

## Gitflow; kind is the kind of branch (feature, release, hotfix).

workspace-flow-finishing = Finishing
workspace-flow-finishing-branch = Finishing { $kind } { $name }
workspace-flow-finished = Finished { $kind } { $name }

## The row of uncommitted work, all of it at once.

workspace-nothing-to-stage = Nothing is left to stage
workspace-staged-files =
    { $count ->
        [one] Staged { $count } file
       *[other] Staged { $count } files
    }
workspace-nothing-staged = Nothing is staged
workspace-unstaged-files =
    { $count ->
        [one] Unstaged { $count } file
       *[other] Unstaged { $count } files
    }
workspace-nothing-to-discard = There are no unstaged changes to discard
workspace-discard-question =
    { $count ->
        [one] Throw away the unstaged changes to { $count } file? New files go too; what is staged stays.
       *[other] Throw away the unstaged changes to { $count } files? New files go too; what is staged stays.
    }
workspace-discarded =
    { $count ->
        [one] Discarded the changes to { $count } file
       *[other] Discarded the changes to { $count } files
    }

## Jumping to a date or a reference.

workspace-jump-title = Jump to
workspace-jump = Jump
workspace-jump-placeholder = a date (2026-10-01, 3w) or a branch, tag or commit
# text is what was typed.
workspace-jump-nothing = Nothing to jump to for "{ $text }": it is neither a date nor a branch, tag or commit here.

## Gitflow's set-up.

workspace-flow-saved = Gitflow's branches and prefixes are saved
# main and develop are the names of the two branches.
workspace-flow-set-up = Gitflow is set up on { $main } and { $develop }

## Locks on large files, given up.

workspace-lfs-unlocking = Unlocking
workspace-lfs-unlocked = Unlocked { $path }
workspace-lfs-force-unlock-title = Unlock { $path } by force?
workspace-lfs-force-unlock-description = The lock is not yours. Whoever holds it is not told, and may be working on the file: what they push afterwards can collide with what you do.
workspace-lfs-force-unlock = Unlock by force

## Patches written out.

workspace-patches-save-title =
    { $count ->
        [one] Save the commit as a patch
       *[other] Save { $count } commits as patches
    }
workspace-patches-folder = folder to write into
workspace-patches-written =
    { $count ->
        [one] Wrote the patch into { $dir }
       *[other] Wrote { $count } patches into { $dir }
    }
workspace-patch-save-staged = Save the staged changes as a patch
workspace-patch-save-unstaged = Save the changes that are not staged as a patch
workspace-patch-file = file to write
workspace-patch-written = Wrote the changes to { $file }

## Settings of the repository.

workspace-rerere-off = Resolutions of conflicts are no longer remembered here
workspace-rerere-on = Resolutions of conflicts are remembered here from now on and used again when the same conflict comes back
# name is the profile's.
workspace-profile-used = This repository commits as { $name } now
workspace-links-registered = Links that begin with bitbraid:// open here now

## The status bar while operations run.

# After what runs now: how many more wait behind it.
workspace-operations-waiting = (+{ $count } waiting)

## Remotes.

workspace-fetching = Fetching
workspace-fetched = Fetched
workspace-fetched-remote = Fetched { $name }

workspace-remote-urls-title = URLs of { $name }
workspace-remote-url = URL
workspace-remote-push-url = push URL, when it is another one (optional)
workspace-remote-urls-saved = Saved the URLs of { $name }
workspace-remote-remove-question = Remove the remote { $name }? Its branches leave the graph; undo brings it back.
workspace-remote-removed = Removed the remote { $name }

## Pulling and pushing.

workspace-pulling-upstream = Pulling { $upstream }
workspace-pull-anyway = Pull anyway
workspace-pull-caveat = This is the upstream as it was last fetched; the pull fetches first, and what arrives may change the picture.
workspace-pulling = Pulling
workspace-pulled = Pulled
workspace-pushing = Pushing
# what is the branch that was pushed.
workspace-pushed = Pushed { $what }
workspace-push-rejected = The remote has commits this branch does not have: pull them in first.
workspace-pull = Pull
workspace-force-push = Force push

## Checking out.

workspace-checked-out = Checked out { $target }
workspace-dirty-title = Local changes are in the way
workspace-dirty-overwrite = Checking out { $target } would overwrite files you have changed.
workspace-dirty-explained = Stashing sets the changes aside and brings them back after the checkout; if they do not fit there they stay in the stash. Discarding throws them away; undo brings them back.
workspace-dirty-discard = Discard and check out
workspace-dirty-stash = Stash and check out

workspace-rename-branch-title = Rename branch { $name }
workspace-rename = Rename
workspace-renamed-branch = Renamed { $old } to { $new_name }

## Worktrees; name is the name of a worktree's folder.

workspace-worktree-lock-title = Lock the worktree { $name }
workspace-worktree-lock = Lock
workspace-worktree-lock-why = why (optional)
workspace-worktree-locked = Locked the worktree { $name }
workspace-worktree-move-title = Move the worktree { $name }
workspace-worktree-move = Move
workspace-worktree-move-to = the folder it moves to, which is not there yet
workspace-worktree-moving = Moving a worktree
workspace-worktree-moved = Moved the worktree { $name } to { $to }
workspace-worktree-unlocked = Unlocked the worktree { $name }
workspace-worktree-remove-with-branch-question = Remove the worktree { $name } and its branch? Undo brings the branch back.
workspace-worktree-remove-question = Remove the worktree { $name }? Its folder is deleted; its branch stays.
workspace-worktrees-pruned = Forgot the worktrees that are gone
workspace-worktree-remove-anyway-question = { $name } has work in it that would go. Remove it all the same?
# why is what was said of why the branch stayed.
workspace-worktree-removed-branch-kept = Removed the worktree { $name }. Its branch stays: { $why }
workspace-worktree-removed = Removed the worktree { $name }

## Submodules.

workspace-submodule-synced = Synced the URL of { $path }
workspace-submodules-synced = Synced the URLs of the submodules
workspace-local-clone-title = Clone from a path on this machine?
workspace-local-clone-description = Git does not clone a submodule from a local path unless it is told to: a repository made for it can make such a clone read files it should not. Go on if you trust where this one comes from.
workspace-submodules-updating = Updating submodules
workspace-submodule-updated = Updated the submodule { $path }
workspace-submodules-updated = Updated the submodules

## Gitflow's finish, and notes on branches.

# explained says what the finish does, in a sentence or two.
workspace-flow-finish-question = Finish { $kind } { $name }? { $explained }
workspace-flow-finish = Finish
workspace-describe-title = Describe { $name }
workspace-describe-placeholder = what the branch is for (empty removes it)
workspace-description-removed = { $branch } has no description now
workspace-described = Described { $branch }

## Deleting the branches that are merged.

# Where the engine does not name the branch they are merged into.
workspace-base-branch = the base branch
workspace-delete-merged-none = No branch that may go is merged into { $base }
# names is the first twelve, with commas between; more is how many more.
workspace-names-and-more = { $names } and { $more } more
workspace-delete-merged-title =
    { $count ->
        [one] Delete { $count } branch that is merged into { $base }?
       *[other] Delete { $count } branches that are merged into { $base }?
    }
# listed names the branches.
workspace-delete-merged-description = { $listed }. Undo brings each of them back.
workspace-deleted-merged =
    { $count ->
        [one] Deleted { $count } merged branch
       *[other] Deleted { $count } merged branches
    }

## Tags.

workspace-push-tags-title = Push every tag to
workspace-push-tags = Push tags
workspace-annotate-title = Message of the tag { $name }
workspace-annotate = Annotate
workspace-prompt-message = message
workspace-annotated = Annotated tag { $name }
workspace-tag-title = Tag { $name }
workspace-push-tag-title = Push tag { $name } to
workspace-push-tag = Push tag
workspace-pushing-tag = Pushing tag
workspace-pushed-tag-to = Pushed tag { $name } to { $remote }
workspace-pushed-tag = Pushed tag { $name }
workspace-delete-tag-question = Delete tag { $name } here? It stays on the remotes; undo brings it back.
workspace-deleted-tag = Deleted tag { $name }
# Asks which remote to delete it on.
workspace-delete-remote-tag-title = Delete tag { $name } on
# Where no remote is named, there being only one.
workspace-the-remote = the remote
# there is the remote's name, or "the remote".
workspace-delete-remote-tag-question = Delete tag { $name } on { $there }? Who fetched it keeps it.
workspace-deleting-remote-tag = Deleting tag on the remote
workspace-deleted-remote-tag = Deleted tag { $name } on { $there }

## Stashes.

workspace-stash-applied = Applied the stash; it stays in the list
workspace-stash-popped = Popped the stash
workspace-stash-message-title = Message of the stash
workspace-stash-reworded = Gave the stash another message
workspace-stash-branch-title = Pop the stash on a new branch
workspace-stash-branch = Create and pop
workspace-stash-branched = Popped the stash on { $name }
workspace-stash-drop-question = Drop this stash? Undo puts it back in the list.
workspace-stash-dropped = Dropped the stash

## A drop in the graph.

workspace-dirty-before-drop = Local changes are in the way of checking out { $branch }; commit or stash them first

## Rewriting history; branch is the branch checked out, or HEAD.

workspace-drop = Drop
# short is the commit's id, cut short.
workspace-drop-commit-question = Drop commit { $short } from { $branch }? The commits after it are made again without it; undo brings everything back.
# changes is how many files have changes a reset drops.
workspace-reset-question =
    { $changes ->
        [0] Reset { $branch } to this commit? Undo moves it back.
        [one] Reset { $branch } to this commit and drop the changes of { $changes } file? Undo brings both back.
       *[other] Reset { $branch } to this commit and drop the changes of { $changes } files? Undo brings both back.
    }
workspace-reset = Reset

## Several branches at once.

workspace-delete-branches-title = Delete { $count } branches?
# listed names the branches.
workspace-delete-branches-description = { $listed }. Branches with commits that are nowhere else are left alone. Undo brings the others back, one at a time.
workspace-deleted-branches = Deleted { $deleted } branches
# left names the branches git would not delete.
workspace-deleted-branches-some-left = Deleted { $deleted } branches; left as they are: { $left }

## A branch pulled, or deleted on its remote.

workspace-pulled-branch = Pulled { $name }
workspace-delete-on-remote-question = Delete { $branch } on { $remote }, for everyone? Undo pushes it back.
workspace-deleting-on-remote = Deleting on the remote
workspace-deleted-on-remote = Deleted { $branch } on { $remote }

## A merge, rebase, cherry-pick, revert or patch series that stopped;
## noun is what it is ("merge", "rebase").

workspace-nothing-in-progress = No merge, rebase or patch series is in progress here
workspace-nothing-to-skip = A { $noun } has nothing to skip: continue it or abort it
workspace-still-in-conflict =
    { $count ->
        [one] { $count } file is still in conflict: resolve it first
       *[other] { $count } files are still in conflict: resolve them first
    }
workspace-skipping = Skipping
workspace-continuing = Continuing
workspace-operation-finished = The { $noun } is finished
workspace-abort-question = Give the { $noun } up? Everything goes back to where it was before it began, and what has been resolved is lost.
workspace-abort = Abort
workspace-aborted = Aborted the { $noun }

## Undo and redo.

workspace-undoing = Undoing
workspace-redoing = Redoing
# label is what was undone, as the journal says it.
workspace-undone-stash-kept = Undone: { $label }. Your changes did not fit there; they are in the stash.
workspace-redone-stash-kept = Redone: { $label }. Your changes did not fit there; they are in the stash.

## Revealing a commit, finding a file.

# id is the commit's, cut short.
workspace-not-in-graph = { $id } is not in the current graph
workspace-find-file-title = Find a file and show its history

## What HEAD is on, for the status bar and the window's title.

workspace-head-detached = detached HEAD
workspace-head-unborn = unborn
# text is the rest of the status line.
workspace-status-read-only = { $text } · read-only (bare)

## The right pane, while the row of another worktree's uncommitted work
## is selected.

workspace-other-worktree-heading = Uncommitted work in another worktree
workspace-other-worktree-on = On { $branch }
workspace-other-worktree-detached = On no branch (detached)
workspace-other-worktree-gone = No longer a worktree of this repository
workspace-other-worktree-changed =
    { $count ->
        [one] { $count } file changed
       *[other] { $count } files changed
    }
workspace-other-worktree-new =
    { $count ->
        [one] { $count } file new
       *[other] { $count } files new
    }
workspace-other-worktree-removed =
    { $count ->
        [one] { $count } file removed
       *[other] { $count } files removed
    }
workspace-other-worktree-moved =
    { $count ->
        [one] { $count } file moved
       *[other] { $count } files moved
    }
workspace-other-worktree-explained = As it was when the worktree was last looked at: after its HEAD or index changed, on Refresh, and while the worktrees are listed open in the panel. Its changes are staged and committed where it is open.
workspace-other-worktree-open = Open this worktree

## The banners over the centre.

# id is the commit's, cut short.
workspace-detached-at = HEAD is detached at { $id } (on no branch)
workspace-detached = HEAD is detached.
# branch is the branch HEAD was on before.
workspace-detached-back = Back to { $branch }
workspace-detached-new-branch = New branch here…
workspace-skip-patch = Skip this patch
workspace-skip-commit = Skip this commit
workspace-continue = Continue
workspace-large-heading = A large repository
workspace-large-hide = Hide

## The status bar's words about the repository.

# changes is how many files are changed.
workspace-status-changed = { $changes } changed
# The same when the status was cut short: there are more.
workspace-status-changed-or-more = { $changes } or more changed

## The banner of a large repository: one line for each thing it is
## large in.

workspace-large-commits = { $commits } commits: the graph shows its first rows while the rest is read, and reading it takes a while after every change to the references
workspace-large-references = { $references } references: hiding remotes or tags in the panel keeps the graph and its labels manageable
# limit is how many paths a status holds at most.
workspace-large-status = more than { $limit } changed or untracked paths: the list of changes stops there (an ignore rule for what is not meant to be committed brings it back)

## A file in conflict that one side changed and the other deleted; name
## is the file's name.

workspace-deleted-question = { $name } was changed on one side and deleted on the other.
workspace-deleted-keep = Keep File
workspace-deleted-kept = Kept { $name }
workspace-deleted-delete = Delete File
workspace-deleted-deleted = Deleted { $name }
workspace-deleted-external = External Tool

## A file in the place of the graph, under two rows of buttons.

workspace-file-stage = Stage File
workspace-file-unstage = Unstage File
workspace-file-close = Close the file
workspace-file-close-unsaved = Close; the changes are not saved yet
workspace-file-edit = Edit
# Said in place of the Edit button while the file is edited.
workspace-file-editing = Editing
workspace-file-resolve = Resolve
# The two sides of the index the file is shown from.
workspace-file-unstaged = Unstaged
workspace-file-staged = Staged
workspace-file-view = File View
workspace-diff-view = Diff View
workspace-blame = Blame
workspace-history = History
workspace-file-reload = Read the file again
workspace-file-reload-tooltip = Read the file from the disk again; what was typed goes
workspace-file-save-tooltip = Save (Ctrl+S)

## What is done with a file picked from a list.

workspace-compare-select-one = Select one commit to compare the working tree with
workspace-submodule-not-cloned = { $path } is not cloned here, so its commits cannot be shown; update the submodule first
# commit is the id of the commit, cut short.
workspace-file-restored = Put { $path } back as { $commit } has it
workspace-sparse-only-running = Checking out only { $path }
workspace-sparse-only-done = Only { $path } and the files at the top are checked out
workspace-sparse-add-running = Checking out { $path }
workspace-sparse-add-done = { $path } is checked out too

## The forms the tab asks in: a name with suggestions under it.

workspace-form-no-match = Nothing of that name.
# more is how many suggestions are not shown.
workspace-form-more-matches = and { $more } more: type to narrow them down

## Leaving out the hooks that could refuse an operation.

# The switch in a dialog; hooks names them in a list, count is how many.
workspace-hooks-switch =
    { $count ->
        [one] Skip the { $hooks } hook, which could refuse it
       *[other] Skip the { $hooks } hooks, which could refuse it
    }
# What an operation is called while it runs without them; label is what
# it is called otherwise ("Merging").
workspace-hooks-running = { $label } without hooks
# done is what is said when it is done; hooks names the hooks left out.
workspace-hooks-skipped = { $done }, without running { $hooks }
workspace-push-hook-none = This repository has no pre-push hook, so there is none to skip.
workspace-push-hook-skipped = Pushes from this tab go without the pre-push hook until this is switched back or the tab is closed.
workspace-push-hook-again = Pushes run the pre-push hook again.

## A worktree for a pull request; named is the pull request's number as
## its provider writes it.

workspace-pull-head-fetching = Fetching the head of { $named }
workspace-pull-head-unreadable = The head of { $named } could not be read.

# Closing a file whose edited text is not saved; name is the file's name.
workspace-file-unsaved-question = { $name } was changed here and is not saved.

## A tag named in the graph.

workspace-create-tag = Create tag
workspace-created-tag = Created tag { $name }

## A file's past: its history beside it.

workspace-past-heading = File history:
workspace-past-close-tooltip = Close (Escape)
workspace-past-blame-tooltip = Say for every line which commit it is from

## A terminal that could not be started: said in its panel.

workspace-terminal-unclosed-quote = The shell in the preferences (Terminal) has a quote that is not closed.
workspace-terminal-no-shell-named = The engine's answer named no shell.
workspace-terminal-no-shell = No shell could be asked for.

## What the toolbar's Pull button can be made to do, in the order its caret lists them.

workspace-pull-button-fetch = Fetch all
workspace-pull-button-ff = Pull, fast-forward if possible
workspace-pull-button-ff-only = Pull, fast-forward only
workspace-pull-button-rebase = Pull with rebase
workspace-pull-button-merge = Pull, always a merge commit

## The four ways to pull, as menus and preferences name them.

workspace-pull-mode-ff-only = Fast-forward only
workspace-pull-mode-ff = Fast-forward if possible, else merge
workspace-pull-mode-rebase = Rebase onto the upstream
workspace-pull-mode-merge = Always a merge commit

## The banner of what Windows has to say of a repository.

workspace-platform-long-paths =
    { $files ->
        [one] One file here has a path longer than Windows takes (the longest is { $longest } characters, 260 are taken), and git's long paths are off: git cannot check them out or read them.
       *[other] { $files } files here have paths longer than Windows takes (the longest is { $longest } characters, 260 are taken), and git's long paths are off: git cannot check them out or read them.
    }
# The name of the WSL distribution the repository is in.
workspace-platform-wsl = This repository is inside WSL ({ $distribution }): its git runs there, and what is read of it from Windows goes through the share, which is slower.
workspace-platform-long-paths-on = Long paths are on for this repository
workspace-platform-turn-on-long-paths = Turn on long paths
workspace-platform-hide = Hide
