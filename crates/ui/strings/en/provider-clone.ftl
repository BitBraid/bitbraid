### Cloning from a provider.

## Which repositories of the account are listed.

provider-clone-filter-own = Own
provider-clone-filter-organisation = Organisations
provider-clone-filter-starred = Starred
provider-clone-filter-all = All

## The form. $site is the provider's name (GitHub, GitLab Self-Managed and
## the like).

# $login is the account's user; $site the provider's name, or the server's
# host for a server of one's own.
provider-clone-heading = { $login } at { $site }
provider-clone-into-folder = Into folder
provider-clone-folder-placeholder = the folder the clone goes into
provider-clone-browse = Browse
# The prompt of the desktop's folder chooser.
provider-clone-browse-prompt = Clone into
provider-clone-show = Show
provider-clone-repository = Repository
provider-clone-search-placeholder = Search the repositories
# Before the choice of SSH or HTTPS.
provider-clone-clone-over = Clone over
provider-clone-clone = Clone

## Under the list and the form.

provider-clone-asking = Asking { $site } for the repositories…
# $problem is what the provider said.
provider-clone-unread = The list could not be read: { $problem }
provider-clone-none = No repositories here.
provider-clone-no-match = None of them matches.
provider-clone-more = { $site } has more than it sent: type to search them all.
# $protocol is SSH or HTTPS, $url the address, $path the new folder.
provider-clone-over = Over { $protocol }: { $url }, into { $path }
provider-clone-no-address = The provider gave no address to clone from.
provider-clone-pick = Pick a repository.
provider-clone-folder-not-full = The folder to clone into has to be a full path.
provider-clone-bad-name = The repository's name cannot be a folder's.
provider-clone-taken = There is something at { $path } already.
