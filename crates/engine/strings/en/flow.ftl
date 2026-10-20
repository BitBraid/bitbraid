### Gitflow.

## The commands: what the palette and the command line call each, and what it does.

flow-status = Gitflow
flow-status-description = Whether gitflow is set up here, its two branches and its prefixes, and the feature, bugfix, release, hotfix and support branches there are
flow-init = Set up gitflow
flow-init-description = Write the gitflow settings and create the develop branch when it is missing; params: main (main or master when absent), develop (develop), feature, bugfix, release, hotfix, support (branch prefixes), tag (tag prefix, empty when absent); what is absent stays as it is configured
flow-start = Start
flow-start-description = Create and check out a feature, bugfix or release from develop, or a hotfix or support branch from main; params: kind (feature|bugfix|release|hotfix|support), name, base (what to start from instead)
flow-finish = Finish
flow-finish-description = Merge a feature or bugfix into develop, or a release or hotfix into main, tagged there, and back into develop, then delete the branch; goes on from where an earlier finish stopped; a support branch is not finished; params: kind (feature|bugfix|release|hotfix), name, message (of the tag), keep (bool: leave the branch)

## Journal entries: what the undo menu and the activity log call each change.

flow-entry-init = Set up gitflow
flow-entry-init-develop = Set up gitflow: create branch { $develop }
# The kind is gitflow's word for the branch: feature, bugfix, release, hotfix or support.
flow-entry-start = Start { $kind } { $name }
flow-entry-finish = Finish { $kind } { $name }
# The steps of a finish, each an entry of its own; what is the finish's entry above.
flow-entry-finish-checkout = { $what }: check out { $onto }
flow-entry-finish-merge = { $what }: merge into { $onto }
flow-entry-finish-tag = { $what }: tag { $name }
flow-entry-finish-delete = { $what }: delete { $name }

## A step of a finish failed after others were done; what git said follows after a colon.

flow-failed-checkout-main = the finish failed at checking out the main branch
flow-failed-merge-main = the finish failed at the merge into the main branch
flow-failed-tag = the finish failed at the tag
flow-failed-checkout-develop = the finish failed at checking out the develop branch
flow-failed-merge-develop = the finish failed at the merge into the develop branch
flow-failed-delete = the finish failed at deleting the branch
