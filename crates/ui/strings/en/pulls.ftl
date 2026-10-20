### Pull requests.

## What goes wrong when a provider is asked.

pulls-answer-unreadable = The provider's answer could not be read.
pulls-no-answer = The command ended without an answer.
# A list, or a pull request, the provider answered with something that cannot be read.
pulls-reply-unreadable = The answer could not be read

## Words shared by the section, the card and the view.

pulls-reviewers = Reviewers
pulls-assignees = Assignees
pulls-labels = Labels
# Between the author and the branch to be merged: "octo wants to merge a/topic into a/main".
pulls-wants-to-merge = wants to merge
pulls-into = into
pulls-merged = Merged
pulls-closed = Closed

## The card beside a row.

# $age is how long ago, as "3 h ago".
pulls-card-opened = Opened { $age }
pulls-card-updated = Updated { $age }

## The groups of the section.

pulls-group-mine = Mine
pulls-group-assigned = Assigned to me
pulls-group-review-requested = Waiting for my review
pulls-group-all = All

## What is wrong with a filter that is about to be saved.

pulls-filter-no-name = Give it a name
pulls-filter-name-too-long = A name of at most { $most } characters
pulls-filter-no-query = Say what it looks for
pulls-filter-query-too-long = A query of at most { $most } characters
# $why is what the reader of queries said of it.
pulls-filter-query-unreadable = The query cannot be read: { $why }
pulls-filter-name-taken = A filter of that name is there already
pulls-filter-too-many = At most { $most } filters

## What GitLab's lists cannot be asked by; $key is the word of the query.

pulls-query-gitlab-unknown-key = { $key }: is not a word GitLab's lists take
pulls-query-gitlab-negated-branch = -{ $key }: GitLab cannot leave a branch out

## Where a pull request stands, as a row marks it.

pulls-mark-ready = Checks passed, no review awaited
pulls-mark-waiting = Waiting for checks or reviews
pulls-mark-failing = A check failed or changes were asked for
# After a branch's pull request in the hover of its mark in the graph.
pulls-mark-stack-place = { $place }/{ $of } in a stack

## Kept from before because the provider could not be reached; $when is "1 h ago".

pulls-offline-as-read = Offline: as read { $when }
pulls-offline-as-read-before = Offline: as read before

## Whether it merges.

pulls-merges-cleanly = Merges into { $base } without conflicts
pulls-merge-conflicts = Conflicts with { $base }
pulls-merge-blocked = Something the repository requires is missing: reviews or checks
pulls-merge-unknown = Whether it merges cleanly is not known yet

## Checks.

pulls-checks-none = No checks
pulls-checks-all-passed =
    { $all ->
        [one] All { $all } check passed
       *[other] All { $all } checks passed
    }
pulls-checks-some-said-nothing =
    { $all ->
        [one] { $all } check: { $passed } passed, the others did not run or said nothing
       *[other] { $all } checks: { $passed } passed, the others did not run or said nothing
    }
# $parts is the counts below, put together with commas.
pulls-checks-counted =
    { $all ->
        [one] { $all } check: { $parts }
       *[other] { $all } checks: { $parts }
    }
pulls-checks-failed = { $failed } failed
pulls-checks-running = { $running } running or waiting
pulls-checks-passed = { $passed } passed
# $word is one of the states below.
pulls-checks-word = Checks: { $word }
# A check's state, then how long ago it ran ("passed 3 h ago"); $age may be empty.
pulls-check-state-age = { $state } { $age }
# Under the checks listed, for those that are not.
pulls-checks-more = and { $count } more

## A check's state, in a word.

pulls-ci-unknown = unknown
pulls-ci-waiting = waiting
pulls-ci-running = running
pulls-ci-passed = passed
pulls-ci-failed = failed
pulls-ci-cancelled = cancelled
pulls-ci-skipped = skipped
pulls-ci-neutral = neutral

## What a reviewer said, beside their name.

pulls-review-approved = approved
pulls-review-changes-requested = asked for changes
pulls-review-commented = commented
pulls-review-pending = not yet
pulls-review-dismissed = dismissed

## Buttons of more than one dialog.

pulls-cancel = Cancel

## The dialog that adds a filter to the section.

pulls-add-filter = Add a filter
pulls-filter-name = Name
pulls-filter-name-placeholder = Name of its group
pulls-filter-query = Query
# An example of a query in GitHub's search words.
pulls-filter-query-placeholder = author:octo label:bug -draft:true
# Under the query. $words is the words a query knows but the last, put together with commas, each with its colon ("author:, label:"); $last is the last of them.
pulls-filter-words = It knows { $words } and { $last }; a - in front leaves out what matches, a comma between values means either, and @me is you.
pulls-filter-add = Add

## A provider's Markdown, as it is drawn.

# At the end of a text that was cut because it is long.
pulls-markdown-cut = Cut here: the rest is at the provider.
# Before what a picture is said to show, where the picture was in a provider's text: "[image: the graph]".
pulls-markdown-image = image

## A file's changes as the provider sent them.

# The last line of a long patch; $most is how many lines are shown.
pulls-patch-cut = The rest is left out: { $most } lines are shown

## The PULL REQUESTS section of the left panel.

pulls-search = Search
pulls-reading = Reading…
pulls-remove-filter = Remove this filter
# The tooltip of a remote's heading: "origin at GitHub: octo/tools".
pulls-remote-tooltip = { $remote } at { $provider }: { $owner }/{ $name }
pulls-row-menu = Menu of the pull request
pulls-no-account = No account is connected for { $host }.
pulls-connect-account = Connect an account
pulls-offline-tooltip = The provider could not be reached: this is what was read last.

## The menu of a pull request's row; $named is its number as the provider writes it ("#12", "!12").

pulls-menu-open-at = Open { $named } at { $provider }
pulls-menu-copy-link = Copy the link of { $named }
pulls-menu-show-here = Show { $named } here
pulls-menu-check-out = Check out its branch
pulls-menu-select-head = Select { $branch } in the graph

## Fields of more than one form.

pulls-title = Title
pulls-description = Description
pulls-description-placeholder = What it changes and why (Markdown)
pulls-milestone = Milestone
pulls-branch = Branch
# A list to pick from that has nothing picked.
pulls-none = None
pulls-nothing-to-choose = Nothing to choose from
pulls-reading-choices = Reading what can be chosen…
pulls-back-to-graph = Back to the graph

## The entries of a branch's menu that start a pull request; $branch is the branch's name.

pulls-start-from = Start a pull request from { $branch }…
pulls-start-into = Start a pull request into { $branch }…

## The panel that makes a pull request (a merge request at GitLab).

pulls-new-pull-request = New pull request
pulls-new-merge-request = New merge request
pulls-create-pull-request = Create pull request
pulls-create-merge-request = Create merge request
pulls-push-and-create-pull-request = Push and create pull request
pulls-push-and-create-merge-request = Push and create merge request
pulls-new-pushing = Pushing…
pulls-new-creating = Creating…
# Over the two columns: where it comes from, where it goes.
pulls-new-from = From
pulls-new-into = Into
# What a screen reader says of the button that picks a column's repository.
pulls-new-from-repository = From repository
pulls-new-into-repository = Into repository
# The tooltip of that button: "origin: octo/tools at GitHub".
pulls-new-remote-tooltip = { $remote }: { $repository } at { $provider }
# What a screen reader says of the button that picks a column's branch.
pulls-new-branch-named = Branch: { $branch }
# Under the branch it comes from, when that is a remote's branch, not a local one.
pulls-new-remote-branch = a branch of the remote
pulls-new-choose-reviewers = Choose reviewers
pulls-new-choose-assignees = Choose assignees
pulls-new-choose-labels = Choose labels
pulls-new-no-milestone = The repository has no open milestone
pulls-new-template = Template
# What a screen reader says of the button that picks a template; $shown is the template's name, or None.
pulls-new-template-named = Template: { $shown }
pulls-new-draft = Open it as a draft
pulls-new-push-first = { $branch } is not on { $remote } yet: it is pushed first, and follows what it is pushed to.
# $named is the number of the open pull request of the branch it goes into ("#12"), $title its title.
pulls-new-stack = Add to the stack: on top of { $named } { $title }
# $below is the branch the pull request under it goes into.
pulls-new-beside = It goes into { $below }, where that one goes, beside it.
pulls-new-clean = The two branches merge without conflicts.
# How many files the two branches conflict in, for the message under it.
pulls-new-conflict-files =
    { $count ->
        [one] { $count } file
       *[other] { $count } files
    }
# $files is the message above; $named the first few files, put together with commas; $more how many are not named.
pulls-new-conflicts =
    { $more ->
        [0] The two branches conflict in { $files }: { $named }. It can be made; it cannot be merged until they are resolved.
       *[other] The two branches conflict in { $files }: { $named } and { $more } more. It can be made; it cannot be merged until they are resolved.
    }

## A pull request's view.

# The heading of the view.
pulls-view-github = GitHub pull request
pulls-view-gitlab = GitLab merge request
pulls-refresh = Read it again
# $provider is GitHub or GitLab.
pulls-open-at = Open at { $provider }
# Where it stands, in the pill under the title.
pulls-draft = Draft
pulls-open = Open
# The tooltip of a branch's chip under the title.
pulls-branch-check-out = Two clicks check it out
pulls-save = Save
pulls-change-title = Change the title
pulls-change-description = Change the description
# After the branches under the title, behind a dot; $age is how long ago, as "3 h ago".
pulls-opened = opened { $age }
# A conversation on a line of a file that was marked resolved.
pulls-thread-resolved = resolved
pulls-back-to-conversation = Back to the conversation
pulls-file-no-patch = The provider sent no changes for this file: it is large, or not text.
pulls-no-description = No description.
pulls-comments = Comments
pulls-no-conversation = No conversation yet.
# Under a milestone; $due is the day it is due.
pulls-due = due { $due }
# While it is read; $named is its number, as "#12".
pulls-view-reading = Reading { $named } …

## The parts of the view's right column.

pulls-files-changed = Files changed
pulls-commits = Commits
pulls-checks = Checks
pulls-participants = Participants

## Changing a pull request in place. What is being sent comes before what went wrong, as "Changing the title: …".

pulls-title-needed = A pull request needs a title.
pulls-changing-title = Changing the title
pulls-changing-description = Changing the description
pulls-changing-reviewers = Changing the reviewers
pulls-changing-assignees = Changing the assignees
pulls-changing-labels = Changing the labels
pulls-changing-milestone = Changing the milestone
# $error is what the provider said.
pulls-choices-unread-why = What can be chosen: { $error }
pulls-choices-unread = What can be chosen was not read
# What a screen reader says of a pencil in the right column.
pulls-change = Change

## Merging a pull request.

# The ways to merge, as the list of them names them.
pulls-method-merge = Merge commit
pulls-method-squash = Squash
pulls-method-rebase = Rebase
# What each way does, under its name.
pulls-method-merge-line = Its commits stay as they are, joined to the base by a merge commit.
pulls-method-squash-line = All its changes become one commit on the base.
pulls-method-rebase-line = Its commits are replayed onto the base, one by one, without a merge commit.
# The merge button, for each way.
pulls-merge = Merge
pulls-merge-squash = Squash and merge
pulls-merge-rebase = Rebase and merge
pulls-merge-how = How it is merged
# What the bar asks before it merges; $named is its number ("#12"), $base the branch it goes into.
pulls-merge-question = Merge { $named } into { $base } with a merge commit? It cannot be taken back from here.
pulls-merge-question-squash = Merge { $named } into { $base } as one squashed commit? It cannot be taken back from here.
pulls-merge-question-rebase = Merge { $named } into { $base } by replaying its commits? It cannot be taken back from here.
# Why the merge button cannot be pressed.
pulls-merge-refused-not-open = It is not open
pulls-merge-refused-draft = A draft is not merged: mark it ready first
pulls-merge-refused-conflicts = Its changes conflict with the base
pulls-merge-refused-no-method = The repository allows no way to merge it
pulls-merge-refused-busy = Something is being sent
# What is being sent while it merges; before what went wrong, as "Merging: …".
pulls-merging = Merging
# $why is what the provider said, or the message below.
pulls-merge-failed = Merging: { $why }
pulls-merge-not-done = the provider did not merge it.
# The notice when it was merged, with the parts below after it.
pulls-merged-notice = Merged { $named }
pulls-merged-branch-removed = ; its branch was removed
# $moved is the numbers of the pull requests that were above it in its stack, put together with commas; $count how many.
pulls-merged-retargeted =
    { $count ->
        [one] ; { $moved } now goes into its base
       *[other] ; { $moved } now go into its base
    }
pulls-was-merged = It was merged
pulls-was-closed = It was closed without being merged
# $head is its branch.
pulls-merge-remove-branch = Remove { $head } after merging
# $named is the number of the pull request above it in its stack; $base the branch this one goes into.
pulls-merge-retarget = Then { $named } goes into { $base }

## Its place in a stack.

pulls-stack-below = The one below
pulls-stack-above = The one above
pulls-stack-below-named = The one below: { $named }
pulls-stack-above-named = The one above: { $named }
pulls-stack-heading = STACK

## Suggestions from the edits to a file in the working tree.

pulls-suggestion-line = Line { $line }
pulls-suggestion-lines = Lines { $start } to { $end }
# Under the lines of a suggestion that are not drawn.
pulls-suggestion-more = and { $count } lines more
# Why the edits to the open file cannot be offered.
pulls-suggest-refused-no-file = Open a file's changes first.
pulls-suggest-refused-not-open = It is not open any more.
pulls-suggest-refused-no-repository = No repository is open here.
pulls-suggest-refused-not-checked-out = Check out its branch here first; your changes to it are suggested.
pulls-suggest-nothing = Nothing in the working tree differs from the commit checked out in this file.
# What is being sent while suggestions are sent; before what went wrong, as "Suggesting: …".
pulls-suggesting = Suggesting
# $error is what the provider said.
pulls-suggesting-failed = Suggesting: { $error }
pulls-suggest = Suggest my changes
pulls-suggest-tooltip = Offer your edits to this file as suggestions on its lines
pulls-suggest-sending = Sending { $sent } of { $total }
pulls-suggest-send =
    { $count ->
        [one] Send { $count } suggestion
       *[other] Send { $count } suggestions
    }
pulls-suggest-back = Back to the file's changes
pulls-suggest-back-tooltip = Back to the file's changes; nothing is sent
pulls-suggestions-heading = Suggestions from your changes
pulls-suggest-comparing = Comparing the commit checked out with the working tree…
pulls-suggestions-hint = Each is a comment on its lines that the author can take with one click.

## Talking on a pull request: the field at the bottom of the conversation.

pulls-comment-placeholder = Leave a comment (Markdown)
# Over the field while it answers a conversation; $who is who began it, $place the file (and line) it is on.
pulls-answering = Answering { $who }
pulls-answering-on = Answering { $who } on { $place }
pulls-reply-clear = Comment on the whole of it instead
pulls-comment-menu = Menu of the comment
pulls-reply = Reply
pulls-quote = Quote
pulls-comment = Comment
# What is being sent while a comment is sent; before what went wrong, as "Commenting: …".
pulls-commenting = Commenting
pulls-review = Review
pulls-review-hint = A review uses what is written here.
# What a review can say, in the Review button's list.
pulls-verdict-approve = Approve
pulls-verdict-request-changes = Request changes
pulls-verdict-comment = Comment as a review
# What is being sent while a review is sent; before what went wrong, as "Approved: …".
pulls-verdict-approved = Approved
pulls-verdict-changes-requested = Changes requested
pulls-verdict-reviewed = Reviewed
