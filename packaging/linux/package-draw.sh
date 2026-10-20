#!/bin/sh
# Run by package-test.sh as an ordinary user: starts a program in a
# headless Wayland compositor (cage, on wlroots' headless backend, drawing
# with pixman; the program draws with the software Vulkan driver), takes
# the screen with grim after 30 seconds, and fails unless the program is
# still running and the screen shows something drawn. The picture is kept
# in /out/<picture>.png when /out is there to write to.
#
#   package-draw.sh <picture> <program> [<argument>...]

set -eu

picture=$1
shift
HOME=$(mktemp -d)
XDG_RUNTIME_DIR=$(mktemp -d)
chmod 700 "$XDG_RUNTIME_DIR"
export HOME XDG_RUNTIME_DIR
export WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1

cage -- "$@" >"$HOME/cage.log" 2>&1 &
cage=$!
sleep 30
stop() {
    kill "$cage" 2>/dev/null || true
    wait "$cage" 2>/dev/null || true
}
fail() {
    stop
    echo "--- what it wrote:"
    cat "$HOME/cage.log"
    echo "FAILED: $*" >&2
    exit 1
}

socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -name 'wayland-[0-9]*' ! -name '*.lock' -exec basename {} \; | head -n 1)
[ -n "$socket" ] || fail "the compositor did not start"
kill -0 "$cage" 2>/dev/null || fail "it stopped"
WAYLAND_DISPLAY=$socket grim -t ppm "$HOME/screen.ppm" || fail "grim"
if [ -d /out ] && [ -w /out ]; then
    WAYLAND_DISPLAY=$socket grim "/out/$picture.png" || true
fi
stop

# A screen with nothing drawn has one value in it after the header
# ("P6", the size, 255).
values=$(tail -c +16 "$HOME/screen.ppm" | od -An -v -tu1 | tr -s ' ' '\n' | sort -u | wc -l)
[ "$values" -gt 32 ] || fail "nothing drawn ($values values on the screen)"
echo "drawn: $values values on the screen"
