### What any operation can say when it fails.

# A repository could not be opened: the path tried, and why.
errors-open = cannot open a repository at { $path }: { $source }
# Input that cannot be used: which input (a few words such as "branch name"), and why.
errors-invalid = invalid { $what }: { $detail }
# Git ran and ended badly: its command line, how it ended, and what it wrote to standard error.
errors-command = { $command } failed ({ $status }): { $stderr }
