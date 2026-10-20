### Partial clones.

## Why a read in a partial clone did not fetch what it missed, in work
## nobody asked for.

# Which read was refused: the first words of the error.
promisor-what-not-on-this-machine = not on this machine
# The words the interface looks for to offer to fetch
# (promisor-left-on-remote), and why the read failed.
promisor-fetched-only-when-asked = { $left }, and it is fetched only when asked for ({ $error })
# What is said of a read that failed in a partial clone while it may read
# only what is here. The interface looks for these words to offer to fetch.
promisor-left-on-remote = the partial clone left it on the remote
