### The activity log's commands.

## The commands: what the palette and the command line call each, and what it does.

activity-list = Activity log
activity-list-description = The git processes run for this repository, newest first, those of earlier runs too (the last 1000 that did something); params: limit (200 when absent, at most 2000), questions (bool: also those that only ask, which are the running application's only), text (kept when the command line or the output has it, letters compared without case)
activity-clear = Clear activity log
activity-clear-description = Forget the git processes recorded for this repository, those kept from earlier runs too
activity-hook-list = Hooks
activity-hook-list-description = The hooks git will run here: name and file of each, the directory they are in and whether core.hooksPath names it

## Refusals.

# The text to look for is longer than max bytes.
activity-text-too-long = longer than { $max } bytes
