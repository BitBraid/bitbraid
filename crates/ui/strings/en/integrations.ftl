### Accounts at providers, sign-in, SSH keys.

## A provider's page. $site is the provider's name (GitHub, GitLab
## Self-Managed and the like), $login an account's user.

integrations-token-placeholder = paste the token here
# The application id's field, empty: a server then takes a token only.
integrations-app-placeholder = none: a token only
integrations-connected-as = Connected to { $site } as { $login }.
integrations-paste-token-first = Paste a token first.
integrations-disconnect-ask = Disconnect { $login } from { $site }? Its token is taken out of the keychain.
integrations-disconnect = Disconnect
integrations-disconnected = { $login } is disconnected.
# $problem is why the accounts could not be read.
integrations-accounts-unread = The accounts could not be read: { $problem }
integrations-add-account = Add another account
integrations-add-account-tooltip = Several accounts can be connected here; the default one is used where a repository does not say which.
integrations-server-address = Server address
integrations-app-id = Application id
integrations-app-why-gitlab = For signing in with a code: the id of an application made on the server (not confidential, scopes api and read_user).
integrations-app-why-github = For signing in with a code: the client id of an OAuth app with the device flow on, made on the server.
integrations-sign-in-with-code = Sign in with a code
integrations-token = Token
integrations-save = Save
# $scopes are the scopes' names, with commas between.
integrations-token-scopes = The token needs these scopes: { $scopes }. It is kept in the system's keychain and never shown again.
# $host is the server's host name.
integrations-token-page-at = Make a token on { $host }
integrations-token-page = Make a token on the server
integrations-token-page-tooltip = Opens the provider's page for a new token in your browser, with its name and scopes filled in.
integrations-connect = Connect
integrations-connect-tooltip = Sign in with a code typed at the provider's site
integrations-use-token = Use a token
integrations-connect-with-token = Connect with a token
integrations-cancel = Cancel

## An account's card.

# Marks the account used where a repository does not say which.
integrations-default = Default
integrations-make-default = Make default
integrations-not-connected = Not connected
integrations-connecting = Connecting…
integrations-connected = Connected

## The dialog of a sign-in by code.

integrations-sign-in-to = Sign in to { $site }
integrations-sign-in = Sign in
integrations-asking-code = Asking { $site } for a code…
integrations-type-code = Open the page below, sign in to { $site } there if it asks, and type this code. This window waits for you meanwhile.
integrations-signed-in = Signed in to { $site } as { $login }.
integrations-denied = { $site } says the sign-in was turned down there.
integrations-expired = The code ran out before the sign-in was done.
integrations-unavailable = { $site } does not offer signing in with a code from here. Use a token instead: make one on { $site }'s page for tokens and paste it in the field.
integrations-no-code = The provider gave no code that can be typed.
integrations-copy-code = Copy the code
integrations-open-page = Open the page
# $interval is in seconds.
integrations-waiting = Waiting for { $site }: asked every { $interval } s
integrations-try-again = Try again
integrations-done = Done
integrations-close = Close

## The SSH key's rows. $provider is GitHub or GitLab; $to the account a
## key goes to.

integrations-ssh-key = SSH key
integrations-ssh-no-key = No key file yet
integrations-ssh-add = Add to { $provider }
integrations-ssh-new = New SSH key
integrations-ssh-generate = Generate and add to { $provider }
integrations-ssh-generate-tooltip = The title names the key at { $provider }, for { $to }. A new Ed25519 key goes into the .ssh folder of your home, never over a file that is there, and the profile connects with it from then on.
integrations-ssh-others = Other keys
integrations-ssh-private = Private key…
integrations-ssh-private-tooltip = A private key becomes the one the profile connects with.
integrations-ssh-public = Public key…
integrations-ssh-public-tooltip = A public key is the one added to the provider.
# The prompts of the desktop's file chooser.
integrations-ssh-choose-public = Add this public key
integrations-ssh-choose-private = Connect with this key

## What is said under the pointer on the key in use, a line each.

integrations-ssh-none-here = The profile connects with the SSH agent's keys and ssh's own files, and none of those is here. Make a key below, or choose one.
integrations-ssh-from-profile = The current profile connects with this key.
integrations-ssh-from-chosen = Chosen to be added; the profile's key stays as it is.
integrations-ssh-from-ssh = The profile uses the SSH agent's keys; this is ssh's own first key file.
integrations-ssh-added-to = It is added to { $to }.
integrations-ssh-no-fingerprint = ssh-keygen gave no fingerprint.

## What came of adding or making a key. $site is the provider's name,
## $private the new private key file.

integrations-ssh-added = The key was added to { $site } as "{ $title }".
integrations-ssh-no-free-name = There is no free name for a new key in ~/.ssh.
integrations-ssh-no-public = ssh-keygen made no public key.
integrations-ssh-generated = A new key is at { $private } and was added to { $site }; the profile connects with it from now on.
# $problem is what the provider said.
integrations-ssh-generated-not-added = A new key is at { $private }, but adding it failed: { $problem }
