### A commit's details.

## The strip over the details while the working tree has changes or conflicts.

details-uncommitted =
    { $count ->
        [one] { $count } file changed in the working tree
       *[other] { $count } files changed in the working tree
    }
details-view-changes = View Changes
details-conflicts =
    { $count ->
        [one] { $count } file in conflict
       *[other] { $count } files in conflict
    }
details-view-conflicts = View Conflicts

## The row with the commit's id.

details-nothing-selected = Select a commit to see its details.
# Before the short id of the commit.
details-commit-label = commit:
details-copy-id = Copy the whole id
details-loading = Loading…
# What the mark of the signature says, with the key it was made with.
# $sentence is what is said of the signature.
details-signature-with-key =
    { $sentence }
    Key: { $key }
details-allow-key = Allow key…
details-allow-key-tooltip = Add the key this commit was signed with to the allowed signers

## Who wrote the commit and where it comes from.

# Under the author's name. $time is a date and a time.
details-authored = authored { $time }
# Under the committer's name. $time is a date and a time.
details-committed = committed { $time }
# Before the short ids of the parent commits.
details-parent-label = parent:
details-co-authors-label = Co-authors:

## Several commits selected.

details-many-together = What these commits change together
# $old and $new are short commit ids.
details-many-differs = What differs from { $old } to { $new }
# $new is a short commit id.
details-many-differs-from-nothing = What differs from nothing to { $new }
details-many-not-compared = Two commits, or commits that follow one another, are compared here.
details-many-selected = { $count } commits selected

## The files.

details-count-modified = { $count } modified
details-count-added = { $count } added
details-count-deleted = { $count } deleted
details-count-renamed = { $count } renamed
details-against-comparing = Comparing with the working tree…
details-against-same = The working tree is as this commit has it
details-against-differs = What differs from this commit to the working tree
# The two ways the files are listed.
details-layout-path = Path
details-layout-tree = Tree
details-against = Compare with the working tree
details-against-off = Show what this commit changed
details-files-on-remote = The partial clone left what these files need on the remote. It is fetched when you ask for it.
details-fetch-files = Fetch the files
details-fetch-files-tooltip = Fetch what listing these files needs from the remote
# $error is what the engine said.
details-files-error = The files could not be listed: { $error }
details-collapse-all = Collapse All
details-expand-all = Expand All
details-terminal = Open a terminal here
# Beside a file that is not compared line by line.
details-file-binary = binary
details-file-large = large
