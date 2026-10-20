### Signatures and verification.

## Which input or operation was refused: the first words of the error.

signature-what-signing-key = signing key
signature-what-signing-settings = signing settings
# The commits asked about, by their ids.
signature-what-ids = ids
signature-what-tag = tag

## The steps of reading a tag; each is followed by what failed.

signature-loading-tag = loading a tag
signature-loading-tag-target = loading what a tag points at

## Why signing cannot work as the repository is set up. A program is
## named as it is set; a format is openpgp, ssh or x509.

# The value of gpg.format, quoted.
signature-format-unknown = gpg.format is set to { $value }; git knows openpgp, ssh and x509
signature-signer-not-found = { $program }, which signs in the { $format } format, was not found; install it, or set gpg.{ $format }.program to where it is
signature-no-key-no-name = no signing key is set (user.signingkey), and there is no name and email to look one up by
signature-program-silent = { $program } could not be started, or did not answer
# The id of the key, as gpg lists it.
signature-key-unusable = the key { $key } cannot sign: it has expired, was revoked or is disabled; renew it or choose another
# $who: the signing key that is set, quoted.
signature-no-secret-key-for-set = { $program } has no secret key for { $who }, the signing key that is set (user.signingkey); choose one it has
# $who: the committer's name and email, quoted.
signature-no-key-no-secret-key = no signing key is set (user.signingkey), and { $program } has no secret key for { $who }
signature-no-key = no signing key is set (user.signingkey)
signature-key-not-public = user.signingkey does not hold a public key
signature-agent-lacks-literal-key = the signing key is given as a public key, and the SSH agent does not have its private half (or is not running); add it with ssh-add, or set user.signingkey to the key file
signature-key-file-missing = the key file { $path } (user.signingkey) does not exist
signature-no-private-key = there is no private key for { $path }: none beside it and none in the SSH agent

## Why signing settings were not written.

signature-not-a-key = it is not a key id, a key file or a public key
signature-nothing-to-change = nothing to change: give format, key, commits or tags
# The setting, and what git said.
signature-setting-not-removed = { $key } could not be removed: { $said }

## What is said of a signature that could not be checked.

signature-too-many-commits = at most { $most } commits at a time
signature-too-many-to-verify = at most { $most } commits can be verified at a time
# What git said, in a line.
signature-git-could-not-check = git could not check this signature: { $said }
signature-git-could-not-check-it = git could not check this signature
# Git gave up and wrote nothing about why.
signature-no-reason = it gave no reason
# The letter git graded the signature with, quoted.
signature-unknown-grade = git graded it { $grade }, which is not a grade known here
signature-form-unknown = the signature is not in a form git knows (OpenPGP, SSH, X.509)
signature-no-allowed-signers = no allowed-signers file is set (gpg.ssh.allowedSignersFile), and git checks SSH signatures against one
signature-checker-not-found = { $program }, which checks signatures in the { $format } format, was not found
signature-not-accepted = git did not accept this signature
signature-more-than-one = it carries more than one signature

## Why a tag could not be read.

# What the reader of tag objects said.
signature-tag-unreadable = the tag object cannot be read: { $error }
signature-no-tag = there is no tag { $name }
signature-tags-too-deep = { $name } is a tag of tags more than { $most } deep

## Why signing failed, and what to do about it. $why is gpg's own reason.

# Goes right after "secret key" in the three below: the key gpg named, quoted.
signature-for-whom = {" "}for { $name }
# $whom: the words above, or nothing.
signature-secret-key-unusable = the secret key{ $whom } cannot be used: it has expired, was revoked or is disabled; renew it, or choose another (user.signingkey)
signature-no-secret-key-set = there is no secret key{ $whom }, the signing key that is set (user.signingkey); choose a key that is here, or leave signing off
signature-no-secret-key-unset = no signing key is set, and there is no secret key{ $whom }; choose a signing key (user.signingkey), or leave signing off
signature-card-pin-refused = the smart card did not take the PIN ({ $why }); a card blocks its PIN after a few wrong tries in a row, and its Admin PIN unblocks it
signature-card-pin-blocked = the PIN of the smart card is blocked ({ $why }), and the card will not sign; unblock it with the Admin PIN or the reset code (gpg --card-edit, then admin and passwd) and sign again
signature-card-no-daemon = the signing key is on a smart card, and gpg-agent could not start scdaemon, the part of GnuPG that talks to cards; install it (some systems package it on its own, as scdaemon) and sign again
signature-card-not-in-reader = the smart card that holds the signing key is not in a reader ({ $why }); put it in and sign again
# On macOS, right after "a pinentry that opens a window" below.
signature-pinentry-for-macos = {" "}such as pinentry-mac
# $which: the words above on macOS, or nothing.
signature-no-passphrase = gpg could not get the passphrase of the signing key ({ $why }); give gpg-agent a pinentry that opens a window{ $which } (pinentry-program in gpg-agent.conf), or unlock the key in a terminal first
signature-no-gpg-agent = gpg could not reach its agent, which holds the secret keys; start it (gpgconf --launch gpg-agent) and sign again
signature-no-ssh-key = no SSH signing key is set; choose one (user.signingkey), or leave signing off
signature-signer-not-started = the signing program { $program } could not be started; install it, or correct where gpg.program (gpg.ssh.program, gpg.x509.program) points
signature-ssh-key-unreadable = the SSH signing key { $path } could not be read: the file is missing or is not a key; choose a key that exists (user.signingkey)
signature-no-private-key-anywhere = there is no private key for { $path }: none beside it and none in the SSH agent; add it to the agent (ssh-add), or choose another key
signature-no-ssh-agent = the signing key is given as a public key, so its private half has to be in the SSH agent, and no agent is running; start one and add the key (ssh-add), or set user.signingkey to the key file
signature-agent-lacks-key = the SSH agent does not have the signing key; add it (ssh-add), or set user.signingkey to the key file
signature-ssh-passphrase = the SSH signing key needs its passphrase and nothing here could ask for it; add the key to the SSH agent (ssh-add) and sign again
# The last thing the signing program said.
signature-signing-failed = signing failed ({ $word }); check the signing settings, or leave signing off
signature-signing-failed-silently = signing failed and the signing program did not say why; check the signing settings, or leave signing off

## What is the matter with the smart card that holds the signing key.
## $card is the card's number as the PIN question shows it; $why is gpg's
## own reason.

# On Windows, right after "smart card service" below; elsewhere the service's own name is written there.
signature-card-service-windows = {" "}(the Smart Card service)
# $service: the words above on Windows, " (pcscd)" on Linux, nothing on macOS.
signature-card-no-service = the signing key is on the smart card { $card }, and no card reader can be reached: the system's smart card service{ $service } is not running; start it, and sign again with the card in its reader
signature-card-absent = the signing key is on the smart card { $card }, which is not in a reader here; put it in and sign again
# $inserted: the number of the card that is in the reader.
signature-card-another = the smart card in the reader ({ $inserted }) is not the one that holds the signing key ({ $card }); put that one in and sign again
signature-card-key-gone = the smart card { $card } is in the reader, and no longer holds the signing key: its keys were replaced or it was reset; choose the key it holds now (user.signingkey), or put the signing key back on it
signature-card-blocked = the PIN of the smart card { $card } is blocked after too many wrong tries, and the card will not sign; unblock it with the Admin PIN or the reset code (gpg --card-edit, then admin and passwd) and sign again
# Goes at the end of the message below: how many tries the card has left.
signature-card-tries-left =
    { $tries ->
        [one] ; one more wrong PIN blocks it
       *[other] ; { $tries } more wrong PINs block it
    }
# $left: the words above, or nothing when the card does not say.
signature-card-wrong-pin = the smart card { $card } did not take the PIN ({ $why }){ $left }
signature-card-pin-not-given = the PIN of the smart card { $card }, which holds the signing key, was not given ({ $why }): the question was cancelled, or gpg-agent has no pinentry that can open a window here (pinentry-program in gpg-agent.conf)
signature-card-could-not-sign = the smart card { $card }, which holds the signing key, could not sign ({ $why }); take it out of the reader, put it in again, and sign again
signature-named-card-pin-refused = the smart card { $card } did not take the PIN ({ $why }); a card blocks its PIN after a few wrong tries in a row, and its Admin PIN unblocks it
signature-card-unusable-unanswered = the signing key is on the smart card { $card }, and gpg could not use it ({ $why }): the card is not in a reader, or its PIN was not given; put it in, type the PIN when asked, and sign again
signature-card-unusable = the signing key is on the smart card { $card }, and gpg could not use it ({ $why }); see that the card is in its reader, and sign again
