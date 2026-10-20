### What a provider's API answered.

## Why a provider did not answer with what was asked.

# The token was refused; what the provider said about it is in the brackets.
api-unauthorized = the provider refused the token ({ $said }): sign in again
api-not-found = not found at the provider, or not visible to this account
api-limited = the provider's allowance of requests is spent
# The reset is when requests are allowed again (2026-10-12T08:30:00Z).
api-limited-until = the provider's allowance of requests is spent until { $reset }
# The status is the HTTP status code; the message is what the provider said.
api-status = the provider answered { $status }: { $message }
# The error is what the reader of the answer said.
api-malformed = the provider's answer cannot be read: { $error }

## What cannot be asked.

api-address-with-question = an API address with a question in it
api-address-not-on-api = an address that is not on the account's API
# A GraphQL answer with neither data nor a message in its errors.
api-answer-without-data = an answer without data
