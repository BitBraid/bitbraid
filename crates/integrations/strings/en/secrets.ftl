### Tokens kept in the keychain.

## Why the keychain did not keep or give a token.

# The detail is why, as the system said it.
secrets-unavailable = no keychain to keep the token in: { $detail }
secrets-locked = the keychain is locked
# The detail is what the keychain said.
secrets-refused = the keychain refused: { $detail }
secrets-unreadable = what the keychain keeps for this account cannot be read
# What a token is written as wherever it would be shown.
secrets-hidden = <hidden>
# The keychain in memory that the tests use.
secrets-poisoned = the store was left half-way by a thread that failed

## Windows' Credential Manager.

secrets-colon-in-service = the name of a service has no colon in it
# The error is what Windows said.
secrets-windows-unreachable = Credential Manager cannot be reached from this logon session ({ $error })
# What is the part of a credential, as the keyring library names it; longest is the most it takes.
secrets-windows-too-long = Credential Manager keeps no { $what } longer than { $longest }
# The error is what Windows said.
secrets-windows-failed = Credential Manager failed: { $error }

## macOS's keychain, and a system this build has no keychain for.

# The error is what the keychain said.
secrets-keychain-says = the keychain says: { $error }
secrets-no-keychain = this build cannot reach the keychain of this system yet
