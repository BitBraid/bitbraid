### SSH keys at a provider.

## Why a public key's file is refused before it is sent.

keys-private = this is a private key: give the public one (.pub)
keys-too-long = a key's line is at most { $most } bytes
keys-not-one-line = a public key is one line
# The kind is in quotes, as the file has it; the kinds are those the providers take, joined by commas.
keys-kind-not-taken = { $kind } is not a kind of key the providers take ({ $kinds })
keys-not-base64 = the key is not in base64
# The kind is what the line says, such as ssh-ed25519.
keys-kind-differs = the key does not say it is { $kind }
