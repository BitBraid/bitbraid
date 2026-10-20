### Hooks that ended badly.

## What a hook that ended badly is to the user, and what there is to do
## about it.

# What the hook printed: nothing.
hookwatch-printed-nothing = it printed nothing
# What the hook printed: the end of it, in one line.
hookwatch-said = it said: { $said }
# A hook that runs after the fact (post-checkout, …) ended badly: its
# name, the status it ended with, and one of the two messages above.
hookwatch-failed-after = the { $hook } hook of this repository failed (it ended with status { $code }; { $what_it_said }). It runs after the fact: what was asked for is done, and only the hook's own work is not. Look at the hook (it is listed under Hooks in the activity log) or run it by hand
# A hook stopped what was asked for: its name, the status it ended
# with, one of the two messages above, and the way round it (one of the
# messages below after a blank, or nothing).
hookwatch-stopped = the { $hook } hook of this repository stopped this: it ended with status { $code }, and { $what_it_said }. Git itself had nothing against it. Put right what the hook reports and try again; the hook is a program in this repository (it is listed under Hooks in the activity log).{ $way_round }
# The ways round a hook that stopped a command, for the hooks that have one.
hookwatch-way-round-commit = A commit can also be made without these two checks (`git commit --no-verify`), if that is what you mean to do.
hookwatch-way-round-merge = `git merge --no-verify` leaves this check out.
hookwatch-way-round-rebase = `git rebase --no-verify` leaves this check out.
hookwatch-way-round-push = `git push --no-verify` leaves this check out.
hookwatch-way-round-am = `git am --no-verify` leaves these two checks out.
