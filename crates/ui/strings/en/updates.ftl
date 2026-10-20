### Updates: the preferences' section and its notices.

# What the switch "Look for updates" says under its name: what is sent when it is on, and to whom.
updates-switch-note = Off: { -product } asks nobody whether there is a newer version. On: when it starts and once a day while it runs, it sends one request to GitHub for the small file published with each release of { -product }, which names the newest version. The request carries { -product }'s name and version and nothing else about you, though GitHub sees this machine's network address, as any site does. When a newer version is out it says so; only when you ask it to install it does it download the package for this system, and it installs nothing whose signature does not match the key built into { -product }.

## The row about this copy: how it was installed and what the updater
## does with it, then, after a blank, what the last look found.

updates-copy-reading = Reading how this copy was installed…
updates-copy-no-key = This build has no update key.
# $path is where the AppImage file is.
updates-copy-appimage = This copy is the AppImage { $path }: a newer version replaces it, and the old one is kept until the new one starts.
# $path is where the application bundle is.
updates-copy-bundle = This copy is the application { $path }: a newer version replaces it, and the old one is kept until the new one starts.
updates-copy-windows-installer = This copy was installed by the Windows installer: a newer version's installer is run, and { -product } closes for it.
updates-copy-not-itself = This copy does not update itself.
# $version is the version being downloaded, $percent how much of it is here.
updates-downloading = Downloading { -product } { $version }: { $percent } %.
updates-installed-restart = { -product } { $version } is installed: restart to use it.
updates-installer-runs = The installer of { -product } { $version } runs.
updates-looking = Looking for a newer version…
# $failed is why, as a sentence.
updates-last-look-failed = The last look failed. { $failed }
# Put after what the last look found; $when is how long ago, as in "5 minutes ago".
updates-looked = Looked { $when }.
updates-not-looked-yet = Nothing was looked for yet.
updates-off-nothing-asked = Updates are off: nothing is asked.

## What the last look found, in the row and in a notice. $latest is the
## newest version published, $current the version that runs.

updates-newer-out = { -product } { $latest } is out; this is { $current }.
updates-newest = { -product } { $current } is the newest version.

## Notices.

# Asked to look or install while updates are off.
updates-off-notice = Updates are off, so { -product } asks nobody about newer versions. They are turned on in Preferences, General, Updates.
updates-turn-on-and-look = Turn on and look
updates-feed-unreadable = The feed's answer could not be read.
# The buttons of the notice of a newer version.
updates-install = Install
updates-what-is-new = What is new
updates-open-page = Open its page
updates-installed-notice = { -product } { $version } is installed. Restart { -product } to use it.
updates-restart-now = Restart now
updates-download-stopped = The download of the new version was stopped; nothing was changed.
