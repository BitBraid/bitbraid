### An operation that stopped part of the way.

# The one sentence: what is going on, how far it is (empty, or one of the steps below, which begin with a comma), and what it waits for.
stopped-sentence = { $what }{ $progress }: { $state }

## What is going on.

# A merge that is one step of something larger; $part_of says what, such as "Finishing feature search".
stopped-merging-part-of = { $part_of }, merging
stopped-merging = Merging
stopped-rebasing-branch = Rebasing { $branch }
stopped-rebasing = Rebasing
stopped-cherry-picking = Cherry-picking
stopped-reverting = Reverting
stopped-applying-patches = Applying patches

## How far it is: put right after what is going on.

stopped-patch-step = , patch { $step } of { $total }
stopped-commit-step = , commit { $step } of { $total }
# Only said when more than one is left.
stopped-commits-to-go = , { $left } commits to go

## What it waits for.

# $commit is the short id of the commit it stopped at.
stopped-amending-at = stopped at { $commit } for it to be amended; continue when it is as it should be
stopped-amending = stopped for a commit to be amended
stopped-patch-stuck = stopped at this patch; stage what it should change and continue, or skip it
stopped-nothing-staged = nothing is staged for this commit; stage what it should change and continue, or skip it
stopped-patch-staged = what this patch changes is staged; ready to continue
stopped-no-conflicts = nothing left in conflict

## The files in conflict, and how many hold a resolution remembered from last time.

stopped-files-in-conflict =
    { $count ->
        [one] { $count } file in conflict
       *[other] { $count } files in conflict
    }
# $files is the line above, for one file.
stopped-one-resolved-as-last-time = { $files }, resolved as last time: accept it or change it
stopped-all-resolved-as-last-time = { $files }, all resolved as last time: accept them or change them
stopped-some-resolved-as-last-time = { $files }, { $reused } of them resolved as last time
