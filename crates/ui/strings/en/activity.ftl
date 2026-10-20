### The activity log.

## The dialog's title row.

activity-heading = Activity log
activity-filter = Filter by command or output
activity-questions = Questions
activity-questions-tooltip = Also what the application only asks git
activity-reload = Read the log again
activity-clear = Clear
activity-clear-tooltip = Forget the entries of this repository
activity-close = Close

## The line under the title row.

# Fewer entries are shown than matched the filter.
activity-summary-last = The last { $shown } of { $matched } entries
activity-entries =
    { $count ->
        [one] { $count } entry
       *[other] { $count } entries
    }
activity-no-hooks = No hooks run in this repository.
# `hooks` is the names of the hooks, put together with commas.
activity-hooks = Hooks that run here: { $hooks }
activity-nothing = Nothing has been run here yet.

## An entry: one git process.

# How a process ended, in a few words.
activity-ended-ok = ok
# It ended with an exit code other than 0.
activity-ended-exit = exit { $code }
activity-ended-killed = killed
activity-ended-stopped = stopped
activity-ended-not-started = not started
activity-ended-running = running
activity-ended-unknown = unknown
# How long it took, for a column a few characters wide.
activity-milliseconds = { $ms } ms
activity-seconds = { $seconds }.{ $tenths } s
activity-minutes = { $minutes } min
activity-cut = Only the end of what it wrote is kept.
activity-wrote-nothing = It wrote nothing.
activity-copy = Copy
