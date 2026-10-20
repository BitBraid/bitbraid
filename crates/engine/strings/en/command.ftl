### The registry and what any command can say.
command-unknown = unknown command { $id }
command-invalid-param = invalid parameter { $name }: { $detail }
command-no-repository = no repository is open
# A bare repository refused a command that would change it; the command is its label in quotes and its id in brackets.
command-read-only = this is a bare repository, which is opened read-only: { $command } would change it. Clone it and work in the clone
# The error is what the JSON writer said.
command-serialize = cannot serialise the result: { $error }
# A parameter the command needs was not given.
command-param-missing = missing

## Registering a command: errors a developer of a command sees.

command-duplicate = command id { $id } is already registered
command-malformed-id = command id { $id } must be lowercase dotted words, e.g. `repo.open`
