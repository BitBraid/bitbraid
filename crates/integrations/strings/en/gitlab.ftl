### GitLab.

## What a search cannot take.

# The negation is "-" when the part was negated, else nothing; the key is the part's key with its colon (author:), or the word below.
gitlab-issues-not-searchable = GitLab's issues cannot be searched by { $negation }{ $key }: it takes @, assignee:, author:, # or label:, $ or milestone:, state: and words
# What a query's plain words are called when they cannot be negated.
gitlab-issues-words = words
# The keys are the parts of the query that are not taken, joined by commas.
gitlab-pulls-not-searchable = GitLab's merge requests cannot be searched by { $keys }: it takes author:, assignee:, review-requested:, label:, milestone:, base:, head:, draft: and words

## Asking several things at once.

gitlab-request-failed-on-thread = a request failed on its own thread

## Opening, changing and merging merge requests.

# The login is the one asked for, in quotes.
gitlab-no-such-user = there is no user { $login } at this GitLab
# The id is what was given, in quotes.
gitlab-not-a-milestone-id = { $id } is not a milestone's id at GitLab
# The from is the source branch of the merge request that was merged, which others went into.
gitlab-stacked-not-read = the merge requests into { $from } could not be read: { $error }
gitlab-stacked-still-goes = !{ $number } still goes into { $from }: { $error }
# The ways are those below, joined by commas.
gitlab-way-not-allowed = this project does not merge that way; it allows { $ways }
# The ways of merging, as pr.merge's method names them.
gitlab-way-merge = merge
gitlab-way-squash = squash
gitlab-way-rebase = rebase
gitlab-merged-already = !{ $number } is merged already
gitlab-closed = !{ $number } is closed
# The error is what GitLab said.
gitlab-not-rebased = it could not be rebased: { $error }
gitlab-branch-not-here = its branch is not in this project and was left
gitlab-branch-kept = its branch was kept, as a merge request still goes into it
gitlab-branch-not-removed = its branch could not be removed: { $error }
# The error is what GitLab said.
gitlab-fork-not-copied = GitLab could not copy the fork: { $error }

## Reviews and comments.

gitlab-verdict-not-kept = GitLab keeps a reviewer's verdict only for someone asked to review !{ $number }
# The same, when the review had something written in it, which was posted.
gitlab-verdict-not-kept-posted = GitLab keeps a reviewer's verdict only for someone asked to review !{ $number }; what was written was posted
# The id is what was given, in quotes.
gitlab-not-a-thread-id = { $id } is not a thread's id at GitLab
gitlab-no-diff = GitLab named no diff for the merge request
gitlab-file-not-changed = the merge request does not change { $path }
gitlab-line-not-changed = line { $line } of { $path } is not in the merge request's changes
gitlab-no-comment = GitLab answered with no comment
