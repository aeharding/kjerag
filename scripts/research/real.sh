#!/usr/bin/env bash
# Branch-only research instrument (research/parity-proof, 2026-08-09): THE REAL
# RUN. docs/research/parity-protocol.md, executed against the two exports the
# owner made with Stitching Optimization OFF, Direction Lock OFF, Chromatic
# Calibration ON, out of VID_20260501_183417_00_002.insv.
#
# Nothing here chooses a threshold. Every gate is the protocol's and every
# number below that is not a gate -- the instants, the scale window, the
# working widths -- is written down here BEFORE the stage that uses it runs.
#
#   INSTANTS. A regular 35 s grid over the export's own 176.6 s span, declared
#   before the first registration: 20, 55, 90, 125, 160. "The frames that
#   worked" is not a sample and the protocol's section 7 already says single
#   frames fail on content -- what it asks is that they fail loudly, and each
#   instant prints its own verdict.
#
#   LABELS. The owner wrote down two sets and did NOT write down which file is
#   which. That is not a nuisance, it is the strongest control this run has:
#   stage `assign` puts each label on each file, and a label that does not
#   belong to a file has to fail to register on it. The tilt is 20.5 degrees
#   apart between the two labels and the told sweep only walks tilt over four,
#   so a wrong pairing cannot quietly succeed.
#
#   THE LOCK. Direction Lock is OFF this time, which changes the GEOMETRY of
#   the told sweep and not a setting in it: the view is fixed in the camera
#   body rather than in the world, so the aim carries no orientation track and
#   is the same three numbers at every instant. Stage `lock` runs both models
#   on the same frame and lets the score say which the export is, because a
#   checkbox is a label like every other label here.
#
# usage: scripts/research/real.sh <stage> <raw.insv> <exports-dir> <out-dir>
set -u

STAGE="${1:?usage: real.sh <lock|assign|register|plants|solve> <raw.insv> <exports-dir> <out-dir>}"
RAW="${2:?usage: real.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
EXPORTS="${3:?usage: real.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
OUT="${4:?usage: real.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
SEAM="./target/release/seam"
E8="$EXPORTS/Untitled38(8).mp4"
E9="$EXPORTS/Untitled38(9).mp4"
# scripts/research/pair.py, measured 2026-08-09 on both new exports: the coarse
# trim stands +0.1496 above every rival more than one export-length away, and
# the fine lag is -0.02298 s on all eight windows of both files with sd
# 0.00000 s -- the same five decimals the July-14 pair measured.
LAG=-0.02298
INSTANTS=20,55,90,125,160
ONE=90
# Studio's own boxes, as the owner wrote them. Both exports: FOV 60,
# Distortion 0, roll 0.
BUILDING="-204.2,-31.4,0"   # aimed at his building area
HORIZON="-111.9,-10.9,0"    # the horizon crossing
mkdir -p "$OUT"

# The scale window is ABSOLUTE and not a factor on their label: 12 degrees to
# whatever the projection itself can draw. Their told 60 sits in the middle of
# it and R4 is what says the answer was not found against an edge.
COMMON="lag=$LAG sweepw=16 coarse=48 keepn=16 pool=300 mid=192 size=512 \
  ratio=1.15 fovlo=12 fov=60 panini=0 dset=0,-0.7,-0.5"

run() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 21600 "$SEAM" "$RAW" mode=register $COMMON "$@" 2>&1 \
    | tee "$OUT/$tag.txt" | tail -2
  echo
}

case "$STAGE" in
# ---------------------------------------------------- which model the export is
# One frame, four runs: each file under each lock model, with the label the
# eye would give it. The lock model that registers is the one the export was
# made with, read off the picture.
lock)
  run lock-e8-body  against="$E8" instants=$ONE told=$BUILDING lock=body &
  run lock-e8-world against="$E8" instants=$ONE told=$BUILDING lock=world &
  run lock-e9-body  against="$E9" instants=$ONE told=$HORIZON  lock=body &
  run lock-e9-world against="$E9" instants=$ONE told=$HORIZON  lock=world &
  wait
  ;;
# ------------------------------------------------- which file is which, proved
# The SWAP. Each file given the other file's label, under the model stage
# `lock` chose. A label that does not belong to a file must not register on it.
assign)
  LOCK="${LOCK:-body}"
  run assign-e8-horizon  against="$E8" instants=$ONE told=$HORIZON  lock=$LOCK &
  run assign-e9-building against="$E9" instants=$ONE told=$BUILDING lock=$LOCK &
  wait
  ;;
# ------------------------------------------------------- the registration table
register)
  LOCK="${LOCK:-body}"
  E_BUILDING="${E_BUILDING:-$E8}"
  E_HORIZON="${E_HORIZON:-$E9}"
  run reg-building against="$E_BUILDING" instants=$INSTANTS told=$BUILDING \
    lock=$LOCK world=0 out="$OUT/reg-building" &
  run reg-horizon against="$E_HORIZON" instants=$INSTANTS told=$HORIZON \
    lock=$LOCK world=0 out="$OUT/reg-horizon" &
  wait
  ;;
*)
  echo "no stage called $STAGE" >&2
  exit 2
  ;;
esac

echo "=================================================== summary"
grep -H -E "^peak:|^score:|^world:|^body aim|^R[0-9]|^REGISTERED|^NOT REGISTERED|^their " \
  "$OUT"/*.txt 2>/dev/null
