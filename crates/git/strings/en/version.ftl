### git's version.

## What a git older than some version does without: each is one line of
## a list.

version-lacks-stash-foretelling = foretelling whether a stash applies cleanly
version-lacks-conflict-foretelling = foretelling conflicts of a merge, a rebase or a pull
version-lacks-worktree-list = listing worktrees
version-lacks-initial-branch = naming the first branch of a new repository
version-lacks-switch-restore = checking out branches and restoring files

## Why an executable is not taken as git: the first words of the error,
## then why. The program is as it was named, or the path to it.

version-what-git-executable = git executable
version-stand-in = { $program } stands in for the git of Apple's Command Line Tools, which are not installed: `xcode-select --install` installs them
# The source is what the system said; the hint says what to do.
version-cannot-run-hint = cannot run { $program }: { $source }; { $hint }
version-cannot-run = cannot run { $program }: { $source }
# The start of what it answered, quoted.
version-not-git = { $program } is not git: it answered { $answer } to `--version`
version-no-name = no executable was named
version-name-control = the name has a control character in it
version-name-too-long = the name is too long
