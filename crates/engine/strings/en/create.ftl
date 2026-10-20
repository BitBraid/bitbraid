### Making a repository.

## The commands: what the palette and the command line call each, and what it does.

create-repo-init = New repository
create-repo-init-description = Create a repository; params: path (absolute), branch (name of the first branch), first_commit (bool: a first commit, empty or with the .gitignore), gitignore (names of templates the .gitignore starts from)
create-repo-ignore-templates = .gitignore templates
create-repo-ignore-templates-description = The .gitignore templates a new repository can start from: name and what for; params: names (the file these would make, as text)
create-repo-clone = Clone repository
create-repo-clone-description = Clone a repository, and have git write its commit-graph afterwards (answers commit_graph: whether it did; not for a shallow clone); params: url, path (absolute; absent or an empty directory), branch, depth, filter (blob:none | tree:0: a partial clone, which fetches what it left out when something needs it; answers partial: whether the remote made one), sparse (bool: check out only the files at the top), recurse_submodules (bool), allow_local (bool: let submodules be cloned from local paths)
