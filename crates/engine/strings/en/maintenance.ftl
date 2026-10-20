### Maintenance.

## The commands: what the palette and the command line call each, and what it does.

maintenance-repo-maintenance = Maintenance
maintenance-repo-maintenance-description = Run one of git's maintenance tasks and say what it freed, counting the objects before and after; nothing git would keep is removed; params: task (gc: pack and tidy | gc_aggressive: the same, packing harder and slowly | prune: remove unreachable loose objects older than gc.pruneExpire | repack: one pack for what is reachable, unreachable objects kept apart | commit_graph: write the commit graph)
maintenance-repo-fsck = Check objects
maintenance-repo-fsck-description = Check every object and the links between them with git fsck, changing nothing: what is missing or damaged, and the dangling objects nothing refers to
maintenance-repo-size = Repository size
maintenance-repo-size-description = How large the repository is: objects loose and packed and by type, the LFS store, the files of each worktree, and the largest blobs in history with the path and commit each came in by; params: blobs (how many, 10 when absent, at most 100), commits (how many to walk for where they came in, 100000 when absent)

## Refusals.

# More of the largest files were asked for than max.
maintenance-too-many-blobs = at most { $max } are listed
# More commits were to be walked than max.
maintenance-too-many-commits = at most { $max } are walked
