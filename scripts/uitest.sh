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

bin=${KYERAG_BIN:-$root/target/release/kyerag}
if [ ! -x "$bin" ]; then
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

	saves_a_still
	flips_the_horizon
	survives_fullscreen

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
