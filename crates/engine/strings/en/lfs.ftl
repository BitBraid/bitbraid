### Large files (Git LFS).

## The commands: what the palette and the command line call each, and what it does.

lfs-status = LFS status
lfs-status-description = Whether git-lfs is installed and which version, whether its hooks and filters are set up here, the tracked patterns with the file that declares each and whether it is lockable, how many files are LFS files, and active when a pattern is tracked
lfs-install = Set up LFS
lfs-install-description = Set the LFS filters in this repository's own configuration and install its hooks; nothing outside the repository is written
lfs-track = Track with LFS
lfs-track-description = Track a pattern with LFS, leaving .gitattributes changed to commit; params: pattern, lockable (bool: mark its files as to be locked, or take the mark away; kept as it is when absent)
lfs-untrack = Stop tracking with LFS
lfs-untrack-description = Take a pattern out of .gitattributes at the top of the worktree, leaving that to commit; params: pattern
lfs-files = LFS files
lfs-files-description = The LFS files of a commit with the id and size of their content, whether it is in the local store and whether the worktree has it; params: rev (HEAD when absent)
lfs-fetch = Fetch LFS files
lfs-fetch-description = Download the content of LFS files without touching the worktree; params: remote (the one the branch follows when absent), all (bool: for every commit of every reference, not just what is checked out)
lfs-pull = Pull LFS files
lfs-pull-description = Download the content of the LFS files that are checked out and put it in place of their pointers; params: remote (the one the branch follows when absent)
lfs-push = Push LFS files
lfs-push-description = Upload the content of the current branch's LFS files; params: remote (the one the branch follows when absent), all (bool: for every commit of every local reference)
lfs-checkout = Check out LFS files
lfs-checkout-description = Put the content of the LFS files that are checked out in place of their pointers, as far as it is in the local store; nothing is downloaded, and files that were changed are left alone
lfs-locks = List LFS locks
lfs-locks-description = The locks the remote's LFS server keeps: file, owner, since when, and whether each is yours
lfs-lock = Lock file
lfs-lock-description = Lock a file on the remote's LFS server; params: path
lfs-unlock = Unlock file
lfs-unlock-description = Give up the lock on a file; params: path, force (bool: also someone else's lock, also with changes in the file)
