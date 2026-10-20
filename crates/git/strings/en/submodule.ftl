### Submodules.

## Which input or operation was refused: the first words of the error.

submodule-what-submodule = submodule
submodule-what-url = submodule URL
submodule-what-path = submodule path
submodule-what-name = submodule name

## Why it was refused. A path, URL or name quoted in code is quoted in the
## argument.

submodule-bare = a bare repository has no submodules checked out
submodule-not-one = { $path } is not a submodule here
submodule-url-unusable = { $url } cannot be used
submodule-path-outside = { $path } is not a path inside the repository
submodule-already = { $path } is a submodule already
submodule-exists = { $path } exists already
submodule-tracked = { $path } is tracked already
submodule-clone-kept = git still keeps a clone of a submodule that was at { $path }; remove it for good first
submodule-in-conflict = { $path } is in a conflict; resolve that first
submodule-changes-of-its-own = { $path } has changes of its own; they would be lost
submodule-moved = { $path } is at a commit that is not the one recorded; that would be lost
submodule-not-a-name = { $name } is not the name of a submodule
submodule-still-declared = { $name } is a submodule of this repository; remove it instead
submodule-no-clone = git keeps no clone of a submodule called { $name }

## The step of forgetting a removed submodule; it is followed by what
## failed.

submodule-deleting-clone = deleting the clone of a removed submodule
