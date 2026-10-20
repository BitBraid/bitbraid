### Signatures and who signed.

## What a signature that was checked is said to be. $signer is who signed, as the key or the allowed-signers file names them.

signatures-good = Good signature
signatures-good-by = Good signature by { $signer }
signatures-untrusted = Signed, with a key that is not trusted here
signatures-untrusted-by = Signed, with a key that is not trusted here, by { $signer }
signatures-untrusted-certificate = Signed, with a certificate that is not trusted here
signatures-untrusted-certificate-by = Signed, with a certificate that is not trusted here, by { $signer }
signatures-expired = Signed; the signature has expired
signatures-expired-by = Signed; the signature has expired. It was made by { $signer }
signatures-key-expired = Signed, with a key that has expired since
signatures-key-expired-by = Signed, with a key that has expired since, by { $signer }
signatures-key-expired-certificate = Signed, with a certificate that has expired since
signatures-key-expired-certificate-by = Signed, with a certificate that has expired since, by { $signer }
signatures-key-revoked = Signed with a key that was revoked
signatures-key-revoked-by = Signed with a key that was revoked, by { $signer }
signatures-key-revoked-certificate = Signed with a certificate that was revoked
signatures-key-revoked-certificate-by = Signed with a certificate that was revoked, by { $signer }
signatures-bad = The signature does not match what it signs
signatures-bad-by = The signature does not match what it signs. It claims to be by { $signer }
signatures-unknown-key = Signed with a key that is not known here
signatures-unknown-key-certificate = Signed with a certificate that is not known here
signatures-cannot-check = Signed; it cannot be checked here
# $reason is what git said.
signatures-cannot-check-why = Signed; it cannot be checked here: { $reason }

## A certificate whose signature holds and whose chain does not. $said is one of the signatures-chain-* below.

signatures-signed-by-but = Signed by { $signer }, but { $said }
signatures-signed-but = Signed, but { $said }
signatures-chain-missing-issuer = the certificate's chain is not complete here: a certificate that issued it is missing
signatures-chain-untrusted-root = the certificate leads to a root certificate that is not trusted here
signatures-chain-revoked = a certificate of its chain was revoked
signatures-chain-revocation-unknown = whether the certificate was revoked could not be checked
# $reason is what gpgsm said.
signatures-chain-revocation-unknown-why = whether the certificate was revoked could not be checked ({ $reason })
signatures-chain-other = the certificate's chain does not hold here
signatures-chain-other-why = the certificate's chain does not hold here ({ $reason })

## A signature nobody has checked yet.

signatures-unchecked = Signed; not checked yet
signatures-unchecked-ssh = Signed with an SSH key; not checked yet
signatures-unchecked-certificate = Signed with a certificate; not checked yet
signatures-unchecked-gpg = Signed with a GPG key; not checked yet
