### Opening files and folders in the user's programs.

## The commands: what the palette and the command line call each, and what it does.

open-folder = Show folder in file manager
open-folder-description = Open the repository's folder in the system's file manager, or with a command; params: path (a folder in the worktree, or a full path; the worktree's top when absent), command (words, with {"{"}path{"}"} filled in)
open-editor = Open folder in editor
open-editor-description = Open the repository's folder with the program of an editor command; params: command (the editor command as a file is opened with: its program is started with the folder), path
open-terminal = Open terminal here
open-terminal-description = Start a terminal in the repository's folder: the one a command names, else the one found on this system; params: command (words, with {"{"}path{"}"} filled in), path
open-terminals = Terminals found
open-terminals-description = The terminal programs found on this system, the one that would be started first, each with the command it is started by

## Refusals: what follows "invalid parameter "path": " or "invalid parameter "command": ".

open-not-a-path = is not a path
# The path is the folder's.
open-not-a-folder = { $path } is not a folder that is there
open-not-a-command = is not a command
open-empty = is empty
# The program is the first word of the command; the error is what the system said.
open-cannot-start = cannot start { $program }: { $error }
open-no-editor = no editor is named; without one there is nothing to open a folder with
open-no-terminal = no terminal was found on this system; name one in the preferences
