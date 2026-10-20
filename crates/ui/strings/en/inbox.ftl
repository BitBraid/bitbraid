### The inbox: pull requests, issues and work in progress.

# The inbox's name: its tab, its heading and the status bar.
inbox-name = Inbox
inbox-search-placeholder = Search, or a number
# A pin or a snooze the command refused; $error is what it said.
inbox-keep-failed = The inbox could not keep that: { $error }
# $named is the pull request as the status bar names it (owner/name#12).
inbox-merged = { $named } is merged.
# The provider answered a merge without merging and without saying why.
inbox-not-merged = The provider did not merge it.
inbox-page-cannot-open = That page cannot be opened from here.

## The tabs.

inbox-tab-pulls = Pull requests
inbox-tab-issues = Issues
inbox-tab-work = Work in progress
inbox-tab-all = All
inbox-tab-snoozed = Snoozed

## Whose rows are kept.

inbox-whose-mine = Mine
inbox-whose-created = Opened by me
inbox-whose-assigned = Assigned to me
inbox-whose-review-requested = Asks my review
inbox-whose-mentioned = Names me

## The headings of the pull requests' groups.

inbox-group-ready-to-merge = Can be merged
inbox-group-review-mine = To review
inbox-group-no-reviewers = No reviewers
inbox-group-failing-checks = Checks failed
inbox-group-conflicts = Conflicts
inbox-group-changes-requested = Changes asked
inbox-group-commented = Commented
inbox-group-waiting-for-review = Awaiting review
inbox-group-draft = Drafts
inbox-group-other = Other

## The status bar: what a group says of a pull request, after its name.

inbox-says-ready-to-merge = can be merged
inbox-says-review-mine = waits for your review
inbox-says-no-reviewers = has no reviewers
inbox-says-failing-checks = has failed checks
inbox-says-conflicts = has conflicts
inbox-says-changes-requested = has changes asked
inbox-says-commented = was commented on
inbox-says-waiting-for-review = awaits review
inbox-says-draft = is a draft
inbox-says-other = is open
# $first is the first pull request of the group (owner/name#12), $sentence
# one of the inbox-says messages, $more how many others the group holds.
inbox-status =
    { $more ->
        [0] { $first } { $sentence }
       *[other] { $first } { $sentence }, and { $more } more
    }

## The status bar's panel: a line for each group.

inbox-urgent-ready-to-merge =
    { $count ->
        [one] { $count } pull request can be merged
       *[other] { $count } pull requests can be merged
    }
inbox-urgent-review-mine =
    { $count ->
        [one] { $count } pull request waits for your review
       *[other] { $count } pull requests wait for your review
    }
inbox-urgent-conflicts =
    { $count ->
        [one] { $count } pull request has conflicts
       *[other] { $count } pull requests have conflicts
    }
inbox-urgent-failing-checks =
    { $count ->
        [one] { $count } pull request has failed checks
       *[other] { $count } pull requests have failed checks
    }
inbox-urgent-changes-requested =
    { $count ->
        [one] { $count } pull request has changes asked
       *[other] { $count } pull requests have changes asked
    }
inbox-urgent-other =
    { $count ->
        [one] { $count } pull request
       *[other] { $count } pull requests
    }

## How long ago a row changed, in the fewest letters.

inbox-age-now = now
inbox-age-minutes = { $count }m
inbox-age-hours = { $count }h
inbox-age-days = { $count }d
inbox-age-weeks = { $count }w
inbox-age-months = { $count }mo
inbox-age-years = { $count }y

## When a snoozed row comes back. $time is a time of day (09:00), $date a
## date (2026-10-17).

inbox-snooze-ends-never = until unsnoozed
inbox-snooze-ends-today = until { $time }
inbox-snooze-ends-tomorrow = until tomorrow { $time }
inbox-snooze-ends-on = until { $date } { $time }

## The top of the inbox.

inbox-refresh = Read again from the providers
inbox-collapse-all = Collapse all
inbox-expand-all = Expand all
inbox-offline = Some of this was kept from before: a provider could not be reached
inbox-workspace = Workspace
inbox-no-workspace = No workspace
inbox-all-accounts = All accounts
inbox-all-labels = All labels
inbox-all-milestones = All milestones

## What is said in place of the list.

inbox-error = The inbox could not be read
inbox-reading = Reading…
inbox-reading-text = The providers are asked for what waits for you.
# $login is the account's user, $provider GitHub or GitLab, $error what it said.
inbox-account-failed = { $login } at { $provider } could not be asked. { $error }
# $problem is why the pins and snoozes could not be read.
inbox-marks-problem = Pins and snoozes are not shown: { $problem }
inbox-work-nothing = No work in progress
inbox-work-nothing-text = None of the repositories looked at has changes that are not committed. They are the open, favourite and recent ones, or the workspace's.
inbox-connect = Connect an account
inbox-no-account = No account is connected
inbox-no-account-text = Connect GitHub or GitLab in the settings, and the pull requests and issues that wait for you are gathered here from all their repositories.
inbox-snoozed-nothing = Nothing is snoozed
inbox-snoozed-nothing-text = The clock at the left of a row puts it here for an hour, four hours, until tomorrow morning or until you bring it back.
inbox-issues-nothing = No issues
inbox-issues-nothing-text = No open issue is assigned to you, opened by you or names you, as far as the choices above let through.
inbox-nothing = Nothing waits for you
inbox-nothing-text = No open pull request or issue of yours is found with the choices above.

## The headings of a table's columns.

inbox-column-state = STATE
inbox-column-item = ITEM
inbox-column-author = AUTHOR
# The others in a row: reviewers and assignees.
inbox-column-with = WITH
inbox-column-repository = REPOSITORY
inbox-column-action = ACTION
inbox-column-changes = CHANGES
inbox-column-branch = BRANCH

## A row: its pin and snooze.

inbox-unpin = Unpin
inbox-pin = Pin to the top
inbox-unsnooze = Bring back
inbox-snooze = Snooze
inbox-snooze-until-back = Until brought back
inbox-snooze-hour = For an hour
inbox-snooze-four-hours = For four hours
inbox-snooze-tomorrow = Until 09:00 tomorrow

## A row's button, and its menu. $provider is GitHub or GitLab.

inbox-action-merge = Merge
inbox-action-review = Review
inbox-action-open-repository = Open repository
inbox-action-open-at = Open at { $provider }
inbox-more = More
inbox-menu-merge = Merge…
inbox-menu-review = Review in its tab
inbox-menu-check-out = Check out { $branch }
inbox-menu-worktree = Check out in a new worktree…
# $name is the repository's name at the provider.
inbox-menu-clone = Clone { $name }
inbox-menu-locate = Locate it here…
inbox-menu-copy-link = Copy link
inbox-menu-show-issue = Show in its tab
inbox-menu-issue-branch = Create a branch…
# Under an issue's repository: makes a branch for it.
inbox-issue-branch = Create a branch
inbox-issue-create-branch = Create branch
inbox-copy-link = Copy its link

## A row's marks: its reviews, its repository.

inbox-review-changes-asked = Changes were asked for
inbox-review-approved = Approved
inbox-review-awaited = A review is awaited
inbox-review-commented = A reviewer commented
# $repo is owner/name; "here" is a clone on this machine.
inbox-repo-here = { $repo }, here
inbox-repo-not-here = { $repo }, not here

## A row of the work in progress.

inbox-work-changed = { $count } changed
inbox-work-new = { $count } new
inbox-work-removed = { $count } removed
inbox-work-moved = { $count } moved
# In place of the branch, when HEAD is detached.
inbox-work-detached = detached
inbox-work-ahead-behind = { $ahead } ahead, { $behind } behind

## The question before a merge.

# $pull is the pull request (owner/name#12), $base the branch it goes into.
inbox-merge-heading = Merge { $pull } into { $base }
inbox-merge-close = Close
inbox-merge-changes-asked = A reviewer asked for changes.
inbox-merge-no-changes-asked = No changes were asked for.
inbox-merge-checks-failed = Some of its checks failed.
inbox-merge-checks-running = Its checks have not ended.
inbox-merge-checks-passed = Its checks passed.
inbox-merge-no-checks = It has no checks.
inbox-merge-conflicts = It conflicts with { $base }.
inbox-merge-may-refuse = The provider may not take it as it stands.
inbox-merge-method-merge = Merge commit
inbox-merge-method-squash = Squash
inbox-merge-method-rebase = Rebase
inbox-merge-delete-branch = Delete its branch afterwards
inbox-merge-cancel = Cancel
inbox-merge-go = Merge
