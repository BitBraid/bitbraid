### Branches.

## Which input or operation was refused: the first words of the error.

branch-what-branch-name = branch name
branch-what-revision = revision
branch-what-start-point = start point
branch-what-branch = branch
branch-what-upstream = upstream
branch-what-fast-forward = fast-forward
branch-what-checkout = checkout
branch-what-checkout-target = checkout target

## Why it was refused. A name, revision or text in an argument is quoted.

branch-empty = must not be empty
branch-starts-with-dash = { $text } must not start with a dash
branch-control-characters = { $text } must not hold control characters
branch-not-valid-name = { $name } is not a valid branch name
branch-names-no-commit = { $rev } names no commit
branch-start-not-commit = { $start } does not name a commit
branch-does-not-exist = { $name } does not exist
branch-moved = { $name } is not where it was; nothing was changed
branch-nowhere-back = there is nowhere to go back to
branch-nothing-to-check-out = { $target } names nothing to check out

## Why a branch was not moved forward.

branch-no-branch = there is no branch { $name }
branch-no-upstream = { $name } has no upstream to follow
# The upstream, as remote/branch.
branch-upstream-missing = { $upstream } is not there; fetch first
# The branch, and the worktree it is checked out in.
branch-checked-out-elsewhere = { $name } is checked out at { $held }; pull there instead
branch-has-own-commits = { $name } has commits that { $upstream } does not; it cannot simply move forward

## What is wrong with a branch name, for whoever is typing it.

branch-name-begins-with-dash = The name cannot begin with a dash.
branch-name-reserved = { $name } is not a name it can have.
branch-name-at-brace = The name cannot contain @{"{"}.
branch-name-two-dots = The name cannot contain two dots in a row.
branch-name-space = The name cannot contain a space.
branch-name-control = The name cannot contain control characters.
# The character it cannot contain.
branch-name-character = The name cannot contain { $bad }
branch-name-slash = The name cannot begin or end with a slash, or have two in a row.
branch-name-ends-with-dot = The name cannot end with a dot.
branch-name-part-dot = No part of the name can begin with a dot.
branch-name-part-lock = No part of the name can end with .lock.
