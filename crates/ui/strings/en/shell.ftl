### The window: tabs, the toolbar, opening and cloning.

## Words the window uses in several places.

shell-cancel = Cancel
shell-close = Close
shell-new-tab = New tab

## The tabs.

shell-find-tab = Find a tab
# The tooltip of a tab with no repository in it.
shell-start-tab-tooltip = Open, clone or make a repository
shell-close-tab = Close the tab
shell-tab-menu-close = Close tab
shell-tab-menu-close-others = Close other tabs
shell-tab-menu-name = Name this tab…
shell-tab-menu-copy-path = Copy the path
shell-tab-menu-move-left = Move left
shell-tab-menu-move-right = Move right
shell-tab-name-no-repository = Only a tab with a repository in it can be given a name
shell-tab-name = Tab name
shell-tab-name-placeholder = its folder's name when empty
shell-tab-name-save = Save
shell-tab-name-clear = Clear
# The button that lists every tab.
shell-tab-list = Tabs
# The caption, in the list of tabs, over the tabs with a repository in them.
shell-tab-list-repositories = OPEN TABS

## The page of repositories.

# What the page's tab is called, in its tooltip and to assistive technology.
shell-repositories-tab = Repositories
# $repository is the repository's name; $error is what went wrong.
shell-workspace-profile-not-taken = The workspace's profile was not taken in { $repository }: { $error }

## The inbox's tab and the status bar's summary.

# What the inbox's tab says in its tooltip; $name is the inbox's name.
shell-inbox-nothing-waits = { $name }: nothing waits for you
shell-inbox-pulls-wait =
    { $name }: { $pulls ->
        [one] { $pulls } pull request
       *[other] { $pulls } pull requests
    }
shell-inbox-issues-wait =
    { $name }: { $issues ->
        [one] { $issues } issue
       *[other] { $issues } issues
    }
shell-inbox-both-wait =
    { $name }: { $pulls ->
        [one] { $pulls } pull request
       *[other] { $pulls } pull requests
    } and { $issues ->
        [one] { $issues } issue
       *[other] { $issues } issues
    }
shell-inbox-connect = Connect an account
shell-inbox-refresh = Read again from the providers

## The tab bar's right end.

shell-notifications = Notifications
# The bell's tooltip: how many notifications are new.
shell-notifications-new =
    { $count ->
        [0] Notifications
       *[other] Notifications: { $count } new
    }
shell-preferences = Preferences

## Who commits: the profiles at the tab bar's right end.

shell-profiles = Profiles
shell-profiles-tooltip = Profiles: who to commit as
# $name is the profile's name.
shell-profile = Profile: { $name }
shell-profile-current = Current profile
shell-profile-commits-as = Commits as
shell-profile-manage = Manage profiles
shell-profile-switched = Committing as the profile { $name } wherever a repository says nothing else

## The repository and the branch at the toolbar's left, and their panels.

# The small captions over the repository's name and the branch's.
shell-finder-repository = repository
shell-finder-branch = branch
shell-find-repository = Find a repository
shell-find-branch = Find a branch
shell-finder-recent = Recent
shell-finder-no-other = No other repository
shell-finder-open = Open a repository…
shell-finder-file-manager = Show in the file manager
shell-finder-editor = Open in the editor
# The terminal inside the window.
shell-finder-terminal = Open a terminal here
shell-finder-system-terminal = Open the system's terminal here
shell-finder-maintenance = Maintenance…

## The toolbar.

shell-undo = Undo
# $action is what would be undone.
shell-undo-tooltip = Undo: { $action }
shell-nothing-to-undo = Nothing to undo
shell-redo = Redo
# $action is what would be redone.
shell-redo-tooltip = Redo: { $action }
shell-nothing-to-redo = Nothing to redo
shell-pull = Pull
shell-pull-more = Ways to pull
shell-pull-mode-heading = What Pull does in this repository
# $name is the remote's name.
shell-fetch-remote = Fetch { $name }
shell-add-remote = Add a remote…
shell-push = Push
# The push button while the pre-push hook is off; it has room for few letters.
shell-push-no-hook = No hook
shell-push-tooltip = Push the current branch. Right click: with lease, force, tags, a remote
shell-branch = Branch
shell-stash = Stash
shell-pop = Pop
shell-terminal = Terminal
# $key is the key that does it.
shell-terminal-tooltip-key = Show or hide the terminal ({ $key })
shell-terminal-tooltip = Show or hide the terminal
shell-lfs = Large files
# The large files button: three letters.
shell-lfs-button = LFS
shell-lfs-fetch = Fetch large files
shell-lfs-checkout = Put fetched files in place
shell-lfs-pull = Pull large files
shell-lfs-push = Push large files
shell-lfs-patterns = Patterns and files…
shell-actions = Actions
shell-search = Search

## The push button's menu.

shell-push-branch = Push { $branch }
shell-push-branch-no-hook = Push { $branch } without the pre-push hook
shell-push-lease = Push with lease…
shell-push-force = Force push…
shell-push-branch-tags = Push { $branch } and every tag
shell-push-tags = Push every tag
shell-push-branch-to = Push { $branch } to { $remote }
shell-push-skip-hook = Skip the pre-push hook, which could refuse it

## The status bar.

shell-no-repository = no repository open
shell-stop = Stop
shell-activity = Activity log
shell-activity-tooltip = Activity log: what git was run with
shell-shortcuts = Keyboard shortcuts
shell-shortcuts-tooltip = Keyboard shortcuts, and what this is
# How many commits the graph holds; $figures is $count written out.
shell-commits =
    { $count ->
        [one] { $figures } commit
       *[other] { $figures } commits
    }

## The palette.

shell-palette-placeholder = Type a command…
shell-palette-window = Window and views
shell-palette-places = Tabs and repositories
shell-palette-commands = Commands
shell-palette-theme = Theme: { $theme }
# $name is a profile's name.
shell-palette-profile = Profile: commit as { $name }
shell-palette-tab = Go to tab: { $title }
shell-palette-recent = Open recent: { $path }

## A command's parameters, asked for before it runs.

shell-param-run = Run
# A choice left to the command.
shell-param-default = as the command has it
# Beside a parameter that has to be given.
shell-param-needed = needed
shell-param-list = several, separated by commas
shell-param-number = a number
shell-param-number-optional = a number (optional)
shell-param-structured = JSON, or a word
shell-param-path = a path
shell-param-path-optional = a path (optional)
shell-param-revision = a branch, tag or commit
shell-param-revision-optional = a branch, tag or commit (optional)
shell-param-optional = (optional)

## What a command run from the palette answered.

# $label is the command's name.
shell-ran = Ran: { $label }
shell-output-more-lines = … and { $count } more lines
shell-output-copy = Copy

## A repository opened read-only.

# $state says, in short, that the repository is opened read-only; $action is what was asked for.
shell-read-only-refused = { $state }, so "{ $action }" is not on offer here.

## Themes and keys of the user's own.

# $theme is the name of the chosen theme, in quotes.
shell-theme-missing = the theme that is chosen, { $theme }, is not there; the dark theme shows instead
shell-themes-reloaded = The themes were read again
shell-example-theme-there = there is an example.toml already
# $file is the full path of the theme file written.
shell-example-theme-written = Wrote { $file }; it is among the themes as "Example"
# $why is what went wrong.
shell-example-theme-not-written = No example theme was written: { $why }
# $problems is what is wrong, each separated from the next by a semicolon.
shell-themes-problems = Themes: { $problems }
shell-themes-problems-more = Themes: { $problems }; and { $more } more (see the preferences)
shell-keys-reloaded = The keys were read again
# $complaints is what is wrong, each separated from the next by a semicolon.
shell-keys-problems = keybindings.toml: { $complaints }
shell-keys-problems-more = keybindings.toml: { $complaints }; and { $more } more (see the shortcuts)

## Links to the application.

shell-link-broken = That link cannot be followed: { $error }
shell-link-open-title = Open a repository a link asked for?
shell-link-open-description = { $path } has not been open here before. Opening a repository lets git act on what that repository's own settings say; go on if you know where it comes from.
shell-link-clone-first = No repository that was open here comes from { $url }; clone it first

## Git asks for something (a password, an answer).

shell-ask-secret-title = Git needs a password or passphrase
shell-ask-title = Git asks
shell-ask-confirm-title = Git asks for confirmation
# $during says which operation is asking.
shell-ask-secret-during = { $during } What you type is handed to git and not kept.
shell-ask-secret-note = What you type is handed to git and not kept.
shell-ask-yes = Yes
shell-ask-ok = OK

## Opening a repository.

# The prompt of the system's folder chooser.
shell-open-prompt = Open repository
# $error is what went wrong, as git or the engine said it.
shell-cannot-open = Cannot open { $path }: { $error }

## Where a repository whose folder is gone is now.

shell-locate-placeholder = the folder it is in now
shell-locate-title = Where is { $name } now?
# $path is where the repository was.
shell-locate-description = { $path } is not there any more. Where the repository is now, everything remembered about it goes with it.
shell-locate-browse = Browse…
shell-locate-ok = It is there
# The prompt of the system's folder chooser.
shell-locate-prompt = The repository is here
shell-locate-not-full = The new place has to be a full path.
shell-located = { $name } is at { $path } from now on

## Cloning a repository.

shell-one-at-a-time = One repository at a time: another is being made
shell-browse = Browse
shell-clone-title = Clone a repository
shell-clone-ok = Clone
# The place in the dialog's list on the left for a clone from an address.
shell-clone-from-address = From an address
shell-clone-into = Into folder
shell-clone-into-placeholder = folder to clone into (new or empty)
# The prompt of the system's folder chooser.
shell-clone-into-prompt = Clone into
shell-clone-into-not-full = The folder to clone into has to be a full path.
shell-clone-from = Repository
shell-clone-from-placeholder = URL or path of the repository
shell-clone-branch = Branch
shell-clone-branch-placeholder = branch (the remote's own)
shell-clone-depth = Depth
shell-clone-depth-placeholder = how many commits back
# $typed is what was typed instead.
shell-clone-depth-not-number = How many commits of history is a whole number from 1 up, not "{ $typed }"
shell-clone-shallow = Shallow clone
shell-clone-submodules = Submodules too
shell-clone-sparse = Sparse checkout
shell-clone-partial = Partial clone
shell-clone-partial-none = No: everything
# What a partial clone leaves on the remote; git's own name for it in brackets.
shell-clone-filter-blobs = Without file contents (blob:none)
shell-clone-filter-trees = Commits only (tree:0)
shell-clone-local-submodule = The clone was given up: a submodule is a folder on this machine, and git does not clone from one by itself. Nothing was left behind.
shell-clone-allow-local = Allow it and clone again
# $done is what was done, such as "Cloned tokio".
shell-clone-not-partial = { $done }, in full: the remote does not leave anything out of a clone, so everything came

## Making a new repository.

shell-init-title = New repository
shell-init-ok = Create
# The place in the dialog's list on the left for a repository made here.
shell-init-on-this-machine = On this machine
shell-init-name = Name
shell-init-name-placeholder = name of the repository's folder
shell-init-name-not-folder = The name is a folder's name: no slashes, and not . or ...
shell-init-in = In folder
shell-init-in-placeholder = the folder it is made in, as a full path
shell-init-in-not-full = The folder it is made in has to be a full path.
# The prompt of the system's folder chooser.
shell-init-prompt = New repository in
shell-init-full-path = Full path
shell-init-branch = First branch
shell-init-branch-placeholder = git's own when empty
# No .gitignore template.
shell-init-gitignore-none = None
shell-init-first-commit = First commit

## In the status bar while a repository is cloned or made, and when it is.

shell-cloning = Cloning { $name }
shell-cloned = Cloned { $name }
shell-creating = Creating { $name }
shell-created = Created the repository { $name }
# $label is what was being done, such as "Cloning tokio".
shell-stopped = { $label }: stopped
# Made at a provider too; $site is the provider, such as GitHub.
shell-provider-making = Making { $owner }/{ $name } at { $site }
shell-provider-made = Made { $owner }/{ $name } at { $site }; it is the remote origin and has the first commit

## The toolbar in a narrow window: the buttons it has no room for, in a menu.

shell-toolbar-more = More
shell-toolbar-more-tooltip = What the toolbar has no room for
