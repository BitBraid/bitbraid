#!/bin/sh
# Takes the website's screenshots: the application on the demo repository
# (site/demo-repository.sh), in its dark and light themes, drawn in a
# compositor of its own with no display (cage's headless backend) at twice
# the density of a plain screen.
#
# usage: site/screenshots.sh <bitbraid binary> [<logical width>x<height>]
#
# Needs bwrap, cage, grim, wlr-randr, dbus-run-session and ImageMagick's
# `magick`. Writes, under site/public/images, app-{dark,light}.webp at
# twice the density, app-{dark,light}-1440.webp at plain density, and
# social.png, the picture shown with links to the site.
#
# Nothing of the user's is read or written. The application runs in a
# bubblewrap sandbox where /home holds nothing but a scratch home at
# /home/ada (so the status bar shows a tidy path), with a scratch runtime
# directory, gpg home and git configuration, and no display, ssh agent or
# session bus but its own. The scratch directory is removed afterwards.

set -eu

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: $0 <bitbraid binary> [<width>x<height>]" >&2
    exit 2
fi
for tool in bwrap cage grim wlr-randr magick dbus-run-session; do
    command -v "$tool" >/dev/null || { echo "$0: $tool is not on PATH" >&2; exit 1; }
done
[ -x "$1" ] || { echo "$0: $1 is not a program" >&2; exit 1; }

size=${2:-1440x900}
width=${size%x*}
height=${size#*x}
site=$(cd "$(dirname "$0")" && pwd)
out=$site/public/images
mkdir -p "$out"

scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT INT TERM
# The real /home is hidden in the sandbox, so the binary comes along.
cp "$1" "$scratch/bitbraid"

home=/home/ada
repo=$home/code/meridian
# A debug build reads the toolkit's icons from the crate's sources in the
# cargo home (rust-embed); the sandbox may read them, and nothing else there.
cargo_home=${CARGO_HOME:-$HOME/.cargo}

shoot() {
    theme=$1
    run=$scratch/$theme
    mkdir -p "$run/home/.config/bitbraid" "$run/runtime" "$run/gnupg"
    chmod 700 "$run/runtime" "$run/gnupg"
    "$site/demo-repository.sh" "$run/home/code" >/dev/null
    printf 'theme = "%s"\n\n[columns]\nrefs_width = 172.0\n' "$theme" \
        > "$run/home/.config/bitbraid/preferences.toml"
    # The uncommitted work is selected, as on any first open; the panel on
    # the left shows the local and remote branches and the tags.
    printf '[repos."%s"]\nclosed_sections = ["gitflow", "worktrees", "stashes", "pulls", "issues"]\n' \
        "$repo" > "$run/home/.config/bitbraid/state.toml"

    env -u WAYLAND_DISPLAY -u DISPLAY -u SSH_AUTH_SOCK -u DBUS_SESSION_BUS_ADDRESS \
        -u XDG_CONFIG_HOME -u XDG_CACHE_HOME -u XDG_DATA_HOME -u XDG_STATE_HOME \
        HOME="$home" XDG_RUNTIME_DIR="$run/runtime" GNUPGHOME="$run/gnupg" \
        GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1 \
        WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
        SHOT="$run/shot.png" BIN="$scratch/bitbraid" REPO="$repo" \
        MODE="$((width * 2))x$((height * 2))" \
        bwrap --dev-bind / / --tmpfs /home --bind "$run/home" "$home" \
            --ro-bind-try "$cargo_home/registry" "$cargo_home/registry" --chdir "$home" \
            --unshare-pid --die-with-parent \
        dbus-run-session -- cage -- sh -c '
            wlr-randr --output HEADLESS-1 --custom-mode "$MODE" --scale 2
            "$BIN" "$REPO" &
            sleep "${SHOT_DELAY:-10}"
            grim "$SHOT"
            kill $!
        ' >"$run/log" 2>&1 || true
    if [ ! -s "$run/shot.png" ]; then
        echo "$0: no picture for the $theme theme; the compositor said:" >&2
        cat "$run/log" >&2
        exit 1
    fi
    magick "$run/shot.png" -strip -quality 90 -define webp:method=6 "$out/app-$theme.webp"
    magick "$run/shot.png" -strip -resize 50% -quality 90 -define webp:method=6 \
        "$out/app-$theme-1440.webp"
    echo "$out/app-$theme.webp"
}

shoot dark
shoot light
# The picture that links to the site show (1200 x 630): the top left of the
# dark one, where the graph and the branches are.
magick "$scratch/dark/shot.png" -strip -crop "$((width * 2 * 1200 / 1440))x$((width * 2 * 630 / 1440))+0+0" \
    -resize 1200x630 -define png:compression-level=9 "$out/social.png"
echo "$out/social.png"
