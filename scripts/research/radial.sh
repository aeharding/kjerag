#!/usr/bin/env bash
# Branch-only research instrument (research/parity-proof, 2026-08-09): THE
# RADIAL STAGE. docs/research/parity-protocol.md section 11, executed against
# the same two exports section 10 reported on.
#
# Nothing here chooses a threshold and nothing here chooses a number after
# seeing an answer. Section 11 was written and committed BEFORE the radial
# model was fitted to anything (0738ec9), and every quantity below that is not
# one of its gates -- the aims, the instants, the working width, the search --
# is section 10's own and is reused unchanged so that the two stages are
# comparable line for line.
#
#   THE AIMS. Section 10's registration table, at the two instants it reported
#   at per export. They are NOT re-registered here: re-fitting an aim after
#   seeing a residual is choosing an aim to suit the answer.
#
#   THE MODEL. Five orthonormal radial modes per lens on top of section 10's
#   nine numbers, orthonormal over a fixed 40-to-95-degree window of field
#   angle. The camera's numbers -- lens 1's five and the two radial deltas --
#   are SHARED across every geometry in a run and each geometry keeps its own
#   four view numbers. That split is what makes B2 a test.
#
#   THE REDUCED ARM. Every stage is run twice: once with all five modes a lens
#   (`full`, which is what section 11.1 pre-registers) and once with only the
#   lowest three (`three`). The second is not a relaxation of the bar -- it is
#   the same bar applied to a smaller model inside the same family, run so that
#   a failure of the five can be told apart from a failure of the physics.
#
# usage: scripts/research/radial.sh <stage> <raw.insv> <exports-dir> <out-dir>
set -u

STAGE="${1:?usage: radial.sh <null|plant|fit|cross|deliver> <raw.insv> <exports-dir> <out-dir>}"
RAW="${2:?usage: radial.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
EXPORTS="${3:?usage: radial.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
OUT="${4:?usage: radial.sh <stage> <raw.insv> <exports-dir> <out-dir>}"
SEAM="./target/release/seam"
E8="$EXPORTS/Untitled38(8).mp4"
E9="$EXPORTS/Untitled38(9).mp4"
LAG=-0.02298
mkdir -p "$OUT"

# Section 10's own aims, at the instants it reported at.
B160="b160@$E8@160@-110.301,-65.215,-77.310@60.373"
B055="b055@$E8@55@-100.313,-47.675,-71.441@60.378"
H160="h160@$E9@160@73.814,-29.441,57.871@59.798"
H055="h055@$E9@55@93.059,-46.304,50.976@59.626"
# The same four geometries with no export behind them: the plant and the null
# draw their own targets, so the arm carries `plant` where a path would go.
P160="b160@plant@160@-110.301,-65.215,-77.310@60.373"
P055="b055@plant@55@-100.313,-47.675,-71.441@60.378"
Q160="h160@plant@160@73.814,-29.441,57.871@59.798"
Q055="h055@plant@55@93.059,-46.304,50.976@59.626"

# Section 10's own working picture and search, unchanged.
FINE="size=3840 patch=15 search=96 sites=40 rounds=8 panini=0 aspect=1.7777778"
# The pre-registered plant of section 11.2 B3, beside section 4's own cx and
# pitch so the two stages' plants are the same plant plus a radial law.
PLANT="plant=cx:8,pitch:0.2 plantradial=1.0e-3,-6.0e-4,3.0e-4,0,0:-8.0e-4,4.0e-4,0,0,0"
NULL="plant=roll:0 plantradial=0,0,0,0,0:0,0,0,0,0"
# What "the lowest three modes a lens" means as an argument.
THREE="free=view,lens1,l0rad1,l0rad2,l0rad3,l1rad1,l1rad2,l1rad3"
FULL="free=all"

run() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 21600 "$SEAM" "$RAW" mode=solve lag=$LAG $FINE "$@" 2>&1 \
    | tee "$OUT/$tag.txt" | grep -E "^residual|^REFUSED|^PLANT" | head -6
  echo
}

case "$STAGE" in
# ------------------------------------------------------------------ B4, the null
null)
  run null-full  $FULL  $NULL arm=$P160 arm=$P055 arm=$Q160 arm=$Q055 &
  run null-three $THREE $NULL arm=$P160 arm=$P055 arm=$Q160 arm=$Q055 &
  wait
  ;;
# ----------------------------------------------------------------- B3, the plant
# Started off truth by the same amounts the fine-scale matrix of section 10
# used, which is what the 96 px search covers.
plant)
  OFF="viewoff=0.30,-0.22,0.10 fovoff=0.75"
  # shellcheck disable=SC2086
  run plant-full  $FULL  $PLANT $OFF arm=$P160 arm=$P055 arm=$Q160 arm=$Q055 &
  # shellcheck disable=SC2086
  run plant-three $THREE $PLANT $OFF arm=$P160 arm=$P055 arm=$Q160 arm=$Q055 &
  wait
  ;;
# ---------------------------------------------------------------- B1, the answer
fit)
  run fit4-full  $FULL  arm=$B160 arm=$B055 arm=$H160 arm=$H055 &
  run fit4-three $THREE arm=$B160 arm=$B055 arm=$H160 arm=$H055 &
  wait
  ;;
# ------------------------------------------------- B2, the cross-validation
# Fit the camera on one aim's two geometries; hold it; let the other aim move
# nothing but its own four view numbers. Both ways round.
cross)
  run fitB-full  $FULL  arm=$B160 arm=$B055 &
  run fitH-full  $FULL  arm=$H160 arm=$H055 &
  run fitB-three $THREE arm=$B160 arm=$B055 &
  run fitH-three $THREE arm=$H160 arm=$H055 &
  wait
  for arm in full three; do
    B=$(scripts/research/knobs.py "$OUT/fitB-$arm.txt")
    H=$(scripts/research/knobs.py "$OUT/fitH-$arm.txt")
    # shellcheck disable=SC2086
    run "predH-$arm" free=view $B arm=$H160 arm=$H055 &
    # shellcheck disable=SC2086
    run "predB-$arm" free=view $H arm=$B160 arm=$B055 &
    wait
    # The diagnostic beside the gate: the same predicted geometries with lens
    # 1's five re-freed and only the radial law held. Not the gate.
    # shellcheck disable=SC2086
    run "predH-lens1-$arm" free=view,lens1 $B arm=$H160 arm=$H055 &
    # shellcheck disable=SC2086
    run "predB-lens1-$arm" free=view,lens1 $H arm=$B160 arm=$B055 &
    wait
  done
  ;;
# --------------------------------------------------------- what the owner is shown
deliver)
  KNOBS=$(scripts/research/knobs.py "$OUT/fit4-full.txt")
  # shellcheck disable=SC2086
  run deliver-full free=view $KNOBS amp=8 out="$OUT/deliver-radial" \
    arm=$B160 arm=$H160
  ;;
*)
  echo "no stage called $STAGE" >&2
  exit 2
  ;;
esac

echo "=================================================== summary"
grep -H -E "^residual|^PLANT|^REFUSED|^closest reading" "$OUT"/*.txt 2>/dev/null
