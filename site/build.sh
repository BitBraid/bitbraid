#!/bin/sh
# Assembles the website into a directory, ready to be served: site/public
# as it is, and the logo files from brand/, which are kept in one place
# only.
#
# usage: site/build.sh <directory> [<version>]
#
# With the version of the latest release, each download link points at its
# file of that release (the link's data-file, with {v} for the version, as
# the release workflow names the files); without one, at the release page.
#
# The directory is made anew; one that exists is replaced only if this
# script made it.

set -eu

if [ $# -lt 1 ] || [ $# -gt 2 ]; then
    echo "usage: $0 <directory> [<version>]" >&2
    exit 2
fi
out=$1
version=${2:-}
site=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$site")

case $version in
'' | *[!0-9A-Za-z.+-]*)
    if [ -n "$version" ]; then
        echo "$0: $version is not a version" >&2
        exit 2
    fi
    ;;
esac

if [ -e "$out" ]; then
    if [ ! -e "$out/.bitbraid-site" ]; then
        echo "$0: $out exists and was not made by this script" >&2
        exit 1
    fi
    rm -rf "$out"
fi
mkdir -p "$out/brand"
cp -R "$site/public/." "$out/"
for file in logo.svg logo-on-light.svg mark.svg mark-on-light.svg icon.svg; do
    cp "$root/brand/$file" "$out/brand/"
done

releases=https://github.com/BitBraid/bitbraid/releases
if [ -n "$version" ]; then
    sed -e "s|href=\"$releases/latest\" data-file=\"\([^\"]*\)\"|href=\"$releases/download/v$version/\1\"|g" \
        -e "s|{v}|$version|g" "$site/public/index.html" >"$out/index.html"
else
    sed -e 's| data-file="[^"]*"||g' "$site/public/index.html" >"$out/index.html"
fi

: > "$out/.bitbraid-site"
echo "$out"
