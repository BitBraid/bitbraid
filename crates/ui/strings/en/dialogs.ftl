### The dialogs and questions of a repository's tab: branches, tags, stashes, submodules, signers, sync and the rest.

## A new branch, what a branch follows, a branch deleted.

# Said under the name of a new branch while it is typed.
dialogs-branch-exists = There is a branch { $name } already.
# other is the branch there is; typed is the name typed.
dialogs-branch-folder-clash = There is a branch { $other }: { $typed } cannot exist beside it, one would be a folder of the other.
dialogs-branch-tag-too = There is a tag { $typed } as well: where the name could mean either, git takes the tag and warns.
# start is a branch, a tag, or a commit's id cut short.
dialogs-new-branch-at = New branch at { $start }
dialogs-new-branch-at-head = New branch at HEAD
dialogs-create-branch = Create branch
dialogs-check-it-out = Check it out
dialogs-created-branch-checked-out = Created branch { $name } and checked it out
dialogs-created-branch-not-checked-out = Created branch { $name }; it is not checked out
dialogs-create-branch-dirty = Local changes are in the way of checking out { $name } there, so it was not created. Commit or stash them first, or create it without checking it out.
dialogs-upstream-none-known = No branch of a remote is known here for { $name } to follow. Pushing it publishes it, and it follows what was pushed.
dialogs-upstream-title = Upstream of { $name }
dialogs-set-upstream = Set upstream
dialogs-upstream-placeholder = a branch of a remote: type to narrow the list
dialogs-upstream-set = { $name } follows { $upstream }
# remote and branch name the branch it follows.
dialogs-delete-branch-or-both = Delete branch { $name }, or both it and { $remote }/{ $branch } on the remote?
dialogs-delete-both = Delete both
dialogs-delete-branch-question = Delete branch { $name }? Undo brings it back.
dialogs-delete-unmerged-question = { $name } has commits on no other branch. Delete it all the same?
dialogs-deleted-branch = Deleted branch { $name }

## The pane that docks beside the panel to add a remote, a worktree or a
## submodule: its title, its button, the word over each field and what
## the field says while it is empty.

dialogs-add-remote-title = Add a remote
dialogs-new-worktree-title = New worktree
dialogs-add-submodule-title = Add a submodule
dialogs-add-and-fetch = Add and fetch
dialogs-add-worktree = Add worktree
dialogs-add-submodule = Add submodule
dialogs-remote-name = Name
dialogs-remote-name-placeholder = origin, for one
dialogs-remote-url-placeholder = where it is fetched from
dialogs-remote-push-url = Push URL
dialogs-remote-push-url-placeholder = the same as the URL
dialogs-worktree-check-out = Check out
dialogs-worktree-new-branch = New branch
dialogs-worktree-new-branch-placeholder = none: what is checked out, as it is
dialogs-folder = Folder
dialogs-submodule-repository = Repository
dialogs-submodule-repository-placeholder = URL or path to clone from
dialogs-submodule-folder-placeholder = inside this repository
dialogs-submodule-branch = Branch
dialogs-submodule-branch-placeholder = the default one
# name is what was typed, in quotes.
dialogs-remote-name-refused = { $name } cannot be the name of a remote.
dialogs-remote-exists = There is a remote { $name } already.
dialogs-worktree-needs-branch = With nothing named to check out, the worktree needs a new branch: it begins at HEAD.
# start is what was named to check out.
dialogs-worktree-checked-out-already = { $start } is checked out in a worktree already: name a new branch to begin there.
dialogs-worktree-detached = { $start } is checked out without a branch (detached).
dialogs-worktree-adding = Adding a worktree
# name is the name of the worktree's folder.
dialogs-worktree-added = Added the worktree { $name }
dialogs-worktree-folder-prompt = Choose the worktree's folder
# The button that lists the references to check out.
dialogs-branches-and-tags = Branches and tags
dialogs-browse = Browse

## Gitflow: a branch started, the one checked out finished. kind is
## gitflow's word for a kind of branch (feature, bugfix, release, hotfix,
## support), as the engine names it.

dialogs-flow-not-set-up-yet = Gitflow is not set up in this repository yet
dialogs-flow-not-set-up = Gitflow is not set up in this repository
# A support branch where it stands alone; the other kinds are called by
# their word.
dialogs-flow-support-branch = support branch
# kind is the kind's word, or "support branch".
dialogs-flow-start-title = Start a { $kind }
dialogs-flow-start-any-title = Gitflow: start a branch
dialogs-flow-start = Start
dialogs-flow-name-placeholder = name, without the prefix
# Beside each kind to choose from: what its branches begin with, and the
# branch they start from.
dialogs-flow-kind-from = { $prefix }…  from { $base }
dialogs-flow-started = Started { $kind } { $name }
# branch is the support branch's full name.
dialogs-flow-support-not-finished = { $branch } is a support branch, and those are not finished
dialogs-flow-none-checked-out = The branch that is checked out is none of gitflow's: check out the feature, bugfix, release or hotfix to finish

## A connected account's tab of the pane that adds a remote: the
## account's forks of the repository a remote is. login is the account's
## name at the provider, site the provider's name.

# error is what went wrong reading the engine's answer.
dialogs-fork-links-unreadable = The links could not be read ({ $error }).
dialogs-no-answer = No answer.
dialogs-forking = Forking
dialogs-forked = Forked the repository of { $remote } and added it as a remote
dialogs-fork-remote-links-unreadable = The remotes' links could not be read: { $problem }
dialogs-fork-nothing-to-fork = None of this repository's remotes is a repository { $login } can see at { $site }: there is nothing to fork here with it.
# full is the repository at the provider (owner/name); remote is the
# remote here that is it.
dialogs-fork-repository = { $full } at { $site } (remote { $remote })
dialogs-forks-heading = { $login }'s forks of it
dialogs-forks-asking = Asking for them…
dialogs-forks-unlisted = They could not be listed: { $problem }
dialogs-forks-none = { $login } has no fork of it yet.
# name is the remote a fork is added as.
dialogs-fork-is-remote = { $name } is a remote
dialogs-fork-add-as = Add as { $name }
dialogs-fork-and-add = Fork and add remote
dialogs-fork-explained = Makes a fork of { $full } at { $login }, and adds it as a remote named { $name } that is fetched at once.

## Ignoring files, and files no longer tracked. file is the ignore file:
## .gitignore, .git/info/exclude, or its path.

# Files a sentence counts without naming any.
dialogs-files-count = { $count } files
# The last of a few paths named; first is the others, with commas between.
dialogs-named-and-last = { $first } and { $last }
dialogs-ignore-already = { $pattern } is in { $file } already
dialogs-ignoring = Ignoring { $pattern } ({ $file })
# named names the tracked files the pattern covers; count is how many
# there are.
dialogs-ignoring-tracked =
    { $count ->
        [one] Ignoring { $pattern } ({ $file }). { $named } is tracked and stays so until tracking stops
       *[other] Ignoring { $pattern } ({ $file }). { $named } are tracked and stay so until tracking stops
    }
dialogs-untrack-question =
    { $count ->
        [one] { $named }. Git tracks it although an ignore rule covers it. It stays on disk; leaving the index is staged, and unstaging takes it back.
       *[other] { $named }. Git tracks them although ignore rules cover them. They stay on disk; leaving the index is staged, and unstaging takes it back.
    }
dialogs-ignore-local-title = Ignore in this clone only (.git/info/exclude)
dialogs-ignore-global-title = Ignore in all my repositories (my own ignore file)
dialogs-ignore-title = Ignore (.gitignore)
dialogs-ignore = Ignore
dialogs-ignore-placeholder = a pattern, as .gitignore writes it: *.log, /build/, docs/**/*.pdf
dialogs-stop-tracking = Stop tracking
# what names the files.
dialogs-untracked = Stopped tracking { $what }: staged as deleted, still on disk
dialogs-untrack-none = No tracked file is covered by an ignore rule
dialogs-untrack-ignored-title =
    { $count ->
        [one] Stop tracking { $count } ignored file?
       *[other] Stop tracking { $count } ignored files?
    }

## A branch for an issue, asked in the bar; number is the issue's.

dialogs-issue-branch-title = Branch for #{ $number }
dialogs-create-and-check-out = Create and check out
dialogs-create = Create
dialogs-issue-branch-created-checked-out = Created branch { $name } for #{ $number } and checked it out
dialogs-issue-branch-created = Created branch { $name } for #{ $number }; it is not checked out

## The repository's upkeep: maintenance, the check of the objects, the
## sparse checkout.

# What went wrong when the engine's answer to a task was not an answer.
dialogs-answer-not-one = the answer was not one
dialogs-checking-objects = Checking the objects
# done is what is said of the change ("Only docs and the files at the top
# are checked out").
dialogs-sparse-files-left = { $done }. Files with changes were left where they are; check them out again once they are committed or put away.
dialogs-sparse-all-already = Every file is checked out here already
dialogs-sparse-disabling = Checking out every file
dialogs-sparse-disabled = Every file is checked out again

## Submodules: one added, one removed; path is the submodule's folder.

dialogs-submodule-folder-outside = The folder has to be inside this repository.
dialogs-submodule-exists = { $path } is a submodule already.
# branch is what was typed, in quotes.
dialogs-submodule-branch-refused = { $branch } is not the name of a branch.
# usual is the folder it would usually be cloned into.
dialogs-submodule-folder-unnamed = A folder for it is still to be named: { $usual }, for one.
dialogs-submodule-cloned-default = Cloned into { $path } at the default branch of where it comes from; that is staged.
dialogs-submodule-cloned-branch = Cloned into { $path } at { $branch }, which it is set to follow; that is staged.
# The switch of the removal; name is the submodule's name, under which
# git keeps its clone.
dialogs-submodule-purge = Also delete the clone git keeps of it (.git/modules/{ $name }): its commits that are nowhere else are gone for good, and the folder is free for another submodule
dialogs-submodule-removed-purged = Removed the submodule { $path } and the clone git kept of it; that is staged
dialogs-submodule-removed = Removed the submodule { $path }; that is staged
dialogs-submodule-adding = Adding a submodule
dialogs-submodule-added = Added the submodule { $path }; it is staged
dialogs-submodule-added-on = Added the submodule { $path } on { $branch }; it is staged
dialogs-submodule-old-clone-question = An old clone of a submodule at { $path } is in the way. Delete it and add this one?
dialogs-delete-it = Delete it
dialogs-submodule-remove-title = Remove the submodule { $path }?
dialogs-submodule-remove-lead = Its folder is deleted and the removal is staged, for you to commit.
dialogs-submodule-remove-anyway-question = { $path } has work in it that would go. Remove it all the same?

## Merging, rebasing, pushing onto a branch of a remote, and a merge
## commit picked or reverted. here is the branch checked out, or HEAD.

# number is the parent's place among the merge commit's parents.
dialogs-parent-first = Parent { $number }, the line it was merged into
dialogs-parent-merged-in = Parent { $number }, what was merged in
dialogs-push-onto-unknown = { $onto } is not a branch of a remote known here.
# remote and there name the remote's branch it was pushed to.
dialogs-pushed-onto = Pushed { $branch } to { $remote }/{ $there }
dialogs-push-onto-refused = The push was refused: { $onto } has commits that { $branch } does not have. Nothing was changed there.
# target is a branch, or a commit's id cut short.
dialogs-merge-title = Merge { $target } into { $here }
dialogs-merge = Merge
dialogs-rebase-title = Rebase { $here }
dialogs-rebase = Rebase
dialogs-rebase-lead-after = The commits after the one named below move onto the new base; what is before them stays where it is.
dialogs-rebase-lead = Its commits move onto the new base: all that the base does not have, or only the ones after a commit you name.
dialogs-rebase-base-placeholder = the new base: a branch, a tag or a commit
dialogs-rebase-after-placeholder = only the commits after this one (empty: all the base lacks)
dialogs-rebase-autostash = Set local changes aside first and bring them back after (autostash)
# short is the commit's id, cut short.
dialogs-revert-merge-title = Revert merge commit { $short }
dialogs-cherry-pick-merge-title = Cherry-pick merge commit { $short }
dialogs-revert = Revert
dialogs-cherry-pick = Cherry-pick
dialogs-revert-merge-lead = A merge commit has several parents. Reverting it undoes what it changed against the parent you choose, whose line is kept.
dialogs-cherry-pick-merge-lead = A merge commit has several parents. Picking it applies what it changed against the parent you choose.

## Patches: a patch file applied to the files, a mailbox applied as
## commits, and what is asked before a patch is applied.

# A file of the patch that has no lines to count.
dialogs-patch-file-binary = { $path } (binary)
dialogs-patch-changes-none = Git finds no file in it that it would change.
# files names the files, with what is added and removed in each.
dialogs-patch-changes =
    { $total ->
        [one] It changes { $total } file: { $files }.
       *[other] It changes { $total } files: { $files }.
    }
# more is how many files are not named.
dialogs-patch-changes-more = It changes { $total } files: { $files }, and { $more } more.
# Where git says nothing of why a patch does not apply.
dialogs-patch-no-reason = git gave no reason
# reason is what git said.
dialogs-patch-does-not-apply = It does not apply to the files as they are: { $reason }. A three-way merge may still bring it in, and can leave files in conflict.
dialogs-patch-applies = It applies as it is; undo takes it out again.
# name is the patch file's name.
dialogs-patch-apply-title = Apply { $name }?
dialogs-patch-does-not-apply-title = { $name } does not apply as it is
dialogs-patch-file-placeholder = the patch file
dialogs-mailbox-title = Apply a mailbox of patches as commits
dialogs-patch-title = Apply a patch to the files here
dialogs-patch-choose = Choose…
dialogs-apply-without-hooks = Apply without hooks
dialogs-apply = Apply
dialogs-patch-look = Look at it
dialogs-patch-choose-prompt = Choose a patch
dialogs-mailbox-applying = Applying patches
dialogs-mailbox-applied = Applied the patches
dialogs-patch-try-three-way = Try a three-way merge
dialogs-patch-applied-three-way = Brought the patch in with a three-way merge
dialogs-patch-applied = Applied the patch; undo takes it out again

## Pull requests opened, made and checked out; named is a pull request
## as its provider writes it ("#12", "!12").

dialogs-pull-no-head = The provider named no commit at the head of that pull request.
dialogs-pull-not-linked = No remote here is linked to an account at GitHub or GitLab: connect one in the settings' Integrations section.
dialogs-pull-detached = HEAD is not on a branch: start a pull request from a branch's menu.
dialogs-pull-opened = Opened { $named }
dialogs-pull-push-refused = Nothing is pushed from here.
dialogs-pull-push-no-answer = The push ended without an answer.
dialogs-pull-branches-unreadable = The branches could not be read after the push.
# fork is the repository at the provider it comes from.
dialogs-pull-from-fork-question = { $named } comes from { $fork }: add it as a remote, then check its branch out?
dialogs-add-and-check-out = Add and check out
dialogs-pull-checked-out-in = Checked out { $named } in { $branch }
dialogs-checking-out = Checking out
# A link to the application named a pull request by its number, and no remote is linked to an account.
dialogs-pull-link-not-linked = The link names pull request { $number }, but no remote here is linked to an account at GitHub or GitLab: connect one in the settings' Integrations section.
# The provider could not give the pull request a link named: $kind is github or gitlab, $provider the provider's name, $error what it said.
dialogs-pull-link-unopened =
    { $kind ->
        [gitlab] Merge request
       *[other] Pull request
    } { $named } at { $provider } could not be opened: { $error }

## The allowed signers: where they are kept, one allowed, one taken out.
## who names a signer: one name, or several with commas between.

# odd is the name that will not do, in quotes.
dialogs-signer-name-refused = { $odd } cannot be written as a signer: an e-mail address is what is usual.
dialogs-signer-key-both = Give the file or the key itself, not both.
dialogs-signer-key-path = Give the full path of the public key file.
dialogs-signer-key-private = That is a private key. It is the public half that is allowed: the file that ends in .pub.
dialogs-signer-key-incomplete = A public key is its kind and the key, like "ssh-ed25519 AAAA…".
# Where the engine's answer names no file.
dialogs-signers-the-file = the file
dialogs-signers-file-new = SSH signatures are checked against { $path } now; it is new, and nobody is in it yet
dialogs-signers-file-chosen = SSH signatures are checked against { $path } now
# Where the engine's answer names nobody; it begins a sentence.
dialogs-signer-the-signer = The signer
dialogs-signer-allowed-already = That key was allowed for { $who } already
dialogs-signer-allowed = { $who } may sign with that key now
dialogs-signers-file-title = Where the allowed signers are kept
dialogs-choose = Choose
dialogs-signers-file-lead = Git checks a signature made with an SSH key against a file that says whose key is whose. Name one that is there, or one to be made.
dialogs-signers-file = File
dialogs-signers-file-placeholder = the usual place
# Whose settings name the file, each with a word more.
dialogs-signers-for-repository = For this repository
dialogs-signers-for-repository-more = its own settings name the file; kept beside them unless named
dialogs-signers-for-all = For all my repositories
dialogs-signers-for-all-more = your own git settings name it
# id is the commit's, cut short.
dialogs-signer-allow-key-of = Allow the key of { $id }
dialogs-allow = Allow
dialogs-signer-allow-key-lead = The commit carries the SSH key it was signed with. Allowed, commits signed with that key are good signatures by the name below. Make sure the key is theirs: the commit itself cannot tell.
dialogs-signer = Signer
dialogs-signer-add-title = Allow a signer
dialogs-signer-add-lead = Commits signed with this SSH key are good signatures by the name you give. Give the public key as a file or as it stands in one.
dialogs-signer-placeholder = their e-mail address
dialogs-signer-key-file = Key file
dialogs-signer-key-file-placeholder = the full path of a .pub file
dialogs-signer-key = or the key
# An example of a public key as it stands in a file.
dialogs-signer-key-placeholder = ssh-ed25519 AAAA…
dialogs-signer-key-missing = A key is still to be given: a file, or the key itself.
dialogs-signer-remove-title = Remove { $who } from the allowed signers?
dialogs-signer-remove-description = Commits signed with the key { $fingerprint } are then no longer good signatures by { $who }. The key is kept nowhere else here: to allow it again it has to be given again.
dialogs-signer-removed = { $who } is no longer among the allowed signers

## Stashing: what is said once the stash is made, and the dialog.

dialogs-stashed-keep-index-untracked = Stashed the local changes, untracked files too; what was staged is still staged
dialogs-stashed-keep-index = Stashed the local changes; what was staged is still staged
dialogs-stashed-untracked = Stashed the local changes, untracked files too
dialogs-stashed = Stashed the local changes; untracked files stay
dialogs-stash-title = Stash changes
dialogs-stash = Stash
dialogs-stash-untracked = Untracked files too
dialogs-stash-keep-index = Leave what is staged in place as well (keep index)
dialogs-stash-staged-only = Only what is staged: the rest stays as it is, and the two above do not apply

## A new tag.

# Said under the name of a new tag while it is typed.
dialogs-tag-exists = There is a tag { $name } already.
# at is a branch, a tag, or a commit's id cut short.
dialogs-new-tag-at = New tag at { $at }
dialogs-tag-message-placeholder = message (optional: makes it an annotated tag)
dialogs-tag-sign = Sign it, with the key this repository signs with (without a message, its name is the message)
dialogs-created-signed-tag = Created the signed tag { $name }

## A trailer for the message in the commit box; key is the trailer's key
## (Reviewed-by, Fixes).

dialogs-trailer-title = Add a trailer
dialogs-add = Add
dialogs-trailer-key-placeholder = the key, like Reviewed-by or Fixes
dialogs-trailer-value-placeholder = its value
dialogs-trailer-key-title = Add a { $key } trailer
# What goes in a trailer that names a person.
dialogs-trailer-person-placeholder = Name <email>

## Pushes that overwrite, pushes of tags, and fetching on a timer. branch
## is a local branch, remote a remote, and there the branch on the remote
## ("origin/topic").

dialogs-push-force-unseen = Whatever was pushed there since the last fetch goes too, unseen; a push with lease would refuse then.
dialogs-force-push-title = Force push { $branch } to { $remote }?
dialogs-force-push-unknown = Nothing is known here of a branch { $branch } on { $remote }. If there is one, it is replaced, whatever it holds.
dialogs-force-push-level = As last fetched, { $there } has nothing that { $branch } here does not have.
# Under the commits a push would take away: how many are not listed.
dialogs-overwrite-more = and { $more } more
# behind is how many commits the remote's branch has that the local one
# lacks.
dialogs-overwrite-title =
    { $behind ->
        [one] Overwrite { $behind } commit on { $there }?
       *[other] Overwrite { $behind } commits on { $there }?
    }
dialogs-overwrite-lead =
    { $behind ->
        [one] { $there } has { $behind } commit that { $branch } here does not have. Pushing replaces it:
       *[other] { $there } has { $behind } commits that { $branch } here does not have. Pushing replaces them:
    }
# short is the commit the remote's branch was at, cut short.
dialogs-lease-refuses-if-moved = The push is refused if { $there } is no longer at { $short }: nothing that arrived since is lost.
dialogs-push-with-lease = Push with lease
# What the status bar says while the timer fetches.
dialogs-fetching-background = Fetching in the background
# Where git's message does not say which remote wants an answer.
dialogs-a-remote = A remote
# who is the remote as git's message names it, or "A remote".
dialogs-fetch-stopped = Fetching in the background has stopped here: { $who } wants an answer (a user name, a password or a passphrase), and nobody is asked out of nowhere. Fetch by hand to give it; the timer goes on after that.
dialogs-fetch = Fetch
dialogs-push-detached = HEAD is not on a branch, so there is no branch to push.
dialogs-pushed-with-tags = Pushed { $branch } and every tag
dialogs-pushed-to = Pushed { $branch } to { $remote }
dialogs-pushed-by-force = Pushed { $branch } to { $remote } by force
dialogs-pushed-with-lease = Pushed { $branch } to { $remote } with lease
dialogs-lease-broken = The push was refused: { $remote }/{ $branch } is no longer where it was when you were shown what would go. Nothing was overwritten. Fetch, look at what arrived, and decide again.
dialogs-push-unknown-branch = The push was refused: { $remote } has a branch { $branch } that is not known here. Fetch first.
dialogs-pushed-tags-to = Pushed every tag to { $remote }
dialogs-pushed-tags = Pushed every tag
dialogs-pushing-tags = Pushing tags
