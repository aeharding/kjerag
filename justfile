# Installing Kyerag onto a desktop, and vendoring it for Flatpak.
#
# Not a build system: `cargo build` and the AGENTS.md gates are still the
# way to build and check the code. This file exists because a double click
# on a .insv needs four files in four places and two cache refreshes in the
# right order, which is more than a README line can hold honestly.
#
# The recipe layout follows cosmic-player's justfile (rev 23d5944), which is
# what a first-party COSMIC app does.

name := 'kyerag'
appid := 'app.kyerag.Kyerag'

# `just install` needs root for this default. `just prefix=$HOME/.local
# install` does not, but then the session's PATH has to contain
# ~/.local/bin, because the desktop entry runs `kyerag`, not a path.
prefix := '/usr/local'
share := prefix / 'share'

default: build-release

build-release:
    cargo build --release

# The two database refreshes are ordered, not decorative: update-mime-database
# is what teaches the system that *.insv is video/x-insta360-insv, and
# update-desktop-database is what records who handles that type. Run the
# second one first and it records a handler for a type nothing produces.
install: build-release
    install -Dm0755 target/release/{{ name }} {{ prefix }}/bin/{{ name }}
    install -Dm0644 res/{{ appid }}.desktop {{ share }}/applications/{{ appid }}.desktop
    install -Dm0644 res/{{ appid }}.metainfo.xml {{ share }}/metainfo/{{ appid }}.metainfo.xml
    install -Dm0644 res/{{ appid }}.xml {{ share }}/mime/packages/{{ appid }}.xml
    update-mime-database {{ share }}/mime
    update-desktop-database {{ share }}/applications

uninstall:
    rm -f {{ prefix }}/bin/{{ name }}
    rm -f {{ share }}/applications/{{ appid }}.desktop
    rm -f {{ share }}/metainfo/{{ appid }}.metainfo.xml
    rm -f {{ share }}/mime/packages/{{ appid }}.xml
    update-mime-database {{ share }}/mime
    update-desktop-database {{ share }}/applications

# Everything cargo would fetch, in one tarball, so the Flatpak build can run
# with no network. This covers the eleven git dependencies and the
# [patch.crates-io] wgpu fork as well as crates.io: `cargo vendor` writes a
# source replacement for each one (verified 2026-07-31, DISTRIBUTION.md 3.5).
#
# The `head -n -1` drops the absolute `directory = ...` line cargo prints and
# the next line puts back a relative one, so the tarball is portable. That
# trick is cosmic-player's.
vendor:
    #!/usr/bin/env bash
    set -euo pipefail
    mkdir -p .cargo
    cargo vendor --locked vendor | head -n -1 > .cargo/config.toml
    echo 'directory = "vendor"' >> .cargo/config.toml
    tar pcf vendor.tar .cargo vendor
    rm -rf .cargo vendor

clean-vendor:
    rm -rf .cargo vendor vendor.tar
