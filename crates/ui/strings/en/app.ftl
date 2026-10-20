### The application: the git it runs, a repository opened read-only.

## The git that is run.

# When the executable could not be asked what it is. $error is why, as the system or git said it.
app-git-cannot-run-warning = Git cannot be run: { $error }. Nothing that needs it will work; choose another executable in the preferences.
# A git that works but does without some things. $lacks lists them, put together with semicolons.
app-git-does-without = Git { $version } does without: { $lacks }.
# A git older than the oldest one everything works with ($minimum).
app-git-too-old = Git { $version } is older than { $minimum } and not everything will work. It does without: { $lacks }.
# Stands for the version in the two warnings above when git did not say it.
app-git-version-unknown = this
# The status bar: the git shipped with the application.
app-git-short-shipped = git { $version } (shipped)
# The status bar.
app-git-short = git { $version }
# The status bar, when there is no git to run.
app-git-short-none = no git
# The preferences: the git shipped with the application, and where it is.
app-git-in-use-shipped-at = In use: git { $version }, shipped with { -product }, at { $path }
app-git-in-use-shipped = In use: git { $version }, shipped with { -product }
app-git-in-use-at = In use: git { $version } at { $path }
app-git-in-use = In use: git { $version }
# The preferences, when the executable could not be asked. $error is why.
app-git-cannot-run = Git cannot be run: { $error }
app-git-asking = Asking git what version it is…

## A repository opened read-only (a bare one).

app-read-only-heading = Read-only
# Under the heading: why, and the way out.
app-read-only-explanation = A bare repository: nothing here changes it. Clone it to work in the clone.
# The same in a line, for a menu, a tooltip or a notice.
app-read-only-in-short = Read-only: this is a bare repository
