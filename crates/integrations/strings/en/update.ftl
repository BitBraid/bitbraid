### The updater: looking for a newer version, and installing it.

## Parameters of update.install

update-param-version = the version that was offered (update.check's latest): nothing is installed when the feed now names another

## The commands: what the palette and the command line call each, and what it does.

update-status = Update status
update-status-description = Say how this copy of { -product } was installed and what the updater can do with it, asking nobody; answers current, key (bool: whether the build has an update key), why_no_key, installation (appimage, bundle, installer, package, elsewhere), path, installs_itself (bool), why_not and feed
update-check = Check for updates
update-check-description = Ask the feed of { -product }'s releases (one HTTPS request to the project's latest release) which version is the newest; answers current, latest, newer (bool), page, installation (appimage, bundle, installer, package, elsewhere), path, installs (bool: whether update.install can install it here), why_not and package. A build without an update key sends nothing
update-install = Install update
update-install-description = Download the newer version's package for this system, check its size, SHA-256 and minisign signature against the key built into { -product }, and install it the system's way: an AppImage or a macOS bundle replaced (the old copy kept until the new one starts), the Windows installer run; a copy installed by a package manager or unpacked by hand is left alone. Nothing that fails a check is installed; answers installed, path, then (restart or quit) and page; params: version

## What the commands say

# What a refusal of the updater is about: "refused: update: …".
update-what = update
# Refusals of the version given to update.install, after the parameter's name.
update-param-not-text = is a text
update-param-not-version = is not a version
# The progress of update.install while the package comes, when no version was given.
update-downloading = Downloading { -product }
# The same, with the version that was offered.
update-downloading-version = Downloading { -product } { $version }

## Why the updater did not do what it was asked

update-no-key = this build of { -product } has no update key, so it neither looks for updates nor installs them; the builds on { -product }'s release page have one
# The reason is what the key's decoder said.
update-bad-key = the update key built into this build of { -product } cannot be read ({ $why }), so it neither looks for updates nor installs them
update-feed-unreadable = the feed of releases at { $url } could not be read: { $why }
# The version is the one that runs.
update-newest = { -product } { $version } is the newest version: there is nothing to install
update-changed = the feed now names { -product } { $now }, not { $offered }: look for updates again
update-download-failed = the download of { -product } { $version } failed: { $why }
# The refusal is one of the checks below (update-refusal-…).
update-download-refused = the download of { -product } { $version } was not installed: { $refusal }
update-install-failed = { -product } { $version } could not be installed: { $why }
# The user stopped it.
update-stopped = stopped

## Reading the feed and downloading: what went wrong

update-redirect-nowhere = a redirect to nowhere
update-redirect-insecure = a redirect from HTTPS to plain HTTP
update-redirect-too-many = too many redirects
update-feed-too-large = it is larger than a feed is
# The status is the HTTP status the feed's server answered with.
update-feed-status = it answered with status { $status }
update-download-status = the server answered with status { $status }
update-download-unwritten = it could not be written to disk: { $error }
# The download stopped part way: the server went away or the line dropped. $got is how many bytes came, $expected how many the package has.
update-download-cut = the connection ended after { $got } of its { $expected } bytes

## What this copy can do with a release

update-no-packages-for-system = { -product } makes no packages for this system
# The kind is one of the names below (update-kind-…), the system Linux, macOS or Windows, the arch the processor's word (x86_64, aarch64).
update-no-package-in-release = the release has no { $kind } for { $system } on { $arch }: download it from its page
update-not-itself = this copy does not update itself

## The kinds of package, as a sentence names them

update-kind-appimage = AppImage
update-kind-deb = Debian package
update-kind-rpm = RPM package
update-kind-tarball = tarball
update-kind-app = application bundle
update-kind-dmg = disk image
update-kind-msi = Windows installer
update-kind-zip = zip

## Why a feed cannot be read

# The key is the name of a field of the feed (version, page, name, url…).
update-feed-no-field = it has no { $key }
update-feed-not-text = its { $key } is not a text
update-feed-empty-or-long = its { $key } is empty or too long
update-feed-version-empty-or-long = its version is empty or too long
# The version is in quotes, as the feed has it.
update-feed-not-a-version = its version { $version } is not one
# $what is the name of the field: url or page.
update-feed-not-an-address = its { $what } is not an address
update-feed-not-https = its { $what } is not an HTTPS address
update-feed-package-not-object = a package is not an object
# A package's system, arch and kind are the feed's words for them (linux x86_64 appimage); the error is one of the messages below.
update-feed-in-package = the { $system } { $arch } { $kind } package: { $error }
update-feed-bad-name = its name is not a file's name
update-feed-bad-size = its size is not a number of bytes it may have
update-feed-bad-sha256 = its sha256 is not 64 hexadecimal digits
update-feed-bad-signature = its signature is not one minisign wrote
update-feed-not-json = it is not JSON
update-feed-not-object = it is not a JSON object
# The schema is the number of the feed's format.
update-feed-later-format = it is of a later format ({ $schema }) than this version of { -product } reads
update-feed-schema = its schema is not 1
update-feed-no-packages = it has no list of packages
update-feed-too-many-packages = it lists more packages than a release has

## Why a copy does not update itself

update-translocated = macOS runs this copy from a place of its own, where it cannot be replaced: move { -product } to the Applications folder, start it from there and update again
# The path is the running program.
update-package-manager = { -product } was installed by the system's package manager (it runs from { $path }): install the new version with it, as its other programs are
update-nowhere = { -product } cannot tell where it runs from, and replaces nothing: download the new version from its page
update-elsewhere = { -product } runs from { $path }, which it does not replace by itself: download the new version from its page

## Putting a package in place: what went wrong

update-no-download-folder = { -product } has no folder to download the new version into
update-no-name = the running copy has no name to put the new one under
update-folder-not-made = the folder { $folder } could not be made: { $error }
update-appimage-folder-unwritable = { -product } cannot write in the folder of its AppImage ({ $folder }): { $error }
update-not-written = { $path } could not be written: { $error }
update-not-made = { $path } could not be made: { $error }
update-in-the-way = { $path } is in the way and could not be deleted: { $error }
update-appimage-not-runnable = the new AppImage could not be made runnable: { $error }
update-appimage-not-kept = the running AppImage could not be kept beside the new one ({ $path }): { $error }
update-appimage-not-replaced = the new AppImage could not be put in place of the old one: { $error }
update-cannot-write-in = { -product } cannot write in { $folder }: { $error }
update-image-no-bundle = the disk image holds no single application bundle
update-package-no-bundle = the package holds no single application bundle
update-bundle-not-set-aside = the running bundle could not be set aside as { $path }: { $error }
update-bundle-not-replaced = the new bundle could not be put in place of the old one: { $error }
update-installer-not-started = the Windows installer could not be started: { $error }
update-not-started-again = this copy is not started again by { -product }
update-new-not-started = the new version could not be started: { $error }
# A system program that could not be run; what is one of the steps below.
update-program-not-started = { $program } could not be started for { $what }: { $error }
# A step that failed, and what its program said on its error output.
update-program-failed = { $what } failed: { $said }

## The steps of putting a macOS bundle in place, named in the two messages above

update-attaching-image = attaching the disk image
update-copying-bundle = copying the bundle out of the disk image
update-detaching-image = detaching the disk image
update-unpacking-bundle = unpacking the bundle

## Checking a download: why it is not installed, said of "the download"

update-refusal-unreadable = its signature in the feed cannot be read
update-refusal-other-key = it was signed with another key than { -product }'s update key
update-refusal-old-kind = its signature is of minisign's older kind, over the file unhashed, which is not taken
update-refusal-too-large = it is larger than the { $expected } bytes the feed gives
update-refusal-size = it is { $got } bytes, not the { $expected } bytes the feed gives
update-refusal-checksum = its SHA-256 is not the one the feed gives
update-refusal-signature = its signature does not hold for { -product }'s update key: it is not the file that was signed
# The comment is the signature's trusted comment, cut short.
update-refusal-not-this-package = its signature is for another release or file ("{ $comment }")
# The write that would go over the size the feed gives fails with this.
update-more-than-given = more came than the feed gives
