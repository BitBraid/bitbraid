### Gitflow.

## The kinds of branch, as a word in a sentence ("Start feature login");
## the default message of a release's or hotfix's tag is this word with
## a capital and the name ("Release 1.2").

flow-kind-feature = feature
flow-kind-bugfix = bugfix
flow-kind-release = release
flow-kind-hotfix = hotfix
flow-kind-support = support

## Which operation or input was refused: the first words of the error.

flow-what-gitflow = gitflow
flow-what-start = start
flow-what-finish = finish
flow-what-main-branch = main branch
flow-what-develop-branch = develop branch
flow-what-prefix = prefix
flow-what-name = name
flow-what-branch-name = branch name
flow-what-tag-name = tag name
flow-what-tag-message = tag message
# What could not be read; followed by what git said.
flow-what-configuration = configuration

## Why a name was refused. The text in an argument is quoted.

flow-empty = must not be empty
flow-starts-with-dash = { $text } must not start with a dash
flow-control-characters = { $text } must not hold control characters
flow-too-long = is too long

## Why gitflow cannot be used here.

flow-not-set-up = gitflow is not set up in this repository
flow-same-branches = the main and the develop branch are both set to { $main }
flow-no-main = the main branch { $main } does not exist
flow-no-develop = the develop branch { $develop } does not exist
# One of the reasons above.
flow-set-up-first = { $reason }; set gitflow up first

## Why gitflow was not set up.

flow-no-commit-yet = there is no commit yet on any branch; make the first one, then set gitflow up
flow-no-main-or-master = there is neither a main nor a master branch; say which branch is the main one
flow-no-branch = there is no branch { $name }
flow-cannot-both-be = the main and the develop branch cannot both be { $main }
# The prefix, quoted, and the kind of branch (feature, release, …).
flow-prefix-unusable = { $prefix } cannot begin the name of a { $kind } branch
# Two kinds of branch, and the prefix, quoted.
flow-prefix-shared = { $kind } and { $other } branches cannot both begin with { $prefix }
# The prefix, quoted.
flow-tag-prefix-unusable = { $prefix } cannot begin the name of a tag
# The develop branch, and the remote branch of that name.
flow-develop-on-remote = { $develop } is on a remote ({ $found }) and not here; check it out first, then set gitflow up

## Why a branch was not started.

flow-no-worktree-checkout = this repository has no worktree to check a branch out in
flow-branch-exists = { $branch } exists already
flow-tag-exists = the tag { $tag } exists already; choose another name

## Why a branch was not finished.

# The first few paths, joined with commas, and how many more there are.
flow-listed-more = { $shown } and { $more } more
# A list of paths (see flow-listed-more).
flow-changes-in-way = local changes to { $paths } are in the way; commit or stash them first
flow-untracked-in-way = untracked files are in the way: { $paths }; move them or add them first
flow-support-not-finished = a support branch is not finished: it stays beside { $main } for as long as that line of releases is looked after
flow-no-worktree-merge = this repository has no worktree to merge in
flow-branch-is-base = { $branch } is one of the two branches everything is merged into
flow-tag-message-unusable = it is too long or has a NUL character in it
flow-no-branch-tagged = there is no branch { $branch }; the tag { $tag } exists, so it may be finished already
flow-ambiguous = the name { $branch } is ambiguous: something else is called that too
flow-tag-elsewhere = the tag { $tag } exists already and does not hold all of { $branch }; delete the tag or finish under another name
flow-unrelated = { $branch } and { $onto } have no history in common
# A branch, and the worktree it is checked out in.
flow-held = { $name } is checked out in the worktree at { $path }; free it there first
flow-head-not-on = HEAD is not on { $onto }; nothing was merged
flow-feature-no-tag = a feature leaves no tag
# The finished branch, and the branch it is not merged into.
flow-not-merged = { $branch } is not merged into { $name }; it was kept
