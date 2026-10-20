### Sparse checkout.

## The bar over the changes while only part of the files is checked out.

# What is checked out; `what` is one of the three below, `outside` how many tracked files are left out.
sparse-bar-text = Sparse checkout: { $what }
sparse-bar-text-left-out =
    { $outside ->
        [one] Sparse checkout: { $what }; { $outside } file left out
       *[other] Sparse checkout: { $what }; { $outside } files left out
    }
sparse-by-patterns =
    { $count ->
        [one] by { $count } pattern
       *[other] by { $count } patterns
    }
sparse-top-only = only the files at the top
# The first folders of the cone, put together with commas, and how many more there are.
sparse-folders-and-more = { $folders } and { $more } more
# The bar's tooltip, when the cone has no folders.
sparse-top-files = The files at the top of the worktree
sparse-check-out-everything = Check out everything

## A folder's menu in the lists of files.

sparse-menu = Sparse checkout
sparse-only-this-folder = Only this folder
sparse-add-this-folder = Add this folder
