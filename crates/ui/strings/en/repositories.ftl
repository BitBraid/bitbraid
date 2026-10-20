### Repositories and workspaces.

## The page of repositories: its top.

repositories-heading = Repositories
# Opens a folder chooser: on the page, for a repository to open; in the workspace dialog, for repositories to add.
repositories-browse = Browse
# Clones a repository: on the page, one asked for; in a row's menu, the missing one of the row.
repositories-clone = Clone
# Makes a new repository (git init).
repositories-init = Init
# The button, and the title of the dialog it opens.
repositories-new-workspace = New workspace
repositories-new-workspace-tip = Make a group of repositories to work on together
repositories-import = Import
repositories-import-tip = Take in a workspace from its file
# Folds every section.
repositories-collapse = Collapse
# Unfolds every section.
repositories-expand = Expand
repositories-search = Search
# The box that has the rows say what is uncommitted.
repositories-changes = Changes
# The button that counts the hidden sections.
repositories-hidden = Sections that are hidden
repositories-show-all = Show them all

## The four sections the page always has, and their menus.

# The repositories open in a tab.
repositories-section-open = Open
repositories-section-favourites = Favourites
repositories-section-recent = Recent
repositories-section-all = All
# A section's menu button.
repositories-more = More
repositories-hide = Hide
# Where a section has no repository.
repositories-none = None
repositories-workspaces-unreadable = The workspaces could not be read: { $problem }

## A repository's row.

repositories-open = Open
repositories-close-tab = Close its tab
repositories-unfavourite = Take out of the favourites
repositories-favourite = Make it a favourite
repositories-take-out-recent = Take out of this list
repositories-not-readable = Not readable
repositories-not-found = Not found
# In place of the branch, when the repository is on none.
repositories-detached = detached
# $said is how long ago, in words; $time the date and time.
repositories-last-fetched = Last fetched { $said }, { $time } UTC

## A workspace's section: its heading and its menu.

# Where the workspace is kept: on this machine.
repositories-local = Local
repositories-local-tip = On this machine
repositories-local-following = On this machine; every repository in { $folder } is taken in
repositories-add = Add repositories
repositories-fetch-all = Fetch every one
repositories-open-all = Open every one
repositories-pull-ff = Pull every one, fast-forward if possible
repositories-pull-ff-only = Pull every one, fast-forward only
repositories-pull-rebase = Pull every one with rebase
repositories-pull-merge = Pull every one, always with a merge commit
# The caret beside the pull button, and the heading of its menu.
repositories-pull-mode = How to pull
repositories-pull-mode-heading = Pull these repositories
repositories-tick-all = Tick all
repositories-tick-none = Tick none
# $ticked is how many rows are ticked.
repositories-open-ticked-in-tabs = Open { $ticked } in tabs
repositories-open-all-in-tabs = Open all in tabs
repositories-clone-missing = Clone the missing ones
repositories-locate-missing = Locate a missing one
repositories-edit = Edit
# A workspace's colour: its menu's entry, and the caption over the list in its dialog.
repositories-colour = Colour
# Writes the workspace to a file, to be taken in elsewhere.
repositories-export = Export
repositories-delete = Delete
repositories-remove-ticked = Remove { $ticked } from it
repositories-delete-ask = Delete the workspace { $name }? Its repositories stay where they are.

## The colours a workspace can be given.

# No colour: the heading's own.
repositories-colour-none = None
repositories-colour-blue = Blue
repositories-colour-teal = Teal
repositories-colour-green = Green
repositories-colour-olive = Olive
repositories-colour-amber = Amber
repositories-colour-orange = Orange
repositories-colour-red = Red
repositories-colour-rose = Rose
repositories-colour-violet = Violet
repositories-colour-slate = Slate

## A repository's row in a workspace's section.

# The box that ticks the row; $path is the repository's.
repositories-choose = Choose { $path }
repositories-not-there = { $path } is not there: locate it or clone it
repositories-open-in-editor = Open in the editor
repositories-row-locate = Locate
repositories-row-remove = Remove from the workspace
repositories-no-editor = No editor is named in the preferences (External tools), so there is nothing to open the folder with.

## Choosing folders and files for a workspace, and what came of it.

repositories-add-prompt = Add to the workspace
# $name is the folder's name of the repository that is missing.
repositories-locate-prompt = Where is { $name } now?
repositories-import-prompt = Take in a workspace
repositories-already-in =
    { $already ->
        [one] { $already } of them was in the workspace already.
       *[other] { $already } of them were in the workspace already.
    }
# $shown is the file written.
repositories-written = Written to { $shown }
# What a workspace taken in is called when its file names it nothing.
repositories-the-workspace = The workspace
repositories-imported = { $name } was taken in.
repositories-imported-refused = { $name } was taken in; { $refused } of its repositories were left out: their paths lead outside the folder of the file.

## Commands run on many repositories of a workspace at once.

# The buttons of the bar over a workspace whose rows are ticked; $count is how many they act on.
repositories-bulk-fetch = Fetch { $count }
repositories-bulk-pull = Pull { $count }
repositories-bulk-open = Open { $count }
repositories-bulk-editor = Editor { $count }
repositories-bulk-editor-tip = Open them in the editor
repositories-bulk-locate = Locate { $count }
repositories-bulk-locate-tip = Say where a missing one is now
repositories-bulk-clone = Clone { $count }
repositories-bulk-remove = Remove { $count }
repositories-bulk-remove-tip = Take them out of the workspace; their folders stay
repositories-bulk-stop = Stop
# While a command runs: $kind is fetch, pull or clone; $ended how many have ended of $total.
repositories-bulk-running =
    { $kind ->
        [pull] Pulling { $ended } of { $total }
        [clone] Cloning { $ended } of { $total }
       *[fetch] Fetching { $ended } of { $total }
    }
repositories-bulk-stopping = Stopping: { $ended } of { $total }
# A row whose repository waits for its turn.
repositories-bulk-waiting = Waiting
# A row whose repository's command started and says nothing yet; $kind as above.
repositories-bulk-starting =
    { $kind ->
        [pull] Pulling…
        [clone] Cloning…
       *[fetch] Fetching…
    }
# A row whose repository's command ended well.
repositories-bulk-fetched = Fetched
repositories-bulk-pulled = Pulled
repositories-bulk-cloned = Cloned
# A row whose repository was given nothing to say why it failed.
repositories-bulk-did-not-run = It did not run.
# The cross of a row that says why its command failed.
repositories-take-away = Take this away
repositories-answer-not-understood = The answer was not understood: { $error }
# How a command on many ended: $all is how many it was given, $name the workspace; $done how many went well.
repositories-bulk-went-well =
    { $kind ->
        [pull] Pulled { $all } of { $name }.
        [clone] Cloned { $all } of { $name }.
       *[fetch] Fetched { $all } of { $name }.
    }
repositories-bulk-stopped = Stopped: { $done } of { $all } of { $name } were done.
# $failed is how many failed or were passed over.
repositories-bulk-partly = { $done } of { $all } of { $name } went well; { $failed } did not: their rows say why.

## The dialog that makes a workspace or edits one.

repositories-form-edit = Edit workspace
repositories-form-close = Close
repositories-form-icon = Icon
repositories-form-icon-tip = The icon on its heading
repositories-form-name = Name
repositories-form-name-placeholder = What it is called
# The colour's list, for a screen reader; $named is the colour's name.
repositories-form-colour-is = Colour: { $named }
repositories-form-description = Description
repositories-form-description-placeholder = What it is for (optional)
repositories-form-repositories = Repositories
repositories-form-browse-tip = Pick repositories, or folders to look through for the repositories in them
repositories-form-code = VS Code file
repositories-form-code-tip = Take the folders of a .code-workspace file
repositories-form-code-prompt = Take the folders of this file
repositories-form-follow = Follow the folder
repositories-form-follow-tip = Take in every repository that comes to be in it
repositories-form-count =
    { $count ->
        [one] { $count } repository to be in it
       *[other] { $count } repositories to be in it
    }
repositories-form-clear = Clear
repositories-form-leave-out = Leave it out
# What a folder looked through left out: $first is the first (a path, and why), $more how many others.
repositories-form-left-out =
    { $more ->
        [0] Left out: { $first }
       *[other] Left out: { $first }, and { $more } more
    }
repositories-form-none-found = No repository was found there.
repositories-form-opens-with = Opens with
repositories-form-profile = Profile { $name }
repositories-form-no-profile = No profile
repositories-form-profile-tip = The profile its repositories are opened with
# $from is the repository whose tab's layout is offered, or the words of repositories-form-layout-own.
repositories-form-layout = The layout of { $from }
repositories-form-layout-own = the one it has
repositories-form-no-layout = No layout to keep
repositories-form-create = Create
repositories-form-save = Save
# Why the button that makes or saves it is greyed: $waits is one of the repositories-form-needs-* below.
repositories-form-needs = It needs { $waits }
repositories-form-needs-name = a name
repositories-form-needs-free-name = a name no other workspace has
repositories-form-needs-repository = a repository

## The icons a workspace can be given, as the list of them names them.

repositories-icon-layers = layers
repositories-icon-folder = folder
repositories-icon-package = package
repositories-icon-code-xml = code xml
repositories-icon-globe = globe
repositories-icon-server = server
repositories-icon-book-open = book open
repositories-icon-flask-conical = flask conical
repositories-icon-users = users
repositories-icon-wrench = wrench
repositories-icon-star = star
repositories-icon-heart = heart
