### Recorded resolutions (rerere).

## The commands: what the palette and the command line call each, and what it does.

rerere-status = Recorded resolutions
rerere-status-description = Whether rerere is on, how many resolutions it has recorded, and of the files in conflict which it resolved from its record and which are still to do
rerere-enable = Record resolutions
rerere-enable-description = Switch rerere on or off; params: on (bool), scope (repo|user, repo when absent: whose configuration is written)
rerere-forget = Forget resolution
rerere-forget-description = Forget the resolution rerere recorded for the conflict a file is in; params: path
rerere-records = What rerere has recorded
rerere-records-description = The conflicts rerere has a record of, the newest first: each with its id, whether a resolution is recorded and when, how many conflicts the file had, the two sides of the first and what it was resolved to, and the file it is in when that file is being resolved now; params: limit
rerere-forget-record = Forget a recorded resolution
rerere-forget-record-description = Forget one record of rerere's; it is not kept anywhere and cannot be undone. The record of a conflict that is being resolved now has no resolution yet and is refused; params: id (as rerere.records gives it)

## Journal entries: what the undo menu and the activity log call each change.

rerere-entry-on = Switch rerere on
rerere-entry-off = Switch rerere off
