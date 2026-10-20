### Interactive rebase.

## Which operation or input was refused: the first words of the error.

interactive-what = interactive rebase
interactive-what-first-commit = first commit
interactive-what-range = range
interactive-what-commits = commits
interactive-what-plan = plan
interactive-what-new-base = new base

## Why the commits could not be planned. A commit is its short id.

interactive-head-detached = HEAD is not on a branch
interactive-not-in-history = { $commit } is not in the history of { $branch }
interactive-first-of-history = { $commit } is the first commit of the history, and there is nothing before it to replay the others on; begin at a later commit
interactive-parent-unreadable = its parent could not be read
interactive-commit-unreadable = a commit could not be read
# The most commits there can be, the first commit, and the branch.
interactive-too-many = there are more than { $most } commits from { $commit } to { $branch }; begin at a later commit
interactive-commits-unreadable = its commits could not be read
interactive-merge-in-way = { $commit } is a merge commit, and a list of commits cannot say how to redo a merge; begin at a commit after it
interactive-nothing-to-rebase = the current branch has no commit that { $onto } does not have: there is nothing to rebase onto it
interactive-too-far = there are { $count } commits between { $onto } and the current branch, too many to plan
interactive-first-unreadable = its first commit could not be read

## Why commits could not be moved.

interactive-not-in-range = { $commit } is not in the history being moved in
interactive-named-twice = { $commit } is named twice
interactive-none-named = name at least one commit to move
interactive-not-consecutive = they do not follow one another
interactive-nothing-after = nothing comes after them on the branch to move past
interactive-nothing-before = nothing comes before them in what is moved to move past

## Why a plan was refused.

# How many steps the plan has, and how many commits there are.
interactive-step-count = it has { $steps } steps for { $commits } commits
interactive-step-twice = { $commit } is in it twice, or is not one of the commits
interactive-message-unusable = the message for { $commit } cannot be used
interactive-reword-no-message = { $commit } is to be reworded and has no message
interactive-squash-first = { $commit } is the first commit that is kept and has nothing before it to be squashed into
interactive-path-unusable = the path of the repository cannot be handed to git's sequencer
# Why the files could not be written.
interactive-plan-unwritable = the plan could not be written: { $err }

## Where a rebase stopped.

# The commit's short id, or the words below when it is not known.
interactive-stopped-to-edit = stopped at { $at } for it to be amended; continue when it is as it should be
interactive-a-commit = a commit
