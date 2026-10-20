### Resolutions git has recorded (rerere).

rerere-heading = Recorded conflict resolutions
rerere-loading = Loading…
rerere-close = Close
rerere-nothing = Nothing is recorded. With rerere on, git remembers how a conflict was resolved and resolves it the same way the next time.

## The line under the heading: its parts are put together with a dot between.

rerere-off = rerere is off here
rerere-recorded =
    { $count ->
        [0] nothing recorded yet
        [one] { $count } resolution recorded
       *[other] { $count } resolutions recorded
    }
# Records from when rerere was on; it is off now.
rerere-recorded-before =
    { $count ->
        [0] nothing recorded
       *[other] { $count } recorded from before
    }
rerere-being-resolved = { $count } being resolved now

## A record.

rerere-being-resolved-in = being resolved now, in { $path }
rerere-never-resolved = a conflict seen and never resolved
rerere-resolution =
    { $conflicts ->
        [0] a resolution
        [one] a resolution
       *[other] a resolution of { $conflicts } conflicts in one file
    }
rerere-ours = one side
rerere-theirs = the other
rerere-resolved-to = resolved to
# A side of the conflict that has no lines.
rerere-side-empty = nothing
rerere-forget = Forget
rerere-forget-title = Forget this resolution?
rerere-forget-description = Git will not resolve this conflict by itself the next time it comes up. The record is not kept anywhere: this cannot be undone.
rerere-forgot = Forgot the recorded resolution
rerere-gone-already = That record was gone already
