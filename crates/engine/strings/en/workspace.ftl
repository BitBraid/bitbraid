### Workspaces: files, scans and actions on many repositories.

## The commands: what the palette and the command line call each, and what it does.

workspace-list = List workspaces
workspace-list-description = The workspaces that are kept, each with its id, name, colour, icon, description, default_profile, follows_folder, layout and repositories ({"{"}path, remote_url{"}"}), and the paths of its repositories that are not there (missing); params: id (only this one)
workspace-create = New workspace
workspace-create-description = Make a workspace; its id is made from its name; params: name, colour (#rrggbb), icon, description, default_profile, follows_folder (a full path), layout, paths (full paths of repositories), repositories ([{"{"}path, remote_url{"}"}]: also ones not cloned yet)
workspace-update = Edit workspace
workspace-update-description = Change a workspace; what is not given stays, an empty text takes it away, and so does an empty layout; a folder to follow brings the repositories in it at once (added); params: id, name, colour, icon, description, default_profile, follows_folder, layout
workspace-delete = Delete workspace
workspace-delete-description = Forget a workspace; its repositories stay as they are; params: id
workspace-reorder = Reorder workspaces
workspace-reorder-description = Put the workspaces in an order, the one they are listed and shown in: those named first, in that order, and the others after them as they were; params: ids
workspace-add = Add repositories to workspace
workspace-add-description = Add repositories to a workspace: folders that are repositories, or ones that are not there yet with the address to clone them from; params: id, paths (full paths), repositories ([{"{"}path, remote_url{"}"}])
workspace-remove = Remove repositories from workspace
workspace-remove-description = Take repositories out of a workspace; their folders stay as they are; params: id, paths
workspace-scan = Find repositories in folder
workspace-scan-description = The repositories in a folder, a few levels down, without following links or going into a repository; answers found ([{"{"}path, remote_url, bare{"}"}]), skipped ([{"{"}path, why{"}"}]), more_skipped and complete; params: path (a full path), depth (at most 3, the default), limit (at most 500, the default)
workspace-sync-folder = Bring workspace in step with its folder
workspace-sync-folder-description = Add to a workspace that follows a folder the repositories found in it since; answers added and what the scan skipped; params: id
workspace-import-code-workspace = Repositories of a VS Code workspace
workspace-import-code-workspace-description = The repositories a .code-workspace file lists among its folders (paths relative to the file), and what it lists that is none; with id they are added to that workspace, with name a workspace of them is made; params: file, id, name
workspace-fetch = Fetch workspace
workspace-fetch-description = Fetch every remote of a workspace's repositories, or of those named, several at once; each repository's progress goes to the progress sink as path, state and line, and the answer says what became of each (queued, running, done, failed or skipped, with the error); params: id, paths, parallel (at most 32; 8 when absent), prune (bool), tags (bool), quiet (bool)
workspace-pull = Pull workspace
workspace-pull-description = Pull the current branch of a workspace's repositories, or of those named, several at once, each as remote.pull does it; params: id, paths, parallel (at most 32; 8 when absent), mode (ff_only|ff|rebase|merge)
workspace-clone-missing = Clone missing repositories
workspace-clone-missing-description = Clone a workspace's repositories that are not there from their addresses, several at once, where the workspace has them or into a folder (and the workspace then has them there); params: id, paths, parallel (at most 32; 8 when absent), folder (a full path)
workspace-locate = Locate repository
workspace-locate-description = Point a workspace's entry for a repository that moved at the folder where it is now, which has to be a repository; answers same_remote (whether it is cloned from the address the entry has, when both say one); params: id, path (the entry's path), to (the new full path)
workspace-defaults = Workspace defaults for a repository
workspace-defaults-description = What a tab of one of a workspace's repositories starts with: its default profile is written to the repository's configuration with profile.use unless that identity is in effect there already (undoable there), and its saved layout is answered for the window to take; params: id, path (the repository), apply_profile (bool; true when absent)
workspace-export = Export workspace
workspace-export-description = Write a workspace to a TOML file that can be kept in a repository: its name, colour, icon, description, default profile and layout, and its repositories by their paths relative to the file's folder and their addresses; one outside that folder by its folder's name; params: id, file (a full path), overwrite (bool)
workspace-import = Import workspace
workspace-import-description = Make a workspace of a file workspace.export wrote: its repositories are found under a folder (the file's when absent), and one whose path would lead out of it is refused and named, as is one that is not there and has no address; params: file (a full path), folder (a full path), name (instead of the file's)

## What is wrong with a field of a workspace, after its name.

workspace-empty = must not be empty
# The most is a number of characters.
workspace-longer-than-characters = is longer than { $most } characters
# The most is a number of bytes.
workspace-longer-than-bytes = is longer than { $most } bytes
workspace-control-characters = must not hold control characters
workspace-colour-form = is a colour written #rrggbb
workspace-icon-length = is a name of 1 to { $most } characters
workspace-icon-characters = has only lowercase letters, digits and dashes
workspace-not-full-path = has to be a full path
workspace-goes-up = must not go up with ..
workspace-not-text = is not text
workspace-starts-with-dash = must not start with -
workspace-id-form = is 1 to { $most } lowercase letters, digits and dashes

## What is wrong with a layout.

# What is one of the two widths below.
workspace-width-bounds = the { $what } is a number of pixels from 0 to { $most }
workspace-left-panel-width = width of the left panel
workspace-right-panel-width = width of the right panel
# Why is one of the reasons above.
workspace-section-name = a section's name { $why }
# What is the key of the list in the file: closed_sections, hidden_sections or section_order.
workspace-too-many-names = { $what } has more than { $most } names
workspace-too-many-sizes = the sizes are more than { $most }
# The name is the section's, in quotes.
workspace-not-a-weight = the size of { $name } is not a weight

## Which field of a workspace or of its repository is wrong; why is one of the reasons above. remote_url, default_profile and follows_folder are keys of the file.

workspace-its-path = its path { $why }
workspace-its-remote-url = its remote_url { $why }
workspace-its-id = its id { $why }
workspace-its-name = its name { $why }
workspace-its-colour = its colour { $why }
workspace-its-icon = its icon { $why }
workspace-its-description = its description { $why }
workspace-its-default-profile = its default_profile { $why }
workspace-its-follows-folder = its follows_folder { $why }
workspace-its-layout = its layout: { $why }
workspace-too-many-repositories = it has more than { $most } repositories
# The number counts the workspace's repositories from 1.
workspace-its-repository = its repository number { $number }: { $why }

## The file of workspaces cannot be used: its path, then one of the reasons below.

workspace-file-unusable = { $path } { $why }; it is left as it is until it is put right or moved away
# The error is what the system said.
workspace-cannot-be-read = cannot be read ({ $error })
workspace-not-a-file = is not a file
workspace-larger-than = is larger than { $kib } KiB
# Said is what the reader of the file said was wrong.
workspace-not-understood-at = cannot be understood (line { $line }: { $said })
workspace-not-understood = cannot be understood ({ $said })
workspace-file-too-many = has { $count } workspaces, and no more than { $most } are kept
workspace-file-newer = was written by a newer version of the application (its version is { $version }), and is read but not written over
# The number counts the workspaces of the file from 1; why is which field is wrong.
workspace-file-unusable-workspace = has a workspace that cannot be used (number { $number }: { $why })
workspace-file-same-id = has two workspaces of one id (the second is number { $number })
workspace-file-same-name = has two workspaces of one name (the second is number { $number })

## Writing a file: after its name.

workspace-cannot-be-written = cannot be written ({ $error })
workspace-cannot-be-placed = cannot be put in its place ({ $error })

## Refusals.

# The operation a refusal names, before a colon.
workspace-refused = workspace
# A workspace's id, name or a profile's name is in quotes.
workspace-no-such = there is no workspace { $id }
workspace-not-written-down = the workspaces cannot be written down: { $why }
workspace-too-large-to-keep = the workspaces would take more than { $kib } KiB, which is more than is read; remove some repositories or workspaces first
workspace-directory-not-made = the configuration directory cannot be made: { $error }
# Why is one of the reasons for writing a file.
workspace-file-failed = the workspaces file { $why }
workspace-no-directory = no configuration directory is set to keep workspaces in
workspace-name-taken = there is a workspace { $name } already
workspace-holds-at-most = a workspace holds at most { $most } repositories
workspace-too-many-workspaces = there are { $most } workspaces already; delete one first
workspace-folder-missing = { $folder } is not a folder that is there
workspace-follows-no-folder = the workspace { $id } follows no folder
workspace-no-repository-at = the workspace has no repository at { $path }
workspace-not-a-repository-there = { $path } is not a repository that is there
workspace-no-profile = there is no profile { $name }
workspace-not-there-no-address = { $path } is not there, and there is no address to clone it from
workspace-path-not-a-folder = { $path } is not a folder
workspace-not-a-repository-folder = { $path } is not the folder of a repository

## What is wrong with a parameter, after its name.

workspace-missing = missing
workspace-at-most = at most { $most }
workspace-from-one-to = from 1 to { $most }
workspace-at-most-at-a-time = at most { $most } at a time
workspace-at-most-repositories-at-a-time = at most { $most } repositories at a time
workspace-id-or-name = either id (add to that workspace) or name (make one), not both
# The id is in quotes.
workspace-named-twice = { $id } is named twice
workspace-name-at-least-one = name at least one repository
workspace-not-one-of-its-repositories = { $path } is not one of the workspace's repositories

## A workspace as a file to share: exporting it, and importing it.

workspace-file-in-a-folder = has to be a file in a folder
workspace-is-a-folder = { $file } is a folder
workspace-file-there = { $file } is there already; say overwrite to write over it
# The error is what the writer of the file said.
workspace-one-not-written-down = the workspace cannot be written down: { $error }
# After the file's path. The format is the word the file has to say it is.
workspace-not-a-workspace-file = is not a workspace file (it does not say format = "{ $format }")
workspace-written-by-newer = was written by a newer version of the application
workspace-file-too-many-repositories = has more than { $most } repositories
workspace-folder-not-followed = { $folder } cannot be followed ({ $error })
# A repository of the file that has neither a path nor a name, by its place in the file.
workspace-entry-number = number { $number }

## Why a repository of a shared file is not imported.

workspace-place-path-unusable = its path cannot be used
workspace-place-full-path = a full path: only paths inside { $folder } are taken
workspace-place-name-unusable = its name is not a folder's name
workspace-place-neither = it has neither a path nor a name
workspace-place-leads-out = it leads out of { $folder }
workspace-place-not-followed = its path cannot be followed
workspace-place-through-link = it leads out of { $folder } through a symbolic link
workspace-path-not-text = its path is not text

## Why a folder is passed over when repositories are looked for in a folder or a VS Code workspace.

workspace-skip-not-kept = its path cannot be kept
workspace-skip-stopped = stopped before it was looked into
workspace-skip-limit = not taken: { $limit } repositories were found already
workspace-skip-too-deep = not looked into: deeper than { $depth } levels
workspace-skip-too-many-folders = not looked into: { $most } folders were looked into already
workspace-skip-link = a symbolic link, which is not followed
workspace-skip-not-local = an address that is not a folder on this machine
workspace-skip-neither = has neither a path nor a file address
workspace-skip-path-unusable = a path that cannot be used
workspace-skip-not-a-repository = not the folder of a repository
workspace-skip-not-there = not there
# A folder of a VS Code workspace, by its place in the file.
workspace-folder-number = folder number { $number }

## What is wrong with a VS Code workspace file, after its path.

workspace-code-folders-not-a-list = has "folders" that are not a list
workspace-code-no-folders = has no "folders"
workspace-code-too-many-folders = has { $count } folders, and no more than { $most } are read

## Fetching, pulling and cloning a workspace's repositories: why one was skipped or failed.

workspace-bulk-not-there = it is not there; clone it first
workspace-bulk-not-a-repository = its folder is not a repository's (any more)
workspace-bulk-no-answer = its command ended without an answer
workspace-bulk-stopped = stopped before its turn
# What a pull answered when it stopped without saying how.
workspace-bulk-stopped-half-way = it stopped half-way
# Said is how the pull stopped.
workspace-bulk-conflicts = stopped with conflicts to resolve: { $said }
workspace-bulk-there-already = it is there already
workspace-bulk-no-address = there is no address to clone it from
workspace-bulk-name-not-a-folder = its name cannot be a folder's
