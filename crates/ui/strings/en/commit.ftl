### The commit box: the message, amending, co-authors.

## The tabs over the form, and the message.

commit-tab-commit = Commit
commit-tab-stash = Stash
commit-summary = Summary
commit-description = Description (optional)
commit-amend-last = Amend the last commit

## The row under the message.

commit-options = Commit options
# The description's widest line is wider than git advises: which line, how wide, and the advice.
commit-line-too-wide = line { $line }: { $widest }/{ $limit }
commit-widest-line = widest line { $widest }/{ $limit }
# The messages of earlier commits, to take one back.
commit-history = History
commit-history-none = No commit was made here yet
# $typed is the start of what was in the box before a message was taken back.
commit-history-typed = What was typed: { $typed }
commit-add-trailer = Add trailer
# Adds a sign-off of the person committing; "Signed-off-by" is git's trailer.
commit-sign-off-me = Signed-off-by: me
commit-another-trailer = Another trailer…
commit-no-people = No people in recent history
commit-someone-else = Someone else…
commit-more = Other ways to commit
commit-stage-all-commit = Stage all and commit
commit-commit-push = Commit and push
commit-stage-all-commit-push = Stage all, commit and push
commit-stage-all-amend = Stage all and amend
commit-amend-push = Amend and push
commit-stage-all-amend-push = Stage all, amend and push

## The options of the next commit.

commit-push-after = Push afterwards
commit-sign = Sign
commit-skip-hooks = Skip hooks
# Under the button while hooks are left out. $hooks names them ("pre-commit and commit-msg"); $count is how many.
commit-skip-warning =
    { $count ->
        [one] The { $hooks } hook does not run and cannot refuse the commit.
       *[other] The { $hooks } hooks do not run and cannot refuse the commit.
    }
# The last two of a list of names; $first holds the others, with commas between.
commit-name-list = { $first } and { $last }

## The button.

commit-type-summary = Type a summary to commit
commit-stage-a-file = Stage a file to commit
commit-files =
    { $count ->
        [one] Commit 1 file
       *[other] Commit { $count } files
    }
commit-files-without-hooks =
    { $count ->
        [one] Commit 1 file without hooks
       *[other] Commit { $count } files without hooks
    }
commit-amend = Amend
commit-amend-without-hooks = Amend without hooks
commit-stash-stage-a-file = Stage a file to stash
commit-stash-files =
    { $count ->
        [one] Stash 1 file
       *[other] Stash { $count } files
    }

## While a merge, a rebase or a pick is stopped part way. $noun is git's word for it ("merge", "rebase").

commit-continue = Continue { $noun }
commit-abort = Abort { $noun }
# A merge concluded under the message typed.
commit-merge = Commit merge
commit-skip-this = Skip this commit
