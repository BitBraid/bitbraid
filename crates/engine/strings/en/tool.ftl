### Diff and merge tools.

## The commands: what the palette and the command line call each, and what it does.

tool-list = Configured tools
tool-list-description = The diff and merge tools git is configured with, by name
tool-diff = Open in diff tool
tool-diff-description = Show a file's changes in the configured diff tool, or with a command that uses $LOCAL and $REMOTE; params: path, staged (bool), commit (shows what that commit changed), to with from (what differs between two commits; without from, against nothing), command
tool-merge = Resolve in merge tool
tool-merge-description = Resolve a conflicted file in the configured merge tool, or with a command that uses $LOCAL, $REMOTE, $BASE and $MERGED, and wait for it; params: path, command
tool-known = Diff and merge tools found
tool-known-description = The diff and merge tools known by name that are on this machine, each with a command for diffs and one for merges written with $LOCAL, $REMOTE, $BASE and $MERGED, ready to be used as a tool command; params: all (bool: also the ones that are not here, as known)

## What is wrong with the parameters.

tool-range-alone = two commits are compared by themselves: without commit or staged
tool-from-without-to = it is the older of two commits: say the newer one as `to`
