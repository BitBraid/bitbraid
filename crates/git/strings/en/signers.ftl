### Allowed signers.

## Which input or operation was refused: the first words of the error.

signers-what-allowed-signers = allowed signers
signers-what-public-key = public key
signers-what-signature = signature
signers-what-commit = commit
signers-what-signer = signer
signers-what-key-file = key file
signers-what-key = key
signers-what-file = allowed-signers file

## Why a key, a signature or a signer cannot be used.

signers-private-key = that is a private key; give the public half, the file that ends in .pub
signers-no-public-key = there is no SSH public key in it (a line like "ssh-ed25519 AAAA…"); a certificate cannot be a signer by itself
signers-not-ssh-signature = it is not an SSH signature; only the key of one can be allowed here
signers-signature-unreadable = the SSH signature cannot be read
signers-signature-key-unknown = it was made with a certificate or a kind of key that is not known here; allow the key from its file instead
# The commit's full id.
signers-not-a-commit = { $id } is not a commit here
signers-too-long = it is too long to be one
# The commit's short id.
signers-not-signed = { $id } is not signed
signers-say-who = say who it is: usually their e-mail address
signers-name-too-long = the name is too long
# The name, quoted.
signers-principal-unwritable = { $principal } cannot be written as a signer: no blanks, quotes or commas
signers-key-file-full-path = give the full path of a public key file
signers-too-many-names = too many names for one key
signers-give-fingerprint = give the fingerprint of the signer's key, as the list shows it

## The steps of reading and writing the file; each is followed by what
## failed.

signers-looking-at-file = looking at the allowed-signers file
signers-reading-file = reading the allowed-signers file
signers-writing-file = writing the allowed-signers file
signers-removing-file = removing the allowed-signers file
signers-reading-key-file = reading the key file

## Why the file was not read, changed or chosen. A path is the file's.

signers-not-a-file = { $path } is not a file
# The largest file that is read, in MiB.
signers-file-too-large = { $path } is larger than { $mib } MiB; it is left alone
signers-not-text = { $path } is not text; it is left alone
signers-cannot-write = { $path } cannot be written
# The file, as the action that is undone or made again wrote its path.
signers-not-as-left = { $file } is not as this action left it; nothing was changed
# The setting that names the file.
signers-no-file-choose = no allowed-signers file is set ({ $setting }); choose one first
# The setting that names the file.
signers-no-file = no allowed-signers file is set ({ $setting })
signers-not-there = { $path } is not there
signers-no-such-signer = no signer with that key is in { $path }
signers-say-where = say where it is to be
# The path given, quoted.
signers-bad-path = { $path } cannot be its path
# What git wrote when it refused to write the setting.
signers-setting-not-written = the setting could not be written: { $stderr }
signers-no-worktree = this repository has no worktree a relative path could start from; give the full path
# Why the file could not be made.
signers-cannot-make = { $path } cannot be made: { $error }
