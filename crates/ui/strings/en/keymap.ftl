### What each key does.

## Where a key applies: a heading of the keyboard shortcuts, and a word in
## sentences about a key (in lower case there).

keymap-place-everywhere = Everywhere
keymap-place-graph = Graph
keymap-place-references = References
keymap-place-changes = Changes
keymap-place-diff = Diff
keymap-place-editor = Editor
keymap-place-blame = Blame
# The keys of a view that has no name of its own here.
keymap-place-elsewhere = Elsewhere

## Keys pressed one after another, as a key is shown: "Ctrl+K, then Ctrl+S".

# first is what comes before (itself perhaps keys one after another), next the key after it.
keymap-key-then = { $first }, then { $next }

## What is wrong in keybindings.toml, said on the keyboard shortcuts.

# id is a name in the file, cut short.
keymap-not-bindable = "{ $id }" is not something a key can be bound to
# key is what the file has for a key, cut short; id the name of the action.
keymap-not-a-key = "{ $key }" is not a key (for { $id })
# Two actions wanted one key: key is the key, winner the name of the action that keeps it, id the other's name, place where the key applies (in lower case).
keymap-conflict = { $key } is the key of { $winner } and of { $id } ({ $place }); { $winner } keeps it

## What giving a key to an action would do, said before it is saved. key is
## the key, action what the other action does.

keymap-check-taken = { $key } is the key of “{ $action }” now; saving takes it away from there
# place is where the other action applies, in lower case.
keymap-check-kept = In the { $place } view { $key } stays the key of “{ $action }”
keymap-check-hides = Here { $key } will no longer reach “{ $action }”
keymap-check-typed = { $key } is also used for typing: it will not apply while a text field is typed in

## Reading and writing keybindings.toml. error and why are what the system or the reader of the file said.

keymap-file-unreadable = keybindings.toml cannot be read: { $error }
keymap-file-unwritable = keybindings.toml cannot be written: { $error }
keymap-file-keys-not-table = keybindings.toml: keys is not a table
# When keys are to be changed.
keymap-file-not-text-to-change = keybindings.toml is not text; put it right or remove it, then change keys here
keymap-file-broken-to-change = keybindings.toml cannot be read ({ $why }); put it right or remove it, then change keys here
# When the keys are read.
keymap-file-not-text-defaults = keybindings.toml is not text; the default keys are in use
keymap-file-broken-defaults = keybindings.toml cannot be read ({ $why }); the default keys are in use

## What is done with the pointer and has no key, on the keyboard shortcuts:
## how it is done, and (-does) what it does. Middle click on a tab does what
## keymap-tab-close says.

keymap-pointer-several = Ctrl+click, Shift+click
keymap-pointer-several-does = Select several commits
keymap-pointer-label-twice = Two clicks on a label in the graph
keymap-pointer-label-twice-does = Check that branch or tag out (also the faint label of the nearest branch)
keymap-pointer-label-right = Right click on a label in the graph
keymap-pointer-label-right-does = The menu of that branch or tag
keymap-pointer-tab-middle = Middle click on a tab
keymap-pointer-tab-drag = Drag a tab
keymap-pointer-tab-drag-does = Move it to another place
keymap-pointer-branch-drag = Drag a branch onto another
keymap-pointer-branch-drag-does = Merge, rebase or more, from a menu
keymap-pointer-heading-drag = Drag a heading of the references panel
keymap-pointer-heading-drag-does = Put the section in front of another
keymap-pointer-edge-drag = Drag the edge between two open sections
keymap-pointer-edge-drag-does = Give one more of the height

## Everything a key can be bound to, by its id in keybindings.toml: what it
## does, as the palette, the keyboard shortcuts and the menus of a view
## show it, and (-keywords) further words the palette finds it by, in lower
## case and separated by spaces, which are not shown.

## The window and its tabs.

keymap-palette-open = Command palette
keymap-palette-open-keywords = commands run find
keymap-repo-open = Open a repository…
keymap-repo-open-keywords = open folder
keymap-tab-new = New tab
keymap-tab-new-keywords = start
keymap-repo-clone = Clone a repository…
keymap-repo-clone-keywords = clone download url remote
keymap-repo-init = Start a new repository…
keymap-repo-init-keywords = init new create empty
keymap-tab-list = List the tabs
keymap-tab-list-keywords = tabs find switch
keymap-tab-close = Close the tab
keymap-tab-close-keywords = close
keymap-tab-next = Next tab
keymap-tab-next-keywords = switch
keymap-tab-previous = Previous tab
keymap-tab-previous-keywords = switch
keymap-tab-move-left = Move the tab to the left
keymap-tab-move-left-keywords = reorder tabs
keymap-tab-move-right = Move the tab to the right
keymap-tab-move-right-keywords = reorder tabs
keymap-tab-rename = Name this tab…
keymap-tab-rename-keywords = rename alias title
keymap-tab-go-1 = Go to tab 1
keymap-tab-go-2 = Go to tab 2
keymap-tab-go-3 = Go to tab 3
keymap-tab-go-4 = Go to tab 4
keymap-tab-go-5 = Go to tab 5
keymap-tab-go-6 = Go to tab 6
keymap-tab-go-7 = Go to tab 7
keymap-tab-go-8 = Go to tab 8
keymap-tab-go-9 = Go to tab 9
keymap-panel-references = Show or hide the references panel
keymap-panel-references-keywords = left sidebar branches toggle
keymap-panel-details = Show or hide the details panel
keymap-panel-details-keywords = right commit toggle
keymap-view-refresh = Refresh
keymap-view-refresh-keywords = reload
keymap-theme-cycle = Next theme
keymap-theme-cycle-keywords = dark light contrast cycle
keymap-zoom-in = Zoom in
keymap-zoom-in-keywords = bigger
keymap-zoom-out = Zoom out
keymap-zoom-out-keywords = smaller
keymap-zoom-reset = Zoom back to 100 %
keymap-zoom-reset-keywords = reset
keymap-view-fullscreen = Fullscreen: on or off
keymap-view-fullscreen-keywords = full screen maximise window
keymap-preferences-open = Preferences
keymap-preferences-open-keywords = settings options configure git editor theme
keymap-help-shortcuts = Keyboard shortcuts
keymap-help-shortcuts-keywords = help keys bindings about
keymap-help-about = About { -product }
keymap-help-about-keywords = about version licence credits
keymap-view-notifications = Notifications: what was said
keymap-view-notifications-keywords = notifications history toasts messages bell
keymap-keys-change = Change keys…
keymap-keys-change-keywords = keys bindings shortcuts rebind
keymap-keys-reload = Read keybindings.toml again
keymap-keys-reload-keywords = keys bindings shortcuts reload
keymap-keys-reset = Give every key its default again…
keymap-keys-reset-keywords = keys bindings shortcuts reset defaults rebind
keymap-app-quit = Quit
keymap-app-quit-keywords = exit close leave application

## The repository.

keymap-remote-fetch = Fetch every remote
keymap-remote-fetch-keywords = fetch download prune
keymap-remote-pull = Pull the current branch
keymap-remote-pull-keywords = pull download merge rebase
keymap-remote-push = Push the current branch
keymap-remote-push-keywords = push upload publish
keymap-remote-push-lease = Push the current branch with lease…
keymap-remote-push-lease-keywords = push force overwrite lease
keymap-remote-push-force = Force push the current branch…
keymap-remote-push-force-keywords = push force overwrite
keymap-pr-create = Create a pull request of the current branch…
keymap-pr-create-keywords = pull request merge request github gitlab review open
keymap-remote-push-skip-hook = Push without the pre-push hook: switch on or off
keymap-remote-push-skip-hook-keywords = push hook skip verify
keymap-tag-push-all = Push every tag
keymap-tag-push-all-keywords = push tags upload
keymap-branch-create = New branch at HEAD…
keymap-branch-create-keywords = branch create
keymap-stash-push = Stash the uncommitted changes…
keymap-stash-push-keywords = stash save shelve
keymap-stash-pop = Pop the newest stash
keymap-stash-pop-keywords = stash apply restore
keymap-commit-create = Commit what is staged
keymap-commit-create-keywords = commit
keymap-commit-all = Stage everything and commit
keymap-commit-all-keywords = commit stage all
keymap-undo-last = Undo the last git action
keymap-undo-last-keywords = undo revert back
keymap-redo-last = Redo the last undone action
keymap-redo-last-keywords = redo again
keymap-search-commits = Search commits
keymap-search-commits-keywords = find filter
keymap-search-only-matches = Search: show only the commits that were found, or all again
keymap-search-only-matches-keywords = filter matches results narrow
keymap-search-files = Find a file by its path and show its history…
keymap-search-files-keywords = file path fuzzy history open finder
keymap-graph-jump = Jump to a date or reference…
keymap-graph-jump-keywords = go locate when tag branch
keymap-refs-filter = Filter the references
keymap-refs-filter-keywords = find branch tag remote panel fuzzy
keymap-graph-focus = Back to the graph
keymap-graph-focus-keywords = escape focus
keymap-graph-order = Graph: order by date, by author date, or each line of history together
keymap-graph-order-keywords = topological topo date author order sort
keymap-graph-dates = Graph: show author dates, or commit dates
keymap-graph-dates-keywords = author committer commit date column
keymap-repo-open-folder = Show the repository's folder in the file manager
keymap-repo-open-folder-keywords = open folder files explorer finder reveal
keymap-repo-open-terminal = Open the system's terminal in the repository's folder
keymap-repo-open-terminal-keywords = terminal shell console command line

## The terminal inside the window.

keymap-terminal-toggle = Terminal: show or hide
keymap-terminal-toggle-keywords = terminal shell console command line panel prompt
keymap-terminal-new = Terminal: start another
keymap-terminal-new-keywords = terminal shell console new tab plus
keymap-terminal-next = Terminal: the next tab
keymap-terminal-next-keywords = terminal shell console tab switch
keymap-terminal-previous = Terminal: the previous tab
keymap-terminal-previous-keywords = terminal shell console tab switch
keymap-terminal-close = Terminal: end the shell on show
keymap-terminal-close-keywords = terminal shell console close kill exit tab
keymap-repo-open-editor = Open the repository's folder in the editor
keymap-repo-open-editor-keywords = editor code ide open folder
keymap-diff-next-change = Diff: next change
keymap-diff-next-change-keywords = hunk
keymap-diff-previous-change = Diff: previous change
keymap-diff-previous-change-keywords = hunk
keymap-diff-wrap = Diff: wrap long lines, or not
keymap-diff-wrap-keywords = word wrap soft
keymap-diff-against-worktree = Compare the selected commit with the working tree
keymap-diff-against-worktree-keywords = diff commit worktree changes since

## Views that take the centre.

keymap-view-activity = Activity log
keymap-view-activity-keywords = git commands output hooks
keymap-view-undo-history = Undo history
keymap-view-undo-history-keywords = journal back redo
keymap-view-signing = Signing: keys and defaults
keymap-view-signing-keywords = gpg ssh sign key verify
keymap-view-profiles = Profiles: who to commit as
keymap-view-profiles-keywords = identity name email user switch
keymap-view-integrations = Integrations: accounts at GitHub and GitLab
keymap-view-integrations-keywords = github gitlab account token sign in connect provider
keymap-view-repositories = Repositories: every one known, and the workspaces
keymap-view-repositories-keywords = open recent favourites workspaces folders list page
keymap-view-inbox = Inbox: pull requests and issues waiting for you
keymap-view-inbox-keywords = inbox pull requests issues review merge pin snooze work in progress
keymap-view-lfs = Large files (LFS)
keymap-view-lfs-keywords = lfs track binary big
keymap-view-maintenance = Maintenance: the repository's size, clean-up and check
keymap-view-maintenance-keywords = gc garbage prune repack fsck commit graph size largest blobs disk space
keymap-sparse-disable = Sparse checkout: check out every file again
keymap-sparse-disable-keywords = sparse cone partial folders all files
keymap-view-index-flags = Files hidden from the status (index flags)
keymap-view-index-flags-keywords = assume unchanged skip worktree intent
keymap-patch-apply = Apply a patch to the files here…
keymap-patch-apply-keywords = diff apply patch
keymap-patch-mailbox = Apply a mailbox of patches as commits…
keymap-patch-mailbox-keywords = am mbox patch series
keymap-patch-save = Save the changes that are not staged as a patch…
keymap-patch-save-keywords = diff export patch uncommitted worktree
keymap-patch-save-staged = Save the staged changes as a patch…
keymap-patch-save-staged-keywords = diff export patch index cached
keymap-rerere-toggle = Remember conflict resolutions (rerere): switch on or off
keymap-rerere-toggle-keywords = rerere conflict reuse

## What the banner of an operation that stopped offers, and more for the palette.

keymap-op-continue = Continue the merge, rebase or patch series in progress
keymap-op-continue-keywords = continue conflict resolved go on
keymap-op-skip = Skip the commit or patch the operation in progress stopped at
keymap-op-skip-keywords = skip conflict rebase patch
keymap-op-abort = Abort the merge, rebase or patch series in progress…
keymap-op-abort-keywords = abort cancel give up conflict
keymap-view-rerere = Recorded conflict resolutions (rerere): list and forget
keymap-view-rerere-keywords = rerere conflict reuse record forget
keymap-flow-settings = Gitflow: branches and prefixes…
keymap-flow-settings-keywords = gitflow git-flow set up init main develop feature bugfix release hotfix support tag prefix
keymap-flow-start = Gitflow: start a branch…
keymap-flow-start-keywords = gitflow git-flow feature bugfix release hotfix support new
keymap-flow-finish = Gitflow: finish the branch that is checked out…
keymap-flow-finish-keywords = gitflow git-flow feature bugfix release hotfix merge tag
keymap-sign-allow = Signatures: allow a signer's SSH key…
keymap-sign-allow-keywords = signature ssh allowed signers trust key verify
keymap-sign-signers-file = Signatures: choose the allowed-signers file…
keymap-sign-signers-file-keywords = signature ssh allowed signers trust file verify
keymap-commit-trailer = Add a trailer to the commit message…
keymap-commit-trailer-keywords = trailer signed-off reviewed co-author fixes
keymap-ignore-pattern = Ignore a pattern…
keymap-ignore-pattern-keywords = gitignore exclude hide files
keymap-ignore-untrack = Stop tracking files that are ignored…
keymap-ignore-untrack-keywords = gitignore untrack rm cached index
keymap-link-register = Register bitbraid:// links with the desktop
keymap-link-register-keywords = deep link url handler

## Keys of the graph alone. The palette does not offer these: they have no further words.

keymap-graph-select-next = Next commit
keymap-graph-select-previous = Previous commit
keymap-graph-extend-next = Add the next commit to the selection
keymap-graph-extend-previous = Add the previous commit to the selection
keymap-graph-select-first = First commit
keymap-graph-select-last = Last commit
keymap-graph-next-branch = Next branch down the graph
keymap-graph-previous-branch = Next branch up the graph
keymap-graph-page-down = A page down
keymap-graph-page-up = A page up
keymap-graph-copy-id = Copy the selected commit's id

## Keys of the changes panel alone.

keymap-changes-select-next = Next file
keymap-changes-select-previous = Previous file
keymap-changes-select-first = First file
keymap-changes-select-last = Last file
keymap-changes-stage = Stage the selected files
keymap-changes-unstage = Unstage the selected files
keymap-changes-discard = Discard the selected files' changes
keymap-changes-stage-all = Stage everything
keymap-changes-unstage-all = Unstage everything
keymap-changes-discard-all = Discard all unstaged changes (undo brings them back)
keymap-changes-select-all = Select every file of the list the cursor is in
keymap-changes-open = Open the selected file's diff
keymap-changes-tree = Switch between the path list and the tree
keymap-changes-take-ours = Resolve the conflicted file with our side
keymap-changes-take-theirs = Resolve the conflicted file with their side
keymap-changes-mark-resolved = Mark the conflicted file resolved as it is now
keymap-changes-menu = Open the menu of the file

## Keys of the diff, the blame and the editor alone.

keymap-diffview-next-change = Next change
keymap-diffview-previous-change = Previous change
keymap-diffview-copy = Copy the selected text
keymap-blame-copy = Copy the selected text
keymap-edit-save = Save the file
