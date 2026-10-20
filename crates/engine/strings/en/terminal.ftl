### Terminals.

## The commands: what the palette and the command line call each, and what it does.

terminal-shell = Shell for a terminal
terminal-shell-description = Which shell a terminal starts for the open repository, in which folder and with which environment; params: path (a folder of one of its worktrees; the open one's top when absent), program (the shell chosen in the preferences), args
terminal-paste-paths = Paths as a shell reads them
terminal-paste-paths-description = The text that pastes files into a terminal: each path quoted for the shell, separated by spaces; params: paths, shell (posix, fish, nushell, csh, powershell, cmd, or the shell's program; posix when absent)

## What is wrong with a parameter, after its name.

terminal-not-a-program = is not a program
terminal-program-not-found = { $program } was not found
terminal-too-many-arguments = more arguments than a shell is given
terminal-argument-not-passed = an argument cannot be passed on
terminal-not-a-path = is not a path
terminal-folder-missing = { $path } is not a folder that is there
terminal-not-a-folder = { $path } is not a folder
terminal-not-in-a-worktree = { $path } is not in a worktree of this repository
terminal-too-many-paths = more paths than are pasted at once
terminal-one-not-a-path = one of them is not a path
terminal-not-a-shell = is not a shell
