### Issues.

## Parameters of issue.show, issue.branch

issue-param-issue-number = the issue's number (its iid at GitLab)

## Parameters of issue.list

issue-param-list-filter = whose: all, assigned to the user, opened by them, or naming them; all when absent
issue-param-list-query = the provider's search words
issue-param-list-state = open when absent

## Parameters of issue.branch

issue-param-branch-template = how the branch is named, with {"{"}number{"}"} and {"{"}title{"}"}; {"{"}number{"}"}-{"{"}title{"}"} when absent
issue-param-branch = the branch's name as settled; made from the template and the issue's title when absent
issue-param-branch-start = where the branch starts; HEAD when absent
issue-param-branch-checkout = check the branch out

## Parameters of issue.key_from_branch

issue-param-key-branch = the branch's name

## Parameters of ci.status

issue-param-ci-shas = the commits' ids, at most 100

## The commands: what the palette and the command line call each, and what it does.

issue-list = Issues
issue-list-description = A repository's issues, pull requests left out; params: account, owner, name (the open repository's link when absent), filter (all|assigned|created|mentioned), query (GitHub: assignee:, creator:, mentioned:, state:, milestone:, labels:, since:, sort:, direction: and words in titles; GitLab: @user or assignee:, author:, #label or label:, $milestone or milestone:, state:, -key: to leave out, and words), state (open|closed|all)
issue-show = Show issue
issue-show-description = An issue with its description and comments; params: account, owner, name, number
issue-branch = Create branch for issue
issue-branch-description = Make a branch for an issue, named {"{"}number{"}"}-{"{"}title{"}"} or by a template, the title in lowercase letters, digits and hyphens, the name at most 60 characters; check it out when asked; params: account, owner, name, number, template, branch (the name, which then needs no provider), start, checkout (bool)
issue-key-from-branch = Issue of a branch
issue-key-from-branch-description = The issue a branch's name holds, if it has one (123-fix, fix-123, feature/123-x, issue-123, gh-123, #123): its number and key, or nulls; params: branch
issue-ci-status = Checks of commits
issue-ci-status-description = The checks run on commits, each commit's summed up and one by one; params: account, owner, name, shas (at most 100)

## Refusals: what follows the name of the parameter refused.

issue-missing = missing
# The value is in quotes.
issue-not-a-commit = has { $value }, which is not a commit's id
