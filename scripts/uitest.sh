#!/usr/bin/env bash
#
# Headless UI checks: drive the real app in a throwaway compositor and look
# at what came out.
#
#   scripts/uitest.sh [file.insv]      # or set KYERAG_TEST_MEDIA
#
# `cage` runs one client on a wlroots headless backend, which is a whole
# Wayland session with no monitor and no connection to the desktop the
# developer is looking at. `wtype` presses keys into it over the virtual
# keyboard protocol and `grim` copies the output out. The app is the release
# binary, unchanged: nothing here is a test hook.
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
#
# Local only, and never in CI: see "UI verification" in AGENTS.md.
#
# Exit: 0 all checks passed, 1 a check failed, 2 the harness could not run,
# 3 the session died under the harness (see PRESSES below).

set -uo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
session=$root/scratch/uitest
media=${1:-${KYERAG_TEST_MEDIA:-}}

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

# The build is part of the run, not a fallback for a missing binary. Cargo
# is a no-op on a fresh one, and the version that only built when the file
# was absent drove whatever happened to be there instead: on 2026-07-31 a
# binary built before a `git revert` failed the ball check for an hour while
# the source it was meant to be checking passed on every run.
#
# KYERAG_BIN is the way to point the harness at a binary on purpose, which is
# how that was measured, so it is taken as given and never rebuilt.
if [ -n "${KYERAG_BIN:-}" ]; then
	bin=$KYERAG_BIN
	[ -x "$bin" ] || die "no binary at $bin (KYERAG_BIN)"
	printf 'binary %s (KYERAG_BIN: not rebuilt)\n' "$bin"
else
	bin=$root/target/release/kyerag
	printf 'building %s\n' "$bin"
	(cd "$root" && cargo build --release) || die "the app did not build"
	[ -x "$bin" ] || die "no binary at $bin"
fi

[ -z "$media" ] || [ -f "$media" ] || die "no file at $media"

rm -rf "$session"
mkdir -p "$session/shots"
printf 'session %s\n' "$session"
[ -n "$media" ] || printf 'no test media: pass a file or set KYERAG_TEST_MEDIA for the playback checks\n'

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
	runtime=$(mktemp -d "${TMPDIR:-/tmp}/kyerag-uitest.XXXXXXXX")
	chmod 700 "$runtime"

	env \
		XDG_RUNTIME_DIR="$runtime" \
		XDG_CONFIG_HOME="$session/config" \
		XDG_STATE_HOME="$session/state" \
		XDG_DATA_HOME="$session/data" \
		XDG_CACHE_HOME="$session/cache" \
		XDG_SCREENSHOTS_DIR="$session/shots" \
		WLR_BACKENDS=headless \
		WLR_LIBINPUT_NO_DEVICES=1 \
		cage -- "$bin" ${file:+"$file"} >"$log" 2>&1 &
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
	flips_the_horizon
	survives_fullscreen
	fullscreen_holds_the_view

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
# the window whose colour the app decides rather than the footage:
# OUTSIDE_GRAY, flat and neutral, where a frame of video at the same place
# is neither. So the check reads one patch a fourteenth of the way in from
# the left, level with the middle, which is outside the ball at the far end
# of the zoom and inside the picture at the default view.
#
# Presses one at a time and looks after each, because the zoom is a clamp
# rather than a state: pressing past the end is free, and a dropped key
# costs a press rather than the check.
# Measured on this harness's 1280x672 widget: the room reaches the patch on
# the fifth press (90, 129, 185, 265, 380, 544 degrees, and the patch is off
# the ball past about 403). wtype drops about one key in twenty, so the rest
# is headroom, and the loop breaks on success so headroom costs nothing.
BALL_PRESSES=20
ROOM_SPREAD=4
ROOM_DARK=60

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

# Whether a patch is the flat neutral grey the pass paints where the frame
# has no sphere in it, rather than a piece of picture: dark, and the three
# channels within a code or two of each other. Measured on this harness, the
# room reads 25 25 25, which is OUTSIDE_GRAY through the surface's own sRGB
# round trip, and the same patch of the default view read 185 224 241 on the
# owner's footage, which is sky.
is_room() {
	local rgb=($1) hi lo
	[ "${#rgb[@]}" = 3 ] || return 1
	hi=$(printf '%s\n' "${rgb[@]}" | sort -n | tail -1)
	lo=$(printf '%s\n' "${rgb[@]}" | sort -n | head -1)
	[ "$hi" -le "$ROOM_DARK" ] && [ $((hi - lo)) -le "$ROOM_SPREAD" ]
}

# The mark above "No video open" is the app icon, which the binary carries as
# bytes (`crates/app/src/app.rs`, `APP_ICON`) because the icon theme has
# nothing under this app's ID yet (issue #75). Colour in that patch is what
# separates the drawing from the symbolic icon it replaced, and from an SVG
# that failed to load, which draws nothing at all.
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

# `h` flips the horizon lock, which is on by default, so the session's config
# ends up saying false. Reading the config rather than the picture is what
# separates a key that landed from a key that was swallowed: the two views
# differ by a rotation that one still frame need not show.
flips_the_horizon() {
	local locked=$session/config/cosmic/app.kyerag.Kyerag/v1/horizon_lock
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
# The instrument is the room around the ball (`is_room`), which is the app's
# own flat grey rather than footage, so it reads the same however the window
# is shaped underneath and whether the picture is paused or playing.
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

# ------------------------------------------------------------------- run

if [ -n "$media" ]; then
	with_media
else
	welcome
fi
dud

printf '\n%s checks, %s failed\n' "$checks" "$failures"
printf 'captures and logs: %s\n' "$session"
[ "$failures" = 0 ] || exit 1
