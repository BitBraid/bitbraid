#!/bin/sh
# Installs and starts BitBraid's Linux packages in a container of a
# distribution. `cargo xtask package-test` runs it, with the
# packages in /packages, this directory in /test and a directory for the
# pictures in /out:
#
#   package-test.sh deb|rpm|arch <version> <system>
#
# The system's package is installed (the .deb, the .rpm, or the PKGBUILD
# built by makepkg from the tarball), asked for its version, checked for
# the libraries it needs, its desktop entries and its AppStream metadata
# validated, and started with a software Vulkan driver: in a virtual X
# display, where its window has to come up and stay, and in a headless
# Wayland compositor, where the screen has to show it drawn
# (package-draw.sh; the picture is kept as /out/<system>-package.png).
# Then the AppImage the same way, the tarball where it was unpacked, and
# the package is taken out again.
#
# Under Xvfb the window comes up but shows no frame (the toolkit presents
# nothing there); drawing is held on Wayland.

set -eu

family=$1
version=$2
system=$3
packages=/packages
arch=x86_64

say() { printf '\n--> %s\n' "$*"; }
fail() {
    printf 'FAILED: %s\n' "$*" >&2
    exit 1
}

# Runs a download from the distribution's mirrors up to three times: a
# mirror that fails once is not the package's failure.
retry() {
    n=1
    until "$@"; do
        [ $n -lt 3 ] || fail "$1 failed three times"
        echo "$1 failed; again in 15 seconds"
        sleep 15
        n=$((n + 1))
    done
}

# Starts a program in a virtual X display with a scratch home, once the
# display answers; its window, named BitBraid, has to come up within 60
# seconds and be there 10 seconds later with the program still running.
start() {
    log=$(mktemp)
    xlog=$(mktemp)
    # The script given to sh is its own, expanded there (SC2016).
    # shellcheck disable=SC2016
    if HOME=$(mktemp -d) XDG_RUNTIME_DIR=$(mktemp -d) LOG=$log \
        xvfb-run -a -e "$xlog" -s '-screen 0 1440x900x24' sh -c '
            n=0
            until xwininfo -root >/dev/null 2>&1; do
                [ $n -lt 20 ] || { echo "the X server did not answer"; exit 1; }
                sleep 1
                n=$((n + 1))
            done
            "$@" >"$LOG" 2>&1 &
            pid=$!
            n=0
            while [ $n -lt 60 ]; do
                if xwininfo -root -tree | grep -q "\"BitBraid\""; then
                    sleep 10
                    kill -0 $pid 2>/dev/null || { echo "it stopped after its window came up"; exit 1; }
                    xwininfo -root -tree | grep "\"BitBraid\""
                    kill $pid
                    wait $pid 2>/dev/null || true
                    exit 0
                fi
                kill -0 $pid 2>/dev/null || { echo "it stopped before a window came up"; exit 1; }
                sleep 1
                n=$((n + 1))
            done
            echo "no window after 60 seconds"
            kill $pid
            exit 1' sh "$@"; then
        rm -f "$log" "$xlog"
    else
        echo "--- what it wrote:"
        cat "$log"
        echo "--- what the X server wrote:"
        cat "$xlog"
        fail "$* did not come up"
    fi
}

case $family in
deb)
    export DEBIAN_FRONTEND=noninteractive
    retry apt-get update -qq
    say "apt-get install ./bitbraid_${version}_amd64.deb"
    dpkg-deb --info "$packages/bitbraid_${version}_amd64.deb"
    retry apt-get install -y -qq "$packages/bitbraid_${version}_amd64.deb" >/dev/null
    say "tools for the checks"
    retry apt-get install -y -qq --no-install-recommends xvfb xauth x11-utils \
        mesa-vulkan-drivers libegl1 fonts-dejavu-core desktop-file-utils \
        appstream cage grim xwayland >/dev/null
    licences=/usr/share/doc/bitbraid
    ;;
rpm)
    rpmfile=$packages/bitbraid-${version}-1.$arch.rpm
    say "rpm -K: the package's digests"
    rpm -K "$rpmfile"
    rpm -qip "$rpmfile"
    say "dnf install ./bitbraid-${version}-1.$arch.rpm"
    retry dnf install -y -q "$rpmfile"
    say "rpm -V: the files as installed"
    rpm -V bitbraid || fail "rpm -V bitbraid"
    say "tools for the checks"
    retry dnf install -y -q xorg-x11-server-Xvfb xauth xwininfo mesa-vulkan-drivers \
        dejavu-sans-fonts desktop-file-utils appstream cage grim \
        xorg-x11-server-Xwayland >/dev/null
    licences=/usr/share/licenses/bitbraid
    ;;
arch)
    say "makepkg: the PKGBUILD, from the tarball"
    retry pacman -Syu --noconfirm --needed fakeroot binutils diffutils >/dev/null
    useradd -m builder
    build=/home/builder/build
    install -d -o builder "$build"
    cp "$packages/aur/PKGBUILD" "$packages/aur/.SRCINFO" \
        "$packages/bitbraid-$version-$arch-linux.tar.gz" "$build/"
    chown builder "$build"/* "$build/.SRCINFO"
    su builder -c "cd $build && makepkg --noconfirm --nodeps"
    say ".SRCINFO: as makepkg writes it"
    su builder -c "cd $build && makepkg --printsrcinfo" | diff - "$build/.SRCINFO" \
        || fail ".SRCINFO is not what makepkg writes"
    say "pacman -U"
    retry pacman -U --noconfirm "$build"/bitbraid-bin-*.pkg.tar.zst >/dev/null
    say "tools for the checks"
    retry pacman -S --noconfirm --needed xorg-server-xvfb xorg-xauth xorg-xwininfo \
        vulkan-swrast ttf-dejavu desktop-file-utils appstream cage grim \
        xorg-xwayland >/dev/null
    licences=/usr/share/licenses/bitbraid
    ;;
*)
    fail "no family $family"
    ;;
esac

say "bitbraid --version"
said=$(bitbraid --version)
echo "$said"
[ "$said" = "bitbraid $version" ] || fail "it said $said"

say "the libraries it is linked against"
missing=$(ldd /usr/bin/bitbraid | grep 'not found' || true)
[ -z "$missing" ] || fail "$missing"
say "the libraries it opens"
for lib in libvulkan.so.1 libwayland-client.so.0 libxkbcommon-x11.so.0; do
    ldconfig -p | grep -q "$lib" || fail "$lib is not installed"
done

say "its files"
for file in \
    /usr/share/applications/bitbraid.desktop \
    /usr/share/applications/bitbraid-url.desktop \
    /usr/share/icons/hicolor/scalable/apps/bitbraid.svg \
    /usr/share/icons/hicolor/48x48/apps/bitbraid.png \
    /usr/share/icons/hicolor/512x512/apps/bitbraid.png \
    /usr/share/metainfo/bitbraid.metainfo.xml; do
    [ -s "$file" ] || fail "$file is missing"
done
say "its licences"
if [ "$family" = deb ]; then
    # All in the one copyright file, as Debian has it.
    licence_files=/usr/share/doc/bitbraid/copyright
else
    licence_files="$licences/LICENSE $licences/THIRD-PARTY-LICENSES.txt"
fi
# shellcheck disable=SC2086 # the list is split on purpose
cat $licence_files >/tmp/licences || fail "the licences: $licence_files"
for text in 'MIT License' 'The name and logo' 'Third-party software in BitBraid' 'Apache License'; do
    grep -q "$text" /tmp/licences || fail "the licences do not say: $text"
done
grep -q '^Exec=bitbraid %f$' /usr/share/applications/bitbraid.desktop || fail "the desktop entry"
grep -q "^MimeType=x-scheme-handler/bitbraid;$" /usr/share/applications/bitbraid-url.desktop \
    || fail "the entry for links"
grep -q "<release version=\"$version\"" /usr/share/metainfo/bitbraid.metainfo.xml \
    || fail "the metadata's release"

say "desktop-file-validate"
desktop-file-validate /usr/share/applications/bitbraid.desktop \
    /usr/share/applications/bitbraid-url.desktop
say "appstreamcli validate"
# The id is the desktop entry's, not reverse-DNS: that one finding is a
# note, the rest stays as strict as it is.
metainfo=/usr/share/metainfo/bitbraid.metainfo.xml
if appstreamcli validate --help 2>&1 | grep -q -- --override; then
    appstreamcli validate --no-net --override=cid-desktopapp-is-not-rdns=info "$metainfo"
else
    # Older (0.15, Ubuntu 22.04): no --override, so that finding is let
    # through here and nothing else.
    said=$(appstreamcli validate --no-net "$metainfo" 2>&1 || true)
    echo "$said"
    if printf '%s\n' "$said" | grep -E '^[EW]: ' | grep -v ' cid-desktopapp-is-not-rdns ' | grep -q .; then
        fail "appstreamcli validate"
    fi
    printf '%s\n' "$said" | grep -q 'Validation' || fail "appstreamcli said nothing of validation"
fi

# cage will not run as root; the pictures are written as that user.
id tester >/dev/null 2>&1 || useradd -m tester
[ -d /out ] && chmod 777 /out

say "its window, on X11 in a virtual display"
start bitbraid
say "its window, drawn on Wayland in a headless compositor"
su tester -c "sh /test/package-draw.sh $system-package bitbraid"

say "the AppImage"
cp "$packages/bitbraid-$version-$arch.AppImage" /tmp/bitbraid.AppImage
chmod 755 /tmp/bitbraid.AppImage
# A container has no FUSE: the AppImage unpacks itself and runs.
export APPIMAGE_EXTRACT_AND_RUN=1
said=$(/tmp/bitbraid.AppImage --version)
echo "$said"
[ "$said" = "bitbraid $version" ] || fail "the AppImage said $said"
start /tmp/bitbraid.AppImage
su tester -c "APPIMAGE_EXTRACT_AND_RUN=1 sh /test/package-draw.sh $system-appimage /tmp/bitbraid.AppImage"
unset APPIMAGE_EXTRACT_AND_RUN

say "the tarball"
mkdir -p /tmp/tarball
tar -xzf "$packages/bitbraid-$version-$arch-linux.tar.gz" -C /tmp/tarball
said=$("/tmp/tarball/bitbraid-$version-$arch-linux/bin/bitbraid" --version)
echo "$said"
[ "$said" = "bitbraid $version" ] || fail "the tarball's said $said"
if tar -tzvf "$packages/bitbraid-$version-$arch-linux.tar.gz" | grep -v ' root/root ' | grep -q .; then
    fail "the tarball has files not owned by root"
fi

say "taken out again"
case $family in
deb) apt-get remove -y -qq bitbraid >/dev/null ;;
rpm) dnf remove -y -q bitbraid >/dev/null ;;
arch) pacman -R --noconfirm bitbraid-bin >/dev/null ;;
esac
[ ! -e /usr/bin/bitbraid ] || fail "/usr/bin/bitbraid is still there"
[ ! -e /usr/share/applications/bitbraid.desktop ] || fail "the desktop entry is still there"

say "passed"
