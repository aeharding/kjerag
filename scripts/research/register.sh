#!/usr/bin/env bash
# Branch-only research instrument (research/parity-proof, 2026-08-09): the
# aim-and-scale search that docs/research/parity-protocol.md section 8 named as
# the blocker, run on the owner's two real Studio exports with the controls
# that say whether to believe it. Not on any shipped path and not in a gate.
#
# What it asks, in order:
#
#   1. does `mode=register` find a PEAK on a real Studio frame -- not the flat
#      0.7366 the protocol recorded, and not a field of view jammed against the
#      top of its own search;
#   2. does it find the SAME view at several instants once the file's own IMU
#      is taken out. Their reframe is DIRECTION LOCKED, so their view is fixed
#      in the world and ours is solved in the camera body's frame: the body aim
#      is different at every instant by exactly the flight, and only the world
#      aim is comparable;
#   3. does it find it with the SCALE labels wrong -- their field of view and
#      distortion numbers reach nothing, and the answer must not move;
#   4. does the capture gate still REFUSE when the answer is outside the window
#      it was given. A search that always answers is a search that will answer
#      about the owner's new export too.
#
# usage: scripts/research/register.sh <raw.insv> <exports-dir> <out-dir>
set -u

RAW="${1:?usage: register.sh <raw.insv> <exports-dir> <out-dir>}"
EXPORTS="${2:?usage: register.sh <raw.insv> <exports-dir> <out-dir>}"
OUT="${3:?usage: register.sh <raw.insv> <exports-dir> <out-dir>}"
SEAM="./target/release/seam"
CREEK="$EXPORTS/20-fov-creek.mp4"
PLANET="$EXPORTS/tiny-planet.mp4"
# scripts/research/pair.py, measured 2026-08-08 on both exports: identical to
# five decimals, sd 0.00000 s over eight disjoint windows.
LAG=-0.02298
# Instants on a regular 300 s grid, declared before the run: "the frames that
# worked" is not a sample. The protocol's section 7 already says single frames
# fail on content; what it asks is that they fail loudly, and each one below
# prints its own verdict.
INSTANTS=60,360,660,960
FEW=360,660
mkdir -p "$OUT"

run() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 14400 "$SEAM" "$RAW" mode=register lag=$LAG sweepw=16 coarse=48 \
    keepn=16 pool=300 "$@" 2>&1 | tee "$OUT/$tag.txt" | tail -3
  echo
}

# ---------------------------------------------------- the two exports, cold
# `told=` is their pan, tilt and roll and NOTHING else: their field of view and
# their distortion are swept over the same label-free window either way. It is
# there because a four dimensional sweep of SO(3) by picture correlation is
# information-limited on a narrow view and measurably so -- the correlation
# reaches about one pixel of the sweep picture, so a sweep W across an F degree
# view captures within F/W, and on the 20 degree creek the answer scores 0.92
# where it stands and under 0.65 at the nearest point any affordable grid
# visited. Their direction lock is what removes three of those dimensions: the
# view is fixed in the WORLD, the file's own orientation track says where the
# body was, and the one angle left between their frame and ours is the heading
# datum the integration started from. That is 360 scores, not two million.
run creek  against="$CREEK"  instants=$INSTANTS fov=20  panini=0 \
  told=-53.7,3.5,0 dset=0,-0.7,-0.5 ratio=1.15 mid=128 size=320 out="$OUT/creek"
run planet against="$PLANET" instants=$INSTANTS fov=150 panini=0.9 \
  told=-53.7,-90,0 dset=0.9,0,-0.7,-0.6,-0.5,-0.42 ratio=1.3 mid=192 size=640 \
  out="$OUT/planet"

# ------------------------------------------- the tiny planet with no labels
# The wide export is searched over the whole sphere and the whole scale window
# with no `told=` at all, because a 290 degree view is exactly where the sweep
# is NOT information-limited: at that scale its own grid step and its own
# capture radius are the same size. This is the run that says the projection
# was found rather than assumed -- their tiny planet is not Panini at any `d`,
# it is stereographic, and nothing here was told so.
run planet-labelfree against="$PLANET" instants=$FEW fov=150 panini=0.9 \
  dset=0.9,0,-0.7,-0.6,-0.5,-0.42 ratio=1.3 mid=192 size=640

# ------------------------------------------------------------- the controls
# MISLABELLED. The same creek frames told a field of view and a distortion that
# are both nonsense, over the full projection set. Neither number reaches the
# search, so the answer has to come back unchanged; if it moves, the search was
# reading a label somewhere it should not have been.
run creek-mislabelled against="$CREEK" instants=$FEW fov=999 panini=1.35 \
  told=-53.7,3.5,0 dset=0,0.45,0.9,1.35,-0.85,-0.7,-0.6,-0.5 ratio=1.3 \
  mid=128 size=320

# SATURATED. The same frames with the scale window moved off the answer. The
# capture gate has to FAIL: a search that reports a number from the edge of its
# own window has reported the edge of its own window. This is the G7-class
# control, and it exists so the window can be trusted rather than widened.
run creek-window-high against="$CREEK" instants=$FEW fov=20 panini=0 \
  told=-53.7,3.5,0 dset=0 fovlo=40 fovhi=90 ratio=1.15 mid=128 size=320
run creek-window-low against="$CREEK" instants=$FEW fov=20 panini=0 \
  told=-53.7,3.5,0 dset=0 fovlo=6 fovhi=13 ratio=1.15 mid=128 size=320

echo "=================================================== summary"
grep -H -E "^R[0-9] |^ *R[0-9] |^their |^REGISTERED|^NOT REGISTERED" "$OUT"/*.txt
