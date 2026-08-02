#!/usr/bin/env bash
#
# How often a window with a file in it draws no picture, counted per minute.
#
#   scripts/blankwatch.sh [file.insv]      # or set KJERAG_TEST_MEDIA
#
# The owner sees the picture disappear "every now and then" under mild load,
# while playing and around the controls coming and going. `scripts/uitest.sh`
# has one check for the paused case (issue #102) and nothing that watches a
# playing window, because a playing blank is over in a quarter of a second:
# the 250 ms controls poll rebuilds the widget tree four times a second, and a
# rebuild is what heals it. This instrument is the one that can see that.
#
# What it does, in one isolated cage session per run, with the same load the
# issue #102 work measured under:
#
#   paused   pause, then twelve captures over five seconds. This is the
#            POSITIVE CONTROL: the defect of issue #102 is known to blank a
#            paused window under load, so a detector that cannot fail here
#            cannot be believed anywhere else.
#   cycle    play, and take the controls away and bring them back, over and
#            over, photographing both transitions at about 50 frames a second.
#   steady   play, hands off, the same number of captures.
#   open     close the file and open it again from a window that is already
#            up, and photograph the gap before the first frame. This is the
#            hold-last-frame slot (issue #124) at the one moment it holds
#            nothing.
#
# Every capture is classified from its pixels alone, so one script reads any
# build:
#
#   blank    the video pane is one flat colour. Bare theme background reads a
#            spread of 0 over the pane; a frame of footage reads over 190.
#   pattern  the pane is the animated test pattern the shader drew before
#            issue #125, told apart by its mirror symmetry (same green and
#            same blue at mirrored places, different reds). It matters here
#            because a build from before that merge draws it where a build
#            after it draws the backdrop, which is flat, which reads blank.
#   picture  anything else.
#
# A blank is split again by what the bottom of the window holds. The video
# pane and the control row are two halves of one column of widgets, so a blank
# with a flat bottom band is a window that lost the whole column (issue #102's
# tree-diff), and a blank with something in the bottom band is a window that
# drew everything except a frame (the pass with no frame to show).
#
# KJERAG_BIN points the run at a binary that is already built, which is how
# two commits are compared: build each into its own target directory
# (AGENTS.md) and run this script at each in turn.
#
# Captures live in /dev/shm for as long as it takes to read them and are
# deleted per burst, because they are frames of somebody's flight. Up to two
# exemplars of each class survive, under scratch/, which is gitignored.
#
# Needs `cage wtype grim ffmpeg`, and `wl-copy` for the open phase.

set -uo pipefail

root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
media=${1:-${KJERAG_TEST_MEDIA:-}}
label=${BLANKWATCH_LABEL:-run}
session=$root/scratch/blankwatch/$label
HOME_DIR=$session/home
STATE_HOME=$HOME_DIR/.local/state
QUIET_SINK=kjerag_quiet

# The load the defect was measured under (issue #102, PR #111): busy loops,
# one more than there are cores plus a half. Zero runs the same protocol on an
# idle box.
LOAD=${BLANKWATCH_LOAD:-18}
# How many times the controls are taken away and brought back, and how many
# captures each burst holds. A capture costs about 60 ms here, so a burst of 90
# watches for five or six seconds without a gap in it: long enough to cover
# one whole cycle of the controls, and fine enough that a blank lasting one
# 250 ms controls poll lands in three or four captures in a row.
CYCLES=${BLANKWATCH_CYCLES:-6}
BURST=${BLANKWATCH_BURST:-90}
# How many hands-off bursts, and how many times the file is opened again.
STEADY=${BLANKWATCH_STEADY:-6}
OPENS=${BLANKWATCH_OPENS:-3}

READY=45
SETTLE=1
PRESSES=3

# The two bands of the window that are chrome, measured under cage at
# 1280x720, the same numbers scripts/uitest.sh reads.
HEADER_BAND=64
CONTROL_BAND=96

# How far apart the codes in a band may be and still be one flat colour.
# Measured: bare theme background is a spread of 0 over the pane, a frame of
# footage 231 to 243, the ball view with its dark room 194.
FLAT=3

# What says the test pattern rather than a picture: mirrored patches read the
# same green and blue and different reds (issue #125's check, on the 32x18
# grid this one averages the pane onto).
PATTERN_RED=20
PATTERN_MATCH=1

die() {
	printf 'blankwatch: %s\n' "$1" >&2
	exit 2
}

for tool in cage wtype grim ffmpeg; do
	command -v "$tool" >/dev/null || die "$tool is not installed"
done
[ -n "$media" ] || die "no media: pass a file or set KJERAG_TEST_MEDIA"
[ -f "$media" ] || die "no file at $media"

if [ -n "${KJERAG_BIN:-}" ]; then
	bin=$KJERAG_BIN
	[ -x "$bin" ] || die "no binary at $bin (KJERAG_BIN)"
else
	bin=$root/target/release/kjerag
	(cd "$root" && cargo build --release) || die "the app did not build"
fi
poker=${KJERAG_POINTER:-$root/target/release/pointer}
[ -x "$poker" ] || die "no pointer at $poker (KJERAG_POINTER)"

# The sound the app plays goes where every other agent run's does (AGENTS.md,
# sound etiquette). Without a sink the app says so and plays silently.
sound=yes
if command -v pactl >/dev/null && [ -S "/run/user/$(id -u)/pipewire-0" ]; then
	pactl list short sinks | grep -q "[[:space:]]$QUIET_SINK[[:space:]]" ||
		pactl load-module module-null-sink "sink_name=$QUIET_SINK" \
			"sink_properties=device.description=$QUIET_SINK" >/dev/null
else
	sound=no
fi

rm -rf "$session"
mkdir -p "$session/shots" "$session/config/cosmic" "$STATE_HOME/cosmic" "$session/keep"
log=$session/app.log
# The app's own stdout, and nothing else's: the instruments write their noise
# somewhere the `said` grep above cannot read it.
tools=$session/tools.log
table=$session/frames.tsv
printf 'phase\tburst\tframe\tstamp\tpane_lo\tpane_hi\tpane_mean\tpat\tctl_spread\tclass\n' >"$table"

shm=$(mktemp -d /dev/shm/blankwatch.XXXXXXXX)

# ------------------------------------------------------------------ load

load_pids=()

spin_up() {
	local i
	[ "$LOAD" -gt 0 ] || return 0
	for ((i = 0; i < LOAD; i++)); do
		sh -c 'while :; do :; done' &
		load_pids+=($!)
	done
}

spin_down() {
	local pid
	for pid in "${load_pids[@]:-}"; do
		[ -n "$pid" ] && kill -KILL "$pid" 2>/dev/null
	done
	load_pids=()
}

# --------------------------------------------------------------- session

sock=
cage_pid=
runtime=

boot() {
	runtime=$(mktemp -d "${TMPDIR:-/tmp}/blankwatch.XXXXXXXX")
	chmod 700 "$runtime"
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
		cage -- "$bin" "$media" >"$log" 2>&1 &
	cage_pid=$!

	local waited=0
	while [ ! -S "$runtime/wayland-0" ]; do
		sleep 0.2
		waited=$((waited + 1))
		[ "$waited" -lt 50 ] || die "no wayland socket after 10 s (see $log)"
	done
	sock=wayland-0
	export XDG_RUNTIME_DIR="$runtime" WAYLAND_DISPLAY="$sock"
}

alive() {
	kill -0 "$cage_pid" 2>/dev/null
}

teardown() {
	if alive; then
		kill -TERM "$cage_pid" 2>/dev/null
		sleep 0.5
		kill -KILL "$cage_pid" 2>/dev/null
	fi
	wait "$cage_pid" 2>/dev/null
	[ -z "$runtime" ] || rm -rf "$runtime"
	spin_down
	rm -rf "$shm"
}

trap teardown EXIT

key() {
	wtype "$@" 2>>"$tools"
	sleep "$SETTLE"
}

said() {
	grep -q -- "$1" "$log"
}

await() {
	local pattern=$1 limit=$2 waited=0
	while ! said "$pattern"; do
		alive || return 1
		sleep 0.5
		waited=$((waited + 1))
		[ "$waited" -le $((limit * 2)) ] || return 1
	done
}

# ---------------------------------------------------------------- bursts

# burst <phase> <n> <count> [gap]: rapid captures with the wall clock beside
# each one. A gap makes it a slow watch rather than a burst, which is what the
# paused control wants.
burst() {
	local phase=$1 n=$2 count=$3 gap=${4:-0} i dir
	dir=$shm/$phase-$n
	rm -rf "$dir"
	mkdir -p "$dir"
	: >"$dir/stamps"
	for ((i = 1; i <= count; i++)); do
		alive || break
		printf '%s\n' "$EPOCHREALTIME" >>"$dir/stamps"
		grim -t ppm "$(printf '%s/f%05d.ppm' "$dir" "$i")" 2>>"$tools"
		[ "$gap" = 0 ] || sleep "$gap"
	done
	read_burst "$phase" "$n" "$dir"
	rm -rf "$dir"
}

# The pane and the bottom band of every capture in one burst, in two runs of
# ffmpeg over the sequence rather than two per capture: the whole point of a
# burst is that it is cheap enough to take a lot of.
band_values() {
	local dir=$1 crop=$2 grid=$3
	ffmpeg -y -loglevel error -f image2 -start_number 1 -i "$dir/f%05d.ppm" \
		-vf "crop=$crop,scale=$grid:flags=area" -f rawvideo -pix_fmt rgb24 - 2>>"$tools" |
		od -An -tu1 -v | tr -s ' ' '\n' | grep -v '^$'
}

# One line per frame: lowest code, highest code, mean, and whether the
# mirrored patches say test pattern.
frame_stats() {
	awk -v cells="$1" -v w="$2" -v red="$PATTERN_RED" -v match_to="$PATTERN_MATCH" '
		function flush(   i, lo, hi, sum, lc, rc, li, ri, dr, dg, db, pat) {
			lo = 255; hi = 0; sum = 0
			for (i = 0; i < cells; i++) {
				if (v[i] < lo) lo = v[i]
				if (v[i] > hi) hi = v[i]
				sum += v[i]
			}
			pat = 0
			if (w > 8) {
				# Two patches the middle of the pane reflects onto each other,
				# on the row through the middle of it.
				lc = int(w * 0.15); rc = w - 1 - lc
				li = (int(h / 2) * w + lc) * 3
				ri = (int(h / 2) * w + rc) * 3
				dr = v[li] - v[ri]; dg = v[li + 1] - v[ri + 1]; db = v[li + 2] - v[ri + 2]
				if (dr < 0) dr = -dr
				if (dg < 0) dg = -dg
				if (db < 0) db = -db
				if (dr >= red && dg <= match_to && db <= match_to) pat = 1
			}
			printf "%d\t%d\t%.1f\t%d\n", lo, hi, sum / cells, pat
		}
		BEGIN { c = 0; h = cells / (w * 3) }
		{ v[c++] = $1; if (c == cells) { flush(); c = 0 } }
	'
}

# read_burst <phase> <n> <dir>: classify what the burst caught and append it
# to the run's table.
read_burst() {
	local phase=$1 n=$2 dir=$3 rows
	rows=$((HEADER_BAND + CONTROL_BAND))
	band_values "$dir" "iw:ih-$rows:0:$HEADER_BAND" "32:18" |
		frame_stats $((32 * 18 * 3)) 32 >"$dir/pane"
	band_values "$dir" "iw:$CONTROL_BAND:0:ih-$CONTROL_BAND" "32:4" |
		frame_stats $((32 * 4 * 3)) 32 >"$dir/ctl"

	paste "$dir/stamps" "$dir/pane" "$dir/ctl" |
		awk -v phase="$phase" -v n="$n" -v flat="$FLAT" -v OFS='\t' '
			NF < 9 { next }
			{
				# stamp lo hi mean pat ctl_lo ctl_hi ctl_mean ctl_pat
				spread = $3 - $2
				ctl = $7 - $6
				class = "picture"
				if ($5 == 1) class = "pattern"
				else if (spread <= flat) class = (ctl <= flat) ? "blank-whole" : "blank-pane"
				print phase, n, NR, $1, $2, $3, $4, $5, ctl, class
			}
		' >>"$table"

	# One frame of each class is kept for looking at, and only one: a burst of
	# real footage is a lot of personal video.
	local keep class
	while read -r class keep; do
		[ -f "$session/keep/$class.ppm" ] && continue
		[ -f "$(printf '%s/f%05d.ppm' "$dir" "$keep")" ] &&
			cp "$(printf '%s/f%05d.ppm' "$dir" "$keep")" "$session/keep/$class.ppm"
	done < <(awk -v phase="$phase" -v n="$n" '$1 == phase && $2 == n { print $10, $3 }' "$table" |
		sort -u -k1,1)
}

# ---------------------------------------------------------------- phases

# The pointer, moved somewhere new: this is what brings the controls and the
# header bar back, which is the tree change issue #102 is about. It runs in
# the background because the pointer holds its device open for half a second
# before it moves anything (crates/spike/src/bin/pointer.rs), and the burst
# has to be running by then.
wave() {
	local x=$1 y=$2
	"$poker" 1280 720 "$x" "$y" >>"$tools" 2>&1 &
}

paused_control() {
	printf '\n-- paused (the positive control)\n'
	key -k space
	burst paused 1 12 0.4
	key -k space
	sleep 1
}

cycled_play() {
	local n x
	printf '\n-- cycle (play, controls away and back)\n'
	for ((n = 1; n <= CYCLES; n++)); do
		alive || return
		x=$((300 + (n % 2) * 400))
		# One burst over the whole cycle: the pointer moves half a second in,
		# which brings the controls and the header bar back, and they hide
		# again two seconds after it stops.
		wave "$x" 300
		burst cycle "$n" "$BURST"
		sleep 0.5
	done
}

steady_play() {
	local n
	printf '\n-- steady (play, hands off)\n'
	sleep 2.5
	for ((n = 1; n <= STEADY; n++)); do
		alive || return
		burst steady "$n" "$BURST"
		sleep 1
	done
}

# The pane between an open and its first frame, which is where the held slot
# of issue #124 holds nothing. Ctrl+V is the only way to open a file from a
# window that is already up under cage: Ctrl+O wants a portal there is none of.
opened_again() {
	local n gotos seconds deep waited
	if ! command -v wl-copy >/dev/null; then
		printf '\n-- open: skipped, no wl-copy\n'
		return
	fi
	printf '\n-- open (the gap before the first frame)\n'
	seconds=$(sed -n 's/^media:.*, \([0-9.]*\) s$/\1/p' "$log" | head -1)
	deep=$(awk -v s="${seconds:-0}" 'BEGIN { printf "%.3f", s * 0.9 }')
	for ((n = 1; n <= OPENS; n++)); do
		alive || return
		key -M ctrl -k w -m ctrl
		wl-copy "$media time=$deep yaw=0.00 pitch=0.00 fov=90.00 lock=1" 2>>"$tools"
		gotos=$(grep -c '^goto:' "$log")
		wtype -M ctrl -k v -m ctrl 2>>"$tools"
		waited=0
		while [ "$(grep -c '^goto:' "$log")" -le "$gotos" ] && [ "$waited" -lt 500 ]; do
			alive || return
			sleep 0.01
			waited=$((waited + 1))
		done
		burst open "$n" 40
		sleep 1
	done
}

# ------------------------------------------------------------------- run

printf 'blankwatch %s\n' "$label"
printf 'binary  %s\n' "$bin"
printf 'built   %s\n' "$(date -r "$bin" '+%F %T')"
printf 'media   %s\n' "$media"
printf 'load    %s busy loops, box load %s\n' "$LOAD" "$(cut -d' ' -f1-3 /proc/loadavg)"
printf 'session %s\n' "$session"

spin_up
boot

if ! await '^media:' "$READY"; then
	printf 'blankwatch: no media line in %s s (see %s)\n' "$READY" "$log" >&2
	exit 2
fi
# The window has to be up before anything is pressed at it.
sleep 3

paused_control
cycled_play
steady_play
opened_again

printf '\nplay lines: %s\n' "$(grep -c '^play:' "$log")"
grep '^play:' "$log" | tail -2

# ---------------------------------------------------------------- report

awk -F'\t' -v OFS='  ' '
	NR == 1 { next }
	{
		phase = $1
		seen[phase]++
		total++
		class[phase "/" $10]++
		key = phase "/" $2
		if (!(key in first) || $4 + 0 < first[key]) first[key] = $4 + 0
		if ($4 + 0 > last[key]) last[key] = $4 + 0
		# An episode is a run of consecutive captures of the same not-picture
		# class inside one burst.
		if ($10 != "picture" && (prev[key] != $10 || prevframe[key] != $3 - 1)) {
			episodes[phase "/" $10]++
		}
		prev[key] = $10
		prevframe[key] = $3
	}
	END {
		# Watched time is the sum of the bursts, not the wall clock: the gaps
		# between bursts are not watched and nothing seen in them is counted.
		for (key in first) {
			split(key, part, "/")
			span[part[1]] += last[key] - first[key]
		}
		printf "\n%-12s %7s %8s %11s %10s %8s %9s %8s\n", \
			"phase", "frames", "seconds", "blank-whole", "blank-pane", "pattern", \
			"episodes", "per min"
		for (phase in seen) {
			eps = episodes[phase "/blank-whole"] + episodes[phase "/blank-pane"] + episodes[phase "/pattern"]
			printf "%-12s %7d %8.1f %11d %10d %8d %9d %8.1f\n", phase, seen[phase], \
				span[phase], class[phase "/blank-whole"], class[phase "/blank-pane"], \
				class[phase "/pattern"], eps, \
				(span[phase] > 0 ? eps * 60 / span[phase] : 0)
			allspan += span[phase]
			alleps += eps
			allblank += class[phase "/blank-whole"] + class[phase "/blank-pane"]
			allpat += class[phase "/pattern"]
		}
		printf "\ntotal %d frames over %.1f s watched: %d blank, %d pattern, %d episodes\n", \
			total, allspan, allblank, allpat, alleps
		if (allspan > 0) {
			printf "rate  %.1f episodes with no picture per minute watched, %.1f%% of captures with no picture\n", \
				alleps * 60 / allspan, (allblank + allpat) * 100 / total
		}
	}
' "$table"

printf '\ntable %s\n' "$table"
