### Gitflow.

## The menu under the heading of the gitflow section.

flow-start-feature = Start a feature…
flow-start-bugfix = Start a bugfix…
flow-start-release = Start a release…
flow-start-hotfix = Start a hotfix…
flow-start-support = Start a support branch…
# An entry that cannot be used before gitflow is set up; `label` is one of the five above, without its ellipsis.
flow-set-up-first = { $label } (set gitflow up first)
flow-branches-and-prefixes = Branches and prefixes…
flow-set-up = Set gitflow up…

## The menu of a branch of gitflow's; `kind` is gitflow's name of its kind (feature, bugfix, release, hotfix), `name` the branch's name without the prefix.

flow-support-stays = A support branch is not finished: it stays
flow-finish-branch = Finish { $kind } { $name }…
flow-finish-and-keep = Finish and keep the branch…
flow-change-description = Change the description…
flow-describe = Describe…
flow-delete-merged = Delete merged branches…

## What finishing a branch does, asked in the bar before it is done; `main` and `develop` are the names of the two branches.

flow-finish-into-develop-kept = It is merged into { $develop }, then kept.
flow-finish-into-develop-deleted = It is merged into { $develop }, then deleted.
flow-finish-released-kept = It is merged into { $main }, tagged, merged into { $develop }, then kept.
flow-finish-released-deleted = It is merged into { $main }, tagged, merged into { $develop }, then deleted.

## Starting a branch, said while its name is typed.

# Nothing is typed yet; `prefix` is what the kind's branches begin with, `base` the branch it is made from.
flow-start-any-name = A branch that begins with { $prefix } is made from { $base } and checked out.
flow-branch-exists = There is a branch { $branch } already.
flow-start-explained = { $branch } is made from { $base } and checked out.

## The settings of gitflow, looked at while they are typed.

flow-same-branches = The main and the develop branch cannot both be { $branch }.
flow-no-such-branch = There is no branch { $branch } here.
# `prefix` is in quotes; `kind` is gitflow's name of a kind of branch.
flow-prefix-cannot-begin = { $prefix } cannot begin the name of a { $kind } branch.
# Two kinds were given the same prefix; `first` is the first kind with a capital, `other` the second.
flow-prefix-shared = { $first } and { $other } branches cannot both begin with { $prefix }.
# `prefix` is in quotes.
flow-tag-prefix-cannot-begin = { $prefix } cannot begin the name of a tag.
flow-develop-made = The branch { $develop } is made from { $main }.
# Branches of a kind whose prefix is changed; `old` is the prefix they begin with.
flow-branches-left =
    { $count ->
        [one] { $count } branch that begins with { $old } is then no { $kind } branch any more.
       *[other] { $count } branches that begin with { $old } are then no { $kind } branches any more.
    }
