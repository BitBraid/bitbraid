### Profiles.

## The commands: what the palette and the command line call each, and what it does.

profile-list = List profiles
profile-list-description = The identities that are kept, each with its name, user_name, user_email, signing_key, signing_format and ssh_key
profile-save = Save profile
profile-save-description = Keep an identity under a name, in place of the one of that name when there is one; params: profile ({"{"}name, user_name, user_email, signing_key, signing_format: openpgp, ssh or x509, ssh_key: the full path of a private key file{"}"}), rename_from (the name it had until now: the workspaces that name it as their default profile are given the new name; how many in workspaces, or why not in workspaces_error)
profile-delete = Delete profile
profile-delete-description = Forget a profile; no configuration is changed, and the workspaces that name it as their default profile name none from then on (how many in workspaces, or why not in workspaces_error); params: name
profile-current = Current profile
profile-current-description = The identity in effect in the open repository, or the user's when none is open, and the profile that has its name and address; params: scope (repo, user or {"{"}file: path{"}"}: what that one configuration says)
profile-use = Use profile
profile-use-description = Write a profile's identity to a configuration: name, address, signing key and format, and its SSH key as core.sshCommand; params: name, scope (repo: this repository's, user: the user's, {"{"}file: path{"}"}: a configuration file that is named)

## Journal entries: what the undo menu and the activity log call each change.

profile-entry-use = Use profile { $name }

## What is wrong with the name of a profile, after what it is the name of.

profile-name-empty = must not be empty
profile-name-too-long = is longer than { $max } characters
profile-name-control = must not hold control characters
# Why is one of the three above.
profile-its-name = its name { $why }

## Refusals.

# The operation a refusal names, before a colon.
profile-refused = profile
profile-no-directory = no configuration directory is set to keep profiles in
# A profile's name is in quotes.
profile-no-such-to-rename = there is no profile { $name } to rename
profile-exists = there is a profile { $name } already
profile-too-many = there are { $max } profiles already; delete one first
profile-no-such = there is no profile { $name }

## The file of profiles cannot be used: the path of the file, then one of the reasons below.

profile-file-unusable = { $path } { $why }; it is left as it is until it is put right or moved away
# The directory is the one the file is said to be in, which is a file.
profile-file-parent-not-directory = cannot be read ({ $directory } is not a directory)
# The error is what the system said.
profile-file-unreadable = cannot be read ({ $error })
profile-file-not-file = is not a file
profile-file-too-large = is larger than { $max } KiB
profile-file-not-text = is not text
# Said is what the TOML reader said, cut short when long; the line is where in the file.
profile-file-not-understood-at = cannot be understood (line { $line }: { $said })
profile-file-not-understood = cannot be understood ({ $said })
profile-file-too-many = has { $count } profiles, and no more than { $max } are kept
# The number counts the file's profiles from 1; why is what is wrong with it.
profile-file-bad-profile = has a profile that cannot be used (number { $number }: { $why })
profile-file-same-name = has two profiles of one name (the second is number { $number })

## Writing the file of profiles failed; the error is what the system said.

profile-not-written-down = the profiles cannot be written down: { $error }
profile-directory-not-made = the configuration directory cannot be made: { $error }
profile-not-written = the profiles cannot be written: { $error }
profile-not-placed = the profiles cannot be put in their place: { $error }
