### GitHub.

## What a search or a list cannot take.

github-issues-no-negation = GitHub's list of issues cannot leave anything out: - is not taken
# The key is the one of the query that the list does not take (the words before the colon).
github-issues-key-not-taken = GitHub's list of issues does not take { $key }:; it takes assignee:, creator:, mentioned:, state:, milestone:, labels:, since:, sort: and direction:
# The title is the milestone's title as it was asked for, in quotes.
github-no-milestone-called = no milestone is called { $title }
github-issue-is-pull-request = #{ $number } is a pull request; pr.show shows it
# The term is the part of the query that is not taken (repo:owner/name).
github-pulls-query-not-taken = the query is about this repository's pull requests: { $term } is not taken
# The login is the account's login, in quotes.
github-login-not-searchable = the login { $login } cannot be searched for

## Opening, changing and merging pull requests.

# The id is what was given, in quotes.
github-not-a-milestone-number = { $id } is not a milestone's number at GitHub
# What is one of the three messages below; the error is what went wrong.
github-opened-but = pull request #{ $number } was opened, but { $what }: { $error }
github-reviewers-not-asked = its reviewers could not be asked
github-fields-not-set = its assignees, labels or milestone could not be set
github-not-read-again = it could not be read again
github-no-node-id = the pull request came without its node id
# The from is the branch of the pull request that was merged, which others went into.
github-stacked-not-read = the pull requests on { $from } could not be read: { $error }
github-stacked-still-goes = #{ $number } still goes into { $from }: { $error }
github-merged-already = #{ $number } is merged already
github-closed = #{ $number } is closed
github-branch-not-here = its branch is not in this repository and was left
github-branch-kept = its branch was kept, as a pull request still goes into it
github-branch-not-removed = its branch could not be removed: { $error }

## Reviews and comments.

github-review-needs-text = GitHub takes a review that asks for changes or only comments with something written in it
# The id is what was given, in quotes.
github-not-a-comment-id = { $id } is not a comment's id at GitHub
github-no-head-commit = GitHub named no head commit for the pull request
