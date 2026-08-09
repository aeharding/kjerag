#!/usr/bin/env bash
# Branch-only research instrument (research/parity-proof, 2026-08-08): the plant
# matrix `mode=solve` has to pass before it is pointed at a real export. Not on
# any shipped path and not wired into a gate.
#
# Every run here builds its OWN target out of our own renderer with a KNOWN
# perturbation on lens 1, so the answer is not a matter of opinion. Two things
# are being asked at once:
#
#   1. does the solver recover what was planted, and to what precision;
#   2. does it REFUSE when the view it is given cannot answer -- which is the
#      control, because a solver that always answers is a solver that will
#      answer about Studio too.
#
# usage: scripts/research/plants.sh <raw.insv> <out-dir>
set -u

RAW="${1:?usage: plants.sh <raw.insv> <out-dir>}"
OUT="${2:?usage: plants.sh <raw.insv> <out-dir>}"
SEAM="./target/release/seam"
FROM=36.303
SIZE=640
ROUNDS=8
mkdir -p "$OUT"

# The plant: a principal point moved most of a hundredth of the lens's width
# and a lens tipped a fifth of a degree. Both are the size of the disagreement
# the campaign is actually chasing (docs/research/offset-v6.md quotes residuals
# of about a degree), so a solver that recovers these can see the real thing.
PLANT="cx:8,pitch:0.2"
# Where the solve starts: two degrees of view error, so the run has to find the
# pose as well as the calibration. Studio's pan frame is not ours and this is
# the size of the gap that leaves.
VIEWOFF="1.2,-0.9,0.4"

run() {
  local tag="$1"; shift
  echo "############ $tag" | tee "$OUT/$tag.txt"
  # shellcheck disable=SC2086
  timeout 3000 "$SEAM" "$RAW" mode=solve from=$FROM size=$SIZE rounds=$ROUNDS \
    sites=40 search=30 "$@" 2>&1 | tee -a "$OUT/$tag.txt"
  echo | tee -a "$OUT/$tag.txt"
}

# ------------------------------------------------- the geometry we will ask for
# 60 degrees wide with the seam down the middle: the aim section 5 of
# docs/research/parity-protocol.md asks the owner to export, measured here
# rather than recommended on a hunch.
run null-seam-60   view=90,0,0 fov=60 panini=0 plant=roll:0 viewoff=$VIEWOFF fovoff=3.0
run plant-seam-60  view=90,0,0 fov=60 panini=0 plant=$PLANT viewoff=$VIEWOFF fovoff=3.0

# ---------------------------------------------------------------- the nulls
# An unperturbed self-render. Whatever this reads is the estimator's own noise
# floor and nothing below it means anything on any later run.
run null-planet-down  view=0,-90,0   fov=150 panini=0.9 plant=roll:0 viewoff=$VIEWOFF fovoff=3.0
run null-planet-tilt  view=90,-60,0  fov=150 panini=0.9 plant=roll:0 viewoff=$VIEWOFF fovoff=3.0
run null-creek-seam   view=90,3.5,0  fov=20  panini=0   plant=roll:0 viewoff=$VIEWOFF

# ---------------------------------------------------------------- the plants
run plant-planet-down view=0,-90,0   fov=150 panini=0.9 plant=$PLANT viewoff=$VIEWOFF fovoff=3.0
run plant-planet-tilt view=90,-60,0  fov=150 panini=0.9 plant=$PLANT viewoff=$VIEWOFF fovoff=3.0
run plant-creek-seam  view=90,3.5,0  fov=20  panini=0   plant=$PLANT viewoff=$VIEWOFF

# ------------------------------------------------------------- the refusals
# A narrow view aimed away from the seam carries one lens and cannot separate
# the view's own pose from lens 1's correction. It has to say so.
run refuse-creek-front view=0,3.5,0   fov=20 panini=0 plant=$PLANT viewoff=$VIEWOFF
run refuse-creek-back  view=180,3.5,0 fov=20 panini=0 plant=$PLANT viewoff=$VIEWOFF

echo "=================================================== summary"
grep -H -E "^lens1 |^view fov|^REFUSED|^residual|separates" "$OUT"/*.txt
