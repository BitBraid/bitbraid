### Connections: HTTP, proxies, TLS.

## Why a request was not answered.

# What of the address cannot be used, in a few words ("the host").
http-address = not an address that can be asked: { $what }
http-insecure = plain HTTP is spoken to this machine only
# The name is the host's name, or what the system's lookup said.
http-resolve = the name was not found: { $name }
# Why is what the system said, or what the proxy did.
http-connect = no connection: { $why }
# Why is what the TLS library said.
http-tls = no secure connection: { $why }
http-timeout = no answer in time
http-too-large = the answer is larger than allowed
# What of the answer cannot be read, in a few words ("the head").
http-malformed = the answer is not one that can be read: { $what }
# A request that whoever asked no longer wants.
http-cancelled = stopped
# Why is what the system said.
http-broken = the connection broke: { $why }
# The request is its method and address.
http-unrecorded = no answer was recorded for { $request }
# What came could not be written where it was to go: a full disk, a file
# that went away.
http-not-stored = what came could not be written: { $error }
http-no-roots = this system has no root certificates to trust
http-nothing-more-taken = nothing more is taken

## What of an address or a request cannot be used: each follows "not an address that can be asked: ".

http-address-too-long = too long
http-address-scheme = neither https nor http
http-address-user-name = a user name in it
http-address-unclosed-bracket = an unclosed bracket
http-address-port = the port
http-address-host = the host
http-address-path = the path
http-address-too-many-headers = too many header lines
http-address-header-name = a header line's name
http-address-header-value = a header line's value
# The kinds of answer that are welcome (the Accept line).
http-address-accept = what is accepted

## What of an answer cannot be read: each follows "the answer is not one that can be read: ".

http-line-too-long = a line is too long
http-body-ends-early = the body ends early
http-head = the head
http-status = the status
http-length = the length
http-transfer-coding = the transfer coding
http-content-coding = the content coding
http-head-too-long = the head is too long
http-nothing-came-back = nothing came back
http-head-ends-early = the head ends early
http-chunk-size = a chunk size
http-chunk-end = the end of a chunk
http-trailer-too-long = the trailer is too long

## A proxy's address that cannot be used: what follows "the proxy named in … cannot be used: ".

http-proxy-address-empty = the address is empty
http-proxy-address-too-long = the address is too long
http-proxy-address-space = the address holds a space or a control character
http-proxy-over-tls = a proxy spoken to over TLS (https://) is not supported
# The scheme is the one the address starts with (socks5).
http-proxy-socks = a SOCKS proxy ({ $scheme }://) is not supported
http-proxy-scheme = its scheme is not http
http-proxy-user-name = the user name in it cannot be read
http-proxy-password = the password in it cannot be read
http-proxy-bracket = a bracket is not closed
http-proxy-port = the port cannot be read
http-proxy-host = the host cannot be read

## Going through a proxy.

# Where a proxy was named when it is git's setting, not a variable: in "the proxy named in … cannot be used".
http-proxy-git-setting = git's http.proxy
# The from is where the proxy was named (HTTPS_PROXY, git's http.proxy); why is what is wrong with it.
http-proxy-unusable = the proxy named in { $from } cannot be used: { $why }
# The host is the proxy's name, which was not found.
http-proxy-not-found = { $host } (the proxy)
# The place is the proxy's host and port; why is what the system said.
http-proxy-connect = the proxy at { $place }: { $why }
http-proxy-takes-nothing-more = the proxy takes nothing more
# The schemes are the ways of signing in the proxy offers ("Negotiate (Kerberos) or NTLM").
http-proxy-wants-other-sign-in = the proxy at { $place } asks to be signed in to by { $schemes }, which the application's own connections do not do: they give a proxy a user name and a password (Basic), from its address, and nothing else
http-proxy-login-refused = the proxy at { $place } refused the user name and password its address gives
http-proxy-login-wanted = the proxy at { $place } asks for a user name and a password, and its address gives none (http://name:password@host:port)
# The target is the host and port asked for; the status is the proxy's HTTP status code.
http-proxy-unreachable = the proxy at { $place } could not reach { $target } ({ $status })
http-proxy-refused = the proxy at { $place } refused the connection to { $target } ({ $status })
http-proxy-answer-too-long = the proxy's answer is too long
http-proxy-closed = the proxy closed the connection without an answer
# What of the proxy's answer cannot be read.
http-proxy-answer = the proxy's answer
# A proxy that asks to be signed in to and names no way of doing it.
http-proxy-scheme-unnamed = a way it does not name
# The last of several ways of signing in, after the others joined by commas: "Negotiate (Kerberos), Digest or NTLM".
http-proxy-schemes-or = { $rest } or { $last }
