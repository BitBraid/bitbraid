### Allowed signers.

## The commands: what the palette and the command line call each, and what it does.

signers-list = Allowed signers
signers-list-description = Who may sign with an SSH key: the allowed-signers file (the setting, the file it names, whose configuration says so, whether it is there) and every signer in it with principals, options, kind of key and fingerprint
signers-file = Choose the allowed-signers file
signers-file-description = Name the allowed-signers file in the configuration and make it when it is not there; params: path (the usual place of the scope when absent: beside the repository's configuration, or git's directory in the user's), scope (repo, user or {"{"}file: path{"}"}; repo when absent)
signers-allow = Allow a signer
signers-allow-description = Add a signer to the allowed-signers file, for git's own signatures; params: principals (who: e-mail addresses; the committer of `commit` when absent), and one of key (a public key as a .pub file has it), key_file (the full path of such a file) or commit (the key that commit was signed with)
signers-disallow = Remove an allowed signer
signers-disallow-description = Take a signer out of the allowed-signers file; params: key (the fingerprint of the key, or the key itself), principals (only the line for exactly these; every line with the key when absent)

## Journal entries: what the undo menu and the activity log call each change.

signers-entry-file = Choose the allowed-signers file
# Who is the signer's principals, joined by commas.
signers-entry-allow = Allow signer { $who }
# Who is the principals of the first line taken out, or the key when it has none.
signers-entry-remove = Remove signer { $who }

## Refusals: what follows "invalid parameter "path": " or "invalid parameter "key": ".

signers-path-not-text = the repository's path is not text; name the file
signers-no-home = there is no home directory; name the file
signers-say-where = say where the file is to be when the configuration is a named file
signers-which-key = say which key: give key, key_file or commit
signers-one-key = give one of key, key_file and commit, not several
