### Issues.

## Words shared by the section, the card and the view.

issues-description = Description
issues-labels = Labels
issues-milestone = Milestone
issues-assignees = Assignees
# Under a milestone; $due is the day it is due.
issues-due = due { $due }
# Where an issue stands.
issues-state-open = Open
issues-state-closed = Closed
# The provider answered with something that cannot be read.
issues-reply-unreadable = The answer could not be read

## The groups of the ISSUES section.

issues-group-open = All open
issues-group-mine = Mine

## The ISSUES section of the left panel.

issues-search = Search
issues-query-placeholder = The provider's query, e.g. label:bug
issues-reading = Reading…
issues-row-menu = Menu of the issue
issues-no-account = No account is connected for { $host }.
issues-connect-account = Connect an account
# The button that picks the repository whose issues are shown.
issues-repository = Whose issues are shown
# What a screen reader says of the button that opens and closes the query.
issues-query = Query
issues-query-open = Ask the provider with a query
issues-query-close = Ask without a query
issues-query-go = Ask with this query
# The tooltip of the query's row; $help is one of the lines below.
issues-query-tooltip = Enter asks the provider. { $help }
# What a query may say, at each provider.
issues-query-help-github = GitHub: assignee:, creator:, mentioned:, labels:a,b, milestone:, state:, since:, sort:, direction:, and words of titles.
issues-query-help-gitlab = GitLab: @user or assignee:, author:, #label or label:, $milestone or milestone:, state:, -key: to leave out, and words.
issues-query-help = The provider's own words for issues.

## The menu of an issue's row; $named is its number ("#12").

issues-menu-branch = Create a branch for it
# $provider is GitHub or GitLab.
issues-menu-open-at = Open { $named } at { $provider }
issues-menu-copy-link = Copy the link of { $named }
issues-menu-show-here = Show { $named } here

## The card over an issue.

# Between where it stands and the provider, each behind a dot; $who is who opened it, $age how long ago ("3 h ago").
issues-opened-by = opened by { $who } { $age }
# The card of an issue key, while it is read or when it could not be; $named is its number ("#12"), $owner/$name its repository.
issues-card-reading = Reading { $named } of { $owner }/{ $name }…
# $problem is what the provider said.
issues-card-failed = { $named } of { $owner }/{ $name } could not be read: { $problem }

## An issue's view.

issues-view-github = GitHub issue
issues-view-gitlab = GitLab issue
issues-refresh = Read it again
# $provider is GitHub or GitLab.
issues-open-at = Open at { $provider }
issues-back-to-graph = Back to the graph
# After who opened it, under the title; $age is how long ago ("3 h ago").
issues-opened-it = opened it { $age }
issues-no-description = No description.
issues-comments = Comments
issues-no-comments = No comments.
# Where a closed issue stands, in the right column; $age is how long ago it last changed.
issues-closed-updated = Closed · updated { $age }
# Beside the branch made for the issue that is checked out.
issues-branch-checked-out = checked out
issues-branch-check-out = Check out
issues-create-branch = Create a branch
issues-create-branch-tooltip = A branch named for this issue, from the commit checked out
# While it is read; $named is its number ("#12").
issues-view-reading = Reading { $named } …

## The parts of the view's right column.

issues-status = Status
issues-branches = Branches
