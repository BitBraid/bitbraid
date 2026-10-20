### Accounts at providers.

## Parameters of account.remove, account.set_default

account-param-id = the account's id

## Parameters of account.sign_in_start

account-param-client-id = the device flow's application id at a server of one's own; kept for that site

## Parameters of account.sign_in_poll

account-param-sign-in-poll-device-code = what account.sign_in_start answered with
account-param-sign-in-poll-interval = the seconds waited between asks so far; 5 when absent

## Parameters of account.add_token

account-param-add-token = a token made at the provider; kept in the system's keychain only

## The commands: what the palette and the command line call each, and what it does.

account-list = Connected accounts
account-list-description = The accounts connected at GitHub and GitLab, each with its id, kind, web, api, login, name, avatar_url, sign_in, scopes and whether it is its site's default; no token
account-sign-in-start = Sign in with a code
account-sign-in-start-description = Ask the provider for a code to type at its site: answers user_code, verification_uri, device_code, expires_in and interval; params: kind (github|gitlab), web (the site; github.com or gitlab.com when absent), client_id (the application id at a server of one's own)
account-sign-in-poll = Finish signing in with a code
account-sign-in-poll-description = Ask whether the code was typed: answers state waiting, slow_down (with interval), denied, expired, or done (with the account, its token kept in the keychain); params: kind, web, device_code, interval
account-add-token = Connect with a token
account-add-token-description = Connect an account with a token made at the provider, after asking it whose the token is; the token is kept in the system's keychain and nowhere else; params: kind (github|gitlab), web, token
account-remove = Disconnect an account
account-remove-description = Forget an account and take its token out of the keychain; params: id
account-set-default = Use as the default account
account-set-default-description = Make an account the one asked at its site when several are connected there; params: id

## Refusals of the commands' parameters: each follows the parameter's name.

account-client-id-own-server-only = is given for a server of one's own only
account-client-id-not-an-id = is not an application id
account-device-code-too-long = is longer than a code is
account-not-a-token = is not a token

## What signing in and keeping the accounts can say when it fails.

# The site is its host, with the port when it is not the usual one.
account-code-sign-in-unavailable = signing in with a code is not available at { $site } yet: connect with a token instead
# Why is what is wrong with the address, in a few words.
account-address = the provider's address: { $why }
account-address-question-or-fragment = a question or a fragment in it
# The file is the accounts' file's name (integrations.toml); why is what is wrong with it.
account-file-unusable = { $file } cannot be used: { $why }
account-file-too-large = it is larger than such a file can be
account-file-too-many = it lists more accounts than are kept
account-change-failed-half-way = another change failed half-way
# The id is the account's id, in quotes.
account-not-connected = no account { $id } is connected
account-odd-login = the provider named no user, or one with an odd name
account-token-empty = the token is empty
account-too-many = no more than { $most } accounts are kept
# The id is the account's id.
account-no-token = no token is kept for { $id }
account-token-ran-out = the token ran out and cannot be renewed here

## The keychain.

# What the system's keychain shows for an account's token: the login, and the site (its host, with the port when it is not the usual one).
account-keychain-label = { -product }: { $login } at { $site }
