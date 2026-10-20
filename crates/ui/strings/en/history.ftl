### A file's history.

history-loading = Loading…
# How much the list holds, for the title of the view around it.
history-commits =
    { $count ->
        [one] 1 commit
       *[other] { $count } commits
    }
# The list stopped at the most it reads.
history-latest-commits = the latest { $count } commits

## A row. Its second line: when and by whom, then what the commit did to the file.

# $date is the day the commit was written.
history-byline = { $date } by { $author }
history-added = added
history-deleted = deleted
# $previous is the file's name before.
history-moved-from = moved from { $previous }
history-moved = moved
history-copied-from = copied from { $previous }
history-copied = copied
history-restore = Restore
history-restore-tooltip = Put the file in the worktree back to this commit
history-blame = Blame
history-blame-tooltip = Blame the file as of this commit
history-reveal = Show the commit in the graph

## Under the last row, when the list reaches where the file began.

history-came-from = It came here from { $previous }.
history-added-here = The file was added here.
history-no-earlier = No earlier commits
