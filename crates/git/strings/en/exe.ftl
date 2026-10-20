### Running git.

## Which operation or input failed: the first words of the error.

exe-what-git-executable = git executable
exe-what-git-command = git command
exe-what-operation = operation

## Why.

# The program tried as git, and the argument it was asked (--version).
exe-no-answer-in-time = { $program } did not answer `{ $command }` in time
exe-handle-lost = the process handle was lost
# An operation that was stopped by whoever started it. The interface
# looks for the word "cancelled" (and "operation: cancelled"): keep it.
exe-cancelled = cancelled

## The steps of running git or a tool; each is followed by what failed.

exe-spawning = spawning git
exe-writing-stdin = writing to git's stdin
exe-waiting = waiting for git
exe-preparing-tool = preparing to run a tool

## The activity log.

# In place of a setting's value, which is not written down.
exe-withheld = (the value is not recorded: it may hold a password)
