#!/usr/bin/env bash
#
# Regenerates flatpak/cargo-sources.json from Cargo.lock (issue #72).
#
#   scripts/cargo-sources.sh
#
# flatpak-builder builds with no network, so every crate has to be a declared
# source. `cargo vendor` answers that too and this repository cannot carry the
# answer: 668 crates, 988 MB, ~950 MB tarred. The generated JSON is 501 KB, it
# is what every COSMIC app on Flathub ships, and it can be committed, so the
# Flatpak build has no step that needs the network at all.
#
# Rerun it in any change that moves Cargo.lock. A stale cargo-sources.json is
# not a build that fetches the missing crate, it is a build that fails.
#
# What it writes into the manifest's world:
#
#   cargo/config          the source replacement, one stanza per git remote
#   cargo/vendor/<crate>  every crate of the lock file
#
# both relative to the build directory, which is why the manifest has to set
# CARGO_HOME to exactly /run/build/<module-name>/cargo: cargo reads
# $CARGO_HOME/config, and anywhere else the file is written and never read,
# and the build dies with "you are in the offline mode".
#
# The generator is pinned by commit for the same reason the wgpu fork is
# (issue #68): `master` is a moving target and a build recipe that regenerates
# differently next month is not a build recipe. Bump TOOL_REV deliberately.
#
# Its dependencies are three Python packages that are not on a stock Pop!_OS
# box, and `python3 -m venv` needs python3-venv installed with root. So they
# are installed with `pip3 install --target` into scratch/, which is
# gitignored, needs no root, and touches nothing outside this repository. Both
# the tool and the packages are fetched once and reused after that.
#
# Exit: 0 written, 1 could not run.

set -euo pipefail

# flatpak/flatpak-builder-tools, cargo/flatpak-cargo-generator.py.
# f03a673 is that file's newest commit as of 2026-07-31 (2025-08-16).
TOOL_REV=f03a673abe6ce189cea1c2857e2b44af2dd79d1f
TOOL_SHA256=b373c8ab1a05378ec5d8ed0645c7b127bcec7d2f7a1798694fbc627d570d856c
# The generator's own PEP 723 header, which is where these three come from.
DEPS=('aiohttp>=3.9.5,<4.0.0' 'PyYAML>=6.0.2,<7.0.0' 'tomlkit>=0.13.3,<1.0')

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
work=$root/scratch/cargo-sources
tool=$work/flatpak-cargo-generator-$TOOL_REV.py
lib=$work/pylib
out=$root/flatpak/cargo-sources.json

die() {
	printf 'cargo-sources: %s\n' "$1" >&2
	exit 1
}

command -v python3 >/dev/null || die 'no python3'
mkdir -p "$work" "$root/flatpak"

if [ ! -f "$tool" ]; then
	printf 'cargo-sources: fetching the generator at %s\n' "${TOOL_REV:0:7}"
	curl -sSfL -o "$tool.part" \
		"https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/$TOOL_REV/cargo/flatpak-cargo-generator.py" ||
		die 'could not fetch the generator'
	printf '%s  %s\n' "$TOOL_SHA256" "$tool.part" | sha256sum --check --status ||
		die "the generator at $TOOL_REV is not the file TOOL_SHA256 names"
	mv "$tool.part" "$tool"
fi

if ! PYTHONPATH=$lib python3 -c 'import aiohttp, tomlkit, yaml' 2>/dev/null; then
	printf 'cargo-sources: installing %s into scratch/\n' "${DEPS[*]}"
	pip3 install --quiet --target "$lib" "${DEPS[@]}" ||
		die 'could not install the generator dependencies'
fi

PYTHONPATH=$lib python3 "$tool" "$root/Cargo.lock" --output "$out" ||
	die 'the generator failed'

printf 'cargo-sources: %s, %s sources, %s bytes\n' \
	"${out#"$root"/}" \
	"$(PYTHONPATH=$lib python3 -c 'import json,sys; print(len(json.load(open(sys.argv[1]))))' "$out")" \
	"$(stat -c %s "$out")"
