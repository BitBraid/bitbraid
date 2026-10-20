### Signing commits and tags.

## How this repository signs.

signing-row-format = Format
# The formats git signs in, as the list names them.
signing-format-openpgp = OpenPGP
signing-format-ssh = SSH
signing-format-x509 = X.509
# $label is one of the formats above.
signing-format-chosen = This repository signs in the { $label } format now
signing-row-program = Program
# Where no program or no file is named.
signing-none = none
signing-program-under = What git signs with in this format, as git's settings say.
signing-row-key = Signing key
# The list of keys, when there is no key to show in it.
signing-key-none = None
signing-reload = Look for keys again
signing-key-under = Written to this repository's own settings. Every key that could sign is listed at the end, with a way to sign with it in every repository.
signing-can-sign = Signing can work with what is set up.
signing-cannot-sign = Signing cannot work yet.
# $reason is what the engine said.
signing-cannot-sign-why = Signing cannot work yet: { $reason }
signing-row-sign-commits = Sign commits by default
signing-row-sign-tags = Sign tags by default
signing-sign-commits = Sign commits
signing-sign-tags = Sign tags
signing-commits-signed = Commits are signed here unless said otherwise
signing-commits-not-signed = Commits are not signed here unless asked for
signing-tags-signed = Tags are signed here unless said otherwise
signing-tags-not-signed = Tags are not signed here unless asked for

## Who may sign with an SSH key.

signing-group-signers = Who may sign with an SSH key
signing-row-signers-file = Allowed signers file
signing-signers-file-browse = Browse
signing-signers-no-file = No file is chosen: git cannot check a signature made with an SSH key, whoever made it.
# Where the allowed signers are kept: $path is the file; $whose says whose settings name it (repo, user, other, or none when that is not known).
signing-signers-place =
    { $whose ->
        [repo] { $path } (this repository's settings name it)
        [user] { $path } (your own git settings name it, for every repository)
        [other] { $path } (a setting outside this repository names it)
       *[none] { $path }
    }
signing-signers-place-missing =
    { $whose ->
        [repo] { $path } (this repository's settings name it); it is not there yet, and is made with the first signer
        [user] { $path } (your own git settings name it, for every repository); it is not there yet, and is made with the first signer
        [other] { $path } (a setting outside this repository names it); it is not there yet, and is made with the first signer
       *[none] { $path }; it is not there yet, and is made with the first signer
    }
signing-row-signers = Allowed signers
signing-signer-add = Add a signer…
signing-signers-under = Whose SSH signatures git trusts here: a commit that is signed with a key of theirs is good, any other untrusted.
signing-signers-nobody = Nobody is in it yet. A commit that is signed with an SSH key offers to allow that key, in its details.
signing-signers-skipped =
    { $skipped ->
        [one] { $skipped } line of the file is not a signer and is left as it is.
       *[other] { $skipped } lines of the file are not signers and are left as they are.
    }
signing-signer-remove = Remove

## Keys that could sign.

signing-group-keys = Keys that could sign
signing-keys-looking = Looking for keys…
signing-keys-none = No keys were found: gpg has no secret key and there is no SSH key in ~/.ssh or in the agent.
signing-key-in-use = In use
signing-key-use-here = Use here
signing-key-use-everywhere = Use everywhere
# Beside a key that cannot sign.
signing-key-expired = expired
signing-key-unusable = cannot sign: revoked or disabled
signing-key-chosen-here = This repository signs with that key now
signing-key-chosen-everywhere = Every repository of yours signs with that key now
# Why a key cannot be chosen.
signing-refused-unusable = That key cannot sign
signing-refused-in-use = That key is the one in use here
signing-refused-read-only = This repository is opened read-only: its own settings stay as they are
