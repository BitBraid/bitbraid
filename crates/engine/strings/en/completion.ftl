### What the terminal's completion offers for git's commands.

## The commands: what the palette and the command line call each, and what it does.

completion-terminal-completion-words = Names for completing in the terminal
completion-terminal-completion-words-description = The repository's names a git command typed in the terminal is completed with: branches, remotes with their branches, tags, stashes, changed paths and tracked ones; params: changes (whether to read what changed, true when absent), most_paths (how many tracked paths at most, 50000 when absent)
completion-terminal-complete = Complete a git command
completion-terminal-complete-description = The word being typed in a git command line and what may take its place, best first; params: line (what is typed before the cursor), folder (where the shell is, from the top of the worktree; the top when absent), shell (posix, fish, nushell, csh, powershell, cmd; posix when absent)

## A parameter refused: what is said after its name.

# Most is how many tracked paths may be asked for.
completion-most-paths-too-many = at most { $most } paths are listed
# Longest is how many characters a line may have.
completion-line-too-long = at most { $longest } characters are completed
completion-folder-outside = is not a folder of the worktree
# The names are those the parameter takes, as they are typed.
completion-shell-unknown = is none of posix, fish, nushell, csh, powershell, cmd

## What the list says of a path, by how it differs from the last commit or the index.

completion-change-added = Added
completion-change-modified = Modified
completion-change-deleted = Deleted
completion-change-renamed = Renamed
completion-change-copied = Copied
completion-change-type-changed = Type changed
completion-change-untracked = Untracked
completion-change-intent-to-add = To be added
completion-change-conflicted = Conflicted
# A change that is staged; change is one of the words above.
completion-change-staged = { $change }, staged

## What the list says of the repository's other names.

# The branch checked out.
completion-detail-checked-out = Checked out
# Any other local branch.
completion-detail-local = Local
completion-detail-tag = Tag
# A remote whose address is not known.
completion-detail-remote = Remote
completion-detail-folder = Folder
# A path git tracks.
completion-detail-tracked = Tracked

## git's commands, as the list offers them before anything is typed. Each says what the command does in a few words, 48 characters at most, as do the options and words below.

completion-command-status = Show what changed and what is staged
completion-command-add = Stage changes for the next commit
completion-command-commit = Record what is staged as a commit
completion-command-push = Send commits to a remote
completion-command-pull = Fetch and bring in a remote branch
completion-command-fetch = Download what a remote has that is new
completion-command-checkout = Switch branches, or put files back
completion-command-switch = Switch to another branch
completion-command-branch = List, make, rename or delete branches
completion-command-merge = Join another line of history into this one
completion-command-rebase = Replay commits on another base
completion-command-log = Show the history
completion-command-diff = Show changes between commits and files
completion-command-show = Show a commit
completion-command-stash = Put changes aside and bring them back
completion-command-reset = Move the branch, and unstage
completion-command-restore = Put files back as they were
completion-command-tag = List, make or delete tags
completion-command-remote = Add, change or remove remotes
completion-command-cherry-pick = Apply the changes of other commits
completion-command-revert = Make commits that undo others
completion-command-clone = Copy a repository
completion-command-init = Make a new repository
completion-command-config = Read and change settings
completion-command-help = Show a command's manual
completion-command-worktree = Working trees beside this one
completion-command-submodule = Repositories inside this one
completion-command-bisect = Find the commit that brought a change
completion-command-blame = Show who last changed each line
completion-command-clean = Delete files git does not track
completion-command-rm = Delete files and stop tracking them
completion-command-mv = Move or rename a file
completion-command-grep = Search the tracked files
completion-command-describe = Name a commit after the nearest tag
completion-command-reflog = Show where a branch pointed before
completion-command-shortlog = Sum up the history by author
completion-command-sparse-checkout = Check out only some folders
completion-command-maintenance = Keep the repository quick
completion-command-gc = Tidy and pack the repository
completion-command-notes = Notes on commits
completion-command-archive = Make an archive of a commit's files
completion-command-format-patch = Write commits out as patches
completion-command-am = Apply patches from mail
completion-command-apply = Apply a patch to the files
completion-command-range-diff = Compare two series of commits

## The options git itself takes before a command.

completion-git-option-version = Print which git this is
completion-git-option-help = Show git's manual
completion-git-option-folder = Run as if started in that folder
completion-git-option-setting = Set a setting for this run only
completion-git-option-no-pager = Print straight to the terminal
completion-git-option-paginate = Show the output in a pager

## Said of options of several commands.

# git commit -m, git commit --message, git merge -m
completion-option-message = The message, given here
# git pull --ff-only, git merge --ff-only
completion-option-ff-only = Only move the branch forward
# git pull --autostash, git rebase --autostash
completion-option-autostash = Put changes aside first, and back after
# git checkout -b, git switch -c, git switch --create
completion-option-new-branch = Make a new branch and switch to it
# git checkout -B, git switch -C
completion-option-reset-branch = Make or reset a branch and switch to it
# git checkout --detach, git worktree add --detach
completion-option-detach = Check out the commit itself, on no branch
# git checkout --track, git switch --track
completion-option-track = Follow the remote branch
# git checkout --force, git switch --discard-changes
completion-option-discard-changes = Throw away local changes
# git checkout --orphan, git switch --orphan
completion-option-orphan = Start a branch without history
# git checkout --patch, git restore --patch
completion-option-patch = Choose the parts to put back
# git checkout --ours, git restore --ours
completion-option-ours = Take our side of a conflict
# git checkout --theirs, git restore --theirs
completion-option-theirs = Take their side of a conflict
# git merge --continue, git rebase --continue, git cherry-pick --continue, git revert --continue, git am --continue
completion-option-continue = Go on once the conflicts are resolved
# git merge --no-edit, git revert --no-edit
completion-option-no-edit = Keep the message git makes
# git rebase --skip, git cherry-pick --skip
completion-option-skip = Leave out the commit that stopped it
# git log --stat, git diff --stat, git show --stat
completion-option-stat = Show which files changed and how much
# git diff --name-only, git show --name-only, git grep --files-with-matches
completion-option-name-only = Only the names of the files
# git diff --ignore-all-space, git diff -w, git blame -w
completion-option-ignore-all-space = Leave out changes of spaces
# git cherry-pick --abort, git revert --abort, git am --abort
completion-option-abort = Give up and go back
# git cherry-pick --mainline, git revert --mainline
completion-option-mainline = Which parent of a merge counts

## git add: its options.

completion-add-option-all = Stage every change, new and deleted files too
completion-add-option-update = Stage changes to tracked files only
completion-add-option-patch = Choose the parts to stage
completion-add-option-interactive = Stage from a menu
completion-add-option-force = Stage files the ignore rules leave out
completion-add-option-dry-run = Only say what would be staged
completion-add-option-intent-to-add = Note new files, to be staged later

## git commit: its options.

completion-commit-option-amend = Change the last commit instead
completion-commit-option-all = Stage changes to tracked files first
completion-commit-option-no-edit = Keep the message as it is
completion-commit-option-signoff = End the message with a sign-off line
completion-commit-option-sign = Sign the commit
completion-commit-option-no-verify = Do not run the commit hooks
completion-commit-option-fixup = Mark it as a fix to a commit
completion-commit-option-allow-empty = Commit even with nothing staged
completion-commit-option-verbose = Show the diff where the message is written
completion-commit-option-file = Take the message from a file
completion-commit-option-author = Name someone else as the author

## git push: its options.

completion-push-option-set-upstream = Remember the remote branch as the upstream
completion-push-option-force-with-lease = Overwrite, unless someone else pushed
completion-push-option-force = Overwrite the remote branch
completion-push-option-tags = Send every tag too
completion-push-option-follow-tags = Send the annotated tags of what is sent
completion-push-option-all = Send every branch
completion-push-option-delete = Delete the remote branch
completion-push-option-dry-run = Only say what would be sent
completion-push-option-no-verify = Do not run the pre-push hook

## git pull: its options.

completion-pull-option-rebase = Replay local commits on top instead of merging
completion-pull-option-no-rebase = Merge
completion-pull-option-tags = Bring every tag too
completion-pull-option-verbose = Say more about what happens

## git fetch: its options.

completion-fetch-option-all = Fetch from every remote
completion-fetch-option-prune = Delete branches the remote no longer has
completion-fetch-option-tags = Fetch every tag
completion-fetch-option-depth = Fetch only so many commits
completion-fetch-option-unshallow = Fetch the whole history

## git switch: its options.

completion-switch-option-detach = Switch to the commit itself, on no branch

## git branch: its options.

completion-branch-option-all = List the remote branches too
completion-branch-option-remotes = List the remote branches
completion-branch-option-delete = Delete a branch that is merged
completion-branch-option-delete-force = Delete a branch, merged or not
completion-branch-option-move = Rename a branch
completion-branch-option-move-force = Rename a branch, over another one
completion-branch-option-copy = Copy a branch
completion-branch-option-set-upstream-to = Set the branch's upstream
completion-branch-option-unset-upstream = Forget the branch's upstream
completion-branch-option-verbose = Show each one's last commit
completion-branch-option-merged = Only those merged into a commit
completion-branch-option-no-merged = Only those not merged into a commit
completion-branch-option-contains = Only those that hold a commit
completion-branch-option-show-current = Print the branch checked out

## git merge: its options.

completion-merge-option-no-ff = Make a merge commit even when not needed
completion-merge-option-squash = Bring the changes in without a merge commit
completion-merge-option-abort = Give up the merge in progress
completion-merge-option-no-commit = Stop before committing
completion-merge-option-strategy = Which way to merge

## git rebase: its options.

completion-rebase-option-interactive = Choose what happens to each commit
completion-rebase-option-onto = Replay onto another base
completion-rebase-option-abort = Give up and go back to where it started
completion-rebase-option-autosquash = Move fixups next to what they fix
completion-rebase-option-update-refs = Move the branches in between too
completion-rebase-option-rebase-merges = Keep the merges

## git log: its options.

completion-log-option-oneline = One line per commit
completion-log-option-graph = Draw the lines of history
completion-log-option-all = Every branch, not only this one
completion-log-option-patch = Show each commit's changes
completion-log-option-author = Only commits by someone
completion-log-option-since = Only commits after a date
completion-log-option-until = Only commits before a date
completion-log-option-grep = Only commits whose message matches
completion-log-option-max-count = Only so many commits
completion-log-option-follow = Follow a file across renames
completion-log-option-first-parent = Follow only the first parent of merges
completion-log-option-reverse = Oldest first
completion-log-option-decorate = Show branch and tag names

## git diff: its options.

completion-diff-option-staged = What is staged, against the last commit
completion-diff-option-name-status = The names of the files and how they changed
completion-diff-option-word-diff = Show changed words, not lines
completion-diff-option-check = Look for whitespace errors
completion-diff-option-no-index = Compare two files outside the repository

## git show: its options.

completion-show-option-oneline = The commit on one line
completion-show-option-no-patch = Leave out the changes

## git stash: its words and their options.

completion-stash-push-option-include-untracked = Put aside new files too
completion-stash-push-option-all = Put aside new and ignored files too
completion-stash-push-option-keep-index = Leave what is staged as it is
completion-stash-push-option-staged = Put aside only what is staged
completion-stash-push-option-message = Say what it holds
completion-stash-push-option-patch = Choose the parts to put aside
completion-stash-apply-option-index = Stage again what was staged
completion-stash-push = Put the changes aside
completion-stash-pop = Bring back one and drop it
completion-stash-apply = Bring back one and keep it
completion-stash-list = List what was put aside
completion-stash-show = Show what one holds
completion-stash-show-option-patch = Show its changes
completion-stash-drop = Delete one
completion-stash-branch = Make a branch from one
completion-stash-clear = Delete them all

## git reset: its options.

completion-reset-option-soft = Keep the index and the files
completion-reset-option-mixed = Keep the files, reset the index
completion-reset-option-hard = Throw away changes to tracked files
completion-reset-option-keep = Keep local changes where it can
completion-reset-option-patch = Choose the parts to unstage

## git restore: its options.

completion-restore-option-source = Take them from another commit
completion-restore-option-staged = Put back the index, not the files
completion-restore-option-worktree = Put back the files

## git tag: its options.

completion-tag-option-annotate = Make an annotated tag
completion-tag-option-message = Its message, given here
completion-tag-option-delete = Delete tags
completion-tag-option-sign = Sign the tag
completion-tag-option-list = List the tags
completion-tag-option-force = Replace a tag of that name
completion-tag-option-verify = Check a tag's signature

## git remote: its words and their options.

completion-remote-add = Add a remote
completion-remote-remove = Remove a remote
completion-remote-rename = Rename a remote
completion-remote-set-url = Change a remote's address
completion-remote-get-url = Print a remote's address
completion-remote-show = Show what a remote holds
completion-remote-prune = Delete branches a remote no longer has
completion-remote-update = Fetch from remotes
completion-remote-option-verbose = Show their addresses

## git cherry-pick: its options.

completion-cherry-pick-option-no-commit = Apply the changes, do not commit
completion-cherry-pick-option-note-origin = Note where the commit came from
completion-cherry-pick-option-edit = Change the message

## git revert: its options.

completion-revert-option-no-commit = Undo the changes, do not commit

## git clone: its options.

completion-clone-option-depth = Copy only so many commits
completion-clone-option-branch = Check out this branch
completion-clone-option-recurse-submodules = Copy the submodules too
completion-clone-option-bare = Copy without a working tree
completion-clone-option-filter = Leave out some objects until needed
completion-clone-option-single-branch = Copy one branch only

## git init: its options.

completion-init-option-bare = Make it without a working tree
completion-init-option-initial-branch = Name the first branch

## git config: its options.

completion-config-option-global = Your settings for every repository
completion-config-option-local = This repository's settings
completion-config-option-list = List the settings
completion-config-option-show-origin = Say which file each setting is in
completion-config-option-edit = Open the settings in an editor
completion-config-option-unset = Remove a setting

## git worktree: its words and their options.

completion-worktree-add = Make a working tree beside this one
completion-worktree-add-option-new-branch = Make a new branch for it
completion-worktree-list = List the working trees
completion-worktree-remove = Remove a working tree
completion-worktree-remove-option-force = Even with changes in it
completion-worktree-move = Move a working tree
completion-worktree-lock = Keep a working tree from removal
completion-worktree-unlock = Allow its removal again
completion-worktree-prune = Forget working trees that are gone
completion-worktree-repair = Mend links to moved working trees

## git submodule: its words and their options.

completion-submodule-status = Show the state of each
completion-submodule-update = Check out what the superproject records
completion-submodule-update-option-init = Set up the ones not set up yet
completion-submodule-update-option-recursive = Inside nested ones too
completion-submodule-update-option-remote = Take the newest of their branch
completion-submodule-init = Set them up from .gitmodules
completion-submodule-add = Add a repository inside this one
completion-submodule-deinit = Take one out of the working tree
completion-submodule-sync = Copy their addresses into settings
completion-submodule-foreach = Run a command in each
completion-submodule-summary = Sum up their changes

## git bisect: its words and their options.

completion-bisect-start = Begin a search
completion-bisect-good = Mark a commit as before the change
completion-bisect-bad = Mark a commit as after the change
completion-bisect-skip = Leave a commit out of the search
completion-bisect-reset = End the search
completion-bisect-log = Show what was marked
completion-bisect-run = Let a script decide each commit

## git sparse-checkout: its words and their options.

completion-sparse-checkout-set = Check out only these folders
completion-sparse-checkout-set-option-cone = Whole folders
completion-sparse-checkout-set-option-no-cone = Patterns
completion-sparse-checkout-add = Check out these folders too
completion-sparse-checkout-list = List what is checked out
completion-sparse-checkout-disable = Check out everything again
completion-sparse-checkout-reapply = Apply the patterns again

## git maintenance: its words and their options.

completion-maintenance-run = Run the tasks now
completion-maintenance-run-option-task = Only this task
completion-maintenance-start = Run them on a schedule
completion-maintenance-stop = Stop the schedule

## git notes: its words and their options.

completion-notes-add = Add a note to a commit
completion-notes-show = Show a commit's note
completion-notes-list = List the notes
completion-notes-edit = Change a commit's note
completion-notes-remove = Remove a commit's note

## git clean: its options.

completion-clean-option-dry-run = Only say what would be deleted
completion-clean-option-force = Delete them
completion-clean-option-folders = Folders too
completion-clean-option-ignored = Ignored files too
completion-clean-option-interactive = Choose from a menu

## git rm: its options.

completion-rm-option-cached = Only from the index; the file stays
completion-rm-option-recursive = Folders and what is in them
completion-rm-option-force = Even with changes
completion-rm-option-dry-run = Only say what would be removed

## git blame: its options.

completion-blame-option-lines = Only these lines
completion-blame-option-show-email = Show e-mail addresses, not names

## git grep: its options.

completion-grep-option-ignore-case = Upper and lower case alike
completion-grep-option-line-number = Number the lines found
completion-grep-option-word-regexp = Whole words only
completion-grep-option-cached = Search what is staged

## git describe: its options.

completion-describe-option-tags = Lightweight tags count too
completion-describe-option-all = Any reference counts
completion-describe-option-always = The commit's id when no tag is near
completion-describe-option-dirty = Say when the working tree has changes

## git shortlog: its options.

completion-shortlog-option-summary = Only the counts
completion-shortlog-option-numbered = Most commits first
completion-shortlog-option-email = With e-mail addresses

## git archive: its options.

completion-archive-option-format = tar, zip or another
completion-archive-option-output = Write it to this file
completion-archive-option-prefix = Put this in front of every path

## git format-patch: its options.

completion-format-patch-option-stdout = Print them, do not write files
completion-format-patch-option-cover-letter = Begin with a letter about them
completion-format-patch-option-output-directory = Write them into this folder

## git am: its options.

completion-am-option-skip = Leave out the patch that stopped it
completion-am-option-3way = Merge when a patch does not apply

## git apply: its options.

completion-apply-option-check = Only say whether it applies
completion-apply-option-stat = Show what it changes
completion-apply-option-cached = Apply it to the index
completion-apply-option-reverse = Take it back out
completion-apply-option-3way = Merge when it does not apply

## git gc: its options.

completion-gc-option-aggressive = Pack harder, taking longer
completion-gc-option-auto = Only when it is needed

## git status: its options.

completion-status-option-short = One line per file
completion-status-option-branch = Show the branch too
completion-status-option-ignored = Show ignored files too
completion-status-option-untracked-files = How to show new files

## git help: its options.

completion-help-option-all = List every command
completion-help-option-guides = List the guides

## git mv: its options.

completion-mv-option-force = Even over another file
