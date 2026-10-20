# The environment the Linux packages are built in, by the
# release workflow and by hand alike:
#
#   docker build -t bitbraid-linux-build packaging/linux
#   docker run --rm -v "$PWD:/src" -e SOURCE_DATE_EPOCH bitbraid-linux-build \
#       cargo xtask package all
#
# Debian 12: the binary asks for at most the glibc it was linked against
# (2.36), and the packages declare the version it asks for. The image, Rust
# and the AppImage tools are pinned, each by its checksum, so that a
# release can be built again to the byte (`cargo xtask package` writes the
# archives without timestamps of their own).
#
# Sources: Debian's and rustup's documentation (`rustup-init -y
# --default-toolchain`, `--profile minimal`), the Dockerfile reference;
# AppImage's documentation of appimagetool and of its type 2 runtime.

FROM debian:bookworm@sha256:2c037a04925515fdd6ea85ea14a682d0e79931f5e9f5d07b6dbfc6ba12f9e858

# What the window toolkit links against or opens at run time, as the gate
# installs it (.github/workflows/ci.yml); git for the build's date; the
# validators of the desktop entry and the AppStream metadata.
RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl git file xz-utils \
        pkg-config libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev \
        libx11-dev libxcb1-dev libxcb-xkb-dev libvulkan-dev \
        libfontconfig1-dev libfreetype-dev libssl-dev libzstd-dev zlib1g-dev \
        desktop-file-utils appstream \
    && rm -rf /var/lib/apt/lists/*

ARG RUSTUP_VERSION=1.29.1
ARG RUSTUP_SHA256=dda7234360b7f578ca8b0ddcb80145646fa61a67c1720a5abc7051b35c9fcb71
ARG RUST_VERSION=1.99.0
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/cargo \
    PATH=/usr/local/rustup-bin:/cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
RUN curl -fsSLo /tmp/rustup-init \
        "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
    && echo "${RUSTUP_SHA256}  /tmp/rustup-init" | sha256sum -c - \
    && chmod +x /tmp/rustup-init \
    && /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" \
    && rm /tmp/rustup-init \
    # The proxies where a build run as another user, with a CARGO_HOME of
    # its own, still finds them.
    && mkdir -p /usr/local/rustup-bin \
    && cp -a /cargo/bin/. /usr/local/rustup-bin/ \
    && chmod -R a+rX /usr/local/rustup /cargo \
    && rustc --version

# appimagetool and the runtime it puts in front of the AppImage, as their
# projects release them. In a container there is no FUSE: the tool runs
# unpacked.
ARG APPIMAGETOOL_VERSION=1.9.1
ARG APPIMAGETOOL_SHA256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
ARG APPIMAGE_RUNTIME_VERSION=20251108
ARG APPIMAGE_RUNTIME_SHA256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d
RUN mkdir -p /opt/appimage \
    && curl -fsSLo /opt/appimage/appimagetool \
        "https://github.com/AppImage/appimagetool/releases/download/${APPIMAGETOOL_VERSION}/appimagetool-x86_64.AppImage" \
    && curl -fsSLo /opt/appimage/runtime-x86_64 \
        "https://github.com/AppImage/type2-runtime/releases/download/${APPIMAGE_RUNTIME_VERSION}/runtime-x86_64" \
    && printf '%s  %s\n' \
        "${APPIMAGETOOL_SHA256}" /opt/appimage/appimagetool \
        "${APPIMAGE_RUNTIME_SHA256}" /opt/appimage/runtime-x86_64 \
        | sha256sum -c - \
    && chmod a+rx /opt/appimage/appimagetool && chmod a+r /opt/appimage/runtime-x86_64
ENV APPIMAGETOOL=/opt/appimage/appimagetool \
    APPIMAGE_RUNTIME=/opt/appimage/runtime-x86_64 \
    APPIMAGE_EXTRACT_AND_RUN=1

WORKDIR /src
