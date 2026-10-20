### Search.

## A query that cannot be read. Key names (author:, sha:) and `is:merge`
## are what is typed in a query, and stay as they are.

# Where in the query (a byte offset), and what was expected there: one of
# the search-expected- messages below, or the forms of an issue or a date.
search-error-at = at byte { $position }: expected { $expected }
search-expected-end-bytes = the query to end here ({ $most } bytes at most)
search-expected-end-words = the query to end here ({ $most } words at most)
search-expected-quote-placement = a quote only at the start of a word or right after the colon of a key
search-expected-closing-quote = a closing quote
search-expected-space-after-quote = a space after the closing quote
search-expected-no-control = text without control characters
search-expected-word-after-second-dash = a word after `-` (in quotes when it begins with a dash itself)
search-expected-word-after-dash = a word after `-`
# The key names, each followed by a colon and a space, then
# search-quoted-word-is-text.
search-expected-key = one of the keys { $keys }
search-quoted-word-is-text = (a word in quotes is text, colon and all)
search-expected-text-in-quotes = text between the quotes
# The key that has no value, as typed in a query.
search-expected-value-after-key = a value after `{ $key }:`
search-expected-hex = hexadecimal digits, 64 at most
search-expected-merge = `merge`, the one kind there is
search-issue-forms = an issue such as 123, #123, owner/name#123 or group/project#123
search-date-forms = a date such as 2026-10-01 or 2026-10-01T12:00, or an age such as 3d, 2w, 6m or 1y
# The most bytes a branch, tag or commit can have.
search-expected-rev = a branch, tag or commit: not beginning with `-`, without control characters, { $most } bytes at most
search-expected-path = a path or a pattern of one: not beginning with `-` or `:`, without control characters, { $most } bytes at most

## Which input or operation was refused: the first words of the error.

search-what-query = search query
search-what-search = search
search-what-operation = operation
search-what-regex = regular expression
search-what-revision = revision
search-what-path = search path
search-what-text = search text
search-what-paths = search paths

## Why a search was refused or stopped.

# A search that was stopped by whoever started it. The interface looks
# for the word "cancelled" (and "operation: cancelled"): keep it.
search-cancelled = cancelled
# The most branches a search of paths or content can start from.
search-too-many-tips = a search of paths or content starts from { $most } separate branches at most; search all of them, or fewer
# The revision, quoted.
search-not-a-commit = { $rev } does not name a commit
search-text-bounds = it has to be one line of { $most } bytes at most, and not empty
search-too-many-words = it has more than { $most } words
search-sha-takes-hex = sha: takes hexadecimal digits, 64 at most
search-date-without-instant = a date word without its instant
# The most paths a search of content can be held to.
search-too-many-paths = { $most } at most

## What a search is doing, while it runs.

search-stage-matching = Matching commits
search-stage-listing = Listing reachable commits
search-stage-paths = Searching paths
search-stage-content = Searching content
