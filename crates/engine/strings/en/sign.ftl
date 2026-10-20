### Signing.

## The commands: what the palette and the command line call each, and what it does.

sign-status = Signing set-up
sign-status-description = How this repository signs: format, key, whether commits and tags are signed by default, the allowed-signers file, the signing program, and whether signing can work, with the reason when not
sign-keys = Signing keys
sign-keys-description = Keys that could sign: secret keys gpg knows, public key files and keys in the SSH agent; params: ssh_dir (~/.ssh when absent), agent (bool, true when absent: ask the SSH agent too)
sign-configure = Set up signing
sign-configure-description = Write signing settings; params: format (openpgp|ssh|x509), key (empty removes it), commits (bool), tags (bool), scope (repo|user, repo when absent); what is absent stays as it is
sign-verify = Verify signatures
sign-verify-description = What git makes of each commit's signature: none, good, untrusted, expired, key_expired, key_revoked, bad, unknown_key or cannot_check, with signer, key and fingerprint, and for an X.509 certificate whose chain does not hold what is wrong with it (chain: missing_issuer, untrusted_root, revoked, revocation_unknown, other); params: ids (at most 256)
sign-signed = Signed commits
sign-signed-description = Whether each commit carries a signature and in which format, without verifying it; params: ids
sign-tag-details = Tag details
sign-tag-details-description = A tag in full: tagger, date, message, what it points at, and what its signature is worth; params: name

## Journal entries: what the undo menu and the activity log call each change.

sign-entry-configure = Change signing settings

## Refusals: what follows "invalid parameter "<name>": ".

sign-ssh-dir-not-full = it has to be the full path of a directory
# The values the format parameter takes, which are git's.
sign-format-values = openpgp, ssh or x509
# The values the scope parameter takes.
sign-scope-values = repo or user
