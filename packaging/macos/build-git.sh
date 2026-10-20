#!/bin/sh
# Builds the git that the macOS bundle ships:
# git's release, relocatable, installed into a directory of its own for
# `cargo xtask bundle --git <dir>`, which puts it in
# BitBraid.app/Contents/Resources/git.
#
#   packaging/macos/build-git.sh <git-x.y.z.tar.xz> <out dir>
#
# The tarball is git's release from kernel.org, its checksum checked by
# the caller (.github/workflows/release.yml). On macOS it is built once
# for Apple silicon and once for Intel, its Rust part for the same
# processor as the rest, and each program of the two is joined into one
# with lipo. Elsewhere it is built once, for this machine: how the build
# is tried on Linux (`BITBRAID_RELOCATABLE_GIT=<out dir> cargo test -p
# bitbraid-cli --test suite shipped_git::`).
#
# RUNTIME_PREFIX: git finds its programs, templates and system
# configuration from where its executable is, wherever the bundle is put.
# Built without gettext, Perl, Python and Tcl/Tk, so the programs that
# need them (git-gui, gitk, git-svn, send-email) are left out; without the
# dashed links of the built-in commands, which a program never needs
# (`git <command>` runs them) and which copied as files take 676 MB; and
# with the links that are left made symbolic, which the bundle keeps.
# On macOS nothing of MacPorts', Fink's or Homebrew's is linked, which
# git's build takes where it finds them (NO_DARWIN_PORTS, NO_FINK,
# NO_HOMEBREW): the Macs it goes to need not have them. Each program is
# checked for that, and for both processors, before it is done.
#
# Beside bin/, libexec/ and share/ go git's licences, as its source has
# them (COPYING, LGPL-2.1, and those of the reftable library and the SHA-1
# collision detection), and SOURCE.txt, which says which release it was
# built from and where that is.
#
# Sources: git's INSTALL and the comments of its Makefile and
# config.mak.uname (RUNTIME_PREFIX, SKIP_DASHED_BUILT_INS,
# INSTALL_SYMLINKS, NO_GETTEXT, NO_PERL, NO_PYTHON, NO_TCLTK, RUST_TARGETS,
# NO_DARWIN_PORTS, NO_FINK, NO_HOMEBREW; read, not copied); lipo(1)
# (-verify_arch), otool(1) (-L); clang's -arch and -mmacosx-version-min.

set -eu

if [ $# -ne 2 ]; then
    echo "usage: $0 <git-x.y.z.tar.xz> <out dir>" >&2
    exit 2
fi
tarball=$(cd "$(dirname "$1")" && pwd)/$(basename "$1")
out=$2
if [ -e "$out" ]; then
    echo "$0: $out exists" >&2
    exit 1
fi
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

# One build, installed under $work/install-$1/git.
build() {
    name=$1
    shift
    mkdir -p "$work/src-$name"
    tar -xf "$tarball" -C "$work/src-$name" --strip-components=1
    jobs=$(getconf _NPROCESSORS_ONLN 2>/dev/null || echo 2)
    make -C "$work/src-$name" -j"$jobs" \
        prefix=/git DESTDIR="$work/install-$name" \
        RUNTIME_PREFIX=YesPlease \
        SKIP_DASHED_BUILT_INS=YesPlease \
        INSTALL_SYMLINKS=YesPlease \
        NO_GETTEXT=YesPlease NO_PERL=YesPlease NO_PYTHON=YesPlease NO_TCLTK=YesPlease \
        "$@" \
        install
}

if [ "$(uname -s)" = Darwin ]; then
    # What the Rust part is built for; the C part has -mmacosx-version-min.
    export MACOSX_DEPLOYMENT_TARGET=11.0
    for arch in arm64 x86_64; do
        case $arch in
            arm64) rust=aarch64-apple-darwin ;;
            *) rust=$arch-apple-darwin ;;
        esac
        build "$arch" \
            RUST_TARGETS="$rust" \
            NO_DARWIN_PORTS=YesPlease NO_FINK=YesPlease NO_HOMEBREW=YesPlease \
            CFLAGS="-O2 -arch $arch -mmacosx-version-min=11.0" \
            LDFLAGS="-arch $arch -mmacosx-version-min=11.0"
    done
    src=$work/src-arm64
    first=$work/install-arm64/git
    second=$work/install-x86_64/git
    mkdir -p "$out"
    # The same tree; each Mach-O file of the two joined into one.
    (cd "$first" && find . -type d) | while IFS= read -r dir; do
        mkdir -p "$out/$dir"
    done
    (cd "$first" && find . ! -type d) | while IFS= read -r file; do
        if [ -L "$first/$file" ]; then
            cp -P "$first/$file" "$out/$file"
        elif file -b "$first/$file" | grep -q '^Mach-O'; then
            lipo -create "$first/$file" "$second/$file" -output "$out/$file"
            chmod "$(stat -f %Lp "$first/$file")" "$out/$file"
        else
            cp -p "$first/$file" "$out/$file"
        fi
    done
    # Every program for both processors, and linking nothing but the
    # system's own libraries.
    problems=$work/problems
    : >"$problems"
    find "$out" -type f | while IFS= read -r file; do
        file -b "$file" | grep -q '^Mach-O' || continue
        for arch in arm64 x86_64; do
            lipo "$file" -verify_arch "$arch" || echo "$file: no $arch" >>"$problems"
        done
        otool -L "$file" | sed -n 's/^[[:space:]][[:space:]]*\([^ ]*\) (.*/\1/p' |
            while IFS= read -r lib; do
                case $lib in
                    /usr/lib/* | /System/Library/*) ;;
                    *) echo "$file: links $lib" >>"$problems" ;;
                esac
            done
    done
    if [ -s "$problems" ]; then
        echo "$0: not a git every Mac can run:" >&2
        cat "$problems" >&2
        exit 1
    fi
else
    build native CFLAGS="-O2"
    src=$work/src-native
    mkdir -p "$(dirname "$out")"
    mv "$work/install-native/git" "$out"
fi

# Its licences, and where its source is.
cp "$src/COPYING" "$src/LGPL-2.1" "$out/"
cp "$src/reftable/LICENSE" "$out/LICENSE-reftable"
cp "$src/sha1dc/LICENSE.txt" "$out/LICENSE-sha1dc.txt"
version=$("$out/bin/git" --version | sed 's/^git version //')
cat >"$out/SOURCE.txt" <<EOF
git $version, as BitBraid carries it

Built from git's release, unmodified:
https://www.kernel.org/pub/software/scm/git/git-$version.tar.xz

by packaging/macos/build-git.sh in BitBraid's source, which names every
option of the build: https://github.com/BitBraid/bitbraid

git is under the GNU General Public License, version 2 (COPYING), with
parts of it under the GNU Lesser General Public License, version 2.1
(LGPL-2.1); its reftable library and its SHA-1 collision detection have
licences of their own (LICENSE-reftable, LICENSE-sha1dc.txt).
EOF

"$out/bin/git" --version
du -sh "$out"
