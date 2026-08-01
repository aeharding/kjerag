#!/usr/bin/env bash
#
# Headless UI checks: drive the real app in a throwaway compositor and look
# at what came out.
#
#   scripts/uitest.sh [file.insv]      # or set KJERAG_TEST_MEDIA
#
# The same checks run against the installed Flatpak with
# KJERAG_FLATPAK=dev.harding.Kjerag, which is how a bundle is checked before
# it is called a release (docs/RELEASING.md).
#
# `cage` runs one client on a wlroots headless backend, which is a whole
# Wayland session with no monitor and no connection to the desktop the
# developer is looking at. `wtype` presses keys into it over the virtual
# keyboard protocol, `target/release/pointer` moves and clicks over the
# virtual pointer one, and `grim` copies the output out. The app is the
# release binary, unchanged: nothing here is a test hook.
#
# What the checks are allowed to believe, strongest first:
#
#   1. the app's own stdout: a report line every 5 s while playing, and one
#      `device:` line saying whether the zero-copy import was available;
#   2. two captures of the same output, which differ while a video is playing
#      and are byte for byte identical while it is paused;
#   3. one capture, which is at least not a black rectangle.
#
# Everything the run writes lands in scratch/uitest/, which is gitignored,
# because a capture of real footage is personal video and this repo is
# public. The session's XDG directories are redirected there too, so pressing
# `h` writes a config the developer's desktop never sees and pressing `s`
# writes a still into scratch/ rather than into their screenshots folder.
# The one thing the session shares with the desktop is the sound server, and
# what it plays there goes into a null sink: see the preflight.
#
# Needs `cage wtype grim ffmpeg`, and `wl-paste` for the clipboard check,
# which skips without it.
#
# Local only, and never in CI: see "UI verification" in AGENTS.md.
#
# Exit: 0 all checks passed, 1 a check failed, 2 the harness could not run,
# 3 the session died under the harness (see PRESSES below).

set -uo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
session=$root/scratch/uitest
media=${1:-${KJERAG_TEST_MEDIA:-}}

# The session's home, and the state directory inside it.
#
# `$STATE_HOME` is spelt the long way round rather than as `$session/state`
# because the Flatpak's grant for the same directory is the literal
# `~/.local/state/cosmic` (flatpak/dev.harding.Kjerag.yml), which follows HOME
# and ignores XDG_STATE_HOME entirely (measured). Naming the same path both
# ways is what lets one set of checks read the app's settings whether the app
# is a binary reading XDG_STATE_HOME or a bundle reading a bind of that
# directory.
HOME_DIR=$session/home
STATE_HOME=$HOME_DIR/.local/state

# Opening a file reads the trailer off the end of it, which is a seek and a
# parse over a multi-gigabyte file, so READY is generous. A report line lands
# every 5 s, so REPORT is a little more than that.
READY=45
REPORT=9
QUIT=10
# Long enough for a key to land and the window to be redrawn from it.
# Measured on this box: the app redraws within 100 ms of a key.
SETTLE=1
# How many times a key is pressed before its effect is called missing.
# wtype drops one key in roughly twenty here (measured: 18 of 19 delivered
# over a run, and 1 in 8 in another), which is the harness being flaky rather
# than the app, so a check that watches for an effect presses again instead
# of failing on the first miss. A key whose effect never arrives fails the
# check, which is what a broken binding would look like too.
PRESSES=3
# Where the session's sound goes: the same null sink scripts/quiet.sh loads,
# by the same name, so a box that has run either has one of them.
QUIET_SINK=kjerag_quiet

failures=0
checks=0

# ------------------------------------------------------------- reporting

pass() {
	checks=$((checks + 1))
	printf 'ok    %s\n' "$1"
}

fail() {
	checks=$((checks + 1))
	failures=$((failures + 1))
	printf 'FAIL  %s\n' "$1"
	shift
	for line in "$@"; do printf '      %s\n' "$line"; done
}

skip() {
	printf 'skip  %s\n' "$1"
}

die() {
	printf 'uitest: %s\n' "$1" >&2
	exit 2
}

# The session went away with checks still to run: a dead compositor cannot
# answer anything, so the run stops rather than reporting a UI failure it did
# not observe.
lost() {
	printf 'uitest: the session died before the checks finished (%s)\n' "$1" >&2
	printf 'uitest: logs in %s\n' "$session" >&2
	teardown
	exit 3
}

# ------------------------------------------------------------- preflight

for tool in cage wtype grim ffmpeg; do
	command -v "$tool" >/dev/null || die "$tool is not installed (AGENTS.md, UI verification)"
done

# One check reads the session's clipboard and nothing else wants this, so a
# box without it loses that half of that check rather than the whole run.
clipboard=yes
command -v wl-paste >/dev/null || clipboard=no

# The build is part of the run, not a fallback for a missing binary. Cargo
# is a no-op on a fresh one, and the version that only built when the file
# was absent drove whatever happened to be there instead: on 2026-07-31 a
# binary built before a `git revert` failed the ball check for an hour while
# the source it was meant to be checking passed on every run.
#
# KJERAG_BIN is the way to point the harness at a binary on purpose, which is
# how that was measured, so it is taken as given and never rebuilt.
#
# KJERAG_FLATPAK runs the INSTALLED bundle instead, which is the one thing
# nothing else checks: the release ritual runs this harness against a local
# binary, and the bundle it then publishes was only ever started once by hand
# to see a window (docs/RELEASING.md). 0.1.1 shipped that way. What the app
# does inside the sandbox is a different question from what it does outside
# one -- the runtime carries its own Mesa, its own ffmpeg and its own libva,
# and the frame path runs through all three -- so this mode asks the same
# questions of the same checks with the answer coming from the sandbox.
launch=()
if [ -n "${KJERAG_FLATPAK:-}" ]; then
	app=$KJERAG_FLATPAK
	flatpak info "$app" >/dev/null 2>&1 || die "no flatpak installed for $app (KJERAG_FLATPAK)"
	printf 'flatpak %s %s, commit %s (not rebuilt)\n' "$app" \
		"$(flatpak info "$app" | sed -n 's/^ *Version: *//p')" \
		"$(flatpak info --show-commit "$app" | cut -c1-12)"
	# `boot` redirects XDG_DATA_HOME, and a user installation lives under it,
	# so flatpak inside the session would answer "not installed" for an app
	# that is. Resolved here, before the redirect, and handed back on the
	# command line: the redirect is what keeps the run out of the developer's
	# directories and is worth more than this costs.
	launch=(env "FLATPAK_USER_DIR=${XDG_DATA_HOME:-$HOME/.local/share}/flatpak" flatpak run)
	# Two paths in, and that is the whole of what this mode arranges: the
	# session, which is where captures land and where the synthetic files the
	# refusal checks feed the app are written, and the file to play. Without
	# the first, `dud` and `foreign` would hand the app a path it cannot open
	# and read the refusal that follows as the refusal they are checking for,
	# which is a check passing for the wrong reason. Nothing is taken away,
	# and the shipped permission set is otherwise byte for byte the one a
	# pilot installs.
	#
	# The settings are hermetic without touching that set at all. flatpak
	# resolves a by-name grant against the CALLER's environment (measured):
	# `xdg-config/cosmic` follows XDG_CONFIG_HOME and `~/.local/state/cosmic`
	# follows HOME, both of which `boot` points into the session. The real
	# ~/.config/cosmic is then not bound into the sandbox at all, so the app
	# cannot write it even though it still holds the grant that says it may.
	launch+=("--env=XDG_SCREENSHOTS_DIR=$session/shots" "--filesystem=$session")
	# Where the sound would go, since a sandbox inherits nothing from `boot`'s
	# environment. This mode has none to route today: the bundle holds
	# `--socket=pulseaudio`, flatpak binds that socket out of the caller's
	# runtime directory, and the session's own has only the PipeWire one in
	# it, so the app says "playing silently" and the checks that need sound
	# skip (measured 2026-08-01, with the desktop's sink muted for the length
	# of it: silent with these two and silent without them). They are here so
	# that the day this session carries a pulse socket, it is routed the way
	# every other run is rather than out of the speakers.
	launch+=("--env=PULSE_SINK=$QUIET_SINK" "--env=PIPEWIRE_NODE=$QUIET_SINK")
	[ -z "$media" ] || launch+=("--filesystem=$media:ro")
	launch+=("$app")
elif [ -n "${KJERAG_BIN:-}" ]; then
	bin=$KJERAG_BIN
	[ -x "$bin" ] || die "no binary at $bin (KJERAG_BIN)"
	printf 'binary %s (KJERAG_BIN: not rebuilt)\n' "$bin"
	launch=("$bin")
else
	bin=$root/target/release/kjerag
	printf 'building %s\n' "$bin"
	(cd "$root" && cargo build --release) || die "the app did not build"
	[ -x "$bin" ] || die "no binary at $bin"
	launch=("$bin")
fi

# The pointer is harness machinery rather than the thing under test, so it is
# built from this tree whatever KJERAG_BIN says.
poker=$root/target/release/pointer
(cd "$root" && cargo build --release -p kjerag-spike --bin pointer) ||
	die "the pointer did not build"
[ -x "$poker" ] || die "no pointer at $poker"

# Sound. The app opens an output device when the file has an audio track, and
# under a session with no PipeWire socket in its runtime directory that open
# fails, the app says "playing silently", and everything about the sound is
# drawn disabled - the speaker button included, which is the way into the
# volume popup. So the session is given the desktop's own socket, and the
# stream is sent to a null sink, which is the same routing scripts/quiet.sh
# does and for the same reason: the owner's speakers are not a test fixture
# (AGENTS.md, sound etiquette). Measured 2026-08-01: the app's stream lands on
# kjerag_quiet, with PIPEWIRE_NODE as the thing that puts it there, because
# what plays what cpal writes is pipewire-alsa.
#
# It is the one hole in the session's isolation and it is only ever a hole
# outward: nothing is read from the desktop, one stream is written to a sink
# that goes nowhere.
sound=yes
if command -v pactl >/dev/null && [ -S "/run/user/$(id -u)/pipewire-0" ]; then
	pactl list short sinks | grep -q "[[:space:]]$QUIET_SINK[[:space:]]" ||
		pactl load-module module-null-sink "sink_name=$QUIET_SINK" \
			"sink_properties=device.description=$QUIET_SINK" >/dev/null
else
	sound=no
	printf 'no pipewire socket or no pactl: the session plays silently\n'
fi

[ -z "$media" ] || [ -f "$media" ] || die "no file at $media"

rm -rf "$session"
# The two directories the app keeps settings in, made before the app runs
# rather than by it. A Flatpak's by-name grant is a bind of a host directory
# and flatpak skips one whose source does not exist (measured), so an absent
# directory here is not one the app creates, it is one the app cannot see.
mkdir -p "$session/shots" "$session/config/cosmic" "$STATE_HOME/cosmic"
printf 'session %s\n' "$session"
[ -n "$media" ] || printf 'no test media: pass a file or set KJERAG_TEST_MEDIA for the playback checks\n'

# --------------------------------------------------------- the session

# Set by boot, read by everything after it.
sock=
cage_pid=
runtime=
log=

# boot <label> [file]
#
# The runtime directory is the session's own, which is what keeps the socket
# away from the developer's desktop and makes its name predictable: wlroots
# takes wayland-0 in an empty directory.
boot() {
	local label=$1 file=${2:-}
	log=$session/$label.log
	runtime=$(mktemp -d "${TMPDIR:-/tmp}/kjerag-uitest.XXXXXXXX")
	chmod 700 "$runtime"
	# The sound out, and only the sound out: see the preflight above.
	[ "$sound" = no ] || ln -s "/run/user/$(id -u)/pipewire-0" "$runtime/pipewire-0"

	env \
		HOME="$HOME_DIR" \
		XDG_RUNTIME_DIR="$runtime" \
		XDG_CONFIG_HOME="$session/config" \
		XDG_STATE_HOME="$STATE_HOME" \
		XDG_DATA_HOME="$session/data" \
		XDG_CACHE_HOME="$session/cache" \
		XDG_SCREENSHOTS_DIR="$session/shots" \
		PULSE_SINK="$QUIET_SINK" \
		PIPEWIRE_NODE="$QUIET_SINK" \
		WLR_BACKENDS=headless \
		WLR_LIBINPUT_NO_DEVICES=1 \
		cage -- "${launch[@]}" ${file:+"$file"} >"$log" 2>&1 &
	cage_pid=$!

	local waited=0
	while [ ! -S "$runtime/wayland-0" ]; do
		sleep 0.2
		waited=$((waited + 1))
		[ "$waited" -lt 50 ] || die "no wayland socket after 10 s (see $log)"
	done
	sock=wayland-0
}

alive() {
	kill -0 "$cage_pid" 2>/dev/null
}

# Quit the way a person does. The app calls exit(0) on Ctrl+Q and cage leaves
# with its client's status, so this is the clean-exit check as well as the
# teardown.
quit() {
	local try=0 waited
	while [ "$try" -lt "$PRESSES" ]; do
		key -M ctrl -k q -m ctrl
		waited=0
		while alive && [ "$waited" -lt $((QUIT * 5 / PRESSES)) ]; do
			sleep 0.2
			waited=$((waited + 1))
		done
		alive || break
		try=$((try + 1))
	done
	if alive; then
		kill -KILL "$cage_pid" 2>/dev/null
		wait "$cage_pid" 2>/dev/null
		rm -rf "$runtime"
		return 1
	fi
	wait "$cage_pid"
	local status=$?
	rm -rf "$runtime"
	return "$status"
}

teardown() {
	if alive; then
		kill -TERM "$cage_pid" 2>/dev/null
		sleep 0.5
		kill -KILL "$cage_pid" 2>/dev/null
	fi
	wait "$cage_pid" 2>/dev/null
	[ -z "$runtime" ] || rm -rf "$runtime"
}

key() {
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" wtype "$@" 2>>"$log"
	sleep "$SETTLE"
}

# grab <name> -> path. PPM rather than PNG: an uncompressed capture is what
# the pixel checks read, and grim writes it without a library.
grab() {
	local out=$session/$1.ppm
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" grim -t ppm "$out" 2>>"$log"
	printf '%s' "$out"
}

# poke <x> <y> [click]: put the pointer there, and press the left button if
# asked. `crates/spike/src/bin/pointer.rs` says why this is a binary of ours
# and not `wlrctl pointer`. The output is the one cage gives a headless
# session, which is 1280x720 and is what every measured coordinate here is in.
poke() {
	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" \
		"$poker" 1280 720 "$1" "$2" "${3:-}" 2>>"$log"
	sleep "$SETTLE"
}

# ------------------------------------------------------------ assertions

# A capture is a picture and not an unpainted output. Every byte of a black
# rectangle is zero, so this counts the ones that are not: the app's own
# background is dark but nowhere near zero (measured: 66% of the bytes in the
# welcome view are nonzero, 99% in a frame of video).
NONBLACK_PERCENT=5

nonblack() {
	local file=$1 total nonzero
	[ -s "$file" ] || return 1
	total=$(stat -c%s "$file")
	nonzero=$(tr -d '\0' <"$file" | wc -c)
	[ $((nonzero * 100 / total)) -ge "$NONBLACK_PERCENT" ]
}

# The two bands of the window that transient chrome must never be drawn over,
# because both hold things that get pressed: the header bar with the menus at
# the top, and the control row with the scrubber at the bottom.
#
# Measured under cage at 1280x720: the header bar is the top 48 rows and the
# control row the bottom 48. The bands are those plus clearance, so a message
# that merely creeps up against the scrubber fails this as well.
HEADER_BAND=64
CONTROL_BAND=96

# band <file> <top|bottom> <rows>: those rows of the capture, as raw bytes.
# grim writes a P6 header of three lines and then the pixels, three bytes to
# a pixel, so a band is a slice at a computed offset.
band() {
	local file=$1 edge=$2 rows=$3 magic width height depth header start
	{
		read -r magic
		read -r width height
		read -r depth
	} <"$file"
	header=$((${#magic} + ${#width} + ${#height} + ${#depth} + 4))
	case $edge in
	top) start=$header ;;
	*) start=$((header + (height - rows) * width * 3)) ;;
	esac
	tail -c "+$((start + 1))" "$file" | head -c $((rows * width * 3))
}

# band_changed <before> <after> <top|bottom> <rows>: something was drawn into
# that band between the two captures.
band_changed() {
	! cmp -s <(band "$1" "$3" "$4") <(band "$2" "$3" "$4")
}

# picture <file>: everything between the header bar and the control row, which
# is the view and none of the chrome around it. Same P6 arithmetic as `band`
# above, from the other direction.
#
# The chrome has to come off for a check that compares two captures of one
# view. Measured on this harness: two captures of the same frame at the same
# camera differ in 14 pixels of the scrubber's thumb, because the thumb is
# drawn at the clock's position and a copied line carries the frame's own
# time, which is up to one frame behind it. The picture is byte for byte
# identical.
picture() {
	local file=$1 magic width height depth header rows
	{
		read -r magic
		read -r width height
		read -r depth
	} <"$file"
	header=$((${#magic} + ${#width} + ${#height} + ${#depth} + 4))
	rows=$((height - HEADER_BAND - CONTROL_BAND))
	tail -c "+$((header + HEADER_BAND * width * 3 + 1))" "$file" |
		head -c $((rows * width * 3))
}

# same_picture <a> <b>: the two captures show the same view.
same_picture() {
	cmp -s <(picture "$1") <(picture "$2")
}

# said <pattern>: the app printed it. Its own instruments say things no
# capture can.
said() {
	grep -q -- "$1" "$log"
}

# await <pattern> <seconds>: for the lines that arrive on a clock.
await() {
	local pattern=$1 limit=$2 waited=0
	while ! said "$pattern"; do
		alive || return 1
		sleep 0.5
		waited=$((waited + 1))
		[ "$waited" -le $((limit * 2)) ] || return 1
	done
}

# Two captures of the same output, a beat apart. The picture is paused if
# they are the same bytes: nothing in the window animates on its own, because
# the timer that refreshes the position label only runs while playing.
still_picture() {
	local a b
	a=$(grab "$1-a")
	sleep 0.7
	b=$(grab "$1-b")
	cmp -s "$a" "$b"
}

moving_picture() {
	! still_picture "$1"
}

# press_until <predicate> <name> <key...>: press until the app shows it
# landed. See PRESSES. The name is what the predicate files its captures
# under, and the predicates that read the log rather than the screen ignore
# it.
press_until() {
	local predicate=$1 name=$2
	shift 2
	local try=0
	while [ "$try" -lt "$PRESSES" ]; do
		key "$@"
		alive || return 1
		"$predicate" "$name" && return 0
		try=$((try + 1))
	done
	return 1
}

# The last report line's presented rate: "play:  12.34 s, 27.34 fps ...".
presented_fps() {
	grep '^play:' "$log" | tail -1 | sed -n 's/.*, \([0-9.]*\) fps presented.*/\1/p'
}

# Nothing is drawn until the first frame, and a key pressed at a window that
# has not been mapped goes nowhere, so every session waits for paint before
# it does anything else. This is also the "it renders" check.
await_paint() {
	local waited=0 shot
	while [ "$waited" -le $((READY * 2)) ]; do
		alive || return 1
		shot=$(grab "$1")
		nonblack "$shot" && return 0
		sleep 0.5
		waited=$((waited + 1))
	done
	return 1
}

# ------------------------------------------------- the checks, with a file

with_media() {
	printf '\n-- playback checks (%s)\n' "$media"
	boot play "$media"

	if ! await '^media:' "$READY"; then
		fail "the file opens" "no media line in $READY s" "log: $log"
		teardown
		return
	fi
	pass "the file opens"

	if await_paint play; then
		pass "the window renders"
	else
		alive || lost "the window renders"
		fail "the window renders" "capture is black: $session/play.ppm" "log: $log"
		teardown
		return
	fi

	# The one check about the frame path rather than the UI. Under a
	# compositor that hands the client no DRM device, wgpu quietly picks a
	# software adapter and the app still draws a picture; this line is how
	# the harness tells the pipeline it means to test from a convincing
	# imitation of it (measured: WLR_RENDERER=pixman does exactly that).
	if await 'dmabuf import: all extensions enabled' "$REPORT"; then
		pass "the zero-copy import is live"
	else
		fail "the zero-copy import is live" \
			"$(grep '^device:' "$log" || echo 'no device line')" \
			"the client fell off the dmabuf path; see $log"
	fi

	if moving_picture playing; then
		pass "the picture moves while playing"
	else
		fail "the picture moves while playing" \
			"two captures 0.7 s apart are identical" \
			"$session/playing-a.ppm" "$session/playing-b.ppm"
	fi

	# Space pauses, and a held frame is the strongest evidence the harness
	# can gather that a key reached the app and did what it says.
	local paused=yes
	if press_until still_picture paused -k space; then
		pass "space pauses"
	else
		alive || lost "space pauses"
		paused=no
		fail "space pauses" "the picture still moved after $PRESSES presses" \
			"$session/paused-a.ppm" "$session/paused-b.ppm"
	fi

	# Before saves_a_still, because it wants a window with no toast on it and
	# nothing has pressed `s` yet. A paused window keeps its control row for
	# good, which is what lets the two captures differ by the toast alone.
	if [ "$paused" = no ]; then
		skip "a toast is drawn clear of the controls (nothing paused)"
	else
		toast_clears_the_controls
	fi

	zooms_out_to_the_ball
	saves_a_still
	a_ball_still_has_a_black_room
	# All three before `i`, which prints view lines of its own: while the only
	# thing that has printed one is a capture, the two counts can be compared.
	a_still_says_where_it_was_looking
	copies_the_view
	returns_to_the_copied_view
	flips_the_horizon
	survives_fullscreen
	fullscreen_holds_the_view
	the_room_belongs_to_the_window
	# While the window is still paused, which is what makes two captures of it
	# differ by the popup and nothing else.
	volume_popup_closes_on_a_click "$paused"

	# Space again, and this time the app's own report line is the evidence.
	# A file that never paused is still playing, so its report lines would
	# pass this check while proving nothing: skip it rather than say ok.
	reported=$(grep -c '^play:' "$log")
	if [ "$paused" = no ]; then
		skip "space resumes (nothing paused to resume)"
	elif press_until more_report_lines resumed -k space; then
		pass "space resumes ($(presented_fps) fps presented)"
	else
		alive || lost "space resumes"
		fail "space resumes" \
			"report lines: $reported before, $(grep -c '^play:' "$log") after" \
			"log: $log"
	fi

	idle_controls_hold_the_view
	opens_onto_the_backdrop
	exits_clean
}

# Report lines counted before the resume key, for the check below.
reported=0

# The report subscription only runs while playing, so a new line is the app
# saying it is playing again. The rate in it has to be a real number of
# frames: a resumed player that presents nothing is not resumed.
more_report_lines() {
	local waited=0
	while [ "$waited" -le $((REPORT * 2)) ]; do
		alive || return 1
		if [ "$(grep -c '^play:' "$log")" -gt "$reported" ] && [ "$(presented_fps)" != 0.00 ]; then
			return 0
		fi
		sleep 0.5
		waited=$((waited + 1))
	done
	return 1
}

# A message that says a capture landed must not be drawn over anything the
# pilot presses. The window is paused, so its picture and its control row are
# the same bytes in both captures and everything that differs between them is
# the toast: the header band and the control band have to come through
# untouched, and something has to have changed somewhere, or the check has
# proved nothing.
#
# PR #74 is why this exists. The stock toaster's overlay is nailed 15 px above
# the bottom of the window whatever it is mounted over (libcosmic
# `src/widget/toaster/widget.rs:199-215`), which put every capture message
# straight across the scrubber, and nothing mechanical noticed.
toast_clears_the_controls() {
	local check="a toast is drawn clear of the controls"
	local before=$session/toast-before.ppm after=$session/toast-up.ppm
	local shots try=0

	grab toast-before >/dev/null
	shots=$(grep -c '^shot:' "$log")
	while [ "$try" -lt "$PRESSES" ]; do
		key -k s
		alive || lost "$check"
		[ "$(grep -c '^shot:' "$log")" -gt "$shots" ] && break
		try=$((try + 1))
	done
	if [ "$(grep -c '^shot:' "$log")" -le "$shots" ]; then
		fail "$check" "no capture landed after $PRESSES presses of s" "log: $log"
		return
	fi
	grab toast-up >/dev/null

	if cmp -s "$before" "$after"; then
		fail "$check" "the two captures are identical" \
			"nothing was on screen to check, so this proves nothing" \
			"$before" "$after"
		return
	fi

	local over=
	band_changed "$before" "$after" top "$HEADER_BAND" && over="the header bar"
	band_changed "$before" "$after" bottom "$CONTROL_BAND" &&
		over="${over:+$over and }the control row"
	if [ -z "$over" ]; then
		pass "$check"
	else
		fail "$check" "the toast is drawn over $over" "$before" "$after"
	fi
}

# Ctrl+- keeps zooming out past the flat range now, until the whole sphere
# is a ball with room around it (issue #47). The room is the one thing in
# the window whose colour the app decides rather than the footage: flat and
# neutral, where a frame of video at the same place is neither. So the check
# reads one patch a fourteenth of the way in from the left, level with the
# middle, which is outside the ball at the far end of the zoom and inside the
# picture at the default view.
#
# Presses one at a time and looks after each, because the zoom is a clamp
# rather than a state: pressing past the end is free, and a dropped key
# costs a press rather than the check.
# Measured on this harness's 1280x672 widget: the room reaches the patch on
# the fifth press (90, 129, 185, 265, 380, 544 degrees, and the patch is off
# the ball past about 403). wtype drops about one key in twenty, so the rest
# is headroom, and the loop breaks on success so headroom costs nothing.
BALL_PRESSES=20
# The room is flat and dark whichever treatment it has (issue #100): the
# window's own pane behind the video, which reads 27 27 27 under cage, and
# pure black in fullscreen and in a saved still. So a patch is the room when
# it is both nearly neutral and nearly black, which is what these two numbers
# say. They cannot tell the two treatments apart, and nothing here asks them
# to: `the room belongs to the window` is the check that does.
#
# Two measurements set them and they were made against different failures, so
# the merge takes the tighter half of each rather than one side:
#
# - spread 1, not 4. On the owner's deck capture the foliage at the ball's rim
#   reads 15 19 15, a spread of exactly 4, so at 4 the zoom-out loop stopped
#   one press early on a patch that was picture and `fullscreen holds the
#   view` then failed on the same patch reading 18 22 17.
# - dark 30, not 60. Measured 2026-07-31, a hillside in shadow read 35 35 36 a
#   fifth of the way out of the zoom, so at 60 `reach_the_ball` took it for the
#   room and stopped there, and every check downstream compared two pictures
#   that were never at the ball.
#
# Both exclusions hold at once: the foliage fails on spread (4 > 1) and the
# hillside on brightness (35 > 30), while the room itself passes both at
# 27 27 27 and at 0 0 0, spread 0 either way.
ROOM_SPREAD=1
ROOM_DARK=30

# reach_the_ball <name>: press ctrl+- until the patch reads the room, and
# say whether it got there. The capture it went by is left under <name>.
reach_the_ball() {
	local try=0
	while [ "$try" -lt "$BALL_PRESSES" ]; do
		key -M ctrl -k minus -m ctrl
		alive || return 1
		is_room "$(patch_rgb "$(grab "$1")")" && return 0
		try=$((try + 1))
	done
	return 1
}

zooms_out_to_the_ball() {
	local before after
	before=$(patch_rgb "$(grab zoom-before)")
	reach_the_ball zoom-ball
	alive || lost "ctrl+- zooms out to the ball"
	after=$(patch_rgb "$session/zoom-ball.ppm")
	if ! is_room "$after"; then
		fail "ctrl+- zooms out to the ball" \
			"after $BALL_PRESSES presses the patch reads $after, which is not the room \
around the ball" "$session/zoom-ball.ppm"
		return
	fi
	pass "ctrl+- zooms out to the ball (patch $after, was $before)"

	key -M ctrl -k 0 -m ctrl
	after=$(patch_rgb "$(grab zoom-reset)")
	if is_room "$after"; then
		fail "ctrl+0 comes back from the ball" \
			"the patch still reads $after, which is the room around the ball" \
			"$session/zoom-reset.ppm"
	else
		pass "ctrl+0 comes back from the ball (patch $after)"
	fi
}

# The mean colour of a small patch of a capture, as three decimal codes.
# `scale=1:1` is the averaging, and rawvideo is what makes `od` the whole
# reader.
patch_rgb() {
	ffmpeg -y -loglevel error -i "$1" \
		-vf "crop=iw*0.05:ih*0.05:iw*0.07:ih*0.47,scale=1:1" \
		-f rawvideo -pix_fmt rgb24 - 2>>"$log" | od -An -tu1 | tr -s ' ' | sed 's/^ //;s/ $//'
}

# The same, over the 64 px mark the welcome view draws above its text. The
# box is where that icon lands at 1280x720, which is the size of this
# harness's only output: rows 312 to 371, centred across the window.
mark_rgb() {
	ffmpeg -y -loglevel error -i "$1" \
		-vf "crop=64:64:(iw-64)/2:312,scale=1:1" \
		-f rawvideo -pix_fmt rgb24 - 2>>"$log" | od -An -tu1 | tr -s ' ' | sed 's/^ //;s/ $//'
}

# How far apart the mark's channels have to be to be called a colour drawing
# rather than a symbolic icon. A symbolic icon is filled with exactly one
# grey, so its patch has a spread of zero whatever the theme. Measured over
# that box: 94 153 138 dark and 130 189 174 light for the app icon this
# draws now, against 156 156 156 and 77 77 77 for the
# `video-x-generic-symbolic` it replaced (issue #93).
MARK_SPREAD=20

# Whether a patch is the room around the ball rather than a piece of picture:
# dark, and the three channels within a code or two of each other. Measured on
# this harness, the room reads 27 27 27 in a window, which is the theme's
# window background showing through the transparent room (issue #100), and
# 0 0 0 in fullscreen; the same patch of the default view read 185 224 241 on
# the owner's footage, which is sky.
is_room() {
	local rgb=($1) hi lo
	[ "${#rgb[@]}" = 3 ] || return 1
	hi=$(printf '%s\n' "${rgb[@]}" | sort -n | tail -1)
	lo=$(printf '%s\n' "${rgb[@]}" | sort -n | head -1)
	[ "$hi" -le "$ROOM_DARK" ] && [ $((hi - lo)) -le "$ROOM_SPREAD" ]
}

# How far off black a patch may be and still be called black. A capture off
# the compositor is lossless and reads exactly 0 0 0; a JPEG is not, and this
# is the whole of the allowance it gets. It uses none of it: measured 0 0 0
# over a patch that sits a fifth of the frame inside the room, far from the
# ball's rim and any ringing around it.
BLACK_MAX=2

is_black() {
	local rgb=($1) hi
	[ "${#rgb[@]}" = 3 ] || return 1
	hi=$(printf '%s\n' "${rgb[@]}" | sort -n | tail -1)
	[ "$hi" -le "$BLACK_MAX" ]
}

# The mark above "No video open" is the app icon, which the binary carries as
# bytes (`crates/app/src/app.rs`, `APP_ICON`) because this run has no icon
# theme to look a name up in. Colour in that patch is what separates the
# drawing from the symbolic icon it replaced, and from an SVG that failed to
# load, which draws nothing at all.
#
# That is also how the bytes were kept at issue #75's rename rather than
# dropped for `icon::from_name(APP_ID)`: this check reads 27 27 27, one flat
# grey, for a name lookup here, and colour for the same build with the icon
# tree on XDG_DATA_DIRS. It catches a missing drawing, which is what makes
# the negative worth anything.
#
# It says nothing about the window background, which the same issue changed:
# under cage there is no compositor blur, so the theme's opaque background is
# painted either way and the two builds' captures are identical outside this
# box (measured: 3596 differing pixels, all of them inside it).
draws_the_app_icon() {
	local rgb hi lo
	rgb=$(mark_rgb "$(grab welcome-mark)")
	local channels=($rgb)
	if [ "${#channels[@]}" != 3 ]; then
		fail "the welcome view draws the app icon" \
			"no patch could be read from $session/welcome-mark.ppm"
		return
	fi
	hi=$(printf '%s\n' "${channels[@]}" | sort -n | tail -1)
	lo=$(printf '%s\n' "${channels[@]}" | sort -n | head -1)
	if [ $((hi - lo)) -ge "$MARK_SPREAD" ]; then
		pass "the welcome view draws the app icon (mark $rgb)"
	else
		fail "the welcome view draws the app icon" \
			"the mark reads $rgb, which is one grey: a symbolic icon, or nothing" \
			"$session/welcome-mark.ppm"
	fi
}

# `s` writes a still into the session's screenshots folder, which is inside
# scratch/ and not the developer's own. The name it looks for is the one the
# app writes: a JPEG since issue #15, and ffmpeg reads what it finds, so a
# file that is not the format its name claims fails the decode below.
saves_a_still() {
	local still shrunk=$session/still.ppm
	local try=0
	while [ "$try" -lt "$PRESSES" ]; do
		key -k s
		alive || lost "s saves a still"
		still=$(find "$session/shots" -name '*.jpg' | head -1)
		[ -n "$still" ] && break
		try=$((try + 1))
	done
	if [ -z "$still" ]; then
		fail "s saves a still" "nothing appeared in $session/shots after $PRESSES presses"
		return
	fi
	ffmpeg -y -loglevel error -i "$still" -vf scale=160:-1 "$shrunk" 2>>"$log"
	if nonblack "$shrunk"; then
		pass "s saves a still"
	else
		fail "s saves a still" "the still is black: $still"
	fi
}

# A still of a ball view (issue #100). In the window the room around the ball
# is transparent and the theme's pane fills it, and a JPEG has no alpha to
# carry that with: the capture pass clears black and the room flattens onto
# that, so what the pilot double clicks months later is the ball on black
# rather than a hole or a grey box.
#
# Two halves, and both are needed. The still has to be a picture at all
# (`nonblack`, which a still of nothing but room would fail), and the room in
# it has to be black. Against the main build of 2026-08-01 the same patch of
# the same still reads 25 25 25, which is what makes the second half a check
# rather than a restatement.
#
# The newest still rather than the first: `saves_a_still` has already written
# one from the default view.
a_ball_still_has_a_black_room() {
	local check="a still of the ball has black around it"
	local shots still room shrunk=$session/ball-still.ppm
	if ! reach_the_ball ball-still-view; then
		alive || lost "$check"
		fail "$check" "the view never reached the ball" "$session/ball-still-view.ppm"
		return
	fi

	shots=$(grep -c '^shot:' "$log")
	local try=0
	while [ "$try" -lt "$PRESSES" ]; do
		key -k s
		alive || lost "$check"
		[ "$(grep -c '^shot:' "$log")" -gt "$shots" ] && break
		try=$((try + 1))
	done
	still=$(ls -t "$session"/shots/*.jpg 2>/dev/null | head -1)
	if [ "$(grep -c '^shot:' "$log")" -le "$shots" ] || [ -z "$still" ]; then
		fail "$check" "no still landed after $PRESSES presses of s" "log: $log"
		return
	fi

	ffmpeg -y -loglevel error -i "$still" -vf scale=160:-1 "$shrunk" 2>>"$log"
	room=$(patch_rgb "$still")
	if ! nonblack "$shrunk"; then
		fail "$check" "the still is black all through, so there is no ball in it" "$still"
	elif is_black "$room"; then
		pass "$check (room $room)"
	else
		fail "$check" "the room in the still reads $room, which is not black" "$still"
	fi

	# Back to the default view, which is where the checks after this one start
	# from. A dropped key leaves them at the ball, which costs them nothing:
	# every one of them moves the view itself.
	key -M ctrl -k 0 -m ctrl
}

# Set by the check below, read by its two predicates: what the room reads in a
# window, measured rather than assumed, so nothing here holds a theme colour.
room_pane=

room_is_black() {
	is_black "$(patch_rgb "$(grab "$1")")"
}

room_is_the_pane() {
	[ "$(patch_rgb "$(grab "$1")")" = "$room_pane" ]
}

# Issue #100: the room around the ball belongs to the window, not to the pass.
#
# The pass writes that room transparent, and what shows through it is whatever
# the shell put behind the video: nothing in a window, so libcosmic's own pane
# is what fills it, and black in fullscreen, where there is no desktop behind
# the window for a pane to be translucent over. The view does not move across
# any of this, so what the patch reads is the treatment and nothing else.
#
# This is the check a pass that paints its own colour there fails: against the
# main build of 2026-08-01 the three captures read 25 25 25 all through, and
# this requires the middle one to be black and the two ends to be the pane.
#
# What it cannot see is the blur. cage implements no
# `ext-background-effect-v1`, so the theme is never frosted under this harness
# and libcosmic paints its pane opaque whichever branch it takes (the same
# limit `draws_the_app_icon` records). That the room is the pane rather than a
# colour of the app's own is what is under test here; that the pane is
# translucent over a blurred desktop is libcosmic's own line, and the owner's
# eyes.
the_room_belongs_to_the_window() {
	local check="the room around the ball belongs to the window"
	if ! reach_the_ball room-pane; then
		alive || lost "$check"
		fail "$check" "the view never reached the ball" "$session/room-pane.ppm"
		return
	fi
	room_pane=$(patch_rgb "$session/room-pane.ppm")
	if is_black "$room_pane"; then
		fail "$check" "the windowed room already reads $room_pane, which is the \
fullscreen treatment: nothing below could tell the two apart" "$session/room-pane.ppm"
		return
	fi

	if ! press_until room_is_black room-black -k f; then
		alive || lost "$check"
		fail "$check" "in fullscreen the room reads $(patch_rgb "$session/room-black.ppm"), \
which is not black" "$session/room-black.ppm"
		key -k Escape
		return
	fi
	if ! press_until room_is_the_pane room-pane-again -k Escape; then
		alive || lost "$check"
		fail "$check" "leaving fullscreen left the room at \
$(patch_rgb "$session/room-pane-again.ppm") rather than the pane's $room_pane" \
			"$session/room-pane-again.ppm"
		return
	fi
	pass "$check (pane $room_pane, fullscreen $(patch_rgb "$session/room-black.ppm"))"
}

# ------------------------------------------- the volume popup, and the pointer
#
# Issue #126, owner-reported: the volume popup stayed up until the speaker
# button was pressed again. cosmic-player takes a press anywhere in the video
# as the way out of an open dropdown (`src/main.rs:1771-1773`, `1507-1513`)
# and the owner ruled that we follow it, so this is that press.
#
# The window is paused, so two captures of it are the same bytes and
# everything that differs between them is what the pointer did. The band is
# the bottom of the window: measured 2026-08-01 against the main build, the
# popup is 240 px wide by 50 rows tall and sits at rows 622 to 671, which is
# directly above the 48-row control row, and the two captures differ in
# 11,960 pixels all of them inside it.
#
# Both halves are needed and the first is what makes the second worth
# anything: a run where the speaker button drew nothing has not shown a popup
# being dismissed, it has shown a window with no popup in it.
VOLUME_BUTTON=1252
CONTROL_ROW=696
# A point in the video clear of both the popup and the control row, which is
# where the pointer sits for the captures either side of the popup as well as
# for the click that has to close it.
VIDEO_X=320
VIDEO_Y=360
# Control row plus popup plus room above it.
POPUP_BAND=144

volume_popup_closes_on_a_click() {
	local check="a click in the video closes the volume popup"
	if [ "${1:-no}" = no ]; then
		skip "$check (nothing paused)"
		return
	fi
	if said 'playing silently'; then
		skip "$check (no sound device here, so the speaker button is disabled)"
		return
	fi
	# A toast still on screen is a second thing that can change between two
	# captures, and it is in the band nothing else is.
	sleep "$TOAST_GONE"

	local parked popup gone try=0
	poke "$VIDEO_X" "$VIDEO_Y"
	parked=$(grab volume-parked)
	while [ "$try" -lt "$PRESSES" ]; do
		poke "$VOLUME_BUTTON" "$CONTROL_ROW" click
		alive || lost "$check"
		popup=$(grab volume-up)
		band_changed "$parked" "$popup" bottom "$POPUP_BAND" && break
		try=$((try + 1))
	done
	if ! band_changed "$parked" "$popup" bottom "$POPUP_BAND"; then
		fail "$check" "the speaker button drew no popup after $PRESSES clicks" \
			"$parked" "$popup" "log: $log"
		return
	fi

	poke "$VIDEO_X" "$VIDEO_Y" click
	alive || lost "$check"
	gone=$(grab volume-gone)
	if band_changed "$parked" "$gone" bottom "$POPUP_BAND"; then
		fail "$check" "the popup is still up after a click in the video" \
			"$parked" "$gone"
		return
	fi
	pass "$check"
}

# The arguments half of a view line, which is `reframe`'s own syntax: the same
# keys in the same order, and each number printed to the places
# `crates/render/src/framing.rs` prints it to. The round trip through
# reframe's parser is a unit test; what this adds is that the line reaching a
# terminal is that line and not a debug print near it.
VIEW_ARGS='time=[0-9]+\.[0-9]{3} yaw=-?[0-9]+\.[0-9]{2} pitch=-?[0-9]+\.[0-9]{2}'
VIEW_ARGS="$VIEW_ARGS fov=-?[0-9]+\.[0-9]{2} lock=[01]"

# view <n> -> the nth-from-last view line, with its label taken off.
view_line() {
	grep '^view:' "$log" | tail -"${1:-1}" | head -1 | sed 's/^view:[[:space:]]*//'
}

# A still carries the video and the timecode in its file name and no direction
# anywhere, so a capture the pilot sends back months later is only placeable if
# the terminal said where it was looking. Every capture prints one line, which
# while nothing has pressed `i` yet means the two counts are equal.
a_still_says_where_it_was_looking() {
	local check="every still prints where it was looking"
	local shots views
	shots=$(grep -c '^shot:' "$log")
	views=$(grep -c '^view:' "$log")
	if [ "$shots" = 0 ]; then
		skip "$check (nothing was captured)"
	elif [ "$views" = "$shots" ]; then
		pass "$check ($views for $shots)"
	else
		fail "$check" "$views view lines for $shots captures" "log: $log"
	fi
}

# Set by copies_the_view before its presses, read by the predicate below.
view_lines=0

more_view_lines() {
	[ "$(grep -c '^view:' "$log")" -gt "$view_lines" ]
}

# `i` copies the view: one line naming the video, the frame and the framing,
# which is what turns "it looks wrong here" into coordinates anyone can
# render.
#
# Two instruments, because they answer different halves. The terminal line
# says the app built the line and carries the whole path, and the clipboard
# says the compositor is holding it, which is the half a pilot actually
# pastes from. Comparing the two also pins the one rule that separates them:
# the copy names the file and never the directories above it, because a
# pilot's report lands in a public issue.
#
# wl-paste rather than a hook: it reads the real selection off the real
# session. cage advertises no wlr-data-control, so this is the ordinary
# focus path, which needs the seat to have a keyboard; the keys pressed
# before this point are what put one there.
copies_the_view() {
	local check="i copies the view"
	local printed args pasted
	view_lines=$(grep -c '^view:' "$log")

	if ! press_until more_view_lines view -k i; then
		alive || lost "$check"
		fail "$check" "no view line after $PRESSES presses of i" "log: $log"
		return
	fi

	printed=$(view_line)
	args=${printed#"$media "}
	if [ "$args" = "$printed" ]; then
		fail "$check" "the printed line does not start with $media" "$printed"
		return
	fi
	if ! printf '%s' "$args" | grep -qE "^$VIEW_ARGS\$"; then
		fail "$check" "these are not reframe's arguments" "$args"
		return
	fi

	if [ "$clipboard" = no ]; then
		pass "$check (terminal only: $args)"
		skip "the view reaches the clipboard (no wl-paste)"
		return
	fi
	pasted=$(env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" \
		wl-paste --no-newline 2>>"$log")
	alive || lost "$check"
	if [ "$pasted" = "$(basename "$media") $args" ]; then
		pass "$check ($pasted)"
	else
		fail "$check" "the clipboard holds: $pasted" \
			"expected: $(basename "$media") $args"
	fi
}

# How long a toast is up, plus a beat. Two captures of the same view are only
# the same bytes once the lines saying so have gone.
TOAST_GONE=6

# Set by returns_to_the_copied_view before its presses, read by the predicate.
goto_lines=0

more_goto_lines() {
	[ "$(grep -c '^goto:' "$log")" -gt "$goto_lines" ]
}

# The whole loop, which is the feature: copy a view, wander off, paste, and be
# back exactly where the copy was taken.
#
# The window is paused here, so the picture is a function of the frame and the
# camera alone and two captures of one view are the same bytes. Two
# instruments, and the first is the stronger: the app's own copied line is
# exact to the millisecond and the hundredth of a degree, where a capture only
# says the pixels came out the same. Both, because a line that matches while
# the picture does not would mean the view is not what the line says it is.
#
# The wander is a ten second seek and a notch of zoom out: one moves the frame
# and the other moves the camera, which are the two halves a paste has to put
# back. A dropped key costs the check nothing, because the capture taken
# afterwards has to differ from the first or the check says so itself.
returns_to_the_copied_view() {
	local check="ctrl+v goes back to the copied view"
	local copied returned

	view_lines=$(grep -c '^view:' "$log")
	if ! press_until more_view_lines goto -k i; then
		alive || lost "$check"
		fail "$check" "no view line after $PRESSES presses of i" "log: $log"
		return
	fi
	copied=$(view_line)
	sleep "$TOAST_GONE"
	grab goto-there >/dev/null

	key -k Right
	key -M ctrl -k minus -m ctrl
	alive || lost "$check"
	grab goto-away >/dev/null
	if same_picture "$session/goto-there.ppm" "$session/goto-away.ppm"; then
		fail "$check" "the seek and the zoom moved nothing, so this proves nothing" \
			"$session/goto-there.ppm" "$session/goto-away.ppm"
		return
	fi

	goto_lines=$(grep -c '^goto:' "$log")
	if ! press_until more_goto_lines goto -M ctrl -k v -m ctrl; then
		alive || lost "$check"
		fail "$check" "no goto line after $PRESSES presses of ctrl+v" "log: $log"
		return
	fi
	sleep "$TOAST_GONE"
	grab goto-back >/dev/null

	view_lines=$(grep -c '^view:' "$log")
	if ! press_until more_view_lines goto -k i; then
		alive || lost "$check"
		fail "$check" "no view line after the paste" "log: $log"
		return
	fi
	returned=$(view_line)

	if [ "$returned" != "$copied" ]; then
		fail "$check" "copied:   $copied" "came back: $returned"
		return
	fi
	if ! same_picture "$session/goto-there.ppm" "$session/goto-back.ppm"; then
		fail "$check" "the line came back but the picture did not" \
			"$session/goto-there.ppm" "$session/goto-back.ppm"
		return
	fi
	pass "$check (${copied#"$media "})"
}

# `h` flips the horizon lock, which is on by default, so the session's config
# ends up saying false. Reading the config rather than the picture is what
# separates a key that landed from a key that was swallowed: the two views
# differ by a rotation that one still frame need not show.
flips_the_horizon() {
	local locked=$session/config/cosmic/dev.harding.Kjerag/v1/horizon_lock
	local try=0
	while [ "$try" -lt "$PRESSES" ]; do
		key -k h
		alive || lost "h flips the horizon lock"
		if [ "$(cat "$locked" 2>/dev/null)" = false ]; then
			pass "h flips the horizon lock"
			return
		fi
		try=$((try + 1))
	done
	fail "h flips the horizon lock" \
		"expected false in $locked, found: $(cat "$locked" 2>/dev/null)"
}

# `f` asks for fullscreen. Under cage the window already fills the only
# output, so there is nothing to see: what is checked is that asking does not
# take the app down or stop it drawing. A dropped key would pass this check,
# which is the honest limit of it.
survives_fullscreen() {
	key -k f
	local shot
	shot=$(grab fullscreen)
	if alive && nonblack "$shot"; then
		pass "f survives fullscreen"
	else
		fail "f survives fullscreen" "alive: $(alive && echo yes || echo no)" "$shot"
	fi
	# Back to windowed, so the checks after this one start where they expect.
	key -k Escape
}

# Issue #77: the view survives fullscreen, both ways, every time.
#
# Entering fullscreen hides the header bar, which changes the shape of the
# window's widget tree, so the camera must not live anywhere that is rebuilt
# with it.
#
# The zoom is what stands in for the pan here, because the zoom is the whole
# of what a keyboard can move: yaw, pitch and field of view are one
# `Viewpoint` in one place, and a transition that keeps one keeps all three.
# The instrument is the room around the ball (`is_room`), which is flat and
# dark rather than footage, so it reads as the room however the window is
# shaped underneath and whether the picture is paused or playing. Its two
# treatments both pass that test (issue #100: the theme's pane in a window,
# black in fullscreen), which is what keeps this check about the view while
# `the room belongs to the window` is about the treatments.
#
# The two ways in a keyboard has, and the two ways out. The other three ways
# in - the View menu, the button in the control row, a double click on the
# video - send this same message and need a pointer the harness has not got.
# A dropped key leaves the view where it already was, so it can only cost a
# real failure, never invent one.
fullscreen_holds_the_view() {
	if ! reach_the_ball fullscreen-ball; then
		alive || lost "fullscreen holds the view"
		fail "fullscreen holds the view" \
			"the view never reached the ball" "$session/fullscreen-ball.ppm"
		return
	fi

	held_the_view f -k f &&
		held_the_view escape -k Escape &&
		held_the_view alt-enter -M alt -k Return -m alt &&
		held_the_view f-again -k f &&
		pass "fullscreen holds the view (f, escape, alt+enter, f)"

	# However that went, leave the window windowed: the check after this one
	# is about the header bar, which fullscreen keeps hidden whatever else
	# happens.
	key -k Escape
}

# Issue #77 again, on the path with no fullscreen in it: the header bar also
# goes away on its own, two seconds after the last pointer input, while
# playing (`CONTROLS_TIMEOUT`). It is the header bar coming and going that
# reshapes the window, so this is the same defect met the way a pilot meets
# it most often - watching a video, hands off - and it is what says the cause
# is the header bar rather than anything about fullscreen.
#
# `h` is pressed first because it is a message that shows the controls, so
# the bar is known to be up before the wait. It leaves the horizon lock back
# on, which nothing after this reads.
CONTROLS_HIDE=4

idle_controls_hold_the_view() {
	if ! reach_the_ball idle-ball; then
		alive || lost "the view survives the controls hiding"
		fail "the view survives the controls hiding" \
			"the view never reached the ball" "$session/idle-ball.ppm"
		return
	fi

	# The bar is up from here and nothing is pressed after it, so the two
	# seconds run out inside the wait below and the hide is inside the
	# check rather than before it.
	key -k h
	local shown hidden
	shown=$(patch_rgb "$(grab idle-shown)")
	sleep "$CONTROLS_HIDE"
	alive || lost "the view survives the controls hiding"
	hidden=$(patch_rgb "$(grab idle-hidden)")

	if ! is_room "$shown"; then
		fail "the view survives the controls hiding" \
			"showing the controls moved the view: the patch reads $shown, which is \
picture rather than the room around the ball" "$session/idle-shown.ppm"
	elif is_room "$hidden"; then
		pass "the view survives the controls hiding"
	else
		fail "the view survives the controls hiding" \
			"after $CONTROLS_HIDE s with no input the patch reads $hidden, which is \
picture rather than the room around the ball" "$session/idle-hidden.ppm"
	fi
}

# held_the_view <name> <key...>: press it, and the view is still at the ball.
held_the_view() {
	local name=$1 patch file
	shift
	key "$@"
	alive || lost "fullscreen holds the view"
	file=$(grab "fullscreen-$name")
	patch=$(patch_rgb "$file")
	is_room "$patch" && return 0
	fail "fullscreen holds the view" \
		"the view reset on $name: the patch reads $patch, which is picture rather \
than the room around the ball" "$file"
	return 1
}

# ------------------------------------- the pane before the first frame
#
# A pane with no frame in it yet draws the backdrop, which is the same one the
# room around the ball floats on (issue #100) - libcosmic's own pane in a
# window, black in fullscreen - and never a picture of the pass's own. What it
# drew until this check existed was an animated test pattern left over from
# the first bring-up of the shader (owner-reported, 2026-08-01).
#
# The command line cannot show it. `kjerag file.insv` opens the file while the
# window is still being mapped, so the decode thread has the whole mapping to
# work in and the first frame is already there when the first pixel is drawn:
# measured 2026-08-01 over 80 captures from launch, none of them caught a pane
# with no frame. A window that is already up draws its pane the instant the
# file is loaded, frame or no frame, and that is the pilot's own path.
#
# Ctrl+V is the only way into it from a keyboard: Ctrl+O opens a portal file
# dialog and cage has no portal behind it. The pasted line names the file with
# its directories, which is what makes a paste an open rather than a seek, and
# it names a time near the end of the file, which is what makes the gap wide
# enough to photograph: the first frame after a paste at 90% comes off a
# keyframe walk deep in the file rather than off the head of the stream.
# Measured on the main build of 2026-08-01: one capture caught the gap at
# time=0, twenty caught it at time=1500 of a 1799.8 s file.

# How many captures are taken once the app says it has loaded the file, and
# how many times the whole open is repeated when a round catches nothing of
# the gap. grim answers in about 17 ms here, so a burst covers 0.4 s; the gap
# has measured 0.33 s on a cold window and 0.05 s on a warm one, and a round
# whose every capture landed after the first frame has seen nothing rather
# than seen something good.
OPEN_BURST=24
OPEN_ROUNDS=3

# What separates the test pattern from a picture and from the backdrop.
#
# The pattern is a sine of the distance from the middle of the view, put into
# blue whole, into green by the vertical place and into red by the horizontal
# one, so two patches at mirrored places read the same green and the same blue
# to the byte and different reds. Measured on the main build of 2026-08-01:
# 41 97 210 on the left against 169 97 210 on the right. A frame of video is
# not mirror symmetric (120 190 242 against 181 213 224, same run) and the
# backdrop is flat, which is symmetric in all three channels rather than two.
PATTERN_RED=20
PATTERN_MATCH=1

# The band of the window that is pane and nothing else: under the header bar,
# well above the welcome view's icon, and further still above any toast.
#
# `flags=area` and not the default, which is what `patch_rgb` above leaves
# alone: bicubic overshoots on a downscale this steep, and measured on a flat
# 27 27 27 capture of this window it answers 30 26 30, a spread of 4 where the
# picture has none. The area scaler is the mean itself, which is what both
# readings below are asking for.
pane_rgb() {
	ffmpeg -y -loglevel error -i "$1" \
		-vf "crop=iw:ih*0.25:0:$HEADER_BAND,scale=1:1:flags=area" \
		-f rawvideo -pix_fmt rgb24 - 2>>"$log" | od -An -tu1 | tr -s ' ' | sed 's/^ //;s/ $//'
}

# mirror_rgb <file> <left|right>: one of two patches at places the middle of
# the window reflects onto each other.
mirror_rgb() {
	local at=0.15
	[ "$2" = left ] || at=0.75
	ffmpeg -y -loglevel error -i "$1" \
		-vf "crop=iw*0.10:ih*0.10:iw*$at:ih*0.45,scale=1:1:flags=area" \
		-f rawvideo -pix_fmt rgb24 - 2>>"$log" | od -An -tu1 | tr -s ' ' | sed 's/^ //;s/ $//'
}

is_test_pattern() {
	local left=($1) right=($2) dr dg db
	[ "${#left[@]}" = 3 ] && [ "${#right[@]}" = 3 ] || return 1
	dr=$((left[0] - right[0]))
	dg=$((left[1] - right[1]))
	db=$((left[2] - right[2]))
	[ "${dr#-}" -ge "$PATTERN_RED" ] &&
		[ "${dg#-}" -le "$PATTERN_MATCH" ] &&
		[ "${db#-}" -le "$PATTERN_MATCH" ]
}

# The pane holds nothing but pane: the window with its file closed, and the
# window with one open whose first frame has not arrived.
pane_is_backdrop() {
	is_room "$(pane_rgb "$1")"
}

pane_is_clear() {
	pane_is_backdrop "$(grab "$1")"
}

opens_onto_the_backdrop() {
	local check="an open with no frame yet draws the backdrop"
	if [ "$clipboard" = no ]; then
		skip "$check (no wl-copy)"
		return
	fi

	local seconds deep round=1
	seconds=$(sed -n 's/^media:.*, \([0-9.]*\) s$/\1/p' "$log" | head -1)
	deep=$(awk -v s="${seconds:-0}" 'BEGIN { printf "%.3f", s * 0.9 }')

	while [ "$round" -le "$OPEN_ROUNDS" ]; do
		open_once "$check" "$deep" || return
		case $verdict in
		pattern)
			fail "$check" \
				"the pane drew the test pattern: its mirrored halves read \
$(mirror_rgb "$caught" left) and $(mirror_rgb "$caught" right), which no picture is" \
				"$caught"
			return
			;;
		backdrop)
			pass "$check ($held captures of backdrop, then the frame)"
			return
			;;
		esac
		round=$((round + 1))
	done
	fail "$check" \
		"$OPEN_ROUNDS opens went by with every capture already a frame, so nothing \
was seen of the gap and this proves nothing" "$caught"
}

# Set by open_once, read by the check around it: what the burst caught, the
# capture that says so, and how much backdrop was in it.
verdict=
caught=
held=0

# open_once <check> <time>: close the file, open it again from the window that
# is still up, and classify the burst of captures that follows. Answers
# through `verdict`; a non-zero return is a failure it has already filed.
open_once() {
	local check=$1 deep=$2 taken shot
	verdict=missed
	caught=
	held=0

	# The default view first: the band this check reads is the room itself
	# while the view is at the ball, whatever is playing behind it.
	key -M ctrl -k 0 -m ctrl
	if ! press_until pane_is_clear closed -M ctrl -k w -m ctrl; then
		alive || lost "$check"
		fail "$check" \
			"ctrl+w left the pane reading $(pane_rgb "$session/closed.ppm"), which is \
still a picture" "$session/closed.ppm"
		return 1
	fi

	env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" \
		wl-copy "$media time=$deep yaw=0.00 pitch=0.00 fov=90.00 lock=1" 2>>"$log"

	# No settle after the key, and the burst starts on the app's own line
	# rather than on a sleep: all of what this check is about happens in the
	# fraction of a second after `goto:` is printed, which is the moment the
	# file has been loaded and the pane is about to be drawn for the first
	# time.
	local gotos try=0 waited
	gotos=$(grep -c '^goto:' "$log")
	while [ "$try" -lt "$PRESSES" ]; do
		env XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock" \
			wtype -M ctrl -k v -m ctrl 2>>"$log"
		waited=0
		while [ "$(grep -c '^goto:' "$log")" -le "$gotos" ] &&
			[ "$waited" -lt $((READY * 100)) ]; do
			alive || lost "$check"
			sleep 0.01
			waited=$((waited + 1))
		done
		[ "$(grep -c '^goto:' "$log")" -gt "$gotos" ] && break
		try=$((try + 1))
	done
	if [ "$(grep -c '^goto:' "$log")" -le "$gotos" ]; then
		fail "$check" "no goto line after $PRESSES presses of ctrl+v" "log: $log"
		return 1
	fi

	for taken in $(seq 1 "$OPEN_BURST"); do
		alive || lost "$check"
		grab "$(printf 'open-%02d' "$taken")" >/dev/null
	done

	for taken in $(seq 1 "$OPEN_BURST"); do
		shot=$(printf '%s/open-%02d.ppm' "$session" "$taken")
		[ -s "$shot" ] || continue
		# The control row is what says the file is open, because the window
		# the paste came from had none: its bottom band is pane and this one
		# is a row of buttons.
		band_changed "$session/closed.ppm" "$shot" bottom "$CONTROL_BAND" || continue
		if pane_is_backdrop "$shot"; then
			held=$((held + 1))
			verdict=backdrop
			caught=$shot
			continue
		fi
		if is_test_pattern "$(mirror_rgb "$shot" left)" "$(mirror_rgb "$shot" right)"; then
			verdict=pattern
			caught=$shot
		elif [ -z "$caught" ]; then
			# A frame, and nothing before it: the burst started after the gap
			# had closed, and this capture is what says so.
			caught=$shot
		fi
		# Either way the gap is over: the frame has landed, or the pattern
		# this check exists for is on screen and there is nothing to add to
		# it.
		break
	done

	# One capture of the verdict is kept and the rest of the burst goes: two
	# dozen captures a round of a window full of somebody's flight is a lot of
	# personal video to leave lying about for no reading.
	if [ -n "$caught" ]; then
		cp "$caught" "$session/open-$verdict.ppm"
		caught=$session/open-$verdict.ppm
	fi
	rm -f "$session"/open-[0-9][0-9].ppm
}

exits_clean() {
	local status
	quit
	status=$?
	if [ "$status" = 0 ]; then
		pass "ctrl+q exits clean"
	else
		fail "ctrl+q exits clean" "the session left with $status, 0 expected" "log: $log"
	fi
}

# ------------------------------- the checks, with a pool to remember
#
# The seam correction is pooled per camera, and nothing asks the pilot for it
# (AGENTS.md, zero-config playback). The claim under test is that a camera the
# pool knows is corrected before the first frame rather than two seconds into
# it, so it is checked through the app: open the file once with an empty pool
# and read the camera out of the report line, write a pool for that camera into
# the session's own state directory, open it again, and require the app to say
# it drew from the pool and never to say it was fitting.
#
# It is also the control for the failure it clears: with the first session's
# empty pool, the app says it is fitting off the file, which is the line the
# second session must not print.

pooled_calibration() {
	local check="a pooled calibration is in the first frame"
	printf '\n-- calibration checks (%s)\n' "$media"

	# The session's directories persist between runs, so a pool a previous run
	# stored is still there and would make this session take the path it is
	# here to rule out. The superseded single-entry key goes too: it is
	# derived data and nothing reads it any more.
	local state=$STATE_HOME/cosmic/dev.harding.Kjerag/v1
	rm -f "$state/seam_pool" "$state/seam_calibration"

	boot fallback "$media"
	if ! await '^seam:' "$READY"; then
		fail "$check" "no seam line in $READY s" "log: $log"
		teardown
		return
	fi
	local camera
	camera=$(sed -n 's/^lens:.*camera \([0-9a-f]*\).*/\1/p' "$log" | head -1)
	if ! grep -q 'nothing pooled for this camera yet' "$log"; then
		fail "$check" "the empty pool did not fall back to fitting" "log: $log"
		teardown
		return
	fi
	quit >/dev/null 2>&1 || teardown
	if [ -z "$camera" ]; then
		fail "$check" "no camera key in the lens line" "log: $log"
		return
	fi

	# The owner's own answer, which is what 6.8 fitted on his static capture,
	# as a pool of one. Any five numbers would do here: what is under test is
	# that they reach the first frame, not what they are.
	mkdir -p "$state"
	printf '{"%s":(samples:[(roll_deg:0.789,yaw_deg:-2.450,pitch_deg:-0.668,cx_px:-2.55,cy_px:-13.84,patches:13,residual_deg:0.108)])}\n' \
		"$camera" >"$state/seam_pool"

	boot calibrated "$media"
	if ! await 'pooled over 1 fits' "$READY"; then
		fail "$check" "the pooled calibration was not read" \
			"$(grep '^seam:' "$log" || echo 'no seam line')" "log: $log"
		teardown
		return
	fi
	if grep -q 'nothing pooled for this camera yet' "$log"; then
		fail "$check" "it fitted off the file anyway" "log: $log"
		teardown
		return
	fi
	pass "$check (camera $camera)"
	exits_clean
}

# ------------------------------------------ the checks, with nothing open

welcome() {
	printf '\n-- welcome view checks (no media)\n'
	boot welcome

	if await_paint welcome; then
		pass "the window renders with nothing open"
	else
		alive || lost "the window renders with nothing open"
		fail "the window renders with nothing open" \
			"capture is black: $session/welcome.ppm" "log: $log"
		teardown
		return
	fi

	draws_the_app_icon
	flips_the_horizon
	survives_fullscreen
	exits_clean
}

# -------------------------------------------------- the checks, with a dud
#
# A file with video in it and no Insta360 trailer is what the app meets when
# someone hands it an ordinary video, and it is the one playback-adjacent
# path that needs no footage: ffmpeg writes the file in a second. What is
# checked is that the refusal is a message and not a crash.

dud() {
	printf '\n-- rejected file checks (synthetic)\n'
	local file=$session/not-an-insv.mp4
	if ! ffmpeg -y -loglevel error \
		-f lavfi -i "testsrc=size=320x320:rate=30:duration=1" \
		-f lavfi -i "testsrc=size=320x320:rate=30:duration=1" \
		-map 0:v -map 1:v -c:v libx265 -tag:v hvc1 -preset ultrafast \
		"$file" 2>>"$session/ffmpeg.log"; then
		skip "a file with no trailer is refused (no two-stream HEVC from ffmpeg here)"
		return
	fi

	boot dud "$file"
	if await 'not shown' "$READY"; then
		pass "a file with no trailer is refused"
	else
		fail "a file with no trailer is refused" "nothing said so in $READY s" "log: $log"
	fi

	if await_paint dud; then
		pass "the refusal leaves a window up"
	else
		alive || lost "the refusal leaves a window up"
		fail "the refusal leaves a window up" \
			"capture is black: $session/dud.ppm" "log: $log"
		teardown
		return
	fi

	exits_clean
}

# ------------------------------------- the checks, with another camera's file
#
# A GoPro capture opens: it is an ordinary MP4 with video in it, so the
# refusal cannot come from a decode that failed. It comes from what the
# container holds (`crates/meta/src/format.rs`, issue #107), which is why the
# fixtures here are 71 bytes and no pictures: the app names the file before
# anything is asked to decode it, and a real frame would not change a word of
# what it says. Whether those bytes are what a GoPro really writes is that
# module's question, and it answers it against the local sample corpus.
#
# Two fixtures, because there are two ways a file gets named: the first
# carries GoPro's own `udta` boxes under a name that says nothing, and the
# second is named `.osv` and holds nothing at all.
#
# What the app does with them is an alert: the stock dialog libcosmic draws
# over the middle of the window in a modal popover. No capture can read the
# words in it, so what is checked is that something is drawn over the view,
# and that Escape takes it away and leaves the window byte for byte what it
# was. A modal layer that only half goes away fails the second half, which is
# what the pair is here to catch.

# Set by the check below and read by its predicate: the window before any of
# this, which is what dismissing the alert has to come back to.
prior=

alert_gone() {
	local shot
	shot=$(grab "$1")
	same_picture "$prior" "$shot"
}

foreign() {
	printf '\n-- foreign format checks (synthetic)\n'
	local gopro=$session/no-name-on-it.mp4 dji=$session/named-only.osv
	printf '\x00\x00\x00\x14ftypmp41\x00\x00\x00\x00mp41' >"$gopro"
	printf '\x00\x00\x00\x33moov\x00\x00\x00\x2budta' >>"$gopro"
	printf '\x00\x00\x00\x17FIRMH19.03.02.00.75\x00\x00\x00\x0cGPMF\x00\x00\x00\x00' >>"$gopro"
	: >"$dji"

	# The window with nothing open and nothing said, taken in a session of its
	# own so that both sides of the comparison are the same window at the same
	# size with the same state directory behind them.
	boot welcome-plain
	if ! await_paint welcome-plain; then
		alive || lost "the refusal puts an alert over the window"
		fail "the refusal puts an alert over the window" \
			"the welcome view is black" "log: $log"
		teardown
		return
	fi
	# A capture taken the moment the output stopped being black can be a
	# window that is still filling in, so every capture compared here is taken
	# a beat after paint instead.
	sleep "$SETTLE"
	prior=$(grab welcome-settled)
	quit >/dev/null 2>&1 || teardown

	boot gopro "$gopro"
	if await 'not shown: a GoPro capture' "$READY"; then
		pass "a GoPro file is refused by name"
	else
		fail "a GoPro file is refused by name" \
			"$(grep 'not shown' "$log" || echo 'nothing was refused')" "log: $log"
	fi

	if await_paint gopro; then
		pass "the refusal leaves a window up"
	else
		alive || lost "the refusal leaves a window up"
		fail "the refusal leaves a window up" \
			"capture is black: $session/gopro.ppm" "log: $log"
		teardown
		return
	fi

	sleep "$SETTLE"
	local alerted
	alerted=$(grab gopro-alert)
	if same_picture "$prior" "$alerted"; then
		fail "the refusal puts an alert over the window" \
			"the window is what it is with nothing open and nothing said" \
			"$prior" "$alerted"
	else
		pass "the refusal puts an alert over the window"
	fi

	if press_until alert_gone gopro-dismissed -k Escape; then
		pass "escape takes the alert away and leaves the window as it was"
	else
		alive || lost "escape takes the alert away and leaves the window as it was"
		fail "escape takes the alert away and leaves the window as it was" \
			"the window did not come back to what it was" \
			"$prior" "$session/gopro-dismissed.ppm"
	fi
	quit >/dev/null 2>&1 || teardown

	# The other half of the rule: the bytes said nothing, so the name is what
	# is left, and an `.osv` is DJI's.
	boot dji "$dji"
	if await 'not shown: a DJI capture' "$READY"; then
		pass "an .osv with nothing in it is still named"
	else
		fail "an .osv with nothing in it is still named" \
			"$(grep 'not shown' "$log" || echo 'nothing was refused')" "log: $log"
	fi
	exits_clean
}

# ------------------------------------------------------------------- run

if [ -n "$media" ]; then
	with_media
	pooled_calibration
else
	welcome
fi
dud
foreign

printf '\n%s checks, %s failed\n' "$checks" "$failures"
printf 'captures and logs: %s\n' "$session"
[ "$failures" = 0 ] || exit 1
