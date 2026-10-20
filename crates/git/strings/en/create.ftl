### Making a repository.

## Which operation or input was refused: the first words of the error.

create-what-init = init
create-what-clone = clone
create-what-clone-url = clone URL
create-what-branch-name = branch name

## Why a repository was not made or cloned.

create-path-not-absolute = the path has to be absolute
create-path-missing = no part of the path exists
create-inside-git-dir = that is a repository's own directory, or inside one; choose a place outside it
# The name, quoted.
create-not-valid-branch = { $name } is not a valid branch name
create-file-at-path = there is a file at that path
create-already-repository = that directory is a repository already
create-gitignore-exists = there is a .gitignore in that directory already; it would be overwritten
# The steps of making one; each is followed by what failed.
create-creating-directory = creating the directory
create-writing-gitignore = writing .gitignore
# What git said when the first commit failed. The interface looks for the
# words "the repository was created" in it: keep them.
create-first-commit-failed = the repository was created, but not its first commit: { $err }
# The address, quoted.
create-url-unusable = { $url } cannot be used
create-not-empty = { $path } exists and is not an empty directory
create-destination-not-utf8 = the destination is not valid UTF-8
