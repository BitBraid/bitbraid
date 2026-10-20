### Who commits: name and address.

## Which input or operation was refused: the first words of the error.

identity-what-identity = identity
identity-what-user-name = user name
identity-what-email = email address
identity-what-signing-key = signing key
identity-what-ssh-key = SSH key
identity-what-config-file = configuration file

## Why a name, an address or a key was refused.

identity-too-long = it is too long
identity-control = must not hold control characters
identity-angle-brackets = must not hold < or >
identity-empty = must not be empty
identity-spaces = must not hold spaces
identity-starts-with-dash = must not start with a dash
identity-key-not-absolute = give the full path of the key file (a ~ is not filled in)
identity-key-fill-in = ssh reads % and ${"{"} in the path of a key as something to fill in; move the key to a path without them

## Why a place for the settings was refused.

identity-no-repository = no repository is open; its configuration cannot be the place
identity-path-not-text = its path is not text
identity-path-too-long = its path is too long
identity-path-control = its path must not hold control characters
identity-full-path = give its full path
identity-not-a-file = { $path } is not a file
# The file to write to.
identity-no-directory = the directory of { $path } is not there

## Why the settings could not be read or written.

# What git said.
identity-cannot-read = the configuration cannot be read: { $said }
# What git said.
identity-cannot-write = the configuration cannot be written: { $said }
identity-no-key-file = there is no key file at { $key }; choose the key again, or use an identity without one
