### Blame: who wrote each line.

blame-loading = Loading…
# A blame of the file before a commit that added it. $commit is the commit's short id.
blame-added-here = { $commit } added { $path }: there is nothing before it to blame

## The line over the lines.

# The switch that has blame pass over the revisions a file lists.
blame-pass-over =
    { $count ->
        [one] Pass over 1 listed revision
       *[other] Pass over { $count } listed revisions
    }
# $file is the file that lists the revisions.
blame-ignore-revs-tooltip = The revisions listed in { $file }
# Back to the commit the file was asked to be blamed at; $commit is its short id.
blame-back-to = Back to { $commit }
blame-back-to-head = Back to HEAD

## A block of lines by one commit, and the card of its commit.

blame-before-this-commit = Before this commit
blame-show-in-graph = Show in graph
blame-what-it-changed = What it changed here
blame-before-it = Blame before it
blame-copy-id = Copy id
