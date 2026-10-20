### Remotes, and what a server said.

## Which input or operation was refused: the first words of the error.

remote-what-remote-name = remote name
remote-what-remote-url = remote URL
remote-what-reference = reference
remote-what-remote = remote
remote-what-fetch = fetch
remote-what-pull = pull
remote-what-push = push
remote-what-delete-on-the-remote = delete on the remote
remote-what-put-the-branch-back = put the branch back

## Why it was refused. A name or text in an argument is quoted.

remote-cannot-be-used = { $text } cannot be used
remote-not-valid-name = { $name } is not a valid name
remote-not-a-reference-to-fetch = { $reference } is not a reference to fetch
remote-not-a-reference-name = { $name } is not a reference name
# The reference fetched, and the remote it was fetched from.
remote-brought-no-commit = { $reference } of { $remote } brought no commit
remote-head-not-on-branch = HEAD is not on a branch
remote-no-branch = there is no branch { $branch }
remote-no-remote = this repository has no remote
remote-several-remotes = there are several remotes and none is called origin; say which one
# The remote's branch is written remote/branch.
remote-branch-not-known = { $remote }/{ $branch } is not known here; fetch { $remote } first
remote-branch-moved = { $remote }/{ $branch } is not where it was last fetched: somebody pushed to it. Fetch { $remote } and look again
remote-branch-there-again = { $remote } has a branch { $branch } again; nothing was changed
# A remote-tracking reference that is there again, by its full name.
remote-reference-there-again = { $name } exists again; nothing was changed
# A remote-tracking reference by its full name (or the id of the object, when no reference had it).
remote-object-gone = what { $name } pointed at is gone; nothing was changed

## A fetch on a timer, which nobody is there to answer for.

# Who could not be fetched: the remote, the names of the remotes, or "a remote".
remote-who-a-remote = a remote
# What is left out after the remote's name; the window looks for these words in the error.
remote-wants-an-answer = wants an answer (a user name, a password or a passphrase)
# $who: the remote, or the remotes, or "a remote"; $wants: the words above.
remote-nobody-was-asked = { $who } { $wants }, and nobody was asked

## What a refusal by a server, or by a proxy on the way, means. Each goes
## before what git, ssh or git-lfs said.

remote-host-key-refused = The key of the host was not accepted, so nothing was sent to it. A host seen for the first time is accepted by answering yes when asked; one whose key has changed has to be taken out of the known hosts file first, once it is certain that it is the right host.
remote-key-refused = The server did not accept the key. Either it knows none of the keys that were offered for this user, or the key could not be offered because the passphrase given for it was not the right one.
remote-credentials-refused = The server refused the user name and the password or token that were given. Where a credential helper had kept them, it was told to forget them, so the next attempt asks again.
remote-user-name-unanswered = The question for the user name was not answered, so there was nobody to sign in as.
remote-password-unanswered = The question for the password or token was not answered.
remote-file-locked = The server refused the push: it changes a file that somebody else has locked with LFS (named below). It can be pushed once the lock is given up, by whoever holds it or by force by someone the server lets do that.
remote-not-allowed = The server did not let the account that signed in do this: that account, or the token it signed in with, does not have the right to it here. Pushing, and taking or giving up a lock, take the right to write to the repository, and a token can have less than its account.
remote-certificate-untrusted = The server's certificate was not accepted, so nothing was sent to it: no authority this system trusts has signed it. If it is the right server, name the certificate of the authority that signed it in the repository's settings (http.sslCAInfo, or http.<address>.sslCAInfo for that server alone).
remote-certificate-expired = The server's certificate was not accepted, so nothing was sent to it: it has expired, or is not valid yet (or the clock of this machine is wrong).
remote-certificate-name = The server's certificate was not accepted, so nothing was sent to it: it was made out for another name than the address the server was reached at.
remote-proxy-login = The proxy refused the request, so nothing was sent to the server: it wants to know who is asking, and was not told in a way it accepts. For a user name and a password, git asks for the proxy's password only when the proxy's address names the user (http://name@host:port); where it did, the password given was refused, and a credential helper that had kept it was told to forget it. For Kerberos, git has to be told to sign in to the proxy with a ticket (http.proxyAuthMethod set to negotiate).
remote-proxy-ticket = The proxy refused the request, so nothing was sent to the server: git was told to sign in to it with a Kerberos ticket (http.proxyAuthMethod is negotiate) and had none that the proxy accepts. Sign in to Kerberos (kinit) and try again. The proxy has to be named by the host name its service has in the realm (HTTP/name), not by its address.
remote-proxy-ntlm = The proxy refused the request, so nothing was sent to the server: git was told to sign in to it with NTLM (http.proxyAuthMethod is ntlm), and the proxy did not accept what it was given. NTLM takes the user the proxy's address names, with the domain (http://DOMAIN%5Cname@host:port), and that user's password, asked for here; and a git whose curl has NTLM, which builds of curl may leave out (curl -V lists it among its features when it has it).
remote-proxy-tunnel = The proxy did not open a connection to the server, so nothing was sent to it: the proxy refused, or could not reach the server.
remote-proxy-absent = Nothing answered at the proxy's address, so nothing was sent to the server. The proxy is named in the settings (http.proxy, or http.<address>.proxy for one server alone) or by the https_proxy, http_proxy or all_proxy variable.
remote-proxy-name = The proxy's name was not found, so nothing was sent to the server. The proxy is named in the settings (http.proxy, or http.<address>.proxy for one server alone) or by the https_proxy, http_proxy or all_proxy variable.
