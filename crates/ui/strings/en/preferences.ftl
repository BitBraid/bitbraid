### Preferences.

## The column at the left: the way out, who is committed as, the sections.

preferences-close = Close preferences
preferences-current-profile = Current profile
# The caption over the sections every repository shares.
preferences-caption = Preferences
# The caption over the sections of the repository whose tab is open.
preferences-this-repository = This repository
# What a screen reader calls the box with the current profile, which opens
# a list of the profiles to switch to.
preferences-profile = Profile

## The sections, as the column and each section's heading name them.

preferences-section-general = General
preferences-section-profiles = Profiles
preferences-section-ssh = SSH
preferences-section-integrations = Integrations
preferences-section-tools = External Tools
preferences-section-interface = Interface
preferences-section-signing = Commit Signing
# How a file and its changes are shown.
preferences-section-editor = Editor
# The terminal inside the window.
preferences-section-terminal = Terminal
preferences-section-experimental = Experimental
preferences-section-keys = Key bindings
preferences-section-files = Files
preferences-section-gitflow = Gitflow
preferences-section-hooks = Git Hooks
preferences-section-lfs = LFS
preferences-section-maintenance = Maintenance

## General.

preferences-auto-fetch = Fetch in the background, every so many minutes
preferences-auto-fetch-note = 0 to 60; 0 is never. Every remote of each open repository. Nothing is said when it works and nobody is asked for a password: a remote that wants one stops it for that repository, with one notification, until you fetch by hand. It waits while something else runs.
preferences-prune = Prune when fetching
preferences-prune-note = Forget the branches of a remote that are gone there, with each fetch from here.
preferences-fetch-tags = Fetch all tags
preferences-fetch-tags-note = Every tag a remote has, with each fetch from here. Without it git brings the tags that are on what it fetches.
preferences-pull-mode = How pull brings the upstream in
preferences-pull-mode-note = A repository that was pulled another way from the toolbar's menu keeps its own.
preferences-checkout-stash = Stash on checkout
preferences-checkout-stash-note = When local changes are in the way of a checkout: set them aside, check out and bring them back, without asking. What does not fit there stays in the stash, and that is said. Off, a dialog asks whether to stash or to discard.
preferences-protected = Protected branches
preferences-protected-note = Never deleted from here. Names, separated by commas.
preferences-stale-days = A branch is stale after, in days
preferences-stale-days-note = A local branch without a commit for this long is marked stale in the references panel.
# The heading of the settings for issues at GitHub and GitLab.
preferences-issues = Issues
preferences-issue-trailer = Name the branch's issue in commit messages
preferences-issue-trailer-note = On a branch whose name holds an issue's number (12-fix, fix-12, feature/12-x, issue-12), an empty description starts as Refs: #12. The provider shows it as a link to the issue; it does not close it.
preferences-issue-branch = Name of a branch for an issue
# {number} and {title} are written as they are typed in the field.
preferences-issue-branch-note = {"{"}number{"}"} is the issue's number and {"{"}title{"}"} its title in lowercase letters, digits and hyphens, cut to keep the name to 60 characters. The name can be changed before the branch is made.

## General: the updater. What the switch and the row about this copy
## say is in updates.ftl.

# The heading of the updater's settings.
preferences-updates = Updates
preferences-look-for-updates = Look for updates
# The row about this copy of the application and its buttons.
preferences-this-copy = This copy
# Stops the download of a new version.
preferences-updates-stop = Stop
# $version is the version that was installed.
preferences-restart-with = Restart with { $version }
preferences-look-now = Look now
# $version is the newer version found.
preferences-install-version = Install { $version }
# Opens the page of the newer version found.
preferences-open-page = Open its page

## Profiles.

preferences-profiles-intro = A profile is who you commit as: a name, an address, and the keys to sign and to connect with. The current one is written to your git settings, where a repository says nothing else, and each profile keeps the tabs that were open under it. A repository can commit as another profile of its own.
preferences-delete-profile = Delete the profile { $name }? Git's settings stay as they are.
preferences-delete = Delete
# The profile was deleted, but not taken out of the workspaces that named
# it; error is why.
preferences-workspaces-still-name = The workspaces that named the profile still do: { $error }

## SSH: the key the current profile connects with.

preferences-ssh-use-agent = Use the SSH agent
preferences-ssh-agent-note = On: the profile { $name } connects with the keys the SSH agent holds, and ssh's own. Off: with one key file only, written to git's settings as the command ssh is run with.
preferences-ssh-key = SSH private key
preferences-ssh-placeholder = a private key file, ~/.ssh/id_ed25519
preferences-ssh-browse = Browse
# What the desktop's file chooser offers to do with the file chosen.
preferences-ssh-use-this-key = Use this key
preferences-ssh-key-note = The file is checked to be there before it is used. Enter, or leaving the field, takes what is typed.
preferences-ssh-no-key-file = There is no key file at { $key }.
preferences-ssh-connects-with-key = The profile { $name } connects with { $key } now
preferences-ssh-connects-with-agent = The profile { $name } connects with the SSH agent's keys now

## External tools: a group for each program started from here.

preferences-tool-merge = Merge Tool
preferences-tool-diff = Diff Tool
# The program that opens a file.
preferences-tool-editor = Editor
# The program that starts in the repository's folder.
preferences-tool-terminal = Terminal
preferences-tool-file-manager = File manager
preferences-tool-merge-words = External merge tool
preferences-tool-diff-words = External diff tool
preferences-tool-editor-words = External editor
preferences-tool-terminal-words = External terminal
preferences-tool-merge-command = Merge tool command
preferences-tool-diff-command = Diff tool command
preferences-tool-editor-command = Editor command
preferences-tool-terminal-command = Terminal command
preferences-tool-file-manager-command = File manager command
# The first choice of a list: what git, or the system, is set to.
preferences-tool-gits-setting = Git's setting
preferences-tool-systems-choice = The system's choice
preferences-tool-first-found = The first one found
# The last choice of a list: a command of one's own.
preferences-tool-custom = Custom
preferences-tool-merge-note = What resolves a file in conflict. Git's setting is the merge tool git is configured with; the others were found on this system.
preferences-tool-diff-note = What shows two versions of a file. Git's setting is the diff tool git is configured with; the others were found on this system.
preferences-tool-editor-note = What opens a file: what the system opens it with, or a command.
preferences-tool-terminal-note = What starts in the repository's folder: the first terminal found on this system, one of the others, or a command.
preferences-tool-file-manager-note = What shows the repository's folder: what the system opens folders with, or a command.
# Commands between backquotes are set apart in the monospace font; $LOCAL
# and the like, and {path}, are written as they are typed in the field.
preferences-tool-merge-example = Written as git writes tool commands, for example `meld "$LOCAL" "$BASE" "$REMOTE" --output "$MERGED"`. Empty: git's setting.
preferences-tool-diff-example = Written as git writes tool commands, for example `meld "$LOCAL" "$REMOTE"`. Empty: git's setting.
preferences-tool-editor-example = For example `code --goto {"{"}path{"}"}:{"{"}line{"}"}`. Empty: what the system opens the file with.
preferences-tool-terminal-example = For example `kitty --directory {"{"}path{"}"}`; {"{"}path{"}"} is the folder, and the program is started there either way. Empty: the first one found.
preferences-tool-file-manager-example = For example `nautilus {"{"}path{"}"}`. Empty: what the system opens folders with.

## Interface.

preferences-theme = Theme
preferences-zoom = Zoom, in per cent
# larger and smaller are the keys that zoom in and out.
preferences-zoom-note = 50 to 300. { $larger } and { $smaller } change it in steps of ten.
preferences-reduce-motion = Reduce motion
preferences-reduce-motion-note = Off: things move as the system's own setting asks. On: nothing moves: dialogs, notifications and menus appear at once, and the marks that turn while something runs stand still. The caret of a text field still blinks.
preferences-dates = Dates
preferences-dates-in-full = Dates in full
preferences-dates-in-full-note = The date and time of a commit instead of how long ago it was.
preferences-toolbar-labels = Words on the toolbar's buttons
preferences-toolbar-labels-note = Without them a button says what it is under the pointer.
preferences-column-author = Author in the graph
preferences-column-author-note = A column with who wrote each commit.
preferences-column-date = Date in the graph
preferences-column-date-note = A column with when each commit was made.
preferences-column-id = Commit id in the graph
preferences-column-id-note = A column with the short id of each commit.
preferences-date-dividers = Mark where a day, week or month ends in the graph
preferences-date-dividers-note = A line across the rows with what begins there.
preferences-graph-order = Order of the commits
# The command between backquotes is set apart in the monospace font.
preferences-graph-order-note = A commit stands above its parents either way. By date, branches that were worked on at the same time are woven into each other; kept together, each comes whole, as `git log --topo-order` lists them, and the date dividers are left out.
preferences-graph-date = Date of a commit
preferences-graph-date-note = What the date column of the graph shows. A rebase, an amend or a cherry-pick keeps the author date and renews the commit date.
preferences-left-panel = References panel
preferences-left-panel-note = The panel on the left with branches, remotes, tags and the rest.
preferences-right-panel = Details panel
preferences-right-panel-note = The panel on the right with the commit, or the changes to commit.
preferences-changes-tree = Changed files as a tree
preferences-changes-tree-note = Folders that open and close, instead of a list of paths.

## The pictures of authors: what is sent, and to whom.

preferences-pictures = Pictures of authors
# host is the picture service's address, such as gravatar.com.
preferences-pictures-from = Pictures of authors from { $host }
# how is one of the two messages below; kept is the sentence after them,
# with a blank before it, or nothing.
preferences-pictures-note = Off: each author has a pattern made on this machine, and nothing is sent anywhere. On: a SHA-256 hash of the e-mail address of each author whose commits come into view is sent to { $host } { $how }, to ask for that author's picture. The service sees this machine's network address, as any site does. No name, no address as it is written and nothing about the repository is sent.{ $kept }
preferences-pictures-over-https = over HTTPS
preferences-pictures-without-tls = on this machine, without TLS
preferences-pictures-kept = Answers are kept in { $folder } so that nobody is asked twice.
# problem is why the address cannot be asked.
preferences-pictures-bad-address = The address of the picture service in preferences.toml (gravatar_base) is not one that can be asked ({ $problem }). Nobody is asked, whatever this says.
preferences-pictures-unreadable = it cannot be read
preferences-provider-pictures = Pictures from the providers
preferences-provider-pictures-note = Off: nothing is asked of the providers for pictures. On: the authors whose commits come into view are looked up with the connected accounts, those linked to the repository first, then each site's default, with that account's token. GitLab is sent each e-mail address; GitHub is sent none: the repository's recent commits are read from it and matched on this machine. The pictures of those users, of the owners of the remotes and of the accounts are then fetched from the addresses the provider gave, without a token; an address at a picture service such as Gravatar only when the service above is on. An author no provider knows is asked of the service above when that is on.

## Themes of one's own.

preferences-own-themes = Themes of your own
preferences-own-themes-example = Write an example theme there
preferences-own-themes-no-place = This system gave no place to keep files in, so there are no themes of your own.
# dir is the folder the themes are read from.
preferences-own-themes-count =
    { $count ->
        [0] Themes of your own are TOML files in { $dir }; there are none yet.
        [one] { $count } theme of your own, from { $dir }.
       *[other] { $count } themes of your own, from { $dir }.
    }
# theme is the name of the theme in the preferences, between quotes.
preferences-own-theme-missing = The theme that is chosen, { $theme }, is not there; the dark theme shows until its file is back.
preferences-own-themes-note = A theme names what it changes (name, mode = "dark" or "light", and colours under [colors]); everything else is the built-in theme of the same mode. Files are read again when they change, and the theme in use follows its file.

## Editor: how a file and its changes are shown.

preferences-layout = Layout
preferences-layout-hunks = Hunks
preferences-layout-one-column = One column
preferences-layout-two-columns = Two columns
preferences-whitespace = Whitespace
# Whitespace counts as a change.
preferences-whitespace-counts = Counts
preferences-whitespace-ignored-at-ends = Ignored at line ends
preferences-whitespace-ignored = Ignored
preferences-whitespace-cr-ignored = Carriage returns ignored at line ends
preferences-word-diff = Mark the words that changed
preferences-word-diff-note = Inside a line that was edited, what is different from before.
preferences-wrap = Wrap long lines
preferences-wrap-note = A line that does not fit goes on under itself. Alt+Z in a diff.

## The terminal inside the window.

preferences-terminal-font = Font
# font is the name of the monospace font the theme has.
preferences-terminal-font-theirs = { $font } (the theme's)
# A font that was chosen and is not on this machine any more.
preferences-terminal-font-missing = { $font } (not found)
preferences-terminal-font-note = The monospace fonts of this machine.
preferences-terminal-font-size = Font size
preferences-terminal-line-height = Line height
preferences-terminal-line-height-note = How high a line is, in sizes of the font: 1 to 3.
preferences-terminal-cursor = Cursor
preferences-terminal-dim = Dim while elsewhere
preferences-terminal-dim-note = The text is fainter while the keys go to something else than the terminal.
preferences-terminal-shell = Shell
# Commands between backquotes are set apart in the monospace font.
preferences-terminal-shell-note = Empty: the shell your account logs in with. Otherwise the command that starts one, such as `zsh` or `fish --login`. For terminals started from now on.
preferences-terminal-completion = Completion
preferences-terminal-git-completion = Git completion
preferences-terminal-git-completion-note = While a git command is typed at the shell's prompt, a list offers its commands, options, branches, tags, remotes, stashes and files.
preferences-terminal-tab = Tab key
preferences-terminal-tab-note = What Tab does while the list is on show. Enter takes the one marked; Escape puts the list away.

## Experimental: the git that is run.

preferences-git-executable = Git Executable
preferences-git-program = Program
preferences-git-placeholder = git (the one found on PATH)
preferences-git-use = Use
# Goes back to the git found on PATH.
preferences-git-default = Default
# minimum is the oldest version of git that does everything.
preferences-git-note = A name looked up on PATH, or a full path. It is asked for its version before it is used, and every repository that is open runs it from then on. Everything works with git { $minimum } or newer.
preferences-git-in-use = Git { $version } at { $program } is in use from now on.
preferences-git-shipped = There is no git on PATH; the one shipped with { -product } ({ $version }) is in use.
preferences-git-found-again = The git found on PATH ({ $version }) is in use again.
preferences-git-not-found = No git was found on PATH.
preferences-git-none-at-all = there is no git on PATH, and none was shipped with { -product }

## Key bindings.

preferences-keys = Keys
preferences-keys-change = Change keys…
preferences-keys-reload = Read the file again
preferences-keys-reset = All defaults again…
preferences-keys-no-place = This system gave no place to keep files in, so the keys cannot be changed.
# own is how many actions have keys of one's own; file is where they are.
preferences-keys-own =
    { $own ->
        [0] Every key is the default. Keys of your own go into { $file }.
        [one] { $own } action has keys of your own, from { $file }.
       *[other] { $own } actions have keys of your own, from { $file }.
    }
# key is a key as the file writes it, such as ctrl-alt-p.
preferences-keys-note = A key is changed in the sheet of shortcuts: open the row of an action, press the new key, and what it would take from another action is said before it is saved. That writes the file, which can be edited by hand as well: under [keys], "remote.pull" = "{ $key }", a list for several, an empty list for none. Two actions on one key in one place are a conflict: yours wins over a default, and it is said here. The file is read again when it changes.

## Files: where all of this is kept.

preferences-files-folder = Folder
preferences-files-everything-in = Everything is in { $dir }, as text:
preferences-files-no-place = This system gave no place to keep files in; nothing is remembered.
# What preferences.toml holds.
preferences-files-preferences = What these pages hold. It can be edited by hand while the application is closed; a file that cannot be read is kept under another name and the defaults are used.
# What state.toml holds.
preferences-files-state = What is remembered without being asked: the open tabs, recent repositories, and per repository the panel widths and what is hidden.
# What keybindings.toml holds.
preferences-files-keybindings = Keys of your own, by the name of the action (Key bindings).
# What profiles.toml holds.
preferences-files-profiles = The identities to commit as (Profiles).

## Gitflow: the open repository's branches and prefixes.

preferences-flow-branches = Branches
preferences-flow-main = Main branch
preferences-flow-develop = Develop branch
preferences-flow-develop-note = Releases are made on the main branch, and work comes together on the develop branch.
preferences-flow-prefixes = Prefixes
preferences-flow-feature = Feature
preferences-flow-bugfix = Bugfix
preferences-flow-release = Release
preferences-flow-hotfix = Hotfix
preferences-flow-support = Support
preferences-flow-version-tag = Version tag
preferences-flow-version-tag-note = What the tag of a finished release or hotfix begins with; empty: nothing.
preferences-flow-save = Save
# Gitflow is not set up in the repository yet.
preferences-flow-set-up = Set up

## Git hooks of the open repository.

preferences-hooks-intro = Hooks are programs git runs on events such as a commit or a push: they can refuse what is being done. They run as git runs them; what they say is in the activity log, and an operation that offers to leave its hooks out says which.
preferences-hooks-directory = Hooks directory
preferences-hooks-configured = Named by core.hooksPath in git's settings, for this repository and its linked worktrees.
preferences-hooks-own = The repository's own hooks directory: git's settings name no other.
preferences-hooks-that-run = Hooks that run
preferences-hooks-none = None: git runs no hook here.

## Commit signing, with no repository open.

preferences-signing-no-repository = How commits and tags are signed is a repository's setting in git, or your own for every repository. Open a repository: its signing is set up here.
