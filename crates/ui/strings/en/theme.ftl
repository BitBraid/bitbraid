### Themes.

## Our themes, as the menus and the preferences name them.

theme-dark = Dark
theme-light = Light
theme-contrast = High contrast
theme-dusk = Dusk
theme-ember = Ember
theme-graphite = Graphite
theme-paper = Paper
theme-sage = Sage

## Why a theme file of one's own cannot be used at all.

theme-not-toml = it is not TOML
theme-name-not-text = name has to be a text
theme-name-unfit = the name is too long or has a control character in it
# name is the name the file gives, between quotes.
theme-name-taken = the name { $name } is one of the built-in themes'
theme-mode = mode is "dark" or "light"
theme-built-in-unreadable = the built-in themes cannot be read

## What in a theme file was left out. key is what the file names, such as
## background or syntax.keyword; value is what it gives, between quotes.

theme-not-a-colour = { $key }: { $value } is not a colour
theme-colour-is-text = { $key }: a colour is a text such as "#1a2b3c"
theme-no-colour = { $key }: there is no colour of that name
theme-no-syntax-colour = { $key }: there is no syntax colour of that name
theme-colour-or-table = { $key }: a colour is a text such as "#1a2b3c", or a table with color, font_style and font_weight
theme-font-style = { $key }: font_style is "normal", "italic" or "underline"
theme-font-weight = { $key }: font_weight is one of 100, 200 and so on to 900
theme-colors-not-a-table = colors has to be a table
# most is the most lane colours a theme may name.
theme-lanes-count = lanes: a list of 1 to { $most } colours
# number is the place of the colour in the list, from 1.
theme-lane-not-a-colour = lanes: number { $number } is not a colour
theme-lanes-not-a-list = lanes has to be a list of colours
theme-syntax-not-a-table = syntax has to be a table
theme-not-a-setting = { $key }: not something a theme sets, or not a value it can have

## A file of the folder of themes that cannot be read; file_name is its
## name.

theme-file-too-large = { $file_name }: too large to be a theme
theme-file-not-text = { $file_name }: it is not text
# name is the theme's name, between quotes.
theme-file-name-taken = { $file_name }: there is a theme called { $name } already

## The example theme file written for the user to start from: TOML, its
## comments in the user's language.

theme-example-head =
    # A theme of your own. It names what it changes; everything else is the
    # built-in theme of the same mode. The file is read again when it changes.
    name = "Example"
    mode = "dark"            # "dark" or "light": which built-in theme it starts from
    # radius = 6              # how round corners are, 0 to 40
    # shadow = true
    # "font.family" = "Inter"
    # "mono_font.family" = "JetBrains Mono"

    # The colours of the graph's lanes, in turn: 1 to 16 of them.
theme-example-terminal = # The terminal inside the window.
theme-example-syntax =
    # The colours of code in the diff and in blame. A name can also be a table:
    # comment = {"{"} color = "#6a9955", font_style = "italic", font_weight = 400 {"}"}
