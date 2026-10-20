### Sparse checkout.

## The commands: what the palette and the command line call each, and what it does.

sparse-status = Sparse checkout
sparse-status-description = Whether only part of the files is checked out, whether by directory (cone) or by patterns, the directories or patterns, and how many tracked files are left out
sparse-set = Check out only
sparse-set-description = Check out only part of the files, turning sparse checkout on: the files at the top and everything below some directories, or what some patterns take; files with changes stay; params: directories (cone mode) or patterns (one of the two)
sparse-add = Add to checkout
sparse-add-description = Check out more of a sparse checkout: directories, or the files patterns take (patterns only where the checkout goes by patterns); params: directories or patterns
sparse-disable = Check out everything
sparse-disable-description = Turn sparse checkout off and check every file out again; what was chosen is kept for the next time

## What is wrong with the parameters.

sparse-not-both = give directories or patterns, not both
sparse-neither = give directories or patterns
sparse-nothing-to-add = nothing to add
