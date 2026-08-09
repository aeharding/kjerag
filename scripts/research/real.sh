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
  ratio=1.15 fovlo=12 fov=60 panini=0"
# Stage `lock` sweeps three projections across both families, because nothing
# here believes their Distortion box either. Stage `register` pins the told
# rectilinear one, and that is section 4b's own finding rather than trust:
# below about sixty degrees of view the projection is NOT identifiable -- every
# smooth radial map is linear over a small enough picture -- so a free `d`
# trades against the field of view without the score moving, which is exactly
# what stage `lock` caught it doing on the building export (d walked to
# compression 0.978 and the field of view to 60.479 for a told 60, where the
# horizon export with the same freedom stayed at panini 0 and 59.918). Section
# 5 item 4 is the reason the owner was asked for Distortion 0 in the first
# place: it is the one setting where nothing has to be identified.
SWEPT="dset=0,-0.7,-0.5"
TOLD_D="dset=0"

run() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 21600 "$SEAM" "$RAW" mode=register $COMMON "$@" 2>&1 \
    | tee "$OUT/$tag.txt" | tail -2
  echo
}

# ============================================================ the solve stages
#
# WHERE THESE AIMS COME FROM. Every `view=` and `fov=` below is what
# `mode=register` reported for that export at that instant, in the register
# stage above whose output is $OUT/reg-{building,horizon}.txt. Nothing here is
# a label and nothing here is a guess: the registration's own gates R1 to R7
# passed at each of these instants, and its ladder is what says the aim is a
# peak rather than a number.
#
#   building, 160 s: yaw -110.301 pitch -65.215 roll -77.310, fov 60.373, 0.7803
#   building,  55 s: yaw -100.313 pitch -47.675 roll -71.441, fov 60.378, 0.5857
#   horizon,  160 s: yaw  +73.814 pitch -29.441 roll +57.871, fov 59.798, 0.8262
#   horizon,   55 s: yaw  +93.059 pitch -46.304 roll +50.976, fov 59.626, 0.6069
#
# The two instants per export are the best-scoring one and a second one 105 s
# before it, so an answer is a repeat rather than a single frame.
B160="view=-110.301,-65.215,-77.310 fov=60.373 from=160"
B055="view=-100.313,-47.675,-71.441 fov=60.378 from=55"
H160="view=73.814,-29.441,57.871 fov=59.798 from=160"
H055="view=93.059,-46.304,50.976 fov=59.626 from=55"

# The picture the answer is measured on is THE EXPORT'S OWN, 3840 across, which
# is what section 3(a) means by "at the export's own pixel scale": at 60 degrees
# that is 64 pixels per degree, so 1.0 px is 0.0156 deg of world angle and the
# criterion is twice as tight here as the 1920-wide example section 3 works
# through. `search` follows from that and is not a taste (G7): the
# registration's ladder puts the aim inside a quarter degree, which is 16 px
# here, and the calibration disagreement being chased is about a degree, which
# is 64 -- so the first round has to reach past both.
FINE="size=3840 patch=15 search=96 sites=40 rounds=8 panini=0 aspect=1.7777778"

solve_at() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 21600 "$SEAM" "$RAW" mode=solve lag=$LAG $FINE "$@" 2>&1 \
    | tee "$OUT/$tag.txt" | grep -E "^residual|^REFUSED" | head -3
  echo
}

plant_at() {
  local tag="$1"; shift
  echo "############ $tag"
  # shellcheck disable=SC2086
  timeout 21600 "$SEAM" "$RAW" mode=solve lag=$LAG rounds=8 sites=40 panini=0 \
    aspect=1.7777778 "$@" 2>&1 | tee "$OUT/$tag.txt" \
    | grep -E "^residual|^PLANT|^REFUSED" | head -4
  echo
}

case "$STAGE" in
# ---------------------------------------------------- which model the export is
# One frame, four runs: each file under each lock model, with the label the
# eye would give it. The lock model that registers is the one the export was
# made with, read off the picture.
lock)
  run lock-e8-body  against="$E8" $SWEPT instants=$ONE told=$BUILDING lock=body &
  run lock-e8-world against="$E8" $SWEPT instants=$ONE told=$BUILDING lock=world &
  run lock-e9-body  against="$E9" $SWEPT instants=$ONE told=$HORIZON  lock=body &
  run lock-e9-world against="$E9" $SWEPT instants=$ONE told=$HORIZON  lock=world &
  wait
  ;;
# ------------------------------------------------- which file is which, proved
# The SWAP. Each file given the other file's label, under the model stage
# `lock` chose. A label that does not belong to a file must not register on it.
assign)
  LOCK="${LOCK:-world}"
  run assign-e8-horizon  against="$E8" $SWEPT instants=$ONE told=$HORIZON  lock=$LOCK &
  run assign-e9-building against="$E9" $SWEPT instants=$ONE told=$BUILDING lock=$LOCK &
  wait
  ;;
# ------------------------------------------------------- the registration table
register)
  LOCK="${LOCK:-world}"
  E_BUILDING="${E_BUILDING:-$E8}"
  E_HORIZON="${E_HORIZON:-$E9}"
  run reg-building against="$E_BUILDING" $TOLD_D instants=$INSTANTS told=$BUILDING \
    lock=$LOCK out="$OUT/reg-building" &
  run reg-horizon against="$E_HORIZON" $TOLD_D instants=$INSTANTS told=$HORIZON \
    lock=$LOCK out="$OUT/reg-horizon" &
  wait
  ;;
# ------------------------------------------------------------- the plant gates
# G1, G2 and G3 of the protocol, EXACTLY as section 4 pre-registers them --
# plant cx +8 px and pitch +0.2 deg on lens 1, start the view 1.2, -0.9, 0.4 deg
# and the field of view 3 deg off truth, 640 px, search 30, 8 rounds -- but at
# THE VIEW GEOMETRY AND THE FRAMES THIS RUN REPORTS ITS ANSWER AT, which is
# what section 4 asks and what makes them evidence rather than decoration.
#
# The width is 640 and not 3840 for a reason G7 states itself: a start 1.2 deg
# off is 77 px at the export's own scale, so the pre-registered plant needs a
# search past that, and the correlation is brute force over (2s+1)^2 positions.
# The pre-registered plant is therefore run at the width it was pre-registered
# and measured at, and a SECOND matrix is run at the full 3840 with a start
# error the fine search covers, so the fine scale has a measured floor of its
# own. The gates are read on the first; the second is extra and is labelled so.
plants)
  OFF="viewoff=1.2,-0.9,0.4 fovoff=3.0"
  PLANT="plant=cx:8,pitch:0.2"
  for aim in b160 b055 h160 h055; do
    case $aim in
      b160) V="$B160";; b055) V="$B055";; h160) V="$H160";; h055) V="$H055";;
    esac
    # shellcheck disable=SC2086
    plant_at "g1-null-$aim"  $V size=640 search=30 plant=roll:0 $OFF &
    # shellcheck disable=SC2086
    plant_at "g2-plant-$aim" $V size=640 search=30 $PLANT $OFF &
  done
  wait
  # G3 REFUSAL. The same solve at a view carrying one lens only. It has to say
  # so, in the build that produced the answer, rather than be assumed to.
  # 90 degrees off the seam in our frame is a hemisphere with one lens in it.
  plant_at g3-refuse-front view=0,0,0   fov=60 from=160 size=640 search=30 $PLANT $OFF &
  plant_at g3-refuse-back  view=180,0,0 fov=60 from=160 size=640 search=30 $PLANT $OFF &
  wait
  # The fine-scale floor: the same plant at the export's own 3840, started
  # inside what the fine search covers. Extra, and labelled extra.
  FINEOFF="viewoff=0.30,-0.22,0.10 fovoff=0.75"
  # shellcheck disable=SC2086
  plant_at g1-null-fine-b160  $B160 size=3840 patch=15 search=96 plant=roll:0 $FINEOFF &
  # shellcheck disable=SC2086
  plant_at g2-plant-fine-b160 $B160 size=3840 patch=15 search=96 $PLANT $FINEOFF &
  # shellcheck disable=SC2086
  plant_at g1-null-fine-h160  $H160 size=3840 patch=15 search=96 plant=roll:0 $FINEOFF &
  # shellcheck disable=SC2086
  plant_at g2-plant-fine-h160 $H160 size=3840 patch=15 search=96 $PLANT $FINEOFF &
  wait
  ;;
# ----------------------------------------------------------------- the answer
# Two arms at every geometry, and the pair of them IS criterion (b): the same
# sites, the same frames, the same start, and the only difference between them
# is whether lens 1's five numbers are allowed to move off what the file's own
# offset_v3 says. If the solved arm does not beat the factory arm by three
# times, this run has not discriminated between calibrations, whatever its
# residual is.
solve)
  solve_at solve-b160 against="$E8" $B160 out="$OUT/deliver/building-160/solved" amp=8 &
  solve_at solve-h160 against="$E9" $H160 out="$OUT/deliver/horizon-160/solved" amp=8 &
  wait
  solve_at v3-b160 against="$E8" $B160 free=view out="$OUT/deliver/building-160/v3" amp=8 &
  solve_at v3-h160 against="$E9" $H160 free=view out="$OUT/deliver/horizon-160/v3" amp=8 &
  wait
  solve_at solve-b055 against="$E8" $B055 &
  solve_at solve-h055 against="$E9" $H055 &
  solve_at v3-b055 against="$E8" $B055 free=view &
  solve_at v3-h055 against="$E9" $H055 free=view &
  wait
  ;;
*)
  echo "no stage called $STAGE" >&2
  exit 2
  ;;
esac

echo "=================================================== summary"
grep -H -E "^peak:|^score:|^world:|^body aim|^R[0-9]|^REGISTERED|^NOT REGISTERED|^their |^residual|^PLANT|^REFUSED" \
  "$OUT"/*.txt 2>/dev/null
