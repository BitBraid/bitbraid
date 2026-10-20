### Links that open BitBraid.

## The commands: what the palette and the command line call each, and what it does.

link-parse = Read link
link-parse-description = What a bitbraid:// link names, as {"{"}repository: {"{"}path{"}"} or {"{"}url{"}"}, target: {"{"}kind: branch, tag, commit, pr or none, value{"}"}{"}"}, or why it is not a link of ours; params: url (the link)
link-make = Make link
link-make-description = A bitbraid:// link to a repository and, in it, a branch, a tag or a commit; params: path (a full path) or url (the address of a remote), the open repository when neither is given; at most one of branch, tag, commit
link-register = Register link handler
link-register-description = Have the system hand bitbraid:// links to the application: on Linux a desktop entry and xdg-mime default, on Windows the key of the scheme under HKEY_CURRENT_USER\Software\Classes, on macOS the application bundle the program is in, registered with Launch Services; params: exe (the program, the running one when absent), applications_dir (Linux: where the entry goes), dry_run (bool: only say what would be written), no_mime (bool, Linux: write the entry and leave the system's choice as it is)

## Why a text is not a link of ours: what follows "invalid parameter "url": ".
## A parameter is the name of one of a link's parameters (path, url, branch, tag, commit, pr).

# The largest number of bytes a link may have.
link-too-long = it is longer than { $max } bytes
link-raw-character = it has a blank or a control character in it; in a link those are written as percent escapes
# The scheme is the links' own, bitbraid.
link-scheme = it does not begin with { $scheme }://
link-action = it asks for something other than open, which is all a link asks for
link-fragment = it has a fragment; a # that belongs to a value is written %23
link-empty-parameter = it has an empty parameter
# The parameter is what the link has, in quotes.
link-unknown-parameter = { $parameter } is not a parameter a link has
link-twice = { $parameter } is given twice
link-no-value = { $parameter } has no value
link-escape = { $parameter } has a percent sign that two hex digits do not follow
link-not-utf8 = { $parameter } is not UTF-8 once its percent escapes are read
link-control = { $parameter } has a control character in it
link-no-repository = it names no repository: neither path nor url is given
link-two-repositories = it names two repositories: path and url are both given
link-targets = it names more than one of branch, tag, commit and pr
link-path-not-absolute = the path is not a full path
link-path-elsewhere = the path is on another machine; a link opens only what is here
link-url = the url is not an address a link may name; https, http, ssh and git addresses are, and host:path
link-url-password = the url has a password in it; a link is for passing on
link-dash = { $parameter } begins with a dash
link-commit = the commit is not 7 to 64 hex digits
link-pull-request = the pr is not the number of a pull request

## Making a link.

# The operation a refusal names, before a colon.
link-refused = link
link-path-and-url = give path or url, not both
link-which-repository = say which repository: path or url; none is open to be the one
link-repository-not-full = the path of this repository cannot be made a full path
link-repository-not-text = the path of this repository is not text
link-one-target = give at most one of branch, tag and commit
# The error is why the link that was asked for would not be read back.
link-not-a-link = that would not be a link: { $error }

## Registering the application for links: what is wrong with a program or a directory that is given, after its parameter's name.

link-empty = must not be empty
link-path-too-long = it is too long
link-control-characters = must not hold control characters
link-program-not-full = give the full path of the program
link-directory-not-full = give the full path of the directory
link-desktop-equals = a desktop entry cannot name a program with = in its path
link-desktop-percent = a program with % in its path is not found from a desktop entry by every desktop
link-windows-quote = a path on Windows has no double quote in it
link-windows-percent = a program with % in its path cannot be named in the command Windows runs for a link
link-mac-only-bundle = is for Linux; macOS takes the scheme from the application bundle
link-mac-no-program = is no path of a program
link-windows-no-desktop = desktop entries are for Linux; on Windows the registry says what takes the links
link-windows-no-mime = the system's choice is made with xdg-mime, on Linux; on Windows the key of the scheme is that choice

## Registering the application for links: why it was not done.

# The program is its path, cut short when long.
link-no-bundle = { $program } is in no application bundle, and macOS hands links to an application (`cargo xtask bundle` makes { -product }.app)
link-no-application = there is no application at { $path }
link-no-program = there is no program at { $path }
# The program is xdg-mime or lsregister; the error is what the system said.
link-program-not-started = { $program } could not be started ({ $error })
# The status is how the program ended; said is what it wrote to standard error, cut short when long.
link-program-failed = { $program } ended with { $status }: { $said }
link-running-program = the running program cannot be told; say which: exe
link-other-system = on this system it is the installer of the application that registers it for its links
link-no-applications-directory = where desktop entries go cannot be told; say where: applications_dir
# The error is what the system said.
link-directory-not-made = the directory for desktop entries cannot be made: { $error }
link-entry-not-written = the entry cannot be written: { $error }
link-entry-not-placed = the entry cannot be put in its place: { $error }
link-key-not-removed = the key that is there cannot be removed: { $error }
link-key-not-written = the key cannot be written: { $error }
link-no-registry = this system has no registry
