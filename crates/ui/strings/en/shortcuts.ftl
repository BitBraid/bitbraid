### The keyboard shortcuts sheet.

## The sheet and the list of keys both.

shortcuts-title = Keyboard shortcuts
shortcuts-close = Close

## The list of keys: a dialog over the window.

shortcuts-filter = Filter
# Between two keys of one action.
shortcuts-or = or
shortcuts-no-key-does-that = No key does that.

## The sheet: what the application is, every key, and changing a key.

# The application's name and version.
shortcuts-product-version = { -product } { $version }
shortcuts-what = A git client.
# git is the git that is run, as shortcuts-git or shortcuts-git-at say it.
shortcuts-what-runs = A git client. Runs { $git }.
shortcuts-git = git { $version }
# path is where the git program is.
shortcuts-git-at = git { $version } at { $path }
shortcuts-trademarks = Git and the Git logo are trademarks of Software Freedom Conservancy, Inc.; this project is not affiliated with or endorsed by the Git Project.
# key is a key as it is typed, such as ctrl+p.
shortcuts-search = Search: what it does, or a key such as { $key }. ↓ and Enter change a key
shortcuts-reset-all = All defaults again…
shortcuts-reset-all-tooltip = Take every key of your own out of keybindings.toml
# Headings, shown in capitals.
shortcuts-complaints = In your keybindings.toml
shortcuts-pointer = With the pointer
shortcuts-nothing-found = Nothing has that name or that key.
# file is the path of keybindings.toml; key a key as it is written there.
shortcuts-how-to-change = A key is changed by opening its row: click it, or go to it with ↓ and press Enter. What is changed here is written into { $file }, which can be edited by hand as well, by the name in the middle column: [keys] and then "remote.pull" = "{ $key }", a list for several keys, an empty list for none. The file is read again when it changes.
shortcuts-no-place = This system gave no place to keep files in, so the keys cannot be changed.

## A row: an action and its keys. In what follows action is what the action
## does, and key a key as it is shown.

# The action's keys are the user's own.
shortcuts-yours = yours
shortcuts-yours-no-key = yours: no key

## A row that is open: what to do with its keys.

shortcuts-choosing = The keys of “{ $action }”. Enter and then the new key puts it in place of the ones there are.
shortcuts-new-key = New key (Enter)
shortcuts-one-more-key = One more key (A)
shortcuts-no-key = No key (N)
shortcuts-default-again = The default again (R)
shortcuts-leave-esc = Leave (Esc)
shortcuts-leave = Leave

## A row that is open: the key to be given to the action is waited for.

shortcuts-press-now = Press the key for “{ $action }” now.
shortcuts-waiting-add = It is added to the keys there are. Whatever is pressed is taken, Esc and Enter too; nothing is saved before it is shown here.
shortcuts-waiting-replace = It takes the place of the keys there are. Whatever is pressed is taken, Esc and Enter too; nothing is saved before it is shown here.

## A row that is open: a key was pressed, and is to be saved or not.

# After the key that was pressed.
shortcuts-pressed-add = one more key for “{ $action }”
shortcuts-pressed-replace = for “{ $action }”
shortcuts-already-this = It is a key of this action already
shortcuts-free = No other action has it here
shortcuts-save-take = Save and take it (Enter)
shortcuts-save = Save (Enter)
shortcuts-press-again = Press again (Backspace)

## What was done.

# count is how many keys the action has now.
shortcuts-saved =
    { $count ->
        [0] { $key } is the key of “{ $action }” now
        [one] { $key } is the key of “{ $action }” now
       *[other] { $key } is a key of “{ $action }” now
    }
shortcuts-unbound = “{ $action }” has no key now
# count is how many keys the action has by default.
shortcuts-reset-done =
    { $count ->
        [one] “{ $action }” has its default key again
       *[other] “{ $action }” has its default keys again
    }

## Every key its default again.

shortcuts-all-default-already = Every key is the default already
shortcuts-reset-all-title = Give every key its default again?
# count is how many actions have keys of the user's own; file is the path of keybindings.toml.
shortcuts-reset-all-description =
    { $count ->
        [one] The keys of { $count } action of your own are taken out of { $file }. What else is in the file stays.
       *[other] The keys of { $count } actions of your own are taken out of { $file }. What else is in the file stays.
    }
shortcuts-all-default-again = Every key is the default again
