### What each command's parameters are.

## Said of a parameter of several commands

# tips: repo.log, repo.graph, repo.history, search.commits, search.content, search.locate
params-tips = where the walk starts: all, head, or {"{"}"ids": [..]{"}"}, the references shown (hidden ones left out)

# zone: search.parse, search.commits, search.locate
params-zone = the time zone dates are read in, like Europe/Vienna or UTC; the system's when absent

# order: repo.log, repo.graph
params-order = by commit date, each line of history together, or by author date; date when absent

# id: workspace.update, workspace.delete, workspace.add, workspace.remove, workspace.sync_folder, workspace.fetch, workspace.pull, workspace.clone_missing, workspace.locate, workspace.defaults, workspace.export
params-workspace-id = the workspace, by its id

# paths: workspace.create, workspace.add
params-workspace-paths = folders that are repositories, by their full paths

# repositories: workspace.create, workspace.add
params-workspace-repositories = [{"{"}"path", "remote_url"{"}"}]: also ones not cloned yet

# paths: workspace.fetch, workspace.pull, workspace.clone_missing
params-workspace-chosen = only these of its repositories, by their paths; all when absent

# parallel: workspace.fetch, workspace.pull, workspace.clone_missing
params-workspace-parallel = how many repositories at once; at most 32, 8 when absent

# layout: workspace.create, workspace.update
params-workspace-layout = the panels and sections a tab starts with; {"{"}{"}"} takes it away

# limit: repo.log, repo.graph
params-limit-commits = stop after this many commits

# first_parent: repo.log, repo.graph, repo.history
params-first-parent = follow first parents only

# ids: repo.commits, sign.signed
params-commit-ids = commit ids

# path: repo.blame, repo.history, stage.hunks, unstage.hunks, discard.hunks, file.open, file.reveal, file.restore, file.bytes, file.write, conflict.detail, conflict.reopen, rerere.forget, tool.diff
params-file-in-worktree = the file, relative to the worktree

# path: repo.open_folder, repo.open_editor, repo.open_terminal
params-folder-to-open = a folder in the worktree, or a full path; the worktree's top when absent

# query: search.parse, search.commits
params-search-query = the search as typed

# now: search.commits, search.locate
params-search-now = unix seconds that ages count back from

# stage_all: commit.create, commit.amend
params-stage-all = stage everything first

# no_verify: commit.create, commit.amend
params-skip-commit-hooks = skip the pre-commit and commit-msg hooks

# mainline: commit.cherry_pick, commit.revert
params-mainline = for a merge commit: which parent it is taken against

# name: branch.create, stash.branch
params-new-branch-name = the new branch's name

# dirty: branch.create, branch.checkout
params-dirty = what to do about local changes in the way

# name: branch.rename, branch.delete
params-branch = the branch

# protected: branch.delete, branch.delete_remote
params-protected = names that are refused

# remote: branch.delete_remote; name: remote.remove, remote.set_url
params-remote = the remote

# name: branch.set_upstream, branch.describe
params-local-branch = the local branch

# no_verify: branch.rebase, rebase.interactive
params-skip-rebase-hook = skip the pre-rebase hook

# prune: remote.fetch, workspace.fetch
params-prune = forget branches that are gone there

# tags: remote.fetch, remote.push, workspace.fetch
params-all-tags = all tags too

# quiet: remote.fetch, workspace.fetch
params-quiet = ask nobody: a remote that wants an answer is not fetched

# mode: remote.pull, workspace.pull
params-pull-mode = how the upstream is brought in

# remote: remote.push, remote.push_preview
params-push-remote = where to; the branch's own remote when absent

# branch: remote.push, remote.push_preview
params-push-branch = which branch; the current one when absent

# stash: stash.apply, stash.pop, stash.show, stash.drop, stash.reword, stash.branch
params-stash = the stash's commit id; the newest when absent

# name: tag.delete, tag.details, tag.delete_remote
params-tag = the tag

# path: conflict.take, conflict.resolve, conflict.write, tool.merge
params-file-in-conflict = the file in conflict

# allow_markers: conflict.resolve, conflict.write
params-allow-markers = also when conflict markers are left in it

# scope: rerere.enable, sign.configure, repo.long_paths_enable
params-config-scope = whose configuration is written; repo when absent

# first: rebase.outline, rebase.interactive
params-plan-first = the oldest commit of the plan

# steps: rebase.outline, rebase.interactive
params-plan-steps = [{"{"}"id", "action": pick|reword|squash|fixup|edit|drop, "message"{"}"}], oldest first

# path: worktree.remove, worktree.lock, worktree.move, worktree.unlock
params-worktree = the worktree

# allow_local: submodule.add, submodule.update
params-submodule-allow-local = let git clone from a local path

# paths: submodule.update, submodule.sync
params-submodule-paths = which submodules; all when absent

# recursive: submodule.update, submodule.sync
params-submodule-recursive = their own submodules too

# path: submodule.remove, submodule.changes
params-submodule = the submodule

# name: flow.start, flow.finish
params-flow-name = its name, without the prefix

# name: profile.delete, profile.use
params-profile = the profile

# description: workspace.create, workspace.update
params-workspace-description = a few words about it

# default_profile: workspace.create, workspace.update
params-workspace-default-profile = the profile its repositories are opened with

# remote: lfs.fetch, lfs.push, lfs.pull
params-lfs-remote = the remote; the one the branch follows when absent

# submodule.changes
params-diff-side = a revision, index, worktree or empty

## repo.open

params-repo-open-path = a folder in the repository; the current one when absent

## repo.status

params-repo-status-limit = stop once this many changed paths were found; the answer then says whether there are more

## repo.graph

params-repo-graph-details = author, committer and summary of each commit too
params-repo-graph-ascii = the graph drawn as text too
params-repo-graph-pinned = branches or revisions whose lines keep the leftmost lanes, in this order
params-repo-graph-stashes = a row for each stash, hung on the commit it was made on

## repo.diff

params-repo-diff-old = the old side: a revision, index, worktree or empty; HEAD when absent
params-repo-diff-new = the new side, likewise; worktree when absent
params-repo-diff-paths = only these files or folders
params-repo-diff-context = unchanged lines around each change; 3 when absent
params-repo-diff-detect-renames = pair files that were renamed; yes when absent
params-repo-diff-whitespace = how whitespace counts
params-repo-diff-max-bytes = files with a side larger than this are listed and not compared

## repo.glance

params-repo-glance-paths = the full paths of the repositories, at most 64

## repo.blame

params-repo-blame-rev = as of this commit; HEAD when absent
params-repo-blame-ignore-revs = pass over the revisions of the ignore-revs file; as git does when absent

## repo.blame_previous

params-repo-blame-previous-path = the file's name in the commit
params-repo-blame-previous-rev = the commit the lines came from

## repo.history

params-repo-history-follow = go on through renames; yes when absent
params-repo-history-limit = walk at most this many commits
params-repo-history-max-entries = stop after this many commits that changed the file

## repo.init

params-repo-init-path = the folder, as a full path; created when missing
params-repo-init-branch = name of the first branch; git's default when absent
params-repo-init-first-commit = make a first commit, so that there is something to branch from
params-repo-init-gitignore = names of the templates the .gitignore starts from

## repo.ignore_templates

params-repo-ignore-templates-names = templates to put together, to see the file they would make

## repo.clone

params-repo-clone-url = where from: a URL or a path
params-repo-clone-path = where to, as a full path: absent or an empty folder
params-repo-clone-branch = check this branch out instead of the remote's default
params-repo-clone-depth = history cut to this many commits
params-repo-clone-filter = a partial clone: leave the content of files, or trees too, on the remote until needed
params-repo-clone-sparse = check out only the files at the top, to choose more later
params-repo-clone-recurse-submodules = clone the submodules too
params-repo-clone-allow-local = let submodules be cloned from folders on this machine

## repo.open_folder

params-repo-open-folder-command = a program to open it with, {"{"}path{"}"} filled in; the file manager when absent

## repo.open_editor

params-repo-open-editor-command = the editor command; its program is started with the folder

## repo.open_terminal

params-repo-open-terminal-command = the terminal, {"{"}path{"}"} filled in; the one found here when absent

## ssh.key_generate

params-ssh-key-generate-path = the new private key, full path; the public key goes beside it with .pub added
params-ssh-key-generate-comment = written into the key; ssh-keygen's user@host when absent

## ssh.key_fingerprint

params-ssh-key-fingerprint-path = a private or a public key file, full path

## terminal.shell

params-terminal-shell-path = a folder in one of its worktrees, relative to this one or full; its top when absent
params-terminal-shell-program = the shell chosen in the preferences; $SHELL, else /bin/sh, when absent
params-terminal-shell-args = the shell's arguments

## terminal.completion_words

params-terminal-completion-words-changes = whether to read what changed; true when absent
params-terminal-completion-words-most-paths = how many tracked paths at most; 50000 when absent

## terminal.complete

params-terminal-complete-line = what is typed before the cursor
params-terminal-complete-folder = where the shell is, from the top of the worktree; the top when absent
params-terminal-complete-shell = posix, fish, nushell, csh, powershell or cmd; posix when absent

## terminal.paste_paths

params-terminal-paste-paths-paths = the files and folders, by their full paths
params-terminal-paste-paths-shell = posix, fish, nushell, csh, powershell, cmd, or the shell's program; posix when absent

## git.info

params-git-info-program = ask this executable instead of the one in use

## git.use

params-git-use-program = a name on PATH or a full path; absent for the default
params-git-use-verify = refuse what does not answer as git; yes when absent

## search.parse

params-search-parse-now = unix seconds that ages like 3d count back from; the clock when absent

## search.commits

params-search-commits-limit = at most this many; 10000 when absent

## search.content

params-search-content-text = the text that was added or removed
params-search-content-regex = take it as a pattern for changed lines instead
params-search-content-paths = only in these files or folders
params-search-content-limit = at most this many

## search.files

params-search-files-query = letters of the path, in order
params-search-files-rev = the commit whose files are looked at; HEAD when absent
params-search-files-limit = at most this many; 50 when absent

## search.locate

params-search-locate-rev = what to jump to, by name
params-search-locate-date = the newest commit at or before this: unix seconds, a date, an age like 3w

## stage.paths, unstage.paths, discard.paths

params-stage-paths-paths = files or folders, relative to the worktree

## stage.hunks, unstage.hunks, discard.hunks

params-stage-hunks-selection = which hunks, and which lines of them: [{"{"}"hunk": 0, "lines": [..]{"}"}]
params-stage-hunks-context = the context the hunks were numbered with; 3 when absent

## ignore.add

params-ignore-add-pattern = what to ignore, as .gitignore writes it; or give path
params-ignore-add-path = a file or folder of the worktree to make the pattern from
params-ignore-add-what = what of the path to ignore; the file when absent
params-ignore-add-target = .gitignore (shared), .git/info/exclude (this clone), or your own ignore file

## ignore.tracked

params-ignore-tracked-limit = how many to list at most; 5000 when absent

## ignore.untrack

params-ignore-untrack-paths = files or folders, relative to the worktree; they stay on disk

## index.set_flag

params-index-set-flag-paths = files, relative to the worktree
params-index-set-flag-flag = which mark
params-index-set-flag-on = set it or take it away

## commit.create

params-commit-create-message = the commit message
params-commit-create-sign = sign it, or not; as the repository is configured when absent
params-commit-create-allow-empty = also when nothing is staged

## commit.amend

params-commit-amend-message = the new message
params-commit-amend-keep-author = leave the author as it is; yes when absent
params-commit-amend-sign = sign it, or not

## commit.messages

params-commit-messages-limit = how many messages at most; 30 when absent

## commit.trailer

params-commit-trailer-message = the commit message to add to
params-commit-trailer-trailers = what to add: [{"{"}"key": "Reviewed-by", "value": "Name <email>"{"}"}]
params-commit-trailer-sign-off = add Signed-off-by for the identity git commits as

## commit.trailer_keys

params-commit-trailer-keys-limit = how many commits to look at; 300 when absent

## commit.people

params-commit-people-limit = look at this many commits; 200 when absent

## undo.to, redo.to

params-undo-to-id = the journal entry to go as far as

## commit.reword

params-commit-reword-commit = the commit, in the history of the current branch
params-commit-reword-message = its new message

## commit.drop

params-commit-drop-commit = the commit to take out

## commit.squash

params-commit-squash-commits = commits that follow one another

## commit.move

params-commit-move-commits = a commit, or commits that follow one another
params-commit-move-up = towards HEAD; down when false

## commit.rewrite_way

params-commit-rewrite-way-commit = the commit that would be rewritten

## commit.cherry_pick

params-commit-cherry-pick-commits = the commits, in the order to apply them
params-commit-cherry-pick-no-commit = leave the changes staged instead of committing

## commit.revert

params-commit-revert-commits = the commits to undo

## file.open

params-file-open-command = the editor, with {"{"}path{"}"} and {"{"}line{"}"} filled in; the system's opener when absent
params-file-open-line = the line to open at
params-file-open-rev = open the file as it is in this commit, from a read-only copy

## file.reveal

params-file-reveal-command = the file manager, with {"{"}path{"}"} filled in with the file's folder

## file.restore

params-file-restore-rev = the commit to take it from

## file.bytes

params-file-bytes-side = worktree, index, HEAD or a revision; worktree when absent
params-file-bytes-max = refuse a file larger than this many bytes; 32 MiB at most

## file.write

params-file-write-text = what the file is to hold
params-file-write-base = the blob id the text was edited from: refused when the file changed since

## branch.create

params-branch-create-start = where it starts; HEAD when absent
params-branch-create-checkout = check it out too

## branch.checkout

params-branch-checkout-target = a local branch, a remote branch, a tag or a commit

## branch.rename

params-branch-rename-new-name = what it is to be called

## branch.delete

params-branch-delete-force = also when it is not merged

## branch.delete_remote

params-branch-delete-remote-branch = the branch there, without the remote's name

## branch.set_upstream

params-branch-set-upstream-upstream = remote/branch; absent clears it

## branch.fast_forward

params-branch-fast-forward-name = a local branch that is not checked out
params-branch-fast-forward-to = where it moves forward to; its upstream when absent

## branch.overview

params-branch-overview-base = what "merged" is asked about
params-branch-overview-stale-days = days without a commit that make a branch stale; 90 when absent
params-branch-overview-now = unix seconds; the clock when absent

## branch.describe

params-branch-describe-description = its description; empty removes it

## branch.delete_merged

params-branch-delete-merged-base = what the branches have to be merged into
params-branch-delete-merged-protected = names that stay
params-branch-delete-merged-dry-run = only say what would go

## branch.merge

params-branch-merge-target = what to merge into the current branch
params-branch-merge-mode = how
params-branch-merge-no-verify = skip the pre-merge-commit and commit-msg hooks

## branch.rebase

params-branch-rebase-onto = what to rebase the current branch onto
params-branch-rebase-after = move only the commits after this one, which is in the branch's history
params-branch-rebase-autostash = set local changes aside before and bring them back after

## branch.predict

params-branch-predict-target = what would be merged
params-branch-predict-ours = what it would be merged into; the current commit when absent
params-branch-predict-aside = write the trial merge's objects outside the repository

## branch.reset

params-branch-reset-target = where the current branch is to point
params-branch-reset-mode = what happens to index and files

## remote.fetch

params-remote-fetch-remote = one remote; all of them when absent
params-remote-fetch-reference = one reference of the remote (refs/pull/7/head), fetched into FETCH_HEAD only

## remote.push

params-remote-push-set-upstream = make what is pushed to the branch's upstream
params-remote-push-force = whether to overwrite what is there
params-remote-push-expect = with lease: the commit the remote's branch has to be at
params-remote-push-to = the branch on the remote, when it has another name
params-remote-push-no-verify = skip the pre-push hook

## remote.add

params-remote-add-name = the remote's name
params-remote-add-url = its address

## remote.set_url

params-remote-set-url-url = the new address
params-remote-set-url-push = the address pushes go to, instead of the one fetched from

## stash.push

params-stash-push-message = what the stash is called
params-stash-push-include-untracked = untracked files too
params-stash-push-keep-index = leave what is staged in place
params-stash-push-staged-only = only what is staged

## stash.apply, stash.pop

params-stash-apply-index = restore what was staged as staged

## stash.show

params-stash-show-context = lines around each change; 3 when absent

## stash.reword

params-stash-reword-message = its new message, one line

## tag.create

params-tag-create-name = the tag's name
params-tag-create-target = what it points at; HEAD when absent
params-tag-create-message = a message makes it an annotated tag
params-tag-create-sign = sign it (needs a message)

## tag.annotate

params-tag-annotate-name = a tag without a message
params-tag-annotate-message = the message it gets

## tag.push

params-tag-push-name = one tag; all of them when absent
params-tag-push-remote = where to; origin, or the only remote, when absent

## tag.delete_remote

params-tag-delete-remote-remote = where; origin, or the only remote, when absent

## conflict.take

params-conflict-take-side = whose version to take

## conflict.write

params-conflict-write-text = what it is resolved to

## rerere.records

params-rerere-records-limit = how many records at most; 200 when absent

## rerere.forget_record

params-rerere-forget-record-id = the record, by the id rerere.records gives

## rerere.enable

params-rerere-enable-on = on or off

## rebase.plan

params-rebase-plan-first = the oldest commit of the plan; needed unless onto is given
params-rebase-plan-onto = a new base: the plan is for the commits it does not have

## rebase.interactive

params-rebase-interactive-onto = replay the commits on this base instead of where they are

## tool.known

params-tool-known-all = also the tools that are known and not on this machine

## tool.diff

params-tool-diff-staged = what is staged, instead of what is not
params-tool-diff-commit = what that commit changed, instead
params-tool-diff-to = the newer of two commits to compare, instead
params-tool-diff-from = the older of the two; without it, against nothing
params-tool-diff-command = a command that uses $LOCAL and $REMOTE; git's diff tool when absent

## tool.merge

params-tool-merge-command = a command that uses $LOCAL, $REMOTE, $BASE and $MERGED; git's merge tool when absent

## worktree.list

params-worktree-list-changes = count the changed files in each; yes when absent

## worktree.changes

params-worktree-changes-known = {"{"}path: stamp{"}"} of worktrees looked at before; one with the same stamp is passed over
params-worktree-changes-skip = worktrees not to be looked at now, by path; one that changed is said to be waiting
params-worktree-changes-at-most = how many worktrees to look at; the others that changed are said to be waiting

## worktree.add

params-worktree-add-path = where the worktree goes
params-worktree-add-branch = an existing local branch to check out there
params-worktree-add-new-branch = or a branch to create for it
params-worktree-add-start = where a new branch, or a detached worktree, starts; HEAD when absent
params-worktree-add-detach = or no branch at all

## worktree.remove

params-worktree-remove-force = with its changes
params-worktree-remove-delete-branch = its branch too

## worktree.lock

params-worktree-lock-reason = why, for whoever finds it locked

## worktree.move

params-worktree-move-to = the folder it moves to, which is not there yet

## submodule.add

params-submodule-add-url = what to clone
params-submodule-add-path = where in the worktree it goes
params-submodule-add-branch = the branch it follows

## submodule.update

params-submodule-update-init = clone those that are missing

## submodule.remove

params-submodule-remove-force = with its own changes
params-submodule-remove-purge = the clone git keeps of it too

## submodule.forget

params-submodule-forget-name = the name the removed submodule had, usually its path

## activity.list

params-activity-list-limit = at most this many; 200 when absent
params-activity-list-questions = also the processes that only ask
params-activity-list-text = kept when the command line or the output has it

## flow.init

params-flow-init-main = the branch releases are on; main or master when absent
params-flow-init-develop = the branch work is gathered on; develop when absent
params-flow-init-feature = prefix of feature branches
params-flow-init-bugfix = prefix of bugfix branches
params-flow-init-release = prefix of release branches
params-flow-init-hotfix = prefix of hotfix branches
params-flow-init-support = prefix of support branches
params-flow-init-tag = prefix of version tags; none when absent

## flow.start

params-flow-start-kind = what to start
params-flow-start-base = what to start from instead

## flow.finish

params-flow-finish-kind = what to finish
params-flow-finish-message = the message of the tag
params-flow-finish-keep = leave the branch

## sign.keys

params-sign-keys-ssh-dir = where public key files are looked for; ~/.ssh when absent
params-sign-keys-agent = ask the SSH agent too; yes when absent

## sign.configure

params-sign-configure-format = the kind of signature
params-sign-configure-key = the key to sign with; empty removes it
params-sign-configure-commits = sign commits by default
params-sign-configure-tags = sign tags by default

## sign.verify

params-sign-verify-ids = commit ids, at most 256

## sign.signers_file

params-sign-signers-file-path = the file; the usual place of the scope when absent
params-sign-signers-file-scope = whose configuration names it: repo, user or {"{"}"file": path{"}"}; repo when absent

## sign.allow

params-sign-allow-principals = who: e-mail addresses; the committer of the commit when absent
params-sign-allow-key = a public key, as a .pub file has it
params-sign-allow-key-file = the full path of a public key file
params-sign-allow-commit = the commit whose signing key it is

## sign.disallow

params-sign-disallow-key = the fingerprint of the key, or the key itself
params-sign-disallow-principals = only the line for exactly these; every line with the key when absent

## profile.save

params-profile-save-profile = {"{"}"name", "user_name", "user_email", "signing_key", "signing_format", "ssh_key"{"}"}
params-profile-save-rename-from = the name it had until now

## profile.current

params-profile-current-scope = what one configuration says: repo, user or {"{"}"file": path{"}"}

## profile.use

params-profile-use-scope = where it is written: repo, user or {"{"}"file": path{"}"}

## workspace.list

params-workspace-list-id = only this workspace

## workspace.create

params-workspace-create-name = what it is called
params-workspace-create-colour = its colour, #rrggbb
params-workspace-create-icon = the name of its icon
params-workspace-create-follows-folder = a folder whose repositories it keeps in step with, by its full path

## workspace.update

params-workspace-update-name = what it is called from now on
params-workspace-update-colour = its colour, #rrggbb; empty takes it away
params-workspace-update-icon = the name of its icon; empty takes it away
params-workspace-update-follows-folder = a folder whose repositories it keeps in step with

## workspace.reorder

params-workspace-reorder-ids = the workspaces in the order they are to have; the rest after

## workspace.remove

params-workspace-remove-paths = the repositories to take out, by their paths

## workspace.scan

params-workspace-scan-path = the folder, by its full path
params-workspace-scan-depth = how many levels down; at most 3, the default
params-workspace-scan-limit = stop after this many repositories; at most 500, the default

## workspace.import_code_workspace

params-workspace-import-code-workspace-file = the .code-workspace file, by its full path
params-workspace-import-code-workspace-id = add its repositories to this workspace
params-workspace-import-code-workspace-name = or make a workspace of them called this

## workspace.clone_missing

params-workspace-clone-missing-folder = clone into this folder instead, by its full path

## workspace.locate

params-workspace-locate-path = the repository's path as the workspace has it
params-workspace-locate-to = where it is now, by its full path

## workspace.defaults

params-workspace-defaults-path = the repository that is opened, by its full path
params-workspace-defaults-apply-profile = write the default profile there; yes when absent

## workspace.export

params-workspace-export-file = the file to write, by its full path
params-workspace-export-overwrite = write over a file that is there

## workspace.import

params-workspace-import-file = the workspace file, by its full path
params-workspace-import-folder = where its repositories are or go; the file's folder when absent
params-workspace-import-name = call it this instead

## link.parse

params-link-parse-url = the link

## link.make

params-link-make-path = a repository by its full path
params-link-make-url = or by the address of a remote; the open repository when neither
params-link-make-branch = a branch in it
params-link-make-tag = or a tag
params-link-make-commit = or a commit

## link.register

params-link-register-exe = the program links are handed to; the running one when absent
params-link-register-applications-dir = where the desktop entry goes
params-link-register-dry-run = only say what would be written
params-link-register-no-mime = write the entry and leave the system's choice as it is

## patch.export

params-patch-export-commits = these commits, each by itself
params-patch-export-from = or the commits after this one
params-patch-export-to = up to this one; HEAD when absent
params-patch-export-staged = or what is staged
params-patch-export-worktree = or what is changed and not staged
params-patch-export-dir = a folder, one file per commit
params-patch-export-file = or one file: a mailbox, or the diff
params-patch-export-overwrite = replace files that exist

## patch.check, patch.apply

params-patch-check-file = the patch file
params-patch-check-index = to the index too
params-patch-check-three-way = merge where it does not apply as it is
params-patch-check-reverse = take the change out instead

## patch.mailbox

params-patch-mailbox-file = the mailbox file
params-patch-mailbox-three-way = merge where a patch does not apply as it is
params-patch-mailbox-no-verify = skip the applypatch-msg and pre-applypatch hooks

## lfs.track

params-lfs-track-pattern = which files, as .gitattributes writes it
params-lfs-track-lockable = mark its files as to be locked, or take the mark away

## lfs.untrack

params-lfs-untrack-pattern = the pattern to take out

## lfs.files

params-lfs-files-rev = of this commit; HEAD when absent

## lfs.fetch, lfs.push

params-lfs-fetch-all = for every commit of every reference

## lfs.lock

params-lfs-lock-path = the file to lock

## lfs.unlock

params-lfs-unlock-path = the file
params-lfs-unlock-force = also someone else's lock

## repo.maintenance

params-repo-maintenance-task = which of git's maintenance tasks

## repo.size

params-repo-size-blobs = how many of the largest blobs to list; 10 when absent
params-repo-size-commits = how many commits to walk for where they came in

## sparse.set, sparse.add

params-sparse-set-directories = directories to check out, with the files at the top
params-sparse-set-patterns = or patterns, as .gitignore writes them, of what to check out

## Why what was typed into a prompt is refused (after the parameter's name)

params-no-such-parameter = the command takes no such parameter
params-missing = missing
params-is-yes-or-no = is yes or no
params-is-a-whole-number = is a whole number
# The choices are the words the parameter takes, separated by commas.
params-is-one-of = is one of { $choices }
