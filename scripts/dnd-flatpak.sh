#!/usr/bin/env bash
#
# Does the Flatpak take a drop? (issue #118)
#
#   scripts/dnd-flatpak.sh [file.insv]      # or set KJERAG_TEST_MEDIA
#
# The one check scripts/uitest.sh cannot make. It runs the app's release
# binary, and the failure this exists for only happens to the sandboxed
# build: a drop from a sandboxed source is a document portal key rather than
# a path, and the released 0.1.1 bundle refused it, so nothing happened when
# a file was dropped on the window.
#
# What it does is the harness's drop check against the INSTALLED Flatpak:
# boot it in a cage session, drag a file onto it with the same instrument
# (`kjerag-spike --bin dragsource`), and read the app's own `media:` line,
# which it prints when it has opened a file and never otherwise. Both offers
# are made, because they fail differently inside a sandbox and only one of
# them is ours to fix:
#
#   portal     a source that registered the files with the document portal,
#              which is every GTK app and every sandboxed one. This is the
#              one issue #118 is about and the one the fix answers.
#   uri-list   a source that hands over a path, which is cosmic-files and
#              most of the rest. The path is the host's own, so it only opens
#              where the sandbox can see it, which since issue #118 is the
#              videos folder (`--filesystem=xdg-videos:ro`).
#
# So the same drop is made twice for the second one, inside the grant and
# outside it. Outside it the file must not open and the window must say why:
# what the pilot gets there is one menu item away and not a corrupt file.
#
# Needs `cage grim` and the Flatpak installed, and it uses the real
# XDG_RUNTIME_DIR through a symlink: a Flatpak reaches the document portal
# and its D-Bus proxy through that directory, so the session's own private
# one would take the portal away and prove nothing.
#
# Exit: 0 the portal drop opened the file, 1 it did not, 2 could not run.

set -uo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
session=$root/scratch/dnd-flatpak
media=${1:-${KJERAG_TEST_MEDIA:-}}
app=${KJERAG_FLATPAK:-dev.harding.Kjerag}
dragsource=$root/target/release/dragsource

# The app has to read a trailer off a multi-gigabyte file before it says it
# opened anything.
OPEN=90

die() {
	printf 'dnd-flatpak: %s\n' "$1" >&2
	exit 2
}

for tool in cage grim flatpak; do
	command -v "$tool" >/dev/null || die "$tool is not installed"
done
[ -n "$media" ] || die "a file to drop is the one argument (or KJERAG_TEST_MEDIA)"
[ -f "$media" ] || die "no file at $media"
flatpak info "$app" >/dev/null 2>&1 || die "$app is not installed"

printf 'building %s\n' "$dragsource"
(cd "$root" && cargo build --release -p kjerag-spike --bin dragsource) ||
	die "the drag source did not build"

rm -rf "$session"
mkdir -p "$session"
printf 'session %s\n' "$session"
printf 'app     %s\n' "$(flatpak info "$app" | sed -n 's/^ *Version: *//p')"

runtime=$(mktemp -d "${TMPDIR:-/tmp}/kjerag-dnd-flatpak.XXXXXXXX")
chmod 700 "$runtime"
link=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}/wayland-kjerag-dnd
ln -sf "$runtime/wayland-0" "$link"

# The launcher runs with the desktop's own runtime and data directories: a
# Flatpak looks its installations up under XDG_DATA_HOME and reaches the
# portals under XDG_RUNTIME_DIR, and the session below redirects both.
launcher=$session/run-flatpak.sh
cat >"$launcher" <<EOF
#!/usr/bin/env bash
exec env XDG_RUNTIME_DIR='${XDG_RUNTIME_DIR:-/run/user/$(id -u)}' \\
	XDG_DATA_HOME='${XDG_DATA_HOME:-$HOME/.local/share}' \\
	WAYLAND_DISPLAY=wayland-kjerag-dnd \\
	flatpak run '$app'
EOF
chmod +x "$launcher"

log=$session/app.log
env XDG_RUNTIME_DIR="$runtime" \
	XDG_CONFIG_HOME="$session/config" XDG_STATE_HOME="$session/state" \
	XDG_DATA_HOME="$session/data" XDG_CACHE_HOME="$session/cache" \
	WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
	cage -- "$launcher" >"$log" 2>&1 &
cage_pid=$!

teardown() {
	kill -TERM "$cage_pid" 2>/dev/null
	sleep 1
	kill -KILL "$cage_pid" 2>/dev/null
	wait "$cage_pid" 2>/dev/null
	rm -rf "$runtime"
	rm -f "$link"
}
trap teardown EXIT

waited=0
while [ ! -S "$runtime/wayland-0" ]; do
	sleep 0.2
	waited=$((waited + 1))
	[ "$waited" -lt 50 ] || die "no wayland socket after 10 s (see $log)"
done
# The window has to be up before anything is dropped on it, and a Flatpak's
# first start is slower than a binary's.
sleep 8
env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
	grim -t ppm "$session/window.ppm" 2>>"$log" ||
	die "nothing to capture, so the window never came up (see $log)"

failures=0

# drop <name> <offer> <file>: drag it in and say whether the app opened it.
drop() {
	local name=$1 offer=$2 file=$3
	local report=$session/drag-$name.log
	local before waited=0 pid opened

	before=$(grep -c '^media:' "$log")
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
		"$dragsource" "$file" "offer=$offer" "linger=$OPEN" >"$report" 2>&1 &
	pid=$!
	while [ "$waited" -le $((OPEN * 2)) ]; do
		kill -0 "$cage_pid" 2>/dev/null || break
		[ "$(grep -c '^media:' "$log")" -gt "$before" ] && break
		sleep 0.5
		waited=$((waited + 1))
	done
	kill "$pid" 2>/dev/null
	wait "$pid" 2>/dev/null

	opened=no
	[ "$(grep -c '^media:' "$log")" -gt "$before" ] && opened=yes
	printf '%-22s the file opened: %-3s  %s\n' "$name" "$opened" \
		"$(sed -n 's/^dragsource: the target read it as /read as /p' "$report" |
			tr '\n' ' ')"
	[ "$opened" = yes ]
}

# A path the grant does not cover, which is the same file reached by a name
# outside the videos folder. Nothing is copied: what is under test is the path,
# and the sandbox has no mount for this one either way.
outside=$session/out-of-reach.insv
ln -sf "$media" "$outside"

printf '\n'
drop portal portal "$media" || failures=$((failures + 1))
drop "uri-list in videos" uri-list "$media" || failures=$((failures + 1))

# And the one that must NOT open, because the grant is a folder and not the
# filesystem. What is required of it is the honest refusal rather than the
# words a corrupt file gets (issue #118).
before=$(grep -c '^kjerag: .* not shown' "$log")
if drop "uri-list outside it" uri-list "$outside"; then
	printf 'FAIL   a file outside the videos folder opened, so the grant is wider than it says\n'
	failures=$((failures + 1))
elif [ "$(grep -c '^kjerag: .* not shown' "$log")" -le "$before" ]; then
	printf 'FAIL   nothing was refused, so the drop never reached the open\n'
	failures=$((failures + 1))
else
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
		grim -t ppm "$session/refused.ppm" 2>>"$log"
	if cmp -s "$session/window.ppm" "$session/refused.ppm"; then
		printf 'FAIL   the window is unchanged, so nothing was said about the refusal\n'
		failures=$((failures + 1))
	else
		printf '%-22s refused, and the window says so: %s\n' "" \
			"$session/refused.ppm"
	fi
fi

printf '\nlogs and captures: %s\n' "$session"
[ "$failures" = 0 ] || exit 1
