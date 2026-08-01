# Installing Kyerag onto a desktop.
#
# Not a build system: `cargo build` and the AGENTS.md gates are still the
# way to build and check the code. This file exists because a double click
# on a .insv needs a binary, three files in three places, an icon theme tree
# and two cache refreshes in the right order, which is more than a README
# line can hold honestly.
#
# The recipe layout follows cosmic-player's justfile (rev 23d5944), which is
# what a first-party COSMIC app does.

name := 'kyerag'
appid := 'app.kyerag.Kyerag'

# The icons are named for the application ID issue #66 settled, and the
# binary does not carry that name yet: issue #75's rename sweep puts it in
# the source and collapses these two into one. Until then the entry's `Icon=`
# key and the installed icon files are different names, so a launcher shows a
# placeholder. Nothing else about the install depends on it.
iconid := 'dev.harding.Kjerag'

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
install: build-release install-icons
    install -Dm0755 target/release/{{ name }} {{ prefix }}/bin/{{ name }}
    install -Dm0644 resources/{{ appid }}.desktop {{ share }}/applications/{{ appid }}.desktop
    install -Dm0644 resources/{{ appid }}.metainfo.xml {{ share }}/metainfo/{{ appid }}.metainfo.xml
    install -Dm0644 resources/{{ appid }}.xml {{ share }}/mime/packages/{{ appid }}.xml
    update-mime-database {{ share }}/mime
    update-desktop-database {{ share }}/applications

# Every file of the theme tree, copied verbatim rather than listed one by
# one. The tree is generated (resources/icons/README.md), so a list written
# here goes stale the first time a size is added or dropped.
install-icons:
    #!/usr/bin/env bash
    set -euo pipefail
    cd resources/icons
    find hicolor -type f -print0 | while IFS= read -r -d '' f; do
        install -Dm0644 "$f" "{{ share }}/icons/$f"
    done

uninstall:
    rm -f {{ prefix }}/bin/{{ name }}
    rm -f {{ share }}/applications/{{ appid }}.desktop
    rm -f {{ share }}/metainfo/{{ appid }}.metainfo.xml
    rm -f {{ share }}/mime/packages/{{ appid }}.xml
    rm -f {{ share }}/icons/hicolor/*/apps/{{ iconid }}.*
    update-mime-database {{ share }}/mime
    update-desktop-database {{ share }}/applications
