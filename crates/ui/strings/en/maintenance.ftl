### Maintenance of a repository.

## Counts that go into the sentences below.

maintenance-objects-count =
    { $count ->
        [one] { $count } object
       *[other] { $count } objects
    }
maintenance-packs-count =
    { $count ->
        [one] { $count } pack
       *[other] { $count } packs
    }

## The size of the repository.

maintenance-size-group = Size
maintenance-objects = Objects
maintenance-measure = Measure
maintenance-measure-again = Measure again
maintenance-stop = Stop
maintenance-measuring = Measuring: every object is looked at, and history is walked for where the largest came in.
maintenance-not-measured = Not measured yet.
maintenance-measure-failed = The repository could not be measured: { $error }
# `objects` is how many objects there are and `packs` how many packs (the counts above); the sizes are written out with their unit.
maintenance-objects-said = { $objects } take { $total }: { $packed } in { $packs } ({ $pack_size }), { $loose } loose ({ $loose_size }).
# Files among the objects that are none of git's, and the room they take.
maintenance-garbage =
    { $count ->
        [one] { $count } file among them is none of git's ({ $size }).
       *[other] { $count } files among them are none of git's ({ $size }).
    }
maintenance-by-type = Commits { $commits } ({ $commits_size }), trees { $trees } ({ $trees_size }), file contents { $blobs } ({ $blobs_size }), tags { $tags } ({ $tags_size }), by the room each takes in the store.
maintenance-lfs = Large files (LFS)
# `tally` is how many file contents there are and their room.
maintenance-lfs-kept = { $tally } kept here.
maintenance-file-contents =
    { $count ->
        [one] { $count } file content, { $size }
       *[other] { $count } file contents, { $size }
    }
# The count stopped short.
maintenance-file-contents-at-least =
    { $count ->
        [one] at least { $count } file content, { $size }
       *[other] at least { $count } file contents, { $size }
    }
maintenance-worktrees = Worktrees
maintenance-files =
    { $count ->
        [one] { $count } file, { $size }
       *[other] { $count } files, { $size }
    }
# The count stopped short.
maintenance-files-at-least =
    { $count ->
        [one] at least { $count } file, { $size }
       *[other] at least { $count } files, { $size }
    }
maintenance-largest = Largest files
maintenance-no-files = History has no files yet.
# The tooltip of one of the largest files; `commit` is the start of the commit's id.
maintenance-came-in-by = Came in by { $commit } { $summary }
maintenance-not-seen = Not seen coming in: the walk stopped before it
# A partial clone left history on its remote; `walked` is how many commits could be walked.
maintenance-walk-left-on-remote =
    { $walked ->
        [0] Where each came in is not known: the partial clone left the history it is read from on the remote.
       *[other] Where each came in is known for the newest { $walked } commits: the partial clone left the rest of history on the remote.
    }
maintenance-walk-complete = Where each came in: the oldest of the { $walked } commits walked that added it.
maintenance-walk-incomplete = Only the newest { $walked } commits were walked for where each came in.

## Git's maintenance tasks.

maintenance-tasks-group = Tasks
maintenance-read-only = This repository is opened read-only: its objects can be measured and checked, not changed.
maintenance-run = Run
maintenance-starting = Starting…
maintenance-gc = Collect garbage
maintenance-gc-note = git gc: packs the loose objects, removes what has expired, tidies the references and their logs, and writes the commit graph.
maintenance-gc-aggressive = Aggressive
maintenance-gc-aggressive-note = git gc --aggressive: the same, looking much harder for what can be stored as a difference. It can take a long time on a large repository.
maintenance-prune = Prune
maintenance-prune-note = git prune: removes loose objects that nothing refers to and that are older than gc.pruneExpire (two weeks unless it is set).
maintenance-repack = Repack
maintenance-repack-note = git repack: everything that is reachable into one pack; what is not is kept apart until it expires, as gc keeps it.
maintenance-commit-graph = Commit graph
maintenance-commit-graph-note = git commit-graph write: the file that makes walking history and drawing the graph faster.
# What the status bar says while a task runs.
maintenance-collecting-garbage = Collecting garbage
maintenance-pruning = Pruning
maintenance-repacking = Repacking
maintenance-writing-commit-graph = Writing the commit graph

## What a task did; `size` is written out with its unit.

maintenance-room-same = The objects take as much room as before
maintenance-room-freed = Freed { $size }
maintenance-room-grew = The objects take { $size } more
# `room` is one of the three above; the packs are counts of packs (above).
maintenance-task-said = { $room }: { $loose_before } loose and { $packs_before } before, { $loose_after } loose and { $packs_after } after.

## The check of the objects.

maintenance-check-group = Check
maintenance-objects-and-links = Objects and links
maintenance-check = Check
maintenance-check-note = git fsck: reads every object and every link between them, and changes nothing.
maintenance-check-failed = The check did not run: { $error }
maintenance-nothing-wrong = Nothing is wrong.
maintenance-could-not-check = Git could not check everything.
maintenance-found-wrong =
    { $count ->
        [one] Git found { $count } thing wrong.
       *[other] Git found { $count } things wrong.
    }
maintenance-found-wrong-unfinished =
    { $count ->
        [one] Git found { $count } thing wrong, and could not check the rest.
       *[other] Git found { $count } things wrong, and could not check the rest.
    }
maintenance-dangling =
    { $count ->
        [one] One object dangles: nothing refers to it, and gc removes it once it is old enough.
       *[other] { $count } objects dangle: nothing refers to them, and gc removes them once they are old enough.
    }
# Under the problems listed, how many more git found.
maintenance-more-problems = and { $count } more
