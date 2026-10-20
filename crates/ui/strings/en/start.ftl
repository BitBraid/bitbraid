### The start page.

start-heading = Repositories

## The three ways to begin.

start-open = Open
start-open-tooltip = Open a repository that is on this machine
start-clone = Clone
start-clone-tooltip = Bring a repository here from somewhere else
start-create = Create
start-create-tooltip = Start a repository in a folder

## The recent repositories.

start-recent = Recent
start-empty = Open a repository to get started.
# Beside a recent repository whose folder is not there any more.
start-folder-gone = folder is gone

## A recent repository's menu.

start-menu-open = Open
# Asks where a repository whose folder is gone is now.
start-menu-locate = Where is it now?
start-menu-favourite = Make it a favourite
start-menu-unfavourite = Take out of the favourites
start-menu-move-up = Move up
start-menu-move-down = Move down
start-menu-take-out = Take out of this list (the repository itself stays)

## The workspaces.

start-workspaces = Workspaces
start-workspace-repositories =
    { $count ->
        [one] { $count } repository
       *[other] { $count } repositories
    }
# $description is what the workspace is for, as its owner wrote it.
start-workspace-repositories-described =
    { $count ->
        [one] { $count } repository · { $description }
       *[other] { $count } repositories · { $description }
    }
