### Ignore rules and templates.

## Which input or operation was refused: the first words of the error.

ignore-what-pattern = pattern
ignore-what-repository = repository
ignore-what-ignore = ignore
ignore-what-paths = paths
ignore-what-path = path

## Why.

ignore-one-line = must be one line
ignore-empty = must not be empty
ignore-comment = a line that starts with # is a comment and ignores nothing; write \# for a name that starts with one
# The pattern, quoted.
ignore-matches-nothing = { $kept } alone matches nothing
ignore-bare = bare repositories have no worktree
ignore-no-place = there is no place for an ignore file of your own: core.excludesFile is not set and there is no home directory
ignore-no-nul = a path has no NUL in it

## The steps of adding a pattern; each is followed by what failed.

ignore-reading = reading the ignore file
ignore-creating-folder = creating the ignore file's folder
ignore-writing = writing the ignore file

## The templates a new repository's .gitignore can start from: what each
## is for, as people say it.

ignore-template-rust = Rust
ignore-template-node = Node.js
ignore-template-python = Python
ignore-template-go = Go
ignore-template-java = Java
ignore-template-c-cpp = C and C++
ignore-template-dotnet = .NET
ignore-template-editors-and-systems = Editors and operating systems

## A template that was asked for and is not there.

ignore-what-template = gitignore template
# The name asked for, quoted; and the names there are, joined with commas.
ignore-no-template = there is no template { $name }; there are { $names }
