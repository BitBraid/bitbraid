### A shell on a pseudo-terminal.

## Why a shell could not be started: shown where the terminal would be.

# The detail is one of the reasons below.
session-spec-error = the shell cannot be started: { $detail }
# The error is what the system said.
session-spawn-error = the shell could not be started: { $error }
session-no-program = no program is named
session-nul = the program or an argument has a NUL character
# The name is in quotes.
session-bad-variable = { $name } is not the name of an environment variable
session-no-folder = { $path } is not a folder that is there
session-not-found = { $program } was not found
session-quote-in-path = { $program } has a quote in its path
session-equals-in-path = { $program } has a = in its path
session-no-env = env(1) was not found
session-no-pty = the PTY could not be set up
session-no-thread = no thread for the terminal
