### Providers.

## Parameters: said of one taken by several commands

# account: provider.repos, provider.namespaces, provider.repo_create, provider.ssh_key_add
provider-param-account = the account's id

## Parameters of provider.repos

provider-param-repos-filter = which: the user's own, of their organisations or groups, starred, or all; all when absent
provider-param-repos-query = words the repository's name has
provider-param-repos-page = the page, from 1; 1 when absent

## Parameters of provider.repo_create

provider-param-repo-create-namespace = the organisation or group path; the user's own when absent
provider-param-repo-create-name = the new repository's name
provider-param-repo-create-description = what it is
provider-param-repo-create-private = whether only some may see it
provider-param-repo-create-remote = add it to the open repository as a remote of this name
provider-param-repo-create-push = push the current branch to it
provider-param-repo-create-protocol = which address the remote has; https when absent

## Parameters of provider.fork

provider-param-fork-remote = the remote whose repository is forked; the first linked one when absent
provider-param-fork-namespace = the organisation or group to fork into; the user's own when absent

## Parameters of provider.ssh_key_add

provider-param-ssh-key-add-path = the public key's file (a .pub)
provider-param-ssh-key-add-title = what it is called at the provider

## Parameters of provider.avatars

provider-param-avatars-emails = the e-mail addresses, at most 100

## The commands: what the palette and the command line call each, and what it does.

provider-links = Linked repositories
provider-links-description = Which repository at which connected account each remote of the open repository is: remote, account, kind, owner and name, origin first
provider-repos = Repositories at a provider
provider-repos-description = The repositories an account may clone; params: account, filter (own|organisation|starred|all), query, page
provider-namespaces = Where a repository may be made
provider-namespaces-description = The user, organisations or groups an account may make a repository in; params: account
provider-repo-create = Create a repository at a provider
provider-repo-create-description = Make a repository at an account, and add it as a remote of the open repository and push the current branch to it when asked; answers the repository, the remote and the branch pushed; params: account, namespace (an organisation or group; the user's own when absent), name, description, private (bool), remote (a name for the remote), push (bool), protocol (https|ssh)
provider-fork = Fork
provider-fork-description = Fork a remote's repository into the user's account or an organisation or group, wait until the provider has made it, and add the fork as a remote named after its owner (a remote that is the fork already is kept); answers the fork and the remote; params: remote (the first linked one when absent), namespace
provider-ssh-key-add = Add an SSH key to a provider
provider-ssh-key-add-description = Upload a public key to an account (a private key, or a kind the providers do not take, is refused before anything is sent): answers its id, title and SHA256 fingerprint; params: account, path (a .pub file), title
provider-avatars = Pictures from a provider
provider-avatars-description = The provider's pictures of the people with these e-mail addresses: an object of address to picture address, for the addresses the provider knows; at GitHub, learnt from the commits of the repository (owner and name, or the open repository's link); params: account, owner, name, emails (at most 100)

## Refusals of the commands: what follows the name of the parameter refused.

provider-missing = missing
provider-not-a-name = is not a name a provider allows
provider-push-without-remote = a push goes to the remote it is given: name one
provider-no-branch = no branch is checked out
provider-not-linked = is not a remote at a connected account
provider-not-a-key-path = is the full path of a public key's file (.pub)
provider-not-a-key-file = is not a public key's file
# The value is in quotes.
provider-not-an-address = has { $value }, which is not an address

## A repository made at a provider, and what could not be done after.

# The repository is owner/name at the provider.
provider-made-elsewhere = { $repository } was made, but its address is not at the account's site, and was not added
# The repository is owner/name at the provider; what is one of the three messages below; the error is why.
provider-made-but = { $repository } was made, but { $what }: { $error }
provider-remote-not-added = the remote could not be added
provider-nothing-pushed = nothing was pushed
provider-push-failed = the push failed
# The operation a refusal names, before a colon.
provider-refused = provider
