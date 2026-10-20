### Search.

## The commands: what the palette and the command line call each, and what it does.

search-parse = Read a search query
search-parse-description = The words of a query as understood, or what is wrong and where; params: query, now (unix seconds that ages like 3d count back from; the clock when absent), zone (the time zone dates are read in, by its name; the system's when absent)
search-commits = Search commits
search-commits-description = Ids of the commits a query holds for, in graph order; people are the ones the mailmap makes of them, dates are in the system's time zone, issue:123 finds the messages naming #123 or owner/name#123; params: query, tips (all|head|{"{"}ids:[..]{"}"}), limit (10000 when absent), now, zone
search-content = Search content
search-content-description = Ids of the commits that add or remove a text, in graph order; params: text, regex (bool: changed lines matching it instead), paths, tips, limit
search-files = Find a file
search-files-description = The files of a commit's tree whose path holds the letters of the query in order, the best matches first; params: query, rev (HEAD when absent), limit (50 when absent, 1000 at most). At most 500,000 files are looked at, and the answer says when there were more
search-locate = Jump to
search-locate-description = The commit to jump to: the newest at or before a date, or what a revision names; params: one of date (unix seconds, or as in before:) and rev; tips, now and zone with date

## Refusals: what follows "invalid parameter "zone": " or "invalid parameter "date": ".

search-bad-zone = expected the name of a time zone, such as Europe/Vienna or UTC
# The forms are the ways a date can be written, as the search of the git crate says them.
search-bad-date = expected unix seconds, or { $forms }
search-date-or-rev = give a date or a rev, one of the two
