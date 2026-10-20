### The providers' commands.

## Parameters taken by many of the provider commands

commands-param-account = the account's id; the open repository's link's when absent
commands-param-owner = the repository's owner or group path; the open repository's link's when absent
commands-param-name = the repository's name; the open repository's link's when absent
commands-param-fresh = ask the provider rather than answer with what was kept
commands-param-kind = the provider
commands-param-web = the site's address, for an Enterprise Server or a GitLab of one's own; github.com or gitlab.com when absent
commands-param-number = the pull request's number (a merge request's iid at GitLab)

## Refusals of a parameter: each follows the parameter's name.

# A parameter the command needs was not given.
commands-missing = missing
commands-is-text = is text
# The max is a number of bytes.
commands-too-long = is longer than { $max } bytes
commands-control-characters = has control characters
commands-is-yes-or-no = is yes or no
commands-is-whole-number = is a whole number
commands-out-of-range = is from { $min } to { $max }
# The choices are the words it may be, joined by commas.
commands-is-one-of = is one of { $choices }
commands-is-list = is a list
commands-too-many-items = has more than { $most } items
commands-is-list-of-texts = is a list of texts
commands-unusable-item = has an item that cannot be used
commands-not-a-path = is not a path a provider allows
commands-not-a-name = is not a name a provider allows
commands-one-name-at-github = is one name at GitHub

## What a provider command says when it cannot go on.

# What a refusal is about, before a colon: "provider: …".
commands-what-provider = provider
# What a stopped command is about: "invalid operation: cancelled".
commands-what-operation = operation
# A command that was stopped by whoever started it. The interface looks for the word "cancelled": keep it.
commands-cancelled = cancelled
commands-no-account = no account: name one, or open a repository with a remote at a connected account
commands-which-repository = which repository: give owner and name, or open a repository linked to the account
