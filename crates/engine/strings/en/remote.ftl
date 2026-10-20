### Remotes, fetch, pull and push.

## The commands: what the palette and the command line call each, and what it does.

remote-fetch = Fetch
remote-fetch-description = Fetch from one remote or all; params: remote (all when absent), prune (bool), tags (bool), quiet (bool: ask nobody, and fail when a remote wants an answer), reference (one reference of the remote, such as refs/pull/7/head, fetched into FETCH_HEAD only; answers its commit)
remote-pull = Pull
remote-pull-description = Pull the current branch from its upstream; params: mode (ff_only|ff|rebase|merge)
remote-push = Push
remote-push-description = Push a branch; params: remote, branch (current when absent), set_upstream (bool), force (no|lease|force), tags (bool), expect (with lease: the commit the remote's branch has to be at), to (the branch there, when it has another name), no_verify (leave out the pre-push hook; named in skipped_hooks when it was there)
remote-push-preview = What a forced push overwrites
remote-push-preview-description = Say what pushing a branch by force would take away on the remote, as last fetched; params: remote, branch (current when absent)
remote-add = Add remote
remote-add-description = Add a remote; undoable; params: name, url
remote-remove = Remove remote
remote-remove-description = Remove a remote with its remote-tracking branches; undoable; params: name
remote-set-url = Set remote URL
remote-set-url-description = Set the fetch URL of a remote, or its push URL; undoable; params: name, url, push (bool)

## Journal entries: what the undo menu and the activity log call each change. A name is the remote's.

remote-entry-pull = Pull
remote-entry-add = Add remote { $name }
remote-entry-remove = Remove remote { $name }
remote-entry-set-url = Set URL of { $name }
remote-entry-set-push-url = Set push URL of { $name }

## Refusals.

# What follows "invalid parameter "remote": " when a reference is asked for without a remote.
remote-reference-from-one = a reference is fetched from one remote
