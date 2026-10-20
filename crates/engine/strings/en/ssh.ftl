### SSH keys.

## The commands: what the palette and the command line call each, and what it does.

ssh-key-generate = Generate an SSH key
ssh-key-generate-description = Make a new Ed25519 SSH key without a passphrase with ssh-keygen, never over a file that is there; answers the private and public key files, the public key and its fingerprint; params: path (the private key's full path; the public key goes beside it with .pub added), comment (written into the key; ssh-keygen's user@host when absent)
ssh-key-fingerprint = SSH key fingerprint
ssh-key-fingerprint-description = The fingerprint of an SSH key as ssh-keygen gives it, its kind and comment, and the public key file beside it when there is one; params: path (a private or a public key file, full path)

## What is wrong with a parameter, after its name.

ssh-path-empty = is empty
ssh-path-not-a-path = is not a path
ssh-path-not-full = has to be a full path
ssh-path-goes-up = may not go up a folder with ..
ssh-path-no-file = names no file
ssh-comment-not-a-line = is not one line of text

## Refusals.

# The operations a refusal names, before a colon.
ssh-fingerprint-refused = the key's fingerprint
ssh-generate-refused = a new SSH key
# After "ssh-keygen: "; the error is what the system said.
ssh-keygen-not-started = it could not be started ({ $error }); is OpenSSH installed?
ssh-keygen-cancelled = cancelled
ssh-keygen-too-slow = it did not finish within a minute
ssh-not-a-key = { $path } is not a key ssh-keygen reads
ssh-keygen-no-fingerprint = ssh-keygen gave none
ssh-folder-not-made = the folder { $folder } could not be made: { $error }
ssh-file-there = there is a file at { $path } already, and it is not written over
ssh-keygen-no-key = ssh-keygen made no key
ssh-no-file = there is no file at { $path }
