### Finding a file.

# The field's placeholder; "uigraph" is an example of what to type.
find-file-placeholder = Letters of the path, in order: uigraph
# The answer to what is typed is still out.
find-file-looking = Looking…

## The line under the list: how many files match, of how many ($scanned) were looked at.

find-file-no-files = There are no files in the last commit.
find-file-none-match =
    None of { $scanned ->
        [one] 1 file
       *[other] { $scanned } files
    } has these letters in order.
# $shown is how many the list shows, $matched how many match.
find-file-best-match =
    The best { $shown } of { $matched } that match, among { $scanned ->
        [one] 1 file
       *[other] { $scanned } files
    }.
find-file-match =
    { $matched } of { $scanned ->
        [one] 1 file
       *[other] { $scanned } files
    } match.
# Nothing is typed yet.
find-file-all =
    { $scanned ->
        [one] 1 file
       *[other] { $scanned } files
    } in the last commit. Type letters of a path.
# Put after the rest of the line, with a blank between.
find-file-more = The tree has more files than were looked at.
