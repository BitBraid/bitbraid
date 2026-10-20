### Large files (Git LFS).

## Setting LFS up.

lfs-set-up-group = Set up
lfs-row = LFS
lfs-install = Set LFS up in this repository
lfs-installed = LFS is set up in this repository
lfs-not-installed = git-lfs is not installed on this machine; large files that are tracked here stay pointers until it is.
# `version` is what git-lfs says its version is.
lfs-set-up-here = LFS is set up here ({ $version }).
lfs-neither-set-up = git-lfs is installed, and neither its hooks nor its filter are set up in this repository.
lfs-hooks-not-set-up = git-lfs is installed, and its hooks are not set up in this repository.
lfs-filter-not-set-up = git-lfs is installed, and its filter is not set up in this repository.

## The patterns LFS tracks.

lfs-tracking-group = Tracking patterns
lfs-new-pattern = New pattern
lfs-pattern-placeholder = a pattern, for example *.psd
lfs-track = Track
lfs-track-explained = A file that matches a tracked pattern is kept as a large file; the pattern is written to .gitattributes.
lfs-tracked-now = { $pattern } is tracked by LFS; .gitattributes is changed for you to commit
lfs-tracked = Tracked
lfs-no-pattern = No pattern is tracked yet.
# Beside a pattern whose files can be locked.
lfs-lockable = lockable
lfs-untrack = Untrack
lfs-untracked-now = { $pattern } is no longer tracked by LFS

## Transfers.

lfs-large-files-group = Large files
lfs-transfers = Transfers
lfs-fetch = Fetch
lfs-fetching = Fetching large files
lfs-fetched = Fetched the large files
lfs-pull = Pull
lfs-pulling = Pulling large files
lfs-pulled = Pulled the large files
lfs-push = Push
lfs-pushing = Pushing large files
lfs-pushed = Pushed the large files
lfs-check-out = Check out
lfs-checking-out = Checking out large files
lfs-checked-out = Put the large files that are here in place
lfs-files =
    { $count ->
        [0] No file here is an LFS file.
        [one] { $count } file here is an LFS file.
       *[other] { $count } files here are LFS files.
    }

## Locks.

lfs-locks-group = Locks
lfs-lock-a-file = Lock a file
lfs-lock-placeholder = a file to lock, from the worktree's top
lfs-lock = Lock
lfs-lock-explained = A path from the worktree's top.
lfs-locks-held = Locks held
lfs-show-locks = Show locks
lfs-ask-again = Ask again
lfs-asking = Asking the server…
lfs-locks-not-asked = Locks are kept by the server: listing them goes over the network, and is done when you ask.
lfs-locks-unreadable = The locks could not be read: { $error }
lfs-no-locks = No file is locked on the server.
lfs-unlock = Unlock
lfs-unlock-by-force = Unlock by force…
# Whose a lock is, beside its path.
lfs-held-yours = yours
lfs-held-by = held by { $owner }
lfs-held-by-somebody = held by somebody
# `who` is one of the three above; `since` the time as the server writes it.
lfs-held-since = { $who }, since { $since }
