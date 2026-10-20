### Pull requests.

## Parameters: said of one taken by several commands

# base: pr.create, pr.update
pr-param-base = the branch it goes into

# remote: pr.checkout, pr.head
pr-param-remote = the remote the pull request's repository is; the one linked to it when absent

## Parameters of pr.list

pr-param-list-filter = whose: all, opened by the user, assigned to them, or waiting for their review; all when absent
pr-param-list-query = the provider's search words
pr-param-list-state = open when absent

## Parameters of pr.create

pr-param-create-head = the branch it brings; owner:branch for a fork's
pr-param-create-title = its title
pr-param-create-body = its description, in Markdown
pr-param-create-draft = open it as a draft
pr-param-create-reviewers = logins asked to review it
pr-param-create-assignees = logins it is assigned to
pr-param-create-labels = its labels
pr-param-create-milestone = its milestone's id

## Parameters of pr.update

pr-param-update-title = a new title
pr-param-update-body = a new description
pr-param-update-reviewers = who is asked to review it, in place of those before
pr-param-update-assignees = who it is assigned to, in place of those before
pr-param-update-labels = its labels, in place of those before
pr-param-update-milestone = its milestone's id; empty takes it away
pr-param-update-draft = whether it is a draft

## Parameters of pr.merge

pr-param-merge-method = a merge commit, one squashed commit, or its commits replayed
pr-param-merge-delete-branch = remove its branch afterwards
pr-param-merge-title = the merge commit's title
pr-param-merge-message = the merge commit's message
pr-param-merge-retarget-stack = move the open pull requests whose base is its branch to its base, before its branch is removed

## Parameters of pr.review

pr-param-review-event = what the review says
pr-param-review-body = what is written with it

## Parameters of pr.comment

pr-param-comment-body = the comment, in Markdown
pr-param-comment-reply-to = the id of the comment or conversation it answers
pr-param-comment-path = a file of it, to comment on lines of it as its head has them
pr-param-comment-line = with path: the line, the last one when on several
pr-param-comment-start-line = with path: the first line, when on several (GitHub)

## Parameters of pr.for_branch

pr-param-for-branch = the branch's name at the provider

## Parameters of pr.checkout

pr-param-checkout-dirty = what happens to changes not committed; refuse when absent

## The commands: what the palette and the command line call each, and what it does.

pr-list = Pull requests
pr-list-description = A repository's pull requests (merge requests at GitLab) with what their rows and hover cards show: checks summed up, reviewers and their verdicts, labels, assignees, comments; params: account, owner, name (the open repository's link when absent), filter (all|mine|assigned|review_requested), query (GitHub's search qualifiers such as author:, label:, review:, -label: to leave out, a,b for either; GitLab takes author:, assignee:, review-requested:, label:, milestone:, base:, head:, draft: and words), state (open|closed|merged|all)
pr-show = Show pull request
pr-show-description = A pull request with everything its view shows: description, milestone, mergeability, checks, commits, files, reviews and conversations; params: account, owner, name, number
pr-options = Pull request choices
pr-options-description = What may be chosen for a pull request: labels, milestones, people who may be asked, merge methods; params: account, owner, name
pr-templates = Pull request templates
pr-templates-description = The repository's templates for a pull request's description, read from the working tree
pr-create = Create pull request
pr-create-description = Open a pull request (a merge request at GitLab, where a draft is marked in its title and its branch is not removed unless asked); answers it as its row shows it; params: account, owner, name, head (owner:branch for a fork's), base, title, body, draft (bool), reviewers (logins; organisation/team for a team at GitHub), assignees, labels, milestone (its number at GitHub, its id at GitLab)
pr-update = Edit pull request
pr-update-description = Change a pull request; answers it as its view shows it; params: account, owner, name, number, and any of title, body, reviewers, assignees, labels (each list in place of the one before), milestone (empty takes it away), draft (bool), base (another branch to go into)
pr-checkout = Check out pull request
pr-checkout-description = Fetch a pull request's head (refs/pull/N/head, refs/merge-requests/N/head) and check it out in a local branch named as its branch (a fork's as owner/branch when that name is taken here, pull-N when both are), following the branch it comes from, its fork added as a remote named after its owner; a branch that is there is moved forward; answers the branch, the commit, what was made, moved and added; params: account, owner, name, number, remote (the remote of the pull request's repository; the linked one when absent), dirty (refuse|stash|discard)
pr-head = Fetch pull request head
pr-head-description = Fetch a pull request's head (refs/pull/N/head, refs/merge-requests/N/head) without checking anything out, and the branch it comes from where a remote here is that branch's repository; answers the commit, the branch, and where a worktree for it starts (start: that remote's branch when it is at the head, else the commit); params: account, owner, name, number, remote (the remote of the pull request's repository; the linked one when absent)
pr-merge = Merge pull request
pr-merge-description = Merge a pull request at the provider in a way the repository allows (GitLab: rebased first when the project needs it); answers whether it merged, the commit, what was moved and removed; params: account, owner, name, number, method (merge|squash|rebase), delete_branch (bool), title, message, retarget_stack (bool: the open pull requests whose base is its branch go to its base first, and its branch is kept while one could not be moved)
pr-review = Review pull request
pr-review-description = Approve a pull request, ask for changes, or comment (GitHub takes the last two only with a body, and no verdict on one's own; GitLab keeps the verdict only of someone asked to review, and asking for changes takes back one's approval); params: account, owner, name, number, event (approve|request_changes|comment), body
pr-comment = Comment on pull request
pr-comment-description = Comment on a pull request, on lines of one of its files, or answer a comment: reply_to is a conversation's id as pr.show gives it (GitHub: a comment on a line is answered in its thread, one on the conversation by a comment quoting it; GitLab: a thread's id); path and line put it on that line of the file as the head has it (start_line for several: GitHub takes the range, GitLab keeps the comment on line), which is how a suggestion from pr.suggestions is sent; params: account, owner, name, number, body, reply_to, path, line, start_line
pr-stack = Pull request stack
pr-stack-description = The pull requests stacked with this one, from the one on the base branch to the top, and its place there; params: account, owner, name, number
pr-for-branch = Pull request of a branch
pr-for-branch-description = The open pull request whose branch this is, or null; params: account, owner, name, branch

## Checking out a pull request: refusals (after the name of the parameter refused) and notes.

# The name is in quotes.
pr-no-remote = there is no remote { $name }
# The repository is owner/name at the provider.
pr-no-linked-remote = no remote of the open repository is { $repository }: name the remote
# The owner is the fork's owner as a remote is named after it.
pr-fork-names-taken = every name for { $owner }'s fork is taken
pr-head-missing = its head did not come
# The branch is in quotes, as the provider named it.
pr-head-unusable = #{ $number } names a branch git does not take: { $branch }
# The fork is owner/name at the provider.
pr-fork-elsewhere = { $fork } was not added as a remote: its address is not at the account's site
# The head is the pull request's branch.
pr-names-taken = { $head }, its other names and pull-{ $number } are branches of other things here
pr-nothing-to-pull = it follows nothing to pull from
# The error is why the branch could not be moved.
pr-left-behind = { $name } was left where it was, as it cannot be moved forward to the head: { $error }

## Refusals of the commands: what follows the name of the parameter refused.

pr-missing = missing
pr-not-a-branch = is not a branch's name
pr-nothing-to-change = nothing to change was given
pr-not-a-comment = is not a comment's id
pr-path-with-reply = is not given with reply_to
pr-not-a-file = is not a file of the pull request
pr-start-after-line = is after line
pr-line-without-path = is given with path
