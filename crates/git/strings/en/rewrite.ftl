### Rewriting commits.

## Which input or operation was refused: the first words of the error.
## The operations' names also stand in the middle of a sentence ("the
## merge could not make its commit").

rewrite-what-merge = merge
rewrite-what-rebase = rebase
rewrite-what-reset = reset
rewrite-what-cherry-pick = cherry-pick
rewrite-what-revert = revert
rewrite-what-undo = undo
# One change to a commit of the current branch: reword, drop or squash.
rewrite-what-rewrite = rewrite
rewrite-what-merge-target = merge target
rewrite-what-rebase-target = rebase target
# The commit after which a rebase takes the commits of the branch.
rewrite-what-range-start = start of the range
# What the conflicts of a merge are foretold for.
rewrite-what-target = target
rewrite-what-current-commit = current commit
# The commit a merge whose conflicts are foretold goes into, when it is not HEAD.
rewrite-what-merged-into = what it is merged into
rewrite-what-reset-target = reset target

## What git is in the middle of, in "… is in progress".

rewrite-in-progress-merge = a merge
rewrite-in-progress-rebase = a rebase
rewrite-in-progress-cherry-pick = a cherry-pick
rewrite-in-progress-revert = a revert
# Git's sequencer is at work, and which of the two it is cannot be told.
rewrite-in-progress-sequencer = a cherry-pick or revert
rewrite-in-progress-bisect = a bisect

## Why a merge, rebase, reset, cherry-pick or revert stopped or did not begin.

# A merge stopped before its commit because that was asked for.
rewrite-stopped-before-commit = stopped before the commit, as asked: continue to make the merge commit, or abort
# The revision given, quoted.
rewrite-cannot-be-used = { $rev } cannot be used
# The revision given, quoted.
rewrite-not-a-commit = { $rev } does not name a commit
# What git is in the middle of (see above).
rewrite-other-in-progress = { $other } is in progress; finish or abort it first
rewrite-unresolved-conflicts = there are unresolved conflicts; resolve them first
rewrite-staged-changes = there are staged changes; commit or unstage them first
# What the operation's own refusal said; what it had staged was taken back.
rewrite-taken-back = { $told }; nothing was changed
# The files, joined by commas.
rewrite-would-conflict = it would conflict in { $files }; nothing was changed
rewrite-untracked-in-the-way = the untracked directory { $directory } is in the way
rewrite-head-not-on-branch = HEAD is not on a branch
# The revision the range starts after, as given.
rewrite-not-in-current-history = { $rev } is not in the history of the current branch
rewrite-no-commits-given = no commits were given
rewrite-head-moved = HEAD is not where this action left it; nothing was changed
rewrite-nothing-to-go-back-to = there is no commit to go back to
# The files, joined by commas.
rewrite-changes-in-the-way = local changes to { $paths } are in the way; nothing was changed

## The steps of foretelling a merge's conflicts; each is followed by what
## failed.

rewrite-making-trial-directory = making a directory for a trial merge

## Why one change to a commit (reword, drop, squash) was refused. A commit
## is named by the first seven digits of its id.

rewrite-merge-not-squashable = { $commit } is a merge commit and cannot be squashed into the commit before it: that would undo the merge and keep its changes
rewrite-first-not-squashable = { $commit } is the first commit of the history and has nothing before it to be squashed into
rewrite-commits-apart = the commits do not follow one another in the history of the current branch, or one is named twice
rewrite-path-not-for-sequencer = the path of the repository cannot be handed to git's sequencer
# What git's list of the rebase names, quoted.
rewrite-list-names-stranger = git's list names { $short }, which is not one commit of the range
rewrite-list-merge-without-commit = git's list has a merge without its commit
rewrite-list-merge-of-nothing = git's list has a merge of nothing
rewrite-list-lacks-commit = git's list does not have { $commit }, which it should replay
# The commit squashed into, and the next one squashed into it.
rewrite-line-starts-between = another line of the history starts between { $into } and { $commit }: squashed, they would no longer be what it started from
rewrite-merge-of-more-than-two = the merge { $merge } after it joins more than two lines, and whether git would make it again as it is cannot be told
rewrite-merge-has-own-changes = the merge { $merge } after it holds changes of its own, beyond what merging gives; git would make it anew without them
rewrite-merge-resolved-by-hand = the merge { $merge } after it was resolved by hand; git would make it anew and ask for that again
rewrite-merge-cannot-be-told = whether git would make the merge { $merge } after it again as it is cannot be told (that takes git 2.38)
rewrite-not-in-branch-history = { $commit } is not in the history of { $branch }
rewrite-message-unusable = the new message is empty or cannot be used
rewrite-range-unreadable = a commit of the range could not be read
# The most commits there can be, the oldest commit changed, and the branch.
rewrite-too-many-commits = there are more than { $most } commits from { $commit } to { $branch }
rewrite-only-commit = { $commit } is the only commit of { $branch }
# Why the system could not write the files of the plan.
rewrite-plan-not-written = the plan could not be written: { $err }
rewrite-nothing-to-replay = git had nothing to replay
