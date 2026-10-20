### Opening and reading a repository.

## The commands: what the palette and the command line call each, and what it does.

repo-open = Open repository
repo-open-description = Open the repository containing `path` and make it the session's repository
repo-summary = Repository summary
repo-summary-description = Where the repository is and what HEAD points at
repo-refs = List references
repo-refs-description = HEAD, branches, remotes, tags, stashes, worktrees and submodules
repo-log = List commits
repo-log-description = Commits in graph order, a commit before its parents; params: tips (all|head|{"{"}ids:[..]{"}"}), limit, first_parent, order (date: newest first, the default | topo: each line of history together)
repo-graph = Commit graph
repo-graph-description = Commits (ids, parents, times) with lane layout rows; same params as repo.log, plus details: bool, ascii: bool, pinned: branches or revisions whose lines keep the leftmost lanes, in that order, and stashes: bool (a row for each stash, on the commit it was made on)
repo-commits = Commit details
repo-commits-description = Author, committer and summary for the given commit ids; params: ids
repo-status = Working tree status
repo-status-description = Staged and unstaged changes per path, as git status reports them; params: limit (stop once that many changed paths were found; the answer is then {"{"}entries, more{"}"} instead of the list)
repo-diff = Diff
repo-diff-description = File diffs with hunks between two sides: every file that differs, also between a commit and the worktree or the index; params: old, new (revision|index|worktree|empty), paths, context, detect_renames, whitespace (exact|ignore_trailing|ignore_all|ignore_cr_at_eol), max_bytes (a file with a side larger than this is listed with its sizes and not compared); each file says how its lines end on each side and whether that is all that changed (line_endings)
repo-blame = Blame
repo-blame-description = Which commit introduced each line of a file; params: path, rev, ignore_revs (pass over the revisions listed in the file blame.ignoreRevsFile names, or in .git-blame-ignore-revs; when absent, only where git's configuration names a file, as git blame does)
repo-blame-previous = Blame before a commit
repo-blame-previous-description = Where the lines a commit gave a file were before it, as git's blame says `previous`: the commit's first parent and the file's name there, which is the name it had before the commit renamed it; null when the commit added the file or has no parent; params: path (the file's name in the commit), rev (the commit)
repo-blame-ignores = Revisions blame passes over
repo-blame-ignores-description = The file of revisions for blame to pass over, when there is one: where it is, whether git's configuration names it (blame.ignoreRevsFile) or it is the conventional .git-blame-ignore-revs, whether it is there, and how many revisions it lists
repo-history = File history
repo-history-description = Commits that changed a path, following renames; params: path, follow, tips, limit, max_entries

## Refusals: what follows "invalid parameter "<name>": ".

# The largest number of lanes that can be pinned.
repo-too-many-pinned = at most { $max } lanes are pinned
repo-not-a-commit = must name a commit
# The file is the conventional name of the file of revisions to pass over, .git-blame-ignore-revs.
repo-no-ignore-revs = there are no revisions to pass over: blame.ignoreRevsFile is not set and there is no { $file } here
