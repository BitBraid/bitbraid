### Making a repository at a provider.

## The form. $site is the provider's name (GitHub, GitLab Self-Managed and
## the like), $login the account's user.

provider-create-heading = { $login } at { $site }
provider-create-make-in = Make it in
# The user's own place, first in the list of places.
provider-create-own = { $login } (your own)
# $problem is what the provider said.
provider-create-namespaces-unread = Only your own: the other places could not be read ({ $problem })
provider-create-name = Name
provider-create-name-placeholder = the repository's name
provider-create-description = Description
provider-create-description-placeholder = what it is (may be empty)
provider-create-visibility = Visibility
provider-create-private = Private
provider-create-public = Public
provider-create-in-folder = In folder
provider-create-folder-placeholder = the folder it is made in, as a full path
provider-create-full-path = Full path
provider-create-create = Create

## What would be made, or why nothing can be yet.

# $path is the new folder, $namespace/$name the repository at the provider.
provider-create-said = Made in { $path } with a first commit, then at { $site } as { $namespace }/{ $name }, which becomes its remote origin; the first commit is pushed there.
provider-create-name-empty = Name the repository.
provider-create-name-long = The name is longer than a provider takes.
provider-create-name-dot = The name may not begin with a dot or end in .git.
provider-create-name-letters = The name is letters, digits, dashes, underscores and dots.
provider-create-folder-not-full = The folder it is made in has to be a full path.
provider-create-taken = There is something at { $path } already.
