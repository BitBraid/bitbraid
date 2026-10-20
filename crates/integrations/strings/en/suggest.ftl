### Suggested changes on a pull request.

## Parameters of pr.suggestions

suggest-param-suggestions-path = only this file of it
suggest-param-suggestions-note = words put before each suggestion, in Markdown

## The commands: what the palette and the command line call each, and what it does.

suggest-pr-suggestions = Suggestions from local edits
suggest-pr-suggestions-description = What the changes in the working tree to the files of a pull request would be sent as: one suggestion per change, on the lines of the commit checked out it replaces (GitHub's suggestion block, GitLab's suggestion on the last line), with why one cannot be sent where its lines are not in the pull request's changes or differ from its head; nothing is sent (pr.comment with path, line and start_line sends one); params: account, owner, name, number, path (only that file), note (words before each)

## Why a local edit cannot be sent as a suggestion.

suggest-too-many-lines = It takes more than { $most } lines.
suggest-not-in-commit = Its lines are not in the commit checked out.
suggest-not-in-one-part = Its lines are not all in one part of the pull request's changes, where comments on lines go.
suggest-head-differs = The pull request's head differs here from the commit checked out: bring the branch up to date first.

## Refusals of pr.suggestions: what follows the name of the parameter refused.

suggest-not-a-path = is not a path of the working tree
suggest-no-working-tree = has no working tree to suggest from
