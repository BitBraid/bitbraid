### Commit messages: recent ones, trailers.

## The commands: what the palette and the command line call each, and what it does.

message-commit-messages = Recent commit messages
message-commit-messages-description = The messages of the commits made in this worktree lately, newest first and each once, from the reflog of HEAD (also of commits that were undone or amended since); params: limit
message-commit-trailer = Add trailers to a message
message-commit-trailer-description = The message with trailers added as git interpret-trailers adds them; params: message, trailers [{"{"}key, value{"}"}], sign_off (add Signed-off-by for the identity git commits as). Changes nothing
message-commit-trailer-keys = Trailer keys in use
message-commit-trailer-keys-description = The trailer keys of recent commits and of the trailer.* configuration, the configured first, then the most used; params: limit (commits to look at)

## Refusals.

message-trailers-missing = missing: give trailers, or sign_off
