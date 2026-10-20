### The inbox.

## Parameters of inbox.items

inbox-param-items-workspace = a workspace's id (workspace.list): only the rows of its repositories, and their work in progress
inbox-param-items-paths = repositories on this machine, by full path: where rows are found here and work in progress is looked for
inbox-param-items-account = only this account's rows
inbox-param-items-wip = look for work in progress; yes when absent

## Parameters of inbox.pin, inbox.unpin, inbox.unsnooze, inbox.snooze

inbox-param-key = the row's key, as inbox.items answers it

## Parameters of inbox.snooze

inbox-param-snooze-for = how long: an hour, four hours, until nine tomorrow morning by the local clock, or until it is brought back
inbox-param-snooze-until = until this time: seconds since the Unix epoch, or a local date and time such as 2026-10-20T09:00

## The commands: what the palette and the command line call each, and what it does.

inbox-items = Inbox
inbox-items-description = The open pull requests and issues of the user at every connected account (or only those of a workspace's repositories), each with its account, its repository here if there is one, why it is the user's, the groups it falls in (ready_to_merge, review_mine, no_reviewers, failing_checks, conflicts, changes_requested, commented, waiting_for_review, draft, other), its lines added and removed, and whether it is pinned or snoozed; the repositories with uncommitted work; and how each account answered; params: workspace (an id), paths (repositories here), account, wip (bool), fresh (bool)
inbox-pin = Pin in inbox
inbox-pin-description = Keep a row of the inbox at the top of its list; kept in inbox.toml, never sent anywhere; params: key
inbox-unpin = Unpin in inbox
inbox-unpin-description = Let a pinned row of the inbox go back to its place; params: key
inbox-snooze = Snooze in inbox
inbox-snooze-description = Put a row of the inbox out of sight, in the snoozed tab, for an hour, four hours, until nine tomorrow morning, until a time, or until it is brought back; it comes back by itself when the time is up; params: key, for (hour|four_hours|tomorrow|unsnoozed), until (seconds, or a local 2026-10-20T09:00)
inbox-unsnooze = Unsnooze in inbox
inbox-unsnooze-description = Bring a snoozed row of the inbox back; params: key

## Refusals: what follows the name of the parameter refused.

inbox-missing = missing
inbox-not-a-key = is not a row's key
inbox-paths-not-a-list = is a list
inbox-paths-too-many = has more than { $most } items
inbox-paths-not-texts = is a list of texts
inbox-paths-not-full = each has to be the full path of a folder
# The id is in quotes.
inbox-no-workspace = no workspace { $id }
# The id is in quotes.
inbox-no-account = no account { $id } is connected
inbox-until-unreadable = is seconds since the Unix epoch or a local date and time such as 2026-10-20T09:00
inbox-until-past = is not in the future
inbox-until-with-for = is given with for; give one of them
# The operation a refusal names, before a colon.
inbox-refused = inbox

## The file of pins and snoozes: why it is not written over, and why a change is refused.

# The file is its name; the error is what reading it said.
inbox-file-unreadable = { $file } cannot be read ({ $error }); it is left as it is
# The version is the one the file says wrote it.
inbox-file-newer = { $file } was written by a newer version ({ $version }); it is left as it is
inbox-file-too-large = { $file } is larger than { $most } bytes; it is left as it is
# The error is what the system said.
inbox-file-unopenable = { $file } cannot be opened ({ $error })
inbox-marks-poisoned = another change of the pins failed half-way
inbox-marks-too-many = at most { $most } pins and { $most } snoozes are kept
