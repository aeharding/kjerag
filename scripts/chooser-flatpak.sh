#!/usr/bin/env bash
#
# What does the file chooser hand the sandboxed app? (issue #123)
#
#   scripts/chooser-flatpak.sh <file.insv>      # READONLY=1 ticks the box
#
# The other half of scripts/chooser-probe.py: that one asks the document
# portal what it would do, this one drives a real dialog and reads what the
# app was actually handed. It is the check scripts/uitest.sh cannot make,
# because a file chooser is the portal's window rather than the app's.
#
# No window lands on the developer's desktop. The whole portal stack runs a
# second time inside a headless `cage` session on its own D-Bus session bus:
# the backend that draws the dialog is started there, so it draws there. The
# services are started by hand because their D-Bus service files name a
# systemd unit, and systemd would start them on the desktop's bus instead.
#
# The app is the installed Flatpak, unchanged, and the evidence is the
# portal's own Request.Response signal read off that private bus: the array of
# URIs in it is what the app is handed, and the app's `media:` line says what
# it made of it.
#
# `wtype` types the path into the location bar, which it can do, and the
# button is pressed by `kjerag-spike --bin click`, which exists because it
# cannot: no named key (Return, BackSpace, the arrows) reaches a GTK client in
# a cage session here.
#
# Two things in it are the harness rather than the desktop, and both are
# visible in what it prints:
#
#   - the second document portal mounts under this session's runtime
#     directory, so a document path reads `<runtime>/doc/<id>/<name>` rather
#     than `/run/user/<uid>/doc/...`, and the app cannot open it. A document
#     path is still a document path; only the prefix is this session's.
#   - `--filesystem=xdg-run/gvfs` resolves against XDG_RUNTIME_DIR, which this
#     session replaces, so inside it that grant covers nothing and a file on a
#     network mount always comes back as a document. Measure the share with
#     scripts/chooser-probe.py, which runs on the desktop's own session.
#
# Exit: 0 the picker answered with a URI, 1 it did not, 2 could not run.

set -uo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
session=$root/scratch/chooser-flatpak
app=${KJERAG_FLATPAK:-dev.harding.Kjerag}
backend=${BACKEND:-gtk}
desktop=${DESKTOP:-gnome}
target=${1:-}

die() {
	printf 'realchooser: %s\n' "$1" >&2
	exit 2
}

for tool in cage wtype grim flatpak dbus-monitor dbus-run-session; do
	command -v "$tool" >/dev/null || die "$tool is not installed"
done
[ -n "$target" ] || die "the file to pick is the one argument"
[ -f "$target" ] || die "no file at $target"
flatpak info "$app" >/dev/null 2>&1 || die "$app is not installed"

printf 'building %s\n' "$root/target/release/click"
(cd "$root" && cargo build --release -p kjerag-spike --bin click) ||
	die "the click instrument did not build"

rm -rf "$session"
mkdir -p "$session"
printf 'app     %s, commit %s\n' \
	"$(flatpak info "$app" | sed -n 's/^ *Version: *//p')" \
	"$(flatpak info "$app" | sed -n 's/^ *Commit: *//p' | cut -c1-12)"
printf 'backend %s\n' "$backend"
printf 'pick    %s\n' "$target"

runtime=$(mktemp -d "${TMPDIR:-/tmp}/kjerag-chooser.XXXXXXXX")
chmod 700 "$runtime"
printf 'runtime %s\n' "$runtime"

cat >"$session/inner.sh" <<EOF
#!/usr/bin/env bash
# Inside cage, inside a private session bus. The portal services are started
# by hand because their D-Bus service files name a systemd unit, and systemd
# would start them on the desktop's bus rather than this one.
export XDG_CURRENT_DESKTOP=$desktop
# The desktop's own data and config directories, for the portal services as
# well as the app. The document portal decides whether to hand back a real
# path by running \`flatpak info --file-access\` on the app, and flatpak looks
# its installations up under XDG_DATA_HOME: with this session's own empty one
# the app is not installed as far as that check can tell, it falls back to a
# metadata read that finds no metadata, and every file is registered as a
# document whatever the grants say (measured 2026-08-01).
export XDG_DATA_HOME='${XDG_DATA_HOME:-$HOME/.local/share}'
export XDG_CONFIG_HOME='${XDG_CONFIG_HOME:-$HOME/.config}'
printf 'bus %s\n' "\$DBUS_SESSION_BUS_ADDRESS"
dbus-monitor --session "type='signal',interface='org.freedesktop.portal.Request'" \
	>'$session/response.log' 2>&1 &
/usr/libexec/xdg-permission-store >'$session/permission.log' 2>&1 &
/usr/libexec/xdg-document-portal >'$session/document.log' 2>&1 &
sleep 1
/usr/libexec/xdg-desktop-portal-$backend >'$session/backend.log' 2>&1 &
/usr/libexec/xdg-desktop-portal >'$session/xdp.log' 2>&1 &
sleep 2
exec env XDG_DATA_HOME='${XDG_DATA_HOME:-$HOME/.local/share}' \\
	XDG_CONFIG_HOME='${XDG_CONFIG_HOME:-$HOME/.config}' \\
	flatpak run --filesystem='$runtime/doc' '$app'
EOF
chmod +x "$session/inner.sh"

cat >"$session/launcher.sh" <<EOF
#!/usr/bin/env bash
exec dbus-run-session -- '$session/inner.sh'
EOF
chmod +x "$session/launcher.sh"

log=$session/app.log
env XDG_RUNTIME_DIR="$runtime" \
	XDG_CONFIG_HOME="$session/config" XDG_STATE_HOME="$session/state" \
	XDG_DATA_HOME="$session/data" XDG_CACHE_HOME="$session/cache" \
	WLR_BACKENDS=headless WLR_LIBINPUT_NO_DEVICES=1 \
	cage -- "$session/launcher.sh" >"$log" 2>&1 &
cage_pid=$!

teardown() {
	kill -TERM "$cage_pid" 2>/dev/null
	sleep 1
	kill -KILL "$cage_pid" 2>/dev/null
	wait "$cage_pid" 2>/dev/null
	fusermount3 -u "$runtime/doc" 2>/dev/null
	sleep 1
	rm -rf "$runtime"
}
trap teardown EXIT

waited=0
while [ ! -S "$runtime/wayland-0" ]; do
	sleep 1
	waited=$((waited + 1))
	[ "$waited" -lt 20 ] || die "no wayland socket under $runtime"
done

press() {
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 wtype "$@" 2>>"$session/wtype.log"
}
shot() {
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 grim "$session/$1.png" 2>/dev/null
}

sleep 12
shot before

click() {
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY=wayland-0 \
		"$root/target/release/click" "$@" >>"$session/click.log" 2>&1
}

# Drive it by hand: the session stays up and its address is printed, so keys
# can be sent from another shell while the dialog is looked at.
if [ -n "${KEEP:-}" ]; then
	printf 'session held. drive it with:\n'
	printf '  env XDG_RUNTIME_DIR=%s WAYLAND_DISPLAY=wayland-0 wtype ...\n' "$runtime"
	printf '  env XDG_RUNTIME_DIR=%s WAYLAND_DISPLAY=wayland-0 grim shot.png\n' "$runtime"
	printf 'responses land in %s\n' "$session/response.log"
	sleep "${KEEP}"
	printf '\n--- what the picker answered ---\n'
	grep -A6 -E "member=Response" "$session/response.log" | head -60
	exit 0
fi

# wtype drops about one key in twenty on this box (AGENTS.md), and a dropped
# key in a path is a path that does not exist. It also delivers character
# keys into this session and nothing else: Return, BackSpace and the arrows
# never reach the dialog (measured), so the button is clicked rather than
# pressed, and a bad path is retyped from a fresh dialog.
answered() {
	grep -q 'string "file://' "$session/response.log"
}
# The dialog is up once the backend has said something about the window it
# just made; a click that went missing leaves that count where it was. Both
# logs are counted because the backend is often the instance D-Bus activated
# rather than the one started here, and that one writes to the session's log.
dialogs() {
	cat "$session/backend.log" "$log" 2>/dev/null | grep -c "parent window"
}
for attempt in 1 2 3; do
	printf 'opening the chooser (attempt %s)\n' "$attempt"
	before=$(dialogs)
	click 640 474
	waited=0
	while [ "$(dialogs)" -le "$before" ]; do
		sleep 1
		waited=$((waited + 1))
		[ "$waited" -lt 15 ] || break
	done
	[ "$(dialogs)" -gt "$before" ] || {
		printf 'no dialog\n'
		continue
	}
	sleep 2
	shot "dialog-$attempt"
	# The read-only checkbox is the backend's `writable` result, which is
	# what decides whether the document portal is used at all. It is ticked
	# before the location is typed: a click anywhere else takes the focus
	# off the location entry and the typed path with it.
	if [ -n "${READONLY:-}" ]; then
		click 18 696
		sleep 2
	fi
	printf 'typing the path\n'
	press -M ctrl -k l -m ctrl
	sleep 2
	press -d 30 "$target"
	sleep 3
	shot "typed-$attempt"
	click 1234 22
	sleep 8
	shot "after-$attempt"
	answered && break
	# A path the dialog would not take (a dropped key makes one) leaves the
	# dialog open over the app. Cancel it and start from the app again.
	printf 'not answered, cancelling\n'
	click 45 22
	sleep 3
done

printf '\n--- what the picker answered ---\n'
grep -A4 -E "member=Response" "$session/response.log" | head -60
printf '\n--- the app said ---\n'
grep -E "^chose:|^media:|^lens:|kjerag:" "$log" | head -20

grep -q "member=Response" "$session/response.log"
